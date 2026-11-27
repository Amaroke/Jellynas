use crate::compose::{re, read, root};

const KAVITA_COOKIE_DAYS: u64 = 7;
const ACCESS_TOKEN_HOURS: u64 = 1;

fn configuration() -> String {
    read(&root().join("services/authelia/configuration.yml"))
}

fn client(id: &str) -> Vec<String> {
    let text = configuration();
    let start = format!("      - client_id: {id}");
    let mut lines = text.lines().skip_while(|line| *line != start);
    let first = lines.next().unwrap_or_else(|| panic!("no client {id}"));
    std::iter::once(first)
        .chain(lines.take_while(|line| line.starts_with("        ")))
        .map(str::to_string)
        .collect()
}

fn list(lines: &[String], key: &str) -> Vec<String> {
    let header = format!("        {key}:");
    lines
        .iter()
        .skip_while(|line| **line != header)
        .skip(1)
        .take_while(|line| line.starts_with("          - "))
        .map(|line| line.trim_start_matches("          - ").to_string())
        .collect()
}

fn value(lines: &[String], key: &str) -> Option<String> {
    let prefix = format!("        {key}: ");
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(|found| found.trim_matches(['\'', '"']).to_string())
}

fn lifespan(name: &str, token: &str) -> Option<u64> {
    let text = configuration();
    let block = re(&format!(
        r"(?m)^        {}:\n((?:          .*\n)+)",
        regex::escape(name)
    ));
    let found = block.captures(&text)?;
    let line = re(&format!(
        r"(?m)^          {}: (\d+)([hd])$",
        regex::escape(token)
    ));
    let duration = line.captures(&found[1])?;
    let amount: u64 = duration[1].parse().unwrap();
    Some(if &duration[2] == "d" {
        amount * 24
    } else {
        amount
    })
}

#[test]
fn kavita_asks_an_authorization_code_for_its_offline_access() {
    let kavita = client("kavita");
    assert!(list(&kavita, "scopes").contains(&"offline_access".to_string()));
    assert_eq!(list(&kavita, "response_types"), ["code"]);
}

#[test]
fn the_refresh_token_of_kavita_outlives_its_session_cookie() {
    let kavita = client("kavita");
    let name = value(&kavita, "lifespan").expect("kavita has no lifespan");
    let hours = lifespan(&name, "refresh_token").expect("no refresh_token lifespan");
    assert!(
        hours >= KAVITA_COOKIE_DAYS * 24 + ACCESS_TOKEN_HOURS,
        "{hours}h"
    );
}
