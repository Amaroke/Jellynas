import datetime
import email.utils
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
SOURCES = {
    "paid": "paid grab",
    "twin": "Twin",
    "freeleech": "freeleech",
    "cross-seed": "cross-seed of the Library",
    "library": "Library download",
    "zen-hunt": "Zen hunt",
    "unknown": "unknown",
}
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
    def __init__(self, retry_after):
        super().__init__(f"Prowlarr asks to wait {retry_after} seconds")
        self.retry_after = retry_after


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

    def search(self, indexer, title, now):
        return self.query(indexer, {"t": "search", "q": re.sub(r"[._]+", " ", title), "limit": 100}, now, 1)

    def torrent(self, release):
        page = re.search(r"nyaa\.si/view/(\d+)", release["page"])
        link = f"https://nyaa.si/download/{page.group(1)}.torrent" if page else release["link"]
        return self.open(urllib.request.Request(link, headers={"User-Agent": "Mozilla/5.0 (compatible; jellynas-ratio)"}))

    def query(self, indexer, fields, now, seeders_in_peers):
        query = urllib.parse.urlencode({**fields, "apikey": self.api_key})
        return parse_torznab(self.open(urllib.request.Request(f"{self.url}/{indexer}/api?{query}")), now, seeders_in_peers)

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
                raise Throttled(wait_of(error.headers.get("Retry-After"))) from error
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


def pick_rare(releases, known, rules):
    rare = [
        r
        for r in releases
        if r["infohash"] not in known
        and not r["free"]
        and rules["min_size"] <= r["size"] <= rules["max_size"]
        and r["seeders"] >= 1
        and is_rare(r["size"], r["seeders"])
    ]
    return sorted(rare, key=lambda r: (r["seeders"], -r["leechers"]))


def nightly_zen(t):
    days = t["seeding_time"] / DAY
    note = 1 + next((step for at, step in SEEDING_STEPS if days >= at), 0)
    if is_rare(t["size"], t["num_complete"]):
        note += RARE_STEP
    size = t["size"] / 1e9
    weight = (0.25 + size) / 4.25
    return note * (weight if size <= 4 else math.sqrt(weight)) / 7


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
        "grabbed": state.get("grabbed", {}),
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
        "restore": state.get("restore", {}),
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
    for infohash in [h for h, at in state["grabbed"].items() if at < now - DAY]:
        del state["grabbed"][infohash]
    for infohash in [h for h in state["free"] if h not in torrents and h in state["transfers"]]:
        del state["free"][infohash]
    for infohash in [h for h, at in state["checked"].items() if at < now - 2 * DAY]:
        del state["checked"][infohash]
    record_sources(state, torrents, settings)
    record_history(state, torrents, fresh, now)
    record_zen(state, torrents)


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


def farm_of(state, settings, torrents):
    held = [t for t in torrents.values() if t["category"] == FARM]
    size = sum(t["size"] for t in held)
    limit = settings["farm_size"]
    return {
        "seeds": sum(1 for t in torrents.values() if is_c411(state, settings, t) and is_seeding(t)),
        "size_gb": gigabytes(size),
        "budget_gb": gigabytes(limit),
        "fill": round(100 * size / limit, 1) if limit else 0,
        "downloading": sum(1 for t in held if t["progress"] < 1),
    }


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
        "sources": sources,
    }


def serve(latest, port=8000):
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path != "/summary" or "summary" not in latest:
                self.send_error(404)
                return
            body = json.dumps(latest["summary"]).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
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
        try:
            qbt.add(prowlarr.torrent({"page": "", "link": link}), FARM, public["save_path"])
            qbt.match_files(original, qbt.files(twin))
        except Throttled:
            raise
        except Exception as error:
            wait = min(settings["twin_seed_after"] * 2**attempts, DAY)
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


def fetch_newest(prowlarr, settings, now):
    releases = []
    for indexer in settings["indexers"]:
        try:
            releases += prowlarr.newest(indexer, now)
        except Throttled:
            raise
        except Exception as error:
            print(f"Cannot read indexer {indexer} from Prowlarr: {error}")
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
        print(f"Prowlarr asks to slow down, ratio leaves it alone for {throttled.retry_after // 60:g} minutes")


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


def hunt_zen(qbt, prowlarr, settings, state, releases, known, budget, now):
    for release in pick_rare(releases, known, settings["zen_hunt"]):
        if budget["slots"] <= 0 or len(state["grabbed"]) >= settings["zen_hunt_per_day"]:
            return
        if release["infohash"] in known or release["size"] > min(budget[FARM], budget["download"]):
            continue
        if take_from_tracker(qbt, prowlarr, state, budget, known, release, "zen-hunt", "the tracker"):
            state["grabbed"][release["infohash"]] = now
            budget["download"] -= release["size"]


def pending(state, torrents):
    return sum(
        t["size"] * (1 - t["progress"])
        for t in torrents.values()
        if t["hash"] in state["transfers"] and t["hash"] not in state["free"] and t["progress"] < 1
    )


def grab(qbt, prowlarr, settings, state, torrents, account, now):
    downloading = sum(1 for t in torrents.values() if t["category"] == FARM and t["progress"] < 1)
    budget = {
        "slots": settings["max_downloading"] - downloading,
        FARM: settings["farm_size"] - size_of(torrents, FARM),
        "download": 0 if state["paused"] else download_left(account, settings) - pending(state, torrents),
    }
    if budget["slots"] <= 0 or not settings["indexers"]:
        return
    known = set(torrents) | twin_copies(state)
    releases = fetch_newest(prowlarr, settings, now)
    grab_twins(qbt, prowlarr, settings, state, releases, known, budget, now)
    grab_free(qbt, prowlarr, settings, state, releases, known, budget, now)
    hunt_zen(qbt, prowlarr, settings, state, releases, known, budget, now)


def run_once(qbt, prowlarr, c411, settings, state, now):
    prepare(qbt, settings)
    account = read_account(c411, state, now)
    weigh_floor(state, account, settings)
    torrents = {t["hash"].lower(): {**t, "hash": t["hash"].lower()} for t in qbt.torrents()}
    record(state, torrents, settings, now)
    report(state, now)
    use_prowlarr(state, now, lambda: hold_c411(prowlarr, settings, state), always=True)
    adopt(qbt, settings, state, torrents)
    migrate(qbt, settings, torrents)
    drop_twins(qbt, state, torrents)
    use_prowlarr(state, now, lambda: seed_twins(qbt, prowlarr, settings, state, torrents, now))
    clean_public(qbt, settings, state, torrents)
    clean_farm(qbt, settings, state, torrents)
    tag(qbt, settings, state, torrents)
    use_prowlarr(state, now, lambda: grab(qbt, prowlarr, settings, state, torrents, account, now))
    torrents = {t["hash"].lower(): {**t, "hash": t["hash"].lower()} for t in qbt.torrents()}
    return summary(state, torrents, settings, account, now)


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
        "zen_hunt_per_day": 3,
        "zen_hunt": {"min_size": GB, "max_size": 5 * GB},
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
    latest = {}
    serve(latest)
    if not settings["indexers"]:
        print("RATIO_GRAB_INDEXER_IDS is empty, ratio grabs nothing and only cleans")
    while True:
        try:
            latest["summary"] = run_once(qbt, prowlarr, c411, settings, state, time.time())
        except Exception as error:
            print(f"Cannot run the Farm: {error}")
        try:
            save_state(state_path, state)
        except OSError as error:
            print(f"Cannot save {state_path}: {error}")
        time.sleep(600)


if __name__ == "__main__":
    main()
