use crate::clients::{Fail, Indexers, Qbt, Release, Torrent, Tracker, file_lengths, match_files};
use crate::state::{GB, Settings, Snapshot, State};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

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
const TWIN_LOOKUPS: u32 = 30;
const TWIN_MAX_AGE_HOURS: f64 = 8.0;
const TWIN_MIN_LEECHERS: i64 = 1;
const TWIN_SIZE_SLACK: f64 = 0.05;
const TWIN_ATTEMPTS: u32 = 3;
const TWIN_RETRY: f64 = 600.0;
const TWIN_LANGUAGES: [&str; 1] = ["VOSTFR"];
const SEARCHES_PER_MINUTE: usize = 10;
const SEARCHES_PER_HOUR: usize = 200;

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
    now: f64,
    always: bool,
    step: impl FnOnce(&mut State) -> Result<(), Fail>,
) -> Result<(), Fail> {
    if now < state.backoff_until && !always {
        return Ok(());
    }
    match step(state) {
        Err(Fail::Throttled { wait, disabled }) => {
            state.backoff_until = now + wait;
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

pub fn mind_twins(qbt: &dyn Qbt, state: &mut State, torrents: &Torrents) -> Result<(), String> {
    for (twin, (original, ..)) in state.twins.clone() {
        if !torrents.contains_key(&twin) {
            state.twins.remove(&twin);
            continue;
        }
        if let Some(copy) = torrents.get(&original)
            && copy.progress >= 1.0
            && copy.state.starts_with("stopped")
        {
            qbt.start(std::slice::from_ref(&original))?;
            println!("{} now seeds on C411", copy.name);
        }
    }
    Ok(())
}

fn shares_its_folder(t: &Torrent, torrents: &Torrents) -> bool {
    torrents
        .values()
        .any(|o| o.hash != t.hash && o.save_path == t.save_path && o.size == t.size)
}

fn waits_for_its_copy(state: &State, t: &Torrent, torrents: &Torrents) -> bool {
    state
        .twins
        .get(&t.hash)
        .is_some_and(|(original, ..)| !torrents.get(original).is_some_and(is_seeding))
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
        .filter(|t| !waits_for_its_copy(state, t, torrents))
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

fn squeeze(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase()
}

fn group_of(title: &str) -> String {
    title
        .rsplit_once('-')
        .map(|(_, group)| squeeze(group))
        .unwrap_or_default()
}

fn language_of(title: &str) -> Option<&'static str> {
    let upper = title.to_uppercase();
    ["VOSTFR", "MULTI", "FRENCH"]
        .into_iter()
        .find(|language| upper.contains(language))
}

fn two_digits(bytes: &[u8], at: usize) -> bool {
    bytes.get(at).is_some_and(u8::is_ascii_digit)
        && bytes.get(at + 1).is_some_and(u8::is_ascii_digit)
}

pub fn marker_of(title: &str) -> Option<String> {
    let upper = title.to_uppercase();
    let bytes = upper.as_bytes();
    let mut season = None;
    for at in 0..bytes.len() {
        if bytes[at] != b'S' || !two_digits(bytes, at + 1) {
            continue;
        }
        if bytes.get(at + 3) == Some(&b'E') && two_digits(bytes, at + 4) {
            return Some(upper[at..at + 6].into());
        }
        let before = at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
        let after = !bytes.get(at + 3).is_some_and(u8::is_ascii_alphanumeric);
        if season.is_none() && before && after {
            season = Some(upper[at..at + 3].to_string());
        }
    }
    if season.is_some() {
        return season;
    }
    (0..bytes.len().saturating_sub(3))
        .find(|&at| {
            let before = at == 0 || !bytes[at - 1].is_ascii_digit();
            let after = !bytes.get(at + 4).is_some_and(u8::is_ascii_digit);
            before
                && after
                && (bytes[at..at + 2] == *b"19" || bytes[at..at + 2] == *b"20")
                && two_digits(bytes, at + 2)
        })
        .map(|at| upper[at..at + 4].to_string())
}

fn may_be_twin(release: &Release, candidate: &Release) -> bool {
    let group = group_of(&release.title);
    !group.is_empty()
        && squeeze(&candidate.title).contains(&group)
        && marker_of(&candidate.title) == marker_of(&release.title)
        && (candidate.size - release.size).abs() as f64 <= TWIN_SIZE_SLACK * release.size as f64
        && candidate.seeders >= 1
}

pub fn searches_left(searches: &[f64], now: f64) -> usize {
    let minute = searches.iter().filter(|at| now - **at < 60.0).count();
    let hour = searches.iter().filter(|at| now - **at < 3600.0).count();
    SEARCHES_PER_MINUTE
        .saturating_sub(minute)
        .min(SEARCHES_PER_HOUR.saturating_sub(hour))
}

fn twin_worth(release: &Release) -> bool {
    language_of(&release.title).is_some_and(|language| TWIN_LANGUAGES.contains(&language))
}

pub fn twin_due(release: &Release, checked: Option<f64>, now: f64) -> bool {
    if release.age_hours > TWIN_MAX_AGE_HOURS || release.leechers < TWIN_MIN_LEECHERS {
        return false;
    }
    let every = if release.age_hours < 1.0 {
        600.0
    } else {
        1800.0
    };
    checked.is_none_or(|at| now - at >= every)
}

#[derive(Default)]
pub struct Cache {
    searches: HashMap<String, Vec<Release>>,
    files: HashMap<String, Vec<u8>>,
}

pub struct Twin {
    pub release: Release,
    pub torrent: Vec<u8>,
}

fn find_twin(
    prowlarr: &dyn Indexers,
    c411: &dyn Tracker,
    settings: &Settings,
    release: &Release,
    now: f64,
    cache: &mut Cache,
    searches: &mut Vec<f64>,
) -> Result<Option<Twin>, Fail> {
    let (Some(marker), Some(language)) = (marker_of(&release.title), language_of(&release.title))
    else {
        return Ok(None);
    };
    for indexer in &settings.twin_indexers {
        let query = format!("{marker} {language}");
        let key = format!("{indexer} {query}");
        if !cache.searches.contains_key(&key) {
            searches.push(now);
            let found = prowlarr.search(indexer, &query, now)?;
            cache.searches.insert(key.clone(), found);
        }
        let candidates: Vec<Release> = cache.searches[&key]
            .iter()
            .filter(|candidate| may_be_twin(release, candidate))
            .cloned()
            .collect();
        for candidate in candidates {
            let public_key = format!("public {}", candidate.infohash);
            if !cache.files.contains_key(&public_key) {
                let torrent = prowlarr.torrent(&candidate)?;
                cache.files.insert(public_key.clone(), torrent);
            }
            let private_key = format!("c411 {}", release.infohash);
            if !cache.files.contains_key(&private_key) {
                let torrent = c411.download(&release.infohash)?;
                cache.files.insert(private_key.clone(), torrent);
            }
            if file_lengths(&cache.files[&public_key])? == file_lengths(&cache.files[&private_key])?
            {
                return Ok(Some(Twin {
                    torrent: cache.files[&public_key].clone(),
                    release: candidate,
                }));
            }
        }
    }
    Ok(None)
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
pub fn grab_twins(
    qbt: &dyn Qbt,
    prowlarr: &dyn Indexers,
    c411: &dyn Tracker,
    settings: &Settings,
    state: &mut State,
    releases: &[Release],
    known: &mut BTreeSet<String>,
    budget: &mut Budget,
    now: f64,
) -> Result<(), Fail> {
    let mut cache = Cache::default();
    let mut lookups = TWIN_LOOKUPS;
    let mut wanted: Vec<&Release> = releases
        .iter()
        .filter(|r| {
            !known.contains(&r.infohash)
                && marker_of(&r.title).is_some()
                && twin_worth(r)
                && twin_due(r, state.checked.get(&r.infohash).copied(), now)
        })
        .collect();
    wanted.sort_by_key(|r| -r.leechers);
    state.searches.retain(|at| now - at < 3600.0);
    for release in wanted {
        if lookups == 0 || searches_left(&state.searches, now) < settings.twin_indexers.len() {
            return Ok(());
        }
        if release.size as f64 > budget.farm {
            continue;
        }
        lookups -= 1;
        state.checked.insert(release.infohash.clone(), now);
        match find_twin(
            prowlarr,
            c411,
            settings,
            release,
            now,
            &mut cache,
            &mut state.searches,
        ) {
            Ok(Some(twin)) => {
                let via = format!("its public twin {}", twin.release.title);
                match take(qbt, budget, known, release, &twin.torrent, &via) {
                    Ok(()) => {
                        state.twins.insert(
                            twin.release.infohash.clone(),
                            (release.infohash.clone(), FARM.into(), now, String::new()),
                        );
                    }
                    Err(error) => println!("Cannot grab the twin of {}: {error}", release.title),
                }
            }
            Ok(None) => {}
            Err(stop @ (Fail::Throttled { .. } | Fail::OutOfCalls)) => return Err(stop),
            Err(error) => println!("Cannot look for a twin of {}: {error}", release.title),
        }
    }
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

#[allow(clippy::too_many_arguments)]
pub fn grab(
    qbt: &dyn Qbt,
    prowlarr: &dyn Indexers,
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
    if budget.farm <= 0.0 || !(switches.on("twins") || switches.on("free")) {
        return Ok(());
    }
    let mut known: BTreeSet<String> = torrents.keys().cloned().collect();
    known.extend(state.twins.values().map(|(original, ..)| original.clone()));
    known.extend(state.given_up.keys().cloned());
    let releases = c411.catalog(now)?;
    if switches.on("twins") && !settings.twin_indexers.is_empty() {
        use_prowlarr(state, now, false, |state| {
            grab_twins(
                qbt,
                prowlarr,
                c411,
                settings,
                state,
                &releases,
                &mut known,
                &mut budget,
                now,
            )
        })?;
    }
    if switches.on("free") {
        grab_free(
            qbt,
            c411,
            settings,
            state,
            &releases,
            &mut known,
            &mut budget,
            now,
        )?;
    }
    Ok(())
}

pub fn seed_twins(
    qbt: &dyn Qbt,
    c411: &dyn Tracker,
    state: &mut State,
    torrents: &Torrents,
    now: f64,
) -> Result<(), Fail> {
    state.stuck.retain(|twin, _| state.twins.contains_key(twin));
    for (twin, (original, ..)) in state.twins.clone() {
        let Some(public) = torrents.get(&twin) else {
            continue;
        };
        if torrents.contains_key(&original) || public.progress < 1.0 {
            continue;
        }
        let (attempts, next_at) = state.stuck.get(&twin).copied().unwrap_or((0, 0.0));
        if now < next_at {
            continue;
        }
        let mut added = false;
        let tried = c411.download(&original).and_then(|torrent| {
            qbt.add(&torrent, FARM, Some(&public.save_path))?;
            added = true;
            match_files(qbt, &original, &qbt.files(&twin)?)?;
            Ok(())
        });
        match tried {
            Ok(()) => {
                state.stuck.remove(&twin);
                println!("The C411 copy of {} now checks the same files", public.name);
            }
            Err(Fail::OutOfCalls) => return Err(Fail::OutOfCalls),
            Err(error) => {
                let refused = matches!(error, Fail::Refused(_));
                if refused || attempts + 1 >= TWIN_ATTEMPTS {
                    if added {
                        qbt.delete(std::slice::from_ref(&original), false)?;
                    }
                    state.twins.remove(&twin);
                    state.stuck.remove(&twin);
                    state.given_up.insert(original.clone(), now);
                    println!(
                        "Gave up the C411 copy of {}, its public copy seeds alone until it leaves: {error}",
                        public.name
                    );
                    continue;
                }
                let wait = TWIN_RETRY * 2f64.powi(attempts as i32);
                state.stuck.insert(twin.clone(), (attempts + 1, now + wait));
                println!(
                    "Cannot seed the C411 copy of {}, next try in {} minutes: {error}",
                    public.name,
                    wait / 60.0
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod fakes {
    use super::*;
    use crate::clients::{Account, TorrentFile};
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct FakeQbt {
        pub torrents: RefCell<Vec<Torrent>>,
        pub calls: RefCell<Vec<String>>,
        pub files: RefCell<HashMap<String, Vec<TorrentFile>>>,
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
        fn files(&self, hash: &str) -> Result<Vec<TorrentFile>, String> {
            Ok(self.files.borrow().get(hash).cloned().unwrap_or_default())
        }
        fn rename_file(&self, hash: &str, old: &str, new: &str) -> Result<(), String> {
            self.note(format!("rename {hash} {old} {new}"))
        }
        fn recheck(&self, hash: &str) -> Result<(), String> {
            self.note(format!("recheck {hash}"))
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
        pub results: Vec<Release>,
        pub torrents: HashMap<String, Vec<u8>>,
        pub searches: RefCell<Vec<String>>,
        pub throttle: bool,
        pub holds: RefCell<Vec<(String, bool, Option<i64>)>>,
    }

    impl Indexers for FakeProwlarr {
        fn search(&self, indexer: &str, query: &str, _now: f64) -> Result<Vec<Release>, Fail> {
            if self.throttle {
                return Err(Fail::Throttled {
                    wait: 120.0,
                    disabled: false,
                });
            }
            self.searches
                .borrow_mut()
                .push(format!("{indexer} {query}"));
            Ok(self.results.clone())
        }
        fn torrent(&self, release: &Release) -> Result<Vec<u8>, Fail> {
            self.torrents
                .get(&release.infohash)
                .cloned()
                .ok_or(Fail::Other("no torrent".into()))
        }
        fn hold(
            &self,
            indexer: &str,
            paused: bool,
            _profile: &str,
            restore: Option<i64>,
        ) -> Result<Option<i64>, Fail> {
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
            twin_indexers: vec!["1".into()],
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

    fn by_hash(list: Vec<Torrent>) -> Torrents {
        list.into_iter().map(|t| (t.hash.clone(), t)).collect()
    }

    fn snapshot(on: &[&str]) -> Snapshot {
        Snapshot {
            switches: on.iter().map(|name| (name.to_string(), true)).collect(),
            numbers: BTreeMap::new(),
        }
    }

    const ONE_FILE: &[u8] = b"d4:infod6:lengthi1000e4:name1:xee";
    const OTHER_FILE: &[u8] = b"d4:infod6:lengthi999e4:name1:xee";

    #[test]
    fn the_marker_is_the_episode_then_the_season_then_the_year() {
        assert_eq!(
            Some("S01E02".into()),
            marker_of("Show.s01e02.VOSTFR.1080p-Grp")
        );
        assert_eq!(Some("S02".into()), marker_of("Show S02 MULTi 1080p-Grp"));
        assert_eq!(
            Some("2023".into()),
            marker_of("Movie.2023.FRENCH.1080p-Grp")
        );
        assert_eq!(None, marker_of("Show.MULTi.1080p-Grp"));
        assert_eq!(None, marker_of("Movie.12023.1080p-Grp"));
    }

    #[test]
    fn a_release_is_searched_every_ten_minutes_then_every_half_hour_until_eight_hours() {
        let mut young = release("a", "x");
        young.age_hours = 0.5;
        assert!(twin_due(&young, None, 1000.0));
        assert!(!twin_due(&young, Some(500.0), 1000.0));
        assert!(twin_due(&young, Some(400.0), 1000.0));
        let mut older = release("a", "x");
        older.age_hours = 3.0;
        assert!(!twin_due(&older, Some(0.0), 1000.0));
        assert!(twin_due(&older, Some(0.0), 1800.0));
        older.age_hours = 8.5;
        assert!(!twin_due(&older, None, 1000.0));
        older.age_hours = 2.0;
        older.leechers = 0;
        assert!(!twin_due(&older, None, 1000.0));
    }

    #[test]
    fn a_twin_with_the_same_files_is_grabbed_from_the_public_tracker() {
        let qbt = FakeQbt::default();
        let mut public = release("pub", "[Grp] Show S01E02 VOSTFR 1080p-Grp");
        public.size = 1010;
        let prowlarr = FakeProwlarr {
            results: vec![public],
            torrents: HashMap::from([("pub".to_string(), ONE_FILE.to_vec())]),
            ..FakeProwlarr::default()
        };
        let c411 = FakeC411 {
            torrents: HashMap::from([("c".to_string(), ONE_FILE.to_vec())]),
            ..FakeC411::default()
        };
        let mut state = State::default();
        let releases = vec![release("c", "Show.S01E02.VOSTFR.1080p-Grp")];
        let mut known = BTreeSet::new();
        let mut budget = Budget { farm: 1e12 };
        grab_twins(
            &qbt,
            &prowlarr,
            &c411,
            &settings(),
            &mut state,
            &releases,
            &mut known,
            &mut budget,
            100.0,
        )
        .unwrap();
        assert_eq!(vec!["1 S01E02 VOSTFR"], *prowlarr.searches.borrow());
        assert_eq!(1, qbt.calls().len());
        assert!(qbt.calls()[0].starts_with("add d4:info"));
        assert_eq!("c", state.twins["pub"].0);
        assert_eq!(Some(&100.0), state.checked.get("c"));
    }

    #[test]
    fn a_vostfr_movie_or_season_pack_is_searched_by_its_year_or_season() {
        let qbt = FakeQbt::default();
        let prowlarr = FakeProwlarr::default();
        let c411 = FakeC411::default();
        let mut state = State::default();
        let releases = vec![
            release("m", "Movie.2024.VOSTFR.1080p-Grp"),
            release("s", "Show.S03.VOSTFR.1080p-Grp"),
            release("x", "Other.S01E02.MULTi.1080p-Grp"),
        ];
        grab_twins(
            &qbt,
            &prowlarr,
            &c411,
            &settings(),
            &mut state,
            &releases,
            &mut BTreeSet::new(),
            &mut Budget { farm: 1e12 },
            0.0,
        )
        .unwrap();
        let mut searched = prowlarr.searches.borrow().clone();
        searched.sort();
        assert_eq!(vec!["1 2024 VOSTFR", "1 S03 VOSTFR"], searched);
        assert_eq!(vec![0.0, 0.0], state.searches);
    }

    #[test]
    fn twin_searches_stop_at_ten_a_minute_and_two_hundred_an_hour() {
        let qbt = FakeQbt::default();
        let prowlarr = FakeProwlarr::default();
        let c411 = FakeC411::default();
        let releases: Vec<Release> = (0..20)
            .map(|n| release(&format!("r{n}"), &format!("Show.S01E{n:02}.VOSTFR-Grp")))
            .collect();
        let mut state = State::default();
        let run = |state: &mut State, now: f64| {
            grab_twins(
                &qbt,
                &prowlarr,
                &c411,
                &settings(),
                state,
                &releases,
                &mut BTreeSet::new(),
                &mut Budget { farm: 1e12 },
                now,
            )
            .unwrap();
        };
        run(&mut state, 0.0);
        assert_eq!(10, prowlarr.searches.borrow().len());
        run(&mut state, 30.0);
        assert_eq!(10, prowlarr.searches.borrow().len());
        state.searches = vec![1000.0; 195];
        assert_eq!(5, searches_left(&state.searches, 1100.0));
        assert_eq!(10, searches_left(&state.searches, 4700.0));
    }

    #[test]
    fn a_twin_whose_files_differ_is_left_alone() {
        let qbt = FakeQbt::default();
        let prowlarr = FakeProwlarr {
            results: vec![release("pub", "Show S01E02 VOSTFR-Grp")],
            torrents: HashMap::from([("pub".to_string(), OTHER_FILE.to_vec())]),
            ..FakeProwlarr::default()
        };
        let c411 = FakeC411 {
            torrents: HashMap::from([("c".to_string(), ONE_FILE.to_vec())]),
            ..FakeC411::default()
        };
        let mut state = State::default();
        let releases = vec![release("c", "Show.S01E02.VOSTFR-Grp")];
        grab_twins(
            &qbt,
            &prowlarr,
            &c411,
            &settings(),
            &mut state,
            &releases,
            &mut BTreeSet::new(),
            &mut Budget { farm: 1e12 },
            0.0,
        )
        .unwrap();
        assert!(qbt.calls().is_empty());
        assert!(state.twins.is_empty());
    }

    #[test]
    fn a_throttled_prowlarr_backs_off() {
        let qbt = FakeQbt::default();
        let prowlarr = FakeProwlarr {
            throttle: true,
            ..FakeProwlarr::default()
        };
        let c411 = FakeC411 {
            catalog: vec![release("c", "Show.S01E02.VOSTFR-Grp")],
            ..FakeC411::default()
        };
        let mut state = State::default();
        let torrents = Torrents::new();
        grab(
            &qbt,
            &prowlarr,
            &c411,
            &settings(),
            &snapshot(&["twins"]),
            &mut state,
            &torrents,
            50.0,
        )
        .unwrap();
        assert_eq!(170.0, state.backoff_until);
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
            &FakeProwlarr::default(),
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
            &FakeProwlarr::default(),
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
    fn a_public_torrent_at_ratio_one_waits_for_its_import_and_its_c411_copy() {
        let qbt = FakeQbt::default();
        let mut fresh = torrent("fresh", "shows");
        fresh.uploaded = fresh.size * 2;
        fresh.seeding_time = 60;
        fresh.save_path = "/data/torrents/shows/fresh".into();
        let mut twin = torrent("twin", FARM);
        twin.uploaded = twin.size * 2;
        twin.seeding_time = 7200;
        twin.save_path = "/data/torrents/farm/twin".into();
        let mut state = State::default();
        state
            .twins
            .insert("twin".into(), ("c".into(), FARM.into(), 0.0, String::new()));
        let mut torrents = by_hash(vec![fresh, twin]);
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
    fn the_c411_copy_of_a_finished_twin_is_added_on_its_files_at_once() {
        let qbt = FakeQbt::default();
        qbt.files.borrow_mut().insert(
            "pub".into(),
            vec![crate::clients::TorrentFile {
                name: "nyaa.mkv".into(),
                size: 1000,
            }],
        );
        qbt.files.borrow_mut().insert(
            "c".into(),
            vec![crate::clients::TorrentFile {
                name: "c411.mkv".into(),
                size: 1000,
            }],
        );
        let c411 = FakeC411 {
            torrents: HashMap::from([("c".to_string(), b"t".to_vec())]),
            ..FakeC411::default()
        };
        let mut state = State::default();
        state
            .twins
            .insert("pub".into(), ("c".into(), FARM.into(), 0.0, String::new()));
        let mut public = torrent("pub", FARM);
        public.save_path = "/data/torrents/farm".into();
        seed_twins(&qbt, &c411, &mut state, &by_hash(vec![public]), 60.0).unwrap();
        assert_eq!(
            vec![
                "add t farm /data/torrents/farm",
                "rename c c411.mkv nyaa.mkv",
                "recheck c"
            ],
            qbt.calls()
        );
    }

    #[test]
    fn the_c411_copy_is_retried_twice_then_given_up() {
        let qbt = FakeQbt::default();
        let c411 = FakeC411::default();
        let mut state = State::default();
        state
            .twins
            .insert("pub".into(), ("c".into(), FARM.into(), 0.0, String::new()));
        let torrents = by_hash(vec![torrent("pub", FARM)]);
        seed_twins(&qbt, &c411, &mut state, &torrents, 0.0).unwrap();
        assert_eq!((1, 600.0), state.stuck["pub"]);
        seed_twins(&qbt, &c411, &mut state, &torrents, 100.0).unwrap();
        assert_eq!(1, c411.downloads.borrow().len());
        seed_twins(&qbt, &c411, &mut state, &torrents, 600.0).unwrap();
        assert_eq!((2, 1800.0), state.stuck["pub"]);
        seed_twins(&qbt, &c411, &mut state, &torrents, 1800.0).unwrap();
        assert!(state.twins.is_empty());
        assert!(state.given_up.contains_key("c"));
    }

    #[test]
    fn a_refused_c411_copy_is_given_up_at_once() {
        let qbt = FakeQbt::default();
        let c411 = FakeC411 {
            refuse: true,
            ..FakeC411::default()
        };
        let mut state = State::default();
        state
            .twins
            .insert("pub".into(), ("c".into(), FARM.into(), 0.0, String::new()));
        seed_twins(
            &qbt,
            &c411,
            &mut state,
            &by_hash(vec![torrent("pub", FARM)]),
            0.0,
        )
        .unwrap();
        assert!(state.twins.is_empty());
    }

    #[test]
    fn a_checked_c411_copy_starts_and_a_gone_twin_is_forgotten() {
        let qbt = FakeQbt::default();
        let mut state = State::default();
        state
            .twins
            .insert("pub".into(), ("c".into(), FARM.into(), 0.0, String::new()));
        state
            .twins
            .insert("gone".into(), ("d".into(), FARM.into(), 0.0, String::new()));
        let mut copy = torrent("c", FARM);
        copy.state = "stoppedUP".into();
        mind_twins(&qbt, &mut state, &by_hash(vec![torrent("pub", FARM), copy])).unwrap();
        assert_eq!(vec!["start c"], qbt.calls());
        assert!(!state.twins.contains_key("gone"));
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
