use super::format::{bytes, count, duration, percent, rate};
use super::{Figure, Poster, Reading, Row};
use serde_json::Value;

pub type Parsed = Result<Reading, String>;

const ROWS: usize = 8;
const POSTERS: usize = 8;

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
    let track = text(status, "track")?;
    if state != "Idle" && track != "Nothing" {
        found.rows = vec![Row {
            title: track,
            detail: format!("{state} in {channel}"),
            progress: None,
            image: status["artwork"]
                .as_str()
                .and_then(|id| artwork(images, id, 160)),
        }];
    }
    Ok(found)
}

pub fn seerr(requests: &Value, issues: &Value) -> Parsed {
    Ok(reading(vec![
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
    ]))
}

pub fn gluetun(ip: &Value) -> Parsed {
    Ok(reading(vec![
        ("Public IP", text(ip, "public_ip")?),
        ("Region", text(ip, "region")?),
        ("Country", text(ip, "country")?),
    ]))
}

pub fn bazarr(movies: &Value, episodes: &Value) -> Parsed {
    Ok(reading(vec![
        ("Missing episodes", count(whole(episodes, "total")?)),
        ("Missing movies", count(whole(movies, "total")?)),
    ]))
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

pub fn tdarr(stats: &Value) -> Parsed {
    let table = |index: u8| -> Result<u64, String> {
        let viewable = whole(stats, &format!("table{index}ViewableCount")).unwrap_or(0);
        if viewable > 0 {
            return Ok(viewable);
        }
        whole(stats, &format!("table{index}Count"))
    };
    Ok(reading(vec![
        ("Queue", count(table(1)? + table(4)?)),
        ("Processed", count(table(2)? + table(5)?)),
        ("Errored", count(table(3)? + table(6)?)),
        ("Saved", bytes(number(stats, "sizeDiff")? * 1e9)),
    ]))
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

pub fn radarr(movies: &Value, status: &Value, details: &Value) -> Parsed {
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
    found.rows = queue(details, |entry| {
        titled(movies, &entry["movieId"])
            .or(entry["title"].as_str())
            .unwrap_or("Unknown movie")
            .to_string()
    })?;
    Ok(found)
}

pub fn sonarr(series: &Value, queued: &Value, missing: &Value, details: &Value) -> Parsed {
    let series = items(series)?;
    let mut found = reading(vec![
        ("Series", count(series.len() as u64)),
        ("Wanted", count(whole(missing, "totalRecords")?)),
        ("Queued", count(whole(queued, "totalRecords")?)),
    ]);
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

pub fn host(systems: &Value, name: &str) -> Parsed {
    let info = field(system(systems, name)?, "info")?;
    let mut figures = vec![
        ("CPU".to_string(), percent(number(info, "cpu")?)),
        ("RAM".to_string(), percent(number(info, "mp")?)),
    ];
    if let Ok(temperature) = number(info, "dt") {
        figures.push(("Temperature".into(), format!("{temperature:.0} °C")));
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
            },
            Row {
                title: "Heat".into(),
                detail: "carol · direct play".into(),
                progress: None,
                image: Some(
                    "https://jellyfin.example.org/Items/m1/Images/Primary?maxHeight=160".into(),
                ),
            },
            Row {
                title: "Teardrop".into(),
                detail: "dave · direct stream".into(),
                progress: None,
                image: Some(
                    "https://jellyfin.example.org/Items/a1/Images/Primary?maxHeight=160".into(),
                ),
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
        }];
        assert_eq!(expected, reading.rows);
        let idle = json!({"state": "Idle", "track": "Nothing", "queue": 0, "shuffle": "Off", "channel": "None", "artwork": null});
        assert!(music_bot(&idle, IMAGES).unwrap().rows.is_empty());
        let between = json!({"state": "Playing", "track": "Nothing", "queue": 0, "shuffle": "Off", "channel": "Lounge", "artwork": null});
        assert!(music_bot(&between, IMAGES).unwrap().rows.is_empty());
        let odd = json!({"state": "Paused", "track": "Air - La femme d'argent", "queue": 0, "shuffle": "Off", "channel": "Lounge", "artwork": "../x?y"});
        assert_eq!(None, music_bot(&odd, IMAGES).unwrap().rows[0].image);
    }

    #[test]
    fn seerr_counts_the_requests_and_the_open_issues() {
        let requests = json!({"total": 20, "pending": 3, "approved": 2, "processing": 1, "available": 12, "completed": 12});
        let issues = json!({"total": 4, "open": 1, "closed": 3});
        let reading = seerr(&requests, &issues).unwrap();
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
        let reading = tdarr(&stats).unwrap();
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
        let reading = radarr(&movies, &status, &details).unwrap();
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
        let reading = sonarr(&series, &queue, &missing, &details).unwrap();
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
        let reading = host(&systems(), "NAS").unwrap();
        assert_eq!(
            vec![
                ("CPU", "6.4%"),
                ("RAM", "76.6%"),
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
}
