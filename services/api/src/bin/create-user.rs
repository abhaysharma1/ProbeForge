//! Provision a local ProbeForge account. Run with `DATABASE_URL` set:
//!
//!   cargo run -p probe-forge-api --bin create-user -- owner@example.test "Owner"
//!
//! The password is read from the terminal, never from arguments or logs. There
//! is intentionally no public registration endpoint in Phase 3.

use std::{env, error::Error, io};

use probe_forge_api::{
    Config,
    auth::password::{hash_password, normalize_email, validate_password},
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let email_argument = arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: create-user <email> <display name>",
        )
    })?;
    let display_name = arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: create-user <email> <display name>",
        )
    })?;
    if arguments.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: create-user <email> <display name>",
        )
        .into());
    }

    let email = normalize_email(&email_argument).ok_or_else(|| {
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

    let password = rpassword::prompt_password("Password: ")?;
    let confirmation = rpassword::prompt_password("Confirm password: ")?;
    if password != confirmation {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "passwords do not match").into());
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
        "INSERT INTO users (email, display_name, password_hash)
         VALUES ($1, $2, $3)
         RETURNING id",
    )
    .bind(&email)
    .bind(display_name)
    .bind(password_hash)
    .fetch_one(&pool)
    .await
    .map_err(|error| {
        if matches!(
            &error,
            sqlx::Error::Database(database_error)
                if database_error.code().as_deref() == Some("23505")
        ) {
            io::Error::other("a user with that email already exists")
        } else {
            io::Error::other("user creation failed")
        }
    })?;

    println!("Created user {user_id} ({email}).");
    Ok(())
}
