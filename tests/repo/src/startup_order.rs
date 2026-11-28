use crate::compose::{all_services, has, re, section, Services};
use std::collections::BTreeMap;

type Dependency = BTreeMap<String, String>;

fn dependencies(services: &Services, service: &str) -> BTreeMap<String, Dependency> {
    let name = re(r"^      ([a-z0-9_-]+):$");
    let field = re(r"^        ([a-z_]+): (.+)$");
    let mut found = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in section(&services[service], "depends_on") {
        if let Some(captured) = name.captures(&line) {
            current = Some(captured[1].to_string());
            found.insert(captured[1].to_string(), Dependency::new());
        } else if let (Some(captured), Some(current)) = (field.captures(&line), &current) {
            found
                .get_mut(current)
                .unwrap()
                .insert(captured[1].to_string(), captured[2].to_string());
        }
    }
    found
}

fn switchable(services: &Services, service: &str) -> bool {
    has(&services[service], "      dashboard.switchable: \"true\"")
}

#[test]
fn a_service_starts_once_the_services_it_tests_at_startup_are_healthy() {
    let services = all_services();
    for (service, dependency) in [
        ("prowlarr", "flaresolverr"),
        ("radarr", "prowlarr"),
        ("radarr", "qbittorrent"),
        ("sonarr", "prowlarr"),
        ("sonarr", "qbittorrent"),
    ] {
        let found = dependencies(&services, service);
        assert_eq!(
            Some("service_healthy"),
            found
                .get(dependency)
                .and_then(|fields| fields.get("condition"))
                .map(String::as_str),
            "{service} waits for {dependency}"
        );
    }
}

#[test]
fn a_service_waits_only_for_a_health_that_has_a_healthcheck() {
    let services = all_services();
    for service in services.keys() {
        for (dependency, fields) in dependencies(&services, service) {
            if fields.get("condition").map(String::as_str) == Some("service_healthy") {
                assert!(
                    !section(&services[&dependency], "healthcheck").is_empty(),
                    "{service} waits for {dependency}, which has no healthcheck"
                );
            }
        }
    }
}

#[test]
fn a_service_still_starts_when_a_switchable_service_it_waits_for_is_shut_down() {
    let services = all_services();
    for service in services.keys() {
        for (dependency, fields) in dependencies(&services, service) {
            if switchable(&services, &dependency)
                && fields.get("condition").map(String::as_str) == Some("service_healthy")
            {
                assert_eq!(
                    Some("false"),
                    fields.get("required").map(String::as_str),
                    "{service} waits for {dependency}"
                );
            }
        }
    }
}
