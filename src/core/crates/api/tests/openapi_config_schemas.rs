// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use serde_json::{Value, json};
use utoipa::OpenApi;

#[test]
fn openapi_exposes_the_transform_config_and_its_comments() {
    let api = serde_json::to_value(patrolli_api::ApiDoc::openapi()).unwrap();
    let transform = &api["components"]["schemas"]["Transform"];
    assert!(!transform["description"].as_str().unwrap().is_empty());
    let opl = &transform["oneOf"][0];
    assert!(
        !opl["properties"]["opl"]["description"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    assert_eq!(opl["required"], serde_json::json!(["opl"]));
}

#[test]
fn openapi_exposes_the_lookup_config_and_its_field_comments() {
    let api = serde_json::to_value(patrolli_api::ApiDoc::openapi()).unwrap();
    let schemas = &api["components"]["schemas"];
    let lookup = &schemas["AttributeLookup"];
    assert_eq!(lookup["properties"]["overwrite"]["default"], json!(false));
    // Presence, not wording, as `attribute_collect`'s twin does: the point
    // is that the doc comments reach the schema, and a phrase match breaks
    // on any edit to them.
    assert!(!lookup["description"].as_str().unwrap().is_empty());
    for (field, property) in lookup["properties"].as_object().unwrap() {
        // An optional referenced type keeps its comment on the non-null
        // branch of the oneOf, not on the surrounding property.
        let description = property.get("description").or_else(|| {
            property
                .get("oneOf")?
                .as_array()?
                .iter()
                .find_map(|variant| variant.get("description"))
        });
        assert!(
            description
                .and_then(serde_json::Value::as_str)
                .is_some_and(|s| !s.is_empty()),
            "AttributeLookup.{field}"
        );
    }
    let rule = &schemas["AttributeLookupRule"]["properties"];
    assert!(rule.get("match").is_some());
    assert!(rule.get("match_by").is_none());
    for name in ["AttributeLookup", "AttributeLookupRule"] {
        assert!(schemas[name]["properties"].get("when").is_some());
        assert!(
            !schemas[name]["required"]
                .as_array()
                .unwrap()
                .contains(&json!("when"))
        );
    }
    assert!(schemas.get("AttributeLookupInput").is_some());
    assert!(schemas.get("AttributeLookupCast").is_some());
}

#[test]
fn openapi_exposes_the_collect_config_and_its_field_comments() {
    let api = serde_json::to_value(patrolli_api::ApiDoc::openapi()).unwrap();
    let schemas = &api["components"]["schemas"];
    for name in [
        "AttributeCollect",
        "AttributeCollectRule",
        "AttributeCollectInput",
        "AttributeCollectPivot",
        "SnmpObjectFormat",
        "RegexExtractFormat",
    ] {
        assert!(
            schemas[name]["description"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "{name}"
        );
        assert_eq!(schemas[name]["additionalProperties"], json!(false));
        for (field, property) in schemas[name]["properties"].as_object().unwrap() {
            // Optional referenced types keep their field comment on the
            // non-null branch of oneOf, not on the surrounding property.
            let description = property.get("description").or_else(|| {
                property
                    .get("oneOf")?
                    .as_array()?
                    .iter()
                    .find_map(|variant| variant.get("description"))
            });
            assert!(
                description
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty()),
                "{name}.{field}"
            );
        }
    }
    assert_eq!(
        schemas["AttributeCollect"]["required"],
        json!(["collections"])
    );
    assert_eq!(
        schemas["AttributeCollectRule"]["required"],
        json!(["lookup_table", "table", "columns"])
    );
    // Every binding field is optional now: a bare `{}` is a pivoted binding.
    assert!(
        schemas["AttributeCollectInput"]
            .get("required")
            .is_none_or(|r| r.as_array().is_some_and(Vec::is_empty)),
        "{}",
        schemas["AttributeCollectInput"]
    );
    assert_eq!(
        schemas["AttributeCollectPivot"]["required"],
        json!(["array", "name", "value"])
    );
    assert_eq!(
        schemas["RegexExtractFormat"]["required"],
        json!(["pattern"])
    );
    assert_eq!(
        schemas["SnmpObjectFormat"]["properties"]["strip_instance"]["default"],
        json!(true)
    );
    assert_eq!(
        schemas["RegexExtractFormat"]["properties"]["group"]["default"],
        json!(1)
    );
    assert!(schemas.get("NameFormat").is_some());
    assert!(schemas.get("AttributeCollectCast").is_some());
    assert!(
        schemas["AttributeCollect"]["properties"]
            .get("overwrite")
            .is_none()
    );
}
