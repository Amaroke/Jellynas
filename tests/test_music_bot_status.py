import json
import unittest
import urllib.error
import urllib.request

from player import Player, Track
from status import STALE_SECONDS, Status, serve, summary


class Clock:
    def __init__(self):
        self.now = 1000.0

    def __call__(self):
        return self.now


def fetch(url):
    try:
        with urllib.request.urlopen(url, timeout=5) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        with error:
            return error.code, json.load(error)


class StatusTest(unittest.TestCase):
    def setUp(self):
        self.clock = Clock()
        self.status = Status(self.clock)
        server = serve(self.status, "127.0.0.1", 0)
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        self.url = f"http://127.0.0.1:{server.server_port}"

    def test_unhealthy_before_discord_connects(self):
        self.assertEqual(503, fetch(self.url + "/health")[0])

    def test_healthy_while_beats_are_fresh(self):
        self.status.update(True, self.status.snapshot)
        self.assertEqual(200, fetch(self.url + "/health")[0])
        self.clock.now += STALE_SECONDS
        self.assertEqual(503, fetch(self.url + "/health")[0])

    def test_status_shows_the_snapshot(self):
        player = Player()
        player.add([Track("1", "One More Time", "Daft Punk", "Discovery", "/x.flac")] * 2)
        player.start_shuffle()
        player.advance()
        self.status.update(True, summary(player, True, False, "Lounge"))
        code, body = fetch(self.url + "/status")
        self.assertEqual(200, code)
        self.assertEqual({"state": "Playing", "track": "Daft Punk - One More Time", "queue": 1,
                          "shuffle": "On", "channel": "Lounge"}, body)

    def test_idle_shows_nothing(self):
        player = Player()
        self.assertEqual({"state": "Idle", "track": "Nothing", "queue": 0, "shuffle": "Off", "channel": "None"},
                         summary(player, False, False, None))


if __name__ == "__main__":
    unittest.main()
