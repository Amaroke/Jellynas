use crate::cards::{Service, State};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

pub struct Shutdown {
    path: PathBuf,
    services: BTreeSet<String>,
}

impl Shutdown {
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let services = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string())?,
            Err(error) if error.kind() == ErrorKind::NotFound => BTreeSet::new(),
            Err(error) => return Err(error.to_string()),
        };
        Ok(Shutdown { path, services })
    }

    pub fn services(&self) -> &BTreeSet<String> {
        &self.services
    }

    pub fn turn_off(&mut self, service: &str) -> Result<(), String> {
        let mut services = self.services.clone();
        services.insert(service.to_string());
        self.save(services)
    }

    pub fn turn_on(&mut self, service: &str) -> Result<(), String> {
        let mut services = self.services.clone();
        services.remove(service);
        self.save(services)
    }

    fn save(&mut self, services: BTreeSet<String>) -> Result<(), String> {
        let written = self.path.with_extension("tmp");
        let bytes = serde_json::to_vec(&services).map_err(|error| error.to_string())?;
        fs::write(&written, bytes)
            .and_then(|()| fs::rename(&written, &self.path))
            .map_err(|error| error.to_string())?;
        self.services = services;
        Ok(())
    }
}

pub fn to_stop(event: &Value, shutdown: &BTreeSet<String>) -> Option<String> {
    let actor = &event["Actor"];
    let service = actor["Attributes"]["com.docker.compose.service"].as_str()?;
    let started = event["Type"] == "container" && event["Action"] == "start";
    (started && shutdown.contains(service)).then(|| actor["ID"].as_str().map(String::from))?
}

pub fn unstarted(containers: &Value, shutdown: &BTreeSet<String>) -> Vec<String> {
    containers
        .as_array()
        .into_iter()
        .flatten()
        .filter(|container| container["State"] == "created")
        .filter(|container| container["Labels"]["dashboard.switchable"] == "true")
        .filter(|container| {
            container["Labels"]["com.docker.compose.service"]
                .as_str()
                .is_some_and(|service| !shutdown.contains(service))
        })
        .filter_map(|container| container["Id"].as_str().map(String::from))
        .collect()
}

pub fn strays(services: &[Service]) -> Vec<String> {
    services
        .iter()
        .filter(|service| service.shutdown && service.state != State::Stopped)
        .map(|service| service.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::{Card, Section};
    use serde_json::json;

    fn event(action: &str, service: &str) -> Value {
        json!({
            "Type": "container",
            "Action": action,
            "Actor": {
                "ID": format!("id-{service}"),
                "Attributes": {"com.docker.compose.service": service, "name": service},
            },
        })
    }

    fn shut_down(services: &[&str]) -> BTreeSet<String> {
        services.iter().map(|service| service.to_string()).collect()
    }

    #[test]
    fn a_switchable_container_the_deploy_left_unstarted_starts_unless_shut_down() {
        let container = |service: &str, state: &str, switchable: &str| {
            json!({
                "Id": format!("id-{service}"),
                "State": state,
                "Labels": {
                    "com.docker.compose.service": service,
                    "dashboard.switchable": switchable,
                },
            })
        };
        let containers = json!([
            container("aurral", "created", "true"),
            container("tdarr", "created", "true"),
            container("caddy", "created", "false"),
            container("bazarr", "exited", "true"),
            container("sonarr", "running", "true"),
        ]);
        assert_eq!(
            vec!["id-aurral".to_string()],
            unstarted(&containers, &shut_down(&["tdarr"]))
        );
    }

    #[test]
    fn a_shut_down_service_that_starts_again_is_stopped_at_once() {
        let found = to_stop(&event("start", "tdarr"), &shut_down(&["tdarr"]));
        assert_eq!(Some("id-tdarr".to_string()), found);
    }

    #[test]
    fn a_service_that_is_not_shut_down_starts_freely() {
        let found = to_stop(&event("start", "sonarr"), &shut_down(&["tdarr"]));
        assert_eq!(None, found);
    }

    #[test]
    fn only_a_start_of_a_shut_down_service_stops_it() {
        for action in [
            "stop",
            "die",
            "health_status: healthy",
            "exec_start: sh",
            "destroy",
        ] {
            let found = to_stop(&event(action, "tdarr"), &shut_down(&["tdarr"]));
            assert_eq!(None, found, "{action}");
        }
    }

    #[test]
    fn an_event_of_another_type_stops_nothing() {
        let mut network = event("start", "tdarr");
        network["Type"] = json!("network");
        assert_eq!(None, to_stop(&network, &shut_down(&["tdarr"])));
    }

    fn service(name: &str, state: State, shutdown: bool) -> Service {
        Service {
            card: Card {
                service: name.into(),
                name: name.into(),
                section: Section::Automation,
                icon: None,
                href: None,
                switchable: true,
                description: None,
                widget: None,
                monitor: None,
            },
            id: format!("id-{name}"),
            state,
            shutdown,
        }
    }

    #[test]
    fn a_shut_down_service_found_up_is_stopped() {
        let services = [
            service("tdarr", State::Running, true),
            service("bazarr", State::Restarting, true),
            service("radarr", State::Unhealthy, true),
            service("sonarr", State::Stopped, true),
            service("prowlarr", State::Running, false),
        ];
        assert_eq!(
            vec!["id-tdarr", "id-bazarr", "id-radarr"],
            strays(&services)
        );
    }

    #[test]
    fn the_shut_down_services_survive_a_restart_of_the_dashboard() {
        let state = tempfile::tempdir().unwrap();
        let path = state.path().join("shutdown.json");
        let mut list = Shutdown::load(path.clone()).unwrap();
        assert!(list.services().is_empty());
        list.turn_off("tdarr").unwrap();
        list.turn_off("bazarr").unwrap();
        list.turn_on("tdarr").unwrap();
        let reloaded = Shutdown::load(path).unwrap();
        assert_eq!(&shut_down(&["bazarr"]), reloaded.services());
    }
}
