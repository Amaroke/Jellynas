use crate::cards::icon_file;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

const CDN: &str = "https://cdn.jsdelivr.net/gh/selfhst/icons";
const RETRY: Duration = Duration::from_secs(600);

pub struct Icons {
    dir: PathBuf,
    agent: ureq::Agent,
    failed: Mutex<HashMap<String, Instant>>,
}

impl Icons {
    pub fn new(dir: PathBuf) -> Self {
        Icons {
            dir,
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(15)))
                .build()
                .into(),
            failed: Mutex::new(HashMap::new()),
        }
    }

    fn failed(&self) -> MutexGuard<'_, HashMap<String, Instant>> {
        self.failed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn get(&self, file: &str) -> Option<(&'static str, Vec<u8>)> {
        let (_, extension) = file.rsplit_once('.')?;
        let kind = match extension {
            _ if !icon_file(file) => return None,
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "webp" => "image/webp",
            _ => return None,
        };
        let path = self.dir.join(file);
        if let Ok(bytes) = fs::read(&path) {
            return Some((kind, bytes));
        }
        if self
            .failed()
            .get(file)
            .is_some_and(|since| since.elapsed() < RETRY)
        {
            return None;
        }
        match self.fetch(&format!("{CDN}/{extension}/{file}")) {
            Ok(bytes) => {
                let written = self.dir.join(format!("{file}.tmp"));
                let saved = fs::create_dir_all(&self.dir)
                    .and_then(|()| fs::write(&written, &bytes))
                    .and_then(|()| fs::rename(&written, &path));
                if let Err(error) = saved {
                    eprintln!("Cannot cache the icon {file}: {error}");
                }
                self.failed().remove(file);
                Some((kind, bytes))
            }
            Err(error) => {
                eprintln!("Cannot fetch the icon {file}: {error}");
                self.failed().insert(file.to_string(), Instant::now());
                None
            }
        }
    }

    fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        self.agent
            .get(url)
            .call()
            .and_then(|mut response| response.body_mut().read_to_vec())
            .map_err(|error| error.to_string())
    }
}
