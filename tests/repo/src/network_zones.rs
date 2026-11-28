use crate::compose::{
    all_services, doco_cd, has, included_compose_files, re, read, root, section, service_blocks,
    set,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

const PUBLIC_ALIASES: [&str; 5] = [
    "jellyfin.${DOMAIN}",
    "seerr.${DOMAIN}",
    "auth.${DOMAIN}",
    "aurral.${DOMAIN}",
    "kavita.${DOMAIN}",
];
const ZONE_MEMBERS: &[(&str, &[&str])] = &[
    ("jellyfin-zone", &["caddy", "jellyfin", "lldap"]),
    ("seerr-zone", &["caddy", "seerr", "radarr", "sonarr"]),
    ("identity-zone", &["caddy", "authelia", "lldap", "policy"]),
    ("aurral-zone", &["caddy", "aurral", "gluetun"]),
    ("kavita-zone", &["caddy", "kavita"]),
    (
        "dashboard-docker-zone",
        &["dashboard", "dashboard-socket-proxy"],
    ),
    ("autokuma-docker-zone", &["autokuma", "socket-proxy"]),
    (
        "uptime-kuma-docker-zone",
        &["uptime-kuma", "uptime-kuma-socket-proxy"],
    ),
    ("dozzle-docker-zone", &["dozzle", "dozzle-socket-proxy"]),
];
const PROXIES: [&str; 4] = [
    "socket-proxy",
    "dashboard-socket-proxy",
    "uptime-kuma-socket-proxy",
    "dozzle-socket-proxy",
];
const INSPECTORS: [&str; 2] = ["uptime-kuma-socket-proxy", "dozzle-socket-proxy"];
const OUTSIDE_INTERNAL_ZONE: [&str; 6] =
    ["jellyfin", "seerr", "authelia", "lldap", "aurral", "kavita"];
const PROXY_TEMPLATE_IMAGE: &str = "lscr.io/linuxserver/socket-proxy:3.4.9-r0-ls108";
const DIRECT_ADDRESSES: [&str; 5] = [
    "jellyfin:8096",
    "seerr:5055",
    "authelia:9091",
    "aurral:3001",
    "kavita:5000",
];
const READ_ONLY_SOCKET: &str = "      - /var/run/docker.sock:/var/run/docker.sock:ro";
const NO_WRITE: &str = "      POST: \"0\"";

fn proxy_template() -> PathBuf {
    root().join("services/socket-proxy/haproxy.cfg")
}

fn expected(zone: &str) -> BTreeSet<String> {
    let (_, members) = ZONE_MEMBERS.iter().find(|(name, _)| *name == zone).unwrap();
    set(members.iter().copied())
}

fn networks(lines: &[String]) -> BTreeSet<String> {
    let listed = re(r"^      - ([a-z0-9_-]+)$");
    let mapped = re(r"^      ([a-z0-9_-]+):$");
    section(lines, "networks")
        .iter()
        .filter_map(|line| listed.captures(line).or_else(|| mapped.captures(line)))
        .map(|found| found[1].to_string())
        .collect()
}

fn aliases(lines: &[String]) -> BTreeMap<String, BTreeSet<String>> {
    let mapped = re(r"^      ([a-z0-9_-]+):$");
    let alias = re(r"^          - (\S+)$");
    let mut found = BTreeMap::new();
    let mut network: Option<String> = None;
    for line in section(lines, "networks") {
        if let Some(name) = mapped.captures(&line) {
            found.insert(name[1].to_string(), BTreeSet::new());
            network = Some(name[1].to_string());
            continue;
        }
        if let (Some(network), Some(alias)) = (&network, alias.captures(&line)) {
            found.get_mut(network).unwrap().insert(alias[1].to_string());
        }
    }
    found
}

fn network_mode(lines: &[String]) -> Option<String> {
    let mode = re(r"^    network_mode: (\S+)$");
    lines
        .iter()
        .find_map(|line| mode.captures(line).map(|found| found[1].to_string()))
}

fn ports(lines: &[String]) -> Vec<String> {
    let port = re(r"^      - (\S+)$");
    section(lines, "ports")
        .iter()
        .map(|line| port.captures(line).expect(line)[1].to_string())
        .collect()
}

fn mounts_docker_socket(lines: &[String]) -> bool {
    section(lines, "volumes")
        .iter()
        .any(|line| line.contains("/var/run/docker.sock:"))
}

fn allowed_flags(lines: &[String]) -> BTreeSet<String> {
    section(lines, "environment")
        .iter()
        .filter(|line| line.trim().ends_with("\"1\""))
        .map(|line| line.split(':').next().unwrap().trim().to_string())
        .collect()
}

fn zone_members(zone: &str) -> BTreeSet<String> {
    all_services()
        .into_iter()
        .filter(|(_, lines)| networks(lines).contains(zone))
        .map(|(service, _)| service)
        .collect()
}

fn service(name: &str) -> Vec<String> {
    all_services().remove(name).expect(name)
}

#[test]
fn the_zones_are_declared_once_at_the_root() {
    let text = read(&root().join("compose.yaml"));
    let block = re(r"(?m)^networks:\n((?:  .*\n)+)")
        .captures(&text)
        .unwrap();
    let declared: BTreeSet<String> = re(r"(?m)^  ([a-z-]+):$")
        .captures_iter(&block[1])
        .map(|found| found[1].to_string())
        .collect();
    let mut zones = set(ZONE_MEMBERS.iter().map(|(zone, _)| *zone));
    zones.insert("internal-zone".to_string());
    assert_eq!(zones, declared);
}

#[test]
fn jellyfin_sits_with_caddy_and_lldap_only() {
    assert_eq!(expected("jellyfin-zone"), zone_members("jellyfin-zone"));
}

#[test]
fn seerr_sits_with_caddy_radarr_and_sonarr_only() {
    assert_eq!(expected("seerr-zone"), zone_members("seerr-zone"));
}

#[test]
fn authelia_sits_with_caddy_and_lldap_only() {
    assert_eq!(expected("identity-zone"), zone_members("identity-zone"));
}

#[test]
fn aurral_sits_with_caddy_and_gluetun_only() {
    assert_eq!(expected("aurral-zone"), zone_members("aurral-zone"));
}

#[test]
fn kavita_sits_with_caddy_only() {
    assert_eq!(expected("kavita-zone"), zone_members("kavita-zone"));
}

#[test]
fn lldap_has_no_public_name() {
    let caddyfile = read(&root().join("services/caddy/Caddyfile"));
    let public = caddyfile.split("*.lan.{$DOMAIN}").next().unwrap();
    assert!(!public.contains("lldap"));
}

#[test]
fn every_other_service_sits_in_the_internal_zone() {
    let outside = set(OUTSIDE_INTERNAL_ZONE.into_iter().chain(PROXIES));
    let expected: BTreeSet<String> = all_services()
        .into_iter()
        .filter(|(service, lines)| network_mode(lines).is_none() && !outside.contains(service))
        .map(|(service, _)| service)
        .collect();
    assert_eq!(expected, zone_members("internal-zone"));
}

#[test]
fn no_service_falls_back_on_the_default_network() {
    for (service, lines) in all_services() {
        assert!(
            !networks(&lines).is_empty() || network_mode(&lines).is_some(),
            "{service}"
        );
    }
}

#[test]
fn only_the_vpn_and_the_beszel_agent_borrow_another_network() {
    let borrowed: BTreeMap<String, String> = all_services()
        .into_iter()
        .filter_map(|(service, lines)| network_mode(&lines).map(|mode| (service, mode)))
        .collect();
    let expected: BTreeMap<String, String> = [
        ("qbittorrent", "service:gluetun"),
        ("slskd", "service:gluetun"),
        ("beszel-agent", "host"),
    ]
    .into_iter()
    .map(|(service, mode)| (service.to_string(), mode.to_string()))
    .collect();
    assert_eq!(expected, borrowed);
}

#[test]
fn only_the_dashboard_reaches_its_socket_proxy() {
    assert_eq!(
        expected("dashboard-docker-zone"),
        zone_members("dashboard-docker-zone")
    );
}

#[test]
fn caddy_answers_the_public_names_inside_the_nas() {
    let found = aliases(&service("caddy"));
    for zone in ["seerr-zone", "aurral-zone", "kavita-zone", "internal-zone"] {
        assert_eq!(Some(&set(PUBLIC_ALIASES)), found.get(zone), "{zone}");
    }
}

#[test]
fn only_caddy_names_the_public_services_directly() {
    let mut paths: Vec<PathBuf> = included_compose_files()
        .into_iter()
        .filter(|path| path.parent().unwrap().file_name().unwrap() != "caddy")
        .collect();
    let mut scripts: Vec<PathBuf> = fs::read_dir(root().join("services/music-bot"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "py"))
        .collect();
    scripts.sort();
    paths.extend(scripts);
    for path in paths {
        let text = read(&path);
        for address in DIRECT_ADDRESSES {
            assert!(!text.contains(address), "{}: {address}", path.display());
        }
    }
}

#[test]
fn only_the_proxies_backup_and_doco_cd_hold_the_docker_socket() {
    let mut services = all_services();
    services.extend(service_blocks(&doco_cd()));
    let holders: BTreeSet<String> = services
        .into_iter()
        .filter(|(_, lines)| mounts_docker_socket(lines))
        .map(|(service, _)| service)
        .collect();
    assert_eq!(
        set(PROXIES.into_iter().chain(["backup", "doco-cd"])),
        holders
    );
}

#[test]
fn every_proxy_sits_alone_with_its_readers() {
    for zone in [
        "autokuma-docker-zone",
        "uptime-kuma-docker-zone",
        "dozzle-docker-zone",
    ] {
        assert_eq!(expected(zone), zone_members(zone), "{zone}");
    }
}

#[test]
fn every_proxy_reads_the_socket_read_only_and_allows_no_write() {
    for name in PROXIES {
        let proxy = service(name);
        assert!(has(&proxy, READ_ONLY_SOCKET), "{name}");
        assert!(has(&proxy, NO_WRITE), "{name}");
    }
}

#[test]
fn every_proxy_runs_the_template_that_refuses_an_inspect() {
    let template = re(r"^      - (\S+):/templates/haproxy\.cfg:ro$");
    let expected = vec![proxy_template().canonicalize().unwrap()];
    for path in included_compose_files() {
        for (name, lines) in service_blocks(&path) {
            if PROXIES.contains(&name.as_str()) {
                let sources: Vec<PathBuf> = section(&lines, "volumes")
                    .iter()
                    .filter_map(|line| template.captures(line))
                    .map(|found| {
                        path.parent()
                            .unwrap()
                            .join(&found[1])
                            .canonicalize()
                            .unwrap()
                    })
                    .collect();
                assert_eq!(expected, sources, "{name}");
            }
        }
    }
}

#[test]
fn the_template_refuses_an_inspect_before_any_allow() {
    let text = read(&proxy_template());
    let deny = text
        .find("/containers/+[^/]+/+json } !allow_inspect")
        .unwrap();
    assert!(deny < text.find("http-request allow").unwrap());
    assert!(text.contains("acl allow_inspect env(ALLOW_INSPECT) -m bool"));
}

#[test]
fn the_template_refuses_an_attach() {
    assert!(read(&proxy_template()).contains("/containers/+[^/]+/+attach }"));
}

#[test]
fn every_proxy_runs_the_image_the_template_was_copied_from() {
    for name in PROXIES {
        assert!(
            has(
                &service(name),
                &format!("    image: {PROXY_TEMPLATE_IMAGE}")
            ),
            "{name}: copy /templates/haproxy.cfg of the new image into haproxy.cfg, keep the inspect and attach denies, then bump PROXY_TEMPLATE_IMAGE"
        );
    }
}

#[test]
fn only_uptime_kuma_and_dozzle_may_inspect_a_container() {
    let inspectors = set(PROXIES
        .into_iter()
        .filter(|name| allowed_flags(&service(name)).contains("ALLOW_INSPECT")));
    assert_eq!(set(INSPECTORS), inspectors);
}

#[test]
fn the_shared_proxy_lists_and_measures_the_containers_only() {
    assert_eq!(
        set(["CONTAINERS", "INFO", "PING", "VERSION"]),
        allowed_flags(&service("socket-proxy"))
    );
}

#[test]
fn the_uptime_kuma_proxy_reads_the_state_of_a_container() {
    assert_eq!(
        set(["CONTAINERS", "INFO", "PING", "VERSION", "ALLOW_INSPECT"]),
        allowed_flags(&service("uptime-kuma-socket-proxy"))
    );
}

#[test]
fn the_dozzle_proxy_reads_the_logs_and_events() {
    assert_eq!(
        set([
            "CONTAINERS",
            "EVENTS",
            "INFO",
            "PING",
            "VERSION",
            "ALLOW_INSPECT",
            "ALLOW_LOGS",
        ]),
        allowed_flags(&service("dozzle-socket-proxy"))
    );
}

#[test]
fn only_the_shared_proxy_publishes_a_port() {
    for name in PROXIES.into_iter().filter(|name| *name != "socket-proxy") {
        assert!(ports(&service(name)).is_empty(), "{name}");
    }
}

#[test]
fn every_reader_names_its_own_proxy() {
    assert!(has(
        &service("dozzle"),
        "      DOCKER_HOST: tcp://dozzle-socket-proxy:2375"
    ));
    assert!(has(
        &service("autokuma"),
        "      AUTOKUMA__DOCKER__HOSTS: tcp://socket-proxy:2375"
    ));
    assert!(has(
        &service("beszel-agent"),
        "      DOCKER_HOST: tcp://127.0.0.1:2375"
    ));
}

#[test]
fn the_dashboard_proxy_only_starts_stops_restarts_and_reads_logs() {
    let proxy = service("dashboard-socket-proxy");
    assert!(has(&proxy, READ_ONLY_SOCKET));
    assert!(has(&proxy, NO_WRITE));
    assert_eq!(
        set([
            "CONTAINERS",
            "EVENTS",
            "INFO",
            "PING",
            "VERSION",
            "ALLOW_START",
            "ALLOW_STOP",
            "ALLOW_RESTARTS",
            "ALLOW_LOGS",
        ]),
        allowed_flags(&proxy)
    );
    assert!(ports(&proxy).is_empty());
}

#[test]
fn only_caddy_publishes_a_port_to_the_network() {
    for (service, lines) in all_services() {
        for port in ports(&lines) {
            if service == "caddy" {
                assert_eq!(
                    "0.0.0.0:${CADDY_HTTPS_PORT:-443}:${CADDY_HTTPS_PORT:-443}",
                    port
                );
            } else {
                assert!(port.starts_with("127.0.0.1:"), "{service}: {port}");
            }
        }
    }
}

#[test]
fn the_proxy_listens_on_the_loopback_of_the_host_only() {
    assert_eq!(vec!["127.0.0.1:2375:2375"], ports(&service("socket-proxy")));
}
