import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
UPTIME_KUMA = ROOT / "services" / "uptime-kuma" / "compose.yaml"
WITHOUT_MONITOR = {"uptime-kuma"}
MONITOR_TYPES = {"http", "docker"}
LABEL = re.compile(r"^      kuma\.([a-z0-9-]+)\.([a-z_]+)\.([a-z_]+):(?: (.*))?$")


def included_compose_files():
    text = (ROOT / "compose.yaml").read_text(encoding="utf-8")
    return [ROOT / path for path in re.findall(r"^  - ([^ #]*compose\.yaml)$", text, re.MULTILINE)]


def unquoted(value):
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
        return value[1:-1]
    return value


def labels_by_service(path):
    services, section, service, last = {}, None, None, None
    for line in path.read_text(encoding="utf-8").splitlines():
        top = re.match(r"^([a-z_]+):", line)
        if top:
            section, service = top.group(1), None
            continue
        name = re.match(r"^  ([a-z0-9_-]+):$", line)
        if section == "services" and name:
            service = name.group(1)
            services[service] = []
            continue
        label = LABEL.match(line)
        if service and label:
            value = label.group(4) or ""
            services[service].append([label.group(1), label.group(2), label.group(3), value])
            last = services[service][-1]
            continue
        if last and line.startswith("        "):
            last[3] = line.strip() if last[3] in ("", ">-") else f"{last[3]} {line.strip()}"
            continue
        last = None
    for service, labels in services.items():
        services[service] = [(entity_id, kind, key, unquoted(value)) for entity_id, kind, key, value in labels]
    return services


def all_labels():
    services = {}
    for path in included_compose_files():
        services.update(labels_by_service(path))
    return services


def entities():
    found = {}
    for service, labels in all_labels().items():
        for entity_id, entity_type, setting, value in labels:
            found.setdefault(entity_id, {"type": entity_type, "services": set(), "settings": {}})
            found[entity_id]["services"].add(service)
            found[entity_id]["settings"][setting] = value
    return found


def default_settings():
    text = UPTIME_KUMA.read_text(encoding="utf-8")
    block = re.search(r"^      AUTOKUMA__DEFAULT_SETTINGS: \|-\n((?:        .*\n)+)", text, re.MULTILINE)
    settings = {}
    for line in block.group(1).splitlines():
        key, value = line.strip().split(": ", 1)
        entity_type, setting = key.split(".", 1)
        settings.setdefault(entity_type, {})[setting] = value
    return settings


def ids_of(entity_type):
    return {entity_id for entity_id, entity in entities().items() if entity["type"] == entity_type}


class MonitorsTest(unittest.TestCase):
    def test_autokuma_is_included(self):
        self.assertIn("autokuma", labels_by_service(UPTIME_KUMA))
        self.assertIn(UPTIME_KUMA, included_compose_files())

    def test_every_service_has_a_monitor(self):
        missing = {
            service
            for service, labels in all_labels().items()
            if not any(entity_type in MONITOR_TYPES for _, entity_type, _, _ in labels)
        }
        self.assertEqual(set(), missing - WITHOUT_MONITOR)

    def test_each_entity_lives_on_one_service(self):
        shared = {entity_id for entity_id, entity in entities().items() if len(entity["services"]) > 1}
        self.assertEqual(set(), shared)

    def test_each_entity_has_one_type(self):
        for service, labels in all_labels().items():
            types = {}
            for entity_id, entity_type, _, _ in labels:
                types.setdefault(entity_id, set()).add(entity_type)
            for entity_id, found in types.items():
                self.assertEqual(1, len(found), f"{service}: {entity_id}")

    def test_every_monitor_has_a_name(self):
        for entity_id, entity in entities().items():
            self.assertIn("name", entity["settings"], entity_id)

    def test_http_monitors_reach_a_service_or_a_public_name(self):
        services = set(all_labels())
        for entity_id, entity in entities().items():
            if entity["type"] != "http":
                continue
            url = entity["settings"]["url"]
            internal = re.match(r"^http://([a-z0-9_-]+):\d+(/.*)?$", url)
            public = re.match(r"^https://[a-z0-9-]+\.\$\{DOMAIN\}:\$\{CADDY_HTTPS_PORT:-443\}(/.*)?$", url)
            self.assertTrue(public or (internal and internal.group(1) in services), f"{entity_id}: {url}")

    def test_public_monitors_watch_the_certificate(self):
        for entity_id, entity in entities().items():
            if entity["type"] == "http" and entity["settings"]["url"].startswith("https://"):
                self.assertEqual("true", entity["settings"].get("expiry_notification"), entity_id)

    def test_every_monitor_type_has_defaults(self):
        self.assertEqual(MONITOR_TYPES, set(default_settings()))

    def test_every_monitor_alerts_through_a_known_notification(self):
        notifications = ids_of("notification")
        for entity_type, settings in default_settings().items():
            names = json.loads(settings["notification_name_list"])
            self.assertTrue(names, entity_type)
            self.assertEqual(set(), set(names) - notifications, entity_type)

    def test_docker_monitors_use_a_known_docker_host(self):
        self.assertIn(default_settings()["docker"]["docker_host_name"], ids_of("docker_host"))

    def test_docker_hosts_go_through_the_socket_proxy(self):
        for entity_id in ids_of("docker_host"):
            settings = entities()[entity_id]["settings"]
            self.assertEqual("tcp", settings["connection_type"], entity_id)
            self.assertEqual("http://socket-proxy:2375", settings["path"], entity_id)

    def test_notification_config_is_json(self):
        for entity_id in ids_of("notification"):
            config = json.loads(entities()[entity_id]["settings"]["config"])
            self.assertEqual("discord", config["type"])
            self.assertEqual(
                "https://discord.com/api/webhooks/${DISCORD_ALERTS_WEBHOOK_ID}/${DISCORD_ALERTS_WEBHOOK_TOKEN}",
                config["discordWebhookUrl"],
            )


if __name__ == "__main__":
    unittest.main()
