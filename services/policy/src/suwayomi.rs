use crate::api::{Api, drift, report, rules};
use serde_json::{Map, Value, json};

fn wanted() -> Map<String, Value> {
    rules(json!({
        "autoDownloadNewChapters": true,
        "excludeEntryWithUnreadChapters": false,
        "globalUpdateInterval": 12.0,
        "downloadAsCbz": true,
    }))
}

fn graphql(api: &impl Api, query: &str) -> Result<Value, String> {
    let reply = api.post("/graphql", &json!({ "query": query }))?;
    match reply.get("errors") {
        Some(errors) => Err(errors.to_string()),
        None => Ok(reply["data"].clone()),
    }
}

pub fn enforce(api: &impl Api) -> Result<usize, String> {
    let wanted = wanted();
    let fields: Vec<&str> = wanted.keys().map(String::as_str).collect();
    let fields = fields.join(" ");
    let current = graphql(api, &format!("{{ settings {{ {fields} }} }}"))?;
    let changes = drift(&current["settings"], &wanted);
    if changes.is_empty() {
        return Ok(0);
    }
    let input: Vec<String> = changes
        .iter()
        .map(|(key, value)| format!("{key}: {value}"))
        .collect();
    let mutation = format!(
        "mutation {{ setSettings(input: {{ settings: {{ {} }} }}) {{ clientMutationId }} }}",
        input.join(", ")
    );
    graphql(api, &mutation)?;
    Ok(report("Suwayomi", "settings", &[&changes]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::fake::Fake;
    use std::collections::BTreeMap;

    fn reply(page: Value) -> Fake {
        Fake::new(BTreeMap::from([("/graphql".to_string(), page)]))
    }

    fn settings(settings: Map<String, Value>) -> Fake {
        reply(json!({ "data": { "settings": settings } }))
    }

    #[test]
    fn a_suwayomi_in_line_needs_nothing() {
        let fake = settings(wanted());
        assert_eq!(Ok(0), enforce(&fake));
        assert_eq!(vec!["POST /graphql"], fake.paths());
    }

    #[test]
    fn a_setting_changed_by_hand_comes_back() {
        let mut current = wanted();
        current.insert("excludeEntryWithUnreadChapters".into(), json!(true));
        let fake = settings(current);
        assert_eq!(Ok(1), enforce(&fake));
        assert_eq!(vec!["POST /graphql", "POST /graphql"], fake.paths());
        let body = fake.body(1);
        let mutation = body["query"].as_str().unwrap_or_default();
        let fixed = "settings: { excludeEntryWithUnreadChapters: false }";
        assert!(mutation.contains(fixed));
    }

    #[test]
    fn an_error_of_graphql_is_reported() {
        let fake = reply(json!({ "errors": [{ "message": "Unauthorized" }] }));
        assert!(enforce(&fake).is_err());
    }
}
