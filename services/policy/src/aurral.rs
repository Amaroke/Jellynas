use crate::api::{Api, drift, merged, report, rules};
use serde_json::{Map, Value, json};

fn sections() -> Vec<(&'static str, Option<&'static str>, Map<String, Value>)> {
    vec![
        (
            "integrations",
            Some("slskd"),
            rules(json!({
                "enabled": true,
                "url": "http://gluetun:5030",
                "priority": 10,
            })),
        ),
        (
            "integrations",
            Some("ytdlp"),
            rules(json!({
                "enabled": true,
                "priority": 1000,
            })),
        ),
        (
            "qualityProfile",
            None,
            rules(json!({
                "cutoff": "flac-standard",
                "automaticUpgrades": true,
            })),
        ),
        (
            "security",
            None,
            rules(json!({
                "defaultPermissions": permissions(),
            })),
        ),
    ]
}

pub fn permissions() -> Value {
    json!({
        "accessPlaylists": true,
        "addArtist": true,
        "addAlbum": true,
        "changeMonitoring": false,
        "deleteArtist": false,
        "deleteAlbum": false,
        "deleteTrack": false,
    })
}

fn section<'a>(settings: &'a Value, top: &str, inner: Option<&str>) -> &'a Value {
    let outer = &settings[top];
    match inner {
        Some(name) => &outer[name],
        None => outer,
    }
}

fn patch(top: &str, inner: Option<&str>, changes: &Map<String, Value>) -> Value {
    let body = Value::Object(changes.clone());
    match inner {
        Some(name) => json!({ top: { name: body } }),
        None => json!({ top: body }),
    }
}

fn settings(api: &impl Api) -> Result<usize, String> {
    let current = api.get("/settings")?;
    let mut fixed = 0;
    for (top, inner, wanted) in sections() {
        let changes = drift(section(&current, top, inner), &wanted);
        if changes.is_empty() {
            continue;
        }
        api.send("POST", "/settings", &patch(top, inner, &changes))?;
        let page = inner.map_or(top.to_string(), |name| format!("{top}.{name}"));
        fixed += report("Aurral", &page, &[&changes]);
    }
    Ok(fixed)
}

fn following(api: &impl Api) -> Result<usize, String> {
    let wanted = rules(json!({
        "catalog": "none",
        "newReleases": true,
        "topTracks": 0,
    }));
    let current = &api.get("/library/monitoring/defaults")?["rules"];
    let changes = drift(current, &wanted);
    if changes.is_empty() {
        return Ok(0);
    }
    let body = json!({ "rules": merged(current, &changes) });
    api.send("PUT", "/library/monitoring/defaults", &body)?;
    Ok(report("Aurral", "monitoring defaults", &[&changes]))
}

fn users(api: &impl Api) -> Result<usize, String> {
    let wanted = rules(permissions());
    let mut fixed = 0;
    for user in api.get("/users")?.as_array().cloned().unwrap_or_default() {
        if user["role"] == "admin" {
            continue;
        }
        let id = user["id"].as_i64().ok_or("a user has no id")?;
        let changes = drift(&user["permissions"], &wanted);
        if changes.is_empty() {
            continue;
        }
        let body = json!({ "permissions": merged(&user["permissions"], &changes) });
        api.send("PATCH", &format!("/users/{id}"), &body)?;
        let name = user["username"].as_str().unwrap_or("?");
        fixed += report("Aurral", &format!("user {name}"), &[&changes]);
    }
    Ok(fixed)
}

fn toggle(api: &impl Api, playlist: &Value, field: &str, path: &str) -> Result<bool, String> {
    if playlist[field] == true {
        return Ok(false);
    }
    let id = playlist["id"].as_str().ok_or("a playlist has no id")?;
    api.send(
        "PUT",
        &format!("/playlists/static-playlists/{id}/{path}"),
        &json!({ "enabled": true }),
    )?;
    Ok(true)
}

fn playlists(api: &impl Api) -> Result<usize, String> {
    let wanted = rules(json!({
        "syncEnabled": false,
        "syncIntervalHours": 0,
        "keepRemovedTracks": true,
    }));
    let status = api.get("/playlists/status")?;
    let mut fixed = 0;
    for playlist in status["staticPlaylists"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        let id = playlist["id"].as_str().ok_or("a playlist has no id")?;
        let mut changes = Map::new();
        if toggle(api, &playlist, "recordHistory", "record-history")? {
            changes.insert("recordHistory".to_string(), json!(true));
        }
        if toggle(
            api,
            &playlist,
            "showTrackAvailability",
            "track-availability",
        )? {
            changes.insert("showTrackAvailability".to_string(), json!(true));
        }
        let source = &playlist["importSource"];
        if source.is_object() {
            let drifted = drift(source, &wanted);
            if !drifted.is_empty() {
                api.send(
                    "PUT",
                    &format!("/playlists/static-playlists/{id}"),
                    &json!({ "importSource": Value::Object(drifted.clone()) }),
                )?;
                changes.extend(drifted);
            }
        }
        let name = playlist["name"].as_str().unwrap_or("?");
        fixed += report("Aurral", &format!("playlist {name}"), &[&changes]);
    }
    Ok(fixed)
}

pub fn enforce(api: &impl Api) -> Result<usize, String> {
    Ok(settings(api)? + following(api)? + users(api)? + playlists(api)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::fake::Fake;
    use std::collections::BTreeMap;

    fn kept() -> Value {
        json!({
            "integrations": {
                "slskd": {"enabled": true, "url": "http://gluetun:5030", "priority": 10, "apiKey": "secret"},
                "ytdlp": {"enabled": true, "priority": 1000, "stagingPath": "/config/_staging"},
            },
            "qualityProfile": {"cutoff": "flac-standard", "automaticUpgrades": true, "intervalDays": 2},
            "security": {"localNetworkBypass": {"enabled": false}, "defaultPermissions": permissions()},
        })
    }

    fn follow() -> Value {
        json!({"rules": {"catalog": "none", "newReleases": true, "topTracks": 0, "types": ["album", "ep"]}})
    }

    fn api(settings: Value, users: Value) -> Fake {
        Fake::new(BTreeMap::from([
            ("/settings".to_string(), settings),
            ("/library/monitoring/defaults".to_string(), follow()),
            ("/users".to_string(), users),
            (
                "/playlists/status".to_string(),
                json!({"staticPlaylists": []}),
            ),
        ]))
    }

    fn imported() -> Value {
        json!({"provider": "spotify", "externalId": "x", "syncEnabled": false, "syncIntervalHours": 0, "keepRemovedTracks": true})
    }

    fn playlist(id: &str, source: Value) -> Value {
        json!({"id": id, "name": id, "recordHistory": true, "showTrackAvailability": true, "importSource": source})
    }

    #[test]
    fn kept_settings_send_nothing() {
        let fake = api(
            kept(),
            json!([{"id": 3, "username": "friend", "role": "user", "permissions": permissions()}]),
        );
        assert_eq!(Ok(0), enforce(&fake));
        assert!(fake.paths().is_empty());
    }

    #[test]
    fn only_the_drifted_fields_are_sent() {
        let mut settings = kept();
        settings["integrations"]["ytdlp"]["enabled"] = json!(false);
        settings["qualityProfile"]["automaticUpgrades"] = json!(false);
        let fake = api(settings, json!([]));
        assert_eq!(Ok(2), enforce(&fake));
        assert_eq!(vec!["POST /settings", "POST /settings"], fake.paths());
        assert_eq!(
            json!({"integrations": {"ytdlp": {"enabled": true}}}),
            fake.body(0)
        );
        assert_eq!(
            json!({"qualityProfile": {"automaticUpgrades": true}}),
            fake.body(1)
        );
    }

    #[test]
    fn the_slskd_key_is_never_sent() {
        let mut settings = kept();
        settings["integrations"]["slskd"]["enabled"] = json!(false);
        let fake = api(settings, json!([]));
        enforce(&fake).unwrap();
        assert_eq!(
            json!({"integrations": {"slskd": {"enabled": true}}}),
            fake.body(0)
        );
    }

    #[test]
    fn friends_get_the_default_permissions_and_admins_are_left_alone() {
        let mut extra = permissions();
        extra["deleteAlbum"] = json!(true);
        let fake = api(
            kept(),
            json!([
                {"id": 1, "username": "admin", "role": "admin", "permissions": extra},
                {"id": 3, "username": "friend", "role": "user", "permissions": extra},
            ]),
        );
        assert_eq!(Ok(1), enforce(&fake));
        assert_eq!(vec!["PATCH /users/3"], fake.paths());
        assert_eq!(json!({"permissions": permissions()}), fake.body(0));
    }

    #[test]
    fn following_an_artist_brings_only_its_new_releases() {
        let mut fake = api(kept(), json!([]));
        fake.pages.insert(
            "/library/monitoring/defaults".to_string(),
            json!({"rules": {"catalog": "all", "newReleases": true, "topTracks": 0, "types": ["album", "ep"]}}),
        );
        assert_eq!(Ok(1), enforce(&fake));
        assert_eq!(vec!["PUT /library/monitoring/defaults"], fake.paths());
        assert_eq!(follow(), fake.body(0));
    }

    #[test]
    fn kept_playlists_send_nothing() {
        let mut fake = api(kept(), json!([]));
        fake.pages.insert(
            "/playlists/status".to_string(),
            json!({"staticPlaylists": [playlist("a", imported()), playlist("b", Value::Null)]}),
        );
        assert_eq!(Ok(0), enforce(&fake));
        assert!(fake.paths().is_empty());
    }

    #[test]
    fn every_playlist_scrobbles_shows_availability_and_never_syncs() {
        let mut drifted = playlist("a", imported());
        drifted["recordHistory"] = json!(false);
        drifted["showTrackAvailability"] = json!(false);
        drifted["importSource"]["syncEnabled"] = json!(true);
        drifted["importSource"]["syncIntervalHours"] = json!(24);
        drifted["importSource"]["keepRemovedTracks"] = json!(false);
        let mut fake = api(kept(), json!([]));
        fake.pages.insert(
            "/playlists/status".to_string(),
            json!({"staticPlaylists": [drifted]}),
        );
        assert_eq!(Ok(1), enforce(&fake));
        assert_eq!(
            vec![
                "PUT /playlists/static-playlists/a/record-history",
                "PUT /playlists/static-playlists/a/track-availability",
                "PUT /playlists/static-playlists/a",
            ],
            fake.paths()
        );
        assert_eq!(json!({"enabled": true}), fake.body(0));
        assert_eq!(
            json!({"importSource": {"syncEnabled": false, "syncIntervalHours": 0, "keepRemovedTracks": true}}),
            fake.body(2)
        );
    }

    #[test]
    fn a_playlist_made_in_aurral_has_no_sync_to_change() {
        let mut made = playlist("b", Value::Null);
        made["showTrackAvailability"] = json!(false);
        let mut fake = api(kept(), json!([]));
        fake.pages.insert(
            "/playlists/status".to_string(),
            json!({"staticPlaylists": [made]}),
        );
        assert_eq!(Ok(1), enforce(&fake));
        assert_eq!(vec!["PUT /playlists/static-playlists/b/track-availability"], fake.paths());
    }
}
