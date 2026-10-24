//! Local development seeder. Run with `DATABASE_URL` set:
//!
//!   PROBE_FORGE_ALLOW_DEV_SEED=true bun run user:seed
//!
//! It creates or refreshes a single development account from
//! `PROBE_FORGE_DEV_EMAIL`, `PROBE_FORGE_DEV_DISPLAY_NAME`, and
//! `PROBE_FORGE_DEV_PASSWORD`. It refuses to run unless the operator explicitly
//! opts in with `PROBE_FORGE_ALLOW_DEV_SEED=true`, and it never prints the
//! password. Use the terminal-prompt `create-user` binary for real accounts.
//!
//! This is dev-only tooling: it is not exposed over HTTP, it takes the password
//! from the environment (never arguments or logs), and it only touches the one
//! configured account.

use std::{env, error::Error, io};

use probe_forge_api::{
    Config,
    auth::password::{hash_password, normalize_email, validate_password},
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

const DEFAULT_EMAIL: &str = "dev@probeforge.local";
const DEFAULT_DISPLAY_NAME: &str = "Dev User";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let opted_in = matches!(
        env::var("PROBE_FORGE_ALLOW_DEV_SEED").as_deref(),
        Ok("true") | Ok("1")
    );
    if !opted_in {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to seed a development account without PROBE_FORGE_ALLOW_DEV_SEED=true",
        )
        .into());
    }

    let email_raw = env::var("PROBE_FORGE_DEV_EMAIL").unwrap_or_else(|_| DEFAULT_EMAIL.to_owned());
    let display_name = env::var("PROBE_FORGE_DEV_DISPLAY_NAME")
        .unwrap_or_else(|_| DEFAULT_DISPLAY_NAME.to_owned());
    let password = env::var("PROBE_FORGE_DEV_PASSWORD").map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "PROBE_FORGE_DEV_PASSWORD must be set",
        )
    })?;

    let email = normalize_email(&email_raw).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "email must be a valid ASCII address",
        )
    })?;
    let display_name = display_name.trim();
    if display_name.is_empty()
        || display_name.chars().count() > 120
        || display_name.chars().any(char::is_control)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "display name must be between 1 and 120 characters",
        )
        .into());
    }
    validate_password(&password)
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;

    let password_hash =
        hash_password(&password).map_err(|_| io::Error::other("password hashing failed"))?;

    let config = Config::from_env()?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&config.database_url)
        .await
        .map_err(|_| {
            io::Error::other(
                "database connection failed; verify DATABASE_URL and server availability",
            )
        })?;

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash, status)
         VALUES ($1, $2, $3, 'active')
         ON CONFLICT (email) DO UPDATE SET
             display_name = EXCLUDED.display_name,
             password_hash = EXCLUDED.password_hash,
             status = 'active'
         RETURNING id",
    )
    .bind(&email)
    .bind(display_name)
    .bind(password_hash)
    .fetch_one(&pool)
    .await
    .map_err(|_| io::Error::other("development user seeding failed"))?;

    println!("Dev user ready: {user_id} ({email}).");
    Ok(())
}
