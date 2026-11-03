import unittest

from ratio_grab import pick_grabs

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

    def test_no_slot_means_no_grab(self):
        self.assertEqual(pick([release("a", 70)], slots=0), [])


if __name__ == "__main__":
    unittest.main()
