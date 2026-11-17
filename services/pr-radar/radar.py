import datetime
import http.client
import http.server
import json
import os
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

GITHUB = "https://api.github.com/graphql"
DISCORD = "https://discord.com/api/v10/webhooks"
USER_AGENT = "DiscordBot (https://github.com/Amaroke/Jellynas, 1.0)"
INTERVAL = 180
CLOSED_WINDOW = datetime.timedelta(days=2)
EMBEDS_PER_MESSAGE = 10
TITLE_LENGTH = 256
FAILED = {"FAILURE", "ERROR"}
BOTS = {"renovate", "dependabot", "github-actions"}
FAILURES = (OSError, ValueError, KeyError, TypeError, http.client.HTTPException)
VIEWER = "query { viewer { login } }"
SEARCH = """query($q: String!) {
  search(query: $q, type: ISSUE, first: 100) {
    nodes {
      ... on PullRequest {
        number title url isDraft state headRefOid
        author { login }
        repository { nameWithOwner }
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
      }
    }
  }
}"""
KINDS = {
    "opened": ("nouvelle PR", 0x3FB950),
    "failed": ("CI en échec", 0xF85149),
    "merged": ("mergée", 0xA371F7),
    "closed": ("fermée sans merge", 0x8B949E),
}
CI = {"SUCCESS": "verte", "FAILURE": "en échec", "ERROR": "en échec", "PENDING": "en cours", "EXPECTED": "en attente"}


def request(url, payload, headers=None, method="POST"):
    body = json.dumps(payload).encode()
    headers = {"Content-Type": "application/json", "User-Agent": USER_AGENT, **(headers or {})}
    with urllib.request.urlopen(urllib.request.Request(url, data=body, method=method, headers=headers), timeout=30) as response:
        text = response.read()
    return json.loads(text) if text else None


def graphql(token, query, variables=None, api=GITHUB):
    answer = request(api, {"query": query, "variables": variables or {}}, {"Authorization": f"Bearer {token}"})
    if answer.get("errors"):
        raise ValueError(f"GitHub answered {answer['errors'][0].get('message')}")
    return answer["data"]


def queries(login, since):
    day = since.strftime("%Y-%m-%d")
    return [
        f"is:pr is:open archived:false user:{login}",
        f"is:pr is:open archived:false author:{login} -user:{login}",
        f"is:pr is:closed archived:false user:{login} closed:>={day}",
        f"is:pr is:closed archived:false author:{login} -user:{login} closed:>={day}",
    ]


def summarize(node):
    commits = (node.get("commits") or {}).get("nodes") or []
    rollup = (commits[0].get("commit") or {}).get("statusCheckRollup") if commits else None
    return {
        "repo": node["repository"]["nameWithOwner"],
        "number": node["number"],
        "title": node["title"],
        "url": node["url"],
        "author": (node.get("author") or {}).get("login", "ghost"),
        "state": node["state"],
        "draft": node.get("isDraft", False),
        "sha": node.get("headRefOid", ""),
        "ci": (rollup or {}).get("state"),
    }


def fetch(token, api=GITHUB, now=None):
    now = now or datetime.datetime.now(datetime.UTC)
    login = graphql(token, VIEWER, api=api)["viewer"]["login"]
    pulls = {}
    for query in queries(login, now - CLOSED_WINDOW):
        for node in graphql(token, SEARCH, {"q": query}, api)["search"]["nodes"]:
            if node:
                pull = summarize(node)
                pulls[f"{pull['repo']}#{pull['number']}"] = pull
    return pulls


def events(previous, current):
    found = []
    for key, pull in sorted(current.items()):
        old = previous.get(key)
        if pull["state"] == "OPEN":
            if old is None or old["state"] != "OPEN":
                found.append(("opened", pull))
            elif pull["ci"] in FAILED and (old["ci"] not in FAILED or old["sha"] != pull["sha"]):
                found.append(("failed", pull))
        elif old is None or old["state"] == "OPEN":
            found.append(("merged" if pull["state"] == "MERGED" else "closed", pull))
    return found


def embed(kind, pull):
    label, color = KINDS[kind]
    title = f"#{pull['number']} {pull['title']}"
    fields = [{"name": "Auteur", "value": pull["author"], "inline": True}]
    if kind in ("opened", "failed"):
        fields.append({"name": "CI", "value": CI.get(pull["ci"], "aucune"), "inline": True})
    if pull["draft"] and kind == "opened":
        fields.append({"name": "Brouillon", "value": "oui", "inline": True})
    return {
        "author": {"name": f"{pull['repo']} · {label}"},
        "title": title if len(title) <= TITLE_LENGTH else title[: TITLE_LENGTH - 1] + "…",
        "url": pull["url"],
        "color": color,
        "fields": fields,
    }


def post(hook, embeds, api=DISCORD):
    for start in range(0, len(embeds), EMBEDS_PER_MESSAGE):
        payload = {"content": "", "embeds": embeds[start : start + EMBEDS_PER_MESSAGE], "allowed_mentions": {"parse": []}}
        request(f"{api}/{hook}", payload)


def stats(pulls):
    open_pulls = [pull for pull in pulls.values() if pull["state"] == "OPEN"]
    return {
        "open": len(open_pulls),
        "bots": sum(pull["author"] in BOTS or pull["author"].endswith("[bot]") for pull in open_pulls),
        "failing": sum(pull["ci"] in FAILED for pull in open_pulls),
    }


def load(path):
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None


def save(path, pulls):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(pulls, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(path)


def poll(state_path, read, send):
    current = read()
    previous = load(state_path)
    found = [] if previous is None else events(previous, current)
    if found:
        send([embed(kind, pull) for kind, pull in found])
    save(state_path, current)
    return found


class Radar:
    def __init__(self, state_path):
        self.state_path = state_path
        self.last_success = None

    def status(self):
        return stats(load(self.state_path) or {})

    def healthy(self, now=None):
        now = now or time.time()
        return self.last_success is not None and now - self.last_success < 3 * INTERVAL

    def run_once(self, read, send):
        found = poll(self.state_path, read, send)
        self.last_success = time.time()
        return found


def serve(radar, port=8000):
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == "/status":
                self.answer(200, json.dumps(radar.status()).encode(), "application/json")
            elif self.path == "/health":
                ok = radar.healthy()
                self.answer(200 if ok else 503, b"ok" if ok else b"no recent poll", "text/plain")
            else:
                self.send_error(404)

        def answer(self, code, body, kind):
            self.send_response(code)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("0.0.0.0", port), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def settings(env):
    token = env.get("PR_RADAR_GITHUB_TOKEN", "").strip()
    hook = "/".join(env.get(f"DISCORD_PR_WEBHOOK_{part}", "").strip() for part in ("ID", "TOKEN"))
    if not token:
        sys.exit("PR_RADAR_GITHUB_TOKEN must be set in .env")
    if hook.startswith("/") or hook.endswith("/"):
        sys.exit("DISCORD_PR_WEBHOOK_ID and DISCORD_PR_WEBHOOK_TOKEN must be set in .env")
    return token, hook


def main():
    token, hook = settings(os.environ)
    radar = Radar(Path(os.environ.get("PR_RADAR_STATE", "/state/state.json")))
    serve(radar)
    while True:
        try:
            found = radar.run_once(lambda: fetch(token), lambda embeds: post(hook, embeds))
            print(f"Polled GitHub, {len(found)} event(s) sent to Discord", flush=True)
        except urllib.error.HTTPError as error:
            error.close()
            print(f"Cannot poll: HTTP {error.code} from {error.url}", flush=True)
        except FAILURES as error:
            print(f"Cannot poll: {error}", flush=True)
        time.sleep(INTERVAL)


if __name__ == "__main__":
    main()
