import unittest

from ratio_limit import pick_evictions

GB = 1024**3
DAY = 86400


def torrent(hash, size_gb, seeding_days=30, uploaded_gb=0, progress=1, completion_on=0):
    return {
        "hash": hash,
        "size": size_gb * GB,
        "progress": progress,
        "seeding_time": seeding_days * DAY,
        "uploaded": uploaded_gb * GB,
        "completion_on": completion_on,
    }


class PickEvictions(unittest.TestCase):
    def test_nothing_is_removed_under_the_limit(self):
        torrents = [torrent("a", 40), torrent("b", 50)]
        self.assertEqual(pick_evictions(torrents, 100 * GB, 7 * DAY), [])

    def test_the_slowest_uploader_goes_first_until_under_the_limit(self):
        torrents = [
            torrent("fast", 40, seeding_days=10, uploaded_gb=100),
            torrent("slow", 40, seeding_days=10, uploaded_gb=1),
            torrent("medium", 40, seeding_days=10, uploaded_gb=20),
        ]
        self.assertEqual(pick_evictions(torrents, 90 * GB, 7 * DAY), ["slow"])
        self.assertEqual(pick_evictions(torrents, 50 * GB, 7 * DAY), ["slow", "medium"])

    def test_upload_is_weighed_by_seeding_time(self):
        torrents = [
            torrent("old", 40, seeding_days=100, uploaded_gb=50),
            torrent("young", 40, seeding_days=10, uploaded_gb=10),
        ]
        self.assertEqual(pick_evictions(torrents, 50 * GB, 7 * DAY), ["old"])

    def test_a_torrent_below_the_minimum_seeding_time_is_kept(self):
        torrents = [
            torrent("new", 40, seeding_days=2, uploaded_gb=0),
            torrent("seeded", 40, seeding_days=20, uploaded_gb=30),
        ]
        self.assertEqual(pick_evictions(torrents, 50 * GB, 7 * DAY), ["seeded"])

    def test_a_download_in_progress_counts_but_is_kept(self):
        torrents = [
            torrent("downloading", 60, seeding_days=0, progress=0.3),
            torrent("seeded", 30, seeding_days=20, uploaded_gb=5),
        ]
        self.assertEqual(pick_evictions(torrents, 50 * GB, 7 * DAY), ["seeded"])

    def test_it_stops_when_nothing_more_can_be_removed(self):
        torrents = [
            torrent("new", 80, seeding_days=1),
            torrent("seeded", 30, seeding_days=20),
        ]
        self.assertEqual(pick_evictions(torrents, 50 * GB, 7 * DAY), ["seeded"])

    def test_equal_rates_remove_the_oldest_completed_first(self):
        torrents = [
            torrent("later", 40, seeding_days=10, completion_on=2000),
            torrent("earlier", 40, seeding_days=10, completion_on=1000),
        ]
        self.assertEqual(pick_evictions(torrents, 50 * GB, 7 * DAY), ["earlier"])


if __name__ == "__main__":
    unittest.main()
