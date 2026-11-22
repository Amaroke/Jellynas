use crate::clients::{Account, Qbt, Torrent, Tracker};
use crate::farm::{
    DAY, FARM, NYAA, ON_C411, OTHER, Torrents, UNKNOWN, WEEK, is_c411, is_seeding, origin_of,
    tracker_of,
};
use crate::state::{Day, GB, Report, Settings, Snapshot, State};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const HISTORY_DAYS: i64 = 30;
const BLOCKED: f64 = 0.8;
const ZEN_PACK: (f64, f64) = (1000.0, 1024.0);
const LIBRARY_ORIGINS: [&str; 3] = ["radarr", "sonarr", "lazylibrarian"];

pub fn gigabytes(size: f64) -> f64 {
    (size / GB * 100.0).round() / 100.0
}

fn share(uploaded: f64, downloaded: f64) -> Option<f64> {
    (downloaded != 0.0).then(|| (uploaded / downloaded * 100.0).round() / 100.0)
}

pub fn day_of(now: f64) -> Date {
    Timestamp::from_second(now as i64)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_zoned_datetime(TimeZone::system())
        .date()
}

fn shown_tracker(origin: &str, tracker: &str) -> Option<String> {
    LIBRARY_ORIGINS
        .contains(&origin)
        .then(|| tracker.to_string())
}

fn tracker_listed(qbt: &dyn Qbt, settings: &Settings, t: &Torrent) -> String {
    let mut urls = vec![t.tracker.to_lowercase()];
    urls.extend(
        qbt.trackers(&t.hash)
            .unwrap_or_default()
            .iter()
            .map(|u| u.to_lowercase()),
    );
    if urls.iter().any(|u| u.contains(&settings.tracker)) {
        ON_C411.into()
    } else if urls.iter().any(|u| u.contains(NYAA)) {
        NYAA.into()
    } else {
        OTHER.into()
    }
}

fn record_origins(qbt: &dyn Qbt, state: &mut State, settings: &Settings, torrents: &Torrents) {
    let hashes: Vec<String> = state.transfers.keys().cloned().collect();
    for hash in &hashes {
        match torrents.get(hash) {
            Some(t) => {
                let tracker = if is_c411(state, settings, t) {
                    ON_C411.to_string()
                } else {
                    tracker_listed(qbt, settings, t)
                };
                let origin = origin_of(t).unwrap_or(UNKNOWN).to_string();
                state.origins.insert(hash.clone(), (origin, tracker));
            }
            None => {
                state
                    .origins
                    .entry(hash.clone())
                    .or_insert((UNKNOWN.into(), OTHER.into()));
            }
        }
    }
    state
        .origins
        .retain(|hash, _| state.transfers.contains_key(hash));
}

fn record_history(state: &mut State, torrents: &Torrents, fresh: &BTreeSet<String>, now: f64) {
    let transfers = state.transfers.clone();
    for (hash, (downloaded, uploaded)) in transfers {
        let t = torrents.get(&hash);
        let last = state
            .history
            .get(&hash)
            .and_then(|samples| samples.last().copied());
        let restarted = t.is_some() && last.is_some_and(|(_, d, u)| downloaded < d || uploaded < u);
        if let Some(t) = t
            && (fresh.contains(&hash) || restarted)
        {
            state
                .history
                .insert(hash.clone(), vec![(t.added_on as f64, 0, 0)]);
        }
        let samples = state.history.entry(hash.clone()).or_default();
        let last = samples.last().copied();
        if t.is_some() && last.is_none_or(|(at, ..)| now - at >= DAY) {
            samples.push((now, downloaded, uploaded));
        }
        if t.is_none() && last.is_some_and(|(_, d, u)| (d, u) != (downloaded, uploaded)) {
            samples.push((now, downloaded, uploaded));
        }
        samples.retain(|(at, ..)| *at >= now - WEEK);
        if samples.is_empty() {
            state.history.remove(&hash);
        }
    }
}

fn record_zen(state: &mut State, torrents: &Torrents) {
    let hashes: Vec<String> = state.transfers.keys().cloned().collect();
    for hash in hashes {
        let Some(t) = torrents.get(&hash) else {
            continue;
        };
        let seeding = t.seeding_time;
        match state.zen.get_mut(&hash) {
            None => {
                if t.category == FARM {
                    state.zen.insert(hash, (seeding, 0));
                }
            }
            Some(entry) => {
                if t.category == FARM {
                    entry.1 += if seeding >= entry.0 {
                        seeding - entry.0
                    } else {
                        seeding
                    };
                }
                entry.0 = seeding;
            }
        }
    }
}

pub fn record(
    qbt: &dyn Qbt,
    state: &mut State,
    torrents: &Torrents,
    settings: &Settings,
    now: f64,
) {
    let mut fresh = BTreeSet::new();
    for t in torrents.values() {
        if !state.transfers.contains_key(&t.hash) {
            fresh.insert(t.hash.clone());
        }
        let downloaded = if state.free.contains_key(&t.hash) {
            0
        } else {
            t.downloaded
        };
        state
            .transfers
            .insert(t.hash.clone(), (downloaded, t.uploaded));
    }
    let transfers = state.transfers.clone();
    state
        .free
        .retain(|hash, _| torrents.contains_key(hash) || !transfers.contains_key(hash));
    state.given_up.retain(|_, at| *at >= now - WEEK);
    state.checked.retain(|_, at| *at >= now - 2.0 * DAY);
    if !torrents.is_empty() {
        state
            .transfers
            .retain(|hash, _| torrents.contains_key(hash));
        state.history.retain(|hash, _| torrents.contains_key(hash));
        state.zen.retain(|hash, _| torrents.contains_key(hash));
    }
    record_origins(qbt, state, settings, torrents);
    record_history(state, torrents, &fresh, now);
    record_zen(state, torrents);
}

type Origin = (String, Option<String>);

fn figures_by_origin(state: &State) -> Vec<(Origin, Value)> {
    let mut totals: BTreeMap<Origin, [f64; 6]> = BTreeMap::new();
    let mut order: Vec<Origin> = vec![];
    for (hash, (downloaded, uploaded)) in &state.transfers {
        let (before_down, before_up) = state
            .history
            .get(hash)
            .and_then(|samples| samples.first())
            .map(|(_, d, u)| (*d, *u))
            .unwrap_or((*downloaded, *uploaded));
        let zen = state.zen.get(hash).map(|(_, total)| *total).unwrap_or(0);
        let (origin, tracker) = state
            .origins
            .get(hash)
            .cloned()
            .unwrap_or((UNKNOWN.into(), OTHER.into()));
        let key = (origin.clone(), shown_tracker(&origin, &tracker));
        if !totals.contains_key(&key) {
            order.push(key.clone());
        }
        let line = [
            1.0,
            *downloaded as f64,
            *uploaded as f64,
            (downloaded - before_down) as f64,
            (uploaded - before_up) as f64,
            zen as f64,
        ];
        let total = totals.entry(key).or_insert([0.0; 6]);
        for (sum, value) in total.iter_mut().zip(line) {
            *sum += value;
        }
    }
    order
        .into_iter()
        .map(|key| {
            let [count, down, up, week_down, week_up, zen] = totals[&key];
            let figures = json!({
                "torrents": count as i64,
                "downloaded_gb": gigabytes(down),
                "uploaded_gb": gigabytes(up),
                "ratio": share(up, down),
                "week_downloaded_gb": gigabytes(week_down),
                "week_uploaded_gb": gigabytes(week_up),
                "week_ratio": share(week_up, week_down),
                "farm_days": (zen / DAY * 10.0).round() / 10.0,
            });
            (key, figures)
        })
        .collect()
}

fn with_origin(key: &Origin, figures: &Value) -> Value {
    let mut entry = json!({"origin": key.0, "tracker": key.1});
    if let (Some(target), Some(source)) = (entry.as_object_mut(), figures.as_object()) {
        target.extend(source.clone());
    }
    entry
}

fn shown(ratio: &Value) -> String {
    ratio
        .as_f64()
        .map_or("-".into(), |value| format!("{value:.2}"))
}

pub fn report(state: &mut State, now: f64) {
    if now - state.report.at < DAY {
        return;
    }
    let figures = figures_by_origin(state);
    state.report = Report {
        at: now,
        origins: figures.iter().map(|(key, f)| with_origin(key, f)).collect(),
    };
    for ((origin, tracker), f) in figures {
        let name = [Some(origin), tracker]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "Origin {name}: {} torrents, {:.1} GB down, {:.1} GB up, ratio {}, last 7 days {:.1} GB down, {:.1} GB up, ratio {}, {:.1} days in the Farm",
            f["torrents"],
            f["downloaded_gb"].as_f64().unwrap_or_default(),
            f["uploaded_gb"].as_f64().unwrap_or_default(),
            shown(&f["ratio"]),
            f["week_downloaded_gb"].as_f64().unwrap_or_default(),
            f["week_uploaded_gb"].as_f64().unwrap_or_default(),
            shown(&f["week_ratio"]),
            f["farm_days"].as_f64().unwrap_or_default(),
        );
    }
}

pub fn read_account(c411: &dyn Tracker, state: &mut State, now: f64) -> Option<Account> {
    let account = match c411.account() {
        Ok(account) => account,
        Err(error) => {
            println!("Cannot read the profile of C411, ratio spends no download on C411: {error}");
            return None;
        }
    };
    if state.account.last().is_none_or(|(at, ..)| now - at >= DAY) {
        state
            .account
            .push((now, account.uploaded, account.downloaded, account.zen));
    }
    state.account.retain(|(at, ..)| *at >= now - WEEK - DAY);
    Some(account)
}

fn download_left(uploaded: f64, downloaded: f64, floor: f64) -> f64 {
    uploaded / floor - downloaded
}

fn with_account(entry: &mut Day, uploaded: f64, downloaded: f64, floor: f64) {
    entry.ratio = share(uploaded, downloaded);
    entry.left_gb = Some(gigabytes(download_left(uploaded, downloaded, floor)));
    entry.uploaded_gb = Some(gigabytes(uploaded));
    entry.downloaded_gb = Some(gigabytes(downloaded));
}

fn backfill(state: &mut State, settings: &Settings, today: &str) {
    let first = state
        .daily
        .first()
        .map(|e| e.day.clone())
        .unwrap_or(today.into());
    let mut older: BTreeMap<String, Day> = BTreeMap::new();
    for (at, uploaded, downloaded, zen) in &state.account {
        let day = day_of(*at).to_string();
        if day < first {
            let mut entry = Day {
                day: day.clone(),
                floor: settings.floor,
                ..Day::default()
            };
            with_account(
                &mut entry,
                *uploaded as f64,
                *downloaded as f64,
                settings.floor,
            );
            entry.zen = Some(*zen);
            older.insert(day, entry);
        }
    }
    let mut daily: Vec<Day> = older.into_values().collect();
    daily.append(&mut state.daily);
    state.daily = daily;
}

fn base_of(samples: &mut Vec<(f64, f64)>, now: f64, value: f64) -> f64 {
    samples.push((now, value));
    let before: Vec<(f64, f64)> = samples
        .iter()
        .copied()
        .filter(|(at, _)| *at <= now - DAY)
        .collect();
    let recent: Vec<(f64, f64)> = samples
        .iter()
        .copied()
        .filter(|(at, _)| *at > now - DAY)
        .collect();
    *samples = before.last().copied().into_iter().chain(recent).collect();
    samples[0].1
}

fn real_upload(account: &Account) -> f64 {
    account.actual.unwrap_or(account.uploaded) as f64
}

pub fn record_day(
    state: &mut State,
    settings: &Settings,
    seeds: usize,
    account: Option<&Account>,
    now: f64,
) {
    let today = day_of(now);
    let today_text = today.to_string();
    backfill(state, settings, &today_text);
    let current = match state.daily.last() {
        Some(last) if last.day == today_text => state.daily.pop(),
        _ => None,
    };
    let yesterday = today
        .checked_sub(1.day())
        .map(|day| day.to_string())
        .unwrap_or_default();
    let earned = state
        .daily
        .iter()
        .find(|e| e.day == yesterday)
        .and_then(|e| e.earned);
    let mut entry = current.unwrap_or_default();
    entry.day = today_text;
    entry.floor = settings.floor;
    entry.seeds = Some(seeds as i64);
    if let Some(account) = account {
        let (up, down) = (account.uploaded as f64, account.downloaded as f64);
        with_account(&mut entry, up, down, settings.floor);
        let received = base_of(&mut state.received, now, down);
        let sent = base_of(&mut state.sent, now, real_upload(account));
        entry.downloaded_day_gb = Some(gigabytes(down - received));
        entry.uploaded_day_gb = Some(gigabytes(real_upload(account) - sent));
        entry.actual_gb = account.actual.map(|actual| gigabytes(actual as f64));
        entry.zen = Some(account.zen);
        entry.earned = account.earned;
        entry.zen_paid = match (earned, account.earned) {
            (Some(before), Some(now)) => Some(now - before),
            _ => None,
        };
    }
    state.daily.push(entry);
    let oldest = today
        .checked_sub((HISTORY_DAYS - 1).days())
        .map(|day| day.to_string())
        .unwrap_or_default();
    state.daily.retain(|e| e.day >= oldest);
}

fn tiers_of(account: Option<&Account>, settings: &Settings) -> Vec<Value> {
    let mut found: Vec<(f64, Value)> = [
        ("blocked", BLOCKED),
        ("even", 1.0),
        ("floor", settings.floor),
        ("goal", 2.0),
    ]
    .into_iter()
    .map(|(name, ratio)| {
        let gap = account.map(|a| {
            let (up, down) = (a.uploaded as f64, a.downloaded as f64);
            if up < ratio * down {
                gigabytes(ratio * down - up)
            } else {
                -gigabytes(up / ratio - down)
            }
        });
        (ratio, json!({"name": name, "ratio": ratio, "gap_gb": gap}))
    })
    .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|(_, tier)| tier).collect()
}

fn zen_gigabytes(zen: f64) -> f64 {
    let packs = (zen / ZEN_PACK.0).floor();
    packs * ZEN_PACK.1 + (zen - packs * ZEN_PACK.0)
}

fn account_board(account: Option<&Account>, state: &State, settings: &Settings) -> Value {
    let Some(a) = account else {
        return json!({"ratio": null, "floor": settings.floor, "paused": true, "allowed": null});
    };
    let (up, down) = (a.uploaded as f64, a.downloaded as f64);
    let available = (gigabytes(up) + zen_gigabytes(a.zen)) / BLOCKED - gigabytes(down);
    json!({
        "ratio": share(up, down),
        "floor": settings.floor,
        "paused": state.paused,
        "allowed": a.allowed,
        "uploaded_gb": gigabytes(up),
        "downloaded_gb": gigabytes(down),
        "actual_gb": a.actual.map(|v| gigabytes(v as f64)),
        "credit_gb": a.credit.map(|v| gigabytes(v as f64)),
        "zen": a.zen,
        "zen_gb": zen_gigabytes(a.zen),
        "available_gb": (available * 100.0).round() / 100.0,
        "class": a.class,
        "grade": a.grade,
    })
}

fn timestamp_text(now: f64) -> String {
    Timestamp::from_second(now as i64)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .strftime("%Y-%m-%dT%H:%M:%S+00:00")
        .to_string()
}

fn origins_board(state: &State) -> Vec<Value> {
    let mut figures = figures_by_origin(state);
    figures.sort_by(|a, b| {
        let up = |f: &Value| f["uploaded_gb"].as_f64().unwrap_or_default();
        up(&b.1).total_cmp(&up(&a.1))
    });
    figures.iter().map(|(key, f)| with_origin(key, f)).collect()
}

pub fn seeded_on_c411(state: &State, settings: &Settings, torrents: &Torrents) -> usize {
    torrents
        .values()
        .filter(|t| is_c411(state, settings, t) && is_seeding(t))
        .count()
}

#[allow(clippy::too_many_arguments)]
pub fn board_of(
    state: &State,
    settings: &Settings,
    controls: &Snapshot,
    torrents: &Torrents,
    seeds: usize,
    account: Option<&Account>,
    now: f64,
) -> Value {
    let mut slices: Vec<(Origin, f64)> = vec![];
    for t in torrents.values().filter(|t| t.category == FARM) {
        let origin = origin_of(t).unwrap_or(UNKNOWN).to_string();
        let key = (
            origin.clone(),
            shown_tracker(&origin, &tracker_of(state, settings, t)),
        );
        match slices.iter_mut().find(|(k, _)| *k == key) {
            Some((_, size)) => *size += t.size as f64,
            None => slices.push((key, t.size as f64)),
        }
    }
    slices.sort_by(|a, b| b.1.total_cmp(&a.1));
    json!({
        "updated_at": timestamp_text(now),
        "account": account_board(account, state, settings),
        "tiers": tiers_of(account, settings),
        "history": state.daily,
        "seeds": seeds,
        "controls": controls,
        "farm": {
            "budget_gb": gigabytes(settings.farm_size),
            "slices": slices.iter().map(|((o, t), size)| json!({"origin": o, "tracker": t, "gb": gigabytes(*size)})).collect::<Vec<_>>(),
        },
        "origins": origins_board(state),
    })
}

pub fn waiting_board(state: &State, settings: &Settings, controls: &Snapshot) -> Value {
    json!({
        "updated_at": null,
        "account": {"ratio": null, "floor": settings.floor, "paused": true, "allowed": null},
        "tiers": tiers_of(None, settings),
        "history": state.daily,
        "seeds": null,
        "controls": controls,
        "farm": {"budget_gb": gigabytes(settings.farm_size), "slices": []},
        "origins": [],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::farm::fakes::*;

    fn account(uploaded: i64, downloaded: i64) -> Account {
        Account {
            uploaded,
            downloaded,
            zen: 1500.0,
            earned: Some(100.0),
            allowed: true,
            ..Account::default()
        }
    }

    #[test]
    fn the_ledger_counts_each_torrent_under_its_origin_and_tracker() {
        let qbt = FakeQbt::default();
        let mut state = State::default();
        let mut movie = torrent("m", "movies");
        movie.downloaded = 100;
        movie.uploaded = 50;
        movie.tracker = "https://c411.org/announce".into();
        let mut farm = torrent("f", FARM);
        farm.uploaded = 70;
        farm.seeding_time = 3600;
        let torrents: Torrents = [movie, farm]
            .into_iter()
            .map(|t| (t.hash.clone(), t))
            .collect();
        record(&qbt, &mut state, &torrents, &settings(), 1000.0);
        assert_eq!(
            ("radarr".to_string(), "c411".to_string()),
            state.origins["m"]
        );
        assert_eq!(
            ("farm".to_string(), "other".to_string()),
            state.origins["f"]
        );
        assert_eq!((3600, 0), state.zen["f"]);
        let board = origins_board(&state);
        assert_eq!("farm", board[0]["origin"]);
        assert_eq!(Value::Null, board[0]["tracker"]);
        assert_eq!("c411", board[1]["tracker"]);
    }

    #[test]
    fn a_freeleech_grab_counts_no_download() {
        let mut state = State::default();
        state.free.insert("f".into(), 0.0);
        let mut farm = torrent("f", FARM);
        farm.downloaded = 500;
        let torrents: Torrents = [(farm.hash.clone(), farm)].into();
        record(
            &FakeQbt::default(),
            &mut state,
            &torrents,
            &settings(),
            10.0,
        );
        assert_eq!((0, 0), state.transfers["f"]);
    }

    #[test]
    fn the_day_keeps_the_account_and_what_moved_since_yesterday() {
        let mut state = State::default();
        let now = 1_800_000_000.0;
        record_day(
            &mut state,
            &settings(),
            4,
            Some(&account(300 * GB as i64, 100 * GB as i64)),
            now - DAY,
        );
        let mut later = account(400 * GB as i64, 120 * GB as i64);
        later.earned = Some(160.0);
        record_day(&mut state, &settings(), 5, Some(&later), now);
        let today = state.daily.last().unwrap();
        assert_eq!(2, state.daily.len());
        assert_eq!(Some(3.33), today.ratio);
        assert_eq!(Some(100.0), today.uploaded_day_gb);
        assert_eq!(Some(20.0), today.downloaded_day_gb);
        assert_eq!(Some(60.0), today.zen_paid);
        assert_eq!(Some(80.0), today.left_gb);
        assert_eq!(Some(5), today.seeds);
    }

    #[test]
    fn the_tiers_sort_with_the_floor_and_tell_how_far_each_is() {
        let tiers = tiers_of(Some(&account(150, 100)), &settings());
        let names: Vec<&str> = tiers.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(vec!["blocked", "even", "floor", "goal"], names);
    }

    #[test]
    fn the_board_splits_the_farm_disk_by_origin() {
        let state = State::default();
        let mut cross = torrent("x", FARM);
        cross.save_path = "/data/torrents/cross-seed/C411".into();
        cross.size = 3 * GB as i64;
        let farm = torrent("f", FARM);
        let torrents: Torrents = [cross, farm]
            .into_iter()
            .map(|t| (t.hash.clone(), t))
            .collect();
        let snapshot = Snapshot {
            switches: BTreeMap::new(),
            numbers: BTreeMap::new(),
        };
        let board = board_of(
            &state,
            &settings(),
            &snapshot,
            &torrents,
            2,
            Some(&account(150, 100)),
            0.0,
        );
        assert_eq!("cross-seed", board["farm"]["slices"][0]["origin"]);
        assert_eq!(3.0, board["farm"]["slices"][0]["gb"]);
        assert_eq!("1970-01-01T00:00:00+00:00", board["updated_at"]);
        assert_eq!(1524.0, board["account"]["zen_gb"]);
    }

    #[test]
    fn zen_trades_by_packs_of_a_thousand() {
        assert_eq!(1024.0 + 500.0, zen_gigabytes(1500.0));
    }
}
