mod clients;
mod farm;
mod http;
mod ledger;
mod state;

use clients::{C411, Fail, Indexers, Prowlarr, QBittorrent, Qbt, Tracker};
use farm::{C411_CALLS, Torrents};
use serde_json::{Value, json};
use state::{Controls, GB, Settings, Snapshot, State};
use std::env;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STATE: &str = "/state/state.json";
const CONTROLS: &str = "/state/controls.json";
const CYCLE: Duration = Duration::from_secs(60);

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64())
        .unwrap_or_default()
}

fn read(name: &str) -> String {
    env::var(name).unwrap_or_default().trim().to_string()
}

fn positive(name: &str, fallback: f64) -> f64 {
    let text = read(name);
    if text.is_empty() {
        return fallback;
    }
    match text.parse::<f64>() {
        Ok(value) if value > 0.0 => value,
        _ => {
            eprintln!("Set {name} in .env to a positive number, or leave it empty");
            std::process::exit(1);
        }
    }
}

fn ids(name: &str) -> Vec<String> {
    read(name)
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

fn settings_from_env() -> Settings {
    let tracker = read("RATIO_TRACKER").to_lowercase();
    Settings {
        tracker: if tracker.is_empty() {
            "c411".into()
        } else {
            tracker
        },
        hold_indexers: ids("RATIO_HOLD_INDEXER_IDS"),
        twin_indexers: ids("RATIO_TWIN_INDEXER_IDS"),
        cross_seed_key: read("CROSS_SEED_API_KEY"),
        farm_size: positive("RATIO_MAX_SIZE_GB", 100.0) * GB,
        floor: positive("RATIO_FLOOR", 2.0),
        free_max_size: 5.0 * GB,
    }
}

fn listed(qbt: &dyn Qbt) -> Result<Torrents, String> {
    Ok(qbt
        .torrents()?
        .into_iter()
        .map(|mut t| {
            t.hash = t.hash.to_lowercase();
            (t.hash.clone(), t)
        })
        .collect())
}

fn spent(outcome: Result<(), Fail>) -> bool {
    match outcome {
        Ok(()) => true,
        Err(Fail::OutOfCalls) => {
            println!(
                "ratio spent its {C411_CALLS} calls to C411 of the run, the rest waits for the next run"
            );
            false
        }
        Err(error) => {
            println!("Cannot read C411: {error}");
            true
        }
    }
}

fn run_once(
    qbt: &dyn Qbt,
    prowlarr: &dyn Indexers,
    c411: &dyn Tracker,
    base: &Settings,
    controls: &Snapshot,
    state: &mut State,
    now: f64,
) -> Result<Value, String> {
    let settings = base.with(controls);
    farm::prepare(qbt, &settings)?;
    c411.allow(C411_CALLS);
    let account = ledger::read_account(c411, state, now);
    farm::weigh_floor(state, account.as_ref(), &settings);
    let mut torrents = listed(qbt)?;
    ledger::record(qbt, state, &torrents, &settings, now);
    ledger::report(state, now);
    let hold = state.paused && controls.on("hold");
    if let Err(error) = farm::use_prowlarr(state, now, true, |state| {
        farm::hold_c411(prowlarr, &settings, state, hold)
    }) {
        println!("Cannot reach Prowlarr: {error}");
    }
    if controls.on("adopt") {
        farm::adopt(qbt, &settings, state, &mut torrents)?;
    }
    farm::mind_twins(qbt, state, &torrents)?;
    if controls.on("clean_public") {
        farm::clean_public(qbt, &settings, state, &mut torrents)?;
    }
    farm::tag(qbt, &torrents)?;
    let seeded = farm::use_prowlarr(state, now, false, |state| {
        farm::seed_twins(qbt, c411, state, &torrents, now)
    });
    if spent(seeded) {
        let grabbed = farm::grab(
            qbt, prowlarr, c411, &settings, controls, state, &torrents, now,
        );
        spent(grabbed);
    }
    let torrents = listed(qbt)?;
    let seeds = ledger::seeded_on_c411(state, &settings, &torrents);
    ledger::record_day(state, &settings, seeds, account.as_ref(), now);
    Ok(ledger::board_of(
        state,
        &settings,
        controls,
        &torrents,
        seeds,
        account.as_ref(),
        now,
    ))
}

struct Shared {
    board: Mutex<Value>,
    controls: Mutex<Controls>,
    settings: Settings,
    wake: Mutex<Sender<()>>,
}

fn answer(request: tiny_http::Request, code: u16, value: &Value) {
    let header =
        tiny_http::Header::from_bytes("Content-Type", "application/json").expect("valid header");
    let response = tiny_http::Response::from_string(value.to_string())
        .with_status_code(code)
        .with_header(header);
    let _ = request.respond(response);
}

fn handle(shared: &Shared, mut request: tiny_http::Request) {
    let path = request.url().trim_end_matches('/').to_string();
    match (request.method(), path.as_str()) {
        (tiny_http::Method::Get, "/board") => {
            let board = shared.board.lock().expect("board lock").clone();
            answer(request, 200, &board);
        }
        (tiny_http::Method::Post, "/controls") => {
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            let asked: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let name = asked["name"].as_str().unwrap_or_default();
            let mut controls = shared.controls.lock().expect("controls lock");
            if let Err(error) = controls.change(name, &asked["value"]) {
                drop(controls);
                answer(request, 400, &json!({"error": error}));
                return;
            }
            let snapshot =
                serde_json::to_value(controls.snapshot(&shared.settings)).unwrap_or(Value::Null);
            drop(controls);
            shared.board.lock().expect("board lock")["controls"] = snapshot.clone();
            let _ = shared.wake.lock().expect("wake lock").send(());
            answer(request, 200, &snapshot);
        }
        _ => answer(request, 404, &json!({"error": "not found"})),
    }
}

fn serve(shared: Arc<Shared>) -> Result<(), String> {
    let server = tiny_http::Server::http("0.0.0.0:8000").map_err(|error| error.to_string())?;
    thread::spawn(move || {
        for request in server.incoming_requests() {
            handle(&shared, request);
        }
    });
    Ok(())
}

fn main() {
    let settings = settings_from_env();
    let proxy = env::var("HTTPS_PROXY").ok();
    let clients = (|| -> Result<_, String> {
        Ok((
            QBittorrent::new(
                "http://gluetun:8080",
                &read("QBITTORRENT_USER"),
                &read("QBITTORRENT_PASSWORD"),
            )?,
            Prowlarr::new(
                "http://prowlarr:9696",
                &read("PROWLARR_API_KEY"),
                proxy.as_deref(),
            )?,
            C411::new(
                &read("C411_PROFILE_API_KEY"),
                &read("C411_TORZNAB_API_KEY"),
                proxy.as_deref(),
            )?,
        ))
    })();
    let (qbt, prowlarr, c411) = clients.unwrap_or_else(|error| {
        eprintln!("Cannot set up the clients: {error}");
        std::process::exit(1);
    });
    let mut state = state::load_state(Path::new(STATE)).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(1);
    });
    let controls = Controls::load(Path::new(CONTROLS));
    let snapshot = controls.snapshot(&settings);
    let (wake, woken): (Sender<()>, Receiver<()>) = mpsc::channel();
    let shared = Arc::new(Shared {
        board: Mutex::new(ledger::waiting_board(
            &state,
            &settings.with(&snapshot),
            &snapshot,
        )),
        controls: Mutex::new(controls),
        settings: settings.clone(),
        wake: Mutex::new(wake),
    });
    if let Err(error) = serve(shared.clone()) {
        eprintln!("Cannot serve the board: {error}");
        std::process::exit(1);
    }
    println!("ratio runs the Farm every {} seconds", CYCLE.as_secs());
    loop {
        let snapshot = shared
            .controls
            .lock()
            .expect("controls lock")
            .snapshot(&settings);
        match run_once(
            &qbt,
            &prowlarr,
            &c411,
            &settings,
            &snapshot,
            &mut state,
            now(),
        ) {
            Ok(board) => {
                let current = shared
                    .controls
                    .lock()
                    .expect("controls lock")
                    .snapshot(&settings);
                let mut board = board;
                board["controls"] = serde_json::to_value(current).unwrap_or(Value::Null);
                *shared.board.lock().expect("board lock") = board;
            }
            Err(error) => {
                println!("Cannot run the Farm: {error}");
                let mut board = shared.board.lock().expect("board lock");
                board["history"] = serde_json::to_value(&state.daily).unwrap_or(Value::Null);
                board["controls"] = serde_json::to_value(&snapshot).unwrap_or(Value::Null);
            }
        }
        if let Err(error) = state::write_json(Path::new(STATE), &state) {
            println!("{error}");
        }
        let _ = woken.recv_timeout(CYCLE);
        while woken.try_recv().is_ok() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farm::fakes::*;

    #[test]
    fn a_run_reads_the_account_and_records_the_day() {
        let qbt = FakeQbt::default();
        *qbt.torrents.borrow_mut() = vec![torrent("A", farm::FARM)];
        let c411 = FakeC411 {
            account: Some(clients::Account {
                uploaded: 300,
                downloaded: 100,
                allowed: true,
                ..Default::default()
            }),
            ..FakeC411::default()
        };
        let mut state = State::default();
        let controls =
            Controls::load(Path::new("/nonexistent/controls.json")).snapshot(&settings());
        let board = run_once(
            &qbt,
            &FakeProwlarr::default(),
            &c411,
            &settings(),
            &controls,
            &mut state,
            1_800_000_000.0,
        )
        .unwrap();
        assert_eq!(3.0, board["account"]["ratio"]);
        assert!(!state.paused);
        assert!(state.transfers.contains_key("a"));
        assert_eq!(1, state.daily.len());
        assert_eq!(0, board["seeds"]);
    }
}
