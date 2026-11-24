mod device;
mod truenas;
mod watch;

use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{env, process, thread};
use watch::{SHUTDOWN, Watch};

const DISCORD: &str = "https://discord.com/api/v10/webhooks";
const USER_AGENT: &str = "DiscordBot (https://github.com/Amaroke/Jellynas, 1.0)";
const INTERVAL: Duration = Duration::from_secs(5);
const PORT: u16 = 8000;

#[derive(Debug, PartialEq)]
struct Settings {
    vendor: String,
    threshold: u8,
    host: String,
    key: String,
    hook: Option<String>,
}

fn settings(env: &HashMap<String, String>) -> Result<Settings, String> {
    let get = |name: &str| env.get(name).map(|value| value.trim()).unwrap_or_default();
    let key = get("TRUENAS_API_KEY");
    if key.is_empty() {
        return Err("TRUENAS_API_KEY must be set in .env".to_string());
    }
    let threshold = match get("UPS_SHUTDOWN_CHARGE") {
        "" => 50,
        found => found
            .parse()
            .ok()
            .filter(|value| *value <= 100)
            .ok_or("UPS_SHUTDOWN_CHARGE must be a percent")?,
    };
    let (id, token) = (
        get("DISCORD_ALERTS_WEBHOOK_ID"),
        get("DISCORD_ALERTS_WEBHOOK_TOKEN"),
    );
    Ok(Settings {
        vendor: Some(get("UPS_VENDOR_ID"))
            .filter(|found| !found.is_empty())
            .unwrap_or("2b89")
            .to_string(),
        threshold,
        host: Some(get("TRUENAS_HOST"))
            .filter(|found| !found.is_empty())
            .unwrap_or("host.docker.internal")
            .to_string(),
        key: key.to_string(),
        hook: (!id.is_empty() && !token.is_empty()).then(|| format!("{id}/{token}")),
    })
}

fn message(event: &str, charge: u8) -> (String, u32) {
    match event {
        "on mains" => (
            format!("Le secteur est revenu, l'onduleur recharge ({charge} %)."),
            0x3FB950,
        ),
        "on battery" => (
            format!("Coupure de courant, le NAS tourne sur l'onduleur ({charge} %)."),
            0xD29922,
        ),
        "low battery" => (
            format!("Batterie de l'onduleur faible ({charge} %)."),
            0xF85149,
        ),
        _ => (
            format!("Batterie à {charge} %, TrueNAS s'éteint proprement."),
            0xF85149,
        ),
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn alert(agent: &ureq::Agent, hook: &str, text: &str, color: u32) -> Result<(), String> {
    let body = json!({
        "embeds": [{"description": text, "color": color}],
        "allowed_mentions": {"parse": []},
    });
    agent
        .post(format!("{DISCORD}/{hook}"))
        .header("Content-Type", "application/json")
        .header("User-Agent", USER_AGENT)
        .send(body.to_string())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn answer(request: tiny_http::Request, code: u16, body: String, kind: &str) {
    let header = tiny_http::Header::from_bytes("Content-Type", kind).expect("valid header");
    let response = tiny_http::Response::from_string(body)
        .with_status_code(code)
        .with_header(header);
    let _ = request.respond(response);
}

fn serve(watch: Arc<Mutex<Watch>>, server: tiny_http::Server) {
    for request in server.incoming_requests() {
        let watch = watch.lock().expect("watch lock");
        match request.url() {
            "/status" => {
                let status: Value = watch.status(now());
                answer(request, 200, status.to_string(), "application/json");
            }
            "/health" if watch.healthy(now()) => answer(request, 200, "ok".into(), "text/plain"),
            "/health" => answer(request, 503, "the UPS cannot be read".into(), "text/plain"),
            _ => answer(request, 404, "not found".into(), "text/plain"),
        }
    }
}

fn healthy() -> bool {
    ureq::get(format!("http://127.0.0.1:{PORT}/health"))
        .call()
        .is_ok()
}

fn main() {
    if env::args().nth(1).as_deref() == Some("health") {
        process::exit(if healthy() { 0 } else { 1 });
    }
    let settings = settings(&env::vars().collect()).unwrap_or_else(|error| {
        eprintln!("{error}");
        process::exit(1);
    });
    let watch = Arc::new(Mutex::new(Watch::new(settings.threshold)));
    let server = tiny_http::Server::http(("0.0.0.0", PORT)).unwrap_or_else(|error| {
        eprintln!("Cannot listen on {PORT}: {error}");
        process::exit(1);
    });
    let served = Arc::clone(&watch);
    thread::spawn(move || serve(served, server));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    println!(
        "Watching the UPS of vendor {}, shutdown at {} %",
        settings.vendor, settings.threshold
    );
    let mut stopped = false;
    loop {
        let read = device::read(&settings.vendor);
        let (events, charge, stopping) = {
            let mut watch = watch.lock().expect("watch lock");
            let events = match read {
                Ok(reading) => watch.update(reading, now()),
                Err(error) => {
                    if watch.failed(error.clone()) {
                        println!("Cannot read the UPS: {error}");
                    }
                    vec![]
                }
            };
            let charge = watch.reading().map_or(0, |found| found.charge);
            (events, charge, watch.stopping)
        };
        for event in events {
            let (text, color) = message(event, charge);
            println!("{text}");
            if let Some(hook) = &settings.hook
                && let Err(error) = alert(&agent, hook, &text, color)
            {
                println!("Cannot alert Discord: {error}");
            }
            if event == SHUTDOWN {
                println!("Shutting TrueNAS down");
            }
        }
        if stopping && !stopped {
            match truenas::shutdown(&settings.host, &settings.key, "UPS battery low") {
                Ok(()) => stopped = true,
                Err(error) => println!("Cannot shut TrueNAS down: {error}"),
            }
        }
        thread::sleep(INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn the_api_key_is_required() {
        assert!(settings(&env(&[])).is_err());
    }

    #[test]
    fn the_defaults_fit_the_ugreen_us3000_and_half_the_battery() {
        let found = settings(&env(&[
            ("TRUENAS_API_KEY", "key"),
            ("DISCORD_ALERTS_WEBHOOK_ID", "1"),
            ("DISCORD_ALERTS_WEBHOOK_TOKEN", "t"),
        ]))
        .unwrap();
        assert_eq!(
            Settings {
                vendor: "2b89".into(),
                threshold: 50,
                host: "host.docker.internal".into(),
                key: "key".into(),
                hook: Some("1/t".into()),
            },
            found
        );
        assert_eq!(
            None,
            settings(&env(&[("TRUENAS_API_KEY", "key")])).unwrap().hook
        );
    }

    #[test]
    fn the_threshold_is_a_percent() {
        let with = |value| {
            settings(&env(&[
                ("TRUENAS_API_KEY", "k"),
                ("UPS_SHUTDOWN_CHARGE", value),
            ]))
        };
        assert_eq!(30, with("30").unwrap().threshold);
        assert!(with("150").is_err());
        assert!(with("half").is_err());
    }

    #[test]
    fn every_event_has_a_message() {
        assert!(message("on battery", 80).0.contains("80 %"));
        assert!(message(SHUTDOWN, 49).0.contains("s'éteint"));
        assert_ne!(message("on mains", 1).1, message("on battery", 1).1);
    }
}
