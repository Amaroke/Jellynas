use crate::compose::{doco_cd, re, read};
use std::collections::BTreeMap;

const REPO: &str = "https://github.com/Amaroke/Jellynas.git";

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn poll_targets() -> Vec<BTreeMap<String, String>> {
    let text = read(&doco_cd());
    let lines: Vec<&str> = text.lines().collect();
    let header = re(r"^\s+POLL_CONFIG: \|$");
    let entry = re(r"^\s*(- )?([a-z_]+): (.+)$");
    let start = lines.iter().position(|line| header.is_match(line)).unwrap();
    let inner = indent(lines[start]) + 2;
    let mut targets: Vec<BTreeMap<String, String>> = Vec::new();
    for line in &lines[start + 1..] {
        if !line.trim().is_empty() && indent(line) < inner {
            break;
        }
        let Some(found) = entry.captures(line) else {
            continue;
        };
        if found.get(1).is_some() {
            targets.push(BTreeMap::new());
        }
        targets
            .last_mut()
            .unwrap()
            .insert(found[2].to_string(), found[3].trim().to_string());
    }
    targets
}

#[test]
fn polls_main_of_the_repo_every_minute() {
    let targets = poll_targets();
    let main: Vec<_> = targets
        .iter()
        .filter(|target| {
            target.get("url").map(String::as_str) == Some(REPO)
                && target.get("reference").map(String::as_str) == Some("main")
        })
        .collect();
    assert_eq!(1, main.len(), "{targets:?}");
    assert_eq!(Some("1m"), main[0].get("interval").map(String::as_str));
}
