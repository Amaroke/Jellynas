mod app;
mod cards;
mod docker;
mod icons;
mod shutdown;
mod space;
mod stats;
mod web;
mod widgets;

use app::App;
use docker::Docker;
use icons::Icons;
use shutdown::Shutdown;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::{env, process, thread};

const PORT: u16 = 8000;
const STATE: &str = "/state";
const WEB: &str = "/web";

fn healthy() -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], PORT));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_secs(5)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut answer = [0; 12];
    stream
        .write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .and_then(|()| stream.read_exact(&mut answer))
        .is_ok_and(|()| answer.ends_with(b" 200"))
}

#[tokio::main]
async fn main() {
    if env::args().nth(1).as_deref() == Some("health") {
        process::exit(if healthy() { 0 } else { 1 });
    }
    let host =
        env::var("DOCKER_HOST").unwrap_or_else(|_| "tcp://dashboard-socket-proxy:2375".into());
    let state = PathBuf::from(STATE);
    let shutdown = Shutdown::load(state.join("shutdown.json")).unwrap_or_else(|error| {
        eprintln!("Cannot read the Shut down services: {error}");
        process::exit(1);
    });
    let app = Arc::new(App::new(
        Docker::new(&host),
        shutdown,
        Icons::new(state.join("icons")),
        PathBuf::from(WEB),
        widgets::Client::new(env::vars().collect()),
    ));
    let watcher = app.clone();
    thread::spawn(move || watcher.watch());
    let measurer = app.clone();
    thread::spawn(move || measurer.measure());
    let poller = app.clone();
    thread::spawn(move || poller.poll());
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", PORT))
        .await
        .unwrap_or_else(|error| {
            eprintln!("Cannot listen on port {PORT}: {error}");
            process::exit(1);
        });
    println!("Serving the Dashboard on port {PORT}");
    let service = web::router(app).into_make_service_with_connect_info::<SocketAddr>();
    if let Err(error) = axum::serve(listener, service).await {
        eprintln!("The Dashboard stopped serving: {error}");
        process::exit(1);
    }
}
