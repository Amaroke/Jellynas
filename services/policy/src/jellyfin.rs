use crate::accounts::{Account, PRIVATE_ROOT, owner};
use crate::api::{Api, encode, report, rules, settle};
use jiff::{SignedDuration, Timestamp};
use serde_json::{Map, Value, json};

const TEST_ACCOUNT: &str = "test";
const IDLE: SignedDuration = SignedDuration::from_hours(30 * 24);
const LDAP_PLUGIN: &str = "/Plugins/958aad6637844d2ab89aa7b6fab6e25c/Configuration";
const INJECTOR_PLUGIN: &str = "/Plugins/f5a34f7b2e8a4e6aa7223a216a81b374/Configuration";
const REQUEST_BUTTON: &str = r#"(function(){var css='.headerRequestButtons{position:absolute;left:50%;top:50%;transform:translate(-50%,-50%);z-index:1;display:flex;gap:8px}.headerRequestButtons a{display:inline-flex;align-items:center;gap:6px;height:34px;padding:0 14px 0 10px;border-radius:8px;background:rgba(255,255,255,.08);color:inherit;font:inherit;font-size:.9em;font-weight:500;white-space:nowrap;text-decoration:none;transition:background .15s}.headerRequestButtons a:hover{background:rgba(255,255,255,.16)}.headerRequestButtons svg{width:18px;height:18px;opacity:.85}';var links=[['seerr','Faire une requête','M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z'],['aurral','Musique','M12 3v10.55A4 4 0 1 0 14 17V7h4V3h-6z'],['kavita','Livres','M18 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zM6 4h5v8l-2.5-1.5L6 12V4z']];function style(){if(document.getElementById('headerRequestStyle'))return;var tag=document.createElement('style');tag.id='headerRequestStyle';tag.textContent=css;document.head.appendChild(tag);}function bar(){var anchor=document.querySelector('[aria-controls="app-user-menu"]');if(!anchor||!anchor.offsetParent)anchor=document.querySelector('.headerRight');if(!anchor||!anchor.offsetParent)return null;return anchor.closest('.MuiToolbar-root,.headerTop,header')||anchor.parentNode;}function add(){var node=bar();if(!node)return;var old=document.querySelector('.headerRequestButtons');if(old&&old.parentNode===node)return;if(old)old.remove();style();if(getComputedStyle(node).position==='static')node.style.position='relative';var group=document.createElement('div');group.className='headerRequestButtons';links.forEach(function(l){var link=document.createElement('a');link.href='https://'+location.host.replace(/^jellyfin\./,l[0]+'.');link.target='_blank';link.rel='noopener';link.innerHTML='<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="'+l[2]+'"/></svg><span>'+l[1]+'</span>';group.appendChild(link);});node.appendChild(group);}new MutationObserver(add).observe(document.body,{childList:true,subtree:true});add();})();"#;
const THEME: &str = "@import url(\"https://cdn.jsdelivr.net/gh/lscambo13/ElegantFin@v26.09.05/Theme/ElegantFin-theme-v26.09.05.css\");\n:root { --extraCardButtonsVisibility: block; }\n";

#[derive(Debug, Default, PartialEq)]
pub struct Shelves {
    pub public: Vec<String>,
    pub private: Vec<(String, String)>,
}

impl Shelves {
    pub fn read(folders: &Value) -> Self {
        let mut shelves = Shelves::default();
        for folder in folders.as_array().into_iter().flatten() {
            let Some(id) = folder["ItemId"].as_str() else {
                continue;
            };
            let locations: Vec<&str> = folder["Locations"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            if !locations.iter().any(|path| path.starts_with(PRIVATE_ROOT)) {
                shelves.public.push(id.to_string());
                continue;
            }
            for owner in locations.iter().filter_map(|path| owner(path)) {
                shelves.private.push((owner.to_string(), id.to_string()));
            }
        }
        shelves.public.sort_unstable();
        shelves
    }

    pub fn of(&self, user: &str) -> Vec<String> {
        let user = user.to_lowercase();
        let mut ids: Vec<String> = self.public.clone();
        ids.extend(
            self.private
                .iter()
                .filter(|(owner, _)| *owner == user)
                .map(|(_, id)| id.clone()),
        );
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

fn kept_order(current: &Value, wanted: &[String]) -> Value {
    let mut have: Vec<&str> = current
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    have.sort_unstable();
    if have == wanted {
        current.clone()
    } else {
        json!(wanted)
    }
}

fn access(current: &Value, wanted: &[String]) -> Map<String, Value> {
    rules(json!({
        "EnableAllFolders": false,
        "EnabledFolders": kept_order(&current["EnabledFolders"], wanted),
    }))
}

fn library_name(account: &Account, folders: &Value) -> String {
    let taken = folders
        .as_array()
        .into_iter()
        .flatten()
        .any(|folder| folder["Name"].as_str() == Some(account.name.as_str()));
    if taken {
        format!("{} ({})", account.name, account.id)
    } else {
        account.name.clone()
    }
}

fn located(folders: &Value, path: &str) -> bool {
    folders
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|folder| folder["Locations"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .any(|location| location.trim_end_matches('/') == path)
}

fn shelve(api: &impl Api, accounts: &[Account]) -> Result<usize, String> {
    let folders = api.get("/Library/VirtualFolders")?;
    let mut created = 0;
    for account in accounts {
        if located(&folders, &account.videos()) {
            continue;
        }
        let name = library_name(account, &folders);
        let options = json!({"LibraryOptions": {
            "PathInfos": [{"Path": account.videos()}],
            "PreferredMetadataLanguage": "fr",
            "MetadataCountryCode": "FR",
            "EnableRealtimeMonitor": true,
        }});
        api.send(
            "POST",
            &format!(
                "/Library/VirtualFolders?name={}&collectionType=mixed&refreshLibrary=true",
                encode(&name)
            ),
            &options,
        )?;
        println!("Jellyfin libraries: added {name} for {}", account.id);
        created += 1;
    }
    Ok(created)
}

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
        (
            "branding",
            "/System/Configuration/branding",
            rules(json!({
                "CustomCss": THEME,
            })),
        ),
        (
            "scripts",
            INJECTOR_PLUGIN,
            rules(json!({
                "CustomJavaScripts": [{
                    "Name": "Request button",
                    "Script": REQUEST_BUTTON,
                    "Src": "",
                    "Module": false,
                    "Enabled": true,
                    "RequiresAuthentication": true,
                }],
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
        "MaxActiveSessions": 3,
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

pub fn wanted_policy(user: &Value, now: Timestamp, shelves: &Shelves) -> Map<String, Value> {
    let mut wanted = policy();
    let name = user["Name"].as_str().unwrap_or_default();
    wanted.extend(access(&user["Policy"], &shelves.of(name)));
    if user["Policy"]["IsAdministrator"] != json!(true) {
        wanted.insert("EnableContentDeletion".into(), json!(false));
        if idle(user, now) || name.eq_ignore_ascii_case(TEST_ACCOUNT) {
            wanted.insert("IsDisabled".into(), json!(true));
        }
    }
    wanted
}

pub fn enforce(api: &impl Api, accounts: &[Account]) -> Result<usize, String> {
    pass(api, Timestamp::now(), accounts)
}

fn pass(api: &impl Api, now: Timestamp, accounts: &[Account]) -> Result<usize, String> {
    let mut changed = 0;
    for (page, path, wanted) in server() {
        let changes = settle(api, "POST", path, &api.get(path)?, &wanted)?;
        changed += report("Jellyfin", page, &[&changes]);
    }
    changed += shelve(api, accounts)?;
    let shelves = Shelves::read(&api.get("/Library/VirtualFolders")?);
    let plugin = api.get(LDAP_PLUGIN)?;
    let defaults = settle(
        api,
        "POST",
        LDAP_PLUGIN,
        &plugin,
        &access(&plugin, &shelves.public),
    )?;
    changed += report("Jellyfin", "new Accounts", &[&defaults]);
    let users = api.get("/Users")?;
    for user in users.as_array().ok_or("Jellyfin sent no list of users")? {
        let id = user["Id"].as_str().ok_or("a user of Jellyfin has no Id")?;
        let policy = settle(
            api,
            "POST",
            &format!("/Users/{id}/Policy"),
            &user["Policy"],
            &wanted_policy(user, now, &shelves),
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

    const MOVIES: &str = "aa";
    const ELEA: &str = "ee";
    const VINCENT: &str = "vv";

    fn library(id: &str, name: &str, path: &str) -> Value {
        json!({"ItemId": id, "Name": name, "Locations": [path]})
    }

    fn libraries() -> Value {
        json!([
            library(MOVIES, "Films", "/data/media/movies"),
            library(ELEA, "Eléa", "/data/media/private/elea_t/videos"),
            library(VINCENT, "Vincent", "/data/media/private/moutondu/videos/"),
        ])
    }

    fn user(name: &str, admin: bool, overrides: Value) -> Value {
        let mut policy = policy();
        policy.insert("IsAdministrator".into(), json!(admin));
        policy.insert("EnableContentDeletion".into(), json!(admin));
        policy.extend(access(&json!({}), &Shelves::read(&libraries()).of(name)));
        policy.extend(rules(overrides));
        let mut configuration = configuration();
        configuration.insert("SubtitleMode".into(), json!("Smart"));
        json!({"Id": format!("id-{name}"), "Name": name, "LastActivityDate": "2026-11-20T21:14:03.1234567Z", "Policy": policy, "Configuration": configuration})
    }

    fn account(id: &str, name: &str) -> Account {
        Account {
            id: id.into(),
            name: name.into(),
        }
    }

    fn now() -> Timestamp {
        "2026-11-23T03:30:00Z".parse().unwrap()
    }

    fn seen(admin: bool, date: &str) -> Value {
        json!({"LastActivityDate": date, "Policy": {"IsAdministrator": admin}})
    }

    fn friend() -> Value {
        json!({"Policy": {"IsAdministrator": false}})
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
        pages.insert("/Library/VirtualFolders".into(), libraries());
        pages.insert(
            LDAP_PLUGIN.into(),
            json!({"EnableAllFolders": false, "EnabledFolders": [MOVIES], "LdapServer": "lldap"}),
        );
        pages.insert(
            "/Users".into(),
            json!([
                user("theo", false, json!({})),
                user("elea_t", false, json!({})),
                user("amaroke", true, json!({}))
            ]),
        );
        pages
    }

    #[test]
    fn a_friend_may_not_delete_content() {
        let wanted = wanted_policy(&friend(), now(), &Shelves::default());
        assert_eq!(Some(&json!(false)), wanted.get("EnableContentDeletion"));
    }

    #[test]
    fn the_admin_keeps_the_right_to_delete() {
        let admin = json!({"Policy": {"IsAdministrator": true}});
        let wanted = wanted_policy(&admin, now(), &Shelves::default());
        assert!(!wanted.contains_key("EnableContentDeletion"));
    }

    #[test]
    fn every_account_gets_three_sessions_and_three_attempts() {
        for admin in [true, false] {
            let user = json!({"Policy": {"IsAdministrator": admin}});
            let wanted = wanted_policy(&user, now(), &Shelves::default());
            assert_eq!(Some(&json!(3)), wanted.get("MaxActiveSessions"));
            assert_eq!(Some(&json!(3)), wanted.get("LoginAttemptsBeforeLockout"));
        }
    }

    #[test]
    fn a_friend_idle_for_thirty_days_is_disabled() {
        let user = seen(false, "2026-10-24T03:30:00Z");
        let wanted = wanted_policy(&user, now(), &Shelves::default());
        assert_eq!(Some(&json!(true)), wanted.get("IsDisabled"));
    }

    #[test]
    fn a_friend_seen_within_thirty_days_is_left_alone() {
        let user = seen(false, "2026-10-24T03:30:01Z");
        let wanted = wanted_policy(&user, now(), &Shelves::default());
        assert!(!wanted.contains_key("IsDisabled"));
    }

    #[test]
    fn the_test_account_is_disabled_at_every_pass() {
        let user = json!({"Name": "test", "LastActivityDate": "2026-11-23T03:00:00Z", "Policy": {"IsAdministrator": false}});
        let wanted = wanted_policy(&user, now(), &Shelves::default());
        assert_eq!(Some(&json!(true)), wanted.get("IsDisabled"));
    }

    #[test]
    fn the_admin_is_never_disabled() {
        let user = seen(true, "2020-01-01T00:00:00Z");
        let wanted = wanted_policy(&user, now(), &Shelves::default());
        assert!(!wanted.contains_key("IsDisabled"));
    }

    #[test]
    fn an_account_without_activity_is_left_alone() {
        let wanted = wanted_policy(&friend(), now(), &Shelves::default());
        assert!(!wanted.contains_key("IsDisabled"));
    }

    #[test]
    fn a_private_library_belongs_to_the_account_named_by_its_folder() {
        let shelves = Shelves::read(&libraries());
        assert_eq!(vec![MOVIES.to_string()], shelves.public);
        assert_eq!(vec![MOVIES, ELEA], shelves.of("elea_t"));
        assert_eq!(vec![MOVIES, VINCENT], shelves.of("Moutondu"));
        assert_eq!(vec![MOVIES], shelves.of("theo"));
    }

    #[test]
    fn a_library_on_a_private_subfolder_belongs_to_the_account_of_that_folder() {
        let shelves = Shelves::read(&json!([
            library("aa", "Films", "/data/media/movies"),
            library("bb", "Livres", "/data/media/private/elea_t/books/"),
        ]));
        assert_eq!(vec!["aa", "bb"], shelves.of("elea_t"));
        assert_eq!(vec!["aa"], shelves.of("theo"));
    }

    #[test]
    fn a_books_library_does_not_stand_for_the_videos_library() {
        let mut pages = in_line();
        pages.insert(
            "/Library/VirtualFolders".into(),
            json!([library("bb", "Livres", "/data/media/private/theo/books")]),
        );
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), shelve(&fake, &[account("theo", "Théo")]));
        assert_eq!(
            json!("/data/media/private/theo/videos"),
            fake.body(0)["LibraryOptions"]["PathInfos"][0]["Path"]
        );
    }

    #[test]
    fn a_library_on_the_whole_private_folder_does_not_stand_for_the_videos_library() {
        let mut pages = in_line();
        pages.insert(
            "/Library/VirtualFolders".into(),
            json!([library("tt", "Théo", "/data/media/private/theo")]),
        );
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), shelve(&fake, &[account("theo", "Théo")]));
        assert_eq!(
            vec![
                "POST /Library/VirtualFolders?name=Th%C3%A9o%20%28theo%29&collectionType=mixed&refreshLibrary=true"
            ],
            fake.paths()
        );
    }

    #[test]
    fn a_library_at_the_private_root_itself_belongs_to_nobody() {
        let shelves = Shelves::read(&json!([library("zz", "All", "/data/media/private")]));
        assert!(shelves.public.is_empty());
        assert!(shelves.private.is_empty());
    }

    #[test]
    fn the_admin_sees_only_their_own_private_library_too() {
        let mut pages = in_line();
        let mut admin = user("amaroke", true, json!({}));
        admin["Policy"]["EnableAllFolders"] = json!(true);
        pages.insert("/Users".into(), json!([admin]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), pass(&fake, now(), &[]));
        assert_eq!(json!(false), fake.body(0)["EnableAllFolders"]);
        assert_eq!(json!([MOVIES]), fake.body(0)["EnabledFolders"]);
    }

    #[test]
    fn a_friend_granted_another_private_library_loses_it() {
        let mut pages = in_line();
        let theo = user("theo", false, json!({"EnabledFolders": [MOVIES, ELEA]}));
        pages.insert("/Users".into(), json!([theo]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), pass(&fake, now(), &[]));
        assert_eq!(json!([MOVIES]), fake.body(0)["EnabledFolders"]);
    }

    #[test]
    fn the_order_of_the_libraries_of_an_account_is_kept() {
        let mut pages = in_line();
        let elea = user("elea_t", false, json!({"EnabledFolders": [ELEA, MOVIES]}));
        pages.insert("/Users".into(), json!([elea]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(0), pass(&fake, now(), &[]));
    }

    #[test]
    fn a_new_account_gets_the_public_libraries_only() {
        let mut pages = in_line();
        pages.insert(
            LDAP_PLUGIN.into(),
            json!({"EnableAllFolders": true, "EnabledFolders": [ELEA], "LdapServer": "lldap"}),
        );
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), pass(&fake, now(), &[]));
        assert_eq!(vec![format!("POST {LDAP_PLUGIN}")], fake.paths());
        assert_eq!(
            json!({"EnableAllFolders": false, "EnabledFolders": [MOVIES], "LdapServer": "lldap"}),
            fake.body(0)
        );
    }

    #[test]
    fn an_account_without_a_private_library_gets_one() {
        let accounts = [
            account("elea_t", "Eléa"),
            account("theo", "Théo B"),
            account("niixe", "Films"),
        ];
        let fake = Fake::new(in_line());
        assert_eq!(Ok(2), pass(&fake, now(), &accounts));
        assert_eq!(
            vec![
                "POST /Library/VirtualFolders?name=Th%C3%A9o%20B&collectionType=mixed&refreshLibrary=true",
                "POST /Library/VirtualFolders?name=Films%20%28niixe%29&collectionType=mixed&refreshLibrary=true",
            ],
            fake.paths()
        );
        assert_eq!(
            json!("/data/media/private/theo/videos"),
            fake.body(0)["LibraryOptions"]["PathInfos"][0]["Path"]
        );
    }

    #[test]
    fn an_idle_friend_is_disabled_at_the_pass() {
        let mut pages = in_line();
        let mut gone = user("theo", false, json!({}));
        gone["LastActivityDate"] = json!("2026-09-01T18:00:00.0000000Z");
        pages.insert("/Users".into(), json!([gone]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), pass(&fake, now(), &[]));
        assert_eq!(vec!["POST /Users/id-theo/Policy"], fake.paths());
        assert_eq!(json!(true), fake.body(0)["IsDisabled"]);
        assert_eq!(json!(false), fake.body(0)["EnableAllFolders"]);
    }

    #[test]
    fn a_server_in_line_needs_nothing() {
        let fake = Fake::new(in_line());
        assert_eq!(Ok(0), pass(&fake, now(), &[account("elea_t", "Eléa")]));
        assert!(fake.paths().is_empty());
    }

    #[test]
    fn a_new_account_is_brought_in_line() {
        let mut pages = in_line();
        let mut fresh = user(
            "nima",
            false,
            json!({"EnableLiveTvAccess": true, "MaxActiveSessions": 0, "LoginAttemptsBeforeLockout": -1, "EnableContentDeletion": true, "EnableAllFolders": true}),
        );
        fresh["Configuration"]["EnableNextEpisodeAutoPlay"] = json!(false);
        pages.insert("/Users".into(), json!([fresh]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), pass(&fake, now(), &[]));
        assert_eq!(
            vec![
                "POST /Users/id-nima/Policy",
                "POST /Users/Configuration?userId=id-nima"
            ],
            fake.paths()
        );
        let saved = fake.body(0);
        assert_eq!(json!(3), saved["MaxActiveSessions"]);
        assert_eq!(json!(false), saved["EnableContentDeletion"]);
        assert_eq!(json!(false), saved["EnableAllFolders"]);
    }

    #[test]
    fn a_server_setting_changed_by_hand_comes_back() {
        let mut pages = in_line();
        pages.get_mut("/System/Configuration/encoding").unwrap()["EnableThrottling"] = json!(false);
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), pass(&fake, now(), &[]));
        assert_eq!(vec!["POST /System/Configuration/encoding"], fake.paths());
        assert_eq!(json!(true), fake.body(0)["EnableThrottling"]);
        assert_eq!(json!("kept"), fake.body(0)["Untouched"]);
    }

    #[test]
    fn a_theme_removed_by_hand_comes_back() {
        let mut pages = in_line();
        pages.get_mut("/System/Configuration/branding").unwrap()["CustomCss"] = json!("");
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), pass(&fake, now(), &[]));
        assert_eq!(vec!["POST /System/Configuration/branding"], fake.paths());
        assert_eq!(json!(THEME), fake.body(0)["CustomCss"]);
    }

    #[test]
    fn a_request_button_removed_by_hand_comes_back() {
        let mut pages = in_line();
        pages.get_mut(INJECTOR_PLUGIN).unwrap()["CustomJavaScripts"] = json!([]);
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), pass(&fake, now(), &[]));
        assert_eq!(vec![format!("POST {INJECTOR_PLUGIN}")], fake.paths());
        assert_eq!(
            json!(REQUEST_BUTTON),
            fake.body(0)["CustomJavaScripts"][0]["Script"]
        );
    }
}
