// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

mod setup;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use setup::api::{Harness, TestUser, send};
use uuid::Uuid;

const PATH: &str = "/api/v1/pipelines";

async fn create(h: &Harness, user: &TestUser, body: Value) -> Value {
    let (status, body) = send(h.request(Method::POST, PATH, user).json(&body)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["data"].clone()
}

async fn get(h: &Harness, user: &TestUser, id: &str) -> (StatusCode, Value) {
    send(h.request(Method::GET, &format!("{PATH}/{id}"), user)).await
}

async fn delete(h: &Harness, user: &TestUser, id: &str) -> (StatusCode, Value) {
    send(h.request(Method::DELETE, &format!("{PATH}/{id}"), user)).await
}

async fn list(h: &Harness, user: &TestUser, query: &str) -> Value {
    let (status, body) = send(h.request(Method::GET, &format!("{PATH}?{query}"), user)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"].clone()
}

fn names(list: &Value) -> Vec<&str> {
    list["pipelines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect()
}

async fn sql(h: &Harness, statements: &str) {
    let client = h.pool.get().await.unwrap();
    client.batch_execute(statements).await.unwrap();
}

async fn count(h: &Harness, query: &str) -> i64 {
    let client = h.pool.get().await.unwrap();
    client.query_one(query, &[]).await.unwrap().get(0)
}

async fn uuids(h: &Harness, query: &str) -> Vec<Uuid> {
    let client = h.pool.get().await.unwrap();
    client
        .query(query, &[])
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

async fn insert_host(h: &Harness) -> Uuid {
    let id = Uuid::now_v7();
    sql(
        h,
        &format!(
            "INSERT INTO hosts (host_id, name, adm_status, last_updated) VALUES ('{id}', 'h-{id}', 1, now())"
        ),
    )
    .await;
    id
}

async fn insert_sink(h: &Harness) -> Uuid {
    let id = Uuid::now_v7();
    sql(
        h,
        &format!(
            "INSERT INTO sinks (sink_id, name, type, adm_status, last_updated, config) \
             VALUES ('{id}', 's-{id}', 'file', 1, now(), '{{}}')"
        ),
    )
    .await;
    id
}

/// A new pipeline is private, enabled, owned by its creator and has one empty canvas.
/// Duplicate host/sink ids collapse into one sorted list, and a get returns the same body.
#[tokio::test]
async fn create_returns_the_pipeline_with_one_empty_canvas() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let (host_a, host_b) = (insert_host(&h).await, insert_host(&h).await);
    let sink = insert_sink(&h).await;

    // ids are sent unsorted and with a duplicate
    let created = create(
        &h,
        &alice,
        json!({ "name": "p1", "host_ids": [host_b, host_a, host_b], "sink_ids": [sink] }),
    )
    .await;

    assert_eq!(created["name"], "p1");
    assert_eq!(created["visibility"], 0, "private by default");
    assert_eq!(created["adm_status"], 1, "enabled by default");
    assert_eq!(created["owner_user_id"], json!(alice.user_id));
    assert_eq!(created["host_ids"], json!([host_a, host_b]));
    assert_eq!(created["sink_ids"], json!([sink]));
    assert_eq!(created["created_at"], created["last_updated"]);
    let canvases = created["canvases"].as_array().unwrap();
    assert_eq!(canvases.len(), 1);
    assert_eq!(canvases[0]["last_updated"], created["last_updated"]);

    // get returns the same thing
    let id = created["pipeline_id"].as_str().unwrap();
    let (status, body) = get(&h, &alice, id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"], created);

    // explicit values are kept
    let created = create(
        &h,
        &alice,
        json!({ "name": "p2", "visibility": 1, "adm_status": 0 }),
    )
    .await;
    assert_eq!(created["visibility"], 1);
    assert_eq!(created["adm_status"], 0);
    assert_eq!(created["host_ids"], json!([]));
    assert_eq!(created["sink_ids"], json!([]));
}

/// Bad input (empty name, unknown host or sink, unknown visibility) is rejected with a clear message.
/// None of the rejected requests leave a row behind.
#[tokio::test]
async fn create_rejects_invalid_input_and_writes_nothing() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let host = insert_host(&h).await;
    let missing = Uuid::now_v7();

    for (body, detail) in [
        (json!({ "name": "" }), "Name cannot be empty".to_string()),
        (json!({ "name": "   " }), "Name cannot be empty".to_string()),
        (
            json!({ "name": "p", "host_ids": [host, missing] }),
            format!("Host IDs not found: {missing}"),
        ),
        (
            json!({ "name": "p", "sink_ids": [missing] }),
            format!("Sink IDs not found: {missing}"),
        ),
    ] {
        let (status, response) = send(h.request(Method::POST, PATH, &alice).json(&body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {response}");
        assert_eq!(response["error"]["detail"], detail, "{body}");
    }

    // more than three missing ids are shortened
    let many: Vec<Uuid> = (0..5).map(|_| Uuid::now_v7()).collect();
    let (status, response) = send(
        h.request(Method::POST, PATH, &alice)
            .json(&json!({ "name": "p", "host_ids": many })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        response["error"]["detail"]
            .as_str()
            .unwrap()
            .ends_with("and 2 more"),
        "{response}"
    );

    // unknown visibility is rejected by the json extractor
    let (status, _) = send(
        h.request(Method::POST, PATH, &alice)
            .json(&json!({ "name": "p", "visibility": 5 })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    assert_eq!(count(&h, "SELECT count(*) FROM pipelines").await, 0);
    assert_eq!(count(&h, "SELECT count(*) FROM pipeline_canvases").await, 0);
}

/// The list holds public pipelines and the caller's own private ones, never others' private ones.
/// The name filter, ordering and paging work, and paging does not change the total.
#[tokio::test]
async fn list_shows_public_pipelines_and_own_private_ones() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let bob = h.user("bob").await;
    create(&h, &alice, json!({ "name": "alice private" })).await;
    create(
        &h,
        &alice,
        json!({ "name": "alice public", "visibility": 1 }),
    )
    .await;
    create(&h, &bob, json!({ "name": "bob private" })).await;
    create(&h, &bob, json!({ "name": "bob public", "visibility": 1 })).await;

    let all = list(&h, &alice, "").await;
    assert_eq!(all["total"], 3);
    assert_eq!(names(&all), ["alice private", "alice public", "bob public"]);
    let first = &all["pipelines"][0];
    assert_eq!(first["owner_user_id"], json!(alice.user_id));
    assert_eq!(first["visibility"], 0);
    assert!(first.get("canvases").is_none(), "list rows carry no detail");

    let all = list(&h, &bob, "").await;
    assert_eq!(names(&all), ["alice public", "bob private", "bob public"]);

    // name filter is a case-insensitive substring match
    let filtered = list(&h, &alice, "name=PUB").await;
    assert_eq!(filtered["total"], 2);
    assert_eq!(names(&filtered), ["alice public", "bob public"]);

    // ordering
    let by_created = list(&h, &alice, "order_by=created_at&order_dir=desc").await;
    assert_eq!(
        names(&by_created),
        ["bob public", "alice public", "alice private"]
    );
    let by_name = list(&h, &alice, "order_dir=desc").await;
    assert_eq!(
        names(&by_name),
        ["bob public", "alice public", "alice private"]
    );

    // paging leaves the total alone
    let page = list(&h, &alice, "limit=1&offset=1").await;
    assert_eq!(page["total"], 3);
    assert_eq!(names(&page), ["alice public"]);
}

/// Someone else's private pipeline answers 404, as if it did not exist.
/// Someone else's public pipeline can be read as usual.
#[tokio::test]
async fn private_pipelines_are_hidden_from_other_users() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let bob = h.user("bob").await;
    let private = create(&h, &alice, json!({ "name": "alice private" })).await;
    let public = create(&h, &bob, json!({ "name": "bob public", "visibility": 1 })).await;
    let private_id = private["pipeline_id"].as_str().unwrap();
    let public_id = public["pipeline_id"].as_str().unwrap();

    let (status, body) = get(&h, &bob, private_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["title"], "Pipeline not found");

    let (status, body) = get(&h, &alice, public_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"], public);

    let (status, _) = get(&h, &alice, &Uuid::now_v7().to_string()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Only the owner can delete: someone else's public pipeline is 403, their private one 404.
/// A deleted pipeline is gone together with its canvas.
#[tokio::test]
async fn only_the_owner_can_delete_a_pipeline() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let bob = h.user("bob").await;
    let private = create(&h, &alice, json!({ "name": "alice private" })).await;
    let public = create(&h, &bob, json!({ "name": "bob public", "visibility": 1 })).await;
    let private_id = private["pipeline_id"].as_str().unwrap();
    let public_id = public["pipeline_id"].as_str().unwrap();

    // a private pipeline of someone else looks like it does not exist
    let (status, _) = delete(&h, &bob, private_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // a public pipeline of someone else can be seen but not deleted
    let (status, body) = delete(&h, &alice, public_id).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(count(&h, "SELECT count(*) FROM pipelines").await, 2);

    let (status, _) = delete(&h, &bob, public_id).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = get(&h, &bob, public_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = delete(&h, &bob, public_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = delete(&h, &alice, private_id).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(count(&h, "SELECT count(*) FROM pipelines").await, 0);
    assert_eq!(count(&h, "SELECT count(*) FROM pipeline_canvases").await, 0);
}

/// Deleting the owner leaves the pipeline public with no owner.
/// Such a pipeline can be read and deleted by anyone.
#[tokio::test]
async fn deleting_the_owner_makes_the_pipeline_public_and_deletable_by_anyone() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let bob = h.user("bob").await;
    let private = create(&h, &alice, json!({ "name": "alice private" })).await;
    let id = private["pipeline_id"].as_str().unwrap();

    let (status, _) = send(h.request(
        Method::DELETE,
        &format!("/api/v1/users/{}", alice.user_id),
        &bob,
    ))
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = get(&h, &bob, id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["visibility"], 1);
    assert_eq!(body["data"]["owner_user_id"], Value::Null);
    assert_eq!(names(&list(&h, &bob, "").await), ["alice private"]);

    let (status, _) = delete(&h, &bob, id).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// A deleted host or sink drops out of the pipeline's host_ids / sink_ids.
/// The pipeline itself stays and keeps working.
#[tokio::test]
async fn deleting_hosts_and_sinks_removes_them_from_the_subsets() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let (host_a, host_b) = (insert_host(&h).await, insert_host(&h).await);
    let (sink_a, sink_b) = (insert_sink(&h).await, insert_sink(&h).await);
    let created = create(
        &h,
        &alice,
        json!({ "name": "p", "host_ids": [host_a, host_b], "sink_ids": [sink_a, sink_b] }),
    )
    .await;
    let id = created["pipeline_id"].as_str().unwrap();

    sql(
        &h,
        &format!(
            "DELETE FROM hosts WHERE host_id = '{host_a}'; DELETE FROM sinks WHERE sink_id = '{sink_b}';"
        ),
    )
    .await;

    let (_, body) = get(&h, &alice, id).await;
    assert_eq!(body["data"]["host_ids"], json!([host_b]));
    assert_eq!(body["data"]["sink_ids"], json!([sink_a]));
}

/// Deleting a pipeline removes the processors it created and every runtime row pointing at them.
/// Processors from the old pipelines API, other pipelines, items and sinks are kept.
#[tokio::test]
async fn delete_removes_owned_processors_and_their_runtime_rows() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let bob = h.user("bob").await;
    let host = insert_host(&h).await;
    let sink = insert_sink(&h).await;
    let mine = create(
        &h,
        &alice,
        json!({ "name": "mine", "host_ids": [host], "sink_ids": [sink] }),
    )
    .await;
    let other = create(&h, &bob, json!({ "name": "other", "host_ids": [host] })).await;
    let id = mine["pipeline_id"].as_str().unwrap();
    let canvas = mine["canvases"][0]["canvas_id"].as_str().unwrap();

    // a canvas with a source, a group and a sink, wired the way a save would,
    // plus processors from the old pipelines API that it is connected to
    let (ht, item) = (Uuid::now_v7(), Uuid::now_v7());
    let (source_node, group_node, sink_node) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let (src, gate, member) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let (foreign, foreign_downstream) = (Uuid::now_v7(), Uuid::now_v7());
    sql(&h, &format!("
        INSERT INTO host_templates (host_template_id, host_id) VALUES ('{ht}', '{host}');
        INSERT INTO items (item_id, host_template_id, type, adm_status, name, schedule, timeout_ms, last_updated, config)
            VALUES ('{item}', '{ht}', 'dummy', 1, 'i', '* * * * *', 1000, now(), '{{}}');
        INSERT INTO processors (processor_id, name, type, adm_status, last_updated, config) VALUES
            ('{src}', 'mine (source)', 'null', 1, now(), 'null'),
            ('{gate}', 'mine (gate)', 'null', 1, now(), 'null'),
            ('{member}', 'member', 'null', 1, now(), 'null'),
            ('{foreign}', 'foreign', 'null', 1, now(), 'null'),
            ('{foreign_downstream}', 'foreign downstream', 'null', 1, now(), 'null');
        INSERT INTO pipeline_nodes (node_id, canvas_id, kind, position_x, position_y, last_updated) VALUES
            ('{source_node}', '{canvas}', 'source', 0, 0, now()),
            ('{group_node}', '{canvas}', 'processor_group', 0, 0, now()),
            ('{sink_node}', '{canvas}', 'sink', 0, 0, now());
        INSERT INTO pipeline_node_sources (node_id, source_processor_id, gate_processor_id)
            VALUES ('{source_node}', '{src}', '{gate}');
        INSERT INTO pipeline_node_sinks (node_id, sink_id) VALUES ('{sink_node}', '{sink}');
        INSERT INTO pipeline_node_source_items (node_id, item_id) VALUES ('{source_node}', '{item}');
        INSERT INTO pipeline_node_processor_groups (node_id) VALUES ('{group_node}');
        INSERT INTO pipeline_node_processors (processor_id, node_id, position) VALUES ('{member}', '{group_node}', 0);
        INSERT INTO pipeline_node_relations (canvas_id, from_node_id, to_node_id) VALUES
            ('{canvas}', '{source_node}', '{group_node}'), ('{canvas}', '{group_node}', '{sink_node}');
        INSERT INTO item_processors (item_id, processor_id) VALUES ('{item}', '{src}');
        INSERT INTO processor_inputs (processor_id, input_id) VALUES
            ('{gate}', '{src}'), ('{member}', '{gate}'), ('{foreign}', '{member}'), ('{foreign_downstream}', '{foreign}');
        INSERT INTO sink_inputs (sink_id, input_id) VALUES ('{sink}', '{member}'), ('{sink}', '{foreign}');
    ")).await;

    // the other pipeline's canvas uses the same item; deleting mine must not touch it
    let other_canvas = other["canvases"][0]["canvas_id"].as_str().unwrap();
    let (other_source_node, other_group_node) = (Uuid::now_v7(), Uuid::now_v7());
    let (other_src, other_gate, other_member) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    sql(&h, &format!("
        INSERT INTO processors (processor_id, name, type, adm_status, last_updated, config) VALUES
            ('{other_src}', 'other (source)', 'null', 1, now(), 'null'),
            ('{other_gate}', 'other (gate)', 'null', 1, now(), 'null'),
            ('{other_member}', 'other member', 'null', 1, now(), 'null');
        INSERT INTO pipeline_nodes (node_id, canvas_id, kind, position_x, position_y, last_updated) VALUES
            ('{other_source_node}', '{other_canvas}', 'source', 0, 0, now()),
            ('{other_group_node}', '{other_canvas}', 'processor_group', 0, 0, now());
        INSERT INTO pipeline_node_sources (node_id, source_processor_id, gate_processor_id)
            VALUES ('{other_source_node}', '{other_src}', '{other_gate}');
        INSERT INTO pipeline_node_source_items (node_id, item_id) VALUES ('{other_source_node}', '{item}');
        INSERT INTO pipeline_node_processor_groups (node_id) VALUES ('{other_group_node}');
        INSERT INTO pipeline_node_processors (processor_id, node_id, position) VALUES ('{other_member}', '{other_group_node}', 0);
        INSERT INTO pipeline_node_relations (canvas_id, from_node_id, to_node_id) VALUES
            ('{other_canvas}', '{other_source_node}', '{other_group_node}');
        INSERT INTO item_processors (item_id, processor_id) VALUES ('{item}', '{other_src}');
        INSERT INTO processor_inputs (processor_id, input_id) VALUES
            ('{other_gate}', '{other_src}'), ('{other_member}', '{other_gate}');
    ")).await;

    let (status, body) = delete(&h, &alice, id).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // only the processors the pipeline did not create are left, with the rows
    // between them
    let client = h.pool.get().await.unwrap();
    let processors: Vec<Uuid> = client
        .query(
            "SELECT processor_id FROM processors ORDER BY processor_id",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(
        processors,
        [
            foreign,
            foreign_downstream,
            other_src,
            other_gate,
            other_member
        ]
    );
    let inputs: Vec<(Uuid, Uuid)> = client
        .query(
            "SELECT processor_id, input_id FROM processor_inputs ORDER BY processor_id",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(
        inputs,
        [
            (foreign_downstream, foreign),
            (other_gate, other_src),
            (other_member, other_gate)
        ]
    );
    let sink_inputs: Vec<(Uuid, Uuid)> = client
        .query("SELECT sink_id, input_id FROM sink_inputs", &[])
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(sink_inputs, [(sink, foreign)]);
    let item_processors: Vec<(Uuid, Uuid)> = client
        .query("SELECT item_id, processor_id FROM item_processors", &[])
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(item_processors, [(item, other_src)]);

    // the canvas content is gone; only the other pipeline's rows are left
    for (table, left) in [
        ("pipeline_nodes", 2),
        ("pipeline_node_sources", 1),
        ("pipeline_node_source_items", 1),
        ("pipeline_node_processor_groups", 1),
        ("pipeline_node_processors", 1),
        ("pipeline_node_sinks", 0),
        ("pipeline_node_relations", 1),
        ("pipeline_hosts", 1),
        ("pipeline_sinks", 0),
    ] {
        assert_eq!(
            count(&h, &format!("SELECT count(*) FROM {table}")).await,
            left,
            "{table}"
        );
    }
    let other_id = other["pipeline_id"].as_str().unwrap();
    let (status, body) = get(&h, &bob, other_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"], other);
    assert_eq!(count(&h, "SELECT count(*) FROM items").await, 1);
    assert_eq!(count(&h, "SELECT count(*) FROM sinks").await, 1);
}

fn graph_path(pipeline: &Value) -> String {
    format!(
        "{PATH}/{}/canvases/{}/graph",
        pipeline["pipeline_id"].as_str().unwrap(),
        pipeline["canvases"][0]["canvas_id"].as_str().unwrap()
    )
}

async fn get_graph(h: &Harness, user: &TestUser, path: &str) -> (StatusCode, Value) {
    send(h.request(Method::GET, path, user)).await
}

/// What `seed_canvas` wrote, and the graph it should read back as.
struct SeededCanvas {
    expected: Value,
    source_node: Uuid,
    sink_k2_node: Uuid,
    hidden_processors: [Uuid; 2],
    removable_item: Uuid,
    sink_k2: Uuid,
    /// The source's items: two direct ones, then the one applied from a template.
    items: [Uuid; 3],
}

/// Writes a canvas the way a save would: a source with three items (one of
/// them applied from a template), a group with three processors (one
/// disabled), an empty group, and two sinks, wired as a diamond. Rows go in
/// out of order, so the read has to sort them.
async fn seed_canvas(h: &Harness, pipeline: &Value, host: Uuid) -> SeededCanvas {
    let canvas = pipeline["canvases"][0]["canvas_id"].as_str().unwrap();
    let (sink_k1, sink_k2) = (insert_sink(h).await, insert_sink(h).await);
    // node ids in increasing order: group A, sink k1, source, sink k2, group B
    let [group_a, sink_k1_node, source_node, sink_k2_node, group_b]: [Uuid; 5] =
        std::array::from_fn(|_| Uuid::now_v7());
    let (src, gate) = (Uuid::now_v7(), Uuid::now_v7());
    let [drop_x, rename_a, pass] = std::array::from_fn(|_| Uuid::now_v7());
    let (ht_direct, ht_template_only, ht_applied, template) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    let [template_item, item_1, item_2, applied_item] = std::array::from_fn(|_| Uuid::now_v7());

    sql(h, &format!("
        INSERT INTO templates (template_id, name) VALUES ('{template}', 't');
        INSERT INTO host_templates (host_template_id, host_id) VALUES ('{ht_direct}', '{host}');
        INSERT INTO host_templates (host_template_id, template_id) VALUES ('{ht_template_only}', '{template}');
        INSERT INTO host_templates (host_template_id, host_id, template_id) VALUES ('{ht_applied}', '{host}', '{template}');
        INSERT INTO items (item_id, host_template_id, type, adm_status, name, schedule, timeout_ms, last_updated, config) VALUES
            ('{template_item}', '{ht_template_only}', 'dummy', 1, 'tmpl', '* * * * *', 1000, now(), '{{}}'),
            ('{item_1}', '{ht_direct}', 'dummy', 1, 'i1', '* * * * *', 1000, now(), '{{}}'),
            ('{item_2}', '{ht_direct}', 'dummy', 0, 'i2', '* * * * *', 1000, now(), '{{}}');
        INSERT INTO items (item_id, host_template_id, template_item_id, type, adm_status, last_updated)
            VALUES ('{applied_item}', '{ht_applied}', '{template_item}', 'dummy', 1, now());
        INSERT INTO processors (processor_id, name, type, adm_status, last_updated, config) VALUES
            ('{src}', 'p (source)', 'null', 1, now(), 'null'),
            ('{gate}', 'p (gate)', 'null', 1, now(), 'null'),
            ('{drop_x}', 'drop x', 'drop_attributes', 1, now(), '{{\"keys\": {{\"record\": [\"x\"]}}}}'),
            ('{rename_a}', 'rename a', 'rename_attributes', 0, now(), '{{\"rename_map\": {{\"record\": {{\"a\": \"b\"}}}}}}'),
            ('{pass}', 'pass', 'null', 1, now(), 'null');
        INSERT INTO pipeline_nodes (node_id, canvas_id, kind, position_x, position_y, last_updated) VALUES
            ('{source_node}', '{canvas}', 'source', 0, 0, now()),
            ('{group_a}', '{canvas}', 'processor_group', -120, 35, now()),
            ('{group_b}', '{canvas}', 'processor_group', 200, -7, now()),
            ('{sink_k2_node}', '{canvas}', 'sink', 400, 50, now()),
            ('{sink_k1_node}', '{canvas}', 'sink', 400, -50, now());
        INSERT INTO pipeline_node_sources (node_id, source_processor_id, gate_processor_id)
            VALUES ('{source_node}', '{src}', '{gate}');
        INSERT INTO pipeline_node_processor_groups (node_id) VALUES ('{group_a}'), ('{group_b}');
        INSERT INTO pipeline_node_sinks (node_id, sink_id) VALUES
            ('{sink_k2_node}', '{sink_k2}'), ('{sink_k1_node}', '{sink_k1}');
        INSERT INTO pipeline_node_source_items (node_id, item_id) VALUES
            ('{source_node}', '{applied_item}'), ('{source_node}', '{item_1}'), ('{source_node}', '{item_2}');
        INSERT INTO pipeline_node_processors (processor_id, node_id, position) VALUES
            ('{drop_x}', '{group_a}', 2), ('{rename_a}', '{group_a}', 0), ('{pass}', '{group_a}', 1);
        INSERT INTO pipeline_node_relations (canvas_id, from_node_id, to_node_id) VALUES
            ('{canvas}', '{group_b}', '{sink_k1_node}'), ('{canvas}', '{source_node}', '{group_b}'),
            ('{canvas}', '{group_a}', '{sink_k2_node}'), ('{canvas}', '{source_node}', '{group_a}'),
            ('{canvas}', '{group_a}', '{sink_k1_node}');
    ")).await;

    let expected = json!({
        "nodes": [
            { "node_id": group_a, "position_x": -120, "position_y": 35, "kind": "processor_group",
              "data": { "processors": [
                  { "processor_id": rename_a, "name": "rename a", "type": "rename_attributes", "adm_status": 0 },
                  { "processor_id": pass, "name": "pass", "type": "null", "adm_status": 1 },
                  { "processor_id": drop_x, "name": "drop x", "type": "drop_attributes", "adm_status": 1 },
              ] } },
            { "node_id": sink_k1_node, "position_x": 400, "position_y": -50, "kind": "sink",
              "data": { "sink_id": sink_k1 } },
            { "node_id": source_node, "position_x": 0, "position_y": 0, "kind": "source",
              "data": { "item_count": 3 } },
            { "node_id": sink_k2_node, "position_x": 400, "position_y": 50, "kind": "sink",
              "data": { "sink_id": sink_k2 } },
            { "node_id": group_b, "position_x": 200, "position_y": -7, "kind": "processor_group",
              "data": { "processors": [] } },
        ],
        "relations": [
            { "from_node_id": group_a, "to_node_id": sink_k1_node },
            { "from_node_id": group_a, "to_node_id": sink_k2_node },
            { "from_node_id": source_node, "to_node_id": group_a },
            { "from_node_id": source_node, "to_node_id": group_b },
            { "from_node_id": group_b, "to_node_id": sink_k1_node },
        ],
    });

    SeededCanvas {
        expected,
        source_node,
        sink_k2_node,
        hidden_processors: [src, gate],
        removable_item: item_2,
        sink_k2,
        items: [item_1, item_2, applied_item],
    }
}

/// A canvas reads back exactly: nodes by id, processors in run order, relations by their ends.
/// Disabled processors keep their type, applied template items count, source/gate stay hidden.
#[tokio::test]
async fn graph_reads_the_canvas_structure_in_a_stable_order() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let host = insert_host(&h).await;
    let pipeline = create(&h, &alice, json!({ "name": "p", "host_ids": [host] })).await;
    let seeded = seed_canvas(&h, &pipeline, host).await;

    let (status, body) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let data = &body["data"];
    assert_eq!(data["pipeline_id"], pipeline["pipeline_id"]);
    assert_eq!(data["canvas_id"], pipeline["canvases"][0]["canvas_id"]);
    assert_eq!(
        data["last_updated"],
        pipeline["canvases"][0]["last_updated"]
    );
    assert_eq!(data["graph"], seeded.expected);
    for id in seeded.hidden_processors {
        assert!(!body.to_string().contains(&id.to_string()), "{id} leaked");
    }

    let (_, again) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    assert_eq!(again, body, "two reads differ");
}

/// A new pipeline's canvas reads as an empty graph.
/// Its last_updated is the one the create response gave for the canvas.
#[tokio::test]
async fn new_canvas_has_an_empty_graph() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let pipeline = create(&h, &alice, json!({ "name": "p" })).await;

    let (status, body) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["data"]["graph"],
        json!({ "nodes": [], "relations": [] })
    );
    assert_eq!(
        body["data"]["last_updated"],
        pipeline["canvases"][0]["last_updated"]
    );
}

/// A graph holds only its own canvas, even when another canvas has content.
/// Anyone can read a public pipeline's graph.
#[tokio::test]
async fn graph_is_scoped_to_its_canvas() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let bob = h.user("bob").await;
    let host = insert_host(&h).await;
    let with_content = create(&h, &bob, json!({ "name": "with content", "visibility": 1 })).await;
    let seeded = seed_canvas(&h, &with_content, host).await;
    let empty = create(&h, &alice, json!({ "name": "empty" })).await;

    let (_, body) = get_graph(&h, &alice, &graph_path(&empty)).await;
    assert_eq!(
        body["data"]["graph"],
        json!({ "nodes": [], "relations": [] })
    );

    let (status, body) = get_graph(&h, &alice, &graph_path(&with_content)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["graph"], seeded.expected);
}

/// The graph is 404 when the pipeline is unknown or hidden, or the canvas is not the pipeline's.
/// The error title says which of the two was not found.
#[tokio::test]
async fn graph_not_found_cases() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let bob = h.user("bob").await;
    let alice_a = create(&h, &alice, json!({ "name": "a" })).await;
    let alice_b = create(&h, &alice, json!({ "name": "b" })).await;
    let bob_public = create(&h, &bob, json!({ "name": "c", "visibility": 1 })).await;
    let id = |p: &Value| p["pipeline_id"].as_str().unwrap().to_string();
    let canvas = |p: &Value| p["canvases"][0]["canvas_id"].as_str().unwrap().to_string();
    let path = |p: String, c: String| format!("{PATH}/{p}/canvases/{c}/graph");
    let random = || Uuid::now_v7().to_string();

    for (case, user, path, title) in [
        (
            "unknown pipeline",
            &alice,
            path(random(), canvas(&alice_a)),
            "Pipeline not found",
        ),
        (
            "someone else's private pipeline",
            &bob,
            path(id(&alice_a), canvas(&alice_a)),
            "Pipeline not found",
        ),
        (
            "unknown canvas",
            &alice,
            path(id(&alice_a), random()),
            "Canvas not found",
        ),
        (
            "canvas of another own pipeline",
            &alice,
            path(id(&alice_a), canvas(&alice_b)),
            "Canvas not found",
        ),
        (
            "own canvas under a public pipeline",
            &alice,
            path(id(&bob_public), canvas(&alice_a)),
            "Canvas not found",
        ),
        (
            "public canvas under an own pipeline",
            &alice,
            path(id(&alice_a), canvas(&bob_public)),
            "Canvas not found",
        ),
    ] {
        let (status, body) = get_graph(&h, user, &path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{case}: {body}");
        assert_eq!(body["error"]["title"], title, "{case}");
    }
}

/// Deleting an item lowers the source's item_count.
/// Deleting a sink leaves its node empty; the node and the relations into it stay.
#[tokio::test]
async fn graph_follows_item_and_sink_deletion() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let host = insert_host(&h).await;
    let pipeline = create(&h, &alice, json!({ "name": "p", "host_ids": [host] })).await;
    let seeded = seed_canvas(&h, &pipeline, host).await;

    sql(
        &h,
        &format!(
            "DELETE FROM items WHERE item_id = '{}'; DELETE FROM sinks WHERE sink_id = '{}';",
            seeded.removable_item, seeded.sink_k2
        ),
    )
    .await;

    let (_, body) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    let graph = &body["data"]["graph"];
    let mut expected = seeded.expected.clone();
    for node in expected["nodes"].as_array_mut().unwrap() {
        if node["node_id"] == json!(seeded.source_node) {
            node["data"]["item_count"] = json!(2);
        }
        if node["node_id"] == json!(seeded.sink_k2_node) {
            node["data"] = Value::Null;
        }
    }
    assert_eq!(*graph, expected);
}

async fn put_graph(h: &Harness, user: &TestUser, path: &str, body: Value) -> (StatusCode, Value) {
    send(h.request(Method::PUT, path, user).json(&body)).await
}

fn node(node_id: Uuid, kind: &str, x: i32, y: i32) -> Value {
    json!({ "node_id": node_id, "position_x": x, "position_y": y, "kind": kind })
}

fn rel(from: Uuid, to: Uuid) -> Value {
    json!({ "from_node_id": from, "to_node_id": to })
}

/// The node as GET returns it while it is still empty.
fn empty(mut node: Value) -> Value {
    node["data"] = match node["kind"].as_str().unwrap() {
        "source" => json!({ "item_count": 0 }),
        "processor_group" => json!({ "processors": [] }),
        _ => Value::Null,
    };
    node
}

/// Saving a graph creates, updates and deletes nodes and relations, and GET then reads the response.
/// New nodes start empty, even a sink node sent with a sink_id; a source gets two hidden processors.
#[tokio::test]
async fn save_graph_creates_updates_and_deletes_nodes() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let sink = insert_sink(&h).await;
    let pipeline = create(&h, &alice, json!({ "name": "p", "sink_ids": [sink] })).await;
    let path = graph_path(&pipeline);
    let [source, group_a, out_1, out_2, group_b, source_2] =
        std::array::from_fn(|_| Uuid::now_v7());

    // everything is new; the sink_id sent for out_1 is ignored
    let mut out_1_with_sink = node(out_1, "sink", 20, 0);
    out_1_with_sink["data"] = json!({ "sink_id": sink });
    let (status, body) = put_graph(
        &h,
        &alice,
        &path,
        json!({ "last_updated": pipeline["canvases"][0]["last_updated"],
                "graph": { "nodes": [
                    node(source, "source", 0, 0),
                    node(group_a, "processor_group", 10, 0),
                    out_1_with_sink,
                    node(out_2, "sink", 20, 5),
                ], "relations": [rel(group_a, out_1), rel(source, group_a), rel(source, out_2)] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let saved = body["data"].clone();
    assert_ne!(
        saved["last_updated"],
        pipeline["canvases"][0]["last_updated"]
    );
    assert_eq!(
        saved["graph"],
        json!({
            "nodes": [
                empty(node(source, "source", 0, 0)),
                empty(node(group_a, "processor_group", 10, 0)),
                empty(node(out_1, "sink", 20, 0)),
                empty(node(out_2, "sink", 20, 5)),
            ],
            "relations": [rel(source, group_a), rel(source, out_2), rel(group_a, out_1)],
        })
    );
    let (_, read) = get_graph(&h, &alice, &path).await;
    assert_eq!(read["data"], saved);
    // the source got its hidden processors, the group its row, the sinks nothing
    assert_eq!(
        count(
            &h,
            &format!(
                "SELECT count(*) FROM pipeline_node_sources n \
                 JOIN processors s ON s.processor_id = n.source_processor_id \
                 JOIN processors g ON g.processor_id = n.gate_processor_id \
                 WHERE n.node_id = '{source}' AND s.name = 'p (source)' AND g.name = 'p (gate)' \
                   AND s.type = 'null' AND g.type = 'null' \
                   AND s.config = 'null' AND g.config = 'null' \
                   AND s.adm_status = 1 AND g.adm_status = 1"
            )
        )
        .await,
        1
    );
    assert_eq!(count(&h, "SELECT count(*) FROM processors").await, 2);
    assert_eq!(
        uuids(&h, "SELECT node_id FROM pipeline_node_processor_groups").await,
        [group_a]
    );
    assert_eq!(
        count(&h, "SELECT count(*) FROM pipeline_node_sinks").await,
        0
    );
    let first_hidden = uuids(
        &h,
        "SELECT processor_id FROM processors ORDER BY processor_id",
    )
    .await;

    // move and rename, add a group, rewire
    let (status, body) = put_graph(
        &h,
        &alice,
        &path,
        json!({ "last_updated": saved["last_updated"],
                "graph": { "nodes": [
                    node(source, "source", 1, 1),
                    node(group_a, "processor_group", 10, 0),
                    node(out_1, "sink", 20, 0),
                    node(out_2, "sink", 20, 5),
                    node(group_b, "processor_group", 5, 5),
                ], "relations": [
                    rel(source, group_b), rel(group_b, group_a), rel(group_a, out_1), rel(group_a, out_2),
                ] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let saved = body["data"].clone();
    assert_eq!(
        saved["graph"],
        json!({
            "nodes": [
                empty(node(source, "source", 1, 1)),
                empty(node(group_a, "processor_group", 10, 0)),
                empty(node(out_1, "sink", 20, 0)),
                empty(node(out_2, "sink", 20, 5)),
                empty(node(group_b, "processor_group", 5, 5)),
            ],
            "relations": [
                rel(source, group_b), rel(group_a, out_1), rel(group_a, out_2), rel(group_b, group_a),
            ],
        })
    );
    let (_, read) = get_graph(&h, &alice, &path).await;
    assert_eq!(read["data"], saved);
    // only the new node carries the time of this save
    assert_eq!(
        uuids(
            &h,
            "SELECT n.node_id FROM pipeline_nodes n \
             JOIN pipeline_canvases c USING (canvas_id) \
             WHERE n.last_updated = c.last_updated"
        )
        .await,
        [group_b]
    );
    assert_eq!(
        count(
            &h,
            "SELECT count(DISTINCT last_updated) FROM pipeline_nodes"
        )
        .await,
        2
    );
    assert_eq!(
        uuids(
            &h,
            "SELECT processor_id FROM processors ORDER BY processor_id"
        )
        .await,
        first_hidden,
        "the hidden processors are created once"
    );
    assert_eq!(
        uuids(
            &h,
            "SELECT node_id FROM pipeline_node_processor_groups ORDER BY node_id"
        )
        .await,
        [group_a, group_b]
    );

    // a new source replaces the old one in the same save; group B and out_2 go
    let (status, body) = put_graph(
        &h,
        &alice,
        &path,
        json!({ "last_updated": saved["last_updated"],
                "graph": { "nodes": [
                    node(group_a, "processor_group", 10, 0),
                    node(out_1, "sink", 20, 0),
                    node(source_2, "source", 0, 0),
                ], "relations": [rel(source_2, group_a), rel(group_a, out_1)] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["data"]["graph"],
        json!({
            "nodes": [
                empty(node(group_a, "processor_group", 10, 0)),
                empty(node(out_1, "sink", 20, 0)),
                empty(node(source_2, "source", 0, 0)),
            ],
            "relations": [rel(group_a, out_1), rel(source_2, group_a)],
        })
    );
    // the new source has its own hidden processors, the old one's are gone
    assert_eq!(
        count(
            &h,
            &format!(
                "SELECT count(*) FROM processors p JOIN pipeline_node_sources n \
                 ON p.processor_id IN (n.source_processor_id, n.gate_processor_id) \
                 WHERE n.node_id = '{source_2}'"
            )
        )
        .await,
        2
    );
    assert_eq!(count(&h, "SELECT count(*) FROM processors").await, 2);
    assert_eq!(
        uuids(&h, "SELECT node_id FROM pipeline_node_sources").await,
        [source_2]
    );
    assert_eq!(
        uuids(&h, "SELECT node_id FROM pipeline_node_processor_groups").await,
        [group_a]
    );
}

/// A save with a stale last_updated is rejected with 409 and changes nothing.
/// Of two saves sent at the same time from the same read, exactly one wins.
#[tokio::test]
async fn save_graph_rejects_a_stale_canvas() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let pipeline = create(&h, &alice, json!({ "name": "p" })).await;
    let path = graph_path(&pipeline);
    let save = |read: &Value| {
        json!({ "last_updated": read,
                "graph": { "nodes": [node(Uuid::now_v7(), "processor_group", 0, 0)], "relations": [] } })
    };

    let first_read = &pipeline["canvases"][0]["last_updated"];
    let (status, saved) = put_graph(&h, &alice, &path, save(first_read)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, stale) = put_graph(&h, &alice, &path, save(first_read)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["error"]["title"], "Canvas changed");
    let (_, read) = get_graph(&h, &alice, &path).await;
    assert_eq!(read["data"], saved["data"]);

    for _ in 0..10 {
        let (_, read) = get_graph(&h, &alice, &path).await;
        let read = &read["data"]["last_updated"];
        let (a, b) = tokio::join!(
            put_graph(&h, &alice, &path, save(read)),
            put_graph(&h, &alice, &path, save(read)),
        );
        let mut statuses = [a.0, b.0];
        statuses.sort();
        assert_eq!(
            statuses,
            [StatusCode::OK, StatusCode::CONFLICT],
            "{} {}",
            a.1,
            b.1
        );
    }
    assert_eq!(count(&h, "SELECT count(*) FROM pipeline_nodes").await, 1);
    assert_eq!(
        count(&h, "SELECT count(*) FROM pipeline_node_processor_groups").await,
        1
    );
}

/// Checks that need the database: a node id from another canvas, a node changing its kind
/// (an empty sink node too). Each is a 400 that leaves the canvas as it was, like a broken graph.
#[tokio::test]
async fn save_graph_rejects_what_the_database_does_not_allow() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let pipeline = create(&h, &alice, json!({ "name": "p" })).await;
    let other = create(&h, &alice, json!({ "name": "other" })).await;
    let path = graph_path(&pipeline);
    let [group, sink, foreign] = std::array::from_fn(|_| Uuid::now_v7());

    let (status, body) = put_graph(
        &h,
        &alice,
        &graph_path(&other),
        json!({ "last_updated": other["canvases"][0]["last_updated"],
                "graph": { "nodes": [node(foreign, "processor_group", 0, 0)], "relations": [] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = put_graph(
        &h,
        &alice,
        &path,
        json!({ "last_updated": pipeline["canvases"][0]["last_updated"],
                "graph": { "nodes": [
                    node(group, "processor_group", 0, 0),
                    node(sink, "sink", 1, 1),
                ], "relations": [rel(group, sink)] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let saved = body["data"].clone();

    for (case, nodes, relations, detail) in [
        (
            "node of another canvas",
            vec![node(foreign, "processor_group", 0, 0)],
            vec![],
            format!("Node IDs already used: {foreign}"),
        ),
        (
            "group becoming a source",
            vec![node(group, "source", 0, 0)],
            vec![],
            format!("Node {group} is a processor_group node and cannot become a source node"),
        ),
        (
            "empty sink becoming a group",
            vec![node(sink, "processor_group", 1, 1)],
            vec![],
            format!("Node {sink} is a sink node and cannot become a processor_group node"),
        ),
        (
            "broken graph",
            vec![node(group, "processor_group", 0, 0)],
            vec![rel(group, group)],
            format!("Relation {group} -> {group} connects a node to itself"),
        ),
    ] {
        let (status, body) = put_graph(
            &h,
            &alice,
            &path,
            json!({ "last_updated": saved["last_updated"],
                    "graph": { "nodes": nodes, "relations": relations } }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{case}: {body}");
        assert_eq!(body["error"]["detail"], detail, "{case}");
    }

    let (_, read) = get_graph(&h, &alice, &path).await;
    assert_eq!(read["data"], saved);
    assert_eq!(
        uuids(
            &h,
            "SELECT node_id FROM pipeline_node_processor_groups ORDER BY node_id"
        )
        .await,
        [group, foreign]
    );
}

/// Sending back what GET returned keeps every node's content, whatever its data says.
/// Removing a node deletes its content: a group's processors with their runtime rows, a sink node's sink.
#[tokio::test]
async fn save_graph_keeps_content_and_deletes_it_with_the_node() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let host = insert_host(&h).await;
    let pipeline = create(&h, &alice, json!({ "name": "p", "host_ids": [host] })).await;
    let seeded = seed_canvas(&h, &pipeline, host).await;
    let path = graph_path(&pipeline);

    // send back what was read with group A moved and another sink in sink k1's data
    let (_, read) = get_graph(&h, &alice, &path).await;
    let mut graph = read["data"]["graph"].clone();
    graph["nodes"][0]["position_x"] = json!(-999);
    graph["nodes"][1]["data"]["sink_id"] = json!(seeded.sink_k2);
    let (status, body) = put_graph(
        &h,
        &alice,
        &path,
        json!({ "last_updated": read["data"]["last_updated"], "graph": graph }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut expected = seeded.expected.clone();
    expected["nodes"][0]["position_x"] = json!(-999);
    assert_eq!(body["data"]["graph"], expected);

    // the save wired group A's processors; add a processor from the old API reading one
    let members: Vec<Uuid> = expected["nodes"][0]["data"]["processors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["processor_id"].as_str().unwrap().parse().unwrap())
        .collect();
    let [src, gate] = seeded.hidden_processors;
    let sink_k1: Uuid = expected["nodes"][1]["data"]["sink_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let foreign = Uuid::now_v7();
    sql(
        &h,
        &format!(
            "
        INSERT INTO processors (processor_id, name, type, adm_status, last_updated, config)
            VALUES ('{foreign}', 'foreign', 'null', 1, now(), 'null');
        INSERT INTO processor_inputs (processor_id, input_id) VALUES ('{foreign}', '{a2}');
    ",
            a2 = members[2],
        ),
    )
    .await;
    assert_eq!(
        runtime_rows(&h).await,
        rows(
            &[],
            &[
                (gate, src),
                (members[0], gate),
                (members[1], members[0]),
                (members[2], members[1]),
                (foreign, members[2]),
            ],
            &[
                (sink_k1, members[2]),
                (seeded.sink_k2, members[2]),
                (sink_k1, gate)
            ],
        )
    );

    // remove group A and the sink k2 node
    let mut graph = body["data"]["graph"].clone();
    let gone = [
        expected["nodes"][0]["node_id"].clone(),
        json!(seeded.sink_k2_node),
    ];
    graph["nodes"]
        .as_array_mut()
        .unwrap()
        .retain(|n| !gone.contains(&n["node_id"]));
    graph["relations"]
        .as_array_mut()
        .unwrap()
        .retain(|r| !gone.contains(&r["from_node_id"]) && !gone.contains(&r["to_node_id"]));
    let (status, body) = put_graph(
        &h,
        &alice,
        &path,
        json!({ "last_updated": body["data"]["last_updated"], "graph": graph }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["graph"], graph);

    // group A's processors are gone with every row pointing at them, the rest stays
    assert_eq!(
        uuids(
            &h,
            "SELECT processor_id FROM processors ORDER BY processor_id"
        )
        .await,
        [src, gate, foreign]
    );
    let client = h.pool.get().await.unwrap();
    let inputs: Vec<(Uuid, Uuid)> = client
        .query("SELECT processor_id, input_id FROM processor_inputs", &[])
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(inputs, [(gate, src)]);
    assert_eq!(
        runtime_rows(&h).await["sink_inputs"],
        json!([[sink_k1, gate]])
    );
    // the sink k2 node's sink went with the node, the sink itself stays
    assert_eq!(
        count(&h, "SELECT count(*) FROM pipeline_node_sinks").await,
        1
    );
    assert_eq!(count(&h, "SELECT count(*) FROM sinks").await, 2);
}

/// Saving needs the pipeline to be visible and the canvas to be the pipeline's; otherwise 404.
/// Anyone can save the graph of a public pipeline, not only its owner.
#[tokio::test]
async fn save_graph_follows_visibility() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let bob = h.user("bob").await;
    let private = create(&h, &alice, json!({ "name": "private" })).await;
    let public = create(&h, &alice, json!({ "name": "public", "visibility": 1 })).await;
    let save = |p: &Value| {
        json!({ "last_updated": p["canvases"][0]["last_updated"],
                "graph": { "nodes": [node(Uuid::now_v7(), "source", 0, 0)], "relations": [] } })
    };

    let (status, body) = put_graph(&h, &bob, &graph_path(&private), save(&private)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["title"], "Pipeline not found");
    let wrong_canvas = format!(
        "{PATH}/{}/canvases/{}/graph",
        public["pipeline_id"].as_str().unwrap(),
        private["canvases"][0]["canvas_id"].as_str().unwrap()
    );
    let (status, body) = put_graph(&h, &alice, &wrong_canvas, save(&public)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["title"], "Canvas not found");
    assert_eq!(count(&h, "SELECT count(*) FROM pipeline_nodes").await, 0);

    let (status, body) = put_graph(&h, &bob, &graph_path(&public), save(&public)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(count(&h, "SELECT count(*) FROM pipeline_nodes").await, 1);
}

fn node_path(pipeline: &Value, node_id: &Value) -> String {
    format!(
        "{PATH}/{}/canvases/{}/nodes/{}",
        pipeline["pipeline_id"].as_str().unwrap(),
        pipeline["canvases"][0]["canvas_id"].as_str().unwrap(),
        node_id.as_str().unwrap()
    )
}

async fn get_node(h: &Harness, user: &TestUser, path: &str) -> Value {
    let (status, body) = send(h.request(Method::GET, path, user)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"].clone()
}

/// A source lists its items by name, one page at a time, with template items named after their template.
/// A group lists its processors in order with their config; a sink node shows its sink, or null.
#[tokio::test]
async fn node_content_by_kind() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let host = insert_host(&h).await;
    let pipeline = create(&h, &alice, json!({ "name": "p", "host_ids": [host] })).await;
    let seeded = seed_canvas(&h, &pipeline, host).await;
    let [item_1, item_2, applied] = seeded.items;
    let nodes = &seeded.expected["nodes"];
    let item = |id: Uuid, name: &str, adm_status: i32| {
        json!({ "item_id": id, "name": name, "host_id": host, "host_name": format!("h-{host}"),
                "type": "dummy", "adm_status": adm_status })
    };

    // the oldest item sorts last by name
    sql(
        &h,
        &format!("UPDATE items SET name = 'z1' WHERE item_id = '{item_1}'"),
    )
    .await;
    let source = node_path(&pipeline, &json!(seeded.source_node));
    let data = get_node(&h, &alice, &source).await;
    assert_eq!(data["kind"], "source");
    assert_eq!(data["node_id"], json!(seeded.source_node));
    assert_eq!(
        data["data"],
        json!({ "items": [item(item_2, "i2", 0), item(applied, "tmpl", 1), item(item_1, "z1", 1)],
                "total": 3 })
    );
    let page = get_node(&h, &alice, &format!("{source}?limit=1&offset=1")).await;
    assert_eq!(
        page["data"],
        json!({ "items": [item(applied, "tmpl", 1)], "total": 3 })
    );
    let filtered = get_node(&h, &alice, &format!("{source}?name=TMP")).await;
    assert_eq!(
        filtered["data"],
        json!({ "items": [item(applied, "tmpl", 1)], "total": 1 })
    );
    let past_the_end = get_node(&h, &alice, &format!("{source}?offset=10")).await;
    assert_eq!(past_the_end["data"], json!({ "items": [], "total": 3 }));

    // a name two template items up, and a chain that loops back on itself
    let template_items = h
        .pool
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT host_template_id, template_item_id FROM items WHERE item_id = $1",
            &[&applied],
        )
        .await
        .unwrap();
    let (applied_row, base): (Uuid, Uuid) = (template_items.get(0), template_items.get(1));
    let [mid, deep, loop_a, loop_b, in_loop] = std::array::from_fn(|_| Uuid::now_v7());
    sql(&h, &format!("
        INSERT INTO items (item_id, host_template_id, template_item_id, type, adm_status, last_updated) VALUES
            ('{mid}', '{applied_row}', '{base}', 'dummy', 1, now()),
            ('{deep}', '{applied_row}', '{mid}', 'dummy', 1, now()),
            ('{loop_a}', '{applied_row}', '{base}', 'dummy', 1, now()),
            ('{loop_b}', '{applied_row}', '{loop_a}', 'dummy', 1, now()),
            ('{in_loop}', '{applied_row}', '{loop_b}', 'dummy', 1, now());
        UPDATE items SET template_item_id = '{loop_b}' WHERE item_id = '{loop_a}';
        INSERT INTO pipeline_node_source_items (node_id, item_id) VALUES
            ('{s}', '{deep}'), ('{s}', '{in_loop}');
    ", s = seeded.source_node)).await;
    let filtered = get_node(&h, &alice, &format!("{source}?name=tmpl")).await;
    assert_eq!(
        filtered["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["item_id"].clone())
            .collect::<Vec<_>>(),
        [json!(applied), json!(deep)]
    );
    let all = get_node(&h, &alice, &source).await;
    assert_eq!(all["data"]["total"], 5);
    assert_eq!(all["data"]["items"][4]["item_id"], json!(in_loop));
    assert_eq!(all["data"]["items"][4]["name"], json!(null));
    sql(
        &h,
        &format!("DELETE FROM pipeline_node_source_items WHERE item_id IN ('{deep}', '{in_loop}')"),
    )
    .await;

    let group_a = get_node(&h, &alice, &node_path(&pipeline, &nodes[0]["node_id"])).await;
    let summary = &nodes[0]["data"]["processors"];
    assert_eq!(group_a["kind"], "processor_group");
    assert_eq!(
        group_a["data"],
        json!({ "processors": [
            { "processor_id": summary[0]["processor_id"], "name": "rename a", "adm_status": 0,
              "config": { "type": "rename_attributes", "config": { "rename_map": { "record": { "a": "b" } } } } },
            { "processor_id": summary[1]["processor_id"], "name": "pass", "adm_status": 1,
              "config": { "type": "null" } },
            { "processor_id": summary[2]["processor_id"], "name": "drop x", "adm_status": 1,
              "config": { "type": "drop_attributes", "config": { "keys": { "record": ["x"] } } } },
        ] })
    );
    let group_b = get_node(&h, &alice, &node_path(&pipeline, &nodes[4]["node_id"])).await;
    assert_eq!(group_b["data"], json!({ "processors": [] }));

    let sink = get_node(
        &h,
        &alice,
        &node_path(&pipeline, &json!(seeded.sink_k2_node)),
    )
    .await;
    assert_eq!(sink["kind"], "sink");
    assert_eq!(
        sink["data"],
        json!({ "sink_id": seeded.sink_k2, "name": format!("s-{}", seeded.sink_k2),
                "type": "file", "adm_status": 1 })
    );
    // a deleted sink leaves its node empty
    sql(
        &h,
        &format!("DELETE FROM sinks WHERE sink_id = '{}'", seeded.sink_k2),
    )
    .await;
    let sink = get_node(
        &h,
        &alice,
        &node_path(&pipeline, &json!(seeded.sink_k2_node)),
    )
    .await;
    assert_eq!(sink["data"], Value::Null);

    // a source without items
    sql(&h, "DELETE FROM pipeline_node_source_items").await;
    let empty = get_node(&h, &alice, &format!("{source}?offset=10")).await;
    assert_eq!(empty["data"], json!({ "items": [], "total": 0 }));

    // each node reports its own last_updated, here the time the seed wrote it
    let seeded_at: chrono::DateTime<chrono::Utc> = h
        .pool
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT last_updated FROM pipeline_nodes WHERE node_id = $1",
            &[&seeded.source_node],
        )
        .await
        .unwrap()
        .get(0);
    let reported: chrono::DateTime<chrono::Utc> =
        serde_json::from_value(data["last_updated"].clone()).unwrap();
    assert_eq!(reported, seeded_at);
}

/// A node is 404 when it is unknown or belongs to another canvas, with the title naming the node.
/// A hidden pipeline or a foreign canvas is 404 before the node is looked at.
#[tokio::test]
async fn node_not_found_cases() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let bob = h.user("bob").await;
    let host = insert_host(&h).await;
    let pipeline = create(&h, &alice, json!({ "name": "p", "host_ids": [host] })).await;
    let other = create(&h, &alice, json!({ "name": "other" })).await;
    let seeded = seed_canvas(&h, &pipeline, host).await;
    let source = json!(seeded.source_node);

    for (case, user, path, title) in [
        (
            "unknown node",
            &alice,
            node_path(&pipeline, &json!(Uuid::now_v7())),
            "Node not found",
        ),
        (
            "node of another canvas",
            &alice,
            node_path(&other, &source),
            "Node not found",
        ),
        (
            "hidden pipeline",
            &bob,
            node_path(&pipeline, &source),
            "Pipeline not found",
        ),
        (
            "canvas of another pipeline",
            &alice,
            format!(
                "{PATH}/{}/canvases/{}/nodes/{}",
                pipeline["pipeline_id"].as_str().unwrap(),
                other["canvases"][0]["canvas_id"].as_str().unwrap(),
                seeded.source_node
            ),
            "Canvas not found",
        ),
    ] {
        let (status, body) = send(h.request(Method::GET, &path, user)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{case}: {body}");
        assert_eq!(body["error"]["title"], title, "{case}");
    }
}

async fn put_node(h: &Harness, user: &TestUser, path: &str, body: Value) -> (StatusCode, Value) {
    send(h.request(Method::PUT, path, user).json(&body)).await
}

/// An item made directly on a host, or a template item when `host` is None.
async fn insert_item(h: &Harness, host: Option<Uuid>, name: &str) -> Uuid {
    let (item, host_template) = (Uuid::now_v7(), Uuid::now_v7());
    // a host has one row of its own in host_templates
    let owner = match host {
        Some(host) => format!(
            "INSERT INTO host_templates (host_template_id, host_id) SELECT '{host_template}', '{host}' \
             WHERE NOT EXISTS (SELECT 1 FROM host_templates WHERE host_id = '{host}' AND template_id IS NULL); \
             CREATE TEMP TABLE owner AS SELECT host_template_id FROM host_templates \
             WHERE host_id = '{host}' AND template_id IS NULL;"
        ),
        None => {
            let template = Uuid::now_v7();
            format!(
                "INSERT INTO templates (template_id, name) VALUES ('{template}', 't-{template}'); \
                 INSERT INTO host_templates (host_template_id, template_id) VALUES ('{host_template}', '{template}'); \
                 CREATE TEMP TABLE owner AS SELECT '{host_template}'::uuid AS host_template_id;"
            )
        }
    };
    sql(h, &format!("
        {owner}
        INSERT INTO items (item_id, host_template_id, type, adm_status, name, schedule, timeout_ms, last_updated, config)
            SELECT '{item}', host_template_id, 'dummy', 1, '{name}', '* * * * *', 1000, now(), '{{}}' FROM owner;
        DROP TABLE owner;
    ")).await;
    item
}

/// Saves a graph with the given nodes and no relations, and returns the response data.
async fn save_nodes(h: &Harness, user: &TestUser, pipeline: &Value, nodes: Vec<Value>) -> Value {
    let (_, read) = get_graph(h, user, &graph_path(pipeline)).await;
    let (status, body) = put_graph(
        h,
        user,
        &graph_path(pipeline),
        json!({ "last_updated": read["data"]["last_updated"],
                "graph": { "nodes": nodes, "relations": [] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"].clone()
}

/// A source takes items to add and remove; items already there or missing are skipped.
/// Only items on a host of the pipeline are allowed, and a stale node is a 409.
#[tokio::test]
async fn save_source_adds_and_removes_items() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let (host_a, host_b) = (insert_host(&h).await, insert_host(&h).await);
    let pipeline = create(&h, &alice, json!({ "name": "p", "host_ids": [host_a] })).await;
    let [a1, a2, a3] = [
        insert_item(&h, Some(host_a), "a1").await,
        insert_item(&h, Some(host_a), "a2").await,
        insert_item(&h, Some(host_a), "a3").await,
    ];
    let on_b = insert_item(&h, Some(host_b), "b1").await;
    let template_item = insert_item(&h, None, "t1").await;
    let source = Uuid::now_v7();
    let saved = save_nodes(&h, &alice, &pipeline, vec![node(source, "source", 0, 0)]).await;
    let path = node_path(&pipeline, &json!(source));
    let names = |data: &Value| -> Vec<String> {
        data["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["name"].as_str().unwrap().to_string())
            .collect()
    };

    let read = get_node(&h, &alice, &path).await;
    assert_eq!(read["data"], json!({ "items": [], "total": 0 }));
    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": read["last_updated"], "kind": "source", "data": { "add": [a3, a1, a2] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let first = body["data"].clone();
    assert_eq!(names(&first), ["a1", "a2", "a3"]);
    assert_eq!(first["data"]["total"], 3);
    assert_ne!(first["last_updated"], read["last_updated"]);
    assert_eq!(get_node(&h, &alice, &path).await, first);

    // the read before the save is stale now
    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": read["last_updated"], "kind": "source", "data": { "remove": [a1] } }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["title"], "Node changed");

    // an add that is there and a remove that is not are skipped
    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": first["last_updated"], "kind": "source",
                "data": { "add": [a1], "remove": [a2, on_b] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let second = body["data"].clone();
    assert_eq!(names(&second), ["a1", "a3"]);

    let unknown = Uuid::now_v7();
    for (case, data, detail) in [
        (
            "item of another host",
            json!({ "add": [on_b] }),
            format!("Items not on a host of this pipeline: {on_b}"),
        ),
        (
            "template item",
            json!({ "add": [template_item] }),
            format!("Items not on a host of this pipeline: {template_item}"),
        ),
        (
            "unknown item",
            json!({ "add": [unknown] }),
            format!("Item IDs not found: {unknown}"),
        ),
        (
            "added and removed",
            json!({ "add": [a2], "remove": [a2] }),
            format!("Item {a2} is both added and removed"),
        ),
    ] {
        let (status, body) = put_node(
            &h,
            &alice,
            &path,
            json!({ "last_updated": second["last_updated"], "kind": "source", "data": data }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{case}: {body}");
        assert_eq!(body["error"]["detail"], detail, "{case}");
    }
    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": second["last_updated"], "kind": "processor_group",
                "data": { "processors": [] } }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"]["detail"],
        format!("Node {source} is a source node, not a processor_group node")
    );
    assert_eq!(get_node(&h, &alice, &path).await, second);

    // the graph counts the items, and its last_updated did not move
    let (_, graph) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    assert_eq!(graph["data"]["graph"]["nodes"][0]["data"]["item_count"], 2);
    assert_eq!(graph["data"]["last_updated"], saved["last_updated"]);
}

fn processor(id: Uuid, name: &str, adm_status: i32, config: Value) -> Value {
    json!({ "processor_id": id, "name": name, "adm_status": adm_status, "config": config })
}

/// A group takes its whole processor list: new ids are created, missing ones deleted, and
/// the order is the list order. Unchanged processors are not rewritten.
#[tokio::test]
async fn save_group_creates_updates_reorders_and_deletes_processors() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let pipeline = create(&h, &alice, json!({ "name": "p" })).await;
    let group = Uuid::now_v7();
    save_nodes(
        &h,
        &alice,
        &pipeline,
        vec![node(group, "processor_group", 0, 0)],
    )
    .await;
    let path = node_path(&pipeline, &json!(group));
    let [p1, p2, p3, p4] = std::array::from_fn(|_| Uuid::now_v7());
    let drop_x = json!({ "type": "drop_attributes", "config": { "keys": { "record": ["x"] } } });
    let filter = |n: &str| -> Value {
        // config before type, and a decimal not written the shortest way
        serde_json::from_str(&format!(
            r#"{{"config":{{"match":{{"table":"record","key":"x","key_is_regex":false,"op":">","value":{{"float":{n}}}}}}},"type":"attribute_filter"}}"#
        ))
        .unwrap()
    };

    let read = get_node(&h, &alice, &path).await;
    assert_eq!(read["data"], json!({ "processors": [] }));
    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": read["last_updated"], "kind": "processor_group", "data": { "processors": [
            processor(p1, "drop x", 1, drop_x.clone()),
            processor(p2, "filter", 1, filter("1.50")),
            processor(p3, "pass", 0, json!({ "type": "null" })),
        ] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let first = body["data"].clone();
    let stored_filter = json!({ "type": "attribute_filter", "config": { "match": {
        "table": "record", "key": "x", "key_is_regex": false, "op": ">", "value": { "float": 1.5 } } } });
    assert_eq!(
        first["data"]["processors"],
        json!([
            processor(p1, "drop x", 1, drop_x.clone()),
            processor(p2, "filter", 1, stored_filter.clone()),
            processor(p3, "pass", 0, json!({ "type": "null" })),
        ])
    );
    assert_eq!(get_node(&h, &alice, &path).await, first);
    let p3_written = count(&h, &format!(
        "SELECT extract(epoch FROM last_updated)::bigint * 1000000 + extract(microseconds FROM last_updated)::bigint % 1000000 \
         FROM processors WHERE processor_id = '{p3}'"
    )).await;

    // reorder, rename one, drop one, add one
    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": first["last_updated"], "kind": "processor_group", "data": { "processors": [
            processor(p3, "pass", 0, json!({ "type": "null" })),
            processor(p4, "filter 2", 1, filter("2.5e-3")),
            processor(p1, "drop it", 1, drop_x.clone()),
        ] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let second = body["data"].clone();
    assert_eq!(
        second["data"]["processors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["pass", "filter 2", "drop it"]
    );
    assert_eq!(
        second["data"]["processors"][1]["config"]["config"]["match"]["value"]["float"],
        json!(0.0025)
    );
    assert_eq!(
        count(
            &h,
            &format!("SELECT count(*) FROM processors WHERE processor_id = '{p2}'")
        )
        .await,
        0
    );
    assert_eq!(
        count(&h, &format!(
            "SELECT extract(epoch FROM last_updated)::bigint * 1000000 + extract(microseconds FROM last_updated)::bigint % 1000000 \
             FROM processors WHERE processor_id = '{p3}'"
        )).await,
        p3_written,
        "an unchanged processor is not rewritten"
    );
    assert_eq!(
        count(&h, &format!(
            "SELECT count(*) FROM pipeline_node_processors WHERE node_id = '{group}' \
             AND ((processor_id = '{p3}' AND position = 0) OR (processor_id = '{p4}' AND position = 1) \
             OR (processor_id = '{p1}' AND position = 2))"
        )).await,
        3
    );

    // an empty list deletes every processor
    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": second["last_updated"], "kind": "processor_group",
                "data": { "processors": [] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(count(&h, "SELECT count(*) FROM processors").await, 0);
}

/// Bad processors are a 400 and change nothing: duplicates, empty names, invalid config, ids
/// in use elsewhere, and values the engine could not read back once stored, like 1e20.
#[tokio::test]
async fn save_group_rejects_bad_processors() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let pipeline = create(&h, &alice, json!({ "name": "p" })).await;
    let [group, other_group] = std::array::from_fn(|_| Uuid::now_v7());
    save_nodes(
        &h,
        &alice,
        &pipeline,
        vec![
            node(group, "processor_group", 0, 0),
            node(other_group, "processor_group", 0, 0),
        ],
    )
    .await;
    let other_path = node_path(&pipeline, &json!(other_group));
    let read = get_node(&h, &alice, &other_path).await;
    let taken = Uuid::now_v7();
    let pass = json!({ "type": "null" });
    let (status, body) = put_node(
        &h,
        &alice,
        &other_path,
        json!({ "last_updated": read["last_updated"], "kind": "processor_group",
                "data": { "processors": [processor(taken, "taken", 1, pass.clone())] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let path = node_path(&pipeline, &json!(group));
    let read = get_node(&h, &alice, &path).await;
    let id = Uuid::now_v7();
    let float = |n: &str| -> Value {
        serde_json::from_str(&format!(
            r#"{{"type":"attribute_filter","config":{{"match":{{"table":"record","key":"x","key_is_regex":false,"op":">","value":{{"float":{n}}}}}}}}}"#
        ))
        .unwrap()
    };
    for (case, processors, detail) in [
        (
            "duplicate id",
            vec![
                processor(id, "a", 1, pass.clone()),
                processor(id, "b", 1, pass.clone()),
            ],
            format!("Duplicate processor id: {id}"),
        ),
        (
            "empty name",
            vec![processor(id, " ", 1, pass.clone())],
            format!("Processor {id} needs a name"),
        ),
        (
            "unknown module",
            vec![processor(id, "a", 1, json!({ "type": "nope" }))],
            format!("Invalid config of processor {id}"),
        ),
        (
            "overlapping renames",
            vec![processor(
                id,
                "a",
                1,
                json!({ "type": "rename_attributes",
            "config": { "rename_map": { "record": { "a": "b", "b": "c" } } } }),
            )],
            format!("Invalid config of processor {id}"),
        ),
        (
            "id of another group",
            vec![processor(taken, "a", 1, pass.clone())],
            format!("Processor IDs already used: {taken}"),
        ),
        (
            "float the engine cannot read back",
            vec![processor(id, "a", 1, float("1e20"))],
            format!("Processor {id} cannot be read back as stored"),
        ),
    ] {
        let (status, body) = put_node(
            &h,
            &alice,
            &path,
            json!({ "last_updated": read["last_updated"], "kind": "processor_group",
                    "data": { "processors": processors } }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{case}: {body}");
        let message = body["error"]["detail"].as_str().unwrap();
        assert!(message.starts_with(&detail), "{case}: {message}");
    }
    assert_eq!(get_node(&h, &alice, &path).await, read);
    assert_eq!(count(&h, "SELECT count(*) FROM processors").await, 1);

    // a disabled processor is checked too
    let (status, _) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": read["last_updated"], "kind": "processor_group",
                "data": { "processors": [processor(id, "a", 0, float("1e20"))] } }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The graph and the content of a node are saved apart: saving one never makes the other 409.
/// Moving a node in the graph keeps the node's last_updated, and a deleted node is 404.
#[tokio::test]
async fn graph_and_node_saves_do_not_conflict() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let pipeline = create(&h, &alice, json!({ "name": "p" })).await;
    let group = Uuid::now_v7();
    let saved = save_nodes(
        &h,
        &alice,
        &pipeline,
        vec![node(group, "processor_group", 0, 0)],
    )
    .await;
    let path = node_path(&pipeline, &json!(group));
    let read = get_node(&h, &alice, &path).await;

    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": read["last_updated"], "kind": "processor_group",
                "data": { "processors": [processor(Uuid::now_v7(), "pass", 1, json!({ "type": "null" }))] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let node_saved = body["data"]["last_updated"].clone();

    // the graph read before the node save still saves, and moving the node keeps its time
    let (status, body) = put_graph(
        &h,
        &alice,
        &graph_path(&pipeline),
        json!({ "last_updated": saved["last_updated"],
                "graph": { "nodes": [node(group, "processor_group", 50, 50)], "relations": [] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let moved = get_node(&h, &alice, &path).await;
    assert_eq!(moved["last_updated"], node_saved);

    // removing the node from the graph makes it 404 and deletes its processors
    let (status, _) = put_graph(
        &h,
        &alice,
        &graph_path(&pipeline),
        json!({ "last_updated": body["data"]["last_updated"],
                "graph": { "nodes": [], "relations": [] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": node_saved, "kind": "processor_group", "data": { "processors": [] } }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["title"], "Node not found");
    assert_eq!(count(&h, "SELECT count(*) FROM processors").await, 0);
}

/// A sink node takes a sink of the pipeline's subset, can switch to another, and null empties it.
/// The graph shows the sink id, and the sink itself is never deleted.
#[tokio::test]
async fn save_sink_sets_and_clears_the_sink() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let [sink_a, sink_b, outside] = [
        insert_sink(&h).await,
        insert_sink(&h).await,
        insert_sink(&h).await,
    ];
    let pipeline = create(
        &h,
        &alice,
        json!({ "name": "p", "sink_ids": [sink_a, sink_b] }),
    )
    .await;
    let sink_node = Uuid::now_v7();
    save_nodes(&h, &alice, &pipeline, vec![node(sink_node, "sink", 0, 0)]).await;
    let path = node_path(&pipeline, &json!(sink_node));
    let sink = |id: Uuid| json!({ "sink_id": id, "name": format!("s-{id}"), "type": "file", "adm_status": 1 });
    let graph_sink = |graph: Value| graph["data"]["graph"]["nodes"][0]["data"].clone();

    let read = get_node(&h, &alice, &path).await;
    assert_eq!(read["data"], Value::Null);
    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": read["last_updated"], "kind": "sink", "data": { "sink_id": sink_a } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let first = body["data"].clone();
    assert_eq!(first["data"], sink(sink_a));
    assert_eq!(get_node(&h, &alice, &path).await, first);
    let (_, graph) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    assert_eq!(graph_sink(graph), json!({ "sink_id": sink_a }));

    // switch to another sink
    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": first["last_updated"], "kind": "sink", "data": { "sink_id": sink_b } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let second = body["data"].clone();
    assert_eq!(second["data"], sink(sink_b));

    let unknown = Uuid::now_v7();
    for (case, body, detail) in [
        (
            "sink outside the subset",
            json!({ "kind": "sink", "data": { "sink_id": outside } }),
            format!("Sink {outside} is not in this pipeline"),
        ),
        (
            "unknown sink",
            json!({ "kind": "sink", "data": { "sink_id": unknown } }),
            format!("Sink {unknown} is not in this pipeline"),
        ),
        (
            "content of another kind",
            json!({ "kind": "source", "data": { "add": [] } }),
            format!("Node {sink_node} is a sink node, not a source node"),
        ),
    ] {
        let mut body_with_time = body.clone();
        body_with_time["last_updated"] = second["last_updated"].clone();
        let (status, response) = put_node(&h, &alice, &path, body_with_time).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{case}: {response}");
        assert_eq!(response["error"]["detail"], detail, "{case}");
    }
    assert_eq!(get_node(&h, &alice, &path).await, second);

    // data must be sent, even when it is null
    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": second["last_updated"], "kind": "sink" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(get_node(&h, &alice, &path).await, second);

    // null empties the node
    let (status, body) = put_node(
        &h,
        &alice,
        &path,
        json!({ "last_updated": second["last_updated"], "kind": "sink", "data": null }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["data"], Value::Null);
    let (_, graph) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    assert_eq!(graph_sink(graph), Value::Null);
    assert_eq!(
        count(&h, "SELECT count(*) FROM pipeline_node_sinks").await,
        0
    );
    assert_eq!(count(&h, "SELECT count(*) FROM sinks").await, 3);
}

/// The runtime rows of every kind, as `(a, b)` pairs sorted.
async fn runtime_rows(h: &Harness) -> Value {
    let client = h.pool.get().await.unwrap();
    let mut out = serde_json::Map::new();
    for (table, a, b) in [
        ("item_processors", "item_id", "processor_id"),
        ("processor_inputs", "processor_id", "input_id"),
        ("sink_inputs", "sink_id", "input_id"),
    ] {
        let rows = client
            .query(
                &format!("SELECT {a}, {b} FROM {table} ORDER BY {a}, {b}"),
                &[],
            )
            .await
            .unwrap();
        let pairs: Vec<Value> = rows
            .iter()
            .map(|r| json!([r.get::<_, Uuid>(0), r.get::<_, Uuid>(1)]))
            .collect();
        out.insert(table.to_string(), json!(pairs));
    }
    Value::Object(out)
}

fn rows(
    item_processors: &[(Uuid, Uuid)],
    processor_inputs: &[(Uuid, Uuid)],
    sink_inputs: &[(Uuid, Uuid)],
) -> Value {
    let sorted = |pairs: &[(Uuid, Uuid)]| {
        let mut pairs = pairs.to_vec();
        pairs.sort();
        json!(pairs.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>())
    };
    json!({ "item_processors": sorted(item_processors), "processor_inputs": sorted(processor_inputs),
            "sink_inputs": sorted(sink_inputs) })
}

async fn hidden_processors(h: &Harness, source_node: Uuid) -> (Uuid, Uuid) {
    let row = h
        .pool
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT source_processor_id, gate_processor_id FROM pipeline_node_sources WHERE node_id = $1",
            &[&source_node],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1))
}

/// Saves the content of a node from its current read.
async fn save_node(
    h: &Harness,
    user: &TestUser,
    pipeline: &Value,
    node_id: Uuid,
    kind: &str,
    data: Value,
) -> Value {
    let path = node_path(pipeline, &json!(node_id));
    let read = get_node(h, user, &path).await;
    let (status, body) = put_node(
        h,
        user,
        &path,
        json!({ "last_updated": read["last_updated"], "kind": kind, "data": data }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"].clone()
}

/// Saving wires items -> source -> gate -> groups -> sinks, with empty groups passed through.
/// Each save rewires what changed and drops rows other clients added, but keeps outside readers.
#[tokio::test]
async fn saving_wires_the_runtime_rows() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let host = insert_host(&h).await;
    let (k1, k2) = (insert_sink(&h).await, insert_sink(&h).await);
    let pipeline = create(
        &h,
        &alice,
        json!({ "name": "p", "host_ids": [host], "sink_ids": [k1, k2] }),
    )
    .await;
    let (a1, a2) = (
        insert_item(&h, Some(host), "a1").await,
        insert_item(&h, Some(host), "a2").await,
    );
    let [s, g, e, n1, n2] = std::array::from_fn(|_| Uuid::now_v7());
    let [p1, p2] = std::array::from_fn(|_| Uuid::now_v7());
    let nodes = vec![
        node(s, "source", 0, 0),
        node(g, "processor_group", 0, 0),
        node(e, "processor_group", 0, 0),
        node(n1, "sink", 0, 0),
        node(n2, "sink", 0, 0),
    ];
    let (_, read) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    let (status, body) = put_graph(
        &h,
        &alice,
        &graph_path(&pipeline),
        json!({ "last_updated": read["data"]["last_updated"], "graph": { "nodes": nodes.clone(),
                "relations": [rel(s, g), rel(g, n1), rel(s, e), rel(e, n2)] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (src, gate) = hidden_processors(&h, s).await;
    assert_eq!(
        runtime_rows(&h).await,
        rows(&[], &[(gate, src)], &[]),
        "empty sink nodes take nothing"
    );
    save_node(&h, &alice, &pipeline, n1, "sink", json!({ "sink_id": k1 })).await;
    save_node(&h, &alice, &pipeline, n2, "sink", json!({ "sink_id": k2 })).await;
    assert_eq!(
        runtime_rows(&h).await,
        rows(&[], &[(gate, src)], &[(k1, gate), (k2, gate)]),
        "an empty group hands the gate's output to its sink"
    );

    save_node(
        &h,
        &alice,
        &pipeline,
        s,
        "source",
        json!({ "add": [a1, a2] }),
    )
    .await;
    let pass = json!({ "type": "null" });
    save_node(
        &h,
        &alice,
        &pipeline,
        g,
        "processor_group",
        json!({ "processors": [
        processor(p1, "one", 1, pass.clone()), processor(p2, "two", 1, pass.clone()),
    ] }),
    )
    .await;
    assert_eq!(
        runtime_rows(&h).await,
        rows(
            &[(a1, src), (a2, src)],
            &[(gate, src), (p1, gate), (p2, p1)],
            &[(k1, p2), (k2, gate)]
        )
    );

    // move the group's output to the other sink and drop an item
    let (_, read) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    let (status, body) = put_graph(
        &h,
        &alice,
        &graph_path(&pipeline),
        json!({ "last_updated": read["data"]["last_updated"], "graph": { "nodes": nodes.clone(),
                "relations": [rel(s, g), rel(g, n2), rel(s, e), rel(e, n2)] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    save_node(
        &h,
        &alice,
        &pipeline,
        s,
        "source",
        json!({ "remove": [a2] }),
    )
    .await;
    assert_eq!(
        runtime_rows(&h).await,
        rows(
            &[(a1, src)],
            &[(gate, src), (p1, gate), (p2, p1)],
            &[(k2, gate), (k2, p2)]
        )
    );

    // rows other clients added into the canvas go, a reader from outside stays
    let (foreign, reader) = (Uuid::now_v7(), Uuid::now_v7());
    sql(&h, &format!("
        INSERT INTO processors (processor_id, name, type, adm_status, last_updated, config) VALUES
            ('{foreign}', 'foreign', 'null', 1, now(), 'null'), ('{reader}', 'reader', 'null', 1, now(), 'null');
        INSERT INTO processor_inputs (processor_id, input_id) VALUES ('{p1}', '{foreign}'), ('{reader}', '{p2}');
        INSERT INTO item_processors (item_id, processor_id) VALUES ('{a2}', '{p1}');
        INSERT INTO sink_inputs (sink_id, input_id) VALUES ('{k1}', '{p1}');
    ")).await;
    let (_, read) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    let (status, _) = put_graph(
        &h,
        &alice,
        &graph_path(&pipeline),
        json!({ "last_updated": read["data"]["last_updated"], "graph": read["data"]["graph"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        runtime_rows(&h).await,
        rows(
            &[(a1, src)],
            &[(gate, src), (p1, gate), (p2, p1), (reader, p2)],
            &[(k2, gate), (k2, p2)]
        )
    );
}

/// A disabled pipeline is wired like an enabled one except for the source -> gate row,
/// so its items stay routed into the source and do not fall through to catch-all sinks.
#[tokio::test]
async fn a_disabled_pipeline_has_no_gate_row() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let host = insert_host(&h).await;
    let k = insert_sink(&h).await;
    let pipeline = create(
        &h,
        &alice,
        json!({ "name": "p", "adm_status": 0, "host_ids": [host], "sink_ids": [k] }),
    )
    .await;
    let item = insert_item(&h, Some(host), "a1").await;
    let [s, n] = std::array::from_fn(|_| Uuid::now_v7());
    let (_, read) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    let (status, body) = put_graph(
        &h,
        &alice,
        &graph_path(&pipeline),
        json!({ "last_updated": read["data"]["last_updated"], "graph": {
                "nodes": [node(s, "source", 0, 0), node(n, "sink", 0, 0)], "relations": [rel(s, n)] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    save_node(&h, &alice, &pipeline, n, "sink", json!({ "sink_id": k })).await;
    save_node(&h, &alice, &pipeline, s, "source", json!({ "add": [item] })).await;
    let (src, gate) = hidden_processors(&h, s).await;
    assert_eq!(
        runtime_rows(&h).await,
        rows(&[(item, src)], &[], &[(k, gate)])
    );
}

/// A cycle outside the canvas is not the save's to fix, so it does not block it.
/// A cycle through a row another client added into the canvas goes with that row.
#[tokio::test]
async fn cycles_outside_the_canvas_do_not_block_saves() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let pipeline = create(&h, &alice, json!({ "name": "p" })).await;
    let [s, g] = std::array::from_fn(|_| Uuid::now_v7());
    save_nodes(
        &h,
        &alice,
        &pipeline,
        vec![node(s, "source", 0, 0), node(g, "processor_group", 0, 0)],
    )
    .await;
    let (src, gate) = hidden_processors(&h, s).await;

    // two processors of the old API that read each other
    let (x, y) = (Uuid::now_v7(), Uuid::now_v7());
    sql(
        &h,
        &format!(
            "
        INSERT INTO processors (processor_id, name, type, adm_status, last_updated, config) VALUES
            ('{x}', 'x', 'null', 1, now(), 'null'), ('{y}', 'y', 'null', 1, now(), 'null');
        INSERT INTO processor_inputs (processor_id, input_id) VALUES ('{x}', '{y}'), ('{y}', '{x}');
    "
        ),
    )
    .await;

    let (_, read) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    let (status, body) = put_graph(
        &h,
        &alice,
        &graph_path(&pipeline),
        json!({ "last_updated": read["data"]["last_updated"], "graph": {
                "nodes": read["data"]["graph"]["nodes"], "relations": [rel(s, g)] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let p = Uuid::now_v7();
    save_node(
        &h,
        &alice,
        &pipeline,
        g,
        "processor_group",
        json!({ "processors": [processor(p, "pass", 1, json!({ "type": "null" }))] }),
    )
    .await;
    assert_eq!(
        runtime_rows(&h).await,
        rows(&[], &[(gate, src), (p, gate), (x, y), (y, x)], &[])
    );

    // the save drops the row another client added into the canvas, and the
    // cycle through it with it
    sql(
        &h,
        &format!("DELETE FROM processor_inputs WHERE processor_id = '{x}'; \
                  INSERT INTO processor_inputs (processor_id, input_id) VALUES ('{x}', '{p}'), ('{p}', '{x}')"),
    )
    .await;
    let (_, read) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    let (status, body) = put_graph(
        &h,
        &alice,
        &graph_path(&pipeline),
        json!({ "last_updated": read["data"]["last_updated"], "graph": read["data"]["graph"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        runtime_rows(&h).await,
        rows(&[], &[(gate, src), (p, gate), (x, p), (y, x)], &[])
    );
}

async fn put_pipeline(
    h: &Harness,
    user: &TestUser,
    pipeline: &Value,
    changes: Value,
) -> (StatusCode, Value) {
    let id = pipeline["pipeline_id"].as_str().unwrap();
    let (_, read) = get(h, user, id).await;
    let read = &read["data"];
    let mut body = json!({
        "last_updated": read["last_updated"], "name": read["name"], "visibility": read["visibility"],
        "adm_status": read["adm_status"], "host_ids": read["host_ids"], "sink_ids": read["sink_ids"],
    });
    for (key, value) in changes.as_object().unwrap() {
        body[key] = value.clone();
    }
    send(
        h.request(Method::PUT, &format!("{PATH}/{id}"), user)
            .json(&body),
    )
    .await
}

/// An update replaces the metadata and renames the hidden processors with the pipeline.
/// A stale last_updated is a 409, and bad input is a 400 that changes nothing.
#[tokio::test]
async fn update_replaces_metadata_and_renames_hidden_processors() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let pipeline = create(&h, &alice, json!({ "name": "p" })).await;
    let s = Uuid::now_v7();
    save_nodes(&h, &alice, &pipeline, vec![node(s, "source", 0, 0)]).await;

    let (status, body) = put_pipeline(&h, &alice, &pipeline, json!({ "name": "renamed" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let updated = body["data"].clone();
    assert_eq!(updated["name"], "renamed");
    assert_ne!(updated["last_updated"], pipeline["last_updated"]);
    assert_eq!(updated["created_at"], pipeline["created_at"]);
    assert_eq!(
        updated["canvases"][0]["last_updated"],
        get_graph(&h, &alice, &graph_path(&pipeline)).await.1["data"]["last_updated"],
        "the canvas keeps its own time"
    );
    let (src, gate) = hidden_processors(&h, s).await;
    assert_eq!(
        count(&h, &format!("SELECT count(*) FROM processors WHERE (processor_id = '{src}' AND name = 'renamed (source)') \
                            OR (processor_id = '{gate}' AND name = 'renamed (gate)')")).await,
        2
    );

    let id = pipeline["pipeline_id"].as_str().unwrap();
    let (status, body) = send(h.request(Method::PUT, &format!("{PATH}/{id}"), &alice).json(&json!({
        "last_updated": pipeline["last_updated"], "name": "stale", "visibility": 0, "adm_status": 1,
        "host_ids": [], "sink_ids": [],
    }))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["title"], "Pipeline changed");

    let unknown = Uuid::now_v7();
    for (changes, detail) in [
        (json!({ "name": " " }), "Name cannot be empty".to_string()),
        (
            json!({ "host_ids": [unknown] }),
            format!("Host IDs not found: {unknown}"),
        ),
        (
            json!({ "sink_ids": [unknown] }),
            format!("Sink IDs not found: {unknown}"),
        ),
    ] {
        let (status, body) = put_pipeline(&h, &alice, &pipeline, changes).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"]["detail"], detail);
    }
    assert_eq!(get(&h, &alice, id).await.1["data"], updated);
}

/// Disabling removes only the source -> gate row, enabling puts it back.
/// Items stay routed into the source the whole time.
#[tokio::test]
async fn disable_and_enable_toggle_the_gate_row() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let host = insert_host(&h).await;
    let k = insert_sink(&h).await;
    let pipeline = create(
        &h,
        &alice,
        json!({ "name": "p", "host_ids": [host], "sink_ids": [k] }),
    )
    .await;
    let item = insert_item(&h, Some(host), "a1").await;
    let [s, n] = std::array::from_fn(|_| Uuid::now_v7());
    let (_, read) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    let (status, _) = put_graph(
        &h,
        &alice,
        &graph_path(&pipeline),
        json!({ "last_updated": read["data"]["last_updated"], "graph": {
                "nodes": [node(s, "source", 0, 0), node(n, "sink", 0, 0)], "relations": [rel(s, n)] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    save_node(&h, &alice, &pipeline, n, "sink", json!({ "sink_id": k })).await;
    save_node(&h, &alice, &pipeline, s, "source", json!({ "add": [item] })).await;
    let (src, gate) = hidden_processors(&h, s).await;
    let enabled = rows(&[(item, src)], &[(gate, src)], &[(k, gate)]);
    assert_eq!(runtime_rows(&h).await, enabled);

    let (status, body) = put_pipeline(&h, &alice, &pipeline, json!({ "adm_status": 0 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["adm_status"], 0);
    assert_eq!(
        runtime_rows(&h).await,
        rows(&[(item, src)], &[], &[(k, gate)])
    );

    let (status, _) = put_pipeline(&h, &alice, &pipeline, json!({ "adm_status": 1 })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(runtime_rows(&h).await, enabled);
}

/// Anyone may edit a public pipeline, but only its owner may change who sees it.
/// A pipeline without an owner stays public.
#[tokio::test]
async fn only_the_owner_changes_visibility() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let bob = h.user("bob").await;
    let public = create(&h, &alice, json!({ "name": "public", "visibility": 1 })).await;

    let (status, body) = put_pipeline(&h, &bob, &public, json!({ "name": "by bob" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = put_pipeline(&h, &bob, &public, json!({ "visibility": 0 })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["title"], "Not the owner");
    let (status, body) = put_pipeline(&h, &alice, &public, json!({ "visibility": 0 })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = public["pipeline_id"].as_str().unwrap();
    assert_eq!(get(&h, &bob, id).await.0, StatusCode::NOT_FOUND);

    sql(
        &h,
        &format!(
            "UPDATE pipelines SET visibility = 1 WHERE pipeline_id = '{id}'; \
                      DELETE FROM users WHERE user_id = '{}';",
            alice.user_id
        ),
    )
    .await;
    let (status, _) = put_pipeline(&h, &bob, &public, json!({ "visibility": 0 })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = put_pipeline(&h, &bob, &public, json!({ "name": "anonymous" })).await;
    assert_eq!(status, StatusCode::OK);
}

/// Dropping a host or sink still in use is a 409 that says which nodes and items use it.
/// With force the items leave their source and the sink nodes are emptied, which the runtime follows.
#[tokio::test]
async fn shrinking_subsets_in_use_needs_force() {
    let h = Harness::start().await;
    let alice = h.user("alice").await;
    let (host_a, host_b) = (insert_host(&h).await, insert_host(&h).await);
    let (k1, k2) = (insert_sink(&h).await, insert_sink(&h).await);
    let pipeline = create(
        &h,
        &alice,
        json!({ "name": "p", "host_ids": [host_a, host_b], "sink_ids": [k1, k2] }),
    )
    .await;
    let (on_a, on_b) = (
        insert_item(&h, Some(host_a), "a").await,
        insert_item(&h, Some(host_b), "b").await,
    );
    let [s, n1, n2] = std::array::from_fn(|_| Uuid::now_v7());
    let (_, read) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    let (status, _) = put_graph(
        &h,
        &alice,
        &graph_path(&pipeline),
        json!({ "last_updated": read["data"]["last_updated"], "graph": {
                "nodes": [node(s, "source", 0, 0), node(n1, "sink", 0, 0), node(n2, "sink", 0, 0)],
                "relations": [rel(s, n1), rel(s, n2)] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let n1_read = save_node(&h, &alice, &pipeline, n1, "sink", json!({ "sink_id": k1 })).await;
    save_node(&h, &alice, &pipeline, n2, "sink", json!({ "sink_id": k2 })).await;
    let source_read = save_node(
        &h,
        &alice,
        &pipeline,
        s,
        "source",
        json!({ "add": [on_a, on_b] }),
    )
    .await;
    let (_, graph_before) = get_graph(&h, &alice, &graph_path(&pipeline)).await;

    let drop = json!({ "host_ids": [host_b], "sink_ids": [k2] });
    let (status, body) = put_pipeline(&h, &alice, &pipeline, drop.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["title"], "Subset in use");
    let mut node_ids = vec![s, n1];
    node_ids.sort();
    assert_eq!(
        body["error"]["data"],
        json!({ "node_ids": node_ids, "item_ids": [on_a], "item_count": 1 })
    );

    let mut forced = drop.clone();
    forced["force"] = json!(true);
    let (status, body) = put_pipeline(&h, &alice, &pipeline, forced).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["host_ids"], json!([host_b]));
    assert_eq!(body["data"]["sink_ids"], json!([k2]));

    // the source lost the item and the sink node its sink, both moved on; the
    // canvas keeps its nodes and relations
    let source = get_node(&h, &alice, &node_path(&pipeline, &json!(s))).await;
    assert_eq!(source["data"]["total"], 1);
    assert_ne!(source["last_updated"], source_read["last_updated"]);
    let n1_now = get_node(&h, &alice, &node_path(&pipeline, &json!(n1))).await;
    assert_eq!(n1_now["data"], Value::Null);
    assert_ne!(n1_now["last_updated"], n1_read["last_updated"]);
    let (_, graph) = get_graph(&h, &alice, &graph_path(&pipeline)).await;
    assert_eq!(
        graph["data"]["last_updated"],
        graph_before["data"]["last_updated"]
    );
    assert_eq!(
        graph["data"]["graph"]["relations"],
        graph_before["data"]["graph"]["relations"]
    );
    let (src, gate) = hidden_processors(&h, s).await;
    assert_eq!(
        runtime_rows(&h).await,
        rows(&[(on_b, src)], &[(gate, src)], &[(k2, gate)])
    );
}
