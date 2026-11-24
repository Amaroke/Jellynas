use crate::device::Reading;
use serde_json::{Value, json};

pub const STALE: u64 = 30;
pub const SHUTDOWN: &str = "shutdown";

pub fn state(reading: Option<&Reading>) -> &'static str {
    match reading {
        None => "unreachable",
        Some(found) if found.mains => "on mains",
        Some(found) if found.low || found.imminent => "low battery",
        Some(_) => "on battery",
    }
}

pub struct Watch {
    threshold: u8,
    reading: Option<Reading>,
    read_at: Option<u64>,
    since: Option<u64>,
    on_battery: u32,
    pub stopping: bool,
    error: Option<String>,
}

impl Watch {
    pub fn new(threshold: u8) -> Self {
        Watch {
            threshold,
            reading: None,
            read_at: None,
            since: None,
            on_battery: 0,
            stopping: false,
            error: None,
        }
    }

    pub fn reading(&self) -> Option<&Reading> {
        self.reading.as_ref()
    }

    pub fn update(&mut self, reading: Reading, now: u64) -> Vec<&'static str> {
        let mut events = vec![];
        let before = self.read_at.map(|_| state(self.reading.as_ref()));
        let after = state(Some(&reading));
        if before != Some(after) {
            self.since = Some(now);
            let first = before.is_none_or(|found| found == "unreachable");
            if !first || after != "on mains" {
                events.push(after);
            }
        }
        self.on_battery = if reading.mains {
            0
        } else {
            self.on_battery + 1
        };
        let low = reading.low || reading.imminent || reading.charge <= self.threshold;
        if self.on_battery >= 2 && low && !self.stopping {
            self.stopping = true;
            events.push(SHUTDOWN);
        }
        self.reading = Some(reading);
        self.read_at = Some(now);
        self.error = None;
        events
    }

    pub fn failed(&mut self, error: String) -> bool {
        let new = self.error.as_ref() != Some(&error);
        self.error = Some(error);
        new
    }

    fn fresh(&self, now: u64) -> Option<&Reading> {
        self.read_at
            .filter(|at| now.saturating_sub(*at) <= STALE)
            .and(self.reading.as_ref())
    }

    pub fn status(&self, now: u64) -> Value {
        let reading = self.fresh(now);
        let mut found = reading.map_or_else(|| json!({}), Reading::json);
        found["state"] = json!(state(reading));
        found["since"] = json!(self.since);
        found["threshold"] = json!(self.threshold);
        found["stopping"] = json!(self.stopping);
        found["error"] = json!(self.error);
        found
    }

    pub fn healthy(&self, now: u64) -> bool {
        self.fresh(now).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::parse;
    use crate::device::tests::{BATTERY, LOAD, MAINS, capacity};

    struct Run {
        watch: Watch,
        now: u64,
    }

    impl Run {
        fn new() -> Self {
            Run {
                watch: Watch::new(50),
                now: 1000,
            }
        }

        fn tick(&mut self, status: &[u8], charge: u8) -> Vec<&'static str> {
            self.now += 5;
            let reading = parse(status, &capacity(charge, 0xFFFF), &LOAD).unwrap();
            self.watch.update(reading, self.now)
        }
    }

    #[test]
    fn a_first_reading_on_mains_sends_nothing() {
        let mut run = Run::new();
        assert!(run.tick(&MAINS, 100).is_empty());
        assert_eq!("on mains", run.watch.status(run.now)["state"]);
    }

    #[test]
    fn an_outage_and_its_end_are_sent_once() {
        let mut run = Run::new();
        run.tick(&MAINS, 100);
        assert_eq!(vec!["on battery"], run.tick(&BATTERY, 99));
        assert!(run.tick(&BATTERY, 98).is_empty());
        assert_eq!(vec!["on mains"], run.tick(&MAINS, 98));
    }

    #[test]
    fn half_the_battery_shuts_down_once_after_two_readings_on_battery() {
        let mut run = Run::new();
        run.tick(&MAINS, 100);
        assert_eq!(vec!["on battery"], run.tick(&BATTERY, 50));
        assert_eq!(vec![SHUTDOWN], run.tick(&BATTERY, 49));
        assert!(run.watch.stopping);
        assert!(run.tick(&BATTERY, 48).is_empty());
        assert_eq!(true, run.watch.status(run.now)["stopping"]);
    }

    #[test]
    fn a_battery_over_the_threshold_keeps_running() {
        let mut run = Run::new();
        run.tick(&MAINS, 100);
        run.tick(&BATTERY, 51);
        assert!(run.tick(&BATTERY, 51).is_empty());
        assert!(!run.watch.stopping);
    }

    #[test]
    fn a_low_charge_on_mains_never_shuts_down() {
        let mut run = Run::new();
        run.tick(&MAINS, 5);
        assert!(run.tick(&MAINS, 5).is_empty());
        assert!(!run.watch.stopping);
    }

    #[test]
    fn a_stale_reading_is_unreachable() {
        let mut run = Run::new();
        assert!(!run.watch.healthy(run.now));
        run.tick(&MAINS, 100);
        assert!(run.watch.healthy(run.now));
        let status = run.watch.status(run.now);
        assert_eq!(100, status["charge"]);
        assert_eq!(50, status["threshold"]);
        let later = run.now + STALE + 1;
        assert_eq!("unreachable", run.watch.status(later)["state"]);
        assert!(!run.watch.healthy(later));
    }

    #[test]
    fn an_error_is_logged_once() {
        let mut watch = Watch::new(50);
        assert!(watch.failed("gone".into()));
        assert!(!watch.failed("gone".into()));
        assert_eq!("gone", watch.status(0)["error"]);
    }
}
