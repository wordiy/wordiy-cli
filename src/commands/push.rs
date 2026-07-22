//! `wordiy push` — upload local resource files to wordiy (import).
//!
//! Walks `--path` for resource files, uploads each as a `files[<relpath>]` multipart
//! part via an [`ImportClient`], and reports the applied result. The CLI is a thin
//! pipe: the backend infers language (from the path) and format (from the extension)
//! unless `--language`/`--format` override them.

use std::path::{Component, Path, PathBuf};

use clap::ValueEnum;
use serde::Serialize;

use crate::cli::{ForceMode, ImportFormat, PushArgs};
use crate::client::{HttpImportClient, ImportClient, ImportRequest, ImportResult};
use crate::config::{LoadedConfig, PushConfig};
use crate::context::Context;
use crate::error::{fail, CliError, Result};
use crate::multipart::{validate_part_path, FilePart};

/// Push options after merging CLI flags over the config file over defaults.
struct ResolvedPush {
    path: PathBuf,
    force_mode: ForceMode,
    format: Option<ImportFormat>,
    language: Option<String>,
    create_new_keys: bool,
    override_descriptions: bool,
}

pub fn run(ctx: &Context, args: &PushArgs, loaded: &LoadedConfig) -> Result<()> {
    let resolved = resolve(args, &loaded.config.push, &loaded.dir)?;

    let Some(api_key) = ctx.api_key.clone() else {
        return fail("Missing API key: pass --api-key or set WORDIY_API_KEY");
    };

    let files = collect_files(&resolved.path)?;
    if files.is_empty() {
        return fail(format!("no files to push under {}", resolved.path.display()));
    }

    let params_json = build_params(&resolved, &files);
    let client = HttpImportClient::new(ctx.api_url.clone(), api_key, ctx.verbose);
    let result = client.import(&ImportRequest { files, params_json })?;
    print_summary(&result);
    Ok(())
}

/// Merge CLI flags (highest priority) with the config file, then defaults. A relative
/// `push.path` from the config resolves against the config file's directory.
fn resolve(args: &PushArgs, cfg: &PushConfig, config_dir: &Path) -> Result<ResolvedPush> {
    let path = match &args.path {
        Some(p) => p.clone(),
        None => match &cfg.path {
            Some(p) if p.is_absolute() => p.clone(),
            Some(p) => config_dir.join(p),
            None => return fail("Missing --path: pass --path or set push.path in the config"),
        },
    };

    let force_mode = match args.force_mode {
        Some(m) => m,
        None => match &cfg.force_mode {
            Some(s) => ForceMode::from_str(s, false).map_err(|_| {
                CliError::Message(format!("invalid push.forceMode value in config: '{s}'"))
            })?,
            None => ForceMode::default(),
        },
    };

    Ok(ResolvedPush {
        path,
        force_mode,
        format: args.format,
        language: args.language.clone(),
        create_new_keys: !args.no_create_new_keys,
        override_descriptions: args.override_descriptions,
    })
}

/// Collect every file under `root` into validated [`FilePart`]s keyed by POSIX relative
/// path, sorted for a deterministic request. Dotfiles and macOS junk are skipped — the
/// server hard-fails a directly-uploaded unsupported file, so we never send them.
fn collect_files(root: &Path) -> Result<Vec<FilePart>> {
    if !root.is_dir() {
        return fail(format!(
            "{} is not a directory (archive push is a follow-up)",
            root.display()
        ));
    }
    let mut files = Vec::new();
    collect_into(root, root, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn collect_into(root: &Path, dir: &Path, out: &mut Vec<FilePart>) -> Result<()> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| CliError::Message(format!("could not read {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| CliError::Message(e.to_string()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "__MACOSX" {
            continue;
        }
        // Inspect the entry's own type — `file_type()` does NOT follow symlinks. Skip
        // symlinks: one pointing at an ancestor would recurse forever, and one pointing
        // outside `--path` would upload files we were never asked to.
        let file_type = entry.file_type().map_err(|e| {
            CliError::Message(format!("could not read entry type in {}: {e}", dir.display()))
        })?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_into(root, &path, out)?;
        } else if file_type.is_file() {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let posix = posix_path(rel)?;
            let valid = validate_part_path(&posix)?;
            let bytes = std::fs::read(&path)
                .map_err(|e| CliError::Message(format!("could not read {}: {e}", path.display())))?;
            out.push(FilePart { path: valid, bytes });
        }
    }
    Ok(())
}

/// Convert a relative path to a POSIX (`/`-separated) string, rejecting non-UTF-8 or
/// non-normal components.
fn posix_path(rel: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(s) => match s.to_str() {
                Some(s) => parts.push(s),
                None => return fail(format!("non-UTF-8 path: {}", rel.display())),
            },
            _ => return fail(format!("unexpected path component in {}", rel.display())),
        }
    }
    Ok(parts.join("/"))
}

/// Serialize the `params` JSON. Fields equal to the server default are omitted so the
/// common case sends `{}`; `fileMappings` are emitted only when `--format`/`--language`
/// override inference.
fn build_params(r: &ResolvedPush, files: &[FilePart]) -> String {
    let file_mappings = if r.format.is_some() || r.language.is_some() {
        files
            .iter()
            .map(|f| FileMappingWire {
                file_name: f.path.clone(),
                language_tag: r.language.clone(),
                format: r.format.map(ImportFormat::as_wire),
            })
            .collect()
    } else {
        Vec::new()
    };

    let params = ImportParamsWire {
        force_mode: (r.force_mode != ForceMode::NoForce).then(|| r.force_mode.as_wire()),
        create_new_keys: (!r.create_new_keys).then_some(false),
        override_key_descriptions: r.override_descriptions.then_some(true),
        file_mappings,
    };
    serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string())
}

#[derive(Serialize)]
struct ImportParamsWire {
    #[serde(rename = "forceMode", skip_serializing_if = "Option::is_none")]
    force_mode: Option<&'static str>,
    #[serde(rename = "createNewKeys", skip_serializing_if = "Option::is_none")]
    create_new_keys: Option<bool>,
    #[serde(rename = "overrideKeyDescriptions", skip_serializing_if = "Option::is_none")]
    override_key_descriptions: Option<bool>,
    #[serde(rename = "fileMappings", skip_serializing_if = "Vec::is_empty")]
    file_mappings: Vec<FileMappingWire>,
}

#[derive(Serialize)]
struct FileMappingWire {
    #[serde(rename = "fileName")]
    file_name: String,
    #[serde(rename = "languageTag", skip_serializing_if = "Option::is_none")]
    language_tag: Option<String>,
    #[serde(rename = "format", skip_serializing_if = "Option::is_none")]
    format: Option<&'static str>,
}

/// One-line applied-state summary. An all-zero response with keys means "in sync".
fn summary_line(r: &ImportResult) -> String {
    let unchanged = r.created == 0 && r.updated == 0 && r.skipped == 0 && r.failed == 0;
    if unchanged && r.total_keys > 0 {
        format!("already in sync ({} key(s), no changes)", r.total_keys)
    } else {
        let failed = if r.failed > 0 {
            format!(", {} failed", r.failed)
        } else {
            String::new()
        };
        format!(
            "pushed {} key(s): {} created, {} updated, {} skipped{failed}",
            r.total_keys, r.created, r.updated, r.skipped
        )
    }
}

fn print_summary(r: &ImportResult) {
    println!("{}", summary_line(r));

    if !r.unresolved_conflicts.is_empty() {
        println!(
            "{} conflict(s) kept (existing text unchanged):",
            r.unresolved_conflicts.len()
        );
        for c in r.unresolved_conflicts.iter().take(20) {
            println!("    {} ({})", c.key_name, c.language);
        }
    }
    // Don't double-report kept conflicts that also surface as warnings.
    let conflict_keys: Vec<&str> =
        r.unresolved_conflicts.iter().map(|c| c.key_name.as_str()).collect();
    for w in &r.warnings {
        if conflict_keys.iter().any(|k| w.contains(k)) {
            continue;
        }
        println!("warning: {w}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
    use crate::client::ImportResult;
    use clap::Parser;

    fn push_args(argv: &[&str]) -> PushArgs {
        let Command::Push(args) = Cli::try_parse_from(argv).expect("parses").command else {
            panic!("expected a push command");
        };
        args
    }

    fn resolved(argv: &[&str]) -> ResolvedPush {
        resolve(&push_args(argv), &PushConfig::default(), Path::new("/base")).unwrap()
    }

    fn part(path: &str) -> FilePart {
        FilePart { path: path.to_string(), bytes: Vec::new() }
    }

    fn result(total: u64, created: u64, updated: u64) -> ImportResult {
        ImportResult {
            total_keys: total,
            created,
            updated,
            skipped: 0,
            failed: 0,
            unresolved_conflicts: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn force_mode_defaults_to_no_force() {
        assert_eq!(
            resolved(&["wordiy", "push", "--path", "res"]).force_mode,
            ForceMode::NoForce
        );
    }

    #[test]
    fn config_path_resolves_against_config_dir() {
        let cfg = PushConfig { path: Some(PathBuf::from("out")), force_mode: None };
        let r = resolve(&push_args(&["wordiy", "push"]), &cfg, Path::new("/base")).unwrap();
        assert_eq!(r.path, PathBuf::from("/base/out"));
    }

    #[test]
    fn cli_force_mode_overrides_config() {
        let cfg = PushConfig { path: Some(PathBuf::from("out")), force_mode: Some("KEEP".into()) };
        let r = resolve(
            &push_args(&["wordiy", "push", "--force-mode", "OVERRIDE"]),
            &cfg,
            Path::new("/base"),
        )
        .unwrap();
        assert_eq!(r.force_mode, ForceMode::Override);
    }

    #[test]
    fn invalid_config_force_mode_is_rejected() {
        let cfg = PushConfig { path: Some(PathBuf::from("out")), force_mode: Some("NONSENSE".into()) };
        assert!(resolve(&push_args(&["wordiy", "push"]), &cfg, Path::new("/base")).is_err());
    }

    #[test]
    fn params_default_is_empty_object() {
        let r = resolved(&["wordiy", "push", "--path", "res"]);
        assert_eq!(build_params(&r, &[part("values/strings.xml")]), "{}");
    }

    #[test]
    fn params_include_non_default_flags() {
        let r = resolved(&[
            "wordiy", "push", "--path", "res", "--force-mode", "OVERRIDE",
            "--no-create-new-keys", "--override-descriptions",
        ]);
        let json = build_params(&r, &[part("values/strings.xml")]);
        assert!(json.contains(r#""forceMode":"OVERRIDE""#), "{json}");
        assert!(json.contains(r#""createNewKeys":false"#), "{json}");
        assert!(json.contains(r#""overrideKeyDescriptions":true"#), "{json}");
    }

    #[test]
    fn params_emit_file_mappings_when_overriding() {
        let r = resolved(&["wordiy", "push", "--path", "res", "--format", "STRINGS", "-l", "ar"]);
        let json = build_params(&r, &[part("a.strings"), part("b.strings")]);
        assert!(json.contains(r#""fileMappings""#), "{json}");
        assert!(json.contains(r#""fileName":"a.strings""#), "{json}");
        assert!(json.contains(r#""languageTag":"ar""#), "{json}");
        assert!(json.contains(r#""format":"STRINGS""#), "{json}");
    }

    #[test]
    fn collect_files_walks_sorted_and_skips_dotfiles() {
        let dir = std::env::temp_dir().join(format!("wordiy_push_walk_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("values-ar")).unwrap();
        std::fs::create_dir_all(dir.join("values")).unwrap();
        std::fs::write(dir.join("values/strings.xml"), b"<resources/>").unwrap();
        std::fs::write(dir.join("values-ar/strings.xml"), b"<resources/>").unwrap();
        std::fs::write(dir.join(".DS_Store"), b"junk").unwrap();

        let files = collect_files(&dir).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["values-ar/strings.xml", "values/strings.xml"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn collect_files_does_not_follow_symlinks() {
        let dir = std::env::temp_dir().join(format!("wordiy_push_symlink_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("values")).unwrap();
        std::fs::write(dir.join("values/strings.xml"), b"<resources/>").unwrap();

        // A symlink back to an ancestor — following it would recurse forever.
        std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
        // A symlink to a file outside the tree — following it would upload it.
        let outside =
            std::env::temp_dir().join(format!("wordiy_push_outside_{}", std::process::id()));
        std::fs::write(&outside, b"secret").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("link.xml")).unwrap();

        // Must terminate, and collect only the real file (both symlinks skipped).
        let files = collect_files(&dir).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["values/strings.xml"]);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn summary_reports_in_sync_and_changes() {
        assert!(summary_line(&result(5, 0, 0)).contains("already in sync"));
        assert!(summary_line(&result(5, 2, 1)).contains("2 created, 1 updated"));
    }
}
