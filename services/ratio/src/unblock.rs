use crate::clients::Fail;
use crate::http;
use jiff::Timestamp;
use serde_json::{Value, json};

pub const PROWLARR_URL: &str = "http://prowlarr:9696";
const FAILURE_CHECKS: [&str; 2] = ["IndexerStatusCheck", "IndexerLongTermStatusCheck"];

pub trait Apps {
    fn name(&self) -> &str;
    fn held(&self, indexers: &[String], now: f64) -> Result<Vec<Value>, Fail>;
    fn test(&self, definition: &Value) -> Result<(), Fail>;
}

pub struct Servarr {
    name: String,
    url: String,
    key: String,
    prowlarr: bool,
    agent: ureq::Agent,
}

impl Servarr {
    pub fn new(name: &str, url: &str, key: &str) -> Result<Self, String> {
        Ok(Servarr {
            name: name.into(),
            url: url.trim_end_matches('/').into(),
            key: key.into(),
            prowlarr: url.trim_end_matches('/') == PROWLARR_URL,
            agent: http::agent(None, 120)?,
        })
    }

    fn api(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, Fail> {
        let version = if self.prowlarr { "v1" } else { "v3" };
        let reply = http::call(
            &self.agent,
            method,
            &format!("{}/api/{version}{path}", self.url),
            &[("x-api-key", self.key.clone())],
            body.map(|body| ("application/json", body.to_string().into_bytes())),
        )?;
        if !reply.ok() {
            return Err(Fail::Other(format!(
                "{} answered {} to {path}",
                self.name, reply.status
            )));
        }
        if reply.body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&reply.body).map_err(|error| Fail::Other(error.to_string()))
    }
}

impl Apps for Servarr {
    fn name(&self) -> &str {
        &self.name
    }

    fn held(&self, indexers: &[String], now: f64) -> Result<Vec<Value>, Fail> {
        if self.prowlarr {
            let status = self.api("GET", "/indexerstatus", None)?;
            return held_by_prowlarr(&status, indexers, now)
                .iter()
                .map(|id| self.api("GET", &format!("/indexer/{id}"), None))
                .collect();
        }
        let listed = self.api("GET", "/indexer", None)?;
        let health = self.api("GET", "/health", None)?;
        Ok(held_by_arr(&listed, &health, indexers))
    }

    fn test(&self, definition: &Value) -> Result<(), Fail> {
        self.api("POST", "/indexer/test", Some(definition.clone()))?;
        if !self.prowlarr {
            self.api("POST", "/command", Some(json!({"name": "CheckHealth"})))?;
        }
        Ok(())
    }
}

pub fn held_by_prowlarr(status: &Value, indexers: &[String], now: f64) -> Vec<String> {
    status
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry["disabledTill"]
                .as_str()
                .and_then(|until| until.parse::<Timestamp>().ok())
                .is_some_and(|until| until.as_second() as f64 > now)
        })
        .filter_map(|entry| entry["indexerId"].as_i64().map(|id| id.to_string()))
        .filter(|id| indexers.contains(id))
        .collect()
}

fn synced_from(indexer: &Value, indexers: &[String]) -> bool {
    let base = indexer["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|field| field["name"] == "baseUrl")
        .and_then(|field| field["value"].as_str())
        .unwrap_or_default();
    match base.trim_end_matches('/').rsplit_once('/') {
        Some((prefix, id)) => prefix == PROWLARR_URL && indexers.iter().any(|wanted| wanted == id),
        None => false,
    }
}

pub fn held_by_arr(listed: &Value, health: &Value, indexers: &[String]) -> Vec<Value> {
    let failing: Vec<&str> = health
        .as_array()
        .into_iter()
        .flatten()
        .filter(|check| {
            check["source"]
                .as_str()
                .is_some_and(|source| FAILURE_CHECKS.contains(&source))
        })
        .filter_map(|check| check["message"].as_str())
        .collect();
    listed
        .as_array()
        .into_iter()
        .flatten()
        .filter(|indexer| {
            let name = indexer["name"].as_str().unwrap_or_default();
            !name.is_empty()
                && synced_from(indexer, indexers)
                && failing.iter().any(|message| message.contains(name))
        })
        .cloned()
        .collect()
}

pub fn lift(apps: &[&dyn Apps], indexers: &[String], reachable: bool, now: f64) {
    if !reachable || indexers.is_empty() {
        return;
    }
    for app in apps {
        let held = match app.held(indexers, now) {
            Ok(held) => held,
            Err(error) => {
                println!("Cannot read the indexers of {}: {error}", app.name());
                continue;
            }
        };
        for definition in held {
            let name = definition["name"].as_str().unwrap_or("C411");
            match app.test(&definition) {
                Ok(()) => println!("{} uses {name} again, since C411 answers", app.name()),
                Err(error) => println!("{} still holds {name} back: {error}", app.name()),
            }
        }
    }
}

#[cfg(test)]
pub mod fakes {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct FakeApp {
        pub held: Vec<Value>,
        pub tested: RefCell<Vec<String>>,
    }

    impl Apps for FakeApp {
        fn name(&self) -> &str {
            "Radarr"
        }
        fn held(&self, _indexers: &[String], _now: f64) -> Result<Vec<Value>, Fail> {
            Ok(self.held.clone())
        }
        fn test(&self, definition: &Value) -> Result<(), Fail> {
            self.tested
                .borrow_mut()
                .push(definition["name"].as_str().unwrap_or_default().into());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::FakeApp;
    use super::*;

    const NOW: f64 = 1_800_000_000.0;

    fn ids() -> Vec<String> {
        vec!["3".into()]
    }

    fn arr_indexer(name: &str, base: &str) -> Value {
        json!({"name": name, "fields": [{"name": "baseUrl", "value": base}]})
    }

    #[test]
    fn prowlarr_holds_back_only_a_c411_indexer_still_disabled() {
        let later = Timestamp::from_second(NOW as i64 + 600)
            .unwrap()
            .to_string();
        let earlier = Timestamp::from_second(NOW as i64 - 600)
            .unwrap()
            .to_string();
        let status = json!([
            {"indexerId": 3, "disabledTill": later},
            {"indexerId": 1, "disabledTill": later},
        ]);
        assert_eq!(vec!["3"], held_by_prowlarr(&status, &ids(), NOW));
        let expired = json!([{"indexerId": 3, "disabledTill": earlier}]);
        assert!(held_by_prowlarr(&expired, &ids(), NOW).is_empty());
        let cleared = json!([{"indexerId": 3, "disabledTill": null}]);
        assert!(held_by_prowlarr(&cleared, &ids(), NOW).is_empty());
    }

    #[test]
    fn radarr_holds_back_the_c411_indexer_its_health_reports() {
        let listed = json!([
            arr_indexer("C411 (Prowlarr)", "http://prowlarr:9696/3/"),
            arr_indexer("Nyaa.si (Prowlarr)", "http://prowlarr:9696/1/"),
        ]);
        let health = json!([{
            "source": "IndexerStatusCheck",
            "message": "Indexers unavailable due to failures: C411 (Prowlarr), Nyaa.si (Prowlarr)"
        }]);
        let held = held_by_arr(&listed, &health, &ids());
        assert_eq!(1, held.len());
        assert_eq!("C411 (Prowlarr)", held[0]["name"]);
        let healthy = json!([{"source": "DownloadClientCheck", "message": "C411 (Prowlarr)"}]);
        assert!(held_by_arr(&listed, &healthy, &ids()).is_empty());
    }

    #[test]
    fn an_indexer_of_another_prowlarr_id_is_left_alone() {
        let listed = json!([arr_indexer("C411 (Prowlarr)", "http://prowlarr:9696/30/")]);
        let health = json!([{
            "source": "IndexerLongTermStatusCheck",
            "message": "All indexers are unavailable due to failures for more than 6 hours: C411 (Prowlarr)"
        }]);
        assert!(held_by_arr(&listed, &health, &ids()).is_empty());
    }

    #[test]
    fn a_held_indexer_is_tested_only_once_c411_answers() {
        let app = FakeApp {
            held: vec![json!({"name": "C411 (Prowlarr)"})],
            ..FakeApp::default()
        };
        lift(&[&app], &ids(), false, NOW);
        assert!(app.tested.borrow().is_empty());
        lift(&[&app], &[], true, NOW);
        assert!(app.tested.borrow().is_empty());
        lift(&[&app], &ids(), true, NOW);
        assert_eq!(vec!["C411 (Prowlarr)"], *app.tested.borrow());
    }
}
