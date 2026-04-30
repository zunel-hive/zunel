use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

/// Workspace-relative path guard.
#[derive(Debug, Clone, Default)]
pub struct PathPolicy {
    pub restrict_to: Option<PathBuf>,
    pub allowed_extras: Vec<PathBuf>,
}

impl PathPolicy {
    pub fn unrestricted() -> Self {
        Self::default()
    }

    pub fn restricted(workspace: &Path) -> Self {
        Self {
            restrict_to: Some(normalize(workspace)),
            allowed_extras: Vec::new(),
        }
    }

    pub fn with_media_dir(mut self, dir: &Path) -> Self {
        self.allowed_extras.push(normalize(dir));
        self
    }

    pub fn check(&self, path: &Path) -> Result<PathBuf> {
        let resolved = normalize(path);
        let Some(root) = &self.restrict_to else {
            return Ok(resolved);
        };
        if starts_with(&resolved, root) {
            return Ok(resolved);
        }
        for extra in &self.allowed_extras {
            if starts_with(&resolved, extra) {
                return Ok(resolved);
            }
        }
        Err(Error::PolicyViolation {
            tool: "<fs>".into(),
            reason: format!("path {resolved:?} is outside workspace {root:?}"),
        })
    }
}

fn normalize(path: &Path) -> PathBuf {
    // Non-filesystem path normalization: collapse `..` and `.` without
    // resolving symlinks (matches Python's `Path.resolve(strict=False)`
    // closely enough for the sandboxing check).
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(comp),
        }
    }
    out
}

fn starts_with(candidate: &Path, root: &Path) -> bool {
    candidate
        .components()
        .collect::<Vec<_>>()
        .starts_with(&root.components().collect::<Vec<_>>())
}
