//! HTTP client for the translations export endpoint.
//!
//! `pull` depends on the [`ExportClient`] trait, not the concrete HTTP type, so the
//! command can be unit-tested with a fake. [`HttpExportClient`] is the real
//! implementation: it `POST`s the [`ExportRequest`] to the project export endpoint
//! with the `Api-Key` header and returns the raw ZIP bytes, mapping a `{code,
//! params}` error envelope to a typed [`CliError`].

use std::io::Read;

use serde::{Deserialize, Serialize};

use crate::cli::{Format, State};
use crate::error::{CliError, Result};

/// Upper bound on the export response we will buffer, so a hostile/MITM'd server
/// can't exhaust memory with an unbounded body.
const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

/// Path prefix shared by every v1 API endpoint, defined once so a version/prefix
/// change (e.g. `/api/v2`) is a single edit rather than a per-endpoint sweep.
const API_BASE_PATH: &str = "/api/v1";

/// The user's export selection — the semantic inputs to an export request, grouped so
/// the request is built from named fields instead of positional arguments. Borrows its
/// inputs; construct it inline, typically with `..Default::default()`.
#[derive(Debug, Default)]
pub struct ExportQuery<'a> {
    pub format: Format,
    pub languages: &'a [String],
    pub states: &'a [State],
    pub tags: &'a [String],
    pub exclude_tags: &'a [String],
    pub key_prefix: Option<&'a str>,
}

/// Body of the export request. Serialized as camelCase; empty filters are omitted
/// so the server applies its defaults.
#[derive(Debug, Serialize)]
pub struct ExportRequest {
    #[serde(rename = "exportFormat")]
    export_format: String,
    #[serde(rename = "languages", skip_serializing_if = "Vec::is_empty")]
    languages: Vec<String>,
    #[serde(rename = "filterState", skip_serializing_if = "Vec::is_empty")]
    filter_state: Vec<String>,
    #[serde(rename = "filterTagIn", skip_serializing_if = "Vec::is_empty")]
    filter_tag_in: Vec<String>,
    #[serde(rename = "filterTagNotIn", skip_serializing_if = "Vec::is_empty")]
    filter_tag_not_in: Vec<String>,
    #[serde(rename = "filterKeyPrefix", skip_serializing_if = "Option::is_none")]
    filter_key_prefix: Option<String>,
}

impl ExportRequest {
    pub fn new(query: ExportQuery) -> Self {
        Self {
            export_format: query.format.as_wire().to_string(),
            languages: query.languages.to_vec(),
            filter_state: query.states.iter().map(|s| s.as_wire().to_string()).collect(),
            filter_tag_in: query.tags.to_vec(),
            filter_tag_not_in: query.exclude_tags.to_vec(),
            filter_key_prefix: query.key_prefix.map(str::to_string),
        }
    }

    /// Compact JSON, for debug logging.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Fetches a project's translations as ZIP bytes.
pub trait ExportClient {
    fn export(&self, req: &ExportRequest) -> Result<Vec<u8>>;
}

/// The server's typed error envelope: a stable `code` plus optional named `params`
/// (e.g. `unsupported_export_format` carries `{ format, supported }`).
#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    code: String,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

/// Map a server error `code` (and its optional `params`) to a friendly message.
fn friendly_message(status: u16, code: &str, params: Option<&serde_json::Value>) -> String {
    match code {
        "unsupported_export_format" => unsupported_format_message(params),
        "request_parse_error" => {
            "the export request was rejected — check the format and parameters".to_string()
        }
        "no_exported_result" => "no translations matched the requested filters".to_string(),
        "unauthenticated" => "missing API key — pass --api-key or set WORDIY_API_KEY".to_string(),
        "invalid_api_key" => "the API key is invalid".to_string(),
        "key_type_not_authorized" => {
            "this API key type is not allowed to export (use a srv_ or adm_ key)".to_string()
        }
        "project_not_found" => "no project is associated with this API key".to_string(),
        other => format!("export failed (HTTP {status}): {other}"),
    }
}

/// Message for `unsupported_export_format`, surfacing the offending `format` and the
/// set the server supports from `params: { format, supported }`. This normally means
/// the CLI's known formats have drifted ahead of the backend's.
fn unsupported_format_message(params: Option<&serde_json::Value>) -> String {
    let format = params.and_then(|p| p.get("format")).and_then(|v| v.as_str());
    let supported = params
        .and_then(|p| p.get("supported"))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
        .filter(|s| !s.is_empty());

    let base = match format {
        Some(f) => format!("the server does not support export format '{f}'"),
        None => "no supported export format was provided".to_string(),
    };
    match supported {
        Some(s) => format!("{base} (the server supports: {s})"),
        None => base,
    }
}

/// Real HTTP implementation over `ureq`.
pub struct HttpExportClient {
    base_url: String,
    api_key: String,
    verbose: bool,
}

impl HttpExportClient {
    pub fn new(base_url: String, api_key: String, verbose: bool) -> Self {
        Self {
            base_url,
            api_key,
            verbose,
        }
    }

    /// Absolute URL for an endpoint path such as `/project/export`. The base URL and
    /// the API version prefix ([`API_BASE_PATH`]) are joined here, so each endpoint
    /// names only its own resource path.
    fn url(&self, path: &str) -> String {
        format!("{}{API_BASE_PATH}{path}", self.base_url.trim_end_matches('/'))
    }
}

impl ExportClient for HttpExportClient {
    fn export(&self, req: &ExportRequest) -> Result<Vec<u8>> {
        let url = self.url("/project/export");
        if self.verbose {
            eprintln!("[debug] POST {url} body={}", req.to_json());
        }

        let body = serde_json::to_vec(req)
            .map_err(|e| CliError::Message(format!("could not encode the request: {e}")))?;

        match ureq::post(&url)
            .set("Api-Key", &self.api_key)
            .set("Content-Type", "application/json")
            .send_bytes(&body)
        {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader()
                    .take(MAX_RESPONSE_BYTES + 1)
                    .read_to_end(&mut buf)
                    .map_err(|e| CliError::Transport(e.to_string()))?;
                if buf.len() as u64 > MAX_RESPONSE_BYTES {
                    return Err(CliError::Transport("export response too large".into()));
                }
                Ok(buf)
            }
            // 4xx/5xx with a body: parse the typed envelope.
            Err(ureq::Error::Status(status, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                let parsed = serde_json::from_str::<ApiErrorBody>(&body).ok();
                let code = parsed
                    .as_ref()
                    .map(|e| e.code.clone())
                    .unwrap_or_else(|| "unknown_error".to_string());
                let message =
                    friendly_message(status, &code, parsed.as_ref().and_then(|e| e.params.as_ref()));
                Err(CliError::Api { status, code, message })
            }
            Err(ureq::Error::Transport(t)) => Err(CliError::Transport(t.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_minimal_request_as_camelcase() {
        let req = ExportRequest::new(ExportQuery::default());
        // No filters → only exportFormat is present.
        assert_eq!(req.to_json(), r#"{"exportFormat":"ANDROID_XML"}"#);
    }

    #[test]
    fn serializes_filters_when_present() {
        let req = ExportRequest::new(ExportQuery {
            languages: &["en".to_string(), "ar".to_string()],
            states: &[State::Translated, State::Reviewed],
            tags: &["mobile".to_string()],
            exclude_tags: &["legacy".to_string()],
            key_prefix: Some("home_"),
            ..Default::default()
        });
        assert_eq!(
            req.to_json(),
            r#"{"exportFormat":"ANDROID_XML","languages":["en","ar"],"filterState":["TRANSLATED","REVIEWED"],"filterTagIn":["mobile"],"filterTagNotIn":["legacy"],"filterKeyPrefix":"home_"}"#
        );
    }

    #[test]
    fn serializes_tags_only() {
        let req = ExportRequest::new(ExportQuery {
            tags: &["checkout".to_string()],
            ..Default::default()
        });
        assert_eq!(req.to_json(), r#"{"exportFormat":"ANDROID_XML","filterTagIn":["checkout"]}"#);
    }

    #[test]
    fn serializes_exclude_tags_only() {
        let req = ExportRequest::new(ExportQuery {
            exclude_tags: &["legacy".to_string()],
            ..Default::default()
        });
        assert_eq!(
            req.to_json(),
            r#"{"exportFormat":"ANDROID_XML","filterTagNotIn":["legacy"]}"#
        );
    }

    #[test]
    fn serializes_key_prefix_only() {
        let req = ExportRequest::new(ExportQuery {
            key_prefix: Some("home_"),
            ..Default::default()
        });
        assert_eq!(
            req.to_json(),
            r#"{"exportFormat":"ANDROID_XML","filterKeyPrefix":"home_"}"#
        );
    }

    #[test]
    fn parses_error_envelope_to_code() {
        let body = r#"{"code":"no_exported_result","params":null}"#;
        let parsed: ApiErrorBody = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.code, "no_exported_result");
    }

    #[test]
    fn maps_known_codes_to_friendly_messages() {
        assert!(friendly_message(401, "invalid_api_key", None).contains("invalid"));
        assert!(friendly_message(400, "no_exported_result", None).contains("no translations"));
        // Unknown codes fall back to a generic message that keeps the code.
        assert!(friendly_message(400, "weird_new_code", None).contains("weird_new_code"));
    }

    #[test]
    fn unsupported_format_message_surfaces_offending_format_and_supported_set() {
        // The exact params shape the backend returns for a bad exportFormat.
        let params = serde_json::json!({ "format": "JSON_ICU", "supported": ["ANDROID_XML"] });
        let msg = friendly_message(400, "unsupported_export_format", Some(&params));
        assert!(msg.contains("JSON_ICU"), "names the offending format: {msg}");
        assert!(msg.contains("ANDROID_XML"), "lists the supported set: {msg}");
    }

    #[test]
    fn unsupported_format_message_handles_missing_format_and_no_params() {
        // Missing exportFormat → format is null; still mention what's supported.
        let params = serde_json::json!({ "format": null, "supported": ["ANDROID_XML"] });
        assert!(friendly_message(400, "unsupported_export_format", Some(&params)).contains("ANDROID_XML"));
        // No params at all → still a sensible, non-panicking message.
        assert!(friendly_message(400, "unsupported_export_format", None).contains("format"));
    }

    #[test]
    fn url_joins_base_prefix_and_path() {
        let c = HttpExportClient::new("http://localhost:3001/".to_string(), "srv_x".into(), false);
        assert_eq!(c.url("/project/export"), "http://localhost:3001/api/v1/project/export");
    }
}
