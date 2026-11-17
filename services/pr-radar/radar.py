import datetime
import html
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
        number title url isDraft state headRefOid createdAt
        additions deletions changedFiles mergeStateStatus reviewDecision
        author { login }
        repository { nameWithOwner }
        labels(first: 5) { nodes { name } }
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
        "created": node.get("createdAt", ""),
        "additions": node.get("additions", 0),
        "deletions": node.get("deletions", 0),
        "files": node.get("changedFiles", 0),
        "merge": node.get("mergeStateStatus"),
        "review": node.get("reviewDecision"),
        "labels": [label["name"] for label in (node.get("labels") or {}).get("nodes") or [] if label],
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


MERGE = {
    "CLEAN": ("prête à merger", "ok"),
    "HAS_HOOKS": ("prête à merger", "ok"),
    "UNSTABLE": ("prête, checks facultatifs en attente", "ok"),
    "BEHIND": ("à mettre à jour avec la base", "wait"),
    "BLOCKED": ("bloquée par les règles", "wait"),
    "DIRTY": ("conflits", "bad"),
    "DRAFT": ("brouillon", "muted"),
}
REVIEW = {"APPROVED": "approuvée", "CHANGES_REQUESTED": "changements demandés", "REVIEW_REQUIRED": "review requise"}
CI_TONE = {"SUCCESS": "ok", "FAILURE": "bad", "ERROR": "bad", "PENDING": "wait", "EXPECTED": "wait"}
PAGE = """<!doctype html>
<html lang="fr">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Pull requests</title>
<style>
* {{ box-sizing: border-box; }}
html, body {{ margin: 0; background: transparent; color: #e2e8f0;
  font: 13px/1.4 Manrope, ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif; }}
main {{ display: grid; gap: 8px; padding: 6px; }}
header {{ display: flex; flex-wrap: wrap; gap: 6px 16px; align-items: baseline; color: #94a3b8; font-size: 12px; padding: 0 2px 2px; }}
header b {{ color: #e2e8f0; font-size: 15px; }}
.empty {{ padding: 18px; text-align: center; color: #94a3b8; border: 1px dashed rgba(255, 255, 255, 0.12); border-radius: 10px; }}
article {{ position: relative; display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 6px 14px; align-items: center;
  padding: 9px 12px 9px 15px; border-radius: 10px; border: 1px solid rgba(255, 255, 255, 0.07);
  background: linear-gradient(160deg, rgba(255, 255, 255, 0.07), rgba(255, 255, 255, 0.025)); }}
article::before {{ content: ""; position: absolute; left: 0; top: 10px; bottom: 10px; width: 3px; border-radius: 0 3px 3px 0; background: var(--tone); }}
.ok {{ --tone: #34d399; }} .wait {{ --tone: #fbbf24; }} .bad {{ --tone: #fb7185; }} .muted {{ --tone: #64748b; }}
.main {{ display: grid; gap: 4px; min-width: 0; }}
.title {{ color: #f1f5f9; font-weight: 600; text-decoration: none; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
.title:hover {{ text-decoration: underline; }}
.title span {{ color: #94a3b8; font-weight: 500; }}
.meta {{ display: flex; flex-wrap: wrap; gap: 4px 6px; align-items: center; color: #94a3b8; font-size: 12px; }}
.chip {{ padding: 1px 7px; border-radius: 999px; background: rgba(255, 255, 255, 0.06); white-space: nowrap; }}
.chip.ok, .chip.wait, .chip.bad {{ color: var(--tone); background: color-mix(in srgb, var(--tone) 14%, transparent); }}
.size {{ font-variant-numeric: tabular-nums; }} .add {{ color: #34d399; }} .del {{ color: #fb7185; }}
nav {{ display: flex; gap: 6px; }}
nav a {{ color: #cbd5e1; text-decoration: none; font-size: 12px; padding: 5px 10px; border-radius: 8px; border: 1px solid rgba(255, 255, 255, 0.1); white-space: nowrap; }}
nav a:hover, nav a:focus-visible {{ border-color: var(--tone); color: #f8fafc; outline: none; }}
@media (max-width: 560px) {{ article {{ grid-template-columns: 1fr; }} }}
</style>
<main>
<header><b>{count}</b><span>{summary}</span><span>{updated}</span></header>
{rows}
</main>
"""


def age(created, now):
    try:
        seconds = (now - datetime.datetime.fromisoformat(created)).total_seconds()
    except (AttributeError, ValueError):
        return ""
    for unit, size in (("j", 86400), ("h", 3600), ("min", 60)):
        if seconds >= size:
            return f"{int(seconds // size)} {unit}"
    return "à l'instant"


def readiness(pull):
    if pull.get("draft"):
        return "muted"
    if pull.get("ci") in FAILED or pull.get("merge") == "DIRTY":
        return "bad"
    if pull.get("merge") in ("CLEAN", "HAS_HOOKS", "UNSTABLE") and pull.get("ci") in ("SUCCESS", None):
        return "ok"
    return "wait"


def row(pull, now):
    escape = html.escape
    tone = readiness(pull)
    chips = [f'<span class="chip">{escape(pull["repo"])}</span>', f'<span class="chip">{escape(pull["author"])}</span>']
    chips.append(f'<span class="chip {CI_TONE.get(pull.get("ci"), "")}">CI {CI.get(pull.get("ci"), "aucune")}</span>')
    merge = MERGE.get(pull.get("merge"))
    if merge:
        chips.append(f'<span class="chip {merge[1]}">{merge[0]}</span>')
    if pull.get("review") in REVIEW:
        chips.append(f'<span class="chip">{REVIEW[pull["review"]]}</span>')
    chips += [f'<span class="chip">{escape(label)}</span>' for label in pull.get("labels", [])]
    chips.append(
        f'<span class="size"><span class="add">+{pull.get("additions", 0)}</span> '
        f'<span class="del">−{pull.get("deletions", 0)}</span> · {pull.get("files", 0)} fichier(s)</span>'
    )
    when = age(pull.get("created", ""), now)
    if when:
        chips.append(f"<span>{when}</span>")
    url = escape(pull["url"])
    return (
        f'<article class="{tone}"><div class="main">'
        f'<a class="title" href="{url}" target="_blank" rel="noopener"><span>#{pull["number"]}</span> {escape(pull["title"])}</a>'
        f'<div class="meta">{"".join(chips)}</div></div>'
        f'<nav><a href="{url}/files" target="_blank" rel="noopener">Fichiers</a>'
        f'<a href="{url}/checks" target="_blank" rel="noopener">Checks</a>'
        f'<a href="{url}#partial-pull-merging" target="_blank" rel="noopener">Merger</a></nav></article>'
    )


def pulls_page(pulls, now=None, updated=None):
    now = now or datetime.datetime.now(datetime.UTC)
    order = {"ok": 0, "wait": 1, "bad": 2, "muted": 3}
    open_pulls = sorted(
        (pull for pull in pulls.values() if pull["state"] == "OPEN"),
        key=lambda pull: (order[readiness(pull)], pull.get("created", "")),
    )
    tones = [readiness(pull) for pull in open_pulls]
    summary = f'{tones.count("ok")} prête(s) · {tones.count("wait")} en attente · {tones.count("bad")} à corriger'
    rows = "".join(row(pull, now) for pull in open_pulls) or '<p class="empty">Aucune pull request ouverte</p>'
    stamp = f"mis à jour à {updated:%H:%M}" if updated else "en attente du premier passage"
    return PAGE.format(count=f"{len(open_pulls)} ouverte(s)", summary=summary, updated=stamp, rows=rows)


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

    def page(self):
        updated = datetime.datetime.fromtimestamp(self.last_success).astimezone() if self.last_success else None
        return pulls_page(load(self.state_path) or {}, updated=updated)

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
            elif self.path == "/pulls":
                self.answer(200, radar.page().encode(), "text/html; charset=utf-8")
            else:
                self.send_error(404)

        def answer(self, code, body, kind):
            self.send_response(code)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            if kind.startswith("text/html"):
                self.send_header("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'")
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
