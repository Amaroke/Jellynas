import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HOMEPAGE = ROOT / "homepage"
CONFIG_FILES = [HOMEPAGE / name for name in ("settings.yaml", "services.yaml", "widgets.yaml", "bookmarks.yaml")]
WITHOUT_TILE = {"homepage", "beszelagent", "autokuma", "ratio"}
SECRET_FIELDS = {"key", "password", "username", "apikey", "token"}
PLACEHOLDER = re.compile(r"\{\{(HOMEPAGE_[A-Z_]+)\}\}")


def normalized(name):
    return re.sub(r"[^a-z0-9]", "", name.lower())


def included_compose_files():
    text = (ROOT / "compose.yaml").read_text(encoding="utf-8")
    return [ROOT / path for path in re.findall(r"^  - ([^ #]*compose\.yaml)$", text, re.MULTILINE)]


def compose_services(path):
    services, section = [], None
    for line in path.read_text(encoding="utf-8").splitlines():
        top = re.match(r"^([a-z_]+):", line)
        if top:
            section = top.group(1)
            continue
        service = re.match(r"^  ([a-z0-9_-]+):$", line)
        if section == "services" and service:
            services.append(service.group(1))
    return services


def tiles():
    text = (HOMEPAGE / "services.yaml").read_text(encoding="utf-8")
    return re.findall(r"^    - ([^:]+):$", text, re.MULTILINE)


def homepage_environment():
    text = (HOMEPAGE / "compose.yaml").read_text(encoding="utf-8")
    return set(re.findall(r"^      (HOMEPAGE_VAR_[A-Z_]+):", text, re.MULTILINE))


class HomepageConfigTest(unittest.TestCase):
    def test_homepage_is_included(self):
        self.assertIn(HOMEPAGE / "compose.yaml", included_compose_files())

    def test_every_service_has_a_tile(self):
        services = {normalized(service) for path in included_compose_files() for service in compose_services(path)}
        missing = services - WITHOUT_TILE - {normalized(tile) for tile in tiles()}
        self.assertEqual(set(), missing)

    def test_secret_fields_come_from_env(self):
        for path in CONFIG_FILES:
            for line in path.read_text(encoding="utf-8").splitlines():
                field = re.match(r"^\s*([A-Za-z]+):\s*(.*)$", line)
                if field and field.group(1) in SECRET_FIELDS:
                    self.assertRegex(field.group(2), r"^'\{\{HOMEPAGE_VAR_[A-Z_]+\}\}'$", f"{path.name}: {line.strip()}")

    def test_no_personal_value_in_config(self):
        for path in CONFIG_FILES:
            text = path.read_text(encoding="utf-8")
            self.assertNotRegex(text, r"\b\d{1,3}(\.\d{1,3}){3}\b", path.name)
            for host in re.findall(r"https?://([^/\s]+)", text):
                self.assertRegex(host, r"^([a-z0-9-]+\.)*\{\{HOMEPAGE_VAR_DOMAIN\}\}:|^[a-z0-9_-]+(:\d+)?$", path.name)

    def test_every_placeholder_is_set_by_compose(self):
        used = {name for path in CONFIG_FILES for name in PLACEHOLDER.findall(path.read_text(encoding="utf-8"))}
        self.assertEqual(set(), used - homepage_environment())

    def test_every_link_has_a_lan_name(self):
        caddyfile = (ROOT / "caddy" / "Caddyfile").read_text(encoding="utf-8")
        routed = set(re.findall(r"^\s*import lan ([a-z0-9-]+) ", caddyfile, re.MULTILINE))
        routed |= set(re.findall(r"^\s*host ([a-z0-9-]+)\.lan\.", caddyfile, re.MULTILINE))
        linked = set(re.findall(r"(?:href|src): https://([a-z0-9-]+)\.lan\.", (HOMEPAGE / "services.yaml").read_text(encoding="utf-8")))
        self.assertEqual(set(), linked - routed)

    def test_the_account_tile_reads_the_account_block_of_ratio(self):
        text = (HOMEPAGE / "services.yaml").read_text(encoding="utf-8")
        self.assertNotIn("Ratio", tiles())
        account = re.search(r"^    - Account:$(.*?)(?=^    - )", text, re.MULTILINE | re.DOTALL)
        self.assertIsNotNone(account)
        self.assertIn("url: http://ratio:8000/summary", account.group(1))
        fields = re.findall(r"- field: (\S+)", account.group(1))
        self.assertEqual(fields, ["account.ratio", "account.zen", "account.week_uploaded_gb", "account.paused"])
        self.assertNotIn("grabs.", text)

    def test_the_farm_tile_reads_the_farm_block_of_ratio(self):
        text = (HOMEPAGE / "services.yaml").read_text(encoding="utf-8")
        self.assertNotIn("Zen reserve", tiles())
        self.assertNotIn("Sources", tiles())
        farm = re.search(r"^    - Farm:$(.*?)(?=^    - )", text, re.MULTILINE | re.DOTALL)
        self.assertIsNotNone(farm)
        self.assertIn("url: http://ratio:8000/summary", farm.group(1))
        fields = re.findall(r"- field: (\S+)", farm.group(1))
        self.assertEqual(fields, ["farm.seeds", "farm.size_gb", "farm.fill", "farm.downloading"])
        self.assertNotIn("zen.", text)

    def test_the_quests_tile_reads_the_quests_block_of_ratio(self):
        text = (HOMEPAGE / "services.yaml").read_text(encoding="utf-8")
        quests = re.search(r"^    - Quests:$(.*?)(?=^    - )", text, re.MULTILINE | re.DOTALL)
        self.assertIsNotNone(quests)
        self.assertIn("url: http://ratio:8000/summary", quests.group(1))
        fields = re.findall(r"- field: (\S+)", quests.group(1))
        self.assertEqual(fields, ["quests.name", "quests.shown"])

    def test_the_graphs_of_ratio_are_embedded_in_an_iframe_of_their_own_group(self):
        text = (HOMEPAGE / "services.yaml").read_text(encoding="utf-8")
        group = re.search(r"^- Graphs:$(.*?)(?=^- |\Z)", text, re.MULTILINE | re.DOTALL)
        self.assertIsNotNone(group)
        self.assertRegex(group.group(1), r"(?m)^    - Farm graphs:$")
        self.assertIn("type: iframe", group.group(1))
        self.assertIn("src: https://ratio.lan.{{HOMEPAGE_VAR_DOMAIN}}:{{HOMEPAGE_VAR_HTTPS_PORT}}/graphs", group.group(1))
        settings = (HOMEPAGE / "settings.yaml").read_text(encoding="utf-8")
        self.assertRegex(settings, r"(?m)^  Graphs:\n(    .*\n)*    columns: 1$")

    def test_every_variable_is_documented(self):
        documented = set(re.findall(r"^([A-Z][A-Z0-9_]*)=", (ROOT / ".env.example").read_text(encoding="utf-8"), re.MULTILINE))
        for path in included_compose_files():
            used = set(re.findall(r"\$\{([A-Z][A-Z0-9_]*)", path.read_text(encoding="utf-8")))
            self.assertEqual(set(), used - documented, path.name)


if __name__ == "__main__":
    unittest.main()
