import json
import os
import re
import string
import sys
import urllib.error
import urllib.request
from pathlib import Path

API = "https://discord.com/api/v10/webhooks"
USER_AGENT = "DiscordBot (https://github.com/Amaroke/Jellynas, 1.0)"
MAX_LENGTH = 2000
ROOT = Path(__file__).resolve().parent.parent


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


def render(path, env):
    try:
        text = string.Template(Path(path).read_text(encoding="utf-8")).substitute(env).strip()
    except KeyError as error:
        sys.exit(f"{path} uses ${error.args[0]}, which .env does not set")
    if len(text) > MAX_LENGTH:
        sys.exit(f"{path} holds {len(text)} characters once filled in, Discord takes {MAX_LENGTH} at most")
    return text


def send(url, method, content):
    body = json.dumps({"content": content, "allowed_mentions": {"parse": []}}).encode()
    request = urllib.request.Request(
        url, data=body, method=method, headers={"Content-Type": "application/json", "User-Agent": USER_AGENT},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)["id"]


def publish(channel, path, env, api=API):
    hook = f"{api}/{webhook(env, channel)}"
    content = render(path, env)
    appdata = env.get("APPDATA_ROOT", "").strip()
    if not appdata:
        sys.exit("APPDATA_ROOT is empty in .env")
    state_path = Path(appdata) / "discord-posts" / "state.json"
    state = json.loads(state_path.read_text(encoding="utf-8")) if state_path.exists() else {}
    key = f"{channel}/{Path(path).name}"

    message = state.get(key)
    if message:
        try:
            send(f"{hook}/messages/{message}", "PATCH", content)
            return f"Edited the message of {path} in {channel}"
        except urllib.error.HTTPError as error:
            error.close()
            if error.code != 404:
                raise
    state[key] = send(f"{hook}?wait=true", "POST", content)
    state_path.parent.mkdir(parents=True, exist_ok=True)
    state_path.write_text(json.dumps(state, indent=2) + "\n", encoding="utf-8")
    return f"Posted {path} in {channel}"


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("Usage: discord_post.py <channel> <message file>")
    print(publish(sys.argv[1], sys.argv[2], {**read_env(ROOT / ".env"), **os.environ}))
