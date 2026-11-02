//! Minimal replacement for duck-core::AppPaths and its fs helpers.
//!
//! The original crate receives a root plus a writable data root from the host
//! CLI. Here the caller passes the two directories explicitly, which keeps the
//! engine usable from a standalone binary without the duckd feature layer.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
    pub data_root: PathBuf,
    pub var_dir: PathBuf,
    pub profile_path: PathBuf,
    pub profile_secrets_path: PathBuf,
    pub outputs_dir: PathBuf,
    pub tmp_dir: PathBuf,
    pub logs_dir: PathBuf,
}

impl AppPaths {
    pub fn for_root(root: &Path, data_root: &Path) -> Self {
        let var_dir = data_root.join("var");
        let logs_dir = var_dir.join("logs");
        Self {
            root: root.to_path_buf(),
            data_root: data_root.to_path_buf(),
            var_dir: var_dir.clone(),
            profile_path: var_dir.join("profile.toml"),
            profile_secrets_path: var_dir.join("profile.secrets.toml"),
            outputs_dir: var_dir.join("outputs"),
            tmp_dir: var_dir.join("tmp"),
            logs_dir,
        }
    }

    /// Relative inputs resolve inside the root, absolute inputs stay untouched.
    pub fn resolve_in_root(&self, path: &str) -> PathBuf {
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.root.join(candidate)
        }
    }
}

pub mod fs_helpers {
    use super::*;

    pub fn read_optional(path: &Path) -> Result<Option<String>> {
        match fs::read_to_string(path) {
            Ok(content) => Ok(Some(content)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
        }
    }

    pub fn write_string_atomic(path: &Path, contents: &str) -> Result<()> {
        write_bytes_atomic(path, contents.as_bytes())
    }

    pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(
            ".{}.tmp-{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file"),
            std::process::id()
        ));
        fs::write(&temporary, bytes)?;
        fs::rename(&temporary, path)?;
        Ok(())
    }
}
