//! `relix-server`: one binary, one SQLite file. Configured by environment variables:
//!
//! - `RELIX_DB` database file (default `relix-server.db`)
//! - `RELIX_ADDR` listen address (default `0.0.0.0:8787`)
//! - `RELIX_SMTP_URL` e.g. `smtps://user:pass@smtp.example.com`; enables emailed verification codes
//! - `RELIX_SMTP_FROM` sender, e.g. `Relix <no-reply@example.com>`
//! - `RELIX_VERIFY_EMAIL` `true`/`false`; defaults to `true` when SMTP is set. Without SMTP, codes are printed.

use relix_server::{Config, Mailer, app};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let router = app(db, Config { verify_email, mailer })?;
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("relix-server listening on {addr} (email verification {})", if verify_email { "on" } else { "off" });
    axum::serve(listener, router).await?;
    Ok(())
}
