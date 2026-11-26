use serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    High,
    Medium,
    Low,
}

impl Severity {
    fn name(self) -> &'static str {
        match self {
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    pub source: String,
    pub detail: String,
}

#[derive(Default)]
pub struct Report {
    pub facts: Map<String, Value>,
    pub findings: Vec<Finding>,
    pub errors: Vec<String>,
}

impl Report {
    pub fn fact(&mut self, key: &str, value: Value) {
        self.facts.insert(key.into(), value);
    }

    pub fn flag(&mut self, severity: Severity, source: &str, detail: impl Into<String>) {
        self.findings.push(Finding {
            severity,
            source: source.into(),
            detail: detail.into(),
        });
    }

    pub fn failed(&mut self, what: &str, error: &str) {
        self.errors.push(format!("{what}: {error}"));
    }

    #[cfg(test)]
    pub fn flagged(&self, severity: Severity, part: &str) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == severity && finding.detail.contains(part))
    }

    pub fn into_json(mut self, app: &str) -> Value {
        self.findings.sort_by_key(|finding| finding.severity);
        let findings: Vec<Value> = self
            .findings
            .iter()
            .map(|finding| {
                json!({
                    "severity": finding.severity.name(),
                    "source": finding.source,
                    "detail": finding.detail,
                })
            })
            .collect();
        json!({
            "app": app,
            "findings": findings,
            "errors": self.errors,
            "facts": self.facts,
        })
    }
}

pub fn trimmed(value: &Value, limit: usize) -> Value {
    match value.as_array() {
        Some(items) => json!({
            "count": items.len(),
            "items": items.iter().take(limit).cloned().collect::<Vec<_>>(),
        }),
        None => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn findings_come_out_most_severe_first() {
        let mut report = Report::default();
        report.flag(Severity::Low, "a", "low");
        report.flag(Severity::High, "b", "high");
        let out = report.into_json("radarr");
        assert_eq!(out["findings"][0]["severity"], "high");
        assert_eq!(out["findings"][1]["severity"], "low");
    }

    #[test]
    fn a_long_list_is_cut_and_counted() {
        let out = trimmed(&json!([1, 2, 3]), 2);
        assert_eq!(out, json!({"count": 3, "items": [1, 2]}));
    }
}
