use serde_json::{Map, Value};
use std::time::Duration;

pub trait Api {
    fn get(&self, path: &str) -> Result<Value, String>;
    fn send(&self, method: &str, path: &str, body: &Value) -> Result<(), String>;
    fn text(&self, path: &str) -> Result<String, String>;
    fn post(&self, path: &str, body: &Value) -> Result<Value, String>;
}

pub struct Http {
    pub base: String,
    pub header: Option<(&'static str, String)>,
}

impl Http {
    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into()
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<ureq::http::Response<ureq::Body>, String> {
        let mut builder = ureq::http::Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.base));
        if let Some((name, value)) = &self.header {
            builder = builder.header(*name, value);
        }
        let sent = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(body.to_string())
                .map(|request| Self::agent().run(request)),
            None => builder.body(()).map(|request| Self::agent().run(request)),
        };
        let response = sent.map_err(|error| error.to_string())?;
        response.map_err(|error| error.to_string())
    }
}

impl Api for Http {
    fn get(&self, path: &str) -> Result<Value, String> {
        let mut response = self.request("GET", path, None)?;
        response
            .body_mut()
            .read_json()
            .map_err(|error| error.to_string())
    }

    fn send(&self, method: &str, path: &str, body: &Value) -> Result<(), String> {
        self.request(method, path, Some(body)).map(|_| ())
    }

    fn text(&self, path: &str) -> Result<String, String> {
        let mut response = self.request("GET", path, None)?;
        response
            .body_mut()
            .read_to_string()
            .map_err(|error| error.to_string())
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        let mut response = self.request("POST", path, Some(body))?;
        response
            .body_mut()
            .read_json()
            .map_err(|error| error.to_string())
    }
}

pub fn rules(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

pub fn drift(current: &Value, wanted: &Map<String, Value>) -> Map<String, Value> {
    wanted
        .iter()
        .filter(|(key, value)| current.get(key.as_str()) != Some(value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

pub fn merged(current: &Value, changes: &Map<String, Value>) -> Value {
    let mut whole = current.as_object().cloned().unwrap_or_default();
    whole.extend(changes.clone());
    Value::Object(whole)
}

pub fn keys(changes: &[&Map<String, Value>]) -> String {
    let mut names: Vec<&str> = changes
        .iter()
        .flat_map(|change| change.keys())
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    names.join(", ")
}

pub fn settle(
    api: &impl Api,
    method: &str,
    path: &str,
    current: &Value,
    wanted: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    let changes = drift(current, wanted);
    if !changes.is_empty() {
        api.send(method, path, &merged(current, &changes))?;
    }
    Ok(changes)
}

pub fn report(app: &str, page: &str, changes: &[&Map<String, Value>]) -> usize {
    if changes.iter().all(|change| change.is_empty()) {
        return 0;
    }
    println!("{app} {page}: set {}", keys(changes));
    1
}

#[cfg(test)]
pub mod fake {
    use super::Api;
    use serde_json::Value;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    pub struct Fake {
        pub pages: BTreeMap<String, Value>,
        pub sent: RefCell<Vec<(String, String, Value)>>,
    }

    impl Fake {
        pub fn new(pages: BTreeMap<String, Value>) -> Self {
            Fake {
                pages,
                sent: RefCell::new(vec![]),
            }
        }

        pub fn paths(&self) -> Vec<String> {
            self.sent
                .borrow()
                .iter()
                .map(|(method, path, _)| format!("{method} {path}"))
                .collect()
        }

        pub fn body(&self, index: usize) -> Value {
            self.sent.borrow()[index].2.clone()
        }
    }

    impl Api for Fake {
        fn get(&self, path: &str) -> Result<Value, String> {
            self.pages
                .get(path)
                .cloned()
                .ok_or(format!("no page {path}"))
        }

        fn send(&self, method: &str, path: &str, body: &Value) -> Result<(), String> {
            self.sent
                .borrow_mut()
                .push((method.to_string(), path.to_string(), body.clone()));
            Ok(())
        }

        fn text(&self, path: &str) -> Result<String, String> {
            self.send("GET", path, &Value::Null)?;
            let page = self.pages.get(path).and_then(Value::as_str);
            Ok(page.unwrap_or("OK").to_string())
        }

        fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
            self.send("POST", path, body)?;
            self.get(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn drift_lists_only_what_differs() {
        let wanted = rules(json!({"a": 1, "b": 2}));
        assert_eq!(
            rules(json!({"b": 2})),
            drift(&json!({"a": 1, "b": 3}), &wanted)
        );
    }

    #[test]
    fn a_merge_keeps_every_other_field() {
        let whole = merged(&json!({"a": 1, "kept": true}), &rules(json!({"a": 2})));
        assert_eq!(json!({"a": 2, "kept": true}), whole);
    }

    #[test]
    fn keys_are_listed_in_order() {
        let first = rules(json!({"b": 1, "a": 2}));
        let second = rules(json!({"c": 3}));
        assert_eq!("a, b, c", keys(&[&first, &second]));
    }
}
