(function () {
    "use strict";

    // ─── state ────────────────────────────────────────────────────────
    // Persist a small slice of state to localStorage so that after a page
    // refresh (state is otherwise reinitialised to zero) we can still render
    // the room correctly — without this, getNickname returns null and every
    // seat panel falls back to "uid=N". Specifically we persist:
    //   - currentRoomId: the room the user is currently inside
    //   - roomCache:     the cached RoomInfo for that room (has player
    //                    uids and nicknames, which getNickname looks up)
    function loadRoomCacheFromStorage() {
        try {
            const obj = JSON.parse(localStorage.getItem("lobby_room_cache") || "{}");
            // JSON.stringify serialises Map keys as strings, so a key like
            // number 5 becomes "5" in storage. The rest of the code looks
            // up with the number (e.g. `state.roomCache.get(roomId)`) — a
            // string key would make every lookup miss and force every
            // getNickname() to fall back to "uid=N". Re-parse to numbers.
            const m = new Map();
            for (const [k, v] of Object.entries(obj)) {
                const n = parseInt(k, 10);
                if (!Number.isNaN(n)) m.set(n, v);
            }
            return m;
        } catch { return new Map(); }
    }
    function saveRoomCacheToStorage() {
        const obj = {};
        for (const [k, v] of state.roomCache) obj[k] = v;
        try { localStorage.setItem("lobby_room_cache", JSON.stringify(obj)); } catch {}
    }

    const state = {
        token: localStorage.getItem("lobby_token") || "",
        uid: parseInt(localStorage.getItem("lobby_uid") || "0", 10),
        nickname: localStorage.getItem("lobby_nick") || "",
        ws: null,
        wsInstance: 0,
        wsLog: [],
        roomCache: loadRoomCacheFromStorage(),
        lastRoomPlayerUids: new Set(),
        roomPollTimer: null,
        roomPollRoomId: 0,
        roomLastFetchAt: 0,
        knownGames: new Map(),
        // The room_id of the page the user is currently inside. Tracked so
        // the game-over modal can route back to #room/<id> (not #lobby).
        // Persisted so a refresh on #game/<id> still has a valid room to
        // look up nicknames against.
        currentRoomId: parseInt(localStorage.getItem("lobby_current_room_id") || "0", 10),
        // The game_type of the game view the user last entered (e.g.
        // "take_your_position"). Persisted so that on a page refresh, the
        // router at #game/<id> still picks the right renderer — the room
        // cache is empty at refresh time, so without this hint we'd fall
        // back to the default ("tictactoe") and the TYP DOM scaffold would
        // never be created. Cleared when the user leaves the game view.
        lastGameType: localStorage.getItem("lobby_last_game_type") || "",
    };

    // ─── game metadata (frontend cache of /api/games) ────────────────
    // Filled on lobby page mount; read on room page render.
    // icon is a token rendered via CSS — NOT an emoji. See docs/frontend-design.md §3.
    const GAME_META = {
        tictactoe: { icon: "tic-tac-toe", name: "井字棋", description: "3×3 棋盘，两位玩家轮流落子，先三连一线者胜。" },
    };
    function gameMeta(type) {
        if (GAME_META[type]) return GAME_META[type];
        return state.knownGames.get(type) || { icon: "generic", name: type, description: "" };
    }

    // ─── helpers ──────────────────────────────────────────────────────
    const $ = (sel, el = document) => el.querySelector(sel);
    const $$ = (sel, el = document) => Array.from(el.querySelectorAll(sel));
    const el = (tag, attrs = {}, children = []) => {
        const e = document.createElement(tag);
        for (const [k, v] of Object.entries(attrs)) {
            if (k === "class") e.className = v;
            else if (k === "html") e.innerHTML = v;
            else if (k.startsWith("on") && typeof v === "function") e.addEventListener(k.slice(2), v);
            else if (typeof v === "boolean") {
                // Boolean HTML attributes (disabled / checked / required / etc.):
                // present means on, absent means off. setAttribute(name, "false") is still ON.
                if (v) e.setAttribute(k, "");
                else e.removeAttribute(k);
            } else {
                e.setAttribute(k, v);
            }
        }
        for (const c of [].concat(children)) {
            if (c == null) continue;
            e.appendChild(typeof c === "string" ? document.createTextNode(c) : c);
        }
        return e;
    };

    function prettyJSON(t) { try { return JSON.stringify(JSON.parse(t), null, 2); } catch { return t; } }

    function escapeHtml(s) {
        return String(s).replace(/[&<>"']/g, (c) => ({
            "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"
        })[c]);
    }

    function toast(msg, kind = "") {
        const t = $("#toast");
        t.textContent = msg;
        t.className = "toast show " + kind;
        setTimeout(() => { t.className = "toast " + kind; }, 2500);
    }

    async function logout() {
        // Best-effort server-side session revoke. Failures (offline, 401,
        // already-revoked) are silently swallowed — the local clear below
        // is what actually un-authenticates the UI.
        try {
            await fetch("/api/logout", { method: "POST", headers: { "Authorization": "Bearer " + state.token } });
        } catch (_) { /* network error etc. — keep going */ }
        localStorage.removeItem("lobby_token");
        localStorage.removeItem("lobby_uid");
        localStorage.removeItem("lobby_nick");
        state.token = state.uid = 0; state.nickname = "";
        stopRoomPolling();
        if (gameOverNavTimer) { clearTimeout(gameOverNavTimer); gameOverNavTimer = null; }
        location.hash = "#login";
        render();
    }

    async function api(path, body) {
        const headers = { "Content-Type": "application/json" };
        if (state.token) headers["Authorization"] = "Bearer " + state.token;
        const res = await fetch(path, {
            method: "POST",
            headers,
            body: JSON.stringify(body || {}),
        });
        const text = await res.text();
        if (!res.ok) {
            let msg = text;
            try { msg = JSON.parse(text).error?.message || msg; } catch {}
            throw new Error(`${res.status} ${msg}`);
        }
        return JSON.parse(text);
    }

    async function apiGet(path) {
        const headers = {};
        if (state.token) headers["Authorization"] = "Bearer " + state.token;
        const res = await fetch(path, { headers });
        const text = await res.text();
        if (!res.ok) {
            let msg = text;
            try { msg = JSON.parse(text).error?.message || msg; } catch {}
            throw new Error(`${res.status} ${msg}`);
        }
        return JSON.parse(text);
    }

    // ─── captcha ──────────────────────────────────────────────────────
    async function solveCaptcha() {
        const res = await fetch("/api/captcha/challenge", { method: "POST" });
        if (!res.ok) throw new Error("captcha fetch failed");
        const { challenge, difficulty } = await res.json();
        const enc = new TextEncoder();
        const digest = await pickSha256();
        let nonce = 0;
        while (true) {
            const input = enc.encode(challenge + ":" + nonce);
            const hash = await digest(input);
            if (leadingZeros(hash) >= difficulty) return { challenge, nonce: String(nonce) };
            nonce++;
        }
    }

    function leadingZeros(hash) {
        let bits = 0;
        for (const b of hash) {
            if (b === 0) bits += 8;
            else { bits += Math.clz32(b) - 24; break; }
        }
        return bits;
    }

    // WebCrypto.subtle is only available in a Secure Context (HTTPS or
    // localhost/127.0.0.1). When the page is served over plain HTTP from a
    // remote address we fall back to a pure-JS SHA-256 so the PoW solver
    // still works (slower, but functional).
    function pickSha256() {
        if (typeof crypto !== "undefined" && crypto.subtle && typeof crypto.subtle.digest === "function") {
            return (input) => crypto.subtle.digest("SHA-256", input).then((buf) => new Uint8Array(buf));
        }
        return (input) => Promise.resolve(jsSha256(input));
    }

    // Minimal SHA-256 (pure JS). Returns Uint8Array(32).
    function jsSha256(bytes) {
        function rotr(n, x) { return (x >>> n) | (x << (32 - n)); }
        const K = [
            0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
            0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
            0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
            0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
            0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
            0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
            0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
            0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2,
        ];
        let l = bytes.length;
        const withLen = (l + 9 + 63) & ~0x3f;
        const buf = new Uint8Array(withLen);
        buf.set(bytes);
        buf[l] = 0x80;
        const bitLen = l * 8;
        const dv = new DataView(buf.buffer);
        dv.setUint32(withLen - 4, bitLen >>> 0, false);
        dv.setUint32(withLen - 8, Math.floor(bitLen / 0x100000000), false);

        const H = [
            0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,
            0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19,
        ];
        const W = new Uint32Array(64);
        for (let off = 0; off < withLen; off += 64) {
            for (let i = 0; i < 16; i++) W[i] = dv.getUint32(off + i * 4, false);
            for (let i = 16; i < 64; i++) {
                const s0 = rotr(7, W[i-15]) ^ rotr(18, W[i-15]) ^ (W[i-15] >>> 3);
                const s1 = rotr(17, W[i-2]) ^ rotr(19, W[i-2]) ^ (W[i-2] >>> 10);
                W[i] = (W[i-16] + s0 + W[i-7] + s1) | 0;
            }
            let [a,b,c,d,e,f,g,h] = H;
            for (let i = 0; i < 64; i++) {
                const S1 = rotr(6, e) ^ rotr(11, e) ^ rotr(25, e);
                const ch = (e & f) ^ (~e & g);
                const t1 = (h + S1 + ch + K[i] + W[i]) | 0;
                const S0 = rotr(2, a) ^ rotr(13, a) ^ rotr(22, a);
                const mj = (a & b) ^ (a & c) ^ (b & c);
                const t2 = (S0 + mj) | 0;
                h = g; g = f; f = e; e = (d + t1) | 0;
                d = c; c = b; b = a; a = (t1 + t2) | 0;
            }
            H[0] = (H[0] + a) | 0; H[1] = (H[1] + b) | 0;
            H[2] = (H[2] + c) | 0; H[3] = (H[3] + d) | 0;
            H[4] = (H[4] + e) | 0; H[5] = (H[5] + f) | 0;
            H[6] = (H[6] + g) | 0; H[7] = (H[7] + h) | 0;
        }
        const out = new Uint8Array(32);
        const odv = new DataView(out.buffer);
        for (let i = 0; i < 8; i++) odv.setUint32(i * 4, H[i], false);
        return out;
    }

    // ─── auth pages ───────────────────────────────────────────────────
    function renderAuth() {
        $("#topbar").classList.add("hidden");
        const app = $("#app");
        app.innerHTML = "";
        app.appendChild(el("h1", {}, "大厅"));
        app.appendChild(el("p", { class: "subtitle" }, "登录账号或注册新账号。"));
        const tabs = el("nav", { class: "tabs" }, [
            el("button", { id: "tab-login", class: "active", onclick: () => showTab("login") }, "登录"),
            el("button", { id: "tab-register", onclick: () => showTab("register") }, "注册"),
        ]);
        app.appendChild(tabs);

        const loginForm = el("form", { class: "card", id: "form-login", novalidate: true, onsubmit: handleLogin }, [
            fieldGroup("用户名", el("input", { name: "username", required: true, autocomplete: "username" })),
            fieldGroup("密码", passwordInput("password", "current-password")),
            el("button", { class: "primary", type: "submit" }, "登录"),
        ]);
        const registerForm = el("form", { class: "card hidden", id: "form-register", novalidate: true, onsubmit: handleRegister }, [
            fieldGroup("用户名", el("input", { name: "username", required: true, autocomplete: "username" }), "3-20 字符"),
            fieldGroup("密码", passwordInput("password", "new-password"), "至少 8 位"),
            fieldGroup("确认密码", passwordInput("password_confirm", "new-password"), "再次输入密码，必须一致"),
            fieldGroup("昵称", el("input", { name: "nickname", required: true })),
            el("button", { class: "primary", type: "submit" }, "创建账号"),
        ]);
        app.appendChild(loginForm);
        app.appendChild(registerForm);
    }

    function fieldGroup(label, inputOrWrap, hint) {
        // The wrapped password field is a `<div class="pwd-wrap">`; pull
        // the inner input so error binding still works.
        const input = inputOrWrap.matches?.("input")
            ? inputOrWrap
            : inputOrWrap.querySelector("input");
        const id = input?.getAttribute("name") || "";
        const errorSlot = el("div", { class: "field-error", "data-error-for": id, role: "alert" });
        const wrap = el("label", { class: "field-group" }, [
            label,
            inputOrWrap,
            hint ? el("div", { class: "field-hint" }, hint) : null,
            errorSlot,
        ]);
        // Mark invalid + clear as user types.
        if (input) input.addEventListener("input", () => {
            input.classList.remove("invalid");
            errorSlot.textContent = "";
        });
        return wrap;
    }

    // Password input wrapped with a show/hide toggle button. Toggle swaps
    // `type` between "password" and "text" and updates the label. Plain text
    // (no emoji — see docs/frontend-design.md §5) so the affordance reads
    // the same on every OS.
    function passwordInput(name, autocomplete) {
        const input = el("input", { name, type: "password", required: true, autocomplete });
        const btn = el("button", {
            type: "button",
            class: "pwd-toggle",
            "aria-label": "显示密码",
            "aria-pressed": "false",
            onclick: () => {
                const show = input.type === "password";
                input.type = show ? "text" : "password";
                btn.textContent = show ? "隐藏" : "显示";
                btn.setAttribute("aria-label", show ? "隐藏密码" : "显示密码");
                btn.setAttribute("aria-pressed", show ? "true" : "false");
            },
        }, "显示");
        return el("div", { class: "pwd-wrap" }, [input, btn]);
    }

    function setFieldError(form, name, message) {
        const input = form.querySelector(`[name="${name}"]`);
        if (!input) return;
        input.classList.add("invalid");
        const slot = form.querySelector(`[data-error-for="${name}"]`);
        if (slot) slot.textContent = message;
    }

    function clearFieldErrors(form) {
        form.querySelectorAll(".field-error").forEach(s => s.textContent = "");
        form.querySelectorAll("input.invalid").forEach(i => i.classList.remove("invalid"));
    }

    function showTab(name) {
        $$("#app .tabs button").forEach(b => b.classList.toggle("active", b.id === "tab-" + name));
        $("#form-login").classList.toggle("hidden", name !== "login");
        $("#form-register").classList.toggle("hidden", name !== "register");
    }

    function clientValidate(form) {
        clearFieldErrors(form);
        const fd = new FormData(form);
        const username = (fd.get("username") || "").trim();
        const password = fd.get("password") || "";
        let ok = true;
        if (form.id === "form-register") {
            if (username.length < 3 || username.length > 20) {
                setFieldError(form, "username", "用户名需 3-20 字符");
                ok = false;
            }
            if (password.length < 8) {
                setFieldError(form, "password", "密码至少 8 位");
                ok = false;
            }
            const passwordConfirm = fd.get("password_confirm") || "";
            if (password !== passwordConfirm) {
                setFieldError(form, "password_confirm", "两次输入的密码不一致");
                ok = false;
            }
            const nickname = (fd.get("nickname") || "").trim();
            if (!nickname) {
                setFieldError(form, "nickname", "请填写昵称");
                ok = false;
            }
        } else {
            if (!username) { setFieldError(form, "username", "请填写用户名"); ok = false; }
            if (!password) { setFieldError(form, "password", "请填写密码"); ok = false; }
        }
        return ok;
    }

    // Map a backend error code to the field it should be shown against.
    function mapBackendErrorToField(form, errMessage) {
        // errMessage looks like "400 WEAK_PASSWORD: ...".
        const m = errMessage.match(/^[0-9]+\s+(\w+):\s*(.*)$/);
        const code = m ? m[1] : "";
        const detail = m ? m[2] : errMessage;
        if (code === "USERNAME_TAKEN" || code === "USER_NOT_FOUND" || code === "INVALID_CREDENTIALS") {
            setFieldError(form, "username", detail);
            return true;
        }
        if (code === "WEAK_PASSWORD" || code === "INVALID_CREDENTIALS") {
            setFieldError(form, "password", detail);
            return true;
        }
        return false;
    }

    async function handleLogin(e) {
        e.preventDefault();
        const form = e.target;
        if (!clientValidate(form)) return;
        const fd = new FormData(form);
        const btn = form.querySelector("button[type=submit]");
        if (btn) { btn.disabled = true; btn.textContent = "登录中…"; }
        try {
            toast("正在计算人机验证…", "");
            const captcha = await solveCaptcha();
            const r = await api("/api/login", { username: fd.get("username"), password: fd.get("password"), captcha });
            state.token = r.token; state.uid = r.uid;
            state.nickname = fd.get("username");
            localStorage.setItem("lobby_token", r.token);
            localStorage.setItem("lobby_uid", String(r.uid));
            localStorage.setItem("lobby_nick", state.nickname);
            toast("登录成功", "ok");
            location.hash = "#lobby";
            render();
        } catch (err) {
            clearFieldErrors(form);
            if (!mapBackendErrorToField(form, err.message)) {
                toast(err.message, "error");
            }
            if (btn) { btn.disabled = false; btn.textContent = "登录"; }
        }
    }

    async function handleRegister(e) {
        e.preventDefault();
        const form = e.target;
        if (!clientValidate(form)) return;
        const fd = new FormData(form);
        const btn = form.querySelector("button[type=submit]");
        if (btn) { btn.disabled = true; btn.textContent = "创建中…"; }
        try {
            toast("正在计算人机验证…", "");
            const captcha = await solveCaptcha();
            const r = await api("/api/register", {
                username: fd.get("username"), password: fd.get("password"), nickname: fd.get("nickname"), captcha,
            });
            toast("注册成功", "ok");
            // Reset form and switch to login tab.
            form.reset();
            showTab("login");
        } catch (err) {
            clearFieldErrors(form);
            if (!mapBackendErrorToField(form, err.message)) {
                toast(err.message, "error");
            }
            if (btn) { btn.disabled = false; btn.textContent = "创建账号"; }
        }
    }

    // ─── lobby ────────────────────────────────────────────────────────
    async function renderLobby() {
        $("#topbar").classList.remove("hidden");
        $("#user-info").textContent = `${state.nickname} (#${state.uid})`;
        const app = $("#app");
        app.innerHTML = "";
        app.appendChild(el("h1", {}, "房间"));
        app.appendChild(el("p", { class: "subtitle" }, "创建或加入一个房间开始游戏。"));

        // Refresh game registry for the lobby dropdown.
        try {
            const { games } = await apiGet("/api/games");
            state.knownGames = new Map();
            for (const g of (games || [])) state.knownGames.set(g.type, g);
        } catch (e) { /* keep static fallback */ }

        const gameOptions = Array.from(state.knownGames.values()).map(g =>
            el("option", { value: g.type }, `${g.icon || ""} ${g.name || g.type}`)
        );
        if (gameOptions.length === 0) gameOptions.push(el("option", { value: "tictactoe" }, "井字棋"));

        const createCard = el("div", { class: "card" }, [
            el("h2", { style: "margin:0;font-size:15px;color:var(--muted)" }, "创建房间"),
            el("div", { class: "row" }, [
                el("select", { id: "game-type", onchange: onGameTypeChange }, gameOptions),
                el("div", { class: "spacer" }),
                el("button", { class: "primary", onclick: createRoom }, "创建"),
            ]),
            // Step-limit selection (only for TYP games).
            el("div", { id: "timer-row", class: "row hidden", style: "margin-top:10px" }, [
                el("label", { style: "margin-right:8px" }, "限时"),
                el("select", { id: "timer-preset" }, [
                    el("option", { value: "30+60" }, "快速（ 30+60 s ）"),
                    el("option", { value: "40+120", selected: true }, "标准（ 40+120 s ）"),
                    el("option", { value: "60+180" }, "宽松（ 60+180 s ）"),
                    el("option", { value: "300+0" }, "超长（ 300+0 s ）"),
                ]),
            ]),
        ]);
        app.appendChild(createCard);

        function onGameTypeChange() {
            const v = $("#game-type").value;
            $("#timer-row").classList.toggle("hidden", v !== "take_your_position");
        }
        onGameTypeChange();  // reflect initial value (TYP → show timer row)

        app.appendChild(el("h2", { style: "margin:24px 0 8px;font-size:15px;color:var(--muted)" }, "现有房间"));
        const list = el("div", { class: "room-list", id: "room-list" }, [renderRoomListSkeleton()]);
        app.appendChild(list);

        try {
            const data = await apiGet("/api/rooms");
            renderRoomList(data.rooms || []);
        } catch (e) {
            list.innerHTML = `<div class="status-bar error">${escapeHtml(e.message)}</div>`;
        }
    }

    function renderRoomListSkeleton() {
        const wrap = el("div", { class: "room-list" });
        for (let i = 0; i < 3; i++) {
            wrap.appendChild(el("div", { class: "room-item" }, [
                el("div", { class: "info" }, [
                    el("div", { class: "skeleton skel-row medium" }),
                    el("div", { class: "skeleton skel-row short" }),
                ]),
                el("div", { class: "skeleton skel-button" }),
            ]));
        }
        return wrap;
    }

    function renderRoomList(rooms) {
        const list = $("#room-list");
        list.innerHTML = "";
        if (rooms.length === 0) {
            list.appendChild(el("div", { class: "status-bar" }, "暂无房间，创建一个吧。"));
            return;
        }
        for (const r of rooms) {
            const meta = gameMeta(r.game_type);
            const players = r.players.map(p => p.nickname).join(", ") || "(空)";
            const item = el("div", { class: "room-item" }, [
                el("div", { class: "info" }, [
                    el("div", {}, [
                        el("strong", {}, `#${r.room_id}`),
                        " ",
                        el("span", { class: "game-icon game-icon--" + meta.icon + " room-list-icon", "aria-hidden": "true" }),
                        " ",
                        meta.name,
                        " ",
                        el("span", { class: "status-tag status-" + r.status }, statusLabel(r.status)),
                    ]),
                    el("div", { class: "meta" }, `玩家 ${r.players.length}/${r.max_players ?? "?"}: ${players}`),
                ]),
                el("button", { class: "primary", onclick: () => location.hash = "#room/" + r.room_id }, "进入"),
            ]);
            list.appendChild(item);
        }
    }

    async function createRoom() {
        const gt = $("#game-type").value;
        const body = { game_type: gt };
        const timerRow = $("#timer-row");
        if (timerRow && !timerRow.classList.contains("hidden")) {
            body.timer_preset = $("#timer-preset").value;
        }
        try {
            const r = await api("/api/rooms", body);
            toast("房间已创建", "ok");
            location.hash = "#room/" + r.room_id;
        } catch (e) { toast(e.message, "error"); }
    }

    // ─── room detail (live polling) ──────────────────────────────────
    function stopRoomPolling() {
        if (state.roomPollTimer) {
            clearInterval(state.roomPollTimer);
            state.roomPollTimer = null;
        }
        state.roomPollRoomId = 0;
    }

    function startRoomPolling(roomId) {
        stopRoomPolling();
        state.roomPollRoomId = roomId;
        state.roomPollTimer = setInterval(() => pollRoom(roomId), 2000);
    }

    async function pollRoom(roomId) {
        if (location.hash !== "#room/" + roomId) { stopRoomPolling(); return; }
        try {
            const r = await apiGet(`/api/rooms/${roomId}`);
            const prev = state.roomCache.get(roomId);
            detectPlayerChanges(prev, r);
            state.roomCache.set(roomId, r);
            saveRoomCacheToStorage();
            renderRoomDetail(r, { fromPoll: true });
            state.roomLastFetchAt = Date.now();
            const dot = $("#poll-dot");
            if (dot) dot.classList.remove("error");
        } catch (e) {
            const dot = $("#poll-dot");
            if (dot) dot.classList.add("error");
            console.warn("pollRoom failed", e);
        }
    }

    function detectPlayerChanges(prev, next) {
        if (!prev) return;
        const prevUids = new Set(prev.players.map(p => p.uid));
        const nextUids = new Set(next.players.map(p => p.uid));
        for (const p of next.players) {
            if (!prevUids.has(p.uid)) toast(`${p.nickname} 加入了`, "ok");
        }
        for (const p of prev.players) {
            if (!nextUids.has(p.uid) && p.uid !== state.uid) toast(`${p.nickname} 离开了`, "");
        }
        // host changed
        if (prev.host_uid !== next.host_uid) {
            if (next.host_uid === state.uid) toast("你现在是房主了", "ok");
        }
        // status transitions
        if (prev.status !== next.status) {
            const labels = {
                Waiting: "回到等待中",
                Starting: "游戏启动中…",
                Running: "游戏已开始",
                Finished: "本局结束",
                Destroyed: "房间已销毁",
            };
            toast(labels[next.status] || ("状态 → " + next.status), next.status === "Running" ? "ok" : "");
            // Auto-enter the game once it starts running (all players join).
            if (next.status === "Running" && next.current_instance_id) {
                const inRoom = next.players.some(p => p.uid === state.uid);
                if (inRoom) {
                    stopRoomPolling();
                    location.hash = "#game/" + next.current_instance_id;
                }
            }
        }
    }

    async function renderRoom(roomId) {
        stopRoomPolling();
        state.currentRoomId = roomId;
        localStorage.setItem("lobby_current_room_id", String(roomId));
        $("#topbar").classList.remove("hidden");
        $("#user-info").textContent = `${state.nickname} (#${state.uid})`;
        const app = $("#app");
        app.innerHTML = "";

        // Header
        const header = el("div", { class: "room-header" }, [
            el("button", { class: "ghost room-back", onclick: () => location.hash = "#lobby" }, "← 返回"),
            el("div", { class: "room-id" }, [
                el("span", { class: "muted" }, "房间"),
                " ",
                el("strong", { id: "room-id-num" }, "#" + roomId),
                " ",
                el("button", { class: "ghost icon-btn", title: "复制房间号", "aria-label": "复制房间号", onclick: () => copyRoomId(roomId) }, "复制"),
            ]),
            el("div", { class: "poll-indicator", id: "poll-dot-wrap", title: "实时同步中" }, [
                el("span", { id: "poll-dot", class: "poll-dot" }),
                el("span", { class: "muted", style: "font-size:11px" }, "live"),
            ]),
        ]);
        app.appendChild(header);

        const wrap = el("div", { id: "room-detail", class: "room-detail" });
        wrap.appendChild(renderRoomDetailSkeleton());
        app.appendChild(wrap);

        try {
            const r = await apiGet(`/api/rooms/${roomId}`);
            state.roomCache.set(roomId, r);
            saveRoomCacheToStorage();
            wrap.innerHTML = "";
            renderRoomDetail(r, { fromPoll: false });
            startRoomPolling(roomId);
        } catch (e) {
            wrap.innerHTML = "";
            wrap.appendChild(el("div", { class: "status-bar error" }, escapeHtml(e.message)));
        }
    }

    function renderRoomDetailSkeleton() {
        const wrap = el("div", {});
        // banner
        wrap.appendChild(el("div", { class: "room-banner" }, [
            el("div", { class: "game-icon skeleton" }),
            el("div", { class: "room-banner-text" }, [
                el("div", { class: "skeleton skel-row medium" }),
                el("div", { class: "skeleton skel-row long" }),
                el("div", { class: "skeleton skel-row long" }),
            ]),
        ]));
        // seats
        const seats = el("div", { class: "seat-grid" });
        for (let i = 0; i < 2; i++) seats.appendChild(el("div", { class: "seat skeleton skel-seat" }));
        const card = el("div", { class: "card players-card" }, [
            el("div", { class: "skeleton skel-row medium" }),
            seats,
        ]);
        wrap.appendChild(card);
        // actions
        wrap.appendChild(el("div", { class: "card room-actions" }, [
            el("div", { class: "skeleton skel-row medium" }),
            el("div", { class: "row", style: "margin-top:8px;gap:8px" }, [
                el("div", { class: "skeleton skel-button" }),
                el("div", { class: "skeleton skel-button" }),
            ]),
        ]));
        return wrap;
    }

    async function copyRoomId(roomId) {
        try {
            await navigator.clipboard.writeText(String(roomId));
            toast("房间号已复制", "ok");
        } catch (e) {
            toast("复制失败，请手动选择", "error");
        }
    }

    function renderRoomDetail(r, opts = {}) {
        const wrap = $("#room-detail");
        if (!wrap) return;
        wrap.innerHTML = "";

        const isHost = r.host_uid === state.uid;
        const playerUids = new Set(r.players.map(p => p.uid));
        const inRoom = playerUids.has(state.uid);
        const meta = gameMeta(r.game_type);
        const minP = r.min_players ?? 2;
        const maxP = r.max_players ?? 2;
        const need = Math.max(0, minP - r.players.length);

        // ── status banner ──────────────────────────────────────────
        const banner = el("div", { class: "room-banner" }, [
            el("div", { class: "game-icon game-icon--" + (meta.icon || "generic") }, [
                el("span", { class: "visually-hidden" }, meta.name || r.game_type),
            ]),
            el("div", { class: "room-banner-text" }, [
                el("div", { class: "room-banner-name" }, [
                    el("strong", {}, meta.name || r.game_type),
                    r.variant ? el("span", { class: "variant-tag" }, " · " + r.variant) : null,
                ]),
                el("div", { class: "room-banner-desc muted" }, meta.description || ""),
            ]),
            el("div", { class: "spacer" }),
            el("div", { class: "room-banner-status" }, [
                el("span", { class: "status-tag status-" + r.status, id: "room-status-tag" }, statusLabel(r.status)),
            ]),
        ]);
        wrap.appendChild(banner);

        // ── players ────────────────────────────────────────────────
        const playerBlock = el("div", { class: "card players-card" });
        playerBlock.appendChild(el("div", { class: "players-header" }, [
            el("strong", {}, "玩家"),
            el("span", { class: "muted" }, ` ${r.players.length} / ${maxP}`),
            need > 0 ? el("span", { class: "players-need muted" }, ` · 至少还需 ${need} 人`) : null,
        ]));

        const seats = el("div", { class: "seat-grid" });
        const taken = new Map();
        for (const p of r.players) taken.set(p.seat, p);

        for (let s = 0; s < maxP; s++) {
            const p = taken.get(s);
            if (p) {
                const isMe = p.uid === state.uid;
                const isH = p.uid === r.host_uid;
                seats.appendChild(el("div", { class: "seat seat-filled" + (isMe ? " seat-self" : "") }, [
                    el("div", { class: "seat-num" }, "座位 " + s),
                    el("div", { class: "seat-avatar" }, (p.nickname || "?").slice(0, 1).toUpperCase()),
                    el("div", { class: "seat-name" }, [
                        el("span", {}, p.nickname || "(无昵称)"),
                        isMe ? el("span", { class: "self-tag" }, "你") : null,
                        isH ? el("span", { class: "host-tag" }, "房主") : null,
                    ]),
                    el("div", { class: "seat-uid muted" }, "#" + p.uid),
                ]));
            } else {
                seats.appendChild(el("div", { class: "seat seat-empty" }, [
                    el("div", { class: "seat-num" }, "座位 " + s),
                    el("div", { class: "seat-avatar seat-avatar-empty" }, "·"),
                    el("div", { class: "seat-name muted" }, "等待玩家"),
                    el("div", { class: "seat-uid" }, ""),
                ]));
            }
        }
        playerBlock.appendChild(seats);
        wrap.appendChild(playerBlock);

        // ── actions ────────────────────────────────────────────────
        const actions = el("div", { class: "room-actions card" });
        renderRoomActions(actions, r, { isHost, inRoom, minP, maxP });
        wrap.appendChild(actions);

        // ── instance panel (Running / Finished) ────────────────────
        if ((r.status === "Running" || r.status === "Finished") && r.current_instance_id) {
            const inst = el("div", { class: "card instance-card" }, [
                el("div", { class: "instance-header" }, [
                    el("strong", {}, statusLabel(r.status)),
                    el("span", { class: "muted" }, ` · 实例 #${r.current_instance_id}`),
                ]),
                r.status === "Running" && inRoom
                    ? el("button", { class: "primary", onclick: () => location.hash = "#game/" + r.current_instance_id }, "进入游戏")
                    : null,
                r.status === "Finished" && isHost
                    ? el("button", { class: "primary", onclick: () => startGame(r.room_id) }, "再来一局")
                    : null,
                r.status === "Finished" && !isHost && inRoom
                    ? el("span", { class: "muted" }, "等待房主开下一局…")
                    : null,
            ]);
            wrap.appendChild(inst);
        }

        // ── info footer ────────────────────────────────────────────
        const footer = el("div", { class: "room-footer muted" }, [
            el("span", {}, "房主 UID: " + r.host_uid),
            " · ",
            el("span", {}, "游戏类型: " + r.game_type),
            r.variant ? " · " : "",
            r.variant ? el("span", {}, "variant: " + r.variant) : null,
        ]);
        wrap.appendChild(footer);

        if (!opts.fromPoll) {
            // initial: focus title
            const idEl = $("#room-id-num");
            if (idEl) idEl.textContent = "#" + r.room_id;
        }
    }

    function renderRoomActions(container, r, ctx) {
        const { isHost, inRoom, minP, maxP } = ctx;
        container.innerHTML = "";

        const banner = el("div", { class: "actions-hint muted" }, actionHint(r, ctx));
        container.appendChild(banner);

        const btns = el("div", { class: "actions-buttons" });

        // Waiting — not in room: join button
        if (r.status === "Waiting" && !inRoom) {
            const roomFull = r.players.length >= maxP;
            btns.appendChild(el("button", {
                class: "primary",
                "data-action": "join",
                disabled: roomFull,
                title: roomFull ? "房间已满" : "加入此房间",
                onclick: () => joinRoom(r.room_id),
            }, roomFull ? "房间已满" : "加入房间"));
        }

        // Waiting — in room: leave button (everyone), start button (host only)
        if (r.status === "Waiting" && inRoom) {
            if (isHost) {
                const ready = r.players.length >= minP;
                btns.appendChild(el("button", {
                    class: "primary",
                    "data-action": "start",
                    disabled: !ready,
                    title: ready ? "开始游戏" : `需要至少 ${minP} 名玩家`,
                    onclick: () => startGame(r.room_id),
                }, ready ? "开始游戏" : `等待玩家 (${r.players.length}/${minP})`));
            }
            btns.appendChild(el("button", {
                class: "ghost danger",
                "data-action": "leave",
                title: "离开房间（点击确认）",
                onclick: (ev) => confirmThen(ev.currentTarget, "确认离开？", () => leaveRoom(r.room_id)),
            }, "离开房间"));
        }

        // Starting — non-actionable hint
        if (r.status === "Starting") {
            btns.appendChild(el("span", { class: "muted" }, "游戏启动中…"));
        }

        // Running — handled by instance panel below (button there)
        // Finished — host can replay; non-host waits
        if (r.status === "Finished") {
            if (isHost) {
                btns.appendChild(el("button", {
                    class: "primary",
                    "data-action": "start",
                    onclick: () => startGame(r.room_id),
                }, "再来一局"));
            } else {
                btns.appendChild(el("span", { class: "muted" }, "等待房主开下一局…"));
            }
            btns.appendChild(el("button", {
                class: "ghost danger",
                "data-action": "leave",
                title: "离开房间（点击确认）",
                onclick: (ev) => confirmThen(ev.currentTarget, "确认离开？", () => leaveRoom(r.room_id)),
            }, "离开房间"));
        }

        // Destroyed — only show "back" hint
        if (r.status === "Destroyed") {
            btns.appendChild(el("span", { class: "muted" }, "房间已销毁"));
        }

        container.appendChild(btns);
    }

    function actionHint(r, ctx) {
        const { isHost, inRoom, minP } = ctx;
        if (r.status === "Waiting") {
            if (!inRoom) return "空闲中，加入即可参与。";
            if (isHost) {
                if (r.players.length < minP) return `你是房主，至少需要 ${minP} 名玩家才能开始。`;
                return "所有玩家已就位，可以开局了。";
            }
            return "等待房主开局…";
        }
        if (r.status === "Starting") return "正在 spawn 游戏进程…";
        if (r.status === "Running") return "游戏进行中。";
        if (r.status === "Finished") return "本局已结束。";
        if (r.status === "Destroyed") return "房间已销毁。";
        return "";
    }

    function statusLabel(s) {
        return ({
            Waiting: "等待中",
            Starting: "启动中",
            Running: "进行中",
            Finished: "已结束",
            Destroyed: "已销毁",
        })[s] || s;
    }

    async function joinRoom(id) {
        const btn = findActionButton("join");
        if (btn) { btn.disabled = true; btn.textContent = "加入中…"; }
        try {
            const r = await api(`/api/rooms/${id}/join`, {});
            toast("已加入房间", "ok");
            state.roomCache.set(id, r);
            saveRoomCacheToStorage();
            renderRoomDetail(r, { fromPoll: false });
        } catch (e) {
            toast(e.message, "error");
            if (btn) { btn.disabled = false; btn.textContent = "加入房间"; }
        }
    }

    async function leaveRoom(id) {
        const btn = findActionButton("leave");
        if (btn) { btn.disabled = true; btn.textContent = "离开中…"; }
        try {
            await api(`/api/rooms/${id}/leave`, {});
            toast("已离开", "ok");
            stopRoomPolling();
            location.hash = "#lobby";
        } catch (e) {
            toast(e.message, "error");
            if (btn) { btn.disabled = false; btn.textContent = "离开房间"; }
        }
    }

    async function startGame(id) {
        const btn = findActionButton("start");
        if (btn) { btn.disabled = true; btn.textContent = "启动中…"; }
        try {
            // Backend returns {instance_id, ws_url}, NOT a RoomInfo — do NOT
            // pass this to renderRoomDetail (it would crash on r.players).
            const r = await api(`/api/rooms/${id}/start`, {});
            toast("游戏已启动", "ok");
            // Host jumps straight into the game. Non-hosts stay on the room
            // page and click "进入游戏" once polling shows Running.
            setTimeout(() => { location.hash = "#game/" + r.instance_id; }, 600);
        } catch (e) {
            toast(e.message, "error");
            if (btn) { btn.disabled = false; btn.textContent = "开始游戏"; }
        }
    }

    function findActionButton(name) {
        const root = document.querySelector("#room-detail");
        return root ? root.querySelector(`button[data-action="${name}"]`) : null;
    }

    // Inline two-step confirmation: first click changes label/color + arms a 5s timer.
    // Second click within the window runs the action. Otherwise reverts.
    function confirmThen(btn, prompt, action) {
        if (!btn || btn.dataset.armed === "1") {
            if (btn) { btn.dataset.armed = ""; }
            return action();
        }
        btn.dataset.armed = "1";
        const original = btn.textContent;
        btn.textContent = prompt;
        btn.classList.add("confirm");
        if (btn._confirmTimer) clearTimeout(btn._confirmTimer);
        btn._confirmTimer = setTimeout(() => {
            btn.textContent = original;
            btn.classList.remove("confirm");
            btn.dataset.armed = "";
        }, 5000);
    }

    // ─── game (websocket) ──────────────────────────────────────────────
    let boardState = null;
    let myUid = state.uid;
    let gameWs = null;
    let gameOverNavTimer = null;  // set when modal shows, cleared on hide/click
    let cardSelectedIndex = -1;   // index in own hand selected for play_card
    let prevRenderedPhase = null; // last phase seen by renderCardBoard — used to
                                  // detect transitions into "play" so we can
                                  // clear the selection on a new round only.
    // Card-game UI state (TYP)
    let pendingPredictRank = undefined;  // 1..5 selected before confirm, null = 跳过, undefined = 未选
    let pendingPosteriorRanks = {};       // uid → 1..5 in-progress posterior assignment
    let typSnapshot = null;              // last snapshot for re-rendering from event handlers
    let shownRounds = new Set();          // round numbers already shown a summary banner for
    let shownPosteriorReveals = new Set();// rounds whose committed posterior we've already shown the reveal for
    let pendingGameOver = false;          // set when the 5th round ends; showGameOverForCard
                                          // fires after the user closes the round-5 summary
    // (round, uid) pairs whose first-time-commit "flash" highlight has already
    // been applied. The flash highlights the row in the posterior theme
    // yellow for 3 seconds so the user actually notices the rank changing
    // from "—" / "(待编辑)" to "第 N 名" / "未预测".
    let shownPosteriorFlash = new Set();
    // Posterior reveal scheduling: when a reveal pops, RoundResult events
    // that arrive during the 3-second window are queued and shown after.
    let revealActiveAt = 0;
    const roundSummaryQueue = [];

    function currentGameType() {
        // Persisted hint survives page refresh on #game/<id> — see lastGameType
        // in the state object. Without this, a refresh would route to the
        // tictactoe renderer because state.currentRoomId / roomCache are empty
        // until the user navigates back through the room page.
        if (state.lastGameType) return state.lastGameType;
        const rid = state.currentRoomId;
        if (!rid) return "tictactoe";
        const cached = state.roomCache.get(rid);
        return (cached && cached.game_type) || "tictactoe";
    }

    function renderGame(instanceId) {
        // Keep currentRoomId so the game-over modal can route back to #room/<id>.
        // If the user navigated directly to #game/<id> without going through a
        // room, this stays 0 and the modal falls back to #lobby.
        $("#topbar").classList.remove("hidden");
        $("#user-info").textContent = `${state.nickname} (#${state.uid})`;
        const app = $("#app");
        app.innerHTML = "";
        app.appendChild(el("h1", {}, "游戏"));
        app.appendChild(el("div", { class: "subtitle", id: "game-status" }, "连接中..."));

        const gt = currentGameType();
        // Persist so a later refresh on #game/<id> still routes to the
        // correct renderer. Cleared in the room / lobby navigation handlers
        // when the user leaves the game view.
        state.lastGameType = gt;
        localStorage.setItem("lobby_last_game_type", gt);
        if (gt === "take_your_position") {
            renderCardGameStage(instanceId);
        } else {
            renderTictactoeStage(instanceId);
        }
    }

    function renderTictactoeStage(instanceId) {
        const board = el("div", { class: "board hidden", id: "board" }, []);
        for (let i = 0; i < 9; i++) {
            const cell = el("div", { class: "cell", "data-i": String(i) });
            cell.addEventListener("click", () => onCellClick(i, cell));
            board.appendChild(cell);
        }
        const app = $("#app");
        app.appendChild(board);

        const log = el("div", { class: "event-log", id: "game-log", style: "margin-top:16px" }, "");
        app.appendChild(log);

        const wsHost = location.host;
        const url = `ws://${wsHost}/ws/${instanceId}`;
        gameLog(`连接 ${url}`);
        connectGame(url);
    }

    // ─── card game (take_your_position etc.) ────────────────────────
    // ─── card game (take_your_position) ──────────────────────────
    function renderCardGameStage(instanceId) {
        const app = $("#app");
        const stage = el("div", { class: "card-stage", id: "card-stage" });
        stage.appendChild(el("div", { class: "seat-tl",     id: "seat-tl" }));
        stage.appendChild(el("div", { class: "seat-tr",     id: "seat-tr" }));
        stage.appendChild(el("div", { class: "seat-l",      id: "seat-l" }));
        stage.appendChild(el("div", { class: "center-area", id: "center-area" }));
        stage.appendChild(el("div", { class: "seat-r",      id: "seat-r" }));
        stage.appendChild(el("div", { class: "seat-b",      id: "seat-b" }));
        app.appendChild(stage);
        app.appendChild(el("div", { class: "card-actions", id: "card-actions" }, ""));
        app.appendChild(el("div", { class: "hand", id: "card-hand" }, ""));
        app.appendChild(el("div", { class: "event-log", id: "game-log", style: "margin-top:16px" }, ""));

        // Reset ALL per-game state before the new instance's first snapshot
        // arrives. Without this, a second TYP in the same room inherits
        // stale values from the first: shownRounds / shownPosteriorReveals
        // would dedupe-skip the new round-0 summary, boardState / typSnapshot
        // could still carry the previous game's pending_events, the
        // 1-second countdown timer could keep ticking on stale data, and
        // the game-over modal could still be sitting on top blocking the
        // new round result popup.
        boardState = null;
        typSnapshot = null;
        cardSelectedIndex = -1;
        prevRenderedPhase = null;
        pendingPredictRank = undefined;
        pendingPosteriorRanks = {};
        shownRounds = new Set();
        shownPosteriorReveals = new Set();
        shownPosteriorFlash = new Set();
        revealActiveAt = 0;
        roundSummaryQueue.length = 0;
        pendingGameOver = false;
        if (typCountdownTimer) {
            clearInterval(typCountdownTimer);
            typCountdownTimer = null;
        }
        typSnapshotAt = Date.now();
        if (gameOverNavTimer) {
            clearTimeout(gameOverNavTimer);
            gameOverNavTimer = null;
        }
        // Hide any leftover game-over modal from the previous instance.
        const prevModal = document.getElementById("game-over-modal");
        if (prevModal) prevModal.classList.add("hidden");
        // Remove any leftover round-summary banner.
        const prevBanner = document.getElementById("round-summary-banner");
        if (prevBanner) prevBanner.remove();

        const wsHost = location.host;
        const url = `ws://${wsHost}/ws/${instanceId}`;
        gameLog(`连接 ${url}`);
        connectGame(url);
    }

    function renderCardBoard(s) {
        if (!s) return;
        typSnapshot = s;
        startTypCountdown(s);  // one-shot: live per-second re-render for countdowns
        const status = $("#game-status");
        const players = s.players || [];
        if (players.length === 0) return;

        const mySeatIdx = players.indexOf(myUid);
        if (mySeatIdx < 0) {
            // Not a player of this game (e.g. logged in as a different account,
            // or session/uid mismatch on reconnect). Tell the user instead of
            // leaving the status stuck on "已登录，等待快照".
            if (status) {
                status.textContent = `你(uid=${myUid})不在本局玩家列表 ${JSON.stringify(players)} 中`;
                status.className = "subtitle status-bar error";
            }
            return;
        }

        if (s.phase === "prior_prediction") {
            const myEntry = (s.predictions || []).find(([u]) => u === myUid);
            const myCommitted = myEntry && myEntry[1] !== null && myEntry[2];
            if (myCommitted) {
                pendingPredictRank = myEntry[1];
            } else if (s.current_player !== myUid) {
                pendingPredictRank = undefined;
            }
        } else if (s.phase === "posterior_prediction") {
            if (s.start_player !== myUid) pendingPosteriorRanks = {};
        } else if (s.phase === "play") {
            pendingPosteriorRanks = {};
            // Only reset the card selection when entering "play" from a
            // different phase (i.e. a new round). Do NOT reset on every
            // render — that path is hit on every snapshot AND every 1s
            // countdown tick AND inside the card click handler, and would
            // wipe the player's selection before they can press Confirm.
            if (prevRenderedPhase && prevRenderedPhase !== "play") {
                cardSelectedIndex = -1;
            }
        } else if (s.phase === "ended" || s.is_over) {
            pendingPosteriorRanks = {};
            pendingPredictRank = undefined;
            cardSelectedIndex = -1;
        }

        // Seat placement around the table. Self is always at the bottom.
        // Going counter-clockwise (the prediction order) from self: bottom → left
        // → top-left → top-right → right. So seat index decreases counter-clockwise.
        const positions = ["seat-b", "seat-l", "seat-tl", "seat-tr", "seat-r"];
        for (const id of positions) {
            const el_ = $("#" + id);
            if (el_) el_.innerHTML = "";
        }
        for (let phys = 0; phys < 5; phys++) {
            const seatIdx = (mySeatIdx - phys + 5) % 5;
            const uid = players[seatIdx];
            const container = $("#" + positions[phys]);
            if (!container) continue;
            // Tag the seat with the player uid so the per-second countdown
            // updater (updateTimeDisplays) can find the right time block
            // without rebuilding the entire board.
            container.dataset.uid = String(uid);
            container.appendChild(buildSeatPanel(s, seatIdx, uid, phys === 0));
        }

        renderCenterArea(s);
        renderOwnHand(s);
        renderActionPanel(s);
        prevRenderedPhase = s.phase;

        if (status) {
            if (s.phase === "waiting_all") {
                const joined = (s.online || []).length;
                const total = (s.players || []).length;
                status.textContent = `等待全部玩家连接（${joined}/${total}）`;
                status.className = "subtitle status-bar connected";
            } else if (s.is_over || s.phase === "ended") {
                status.textContent = "游戏结束";
                status.className = "subtitle status-bar";
            } else if (s.phase === "prior_prediction") {
                status.textContent = s.current_player === myUid
                    ? "轮到你：选择名次后点确认"
                    : `等待 ${getNickname(s.current_player)} 选择预测`;
                status.className = "subtitle status-bar connected";
            } else if (s.phase === "play") {
                const pending = (s.committed || []).filter(([_, c]) => c !== null).length;
                status.textContent = `出牌阶段（同时进行）— ${pending}/5 已提交`;
                status.className = "subtitle status-bar connected";
            } else if (s.phase === "posterior_prediction") {
                status.textContent = s.start_player === myUid
                    ? "你是首位玩家：编辑各玩家排名后点确认（或跳过）"
                    : `等待首位玩家 ${getNickname(s.start_player)} 提交后验预测`;
                status.className = "subtitle status-bar connected";
            }
        }

        if (s.is_over || s.phase === "ended") {
            // Defer the game-over screen until the round-5 summary closes.
            // Spec: "last round: show round summary first, then the game-over
            // page after the user dismisses the summary." For earlier rounds
            // this branch is never reached (phase never reaches "ended" until
            // round 5 finishes), so the flag is only ever set once.
            pendingGameOver = true;
        }

        // Set the reveal-before-round-summary gate BEFORE the events loop runs.
        // The trigger can't be `s.phase === "posterior_prediction"` because
        // the backend's action handler transitions to PriorPrediction **in
        // the same snapshot** that carries the posterior commitment (see
        // logic.rs advance_phase on commit). So by the time the snapshot
        // reaches the client, `s.phase` is already "prior_prediction" and
        // the RoundResult event is sitting in `pending_events` waiting to
        // fire. We anchor on the posterior commitment itself.
        //
        // The committed entry is on the FIRST player's (uid, list, bool)
        // tuple — only theirs has `committed = true`. We can't use
        // `s.start_player` to find it because `advance_phase` already
        // rotated `start_player` to the next round (so the committed tuple
        // belongs to the previous round's start_player, not the current
        // snapshot's). Match on the bool flag instead.
        if (Array.isArray(s.posterior)) {
            const postEntry = s.posterior.find(([, , committed]) => committed);
            if (postEntry) {
                if (!shownPosteriorReveals.has(s.round)) {
                    shownPosteriorReveals.add(s.round);
                    revealActiveAt = Date.now() + 3000;
                }
            }
        }

        const events = s.pending_events || [];
        events.forEach(ev => {
            gameLog("· " + JSON.stringify(ev));
            if (ev && ev.kind === "RoundResult") {
                // Show the banner once per round — pending_events repeats across
                // snapshots, so dedupe by round number. If a posterior reveal
                // is currently on screen, delay this banner by enough ms to
                // let the reveal finish first (the user sees "后验排名" for
                // 3 s, then the round result page).
                if (!shownRounds.has(ev.round)) {
                    shownRounds.add(ev.round);
                    const delay = Math.max(0, revealActiveAt - Date.now());
                    if (delay > 0) {
                        setTimeout(() => showRoundSummary(ev), delay);
                    } else {
                        showRoundSummary(ev);
                    }
                }
            }
        });
    }

    // Live countdown: track the last snapshot's wall-clock time so we can
    // decay each player's A+B locally every second without waiting for the
    // next snapshot (which only arrives on actions or timeouts).
    let typCountdownTimer = null;
    let typSnapshotAt = Date.now();
    function startTypCountdown(s) {
        // Reset the decay baseline only when a fresh server snapshot arrives
        // (this function is called from renderCardBoard). The per-second tick
        // must NOT reset it — otherwise `Date.now() - typSnapshotAt` is always
        // ~0 ms when buildTimeEl runs, so the time displays never tick down.
        typSnapshotAt = Date.now();
        if (typCountdownTimer) return;
        typCountdownTimer = setInterval(() => {
            if (!typSnapshot) return;
            if (typSnapshot.phase === "ended" || typSnapshot.phase === "waiting_all") {
                clearInterval(typCountdownTimer);
                typCountdownTimer = null;
                return;
            }
            // Only refresh the time display in each seat panel. The previous
            // implementation called the full renderCardBoard here, which
            // (a) wiped the player's in-progress card selection, and
            // (b) rebuilt every DOM element on every tick, causing flicker.
            // It also made per-client countdown visibly drift because each
            // client ran the re-render on its own local snapshot.
            updateTimeDisplays(typSnapshot);
        }, 1000);
    }

    // Refresh only the .seat-time blocks in each seat container. Seats are
    // looked up by data-uid (set in renderCardBoard) so we don't need to
    // recompute seat indices.
    function updateTimeDisplays(s) {
        const positions = ["seat-b", "seat-l", "seat-tl", "seat-tr", "seat-r"];
        for (const id of positions) {
            const seatEl = $("#" + id);
            if (!seatEl) continue;
            const uidAttr = seatEl.dataset.uid;
            if (!uidAttr) continue;
            const uid = parseInt(uidAttr, 10);
            if (Number.isNaN(uid)) continue;
            const oldTime = seatEl.querySelector(".seat-time");
            if (!oldTime) continue;
            const newTime = buildTimeEl(s, uid);
            if (newTime) oldTime.replaceWith(newTime);
        }
    }

    // Time display: A (refresh, white) + B (reserve, orange). Zero pools are
    // hidden and the '+' sign goes away. If both are present, '+' is orange.
    //
    // The snapshot gives each player `time_a_ms`, `time_b_ms` (the FULL pools,
    // before any current thinking deduction) and `remaining_ms` (the actual
    // remaining time at the moment of the snapshot, with the ongoing thinking
    // interval already deducted by the server). The previous implementation
    // applied `decayMs` to every player's remaining time uniformly — a player
    // watching the table would see their own clock tick down while another
    // player is deliberating, which is wrong: their clock is paused.
    //
    // The server's `start_thinking` only sets `thinking_since` for the
    // player(s) actually expected to act this step:
    //   - PriorPrediction: only `current_player`
    //   - Play: every player who hasn't yet committed a card (so a player
    //     who has already played should NOT see their clock keep ticking)
    //   - PosteriorPrediction: only `start_player`
    // So the client's `decayMs` (time since the snapshot arrived locally)
    // should only be subtracted for the active thinker(s). For everyone
    // else, the clock is paused at the snapshot's frozen value.
    //
    // Correct A/B split (A drains first, then B): from
    //   totalElapsed = aMs + bMs - remaining
    //   A_rem = max(0, aMs - totalElapsed)
    //   B_rem = max(0, remaining - A_rem)
    function buildTimeEl(s, uid) {
        const t = (s.times || []).find(([u]) => u === uid);
        if (!t) return null;
        let [, aFull, bFull, remInit] = t;
        const inPhase = s.phase === "prior_prediction" || s.phase === "play" || s.phase === "posterior_prediction";
        let aRem, bRem;
        if (inPhase) {
            let isActiveThinker;
            if (s.phase === "prior_prediction") {
                isActiveThinker = s.current_player === uid;
            } else if (s.phase === "play") {
                // All five players decide their card simultaneously — but
                // once a player has committed their card, the backend's
                // settle_action deducts elapsed and refills A, and their
                // thinking clock is effectively paused. Use `s.committed`
                // (per-player `(uid, card | null)`) to decide: a player
                // whose entry is `null` is still deciding; a player whose
                // entry is `Some(card)` has already locked in their move.
                const committedEntry = (s.committed || []).find(([u]) => u === uid);
                const hasCommitted = !!(committedEntry && committedEntry[1] != null);
                isActiveThinker = !hasCommitted;
            } else {
                // posterior_prediction
                isActiveThinker = s.start_player === uid;
            }
            const decayMs = isActiveThinker ? Math.max(0, Date.now() - typSnapshotAt) : 0;
            const remaining = Math.max(0, remInit - decayMs);
            const totalElapsed = (aFull + bFull) - remaining;
            aRem = Math.max(0, aFull - totalElapsed);
            bRem = Math.max(0, remaining - aRem);
        } else {
            aRem = aFull;
            bRem = bFull;
        }
        const aSecs = Math.ceil(aRem / 1000);
        const bSecs = Math.ceil(bRem / 1000);
        const wrap = el("div", { class: "seat-time" });
        if (aSecs > 0) {
            wrap.appendChild(el("span", { class: "time-a" }, `${aSecs}s`));
        }
        if (aSecs > 0 && bSecs > 0) {
            wrap.appendChild(el("span", { class: "time-plus" }, " + "));
        }
        if (bSecs > 0) {
            wrap.appendChild(el("span", { class: "time-b" }, `${bSecs}s`));
        }
        if (wrap.children.length === 0) {
            wrap.appendChild(el("span", { class: "time-a" }, "0s"));
        }
        return wrap;
    }

    function buildSeatPanel(s, seatIdx, uid, isSelf) {
        const data = readPlayerData(s, uid);
        const nickname = getNickname(uid) || `uid=${uid}`;
        const avatarChar = (nickname || "?").trim().charAt(0).toUpperCase() || "?";
        const avatarColor = colorFromUid(uid);

        const panel = el("div", { class: "seat-panel" + (isSelf ? " is-self" : "") });
        if (s.start_player === uid) panel.classList.add("is-first-player");
        if (s.current_player === uid) panel.classList.add("is-active");
        // Offline if `s.online` exists and doesn't include this uid.
        if (Array.isArray(s.online) && !s.online.includes(uid)) {
            panel.classList.add("is-offline");
        }

        const header = el("div", { class: "seat-panel-header" });
        header.appendChild(el("div", { class: "avatar", style: `background: ${avatarColor}` }, avatarChar));
        const info = el("div", { class: "seat-info" });
        info.appendChild(el("div", { class: "seat-name" }, nickname));
        info.appendChild(el("div", { class: "seat-score" }, `${data.score} 分`));
        header.appendChild(info);
        panel.appendChild(header);

        // Player time: refresh pool (white) + reserve pool (orange).
        // e.g. "30s + 60s"; hides a zero pool and its '+' sign.
        // Only show for the local player — other players' remaining time
        // is just noise here (the viewer can only act on their own clock).
        if (isSelf) {
            const timeEl = buildTimeEl(s, uid);
            if (timeEl) panel.appendChild(timeEl);
        }

        const preds = el("div", { class: "seat-predictions" });
        const priorText = data.predictionCommitted
            ? (data.prediction !== null && data.prediction !== undefined
                ? `第 ${data.prediction} 名`
                : "放弃")
            : "—";
        const priorCls = data.predictionCommitted ? "committed" : "";
        preds.appendChild(el("div", { class: "prior " + priorCls }, [
            el("span", { class: "label" }, "先验"),
            priorText,
        ]));
        // Posterior: rank is determined by the FIRST player's committed list
        // (it covers all 5 players). For every other player the snapshot's
        // `posterior` field is null — we have to look up the first player's
        // entry and find the index of `uid` in their list. Skip shows "未预测"
        // for everyone. Format mirrors the prior ("第 N 名") so the seat
        // panel reads like a single column; the .committed class picks up
        // the posterior yellow theme color.
        //
        // Find the committed entry by the `committed` flag, not by matching
        // `s.start_player` — the action handler transitions to the next
        // round before broadcasting, so the snapshot's `s.start_player` is
        // the NEXT round's start_player (committed=null), not the one who
        // just committed.
        const postEntry = (s.posterior || []).find(([, , committed]) => committed);
        const startCommitted = !!postEntry;
        const committedList = postEntry ? postEntry[1] : null;
        const postText = (() => {
            if (startCommitted) {
                if (!Array.isArray(committedList) || committedList.length === 0) return "未预测";
                const idx = committedList.indexOf(uid);
                return idx >= 0 ? `第 ${idx + 1} 名` : "—";
            }
            if (s.phase === "posterior_prediction" && s.start_player === uid) return "(待编辑)";
            return "—";
        })();
        const postCls = startCommitted ? "committed" : "";
        // 3-second highlight when the seat panel first shows the committed
        // rank. The first snapshot for a given (round, uid) gets the
        // `flash` class; subsequent snapshots don't. CSS animates the
        // background pulse and removes the highlight after 3 s.
        let flashCls = "";
        if (startCommitted) {
            const flashKey = s.round + ":" + uid;
            if (!shownPosteriorFlash.has(flashKey)) {
                shownPosteriorFlash.add(flashKey);
                flashCls = "flash";
            }
        }
        preds.appendChild(el("div", { class: "posterior " + postCls + (flashCls ? " " + flashCls : "") }, [
            el("span", { class: "label" }, "后验"),
            postText,
        ]));
        panel.appendChild(preds);

        // Posterior prediction rank input — shown on EVERY player's panel during
        // the posterior phase. Only the first player can edit; others read-only.
        if (s.phase === "posterior_prediction") {
            panel.appendChild(buildPosteriorRankRow(s, uid));
        }

        // The card this player played this round — displayed next to their
        // own role card. Owner sees face-up; others see face-down.
        if (s.phase === "play" && data.committed !== null && data.committed !== undefined) {
            const slot = el("div", { class: "committed-slot" });
            if (data.committed.hidden === false) {
                slot.appendChild(window.cardRender.renderCardEl({
                    s: data.committed.s, r: data.committed.r,
                }));
            } else {
                slot.appendChild(window.cardRender.renderCardEl(null, { faceDown: true }));
            }
            panel.appendChild(slot);
        }

        if (data.history && data.history.length > 0) {
            const hist = el("div", { class: "seat-history" });
            hist.appendChild(el("span", { class: "label" }, "已出:"));
            data.history.forEach(c => hist.appendChild(window.cardRender.renderCardInlineEl(c)));
            panel.appendChild(hist);
        }

        return panel;
    }

    function readPlayerData(s, uid) {
        const predEntry = (s.predictions || []).find(([u]) => u === uid);
        const commitEntry = (s.committed || []).find(([u]) => u === uid);
        const postEntry = (s.posterior || []).find(([u]) => u === uid);
        const scoreEntry = (s.scores || []).find(([u]) => u === uid);
        const histEntry = (s.history || []).find(([u]) => u === uid);
        return {
            prediction: predEntry ? predEntry[1] : null,
            predictionCommitted: !!(predEntry && predEntry[2]),
            committed: commitEntry ? commitEntry[1] : null,
            posterior: postEntry ? postEntry[1] : null,
            posteriorCommitted: !!(postEntry && postEntry[1] !== undefined && postEntry[2]),
            score: scoreEntry ? scoreEntry[1] : 0,
            history: histEntry ? histEntry[1] : [],
        };
    }

    function buildPosteriorRankRow(s, forUid) {
        // Free-rank assignment UI: a 5x5 grid where the row is the player
        // and the columns are ranks 1..5. Clicking a cell pins that player
        // to that rank; clicking the same cell again unpins. Any other
        // player's pin for the same rank is replaced (the system enforces
        // "no duplicates" by construction here; the backend re-validates).
        //
        // State lives server-side as a dict `{ uid: rank }` in
        // `s.posterior_draft`. The first player can pick any rank for any
        // player in any order — there's no "fill in order" constraint any
        // more.
        const isEditor = s.start_player === myUid;
        const row = el("div", { class: "posterior-row" });
        // Defensive: backend may send a stale list (older binary) or a fresh
        // dict. Coerce to dict for the rendering code below.
        const draft = coercePosteriorDraft(s.posterior_draft);
        const myRank = draft[forUid] || 0;
        for (let r = 1; r <= 5; r++) {
            const btn = el("button", { class: "rank-mini" }, String(r));
            if (myRank === r) btn.classList.add("assigned");
            if (!isEditor) {
                btn.disabled = true;
                btn.title = "";
                row.appendChild(btn);
                continue;
            }
            // Disable rank r if it's already pinned to ANOTHER player. The
            // player who currently holds rank r can still click it (toggle
            // off). Disabled-state tooltip explains why.
            const occupiedByOther = Object.entries(draft).some(([u, rank]) =>
                Number(u) !== forUid && Number(rank) === r);
            btn.disabled = occupiedByOther;
            if (btn.disabled) {
                btn.title = `第 ${r} 名已被其他玩家占用`;
            }
            btn.addEventListener("click", () => {
                if (btn.disabled) return;
                const next = { ...draft };
                if (myRank === r) {
                    delete next[forUid];
                } else {
                    next[forUid] = r;
                }
                sendCardAction({ action: "draft_posterior", assignments: next });
                renderCardBoard(typSnapshot);
            });
            row.appendChild(btn);
        }
        return row;
    }

    // Normalise whatever shape `s.posterior_draft` arrived in (a dict from
    // the current backend, or — during a binary skew / rollout — a stale
    // flat array from an older build) into `{ uid: rank }`.
    function coercePosteriorDraft(raw) {
        const out = {};
        if (raw && typeof raw === "object" && !Array.isArray(raw)) {
            for (const [k, v] of Object.entries(raw)) {
                const uid = parseInt(k, 10);
                const r = Number(v);
                if (!Number.isNaN(uid) && !Number.isNaN(r) && r >= 1) out[uid] = r;
            }
        } else if (Array.isArray(raw)) {
            raw.forEach((uid, idx) => {
                const u = parseInt(uid, 10);
                if (!Number.isNaN(u)) out[u] = idx + 1;
            });
        }
        return out;
    }

    function draftToMap(draft) {
        const m = {};
        draft.forEach((uid, idx) => { m[uid] = idx + 1; });
        return m;
    }

    function renderCenterArea(s) {
        const center = $("#center-area");
        if (!center) return;
        center.innerHTML = "";
        const players = s.players || [];
        const mySeatIdx = players.indexOf(myUid);
        if (mySeatIdx < 0) return;

        // 5 slots: one per seat, positioned relative to my seat.
        //   rel 0 = bottom, 1 = right, 2 = top-right, 3 = top-left, 4 = left
        const tbl = el("div", { class: "center-table" });

        if (s.phase === "play" || s.phase === "posterior_prediction") {
            // Cards stay face-down on the table during BOTH the play phase and
            // the posterior phase (reveal happens only when the round scores).
            // The owner sees their own card face-up so they can confirm what
            // they played; everyone else sees face-down.
            const committedMap = new Map(s.committed || []);
            for (let i = 0; i < players.length; i++) {
                const uid = players[i];
                // rel must mirror the seat-panel direction (counter-clockwise
                // from self in `players[]` order): the player one seat to the
                // left of self in the panel must have their card on the bottom-
                // left of the table (data-pos=1), not the bottom-right (data-
                // pos=4). The previous `(i - mySeatIdx + 5) % 5` went the
                // opposite way and put every non-self card on the wrong side.
                const rel = (mySeatIdx - i + 5) % 5;
                const committed = committedMap.get(uid);
                let card;
                if (committed && committed.hidden === false) {
                    // Own card: face-up
                    card = window.cardRender.renderCardEl({ s: committed.s, r: committed.r });
                } else if (committed) {
                    // Others' cards, or own card before fix: face-down
                    card = window.cardRender.renderCardEl(null, { faceDown: true });
                } else {
                    card = window.cardRender.renderCardEl(null);
                }
                tbl.appendChild(el("div", { class: "center-slot", "data-pos": String(rel) }, [
                    card,
                    el("div", { class: "slot-name" },
                        uid === myUid ? "你" : (getNickname(uid) || `P${i}`)),
                ]));
            }
        } else if (s.phase === "ended" || s.is_over) {
            // Revealed cards (round already scored at End).
            // `table` is now keyed by uid (per backend change).
            const tableMap = new Map(s.table || []);
            for (let i = 0; i < players.length; i++) {
                const uid = players[i];
                // Same direction as the seat panels and the play-phase slot
                // loop above — see comment there.
                const rel = (mySeatIdx - i + 5) % 5;
                const cardData = tableMap.get(uid);
                const card = cardData
                    ? window.cardRender.renderCardEl({ s: cardData.s, r: cardData.r })
                    : window.cardRender.renderCardEl(null);
                tbl.appendChild(el("div", { class: "center-slot", "data-pos": String(rel) }, [
                    card,
                    el("div", { class: "slot-name" },
                        uid === myUid ? "你" : (getNickname(uid) || `P${i}`)),
                ]));
            }
        }

        center.appendChild(tbl);
    }

    function renderOwnHand(s) {
        const handEl = $("#card-hand");
        if (!handEl) return;
        handEl.innerHTML = "";
        if (!s.hand) return;
        const canPlay = s.phase === "play" && !s.is_over;
        s.hand.forEach((c, idx) => {
            // Clicking selects the card (no immediate send); a confirm button in
            // the action panel then submits play_card. Selected card is raised.
            const card = window.cardRender.renderCardEl({ s: c.s, r: c.r }, {
                clickable: canPlay,
                selected: cardSelectedIndex === idx,
                onClick: () => {
                    if (cardSelectedIndex === idx) cardSelectedIndex = -1;
                    else cardSelectedIndex = idx;
                    // Re-render only the hand + action panel — the full
                    // renderCardBoard would rebuild every seat panel, which
                    // (a) is expensive, (b) destroys DOM nodes that may be in
                    // the middle of being interacted with, and (c) re-enters
                    // the play-phase branch that resets selection state.
                    renderOwnHand(typSnapshot);
                    renderActionPanel(typSnapshot);
                },
            });
            handEl.appendChild(card);
        });
    }

    function renderActionPanel(s) {
        const actionsEl = $("#card-actions");
        if (!actionsEl) return;
        actionsEl.innerHTML = "";

        if (s.is_over || s.phase === "ended") {
            actionsEl.appendChild(buildRestartVotePanel(s));
            return;
        }
        if (s.phase === "prior_prediction") {
            if (s.current_player === myUid) {
                actionsEl.appendChild(buildPredictPanel(s));
            } else {
                actionsEl.appendChild(el("div", { class: "subtitle" },
                    `等待 ${getNickname(s.current_player)} 预测中…`));
            }
            return;
        }
        if (s.phase === "play") {
            const committed = (s.committed || []).filter(([_, c]) => c !== null).length;
            const group = el("div", { class: "group" }, [
                el("span", { class: "subtitle" },
                    `出牌阶段 — ${committed}/5 已提交`),
            ]);
            if (committed < (s.players || []).length) {
                // Confirm button: enabled once a hand card is selected.
                const sel = cardSelectedIndex >= 0 ? cardSelectedIndex : null;
                const confirm = el("button", { class: "primary confirm", disabled: sel === null },
                    sel === null ? "先点选一张手牌" : `确认出第 ${sel + 1} 张`);
                confirm.addEventListener("click", () => {
                    if (cardSelectedIndex < 0) return;
                    sendCardAction({ action: "play_card", card_index: cardSelectedIndex });
                    cardSelectedIndex = -1;
                });
                group.appendChild(confirm);
            }
            actionsEl.appendChild(group);
            return;
        }
        if (s.phase === "posterior_prediction") {
            if (s.start_player === myUid) {
                actionsEl.appendChild(buildPosteriorActions(s));
            } else {
                actionsEl.appendChild(el("div", { class: "subtitle" },
                    `等待首位玩家 ${getNickname(s.start_player)} 提交后验预测`));
            }
        }
    }

    function buildPredictPanel(s) {
        const group = el("div", { class: "group" }, [el("label", {}, "选择名次：")]);
        const myEntry = (s.predictions || []).find(([u]) => u === myUid);
        // has_predicted (entry[2]) is the authoritative "已提交" flag — a skipped
        // player has prediction=null but has_predicted=true.
        const myCommitted = myEntry && myEntry[2];

        if (myCommitted) {
            group.appendChild(el("span", { style: "color:#4ade80;font-weight:700" },
                myEntry[1] !== null
                    ? `已预测第 ${myEntry[1]} 名（不可更改）`
                    : "已放弃预测（不可更改）"));
            return group;
        }

        for (let r = 1; r <= 5; r++) {
            const btn = el("button", { class: "rank-pick" + (pendingPredictRank === r ? " selected" : "") }, String(r));
            btn.addEventListener("click", () => {
                pendingPredictRank = r;
                renderActionPanel(typSnapshot);
            });
            group.appendChild(btn);
        }
        const skipBtn = el("button", {
            class: "rank-pick skip" + (pendingPredictRank === null ? " selected" : ""),
        }, "放弃");
        // 放弃 = 选择"不预测"作为输入，仍需点确认提交。
        skipBtn.addEventListener("click", () => {
            pendingPredictRank = null;
            renderActionPanel(typSnapshot);
        });
        group.appendChild(skipBtn);

        const confirm = el("button", { class: "primary confirm" }, "确认");
        confirm.disabled = (pendingPredictRank === undefined);
        confirm.addEventListener("click", () => {
            if (pendingPredictRank === undefined) return;
            sendCardAction({ action: "predict", rank: pendingPredictRank });
        });
        group.appendChild(confirm);

        return group;
    }

    function buildPosteriorActions(s) {
        const group = el("div", { class: "group" }, [el("label", {}, "后验预测：")]);

        const myEntry = (s.posterior || []).find(([u]) => u === myUid);
        const myCommitted = myEntry && myEntry[1] !== undefined && myEntry[2];
        if (myCommitted) {
            group.appendChild(el("span", { style: "color:#facc15;font-weight:700" }, "已提交"));
            return group;
        }

        // "Upload" is enabled only when every player is pinned to a distinct
        // rank (5 unique ranks covering all 5 players). The dict→list
        // conversion happens at click time, not on every keystroke, so the
        // UI doesn't churn while the first player is still picking.
        const draft = coercePosteriorDraft(s.posterior_draft);
        const players = s.players || [];
        const allPinned = players.length > 0 && players.every(uid => draft[uid] != null);
        const ranksUsed = Object.values(draft);
        const allUnique = new Set(ranksUsed).size === ranksUsed.length;
        const ready = allPinned && allUnique && ranksUsed.length === players.length;

        const confirm = el("button", { class: "primary confirm" }, "上传");
        confirm.disabled = !ready;
        confirm.title = ready ? "" : "需要给所有玩家各分配一个不重复的名次";
        confirm.addEventListener("click", () => {
            if (!ready) return;
            // Convert the dict to a best→worst list (rank 1 first, rank n last)
            // for the wire — the backend still validates the list strictly.
            const list = [];
            for (let r = 1; r <= players.length; r++) {
                const uid = Object.entries(draft).find(([_, rank]) => Number(rank) === r)?.[0];
                if (uid) list.push(parseInt(uid, 10));
            }
            sendCardAction({ action: "posterior_predict", rank_list: list });
        });
        group.appendChild(confirm);

        const skip = el("button", {}, "跳过");
        skip.addEventListener("click", () => {
            sendCardAction({ action: "posterior_predict", rank_list: [] });
        });
        group.appendChild(skip);

        return group;
    }

    // Pop a centered 3-second overlay listing the first player's committed
    // posterior prediction (best → worst). Triggered from renderCardBoard on
    // the transition into "posterior_prediction committed" within a round.
    function showPosteriorReveal(rankList) {
        const overlay = el("div", { class: "posterior-reveal-overlay" }, [
            el("div", { class: "posterior-reveal" }, [
                el("div", { class: "posterior-reveal-title" }, "后验预测"),
                el("ol", { class: "posterior-reveal-list" },
                    rankList.map((uid, idx) =>
                        el("li", { class: "posterior-reveal-item" }, [
                            el("span", { class: "rank-num" }, String(idx + 1)),
                            el("span", { class: "rank-name" }, getNickname(uid) || `uid=${uid}`),
                        ])
                    )
                ),
                el("div", { class: "posterior-reveal-hint" }, "3 秒后自动关闭"),
            ]),
        ]);
        document.body.appendChild(overlay);
        setTimeout(() => overlay.remove(), 3000);
    }

    function buildRestartVotePanel(s) {
        const group = el("div", { class: "group" }, [
            el("label", {}, "再来一局？"),
        ]);
        const yes = el("button", { class: "primary" }, "同意");
        yes.addEventListener("click", () => sendCardAction({ action: "restart_vote", yes: true }));
        const no = el("button", {}, "否决");
        no.addEventListener("click", () => sendCardAction({ action: "restart_vote", yes: false }));
        group.appendChild(yes); group.appendChild(no);
        return group;
    }

    function sendCardAction(actionObj) {
        if (!gameWs || gameWs.readyState !== 1) return;
        gameWs.send(JSON.stringify({ type: "game", data: actionObj }));
    }

    function getNickname(uid) {
        const rid = state.currentRoomId;
        if (!rid) return null;
        const room = state.roomCache.get(rid);
        if (!room) return null;
        const p = (room.players || []).find(pp => pp.uid === uid);
        return p ? p.nickname : null;
    }

    function colorFromUid(uid) {
        const h = ((uid * 137) % 360 + 360) % 360;
        return `hsl(${h}, 55%, 45%)`;
    }

    function showRoundSummary(ev) {
        // Centered, text-based round result with a per-player score breakdown.
        const prev = document.getElementById("round-summary-banner");
        if (prev) prev.remove();
        const wrap = el("div", { class: "round-summary", id: "round-summary-banner" });
        wrap.appendChild(el("div", { class: "round-summary-title" }, `第 ${(ev.round || 0) + 1} 轮结算`));

        // Header row (columns).
        const head = el("div", { class: "rs-row rs-head" }, [
            el("span", { class: "rs-rank" }, "名次"),
            el("span", { class: "rs-name" }, "玩家"),
            el("span", { class: "rs-card" }, "出牌"),
            el("span", { class: "rs-num" }, "排序分"),
            el("span", { class: "rs-num" }, "先验分"),
            el("span", { class: "rs-num" }, "后验分"),
            el("span", { class: "rs-num" }, "本轮总"),
        ]);
        wrap.appendChild(head);

        const score_delta = ev.score_delta || [];
        const rank_score = ev.rank_score || [];
        const prediction_score = ev.prediction_score || [];
        const posterior_score = ev.posterior_score || [];
        // Map uid → its row index in score_* arrays (aligned by seat, not by rank).
        // score arrays are indexed by seat; ranking is ordered best→worst.
        const playersOrder = (typSnapshot && typSnapshot.players) || [];
        const seatOfUid = new Map(playersOrder.map((uid, i) => [uid, i]));

        (ev.ranking || []).forEach((uid, i) => {
            const seat = seatOfUid.get(uid);
            const safe = (arr, dflt) => seat != null && seat < arr.length ? arr[seat] : dflt;
            const rs = safe(rank_score, 0);
            const ps = safe(prediction_score, 0);
            const pos = safe(posterior_score, 0);
            const total = safe(score_delta, rs + ps + pos);

            // cards are [suit, rank] integer pairs aligned with ranking.
            const cardPair = (ev.cards || [])[i];
            const cardText = cardPair
                ? window.cardRender.renderCardInline({ s: cardPair[0], r: cardPair[1] })
                : "?";
            const cardColor = cardPair ? (cardPair[0] === 1 || cardPair[0] === 2 ? "red" : "blue") : "red";
            const row = el("div", { class: "rs-row" + (uid === myUid ? " rs-me" : "") }, [
                el("span", { class: "rs-rank" }, String(i + 1)),
                el("span", { class: "rs-name" }, uid === myUid ? "你" : (getNickname(uid) || `uid=${uid}`)),
                el("span", { class: "rs-card rs-card-" + cardColor }, cardText),
                el("span", { class: "rs-num" }, fmtDelta(rs)),
                el("span", { class: "rs-num" }, fmtDelta(ps)),
                el("span", { class: "rs-num" }, fmtDelta(pos)),
                el("span", { class: "rs-num rs-total" }, fmtDelta(total)),
            ]);
            wrap.appendChild(row);
        });

        // Close button — dismiss the modal manually (otherwise auto-closes in 15s).
        // If this is the round-5 summary (game over pending), reveal the final
        // scores AFTER the user has had a chance to read the round result.
        const finishIfPending = () => {
            if (wrap.parentNode) wrap.remove();
            if (pendingGameOver) {
                pendingGameOver = false;
                if (typSnapshot) showGameOverForCard(typSnapshot);
            }
        };
        const close = el("button", { class: "rs-close" }, "关闭");
        close.addEventListener("click", finishIfPending);
        wrap.appendChild(close);

        document.body.appendChild(wrap);
        // Longer display, centered; user can close earlier with the button.
        setTimeout(finishIfPending, 15000);
    }

    function fmtDelta(v) {
        if (v === 0) return "0";
        return (v > 0 ? "+" : "") + v;
    }

    function showGameOverForCard(s) {
        const modal = $("#game-over-modal");
        if (!modal) return;
        if (!modal.classList.contains("hidden")) return;
        const title = $("#game-over-title");
        const detail = $("#game-over-detail");
        const finalBoard = $("#game-over-final");
        if (title) title.textContent = "游戏结束";
        if (detail) {
            const final = s.pending_events && s.pending_events.find(e => e.kind === "GameEnded");
            const scores = (final && final.final_scores) || (s.scores || []);
            const ranked = scores.slice().sort((a, b) => b[1] - a[1]);
            const me = ranked.find(([u]) => u === myUid);
            if (me) {
                const myRank = ranked.indexOf(me) + 1;
                detail.textContent = `你的最终名次：第 ${myRank} / ${ranked.length}（${me[1]} 分）`;
            } else {
                detail.textContent = "";
            }
        }
        if (finalBoard) {
            finalBoard.innerHTML = "";
            // Use a vertical ranked list (one row per player, sorted by
            // score high→low). The .cell class is shared with tictactoe's
            // 3×3 grid; the .final-rank-* classes below only apply to TYP
            // and lay things out as a single column.
            finalBoard.classList.add("final-rank-list");
            const final = s.pending_events && s.pending_events.find(e => e.kind === "GameEnded");
            const scores = (final && final.final_scores) || (s.scores || []);
            const ranked = scores
                .slice()
                .sort((a, b) => b[1] - a[1]);
            if (ranked.length === 0) {
                // Fallback: derive from s.players / s.scores if no GameEnded
                // event arrived (e.g. lobby restarted the snapshot).
                (s.players || []).forEach(uid => {
                    const sc = (s.scores || []).find(([u]) => u === uid);
                    ranked.push([uid, sc ? sc[1] : 0]);
                });
                ranked.sort((a, b) => b[1] - a[1]);
            }
            ranked.forEach(([uid, score], idx) => {
                const isMe = uid === myUid;
                const row = el("div",
                    { class: "cell final-rank-row" + (isMe ? " final-rank-me" : "") },
                    [
                        el("span", { class: "final-rank-num" }, String(idx + 1)),
                        el("span", { class: "final-rank-name" }, isMe ? "你" : (getNickname(uid) || `uid=${uid}`)),
                        el("span", { class: "final-rank-score" }, `${score} 分`),
                    ]);
                finalBoard.appendChild(row);
            });
        }
        modal.classList.remove("hidden");
        const roomId = state.currentRoomId;
        if (roomId && !gameOverNavTimer) {
            gameOverNavTimer = setTimeout(() => {
                if (location.hash.startsWith("#game/")) location.hash = `#room/${roomId}`;
                gameOverNavTimer = null;
            }, 5000);
        }
    }

    function gameLog(line) {
        const log = $("#game-log");
        if (!log) return;
        const ts = new Date().toLocaleTimeString();
        // Append one small div per entry instead of growing a giant text node.
        const entry = el("div", { class: "log-entry" }, `[${ts}] ${line}`);
        log.appendChild(entry);
        // Cap the log so the DOM doesn't grow unbounded across a long game.
        while (log.children.length > 300) {
            log.removeChild(log.firstChild);
        }
        log.scrollTop = log.scrollHeight;
    }

    function connectGame(url) {
        if (gameWs) try { gameWs.close(); } catch {}
        gameWs = new WebSocket(url);
        gameWs.onopen = () => {
            gameLog("已连接，等待登录");
            gameWs.send(JSON.stringify({ type: "login", uid: myUid, session: state.token }));
        };
        gameWs.onmessage = (ev) => {
            let msg; try { msg = JSON.parse(ev.data); } catch { gameLog("RAW: " + ev.data); return; }
            handleGameMessage(msg);
        };
        gameWs.onerror = (e) => {
            $("#game-status").textContent = "连接错误";
            $("#game-status").className = "subtitle status-bar error";
            gameLog("错误: " + e);
        };
        gameWs.onclose = () => {
            $("#game-status").textContent = "已断开";
            $("#game-status").className = "subtitle status-bar";
            gameLog("已断开");
        };
    }

    function handleGameMessage(msg) {
        gameLog("← " + JSON.stringify(msg));
        const status = $("#game-status");
        switch (msg.type) {
            case "login_ok":
                status.textContent = "已登录，等待快照";
                status.className = "subtitle status-bar connected";
                break;
            case "snapshot":
                boardState = msg.state;
                if (currentGameType() === "take_your_position") {
                    try {
                        renderCardBoard(msg.state);
                    } catch (e) {
                        // Don't leave the UI stuck on "已登录，等待快照" forever —
                        // surface the real error so it's diagnosable.
                        console.error("renderCardBoard error", e);
                        toast("渲染游戏界面出错: " + e.message, "error");
                        if (status) { status.textContent = "渲染出错"; status.className = "subtitle status-bar error"; }
                    }
                } else {
                    const boardEl = $("#board");
                    if (boardEl) boardEl.classList.remove("hidden");
                    renderBoard();
                    if (boardState.phase === "finished") {
                        showGameOver(boardState);
                    } else {
                        status.textContent = boardState.phase === "playing" ? "游戏进行中" : "等待开始";
                        status.className = "subtitle status-bar connected";
                    }
                }
                break;
            case "game":
                if (msg.data && msg.data.state) {
                    boardState = msg.data.state;
                    if (currentGameType() === "take_your_position") {
                        renderCardBoard(msg.state);
                    } else {
                        renderBoard();
                        if (boardState.phase === "finished") {
                            showGameOver(boardState);
                        }
                    }
                }
                break;
            case "game_error":
                toast(msg.message, "error");
                break;
            case "error":
                toast(`${msg.code}: ${msg.message}`, "error");
                break;
        }
    }

    function renderBoard() {
        if (!boardState) return;
        const cells = $$("#board .cell");
        const board = boardState.board || [];
        for (let i = 0; i < 9; i++) {
            const v = board[i];
            const c = cells[i];
            c.textContent = v && v !== 0 ? seatMark(v) : "";
            c.className = "cell";
            if (v === myUid) c.classList.add("x");
            else if (v && v !== 0) c.classList.add("o");
            if (boardState.phase !== "playing" || boardState.turn !== myUid || v) c.classList.add("disabled");
        }
        const status = $("#game-status");
        if (boardState.phase === "playing") {
            status.textContent = boardState.turn === myUid ? "轮到你" : `等待 uid=${boardState.turn} 操作`;
        }
    }

    function showGameOver(state) {
        const modal = $("#game-over-modal");
        if (!modal) return;
        const title = $("#game-over-title");
        const detail = $("#game-over-detail");
        const finalBoard = $("#game-over-final");
        const againBtn = $("#game-over-again");
        const seatLabel = (uid) => {
            const idx = (state.players || []).indexOf(uid);
            return idx === 0 ? "X" : (idx === 1 ? "O" : `uid=${uid}`);
        };
        if (state.winner) {
            const iWon = state.winner === myUid;
            title.textContent = iWon ? "胜利！" : "失败";
            title.className = iWon ? "win" : "lose";
            const winnerSeat = seatLabel(state.winner);
            detail.textContent = iWon
                ? `${winnerSeat}（你）获胜`
                : `${winnerSeat} 获胜`;
        } else {
            title.textContent = "平局";
            title.className = "draw";
            detail.textContent = "棋盘已满，势均力敌";
        }
        // Final board snapshot.
        finalBoard.innerHTML = "";
        const board = state.board || [];
        for (let i = 0; i < board.length; i++) {
            const v = board[i];
            const cell = el("div", { class: "cell" }, "");
            if (v && v !== 0) {
                cell.textContent = seatLabel(v);
                cell.classList.add(v === myUid ? "x" : "o");
            }
            finalBoard.appendChild(cell);
        }
        // "再来一局" button is only meaningful for the host of the current
        // room. We need currentRoomId (set by renderRoom) and host_uid (from
        // roomCache). The host is identified by uid match, not by the WS.
        const roomId = state.currentRoomId;
        const cached = roomId ? state.roomCache.get(roomId) : null;
        const isHost = !!(cached && cached.host_uid === myUid);
        if (againBtn) {
            againBtn.classList.toggle("hidden", !(isHost && roomId));
            againBtn.disabled = false;
        }
        modal.classList.remove("hidden");
        // Stop accepting further moves on the underlying board.
        try { gameWs && gameWs.close(); } catch {}

        // Auto-navigate to #room/<id> after a few seconds if the user does
        // not interact. The room stays alive (multi-game design) so the user
        // lands on the room page, ready for the next round.
        if (roomId && !gameOverNavTimer) {
            gameOverNavTimer = setTimeout(() => {
                if (location.hash.startsWith("#game/")) {
                    location.hash = `#room/${roomId}`;
                }
                gameOverNavTimer = null;
            }, 5000);
        }
    }

    function hideGameOver() {
        const modal = $("#game-over-modal");
        if (modal) modal.classList.add("hidden");
        if (gameOverNavTimer) { clearTimeout(gameOverNavTimer); gameOverNavTimer = null; }
    }

    function seatMark(uid) {
        // For tictactoe: seat 0 = X, seat 1 = O.
        // We use plain Latin letters + CSS coloring (see .cell.x / .cell.o),
        // NOT emoji ✕ / ○ — see docs/frontend-design.md §3.
        if (!boardState || !boardState.players) return "?";
        const idx = boardState.players.indexOf(uid);
        return idx === 0 ? "X" : "O";
    }

    function onCellClick(i, cellEl) {
        if (!boardState || boardState.phase !== "playing") return;
        if (boardState.turn !== myUid) { toast("不是你的回合", "error"); return; }
        if (boardState.board[i]) return;
        if (!gameWs || gameWs.readyState !== 1) return;
        gameWs.send(JSON.stringify({ type: "game", data: { action: "move", cell: i } }));
        cellEl.classList.add("disabled");
    }

    // ─── router ───────────────────────────────────────────────────────
    function render() {
        const hash = location.hash || "#login";
        // Capture the route name in m[1] and the optional numeric id in m[2].
        // m[1] must be the bare segment ("room"/"game"/...) so the switch works.
        const m = hash.match(/^#(login|register|lobby|room|game)(?:\/(\d+))?$/);
        if (!m) { location.hash = state.token ? "#lobby" : "#login"; return; }
        if (state.token && (hash === "#login" || hash === "#register" || hash === "")) {
            location.hash = "#lobby"; return;
        }
        if (!state.token && (hash !== "#login" && hash !== "#register")) {
            location.hash = "#login"; return;
        }
        // Stop room polling when leaving the room view.
        if (m[1] !== "room" && state.roomPollTimer) {
            stopRoomPolling();
        }
        // Once we leave the game view (#game/<id>), the persisted hint is no
        // longer authoritative — the next render sets it again. This keeps a
        // stale entry from a previous game (e.g. take_your_position) from
        // hijacking the router when the user navigates elsewhere.
        if (m[1] !== "game") {
            state.lastGameType = "";
            localStorage.removeItem("lobby_last_game_type");
        }
        switch (m[1]) {
            case "login": renderAuth(); break;
            case "lobby": renderLobby(); break;
            case "room":
                if (!m[2]) { location.hash = "#lobby"; return; }
                renderRoom(parseInt(m[2], 10));
                break;
            case "game":
                if (!m[2]) { location.hash = "#lobby"; return; }
                renderGame(parseInt(m[2], 10));
                break;
        }
    }

    window.addEventListener("hashchange", render);
    document.addEventListener("DOMContentLoaded", () => {
        $("#logout").addEventListener("click", (e) => { e.preventDefault(); logout(); });
        // Modal buttons: route back to the room we came from (not #lobby), so
        // a multi-game room stays alive between rounds. If we got here without
        // ever visiting a room (deep-link to #game/<id>), fall back to #lobby.
        const back = $("#game-over-back");
        if (back) {
            back.addEventListener("click", () => {
                hideGameOver();
                location.hash = state.currentRoomId ? `#room/${state.currentRoomId}` : "#lobby";
            });
        }
        // "再来一局" — host-only; calls startGame(currentRoomId) which POSTs
        // /api/rooms/<id>/start. The lobby transitions Finished → Starting.
        const again = $("#game-over-again");
        if (again) {
            again.addEventListener("click", async () => {
                if (!state.currentRoomId) return;
                again.disabled = true;
                hideGameOver();
                await startGame(state.currentRoomId);
            });
        }
        render();
    });
})();