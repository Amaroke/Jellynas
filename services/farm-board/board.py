import datetime
import http.server
import json
import logging
import os
import re
import subprocess
import sys
import tempfile
import threading
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zlib
from pathlib import Path

from page import PARTS, SCRIPT_HASH, board_page, pixels

RATIO = "http://ratio:8000"
SELF = "http://127.0.0.1:8000"
SHOTS = ("1",)
WIDTHS = (560, 680, 800, 920, 1040)
SCALE = 2
PROBE_HEIGHT = 1000
CHANNELS = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}
BACKGROUND = "0f172aff"
STATE = Path("/state/state.json")
API = "https://discord.com/api/v10/webhooks"
USER_AGENT = "DiscordBot (https://github.com/Amaroke/Jellynas, 1.0)"
VIOLET = 0xA78BFA
ROSE = 0xFB7185
CSP = f"default-src 'none'; style-src 'unsafe-inline'; script-src 'sha256-{SCRIPT_HASH}'; form-action 'self'"

LOG = logging.getLogger("farm-board")


def required(env, name):
    value = (env.get(name) or "").strip()
    if not value:
        sys.exit(f"{name} is empty, set it in .env")
    return value


def report_day(now):
    return now.date().isoformat()


def next_hour(now):
    return (now + datetime.timedelta(hours=1)).replace(minute=0, second=0, microsecond=0)


def embeds(board, now):
    paused = bool((board.get("account") or {}).get("paused"))
    color = ROSE if paused else VIOLET
    found = [{"color": color, "image": {"url": f"attachment://part{part}.png"}} for part in SHOTS]
    found[-1]["timestamp"] = now.isoformat()
    return found


def multipart(payload, files):
    boundary = uuid.uuid4().hex.encode()
    parts = [(b'Content-Disposition: form-data; name="payload_json"\r\nContent-Type: application/json\r\n\r\n', json.dumps(payload).encode())]
    for index, (name, data) in enumerate(files):
        head = f'Content-Disposition: form-data; name="files[{index}]"; filename="{name}"\r\nContent-Type: image/png\r\n\r\n'
        parts.append((head.encode(), data))
    body = b"".join(b"--" + boundary + b"\r\n" + head + data + b"\r\n" for head, data in parts) + b"--" + boundary + b"--\r\n"
    return body, f"multipart/form-data; boundary={boundary.decode()}"


class Webhook:
    def __init__(self, webhook_id, token, opener=urllib.request.urlopen):
        self.url = f"{API}/{webhook_id}/{token}"
        self.opener = opener

    def send(self, method, path, payload, files):
        body, content_type = multipart(payload, files)
        request = urllib.request.Request(
            self.url + path, data=body, method=method, headers={"Content-Type": content_type, "User-Agent": USER_AGENT},
        )
        with self.opener(request, timeout=30) as response:
            return json.load(response)

    def post(self, payload, files):
        return self.send("POST", "?wait=true", payload, files)["id"]

    def edit(self, message_id, payload, files):
        self.send("PATCH", f"/messages/{message_id}", payload, files)


def publish(webhook, state, board, images, now):
    files = [(f"part{part}.png", image) for part, image in zip(SHOTS, images, strict=True)]
    payload = {
        "embeds": embeds(board, now),
        "attachments": [{"id": index, "filename": name} for index, (name, _) in enumerate(files)],
    }
    day = report_day(now)
    if state.get("day") == day and state.get("message_id"):
        try:
            webhook.edit(state["message_id"], payload, files)
            return state
        except urllib.error.HTTPError as error:
            if error.code != 404:
                raise
    return {"day": day, "message_id": webhook.post(payload, files)}


def read_board(url=RATIO):
    with urllib.request.urlopen(f"{url}/board", timeout=30) as response:
        return json.load(response)


def send_control(name, value, url=RATIO):
    request = urllib.request.Request(
        f"{url}/controls", data=json.dumps({"name": name, "value": value}).encode(), method="POST",
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=30):
            pass
    except urllib.error.HTTPError as error:
        if error.code == 400:
            raise ValueError(error.read().decode(errors="replace")) from error
        raise


def control_of(fields):
    name = fields.get("name", "")
    value = fields.get("value", "")
    if value in ("on", "off"):
        return name, value == "on"
    return name, float(value.replace(",", "."))


def chromium(folder, *arguments):
    return subprocess.run(
        [
            "chromium-browser", "--headless", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
            "--hide-scrollbars", f"--user-data-dir={folder}/profile", f"--default-background-color={BACKGROUND}",
            *arguments,
        ],
        check=True, capture_output=True, text=True, timeout=90,
    ).stdout


def hidden_height(dom):
    return PROBE_HEIGHT - int(re.search(r"<body>(\d+)</body>", dom).group(1))


def chunks(png):
    position = 8
    while position < len(png):
        length = int.from_bytes(png[position:position + 4])
        yield png[position + 4:position + 8], png[position + 8:position + 8 + length]
        position += length + 12


def cropped(png, height):
    found = list(chunks(png))
    header = found[0][1]
    width, depth, color = int.from_bytes(header[:4]), header[8], header[9]
    row = (width * CHANNELS[color] * depth + 7) // 8 + 1
    pixels_of = zlib.decompress(b"".join(data for kind, data in found if kind == b"IDAT"))[: row * height]
    kept = [(b"IHDR", header[:4] + height.to_bytes(4) + header[8:])]
    kept += [(kind, data) for kind, data in found[1:] if kind not in (b"IDAT", b"IEND")]
    kept += [(b"IDAT", zlib.compress(pixels_of, 9)), (b"IEND", b"")]
    return png[:8] + b"".join(len(data).to_bytes(4) + kind + data + zlib.crc32(kind + data).to_bytes(4) for kind, data in kept)


MEASURED = {}


def hidden_of(folder):
    if folder not in MEASURED:
        probe = Path(folder) / "probe.html"
        probe.write_text("<script>document.write(innerHeight)</script>", encoding="utf-8")
        MEASURED.clear()
        MEASURED[folder] = hidden_height(chromium(folder, f"--window-size=100,{PROBE_HEIGHT}", "--dump-dom", probe.as_uri()))
    return MEASURED[folder]


def shot_url(part, width):
    return f"{SELF}/graphs?lang=fr&part={part}&shot={width}"


def measured_height(dom):
    return int(re.search(r'data-height="(\d+)"', dom).group(1))


def square_of(heights):
    sides = {width: max(width, height) for width, height in heights.items()}
    width = min(sides, key=lambda w: (sides[w], -w))
    return width, sides[width]


def screenshot(part, folder):
    hidden = hidden_of(folder)
    heights = {
        width: measured_height(chromium(folder, f"--window-size={width},{PROBE_HEIGHT}", "--dump-dom", shot_url(part, width)))
        for width in WIDTHS
    }
    width, side = square_of(heights)
    path = Path(folder) / f"part{part}.png"
    chromium(
        folder, f"--window-size={side},{side + hidden}", f"--force-device-scale-factor={SCALE}", f"--screenshot={path}",
        shot_url(part, width),
    )
    png = path.read_bytes()
    scale = max(1, int.from_bytes(png[16:20]) // side)
    return cropped(png, side * scale)


def load(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {}


def save(path, state):
    path.write_text(json.dumps(state), encoding="utf-8")


def report(webhook, state_path=STATE, now=None, board_reader=read_board, shooter=screenshot):
    now = now or datetime.datetime.now().astimezone()
    try:
        board = board_reader()
        with tempfile.TemporaryDirectory() as folder:
            images = [shooter(part, folder) for part in SHOTS]
        before = load(state_path)
        after = publish(webhook, before, board, images, now)
        save(state_path, after)
        action = "edited" if after == before else "posted"
        print(f"Farm board {action}, message {after['message_id']} of {after['day']}")
    except urllib.error.HTTPError as error:
        print(f"Cannot publish the Farm board, the next try is in one hour: {error} {error.read()[:300]!r}")
    except Exception:
        LOG.exception("Cannot publish the Farm board, the next try is in one hour")


def same_origin(headers):
    origin = headers.get("Origin")
    if not origin or origin == "null":
        return origin is None
    return urllib.parse.urlsplit(origin).netloc == headers.get("Host", "")


def serve(wake, port=8000, board_reader=read_board, sender=send_control):
    latest = {"board": None}

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            url = urllib.parse.urlsplit(self.path)
            if url.path.rstrip("/") != "/graphs":
                self.send_error(404)
                return
            try:
                latest["board"] = board_reader()
            except (OSError, ValueError) as error:
                print(f"Cannot read the board of ratio: {error}")
            query = urllib.parse.parse_qs(url.query)
            page = board_page(
                latest["board"], query.get("part", [None])[0], query.get("lang", ["en"])[0], pixels(query.get("shot", [""])[0]),
            )
            body = page.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Content-Security-Policy", CSP)
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            path = urllib.parse.urlsplit(self.path).path.rstrip("/")
            if path == "/refresh":
                wake.set()
                self.empty(202)
                return
            if path != "/controls":
                self.send_error(404)
                return
            if not same_origin(self.headers):
                self.send_error(403)
                return
            length = int(self.headers.get("Content-Length") or 0)
            fields = {k: v[0] for k, v in urllib.parse.parse_qs(self.rfile.read(length).decode()).items()}
            try:
                name, value = control_of(fields)
                sender(name, value)
            except ValueError:
                self.send_error(400)
                return
            except (OSError, urllib.error.HTTPError) as error:
                print(f"Cannot change {fields.get('name')} in ratio: {error}")
                self.send_error(502)
                return
            wake.set()
            part = fields.get("part") if fields.get("part") in PARTS else "1"
            language = "fr" if fields.get("lang") == "fr" else "en"
            self.send_response(303)
            self.send_header("Location", f"/graphs?lang={language}&part={part}")
            self.send_header("Content-Length", "0")
            self.end_headers()

        def empty(self, code):
            self.send_response(code)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("", port), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def main():
    wake = threading.Event()
    serve(wake)
    webhook = Webhook(required(os.environ, "DISCORD_FARM_WEBHOOK_ID"), required(os.environ, "DISCORD_FARM_WEBHOOK_TOKEN"))
    while True:
        report(webhook)
        now = datetime.datetime.now().astimezone()
        wake.wait(max(1.0, (next_hour(now) - now).total_seconds() + 30))
        wake.clear()


if __name__ == "__main__":
    main()
