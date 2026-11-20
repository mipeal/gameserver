"""Simulation checker for the flagboard fixture (port 31337, 1 payload)."""

import requests
from gamelib import FlagMissingException, MumbleException, OfflineException, ServiceInterface

URL = 'http://{host}:31337/'


class FlagboardService(ServiceInterface):

	def check_integrity(self, team, tick) -> None:
		try:
			response = requests.get(URL.format(host=team.ip), timeout=5)
		except requests.exceptions.ConnectionError:
			raise OfflineException(f'{team.ip}:31337 is offline')
		if response.status_code != 200:
			raise MumbleException(str(response.status_code))

	def store_flags(self, team, tick) -> None:
		try:
			response = requests.post(URL.format(host=team.ip), data={
				'team_id': team.id,
				'service_id': self.id,
				'tick': tick,
				'flag': self.get_flag(team, tick, 0),
			}, timeout=5)
			if response.status_code != 200:
				raise MumbleException(str(response.status_code))
			return 1
		except requests.exceptions.ConnectionError:
			raise OfflineException(f'{team.ip}:31337 is offline')

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
			if len(flags) < 1:
				raise FlagMissingException('No flags found')
			for flag in flags:
				teamid, serviceid, tick_, payload = self.check_flag(flag, team.id, tick)
				if not teamid:
					raise FlagMissingException('Invalid flag retrieved')
			return 1
		except requests.exceptions.ConnectionError:
			raise OfflineException(f'{team.ip}:31337 is offline')
