// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Configuration for the `transform` processor: a program in one of the
//! query languages otel-arrow's columnar engine runs (OPL, the OpenTelemetry
//! Processing Language, for now); `processor::transform` executes it. Every
//! language compiles to the same expression tree. Validation parses the
//! program, so a syntax error fails here with its position. What the engine
//! cannot plan is only known once it sees a batch: that fails per batch, at
//! run time.

use anyhow::{Result, anyhow, bail};
use otel_arrow_contrib_data_engine_expressions::{DataExpression, PipelineExpression};
use otel_arrow_contrib_data_engine_parser_abstractions::{Parser, ParserError};
use otel_arrow_dfe_query_engine::parser::default_parser_options;
use otel_arrow_dfe_query_engine::pipeline::SignalContext;
use otel_arrow_dfe_query_engine_languages::opl::parser::OplParser;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Transform telemetry with a program, keyed by its language (one key, e.g.
/// `opl: ...`). OPL is the only language so far.
///
/// Batches of a signal the program does not select pass through unchanged. A
/// batch the engine fails on is dropped and counted as errored. `route_to` is
/// not supported: route with processor inputs instead.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Transform {
    /// An OPL program: the signals it applies to (`logs`, `traces`, `metrics`,
    /// `signals`, or one metric type such as `gauges`) followed by
    /// `|`-separated operators (`where`, `drop`, `set`, `rename`, `remove`,
    /// `apply`, `if`/`else`), e.g.
    /// `logs | where severity_number >= 17 | set attributes["critical"] = true`.
    Opl(String),
}

impl Transform {
    pub fn validate(&self) -> Result<()> {
        self.compile().map(|_| ())
    }

    /// Parse the program into the engine's expression tree and the signals it
    /// selects.
    pub(crate) fn compile(&self) -> Result<(PipelineExpression, SignalContext)> {
        let expr = match self {
            Self::Opl(program) => {
                OplParser::parse_with_options(program, default_parser_options())
                    .map_err(|errors| anyhow!("invalid OPL: {}", render_parse_errors(&errors)))?
                    .pipeline
            }
        };
        if let Some(location) = find_route_to(expr.get_expressions()) {
            bail!("{location}: route_to is not supported, route with processor inputs");
        }
        let signals =
            SignalContext::try_infer(&expr).map_err(|e| anyhow!("invalid program: {e}"))?;
        Ok((expr, signals))
    }
}

fn render_parse_errors(errors: &[ParserError]) -> String {
    let rendered: Vec<String> = errors
        .iter()
        .map(|error| {
            let location = match error {
                ParserError::SyntaxNotSupported(location, _)
                | ParserError::SyntaxError(location, _)
                | ParserError::QueryLanguageDiagnostic { location, .. }
                | ParserError::KeyNotFound { location, .. } => Some(location),
                ParserError::SchemaError(_) => None,
            };
            match location.map(|l| l.get_line_and_column_numbers()) {
                // A fake location (no position) reports line 0.
                Some((line, column)) if line > 0 => {
                    format!("line {line}, column {column}: {error}")
                }
                _ => error.to_string(),
            }
        })
        .collect();
    rendered.join("; ")
}

/// The position of the first `route_to`, searching `if`/`else` branches too.
fn find_route_to(expressions: &[DataExpression]) -> Option<String> {
    use otel_arrow_contrib_data_engine_expressions::Expression;
    expressions.iter().find_map(|expression| match expression {
        DataExpression::Output(output) => {
            let (line, column) = output.get_query_location().get_line_and_column_numbers();
            Some(format!("line {line}, column {column}"))
        }
        DataExpression::Branch(branch) => branch
            .get_branches()
            .iter()
            .find_map(|b| find_route_to(b.get_expressions())),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::processor::{Config, Processor};

    fn compile(opl: &str) -> Result<(PipelineExpression, SignalContext)> {
        Transform::Opl(opl.to_owned()).compile()
    }

    fn processor(yaml: &str) -> Result<Processor> {
        let mut processor: Processor = serde_yaml_ng::from_str(yaml)?;
        processor.validate()?;
        Ok(processor)
    }

    #[test]
    fn a_transform_processor_deserializes_and_validates() {
        let processor = processor(
            r#"
id: 019c98c8-5512-70a2-906e-7ccd03618720
name: critical
type: transform
config:
  opl: |
    logs
    | where severity_number >= 17
    | set attributes["critical"] = true
"#,
        )
        .unwrap();
        let Config::Transform(Transform::Opl(program)) = processor.config else {
            panic!("not an OPL transform: {:?}", processor.config);
        };
        assert!(program.contains("severity_number"));
    }

    #[test]
    fn an_unknown_language_or_a_second_key_is_rejected() {
        for (config, expected) in [
            ("{kql: x}", "unknown variant `kql`, expected `opl`"),
            ("{opl: logs, query: nope}", "single key"),
            ("{}", "single key"),
        ] {
            let err = processor(&format!(
                "id: 019c98c8-5512-70a2-906e-7ccd03618720\nname: t\ntype: transform\nconfig: {config}\n"
            ))
            .unwrap_err();
            assert!(format!("{err:#}").contains(expected), "{config}: {err:#}");
        }
    }

    #[test]
    fn a_syntax_error_reports_its_position() {
        let err = compile("logs\n| where severity_number >=").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("line 2"), "{msg}");
    }

    #[test]
    fn route_to_is_rejected_even_inside_a_branch() {
        for opl in [
            r#"logs | route_to "errors""#,
            r#"logs | if (severity_number >= 17) { route_to "errors" }"#,
        ] {
            let err = compile(opl).unwrap_err();
            assert!(format!("{err:#}").contains("route_to"), "{opl}: {err:#}");
        }
    }

    #[test]
    fn every_signal_source_compiles() {
        for opl in [
            r#"logs | set attributes["x"] = 1"#,
            r#"traces | set attributes["x"] = 1"#,
            r#"metrics | set attributes["x"] = 1"#,
            r#"gauges | set attributes["x"] = 1"#,
            r#"signals | set resource.attributes["x"] = 1"#,
        ] {
            compile(opl).unwrap_or_else(|e| panic!("{opl}: {e:#}"));
        }
    }

    /// `the_shipped_configs_deserialize` only deserializes; the example's OPL
    /// must also parse.
    #[test]
    fn the_config_full_example_validates() {
        let config: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(include_str!("../../../config-full.yaml")).unwrap();
        let examples: Vec<Processor> = config["processors"]
            .as_sequence()
            .unwrap()
            .iter()
            .filter(|p| p["type"].as_str() == Some("transform"))
            .map(|p| serde_yaml_ng::from_value(p.clone()).unwrap())
            .collect();
        assert!(!examples.is_empty());
        for mut example in examples {
            example
                .validate()
                .unwrap_or_else(|e| panic!("{}: {e:#}", example.name));
        }
    }

    #[test]
    fn openapi_exposes_the_transform_config_and_its_comments() {
        use utoipa::OpenApi;
        let api = serde_json::to_value(crate::api::ApiDoc::openapi()).unwrap();
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
}
