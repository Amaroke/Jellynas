use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub total: u64,
    pub system: u64,
    pub cpus: u64,
    pub memory: u64,
    pub network: Option<Traffic>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Traffic {
    pub at: i64,
    pub received: u64,
    pub sent: u64,
}

fn traffic(stats: &Value) -> Option<Traffic> {
    let at = stats["read"]
        .as_str()?
        .parse::<jiff::Timestamp>()
        .ok()?
        .as_millisecond();
    let networks = stats["networks"]
        .as_object()
        .filter(|found| !found.is_empty())?;
    let sum = |key: &str| {
        networks
            .values()
            .map(|network| network[key].as_u64())
            .sum::<Option<u64>>()
    };
    Some(Traffic {
        at,
        received: sum("rx_bytes")?,
        sent: sum("tx_bytes")?,
    })
}

pub fn sample(stats: &Value) -> Option<Sample> {
    let cpu = &stats["cpu_stats"];
    let memory = &stats["memory_stats"];
    let usage = memory["usage"].as_u64()?;
    let inactive = memory["stats"]["total_inactive_file"]
        .as_u64()
        .or_else(|| memory["stats"]["inactive_file"].as_u64())
        .unwrap_or(0);
    let cores = cpu["cpu_usage"]["percpu_usage"]
        .as_array()
        .map(|cores| cores.len() as u64)
        .filter(|cores| *cores > 0);
    Some(Sample {
        total: cpu["cpu_usage"]["total_usage"].as_u64()?,
        system: cpu["system_cpu_usage"].as_u64()?,
        cpus: cpu["online_cpus"].as_u64().or(cores).unwrap_or(1),
        memory: usage.saturating_sub(inactive),
        network: traffic(stats),
    })
}

pub fn rates(before: &Traffic, after: &Traffic) -> Option<(f64, f64)> {
    let elapsed = after.at.checked_sub(before.at).filter(|ms| *ms > 0)? as f64 / 1000.0;
    let received = after.received.checked_sub(before.received)?;
    let sent = after.sent.checked_sub(before.sent)?;
    Some((received as f64 / elapsed, sent as f64 / elapsed))
}

pub fn cpu(before: &Sample, after: &Sample) -> Option<f64> {
    let system = after
        .system
        .checked_sub(before.system)
        .filter(|delta| *delta > 0)?;
    let total = after.total.checked_sub(before.total)?;
    Some(total as f64 / system as f64 * after.cpus as f64 * 100.0)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct History(BTreeMap<String, BTreeMap<String, Vec<f64>>>);

impl History {
    pub fn record(&mut self, service: &str, metrics: &[(&str, f64)], cap: usize) {
        let series = self.0.entry(service.to_string()).or_default();
        for (name, value) in metrics {
            let points = series.entry((*name).to_string()).or_default();
            points.push(*value);
            if points.len() > cap {
                points.drain(..points.len() - cap);
            }
        }
    }

    pub fn keep(&mut self, services: &[&str]) {
        self.0
            .retain(|service, _| services.contains(&service.as_str()));
    }

    #[cfg(test)]
    pub fn series(&self, service: &str, metric: &str) -> Option<&Vec<f64>> {
        self.0.get(service)?.get(metric)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stats(total: u64, system: u64, usage: u64, inactive: u64) -> Value {
        json!({
            "cpu_stats": {"cpu_usage": {"total_usage": total}, "system_cpu_usage": system, "online_cpus": 4},
            "memory_stats": {"usage": usage, "stats": {"inactive_file": inactive}}
        })
    }

    #[test]
    fn a_sample_reads_the_cpu_counters_and_the_memory_in_use() {
        let expected = Sample {
            total: 100,
            system: 1000,
            cpus: 4,
            memory: 7_000_000,
            network: None,
        };
        assert_eq!(
            Some(expected),
            sample(&stats(100, 1000, 7_500_000, 500_000))
        );
        assert_eq!(None, sample(&json!({"cpu_stats": {}, "memory_stats": {}})));
        let legacy = json!({
            "cpu_stats": {"cpu_usage": {"total_usage": 5, "percpu_usage": [1, 2]}, "system_cpu_usage": 9},
            "memory_stats": {"usage": 900, "stats": {"total_inactive_file": 300, "inactive_file": 100}}
        });
        let expected = Sample {
            total: 5,
            system: 9,
            cpus: 2,
            memory: 600,
            network: None,
        };
        assert_eq!(Some(expected), sample(&legacy));
    }

    #[test]
    fn the_cpu_share_compares_two_samples_like_docker_stats() {
        let before = sample(&stats(1_000, 100_000, 0, 0)).unwrap();
        let after = sample(&stats(3_000, 200_000, 0, 0)).unwrap();
        assert_eq!(Some(8.0), cpu(&before, &after));
        assert_eq!(None, cpu(&after, &after));
        assert_eq!(None, cpu(&after, &before));
    }

    fn traffic_at(read: &str, received: u64, sent: u64) -> Option<Traffic> {
        let mut found = stats(0, 0, 0, 0);
        found["read"] = json!(read);
        found["networks"] = json!({
            "eth0": {"rx_bytes": received, "tx_bytes": sent},
            "eth1": {"rx_bytes": 1_000, "tx_bytes": 0}
        });
        sample(&found)?.network
    }

    #[test]
    fn a_sample_adds_up_the_traffic_of_every_network() {
        assert_eq!(
            Some(Traffic {
                at: 1_795_384_545_000,
                received: 5_000,
                sent: 2_000
            }),
            traffic_at("2026-11-22T21:55:45.000000000Z", 4_000, 2_000)
        );
        let mut alone = stats(0, 0, 0, 0);
        alone["read"] = json!("2026-11-22T21:55:45Z");
        assert_eq!(None, sample(&alone).unwrap().network);
        alone["networks"] = json!({});
        assert_eq!(None, sample(&alone).unwrap().network);
    }

    #[test]
    fn the_rates_divide_the_traffic_by_the_seconds_between_samples() {
        let before = traffic_at("2026-11-22T21:55:45Z", 0, 0).unwrap();
        let after = traffic_at("2026-11-22T21:55:49Z", 8_000, 2_000).unwrap();
        assert_eq!(Some((2_000.0, 500.0)), rates(&before, &after));
        assert_eq!(None, rates(&after, &after));
        assert_eq!(None, rates(&after, &before));
    }

    #[test]
    fn a_history_keeps_the_latest_points_of_each_metric() {
        let mut history = History::default();
        for value in 1..=5 {
            history.record("qbittorrent", &[("download", f64::from(value))], 3);
        }
        history.record("qbittorrent", &[("upload", 9.0)], 3);
        assert_eq!(
            Some(&vec![3.0, 4.0, 5.0]),
            history.series("qbittorrent", "download")
        );
        assert_eq!(Some(&vec![9.0]), history.series("qbittorrent", "upload"));
        history.keep(&["sonarr"]);
        assert_eq!(None, history.series("qbittorrent", "download"));
    }
}
