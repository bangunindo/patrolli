// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod models;
pub mod query;
pub mod schema;

use crate::config::sensitive::{SecretKeys, SensitiveFields, header};
use crate::config::{endpoint, listener, sink};
use anyhow::{Context, Result};
use deadpool_postgres::{ClientWrapper, Object, Pool};
use refinery::embed_migrations;
use tracing::{info, warn};
use uuid::Uuid;

embed_migrations!("migrations");

#[repr(u8)]
pub enum AdvisoryLock {
    Migration = 1,
    Leader = 2,
    NodeLock = 3,
    StandbyLock = 4,
    SecretReencrypt = 5,
}

const NOT_ON_ACTIVE_KEY: &str = r#"strpos(replace(config::text, $1, ''), '"enc:v1:') > 0"#;

pub async fn run_migrations(pool: &Pool) -> Result<()> {
    info!("connecting to database");
    let mut client = Object::take(pool.get().await.context("failed to get db connection")?);
    client
        .execute("SELECT 1", &[])
        .await
        .context("failed to execute test query")?;

    // blocks while another node holds the lock and is migrating
    info!("waiting for migration lock");
    client
        .execute(
            "SELECT pg_advisory_lock($1)",
            &[&(AdvisoryLock::Migration as i64)],
        )
        .await
        .context("failed to acquire migration lock")?;

    info!("running database migrations");
    let report = migrations::runner()
        .run_async(&mut *client)
        .await
        .context("failed to run migrations")?;

    // Populate users table with username and password from environment variables if it is empty
    let user_count: i64 = client
        .query_one("SELECT COUNT(*) FROM users", &[])
        .await
        .context("failed to query users table")?
        .get(0);

    if user_count == 0 {
        let username = std::env::var("PATROLLI_ADMIN_USER");
        let password = std::env::var("PATROLLI_ADMIN_PASSWORD");
        if let (Ok(username), Ok(password)) = (username, password) {
            let allow_sso: bool = match std::env::var("PATROLLI_ADMIN_ALLOW_SSO") {
                Ok(value) => value
                    .parse()
                    .context("PATROLLI_ADMIN_ALLOW_SSO must be true or false")?,
                Err(_) => false,
            };
            let created = client
                .execute(
                    "INSERT INTO users (user_id, username, password_hash, allow_sso) VALUES ($1, $2, crypt($3, gen_salt('bf', 12)), $4) ON CONFLICT (username) DO NOTHING",
                    &[&Uuid::now_v7(), &username, &password, &allow_sso],
                )
                .await
                .context("failed to create default admin user")?;
            info!(user = %username, allow_sso, created = created == 1, "default admin user seeded");
        } else {
            // nobody can log in to the API until a user exists
            warn!(
                vars = "PATROLLI_ADMIN_USER,PATROLLI_ADMIN_PASSWORD",
                "users table is empty and admin env vars are not set, skipping default user creation"
            );
        }
    }

    client
        .execute(
            "SELECT pg_advisory_unlock($1)",
            &[&(AdvisoryLock::Migration as i64)],
        )
        .await
        .context("failed to release migration lock")?;

    info!(
        applied = report.applied_migrations().len(),
        "migrations completed successfully"
    );
    Ok(())
}

/// Moves every stored secret that is not on the active key onto it. A value
/// that fails is logged and left as it is, so one bad row cannot keep the node
/// from starting.
pub async fn reencrypt_secrets(pool: &Pool, keys: &SecretKeys) -> Result<()> {
    // out of the pool, so an early return closes the session and frees the lock
    let mut client = Object::take(pool.get().await.context("failed to get db connection")?);
    client
        .execute(
            "SELECT pg_advisory_lock($1)",
            &[&(AdvisoryLock::SecretReencrypt as i64)],
        )
        .await
        .context("failed to acquire secret re-encrypt lock")?;
    let active_header = format!("\"{}", header(keys.active_key_id()));
    let reencrypted =
        reencrypt_table::<sink::Config>(&mut client, "sinks", "sink_id", &active_header, keys)
            .await?
            + reencrypt_table::<listener::Config>(
                &mut client,
                "listeners",
                "listener_id",
                &active_header,
                keys,
            )
            .await?
            + reencrypt_table::<endpoint::Config>(
                &mut client,
                "host_endpoints",
                "endpoint_id",
                &active_header,
                keys,
            )
            .await?;
    client
        .execute(
            "SELECT pg_advisory_unlock($1)",
            &[&(AdvisoryLock::SecretReencrypt as i64)],
        )
        .await
        .context("failed to release secret re-encrypt lock")?;

    let remaining: i64 = client
        .query_one(
            &format!(
                "SELECT (SELECT count(*) FROM sinks WHERE {NOT_ON_ACTIVE_KEY}) \
                 + (SELECT count(*) FROM listeners WHERE {NOT_ON_ACTIVE_KEY}) \
                 + (SELECT count(*) FROM host_endpoints WHERE {NOT_ON_ACTIVE_KEY})"
            ),
            &[&active_header],
        )
        .await
        .context("failed to count rows with secrets on other keys")?
        .get(0);
    let active_key_id = keys.active_key_id();
    if remaining > 0 {
        warn!(
            reencrypted,
            remaining, active_key_id, "secret values re-encrypted"
        );
    } else {
        info!(
            reencrypted,
            remaining, active_key_id, "secret values re-encrypted"
        );
    }
    Ok(())
}

async fn reencrypt_table<C: SensitiveFields>(
    client: &mut ClientWrapper,
    table: &str,
    id_column: &str,
    active_header: &str,
    keys: &SecretKeys,
) -> Result<usize> {
    const BATCH: usize = 1000;
    let select = format!(
        "SELECT {id_column}, type, config FROM {table} \
         WHERE {NOT_ON_ACTIVE_KEY} AND {id_column} > $2 \
         ORDER BY {id_column} LIMIT {BATCH} FOR UPDATE"
    );
    let update = format!(
        "UPDATE {table} SET config = rekeyed.config \
         FROM unnest($1::uuid[], $2::jsonb[]) AS rekeyed(id, config) \
         WHERE {table}.{id_column} = rekeyed.id"
    );
    let mut reencrypted = 0;
    // a row that failed still matches, so each batch starts after the last id read
    let mut last_id = Uuid::nil();
    loop {
        let tx = client
            .transaction()
            .await
            .context("failed to start db transaction")?;
        let rows = tx
            .query(&select, &[&active_header, &last_id])
            .await
            .with_context(|| format!("failed to query {table}"))?;
        let (mut ids, mut configs) = (Vec::new(), Vec::new());
        for row in &rows {
            let id: Uuid = row
                .try_get(0)
                .with_context(|| format!("failed to get {id_column}"))?;
            last_id = id;
            let kind: String = row.try_get(1).context("failed to get type")?;
            let config: serde_json::Value = row.try_get(2).context("failed to get config")?;
            let mut config: C = match serde_json::from_value(
                serde_json::json!({ "type": kind, "config": config }),
            ) {
                Ok(config) => config,
                Err(e) => {
                    warn!(table, row_id = %id, error = %e, "secret value could not be re-encrypted");
                    continue;
                }
            };
            let (changed, errors) = config.rekey_all(keys);
            for e in errors {
                warn!(table, row_id = %id, key_id = e.key_id(), error = %e, "secret value could not be re-encrypted");
            }
            // only a plain value fails to serialize, and it is one of the
            // errors logged above; the row is then left as it is
            if changed > 0
                && let Ok(mut value) = serde_json::to_value(&config)
            {
                ids.push(id);
                configs.push(value["config"].take());
                reencrypted += changed;
            }
        }
        if !ids.is_empty() {
            tx.execute(&update, &[&ids, &configs])
                .await
                .with_context(|| format!("failed to update {table}"))?;
        }
        tx.commit()
            .await
            .context("failed to commit db transaction")?;
        if rows.len() < BATCH {
            return Ok(reencrypted);
        }
    }
}
