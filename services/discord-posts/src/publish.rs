use crate::message::{self, Message};
use jiff::ZonedDateTime;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const PATCH_CHANNEL: &str = "patch";
pub const PATCH_NOTES: &str = "patch-notes";
const VIOLET: u32 = 0xA78BFA;
const POSTS: [(&str, &str); 5] = [
    ("links", "links.md"),
    ("news", "welcome.md"),
    ("tutos", "aurral-video.md"),
    ("tutos", "aurral.md"),
    ("tutos", "tempus.md"),
];

pub trait Discord {
    fn send(&self, method: &str, url: &str, body: &Value) -> Result<(u16, Value), String>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Post {
    pub channel: String,
    pub file: String,
}

impl Post {
    fn key(&self) -> String {
        let name = Path::new(&self.file)
            .file_name()
            .map_or(self.file.clone(), |name| {
                name.to_string_lossy().into_owned()
            });
        format!("{}/{name}", self.channel)
    }
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Posted,
    Edited,
    Skipped(String),
    Failed(String),
}

impl Outcome {
    pub fn json(&self) -> Value {
        match self {
            Outcome::Posted => json!({"outcome": "posted"}),
            Outcome::Edited => json!({"outcome": "edited"}),
            Outcome::Skipped(reason) => json!({"outcome": "skipped", "reason": reason}),
            Outcome::Failed(error) => json!({"outcome": "failed", "error": error}),
        }
    }
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

impl Publisher<'_> {
    pub fn posts(&self) -> Vec<Post> {
        let mut notes: Vec<String> = fs::read_dir(self.folder.join(PATCH_NOTES))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".md"))
            .collect();
        notes.sort();
        POSTS
            .iter()
            .map(|(channel, file)| Post {
                channel: (*channel).into(),
                file: (*file).into(),
            })
            .chain(notes.into_iter().map(|name| Post {
                channel: PATCH_CHANNEL.into(),
                file: format!("{PATCH_NOTES}/{name}"),
            }))
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

    pub fn message_id(&self, post: &Post) -> Option<String> {
        self.saved()
            .get(&post.key())
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    pub fn read(&self, post: &Post) -> Result<String, String> {
        fs::read_to_string(self.folder.join(&post.file)).map_err(|error| error.to_string())
    }

    pub fn render(&self, post: &Post) -> Result<Message, String> {
        let source = self.read(post)?;
        if post.channel == PATCH_CHANNEL && message::covers(&source).is_none() {
            return Err(
                "it must start with a header that gives the last pull request it covers, covers: #<number>".into(),
            );
        }
        message::render(&source, &self.env)
    }

    pub fn publish(&self, post: &Post, now: &ZonedDateTime) -> Outcome {
        let Some(hook) = self.webhook(&post.channel) else {
            let name = post.channel.to_uppercase().replace('-', "_");
            let reason = format!(
                "DISCORD_{name}_WEBHOOK_ID and DISCORD_{name}_WEBHOOK_TOKEN are not set in .env"
            );
            return if post.channel == PATCH_CHANNEL {
                Outcome::Skipped(reason)
            } else {
                Outcome::Failed(reason)
            };
        };
        match self.send(post, &hook, now) {
            Ok(outcome) => outcome,
            Err(error) => Outcome::Failed(error),
        }
    }

    fn send(&self, post: &Post, hook: &str, now: &ZonedDateTime) -> Result<Outcome, String> {
        let Message { content, mut embed } = self.render(post)?;
        embed.insert("color".into(), json!(color(&post.channel)));
        embed.insert(
            "footer".into(),
            json!({"text": format!("Dernière mise à jour : {} à {}", now.strftime("%d/%m/%Y"), now.strftime("%H:%M"))}),
        );
        let body =
            json!({"content": content, "embeds": [embed], "allowed_mentions": {"parse": []}});
        if let Some(id) = self.message_id(post) {
            let (status, reply) =
                self.discord
                    .send("PATCH", &format!("{hook}/messages/{id}"), &body)?;
            match status {
                200..300 => return Ok(Outcome::Edited),
                404 => {}
                _ => return Err(format!("Discord answered {status} to the edit: {reply}")),
            }
        }
        let (status, reply) = self
            .discord
            .send("POST", &format!("{hook}?wait=true"), &body)?;
        let id = match (status, reply["id"].as_str()) {
            (200..300, Some(id)) => id.to_string(),
            _ => return Err(format!("Discord answered {status} to the post: {reply}")),
        };
        let mut saved = self.saved();
        saved.insert(post.key(), json!(id));
        if let Some(parent) = self.state.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let text = serde_json::to_string_pretty(&saved).map_err(|error| error.to_string())?;
        fs::write(&self.state, text + "\n").map_err(|error| error.to_string())?;
        Ok(Outcome::Posted)
    }

    pub fn publish_all(&self, only: Option<&str>, now: &ZonedDateTime) -> Vec<(Post, Outcome)> {
        self.posts()
            .into_iter()
            .filter(|post| only.is_none_or(|file| file == post.file))
            .map(|post| {
                let outcome = self.publish(&post, now);
                (post, outcome)
            })
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
        calls: RefCell<Vec<(String, String, Value)>>,
        messages: RefCell<Vec<Option<String>>>,
    }

    impl Discord for Fake {
        fn send(&self, method: &str, url: &str, body: &Value) -> Result<(u16, Value), String> {
            self.calls
                .borrow_mut()
                .push((method.into(), url.into(), body.clone()));
            let description = body["embeds"][0]["description"]
                .as_str()
                .map(str::to_string);
            let mut messages = self.messages.borrow_mut();
            if method == "POST" {
                messages.push(description);
                return Ok((200, json!({"id": messages.len().to_string()})));
            }
            let id: usize = url.rsplit('/').next().unwrap().parse().unwrap();
            match messages.get_mut(id - 1) {
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

        fn descriptions(&self) -> Vec<String> {
            self.calls
                .borrow()
                .iter()
                .map(|call| {
                    call.2["embeds"][0]["description"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect()
        }
    }

    struct Setup {
        folder: TempDir,
        fake: Fake,
        env: HashMap<String, String>,
    }

    impl Setup {
        fn new() -> Setup {
            let folder = TempDir::new().unwrap();
            fs::create_dir(folder.path().join(PATCH_NOTES)).unwrap();
            Setup {
                folder,
                fake: Fake::default(),
                env: HashMap::from([
                    ("DOMAIN".into(), "example.com".into()),
                    ("CADDY_HTTPS_PORT".into(), "443".into()),
                    ("DISCORD_LINKS_WEBHOOK_ID".into(), "1234".into()),
                    ("DISCORD_LINKS_WEBHOOK_TOKEN".into(), "abcd".into()),
                    ("DISCORD_PATCH_WEBHOOK_ID".into(), "5678".into()),
                    ("DISCORD_PATCH_WEBHOOK_TOKEN".into(), "efgh".into()),
                ]),
            }
        }

        fn write(&self, file: &str, text: &str) {
            fs::write(self.folder.path().join(file), text).unwrap();
        }

        fn note(&self, name: &str, pull: u64, text: &str) {
            self.write(
                &format!("{PATCH_NOTES}/{name}"),
                &format!("---\ncovers: #{pull}\n---\n\n# Patch note\n\n{text}\n"),
            );
        }

        fn publisher(&self) -> Publisher<'_> {
            Publisher {
                folder: self.folder.path().into(),
                state: self.folder.path().join("appdata").join("state.json"),
                api: "https://discord".into(),
                env: self.env.clone(),
                discord: &self.fake,
            }
        }

        fn publish_all(&self) -> Vec<(Post, Outcome)> {
            self.publisher().publish_all(None, &now())
        }
    }

    fn now() -> ZonedDateTime {
        "2026-11-10T00:30[Europe/Paris]".parse().unwrap()
    }

    fn links() -> Post {
        Post {
            channel: "links".into(),
            file: "links.md".into(),
        }
    }

    #[test]
    fn posts_the_message_filled_from_the_env() {
        let setup = Setup::new();
        setup.write(
            "links.md",
            "https://jellyfin.${DOMAIN}:${CADDY_HTTPS_PORT}\n",
        );
        assert_eq!(Outcome::Posted, setup.publisher().publish(&links(), &now()));
        let (method, url, body) = setup.fake.calls.borrow()[0].clone();
        assert_eq!(
            ("POST", "https://discord/1234/abcd?wait=true"),
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
    fn edits_the_same_message_the_next_time() {
        let setup = Setup::new();
        setup.write("links.md", "first");
        setup.publisher().publish(&links(), &now());
        setup.write("links.md", "changed");
        assert_eq!(Outcome::Edited, setup.publisher().publish(&links(), &now()));
        assert_eq!(
            "https://discord/1234/abcd/messages/1",
            setup.fake.calls.borrow()[1].1
        );
        assert_eq!(
            vec![Some("changed".to_string())],
            *setup.fake.messages.borrow()
        );
    }

    #[test]
    fn posts_again_once_the_message_was_deleted() {
        let setup = Setup::new();
        setup.write("links.md", "first");
        setup.publisher().publish(&links(), &now());
        setup.fake.messages.borrow_mut()[0] = None;
        assert_eq!(Outcome::Posted, setup.publisher().publish(&links(), &now()));
        assert_eq!(vec!["POST", "PATCH", "POST"], setup.fake.methods());
        assert_eq!(
            json!({"links/links.md": "2"}),
            Value::Object(setup.publisher().saved())
        );
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
        setup.write("links.md", "x");
        let post = Post {
            channel: "farm".into(),
            file: "links.md".into(),
        };
        setup.publisher().publish(&post, &now());
        assert_eq!(
            json!(VIOLET),
            setup.fake.calls.borrow()[0].2["embeds"][0]["color"]
        );
    }

    #[test]
    fn a_channel_that_fails_does_not_stop_the_others() {
        let setup = Setup::new();
        setup.write("links.md", "links");
        let results = setup.publish_all();
        assert_eq!(vec!["links"], setup.fake.descriptions());
        let welcome = results
            .iter()
            .find(|(post, _)| post.file == "welcome.md")
            .unwrap();
        assert!(
            matches!(&welcome.1, Outcome::Failed(error) if error.contains("DISCORD_NEWS_WEBHOOK_ID"))
        );
    }

    #[test]
    fn a_note_is_posted_in_the_patch_channel_without_its_header() {
        let setup = Setup::new();
        setup.note("2026-11-02.md", 30, "Nouveau.");
        setup.publish_all();
        let (_, url, body) = setup.fake.calls.borrow()[0].clone();
        assert_eq!("https://discord/5678/efgh?wait=true", url);
        assert_eq!("Patch note", body["embeds"][0]["title"]);
        assert_eq!(json!(0xFBBF24), body["embeds"][0]["color"]);
    }

    #[test]
    fn notes_are_posted_oldest_first_after_the_other_messages() {
        let setup = Setup::new();
        setup.write("links.md", "links");
        setup.note("2026-11-16.md", 60, "Troisième.");
        setup.note("2026-11-02.md", 30, "Première.");
        setup.note("2026-11-09.md", 45, "Deuxième.");
        setup.publish_all();
        assert_eq!(
            vec!["links", "Première.", "Deuxième.", "Troisième."],
            setup.fake.descriptions()
        );
    }

    #[test]
    fn a_changed_note_is_edited_in_place() {
        let setup = Setup::new();
        setup.note("2026-11-02.md", 30, "Première.");
        setup.publish_all();
        setup.note("2026-11-02.md", 30, "Corrigée.");
        setup.publish_all();
        let patch: Vec<String> = setup
            .fake
            .calls
            .borrow()
            .iter()
            .map(|call| call.0.clone())
            .collect();
        assert_eq!(vec!["POST", "PATCH"], patch);
        assert_eq!(
            vec![Some("Corrigée.".to_string())],
            *setup.fake.messages.borrow()
        );
    }

    #[test]
    fn a_note_without_its_last_pull_request_is_refused() {
        let setup = Setup::new();
        setup.write(
            &format!("{PATCH_NOTES}/2026-11-02.md"),
            "# Patch note\n\nRien.\n",
        );
        let results = setup.publish_all();
        assert!(setup.fake.calls.borrow().is_empty());
        assert!(
            matches!(&results.last().unwrap().1, Outcome::Failed(error) if error.contains("covers"))
        );
    }

    #[test]
    fn without_the_webhook_the_notes_are_skipped() {
        let mut setup = Setup::new();
        setup.env.remove("DISCORD_PATCH_WEBHOOK_ID");
        setup.note("2026-11-02.md", 30, "Première.");
        let results = setup.publish_all();
        assert!(setup.fake.calls.borrow().is_empty());
        assert!(matches!(results.last().unwrap().1, Outcome::Skipped(_)));
    }

    #[test]
    fn publishes_a_single_file_when_asked() {
        let setup = Setup::new();
        setup.write("links.md", "links");
        setup.note("2026-11-02.md", 30, "Première.");
        let results = setup
            .publisher()
            .publish_all(Some("patch-notes/2026-11-02.md"), &now());
        assert_eq!(1, results.len());
        assert_eq!(vec!["Première."], setup.fake.descriptions());
    }
}
