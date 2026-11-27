use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

pub const FOLDERS: [&str; 2] = ["downloads", "incomplete"];
const SETTLED: Duration = Duration::from_secs(60 * 60);

fn settled(path: &Path, now: SystemTime) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| now.duration_since(modified).unwrap_or_default() >= SETTLED)
}

fn sweep(folder: &Path, now: SystemTime, delete: bool, files: &mut usize, bytes: &mut u64) -> bool {
    let Ok(entries) = fs::read_dir(folder) else {
        return false;
    };
    let mut empty = true;
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {
                if sweep(&path, now, delete, files, bytes) && delete {
                    let _ = fs::remove_dir(&path);
                } else {
                    empty = false;
                }
            }
            Ok(_) if settled(&path, now) => {
                *files += 1;
                *bytes += entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
                if !(delete && fs::remove_file(&path).is_ok()) {
                    empty = false;
                }
            }
            _ => empty = false,
        }
    }
    empty
}

pub fn leftovers(root: &Path, now: SystemTime, delete: bool) -> Value {
    let mut out = json!({});
    for name in FOLDERS {
        let (mut files, mut bytes) = (0, 0);
        sweep(&root.join(name), now, delete, &mut files, &mut bytes);
        out[name] = json!({"files": files, "megabytes": bytes / 1_000_000});
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("check-slskd-{name}-{}", std::process::id()));
        fs::create_dir_all(root.join("downloads/Album")).unwrap();
        fs::create_dir_all(root.join("incomplete/user/Empty")).unwrap();
        fs::write(root.join("downloads/Album/01.flac"), [0; 2_000_000]).unwrap();
        root
    }

    #[test]
    fn settled_files_are_counted_and_fresh_ones_left_out() {
        let root = root("count");
        let later = SystemTime::now() + SETTLED;
        let out = leftovers(&root, later, false);
        assert_eq!(json!({"files": 1, "megabytes": 2}), out["downloads"]);
        assert_eq!(
            0,
            leftovers(&root, SystemTime::now(), false)["downloads"]["files"]
        );
        assert!(root.join("downloads/Album/01.flac").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cleaning_removes_settled_files_and_empty_folders_but_keeps_the_roots() {
        let root = root("clean");
        leftovers(&root, SystemTime::now() + SETTLED, true);
        assert!(!root.join("downloads/Album").exists());
        assert!(!root.join("incomplete/user").exists());
        assert!(root.join("downloads").exists());
        assert!(root.join("incomplete").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_fresh_file_keeps_its_folder() {
        let root = root("fresh");
        leftovers(&root, SystemTime::now(), true);
        assert!(root.join("downloads/Album/01.flac").exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
