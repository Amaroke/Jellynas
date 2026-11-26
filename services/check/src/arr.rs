use crate::api::{Api, fetch};
use crate::report::{Report, Severity, trimmed};
use crate::upstream::{disabled_till, flag_health};
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const LOW_SPACE_GB: f64 = 100.0;
const GB: f64 = 1_000_000_000.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Radarr,
    Sonarr,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Radarr => "radarr",
            Kind::Sonarr => "sonarr",
        }
    }
}

fn records(value: &Value) -> Vec<Value> {
    value["records"].as_array().cloned().unwrap_or_default()
}

fn gigabytes(value: &Value) -> Value {
    value.as_f64().map_or(Value::Null, |bytes| {
        json!((bytes / GB * 10.0).round() / 10.0)
    })
}

fn by_title(items: &mut [Value]) {
    items.sort_by_key(|item| item["title"].as_str().unwrap_or("").to_lowercase());
}

fn library(report: &mut Report, api: &impl Api, kind: Kind) {
    let path = match kind {
        Kind::Radarr => "/api/v3/movie",
        Kind::Sonarr => "/api/v3/series",
    };
    let Some(items) = fetch(report, api, path) else {
        return;
    };
    let items = items.as_array().cloned().unwrap_or_default();
    let monitored = items
        .iter()
        .filter(|item| item["monitored"] == json!(true))
        .count();
    let mut rows: Vec<Value> = match kind {
        Kind::Radarr => items
            .iter()
            .map(|movie| {
                json!({
                    "id": movie["id"],
                    "title": movie["title"],
                    "year": movie["year"],
                    "monitored": movie["monitored"],
                    "status": movie["status"],
                    "isAvailable": movie["isAvailable"],
                    "hasFile": movie["hasFile"],
                    "quality": movie["movieFile"]["quality"]["quality"]["name"],
                    "sizeGB": gigabytes(&movie["sizeOnDisk"]),
                    "rootFolder": movie["rootFolderPath"],
                })
            })
            .collect(),
        Kind::Sonarr => items
            .iter()
            .map(|series| {
                let statistics = &series["statistics"];
                json!({
                    "id": series["id"],
                    "title": series["title"],
                    "year": series["year"],
                    "monitored": series["monitored"],
                    "status": series["status"],
                    "seriesType": series["seriesType"],
                    "episodeFiles": statistics["episodeFileCount"],
                    "episodesWanted": statistics["episodeCount"],
                    "episodesTotal": statistics["totalEpisodeCount"],
                    "sizeGB": gigabytes(&statistics["sizeOnDisk"]),
                    "rootFolder": series["rootFolderPath"],
                })
            })
            .collect(),
    };
    by_title(&mut rows);
    let facts = match kind {
        Kind::Radarr => json!({
            "movies": items.len(),
            "monitored": monitored,
            "withFile": items.iter().filter(|item| item["hasFile"] == json!(true)).count(),
        }),
        Kind::Sonarr => json!({
            "series": items.len(),
            "monitored": monitored,
            "episodeFiles": items.iter().map(|item| item["statistics"]["episodeFileCount"].as_u64().unwrap_or(0)).sum::<u64>(),
        }),
    };
    report.fact("library", facts);
    report.fact(
        match kind {
            Kind::Radarr => "movies",
            Kind::Sonarr => "series",
        },
        json!(rows),
    );
}

fn missing(report: &mut Report, api: &impl Api, kind: Kind) {
    let path = match kind {
        Kind::Radarr => "/api/v3/wanted/missing?page=1&pageSize=200&monitored=true",
        Kind::Sonarr => {
            "/api/v3/wanted/missing?page=1&pageSize=200&monitored=true&includeSeries=true"
        }
    };
    let Some(page) = fetch(report, api, path) else {
        return;
    };
    let items: Value = match kind {
        Kind::Radarr => records(&page)
            .iter()
            .map(|movie| {
                json!({
                    "id": movie["id"],
                    "title": movie["title"],
                    "year": movie["year"],
                    "status": movie["status"],
                    "isAvailable": movie["isAvailable"],
                })
            })
            .collect(),
        Kind::Sonarr => {
            let mut counts: BTreeMap<String, u64> = BTreeMap::new();
            for episode in records(&page) {
                let title = episode["series"]["title"]
                    .as_str()
                    .unwrap_or("?")
                    .to_string();
                *counts.entry(title).or_default() += 1;
            }
            json!(counts)
        }
    };
    report.fact(
        "missing",
        json!({"total": page["totalRecords"], "items": items}),
    );
}

fn total(report: &mut Report, api: &impl Api, key: &str, path: &str) {
    if let Some(page) = fetch(report, api, path) {
        report.fact(key, page["totalRecords"].clone());
    }
}

fn queue(report: &mut Report, api: &impl Api, kind: Kind) {
    let unknown = match kind {
        Kind::Radarr => "includeUnknownMovieItems",
        Kind::Sonarr => "includeUnknownSeriesItems",
    };
    let path = format!("/api/v3/queue?page=1&pageSize=200&{unknown}=true");
    let Some(page) = fetch(report, api, &path) else {
        return;
    };
    let mut items = vec![];
    for item in records(&page) {
        let title = item["title"].as_str().unwrap_or("?");
        let tracked = item["trackedDownloadStatus"].as_str().unwrap_or("");
        let state = item["trackedDownloadState"].as_str().unwrap_or("");
        let mut messages: Vec<String> = item["statusMessages"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|status| status["messages"].as_array().cloned().unwrap_or_default())
            .filter_map(|message| message.as_str().map(String::from))
            .collect();
        if let Some(error) = item["errorMessage"].as_str() {
            messages.push(error.into());
        }
        if matches!(tracked, "warning" | "error")
            || matches!(state, "importBlocked" | "failedPending")
        {
            report.flag(
                Severity::Medium,
                kind.name(),
                format!(
                    "Queue item {title} is {tracked} {state}: {}",
                    messages.join(" / ")
                ),
            );
        }
        items.push(json!({
            "title": title,
            "status": item["status"],
            "tracked": tracked,
            "state": state,
            "timeleft": item["timeleft"],
            "messages": messages,
        }));
    }
    report.fact("queue", trimmed(&json!(items), 50));
}

fn failed(report: &mut Report, api: &impl Api, kind: Kind, now: Timestamp) {
    let path =
        "/api/v3/history?page=1&pageSize=100&sortKey=date&sortDirection=descending&eventType=4";
    let Some(page) = fetch(report, api, path) else {
        return;
    };
    let since = now - SignedDuration::from_hours(24 * 7);
    let mut items: Vec<Value> = vec![];
    for record in records(&page) {
        let Some(date) = record["date"]
            .as_str()
            .and_then(|date| date.parse::<Timestamp>().ok())
        else {
            continue;
        };
        if date < since {
            continue;
        }
        let title = record["sourceTitle"].as_str().unwrap_or("?");
        let message = record["data"]["message"].as_str().unwrap_or("");
        if let Some(item) = items
            .iter_mut()
            .find(|item| item["title"] == title && item["message"] == message)
        {
            item["count"] = json!(item["count"].as_u64().unwrap_or(1) + 1);
            continue;
        }
        if message != "Manually marked as failed" {
            report.flag(
                Severity::Low,
                kind.name(),
                format!("Download of {title} failed: {message}"),
            );
        }
        items.push(json!({"title": title, "date": record["date"], "message": message, "count": 1}));
    }
    report.fact("failedLast7Days", trimmed(&json!(items), 30));
}

fn disabled(statuses: &[String], id: &Value) -> Value {
    statuses
        .iter()
        .filter_map(|line| line.split_once('|'))
        .find(|(provider, _)| {
            id.as_u64()
                .is_some_and(|id| provider.trim() == id.to_string())
        })
        .map_or(
            Value::Null,
            |(_, till)| json!({"disabledTill": till.trim().replacen(' ', "T", 1)}),
        )
}

fn indexers(report: &mut Report, api: &impl Api, kind: Kind, now: Timestamp, statuses: &[String]) {
    let Some(indexers) = fetch(report, api, "/api/v3/indexer") else {
        return;
    };
    let mut usable = 0;
    let mut facts = vec![];
    for indexer in indexers.as_array().into_iter().flatten() {
        let status = disabled(statuses, &indexer["id"]);
        let till = disabled_till(&status, now);
        let rss = indexer["enableRss"] == json!(true);
        if rss && till.is_none() {
            usable += 1;
        }
        facts.push(json!({
            "name": indexer["name"],
            "rss": rss,
            "automaticSearch": indexer["enableAutomaticSearch"],
            "interactiveSearch": indexer["enableInteractiveSearch"],
            "priority": indexer["priority"],
            "disabledTill": till,
        }));
    }
    if usable == 0 {
        report.flag(
            Severity::High,
            kind.name(),
            "No indexer can be used for RSS right now, so nothing new is grabbed",
        );
    }
    report.fact("indexers", json!(facts));
}

fn clients(report: &mut Report, api: &impl Api, kind: Kind) {
    let Some(clients) = fetch(report, api, "/api/v3/downloadclient") else {
        return;
    };
    let list = clients.as_array().cloned().unwrap_or_default();
    if !list.iter().any(|client| client["enable"] == json!(true)) {
        report.flag(Severity::High, kind.name(), "No download client is enabled");
    }
    let facts: Vec<Value> = list
        .iter()
        .map(|client| json!({"name": client["name"], "enable": client["enable"]}))
        .collect();
    report.fact("downloadClients", json!(facts));
}

fn roots(report: &mut Report, api: &impl Api, kind: Kind) {
    let Some(roots) = fetch(report, api, "/api/v3/rootfolder") else {
        return;
    };
    let mut facts = vec![];
    for root in roots.as_array().into_iter().flatten() {
        let path = root["path"].as_str().unwrap_or("?");
        let free = root["freeSpace"].as_f64().map(|bytes| (bytes / GB).round());
        if root["accessible"] == json!(false) {
            report.flag(
                Severity::High,
                kind.name(),
                format!("Root folder {path} is not accessible"),
            );
        }
        if let Some(free) = free.filter(|free| *free < LOW_SPACE_GB) {
            report.flag(
                Severity::Medium,
                kind.name(),
                format!("Root folder {path} has only {free} GB free"),
            );
        }
        facts.push(json!({"path": path, "accessible": root["accessible"], "freeGB": free}));
    }
    report.fact("rootFolders", json!(facts));
}

pub fn check(report: &mut Report, api: &impl Api, kind: Kind, now: Timestamp, statuses: &[String]) {
    if let Some(status) = fetch(report, api, "/api/v3/system/status") {
        report.fact("version", status["version"].clone());
    }
    if let Some(health) = fetch(report, api, "/api/v3/health") {
        flag_health(report, kind.name(), &health);
        report.fact("health", health);
    }
    library(report, api, kind);
    missing(report, api, kind);
    total(
        report,
        api,
        "cutoffUnmet",
        "/api/v3/wanted/cutoff?page=1&pageSize=1&monitored=true",
    );
    queue(report, api, kind);
    failed(report, api, kind, now);
    indexers(report, api, kind, now, statuses);
    clients(report, api, kind);
    roots(report, api, kind);
    total(
        report,
        api,
        "blocklist",
        "/api/v3/blocklist?page=1&pageSize=1",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::fake::Fake;

    fn now() -> Timestamp {
        "2026-11-26T13:00:00Z".parse().unwrap()
    }

    fn sonarr(extra: &[(&str, Value)]) -> Fake {
        let mut pages = vec![
            ("/api/v3/system/status", json!({"version": "4.0.20"})),
            ("/api/v3/health", json!([])),
            (
                "/api/v3/series",
                json!([
                    {"id": 2, "title": "VisionQuest", "monitored": true, "statistics": {"episodeFileCount": 10, "episodeCount": 12, "sizeOnDisk": 2.5e9}},
                    {"id": 1, "title": "Arcane", "monitored": true, "statistics": {"episodeFileCount": 9}},
                ]),
            ),
            (
                "/api/v3/wanted/missing?page=1&pageSize=200&monitored=true&includeSeries=true",
                json!({"totalRecords": 2, "records": [{"series": {"title": "VisionQuest"}}, {"series": {"title": "VisionQuest"}}]}),
            ),
            (
                "/api/v3/wanted/cutoff?page=1&pageSize=1&monitored=true",
                json!({"totalRecords": 4}),
            ),
            (
                "/api/v3/queue?page=1&pageSize=200&includeUnknownSeriesItems=true",
                json!({"records": []}),
            ),
            (
                "/api/v3/history?page=1&pageSize=100&sortKey=date&sortDirection=descending&eventType=4",
                json!({"records": []}),
            ),
            (
                "/api/v3/indexer",
                json!([
                    {"id": 1, "name": "Nyaa.si (Prowlarr)", "enableRss": true},
                    {"id": 3, "name": "C411 (Prowlarr)", "enableRss": false},
                ]),
            ),
            (
                "/api/v3/downloadclient",
                json!([{"name": "qBittorrent", "enable": true}]),
            ),
            (
                "/api/v3/rootfolder",
                json!([{"path": "/data/media/shows/", "accessible": true, "freeSpace": 2.0e12}]),
            ),
            (
                "/api/v3/blocklist?page=1&pageSize=1",
                json!({"totalRecords": 0}),
            ),
        ];
        pages.extend(extra.iter().cloned());
        Fake::new(&pages)
    }

    #[test]
    fn a_healthy_sonarr_has_no_finding() {
        let mut report = Report::default();
        check(&mut report, &sonarr(&[]), Kind::Sonarr, now(), &[]);
        assert_eq!(report.findings, vec![]);
        assert!(report.errors.is_empty());
        assert_eq!(report.facts["missing"]["items"]["VisionQuest"], 2);
        assert_eq!(report.facts["cutoffUnmet"], 4);
        assert_eq!(report.facts["series"][0]["title"], "Arcane");
        assert_eq!(report.facts["series"][1]["episodesWanted"], 12);
        assert_eq!(report.facts["series"][1]["sizeGB"], 2.5);
    }

    #[test]
    fn no_indexer_left_for_rss_is_flagged() {
        let statuses = vec![
            "1|2026-11-26 19:06:09.4211604Z".to_string(),
            "3|".to_string(),
        ];
        let mut report = Report::default();
        check(&mut report, &sonarr(&[]), Kind::Sonarr, now(), &statuses);
        assert!(report.flagged(Severity::High, "No indexer can be used for RSS"));
        assert_eq!(
            report.facts["indexers"][0]["disabledTill"],
            "2026-11-26T19:06:09.4211604Z"
        );
    }

    #[test]
    fn a_blocked_import_is_flagged_with_its_reason() {
        let api = sonarr(&[(
            "/api/v3/queue?page=1&pageSize=200&includeUnknownSeriesItems=true",
            json!({"records": [{
                "title": "Show.S01E01",
                "trackedDownloadStatus": "warning",
                "trackedDownloadState": "importBlocked",
                "statusMessages": [{"messages": ["No files found are eligible for import"]}],
            }]}),
        )]);
        let mut report = Report::default();
        check(&mut report, &api, Kind::Sonarr, now(), &[]);
        assert!(report.flagged(Severity::Medium, "No files found are eligible for import"));
    }

    #[test]
    fn only_failures_not_marked_by_hand_are_flagged() {
        let api = sonarr(&[(
            "/api/v3/history?page=1&pageSize=100&sortKey=date&sortDirection=descending&eventType=4",
            json!({"records": [
                {"sourceTitle": "A", "date": "2026-11-25T10:00:00Z", "data": {"message": "Manually marked as failed"}},
                {"sourceTitle": "A", "date": "2026-11-25T10:00:00Z", "data": {"message": "Manually marked as failed"}},
                {"sourceTitle": "B", "date": "2026-11-25T10:00:00Z", "data": {"message": "Disk full"}},
                {"sourceTitle": "C", "date": "2026-10-01T10:00:00Z", "data": {"message": "Old"}},
            ]}),
        )]);
        let mut report = Report::default();
        check(&mut report, &api, Kind::Sonarr, now(), &[]);
        assert_eq!(report.findings.len(), 1);
        assert!(report.flagged(Severity::Low, "Download of B failed: Disk full"));
        assert_eq!(report.facts["failedLast7Days"]["count"], 2);
        assert_eq!(report.facts["failedLast7Days"]["items"][0]["count"], 2);
    }

    #[test]
    fn a_full_or_missing_root_folder_is_flagged() {
        let api = sonarr(&[(
            "/api/v3/rootfolder",
            json!([
                {"path": "/a", "accessible": false},
                {"path": "/b", "accessible": true, "freeSpace": 5.0e10},
            ]),
        )]);
        let mut report = Report::default();
        check(&mut report, &api, Kind::Sonarr, now(), &[]);
        assert!(report.flagged(Severity::High, "Root folder /a is not accessible"));
        assert!(report.flagged(Severity::Medium, "Root folder /b has only 50 GB free"));
    }

    #[test]
    fn an_endpoint_that_fails_is_reported_and_the_rest_goes_on() {
        let mut api = sonarr(&[]);
        api.0.remove("/api/v3/health");
        let mut report = Report::default();
        check(&mut report, &api, Kind::Sonarr, now(), &[]);
        assert!(report.errors[0].starts_with("/api/v3/health"));
        assert!(report.facts.contains_key("rootFolders"));
    }
}
