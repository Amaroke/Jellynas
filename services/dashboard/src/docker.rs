use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::time::Duration;

const LABELLED: &str = r#"{"label":["dashboard.section"]}"#;
const WATCHED: &str = r#"{"type":["container"],"event":["create","start","restart","stop","die","destroy","pause","unpause","health_status"]}"#;

const LOGS: u64 = 8 * 1024 * 1024;

pub fn demux(body: &[u8]) -> String {
    let framed = body.len() >= 8 && body[0] <= 2 && body[1..4] == [0, 0, 0];
    if !framed {
        return String::from_utf8_lossy(body).into_owned();
    }
    let mut text = Vec::new();
    let mut rest = body;
    while rest.len() >= 8 {
        let size = u32::from_be_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
        let end = (8 + size).min(rest.len());
        text.extend_from_slice(&rest[8..end]);
        rest = &rest[end..];
    }
    String::from_utf8_lossy(&text).into_owned()
}

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

    pub fn logs(&self, id: &str, since: u64, tail: u32) -> Result<String, String> {
        let mut response = self
            .agent
            .get(format!("{}/containers/{id}/logs", self.base))
            .query("stdout", "true")
            .query("stderr", "true")
            .query("since", since.to_string())
            .query("tail", tail.to_string())
            .config()
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .call()
            .map_err(|error| error.to_string())?;
        let body = response
            .body_mut()
            .with_config()
            .limit(LOGS)
            .read_to_vec()
            .map_err(|error| error.to_string())?;
        Ok(demux(&body))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_lose_their_stream_headers() {
        let mut framed = vec![1, 0, 0, 0, 0, 0, 0, 6];
        framed.extend(
            b"hello
",
        );
        framed.extend([2, 0, 0, 0, 0, 0, 0, 4]);
        framed.extend(
            b"bye
",
        );
        assert_eq!(
            "hello
bye
",
            demux(&framed)
        );
        assert_eq!(
            "plain tty
",
            demux(
                b"plain tty
"
            )
        );
    }
}
