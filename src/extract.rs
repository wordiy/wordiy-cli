//! ZIP extraction for `pull`: write an export archive's files into a directory.
//!
//! The archive's entry paths define the on-disk layout (e.g. `values/strings.xml`,
//! `values-ar/strings.xml`); we mirror them verbatim. The archive bytes come over
//! the network, so it is treated as untrusted: every entry is decoded and validated
//! *in memory first* (zip-slip via [`zip::read::ZipFile::enclosed_name`], per-entry
//! and total size caps, at least one file) and only a fully valid archive is written
//! to disk. Nothing on disk is touched — including the `--empty-dir` wipe — until the
//! archive has passed validation, so a malformed or hostile response can never delete
//! existing translations or leave a partial extraction.

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use crate::error::{CliError, Result};

/// Per-file decompressed size cap (real Android resources are KBs).
const MAX_ENTRY_BYTES: u64 = 16 * 1024 * 1024;
/// Total decompressed size cap across all files.
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// Upper bound on the capacity hint taken from the (untrusted) size header.
const MAX_PREALLOC: u64 = 1024 * 1024;

/// Prepare the destination directory: if `empty`, remove it first, then ensure it
/// exists. Without `empty`, existing files are left in place (no pruning).
fn prepare_dir(path: &Path, empty: bool) -> Result<()> {
    if empty && path.exists() {
        fs::remove_dir_all(path)
            .map_err(|e| CliError::Message(format!("could not empty {}: {e}", path.display())))?;
    }
    fs::create_dir_all(path)
        .map_err(|e| CliError::Message(format!("could not create {}: {e}", path.display())))?;
    Ok(())
}

/// Decode and validate every file entry into memory, without touching disk.
/// Rejects path-traversal entries, oversized entries / archives, and empty archives.
fn decode_entries(bytes: &[u8]) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| CliError::Message(format!("invalid export archive: {e}")))?;

    let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let mut total: u64 = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| CliError::Message(format!("could not read archive entry: {e}")))?;

        if entry.is_dir() {
            continue;
        }

        let name = entry.name().to_string();

        // zip-slip guard: `enclosed_name()` is `None` for any path that escapes the
        // destination (`..`, absolute, drive prefix).
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| CliError::Message(format!("unsafe path in export archive: {name}")))?;

        // Cap the capacity hint so a lying size header can't trigger a huge alloc.
        let cap = entry.size().min(MAX_PREALLOC) as usize;
        let mut buf = Vec::with_capacity(cap);
        // Read at most MAX_ENTRY_BYTES + 1 so an over-cap (or bomb) entry is detected
        // without unbounded growth.
        entry
            .by_ref()
            .take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|e| CliError::Message(format!("could not read entry {name}: {e}")))?;

        if buf.len() as u64 > MAX_ENTRY_BYTES {
            return Err(CliError::Message(format!("export entry {name} is too large")));
        }
        total += buf.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(CliError::Message("export archive is too large".into()));
        }

        files.push((rel, buf));
    }

    if files.is_empty() {
        return Err(CliError::Message(
            "export archive contained no files — check your filters or the backend".into(),
        ));
    }
    Ok(files)
}

/// Extract the ZIP `bytes` into `dest`, returning the number of files written.
///
/// The archive is fully decoded and validated before `dest` is prepared (and before
/// `--empty-dir` removes anything), so an invalid archive never mutates the filesystem.
pub fn extract_zip(bytes: &[u8], dest: &Path, empty: bool) -> Result<usize> {
    let files = decode_entries(bytes)?;

    prepare_dir(dest, empty)?;

    for (rel, buf) in &files {
        let out = dest.join(rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                CliError::Message(format!("could not create {}: {e}", parent.display()))
            })?;
        }
        fs::write(&out, buf)
            .map_err(|e| CliError::Message(format!("could not write {}: {e}", out.display())))?;
    }
    Ok(files.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};
    use zip::write::{SimpleFileOptions, ZipWriter};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// A unique, fresh temp directory for a test (no external deps).
    fn temp_dir(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("wordiy_extract_{tag}_{}_{n}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zw = ZipWriter::new(&mut cursor);
            for (name, data) in entries {
                zw.start_file(*name, SimpleFileOptions::default()).unwrap();
                zw.write_all(data).unwrap();
            }
            zw.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn extracts_android_layout() {
        let zip = build_zip(&[
            ("values/strings.xml", b"<resources>base</resources>"),
            ("values-ar/strings.xml", b"<resources>ar</resources>"),
        ]);
        let dir = temp_dir("layout");

        let n = extract_zip(&zip, &dir, false).expect("extract");

        assert_eq!(n, 2);
        assert_eq!(
            fs::read_to_string(dir.join("values/strings.xml")).unwrap(),
            "<resources>base</resources>"
        );
        assert_eq!(
            fs::read_to_string(dir.join("values-ar/strings.xml")).unwrap(),
            "<resources>ar</resources>"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_zip_slip() {
        let zip = build_zip(&[("../escape.txt", b"evil")]);
        let dir = temp_dir("slip");

        let res = extract_zip(&zip, &dir, false);

        assert!(res.is_err(), "path-traversal entries must be rejected");
        assert!(!dir.parent().unwrap().join("escape.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_directory_entries() {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zw = ZipWriter::new(&mut cursor);
            zw.add_directory("values", SimpleFileOptions::default()).unwrap();
            zw.start_file("values/strings.xml", SimpleFileOptions::default()).unwrap();
            zw.write_all(b"x").unwrap();
            zw.finish().unwrap();
        }
        let dir = temp_dir("dirs");

        let n = extract_zip(&cursor.into_inner(), &dir, false).expect("extract");

        assert_eq!(n, 1); // only the file is counted
        assert!(dir.join("values/strings.xml").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_empty_archive() {
        let zip = build_zip(&[]); // valid container, no files
        let dir = temp_dir("empty");
        assert!(extract_zip(&zip, &dir, false).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_archive_does_not_wipe_existing_files() {
        // A pre-existing translation, and --empty-dir requested.
        let dir = temp_dir("nowipe");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("keep.xml"), b"important").unwrap();

        // A path-unsafe (invalid) archive must be rejected *before* the wipe.
        let zip = build_zip(&[("../escape.txt", b"evil")]);
        assert!(extract_zip(&zip, &dir, true).is_err());

        assert!(dir.join("keep.xml").exists(), "existing files must survive an invalid archive");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_dir_creates_then_optionally_empties() {
        let dir = temp_dir("prep");

        prepare_dir(&dir, false).unwrap();
        assert!(dir.exists());

        fs::write(dir.join("stale.txt"), b"old").unwrap();
        prepare_dir(&dir, false).unwrap(); // no empty → stale stays
        assert!(dir.join("stale.txt").exists());

        prepare_dir(&dir, true).unwrap(); // empty → stale removed
        assert!(dir.exists());
        assert!(!dir.join("stale.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
