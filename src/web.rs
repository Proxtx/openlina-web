//! The pages: static HTML/CSS/JS compiled into the binary. They talk to the JSON API.

use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;

const LAYOUT: &str = include_str!("../static/layout.html");

fn page(title: &str, page: &str) -> Html<String> {
    Html(LAYOUT.replace("{{title}}", title).replace("{{page}}", page))
}

pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/", get(|| async { page("OpenLina: mods for Mosa Lina", "browse") }))
        .route("/mods/{id}", get(|| async { page("OpenLina", "mod") }))
        .route("/pack", get(|| async { page("OpenLina: your pack", "pack") }))
        .route("/agents", get(|| async { page("OpenLina: for agents", "agents") }))
        .route("/review", get(|| async { page("OpenLina: review", "review") }))
        .route("/static/{file}", get(asset))
}

async fn asset(Path(file): Path<String>) -> Response {
    let (body, mime): (&'static str, &str) = match file.as_str() {
        "app.js" => (include_str!("../static/app.js"), "text/javascript; charset=utf-8"),
        "app.css" => (include_str!("../static/app.css"), "text/css; charset=utf-8"),
        "favicon.svg" => (include_str!("../static/favicon.svg"), "image/svg+xml"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "no-cache")], body).into_response()
}
