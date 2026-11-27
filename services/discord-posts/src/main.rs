mod message;
mod publish;

use jiff::civil::Time;
use jiff::{ToSpan, ZonedDateTime};
use publish::{Discord, Outcome, PATCH_CHANNEL, Post, Publisher};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{env, process, thread};

const API: &str = "https://discord.com/api/v10/webhooks";
const USER_AGENT: &str = "DiscordBot (https://github.com/Amaroke/Jellynas, 1.0)";
const FOLDER: &str = "/app/discord-posts";
const PORT: u16 = 8000;
const ACTION_RUNNING: &str = "Publication en cours";
const ACTIONS: [(&str, &str, &str); 1] =
    [("discord", "Messages Discord", "Liens, tutos et patch notes")];
const ACTIONS_PAGE: &str = include_str!("actions.html");

struct Webhook {
    agent: ureq::Agent,
}

impl Webhook {
    fn new() -> Webhook {
        Webhook {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(30)))
                .http_status_as_error(false)
                .build()
                .into(),
        }
    }
}

impl Discord for Webhook {
    fn send(&self, method: &str, url: &str, body: &Value) -> Result<(u16, Value), String> {
        let request = ureq::http::Request::builder()
            .method(method)
            .uri(url)
            .header("Content-Type", "application/json")
            .header("User-Agent", USER_AGENT)
            .body(body.to_string())
            .map_err(|error| error.to_string())?;
        let mut response = self.agent.run(request).map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|error| error.to_string())?;
        Ok((
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        ))
    }
}

struct App {
    folder: PathBuf,
    state: PathBuf,
    api: String,
    env: HashMap<String, String>,
    discord: Box<dyn Discord + Send + Sync>,
    running: Mutex<()>,
    wake: Mutex<Sender<()>>,
}

struct Reply {
    status: u16,
    kind: &'static str,
    body: String,
    location: Option<String>,
}

impl Reply {
    fn json(status: u16, value: &Value) -> Reply {
        Reply {
            status,
            kind: "application/json",
            body: value.to_string(),
            location: None,
        }
    }

    fn empty(status: u16) -> Reply {
        Reply {
            status,
            kind: "text/plain",
            body: String::new(),
            location: None,
        }
    }
}

fn post_json(post: &Post) -> Value {
    json!({"channel": post.channel, "file": post.file})
}

impl App {
    fn publisher(&self) -> Publisher<'_> {
        Publisher {
            folder: self.folder.clone(),
            state: self.state.clone(),
            api: self.api.clone(),
            env: self.env.clone(),
            discord: self.discord.as_ref(),
        }
    }

    fn publish(&self, only: Option<&str>) -> Vec<(Post, Outcome)> {
        let _running = self
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let results = self.publisher().publish_all(only, &ZonedDateTime::now());
        for (post, outcome) in &results {
            match outcome {
                Outcome::Posted => println!("Posted {} in {}", post.file, post.channel),
                Outcome::Edited => {
                    println!("Edited the message of {} in {}", post.file, post.channel)
                }
                Outcome::Skipped(_) => {}
                Outcome::Failed(error) => {
                    println!("Cannot publish {} in {}: {error}", post.file, post.channel)
                }
            }
        }
        results
    }

    fn posts(&self) -> Value {
        let publisher = self.publisher();
        let posts: Vec<Value> = publisher
            .posts()
            .iter()
            .map(|post| {
                let mut entry = post_json(post);
                entry["message"] = json!(publisher.message_id(post));
                if post.channel == PATCH_CHANNEL {
                    entry["covers"] = json!(
                        publisher
                            .read(post)
                            .ok()
                            .and_then(|source| message::covers(&source))
                    );
                }
                entry
            })
            .collect();
        json!({"posts": posts})
    }

    fn route(&self, method: &str, url: &str, accept: &str) -> Reply {
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        let param = |name: &str| {
            query
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .find(|(key, _)| *key == name)
                .map(|(_, value)| decode(value))
        };
        match (method, path) {
            ("GET", "/actions.json") => {
                let actions: Vec<Value> = ACTIONS
                    .iter()
                    .map(|(key, label, hint)| json!({"key": key, "label": label, "hint": hint}))
                    .collect();
                Reply::json(200, &json!({"actions": actions}))
            }
            ("GET", "/actions") => Reply {
                status: 200,
                kind: "text/html; charset=utf-8",
                body: actions_page(param("done").as_deref().unwrap_or("")),
                location: None,
            },
            ("POST", "/actions/discord") => {
                let _ = self
                    .wake
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .send(());
                if accept.contains("application/json") {
                    Reply::empty(202)
                } else {
                    Reply {
                        location: Some("/actions?done=discord".into()),
                        ..Reply::empty(303)
                    }
                }
            }
            ("GET", "/api/posts") => Reply::json(200, &self.posts()),
            ("POST", "/api/publish") => {
                let only = param("file");
                if let Some(file) = &only
                    && !self
                        .publisher()
                        .posts()
                        .iter()
                        .any(|post| &post.file == file)
                {
                    return Reply::json(
                        404,
                        &json!({"error": format!("{file} is no message of discord-posts")}),
                    );
                }
                let results: Vec<Value> = self
                    .publish(only.as_deref())
                    .iter()
                    .map(|(post, outcome)| {
                        let mut entry = post_json(post);
                        if let (Value::Object(entry), Value::Object(outcome)) =
                            (&mut entry, outcome.json())
                        {
                            entry.extend(outcome);
                        }
                        entry
                    })
                    .collect();
                Reply::json(200, &json!({"results": results}))
            }
            _ => Reply::empty(404),
        }
    }
}

fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = bytes
            .get(index + 1..index + 3)
            .filter(|_| bytes[index] == b'%')
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (bytes[index], escaped) {
            (_, Some(byte)) => {
                out.push(byte);
                index += 2;
            }
            (b'+', None) => out.push(b' '),
            (byte, None) => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn action_card(key: &str, label: &str, hint: &str, done: &str) -> String {
    let (state, text) = if done == key {
        (" running", ACTION_RUNNING)
    } else {
        ("", hint)
    };
    format!(
        r#"<form method="post" action="/actions/{key}"><button class="{key}{state}"><span class="label">{label}</span><span class="value">Actualiser</span><span class="sub">{text}</span></button></form>"#
    )
}

fn actions_page(done: &str) -> String {
    let cards: String = ACTIONS
        .iter()
        .map(|(key, label, hint)| action_card(key, label, hint, done))
        .collect();
    ACTIONS_PAGE.replace("{cards}", &cards)
}

fn next_run(now: &ZonedDateTime) -> ZonedDateTime {
    let today = now
        .with()
        .time(Time::constant(0, 30, 0, 0))
        .build()
        .unwrap_or_else(|_| now.clone());
    if &today > now {
        today
    } else {
        today.checked_add(1.day()).unwrap_or(today)
    }
}

fn serve(app: Arc<App>) -> Result<(), String> {
    let server = tiny_http::Server::http(("0.0.0.0", PORT)).map_err(|error| error.to_string())?;
    thread::spawn(move || {
        for request in server.incoming_requests() {
            let app = Arc::clone(&app);
            thread::spawn(move || {
                let accept = request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv("Accept"))
                    .map(|header| header.value.to_string())
                    .unwrap_or_default();
                let reply = app.route(request.method().as_str(), request.url(), &accept);
                let mut response = tiny_http::Response::from_string(reply.body)
                    .with_status_code(reply.status)
                    .with_header(
                        tiny_http::Header::from_bytes("Content-Type", reply.kind)
                            .expect("valid header"),
                    );
                if reply.kind.starts_with("text/html") {
                    response.add_header(
                        tiny_http::Header::from_bytes(
                            "Content-Security-Policy",
                            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'",
                        )
                        .expect("valid header"),
                    );
                }
                if let Some(location) = reply.location
                    && let Ok(header) = tiny_http::Header::from_bytes("Location", location)
                {
                    response.add_header(header);
                }
                let _ = request.respond(response);
            });
        }
    });
    Ok(())
}

fn daily(app: &App, wake: &Receiver<()>) {
    loop {
        let now = ZonedDateTime::now();
        let wait = now
            .duration_until(&next_run(&now))
            .try_into()
            .unwrap_or(Duration::from_secs(60));
        if let Err(RecvTimeoutError::Disconnected) = wake.recv_timeout(wait) {
            return;
        }
        app.publish(None);
    }
}

fn run() -> Result<(), String> {
    let env: HashMap<String, String> = env::vars().collect();
    let appdata = env
        .get("APPDATA_ROOT")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or("APPDATA_ROOT is empty")?;
    let (sender, receiver) = mpsc::channel();
    let app = Arc::new(App {
        folder: PathBuf::from(
            env.get("DISCORD_POSTS_FOLDER")
                .cloned()
                .unwrap_or(FOLDER.into()),
        ),
        state: PathBuf::from(appdata)
            .join("discord-posts")
            .join("state.json"),
        api: API.into(),
        env,
        discord: Box::new(Webhook::new()),
        running: Mutex::new(()),
        wake: Mutex::new(sender),
    });
    serve(Arc::clone(&app))?;
    daily(&app, &receiver);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<String>>,
    }

    impl Discord for Fake {
        fn send(&self, method: &str, url: &str, _: &Value) -> Result<(u16, Value), String> {
            let mut calls = self.calls.lock().unwrap();
            calls.push(format!("{method} {url}"));
            Ok((200, json!({"id": calls.len().to_string()})))
        }
    }

    fn app(folder: &TempDir) -> (App, Receiver<()>) {
        let (sender, receiver) = mpsc::channel();
        fs::create_dir_all(folder.path().join("patch-notes")).unwrap();
        fs::write(
            folder.path().join("patch-notes/2026-11-02.md"),
            "---\ncovers: #30\n---\n\n- Nouveau.\n",
        )
        .unwrap();
        let env = HashMap::from([
            ("DISCORD_PATCH_WEBHOOK_ID".to_string(), "1".to_string()),
            ("DISCORD_PATCH_WEBHOOK_TOKEN".to_string(), "a".to_string()),
        ]);
        let app = App {
            folder: folder.path().into(),
            state: folder.path().join("state.json"),
            api: "https://discord".into(),
            env,
            discord: Box::new(Fake::default()),
            running: Mutex::new(()),
            wake: Mutex::new(sender),
        };
        (app, receiver)
    }

    fn body(reply: &Reply) -> Value {
        serde_json::from_str(&reply.body).unwrap()
    }

    #[test]
    fn runs_today_before_half_past_midnight() {
        let now: ZonedDateTime = "2026-11-10T00:29[Europe/Paris]".parse().unwrap();
        assert_eq!(
            "2026-11-10T00:30:00+01:00[Europe/Paris]",
            next_run(&now).to_string()
        );
    }

    #[test]
    fn runs_tomorrow_from_half_past_midnight() {
        for time in [
            "2026-11-10T00:30[Europe/Paris]",
            "2026-11-10T18:00[Europe/Paris]",
        ] {
            let now: ZonedDateTime = time.parse().unwrap();
            assert_eq!(
                "2026-11-11T00:30:00+01:00[Europe/Paris]",
                next_run(&now).to_string()
            );
        }
    }

    #[test]
    fn the_page_offers_the_button_in_post() {
        let folder = TempDir::new().unwrap();
        let (app, wake) = app(&folder);
        let reply = app.route("GET", "/actions", "");
        assert!(
            reply
                .body
                .contains(r#"<form method="post" action="/actions/discord">"#)
        );
        assert!(wake.try_recv().is_err());
    }

    #[test]
    fn the_discord_button_wakes_the_publication() {
        let folder = TempDir::new().unwrap();
        let (app, wake) = app(&folder);
        let reply = app.route("POST", "/actions/discord", "text/html");
        assert_eq!(
            (303, Some("/actions?done=discord".to_string())),
            (reply.status, reply.location)
        );
        assert!(wake.try_recv().is_ok());
        let page = app.route("GET", "/actions?done=discord", "").body;
        assert!(page.contains(r#"class="discord running""#));
        assert!(page.contains(ACTION_RUNNING));
    }

    #[test]
    fn a_json_client_gets_a_status_instead_of_the_page() {
        let folder = TempDir::new().unwrap();
        let (app, wake) = app(&folder);
        assert_eq!(
            202,
            app.route("POST", "/actions/discord", "application/json")
                .status
        );
        assert!(wake.try_recv().is_ok());
    }

    #[test]
    fn the_actions_are_listed_as_json() {
        let folder = TempDir::new().unwrap();
        let (app, _wake) = app(&folder);
        let found = body(&app.route("GET", "/actions.json", ""));
        assert_eq!(json!("discord"), found["actions"][0]["key"]);
        assert_eq!(json!("Messages Discord"), found["actions"][0]["label"]);
    }

    #[test]
    fn an_unknown_route_is_refused() {
        let folder = TempDir::new().unwrap();
        let (app, _wake) = app(&folder);
        assert_eq!(404, app.route("POST", "/actions/other", "").status);
    }

    #[test]
    fn the_api_lists_the_messages_with_their_state() {
        let folder = TempDir::new().unwrap();
        let (app, _wake) = app(&folder);
        let posts = body(&app.route("GET", "/api/posts", ""))["posts"].clone();
        let note = posts.as_array().unwrap().last().unwrap().clone();
        assert_eq!(
            json!({"channel": "patch", "file": "patch-notes/2026-11-02.md", "message": null, "covers": 30}),
            note
        );
        assert_eq!(
            json!({"channel": "links", "file": "links.md", "message": null}),
            posts[0]
        );
    }

    #[test]
    fn the_api_publishes_and_reports_each_message() {
        let folder = TempDir::new().unwrap();
        let (app, _wake) = app(&folder);
        let results = body(&app.route("POST", "/api/publish", ""))["results"].clone();
        let note = results.as_array().unwrap().last().unwrap().clone();
        assert_eq!(
            json!({"channel": "patch", "file": "patch-notes/2026-11-02.md", "outcome": "posted"}),
            note
        );
        assert_eq!(json!("failed"), results[0]["outcome"]);
        let posts = body(&app.route("GET", "/api/posts", ""))["posts"].clone();
        assert_eq!(
            json!("1"),
            posts.as_array().unwrap().last().unwrap()["message"]
        );
    }

    #[test]
    fn the_api_publishes_a_single_file() {
        let folder = TempDir::new().unwrap();
        let (app, _wake) = app(&folder);
        let reply = app.route("POST", "/api/publish?file=patch-notes%2F2026-11-02.md", "");
        let results = body(&reply)["results"].clone();
        assert_eq!(1, results.as_array().unwrap().len());
        assert_eq!(json!("posted"), results[0]["outcome"]);
    }

    #[test]
    fn the_api_refuses_an_unknown_file() {
        let folder = TempDir::new().unwrap();
        let (app, _wake) = app(&folder);
        assert_eq!(
            404,
            app.route("POST", "/api/publish?file=../.env", "").status
        );
    }

    #[test]
    fn every_message_of_the_repo_fits_in_discord() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../discord-posts");
        let fake = Fake::default();
        let publisher = Publisher {
            folder: root,
            state: PathBuf::new(),
            api: String::new(),
            env: HashMap::from([
                ("DOMAIN".to_string(), "example.com".to_string()),
                ("CADDY_HTTPS_PORT".to_string(), "50000".to_string()),
            ]),
            discord: &fake,
        };
        publisher.check_notes().unwrap();
        for post in publisher.posts() {
            let message = publisher
                .render(&post)
                .unwrap_or_else(|error| panic!("{}: {error}", post.file));
            assert!(message.embed.contains_key("title"), "{}", post.file);
        }
    }
}
