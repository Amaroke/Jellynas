use crate::accounts::Account;
use crate::api::{Api, encode, report};
use serde_json::{Map, Value};

const SETTINGS: [(&str, &str, &str); 19] = [
    ("General", "AUTH_TYPE", "BASIC"),
    ("General", "USER_ACCOUNTS", ""),
    ("General", "EBOOK_DIR", "/data/media/books"),
    ("General", "DESTINATION_COPY", "1"),
    ("PostProcess", "EBOOK_DEST_FOLDER", "{$SerName|$Title}"),
    (
        "PostProcess",
        "EBOOK_DEST_FILE",
        "$Title{ ($SerName, #$PadNum)}",
    ),
    ("Comics", "COMIC_TAB", "1"),
    ("Comics", "COMIC_RELATIVE", ""),
    ("Comics", "COMIC_DEST_FOLDER", "/data/media/manga/$Title"),
    ("TORRENT", "TOR_DOWNLOADER_QBITTORRENT", "1"),
    ("TORRENT", "KEEP_SEEDING", "1"),
    ("TORRENT", "SEED_WAIT", "1"),
    ("QBITTORRENT", "QBITTORRENT_HOST", "gluetun"),
    ("QBITTORRENT", "QBITTORRENT_PORT", "8080"),
    ("QBITTORRENT", "QBITTORRENT_LABEL", "books"),
    ("QBITTORRENT", "QBITTORRENT_DIR", "/data/torrents/books"),
    ("Telemetry", "TELEMETRY_ENABLE", ""),
    ("LibraryScan", "NEWAUTHOR_BOOKS", "1"),
    ("API", "CV_WEBSEARCH", "1"),
];

fn read_path(group: &str, name: &str) -> String {
    format!("&cmd=readCFG&name={name}&group={group}")
}

fn write_path(group: &str, name: &str, value: &str) -> String {
    format!(
        "&cmd=writeCFG&name={name}&group={group}&value={}",
        encode(value)
    )
}

fn scan_path(folder: &str) -> String {
    format!("&cmd=forceLibraryScan&dir={}", encode(folder))
}

fn read(api: &impl Api, group: &str, name: &str) -> Result<String, String> {
    let reply = api.text(&read_path(group, name))?;
    reply
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .map(String::from)
        .ok_or(format!("cannot read {name}: {reply}"))
}

pub fn enforce(api: &impl Api, accounts: &[Account]) -> Result<usize, String> {
    let mut changes = Map::new();
    for (group, name, wanted) in SETTINGS {
        if read(api, group, name)? == wanted {
            continue;
        }
        let reply = api.text(&write_path(group, name, wanted))?;
        if reply != "OK" {
            return Err(format!("cannot set {name}: {reply}"));
        }
        changes.insert(name.to_string(), Value::from(wanted));
    }
    for account in accounts {
        let reply = api.text(&scan_path(&account.books()))?;
        if reply != "OK" {
            return Err(format!("cannot scan the books of {}: {reply}", account.id));
        }
    }
    Ok(report("LazyLibrarian", "config", &[&changes]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::fake::Fake;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn in_line() -> BTreeMap<String, Value> {
        let mut pages = BTreeMap::new();
        for (group, name, value) in SETTINGS {
            pages.insert(read_path(group, name), json!(format!("[{value}]")));
        }
        pages
    }

    fn writes(fake: &Fake) -> Vec<String> {
        let paths = fake.paths();
        paths
            .into_iter()
            .filter_map(|path| path.strip_prefix("GET ").map(String::from))
            .filter(|path| path.contains("writeCFG"))
            .collect()
    }

    #[test]
    fn a_lazylibrarian_in_line_needs_nothing() {
        let fake = Fake::new(in_line());
        assert_eq!(Ok(0), enforce(&fake, &[]));
        assert!(writes(&fake).is_empty());
    }

    #[test]
    fn a_setting_changed_by_hand_comes_back() {
        let mut pages = in_line();
        let label = read_path("QBITTORRENT", "QBITTORRENT_LABEL");
        pages.insert(label, json!("[ebooks]"));
        pages.insert(read_path("General", "USER_ACCOUNTS"), json!("[1]"));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&fake, &[]));
        let expected = [
            write_path("General", "USER_ACCOUNTS", ""),
            write_path("QBITTORRENT", "QBITTORRENT_LABEL", "books"),
        ];
        assert_eq!(expected.to_vec(), writes(&fake));
    }

    #[test]
    fn a_book_lands_in_the_folder_of_its_series_with_its_index() {
        let mut pages = in_line();
        pages.insert(
            read_path("PostProcess", "EBOOK_DEST_FILE"),
            json!("[$Title - $Author]"),
        );
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&fake, &[]));
        assert_eq!(
            vec![
                "&cmd=writeCFG&name=EBOOK_DEST_FILE&group=PostProcess&value=%24Title%7B%20%28%24SerName%2C%20%23%24PadNum%29%7D"
            ],
            writes(&fake)
        );
    }

    #[test]
    fn manga_are_filed_in_the_manga_folder() {
        let mut pages = in_line();
        pages.insert(read_path("Comics", "COMIC_RELATIVE"), json!("[1]"));
        pages.insert(read_path("Comics", "COMIC_TAB"), json!("[]"));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&fake, &[]));
        assert_eq!(
            vec![
                write_path("Comics", "COMIC_TAB", "1"),
                write_path("Comics", "COMIC_RELATIVE", ""),
            ],
            writes(&fake)
        );
    }

    fn scans(fake: &Fake) -> Vec<String> {
        fake.paths()
            .into_iter()
            .filter_map(|path| path.strip_prefix("GET ").map(String::from))
            .filter(|path| path.contains("forceLibraryScan"))
            .collect()
    }

    fn account(id: &str) -> Account {
        Account {
            id: id.into(),
            name: id.into(),
        }
    }

    #[test]
    fn the_books_of_every_private_library_are_scanned() {
        let fake = Fake::new(in_line());
        assert_eq!(
            Ok(0),
            enforce(&fake, &[account("amaroke"), account("guest")])
        );
        assert_eq!(
            vec![
                "&cmd=forceLibraryScan&dir=%2Fdata%2Fmedia%2Fprivate%2Famaroke%2Fbooks",
                "&cmd=forceLibraryScan&dir=%2Fdata%2Fmedia%2Fprivate%2Fguest%2Fbooks",
            ],
            scans(&fake)
        );
    }

    #[test]
    fn a_refused_scan_is_an_error() {
        let mut pages = in_line();
        let scan = scan_path("/data/media/private/amaroke/books");
        pages.insert(scan, json!("Unknown command"));
        let fake = Fake::new(pages);
        assert_eq!(
            Err("cannot scan the books of amaroke: Unknown command".to_string()),
            enforce(&fake, &[account("amaroke")])
        );
    }

    #[test]
    fn found_authors_bring_their_books_and_manga_match_without_a_key() {
        let mut pages = in_line();
        pages.insert(read_path("LibraryScan", "NEWAUTHOR_BOOKS"), json!("[]"));
        pages.insert(read_path("API", "CV_WEBSEARCH"), json!("[]"));
        let fake = Fake::new(pages);
        assert_eq!(Ok(1), enforce(&fake, &[]));
        assert_eq!(
            vec![
                write_path("LibraryScan", "NEWAUTHOR_BOOKS", "1"),
                write_path("API", "CV_WEBSEARCH", "1"),
            ],
            writes(&fake)
        );
    }

    #[test]
    fn a_refused_write_is_an_error() {
        let mut pages = in_line();
        pages.insert(read_path("TORRENT", "SEED_WAIT"), json!("[]"));
        let write = write_path("TORRENT", "SEED_WAIT", "1");
        pages.insert(write, json!("SEED_WAIT not found"));
        let fake = Fake::new(pages);
        assert_eq!(
            Err("cannot set SEED_WAIT: SEED_WAIT not found".to_string()),
            enforce(&fake, &[])
        );
    }

    #[test]
    fn an_unreadable_reply_is_an_error() {
        let mut pages = in_line();
        pages.insert(read_path("General", "AUTH_TYPE"), json!("API not enabled"));
        let fake = Fake::new(pages);
        assert!(enforce(&fake, &[]).is_err());
    }
}
