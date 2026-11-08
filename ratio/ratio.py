import datetime
import email.utils
import html
import http.cookiejar
import http.server
import json
import math
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
LEGACY = {
    "ratio": FARM,
    "zen": FARM,
    "cross-seed-link": FARM,
    "radarr": "movies",
    "tv-sonarr": "shows",
    "lidarr": "music",
    "lazylibrarian": "books",
}
PUBLIC = "public"
SEEDING = ("uploading", "stalledUP", "queuedUP", "forcedUP")
SHARE_LIMITS = ("max_ratio_enabled", "max_seeding_time_enabled", "max_inactive_seeding_time_enabled")
SEEDING_STEPS = ((180, 0.5), (90, 0.35), (30, 0.25), (7, 0.1))
RARE_STEP = 0.25
BEST_NOTE = 1 + SEEDING_STEPS[0][1] + RARE_STEP
GOOD_NOTE = 0.625
SEED_STEPS = (100, 150, 200, 250, 300)
CONTRIBUTION_SHARES = ("62.5", "75", "100", "100", "100", "100")
DURATION_STEPS = ((25, 7), (100, 30), (150, 90), (250, 180), (300, 365))
HISTORY_DAYS = 30
SOURCES = {
    "paid": "paid grab",
    "twin": "Twin",
    "freeleech": "freeleech",
    "cross-seed": "cross-seed of the Library",
    "library": "Library download",
    "zen-hunt": "Zen hunt",
    "volume-hunt": "Volume hunt",
    "unknown": "unknown",
}
FILMS = "Films & Vidéos"
QUEST_CATEGORIES = {
    FILMS: "102000,102010,102030,102050,102060,102070,102080,102090,105000,105060,105070,105080",
    "Ebook": "107000,107010,107020,107030",
    "Audio": "103000,103010,103030,103050",
    "Emulation": "101000,101030,101040,101080,101090",
    "Imprimante 3D": "108010",
}
ANIME = "102060,105070"
FILE_KINDS = {
    FILMS: {"mkv", "mp4", "avi", "m2ts", "ts", "mov", "wmv", "webm", "m4v"},
    "Ebook": {"epub", "pdf", "mobi", "azw3", "cbz", "cbr", "djvu", "fb2"},
    "Audio": {"mp3", "flac", "m4a", "m4b", "ogg", "opus", "wav", "aac", "alac", "ape"},
    "Emulation": {"nes", "sfc", "smc", "gb", "gbc", "gba", "nds", "3ds", "n64", "z64", "chd", "cue", "rvz", "wbfs", "nsp", "xci", "md", "sms", "gg", "pce", "a26"},
    "Imprimante 3D": {"stl", "3mf", "obj", "gcode", "step", "stp"},
}
KINDS = {"movies": FILMS, "shows": FILMS, "music": "Audio", "books": "Ebook"}
TWIN_ATTEMPTS = 3
TORZNAB = "{http://torznab.com/schemas/2015/feed}attr"


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

    def remove_categories(self, names):
        self.post("/api/v2/torrents/removeCategories", {"categories": "\n".join(names)})

    def add_tags(self, hashes, tag):
        self.post("/api/v2/torrents/addTags", {"hashes": "|".join(hashes), "tags": tag})

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


def wait_of(header):
    return max(int(header), 60) if header and header.strip().isdigit() else 3600


class Prowlarr:
    def __init__(self, url, api_key, tracker_seeders_in_peers, spacing=2):
        self.url = url.rstrip("/")
        self.api_key = api_key
        self.tracker_seeders_in_peers = tracker_seeders_in_peers
        self.spacing = spacing
        self.last = 0

    def newest(self, indexer, now):
        fields = {"t": "search", "cat": "2000,5000", "limit": 100}
        return self.query(indexer, fields, now, self.tracker_seeders_in_peers)

    def category(self, indexer, cats, now):
        return self.query(indexer, {"t": "search", "cat": cats, "limit": 100}, now, self.tracker_seeders_in_peers)

    def search(self, indexer, title, now):
        return self.query(indexer, {"t": "search", "q": re.sub(r"[._]+", " ", title), "limit": 100}, now, 1)

    def torrent(self, release):
        page = re.search(r"nyaa\.si/view/(\d+)", release["page"])
        link = f"https://nyaa.si/download/{page.group(1)}.torrent" if page else release["link"]
        return self.open(urllib.request.Request(link, headers={"User-Agent": "Mozilla/5.0 (compatible; jellynas-ratio)"}))

    def query(self, indexer, fields, now, seeders_in_peers):
        request = urllib.request.Request(f"{self.url}/{indexer}/api?{urllib.parse.urlencode(fields)}", headers={"X-Api-Key": self.api_key})
        return parse_torznab(self.open(request), now, seeders_in_peers)

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


class C411:
    def __init__(self, api_key, url="https://c411.org/api/user/profile"):
        self.api_key = api_key
        self.url = url

    def account(self):
        if not self.api_key:
            raise RuntimeError("C411_PROFILE_API_KEY is empty")
        request = urllib.request.Request(
            self.url, headers={"Authorization": f"Bearer {self.api_key}", "User-Agent": "jellynas-ratio"}
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            profile = json.load(response)
        return {
            "uploaded": profile["transfer"]["uploaded"]["total"],
            "downloaded": profile["transfer"]["downloaded"]["total"],
            "zen": profile["zen"]["balance"],
            "earned": profile["zen"].get("totalEarned"),
            "actual": profile["transfer"]["uploaded"].get("actual"),
        }


def parse_torznab(xml_bytes, now, seeders_in_peers=1):
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
                "leechers": max(peers - seeders_in_peers * seeders, 0),
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


def rare_limit(size):
    if size < GB:
        return 24
    if size <= 100 * GB:
        return 50
    return 199


def is_rare(size, seeders):
    return seeders <= rare_limit(size)


def pick_free(releases, known, max_size):
    free = [r for r in releases if r["free"] and r["infohash"] not in known and r["seeders"] >= 1 and r["size"] <= max_size]
    return sorted(free, key=lambda r: -r["leechers"])


def cost_of(release):
    return 0 if release["free"] else release["size"]


def pick_small(releases, known, max_size):
    small = [
        r
        for r in releases
        if r["infohash"] not in known
        and r["size"] <= max_size
        and r["seeders"] >= 1
        and is_rare(r["size"], r["seeders"])
    ]
    return sorted(small, key=lambda r: (cost_of(r), r["size"], r["seeders"]))


def note_of(t):
    days = t["seeding_time"] / DAY
    note = 1 + next((step for at, step in SEEDING_STEPS if days >= at), 0)
    if is_rare(t["size"], t["num_complete"]):
        note += RARE_STEP
    return note


def nightly_zen(t):
    size = t["size"] / 1e9
    weight = (0.25 + size) / 4.25
    return note_of(t) * (weight if size <= 4 else math.sqrt(weight)) / 7


def zen_per_gb(t):
    return nightly_zen(t) / max(t["size"] / GB, 0.001)


def size_of(torrents, category):
    return sum(t["size"] for t in torrents.values() if t["category"] == category)


def load_state(path):
    try:
        with open(path, encoding="utf-8") as file:
            state = json.load(file)
    except FileNotFoundError:
        state = {}
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
        "held": state.get("held"),
        "backoff_until": state.get("backoff_until", 0),
        "stuck": state.get("stuck", {}),
        "given_up": state.get("given_up", {}),
        "restore": state.get("restore", {}),
        "kinds": state.get("kinds", {}),
        "hunt_at": state.get("hunt_at", {}),
        "daily": state.get("daily", []),
    }


def save_state(path, state):
    temporary = f"{path}.tmp"
    with open(temporary, "w", encoding="utf-8") as file:
        json.dump(state, file)
    os.replace(temporary, path)


def prepare(qbt, settings):
    categories = qbt.categories()
    for name, path in CATEGORIES.items():
        if name not in categories:
            qbt.create_category(name, path)
    preferences = qbt.preferences()
    limits = {key: False for key in SHARE_LIMITS if preferences.get(key)}
    if limits:
        qbt.set_preferences(limits)
        print("qBittorrent no longer stops torrents at its global share limits, ratio removes them")
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
    return t["category"] in CATEGORIES or t["category"] in LEGACY


def is_library(settings, category):
    return category in settings["library_categories"] or LEGACY.get(category) in settings["library_categories"]


def is_seeding(t):
    return t["progress"] >= 1 and t["state"] in SEEDING


def twin_copies(state):
    return {entry[0] for entry in state["twins"].values()}


def source_of(state, settings, copies, t):
    if t is None:
        return "unknown"
    if t["hash"] in copies:
        return "twin"
    if t["hash"] in state["free"]:
        return "freeleech"
    if t["category"] == "ratio":
        return "paid"
    if t["category"] in ("cross-seed-link", FARM):
        return "cross-seed"
    if is_library(settings, t["category"]):
        return "library"
    if t["category"] == "zen":
        return "zen-hunt"
    return "unknown"


def record_sources(state, torrents, settings):
    copies = twin_copies(state)
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
    record_sources(state, torrents, settings)
    record_kinds(state, torrents, settings)
    record_history(state, torrents, fresh, now)
    record_zen(state, torrents)


def record_kinds(state, torrents, settings):
    for t in torrents.values():
        kind = KINDS.get(LEGACY.get(t["category"], t["category"]))
        if kind and t["hash"] not in state["kinds"] and is_c411(state, settings, t):
            state["kinds"][t["hash"]] = kind
    copies = twin_copies(state)
    for infohash in [h for h in state["kinds"] if h not in torrents and h not in copies]:
        del state["kinds"][infohash]


def kind_of_files(files):
    sizes = {}
    for f in files:
        extension = f["name"].rsplit(".", 1)[-1].lower() if "." in f["name"] else ""
        kind = next((k for k, extensions in FILE_KINDS.items() if extension in extensions), None)
        if kind:
            sizes[kind] = sizes.get(kind, 0) + f["size"]
    return max(sizes, key=sizes.get) if sizes else ""


def classify(qbt, state, settings, torrents):
    for t in torrents.values():
        if t["hash"] in state["kinds"] or not is_c411(state, settings, t) or t["progress"] < 1:
            continue
        try:
            state["kinds"][t["hash"]] = kind_of_files(qbt.files(t["hash"]))
        except Exception as error:
            print(f"Cannot read the files of {t['name']}: {error}")


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
            f"Source {SOURCES[source]}: {f['torrents']} torrents, {f['downloaded_gb']:.1f} GB down, "
            f"{f['uploaded_gb']:.1f} GB up, ratio {shown(f['ratio'])}, last 7 days {f['week_downloaded_gb']:.1f} GB down, "
            f"{f['week_uploaded_gb']:.1f} GB up, ratio {shown(f['week_ratio'])}, {f['farm_days']:.1f} days in the Farm"
        )


def seeded_on_c411(state, settings, torrents):
    return [t for t in torrents.values() if is_c411(state, settings, t) and is_seeding(t)]


def farm_of(state, settings, torrents):
    held = [t for t in torrents.values() if t["category"] == FARM]
    size = sum(t["size"] for t in held)
    limit = settings["farm_size"]
    return {
        "seeds": len(seeded_on_c411(state, settings, torrents)),
        "size_gb": gigabytes(size),
        "budget_gb": gigabytes(limit),
        "fill": round(100 * size / limit, 1) if limit else 0,
        "downloading": sum(1 for t in held if t["progress"] < 1),
    }


def challenges_of(state, settings, torrents):
    seeded = seeded_on_c411(state, settings, torrents)
    kinds = [state["kinds"].get(t["hash"]) for t in seeded]
    tops = sorted((kinds.count(name) for name in QUEST_CATEGORIES), reverse=True)

    def good(share_of_best):
        return sum(1 for t in seeded if note_of(t) >= share_of_best * BEST_NOTE)

    def lasting(days):
        return sum(1 for t in seeded if t["seeding_time"] >= days * DAY)

    return [
        ("seed_volume", [(n, len(seeded)) for n in SEED_STEPS], True),
        ("contribution", [(50, good(GOOD_NOTE)), (100, good(0.75))] + [(n, good(1)) for n in (150, 200, 250, 300)], True),
        ("duration", [(n, lasting(days)) for n, days in DURATION_STEPS], True),
        ("diversity", [(25 * k, sum(min(c, 25) for c in tops[:k])) for k in range(1, len(QUEST_CATEGORIES) + 1)], False),
        ("category", [(50, tops[0]), (100, tops[0]), (250, kinds.count(FILMS)), (500, kinds.count(FILMS))], False),
    ]


def step_name(key, index, target):
    if key == "contribution":
        return f"{target} torrents at {CONTRIBUTION_SHARES[index]} % of their best note"
    if key == "duration":
        return f"{target} seeds for {DURATION_STEPS[index][1]} days"
    if key == "diversity":
        return f"25 seeds in {index + 1} categor{'y' if index == 0 else 'ies'}"
    if key == "category":
        return f"{target} seeds in one category" if index < 2 else f"{target} seeds in {FILMS}"
    return f"Seed count {target}"


def quest_steps(state, settings, torrents):
    for key, steps, _ in challenges_of(state, settings, torrents):
        unit = "torrents" if key == "contribution" else "seeds"
        for index, (target, progress) in enumerate(steps):
            yield step_name(key, index, target), progress, target, unit


def quests_of(state, settings, torrents):
    left = [step for step in quest_steps(state, settings, torrents) if step[1] < step[2]]
    if not left:
        return {"name": "every Quest reached", "progress": None, "target": None, "shown": "done"}
    name, progress, target, unit = max(left, key=lambda step: step[1] / step[2])
    return {"name": name, "progress": progress, "target": target, "shown": f"{progress} of {target} {unit}"}


def card_of(state, t):
    days = t["seeding_time"] / DAY
    ratio = t["uploaded"] / max(t["size"], 1)
    source = state["sources"].get(t["hash"])
    return {
        "name": t["name"],
        "source": source if source in SOURCES else "unknown",
        "ratio": round(ratio, 3),
        "per_day": round(ratio / max(days, 1), 4),
        "note": round(note_of(t), 2),
        "size_gb": gigabytes(t["size"]),
        "days": int(days),
    }


def board_of(state, settings, torrents):
    farm = [t for t in torrents.values() if t["category"] == FARM]
    slices = {}
    for t in farm:
        source = state["sources"].get(t["hash"]) if is_c411(state, settings, t) else PUBLIC
        source = source if source in SOURCES or source == PUBLIC else "unknown"
        slices[source] = slices.get(source, 0) + t["size"]
    cards = [card_of(state, t) for t in farm if is_c411(state, settings, t) and t["progress"] >= 1 and t["seeding_time"] >= DAY]
    best = sorted(cards, key=lambda c: -c["per_day"])[:3]
    worst = sorted((c for c in cards if all(c is not b for b in best)), key=lambda c: c["per_day"])[:3]
    return {
        "history": [dict(e) for e in state["daily"]],
        "challenges": [
            {"key": key, "steps": [{"target": t, "progress": p} for t, p in steps], "max": has_max}
            for key, steps, has_max in challenges_of(state, settings, torrents)
        ],
        "farm": {
            "budget_gb": gigabytes(settings["farm_size"]),
            "slices": [{"source": s, "gb": gigabytes(size)} for s, size in sorted(slices.items(), key=lambda i: -i[1])],
        },
        "best": best,
        "worst": worst,
    }


def waiting_board(state, settings):
    return {
        "history": [dict(e) for e in state["daily"]],
        "challenges": [],
        "farm": {"budget_gb": gigabytes(settings["farm_size"]), "slices": []},
        "best": [],
        "worst": [],
    }


def entry_of(day, settings):
    return {
        "day": day, "floor": settings["floor"], "ratio": None, "left_gb": None, "uploaded_gb": None,
        "actual_gb": None, "downloaded_gb": None, "zen": None, "earned": None, "zen_paid": None, "seeds": None,
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


def record_day(state, settings, torrents, account, now):
    today = datetime.date.fromtimestamp(now)
    backfill(state, settings, today)
    daily = state["daily"]
    current = daily.pop() if daily and daily[-1]["day"] == today.isoformat() else {}
    yesterday = (today - datetime.timedelta(days=1)).isoformat()
    earned = next((e["earned"] for e in daily if e["day"] == yesterday), None)
    entry = {**entry_of(today.isoformat(), settings), **{k: v for k, v in current.items() if v is not None}}
    entry.update(floor=settings["floor"], seeds=len(seeded_on_c411(state, settings, torrents)))
    entry.pop("sources", None)
    if account is not None:
        entry.update(
            account_fields(account["uploaded"], account["downloaded"], settings),
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
    except Exception as error:
        print(f"Cannot read the profile of C411, ratio spends no download on C411: {error}")
        return None
    samples = state["account"]
    if not samples or now - samples[-1][0] >= DAY:
        samples.append([now, account["uploaded"], account["downloaded"], account["zen"]])
    samples[:] = [s for s in samples if s[0] >= now - WEEK - DAY]
    return account


def is_under_floor(account, settings):
    return account is None or account["uploaded"] < settings["floor"] * account["downloaded"]


def download_left(account, settings):
    return account["uploaded"] / settings["floor"] - account["downloaded"]


def account_of(account, state, settings, now):
    if account is None:
        return {"ratio": None, "floor": settings["floor"], "paused": True, "zen": None, "week_uploaded_gb": None}
    older = [s for s in state["account"] if s[0] <= now - WEEK]
    week_start = (older[-1] if older else state["account"][0])[1]
    return {
        "ratio": share(account["uploaded"], account["downloaded"]),
        "floor": settings["floor"],
        "paused": state["paused"],
        "zen": account["zen"],
        "week_uploaded_gb": gigabytes(account["uploaded"] - week_start),
    }


def summary(state, torrents, settings, account, now):
    count, downloaded, uploaded, week_downloaded, week_uploaded, _ = [
        sum(column) for column in zip(*(line for _, line in ledger_lines(state)), strict=True)
    ] or [0] * 6
    sources = [
        {
            "name": SOURCES[source],
            "figures": f"{f['torrents']} torrent{'s' if f['torrents'] != 1 else ''}, "
                       f"{f['uploaded_gb']:.1f} / {f['downloaded_gb']:.1f} GB, ratio {shown(f['ratio'])}",
        }
        for source, f in figures_by_source(state).items()
    ]
    return {
        "updated_at": datetime.datetime.fromtimestamp(now, datetime.UTC).isoformat(),
        "ledger": {
            "torrents": count,
            "downloaded_gb": gigabytes(downloaded),
            "uploaded_gb": gigabytes(uploaded),
            "ratio": share(uploaded, downloaded),
            "surplus_gb": gigabytes(uploaded - downloaded),
            "week_downloaded_gb": gigabytes(week_downloaded),
            "week_uploaded_gb": gigabytes(week_uploaded),
            "week_ratio": share(week_uploaded, week_downloaded),
        },
        "account": account_of(account, state, settings, now),
        "farm": farm_of(state, settings, torrents),
        "quests": quests_of(state, settings, torrents),
        "sources": sources,
    }


COLORS = {
    "paid": "#f472b6", "twin": "#a78bfa", "freeleech": "#34d399", "cross-seed": "#60a5fa", "library": "#38bdf8",
    "zen-hunt": "#fbbf24", "volume-hunt": "#fb923c", "unknown": "#94a3b8", PUBLIC: "#64748b",
}
REWARDS = {
    "seed_volume": (("100 Zen",) * 2, ("150 Zen",) * 2, ("200 Zen",) * 2, ("250 Zen",) * 2, ("300 Zen",) * 2),
    "contribution": (
        ("5 free downloads", "5 DL gratuits"), ("+1 % weight", "+1 % de pondération"),
        ("+2 % weight", "+2 % de pondération"), ("+3 % weight", "+3 % de pondération"),
        ("+4 % weight", "+4 % de pondération"), ("+5 % weight", "+5 % de pondération"),
    ),
    "duration": (
        ("25 Zen",) * 2, ("Silver Leech 7 days", "Silver Leech 7 jours"), ("300 Zen",) * 2, ("C411",) * 2,
        ("Free Leech 7 days", "Free Leech 7 jours"),
    ),
    "diversity": (
        ("25 Zen",) * 2, ("C411",) * 2, ("Corridor certified badge", "Badge Certifié Corridor"),
        ("T411 badge", "Badge T411"), ("C411 badge", "Badge C411"),
    ),
    "category": (
        ("Category badge", "Badge Catégorie"), ("Specialty badge", "Badge Spécialité"),
        ("Silver upgrade, Specialty dial", "Upgrade Argent, Cadran Spécialité"), ("Gold upgrade", "Upgrade Or"),
    ),
}
MAX_REWARDS = {
    "seed_volume": ("Prime badge", "Badge Prime"),
    "contribution": ("Contributor badge", "Badge Contributeur"),
    "duration": ("Timeless badge", "Badge Timeless"),
}
TEXT = {
    "en": {
        "gb": "GB", "decimal": ".", "empty": "No history yet, ratio records its first day after its first run.",
        "ratio": "Account ratio", "floor": "floor {v}", "left": "Before the floor", "uploaded": "Uploaded",
        "actual": "{v} real", "downloaded": "Downloaded", "zen": "Zen", "paid_last": "+{v} last night",
        "zen_chart": "Zen", "paid": "paid each night", "balance": "balance", "transfer": "Upload and download",
        "challenges": {
            "seed_volume": "Seed volume", "contribution": "Minimum contribution", "duration": "Seed duration",
            "diversity": "Category diversity", "category": "Category volume",
        },
        "short": {
            "seed_volume": "{n} seeds", "contribution": "{n} at {share} % of best", "duration": "{n} for {days} days",
            "diversity": ("25 in 1 category", "25 in {k} categories"), "category": ("{n} in one category", "{n} in Films"),
        },
        "max": "Every step",
        "disk": "Farm disk", "free": "Free", "of": "of {budget} {gb}",
        "best": "Best ratio per day", "worst": "Lowest ratio per day",
        "per_day": "{v} a day", "card": "ratio {ratio}, note {note}, {size}, {days}", "days": "{n} days",
        "sources": {**SOURCES, PUBLIC: "Public"},
    },
    "fr": {
        "gb": "Go", "decimal": ",", "empty": "Pas encore d'historique, ratio note son premier jour après son premier passage.",
        "ratio": "Ratio du compte", "floor": "plancher {v}", "left": "Avant le plancher", "uploaded": "Envoyé",
        "actual": "{v} réels", "downloaded": "Reçu", "zen": "Zen", "paid_last": "+{v} cette nuit",
        "zen_chart": "Zen", "paid": "payé chaque nuit", "balance": "solde", "transfer": "Envoi et réception",
        "challenges": {
            "seed_volume": "Volume de seed", "contribution": "Contribution minimum", "duration": "Durée de seed",
            "diversity": "Diversité catégories", "category": "Volume catégorie",
        },
        "short": {
            "seed_volume": "{n} seeds", "contribution": "{n} à {share} % du max", "duration": "{n} pendant {days} jours",
            "diversity": ("25 dans 1 catégorie", "25 dans {k} catégories"),
            "category": ("{n} dans une catégorie", "{n} en Films"),
        },
        "max": "Tous les paliers",
        "disk": "Disque de la Farm", "free": "Libre", "of": "sur {budget} {gb}",
        "best": "Meilleur ratio par jour", "worst": "Plus faible ratio par jour",
        "per_day": "{v} par jour", "card": "ratio {ratio}, note {note}, {size}, {days}", "days": "{n} jours",
        "sources": {
            "paid": "Grab payant", "twin": "Twin", "freeleech": "Freeleech", "cross-seed": "Cross-seed de la Library",
            "library": "Téléchargement de la Library", "zen-hunt": "Zen hunt", "volume-hunt": "Volume hunt",
            "unknown": "Inconnue", PUBLIC: "Public",
        },
    },
}
STYLE = """
:root { --text: #e2e8f0; --muted: #94a3b8; --line: rgba(148, 163, 184, 0.22);
  --violet: #a78bfa; --green: #34d399; --amber: #fbbf24; --blue: #60a5fa; --rose: #fb7185; }
* { box-sizing: border-box; }
html, body { margin: 0; height: 100%; background: transparent; color: var(--text);
  font: 13px/1.35 Manrope, ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif; }
html.standalone { color-scheme: dark; background: radial-gradient(circle at 20% 0%, #2e1065 0%, #0f172a 60%); }
html.standalone main { height: auto; min-height: 100vh; }
main { display: flex; flex-direction: column; gap: 10px; height: 100vh; padding: 6px; overflow: auto;
  scrollbar-width: thin; scrollbar-color: rgba(148, 163, 184, 0.35) transparent; }
.kpis { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 10px; }
.kpi, figure, .quest { background: linear-gradient(160deg, rgba(255, 255, 255, 0.07), rgba(255, 255, 255, 0.025));
  border: 1px solid rgba(255, 255, 255, 0.07); border-radius: 10px; padding: 9px 12px;
  box-shadow: 0 1px 0 rgba(255, 255, 255, 0.05) inset, 0 6px 18px rgba(0, 0, 0, 0.12); }
.kpi { position: relative; display: flex; flex-direction: column; min-width: 0; overflow: hidden; }
.kpi::before { content: ""; position: absolute; left: 0; top: 10px; bottom: 10px; width: 3px; border-radius: 0 3px 3px 0; background: var(--accent, var(--muted)); }
.ratio { --accent: var(--violet); } .left { --accent: var(--green); } .up { --accent: var(--blue); }
.down { --accent: var(--rose); } .zen { --accent: var(--amber); }
.label, figcaption, h3 { color: var(--muted); font-size: 11px; font-weight: 500; text-transform: uppercase; letter-spacing: 0.04em; margin: 0; }
.value { font-size: 26px; font-weight: 700; letter-spacing: -0.01em; font-variant-numeric: tabular-nums; line-height: 1.15; white-space: nowrap; }
.sub { color: var(--muted); font-size: 12px; min-height: 16px; }
.ratio .value { color: var(--violet); } .left .value { color: var(--green); } .zen .value { color: var(--amber); }
.charts, .bottom { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 10px; flex: 1; min-height: 140px; }
figure { margin: 0; display: flex; flex-direction: column; gap: 6px; min-width: 0; min-height: 0; }
figcaption { display: flex; gap: 10px; flex-wrap: wrap; align-items: baseline; }
.key { text-transform: none; letter-spacing: 0; font-size: 11px; }
.key::before { content: ""; display: inline-block; width: 8px; height: 8px; border-radius: 2px; margin-right: 4px; background: currentColor; }
.chart { position: relative; flex: 1; min-height: 70px; border-bottom: 1px solid var(--line);
  background: repeating-linear-gradient(to bottom, transparent 0, transparent calc(25% - 1px), rgba(148, 163, 184, 0.07) calc(25% - 1px), rgba(148, 163, 184, 0.07) 25%); }
.chart svg { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; }
.mark { position: absolute; font-size: 10px; color: var(--muted); transform: translateY(-100%); white-space: nowrap; }
.mark.right { right: 0; } .mark.left { left: 0; }
.axis { display: flex; justify-content: space-between; color: var(--muted); font-size: 10px; }
.line { fill: none; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; vector-effect: non-scaling-stroke; }
polygon.area { stroke: none; fill-opacity: 0.14; }
.area.c-ratio { fill: var(--violet); } .area.c-balance { fill: var(--green); } .area.c-up { fill: var(--blue); } .area.c-down { fill: var(--rose); }
.dot { stroke-width: 6; stroke-linecap: round; vector-effect: non-scaling-stroke; }
.guide { stroke: var(--muted); stroke-width: 1; stroke-dasharray: 4 4; vector-effect: non-scaling-stroke; }
.c-ratio { stroke: var(--violet); color: var(--violet); } .c-balance { stroke: var(--green); color: var(--green); }
.c-paid { fill: var(--amber); fill-opacity: 0.85; color: var(--amber); } .c-up { stroke: var(--blue); color: var(--blue); }
.c-down { stroke: var(--rose); color: var(--rose); }
.quests { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 10px; }
.quest ol, .ranking { list-style: none; margin: 6px 0 0; padding: 0; display: flex; flex-direction: column; gap: 1px; }
.quest li { display: grid; grid-template-columns: 1fr auto; gap: 0 8px; font-size: 12px; }
.quest .reward { color: var(--muted); font-size: 11px; text-align: right; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 120px; }
.quest .done { color: var(--muted); opacity: 0.55; } .quest .done .step::before { content: "✓ "; color: var(--green); }
.quest .later { color: var(--muted); opacity: 0.7; }
.quest .next { color: var(--text); font-weight: 600; background: rgba(251, 191, 36, 0.08); border-radius: 6px; margin: 1px -6px; padding: 2px 6px; } .quest .next .reward { color: var(--amber); }
.bar { grid-column: 1 / -1; display: flex; align-items: center; gap: 6px; font-size: 11px; font-weight: 400; color: var(--muted); }
.bar span { flex: 1; height: 4px; border-radius: 2px; background: var(--line); overflow: hidden; }
.bar i { display: block; height: 100%; background: var(--amber); }
.disk { display: flex; gap: 14px; align-items: center; min-height: 0; }
.donut { position: relative; width: 118px; height: 118px; flex: none; }
.donut svg { width: 100%; height: 100%; transform: rotate(-90deg); }
.donut circle { fill: none; stroke-width: 0.3; }
.donut .center { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; text-align: center; font-size: 11px; color: var(--muted); }
.donut .center b { color: var(--green); font-size: 20px; font-weight: 600; line-height: 1.1; }
.legend { list-style: none; margin: 0; padding: 0; font-size: 12px; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.legend li { display: flex; align-items: baseline; gap: 6px; min-width: 0; }
.swatch { width: 8px; height: 8px; border-radius: 2px; flex: none; }
.ranking li { display: grid; grid-template-columns: auto 1fr auto; gap: 0 6px; font-size: 12px; min-width: 0; padding: 3px 0; border-top: 1px solid rgba(255, 255, 255, 0.05); }
.ranking li:first-child { border-top: 0; }
.ranking .swatch { align-self: center; }
.name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.legend .name { flex: 1; }
.figure { font-variant-numeric: tabular-nums; font-weight: 600; white-space: nowrap; }
.meta { grid-column: 2 / -1; color: var(--muted); font-size: 11px; }
.empty { color: var(--muted); }
"""


def figure_text(value, digits, words):
    return "-" if value is None else f"{value:,.{digits}f}".replace(",", " ").replace(".", words["decimal"])


def size_text(value, words, digits=1):
    return f"{figure_text(value, digits, words)} {words['gb']}"


def window_of(history):
    end = datetime.date.fromisoformat(history[-1]["day"])
    start = end - datetime.timedelta(days=HISTORY_DAYS - 1)
    entries = [e for e in history if e["day"] >= start.isoformat()]
    offsets = [(datetime.date.fromisoformat(e["day"]) - start).days for e in entries]
    return start, end, entries, offsets


def x_of(offset):
    return (offset + 0.5) * 100 / HISTORY_DAYS


def y_of(value, top):
    return 100 - 90 * value / top


def top_of(values):
    found = max([v for v in values if v is not None] or [0])
    return found if found > 0 else 1


def lines(css, offsets, values, top):
    parts, run = [], []
    for at, value in [*zip(offsets, values), (None, None)]:
        if run and (value is None or at > run[-1][0] + 1):
            if len(run) == 1:
                x, y = x_of(run[0][0]), y_of(run[0][1], top)
                parts.append(f'<line class="dot {css}" x1="{x:.2f}" y1="{y:.2f}" x2="{x:.2f}" y2="{y:.2f}"/>')
            else:
                points = " ".join(f"{x_of(a):.2f},{y_of(v, top):.2f}" for a, v in run)
                base = f"{x_of(run[-1][0]):.2f},100 {x_of(run[0][0]):.2f},100"
                parts.append(f'<polygon class="area {css}" points="{points} {base}"/>')
                parts.append(f'<polyline class="line {css}" points="{points}"/>')
            run = []
        if value is not None:
            run.append((at, value))
    return parts


def bars(css, offsets, values, top):
    width = 100 / HISTORY_DAYS * 0.6
    return [
        f'<rect class="{css}" x="{x_of(at) - width / 2:.2f}" y="{y_of(v, top):.2f}" '
        f'width="{width:.2f}" height="{100 - y_of(v, top):.2f}"/>'
        for at, v in zip(offsets, values)
        if v and v > 0
    ]


def chart(caption, keys, shapes, marks, start, end):
    svg = '<svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">' + "".join(shapes) + "</svg>"
    legend = "".join(f'<span class="key {css}">{html.escape(text)}</span>' for css, text in keys)
    labels = "".join(
        f'<span class="mark {side}" style="top:{top:.1f}%">{html.escape(text)}</span>' for side, top, text in marks
    )
    return (
        f"<figure><figcaption>{html.escape(caption)}{legend}</figcaption>"
        f'<div class="chart">{svg}{labels}</div>'
        f'<div class="axis"><span>{start.strftime("%d/%m")}</span><span>{end.strftime("%d/%m")}</span></div></figure>'
    )


def kpi(css, label, value, sub=""):
    return (
        f'<div class="kpi {css}"><span class="label">{html.escape(label)}</span>'
        f'<span class="value">{html.escape(value)}</span><span class="sub">{html.escape(sub)}</span></div>'
    )


def trends(history, words):
    start, end, entries, offsets = window_of(history)
    today = history[-1]
    paid = today.get("zen_paid")
    floor = today["floor"]
    actual = today.get("actual_gb")
    strip = "".join([
        kpi("ratio", words["ratio"], figure_text(today.get("ratio"), 2, words),
            words["floor"].format(v=figure_text(floor, 1, words))),
        kpi("left", words["left"], size_text(today.get("left_gb"), words)),
        kpi("up", words["uploaded"], size_text(today.get("uploaded_gb"), words),
            "" if actual is None else words["actual"].format(v=size_text(actual, words))),
        kpi("down", words["downloaded"], size_text(today.get("downloaded_gb"), words)),
        kpi("zen", words["zen"], figure_text(today.get("zen"), 0, words),
            words["paid_last"].format(v=figure_text(paid, 0, words)) if paid else ""),
    ])
    ratios = [e.get("ratio") for e in entries]
    top = top_of(ratios + [floor]) * 1.05
    ratio_chart = chart(
        words["ratio"], [],
        [f'<line class="guide" x1="0" y1="{y_of(floor, top):.2f}" x2="100" y2="{y_of(floor, top):.2f}"/>',
         *lines("c-ratio", offsets, ratios, top)],
        [*([("left", y_of(top_of(ratios), top), figure_text(top_of(ratios), 2, words))] if any(ratios) else []),
         ("right", y_of(floor, top), words["floor"].format(v=figure_text(floor, 1, words)))],
        start, end,
    )
    paid_values = [e.get("zen_paid") for e in entries]
    balance = [e.get("zen") for e in entries]
    zen_chart = chart(
        words["zen_chart"], [("c-paid", words["paid"]), ("c-balance", words["balance"])],
        bars("c-paid", offsets, paid_values, top_of(paid_values)) + lines("c-balance", offsets, balance, top_of(balance)),
        [("left", 10, figure_text(top_of(paid_values), 0, words)), ("right", 10, figure_text(top_of(balance), 0, words))],
        start, end,
    )
    up = [e.get("uploaded_gb") for e in entries]
    down = [e.get("downloaded_gb") for e in entries]
    both = top_of(up + down)
    transfer_chart = chart(
        words["transfer"], [("c-up", words["uploaded"]), ("c-down", words["downloaded"])],
        lines("c-up", offsets, up, both) + lines("c-down", offsets, down, both),
        [("left", 10, size_text(both, words, 0))],
        start, end,
    )
    return (
        f'<section class="kpis">{strip}</section>'
        f'<section class="charts">{ratio_chart}{zen_chart}{transfer_chart}</section>'
    )


def short_step(key, index, target, words):
    text = words["short"][key]
    if key == "duration":
        return text.format(n=target, days=DURATION_STEPS[index][1])
    if key == "contribution":
        return text.format(n=target, share=CONTRIBUTION_SHARES[index].replace(".", words["decimal"]))
    if key == "diversity":
        return text[0 if index == 0 else 1].format(k=index + 1)
    if key == "category":
        return text[0 if index < 2 else 1].format(n=target)
    return text.format(n=target)


def quest_column(challenge, words, pick):
    key = challenge["key"]
    rows, waiting = [], False
    for index, step in enumerate(challenge["steps"]):
        reached = step["progress"] >= step["target"]
        status = "done" if reached else "later" if waiting else "next"
        waiting = waiting or not reached
        bar = ""
        if status == "next":
            fill = min(100, 100 * step["progress"] / step["target"])
            bar = f'<span class="bar"><span><i style="width:{fill:.0f}%"></i></span>{step["progress"]} / {step["target"]}</span>'
        rows.append(
            f'<li class="{status}"><span class="step">{html.escape(short_step(key, index, step["target"], words))}</span>'
            f'<span class="reward" title="{html.escape(REWARDS[key][index][pick])}">{html.escape(REWARDS[key][index][pick])}</span>{bar}</li>'
        )
    if challenge["max"]:
        rows.append(
            f'<li class="{"later" if waiting else "done"}"><span class="step">{html.escape(words["max"])}</span>'
            f'<span class="reward">{html.escape(MAX_REWARDS[key][pick])}</span></li>'
        )
    return f'<div class="quest"><h3>{html.escape(words["challenges"][key])}</h3><ol>{"".join(rows)}</ol></div>'


def donut(farm, words):
    budget = farm["budget_gb"] or 1
    used = sum(s["gb"] for s in farm["slices"])
    start, rings, legend = 0, [], []
    for s in farm["slices"]:
        part = 100 * s["gb"] / max(budget, used)
        color = COLORS.get(s["source"], COLORS["unknown"])
        rings.append(
            f'<circle r="0.84" pathLength="100" stroke="{color}" '
            f'stroke-dasharray="{part:.2f} {100 - part:.2f}" stroke-dashoffset="{-start:.2f}"/>'
        )
        legend.append(
            f'<li><span class="swatch" style="background:{color}"></span>'
            f'<span class="name">{html.escape(words["sources"].get(s["source"], s["source"]))}</span>'
            f'<span class="figure">{html.escape(size_text(s["gb"], words))}</span></li>'
        )
        start += part
    legend.append(
        f'<li><span class="swatch" style="background:var(--line)"></span><span class="name">{html.escape(words["free"])}</span>'
        f'<span class="figure">{html.escape(size_text(max(budget - used, 0), words))}</span></li>'
    )
    center = words["of"].format(budget=figure_text(budget, 0, words), gb=words["gb"])
    svg = (
        '<svg viewBox="-1 -1 2 2" aria-hidden="true"><circle r="0.84" stroke="rgba(148,163,184,0.22)"/>'
        + "".join(rings) + "</svg>"
    )
    return (
        f'<figure><figcaption>{html.escape(words["disk"])}</figcaption><div class="disk">'
        f'<div class="donut">{svg}<div class="center"><b>{figure_text(100 * used / budget, 0, words)} %</b>'
        f'{html.escape(center)}</div></div><ul class="legend">{"".join(legend)}</ul></div></figure>'
    )


def card_text(c, words):
    return words["card"].format(
        ratio=figure_text(c["ratio"], 2, words), note=figure_text(c["note"], 2, words),
        size=size_text(c["size_gb"], words, 2), days=words["days"].format(n=c["days"]),
    )


def per_day_text(c, words):
    return words["per_day"].format(v=figure_text(c["per_day"], 3, words))


def ranking(title, cards, words):
    rows = "".join(
        f'<li><span class="swatch" style="background:{COLORS.get(c["source"], COLORS["unknown"])}"></span>'
        f'<span class="name" title="{html.escape(c["name"])}">{html.escape(c["name"])}</span>'
        f'<span class="figure">{html.escape(per_day_text(c, words))}</span>'
        f'<span class="meta">{html.escape(words["sources"].get(c["source"], c["source"]))}, {html.escape(card_text(c, words))}</span></li>'
        for c in cards
    )
    return f'<figure><figcaption>{html.escape(title)}</figcaption><ol class="ranking">{rows}</ol></figure>'


def goals(board, words, pick):
    columns = "".join(quest_column(c, words, pick) for c in board["challenges"])
    best = ranking(words["best"], board["best"], words)
    worst = ranking(words["worst"], board["worst"], words)
    return f'<section class="quests">{columns}</section><section class="bottom">{donut(board["farm"], words)}{best}{worst}</section>'


def board_page(board, part=None, language="en"):
    language = language if language in TEXT else "en"
    words = TEXT[language]
    pick = 0 if language == "en" else 1
    if not board or not board["history"]:
        body = f'<p class="empty">{html.escape(words["empty"])}</p>'
    else:
        body = (trends(board["history"], words) if part != "2" else "") + (goals(board, words, pick) if part != "1" else "")
    return (
        f'<!doctype html><html lang="{language}"{"" if part else ' class="standalone"'}><head><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width, initial-scale=1">'
        f"<title>Farm</title><style>{STYLE}</style></head><body><main>{body}</main></body></html>"
    )


def serve(latest, port=8000):
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            url = urllib.parse.urlsplit(self.path)
            path = url.path.rstrip("/")
            if path == "/graphs":
                query = urllib.parse.parse_qs(url.query)
                page = board_page(latest.get("board"), query.get("part", [None])[0], query.get("lang", ["en"])[0])
                self.answer(page.encode(), "text/html; charset=utf-8", {
                    "Content-Security-Policy": "default-src 'none'; style-src 'unsafe-inline'",
                })
                return
            if path != "/summary" or "summary" not in latest:
                self.send_error(404)
                return
            self.answer(json.dumps(latest["summary"]).encode(), "application/json")

        def answer(self, body, content_type, headers=None):
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            for name, value in (headers or {}).items():
                self.send_header(name, value)
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("", port), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


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
        print(f"{seeding['name']} now seeds on the tracker, its public twin is gone")


def seed_twins(qbt, prowlarr, settings, state, torrents, now):
    for twin in [t for t in state["stuck"] if t not in state["twins"]]:
        del state["stuck"][twin]
    for twin, (original, _, grabbed_at, link) in list(state["twins"].items()):
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
                torrent_file = prowlarr.torrent({"page": "", "link": link})
            except urllib.error.HTTPError as error:
                refused = 400 <= error.code < 500
                raise
            qbt.add(torrent_file, FARM, public["save_path"])
            added = True
            qbt.match_files(original, qbt.files(twin))
        except Throttled:
            raise
        except Exception as error:
            if refused or attempts + 1 >= TWIN_ATTEMPTS:
                if added:
                    qbt.delete([original], delete_files=False)
                del state["twins"][twin]
                state["stuck"].pop(twin, None)
                state["given_up"][original] = now
                print(f"Gave up the tracker copy of {public['name']}, its public copy seeds alone until it leaves: {error}")
                continue
            wait = settings["twin_seed_after"] * 2**attempts
            state["stuck"][twin] = [attempts + 1, now + wait]
            print(f"Cannot seed the tracker copy of {public['name']}, next try in {wait // 60:g} minutes: {error}")
            continue
        state["stuck"].pop(twin, None)
        print(f"cross-seed missed {public['name']}, its tracker copy now checks the same files")


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
            print(f"Moved {t['name']} from {category} to the Farm, it keeps paying Zen")


def migrate(qbt, settings, torrents):
    for t in torrents.values():
        target = LEGACY.get(t["category"])
        if target == FARM:
            into_farm(qbt, t)
        elif target and t["progress"] >= 1 and t["seeding_time"] >= settings["adopt_after"]:
            qbt.keep_in_place([t["hash"]])
            qbt.set_category([t["hash"]], target)
            t["category"] = target
    used = {t["category"] for t in torrents.values()}
    old = [name for name in qbt.categories() if name in LEGACY and name not in used]
    if old:
        qbt.remove_categories(old)
        print(f"Removed the old categories {', '.join(old)} of qBittorrent")


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


def clean_farm(qbt, settings, state, torrents):
    farm = [t for t in torrents.values() if t["category"] == FARM]
    used = sum(t["size"] for t in farm)
    candidates = sorted(
        (
            t
            for t in farm
            if is_c411(state, settings, t) and t["progress"] >= 1 and t["seeding_time"] >= settings["min_seeding"]
        ),
        key=lambda t: (zen_per_gb(t), t["completion_on"]),
    )
    for t in candidates:
        if used <= settings["farm_size"]:
            break
        qbt.delete([t["hash"]])
        del torrents[t["hash"]]
        used -= t["size"]
        print(f"Removed {t['name']}, the Farm is full and it pays the least Zen for its size")
    if used > settings["farm_size"]:
        print(f"The Farm holds {used / GB:.1f} GB, over {settings['farm_size'] / GB:g} GB, the rest is still downloading or short of its Seeding goal")


def tag(qbt, settings, state, torrents):
    wanted = {}
    for t in torrents.values():
        if is_owned(t) and is_public(state, settings, t):
            name = PUBLIC
        elif is_c411(state, settings, t) and t["hash"] in state["sources"]:
            name = state["sources"][t["hash"]]
        else:
            continue
        if name not in {x.strip() for x in t.get("tags", "").split(",")}:
            wanted.setdefault(name, []).append(t["hash"])
    for name, hashes in wanted.items():
        qbt.add_tags(hashes, name)


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


def fetched(prowlarr, release, cache):
    key = ("torrent", release["infohash"])
    if key not in cache:
        cache[key] = prowlarr.torrent(release)
    return cache[key]


def find_twin(prowlarr, settings, release, now, cache):
    episode, language = episode_of(release["title"]), language_of(release["title"])
    if not episode or not language:
        return None
    for indexer in settings["twin_indexers"]:
        query = (indexer, f"{episode} {language}")
        if query not in cache:
            cache[query] = prowlarr.search(indexer, query[1], now)
        for candidate in cache[query]:
            if may_be_twin(release, candidate):
                twin_torrent = fetched(prowlarr, candidate, cache)
                if file_lengths(twin_torrent) == file_lengths(fetched(prowlarr, release, cache)):
                    return {**candidate, "torrent": twin_torrent}
    return None


def fetch(settings, read, what="indexer"):
    releases = []
    for indexer in settings["indexers"]:
        try:
            releases += read(indexer)
        except Throttled:
            raise
        except Exception as error:
            print(f"Cannot read {what} {indexer} from Prowlarr: {error}")
    return releases


def weigh_floor(state, account, settings):
    paused = is_under_floor(account, settings)
    if paused and not state["paused"]:
        print(f"Paused: the Account ratio is unknown or under {settings['floor']:g}, ratio spends no download on C411")
    if state["paused"] and not paused:
        print(f"Resumed: the Account ratio is back over {settings['floor']:g}")
    state["paused"] = paused


def hold_c411(prowlarr, settings, state):
    paused = state["paused"]
    if state["held"] == paused or not settings["indexers"]:
        return
    try:
        for indexer in settings["indexers"]:
            before = prowlarr.hold(indexer, paused, settings["paused_profile"], state["restore"].get(indexer))
            if paused and before is not None:
                state["restore"][indexer] = before
    except Throttled:
        raise
    except Exception as error:
        print(f"Cannot {'hold' if paused else 'release'} C411 in Prowlarr: {error}")
        return
    state["held"] = paused
    print(f"C411 is {'held' if paused else 'open again'} for Radarr, Sonarr and Lidarr")


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
    budget[FARM] -= release["size"]
    budget["slots"] -= 1
    print(
        f"Grabbed {release['title']} for the Farm from {via}, "
        f"{release['seeders']} seeders, {release['leechers']} leechers, {release['size'] / GB:.1f} GB"
    )


def grab_twins(qbt, prowlarr, settings, state, releases, known, budget, now):
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
            twin = find_twin(prowlarr, settings, release, now, searches)
            if twin:
                take(qbt, state, budget, known, release, twin["torrent"], "twin", f"its public twin {twin['title']}")
                state["twins"][twin["infohash"]] = [release["infohash"], FARM, now, release["link"]]
                state["kinds"][release["infohash"]] = FILMS
        except Throttled:
            raise
        except Exception as error:
            print(f"Cannot look for a twin of {release['title']}: {error}")


def take_from_tracker(qbt, prowlarr, state, budget, known, release, source, via):
    try:
        take(qbt, state, budget, known, release, prowlarr.torrent(release), source, via)
    except Throttled:
        raise
    except Exception as error:
        print(f"Cannot grab {release['title']}: {error}")
        return False
    return True


def grab_free(qbt, prowlarr, settings, state, releases, known, budget, now):
    for release in pick_free(releases, known, settings["free_max_size"]):
        if budget["slots"] <= 0:
            return
        if release["infohash"] in known or release["size"] > budget[FARM]:
            continue
        if take_from_tracker(qbt, prowlarr, state, budget, known, release, "freeleech", "the tracker, freeleech"):
            state["free"][release["infohash"]] = now


def hunt_target(settings, state, counts, now):
    short = [name for name in QUEST_CATEGORIES if counts[name] < settings["volume"]["per_category"]]
    if not short:
        return FILMS, ANIME, False
    kind = min(short, key=lambda name: state["hunt_at"].get(name, 0))
    return kind, QUEST_CATEGORIES[kind], True


def hunt_volume(qbt, prowlarr, settings, state, torrents, known, budget, now):
    rules = settings["volume"]
    held = [state["kinds"].get(t["hash"]) for t in torrents.values() if is_c411(state, settings, t)]
    counts = {name: held.count(name) for name in QUEST_CATEGORIES}
    kind, cats, filling = hunt_target(settings, state, counts, now)
    releases = fetch(settings, lambda indexer: prowlarr.category(indexer, cats, now), f"{kind} of indexer")
    if filling:
        state["hunt_at"][kind] = now
    for release in pick_small(releases, known, rules["max_size"]):
        if budget["slots"] <= 0 or (filling and counts[kind] >= rules["per_category"]):
            return
        if release["infohash"] in known or release["size"] > budget[FARM]:
            continue
        if not release["free"] and release["size"] > budget["download"]:
            continue
        free = release["free"]
        source, via = ("freeleech", "the tracker, freeleech") if free else ("volume-hunt", "the tracker")
        if not take_from_tracker(qbt, prowlarr, state, budget, known, release, source, via):
            continue
        state["kinds"][release["infohash"]] = kind
        counts[kind] += 1
        if free:
            state["free"][release["infohash"]] = now
        else:
            state["hunted"][release["infohash"]] = [now, release["size"]]
            budget["download"] -= release["size"]


def pending(state, torrents):
    return sum(
        t["size"] * (1 - t["progress"])
        for t in torrents.values()
        if t["hash"] in state["transfers"] and t["hash"] not in state["free"] and t["progress"] < 1
    )


def grab(qbt, prowlarr, settings, state, torrents, account, now):
    downloading = sum(1 for t in torrents.values() if t["category"] == FARM and t["progress"] < 1)
    spent = sum(size for _, size in state["hunted"].values())
    budget = {
        "slots": settings["max_downloading"] - downloading,
        FARM: settings["farm_size"] - size_of(torrents, FARM),
        "download": 0 if state["paused"] else min(
            download_left(account, settings) - pending(state, torrents), settings["volume"]["daily"] - spent
        ),
    }
    if budget["slots"] <= 0 or not settings["indexers"]:
        return
    known = set(torrents) | twin_copies(state) | set(state["given_up"])
    releases = fetch(settings, lambda indexer: prowlarr.newest(indexer, now))
    grab_twins(qbt, prowlarr, settings, state, releases, known, budget, now)
    grab_free(qbt, prowlarr, settings, state, releases, known, budget, now)
    hunt_volume(qbt, prowlarr, settings, state, torrents, known, budget, now)


def run_once(qbt, prowlarr, c411, settings, state, now):
    prepare(qbt, settings)
    account = read_account(c411, state, now)
    weigh_floor(state, account, settings)
    torrents = {t["hash"].lower(): {**t, "hash": t["hash"].lower()} for t in qbt.torrents()}
    record(state, torrents, settings, now)
    classify(qbt, state, settings, torrents)
    report(state, now)
    use_prowlarr(state, now, lambda: hold_c411(prowlarr, settings, state), always=True)
    adopt(qbt, settings, state, torrents)
    migrate(qbt, settings, torrents)
    drop_twins(qbt, state, torrents)
    clean_public(qbt, settings, state, torrents)
    clean_farm(qbt, settings, state, torrents)
    tag(qbt, settings, state, torrents)
    use_prowlarr(state, now, lambda: grab(qbt, prowlarr, settings, state, torrents, account, now))
    use_prowlarr(state, now, lambda: seed_twins(qbt, prowlarr, settings, state, torrents, now))
    torrents = {t["hash"].lower(): {**t, "hash": t["hash"].lower()} for t in qbt.torrents()}
    record_day(state, settings, torrents, account, now)
    return {**summary(state, torrents, settings, account, now), "board": board_of(state, settings, torrents)}


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
        "indexers": ids("RATIO_GRAB_INDEXER_IDS"),
        "twin_indexers": ids("RATIO_TWIN_INDEXER_IDS"),
        "cross_seed_key": os.environ.get("CROSS_SEED_API_KEY", ""),
        "farm_size": positive("RATIO_MAX_SIZE_GB", 100) * GB,
        "floor": positive("RATIO_FLOOR", 1.2),
        "paused_profile": "Paused",
        "volume": {"max_size": GB, "per_category": 25, "daily": number("RATIO_VOLUME_DAILY_GB", 2) * GB},
        "free_max_size": 5 * GB,
        "min_seeding": number("RATIO_MIN_SEEDING_DAYS") * DAY,
        "public_seeding": 14 * DAY,
        "max_downloading": 2,
        "twin_lookups": 10,
        "twin_max_age_hours": 24,
        "twin_min_leechers": 1,
        "twin_recheck": 2 * 3600,
        "twin_seed_after": 3600,
        "library_categories": ("movies", "shows", "music", "books"),
        "adopt_after": 6 * 3600,
        "tracker_seeders_in_peers": 2,
    }


def main():
    settings = settings_from_env()
    qbt = QBittorrent(
        "http://gluetun:8080",
        os.environ.get("QBITTORRENT_USER", ""),
        os.environ.get("QBITTORRENT_PASSWORD", ""),
    )
    prowlarr = Prowlarr(
        "http://prowlarr:9696",
        os.environ.get("PROWLARR_API_KEY", ""),
        settings["tracker_seeders_in_peers"],
    )
    c411 = C411(os.environ.get("C411_PROFILE_API_KEY", ""))
    state_path = "/state/state.json"
    state = load_state(state_path)
    latest = {"board": waiting_board(state, settings)}
    serve(latest)
    if not settings["indexers"]:
        print("RATIO_GRAB_INDEXER_IDS is empty, ratio grabs nothing and only cleans")
    while True:
        try:
            board_and_summary = run_once(qbt, prowlarr, c411, settings, state, time.time())
            latest["board"] = board_and_summary.pop("board")
            latest["summary"] = board_and_summary
        except Exception as error:
            print(f"Cannot run the Farm: {error}")
            latest["board"] = {**latest["board"], "history": [dict(e) for e in state["daily"]]}
        try:
            save_state(state_path, state)
        except OSError as error:
            print(f"Cannot save {state_path}: {error}")
        time.sleep(600)


if __name__ == "__main__":
    main()
