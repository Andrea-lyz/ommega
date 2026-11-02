use std::{fs, io::Write as _, path::Path};

use anyhow::{Context, Result, ensure};

/// publishes a new private artifact only after its temporary contents pass verification
pub fn write_new_verified(
    path: &Path,
    bytes: &[u8],
    verify: impl Fn(&[u8]) -> Result<()>,
) -> Result<()> {
    let parent = path.parent().context("private output has no parent")?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder
        .create(parent)
        .context("create private output directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    let stored = zeroize::Zeroizing::new(fs::read(temporary.path())?);
    ensure!(
        stored.as_slice() == bytes,
        "temporary artifact readback differs"
    );
    verify(&stored)?;
    let saved = temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)
        .context("publish private output without overwriting")?;
    let result = (|| {
        let stored = zeroize::Zeroizing::new(fs::read(path)?);
        ensure!(
            stored.as_slice() == bytes,
            "published artifact readback differs"
        );
        verify(&stored)
    })();
    if result.is_err() && same_file(&saved, path) {
        fs::remove_file(path).context("remove failed private output")?;
    }
    result
}

#[cfg(unix)]
fn same_file(file: &fs::File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    file.metadata()
        .ok()
        .zip(fs::symlink_metadata(path).ok())
        .is_some_and(|(saved, current)| {
            saved.dev() == current.dev() && saved.ino() == current.ino()
        })
}

#[cfg(not(unix))]
fn same_file(_file: &fs::File, _path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::write_new_verified;
    use anyhow::ensure;
    use std::{cell::Cell, fs};

    #[test]
    fn writes_private_output_after_two_readback_checks_without_overwriting() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("keybox.xml");
        let calls = Cell::new(0);
        write_new_verified(&path, b"secret", |value| {
            ensure!(value == b"secret");
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(write_new_verified(&path, b"new", |_| Ok(())).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"secret");
    }

    #[test]
    fn failed_validation_removes_only_the_new_output() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("keybox.xml");
        let calls = Cell::new(0);
        let result = write_new_verified(&path, b"secret", |_| {
            calls.set(calls.get() + 1);
            ensure!(calls.get() < 2, "readback rejected");
            Ok(())
        });
        assert!(result.is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn creates_private_parents_and_never_follows_destination_symlinks() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("private/nested/keybox.xml");
        write_new_verified(&path, b"secret", |_| Ok(())).unwrap();
        for parent in [
            path.parent().unwrap(),
            path.parent().unwrap().parent().unwrap(),
        ] {
            assert_eq!(
                fs::metadata(parent).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        let target = root.path().join("target");
        fs::write(&target, b"keep").unwrap();
        let link = root.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(write_new_verified(&link, b"new", |_| Ok(())).is_err());
        assert_eq!(fs::read(target).unwrap(), b"keep");
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    }

    #[test]
    fn cleanup_does_not_delete_a_concurrently_replaced_output() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("keybox.xml");
        let calls = Cell::new(0);
        let result = write_new_verified(&path, b"secret", |_| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                fs::remove_file(&path)?;
                fs::write(&path, b"replacement")?;
                anyhow::bail!("readback rejected");
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(fs::read(path).unwrap(), b"replacement");
    }
}
