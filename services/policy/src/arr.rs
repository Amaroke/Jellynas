use crate::api::{Api, report, rules, settle};
use serde_json::{Map, Value, json};

pub struct Arr {
    pub name: &'static str,
    pub roots: &'static [&'static str],
    pub recycle_bin: &'static str,
    pub unmonitor_deleted: &'static str,
    pub naming: Map<String, Value>,
    pub category: (&'static str, &'static str),
}

pub fn radarr() -> Arr {
    Arr {
        name: "Radarr",
        roots: &["/data/media/movies"],
        recycle_bin: "/data/media/movies/.recycle",
        unmonitor_deleted: "autoUnmonitorPreviouslyDownloadedMovies",
        naming: rules(json!({
            "renameMovies": true,
            "standardMovieFormat": "{Movie Title} ({Release Year}) [tmdbid-{TmdbId}]",
            "movieFolderFormat": "{Movie Title} ({Release Year}) [tmdbid-{TmdbId}]",
        })),
        category: ("movieCategory", "movies"),
    }
}

pub fn sonarr() -> Arr {
    Arr {
        name: "Sonarr",
        roots: &["/data/media/shows", "/data/media/anime"],
        recycle_bin: "/data/media/shows/.recycle",
        unmonitor_deleted: "autoUnmonitorPreviouslyDownloadedEpisodes",
        naming: rules(json!({
            "renameEpisodes": true,
            "seriesFolderFormat": "{Series TitleYear} [tvdbid-{TvdbId}]",
        })),
        category: ("tvCategory", "shows"),
    }
}

impl Arr {
    fn pages(&self) -> Vec<(&'static str, &'static str, Map<String, Value>)> {
        vec![
            (
                "general",
                "/config/host",
                rules(json!({
                    "authenticationMethod": "forms",
                    "authenticationRequired": "enabled",
                    "analyticsEnabled": false,
                    "logLevel": "info",
                })),
            ),
            (
                "media management",
                "/config/mediamanagement",
                rules(json!({
                    "copyUsingHardlinks": true,
                    "recycleBin": self.recycle_bin,
                    "recycleBinCleanupDays": 30,
                    self.unmonitor_deleted: true,
                })),
            ),
            ("naming", "/config/naming", self.naming.clone()),
        ]
    }

    fn client_fields(&self) -> Map<String, Value> {
        let (field, category) = self.category;
        rules(json!({"host": "gluetun", "port": 8080, field: category}))
    }
}

fn client_changes(client: &Value, wanted: &Map<String, Value>) -> (Map<String, Value>, Value) {
    let mut changes = Map::new();
    let mut whole = client.clone();
    for (key, value) in [
        ("removeCompletedDownloads", json!(true)),
        ("removeFailedDownloads", json!(true)),
    ] {
        if whole[key] != value {
            whole[key] = value.clone();
            changes.insert(key.into(), value);
        }
    }
    if let Some(fields) = whole["fields"].as_array_mut() {
        for field in fields {
            let Some(name) = field["name"].as_str().map(String::from) else {
                continue;
            };
            if let Some(value) = wanted.get(&name)
                && field["value"] != *value
            {
                field["value"] = value.clone();
                changes.insert(name, value.clone());
            }
        }
    }
    (changes, whole)
}

pub fn enforce(arr: &Arr, api: &impl Api) -> Result<usize, String> {
    let mut changed = 0;
    for (page, path, wanted) in arr.pages() {
        let current = api.get(path)?;
        let id = &current["id"];
        let changes = settle(api, "PUT", &format!("{path}/{id}"), &current, &wanted)?;
        changed += report(arr.name, page, &[&changes]);
    }
    let roots = api.get("/rootfolder")?;
    let known: Vec<&str> = roots
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|root| root["path"].as_str())
                .collect()
        })
        .unwrap_or_default();
    for root in arr.roots.iter().filter(|root| !known.contains(root)) {
        api.send("POST", "/rootfolder", &json!({"path": root}))?;
        println!("{} root folders: added {root}", arr.name);
        changed += 1;
    }
    let clients = api.get("/downloadclient")?;
    for client in clients.as_array().into_iter().flatten() {
        if client["implementation"] != json!("QBittorrent") {
            continue;
        }
        let (changes, whole) = client_changes(client, &arr.client_fields());
        if !changes.is_empty() {
            api.send(
                "PUT",
                &format!("/downloadclient/{}?forceSave=true", client["id"]),
                &whole,
            )?;
        }
        changed += report(arr.name, "download client", &[&changes]);
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::fake::Fake;
    use std::collections::BTreeMap;

    fn client(category: &str, enable: bool) -> Value {
        json!({
            "id": 1,
            "enable": enable,
            "implementation": "QBittorrent",
            "removeCompletedDownloads": true,
            "removeFailedDownloads": true,
            "fields": [
                {"name": "host", "value": "gluetun"},
                {"name": "port", "value": 8080},
                {"name": "password", "value": "********"},
                {"name": "movieCategory", "value": category},
            ],
        })
    }

    fn in_line(arr: &Arr) -> BTreeMap<String, Value> {
        let mut pages: BTreeMap<String, Value> = arr
            .pages()
            .into_iter()
            .map(|(_, path, wanted)| {
                let mut page = wanted;
                page.insert("id".into(), json!(1));
                page.insert("untouched".into(), json!("kept"));
                (path.to_string(), Value::Object(page))
            })
            .collect();
        let roots: Vec<Value> = arr.roots.iter().map(|path| json!({"path": path})).collect();
        pages.insert("/rootfolder".into(), json!(roots));
        pages.insert("/downloadclient".into(), json!([client("movies", true)]));
        pages
    }

    #[test]
    fn a_radarr_in_line_needs_nothing() {
        let fake = Fake::new(in_line(&radarr()));
        assert_eq!(Ok(0), enforce(&radarr(), &fake));
        assert!(fake.paths().is_empty());
    }

    #[test]
    fn a_page_changed_by_hand_comes_back_with_its_id() {
        let mut pages = in_line(&radarr());
        pages.get_mut("/config/host").unwrap()["analyticsEnabled"] = json!(true);
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&radarr(), &fake));
        assert_eq!(vec!["PUT /config/host/1"], fake.paths());
        assert_eq!(json!(false), fake.body(0)["analyticsEnabled"]);
        assert_eq!(json!("kept"), fake.body(0)["untouched"]);
    }

    #[test]
    fn an_import_by_copy_turns_back_into_a_hardlink() {
        for arr in [radarr(), sonarr()] {
            let mut pages = in_line(&arr);
            pages.get_mut("/config/mediamanagement").unwrap()["copyUsingHardlinks"] = json!(false);
            let fake = Fake::new(pages);
            assert_eq!(Ok(1), enforce(&arr, &fake));
            assert_eq!(vec!["PUT /config/mediamanagement/1"], fake.paths());
            assert_eq!(json!(true), fake.body(0)["copyUsingHardlinks"]);
        }
    }

    #[test]
    fn a_missing_root_folder_is_added() {
        let mut pages = in_line(&sonarr());
        pages.insert("/rootfolder".into(), json!([{"path": "/data/media/shows"}]));
        let mut tv = client("shows", true);
        tv["fields"][3] = json!({"name": "tvCategory", "value": "shows"});
        pages.insert("/downloadclient".into(), json!([tv]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&sonarr(), &fake));
        assert_eq!(vec!["POST /rootfolder"], fake.paths());
        assert_eq!(json!({"path": "/data/media/anime"}), fake.body(0));
    }

    #[test]
    fn the_client_is_fixed_but_its_pause_and_password_are_kept() {
        let mut paused = client("films", false);
        paused["removeCompletedDownloads"] = json!(false);
        let mut pages = in_line(&radarr());
        pages.insert("/downloadclient".into(), json!([paused]));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&radarr(), &fake));
        assert_eq!(vec!["PUT /downloadclient/1?forceSave=true"], fake.paths());
        let saved = fake.body(0);
        assert_eq!(json!(false), saved["enable"]);
        assert_eq!(json!(true), saved["removeCompletedDownloads"]);
        assert_eq!(json!("movies"), saved["fields"][3]["value"]);
        assert_eq!(json!("********"), saved["fields"][2]["value"]);
    }

    #[test]
    fn another_kind_of_client_is_left_alone() {
        let mut pages = in_line(&radarr());
        pages.insert(
            "/downloadclient".into(),
            json!([{"id": 2, "implementation": "Transmission", "removeCompletedDownloads": false, "fields": []}]),
        );
        let fake = Fake::new(pages);
        assert_eq!(Ok(0), enforce(&radarr(), &fake));
    }
}
