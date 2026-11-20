#!/bin/sh
# seed_admin_data.sh — fill the admin-GUI areas the sim never populates:
#   1. sample patches (the real handler creates script + ansible files), since
#      nothing in the sim ever POSTs /api/patches/create;
#   2. team_traffic_stats history, since vpn-manage-trafficstats needs BPF maps
#      and the sim never runs it. TeamView graphs and /overview/vpn are empty
#      without rows.
# Idempotent: patches are only written when missing, rows conflict-skip.
# One-time backfill (60 samples, one per minute — the manage-trafficstats
# cadence); re-run after a long gap to keep the graphs inside their windows.
# KNOWN APP BUG: /overview/vpn 500s once rows exist — overview_vpn decodes
# sum(BIGINT) (NUMERIC) into i64. Fix there is a ::bigint cast; not touched here.
set -e
cd "$(dirname "$0")"
ENV_FILE=".env.sim"
[ -f "$ENV_FILE" ] || ENV_FILE=".env.sim.example"
ROOT="$(cd .. && pwd)"
API="${CONTROLSERVER_URL:-http://localhost:15000}"

for spec in "example-patch.sh bash" "refresh-cache.py python"; do
    set -- $spec
    curl -sf -X POST "$API/api/patches/create" \
        -H 'Content-Type: application/json' \
        -d "{\"filename\": \"$1\", \"type\": \"$2\"}" >/dev/null
    echo "patch: $1"
done

COMPOSE="docker compose --env-file $ENV_FILE \
  -f $ROOT/docker-compose.simulation.yml \
  -f ./docker-compose.vulnboxes.yml"
if grep -q '^SIM_VPN=1' "$ENV_FILE"; then
    COMPOSE="$COMPOSE -f ./docker-compose.sim-vpn.yml"
fi

# Values mirror a quiet-but-live game: team-to-team bytes dominate, game-server
# bytes are smaller; packets/syns are derived from bytes. All 8 fixture teams.
# Derived from the absolute epoch minute so re-runs only add the new minute
# instead of duplicating the previous seam value.
$COMPOSE exec -T postgres psql -U saarctf -d saarctf -v ON_ERROR_STOP=1 <<'SQL'
WITH base AS (
    SELECT date_trunc('minute', now()) AS now_min
), grid AS (
    SELECT now_min - make_interval(mins => s) AS ts,
           extract(epoch FROM now_min)::bigint / 60 - s AS m
    FROM base, generate_series(0, 59) AS s
), samples AS (
    SELECT
        ts, m, t.id AS team_id,
        (180000 + ((m * 37 + t.id * 97) % 40) * 45000)::bigint AS down_teams_bytes,
        (30000  + ((m * 53 + t.id * 71) % 18) * 9000)::bigint  AS down_game_bytes,
        (90000  + ((m * 29 + t.id * 61) % 32) * 26000)::bigint AS up_teams_bytes,
        (15000  + ((m * 43 + t.id * 89) % 14) * 7000)::bigint  AS up_game_bytes
    FROM grid
    CROSS JOIN teams t
)
INSERT INTO team_traffic_stats (
    "time", team_id,
    down_teams_packets, down_teams_bytes, down_teams_syns, down_teams_syn_acks,
    down_game_packets, down_game_bytes, down_game_syns, down_game_syn_acks,
    up_teams_packets, up_teams_bytes, up_teams_syns, up_teams_syn_acks,
    up_game_packets, up_game_bytes, up_game_syns, up_game_syn_acks,
    forward_self_packets, forward_self_bytes, forward_self_syns, forward_self_syn_acks)
SELECT ts, team_id,
       down_teams_bytes / 1200, down_teams_bytes, down_teams_bytes / 120000, down_teams_bytes / 130000,
       down_game_bytes / 900, down_game_bytes, down_game_bytes / 90000, down_game_bytes / 95000,
       up_teams_bytes / 1200, up_teams_bytes, up_teams_bytes / 120000, up_teams_bytes / 130000,
       up_game_bytes / 900, up_game_bytes, up_game_bytes / 90000, up_game_bytes / 95000,
       0, 0, 0, 0
FROM samples
ON CONFLICT ("time", team_id) DO NOTHING;
SQL
$COMPOSE exec -T postgres psql -U saarctf -d saarctf -tAc \
    'SELECT format('"'"'traffic: %s rows for %s teams'"'"', count(*), count(DISTINCT team_id)) FROM team_traffic_stats'
