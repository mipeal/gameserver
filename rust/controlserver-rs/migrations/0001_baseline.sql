-- Baseline schema (ADR-005): squashed final state of the 38 Alembic migrations,
-- derived from controlserver/models.py. One-way: existing deployments are already here.

CREATE TABLE IF NOT EXISTS teams (
    id SERIAL PRIMARY KEY,
    name VARCHAR(128) NOT NULL,
    affiliation VARCHAR(128) DEFAULT NULL,
    website VARCHAR(128) DEFAULT NULL,
    logo VARCHAR(64) DEFAULT NULL,
    vpn_connected BOOLEAN NOT NULL DEFAULT FALSE,
    vpn2_connected BOOLEAN NOT NULL DEFAULT FALSE,
    vpn_connection_count INTEGER NOT NULL DEFAULT 0,
    vpn_last_connect TIMESTAMPTZ DEFAULT NULL,
    vpn_last_disconnect TIMESTAMPTZ DEFAULT NULL,
    wg_vulnbox_connected BOOLEAN NOT NULL DEFAULT FALSE,
    wg_boxes_connected BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS team_logos (
    id SERIAL PRIMARY KEY,
    hash VARCHAR(64) NOT NULL,
    content BYTEA NOT NULL,
    CONSTRAINT team_logos_hash_key UNIQUE (hash)
);
CREATE INDEX IF NOT EXISTS ix_team_logos_hash ON team_logos (hash);

CREATE TABLE IF NOT EXISTS services (
    id SERIAL PRIMARY KEY,
    name VARCHAR(128) NOT NULL,
    checker_timeout INTEGER NOT NULL DEFAULT 30,
    checker_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    checker_runner TEXT NOT NULL DEFAULT '',
    checker_script_dir TEXT DEFAULT NULL,
    checker_script TEXT NOT NULL,
    package VARCHAR(32) DEFAULT NULL,
    setup_package VARCHAR(32) DEFAULT NULL,
    checker_subprocess BOOLEAN NOT NULL DEFAULT FALSE,
    checker_route VARCHAR(64) DEFAULT NULL,
    runner_config JSONB DEFAULT NULL,
    num_payloads INTEGER NOT NULL DEFAULT 0,
    flag_ids VARCHAR(128) DEFAULT NULL,
    flags_per_tick DOUBLE PRECISION NOT NULL DEFAULT 1,
    ports TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS team_points (
    id SERIAL PRIMARY KEY,
    tick INTEGER NOT NULL,
    team_id SMALLINT NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    service_id SMALLINT NOT NULL REFERENCES services (id) ON DELETE CASCADE,
    flag_captured_count INTEGER NOT NULL,
    flag_stolen_count INTEGER NOT NULL,
    off_points DOUBLE PRECISION NOT NULL,
    def_points DOUBLE PRECISION NOT NULL,
    sla_points DOUBLE PRECISION NOT NULL,
    sla_delta DOUBLE PRECISION NOT NULL,
    CONSTRAINT team_points_unique_1 UNIQUE (tick, team_id, service_id)
);
CREATE INDEX IF NOT EXISTS ix_team_points_tick ON team_points (tick);

CREATE TABLE IF NOT EXISTS team_rankings (
    id SERIAL PRIMARY KEY,
    tick INTEGER NOT NULL,
    team_id SMALLINT NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    rank INTEGER NOT NULL,
    points DOUBLE PRECISION NOT NULL,
    CONSTRAINT team_rankings_unique_1 UNIQUE (tick, team_id)
);
CREATE INDEX IF NOT EXISTS ix_team_rankings_tick ON team_rankings (tick);

CREATE TABLE IF NOT EXISTS team_traffic_stats (
    id SERIAL PRIMARY KEY,
    "time" TIMESTAMPTZ NOT NULL,
    team_id SMALLINT NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    down_teams_packets BIGINT NOT NULL,
    down_teams_bytes BIGINT NOT NULL,
    down_teams_syns BIGINT NOT NULL,
    down_teams_syn_acks BIGINT NOT NULL,
    down_game_packets BIGINT NOT NULL,
    down_game_bytes BIGINT NOT NULL,
    down_game_syns BIGINT NOT NULL,
    down_game_syn_acks BIGINT NOT NULL,
    up_teams_packets BIGINT NOT NULL,
    up_teams_bytes BIGINT NOT NULL,
    up_teams_syns BIGINT NOT NULL,
    up_teams_syn_acks BIGINT NOT NULL,
    up_game_packets BIGINT NOT NULL,
    up_game_bytes BIGINT NOT NULL,
    up_game_syns BIGINT NOT NULL,
    up_game_syn_acks BIGINT NOT NULL,
    forward_self_packets BIGINT NOT NULL,
    forward_self_bytes BIGINT NOT NULL,
    forward_self_syns BIGINT NOT NULL,
    forward_self_syn_acks BIGINT NOT NULL,
    CONSTRAINT team_traffic_stats_unique_1 UNIQUE ("time", team_id)
);
CREATE INDEX IF NOT EXISTS ix_team_traffic_stats_time ON team_traffic_stats ("time");

CREATE TABLE IF NOT EXISTS submitted_flags (
    id SERIAL PRIMARY KEY,
    submitted_by SMALLINT NOT NULL,
    team_id SMALLINT NOT NULL,
    service_id SMALLINT NOT NULL,
    tick_issued SMALLINT NOT NULL,
    payload INTEGER NOT NULL DEFAULT 0,
    tick_submitted SMALLINT NOT NULL,
    is_firstblood SMALLINT NOT NULL DEFAULT 0,
    ts TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT submitted_flags_unique_1
        UNIQUE (submitted_by, team_id, service_id, tick_issued, payload)
);
CREATE INDEX IF NOT EXISTS ix_submitted_flags_tick_submitted ON submitted_flags (tick_submitted);
CREATE INDEX IF NOT EXISTS ix_submitted_flags_is_firstblood ON submitted_flags (is_firstblood);

CREATE TABLE IF NOT EXISTS checker_results (
    id SERIAL PRIMARY KEY,
    tick INTEGER NOT NULL,
    team_id SMALLINT NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    service_id SMALLINT NOT NULL REFERENCES services (id) ON DELETE CASCADE,
    status VARCHAR(12) NOT NULL,
    message TEXT DEFAULT NULL,
    "time" DOUBLE PRECISION DEFAULT NULL,
    celery_id VARCHAR(40) NOT NULL,
    output TEXT DEFAULT NULL,
    finished TIMESTAMPTZ DEFAULT NULL,
    data JSONB DEFAULT NULL,
    run_over_time BOOLEAN NOT NULL DEFAULT FALSE,
    CONSTRAINT checker_results_unique_1 UNIQUE (tick, team_id, service_id)
);
CREATE INDEX IF NOT EXISTS ix_checker_results_tick ON checker_results (tick);
CREATE INDEX IF NOT EXISTS ix_checker_results_status ON checker_results (status);

CREATE TABLE IF NOT EXISTS logmessages (
    id SERIAL PRIMARY KEY,
    created TIMESTAMPTZ NOT NULL DEFAULT now(),
    component VARCHAR(128) NOT NULL,
    level SMALLINT DEFAULT 0,
    title TEXT,
    text TEXT
);

CREATE TABLE IF NOT EXISTS checker_filesystem (
    id SERIAL PRIMARY KEY,
    package VARCHAR(32) NOT NULL,
    path TEXT NOT NULL,
    file_hash VARCHAR(32) DEFAULT NULL,
    CONSTRAINT checker_filesystem_unique_1 UNIQUE (package, path)
);
CREATE INDEX IF NOT EXISTS ix_checker_filesystem_package ON checker_filesystem (package);

CREATE TABLE IF NOT EXISTS checker_files (
    id SERIAL PRIMARY KEY,
    file_hash VARCHAR(32) NOT NULL,
    content BYTEA NOT NULL,
    CONSTRAINT checker_files_unique_1 UNIQUE (file_hash)
);
CREATE INDEX IF NOT EXISTS ix_checker_files_file_hash ON checker_files (file_hash);

CREATE TABLE IF NOT EXISTS ticks (
    tick INTEGER PRIMARY KEY,
    "start" TIMESTAMPTZ DEFAULT NULL,
    "end" TIMESTAMPTZ DEFAULT NULL
);
