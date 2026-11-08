import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HOMEPAGE = ROOT / "homepage"
CONFIG_FILES = [HOMEPAGE / name for name in ("settings.yaml", "services.yaml", "widgets.yaml", "bookmarks.yaml")]
WITHOUT_TILE = {"homepage", "beszelagent", "autokuma", "ratio", "socketproxy"}
SECRET_FIELDS = {"key", "password", "username", "apikey", "token"}
BOARD_CARDS = ("Farm board", "Farm quests")
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
        self.assertEqual(fields, ["account.ratio", "account.left_gb", "account.zen", "account.week_uploaded_gb"])
        self.assertNotIn("grabs.", text)

    def test_the_farm_and_quests_have_no_tile_the_board_shows_them(self):
        text = (HOMEPAGE / "services.yaml").read_text(encoding="utf-8")
        self.assertNotIn("    - Farm:", text)
        self.assertNotIn("    - Quests:", text)
        self.assertNotIn("farm.", text)
        self.assertNotIn("quests.", text)

    def test_the_board_of_ratio_fills_two_iframes_of_their_own_group_in_french(self):
        text = (HOMEPAGE / "services.yaml").read_text(encoding="utf-8")
        group = re.search(r"^- Graphs:$(.*?)(?=^- |\Z)", text, re.MULTILINE | re.DOTALL)
        self.assertIsNotNone(group)
        cards = re.findall(r"^    - ([^:]+):$(.*?)(?=^    - |\Z)", group.group(1), re.MULTILINE | re.DOTALL)
        self.assertEqual([name for name, _ in cards], list(BOARD_CARDS))
        src = "src: https://ratio.lan.{{HOMEPAGE_VAR_DOMAIN}}:{{HOMEPAGE_VAR_HTTPS_PORT}}/graphs?lang=fr&part="
        for (name, body), part in zip(cards, ("1", "2"), strict=True):
            self.assertIn("type: iframe", body, name)
            self.assertIn(src + part, body, name)
            self.assertIn("classes: h-96", body, name)
            self.assertNotIn("allowScrolling", body, name)
        settings = (HOMEPAGE / "settings.yaml").read_text(encoding="utf-8")
        self.assertRegex(settings, r"(?m)^  Graphs:\n(    .*\n)*    columns: 1$")

    def test_the_cards_of_the_board_hide_their_title_and_the_frame_around_it(self):
        css = (HOMEPAGE / "custom.css").read_text(encoding="utf-8")
        rules = {
            frozenset(s.strip() for s in selectors.split(",")): body
            for selectors, body in re.findall(r"([^{}]*)\{([^}]*)\}", css)
        }
        for part, declaration in ((".service-title", "display: none"), (".service-block", "background: transparent")):
            selectors = frozenset(f'li.service[data-name="{name}"] {part}' for name in BOARD_CARDS)
            self.assertIn(selectors, rules)
            self.assertIn(declaration, rules[selectors])
        compose = (HOMEPAGE / "compose.yaml").read_text(encoding="utf-8")
        self.assertIn("./custom.css:/app/config/custom.css:ro", compose)

    def test_the_board_of_ratio_grows_to_its_full_height_on_a_phone(self):
        script = (HOMEPAGE / "custom.js").read_text(encoding="utf-8")
        self.assertIn('addEventListener("message"', script)
        self.assertIn("ratioBoardHeight", script)
        self.assertIn("frame.contentWindow !== event.source", script)
        compose = (HOMEPAGE / "compose.yaml").read_text(encoding="utf-8")
        self.assertIn("./custom.js:/app/config/custom.js:ro", compose)

    def test_every_variable_is_documented(self):
        documented = set(re.findall(r"^([A-Z][A-Z0-9_]*)=", (ROOT / ".env.example").read_text(encoding="utf-8"), re.MULTILINE))
        for path in included_compose_files():
            used = set(re.findall(r"\$\{([A-Z][A-Z0-9_]*)", path.read_text(encoding="utf-8")))
            self.assertEqual(set(), used - documented, path.name)


if __name__ == "__main__":
    unittest.main()
