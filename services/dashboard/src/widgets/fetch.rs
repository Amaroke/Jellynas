use super::Reading;
use super::format::{count, date};
use super::parse;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LIMIT: u64 = 64 * 1024 * 1024;
const SLOW: Duration = Duration::from_secs(120);
const LATEST: &str = "/Items?SortBy=DateCreated&SortOrder=Descending&IncludeItemTypes=Movie,Episode&Recursive=true&Limit=300";
const SYSTEMS: Duration = Duration::from_secs(10);

pub struct Client {
    agent: ureq::Agent,
    probe: ureq::Agent,
    slow: ureq::Agent,
    env: HashMap<String, String>,
    systems: Mutex<Option<(Instant, Value)>>,
}

type Headers = Vec<(&'static str, String)>;

fn encode(input: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::new();
    for chunk in input.as_bytes().chunks(3) {
        let bits = chunk.iter().enumerate().fold(0u32, |bits, (index, byte)| {
            bits | (u32::from(*byte) << (16 - 8 * index))
        });
        for index in 0..4 {
            if index <= chunk.len() {
                output.push(char::from(
                    ALPHABET[(bits >> (18 - 6 * index)) as usize & 63],
                ));
            } else {
                output.push('=');
            }
        }
    }
    output
}

fn week() -> (String, String) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    (date(now), date(now + 7 * 86400))
}

impl Client {
    pub fn new(env: HashMap<String, String>) -> Self {
        Client {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(10)))
                .build()
                .into(),
            probe: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(10)))
                .max_redirects(0)
                .http_status_as_error(false)
                .build()
                .into(),
            slow: ureq::Agent::config_builder()
                .timeout_global(Some(SLOW))
                .build()
                .into(),
            env,
            systems: Mutex::new(None),
        }
    }

    fn var(&self, name: &str) -> Result<&str, String> {
        self.env
            .get(name)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{name} is not set"))
    }

    fn url(&self, prefix: &str, path: &str) -> Result<String, String> {
        let base = self.var(&format!("{prefix}_URL"))?;
        Ok(format!("{}{path}", base.trim_end_matches('/')))
    }

    fn key(&self, prefix: &str, header: &'static str) -> Result<Headers, String> {
        let key = self.var(&format!("{prefix}_API_KEY"))?;
        Ok(vec![(header, key.to_string())])
    }

    fn get(&self, url: &str, headers: &Headers) -> Result<Value, String> {
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        request
            .call()
            .and_then(|mut response| response.body_mut().with_config().limit(LIMIT).read_json())
            .map_err(|error| error.to_string())
    }

    fn post(&self, url: &str, headers: &Headers, body: &Value) -> Result<Value, String> {
        let mut request = self.agent.post(url);
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        request
            .send_json(body)
            .and_then(|mut response| response.body_mut().with_config().limit(LIMIT).read_json())
            .map_err(|error| error.to_string())
    }

    fn fetch(&self, prefix: &str, path: &str, headers: &Headers) -> Result<Value, String> {
        self.get(&self.url(prefix, path)?, headers)
    }

    pub fn feed(&self, prefix: &str, path: &str) -> Result<Value, String> {
        self.slow
            .get(self.url(prefix, path)?)
            .call()
            .and_then(|mut response| response.body_mut().with_config().limit(LIMIT).read_json())
            .map_err(|error| error.to_string())
    }

    pub fn send(&self, prefix: &str, path: &str, body: &Value) -> Result<(), String> {
        let request = self
            .slow
            .post(self.url(prefix, path)?)
            .header("Accept", "application/json");
        let sent = if body.is_null() {
            request.send_empty()
        } else {
            request.send_json(body)
        };
        sent.map(|_| ()).map_err(|error| error.to_string())
    }

    pub fn widget(&self, kind: &str) -> parse::Parsed {
        let arr = |prefix| self.key(prefix, "X-Api-Key");
        match kind {
            "jellyfin" => {
                let token = self.var("JELLYFIN_API_KEY")?;
                let auth = vec![("Authorization", format!("MediaBrowser Token=\"{token}\""))];
                parse::jellyfin(
                    &self.fetch("JELLYFIN", "/Items/Counts", &auth)?,
                    &self.fetch("JELLYFIN", "/Sessions?activeWithinSeconds=60", &auth)?,
                    &self
                        .fetch("JELLYFIN", LATEST, &auth)
                        .unwrap_or_else(|_| json!({"Items": []})),
                    &self.url("JELLYFIN", "")?,
                )
            }
            "music-bot" => parse::music_bot(
                &self.fetch("MUSIC_BOT", "/status", &vec![])?,
                &self.url("JELLYFIN", "")?,
            ),
            "seerr" => parse::seerr(
                &self.fetch("SEERR", "/api/v1/request/count", &arr("SEERR")?)?,
                &self.fetch("SEERR", "/api/v1/issue/count", &arr("SEERR")?)?,
            ),
            "qbittorrent" => {
                let key = self.var("QBITTORRENT_API_KEY")?;
                let auth = vec![("Authorization", format!("Bearer {key}"))];
                parse::qbittorrent(&self.fetch("QBITTORRENT", "/api/v2/torrents/info", &auth)?)
            }
            "gluetun" => parse::gluetun(&self.fetch(
                "GLUETUN",
                "/v1/publicip/ip",
                &self.key("GLUETUN", "X-API-Key").unwrap_or_default(),
            )?),
            "slskd" => {
                let auth = self.key("SLSKD", "X-API-Key")?;
                parse::slskd(
                    &self.fetch("SLSKD", "/api/v0/application", &auth)?,
                    &self.fetch("SLSKD", "/api/v0/transfers/downloads", &auth)?,
                    &self.fetch("SLSKD", "/api/v0/transfers/uploads", &auth)?,
                )
            }
            "autobrr" => {
                let auth = self.key("AUTOBRR", "X-API-Token")?;
                parse::autobrr(
                    &self.fetch("AUTOBRR", "/api/release/stats", &auth)?,
                    &self.fetch("AUTOBRR", "/api/filters", &auth)?,
                    &self.fetch("AUTOBRR", "/api/release/indexers", &auth)?,
                )
            }
            "prowlarr" => parse::prowlarr(&self.fetch(
                "PROWLARR",
                "/api/v1/indexerstats",
                &arr("PROWLARR")?,
            )?),
            "radarr" => {
                let (today, end) = week();
                parse::radarr(
                    &self.fetch("RADARR", "/api/v3/movie", &arr("RADARR")?)?,
                    &self.fetch("RADARR", "/api/v3/queue/status", &arr("RADARR")?)?,
                    &self.fetch("RADARR", "/api/v3/queue/details", &arr("RADARR")?)?,
                    &self
                        .fetch(
                            "RADARR",
                            &format!("/api/v3/calendar?start={today}&end={end}"),
                            &arr("RADARR")?,
                        )
                        .unwrap_or_else(|_| json!([])),
                    &today,
                )
            }
            "sonarr" => {
                let (today, end) = week();
                parse::sonarr(
                    &self.fetch("SONARR", "/api/v3/series", &arr("SONARR")?)?,
                    &self.fetch("SONARR", "/api/v3/queue", &arr("SONARR")?)?,
                    &self.fetch("SONARR", "/api/v3/wanted/missing", &arr("SONARR")?)?,
                    &self.fetch(
                        "SONARR",
                        "/api/v3/queue/details?includeEpisode=true",
                        &arr("SONARR")?,
                    )?,
                    &self
                        .fetch(
                            "SONARR",
                            &format!("/api/v3/calendar?start={today}&end={end}&includeSeries=true"),
                            &arr("SONARR")?,
                        )
                        .unwrap_or_else(|_| json!([])),
                )
            }
            "suwayomi" => self.suwayomi(),
            "bazarr" => {
                let auth = self.key("BAZARR", "X-API-KEY")?;
                parse::bazarr(
                    &self.fetch("BAZARR", "/api/movies/wanted?start=0&length=8", &auth)?,
                    &self.fetch("BAZARR", "/api/episodes/wanted?start=0&length=8", &auth)?,
                )
            }
            "tdarr" => {
                let body = json!({"data": {"collection": "StatisticsJSONDB", "mode": "getById", "docID": "statistics"}});
                let auth = self.key("TDARR", "x-api-key")?;
                parse::tdarr(
                    &self.post(&self.url("TDARR", "/api/v2/cruddb")?, &auth, &body)?,
                    &self
                        .fetch("TDARR", "/api/v2/get-nodes", &auth)
                        .unwrap_or_else(|_| json!({})),
                )
            }
            "beszel" => parse::beszel(&self.systems()?, self.var("BESZEL_SYSTEM")?),
            "uptime-kuma" => {
                let slug = self.var("UPTIME_KUMA_SLUG")?;
                parse::uptime_kuma(
                    &self.fetch("UPTIME_KUMA", &format!("/api/status-page/{slug}"), &vec![])?,
                    &self.fetch(
                        "UPTIME_KUMA",
                        &format!("/api/status-page/heartbeat/{slug}"),
                        &vec![],
                    )?,
                )
            }
            _ => Err(format!("unknown widget {kind}")),
        }
    }

    fn suwayomi(&self) -> parse::Parsed {
        let credentials = format!(
            "{}:{}",
            self.var("SUWAYOMI_USERNAME")?,
            self.var("SUWAYOMI_PASSWORD")?
        );
        let chapters = |name: &str, condition: &str| {
            format!(
                "{name}: chapters(condition: {{{condition}}} filter: {{inLibrary: {{equalTo: true}}}}) {{ totalCount }}"
            )
        };
        let query = format!(
            "query Counts {{ {} {} {} {} }}",
            chapters("download", "isDownloaded: true"),
            chapters("nondownload", "isDownloaded: false"),
            chapters("read", "isRead: true"),
            chapters("unread", "isRead: false"),
        );
        parse::suwayomi(&self.post(
            &self.url("SUWAYOMI", "/api/graphql")?,
            &vec![("Authorization", format!("Basic {}", encode(&credentials)))],
            &json!({"operationName": "Counts", "query": query}),
        )?)
    }

    fn systems(&self) -> Result<Value, String> {
        let mut cached = self
            .systems
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((read, systems)) = cached.as_ref()
            && read.elapsed() < SYSTEMS
        {
            return Ok(systems.clone());
        }
        let systems = self.login_and_list()?;
        *cached = Some((Instant::now(), systems.clone()));
        Ok(systems)
    }

    fn login_and_list(&self) -> Result<Value, String> {
        let login = self.post(
            &self.url("BESZEL", "/api/collections/_superusers/auth-with-password")?,
            &vec![],
            &json!({"identity": self.var("BESZEL_EMAIL")?, "password": self.var("BESZEL_PASSWORD")?}),
        )?;
        let token = login["token"]
            .as_str()
            .ok_or("Beszel gave no token")?
            .to_string();
        self.fetch(
            "BESZEL",
            "/api/collections/systems/records?page=1&perPage=500",
            &vec![("Authorization", format!("Bearer {token}"))],
        )
    }

    pub fn host(&self) -> parse::Parsed {
        parse::host(&self.systems()?, self.var("BESZEL_SYSTEM")?)
    }

    pub fn monitor(&self, url: &str) -> parse::Parsed {
        let started = Instant::now();
        let response = self
            .probe
            .get(url)
            .call()
            .map_err(|error| error.to_string())?;
        let elapsed = started.elapsed().as_millis() as u64;
        let status = response.status().as_u16();
        if status >= 400 {
            return Err(format!("http status: {status}"));
        }
        Ok(Reading {
            figures: vec![
                super::Figure {
                    label: "Status".into(),
                    value: status.to_string(),
                },
                super::Figure {
                    label: "Latency".into(),
                    value: format!("{} ms", count(elapsed)),
                },
            ],
            rows: vec![],
            posters: vec![],
            shelf: None,
            idle: None,
            metrics: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_credentials_are_base64() {
        assert_eq!("", encode(""));
        assert_eq!("YQ==", encode("a"));
        assert_eq!("YWI=", encode("ab"));
        assert_eq!("YWRtaW46czNjcjN0", encode("admin:s3cr3t"));
    }
}
