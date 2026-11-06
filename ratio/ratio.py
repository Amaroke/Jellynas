import email.utils
import http.cookiejar
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import xml.etree.ElementTree as ET

GB = 1024**3
DAY = 86400
FARM = ("ratio", "cross-seed-link")
ZEN = "zen"
TORZNAB = "{http://torznab.com/schemas/2015/feed}attr"


class QBittorrent:
    def __init__(self, url, username, password):
        self.url = url.rstrip("/")
        self.username = username
        self.password = password
        self.opener = None

    def torrents(self):
        return json.loads(self.send("/api/v2/torrents/info"))

    def add(self, torrent, category):
        boundary = uuid.uuid4().hex
        body = b"".join(
            [
                f'--{boundary}\r\nContent-Disposition: form-data; name="category"\r\n\r\n{category}\r\n'.encode(),
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

    def delete(self, hashes):
        self.post("/api/v2/torrents/delete", {"hashes": "|".join(hashes), "deleteFiles": "true"})

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

    def download(self, link):
        with urllib.request.urlopen(link, timeout=60) as response:
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
                "size": int(item.findtext("size") or 0),
                "seeders": seeders,
                "leechers": max(peers - seeders_in_peers * seeders, 0),
                "age_hours": (now - published) / 3600,
                "free": attrs.get("downloadvolumefactor") == "0",
            }
        )
    return releases


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
        "paused": state.get("paused", False),
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


def record(state, torrents, settings, now):
    for t in torrents.values():
        if t["hash"] in state["transfers"] or settings["tracker"] in t["tracker"].lower():
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


def drop_twins(qbt, state, torrents):
    for twin, (original, category) in list(state["twins"].items()):
        if twin not in torrents:
            del state["twins"][twin]
            continue
        seeding = torrents.get(original)
        if seeding is None or seeding["progress"] < 1:
            continue
        if category == ZEN and seeding["category"] != ZEN:
            qbt.set_category([original], ZEN)
            seeding["category"] = ZEN
        qbt.delete([twin])
        del torrents[twin]
        del state["twins"][twin]
        print(f"{seeding['name']} now seeds on the tracker, its public twin is gone")


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


def find_twin(prowlarr, settings, release, now):
    for indexer in settings["twin_indexers"]:
        try:
            results = prowlarr.search(indexer, release["title"], now)
        except Exception as error:
            print(f"Cannot search indexer {indexer} for a twin: {error}")
            continue
        for r in results:
            if r["size"] == release["size"] and r["seeders"] >= 1:
                return r
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


def grab(qbt, prowlarr, settings, state, torrents, now):
    downloading = sum(1 for t in torrents.values() if t["category"] in FARM + (ZEN,) and t["progress"] < 1)
    slots = settings["max_downloading"] - downloading
    if slots <= 0 or not settings["indexers"]:
        return
    room = {
        "ratio": settings["farm_size"] - size_of(torrents, FARM),
        ZEN: settings["zen_size"] - size_of(torrents, (ZEN,)),
    }
    known = set(torrents) | {original for original, _ in state["twins"].values()}
    lookups = settings["twin_lookups"]
    twins = {}
    for release, category in wishes(fetch_newest(prowlarr, settings, now), known, settings, torrents):
        if slots <= 0:
            return
        if release["infohash"] in known or release["size"] > room[category]:
            continue
        if release["infohash"] not in twins and lookups > 0:
            lookups -= 1
            twins[release["infohash"]] = find_twin(prowlarr, settings, release, now)
        twin = twins.get(release["infohash"])
        try:
            if twin:
                qbt.add(prowlarr.download(twin["link"]), category)
                state["twins"][twin["infohash"]] = [release["infohash"], category]
                source = "its public twin"
            elif release["free"]:
                qbt.add(prowlarr.download(release["link"]), category)
                state["free"][release["infohash"]] = now
                source = "the tracker, freeleech"
            elif may_pay(state, settings, category):
                qbt.add(prowlarr.download(release["link"]), category)
                state["grabbed"][release["infohash"]] = now
                source = "the tracker"
            else:
                continue
        except Exception as error:
            print(f"Cannot grab {release['title']}: {error}")
            continue
        known.add(release["infohash"])
        room[category] -= release["size"]
        slots -= 1
        print(
            f"Grabbed {release['title']} for the {'Zen reserve' if category == ZEN else 'farm'} from {source}, "
            f"{release['seeders']} seeders, {release['leechers']} leechers, {release['size'] / GB:.1f} GB"
        )


def run_once(qbt, prowlarr, settings, state, now):
    prepare(qbt, settings)
    torrents = {t["hash"].lower(): {**t, "hash": t["hash"].lower()} for t in qbt.torrents()}
    record(state, torrents, settings, now)
    drop_twins(qbt, state, torrents)
    clean_farm(qbt, settings, state, torrents, now)
    adopt(qbt, settings, state, torrents)
    clean_zen(qbt, settings, torrents)
    grab(qbt, prowlarr, settings, state, torrents, now)


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
        "tracker": os.environ.get("RATIO_TRACKER", "c411").lower(),
        "indexers": ids("RATIO_GRAB_INDEXER_IDS"),
        "twin_indexers": ids("RATIO_TWIN_INDEXER_IDS"),
        "cross_seed_key": os.environ.get("CROSS_SEED_API_KEY", ""),
        "farm_size": number("RATIO_MAX_SIZE_GB") * GB,
        "zen_size": number("RATIO_ZEN_MAX_SIZE_GB", 50) * GB,
        "zen_min_torrents": int(number("RATIO_ZEN_MIN_TORRENTS", 15)),
        "zen_hunt_surplus": number("RATIO_ZEN_HUNT_MIN_SURPLUS_GB", 20) * GB,
        "min_seeding": number("RATIO_MIN_SEEDING_DAYS") * DAY,
        "stale_after": number("RATIO_STALE_DAYS", 7) * DAY,
        "stale_upload": number("RATIO_STALE_MIN_UPLOAD", 0.2),
        "max_deficit": number("RATIO_GRAB_MAX_DEFICIT_GB", 5) * GB,
        "per_day": int(number("RATIO_GRAB_PER_DAY", 3)),
        "max_downloading": int(number("RATIO_GRAB_MAX_DOWNLOADING", 2)),
        "twin_lookups": int(number("RATIO_TWIN_LOOKUPS", 10)),
        "library_categories": tuple(c.strip() for c in os.environ.get("RATIO_LIBRARY_CATEGORIES", "radarr,tv-sonarr").split(",")),
        "adopt_after": number("RATIO_ADOPT_AFTER_HOURS", 6) * 3600,
        "tracker_seeders_in_peers": int(number("RATIO_TRACKER_SEEDERS_IN_PEERS", 2)),
        "rules": {
            "min_size": number("RATIO_GRAB_MIN_SIZE_GB", 1) * GB,
            "max_size": number("RATIO_GRAB_MAX_SIZE_GB", 5) * GB,
            "max_age_hours": number("RATIO_GRAB_MAX_AGE_HOURS", 1),
            "min_leechers": number("RATIO_GRAB_MIN_LEECHERS", 10),
            "min_leechers_per_seeder": number("RATIO_GRAB_MIN_LEECHERS_PER_SEEDER", 5),
        },
    }


def main():
    settings = settings_from_env()
    interval = int(number("RATIO_MINUTES", 10)) * 60
    qbt = QBittorrent(
        os.environ.get("QBITTORRENT_URL", "http://gluetun:8080"),
        os.environ.get("QBITTORRENT_USER", ""),
        os.environ.get("QBITTORRENT_PASSWORD", ""),
    )
    prowlarr = Prowlarr(
        os.environ.get("PROWLARR_URL", "http://prowlarr:9696"),
        os.environ.get("PROWLARR_API_KEY", ""),
        settings["tracker_seeders_in_peers"],
    )
    state_path = os.environ.get("RATIO_STATE", "/state/ratio-grab.json")
    state = load_state(state_path)
    if not settings["indexers"]:
        print("RATIO_GRAB_INDEXER_IDS is empty, ratio grabs nothing and only cleans")
    while True:
        try:
            run_once(qbt, prowlarr, settings, state, time.time())
        except Exception as error:
            print(f"Cannot run the farm and the Zen reserve: {error}")
        try:
            save_state(state_path, state)
        except OSError as error:
            print(f"Cannot save {state_path}: {error}")
        time.sleep(interval)


if __name__ == "__main__":
    main()
