//! Background scanning: spawns a worker thread and reports progress.
//!
//! Volume roots on NTFS are scanned through the MFT when the process has
//! administrator rights; everything else (subdirectories, other file systems,
//! no elevation) falls back to a parallel directory walk.

pub mod mft;
pub mod walk;
pub mod win;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

use crossbeam_channel::{Receiver, bounded};

use crate::model::Model;
pub use walk::Progress;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Mft,
    Walk,
}

#[derive(Debug)]
pub struct ScanInfo {
    pub method: Method,
    /// Why the MFT scanner was not used, if it was attempted.
    pub fallback_reason: Option<String>,
}

pub type ScanResult = anyhow::Result<(Model, ScanInfo)>;

/// Scan `path` with the fastest available method.
pub fn scan(path: &Path, progress: &Progress, allow_mft: bool) -> ScanResult {
    let mut fallback_reason = None;
    if allow_mft && mft::volume_letter(path).is_some() {
        match mft::scan(path, progress) {
            Ok(m) => {
                return Ok((
                    m,
                    ScanInfo {
                        method: Method::Mft,
                        fallback_reason: None,
                    },
                ));
            }
            Err(e) => {
                if progress.cancel.load(Relaxed) {
                    return Err(e);
                }
                log::info!("MFT scan unavailable, walking directories: {e:#}");
                fallback_reason = Some(format!("{e:#}"));
                progress.reset_counters();
            }
        }
    }
    let m = walk::scan(path, progress)?;
    Ok((
        m,
        ScanInfo {
            method: Method::Walk,
            fallback_reason,
        },
    ))
}

pub struct ScanHandle {
    pub path: PathBuf,
    pub progress: Arc<Progress>,
    pub started: Instant,
    rx: Receiver<ScanResult>,
}

impl ScanHandle {
    /// Non-blocking poll; `Some` exactly once when the scan finishes.
    pub fn try_result(&self) -> Option<ScanResult> {
        self.rx.try_recv().ok()
    }

    pub fn cancel(&self) {
        self.progress.cancel.store(true, Relaxed);
    }
}

pub fn start(path: PathBuf) -> ScanHandle {
    let progress = Arc::new(Progress::default());
    let (tx, rx) = bounded(1);
    let p2 = progress.clone();
    let path2 = path.clone();
    std::thread::Builder::new()
        .name("scan".into())
        .spawn(move || {
            let res = scan(&path2, &p2, true);
            let _ = tx.send(res);
        })
        .expect("spawn scan thread");
    ScanHandle {
        path,
        progress,
        started: Instant::now(),
        rx,
    }
}
