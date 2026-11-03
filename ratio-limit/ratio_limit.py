import http.cookiejar
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

GB = 1024**3
DAY = 86400


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
        evicted.append(t["hash"])
        used -= t["size"]
    return evicted


class QBittorrent:
    def __init__(self, url, username, password):
        self.url = url.rstrip("/")
        self.username = username
        self.password = password
        self.opener = None

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

    def request(self, path, data=None):
        if self.opener is None:
            self.login()
        body = urllib.parse.urlencode(data).encode() if data is not None else None
        try:
            with self.opener.open(f"{self.url}{path}", data=body, timeout=30) as response:
                return response.read()
        except urllib.error.HTTPError as error:
            if error.code != 403:
                raise
        self.login()
        with self.opener.open(f"{self.url}{path}", data=body, timeout=30) as response:
            return response.read()

    def torrents(self, category):
        query = urllib.parse.urlencode({"category": category})
        return json.loads(self.request(f"/api/v2/torrents/info?{query}"))

    def delete(self, hashes):
        self.request("/api/v2/torrents/delete", {"hashes": "|".join(hashes), "deleteFiles": "true"})


def run_once(client, category, limit_bytes, min_seeding_seconds):
    torrents = client.torrents(category)
    evicted = set(pick_evictions(torrents, limit_bytes, min_seeding_seconds))
    if evicted:
        client.delete(sorted(evicted))
    for t in torrents:
        if t["hash"] in evicted:
            print(f"Removed {t['name']} to keep the {category} folder under {limit_bytes / GB:g} GB", flush=True)
    remaining = sum(t["size"] for t in torrents if t["hash"] not in evicted)
    if remaining > limit_bytes:
        print(
            f"The {category} folder holds {remaining / GB:.1f} GB, over {limit_bytes / GB:g} GB, "
            "every other torrent is still downloading or below its minimum seeding time",
            flush=True,
        )


def number_from_env(name):
    try:
        return float(os.environ[name])
    except (KeyError, ValueError):
        sys.exit(f"Set {name} in .env to a number, then recreate ratio-limit")


def main():
    category = os.environ.get("RATIO_CATEGORY", "ratio")
    limit_bytes = number_from_env("RATIO_MAX_SIZE_GB") * GB
    min_seeding_seconds = number_from_env("RATIO_MIN_SEEDING_DAYS") * DAY
    interval = int(os.environ.get("RATIO_CHECK_MINUTES", "5")) * 60
    client = QBittorrent(
        os.environ.get("QBITTORRENT_URL", "http://gluetun:8080"),
        os.environ.get("QBITTORRENT_USER", ""),
        os.environ.get("QBITTORRENT_PASSWORD", ""),
    )
    while True:
        try:
            run_once(client, category, limit_bytes, min_seeding_seconds)
        except Exception as error:
            print(f"Cannot check the {category} folder: {error}", flush=True)
        time.sleep(interval)


if __name__ == "__main__":
    main()
