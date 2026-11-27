mod api;
mod arr;
mod input;
mod lazylibrarian;
mod logs;
mod report;
mod upstream;

use api::{Api, Http, Key, reach};
use arr::Kind;
use jiff::Timestamp;
use report::Report;
use std::io::Read;
use std::{env, io, process};

const USAGE: &str = "Usage: check radarr|sonarr|lazylibrarian < facts\n       check api radarr|sonarr|prowlarr|lazylibrarian|discord-posts METHOD PATH [JSON|-]";
const QBITTORRENT: &str = "http://gluetun:8080/api/v2/app/version";

fn http(app: &str) -> Result<Http, String> {
    let (base, variable) = match app {
        "radarr" => ("http://radarr:7878", "RADARR_API_KEY"),
        "sonarr" => ("http://sonarr:8989", "SONARR_API_KEY"),
        "prowlarr" => ("http://prowlarr:9696", "PROWLARR_API_KEY"),
        "lazylibrarian" => ("http://lazylibrarian:5299", "LAZYLIBRARIAN_API_KEY"),
        "discord-posts" => {
            return Ok(Http {
                base: "http://discord-posts:8000".into(),
                key: Key::None,
            });
        }
        _ => return Err(format!("Unknown app {app}")),
    };
    let key = env::var(variable)
        .ok()
        .filter(|key| !key.is_empty())
        .ok_or(format!("{variable} is not set in .env"))?;
    Ok(Http {
        base: base.into(),
        key: if app == "lazylibrarian" {
            Key::Query(key)
        } else {
            Key::Header(key)
        },
    })
}

fn check(app: &str) -> Result<(), String> {
    let mut text = String::new();
    io::stdin()
        .read_to_string(&mut text)
        .map_err(|error| error.to_string())?;
    let input = input::parse(&text);
    let now = Timestamp::now();
    let mut report = Report::default();
    let names = [app, "prowlarr", "gluetun", "qbittorrent", "ratio"];
    upstream::containers(&mut report, &input, &names);
    match app {
        "radarr" | "sonarr" => {
            let kind = if app == "radarr" {
                Kind::Radarr
            } else {
                Kind::Sonarr
            };
            let statuses = input.indexers.get(app).cloned().unwrap_or_default();
            arr::check(&mut report, &http(app)?, kind, now, &statuses);
        }
        _ => lazylibrarian::check(&mut report, &http(app)?, &input),
    }
    upstream::prowlarr(&mut report, &http("prowlarr")?, now);
    upstream::qbittorrent(&mut report, reach(QBITTORRENT));
    upstream::ratio(&mut report, &input);
    upstream::query_limits(&mut report, &input);
    for (source, lines) in &input.logs {
        logs::summarize(&mut report, source, lines);
    }
    let out = report.into_json(app);
    println!(
        "{}",
        serde_json::to_string_pretty(&out).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn call(args: &[String]) -> Result<(), String> {
    let [app, method, path, rest @ ..] = args else {
        return Err(USAGE.into());
    };
    let mut body = rest.first().cloned();
    if body.as_deref() == Some("-") {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| error.to_string())?;
        body = Some(text);
    }
    let (status, text) = http(app)?.call(method, path, body.as_deref())?;
    eprintln!("HTTP {status}");
    println!("{}", logs::mask(&text).0);
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(format!("{app} answered {status}"))
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some(app @ ("radarr" | "sonarr" | "lazylibrarian")) => check(app),
        Some("api") => call(&args[1..]),
        _ => Err(USAGE.into()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        process::exit(1);
    }
}
