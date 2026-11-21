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
SECTIONS = {"Status", "Quick actions", "Pull requests", "Watch", "Downloads", "Farm", "Automation", "Observation", "Hosting"}


def services():
    return {**all_services(), **service_blocks(DOCO_CD)}


def labels(lines):
    found = {}
    for line in section(lines, "labels"):
        label = re.match(r"^      (dashboard\.[a-z]+): (.*)$", line)
        if label:
            found[label.group(1)] = label.group(2).strip('"')
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


if __name__ == "__main__":
    unittest.main()
