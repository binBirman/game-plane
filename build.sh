#!/usr/bin/env bash
set -euo pipefail

VERSION="${VERSION:-0.1.0}"
TARGET="${TARGET:-x86_64-unknown-linux-musl}"
PROFILE="${PROFILE:-release}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "==> Lobby build script (v$VERSION, target=$TARGET)"

# 1. ensure rustup target is installed
if command -v rustup >/dev/null 2>&1; then
    if ! rustup target list --installed | grep -q "^${TARGET}$"; then
        echo "==> Adding target $TARGET via rustup"
        rustup target add "$TARGET"
    fi
fi

# 2. ensure musl-gcc is available (Debian/Ubuntu: musl-tools)
if ! command -v musl-gcc >/dev/null 2>&1; then
    echo "WARNING: musl-gcc not found. If build fails, install it:"
    echo "  Debian/Ubuntu:  sudo apt install musl-tools"
    echo "  Alpine:         apk add musl-dev gcc"
fi

# 3. set up musl linker for cargo
export CC_musl="${CC_musl:-musl-gcc}"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER:-musl-gcc}"

# 4. build
echo "==> cargo build --$PROFILE --target $TARGET"
cargo build --"$PROFILE" --target "$TARGET"

LOBBY_BIN="target/$TARGET/$PROFILE/lobby"
GAME_BIN="target/$TARGET/$PROFILE/tictactoe"
TYP_BIN="target/$TARGET/$PROFILE/take_your_position"
[[ -f "$LOBBY_BIN" ]] || { echo "Build failed: $LOBBY_BIN not found" >&2; exit 1; }
[[ -f "$GAME_BIN" ]] || { echo "Build failed: $GAME_BIN not found" >&2; exit 1; }
[[ -f "$TYP_BIN"  ]] || { echo "Build failed: $TYP_BIN not found"  >&2; exit 1; }

# 5. assemble dist
DIST="dist/lobby-$VERSION"
rm -rf "$DIST" "dist/lobby-$VERSION.tar.gz"
mkdir -p "$DIST"
cp "$LOBBY_BIN" "$DIST/lobby"
cp "$GAME_BIN" "$DIST/tictactoe"
cp "$TYP_BIN"  "$DIST/take_your_position"
cp packaging/lobby.service       "$DIST/"
cp packaging/lobby.env.example  "$DIST/lobby.env.example"
cp packaging/games.toml          "$DIST/games.toml"
cp packaging/install.sh          "$DIST/install.sh"
cp packaging/uninstall.sh        "$DIST/uninstall.sh"
cp packaging/README.md           "$DIST/README.md"
cp packaging/DEPLOY.md           "$DIST/DEPLOY.md"
cp packaging/nginx.conf          "$DIST/nginx.conf.example"
cp packaging/RUNBOOK.md          "$DIST/RUNBOOK.md"
cp README.md                     "$DIST/README.upstream.md"
cp -r crates/lobby/static        "$DIST/static"
chmod +x "$DIST/install.sh" "$DIST/uninstall.sh" "$DIST/lobby" "$DIST/tictactoe" "$DIST/take_your_position"

# 6. tarball
tar czf "dist/lobby-$VERSION.tar.gz" -C dist "lobby-$VERSION"

echo
echo "==> Done"
echo "    lobby  : $LOBBY_BIN ($(stat -c '%s bytes' "$LOBBY_BIN"))"
echo "    game   : $GAME_BIN ($(stat -c '%s bytes' "$GAME_BIN"))"
echo "    typ    : $TYP_BIN ($(stat -c '%s bytes' "$TYP_BIN"))"
echo "    tarball: dist/lobby-$VERSION.tar.gz"
echo
echo "Next: scp dist/lobby-$VERSION.tar.gz user@server:"
echo "      ssh user@server 'tar xzf lobby-$VERSION.tar.gz && cd lobby-$VERSION && sudo ./install.sh'"