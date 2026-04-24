use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Tracks which paths the agent has `read_file`'d in this session and
/// the mtime at read time. `edit_file` uses this to refuse stale edits.
#[derive(Debug, Clone, Default)]
pub struct FileStateTracker {
    inner: Arc<Mutex<HashMap<PathBuf, SystemTime>>>,
}

impl FileStateTracker {
    pub fn mark_read(&self, path: PathBuf, mtime: SystemTime) {
        self.inner.lock().unwrap().insert(path, mtime);
    }
    pub fn last_read(&self, path: &Path) -> Option<SystemTime> {
        self.inner.lock().unwrap().get(path).copied()
    }
    pub fn invalidate(&self, path: &Path) {
        self.inner.lock().unwrap().remove(path);
    }
}
