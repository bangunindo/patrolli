// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

mod setup;

use anyhow::{Context, Result};
use deadpool_postgres::Pool;
use patrolli::config::sensitive::{SecretKeys, Sensitive, SensitiveFields};
use patrolli::config::{endpoint, listener, sink};
use patrolli::db;
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

const OLD_KEY: &str = "old:YWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWE=";
const NEW_KEY: &str = "new:YmJiYmJiYmJiYmJiYmJiYmJiYmJiYmJiYmJiYmJiYmI=";
const GONE_KEY: &str = "gone:Y2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2M=";
const OLD_ID_WRONG_BYTES: &str = "old:Y2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2M=";

fn keys(keys: &str, active: &str) -> SecretKeys {
    SecretKeys::parse(Some(keys), Some(active)).expect("invalid test keys")
}

fn stored<C: SensitiveFields>(kind: &str, config: Value, keys: &SecretKeys) -> Value {
    let mut config: C = serde_json::from_value(json!({ "type": kind, "config": config }))
        .expect("invalid test config");
    config.encrypt_all(keys).expect("encrypt test config");
    serde_json::to_value(&config).unwrap()["config"].take()
}

fn opened(config: &Value, keys: &SecretKeys) -> Result<Value> {
    Ok(match config {
        Value::String(s) if s.starts_with("enc:v1:") => {
            let mut value = Sensitive::Cipher(s.clone());
            value.decrypt(keys)?;
            Value::String(value.expose()?.to_owned())
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| opened(item, keys))
                .collect::<Result<_>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), opened(v, keys)?)))
                .collect::<Result<_>>()?,
        ),
        other => other.clone(),
    })
}

async fn configs(pool: &Pool) -> Result<HashMap<Uuid, Value>> {
    let client = pool.get().await.context("failed to get connection")?;
    let rows = client
        .query(
            "SELECT sink_id, config FROM sinks \
             UNION ALL SELECT listener_id, config FROM listeners \
             UNION ALL SELECT endpoint_id, config FROM host_endpoints",
            &[],
        )
        .await
        .context("failed to query configs")?;
    rows.iter()
        .map(|row| -> Result<(Uuid, Value)> { Ok((row.try_get(0)?, row.try_get(1)?)) })
        .collect()
}

struct Seeded {
    before: HashMap<Uuid, Value>,
    good: Vec<Uuid>,
    mixed: Uuid,
    wrong_key: Uuid,
}

async fn seed(pool: &Pool) -> Result<Seeded> {
    let old = keys(OLD_KEY, "old");
    let gone = keys(GONE_KEY, "gone");
    let clickhouse = json!({
        "endpoints": ["http://192.0.2.1:8123"],
        "password": "clickhouse-pass",
        "tls": { "key_password": "clickhouse-tls-pass" },
    });
    let mut mixed = stored::<sink::Config>("clickhouse", clickhouse.clone(), &old);
    mixed["tls"] = stored::<sink::Config>("clickhouse", clickhouse.clone(), &gone)["tls"].take();
    let sinks = [
        (
            "clickhouse",
            stored::<sink::Config>("clickhouse", clickhouse, &old),
        ),
        (
            "kafka",
            stored::<sink::Config>(
                "kafka",
                json!({
                    "hosts": ["192.0.2.1:9092"],
                    "topic": "t",
                    "format": "proto_otlp",
                    "security": { "protocol": "sasl_ssl" },
                    "sasl": { "mechanism": "plain", "username": "u", "password": "kafka-pass" },
                }),
                &old,
            ),
        ),
        ("clickhouse", mixed),
    ];
    let listener = stored::<listener::Config>(
        "snmp_trap",
        json!({ "v3": { "security_models": [{ "type": "usm", "config": {
            "username": "trap-user",
            "authentication": { "protocol": "sha256", "passphrase": "trap-auth-pass" },
            "privacy": { "protocol": "aes128", "passphrase": "trap-priv-pass" },
        } }] } }),
        &old,
    );
    let endpoints = [
        stored::<endpoint::Config>(
            "http",
            json!({
                "url": "http://192.0.2.1/metrics",
                "auth": { "Basic": { "username": "u", "password": "http-pass" } },
            }),
            &old,
        ),
        stored::<endpoint::Config>(
            "http",
            json!({ "url": "http://192.0.2.1/metrics", "auth": { "Bearer": "http-token" } }),
            &keys(OLD_ID_WRONG_BYTES, "old"),
        ),
    ];

    let mut client = pool.get().await.context("failed to get connection")?;
    let tx = client.transaction().await?;
    let mut sink_ids = Vec::new();
    for (kind, config) in &sinks {
        let id = Uuid::now_v7();
        tx.execute(
            "INSERT INTO sinks (sink_id, name, type, adm_status, last_updated, config) \
             VALUES ($1, $2, $3, 1, now(), $4)",
            &[&id, &id.to_string(), kind, config],
        )
        .await
        .context("failed to insert sink")?;
        sink_ids.push(id);
    }
    let listener_id = Uuid::now_v7();
    tx.execute(
        "INSERT INTO listeners (listener_id, name, type, adm_status, last_updated, config) \
         VALUES ($1, 'trap', 'snmp_trap', 1, now(), $2)",
        &[&listener_id, &listener],
    )
    .await
    .context("failed to insert listener")?;
    let host_id = setup::create_host(&tx, "host").await?;
    let mut endpoint_ids = Vec::new();
    for config in &endpoints {
        let id = Uuid::now_v7();
        tx.execute(
            "INSERT INTO host_endpoints (endpoint_id, host_id, name, type, adm_status, last_updated, config) \
             VALUES ($1, $2, $3, 'http', 1, now(), $4)",
            &[&id, &host_id, &id.to_string(), config],
        )
        .await
        .context("failed to insert endpoint")?;
        endpoint_ids.push(id);
    }
    tx.commit().await?;

    Ok(Seeded {
        before: configs(pool).await?,
        good: vec![sink_ids[0], sink_ids[1], listener_id, endpoint_ids[0]],
        mixed: sink_ids[2],
        wrong_key: endpoint_ids[1],
    })
}

fn assert_rotated(seeded: &Seeded, after: &HashMap<Uuid, Value>) {
    let old = keys(OLD_KEY, "old");
    let gone = keys(GONE_KEY, "gone");
    let new = keys(NEW_KEY, "new");
    for id in &seeded.good {
        assert_eq!(
            opened(&after[id], &new).unwrap(),
            opened(&seeded.before[id], &old).unwrap(),
            "row {id}"
        );
    }

    let mixed = &after[&seeded.mixed];
    assert_eq!(mixed["tls"], seeded.before[&seeded.mixed]["tls"]);
    assert_eq!(
        opened(&mixed["tls"], &gone).unwrap()["key_password"],
        "clickhouse-tls-pass"
    );
    assert_eq!(opened(&mixed["password"], &new).unwrap(), "clickhouse-pass");

    assert_eq!(after[&seeded.wrong_key], seeded.before[&seeded.wrong_key]);
}

#[tokio::test]
async fn reencrypt_moves_secrets_to_the_active_key_once() -> Result<()> {
    setup::setup_logging();
    let (_db, pool) = setup::setup_db().await;
    let seeded = seed(&pool).await?;
    let rotated = keys(&format!("{OLD_KEY},{NEW_KEY}"), "new");

    db::reencrypt_secrets(&pool, &rotated).await?;
    let after = configs(&pool).await?;
    assert_rotated(&seeded, &after);

    db::reencrypt_secrets(&pool, &rotated).await?;
    assert_eq!(configs(&pool).await?, after);
    Ok(())
}

#[tokio::test]
async fn concurrent_reencrypts_agree() -> Result<()> {
    setup::setup_logging();
    let (_db, pool) = setup::setup_db().await;
    let seeded = seed(&pool).await?;
    let rotated = keys(&format!("{OLD_KEY},{NEW_KEY}"), "new");

    tokio::try_join!(
        db::reencrypt_secrets(&pool, &rotated),
        db::reencrypt_secrets(&pool, &rotated),
    )?;
    assert_rotated(&seeded, &configs(&pool).await?);
    Ok(())
}
