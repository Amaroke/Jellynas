import contextlib
import http.server
import io
import json
import os
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from unittest import mock

from ratio import (
    C411,
    DAY,
    GB,
    Prowlarr,
    QBittorrent,
    Throttled,
    bdecode,
    is_rare,
    load_state,
    parse_torznab,
    pick_evictions,
    run_once,
    save_state,
    serve,
)

NOW = 1_800_000_000
TRACKER = "https://tracker.c411.org/announce"


def torrent(hash, category="ratio", size_gb=3, seeding_days=10, uploaded_gb=0, downloaded_gb=0, progress=1,
            seeders=100, tracker=TRACKER, completion_on=0, save_path=None, state="stalledUP"):
    return {
        "hash": hash,
        "save_path": save_path or f"/data/torrents/{category}",
        "state": state,
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
        "page": "",
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
        self.kept = []
        self.matched = []
        self.started = []

    def torrents(self):
        return [dict(t) for t in self.list]

    def add(self, torrent, category, save_path=None):
        name = bdecode(torrent)[0][b"info"][b"name"].decode()
        self.added.append((name, category) if save_path is None else (name, category, save_path))

    def files(self, infohash):
        return [{"name": f"{infohash}.mkv", "size": 7}]

    def match_files(self, infohash, wanted):
        self.matched.append((infohash, wanted))

    def start(self, hashes):
        self.started += hashes

    def delete(self, hashes, delete_files=True):
        if not delete_files:
            self.kept += hashes
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


def torrent_file(name, lengths):
    files = b"".join(b"d6:lengthi%de4:pathl1:xee" % length for length in lengths)
    return b"d4:infod5:filesl" + files + b"e4:name%d:" % len(name) + name.encode() + b"ee"


class FakeProwlarr:
    def __init__(self, newest=(), twins=(), files=None, throttle=None, broken=()):
        self.feed = list(newest)
        self.twins = list(twins)
        self.files = files or {}
        self.throttle = throttle
        self.broken = set(broken)
        self.queries = []
        self.reads = 0
        self.fetched = []
        self.holds = []
        self.restores = []

    def newest(self, indexer, now):
        self.reads += 1
        if self.throttle:
            raise Throttled(self.throttle)
        return list(self.feed)

    def search(self, indexer, title, now):
        self.queries.append(title)
        return list(self.twins)

    def torrent(self, release):
        infohash = release.get("infohash") or release["link"].rsplit("/", 1)[1].removesuffix(".torrent")
        self.fetched.append(infohash)
        if infohash in self.broken:
            raise RuntimeError("the tracker refused the torrent file")
        return torrent_file(infohash, self.files.get(infohash, [7]))

    def hold(self, indexer, paused, profile, restore=None):
        self.holds.append((indexer, paused, profile))
        self.restores.append(restore)
        return 7 if paused else None


class FakeC411:
    def __init__(self, uploaded_gb=60, downloaded_gb=30, zen=25, broken=False):
        self.uploaded = uploaded_gb * GB
        self.downloaded = downloaded_gb * GB
        self.zen = zen
        self.broken = broken
        self.reads = 0

    def account(self):
        self.reads += 1
        if self.broken:
            raise RuntimeError("Site is currently restricted")
        return {"uploaded": self.uploaded, "downloaded": self.downloaded, "zen": self.zen}


def under_floor():
    return FakeC411(uploaded_gb=30, downloaded_gb=30)


SETTINGS = {
    "tracker": "c411",
    "indexers": ["3"],
    "twin_indexers": ["9"],
    "cross_seed_key": "",
    "farm_size": 100 * GB,
    "zen_size": 50 * GB,
    "floor": 1.2,
    "paused_profile": "Paused",
    "zen_hunt_per_day": 3,
    "zen_hunt": {"min_size": 1 * GB, "max_size": 5 * GB},
    "free_max_size": 5 * GB,
    "min_seeding": 3 * DAY,
    "stale_after": 7 * DAY,
    "stale_upload": 0.2,
    "max_downloading": 2,
    "twin_lookups": 10,
    "library_categories": ("radarr", "tv-sonarr"),
    "adopt_after": 6 * 3600,
    "tracker_seeders_in_peers": 2,
    "twin_max_age_hours": 24,
    "twin_min_leechers": 1,
    "twin_recheck": 2 * 3600,
    "twin_seed_after": 3600,
}


def settings(**changes):
    return {**SETTINGS, **changes}


def empty_state(**changes):
    return {
        "transfers": {}, "grabbed": {}, "twins": {}, "uploads": {}, "free": {}, "checked": {}, "paused": False,
        "sources": {}, "history": {}, "zen": {}, "report": {}, "account": [], "held": None, "backoff_until": 0,
        "stuck": {}, "restore": {}, **changes,
    }


def run(qbt, prowlarr=None, state=None, now=NOW, c411=None, **changes):
    state = state if state is not None else empty_state()
    run_once(qbt, prowlarr or FakeProwlarr(), c411 or FakeC411(), settings(**changes), state, now)
    return state


EPISODE = "Les.Carnets.de.l.apothicaire.S03E06.VOSTFR.1080p.WEBRip.x265-KAF"
NYAA = "The.Apothecary.Diaries.S03E06.VOSTFR.1080p.WEBRiP.x265-KAF"


def episode(leechers=3):
    return release("episode", title=EPISODE, leechers=leechers, seeders=60, size_gb=0.27)


def twin(title=NYAA, size_gb=0.28):
    return release("public", title=title, seeders=200, leechers=5, size_gb=size_gb)


class Twins(unittest.TestCase):
    def test_a_fresh_episode_with_a_twin_downloads_the_twin(self):
        qbt = FakeQBittorrent()
        prowlarr = FakeProwlarr([episode()], twins=[twin()])
        state = run(qbt, prowlarr)
        self.assertEqual(prowlarr.queries, ["S03E06 VOSTFR"])
        self.assertEqual(qbt.added, [("public", "ratio")])
        self.assertEqual(state["twins"], {"public": ["episode", "ratio", NOW, "http://prowlarr/episode.torrent"]})
        self.assertEqual(state["grabbed"], {})

    def test_a_twin_is_grabbed_even_under_the_floor(self):
        qbt = FakeQBittorrent()
        run(qbt, FakeProwlarr([episode()], twins=[twin()]), c411=under_floor())
        self.assertEqual(qbt.added, [("public", "ratio")])

    def test_a_release_is_fetched_once_for_several_candidates(self):
        prowlarr = FakeProwlarr([episode()], twins=[twin(), twin()], files={"public": [8]})
        run(FakeQBittorrent(), prowlarr)
        self.assertEqual(prowlarr.fetched.count("episode"), 1)

    def test_a_twin_must_hold_the_same_files(self):
        qbt = FakeQBittorrent()
        prowlarr = FakeProwlarr([episode()], twins=[twin()], files={"public": [8]})
        state = run(qbt, prowlarr)
        self.assertEqual(qbt.added, [])
        self.assertEqual(state["twins"], {})

    def test_a_twin_must_come_from_the_same_group(self):
        qbt = FakeQBittorrent()
        run(qbt, FakeProwlarr([episode()], twins=[twin(title="Show.S03E06.VOSTFR.1080p-OTHER")]))
        self.assertEqual(qbt.added, [])

    def test_a_twin_size_may_be_rounded_but_not_far_off(self):
        qbt = FakeQBittorrent()
        run(qbt, FakeProwlarr([episode()], twins=[twin(size_gb=0.4)]))
        self.assertEqual(qbt.added, [])

    def test_a_release_without_an_episode_or_a_leecher_is_not_searched(self):
        prowlarr = FakeProwlarr([release("movie", leechers=0), episode(leechers=0)], twins=[twin()])
        run(FakeQBittorrent(), prowlarr)
        self.assertEqual(prowlarr.queries, [])

    def test_a_release_checked_recently_is_not_searched_again(self):
        prowlarr = FakeProwlarr([episode()], twins=[twin()])
        run(FakeQBittorrent(), prowlarr, empty_state(checked={"episode": NOW - 3600}))
        self.assertEqual(prowlarr.queries, [])

    def test_once_the_tracker_copy_seeds_the_twin_is_removed(self):
        qbt = FakeQBittorrent([
            torrent("public", tracker="http://nyaa.tracker.wf/announce"),
            torrent("hungry", category="cross-seed-link"),
        ])
        state = run(qbt, state=empty_state(twins={"public": ["hungry", "ratio", NOW - 7200, "http://prowlarr/hungry.torrent"]}))
        self.assertEqual(qbt.deleted, ["public"])
        self.assertEqual(qbt.moved, [])
        self.assertEqual(state["twins"], {})

    def test_the_twin_stays_until_cross_seed_adds_the_tracker_copy(self):
        qbt = FakeQBittorrent([torrent("public", tracker="http://nyaa.tracker.wf/announce")])
        twins = {"public": ["hungry", "ratio", NOW - 1800, "http://prowlarr/hungry.torrent"]}
        state = run(qbt, state=empty_state(twins=dict(twins)))
        self.assertEqual(qbt.deleted, [])
        self.assertEqual(qbt.added, [])
        self.assertEqual(state["twins"], twins)

    def test_when_cross_seed_misses_the_tracker_copy_ratio_seeds_it_from_the_twin_files(self):
        qbt = FakeQBittorrent([torrent("public", tracker="http://nyaa.tracker.wf/announce")])
        twins = {"public": ["hungry", "ratio", NOW - 3600, "http://prowlarr/hungry.torrent"]}
        run(qbt, state=empty_state(twins=twins))
        self.assertEqual(qbt.added, [("hungry", "ratio", "/data/torrents/ratio")])
        self.assertEqual(qbt.matched, [("hungry", [{"name": "public.mkv", "size": 7}])])

    def test_a_twin_sharing_its_files_with_the_tracker_copy_leaves_them_on_disk(self):
        qbt = FakeQBittorrent([
            torrent("public", tracker="http://nyaa.tracker.wf/announce"),
            torrent("hungry"),
        ])
        run(qbt, state=empty_state(twins={"public": ["hungry", "ratio", NOW - 7200, "http://prowlarr/hungry.torrent"]}))
        self.assertEqual(qbt.kept, ["public"])

    def test_a_tracker_copy_that_fails_waits_longer_before_each_retry(self):
        qbt = FakeQBittorrent([torrent("public", tracker="http://nyaa.tracker.wf/announce")])
        state = empty_state(twins={"public": ["hungry", "ratio", NOW - 3600, "http://prowlarr/hungry.torrent"]})
        prowlarr = FakeProwlarr(broken={"hungry"})
        for minutes in range(0, 120, 10):
            run(qbt, prowlarr, state, now=NOW + 60 * minutes)
        self.assertEqual(prowlarr.fetched, ["hungry", "hungry"])
        run(qbt, prowlarr, state, now=NOW + 3 * 3600)
        self.assertEqual(prowlarr.fetched, ["hungry", "hungry", "hungry"])

    def test_a_tracker_copy_left_stopped_after_its_check_is_started(self):
        qbt = FakeQBittorrent([
            torrent("public", tracker="http://nyaa.tracker.wf/announce"),
            torrent("hungry", state="stoppedUP"),
        ])
        run(qbt, state=empty_state(twins={"public": ["hungry", "ratio", NOW - 7200, "http://prowlarr/hungry.torrent"]}))
        self.assertEqual(qbt.started, ["hungry"])


class Farm(unittest.TestCase):
    def test_a_fresh_release_is_no_longer_paid_for_upload(self):
        qbt = FakeQBittorrent()
        state = run(qbt, FakeProwlarr([release("hungry", seeders=80, leechers=200)]))
        self.assertEqual(qbt.added, [])
        self.assertEqual(state["grabbed"], {})

    def test_a_freeleech_release_is_grabbed_even_under_the_floor(self):
        qbt = FakeQBittorrent()
        state = run(qbt, FakeProwlarr([release("free", seeders=30, free=True)]), c411=under_floor())
        self.assertEqual(qbt.added, [("free", "ratio")])
        self.assertEqual(state["free"], {"free": NOW})
        self.assertEqual(state["grabbed"], {})

    def test_a_freeleech_download_does_not_count_in_the_deficit(self):
        qbt = FakeQBittorrent([torrent("free", downloaded_gb=4, uploaded_gb=1)])
        state = run(qbt, state=empty_state(free={"free": NOW - 100}))
        self.assertEqual(state["transfers"], {"free": [0, 1 * GB]})

    def test_two_downloads_at_once_at_most(self):
        qbt = FakeQBittorrent([torrent("a", progress=0.5), torrent("b", category="zen", progress=0.1)])
        run(qbt, FakeProwlarr([release("free", free=True), episode()], twins=[twin()]))
        self.assertEqual(qbt.added, [])


class Floor(unittest.TestCase):
    def test_the_profile_is_read_once_per_run(self):
        c411 = FakeC411()
        run(FakeQBittorrent(), FakeProwlarr([episode(), release("rare", seeders=3)], twins=[twin()]), c411=c411)
        self.assertEqual(c411.reads, 1)

    def test_under_the_floor_nothing_paid_is_grabbed_and_c411_is_held_in_prowlarr(self):
        qbt = FakeQBittorrent()
        prowlarr = FakeProwlarr([release("rare", leechers=0, seeders=3)])
        state = run(qbt, prowlarr, c411=FakeC411(uploaded_gb=35, downloaded_gb=30))
        self.assertEqual(qbt.added, [])
        self.assertEqual(prowlarr.holds, [("3", True, "Paused")])
        self.assertTrue(state["paused"])
        self.assertTrue(state["held"])
        self.assertEqual(state["restore"], {"3": 7})

    def test_above_the_floor_again_c411_is_released_in_prowlarr(self):
        prowlarr = FakeProwlarr()
        state = run(FakeQBittorrent(), prowlarr, empty_state(paused=True, held=True, restore={"3": 7}))
        self.assertEqual(prowlarr.holds, [("3", False, "Paused")])
        self.assertEqual(prowlarr.restores, [7])
        self.assertFalse(state["paused"])
        self.assertFalse(state["held"])

    def test_prowlarr_is_only_told_when_the_floor_changes_side(self):
        prowlarr = FakeProwlarr()
        run(FakeQBittorrent(), prowlarr, empty_state(held=False))
        self.assertEqual(prowlarr.holds, [])

    def test_an_unreadable_profile_counts_as_under_the_floor(self):
        qbt = FakeQBittorrent()
        prowlarr = FakeProwlarr([release("rare", leechers=0, seeders=3)])
        state = run(qbt, prowlarr, c411=FakeC411(broken=True))
        self.assertEqual(qbt.added, [])
        self.assertEqual(prowlarr.holds, [("3", True, "Paused")])
        self.assertTrue(state["paused"])

    def test_a_paid_grab_never_spends_past_the_floor(self):
        qbt = FakeQBittorrent()
        prowlarr = FakeProwlarr([release("a", leechers=0, seeders=3), release("b", leechers=0, seeders=4)])
        run(qbt, prowlarr, c411=FakeC411(uploaded_gb=42, downloaded_gb=31))
        self.assertEqual(qbt.added, [("a", "zen")])

    def test_a_tracker_download_in_progress_counts_against_the_floor(self):
        qbt = FakeQBittorrent([torrent("movie", category="radarr", size_gb=4, progress=0.5)])
        prowlarr = FakeProwlarr([release("rare", leechers=0, seeders=3)])
        run(qbt, prowlarr, c411=FakeC411(uploaded_gb=42, downloaded_gb=31))
        self.assertEqual(qbt.added, [])


class Throttling(unittest.TestCase):
    def test_a_429_from_prowlarr_stops_its_use_until_the_wait_is_over(self):
        prowlarr = FakeProwlarr(throttle=1800)
        state = run(FakeQBittorrent(), prowlarr)
        self.assertEqual(state["backoff_until"], NOW + 1800)
        prowlarr.throttle = None
        run(FakeQBittorrent(), prowlarr, state, now=NOW + 600)
        self.assertEqual(prowlarr.reads, 1)
        run(FakeQBittorrent(), prowlarr, state, now=NOW + 1800)
        self.assertEqual(prowlarr.reads, 2)


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
    def test_above_the_floor_the_hunt_grabs_a_rare_release(self):
        qbt = FakeQBittorrent()
        state = run(qbt, FakeProwlarr([release("rare", leechers=0, seeders=3)]))
        self.assertEqual(qbt.added, [("rare", "zen")])
        self.assertEqual(state["grabbed"], {"rare": NOW})

    def test_three_hunted_grabs_a_day_at_most(self):
        qbt = FakeQBittorrent()
        state = empty_state(grabbed={"a": NOW - 100, "b": NOW - 200, "c": NOW - 300})
        run(qbt, FakeProwlarr([release("rare", leechers=0, seeders=3)]), state)
        self.assertEqual(qbt.added, [])

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


def report_of(state, source):
    return state["report"]["sources"].get(source)


class Sources(unittest.TestCase):
    def test_a_paid_grab_stays_a_paid_grab_once_removed(self):
        state = empty_state(sources={"hungry": "paid"})
        run(FakeQBittorrent([torrent("hungry", downloaded_gb=3, uploaded_gb=1)]), state=state)
        run(FakeQBittorrent(), state=state, now=NOW + 2 * DAY)
        self.assertEqual(state["sources"], {"hungry": "paid"})
        self.assertEqual(report_of(state, "paid")["torrents"], 1)
        self.assertEqual(report_of(state, "paid")["downloaded_gb"], 3)
        self.assertEqual(report_of(state, "paid")["uploaded_gb"], 1)

    def test_a_freeleech_grab_is_reported_as_freeleech(self):
        state = run(FakeQBittorrent(), FakeProwlarr([release("free", free=True)]))
        run(FakeQBittorrent([torrent("free", downloaded_gb=3, uploaded_gb=1)]), state=state, now=NOW + DAY)
        self.assertEqual(state["sources"], {"free": "freeleech"})
        self.assertEqual(report_of(state, "freeleech")["downloaded_gb"], 0)

    def test_a_zen_hunt_is_reported_as_a_zen_hunt(self):
        qbt = FakeQBittorrent([torrent("old", category="radarr", downloaded_gb=10, uploaded_gb=35)])
        state = run(qbt, FakeProwlarr([release("rare", leechers=0, seeders=3)]))
        self.assertEqual(state["sources"]["rare"], "zen-hunt")
        self.assertEqual(state["sources"]["old"], "library")

    def test_the_tracker_copy_of_a_twin_is_a_twin_whoever_adds_it(self):
        state = run(FakeQBittorrent(), FakeProwlarr([episode()], twins=[twin()]))
        self.assertEqual(state["sources"], {"episode": "twin"})
        qbt = FakeQBittorrent([
            torrent("public", tracker="http://nyaa.tracker.wf/announce"),
            torrent("episode", category="cross-seed-link", state="stoppedUP", tracker=""),
        ])
        run(qbt, state=state, now=NOW + 600)
        self.assertEqual(state["twins"], {})
        run(FakeQBittorrent([torrent("episode", category="cross-seed-link")]), state=state, now=NOW + 1200)
        self.assertEqual(state["sources"], {"episode": "twin"})

    def test_library_torrents_and_their_cross_seeds_are_told_apart(self):
        qbt = FakeQBittorrent([
            torrent("movie", category="radarr", seeders=100),
            torrent("show", category="tv-sonarr", seeders=100),
            torrent("linked", category="cross-seed-link"),
            torrent("public", category="radarr", tracker="http://nyaa.tracker.wf/announce"),
        ])
        state = run(qbt)
        self.assertEqual(state["sources"], {"movie": "library", "show": "library", "linked": "cross-seed"})

    def test_a_tracker_torrent_without_a_source_gets_one_from_what_ratio_sees(self):
        qbt = FakeQBittorrent([
            torrent("farmed", downloaded_gb=4),
            torrent("free", downloaded_gb=4),
            torrent("kept", category="zen", seeders=3),
            torrent("movie", category="radarr", seeders=100),
        ])
        transfers = {h: [GB, 0] for h in ("farmed", "free", "kept", "movie", "gone")}
        state = run(qbt, state=empty_state(transfers=transfers, free={"free": NOW - 100}))
        self.assertEqual(state["sources"], {
            "farmed": "paid", "free": "freeleech", "kept": "zen-hunt", "movie": "library", "gone": "unknown",
        })
        self.assertEqual(report_of(state, "unknown")["downloaded_gb"], 1)

    def test_a_pending_source_that_never_shows_up_is_forgotten(self):
        state = run(FakeQBittorrent(), state=empty_state(sources={"lost": "paid"}))
        self.assertEqual(state["sources"], {})


class Report(unittest.TestCase):
    def run_quietly(self, qbt, state, now):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            run(qbt, state=state, now=now)
        return [line for line in out.getvalue().splitlines() if line.startswith("Source ")]

    def test_the_report_is_logged_and_saved_once_a_day(self):
        qbt = FakeQBittorrent([torrent("movie", category="radarr", seeders=100, downloaded_gb=4, uploaded_gb=2)])
        state = empty_state()
        lines = self.run_quietly(qbt, state, NOW)
        self.assertEqual(len(lines), 1)
        self.assertIn("Library download", lines[0])
        self.assertEqual(state["report"]["at"], NOW)
        self.assertEqual(report_of(state, "library")["ratio"], 0.5)
        self.assertEqual(self.run_quietly(qbt, state, NOW + 600), [])
        self.assertEqual(state["report"]["at"], NOW)
        self.assertEqual(len(self.run_quietly(qbt, state, NOW + DAY)), 1)

    def test_the_week_counts_only_the_last_seven_days(self):
        state = empty_state(
            transfers={"movie": [5 * GB, GB], "gone": [GB, GB]},
            sources={"movie": "library", "gone": "library"},
            history={
                "movie": [[NOW - 9 * DAY, GB, 0], [NOW - 6.5 * DAY, 2 * GB, GB // 2]],
                "gone": [[NOW - 8 * DAY, GB, GB]],
            },
        )
        qbt = FakeQBittorrent([torrent("movie", category="radarr", seeders=100, downloaded_gb=6, uploaded_gb=3)])
        self.run_quietly(qbt, state, NOW)
        figures = report_of(state, "library")
        self.assertEqual(figures["downloaded_gb"], 7)
        self.assertEqual(figures["week_downloaded_gb"], 4)
        self.assertEqual(figures["week_uploaded_gb"], 2.5)
        self.assertEqual(state["history"]["movie"][0], [NOW - 6.5 * DAY, 2 * GB, GB // 2])
        self.assertNotIn("gone", state["history"])

    def test_a_new_torrent_counts_whole_in_the_week(self):
        state = empty_state()
        qbt = FakeQBittorrent([torrent("movie", category="radarr", seeders=100, seeding_days=2, downloaded_gb=6)])
        self.run_quietly(qbt, state, NOW)
        self.assertEqual(report_of(state, "library")["week_downloaded_gb"], 6)

    def test_a_torrent_removed_this_week_still_counts(self):
        state = empty_state()
        qbt = FakeQBittorrent([torrent("movie", category="radarr", seeders=100, seeding_days=2, downloaded_gb=6)])
        self.run_quietly(qbt, state, NOW)
        self.run_quietly(FakeQBittorrent(), state, NOW + DAY)
        self.assertEqual(report_of(state, "library")["week_downloaded_gb"], 6)
        self.run_quietly(FakeQBittorrent(), state, NOW + 8 * DAY)
        self.assertEqual(report_of(state, "library")["week_downloaded_gb"], 0)

    def test_days_in_the_zen_reserve_count_from_the_move(self):
        state = empty_state()
        self.run_quietly(FakeQBittorrent([torrent("kept", category="zen", seeders=3, seeding_days=10)]), state, NOW)
        self.run_quietly(FakeQBittorrent([torrent("kept", category="zen", seeders=3, seeding_days=13)]), state, NOW + 3 * DAY)
        self.assertEqual(report_of(state, "zen-hunt")["zen_days"], 3)
        self.run_quietly(FakeQBittorrent(), state, NOW + 4 * DAY)
        self.assertEqual(report_of(state, "zen-hunt")["zen_days"], 3)

    def test_a_torrent_added_again_starts_over_without_losing_what_it_did(self):
        state = empty_state()
        self.run_quietly(FakeQBittorrent([torrent("kept", category="zen", seeders=3, seeding_days=5, downloaded_gb=6)]), state, NOW)
        self.run_quietly(FakeQBittorrent([torrent("kept", category="zen", seeders=3, seeding_days=6, downloaded_gb=6)]), state, NOW + DAY)
        again = torrent("kept", category="zen", seeders=3, seeding_days=1, downloaded_gb=2)
        again["added_on"] = NOW + DAY
        self.run_quietly(FakeQBittorrent([again]), state, NOW + 2 * DAY)
        figures = report_of(state, "zen-hunt")
        self.assertEqual(figures["zen_days"], 2)
        self.assertEqual(figures["week_downloaded_gb"], 2)


def summary_of(qbt, state=None, now=NOW, c411=None, **changes):
    state = state if state is not None else empty_state()
    with contextlib.redirect_stdout(io.StringIO()):
        return run_once(qbt, FakeProwlarr(), c411 or FakeC411(), settings(**changes), state, now)


class Summary(unittest.TestCase):
    def test_the_ledger_counts_every_tracker_torrent_removed_ones_included(self):
        qbt = FakeQBittorrent([
            torrent("farmed", downloaded_gb=3, uploaded_gb=6),
            torrent("kept", category="zen", size_gb=2, seeders=3, downloaded_gb=2, uploaded_gb=1),
            torrent("movie", category="radarr", size_gb=4, seeders=100, downloaded_gb=4),
            torrent("public", tracker="http://nyaa.tracker.wf/announce", downloaded_gb=9),
        ])
        summary = summary_of(qbt, empty_state(transfers={"gone": [GB, 0]}))
        self.assertEqual(summary["ledger"], {
            "torrents": 4, "downloaded_gb": 10, "uploaded_gb": 7, "ratio": 0.7, "surplus_gb": -3,
            "week_downloaded_gb": 0, "week_uploaded_gb": 0, "week_ratio": None,
        })

    def test_the_farm_and_the_zen_reserve_show_their_fill(self):
        qbt = FakeQBittorrent([
            torrent("farmed", size_gb=3),
            torrent("linked", category="cross-seed-link", size_gb=2, progress=0.5),
            torrent("kept", category="zen", size_gb=4, seeders=3),
        ])
        summary = summary_of(qbt, indexers=[])
        self.assertEqual(summary["farm"], {"torrents": 2, "downloading": 1, "size_gb": 5, "limit_gb": 100, "fill": 5.0})
        self.assertEqual(summary["zen"], {"torrents": 1, "downloading": 0, "size_gb": 4, "limit_gb": 50, "fill": 8.0})

    def test_the_account_shows_its_ratio_its_zen_and_its_upload_of_the_week(self):
        state = empty_state(account=[
            [NOW - 8 * DAY, 50 * GB, 20 * GB, 10],
            [NOW - 7 * DAY, 55 * GB, 25 * GB, 15],
            [NOW - DAY, 58 * GB, 28 * GB, 20],
        ])
        summary = summary_of(FakeQBittorrent(), state, indexers=[])
        self.assertEqual(summary["account"], {
            "ratio": 2.0, "floor": 1.2, "paused": False, "zen": 25, "week_uploaded_gb": 5,
        })
        self.assertNotIn("grabs", summary)

    def test_a_new_account_counts_its_week_from_its_first_reading(self):
        state = empty_state(account=[[NOW - 2 * DAY, 57 * GB, 28 * GB, 20]])
        self.assertEqual(summary_of(FakeQBittorrent(), state, indexers=[])["account"]["week_uploaded_gb"], 3)

    def test_an_unreadable_account_shows_the_pause_and_no_figures(self):
        summary = summary_of(FakeQBittorrent(), c411=FakeC411(broken=True), indexers=[])
        self.assertEqual(summary["account"], {
            "ratio": None, "floor": 1.2, "paused": True, "zen": None, "week_uploaded_gb": None,
        })

    def test_each_source_is_listed_by_name_with_its_figures(self):
        qbt = FakeQBittorrent([torrent("movie", category="radarr", seeders=100, downloaded_gb=4, uploaded_gb=2)])
        summary = summary_of(qbt)
        self.assertEqual(summary["sources"], [
            {"name": "Library download", "figures": "1 torrent, 2.0 / 4.0 GB, ratio 0.50"},
        ])

    def test_the_time_of_the_summary_is_an_iso_date(self):
        self.assertEqual(summary_of(FakeQBittorrent())["updated_at"], "2027-01-15T08:00:00+00:00")


class Server(unittest.TestCase):
    def test_the_last_summary_is_served_as_json_and_nothing_else(self):
        latest = {"summary": {"ledger": {"ratio": 1.5}}}
        server = serve(latest, port=0)
        try:
            url = f"http://127.0.0.1:{server.server_port}"
            with urllib.request.urlopen(f"{url}/summary") as response:
                self.assertEqual(json.load(response), {"ledger": {"ratio": 1.5}})
            latest["summary"] = {"ledger": {"ratio": 2}}
            with urllib.request.urlopen(f"{url}/summary") as response:
                self.assertEqual(json.load(response), {"ledger": {"ratio": 2}})
            with self.assertRaises(urllib.error.HTTPError) as error:
                urllib.request.urlopen(f"{url}/state")
            self.assertEqual(error.exception.code, 404)
            error.exception.close()
        finally:
            server.shutdown()
            server.server_close()


class Setup(unittest.TestCase):
    def test_categories_and_the_cross_seed_hook_are_set(self):
        qbt = FakeQBittorrent()
        qbt.categories = dict
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
    def test_a_file_without_the_newer_keys_still_loads(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "state.json")
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


class TorrentFiles(unittest.TestCase):
    def fetched_url(self, release):
        response = mock.MagicMock()
        response.__enter__.return_value.read.return_value = b"torrent"
        with mock.patch("urllib.request.urlopen", return_value=response) as urlopen:
            Prowlarr("http://prowlarr:9696", "key", 2).torrent(release)
        return urlopen.call_args.args[0].full_url

    def test_a_nyaa_torrent_downloads_straight_from_nyaa(self):
        release = {"page": "https://nyaa.si/view/1234567", "link": "http://prowlarr:9696/9/download?apikey=key"}
        self.assertEqual(self.fetched_url(release), "https://nyaa.si/download/1234567.torrent")

    def test_any_other_torrent_downloads_through_prowlarr(self):
        release = {"page": "https://c411.org/torrents/42", "link": "http://prowlarr:9696/3/download?apikey=key"}
        self.assertEqual(self.fetched_url(release), "http://prowlarr:9696/3/download?apikey=key")


class FakeServer(http.server.BaseHTTPRequestHandler):
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

    def do_GET(self):
        FakeServer.calls.append((self.path, "", b""))
        self.answer(json.dumps([{"name": "c411/Keroro.mkv", "size": 5}, {"name": "c411/x.nfo", "size": 1}]).encode())

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

    def test_a_tracker_copy_is_added_stopped_in_the_twin_folder(self):
        self.qbt.add(b"d8:announce", "ratio", "/data/torrents/ratio")
        _, _, body = FakeServer.calls[-1]
        self.assertIn(b'name="savepath"\r\n\r\n/data/torrents/ratio\r\n', body)
        self.assertIn(b'name="autoTMM"\r\n\r\nfalse\r\n', body)
        self.assertIn(b'name="stopped"\r\n\r\ntrue\r\n', body)

    def test_the_tracker_copy_takes_the_twin_file_names_then_checks_them(self):
        self.qbt.match_files("abc", [{"name": "Sgt Frog.mkv", "size": 5}, {"name": "c411/x.nfo", "size": 1}])
        calls = [(path, body) for path, _, body in FakeServer.calls if path != "/api/v2/auth/login"]
        self.assertEqual(calls, [
            ("/api/v2/torrents/files?hash=abc", b""),
            ("/api/v2/torrents/renameFile", b"hash=abc&oldPath=c411%2FKeroro.mkv&newPath=Sgt+Frog.mkv"),
            ("/api/v2/torrents/recheck", b"hashes=abc"),
        ])

    def test_an_expired_session_logs_in_again(self):
        self.qbt.delete(["a"])
        FakeServer.expire_once = True
        self.qbt.delete(["b"])
        self.assertEqual(
            [path for path, _, _ in FakeServer.calls],
            ["/api/v2/auth/login", "/api/v2/torrents/delete", "/api/v2/torrents/delete",
             "/api/v2/auth/login", "/api/v2/torrents/delete"],
        )


class FakeProwlarrServer(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        FakeProwlarrServer.calls.append(("GET", self.path, self.headers.get("X-Api-Key"), None))
        if self.path.startswith("/3/api"):
            self.answer(429, b"", {"Retry-After": "1200"})
        elif self.path == "/api/v1/appprofile":
            self.answer(200, json.dumps(FakeProwlarrServer.profiles).encode())
        elif self.path == "/api/v1/indexer/3":
            self.answer(200, json.dumps({"id": 3, "name": "C411", "appProfileId": FakeProwlarrServer.profile}).encode())

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        FakeProwlarrServer.calls.append(("POST", self.path, self.headers.get("X-Api-Key"), body))
        self.answer(201, json.dumps({**body, "id": 2}).encode())

    def do_PUT(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        FakeProwlarrServer.calls.append(("PUT", self.path, self.headers.get("X-Api-Key"), body))
        self.answer(202, json.dumps(body).encode())

    def answer(self, code, body, headers=None):
        self.send_response(code)
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class RealProwlarr(unittest.TestCase):
    def setUp(self):
        FakeProwlarrServer.calls = []
        FakeProwlarrServer.profiles = [{"id": 1, "name": "Standard"}]
        FakeProwlarrServer.profile = 1
        self.server = http.server.HTTPServer(("127.0.0.1", 0), FakeProwlarrServer)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.prowlarr = Prowlarr(f"http://127.0.0.1:{self.server.server_port}", "key", 2, spacing=0)

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()

    def writes(self):
        return [(method, path, body) for method, path, _, body in FakeProwlarrServer.calls if method != "GET"]

    def test_holding_c411_creates_the_paused_profile_and_moves_the_indexer_onto_it(self):
        self.prowlarr.hold("3", True, "Paused")
        self.assertEqual(self.writes(), [
            ("POST", "/api/v1/appprofile", {
                "name": "Paused", "enableRss": False, "enableAutomaticSearch": False,
                "enableInteractiveSearch": False, "minimumSeeders": 1,
            }),
            ("PUT", "/api/v1/indexer/3", {"id": 3, "name": "C411", "appProfileId": 2}),
        ])
        self.assertTrue(all(key == "key" for _, _, key, _ in FakeProwlarrServer.calls))

    def test_releasing_c411_puts_it_back_on_the_other_profile(self):
        FakeProwlarrServer.profiles.append({"id": 2, "name": "Paused"})
        FakeProwlarrServer.profile = 2
        self.prowlarr.hold("3", False, "Paused")
        self.assertEqual(self.writes(), [("PUT", "/api/v1/indexer/3", {"id": 3, "name": "C411", "appProfileId": 1})])

    def test_releasing_c411_restores_the_profile_it_had_before(self):
        FakeProwlarrServer.profiles += [{"id": 2, "name": "Paused"}, {"id": 5, "name": "Interactive"}]
        FakeProwlarrServer.profile = 2
        self.prowlarr.hold("3", False, "Paused", 5)
        self.assertEqual(self.writes(), [("PUT", "/api/v1/indexer/3", {"id": 3, "name": "C411", "appProfileId": 5})])

    def test_holding_c411_tells_the_profile_it_leaves(self):
        self.assertEqual(self.prowlarr.hold("3", True, "Paused"), 1)

    def test_an_indexer_already_on_the_right_profile_is_left_alone(self):
        self.prowlarr.hold("3", False, "Paused")
        self.assertEqual(self.writes(), [])

    def test_a_429_raises_with_the_wait_prowlarr_asks_for(self):
        with self.assertRaises(Throttled) as throttled:
            self.prowlarr.newest("3", NOW)
        self.assertEqual(throttled.exception.retry_after, 1200)


PROFILE = {
    "zen": {"balance": 25, "totalEarned": 25, "totalSpent": 0},
    "transfer": {
        "uploaded": {"actual": 5447372745, "credit": 53687091200, "total": 59134463945},
        "downloaded": {"actual": 28231492173, "credit": 0, "total": 28231492173},
    },
}


class FakeC411Server(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        FakeC411Server.calls.append((self.path, self.headers.get("Authorization")))
        body = json.dumps(PROFILE).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class RealC411(unittest.TestCase):
    def test_the_account_is_read_from_the_profile_with_the_bearer_key(self):
        FakeC411Server.calls = []
        server = http.server.HTTPServer(("127.0.0.1", 0), FakeC411Server)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            account = C411("secret", f"http://127.0.0.1:{server.server_port}/api/user/profile").account()
        finally:
            server.shutdown()
            server.server_close()
        self.assertEqual(account, {"uploaded": 59134463945, "downloaded": 28231492173, "zen": 25})
        self.assertEqual(FakeC411Server.calls, [("/api/user/profile", "Bearer secret")])


if __name__ == "__main__":
    unittest.main()
