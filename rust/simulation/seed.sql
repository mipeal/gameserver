-- Simulation seed (SAARCTF_SIM_SEED, applied by sim_boot): fixture teams.
-- Idempotent: re-running is a no-op. vpn_connected = TRUE is cosmetic
-- (consumers that render it); SIM_VPN=1 gating uses real wg state.
INSERT INTO teams (id, name, affiliation, website, vpn_connected) VALUES
  (1, 'NOP',                NULL, NULL, TRUE),
  (2, 'Bushwhackers',       NULL, NULL, TRUE),
  (3, 'c00kies@venice',     NULL, NULL, TRUE),
  (4, 'saarsec',            NULL, NULL, TRUE),
  (5, 'LC↯BC',              NULL, NULL, TRUE),
  (6, 'Destructive Voice',  NULL, NULL, TRUE),
  (7, 'Shadow Servants',    NULL, NULL, TRUE),
  (8, 'SUSlo.PAS',          NULL, NULL, TRUE)
ON CONFLICT (id) DO NOTHING;
SELECT setval('teams_id_seq', (SELECT COALESCE(MAX(id), 1) FROM teams), TRUE);
