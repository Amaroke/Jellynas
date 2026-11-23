use super::format::{bytes, count, day, duration, epoch, percent, rate};
use super::{Figure, Health, Link, Poster, Reading, Row, Uptime};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

pub type Parsed = Result<Reading, String>;
pub type Linked = Result<(Vec<Link>, Vec<Health>), String>;

const ROWS: usize = 8;
const POSTERS: usize = 8;
const NOTHING_PLAYING: &str = "Nothing playing";

fn artwork(images: &str, id: &str, height: u32) -> Option<String> {
    let plain = !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    plain.then(|| format!("{images}/Items/{id}/Images/Primary?maxHeight={height}"))
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    value
        .get(key)
        .filter(|found| !found.is_null())
        .ok_or_else(|| format!("missing {key}"))
}

fn number(value: &Value, key: &str) -> Result<f64, String> {
    field(value, key)?
        .as_f64()
        .ok_or_else(|| format!("{key} is not a number"))
}

fn whole(value: &Value, key: &str) -> Result<u64, String> {
    Ok(number(value, key)?.max(0.0) as u64)
}

fn text(value: &Value, key: &str) -> Result<String, String> {
    field(value, key)?
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("{key} is not a text"))
}

fn items(value: &Value) -> Result<&Vec<Value>, String> {
    value.as_array().ok_or_else(|| "not a list".to_string())
}

fn reading(figures: Vec<(&str, String)>) -> Reading {
    Reading {
        figures: figures
            .into_iter()
            .map(|(label, value)| Figure {
                label: label.into(),
                value,
            })
            .collect(),
        rows: vec![],
        posters: vec![],
        shelf: None,
        idle: None,
        links: vec![],
        health: vec![],
        metrics: vec![],
    }
}

fn playing(session: &Value, images: &str) -> Option<Row> {
    let item = session.get("NowPlayingItem")?;
    let name = item["Name"].as_str()?;
    let title = match item["SeriesName"].as_str() {
        Some(series) => match (
            item["ParentIndexNumber"].as_u64(),
            item["IndexNumber"].as_u64(),
        ) {
            (Some(season), Some(episode)) => {
                format!("{series} S{season:02}E{episode:02} · {name}")
            }
            _ => format!("{series} · {name}"),
        },
        None => name.to_string(),
    };
    let state = &session["PlayState"];
    let mut detail = session["UserName"]
        .as_str()
        .unwrap_or("someone")
        .to_string();
    let method = match state["PlayMethod"].as_str() {
        Some("Transcode") => Some("transcode"),
        Some("DirectPlay") => Some("direct play"),
        Some("DirectStream") => Some("direct stream"),
        _ => None,
    };
    if let Some(method) = method {
        detail.push_str(" · ");
        detail.push_str(method);
    }
    if state["IsPaused"].as_bool() == Some(true) {
        detail.push_str(" · paused");
    }
    let image = item["SeriesId"]
        .as_str()
        .or(item["AlbumId"].as_str())
        .or(item["Id"].as_str())
        .and_then(|id| artwork(images, id, 160));
    let runtime = item["RunTimeTicks"].as_f64().filter(|ticks| *ticks > 0.0);
    let progress = runtime.and_then(|runtime| Some(state["PositionTicks"].as_f64()? / runtime));
    Some(Row {
        title,
        detail,
        progress,
        image,
        id: None,
    })
}

fn addition(item: &Value, images: &str) -> Option<(String, Poster)> {
    let (key, title, detail) = match item["SeriesId"].as_str() {
        Some(series) => {
            let detail = match (
                item["ParentIndexNumber"].as_u64(),
                item["IndexNumber"].as_u64(),
            ) {
                (Some(season), Some(episode)) => format!("S{season:02}E{episode:02}"),
                _ => item["Name"].as_str()?.to_string(),
            };
            (series, item["SeriesName"].as_str()?, detail)
        }
        None => {
            let detail = item["ProductionYear"]
                .as_u64()
                .map_or_else(|| "Movie".to_string(), |year| year.to_string());
            (item["Id"].as_str()?, item["Name"].as_str()?, detail)
        }
    };
    let poster = Poster {
        title: title.to_string(),
        detail,
        image: artwork(images, key, 300)?,
    };
    Some((key.to_string(), poster))
}

const WEEK: &str = "Next 7 days";

fn poster_of(images: &Value) -> String {
    images
        .as_array()
        .into_iter()
        .flatten()
        .find(|image| image["coverType"].as_str() == Some("poster"))
        .and_then(|image| image["remoteUrl"].as_str())
        .filter(|url| url.starts_with("https://"))
        .unwrap_or("")
        .to_string()
}

fn within_week(date: &str, today: &str) -> bool {
    let (Some(at), Some(start)) = (
        epoch(&format!("{}T00:00:00Z", date.get(..10).unwrap_or(""))),
        epoch(&format!("{today}T00:00:00Z")),
    ) else {
        return false;
    };
    at >= start && at < start + 7 * 86400
}

pub fn jellyfin(counts: &Value, sessions: &Value, latest: &Value, images: &str) -> Parsed {
    let mut found = reading(vec![
        ("Movies", count(whole(counts, "MovieCount")?)),
        ("Series", count(whole(counts, "SeriesCount")?)),
        ("Episodes", count(whole(counts, "EpisodeCount")?)),
    ]);
    found.rows = items(sessions)?
        .iter()
        .filter_map(|session| playing(session, images))
        .collect();
    found.shelf = Some("Latest additions".to_string());
    found.idle = Some(NOTHING_PLAYING.to_string());
    let mut seen = Vec::new();
    for (key, poster) in items(field(latest, "Items")?)?
        .iter()
        .filter_map(|item| addition(item, images))
    {
        if found.posters.len() == POSTERS {
            break;
        }
        if !seen.contains(&key) {
            seen.push(key);
            found.posters.push(poster);
        }
    }
    Ok(found)
}

pub fn music_bot(status: &Value, images: &str) -> Parsed {
    let state = text(status, "state")?;
    let channel = text(status, "channel")?;
    let mut found = reading(vec![
        ("State", state.clone()),
        ("Queue", count(whole(status, "queue")?)),
        ("Shuffle", text(status, "shuffle")?),
        ("Channel", channel.clone()),
    ]);
    found.idle = Some(NOTHING_PLAYING.to_string());
    let track = text(status, "track")?;
    if state != "Idle" && track != "Nothing" {
        found.rows = vec![Row {
            title: track,
            detail: format!("{state} in {channel}"),
            progress: None,
            image: status["artwork"]
                .as_str()
                .and_then(|id| artwork(images, id, 160)),
            id: None,
        }];
    }
    Ok(found)
}

fn ago(now: u64, then: u64) -> String {
    format!("{} ago", duration(now.saturating_sub(then)))
}

fn failure(title: &str, detail: &str) -> Row {
    Row {
        title: title.to_string(),
        detail: detail.to_string(),
        progress: None,
        image: None,
        id: None,
    }
}

const COMPLETED: &str = "job completed successfully";

pub fn deploy(containers: &Value, logs: &str, now: u64) -> Parsed {
    let (at, sha) = items(containers)?
        .iter()
        .filter_map(|container| {
            let labels = &container["Labels"];
            let at = epoch(labels["cd.doco.deployment.timestamp"].as_str()?)?;
            Some((at, labels["cd.doco.deployment.target.sha"].as_str()?))
        })
        .max_by_key(|(at, _)| *at)
        .ok_or("no deploy found")?;
    let error = logs
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["time"].as_str().and_then(epoch) >= Some(at))
        .find(|entry| {
            let failed = entry["level"]
                .as_str()
                .is_some_and(|level| level.eq_ignore_ascii_case("error"));
            failed || entry["msg"].as_str() == Some(COMPLETED)
        })
        .filter(|entry| entry["msg"].as_str() != Some(COMPLETED));
    let mut found = reading(vec![
        ("Commit", sha.chars().take(7).collect()),
        ("Deployed", ago(now, at)),
        (
            "Result",
            if error.is_some() { "Failed" } else { "Success" }.to_string(),
        ),
    ]);
    found.idle = Some("No error since the deploy".to_string());
    if let Some(entry) = error {
        found.rows = vec![failure(
            entry["msg"].as_str().unwrap_or("error"),
            entry["error"].as_str().unwrap_or(""),
        )];
    }
    Ok(found)
}

fn logfmt<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let marker = format!("{key}=");
    let start = if line.starts_with(&marker) {
        marker.len()
    } else {
        line.find(&format!(" {marker}"))? + marker.len() + 1
    };
    let rest = &line[start..];
    match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next(),
        None => rest.split(' ').next(),
    }
}

const RUNNING: u64 = 3 * 3600;
const NIGHTLY: u64 = 27 * 3600;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Archive {
    pub at: u64,
    pub size: u64,
}

pub fn backup(logs: &str, archives: &[Archive], now: u64) -> Parsed {
    let mut started = None;
    let mut run = None;
    let mut stored = None;
    let mut took = None;
    let mut error = None;
    for line in logs.lines() {
        let Some(at) = logfmt(line, "time").and_then(epoch) else {
            continue;
        };
        let message = logfmt(line, "msg").unwrap_or("");
        if message.starts_with("Now running script") {
            run = Some(at);
            error = None;
        } else if message.starts_with("Stored copy of backup") {
            stored = Some(at);
            took = run.map(|start| at.saturating_sub(start));
            started = run;
        } else if logfmt(line, "level") == Some("ERROR") {
            error = Some(failure(message, logfmt(line, "error").unwrap_or("")));
        }
    }
    let newest = archives.iter().map(|archive| archive.at).max();
    let last = stored.max(newest);
    let missed = newest.is_some_and(|at| now.saturating_sub(at) > NIGHTLY);
    let result = if error.is_some() {
        "Failed"
    } else if stored.is_some() && run == started && !missed {
        "Success"
    } else if run.is_some_and(|at| now.saturating_sub(at) < RUNNING && stored < Some(at)) {
        "Running"
    } else if run.is_some() && stored < run {
        "Interrupted"
    } else if missed {
        "Missed"
    } else if newest.is_some() {
        "Success"
    } else {
        "Unknown"
    };
    let size: u64 = archives.iter().map(|archive| archive.size).sum();
    let mut found = reading(vec![
        (
            "Last backup",
            last.map_or_else(|| "—".to_string(), |at| ago(now, at)),
        ),
        ("Took", took.map_or_else(|| "—".to_string(), duration)),
        ("Result", result.to_string()),
        (
            "Archives",
            format!("{} · {}", count(archives.len() as u64), bytes(size as f64)),
        ),
    ]);
    found.idle = Some("No error in the last run".to_string());
    found.rows = error.into_iter().collect();
    if result == "Missed" {
        found.rows.push(failure(
            "The nightly backup left no archive",
            &format!(
                "Last archive {}",
                last.map_or_else(String::new, |at| ago(now, at))
            ),
        ));
    }
    Ok(found)
}

fn request(entry: &Value) -> Option<Row> {
    let id = entry["id"].as_u64()?;
    let details = &entry["details"];
    let name = details["title"].as_str().or(details["name"].as_str());
    let year = details["releaseDate"]
        .as_str()
        .or(details["firstAirDate"].as_str())
        .and_then(|date| date.get(..4));
    let title = match (name, year) {
        (Some(name), Some(year)) => format!("{name} ({year})"),
        (Some(name), None) => name.to_string(),
        _ => format!("Request {id}"),
    };
    let kind = if entry["type"].as_str() == Some("tv") {
        let seasons: Vec<String> = entry["seasons"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|season| season["seasonNumber"].as_u64())
            .map(|number| number.to_string())
            .collect();
        if seasons.is_empty() {
            "Series".to_string()
        } else {
            format!("Series, seasons {}", seasons.join(", "))
        }
    } else {
        "Movie".to_string()
    };
    let by = entry["requestedBy"]["displayName"]
        .as_str()
        .unwrap_or("someone");
    Some(Row {
        title,
        detail: format!("{kind} \u{b7} {by}"),
        progress: None,
        image: details["posterPath"]
            .as_str()
            .filter(|path| path.starts_with('/') && !path.contains(".."))
            .map(|path| format!("https://image.tmdb.org/t/p/w154{path}")),
        id: Some(id),
    })
}

pub fn aurral(worker: &Value, active: &Value) -> Parsed {
    let paused = worker["paused"].as_bool() == Some(true);
    let downloading = items(field(active, "tracks")?)?.len() as u64;
    let mut found = reading(vec![
        (
            "Downloads",
            if paused { "Paused" } else { "On" }.to_string(),
        ),
        ("Downloading", count(downloading)),
    ]);
    found.links = vec![Link {
        kind: "worker".into(),
        id: 0,
        name: "Downloads".into(),
        hint: None,
        state: if paused { "off" } else { "on" }.into(),
        until: None,
    }];
    Ok(found)
}

pub fn seerr(requests: &Value, issues: &Value, pending: &Value) -> Parsed {
    let mut found = reading(vec![
        ("Pending", count(whole(requests, "pending")?)),
        ("Processing", count(whole(requests, "processing")?)),
        ("Available", count(whole(requests, "available")?)),
        (
            "Issues",
            format!(
                "{} / {}",
                count(whole(issues, "open")?),
                count(whole(issues, "total")?)
            ),
        ),
    ]);
    found.idle = Some("No pending request".to_string());
    found.rows = items(field(pending, "results")?)?
        .iter()
        .filter_map(request)
        .take(ROWS)
        .collect();
    Ok(found)
}

pub fn badge(svg: &str) -> Option<f64> {
    let end = svg.rfind("%<")?;
    let start = svg[..end].rfind('>')? + 1;
    svg[start..end].trim().parse().ok()
}

pub fn uptimes(
    page: &Value,
    heartbeat: &Value,
    week: &HashMap<u64, f64>,
) -> Result<BTreeMap<String, Uptime>, String> {
    let days = field(heartbeat, "uptimeList")?;
    let round = |value: f64| (value * 100.0).round() / 100.0;
    Ok(items(field(page, "publicGroupList")?)?
        .iter()
        .filter_map(|group| group["monitorList"].as_array())
        .flatten()
        .filter_map(|monitor| {
            let id = monitor["id"].as_u64()?;
            let uptime = Uptime {
                day: days[format!("{id}_24")]
                    .as_f64()
                    .map(|day| round(day * 100.0)),
                week: week.get(&id).copied(),
            };
            Some((monitor["name"].as_str()?.to_string(), uptime))
        })
        .collect())
}

pub fn gluetun(ip: &Value) -> Parsed {
    Ok(reading(vec![
        ("Public IP", text(ip, "public_ip")?),
        ("Region", text(ip, "region")?),
        ("Country", text(ip, "country")?),
    ]))
}

fn missing(entry: &Value) -> String {
    let names: Vec<&str> = entry["missing_subtitles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|language| language["name"].as_str())
        .collect();
    if names.is_empty() {
        "Missing subtitles".to_string()
    } else {
        format!("Missing {}", names.join(", "))
    }
}

pub fn bazarr(movies: &Value, episodes: &Value) -> Parsed {
    let mut found = reading(vec![
        ("Missing episodes", count(whole(episodes, "total")?)),
        ("Missing movies", count(whole(movies, "total")?)),
    ]);
    found.idle = Some("No subtitles missing".to_string());
    let shows = episodes["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let title = match entry["episode_number"].as_str() {
                Some(number) => format!("{} {number}", entry["seriesTitle"].as_str()?),
                None => entry["seriesTitle"].as_str()?.to_string(),
            };
            Some(failure(&title, &missing(entry)))
        });
    let films: Vec<Row> = movies["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| Some(failure(entry["title"].as_str()?, &missing(entry))))
        .take(ROWS / 2)
        .collect();
    found.rows = shows.take(ROWS - films.len()).chain(films).collect();
    Ok(found)
}

fn failing_until(statuses: &Value, id: u64, now: u64) -> Option<u64> {
    statuses
        .as_array()?
        .iter()
        .filter(|status| status["indexerId"].as_u64() == Some(id))
        .filter_map(|status| status["disabledTill"].as_str().and_then(epoch))
        .find(|until| *until > now)
}

fn said(health: &Value, expected: &[&str]) -> Vec<Health> {
    health
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| {
            !item["source"]
                .as_str()
                .is_some_and(|source| expected.contains(&source))
        })
        .filter_map(|item| {
            let level = item["type"].as_str()?;
            let message = item["message"].as_str()?;
            matches!(level, "error" | "warning").then(|| Health {
                level: level.into(),
                message: message.into(),
            })
        })
        .collect()
}

fn unavailable(health: &Value, source: &str, name: &str) -> Result<bool, String> {
    Ok(items(health)?
        .iter()
        .filter(|item| item["source"].as_str() == Some(source))
        .any(|item| {
            item["type"].as_str() == Some("error")
                || item["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(name))
        }))
}

fn link(kind: &str, item: &Value, name: String, hint: Option<String>) -> Result<Link, String> {
    Ok(Link {
        kind: kind.into(),
        id: whole(item, "id")?,
        name,
        hint,
        state: "on".into(),
        until: None,
    })
}

pub fn prowlarr_links(
    indexers: &Value,
    statuses: &Value,
    profiles: &Value,
    proxies: &Value,
    health: &Value,
    now: u64,
) -> Linked {
    let paused: Vec<u64> = items(profiles)?
        .iter()
        .filter(|profile| profile["name"].as_str() == Some("Paused"))
        .filter_map(|profile| profile["id"].as_u64())
        .collect();
    let mut found = Vec::new();
    for indexer in items(indexers)? {
        let mut item = link(
            "indexer",
            indexer,
            text(indexer, "name")?,
            indexer["protocol"].as_str().map(String::from),
        )?;
        let until = failing_until(statuses, item.id, now);
        item.state = if indexer["enable"].as_bool() != Some(true) {
            "off"
        } else if until.is_some() {
            "failing"
        } else if indexer["appProfileId"]
            .as_u64()
            .is_some_and(|profile| paused.contains(&profile))
        {
            "paused"
        } else {
            "on"
        }
        .into();
        item.until = until.filter(|_| item.state == "failing");
        found.push(item);
    }
    found.sort_by_key(|item| item.name.to_lowercase());
    let mut gateways = Vec::new();
    for proxy in items(proxies)? {
        let name = text(proxy, "name")?;
        let mut item = link("proxy", proxy, name.clone(), None)?;
        if unavailable(health, "IndexerProxyStatusCheck", &name)? {
            item.state = "failing".into();
        }
        gateways.push(item);
    }
    gateways.sort_by_key(|item| item.name.to_lowercase());
    found.extend(gateways);
    Ok((found, said(health, &[])))
}

pub fn switched(widget: &str, kind: &str, item: &Value, on: bool) -> Value {
    let mut item = item.clone();
    match (widget, kind) {
        ("prowlarr", "indexer") | (_, "client") => item["enable"] = Value::Bool(on),
        (_, "indexer") => {
            let rss = on && item["supportsRss"].as_bool() != Some(false);
            let search = on && item["supportsSearch"].as_bool() != Some(false);
            item["enableRss"] = Value::Bool(rss);
            item["enableAutomaticSearch"] = Value::Bool(search);
            item["enableInteractiveSearch"] = Value::Bool(search);
        }
        _ => {}
    }
    item
}

pub fn starr_links(indexers: &Value, clients: &Value, health: &Value) -> Linked {
    let on =
        |item: &Value, keys: &[&str]| keys.iter().any(|key| item[*key].as_bool() == Some(true));
    let mut found = Vec::new();
    for indexer in items(indexers)? {
        let full = text(indexer, "name")?;
        let (name, hint) = match full.strip_suffix(" (Prowlarr)") {
            Some(name) => (name.to_string(), Some("from Prowlarr".to_string())),
            None => (full.clone(), None),
        };
        let mut item = link("indexer", indexer, name, hint)?;
        let failing = unavailable(health, "IndexerStatusCheck", &full)?
            || unavailable(health, "IndexerLongTermStatusCheck", &full)?;
        item.state = if !on(
            indexer,
            &[
                "enableRss",
                "enableAutomaticSearch",
                "enableInteractiveSearch",
            ],
        ) {
            "off"
        } else if failing {
            "failing"
        } else {
            "on"
        }
        .into();
        found.push(item);
    }
    found.sort_by_key(|item| item.name.to_lowercase());
    let mut takers = Vec::new();
    for client in items(clients)? {
        let name = text(client, "name")?;
        let mut item = link("client", client, name.clone(), None)?;
        if !on(client, &["enable"]) {
            item.state = "off".into();
        } else if unavailable(health, "DownloadClientStatusCheck", &name)? {
            item.state = "failing".into();
        }
        takers.push(item);
    }
    takers.sort_by_key(|item| item.name.to_lowercase());
    let every_off = |kind: &str| {
        found
            .iter()
            .chain(&takers)
            .filter(|item| item.kind == kind)
            .all(|item| item.state == "off")
    };
    let mut expected = Vec::new();
    if every_off("indexer") {
        expected.extend(["IndexerRssCheck", "IndexerSearchCheck"]);
    }
    if every_off("client") {
        expected.push("DownloadClientCheck");
    }
    found.extend(takers);
    Ok((found, said(health, &expected)))
}

pub fn prowlarr(stats: &Value) -> Parsed {
    let mut totals = [0; 4];
    for indexer in items(field(stats, "indexers")?)? {
        for (total, key) in totals.iter_mut().zip([
            "numberOfGrabs",
            "numberOfQueries",
            "numberOfFailedGrabs",
            "numberOfFailedQueries",
        ]) {
            *total += whole(indexer, key)?;
        }
    }
    Ok(reading(vec![
        ("Grabs", count(totals[0])),
        ("Queries", count(totals[1])),
        ("Failed grabs", count(totals[2])),
        ("Failed queries", count(totals[3])),
    ]))
}

fn worker(job: &Value) -> Option<Row> {
    let file = job["file"].as_str()?;
    let title = file.rsplit(['/', '\\']).next().unwrap_or(file).to_string();
    let kind = match job["workerType"].as_str().unwrap_or("") {
        kind if kind.starts_with("transcode") => "transcode",
        kind if kind.starts_with("healthcheck") => "health check",
        _ => "work",
    };
    let detail = match job["ETA"].as_str().filter(|eta| !eta.is_empty()) {
        Some(eta) => format!("{kind} \u{b7} {eta} left"),
        None => kind.to_string(),
    };
    Some(Row {
        title,
        detail,
        progress: job["percentage"].as_f64().map(|percent| percent / 100.0),
        image: None,
        id: None,
    })
}

pub fn tdarr(stats: &Value, nodes: &Value) -> Parsed {
    let table = |index: u8| -> Result<u64, String> {
        let viewable = whole(stats, &format!("table{index}ViewableCount")).unwrap_or(0);
        if viewable > 0 {
            return Ok(viewable);
        }
        whole(stats, &format!("table{index}Count"))
    };
    let mut found = reading(vec![
        ("Queue", count(table(1)? + table(4)?)),
        ("Processed", count(table(2)? + table(5)?)),
        ("Errored", count(table(3)? + table(6)?)),
        ("Saved", bytes(number(stats, "sizeDiff")? * 1e9)),
    ]);
    found.idle = Some("No transcode running".to_string());
    found.rows = nodes
        .as_object()
        .into_iter()
        .flat_map(|nodes| nodes.values())
        .filter_map(|node| node["workers"].as_object())
        .flat_map(|workers| workers.values())
        .filter_map(worker)
        .take(ROWS)
        .collect();
    Ok(found)
}

pub fn suwayomi(counts: &Value) -> Parsed {
    let data = field(counts, "data")?;
    let total = |key: &str| -> Result<String, String> {
        Ok(count(whole(field(data, key)?, "totalCount")?))
    };
    Ok(reading(vec![
        ("Downloaded", total("download")?),
        ("Not downloaded", total("nondownload")?),
        ("Read", total("read")?),
        ("Unread", total("unread")?),
    ]))
}

pub fn qbittorrent(torrents: &Value) -> Parsed {
    const LEECHING: [&str; 8] = [
        "downloading",
        "forcedDL",
        "metaDL",
        "forcedMetaDL",
        "checkingDL",
        "stalledDL",
        "queuedDL",
        "pausedDL",
    ];
    let torrents = items(torrents)?;
    let (mut down, mut up, mut seeds) = (0.0, 0.0, 0);
    let mut leeches = vec![];
    for torrent in torrents {
        down += number(torrent, "dlspeed")?;
        up += number(torrent, "upspeed")?;
        let progress = number(torrent, "progress")?;
        if progress >= 1.0 {
            seeds += 1;
        }
        let state = text(torrent, "state")?;
        if state.contains("DL") || state == "downloading" {
            let rank = LEECHING
                .iter()
                .position(|known| *known == state)
                .unwrap_or(LEECHING.len());
            leeches.push((rank, progress, torrent));
        }
    }
    leeches.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)));
    let mut found = reading(vec![
        ("Leech", count((torrents.len() - seeds) as u64)),
        ("Download", rate(down)),
        ("Seed", count(seeds as u64)),
        ("Upload", rate(up)),
    ]);
    found.metrics = vec![("download", down), ("upload", up)];
    found.idle = Some("Nothing downloading".to_string());
    found.rows = leeches
        .into_iter()
        .take(ROWS)
        .map(|(_, progress, torrent)| -> Result<Row, String> {
            Ok(Row {
                title: text(torrent, "name")?,
                detail: format!(
                    "{} of {}",
                    percent(progress * 100.0),
                    bytes(number(torrent, "size")?)
                ),
                progress: Some(progress),
                image: None,
                id: None,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(found)
}

fn queue(details: &Value, title: impl Fn(&Value) -> String) -> Result<Vec<Row>, String> {
    let mut entries: Vec<(bool, f64, Row)> = items(details)?
        .iter()
        .map(|entry| {
            let status = entry["status"]
                .as_str()
                .or(entry["trackedDownloadState"].as_str())
                .unwrap_or("queued");
            let size = entry["size"].as_f64().unwrap_or(0.0);
            let left = entry["sizeleft"].as_f64().unwrap_or(0.0);
            let remaining = if size > 0.0 { left / size } else { 1.0 };
            let detail = match entry["timeleft"]
                .as_str()
                .and_then(|time| time.split('.').next())
            {
                Some(time) => format!("{status} · {time}"),
                None => status.to_string(),
            };
            let row = Row {
                title: title(entry),
                detail,
                progress: (size > 0.0).then_some(1.0 - remaining),
                image: None,
                id: None,
            };
            (status == "downloading", remaining, row)
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.total_cmp(&b.1)));
    Ok(entries
        .into_iter()
        .take(ROWS)
        .map(|entry| entry.2)
        .collect())
}

fn titled<'a>(list: &'a [Value], id: &Value) -> Option<&'a str> {
    list.iter()
        .find(|item| item["id"] == *id)
        .and_then(|item| item["title"].as_str())
}

fn release(movie: &Value, today: &str) -> Option<(String, Poster)> {
    let (date, kind) = [
        ("inCinemas", "Cinema"),
        ("digitalRelease", "Digital"),
        ("physicalRelease", "Physical"),
    ]
    .into_iter()
    .filter_map(|(key, kind)| Some((movie[key].as_str()?, kind)))
    .filter(|(date, _)| within_week(date, today))
    .min_by_key(|(date, _)| date.get(..10).map(String::from))?;
    let poster = Poster {
        title: movie["title"].as_str()?.to_string(),
        detail: format!("{kind} \u{b7} {}", day(date)?),
        image: poster_of(&movie["images"]),
    };
    Some((date.get(..10)?.to_string(), poster))
}

pub fn radarr(
    movies: &Value,
    status: &Value,
    details: &Value,
    calendar: &Value,
    today: &str,
) -> Parsed {
    let movies = items(movies)?;
    let flag = |movie: &Value, key: &str| movie[key].as_bool() == Some(true);
    let have = movies.iter().filter(|movie| flag(movie, "hasFile")).count();
    let missing: Vec<&Value> = movies
        .iter()
        .filter(|movie| flag(movie, "monitored") && !flag(movie, "hasFile"))
        .collect();
    let wanted = missing
        .iter()
        .filter(|movie| flag(movie, "isAvailable"))
        .count();
    let mut found = reading(vec![
        ("Movies", count(have as u64)),
        ("Wanted", count(wanted as u64)),
        ("Missing", count(missing.len() as u64)),
        ("Queued", count(whole(status, "totalCount")?)),
    ]);
    found.idle = Some("Queue empty".to_string());
    found.rows = queue(details, |entry| {
        titled(movies, &entry["movieId"])
            .or(entry["title"].as_str())
            .unwrap_or("Unknown movie")
            .to_string()
    })?;
    let mut releases: Vec<(String, Poster)> = items(calendar)?
        .iter()
        .filter_map(|movie| release(movie, today))
        .collect();
    releases.sort_by(|a, b| a.0.cmp(&b.0));
    found.shelf = Some(WEEK.to_string());
    found.posters = releases
        .into_iter()
        .map(|(_, poster)| poster)
        .take(POSTERS)
        .collect();
    Ok(found)
}

fn airing(episode: &Value) -> Option<Poster> {
    let series = &episode["series"];
    Some(Poster {
        title: series["title"].as_str()?.to_string(),
        detail: format!(
            "S{:02}E{:02} \u{b7} {}",
            episode["seasonNumber"].as_u64()?,
            episode["episodeNumber"].as_u64()?,
            day(episode["airDate"].as_str()?)?
        ),
        image: poster_of(&series["images"]),
    })
}

pub fn sonarr(
    series: &Value,
    queued: &Value,
    missing: &Value,
    details: &Value,
    calendar: &Value,
) -> Parsed {
    let series = items(series)?;
    let mut found = reading(vec![
        ("Series", count(series.len() as u64)),
        ("Wanted", count(whole(missing, "totalRecords")?)),
        ("Queued", count(whole(queued, "totalRecords")?)),
    ]);
    found.idle = Some("Queue empty".to_string());
    found.rows = queue(details, |entry| {
        let episode = entry["episode"]["title"]
            .as_str()
            .or(entry["title"].as_str())
            .unwrap_or("Unknown episode");
        match titled(series, &entry["seriesId"]) {
            Some(show) => format!("{show}: {episode}"),
            None => episode.to_string(),
        }
    })?;
    let mut airings: Vec<&Value> = items(calendar)?.iter().collect();
    airings.sort_by_key(|episode| {
        episode["airDateUtc"]
            .as_str()
            .or(episode["airDate"].as_str())
    });
    found.shelf = Some(WEEK.to_string());
    found.posters = airings
        .into_iter()
        .filter_map(airing)
        .take(POSTERS)
        .collect();
    Ok(found)
}

pub fn slskd(application: &Value, downloads: &Value, uploads: &Value) -> Parsed {
    let connected = application["server"]["isConnected"].as_bool() == Some(true);
    Ok(reading(vec![
        (
            "Status",
            if connected {
                "Connected"
            } else {
                "Disconnected"
            }
            .to_string(),
        ),
        ("Downloads", count(items(downloads)?.len() as u64)),
        ("Uploads", count(items(uploads)?.len() as u64)),
        (
            "Shared files",
            count(whole(field(application, "shares")?, "files")?),
        ),
    ]))
}

pub fn autobrr(stats: &Value, filters: &Value, indexers: &Value) -> Parsed {
    Ok(reading(vec![
        ("Approved", count(whole(stats, "push_approved_count")?)),
        ("Rejected", count(whole(stats, "push_rejected_count")?)),
        ("Filters", count(items(filters)?.len() as u64)),
        ("Indexers", count(items(indexers)?.len() as u64)),
    ]))
}

pub fn uptime_kuma(status: &Value, heartbeat: &Value) -> Parsed {
    let monitors = field(heartbeat, "heartbeatList")?
        .as_object()
        .ok_or("heartbeatList is not a map")?;
    let up = monitors
        .values()
        .filter(|beats| {
            beats
                .as_array()
                .and_then(|beats| beats.last())
                .is_some_and(|beat| beat["status"].as_u64() == Some(1))
        })
        .count();
    let uptimes: Vec<f64> = field(heartbeat, "uptimeList")?
        .as_object()
        .ok_or("uptimeList is not a map")?
        .values()
        .filter_map(Value::as_f64)
        .collect();
    let average = if uptimes.is_empty() {
        0.0
    } else {
        uptimes.iter().sum::<f64>() / uptimes.len() as f64
    };
    let mut found = reading(vec![
        ("Up", count(up as u64)),
        ("Down", count((monitors.len() - up) as u64)),
        ("Uptime", percent(average * 100.0)),
    ]);
    if let Some(title) = status["incident"]["title"].as_str() {
        found.figures.push(Figure {
            label: "Incident".into(),
            value: title.into(),
        });
    }
    Ok(found)
}

fn system<'a>(systems: &'a Value, name: &str) -> Result<&'a Value, String> {
    items(field(systems, "items")?)?
        .iter()
        .find(|system| system["id"] == name || system["name"] == name)
        .ok_or_else(|| format!("no system {name}"))
}

pub fn beszel(systems: &Value, name: &str) -> Parsed {
    let system = system(systems, name)?;
    let info = field(system, "info")?;
    Ok(reading(vec![
        ("Status", text(system, "status")?),
        ("CPU", percent(number(info, "cpu")?)),
        ("Memory", percent(number(info, "mp")?)),
        ("Network", rate(number(info, "bb")?)),
    ]))
}

pub fn with_stats(mut systems: Value, stats: &Value) -> Value {
    let latest = stats["items"].as_array().cloned().unwrap_or_default();
    if let Some(found) = systems["items"].as_array_mut() {
        for system in found {
            if let Some(record) = latest
                .iter()
                .find(|record| record["system"] == system["id"])
            {
                system["stats"] = record["stats"].clone();
            }
        }
    }
    systems
}

fn network(system: &Value) -> Option<String> {
    let bandwidth = system["stats"]["b"].as_array()?;
    let up = bandwidth.first()?.as_f64()?;
    let down = bandwidth.get(1)?.as_f64()?;
    Some(format!("{} / {}", rate(down), rate(up)))
}

pub fn host(systems: &Value, name: &str) -> Parsed {
    let system = system(systems, name)?;
    let info = field(system, "info")?;
    let mut figures = vec![
        ("CPU".to_string(), percent(number(info, "cpu")?)),
        ("RAM".to_string(), percent(number(info, "mp")?)),
    ];
    if let Ok(temperature) = number(info, "dt") {
        figures.push(("Temperature".into(), format!("{temperature:.0} °C")));
    }
    if let Some(network) = network(system) {
        figures.push(("Network D/U".into(), network));
    }
    figures.push(("Uptime".into(), duration(whole(info, "u")?)));
    for (disk, used) in info["efs"].as_object().into_iter().flatten() {
        if let Some(used) = used.as_f64() {
            figures.push((format!("{disk} disk"), percent(used)));
        }
    }
    Ok(Reading {
        figures: figures
            .into_iter()
            .map(|(label, value)| Figure { label, value })
            .collect(),
        rows: vec![],
        posters: vec![],
        shelf: None,
        idle: None,
        links: vec![],
        health: vec![],
        metrics: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn figures(reading: &Reading) -> Vec<(&str, &str)> {
        reading
            .figures
            .iter()
            .map(|figure| (figure.label.as_str(), figure.value.as_str()))
            .collect()
    }

    const IMAGES: &str = "https://jellyfin.example.org";

    #[test]
    fn jellyfin_counts_the_library_and_lists_what_plays() {
        let counts =
            json!({"MovieCount": 1, "SeriesCount": 2, "EpisodeCount": 16803, "SongCount": 0});
        let sessions = json!([
            {
                "UserName": "alice",
                "NowPlayingItem": {
                    "Id": "e1",
                    "Name": "Who Is Alive?",
                    "SeriesName": "Severance",
                    "SeriesId": "s1",
                    "ParentIndexNumber": 2,
                    "IndexNumber": 3,
                    "RunTimeTicks": 4000
                },
                "PlayState": {"PositionTicks": 1000, "IsPaused": true, "PlayMethod": "Transcode"}
            },
            {"UserName": "bob", "PlayState": {}},
            {
                "UserName": "carol",
                "NowPlayingItem": {"Id": "m1", "Name": "Heat", "RunTimeTicks": 0},
                "PlayState": {"IsPaused": false, "PlayMethod": "DirectPlay"}
            },
            {
                "UserName": "dave",
                "NowPlayingItem": {"Id": "t1", "Name": "Teardrop", "AlbumId": "a1"},
                "PlayState": {"PlayMethod": "DirectStream"}
            }
        ]);
        let quiet = jellyfin(&counts, &json!([]), &json!({"Items": []}), IMAGES).unwrap();
        assert!(quiet.rows.is_empty());
        assert_eq!(Some("Nothing playing".to_string()), quiet.idle);
        let reading = jellyfin(&counts, &sessions, &json!({"Items": []}), IMAGES).unwrap();
        assert_eq!(
            vec![("Movies", "1"), ("Series", "2"), ("Episodes", "16,803")],
            figures(&reading)
        );
        let expected = vec![
            Row {
                title: "Severance S02E03 · Who Is Alive?".into(),
                detail: "alice · transcode · paused".into(),
                progress: Some(0.25),
                image: Some(
                    "https://jellyfin.example.org/Items/s1/Images/Primary?maxHeight=160".into(),
                ),
                id: None,
            },
            Row {
                title: "Heat".into(),
                detail: "carol · direct play".into(),
                progress: None,
                image: Some(
                    "https://jellyfin.example.org/Items/m1/Images/Primary?maxHeight=160".into(),
                ),
                id: None,
            },
            Row {
                title: "Teardrop".into(),
                detail: "dave · direct stream".into(),
                progress: None,
                image: Some(
                    "https://jellyfin.example.org/Items/a1/Images/Primary?maxHeight=160".into(),
                ),
                id: None,
            },
        ];
        assert_eq!(expected, reading.rows);
    }

    #[test]
    fn jellyfin_shows_the_latest_additions_once_per_series() {
        let latest = json!({"Items": [
            {"Id": "e8", "Type": "Episode", "Name": "Episode 8", "SeriesName": "Sword", "SeriesId": "s1", "ParentIndexNumber": 2, "IndexNumber": 8},
            {"Id": "m1", "Type": "Movie", "Name": "Heat", "ProductionYear": 1995},
            {"Id": "e7", "Type": "Episode", "Name": "Episode 7", "SeriesName": "Sword", "SeriesId": "s1", "ParentIndexNumber": 2, "IndexNumber": 7},
            {"Id": "m2", "Type": "Movie", "Name": "Arrival"}
        ]});
        let reading = jellyfin(
            &json!({"MovieCount": 2, "SeriesCount": 1, "EpisodeCount": 2}),
            &json!([]),
            &latest,
            IMAGES,
        )
        .unwrap();
        let poster = |title: &str, detail: &str, id: &str| Poster {
            title: title.into(),
            detail: detail.into(),
            image: format!("https://jellyfin.example.org/Items/{id}/Images/Primary?maxHeight=300"),
        };
        assert_eq!(
            vec![
                poster("Sword", "S02E08", "s1"),
                poster("Heat", "1995", "m1"),
                poster("Arrival", "Movie", "m2"),
            ],
            reading.posters
        );
    }

    #[test]
    fn the_music_bot_shows_its_track_with_the_album_cover() {
        let status = json!({"state": "Playing", "track": "Massive Attack - Teardrop", "queue": 4, "shuffle": "On", "channel": "Lounge", "artwork": "a1"});
        let reading = music_bot(&status, IMAGES).unwrap();
        assert_eq!(
            vec![
                ("State", "Playing"),
                ("Queue", "4"),
                ("Shuffle", "On"),
                ("Channel", "Lounge")
            ],
            figures(&reading)
        );
        let expected = vec![Row {
            title: "Massive Attack - Teardrop".into(),
            detail: "Playing in Lounge".into(),
            progress: None,
            image: Some(
                "https://jellyfin.example.org/Items/a1/Images/Primary?maxHeight=160".into(),
            ),
            id: None,
        }];
        assert_eq!(expected, reading.rows);
        let idle = json!({"state": "Idle", "track": "Nothing", "queue": 0, "shuffle": "Off", "channel": "None", "artwork": null});
        let idle = music_bot(&idle, IMAGES).unwrap();
        assert!(idle.rows.is_empty());
        assert_eq!(Some("Nothing playing".to_string()), idle.idle);
        let between = json!({"state": "Playing", "track": "Nothing", "queue": 0, "shuffle": "Off", "channel": "Lounge", "artwork": null});
        assert!(music_bot(&between, IMAGES).unwrap().rows.is_empty());
        let odd = json!({"state": "Paused", "track": "Air - La femme d'argent", "queue": 0, "shuffle": "Off", "channel": "Lounge", "artwork": "../x?y"});
        assert_eq!(None, music_bot(&odd, IMAGES).unwrap().rows[0].image);
    }

    #[test]
    fn seerr_lists_the_pending_requests_with_their_poster() {
        let pending = json!({"results": [
            {"id": 12, "type": "movie", "requestedBy": {"displayName": "alice"},
             "details": {"title": "Dune", "releaseDate": "2021-09-15", "posterPath": "/dune.jpg"}},
            {"id": 13, "type": "tv", "requestedBy": {"displayName": "bob"}, "seasons": [{"seasonNumber": 1}, {"seasonNumber": 2}],
             "details": {"name": "Frieren", "firstAirDate": "2023-09-29", "posterPath": null}},
            {"id": 14, "type": "movie", "requestedBy": {}, "details": {}}
        ]});
        let reading = seerr(
            &json!({"pending": 3, "processing": 0, "available": 0}),
            &json!({"open": 0, "total": 0}),
            &pending,
        )
        .unwrap();
        let expected = vec![
            Row {
                title: "Dune (2021)".into(),
                detail: "Movie \u{b7} alice".into(),
                progress: None,
                image: Some("https://image.tmdb.org/t/p/w154/dune.jpg".into()),
                id: Some(12),
            },
            Row {
                title: "Frieren (2023)".into(),
                detail: "Series, seasons 1, 2 \u{b7} bob".into(),
                progress: None,
                image: None,
                id: Some(13),
            },
            Row {
                title: "Request 14".into(),
                detail: "Movie \u{b7} someone".into(),
                progress: None,
                image: None,
                id: Some(14),
            },
        ];
        assert_eq!(expected, reading.rows);
    }

    #[test]
    fn uptime_kuma_gives_each_monitor_its_day_and_week() {
        let page = json!({"publicGroupList": [
            {"monitorList": [{"id": 50, "name": "Jellyfin"}, {"id": 46, "name": "Seerr"}]},
            {"monitorList": [{"id": 7, "name": "Backup"}]}
        ]});
        let heartbeat = json!({"uptimeList": {"50_24": 0.9943, "46_24": 1}});
        let week = std::collections::HashMap::from([(50, 98.24)]);
        let found = uptimes(&page, &heartbeat, &week).unwrap();
        assert_eq!(
            vec![
                (
                    "Backup".to_string(),
                    Uptime {
                        day: None,
                        week: None
                    }
                ),
                (
                    "Jellyfin".to_string(),
                    Uptime {
                        day: Some(99.43),
                        week: Some(98.24)
                    }
                ),
                (
                    "Seerr".to_string(),
                    Uptime {
                        day: Some(100.0),
                        week: None
                    }
                ),
            ],
            found.into_iter().collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_badge_tells_its_percent() {
        let svg = r#"<text x="1">Uptime (168h)</text><text x="1165" textLength="430">98.24%</text></g></svg>"#;
        assert_eq!(Some(98.24), badge(svg));
        assert_eq!(None, badge("<svg>N/A</svg>"));
    }

    #[test]
    fn seerr_counts_the_requests_and_the_open_issues() {
        let requests = json!({"total": 20, "pending": 3, "approved": 2, "processing": 1, "available": 12, "completed": 12});
        let issues = json!({"total": 4, "open": 1, "closed": 3});
        let reading = seerr(&requests, &issues, &json!({"results": []})).unwrap();
        assert_eq!(Some("No pending request".to_string()), reading.idle);
        assert_eq!(
            vec![
                ("Pending", "3"),
                ("Processing", "1"),
                ("Available", "12"),
                ("Issues", "1 / 4")
            ],
            figures(&reading)
        );
    }

    #[test]
    fn gluetun_shows_the_exit_of_the_vpn() {
        let ip = json!({"public_ip": "203.0.113.7", "region": "England", "country": "United Kingdom", "city": "Manchester"});
        let reading = gluetun(&ip).unwrap();
        assert_eq!(
            vec![
                ("Public IP", "203.0.113.7"),
                ("Region", "England"),
                ("Country", "United Kingdom")
            ],
            figures(&reading)
        );
    }

    #[test]
    fn bazarr_counts_the_missing_subtitles() {
        let reading = bazarr(
            &json!({"data": [], "total": 0}),
            &json!({"data": [{}], "total": 6}),
        )
        .unwrap();
        assert_eq!(
            vec![("Missing episodes", "6"), ("Missing movies", "0")],
            figures(&reading)
        );
        assert_eq!(Some("No subtitles missing".to_string()), reading.idle);
    }

    #[test]
    fn bazarr_lists_what_misses_subtitles() {
        let episodes = json!({"total": 2, "data": [
            {"seriesTitle": "Reincarnated as a Sword", "episode_number": "2x8", "episodeTitle": "The Island Kingdom", "missing_subtitles": [{"name": "Japanese"}, {"name": "French"}]},
            {"seriesTitle": "Frieren", "episode_number": "1x1", "missing_subtitles": []}
        ]});
        let movies = json!({"total": 1, "data": [{"title": "Heat", "missing_subtitles": [{"name": "English"}]}]});
        let reading = bazarr(&movies, &episodes).unwrap();
        let row = |title: &str, detail: &str| Row {
            title: title.into(),
            detail: detail.into(),
            progress: None,
            image: None,
            id: None,
        };
        assert_eq!(
            vec![
                row("Reincarnated as a Sword 2x8", "Missing Japanese, French"),
                row("Frieren 1x1", "Missing subtitles"),
                row("Heat", "Missing English"),
            ],
            reading.rows
        );
    }

    #[test]
    fn prowlarr_sums_every_indexer() {
        let stats = json!({"indexers": [
            {"indexerName": "C411", "numberOfGrabs": 10, "numberOfQueries": 900, "numberOfFailedGrabs": 1, "numberOfFailedQueries": 30},
            {"indexerName": "Nyaa", "numberOfGrabs": 5, "numberOfQueries": 1200, "numberOfFailedGrabs": 0, "numberOfFailedQueries": 2}
        ]});
        let reading = prowlarr(&stats).unwrap();
        assert_eq!(
            vec![
                ("Grabs", "15"),
                ("Queries", "2,100"),
                ("Failed grabs", "1"),
                ("Failed queries", "32")
            ],
            figures(&reading)
        );
    }

    #[test]
    fn tdarr_lists_its_running_workers() {
        let stats = json!({
            "sizeDiff": 0,
            "table1Count": 0, "table2Count": 0, "table3Count": 0,
            "table4Count": 0, "table5Count": 0, "table6Count": 0
        });
        let nodes = json!({"n1": {"workers": {
            "w1": {"file": "/data/media/tv/Severance/S02E03.mkv", "percentage": 42.5, "workerType": "transcodecpu", "ETA": "0:12:30"},
            "w2": {"file": "/data/media/movies/Heat.mkv", "workerType": "healthcheckcpu"}
        }}});
        let reading = tdarr(&stats, &nodes).unwrap();
        let expected = vec![
            Row {
                title: "S02E03.mkv".into(),
                detail: "transcode · 0:12:30 left".into(),
                progress: Some(0.425),
                image: None,
                id: None,
            },
            Row {
                title: "Heat.mkv".into(),
                detail: "health check".into(),
                progress: None,
                image: None,
                id: None,
            },
        ];
        assert_eq!(expected, reading.rows);
    }

    #[test]
    fn tdarr_prefers_the_viewable_counts() {
        let stats = json!({
            "sizeDiff": 1.5,
            "table1Count": 9, "table1ViewableCount": 4,
            "table2Count": 100, "table2ViewableCount": 0,
            "table3Count": 2, "table3ViewableCount": 2,
            "table4Count": 1, "table4ViewableCount": 1,
            "table5Count": 7, "table5ViewableCount": 7,
            "table6Count": 0, "table6ViewableCount": 0
        });
        let reading = tdarr(&stats, &json!({"n1": {"workers": {}}})).unwrap();
        assert!(reading.rows.is_empty());
        assert_eq!(Some("No transcode running".to_string()), reading.idle);
        assert_eq!(
            vec![
                ("Queue", "5"),
                ("Processed", "107"),
                ("Errored", "2"),
                ("Saved", "1.4 GiB")
            ],
            figures(&reading)
        );
    }

    #[test]
    fn suwayomi_counts_the_chapters() {
        let counts = json!({"data": {
            "download": {"totalCount": 40},
            "nondownload": {"totalCount": 1200},
            "read": {"totalCount": 35},
            "unread": {"totalCount": 1205}
        }});
        let reading = suwayomi(&counts).unwrap();
        assert_eq!(
            vec![
                ("Downloaded", "40"),
                ("Not downloaded", "1,200"),
                ("Read", "35"),
                ("Unread", "1,205")
            ],
            figures(&reading)
        );
    }

    fn rows(reading: &Reading) -> Vec<(&str, &str, Option<f64>)> {
        reading
            .rows
            .iter()
            .map(|row| (row.title.as_str(), row.detail.as_str(), row.progress))
            .collect()
    }

    #[test]
    fn qbittorrent_sums_the_speeds_and_lists_the_leeches() {
        let gib = 1_073_741_824u64;
        let torrents = json!([
            {"name": "Seeding", "state": "uploading", "progress": 1, "dlspeed": 0, "upspeed": 1000, "size": gib},
            {"name": "Stalled", "state": "stalledDL", "progress": 0.5, "dlspeed": 0, "upspeed": 0, "size": 2 * gib},
            {"name": "Active", "state": "downloading", "progress": 0.1, "dlspeed": 2048, "upspeed": 0, "size": gib},
            {"name": "Paused", "state": "stoppedUP", "progress": 1, "dlspeed": 0, "upspeed": 0, "size": gib}
        ]);
        let reading = qbittorrent(&torrents).unwrap();
        assert_eq!(Some("Nothing downloading".to_string()), reading.idle);
        assert_eq!(
            vec![
                ("Leech", "2"),
                ("Download", "2.0 KiB/s"),
                ("Seed", "2"),
                ("Upload", "1000 B/s")
            ],
            figures(&reading)
        );
        assert_eq!(
            vec![
                ("Active", "10.0% of 1.0 GiB", Some(0.1)),
                ("Stalled", "50.0% of 2.0 GiB", Some(0.5)),
            ],
            rows(&reading)
        );
        assert_eq!(
            vec![("download", 2048.0), ("upload", 1000.0)],
            reading.metrics
        );
    }

    const TODAY: &str = "2026-11-22";

    #[test]
    fn radarr_shows_the_releases_of_the_week() {
        let calendar = json!([
            {"title": "Dune: Part Three", "inCinemas": "2026-12-18T00:00:00Z", "digitalRelease": "2026-11-24T00:00:00Z",
             "images": [{"coverType": "fanart", "remoteUrl": "https://image.tmdb.org/f.jpg"}, {"coverType": "poster", "remoteUrl": "https://image.tmdb.org/p.jpg"}]},
            {"title": "Old News", "physicalRelease": "2026-11-10T00:00:00Z", "images": []},
            {"title": "No Poster", "inCinemas": "2026-11-22T00:00:00Z"}
        ]);
        let reading = radarr(
            &json!([]),
            &json!({"totalCount": 0}),
            &json!([]),
            &calendar,
            TODAY,
        )
        .unwrap();
        assert_eq!(
            vec![
                Poster {
                    title: "No Poster".into(),
                    detail: "Cinema · Sun 22 Nov".into(),
                    image: String::new(),
                },
                Poster {
                    title: "Dune: Part Three".into(),
                    detail: "Digital · Tue 24 Nov".into(),
                    image: "https://image.tmdb.org/p.jpg".into(),
                },
            ],
            reading.posters
        );
    }

    #[test]
    fn sonarr_shows_the_episodes_of_the_week() {
        let calendar = json!([
            {"seasonNumber": 3, "episodeNumber": 8, "airDate": "2026-11-27",
             "series": {"title": "The Apothecary Diaries", "images": [{"coverType": "poster", "remoteUrl": "https://artworks.thetvdb.com/p.jpg"}]}}
        ]);
        let reading = sonarr(
            &json!([]),
            &json!({"totalRecords": 0}),
            &json!({"totalRecords": 0}),
            &json!([]),
            &calendar,
        )
        .unwrap();
        let expected = vec![Poster {
            title: "The Apothecary Diaries".into(),
            detail: "S03E08 · Fri 27 Nov".into(),
            image: "https://artworks.thetvdb.com/p.jpg".into(),
        }];
        assert_eq!(expected, reading.posters);
    }

    #[test]
    fn radarr_counts_the_movies_and_lists_the_queue() {
        let movies = json!([
            {"id": 1, "title": "Heat", "monitored": true, "hasFile": true, "isAvailable": true},
            {"id": 2, "title": "Dune", "monitored": true, "hasFile": false, "isAvailable": true},
            {"id": 3, "title": "Alien", "monitored": true, "hasFile": false, "isAvailable": false},
            {"id": 4, "title": "Ran", "monitored": false, "hasFile": false, "isAvailable": true}
        ]);
        let status = json!({"totalCount": 2});
        let details = json!([
            {"movieId": 3, "title": "Alien.1979.1080p", "status": "queued", "trackedDownloadState": "downloading", "size": 100, "sizeleft": 100},
            {"movieId": 2, "title": "Dune.2021.1080p", "status": "downloading", "trackedDownloadState": "downloading", "size": 100, "sizeleft": 25, "timeleft": "00:12:00.3802167"}
        ]);
        let reading = radarr(&movies, &status, &details, &json!([]), TODAY).unwrap();
        assert_eq!(Some("Next 7 days".to_string()), reading.shelf);
        assert!(reading.posters.is_empty());
        assert_eq!(Some("Queue empty".to_string()), reading.idle);
        assert_eq!(
            vec![
                ("Movies", "1"),
                ("Wanted", "1"),
                ("Missing", "2"),
                ("Queued", "2")
            ],
            figures(&reading)
        );
        assert_eq!(
            vec![
                ("Dune", "downloading · 00:12:00", Some(0.75)),
                ("Alien", "queued", Some(0.0)),
            ],
            rows(&reading)
        );
    }

    #[test]
    fn sonarr_counts_the_series_and_names_the_queued_episodes() {
        let series = json!([{"id": 7, "title": "Severance"}, {"id": 8, "title": "Frieren"}]);
        let queue = json!({"totalRecords": 1});
        let missing = json!({"totalRecords": 2});
        let details = json!([
            {"seriesId": 7, "title": "Severance.S02E03", "episode": {"title": "Who Is Alive?"}, "status": "downloading", "size": 0, "sizeleft": 0}
        ]);
        let reading = sonarr(&series, &queue, &missing, &details, &json!([])).unwrap();
        assert_eq!(Some("Next 7 days".to_string()), reading.shelf);
        assert_eq!(Some("Queue empty".to_string()), reading.idle);
        assert_eq!(
            vec![("Series", "2"), ("Wanted", "2"), ("Queued", "1")],
            figures(&reading)
        );
        assert_eq!(
            vec![("Severance: Who Is Alive?", "downloading", None)],
            rows(&reading)
        );
    }

    #[test]
    fn slskd_shows_the_link_and_the_transfers() {
        let application = json!({"server": {"isConnected": true}, "shares": {"files": 1413}});
        let reading = slskd(&application, &json!([{}, {}]), &json!([{}])).unwrap();
        assert_eq!(
            vec![
                ("Status", "Connected"),
                ("Downloads", "2"),
                ("Uploads", "1"),
                ("Shared files", "1,413")
            ],
            figures(&reading)
        );
        let offline = slskd(
            &json!({"server": {"isConnected": false}, "shares": {"files": 0}}),
            &json!([]),
            &json!([]),
        )
        .unwrap();
        assert_eq!(("Status", "Disconnected"), figures(&offline)[0]);
    }

    #[test]
    fn autobrr_counts_the_pushes_filters_and_indexers() {
        let stats = json!({"push_approved_count": 1200, "push_rejected_count": 30});
        let reading = autobrr(&stats, &json!([{}, {}, {}]), &json!([{}])).unwrap();
        assert_eq!(
            vec![
                ("Approved", "1,200"),
                ("Rejected", "30"),
                ("Filters", "3"),
                ("Indexers", "1")
            ],
            figures(&reading)
        );
    }

    #[test]
    fn uptime_kuma_reads_the_last_beat_of_every_monitor() {
        let status = json!({"incident": null});
        let heartbeat = json!({
            "heartbeatList": {
                "1": [{"status": 0}, {"status": 1}],
                "2": [{"status": 1}, {"status": 0}],
                "3": [{"status": 1}]
            },
            "uptimeList": {"1_24": 1, "2_24": 0.9, "3_24": 0.95}
        });
        let reading = uptime_kuma(&status, &heartbeat).unwrap();
        assert_eq!(
            vec![("Up", "2"), ("Down", "1"), ("Uptime", "95.0%")],
            figures(&reading)
        );
        let incident = json!({"incident": {"title": "Disk replaced"}});
        let reading = uptime_kuma(&incident, &heartbeat).unwrap();
        assert_eq!(("Incident", "Disk replaced"), figures(&reading)[3]);
    }

    fn systems() -> Value {
        json!({"items": [
            {"id": "other", "name": "Laptop", "status": "down", "info": {}},
            {"id": "6oy4", "name": "NAS", "status": "up", "info": {
                "u": 52994, "cpu": 6.4, "mp": 76.56, "dp": 18.56, "bb": 42060,
                "efs": {"Data": 71.25}
            }}
        ]})
    }

    #[test]
    fn beszel_shows_the_system_of_the_nas() {
        let reading = beszel(&systems(), "NAS").unwrap();
        assert_eq!(
            vec![
                ("Status", "up"),
                ("CPU", "6.4%"),
                ("Memory", "76.6%"),
                ("Network", "41.1 KiB/s")
            ],
            figures(&reading)
        );
        assert_eq!(
            Err("no system NAS".to_string()),
            beszel(&json!({"items": []}), "NAS")
        );
    }

    #[test]
    fn the_host_shows_its_resources_and_disks() {
        let stats = json!({"items": [
            {"system": "6oy4", "stats": {"b": [938_761, 83_557]}},
            {"system": "6oy4", "stats": {"b": [1, 1]}}
        ]});
        let reading = host(&with_stats(systems(), &stats), "NAS").unwrap();
        assert_eq!(
            vec![
                ("CPU", "6.4%"),
                ("RAM", "76.6%"),
                ("Network D/U", "81.6 KiB/s / 916.8 KiB/s"),
                ("Uptime", "14h 43m"),
                ("Data disk", "71.2%")
            ],
            figures(&reading)
        );
        let mut warm = systems();
        warm["items"][1]["info"]["dt"] = json!(47.6);
        let reading = host(&warm, "6oy4").unwrap();
        assert_eq!(("Temperature", "48 °C"), figures(&reading)[2]);
    }

    #[test]
    fn a_missing_field_is_an_error() {
        assert_eq!(
            Err("missing MovieCount".to_string()),
            jellyfin(&json!({}), &json!([]), &json!({"Items": []}), IMAGES)
        );
    }

    const NOW: u64 = 1_795_350_000;

    fn deployed(sha: &str, at: &str) -> Value {
        json!({"Labels": {
            "cd.doco.deployment.target.sha": sha,
            "cd.doco.deployment.timestamp": at
        }})
    }

    #[test]
    fn the_last_deploy_is_the_newest_label_and_succeeds_without_errors() {
        let containers = json!([
            deployed("aaaaaaaaaaaa", "2026-11-20T10:00:00Z"),
            deployed("df8b53c628732a6a", "2026-11-22T08:26:17.955548255Z"),
            {"Labels": {"com.docker.compose.service": "manual"}}
        ]);
        let logs = r#"{"time":"2026-11-22T08:27:37.12Z","level":"info","msg":"job completed successfully"}
not json
"#;
        let reading = deploy(&containers, logs, NOW).unwrap();
        assert_eq!(
            vec![
                ("Commit", "df8b53c"),
                ("Deployed", "3h 53m ago"),
                ("Result", "Success")
            ],
            figures(&reading)
        );
        assert!(reading.rows.is_empty());
        assert_eq!(Some("No error since the deploy".to_string()), reading.idle);
        assert_eq!(
            Err("no deploy found".to_string()),
            deploy(&json!([]), "", NOW)
        );
    }

    #[test]
    fn a_deploy_error_after_the_last_deploy_is_a_failure() {
        let containers = json!([deployed("df8b53c628732a6a", "2026-11-22T08:26:17Z")]);
        let logs = r#"{"time":"2026-11-22T08:10:00Z","level":"error","msg":"too old","error":"ignored"}
{"time":"2026-11-22T08:40:00Z","level":"ERROR","msg":"failed to fetch","error":"github unreachable"}
{"time":"2026-11-22T08:50:00Z","level":"info","msg":"job completed successfully"}
{"time":"2026-11-22T09:00:00Z","level":"ERROR","msg":"deployment failed","error":"service radarr: image not found"}
"#;
        let reading = deploy(&containers, logs, NOW).unwrap();
        assert_eq!(("Result", "Failed"), figures(&reading)[2]);
        let expected = vec![Row {
            title: "deployment failed".into(),
            detail: "service radarr: image not found".into(),
            progress: None,
            image: None,
            id: None,
        }];
        assert_eq!(expected, reading.rows);
        let recovered = format!(
            "{logs}{}\n",
            r#"{"time":"2026-11-22T09:10:00Z","level":"info","msg":"job completed successfully"}"#
        );
        let reading = deploy(&containers, &recovered, NOW).unwrap();
        assert_eq!(("Result", "Success"), figures(&reading)[2]);
    }

    #[test]
    fn the_backup_reports_its_last_run() {
        let logs = r#"time=2026-11-22T00:23:02.005+01:00 level=INFO msg="Successfully scheduled backup from environment with expression 30 3 * * *"
time=2026-11-22T03:30:00.120+01:00 level=INFO msg="Now running script on schedule 30 3 * * *"
time=2026-11-22T03:31:41.565+01:00 level=INFO msg="Stored copy of backup `/tmp/appdata-2026-11-22T03-30-00.tar.gz` in `/archive`." storage=Local
time=2026-11-22T03:31:41.609+01:00 level=INFO msg="None of 6 existing backups were pruned." storage=Local
"#;
        let reading = backup(logs, &[ARCHIVE], NOW).unwrap();
        assert_eq!(
            vec![
                ("Last backup", "9h 48m ago"),
                ("Took", "1m"),
                ("Result", "Success"),
                ("Archives", "1 · 474.0 MiB")
            ],
            figures(&reading)
        );
        assert_eq!(Some("No error in the last run".to_string()), reading.idle);
    }

    #[test]
    fn a_backup_error_after_the_last_start_is_a_failure() {
        let logs = r#"time=2026-11-21T03:30:00.000+01:00 level=INFO msg="Now running script on schedule 30 3 * * *"
time=2026-11-21T03:31:00.000+01:00 level=INFO msg="Stored copy of backup `/tmp/a.tar.gz` in `/archive`." storage=Local
time=2026-11-22T03:30:00.000+01:00 level=INFO msg="Now running script on schedule 30 3 * * *"
time=2026-11-22T03:30:05.000+01:00 level=ERROR msg="Fatal error running command: archive failed" error="no space left on device"
"#;
        let reading = backup(logs, &[], NOW).unwrap();
        assert_eq!(
            vec![
                ("Last backup", "1d 9h ago"),
                ("Took", "1m"),
                ("Result", "Failed"),
                ("Archives", "0 · 0 B")
            ],
            figures(&reading)
        );
        let expected = vec![Row {
            title: "Fatal error running command: archive failed".into(),
            detail: "no space left on device".into(),
            progress: None,
            image: None,
            id: None,
        }];
        assert_eq!(expected, reading.rows);
        let never = backup(
            "time=2026-11-22T00:23:02.005+01:00 level=INFO msg=\"scheduled\"\n",
            &[],
            NOW,
        )
        .unwrap();
        assert_eq!(
            vec![
                ("Last backup", "—"),
                ("Took", "—"),
                ("Result", "Unknown"),
                ("Archives", "0 · 0 B")
            ],
            figures(&never)
        );
        let cut = "time=2026-11-22T03:30:00.000+01:00 level=INFO msg=\"Now running script\"\n";
        assert_eq!(
            ("Result", "Interrupted"),
            figures(&backup(cut, &[], NOW).unwrap())[2]
        );
        assert_eq!(
            ("Result", "Running"),
            figures(&backup(cut, &[], 1_795_314_600).unwrap())[2]
        );
    }

    const ARCHIVE: Archive = Archive {
        at: 1_795_314_701,
        size: 497_025_024,
    };

    #[test]
    fn the_archives_tell_the_last_backup_once_the_logs_are_gone() {
        let restarted =
            "time=2026-11-22T10:20:30.342+01:00 level=INFO msg=\"Successfully scheduled backup\"\n";
        let older = Archive {
            at: ARCHIVE.at - 86_400,
            size: 1_000,
        };
        assert_eq!(
            vec![
                ("Last backup", "9h 48m ago"),
                ("Took", "—"),
                ("Result", "Success"),
                ("Archives", "2 · 474.0 MiB")
            ],
            figures(&backup(restarted, &[older, ARCHIVE], NOW).unwrap())
        );
        let two_days = NOW + 2 * 86_400;
        let missed = backup(restarted, &[ARCHIVE], two_days).unwrap();
        assert_eq!(("Result", "Missed"), figures(&missed)[2]);
        assert_eq!(
            vec![failure(
                "The nightly backup left no archive",
                "Last archive 2d 9h ago"
            )],
            missed.rows
        );
    }

    #[test]
    fn aurral_shows_whether_its_downloads_run_and_offers_to_switch_them() {
        let active = json!({"albums": [], "artists": [], "tracks": [{"trackName": "Echo"}, {"trackName": "Girls"}]});
        let found = aurral(&json!({"concurrency": 3, "paused": true}), &active).unwrap();
        assert_eq!(
            vec![("Downloads", "Paused"), ("Downloading", "2")],
            figures(&found)
        );
        assert_eq!(
            vec![("worker", "Downloads", "off", None)],
            links(&found.links)
        );
        let found = aurral(&json!({"concurrency": 3}), &json!({"tracks": []})).unwrap();
        assert_eq!(
            vec![("Downloads", "On"), ("Downloading", "0")],
            figures(&found)
        );
        assert_eq!(
            vec![("worker", "Downloads", "on", None)],
            links(&found.links)
        );
        assert!(aurral(&json!({}), &json!({})).is_err());
    }

    #[test]
    fn switching_a_connection_flips_only_its_enable_flags() {
        let prowlarr = json!({"id": 3, "name": "C411", "enable": true, "appProfileId": 1});
        assert_eq!(
            json!({"id": 3, "name": "C411", "enable": false, "appProfileId": 1}),
            switched("prowlarr", "indexer", &prowlarr, false)
        );
        let client = json!({"id": 1, "name": "qBittorrent", "enable": false, "priority": 1});
        assert_eq!(
            json!({"id": 1, "name": "qBittorrent", "enable": true, "priority": 1}),
            switched("radarr", "client", &client, true)
        );
        let starr = json!({"id": 2, "enableRss": false, "enableAutomaticSearch": false, "enableInteractiveSearch": false, "supportsRss": false, "supportsSearch": true});
        assert_eq!(
            json!({"id": 2, "enableRss": false, "enableAutomaticSearch": true, "enableInteractiveSearch": true, "supportsRss": false, "supportsSearch": true}),
            switched("sonarr", "indexer", &starr, true)
        );
        let on = switched("sonarr", "indexer", &starr, true);
        assert_eq!(
            json!({"id": 2, "enableRss": false, "enableAutomaticSearch": false, "enableInteractiveSearch": false, "supportsRss": false, "supportsSearch": true}),
            switched("sonarr", "indexer", &on, false)
        );
        let proxy = json!({"id": 1, "name": "Gluetun"});
        assert_eq!(proxy, switched("prowlarr", "proxy", &proxy, false));
    }

    fn links(found: &[Link]) -> Vec<(&str, &str, &str, Option<u64>)> {
        found
            .iter()
            .map(|link| {
                (
                    link.kind.as_str(),
                    link.name.as_str(),
                    link.state.as_str(),
                    link.until,
                )
            })
            .collect()
    }

    const NOON: u64 = 1_795_348_800;

    #[test]
    fn a_prowlarr_indexer_is_on_off_failing_or_paused_by_ratio() {
        let indexers = json!([
            {"id": 3, "name": "C411", "enable": true, "appProfileId": 2, "protocol": "torrent"},
            {"id": 1, "name": "Nyaa.si", "enable": true, "appProfileId": 1, "protocol": "torrent"},
            {"id": 5, "name": "Old", "enable": false, "appProfileId": 1, "protocol": "usenet"},
            {"id": 6, "name": "Flaky", "enable": true, "appProfileId": 1, "protocol": "torrent"}
        ]);
        let statuses = json!([
            {"indexerId": 6, "disabledTill": "2026-11-22T12:30:00Z"},
            {"indexerId": 1, "disabledTill": "2026-11-22T10:00:00Z"}
        ]);
        let profiles = json!([{"id": 1, "name": "Standard"}, {"id": 2, "name": "Paused"}]);
        let (found, health) = prowlarr_links(
            &indexers,
            &statuses,
            &profiles,
            &json!([]),
            &json!([]),
            NOON,
        )
        .unwrap();
        assert_eq!(
            vec![
                ("indexer", "C411", "paused", None),
                ("indexer", "Flaky", "failing", Some(NOON + 1800)),
                ("indexer", "Nyaa.si", "on", None),
                ("indexer", "Old", "off", None),
            ],
            links(&found)
        );
        assert_eq!(Some("torrent".to_string()), found[0].hint);
        assert_eq!(3, found[0].id);
        assert!(health.is_empty());
    }

    #[test]
    fn a_prowlarr_proxy_fails_when_its_health_check_names_it_or_all_of_them() {
        let proxies = json!([{"id": 1, "name": "FlareSolverr"}, {"id": 2, "name": "Gluetun"}]);
        let all = json!([
            {"source": "IndexerProxyStatusCheck", "type": "error", "message": "All indexer proxies are unavailable due to failures"},
            {"source": "UpdateCheck", "type": "notice", "message": "A new version is out"}
        ]);
        let (found, health) =
            prowlarr_links(&json!([]), &json!([]), &json!([]), &proxies, &all, NOON).unwrap();
        assert_eq!(
            vec![
                ("proxy", "FlareSolverr", "failing", None),
                ("proxy", "Gluetun", "failing", None)
            ],
            links(&found)
        );
        assert_eq!(
            vec![Health {
                level: "error".into(),
                message: "All indexer proxies are unavailable due to failures".into()
            }],
            health
        );
        let one = json!([{"source": "IndexerProxyStatusCheck", "type": "warning", "message": "Proxys indisponibles suite à des échecs : Gluetun"}]);
        let (found, _) =
            prowlarr_links(&json!([]), &json!([]), &json!([]), &proxies, &one, NOON).unwrap();
        assert_eq!(
            vec![
                ("proxy", "FlareSolverr", "on", None),
                ("proxy", "Gluetun", "failing", None)
            ],
            links(&found)
        );
        assert!(prowlarr_links(&json!({}), &json!([]), &json!([]), &proxies, &one, NOON).is_err());
    }

    #[test]
    fn radarr_and_sonarr_list_their_indexers_then_their_download_clients() {
        let indexers = json!([
            {"id": 3, "name": "C411 (Prowlarr)", "enableRss": true, "enableAutomaticSearch": true, "enableInteractiveSearch": true},
            {"id": 1, "name": "Nyaa.si (Prowlarr)", "enableRss": false, "enableAutomaticSearch": false, "enableInteractiveSearch": false},
            {"id": 4, "name": "Slow (Prowlarr)", "enableRss": true, "enableAutomaticSearch": true, "enableInteractiveSearch": true}
        ]);
        let clients = json!([{"id": 1, "name": "qBittorrent", "enable": false}, {"id": 2, "name": "Aria", "enable": true}]);
        let health = json!([
            {"source": "DownloadClientCheck", "type": "warning", "message": "No download client is available"},
            {"source": "IndexerStatusCheck", "type": "warning", "message": "Indexers unavailable due to failures: Slow (Prowlarr)"}
        ]);
        let (found, said) = starr_links(&indexers, &clients, &health).unwrap();
        assert_eq!(
            vec![
                ("indexer", "C411", "on", None),
                ("indexer", "Nyaa.si", "off", None),
                ("indexer", "Slow", "failing", None),
                ("client", "Aria", "on", None),
                ("client", "qBittorrent", "off", None),
            ],
            links(&found)
        );
        assert_eq!(Some("from Prowlarr".to_string()), found[0].hint);
        assert_eq!(None, found[3].hint);
        let down = json!([{"source": "DownloadClientStatusCheck", "type": "warning", "message": "Clients indisponibles : Aria"}]);
        let (found, _) = starr_links(&indexers, &clients, &down).unwrap();
        assert_eq!(("client", "Aria", "failing", None), links(&found)[3]);
        let all = json!([{"source": "DownloadClientStatusCheck", "type": "error", "message": "Tous les clients sont indisponibles"}]);
        let (found, _) = starr_links(&indexers, &clients, &all).unwrap();
        assert_eq!(
            vec![
                ("client", "Aria", "failing", None),
                ("client", "qBittorrent", "off", None)
            ],
            links(&found)[3..].to_vec()
        );
        assert_eq!(2, said.len());
        let long = json!([{"source": "IndexerLongTermStatusCheck", "type": "error", "message": "Tous les indexeurs sont indisponibles depuis plus de 6 heures"}]);
        let (found, _) = starr_links(&indexers, &clients, &long).unwrap();
        assert_eq!(
            vec![
                ("indexer", "C411", "failing", None),
                ("indexer", "Nyaa.si", "off", None),
                ("indexer", "Slow", "failing", None)
            ],
            links(&found)[..3].to_vec()
        );
        assert!(starr_links(&indexers, &json!(null), &health).is_err());
    }

    #[test]
    fn a_warning_about_what_every_switch_turned_off_is_not_trouble() {
        let indexers = json!([
            {"id": 3, "name": "C411 (Prowlarr)", "enableRss": false, "enableAutomaticSearch": false, "enableInteractiveSearch": false}
        ]);
        let clients = json!([{"id": 1, "name": "qBittorrent", "enable": false}]);
        let health = json!([
            {"source": "DownloadClientCheck", "type": "warning", "message": "No download client is available"},
            {"source": "IndexerRssCheck", "type": "warning", "message": "No indexers available with RSS sync enabled"},
            {"source": "IndexerSearchCheck", "type": "warning", "message": "No indexers available with Automatic Search enabled"},
            {"source": "RecyclingBinCheck", "type": "error", "message": "Unable to write to the recycling bin"}
        ]);
        let (_, said) = starr_links(&indexers, &clients, &health).unwrap();
        assert_eq!(
            vec!["Unable to write to the recycling bin"],
            said.iter()
                .map(|item| item.message.as_str())
                .collect::<Vec<_>>()
        );
        let on = json!([{"id": 1, "name": "qBittorrent", "enable": true}]);
        let (_, said) = starr_links(&indexers, &on, &health).unwrap();
        assert!(
            said.iter()
                .any(|item| item.message == "No download client is available")
        );
    }
}
