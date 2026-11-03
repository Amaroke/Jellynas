import email.utils
import os
import time
import urllib.parse
import urllib.request
import uuid
import xml.etree.ElementTree as ET

from ratio_limit import GB, QBittorrent

TORZNAB = "{http://torznab.com/schemas/2015/feed}attr"


def pick_grabs(releases, known_hashes, slots, free_bytes, rules):
    eligible = [
        r
        for r in releases
        if r["infohash"] not in known_hashes
        and rules["min_size"] <= r["size"] <= rules["max_size"]
        and r["age_hours"] <= rules["max_age_hours"]
        and r["seeders"] >= 1
        and r["leechers"] >= rules["min_leechers"]
        and r["leechers"] >= rules["min_leechers_per_seeder"] * r["seeders"]
    ]
    eligible.sort(key=lambda r: (r["leechers"] / (r["seeders"] + 1), r["leechers"]), reverse=True)
    picked = []
    for r in eligible:
        if len(picked) >= slots:
            break
        if r["size"] > free_bytes:
            continue
        picked.append(r)
        free_bytes -= r["size"]
    return picked


def parse_torznab(xml_bytes, now):
    releases = []
    for item in ET.fromstring(xml_bytes).iter("item"):
        attrs = {a.get("name"): a.get("value") for a in item.iter(TORZNAB)}
        if not attrs.get("infohash"):
            continue
        seeders = int(attrs.get("seeders") or 0)
        peers = int(attrs.get("peers") or 0)
        published = email.utils.parsedate_to_datetime(item.findtext("pubDate")).timestamp()
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
    parts = [
        f'--{boundary}\r\nContent-Disposition: form-data; name="category"\r\n\r\n{category}\r\n'.encode(),
        f'--{boundary}\r\nContent-Disposition: form-data; name="torrents"; filename="grab.torrent"\r\n'
        "Content-Type: application/x-bittorrent\r\n\r\n".encode()
        + torrent
        + b"\r\n",
        f"--{boundary}--\r\n".encode(),
    ]
    if client.opener is None:
        client.login()
    request = urllib.request.Request(
        f"{client.url}/api/v2/torrents/add",
        data=b"".join(parts),
        headers={"Content-Type": f"multipart/form-data; boundary={boundary}"},
    )
    with client.opener.open(request, timeout=30) as response:
        response.read()


def run_once(client, settings):
    now = time.time()
    everything = client.torrents(None)
    ratio = [t for t in everything if t["category"] == settings["category"]]
    grabbed_today = sum(1 for t in ratio if t["added_on"] >= now - 86400)
    downloading = sum(1 for t in ratio if t["progress"] < 1)
    slots = min(settings["per_day"] - grabbed_today, settings["max_downloading"] - downloading)
    free_bytes = settings["max_size"] - sum(t["size"] for t in ratio)
    if slots <= 0 or free_bytes <= 0:
        return
    releases = []
    for indexer in settings["indexers"]:
        query = urllib.parse.urlencode({"t": "search", "cat": "2000,5000", "limit": 100, "apikey": settings["prowlarr_key"]})
        with urllib.request.urlopen(f"{settings['prowlarr_url']}/{indexer}/api?{query}", timeout=60) as response:
            releases += parse_torznab(response.read(), now)
    known = {t["hash"].lower() for t in everything}
    for r in pick_grabs(releases, known, slots, free_bytes, settings["rules"]):
        with urllib.request.urlopen(r["link"], timeout=60) as response:
            add_torrent(client, response.read(), settings["category"])
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
        "max_size": env_number("RATIO_MAX_SIZE_GB", 0) * GB,
        "rules": {
            "min_size": env_number("RATIO_GRAB_MIN_SIZE_GB", 1) * GB,
            "max_size": env_number("RATIO_GRAB_MAX_SIZE_GB", 15) * GB,
            "max_age_hours": env_number("RATIO_GRAB_MAX_AGE_HOURS", 6),
            "min_leechers": env_number("RATIO_GRAB_MIN_LEECHERS", 10),
            "min_leechers_per_seeder": env_number("RATIO_GRAB_MIN_LEECHERS_PER_SEEDER", 5),
        },
    }
    interval = int(env_number("RATIO_GRAB_MINUTES", 10)) * 60
    if not settings["indexers"]:
        print("RATIO_GRAB_INDEXER_IDS is empty, ratio-grab grabs nothing", flush=True)
    client = QBittorrent(
        os.environ.get("QBITTORRENT_URL", "http://gluetun:8080"),
        os.environ.get("QBITTORRENT_USER", ""),
        os.environ.get("QBITTORRENT_PASSWORD", ""),
    )
    while True:
        if settings["indexers"]:
            try:
                run_once(client, settings)
            except Exception as error:
                print(f"Cannot grab for the {settings['category']} folder: {error}", flush=True)
        time.sleep(interval)


if __name__ == "__main__":
    main()
