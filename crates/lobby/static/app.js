(function () {
    "use strict";

    // ─── state ────────────────────────────────────────────────────────
    const state = {
        token: localStorage.getItem("lobby_token") || "",
        uid: parseInt(localStorage.getItem("lobby_uid") || "0", 10),
        nickname: localStorage.getItem("lobby_nick") || "",
        ws: null,
        wsInstance: 0,
        wsLog: [],
        roomCache: new Map(),
        lastRoomPlayerUids: new Set(),
        roomPollTimer: null,
        roomPollRoomId: 0,
        roomLastFetchAt: 0,
        knownGames: new Map(),
        // The room_id of the page the user is currently inside. Tracked so
        // the game-over modal can route back to #room/<id> (not #lobby).
        currentRoomId: 0,
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

    function logout() {
        localStorage.removeItem("lobby_token");
        localStorage.removeItem("lobby_uid");
        localStorage.removeItem("lobby_nick");
        state.token = state.uid = 0; state.nickname = "";
        stopRoomPolling();
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
                el("select", { id: "game-type" }, gameOptions),
                el("div", { class: "spacer" }),
                el("button", { class: "primary", onclick: createRoom }, "创建"),
            ]),
        ]);
        app.appendChild(createCard);

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
        try {
            const r = await api("/api/rooms", { game_type: gt });
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
        }
    }

    async function renderRoom(roomId) {
        stopRoomPolling();
        state.currentRoomId = roomId;
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

        const board = el("div", { class: "board hidden", id: "board" }, []);
        for (let i = 0; i < 9; i++) {
            const cell = el("div", { class: "cell", "data-i": String(i) });
            cell.addEventListener("click", () => onCellClick(i, cell));
            board.appendChild(cell);
        }
        app.appendChild(board);

        const log = el("div", { class: "event-log", id: "game-log", style: "margin-top:16px" }, "");
        app.appendChild(log);

        const wsHost = location.host;
        const url = `ws://${wsHost}/ws/${instanceId}`;
        gameLog(`连接 ${url}`);
        connectGame(url);
    }

    function gameLog(line) {
        const log = $("#game-log");
        if (!log) return;
        const ts = new Date().toLocaleTimeString();
        log.textContent += `[${ts}] ${line}\n`;
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
                $("#board").classList.remove("hidden");
                renderBoard();
                if (boardState.phase === "finished") {
                    showGameOver(boardState);
                } else {
                    status.textContent = boardState.phase === "playing" ? "游戏进行中" : "等待开始";
                    status.className = "subtitle status-bar connected";
                }
                break;
            case "game":
                if (msg.data && msg.data.state) {
                    boardState = msg.data.state;
                    renderBoard();
                    if (boardState.phase === "finished") {
                        showGameOver(boardState);
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
    }

    function hideGameOver() {
        const modal = $("#game-over-modal");
        if (modal) modal.classList.add("hidden");
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