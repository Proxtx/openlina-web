use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use openlina_web::api::{self, Config};

#[derive(Parser)]
#[command(name = "openlina-web", about = "OpenLina: the mod hub for Mosa Lina")]
struct Cli {
    /// Data directory (database, packages, media, helpers).
    #[arg(long, global = true, default_value = "data", env = "OPENLINA_DATA")]
    data: PathBuf,
    /// Base URL in exported packs and API responses.
    #[arg(long, global = true, default_value = "http://127.0.0.1:8080", env = "OPENLINA_PUBLIC_URL")]
    public_url: String,
    /// Steam build the mods are made for (shown on the site and in packs).
    #[arg(long, global = true, default_value = "22056877")]
    game_build: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the web server.
    Serve {
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: SocketAddr,
        /// Behind a reverse proxy: take client addresses (for votes) from X-Forwarded-For.
        #[arg(long)]
        trust_proxy: bool,
    },
    /// Create a user and print their upload token.
    UserAdd {
        name: String,
        #[arg(long)]
        admin: bool,
    },
    /// Replace a user's token and print the new one.
    UserToken { name: String },
    /// List users.
    Users,
    /// Add package zips (`lina pack` output) as a user (default: the first admin), bypassing HTTP.
    Import {
        zips: Vec<PathBuf>,
        #[arg(long)]
        user: Option<String>,
    },
    /// List unreviewed uploads, or set a version's status.
    Review { id: Option<String>, version: Option<String>, status: Option<String> },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let trust_proxy = matches!(cli.cmd, Cmd::Serve { trust_proxy: true, .. });
    let cfg = Config { public_url: cli.public_url.clone(), game_build: cli.game_build.clone(), trust_proxy };
    let app = openlina_web::open(&cli.data, cfg)?;
    match cli.cmd {
        Cmd::Serve { addr, .. } => {
            let listener = tokio::net::TcpListener::bind(addr).await.with_context(|| format!("binding {addr}"))?;
            println!("OpenLina on http://{addr} (public URL {}, data {})", cli.public_url, cli.data.display());
            axum::serve(listener, openlina_web::router(app).into_make_service_with_connect_info::<SocketAddr>())
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await?;
        }
        Cmd::UserAdd { name, admin } => {
            let token = app.db.add_user(&name, admin)?;
            println!("{token}");
            eprintln!("user `{name}` created{}; the token is shown only now", if admin { " (admin)" } else { "" });
        }
        Cmd::UserToken { name } => match app.db.rotate_token(&name)? {
            Some(t) => println!("{t}"),
            None => bail!("no user `{name}`"),
        },
        Cmd::Users => {
            for u in app.db.users()? {
                println!("{}{}", u.name, if u.admin { " (admin)" } else { "" });
            }
        }
        Cmd::Import { zips, user } => {
            let users = app.db.users()?;
            let u = match &user {
                Some(n) => users.into_iter().find(|u| &u.name == n).with_context(|| format!("no user `{n}`"))?,
                None => users.into_iter().find(|u| u.admin).context("no admin user; create one with `user-add <name> --admin`")?,
            };
            for z in zips {
                let bytes = std::fs::read(&z).with_context(|| format!("reading {}", z.display()))?;
                match api::add_package(&app, &bytes, &u) {
                    Ok(v) => println!("imported {} {} ({})", v.mod_id, v.version, v.status),
                    Err(e) => eprintln!("{}: {}", z.display(), e.message()),
                }
            }
        }
        Cmd::Review { id, version, status } => match (id, version, status) {
            (None, _, _) => {
                for v in app.db.versions_by_status("unreviewed")? {
                    println!("{} {} by {} (sha256 {})", v.mod_id, v.version, v.uploaded_by, v.sha256);
                }
            }
            (Some(id), Some(version), Some(status)) => {
                anyhow::ensure!(openlina_web::db::STATUSES.contains(&status.as_str()), "status: unreviewed, reviewed or rejected");
                anyhow::ensure!(app.db.set_status(&id, &version, &status)?, "no {id} {version}");
                println!("{id} {version}: {status}");
            }
            _ => bail!("usage: review [<id> <version> <status>]"),
        },
    }
    Ok(())
}
