CREATE TABLE IF NOT EXISTS users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    nickname      TEXT NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS sessions (
    token      TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS rooms (
    room_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    game_type  TEXT NOT NULL,
    host_uid   INTEGER NOT NULL REFERENCES users(id),
    status     TEXT NOT NULL DEFAULT 'Waiting',  -- Waiting/Starting/Running/Finished/Destroyed
    variant    TEXT,
    config     TEXT,
    timer_preset TEXT NOT NULL DEFAULT '30+60', -- 30+60 / 40+120 / 60+180 (per-round or global)
    last_active_at TEXT,                          -- updated on GET /api/rooms/:id (room-page heartbeat)
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS room_players (
    room_id   INTEGER NOT NULL REFERENCES rooms(room_id),
    uid       INTEGER NOT NULL REFERENCES users(id),
    seat      INTEGER NOT NULL,
    online    INTEGER NOT NULL DEFAULT 1,   -- 1=WS connected, 0=disconnected (lobby tracks via ws_proxy)
    joined_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (room_id, uid)
);

CREATE TABLE IF NOT EXISTS game_instances (
    instance_id INTEGER PRIMARY KEY AUTOINCREMENT,
    room_id     INTEGER NOT NULL REFERENCES rooms(room_id),
    pid         INTEGER,
    port        INTEGER,
    status      TEXT NOT NULL,                  -- starting/ready/running/finished/abnormal/stopped
    start_time  TEXT,
    end_time    TEXT,
    last_action_at TEXT                          -- updated on every game action; cleanup task uses for stale detection
);

CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at);
CREATE INDEX IF NOT EXISTS idx_sessions_user    ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_room_players_uid ON room_players(uid);
CREATE INDEX IF NOT EXISTS idx_instances_room  ON game_instances(room_id);
-- NOTE: idx_room_players_online / idx_instances_action are created in
-- migrations.rs AFTER the ALTER TABLE adds those columns (new-DB-first
-- ordering problem otherwise).