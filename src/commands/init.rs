//! `wordiy init` — write a starter `.wordiyrc.toml` in the current directory.

use std::fs;
use std::path::Path;

use crate::cli::InitArgs;
use crate::error::{fail, CliError, Result};

/// The documented config template, embedded at build time so it stays identical to the
/// repo's `.wordiyrc.example.toml` (single source of truth).
const TEMPLATE: &str = include_str!("../../.wordiyrc.example.toml");

/// The file `init` creates.
const CONFIG_FILE: &str = ".wordiyrc.toml";

/// Write a starter `.wordiyrc.toml` into the current directory, refusing to overwrite
/// an existing one unless `--force`.
pub fn run(args: &InitArgs) -> Result<()> {
    write_config(Path::new(CONFIG_FILE), args.force)?;
    println!("Created {CONFIG_FILE} — edit it to configure your project.");
    Ok(())
}

/// Write the config template to `target`, refusing to overwrite unless `force`.
fn write_config(target: &Path, force: bool) -> Result<()> {
    if target.exists() && !force {
        return fail(format!(
            "{} already exists — pass --force to overwrite",
            target.display()
        ));
    }
    fs::write(target, TEMPLATE)
        .map_err(|e| CliError::Message(format!("could not write {}: {e}", target.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir() -> std::path::PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("wordiy_init_{}_{n}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn writes_template_then_refuses_to_overwrite() {
        let dir = temp_dir();
        let target = dir.join(CONFIG_FILE);

        write_config(&target, false).expect("first write");
        assert!(
            fs::read_to_string(&target).unwrap().contains("[pull]"),
            "the template should be written"
        );

        // Refuses to clobber without --force.
        assert!(write_config(&target, false).is_err());
        // --force overwrites.
        write_config(&target, true).expect("force write");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn embedded_template_is_a_valid_config() {
        // Guard against the template ever drifting into something that won't parse.
        let parsed: std::result::Result<crate::config::Config, _> = toml::from_str(TEMPLATE);
        assert!(parsed.is_ok(), "embedded template must parse as a Config");
    }
}
