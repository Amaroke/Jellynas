use crate::cards::{self, Service, State};
use crate::docker::Docker;
use crate::icons::Icons;
use crate::shutdown::{self, Shutdown};
use crate::widgets::{self, Board, Client, Panel};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

pub type Snapshot = Arc<Vec<Service>>;

const POLL: Duration = Duration::from_secs(20);

pub enum Outcome {
    Done,
    Unknown,
    Invalid,
    Refused,
    Failed,
}

pub struct App {
    pub docker: Docker,
    icons: Icons,
    pub web: PathBuf,
    pub snapshot: watch::Sender<Snapshot>,
    pub board: watch::Sender<Arc<Board>>,
    pub healthy: AtomicBool,
    shutdown: Mutex<Shutdown>,
    client: Client,
    farm_changes: AtomicU64,
}

impl App {
    pub fn new(
        docker: Docker,
        shutdown: Shutdown,
        icons: Icons,
        web: PathBuf,
        client: Client,
    ) -> Self {
        App {
            docker,
            icons,
            web,
            snapshot: watch::Sender::new(Arc::new(vec![])),
            board: watch::Sender::new(Arc::new(Board::default())),
            healthy: AtomicBool::new(false),
            shutdown: Mutex::new(shutdown),
            client,
            farm_changes: AtomicU64::new(0),
        }
    }

    pub fn poll(&self) {
        loop {
            self.collect();
            let waiting = self.snapshot.borrow().is_empty();
            thread::sleep(if waiting {
                Duration::from_secs(2)
            } else {
                POLL
            });
        }
    }

    fn read(&self, service: &Service) -> widgets::parse::Parsed {
        match (&service.card.widget, &service.card.monitor) {
            (Some(kind), _) => self.client.widget(kind),
            (None, Some(url)) => self.client.monitor(url),
            (None, None) => Err("no widget".into()),
        }
    }

    fn feed(&self, prefix: &str, path: &str, what: &str) -> Result<Value, String> {
        let found = self.client.feed(prefix, path);
        if let Err(error) = &found {
            eprintln!("Cannot read {what}: {error}");
        }
        found
    }

    fn collect(&self) {
        let services = self.snapshot.borrow().clone();
        let previous = self.board.borrow().clone();
        let changes = self.farm_changes.load(Ordering::SeqCst);
        let now = now();
        let watched = services.iter().filter(|service| {
            !service.shutdown
                && service.state != State::Stopped
                && (service.card.widget.is_some() || service.card.monitor.is_some())
        });
        fn joined<T>(handle: thread::ScopedJoinHandle<'_, Result<T, String>>) -> Result<T, String> {
            handle
                .join()
                .unwrap_or_else(|_| Err("the reading crashed".to_string()))
        }
        let (host, readings, farm, actions, pulls) = thread::scope(|scope| {
            let host = scope.spawn(|| self.client.host());
            let farm = scope.spawn(|| self.feed("FARM_BOARD", "/board.json", "the Farm"));
            let actions =
                scope.spawn(|| self.feed("DISCORD_POSTS", "/actions.json", "the Quick actions"));
            let pulls = scope.spawn(|| self.feed("PR_RADAR", "/pulls.json", "the Pull requests"));
            let pending: Vec<_> = watched
                .map(|service| {
                    let name = service.card.service.clone();
                    (name, scope.spawn(move || self.read(service)))
                })
                .collect();
            let readings: Vec<_> = pending
                .into_iter()
                .map(|(name, reading)| (name, joined(reading)))
                .collect();
            (
                joined(host),
                readings,
                joined(farm),
                joined(actions),
                joined(pulls),
            )
        });
        let farm = Some(widgets::settle(previous.farm.as_ref(), farm, now));
        let actions = Some(widgets::settle(previous.actions.as_ref(), actions, now));
        let pulls = Some(widgets::settle(previous.pulls.as_ref(), pulls, now));
        let widgets: BTreeMap<String, Panel> = readings
            .into_iter()
            .map(|(name, reading)| {
                if let Err(error) = &reading {
                    eprintln!("Cannot read the widget of {name}: {error}");
                }
                let panel = widgets::settle(previous.widgets.get(&name), reading, now);
                (name, panel)
            })
            .collect();
        if let Err(error) = &host {
            eprintln!("Cannot read the Host: {error}");
        }
        let host = Some(widgets::settle(previous.host.as_ref(), host, now));
        self.board.send_modify(|board| {
            let farm = if self.farm_changes.load(Ordering::SeqCst) == changes {
                farm
            } else {
                board.farm.clone()
            };
            *board = Arc::new(Board {
                host,
                widgets,
                farm,
                actions,
                pulls,
            });
        });
    }

    pub fn farm_control(&self, control: &Value) -> Outcome {
        let valid = control["name"].is_string()
            && (control["value"].is_boolean() || control["value"].is_number());
        if !valid {
            return Outcome::Invalid;
        }
        let sent = self.client.send("FARM_BOARD", "/controls", control);
        let farm = self.feed("FARM_BOARD", "/board.json", "the Farm");
        self.board.send_modify(|board| {
            self.farm_changes.fetch_add(1, Ordering::SeqCst);
            let mut changed = (**board).clone();
            changed.farm = Some(widgets::settle(changed.farm.as_ref(), farm, now()));
            *board = Arc::new(changed);
        });
        match sent {
            Ok(()) => Outcome::Done,
            Err(error) => {
                eprintln!("Cannot change {} in the Farm: {error}", control["name"]);
                Outcome::Failed
            }
        }
    }

    pub fn quick_action(&self, key: &str) -> Outcome {
        let known = self
            .board
            .borrow()
            .actions
            .as_ref()
            .and_then(|panel| panel.reading.as_ref())
            .and_then(|actions| actions["actions"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .any(|action| action["key"] == key);
        if !known {
            return Outcome::Unknown;
        }
        match self
            .client
            .send("DISCORD_POSTS", &format!("/actions/{key}"), &Value::Null)
        {
            Ok(()) => Outcome::Done,
            Err(error) => {
                eprintln!("Cannot run the Quick action {key}: {error}");
                Outcome::Failed
            }
        }
    }

    fn shutdown(&self) -> MutexGuard<'_, Shutdown> {
        self.shutdown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn stop(&self, id: &str) {
        if let Err(error) = self.docker.act(id, "stop") {
            eprintln!("Cannot stop the Shut down container {id}: {error}");
        }
    }

    pub fn refresh(&self) {
        let containers = match self.docker.containers() {
            Ok(containers) => containers,
            Err(error) => {
                eprintln!("Cannot list the containers: {error}");
                self.healthy.store(false, Ordering::Relaxed);
                return;
            }
        };
        let services = cards::services(&containers, self.shutdown().services());
        for id in shutdown::strays(&services) {
            println!("Stopping the Shut down container {id}");
            self.stop(&id);
        }
        self.healthy.store(true, Ordering::Relaxed);
        self.snapshot.send_replace(Arc::new(services));
    }

    fn enforced(&self) -> BTreeSet<String> {
        self.snapshot
            .borrow()
            .iter()
            .filter(|service| service.shutdown)
            .map(|service| service.card.service.clone())
            .collect()
    }

    pub fn watch(&self) {
        loop {
            match self.docker.events() {
                Ok(events) => {
                    self.refresh();
                    for event in events {
                        if let Some(id) = shutdown::to_stop(&event, &self.enforced()) {
                            println!("Stopping the Shut down container {id} that started again");
                            self.stop(&id);
                        }
                        self.refresh();
                    }
                }
                Err(error) => {
                    eprintln!("Cannot follow the Docker events: {error}");
                    self.healthy.store(false, Ordering::Relaxed);
                    thread::sleep(Duration::from_secs(5));
                }
            }
        }
    }

    pub fn icon(&self, file: &str) -> Option<(&'static str, Vec<u8>)> {
        let named = self
            .snapshot
            .borrow()
            .iter()
            .any(|service| service.card.icon.as_deref() == Some(file));
        if !named {
            return None;
        }
        self.icons.get(file)
    }

    pub fn control(&self, service: &str, action: &str) -> Outcome {
        let found = self
            .snapshot
            .borrow()
            .iter()
            .find(|found| found.card.service == service && found.card.switchable)
            .map(|found| found.id.clone());
        let Some(id) = found else {
            return Outcome::Unknown;
        };
        let result = match action {
            "off" => {
                let listed = self.shutdown().turn_off(service);
                listed.and_then(|()| self.docker.act(&id, "stop"))
            }
            "on" => {
                let listed = self.shutdown().turn_on(service);
                listed.and_then(|()| self.docker.act(&id, "start"))
            }
            "restart" => {
                if self.shutdown().services().contains(service) {
                    return Outcome::Refused;
                }
                self.docker.act(&id, "restart")
            }
            _ => return Outcome::Unknown,
        };
        self.refresh();
        match result {
            Ok(()) => {
                println!("{service}: {action} done");
                Outcome::Done
            }
            Err(error) => {
                eprintln!("Cannot turn {action} {service}: {error}");
                Outcome::Failed
            }
        }
    }
}
