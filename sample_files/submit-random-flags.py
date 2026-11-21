import os
import random
import sys


sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from saarctf_commons.config import config
from saarctf_commons.redis import get_redis_connection

config.EXTERNAL_TIMER = True
import psycopg2
from saarctf_commons.debug_sql_timing import timing


def submit_random_flags(conn, endround: int, chance=0.33):
	cur = conn.cursor()
	cur.execute('SELECT id FROM teams WHERE id != 1 ORDER BY id')
	teams = [row[0] for row in cur.fetchall()]
	cur.execute('SELECT id, num_payloads FROM services ORDER BY id')
	services = cur.fetchall()
	cur.execute('SELECT name, id FROM teams ORDER BY id')
	team_names = dict(cur.fetchall())
	team_exploit_ready = {}
	team_patch_ready = {}
	for s in services:
		sid, num_payloads = s
		service_difficulty = random.randint(1, 5)
		for t in teams:
			team_exploit_ready[(t, sid)] = random.randint(1, round(endround / chance)) * service_difficulty
			team_patch_ready[(t, sid)] = random.randint(1, round(endround)) * service_difficulty // 3
			if team_names[t] == 'Team 4':
				team_exploit_ready[(t, sid)] = int(team_exploit_ready[(t, sid)] * 0.8)
				team_patch_ready[(t, sid)] = int(team_patch_ready[(t, sid)] * 0.8)
	for s in services:
		sid, num_payloads = s
		submitted_flags = []
		for attacker in teams:
			# print(f'Service {sid}, team {attacker}: exploit {team_exploit_ready[(attacker, sid)]} / patch {team_patch_ready[(attacker, sid)]}')
			if team_exploit_ready[(attacker, sid)] > endround: continue
			for victim in teams:
				for r in range(team_exploit_ready[(attacker, sid)], min(endround, team_patch_ready[(victim, sid)])):
					# hack!
					p = 0
					if num_payloads and num_payloads > 1: p = random.randint(0, num_payloads - 1)
					flag = (attacker, sid, victim, r, p, r + random.randint(0, 4))
					submitted_flags.append(flag)
			# print(len(submitted_flags))
		print(f'Service {sid}, submitting {len(submitted_flags)} flags...')
		cur.executemany(
			'INSERT INTO submitted_flags (submitted_by, service_id, team_id, tick_issued, payload, tick_submitted) '
			'VALUES (%s, %s, %s, %s, %s, %s)', submitted_flags)
		conn.commit()


if __name__ == '__main__':
	config.set_redis_clientname('script-' + os.path.basename(__file__))
	# Timer with EXTERNAL_TIMER=True reads its state from redis (timing:state /
	# timing:currentRound — the same keys the Rust controlserver publishes)
	redis = get_redis_connection()
	roundnumber = int(redis.get('timing:currentRound') or 0)
	if (redis.get('timing:state') or b'STOPPED').decode() == 'RUNNING':
		roundnumber -= 1
	conn = psycopg2.connect(config.postgres_psycopg2())
	timing()
	submit_random_flags(conn, roundnumber)
	timing('Submitter')
	print('Done.')
