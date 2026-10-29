//! `relix-server`: one binary, one SQLite file. Configured by environment variables:
//!
//! - `RELIX_DB` database file (default `relix-server.db`)
//! - `RELIX_ADDR` listen address (default `0.0.0.0:8787`)
//! - `RELIX_SMTP_URL` e.g. `smtps://user:pass@smtp.example.com`; enables emailed verification codes
//! - `RELIX_SMTP_FROM` sender, e.g. `Relix <no-reply@example.com>`
//! - `RELIX_VERIFY_EMAIL` `true`/`false`; defaults to `true` when SMTP is set. Without SMTP, codes are printed.
//! - `RELIX_ADMIN_TOKEN` enables the `/admin` endpoints and the `admin` command (see `relix-server admin`).

use relix_proto::addon::{Addon, Manifest};
use relix_proto::sync::{AddedAdminAddon, AdminAddon};
use relix_server::cli::{AdminCommand, USAGE, parse_admin};
use relix_server::{Config, Mailer, app};

/// Turns admin API failures into messages an operator can act on.
fn checked(res: reqwest::Response) -> Result<reqwest::Response, Box<dyn std::error::Error>> {
    match res.status() {
        reqwest::StatusCode::UNAUTHORIZED => Err("the server rejected RELIX_ADMIN_TOKEN (or has no admin token set)".into()),
        reqwest::StatusCode::NOT_FOUND => Err("no published addon with that id; see `list`".into()),
        s if !s.is_success() => Err(format!("server error {s}").into()),
        _ => Ok(res),
    }
}

/// `relix-server admin ...`: talks to a running server's admin API.
async fn admin(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let cmd = parse_admin(args)?;
    let token = std::env::var("RELIX_ADMIN_TOKEN").map_err(|_| format!("set RELIX_ADMIN_TOKEN
{USAGE}"))?;
    let server = std::env::var("RELIX_SERVER_URL").unwrap_or_else(|_| "http://127.0.0.1:8787".into());
    let server = server.trim_end_matches('/');
    let http = reqwest::Client::new();
    match cmd {
        AdminCommand::Add(url) => {
            // Check it's a real addon before every account receives it.
            let manifest: Manifest = http.get(url.clone()).send().await?.error_for_status()?.json().await
                .map_err(|e| format!("{url} didn't return an addon manifest: {e}"))?;
            let name = manifest.name.clone();
            let added: AddedAdminAddon = http
                .post(format!("{server}/admin/addons"))
                .bearer_auth(&token)
                .json(&Addon { manifest_url: url, manifest })
                .send()
                .await
                .map(checked)??
                .json()
                .await?;
            println!("published {name} as #{}", added.id);
        }
        AdminCommand::List => {
            let list: Vec<AdminAddon> =
                checked(http.get(format!("{server}/admin/addons")).bearer_auth(&token).send().await?)?.json().await?;
            if list.is_empty() {
                println!("no published addons");
            }
            for a in list {
                println!("#{}  {}  {}", a.id, a.addon.manifest.name, a.addon.manifest_url);
            }
        }
        AdminCommand::Remove(id) => {
            checked(http.delete(format!("{server}/admin/addons/{id}")).bearer_auth(&token).send().await?)?;
            println!("removed #{id}");
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("admin") {
        if let Err(e) = admin(&args[1..]).await {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mailer = match (env("RELIX_SMTP_URL"), env("RELIX_SMTP_FROM")) {
        (Some(url), Some(from)) => Mailer::Smtp {
            transport: Box::new(lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::from_url(&url)?.build()),
            from: from.parse()?,
        },
        (Some(_), None) => return Err("RELIX_SMTP_FROM is required with RELIX_SMTP_URL".into()),
        _ => Mailer::Log,
    };
    let verify_email = match env("RELIX_VERIFY_EMAIL").as_deref() {
        Some("true") => true,
        Some("false") => false,
        Some(other) => return Err(format!("RELIX_VERIFY_EMAIL must be true or false, not {other:?}").into()),
        None => matches!(mailer, Mailer::Smtp { .. }),
    };

    let db = rusqlite::Connection::open(env("RELIX_DB").unwrap_or_else(|| "relix-server.db".into()))?;
    let addr = env("RELIX_ADDR").unwrap_or_else(|| "0.0.0.0:8787".into());
    let router = app(db, Config { verify_email, mailer, admin_token: env("RELIX_ADMIN_TOKEN") })?;
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("relix-server listening on {addr} (email verification {})", if verify_email { "on" } else { "off" });
    axum::serve(listener, router).await?;
    Ok(())
}
