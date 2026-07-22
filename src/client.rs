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
use crate::multipart::{self, FilePart};

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

// ---------- Import (`push`) ----------

/// A key+language whose stored text differs from the imported value. The same shape
/// appears in a `200 KEEP` response and inside a `409 import_conflicts_unresolved`.
/// (The contract's `keyNamespace` is reserved/always-null until namespaces ship, so it
/// is intentionally not modeled — extra JSON fields are ignored on parse.)
#[derive(Debug, Deserialize)]
pub struct UnresolvedConflict {
    #[serde(rename = "keyName")]
    pub key_name: String,
    pub language: String,
}

/// The applied state of a successful import (`200`). Counts are distinct-key; an
/// all-zero response with a non-zero `total_keys` means everything was already in sync.
#[derive(Debug, Deserialize)]
pub struct ImportResult {
    #[serde(rename = "totalKeys")]
    pub total_keys: u64,
    pub created: u64,
    pub updated: u64,
    pub skipped: u64,
    pub failed: u64,
    #[serde(rename = "unresolvedConflicts", default)]
    pub unresolved_conflicts: Vec<UnresolvedConflict>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Inputs for an import: the validated files to upload and the pre-serialized `params`
/// JSON. The command builds both; the client turns them into the multipart body.
pub struct ImportRequest {
    pub files: Vec<FilePart>,
    pub params_json: String,
}

/// Uploads local resource files to the project's import endpoint.
pub trait ImportClient {
    fn import(&self, req: &ImportRequest) -> Result<ImportResult>;
}

/// Real HTTP implementation of [`ImportClient`] over `ureq`.
pub struct HttpImportClient {
    base_url: String,
    api_key: String,
    verbose: bool,
}

impl HttpImportClient {
    pub fn new(base_url: String, api_key: String, verbose: bool) -> Self {
        Self {
            base_url,
            api_key,
            verbose,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{API_BASE_PATH}{path}", self.base_url.trim_end_matches('/'))
    }
}

impl ImportClient for HttpImportClient {
    fn import(&self, req: &ImportRequest) -> Result<ImportResult> {
        let url = self.url("/project/import");
        let boundary = multipart::pick_boundary(&req.files, &req.params_json);
        let body = multipart::build_body(&boundary, &req.files, &req.params_json);
        if self.verbose {
            eprintln!(
                "[debug] POST {url} ({} file(s)) params={}",
                req.files.len(),
                req.params_json
            );
        }

        match ureq::post(&url)
            .set("Api-Key", &self.api_key)
            .set("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
            .send_bytes(&body)
        {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader()
                    .take(MAX_RESPONSE_BYTES + 1)
                    .read_to_end(&mut buf)
                    .map_err(|e| CliError::Transport(e.to_string()))?;
                if buf.len() as u64 > MAX_RESPONSE_BYTES {
                    return Err(CliError::Transport("import response too large".into()));
                }
                serde_json::from_slice::<ImportResult>(&buf)
                    .map_err(|e| CliError::Message(format!("could not parse the import response: {e}")))
            }
            Err(ureq::Error::Status(status, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                let parsed = serde_json::from_str::<ApiErrorBody>(&body).ok();
                let code = parsed
                    .as_ref()
                    .map(|e| e.code.clone())
                    .unwrap_or_else(|| "unknown_error".to_string());
                let message = import_friendly_message(
                    status,
                    &code,
                    parsed.as_ref().and_then(|e| e.params.as_ref()),
                );
                Err(CliError::Api { status, code, message })
            }
            Err(ureq::Error::Transport(t)) => Err(CliError::Transport(t.to_string())),
        }
    }
}

/// Map an import error `code` (+ optional `params`) to a friendly message.
fn import_friendly_message(status: u16, code: &str, params: Option<&serde_json::Value>) -> String {
    let field = |name: &str| params.and_then(|p| p.get(name)).and_then(|v| v.as_str());
    match code {
        "import_conflicts_unresolved" => conflicts_message(params),
        "import_no_files" => "no importable files were found under the given path".to_string(),
        "unsupported_import_format" => {
            let base = match field("fileName") {
                Some(f) => format!("unsupported or undetectable format for '{f}'"),
                None => "unsupported import format".to_string(),
            };
            match string_list(params, "supported") {
                Some(s) => format!("{base} (the server supports: {s})"),
                None => base,
            }
        }
        "import_language_unresolved" => match field("fileName") {
            Some(f) => format!(
                "could not determine the language for '{f}' — its path has no locale; pass --language"
            ),
            None => "could not determine a file's language".to_string(),
        },
        "import_file_parse_error" => match (field("fileName"), field("detail")) {
            (Some(f), Some(d)) => format!("failed to parse '{f}': {d}"),
            (Some(f), None) => format!("failed to parse '{f}'"),
            _ => "a file failed to parse".to_string(),
        },
        "import_file_mapping_mismatch" => match string_list(params, "fileNames") {
            Some(s) => format!("file mapping(s) matched no uploaded file: {s}"),
            None => "a file mapping matched no uploaded file".to_string(),
        },
        "plan_limit_exceeded" => {
            "importing these keys would exceed your plan's key limit".to_string()
        }
        "request_parse_error" => match field("detail") {
            Some(d) => format!("the import request was rejected: {d}"),
            None => "the import request was rejected — check params and file mappings".to_string(),
        },
        "unauthenticated" => "missing API key — pass --api-key or set WORDIY_API_KEY".to_string(),
        "invalid_api_key" => "the API key is invalid".to_string(),
        "key_type_not_authorized" => {
            "this API key type is not allowed to import (use a srv_ or adm_ key)".to_string()
        }
        "project_not_found" => "no project is associated with this API key".to_string(),
        other => format!("import failed (HTTP {status}): {other}"),
    }
}

/// Render the `409` conflict list (also the flag-only conflict UX): the conflicting
/// key+language pairs from `params.conflicts`, plus how to resolve them.
fn conflicts_message(params: Option<&serde_json::Value>) -> String {
    let conflicts = params.and_then(|p| p.get("conflicts")).and_then(|v| v.as_array());
    let count = conflicts.map(|a| a.len()).unwrap_or(0);
    let mut msg =
        format!("{count} conflict(s) — nothing was written (key+language already differs):");
    if let Some(list) = conflicts {
        for c in list.iter().take(20) {
            let key = c.get("keyName").and_then(|v| v.as_str()).unwrap_or("?");
            let lang = c.get("language").and_then(|v| v.as_str()).unwrap_or("?");
            msg.push_str(&format!("\n    {key} ({lang})"));
        }
        if list.len() > 20 {
            msg.push_str(&format!("\n    … and {} more", list.len() - 20));
        }
    }
    msg.push_str(
        "\n  re-run with --force-mode OVERRIDE (overwrite) or --force-mode KEEP (skip conflicts)",
    );
    msg
}

/// Join a string-array `param` (e.g. `supported`, `fileNames`) into a comma list.
fn string_list(params: Option<&serde_json::Value>, key: &str) -> Option<String> {
    params
        .and_then(|p| p.get(key))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
        .filter(|s| !s.is_empty())
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

    #[test]
    fn parses_import_result() {
        let body = r#"{"totalKeys":3,"created":1,"updated":2,"skipped":0,"failed":0,"unresolvedConflicts":[],"warnings":[]}"#;
        let r: ImportResult = serde_json::from_str(body).unwrap();
        assert_eq!(r.total_keys, 3);
        assert_eq!(r.updated, 2);
        assert!(r.unresolved_conflicts.is_empty());
    }

    #[test]
    fn import_conflict_message_lists_keys_and_guidance() {
        let params = serde_json::json!({ "conflicts": [
            { "keyName": "greeting_hello", "keyNamespace": null, "language": "ar" },
            { "keyName": "cart_title", "keyNamespace": null, "language": "ar" }
        ]});
        let msg = import_friendly_message(409, "import_conflicts_unresolved", Some(&params));
        assert!(msg.contains("2 conflict"), "{msg}");
        assert!(msg.contains("greeting_hello (ar)"), "{msg}");
        assert!(msg.contains("--force-mode OVERRIDE"), "{msg}");
    }

    #[test]
    fn import_maps_known_and_unknown_codes() {
        assert!(import_friendly_message(401, "invalid_api_key", None).contains("invalid"));
        assert!(import_friendly_message(400, "import_no_files", None).contains("no importable"));
        let p = serde_json::json!({ "fileName": "weird.txt", "supported": ["ANDROID_XML", "STRINGS", "STRINGSDICT"] });
        let m = import_friendly_message(400, "unsupported_import_format", Some(&p));
        assert!(m.contains("weird.txt") && m.contains("ANDROID_XML"), "{m}");
        // Unknown codes keep the code (future-reachable error codes stay legible).
        assert!(import_friendly_message(400, "brand_new_code", None).contains("brand_new_code"));
    }

    #[test]
    fn import_url_joins_prefix() {
        let c = HttpImportClient::new("http://localhost:3001/".to_string(), "srv_x".into(), false);
        assert_eq!(c.url("/project/import"), "http://localhost:3001/api/v1/project/import");
    }
}
