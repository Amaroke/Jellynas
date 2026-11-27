use crate::compose::{included_compose_files, re, read, root, set};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const WITHOUT_MONITOR: [&str; 1] = ["uptime-kuma"];
const MONITOR_TYPES: [&str; 2] = ["http", "docker"];

struct Label {
    entity: String,
    kind: String,
    setting: String,
    value: String,
}

#[derive(Default)]
struct Entity {
    kind: String,
    services: BTreeSet<String>,
    settings: BTreeMap<String, String>,
}

fn uptime_kuma() -> PathBuf {
    root().join("services/uptime-kuma/compose.yaml")
}

fn unquoted(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[0] == bytes[bytes.len() - 1] && matches!(bytes[0], b'\'' | b'"') {
        return value[1..value.len() - 1].to_string();
    }
    value.to_string()
}

fn labels_by_service(path: &Path) -> BTreeMap<String, Vec<Label>> {
    let top = re(r"^([a-z_]+):");
    let name = re(r"^  ([a-z0-9_-]+):$");
    let label = re(r"^      kuma\.([a-z0-9-]+)\.([a-z_]+)\.([a-z_]+):(?: (.*))?$");
    let mut services: BTreeMap<String, Vec<Label>> = BTreeMap::new();
    let mut section = None;
    let mut service: Option<String> = None;
    let mut last: Option<String> = None;
    for line in read(path).lines() {
        if let Some(found) = top.captures(line) {
            section = Some(found[1].to_string());
            service = None;
            continue;
        }
        if section.as_deref() == Some("services")
            && let Some(found) = name.captures(line)
        {
            services.insert(found[1].to_string(), Vec::new());
            service = Some(found[1].to_string());
            continue;
        }
        if let Some(current) = &service
            && let Some(found) = label.captures(line)
        {
            services.get_mut(current).unwrap().push(Label {
                entity: found[1].to_string(),
                kind: found[2].to_string(),
                setting: found[3].to_string(),
                value: found.get(4).map_or("", |value| value.as_str()).to_string(),
            });
            last = Some(current.clone());
            continue;
        }
        if let Some(owner) = &last
            && line.starts_with("        ")
        {
            let value = &mut services.get_mut(owner).unwrap().last_mut().unwrap().value;
            *value = if value.is_empty() || value == ">-" {
                line.trim().to_string()
            } else {
                format!("{value} {}", line.trim())
            };
            continue;
        }
        last = None;
    }
    for labels in services.values_mut() {
        for label in labels.iter_mut() {
            label.value = unquoted(&label.value);
        }
    }
    services
}

fn all_labels() -> BTreeMap<String, Vec<Label>> {
    included_compose_files()
        .iter()
        .flat_map(|path| labels_by_service(path))
        .collect()
}

fn entities() -> BTreeMap<String, Entity> {
    let mut found: BTreeMap<String, Entity> = BTreeMap::new();
    for (service, labels) in all_labels() {
        for label in labels {
            let entity = found.entry(label.entity).or_insert_with(|| Entity {
                kind: label.kind.clone(),
                ..Entity::default()
            });
            entity.services.insert(service.clone());
            entity.settings.insert(label.setting, label.value);
        }
    }
    found
}

fn default_settings() -> BTreeMap<String, BTreeMap<String, String>> {
    let text = read(&uptime_kuma());
    let block = re(r"(?m)^      AUTOKUMA__DEFAULT_SETTINGS: \|-\n((?:        .*\n)+)")
        .captures(&text)
        .unwrap();
    let mut settings: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for line in block[1].lines() {
        let (key, value) = line.trim().split_once(": ").unwrap();
        let (kind, setting) = key.split_once('.').unwrap();
        settings
            .entry(kind.to_string())
            .or_default()
            .insert(setting.to_string(), value.to_string());
    }
    settings
}

fn ids_of(kind: &str) -> BTreeSet<String> {
    entities()
        .into_iter()
        .filter(|(_, entity)| entity.kind == kind)
        .map(|(id, _)| id)
        .collect()
}

fn monitors_of(kind: &str) -> impl Iterator<Item = (String, Entity)> {
    entities()
        .into_iter()
        .filter(move |(_, entity)| entity.kind == kind)
}

#[test]
fn autokuma_is_included() {
    assert!(labels_by_service(&uptime_kuma()).contains_key("autokuma"));
    assert!(included_compose_files().contains(&uptime_kuma()));
}

#[test]
fn every_service_has_a_monitor() {
    let missing: BTreeSet<String> = all_labels()
        .into_iter()
        .filter(|(service, labels)| {
            !WITHOUT_MONITOR.contains(&service.as_str())
                && !labels
                    .iter()
                    .any(|label| MONITOR_TYPES.contains(&label.kind.as_str()))
        })
        .map(|(service, _)| service)
        .collect();
    assert!(missing.is_empty(), "{missing:?}");
}

#[test]
fn each_entity_lives_on_one_service() {
    let shared: BTreeSet<String> = entities()
        .into_iter()
        .filter(|(_, entity)| entity.services.len() > 1)
        .map(|(id, _)| id)
        .collect();
    assert!(shared.is_empty(), "{shared:?}");
}

#[test]
fn each_entity_has_one_type() {
    for (service, labels) in all_labels() {
        let mut kinds: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for label in &labels {
            kinds.entry(&label.entity).or_default().insert(&label.kind);
        }
        for (entity, found) in kinds {
            assert_eq!(1, found.len(), "{service}: {entity}");
        }
    }
}

#[test]
fn every_monitor_has_a_name() {
    for (id, entity) in entities() {
        assert!(entity.settings.contains_key("name"), "{id}");
    }
}

#[test]
fn http_monitors_reach_a_service_or_a_public_name() {
    let services: BTreeSet<String> = all_labels().into_keys().collect();
    let internal = re(r"^http://([a-z0-9_-]+):\d+(/.*)?$");
    let public = re(r"^https://[a-z0-9-]+\.\$\{DOMAIN\}:\$\{CADDY_HTTPS_PORT:-443\}(/.*)?$");
    for (id, entity) in monitors_of("http") {
        let url = &entity.settings["url"];
        let reaches_a_service = internal
            .captures(url)
            .is_some_and(|found| services.contains(&found[1]));
        assert!(public.is_match(url) || reaches_a_service, "{id}: {url}");
    }
}

#[test]
fn public_monitors_watch_the_certificate() {
    for (id, entity) in monitors_of("http") {
        if entity.settings["url"].starts_with("https://") {
            assert_eq!(
                Some("true"),
                entity
                    .settings
                    .get("expiry_notification")
                    .map(String::as_str),
                "{id}"
            );
        }
    }
}

#[test]
fn every_monitor_type_has_defaults() {
    let kinds: BTreeSet<String> = default_settings().into_keys().collect();
    assert_eq!(set(MONITOR_TYPES), kinds);
}

#[test]
fn every_monitor_alerts_through_a_known_notification() {
    let notifications = ids_of("notification");
    for (kind, settings) in default_settings() {
        let names: Vec<String> = serde_json::from_str(&settings["notification_name_list"]).unwrap();
        assert!(!names.is_empty(), "{kind}");
        let unknown: Vec<&String> = names
            .iter()
            .filter(|name| !notifications.contains(*name))
            .collect();
        assert!(unknown.is_empty(), "{kind}: {unknown:?}");
    }
}

#[test]
fn docker_monitors_use_a_known_docker_host() {
    let host = &default_settings()["docker"]["docker_host_name"];
    assert!(ids_of("docker_host").contains(host), "{host}");
}

#[test]
fn docker_hosts_go_through_the_socket_proxy() {
    for (id, entity) in monitors_of("docker_host") {
        assert_eq!("tcp", entity.settings["connection_type"], "{id}");
        assert_eq!(
            "http://uptime-kuma-socket-proxy:2375", entity.settings["path"],
            "{id}"
        );
    }
}

#[test]
fn notification_config_is_json() {
    for (id, entity) in monitors_of("notification") {
        let config: Value = serde_json::from_str(&entity.settings["config"]).unwrap();
        assert_eq!("discord", config["type"], "{id}");
        assert_eq!(
            "https://discord.com/api/webhooks/${DISCORD_ALERTS_WEBHOOK_ID}/${DISCORD_ALERTS_WEBHOOK_TOKEN}",
            config["discordWebhookUrl"],
            "{id}"
        );
    }
}
