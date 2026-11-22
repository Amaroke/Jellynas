use crate::widgets::{Figure, Reading, Row, bytes};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, ring};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use serde_json::{Value, json};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;
use tungstenite::{Connector, Message};

const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
struct Pinned {
    provider: CryptoProvider,
    fingerprint: Vec<u8>,
}

pub fn fingerprint(text: &str) -> Option<Vec<u8>> {
    let digits: String = text.chars().filter(char::is_ascii_hexdigit).collect();
    if digits.len() != 64 {
        return None;
    }
    (0..32)
        .map(|index| u8::from_str_radix(&digits[index * 2..index * 2 + 2], 16).ok())
        .collect()
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        certificate: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let digest = ::ring::digest::digest(&::ring::digest::SHA256, certificate.as_ref());
        if digest.as_ref() == self.fingerprint.as_slice() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "the certificate of TrueNAS does not match TRUENAS_FINGERPRINT".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signed,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signed,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn connector(fingerprint: Vec<u8>) -> Result<Connector, String> {
    let provider = ring::default_provider();
    let config = ClientConfig::builder_with_provider(Arc::new(provider.clone()))
        .with_safe_default_protocol_versions()
        .map_err(|error| error.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned {
            provider,
            fingerprint,
        }))
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}

pub fn address(url: &str) -> Result<String, String> {
    let host = url
        .strip_prefix("wss://")
        .and_then(|rest| rest.split('/').next())
        .filter(|host| !host.is_empty())
        .ok_or("TRUENAS_URL must start with wss://")?;
    let after = host.rsplit(']').next().unwrap_or(host);
    Ok(if after.contains(':') {
        host.to_string()
    } else {
        format!("{host}:443")
    })
}

pub fn read(url: &str, key: &str, fingerprint: Vec<u8>) -> Result<Value, String> {
    let target = address(url)?
        .to_socket_addrs()
        .map_err(|error| error.to_string())?
        .next()
        .ok_or("TRUENAS_URL resolves to no address")?;
    let stream = TcpStream::connect_timeout(&target, TIMEOUT).map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|error| error.to_string())?;
    let (mut socket, _) =
        tungstenite::client_tls_with_config(url, stream, None, Some(connector(fingerprint)?))
            .map_err(|error| error.to_string())?;
    let mut next = 0;
    let mut call = |method: &str, params: Value| -> Result<Value, String> {
        next += 1;
        let request = json!({"jsonrpc": "2.0", "id": next, "method": method, "params": params});
        socket
            .send(Message::text(request.to_string()))
            .map_err(|error| error.to_string())?;
        loop {
            let message = socket.read().map_err(|error| error.to_string())?;
            let Ok(text) = message.to_text() else {
                continue;
            };
            let Ok(answer) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            if answer["id"].is_null()
                && let Some(error) = answer.get("error")
            {
                return Err(format!("{method}: {}", error["message"]));
            }
            if answer["id"] != next {
                continue;
            }
            if let Some(error) = answer.get("error") {
                return Err(format!("{method}: {}", error["message"]));
            }
            return Ok(answer["result"].clone());
        }
    };
    if call("auth.login_with_api_key", json!([key]))? != json!(true) {
        return Err("TrueNAS refused the API key".into());
    }
    let found = json!({
        "pools": call("pool.query", json!([[], {"select": ["name", "status", "healthy", "scan", "size", "allocated", "free"]}]))?,
        "disks": call("disk.query", json!([[], {"select": ["name", "model", "size"]}]))?,
        "temperatures": call("disk.temperatures", json!([]))?,
        "alerts": call("alert.list", json!([]))?,
    });
    let _ = socket.close(None);
    Ok(found)
}

fn instant(value: &Value) -> Option<u64> {
    value["$date"]
        .as_u64()
        .map(|millis| millis / 1000)
        .or_else(|| value.as_u64())
}

fn ago(now: u64, then: u64) -> String {
    let seconds = now.saturating_sub(then);
    let (days, hours) = (seconds / 86400, seconds / 3600 % 24);
    if days > 0 {
        format!("{days}d ago")
    } else {
        format!("{hours}h ago")
    }
}

pub fn reading(found: &Value, now: u64) -> Result<Reading, String> {
    let pools = found["pools"].as_array().ok_or("no pools")?;
    let healthy = pools
        .iter()
        .filter(|pool| pool["healthy"].as_bool() == Some(true))
        .count();
    let size: f64 = pools.iter().filter_map(|pool| pool["size"].as_f64()).sum();
    let used: f64 = pools
        .iter()
        .filter_map(|pool| pool["allocated"].as_f64())
        .sum();
    let free: f64 = pools.iter().filter_map(|pool| pool["free"].as_f64()).sum();
    let scans: Vec<&Value> = pools
        .iter()
        .map(|pool| &pool["scan"])
        .filter(|scan| scan["function"].as_str() == Some("SCRUB"))
        .collect();
    let scrub = if let Some(scan) = scans
        .iter()
        .find(|scan| scan["state"].as_str() == Some("SCANNING"))
    {
        format!("Running {:.0}%", scan["percentage"].as_f64().unwrap_or(0.0))
    } else {
        scans
            .iter()
            .filter_map(|scan| instant(&scan["end_time"]))
            .min()
            .map_or_else(|| "Never".to_string(), |at| ago(now, at))
    };
    let alerts: Vec<&Value> = found["alerts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|alert| alert["dismissed"].as_bool() != Some(true))
        .collect();
    let figure = |label: &str, value: String| Figure {
        label: label.into(),
        value,
    };
    let row = |title: String, detail: String| Row {
        title,
        detail,
        progress: None,
        image: None,
        id: None,
    };
    let mut rows: Vec<Row> = alerts
        .iter()
        .map(|alert| {
            row(
                alert["formatted"]
                    .as_str()
                    .or(alert["text"].as_str())
                    .unwrap_or("Alert")
                    .to_string(),
                alert["level"].as_str().unwrap_or("INFO").to_lowercase(),
            )
        })
        .collect();
    rows.extend(pools.iter().filter_map(|pool| {
        let share =
            pool["allocated"].as_f64()? / pool["size"].as_f64().filter(|size| *size > 0.0)?;
        Some(Row {
            progress: Some(share),
            ..row(
                format!("Pool {}", pool["name"].as_str()?),
                format!(
                    "{} \u{b7} {} free",
                    pool["status"].as_str().unwrap_or("UNKNOWN"),
                    bytes(pool["free"].as_f64().unwrap_or(0.0))
                ),
            )
        })
    }));
    rows.extend(
        found["disks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|disk| {
                let name = disk["name"].as_str()?;
                let temperature = found["temperatures"][name].as_f64().map_or_else(
                    || "no temperature".to_string(),
                    |degrees| format!("{degrees:.0} \u{b0}C"),
                );
                Some(row(
                    format!("{name} \u{b7} {}", disk["model"].as_str().unwrap_or("disk")),
                    format!(
                        "{} \u{b7} {temperature}",
                        bytes(disk["size"].as_f64().unwrap_or(0.0))
                    ),
                ))
            }),
    );
    Ok(Reading {
        figures: vec![
            figure("Healthy pools", format!("{healthy} / {}", pools.len())),
            figure(
                "Used",
                if size > 0.0 {
                    format!("{:.1}%", used / size * 100.0)
                } else {
                    "\u{2014}".into()
                },
            ),
            figure("Free", bytes(free)),
            figure("Last scrub", scrub),
            figure("Alerts", alerts.len().to_string()),
        ],
        rows,
        idle: Some("No pool".into()),
        ..Reading::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_795_350_000;

    fn figures(reading: &Reading) -> Vec<(&str, &str)> {
        reading
            .figures
            .iter()
            .map(|figure| (figure.label.as_str(), figure.value.as_str()))
            .collect()
    }

    fn found() -> Value {
        json!({
            "pools": [{"name": "tank", "status": "ONLINE", "healthy": true,
                       "scan": {"function": "SCRUB", "state": "FINISHED", "end_time": {"$date": 1_795_090_800_000_u64}},
                       "size": 4096.0, "allocated": 3072.0, "free": 1024.0}],
            "disks": [{"name": "sda", "model": "WDC WD40", "size": 4096.0}, {"name": "sdb", "model": "Virtual_Disk", "size": 2048.0}],
            "temperatures": {"sda": 38.4, "sdb": null},
            "alerts": [
                {"formatted": "Pool tank is 80% full", "level": "WARNING", "dismissed": false},
                {"formatted": "Old news", "level": "INFO", "dismissed": true}
            ]
        })
    }

    #[test]
    fn truenas_sums_up_the_pools_the_disks_and_the_alerts() {
        let reading = reading(&found(), NOW).unwrap();
        assert_eq!(
            vec![
                ("Healthy pools", "1 / 1"),
                ("Used", "75.0%"),
                ("Free", "1.0 KiB"),
                ("Last scrub", "3d ago"),
                ("Alerts", "1")
            ],
            figures(&reading)
        );
        let rows: Vec<(&str, &str, Option<f64>)> = reading
            .rows
            .iter()
            .map(|row| (row.title.as_str(), row.detail.as_str(), row.progress))
            .collect();
        assert_eq!(
            vec![
                ("Pool tank is 80% full", "warning", None),
                ("Pool tank", "ONLINE \u{b7} 1.0 KiB free", Some(0.75)),
                ("sda \u{b7} WDC WD40", "4.0 KiB \u{b7} 38 \u{b0}C", None),
                (
                    "sdb \u{b7} Virtual_Disk",
                    "2.0 KiB \u{b7} no temperature",
                    None
                ),
            ],
            rows
        );
    }

    #[test]
    fn a_scrub_in_progress_or_never_run_says_so() {
        let mut running = found();
        running["pools"][0]["scan"] =
            json!({"function": "SCRUB", "state": "SCANNING", "percentage": 42.4});
        assert_eq!(
            ("Last scrub", "Running 42%"),
            figures(&reading(&running, NOW).unwrap())[3]
        );
        let mut never = found();
        never["pools"][0]["scan"] = json!({"function": null, "state": null, "end_time": null});
        assert_eq!(
            ("Last scrub", "Never"),
            figures(&reading(&never, NOW).unwrap())[3]
        );
        assert!(reading(&json!({}), NOW).is_err());
        let mut resilver = found();
        resilver["pools"][0]["scan"] =
            json!({"function": "RESILVER", "state": "SCANNING", "percentage": 10.0});
        assert_eq!(
            ("Last scrub", "Never"),
            figures(&reading(&resilver, NOW).unwrap())[3]
        );
    }

    #[test]
    fn the_address_gets_the_https_port_unless_it_names_one() {
        assert_eq!(
            Ok("host.docker.internal:443".to_string()),
            address("wss://host.docker.internal/api/current")
        );
        assert_eq!(
            Ok("nas:8443".to_string()),
            address("wss://nas:8443/api/current")
        );
        assert_eq!(
            Ok("[fd00::25]:443".to_string()),
            address("wss://[fd00::25]/api/current")
        );
        assert_eq!(
            Ok("[fd00::25]:8443".to_string()),
            address("wss://[fd00::25]:8443/api/current")
        );
        assert!(address("ws://nas/api/current").is_err());
    }

    #[test]
    fn a_fingerprint_reads_with_or_without_colons() {
        let colons = "69:01:67:EE:32:9E:37:33:3D:9B:0F:6B:25:3D:A0:E9:B8:B7:65:DF:40:75:33:78:35:1C:6B:54:37:5E:F0:37";
        let found = fingerprint(colons).unwrap();
        assert_eq!(32, found.len());
        assert_eq!(0x69, found[0]);
        assert_eq!(
            Some(found),
            fingerprint(&colons.replace(':', "").to_lowercase())
        );
        assert_eq!(None, fingerprint("69:01"));
    }
}
