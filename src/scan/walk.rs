//! Parallel directory walk.
//!
//! Each directory is listed with `win::list_dir`, which returns names,
//! attributes and sizes in 64 KiB batches (no extra syscall per entry).
//! Subdirectories are processed through rayon's work-stealing pool.

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
    /// Zero the counters (not the cancel flag) before a fallback rescan.
    pub fn reset_counters(&self) {
        self.files.store(0, Relaxed);
        self.dirs.store(0, Relaxed);
        self.bytes.store(0, Relaxed);
        self.errors.store(0, Relaxed);
    }

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
    let mut dir = RawDir {
        name,
        ..Default::default()
    };
    if progress.cancel.load(Relaxed) {
        return dir;
    }
    let mut entries = Vec::new();
    // An unreadable directory counts as one error; entries listed before a
    // failure are kept.
    let errors = u64::from(super::win::list_dir(path, &mut entries).is_err());

    let mut subdirs: Vec<(PathBuf, String)> = Vec::new();
    let mut bytes = 0u64;
    for e in entries {
        // Symlinks and junctions are skipped to avoid cycles / double counting.
        if e.is_link() {
            continue;
        }
        if e.is_dir() {
            subdirs.push((path.join(&e.name), e.name));
            continue;
        }
        let alloc = if e.attrs & (FILE_ATTRIBUTE_COMPRESSED | FILE_ATTRIBUTE_SPARSE_FILE) != 0 {
            super::win::compressed_size(&path.join(&e.name))
                .map(|s| round_up(s, cluster))
                .unwrap_or_else(|| round_up(e.size, cluster))
        } else {
            round_up(e.size, cluster)
        };
        bytes += e.size;
        dir.files.push(RawFile {
            name: e.name,
            size: e.size,
            alloc,
        });
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
