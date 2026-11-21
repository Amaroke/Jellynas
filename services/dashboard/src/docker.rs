use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::time::Duration;

const LABELLED: &str = r#"{"label":["dashboard.section"]}"#;
const WATCHED: &str = r#"{"type":["container"],"event":["create","start","restart","stop","die","destroy","pause","unpause","health_status"]}"#;

pub struct Docker {
    base: String,
    agent: ureq::Agent,
    stream: ureq::Agent,
}

impl Docker {
    pub fn new(host: &str) -> Self {
        Docker {
            base: host.trim_end_matches('/').replacen("tcp://", "http://", 1),
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(30)))
                .build()
                .into(),
            stream: ureq::Agent::config_builder()
                .timeout_connect(Some(Duration::from_secs(10)))
                .build()
                .into(),
        }
    }

    pub fn containers(&self) -> Result<Value, String> {
        self.agent
            .get(format!("{}/containers/json", self.base))
            .query("all", "true")
            .query("filters", LABELLED)
            .call()
            .and_then(|mut response| response.body_mut().read_json())
            .map_err(|error| error.to_string())
    }

    pub fn stats(&self, id: &str) -> Result<Value, String> {
        self.agent
            .get(format!("{}/containers/{id}/stats", self.base))
            .query("stream", "false")
            .query("one-shot", "true")
            .config()
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .call()
            .and_then(|mut response| response.body_mut().read_json())
            .map_err(|error| error.to_string())
    }

    pub fn act(&self, id: &str, action: &str) -> Result<(), String> {
        self.agent
            .post(format!("{}/containers/{id}/{action}", self.base))
            .send_empty()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn events(&self) -> Result<impl Iterator<Item = Value>, String> {
        let response = self
            .stream
            .get(format!("{}/events", self.base))
            .query("filters", WATCHED)
            .call()
            .map_err(|error| error.to_string())?;
        let reader = BufReader::new(response.into_body().into_reader());
        Ok(reader
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str(&line).ok()))
    }
}
