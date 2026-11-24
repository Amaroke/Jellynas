import re
import unittest
from pathlib import Path

from tests.test_network_zones import (
    DOCO_CD,
    all_services,
    included_compose_files,
    section,
    service_blocks,
)

ROOT = Path(__file__).resolve().parent.parent
DASHBOARD = ROOT / "services" / "dashboard"
ESSENTIAL = {
    "caddy",
    "authelia",
    "lldap",
    "socket-proxy",
    "dashboard-socket-proxy",
    "doco-cd",
    "gluetun",
    "ups",
    "uptime-kuma",
    "dashboard",
    "backup",
}
CADDYFILE = ROOT / "services" / "caddy" / "Caddyfile"
SECRET = re.compile(r"\$\{[A-Z_]*(KEY|PASSWORD|SECRET|TOKEN|USERNAME|EMAIL)[A-Z_]*")
WIDGETS = {
    "jellyfin",
    "music-bot",
    "seerr",
    "aurral",
    "qbittorrent",
    "gluetun",
    "slskd",
    "autobrr",
    "prowlarr",
    "radarr",
    "sonarr",
    "suwayomi",
    "bazarr",
    "tdarr",
    "beszel",
    "uptime-kuma",
    "ups",
}
DOCKER_WIDGETS = {"doco-cd", "backup"}
SECTIONS = {"Status", "Pull requests", "Watch", "Downloads", "Automation", "Farm", "Discord", "Observation", "Hosting"}


def services():
    return {**all_services(), **service_blocks(DOCO_CD)}


def labels(lines):
    found, key = {}, None
    for line in section(lines, "labels"):
        label = re.match(r"^      (dashboard\.[a-z]+):(?: (.*))?$", line)
        if label:
            key = label.group(1)
            found[key] = (label.group(2) or "").strip('"')
        elif key and line.startswith("        "):
            found[key] = (found[key] + " " + line.strip()).strip()
        else:
            key = None
    return found


class DashboardLabelsTest(unittest.TestCase):
    def test_the_dashboard_is_included(self):
        self.assertIn(DASHBOARD / "compose.yaml", included_compose_files())

    def test_no_essential_service_can_be_turned_off(self):
        switchable = {service for service, lines in services().items() if labels(lines).get("dashboard.switchable") == "true"}
        self.assertEqual(set(), switchable & ESSENTIAL)

    def test_every_essential_service_has_a_card(self):
        listed = {service for service, lines in services().items() if "dashboard.section" in labels(lines)}
        self.assertEqual(set(), ESSENTIAL - listed)

    def test_every_card_sits_in_a_known_section(self):
        for service, lines in services().items():
            found = labels(lines)
            if found:
                self.assertIn(found.get("dashboard.section"), SECTIONS, service)

    def test_switchable_is_only_ever_true(self):
        for service, lines in services().items():
            self.assertIn(labels(lines).get("dashboard.switchable", "true"), {"true"}, service)

    def test_doco_cd_leaves_the_running_state_of_every_switchable_service_to_the_dashboard(self):
        for service, lines in services().items():
            switchable = labels(lines).get("dashboard.switchable") == "true"
            autostart = '      cd.doco.deployment.autostart: "false"' in section(lines, "labels")
            self.assertEqual(switchable, autostart, service)

    def test_every_widget_is_a_known_kind(self):
        for service, lines in services().items():
            kind = labels(lines).get("dashboard.widget")
            if kind:
                self.assertIn(kind, WIDGETS | DOCKER_WIDGETS, service)

    def test_the_dashboard_knows_where_every_widget_lives(self):
        environment = "\n".join(section(all_services()["dashboard"], "environment"))
        for kind in WIDGETS:
            prefix = kind.upper().replace("-", "_")
            self.assertIn(f"      {prefix}_URL: ", environment, kind)

    def test_the_dashboard_knows_where_the_native_sections_come_from(self):
        environment = section(all_services()["dashboard"], "environment")
        for line in ("RATIO_URL: http://ratio:8000", "PR_RADAR_URL: http://pr-radar:8000"):
            self.assertIn(f"      {line}", environment)

    def test_every_monitor_is_a_web_address(self):
        for service, lines in services().items():
            monitor = labels(lines).get("dashboard.monitor")
            if monitor is not None:
                self.assertRegex(monitor, r"^https?://\S+$", service)

    def test_every_service_has_a_card(self):
        missing = {service for service, lines in services().items() if "dashboard.name" not in labels(lines)}
        self.assertEqual(set(), missing)

    def test_no_label_holds_a_secret(self):
        for service, lines in services().items():
            for key, value in labels(lines).items():
                self.assertNotRegex(value, SECRET, f"{service} {key}")

    def test_no_label_holds_a_personal_value(self):
        for service, lines in services().items():
            for key, value in labels(lines).items():
                self.assertNotRegex(value, r"\b\d{1,3}(\.\d{1,3}){3}\b", f"{service} {key}")
                for host in re.findall(r"https?://([^/\s]+)", value):
                    self.assertRegex(host, r"^([a-z0-9-]+\.)*\$\{DOMAIN\}:|^[a-z0-9_-]+(:\d+)?$", f"{service} {key}")

    def test_every_link_has_a_lan_name(self):
        caddyfile = CADDYFILE.read_text(encoding="utf-8")
        routed = set(re.findall(r"^\s*import lan ([a-z0-9-]+) ", caddyfile, re.MULTILINE))
        routed |= set(re.findall(r"^\s*host ([a-z0-9-]+)\.lan\.", caddyfile, re.MULTILINE))
        for service, lines in services().items():
            href = labels(lines).get("dashboard.href")
            if href:
                self.assertRegex(href, r"^https://[a-z0-9-]+\.lan\.\$\{DOMAIN\}:", service)
                self.assertIn(re.match(r"^https://([a-z0-9-]+)\.", href).group(1), routed, service)

    def test_homepage_is_gone(self):
        self.assertFalse((ROOT / "services" / "homepage").exists())
        self.assertNotIn("homepage", services())
        self.assertNotIn("HOMEPAGE_", (ROOT / ".env.example").read_text(encoding="utf-8"))

    def test_the_old_address_of_homepage_leads_to_the_dashboard(self):
        caddyfile = CADDYFILE.read_text(encoding="utf-8")
        self.assertNotRegex(caddyfile, r"import lan homepage ")
        block = re.search(r"@homepage \{\n(.*?)\n\t\}\n\thandle @homepage \{\n(.*?)\n\t\}", caddyfile, re.DOTALL)
        self.assertIsNotNone(block)
        self.assertIn("host homepage.lan.{$DOMAIN}", block.group(1))
        self.assertIn("remote_ip {$LAN_SUBNET}", block.group(1))
        self.assertIn("redir https://dashboard.lan.{$DOMAIN}:{$CADDY_HTTPS_PORT}{uri}", block.group(2))

    def test_every_variable_is_documented(self):
        documented = set(re.findall(r"^([A-Z][A-Z0-9_]*)=", (ROOT / ".env.example").read_text(encoding="utf-8"), re.MULTILINE))
        for path in included_compose_files():
            used = set(re.findall(r"\$\{([A-Z][A-Z0-9_]*)", path.read_text(encoding="utf-8")))
            self.assertEqual(set(), used - documented, path.name)


if __name__ == "__main__":
    unittest.main()
