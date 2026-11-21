use crate::cards::{self, Service};
use crate::docker::Docker;
use crate::icons::Icons;
use crate::shutdown::{self, Shutdown};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;
use tokio::sync::watch;

pub type Snapshot = Arc<Vec<Service>>;

pub enum Outcome {
    Done,
    Unknown,
    Refused,
    Failed,
}

pub struct App {
    pub docker: Docker,
    icons: Icons,
    pub web: PathBuf,
    pub snapshot: watch::Sender<Snapshot>,
    pub healthy: AtomicBool,
    shutdown: Mutex<Shutdown>,
}

impl App {
    pub fn new(docker: Docker, shutdown: Shutdown, icons: Icons, web: PathBuf) -> Self {
        App {
            docker,
            icons,
            web,
            snapshot: watch::Sender::new(Arc::new(vec![])),
            healthy: AtomicBool::new(false),
            shutdown: Mutex::new(shutdown),
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
