//! API tests against an in-memory database and a temp data directory.

use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use openlina_web::api::{App, Config, Shared};
use serde_json::{json, Value};
use tower::ServiceExt;

struct T {
    app: Shared,
    _dir: tempfile::TempDir,
    admin: String,
    alice: String,
    bob: String,
}

fn setup() -> T {
    let dir = tempfile::tempdir().unwrap();
    let store = openlina_web::store::Store::new(dir.path()).unwrap();
    let db = openlina_web::db::Db::open_memory().unwrap();
    let cfg = Config { public_url: "http://hub.test".into(), game_build: "22056877".into(), trust_proxy: false };
    let app = Arc::new(App::new(db, store, cfg).unwrap());
    let admin = app.db.add_user("admin", true).unwrap();
    let alice = app.db.add_user("alice", false).unwrap();
    let bob = app.db.add_user("bob", false).unwrap();
    T { app, _dir: dir, admin, alice, bob }
}

impl T {
    fn router(&self, ip: [u8; 4]) -> Router {
        openlina_web::router(self.app.clone()).layer(MockConnectInfo(SocketAddr::from((ip, 1234))))
    }

    async fn call(&self, req: Request<Body>) -> (StatusCode, Vec<u8>) {
        self.call_from([10, 0, 0, 1], req).await
    }

    async fn call_from(&self, ip: [u8; 4], req: Request<Body>) -> (StatusCode, Vec<u8>) {
        let res = self.router(ip).oneshot(req).await.unwrap();
        let status = res.status();
        (status, res.into_body().collect().await.unwrap().to_bytes().to_vec())
    }

    async fn json(&self, req: Request<Body>) -> (StatusCode, Value) {
        let (s, b) = self.call(req).await;
        (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    async fn upload(&self, token: &str, zip: Vec<u8>) -> (StatusCode, Value) {
        self.json(Request::post("/api/mods").header("authorization", format!("Bearer {token}")).body(Body::from(zip)).unwrap()).await
    }
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

fn post_json(uri: &str, v: Value) -> Request<Body> {
    Request::post(uri).header("content-type", "application/json").body(Body::from(v.to_string())).unwrap()
}

/// A package zip like `lina pack` writes, with a fake (but well-formed header) wasm patch.
fn package(id: &str, version: &str, section: &str, extra: &str) -> Vec<u8> {
    package_files(id, &[
        ("mod.toml", format!(
            "[mod]\nid = \"{id}\"\nname = \"{id} name\"\nversion = \"{version}\"\nsection = \"{section}\"\ndescription = \"test mod\"\n{extra}\n\
             [options.power]\ntype = \"int\"\ndefault = 1\ndescription = \"how strong\"\n\n[stats]\nammo = 3\n"
        ).into_bytes()),
        ("patch.wasm", b"\0asm\x01\0\0\0".to_vec()),
        ("media/icon.png", b"\x89PNG fake".to_vec()),
        ("media/show.gif", b"GIF89a fake".to_vec()),
        ("assets/images/x.png", b"\x89PNG".to_vec()),
    ])
}

fn package_files(prefix: &str, files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in files {
        let path = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
        z.start_file(path, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

#[tokio::test]
async fn upload_list_and_detail() {
    let t = setup();
    let (s, _) = t.json(Request::post("/api/mods").body(Body::from(package("zap", "0.1.0", "items", ""))).unwrap()).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, v) = t.upload(&t.alice, package("zap", "0.1.0", "items", "")).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["status"], "unreviewed");

    let (s, v) = t.json(get("/api/mods?section=items")).await;
    assert_eq!(s, StatusCode::OK);
    let m = &v["mods"][0];
    assert_eq!(m["id"], "zap");
    assert_eq!(m["status"], "unreviewed");
    assert_eq!(m["icon"], "http://hub.test/media/zap/0.1.0/icon.png");
    assert_eq!(m["gifs"][0], "http://hub.test/media/zap/0.1.0/show.gif");
    assert_eq!(m["stats"]["ammo"], 3);
    assert_eq!(v["counts"]["items"], 1);
    // other sections and searches
    let (_, v) = t.json(get("/api/mods?section=levels")).await;
    assert_eq!(v["mods"].as_array().unwrap().len(), 0);
    let (_, v) = t.json(get("/api/mods?q=ZAP")).await;
    assert_eq!(v["mods"].as_array().unwrap().len(), 1);

    let (s, v) = t.json(get("/api/mods/zap")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["versions"][0]["version"], "0.1.0");
    assert_eq!(v["options"][0]["key"], "power");

    let (s, body) = t.call(get("/media/zap/0.1.0/show.gif")).await;
    assert_eq!((s, &body[..6]), (StatusCode::OK, &b"GIF89a"[..]));
    let (s, _) = t.call(get("/media/zap/0.1.0/..%2Fx.gif")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, body) = t.call(get("/api/mods/zap/0.1.0/package")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body, package("zap", "0.1.0", "items", ""));
}

#[tokio::test]
async fn upload_rules() {
    let t = setup();
    assert_eq!(t.upload(&t.alice, package("zap", "0.1.0", "items", "")).await.0, StatusCode::CREATED);
    // same version again, someone else's mod, core and dev mods
    assert_eq!(t.upload(&t.alice, package("zap", "0.1.0", "items", "")).await.0, StatusCode::CONFLICT);
    assert_eq!(t.upload(&t.bob, package("zap", "0.2.0", "items", "")).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.upload(&t.alice, package("zap", "0.2.0", "items", "")).await.0, StatusCode::CREATED);
    assert_eq!(t.upload(&t.alice, package("core", "1.0.0", "core", "")).await.0, StatusCode::FORBIDDEN);
    assert_eq!(t.upload(&t.alice, package("fixture", "1.0.0", "dev", "")).await.0, StatusCode::BAD_REQUEST);
    // broken packages
    let no_wasm = package_files("x", &[("mod.toml", b"[mod]\nid=\"x\"\nname=\"x\"\nversion=\"1.0.0\"\nsection=\"items\"\ndescription=\"\"".to_vec())]);
    let (s, v) = t.upload(&t.alice, no_wasm).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("patch.wasm"), "{v}");
    let wrong_folder = package_files("y", &[("mod.toml", b"[mod]\nid=\"x\"\nname=\"x\"\nversion=\"1.0.0\"\nsection=\"items\"\ndescription=\"\"".to_vec()), ("patch.wasm", b"\0asm".to_vec())]);
    assert_eq!(t.upload(&t.alice, wrong_folder).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.upload(&t.alice, b"not a zip".to_vec()).await.0, StatusCode::BAD_REQUEST);
    let bad_version = package("v", "1.0", "items", "");
    assert_eq!(t.upload(&t.alice, bad_version).await.0, StatusCode::BAD_REQUEST);
    // the newest version is listed
    let (_, v) = t.json(get("/api/mods/zap")).await;
    assert_eq!(v["version"], "0.2.0");
    // uploads show up under /api/me
    let (_, v) = t.json(Request::get("/api/me").header("authorization", format!("Bearer {}", t.alice)).body(Body::empty()).unwrap()).await;
    assert_eq!(v["uploads"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn votes_are_per_network() {
    let t = setup();
    t.upload(&t.admin, package("zap", "0.1.0", "items", "")).await;
    let vote = |v: i64| post_json("/api/mods/zap/vote", json!({ "value": v }));
    let (s, b) = t.call_from([1, 1, 1, 1], vote(1)).await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
    t.call_from([1, 1, 1, 1], vote(1)).await; // same network: still one vote
    t.call_from([2, 2, 2, 2], vote(1)).await;
    t.call_from([3, 3, 3, 3], vote(-1)).await;
    let (_, b) = t.call_from([1, 1, 1, 1], get("/api/mods/zap")).await;
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!((v["score"].as_i64(), v["my_vote"].as_i64()), (Some(1), Some(1)));
    t.call_from([1, 1, 1, 1], vote(0)).await; // take it back
    let (_, v) = t.json(get("/api/mods/zap")).await;
    assert_eq!(v["score"], 0);
    assert_eq!(t.call(vote(5)).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(t.call(post_json("/api/mods/nope/vote", json!({ "value": 1 }))).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn review_hides_rejected() {
    let t = setup();
    t.upload(&t.alice, package("zap", "0.1.0", "items", "")).await;
    let review = |token: &str, status: &str| {
        Request::post("/api/mods/zap/0.1.0/review")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(json!({ "status": status }).to_string()))
            .unwrap()
    };
    assert_eq!(t.call(review(&t.alice, "reviewed")).await.0, StatusCode::FORBIDDEN);
    let (_, q) = t.json(Request::get("/api/review").header("authorization", format!("Bearer {}", t.admin)).body(Body::empty()).unwrap()).await;
    assert_eq!(q["pending"][0]["id"], "zap");
    assert_eq!(t.call(review(&t.admin, "rejected")).await.0, StatusCode::OK);
    let (_, v) = t.json(get("/api/mods?section=items")).await;
    assert_eq!(v["mods"].as_array().unwrap().len(), 0);
    assert_eq!(t.call(get("/api/mods/zap/0.1.0/package")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(t.call(review(&t.admin, "reviewed")).await.0, StatusCode::OK);
    let (_, v) = t.json(get("/api/mods/zap")).await;
    assert_eq!(v["status"], "reviewed");
}

#[tokio::test]
async fn packs_resolve_requirements_and_export_zips() {
    let t = setup();
    t.upload(&t.admin, package("core", "0.4.0", "core", "")).await;
    t.upload(&t.admin, package("zap", "0.1.0", "items", "requires = [\"core\"]")).await;
    t.upload(&t.admin, package("wrap", "0.1.0", "modifiers", "requires = [\"core\"]\nconflicts = [\"edges\"]")).await;
    t.upload(&t.admin, package("edges", "0.1.0", "modifiers", "requires = [\"core\"]")).await;

    // A conflict doesn't block the pack; it is listed for an agent to resolve.
    let (s, v) = t.json(post_json("/api/packs", json!({ "mods": [{ "id": "wrap" }, { "id": "edges" }] }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["conflicts"], json!([["edges", "wrap"]]));
    let (_, zip) = t.call(get(&format!("/api/packs/{}/zip", v["id"].as_str().unwrap()))).await;
    let mut readme = String::new();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
    std::io::Read::read_to_string(&mut z.by_name("openlina-pack/README.txt").unwrap(), &mut readme).unwrap();
    assert!(readme.contains("can't be installed yet: edges conflicts with wrap"), "{readme}");
    let (s, _) = t.json(post_json("/api/packs", json!({ "mods": [{ "id": "zap", "options": { "nope": 1 } }] }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = t.json(post_json("/api/packs", json!({ "mods": [{ "id": "zap" }], "section_requests": { "bogus": "x" } }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, pack) = t
        .json(post_json(
            "/api/packs",
            json!({
                "mods": [{ "id": "zap", "request": "  3 shots  ", "options": { "power": 2 } }, { "id": "wrap" }],
                "section_requests": { "items": "stronger", "levels": "  " }
            }),
        ))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{pack}");
    let ids: Vec<&str> = pack["mods"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["zap", "wrap", "core"]);
    assert_eq!(pack["mods"][0]["request"], "3 shots");
    assert_eq!(pack["mods"][0]["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(pack["mods"][0]["options"]["power"], 2);
    assert_eq!(pack["mods"][2]["required_by"], json!(["wrap", "zap"]));
    assert_eq!(pack["section_requests"], json!({ "items": "stronger" }));
    let id = pack["id"].as_str().unwrap();
    assert_eq!(pack["url"], format!("http://hub.test/api/packs/{id}"));

    let (s, again) = t.json(get(&format!("/api/packs/{id}"))).await;
    assert_eq!((s, &again), (StatusCode::OK, &pack));

    let (s, zip) = t.call(get(&format!("/api/packs/{id}/zip"))).await;
    assert_eq!(s, StatusCode::OK);
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
    let names: Vec<String> = z.file_names().map(String::from).collect();
    for want in ["openlina-pack/modpack.toml", "openlina-pack/README.txt", "openlina-pack/mods/zap/mod.toml", "openlina-pack/mods/zap/patch.wasm", "openlina-pack/mods/zap/assets/images/x.png", "openlina-pack/mods/core/mod.toml", "openlina-pack/mods/wrap/mod.toml"] {
        assert!(names.iter().any(|n| n == want), "missing {want} in {names:?}");
    }
    let mut modpack = String::new();
    std::io::Read::read_to_string(&mut z.by_name("openlina-pack/modpack.toml").unwrap(), &mut modpack).unwrap();
    let mp: openlina_sdk::manifest::ModPack = toml::from_str(&modpack).unwrap();
    assert_eq!(mp.mods.len(), 3);
    assert_eq!(mp.mods[0].request.as_deref(), Some("3 shots"));
    assert_eq!(mp.mods[0].status.as_deref(), Some("reviewed"));
    assert_eq!(mp.mods[0].options.get("power").and_then(|v| v.as_integer()), Some(2));
    assert_eq!(mp.section_requests.get("items").map(String::as_str), Some("stronger"));
}

#[tokio::test]
async fn pages_and_openapi() {
    let t = setup();
    for p in ["/", "/mods/zap", "/pack", "/agents", "/review", "/static/app.js", "/static/app.css", "/api/openapi.json"] {
        assert_eq!(t.call(get(p)).await.0, StatusCode::OK, "{p}");
    }
    assert_eq!(t.call(get("/static/nope.js")).await.0, StatusCode::NOT_FOUND);
    let (_, spec) = t.json(get("/api/openapi.json")).await;
    assert_eq!(spec["servers"][0]["url"], "http://hub.test");
}

#[tokio::test]
async fn showcase_order() {
    let t = setup();
    let toml = b"[mod]\nid = \"gifs\"\nname = \"Gifs\"\nversion = \"1.0.0\"\nsection = \"items\"\ndescription = \"\"\nshowcase = [\"z.gif\"]\n".to_vec();
    let zip = package_files("gifs", &[
        ("mod.toml", toml),
        ("patch.wasm", b"\0asm".to_vec()),
        ("media/a.gif", b"GIF89a".to_vec()),
        ("media/z.gif", b"GIF89a".to_vec()),
    ]);
    assert_eq!(t.upload(&t.admin, zip).await.0, StatusCode::CREATED);
    let (_, v) = t.json(get("/api/mods/gifs")).await;
    let names: Vec<&str> = v["gifs"].as_array().unwrap().iter().map(|g| g.as_str().unwrap().rsplit('/').next().unwrap()).collect();
    assert_eq!(names, ["z.gif", "a.gif"]);
}
