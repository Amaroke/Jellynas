import datetime
import http.client
import http.server
import json
import os
import re
import string
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

API = "https://discord.com/api/v10/webhooks"
USER_AGENT = "DiscordBot (https://github.com/Amaroke/Jellynas, 1.0)"
MAX_LENGTH = 4096
TITLE_LENGTH = 256
COLOR = 0xA78BFA
COLORS = {"news": 0x34D399, "links": 0x60A5FA}
ROOT = Path(__file__).resolve().parent.parent
POSTS = {"links": "links.md", "news": "welcome.md"}
FAILURES = (OSError, ValueError, KeyError, http.client.HTTPException, SystemExit)
FARM_REPORT = "http://farm-report:8000/refresh"
ACTION_RESULTS = {
    "discord": "Messages Discord en cours de publication",
    "farm-report": "Farm report en cours de publication",
    "error": "Le Farm report ne répond pas, réessaie plus tard",
}
ACTIONS_PAGE = """<!doctype html>
<html lang="fr">
<meta charset="utf-8">
<title>Actions rapides</title>
<style>
html, body {{ margin: 0; background: transparent; color: #e2e8f0; font: 14px system-ui, sans-serif; }}
form {{ display: inline-block; margin: 4px 8px 4px 0; }}
button {{ padding: 8px 14px; border: 1px solid #a78bfa; border-radius: 6px; background: #1e293b; color: inherit; font: inherit; cursor: pointer; }}
button:hover {{ background: #334155; }}
p {{ margin: 6px 0 0; color: #94a3b8; }}
</style>
<form method="post" action="/actions/discord"><button>Actualiser les messages Discord</button></form>
<form method="post" action="/actions/farm-report"><button>Actualiser le Farm report</button></form>
<p>{result}</p>
"""
BLOCK = re.compile(r"^\s*(?:[-*+]\s|\d+[.)]\s|-#|#|>|\||```)")


def read_env(path):
    values = {}
    if not path.exists():
        return values
    for line in path.read_text(encoding="utf-8").splitlines():
        match = re.match(r"^([A-Z][A-Z0-9_]*)=(.*)$", line)
        if match:
            values[match[1]] = re.sub(r"^(['\"])(.*)\1$", r"\2", match[2].strip())
    return values


def webhook(env, channel):
    name = channel.upper().replace("-", "_")
    parts = [env.get(f"DISCORD_{name}_WEBHOOK_{part}", "").strip() for part in ("ID", "TOKEN")]
    if not all(parts):
        sys.exit(f"DISCORD_{name}_WEBHOOK_ID and DISCORD_{name}_WEBHOOK_TOKEN must be set in .env")
    return "/".join(parts)


def unwrap(text):
    lines = []
    fence = False
    for line in text.split("\n"):
        if line.lstrip().startswith("```"):
            fence = not fence
        elif fence or not lines or not lines[-1].strip() or lines[-1].lstrip().startswith("#"):
            pass
        elif line.strip() and not BLOCK.match(line):
            lines[-1] += " " + line.strip()
            continue
        lines.append(line)
    return "\n".join(lines)


def render(path, env):
    try:
        text = unwrap(string.Template(Path(path).read_text(encoding="utf-8")).substitute(env).strip())
    except KeyError as error:
        sys.exit(f"{path} uses ${error.args[0]}, which .env does not set")
    embed = {"description": text, "color": COLOR}
    if text.startswith("# "):
        title, _, description = text.partition("\n")
        embed = {"title": title.removeprefix("# ").strip(), "description": description.strip(), "color": COLOR}
    if len(embed.get("title", "")) > TITLE_LENGTH:
        sys.exit(f"{path} has a title of {len(embed['title'])} characters, Discord takes {TITLE_LENGTH} at most")
    if len(embed["description"]) > MAX_LENGTH:
        sys.exit(f"{path} holds {len(embed['description'])} characters once filled in, Discord takes {MAX_LENGTH} at most")
    return embed


def send(url, method, embed):
    body = json.dumps({"content": "", "embeds": [embed], "allowed_mentions": {"parse": []}}).encode()
    request = urllib.request.Request(
        url, data=body, method=method, headers={"Content-Type": "application/json", "User-Agent": USER_AGENT},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)["id"]


def publish(channel, path, env, api=API):
    hook = f"{api}/{webhook(env, channel)}"
    embed = {**render(path, env), "color": COLORS.get(channel, COLOR)}
    appdata = env.get("APPDATA_ROOT", "").strip()
    if not appdata:
        sys.exit("APPDATA_ROOT is empty in .env")
    state_path = Path(appdata) / "discord-posts" / "state.json"
    state = json.loads(state_path.read_text(encoding="utf-8")) if state_path.exists() else {}
    key = f"{channel}/{Path(path).name}"

    message = state.get(key)
    if message:
        try:
            send(f"{hook}/messages/{message}", "PATCH", embed)
            return f"Edited the message of {path} in {channel}"
        except urllib.error.HTTPError as error:
            error.close()
            if error.code != 404:
                raise
    state[key] = send(f"{hook}?wait=true", "POST", embed)
    state_path.parent.mkdir(parents=True, exist_ok=True)
    state_path.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")
    return f"Posted {path} in {channel}"


def publish_all(env, folder=ROOT / "discord-posts", api=API):
    for channel, name in POSTS.items():
        try:
            print(publish(channel, Path(folder) / name, env, api))
        except FAILURES as error:
            print(f"Cannot publish {name} in {channel}: {error}")


def actions_page(done):
    return ACTIONS_PAGE.format(result=ACTION_RESULTS.get(done, ""))


def ask_farm_report(url):
    request = urllib.request.Request(url, data=b"", method="POST")
    with urllib.request.urlopen(request, timeout=10):
        pass


def serve(wake, farm_report=FARM_REPORT, port=8000):
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            url = urllib.parse.urlsplit(self.path)
            if url.path != "/actions":
                self.send_error(404)
                return
            done = urllib.parse.parse_qs(url.query).get("done", [""])[0]
            body = actions_page(done).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'")
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            if self.path == "/actions/discord":
                wake.set()
                done = "discord"
            elif self.path == "/actions/farm-report":
                try:
                    ask_farm_report(farm_report)
                    done = "farm-report"
                except FAILURES as error:
                    print(f"Cannot ask for the Farm report: {error}")
                    done = "error"
            else:
                self.send_error(404)
                return
            self.send_response(303)
            self.send_header("Location", f"/actions?done={done}")
            self.send_header("Content-Length", "0")
            self.end_headers()

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("", port), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def next_run(now):
    run = now.replace(hour=0, minute=30, second=0, microsecond=0)
    return run if run > now else run + datetime.timedelta(days=1)


def daily(env):
    wake = threading.Event()
    serve(wake)
    while True:
        now = datetime.datetime.now().astimezone()
        wake.wait((next_run(now) - now).total_seconds())
        wake.clear()
        publish_all(env)


if __name__ == "__main__":
    environment = {**read_env(ROOT / ".env"), **os.environ}
    if sys.argv[1:] == ["--daily"]:
        daily(environment)
    if len(sys.argv) != 3:
        sys.exit("Usage: discord_post.py --daily, or discord_post.py <channel> <message file>")
    print(publish(sys.argv[1], sys.argv[2], environment))
