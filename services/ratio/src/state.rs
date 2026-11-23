use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const GB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Day {
    pub day: String,
    pub floor: f64,
    pub ratio: Option<f64>,
    pub left_gb: Option<f64>,
    pub uploaded_gb: Option<f64>,
    pub actual_gb: Option<f64>,
    pub uploaded_day_gb: Option<f64>,
    pub downloaded_gb: Option<f64>,
    pub downloaded_day_gb: Option<f64>,
    pub zen: Option<f64>,
    pub earned: Option<f64>,
    pub zen_paid: Option<f64>,
    pub seeds: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Report {
    pub at: f64,
    pub origins: Vec<Value>,
}

pub type Twin = (String, String, f64, String);

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct State {
    pub transfers: BTreeMap<String, (i64, i64)>,
    pub twins: BTreeMap<String, Twin>,
    pub free: BTreeMap<String, f64>,
    pub checked: BTreeMap<String, f64>,
    pub paused: bool,
    pub origins: BTreeMap<String, (String, String)>,
    pub history: BTreeMap<String, Vec<(f64, i64, i64)>>,
    pub zen: BTreeMap<String, (i64, i64)>,
    pub report: Report,
    pub account: Vec<(f64, i64, i64, f64)>,
    pub received: Vec<(f64, f64)>,
    pub sent: Vec<(f64, f64)>,
    pub held: Option<bool>,
    pub backoff_until: f64,
    pub stuck: BTreeMap<String, (u32, f64)>,
    pub given_up: BTreeMap<String, f64>,
    pub restore: BTreeMap<String, i64>,
    pub daily: Vec<Day>,
    pub shown_account: Option<crate::clients::Account>,
}

pub fn load_state(path: &Path) -> Result<State, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("Cannot read {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(error) => Err(format!("Cannot read {}: {error}", path.display())),
    }
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes)
        .and_then(|_| fs::rename(&temporary, path))
        .map_err(|error| format!("Cannot save {}: {error}", path.display()))
}

pub struct Number {
    pub name: &'static str,
    pub unit: f64,
    pub low: f64,
    pub high: f64,
}

pub const SWITCHES: [(&str, bool); 5] = [
    ("free", false),
    ("twins", false),
    ("clean_public", true),
    ("hold", true),
    ("adopt", false),
];

pub const NUMBERS: [Number; 3] = [
    Number {
        name: "farm_size_gb",
        unit: GB,
        low: 1.0,
        high: 100000.0,
    },
    Number {
        name: "floor",
        unit: 1.0,
        low: 0.5,
        high: 5.0,
    },
    Number {
        name: "free_max_gb",
        unit: GB,
        low: 0.5,
        high: 1000.0,
    },
];

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub tracker: String,
    pub hold_indexers: Vec<String>,
    pub twin_indexers: Vec<String>,
    pub cross_seed_key: String,
    pub farm_size: f64,
    pub floor: f64,
    pub free_max_size: f64,
}

impl Settings {
    pub fn number(&self, name: &str) -> f64 {
        match name {
            "farm_size_gb" => self.farm_size,
            "floor" => self.floor,
            "free_max_gb" => self.free_max_size,
            _ => 0.0,
        }
    }

    pub fn with(&self, controls: &Snapshot) -> Settings {
        let mut found = self.clone();
        for number in &NUMBERS {
            let value = controls.numbers[number.name] * number.unit;
            match number.name {
                "farm_size_gb" => found.farm_size = value,
                "floor" => found.floor = value,
                _ => found.free_max_size = value,
            }
        }
        found
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Snapshot {
    pub switches: BTreeMap<String, bool>,
    pub numbers: BTreeMap<String, f64>,
}

impl Snapshot {
    pub fn on(&self, name: &str) -> bool {
        self.switches.get(name).copied().unwrap_or(false)
    }
}

pub struct Controls {
    path: PathBuf,
    switches: Map<String, Value>,
    numbers: Map<String, Value>,
}

fn kept(saved: &Value, group: &str, known: &[&str]) -> Map<String, Value> {
    saved[group]
        .as_object()
        .map(|found| {
            found
                .iter()
                .filter(|(name, _)| known.contains(&name.as_str()))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

impl Controls {
    pub fn load(path: &Path) -> Controls {
        let saved: Value = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        let switch_names: Vec<&str> = SWITCHES.iter().map(|(name, _)| *name).collect();
        let number_names: Vec<&str> = NUMBERS.iter().map(|number| number.name).collect();
        let controls = Controls {
            path: path.into(),
            switches: kept(&saved, "switches", &switch_names),
            numbers: kept(&saved, "numbers", &number_names),
        };
        if !saved.is_null() && saved != controls.saved() {
            match controls.save() {
                Ok(()) => println!("ratio dropped the controls it no longer knows"),
                Err(error) => println!("{error}"),
            }
        }
        controls
    }

    fn saved(&self) -> Value {
        json!({"switches": self.switches, "numbers": self.numbers})
    }

    fn save(&self) -> Result<(), String> {
        write_json(&self.path, &self.saved())
    }

    pub fn snapshot(&self, settings: &Settings) -> Snapshot {
        let switches = SWITCHES
            .iter()
            .map(|(name, on)| {
                let value = self
                    .switches
                    .get(*name)
                    .and_then(Value::as_bool)
                    .unwrap_or(*on);
                (name.to_string(), value)
            })
            .collect();
        let numbers = NUMBERS
            .iter()
            .map(|number| {
                let fallback = (settings.number(number.name) / number.unit * 100.0).round() / 100.0;
                let value = self
                    .numbers
                    .get(number.name)
                    .and_then(Value::as_f64)
                    .unwrap_or(fallback);
                (number.name.to_string(), value)
            })
            .collect();
        Snapshot { switches, numbers }
    }

    pub fn change(&mut self, name: &str, value: &Value) -> Result<(), String> {
        let mut switches = self.switches.clone();
        let mut numbers = self.numbers.clone();
        if SWITCHES.iter().any(|(known, _)| *known == name) {
            if !value.is_boolean() {
                return Err(format!("{name} takes true or false"));
            }
            switches.insert(name.into(), value.clone());
        } else if let Some(number) = NUMBERS.iter().find(|number| number.name == name) {
            match value.as_f64() {
                Some(found)
                    if !value.is_boolean() && (number.low..=number.high).contains(&found) =>
                {
                    numbers.insert(name.into(), value.clone());
                }
                _ => {
                    return Err(format!(
                        "{name} takes a number from {} to {}",
                        number.low, number.high
                    ));
                }
            }
        } else {
            return Err(format!("{name} is not a control"));
        }
        write_json(
            &self.path,
            &json!({"switches": switches, "numbers": numbers}),
        )?;
        self.switches = switches;
        self.numbers = numbers;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            tracker: "c411".into(),
            hold_indexers: vec![],
            twin_indexers: vec![],
            cross_seed_key: String::new(),
            farm_size: 100.0 * GB,
            floor: 2.0,
            free_max_size: 5.0 * GB,
        }
    }

    #[test]
    fn a_state_of_the_python_ratio_loads_and_drops_what_is_gone() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.json");
        fs::write(
            &path,
            r#"{"transfers": {"a": [10, 20]}, "hunted": {"b": [1.5, 3]}, "twins": {"n": ["c", "farm", 1700000000.5, ""]},
            "origins": {"a": ["farm", "c411"]}, "history": {"a": [[1700000000.25, 0, 0]]}, "zen": {"a": [5, 6]},
            "report": {"at": 1.0, "origins": [{"origin": "farm"}]}, "account": [[1.0, 2, 3, 4]],
            "received": [[1.0, 2]], "sent": [[1.0, 2.5]], "held": null, "stuck": {"n": [1, 2.0]},
            "restore": {"3": 1}, "daily": [{"day": "2026-11-20", "floor": 1.2, "ratio": 1.4, "sources": {}}], "tier": 2}"#,
        )
        .unwrap();
        let state = load_state(&path).unwrap();
        assert_eq!((10, 20), state.transfers["a"]);
        assert_eq!("c", state.twins["n"].0);
        assert_eq!(Some(1.4), state.daily[0].ratio);
        assert_eq!(1, state.restore["3"]);
    }

    #[test]
    fn a_missing_state_starts_empty() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(
            State::default(),
            load_state(&folder.path().join("none.json")).unwrap()
        );
    }

    #[test]
    fn a_broken_state_stops_ratio_instead_of_wiping_it() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state.json");
        fs::write(&path, "{broken").unwrap();
        assert!(load_state(&path).is_err());
    }

    #[test]
    fn dead_controls_are_dropped_from_the_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("controls.json");
        fs::write(
            &path,
            r#"{"switches": {"twins": true, "paid": true, "volume:gps": false, "quantity_seed": false}, "numbers": {"floor": 1.0, "daily_gb": 5}}"#,
        )
        .unwrap();
        let controls = Controls::load(&path);
        let snapshot = controls.snapshot(&settings());
        assert!(snapshot.on("twins"));
        assert!(!snapshot.switches.contains_key("paid"));
        assert_eq!(1.0, snapshot.numbers["floor"]);
        assert_eq!(100.0, snapshot.numbers["farm_size_gb"]);
        assert_eq!(5.0, snapshot.numbers["free_max_gb"]);
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            json!({"switches": {"twins": true}, "numbers": {"floor": 1.0}}),
            saved
        );
    }

    #[test]
    fn a_control_out_of_range_or_unknown_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let mut controls = Controls::load(&folder.path().join("controls.json"));
        assert!(controls.change("floor", &json!(9)).is_err());
        assert!(controls.change("floor", &json!(true)).is_err());
        assert!(controls.change("free", &json!(1)).is_err());
        assert!(controls.change("paid", &json!(true)).is_err());
        assert!(controls.change("floor", &json!(2.5)).is_ok());
        assert_eq!(2.5, controls.snapshot(&settings()).numbers["floor"]);
    }

    #[test]
    fn a_control_that_cannot_be_saved_does_not_take_effect() {
        let folder = tempfile::tempdir().unwrap();
        let mut controls = Controls::load(&folder.path().join("missing").join("controls.json"));
        assert!(controls.change("free", &json!(true)).is_err());
        assert!(!controls.snapshot(&settings()).on("free"));
    }

    #[test]
    fn the_numbers_of_the_controls_override_the_settings() {
        let folder = tempfile::tempdir().unwrap();
        let mut controls = Controls::load(&folder.path().join("controls.json"));
        controls.change("farm_size_gb", &json!(150)).unwrap();
        let effective = settings().with(&controls.snapshot(&settings()));
        assert_eq!(150.0 * GB, effective.farm_size);
        assert_eq!(2.0, effective.floor);
    }
}
