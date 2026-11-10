import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCO_CD = ROOT / "services" / "doco-cd" / "compose.yaml"
PUBLIC_ALIASES = {"jellyfin.${DOMAIN}", "seerr.${DOMAIN}", "auth.${DOMAIN}", "aurral.${DOMAIN}"}
ZONE_MEMBERS = {
    "jellyfin-zone": {"caddy", "jellyfin", "lldap"},
    "seerr-zone": {"caddy", "seerr", "radarr", "sonarr"},
    "identity-zone": {"caddy", "authelia", "lldap"},
    "aurral-zone": {"caddy", "aurral", "lidarr"},
}
OUTSIDE_INTERNAL_ZONE = {"jellyfin", "seerr", "authelia", "lldap", "aurral"}
SOCKET_HOLDERS = {"socket-proxy", "backup", "doco-cd"}
DIRECT_ADDRESSES = ("jellyfin:8096", "seerr:5055", "authelia:9091", "aurral:3001")


def included_compose_files():
    text = (ROOT / "compose.yaml").read_text(encoding="utf-8")
    return [ROOT / path for path in re.findall(r"^  - ([^ #]*compose\.yaml)$", text, re.MULTILINE)]


def service_blocks(path):
    services, section, service = {}, None, None
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
        if service:
            services[service].append(line)
    return services


def all_services():
    services = {}
    for path in included_compose_files():
        services.update(service_blocks(path))
    return services


def section(lines, key):
    found, inside = [], False
    for line in lines:
        if re.match(r"^    [a-z_]+:", line):
            inside = line.strip() == f"{key}:"
            continue
        if inside and line.startswith("      "):
            found.append(line)
    return found


def networks(lines):
    names = set()
    for line in section(lines, "networks"):
        listed = re.match(r"^      - ([a-z0-9_-]+)$", line)
        mapped = re.match(r"^      ([a-z0-9_-]+):$", line)
        if listed or mapped:
            names.add((listed or mapped).group(1))
    return names


def aliases(lines):
    found, network = {}, None
    for line in section(lines, "networks"):
        mapped = re.match(r"^      ([a-z0-9_-]+):$", line)
        if mapped:
            network = mapped.group(1)
            found[network] = set()
            continue
        alias = re.match(r"^          - (\S+)$", line)
        if network and alias:
            found[network].add(alias.group(1))
    return found


def network_mode(lines):
    for line in lines:
        found = re.match(r"^    network_mode: (\S+)$", line)
        if found:
            return found.group(1)
    return None


def ports(lines):
    return [re.match(r"^      - (\S+)$", line).group(1) for line in section(lines, "ports")]


def mounts_docker_socket(lines):
    return any("/var/run/docker.sock:" in line for line in section(lines, "volumes"))


def zone_members(zone):
    return {service for service, lines in all_services().items() if zone in networks(lines)}


class NetworkZonesTest(unittest.TestCase):
    def test_the_zones_are_declared_once_at_the_root(self):
        text = (ROOT / "compose.yaml").read_text(encoding="utf-8")
        block = re.search(r"^networks:\n((?:  .*\n)+)", text, re.MULTILINE)
        self.assertEqual(set(ZONE_MEMBERS) | {"internal-zone"}, set(re.findall(r"^  ([a-z-]+):$", block.group(1), re.MULTILINE)))

    def test_jellyfin_sits_with_caddy_and_lldap_only(self):
        self.assertEqual(ZONE_MEMBERS["jellyfin-zone"], zone_members("jellyfin-zone"))

    def test_seerr_sits_with_caddy_radarr_and_sonarr_only(self):
        self.assertEqual(ZONE_MEMBERS["seerr-zone"], zone_members("seerr-zone"))

    def test_authelia_sits_with_caddy_and_lldap_only(self):
        self.assertEqual(ZONE_MEMBERS["identity-zone"], zone_members("identity-zone"))

    def test_aurral_sits_with_caddy_and_lidarr_only(self):
        self.assertEqual(ZONE_MEMBERS["aurral-zone"], zone_members("aurral-zone"))

    def test_lldap_has_no_public_name(self):
        caddyfile = (ROOT / "services" / "caddy" / "Caddyfile").read_text(encoding="utf-8")
        public = caddyfile.split("*.lan.{$DOMAIN}")[0]
        self.assertNotIn("lldap", public)

    def test_every_other_service_sits_in_the_internal_zone(self):
        expected = {
            service for service, lines in all_services().items() if network_mode(lines) is None
        } - OUTSIDE_INTERNAL_ZONE
        self.assertEqual(expected, zone_members("internal-zone"))

    def test_no_service_falls_back_on_the_default_network(self):
        for service, lines in all_services().items():
            self.assertTrue(networks(lines) or network_mode(lines), service)

    def test_only_the_vpn_and_the_beszel_agent_borrow_another_network(self):
        borrowed = {service: network_mode(lines) for service, lines in all_services().items() if network_mode(lines)}
        self.assertEqual(
            {"qbittorrent": "service:gluetun", "slskd": "service:gluetun", "suwayomi": "service:gluetun", "beszel-agent": "host"},
            borrowed,
        )

    def test_caddy_answers_the_public_names_inside_the_nas(self):
        found = aliases(all_services()["caddy"])
        for zone in ("seerr-zone", "aurral-zone", "internal-zone"):
            self.assertEqual(PUBLIC_ALIASES, found[zone], zone)

    def test_only_caddy_names_the_public_services_directly(self):
        paths = [path for path in included_compose_files() if path.parent.name != "caddy"]
        paths += sorted((ROOT / "services" / "homepage").glob("*.yaml")) + sorted((ROOT / "services" / "music-bot").glob("*.py"))
        for path in paths:
            text = path.read_text(encoding="utf-8")
            for address in DIRECT_ADDRESSES:
                self.assertNotIn(address, text, path.relative_to(ROOT).as_posix())

    def test_only_the_proxy_backup_and_doco_cd_hold_the_docker_socket(self):
        services = {**all_services(), **service_blocks(DOCO_CD)}
        holders = {service for service, lines in services.items() if mounts_docker_socket(lines)}
        self.assertEqual(SOCKET_HOLDERS, holders)

    def test_the_proxy_reads_the_socket_read_only_and_allows_no_write(self):
        proxy = all_services()["socket-proxy"]
        self.assertIn("      - /var/run/docker.sock:/var/run/docker.sock:ro", proxy)
        self.assertIn('      POST: "0"', proxy)

    def test_only_caddy_publishes_a_port_to_the_network(self):
        for service, lines in all_services().items():
            for port in ports(lines):
                if service == "caddy":
                    self.assertEqual("0.0.0.0:${CADDY_HTTPS_PORT:-443}:${CADDY_HTTPS_PORT:-443}", port)
                else:
                    self.assertTrue(port.startswith("127.0.0.1:"), f"{service}: {port}")

    def test_the_proxy_listens_on_the_loopback_of_the_host_only(self):
        self.assertEqual(["127.0.0.1:2375:2375"], ports(all_services()["socket-proxy"]))


if __name__ == "__main__":
    unittest.main()
