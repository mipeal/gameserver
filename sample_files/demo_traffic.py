import random
import time
from math import floor
from typing import List
import sys
import os

sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from saarctf_commons.config import config
import psycopg2


def random_subvec(old: List[int]):
	if not old:
		pc = random.randint(32, 5000)
		by = pc * random.randint(0x1000, 0xffff)
		syn = int(random.randint(5, 40) * pc / 100)
		synack = int(random.randint(30, 90) * syn / 100)
		return [pc, by, syn, synack]
	pc, by, syn, synack = old
	mod = random.uniform(0.8, 1.21)
	pc = int(round(pc * mod))
	if pc <= 10: pc = 10
	by = int(round(by * mod * random.uniform(0.9, 1.1)))
	if by <= 1000: by = 2000
	syn = int(round(random.randint(5, 40) * pc / 100))
	synack = int(round(random.randint(30, 90) * syn / 100))
	return [pc, by, syn, synack]


def random_vector(old: List[int]) -> List[int]:
	if not old:
		return random_subvec(old) + random_subvec(old) + random_subvec(old) + random_subvec(old)
	return random_subvec(old[:4]) + random_subvec(old[4:8]) + random_subvec(old[8:12]) + random_subvec(old[12:])


conn = psycopg2.connect(config.postgres_psycopg2())
cur = conn.cursor()
cur.execute('SELECT id FROM teams ORDER BY id')
team_ids = [r[0] for r in cur.fetchall()]
cur.execute('DELETE FROM team_traffic_stats')
conn.commit()

# column order mirrors the old TeamTrafficStats.efficient_insert
sql = (
	'INSERT INTO team_traffic_stats ("time", team_id, '
	"down_game_packets, down_game_bytes, down_game_syns, down_game_syn_acks, "
	"down_teams_packets, down_teams_bytes, down_teams_syns, down_teams_syn_acks, "
	"up_game_packets, up_game_bytes, up_game_syns, up_game_syn_acks, "
	+ "up_teams_packets, up_teams_bytes, up_teams_syns, up_teams_syn_acks, "
	"forward_self_packets, forward_self_bytes, forward_self_syns, forward_self_syn_acks) "
	f"SELECT to_timestamp(%(ts)s), unnest(%(teams)s), "
	"unnest(%(v0)s), unnest(%(v1)s), unnest(%(v2)s), unnest(%(v3)s), "
	"unnest(%(v4)s), unnest(%(v5)s), unnest(%(v6)s), unnest(%(v7)s), "
	"unnest(%(v8)s), unnest(%(v9)s), unnest(%(v10)s), unnest(%(v11)s), "
	"unnest(%(v12)s), unnest(%(v13)s), unnest(%(v14)s), unnest(%(v15)s), "
	"unnest(%(v16)s), unnest(%(v17)s), unnest(%(v18)s), unnest(%(v19)s)"
)

endtime = int(floor(time.time() // 60)) * 60

data = {}
for ts in range(endtime - 720 * 60, endtime + 1, 60):
	print(ts, '...', endtime)
	data = {team_id: random_vector(data.get(team_id, False)) for team_id in team_ids}
	params: dict = {'ts': ts, 'teams': list(data.keys())}
	for i in range(20):
		params[f'v{i}'] = []
	for team_id, values in data.items():
		for i, v in enumerate(values):
			params[f'v{i}'].append(v)
	cur.execute(sql, params)
	conn.commit()

print('[DONE]')
