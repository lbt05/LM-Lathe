//! Shared helpers for the integration tests.

use std::path::{Path, PathBuf};

/// A temporary directory that removes itself when it is dropped.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a unique temporary directory.
    pub fn new(prefix: &str) -> Self {
        let unique = uuid::Uuid::new_v4().simple().to_string();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique}"));
        std::fs::create_dir_all(&path).expect("failed to create the temporary directory");
        Self { path }
    }

    /// The path of the directory.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
