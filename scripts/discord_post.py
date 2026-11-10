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
MAX_LENGTH = 4096
TITLE_LENGTH = 256
COLOR = 0xA78BFA
COLORS = {"news": 0x34D399, "links": 0x60A5FA, "admin": 0xFBBF24}
ROOT = Path(__file__).resolve().parent.parent
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


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("Usage: discord_post.py <channel> <message file>")
    print(publish(sys.argv[1], sys.argv[2], {**read_env(ROOT / ".env"), **os.environ}))
