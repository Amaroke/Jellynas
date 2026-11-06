import http.server
import os
import tempfile
import threading
import unittest

from ratio import (
    DAY,
    GB,
    QBittorrent,
    is_rare,
    load_state,
    parse_torznab,
    pick_evictions,
    pick_hungry,
    run_once,
    save_state,
)

NOW = 1_800_000_000
TRACKER = "https://tracker.c411.org/announce"


def torrent(hash, category="ratio", size_gb=3, seeding_days=10, uploaded_gb=0, downloaded_gb=0, progress=1,
            seeders=100, tracker=TRACKER, completion_on=0):
    return {
        "hash": hash,
        "name": f"Torrent {hash}",
        "category": category,
        "size": size_gb * GB,
        "progress": progress,
        "seeding_time": seeding_days * DAY,
        "uploaded": uploaded_gb * GB,
        "downloaded": downloaded_gb * GB,
        "completion_on": completion_on,
        "added_on": NOW - seeding_days * DAY,
        "num_complete": seeders,
        "tracker": tracker,
    }


def release(infohash, leechers=40, seeders=1, size_gb=3, age_hours=0.5, title=None, free=False):
    return {
        "infohash": infohash,
        "title": title or f"Release.{infohash}.1080p",
        "link": f"http://prowlarr/{infohash}.torrent",
        "size": int(size_gb * GB),
        "seeders": seeders,
        "leechers": leechers,
        "age_hours": age_hours,
        "free": free,
    }


class FakeQBittorrent:
    def __init__(self, torrents=()):
        self.list = list(torrents)
        self.added = []
        self.deleted = []
        self.moved = []
        self.made = {}
        self.prefs = {}

    def torrents(self):
        return [dict(t) for t in self.list]

    def add(self, torrent, category):
        self.added.append((torrent.decode(), category))

    def delete(self, hashes):
        self.deleted += hashes
        self.list = [t for t in self.list if t["hash"] not in hashes]

    def set_category(self, hashes, category):
        self.moved += [(h, category) for h in hashes]
        for t in self.list:
            if t["hash"] in hashes:
                t["category"] = category

    def categories(self):
        return {"ratio": {}, "zen": {}, **self.made}

    def create_category(self, name, path):
        self.made[name] = path

    def preferences(self):
        return dict(self.prefs)

    def set_preferences(self, preferences):
        self.prefs.update(preferences)


class FakeProwlarr:
    def __init__(self, newest=(), twins=()):
        self.feed = list(newest)
        self.twins = list(twins)
        self.searches = 0

    def newest(self, indexer, now):
        return list(self.feed)

    def search(self, indexer, title, now):
        self.searches += 1
        return list(self.twins)

    def download(self, link):
        return link.encode()


SETTINGS = {
    "tracker": "c411",
    "indexers": ["3"],
    "twin_indexers": ["9"],
    "cross_seed_key": "",
    "farm_size": 100 * GB,
    "zen_size": 50 * GB,
    "zen_min_torrents": 0,
    "zen_hunt_surplus": 20 * GB,
    "min_seeding": 3 * DAY,
    "stale_after": 7 * DAY,
    "stale_upload": 0.2,
    "max_deficit": 5 * GB,
    "per_day": 3,
    "max_downloading": 2,
    "twin_lookups": 10,
    "library_categories": ("radarr", "tv-sonarr"),
    "adopt_after": 6 * 3600,
    "tracker_seeders_in_peers": 2,
    "rules": {
        "min_size": 1 * GB,
        "max_size": 5 * GB,
        "max_age_hours": 1,
        "min_leechers": 10,
        "min_leechers_per_seeder": 5,
    },
}


def settings(**changes):
    return {**SETTINGS, **changes}


def empty_state(**changes):
    return {"transfers": {}, "grabbed": {}, "twins": {}, "uploads": {}, "free": {}, "paused": False, **changes}


def run(qbt, prowlarr=None, state=None, now=NOW, **changes):
    state = state if state is not None else empty_state()
    run_once(qbt, prowlarr or FakeProwlarr(), settings(**changes), state, now)
    return state


class Farm(unittest.TestCase):
    def test_a_hungry_release_with_a_public_twin_downloads_the_twin(self):
        qbt = FakeQBittorrent()
        prowlarr = FakeProwlarr([release("hungry")], twins=[release("public", seeders=200)])
        state = run(qbt, prowlarr)
        self.assertEqual(qbt.added, [("http://prowlarr/public.torrent", "ratio")])
        self.assertEqual(state["twins"], {"public": ["hungry", "ratio"]})
        self.assertEqual(state["grabbed"], {})

    def test_a_twin_is_grabbed_even_while_paused(self):
        qbt = FakeQBittorrent([torrent("old", downloaded_gb=20, progress=1)])
        prowlarr = FakeProwlarr([release("hungry")], twins=[release("public")])
        run(qbt, prowlarr)
        self.assertEqual(qbt.added, [("http://prowlarr/public.torrent", "ratio")])

    def test_a_twin_must_have_the_exact_same_size(self):
        qbt = FakeQBittorrent()
        prowlarr = FakeProwlarr([release("hungry", size_gb=3)], twins=[release("public", size_gb=3.01)])
        run(qbt, prowlarr)
        self.assertEqual(qbt.added, [("http://prowlarr/hungry.torrent", "ratio")])

    def test_without_a_twin_the_tracker_is_paid_and_remembered(self):
        qbt = FakeQBittorrent()
        state = run(qbt, FakeProwlarr([release("hungry")]))
        self.assertEqual(qbt.added, [("http://prowlarr/hungry.torrent", "ratio")])
        self.assertEqual(state["grabbed"], {"hungry": NOW})

    def test_a_deficit_over_the_limit_pauses_paid_grabs(self):
        qbt = FakeQBittorrent([torrent("old", downloaded_gb=20, uploaded_gb=2, category="radarr")])
        state = run(qbt, FakeProwlarr([release("hungry")]))
        self.assertEqual(qbt.added, [])
        self.assertTrue(state["paused"])

    def test_a_freeleech_release_is_grabbed_even_while_paused(self):
        qbt = FakeQBittorrent([torrent("old", category="radarr", downloaded_gb=20)])
        state = run(qbt, FakeProwlarr([release("free", free=True)]))
        self.assertEqual(qbt.added, [("http://prowlarr/free.torrent", "ratio")])
        self.assertEqual(state["free"], {"free": NOW})
        self.assertEqual(state["grabbed"], {})

    def test_a_freeleech_download_does_not_count_in_the_deficit(self):
        qbt = FakeQBittorrent([torrent("free", downloaded_gb=4, uploaded_gb=1)])
        state = run(qbt, state=empty_state(free={"free": NOW - 100}))
        self.assertEqual(state["transfers"], {"free": [0, 1 * GB]})

    def test_three_paid_grabs_a_day_at_most(self):
        qbt = FakeQBittorrent()
        state = empty_state(grabbed={"a": NOW - 100, "b": NOW - 200, "c": NOW - 300})
        run(qbt, FakeProwlarr([release("hungry")]), state)
        self.assertEqual(qbt.added, [])

    def test_two_downloads_at_once_at_most(self):
        qbt = FakeQBittorrent([torrent("a", progress=0.5), torrent("b", category="zen", progress=0.1)])
        run(qbt, FakeProwlarr([release("hungry")], twins=[release("public")]))
        self.assertEqual(qbt.added, [])

    def test_a_cold_or_old_release_is_left_alone(self):
        releases = [release("cold", leechers=5), release("old", age_hours=3)]
        self.assertEqual(pick_hungry(releases, set(), SETTINGS["rules"]), [])

    def test_the_hungriest_swarm_comes_first(self):
        releases = [release("a", leechers=20, seeders=2), release("b", leechers=70), release("c", leechers=30)]
        self.assertEqual([r["infohash"] for r in pick_hungry(releases, set(), SETTINGS["rules"])], ["b", "c", "a"])


class Twins(unittest.TestCase):
    def test_once_the_tracker_copy_seeds_the_twin_is_removed(self):
        qbt = FakeQBittorrent([
            torrent("public", tracker="http://nyaa.tracker.wf/announce"),
            torrent("hungry", category="cross-seed-link"),
        ])
        state = run(qbt, state=empty_state(twins={"public": ["hungry", "ratio"]}))
        self.assertEqual(qbt.deleted, ["public"])
        self.assertEqual(qbt.moved, [])
        self.assertEqual(state["twins"], {})

    def test_a_zen_twin_moves_its_tracker_copy_to_the_reserve(self):
        qbt = FakeQBittorrent([
            torrent("public", category="zen", tracker="http://nyaa.tracker.wf/announce"),
            torrent("rare", category="cross-seed-link", seeders=3),
        ])
        run(qbt, state=empty_state(twins={"public": ["rare", "zen"]}))
        self.assertEqual(qbt.moved, [("rare", "zen")])
        self.assertEqual(qbt.deleted, ["public"])

    def test_the_twin_stays_until_cross_seed_adds_the_tracker_copy(self):
        qbt = FakeQBittorrent([torrent("public", tracker="http://nyaa.tracker.wf/announce")])
        state = run(qbt, state=empty_state(twins={"public": ["hungry", "ratio"]}))
        self.assertEqual(qbt.deleted, [])
        self.assertEqual(state["twins"], {"public": ["hungry", "ratio"]})


class Cleaning(unittest.TestCase):
    def week_old_samples(self, *hashes, uploaded_gb=0):
        return {h: [[NOW - 7 * DAY - 60, uploaded_gb * GB]] for h in hashes}

    def test_a_torrent_that_uploaded_too_little_this_week_is_removed(self):
        qbt = FakeQBittorrent([torrent("dead", size_gb=4, uploaded_gb=10)])
        run(qbt, state=empty_state(uploads=self.week_old_samples("dead", uploaded_gb=9.5)))
        self.assertEqual(qbt.deleted, ["dead"])

    def test_a_torrent_that_uploaded_enough_this_week_stays(self):
        qbt = FakeQBittorrent([torrent("alive", size_gb=4, uploaded_gb=10)])
        run(qbt, state=empty_state(uploads=self.week_old_samples("alive", uploaded_gb=9)))
        self.assertEqual(qbt.deleted, [])

    def test_a_torrent_younger_than_a_week_of_history_stays(self):
        qbt = FakeQBittorrent([torrent("young", size_gb=4)])
        state = run(qbt)
        self.assertEqual(qbt.deleted, [])
        self.assertEqual(state["uploads"], {"young": [[NOW, 0]]})

    def test_a_dead_but_rare_tracker_torrent_moves_to_the_zen_reserve(self):
        qbt = FakeQBittorrent([torrent("rare", seeders=4)])
        run(qbt, state=empty_state(uploads=self.week_old_samples("rare")))
        self.assertEqual(qbt.moved, [("rare", "zen")])
        self.assertEqual(qbt.deleted, [])

    def test_a_dead_rare_public_torrent_is_removed(self):
        qbt = FakeQBittorrent([torrent("public", seeders=4, tracker="http://nyaa.tracker.wf/announce")])
        run(qbt, state=empty_state(uploads=self.week_old_samples("public")))
        self.assertEqual(qbt.deleted, ["public"])

    def test_a_full_zen_reserve_takes_no_more(self):
        qbt = FakeQBittorrent([torrent("rare", seeders=4), torrent("kept", category="zen", size_gb=49, seeders=2)])
        run(qbt, state=empty_state(uploads=self.week_old_samples("rare")))
        self.assertEqual(qbt.deleted, ["rare"])

    def test_a_full_farm_removes_the_slowest_uploader(self):
        qbt = FakeQBittorrent([
            torrent("fast", size_gb=40, uploaded_gb=100),
            torrent("slow", size_gb=40, uploaded_gb=10),
            torrent("medium", size_gb=40, uploaded_gb=50),
        ])
        run(qbt)
        self.assertEqual(qbt.deleted, ["slow"])

    def test_the_minimum_seeding_time_is_always_kept(self):
        qbt = FakeQBittorrent([torrent("new", size_gb=80, seeding_days=1), torrent("old", size_gb=30)])
        run(qbt)
        self.assertEqual(qbt.deleted, ["old"])

    def test_a_zen_torrent_that_is_no_longer_rare_leaves(self):
        qbt = FakeQBittorrent([torrent("crowded", category="zen", seeders=80), torrent("rare", category="zen", seeders=3)])
        run(qbt)
        self.assertEqual(qbt.deleted, ["crowded"])

    def test_an_overfull_zen_reserve_drops_the_least_rare(self):
        qbt = FakeQBittorrent([
            torrent("rarest", category="zen", size_gb=30, seeders=2),
            torrent("rare", category="zen", size_gb=30, seeders=20),
        ])
        run(qbt)
        self.assertEqual(qbt.deleted, ["rare"])


class ZenReserve(unittest.TestCase):
    def test_a_rare_release_with_a_twin_fills_the_reserve(self):
        qbt = FakeQBittorrent()
        prowlarr = FakeProwlarr([release("rare", leechers=0, seeders=3)], twins=[release("public")])
        state = run(qbt, prowlarr, zen_min_torrents=15)
        self.assertEqual(qbt.added, [("http://prowlarr/public.torrent", "zen")])
        self.assertEqual(state["twins"], {"public": ["rare", "zen"]})

    def test_without_a_twin_the_hunt_waits_for_a_surplus(self):
        qbt = FakeQBittorrent([torrent("old", category="radarr", downloaded_gb=10, uploaded_gb=25)])
        run(qbt, FakeProwlarr([release("rare", leechers=0, seeders=3)]), zen_min_torrents=15)
        self.assertEqual(qbt.added, [])

    def test_a_surplus_over_the_threshold_opens_the_hunt(self):
        qbt = FakeQBittorrent([torrent("old", category="radarr", downloaded_gb=10, uploaded_gb=35)])
        run(qbt, FakeProwlarr([release("rare", leechers=0, seeders=3)]), zen_min_torrents=15)
        self.assertEqual(qbt.added, [("http://prowlarr/rare.torrent", "zen")])

    def test_the_reserve_comes_first_until_it_holds_enough_torrents(self):
        qbt = FakeQBittorrent()
        prowlarr = FakeProwlarr([release("hungry", leechers=80), release("rare", leechers=0, seeders=3)],
                                twins=[release("public")])
        state = run(qbt, prowlarr, zen_min_torrents=15, max_downloading=1)
        self.assertEqual(state["twins"], {"public": ["rare", "zen"]})

    def test_rare_depends_on_the_size(self):
        self.assertTrue(is_rare(GB // 2, 24))
        self.assertFalse(is_rare(GB // 2, 25))
        self.assertTrue(is_rare(50 * GB, 50))
        self.assertFalse(is_rare(50 * GB, 51))
        self.assertTrue(is_rare(200 * GB, 199))


class Adoption(unittest.TestCase):
    def test_a_rare_tracker_torrent_of_radarr_joins_the_zen_reserve(self):
        qbt = FakeQBittorrent([torrent("movie", category="radarr", seeders=6)])
        run(qbt)
        self.assertEqual(qbt.moved, [("movie", "zen")])

    def test_a_crowded_public_or_fresh_library_torrent_stays_with_radarr(self):
        qbt = FakeQBittorrent([
            torrent("crowded", category="radarr", seeders=80),
            torrent("public", category="tv-sonarr", seeders=3, tracker="http://nyaa.tracker.wf/announce"),
            torrent("fresh", category="tv-sonarr", seeders=3, seeding_days=0.1),
        ])
        run(qbt)
        self.assertEqual(qbt.moved, [])


class Ledger(unittest.TestCase):
    def test_only_tracker_torrents_count_and_removed_ones_stay(self):
        qbt = FakeQBittorrent([
            torrent("c411", category="radarr", downloaded_gb=6, uploaded_gb=1),
            torrent("public", tracker="http://nyaa.tracker.wf/announce", downloaded_gb=9),
        ])
        state = run(qbt, state=empty_state(transfers={"gone": [2 * GB, 0]}))
        self.assertEqual(state["transfers"], {"c411": [6 * GB, 1 * GB], "gone": [2 * GB, 0]})


class Setup(unittest.TestCase):
    def test_categories_and_the_cross_seed_hook_are_set(self):
        qbt = FakeQBittorrent()
        qbt.categories = lambda: {}
        run(qbt, cross_seed_key="secret")
        self.assertEqual(qbt.made, {"ratio": "/data/torrents/ratio", "zen": "/data/torrents/zen"})
        self.assertTrue(qbt.prefs["autorun_enabled"])
        self.assertIn('X-Api-Key: secret', qbt.prefs["autorun_program"])
        self.assertIn("infoHash=%I", qbt.prefs["autorun_program"])


class Evictions(unittest.TestCase):
    def test_upload_is_weighed_by_seeding_time(self):
        torrents = [torrent("old", size_gb=40, seeding_days=100, uploaded_gb=50), torrent("young", size_gb=40, uploaded_gb=10)]
        self.assertEqual([t["hash"] for t in pick_evictions(torrents, 50 * GB, 3 * DAY)], ["old"])

    def test_a_download_in_progress_counts_but_is_kept(self):
        torrents = [torrent("downloading", size_gb=60, seeding_days=0, progress=0.3), torrent("seeded", size_gb=30)]
        self.assertEqual([t["hash"] for t in pick_evictions(torrents, 50 * GB, 3 * DAY)], ["seeded"])


class State(unittest.TestCase):
    def test_an_old_ratio_grab_file_still_loads(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "ratio-grab.json")
            save_state(path, {"transfers": {"a": [6 * GB, GB]}, "grabbed": {"a": 100}})
            self.assertEqual(load_state(path), empty_state(transfers={"a": [6 * GB, GB]}, grabbed={"a": 100}))

    def test_a_missing_file_is_an_empty_state(self):
        with tempfile.TemporaryDirectory() as folder:
            self.assertEqual(load_state(os.path.join(folder, "none.json")), empty_state())


FEED = b"""<?xml version="1.0"?>
<rss xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>
<item><title>Good</title><link>http://prowlarr/1</link><size>100</size>
<pubDate>Tue, 03 Nov 2026 22:00:00 +0100</pubDate>
<torznab:attr name="infohash" value="ABC"/><torznab:attr name="seeders" value="2"/><torznab:attr name="peers" value="12"/></item>
<item><title>No date</title><link>http://prowlarr/2</link><size>100</size>
<torznab:attr name="infohash" value="DEF"/></item>
</channel></rss>"""


C411_FEED = b"""<?xml version="1.0"?>
<rss xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>
<item><title>The.Ark.S03E03</title><link>http://prowlarr/1</link><size>100</size>
<pubDate>Tue, 03 Nov 2026 22:00:00 +0100</pubDate>
<torznab:attr name="infohash" value="ABC"/><torznab:attr name="seeders" value="87"/><torznab:attr name="peers" value="174"/>
<torznab:attr name="downloadvolumefactor" value="0"/></item>
</channel></rss>"""


class Torznab(unittest.TestCase):
    def test_c411_counts_each_seeder_twice_in_its_peers(self):
        releases = parse_torznab(C411_FEED, 1793743200, seeders_in_peers=2)
        self.assertEqual(releases[0]["leechers"], 0)
        self.assertTrue(releases[0]["free"])

    def test_items_without_a_date_are_skipped(self):
        releases = parse_torznab(FEED, 1793743200)
        self.assertEqual([r["infohash"] for r in releases], ["abc"])
        self.assertEqual(releases[0]["leechers"], 10)
        self.assertAlmostEqual(releases[0]["age_hours"], 1.0)


class FakeServer(http.server.BaseHTTPRequestHandler):
    calls = []
    expire_once = False

    def log_message(self, *args):
        pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers["Content-Length"]))
        FakeServer.calls.append((self.path, self.headers.get("Content-Type", ""), body))
        if self.path == "/api/v2/auth/login":
            self.answer(b"Ok.", {"Set-Cookie": "SID=abc; path=/"})
        elif FakeServer.expire_once:
            FakeServer.expire_once = False
            self.send_response(403)
            self.end_headers()
        else:
            self.answer(b"Ok.")

    def answer(self, body, headers=None):
        self.send_response(200)
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class RealClient(unittest.TestCase):
    def setUp(self):
        FakeServer.calls = []
        FakeServer.expire_once = False
        self.server = http.server.HTTPServer(("127.0.0.1", 0), FakeServer)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.qbt = QBittorrent(f"http://127.0.0.1:{self.server.server_port}", "admin", "pass")

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()

    def test_a_torrent_file_is_sent_as_multipart_with_its_category(self):
        self.qbt.add(b"d8:announce", "zen")
        path, content_type, body = FakeServer.calls[-1]
        self.assertEqual(path, "/api/v2/torrents/add")
        self.assertTrue(content_type.startswith("multipart/form-data; boundary="))
        self.assertIn(b'name="category"\r\n\r\nzen\r\n', body)
        self.assertIn(b"d8:announce", body)

    def test_an_expired_session_logs_in_again(self):
        self.qbt.delete(["a"])
        FakeServer.expire_once = True
        self.qbt.delete(["b"])
        self.assertEqual(
            [path for path, _, _ in FakeServer.calls],
            ["/api/v2/auth/login", "/api/v2/torrents/delete", "/api/v2/torrents/delete",
             "/api/v2/auth/login", "/api/v2/torrents/delete"],
        )


if __name__ == "__main__":
    unittest.main()
