//! `wordiy pull` — download translations into a local directory.
//!
//! Resolves the destination + filters (a CLI flag overrides the config file, which
//! overrides the built-in default), fetches the export ZIP via an [`ExportClient`],
//! and extracts its files into the destination.

use std::path::{Path, PathBuf};

use clap::ValueEnum;

use crate::cli::{Format, PullArgs, State};
use crate::client::{ExportClient, ExportQuery, ExportRequest, HttpExportClient};
use crate::config::{LoadedConfig, PullConfig};
use crate::context::Context;
use crate::error::{fail, CliError, Result};
use crate::extract::{check_path_not_a_file, extract_zip};

/// Pull options after merging CLI flags over the config file over defaults.
struct ResolvedPull {
    path: PathBuf,
    format: Format,
    languages: Vec<String>,
    states: Vec<State>,
    tags: Vec<String>,
    exclude_tags: Vec<String>,
    key_prefix: Option<String>,
    empty_dir: bool,
}

pub fn run(ctx: &Context, args: &PullArgs, loaded: &LoadedConfig) -> Result<()> {
    let resolved = resolve(args, &loaded.config.pull, &loaded.dir)?;
    check_path_not_a_file(&resolved.path)?;

    let Some(api_key) = ctx.api_key.clone() else {
        return fail("Missing API key: pass --api-key or set WORDIY_API_KEY");
    };

    let client = HttpExportClient::new(ctx.api_url.clone(), api_key, ctx.verbose);
    pull_into(&resolved, &client)
}

/// Merge CLI flags (highest priority) with the config file, then defaults. A relative
/// `pull.path` from the config resolves against the config file's directory.
fn resolve(args: &PullArgs, cfg: &PullConfig, config_dir: &Path) -> Result<ResolvedPull> {
    let path = match &args.path {
        Some(p) => p.clone(),
        None => match &cfg.path {
            Some(p) if p.is_absolute() => p.clone(),
            Some(p) => config_dir.join(p),
            None => return fail("Missing --path: pass --path or set pull.path in the config"),
        },
    };

    let format = match args.format {
        Some(f) => f,
        None => match &cfg.format {
            Some(s) => parse_enum::<Format>(s, "pull.format")?,
            None => Format::default(),
        },
    };

    let states = if args.states.is_empty() {
        cfg.states
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|s| parse_enum::<State>(s, "pull.states"))
            .collect::<Result<Vec<_>>>()?
    } else {
        args.states.clone()
    };

    // Tri-state so the CLI can force empty-dir on OR off, overriding the config.
    let empty_dir = if args.no_empty_dir {
        false
    } else if args.empty_dir {
        true
    } else {
        cfg.empty_dir.unwrap_or(false)
    };

    Ok(ResolvedPull {
        path,
        format,
        languages: pick(&args.languages, cfg.languages.as_deref()),
        states,
        tags: pick(&args.tags, cfg.tags.as_deref()),
        exclude_tags: pick(&args.exclude_tags, cfg.exclude_tags.as_deref()),
        key_prefix: args.key_prefix.clone().or_else(|| cfg.key_prefix.clone()),
        empty_dir,
    })
}

/// The CLI list wins when non-empty; otherwise fall back to the config list (or empty).
fn pick(cli: &[String], cfg: Option<&[String]>) -> Vec<String> {
    if cli.is_empty() {
        cfg.unwrap_or(&[]).to_vec()
    } else {
        cli.to_vec()
    }
}

/// Parse a config string into a CLI value enum, reusing clap's accepted values.
fn parse_enum<T: ValueEnum>(value: &str, field: &str) -> Result<T> {
    T::from_str(value, false)
        .map_err(|_| CliError::Message(format!("invalid {field} value in config: '{value}'")))
}

fn pull_into(resolved: &ResolvedPull, client: &dyn ExportClient) -> Result<()> {
    let req = ExportRequest::new(ExportQuery {
        format: resolved.format,
        languages: &resolved.languages,
        states: &resolved.states,
        tags: &resolved.tags,
        exclude_tags: &resolved.exclude_tags,
        key_prefix: resolved.key_prefix.as_deref(),
    });
    let bytes = client.export(&req)?;
    let count = extract_zip(&bytes, &resolved.path, resolved.empty_dir)?;
    println!("Pulled {count} file(s) into {}", resolved.path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
    use crate::config::{Config, LoadedConfig};
    use clap::Parser;
    use std::io::{Cursor, Write};

    fn ctx() -> Context {
        Context {
            api_url: "http://localhost:3001".into(),
            api_key: None,
            verbose: false,
        }
    }

    fn pull_args(argv: &[&str]) -> PullArgs {
        let Command::Pull(args) = Cli::try_parse_from(argv).expect("parses").command;
        args
    }

    fn empty_loaded() -> LoadedConfig {
        LoadedConfig {
            config: Config::default(),
            dir: PathBuf::from("."),
        }
    }

    struct FakeClient(Vec<u8>);
    impl ExportClient for FakeClient {
        fn export(&self, _req: &ExportRequest) -> Result<Vec<u8>> {
            Ok(self.0.clone())
        }
    }

    fn one_file_zip(name: &str, data: &[u8]) -> Vec<u8> {
        use zip::write::{SimpleFileOptions, ZipWriter};
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zw = ZipWriter::new(&mut cursor);
            zw.start_file(name, SimpleFileOptions::default()).unwrap();
            zw.write_all(data).unwrap();
            zw.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn errors_when_no_path_from_flag_or_config() {
        let err = run(&ctx(), &pull_args(&["wordiy", "pull"]), &empty_loaded()).unwrap_err();
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn errors_when_api_key_is_missing() {
        let args = pull_args(&["wordiy", "pull", "--path", "./i18n"]);
        let err = run(&ctx(), &args, &empty_loaded()).unwrap_err();
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn errors_when_path_is_an_existing_file() {
        let file = std::env::temp_dir().join(format!("wordiy_pull_isfile_{}", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        let args = pull_args(&["wordiy", "pull", "--path", file.to_str().unwrap()]);
        let err = run(&ctx(), &args, &empty_loaded()).expect_err("path is a file");
        assert_eq!(err.exit_code(), 1);
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn config_fills_path_and_filters_when_flags_omitted() {
        let cfg = PullConfig {
            path: Some(PathBuf::from("res")),
            tags: Some(vec!["mobile".into()]),
            states: Some(vec!["REVIEWED".into()]),
            ..Default::default()
        };
        let resolved = resolve(&pull_args(&["wordiy", "pull"]), &cfg, Path::new("/base")).unwrap();

        // Relative config path resolves against the config's directory.
        assert_eq!(resolved.path, PathBuf::from("/base/res"));
        assert_eq!(resolved.tags, vec!["mobile"]);
        assert_eq!(resolved.states, vec![State::Reviewed]);
        assert_eq!(resolved.format, Format::AndroidXml); // default
    }

    #[test]
    fn cli_flags_override_config() {
        let cfg = PullConfig {
            path: Some(PathBuf::from("res")),
            tags: Some(vec!["mobile".into()]),
            ..Default::default()
        };
        let resolved = resolve(
            &pull_args(&["wordiy", "pull", "--path", "/abs", "-t", "checkout"]),
            &cfg,
            Path::new("/base"),
        )
        .unwrap();

        assert_eq!(resolved.path, PathBuf::from("/abs"));
        assert_eq!(resolved.tags, vec!["checkout"]);
    }

    #[test]
    fn empty_dir_cli_overrides_config() {
        let on = PullConfig {
            path: Some(PathBuf::from("res")),
            empty_dir: Some(true),
            ..Default::default()
        };
        // config alone → true
        let r = resolve(&pull_args(&["wordiy", "pull"]), &on, Path::new("/b")).unwrap();
        assert!(r.empty_dir);
        // --no-empty-dir turns a config `true` back off
        let r = resolve(&pull_args(&["wordiy", "pull", "--no-empty-dir"]), &on, Path::new("/b")).unwrap();
        assert!(!r.empty_dir);
        // --empty-dir forces it on even when config is false
        let off = PullConfig {
            path: Some(PathBuf::from("res")),
            empty_dir: Some(false),
            ..Default::default()
        };
        let r = resolve(&pull_args(&["wordiy", "pull", "--empty-dir"]), &off, Path::new("/b")).unwrap();
        assert!(r.empty_dir);
    }

    #[test]
    fn invalid_config_enum_is_rejected() {
        let cfg = PullConfig {
            path: Some(PathBuf::from("res")),
            states: Some(vec!["NONSENSE".into()]),
            ..Default::default()
        };
        assert!(resolve(&pull_args(&["wordiy", "pull"]), &cfg, Path::new("/base")).is_err());
    }

    #[test]
    fn pull_into_writes_files_with_a_fake_client() {
        let dir = std::env::temp_dir().join(format!("wordiy_pull_cfg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let resolved = ResolvedPull {
            path: dir.clone(),
            format: Format::AndroidXml,
            languages: vec![],
            states: vec![],
            tags: vec![],
            exclude_tags: vec![],
            key_prefix: None,
            empty_dir: false,
        };
        let fake = FakeClient(one_file_zip("values/strings.xml", b"<resources/>"));

        pull_into(&resolved, &fake).expect("should pull");

        assert!(dir.join("values/strings.xml").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
