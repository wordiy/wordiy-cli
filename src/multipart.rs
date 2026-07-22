//! Building `multipart/form-data` bodies for `wordiy push`.
//!
//! The import endpoint names each file part `files[<path>]` — the file's POSIX
//! relative path rides in the part **name**, not the `filename` attribute (which the
//! server ignores). That makes the part name security- and correctness-critical, so
//! all of it lives here: strict path validation and deterministic body assembly, with
//! the server's multipart quirks encoded once and tested hard.

use crate::error::{fail, Result};

const CRLF: &str = "\r\n";

/// Conservative cap on the relative path. The server drops a `Content-Disposition`
/// line past ~1536 bytes (then reads the request as having no files). The rendered
/// line is roughly `57 + path + basename` bytes; capping the path at 512 keeps the
/// worst case (a slash-less name, where basename == path) comfortably under the limit.
const MAX_PATH_LEN: usize = 512;

/// A single file to upload. `path` is the validated POSIX relative path that becomes
/// the `files[<path>]` part name; `bytes` is the raw file content.
pub struct FilePart {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// Validate and normalize a relative path into the POSIX form used inside a
/// `files[<path>]` part name. Fails fast with a clear message rather than emit a part
/// name the server will silently corrupt or reject.
///
/// Rejected: absolute paths, `..` traversal, empty (or empty-segment) paths,
/// backslashes (eaten by multipart quoted-string unescaping), `"` (ends the quoted
/// name), `[`/`]` (would break the server's `files[<path>]` bracket parsing), control
/// characters, and anything over [`MAX_PATH_LEN`].
pub fn validate_part_path(raw: &str) -> Result<String> {
    let path = raw.strip_prefix("./").unwrap_or(raw);

    if path.is_empty() {
        return fail("empty file path");
    }
    if path.len() > MAX_PATH_LEN {
        return fail(format!(
            "file path too long ({} bytes, max {MAX_PATH_LEN}): {raw}",
            path.len()
        ));
    }
    if path.starts_with('/') {
        return fail(format!("absolute file paths are not allowed: {raw}"));
    }
    for ch in path.chars() {
        match ch {
            '\\' => return fail(format!("backslash is not allowed in a file path: {raw}")),
            '"' => return fail(format!("double-quote is not allowed in a file path: {raw}")),
            '[' | ']' => {
                return fail(format!("square brackets are not allowed in a file path: {raw}"))
            }
            c if c.is_control() => {
                return fail(format!("control character is not allowed in a file path: {raw}"))
            }
            _ => {}
        }
    }
    for segment in path.split('/') {
        if segment.is_empty() {
            return fail(format!("empty path segment is not allowed: {raw}"));
        }
        if segment == ".." {
            return fail(format!("'..' is not allowed in a file path: {raw}"));
        }
    }

    Ok(path.to_string())
}

/// Pick a boundary token that occurs in none of the content (RFC 7578 requires the
/// boundary not appear in any encapsulated part). Deterministic: a fixed base, then a
/// numeric suffix bumped until it is collision-free.
pub fn pick_boundary(files: &[FilePart], params_json: &str) -> String {
    let mut n: u64 = 0;
    loop {
        let candidate = if n == 0 {
            "----wordiyFormBoundary".to_string()
        } else {
            format!("----wordiyFormBoundary{n}")
        };
        let needle = candidate.as_bytes();
        let collides = bytes_contain(params_json.as_bytes(), needle)
            || files.iter().any(|f| bytes_contain(&f.bytes, needle));
        if !collides {
            return candidate;
        }
        n += 1;
    }
}

/// Assemble the `multipart/form-data` body: one `files[<path>]` part per file, then a
/// single `params` JSON part. `boundary` must be collision-free (see [`pick_boundary`]);
/// each file's `path` must already be validated (see [`validate_part_path`]).
pub fn build_body(boundary: &str, files: &[FilePart], params_json: &str) -> Vec<u8> {
    let mut body = Vec::new();
    for file in files {
        let filename = file.path.rsplit('/').next().unwrap_or(&file.path);
        push(&mut body, &format!("--{boundary}{CRLF}"));
        push(
            &mut body,
            &format!(
                "Content-Disposition: form-data; name=\"files[{}]\"; filename=\"{}\"{CRLF}",
                file.path, filename
            ),
        );
        push(
            &mut body,
            &format!("Content-Type: {}{CRLF}{CRLF}", content_type_for(&file.path)),
        );
        body.extend_from_slice(&file.bytes);
        push(&mut body, CRLF);
    }

    push(&mut body, &format!("--{boundary}{CRLF}"));
    push(&mut body, &format!("Content-Disposition: form-data; name=\"params\"{CRLF}"));
    push(&mut body, &format!("Content-Type: application/json{CRLF}{CRLF}"));
    push(&mut body, params_json);
    push(&mut body, CRLF);

    push(&mut body, &format!("--{boundary}--{CRLF}"));
    body
}

fn push(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
}

/// Best-effort part content type. The server ignores it (format is detected from the
/// path/content), but a well-formed part carries one.
fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("xml") | Some("stringsdict") => "application/xml",
        Some("strings") => "text/plain",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(path: &str, bytes: &[u8]) -> FilePart {
        FilePart { path: path.to_string(), bytes: bytes.to_vec() }
    }

    #[test]
    fn accepts_normal_posix_paths() {
        assert_eq!(validate_part_path("values-ar/strings.xml").unwrap(), "values-ar/strings.xml");
        assert_eq!(
            validate_part_path("en.lproj/Localizable.strings").unwrap(),
            "en.lproj/Localizable.strings"
        );
        assert_eq!(validate_part_path("strings.xml").unwrap(), "strings.xml");
    }

    #[test]
    fn strips_leading_dot_slash() {
        assert_eq!(validate_part_path("./values/strings.xml").unwrap(), "values/strings.xml");
    }

    #[test]
    fn rejects_hazardous_paths() {
        // Each of these must fail fast rather than reach the wire.
        for bad in [
            "/etc/passwd",  // absolute
            "../secret",    // traversal
            "a/../b",       // interior traversal
            "a\\b",         // backslash (quoted-string unescaping hazard)
            "a\"b",         // double-quote ends the part name
            "files[x]",     // brackets would break files[<path>] parsing
            "a[b",          // stray open bracket
            "a]b",          // stray close bracket
            "a\nb",         // control: newline
            "a\tb",         // control: tab
            "a\u{7f}b",     // control: DEL
            "",             // empty
            "a//b",         // empty segment
            "trailing/",    // empty trailing segment
        ] {
            assert!(validate_part_path(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn rejects_overlong_path() {
        let long = format!("{}x", "a/".repeat(300)); // > 512 bytes
        assert!(validate_part_path(&long).is_err());
    }

    #[test]
    fn build_body_is_byte_exact() {
        let files = [fp("values-ar/strings.xml", b"<resources/>")];
        let body = build_body("BX", &files, "{\"forceMode\":\"OVERRIDE\"}");
        let expected = concat!(
            "--BX\r\n",
            "Content-Disposition: form-data; name=\"files[values-ar/strings.xml]\"; filename=\"strings.xml\"\r\n",
            "Content-Type: application/xml\r\n",
            "\r\n",
            "<resources/>\r\n",
            "--BX\r\n",
            "Content-Disposition: form-data; name=\"params\"\r\n",
            "Content-Type: application/json\r\n",
            "\r\n",
            "{\"forceMode\":\"OVERRIDE\"}\r\n",
            "--BX--\r\n",
        );
        assert_eq!(String::from_utf8(body).unwrap(), expected);
    }

    #[test]
    fn build_body_preserves_raw_binary() {
        // A zip part with NUL bytes must survive verbatim.
        let raw = [0u8, 1, 2, b'P', b'K', 3, 4, 0];
        let files = [fp("bundle.zip", &raw)];
        let body = build_body("BX", &files, "{}");
        assert!(bytes_contain(&body, &raw), "binary bytes must pass through untouched");
    }

    #[test]
    fn build_body_handles_no_files() {
        // The command enforces >= 1 file; the builder must still not panic without one.
        let body = build_body("BX", &[], "{}");
        let expected = concat!(
            "--BX\r\n",
            "Content-Disposition: form-data; name=\"params\"\r\n",
            "Content-Type: application/json\r\n",
            "\r\n",
            "{}\r\n",
            "--BX--\r\n",
        );
        assert_eq!(String::from_utf8(body).unwrap(), expected);
    }

    #[test]
    fn pick_boundary_default_when_clear() {
        let files = [fp("a.xml", b"hello")];
        assert_eq!(pick_boundary(&files, "{}"), "----wordiyFormBoundary");
    }

    #[test]
    fn pick_boundary_avoids_collision() {
        // Content that literally contains the default boundary forces a different one.
        let params = "noise ----wordiyFormBoundary noise";
        let files = [fp("a.xml", b"----wordiyFormBoundary1 also here")];
        let chosen = pick_boundary(&files, params);
        assert_ne!(chosen, "----wordiyFormBoundary");
        assert!(!params.contains(&chosen));
        assert!(!bytes_contain(&files[0].bytes, chosen.as_bytes()));
    }
}
