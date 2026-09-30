//! OpenLina: the mod hub for Mosa Lina. One binary: JSON API, pages, SQLite, package files.
//! See README.md.

pub mod api;
pub mod db;
pub mod openapi;
pub mod store;
pub mod web;

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

/// Open (or create) a data directory and build the app.
pub fn open(data: &Path, cfg: api::Config) -> Result<api::Shared> {
    let store = store::Store::new(data)?;
    let db = db::Db::open(&data.join("openlina.db"))?;
    Ok(Arc::new(api::App::new(db, store, cfg)?))
}

pub fn router(app: api::Shared) -> axum::Router {
    api::router(app).merge(web::router())
}
