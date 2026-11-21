use serde_json::{Map, Value, json};
use std::time::{Duration, SystemTime};
use std::{env, fs, process, thread};

const INTERVAL: Duration = Duration::from_secs(600);
const HEARTBEAT: &str = "/tmp/heartbeat";

fn rules(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn policy() -> Map<String, Value> {
    rules(json!({
        "EnableLiveTvAccess": false,
        "EnableLiveTvManagement": false,
        "EnablePublicSharing": false,
        "EnableMediaConversion": false,
        "EnableContentDownloading": true,
        "EnableRemoteControlOfOtherUsers": false,
        "MaxActiveSessions": 1,
        "LoginAttemptsBeforeLockout": 3,
        "SyncPlayAccess": "CreateAndJoinGroups",
    }))
}

fn configuration() -> Map<String, Value> {
    rules(json!({
        "EnableNextEpisodeAutoPlay": true,
    }))
}

fn server() -> Vec<(&'static str, &'static str, Map<String, Value>)> {
    vec![
        (
            "General",
            "/System/Configuration",
            rules(json!({
                "ServerName": "Jellynas",
                "UICulture": "fr",
                "PreferredMetadataLanguage": "fr",
                "MetadataCountryCode": "FR",
                "QuickConnectAvailable": true,
                "AllowClientLogUpload": true,
                "EnableMetrics": false,
                "LogFileRetentionDays": 3,
                "ActivityLogRetentionDays": 30,
                "RemoteClientBitrateLimit": 0,
            })),
        ),
        (
            "Playback",
            "/System/Configuration/encoding",
            rules(json!({
                "HardwareAccelerationType": "none",
                "EnableThrottling": true,
                "EnableSegmentDeletion": true,
                "EnableTonemapping": false,
            })),
        ),
        (
            "Networking",
            "/System/Configuration/network",
            rules(json!({
                "KnownProxies": ["caddy"],
                "EnableRemoteAccess": true,
                "EnableUPnP": false,
            })),
        ),
    ]
}

fn wanted_policy(current: &Value) -> Map<String, Value> {
    let mut wanted = policy();
    if current["IsAdministrator"] != json!(true) {
        wanted.insert("EnableContentDeletion".into(), json!(false));
    }
    wanted
}

fn drift(current: &Value, wanted: &Map<String, Value>) -> Map<String, Value> {
    wanted
        .iter()
        .filter(|(key, value)| current.get(key.as_str()) != Some(value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn merged(current: &Value, changes: &Map<String, Value>) -> Value {
    let mut whole = current.as_object().cloned().unwrap_or_default();
    whole.extend(changes.clone());
    Value::Object(whole)
}

fn keys(changes: &[&Map<String, Value>]) -> String {
    let mut names: Vec<&str> = changes
        .iter()
        .flat_map(|change| change.keys())
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    names.join(", ")
}

trait Jellyfin {
    fn get(&self, path: &str) -> Result<Value, String>;
    fn post(&self, path: &str, body: &Value) -> Result<(), String>;
}

struct Server {
    base: String,
    authorization: String,
}

impl Jellyfin for Server {
    fn get(&self, path: &str) -> Result<Value, String> {
        ureq::get(&format!("{}{path}", self.base))
            .set("Authorization", &self.authorization)
            .timeout(Duration::from_secs(30))
            .call()
            .map_err(|error| error.to_string())?
            .into_json()
            .map_err(|error| error.to_string())
    }

    fn post(&self, path: &str, body: &Value) -> Result<(), String> {
        ureq::post(&format!("{}{path}", self.base))
            .set("Authorization", &self.authorization)
            .timeout(Duration::from_secs(30))
            .send_json(body)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn settle(
    jellyfin: &impl Jellyfin,
    path: &str,
    current: &Value,
    wanted: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    let changes = drift(current, wanted);
    if !changes.is_empty() {
        jellyfin.post(path, &merged(current, &changes))?;
    }
    Ok(changes)
}

fn enforce_server(jellyfin: &impl Jellyfin) -> Result<usize, String> {
    let mut changed = 0;
    for (name, path, wanted) in server() {
        let changes = settle(jellyfin, path, &jellyfin.get(path)?, &wanted)?;
        if !changes.is_empty() {
            println!("{name}: set {}", keys(&[&changes]));
            changed += 1;
        }
    }
    Ok(changed)
}

fn enforce_users(jellyfin: &impl Jellyfin) -> Result<usize, String> {
    let users = jellyfin.get("/Users")?;
    let mut changed = 0;
    for user in users.as_array().ok_or("Jellyfin sent no list of users")? {
        let id = user["Id"].as_str().ok_or("a user has no Id")?;
        let policy = settle(
            jellyfin,
            &format!("/Users/{id}/Policy"),
            &user["Policy"],
            &wanted_policy(&user["Policy"]),
        )?;
        let configuration = settle(
            jellyfin,
            &format!("/Users/Configuration?userId={id}"),
            &user["Configuration"],
            &configuration(),
        )?;
        if !policy.is_empty() || !configuration.is_empty() {
            let name = user["Name"].as_str().unwrap_or("?");
            println!("{name}: set {}", keys(&[&policy, &configuration]));
            changed += 1;
        }
    }
    Ok(changed)
}

fn enforce(jellyfin: &impl Jellyfin) -> Result<usize, String> {
    Ok(enforce_server(jellyfin)? + enforce_users(jellyfin)?)
}

fn settings(vars: impl Fn(&str) -> Option<String>) -> Result<Server, String> {
    let read = |name: &str| {
        vars(name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let (Some(domain), Some(key)) = (read("DOMAIN"), read("JELLYFIN_API_KEY")) else {
        return Err("DOMAIN and JELLYFIN_API_KEY must be set in .env".into());
    };
    let port = read("CADDY_HTTPS_PORT").unwrap_or_else(|| "443".into());
    Ok(Server {
        base: format!("https://jellyfin.{domain}:{port}"),
        authorization: format!("MediaBrowser Token=\"{key}\""),
    })
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
    let server = settings(|name| env::var(name).ok()).unwrap_or_else(|error| {
        eprintln!("{error}");
        process::exit(1);
    });
    loop {
        match enforce(&server) {
            Ok(_) => {
                if let Err(error) = fs::write(HEARTBEAT, b"ok") {
                    eprintln!("Cannot write the heartbeat: {error}");
                }
            }
            Err(error) => eprintln!("Cannot apply the policy: {error}"),
        }
        thread::sleep(INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    fn user(name: &str, admin: bool, overrides: Value) -> Value {
        let mut policy = policy();
        policy.insert("IsAdministrator".into(), json!(admin));
        policy.insert("EnableContentDeletion".into(), json!(admin));
        policy.insert("EnableAllFolders".into(), json!(true));
        policy.extend(rules(overrides));
        let mut configuration = configuration();
        configuration.insert("SubtitleMode".into(), json!("Smart"));
        json!({"Id": format!("id-{name}"), "Name": name, "Policy": policy, "Configuration": configuration})
    }

    fn in_line() -> BTreeMap<String, Value> {
        let mut pages: BTreeMap<String, Value> = server()
            .into_iter()
            .map(|(_, path, wanted)| {
                let mut page = wanted;
                page.insert("Untouched".into(), json!("kept"));
                (path.to_string(), Value::Object(page))
            })
            .collect();
        pages.insert(
            "/Users".into(),
            json!([
                user("theo", false, json!({})),
                user("amaroke", true, json!({}))
            ]),
        );
        pages
    }

    struct Fake {
        pages: BTreeMap<String, Value>,
        posted: RefCell<Vec<(String, Value)>>,
    }

    impl Fake {
        fn new(pages: BTreeMap<String, Value>) -> Self {
            Fake {
                pages,
                posted: RefCell::new(vec![]),
            }
        }

        fn paths(&self) -> Vec<String> {
            self.posted
                .borrow()
                .iter()
                .map(|(path, _)| path.clone())
                .collect()
        }
    }

    impl Jellyfin for Fake {
        fn get(&self, path: &str) -> Result<Value, String> {
            self.pages
                .get(path)
                .cloned()
                .ok_or(format!("no page {path}"))
        }

        fn post(&self, path: &str, body: &Value) -> Result<(), String> {
            self.posted
                .borrow_mut()
                .push((path.to_string(), body.clone()));
            Ok(())
        }
    }

    #[test]
    fn a_friend_may_not_delete_content() {
        assert_eq!(
            Some(&json!(false)),
            wanted_policy(&json!({"IsAdministrator": false})).get("EnableContentDeletion")
        );
    }

    #[test]
    fn the_admin_keeps_the_right_to_delete() {
        assert!(
            !wanted_policy(&json!({"IsAdministrator": true})).contains_key("EnableContentDeletion")
        );
    }

    #[test]
    fn every_account_gets_one_session_and_three_attempts() {
        for admin in [true, false] {
            let wanted = wanted_policy(&json!({"IsAdministrator": admin}));
            assert_eq!(Some(&json!(1)), wanted.get("MaxActiveSessions"));
            assert_eq!(Some(&json!(3)), wanted.get("LoginAttemptsBeforeLockout"));
        }
    }

    #[test]
    fn a_server_in_line_needs_nothing() {
        let fake = Fake::new(in_line());
        assert_eq!(Ok(0), enforce(&fake));
        assert!(fake.paths().is_empty());
    }

    #[test]
    fn a_new_account_is_brought_in_line() {
        let mut pages = in_line();
        let mut fresh = user(
            "nima",
            false,
            json!({"EnableLiveTvAccess": true, "EnablePublicSharing": true, "MaxActiveSessions": 0, "LoginAttemptsBeforeLockout": -1, "EnableContentDeletion": true}),
        );
        fresh["Configuration"]["EnableNextEpisodeAutoPlay"] = json!(false);
        pages.insert("/Users".into(), json!([fresh]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&fake));
        assert_eq!(
            vec![
                "/Users/id-nima/Policy",
                "/Users/Configuration?userId=id-nima"
            ],
            fake.paths()
        );
        let (_, saved) = &fake.posted.borrow()[0];
        assert_eq!(json!(1), saved["MaxActiveSessions"]);
        assert_eq!(json!(false), saved["EnableContentDeletion"]);
        assert_eq!(json!(true), saved["EnableAllFolders"]);
    }

    #[test]
    fn a_server_setting_changed_by_hand_comes_back() {
        let mut pages = in_line();
        pages.get_mut("/System/Configuration/encoding").unwrap()["EnableThrottling"] = json!(false);
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&fake));
        let (path, saved) = &fake.posted.borrow()[0];
        assert_eq!("/System/Configuration/encoding", path);
        assert_eq!(json!(true), saved["EnableThrottling"]);
        assert_eq!(json!("kept"), saved["Untouched"]);
    }

    #[test]
    fn keys_are_listed_in_order() {
        let first = rules(json!({"b": 1, "a": 2}));
        let second = rules(json!({"c": 3}));
        assert_eq!("a, b, c", keys(&[&first, &second]));
    }

    #[test]
    fn settings_point_at_the_public_name_of_jellyfin() {
        let vars = |port: Option<&'static str>| {
            move |name: &str| match name {
                "DOMAIN" => Some("example.org".to_string()),
                "JELLYFIN_API_KEY" => Some("k".to_string()),
                "CADDY_HTTPS_PORT" => port.map(String::from),
                _ => None,
            }
        };
        let server = settings(vars(Some("50000"))).unwrap();
        assert_eq!("https://jellyfin.example.org:50000", server.base);
        assert_eq!("MediaBrowser Token=\"k\"", server.authorization);
        assert_eq!(
            "https://jellyfin.example.org:443",
            settings(vars(None)).unwrap().base
        );
    }

    #[test]
    fn settings_stop_without_a_key() {
        assert!(settings(|name| (name == "DOMAIN").then(|| "example.org".to_string())).is_err());
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
