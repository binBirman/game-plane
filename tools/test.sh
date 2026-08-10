#!/usr/bin/env bash
# Lobby smoke-test tool. Requires: curl, python3.
#
# Usage:
#   ./tools/test.sh                          # default 127.0.0.1:8192
#   HOST=10.0.0.1 PORT=9000 ./tools/test.sh
#   TEST_USER=alice ./tools/test.sh          # override test username
#
# Exits 0 if all tests pass, 1 otherwise.

set -uo pipefail

HOST="${HOST:-127.0.0.1}"
PORT="${PORT:-8192}"
BASE="${BASE_URL:-http://${HOST}:${PORT}}"
TEST_USER="${TEST_USER:-lobbytest_$(date +%s%N)}"
TEST_USER_B="${TEST_USER_B:-${TEST_USER}_b}"
STRONG_PASS="Test_123!"

# auto-bypass system proxy
CURL_OPTS=()
if env | grep -qE '^(http_proxy|https_proxy|HTTP_PROXY|HTTPS_PROXY)=.+'; then
    CURL_OPTS+=(--noproxy '*')
fi

# colors
if [[ -t 1 ]]; then
    R=$'\033[0;31m'; G=$'\033[0;32m'; Y=$'\033[0;33m'; B=$'\033[0;34m'; D=$'\033[2m'; N=$'\033[0m'
else
    R=""; G=""; Y=""; B=""; D=""; N=""
fi

# python3 is required for the PoW solver
if ! command -v python3 >/dev/null 2>&1; then
    echo "${R}ERROR: python3 required (for PoW solver)${N}" >&2
    exit 2
fi

pass=0; fail=0; total=0
CAP=""

run_test() {
    local name="$1" want="$2"; shift 2
    total=$((total+1))
    echo
    echo "${B}─── Test $total: $name ───${N}"

    local headers_file body_file status body req_id
    headers_file=$(mktemp)
    body_file=$(mktemp)

    status=$(curl -sS "${CURL_OPTS[@]}" -o "$body_file" -w "%{http_code}" \
        -D "$headers_file" "$@" 2>&1) || {
        echo "${R}curl failed: $status${N}"
        rm -f "$headers_file" "$body_file"
        fail=$((fail+1)); return
    }

    body=$(cat "$body_file" 2>/dev/null || echo "")
    rm -f "$body_file"
    req_id=$(grep -i '^x-request-id:' "$headers_file" 2>/dev/null | tr -d '\r' | awk '{print $2}')
    rm -f "$headers_file"

    echo "  ${D}status        :${N} $status"
    echo "  ${D}body          :${N} $body"
    echo "  ${D}x-request-id  :${N} ${req_id:-<none>}"

    if [[ "$status" == "$want" ]]; then
        echo "  ${G}PASS${N}"; pass=$((pass+1))
    else
        echo "  ${R}FAIL (expected $want)${N}"; fail=$((fail+1))
    fi
}

# Solve proof-of-work: find nonce where SHA256(challenge+":"+nonce) has >= difficulty leading zero bits.
solve_pow() {
    local challenge="$1" difficulty="$2"
    python3 -c "
import hashlib
c, d = '$challenge', int('$difficulty')
n = 0
while True:
    h = hashlib.sha256(f'{c}:{n}'.encode()).hexdigest()
    bits = 0
    for ch in h:
        if   ch == '0': bits += 4
        elif ch == '1': bits += 3; break
        elif ch == '2': bits += 2; break
        elif ch == '3': bits += 1; break
        else: break
    if bits >= d:
        print(n); break
    n += 1
"
}

# Fetch challenge and solve PoW. Caches in CAP.
ensure_captcha() {
    if [[ -n "$CAP" ]]; then return; fi
    local resp challenge difficulty nonce
    resp=$(curl -sS "${CURL_OPTS[@]}" -X POST "$BASE/api/captcha/challenge")
    challenge=$(echo "$resp" | python3 -c "import sys,json; print(json.load(sys.stdin)['challenge'])")
    difficulty=$(echo "$resp" | python3 -c "import sys,json; print(json.load(sys.stdin)['difficulty'])")
    echo "  ${D}(pre-solving PoW difficulty=$difficulty...)${N}"
    nonce=$(solve_pow "$challenge" "$difficulty")
    CAP="{\"challenge\":\"$challenge\",\"nonce\":\"$nonce\"}"
}

echo "${B}Lobby smoke test${N}"
echo "  target : $BASE"
echo "  curl   : ${CURL_OPTS[*]:-system default}"
echo "  user   : $TEST_USER"
echo "  pass   : $STRONG_PASS"

# ─── 1. server reachable ──────────────────────────────────────────────
total=$((total+1))
echo
echo "${B}─── Test $total: server reachability ───${N}"
reach_status=$(curl -sS "${CURL_OPTS[@]}" -o /dev/null -w "%{http_code}" --max-time 5 "$BASE/" 2>/dev/null || echo "000")
if [[ "$reach_status" =~ ^[2-5][0-9][0-9]$ ]]; then
    echo "  ${D}got status:${N} $reach_status"
    echo "  ${G}PASS${N}"; pass=$((pass+1))
else
    echo "  ${R}FAIL${N} — cannot reach $BASE"
    fail=$((fail+1))
    exit 1
fi

# ─── 2. captcha endpoint ──────────────────────────────────────────────
run_test "POST /api/captcha/challenge" 200 \
    -X POST "$BASE/api/captcha/challenge"

# Pre-solve PoW (cached in CAP, reused by all subsequent captcha tests)
ensure_captcha

# ─── 3. register: no captcha ──────────────────────────────────────────
run_test "POST /api/register (no captcha)" 400 \
    -X POST "$BASE/api/register" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TEST_USER\",\"password\":\"$STRONG_PASS\",\"nickname\":\"x\"}"

# ─── 4. register: invalid captcha ─────────────────────────────────────
run_test "POST /api/register (invalid captcha)" 400 \
    -X POST "$BASE/api/register" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TEST_USER\",\"password\":\"$STRONG_PASS\",\"nickname\":\"x\",\"captcha\":{\"challenge\":\"deadbeefdeadbeefdeadbeefdeadbeef\",\"nonce\":\"0\"}}"

# ─── 5-8. register: weak password (with valid captcha) ────────────────
run_test "POST /api/register (weak: short <9)" 400 \
    -X POST "$BASE/api/register" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"short_$TEST_USER\",\"password\":\"Ab1!\",\"nickname\":\"x\",\"captcha\":$CAP}"

run_test "POST /api/register (weak: no digit)" 400 \
    -X POST "$BASE/api/register" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"nodigit_$TEST_USER\",\"password\":\"NoDigits_!!\",\"nickname\":\"x\",\"captcha\":$CAP}"

run_test "POST /api/register (weak: no letter)" 400 \
    -X POST "$BASE/api/register" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"noletter_$TEST_USER\",\"password\":\"12345678!\",\"nickname\":\"x\",\"captcha\":$CAP}"

run_test "POST /api/register (weak: no special)" 400 \
    -X POST "$BASE/api/register" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"nospcl_$TEST_USER\",\"password\":\"NoSpecial123\",\"nickname\":\"x\",\"captcha\":$CAP}"

# ─── 9. register: happy path ──────────────────────────────────────────
run_test "POST /api/register (new user)" 200 \
    -X POST "$BASE/api/register" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TEST_USER\",\"password\":\"$STRONG_PASS\",\"nickname\":\"Test User\",\"captcha\":$CAP}"

# ─── 10. register: duplicate ──────────────────────────────────────────
run_test "POST /api/register (duplicate)" 409 \
    -X POST "$BASE/api/register" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TEST_USER\",\"password\":\"$STRONG_PASS\",\"nickname\":\"Test User\",\"captcha\":$CAP}"

# ─── 11. login: no captcha ────────────────────────────────────────────
run_test "POST /api/login (no captcha)" 400 \
    -X POST "$BASE/api/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TEST_USER\",\"password\":\"$STRONG_PASS\"}"

# ─── 12. login: correct creds ─────────────────────────────────────────
login_body=$(curl -sS "${CURL_OPTS[@]}" -X POST "$BASE/api/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TEST_USER\",\"password\":\"$STRONG_PASS\",\"captcha\":$CAP}")
token=$(echo "$login_body" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('token',''))" 2>/dev/null || echo "")
uid=$(echo "$login_body" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('uid',''))" 2>/dev/null || echo "")

total=$((total+1))
echo
echo "${B}─── Test $total: POST /api/login (correct creds) ───${N}"
echo "  ${D}body  :${N} $login_body"
echo "  ${D}token :${N} ${token:-<none>}"
echo "  ${D}uid   :${N} ${uid:-<none>}"
if [[ -n "$token" && ${#token} -eq 64 ]]; then
    echo "  ${G}PASS${N} (64-hex token)"; pass=$((pass+1))
else
    echo "  ${R}FAIL${N}"; fail=$((fail+1))
fi

# ─── 13. login: wrong password ────────────────────────────────────────
run_test "POST /api/login (wrong password)" 401 \
    -X POST "$BASE/api/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TEST_USER\",\"password\":\"WrongPass_1!\",\"captcha\":$CAP}"

# ─── 14. login: no such user ──────────────────────────────────────────
run_test "POST /api/login (no such user)" 401 \
    -X POST "$BASE/api/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"nonexistent_$$_$(date +%s)\",\"password\":\"x\",\"captcha\":$CAP}"

# ─── 15-22. room + game spawn + WS roundtrip ───────────────────────────
TOKEN_A=""
UID_A=""
TOKEN_B=""
UID_B=""
ROOM_ID=""
INSTANCE_ID=""
WS_URL=""

ensure_captcha
run_test "POST /api/register (user B)" 200 \
    -X POST "$BASE/api/register" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TEST_USER_B\",\"password\":\"$STRONG_PASS\",\"nickname\":\"User B\",\"captcha\":$CAP}"

ensure_captcha
login_b_body=$(curl -sS "${CURL_OPTS[@]}" -X POST "$BASE/api/login" \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"$TEST_USER_B\",\"password\":\"$STRONG_PASS\",\"captcha\":$CAP}")
TOKEN_B=$(echo "$login_b_body" | python3 -c "import sys,json; print(json.load(sys.stdin).get('token',''))" 2>/dev/null || echo "")
UID_B=$(echo "$login_b_body" | python3 -c "import sys,json; print(json.load(sys.stdin).get('uid',''))" 2>/dev/null || echo "")

total=$((total+1))
echo
echo "${B}─── Test $total: login user B (correct creds) ───${N}"
echo "  ${D}body:${N} $login_b_body"
if [[ -n "$TOKEN_B" && ${#TOKEN_B} -eq 64 ]]; then
    echo "  ${G}PASS${N} (64-hex token)"; pass=$((pass+1))
else
    echo "  ${R}FAIL${N}"; fail=$((fail+1))
fi

run_test "POST /api/rooms (create as A)" 201 \
    -X POST "$BASE/api/rooms" \
    -H "Authorization: Bearer $token" \
    -H 'Content-Type: application/json' \
    -d '{"game_type":"tictactoe"}'

room_create_body=$(curl -sS "${CURL_OPTS[@]}" -X POST "$BASE/api/rooms" \
    -H "Authorization: Bearer $token" \
    -H 'Content-Type: application/json' \
    -d '{"game_type":"tictactoe"}')
ROOM_ID=$(echo "$room_create_body" | python3 -c "import sys,json; print(json.load(sys.stdin).get('room_id',''))" 2>/dev/null || echo "")

total=$((total+1))
echo
echo "${B}─── Test $total: capture room_id ───${N}"
echo "  ${D}room_id:${N} ${ROOM_ID:-<none>}"
if [[ -n "$ROOM_ID" && "$ROOM_ID" =~ ^[0-9]+$ ]]; then
    echo "  ${G}PASS${N}"; pass=$((pass+1))
else
    echo "  ${R}FAIL${N}"; fail=$((fail+1))
fi

run_test "POST /api/rooms/$ROOM_ID/join (as B)" 200 \
    -X POST "$BASE/api/rooms/$ROOM_ID/join" \
    -H "Authorization: Bearer $TOKEN_B" \
    -H 'Content-Type: application/json' \
    -d '{}'

run_test "POST /api/rooms/$ROOM_ID/start (as A)" 200 \
    -X POST "$BASE/api/rooms/$ROOM_ID/start" \
    -H "Authorization: Bearer $token" \
    -H 'Content-Type: application/json' \
    -d '{}'

start_body=$(curl -sS "${CURL_OPTS[@]}" -X POST "$BASE/api/rooms/$ROOM_ID/start" \
    -H "Authorization: Bearer $token" \
    -H 'Content-Type: application/json' \
    -d '{}')
INSTANCE_ID=$(echo "$start_body" | python3 -c "import sys,json; print(json.load(sys.stdin).get('instance_id',''))" 2>/dev/null || echo "")
WS_URL=$(echo "$start_body" | python3 -c "import sys,json; print(json.load(sys.stdin).get('ws_url',''))" 2>/dev/null || echo "")

total=$((total+1))
echo
echo "${B}─── Test $total: start returns ws_url with instance_id ───${N}"
echo "  ${D}instance_id:${N} ${INSTANCE_ID:-<none>}"
echo "  ${D}ws_url     :${N} ${WS_URL:-<none>}"
if [[ -n "$INSTANCE_ID" && "$WS_URL" == *"/ws/$INSTANCE_ID"* ]]; then
    echo "  ${G}PASS${N}"; pass=$((pass+1))
else
    echo "  ${R}FAIL${N}"; fail=$((fail+1))
fi

# WS roundtrip via tools/ws_client.py
if [[ -n "$INSTANCE_ID" && -n "$token" ]]; then
    total=$((total+1))
    echo
    echo "${B}─── Test $total: WS roundtrip (login + snapshot + move) ───${N}"
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    ws_out=$(python3 - "$HOST" "$PORT" "$INSTANCE_ID" "$token" "$UID_A" <<'PYEOF'
import sys
sys.path.insert(0, sys.argv[0] and "" or ".")
sys.path.insert(0, "$SCRIPT_DIR")
from ws_client import handshake, send_text, recv_text

host, port, instance_id, token, uid_a = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5]

try:
    sock = handshake(host, port, "/ws/" + instance_id)
except Exception as e:
    print("HANDSHAKE_FAIL:" + str(e))
    sys.exit(1)

try:
    send_text(sock, '{"type":"login","uid":' + str(uid_a) + ',"session":"' + token + '"}')
    f1 = recv_text(sock)
    f2 = recv_text(sock)
    if not f1 or 'login_ok' not in f1:
        print("LOGIN_FAIL:" + (f1 or "<none>"))
        sys.exit(2)
    if not f2 or 'snapshot' not in f2:
        print("SNAPSHOT_FAIL:" + (f2 or "<none>"))
        sys.exit(3)
    send_text(sock, '{"type":"game","data":{"action":"move","cell":0}}')
    f3 = recv_text(sock)
    if not f3 or 'game' not in f3:
        print("MOVE_FAIL:" + (f3 or "<none>"))
        sys.exit(4)
    print("OK")
except Exception as e:
    print("ERR:" + str(e))
    sys.exit(5)
finally:
    try: sock.close()
    except: pass
PYEOF
    )
    if [[ "$ws_out" == "OK" ]]; then
        echo "  ${D}handshake+login+snapshot+move all OK${N}"
        echo "  ${G}PASS${N}"; pass=$((pass+1))
    else
        echo "  ${D}output:${N} $ws_out"
        echo "  ${R}FAIL${N}"; fail=$((fail+1))
    fi
fi

# Cleanup: A leaves, then B leaves
run_test "POST /api/rooms/$ROOM_ID/leave (A)" 200 \
    -X POST "$BASE/api/rooms/$ROOM_ID/leave" \
    -H "Authorization: Bearer $token" \
    -H 'Content-Type: application/json' \
    -d '{}'

run_test "POST /api/rooms/$ROOM_ID/leave (B)" 200 \
    -X POST "$BASE/api/rooms/$ROOM_ID/leave" \
    -H "Authorization: Bearer $TOKEN_B" \
    -H 'Content-Type: application/json' \
    -d '{}'

# ─── summary ──────────────────────────────────────────────────────────
echo
echo "${B}══════════════════════════════════════════════════════════${N}"
if [[ $fail -eq 0 ]]; then
    echo "${G}All $total tests passed${N}"
    exit 0
else
    echo "${R}$fail/$total tests failed${N}, ${G}$pass passed${N}"
    echo
    echo "Tips:"
    echo "  - Inspect lobby journal:  sudo journalctl -u lobby -n 50"
    echo "  - Inspect file logs   :  tail -f /var/log/lobby/lobby.YYYY-MM-DD.log"
    echo "  - Look up by request_id: pass --noproxy '*' curl -H 'x-request-id: <id>' ..."
    exit 1
fi