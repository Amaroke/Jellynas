use crate::api::{Api, report, rules, settle};
use jiff::{SignedDuration, Timestamp};
use serde_json::{Map, Value, json};

const IDLE: SignedDuration = SignedDuration::from_hours(30 * 24);

pub fn server() -> Vec<(&'static str, &'static str, Map<String, Value>)> {
    vec![
        (
            "general",
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
            "playback",
            "/System/Configuration/encoding",
            rules(json!({
                "HardwareAccelerationType": "none",
                "EnableThrottling": true,
                "EnableSegmentDeletion": true,
                "EnableTonemapping": false,
            })),
        ),
        (
            "networking",
            "/System/Configuration/network",
            rules(json!({
                "KnownProxies": ["caddy"],
                "EnableRemoteAccess": true,
                "EnableUPnP": false,
            })),
        ),
    ]
}

pub fn policy() -> Map<String, Value> {
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

pub fn configuration() -> Map<String, Value> {
    rules(json!({
        "EnableNextEpisodeAutoPlay": true,
    }))
}

pub fn idle(user: &Value, now: Timestamp) -> bool {
    user["LastActivityDate"]
        .as_str()
        .and_then(|date| date.parse::<Timestamp>().ok())
        .is_some_and(|last| now.duration_since(last) >= IDLE)
}

pub fn wanted_policy(user: &Value, now: Timestamp) -> Map<String, Value> {
    let mut wanted = policy();
    if user["Policy"]["IsAdministrator"] != json!(true) {
        wanted.insert("EnableContentDeletion".into(), json!(false));
        if idle(user, now) {
            wanted.insert("IsDisabled".into(), json!(true));
        }
    }
    wanted
}

pub fn enforce(api: &impl Api) -> Result<usize, String> {
    pass(api, Timestamp::now())
}

fn pass(api: &impl Api, now: Timestamp) -> Result<usize, String> {
    let mut changed = 0;
    for (page, path, wanted) in server() {
        let changes = settle(api, "POST", path, &api.get(path)?, &wanted)?;
        changed += report("Jellyfin", page, &[&changes]);
    }
    let users = api.get("/Users")?;
    for user in users.as_array().ok_or("Jellyfin sent no list of users")? {
        let id = user["Id"].as_str().ok_or("a user of Jellyfin has no Id")?;
        let policy = settle(
            api,
            "POST",
            &format!("/Users/{id}/Policy"),
            &user["Policy"],
            &wanted_policy(user, now),
        )?;
        let configuration = settle(
            api,
            "POST",
            &format!("/Users/Configuration?userId={id}"),
            &user["Configuration"],
            &configuration(),
        )?;
        let name = user["Name"].as_str().unwrap_or("?");
        changed += report("Jellyfin", name, &[&policy, &configuration]);
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::fake::Fake;
    use std::collections::BTreeMap;

    fn user(name: &str, admin: bool, overrides: Value) -> Value {
        let mut policy = policy();
        policy.insert("IsAdministrator".into(), json!(admin));
        policy.insert("EnableContentDeletion".into(), json!(admin));
        policy.insert("EnableAllFolders".into(), json!(true));
        policy.extend(rules(overrides));
        let mut configuration = configuration();
        configuration.insert("SubtitleMode".into(), json!("Smart"));
        json!({"Id": format!("id-{name}"), "Name": name, "LastActivityDate": "2026-11-20T21:14:03.1234567Z", "Policy": policy, "Configuration": configuration})
    }

    fn now() -> Timestamp {
        "2026-11-23T03:30:00Z".parse().unwrap()
    }

    fn seen(admin: bool, date: &str) -> Value {
        json!({"LastActivityDate": date, "Policy": {"IsAdministrator": admin}})
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

    #[test]
    fn a_friend_may_not_delete_content() {
        let wanted = wanted_policy(&json!({"Policy": {"IsAdministrator": false}}), now());
        assert_eq!(Some(&json!(false)), wanted.get("EnableContentDeletion"));
    }

    #[test]
    fn the_admin_keeps_the_right_to_delete() {
        let wanted = wanted_policy(&json!({"Policy": {"IsAdministrator": true}}), now());
        assert!(!wanted.contains_key("EnableContentDeletion"));
    }

    #[test]
    fn every_account_gets_one_session_and_three_attempts() {
        for admin in [true, false] {
            let wanted = wanted_policy(&json!({"Policy": {"IsAdministrator": admin}}), now());
            assert_eq!(Some(&json!(1)), wanted.get("MaxActiveSessions"));
            assert_eq!(Some(&json!(3)), wanted.get("LoginAttemptsBeforeLockout"));
        }
    }

    #[test]
    fn a_friend_idle_for_thirty_days_is_disabled() {
        let wanted = wanted_policy(&seen(false, "2026-10-24T03:30:00Z"), now());
        assert_eq!(Some(&json!(true)), wanted.get("IsDisabled"));
    }

    #[test]
    fn a_friend_seen_within_thirty_days_is_left_alone() {
        let wanted = wanted_policy(&seen(false, "2026-10-24T03:30:01Z"), now());
        assert!(!wanted.contains_key("IsDisabled"));
    }

    #[test]
    fn the_admin_is_never_disabled() {
        let wanted = wanted_policy(&seen(true, "2020-01-01T00:00:00Z"), now());
        assert!(!wanted.contains_key("IsDisabled"));
    }

    #[test]
    fn an_account_without_activity_is_left_alone() {
        let wanted = wanted_policy(&json!({"Policy": {"IsAdministrator": false}}), now());
        assert!(!wanted.contains_key("IsDisabled"));
    }

    #[test]
    fn an_idle_friend_is_disabled_at_the_pass() {
        let mut pages = in_line();
        let mut gone = user("theo", false, json!({}));
        gone["LastActivityDate"] = json!("2026-09-01T18:00:00.0000000Z");
        pages.insert("/Users".into(), json!([gone]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), pass(&fake, now()));
        assert_eq!(vec!["POST /Users/id-theo/Policy"], fake.paths());
        assert_eq!(json!(true), fake.body(0)["IsDisabled"]);
        assert_eq!(json!(true), fake.body(0)["EnableAllFolders"]);
    }

    #[test]
    fn a_server_in_line_needs_nothing() {
        let fake = Fake::new(in_line());
        assert_eq!(Ok(0), pass(&fake, now()));
        assert!(fake.paths().is_empty());
    }

    #[test]
    fn a_new_account_is_brought_in_line() {
        let mut pages = in_line();
        let mut fresh = user(
            "nima",
            false,
            json!({"EnableLiveTvAccess": true, "MaxActiveSessions": 0, "LoginAttemptsBeforeLockout": -1, "EnableContentDeletion": true}),
        );
        fresh["Configuration"]["EnableNextEpisodeAutoPlay"] = json!(false);
        pages.insert("/Users".into(), json!([fresh]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), pass(&fake, now()));
        assert_eq!(
            vec![
                "POST /Users/id-nima/Policy",
                "POST /Users/Configuration?userId=id-nima"
            ],
            fake.paths()
        );
        let saved = fake.body(0);
        assert_eq!(json!(1), saved["MaxActiveSessions"]);
        assert_eq!(json!(false), saved["EnableContentDeletion"]);
        assert_eq!(json!(true), saved["EnableAllFolders"]);
    }

    #[test]
    fn a_server_setting_changed_by_hand_comes_back() {
        let mut pages = in_line();
        pages.get_mut("/System/Configuration/encoding").unwrap()["EnableThrottling"] = json!(false);
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), pass(&fake, now()));
        assert_eq!(vec!["POST /System/Configuration/encoding"], fake.paths());
        assert_eq!(json!(true), fake.body(0)["EnableThrottling"]);
        assert_eq!(json!("kept"), fake.body(0)["Untouched"]);
    }
}
