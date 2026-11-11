import datetime
import email.utils
import http.client
import http.cookiejar
import http.server
import json
import logging
import os
import re
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import xml.etree.ElementTree as ET

GB = 1024**3
DAY = 86400
WEEK = 7 * DAY
FARM = "farm"
CATEGORIES = {
    "movies": "/data/torrents/movies",
    "shows": "/data/torrents/shows",
    "music": "/data/torrents/music",
    "books": "/data/torrents/books",
    FARM: "/data/torrents/farm",
}
PUBLIC = "public"
SEEDING = ("uploading", "stalledUP", "queuedUP", "forcedUP")
SHARE_LIMITS = ("max_ratio_enabled", "max_seeding_time_enabled", "max_inactive_seeding_time_enabled")
HISTORY_DAYS = 30
SOURCES = ("paid", "free", "library", "quest", "unknown")
RETIRED_SOURCES = {
    "zen-hunt": "quest", "volume-hunt": "quest", "twin": "free", "freeleech": "free", "cross-seed": "free",
}
C411_URL = "https://c411.org"
C411_CATEGORIES = {
    "films-videos": (1, "Films & Vidéos"),
    "ebook": (2, "Ebook"),
    "audio": (3, "Audio"),
    "applications": (4, "Applications"),
    "jeux-video": (5, "Jeux Vidéo"),
    "emulation": (6, "Emulation"),
    "gps": (7, "GPS"),
    "xxx": (8, "XXX"),
    "nulled": (9, "Nulled"),
    "imprimante-3d": (10, "Imprimante 3D"),
}
RETIRED_KINDS = {"Films & Vidéos": "films-videos", "Ebook": "ebook", "Audio": "audio", "Imprimante 3D": "imprimante-3d"}
SPECIALIZATION = "films-videos"
BADGE_SEEDS = 50
DEFAULT_SCALES = {"duration": [[7, 0.1], [30, 0.25], [90, 0.35], [180, 0.5]], "rare": []}
TIERS = (("blocked", 0.8), ("even", 1.0), ("floor", None), ("goal", 2.0))


def quest(id, voie, challenge, level, description, **criteria):
    return {"id": id, "voie": voie, "challenge": challenge, "level": level, "description": description, "criteria": criteria}


QUESTS = (
    quest(42, "seed", "quantity_seed", 1, "100 seeds", threshold=100),
    quest(43, "seed", "quantity_seed", 2, "150 seeds", threshold=150),
    quest(44, "seed", "quantity_seed", 3, "200 seeds", threshold=200),
    quest(45, "seed", "quantity_seed", 4, "250 seeds", threshold=250),
    quest(46, "seed", "quantity_seed", 5, "300 seeds", threshold=300),
    quest(47, "seed", "quantity_seed", 99, "Tous niveaux quantity_seed validés", all_levels=True),
    quest(48, "seed", "vtr_min", 1, "50 seeds à 62,5 % de leur note maximale", min_seeds=50, min_share_pct=62.5),
    quest(49, "seed", "vtr_min", 2, "100 seeds à 75 % de leur note maximale", min_seeds=100, min_share_pct=75),
    quest(50, "seed", "vtr_min", 3, "150 seeds à leur note maximale", min_seeds=150, min_share_pct=100),
    quest(51, "seed", "vtr_min", 4, "200 seeds à leur note maximale", min_seeds=200, min_share_pct=100),
    quest(52, "seed", "vtr_min", 5, "250 seeds à leur note maximale", min_seeds=250, min_share_pct=100),
    quest(53, "seed", "vtr_min", 6, "300 seeds à leur note maximale", min_seeds=300, min_share_pct=100),
    quest(54, "seed", "vtr_min", 99, "Tous les niveaux de ce défi validés", all_levels=True),
    quest(55, "seed", "duration_seed", 1, "25 seeds pendant 7 jours", min_seeds=25, min_days=7),
    quest(56, "seed", "duration_seed", 2, "100 seeds pendant 30 jours", min_seeds=100, min_days=30),
    quest(57, "seed", "duration_seed", 3, "150 seeds pendant 90 jours", min_seeds=150, min_days=90),
    quest(58, "seed", "duration_seed", 4, "250 seeds pendant 180 jours", min_seeds=250, min_days=180),
    quest(59, "seed", "duration_seed", 5, "300 seeds pendant 365 jours", min_seeds=300, min_days=365),
    quest(60, "seed", "duration_seed", 99, "Tous niveaux duration_seed validés", all_levels=True),
    quest(67, "specialization", "category_diversity_seed", 1, "25 seeds dans 1 catégorie", min_categories=1, min_seeds_per_cat=25),
    quest(68, "specialization", "category_diversity_seed", 2, "25 seeds dans 2 catégories", min_categories=2, min_seeds_per_cat=25),
    quest(69, "specialization", "category_diversity_seed", 3, "25 seeds dans 3 catégories", min_categories=3, min_seeds_per_cat=25),
    quest(70, "specialization", "category_diversity_seed", 4, "25 seeds dans 4 catégories", min_categories=4, min_seeds_per_cat=25),
    quest(71, "specialization", "category_diversity_seed", 5, "25 seeds dans 5 catégories", min_categories=5, min_seeds_per_cat=25),
    quest(72, "specialization", "category_volume_seed", 1, "50 seeds dans la même catégorie", min_seeds_same_cat=50),
    quest(73, "specialization", "category_volume_seed", 2, "100 seeds dans la même catégorie", min_seeds_same_cat=100),
    quest(74, "specialization", "category_volume_seed", 3, "250 seeds dans la spécialisation", min_seeds_same_cat=250, in_specialization=True),
    quest(75, "specialization", "category_volume_seed", 4, "500 seeds dans la spécialisation", min_seeds_same_cat=500, in_specialization=True),
)
DONE_AT_START = (42, 43, 48, 67, 68, 69, 70, 71, 72)
CHALLENGES = ("quantity_seed", "category_volume_seed", "category_diversity_seed", "vtr_min", "duration_seed")
VOIES = ("seed", "specialization")
SWITCHES = {
    "free": True,
    "twins": True,
    "paid": False,
    "quantity_seed": True,
    "clean_public": True,
    "drop_twins": True,
    "hold": True,
    "adopt": False,
    **{f"volume:{slug}": slug in ("ebook", "audio", "jeux-video", "imprimante-3d") for slug in C411_CATEGORIES},
}
NUMBERS = {
    "farm_size_gb": ("farm_size", GB, 1, 100000),
    "floor": ("floor", 1, 0.5, 5),
    "daily_gb": ("daily", GB, 0, 1000),
    "max_size_gb": ("max_size", GB, 0.01, 1000),
}
TWIN_ATTEMPTS = 3
TORZNAB = "{http://torznab.com/schemas/2015/feed}attr"

FAILURES = (OSError, ValueError, KeyError, TypeError, RuntimeError, http.client.HTTPException, ET.ParseError)

LOG = logging.getLogger("ratio")


class QBittorrent:
    def __init__(self, url, username, password):
        self.url = url.rstrip("/")
        self.username = username
        self.password = password
        self.opener = None

    def torrents(self):
        return json.loads(self.send("/api/v2/torrents/info"))

    def add(self, torrent, category, save_path=None):
        boundary = uuid.uuid4().hex
        fields = {"category": category}
        if save_path:
            fields.update({"savepath": save_path, "autoTMM": "false", "stopped": "true"})
        body = b"".join(
            [
                *(
                    f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode()
                    for name, value in fields.items()
                ),
                f'--{boundary}\r\nContent-Disposition: form-data; name="torrents"; filename="grab.torrent"\r\n'
                "Content-Type: application/x-bittorrent\r\n\r\n".encode()
                + torrent
                + b"\r\n",
                f"--{boundary}--\r\n".encode(),
            ]
        )
        answer = self.send("/api/v2/torrents/add", body, f"multipart/form-data; boundary={boundary}")
        if answer.decode(errors="replace").strip() == "Fails.":
            raise RuntimeError("qBittorrent refused the torrent file")

    def delete(self, hashes, delete_files=True):
        self.post("/api/v2/torrents/delete", {"hashes": "|".join(hashes), "deleteFiles": str(delete_files).lower()})

    def files(self, infohash):
        return json.loads(self.send(f"/api/v2/torrents/files?hash={infohash}"))

    def match_files(self, infohash, wanted):
        files = []
        for _ in range(10):
            files = self.files(infohash)
            if files:
                break
            time.sleep(1)
        if not files:
            raise RuntimeError("qBittorrent has not read the torrent file yet")
        names = {}
        for f in wanted:
            names.setdefault(f["size"], []).append(f["name"])
        for f in files:
            name = names[f["size"]].pop(0)
            if f["name"] != name:
                self.post("/api/v2/torrents/renameFile", {"hash": infohash, "oldPath": f["name"], "newPath": name})
        self.post("/api/v2/torrents/recheck", {"hashes": infohash})

    def start(self, hashes):
        self.post("/api/v2/torrents/start", {"hashes": "|".join(hashes)})

    def set_category(self, hashes, category):
        self.post("/api/v2/torrents/setCategory", {"hashes": "|".join(hashes), "category": category})

    def categories(self):
        return json.loads(self.send("/api/v2/torrents/categories"))

    def create_category(self, name, path):
        self.post("/api/v2/torrents/createCategory", {"category": name, "savePath": path})

    def add_tags(self, hashes, tag):
        self.post("/api/v2/torrents/addTags", {"hashes": "|".join(hashes), "tags": tag})

    def remove_tags(self, hashes, tag):
        self.post("/api/v2/torrents/removeTags", {"hashes": "|".join(hashes), "tags": tag})

    def keep_in_place(self, hashes):
        self.post("/api/v2/torrents/setAutoManagement", {"hashes": "|".join(hashes), "enable": "false"})

    def reset_share_limits(self, hashes):
        self.post("/api/v2/torrents/setShareLimits", {
            "hashes": "|".join(hashes), "ratioLimit": -2, "seedingTimeLimit": -2, "inactiveSeedingTimeLimit": -2,
            "shareLimitAction": "Default", "shareLimitsMode": "Default",
        })

    def preferences(self):
        return json.loads(self.send("/api/v2/app/preferences"))

    def set_preferences(self, preferences):
        self.post("/api/v2/app/setPreferences", {"json": json.dumps(preferences)})

    def post(self, path, fields):
        return self.send(path, urllib.parse.urlencode(fields).encode(), "application/x-www-form-urlencoded")

    def send(self, path, body=None, content_type=None):
        if self.opener is None:
            self.login()
        try:
            return self.open(path, body, content_type)
        except urllib.error.HTTPError as error:
            if error.code != 403:
                raise
        self.login()
        return self.open(path, body, content_type)

    def open(self, path, body, content_type):
        headers = {"Content-Type": content_type} if content_type else {}
        request = urllib.request.Request(f"{self.url}{path}", data=body, headers=headers)
        with self.opener.open(request, timeout=30) as response:
            return response.read()

    def login(self):
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
        body = urllib.parse.urlencode({"username": self.username, "password": self.password}).encode()
        try:
            with self.opener.open(f"{self.url}/api/v2/auth/login", data=body, timeout=30) as response:
                accepted = response.status == 204 or response.read().decode().strip() == "Ok."
        except urllib.error.HTTPError as error:
            if error.code not in (401, 403):
                raise
            accepted = False
        if not accepted:
            self.opener = None
            raise RuntimeError("qBittorrent refused the login, check QBITTORRENT_USER and QBITTORRENT_PASSWORD")


class Throttled(Exception):
    def __init__(self, retry_after, disabled=False):
        super().__init__(f"Prowlarr asks to wait {retry_after} seconds")
        self.retry_after = retry_after
        self.disabled = disabled


class OutOfCalls(Exception):
    pass


def wait_of(header):
    return max(int(header), 60) if header and header.strip().isdigit() else 3600


class Prowlarr:
    def __init__(self, url, api_key, spacing=2):
        self.url = url.rstrip("/")
        self.api_key = api_key
        self.spacing = spacing
        self.last = 0

    def search(self, indexer, title, now):
        request = urllib.request.Request(
            f"{self.url}/{indexer}/api?{urllib.parse.urlencode({'t': 'search', 'q': re.sub(r'[._]+', ' ', title), 'limit': 100})}",
            headers={"X-Api-Key": self.api_key},
        )
        return parse_torznab(self.open(request), now)

    def torrent(self, release):
        page = re.search(r"nyaa\.si/view/(\d+)", release["page"])
        link = f"https://nyaa.si/download/{page.group(1)}.torrent" if page else release["link"]
        return self.open(urllib.request.Request(link, headers={"User-Agent": "Mozilla/5.0 (compatible; jellynas-ratio)"}))

    def hold(self, indexer, paused, profile_name, restore=None):
        profiles = self.api("/appprofile")
        held = next((p for p in profiles if p["name"] == profile_name), None)
        if held is None:
            if not paused:
                return None
            held = self.api("/appprofile", "POST", {
                "name": profile_name,
                "enableRss": False,
                "enableAutomaticSearch": False,
                "enableInteractiveSearch": False,
                "minimumSeeders": 1,
            })
        definition = self.api(f"/indexer/{indexer}")
        before = definition["appProfileId"]
        if paused == (before == held["id"]):
            return None
        if paused:
            definition["appProfileId"] = held["id"]
        else:
            others = [p["id"] for p in profiles if p["id"] != held["id"]]
            if not others:
                raise RuntimeError(f"Prowlarr has no sync profile other than {profile_name}")
            definition["appProfileId"] = restore if restore in others else others[0]
        self.api(f"/indexer/{indexer}", "PUT", definition)
        return before

    def api(self, path, method="GET", body=None):
        request = urllib.request.Request(
            f"{self.url}/api/v1{path}",
            data=None if body is None else json.dumps(body).encode(),
            headers={"X-Api-Key": self.api_key, "Content-Type": "application/json"},
            method=method,
        )
        return json.loads(self.open(request))

    def open(self, request):
        if not request.full_url.startswith(self.url):
            with urllib.request.urlopen(request, timeout=60) as response:
                return response.read()
        wait = self.last + self.spacing - time.monotonic()
        if wait > 0:
            time.sleep(wait)
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return response.read()
        except urllib.error.HTTPError as error:
            if error.code == 429:
                try:
                    disabled = b"disabled" in error.read()
                except OSError:
                    disabled = False
                raise Throttled(wait_of(error.headers.get("Retry-After")), disabled) from error
            raise
        finally:
            self.last = time.monotonic()


def iso_time(text):
    return datetime.datetime.fromisoformat(text).timestamp()


def release_of(item, now):
    category = item.get("category") or {}
    return {
        "infohash": item["infoHash"].lower(),
        "title": item["name"],
        "page": "",
        "link": "",
        "size": int(item["size"]),
        "seeders": int(item.get("seeders") or 0),
        "leechers": int(item.get("leechers") or 0),
        "age_hours": (now - iso_time(item["createdAt"])) / 3600,
        "free": bool(item.get("isFreeleech")),
        "category": category.get("slug", ""),
    }


def bucket_bounds(text):
    if text.startswith("<"):
        return float(text[1:])
    if text.startswith(">"):
        return float("inf")
    return float(text.split("-")[1])


def scales_of(definition):
    tiers = definition.get("bonusTiers") or {}
    duration = sorted([int(t["bucket"]), t["coef"]] for t in tiers.get("seed_duration", []))
    found = [(t["bucket"].split("|"), t["coef"]) for t in tiers.get("seeders_weight", []) if "|" in t["bucket"]]
    highs = sorted({bucket_bounds(size) for (size, _), _ in found})
    rare = []
    for (size, seeders), coef in found:
        high = bucket_bounds(size)
        low = max([h for h in highs if h < high], default=0)
        first, _, last = seeders.partition("-")
        rare.append([low, high, int(first), int(last or first), coef])
    return {"duration": duration or DEFAULT_SCALES["duration"], "rare": sorted(rare)}


class C411:
    def __init__(self, profile_key, torznab_key, url=C411_URL, spacing=6.0):
        self.profile_key = profile_key
        self.torznab_key = torznab_key
        self.url = url.rstrip("/")
        self.spacing = spacing
        self.last = 0
        self.calls_left = None

    def allow(self, calls):
        self.calls_left = calls

    def get(self, path, key, query=None):
        if not key:
            raise RuntimeError("the API key of C411 is empty")
        if self.calls_left is not None:
            if self.calls_left <= 0:
                raise OutOfCalls()
            self.calls_left -= 1
        wait = self.last + self.spacing - time.monotonic()
        if wait > 0:
            time.sleep(wait)
        address = f"{self.url}{path}" + (f"?{urllib.parse.urlencode(query)}" if query else "")
        request = urllib.request.Request(address, headers={"Authorization": f"Bearer {key}", "User-Agent": "jellynas-ratio"})
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return response.read()
        finally:
            self.last = time.monotonic()

    def json(self, path, key, query=None):
        return json.loads(self.get(path, key, query))

    def account(self):
        profile = self.json("/api/user/profile", self.profile_key)
        transfer = profile["transfer"]
        kind = profile.get("class") or {}
        grade = (kind.get("progress") or {}).get("grade") or {}
        return {
            "uploaded": transfer["uploaded"]["total"],
            "downloaded": transfer["downloaded"]["total"],
            "actual": transfer["uploaded"].get("actual"),
            "credit": transfer["uploaded"].get("credit"),
            "zen": profile["zen"]["balance"],
            "earned": profile["zen"].get("totalEarned"),
            "class": kind.get("slug"),
            "grade": [grade.get("tier"), grade.get("level")] if grade else None,
            "voies": {name: v.get("questsCompleted", 0) for name, v in (kind.get("voies") or {}).items()},
            "allowed": (profile.get("download") or {}).get("allowed", True),
        }

    def catalog(self, now, category=None, sort="createdAt", order="desc", page=1):
        query = {"sortBy": sort, "sortOrder": order, "perPage": 50, "page": page}
        if category:
            query["category"] = C411_CATEGORIES[category][0]
        answer = self.json("/api/torrents", self.profile_key, query)
        return [release_of(item, now) for item in answer.get("data", [])], (answer.get("meta") or {}).get("totalPages", 0)

    def category(self, infohash):
        answer = self.json(f"/api/torrents/{infohash}", self.profile_key)
        item = answer.get("data", answer) if isinstance(answer, dict) else {}
        return (item.get("category") or {}).get("slug", "")

    def download(self, infohash):
        return self.get(f"/api/torrents/{infohash}/download", self.torznab_key, {"apikey": self.torznab_key})

    def scales(self, slug):
        classes = self.json("/api/classes", self.profile_key).get("classes", [])
        found = next((c for c in classes if slug in (c.get("slug"), c.get("canonicalSlug"))), None)
        return scales_of(found) if found else dict(DEFAULT_SCALES)


def parse_torznab(xml_bytes, now):
    releases = []
    for item in ET.fromstring(xml_bytes).iter("item"):
        attrs = {a.get("name"): a.get("value") for a in item.iter(TORZNAB)}
        try:
            published = email.utils.parsedate_to_datetime(item.findtext("pubDate")).timestamp()
        except (TypeError, ValueError):
            continue
        if not attrs.get("infohash") or not item.findtext("link"):
            continue
        seeders = int(attrs.get("seeders") or 0)
        peers = int(attrs.get("peers") or 0)
        releases.append(
            {
                "infohash": attrs["infohash"].lower(),
                "title": item.findtext("title"),
                "link": item.findtext("link"),
                "page": item.findtext("comments") or item.findtext("guid") or "",
                "size": int(item.findtext("size") or 0),
                "seeders": seeders,
                "leechers": max(peers - seeders, 0),
                "age_hours": (now - published) / 3600,
                "free": attrs.get("downloadvolumefactor") == "0",
            }
        )
    return releases


def bdecode(data, start=0):
    kind = data[start:start + 1]
    if kind == b"i":
        end = data.index(b"e", start)
        return int(data[start + 1:end]), end + 1
    if kind in (b"l", b"d"):
        items, position = [], start + 1
        while data[position:position + 1] != b"e":
            item, position = bdecode(data, position)
            items.append(item)
        value = items if kind == b"l" else dict(zip(items[::2], items[1::2]))
        return value, position + 1
    colon = data.index(b":", start)
    length = int(data[start:colon])
    return data[colon + 1:colon + 1 + length], colon + 1 + length


def file_lengths(torrent):
    info = bdecode(torrent)[0][b"info"]
    if b"files" in info:
        return sorted(f[b"length"] for f in info[b"files"])
    return [info[b"length"]]


def rare_coef(size, seeders, scales):
    size_gb = size / GB
    return next(
        (coef for low, high, first, last, coef in scales["rare"] if low <= size_gb < high and first <= seeders <= last),
        0,
    )


def note_of(t, scales):
    days = t["seeding_time"] / DAY
    duration = max([coef for at, coef in scales["duration"] if days >= at], default=0)
    return 1 + duration + rare_coef(t["size"], t["num_complete"], scales)


def top_note(scales):
    return 1 + max([c for _, c in scales["duration"]], default=0) + max([r[4] for r in scales["rare"]], default=0)


def cost_of(release):
    return 0 if release["free"] else release["size"]


def pick_free(releases, known, max_size):
    free = [r for r in releases if r["free"] and r["infohash"] not in known and r["seeders"] >= 1 and r["size"] <= max_size]
    return sorted(free, key=lambda r: -r["leechers"])


def pick_small(releases, known, max_size, scales):
    small = [r for r in releases if r["infohash"] not in known and r["size"] <= max_size and r["seeders"] >= 1]
    return sorted(small, key=lambda r: (cost_of(r), -rare_coef(r["size"], r["seeders"], scales), r["size"]))


def pick_paid(releases, known, settings):
    fresh = [
        r
        for r in releases
        if r["infohash"] not in known
        and not r["free"]
        and r["seeders"] >= 1
        and r["age_hours"] <= settings["paid_max_age_hours"]
        and r["leechers"] >= settings["paid_min_leechers"]
        and r["size"] <= settings["free_max_size"]
    ]
    return sorted(fresh, key=lambda r: -r["leechers"] / max(r["size"], 1))


def size_of(torrents, category):
    return sum(t["size"] for t in torrents.values() if t["category"] == category)


def load_state(path):
    try:
        with open(path, encoding="utf-8") as file:
            state = json.load(file)
    except FileNotFoundError:
        state = {}
    kinds = state.get("kinds", {})
    return {
        "transfers": state.get("transfers", {}),
        "hunted": state.get("hunted", {}),
        "twins": state.get("twins", {}),
        "free": state.get("free", {}),
        "checked": state.get("checked", {}),
        "paused": state.get("paused", False),
        "sources": state.get("sources", {}),
        "history": state.get("history", {}),
        "zen": state.get("zen", {}),
        "report": state.get("report", {}),
        "account": state.get("account", []),
        "received": state.get("received", []),
        "held": state.get("held"),
        "backoff_until": state.get("backoff_until", 0),
        "stuck": state.get("stuck", {}),
        "given_up": state.get("given_up", {}),
        "restore": state.get("restore", {}),
        "categories": state.get("categories") or {h: RETIRED_KINDS[k] for h, k in kinds.items() if k in RETIRED_KINDS},
        "pages": state.get("pages", {}),
        "daily": state.get("daily", []),
        "done": state.get("done") or list(DONE_AT_START),
        "scales": state.get("scales"),
        "tier": state.get("tier"),
    }


def save_state(path, state):
    temporary = f"{path}.tmp"
    with open(temporary, "w", encoding="utf-8") as file:
        json.dump(state, file)
    os.replace(temporary, path)


class Controls:
    def __init__(self, path):
        self.path = path
        self.lock = threading.Lock()
        try:
            with open(path, encoding="utf-8") as file:
                self.saved = json.load(file)
        except (FileNotFoundError, ValueError):
            self.saved = {}

    def snapshot(self, settings):
        with self.lock:
            switches = {name: bool(self.saved.get("switches", {}).get(name, on)) for name, on in SWITCHES.items()}
            numbers = {}
            for name, (key, unit, _, _) in NUMBERS.items():
                numbers[name] = self.saved.get("numbers", {}).get(name, round(settings[key] / unit, 2))
            return {"switches": switches, "numbers": numbers}

    def change(self, name, value):
        with self.lock:
            if name in SWITCHES:
                if not isinstance(value, bool):
                    raise ValueError(f"{name} takes true or false")
                self.saved.setdefault("switches", {})[name] = value
            elif name in NUMBERS:
                _, _, low, high = NUMBERS[name]
                if isinstance(value, bool) or not isinstance(value, (int, float)) or not low <= value <= high:
                    raise ValueError(f"{name} takes a number from {low:g} to {high:g}")
                self.saved.setdefault("numbers", {})[name] = value
            else:
                raise ValueError(f"{name} is not a control")
            temporary = f"{self.path}.tmp"
            with open(temporary, "w", encoding="utf-8") as file:
                json.dump(self.saved, file)
            os.replace(temporary, self.path)


def effective(settings, controls):
    found = dict(settings)
    for name, (key, unit, _, _) in NUMBERS.items():
        found[key] = controls["numbers"][name] * unit
    return found


def prepare(qbt, settings):
    categories = qbt.categories()
    for name, path in CATEGORIES.items():
        if name not in categories:
            qbt.create_category(name, path)
    preferences = qbt.preferences()
    limits = {key: False for key in SHARE_LIMITS if preferences.get(key)}
    if limits:
        qbt.set_preferences(limits)
        print("qBittorrent no longer stops torrents at its global share limits")
    if preferences.get("queueing_enabled"):
        qbt.set_preferences({"queueing_enabled": False})
        print("qBittorrent no longer queues torrents, so every torrent announces and seeds")
    if not settings["cross_seed_key"]:
        return
    hook = (
        "curl -XPOST http://cross-seed:2468/api/webhook "
        f'-H "X-Api-Key: {settings["cross_seed_key"]}" -d "infoHash=%I" -d "includeSingleEpisodes=true"'
    )
    if not preferences.get("autorun_enabled") or preferences.get("autorun_program") != hook:
        qbt.set_preferences({"autorun_enabled": True, "autorun_program": hook})
        print("qBittorrent now tells cross-seed about every finished torrent")


def is_c411(state, settings, t):
    return t["hash"] in state["transfers"] or settings["tracker"] in t["tracker"].lower()


def is_public(state, settings, t):
    return bool(t["tracker"]) and not t.get("private") and not is_c411(state, settings, t)


def is_owned(t):
    return t["category"] in CATEGORIES


def is_library(settings, category):
    return category in settings["library_categories"]


def is_seeding(t):
    return t["progress"] >= 1 and t["state"] in SEEDING


def twin_copies(state):
    return {entry[0] for entry in state["twins"].values()}


def source_of(state, settings, copies, t):
    if t is None:
        return "unknown"
    if t["hash"] in copies or t["hash"] in state["free"] or t["category"] == FARM:
        return "free"
    if is_library(settings, t["category"]):
        return "library"
    return "unknown"


def record_sources(state, torrents, settings):
    copies = twin_copies(state)
    for infohash, source in state["sources"].items():
        state["sources"][infohash] = RETIRED_SOURCES.get(source, source)
    for infohash in state["transfers"]:
        if infohash not in state["sources"]:
            state["sources"][infohash] = source_of(state, settings, copies, torrents.get(infohash))
    for infohash in list(state["sources"]):
        if infohash not in state["transfers"] and infohash not in torrents and infohash not in copies:
            del state["sources"][infohash]


def record_history(state, torrents, fresh, now):
    for infohash, (downloaded, uploaded) in state["transfers"].items():
        t = torrents.get(infohash)
        samples = state["history"].get(infohash, [])
        restarted = t is not None and samples and (downloaded < samples[-1][1] or uploaded < samples[-1][2])
        if infohash in fresh or restarted:
            state["history"][infohash] = [[t["added_on"], 0, 0]]
        samples = state["history"].setdefault(infohash, [])
        if t is not None and (not samples or now - samples[-1][0] >= DAY):
            samples.append([now, downloaded, uploaded])
        if t is None and samples and samples[-1][1:] != [downloaded, uploaded]:
            samples.append([now, downloaded, uploaded])
        samples[:] = [s for s in samples if s[0] >= now - WEEK]
        if not samples:
            del state["history"][infohash]


def record_zen(state, torrents):
    for infohash in state["transfers"]:
        t = torrents.get(infohash)
        if t is None:
            continue
        seeding = t["seeding_time"]
        entry = state["zen"].get(infohash)
        if entry is None:
            if t["category"] == FARM:
                state["zen"][infohash] = [seeding, 0]
            continue
        if t["category"] == FARM:
            entry[1] += seeding - entry[0] if seeding >= entry[0] else seeding
        entry[0] = seeding


def record(state, torrents, settings, now):
    fresh = set()
    for t in torrents.values():
        if t["hash"] in state["transfers"] or settings["tracker"] in t["tracker"].lower():
            if t["hash"] not in state["transfers"]:
                fresh.add(t["hash"])
            downloaded = 0 if t["hash"] in state["free"] else t["downloaded"]
            state["transfers"][t["hash"]] = [downloaded, t["uploaded"]]
    for infohash in [h for h, (at, _) in state["hunted"].items() if at < now - DAY]:
        del state["hunted"][infohash]
    for infohash in [h for h in state["free"] if h not in torrents and h in state["transfers"]]:
        del state["free"][infohash]
    for infohash in [h for h, at in state["given_up"].items() if at < now - WEEK]:
        del state["given_up"][infohash]
    for infohash in [h for h, at in state["checked"].items() if at < now - 2 * DAY]:
        del state["checked"][infohash]
    copies = twin_copies(state)
    for infohash in [h for h in state["categories"] if h not in torrents and h not in copies]:
        del state["categories"][infohash]
    record_sources(state, torrents, settings)
    record_history(state, torrents, fresh, now)
    record_zen(state, torrents)


def learn_categories(c411, state, settings, torrents):
    for t in torrents.values():
        if t["hash"] in state["categories"] or not is_c411(state, settings, t):
            continue
        try:
            state["categories"][t["hash"]] = c411.category(t["hash"])
        except urllib.error.HTTPError as error:
            if error.code != 404:
                print(f"Cannot read the C411 category of {t['name']}: {error}")
                return
            state["categories"][t["hash"]] = ""
        except FAILURES as error:
            print(f"Cannot read the C411 category of {t['name']}: {error}")
            return


def refresh_scales(c411, state, account, now):
    scales = state["scales"]
    slug = (account or {}).get("class")
    if not slug or (scales and scales.get("class") == slug and now - scales.get("at", 0) < DAY):
        return
    try:
        state["scales"] = {**c411.scales(slug), "class": slug, "at": now}
    except FAILURES as error:
        print(f"Cannot read the bonus scales of the C411 class {slug}: {error}")


def scales_in(state):
    found = state["scales"] or DEFAULT_SCALES
    return {"duration": found["duration"], "rare": found["rare"]}


def gigabytes(size):
    return round(size / GB, 2)


def share(uploaded, downloaded):
    return round(uploaded / downloaded, 2) if downloaded else None


def ledger_lines(state):
    for infohash, (downloaded, uploaded) in state["transfers"].items():
        samples = state["history"].get(infohash)
        _, downloaded_before, uploaded_before = samples[0] if samples else [0, downloaded, uploaded]
        zen = state["zen"].get(infohash, [0, 0])[1]
        source = state["sources"].get(infohash)
        source = source if source in SOURCES else "unknown"
        yield source, [1, downloaded, uploaded, downloaded - downloaded_before, uploaded - uploaded_before, zen]


def figures_by_source(state):
    totals = {}
    for source, line in ledger_lines(state):
        totals[source] = [a + b for a, b in zip(totals.get(source, [0] * len(line)), line)]
    figures = {}
    for source in [s for s in SOURCES if s in totals]:
        count, downloaded, uploaded, week_downloaded, week_uploaded, zen = totals[source]
        figures[source] = {
            "torrents": count,
            "downloaded_gb": gigabytes(downloaded),
            "uploaded_gb": gigabytes(uploaded),
            "ratio": share(uploaded, downloaded),
            "week_downloaded_gb": gigabytes(week_downloaded),
            "week_uploaded_gb": gigabytes(week_uploaded),
            "week_ratio": share(week_uploaded, week_downloaded),
            "farm_days": round(zen / DAY, 1),
        }
    return figures


def shown(ratio):
    return "-" if ratio is None else f"{ratio:.2f}"


def report(state, now):
    if now - state["report"].get("at", 0) < DAY:
        return
    figures = figures_by_source(state)
    state["report"] = {"at": now, "sources": figures}
    for source, f in figures.items():
        print(
            f"Source {source}: {f['torrents']} torrents, {f['downloaded_gb']:.1f} GB down, "
            f"{f['uploaded_gb']:.1f} GB up, ratio {shown(f['ratio'])}, last 7 days {f['week_downloaded_gb']:.1f} GB down, "
            f"{f['week_uploaded_gb']:.1f} GB up, ratio {shown(f['week_ratio'])}, {f['farm_days']:.1f} days in the Farm"
        )


def seeded_on_c411(state, settings, torrents):
    return [t for t in torrents.values() if is_c411(state, settings, t) and is_seeding(t)]


def category_counts(state, seeds):
    counts = dict.fromkeys(C411_CATEGORIES, 0)
    for t in seeds:
        slug = state["categories"].get(t["hash"])
        if slug in counts:
            counts[slug] += 1
    return counts


def levels_of(challenge):
    return [q for q in QUESTS if q["challenge"] == challenge and q["level"] < 99]


def progress_of(q, seeds, counts, scales, done):
    c = q["criteria"]
    if c.get("all_levels"):
        levels = levels_of(q["challenge"])
        return sum(1 for x in levels if x["id"] in done), len(levels)
    challenge = q["challenge"]
    if challenge == "quantity_seed":
        return len(seeds), c["threshold"]
    if challenge == "duration_seed":
        return sum(1 for t in seeds if t["seeding_time"] >= c["min_days"] * DAY), c["min_seeds"]
    if challenge == "vtr_min":
        top = top_note(scales)
        return sum(1 for t in seeds if note_of(t, scales) >= top * c["min_share_pct"] / 100 - 1e-9), c["min_seeds"]
    if challenge == "category_volume_seed":
        current = counts[SPECIALIZATION] if c.get("in_specialization") else max(counts.values())
        return current, c["min_seeds_same_cat"]
    return sum(1 for n in counts.values() if n >= c["min_seeds_per_cat"]), c["min_categories"]


def next_level(challenge, done):
    return next((q for q in levels_of(challenge) if q["id"] not in done), None)


def validate_quests(state, account, seeds, counts, scales):
    if account is None:
        return
    done = set(state["done"])
    for voie in VOIES:
        seen = account["voies"].get(voie)
        if seen is None:
            continue
        mine = [q for q in QUESTS if q["voie"] == voie]
        missing = seen - sum(1 for q in mine if q["id"] in done)
        for q in sorted((q for q in mine if q["id"] not in done), key=lambda q: (q["level"] == 99, CHALLENGES.index(q["challenge"]), q["level"])):
            if missing <= 0:
                break
            current, target = progress_of(q, seeds, counts, scales, done)
            if current >= target:
                done.add(q["id"])
                missing -= 1
                print(f"C411 validated the quest {q['description']}")
    state["done"] = sorted(done)


def category_target(slug, done):
    if slug != SPECIALIZATION:
        return BADGE_SEEDS
    level = next_level("category_volume_seed", done)
    return level["criteria"]["min_seeds_same_cat"] if level else BADGE_SEEDS


def wanted_hunts(switches, state, seeds, counts):
    done = set(state["done"])
    hunts = []
    for slug in C411_CATEGORIES:
        target = category_target(slug, done)
        if switches[f"volume:{slug}"] and counts[slug] < target:
            hunts.append((slug, target - counts[slug]))
    level = next_level("quantity_seed", done)
    if switches["quantity_seed"] and level and len(seeds) < level["criteria"]["threshold"]:
        hunts.append((None, level["criteria"]["threshold"] - len(seeds)))
    return hunts


def drop_twins(qbt, state, torrents):
    for twin, (original, *_) in list(state["twins"].items()):
        if twin not in torrents:
            del state["twins"][twin]
            continue
        seeding = torrents.get(original)
        if seeding is None or seeding["progress"] < 1:
            continue
        if seeding["state"].startswith("stopped"):
            qbt.start([original])
        qbt.delete([twin], seeding["save_path"] != torrents[twin]["save_path"])
        del torrents[twin]
        del state["twins"][twin]
        print(f"{seeding['name']} now seeds on C411, its public twin is gone")


def seed_twins(qbt, c411, settings, state, torrents, now):
    for twin in [t for t in state["stuck"] if t not in state["twins"]]:
        del state["stuck"][twin]
    for twin, (original, _, grabbed_at, _) in list(state["twins"].items()):
        public = torrents.get(twin)
        if original in torrents or public is None or public["progress"] < 1:
            continue
        if now - grabbed_at < settings["twin_seed_after"]:
            continue
        attempts, next_at = state["stuck"].get(twin, [0, 0])
        if now < next_at:
            continue
        added, refused = False, False
        try:
            try:
                torrent_file = c411.download(original)
            except urllib.error.HTTPError as error:
                refused = 400 <= error.code < 500 and error.code != 429
                raise
            qbt.add(torrent_file, FARM, public["save_path"])
            added = True
            qbt.match_files(original, qbt.files(twin))
        except OutOfCalls:
            raise
        except FAILURES as error:
            if refused or attempts + 1 >= TWIN_ATTEMPTS:
                if added:
                    qbt.delete([original], delete_files=False)
                del state["twins"][twin]
                state["stuck"].pop(twin, None)
                state["given_up"][original] = now
                print(f"Gave up the C411 copy of {public['name']}, its public copy seeds alone until it leaves: {error}")
                continue
            wait = settings["twin_seed_after"] * 2**attempts
            state["stuck"][twin] = [attempts + 1, now + wait]
            print(f"Cannot seed the C411 copy of {public['name']}, next try in {wait // 60:g} minutes: {error}")
            continue
        state["stuck"].pop(twin, None)
        print(f"cross-seed missed {public['name']}, its C411 copy now checks the same files")


def into_farm(qbt, t):
    qbt.keep_in_place([t["hash"]])
    qbt.set_category([t["hash"]], FARM)
    qbt.reset_share_limits([t["hash"]])
    if t["progress"] >= 1 and t["state"].startswith("stopped"):
        qbt.start([t["hash"]])
        t["state"] = "stalledUP"
    t["category"] = FARM


def adopt(qbt, settings, state, torrents):
    room = settings["farm_size"] - size_of(torrents, FARM)
    for t in torrents.values():
        if (
            is_library(settings, t["category"])
            and is_c411(state, settings, t)
            and t["progress"] >= 1
            and t["seeding_time"] >= settings["adopt_after"]
            and t["size"] <= room
        ):
            category = t["category"]
            into_farm(qbt, t)
            room -= t["size"]
            print(f"Moved {t['name']} from {category} to the Farm")


def file_sizes(qbt, t, cache):
    if t["hash"] not in cache:
        cache[t["hash"]] = sorted(f["size"] for f in qbt.files(t["hash"]))
    return cache[t["hash"]]


def same_files(qbt, a, b, cache):
    return a["size"] == b["size"] and file_sizes(qbt, a, cache) == file_sizes(qbt, b, cache)


def shares_its_folder(t, torrents):
    return any(o is not t and o["save_path"] == t["save_path"] and o["size"] == t["size"] for o in torrents.values())


def clean_public(qbt, settings, state, torrents):
    copies = [t for t in torrents.values() if is_c411(state, settings, t) and is_seeding(t)]
    cache = {}
    for t in [t for t in torrents.values() if is_owned(t) and is_public(state, settings, t) and t["progress"] >= 1]:
        if t["seeding_time"] >= settings["public_seeding"]:
            qbt.delete([t["hash"]], not shares_its_folder(t, torrents))
            del torrents[t["hash"]]
            print(f"Removed {t['name']}, a Public torrent that seeded two weeks")
            continue
        if t["seeding_time"] < settings["adopt_after"]:
            continue
        copy = next((c for c in copies if same_files(qbt, c, t, cache)), None)
        if copy is not None:
            qbt.delete([t["hash"]], not shares_its_folder(t, torrents))
            del torrents[t["hash"]]
            print(f"Removed {t['name']}, a Public torrent whose files {copy['name']} now seeds on C411")


def tag(qbt, settings, state, torrents):
    known = {*SOURCES, *RETIRED_SOURCES, PUBLIC}
    wanted, stale = {}, {}
    for t in torrents.values():
        if is_owned(t) and is_public(state, settings, t):
            name = PUBLIC
        elif is_c411(state, settings, t) and t["hash"] in state["sources"]:
            name = state["sources"][t["hash"]]
        else:
            continue
        tags = {x.strip() for x in t.get("tags", "").split(",")}
        if name not in tags:
            wanted.setdefault(name, []).append(t["hash"])
        for old in tags & known - {name}:
            stale.setdefault(old, []).append(t["hash"])
    for name, hashes in wanted.items():
        qbt.add_tags(hashes, name)
    for name, hashes in stale.items():
        qbt.remove_tags(hashes, name)


def episode_of(title):
    found = re.search(r"S\d{2}E\d{2}", title, re.IGNORECASE)
    return found.group(0).upper() if found else None


def language_of(title):
    upper = title.upper()
    return next((language for language in ("VOSTFR", "MULTI", "FRENCH") if language in upper), None)


def squeeze(text):
    return re.sub(r"[^a-z0-9]", "", text.lower())


def group_of(title):
    return squeeze(title.rsplit("-", 1)[1]) if "-" in title else ""


def may_be_twin(release, candidate):
    group = group_of(release["title"])
    return (
        group != ""
        and group in squeeze(candidate["title"])
        and episode_of(candidate["title"]) == episode_of(release["title"])
        and abs(candidate["size"] - release["size"]) <= 0.05 * release["size"]
        and candidate["seeders"] >= 1
    )


def fetched(read, key, cache):
    if key not in cache:
        cache[key] = read()
    return cache[key]


def find_twin(prowlarr, c411, settings, release, now, cache):
    episode, language = episode_of(release["title"]), language_of(release["title"])
    if not episode or not language:
        return None
    for indexer in settings["twin_indexers"]:
        query = (indexer, f"{episode} {language}")
        if query not in cache:
            cache[query] = prowlarr.search(indexer, query[1], now)
        for candidate in cache[query]:
            if may_be_twin(release, candidate):
                twin_torrent = fetched(lambda c=candidate: prowlarr.torrent(c), ("public", candidate["infohash"]), cache)
                tracker = fetched(lambda: c411.download(release["infohash"]), ("c411", release["infohash"]), cache)
                if file_lengths(twin_torrent) == file_lengths(tracker):
                    return {**candidate, "torrent": twin_torrent}
    return None


def weigh_floor(state, account, settings):
    paused = account is None or account["uploaded"] < settings["floor"] * account["downloaded"]
    if paused and not state["paused"]:
        print(f"Paused: the Account ratio is unknown or under {settings['floor']:g}, ratio spends no download on C411")
    if state["paused"] and not paused:
        print(f"Resumed: the Account ratio is back over {settings['floor']:g}")
    state["paused"] = paused


def download_left(account, settings):
    return account["uploaded"] / settings["floor"] - account["downloaded"]


def hold_c411(prowlarr, settings, state, wanted):
    if state["held"] == wanted or not settings["hold_indexers"]:
        return
    try:
        for indexer in settings["hold_indexers"]:
            before = prowlarr.hold(indexer, wanted, settings["paused_profile"], state["restore"].get(indexer))
            if wanted and before is not None:
                state["restore"][indexer] = before
    except Throttled:
        raise
    except FAILURES as error:
        print(f"Cannot {'hold' if wanted else 'release'} C411 in Prowlarr: {error}")
        return
    state["held"] = wanted
    print(f"C411 is {'held' if wanted else 'open again'} for Radarr, Sonarr and Lidarr")


def use_prowlarr(state, now, step, always=False):
    if now < state["backoff_until"] and not always:
        return
    try:
        step()
    except Throttled as throttled:
        state["backoff_until"] = now + throttled.retry_after
        why = "has disabled the indexer after failures" if throttled.disabled else "asks to slow down"
        print(f"Prowlarr {why}, ratio leaves it alone for {throttled.retry_after // 60:g} minutes")


def take(qbt, state, budget, known, release, torrent, source, via):
    qbt.add(torrent, FARM)
    known.add(release["infohash"])
    state["sources"][release["infohash"]] = source
    if release.get("category"):
        state["categories"][release["infohash"]] = release["category"]
    budget[FARM] -= release["size"]
    budget["slots"] -= 1
    print(
        f"Grabbed {release['title']} for the Farm from {via}, "
        f"{release['seeders']} seeders, {release['leechers']} leechers, {release['size'] / GB:.2f} GB"
    )


def grab_twins(qbt, prowlarr, c411, settings, state, releases, known, budget, now):
    searches = {}
    lookups = settings["twin_lookups"]
    episodes = [
        r
        for r in releases
        if r["infohash"] not in known
        and episode_of(r["title"])
        and r["age_hours"] <= settings["twin_max_age_hours"]
        and r["leechers"] >= settings["twin_min_leechers"]
        and state["checked"].get(r["infohash"], 0) < now - settings["twin_recheck"]
    ]
    for release in sorted(episodes, key=lambda r: -r["leechers"]):
        if budget["slots"] <= 0 or lookups <= 0:
            return
        if release["size"] > budget[FARM]:
            continue
        lookups -= 1
        state["checked"][release["infohash"]] = now
        try:
            twin = find_twin(prowlarr, c411, settings, release, now, searches)
            if twin:
                take(qbt, state, budget, known, release, twin["torrent"], "free", f"its public twin {twin['title']}")
                state["twins"][twin["infohash"]] = [release["infohash"], FARM, now, ""]
        except (Throttled, OutOfCalls):
            raise
        except FAILURES as error:
            print(f"Cannot look for a twin of {release['title']}: {error}")


def take_from_c411(qbt, c411, state, budget, known, release, source, via):
    try:
        take(qbt, state, budget, known, release, c411.download(release["infohash"]), source, via)
    except OutOfCalls:
        raise
    except FAILURES as error:
        state["given_up"][release["infohash"]] = time.time()
        known.add(release["infohash"])
        print(f"Cannot grab {release['title']}, ratio leaves it alone for a week: {error}")
        return False
    return True


def take_paid(qbt, c411, state, budget, known, release, source, now):
    free = release["free"]
    via = "C411, freeleech" if free else "C411"
    if not take_from_c411(qbt, c411, state, budget, known, release, "free" if free else source, via):
        return False
    if free:
        state["free"][release["infohash"]] = now
    else:
        state["hunted"][release["infohash"]] = [now, release["size"]]
        budget["download"] -= release["size"]
    return True


def affordable(release, budget):
    return release["size"] <= budget[FARM] and (release["free"] or release["size"] <= budget["download"])


def grab_free(qbt, c411, settings, state, releases, known, budget, now):
    for release in pick_free(releases, known, settings["free_max_size"]):
        if budget["slots"] <= 0:
            return
        if release["infohash"] in known or release["size"] > budget[FARM]:
            continue
        take_paid(qbt, c411, state, budget, known, release, "free", now)


def grab_paid(qbt, c411, settings, state, releases, known, budget, now):
    for release in pick_paid(releases, known, settings):
        if budget["slots"] <= 0:
            return
        if release["infohash"] in known or not affordable(release, budget):
            continue
        take_paid(qbt, c411, state, budget, known, release, "paid", now)


def hunt_quests(qbt, c411, settings, state, hunts, known, budget, now):
    scales = scales_in(state)
    for slug, missing in hunts:
        if budget["slots"] <= 0:
            return
        key = slug or "all"
        page = state["pages"].get(key, 1)
        releases, pages = c411.catalog(now, slug, "size", "asc", page)
        picks = [r for r in pick_small(releases, known, settings["max_size"], scales) if affordable(r, budget)]
        if not picks:
            state["pages"][key] = page + 1 if page < pages else 1
            continue
        for release in picks:
            if missing <= 0 or budget["slots"] <= 0:
                break
            if release["infohash"] in known or not affordable(release, budget):
                continue
            if take_paid(qbt, c411, state, budget, known, release, "quest", now):
                missing -= 1


def pending(state, torrents):
    return sum(
        t["size"] * (1 - t["progress"])
        for t in torrents.values()
        if t["hash"] in state["transfers"] and t["hash"] not in state["free"] and t["progress"] < 1
    )


def grab(qbt, prowlarr, c411, settings, switches, state, torrents, account, hunts, now):
    downloading = sum(1 for t in torrents.values() if t["category"] == FARM and t["progress"] < 1)
    spent = sum(size for _, size in state["hunted"].values())
    budget = {
        "slots": settings["max_downloading"] - downloading,
        FARM: settings["farm_size"] - size_of(torrents, FARM),
        "download": 0 if state["paused"] or account is None else max(
            min(download_left(account, settings) - pending(state, torrents), settings["daily"] - spent), 0
        ),
    }
    if budget["slots"] <= 0 or budget[FARM] <= 0:
        return
    known = set(torrents) | twin_copies(state) | set(state["given_up"])
    if switches["twins"] or switches["free"] or switches["paid"]:
        releases, _ = c411.catalog(now)
        if switches["twins"] and settings["twin_indexers"]:
            use_prowlarr(state, now, lambda: grab_twins(qbt, prowlarr, c411, settings, state, releases, known, budget, now))
        if switches["free"]:
            grab_free(qbt, c411, settings, state, releases, known, budget, now)
        if switches["paid"]:
            grab_paid(qbt, c411, settings, state, releases, known, budget, now)
    hunt_quests(qbt, c411, settings, state, hunts, known, budget, now)


def entry_of(day, settings):
    return {
        "day": day, "floor": settings["floor"], "ratio": None, "left_gb": None, "uploaded_gb": None,
        "actual_gb": None, "downloaded_gb": None, "downloaded_day_gb": None, "zen": None, "earned": None,
        "zen_paid": None, "seeds": None,
    }


def account_fields(uploaded, downloaded, settings):
    return {
        "ratio": share(uploaded, downloaded),
        "left_gb": gigabytes(download_left({"uploaded": uploaded, "downloaded": downloaded}, settings)),
        "uploaded_gb": gigabytes(uploaded),
        "downloaded_gb": gigabytes(downloaded),
    }


def backfill(state, settings, today):
    first = state["daily"][0]["day"] if state["daily"] else today.isoformat()
    older = {}
    for at, uploaded, downloaded, zen in state["account"]:
        day = datetime.date.fromtimestamp(at).isoformat()
        if day < first:
            older[day] = {**entry_of(day, settings), **account_fields(uploaded, downloaded, settings), "zen": zen}
    state["daily"][:0] = [older[day] for day in sorted(older)]


def received_base(state, account, now):
    samples = state["received"]
    samples.append([now, account["downloaded"]])
    samples[:] = [s for s in samples if s[0] <= now - DAY][-1:] + [s for s in samples if s[0] > now - DAY]
    return samples[0][1]


def record_day(state, settings, seeds, account, now):
    today = datetime.date.fromtimestamp(now)
    backfill(state, settings, today)
    daily = state["daily"]
    current = daily.pop() if daily and daily[-1]["day"] == today.isoformat() else {}
    yesterday = (today - datetime.timedelta(days=1)).isoformat()
    earned = next((e["earned"] for e in daily if e["day"] == yesterday), None)
    entry = {**entry_of(today.isoformat(), settings), **{k: v for k, v in current.items() if v is not None}}
    entry.update(floor=settings["floor"], seeds=len(seeds))
    entry.pop("sources", None)
    if account is not None:
        entry.update(
            account_fields(account["uploaded"], account["downloaded"], settings),
            downloaded_day_gb=gigabytes(account["downloaded"] - received_base(state, account, now)),
            actual_gb=None if account.get("actual") is None else gigabytes(account["actual"]),
            zen=account["zen"],
            earned=account["earned"],
            zen_paid=None if earned is None or account["earned"] is None else account["earned"] - earned,
        )
    daily.append(entry)
    oldest = (today - datetime.timedelta(days=HISTORY_DAYS - 1)).isoformat()
    daily[:] = [e for e in daily if e["day"] >= oldest]


def read_account(c411, state, now):
    try:
        account = c411.account()
    except FAILURES as error:
        print(f"Cannot read the profile of C411, ratio spends no download on C411: {error}")
        return None
    samples = state["account"]
    if not samples or now - samples[-1][0] >= DAY:
        samples.append([now, account["uploaded"], account["downloaded"], account["zen"]])
    samples[:] = [s for s in samples if s[0] >= now - WEEK - DAY]
    return account


def tiers_of(account, settings):
    found = []
    for name, ratio in TIERS:
        ratio = settings["floor"] if ratio is None else ratio
        gap = None
        if account is not None:
            up, down = account["uploaded"], account["downloaded"]
            gap = gigabytes(ratio * down - up) if up < ratio * down else -gigabytes(up / ratio - down)
        found.append({"name": name, "ratio": ratio, "gap_gb": gap})
    return sorted(found, key=lambda t: t["ratio"])


def tier_index(account, settings):
    if account is None or not account["downloaded"]:
        return None
    ratio = account["uploaded"] / account["downloaded"]
    return sum(1 for t in tiers_of(account, settings) if ratio >= t["ratio"])


def account_board(account, state, settings):
    if account is None:
        return {"ratio": None, "floor": settings["floor"], "paused": True, "allowed": None}
    return {
        "ratio": share(account["uploaded"], account["downloaded"]),
        "floor": settings["floor"],
        "paused": state["paused"],
        "allowed": account["allowed"],
        "uploaded_gb": gigabytes(account["uploaded"]),
        "downloaded_gb": gigabytes(account["downloaded"]),
        "actual_gb": None if account["actual"] is None else gigabytes(account["actual"]),
        "credit_gb": None if account["credit"] is None else gigabytes(account["credit"]),
        "zen": account["zen"],
        "class": account["class"],
        "grade": account["grade"],
        "voies": account["voies"],
    }


def board_of(state, settings, controls, torrents, seeds, account, now):
    scales = scales_in(state)
    counts = category_counts(state, seeds)
    done = set(state["done"])
    challenges = []
    for challenge in CHALLENGES:
        level = next_level(challenge, done)
        levels = levels_of(challenge)
        shown_level = level or levels[-1]
        current, target = progress_of(shown_level, seeds, counts, scales, done)
        challenges.append({
            "key": challenge,
            "voie": shown_level["voie"],
            "level": shown_level["level"],
            "levels": len(levels),
            "done": level is None,
            "description": shown_level["description"],
            "current": current,
            "target": target,
            "switch": challenge if challenge in SWITCHES else None,
        })
    farm = [t for t in torrents.values() if t["category"] == FARM]
    slices = {}
    for t in farm:
        source = state["sources"].get(t["hash"]) if is_c411(state, settings, t) else PUBLIC
        source = source if source in SOURCES or source == PUBLIC else "unknown"
        slices[source] = slices.get(source, 0) + t["size"]
    return {
        "updated_at": datetime.datetime.fromtimestamp(now, datetime.UTC).isoformat(),
        "account": account_board(account, state, settings),
        "tiers": tiers_of(account, settings),
        "history": [dict(e) for e in state["daily"]],
        "seeds": len(seeds),
        "known": sum(1 for t in seeds if t["hash"] in state["categories"]),
        "done": sorted(done),
        "voies": {
            voie: {"done": sum(1 for q in QUESTS if q["voie"] == voie and q["id"] in done), "total": sum(1 for q in QUESTS if q["voie"] == voie)}
            for voie in VOIES
        },
        "challenges": challenges,
        "categories": [
            {"slug": slug, "name": name, "count": counts[slug], "target": category_target(slug, done), "switch": f"volume:{slug}"}
            for slug, (_, name) in C411_CATEGORIES.items()
        ],
        "controls": controls,
        "farm": {
            "budget_gb": gigabytes(settings["farm_size"]),
            "slices": [{"source": s, "gb": gigabytes(size)} for s, size in sorted(slices.items(), key=lambda i: -i[1])],
        },
        "sources": [
            {"source": s, **f} for s, f in sorted(figures_by_source(state).items(), key=lambda i: -i[1]["uploaded_gb"])
        ],
    }


def waiting_board(state, settings, controls):
    return {
        "updated_at": None, "account": {"ratio": None, "floor": settings["floor"], "paused": True, "allowed": None},
        "tiers": tiers_of(None, settings), "history": [dict(e) for e in state["daily"]], "seeds": None, "known": 0,
        "done": list(state["done"]), "voies": {}, "challenges": [], "categories": [], "controls": controls,
        "farm": {"budget_gb": gigabytes(settings["farm_size"]), "slices": []}, "sources": [],
    }


def tell_board(url):
    try:
        with urllib.request.urlopen(urllib.request.Request(url, data=b"", method="POST"), timeout=10):
            pass
    except FAILURES as error:
        print(f"Cannot tell farm-board to refresh: {error}")


def run_once(qbt, prowlarr, c411, settings, controls, state, now, notify=tell_board):
    settings = effective(settings, controls)
    switches = controls["switches"]
    prepare(qbt, settings)
    c411.allow(settings["c411_calls"])
    account = read_account(c411, state, now)
    weigh_floor(state, account, settings)
    torrents = {t["hash"].lower(): {**t, "hash": t["hash"].lower()} for t in qbt.torrents()}
    record(state, torrents, settings, now)
    report(state, now)
    refresh_scales(c411, state, account, now)
    use_prowlarr(state, now, lambda: hold_c411(prowlarr, settings, state, state["paused"] and switches["hold"]), always=True)
    if switches["adopt"]:
        adopt(qbt, settings, state, torrents)
    if switches["drop_twins"]:
        drop_twins(qbt, state, torrents)
    if switches["clean_public"]:
        clean_public(qbt, settings, state, torrents)
    tag(qbt, settings, state, torrents)
    seeds = seeded_on_c411(state, settings, torrents)
    validate_quests(state, account, seeds, category_counts(state, seeds), scales_in(state))
    hunts = wanted_hunts(switches, state, seeds, category_counts(state, seeds))
    for step in (
        lambda: grab(qbt, prowlarr, c411, settings, switches, state, torrents, account, hunts, now),
        lambda: use_prowlarr(state, now, lambda: seed_twins(qbt, c411, settings, state, torrents, now)),
        lambda: learn_categories(c411, state, settings, torrents),
    ):
        try:
            step()
        except OutOfCalls:
            print(f"ratio spent its {settings['c411_calls']} calls to C411 of the run, the rest waits for the next run")
            break
        except FAILURES as error:
            print(f"Cannot read C411: {error}")
    torrents = {t["hash"].lower(): {**t, "hash": t["hash"].lower()} for t in qbt.torrents()}
    seeds = seeded_on_c411(state, settings, torrents)
    record_day(state, settings, seeds, account, now)
    tier = tier_index(account, settings)
    if tier is not None and state["tier"] is not None and tier != state["tier"]:
        print("The Account ratio crossed a tier, farm-board refreshes the Discord message")
        notify(settings["board_refresh"])
    if tier is not None:
        state["tier"] = tier
    return board_of(state, settings, controls, torrents, seeds, account, now)


def serve(latest, controls, settings, wake, port=8000):
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path.rstrip("/") != "/board":
                self.send_error(404)
                return
            self.answer(200, latest["board"])

        def do_POST(self):
            if self.path.rstrip("/") != "/controls":
                self.send_error(404)
                return
            try:
                length = int(self.headers.get("Content-Length") or 0)
                asked = json.loads(self.rfile.read(length) or b"{}")
                controls.change(asked.get("name"), asked.get("value"))
            except (ValueError, AttributeError) as error:
                self.answer(400, {"error": str(error)})
                return
            snapshot = controls.snapshot(settings)
            latest["board"] = {**latest["board"], "controls": snapshot}
            wake.set()
            self.answer(200, snapshot)

        def answer(self, code, value):
            body = json.dumps(value).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("", port), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def number(name, default=None):
    value = os.environ.get(name) or default
    try:
        return float(value)
    except (TypeError, ValueError):
        sys.exit(f"Set {name} in .env to a number, then recreate ratio")


def positive(name, default):
    value = number(name, default)
    if value <= 0:
        sys.exit(f"Set {name} in .env to a number over 0, then recreate ratio")
    return value


def ids(name):
    return [i.strip() for i in os.environ.get(name, "").split(",") if i.strip()]


def settings_from_env():
    return {
        "tracker": (os.environ.get("RATIO_TRACKER") or "c411").lower(),
        "hold_indexers": ids("RATIO_HOLD_INDEXER_IDS"),
        "twin_indexers": ids("RATIO_TWIN_INDEXER_IDS"),
        "cross_seed_key": os.environ.get("CROSS_SEED_API_KEY", ""),
        "farm_size": positive("RATIO_MAX_SIZE_GB", 100) * GB,
        "floor": positive("RATIO_FLOOR", 1.2),
        "daily": number("RATIO_DAILY_GB", 5) * GB,
        "max_size": GB,
        "paused_profile": "Paused",
        "free_max_size": 5 * GB,
        "paid_max_age_hours": 24,
        "paid_min_leechers": 5,
        "public_seeding": 14 * DAY,
        "max_downloading": int(positive("RATIO_MAX_DOWNLOADING", 2)),
        "c411_calls": 30,
        "twin_lookups": 10,
        "twin_max_age_hours": 24,
        "twin_min_leechers": 1,
        "twin_recheck": 2 * 3600,
        "twin_seed_after": 3600,
        "library_categories": ("movies", "shows", "music", "books"),
        "adopt_after": 6 * 3600,
        "board_refresh": "http://farm-board:8000/refresh",
    }


def main():
    settings = settings_from_env()
    qbt = QBittorrent("http://gluetun:8080", os.environ.get("QBITTORRENT_USER", ""), os.environ.get("QBITTORRENT_PASSWORD", ""))
    prowlarr = Prowlarr("http://prowlarr:9696", os.environ.get("PROWLARR_API_KEY", ""))
    c411 = C411(os.environ.get("C411_PROFILE_API_KEY", ""), os.environ.get("C411_TORZNAB_API_KEY", ""))
    state_path = "/state/state.json"
    state = load_state(state_path)
    controls = Controls("/state/controls.json")
    wake = threading.Event()
    latest = {"board": waiting_board(state, settings, controls.snapshot(settings))}
    serve(latest, controls, settings, wake)
    while True:
        snapshot = controls.snapshot(settings)
        try:
            board = run_once(qbt, prowlarr, c411, settings, snapshot, state, time.time())
            latest["board"] = {**board, "controls": controls.snapshot(settings)}
        except Exception:
            LOG.exception("Cannot run the Farm")
            latest["board"] = {**latest["board"], "history": [dict(e) for e in state["daily"]], "controls": snapshot}
        try:
            save_state(state_path, state)
        except OSError as error:
            print(f"Cannot save {state_path}: {error}")
        wake.wait(600)
        wake.clear()


if __name__ == "__main__":
    main()
