//! Parallel directory walk built on `std::fs::read_dir`.
//!
//! On Windows `read_dir` maps to `FindFirstFileExW`/`FindNextFileW` and the
//! `DirEntry` carries the full `WIN32_FIND_DATAW`, so `file_type()` and
//! `metadata()` are free (no extra syscalls per entry). Subdirectories are
//! processed through rayon's work-stealing pool.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

use rayon::prelude::*;

use crate::model::{Model, RawDir, RawFile};

const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x800;
const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;

#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn snapshot(&self) -> (u64, u64, u64, u64) {
        (
            self.files.load(Relaxed),
            self.dirs.load(Relaxed),
            self.bytes.load(Relaxed),
            self.errors.load(Relaxed),
        )
    }
}

/// Scan `root` and pack the result into a [`Model`].
pub fn scan(root: &Path, progress: &Progress) -> anyhow::Result<Model> {
    let root_path = root.to_string_lossy().into_owned();
    let cluster = super::win::cluster_size(root);
    let name = root_display_name(root);
    let raw = scan_dir(root, name, cluster, progress);
    if progress.cancel.load(Relaxed) {
        anyhow::bail!("scan cancelled");
    }
    Ok(Model::from_raw(raw, root_path, cluster))
}

fn root_display_name(root: &Path) -> String {
    match root.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => root.to_string_lossy().trim_end_matches('\\').to_string(),
    }
}

#[inline]
fn round_up(size: u64, cluster: u64) -> u64 {
    if size == 0 {
        0
    } else {
        size.div_ceil(cluster) * cluster
    }
}

fn scan_dir(path: &Path, name: String, cluster: u64, progress: &Progress) -> RawDir {
    use std::os::windows::fs::MetadataExt;

    let mut dir = RawDir {
        name,
        ..Default::default()
    };
    if progress.cancel.load(Relaxed) {
        return dir;
    }
    let rd = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(_) => {
            progress.errors.fetch_add(1, Relaxed);
            return dir;
        }
    };

    let mut subdirs: Vec<(PathBuf, String)> = Vec::new();
    let mut bytes = 0u64;
    let mut errors = 0u64;
    for entry in rd {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                errors += 1;
                continue;
            }
        };
        let Ok(ft) = entry.file_type() else {
            errors += 1;
            continue;
        };
        // Symlinks and junctions are skipped to avoid cycles / double counting.
        if ft.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if ft.is_dir() {
            subdirs.push((entry.path(), name));
            continue;
        }
        let Ok(md) = entry.metadata() else {
            errors += 1;
            continue;
        };
        let size = md.len();
        let attrs = md.file_attributes();
        let alloc = if attrs & (FILE_ATTRIBUTE_COMPRESSED | FILE_ATTRIBUTE_SPARSE_FILE) != 0 {
            super::win::compressed_size(&entry.path())
                .map(|s| round_up(s, cluster))
                .unwrap_or_else(|| round_up(size, cluster))
        } else {
            round_up(size, cluster)
        };
        bytes += size;
        dir.files.push(RawFile { name, size, alloc });
    }

    progress.files.fetch_add(dir.files.len() as u64, Relaxed);
    progress.dirs.fetch_add(1, Relaxed);
    progress.bytes.fetch_add(bytes, Relaxed);
    if errors > 0 {
        progress.errors.fetch_add(errors, Relaxed);
    }

    if subdirs.is_empty() {
        return dir;
    }
    dir.subdirs = subdirs
        .into_par_iter()
        .map(|(p, n)| scan_dir(&p, n, cluster, progress))
        .collect();
    dir
}
