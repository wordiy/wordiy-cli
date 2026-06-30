//! HTTP client for the translations export endpoint.
//!
//! `pull` depends on the [`ExportClient`] trait, not the concrete HTTP type, so the
//! command can be unit-tested with a fake. [`HttpExportClient`] is the real
//! implementation: it `POST`s the [`ExportRequest`] to `/api/v1/project/export`
//! with the `Api-Key` header and returns the raw ZIP bytes, mapping a `{code,
//! params}` error envelope to a typed [`CliError`].

use std::io::Read;

use serde::{Deserialize, Serialize};

use crate::cli::{Format, State};
use crate::error::{CliError, Result};

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
}

impl ExportRequest {
    pub fn new(format: Format, languages: &[String], states: &[State]) -> Self {
        Self {
            export_format: format.as_wire().to_string(),
            languages: languages.to_vec(),
            filter_state: states.iter().map(|s| s.as_wire().to_string()).collect(),
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

/// The server's typed error envelope. Only `code` is consumed; `params` (always
/// null for export in v1) is ignored.
#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    code: String,
}

/// Map a server error `code` to a friendly message.
fn friendly_message(status: u16, code: &str) -> String {
    match code {
        "request_parse_error" => {
            "the export request was rejected — check the format and parameters".to_string()
        }
        "no_exported_result" => {
            "no translations matched the requested languages/states".to_string()
        }
        "unauthenticated" => "missing API key — pass --api-key or set WORDIY_API_KEY".to_string(),
        "invalid_api_key" => "the API key is invalid".to_string(),
        "key_type_not_authorized" => {
            "this API key type is not allowed to export (use a srv_ or adm_ key)".to_string()
        }
        "project_not_found" => "no project is associated with this API key".to_string(),
        other => format!("export failed (HTTP {status}): {other}"),
    }
}

/// Real HTTP implementation over `ureq`.
pub struct HttpExportClient {
    base_url: String,
    api_key: String,
}

impl HttpExportClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self { base_url, api_key }
    }

    fn export_url(&self) -> String {
        format!(
            "{}/api/v1/project/export",
            self.base_url.trim_end_matches('/')
        )
    }
}

impl ExportClient for HttpExportClient {
    fn export(&self, req: &ExportRequest) -> Result<Vec<u8>> {
        let body = serde_json::to_vec(req)
            .map_err(|e| CliError::Message(format!("could not encode the request: {e}")))?;

        match ureq::post(&self.export_url())
            .set("Api-Key", &self.api_key)
            .set("Content-Type", "application/json")
            .send_bytes(&body)
        {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|e| CliError::Transport(e.to_string()))?;
                Ok(buf)
            }
            // 4xx/5xx with a body: parse the typed envelope.
            Err(ureq::Error::Status(status, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                let code = serde_json::from_str::<ApiErrorBody>(&body)
                    .map(|e| e.code)
                    .unwrap_or_else(|_| "unknown_error".to_string());
                Err(CliError::Api {
                    status,
                    message: friendly_message(status, &code),
                    code,
                })
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
        let req = ExportRequest::new(Format::AndroidXml, &[], &[]);
        // No filters → only exportFormat is present.
        assert_eq!(req.to_json(), r#"{"exportFormat":"ANDROID_XML"}"#);
    }

    #[test]
    fn serializes_filters_when_present() {
        let req = ExportRequest::new(
            Format::AndroidXml,
            &["en".to_string(), "ar".to_string()],
            &[State::Translated, State::Reviewed],
        );
        assert_eq!(
            req.to_json(),
            r#"{"exportFormat":"ANDROID_XML","languages":["en","ar"],"filterState":["TRANSLATED","REVIEWED"]}"#
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
        assert!(friendly_message(401, "invalid_api_key").contains("invalid"));
        assert!(friendly_message(400, "no_exported_result").contains("no translations"));
        // Unknown codes fall back to a generic message that keeps the code.
        assert!(friendly_message(400, "weird_new_code").contains("weird_new_code"));
    }

    #[test]
    fn url_joins_without_double_slash() {
        let c = HttpExportClient::new("http://localhost:3001/".to_string(), "srv_x".to_string());
        assert_eq!(c.export_url(), "http://localhost:3001/api/v1/project/export");
    }
}
