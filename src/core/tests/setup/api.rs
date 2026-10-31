// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use deadpool_postgres::Pool;
use patrolli::api::jwt::Jwt;
use patrolli::api::state::AppState;
use patrolli::cluster::inter::InterComm;
use patrolli::{api, cache, config};
use reqwest::{Method, RequestBuilder, StatusCode};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::{TestDb, secret_keys, setup_db, setup_logging};

/// One API server on its own Postgres. Every test starts its own, so tests
/// share no rows and can run in parallel.
pub struct Harness {
    base_url: String,
    http: reqwest::Client,
    jwt: Arc<Jwt>,
    pub pool: Pool,
    server: JoinHandle<()>,
    // dropped last: drops the test database (or its Postgres container)
    _db: TestDb,
}

pub struct TestUser {
    pub user_id: Uuid,
    token: String,
}

impl Harness {
    pub async fn start() -> Harness {
        setup_logging();
        let (db, pool) = setup_db().await;

        let jwt = Arc::new(Jwt::new(
            "pipeline-management-test-secret-0123456789",
            "patrolli-test".to_string(),
            Duration::from_secs(3600),
            Duration::from_secs(3600),
        ));
        // capture and forwarding are not under test; the sender is dropped so
        // the forwarder task ends right away
        let (_, forward_receiver) = flume::unbounded();
        let inter_comm = InterComm::new(
            forward_receiver,
            config::cluster::InterNode::default(),
            Uuid::now_v7(),
            "test-node".to_string(),
            cache::Cache::new(),
        )
        .expect("failed to create inter comm");
        let state = AppState::new(
            pool.clone(),
            cache::Cache::new(),
            inter_comm,
            jwt.clone(),
            secret_keys(),
        );
        let router = api::create_router(state, &None);

        // bound before the server task starts, so requests can go out right away
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind api listener");
        let addr = listener.local_addr().expect("failed to read api address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("api server failed");
        });

        Harness {
            base_url: format!("http://{addr}"),
            http: reqwest::Client::new(),
            jwt,
            pool,
            server,
            _db: db,
        }
    }

    /// Inserts a user and signs an access token for it.
    pub async fn user(&self, username: &str) -> TestUser {
        let user_id = Uuid::now_v7();
        let client = self.pool.get().await.expect("failed to get connection");
        client
            .execute(
                "INSERT INTO users (user_id, username) VALUES ($1, $2)",
                &[&user_id, &username],
            )
            .await
            .expect("failed to insert user");
        let token = self
            .jwt
            .sign_access(user_id, username)
            .expect("failed to sign token");
        TestUser { user_id, token }
    }

    /// A request to `path` as `user`.
    pub fn request(&self, method: Method, path: &str, user: &TestUser) -> RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.base_url, path))
            .bearer_auth(&user.token)
    }
}

/// Sends a request and returns its status and body. An empty body reads as
/// `Null`, a body that is not JSON as a `String`.
pub async fn send(request: RequestBuilder) -> (StatusCode, serde_json::Value) {
    let response = request.send().await.expect("request failed");
    let status = response.status();
    let body = response.text().await.expect("failed to read response body");
    let body = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&body).unwrap_or(serde_json::Value::String(body))
    };
    (status, body)
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}
