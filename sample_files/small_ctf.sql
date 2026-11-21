DELETE FROM teams;
DELETE FROM services;
DELETE FROM submitted_flags;
DELETE FROM logmessages;
DELETE FROM submitted_flags;


SELECT setval('services_id_seq', 1, false);
INSERT INTO services ("name", checker_script, checker_timeout, checker_script_dir, checker_enabled) VALUES
  ('Service1', 'checker_runner.demo_checker:WorkingService', 5, NULL, TRUE),
  ('Service2', 'checker_runner.demo_checker:WorkingService', 5, NULL, TRUE),
  ('Service3', 'checker_runner.demo_checker:WorkingService', 5, NULL, TRUE);


SELECT setval('teams_id_seq', 1, false);
INSERT INTO teams ("name") VALUES
('Team 1'),
('Team 2'),
('Team 3'),
('Team 4'),
('Team 5'),
('Team 6'),
('Team 7'),
('Team 8'),
('SiBears'),
('Lights Out'),
('STT'),
('EpicTeam'),
('Tower of Hanoi'),
('ENOFLAG'),
('Honeypot'),
('Corrupted Reflection'),
('girav'),
('SharLike'),
('Shellphish'),
('We_0wn_Y0u'),
('Novosibirsk SU X'),
('SwissMadeSecurity'),
('PeterPen'),
('Invisible');
