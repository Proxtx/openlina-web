//! The HTTP API (`/api/…`, JSON) and media files. The pages in `web` use the same API.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use openlina_sdk::manifest::{ModManifest, ModPack, OptionType, PackEntry, Section};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::db::{self, Db, User, Version};
use crate::store::{self, Store};

/// Sections shown on the website (and accepted for section-wide change requests).
pub const SECTIONS: &[&str] = &["items", "modifiers", "levels", "general"];

pub struct Config {
    /// Base URL used in exported packs, e.g. `https://openlina.example` (no trailing slash).
    pub public_url: String,
    pub game_build: String,
    /// Take the client address from `X-Forwarded-For` (only behind a reverse proxy you control).
    pub trust_proxy: bool,
}

pub struct App {
    pub db: Db,
    pub store: Store,
    pub cfg: Config,
    salt: Vec<u8>,
}

impl App {
    pub fn new(db: Db, store: Store, cfg: Config) -> anyhow::Result<Self> {
        let salt = db.salt()?;
        Ok(Self { db, store, cfg, salt })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.cfg.public_url.trim_end_matches('/'))
    }
}

pub type Shared = Arc<App>;

pub fn router(app: Shared) -> Router {
    Router::new()
        .route("/api/mods", get(list_mods).post(upload))
        .route("/api/mods/{id}", get(mod_detail))
        .route("/api/mods/{id}/vote", post(vote))
        .route("/api/mods/{id}/{version}/package", get(package))
        .route("/api/mods/{id}/{version}/review", post(review))
        .route("/api/packs", post(create_pack))
        .route("/api/packs/{id}", get(get_pack))
        .route("/api/packs/{id}/zip", get(pack_zip))
        .route("/api/me", get(me))
        .route("/api/me/token", post(new_token))
        .route("/api/review", get(review_queue))
        .route("/api/openapi.json", get(openapi))
        .route("/media/{id}/{version}/{file}", get(media))
        .layer(DefaultBodyLimit::max(store::MAX_PACKAGE + (1 << 20)))
        .with_state(app)
}

// ---------------------------------------------------------------------- errors

pub struct ApiError(StatusCode, String);

impl ApiError {
    fn new(code: StatusCode, msg: impl Into<String>) -> Self {
        Self(code, msg.into())
    }

    pub fn message(&self) -> &str {
        &self.1
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        eprintln!("internal error: {e:#}");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        anyhow::Error::from(e).into()
    }
}

type ApiResult<T> = Result<T, ApiError>;

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, msg)
}

fn not_found(what: &str) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, format!("{what} not found"))
}

// ---------------------------------------------------------------------- identity

/// The salted hash identifying a voter's network. Raw addresses are never stored.
fn voter(app: &App, headers: &HeaderMap, addr: SocketAddr) -> String {
    let forwarded = app
        .cfg
        .trust_proxy
        .then(|| headers.get("x-forwarded-for")?.to_str().ok()?.split(',').next().map(|s| s.trim().to_string()))
        .flatten();
    let ip = forwarded.unwrap_or_else(|| addr.ip().to_string());
    let mut data = app.salt.clone();
    data.extend_from_slice(ip.as_bytes());
    db::sha256_hex(&data)
}

fn user(app: &App, headers: &HeaderMap) -> ApiResult<User> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "missing `Authorization: Bearer <token>`"))?;
    app.db.user_by_token(token.trim())?.ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unknown token"))
}

fn admin(app: &App, headers: &HeaderMap) -> ApiResult<User> {
    let u = user(app, headers)?;
    if !u.admin {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "admins only"));
    }
    Ok(u)
}

// ---------------------------------------------------------------------- mods

#[derive(Serialize, Clone)]
pub struct OptionOut {
    key: String,
    #[serde(rename = "type")]
    kind: OptionType,
    default: Value,
    description: String,
}

#[derive(Serialize, Clone)]
pub struct ModOut {
    id: String,
    name: String,
    section: Section,
    description: String,
    authors: Vec<String>,
    version: String,
    status: String,
    uploaded_by: String,
    updated: i64,
    score: i64,
    my_vote: i64,
    icon: Option<String>,
    gifs: Vec<String>,
    stats: Value,
    requires: Vec<String>,
    conflicts: Vec<String>,
    game_builds: Vec<String>,
    options: Vec<OptionOut>,
    package: String,
    size: i64,
    sha256: String,
}

/// The newest version that is not rejected, per mod.
fn latest(versions: Vec<Version>) -> BTreeMap<String, Version> {
    let mut out: BTreeMap<String, Version> = BTreeMap::new();
    for v in versions.into_iter().filter(|v| v.status != "rejected") {
        match out.get(&v.mod_id) {
            Some(cur) if store::version_key(&cur.version) >= store::version_key(&v.version) => {}
            _ => {
                out.insert(v.mod_id.clone(), v);
            }
        }
    }
    out
}

fn mod_out(app: &App, v: &Version, scores: &HashMap<String, i64>, mine: &HashMap<String, i64>) -> ApiResult<ModOut> {
    let m = ModManifest::parse(&v.manifest)?;
    let media = |f: &str| app.url(&format!("/media/{}/{}/{f}", v.mod_id, v.version));
    let icon = v.media.iter().find(|f| f.as_str() == "icon.png").map(|f| media(f));
    // mod.toml's `showcase` order first, then the rest alphabetically (media is stored sorted)
    let mut gifs: Vec<&String> = v.media.iter().filter(|f| f.ends_with(".gif")).collect();
    let rank = |f: &String| m.info.showcase.iter().position(|s| s == f).unwrap_or(usize::MAX);
    gifs.sort_by_key(|f| rank(f));
    let gifs = gifs.into_iter().map(|f| media(f)).collect();
    let options = m
        .options
        .iter()
        .map(|(k, o)| OptionOut {
            key: k.clone(),
            kind: o.kind,
            default: serde_json::to_value(&o.default).unwrap_or(Value::Null),
            description: o.description.clone(),
        })
        .collect();
    Ok(ModOut {
        id: v.mod_id.clone(),
        name: m.info.name,
        section: m.info.section,
        description: m.info.description,
        authors: m.info.authors,
        version: v.version.clone(),
        status: v.status.clone(),
        uploaded_by: v.uploaded_by.clone(),
        updated: v.created,
        score: scores.get(&v.mod_id).copied().unwrap_or(0),
        my_vote: mine.get(&v.mod_id).copied().unwrap_or(0),
        icon,
        gifs,
        stats: serde_json::to_value(&m.stats).unwrap_or(Value::Null),
        requires: m.info.requires,
        conflicts: m.info.conflicts,
        game_builds: m.info.game_builds,
        options,
        package: app.url(&format!("/api/mods/{}/{}/package", v.mod_id, v.version)),
        size: v.size,
        sha256: v.sha256.clone(),
    })
}

#[derive(Deserialize)]
struct ListQuery {
    section: Option<String>,
    q: Option<String>,
    sort: Option<String>,
}

async fn list_mods(
    State(app): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let scores = app.db.scores()?;
    let mine = app.db.votes_of(&voter(&app, &headers, addr))?;
    let needle = q.q.as_deref().unwrap_or("").trim().to_lowercase();
    let mut mods = Vec::new();
    for v in latest(app.db.all_versions()?).values() {
        let m = mod_out(&app, v, &scores, &mine)?;
        let section = serde_json::to_value(m.section)?.as_str().unwrap_or_default().to_string();
        match q.section.as_deref() {
            Some(s) if s != section => continue,
            // Without a section: the four website sections (core and dev mods only on request).
            None if !SECTIONS.contains(&section.as_str()) => continue,
            _ => {}
        }
        if !needle.is_empty()
            && ![&m.id, &m.name, &m.description].iter().any(|t| t.to_lowercase().contains(&needle))
        {
            continue;
        }
        mods.push(m);
    }
    match q.sort.as_deref().unwrap_or("top") {
        "new" => mods.sort_by(|a, b| b.updated.cmp(&a.updated).then(a.name.cmp(&b.name))),
        "name" => mods.sort_by_key(|m| m.name.to_lowercase()),
        "top" => mods.sort_by(|a, b| b.score.cmp(&a.score).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))),
        other => return Err(bad(format!("sort must be top, new or name, not `{other}`"))),
    }
    let mut counts: BTreeMap<&str, usize> = SECTIONS.iter().map(|s| (*s, 0)).collect();
    for v in latest(app.db.all_versions()?).values() {
        if let Ok(m) = ModManifest::parse(&v.manifest) {
            if let Some(c) = serde_json::to_value(m.info.section)?.as_str().and_then(|s| counts.get_mut(s)) {
                *c += 1;
            }
        }
    }
    Ok(Json(json!({ "mods": mods, "counts": counts, "game_build": app.cfg.game_build })))
}

async fn mod_detail(
    State(app): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let versions = app.db.versions_of(&id)?;
    let history: Vec<Value> = versions
        .iter()
        .filter(|v| v.status != "rejected")
        .map(|v| json!({ "version": v.version, "status": v.status, "created": v.created, "size": v.size }))
        .collect();
    let v = latest(versions).remove(&id).ok_or_else(|| not_found("mod"))?;
    let scores = app.db.scores()?;
    let mine = app.db.votes_of(&voter(&app, &headers, addr))?;
    let mut out = serde_json::to_value(mod_out(&app, &v, &scores, &mine)?)?;
    out["versions"] = Value::Array(history);
    Ok(Json(out))
}

async fn package(State(app): State<Shared>, Path((id, version)): Path<(String, String)>) -> ApiResult<Response> {
    let v = app.db.versions_of(&id)?.into_iter().find(|v| v.version == version && v.status != "rejected");
    let v = v.ok_or_else(|| not_found("version"))?;
    let bytes = tokio::fs::read(app.store.package_path(&v.mod_id, &v.version)).await.map_err(|e| anyhow::anyhow!(e))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{id}-{version}.zip\"")),
        ],
        bytes,
    )
        .into_response())
}

async fn media(State(app): State<Shared>, Path((id, version, file)): Path<(String, String, String)>) -> ApiResult<Response> {
    let safe = |s: &str| !s.is_empty() && !s.starts_with('.') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    if !(safe(&id) && safe(&version) && safe(&file)) {
        return Err(not_found("file"));
    }
    let mime = match file.rsplit('.').next() {
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        _ => return Err(not_found("file")),
    };
    let bytes = tokio::fs::read(app.store.media_path(&id, &version, &file)).await.map_err(|_| not_found("file"))?;
    Ok((
        [(header::CONTENT_TYPE, HeaderValue::from_static(mime)), (header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=86400"))],
        bytes,
    )
        .into_response())
}

// ---------------------------------------------------------------------- uploads

/// Store an uploaded package. `status` for admin uploads and imports is "reviewed".
pub fn add_package(app: &App, bytes: &[u8], uploader: &User) -> ApiResult<Version> {
    let pkg = store::inspect(bytes).map_err(|e| bad(format!("{e:#}")))?;
    let info = &pkg.manifest.info;
    match info.section {
        Section::Dev => return Err(bad("dev mods (test fixtures) are not published")),
        Section::Core if !uploader.admin => return Err(ApiError::new(StatusCode::FORBIDDEN, "only admins publish core mods")),
        _ => {}
    }
    if let Some(owner) = app.db.mod_owner(&info.id)? {
        if owner != uploader.id && !uploader.admin {
            return Err(ApiError::new(StatusCode::FORBIDDEN, format!("`{}` belongs to another user", info.id)));
        }
    }
    if app.db.versions_of(&info.id)?.iter().any(|v| v.version == info.version) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("{} {} exists already; bump the version in mod.toml", info.id, info.version),
        ));
    }
    let v = Version {
        mod_id: info.id.clone(),
        version: info.version.clone(),
        manifest: pkg.manifest_text.clone(),
        sha256: db::sha256_hex(bytes),
        size: bytes.len() as i64,
        media: pkg.media.iter().map(|(n, _)| n.clone()).collect(),
        status: if uploader.admin { "reviewed" } else { "unreviewed" }.into(),
        uploaded_by: uploader.name.clone(),
        created: db::now(),
    };
    app.store.save(&pkg, bytes)?;
    app.db.insert_version(&v, uploader.id)?;
    Ok(v)
}

async fn upload(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = user(&app, &headers)?;
    let v = add_package(&app, &body, &u)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": v.mod_id, "version": v.version, "status": v.status,
            "url": app.url(&format!("/mods/{}", v.mod_id)),
            "package": app.url(&format!("/api/mods/{}/{}/package", v.mod_id, v.version)),
        })),
    ))
}

#[derive(Deserialize)]
struct ReviewIn {
    status: String,
}

async fn review(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((id, version)): Path<(String, String)>,
    Json(r): Json<ReviewIn>,
) -> ApiResult<Json<Value>> {
    admin(&app, &headers)?;
    if !db::STATUSES.contains(&r.status.as_str()) {
        return Err(bad(format!("status must be one of {}", db::STATUSES.join(", "))));
    }
    if !app.db.set_status(&id, &version, &r.status)? {
        return Err(not_found("version"));
    }
    Ok(Json(json!({ "id": id, "version": version, "status": r.status })))
}

async fn review_queue(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    admin(&app, &headers)?;
    let pending: Vec<Value> = app
        .db
        .versions_by_status("unreviewed")?
        .into_iter()
        .map(|v| {
            json!({
                "id": v.mod_id, "version": v.version, "uploaded_by": v.uploaded_by, "created": v.created,
                "sha256": v.sha256, "size": v.size, "manifest": v.manifest,
                "package": app.url(&format!("/api/mods/{}/{}/package", v.mod_id, v.version)),
            })
        })
        .collect();
    Ok(Json(json!({ "pending": pending })))
}

async fn me(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let u = user(&app, &headers)?;
    let uploads: Vec<Value> = app
        .db
        .versions_by_uploader(u.id)?
        .into_iter()
        .map(|v| {
            let name = ModManifest::parse(&v.manifest).map(|m| m.info.name).unwrap_or_else(|_| v.mod_id.clone());
            json!({ "id": v.mod_id, "name": name, "version": v.version, "status": v.status, "created": v.created })
        })
        .collect();
    Ok(Json(json!({ "name": u.name, "admin": u.admin, "uploads": uploads })))
}

async fn new_token(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let u = user(&app, &headers)?;
    let token = app.db.rotate_token(&u.name)?.ok_or_else(|| not_found("user"))?;
    Ok(Json(json!({ "name": u.name, "token": token })))
}

// ---------------------------------------------------------------------- votes

#[derive(Deserialize)]
struct VoteIn {
    /// 1 (up), -1 (down) or 0 (take the vote back).
    value: i64,
}

async fn vote(
    State(app): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(v): Json<VoteIn>,
) -> ApiResult<Json<Value>> {
    if ![-1, 0, 1].contains(&v.value) {
        return Err(bad("value must be 1, -1 or 0"));
    }
    if app.db.mod_owner(&id)?.is_none() {
        return Err(not_found("mod"));
    }
    app.db.vote(&id, &voter(&app, &headers, addr), v.value)?;
    let score = app.db.scores()?.get(&id).copied().unwrap_or(0);
    Ok(Json(json!({ "id": id, "score": score, "my_vote": v.value })))
}

// ---------------------------------------------------------------------- packs

#[derive(Deserialize)]
pub struct PackIn {
    pub mods: Vec<PackModIn>,
    #[serde(default)]
    pub section_requests: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub struct PackModIn {
    pub id: String,
    pub version: Option<String>,
    #[serde(default)]
    pub options: toml::Table,
    pub request: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct PackOut {
    pub openlina: u32,
    pub id: String,
    pub game_build: String,
    pub url: String,
    pub zip_url: String,
    pub created: i64,
    pub mods: Vec<PackModOut>,
    pub section_requests: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
pub struct PackModOut {
    pub id: String,
    pub name: String,
    pub section: Section,
    pub version: String,
    pub status: String,
    pub package: String,
    /// sha256 of the package zip.
    #[serde(default)]
    pub sha256: String,
    pub options: toml::Table,
    pub request: Option<String>,
    /// Added because these mods require it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_by: Vec<String>,
}

const MAX_REQUEST: usize = 2000;

fn clean_request(r: Option<String>) -> ApiResult<Option<String>> {
    let r = r.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if r.as_ref().is_some_and(|s| s.chars().count() > MAX_REQUEST) {
        return Err(bad(format!("change requests are limited to {MAX_REQUEST} characters")));
    }
    Ok(r)
}

async fn create_pack(State(app): State<Shared>, Json(input): Json<PackIn>) -> ApiResult<(StatusCode, Json<PackOut>)> {
    if input.mods.is_empty() || input.mods.len() > 100 {
        return Err(bad("a pack holds 1 to 100 mods"));
    }
    let all = app.db.all_versions()?;
    let newest = latest(all.clone());
    let mut out: Vec<PackModOut> = Vec::new();
    let mut manifests: BTreeMap<String, ModManifest> = BTreeMap::new();

    let mut r = Resolver { app: &app, all: &all, newest: &newest, out: &mut out, manifests: &mut manifests };
    for m in input.mods {
        r.add(&m.id, m.version.as_deref(), m.options, clean_request(m.request)?, None)?;
    }
    // Requirements, recursively (newest versions).
    loop {
        let missing: Vec<(String, String)> = r
            .manifests
            .values()
            .flat_map(|m| m.info.requires.iter().map(|r| (r.clone(), m.info.id.clone())))
            .filter(|(req, _)| !r.manifests.contains_key(req))
            .collect();
        if missing.is_empty() {
            break;
        }
        for (req, by) in missing {
            r.add(&req, None, toml::Table::new(), None, Some(&by))
                .map_err(|_| bad(format!("`{by}` requires `{req}`, which is not on this site")))?;
        }
    }
    for m in &mut out {
        m.required_by.sort();
        m.required_by.dedup();
    }
    for m in manifests.values() {
        if let Some(c) = m.info.conflicts.iter().find(|c| manifests.contains_key(*c)) {
            return Err(bad(format!("`{}` conflicts with `{c}`; remove one of them", m.info.id)));
        }
    }
    let mut section_requests = BTreeMap::new();
    for (k, v) in input.section_requests {
        if !SECTIONS.contains(&k.as_str()) {
            return Err(bad(format!("unknown section `{k}`")));
        }
        if let Some(r) = clean_request(Some(v))? {
            section_requests.insert(k, r);
        }
    }
    let id = db::random_hex(5);
    let pack = PackOut {
        openlina: 1,
        game_build: app.cfg.game_build.clone(),
        url: app.url(&format!("/api/packs/{id}")),
        zip_url: app.url(&format!("/api/packs/{id}/zip")),
        created: db::now(),
        mods: out,
        section_requests,
        id,
    };
    app.db.insert_pack(&pack.id, &serde_json::to_string(&pack)?)?;
    Ok((StatusCode::CREATED, Json(pack)))
}

/// Resolves the mods of a new pack: versions, options, requirements.
struct Resolver<'a> {
    app: &'a App,
    all: &'a [Version],
    newest: &'a BTreeMap<String, Version>,
    out: &'a mut Vec<PackModOut>,
    manifests: &'a mut BTreeMap<String, ModManifest>,
}

impl Resolver<'_> {
    fn add(&mut self, id: &str, version: Option<&str>, options: toml::Table, request: Option<String>, by: Option<&str>) -> ApiResult<()> {
        if let Some(existing) = self.out.iter_mut().find(|m| m.id == id) {
            return match by {
                Some(by) => {
                    existing.required_by.push(by.to_string());
                    Ok(())
                }
                None => Err(bad(format!("`{id}` is listed twice"))),
            };
        }
        let v = match version {
            Some(ver) => self.all.iter().find(|v| v.mod_id == id && v.version == ver && v.status != "rejected"),
            None => self.newest.get(id),
        }
        .ok_or_else(|| bad(format!("no mod `{id}`{}", version.map(|v| format!(" version {v}")).unwrap_or_default())))?;
        let m = ModManifest::parse(&v.manifest)?;
        let resolved = m.resolve_options(&options).map_err(|e| bad(format!("{e:#}")))?;
        // Keep only the user's overrides in the pack (defaults come from mod.toml).
        let options: toml::Table =
            resolved.into_iter().filter(|(k, val)| m.options.get(k).is_some_and(|o| &o.default != val)).collect();
        self.out.push(PackModOut {
            id: id.to_string(),
            name: m.info.name.clone(),
            section: m.info.section,
            version: v.version.clone(),
            status: v.status.clone(),
            package: self.app.url(&format!("/api/mods/{id}/{}/package", v.version)),
            sha256: v.sha256.clone(),
            options,
            request,
            required_by: by.map(|b| vec![b.to_string()]).unwrap_or_default(),
        });
        self.manifests.insert(id.to_string(), m);
        Ok(())
    }
}

fn load_pack(app: &App, id: &str) -> ApiResult<PackOut> {
    let body = app.db.pack(id)?.ok_or_else(|| not_found("pack"))?;
    Ok(serde_json::from_str(&body).map_err(|e| anyhow::anyhow!(e))?)
}

async fn get_pack(State(app): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<PackOut>> {
    Ok(Json(load_pack(&app, &id)?))
}

async fn pack_zip(State(app): State<Shared>, Path(id): Path<String>) -> ApiResult<Response> {
    let pack = load_pack(&app, &id)?;
    let modpack = ModPack {
        openlina: 1,
        game_build: Some(pack.game_build.clone()),
        mods: pack
            .mods
            .iter()
            .map(|m| PackEntry {
                id: m.id.clone(),
                version: Some(m.version.clone()),
                options: m.options.clone(),
                request: m.request.clone(),
                url: Some(m.package.clone()),
                status: Some(m.status.clone()),
            })
            .collect(),
        section_requests: pack.section_requests.clone(),
    };
    let helpers = app.store.helpers();
    let run = if helpers.is_empty() {
        "Get the `openlina` helper from the openlina-kit repository (cargo build --release -p openlina), then run\n\n    openlina install openlina-pack.zip".to_string()
    } else {
        "Unzip anywhere, then run\n\n    ./openlina install .      (Windows: openlina.exe install .)".to_string()
    };
    let requests: BTreeSet<&str> = pack.mods.iter().filter_map(|m| m.request.as_deref()).collect();
    let readme = format!(
        "OpenLina pack {id} for Mosa Lina (Steam build {build})\n\n{run}\n\nIt prints a Steam launch option (Mosa Lina > Properties > Launch Options).\nClear it to play vanilla again. No game files are in this zip: the helper patches\nyour own copy of the game when it starts and never changes the install.\n\nMods:\n{mods}\n{req}Pack: {url}\n",
        build = pack.game_build,
        mods = pack.mods.iter().map(|m| format!("  {} {} ({})\n", m.id, m.version, m.status)).collect::<String>(),
        req = if requests.is_empty() && pack.section_requests.is_empty() {
            String::new()
        } else {
            "This pack has change requests (see modpack.toml). Give it to an agent with openlina-kit:\n    lina pull <pack url>\n\n".into()
        },
        url = pack.url,
    );
    let mods: Vec<(String, String)> = pack.mods.iter().map(|m| (m.id.clone(), m.version.clone())).collect();
    let bytes = store::pack_zip(&app.store, &modpack, &mods, &readme)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"openlina-pack-{id}.zip\"")),
        ],
        bytes,
    )
        .into_response())
}

async fn openapi(State(app): State<Shared>) -> Json<Value> {
    Json(crate::openapi::spec(&app.cfg.public_url))
}
