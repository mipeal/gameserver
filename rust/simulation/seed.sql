-- Simulation seed (SAARCTF_SIM_SEED, applied by sim_boot): fixture teams.
-- Idempotent: re-running is a no-op. vpn_connected stays FALSE — with
-- SIM_VPN=1 the live wg state (teams.wg_vulnbox_connected, written by
-- vpnboard) is the single source of truth for dispatching and the
-- scoreboard's online flag; seeding it TRUE would short-circuit both.
INSERT INTO teams (id, name, affiliation, website) VALUES
  (1, 'NOP',                NULL, NULL),
  (2, 'Bushwhackers',       NULL, NULL),
  (3, 'c00kies@venice',     NULL, NULL),
  (4, 'saarsec',            NULL, NULL),
  (5, 'LC↯BC',              NULL, NULL),
  (6, 'Destructive Voice',  NULL, NULL),
  (7, 'Shadow Servants',    NULL, NULL),
  (8, 'SUSlo.PAS',          NULL, NULL)
ON CONFLICT (id) DO NOTHING;
SELECT setval('teams_id_seq', (SELECT COALESCE(MAX(id), 1) FROM teams), TRUE);
