use crate::app::{App, Outcome};
use crate::cards::Section;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::Stream;
use minijinja::value::Serde;
use minijinja::{Environment, context};
use serde::Serialize;
use serde_json::Value;
use std::convert::Infallible;
use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::watch;
use tokio::task;

const CADDY: &str = "caddy";

#[derive(Serialize)]
struct Heading {
    slug: Section,
    title: &'static str,
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/theme.css", get(theme))
        .route("/assets/{file}", get(asset))
        .route("/events", get(events))
        .route("/icons/{file}", get(icon))
        .route("/services/{service}/{action}", post(control))
        .route("/farm/controls", post(farm_control))
        .route("/quick-actions/{key}", post(quick_action))
        .route("/seerr/requests/{id}/{action}", post(answer_request))
        .route("/healthz", get(health))
        .with_state(app)
}

fn failed(what: &str, error: impl std::fmt::Display) -> Response {
    eprintln!("Cannot {what}: {error}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

async fn page(State(app): State<Arc<App>>) -> Response {
    let source = match fs::read_to_string(app.web.join("index.html")) {
        Ok(source) => source,
        Err(error) => return failed("read the page template", error),
    };
    let sections: Vec<Heading> = Section::ALL
        .into_iter()
        .map(|slug| Heading {
            slug,
            title: slug.title(),
        })
        .collect();
    let services = app.snapshot.borrow().clone();
    let board = app.board.borrow().clone();
    let rendered = Environment::new().render_str(
        &source,
        context! {
            sections => Serde(&sections),
            services => Serde(&*services),
            board => Serde(&*board),
        },
    );
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(error) => failed("render the page", error),
    }
}

fn asset_kind(file: &str) -> Option<&'static str> {
    let (name, extension) = file.split_once('.')?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
    {
        return None;
    }
    match extension {
        "js" => Some("text/javascript"),
        "css" => Some("text/css"),
        "woff2" => Some("font/woff2"),
        "png" => Some("image/png"),
        "webmanifest" => Some("application/manifest+json"),
        _ => None,
    }
}

fn cache_of(kind: &str) -> Option<&'static str> {
    matches!(kind, "font/woff2" | "image/png").then_some("public, max-age=2592000")
}

async fn asset(State(app): State<Arc<App>>, Path(file): Path<String>) -> Response {
    let Some(kind) = asset_kind(&file) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match fs::read(app.web.join(&file)) {
        Ok(body) => match cache_of(kind) {
            Some(cache) => (
                [(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, cache)],
                body,
            )
                .into_response(),
            None => ([(header::CONTENT_TYPE, kind)], body).into_response(),
        },
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn theme(State(app): State<Arc<App>>) -> Response {
    match fs::read(app.web.join("theme.css")) {
        Ok(css) => ([(header::CONTENT_TYPE, "text/css")], css).into_response(),
        Err(error) => failed("read the theme", error),
    }
}

fn follow<T: Serialize + Send + Sync + 'static>(
    receiver: watch::Receiver<Arc<T>>,
    name: &'static str,
) -> impl Stream<Item = Result<Event, Infallible>> {
    futures_util::stream::unfold((receiver, true), move |(mut receiver, first)| async move {
        if !first {
            receiver.changed().await.ok()?;
        }
        let data = serde_json::to_string(&**receiver.borrow_and_update()).ok()?;
        let event = Event::default().event(name).data(data);
        Some((Ok(event), (receiver, false)))
    })
}

async fn events(State(app): State<Arc<App>>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = futures_util::stream::select(
        follow(app.snapshot.subscribe(), "services"),
        follow(app.board.subscribe(), "board"),
    );
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn icon(State(app): State<Arc<App>>, Path(file): Path<String>) -> Response {
    match task::spawn_blocking(move || app.icon(&file)).await {
        Ok(Some((kind, bytes))) => (
            [
                (header::CONTENT_TYPE, kind),
                (header::CACHE_CONTROL, "public, max-age=604800"),
                (
                    header::CONTENT_SECURITY_POLICY,
                    "default-src 'none'; style-src 'unsafe-inline'",
                ),
            ],
            bytes,
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => failed("serve an icon", error),
    }
}

async fn from_caddy(peer: SocketAddr) -> bool {
    match tokio::net::lookup_host((CADDY, 0)).await {
        Ok(mut addresses) => addresses.any(|address| address.ip() == peer.ip()),
        Err(error) => {
            eprintln!("Cannot resolve {CADDY}: {error}");
            false
        }
    }
}

async fn act(
    peer: SocketAddr,
    headers: &HeaderMap,
    work: impl FnOnce() -> Outcome + Send + 'static,
) -> StatusCode {
    if !headers.contains_key("x-dashboard") || !from_caddy(peer).await {
        return StatusCode::FORBIDDEN;
    }
    match task::spawn_blocking(work).await {
        Ok(Outcome::Done) => StatusCode::NO_CONTENT,
        Ok(Outcome::Unknown) => StatusCode::NOT_FOUND,
        Ok(Outcome::Invalid) => StatusCode::BAD_REQUEST,
        Ok(Outcome::Refused) => StatusCode::CONFLICT,
        Ok(Outcome::Failed) => StatusCode::BAD_GATEWAY,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn control(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((service, action)): Path<(String, String)>,
    headers: HeaderMap,
) -> StatusCode {
    act(peer, &headers, move || app.control(&service, &action)).await
}

async fn farm_control(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(control): Json<Value>,
) -> StatusCode {
    act(peer, &headers, move || app.farm_control(&control)).await
}

async fn quick_action(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> StatusCode {
    act(peer, &headers, move || app.quick_action(&key)).await
}

async fn answer_request(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((id, action)): Path<(u64, String)>,
    headers: HeaderMap,
) -> StatusCode {
    act(peer, &headers, move || app.answer_request(id, &action)).await
}

async fn health(State(app): State<Arc<App>>) -> StatusCode {
    if app.healthy.load(Ordering::Relaxed) {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_starts_at_the_page_and_names_icons_that_are_served() {
        let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web");
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(web.join("manifest.webmanifest")).unwrap())
                .unwrap();
        assert_eq!("/", manifest["start_url"]);
        assert_eq!("/", manifest["scope"]);
        assert_eq!("standalone", manifest["display"]);
        let icons = manifest["icons"].as_array().unwrap();
        let sizes: Vec<&str> = icons
            .iter()
            .filter_map(|icon| icon["sizes"].as_str())
            .collect();
        assert!(sizes.contains(&"192x192"));
        assert!(sizes.contains(&"512x512"));
        assert!(icons.iter().any(|icon| icon["purpose"] == "maskable"));
        for icon in icons {
            let file = icon["src"]
                .as_str()
                .unwrap()
                .strip_prefix("/assets/")
                .unwrap();
            assert_eq!(Some("image/png"), asset_kind(file), "{file}");
            assert!(web.join(file).is_file(), "{file}");
        }
    }

    #[test]
    fn only_fonts_and_icons_are_cached_since_the_rest_changes_with_each_deploy() {
        assert_eq!(Some("public, max-age=2592000"), cache_of("font/woff2"));
        assert_eq!(Some("public, max-age=2592000"), cache_of("image/png"));
        assert_eq!(None, cache_of("application/manifest+json"));
        assert_eq!(None, cache_of("text/javascript"));
        assert_eq!(None, cache_of("text/css"));
    }

    #[test]
    fn only_plain_scripts_styles_fonts_images_and_the_manifest_are_assets() {
        assert_eq!(Some("text/javascript"), asset_kind("sections.js"));
        assert_eq!(Some("text/css"), asset_kind("sections.css"));
        assert_eq!(Some("font/woff2"), asset_kind("plex-sans-medium.woff2"));
        assert_eq!(Some("image/png"), asset_kind("icon-maskable.png"));
        assert_eq!(
            Some("application/manifest+json"),
            asset_kind("manifest.webmanifest")
        );
        for file in [
            "index.html",
            "../state.js",
            "Sections.js",
            "a/b.css",
            ".js",
            "theme.css.map",
            "plex-sans.woff",
        ] {
            assert_eq!(None, asset_kind(file), "{file}");
        }
    }
}
