import base64
import contextlib
import datetime
import hashlib
import http.server
import io
import json
import os
import re
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from unittest import mock

from ratio import (
    ANIME,
    C411,
    DAY,
    GB,
    QUEST_CATEGORIES,
    Prowlarr,
    QBittorrent,
    Throttled,
    bdecode,
    is_rare,
    load_state,
    parse_torznab,
    run_once,
    save_state,
    serve,
)

NOW = 1_800_000_000
TRACKER = "https://tracker.c411.org/announce"


def torrent(hash, category="farm", size_gb=3, seeding_days=10, uploaded_gb=0, downloaded_gb=0, progress=1,
            seeders=100, tracker=TRACKER, completion_on=0, save_path=None, state="stalledUP", tags=""):
    return {
        "tags": tags,
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
        self.tagged = []
        self.limited = []
        self.pinned = []
        self.cats = {name: {} for name in ("movies", "shows", "music", "books", "farm")}
        self.names = {}

    def torrents(self):
        return [dict(t) for t in self.list]

    def add(self, torrent, category, save_path=None):
        name = bdecode(torrent)[0][b"info"][b"name"].decode()
        self.added.append((name, category) if save_path is None else (name, category, save_path))

    def files(self, infohash):
        return [{"name": name, "size": size} for name, size in self.names.get(infohash, [(f"{infohash}.dat", 7)])]

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
        return dict(self.cats)

    def create_category(self, name, path):
        self.made[name] = path
        self.cats[name] = {"savePath": path}

    def remove_tags(self, hashes, tag):
        for t in self.list:
            if t["hash"] in hashes:
                t["tags"] = ", ".join(x for x in t["tags"].split(", ") if x != tag)

    def add_tags(self, hashes, tag):
        self.tagged += [(h, tag) for h in hashes]
        for t in self.list:
            if t["hash"] in hashes:
                t["tags"] = ", ".join(x for x in (t["tags"], tag) if x)

    def reset_share_limits(self, hashes):
        self.limited += hashes

    def keep_in_place(self, hashes):
        self.pinned += hashes

    def preferences(self):
        return dict(self.prefs)

    def set_preferences(self, preferences):
        self.prefs.update(preferences)


def torrent_file(name, lengths):
    files = b"".join(b"d6:lengthi%de4:pathl1:xee" % length for length in lengths)
    return b"d4:infod5:filesl" + files + b"e4:name%d:" % len(name) + name.encode() + b"ee"


class FakeProwlarr:
    def __init__(self, newest=(), twins=(), files=None, throttle=None, broken=(), hunt=None, refused=(), disabled=False):
        self.feed = list(newest)
        self.hunt = hunt or {}
        self.hunts = []
        self.twins = list(twins)
        self.files = files or {}
        self.throttle = throttle
        self.broken = set(broken)
        self.refused = set(refused)
        self.disabled = disabled
        self.log = []
        self.queries = []
        self.reads = 0
        self.fetched = []
        self.holds = []
        self.restores = []

    def newest(self, indexer, now):
        self.reads += 1
        self.log.append("newest")
        if self.throttle:
            raise Throttled(self.throttle, self.disabled)
        return list(self.feed)

    def category(self, indexer, cats, now):
        self.hunts.append(cats)
        return list(self.hunt.get(cats, []))

    def search(self, indexer, title, now):
        self.queries.append(title)
        return list(self.twins)

    def torrent(self, release):
        infohash = release.get("infohash") or release["link"].rsplit("/", 1)[1].removesuffix(".torrent")
        self.fetched.append(infohash)
        self.log.append(f"torrent {infohash}")
        if infohash in self.refused:
            raise urllib.error.HTTPError(release["link"], 403, "Forbidden", {}, None)
        if infohash in self.broken:
            raise RuntimeError("the tracker refused the torrent file")
        return torrent_file(infohash, self.files.get(infohash, [7]))

    def hold(self, indexer, paused, profile, restore=None):
        self.holds.append((indexer, paused, profile))
        self.restores.append(restore)
        return 7 if paused else None


class FakeC411:
    def __init__(self, uploaded_gb=60, downloaded_gb=30, zen=25, broken=False, earned=None, actual_gb=5):
        self.uploaded = uploaded_gb * GB
        self.downloaded = downloaded_gb * GB
        self.zen = zen
        self.earned = zen if earned is None else earned
        self.actual = actual_gb * GB
        self.broken = broken
        self.reads = 0

    def account(self):
        self.reads += 1
        if self.broken:
            raise RuntimeError("Site is currently restricted")
        return {"uploaded": self.uploaded, "downloaded": self.downloaded, "zen": self.zen, "earned": self.earned,
                "actual": self.actual}


def under_floor():
    return FakeC411(uploaded_gb=30, downloaded_gb=30)


def small(infohash, size_gb=0.5, seeders=3, free=False):
    return release(infohash, leechers=0, seeders=seeders, size_gb=size_gb, free=free)


def hunting(*releases, name="Films & Vidéos", **changes):
    return FakeProwlarr(hunt={QUEST_CATEGORIES[name]: list(releases)}, **changes)


def filled(name, count, prefix=None, **changes):
    prefix = prefix or name[:2].lower()
    hashes = [f"{prefix}{i}" for i in range(count)]
    return [torrent(h, size_gb=0.1, **changes) for h in hashes], {h: name for h in hashes}


def all_filled(count=25, **changes):
    held, kinds = [], {}
    for name in QUEST_CATEGORIES:
        more, names = filled(name, count, **changes)
        held += more
        kinds.update(names)
    return held, kinds


SETTINGS = {
    "tracker": "c411",
    "indexers": ["3"],
    "twin_indexers": ["9"],
    "cross_seed_key": "",
    "farm_size": 100 * GB,
    "floor": 1.2,
    "paused_profile": "Paused",
    "volume": {"max_size": GB, "per_category": 25, "daily": 2 * GB},
    "free_max_size": 5 * GB,
    "min_seeding": 3 * DAY,
    "public_seeding": 14 * DAY,
    "max_downloading": 2,
    "twin_lookups": 10,
    "library_categories": ("movies", "shows", "music", "books"),
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
        "transfers": {}, "hunted": {}, "twins": {}, "free": {}, "checked": {}, "paused": False,
        "sources": {}, "history": {}, "zen": {}, "report": {}, "account": [], "held": None, "backoff_until": 0,
        "stuck": {}, "restore": {}, "kinds": {}, "hunt_at": {}, "daily": [], "given_up": {}, **changes,
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
        self.assertEqual(qbt.added, [("public", "farm")])
        self.assertEqual(state["twins"], {"public": ["episode", "farm", NOW, "http://prowlarr/episode.torrent"]})
        self.assertEqual(state["hunted"], {})

    def test_a_twin_is_grabbed_even_under_the_floor(self):
        qbt = FakeQBittorrent()
        run(qbt, FakeProwlarr([episode()], twins=[twin()]), c411=under_floor())
        self.assertEqual(qbt.added, [("public", "farm")])

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
            torrent("hungry", save_path="/data/torrents/cross-seed/C411"),
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
        self.assertEqual(qbt.added, [("hungry", "farm", "/data/torrents/farm")])
        self.assertEqual(qbt.matched, [("hungry", [{"name": "public.dat", "size": 7}])])

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

    def test_a_tracker_copy_is_given_up_after_its_third_failure(self):
        qbt = FakeQBittorrent([torrent("public", tracker="http://nyaa.tracker.wf/announce")])
        state = empty_state(twins={"public": ["hungry", "ratio", NOW - 3600, "http://prowlarr/hungry.torrent"]})
        prowlarr = FakeProwlarr(broken={"hungry"})
        for hours in range(6):
            run(qbt, prowlarr, state, now=NOW + 3600 * hours)
        self.assertEqual(prowlarr.fetched, ["hungry"] * 3)
        self.assertEqual((state["twins"], state["stuck"]), ({}, {}))
        self.assertEqual(qbt.deleted, [])

    def test_a_tracker_copy_given_up_is_not_grabbed_again_for_a_week(self):
        qbt = FakeQBittorrent([torrent("public", tracker="http://nyaa.tracker.wf/announce")])
        state = empty_state(twins={"public": ["hungry", "ratio", NOW - 3600, "http://prowlarr/hungry.torrent"]})
        run(qbt, FakeProwlarr(refused={"hungry"}), state)
        prowlarr = FakeProwlarr([release("hungry", free=True)])
        run(qbt, prowlarr, state, now=NOW + DAY)
        self.assertEqual(qbt.added, [])
        run(qbt, prowlarr, state, now=NOW + 8 * DAY)
        self.assertEqual(qbt.added, [("hungry", "farm")])

    def test_a_qbittorrent_refusal_is_not_a_tracker_refusal(self):
        qbt = FakeQBittorrent([torrent("public", tracker="http://nyaa.tracker.wf/announce")])

        def conflict(infohash, wanted):
            raise urllib.error.HTTPError("http://gluetun:8080", 409, "Conflict", {}, None)

        qbt.match_files = conflict
        state = empty_state(twins={"public": ["hungry", "ratio", NOW - 3600, "http://prowlarr/hungry.torrent"]})
        run(qbt, FakeProwlarr(), state)
        self.assertIn("public", state["twins"])
        self.assertEqual(state["stuck"]["public"][0], 1)

    def test_a_tracker_copy_given_up_after_its_add_leaves_no_stopped_copy(self):
        qbt = FakeQBittorrent([torrent("public", tracker="http://nyaa.tracker.wf/announce")])

        def mismatch(infohash, wanted):
            raise RuntimeError("the files differ")

        qbt.match_files = mismatch
        state = empty_state(twins={"public": ["hungry", "ratio", NOW - 3600, "http://prowlarr/hungry.torrent"]})
        for hours in range(6):
            run(qbt, FakeProwlarr(), state, now=NOW + 3600 * hours)
        self.assertEqual(state["twins"], {})
        self.assertEqual(qbt.deleted, ["hungry"])
        self.assertEqual(qbt.kept, ["hungry"])

    def test_a_tracker_copy_the_tracker_refuses_is_given_up_at_once(self):
        qbt = FakeQBittorrent([torrent("public", tracker="http://nyaa.tracker.wf/announce")])
        state = empty_state(twins={"public": ["hungry", "ratio", NOW - 3600, "http://prowlarr/hungry.torrent"]})
        prowlarr = FakeProwlarr(refused={"hungry"})
        run(qbt, prowlarr, state)
        run(qbt, prowlarr, state, now=NOW + DAY)
        self.assertEqual(prowlarr.fetched, ["hungry"])
        self.assertEqual(state["twins"], {})

    def test_the_tracker_copies_wait_for_the_search_of_the_run(self):
        qbt = FakeQBittorrent([torrent("public", tracker="http://nyaa.tracker.wf/announce")])
        state = empty_state(twins={"public": ["hungry", "ratio", NOW - 3600, "http://prowlarr/hungry.torrent"]})
        prowlarr = FakeProwlarr()
        run(qbt, prowlarr, state)
        self.assertEqual(prowlarr.log[:2], ["newest", "torrent hungry"])

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
        self.assertEqual(state["hunted"], {})

    def test_a_freeleech_release_is_grabbed_even_under_the_floor(self):
        qbt = FakeQBittorrent()
        state = run(qbt, FakeProwlarr([release("free", seeders=30, free=True)]), c411=under_floor())
        self.assertEqual(qbt.added, [("free", "farm")])
        self.assertEqual(state["free"], {"free": NOW})
        self.assertEqual(state["hunted"], {})

    def test_a_freeleech_download_does_not_count_in_the_deficit(self):
        qbt = FakeQBittorrent([torrent("free", downloaded_gb=4, uploaded_gb=1)])
        state = run(qbt, state=empty_state(free={"free": NOW - 100}))
        self.assertEqual(state["transfers"], {"free": [0, 1 * GB]})

    def test_two_downloads_at_once_at_most(self):
        qbt = FakeQBittorrent([torrent("a", progress=0.5), torrent("b", progress=0.1)])
        run(qbt, FakeProwlarr([release("free", free=True), episode()], twins=[twin()]))
        self.assertEqual(qbt.added, [])


class Floor(unittest.TestCase):
    def test_the_profile_is_read_once_per_run(self):
        c411 = FakeC411()
        run(FakeQBittorrent(), FakeProwlarr([episode(), release("rare", seeders=3)], twins=[twin()]), c411=c411)
        self.assertEqual(c411.reads, 1)

    def test_under_the_floor_nothing_paid_is_grabbed_and_c411_is_held_in_prowlarr(self):
        qbt = FakeQBittorrent()
        prowlarr = hunting(small("rare"))
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
        prowlarr = hunting(small("rare"))
        state = run(qbt, prowlarr, c411=FakeC411(broken=True))
        self.assertEqual(qbt.added, [])
        self.assertEqual(prowlarr.holds, [("3", True, "Paused")])
        self.assertTrue(state["paused"])

    def test_a_paid_grab_never_spends_past_the_floor(self):
        qbt = FakeQBittorrent()
        prowlarr = hunting(small("b", size_gb=0.4), small("a", size_gb=0.3))
        run(qbt, prowlarr, c411=FakeC411(uploaded_gb=42, downloaded_gb=34.5))
        self.assertEqual(qbt.added, [("a", "farm")])

    def test_a_tracker_download_in_progress_counts_against_the_floor(self):
        qbt = FakeQBittorrent([torrent("movie", category="movies", size_gb=4, progress=0.5)])
        run(qbt, hunting(small("rare")), c411=FakeC411(uploaded_gb=42, downloaded_gb=33))
        self.assertEqual(qbt.added, [])


class Throttling(unittest.TestCase):
    def test_a_fall_under_the_floor_holds_c411_even_while_prowlarr_asks_to_wait(self):
        prowlarr = FakeProwlarr()
        run(FakeQBittorrent(), prowlarr, empty_state(held=False, backoff_until=NOW + 1800), c411=under_floor())
        self.assertEqual(prowlarr.holds, [("3", True, "Paused")])
        self.assertEqual(prowlarr.reads, 0)

    def test_a_429_from_prowlarr_stops_its_use_until_the_wait_is_over(self):
        prowlarr = FakeProwlarr(throttle=1800)
        state = run(FakeQBittorrent(), prowlarr)
        self.assertEqual(state["backoff_until"], NOW + 1800)
        prowlarr.throttle = None
        run(FakeQBittorrent(), prowlarr, state, now=NOW + 600)
        self.assertEqual(prowlarr.reads, 1)
        run(FakeQBittorrent(), prowlarr, state, now=NOW + 1800)
        self.assertEqual(prowlarr.reads, 2)


PUBLIC = "http://nyaa.tracker.wf/announce"


class Budget(unittest.TestCase):
    def test_nothing_leaves_the_farm_while_it_has_room(self):
        qbt = FakeQBittorrent([torrent("dead", size_gb=40, seeding_days=400, seeders=150), torrent("rare", size_gb=50, seeders=2)])
        run(qbt)
        self.assertEqual(qbt.deleted, [])

    def test_a_full_farm_lets_go_first_of_the_least_zen_per_gb(self):
        qbt = FakeQBittorrent([
            torrent("big", size_gb=60, seeders=100),
            torrent("small", size_gb=0.5, seeders=100),
            torrent("medium", size_gb=45, seeders=100),
        ])
        run(qbt)
        self.assertEqual(qbt.deleted, ["big"])

    def test_a_rare_torrent_pays_more_zen_than_a_crowded_one_of_the_same_size(self):
        qbt = FakeQBittorrent([torrent("rare", size_gb=60, seeders=10), torrent("crowded", size_gb=60, seeders=100)])
        run(qbt)
        self.assertEqual(qbt.deleted, ["crowded"])

    def test_a_torrent_seeded_longer_pays_more_zen(self):
        qbt = FakeQBittorrent([torrent("old", size_gb=60, seeding_days=200), torrent("young", size_gb=60, seeding_days=10)])
        run(qbt)
        self.assertEqual(qbt.deleted, ["young"])

    def test_no_torrent_leaves_before_its_seeding_goal(self):
        qbt = FakeQBittorrent([torrent("new", size_gb=80, seeding_days=1), torrent("old", size_gb=30)])
        run(qbt)
        self.assertEqual(qbt.deleted, ["old"])

    def test_a_download_in_progress_counts_but_stays(self):
        qbt = FakeQBittorrent([torrent("downloading", size_gb=90, seeding_days=0, progress=0.3), torrent("seeded", size_gb=30)])
        run(qbt)
        self.assertEqual(qbt.deleted, ["seeded"])

    def test_a_public_torrent_of_the_farm_is_not_weighed_for_zen(self):
        qbt = FakeQBittorrent([torrent("public", size_gb=60, tracker=PUBLIC), torrent("c411", size_gb=50)])
        run(qbt)
        self.assertEqual(qbt.deleted, ["c411"])


class Categories(unittest.TestCase):
    def test_a_category_ratio_does_not_own_is_left_alone(self):
        qbt = FakeQBittorrent([torrent("manual", category="mine")])
        qbt.cats["mine"] = {}
        run(qbt)
        self.assertEqual(qbt.moved, [])
        self.assertIn("mine", qbt.cats)


class Tags(unittest.TestCase):
    def test_every_public_torrent_carries_the_public_tag(self):
        qbt = FakeQBittorrent([
            torrent("public", category="movies", tracker=PUBLIC),
            torrent("tagged", category="movies", tracker=PUBLIC, tags="public"),
        ])
        run(qbt)
        self.assertEqual(qbt.tagged, [("public", "public")])

    def test_every_c411_torrent_carries_a_tag_of_its_source(self):
        qbt = FakeQBittorrent([torrent("movie", category="movies", size_gb=200), torrent("free")])
        run(qbt, state=empty_state(free={"free": NOW - 100}))
        self.assertEqual(sorted(qbt.tagged), [("free", "freeleech"), ("movie", "library")])

    def test_a_c411_torrent_whose_tracker_is_down_is_not_public(self):
        qbt = FakeQBittorrent([torrent("c411", category="movies", tracker="", size_gb=200, seeding_days=20)])
        run(qbt, state=empty_state(transfers={"c411": [GB, 0]}, sources={"c411": "library"}))
        self.assertEqual(qbt.deleted, [])
        self.assertNotIn(("c411", "public"), qbt.tagged)


class PublicTorrents(unittest.TestCase):
    def test_a_public_torrent_leaves_after_two_weeks_of_seeding(self):
        qbt = FakeQBittorrent([
            torrent("old", category="movies", tracker=PUBLIC, seeding_days=14),
            torrent("young", category="shows", tracker=PUBLIC, seeding_days=13, uploaded_gb=50),
        ])
        run(qbt)
        self.assertEqual(qbt.deleted, ["old"])

    def test_a_public_torrent_leaves_once_its_c411_copy_seeds_the_same_files(self):
        qbt = FakeQBittorrent([
            torrent("public", category="movies", tracker=PUBLIC, seeding_days=1),
            torrent("copy", save_path="/data/torrents/cross-seed/C411", seeding_days=0.5),
        ])
        run(qbt)
        self.assertEqual(qbt.deleted, ["public"])
        self.assertEqual(qbt.kept, [])

    def test_a_public_torrent_waits_for_its_c411_copy_to_finish(self):
        qbt = FakeQBittorrent([
            torrent("public", category="movies", tracker=PUBLIC, seeding_days=1),
            torrent("copy", progress=0.5, seeding_days=0),
            torrent("other", size_gb=4),
        ])
        run(qbt)
        self.assertEqual(qbt.deleted, [])

    def test_a_public_torrent_leaves_its_files_to_the_c411_copy_checking_them(self):
        qbt = FakeQBittorrent([
            torrent("public", tracker=PUBLIC, seeding_days=14),
            torrent("copy", progress=0.4, seeding_days=0, state="checkingUP"),
        ])
        run(qbt, state=empty_state(transfers={"copy": [0, 0]}, sources={"copy": "twin"}))
        self.assertEqual(qbt.deleted, ["public"])
        self.assertEqual(qbt.kept, ["public"])

    def test_a_torrent_of_another_private_tracker_is_not_public(self):
        other = torrent("other", category="movies", tracker="https://other.example/announce", seeding_days=20)
        other["private"] = True
        qbt = FakeQBittorrent([other, torrent("copy", save_path="/data/torrents/cross-seed/C411")])
        run(qbt)
        self.assertEqual(qbt.deleted, [])
        self.assertNotIn(("other", "public"), qbt.tagged)

    def test_a_public_torrent_of_a_category_ratio_does_not_own_stays(self):
        qbt = FakeQBittorrent([torrent("mine", category="", tracker=PUBLIC, seeding_days=30)])
        run(qbt)
        self.assertEqual(qbt.deleted, [])
        self.assertEqual(qbt.tagged, [])


class VolumeHunt(unittest.TestCase):
    def test_the_hunt_grabs_rare_c411_torrents_of_at_most_1_gb_with_a_seeder_cheapest_first(self):
        qbt = FakeQBittorrent()
        prowlarr = hunting(
            small("big", size_gb=1.5, seeders=2),
            small("crowded", size_gb=0.5, seeders=30),
            small("dead", size_gb=0.1, seeders=0),
            small("dear", size_gb=0.8),
            small("cheap", size_gb=0.2),
        )
        state = run(qbt, prowlarr)
        self.assertEqual(qbt.added, [("cheap", "farm"), ("dear", "farm")])
        self.assertEqual(state["hunted"], {"cheap": [NOW, int(0.2 * GB)], "dear": [NOW, int(0.8 * GB)]})
        self.assertEqual(state["sources"], {"cheap": "volume-hunt", "dear": "volume-hunt"})
        self.assertEqual(state["kinds"], {"cheap": "Films & Vidéos", "dear": "Films & Vidéos"})

    def test_a_freeleech_release_costs_nothing_and_comes_first(self):
        qbt = FakeQBittorrent()
        state = run(qbt, hunting(small("paid", size_gb=0.1), small("free", size_gb=0.9, free=True)))
        self.assertEqual(qbt.added, [("free", "farm"), ("paid", "farm")])
        self.assertEqual(state["free"], {"free": NOW})
        self.assertEqual(state["sources"]["free"], "freeleech")
        self.assertEqual(list(state["hunted"]), ["paid"])

    def test_under_the_floor_only_a_freeleech_release_is_hunted(self):
        qbt = FakeQBittorrent()
        run(qbt, hunting(small("paid", size_gb=0.1), small("free", size_gb=0.9, free=True)), c411=under_floor())
        self.assertEqual(qbt.added, [("free", "farm")])

    def test_a_freeleech_release_is_hunted_even_when_downloads_in_progress_use_up_the_margin(self):
        qbt = FakeQBittorrent([torrent("movie", category="movies", size_gb=4, progress=0.5)])
        run(qbt, hunting(small("free", free=True)), c411=FakeC411(uploaded_gb=42, downloaded_gb=34))
        self.assertEqual(qbt.added, [("free", "farm")])

    def test_each_run_reads_one_category_and_turns_through_those_short_of_25(self):
        held, kinds = filled("Films & Vidéos", 25)
        qbt = FakeQBittorrent(held)
        prowlarr = FakeProwlarr()
        state = empty_state(kinds=kinds)
        for minutes in (0, 10, 20, 30, 40):
            run(qbt, prowlarr, state, now=NOW + 60 * minutes)
        self.assertEqual(prowlarr.hunts, [
            QUEST_CATEGORIES["Ebook"], QUEST_CATEGORIES["Audio"], QUEST_CATEGORIES["Emulation"],
            QUEST_CATEGORIES["Imprimante 3D"], QUEST_CATEGORIES["Ebook"],
        ])

    def test_a_category_stops_at_25_while_another_is_short(self):
        held, kinds = filled("Ebook", 24)
        prowlarr = hunting(small("a", size_gb=0.1), small("b", size_gb=0.2), name="Ebook")
        qbt = FakeQBittorrent(held)
        run(qbt, prowlarr, empty_state(kinds=kinds, hunt_at={"Films & Vidéos": NOW - 60}))
        self.assertEqual(prowlarr.hunts, [QUEST_CATEGORIES["Ebook"]])
        self.assertEqual(qbt.added, [("a", "farm")])

    def test_once_every_category_holds_25_the_hunt_concentrates_on_anime(self):
        held, kinds = all_filled()
        prowlarr = FakeProwlarr(hunt={ANIME: [small("a", size_gb=0.1), small("b", size_gb=0.2)]})
        qbt = FakeQBittorrent(held)
        state = run(qbt, prowlarr, empty_state(kinds=kinds))
        self.assertEqual(prowlarr.hunts, [ANIME])
        self.assertEqual(qbt.added, [("a", "farm"), ("b", "farm")])
        self.assertEqual(state["kinds"]["a"], "Films & Vidéos")

    def test_paid_download_is_paced_per_day(self):
        qbt = FakeQBittorrent()
        state = empty_state(hunted={"old": [NOW - DAY - 60, GB], "recent": [NOW - 3600, int(1.7 * GB)]})
        run(qbt, hunting(small("dear", size_gb=0.4), small("cheap", size_gb=0.2)), state)
        self.assertEqual(qbt.added, [("cheap", "farm")])
        self.assertEqual(set(state["hunted"]), {"recent", "cheap"})

    def test_a_paid_anime_grab_stays_rare_and_at_most_1_gb(self):
        held, kinds = all_filled()
        prowlarr = FakeProwlarr(hunt={ANIME: [small("big", size_gb=1.4), small("crowded", seeders=40)]})
        qbt = FakeQBittorrent(held)
        run(qbt, prowlarr, empty_state(kinds=kinds))
        self.assertEqual(qbt.added, [])

    def test_an_anime_twin_is_grabbed_whatever_its_size(self):
        qbt = FakeQBittorrent()
        big = release("episode", title=EPISODE, leechers=3, seeders=60, size_gb=1.4)
        state = run(qbt, FakeProwlarr([big], twins=[twin(size_gb=1.42)]))
        self.assertEqual(qbt.added, [("public", "farm")])
        self.assertEqual(state["kinds"], {"episode": "Films & Vidéos"})

    def test_a_library_torrent_counts_in_the_category_of_what_it_holds(self):
        qbt = FakeQBittorrent([
            torrent("movie", category="movies", size_gb=200),
            torrent("album", category="music", size_gb=200),
            torrent("book", category="books", size_gb=200),
            torrent("public", category="movies", tracker=PUBLIC),
        ])
        state = run(qbt)
        self.assertEqual(state["kinds"], {"movie": "Films & Vidéos", "album": "Audio", "book": "Ebook"})

    def test_a_torrent_gone_from_qbittorrent_loses_its_category(self):
        state = run(FakeQBittorrent(), state=empty_state(kinds={"gone": "Ebook"}))
        self.assertEqual(state["kinds"], {})

    def test_rare_depends_on_the_size(self):
        self.assertTrue(is_rare(GB // 2, 24))
        self.assertFalse(is_rare(GB // 2, 25))
        self.assertTrue(is_rare(50 * GB, 50))
        self.assertFalse(is_rare(50 * GB, 51))
        self.assertTrue(is_rare(200 * GB, 199))


class Adoption(unittest.TestCase):
    def test_an_imported_c411_torrent_of_the_library_moves_into_the_farm(self):
        qbt = FakeQBittorrent([torrent("movie", category="movies", seeders=80), torrent("album", category="music", seeders=3)])
        run(qbt)
        self.assertEqual(qbt.moved, [("movie", "farm"), ("album", "farm")])
        self.assertEqual(qbt.limited, ["movie", "album"])

    def test_a_public_fresh_or_too_big_library_torrent_stays(self):
        qbt = FakeQBittorrent([
            torrent("public", category="shows", seeders=3, tracker=PUBLIC),
            torrent("fresh", category="shows", seeders=3, seeding_days=0.1),
            torrent("huge", category="movies", size_gb=90),
            torrent("farmed", size_gb=20),
        ])
        run(qbt)
        self.assertEqual(qbt.moved, [])


class Ledger(unittest.TestCase):
    def test_only_tracker_torrents_count_and_removed_ones_stay(self):
        qbt = FakeQBittorrent([
            torrent("c411", category="movies", downloaded_gb=6, uploaded_gb=1),
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

    def test_a_volume_hunt_grab_is_reported_as_a_volume_hunt(self):
        qbt = FakeQBittorrent([torrent("old", category="movies", downloaded_gb=10, uploaded_gb=35)])
        state = run(qbt, hunting(small("rare")))
        self.assertEqual(state["sources"]["rare"], "volume-hunt")
        self.assertEqual(state["sources"]["old"], "library")

    def test_the_tracker_copy_of_a_twin_is_a_twin_whoever_adds_it(self):
        state = run(FakeQBittorrent(), FakeProwlarr([episode()], twins=[twin()]))
        self.assertEqual(state["sources"], {"episode": "twin"})
        qbt = FakeQBittorrent([
            torrent("public", tracker="http://nyaa.tracker.wf/announce"),
            torrent("episode", state="stoppedUP", tracker=""),
        ])
        run(qbt, state=state, now=NOW + 600)
        self.assertEqual(state["twins"], {})
        run(FakeQBittorrent([torrent("episode")]), state=state, now=NOW + 1200)
        self.assertEqual(state["sources"], {"episode": "twin"})

    def test_library_torrents_and_their_cross_seeds_are_told_apart(self):
        qbt = FakeQBittorrent([
            torrent("movie", category="movies", seeders=100),
            torrent("show", category="shows", seeders=100),
            torrent("linked"),
            torrent("public", category="movies", tracker="http://nyaa.tracker.wf/announce"),
        ])
        state = run(qbt)
        self.assertEqual(state["sources"], {"movie": "library", "show": "library", "linked": "cross-seed"})

    def test_a_tracker_torrent_without_a_source_gets_one_from_what_ratio_sees(self):
        qbt = FakeQBittorrent([
            torrent("linked", downloaded_gb=4),
            torrent("free", downloaded_gb=4),
            torrent("movie", category="movies", seeders=100),
        ])
        transfers = {h: [GB, 0] for h in ("linked", "free", "movie", "gone")}
        state = run(qbt, state=empty_state(transfers=transfers, free={"free": NOW - 100}))
        self.assertEqual(state["sources"], {
            "linked": "cross-seed", "free": "freeleech", "movie": "library", "gone": "unknown",
        })
        self.assertEqual(report_of(state, "unknown")["downloaded_gb"], 1)

    def test_a_retired_zen_hunt_becomes_a_volume_hunt(self):
        qbt = FakeQBittorrent([torrent("kept", tags="zen-hunt")])
        state = run(qbt, state=empty_state(transfers={"kept": [GB, 0]}, sources={"kept": "zen-hunt"}))
        self.assertEqual(state["sources"], {"kept": "volume-hunt"})
        self.assertEqual(qbt.list[0]["tags"], "volume-hunt")

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
        qbt = FakeQBittorrent([torrent("movie", category="movies", seeders=100, downloaded_gb=4, uploaded_gb=2)])
        state = empty_state()
        lines = self.run_quietly(qbt, state, NOW)
        self.assertEqual(len(lines), 1)
        self.assertIn("Source library:", lines[0])
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
        qbt = FakeQBittorrent([torrent("movie", category="movies", seeders=100, downloaded_gb=6, uploaded_gb=3)])
        self.run_quietly(qbt, state, NOW)
        figures = report_of(state, "library")
        self.assertEqual(figures["downloaded_gb"], 7)
        self.assertEqual(figures["week_downloaded_gb"], 4)
        self.assertEqual(figures["week_uploaded_gb"], 2.5)
        self.assertEqual(state["history"]["movie"][0], [NOW - 6.5 * DAY, 2 * GB, GB // 2])
        self.assertNotIn("gone", state["history"])

    def test_a_new_torrent_counts_whole_in_the_week(self):
        state = empty_state()
        qbt = FakeQBittorrent([torrent("movie", category="movies", seeders=100, seeding_days=2, downloaded_gb=6)])
        self.run_quietly(qbt, state, NOW)
        self.assertEqual(report_of(state, "library")["week_downloaded_gb"], 6)

    def test_a_torrent_removed_this_week_still_counts(self):
        state = empty_state()
        qbt = FakeQBittorrent([torrent("movie", category="movies", seeders=100, seeding_days=2, downloaded_gb=6)])
        self.run_quietly(qbt, state, NOW)
        self.run_quietly(FakeQBittorrent(), state, NOW + DAY)
        self.assertEqual(report_of(state, "library")["week_downloaded_gb"], 6)
        self.run_quietly(FakeQBittorrent(), state, NOW + 8 * DAY)
        self.assertEqual(report_of(state, "library")["week_downloaded_gb"], 0)

    def test_days_in_the_farm_count_from_the_move(self):
        state = empty_state(sources={"kept": "library"})
        self.run_quietly(FakeQBittorrent([torrent("kept", category="movies", size_gb=200, seeding_days=10)]), state, NOW)
        self.run_quietly(FakeQBittorrent([torrent("kept", seeding_days=13)]), state, NOW + 3 * DAY)
        self.run_quietly(FakeQBittorrent([torrent("kept", seeding_days=16)]), state, NOW + 6 * DAY)
        self.assertEqual(report_of(state, "library")["farm_days"], 3)
        self.run_quietly(FakeQBittorrent(), state, NOW + 7 * DAY)
        self.assertEqual(report_of(state, "library")["farm_days"], 3)

    def test_a_torrent_added_again_starts_over_without_losing_what_it_did(self):
        state = empty_state()
        self.run_quietly(FakeQBittorrent([torrent("kept", seeders=3, seeding_days=5, downloaded_gb=6)]), state, NOW)
        self.run_quietly(FakeQBittorrent([torrent("kept", seeders=3, seeding_days=6, downloaded_gb=6)]), state, NOW + DAY)
        again = torrent("kept", seeders=3, seeding_days=1, downloaded_gb=2)
        again["added_on"] = NOW + DAY
        self.run_quietly(FakeQBittorrent([again]), state, NOW + 2 * DAY)
        figures = report_of(state, "cross-seed")
        self.assertEqual(figures["farm_days"], 2)
        self.assertEqual(figures["week_downloaded_gb"], 2)


def summary_of(qbt, state=None, now=NOW, c411=None, **changes):
    state = state if state is not None else empty_state()
    with contextlib.redirect_stdout(io.StringIO()):
        return run_once(qbt, FakeProwlarr(), c411 or FakeC411(), settings(**changes), state, now)


class Summary(unittest.TestCase):
    def test_the_ledger_counts_every_tracker_torrent_removed_ones_included(self):
        qbt = FakeQBittorrent([
            torrent("farmed", downloaded_gb=3, uploaded_gb=6),
            torrent("kept", size_gb=2, seeders=3, downloaded_gb=2, uploaded_gb=1),
            torrent("movie", category="movies", size_gb=4, seeders=100, downloaded_gb=4),
            torrent("public", tracker="http://nyaa.tracker.wf/announce", downloaded_gb=9),
        ])
        summary = summary_of(qbt, empty_state(transfers={"gone": [GB, 0]}))
        self.assertEqual(summary["ledger"], {
            "torrents": 4, "downloaded_gb": 10, "uploaded_gb": 7, "ratio": 0.7, "surplus_gb": -3,
            "week_downloaded_gb": 0, "week_uploaded_gb": 0, "week_ratio": None,
        })

    def test_the_farm_shows_the_seed_count_its_fill_and_its_downloads(self):
        qbt = FakeQBittorrent([
            torrent("farmed", size_gb=3),
            torrent("linked", size_gb=2, progress=0.5, seeding_days=0),
            torrent("kept", size_gb=4, seeders=3),
            torrent("movie", category="movies", size_gb=200),
            torrent("stopped", size_gb=1, state="stoppedUP"),
            torrent("public", size_gb=1, tracker=PUBLIC),
        ])
        qbt.start = lambda hashes: None
        summary = summary_of(qbt, indexers=[])
        self.assertEqual(summary["farm"], {"seeds": 3, "size_gb": 11, "budget_gb": 100, "fill": 11.0, "downloading": 1})
        self.assertNotIn("zen", summary)

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
        qbt = FakeQBittorrent([torrent("movie", category="movies", seeders=100, downloaded_gb=4, uploaded_gb=2)])
        summary = summary_of(qbt)
        self.assertEqual(summary["sources"], [
            {"name": "library", "figures": "1 torrent, 2.0 / 4.0 GB, ratio 0.50"},
        ])

    def test_the_next_quest_is_the_closest_step_not_reached(self):
        held = [torrent(f"t{i}", size_gb=0.1, seeding_days=1, seeders=100) for i in range(112)]
        quests = summary_of(FakeQBittorrent(held), indexers=[])["quests"]
        self.assertEqual(quests, {"name": "Seed count 150", "progress": 112, "target": 150, "shown": "112 of 150 seeds"})

    def test_the_seeds_of_a_category_count_for_the_category_quests(self):
        held, kinds = filled("Ebook", 40, seeding_days=1, seeders=100)
        quests = summary_of(FakeQBittorrent(held), empty_state(kinds=kinds), indexers=[])["quests"]
        self.assertEqual(quests, {"name": "50 seeds in one category", "progress": 40, "target": 50, "shown": "40 of 50 seeds"})

    def test_the_seeds_of_several_categories_count_25_each(self):
        held, kinds = all_filled(seeding_days=1, seeders=100)
        more, names = filled("Ebook", 20, prefix="x", seeding_days=1, seeders=100)
        kinds.update(names)
        quests = summary_of(FakeQBittorrent(held + more), empty_state(kinds=kinds), indexers=[])["quests"]
        self.assertEqual(quests["name"], "Seed count 150")
        self.assertEqual(quests["progress"], 145)

    def test_long_seeding_and_a_good_note_count_for_their_quests(self):
        rare = [torrent(f"r{i}", size_gb=0.1, seeding_days=1, seeders=3) for i in range(45)]
        old = [torrent(f"o{i}", size_gb=0.1, seeding_days=8, seeders=100) for i in range(4)]
        quests = summary_of(FakeQBittorrent(rare + old), indexers=[])["quests"]
        self.assertEqual(quests, {
            "name": "50 torrents at 62.5 % of their best note", "progress": 49, "target": 50, "shown": "49 of 50 torrents",
        })

    def test_every_quest_reached_shows_none_left(self):
        held = [torrent(f"t{i}", size_gb=0.01, seeding_days=400, seeders=3) for i in range(600)]
        names = ["Films & Vidéos"] * 500 + [n for n in QUEST_CATEGORIES if n != "Films & Vidéos" for _ in range(25)]
        kinds = {f"t{i}": name for i, name in enumerate(names)}
        quests = summary_of(FakeQBittorrent(held), empty_state(kinds=kinds), indexers=[], farm_size=1000 * GB)["quests"]
        self.assertEqual(quests, {"name": "every Quest reached", "progress": None, "target": None, "shown": "done"})

    def test_the_time_of_the_summary_is_an_iso_date(self):
        self.assertEqual(summary_of(FakeQBittorrent())["updated_at"], "2027-01-15T08:00:00+00:00")


def day(now):
    return datetime.date.fromtimestamp(now).isoformat()


class History(unittest.TestCase):
    def test_each_day_keeps_one_entry_of_the_account_and_the_seed_count(self):
        qbt = FakeQBittorrent([torrent("movie", category="movies", size_gb=2, seeders=100)])
        state = empty_state()
        run(qbt, state=state, c411=FakeC411(zen=20))
        run(qbt, state=state, now=NOW + 600, c411=FakeC411(zen=21))
        self.assertEqual(len(state["daily"]), 1)
        run(qbt, state=state, now=NOW + DAY, c411=FakeC411(zen=22))
        self.assertEqual([e["day"] for e in state["daily"]], [day(NOW), day(NOW + DAY)])
        self.assertEqual(state["daily"][0]["zen"], 21)
        entry = state["daily"][1]
        self.assertEqual(
            {k: entry[k] for k in ("ratio", "floor", "left_gb", "uploaded_gb", "actual_gb", "downloaded_gb", "zen", "seeds")},
            {"ratio": 2.0, "floor": 1.2, "left_gb": 20.0, "uploaded_gb": 60, "actual_gb": 5, "downloaded_gb": 30,
             "zen": 22, "seeds": 1},
        )

    def test_the_zen_paid_each_night_is_what_the_zen_earned_grew_by(self):
        state = empty_state()
        run(FakeQBittorrent(), state=state, c411=FakeC411(zen=20, earned=30))
        run(FakeQBittorrent(), state=state, now=NOW + DAY, c411=FakeC411(zen=12, earned=34))
        self.assertEqual([e["zen_paid"] for e in state["daily"]], [None, 4])

    def test_after_a_missing_day_the_zen_paid_is_unknown(self):
        state = empty_state()
        run(FakeQBittorrent(), state=state, c411=FakeC411(earned=30))
        run(FakeQBittorrent(), state=state, now=NOW + 3 * DAY, c411=FakeC411(earned=40))
        self.assertEqual([e["zen_paid"] for e in state["daily"]], [None, None])

    def test_a_profile_without_the_zen_earned_still_counts_for_the_floor(self):
        state = empty_state()
        run(FakeQBittorrent(), state=state, c411=FakeC411(earned=30))
        c411 = FakeC411()
        c411.earned = None
        run(FakeQBittorrent(), state=state, now=NOW + DAY, c411=c411)
        self.assertFalse(state["paused"])
        self.assertEqual(state["daily"][1]["zen_paid"], None)

    def test_an_unreadable_account_keeps_the_figures_the_day_already_had(self):
        state = empty_state()
        run(FakeQBittorrent(), state=state, c411=FakeC411(zen=20))
        run(FakeQBittorrent(), state=state, now=NOW + 600, c411=FakeC411(broken=True))
        self.assertEqual(state["daily"][0]["zen"], 20)
        self.assertEqual(state["daily"][0]["ratio"], 2.0)
        run(FakeQBittorrent(), state=state, now=NOW + DAY, c411=FakeC411(broken=True))
        self.assertEqual(state["daily"][1]["ratio"], None)
        self.assertEqual(state["daily"][1]["zen_paid"], None)

    def test_the_days_before_the_first_run_come_from_the_account_readings(self):
        state = empty_state(account=[[NOW - 3 * DAY, 50 * GB, 25 * GB, 10], [NOW - 2 * DAY, 54 * GB, 27 * GB, 14]])
        run(FakeQBittorrent(), state=state)
        self.assertEqual([e["day"] for e in state["daily"]], [day(NOW - 3 * DAY), day(NOW - 2 * DAY), day(NOW)])
        first = state["daily"][0]
        self.assertEqual((first["ratio"], first["uploaded_gb"], first["downloaded_gb"], first["zen"]), (2.0, 50, 25, 10))
        self.assertEqual(first["zen_paid"], None)

    def test_the_history_survives_a_restart_and_keeps_the_last_30_days(self):
        state = empty_state()
        with tempfile.TemporaryDirectory() as folder, contextlib.redirect_stdout(io.StringIO()):
            path = os.path.join(folder, "state.json")
            for n in range(40):
                run(FakeQBittorrent(), state=state, now=NOW + n * DAY)
                save_state(path, state)
                state = load_state(path)
        days = [e["day"] for e in state["daily"]]
        self.assertEqual(len(days), 30)
        self.assertEqual(days[0], day(NOW + 10 * DAY))
        self.assertEqual(days[-1], day(NOW + 39 * DAY))


def board_of_run(qbt, state=None, now=NOW, **changes):
    return summary_of(qbt, state, now, indexers=[], **changes)["board"]


class Board(unittest.TestCase):
    def test_each_challenge_lists_its_steps_with_the_progress_of_the_farm(self):
        held, kinds = filled("Films & Vidéos", 112, seeding_days=40, seeders=100)
        challenges = {c["key"]: c for c in board_of_run(FakeQBittorrent(held), empty_state(kinds=kinds))["challenges"]}
        self.assertEqual(list(challenges), ["seed_volume", "contribution", "duration", "diversity", "category"])
        steps = {key: [(s["target"], s["progress"]) for s in c["steps"]] for key, c in challenges.items()}
        self.assertEqual(steps["seed_volume"], [(n, 112) for n in (100, 150, 200, 250, 300)])
        self.assertEqual(steps["contribution"], [(50, 112), (100, 0), (150, 0), (200, 0), (250, 0), (300, 0)])
        self.assertEqual(steps["duration"], [(25, 112), (100, 112), (150, 0), (250, 0), (300, 0)])
        self.assertEqual(steps["diversity"], [(25, 25), (50, 25), (75, 25), (100, 25), (125, 25)])
        self.assertEqual(steps["category"], [(50, 112), (100, 112), (250, 112), (500, 112)])
        self.assertEqual([c["max"] for c in challenges.values()], [True, True, True, False, False])

    def test_the_farm_disk_is_split_by_source(self):
        qbt = FakeQBittorrent([
            torrent("twin1", size_gb=2), torrent("twin2", size_gb=1), torrent("hunt", size_gb=0.5),
            torrent("public", size_gb=4, tracker="http://nyaa.tracker.wf/announce", seeding_days=1),
            torrent("movie", category="movies", size_gb=200),
        ])
        state = empty_state(sources={"twin1": "twin", "twin2": "twin", "hunt": "volume-hunt"})
        farm = board_of_run(qbt, state)["farm"]
        self.assertEqual(farm, {
            "budget_gb": 100,
            "slices": [{"source": "public", "gb": 4}, {"source": "twin", "gb": 3}, {"source": "volume-hunt", "gb": 0.5}],
        })

    def test_the_rankings_go_by_ratio_per_day_of_seeding(self):
        held = [
            torrent("a", size_gb=1, uploaded_gb=10), torrent("b", size_gb=1, uploaded_gb=6, seeding_days=2),
            torrent("c", size_gb=2, uploaded_gb=8), torrent("d", size_gb=1, uploaded_gb=2),
            torrent("big", size_gb=20, uploaded_gb=3), torrent("e", size_gb=1, uploaded_gb=1, seeding_days=20),
            torrent("loading", size_gb=1, progress=0.5), torrent("fresh", size_gb=1, seeding_days=0.5),
        ]
        board = board_of_run(FakeQBittorrent(held), empty_state(sources={"b": "twin"}))
        self.assertEqual([c["name"] for c in board["best"]], ["Torrent b", "Torrent a", "Torrent c"])
        self.assertEqual([c["name"] for c in board["worst"]], ["Torrent big", "Torrent e", "Torrent d"])
        self.assertEqual(board["best"][0], {
            "name": "Torrent b", "source": "twin", "ratio": 6.0, "per_day": 3.0, "note": 1.0, "size_gb": 1, "days": 2,
        })


class Kinds(unittest.TestCase):
    def test_a_c411_torrent_takes_the_category_of_its_largest_files_once(self):
        qbt = FakeQBittorrent([torrent("show"), torrent("book"), torrent("game"), torrent("misc")])
        qbt.names = {
            "show": [("Show.S01E01.mkv", 900), ("Show.nfo", 1)],
            "book": [("Book.epub", 5), ("cover.jpg", 2)],
            "game": [("Tetris.gbc", 1)],
        }
        state = run(qbt)
        self.assertEqual(state["kinds"], {"show": "Films & Vidéos", "book": "Ebook", "game": "Emulation", "misc": ""})
        qbt.names = {"misc": [("Model.stl", 3)]}
        run(qbt, state=state, now=NOW + 600)
        self.assertEqual(state["kinds"]["misc"], "")

    def test_a_public_or_unfinished_torrent_is_not_read(self):
        qbt = FakeQBittorrent([
            torrent("public", tracker="http://nyaa.tracker.wf/announce", seeding_days=1), torrent("loading", progress=0.5),
        ])
        self.assertEqual(run(qbt)["kinds"], {})

    def test_the_seeds_of_each_kind_count_for_the_diversity_challenge(self):
        qbt = FakeQBittorrent([torrent(f"s{i}", size_gb=0.5) for i in range(30)] + [torrent(f"b{i}", size_gb=0.5) for i in range(10)])
        qbt.names = {**{f"s{i}": [("x.mkv", 9)] for i in range(30)}, **{f"b{i}": [("x.epub", 9)] for i in range(10)}}
        challenges = {c["key"]: c for c in board_of_run(qbt)["challenges"]}
        self.assertEqual([s["progress"] for s in challenges["diversity"]["steps"]][:2], [25, 35])


def entry(n, **changes):
    return {
        "day": f"2027-01-{n:02d}", "floor": 1.2, "ratio": 2.0, "left_gb": 20.0, "uploaded_gb": 60.0, "actual_gb": 5.0,
        "downloaded_gb": 30.0, "zen": 25, "earned": 25, "zen_paid": 3, "seeds": 112, **changes,
    }


def challenge(key, targets, progress, has_max=False):
    return {"key": key, "steps": [{"target": t, "progress": progress} for t in targets], "max": has_max}


BOARD = {
    "history": [entry(n) for n in range(1, 31)],
    "challenges": [
        challenge("seed_volume", (100, 150, 200, 250, 300), 112, True),
        challenge("contribution", (50, 100, 150, 200, 250, 300), 40, True),
        challenge("duration", (25, 100, 150, 250, 300), 30, True),
        challenge("diversity", (25, 50, 75, 100, 125), 25),
        challenge("category", (50, 100, 250, 500), 60),
    ],
    "farm": {"budget_gb": 100, "slices": [{"source": "twin", "gb": 30.5}, {"source": "volume-hunt", "gb": 9.5}]},
    "best": [{"name": "Frieren 01", "source": "twin", "ratio": 2.4, "per_day": 0.12, "note": 1.35, "size_gb": 1.4, "days": 20}],
    "worst": [{"name": "Old book", "source": "volume-hunt", "ratio": 0.06, "per_day": 0.002, "note": 1.1, "size_gb": 0.9, "days": 30}],
}


def items(body, css):
    return re.findall(rf'<li class="{css}"><span class="step">([^<]+)</span>', body)


class Page(unittest.TestCase):
    def page(self, board, query=""):
        server = serve({"board": board}, port=0)
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{server.server_port}/graphs{query}") as response:
                return response.headers, response.read().decode()
        finally:
            server.shutdown()
            server.server_close()

    def test_the_first_part_shows_the_big_figures_and_three_curves(self):
        headers, body = self.page(BOARD, "?part=1")
        self.assertTrue(headers["Content-Type"].startswith("text/html"))
        self.assertIn("default-src 'none'", headers["Content-Security-Policy"])
        self.assertNotRegex(body, r"\b(src|href)=")
        values = re.findall(r'<span class="value">([^<]+)</span>', body)
        self.assertEqual(values, ["2.00", "20.0 GB", "60.0 GB", "30.0 GB", "25"])
        self.assertIn("5.0 GB real", body)
        self.assertIn("+3 last night", body)
        self.assertEqual(re.findall(r"<figcaption>([^<]+)", body), ["Account ratio", "Zen", "Upload and download"])
        self.assertIn(">floor 1.2<", body)
        self.assertNotIn('class="quests"', body)

    def test_the_only_script_tells_homepage_the_height_of_the_board_on_a_phone(self):
        headers, body = self.page(BOARD, "?part=1")
        scripts = re.findall(r"<script>(.*?)</script>", body, re.DOTALL)
        self.assertEqual(len(scripts), 1)
        self.assertIn("ratioBoardHeight", scripts[0])
        self.assertIn("(max-width: 640px)", scripts[0])
        digest = base64.b64encode(hashlib.sha256(scripts[0].encode()).digest()).decode()
        self.assertIn(f"script-src 'sha256-{digest}'", headers["Content-Security-Policy"])
        self.assertNotRegex(body, r"b(src|href)=")
        self.assertIn("@media (max-width: 640px)", body)

    def test_the_second_part_shows_each_step_the_farm_disk_and_the_rankings(self):
        _, body = self.page(BOARD, "?part=2")
        self.assertEqual(
            re.findall(r"<h3>([^<]+)</h3>", body),
            ["Seed volume", "Minimum contribution", "Seed duration", "Category diversity", "Category volume"],
        )
        self.assertEqual(items(body, "done")[:1], ["100 seeds"])
        self.assertIn("150 seeds", items(body, "next"))
        self.assertIn("112 / 150", body)
        self.assertIn("300 seeds", items(body, "later"))
        self.assertIn("Every step", items(body, "later"))
        self.assertIn(">150 Zen<", body)
        self.assertIn(">Prime badge<", body)
        self.assertEqual(len(re.findall(r'<li class="(?:done|next|later)">', body)), 6 + 7 + 6 + 5 + 4)
        self.assertIn("40 %", body)
        self.assertIn(">Free<", body)
        self.assertIn(">60.0 GB<", body)
        self.assertIn("Frieren 01", body)
        self.assertIn(">0.120 a day<", body)
        self.assertIn("twin, ratio 2.40, note 1.35, 1.40 GB, 20 days", body)
        self.assertIn(">Best ratio per day<", body)
        self.assertIn(">Lowest ratio per day<", body)
        self.assertIn("Old book", body)
        self.assertNotIn('class="kpis"', body)

    def test_the_french_page_speaks_french(self):
        _, body = self.page(BOARD, "?lang=fr")
        self.assertIn('<html lang="fr" class="standalone">', body)
        self.assertIn(">Ratio du compte<", body)
        self.assertIn(">plancher 1,2<", body)
        self.assertIn(">20,0 Go<", body)
        self.assertIn("<h3>Volume de seed</h3>", body)
        self.assertIn(">Badge Prime<", body)
        self.assertIn(">Plus faible ratio par jour<", body)
        self.assertIn(">0,120 par jour<", body)
        self.assertIn(">Même release prise sur un tracker public, gratuite sur C411<", body)

    def test_the_time_axis_always_spans_30_days(self):
        _, body = self.page({**BOARD, "history": [entry(30)]}, "?part=1")
        self.assertIn('x1="98.33"', body)
        self.assertIn("<span>01/01</span><span>30/01</span>", body)

    def test_a_missing_day_leaves_a_gap_in_the_curve(self):
        _, body = self.page({**BOARD, "history": [entry(1), entry(2), entry(5)]}, "?part=1")
        self.assertEqual(len(re.findall(r'<polyline class="line c-ratio"', body)), 1)
        self.assertEqual(len(re.findall(r'<line class="dot c-ratio"', body)), 1)

    def test_a_part_has_no_background_of_its_own_so_homepage_shows_through(self):
        for part in ("1", "2"):
            _, body = self.page(BOARD, f"?part={part}")
            self.assertIn('<html lang="en">', body)
            self.assertNotRegex(body.split("html.standalone")[0], r"color-scheme")

    def test_without_a_part_the_page_shows_both_on_a_dark_background(self):
        _, body = self.page(BOARD)
        self.assertIn('<html lang="en" class="standalone">', body)
        self.assertIn('class="kpis"', body)
        self.assertIn('class="quests"', body)

    def test_the_page_answers_with_a_query_or_a_trailing_slash(self):
        for query in ("/", "?refresh=1"):
            _, body = self.page(BOARD, query)
            self.assertIn('class="quests"', body)

    def test_without_history_the_page_says_so(self):
        for board in (None, {**BOARD, "history": []}):
            _, body = self.page(board)
            self.assertIn("No history yet", body)
            self.assertNotIn("<svg", body)


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
        qbt.cats = {}
        qbt.prefs = {
            "max_ratio_enabled": True, "max_seeding_time_enabled": True, "max_inactive_seeding_time_enabled": True,
            "queueing_enabled": True,
        }
        run(qbt, cross_seed_key="secret")
        self.assertEqual(qbt.made, {
            "movies": "/data/torrents/movies", "shows": "/data/torrents/shows", "music": "/data/torrents/music",
            "books": "/data/torrents/books", "farm": "/data/torrents/farm",
        })
        self.assertFalse(qbt.prefs["max_ratio_enabled"])
        self.assertFalse(qbt.prefs["max_seeding_time_enabled"])
        self.assertFalse(qbt.prefs["max_inactive_seeding_time_enabled"])
        self.assertFalse(qbt.prefs["queueing_enabled"])
        self.assertTrue(qbt.prefs["autorun_enabled"])
        self.assertIn('X-Api-Key: secret', qbt.prefs["autorun_program"])
        self.assertIn("infoHash=%I", qbt.prefs["autorun_program"])


class State(unittest.TestCase):
    def test_a_file_without_the_newer_keys_still_loads(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "state.json")
            save_state(path, {"transfers": {"a": [6 * GB, GB]}, "grabbed": {"a": 100}})
            self.assertEqual(load_state(path), empty_state(transfers={"a": [6 * GB, GB]}))

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

    def test_tags_share_limits_and_auto_management_go_to_their_endpoints(self):
        self.qbt.add_tags(["a", "b"], "public")
        self.qbt.reset_share_limits(["a"])
        self.qbt.remove_tags(["a"], "zen-hunt")
        self.qbt.keep_in_place(["a"])
        calls = [(path, body) for path, _, body in FakeServer.calls if path != "/api/v2/auth/login"]
        self.assertEqual(calls, [
            ("/api/v2/torrents/addTags", b"hashes=a%7Cb&tags=public"),
            ("/api/v2/torrents/setShareLimits", b"hashes=a&ratioLimit=-2&seedingTimeLimit=-2&inactiveSeedingTimeLimit=-2&shareLimitAction=Default&shareLimitsMode=Default"),
            ("/api/v2/torrents/removeTags", b"hashes=a&tags=zen-hunt"),
            ("/api/v2/torrents/setAutoManagement", b"hashes=a&enable=false"),
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
        if self.path.startswith("/9/api"):
            self.answer(200, FEED)
        elif self.path.startswith("/3/api"):
            self.answer(429, b"", {"Retry-After": "1200"})
        elif self.path.startswith("/5/api"):
            body = b'<error code="429" description="Indexer is disabled till 11/08/2026 15:39:18 due to recent failures." />'
            self.answer(429, body, {"Retry-After": "6614"})
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

    def test_a_search_sends_the_key_in_a_header_and_not_in_the_url(self):
        self.prowlarr.search("9", "Some.Show.S01", 1793743200)
        [(_, path, key, _)] = FakeProwlarrServer.calls
        self.assertTrue(path.startswith("/9/api?"))
        self.assertNotIn("apikey", path)
        self.assertEqual(key, "key")

    def test_holding_c411_tells_the_profile_it_leaves(self):
        self.assertEqual(self.prowlarr.hold("3", True, "Paused"), 1)

    def test_an_indexer_already_on_the_right_profile_is_left_alone(self):
        self.prowlarr.hold("3", False, "Paused")
        self.assertEqual(self.writes(), [])

    def test_a_429_raises_with_the_wait_prowlarr_asks_for(self):
        with self.assertRaises(Throttled) as throttled:
            self.prowlarr.newest("3", NOW)
        self.assertEqual(throttled.exception.retry_after, 1200)
        self.assertFalse(throttled.exception.disabled)

    def test_a_429_of_an_indexer_disabled_after_failures_says_so(self):
        with self.assertRaises(Throttled) as throttled:
            self.prowlarr.newest("5", NOW)
        self.assertEqual(throttled.exception.retry_after, 6614)
        self.assertTrue(throttled.exception.disabled)


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
        self.assertEqual(account, {
            "uploaded": 59134463945, "downloaded": 28231492173, "zen": 25, "earned": 25, "actual": 5447372745,
        })
        self.assertEqual(FakeC411Server.calls, [("/api/user/profile", "Bearer secret")])


if __name__ == "__main__":
    unittest.main()
