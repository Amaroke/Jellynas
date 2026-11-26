use crate::api::{Api, fetch};
use crate::input::Input;
use crate::report::{Report, Severity, trimmed};
use serde_json::{Value, json};

const LISTS: [(&str, &str); 7] = [
    ("authors", "cmd=getIndex"),
    ("wanted", "cmd=getWanted"),
    ("snatched", "cmd=getSnatched"),
    ("history", "cmd=getHistory"),
    ("providers", "cmd=listProviders"),
    ("jobs", "cmd=showJobs&json=1"),
    ("stats", "cmd=showStats&json=1"),
];

const KEPT: [&str; 11] = [
    "AuthorName",
    "BookName",
    "BookID",
    "Status",
    "BookLibrary",
    "NZBtitle",
    "NZBprov",
    "NZBdate",
    "NZBmode",
    "Source",
    "DLResult",
];

const PATHS: [(&str, &str); 4] = [
    ("General", "EBOOK_DIR"),
    ("General", "DOWNLOAD_DIR"),
    ("QBITTORRENT", "QBITTORRENT_DIR"),
    ("Comics", "COMIC_DEST_FOLDER"),
];

fn mounted(path: &str, mounts: &[String]) -> bool {
    mounts.iter().any(|mount| {
        path == mount
            || path
                .strip_prefix(mount.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

fn setting(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    text.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string()
}

fn slim(value: &Value) -> Value {
    let Some(items) = value.as_array() else {
        return value.clone();
    };
    items
        .iter()
        .map(|item| match item.as_object() {
            Some(fields) if KEPT.iter().any(|key| fields.contains_key(*key)) => fields
                .iter()
                .filter(|(key, _)| KEPT.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            _ => item.clone(),
        })
        .collect()
}

fn failures(history: &Value) -> usize {
    history
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item.to_string().contains("\"Failed\""))
        .count()
}

pub fn check(report: &mut Report, api: &impl Api, input: &Input) {
    if let Some(version) = fetch(report, api, "cmd=getVersion") {
        report.fact("version", version);
    }
    for (key, path) in LISTS {
        if let Some(value) = fetch(report, api, path) {
            if key == "history" {
                let failed = failures(&value);
                if failed > 0 {
                    report.flag(
                        Severity::Low,
                        "lazylibrarian",
                        format!("{failed} downloads in the history of LazyLibrarian failed"),
                    );
                }
            }
            report.fact(key, trimmed(&slim(&value), 40));
        }
    }
    let mounts = input.mounts.get("lazylibrarian");
    let mut paths = json!({});
    for (group, name) in PATHS {
        let Some(value) = fetch(
            report,
            api,
            &format!("cmd=readCFG&name={name}&group={group}"),
        ) else {
            continue;
        };
        let path = setting(&value);
        if let Some(mounts) = mounts
            && path.starts_with('/')
            && !mounted(&path, mounts)
        {
            report.flag(
                Severity::Medium,
                "lazylibrarian",
                format!("{name} is {path}, which is not on a mounted volume of lazylibrarian, so its files live and die with the container"),
            );
        }
        paths[name] = json!(path);
    }
    report.fact("paths", paths);
    if let Some(mounts) = mounts {
        report.fact("mounts", json!(mounts));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::fake::Fake;
    use crate::input::parse;

    fn api(download_dir: &str) -> Fake {
        let mut pages = vec![("cmd=getVersion", json!({"current_version": "1"}))];
        for (_, path) in LISTS {
            pages.push((path, json!([])));
        }
        let paths = [
            (
                "cmd=readCFG&name=EBOOK_DIR&group=General",
                "[/data/media/books]",
            ),
            ("cmd=readCFG&name=DOWNLOAD_DIR&group=General", download_dir),
            (
                "cmd=readCFG&name=QBITTORRENT_DIR&group=QBITTORRENT",
                "[/data/torrents/books]",
            ),
            (
                "cmd=readCFG&name=COMIC_DEST_FOLDER&group=Comics",
                "[/data/media/manga/$Title]",
            ),
        ];
        for (path, value) in paths {
            pages.push((path, json!(value)));
        }
        Fake::new(&pages)
    }

    fn input() -> Input {
        parse(
            "mount|lazylibrarian|/config\nmount|lazylibrarian|/data/media/books\nmount|lazylibrarian|/data/media/manga\nmount|lazylibrarian|/data/torrents\n",
        )
    }

    #[test]
    fn a_download_dir_outside_every_mount_is_flagged() {
        let mut report = Report::default();
        check(&mut report, &api("[/downloads]"), &input());
        assert!(report.flagged(Severity::Medium, "DOWNLOAD_DIR is /downloads"));
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.facts["paths"]["EBOOK_DIR"], "/data/media/books");
    }

    #[test]
    fn mounted_paths_raise_nothing() {
        let mut report = Report::default();
        check(&mut report, &api("[/data/torrents/books]"), &input());
        assert!(report.findings.is_empty());
        assert!(report.errors.is_empty());
    }

    #[test]
    fn book_lists_keep_only_the_telling_fields() {
        let out = slim(&json!([{"BookName": "A", "BookDesc": "long", "Status": "Wanted"}, 3]));
        assert_eq!(out, json!([{"BookName": "A", "Status": "Wanted"}, 3]));
    }

    #[test]
    fn a_path_only_sharing_a_prefix_with_a_mount_is_not_mounted() {
        assert!(!mounted("/data/torrents2", &["/data/torrents".into()]));
        assert!(mounted("/data/torrents/books", &["/data/torrents".into()]));
    }

    #[test]
    fn failed_downloads_in_the_history_are_flagged() {
        let mut fake = api("[/data/torrents/books]");
        fake.0.insert(
            "cmd=getHistory".into(),
            json!([{"Status": "Failed"}, {"Status": "Processed"}]),
        );
        let mut report = Report::default();
        check(&mut report, &fake, &input());
        assert!(report.flagged(Severity::Low, "1 downloads"));
    }
}
