mod api;
mod arr;
mod jellyfin;

use api::Http;
use std::time::{Duration, SystemTime};
use std::{env, fs, process, thread};

const INTERVAL: Duration = Duration::from_secs(600);
const HEARTBEAT: &str = "/tmp/heartbeat";

enum App {
    Jellyfin(Http),
    Arr(arr::Arr, Http),
}

impl App {
    fn name(&self) -> &'static str {
        match self {
            App::Jellyfin(_) => "Jellyfin",
            App::Arr(arr, _) => arr.name,
        }
    }

    fn enforce(&self) -> Result<usize, String> {
        match self {
            App::Jellyfin(api) => jellyfin::enforce(api),
            App::Arr(arr, api) => arr::enforce(arr, api),
        }
    }
}

fn apps(vars: impl Fn(&str) -> Option<String>) -> Vec<App> {
    let read = |name: &str| {
        vars(name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let mut apps = vec![];
    if let (Some(domain), Some(key)) = (read("DOMAIN"), read("JELLYFIN_API_KEY")) {
        let port = read("CADDY_HTTPS_PORT").unwrap_or_else(|| "443".into());
        apps.push(App::Jellyfin(Http {
            base: format!("https://jellyfin.{domain}:{port}"),
            header: "Authorization",
            secret: format!("MediaBrowser Token=\"{key}\""),
        }));
    }
    for (arr, base, variable) in [
        (arr::radarr(), "http://radarr:7878/api/v3", "RADARR_API_KEY"),
        (arr::sonarr(), "http://sonarr:8989/api/v3", "SONARR_API_KEY"),
    ] {
        if let Some(key) = read(variable) {
            let api = Http {
                base: base.into(),
                header: "X-Api-Key",
                secret: key,
            };
            apps.push(App::Arr(arr, api));
        }
    }
    apps
}

fn healthy(last_pass: Option<SystemTime>, now: SystemTime) -> bool {
    last_pass
        .and_then(|time| now.duration_since(time).ok())
        .is_some_and(|age| age < INTERVAL * 3)
}

fn main() {
    if env::args().nth(1).as_deref() == Some("health") {
        let last_pass = fs::metadata(HEARTBEAT)
            .and_then(|meta| meta.modified())
            .ok();
        process::exit(if healthy(last_pass, SystemTime::now()) {
            0
        } else {
            1
        });
    }
    let apps = apps(|name| env::var(name).ok());
    if apps.is_empty() {
        eprintln!("No app to keep: set JELLYFIN_API_KEY, RADARR_API_KEY or SONARR_API_KEY in .env");
        process::exit(1);
    }
    let names: Vec<&str> = apps.iter().map(App::name).collect();
    println!("Keeping the settings of {}", names.join(", "));
    loop {
        let mut failed = false;
        for app in &apps {
            if let Err(error) = app.enforce() {
                eprintln!("Cannot keep the settings of {}: {error}", app.name());
                failed = true;
            }
        }
        if !failed && let Err(error) = fs::write(HEARTBEAT, b"ok") {
            eprintln!("Cannot write the heartbeat: {error}");
        }
        thread::sleep(INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn every_app_with_a_key_is_kept() {
        let found = apps(vars(&[
            ("DOMAIN", "example.org"),
            ("CADDY_HTTPS_PORT", "50000"),
            ("JELLYFIN_API_KEY", "j"),
            ("RADARR_API_KEY", "r"),
            ("SONARR_API_KEY", "s"),
        ]));
        let names: Vec<&str> = found.iter().map(App::name).collect();
        assert_eq!(vec!["Jellyfin", "Radarr", "Sonarr"], names);
        let App::Jellyfin(api) = &found[0] else {
            panic!("Jellyfin comes first")
        };
        assert_eq!("https://jellyfin.example.org:50000", api.base);
        assert_eq!("MediaBrowser Token=\"j\"", api.secret);
        let App::Arr(_, api) = &found[1] else {
            panic!("Radarr comes second")
        };
        assert_eq!("http://radarr:7878/api/v3", api.base);
        assert_eq!("r", api.secret);
    }

    #[test]
    fn an_app_without_a_key_is_skipped() {
        let found = apps(vars(&[("DOMAIN", "example.org"), ("SONARR_API_KEY", " ")]));
        assert!(found.is_empty());
    }

    #[test]
    fn jellyfin_defaults_to_the_https_port() {
        let found = apps(vars(&[
            ("DOMAIN", "example.org"),
            ("JELLYFIN_API_KEY", "j"),
        ]));
        let App::Jellyfin(api) = &found[0] else {
            panic!("Jellyfin is kept")
        };
        assert_eq!("https://jellyfin.example.org:443", api.base);
    }

    #[test]
    fn health_needs_a_recent_pass() {
        let now = SystemTime::now();
        assert!(!healthy(None, now));
        assert!(healthy(Some(now - INTERVAL), now));
        assert!(!healthy(
            Some(now - INTERVAL * 3 - Duration::from_secs(1)),
            now
        ));
    }
}
