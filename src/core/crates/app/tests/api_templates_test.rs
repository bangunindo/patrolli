// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

mod setup;

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use setup::api::{Harness, TestUser, send};
use uuid::Uuid;

async fn create(h: &Harness, user: &TestUser, kind: &str, name: &str) -> Uuid {
    let (status, body) = send(
        h.request(Method::POST, &format!("/api/v1/{kind}s"), user)
            .json(&json!({"name": name, "adm_status": 1, "labels": {}})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    serde_json::from_value(body["data"][format!("{kind}_id")].clone()).unwrap()
}

async fn link(h: &Harness, source: Uuid, target: Uuid) {
    h.pool
        .get()
        .await
        .unwrap()
        .execute(
            "INSERT INTO template_links (template_link_id, template_id, link_to_template_id)
             VALUES (patrolli_uuidv7(), $1, $2)",
            &[&source, &target],
        )
        .await
        .unwrap();
}

async fn apply(
    h: &Harness,
    user: &TestUser,
    hosts: &[Uuid],
    templates: &[Uuid],
) -> (StatusCode, Value) {
    send(
        h.request(Method::POST, "/api/v1/templates/apply", user)
            .json(&json!({"host_ids": hosts, "template_ids": templates})),
    )
    .await
}

async fn applied_templates(h: &Harness, host: Uuid) -> Vec<Uuid> {
    h.pool
        .get()
        .await
        .unwrap()
        .query(
            "SELECT template_id FROM host_templates
             WHERE host_id = $1 AND template_id IS NOT NULL ORDER BY template_id",
            &[&host],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

#[tokio::test]
async fn rejects_selected_templates_connected_through_unselected_inheritance() {
    let h = Harness::start().await;
    let user = h.user("alice").await;
    let host = create(&h, &user, "host", "fresh").await;
    let a = create(&h, &user, "template", "first branch").await;
    let b = create(&h, &user, "template", "second branch").await;
    let parent = create(&h, &user, "template", "shared parent").await;
    let unrelated = create(&h, &user, "template", "unrelated").await;
    link(&h, a, parent).await;
    link(&h, b, parent).await;
    let (status, body) = apply(&h, &user, &[host], &[a]).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = apply(&h, &user, &[host], &[b, unrelated, a]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["title"], "Conflicting template inheritance");
    let mut expected = [a, b];
    expected.sort_unstable();
    assert_eq!(
        body["error"]["data"]["conflicting_templates"],
        json!(expected)
    );
    assert_eq!(applied_templates(&h, host).await, [a]);
}

#[tokio::test]
async fn rejects_direct_and_indirect_conflicts_without_applying_to_any_host() {
    let h = Harness::start().await;
    let user = h.user("alice").await;
    let clean = create(&h, &user, "host", "clean host").await;
    let direct = create(&h, &user, "host", "host with direct link").await;
    let indirect = create(&h, &user, "host", "host with inherited link").await;
    let target = create(&h, &user, "template", "selected template").await;
    let sibling = create(&h, &user, "template", "sibling template").await;
    let parent = create(&h, &user, "template", "shared parent").await;
    link(&h, target, parent).await;
    link(&h, sibling, parent).await;
    for (host, template) in [(direct, target), (indirect, sibling)] {
        let (status, body) = apply(&h, &user, &[host], &[template]).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let (status, body) = apply(&h, &user, &[clean, direct, indirect], &[target]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["title"], "Conflicting template application");
    let detail = body["error"]["detail"].as_str().unwrap();
    assert!(
        detail.contains("host with direct link") && detail.contains("host with inherited link"),
        "{body}"
    );
    assert!(!detail.contains("clean host"), "{body}");
    assert!(applied_templates(&h, clean).await.is_empty());
    assert_eq!(applied_templates(&h, direct).await, [target]);
    assert_eq!(applied_templates(&h, indirect).await, [sibling]);
}

#[tokio::test]
async fn detects_conflicts_through_shared_hosts_in_a_graph_with_multiple_paths() {
    let h = Harness::start().await;
    let user = h.user("alice").await;
    let first_bridge = create(&h, &user, "host", "first bridge").await;
    let second_bridge = create(&h, &user, "host", "second bridge").await;
    let host = create(&h, &user, "host", "target host").await;
    let a = create(&h, &user, "template", "first template").await;
    let b = create(&h, &user, "template", "second template").await;
    let unrelated = create(&h, &user, "template", "unrelated template").await;

    let (status, body) = apply(&h, &user, &[first_bridge, second_bridge], &[a, b]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = apply(&h, &user, &[host], &[a, unrelated]).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = apply(&h, &user, &[host], &[b]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["title"], "Conflicting template application");
    assert!(
        body["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("target host"),
        "{body}"
    );
    let mut expected = [a, unrelated];
    expected.sort_unstable();
    assert_eq!(applied_templates(&h, host).await, expected);
}

#[tokio::test]
async fn distinguishes_host_and_template_with_the_same_uuid() {
    let h = Harness::start().await;
    let user = h.user("alice").await;
    let id = create(&h, &user, "template", "selected template").await;
    let client = h.pool.get().await.unwrap();
    client
        .execute(
            "WITH new_host AS (
                 INSERT INTO hosts (host_id, name, adm_status, last_updated)
                 VALUES ($1, 'matching host', 1, now()) RETURNING host_id
             )
             INSERT INTO host_templates (host_template_id, host_id)
             SELECT patrolli_uuidv7(), host_id FROM new_host",
            &[&id],
        )
        .await
        .unwrap();
    let source_item = create_source_item(&h, id).await;

    let (status, body) = apply(&h, &user, &[id], &[id]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = apply(&h, &user, &[id], &[id]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["title"], "Conflicting template application");
    assert_eq!(applied_templates(&h, id).await, [id]);
    let copied: Uuid = client
        .query_one(
            "SELECT i.template_item_id FROM items i
             JOIN host_templates ht USING (host_template_id)
             WHERE ht.host_id = $1 AND ht.template_id = $1",
            &[&id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(copied, source_item);
}

async fn create_source_item(h: &Harness, template: Uuid) -> Uuid {
    h.pool
        .get()
        .await
        .unwrap()
        .query_one(
            "INSERT INTO items (item_id, host_template_id, type, adm_status,
                                name, schedule, timeout_ms, config, last_updated)
             SELECT patrolli_uuidv7(), host_template_id, 'dummy', 1,
                    'template item', '1s', 1000, '{}'::jsonb, now()
             FROM host_templates WHERE template_id = $1 AND host_id IS NULL
             RETURNING item_id",
            &[&template],
        )
        .await
        .unwrap()
        .get(0)
}

async fn assert_apply_lock_timeout(lock_sql: &str, table: Option<&str>, detail: &str) {
    let h = Harness::start().await;
    let user = h.user("alice").await;
    let host = create(&h, &user, "host", "new host").await;
    let template = create(&h, &user, "template", "selected template").await;
    create_source_item(&h, template).await;
    let mut blocker = h.pool.get().await.unwrap();
    let tx = blocker.transaction().await.unwrap();
    tx.batch_execute(lock_sql).await.unwrap();

    let response = h
        .request(Method::POST, "/api/v1/templates/apply", &user)
        .json(&json!({"host_ids": [host], "template_ids": [template]}))
        .timeout(Duration::from_secs(12))
        .send()
        .await
        .expect("apply did not return a lock timeout response before the client deadline");
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert_eq!(status, StatusCode::LOCKED, "{body}");
    assert_eq!(body["error"]["title"], "Database lock timeout");
    assert!(
        body["error"]["detail"].as_str().unwrap().contains(detail),
        "{body}"
    );
    assert_eq!(body["error"]["data"]["table"].as_str(), table, "{body}");

    tx.rollback().await.unwrap();
    assert!(applied_templates(&h, host).await.is_empty());
    let (status, body) = apply(&h, &user, &[host], &[template]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(applied_templates(&h, host).await, [template]);
}

#[tokio::test]
async fn reports_a_timeout_acquiring_the_host_lock() {
    assert_apply_lock_timeout(
        "SELECT host_id FROM hosts FOR UPDATE",
        Some("hosts"),
        "hosts table",
    )
    .await;
}

#[tokio::test]
async fn reports_a_timeout_acquiring_the_template_lock() {
    assert_apply_lock_timeout(
        "SELECT template_id FROM templates FOR UPDATE",
        Some("templates"),
        "templates table",
    )
    .await;
}

#[tokio::test]
async fn reports_a_timeout_during_bulk_apply_without_guessing_the_table() {
    assert_apply_lock_timeout(
        "SELECT item_id FROM items FOR UPDATE",
        None,
        "applying the templates",
    )
    .await;
}
