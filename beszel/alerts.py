import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request

NTFY_PREFIX = "ntfy://ntfy.sh/"
ALERT_MINUTES = 10


def request(method, url, body=None, token=None):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = token
    data = json.dumps(body).encode() if body is not None else None
    with urllib.request.urlopen(urllib.request.Request(url, data, headers, method=method), timeout=30) as response:
        return json.load(response)


def threshold(env, name, default):
    value = env.get(name, "").strip()
    try:
        return int(value) if value else default
    except ValueError:
        sys.exit(f"{name} must be a whole number, got {value!r}")


def with_topic(settings, topic):
    webhooks = [hook for hook in settings.get("webhooks") or [] if not hook.startswith(NTFY_PREFIX)]
    return {**settings, "webhooks": [*webhooks, NTFY_PREFIX + topic]}


def apply_alerts(hub_url, env):
    topic = env.get("NTFY_TOPIC", "").strip()
    if not topic:
        sys.exit("NTFY_TOPIC is empty in .env")
    disk = threshold(env, "BESZEL_DISK_ALERT_PERCENT", 85)
    temperature = threshold(env, "BESZEL_TEMPERATURE_ALERT_CELSIUS", 80)
    api = hub_url.rstrip("/") + "/api"

    try:
        auth = request("POST", f"{api}/collections/users/auth-with-password",
                       {"identity": env.get("BESZEL_EMAIL", ""), "password": env.get("BESZEL_PASSWORD", "")})
    except urllib.error.HTTPError as error:
        error.close()
        sys.exit("Beszel refuses BESZEL_EMAIL and BESZEL_PASSWORD of .env")
    token, user = auth["token"], auth["record"]["id"]

    query = urllib.parse.urlencode({"filter": f"user='{user}'"})
    existing = request("GET", f"{api}/collections/user_settings/records?{query}", token=token)["items"]
    if existing:
        record = existing[0]
    else:
        record = request("POST", f"{api}/collections/user_settings/records", {"user": user}, token)
    request("PATCH", f"{api}/collections/user_settings/records/{record['id']}",
            {"settings": with_topic(record.get("settings") or {}, topic)}, token)

    systems = [system["id"] for system in
               request("GET", f"{api}/collections/systems/records?perPage=500", token=token)["items"]]
    if not systems:
        sys.exit("Beszel has no system yet, check that beszel-agent runs")
    for name, value in (("Disk", disk), ("Temperature", temperature)):
        request("POST", f"{api}/beszel/user-alerts",
                {"name": name, "value": value, "min": ALERT_MINUTES, "systems": systems, "overwrite": True}, token)
    print(f"Beszel alerts on {len(systems)} system(s): disk over {disk}%, temperature over {temperature}°C, "
          f"sent to ntfy.sh")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("Usage: alerts.py <hub url>")
    apply_alerts(sys.argv[1], os.environ)
