use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::LazyLock;

pub const MAX_LENGTH: usize = 4096;
pub const TITLE_LENGTH: usize = 256;

static HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A---\r?\n((?s:.*?))\r?\n---\r?\n").expect("valid regex"));
static COVERS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^covers: #(\d+)\r?$").expect("valid regex"));
static BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[-*+]\s|\d+[.)]\s|-#|#|>|\||```)").expect("valid regex"));
static BRACKETED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(https?://[^>]+)>").expect("valid regex"));
static BLANKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").expect("valid regex"));

pub struct Message {
    pub content: String,
    pub embed: Map<String, Value>,
}

pub fn covers(source: &str) -> Option<u64> {
    let header = HEADER.captures(source)?;
    COVERS.captures(&header[1])?[1].parse().ok()
}

fn without_header(source: &str) -> &str {
    HEADER
        .find(source)
        .map_or(source, |found| &source[found.end()..])
}

pub fn substitute(text: &str, env: &HashMap<String, String>) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let (name, used) = if let Some(braced) = after.strip_prefix('{') {
            let end = braced
                .find('}')
                .ok_or_else(|| "an unclosed ${ placeholder".to_string())?;
            (&braced[..end], end + 2)
        } else if let Some(escaped) = after.strip_prefix('$') {
            out.push('$');
            rest = escaped;
            continue;
        } else {
            let end = after
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(after.len());
            (&after[..end], end)
        };
        let valid = name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            return Err("a $ that is no placeholder, write $$ for a dollar".into());
        }
        let value = env
            .get(name)
            .ok_or_else(|| format!("${name}, which .env does not set"))?;
        out.push_str(value);
        rest = &after[used..];
    }
    out.push_str(rest);
    Ok(out)
}

pub fn unwrap(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut fence = false;
    for line in text.split('\n') {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        } else if let Some(last) = lines
            .last_mut()
            .filter(|last| !fence && !last.trim().is_empty() && !last.trim_start().starts_with('#'))
            && !line.trim().is_empty()
            && !BLOCK.is_match(line)
        {
            last.push(' ');
            last.push_str(line.trim());
            continue;
        }
        lines.push(line.to_string());
    }
    lines.join("\n")
}

pub fn render(source: &str, env: &HashMap<String, String>) -> Result<Message, String> {
    let filled = substitute(without_header(source).replace("\r\n", "\n").as_str(), env)?;
    let text = unwrap(filled.trim());
    let (videos, kept): (Vec<&str>, Vec<&str>) =
        text.split('\n').partition(|line| line.starts_with("📺 "));
    let content = BRACKETED
        .replace_all(
            &videos
                .iter()
                .map(|line| line.trim())
                .collect::<Vec<_>>()
                .join("\n"),
            "$1",
        )
        .into_owned();
    let text = BLANKS
        .replace_all(&kept.join("\n"), "\n\n")
        .trim()
        .to_string();
    let mut embed = Map::new();
    match text.strip_prefix("# ") {
        Some(titled) => {
            let (title, description) = titled.split_once('\n').unwrap_or((titled, ""));
            embed.insert("title".into(), json!(title.trim()));
            embed.insert("description".into(), json!(description.trim()));
        }
        None => {
            embed.insert("description".into(), json!(text));
        }
    }
    let length = |key: &str| {
        embed
            .get(key)
            .and_then(Value::as_str)
            .map_or(0, |text| text.chars().count())
    };
    if length("title") > TITLE_LENGTH {
        return Err(format!(
            "a title of {} characters, Discord takes {TITLE_LENGTH} at most",
            length("title")
        ));
    }
    if length("description") > MAX_LENGTH {
        return Err(format!(
            "{} characters once filled in, Discord takes {MAX_LENGTH} at most",
            length("description")
        ));
    }
    Ok(Message { content, embed })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> HashMap<String, String> {
        HashMap::from([
            ("DOMAIN".into(), "example.com".into()),
            ("CADDY_HTTPS_PORT".into(), "443".into()),
        ])
    }

    fn description(message: &Message) -> &str {
        message.embed["description"].as_str().unwrap()
    }

    #[test]
    fn fills_the_placeholders_from_the_env() {
        let message = render("https://jellyfin.${DOMAIN}:$CADDY_HTTPS_PORT $$5\n", &env()).unwrap();
        assert_eq!("https://jellyfin.example.com:443 $5", description(&message));
        assert!(!message.embed.contains_key("title"));
    }

    #[test]
    fn refuses_a_placeholder_missing_from_the_env() {
        let error = render("${MISSING}", &env()).err().unwrap();
        assert!(error.contains("MISSING"), "{error}");
    }

    #[test]
    fn the_first_heading_is_the_title_of_the_embed() {
        let message = render(
            "# Liens\n\n## Jellyfin\nhttps://jellyfin.${DOMAIN}\n",
            &env(),
        )
        .unwrap();
        assert_eq!("Liens", message.embed["title"]);
        assert_eq!(
            "## Jellyfin\nhttps://jellyfin.example.com",
            description(&message)
        );
    }

    #[test]
    fn a_video_line_goes_in_the_content_so_discord_embeds_it() {
        let message = render(
            "# Tuto\n\nIntro.\n\n📺 **Vidéo** <https://youtu.be/x>\n\n## Next\n",
            &env(),
        )
        .unwrap();
        assert_eq!("📺 **Vidéo** https://youtu.be/x", message.content);
        assert_eq!("Intro.\n\n## Next", description(&message));
    }

    #[test]
    fn joins_the_lines_wrapped_in_the_file() {
        let text = "# Title\n\nA long\nparagraph.\n-# A long\nsubtext.\n\n- An item\n  wrapped\n- Another\n\n```\nkept\nas is\n```";
        let joined = "# Title\n\nA long paragraph.\n-# A long subtext.\n\n- An item wrapped\n- Another\n\n```\nkept\nas is\n```";
        assert_eq!(joined, unwrap(text));
    }

    #[test]
    fn refuses_a_message_too_long_for_discord() {
        let error = render(&"x".repeat(MAX_LENGTH + 1), &env()).err().unwrap();
        assert!(error.contains("4096"), "{error}");
    }

    #[test]
    fn counts_characters_not_bytes() {
        assert!(render(&"é".repeat(MAX_LENGTH), &env()).is_ok());
    }

    #[test]
    fn refuses_a_title_too_long_for_discord() {
        let error = render(&format!("# {}\n\nx", "t".repeat(TITLE_LENGTH + 1)), &env())
            .err()
            .unwrap();
        assert!(error.contains("256"), "{error}");
    }

    #[test]
    fn the_header_of_a_patch_note_stays_off_discord() {
        let source = "---\ncovers: #507\n---\n\n# Patch note\n\nNouveau.\n";
        assert_eq!(Some(507), covers(source));
        let message = render(source, &env()).unwrap();
        assert_eq!("Patch note", message.embed["title"]);
        assert_eq!("Nouveau.", description(&message));
    }

    #[test]
    fn a_note_without_its_header_covers_nothing() {
        assert_eq!(None, covers("# Patch note\n\ncovers: #507\n"));
        assert_eq!(None, covers("---\nother: 1\n---\n"));
    }
}
