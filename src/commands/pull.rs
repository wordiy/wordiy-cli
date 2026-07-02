//! `wordiy pull` — download translations into a local directory.
//!
//! Resolves the destination + API key, fetches the export ZIP via an
//! [`ExportClient`], and extracts its files into `--path`.

use std::path::Path;

use crate::cli::PullArgs;
use crate::client::{ExportClient, ExportQuery, ExportRequest, HttpExportClient};
use crate::context::Context;
use crate::error::{fail, Result};
use crate::extract::extract_zip;

pub fn run(ctx: &Context, args: &PullArgs) -> Result<()> {
    // A destination is mandatory. Once config loading exists this also accepts
    // `pull.path` from the config file.
    let Some(path) = args.path.as_deref() else {
        return fail("Missing --path: no destination directory was provided");
    };

    let Some(api_key) = ctx.api_key.clone() else {
        return fail("Missing API key: pass --api-key or set WORDIY_API_KEY");
    };

    let client = HttpExportClient::new(ctx.api_url.clone(), api_key, ctx.verbose);
    pull_into(args, path, &client)
}

/// Fetch + extract. Split from [`run`] so it can be unit-tested with a fake
/// [`ExportClient`] (no network).
fn pull_into(args: &PullArgs, path: &Path, client: &dyn ExportClient) -> Result<()> {
    let req = ExportRequest::new(ExportQuery {
        format: args.format,
        languages: &args.languages,
        states: &args.states,
        tags: &args.tags,
        exclude_tags: &args.exclude_tags,
        key_prefix: args.key_prefix.as_deref(),
    });
    let bytes = client.export(&req)?;
    let count = extract_zip(&bytes, path, args.empty_dir)?;
    println!("Pulled {count} file(s) into {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
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

    /// Returns canned ZIP bytes without any network access.
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
    fn errors_when_path_is_missing() {
        let err = run(&ctx(), &pull_args(&["wordiy", "pull"])).expect_err("needs a path");
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn errors_when_api_key_is_missing() {
        let args = pull_args(&["wordiy", "pull", "--path", "./i18n"]);
        let err = run(&ctx(), &args).expect_err("needs a key");
        assert_eq!(err.exit_code(), 1);
    }

    #[test]
    fn pull_into_writes_files_with_a_fake_client() {
        let fake = FakeClient(one_file_zip("values/strings.xml", b"<resources/>"));
        let args = pull_args(&["wordiy", "pull"]);
        let dir = std::env::temp_dir().join(format!("wordiy_pull_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        pull_into(&args, &dir, &fake).expect("should pull");

        assert!(dir.join("values/strings.xml").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
