import unittest

from ratio_grab import deficit, grabs_in_last_day, parse_torznab, pick_grabs

GB = 1024**3


def release(infohash, leechers, seeders=1, size_gb=5, age_hours=1):
    return {
        "infohash": infohash,
        "title": f"Release {infohash}",
        "link": f"http://prowlarr:9696/3/download?{infohash}",
        "size": size_gb * GB,
        "seeders": seeders,
        "leechers": leechers,
        "age_hours": age_hours,
    }


RULES = {
    "min_size": 1 * GB,
    "max_size": 15 * GB,
    "max_age_hours": 6,
    "min_leechers": 10,
    "min_leechers_per_seeder": 5,
}


def pick(releases, known=(), slots=3, free_bytes=100 * GB):
    return [r["infohash"] for r in pick_grabs(releases, set(known), slots, free_bytes, RULES)]


class PickGrabs(unittest.TestCase):
    def test_the_hungriest_swarm_comes_first(self):
        releases = [release("a", 20, seeders=2), release("b", 70, seeders=1), release("c", 30, seeders=1)]
        self.assertEqual(pick(releases, slots=2), ["b", "c"])

    def test_a_swarm_with_too_few_leechers_is_skipped(self):
        self.assertEqual(pick([release("a", 9, seeders=0)]), [])

    def test_a_swarm_with_too_many_seeders_per_leecher_is_skipped(self):
        self.assertEqual(pick([release("a", 40, seeders=9)]), [])

    def test_a_release_without_any_seeder_is_skipped(self):
        self.assertEqual(pick([release("a", 40, seeders=0)]), [])

    def test_size_and_age_bounds_apply(self):
        releases = [
            release("small", 40, size_gb=0.5),
            release("big", 40, size_gb=20),
            release("old", 40, age_hours=8),
            release("fine", 40),
        ]
        self.assertEqual(pick(releases), ["fine"])

    def test_a_release_already_in_qbittorrent_is_skipped(self):
        self.assertEqual(pick([release("a", 40), release("b", 30)], known={"a"}), ["b"])

    def test_grabs_stop_when_the_ratio_folder_has_no_room(self):
        releases = [release("a", 70, size_gb=8), release("b", 50, size_gb=8), release("c", 30, size_gb=3)]
        self.assertEqual(pick(releases, free_bytes=12 * GB), ["a", "c"])

    def test_the_same_release_twice_is_grabbed_once(self):
        self.assertEqual(pick([release("a", 70), release("a", 70)]), ["a"])

    def test_the_hungriest_is_measured_per_seeder(self):
        releases = [release("one", 50, seeders=1), release("two", 90, seeders=2)]
        self.assertEqual(pick(releases, slots=1), ["one"])

    def test_no_slot_means_no_grab(self):
        self.assertEqual(pick([release("a", 70)], slots=0), [])


class GrabsInLastDay(unittest.TestCase):
    def test_a_grab_removed_since_still_counts(self):
        now = 100000
        present = [{"hash": "a", "added_on": now - 100}]
        remembered = {"a": now - 100, "b": now - 200}
        self.assertEqual(grabs_in_last_day(present, remembered, now), 2)

    def test_grabs_older_than_a_day_do_not_count(self):
        now = 100000
        present = [{"hash": "a", "added_on": now - 90000}]
        remembered = {"b": now - 90000, "c": now - 10}
        self.assertEqual(grabs_in_last_day(present, remembered, now), 1)


class Deficit(unittest.TestCase):
    def test_it_is_what_was_downloaded_minus_what_was_uploaded(self):
        torrents = [
            {"downloaded": 10 * GB, "uploaded": 2 * GB},
            {"downloaded": 5 * GB, "uploaded": 8 * GB},
        ]
        self.assertEqual(deficit(torrents), 5 * GB)

    def test_an_upload_surplus_is_a_negative_deficit(self):
        self.assertEqual(deficit([{"downloaded": 1 * GB, "uploaded": 4 * GB}]), -3 * GB)


FEED = b"""<?xml version="1.0"?>
<rss xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>
<item><title>Good</title><link>http://prowlarr/1</link><size>100</size>
<pubDate>Tue, 03 Nov 2026 22:00:00 +0100</pubDate>
<torznab:attr name="infohash" value="ABC"/><torznab:attr name="seeders" value="2"/><torznab:attr name="peers" value="12"/></item>
<item><title>No date</title><link>http://prowlarr/2</link><size>100</size>
<torznab:attr name="infohash" value="DEF"/></item>
<item><title>No hash</title><link>http://prowlarr/3</link><size>100</size>
<pubDate>Tue, 03 Nov 2026 22:00:00 +0100</pubDate></item>
</channel></rss>"""


class ParseTorznab(unittest.TestCase):
    def test_items_without_a_date_or_a_hash_are_skipped(self):
        releases = parse_torznab(FEED, 1793743200)
        self.assertEqual([r["infohash"] for r in releases], ["abc"])
        self.assertEqual(releases[0]["leechers"], 10)
        self.assertAlmostEqual(releases[0]["age_hours"], 1.0)


if __name__ == "__main__":
    unittest.main()
