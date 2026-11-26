use crate::logs::mask;
use crate::report::Report;
use serde_json::Value;
use std::time::Duration;

pub trait Api {
    fn call(&self, method: &str, path: &str, body: Option<&str>) -> Result<(u16, String), String>;

    fn get(&self, path: &str) -> Result<Value, String> {
        let (status, text) = self.call("GET", path, None)?;
        if !(200..300).contains(&status) {
            return Err(format!(
                "HTTP {status}: {}",
                text.chars().take(200).collect::<String>()
            ));
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
}

pub fn fetch(report: &mut Report, api: &impl Api, path: &str) -> Option<Value> {
    match api.get(path) {
        Ok(value) => Some(value),
        Err(error) => {
            report.failed(path, &error);
            None
        }
    }
}

pub enum Key {
    Header(String),
    Query(String),
}

pub struct Http {
    pub base: String,
    pub key: Key,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn clean(error: impl ToString) -> String {
    mask(&error.to_string()).0
}

impl Api for Http {
    fn call(&self, method: &str, path: &str, body: Option<&str>) -> Result<(u16, String), String> {
        let uri = match &self.key {
            Key::Header(_) => format!("{}{path}", self.base),
            Key::Query(key) => format!("{}/api?apikey={key}&{path}", self.base),
        };
        let mut builder = ureq::http::Request::builder().method(method).uri(uri);
        if let Key::Header(key) = &self.key {
            builder = builder.header("X-Api-Key", key);
        }
        let sent = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(body.to_string())
                .map(|request| agent().run(request)),
            None => builder.body(()).map(|request| agent().run(request)),
        };
        let mut response = sent.map_err(clean)?.map_err(clean)?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_string()
            .map_err(clean)?;
        Ok((status, text))
    }
}

pub fn reach(url: &str) -> Result<u16, String> {
    agent()
        .get(url)
        .call()
        .map(|response| response.status().as_u16())
        .map_err(clean)
}

#[cfg(test)]
pub mod fake {
    use super::Api;
    use serde_json::Value;
    use std::collections::BTreeMap;

    pub struct Fake(pub BTreeMap<String, Value>);

    impl Fake {
        pub fn new(pages: &[(&str, Value)]) -> Self {
            Fake(
                pages
                    .iter()
                    .map(|(path, value)| (path.to_string(), value.clone()))
                    .collect(),
            )
        }
    }

    impl Api for Fake {
        fn call(&self, _: &str, path: &str, _: Option<&str>) -> Result<(u16, String), String> {
            match self.0.get(path) {
                Some(Value::String(text)) => Ok((200, text.clone())),
                Some(value) => Ok((200, value.to_string())),
                None => Ok((404, "not found".into())),
            }
        }
    }
}
