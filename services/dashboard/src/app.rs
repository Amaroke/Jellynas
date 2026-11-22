use crate::cards::{self, Service, State};
use crate::docker::Docker;
use crate::farm_post::{self, Poster};
use crate::icons::Icons;
use crate::shutdown::{self, Shutdown};
use crate::stats::{self, History, Sample};
use crate::widgets::{self, Board, Client, Panel};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn latest_deploy(containers: &Value) -> u64 {
    containers
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|container| {
            container["Labels"]["cd.doco.deployment.timestamp"]
                .as_str()
                .and_then(widgets::epoch)
        })
        .max()
        .unwrap_or(0)
}

pub type Snapshot = Arc<Vec<Service>>;

const POLL: Duration = Duration::from_secs(20);
const POINTS: usize = 90;
const ARCHIVES: &str = "/backup";

fn archives(folder: &Path) -> Vec<widgets::parse::Archive> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with("appdata-") && name.ends_with(".tar.gz")
        })
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let at = metadata
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_secs();
            Some(widgets::parse::Archive {
                at,
                size: metadata.len(),
            })
        })
        .collect()
}
const FARM_RETRY: Duration = Duration::from_secs(60);

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
    farm_wake: Mutex<Option<Sender<()>>>,
    answers: AtomicU64,
    samples: Mutex<HashMap<String, (String, Sample)>>,
    history: Mutex<History>,
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
            farm_wake: Mutex::new(None),
            answers: AtomicU64::new(0),
            samples: Mutex::new(HashMap::new()),
            history: Mutex::new(History::default()),
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
            (Some(kind), _) if kind == "doco-cd" => {
                let containers = self.docker.containers()?;
                let since = latest_deploy(&containers).saturating_sub(60);
                let logs = self.docker.logs(&service.id, since, 1000)?;
                widgets::parse::deploy(&containers, &logs, now())
            }
            (Some(kind), _) if kind == "backup" => {
                let logs = self.docker.logs(&service.id, 0, 200)?;
                widgets::parse::backup(&logs, &archives(Path::new(ARCHIVES)), now())
            }
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
        let answered = self.answers.load(Ordering::SeqCst);
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
        let (host, readings, farm, pulls, usage, uptime) = thread::scope(|scope| {
            let uptime = scope.spawn(|| self.client.uptime());
            let measured: Vec<_> = services
                .iter()
                .filter(|service| !service.shutdown && service.state != State::Stopped)
                .map(|service| {
                    let name = service.card.service.clone();
                    let id = service.id.clone();
                    (
                        name,
                        id,
                        scope.spawn(move || self.docker.stats(&service.id)),
                    )
                })
                .collect();
            let host = scope.spawn(|| self.client.host());
            let farm = scope.spawn(|| self.feed("RATIO", "/board", "the Farm"));
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
            let usage: Vec<_> = measured
                .into_iter()
                .map(|(name, id, stats)| (name, id, joined(stats)))
                .collect();
            (
                joined(host),
                readings,
                joined(farm),
                joined(pulls),
                usage,
                joined(uptime),
            )
        });
        let history = self.remember(&usage, &readings);
        let tier_before = Self::farm_tier(previous.farm.as_ref());
        let farm = Some(widgets::settle(previous.farm.as_ref(), farm, now));
        if farm_post::crossed(tier_before, Self::farm_tier(farm.as_ref())) {
            println!("The Account ratio crossed a tier, the Farm board goes to Discord");
            self.wake_farm();
        }
        let pulls = Some(widgets::settle(previous.pulls.as_ref(), pulls, now));
        let mut widgets: BTreeMap<String, Panel> = readings
            .into_iter()
            .map(|(name, reading)| {
                if let Err(error) = &reading {
                    eprintln!("Cannot read the widget of {name}: {error}");
                }
                let panel = widgets::settle(previous.widgets.get(&name), reading, now);
                (name, panel)
            })
            .collect();
        for service in services.iter() {
            let name = &service.card.service;
            let shown = service.card.widget.is_some() || service.card.monitor.is_some();
            if shown
                && !widgets.contains_key(name)
                && let Some(panel) = widgets::rest(previous.widgets.get(name))
            {
                widgets.insert(name.clone(), panel);
            }
        }
        if let Err(error) = &host {
            eprintln!("Cannot read the Host: {error}");
        }
        let host = Some(widgets::settle(previous.host.as_ref(), host, now));
        let uptime = uptime.unwrap_or_else(|error| {
            eprintln!("Cannot read the uptimes: {error}");
            previous.uptime.clone()
        });
        self.board.send_modify(|board| {
            let farm = if self.farm_changes.load(Ordering::SeqCst) == changes {
                farm
            } else {
                board.farm.clone()
            };
            let mut widgets = widgets;
            if self.answers.load(Ordering::SeqCst) != answered {
                for service in services.iter() {
                    let name = &service.card.service;
                    if matches!(
                        service.card.widget.as_deref(),
                        Some("seerr" | "prowlarr" | "radarr" | "sonarr" | "aurral")
                    ) && let Some(panel) = board.widgets.get(name)
                    {
                        widgets.insert(name.clone(), panel.clone());
                    }
                }
            }
            *board = Arc::new(Board {
                host,
                widgets,
                farm,
                pulls,
                history,
                uptime,
            });
        });
    }

    fn remember(
        &self,
        usage: &[(String, String, Result<Value, String>)],
        readings: &[(String, widgets::parse::Parsed)],
    ) -> History {
        let mut samples = self
            .samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut history = self
            .history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (name, id, found) in usage {
            let Some(sample) = found.as_ref().ok().and_then(stats::sample) else {
                continue;
            };
            let mut metrics = vec![("memory", sample.memory as f64)];
            if let Some(share) = samples
                .get(name)
                .filter(|(before, _)| before == id)
                .and_then(|(_, before)| stats::cpu(before, &sample))
            {
                metrics.push(("cpu", share));
            }
            history.record(name, &metrics, POINTS);
            samples.insert(name.clone(), (id.clone(), sample));
        }
        for (name, reading) in readings {
            if let Ok(reading) = reading {
                history.record(name, &reading.metrics, POINTS);
            }
        }
        let names: Vec<&str> = usage.iter().map(|(name, _, _)| name.as_str()).collect();
        history.keep(&names);
        samples.retain(|name, _| names.contains(&name.as_str()));
        history.clone()
    }

    pub fn farm_control(&self, control: &Value) -> Outcome {
        let valid = control["name"].is_string()
            && (control["value"].is_boolean() || control["value"].is_number());
        if !valid {
            return Outcome::Invalid;
        }
        let sent = self.client.send("RATIO", "/controls", control);
        let farm = self.feed("RATIO", "/board", "the Farm");
        self.board.send_modify(|board| {
            self.farm_changes.fetch_add(1, Ordering::SeqCst);
            let mut changed = (**board).clone();
            changed.farm = Some(widgets::settle(changed.farm.as_ref(), farm, now()));
            *board = Arc::new(changed);
        });
        match sent {
            Ok(()) => {
                self.wake_farm();
                Outcome::Done
            }
            Err(error) => {
                eprintln!("Cannot change {} in the Farm: {error}", control["name"]);
                Outcome::Failed
            }
        }
    }

    pub fn switch_link(&self, service: &str, kind: &str, id: u64, action: &str) -> Outcome {
        if !matches!(action, "on" | "off" | "retry") {
            return Outcome::Invalid;
        }
        let services = self.snapshot.borrow().clone();
        let Some(widget) = services
            .iter()
            .find(|found| found.card.service == service)
            .and_then(|found| found.card.widget.clone())
            .filter(|widget| {
                matches!(widget.as_str(), "prowlarr" | "radarr" | "sonarr" | "aurral")
            })
        else {
            return Outcome::Unknown;
        };
        let link = self
            .board
            .borrow()
            .widgets
            .get(service)
            .and_then(|panel| panel.reading.as_ref())
            .and_then(|reading| {
                reading
                    .links
                    .iter()
                    .find(|link| link.kind == kind && link.id == id)
                    .cloned()
            });
        let Some(link) = link else {
            return Outcome::Unknown;
        };
        if !link.allows(action) {
            return Outcome::Refused;
        }
        let sent = self.client.switch_link(&widget, kind, id, action);
        let reading = self.client.widget(&widget);
        self.board.send_modify(|board| {
            self.answers.fetch_add(1, Ordering::SeqCst);
            let mut changed = (**board).clone();
            let panel = widgets::settle(changed.widgets.get(service), reading, now());
            changed.widgets.insert(service.to_string(), panel);
            *board = Arc::new(changed);
        });
        match sent {
            Ok(()) => Outcome::Done,
            Err(error) => {
                eprintln!(
                    "Cannot {action} the {kind} {} of {service}: {error}",
                    link.name
                );
                Outcome::Failed
            }
        }
    }

    pub fn answer_request(&self, id: u64, action: &str) -> Outcome {
        if action != "approve" && action != "decline" {
            return Outcome::Invalid;
        }
        let services = self.snapshot.borrow().clone();
        let Some(service) = services
            .iter()
            .find(|service| service.card.widget.as_deref() == Some("seerr"))
        else {
            return Outcome::Unknown;
        };
        let name = service.card.service.clone();
        let listed = self
            .board
            .borrow()
            .widgets
            .get(&name)
            .and_then(|panel| panel.reading.as_ref())
            .is_some_and(|reading| reading.rows.iter().any(|row| row.id == Some(id)));
        if !listed {
            return Outcome::Unknown;
        }
        let sent = self.client.answer_request(id, action);
        let reading = self.client.widget("seerr");
        self.board.send_modify(|board| {
            self.answers.fetch_add(1, Ordering::SeqCst);
            let mut changed = (**board).clone();
            let panel = widgets::settle(changed.widgets.get(&name), reading, now());
            changed.widgets.insert(name.clone(), panel);
            *board = Arc::new(changed);
        });
        match sent {
            Ok(()) => Outcome::Done,
            Err(error) => {
                eprintln!("Cannot {action} the request {id}: {error}");
                Outcome::Failed
            }
        }
    }

    fn farm_tier(panel: Option<&Panel<Value>>) -> Option<(usize, f64)> {
        panel
            .and_then(|panel| panel.reading.as_ref())
            .and_then(farm_post::tier_of)
    }

    fn wake_farm(&self) {
        if let Some(wake) = self
            .farm_wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            let _ = wake.send(());
        }
    }

    pub fn listen_farm(&self, wake: Sender<()>) {
        *self
            .farm_wake
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(wake);
    }

    pub fn post_farm(&self, poster: Poster, woken: Receiver<()>) {
        loop {
            let board = self.client.feed("RATIO", "/board");
            let wait = match board {
                Ok(board) if farm_post::ready(&board) => {
                    match poster.post(&board, &jiff::ZonedDateTime::now()) {
                        Ok(done) => println!("{done}"),
                        Err(error) => {
                            println!(
                                "Cannot publish the Farm board, the next try is in one hour: {error}"
                            )
                        }
                    }
                    farm_post::until_next_hour(&jiff::ZonedDateTime::now())
                }
                Ok(_) => {
                    println!("ratio has not filled its board yet, the next try is in one minute");
                    FARM_RETRY
                }
                Err(_) => FARM_RETRY,
            };
            let _ = woken.recv_timeout(wait);
            while woken.try_recv().is_ok() {}
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
        for id in shutdown::unstarted(&containers, self.shutdown().services()) {
            println!("Starting the container {id} the deploy left unstarted");
            if let Err(error) = self.docker.act(&id, "start") {
                eprintln!("Cannot start the container {id}: {error}");
            }
        }
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
