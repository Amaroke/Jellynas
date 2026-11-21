use serde_json::{Map, Value, json};
use std::time::{Duration, SystemTime};
use std::{env, fs, process, thread};

const INTERVAL: Duration = Duration::from_secs(600);
const HEARTBEAT: &str = "/tmp/heartbeat";

fn policy() -> Map<String, Value> {
    let rules = json!({
        "EnableLiveTvAccess": false,
        "EnableLiveTvManagement": false,
        "EnablePublicSharing": false,
        "EnableMediaConversion": false,
        "EnableContentDownloading": true,
        "EnableRemoteControlOfOtherUsers": false,
        "MaxActiveSessions": 1,
        "LoginAttemptsBeforeLockout": 3,
        "SyncPlayAccess": "CreateAndJoinGroups",
    });
    rules.as_object().cloned().unwrap_or_default()
}

fn configuration() -> Map<String, Value> {
    let rules = json!({
        "EnableNextEpisodeAutoPlay": true,
    });
    rules.as_object().cloned().unwrap_or_default()
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

struct Change {
    user: Value,
    policy: Map<String, Value>,
    configuration: Map<String, Value>,
}

fn plan(users: &[Value]) -> Vec<Change> {
    users
        .iter()
        .filter_map(|user| {
            let policy = drift(&user["Policy"], &wanted_policy(&user["Policy"]));
            let configuration = drift(&user["Configuration"], &configuration());
            (!policy.is_empty() || !configuration.is_empty()).then(|| Change {
                user: user.clone(),
                policy,
                configuration,
            })
        })
        .collect()
}

fn merged(current: &Value, changes: &Map<String, Value>) -> Value {
    let mut whole = current.as_object().cloned().unwrap_or_default();
    whole.extend(changes.clone());
    Value::Object(whole)
}

fn changed_keys(change: &Change) -> String {
    let mut keys: Vec<&str> = change
        .policy
        .keys()
        .chain(change.configuration.keys())
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys.join(", ")
}

trait Jellyfin {
    fn users(&self) -> Result<Vec<Value>, String>;
    fn save(&self, change: &Change) -> Result<(), String>;
}

struct Server {
    base: String,
    authorization: String,
}

impl Server {
    fn post(&self, path: &str, body: &Value) -> Result<(), String> {
        ureq::post(&format!("{}{path}", self.base))
            .set("Authorization", &self.authorization)
            .timeout(Duration::from_secs(30))
            .send_json(body)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

impl Jellyfin for Server {
    fn users(&self) -> Result<Vec<Value>, String> {
        ureq::get(&format!("{}/Users", self.base))
            .set("Authorization", &self.authorization)
            .timeout(Duration::from_secs(30))
            .call()
            .map_err(|error| error.to_string())?
            .into_json()
            .map_err(|error| error.to_string())
    }

    fn save(&self, change: &Change) -> Result<(), String> {
        let id = change.user["Id"].as_str().ok_or("a user has no Id")?;
        if !change.policy.is_empty() {
            self.post(
                &format!("/Users/{id}/Policy"),
                &merged(&change.user["Policy"], &change.policy),
            )?;
        }
        if !change.configuration.is_empty() {
            self.post(
                &format!("/Users/Configuration?userId={id}"),
                &merged(&change.user["Configuration"], &change.configuration),
            )?;
        }
        Ok(())
    }
}

fn enforce(jellyfin: &impl Jellyfin) -> Result<usize, String> {
    let changes = plan(&jellyfin.users()?);
    for change in &changes {
        jellyfin.save(change)?;
        let name = change.user["Name"].as_str().unwrap_or("?");
        println!("{name}: set {}", changed_keys(change));
    }
    Ok(changes.len())
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

    fn user(name: &str, admin: bool, overrides: Value) -> Value {
        let mut policy = policy();
        policy.insert("IsAdministrator".into(), json!(admin));
        policy.insert("EnableContentDeletion".into(), json!(admin));
        policy.insert("EnableAllFolders".into(), json!(true));
        policy.extend(overrides.as_object().cloned().unwrap_or_default());
        let mut configuration = configuration();
        configuration.insert("SubtitleMode".into(), json!("Smart"));
        json!({"Id": format!("id-{name}"), "Name": name, "Policy": policy, "Configuration": configuration})
    }

    struct Fake {
        users: Vec<Value>,
        saved: RefCell<Vec<String>>,
    }

    impl Jellyfin for Fake {
        fn users(&self) -> Result<Vec<Value>, String> {
            Ok(self.users.clone())
        }

        fn save(&self, change: &Change) -> Result<(), String> {
            let name = change.user["Name"].as_str().unwrap_or_default();
            self.saved
                .borrow_mut()
                .push(format!("{name}: {}", changed_keys(change)));
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
    fn an_account_in_line_needs_nothing() {
        assert!(
            plan(&[
                user("theo", false, json!({})),
                user("amaroke", true, json!({}))
            ])
            .is_empty()
        );
    }

    #[test]
    fn a_new_account_is_brought_in_line() {
        let mut fresh = user(
            "nima",
            false,
            json!({"EnableLiveTvAccess": true, "EnablePublicSharing": true, "MaxActiveSessions": 0, "LoginAttemptsBeforeLockout": -1, "EnableContentDeletion": true}),
        );
        fresh["Configuration"]["EnableNextEpisodeAutoPlay"] = json!(false);
        let changes = plan(&[fresh]);
        assert_eq!(1, changes.len());
        assert_eq!(
            "EnableContentDeletion, EnableLiveTvAccess, EnableNextEpisodeAutoPlay, EnablePublicSharing, LoginAttemptsBeforeLockout, MaxActiveSessions",
            changed_keys(&changes[0])
        );
    }

    #[test]
    fn a_save_keeps_every_other_field() {
        let current =
            json!({"IsAdministrator": false, "EnableAllFolders": true, "MaxActiveSessions": 0});
        let changes = drift(&current, &wanted_policy(&current));
        let whole = merged(&current, &changes);
        assert_eq!(json!(true), whole["EnableAllFolders"]);
        assert_eq!(json!(1), whole["MaxActiveSessions"]);
    }

    #[test]
    fn enforce_saves_only_the_accounts_that_drifted() {
        let fake = Fake {
            users: vec![
                user("theo", false, json!({})),
                user("yann", false, json!({"EnableMediaConversion": true})),
            ],
            saved: RefCell::new(vec![]),
        };
        assert_eq!(Ok(1), enforce(&fake));
        assert_eq!(
            vec!["yann: EnableMediaConversion".to_string()],
            *fake.saved.borrow()
        );
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
