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
    };

    // ─── helpers ──────────────────────────────────────────────────────
    const $ = (sel, el = document) => el.querySelector(sel);
    const $$ = (sel, el = document) => Array.from(el.querySelectorAll(sel));
    const el = (tag, attrs = {}, children = []) => {
        const e = document.createElement(tag);
        for (const [k, v] of Object.entries(attrs)) {
            if (k === "class") e.className = v;
            else if (k === "html") e.innerHTML = v;
            else if (k.startsWith("on") && typeof v === "function") e.addEventListener(k.slice(2), v);
            else e.setAttribute(k, v);
        }
        for (const c of [].concat(children)) {
            if (c == null) continue;
            e.appendChild(typeof c === "string" ? document.createTextNode(c) : c);
        }
        return e;
    };

    function prettyJSON(t) { try { return JSON.stringify(JSON.parse(t), null, 2); } catch { return t; } }

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
        let nonce = 0;
        while (true) {
            const input = enc.encode(challenge + ":" + nonce);
            const buf = await crypto.subtle.digest("SHA-256", input);
            const hash = new Uint8Array(buf);
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

        const loginForm = el("form", { class: "card", id: "form-login", onsubmit: handleLogin }, [
            el("label", {}, ["用户名", el("input", { name: "username", required: true, autocomplete: "username" })]),
            el("label", {}, ["密码", el("input", { name: "password", type: "password", required: true, autocomplete: "current-password" })]),
            el("button", { class: "primary", type: "submit" }, "登录"),
        ]);
        const registerForm = el("form", { class: "card hidden", id: "form-register", onsubmit: handleRegister }, [
            el("label", {}, ["用户名", el("input", { name: "username", required: true, autocomplete: "username" })]),
            el("label", {}, ["密码（≥9 位，含数字/字母/特殊字符）", el("input", { name: "password", type: "password", required: true, minlength: "9", autocomplete: "new-password" })]),
            el("label", {}, ["昵称", el("input", { name: "nickname", required: true })]),
            el("button", { class: "primary", type: "submit" }, "创建账号"),
        ]);
        app.appendChild(loginForm);
        app.appendChild(registerForm);
    }

    function showTab(name) {
        $$("#app .tabs button").forEach(b => b.classList.toggle("active", b.id === "tab-" + name));
        $("#form-login").classList.toggle("hidden", name !== "login");
        $("#form-register").classList.toggle("hidden", name !== "register");
    }

    async function handleLogin(e) {
        e.preventDefault();
        const fd = new FormData(e.target);
        try {
            toast("正在计算人机验证...", "");
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
        } catch (e) { toast(e.message, "error"); }
    }

    async function handleRegister(e) {
        e.preventDefault();
        const fd = new FormData(e.target);
        try {
            toast("正在计算人机验证...", "");
            const captcha = await solveCaptcha();
            const r = await api("/api/register", {
                username: fd.get("username"), password: fd.get("password"), nickname: fd.get("nickname"), captcha,
            });
            toast("注册成功 uid=" + r.uid, "ok");
            showTab("login");
        } catch (e) { toast(e.message, "error"); }
    }

    // ─── lobby ────────────────────────────────────────────────────────
    async function renderLobby() {
        $("#topbar").classList.remove("hidden");
        $("#user-info").textContent = `${state.nickname} (#${state.uid})`;
        const app = $("#app");
        app.innerHTML = "";
        app.appendChild(el("h1", {}, "房间"));
        app.appendChild(el("p", { class: "subtitle" }, "创建或加入一个房间开始游戏。"));

        const createCard = el("div", { class: "card" }, [
            el("h2", { style: "margin:0;font-size:15px;color:var(--muted)" }, "创建房间"),
            el("div", { class: "row" }, [
                el("select", { id: "game-type" }, [
                    el("option", { value: "tictactoe" }, "井字棋"),
                ]),
                el("div", { class: "spacer" }),
                el("button", { class: "primary", onclick: createRoom }, "创建"),
            ]),
        ]);
        app.appendChild(createCard);

        app.appendChild(el("h2", { style: "margin:24px 0 8px;font-size:15px;color:var(--muted)" }, "现有房间"));
        const list = el("div", { class: "room-list", id: "room-list" }, [el("div", { class: "muted" }, "加载中...")]);
        app.appendChild(list);

        try {
            const data = await apiGet("/api/rooms");
            renderRoomList(data.rooms || []);
        } catch (e) {
            list.innerHTML = `<div class="status-bar error">${e.message}</div>`;
        }
    }

    function renderRoomList(rooms) {
        const list = $("#room-list");
        list.innerHTML = "";
        if (rooms.length === 0) {
            list.appendChild(el("div", { class: "status-bar" }, "暂无房间，创建一个吧。"));
            return;
        }
        for (const r of rooms) {
            const players = r.players.map(p => p.nickname).join(", ") || "(空)";
            const item = el("div", { class: "room-item" }, [
                el("div", { class: "info" }, [
                    el("div", {}, [
                        el("strong", {}, `#${r.room_id}`),
                        " ",
                        r.game_type,
                        " ",
                        el("span", { class: "status-tag status-" + r.status }, r.status),
                    ]),
                    el("div", { class: "meta" }, `玩家: ${players}`),
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

    // ─── room detail ──────────────────────────────────────────────────
    async function renderRoom(roomId) {
        $("#topbar").classList.remove("hidden");
        $("#user-info").textContent = `${state.nickname} (#${state.uid})`;
        const app = $("#app");
        app.innerHTML = "";
        app.appendChild(el("h1", {}, `房间 #${roomId}`));
        const wrap = el("div", { id: "room-detail" }, [el("div", { class: "muted" }, "加载中...")]);
        app.appendChild(wrap);

        try {
            const r = await apiGet(`/api/rooms/${roomId}`);
            state.roomCache.set(roomId, r);
            renderRoomDetail(r);
        } catch (e) {
            wrap.innerHTML = `<div class="status-bar error">${e.message}</div>`;
        }
    }

    function renderRoomDetail(r) {
        const wrap = $("#room-detail");
        wrap.innerHTML = "";

        const isHost = r.host_uid === state.uid;
        const playerUids = new Set(r.players.map(p => p.uid));
        const canStart = isHost && r.status === "Waiting" && r.players.length === 2;

        wrap.appendChild(el("div", { class: "row" }, [
            el("span", { class: "status-tag status-" + r.status }, r.status),
            el("span", { class: "muted" }, `游戏: ${r.game_type}`),
            el("div", { class: "spacer" }),
            el("button", { class: "ghost", onclick: () => location.hash = "#lobby" }, "返回房间列表"),
        ]));

        const list = el("div", { class: "player-list", style: "margin-top:16px" });
        for (const p of r.players) {
            list.appendChild(el("div", { class: "player-row" }, [
                el("span", {}, `${p.nickname} (#${p.uid}) ${p.uid === r.host_uid ? "[房主]" : ""}`),
                el("span", { class: "muted" }, `座位 ${p.seat}`),
            ]));
        }
        if (r.players.length < 2 && r.status === "Waiting") {
            list.appendChild(el("div", { class: "player-row" }, [
                el("span", { class: "muted" }, "(等待第二位玩家加入)"),
            ]));
        }
        wrap.appendChild(list);

        const actions = el("div", { class: "row", style: "margin-top:16px" });
        if (!playerUids.has(state.uid) && r.status === "Waiting") {
            actions.appendChild(el("button", { class: "primary", onclick: () => joinRoom(r.room_id) }, "加入房间"));
        }
        if (isHost && r.status === "Waiting") {
            actions.appendChild(el("button", { class: "primary", disabled: !canStart, onclick: () => startGame(r.room_id) }, "开始游戏"));
        }
        if (playerUids.has(state.uid) && r.status === "Waiting") {
            actions.appendChild(el("button", { class: "ghost", onclick: () => leaveRoom(r.room_id) }, "离开"));
        }
        if (r.status === "Running" && playerUids.has(state.uid)) {
            const instanceId = state.instanceIdByRoom?.get(r.room_id);
            if (instanceId) {
                actions.appendChild(el("button", { class: "primary", onclick: () => location.hash = "#game/" + instanceId }, "进入游戏"));
            } else {
                actions.appendChild(el("span", { class: "muted" }, "等待房主开始游戏..."));
            }
        }
        wrap.appendChild(actions);

        if (r.status === "Starting") {
            wrap.appendChild(el("div", { class: "status-bar", id: "start-wait" }, "正在启动游戏..."));
            setTimeout(() => location.hash === "#room/" + r.room_id && renderRoom(r.room_id), 1500);
        }
    }

    async function joinRoom(id) {
        try {
            const r = await api(`/api/rooms/${id}/join`, {});
            state.roomCache.set(id, r);
            toast("已加入", "ok");
            renderRoomDetail(r);
        } catch (e) { toast(e.message, "error"); }
    }

    async function leaveRoom(id) {
        try {
            await api(`/api/rooms/${id}/leave`, {});
            toast("已离开", "ok");
            location.hash = "#lobby";
        } catch (e) { toast(e.message, "error"); }
    }

    async function startGame(id) {
        try {
            const r = await api(`/api/rooms/${id}/start`, {});
            state.instanceIdByRoom = state.instanceIdByRoom || new Map();
            state.instanceIdByRoom.set(id, r.instance_id);
            toast("游戏已启动", "ok");
            setTimeout(() => location.hash = "#game/" + r.instance_id, 800);
        } catch (e) { toast(e.message, "error"); }
    }

    // ─── game (websocket) ──────────────────────────────────────────────
    let boardState = null;
    let myUid = state.uid;
    let gameWs = null;

    function renderGame(instanceId) {
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
                status.textContent = boardState.phase === "playing" ? "游戏进行中" : "等待开始";
                status.className = "subtitle status-bar connected";
                break;
            case "game":
                if (msg.data && msg.data.state) {
                    boardState = msg.data.state;
                    renderBoard();
                    if (boardState.phase === "finished") {
                        status.textContent = boardState.winner
                            ? `游戏结束，胜者 uid=${boardState.winner}`
                            : "平局";
                        status.className = "subtitle status-bar";
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

    function seatMark(uid) {
        // For tictactoe: seat 0 = X, seat 1 = O. We don't know seat by uid alone in client.
        // Use a simple heuristic: first player (lower uid) = X, other = O.
        if (!boardState || !boardState.players) return "?";
        const idx = boardState.players.indexOf(uid);
        return idx === 0 ? "✕" : "○";
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
        const m = hash.match(/^#(login|register|lobby|room\/(\d+)|game\/(\d+))$/);
        if (!m) { location.hash = state.token ? "#lobby" : "#login"; return; }
        if (state.token && (hash === "#login" || hash === "#register" || hash === "")) {
            location.hash = "#lobby"; return;
        }
        if (!state.token && (hash !== "#login" && hash !== "#register")) {
            location.hash = "#login"; return;
        }
        switch (m[1]) {
            case "login": renderAuth(); break;
            case "lobby": renderLobby(); break;
            case "room": renderRoom(parseInt(m[2], 10)); break;
            case "game": renderGame(parseInt(m[3], 10)); break;
        }
    }

    window.addEventListener("hashchange", render);
    document.addEventListener("DOMContentLoaded", () => {
        $("#logout").addEventListener("click", (e) => { e.preventDefault(); logout(); });
        render();
    });
})();