// Copyright 2026, The Android Open Source Project
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::format;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::{
    fd::AsRawFd,
    unix::fs::{MetadataExt, OpenOptionsExt},
};
use std::path::Path;

use anyhow::Context;

/// Serialize cooperating writers across processes. The directory fd remains
/// locked across the comparison and atomic rename, and closes on every exit.
/// A busy editor fails immediately, without stale persistent lock files.
pub fn atomic_compare_replace_preserving_metadata(
    path: &Path,
    expected: Option<&[u8]>,
    contents: &[u8],
    default_mode: u32,
    default_uid: u32,
    default_gid: u32,
) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("path has no parent"))?;
    let directory_lock = fs::File::open(parent)?;
    if unsafe { libc::flock(directory_lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let current = match fs::read(path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if current.as_deref() != expected {
        return Err(io::Error::other("Config changed; reload before saving"));
    }
    atomic_replace_preserving_metadata(path, contents, default_mode, default_uid, default_gid)
}

pub fn atomic_replace_preserving_metadata(
    path: &Path,
    contents: &[u8],
    default_mode: u32,
    default_uid: u32,
    default_gid: u32,
) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    fs::create_dir_all(parent)?;

    let (mode, uid, gid) = match fs::metadata(path) {
        Ok(metadata) => (metadata.mode() & 0o7777, metadata.uid(), metadata.gid()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            (default_mode, default_uid, default_gid)
        }
        Err(error) => return Err(error),
    };
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let temp_path = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));

    let result = (|| {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(mode)
            .open(&temp_path)?;
        file.write_all(contents)?;
        if unsafe { libc::fchown(file.as_raw_fd(), uid, gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) } != 0 {
            return Err(io::Error::last_os_error());
        }
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, path)?;
        std::fs::File::open(parent)?.sync_all()
    })();

    if result.is_err() {
        let _ = fs::remove_file(temp_path);
    }
    result
}

pub fn backup_file_with_reason(
    path: &Path,
    backup: &Path,
    reason_header: &str,
    reason: &str,
    allow_copy_fallback: bool,
) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    if backup.exists() {
        fs::remove_file(backup)
            .with_context(|| format!("failed to remove stale backup {}", backup.display()))?;
    }

    if let Err(rename_error) = fs::rename(path, backup) {
        if !allow_copy_fallback {
            return Err(rename_error)
                .with_context(|| format!("failed to move invalid file to {}", backup.display()));
        }

        fs::copy(path, backup)
            .with_context(|| {
                format!(
                    "failed to copy invalid file to backup {} after rename error {rename_error}",
                    backup.display()
                )
            })
            .and_then(|_| {
                fs::remove_file(path)
                    .with_context(|| format!("failed to remove original file {}", path.display()))
            })
            .with_context(|| format!("failed to move invalid file to {}", backup.display()))?;
    }

    let mut file = OpenOptions::new()
        .append(true)
        .open(backup)
        .with_context(|| format!("failed to open backup {}", backup.display()))?;
    writeln!(file)?;
    writeln!(file, "# {reason_header}:")?;
    for line in reason.lines() {
        writeln!(file, "# {line}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn policy_compare_replace_rejects_stale_and_concurrent_writers() {
        let directory =
            std::env::temp_dir().join(format!("ommega-policy-cas-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = Arc::new(directory.join("auth-signing.toml"));
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        atomic_compare_replace_preserving_metadata(&path, None, b"original", 0o600, uid, gid)
            .unwrap();
        assert!(atomic_compare_replace_preserving_metadata(
            &path,
            None,
            b"overwrite",
            0o600,
            uid,
            gid
        )
        .is_err());
        let barrier = Arc::new(Barrier::new(2));
        let threads: Vec<_> = [b"first".as_slice(), b"second".as_slice()]
            .into_iter()
            .map(|value| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    atomic_compare_replace_preserving_metadata(
                        &path,
                        Some(b"original"),
                        value,
                        0o600,
                        uid,
                        gid,
                    )
                    .is_ok()
                })
            })
            .collect();
        let successes = threads
            .into_iter()
            .map(|thread| usize::from(thread.join().unwrap()))
            .sum::<usize>();
        assert_eq!(successes, 1);
        let saved = fs::read(&*path).unwrap();
        assert!(saved == b"first" || saved == b"second");
        assert_eq!(fs::metadata(&*path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            fs::read_dir(&directory).unwrap().count(),
            1,
            "temporary files must be removed"
        );
        fs::remove_file(&*path).unwrap();
        fs::remove_dir(&directory).unwrap();
    }
}
