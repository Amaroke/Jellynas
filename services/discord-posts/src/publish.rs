use crate::message::{self, Message};
use jiff::ZonedDateTime;
use jiff::civil::Date;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub const PATCH_CHANNEL: &str = "patch";
pub const LIMIT: usize = 3;
const VIOLET: u32 = 0xA78BFA;
const MONTHS: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];

pub trait Discord {
    fn send(&self, method: &str, url: &str, body: Option<&Value>) -> Result<(u16, Value), String>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Post {
    pub channel: String,
    pub file: String,
    pub key: String,
    pub video: bool,
}

impl Post {
    pub fn label(&self) -> String {
        if self.video {
            format!("{} (video)", self.file)
        } else {
            self.file.clone()
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Posted,
    Edited,
    Reposted,
    Deleted,
    Skipped(String),
    Failed(String),
}

impl Outcome {
    pub fn json(&self) -> Value {
        match self {
            Outcome::Posted => json!({"outcome": "posted"}),
            Outcome::Edited => json!({"outcome": "edited"}),
            Outcome::Reposted => json!({"outcome": "reposted"}),
            Outcome::Deleted => json!({"outcome": "deleted"}),
            Outcome::Skipped(reason) => json!({"outcome": "skipped", "reason": reason}),
            Outcome::Failed(error) => json!({"outcome": "failed", "error": error}),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Entry {
    pub channel: String,
    pub file: String,
    pub outcome: Outcome,
}

pub struct Channel {
    pub name: String,
    pub posts: Vec<Post>,
    pub errors: Vec<(String, String)>,
}

pub struct Plan {
    pub actions: Vec<(Post, &'static str)>,
    pub orphans: Vec<String>,
    pub deletions: usize,
}

pub struct Publisher<'a> {
    pub folder: PathBuf,
    pub state: PathBuf,
    pub api: String,
    pub env: HashMap<String, String>,
    pub discord: &'a dyn Discord,
}

fn color(channel: &str) -> u32 {
    match channel {
        "news" => 0x34D399,
        "links" => 0x60A5FA,
        "tutos" => 0xF472B6,
        PATCH_CHANNEL => 0xFBBF24,
        _ => VIOLET,
    }
}

fn note_date(name: &str) -> Option<Date> {
    name.strip_suffix(".md")?.parse().ok()
}

fn identity(channel: &str, name: &str) -> Result<((u64, String), String), String> {
    if channel == PATCH_CHANNEL {
        note_date(name)
            .map(|date| ((0, date.to_string()), format!("{channel}/{name}")))
            .ok_or_else(|| {
                "a Patch note must be named after its date, such as 2026-11-27.md".into()
            })
    } else {
        name.split_once('-')
            .and_then(|(number, rest)| Some((number.parse::<u64>().ok()?, rest)))
            .filter(|(_, rest)| rest.len() > ".md".len())
            .map(|(number, rest)| ((number, rest.to_string()), format!("{channel}/{rest}")))
            .ok_or_else(|| "a message must be named after its place, such as 02-aurral.md".into())
    }
}

fn id_of(id: &str) -> u64 {
    id.parse().unwrap_or(u64::MAX)
}

fn others_wait(entries: &mut Vec<Entry>, posts: &[Post], reason: &str) {
    for post in posts {
        entries.push(Entry {
            channel: post.channel.clone(),
            file: post.label(),
            outcome: Outcome::Failed(reason.to_string()),
        });
    }
}

impl Publisher<'_> {
    pub fn channels(&self) -> Vec<Channel> {
        let mut names: Vec<String> = fs::read_dir(&self.folder)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
            .into_iter()
            .map(|name| {
                let mut found = Vec::new();
                let mut errors = Vec::new();
                for entry in fs::read_dir(self.folder.join(&name))
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    let file = entry.file_name().to_string_lossy().into_owned();
                    if !file.ends_with(".md") {
                        continue;
                    }
                    match identity(&name, &file) {
                        Ok((order, key)) => found.push((
                            order,
                            Post {
                                channel: name.clone(),
                                file: format!("{name}/{file}"),
                                key,
                                video: false,
                            },
                        )),
                        Err(error) => errors.push((format!("{name}/{file}"), error)),
                    }
                }
                found.sort_by(|a, b| a.0.cmp(&b.0));
                let mut first: HashMap<&str, &str> = HashMap::new();
                for (_, post) in &found {
                    let earlier = *first.entry(&post.key).or_insert(&post.file);
                    if earlier != post.file {
                        errors.push((post.file.clone(), format!("{earlier} has the same name")));
                    }
                }
                let posts = found
                    .into_iter()
                    .flat_map(|(_, post)| {
                        let video = self
                            .render(&post)
                            .is_ok_and(|message| !message.content.is_empty())
                            .then(|| Post {
                                key: format!("{}#video", post.key),
                                video: true,
                                ..post.clone()
                            });
                        std::iter::once(post).chain(video)
                    })
                    .collect();
                Channel {
                    name,
                    posts,
                    errors,
                }
            })
            .collect()
    }

    fn webhook(&self, channel: &str) -> Option<String> {
        let name = channel.to_uppercase().replace('-', "_");
        let part = |part: &str| {
            self.env
                .get(&format!("DISCORD_{name}_WEBHOOK_{part}"))
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        Some(format!("{}/{}/{}", self.api, part("ID")?, part("TOKEN")?))
    }

    pub fn saved(&self) -> Map<String, Value> {
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

    fn set(&self, key: &str, id: Option<&str>) -> Result<(), String> {
        let mut saved = self.saved();
        match id {
            Some(id) => saved.insert(key.into(), json!(id)),
            None => saved.remove(key),
        };
        self.save(&saved)
    }

    pub fn message_id(&self, post: &Post) -> Option<String> {
        self.saved()
            .get(&post.key)
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    pub fn read(&self, post: &Post) -> Result<String, String> {
        fs::read_to_string(self.folder.join(&post.file)).map_err(|error| error.to_string())
    }

    pub fn render(&self, post: &Post) -> Result<Message, String> {
        let source = self.read(post)?;
        let mut rendered = message::render(&source, &self.env)?;
        if post.channel != PATCH_CHANNEL {
            return Ok(rendered);
        }
        let name = post.file.rsplit('/').next().unwrap_or_default();
        let date = note_date(name).ok_or("a Patch note must be named after its date")?;
        if message::covers(&source).is_none() {
            return Err(
                "it must start with a header that gives the last pull request it covers, covers: #<number>".into(),
            );
        }
        if rendered.embed.contains_key("title") {
            return Err("a Patch note takes its title from its date, drop its # heading".into());
        }
        let description = rendered.embed["description"].as_str().unwrap_or_default();
        if !description.starts_with("- ") {
            return Err("a Patch note is the list of what changed, it starts with a - item".into());
        }
        let month = MONTHS[usize::try_from(date.month() - 1).unwrap_or_default()];
        rendered.embed.insert(
            "title".into(),
            json!(format!(
                "Patch note du {} {month} {}",
                date.day(),
                date.year()
            )),
        );
        rendered
            .embed
            .insert("timestamp".into(), json!(format!("{date}T12:00:00Z")));
        Ok(rendered)
    }

    pub fn check(&self, channel: &Channel) -> Vec<(String, String)> {
        let mut problems = channel.errors.clone();
        for post in channel.posts.iter().filter(|post| !post.video) {
            if let Err(error) = self.render(post) {
                problems.push((post.file.clone(), error));
            }
        }
        if channel.name == PATCH_CHANNEL {
            let mut last: Option<(String, u64)> = None;
            for post in &channel.posts {
                let Some(covers) = self
                    .read(post)
                    .ok()
                    .and_then(|source| message::covers(&source))
                else {
                    continue;
                };
                if let Some((file, previous)) = &last
                    && covers <= *previous
                {
                    problems.push((
                        post.file.clone(),
                        format!(
                            "it covers up to #{covers}, which is not after #{previous} of {file}"
                        ),
                    ));
                }
                last = Some((post.file.clone(), covers));
            }
        }
        problems
    }

    pub fn plan(&self, channel: &Channel) -> Plan {
        let saved = self.saved();
        let id = |post: &Post| {
            saved
                .get(&post.key)
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let keys: Vec<&str> = channel.posts.iter().map(|post| post.key.as_str()).collect();
        let prefix = format!("{}/", channel.name);
        let orphans: Vec<String> = saved
            .keys()
            .filter(|key| key.starts_with(&prefix) && !keys.contains(&key.as_str()))
            .cloned()
            .collect();
        let mut previous = 0;
        let kept = channel
            .posts
            .iter()
            .position(|post| match id(post) {
                Some(found) if id_of(&found) > previous => {
                    previous = id_of(&found);
                    false
                }
                _ => true,
            })
            .unwrap_or(channel.posts.len());
        let actions: Vec<(Post, &'static str)> = channel
            .posts
            .iter()
            .enumerate()
            .map(|(at, post)| {
                let action = match (at < kept, id(post).is_some()) {
                    (true, _) => "edit",
                    (false, true) => "repost",
                    (false, false) => "post",
                };
                (post.clone(), action)
            })
            .collect();
        let deletions = orphans.len()
            + actions
                .iter()
                .filter(|(_, action)| *action == "repost")
                .count();
        Plan {
            actions,
            orphans,
            deletions,
        }
    }

    fn body(&self, post: &Post, now: &ZonedDateTime) -> Result<Value, String> {
        let Message { content, mut embed } = self.render(post)?;
        if post.video {
            return Ok(
                json!({"content": content, "embeds": [], "allowed_mentions": {"parse": []}}),
            );
        }
        embed.insert("color".into(), json!(color(&post.channel)));
        if post.channel != PATCH_CHANNEL {
            embed.insert(
                "footer".into(),
                json!({"text": format!("Dernière mise à jour : {} à {}", now.strftime("%d/%m/%Y"), now.strftime("%H:%M"))}),
            );
        }
        Ok(json!({"content": "", "embeds": [embed], "allowed_mentions": {"parse": []}}))
    }

    fn delete(&self, hook: &str, key: &str, id: &str) -> Result<(), String> {
        let (status, reply) =
            self.discord
                .send("DELETE", &format!("{hook}/messages/{id}"), None)?;
        if !(200..300).contains(&status) && status != 404 {
            return Err(format!(
                "Discord answered {status} to the deletion: {reply}"
            ));
        }
        self.set(key, None)
    }

    fn post(&self, hook: &str, post: &Post, body: &Value) -> Result<(), String> {
        let (status, reply) =
            self.discord
                .send("POST", &format!("{hook}?wait=true"), Some(body))?;
        match (status, reply["id"].as_str()) {
            (200..300, Some(id)) => self.set(&post.key, Some(id)),
            _ => Err(format!("Discord answered {status} to the post: {reply}")),
        }
    }

    pub fn sync(&self, channel: &Channel, now: &ZonedDateTime, force: bool) -> Vec<Entry> {
        let mut entries = Vec::new();
        let entry = |post: &Post, outcome: Outcome| Entry {
            channel: post.channel.clone(),
            file: post.label(),
            outcome,
        };
        let Some(hook) = self.webhook(&channel.name) else {
            let name = channel.name.to_uppercase().replace('-', "_");
            let reason = format!(
                "DISCORD_{name}_WEBHOOK_ID and DISCORD_{name}_WEBHOOK_TOKEN are not set in .env"
            );
            for post in &channel.posts {
                entries.push(entry(
                    post,
                    if channel.name == PATCH_CHANNEL {
                        Outcome::Skipped(reason.clone())
                    } else {
                        Outcome::Failed(reason.clone())
                    },
                ));
            }
            return entries;
        };
        let problems = self.check(channel);
        if !problems.is_empty() {
            for (file, error) in &problems {
                entries.push(Entry {
                    channel: channel.name.clone(),
                    file: file.clone(),
                    outcome: Outcome::Failed(error.clone()),
                });
            }
            let blocked: Vec<Post> = channel
                .posts
                .iter()
                .filter(|post| !problems.iter().any(|(file, _)| *file == post.file))
                .cloned()
                .collect();
            others_wait(
                &mut entries,
                &blocked,
                &format!(
                    "{} must be fixed first, the channel keeps its order",
                    problems[0].0
                ),
            );
            return entries;
        }
        let plan = self.plan(channel);
        let mut tail = plan
            .actions
            .iter()
            .position(|(_, action)| *action != "edit")
            .unwrap_or(plan.actions.len());
        for (at, (post, _)) in plan.actions[..tail].iter().enumerate() {
            let sent = self.body(post, now).and_then(|body| {
                let id = self.message_id(post).unwrap_or_default();
                self.discord
                    .send("PATCH", &format!("{hook}/messages/{id}"), Some(&body))
            });
            match sent {
                Ok((200..300, _)) => entries.push(entry(post, Outcome::Edited)),
                Ok((404, _)) => {
                    if let Err(error) = self.set(&post.key, None) {
                        entries.push(entry(post, Outcome::Failed(error)));
                        return entries;
                    }
                    tail = at;
                    break;
                }
                Ok((status, reply)) => entries.push(entry(
                    post,
                    Outcome::Failed(format!("Discord answered {status} to the edit: {reply}")),
                )),
                Err(error) => entries.push(entry(post, Outcome::Failed(error))),
            }
        }
        let rest: Vec<&Post> = plan.actions[tail..].iter().map(|(post, _)| post).collect();
        let deletions = plan.orphans.len()
            + rest
                .iter()
                .filter(|post| self.message_id(post).is_some())
                .count();
        if deletions > LIMIT && !force {
            let reason = format!(
                "putting {} back in order deletes {deletions} messages, more than {LIMIT}, publish with force once the Admin agrees",
                channel.name
            );
            println!("{reason}");
            others_wait(
                &mut entries,
                &rest.into_iter().cloned().collect::<Vec<_>>(),
                &reason,
            );
            for key in &plan.orphans {
                entries.push(Entry {
                    channel: channel.name.clone(),
                    file: key.clone(),
                    outcome: Outcome::Failed(reason.clone()),
                });
            }
            return entries;
        }
        let saved = self.saved();
        for key in &plan.orphans {
            let id = saved.get(key).and_then(Value::as_str).unwrap_or_default();
            let outcome = match self.delete(&hook, key, id) {
                Ok(()) => Outcome::Deleted,
                Err(error) => Outcome::Failed(error),
            };
            entries.push(Entry {
                channel: channel.name.clone(),
                file: key.clone(),
                outcome,
            });
        }
        let stale: Vec<(&Post, String)> = rest
            .iter()
            .filter_map(|post| Some((*post, self.message_id(post)?)))
            .collect();
        for (post, id) in &stale {
            if let Err(error) = self.delete(&hook, &post.key, id) {
                let at = rest
                    .iter()
                    .position(|other| other == post)
                    .unwrap_or_default();
                entries.push(entry(post, Outcome::Failed(error)));
                let waiting: Vec<Post> =
                    rest[at + 1..].iter().map(|post| (*post).clone()).collect();
                others_wait(
                    &mut entries,
                    &waiting,
                    &format!("{} could not be deleted first", post.file),
                );
                return entries;
            }
        }
        for (at, post) in rest.iter().enumerate() {
            let reposted = stale.iter().any(|(other, _)| other == post);
            match self
                .body(post, now)
                .and_then(|body| self.post(&hook, post, &body))
            {
                Ok(()) => entries.push(entry(
                    post,
                    if reposted {
                        Outcome::Reposted
                    } else {
                        Outcome::Posted
                    },
                )),
                Err(error) => {
                    entries.push(entry(post, Outcome::Failed(error)));
                    let waiting: Vec<Post> =
                        rest[at + 1..].iter().map(|post| (*post).clone()).collect();
                    others_wait(
                        &mut entries,
                        &waiting,
                        &format!("{} must be posted first", post.file),
                    );
                    return entries;
                }
            }
        }
        entries
    }

    pub fn publish_all(&self, only: Option<&str>, now: &ZonedDateTime, force: bool) -> Vec<Entry> {
        self.channels()
            .iter()
            .filter(|channel| only.is_none_or(|name| name == channel.name))
            .flat_map(|channel| self.sync(channel, now, force))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use tempfile::TempDir;

    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<(String, String, Option<Value>)>>,
        messages: RefCell<Vec<Option<String>>>,
    }

    impl Discord for Fake {
        fn send(
            &self,
            method: &str,
            url: &str,
            body: Option<&Value>,
        ) -> Result<(u16, Value), String> {
            self.calls
                .borrow_mut()
                .push((method.into(), url.into(), body.cloned()));
            let description = body.and_then(|body| {
                body["embeds"][0]["description"]
                    .as_str()
                    .or_else(|| body["content"].as_str())
                    .map(str::to_string)
            });
            let mut messages = self.messages.borrow_mut();
            if method == "POST" {
                messages.push(description);
                return Ok((200, json!({"id": messages.len().to_string()})));
            }
            let id: usize = url.rsplit('/').next().unwrap().parse().unwrap();
            match messages.get_mut(id - 1) {
                Some(slot @ Some(_)) if method == "DELETE" => {
                    *slot = None;
                    Ok((204, Value::Null))
                }
                Some(slot @ Some(_)) => {
                    *slot = description;
                    Ok((200, json!({"id": id.to_string()})))
                }
                _ => Ok((404, json!({"message": "Unknown Message"}))),
            }
        }
    }

    impl Fake {
        fn methods(&self) -> Vec<String> {
            self.calls
                .borrow()
                .iter()
                .map(|call| call.0.clone())
                .collect()
        }

        fn shown(&self) -> Vec<String> {
            self.messages.borrow().iter().flatten().cloned().collect()
        }
    }

    struct Setup {
        folder: TempDir,
        fake: Fake,
        env: HashMap<String, String>,
    }

    impl Setup {
        fn new() -> Setup {
            let mut env = HashMap::from([
                ("DOMAIN".to_string(), "example.com".to_string()),
                ("CADDY_HTTPS_PORT".to_string(), "443".to_string()),
            ]);
            for (channel, id) in [("LINKS", "1234"), ("TUTOS", "4321"), ("PATCH", "5678")] {
                env.insert(format!("DISCORD_{channel}_WEBHOOK_ID"), id.into());
                env.insert(format!("DISCORD_{channel}_WEBHOOK_TOKEN"), "token".into());
            }
            Setup {
                folder: TempDir::new().unwrap(),
                fake: Fake::default(),
                env,
            }
        }

        fn write(&self, file: &str, text: &str) {
            let path = self.folder.path().join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn remove(&self, file: &str) {
            fs::remove_file(self.folder.path().join(file)).unwrap();
        }

        fn note(&self, name: &str, pull: u64, text: &str) {
            self.write(
                &format!("patch/{name}"),
                &format!("---\ncovers: #{pull}\n---\n\n- {text}\n"),
            );
        }

        fn publisher(&self) -> Publisher<'_> {
            Publisher {
                folder: self.folder.path().into(),
                state: self.folder.path().join("state.json"),
                api: "https://discord".into(),
                env: self.env.clone(),
                discord: &self.fake,
            }
        }

        fn run(&self) -> Vec<Entry> {
            self.publisher().publish_all(None, &now(), false)
        }

        fn forced(&self) -> Vec<Entry> {
            self.publisher().publish_all(None, &now(), true)
        }
    }

    fn now() -> ZonedDateTime {
        "2026-11-10T00:30[Europe/Paris]".parse().unwrap()
    }

    fn outcome(entries: &[Entry], file: &str) -> String {
        let found = entries.iter().find(|entry| entry.file == file).unwrap();
        match &found.outcome {
            Outcome::Failed(error) => format!("failed: {error}"),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn posts_the_message_filled_from_the_env() {
        let setup = Setup::new();
        setup.write(
            "links/01-links.md",
            "https://jellyfin.${DOMAIN}:${CADDY_HTTPS_PORT}\n",
        );
        setup.run();
        let (method, url, body) = setup.fake.calls.borrow()[0].clone();
        let body = body.unwrap();
        assert_eq!(
            ("POST", "https://discord/1234/token?wait=true"),
            (method.as_str(), url.as_str())
        );
        assert_eq!("", body["content"]);
        assert_eq!(
            json!([{"description": "https://jellyfin.example.com:443", "color": 0x60A5FA, "footer": {"text": "Dernière mise à jour : 10/11/2026 à 00:30"}}]),
            body["embeds"]
        );
        assert_eq!(json!({"parse": []}), body["allowed_mentions"]);
    }

    #[test]
    fn a_message_is_known_by_its_name_without_its_number() {
        let setup = Setup::new();
        setup.write("links/01-links.md", "links");
        setup.run();
        assert_eq!(
            json!({"links/links.md": "1"}),
            Value::Object(setup.publisher().saved())
        );
    }

    #[test]
    fn edits_the_same_message_the_next_time() {
        let setup = Setup::new();
        setup.write("links/01-links.md", "first");
        setup.run();
        setup.write("links/01-links.md", "changed");
        let entries = setup.run();
        assert_eq!("Edited", outcome(&entries, "links/01-links.md"));
        assert_eq!(
            "https://discord/1234/token/messages/1",
            setup.fake.calls.borrow()[1].1
        );
        assert_eq!(vec!["changed"], setup.fake.shown());
    }

    #[test]
    fn renumbering_a_file_keeps_its_message() {
        let setup = Setup::new();
        setup.write("links/01-links.md", "links");
        setup.run();
        setup.remove("links/01-links.md");
        setup.write("links/05-links.md", "links");
        let entries = setup.run();
        assert_eq!("Edited", outcome(&entries, "links/05-links.md"));
        assert_eq!(vec!["POST", "PATCH"], setup.fake.methods());
    }

    #[test]
    fn the_messages_of_a_channel_are_posted_in_the_order_of_their_number() {
        let setup = Setup::new();
        setup.write("tutos/10-tempus.md", "tempus");
        setup.write("tutos/2-aurral.md", "aurral");
        setup.write("tutos/1-video.md", "video");
        setup.run();
        assert_eq!(vec!["video", "aurral", "tempus"], setup.fake.shown());
    }

    #[test]
    fn a_message_inserted_before_others_reposts_the_rest_of_the_channel() {
        let setup = Setup::new();
        setup.write("tutos/01-video.md", "video");
        setup.write("tutos/03-tempus.md", "tempus");
        setup.run();
        setup.write("tutos/02-aurral.md", "aurral");
        let entries = setup.run();
        assert_eq!("Edited", outcome(&entries, "tutos/01-video.md"));
        assert_eq!("Posted", outcome(&entries, "tutos/02-aurral.md"));
        assert_eq!("Reposted", outcome(&entries, "tutos/03-tempus.md"));
        assert_eq!(vec!["video", "aurral", "tempus"], setup.fake.shown());
    }

    #[test]
    fn a_message_deleted_on_discord_is_posted_again_with_the_rest_after_it() {
        let setup = Setup::new();
        setup.write("tutos/01-video.md", "video");
        setup.write("tutos/02-aurral.md", "aurral");
        setup.write("tutos/03-tempus.md", "tempus");
        setup.run();
        setup.fake.messages.borrow_mut()[1] = None;
        let entries = setup.run();
        assert_eq!("Posted", outcome(&entries, "tutos/02-aurral.md"));
        assert_eq!("Reposted", outcome(&entries, "tutos/03-tempus.md"));
        assert_eq!(vec!["video", "aurral", "tempus"], setup.fake.shown());
    }

    #[test]
    fn a_file_removed_from_the_repo_deletes_its_message() {
        let setup = Setup::new();
        setup.write("tutos/01-video.md", "video");
        setup.write("tutos/02-aurral.md", "aurral");
        setup.run();
        setup.remove("tutos/02-aurral.md");
        let entries = setup.run();
        assert_eq!("Deleted", outcome(&entries, "tutos/aurral.md"));
        assert_eq!(vec!["video"], setup.fake.shown());
        assert_eq!(
            json!({"tutos/video.md": "1"}),
            Value::Object(setup.publisher().saved())
        );
    }

    #[test]
    fn deleting_more_than_the_limit_waits_for_force() {
        let setup = Setup::new();
        for (number, name) in [(2, "a"), (3, "b"), (4, "c"), (5, "d")] {
            setup.write(&format!("tutos/0{number}-{name}.md"), name);
        }
        setup.run();
        setup.write("tutos/01-first.md", "first");
        let entries = setup.run();
        assert!(outcome(&entries, "tutos/02-a.md").contains("more than 3"));
        assert_eq!(vec!["a", "b", "c", "d"], setup.fake.shown());
        setup.forced();
        assert_eq!(vec!["first", "a", "b", "c", "d"], setup.fake.shown());
    }

    #[test]
    fn a_channel_with_a_broken_file_is_left_alone() {
        let setup = Setup::new();
        setup.write("tutos/01-video.md", "${MISSING}");
        setup.write("tutos/02-aurral.md", "aurral");
        setup.write("tutos/tempus.md", "tempus");
        let entries = setup.run();
        assert!(setup.fake.calls.borrow().is_empty());
        assert!(outcome(&entries, "tutos/01-video.md").contains("MISSING"));
        assert!(outcome(&entries, "tutos/tempus.md").contains("named after its place"));
        assert!(outcome(&entries, "tutos/02-aurral.md").contains("must be fixed first"));
    }

    #[test]
    fn two_files_of_the_same_name_are_refused() {
        let setup = Setup::new();
        setup.write("tutos/01-aurral.md", "a");
        setup.write("tutos/02-aurral.md", "b");
        let entries = setup.run();
        assert!(outcome(&entries, "tutos/02-aurral.md").contains("same name"));
    }

    #[test]
    fn two_files_of_the_same_name_apart_are_refused() {
        let setup = Setup::new();
        setup.write("tutos/01-aurral.md", "a");
        setup.write("tutos/02-seerr.md", "b");
        setup.write("tutos/03-aurral.md", "c");
        let entries = setup.run();
        assert!(outcome(&entries, "tutos/03-aurral.md").contains("same name"));
    }

    #[test]
    fn a_channel_that_fails_does_not_stop_the_others() {
        let setup = Setup::new();
        setup.write("links/01-links.md", "links");
        setup.write("news/01-welcome.md", "welcome");
        let entries = setup.run();
        assert_eq!(vec!["links"], setup.fake.shown());
        assert!(outcome(&entries, "news/01-welcome.md").contains("DISCORD_NEWS_WEBHOOK_ID"));
    }

    #[test]
    fn the_video_of_a_tuto_is_posted_alone_under_its_embed() {
        let setup = Setup::new();
        setup.write(
            "tutos/01-jellyfin.md",
            "# Tuto\n\nIntro.\n\n📺 <https://youtu.be/x>\n\n## Next\n",
        );
        let entries = setup.run();
        assert_eq!("Posted", outcome(&entries, "tutos/01-jellyfin.md"));
        assert_eq!("Posted", outcome(&entries, "tutos/01-jellyfin.md (video)"));
        let calls = setup.fake.calls.borrow();
        let embed = calls[0].2.clone().unwrap();
        assert_eq!("", embed["content"]);
        assert_eq!("Intro.\n\n## Next", embed["embeds"][0]["description"]);
        let video = calls[1].2.clone().unwrap();
        assert_eq!("📺 https://youtu.be/x", video["content"]);
        assert_eq!(json!([]), video["embeds"]);
        assert_eq!(
            json!({"tutos/jellyfin.md": "1", "tutos/jellyfin.md#video": "2"}),
            Value::Object(setup.publisher().saved())
        );
    }

    #[test]
    fn a_video_dropped_from_a_tuto_deletes_its_message() {
        let setup = Setup::new();
        setup.write(
            "tutos/01-jellyfin.md",
            "# Tuto\n\n📺 <https://youtu.be/x>\n",
        );
        setup.run();
        setup.write("tutos/01-jellyfin.md", "# Tuto\n\nIntro.\n");
        let entries = setup.run();
        assert_eq!("Edited", outcome(&entries, "tutos/01-jellyfin.md"));
        assert_eq!("Deleted", outcome(&entries, "tutos/jellyfin.md#video"));
        assert_eq!(vec!["Intro."], setup.fake.shown());
    }

    #[test]
    fn a_channel_without_its_own_color_gets_violet() {
        let mut setup = Setup::new();
        setup
            .env
            .insert("DISCORD_FARM_WEBHOOK_ID".into(), "1".into());
        setup
            .env
            .insert("DISCORD_FARM_WEBHOOK_TOKEN".into(), "a".into());
        setup.write("farm/01-farm.md", "x");
        setup.run();
        let body = setup.fake.calls.borrow()[0].2.clone().unwrap();
        assert_eq!(json!(VIOLET), body["embeds"][0]["color"]);
    }

    #[test]
    fn publishes_a_single_channel_when_asked() {
        let setup = Setup::new();
        setup.write("links/01-links.md", "links");
        setup.note("2026-11-02.md", 30, "Première.");
        let entries = setup.publisher().publish_all(Some("patch"), &now(), false);
        assert_eq!(1, entries.len());
        assert_eq!(vec!["- Première."], setup.fake.shown());
    }

    #[test]
    fn a_note_is_an_embed_titled_by_its_date_and_dated_on_discord() {
        let setup = Setup::new();
        setup.note("2026-11-02.md", 30, "Nouveau.");
        setup.run();
        let (_, url, body) = setup.fake.calls.borrow()[0].clone();
        assert_eq!("https://discord/5678/token?wait=true", url);
        assert_eq!(
            json!([{
                "title": "Patch note du 2 novembre 2026",
                "description": "- Nouveau.",
                "color": 0xFBBF24,
                "timestamp": "2026-11-02T12:00:00Z",
            }]),
            body.unwrap()["embeds"]
        );
    }

    #[test]
    fn notes_are_posted_oldest_first() {
        let setup = Setup::new();
        setup.note("2026-11-16.md", 60, "Troisième.");
        setup.note("2026-11-02.md", 30, "Première.");
        setup.note("2026-11-09.md", 45, "Deuxième.");
        setup.run();
        assert_eq!(
            vec!["- Première.", "- Deuxième.", "- Troisième."],
            setup.fake.shown()
        );
    }

    #[test]
    fn an_older_note_added_later_reposts_the_newer_ones() {
        let setup = Setup::new();
        setup.note("2026-11-02.md", 30, "Première.");
        setup.note("2026-11-16.md", 60, "Troisième.");
        setup.run();
        setup.note("2026-11-09.md", 45, "Deuxième.");
        let entries = setup.run();
        assert_eq!("Reposted", outcome(&entries, "patch/2026-11-16.md"));
        assert_eq!(
            vec!["- Première.", "- Deuxième.", "- Troisième."],
            setup.fake.shown()
        );
    }

    #[test]
    fn a_note_without_its_last_pull_request_is_refused() {
        let setup = Setup::new();
        setup.write("patch/2026-11-02.md", "- Rien.\n");
        let entries = setup.run();
        assert!(setup.fake.calls.borrow().is_empty());
        assert!(outcome(&entries, "patch/2026-11-02.md").contains("covers"));
    }

    #[test]
    fn a_note_takes_no_heading_of_its_own() {
        let setup = Setup::new();
        setup.write(
            "patch/2026-11-02.md",
            "---\ncovers: #30\n---\n\n# Titre\n\n- Rien.\n",
        );
        assert!(outcome(&setup.run(), "patch/2026-11-02.md").contains("heading"));
    }

    #[test]
    fn a_note_is_a_list() {
        let setup = Setup::new();
        setup.write(
            "patch/2026-11-02.md",
            "---\ncovers: #30\n---\n\nUn paragraphe.\n",
        );
        assert!(outcome(&setup.run(), "patch/2026-11-02.md").contains("list"));
    }

    #[test]
    fn a_note_is_named_after_its_date() {
        let setup = Setup::new();
        setup.note("01-novembre.md", 30, "Rien.");
        assert!(outcome(&setup.run(), "patch/01-novembre.md").contains("named after its date"));
    }

    #[test]
    fn a_later_note_must_cover_later_pull_requests() {
        let setup = Setup::new();
        setup.note("2026-11-02.md", 45, "Première.");
        setup.note("2026-11-09.md", 30, "Deuxième.");
        let entries = setup.run();
        assert!(setup.fake.calls.borrow().is_empty());
        assert!(outcome(&entries, "patch/2026-11-09.md").contains("not after #45"));
    }

    #[test]
    fn without_the_webhook_the_notes_are_skipped() {
        let mut setup = Setup::new();
        setup.env.remove("DISCORD_PATCH_WEBHOOK_ID");
        setup.note("2026-11-02.md", 30, "Première.");
        let entries = setup.run();
        assert!(setup.fake.calls.borrow().is_empty());
        assert!(outcome(&entries, "patch/2026-11-02.md").starts_with("Skipped"));
    }

    #[test]
    fn the_plan_shows_what_a_run_would_do() {
        let setup = Setup::new();
        setup.write("tutos/01-video.md", "video");
        setup.write("tutos/03-tempus.md", "tempus");
        setup.write("tutos/04-old.md", "old");
        setup.run();
        setup.remove("tutos/04-old.md");
        setup.write("tutos/02-aurral.md", "aurral");
        let channel = setup.publisher().channels().remove(0);
        let plan = setup.publisher().plan(&channel);
        let actions: Vec<(&str, &str)> = plan
            .actions
            .iter()
            .map(|(post, action)| (post.file.as_str(), *action))
            .collect();
        assert_eq!(
            vec![
                ("tutos/01-video.md", "edit"),
                ("tutos/02-aurral.md", "post"),
                ("tutos/03-tempus.md", "repost")
            ],
            actions
        );
        assert_eq!(vec!["tutos/old.md"], plan.orphans);
        assert_eq!(2, plan.deletions);
    }
}
