use crate::api::{Api, fetch};
use crate::input::Input;
use crate::report::{Report, Severity};
use jiff::Timestamp;
use serde_json::{Value, json};

pub fn containers(report: &mut Report, input: &Input, names: &[&str]) {
    let mut states = json!({});
    for name in names {
        let state = input.states.get(*name).map_or("missing", String::as_str);
        states[*name] = json!(state);
        if !state.starts_with("running") || state.contains("unhealthy") {
            report.flag(
                Severity::High,
                name,
                format!("The container {name} is {state}"),
            );
        }
    }
    report.fact("containers", states);
}

pub fn disabled_till(status: &Value, now: Timestamp) -> Option<String> {
    let till = status["disabledTill"].as_str()?;
    let time: Timestamp = till.parse().ok()?;
    (time > now).then(|| till.to_string())
}

fn status_of(statuses: &Value, field: &str, id: &Value) -> Value {
    statuses
        .as_array()
        .and_then(|list| list.iter().find(|status| &status[field] == id))
        .cloned()
        .unwrap_or(Value::Null)
}

pub fn prowlarr(report: &mut Report, api: &impl Api, now: Timestamp) {
    let Some(indexers) = fetch(report, api, "/api/v1/indexer") else {
        return;
    };
    let profiles = fetch(report, api, "/api/v1/appprofile").unwrap_or(json!([]));
    let statuses = fetch(report, api, "/api/v1/indexerstatus").unwrap_or(json!([]));
    let mut facts = vec![];
    for indexer in indexers.as_array().into_iter().flatten() {
        let name = indexer["name"].as_str().unwrap_or("?");
        let profile = status_of(&profiles, "id", &indexer["appProfileId"]);
        let profile_name = profile["name"].as_str().unwrap_or("?");
        let status = status_of(&statuses, "indexerId", &indexer["id"]);
        let till = disabled_till(&status, now);
        let searches = [
            "enableRss",
            "enableAutomaticSearch",
            "enableInteractiveSearch",
        ]
        .iter()
        .any(|field| profile[*field] == json!(true));
        if indexer["enable"] != json!(true) {
            report.flag(
                Severity::High,
                "prowlarr",
                format!("{name} is disabled in Prowlarr"),
            );
        }
        if !searches {
            report.flag(
                Severity::High,
                "prowlarr",
                format!("{name} sits on the sync profile {profile_name}, which turns RSS and every search off"),
            );
        }
        if let Some(till) = &till {
            report.flag(
                Severity::High,
                "prowlarr",
                format!("{name} is disabled by Prowlarr until {till} after failures"),
            );
        }
        facts.push(json!({
            "id": indexer["id"],
            "name": name,
            "enable": indexer["enable"],
            "priority": indexer["priority"],
            "profile": profile_name,
            "disabledTill": till,
            "mostRecentFailure": status["mostRecentFailure"],
        }));
    }
    if let Some(health) = fetch(report, api, "/api/v1/health") {
        flag_health(report, "prowlarr", &health);
    }
    report.fact("prowlarr", json!(facts));
}

pub fn flag_health(report: &mut Report, source: &str, health: &Value) {
    for item in health.as_array().into_iter().flatten() {
        let severity = match item["type"].as_str() {
            Some("error") => Severity::High,
            Some("warning") => Severity::Medium,
            _ => Severity::Low,
        };
        report.flag(
            severity,
            source,
            format!(
                "Health {}: {}",
                item["source"].as_str().unwrap_or("?"),
                item["message"].as_str().unwrap_or("?")
            ),
        );
    }
}

pub fn ratio(report: &mut Report, input: &Input) {
    let lines = input.logs.get("ratio").cloned().unwrap_or_default();
    let last: Vec<&String> = lines.iter().rev().take(15).collect();
    if let Some(line) = lines
        .iter()
        .rev()
        .find(|line| line.contains("Cannot read the profile"))
    {
        report.flag(
            Severity::High,
            "ratio",
            format!("ratio cannot read the C411 profile, so it spends no download on C411: {line}"),
        );
    }
    report.fact("ratio", json!(last.into_iter().rev().collect::<Vec<_>>()));
}

pub fn query_limits(report: &mut Report, input: &Input) {
    for (source, lines) in &input.logs {
        let hits = lines
            .iter()
            .filter(|line| line.contains("Query Limit") || line.contains("API Request Limit"))
            .count();
        if hits > 0 {
            report.flag(
                Severity::Medium,
                source,
                format!(
                    "{hits} log lines of {source} in 24 hours report an indexer query limit reached"
                ),
            );
        }
    }
}

pub fn qbittorrent(report: &mut Report, reached: Result<u16, String>) {
    match reached {
        Ok(status) => report.fact("qbittorrent", json!({"reachable": true, "status": status})),
        Err(error) => {
            report.flag(
                Severity::High,
                "qbittorrent",
                format!("qBittorrent does not answer at gluetun:8080: {error}"),
            );
            report.fact("qbittorrent", json!({"reachable": false}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::fake::Fake;
    use crate::input::parse;

    fn now() -> Timestamp {
        "2026-11-26T13:00:00Z".parse().unwrap()
    }

    #[test]
    fn an_indexer_on_a_paused_profile_is_flagged() {
        let api = Fake::new(&[
            (
                "/api/v1/indexer",
                json!([{"id": 3, "name": "C411", "enable": true, "priority": 25, "appProfileId": 2}]),
            ),
            (
                "/api/v1/appprofile",
                json!([{"id": 2, "name": "Paused", "enableRss": false, "enableAutomaticSearch": false, "enableInteractiveSearch": false}]),
            ),
            ("/api/v1/indexerstatus", json!([])),
            ("/api/v1/health", json!([])),
        ]);
        let mut report = Report::default();
        prowlarr(&mut report, &api, now());
        assert!(report.flagged(Severity::High, "C411 sits on the sync profile Paused"));
    }

    #[test]
    fn an_indexer_disabled_after_failures_is_flagged_until_it_comes_back() {
        let api = Fake::new(&[
            (
                "/api/v1/indexer",
                json!([{"id": 1, "name": "Nyaa.si", "enable": true, "appProfileId": 1}]),
            ),
            (
                "/api/v1/appprofile",
                json!([{"id": 1, "name": "Standard", "enableRss": true}]),
            ),
            (
                "/api/v1/indexerstatus",
                json!([{"indexerId": 1, "disabledTill": "2026-11-26T18:29:53.0427162Z"}]),
            ),
            ("/api/v1/health", json!([])),
        ]);
        let mut report = Report::default();
        prowlarr(&mut report, &api, now());
        assert!(report.flagged(Severity::High, "Nyaa.si is disabled by Prowlarr until"));
        let mut later = Report::default();
        prowlarr(&mut later, &api, "2026-11-26T20:00:00Z".parse().unwrap());
        assert!(later.findings.is_empty());
    }

    #[test]
    fn ratio_unable_to_read_the_profile_is_flagged() {
        let input = parse(
            "log|ratio|Cannot read the profile of C411, ratio spends no download on C411: io\n",
        );
        let mut report = Report::default();
        ratio(&mut report, &input);
        assert!(report.flagged(Severity::High, "cannot read the C411 profile"));
    }

    #[test]
    fn a_stopped_or_unhealthy_container_is_flagged() {
        let input = parse("state|radarr|running healthy\nstate|gluetun|running unhealthy\n");
        let mut report = Report::default();
        containers(&mut report, &input, &["radarr", "gluetun", "ratio"]);
        assert!(report.flagged(Severity::High, "gluetun is running unhealthy"));
        assert!(report.flagged(Severity::High, "ratio is missing"));
        assert!(!report.flagged(Severity::High, "radarr"));
    }
}
