use crate::compose::{
    Services, all_services, doco_cd, has, included_compose_files, re, read, root, section,
    service_blocks, set,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const ESSENTIAL: [&str; 12] = [
    "caddy",
    "authelia",
    "lldap",
    "socket-proxy",
    "dashboard-socket-proxy",
    "uptime-kuma-socket-proxy",
    "doco-cd",
    "gluetun",
    "ups",
    "uptime-kuma",
    "dashboard",
    "backup",
];
const SECRET: &str = r"\$\{[A-Z_]*(KEY|PASSWORD|SECRET|TOKEN|USERNAME|EMAIL)[A-Z_]*";
const WIDGETS: [&str; 16] = [
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
    "bazarr",
    "tdarr",
    "beszel",
    "uptime-kuma",
    "ups",
];
const DOCKER_WIDGETS: [&str; 2] = ["doco-cd", "backup"];
const SECTIONS: [&str; 9] = [
    "Status",
    "Pull requests",
    "Watch",
    "Downloads",
    "Automation",
    "Farm",
    "Discord",
    "Observation",
    "Hosting",
];

fn caddyfile() -> String {
    read(&root().join("services/caddy/Caddyfile"))
}

fn services() -> Services {
    let mut services = all_services();
    services.extend(service_blocks(&doco_cd()));
    services
}

fn labels(lines: &[String]) -> BTreeMap<String, String> {
    let label = re(r"^      (dashboard\.[a-z]+):(?: (.*))?$");
    let mut found = BTreeMap::new();
    let mut key: Option<String> = None;
    for line in section(lines, "labels") {
        if let Some(entry) = label.captures(&line) {
            let value = entry.get(2).map_or("", |value| value.as_str());
            found.insert(entry[1].to_string(), value.trim_matches('"').to_string());
            key = Some(entry[1].to_string());
        } else if let Some(key) = key.as_ref().filter(|_| line.starts_with("        ")) {
            let value = found.get_mut(key).unwrap();
            *value = format!("{value} {}", line.trim()).trim().to_string();
        } else {
            key = None;
        }
    }
    found
}

fn services_where(keep: impl Fn(&BTreeMap<String, String>) -> bool) -> BTreeSet<String> {
    services()
        .into_iter()
        .filter(|(_, lines)| keep(&labels(lines)))
        .map(|(service, _)| service)
        .collect()
}

#[test]
fn the_dashboard_is_included() {
    let dashboard: PathBuf = root().join("services/dashboard/compose.yaml");
    assert!(included_compose_files().contains(&dashboard));
}

#[test]
fn no_essential_service_can_be_turned_off() {
    let switchable = services_where(|labels| {
        labels.get("dashboard.switchable").map(String::as_str) == Some("true")
    });
    let essential = set(ESSENTIAL);
    assert_eq!(
        BTreeSet::<&String>::new(),
        switchable.intersection(&essential).collect()
    );
}

#[test]
fn every_essential_service_has_a_card() {
    let listed = services_where(|labels| labels.contains_key("dashboard.section"));
    let essential = set(ESSENTIAL);
    assert_eq!(
        BTreeSet::<&String>::new(),
        essential.difference(&listed).collect()
    );
}

#[test]
fn every_card_sits_in_a_known_section() {
    for (service, lines) in services() {
        let found = labels(&lines);
        if !found.is_empty() {
            let section = found.get("dashboard.section").map(String::as_str);
            assert!(
                section.is_some_and(|section| SECTIONS.contains(&section)),
                "{service}: {section:?}"
            );
        }
    }
}

#[test]
fn switchable_is_only_ever_true() {
    for (service, lines) in services() {
        let found = labels(&lines);
        let switchable = found
            .get("dashboard.switchable")
            .map_or("true", String::as_str);
        assert_eq!("true", switchable, "{service}");
    }
}

#[test]
fn doco_cd_leaves_the_running_state_of_every_switchable_service_to_the_dashboard() {
    for (service, lines) in services() {
        let switchable = labels(&lines)
            .get("dashboard.switchable")
            .map(String::as_str)
            == Some("true");
        let autostart = has(
            &section(&lines, "labels"),
            "      cd.doco.deployment.autostart: \"false\"",
        );
        assert_eq!(switchable, autostart, "{service}");
    }
}

#[test]
fn every_widget_is_a_known_kind() {
    for (service, lines) in services() {
        if let Some(kind) = labels(&lines).get("dashboard.widget") {
            assert!(
                WIDGETS.contains(&kind.as_str()) || DOCKER_WIDGETS.contains(&kind.as_str()),
                "{service}: {kind}"
            );
        }
    }
}

#[test]
fn the_dashboard_knows_where_every_widget_lives() {
    let dashboard = all_services().remove("dashboard").unwrap();
    let environment = section(&dashboard, "environment").join("\n");
    for kind in WIDGETS {
        let prefix = kind.to_uppercase().replace('-', "_");
        assert!(
            environment.contains(&format!("      {prefix}_URL: ")),
            "{kind}"
        );
    }
}

#[test]
fn the_dashboard_knows_where_the_native_sections_come_from() {
    let dashboard = all_services().remove("dashboard").unwrap();
    let environment = section(&dashboard, "environment");
    for line in [
        "RATIO_URL: http://ratio:8000",
        "PR_RADAR_URL: http://pr-radar:8000",
    ] {
        assert!(has(&environment, &format!("      {line}")), "{line}");
    }
}

#[test]
fn every_monitor_is_a_web_address() {
    let address = re(r"^https?://\S+$");
    for (service, lines) in services() {
        if let Some(monitor) = labels(&lines).get("dashboard.monitor") {
            assert!(address.is_match(monitor), "{service}: {monitor}");
        }
    }
}

#[test]
fn every_service_has_a_card() {
    let missing = services_where(|labels| !labels.contains_key("dashboard.name"));
    assert!(missing.is_empty(), "{missing:?}");
}

#[test]
fn no_label_holds_a_secret() {
    let secret = re(SECRET);
    for (service, lines) in services() {
        for (key, value) in labels(&lines) {
            assert!(!secret.is_match(&value), "{service} {key}");
        }
    }
}

#[test]
fn no_label_holds_a_personal_value() {
    let address = re(r"\b\d{1,3}(\.\d{1,3}){3}\b");
    let url = re(r"https?://([^/\s]+)");
    let allowed = re(r"^([a-z0-9-]+\.)*\$\{DOMAIN\}:|^[a-z0-9_-]+(:\d+)?$");
    for (service, lines) in services() {
        for (key, value) in labels(&lines) {
            assert!(!address.is_match(&value), "{service} {key}");
            for host in url.captures_iter(&value) {
                assert!(allowed.is_match(&host[1]), "{service} {key}: {}", &host[1]);
            }
        }
    }
}

#[test]
fn every_link_has_a_lan_name() {
    let caddyfile = caddyfile();
    let routed: BTreeSet<String> = [
        r"(?m)^\s*import lan ([a-z0-9-]+) ",
        r"(?m)^\s*host ([a-z0-9-]+)\.lan\.",
    ]
    .into_iter()
    .flat_map(|pattern| {
        re(pattern)
            .captures_iter(&caddyfile)
            .map(|found| found[1].to_string())
            .collect::<Vec<_>>()
    })
    .collect();
    let lan = re(r"^https://[a-z0-9-]+\.lan\.\$\{DOMAIN\}:");
    let name = re(r"^https://([a-z0-9-]+)\.");
    for (service, lines) in services() {
        if let Some(href) = labels(&lines).get("dashboard.href") {
            assert!(lan.is_match(href), "{service}: {href}");
            assert!(
                routed.contains(&name.captures(href).unwrap()[1]),
                "{service}: {href}"
            );
        }
    }
}

#[test]
fn homepage_is_gone() {
    assert!(!root().join("services/homepage").exists());
    assert!(!services().contains_key("homepage"));
    assert!(!read(&root().join(".env.example")).contains("HOMEPAGE_"));
}

#[test]
fn the_old_address_of_homepage_leads_to_the_dashboard() {
    let caddyfile = caddyfile();
    assert!(!caddyfile.contains("import lan homepage "));
    let block = re(r"(?s)@homepage \{\n(.*?)\n\t\}\n\thandle @homepage \{\n(.*?)\n\t\}")
        .captures(&caddyfile)
        .unwrap();
    assert!(block[1].contains("host homepage.lan.{$DOMAIN}"));
    assert!(block[1].contains("remote_ip {$LAN_SUBNET}"));
    assert!(block[2].contains("redir https://dashboard.lan.{$DOMAIN}:{$CADDY_HTTPS_PORT}{uri}"));
}

#[test]
fn every_variable_is_documented() {
    let documented: BTreeSet<String> = re(r"(?m)^([A-Z][A-Z0-9_]*)=")
        .captures_iter(&read(&root().join(".env.example")))
        .map(|found| found[1].to_string())
        .collect();
    let variable = re(r"\$\{([A-Z][A-Z0-9_]*)");
    for path in included_compose_files() {
        let text = read(&path);
        let undocumented: BTreeSet<String> = variable
            .captures_iter(&text)
            .map(|found| found[1].to_string())
            .filter(|name| !documented.contains(name))
            .collect();
        assert!(
            undocumented.is_empty(),
            "{}: {undocumented:?}",
            path.display()
        );
    }
}
