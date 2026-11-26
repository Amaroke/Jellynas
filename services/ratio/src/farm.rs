use crate::clients::{Fail, Indexers, Qbt, Release, Torrent, Tracker};
use crate::state::{GB, Settings, Snapshot, State};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const DAY: f64 = 86400.0;
pub const WEEK: f64 = 7.0 * DAY;
pub const FARM: &str = "farm";
pub const CROSS_SEED: &str = "/data/torrents/cross-seed";
pub const CATEGORIES: [(&str, &str); 5] = [
    ("movies", "/data/torrents/movies"),
    ("shows", "/data/torrents/shows"),
    ("music", "/data/torrents/music"),
    ("books", "/data/torrents/books"),
    (FARM, "/data/torrents/farm"),
];
pub const LIBRARY: [&str; 4] = ["movies", "shows", "music", "books"];
const SHARE_LIMITS: [&str; 3] = [
    "max_ratio_enabled",
    "max_seeding_time_enabled",
    "max_inactive_seeding_time_enabled",
];
const SEEDING: [&str; 4] = ["uploading", "stalledUP", "queuedUP", "forcedUP"];
pub const ON_C411: &str = "c411";
pub const NYAA: &str = "nyaa";
pub const OTHER: &str = "other";
pub const UNKNOWN: &str = "unknown";
const PAUSED_PROFILE: &str = "Paused";
const ADOPT_AFTER: i64 = 6 * 3600;
const PUBLIC_SEEDING: i64 = 7 * 86400;
const PUBLIC_SETTLE: i64 = 3600;
pub const C411_CALLS: u32 = 30;

pub type Torrents = BTreeMap<String, Torrent>;

pub fn origin_of(t: &Torrent) -> Option<&'static str> {
    if t.save_path.trim_end_matches('/').starts_with(CROSS_SEED) {
        return Some("cross-seed");
    }
    match t.category.as_str() {
        "movies" => Some("radarr"),
        "shows" => Some("sonarr"),
        "books" => Some("lazylibrarian"),
        FARM => Some(FARM),
        _ => None,
    }
}

pub fn tracker_of(state: &State, settings: &Settings, t: &Torrent) -> String {
    let known = state
        .origins
        .get(&t.hash)
        .map(|(_, tracker)| tracker.as_str());
    if known == Some(ON_C411) || t.tracker.to_lowercase().contains(&settings.tracker) {
        return ON_C411.into();
    }
    known.unwrap_or(OTHER).into()
}

pub fn is_c411(state: &State, settings: &Settings, t: &Torrent) -> bool {
    tracker_of(state, settings, t) == ON_C411
}

fn is_public(state: &State, settings: &Settings, t: &Torrent) -> bool {
    !t.tracker.is_empty() && t.private != Some(true) && !is_c411(state, settings, t)
}

fn is_owned(t: &Torrent) -> bool {
    CATEGORIES.iter().any(|(name, _)| *name == t.category)
}

pub fn is_seeding(t: &Torrent) -> bool {
    t.progress >= 1.0 && SEEDING.contains(&t.state.as_str())
}

pub fn size_of(torrents: &Torrents, category: &str) -> f64 {
    torrents
        .values()
        .filter(|t| t.category == category)
        .map(|t| t.size as f64)
        .sum()
}

pub fn prepare(qbt: &dyn Qbt, settings: &Settings) -> Result<(), String> {
    let categories = qbt.categories()?;
    for (name, path) in CATEGORIES {
        if !categories.iter().any(|found| found == name) {
            qbt.create_category(name, path)?;
        }
    }
    let preferences = qbt.preferences()?;
    let on = |key: &str| preferences[key].as_bool().unwrap_or(false);
    let limits: serde_json::Map<String, Value> = SHARE_LIMITS
        .iter()
        .filter(|key| on(key))
        .map(|key| (key.to_string(), json!(false)))
        .collect();
    if !limits.is_empty() {
        qbt.set_preferences(&Value::Object(limits))?;
        println!("qBittorrent no longer stops torrents at its global share limits");
    }
    if on("queueing_enabled") {
        qbt.set_preferences(&json!({"queueing_enabled": false}))?;
        println!("qBittorrent no longer queues torrents, so every torrent announces and seeds");
    }
    if settings.cross_seed_key.is_empty() {
        return Ok(());
    }
    let hook = format!(
        "curl -XPOST http://cross-seed:2468/api/webhook -H \"X-Api-Key: {}\" -d \"infoHash=%I\" -d \"includeSingleEpisodes=true\"",
        settings.cross_seed_key
    );
    if !on("autorun_enabled") || preferences["autorun_program"].as_str() != Some(hook.as_str()) {
        qbt.set_preferences(&json!({"autorun_enabled": true, "autorun_program": hook}))?;
        println!("qBittorrent now tells cross-seed about every finished torrent");
    }
    Ok(())
}

pub fn use_prowlarr(
    state: &mut State,
    step: impl FnOnce(&mut State) -> Result<(), Fail>,
) -> Result<(), Fail> {
    match step(state) {
        Err(Fail::Throttled { wait, disabled }) => {
            let why = if disabled {
                "has disabled the indexer after failures"
            } else {
                "asks to slow down"
            };
            println!(
                "Prowlarr {why}, ratio leaves it alone for {} minutes",
                wait / 60.0
            );
            Ok(())
        }
        other => other,
    }
}

pub fn weigh_floor(
    state: &mut State,
    account: Option<&crate::clients::Account>,
    settings: &Settings,
) {
    let paused = account.is_none_or(|a| (a.uploaded as f64) < settings.floor * a.downloaded as f64);
    if paused && !state.paused {
        println!(
            "Paused: the Account ratio is unknown or under {}, ratio spends no download on C411",
            settings.floor
        );
    }
    if state.paused && !paused {
        println!("Resumed: the Account ratio is back over {}", settings.floor);
    }
    state.paused = paused;
}

pub fn hold_c411(
    prowlarr: &dyn Indexers,
    settings: &Settings,
    state: &mut State,
    wanted: bool,
) -> Result<(), Fail> {
    if state.held == Some(wanted) || settings.hold_indexers.is_empty() {
        return Ok(());
    }
    for indexer in &settings.hold_indexers {
        let restore = state.restore.get(indexer).copied();
        match prowlarr.hold(indexer, wanted, PAUSED_PROFILE, restore) {
            Ok(Some(before)) if wanted => {
                state.restore.insert(indexer.clone(), before);
            }
            Ok(_) => {}
            Err(throttled @ Fail::Throttled { .. }) => return Err(throttled),
            Err(error) => {
                let verb = if wanted { "hold" } else { "release" };
                println!("Cannot {verb} C411 in Prowlarr: {error}");
                return Ok(());
            }
        }
    }
    state.held = Some(wanted);
    let how = if wanted { "held" } else { "open again" };
    println!("C411 is {how} for Radarr and Sonarr");
    Ok(())
}

fn into_farm(qbt: &dyn Qbt, t: &mut Torrent) -> Result<(), String> {
    let hashes = [t.hash.clone()];
    qbt.keep_in_place(&hashes)?;
    qbt.set_category(&hashes, FARM)?;
    qbt.reset_share_limits(&hashes)?;
    if t.progress >= 1.0 && t.state.starts_with("stopped") {
        qbt.start(&hashes)?;
        t.state = "stalledUP".into();
    }
    t.category = FARM.into();
    Ok(())
}

pub fn adopt(
    qbt: &dyn Qbt,
    settings: &Settings,
    state: &State,
    torrents: &mut Torrents,
) -> Result<(), String> {
    let mut room = settings.farm_size - size_of(torrents, FARM);
    let hashes: Vec<String> = torrents
        .values()
        .filter(|t| {
            LIBRARY.contains(&t.category.as_str())
                && is_c411(state, settings, t)
                && t.progress >= 1.0
                && t.seeding_time >= ADOPT_AFTER
        })
        .map(|t| t.hash.clone())
        .collect();
    for hash in hashes {
        let t = torrents.get_mut(&hash).expect("listed above");
        if t.size as f64 > room {
            continue;
        }
        let category = t.category.clone();
        into_farm(qbt, t)?;
        room -= t.size as f64;
        println!("Moved {} from {category} to the Farm", t.name);
    }
    Ok(())
}

fn shares_its_folder(t: &Torrent, torrents: &Torrents) -> bool {
    torrents
        .values()
        .any(|o| o.hash != t.hash && o.save_path == t.save_path && o.size == t.size)
}

pub fn clean_public(
    qbt: &dyn Qbt,
    settings: &Settings,
    state: &State,
    torrents: &mut Torrents,
) -> Result<(), String> {
    let done: Vec<(String, &'static str)> = torrents
        .values()
        .filter(|t| is_owned(t) && is_public(state, settings, t) && t.progress >= 1.0)
        .filter_map(|t| {
            if t.uploaded >= t.size && t.size > 0 && t.seeding_time >= PUBLIC_SETTLE {
                Some((t.hash.clone(), "reached ratio 1"))
            } else if t.seeding_time >= PUBLIC_SEEDING {
                Some((t.hash.clone(), "seeded a week"))
            } else {
                None
            }
        })
        .collect();
    for (hash, why) in done {
        let keep = shares_its_folder(&torrents[&hash], torrents);
        qbt.delete(std::slice::from_ref(&hash), !keep)?;
        let gone = torrents.remove(&hash).expect("listed above");
        println!("Removed {}, a Public torrent that {why}", gone.name);
    }
    Ok(())
}

pub fn tag(qbt: &dyn Qbt, torrents: &Torrents) -> Result<(), String> {
    let origins = ["cross-seed", "radarr", "sonarr", "lazylibrarian", FARM];
    let mut wanted: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut stale: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for t in torrents.values() {
        let name = origin_of(t);
        let tags: BTreeSet<&str> = t
            .tags
            .split(',')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .collect();
        if let Some(name) = name
            && !tags.contains(name)
        {
            wanted.entry(name).or_default().push(t.hash.clone());
        }
        for old in tags.into_iter().filter(|old| Some(*old) != name) {
            stale.entry(old.into()).or_default().push(t.hash.clone());
        }
    }
    for (name, hashes) in wanted {
        qbt.add_tags(&hashes, name)?;
    }
    for (name, hashes) in stale {
        qbt.remove_tags(&hashes, &name)?;
    }
    let unused: Vec<String> = qbt
        .tags()?
        .into_iter()
        .filter(|name| !origins.contains(&name.as_str()))
        .collect();
    if !unused.is_empty() {
        qbt.delete_tags(&unused)?;
    }
    Ok(())
}

pub struct Budget {
    pub farm: f64,
}

fn take(
    qbt: &dyn Qbt,
    budget: &mut Budget,
    known: &mut BTreeSet<String>,
    release: &Release,
    torrent: &[u8],
    via: &str,
) -> Result<(), String> {
    qbt.add(torrent, FARM, None)?;
    known.insert(release.infohash.clone());
    budget.farm -= release.size as f64;
    println!(
        "Grabbed {} for the Farm from {via}, {} seeders, {} leechers, {:.2} GB",
        release.title,
        release.seeders,
        release.leechers,
        release.size as f64 / GB
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn grab_free(
    qbt: &dyn Qbt,
    c411: &dyn Tracker,
    settings: &Settings,
    state: &mut State,
    releases: &[Release],
    known: &mut BTreeSet<String>,
    budget: &mut Budget,
    now: f64,
) -> Result<(), Fail> {
    let mut free: Vec<&Release> = releases
        .iter()
        .filter(|r| {
            r.free
                && !known.contains(&r.infohash)
                && r.seeders >= 1
                && r.size as f64 <= settings.free_max_size
        })
        .collect();
    free.sort_by_key(|r| -r.leechers);
    for release in free {
        if known.contains(&release.infohash) || release.size as f64 > budget.farm {
            continue;
        }
        let grabbed = c411.download(&release.infohash).and_then(|torrent| {
            take(qbt, budget, known, release, &torrent, "C411, freeleech").map_err(Fail::Other)
        });
        match grabbed {
            Ok(()) => {
                state.free.insert(release.infohash.clone(), now);
            }
            Err(Fail::OutOfCalls) => return Err(Fail::OutOfCalls),
            Err(error) => {
                state.given_up.insert(release.infohash.clone(), now);
                known.insert(release.infohash.clone());
                println!(
                    "Cannot grab {}, ratio leaves it alone for a week: {error}",
                    release.title
                );
            }
        }
    }
    Ok(())
}

pub fn grab(
    qbt: &dyn Qbt,
    c411: &dyn Tracker,
    settings: &Settings,
    switches: &Snapshot,
    state: &mut State,
    torrents: &Torrents,
    now: f64,
) -> Result<(), Fail> {
    let mut budget = Budget {
        farm: settings.farm_size - size_of(torrents, FARM),
    };
    if budget.farm <= 0.0 || !switches.on("free") {
        return Ok(());
    }
    let mut known: BTreeSet<String> = torrents.keys().cloned().collect();
    known.extend(state.given_up.keys().cloned());
    let releases = c411.catalog(now)?;
    grab_free(
        qbt,
        c411,
        settings,
        state,
        &releases,
        &mut known,
        &mut budget,
        now,
    )
}

#[cfg(test)]
pub mod fakes {
    use super::*;
    use crate::clients::Account;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    pub struct FakeQbt {
        pub torrents: RefCell<Vec<Torrent>>,
        pub calls: RefCell<Vec<String>>,
        pub preferences: RefCell<Value>,
        pub tags: RefCell<Vec<String>>,
        pub refuse_add: bool,
    }

    impl FakeQbt {
        fn note(&self, call: String) -> Result<(), String> {
            self.calls.borrow_mut().push(call);
            Ok(())
        }

        pub fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl Qbt for FakeQbt {
        fn torrents(&self) -> Result<Vec<Torrent>, String> {
            Ok(self.torrents.borrow().clone())
        }
        fn add(
            &self,
            torrent: &[u8],
            category: &str,
            save_path: Option<&str>,
        ) -> Result<(), String> {
            if self.refuse_add {
                return Err("qBittorrent refused the torrent file".into());
            }
            self.note(format!(
                "add {} {category} {}",
                String::from_utf8_lossy(torrent),
                save_path.unwrap_or("-")
            ))
        }
        fn delete(&self, hashes: &[String], delete_files: bool) -> Result<(), String> {
            self.note(format!("delete {} {delete_files}", hashes.join("|")))
        }
        fn start(&self, hashes: &[String]) -> Result<(), String> {
            self.note(format!("start {}", hashes.join("|")))
        }
        fn set_category(&self, hashes: &[String], category: &str) -> Result<(), String> {
            self.note(format!("category {} {category}", hashes.join("|")))
        }
        fn categories(&self) -> Result<Vec<String>, String> {
            Ok(vec!["movies".into()])
        }
        fn create_category(&self, name: &str, _path: &str) -> Result<(), String> {
            self.note(format!("create {name}"))
        }
        fn add_tags(&self, hashes: &[String], tag: &str) -> Result<(), String> {
            self.note(format!("tag {} {tag}", hashes.join("|")))
        }
        fn remove_tags(&self, hashes: &[String], tag: &str) -> Result<(), String> {
            self.note(format!("untag {} {tag}", hashes.join("|")))
        }
        fn tags(&self) -> Result<Vec<String>, String> {
            Ok(self.tags.borrow().clone())
        }
        fn delete_tags(&self, names: &[String]) -> Result<(), String> {
            self.note(format!("deletetags {}", names.join(",")))
        }
        fn trackers(&self, _hash: &str) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
        fn keep_in_place(&self, hashes: &[String]) -> Result<(), String> {
            self.note(format!("inplace {}", hashes.join("|")))
        }
        fn reset_share_limits(&self, hashes: &[String]) -> Result<(), String> {
            self.note(format!("limits {}", hashes.join("|")))
        }
        fn preferences(&self) -> Result<Value, String> {
            Ok(self.preferences.borrow().clone())
        }
        fn set_preferences(&self, preferences: &Value) -> Result<(), String> {
            self.note(format!("prefs {preferences}"))
        }
    }

    #[derive(Default)]
    pub struct FakeProwlarr {
        pub throttle: bool,
        pub holds: RefCell<Vec<(String, bool, Option<i64>)>>,
    }

    impl Indexers for FakeProwlarr {
        fn hold(
            &self,
            indexer: &str,
            paused: bool,
            _profile: &str,
            restore: Option<i64>,
        ) -> Result<Option<i64>, Fail> {
            if self.throttle {
                return Err(Fail::Throttled {
                    wait: 120.0,
                    disabled: false,
                });
            }
            self.holds
                .borrow_mut()
                .push((indexer.into(), paused, restore));
            Ok(Some(1))
        }
    }

    #[derive(Default)]
    pub struct FakeC411 {
        pub account: Option<Account>,
        pub catalog: Vec<Release>,
        pub torrents: HashMap<String, Vec<u8>>,
        pub refuse: bool,
        pub downloads: RefCell<Vec<String>>,
    }

    impl Tracker for FakeC411 {
        fn allow(&self, _calls: u32) {}
        fn account(&self) -> Result<Account, Fail> {
            self.account.clone().ok_or(Fail::Other("down".into()))
        }
        fn catalog(&self, _now: f64) -> Result<Vec<Release>, Fail> {
            Ok(self.catalog.clone())
        }
        fn download(&self, infohash: &str) -> Result<Vec<u8>, Fail> {
            self.downloads.borrow_mut().push(infohash.into());
            if self.refuse {
                return Err(Fail::Refused("C411 answered 404".into()));
            }
            self.torrents
                .get(infohash)
                .cloned()
                .ok_or(Fail::Other("C411 is down".into()))
        }
    }

    pub fn settings() -> Settings {
        Settings {
            tracker: "c411".into(),
            hold_indexers: vec!["3".into()],
            cross_seed_key: "key".into(),
            farm_size: 100.0 * GB,
            floor: 2.0,
            free_max_size: 5.0 * GB,
        }
    }

    pub fn torrent(hash: &str, category: &str) -> Torrent {
        Torrent {
            hash: hash.into(),
            name: format!("name-{hash}"),
            category: category.into(),
            progress: 1.0,
            state: "stalledUP".into(),
            size: GB as i64,
            save_path: format!("/data/torrents/{category}"),
            tracker: "https://tracker.example/announce".into(),
            ..Torrent::default()
        }
    }

    pub fn release(hash: &str, title: &str) -> Release {
        Release {
            infohash: hash.into(),
            title: title.into(),
            size: 1000,
            seeders: 3,
            leechers: 5,
            age_hours: 0.5,
            ..Release::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::*;
    use super::*;
    use std::collections::HashMap;

    fn by_hash(list: Vec<Torrent>) -> Torrents {
        list.into_iter().map(|t| (t.hash.clone(), t)).collect()
    }

    fn snapshot(on: &[&str]) -> Snapshot {
        Snapshot {
            switches: on.iter().map(|name| (name.to_string(), true)).collect(),
            numbers: BTreeMap::new(),
        }
    }

    #[test]
    fn a_throttled_prowlarr_is_left_alone() {
        let prowlarr = FakeProwlarr {
            throttle: true,
            ..FakeProwlarr::default()
        };
        let mut state = State {
            paused: true,
            ..State::default()
        };
        use_prowlarr(&mut state, |state| {
            hold_c411(&prowlarr, &settings(), state, true)
        })
        .unwrap();
        assert!(prowlarr.holds.borrow().is_empty());
    }

    #[test]
    fn freeleech_is_grabbed_up_to_its_size_and_counts_no_download() {
        let qbt = FakeQbt::default();
        let mut big = release("big", "Big-Grp");
        big.free = true;
        big.size = (6.0 * GB) as i64;
        let mut small = release("small", "Small-Grp");
        small.free = true;
        let c411 = FakeC411 {
            catalog: vec![big, small],
            torrents: HashMap::from([("small".to_string(), b"t".to_vec())]),
            ..FakeC411::default()
        };
        let mut state = State::default();
        grab(
            &qbt,
            &c411,
            &settings(),
            &snapshot(&["free"]),
            &mut state,
            &Torrents::new(),
            9.0,
        )
        .unwrap();
        assert_eq!(vec!["add t farm -"], qbt.calls());
        assert_eq!(Some(&9.0), state.free.get("small"));
    }

    #[test]
    fn nothing_is_grabbed_once_the_farm_is_full() {
        let qbt = FakeQbt::default();
        let mut full = torrent("f", FARM);
        full.size = (100.0 * GB) as i64;
        let mut small = release("small", "Small-Grp");
        small.free = true;
        let c411 = FakeC411 {
            catalog: vec![small],
            ..FakeC411::default()
        };
        grab(
            &qbt,
            &c411,
            &settings(),
            &snapshot(&["free"]),
            &mut State::default(),
            &by_hash(vec![full]),
            0.0,
        )
        .unwrap();
        assert!(qbt.calls().is_empty());
    }

    #[test]
    fn a_public_torrent_leaves_after_a_week_or_at_ratio_one() {
        let qbt = FakeQbt::default();
        let mut week = torrent("week", FARM);
        week.seeding_time = 7 * 86400;
        let mut shared = torrent("shared", FARM);
        shared.uploaded = shared.size;
        shared.seeding_time = 3600;
        shared.save_path = "/data/torrents/farm/x".into();
        let mut copy = torrent("copy", FARM);
        copy.tracker = "https://c411.org/announce".into();
        copy.save_path = shared.save_path.clone();
        let mut young = torrent("young", FARM);
        young.save_path = "/data/torrents/farm/young".into();
        let mut torrents = by_hash(vec![week, shared, copy, young]);
        clean_public(&qbt, &settings(), &State::default(), &mut torrents).unwrap();
        assert_eq!(vec!["delete shared false", "delete week true"], qbt.calls());
        assert!(torrents.contains_key("young"));
        assert!(torrents.contains_key("copy"));
    }

    #[test]
    fn a_public_torrent_at_ratio_one_waits_for_its_import() {
        let qbt = FakeQbt::default();
        let mut fresh = torrent("fresh", "shows");
        fresh.uploaded = fresh.size * 2;
        fresh.seeding_time = 60;
        fresh.save_path = "/data/torrents/shows/fresh".into();
        let state = State::default();
        let mut torrents = by_hash(vec![fresh]);
        clean_public(&qbt, &settings(), &state, &mut torrents).unwrap();
        assert!(qbt.calls().is_empty());
    }

    #[test]
    fn a_private_or_c411_torrent_never_leaves() {
        let qbt = FakeQbt::default();
        let mut private = torrent("p", FARM);
        private.private = Some(true);
        private.seeding_time = 30 * 86400;
        let mut c411 = torrent("c", "movies");
        c411.tracker = "https://c411.org/announce".into();
        c411.uploaded = c411.size * 3;
        let mut torrents = by_hash(vec![private, c411]);
        clean_public(&qbt, &settings(), &State::default(), &mut torrents).unwrap();
        assert!(qbt.calls().is_empty());
    }

    #[test]
    fn the_floor_pauses_and_the_hold_moves_c411_in_prowlarr() {
        let mut state = State::default();
        let account = crate::clients::Account {
            uploaded: 150,
            downloaded: 100,
            ..Default::default()
        };
        weigh_floor(&mut state, Some(&account), &settings());
        assert!(state.paused);
        let prowlarr = FakeProwlarr::default();
        hold_c411(&prowlarr, &settings(), &mut state, true).unwrap();
        assert_eq!(
            vec![("3".to_string(), true, None)],
            *prowlarr.holds.borrow()
        );
        assert_eq!(Some(true), state.held);
        assert_eq!(1, state.restore["3"]);
        hold_c411(&prowlarr, &settings(), &mut state, true).unwrap();
        assert_eq!(1, prowlarr.holds.borrow().len());
        let account = crate::clients::Account {
            uploaded: 250,
            downloaded: 100,
            ..Default::default()
        };
        weigh_floor(&mut state, Some(&account), &settings());
        assert!(!state.paused);
    }

    #[test]
    fn a_c411_torrent_of_the_library_moves_into_the_farm_after_six_hours() {
        let qbt = FakeQbt::default();
        let mut ready = torrent("r", "movies");
        ready.tracker = "https://c411.org/announce".into();
        ready.seeding_time = ADOPT_AFTER;
        let mut early = ready.clone();
        early.hash = "e".into();
        early.seeding_time = 60;
        let mut torrents = by_hash(vec![ready, early]);
        adopt(&qbt, &settings(), &State::default(), &mut torrents).unwrap();
        assert_eq!(
            vec!["inplace r", "category r farm", "limits r"],
            qbt.calls()
        );
        assert_eq!(FARM, torrents["r"].category);
    }

    #[test]
    fn each_torrent_carries_only_its_origin_tag() {
        let qbt = FakeQbt::default();
        *qbt.tags.borrow_mut() = vec!["radarr".into(), "old".into()];
        let mut movie = torrent("m", "movies");
        movie.tags = "old".into();
        let mut cross = torrent("x", FARM);
        cross.save_path = "/data/torrents/cross-seed/C411".into();
        cross.tags = "cross-seed".into();
        tag(&qbt, &by_hash(vec![movie, cross])).unwrap();
        assert_eq!(
            vec!["tag m radarr", "untag m old", "deletetags old"],
            qbt.calls()
        );
    }

    #[test]
    fn prepare_turns_off_share_limits_and_queueing_and_sets_the_hook() {
        let qbt = FakeQbt::default();
        *qbt.preferences.borrow_mut() =
            json!({"max_ratio_enabled": true, "queueing_enabled": true});
        prepare(&qbt, &settings()).unwrap();
        let calls = qbt.calls();
        assert!(calls.contains(&"create farm".to_string()));
        assert!(!calls.contains(&"create movies".to_string()));
        assert!(calls.contains(&"prefs {\"max_ratio_enabled\":false}".to_string()));
        assert!(calls.contains(&"prefs {\"queueing_enabled\":false}".to_string()));
        assert!(calls.iter().any(|call| call.contains("cross-seed:2468")));
    }
}
