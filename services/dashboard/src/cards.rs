use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Section {
    Status,
    QuickActions,
    PullRequests,
    Watch,
    Downloads,
    Farm,
    Automation,
    Observation,
    Hosting,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Card {
    pub service: String,
    pub name: String,
    pub section: Section,
    pub icon: Option<String>,
    pub href: Option<String>,
    pub switchable: bool,
    pub description: Option<String>,
    pub widget: Option<String>,
    pub monitor: Option<String>,
}

impl Section {
    pub const ALL: [Section; 9] = [
        Section::Status,
        Section::QuickActions,
        Section::PullRequests,
        Section::Watch,
        Section::Downloads,
        Section::Farm,
        Section::Automation,
        Section::Observation,
        Section::Hosting,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Status => "Status",
            Section::QuickActions => "Quick actions",
            Section::PullRequests => "Pull requests",
            Section::Watch => "Watch",
            Section::Downloads => "Downloads",
            Section::Farm => "Farm",
            Section::Automation => "Automation",
            Section::Observation => "Observation",
            Section::Hosting => "Hosting",
        }
    }

    fn parse(title: &str) -> Option<Section> {
        Section::ALL
            .into_iter()
            .find(|section| section.title() == title)
    }
}

pub fn icon_file(file: &str) -> bool {
    let Some((name, extension)) = file.rsplit_once('.') else {
        return false;
    };
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && matches!(extension, "svg" | "png" | "webp")
}

pub fn card(labels: &HashMap<String, String>) -> Option<Card> {
    let label = |key: &str| {
        labels
            .get(key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(String::from)
    };
    let service = label("com.docker.compose.service")?;
    let section = Section::parse(&label("dashboard.section")?)?;
    Some(Card {
        name: label("dashboard.name").unwrap_or_else(|| service.clone()),
        section,
        icon: label("dashboard.icon").filter(|icon| icon_file(icon)),
        href: label("dashboard.href"),
        switchable: label("dashboard.switchable").as_deref() == Some("true"),
        description: label("dashboard.description"),
        widget: label("dashboard.widget"),
        monitor: label("dashboard.monitor"),
        service,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Running,
    Stopped,
    Unhealthy,
    Restarting,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Service {
    #[serde(flatten)]
    pub card: Card,
    pub id: String,
    pub state: State,
    pub shutdown: bool,
}

fn state(container: &Value) -> State {
    let status = container["Status"].as_str().unwrap_or_default();
    match container["State"].as_str() {
        Some("running") if status.contains("(unhealthy)") => State::Unhealthy,
        Some("running") => State::Running,
        Some("restarting") => State::Restarting,
        _ => State::Stopped,
    }
}

pub fn services(containers: &Value, shutdown: &BTreeSet<String>) -> Vec<Service> {
    let mut found: Vec<Service> = containers
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|container| {
            let labels: HashMap<String, String> =
                serde_json::from_value(container["Labels"].clone()).ok()?;
            let card = card(&labels)?;
            Some(Service {
                id: container["Id"].as_str()?.to_string(),
                state: state(container),
                shutdown: card.switchable && shutdown.contains(&card.service),
                card,
            })
        })
        .collect();
    found.sort_by_key(|service| {
        let order = Section::ALL
            .iter()
            .position(|section| *section == service.card.section);
        (order, service.card.name.to_lowercase())
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn labels(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn a_labelled_service_becomes_a_card() {
        let found = card(&labels(&[
            ("com.docker.compose.service", "jellyfin"),
            ("dashboard.name", "Jellyfin"),
            ("dashboard.section", "Watch"),
            ("dashboard.icon", "jellyfin.svg"),
            ("dashboard.href", "https://jellyfin.lan.example.org:443"),
            ("dashboard.switchable", "true"),
            ("dashboard.description", "Streams the Library"),
            ("dashboard.widget", "jellyfin"),
            ("dashboard.monitor", "https://jellyfin.example.org/health"),
        ]));
        let expected = Card {
            service: "jellyfin".into(),
            name: "Jellyfin".into(),
            section: Section::Watch,
            icon: Some("jellyfin.svg".into()),
            href: Some("https://jellyfin.lan.example.org:443".into()),
            switchable: true,
            description: Some("Streams the Library".into()),
            widget: Some("jellyfin".into()),
            monitor: Some("https://jellyfin.example.org/health".into()),
        };
        assert_eq!(Some(expected), found);
    }

    #[test]
    fn an_icon_outside_the_icon_names_is_dropped() {
        for icon in [
            "../shutdown.json",
            "jellyfin",
            "Jellyfin.svg",
            "a/b.svg",
            "x.exe",
        ] {
            let found = card(&labels(&[
                ("com.docker.compose.service", "jellyfin"),
                ("dashboard.section", "Watch"),
                ("dashboard.icon", icon),
            ]));
            assert_eq!(None, found.unwrap().icon, "{icon}");
        }
    }

    fn container(service: &str, section: &str, state: &str, status: &str) -> Value {
        json!({
            "Id": format!("id-{service}"),
            "State": state,
            "Status": status,
            "Labels": {
                "com.docker.compose.service": service,
                "dashboard.section": section,
                "dashboard.switchable": "true",
            },
        })
    }

    #[test]
    fn the_services_come_in_section_order_with_their_live_state() {
        let containers = json!([
            container("caddy", "Hosting", "running", "Up 2 hours (healthy)"),
            container(
                "sonarr",
                "Automation",
                "running",
                "Up 5 minutes (unhealthy)"
            ),
            container(
                "radarr",
                "Automation",
                "restarting",
                "Restarting (1) 3 seconds ago"
            ),
            container("seerr", "Watch", "exited", "Exited (0) 1 hour ago"),
            container(
                "jellyfin",
                "Watch",
                "running",
                "Up 3 days (health: starting)"
            ),
            container("helper", "Nowhere", "running", "Up 1 hour"),
            json!({"Id": "id-bare", "State": "running", "Status": "Up", "Labels": {}}),
        ]);
        let shutdown = BTreeSet::from(["seerr".to_string()]);
        let listed = services(&containers, &shutdown);
        let found: Vec<(&str, &str, State, bool)> = listed
            .iter()
            .map(|service| {
                (
                    service.card.service.as_str(),
                    service.id.as_str(),
                    service.state,
                    service.shutdown,
                )
            })
            .collect();
        let expected = vec![
            ("jellyfin", "id-jellyfin", State::Running, false),
            ("seerr", "id-seerr", State::Stopped, true),
            ("radarr", "id-radarr", State::Restarting, false),
            ("sonarr", "id-sonarr", State::Unhealthy, false),
            ("caddy", "id-caddy", State::Running, false),
        ];
        assert_eq!(expected, found);
    }

    #[test]
    fn a_listed_service_that_is_no_longer_switchable_is_not_shut_down() {
        let mut fixed = container("tdarr", "Automation", "running", "Up 1 hour");
        fixed["Labels"]
            .as_object_mut()
            .unwrap()
            .remove("dashboard.switchable");
        let shutdown = BTreeSet::from(["tdarr".to_string()]);
        let listed = services(&json!([fixed]), &shutdown);
        assert!(!listed[0].shutdown);
    }
}
