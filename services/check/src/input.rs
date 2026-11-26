use std::collections::BTreeMap;

#[derive(Default)]
pub struct Input {
    pub states: BTreeMap<String, String>,
    pub mounts: BTreeMap<String, Vec<String>>,
    pub logs: BTreeMap<String, Vec<String>>,
}

pub fn parse(text: &str) -> Input {
    let mut input = Input::default();
    for line in text.lines() {
        let mut parts = line.splitn(3, '|');
        let (Some(kind), Some(name), Some(value)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let value = value.trim_end().to_string();
        match kind {
            "state" => {
                input.states.insert(name.into(), value);
            }
            "mount" => input.mounts.entry(name.into()).or_default().push(value),
            "log" => input.logs.entry(name.into()).or_default().push(value),
            _ => {}
        }
    }
    input
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_each_line_by_kind_and_container() {
        let input = parse(
            "state|radarr|running healthy\nmount|lazylibrarian|/config\nlog|radarr|a|b\nnoise\n",
        );
        assert_eq!(input.states["radarr"], "running healthy");
        assert_eq!(input.mounts["lazylibrarian"], vec!["/config"]);
        assert_eq!(input.logs["radarr"], vec!["a|b"]);
    }
}
