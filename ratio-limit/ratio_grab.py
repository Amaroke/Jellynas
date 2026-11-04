import email.utils
import json
import os
import time
import urllib.parse
import urllib.request
import uuid
import xml.etree.ElementTree as ET

from ratio_limit import GB, QBittorrent, number_from_env

TORZNAB = "{http://torznab.com/schemas/2015/feed}attr"
DAY = 86400


def hunger(release):
    return release["leechers"] / max(release["seeders"], 1)


def pick_grabs(releases, known_hashes, slots, free_bytes, rules):
    eligible = [
        r
        for r in releases
        if r["infohash"] not in known_hashes
        and rules["min_size"] <= r["size"] <= rules["max_size"]
        and r["age_hours"] <= rules["max_age_hours"]
        and r["seeders"] >= 1
        and r["leechers"] >= rules["min_leechers"]
        and hunger(r) >= rules["min_leechers_per_seeder"]
    ]
    eligible.sort(key=lambda r: (hunger(r), r["leechers"]), reverse=True)
    picked = []
    seen = set()
    for r in eligible:
        if len(picked) >= slots:
            break
        if r["infohash"] in seen or r["size"] > free_bytes:
            continue
        seen.add(r["infohash"])
        picked.append(r)
        free_bytes -= r["size"]
    return picked


def record_transfers(ledger, ratio_torrents):
    for t in ratio_torrents:
        ledger[t["hash"].lower()] = [t["downloaded"], t["uploaded"]]
    return ledger


def deficit(ledger):
    return sum(downloaded - uploaded for downloaded, uploaded in ledger.values())


def load_state(path):
    try:
        with open(path, encoding="utf-8") as file:
            state = json.load(file)
    except FileNotFoundError:
        state = {}
    return {"transfers": state.get("transfers", {}), "grabbed": state.get("grabbed", {})}


def save_state(path, state):
    temporary = f"{path}.tmp"
    with open(temporary, "w", encoding="utf-8") as file:
        json.dump(state, file)
    os.replace(temporary, path)


def grabs_in_last_day(ratio_torrents, remembered, now):
    recent = {h for h, at in remembered.items() if at >= now - DAY}
    recent |= {t["hash"] for t in ratio_torrents if t["added_on"] >= now - DAY}
    return len(recent)


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
                "size": int(item.findtext("size") or 0),
                "seeders": seeders,
                "leechers": max(peers - seeders, 0),
                "age_hours": (now - published) / 3600,
            }
        )
    return releases


def add_torrent(client, torrent, category):
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
    if client.opener is None:
        client.login()
    request = urllib.request.Request(
        f"{client.url}/api/v2/torrents/add",
        data=body,
        headers={"Content-Type": f"multipart/form-data; boundary={boundary}"},
    )
    with client.opener.open(request, timeout=30) as response:
        answer = response.read().decode(errors="replace").strip()
    if answer == "Fails.":
        raise RuntimeError("qBittorrent refused the torrent file")


def fetch_releases(settings, now):
    releases = []
    for indexer in settings["indexers"]:
        query = urllib.parse.urlencode({"t": "search", "cat": "2000,5000", "limit": 100, "apikey": settings["prowlarr_key"]})
        try:
            with urllib.request.urlopen(f"{settings['prowlarr_url']}/{indexer}/api?{query}", timeout=60) as response:
                releases += parse_torznab(response.read(), now)
        except Exception as error:
            print(f"Cannot read indexer {indexer} from Prowlarr: {error}", flush=True)
    return releases


def run_once(client, settings, state):
    now = time.time()
    everything = client.torrents(None)
    ratio = [t for t in everything if t["category"] == settings["category"]]
    record_transfers(state["transfers"], ratio)
    remembered = state["grabbed"]
    for infohash in [h for h, at in remembered.items() if at < now - DAY]:
        del remembered[infohash]
    downloading = sum(1 for t in ratio if t["progress"] < 1)
    slots = min(
        settings["per_day"] - grabs_in_last_day(ratio, remembered, now),
        settings["max_downloading"] - downloading,
    )
    free_bytes = settings["max_size"] - sum(t["size"] for t in ratio)
    if slots <= 0 or free_bytes <= 0:
        return
    owed = deficit(state["transfers"])
    if owed > settings["max_deficit"]:
        if not settings.get("paused"):
            print(
                f"Paused: the {settings['category']} torrents downloaded {owed / GB:.1f} GB more than they uploaded, "
                f"over {settings['max_deficit'] / GB:g} GB",
                flush=True,
            )
        settings["paused"] = True
        return
    if settings.get("paused"):
        print(f"Resumed: the deficit is down to {owed / GB:.1f} GB", flush=True)
    settings["paused"] = False
    known = {t["hash"].lower() for t in everything}
    for r in pick_grabs(fetch_releases(settings, now), known, slots, free_bytes, settings["rules"]):
        try:
            with urllib.request.urlopen(r["link"], timeout=60) as response:
                add_torrent(client, response.read(), settings["category"])
        except Exception as error:
            print(f"Cannot grab {r['title']}: {error}", flush=True)
            continue
        remembered[r["infohash"]] = now
        print(
            f"Grabbed {r['title']}, {r['leechers']} leechers for {r['seeders']} seeders, {r['size'] / GB:.1f} GB",
            flush=True,
        )


def env_number(name, default):
    return float(os.environ.get(name) or default)


def main():
    settings = {
        "category": os.environ.get("RATIO_CATEGORY", "ratio"),
        "indexers": [i.strip() for i in os.environ.get("RATIO_GRAB_INDEXER_IDS", "").split(",") if i.strip()],
        "prowlarr_url": os.environ.get("PROWLARR_URL", "http://prowlarr:9696"),
        "prowlarr_key": os.environ.get("PROWLARR_API_KEY", ""),
        "per_day": int(env_number("RATIO_GRAB_PER_DAY", 3)),
        "max_downloading": int(env_number("RATIO_GRAB_MAX_DOWNLOADING", 2)),
        "max_size": number_from_env("RATIO_MAX_SIZE_GB") * GB,
        "max_deficit": env_number("RATIO_GRAB_MAX_DEFICIT_GB", 5) * GB,
        "rules": {
            "min_size": env_number("RATIO_GRAB_MIN_SIZE_GB", 1) * GB,
            "max_size": env_number("RATIO_GRAB_MAX_SIZE_GB", 5) * GB,
            "max_age_hours": env_number("RATIO_GRAB_MAX_AGE_HOURS", 1),
            "min_leechers": env_number("RATIO_GRAB_MIN_LEECHERS", 10),
            "min_leechers_per_seeder": env_number("RATIO_GRAB_MIN_LEECHERS_PER_SEEDER", 5),
        },
    }
    interval = int(env_number("RATIO_GRAB_MINUTES", 15)) * 60
    if not settings["indexers"]:
        print("RATIO_GRAB_INDEXER_IDS is empty, ratio-grab grabs nothing", flush=True)
    client = QBittorrent(
        os.environ.get("QBITTORRENT_URL", "http://gluetun:8080"),
        os.environ.get("QBITTORRENT_USER", ""),
        os.environ.get("QBITTORRENT_PASSWORD", ""),
    )
    state_path = os.environ.get("RATIO_GRAB_STATE", "/state/ratio-grab.json")
    state = load_state(state_path)
    while True:
        if settings["indexers"]:
            try:
                run_once(client, settings, state)
            except Exception as error:
                print(f"Cannot grab for the {settings['category']} folder: {error}", flush=True)
            try:
                save_state(state_path, state)
            except OSError as error:
                print(f"Cannot save {state_path}: {error}", flush=True)
        time.sleep(interval)


if __name__ == "__main__":
    main()
