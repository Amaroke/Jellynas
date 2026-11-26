use crate::report::{Report, Severity};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MARKERS: [&str; 6] = [
    "apikey=",
    "api_key=",
    "x-api-key: ",
    "token=",
    "password=",
    "passkey=",
];

#[derive(Debug, PartialEq)]
pub enum Level {
    Warning,
    Error,
}

pub fn level(line: &str) -> Option<Level> {
    let errors = ["|Error|", "[Error]", "[Fatal]", " ERROR:", " ERROR "];
    let warnings = ["|Warn|", "[Warn]", " WARNING:", " WARN "];
    if errors.iter().any(|mark| line.contains(mark)) {
        Some(Level::Error)
    } else if warnings.iter().any(|mark| line.contains(mark)) {
        Some(Level::Warning)
    } else {
        None
    }
}

pub fn mask(line: &str) -> (String, bool) {
    let lower = line.to_ascii_lowercase();
    let mut out = String::with_capacity(line.len());
    let mut leaked = false;
    let mut at = 0;
    while at < line.len() {
        let next = MARKERS
            .iter()
            .filter_map(|marker| lower[at..].find(marker).map(|found| (found, marker.len())))
            .min();
        let Some((found, length)) = next else {
            out.push_str(&line[at..]);
            break;
        };
        let start = at + found + length;
        out.push_str(&line[at..start]);
        let secret = line[start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .map_or(line.len(), |end| start + end);
        if secret - start >= 8 {
            out.push_str("***");
            leaked = true;
        } else {
            out.push_str(&line[start..secret]);
        }
        at = secret;
    }
    (out, leaked)
}

fn split_time(line: &str) -> (&str, &str) {
    match line.split_once(' ') {
        Some((time, rest)) if time.len() >= 20 && time.ends_with('Z') => (time, rest),
        _ => ("", line),
    }
}

pub fn normalize(message: &str) -> String {
    let mut out = String::new();
    let mut digits = false;
    for c in message.trim().chars() {
        if c.is_ascii_digit() {
            if !digits {
                out.push('#');
            }
            digits = true;
        } else {
            digits = false;
            out.push(c);
        }
    }
    out.chars().take(180).collect()
}

pub fn summarize(report: &mut Report, source: &str, lines: &[String]) {
    let mut warnings = 0;
    let mut errors = 0;
    let mut leaked = 0;
    let mut example = None;
    let mut groups: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for line in lines {
        let (masked, leak) = mask(line);
        if leak {
            leaked += 1;
            example.get_or_insert_with(|| masked.chars().take(240).collect::<String>());
        }
        let Some(found) = level(&masked) else {
            continue;
        };
        match found {
            Level::Warning => warnings += 1,
            Level::Error => errors += 1,
        }
        let (time, message) = split_time(&masked);
        let group = groups
            .entry(normalize(message))
            .or_insert((0, String::new()));
        group.0 += 1;
        group.1 = time.to_string();
    }
    let mut top: Vec<(String, (usize, String))> = groups.into_iter().collect();
    top.sort_by_key(|group| std::cmp::Reverse(group.1.0));
    let top: Vec<Value> = top
        .into_iter()
        .take(20)
        .map(|(message, (count, last))| json!({"count": count, "last": last, "message": message}))
        .collect();
    if leaked > 0 {
        report.flag(
            Severity::High,
            source,
            format!(
                "{leaked} log lines of {source} show a secret in clear, like: {}",
                example.unwrap_or_default()
            ),
        );
    }
    let mut logs = report.facts.remove("logs").unwrap_or_else(|| json!({}));
    logs[source] = json!({
        "lines": lines.len(),
        "warnings": warnings,
        "errors": errors,
        "top": top,
    });
    report.fact("logs", logs);
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE_KEY: &str = "0000aaaa1111bbbb2222cccc3333dddd";

    #[test]
    fn reads_the_level_of_each_app() {
        assert_eq!(
            level("2026-11-26T12:51:56Z [Warn] FetchAndParseRssService: No available indexers."),
            Some(Level::Warning)
        );
        assert_eq!(
            level("26-Nov-2026 07:12:34,000 ERROR: Too Many Requests [hc.py:609]"),
            Some(Level::Error)
        );
        assert_eq!(level("[Info] RssSyncService: Starting RSS Sync"), None);
    }

    #[test]
    fn masks_a_key_in_a_url() {
        let (masked, leaked) = mask(&format!("GET /1/api?t=caps&apikey={FAKE_KEY}: 429"));
        assert!(leaked);
        assert_eq!(masked, "GET /1/api?t=caps&apikey=***: 429");
    }

    #[test]
    fn a_removed_key_is_no_leak() {
        let (masked, leaked) = mask("GET /1/api?t=caps&apikey=(removed) 429");
        assert!(!leaked);
        assert_eq!(masked, "GET /1/api?t=caps&apikey=(removed) 429");
    }

    #[test]
    fn groups_messages_that_differ_by_numbers_only() {
        let mut report = Report::default();
        let lines = vec![
            "2026-11-26T03:00:00.1Z [Warn] Indexer is disabled till 11/26/2026 04:26:23"
                .to_string(),
            "2026-11-26T05:00:00.1Z [Warn] Indexer is disabled till 11/26/2026 07:28:40"
                .to_string(),
            "2026-11-26T05:00:01.1Z [Info] RSS Sync Completed".to_string(),
        ];
        summarize(&mut report, "sonarr", &lines);
        let logs = &report.facts["logs"]["sonarr"];
        assert_eq!(logs["warnings"], 2);
        assert_eq!(logs["top"][0]["count"], 2);
        assert_eq!(logs["top"][0]["last"], "2026-11-26T05:00:00.1Z");
        assert!(report.findings.is_empty());
    }

    #[test]
    fn a_secret_in_the_logs_is_flagged_without_showing_it() {
        let mut report = Report::default();
        let lines = vec![format!(
            "2026-11-26T03:00:00.1Z [Warn] HTTP Error ?apikey={FAKE_KEY}"
        )];
        summarize(&mut report, "sonarr", &lines);
        assert!(report.flagged(Severity::High, "apikey=***"));
        assert!(!report.findings[0].detail.contains(FAKE_KEY));
    }
}
