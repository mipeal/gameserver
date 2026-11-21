-- Simulation seed (CTF_SIM_SEED, applied by sim_boot): fixture teams.
-- Idempotent: re-running is a no-op. vpn_connected stays FALSE — with
-- SIM_VPN=1 the live wg state (teams.wg_vulnbox_connected, written by
-- vpnboard) is the single source of truth for dispatching and the
-- scoreboard's online flag; seeding it TRUE would short-circuit both.
INSERT INTO teams (id, name, affiliation, website) VALUES
  (1, 'Team 1',  NULL, NULL),
  (2, 'Team 2',  NULL, NULL),
  (3, 'Team 3',  NULL, NULL),
  (4, 'Team 4',  NULL, NULL),
  (5, 'Team 5',  NULL, NULL),
  (6, 'Team 6',  NULL, NULL),
  (7, 'Team 7',  NULL, NULL),
  (8, 'Team 8',  NULL, NULL)
ON CONFLICT (id) DO NOTHING;
SELECT setval('teams_id_seq', (SELECT COALESCE(MAX(id), 1) FROM teams), TRUE);
