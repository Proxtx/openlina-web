# openlina-web

OpenLina, the mod hub for Mosa Lina. People browse mods in four sections (items, modifiers, level mods,
general), vote, collect a pack with change requests and export it as a **zip** for players (`openlina install`)
or as **JSON** for agents (`lina pull`). Agents upload mods with a token; uploads wait in a review queue.

One Rust binary (axum + SQLite, pages compiled in). It uses the mod formats of
[openlina-kit](https://github.com/Proxtx/openlina-kit) (`openlina_sdk::manifest`) through a path dependency, so check out both repos
side by side:

```
mosa-mod/
  openlina-kit/
  openlina-web/
```

## Run locally

Needs a Rust toolchain (rustup; `nix develop` works too, then binaries land in `target/nix/` instead of
`target/`).

```bash
cargo build --release
B=target/release/openlina-web
$B user-add admin --admin                # prints the admin's upload token (shown once)
(cd ../openlina-kit && ./lina pack)      # package zips of the kit's mods in ../openlina-kit/dist/
$B import ../openlina-kit/dist/core-0.5.0.zip ../openlina-kit/dist/portal-gun-0.2.0.zip …
mkdir -p data/helpers && cp ../openlina-kit/target/release/openlina data/helpers/   # helper inside pack zips
$B serve                                 # http://127.0.0.1:8080
```

## Kit versions

The site runs the openlina-kit version it was built with (`openlina_sdk::kit::KIT_VERSION`, shown on the
agents page and in `GET /api/mods` as `kit`); the helpers in `data/helpers/` must come from the same kit
checkout (or its GitHub release). Every mod's `mod.toml` names the kit it was made with (`kit`, none = 0.1.0).
Versions within a line (`0.1.x`) work together; a new line (`0.2`) means mods made for older lines need an
agent to port them:

- uploads made with a newer kit than the site are refused: update the site first (pull the kit, rebuild,
  replace the helpers);
- mods made for an older line show NEEDS AN AGENT (`kit_issue: "port"`);
- a pack with such a mod, conflicting mods or a declared option conflict (`[[conflict]]`) has
  `needs_agent: true`: its zip is refused (409 with the reasons), its JSON goes to an agent (`lina pull`).

Everything lives in `--data` (default `./data`, git-ignored):

| path | |
|---|---|
| `openlina.db` | users (token hashes), mods, versions, votes, packs |
| `packages/<id>/<version>.zip` | uploaded packages, byte for byte |
| `media/<id>/<version>/` | `icon.png` and gifs from the package's `media/`, shown on the site |
| `helpers/` | optional `openlina` helper builds (`openlina`, `openlina.exe`) added to pack zips |

## Commands

| command | |
|---|---|
| `serve [--addr 127.0.0.1:8080] [--trust-proxy]` | run the site; `--trust-proxy` takes voter addresses from `X-Forwarded-For` (only behind your own proxy) |
| `user-add <name> [--admin]` | create a user, print the token |
| `user-token <name>` | replace a user's token |
| `users` | list users |
| `import <zip>… [--user <name>]` | add packages directly (default user: the first admin) |
| `review [<id> <version> <status>]` | list unreviewed uploads, or set `reviewed` / `rejected` / `unreviewed` |

Global: `--data <dir>` (`OPENLINA_DATA`), `--public-url <url>` (`OPENLINA_PUBLIC_URL`, used in pack JSON and
API links), `--game-build <build>`.

## Rules

- **Packages** are what `lina pack <id>` writes: `<id>/mod.toml`, `<id>/patch.wasm`, `assets/`, `media/`. Uploads
  are checked: safe paths, size limits (32 MB zip, 96 MB unpacked, 12 MB per media file), a valid `mod.toml`, a
  WebAssembly `patch.wasm`, a `1.2.3` version, the folder named after the id. A version can't be replaced (bump it).
- **Ownership**: the first uploader of an id owns it; only they (or an admin) upload new versions. `core`
  mods need an admin, `dev` mods (test fixtures) are refused.
- **Review**: uploads by non-admins are `unreviewed` (shown with a badge) until an admin sets `reviewed` or
  `rejected` (hidden, not downloadable). Admin uploads are `reviewed`. Queue: `/review` or `GET /api/review`.
- **Votes**: one per mod per network: the key is `sha256(secret salt + client IP)`; raw addresses are never
  stored. Voting again with the same value is sent as `0` (take it back).
- **Packs**: `POST /api/packs` pins versions, adds required mods (`required_by`), checks options against
  `mod.toml`, lists what needs an agent (see "Kit versions"), and stores the pack under a short id. `GET /api/packs/<id>` (JSON) and
  `/api/packs/<id>/zip` (for players; `openlina-pack/` with `modpack.toml`, `mods/<id>/`, helpers, README).
  Change requests (per mod and per section, up to 2000 characters) travel in both.
- **No game files** are ever stored or served: players' helpers patch their own copy of the game.

API reference: `/api/openapi.json` (also summarized on the `/agents` page).

## Pages

`/` browse (sections, search, top/new, votes, pack sidebar) · `/mods/<id>` detail (gifs, stats, options, change
request, versions) · `/pack` export (zip or JSON) · `/agents` (kit, token, API, your uploads) · `/review` (admins).
Plain HTML/CSS/JS in `static/`, compiled into the binary; the pack being assembled and the token live in the
browser's `localStorage`. Mod data is always inserted as text, never as HTML.

The approved design prototype is in `design/` (Claude Design canvas).

## Tests

`cargo test --release`: uploads and their rules, listing/search/detail/media, votes per network, review,
pack resolution and the exported zip, kit versions and option conflicts (`tests/api.rs`).

## Deploy (rhost / VPS)

Build with `cargo build --release`, copy `target/release/openlina-web` to the server, run it behind a TLS
reverse proxy with `--trust-proxy` and `--public-url https://your.host`. Example unit: `deploy/openlina-web.service`.
Back up the data directory (SQLite in WAL mode: stop the service or use `sqlite3 openlina.db .backup`).
