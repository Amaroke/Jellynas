import http.server
import json
import threading
import unittest

from alerts import apply_alerts

ENV = {
    "BESZEL_EMAIL": "admin@example.com",
    "BESZEL_PASSWORD": "secret",
    "NTFY_TOPIC": "nas-alerts-1234",
}


class FakeHub:
    def __init__(self, settings=None):
        self.settings = settings
        self.alerts = []
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), self.handler())
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_port}"

    def close(self):
        self.server.shutdown()
        self.server.server_close()

    def handler(self):
        hub = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def reply(self, status, body):
                data = json.dumps(body).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def body(self):
                return json.loads(self.rfile.read(int(self.headers["Content-Length"])))

            def authorized(self):
                return self.headers.get("Authorization") == "token-1"

            def do_POST(self):
                if self.path == "/api/collections/users/auth-with-password":
                    body = self.body()
                    if body == {"identity": "admin@example.com", "password": "secret"}:
                        return self.reply(200, {"token": "token-1", "record": {"id": "user-1"}})
                    return self.reply(400, {"message": "Failed to authenticate."})
                if not self.authorized():
                    return self.reply(401, {})
                if self.path == "/api/beszel/user-alerts":
                    hub.alerts.append(self.body())
                    return self.reply(200, {"success": True})
                if self.path == "/api/collections/user_settings/records":
                    body = self.body()
                    settings = {"chartTime": "1h", "emails": ["admin@example.com"]}
                    hub.settings = {"id": "settings-1", "user": body["user"], "settings": settings}
                    return self.reply(200, hub.settings)
                return self.reply(404, {})

            def do_PATCH(self):
                if not self.authorized():
                    return self.reply(401, {})
                if hub.settings and self.path == f"/api/collections/user_settings/records/{hub.settings['id']}":
                    hub.settings = {**hub.settings, **self.body()}
                    return self.reply(200, hub.settings)
                return self.reply(404, {})

            def do_GET(self):
                if not self.authorized():
                    return self.reply(401, {})
                if self.path.startswith("/api/collections/systems/records"):
                    return self.reply(200, {"items": [{"id": "sys-1"}, {"id": "sys-2"}]})
                if self.path.startswith("/api/collections/user_settings/records"):
                    return self.reply(200, {"items": [hub.settings] if hub.settings else []})
                return self.reply(404, {})

        return Handler


class ApplyAlertsTest(unittest.TestCase):
    def hub(self, settings=None):
        hub = FakeHub(settings)
        self.addCleanup(hub.close)
        return hub

    def test_sets_disk_and_temperature_alerts_on_every_system_with_defaults(self):
        hub = self.hub()
        apply_alerts(hub.url, ENV)
        self.assertEqual(hub.alerts, [
            {"name": "Disk", "value": 85, "min": 10, "systems": ["sys-1", "sys-2"], "overwrite": True},
            {"name": "Temperature", "value": 80, "min": 10, "systems": ["sys-1", "sys-2"], "overwrite": True},
        ])

    def test_thresholds_come_from_the_environment(self):
        hub = self.hub()
        apply_alerts(hub.url, {**ENV, "BESZEL_DISK_ALERT_PERCENT": "90", "BESZEL_TEMPERATURE_ALERT_CELSIUS": "65"})
        self.assertEqual([(a["name"], a["value"]) for a in hub.alerts], [("Disk", 90), ("Temperature", 65)])

    def test_creates_the_user_settings_with_the_ntfy_topic(self):
        hub = self.hub()
        apply_alerts(hub.url, ENV)
        self.assertEqual(hub.settings["user"], "user-1")
        self.assertEqual(hub.settings["settings"]["webhooks"], ["ntfy://ntfy.sh/nas-alerts-1234"])

    def test_replaces_an_older_ntfy_topic_and_keeps_other_settings(self):
        hub = self.hub({"id": "settings-1", "user": "user-1", "settings": {
            "chartTime": "1h",
            "emails": ["admin@example.com"],
            "webhooks": ["ntfy://ntfy.sh/old-topic", "discord://token@id"],
        }})
        apply_alerts(hub.url, ENV)
        self.assertEqual(hub.settings["settings"], {
            "chartTime": "1h",
            "emails": ["admin@example.com"],
            "webhooks": ["discord://token@id", "ntfy://ntfy.sh/nas-alerts-1234"],
        })

    def test_refuses_an_empty_topic(self):
        hub = self.hub()
        with self.assertRaisesRegex(SystemExit, "NTFY_TOPIC"):
            apply_alerts(hub.url, {**ENV, "NTFY_TOPIC": ""})
        self.assertEqual(hub.alerts, [])

    def test_refuses_a_wrong_password(self):
        hub = self.hub()
        with self.assertRaisesRegex(SystemExit, "BESZEL_EMAIL"):
            apply_alerts(hub.url, {**ENV, "BESZEL_PASSWORD": "wrong"})


if __name__ == "__main__":
    unittest.main()
