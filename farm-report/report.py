import datetime
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

RATIO = "http://ratio:8000"
PARTS = {"1": (1200, 424), "disk": (760, 260)}
BACKGROUND = "0f172aff"
DAY_START = 6
STATE = Path("/state/state.json")
API = "https://discord.com/api/v10/webhooks"
USER_AGENT = "DiscordBot (https://github.com/Amaroke/Jellynas, 1.0)"
VIOLET = 0xA78BFA
ROSE = 0xFB7185


def required(env, name):
    value = (env.get(name) or "").strip()
    if not value:
        sys.exit(f"{name} is empty, set it in .env")
    return value


def report_day(now):
    return (now - datetime.timedelta(hours=DAY_START)).date().isoformat()


def next_hour(now):
    return (now + datetime.timedelta(hours=1)).replace(minute=0, second=0, microsecond=0)


def embeds(summary):
    account = summary.get("account") or {}
    paused = bool(account.get("paused"))
    color = ROSE if paused else VIOLET
    return [{"color": color, "image": {"url": f"attachment://part{part}.png"}} for part in PARTS]


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


def publish(webhook, state, summary, images, now):
    files = [(f"part{part}.png", image) for part, image in zip(PARTS, images, strict=True)]
    payload = {
        "embeds": embeds(summary),
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


def read_summary():
    with urllib.request.urlopen(f"{RATIO}/summary", timeout=30) as response:
        return json.load(response)


def screenshot(part, folder):
    path = Path(folder) / f"part{part}.png"
    subprocess.run(
        [
            "chromium-browser", "--headless", "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
            "--hide-scrollbars", f"--user-data-dir={folder}/profile", f"--default-background-color={BACKGROUND}",
            "--window-size={},{}".format(*PARTS[part]), f"--screenshot={path}", f"{RATIO}/graphs?lang=fr&pad=1&part={part}",
        ],
        check=True, capture_output=True, timeout=90,
    )
    return path.read_bytes()


def load(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {}


def save(path, state):
    path.write_text(json.dumps(state), encoding="utf-8")


def report(webhook, state_path=STATE, now=None, summary_reader=read_summary, shooter=screenshot):
    now = now or datetime.datetime.now().astimezone()
    try:
        summary = summary_reader()
        with tempfile.TemporaryDirectory() as folder:
            images = [shooter(part, folder) for part in PARTS]
        before = load(state_path)
        after = publish(webhook, before, summary, images, now)
        save(state_path, after)
        action = "edited" if after == before else "posted"
        print(f"Farm report {action}, message {after['message_id']} of {after['day']}")
    except urllib.error.HTTPError as error:
        print(f"Cannot publish the Farm report, the next try is in one hour: {error} {error.read()[:300]!r}")
    except Exception as error:
        print(f"Cannot publish the Farm report, the next try is in one hour: {error}")


def main():
    webhook = Webhook(required(os.environ, "DISCORD_FARM_WEBHOOK_ID"), required(os.environ, "DISCORD_FARM_WEBHOOK_TOKEN"))
    while True:
        report(webhook)
        now = datetime.datetime.now().astimezone()
        time.sleep(max(1.0, (next_hour(now) - now).total_seconds() + 30))


if __name__ == "__main__":
    main()
