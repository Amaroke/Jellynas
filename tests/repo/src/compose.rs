use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub type Services = BTreeMap<String, Vec<String>>;

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

pub fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .replace("\r\n", "\n")
}

pub fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap()
}

pub fn set<'a>(items: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    items.into_iter().map(str::to_string).collect()
}

pub fn has(lines: &[String], line: &str) -> bool {
    lines.iter().any(|found| found == line)
}

pub fn doco_cd() -> PathBuf {
    root().join("services/doco-cd/compose.yaml")
}

pub fn included_compose_files() -> Vec<PathBuf> {
    let text = read(&root().join("compose.yaml"));
    re(r"(?m)^  - ([^ #]*compose\.yaml)$")
        .captures_iter(&text)
        .map(|found| root().join(&found[1]))
        .collect()
}

pub fn service_blocks(path: &Path) -> Services {
    let top = re(r"^([a-z_]+):");
    let name = re(r"^  ([a-z0-9_-]+):$");
    let mut services = Services::new();
    let mut section = None;
    let mut service: Option<String> = None;
    for line in read(path).lines() {
        if let Some(found) = top.captures(line) {
            section = Some(found[1].to_string());
            service = None;
            continue;
        }
        if section.as_deref() == Some("services")
            && let Some(found) = name.captures(line)
        {
            services.insert(found[1].to_string(), Vec::new());
            service = Some(found[1].to_string());
            continue;
        }
        if let Some(service) = &service {
            services.get_mut(service).unwrap().push(line.to_string());
        }
    }
    services
}

pub fn all_services() -> Services {
    included_compose_files()
        .iter()
        .flat_map(|path| service_blocks(path))
        .collect()
}

pub fn section(lines: &[String], key: &str) -> Vec<String> {
    let header = re(r"^    [a-z_]+:");
    let mut found = Vec::new();
    let mut inside = false;
    for line in lines {
        if header.is_match(line) {
            inside = line.trim() == format!("{key}:");
            continue;
        }
        if inside && line.starts_with("      ") {
            found.push(line.clone());
        }
    }
    found
}
