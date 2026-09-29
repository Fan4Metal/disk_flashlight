//! Parallel directory walk.
//!
//! Each directory is listed with `win::list_dir`, which returns names,
//! attributes and sizes in 64 KiB batches (no extra syscall per entry).
//! Subdirectories are processed through rayon's work-stealing pool.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

use rayon::prelude::*;

use crate::model::{Model, RawDir, RawFile};

const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x800;
const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;

/// A folder the walk could not list.
#[derive(Clone, Debug)]
pub struct ScanError {
    pub path: String,
    /// The system's message, e.g. "Access is denied. (os error 5)".
    pub message: String,
}

/// At most this many unreadable folders are kept; all are counted.
pub const MAX_KEPT_ERRORS: usize = 1000;

#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    /// The first `MAX_KEPT_ERRORS` unreadable folders, in no order.
    pub failed: Mutex<Vec<ScanError>>,
    pub cancel: AtomicBool,
}

impl Progress {
    /// Zero the counters (not the cancel flag) before a fallback rescan.
    pub fn reset_counters(&self) {
        self.files.store(0, Relaxed);
        self.dirs.store(0, Relaxed);
        self.bytes.store(0, Relaxed);
        self.errors.store(0, Relaxed);
        self.failed.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// The unreadable folders kept so far, leaving none.
    pub fn take_failed(&self) -> Vec<ScanError> {
        std::mem::take(&mut *self.failed.lock().unwrap_or_else(|e| e.into_inner()))
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

/// Scan `root` and pack the result into a [`Model`]. A root that is missing
/// or not a folder fails the scan (rather than giving an empty chart).
pub fn scan(root: &Path, progress: &Progress) -> anyhow::Result<Model> {
    let meta = std::fs::metadata(root).map_err(|e| anyhow::anyhow!("{}: {e}", root.display()))?;
    if !meta.is_dir() {
        anyhow::bail!(tr!(
            format!("{} is not a folder", root.display()),
            format!("{} — это не папка", root.display())
        ));
    }
    let root_path = root.to_string_lossy().into_owned();
    let cluster = super::win::cluster_size(root);
    let vol = Volume {
        cluster,
        ntfs: super::win::file_system(root).is_some_and(|fs| fs.eq_ignore_ascii_case("NTFS")),
    };
    let name = root_display_name(root);
    let modified = meta.modified().ok().map_or(0, unix_time);
    let raw = scan_dir(root, name, modified, vol, progress);
    if progress.cancel.load(Relaxed) {
        anyhow::bail!(tr!("scan cancelled", "сканирование отменено"));
    }
    Ok(Model::from_raw(raw, root_path, cluster))
}

/// `t` as Unix seconds, 0 before 1970.
fn unix_time(t: std::time::SystemTime) -> u32 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().min(u32::MAX as u64) as u32)
}

fn root_display_name(root: &Path) -> String {
    match root.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => root.to_string_lossy().trim_end_matches('\\').to_string(),
    }
}

/// What the walk needs to know about the volume.
#[derive(Clone, Copy)]
struct Volume {
    cluster: u64,
    ntfs: bool,
}

/// Space on disk of a plain (not compressed, not sparse) file of `size`
/// bytes whose listing gives `listed` allocated bytes. On NTFS a small
/// file's data can live in its MFT record and take no cluster; its listed
/// allocation is then not a whole number of clusters, which a file with
/// clusters never has. Otherwise the size is rounded up to clusters: the
/// listed allocation of a hard link's other names can be out of date.
fn plain_alloc(size: u64, listed: u64, vol: Volume) -> u64 {
    if vol.ntfs && !listed.is_multiple_of(vol.cluster) {
        0
    } else {
        round_up(size, vol.cluster)
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

/// `modified` is the folder's own last write time, from its parent's
/// listing.
fn scan_dir(path: &Path, name: String, modified: u32, vol: Volume, progress: &Progress) -> RawDir {
    let mut dir = RawDir {
        name,
        modified,
        ..Default::default()
    };
    if progress.cancel.load(Relaxed) {
        return dir;
    }
    let mut entries = Vec::new();
    // An unreadable directory counts as one error; entries listed before a
    // failure are kept.
    if let Err(e) = super::win::list_dir(path, &mut entries) {
        progress.errors.fetch_add(1, Relaxed);
        let mut failed = progress.failed.lock().unwrap_or_else(|e| e.into_inner());
        if failed.len() < MAX_KEPT_ERRORS {
            failed.push(ScanError {
                path: path.to_string_lossy().into_owned(),
                message: e.to_string(),
            });
        }
    }

    let mut subdirs: Vec<(PathBuf, String, u32)> = Vec::new();
    let mut bytes = 0u64;
    for e in entries {
        // Symlinks and junctions are skipped to avoid cycles / double counting.
        if e.is_link() {
            continue;
        }
        if e.is_dir() {
            subdirs.push((path.join(&e.name), e.name, e.modified));
            continue;
        }
        let alloc = if e.attrs & (FILE_ATTRIBUTE_COMPRESSED | FILE_ATTRIBUTE_SPARSE_FILE) != 0 {
            super::win::compressed_size(&path.join(&e.name))
                .map(|s| round_up(s, vol.cluster))
                .unwrap_or_else(|| round_up(e.size, vol.cluster))
        } else {
            plain_alloc(e.size, e.alloc, vol)
        };
        bytes += e.size;
        dir.files.push(RawFile {
            name: e.name,
            size: e.size,
            alloc,
            modified: e.modified,
        });
    }

    progress.files.fetch_add(dir.files.len() as u64, Relaxed);
    progress.dirs.fetch_add(1, Relaxed);
    progress.bytes.fetch_add(bytes, Relaxed);

    if subdirs.is_empty() {
        return dir;
    }
    dir.subdirs = subdirs
        .into_par_iter()
        .map(|(p, n, t)| scan_dir(&p, n, t, vol, progress))
        .collect();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_in_the_mft_record_takes_no_cluster() {
        let ntfs = Volume { cluster: 4096, ntfs: true };
        // As listed on NTFS: 10 bytes kept in the record show as 16.
        assert_eq!(plain_alloc(10, 16, ntfs), 0);
        assert_eq!(plain_alloc(900, 4096, ntfs), 4096);
        assert_eq!(plain_alloc(0, 0, ntfs), 0);
        // A hard link listed with an old allocation: the size decides.
        assert_eq!(plain_alloc(50_000, 8192, ntfs), 53_248);
        // Other file systems (a share counting 512-byte blocks) round up.
        let other = Volume { ntfs: false, ..ntfs };
        assert_eq!(plain_alloc(1000, 1024, other), 4096);
    }

    /// On a real NTFS volume (the temporary folder), a tiny file costs no
    /// cluster and a larger one whole clusters.
    #[test]
    fn walk_counts_resident_files_as_free() {
        let dir = std::env::temp_dir().join(format!("df_walk_{}", std::process::id()));
        if !super::super::win::file_system(&std::env::temp_dir()).is_some_and(|fs| fs == "NTFS") {
            return;
        }
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tiny"), b"0123456789").unwrap();
        std::fs::write(dir.join("big"), vec![1u8; 100_000]).unwrap();
        let m = scan(&dir, &Progress::default()).unwrap();
        let alloc = |name: &str| m.node(m.children(0).find(|&c| m.name(c) == name).unwrap()).alloc;
        assert_eq!(alloc("tiny"), 0);
        assert_eq!(alloc("big"), 100_000u64.div_ceil(m.cluster_size) * m.cluster_size);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
