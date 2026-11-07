import datetime
import email.utils
import http.cookiejar
import http.server
import json
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
FARM = ("ratio", "cross-seed-link")
ZEN = "zen"
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


class Prowlarr:
    def __init__(self, url, api_key, tracker_seeders_in_peers):
        self.url = url.rstrip("/")
        self.api_key = api_key
        self.tracker_seeders_in_peers = tracker_seeders_in_peers

    def newest(self, indexer, now):
        fields = {"t": "search", "cat": "2000,5000", "limit": 100}
        return self.query(indexer, fields, now, self.tracker_seeders_in_peers)

    def search(self, indexer, title, now):
        return self.query(indexer, {"t": "search", "q": re.sub(r"[._]+", " ", title), "limit": 100}, now, 1)

    def torrent(self, release):
        page = re.search(r"nyaa\.si/view/(\d+)", release["page"])
        link = f"https://nyaa.si/download/{page.group(1)}.torrent" if page else release["link"]
        request = urllib.request.Request(link, headers={"User-Agent": "Mozilla/5.0 (compatible; jellynas-ratio)"})
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.read()

    def query(self, indexer, fields, now, seeders_in_peers):
        query = urllib.parse.urlencode({**fields, "apikey": self.api_key})
        with urllib.request.urlopen(f"{self.url}/{indexer}/api?{query}", timeout=60) as response:
            return parse_torznab(response.read(), now, seeders_in_peers)


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


def hunger(release):
    return release["leechers"] / max(release["seeders"], 1)


def pick_hungry(releases, known, rules):
    hungry = [
        r
        for r in releases
        if r["infohash"] not in known
        and rules["min_size"] <= r["size"] <= rules["max_size"]
        and r["age_hours"] <= rules["max_age_hours"]
        and r["seeders"] >= 1
        and r["leechers"] >= rules["min_leechers"]
        and hunger(r) >= rules["min_leechers_per_seeder"]
    ]
    return sorted(hungry, key=lambda r: (hunger(r), r["leechers"]), reverse=True)


def pick_rare(releases, known, rules):
    rare = [
        r
        for r in releases
        if r["infohash"] not in known
        and rules["min_size"] <= r["size"] <= rules["max_size"]
        and r["seeders"] >= 1
        and is_rare(r["size"], r["seeders"])
    ]
    return sorted(rare, key=lambda r: (r["seeders"], -r["leechers"]))


def pick_evictions(torrents, limit_bytes, min_seeding_seconds):
    used = sum(t["size"] for t in torrents)
    candidates = sorted(
        (t for t in torrents if t["progress"] >= 1 and t["seeding_time"] >= min_seeding_seconds),
        key=lambda t: (t["uploaded"] / max(t["seeding_time"], 1), t["completion_on"]),
    )
    evicted = []
    for t in candidates:
        if used <= limit_bytes:
            break
        evicted.append(t)
        used -= t["size"]
    return evicted


def uploaded_since(samples, uploaded, since):
    older = [u for at, u in samples if at <= since]
    return None if not older else uploaded - older[-1]


def is_stale(torrent, samples, settings, now):
    if torrent["progress"] < 1 or torrent["seeding_time"] < settings["min_seeding"]:
        return False
    recent = uploaded_since(samples, torrent["uploaded"], now - settings["stale_after"])
    return recent is not None and recent < settings["stale_upload"] * torrent["size"]


def deficit(transfers):
    return sum(downloaded - uploaded for downloaded, uploaded in transfers.values())


def size_of(torrents, categories):
    return sum(t["size"] for t in torrents.values() if t["category"] in categories)


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
        "uploads": state.get("uploads", {}),
        "free": state.get("free", {}),
        "checked": state.get("checked", {}),
        "paused": state.get("paused", False),
        "sources": state.get("sources", {}),
        "history": state.get("history", {}),
        "zen": state.get("zen", {}),
        "report": state.get("report", {}),
    }


def save_state(path, state):
    temporary = f"{path}.tmp"
    with open(temporary, "w", encoding="utf-8") as file:
        json.dump(state, file)
    os.replace(temporary, path)


def prepare(qbt, settings):
    categories = qbt.categories()
    for name in ("ratio", ZEN):
        if name not in categories:
            qbt.create_category(name, f"/data/torrents/{name}")
    if not settings["cross_seed_key"]:
        return
    hook = (
        "curl -XPOST http://cross-seed:2468/api/webhook "
        f'-H "X-Api-Key: {settings["cross_seed_key"]}" -d "infoHash=%I" -d "includeSingleEpisodes=true"'
    )
    preferences = qbt.preferences()
    if not preferences.get("autorun_enabled") or preferences.get("autorun_program") != hook:
        qbt.set_preferences({"autorun_enabled": True, "autorun_program": hook})
        print("qBittorrent now tells cross-seed about every finished torrent")


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
    if t["category"] == "cross-seed-link":
        return "cross-seed"
    if t["category"] in settings["library_categories"]:
        return "library"
    if t["category"] == ZEN:
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
            if t["category"] == ZEN:
                state["zen"][infohash] = [seeding, 0]
            continue
        if t["category"] == ZEN:
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
        if t["category"] in FARM:
            samples = state["uploads"].setdefault(t["hash"], [])
            if not samples or now - samples[-1][0] >= DAY:
                samples.append([now, t["uploaded"]])
    for infohash in list(state["uploads"]):
        if infohash not in torrents:
            del state["uploads"][infohash]
            continue
        keep_after = now - settings["stale_after"] - DAY
        state["uploads"][infohash] = [s for s in state["uploads"][infohash] if s[0] >= keep_after]
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
            "zen_days": round(zen / DAY, 1),
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
            f"{f['week_uploaded_gb']:.1f} GB up, ratio {shown(f['week_ratio'])}, {f['zen_days']:.1f} days in the Zen reserve"
        )


def fill_of(torrents, categories, limit):
    held = [t for t in torrents.values() if t["category"] in categories]
    size = sum(t["size"] for t in held)
    return {
        "torrents": len(held),
        "downloading": sum(1 for t in held if t["progress"] < 1),
        "size_gb": gigabytes(size),
        "limit_gb": gigabytes(limit),
        "fill": round(100 * size / limit, 1) if limit else 0,
    }


def summary(state, torrents, settings, now):
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
        "farm": fill_of(torrents, FARM, settings["farm_size"]),
        "zen": fill_of(torrents, (ZEN,), settings["zen_size"]),
        "grabs": {"today": len(state["grabbed"]), "per_day": settings["per_day"], "paused": state["paused"]},
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
    for twin, (original, category, *_) in list(state["twins"].items()):
        if twin not in torrents:
            del state["twins"][twin]
            continue
        seeding = torrents.get(original)
        if seeding is None or seeding["progress"] < 1:
            continue
        if seeding["state"].startswith("stopped"):
            qbt.start([original])
        if category == ZEN and seeding["category"] != ZEN:
            qbt.set_category([original], ZEN)
            seeding["category"] = ZEN
        qbt.delete([twin], seeding["save_path"] != torrents[twin]["save_path"])
        del torrents[twin]
        del state["twins"][twin]
        print(f"{seeding['name']} now seeds on the tracker, its public twin is gone")


def seed_twins(qbt, prowlarr, settings, state, torrents, now):
    for twin, (original, category, grabbed_at, link) in list(state["twins"].items()):
        public = torrents.get(twin)
        if original in torrents or public is None or public["progress"] < 1:
            continue
        if now - grabbed_at < settings["twin_seed_after"]:
            continue
        try:
            qbt.add(prowlarr.torrent({"page": "", "link": link}), category, public["save_path"])
            qbt.match_files(original, qbt.files(twin))
        except Exception as error:
            print(f"Cannot seed the tracker copy of {public['name']}: {error}")
            continue
        print(f"cross-seed missed {public['name']}, its tracker copy now checks the same files")


def retire(qbt, settings, state, torrents, t, reason):
    room = settings["zen_size"] - size_of(torrents, (ZEN,))
    if t["hash"] in state["transfers"] and is_rare(t["size"], t["num_complete"]) and t["size"] <= room:
        qbt.set_category([t["hash"]], ZEN)
        t["category"] = ZEN
        print(f"Moved {t['name']} to the Zen reserve, {reason}, but only {t['num_complete']} seeders keep it")
        return
    qbt.delete([t["hash"]])
    del torrents[t["hash"]]
    print(f"Removed {t['name']}, {reason}")


def clean_farm(qbt, settings, state, torrents, now):
    for t in [t for t in torrents.values() if t["category"] in FARM]:
        if is_stale(t, state["uploads"].get(t["hash"], []), settings, now):
            retire(qbt, settings, state, torrents, t, "it uploaded too little this week")
    farm = [t for t in torrents.values() if t["category"] in FARM]
    for t in pick_evictions(farm, settings["farm_size"], settings["min_seeding"]):
        retire(qbt, settings, state, torrents, t, "the farm is full and it uploads the least")
    used = size_of(torrents, FARM)
    if used > settings["farm_size"]:
        print(f"The farm holds {used / GB:.1f} GB, over {settings['farm_size'] / GB:g} GB, the rest is still downloading or too young")


def adopt(qbt, settings, state, torrents):
    room = settings["zen_size"] - size_of(torrents, (ZEN,))
    for t in torrents.values():
        if (
            t["category"] in settings["library_categories"]
            and t["hash"] in state["transfers"]
            and t["progress"] >= 1
            and t["seeding_time"] >= settings["adopt_after"]
            and is_rare(t["size"], t["num_complete"])
            and t["size"] <= room
        ):
            qbt.set_category([t["hash"]], ZEN)
            t["category"] = ZEN
            room -= t["size"]
            print(f"Moved {t['name']} from {settings['library_categories']} to the Zen reserve, {t['num_complete']} seeders")


def clean_zen(qbt, settings, torrents):
    zen = [t for t in torrents.values() if t["category"] == ZEN]
    seeded = [t for t in zen if t["progress"] >= 1 and t["seeding_time"] >= settings["min_seeding"]]
    for t in seeded:
        if not is_rare(t["size"], t["num_complete"]):
            qbt.delete([t["hash"]])
            del torrents[t["hash"]]
            print(f"Removed {t['name']} from the Zen reserve, {t['num_complete']} seeders keep it now")
    used = size_of(torrents, (ZEN,))
    for t in sorted((t for t in seeded if t["hash"] in torrents), key=lambda t: -t["num_complete"]):
        if used <= settings["zen_size"]:
            break
        qbt.delete([t["hash"]])
        del torrents[t["hash"]]
        used -= t["size"]
        print(f"Removed {t['name']} from the Zen reserve, it is full and this one is the least rare")


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


def find_twin(prowlarr, settings, release, now, searches):
    episode, language = episode_of(release["title"]), language_of(release["title"])
    if not episode or not language:
        return None
    for indexer in settings["twin_indexers"]:
        query = (indexer, f"{episode} {language}")
        if query not in searches:
            searches[query] = prowlarr.search(indexer, query[1], now)
        for candidate in searches[query]:
            if may_be_twin(release, candidate):
                twin_torrent = prowlarr.torrent(candidate)
                if file_lengths(twin_torrent) == file_lengths(prowlarr.torrent(release)):
                    return {**candidate, "torrent": twin_torrent}
    return None


def fetch_newest(prowlarr, settings, now):
    releases = []
    for indexer in settings["indexers"]:
        try:
            releases += prowlarr.newest(indexer, now)
        except Exception as error:
            print(f"Cannot read indexer {indexer} from Prowlarr: {error}")
    return releases


def may_pay(state, settings, category):
    if len(state["grabbed"]) >= settings["per_day"]:
        return False
    owed = deficit(state["transfers"])
    if category == ZEN:
        return -owed >= settings["zen_hunt_surplus"]
    if owed > settings["max_deficit"]:
        if not state["paused"]:
            print(f"Paused: the tracker torrents downloaded {owed / GB:.1f} GB more than they uploaded")
        state["paused"] = True
        return False
    if state["paused"]:
        print(f"Resumed: the deficit is down to {owed / GB:.1f} GB")
    state["paused"] = False
    return True


def wishes(releases, known, settings, torrents):
    hungry = [(r, "ratio") for r in pick_hungry(releases, known, settings["rules"])]
    rare = [(r, ZEN) for r in pick_rare(releases, known | {r["infohash"] for r, _ in hungry}, settings["rules"])]
    zen_count = sum(1 for t in torrents.values() if t["category"] == ZEN)
    if zen_count < settings["zen_min_torrents"]:
        return rare + hungry
    return hungry + rare


def take(qbt, state, budget, known, release, torrent, category, source, via):
    qbt.add(torrent, category)
    known.add(release["infohash"])
    state["sources"][release["infohash"]] = source
    budget[category] -= release["size"]
    budget["slots"] -= 1
    print(
        f"Grabbed {release['title']} for the {'Zen reserve' if category == ZEN else 'farm'} from {via}, "
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
        if release["size"] > budget["ratio"]:
            continue
        lookups -= 1
        state["checked"][release["infohash"]] = now
        try:
            twin = find_twin(prowlarr, settings, release, now, searches)
            if twin:
                take(qbt, state, budget, known, release, twin["torrent"], "ratio", "twin", f"its public twin {twin['title']}")
                state["twins"][twin["infohash"]] = [release["infohash"], "ratio", now, release["link"]]
        except Exception as error:
            print(f"Cannot look for a twin of {release['title']}: {error}")


def grab_paid(qbt, prowlarr, settings, state, torrents, releases, known, budget, now):
    for release, category in wishes(releases, known, settings, torrents):
        if budget["slots"] <= 0:
            return
        if release["infohash"] in known or release["size"] > budget[category]:
            continue
        if release["free"]:
            source, via, ledger = "freeleech", "the tracker, freeleech", state["free"]
        elif may_pay(state, settings, category):
            source, via, ledger = "zen-hunt" if category == ZEN else "paid", "the tracker", state["grabbed"]
        else:
            continue
        try:
            take(qbt, state, budget, known, release, prowlarr.torrent(release), category, source, via)
        except Exception as error:
            print(f"Cannot grab {release['title']}: {error}")
            continue
        ledger[release["infohash"]] = now


def grab(qbt, prowlarr, settings, state, torrents, now):
    downloading = sum(1 for t in torrents.values() if t["category"] in FARM + (ZEN,) and t["progress"] < 1)
    budget = {
        "slots": settings["max_downloading"] - downloading,
        "ratio": settings["farm_size"] - size_of(torrents, FARM),
        ZEN: settings["zen_size"] - size_of(torrents, (ZEN,)),
    }
    if budget["slots"] <= 0 or not settings["indexers"]:
        return
    known = set(torrents) | twin_copies(state)
    releases = fetch_newest(prowlarr, settings, now)
    grab_twins(qbt, prowlarr, settings, state, releases, known, budget, now)
    grab_paid(qbt, prowlarr, settings, state, torrents, releases, known, budget, now)


def run_once(qbt, prowlarr, settings, state, now):
    prepare(qbt, settings)
    torrents = {t["hash"].lower(): {**t, "hash": t["hash"].lower()} for t in qbt.torrents()}
    record(state, torrents, settings, now)
    report(state, now)
    drop_twins(qbt, state, torrents)
    seed_twins(qbt, prowlarr, settings, state, torrents, now)
    clean_farm(qbt, settings, state, torrents, now)
    adopt(qbt, settings, state, torrents)
    clean_zen(qbt, settings, torrents)
    grab(qbt, prowlarr, settings, state, torrents, now)
    return summary(state, {t["hash"].lower(): {**t, "hash": t["hash"].lower()} for t in qbt.torrents()}, settings, now)


def number(name, default=None):
    value = os.environ.get(name) or default
    try:
        return float(value)
    except (TypeError, ValueError):
        sys.exit(f"Set {name} in .env to a number, then recreate ratio")


def ids(name):
    return [i.strip() for i in os.environ.get(name, "").split(",") if i.strip()]


def settings_from_env():
    return {
        "tracker": (os.environ.get("RATIO_TRACKER") or "c411").lower(),
        "indexers": ids("RATIO_GRAB_INDEXER_IDS"),
        "twin_indexers": ids("RATIO_TWIN_INDEXER_IDS"),
        "cross_seed_key": os.environ.get("CROSS_SEED_API_KEY", ""),
        "farm_size": number("RATIO_MAX_SIZE_GB") * GB,
        "zen_size": number("RATIO_ZEN_MAX_SIZE_GB", 50) * GB,
        "zen_min_torrents": 15,
        "zen_hunt_surplus": number("RATIO_ZEN_HUNT_MIN_SURPLUS_GB", 20) * GB,
        "min_seeding": number("RATIO_MIN_SEEDING_DAYS") * DAY,
        "stale_after": 7 * DAY,
        "stale_upload": 0.2,
        "max_deficit": number("RATIO_GRAB_MAX_DEFICIT_GB", 5) * GB,
        "per_day": 3,
        "max_downloading": 2,
        "twin_lookups": 10,
        "twin_max_age_hours": 24,
        "twin_min_leechers": 1,
        "twin_recheck": 2 * 3600,
        "twin_seed_after": 3600,
        "library_categories": ("radarr", "tv-sonarr"),
        "adopt_after": 6 * 3600,
        "tracker_seeders_in_peers": 2,
        "rules": {
            "min_size": GB,
            "max_size": 5 * GB,
            "max_age_hours": 1,
            "min_leechers": 10,
            "min_leechers_per_seeder": 5,
        },
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
    state_path = "/state/state.json"
    state = load_state(state_path)
    latest = {}
    serve(latest)
    if not settings["indexers"]:
        print("RATIO_GRAB_INDEXER_IDS is empty, ratio grabs nothing and only cleans")
    while True:
        try:
            latest["summary"] = run_once(qbt, prowlarr, settings, state, time.time())
        except Exception as error:
            print(f"Cannot run the farm and the Zen reserve: {error}")
        try:
            save_state(state_path, state)
        except OSError as error:
            print(f"Cannot save {state_path}: {error}")
        time.sleep(600)


if __name__ == "__main__":
    main()
