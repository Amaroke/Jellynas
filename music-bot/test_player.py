import random
import unittest

from player import Player, Track
from rules import (
    ALONE_SECONDS,
    IDLE_SECONDS,
    control_refusal,
    play_refusal,
    should_leave,
)


def track(number):
    return Track(str(number), f"Song {number}", "Artist", "Album", f"/data/media/music/{number}.flac")


LIBRARY = [track(number) for number in range(10)]


class PlayerTest(unittest.TestCase):
    def setUp(self):
        self.player = Player(random.Random(4))

    def test_queue_plays_in_order_then_stops(self):
        self.player.add(LIBRARY[:3])
        self.assertEqual(LIBRARY[:3], [self.player.advance() for _ in range(3)])
        self.assertIsNone(self.player.advance())

    def test_shuffle_plays_every_track_once_before_a_repeat(self):
        self.player.start_shuffle()
        played = []
        for _ in range(len(LIBRARY)):
            if self.player.needs_library():
                self.player.refill(LIBRARY)
            played.append(self.player.advance())
        self.assertCountEqual(LIBRARY, played)
        self.assertTrue(self.player.needs_library())

    def test_shuffle_never_plays_a_track_twice_in_a_row(self):
        self.player.start_shuffle()
        previous = None
        for _ in range(len(LIBRARY) * 20):
            if self.player.needs_library():
                self.player.refill(LIBRARY)
            current = self.player.advance()
            self.assertNotEqual(previous, current)
            previous = current

    def test_queue_comes_before_shuffle(self):
        self.player.start_shuffle()
        self.player.refill(LIBRARY)
        self.player.advance()
        self.player.add([track(99)])
        self.assertEqual(track(99), self.player.advance())
        self.assertIn(self.player.advance(), LIBRARY)

    def test_shuffle_is_not_started_twice(self):
        self.assertTrue(self.player.start_shuffle())
        self.assertFalse(self.player.start_shuffle())

    def test_empty_library_turns_shuffle_off(self):
        self.player.start_shuffle()
        self.player.refill([])
        self.assertFalse(self.player.shuffle)
        self.assertIsNone(self.player.advance())

    def test_artist_is_queued_shuffled(self):
        self.player.add(LIBRARY, shuffled=True)
        queued = list(self.player.queue)
        self.assertCountEqual(LIBRARY, queued)
        self.assertNotEqual(LIBRARY, queued)

    def test_stop_clears_everything(self):
        self.player.add(LIBRARY)
        self.player.start_shuffle()
        self.player.advance()
        self.player.stop()
        self.assertEqual((0, False, None), (len(self.player.queue), self.player.shuffle, self.player.current))


class RulesTest(unittest.TestCase):
    def test_only_a_dj_commands(self):
        self.assertIsNotNone(play_refusal(False, 1, None, False))
        self.assertIsNotNone(control_refusal(False, 1, 1))

    def test_play_needs_a_voice_channel(self):
        self.assertIsNotNone(play_refusal(True, None, None, False))

    def test_play_refused_while_busy_elsewhere(self):
        self.assertIsNotNone(play_refusal(True, 1, 2, True))
        self.assertIsNone(play_refusal(True, 1, 2, False))
        self.assertIsNone(play_refusal(True, 1, 1, True))
        self.assertIsNone(play_refusal(True, 1, None, False))

    def test_controls_need_the_same_channel(self):
        self.assertIsNotNone(control_refusal(True, 1, None))
        self.assertIsNotNone(control_refusal(True, 2, 1))
        self.assertIsNotNone(control_refusal(True, None, 1))
        self.assertIsNone(control_refusal(True, 1, 1))

    def test_leaves_alone_after_a_minute(self):
        self.assertFalse(should_leave(100, 100 - ALONE_SECONDS + 1, None))
        self.assertTrue(should_leave(100, 100 - ALONE_SECONDS, None))

    def test_leaves_idle_after_five_minutes(self):
        self.assertFalse(should_leave(1000, None, 1000 - IDLE_SECONDS + 1))
        self.assertTrue(should_leave(1000, None, 1000 - IDLE_SECONDS))


if __name__ == "__main__":
    unittest.main()
