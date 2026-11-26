mod accounts;
mod api;
mod arr;
mod aurral;
mod jellyfin;
mod kavita;
mod lazylibrarian;

use accounts::{Account, Directory, PRIVATE_ROOT};
use api::Http;
use jiff::ZonedDateTime;
use jiff::civil::{Time, time};
use std::path::Path;
use std::time::{Duration, SystemTime};
use std::{env, fs, process, thread};

const PASS_AT: Time = time(4, 30, 0, 0);
const STALE: Duration = Duration::from_secs(49 * 3600);
const HEARTBEAT: &str = "/tmp/heartbeat";
const RETRY: i64 = 30;

enum App {
    Jellyfin(Http),
    Kavita(Http, String),
    Arr(arr::Arr, Http),
    Aurral(Http),
    LazyLibrarian(Http),
}

impl App {
    fn name(&self) -> &'static str {
        match self {
            App::Jellyfin(_) => "Jellyfin",
            App::Kavita(..) => "Kavita",
            App::Arr(arr, _) => arr.name,
            App::Aurral(_) => "Aurral",
            App::LazyLibrarian(_) => "LazyLibrarian",
        }
    }

    fn enforce(&self, accounts: &[Account]) -> Result<usize, String> {
        match self {
            App::Jellyfin(api) => jellyfin::enforce(api, accounts),
            App::Kavita(api, authority) => kavita::enforce(api, accounts, authority),
            App::Arr(arr, api) => {
                let private: Vec<String> = accounts.iter().map(Account::videos).collect();
                arr::enforce(arr, api, &private)
            }
            App::Aurral(api) => aurral::enforce(api),
            App::LazyLibrarian(api) => lazylibrarian::enforce(api),
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
    let port = read("CADDY_HTTPS_PORT").unwrap_or_else(|| "443".into());
    if let (Some(domain), Some(key)) = (read("DOMAIN"), read("JELLYFIN_API_KEY")) {
        apps.push(App::Jellyfin(Http {
            base: format!("https://jellyfin.{domain}:{port}"),
            header: Some(("Authorization", format!("MediaBrowser Token=\"{key}\""))),
        }));
    }
    if let (Some(domain), Some(key)) = (read("DOMAIN"), read("KAVITA_API_KEY")) {
        apps.push(App::Kavita(
            Http {
                base: format!("https://kavita.{domain}:{port}/api"),
                header: Some(("x-api-key", key)),
            },
            format!("https://auth.{domain}:{port}"),
        ));
    }
    for (arr, base, variable) in [
        (arr::radarr(), "http://radarr:7878/api/v3", "RADARR_API_KEY"),
        (arr::sonarr(), "http://sonarr:8989/api/v3", "SONARR_API_KEY"),
    ] {
        if let Some(key) = read(variable) {
            let api = Http {
                base: base.into(),
                header: Some(("X-Api-Key", key)),
            };
            apps.push(App::Arr(arr, api));
        }
    }
    if let (Some(domain), Some(key)) = (read("DOMAIN"), read("AURRAL_API_KEY")) {
        apps.push(App::Aurral(Http {
            base: format!("https://aurral.{domain}:{port}/api"),
            header: Some(("X-Api-Key", key)),
        }));
    }
    if let Some(key) = read("LAZYLIBRARIAN_API_KEY") {
        apps.push(App::LazyLibrarian(Http {
            base: format!("http://lazylibrarian:5299/api?apikey={key}"),
            header: None,
        }));
    }
    apps
}

fn directory(vars: impl Fn(&str) -> Option<String>) -> Option<Directory> {
    vars("LLDAP_POLICY_PASSWORD")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|password| Directory {
            base: "http://lldap:17170".into(),
            password,
        })
}

fn private_accounts(directory: &Directory) -> Result<Vec<Account>, String> {
    let accounts = directory.accounts()?;
    accounts::shelve(Path::new(PRIVATE_ROOT), &accounts)?;
    Ok(accounts)
}

fn healthy(last_pass: Option<SystemTime>, now: SystemTime) -> bool {
    last_pass
        .and_then(|time| now.duration_since(time).ok())
        .is_some_and(|age| age < STALE)
}

fn next_pass(failed: bool, now: &ZonedDateTime) -> Result<ZonedDateTime, String> {
    if failed {
        return now
            .checked_add(jiff::Span::new().seconds(RETRY))
            .map_err(|error| error.to_string());
    }
    let today = now
        .with()
        .time(PASS_AT)
        .build()
        .map_err(|error| error.to_string())?;
    if today > *now {
        return Ok(today);
    }
    now.tomorrow()
        .and_then(|day| day.with().time(PASS_AT).build())
        .map_err(|error| error.to_string())
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
        eprintln!("No app to keep: set the API key of an app in .env");
        process::exit(1);
    }
    let directory = directory(|name| env::var(name).ok());
    let names: Vec<&str> = apps.iter().map(App::name).collect();
    println!("Keeping the settings of {}", names.join(", "));
    loop {
        let mut failed = false;
        let accounts = match &directory {
            Some(directory) => private_accounts(directory).unwrap_or_else(|error| {
                eprintln!("Cannot prepare the private libraries: {error}");
                failed = true;
                vec![]
            }),
            None => vec![],
        };
        for app in &apps {
            if let Err(error) = app.enforce(&accounts) {
                eprintln!("Cannot keep the settings of {}: {error}", app.name());
                failed = true;
            }
        }
        if !failed && let Err(error) = fs::write(HEARTBEAT, b"ok") {
            eprintln!("Cannot write the heartbeat: {error}");
        }
        let now = ZonedDateTime::now();
        let next = next_pass(failed, &now).unwrap_or_else(|error| {
            eprintln!("Cannot plan the next pass: {error}");
            process::exit(1);
        });
        println!("Next pass at {next}");
        thread::sleep(Duration::try_from(now.duration_until(&next)).unwrap_or_default());
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

    fn secret(api: &Http) -> Option<&str> {
        api.header.as_ref().map(|(_, value)| value.as_str())
    }

    #[test]
    fn every_app_with_a_key_is_kept() {
        let found = apps(vars(&[
            ("DOMAIN", "example.org"),
            ("CADDY_HTTPS_PORT", "50000"),
            ("JELLYFIN_API_KEY", "j"),
            ("KAVITA_API_KEY", "k"),
            ("RADARR_API_KEY", "r"),
            ("SONARR_API_KEY", "s"),
            ("AURRAL_API_KEY", "a"),
            ("LAZYLIBRARIAN_API_KEY", "l"),
        ]));
        let names: Vec<&str> = found.iter().map(App::name).collect();
        let expected = [
            "Jellyfin",
            "Kavita",
            "Radarr",
            "Sonarr",
            "Aurral",
            "LazyLibrarian",
        ];
        assert_eq!(expected.to_vec(), names);
        let App::Jellyfin(api) = &found[0] else {
            panic!("Jellyfin comes first")
        };
        assert_eq!("https://jellyfin.example.org:50000", api.base);
        assert_eq!(Some("MediaBrowser Token=\"j\""), secret(api));
        let App::Kavita(api, authority) = &found[1] else {
            panic!("Kavita comes second")
        };
        assert_eq!("https://kavita.example.org:50000/api", api.base);
        assert_eq!(Some("k"), secret(api));
        assert_eq!("https://auth.example.org:50000", authority);
        let App::Arr(_, api) = &found[2] else {
            panic!("Radarr comes third")
        };
        assert_eq!("http://radarr:7878/api/v3", api.base);
        assert_eq!(Some("r"), secret(api));
        let App::Aurral(api) = &found[4] else {
            panic!("Aurral comes fifth")
        };
        assert_eq!("https://aurral.example.org:50000/api", api.base);
        assert_eq!(Some("a"), secret(api));
        let App::LazyLibrarian(api) = &found[5] else {
            panic!("LazyLibrarian comes sixth")
        };
        assert_eq!("http://lazylibrarian:5299/api?apikey=l", api.base);
        assert_eq!(None, secret(api));
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
    fn the_private_libraries_need_the_password_of_policy_in_lldap() {
        assert!(directory(vars(&[("LLDAP_POLICY_PASSWORD", " ")])).is_none());
        let found = directory(vars(&[("LLDAP_POLICY_PASSWORD", "p")])).unwrap();
        assert_eq!("http://lldap:17170", found.base);
        assert_eq!("p", found.password);
    }

    #[test]
    fn health_needs_a_recent_pass() {
        let now = SystemTime::now();
        assert!(!healthy(None, now));
        assert!(healthy(Some(now - Duration::from_secs(48 * 3600)), now));
        assert!(!healthy(Some(now - STALE), now));
    }

    fn at(moment: &str) -> ZonedDateTime {
        moment.parse().unwrap()
    }

    #[test]
    fn a_pass_before_half_past_four_waits_for_the_same_night() {
        assert_eq!(
            at("2026-11-21T04:30:00+01:00[Europe/Paris]"),
            next_pass(false, &at("2026-11-21T01:00:00+01:00[Europe/Paris]")).unwrap()
        );
    }

    #[test]
    fn a_pass_from_half_past_four_waits_for_the_next_night() {
        assert_eq!(
            at("2026-11-22T04:30:00+01:00[Europe/Paris]"),
            next_pass(false, &at("2026-11-21T04:30:00+01:00[Europe/Paris]")).unwrap()
        );
    }

    #[test]
    fn a_failed_pass_tries_again_half_a_minute_later() {
        assert_eq!(
            at("2026-11-21T15:10:30+01:00[Europe/Paris]"),
            next_pass(true, &at("2026-11-21T15:10:00+01:00[Europe/Paris]")).unwrap()
        );
    }

    #[test]
    fn the_pass_keeps_local_time_across_a_change_of_hour() {
        assert_eq!(
            at("2026-03-29T04:30:00+02:00[Europe/Paris]"),
            next_pass(false, &at("2026-03-28T12:00:00+01:00[Europe/Paris]")).unwrap()
        );
    }
}
