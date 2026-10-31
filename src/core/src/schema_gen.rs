// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use patrolli::api::ApiDoc;
use utoipa::OpenApi;

pub fn main() {
    let doc = ApiDoc::openapi().to_pretty_json().unwrap();
    std::fs::write("openapi.json", doc).unwrap();
}
