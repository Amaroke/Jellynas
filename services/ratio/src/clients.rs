use crate::http::{self, Reply};
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::Cell;
use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

pub const C411_URL: &str = "https://c411.org";
const TORZNAB: &str = "http://torznab.com/schemas/2015/feed";

#[derive(Debug, PartialEq)]
pub enum Fail {
    Throttled { wait: f64, disabled: bool },
    OutOfCalls,
    Refused(String),
    Other(String),
}

impl fmt::Display for Fail {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Fail::Throttled { wait, .. } => {
                write!(formatter, "Prowlarr asks to wait {wait} seconds")
            }
            Fail::OutOfCalls => write!(formatter, "no call to C411 left in this run"),
            Fail::Refused(why) | Fail::Other(why) => write!(formatter, "{why}"),
        }
    }
}

impl From<String> for Fail {
    fn from(why: String) -> Self {
        Fail::Other(why)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Torrent {
    pub hash: String,
    pub name: String,
    pub category: String,
    pub progress: f64,
    pub state: String,
    pub size: i64,
    pub downloaded: i64,
    pub uploaded: i64,
    pub save_path: String,
    pub tracker: String,
    pub private: Option<bool>,
    pub seeding_time: i64,
    pub added_on: i64,
    pub tags: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TorrentFile {
    pub name: String,
    pub size: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Release {
    pub infohash: String,
    pub title: String,
    pub page: String,
    pub link: String,
    pub size: i64,
    pub seeders: i64,
    pub leechers: i64,
    pub age_hours: f64,
    pub free: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Account {
    pub uploaded: i64,
    pub downloaded: i64,
    pub actual: Option<i64>,
    pub credit: Option<i64>,
    pub zen: f64,
    pub earned: Option<f64>,
    pub class: Option<String>,
    pub grade: Value,
    pub allowed: bool,
}

pub trait Qbt {
    fn torrents(&self) -> Result<Vec<Torrent>, String>;
    fn add(&self, torrent: &[u8], category: &str, save_path: Option<&str>) -> Result<(), String>;
    fn delete(&self, hashes: &[String], delete_files: bool) -> Result<(), String>;
    fn files(&self, hash: &str) -> Result<Vec<TorrentFile>, String>;
    fn rename_file(&self, hash: &str, old: &str, new: &str) -> Result<(), String>;
    fn recheck(&self, hash: &str) -> Result<(), String>;
    fn start(&self, hashes: &[String]) -> Result<(), String>;
    fn set_category(&self, hashes: &[String], category: &str) -> Result<(), String>;
    fn categories(&self) -> Result<Vec<String>, String>;
    fn create_category(&self, name: &str, path: &str) -> Result<(), String>;
    fn add_tags(&self, hashes: &[String], tag: &str) -> Result<(), String>;
    fn remove_tags(&self, hashes: &[String], tag: &str) -> Result<(), String>;
    fn tags(&self) -> Result<Vec<String>, String>;
    fn delete_tags(&self, names: &[String]) -> Result<(), String>;
    fn trackers(&self, hash: &str) -> Result<Vec<String>, String>;
    fn keep_in_place(&self, hashes: &[String]) -> Result<(), String>;
    fn reset_share_limits(&self, hashes: &[String]) -> Result<(), String>;
    fn preferences(&self) -> Result<Value, String>;
    fn set_preferences(&self, preferences: &Value) -> Result<(), String>;
    fn pause(&self, _seconds: u64) {}
}

pub trait Indexers {
    fn search(&self, indexer: &str, query: &str, now: f64) -> Result<Vec<Release>, Fail>;
    fn torrent(&self, release: &Release) -> Result<Vec<u8>, Fail>;
    fn hold(
        &self,
        indexer: &str,
        paused: bool,
        profile: &str,
        restore: Option<i64>,
    ) -> Result<Option<i64>, Fail>;
}

pub trait Tracker {
    fn allow(&self, calls: u32);
    fn account(&self) -> Result<Account, Fail>;
    fn catalog(&self, now: f64) -> Result<Vec<Release>, Fail>;
    fn download(&self, infohash: &str) -> Result<Vec<u8>, Fail>;
}

pub fn match_files(qbt: &dyn Qbt, hash: &str, wanted: &[TorrentFile]) -> Result<(), String> {
    let mut files = vec![];
    for _ in 0..10 {
        files = qbt.files(hash)?;
        if !files.is_empty() {
            break;
        }
        qbt.pause(1);
    }
    if files.is_empty() {
        return Err("qBittorrent has not read the torrent file yet".into());
    }
    let mut names: Vec<(i64, Vec<String>)> = vec![];
    for file in wanted {
        match names.iter_mut().find(|(size, _)| *size == file.size) {
            Some((_, list)) => list.push(file.name.clone()),
            None => names.push((file.size, vec![file.name.clone()])),
        }
    }
    for file in files {
        let name = names
            .iter_mut()
            .find(|(size, list)| *size == file.size && !list.is_empty())
            .map(|(_, list)| list.remove(0))
            .ok_or_else(|| format!("no file of {} bytes in the public copy", file.size))?;
        if file.name != name {
            qbt.rename_file(hash, &file.name, &name)?;
        }
    }
    qbt.recheck(hash)
}

pub struct QBittorrent {
    url: String,
    username: String,
    password: String,
    agent: ureq::Agent,
    cookie: std::cell::RefCell<Option<String>>,
}

impl QBittorrent {
    pub fn new(url: &str, username: &str, password: &str) -> Result<Self, String> {
        Ok(QBittorrent {
            url: url.trim_end_matches('/').into(),
            username: username.into(),
            password: password.into(),
            agent: http::agent(None, 30)?,
            cookie: std::cell::RefCell::new(None),
        })
    }

    fn login(&self) -> Result<(), String> {
        let body = http::form(&[
            ("username", self.username.clone()),
            ("password", self.password.clone()),
        ]);
        let reply = http::call(
            &self.agent,
            "POST",
            &format!("{}/api/v2/auth/login", self.url),
            &[],
            Some(("application/x-www-form-urlencoded", body)),
        )?;
        let accepted = reply.status == 204 || (reply.ok() && reply.text().trim() == "Ok.");
        match (accepted, reply.cookie) {
            (true, Some(cookie)) => {
                *self.cookie.borrow_mut() = Some(cookie);
                Ok(())
            }
            (true, None) => Ok(()),
            _ => {
                *self.cookie.borrow_mut() = None;
                Err("qBittorrent refused the login, check QBITTORRENT_USER and QBITTORRENT_PASSWORD".into())
            }
        }
    }

    fn open(&self, path: &str, body: Option<(&str, Vec<u8>)>) -> Result<Reply, String> {
        let headers: Vec<(&str, String)> = self
            .cookie
            .borrow()
            .iter()
            .map(|cookie| ("cookie", cookie.clone()))
            .collect();
        let method = if body.is_some() { "POST" } else { "GET" };
        http::call(
            &self.agent,
            method,
            &format!("{}{path}", self.url),
            &headers,
            body,
        )
    }

    fn send(&self, path: &str, body: Option<(&str, Vec<u8>)>) -> Result<Vec<u8>, String> {
        if self.cookie.borrow().is_none() {
            self.login()?;
        }
        let mut reply = self.open(path, body.clone())?;
        if reply.status == 403 {
            self.login()?;
            reply = self.open(path, body)?;
        }
        if !reply.ok() {
            return Err(format!("qBittorrent answered {} to {path}", reply.status));
        }
        Ok(reply.body)
    }

    fn post(&self, path: &str, fields: &[(&str, String)]) -> Result<Vec<u8>, String> {
        self.send(
            path,
            Some(("application/x-www-form-urlencoded", http::form(fields))),
        )
    }

    fn json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        serde_json::from_slice(&self.send(path, None)?).map_err(|error| error.to_string())
    }
}

fn joined(hashes: &[String]) -> String {
    hashes.join("|")
}

impl Qbt for QBittorrent {
    fn torrents(&self) -> Result<Vec<Torrent>, String> {
        self.json("/api/v2/torrents/info")
    }

    fn add(&self, torrent: &[u8], category: &str, save_path: Option<&str>) -> Result<(), String> {
        let boundary = format!(
            "ratio{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let mut fields = vec![("category", category.to_string())];
        if let Some(path) = save_path {
            fields.push(("savepath", path.into()));
            fields.push(("autoTMM", "false".into()));
            fields.push(("stopped", "true".into()));
        }
        let mut body = vec![];
        for (name, value) in fields {
            body.extend(
                format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                    .into_bytes(),
            );
        }
        body.extend(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"torrents\"; filename=\"grab.torrent\"\r\nContent-Type: application/x-bittorrent\r\n\r\n"
            )
            .into_bytes(),
        );
        body.extend_from_slice(torrent);
        body.extend(format!("\r\n--{boundary}--\r\n").into_bytes());
        let kind = format!("multipart/form-data; boundary={boundary}");
        let answer = self.send("/api/v2/torrents/add", Some((&kind, body)))?;
        if String::from_utf8_lossy(&answer).trim() == "Fails." {
            return Err("qBittorrent refused the torrent file".into());
        }
        Ok(())
    }

    fn delete(&self, hashes: &[String], delete_files: bool) -> Result<(), String> {
        self.post(
            "/api/v2/torrents/delete",
            &[
                ("hashes", joined(hashes)),
                ("deleteFiles", delete_files.to_string()),
            ],
        )
        .map(|_| ())
    }

    fn files(&self, hash: &str) -> Result<Vec<TorrentFile>, String> {
        self.json(&format!("/api/v2/torrents/files?hash={hash}"))
    }

    fn rename_file(&self, hash: &str, old: &str, new: &str) -> Result<(), String> {
        self.post(
            "/api/v2/torrents/renameFile",
            &[
                ("hash", hash.into()),
                ("oldPath", old.into()),
                ("newPath", new.into()),
            ],
        )
        .map(|_| ())
    }

    fn recheck(&self, hash: &str) -> Result<(), String> {
        self.post("/api/v2/torrents/recheck", &[("hashes", hash.into())])
            .map(|_| ())
    }

    fn start(&self, hashes: &[String]) -> Result<(), String> {
        self.post("/api/v2/torrents/start", &[("hashes", joined(hashes))])
            .map(|_| ())
    }

    fn set_category(&self, hashes: &[String], category: &str) -> Result<(), String> {
        self.post(
            "/api/v2/torrents/setCategory",
            &[("hashes", joined(hashes)), ("category", category.into())],
        )
        .map(|_| ())
    }

    fn categories(&self) -> Result<Vec<String>, String> {
        let found: serde_json::Map<String, Value> = self.json("/api/v2/torrents/categories")?;
        Ok(found.keys().cloned().collect())
    }

    fn create_category(&self, name: &str, path: &str) -> Result<(), String> {
        self.post(
            "/api/v2/torrents/createCategory",
            &[("category", name.into()), ("savePath", path.into())],
        )
        .map(|_| ())
    }

    fn add_tags(&self, hashes: &[String], tag: &str) -> Result<(), String> {
        self.post(
            "/api/v2/torrents/addTags",
            &[("hashes", joined(hashes)), ("tags", tag.into())],
        )
        .map(|_| ())
    }

    fn remove_tags(&self, hashes: &[String], tag: &str) -> Result<(), String> {
        self.post(
            "/api/v2/torrents/removeTags",
            &[("hashes", joined(hashes)), ("tags", tag.into())],
        )
        .map(|_| ())
    }

    fn tags(&self) -> Result<Vec<String>, String> {
        self.json("/api/v2/torrents/tags")
    }

    fn delete_tags(&self, names: &[String]) -> Result<(), String> {
        self.post("/api/v2/torrents/deleteTags", &[("tags", names.join(","))])
            .map(|_| ())
    }

    fn trackers(&self, hash: &str) -> Result<Vec<String>, String> {
        let found: Vec<Value> = self.json(&format!("/api/v2/torrents/trackers?hash={hash}"))?;
        Ok(found
            .iter()
            .filter_map(|tracker| tracker["url"].as_str().map(str::to_string))
            .collect())
    }

    fn keep_in_place(&self, hashes: &[String]) -> Result<(), String> {
        self.post(
            "/api/v2/torrents/setAutoManagement",
            &[("hashes", joined(hashes)), ("enable", "false".into())],
        )
        .map(|_| ())
    }

    fn reset_share_limits(&self, hashes: &[String]) -> Result<(), String> {
        self.post(
            "/api/v2/torrents/setShareLimits",
            &[
                ("hashes", joined(hashes)),
                ("ratioLimit", "-2".into()),
                ("seedingTimeLimit", "-2".into()),
                ("inactiveSeedingTimeLimit", "-2".into()),
                ("shareLimitAction", "Default".into()),
                ("shareLimitsMode", "Default".into()),
            ],
        )
        .map(|_| ())
    }

    fn preferences(&self) -> Result<Value, String> {
        self.json("/api/v2/app/preferences")
    }

    fn set_preferences(&self, preferences: &Value) -> Result<(), String> {
        self.post(
            "/api/v2/app/setPreferences",
            &[("json", preferences.to_string())],
        )
        .map(|_| ())
    }

    fn pause(&self, seconds: u64) {
        thread::sleep(Duration::from_secs(seconds));
    }
}

pub fn wait_of(header: Option<&str>) -> f64 {
    match header
        .map(str::trim)
        .and_then(|value| value.parse::<u64>().ok())
    {
        Some(seconds) => seconds.max(60) as f64,
        None => 3600.0,
    }
}

pub struct Prowlarr {
    url: String,
    key: String,
    spacing: Duration,
    last: Cell<Option<Instant>>,
    inside: ureq::Agent,
    outside: ureq::Agent,
}

impl Prowlarr {
    pub fn new(url: &str, key: &str, proxy: Option<&str>) -> Result<Self, String> {
        Ok(Prowlarr {
            url: url.trim_end_matches('/').into(),
            key: key.into(),
            spacing: Duration::from_secs(2),
            last: Cell::new(None),
            inside: http::agent(None, 60)?,
            outside: http::agent(proxy, 60)?,
        })
    }

    fn open(
        &self,
        url: &str,
        method: &str,
        body: Option<Value>,
        headers: &[(&str, String)],
    ) -> Result<Vec<u8>, Fail> {
        if !url.starts_with(&self.url) {
            let reply = http::call(&self.outside, "GET", url, headers, None)?;
            if !reply.ok() {
                return Err(Fail::Other(format!("{url} answered {}", reply.status)));
            }
            return Ok(reply.body);
        }
        if let Some(last) = self.last.get() {
            let since = last.elapsed();
            if since < self.spacing {
                thread::sleep(self.spacing - since);
            }
        }
        let mut all = vec![("x-api-key", self.key.clone())];
        all.extend(headers.iter().cloned());
        let sent = http::call(
            &self.inside,
            method,
            url,
            &all,
            body.map(|body| ("application/json", body.to_string().into_bytes())),
        );
        self.last.set(Some(Instant::now()));
        let reply = sent?;
        if reply.status == 429 {
            return Err(Fail::Throttled {
                wait: wait_of(reply.retry_after.as_deref()),
                disabled: reply.text().contains("disabled"),
            });
        }
        if !reply.ok() {
            return Err(Fail::Other(format!("Prowlarr answered {}", reply.status)));
        }
        Ok(reply.body)
    }

    fn api(&self, path: &str, method: &str, body: Option<Value>) -> Result<Value, Fail> {
        let bytes = self.open(&format!("{}/api/v1{path}", self.url), method, body, &[])?;
        serde_json::from_slice(&bytes).map_err(|error| Fail::Other(error.to_string()))
    }
}

impl Indexers for Prowlarr {
    fn search(&self, indexer: &str, query: &str, now: f64) -> Result<Vec<Release>, Fail> {
        let url = format!(
            "{}/{indexer}/api?t=search&q={}&limit=100",
            self.url,
            http::encode(query)
        );
        parse_torznab(&self.open(&url, "GET", None, &[])?, now).map_err(Fail::Other)
    }

    fn torrent(&self, release: &Release) -> Result<Vec<u8>, Fail> {
        let link = match nyaa_view(&release.page) {
            Some(id) => format!("https://nyaa.si/download/{id}.torrent"),
            None => release.link.clone(),
        };
        self.open(
            &link,
            "GET",
            None,
            &[(
                "user-agent",
                "Mozilla/5.0 (compatible; jellynas-ratio)".into(),
            )],
        )
    }

    fn hold(
        &self,
        indexer: &str,
        paused: bool,
        profile: &str,
        restore: Option<i64>,
    ) -> Result<Option<i64>, Fail> {
        let profiles = self.api("/appprofile", "GET", None)?;
        let profiles = profiles.as_array().cloned().unwrap_or_default();
        let held = match profiles.iter().find(|p| p["name"] == profile) {
            Some(found) => found.clone(),
            None if !paused => return Ok(None),
            None => self.api(
                "/appprofile",
                "POST",
                Some(json!({
                    "name": profile,
                    "enableRss": false,
                    "enableAutomaticSearch": false,
                    "enableInteractiveSearch": false,
                    "minimumSeeders": 1,
                })),
            )?,
        };
        let held_id = held["id"].as_i64().unwrap_or_default();
        let mut definition = self.api(&format!("/indexer/{indexer}"), "GET", None)?;
        let before = definition["appProfileId"].as_i64().unwrap_or_default();
        if paused == (before == held_id) {
            return Ok(None);
        }
        if paused {
            definition["appProfileId"] = json!(held_id);
        } else {
            let others: Vec<i64> = profiles
                .iter()
                .filter_map(|p| p["id"].as_i64())
                .filter(|id| *id != held_id)
                .collect();
            let Some(first) = others.first() else {
                return Err(Fail::Other(format!(
                    "Prowlarr has no sync profile other than {profile}"
                )));
            };
            let back = restore.filter(|id| others.contains(id)).unwrap_or(*first);
            definition["appProfileId"] = json!(back);
        }
        self.api(&format!("/indexer/{indexer}"), "PUT", Some(definition))?;
        Ok(Some(before))
    }
}

fn nyaa_view(page: &str) -> Option<&str> {
    let start = page.find("nyaa.si/view/")? + "nyaa.si/view/".len();
    let digits = &page[start..];
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    (end > 0).then(|| &digits[..end])
}

pub struct C411 {
    url: String,
    profile_key: String,
    torznab_key: String,
    spacing: Duration,
    last: Cell<Option<Instant>>,
    calls_left: Cell<Option<u32>>,
    agent: ureq::Agent,
}

impl C411 {
    pub fn new(profile_key: &str, torznab_key: &str, proxy: Option<&str>) -> Result<Self, String> {
        Ok(C411 {
            url: C411_URL.into(),
            profile_key: profile_key.into(),
            torznab_key: torznab_key.into(),
            spacing: Duration::from_secs(6),
            last: Cell::new(None),
            calls_left: Cell::new(None),
            agent: http::agent(proxy, 30)?,
        })
    }

    fn get(&self, path: &str, key: &str, query: &str) -> Result<Vec<u8>, Fail> {
        if key.is_empty() {
            return Err(Fail::Other("the API key of C411 is empty".into()));
        }
        if let Some(left) = self.calls_left.get() {
            if left == 0 {
                return Err(Fail::OutOfCalls);
            }
            self.calls_left.set(Some(left - 1));
        }
        if let Some(last) = self.last.get() {
            let since = last.elapsed();
            if since < self.spacing {
                thread::sleep(self.spacing - since);
            }
        }
        let address = if query.is_empty() {
            format!("{}{path}", self.url)
        } else {
            format!("{}{path}?{query}", self.url)
        };
        let sent = http::call(
            &self.agent,
            "GET",
            &address,
            &[
                ("authorization", format!("Bearer {key}")),
                ("user-agent", "jellynas-ratio".into()),
            ],
            None,
        );
        self.last.set(Some(Instant::now()));
        let reply = sent?;
        if (400..500).contains(&reply.status) && reply.status != 429 {
            return Err(Fail::Refused(format!("C411 answered {}", reply.status)));
        }
        if !reply.ok() {
            return Err(Fail::Other(format!("C411 answered {}", reply.status)));
        }
        Ok(reply.body)
    }

    fn json(&self, path: &str, key: &str, query: &str) -> Result<Value, Fail> {
        serde_json::from_slice(&self.get(path, key, query)?)
            .map_err(|error| Fail::Other(error.to_string()))
    }
}

impl Tracker for C411 {
    fn allow(&self, calls: u32) {
        self.calls_left.set(Some(calls));
    }

    fn account(&self) -> Result<Account, Fail> {
        account_of(&self.json("/api/user/profile", &self.profile_key, "")?).map_err(Fail::Other)
    }

    fn catalog(&self, now: f64) -> Result<Vec<Release>, Fail> {
        let answer = self.json(
            "/api/torrents",
            &self.profile_key,
            "sortBy=createdAt&sortOrder=desc&perPage=50&page=1",
        )?;
        Ok(answer["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| {
                release_of(item, now)
                    .inspect_err(|error| println!("Skipped {error}"))
                    .ok()
            })
            .collect())
    }

    fn download(&self, infohash: &str) -> Result<Vec<u8>, Fail> {
        let query = format!(
            "t=get&id={}&apikey={}",
            http::encode(infohash),
            http::encode(&self.torznab_key)
        );
        self.get("/api", &self.torznab_key, &query)
    }
}

pub fn account_of(profile: &Value) -> Result<Account, String> {
    let transfer = &profile["transfer"];
    let total = |side: &str| {
        transfer[side]["total"]
            .as_f64()
            .map(|value| value as i64)
            .ok_or_else(|| format!("the profile of C411 has no {side} total"))
    };
    let kind = &profile["class"];
    let grade = &kind["progress"]["grade"];
    Ok(Account {
        uploaded: total("uploaded")?,
        downloaded: total("downloaded")?,
        actual: transfer["uploaded"]["actual"]
            .as_f64()
            .map(|value| value as i64),
        credit: transfer["uploaded"]["credit"]
            .as_f64()
            .map(|value| value as i64),
        zen: profile["zen"]["balance"]
            .as_f64()
            .ok_or("the profile of C411 has no Zen balance")?,
        earned: profile["zen"]["totalEarned"].as_f64(),
        class: kind["slug"].as_str().map(str::to_string),
        grade: if grade.is_object() {
            json!([grade["tier"], grade["level"]])
        } else {
            Value::Null
        },
        allowed: profile["download"]["allowed"].as_bool().unwrap_or(true),
    })
}

fn number_of(value: &Value) -> i64 {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|x| x as i64))
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .unwrap_or_default()
}

pub fn release_of(item: &Value, now: f64) -> Result<Release, String> {
    let created = item["createdAt"]
        .as_str()
        .and_then(|text| text.parse::<jiff::Timestamp>().ok())
        .ok_or("a release of C411 has no creation time")?;
    Ok(Release {
        infohash: item["infoHash"]
            .as_str()
            .ok_or("a release of C411 has no infohash")?
            .to_lowercase(),
        title: item["name"].as_str().unwrap_or_default().into(),
        page: String::new(),
        link: String::new(),
        size: number_of(&item["size"]),
        seeders: number_of(&item["seeders"]),
        leechers: number_of(&item["leechers"]),
        age_hours: (now - created.as_millisecond() as f64 / 1000.0) / 3600.0,
        free: item["isFreeleech"].as_bool().unwrap_or(false),
    })
}

pub fn parse_torznab(xml: &[u8], now: f64) -> Result<Vec<Release>, String> {
    let text = String::from_utf8_lossy(xml);
    let document = roxmltree::Document::parse(&text).map_err(|error| error.to_string())?;
    let mut releases = vec![];
    for item in document
        .descendants()
        .filter(|node| node.has_tag_name("item"))
    {
        let child = |name: &str| {
            item.children()
                .find(|node| node.has_tag_name(name))
                .and_then(|node| node.text())
                .map(str::to_string)
        };
        let attr = |name: &str| {
            item.children()
                .find(|node| {
                    node.tag_name().name() == "attr"
                        && node.tag_name().namespace() == Some(TORZNAB)
                        && node.attribute("name") == Some(name)
                })
                .and_then(|node| node.attribute("value"))
                .map(str::to_string)
        };
        let Some(published) = child("pubDate")
            .and_then(|date| jiff::fmt::rfc2822::parse(date.trim()).ok())
            .map(|zoned| zoned.timestamp().as_millisecond() as f64 / 1000.0)
        else {
            continue;
        };
        let (Some(infohash), Some(link)) = (attr("infohash"), child("link")) else {
            continue;
        };
        let count = |name: &str| attr(name).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let seeders = count("seeders");
        releases.push(Release {
            infohash: infohash.to_lowercase(),
            title: child("title").unwrap_or_default(),
            link,
            page: child("comments")
                .or_else(|| child("guid"))
                .unwrap_or_default(),
            size: child("size").and_then(|s| s.parse().ok()).unwrap_or(0),
            seeders,
            leechers: (count("peers") - seeders).max(0),
            age_hours: (now - published) / 3600.0,
            free: attr("downloadvolumefactor").as_deref() == Some("0"),
        });
    }
    Ok(releases)
}

#[derive(Debug, PartialEq)]
enum Bencode {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Bencode>),
    Dict(Vec<(Vec<u8>, Bencode)>),
}

fn bdecode(data: &[u8], start: usize) -> Result<(Bencode, usize), String> {
    let broken = || "the torrent file is not valid bencode".to_string();
    match data.get(start) {
        Some(b'i') => {
            let end = start
                + data[start..]
                    .iter()
                    .position(|b| *b == b'e')
                    .ok_or_else(broken)?;
            let text = std::str::from_utf8(&data[start + 1..end]).map_err(|_| broken())?;
            Ok((Bencode::Int(text.parse().map_err(|_| broken())?), end + 1))
        }
        Some(b'l') | Some(b'd') => {
            let mut items = vec![];
            let mut position = start + 1;
            while data.get(position) != Some(&b'e') {
                if position >= data.len() {
                    return Err(broken());
                }
                let (item, next) = bdecode(data, position)?;
                items.push(item);
                position = next;
            }
            if data[start] == b'l' {
                return Ok((Bencode::List(items), position + 1));
            }
            let mut pairs = vec![];
            let mut items = items.into_iter();
            while let (Some(Bencode::Bytes(key)), Some(value)) = (items.next(), items.next()) {
                pairs.push((key, value));
            }
            Ok((Bencode::Dict(pairs), position + 1))
        }
        Some(_) => {
            let colon = start
                + data[start..]
                    .iter()
                    .position(|b| *b == b':')
                    .ok_or_else(broken)?;
            let length: usize = std::str::from_utf8(&data[start..colon])
                .map_err(|_| broken())?
                .parse()
                .map_err(|_| broken())?;
            let end = colon + 1 + length;
            let bytes = data.get(colon + 1..end).ok_or_else(broken)?;
            Ok((Bencode::Bytes(bytes.to_vec()), end))
        }
        None => Err(broken()),
    }
}

fn field<'a>(value: &'a Bencode, name: &str) -> Option<&'a Bencode> {
    match value {
        Bencode::Dict(pairs) => pairs
            .iter()
            .find(|(key, _)| key == name.as_bytes())
            .map(|(_, value)| value),
        _ => None,
    }
}

pub fn file_lengths(torrent: &[u8]) -> Result<Vec<i64>, String> {
    let (root, _) = bdecode(torrent, 0)?;
    let info = field(&root, "info").ok_or("the torrent file has no info")?;
    let length = |value: &Bencode| match field(value, "length") {
        Some(Bencode::Int(length)) => Ok(*length),
        _ => Err("a file of the torrent has no length".to_string()),
    };
    let mut lengths = match field(info, "files") {
        Some(Bencode::List(files)) => files.iter().map(length).collect::<Result<Vec<_>, _>>()?,
        _ => vec![length(info)?],
    };
    lengths.sort();
    Ok(lengths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multi_file_torrent_gives_its_sorted_lengths() {
        let torrent =
            b"d4:infod5:filesld6:lengthi30e4:pathl1:aeed6:lengthi10e4:pathl1:beee4:name1:xee";
        assert_eq!(Ok(vec![10, 30]), file_lengths(torrent));
    }

    #[test]
    fn a_single_file_torrent_gives_its_length() {
        assert_eq!(
            Ok(vec![42]),
            file_lengths(b"d4:infod6:lengthi42e4:name1:xee")
        );
    }

    #[test]
    fn a_broken_torrent_is_refused() {
        assert!(file_lengths(b"d4:infod6:length").is_err());
    }

    #[test]
    fn torznab_items_become_releases() {
        let xml = br#"<?xml version="1.0"?><rss xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>
            <item><title>Show S01E02 VOSTFR-Group</title><link>http://prowlarr:9696/1/download?x</link>
            <comments>https://nyaa.si/view/123</comments><size>1000</size>
            <pubDate>Sun, 22 Nov 2026 10:00:00 +0000</pubDate>
            <torznab:attr name="infohash" value="ABC"/><torznab:attr name="seeders" value="4"/>
            <torznab:attr name="peers" value="10"/><torznab:attr name="downloadvolumefactor" value="0"/></item>
            <item><title>No hash</title><link>x</link><pubDate>Sun, 22 Nov 2026 10:00:00 +0000</pubDate></item>
            </channel></rss>"#;
        let now = "2026-11-22T12:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_second() as f64;
        let releases = parse_torznab(xml, now).unwrap();
        assert_eq!(1, releases.len());
        let release = &releases[0];
        assert_eq!("abc", release.infohash);
        assert_eq!(6, release.leechers);
        assert_eq!(2.0, release.age_hours);
        assert!(release.free);
        assert_eq!(Some("123"), nyaa_view(&release.page));
    }

    #[test]
    fn a_c411_profile_becomes_an_account() {
        let profile = json!({
            "transfer": {"uploaded": {"total": 300, "actual": 200, "credit": 100}, "downloaded": {"total": 150}},
            "zen": {"balance": 1234, "totalEarned": 5000},
            "class": {"slug": "elite", "progress": {"grade": {"tier": 2, "level": 3}}},
            "download": {"allowed": false},
        });
        let account = account_of(&profile).unwrap();
        assert_eq!(300, account.uploaded);
        assert_eq!(Some(200), account.actual);
        assert_eq!(json!([2, 3]), account.grade);
        assert!(!account.allowed);
    }

    #[test]
    fn retry_after_waits_a_minute_at_least_and_an_hour_without_it() {
        assert_eq!(60.0, wait_of(Some("5")));
        assert_eq!(120.0, wait_of(Some("120")));
        assert_eq!(3600.0, wait_of(None));
    }
}
