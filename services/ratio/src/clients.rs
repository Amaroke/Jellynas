use crate::http::{self, Reply};
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::Cell;
use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

pub const C411_URL: &str = "https://c411.org";

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

#[derive(Clone, Debug, Default, Deserialize, PartialEq, serde::Serialize)]
#[serde(default)]
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
}

pub trait Indexers {
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

#[cfg(test)]
mod tests {
    use super::*;

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
