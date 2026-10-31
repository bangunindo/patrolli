// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod error;
pub mod handlers;
pub mod jwt;
pub mod response;
pub mod state;
use utoipa::{
    Modify,
    openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
};

use crate::api::handlers::{
    OrderByDirection, auth, endpoints, health, hosts, items, listeners, nodes, pipelines,
    processor_builder, processors, sinks, templates, users,
};
use crate::api::jwt::require_auth;
use crate::api::state::AppState;
use crate::config::cluster::CorsConfig;
use axum::extract::MatchedPath;
use axum::http::{Method, Request, header};
use axum::{
    Router, middleware,
    response::Response,
    routing::{any, delete, get, patch, post, put},
};
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tower_http::{compression::CompressionLayer, decompression::RequestDecompressionLayer};
use tracing::{Span, debug, field::Empty, info, info_span};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;
use uuid::Uuid;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = &mut openapi.components {
            components.add_security_scheme(
                "bearerAuth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    nest(
        (path = "/api/v1/auth", api=auth::AuthHandler),
        (path = "/health", api=health::HealthHandler),
        (path = "/api/v1", api=templates::TemplatesHandler),
        (path = "/api/v1", api=items::ItemsHandler),
        (path = "/api/v1", api=processors::ProcessorsHandler),
        (path = "/api/v1", api=hosts::HostsHandler),
        (path = "/api/v1", api=nodes::NodesHandler),
        (path = "/api/v1", api=endpoints::EndpointsHandler),
        (path = "/api/v1", api=sinks::SinksHandler),
        (path = "/api/v1", api=listeners::ListenersHandler),
        (path = "/api/v1", api=processor_builder::ProcessorBuilderHandler),
        (path = "/api/v1", api=users::UsersHandler),
        (path = "/api/v1", api=pipelines::PipelinesHandler),
    ),
    components(
        schemas(
            OrderByDirection,
        )
    ),
    tags(
        (name = "auth", description = "Authentication endpoints"),
        (name = "health", description = "Health check endpoints"),
        (name = "hosts", description = "Host management endpoints"),
        (name = "nodes", description = "Cluster node management endpoints"),
        (name = "items", description = "Item management endpoints"),
        (name = "templates", description = "Template management endpoints"),
        (name = "users", description = "User management endpoints"),
        (name = "pipelines", description = "Pipeline canvas endpoints"),
    ),
    info(
        title = "Patrolli API",
        description = "RESTful API for Patrolli monitoring system",
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

/// The route template a request matched, e.g. `/api/v1/items/{item_id}`.
///
/// Deliberately not the URI: a raw path carries ids, which would hand the throttle layer a fresh
/// signature per request and blow its LRU. Falls back to the path when nothing matched (404s, and
/// any position where routing has not run yet) — see the test at the bottom of this file, which
/// pins the fact that a layer added with `Router::layer` does see `MatchedPath`.
fn matched_route<B>(req: &Request<B>) -> &str {
    req.extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or_else(|| req.uri().path())
}

/// Per-request span every API log line inherits.
///
/// `user` starts empty and is recorded by [`jwt::require_auth`] once the token verifies.
/// `request_id` is excluded from the throttle signature in `init_logging`.
fn request_span<B>(req: &Request<B>) -> Span {
    info_span!(
        target: "patrolli::api",
        "request",
        request_id = %Uuid::now_v7(),
        method = %req.method(),
        route = %matched_route(req),
        user = Empty,
    )
}

/// Audit trail for successful writes under `/api/v1`, outside `/api/v1/auth`.
///
/// One event per mutation instead of one per handler; `method`, `route`, `user` and
/// `request_id` come from [`request_span`]. Only observes the response.
async fn audit_mutation(req: axum::extract::Request, next: middleware::Next) -> Response {
    let is_mutation = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS)
        && req.uri().path().starts_with("/api/v1/")
        && !req.uri().path().starts_with("/api/v1/auth/");
    let res = next.run(req).await;
    if is_mutation && res.status().is_success() {
        info!(
            target: "patrolli::api::audit",
            status = res.status().as_u16(),
            "config mutated"
        );
    }
    res
}

/// Creates the main API router with all routes configured
// deprecated endpoints are still routed until their clients migrate
#[allow(deprecated)]
pub fn create_router(state: AppState, cors_config: &Option<CorsConfig>) -> Router {
    let state = Arc::new(state);

    // API routes
    let mut api_routes = Router::new()
        // Nodes
        .route("/nodes", get(nodes::list_nodes))
        // Hosts
        .route("/hosts", get(hosts::list_hosts))
        .route("/hosts", post(hosts::create_host))
        .route("/hosts/{id}", put(hosts::update_host))
        .route("/hosts/{id}", delete(hosts::delete_host))
        // Templates
        .route("/templates", get(templates::list_templates))
        .route("/templates/{id}", get(templates::get_template))
        .route("/templates", post(templates::create_template))
        .route("/templates/{id}", put(templates::update_template))
        .route("/templates/{id}", delete(templates::delete_template))
        .route("/templates/link/{id}", put(templates::link_template))
        .route("/templates/unlink/{id}", put(templates::unlink_template))
        .route("/templates/apply", post(templates::apply_template))
        .route("/templates/unapply", post(templates::unapply_template))
        // Items
        .route("/items", get(items::list_items))
        .route(
            "/items/template/{template_id}",
            post(items::create_template_item),
        )
        .route("/items/host/{host_id}", post(items::create_host_item))
        .route("/items/{item_id}", get(items::get_item))
        .route("/items/{item_id}", put(items::update_item))
        .route("/items/{item_id}/host", put(items::update_host_item))
        .route(
            "/items/{item_id}/template",
            put(items::update_template_item),
        )
        .route(
            "/items/{item_id}/applied-template",
            put(items::update_applied_template_item),
        )
        .route("/items/{item_id}", delete(items::delete_item))
        // Endpoints
        .route("/endpoints", get(endpoints::list_endpoints))
        .route("/endpoints", post(endpoints::create_endpoint))
        .route("/endpoints/{endpoint_id}", put(endpoints::update_endpoint))
        .route(
            "/endpoints/{endpoint_id}",
            delete(endpoints::delete_endpoint),
        )
        // Listeners
        .route("/listeners", get(listeners::list_listeners))
        .route("/listeners", post(listeners::create_listener))
        .route("/listeners/{listener_id}", put(listeners::update_listener))
        .route(
            "/listeners/{listener_id}",
            delete(listeners::delete_listener),
        )
        // Sinks
        .route("/sinks", get(sinks::list_sinks))
        .route("/sinks", post(sinks::create_sink))
        .route("/sinks/{sink_id}", get(sinks::get_sink))
        .route("/sinks/{sink_id}", put(sinks::update_sink))
        .route("/sinks/{sink_id}", delete(sinks::delete_sink))
        // Processor
        .route("/processors", get(processors::list_processors))
        .route("/processors", post(processors::create_processor))
        .route(
            "/processors/{processor_id}",
            put(processors::update_processor),
        )
        .route(
            "/processors/{processor_id}",
            delete(processors::delete_processor),
        )
        // Processor Builder
        .route(
            "/processor_builder/graph",
            get(processor_builder::get_graph),
        )
        // Pipelines
        .route("/pipelines", get(pipelines::list_pipelines))
        .route("/pipelines", post(pipelines::create_pipeline))
        .route("/pipelines/{pipeline_id}", get(pipelines::get_pipeline))
        .route(
            "/pipelines/{pipeline_id}",
            delete(pipelines::delete_pipeline),
        )
        .route("/pipelines/{pipeline_id}", put(pipelines::update_pipeline))
        .route(
            "/pipelines/{pipeline_id}/canvases/{canvas_id}/graph",
            get(pipelines::get_pipeline_graph),
        )
        .route(
            "/pipelines/{pipeline_id}/canvases/{canvas_id}/graph",
            put(pipelines::update_pipeline_graph),
        )
        .route(
            "/pipelines/{pipeline_id}/canvases/{canvas_id}/nodes/{node_id}",
            get(pipelines::get_pipeline_node),
        )
        .route(
            "/pipelines/{pipeline_id}/canvases/{canvas_id}/nodes/{node_id}",
            put(pipelines::update_pipeline_node),
        )
        // Users
        .route("/users", get(users::list_users))
        .route("/users", post(users::create_user))
        .route("/users/summary", get(users::get_user_summary))
        .route("/users/{user_id}", get(users::get_user))
        .route("/users/{user_id}", patch(users::update_user))
        .route("/users/{user_id}", delete(users::delete_user))
        .route("/users/{user_id}/unlink-sso", post(users::unlink_sso));

    api_routes = api_routes.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_auth,
    ));

    // WebSocket route for subscribing to capture events
    let mut ws_routes = Router::new();
    ws_routes = ws_routes.route(
        "/processors/subscribe_capture/{processor_id}",
        any(processors::subscribe_capture::ws_subscribe_capture),
    );
    ws_routes = ws_routes.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_auth,
    ));

    // Auth routes
    let auth_routes = Router::new()
        .route("/login", post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/refresh", post(auth::refresh));

    let mut auth_protected_routes =
        Router::new().route("/change-password", post(auth::change_password));

    auth_protected_routes = auth_protected_routes.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_auth,
    ));

    // CORS configuration
    let mut cors = CorsLayer::new()
        .allow_headers(vec![
            header::ACCEPT,
            header::ACCEPT_LANGUAGE,
            header::AUTHORIZATION,
            header::CONTENT_LANGUAGE,
            header::CONTENT_TYPE,
        ])
        .allow_methods(vec![
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
            Method::PATCH,
        ])
        .allow_credentials(true)
        .expose_headers(vec![])
        .max_age(std::time::Duration::from_secs(60 * 60));

    if let Some(cors_config) = cors_config {
        cors = cors.allow_origin(cors_config.allow_origins.clone());
    }

    // Main router with all components
    Router::new()
        .route("/health", get(health::health_check))
        .nest("/api/v1/auth", auth_routes)
        .nest("/api/v1/auth", auth_protected_routes)
        .nest("/api/v1", api_routes)
        .nest("/ws/v1", ws_routes)
        // Swagger UI
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        // Add state
        .with_state(state)
        // Add middleware
        .layer(cors)
        // Inside the trace layer, so the audit event inherits the request span.
        .layer(middleware::from_fn(audit_mutation))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(request_span)
                .on_request(())
                // `.ctx()` already logs every 5xx once; tower-http's default would log it again.
                .on_failure(())
                .on_response(
                    |res: &axum::http::Response<_>, latency: Duration, _: &Span| {
                        debug!(
                            target: "patrolli::api",
                            status = res.status().as_u16(),
                            latency_ms = latency.as_millis(),
                            "request completed"
                        );
                    },
                ),
        )
        .layer(RequestDecompressionLayer::new())
        .layer(CompressionLayer::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::routing::get;
    use std::sync::{Arc, Mutex};

    /// Pins the assumption the request span rests on: a layer added with `Router::layer` runs
    /// *outside* routing, so it is not obvious that `MatchedPath` is in the extensions yet. If a
    /// future axum changes that, `route` silently degrades to the raw URI and every request id in
    /// the path becomes its own throttle signature — so this asserts the template, not the path.
    #[tokio::test]
    async fn the_request_span_sees_the_route_template_not_the_uri() {
        let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let sink = seen.clone();

        let app = Router::new()
            .route("/api/items/{item_id}", get(|| async { "ok" }))
            // mounted exactly where `TraceLayer` is in `create_router`
            .layer(axum::middleware::from_fn(
                move |req: Request<axum::body::Body>, next: axum::middleware::Next| {
                    let sink = sink.clone();
                    async move {
                        *sink.lock().unwrap() = Some(matched_route(&req).to_string());
                        next.run(req).await
                    }
                },
            ));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let body = reqwest::get(format!(
            "http://{addr}/api/items/0199a0e1-dead-beef-cafe-000000000000"
        ))
        .await
        .unwrap();
        assert_eq!(body.status(), 200);
        server.abort();

        assert_eq!(
            seen.lock().unwrap().as_deref(),
            Some("/api/items/{item_id}"),
            "route must be the low-cardinality template, not the URI with the id in it"
        );
    }

    #[test]
    fn openapi_doc_builds_without_overflow() {
        // Regression: recursive ToSchema types (e.g. AttributeFilter) used to
        // stack-overflow during openapi() generation.
        let doc = ApiDoc::openapi();
        assert!(!doc.paths.paths.is_empty());
    }

    /// `info(version)` is left out so utoipa fills it from Cargo; a literal there goes stale.
    #[test]
    fn api_doc_version_is_the_crates() {
        assert_eq!(ApiDoc::openapi().info.version, env!("CARGO_PKG_VERSION"));
    }
}
