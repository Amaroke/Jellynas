import contextlib
import datetime
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
    DONE_AT_START,
    GB,
    NUMBERS,
    SWITCHES,
    Controls,
    OutOfCalls,
    Prowlarr,
    QBittorrent,
    Throttled,
    bdecode,
    load_state,
    note_of,
    parse_torznab,
    rare_coef,
    release_of,
    run_once,
    save_state,
    scales_of,
    serve,
    top_note,
)

NOW = 1_800_000_000
TRACKER = "https://tracker.c411.org/announce"
PUBLIC = "http://nyaa.tracker.wf/announce"
SWARM_KEEPER = {
    "slug": "swarm_keeper",
    "bonusTiers": {
        "seed_duration": [{"bucket": "7", "coef": 0.1}, {"bucket": "30", "coef": 0.25}, {"bucket": "90", "coef": 0.35}, {"bucket": "180", "coef": 0.5}],
        "seeders_weight": [
            {"bucket": "<1|1-10", "coef": 0.2}, {"bucket": "<1|11-24", "coef": 0.2}, {"bucket": "1-5|11-24", "coef": 0.2},
            {"bucket": "1-5|1-10", "coef": 0.35}, {"bucket": "5-10|1-10", "coef": 0.35}, {"bucket": "11-50|1-10", "coef": 0.5},
            {"bucket": ">100|1-10", "coef": 0.5}, {"bucket": "51-100|1-10", "coef": 0.5},
        ],
    },
}
SCALES = scales_of(SWARM_KEEPER)


def torrent(hash, category="farm", size_gb=3, seeding_days=10, uploaded_gb=0, downloaded_gb=0, progress=1,
            seeders=100, tracker=TRACKER, completion_on=0, save_path=None, state="stalledUP", tags=""):
    return {
        "tags": tags,
        "hash": hash,
        "save_path": save_path or f"/data/torrents/{category}",
        "state": state,
        "name": f"Torrent {hash}",
        "category": category,
        "size": int(size_gb * GB),
        "progress": progress,
        "seeding_time": seeding_days * DAY,
        "uploaded": uploaded_gb * GB,
        "downloaded": downloaded_gb * GB,
        "completion_on": completion_on,
        "added_on": NOW - seeding_days * DAY,
        "num_complete": seeders,
        "tracker": tracker,
    }


def release(infohash, leechers=40, seeders=1, size_gb=3, age_hours=0.5, title=None, free=False, category="films-videos"):
    return {
        "infohash": infohash,
        "title": title or f"Release.{infohash}.1080p",
        "link": "",
        "page": "",
        "size": int(size_gb * GB),
        "seeders": seeders,
        "leechers": leechers,
        "age_hours": age_hours,
        "free": free,
        "category": category,
    }


def small(infohash, size_gb=0.5, seeders=3, free=False, category="ebook"):
    return release(infohash, leechers=0, seeders=seeders, size_gb=size_gb, free=free, category=category)


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
    def __init__(self, twins=(), files=None, throttle=None, disabled=False):
        self.twins = list(twins)
        self.files = files or {}
        self.throttle = throttle
        self.disabled = disabled
        self.queries = []
        self.fetched = []
        self.holds = []
        self.restores = []

    def search(self, indexer, title, now):
        if self.throttle:
            raise Throttled(self.throttle, self.disabled)
        self.queries.append(title)
        return list(self.twins)

    def torrent(self, release):
        self.fetched.append(release["infohash"])
        return torrent_file(release["infohash"], self.files.get(release["infohash"], [7]))

    def hold(self, indexer, paused, profile, restore=None):
        self.holds.append((indexer, paused, profile))
        self.restores.append(restore)
        return 7 if paused else None


class FakeC411:
    def __init__(self, newest=(), hunt=None, uploaded_gb=60, downloaded_gb=30, zen=25, broken=False, earned=None,
                 actual_gb=5, files=None, refused=(), failing=(), categories=None, voies=None, slug="swarm_keeper"):
        self.newest = list(newest)
        self.hunt = hunt or {}
        self.uploaded = uploaded_gb * GB
        self.downloaded = downloaded_gb * GB
        self.zen = zen
        self.earned = zen if earned is None else earned
        self.actual = actual_gb * GB
        self.broken = broken
        self.files = files or {}
        self.refused = set(refused)
        self.failing = set(failing)
        self.known = categories or {}
        self.voies = voies or {"seed": 3, "specialization": 6}
        self.slug = slug
        self.reads = 0
        self.scale_reads = 0
        self.calls_left = None
        self.log = []
        self.fetched = []
        self.looked = []

    def allow(self, calls):
        self.calls_left = calls

    def spend(self):
        if self.calls_left is not None:
            if self.calls_left <= 0:
                raise OutOfCalls()
            self.calls_left -= 1

    def account(self):
        self.spend()
        self.reads += 1
        if self.broken:
            raise RuntimeError("Site is currently restricted")
        return {
            "uploaded": self.uploaded, "downloaded": self.downloaded, "zen": self.zen, "earned": self.earned,
            "actual": self.actual, "credit": 50 * GB, "class": self.slug, "grade": ["bronze", 3],
            "voies": dict(self.voies), "allowed": True,
        }

    def catalog(self, now, category=None, sort="createdAt", order="desc", page=1):
        self.spend()
        self.log.append(("catalog", category, sort, page))
        if sort == "createdAt":
            return list(self.newest), 1
        pages = self.hunt.get(category or "all", [])
        if pages and not isinstance(pages[0], list):
            pages = [pages]
        return (list(pages[page - 1]) if page <= len(pages) else []), len(pages)

    def category(self, infohash):
        self.spend()
        self.looked.append(infohash)
        return self.known.get(infohash, "films-videos")

    def download(self, infohash):
        self.spend()
        self.fetched.append(infohash)
        self.log.append(f"download {infohash}")
        if infohash in self.refused:
            raise urllib.error.HTTPError("https://c411.org", 403, "Forbidden", {}, None)
        if infohash in self.failing:
            raise RuntimeError("C411 sent no torrent file")
        return torrent_file(infohash, self.files.get(infohash, [7]))

    def scales(self, slug):
        self.spend()
        self.scale_reads += 1
        return scales_of(SWARM_KEEPER)


def under_floor():
    return FakeC411(uploaded_gb=30, downloaded_gb=30)


SETTINGS = {
    "tracker": "c411",
    "hold_indexers": ["3"],
    "twin_indexers": ["9"],
    "cross_seed_key": "",
    "farm_size": 100 * GB,
    "floor": 1.2,
    "daily": 2 * GB,
    "max_size": GB,
    "paused_profile": "Paused",
    "free_max_size": 5 * GB,
    "paid_max_age_hours": 24,
    "paid_min_leechers": 5,
    "public_seeding": 14 * DAY,
    "max_downloading": 2,
    "c411_calls": 100,
    "twin_lookups": 10,
    "library_categories": ("movies", "shows", "music", "books"),
    "adopt_after": 6 * 3600,
    "twin_max_age_hours": 24,
    "twin_min_leechers": 1,
    "twin_recheck": 2 * 3600,
    "twin_seed_after": 3600,
    "board_refresh": "http://farm-board:8000/refresh",
}
QUIET = {**{name: False for name in SWITCHES if name.startswith("volume:")}, "quantity_seed": False}


def settings(**changes):
    return {**SETTINGS, **changes}


def controls_of(found, switches=None):
    numbers = {name: found[key] / unit for name, (key, unit, _, _) in NUMBERS.items()}
    return {"switches": {**SWITCHES, **QUIET, **(switches or {})}, "numbers": numbers}


def empty_state(**changes):
    return {
        "transfers": {}, "hunted": {}, "twins": {}, "free": {}, "checked": {}, "paused": False,
        "sources": {}, "history": {}, "zen": {}, "report": {}, "account": [], "received": [], "held": None, "backoff_until": 0,
        "stuck": {}, "restore": {}, "categories": {}, "pages": {}, "daily": [], "given_up": {},
        "done": list(DONE_AT_START), "scales": None, "tier": None, **changes,
    }


def run_board(qbt, prowlarr=None, state=None, now=NOW, c411=None, switches=None, notify=None, **changes):
    state = state if state is not None else empty_state()
    found = settings(**changes)
    with contextlib.redirect_stdout(io.StringIO()):
        board = run_once(
            qbt, prowlarr or FakeProwlarr(), c411 or FakeC411(), found, controls_of(found, switches), state, now,
            notify or (lambda url: None),
        )
    return board, state


def run(qbt, prowlarr=None, state=None, now=NOW, c411=None, switches=None, **changes):
    return run_board(qbt, prowlarr, state, now, c411, switches, **changes)[1]


EPISODE = "Les.Carnets.de.l.apothicaire.S03E06.VOSTFR.1080p.WEBRip.x265-KAF"
NYAA = "The.Apothecary.Diaries.S03E06.VOSTFR.1080p.WEBRiP.x265-KAF"


def episode(leechers=3):
    return release("episode", title=EPISODE, leechers=leechers, seeders=60, size_gb=0.27)


def twin(title=NYAA, size_gb=0.28):
    return release("public", title=title, seeders=200, leechers=5, size_gb=size_gb)


def waiting_twin(at=NOW - 3600):
    return {"public": ["hungry", "farm", at, ""]}


def nyaa_copy(**changes):
    return torrent("public", tracker=PUBLIC, **changes)


class Twins(unittest.TestCase):
    def test_a_fresh_episode_with_a_twin_downloads_the_twin(self):
        qbt = FakeQBittorrent()
        prowlarr = FakeProwlarr(twins=[twin()])
        state = run(qbt, prowlarr, c411=FakeC411([episode()]))
        self.assertEqual(prowlarr.queries, ["S03E06 VOSTFR"])
        self.assertEqual(qbt.added, [("public", "farm")])
        self.assertEqual(state["twins"], {"public": ["episode", "farm", NOW, ""]})
        self.assertEqual(state["categories"]["episode"], "films-videos")
        self.assertEqual(state["hunted"], {})

    def test_a_twin_is_grabbed_even_under_the_floor(self):
        qbt = FakeQBittorrent()
        c411 = under_floor()
        c411.newest = [episode()]
        run(qbt, FakeProwlarr(twins=[twin()]), c411=c411)
        self.assertEqual(qbt.added, [("public", "farm")])

    def test_a_release_is_fetched_once_for_several_candidates(self):
        c411 = FakeC411([episode()])
        run(FakeQBittorrent(), FakeProwlarr(twins=[twin(), twin()], files={"public": [8]}), c411=c411)
        self.assertEqual(c411.fetched.count("episode"), 1)

    def test_a_twin_must_hold_the_same_files(self):
        qbt = FakeQBittorrent()
        state = run(qbt, FakeProwlarr(twins=[twin()], files={"public": [8]}), c411=FakeC411([episode()]))
        self.assertEqual(qbt.added, [])
        self.assertEqual(state["twins"], {})

    def test_a_twin_must_come_from_the_same_group(self):
        qbt = FakeQBittorrent()
        run(qbt, FakeProwlarr(twins=[twin(title="Show.S03E06.VOSTFR.1080p-OTHER")]), c411=FakeC411([episode()]))
        self.assertEqual(qbt.added, [])

    def test_a_twin_size_may_be_rounded_but_not_far_off(self):
        qbt = FakeQBittorrent()
        run(qbt, FakeProwlarr(twins=[twin(size_gb=0.4)]), c411=FakeC411([episode()]))
        self.assertEqual(qbt.added, [])

    def test_a_release_without_an_episode_or_a_leecher_is_not_searched(self):
        prowlarr = FakeProwlarr(twins=[twin()])
        run(FakeQBittorrent(), prowlarr, c411=FakeC411([release("movie", leechers=0), episode(leechers=0)]))
        self.assertEqual(prowlarr.queries, [])

    def test_a_release_checked_recently_is_not_searched_again(self):
        prowlarr = FakeProwlarr(twins=[twin()])
        run(FakeQBittorrent(), prowlarr, empty_state(checked={"episode": NOW - 3600}), c411=FakeC411([episode()]))
        self.assertEqual(prowlarr.queries, [])

    def test_the_twins_switch_turned_off_searches_no_twin(self):
        prowlarr = FakeProwlarr(twins=[twin()])
        run(FakeQBittorrent(), prowlarr, c411=FakeC411([episode()]), switches={"twins": False})
        self.assertEqual(prowlarr.queries, [])

    def test_once_the_c411_copy_seeds_the_twin_is_removed(self):
        qbt = FakeQBittorrent([nyaa_copy(), torrent("hungry", save_path="/data/torrents/cross-seed/C411")])
        state = run(qbt, state=empty_state(twins=waiting_twin(NOW - 7200)))
        self.assertEqual(qbt.deleted, ["public"])
        self.assertEqual(state["twins"], {})

    def test_the_drop_switch_turned_off_keeps_the_twin(self):
        qbt = FakeQBittorrent([nyaa_copy(), torrent("hungry")])
        state = run(qbt, state=empty_state(twins=waiting_twin(NOW - 7200)), switches={"drop_twins": False, "clean_public": False})
        self.assertEqual(qbt.deleted, [])
        self.assertIn("public", state["twins"])

    def test_the_twin_stays_until_cross_seed_adds_the_c411_copy(self):
        qbt = FakeQBittorrent([nyaa_copy()])
        state = run(qbt, state=empty_state(twins=waiting_twin(NOW - 1800)))
        self.assertEqual((qbt.deleted, qbt.added), ([], []))
        self.assertEqual(state["twins"], waiting_twin(NOW - 1800))

    def test_when_cross_seed_misses_the_c411_copy_ratio_seeds_it_from_the_twin_files(self):
        qbt = FakeQBittorrent([nyaa_copy()])
        run(qbt, state=empty_state(twins=waiting_twin()))
        self.assertEqual(qbt.added, [("hungry", "farm", "/data/torrents/farm")])
        self.assertEqual(qbt.matched, [("hungry", [{"name": "public.dat", "size": 7}])])

    def test_a_twin_of_an_older_state_still_finds_its_c411_copy(self):
        qbt = FakeQBittorrent([nyaa_copy()])
        run(qbt, state=empty_state(twins={"public": ["hungry", "ratio", NOW - 3600, "http://prowlarr/hungry.torrent"]}))
        self.assertEqual(qbt.added, [("hungry", "farm", "/data/torrents/farm")])

    def test_a_twin_sharing_its_files_with_the_c411_copy_leaves_them_on_disk(self):
        qbt = FakeQBittorrent([nyaa_copy(), torrent("hungry")])
        run(qbt, state=empty_state(twins=waiting_twin(NOW - 7200)))
        self.assertEqual(qbt.kept, ["public"])

    def test_a_c411_copy_that_fails_waits_longer_before_each_retry(self):
        qbt = FakeQBittorrent([nyaa_copy()])
        state = empty_state(twins=waiting_twin())
        c411 = FakeC411(failing={"hungry"})
        for minutes in range(0, 120, 10):
            run(qbt, state=state, now=NOW + 60 * minutes, c411=c411)
        self.assertEqual(c411.fetched, ["hungry", "hungry"])
        run(qbt, state=state, now=NOW + 3 * 3600, c411=c411)
        self.assertEqual(c411.fetched, ["hungry"] * 3)

    def test_a_c411_copy_is_given_up_after_its_third_failure(self):
        qbt = FakeQBittorrent([nyaa_copy()])
        state = empty_state(twins=waiting_twin())
        c411 = FakeC411(failing={"hungry"})
        for hours in range(6):
            run(qbt, state=state, now=NOW + 3600 * hours, c411=c411)
        self.assertEqual(c411.fetched, ["hungry"] * 3)
        self.assertEqual((state["twins"], state["stuck"]), ({}, {}))
        self.assertEqual(qbt.deleted, [])

    def test_a_c411_copy_given_up_is_not_grabbed_again_for_a_week(self):
        qbt = FakeQBittorrent([nyaa_copy()])
        state = empty_state(twins=waiting_twin())
        run(qbt, state=state, c411=FakeC411(refused={"hungry"}))
        c411 = FakeC411([release("hungry", free=True)])
        run(qbt, state=state, now=NOW + DAY, c411=c411)
        self.assertEqual(qbt.added, [])
        run(qbt, state=state, now=NOW + 8 * DAY, c411=c411)
        self.assertEqual(qbt.added, [("hungry", "farm")])

    def test_a_qbittorrent_refusal_is_not_a_c411_refusal(self):
        qbt = FakeQBittorrent([nyaa_copy()])

        def conflict(infohash, wanted):
            raise urllib.error.HTTPError("http://gluetun:8080", 409, "Conflict", {}, None)

        qbt.match_files = conflict
        state = empty_state(twins=waiting_twin())
        run(qbt, state=state)
        self.assertIn("public", state["twins"])
        self.assertEqual(state["stuck"]["public"][0], 1)

    def test_a_c411_copy_given_up_after_its_add_leaves_no_stopped_copy(self):
        qbt = FakeQBittorrent([nyaa_copy()])

        def mismatch(infohash, wanted):
            raise RuntimeError("the files differ")

        qbt.match_files = mismatch
        state = empty_state(twins=waiting_twin())
        for hours in range(6):
            run(qbt, state=state, now=NOW + 3600 * hours)
        self.assertEqual(state["twins"], {})
        self.assertEqual((qbt.deleted, qbt.kept), (["hungry"], ["hungry"]))

    def test_a_c411_copy_c411_refuses_is_given_up_at_once(self):
        qbt = FakeQBittorrent([nyaa_copy()])
        state = empty_state(twins=waiting_twin())
        c411 = FakeC411(refused={"hungry"})
        run(qbt, state=state, c411=c411)
        run(qbt, state=state, now=NOW + DAY, c411=c411)
        self.assertEqual(c411.fetched, ["hungry"])
        self.assertEqual(state["twins"], {})

    def test_a_c411_copy_left_stopped_after_its_check_is_started(self):
        qbt = FakeQBittorrent([nyaa_copy(), torrent("hungry", state="stoppedUP")])
        run(qbt, state=empty_state(twins=waiting_twin(NOW - 7200)))
        self.assertEqual(qbt.started, ["hungry"])


class Grabs(unittest.TestCase):
    def test_a_fresh_paid_release_waits_for_its_switch(self):
        qbt = FakeQBittorrent()
        state = run(qbt, c411=FakeC411([release("hungry", seeders=80, leechers=200)]))
        self.assertEqual(qbt.added, [])
        self.assertEqual(state["hunted"], {})

    def test_with_its_switch_a_fresh_paid_release_with_leechers_is_grabbed_most_leechers_per_gb_first(self):
        qbt = FakeQBittorrent()
        c411 = FakeC411([
            release("few", leechers=4, size_gb=0.5),
            release("old", leechers=50, size_gb=0.5, age_hours=30),
            release("big", leechers=40, size_gb=1.5),
            release("dense", leechers=30, size_gb=0.5),
        ])
        state = run(qbt, c411=c411, switches={"paid": True})
        self.assertEqual(qbt.added, [("dense", "farm"), ("big", "farm")])
        self.assertEqual(state["sources"]["dense"], "paid")
        self.assertEqual(state["hunted"]["dense"], [NOW, int(0.5 * GB)])

    def test_a_paid_grab_never_happens_under_the_floor(self):
        qbt = FakeQBittorrent()
        c411 = under_floor()
        c411.newest = [release("hungry", leechers=50, size_gb=0.5)]
        run(qbt, c411=c411, switches={"paid": True})
        self.assertEqual(qbt.added, [])

    def test_a_paid_grab_stays_within_the_daily_download(self):
        qbt = FakeQBittorrent()
        c411 = FakeC411([release("a", leechers=50, size_gb=1.5), release("b", leechers=40, size_gb=1.5)])
        run(qbt, c411=c411, switches={"paid": True})
        self.assertEqual(qbt.added, [("a", "farm")])

    def test_a_freeleech_release_is_grabbed_even_under_the_floor(self):
        qbt = FakeQBittorrent()
        c411 = under_floor()
        c411.newest = [release("free", seeders=30, free=True)]
        state = run(qbt, c411=c411)
        self.assertEqual(qbt.added, [("free", "farm")])
        self.assertEqual(state["free"], {"free": NOW})
        self.assertEqual(state["hunted"], {})

    def test_the_free_switch_turned_off_grabs_no_freeleech(self):
        qbt = FakeQBittorrent()
        run(qbt, c411=FakeC411([release("free", seeders=30, free=True)]), switches={"free": False})
        self.assertEqual(qbt.added, [])

    def test_a_freeleech_download_does_not_count_in_the_deficit(self):
        qbt = FakeQBittorrent([torrent("free", downloaded_gb=4, uploaded_gb=1)])
        state = run(qbt, state=empty_state(free={"free": NOW - 100}))
        self.assertEqual(state["transfers"], {"free": [0, 1 * GB]})

    def test_two_downloads_at_once_at_most(self):
        qbt = FakeQBittorrent([torrent("a", progress=0.5), torrent("b", progress=0.1)])
        c411 = FakeC411([release("free", free=True), episode()])
        run(qbt, FakeProwlarr(twins=[twin()]), c411=c411)
        self.assertEqual(qbt.added, [])
        self.assertEqual(c411.log, [])

    def test_a_full_farm_grabs_nothing_and_loses_nothing(self):
        qbt = FakeQBittorrent([torrent("big", size_gb=60, seeders=100), torrent("old", size_gb=45, seeding_days=400)])
        c411 = FakeC411([release("free", free=True, size_gb=0.1)])
        run(qbt, c411=c411)
        self.assertEqual((qbt.added, qbt.deleted), ([], []))

    def test_the_farm_budget_set_in_homepage_counts_over_the_one_of_env(self):
        qbt = FakeQBittorrent([torrent("big", size_gb=60)])
        state = empty_state()
        found = settings()
        controls = controls_of(found)
        controls["numbers"]["farm_size_gb"] = 50
        with contextlib.redirect_stdout(io.StringIO()):
            board = run_once(qbt, FakeProwlarr(), FakeC411([release("free", free=True, size_gb=0.1)]), found, controls, state, NOW, lambda url: None)
        self.assertEqual(qbt.added, [])
        self.assertEqual(board["farm"]["budget_gb"], 50)

    def test_an_unreadable_profile_still_grabs_freeleech(self):
        qbt = FakeQBittorrent()
        c411 = FakeC411([release("free", free=True)], broken=True)
        run(qbt, c411=c411, switches={"paid": True})
        self.assertEqual(qbt.added, [("free", "farm")])


class Floor(unittest.TestCase):
    def test_the_profile_is_read_once_per_run(self):
        c411 = FakeC411([episode()])
        run(FakeQBittorrent(), FakeProwlarr(twins=[twin()]), c411=c411)
        self.assertEqual(c411.reads, 1)

    def test_under_the_floor_c411_is_held_in_prowlarr(self):
        prowlarr = FakeProwlarr()
        state = run(FakeQBittorrent(), prowlarr, c411=FakeC411(uploaded_gb=35, downloaded_gb=30))
        self.assertEqual(prowlarr.holds, [("3", True, "Paused")])
        self.assertTrue(state["paused"])
        self.assertTrue(state["held"])
        self.assertEqual(state["restore"], {"3": 7})

    def test_the_hold_switch_turned_off_leaves_c411_open_under_the_floor(self):
        prowlarr = FakeProwlarr()
        state = run(FakeQBittorrent(), prowlarr, empty_state(held=True, restore={"3": 7}), c411=under_floor(), switches={"hold": False})
        self.assertEqual(prowlarr.holds, [("3", False, "Paused")])
        self.assertTrue(state["paused"])
        self.assertFalse(state["held"])

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
        prowlarr = FakeProwlarr()
        state = run(FakeQBittorrent(), prowlarr, c411=FakeC411(broken=True))
        self.assertEqual(prowlarr.holds, [("3", True, "Paused")])
        self.assertTrue(state["paused"])

    def test_a_paid_grab_never_spends_past_the_floor(self):
        qbt = FakeQBittorrent()
        c411 = FakeC411(hunt={"ebook": [small("b", size_gb=0.4), small("a", size_gb=0.3)]}, uploaded_gb=42, downloaded_gb=34.5)
        run(qbt, c411=c411, switches={"volume:ebook": True})
        self.assertEqual(qbt.added, [("a", "farm")])

    def test_a_c411_download_in_progress_counts_against_the_floor(self):
        qbt = FakeQBittorrent([torrent("movie", category="movies", size_gb=4, progress=0.5)])
        c411 = FakeC411(hunt={"ebook": [small("rare")]}, uploaded_gb=42, downloaded_gb=33)
        run(qbt, c411=c411, switches={"volume:ebook": True})
        self.assertEqual(qbt.added, [])


class Throttling(unittest.TestCase):
    def test_a_fall_under_the_floor_holds_c411_even_while_prowlarr_asks_to_wait(self):
        prowlarr = FakeProwlarr()
        run(FakeQBittorrent(), prowlarr, empty_state(held=False, backoff_until=NOW + 1800), c411=under_floor())
        self.assertEqual(prowlarr.holds, [("3", True, "Paused")])

    def test_a_429_from_prowlarr_stops_its_use_until_the_wait_is_over(self):
        prowlarr = FakeProwlarr(twins=[twin()], throttle=1800)
        state = run(FakeQBittorrent(), prowlarr, c411=FakeC411([episode()]))
        self.assertEqual(state["backoff_until"], NOW + 1800)
        prowlarr.throttle = None
        run(FakeQBittorrent(), prowlarr, state, now=NOW + 600, c411=FakeC411([episode()]))
        self.assertEqual(prowlarr.queries, [])
        run(FakeQBittorrent(), prowlarr, state, now=NOW + 3 * 3600, c411=FakeC411([episode()]))
        self.assertEqual(prowlarr.queries, ["S03E06 VOSTFR"])

    def test_once_the_calls_of_the_run_are_spent_the_rest_waits(self):
        qbt = FakeQBittorrent([torrent("a"), torrent("b"), torrent("c")])
        c411 = FakeC411()
        state = run(qbt, c411=c411, c411_calls=4)
        self.assertEqual(len(c411.looked), 1)
        self.assertEqual(len(state["categories"]), 1)
        run(qbt, state=state, now=NOW + 600, c411=c411, c411_calls=4)
        self.assertEqual(len(state["categories"]), 3)


class Quests(unittest.TestCase):
    def test_a_category_switch_hunts_its_category_cheapest_first_freeleech_before_paid(self):
        qbt = FakeQBittorrent()
        c411 = FakeC411(hunt={"ebook": [
            small("big", size_gb=1.5), small("dead", seeders=0), small("dear", size_gb=0.8),
            small("cheap", size_gb=0.2), small("free", size_gb=0.9, free=True),
        ]})
        state = run(qbt, c411=c411, switches={"volume:ebook": True}, max_downloading=3)
        self.assertEqual(qbt.added, [("free", "farm"), ("cheap", "farm"), ("dear", "farm")])
        self.assertEqual([e for e in c411.log if e[0] == "catalog" and e[1]], [("catalog", "ebook", "size", 1)])
        self.assertEqual(state["sources"], {"free": "free", "cheap": "quest", "dear": "quest"})
        self.assertEqual(state["categories"], {"free": "ebook", "cheap": "ebook", "dear": "ebook"})

    def test_among_releases_of_one_size_the_rarest_for_swarm_keeper_comes_first(self):
        qbt = FakeQBittorrent()
        c411 = FakeC411(hunt={"ebook": [small("crowded", seeders=30), small("rare", seeders=3)]})
        run(qbt, c411=c411, switches={"volume:ebook": True}, max_downloading=1)
        self.assertEqual(qbt.added, [("rare", "farm")])

    def test_a_category_stops_at_the_50_seeds_of_its_badge(self):
        held = [torrent(f"e{i}", size_gb=0.1) for i in range(50)]
        state = empty_state(categories={f"e{i}": "ebook" for i in range(50)})
        c411 = FakeC411(hunt={"ebook": [small("more")]})
        run(FakeQBittorrent(held), state=state, c411=c411, switches={"volume:ebook": True})
        self.assertNotIn(("catalog", "ebook", "size", 1), c411.log)

    def test_films_aims_at_the_next_level_of_the_volume_challenge(self):
        held = [torrent(f"f{i}", size_gb=0.1) for i in range(75)]
        state = empty_state(categories={f"f{i}": "films-videos" for i in range(75)})
        c411 = FakeC411(hunt={"films-videos": [small("more", category="films-videos")]})
        board, _ = run_board(FakeQBittorrent(held), state=state, c411=c411, switches={"volume:films-videos": True})
        self.assertIn(("catalog", "films-videos", "size", 1), c411.log)
        films = next(c for c in board["categories"] if c["slug"] == "films-videos")
        self.assertEqual((films["count"], films["target"]), (75, 100))

    def test_the_seed_count_switch_hunts_every_category_up_to_the_next_level(self):
        held = [torrent(f"s{i}", size_gb=0.01) for i in range(199)]
        c411 = FakeC411(hunt={"all": [small("one"), small("two")]})
        qbt = FakeQBittorrent(held)
        run(qbt, c411=c411, switches={"quantity_seed": True})
        self.assertEqual(qbt.added, [("one", "farm")])
        self.assertIn(("catalog", None, "size", 1), c411.log)

    def test_a_page_with_nothing_new_moves_the_hunt_to_the_next_page(self):
        qbt = FakeQBittorrent([torrent("old")])
        c411 = FakeC411(hunt={"ebook": [[small("old")], [small("new")]]})
        state = run(qbt, c411=c411, switches={"volume:ebook": True})
        self.assertEqual(state["pages"], {"ebook": 2})
        run(qbt, state=state, now=NOW + 600, c411=c411, switches={"volume:ebook": True})
        self.assertEqual(qbt.added, [("new", "farm")])

    def test_under_the_floor_only_freeleech_is_hunted(self):
        qbt = FakeQBittorrent()
        c411 = FakeC411(hunt={"ebook": [small("paid", size_gb=0.1), small("free", free=True)]}, uploaded_gb=30, downloaded_gb=30)
        run(qbt, c411=c411, switches={"volume:ebook": True})
        self.assertEqual(qbt.added, [("free", "farm")])

    def test_a_quest_is_validated_once_its_count_is_reached_and_c411_counts_one_more(self):
        held = [torrent(f"s{i}", size_gb=0.01) for i in range(200)]
        state = run(FakeQBittorrent(held), c411=FakeC411(voies={"seed": 3, "specialization": 6}))
        self.assertNotIn(44, state["done"])
        state = run(FakeQBittorrent(held), state=state, now=NOW + 600, c411=FakeC411(voies={"seed": 4, "specialization": 6}))
        self.assertIn(44, state["done"])

    def test_a_count_reached_without_c411_validating_it_is_not_a_quest_done(self):
        held = [torrent(f"s{i}", size_gb=0.01, seeding_days=8) for i in range(30)]
        state = run(FakeQBittorrent(held), c411=FakeC411(voies={"seed": 3, "specialization": 6}))
        self.assertEqual(state["done"], sorted(DONE_AT_START))

    def test_the_board_shows_each_challenge_at_its_next_level(self):
        held = [torrent(f"s{i}", size_gb=0.01, seeding_days=8, seeders=5) for i in range(30)]
        board, _ = run_board(FakeQBittorrent(held))
        by_key = {c["key"]: c for c in board["challenges"]}
        self.assertEqual((by_key["quantity_seed"]["level"], by_key["quantity_seed"]["target"]), (3, 200))
        self.assertEqual(by_key["quantity_seed"]["switch"], "quantity_seed")
        self.assertEqual((by_key["duration_seed"]["current"], by_key["duration_seed"]["target"]), (30, 25))
        self.assertIsNone(by_key["duration_seed"]["switch"])
        self.assertEqual((by_key["vtr_min"]["level"], by_key["vtr_min"]["target"]), (2, 100))
        self.assertTrue(by_key["category_diversity_seed"]["done"])

    def test_the_board_shows_every_level_of_each_challenge(self):
        held = [torrent(f"s{i}", size_gb=0.01) for i in range(160)]
        board, _ = run_board(FakeQBittorrent(held))
        steps = {c["key"]: c["steps"] for c in board["challenges"]}["quantity_seed"]
        self.assertEqual(
            [(s["level"], s["current"], s["target"], s["done"]) for s in steps],
            [(1, 160, 100, True), (2, 160, 150, True), (3, 160, 200, False), (4, 160, 250, False), (5, 160, 300, False)],
        )
        self.assertEqual(steps[2]["description"], "200 seeds")


class Scales(unittest.TestCase):
    def test_the_rare_bonus_follows_the_size_and_seeder_buckets_of_the_class(self):
        self.assertEqual(rare_coef(int(0.5 * GB), 5, SCALES), 0.2)
        self.assertEqual(rare_coef(3 * GB, 5, SCALES), 0.35)
        self.assertEqual(rare_coef(3 * GB, 30, SCALES), 0)
        self.assertEqual(rare_coef(20 * GB, 2, SCALES), 0.5)
        self.assertEqual(rare_coef(200 * GB, 2, SCALES), 0.5)

    def test_the_note_adds_the_seeding_time_and_the_rare_bonus_up_to_its_top(self):
        self.assertEqual(top_note(SCALES), 2.0)
        self.assertEqual(note_of(torrent("a", size_gb=20, seeders=2, seeding_days=200), SCALES), 2.0)
        self.assertEqual(note_of(torrent("b", size_gb=3, seeders=100, seeding_days=8), SCALES), 1.1)

    def test_the_scales_of_the_class_are_read_once_a_day(self):
        c411 = FakeC411()
        state = run(FakeQBittorrent(), c411=c411)
        self.assertEqual(state["scales"]["rare"], SCALES["rare"])
        run(FakeQBittorrent(), state=state, now=NOW + 600, c411=c411)
        self.assertEqual(c411.scale_reads, 1)
        run(FakeQBittorrent(), state=state, now=NOW + DAY, c411=c411)
        self.assertEqual(c411.scale_reads, 2)


class Categories(unittest.TestCase):
    def test_a_category_ratio_does_not_own_is_left_alone(self):
        qbt = FakeQBittorrent([torrent("manual", category="mine")])
        qbt.cats["mine"] = {}
        run(qbt)
        self.assertEqual(qbt.moved, [])
        self.assertIn("mine", qbt.cats)

    def test_the_c411_category_of_each_torrent_is_read_once(self):
        qbt = FakeQBittorrent([torrent("movie", category="movies"), torrent("book"), torrent("public", tracker=PUBLIC)])
        c411 = FakeC411(categories={"book": "ebook"})
        state = run(qbt, c411=c411)
        self.assertEqual(state["categories"], {"movie": "films-videos", "book": "ebook"})
        run(qbt, state=state, now=NOW + 600, c411=c411)
        self.assertEqual(sorted(c411.looked), ["book", "movie"])

    def test_a_torrent_gone_from_qbittorrent_loses_its_category(self):
        state = run(FakeQBittorrent(), state=empty_state(categories={"gone": "ebook"}))
        self.assertEqual(state["categories"], {})

    def test_the_categories_guessed_before_carry_over_except_emulation(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "state.json")
            save_state(path, {"kinds": {"a": "Ebook", "b": "Emulation", "c": "Films & Vidéos"}})
            self.assertEqual(load_state(path)["categories"], {"a": "ebook", "c": "films-videos"})


class Tiers(unittest.TestCase):
    def test_each_tier_tells_how_far_the_account_is(self):
        board, _ = run_board(FakeQBittorrent(), c411=FakeC411(uploaded_gb=45, downloaded_gb=30))
        self.assertEqual(
            [(t["name"], t["ratio"], t["gap_gb"]) for t in board["tiers"]],
            [("blocked", 0.8, -26.25), ("even", 1.0, -15), ("floor", 1.2, -7.5), ("goal", 2.0, 15)],
        )

    def test_the_download_left_before_the_block_counts_the_zen_as_upload(self):
        board, _ = run_board(FakeQBittorrent(), c411=FakeC411(uploaded_gb=40, downloaded_gb=30, zen=2616))
        self.assertEqual(board["account"]["zen_gb"], 2664)
        self.assertEqual(board["account"]["available_gb"], 3350)

    def test_crossing_a_tier_asks_farm_board_to_refresh(self):
        told = []
        state = empty_state()
        run_board(FakeQBittorrent(), state=state, c411=FakeC411(uploaded_gb=45, downloaded_gb=30), notify=told.append)
        run_board(FakeQBittorrent(), state=state, now=NOW + 600, c411=FakeC411(uploaded_gb=46, downloaded_gb=30), notify=told.append)
        self.assertEqual(told, [])
        run_board(FakeQBittorrent(), state=state, now=NOW + 1200, c411=FakeC411(uploaded_gb=34, downloaded_gb=30), notify=told.append)
        self.assertEqual(told, ["http://farm-board:8000/refresh"])


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
        self.assertEqual(sorted(qbt.tagged), [("free", "free"), ("movie", "library")])

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

    def test_the_clean_switch_turned_off_keeps_every_public_torrent(self):
        qbt = FakeQBittorrent([torrent("old", category="movies", tracker=PUBLIC, seeding_days=14)])
        run(qbt, switches={"clean_public": False})
        self.assertEqual(qbt.deleted, [])

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


class Adoption(unittest.TestCase):
    def test_the_library_stays_in_its_categories_by_default(self):
        qbt = FakeQBittorrent([torrent("movie", category="movies", seeders=80)])
        run(qbt)
        self.assertEqual(qbt.moved, [])

    def test_with_its_switch_an_imported_c411_torrent_of_the_library_moves_into_the_farm(self):
        qbt = FakeQBittorrent([torrent("movie", category="movies", seeders=80), torrent("album", category="music", seeders=3)])
        run(qbt, switches={"adopt": True})
        self.assertEqual(qbt.moved, [("movie", "farm"), ("album", "farm")])
        self.assertEqual(qbt.limited, ["movie", "album"])

    def test_a_public_fresh_or_too_big_library_torrent_stays(self):
        qbt = FakeQBittorrent([
            torrent("public", category="shows", seeders=3, tracker=PUBLIC),
            torrent("fresh", category="shows", seeders=3, seeding_days=0.1),
            torrent("huge", category="movies", size_gb=90),
            torrent("farmed", size_gb=20),
        ])
        run(qbt, switches={"adopt": True})
        self.assertEqual(qbt.moved, [])


class Ledger(unittest.TestCase):
    def test_only_c411_torrents_count_and_removed_ones_stay(self):
        qbt = FakeQBittorrent([
            torrent("c411", category="movies", downloaded_gb=6, uploaded_gb=1),
            torrent("public", tracker=PUBLIC, downloaded_gb=9),
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

    def test_a_freeleech_grab_is_reported_as_free(self):
        state = run(FakeQBittorrent(), c411=FakeC411([release("free", free=True)]))
        run(FakeQBittorrent([torrent("free", downloaded_gb=3, uploaded_gb=1)]), state=state, now=NOW + DAY)
        self.assertEqual(state["sources"], {"free": "free"})
        self.assertEqual(report_of(state, "free")["downloaded_gb"], 0)

    def test_the_c411_copy_of_a_twin_is_free_whoever_adds_it(self):
        state = run(FakeQBittorrent(), FakeProwlarr(twins=[twin()]), c411=FakeC411([episode()]))
        self.assertEqual(state["sources"], {"episode": "free"})
        qbt = FakeQBittorrent([torrent("public", tracker=PUBLIC), torrent("episode", state="stoppedUP", tracker="")])
        run(qbt, state=state, now=NOW + 600)
        self.assertEqual(state["twins"], {})
        run(FakeQBittorrent([torrent("episode")]), state=state, now=NOW + 1200)
        self.assertEqual(state["sources"], {"episode": "free"})

    def test_library_torrents_and_their_cross_seeds_are_told_apart(self):
        qbt = FakeQBittorrent([
            torrent("movie", category="movies", seeders=100),
            torrent("show", category="shows", seeders=100),
            torrent("linked"),
            torrent("public", category="movies", tracker=PUBLIC),
        ])
        state = run(qbt)
        self.assertEqual(state["sources"], {"movie": "library", "show": "library", "linked": "free"})

    def test_a_c411_torrent_without_a_source_gets_one_from_what_ratio_sees(self):
        qbt = FakeQBittorrent([
            torrent("linked", downloaded_gb=4),
            torrent("free", downloaded_gb=4),
            torrent("movie", category="movies", seeders=100),
        ])
        transfers = {h: [GB, 0] for h in ("linked", "free", "movie", "gone")}
        state = run(qbt, state=empty_state(transfers=transfers, free={"free": NOW - 100}))
        self.assertEqual(state["sources"], {"linked": "free", "free": "free", "movie": "library", "gone": "unknown"})

    def test_a_retired_hunt_becomes_a_quest(self):
        qbt = FakeQBittorrent([torrent("kept", tags="volume-hunt"), torrent("older", tags="zen-hunt")])
        sources = {"kept": "volume-hunt", "older": "zen-hunt"}
        state = run(qbt, state=empty_state(transfers={"kept": [GB, 0], "older": [GB, 0]}, sources=sources))
        self.assertEqual(state["sources"], {"kept": "quest", "older": "quest"})
        self.assertEqual([t["tags"] for t in qbt.list], ["quest", "quest"])

    def test_twin_freeleech_and_cross_seed_become_free(self):
        qbt = FakeQBittorrent([torrent("a", tags="twin"), torrent("b", tags="freeleech"), torrent("c", tags="cross-seed")])
        transfers = {h: [GB, 0] for h in "abc"}
        sources = {"a": "twin", "b": "freeleech", "c": "cross-seed"}
        state = run(qbt, state=empty_state(transfers=transfers, sources=sources))
        self.assertEqual(state["sources"], {"a": "free", "b": "free", "c": "free"})

    def test_a_pending_source_that_never_shows_up_is_forgotten(self):
        state = run(FakeQBittorrent(), state=empty_state(sources={"lost": "paid"}))
        self.assertEqual(state["sources"], {})


class Report(unittest.TestCase):
    def run_quietly(self, qbt, state, now):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            found = settings()
            run_once(qbt, FakeProwlarr(), FakeC411(), found, controls_of(found), state, now, lambda url: None)
        return [line for line in out.getvalue().splitlines() if line.startswith("Source ")]

    def test_the_report_is_logged_and_saved_once_a_day(self):
        qbt = FakeQBittorrent([torrent("movie", category="movies", seeders=100, downloaded_gb=4, uploaded_gb=2)])
        state = empty_state()
        lines = self.run_quietly(qbt, state, NOW)
        self.assertEqual(len(lines), 1)
        self.assertIn("Source library:", lines[0])
        self.assertEqual(report_of(state, "library")["ratio"], 0.5)
        self.assertEqual(self.run_quietly(qbt, state, NOW + 600), [])
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
        self.assertNotIn("gone", state["history"])

    def test_days_in_the_farm_count_from_the_move(self):
        state = empty_state(sources={"kept": "library"})
        self.run_quietly(FakeQBittorrent([torrent("kept", category="movies", size_gb=200, seeding_days=10)]), state, NOW)
        self.run_quietly(FakeQBittorrent([torrent("kept", seeding_days=13)]), state, NOW + 3 * DAY)
        self.run_quietly(FakeQBittorrent([torrent("kept", seeding_days=16)]), state, NOW + 6 * DAY)
        self.assertEqual(report_of(state, "library")["farm_days"], 3)


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
        entry = state["daily"][1]
        self.assertEqual(
            {k: entry[k] for k in ("ratio", "floor", "left_gb", "uploaded_gb", "actual_gb", "downloaded_gb", "zen", "seeds")},
            {"ratio": 2.0, "floor": 1.2, "left_gb": 20.0, "uploaded_gb": 60, "actual_gb": 5, "downloaded_gb": 30,
             "zen": 22, "seeds": 1},
        )

    def test_the_download_of_the_last_24_hours_counts_from_the_sample_a_day_old(self):
        state = empty_state()
        run(FakeQBittorrent(), state=state, c411=FakeC411(downloaded_gb=30))
        run(FakeQBittorrent(), state=state, now=NOW + DAY // 2, c411=FakeC411(downloaded_gb=31))
        self.assertEqual(state["daily"][-1]["downloaded_day_gb"], 1)
        run(FakeQBittorrent(), state=state, now=NOW + DAY + 600, c411=FakeC411(downloaded_gb=34))
        self.assertEqual(state["daily"][-1]["downloaded_day_gb"], 4)

    def test_the_zen_paid_each_night_is_what_the_zen_earned_grew_by(self):
        state = empty_state()
        run(FakeQBittorrent(), state=state, c411=FakeC411(zen=20, earned=30))
        run(FakeQBittorrent(), state=state, now=NOW + DAY, c411=FakeC411(zen=12, earned=34))
        self.assertEqual([e["zen_paid"] for e in state["daily"]], [None, 4])

    def test_an_unreadable_account_keeps_the_figures_the_day_already_had(self):
        state = empty_state()
        run(FakeQBittorrent(), state=state, c411=FakeC411(zen=20))
        run(FakeQBittorrent(), state=state, now=NOW + 600, c411=FakeC411(broken=True))
        self.assertEqual(state["daily"][0]["zen"], 20)
        self.assertEqual(state["daily"][0]["ratio"], 2.0)

    def test_the_days_before_the_first_run_come_from_the_account_readings(self):
        state = empty_state(account=[[NOW - 3 * DAY, 50 * GB, 25 * GB, 10], [NOW - 2 * DAY, 54 * GB, 27 * GB, 14]])
        run(FakeQBittorrent(), state=state)
        self.assertEqual([e["day"] for e in state["daily"]], [day(NOW - 3 * DAY), day(NOW - 2 * DAY), day(NOW)])

    def test_the_history_survives_a_restart_and_keeps_the_last_30_days(self):
        state = empty_state()
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "state.json")
            for n in range(40):
                run(FakeQBittorrent(), state=state, now=NOW + n * DAY)
                save_state(path, state)
                state = load_state(path)
        days = [e["day"] for e in state["daily"]]
        self.assertEqual(len(days), 30)
        self.assertEqual(days[-1], day(NOW + 39 * DAY))


class Board(unittest.TestCase):
    def test_the_farm_disk_is_split_by_source(self):
        qbt = FakeQBittorrent([
            torrent("twin1", size_gb=2), torrent("twin2", size_gb=1), torrent("hunt", size_gb=0.5),
            torrent("public", size_gb=4, tracker=PUBLIC, seeding_days=1),
            torrent("movie", category="movies", size_gb=200),
        ])
        state = empty_state(sources={"twin1": "free", "twin2": "free", "hunt": "quest"})
        board, _ = run_board(qbt, state=state)
        self.assertEqual(board["farm"], {
            "budget_gb": 100,
            "slices": [{"source": "public", "gb": 4}, {"source": "free", "gb": 3}, {"source": "quest", "gb": 0.5}],
        })

    def test_the_board_carries_the_account_the_seeds_and_the_controls(self):
        board, _ = run_board(FakeQBittorrent([torrent("a"), torrent("b", progress=0.5)]))
        account = board["account"]
        self.assertEqual((account["ratio"], account["zen"], account["credit_gb"], account["grade"]), (2.0, 25, 50, ["bronze", 3]))
        self.assertEqual(account["voies"], {"seed": 3, "specialization": 6})
        self.assertEqual(board["seeds"], 1)
        self.assertEqual(board["controls"]["numbers"]["floor"], 1.2)
        self.assertEqual([c["slug"] for c in board["categories"]][:3], ["films-videos", "ebook", "audio"])

    def test_the_board_sums_the_ledger_by_source_most_uploaded_first(self):
        qbt = FakeQBittorrent([
            torrent("paid", downloaded_gb=4, uploaded_gb=2),
            torrent("free", downloaded_gb=0, uploaded_gb=5),
            torrent("library", category="movies", seeders=100, downloaded_gb=10, uploaded_gb=1),
        ])
        state = empty_state(sources={"paid": "paid", "free": "free", "library": "library"})
        board, _ = run_board(qbt, state=state)
        self.assertEqual([s["source"] for s in board["sources"]], ["free", "paid", "library"])


class ControlsStore(unittest.TestCase):
    def test_controls_start_from_the_defaults_and_env_and_survive_a_restart(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "controls.json")
            controls = Controls(path)
            snapshot = controls.snapshot(SETTINGS)
            self.assertTrue(snapshot["switches"]["free"])
            self.assertFalse(snapshot["switches"]["paid"])
            self.assertTrue(snapshot["switches"]["volume:ebook"])
            self.assertFalse(snapshot["switches"]["volume:films-videos"])
            self.assertEqual(snapshot["numbers"], {"farm_size_gb": 100, "floor": 1.2, "daily_gb": 2, "max_size_gb": 1})
            controls.change("paid", True)
            controls.change("floor", 1.5)
            again = Controls(path).snapshot(SETTINGS)
            self.assertTrue(again["switches"]["paid"])
            self.assertEqual(again["numbers"]["floor"], 1.5)

    def test_a_wrong_control_is_refused(self):
        with tempfile.TemporaryDirectory() as folder:
            controls = Controls(os.path.join(folder, "controls.json"))
            for name, value in (("paid", "yes"), ("floor", 9), ("floor", True), ("nothing", True)):
                with self.assertRaises(ValueError):
                    controls.change(name, value)


class Server(unittest.TestCase):
    def test_the_board_is_served_and_a_control_changes_then_wakes_the_farm(self):
        with tempfile.TemporaryDirectory() as folder:
            controls = Controls(os.path.join(folder, "controls.json"))
            wake = threading.Event()
            latest = {"board": {"seeds": 3, "controls": controls.snapshot(SETTINGS)}}
            server = serve(latest, controls, SETTINGS, wake, port=0)
            try:
                url = f"http://127.0.0.1:{server.server_port}"
                with urllib.request.urlopen(f"{url}/board") as response:
                    self.assertEqual(json.load(response)["seeds"], 3)
                request = urllib.request.Request(f"{url}/controls", data=json.dumps({"name": "paid", "value": True}).encode(), method="POST")
                with urllib.request.urlopen(request) as response:
                    self.assertTrue(json.load(response)["switches"]["paid"])
                self.assertTrue(wake.is_set())
                self.assertTrue(latest["board"]["controls"]["switches"]["paid"])
                wrong = urllib.request.Request(f"{url}/controls", data=json.dumps({"name": "paid", "value": 3}).encode(), method="POST")
                with self.assertRaises(urllib.error.HTTPError) as error:
                    urllib.request.urlopen(wrong)
                self.assertEqual(error.exception.code, 400)
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
        self.assertFalse(qbt.prefs["queueing_enabled"])
        self.assertTrue(qbt.prefs["autorun_enabled"])
        self.assertIn("X-Api-Key: secret", qbt.prefs["autorun_program"])


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


class Torznab(unittest.TestCase):
    def test_items_without_a_date_are_skipped(self):
        releases = parse_torznab(FEED, 1793743200)
        self.assertEqual([r["infohash"] for r in releases], ["abc"])
        self.assertEqual(releases[0]["leechers"], 10)
        self.assertAlmostEqual(releases[0]["age_hours"], 1.0)


class Catalog(unittest.TestCase):
    def test_a_c411_item_becomes_a_release(self):
        item = {
            "infoHash": "ABC", "name": "Hop.Frog.EPUB", "size": 27100, "seeders": 251, "leechers": 2,
            "createdAt": "2026-11-09T09:53:21Z", "isFreeleech": True, "category": {"id": 2, "slug": "ebook"},
        }
        now = datetime.datetime(2026, 11, 9, 11, 53, 21, tzinfo=datetime.UTC).timestamp()
        self.assertEqual(release_of(item, now), {
            "infohash": "abc", "title": "Hop.Frog.EPUB", "page": "", "link": "", "size": 27100, "seeders": 251,
            "leechers": 2, "age_hours": 2.0, "free": True, "category": "ebook",
        })

    def test_scales_turn_the_buckets_of_the_class_into_ranges(self):
        self.assertEqual(SCALES["duration"], [[7, 0.1], [30, 0.25], [90, 0.35], [180, 0.5]])
        self.assertIn([10, 50, 1, 10, 0.5], SCALES["rare"])
        self.assertIn([100, float("inf"), 1, 10, 0.5], SCALES["rare"])
        self.assertIn([0, 1, 11, 24, 0.2], SCALES["rare"])


class TorrentFiles(unittest.TestCase):
    def fetched_url(self, release):
        response = mock.MagicMock()
        response.__enter__.return_value.read.return_value = b"torrent"
        with mock.patch("urllib.request.urlopen", return_value=response) as urlopen:
            Prowlarr("http://prowlarr:9696", "key").torrent(release)
        return urlopen.call_args.args[0].full_url

    def test_a_nyaa_torrent_downloads_straight_from_nyaa(self):
        release = {"page": "https://nyaa.si/view/1234567", "link": "http://prowlarr:9696/9/download?apikey=key"}
        self.assertEqual(self.fetched_url(release), "https://nyaa.si/download/1234567.torrent")


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
        self.qbt.add(b"d8:announce", "farm")
        path, content_type, body = FakeServer.calls[-1]
        self.assertEqual(path, "/api/v2/torrents/add")
        self.assertTrue(content_type.startswith("multipart/form-data; boundary="))
        self.assertIn(b'name="category"\r\n\r\nfarm\r\n', body)

    def test_a_c411_copy_is_added_stopped_in_the_twin_folder(self):
        self.qbt.add(b"d8:announce", "farm", "/data/torrents/farm")
        _, _, body = FakeServer.calls[-1]
        self.assertIn(b'name="savepath"\r\n\r\n/data/torrents/farm\r\n', body)
        self.assertIn(b'name="stopped"\r\n\r\ntrue\r\n', body)

    def test_the_c411_copy_takes_the_twin_file_names_then_checks_them(self):
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
        self.prowlarr = Prowlarr(f"http://127.0.0.1:{self.server.server_port}", "key", spacing=0)

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

    def test_a_429_raises_with_the_wait_prowlarr_asks_for(self):
        with self.assertRaises(Throttled) as throttled:
            self.prowlarr.search("3", "x", NOW)
        self.assertEqual(throttled.exception.retry_after, 1200)
        self.assertFalse(throttled.exception.disabled)

    def test_a_429_of_an_indexer_disabled_after_failures_says_so(self):
        with self.assertRaises(Throttled) as throttled:
            self.prowlarr.search("5", "x", NOW)
        self.assertTrue(throttled.exception.disabled)


PROFILE = {
    "class": {
        "slug": "swarm_keeper",
        "progress": {"questsCompleted": 9, "questsTotal": 40, "grade": {"tier": "bronze", "level": 3}},
        "voies": {"seed": {"questsCompleted": 3}, "specialization": {"questsCompleted": 6}},
    },
    "zen": {"balance": 616, "totalEarned": 416, "totalSpent": 0},
    "transfer": {
        "uploaded": {"actual": 19915312011, "credit": 64424509440, "total": 84339821451},
        "downloaded": {"actual": 70064908300, "credit": 0, "total": 70064908300},
    },
    "download": {"allowed": True, "blockedReason": None},
}
CATALOG = {
    "data": [{"infoHash": "AB", "name": "A", "size": 5, "seeders": 2, "leechers": 0, "createdAt": "2026-11-09T09:53:21Z",
              "isFreeleech": False, "category": {"slug": "ebook"}}],
    "meta": {"totalPages": 7},
}


class FakeC411Server(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        FakeC411Server.calls.append((self.path, self.headers.get("Authorization")))
        if self.path.startswith("/api/user/profile"):
            body = json.dumps(PROFILE).encode()
        elif self.path.startswith("/api?t=get&id=ab"):
            body = b"d4:infod6:lengthi1e4:name1:aee"
        elif self.path.startswith("/api/torrents/ab"):
            body = json.dumps({"infoHash": "AB", "category": {"slug": "audio"}}).encode()
        elif self.path.startswith("/api/torrents"):
            body = json.dumps(CATALOG).encode()
        else:
            body = json.dumps({"success": True, "classes": [{**SWARM_KEEPER, "canonicalSlug": "swarm_keeper"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class RealC411(unittest.TestCase):
    def setUp(self):
        FakeC411Server.calls = []
        self.server = http.server.HTTPServer(("127.0.0.1", 0), FakeC411Server)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.c411 = C411("profile", "torznab", f"http://127.0.0.1:{self.server.server_port}", spacing=0)

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()

    def test_the_account_is_read_from_the_profile_with_the_profile_key(self):
        account = self.c411.account()
        self.assertEqual(account, {
            "uploaded": 84339821451, "downloaded": 70064908300, "actual": 19915312011, "credit": 64424509440,
            "zen": 616, "earned": 416, "class": "swarm_keeper", "grade": ["bronze", 3],
            "voies": {"seed": 3, "specialization": 6}, "allowed": True,
        })
        self.assertEqual(FakeC411Server.calls, [("/api/user/profile", "Bearer profile")])

    def test_the_catalog_asks_for_a_category_sorted_by_size(self):
        releases, pages = self.c411.catalog(NOW, "ebook", "size", "asc", 3)
        self.assertEqual(([r["infohash"] for r in releases], pages), (["ab"], 7))
        self.assertEqual(
            FakeC411Server.calls[0][0], "/api/torrents?sortBy=size&sortOrder=asc&perPage=50&page=3&category=2",
        )

    def test_a_torrent_tells_its_category_and_downloads_with_the_torznab_key(self):
        self.assertEqual(self.c411.category("ab"), "audio")
        self.assertTrue(self.c411.download("ab").startswith(b"d4:info"))
        self.assertEqual(FakeC411Server.calls[1], ("/api?t=get&id=ab&apikey=torznab", "Bearer torznab"))

    def test_the_scales_of_the_class_are_read(self):
        self.assertEqual(self.c411.scales("swarm_keeper")["rare"], SCALES["rare"])

    def test_a_run_spends_no_more_calls_than_allowed(self):
        self.c411.allow(1)
        self.c411.account()
        with self.assertRaises(OutOfCalls):
            self.c411.account()
        self.assertEqual(len(FakeC411Server.calls), 1)


if __name__ == "__main__":
    unittest.main()
