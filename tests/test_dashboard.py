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
    "uptime-kuma",
    "dashboard",
    "backup",
}
HOMEPAGE = ROOT / "services" / "homepage" / "services.yaml"
WIDGETS = {
    "jellyfin",
    "music-bot",
    "seerr",
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
}
SECTIONS = {"Status", "Quick actions", "Pull requests", "Watch", "Downloads", "Farm", "Automation", "Observation", "Hosting"}


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


def homepage_cards():
    cards, name = {}, None
    for line in HOMEPAGE.read_text(encoding="utf-8").splitlines():
        card = re.match(r"^    - (.+):$", line)
        if card:
            name = card.group(1)
            cards[name] = {}
        kind = re.match(r"^          type: (.+)$", line)
        if name and kind:
            cards[name]["widget"] = kind.group(1)
        if name and re.match(r"^        siteMonitor:", line):
            cards[name]["monitor"] = True
    return cards


def dashboard_cards():
    return {found["dashboard.name"]: found for found in map(labels, services().values()) if "dashboard.name" in found}


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

    def test_every_widget_is_a_known_kind(self):
        for service, lines in services().items():
            kind = labels(lines).get("dashboard.widget")
            if kind:
                self.assertIn(kind, WIDGETS, service)

    def test_the_dashboard_knows_where_every_widget_lives(self):
        environment = "\n".join(section(all_services()["dashboard"], "environment"))
        for kind in WIDGETS:
            prefix = kind.upper().replace("-", "_")
            self.assertIn(f"      {prefix}_URL: ", environment, kind)

    def test_the_dashboard_knows_where_the_native_sections_come_from(self):
        environment = section(all_services()["dashboard"], "environment")
        for line in ("FARM_BOARD_URL: http://farm-board:8000", "DISCORD_POSTS_URL: http://discord-posts:8000", "PR_RADAR_URL: http://pr-radar:8000"):
            self.assertIn(f"      {line}", environment)

    def test_every_monitor_is_a_web_address(self):
        for service, lines in services().items():
            monitor = labels(lines).get("dashboard.monitor")
            if monitor is not None:
                self.assertRegex(monitor, r"^https?://\S+$", service)

    @unittest.skipUnless(HOMEPAGE.exists(), "Homepage is gone")
    def test_every_homepage_widget_and_monitor_has_its_counterpart(self):
        cards = dashboard_cards()
        for name, homepage in homepage_cards().items():
            kind = homepage.get("widget")
            if kind == "iframe" or not (kind or homepage.get("monitor")):
                continue
            self.assertIn(name, cards)
            if kind:
                self.assertEqual(kind.replace("uptimekuma", "uptime-kuma"), cards[name].get("dashboard.widget"), name)
            if homepage.get("monitor"):
                self.assertIn("dashboard.monitor", cards[name], name)


if __name__ == "__main__":
    unittest.main()
