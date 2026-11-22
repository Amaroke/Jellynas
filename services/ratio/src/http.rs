use std::time::Duration;

pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub retry_after: Option<String>,
    pub cookie: Option<String>,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub fn agent(proxy: Option<&str>, seconds: u64) -> Result<ureq::Agent, String> {
    let proxy = match proxy.filter(|url| !url.is_empty()) {
        Some(url) => Some(ureq::Proxy::new(url).map_err(|error| error.to_string())?),
        None => None,
    };
    Ok(ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(seconds)))
        .http_status_as_error(false)
        .proxy(proxy)
        .build()
        .into())
}

pub fn call(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    headers: &[(&str, String)],
    body: Option<(&str, Vec<u8>)>,
) -> Result<Reply, String> {
    let mut builder = ureq::http::Request::builder().method(method).uri(url);
    for (name, value) in headers {
        builder = builder.header(*name, value.as_str());
    }
    let sent = match body {
        Some((kind, bytes)) => {
            let request = builder
                .header("content-type", kind)
                .body(bytes)
                .map_err(|error| error.to_string())?;
            agent.run(request)
        }
        None => {
            let request = builder.body(()).map_err(|error| error.to_string())?;
            agent.run(request)
        }
    };
    let mut response = sent.map_err(|error| error.to_string())?;
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    let retry_after = header("retry-after");
    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("SID="))
        .and_then(|value| value.split(';').next())
        .map(str::to_string);
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_vec()
        .map_err(|error| error.to_string())?;
    Ok(Reply {
        status,
        body,
        retry_after,
        cookie,
    })
}

pub fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte))
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn form(fields: &[(&str, String)]) -> Vec<u8> {
    fields
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
        .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_form_encodes_every_reserved_byte() {
        let body = form(&[("hashes", "a|b".into()), ("json", "{\"x\": 1}".into())]);
        assert_eq!(
            "hashes=a%7Cb&json=%7B%22x%22%3A%201%7D",
            String::from_utf8(body).unwrap()
        );
    }
}
