// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use axum::http::StatusCode;
use serde::Serialize;

use axum::Json;
use axum::response::{IntoResponse, Response};
use utoipa::ToSchema;

/// Successful response
#[derive(Serialize, Debug, ToSchema)]
pub struct ResponseData<T> {
    #[serde(with = "http_serde::status_code")]
    #[schema(value_type = u16, example = 200)]
    pub status_code: StatusCode,
    pub success: bool,
    pub data: Option<T>,
}

impl<T> ResponseData<T> {
    /// Initialize with a status code
    pub fn from_status(status: StatusCode) -> Self {
        Self {
            status_code: status,
            success: true,
            data: None,
        }
    }

    /// Attach data to the response
    pub fn with_data(mut self, data: T) -> Self {
        self.data = Some(data);
        self
    }
}

#[derive(Serialize, Debug, ToSchema)]
pub struct ResponseError {
    #[serde(with = "http_serde::status_code")]
    #[schema(value_type = u16, example = 418)]
    pub status_code: StatusCode,
    pub success: bool,
    pub error: ErrorBody,
}

#[derive(Serialize, Debug, Default, ToSchema)]
pub struct ErrorBody {
    #[schema(example = "An error occurred")]
    pub title: String,
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl ResponseError {
    /// Initialize with a status code
    pub fn from_status(status: StatusCode) -> Self {
        Self {
            status_code: status,
            success: false,
            error: ErrorBody::default(),
        }
    }

    /// Set a short, human-readable title for the error
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.error.title = title.into();
        self
    }

    /// Set a detailed explanation of what went wrong
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.error.detail = Some(detail.into());
        self
    }

    /// Attach additional data to the error response
    pub fn with_data(mut self, data: impl Serialize) -> Self {
        self.error.data = Some(serde_json::to_value(data).unwrap_or_else(|_| {
            serde_json::json!({
                "error": "Failed to serialize error data"
            })
        }));
        self
    }

    pub fn to_json_value(self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|_| {
            serde_json::json!({
                "status_code": 500,
                "success": false,
                "error": {
                    "title": "Internal Server Error",
                    "detail": "Failed to serialize error response"
                }
            })
        })
    }

    pub fn to_json_string(self) -> String {
        self.to_json_value().to_string()
    }
}

impl<T> IntoResponse for ResponseData<T>
where
    T: Serialize,
{
    fn into_response(self) -> Response {
        (self.status_code, Json(self)).into_response()
    }
}

impl IntoResponse for ResponseError {
    fn into_response(self) -> Response {
        (self.status_code, Json(self)).into_response()
    }
}
