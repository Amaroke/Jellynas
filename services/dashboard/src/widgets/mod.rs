mod fetch;
mod format;
pub mod parse;

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub use fetch::Client;
pub use format::{bytes, epoch};

use crate::stats::History;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Figure {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    pub title: String,
    pub detail: String,
    pub progress: Option<f64>,
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Uptime {
    pub day: Option<f64>,
    pub week: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Poster {
    pub title: String,
    pub detail: String,
    pub image: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Link {
    pub kind: String,
    pub id: u64,
    pub name: String,
    pub hint: Option<String>,
    pub state: String,
    pub until: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Health {
    pub level: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Reading {
    pub figures: Vec<Figure>,
    pub rows: Vec<Row>,
    pub posters: Vec<Poster>,
    pub shelf: Option<String>,
    pub idle: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub health: Vec<Health>,
    #[serde(skip)]
    pub metrics: Vec<(&'static str, f64)>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Panel<T = Reading> {
    pub reading: Option<T>,
    pub error: Option<String>,
    pub updated: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Board {
    pub host: Option<Panel>,
    pub widgets: BTreeMap<String, Panel>,
    pub farm: Option<Panel<Value>>,
    pub actions: Option<Panel<Value>>,
    pub pulls: Option<Panel<Value>>,
    pub history: History,
    pub uptime: BTreeMap<String, Uptime>,
    pub media: Option<Panel>,
    pub truenas: Option<Panel>,
}

pub fn settle<T: Clone>(
    previous: Option<&Panel<T>>,
    result: Result<T, String>,
    now: u64,
) -> Panel<T> {
    match result {
        Ok(reading) => Panel {
            reading: Some(reading),
            error: None,
            updated: Some(now),
        },
        Err(error) => Panel {
            reading: previous.and_then(|panel| panel.reading.clone()),
            error: Some(error),
            updated: previous.and_then(|panel| panel.updated),
        },
    }
}

pub fn rest(previous: Option<&Panel>) -> Option<Panel> {
    let mut reading = previous?.reading.clone()?;
    for figure in &mut reading.figures {
        figure.value = "—".into();
    }
    reading.rows.clear();
    reading.posters.clear();
    reading.health.clear();
    for link in &mut reading.links {
        link.state = "unknown".into();
        link.until = None;
    }
    if reading.idle.is_some() {
        reading.idle = Some("Service stopped".into());
    }
    Some(Panel {
        reading: Some(reading),
        error: None,
        updated: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(value: &str) -> Reading {
        Reading {
            figures: vec![Figure {
                label: "Movies".into(),
                value: value.into(),
            }],
            rows: vec![],
            posters: vec![],
            shelf: None,
            idle: None,
            links: vec![],
            health: vec![],
            metrics: vec![],
        }
    }

    #[test]
    fn a_resting_panel_keeps_its_connections_in_an_unknown_state() {
        let mut found = reading("1");
        found.links = vec![Link {
            kind: "client".into(),
            id: 1,
            name: "qBittorrent".into(),
            hint: None,
            state: "failing".into(),
            until: Some(500),
        }];
        found.health = vec![Health {
            level: "warning".into(),
            message: "No download client is available".into(),
        }];
        let resting = rest(Some(&settle(None, Ok(found), 100)))
            .unwrap()
            .reading
            .unwrap();
        assert_eq!(
            vec![("qBittorrent", "unknown", None)],
            resting
                .links
                .iter()
                .map(|link| (link.name.as_str(), link.state.as_str(), link.until))
                .collect::<Vec<_>>()
        );
        assert!(resting.health.is_empty());
    }

    #[test]
    fn a_resting_panel_keeps_its_shape_without_values() {
        let mut playing = reading("12");
        playing.rows = vec![Row {
            title: "Heat".into(),
            detail: "carol".into(),
            progress: None,
            image: None,
            id: None,
        }];
        playing.idle = Some("Nothing playing".into());
        playing.posters = vec![Poster {
            title: "Heat".into(),
            detail: "1995".into(),
            image: "https://jellyfin.example.org/Items/m1/Images/Primary".into(),
        }];
        let previous = settle(None, Ok(playing), 100);
        let mut blank = reading("—");
        blank.idle = Some("Service stopped".into());
        let expected = Panel {
            reading: Some(blank),
            error: None,
            updated: None,
        };
        assert_eq!(Some(expected), rest(Some(&previous)));
        assert_eq!(None, rest(None));
    }

    #[test]
    fn a_fresh_reading_replaces_the_panel() {
        let previous = settle(None, Err("timed out".into()), 100);
        let panel = settle(Some(&previous), Ok(reading("12")), 130);
        let expected = Panel {
            reading: Some(reading("12")),
            error: None,
            updated: Some(130),
        };
        assert_eq!(expected, panel);
    }

    #[test]
    fn a_failure_keeps_the_last_reading_as_stale() {
        let previous = settle(None, Ok(reading("12")), 100);
        let panel = settle(Some(&previous), Err("timed out".into()), 130);
        let expected = Panel {
            reading: Some(reading("12")),
            error: Some("timed out".into()),
            updated: Some(100),
        };
        assert_eq!(expected, panel);
    }

    #[test]
    fn a_failure_without_a_reading_shows_only_the_error() {
        let panel: Panel = settle(None, Err("401 Unauthorized".into()), 130);
        let expected = Panel {
            reading: None,
            error: Some("401 Unauthorized".into()),
            updated: None,
        };
        assert_eq!(expected, panel);
    }
}
