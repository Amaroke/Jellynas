use crate::publish::Discord;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;

pub const GITHUB: &str = "https://api.github.com/repos/Amaroke/Jellynas/issues";
pub const ADMIN_LABEL: &str = "from:admin";
pub const FRIEND_LABEL: &str = "from:friend";
const TITLE: &str = "📋 Issues ouvertes";
const COLOR: u32 = 0x3FB950;
const DESCRIPTION_LIMIT: usize = 4000;

static ASKED_BY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^Asked by ([^.\n]+)").expect("valid regex"));

#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub friend: Option<String>,
}

#[derive(Debug, PartialEq)]
enum Closure {
    Done,
    Dropped,
}

pub fn asker(body: &str) -> Option<String> {
    ASKED_BY
        .captures(body)
        .map(|found| found[1].trim().to_string())
        .filter(|name| !name.is_empty() && name != "the Admin" && name != "a Friend")
}

pub fn people(text: &str) -> HashMap<String, HashMap<String, String>> {
    let mut found: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut section = String::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = name.trim().to_string();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            found.entry(section.clone()).or_default().insert(
                key.trim().trim_matches('"').to_string(),
                value.trim().trim_matches('"').to_string(),
            );
        }
    }
    found
}

pub struct Board<'a> {
    pub github: String,
    pub hook: Option<String>,
    pub state: PathBuf,
    pub people: PathBuf,
    pub http: &'a dyn Discord,
}

impl Board<'_> {
    fn friends(&self) -> HashMap<String, String> {
        let text = fs::read_to_string(&self.people).unwrap_or_default();
        people(&text)
            .remove("friends")
            .unwrap_or_default()
            .into_iter()
            .map(|(name, id)| (name.to_lowercase(), id))
            .collect()
    }

    fn get(&self, url: &str) -> Result<Value, String> {
        let (status, reply) = self.http.send("GET", url, None)?;
        if (200..300).contains(&status) {
            Ok(reply)
        } else {
            Err(format!("GitHub answered {status} to {url}: {reply}"))
        }
    }

    fn fetch(&self) -> Result<Vec<Issue>, String> {
        let mut issues = Vec::new();
        for label in [ADMIN_LABEL, FRIEND_LABEL] {
            let url = format!(
                "{}?state=open&per_page=100&labels={}",
                self.github,
                label.replace(':', "%3A")
            );
            let reply = self.get(&url)?;
            let list = reply
                .as_array()
                .ok_or(format!("GitHub answered no list to {url}"))?;
            for item in list
                .iter()
                .filter(|item| item.get("pull_request").is_none())
            {
                let number = item["number"].as_u64().ok_or("an issue has no number")?;
                if issues.iter().any(|issue: &Issue| issue.number == number) {
                    continue;
                }
                issues.push(Issue {
                    number,
                    title: item["title"].as_str().unwrap_or_default().to_string(),
                    url: item["html_url"].as_str().unwrap_or_default().to_string(),
                    friend: (label == FRIEND_LABEL).then(|| {
                        asker(item["body"].as_str().unwrap_or_default())
                            .unwrap_or("un Friend".into())
                    }),
                });
            }
        }
        issues.sort_by_key(|issue| issue.number);
        Ok(issues)
    }

    fn closure(&self, number: u64) -> Result<Option<Closure>, String> {
        let reply = self.get(&format!("{}/{number}", self.github))?;
        if reply["state"].as_str() != Some("closed") {
            return Ok(None);
        }
        let wontfix = reply["labels"]
            .as_array()
            .is_some_and(|labels| labels.iter().any(|label| label["name"] == "wontfix"));
        Ok(Some(
            if reply["state_reason"].as_str() == Some("completed") && !wontfix {
                Closure::Done
            } else {
                Closure::Dropped
            },
        ))
    }

    fn saved(&self) -> Map<String, Value> {
        fs::read(&self.state)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default()
    }

    fn save(&self, saved: &Map<String, Value>) -> Result<(), String> {
        if let Some(parent) = self.state.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let text = serde_json::to_string_pretty(saved).map_err(|error| error.to_string())?;
        fs::write(&self.state, text + "\n").map_err(|error| error.to_string())
    }

    fn send(&self, method: &str, url: &str, body: &Value) -> Result<(u16, Value), String> {
        self.http.send(method, url, Some(body))
    }

    fn post(&self, hook: &str, body: &Value) -> Result<String, String> {
        let (status, reply) = self.send("POST", &format!("{hook}?wait=true"), body)?;
        match (status, reply["id"].as_str()) {
            (200..300, Some(id)) => Ok(id.to_string()),
            _ => Err(format!("Discord answered {status} to the post: {reply}")),
        }
    }

    fn list(
        &self,
        hook: &str,
        issues: &[Issue],
        saved: &mut Map<String, Value>,
    ) -> Result<String, String> {
        let body = list_body(issues, &self.friends());
        if saved.get("list") == Some(&body) {
            return Ok("The list of issues is unchanged".into());
        }
        let message = saved
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string);
        let outcome = match message {
            Some(id) => {
                let (status, reply) =
                    self.send("PATCH", &format!("{hook}/messages/{id}"), &body)?;
                match status {
                    200..300 => "Edited the list of issues",
                    404 => {
                        saved.insert("message".into(), json!(self.post(hook, &body)?));
                        "Posted the list of issues again, its message was gone"
                    }
                    _ => return Err(format!("Discord answered {status} to the edit: {reply}")),
                }
            }
            None => {
                saved.insert("message".into(), json!(self.post(hook, &body)?));
                "Posted the list of issues"
            }
        };
        saved.insert("list".into(), body);
        self.save(saved)?;
        Ok(outcome.into())
    }

    fn notify(&self, hook: &str, issue: &Issue, text: &str) -> Result<(), String> {
        let name = issue.friend.clone().unwrap_or_default();
        let id = self.friends().get(&name.to_lowercase()).cloned();
        let who = id.as_ref().map(|id| format!("<@{id}>")).unwrap_or(name);
        let body = json!({
            "content": format!("{who}, ta demande {text} : [#{} {}](<{}>)", issue.number, issue.title, issue.url),
            "allowed_mentions": {"parse": [], "users": id.into_iter().collect::<Vec<_>>()},
        });
        self.post(hook, &body).map(|_| ())
    }

    pub fn run(&self) -> Result<Vec<String>, String> {
        let hook = self.hook.as_deref().ok_or(
            "DISCORD_ISSUES_WEBHOOK_ID and DISCORD_ISSUES_WEBHOOK_TOKEN are not set in .env",
        )?;
        let open = self.fetch()?;
        let mut saved = self.saved();
        let mut log = vec![self.list(hook, &open, &mut saved)?];
        let mut known: Map<String, Value> = saved
            .get("issues")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for issue in &open {
            let key = issue.number.to_string();
            if known.contains_key(&key) {
                continue;
            }
            if issue.friend.is_some() {
                self.notify(hook, issue, "est notée 📝")?;
                log.push(format!("Told the Friend that #{} is noted", issue.number));
            }
            known.insert(
                key,
                json!({"title": issue.title, "url": issue.url, "friend": issue.friend}),
            );
            saved.insert("issues".into(), Value::Object(known.clone()));
            self.save(&saved)?;
        }
        let gone: Vec<(String, Value)> = known
            .iter()
            .filter(|(key, _)| !open.iter().any(|issue| &issue.number.to_string() == *key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        for (key, value) in gone {
            let number: u64 = key.parse().unwrap_or_default();
            let closure = self.closure(number)?;
            let issue = Issue {
                number,
                title: value["title"].as_str().unwrap_or_default().to_string(),
                url: value["url"].as_str().unwrap_or_default().to_string(),
                friend: value["friend"].as_str().map(str::to_string),
            };
            if issue.friend.is_some() {
                match closure {
                    Some(Closure::Done) => {
                        self.notify(hook, &issue, "est faite ✅")?;
                        log.push(format!("Told the Friend that #{number} is done"));
                    }
                    Some(Closure::Dropped) => {
                        self.notify(hook, &issue, "ne sera pas faite 🚫")?;
                        log.push(format!("Told the Friend that #{number} will not be done"));
                    }
                    None => {}
                }
            }
            known.remove(&key);
            saved.insert("issues".into(), Value::Object(known.clone()));
            self.save(&saved)?;
        }
        Ok(log)
    }
}

fn section(title: &str, issues: &[&Issue], friends: &HashMap<String, String>) -> Vec<String> {
    let mut lines = vec![format!("**{title}**")];
    if issues.is_empty() {
        lines.push("- Aucune".into());
    }
    for issue in issues {
        let mut line = format!("- [#{} {}]({})", issue.number, issue.title, issue.url);
        if let Some(name) = &issue.friend {
            let who = friends
                .get(&name.to_lowercase())
                .map(|id| format!("<@{id}>"))
                .unwrap_or(name.clone());
            line.push_str(&format!(" · {who}"));
        }
        lines.push(line);
    }
    lines
}

fn fit(lines: Vec<String>) -> String {
    let mut text = String::new();
    for (index, line) in lines.iter().enumerate() {
        let next = text.chars().count() + line.chars().count() + 1;
        if next > DESCRIPTION_LIMIT {
            let left = lines[index..]
                .iter()
                .filter(|line| line.starts_with("- [#"))
                .count();
            text.push_str(&format!("- … et {left} autres"));
            return text;
        }
        text.push_str(line);
        text.push('\n');
    }
    text.trim_end().to_string()
}

pub fn list_body(issues: &[Issue], friends: &HashMap<String, String>) -> Value {
    let admin: Vec<&Issue> = issues
        .iter()
        .filter(|issue| issue.friend.is_none())
        .collect();
    let asked: Vec<&Issue> = issues
        .iter()
        .filter(|issue| issue.friend.is_some())
        .collect();
    let mut lines = section("Admin", &admin, friends);
    lines.push(String::new());
    lines.extend(section("Friends", &asked, friends));
    json!({
        "content": "",
        "embeds": [{"title": TITLE, "description": fit(lines), "color": COLOR}],
        "allowed_mentions": {"parse": []},
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    const HOOK: &str = "https://discord/1/a";
    const NIIXE: &str = "222222222222222222";

    #[derive(Default)]
    struct Fake {
        open: Mutex<Vec<Value>>,
        closed: Mutex<HashMap<u64, Value>>,
        calls: Mutex<Vec<(String, String, Option<Value>)>>,
        gone: Mutex<bool>,
    }

    impl Discord for Fake {
        fn send(
            &self,
            method: &str,
            url: &str,
            body: Option<&Value>,
        ) -> Result<(u16, Value), String> {
            self.calls
                .lock()
                .unwrap()
                .push((method.into(), url.into(), body.cloned()));
            if let Some(query) = url.strip_prefix("https://github?") {
                let label = if query.contains("from%3Afriend") {
                    FRIEND_LABEL
                } else {
                    ADMIN_LABEL
                };
                let found: Vec<Value> = self
                    .open
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|issue| issue["labels"][0]["name"] == label)
                    .cloned()
                    .collect();
                return Ok((200, json!(found)));
            }
            if let Some(number) = url.strip_prefix("https://github/") {
                let number: u64 = number.parse().unwrap();
                return Ok(match self.closed.lock().unwrap().get(&number) {
                    Some(issue) => (200, issue.clone()),
                    None => (200, json!({"state": "open"})),
                });
            }
            if method == "PATCH" && *self.gone.lock().unwrap() {
                return Ok((404, json!({})));
            }
            let count = self.calls.lock().unwrap().len();
            Ok((200, json!({"id": format!("m{count}")})))
        }
    }

    impl Fake {
        fn open(&self, number: u64, label: &str, body: &str) {
            self.open.lock().unwrap().push(json!({
                "number": number,
                "title": format!("Issue {number}"),
                "html_url": format!("https://github.com/i/{number}"),
                "body": body,
                "labels": [{"name": label}],
            }));
        }

        fn close(&self, number: u64, reason: &str, labels: &[&str]) {
            self.open
                .lock()
                .unwrap()
                .retain(|issue| issue["number"] != number);
            let labels: Vec<Value> = labels.iter().map(|name| json!({"name": name})).collect();
            self.closed.lock().unwrap().insert(
                number,
                json!({"state": "closed", "state_reason": reason, "labels": labels}),
            );
        }

        fn posts(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(method, url, _)| method != "GET" && url.starts_with(HOOK))
                .map(|(method, _, body)| {
                    let body = body.clone().unwrap_or_default();
                    match body["content"].as_str() {
                        Some(text) if !text.is_empty() => format!("{method} {text}"),
                        _ => format!("{method} list"),
                    }
                })
                .collect()
        }

        fn clear(&self) {
            self.calls.lock().unwrap().clear();
        }
    }

    struct Setup {
        folder: TempDir,
        fake: Fake,
    }

    impl Setup {
        fn new() -> Setup {
            let folder = TempDir::new().unwrap();
            fs::write(
                folder.path().join("friends.toml"),
                format!("[admin]\namaroke = \"1\"\n\n[friends]\nNiixe = \"{NIIXE}\"\n"),
            )
            .unwrap();
            Setup {
                folder,
                fake: Fake::default(),
            }
        }

        fn board(&self) -> Board<'_> {
            Board {
                github: "https://github".into(),
                hook: Some(HOOK.into()),
                state: self.folder.path().join("issues.json"),
                people: self.folder.path().join("friends.toml"),
                http: &self.fake,
            }
        }

        fn run(&self) -> Vec<String> {
            self.fake.clear();
            self.board().run().unwrap();
            self.fake.posts()
        }
    }

    fn issue(number: u64, friend: Option<&str>) -> Issue {
        Issue {
            number,
            title: format!("Issue {number}"),
            url: format!("https://github.com/i/{number}"),
            friend: friend.map(str::to_string),
        }
    }

    #[test]
    fn reads_the_name_of_the_friend_from_the_body() {
        assert_eq!(Some("Niixe".to_string()), asker("Asked by Niixe.\n\nMore"));
        assert_eq!(
            Some("moutondu".to_string()),
            asker("Intro\nAsked by moutondu")
        );
        assert_eq!(None, asker("Asked by the Admin. Post the issues."));
        assert_eq!(None, asker("Asked by a Friend. Show the history."));
        assert_eq!(
            Some("Niixe".to_string()),
            asker("Asked by Niixe. Show the history.")
        );
        assert_eq!(None, asker("Proposed by #422, kept by the Admin."));
    }

    #[test]
    fn reads_the_table_of_people() {
        let found = people(
            "# ids\n[admin]\namaroke = \"1\"\n\n[friends]\nNiixe = \"2\"\n\"mou ton\" = \"3\"\n",
        );
        assert_eq!(Some(&"1".to_string()), found["admin"].get("amaroke"));
        assert_eq!(Some(&"2".to_string()), found["friends"].get("Niixe"));
        assert_eq!(Some(&"3".to_string()), found["friends"].get("mou ton"));
    }

    #[test]
    fn the_list_splits_admin_and_friends_and_tags_known_friends() {
        let friends = HashMap::from([("niixe".to_string(), NIIXE.to_string())]);
        let body = list_body(
            &[
                issue(1, None),
                issue(2, Some("Niixe")),
                issue(3, Some("Inconnu")),
            ],
            &friends,
        );
        assert_eq!(
            format!(
                "**Admin**\n- [#1 Issue 1](https://github.com/i/1)\n\n**Friends**\n- [#2 Issue 2](https://github.com/i/2) · <@{NIIXE}>\n- [#3 Issue 3](https://github.com/i/3) · Inconnu"
            ),
            body["embeds"][0]["description"]
        );
        assert_eq!(json!({"parse": []}), body["allowed_mentions"]);
    }

    #[test]
    fn an_empty_section_says_so() {
        let body = list_body(&[], &HashMap::new());
        assert_eq!(
            "**Admin**\n- Aucune\n\n**Friends**\n- Aucune",
            body["embeds"][0]["description"]
        );
    }

    #[test]
    fn a_long_list_stays_within_discord() {
        let many: Vec<Issue> = (1..400).map(|number| issue(number, None)).collect();
        let text = list_body(&many, &HashMap::new())["embeds"][0]["description"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(text.chars().count() <= 4096);
        assert!(text.ends_with("autres"));
    }

    #[test]
    fn the_first_run_posts_the_list_then_tells_each_friend() {
        let setup = Setup::new();
        setup.fake.open(1, ADMIN_LABEL, "Asked by the Admin.");
        setup.fake.open(2, FRIEND_LABEL, "Asked by Niixe.");
        setup.fake.open(3, FRIEND_LABEL, "Asked by Inconnu.");
        assert_eq!(
            vec![
                "POST list".to_string(),
                format!(
                    "POST <@{NIIXE}>, ta demande est notée 📝 : [#2 Issue 2](<https://github.com/i/2>)"
                ),
                "POST Inconnu, ta demande est notée 📝 : [#3 Issue 3](<https://github.com/i/3>)"
                    .to_string(),
            ],
            setup.run()
        );
        let notes: Vec<Value> = setup
            .fake
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(_, _, body)| body.clone())
            .filter(|body| body["content"] != "")
            .map(|body| body["allowed_mentions"].clone())
            .collect();
        assert_eq!(json!({"parse": [], "users": [NIIXE]}), notes[0]);
        assert_eq!(json!({"parse": [], "users": []}), notes[1]);
    }

    #[test]
    fn a_run_without_change_sends_nothing() {
        let setup = Setup::new();
        setup.fake.open(2, FRIEND_LABEL, "Asked by Niixe.");
        setup.run();
        assert_eq!(Vec::<String>::new(), setup.run());
    }

    #[test]
    fn a_new_issue_edits_the_list_and_tells_only_its_friend() {
        let setup = Setup::new();
        setup.fake.open(2, FRIEND_LABEL, "Asked by Niixe.");
        setup.run();
        setup.fake.open(4, ADMIN_LABEL, "Asked by the Admin.");
        assert_eq!(vec!["PATCH list".to_string()], setup.run());
        setup.fake.open(5, FRIEND_LABEL, "Asked by Niixe.");
        let posts = setup.run();
        assert_eq!(2, posts.len());
        assert!(posts[1].contains("#5"));
    }

    #[test]
    fn a_closed_issue_tells_the_friend_whether_it_is_done() {
        let setup = Setup::new();
        for number in [2, 3, 4] {
            setup.fake.open(number, FRIEND_LABEL, "Asked by Niixe.");
        }
        setup.fake.open(5, ADMIN_LABEL, "Asked by the Admin.");
        setup.run();
        setup.fake.close(2, "completed", &[]);
        setup.fake.close(3, "not_planned", &[]);
        setup.fake.close(4, "completed", &["wontfix"]);
        setup.fake.close(5, "completed", &[]);
        let posts = setup.run();
        assert_eq!(4, posts.len());
        assert_eq!("PATCH list", posts[0]);
        assert!(posts[1].contains("ta demande est faite ✅ : [#2"));
        assert!(posts[2].contains("ta demande ne sera pas faite 🚫 : [#3"));
        assert!(posts[3].contains("ta demande ne sera pas faite 🚫 : [#4"));
        assert_eq!(Vec::<String>::new(), setup.run());
    }

    #[test]
    fn an_issue_that_loses_its_label_leaves_without_a_message() {
        let setup = Setup::new();
        setup.fake.open(2, FRIEND_LABEL, "Asked by Niixe.");
        setup.run();
        setup.fake.open.lock().unwrap().clear();
        assert_eq!(vec!["PATCH list".to_string()], setup.run());
    }

    #[test]
    fn a_deleted_list_is_posted_again() {
        let setup = Setup::new();
        setup.fake.open(1, ADMIN_LABEL, "");
        setup.run();
        *setup.fake.gone.lock().unwrap() = true;
        setup.fake.open(6, ADMIN_LABEL, "");
        assert_eq!(
            vec!["PATCH list".to_string(), "POST list".to_string()],
            setup.run()
        );
    }

    #[test]
    fn pull_requests_stay_off_the_list() {
        let setup = Setup::new();
        setup.fake.open.lock().unwrap().push(json!({
            "number": 9, "title": "PR", "html_url": "", "body": "",
            "labels": [{"name": ADMIN_LABEL}], "pull_request": {},
        }));
        setup.run();
        let saved = setup.board().saved();
        assert_eq!(
            "**Admin**\n- Aucune\n\n**Friends**\n- Aucune",
            saved["list"]["embeds"][0]["description"]
        );
    }

    #[test]
    fn refuses_to_run_without_the_webhook() {
        let setup = Setup::new();
        let board = Board {
            hook: None,
            ..setup.board()
        };
        assert!(
            board
                .run()
                .unwrap_err()
                .contains("DISCORD_ISSUES_WEBHOOK_ID")
        );
    }
}
