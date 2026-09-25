//! Background scanning: spawns a worker thread and reports progress.

pub mod walk;
pub mod win;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{Receiver, bounded};

use crate::model::Model;
pub use walk::Progress;

pub struct ScanHandle {
    pub path: PathBuf,
    pub progress: Arc<Progress>,
    pub started: Instant,
    rx: Receiver<anyhow::Result<Model>>,
}

impl ScanHandle {
    /// Non-blocking poll; `Some` exactly once when the scan finishes.
    pub fn try_result(&self) -> Option<anyhow::Result<Model>> {
        self.rx.try_recv().ok()
    }

    pub fn cancel(&self) {
        self.progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
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
            let res = walk::scan(&path2, &p2);
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
