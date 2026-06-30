//! `wordiy pull` — download translations into a local directory.
//!
//! This stage wires the export request: resolve the destination + API key, build
//! the request, and fetch the ZIP bytes via an [`ExportClient`]. Writing the
//! extracted files to disk lands in the next step; for now it reports the fetch so
//! the request path can be exercised against a running server.

use std::path::Path;

use crate::cli::PullArgs;
use crate::client::{ExportClient, ExportRequest, HttpExportClient};
use crate::context::Context;
use crate::error::{fail, Result};

pub fn run(ctx: &Context, args: &PullArgs) -> Result<()> {
    // A destination is mandatory. Once config loading exists this also accepts
    // `pull.path` from the config file.
    let Some(path) = args.path.as_deref() else {
        return fail("Missing --path: no destination directory was provided");
    };

    let Some(api_key) = ctx.api_key.clone() else {
        return fail("Missing API key: pass --api-key or set WORDIY_API_KEY");
    };

    let client = HttpExportClient::new(ctx.api_url.clone(), api_key);
    fetch(ctx, args, path, &client)
}

/// Build the request, fetch the ZIP, and report. Split from [`run`] so it can be
/// unit-tested with a fake [`ExportClient`] (no network).
fn fetch(ctx: &Context, args: &PullArgs, path: &Path, client: &dyn ExportClient) -> Result<()> {
    let req = ExportRequest::new(args.format, &args.languages, &args.states);

    if ctx.verbose {
        eprintln!(
            "[debug] POST {}/api/v1/project/export body={}",
            ctx.api_url,
            req.to_json()
        );
    }

    let bytes = client.export(&req)?;
    println!(
        "Fetched {} bytes of export data for {} — file extraction lands in the next step.",
        bytes.len(),
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
    use clap::Parser;

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

    /// Returns canned bytes without any network access.
    struct FakeClient(Vec<u8>);
    impl ExportClient for FakeClient {
        fn export(&self, _req: &ExportRequest) -> Result<Vec<u8>> {
            Ok(self.0.clone())
        }
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
    fn fetch_succeeds_with_a_fake_client() {
        let fake = FakeClient(b"PK\x03\x04 zip bytes".to_vec());
        let args = pull_args(&["wordiy", "pull", "--path", "./i18n"]);
        fetch(&ctx(), &args, Path::new("./i18n"), &fake).expect("should fetch");
    }
}
