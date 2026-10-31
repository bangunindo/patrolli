// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Error conversion for handlers.
//!
//! Handlers do not log. They attach a short operation label with [`ApiResultExt::ctx`] (or
//! [`ApiResultExt::bad_request`] for caller mistakes) and let the conversion emit exactly one
//! event. The per-request span installed in [`crate::api::create_router`] supplies `request_id`,
//! `method`, `route` and `user_id`, so the event itself carries only `op` and `error`.
//!
//! Infrastructure failures deliberately return a generic title: pool state, SQL and serde detail
//! belong in the log, not in the response body.

use axum::http::StatusCode;
use tracing::{debug, error};

use crate::api::response::ResponseError;

/// Title returned for every infrastructure failure. The operator detail is in the log, keyed by
/// the `request_id` of the failing request.
const INTERNAL_TITLE: &str = "Internal server error";

pub trait ApiResultExt<T> {
    /// Log the error once at `ERROR` with `op`, and return a generic 500.
    ///
    /// `op` is a short, constant label for the operation that failed — `"db connect"`,
    /// `"query host"`, `"insert endpoint"`. It is a field, so keep it low-cardinality.
    fn ctx(self, op: &'static str) -> Result<T, ResponseError>;

    /// Log the error once at `DEBUG` and return a 400 carrying the error as `detail`.
    ///
    /// For caller mistakes — a failed `validate()`, an unparseable field. Not operator-actionable,
    /// so it is not an `ERROR`.
    fn bad_request(self, title: &'static str) -> Result<T, ResponseError>;
}

/// Appends an error's `source` chain to its own message.
fn chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut cur = e.source();
    while let Some(src) = cur {
        out.push_str(": ");
        out.push_str(&src.to_string());
        cur = src.source();
    }
    out
}

/// Renders an error for the log.
///
/// Most types say everything in `Display`, but two of the ones handlers see constantly do not:
/// `tokio_postgres::Error` displays as literally `"db error"` and keeps the constraint name, the
/// failing column and the server's message in the `DbError` behind `source()`, and `PoolError`
/// wraps it. Those are downcast so the operator gets the part that matters. `anyhow::Error` renders
/// its own chain under `{:#}`.
///
/// A blanket impl per error type is not possible here — coherence rejects
/// `E: std::error::Error` alongside `anyhow::Error` (which deliberately does not implement it), and
/// the `Box<dyn Error>` produced by the row-parsing collects does not implement it either. One
/// `Display` bound plus these downcasts keeps a single `.ctx()` for every call site.
fn render<E: std::fmt::Display + 'static>(e: &E) -> String {
    let any = e as &dyn std::any::Any;
    if let Some(e) = any.downcast_ref::<tokio_postgres::Error>() {
        return chain(e);
    }
    if let Some(e) = any.downcast_ref::<deadpool_postgres::PoolError>() {
        return chain(e);
    }
    if let Some(e) = any.downcast_ref::<anyhow::Error>() {
        return format!("{e:#}");
    }
    e.to_string()
}

impl<T, E> ApiResultExt<T> for Result<T, E>
where
    E: std::fmt::Display + 'static,
{
    fn ctx(self, op: &'static str) -> Result<T, ResponseError> {
        self.map_err(|e| {
            error!(target: "patrolli::api", op, error = %render(&e), "api operation failed");
            ResponseError::from_status(StatusCode::INTERNAL_SERVER_ERROR).with_title(INTERNAL_TITLE)
        })
    }

    fn bad_request(self, title: &'static str) -> Result<T, ResponseError> {
        self.map_err(|e| {
            let detail = render(&e);
            debug!(target: "patrolli::api", error = %detail, "request rejected");
            ResponseError::from_status(StatusCode::BAD_REQUEST)
                .with_title(title)
                .with_detail(detail)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Inner;
    impl std::fmt::Display for Inner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("inner cause")
        }
    }
    impl std::error::Error for Inner {}

    #[derive(Debug)]
    struct Outer(Inner);
    impl std::fmt::Display for Outer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("outer")
        }
    }
    impl std::error::Error for Outer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn chain_walks_sources() {
        assert_eq!(chain(&Outer(Inner)), "outer: inner cause");
    }

    #[test]
    fn render_expands_anyhow_chain() {
        let e = anyhow::anyhow!("root").context("while doing a thing");
        assert_eq!(render(&e), "while doing a thing: root");
    }

    #[test]
    fn render_falls_back_to_display() {
        assert_eq!(render(&"plain message"), "plain message");
    }

    #[test]
    fn bad_request_carries_the_error_as_detail() {
        let err = Err::<(), _>(anyhow::anyhow!("name is empty"))
            .bad_request("Validation error")
            .unwrap_err();
        assert_eq!(err.status_code, StatusCode::BAD_REQUEST);
        assert_eq!(err.error.title, "Validation error");
        assert_eq!(err.error.detail.as_deref(), Some("name is empty"));
    }

    #[test]
    fn ctx_hides_internals_from_the_client() {
        let err = Err::<(), _>(anyhow::anyhow!("password=hunter2 in connection string"))
            .ctx("db connect")
            .unwrap_err();
        assert_eq!(err.status_code, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.error.title, INTERNAL_TITLE);
        assert_eq!(err.error.detail, None);
    }
}
