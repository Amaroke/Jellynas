use crate::app::{App, Outcome};
use crate::cards::Section;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::Stream;
use minijinja::value::Serde;
use minijinja::{Environment, context};
use serde::Serialize;
use std::convert::Infallible;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::task;

#[derive(Serialize)]
struct Heading {
    slug: Section,
    title: &'static str,
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/theme.css", get(theme))
        .route("/events", get(events))
        .route("/icons/{file}", get(icon))
        .route("/services/{service}/{action}", post(control))
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
    let rendered = Environment::new().render_str(
        &source,
        context! {
            sections => Serde(&sections),
            services => Serde(&*services),
        },
    );
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(error) => failed("render the page", error),
    }
}

async fn theme(State(app): State<Arc<App>>) -> Response {
    match fs::read(app.web.join("theme.css")) {
        Ok(css) => ([(header::CONTENT_TYPE, "text/css")], css).into_response(),
        Err(error) => failed("read the theme", error),
    }
}

async fn events(State(app): State<Arc<App>>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let receiver = app.snapshot.subscribe();
    let stream =
        futures_util::stream::unfold((receiver, true), |(mut receiver, first)| async move {
            if !first {
                receiver.changed().await.ok()?;
            }
            let data = serde_json::to_string(&**receiver.borrow_and_update()).ok()?;
            let event = Event::default().event("services").data(data);
            Some((Ok(event), (receiver, false)))
        });
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

async fn control(
    State(app): State<Arc<App>>,
    Path((service, action)): Path<(String, String)>,
    headers: HeaderMap,
) -> StatusCode {
    if !headers.contains_key("x-dashboard") {
        return StatusCode::FORBIDDEN;
    }
    match task::spawn_blocking(move || app.control(&service, &action)).await {
        Ok(Outcome::Done) => StatusCode::NO_CONTENT,
        Ok(Outcome::Unknown) => StatusCode::NOT_FOUND,
        Ok(Outcome::Refused) => StatusCode::CONFLICT,
        Ok(Outcome::Failed) => StatusCode::BAD_GATEWAY,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn health(State(app): State<Arc<App>>) -> StatusCode {
    if app.healthy.load(Ordering::Relaxed) {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
