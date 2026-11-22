use crate::widgets::{Figure, Reading, Row, bytes};
use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::path::Path;

#[derive(Default)]
pub struct Walk {
    seen: HashSet<(u64, u64)>,
    pub unreadable: usize,
}

fn hidden(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    name.starts_with('.') || name == "lost+found"
}

#[cfg(unix)]
fn first_link(walk: &mut Walk, metadata: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() < 2 || walk.seen.insert((metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn first_link(walk: &mut Walk, metadata: &Metadata) -> bool {
    let _ = (&walk.seen, metadata);
    true
}

fn size(walk: &mut Walk, path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        walk.unreadable += 1;
        return 0;
    };
    if metadata.is_file() {
        return if first_link(walk, &metadata) {
            metadata.len()
        } else {
            0
        };
    }
    if !metadata.is_dir() {
        return 0;
    }
    let Ok(entries) = fs::read_dir(path) else {
        walk.unreadable += 1;
        return 0;
    };
    let mut total = 0;
    for entry in entries {
        match entry {
            Ok(entry) if hidden(&entry.file_name()) => {}
            Ok(entry) => total += size(walk, &entry.path()),
            Err(_) => walk.unreadable += 1,
        }
    }
    total
}

pub fn measure(root: &Path) -> Result<(Vec<(String, u64)>, usize), String> {
    let mut walk = Walk::default();
    let mut found: Vec<(String, u64)> = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let Ok(entry) = entry else {
            walk.unreadable += 1;
            continue;
        };
        if hidden(&entry.file_name()) || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let total = size(&mut walk, &entry.path());
        found.push((entry.file_name().to_string_lossy().into_owned(), total));
    }
    found.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok((found, walk.unreadable))
}

fn titled(name: &str) -> String {
    let mut letters = name.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

pub fn reading(sizes: &[(String, u64)]) -> Reading {
    let total: u64 = sizes.iter().map(|(_, size)| size).sum();
    Reading {
        figures: vec![
            Figure {
                label: "Media".into(),
                value: bytes(total as f64),
            },
            Figure {
                label: "Categories".into(),
                value: sizes.len().to_string(),
            },
        ],
        rows: sizes
            .iter()
            .map(|(name, size)| Row {
                title: titled(name),
                detail: bytes(*size as f64),
                progress: Some(if total > 0 {
                    *size as f64 / total as f64
                } else {
                    0.0
                }),
                image: None,
                id: None,
            })
            .collect(),
        idle: Some("The Media tree is empty".into()),
        ..Reading::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_category_sums_every_file_below_it() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("movies/Heat (1995)")).unwrap();
        fs::create_dir_all(root.path().join("music")).unwrap();
        fs::create_dir_all(root.path().join("books")).unwrap();
        fs::write(root.path().join("movies/a.mkv"), vec![0; 100]).unwrap();
        fs::write(root.path().join("movies/Heat (1995)/b.mkv"), vec![0; 50]).unwrap();
        fs::write(root.path().join("music/c.flac"), vec![0; 10]).unwrap();
        fs::write(root.path().join("loose.txt"), vec![0; 999]).unwrap();
        fs::create_dir_all(root.path().join("movies/.recycle")).unwrap();
        fs::write(root.path().join("movies/.recycle/old.mkv"), vec![0; 500]).unwrap();
        fs::create_dir_all(root.path().join(".zfs/snapshot")).unwrap();
        fs::write(root.path().join(".zfs/snapshot/copy.mkv"), vec![0; 700]).unwrap();
        assert_eq!(
            (
                vec![
                    ("movies".to_string(), 150),
                    ("music".to_string(), 10),
                    ("books".to_string(), 0)
                ],
                0
            ),
            measure(root.path()).unwrap()
        );
        assert!(measure(&root.path().join("missing")).is_err());
    }

    #[test]
    fn the_space_lists_each_category_with_its_share() {
        let reading = reading(&[("aurral".into(), 3072), ("movies".into(), 1024)]);
        let figures: Vec<(&str, &str)> = reading
            .figures
            .iter()
            .map(|figure| (figure.label.as_str(), figure.value.as_str()))
            .collect();
        assert_eq!(vec![("Media", "4.0 KiB"), ("Categories", "2")], figures);
        let rows: Vec<(&str, &str, Option<f64>)> = reading
            .rows
            .iter()
            .map(|row| (row.title.as_str(), row.detail.as_str(), row.progress))
            .collect();
        assert_eq!(
            vec![
                ("Aurral", "3.0 KiB", Some(0.75)),
                ("Movies", "1.0 KiB", Some(0.25))
            ],
            rows
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_hard_link_counts_once() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("music")).unwrap();
        fs::create_dir_all(root.path().join("aurral")).unwrap();
        fs::write(root.path().join("music/a.flac"), vec![0; 100]).unwrap();
        fs::hard_link(
            root.path().join("music/a.flac"),
            root.path().join("aurral/a.flac"),
        )
        .unwrap();
        let (sizes, _) = measure(root.path()).unwrap();
        let total: u64 = sizes.iter().map(|(_, size)| size).sum();
        assert_eq!(100, total);
    }
}
