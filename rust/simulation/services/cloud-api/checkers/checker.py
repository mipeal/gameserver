"""Simulation checker for the cloud-api fixture (port 8080)."""

import requests
from gamelib import FlagMissingException, MumbleException, OfflineException, ServiceInterface

URL = 'http://{host}:8080/'


class CloudApiService(ServiceInterface):

	def check_integrity(self, team, tick) -> None:
		try:
			response = requests.get(URL.format(host=team.ip), timeout=5)
		except requests.exceptions.ConnectionError:
			raise OfflineException(f'{team.ip}:8080 is offline')
		if response.status_code != 200:
			raise MumbleException(str(response.status_code))

	def store_flags(self, team, tick) -> None:
		try:
			response = requests.post(URL.format(host=team.ip), data={
				'team_id': team.id,
				'service_id': self.id,
				'tick': tick,
				'flag': '\n'.join(self.get_flag(team, tick, i) for i in range(3)),
			}, timeout=5)
			if response.status_code != 200:
				raise MumbleException(str(response.status_code))
			return 3
		except requests.exceptions.ConnectionError:
			raise OfflineException(f'{team.ip}:8080 is offline')

	def retrieve_flags(self, team, tick) -> None:
		try:
			response = requests.get(URL.format(host=team.ip), params={
				'team_id': team.id,
				'service_id': self.id,
				'tick': tick,
			}, timeout=5)
			if response.status_code != 200:
				raise MumbleException(str(response.status_code))
			flags = self.search_flags(response.text)
			if len(flags) < 3:
				raise FlagMissingException('Not all flags found')
			payloads = set()
			for flag in flags:
				teamid, serviceid, tick_, payload = self.check_flag(flag, team.id, tick)
				if not teamid:
					raise FlagMissingException('Invalid flag retrieved')
				payloads.add(payload)
			if len(payloads) < 3:
				raise FlagMissingException('Flag repeated')
			return 3
		except requests.exceptions.ConnectionError:
			raise OfflineException(f'{team.ip}:8080 is offline')
