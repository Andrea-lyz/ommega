//! Bind an isolated UID's delegated scope to a live process instance.

use std::{fs, io, path::Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessIdentity {
    pub uid: i64,
    pub pid: i64,
    pub start_time: i64,
    pub boot_id: String,
}

impl ProcessIdentity {
    pub fn for_uid(uid: i64) -> io::Result<Self> {
        Self::for_uid_at(Path::new("/proc"), uid)
    }

    pub fn for_pid(pid: i64, uid: i64) -> io::Result<Option<Self>> {
        let root = Path::new("/proc");
        let boot_id = read_boot_id(root)?;
        Self::read_at(root, pid, uid, &boot_id)
    }

    fn for_uid_at(root: &Path, uid: i64) -> io::Result<Self> {
        if u32::try_from(uid).is_err() {
            return Err(io::Error::other("invalid process UID"));
        }
        let boot_id = read_boot_id(root)?;
        let mut oldest: Option<Self> = None;
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<i64>().ok())
            else {
                continue;
            };
            // Other process domains need not be readable by Keystore.
            let Ok(status) = fs::read_to_string(entry.path().join("status")) else {
                continue;
            };
            if effective_uid(&status).ok() != Some(uid) {
                continue;
            }
            let Some(identity) = Self::read_at(root, pid, uid, &boot_id)? else {
                continue;
            };
            // An isolated service can fork helpers sharing its UID. Its oldest
            // live process anchors the delegation; helpers cannot outlive it.
            if oldest.as_ref().is_none_or(|current| {
                (identity.start_time, identity.pid) < (current.start_time, current.pid)
            }) {
                oldest = Some(identity);
            }
        }
        oldest
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "isolated grantee is not live"))
    }

    fn read_at(root: &Path, pid: i64, uid: i64, boot_id: &str) -> io::Result<Option<Self>> {
        if pid <= 0 {
            return Ok(None);
        }
        let result: io::Result<Option<Self>> = (|| {
            let dir = root.join(pid.to_string());
            let before = parse_stat(&fs::read_to_string(dir.join("stat"))?, pid)?;
            let status = fs::read_to_string(dir.join("status"))?;
            let after = parse_stat(&fs::read_to_string(dir.join("stat"))?, pid)?;
            if before != after || !after.1 || effective_uid(&status)? != uid {
                return Ok(None);
            }
            Ok(Some(Self {
                uid,
                pid,
                start_time: after.0,
                boot_id: boot_id.to_owned(),
            }))
        })();
        match result {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            result => result,
        }
    }
}

fn read_boot_id(root: &Path) -> io::Result<String> {
    let boot_id = fs::read_to_string(root.join("sys/kernel/random/boot_id"))?;
    let boot_id = boot_id.trim();
    if boot_id.is_empty() {
        return Err(io::Error::other("empty kernel boot ID"));
    }
    Ok(boot_id.to_owned())
}

fn effective_uid(status: &str) -> io::Result<i64> {
    status
        .lines()
        .find_map(|line| {
            line.strip_prefix("Uid:")?
                .split_ascii_whitespace()
                .nth(1)?
                .parse()
                .ok()
        })
        .ok_or_else(|| io::Error::other("missing effective process UID"))
}

fn parse_stat(stat: &str, pid: i64) -> io::Result<(i64, bool)> {
    // comm may contain spaces and parentheses; fields start after the final ')'.
    let invalid = || io::Error::other("invalid process stat identity");
    let open = stat.find('(').ok_or_else(invalid)?;
    let close = stat.rfind(')').ok_or_else(invalid)?;
    if close <= open || stat[..open].trim().parse::<i64>().ok() != Some(pid) {
        return Err(invalid());
    }
    let mut fields = stat[close + 1..].split_ascii_whitespace();
    let state = fields.next().ok_or_else(invalid)?;
    let start_time = fields
        .nth(18)
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(invalid)?;
    if start_time < 0 {
        return Err(invalid());
    }
    Ok((start_time, !matches!(state, "Z" | "X" | "x")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct TestProc(PathBuf);

    impl TestProc {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "ommega-grant-proc-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("sys/kernel/random")).unwrap();
            fs::write(root.join("sys/kernel/random/boot_id"), "boot-a\n").unwrap();
            Self(root)
        }

        fn process(&self, pid: i64, uid: i64, start_time: i64, state: &str) {
            let dir = self.0.join(pid.to_string());
            fs::create_dir_all(&dir).unwrap();
            let mut fields = vec!["0".to_owned(); 20];
            fields[0] = state.to_owned();
            fields[19] = start_time.to_string();
            fs::write(
                dir.join("stat"),
                format!("{pid} (worker ) name) {}", fields.join(" ")),
            )
            .unwrap();
            fs::write(
                dir.join("status"),
                format!("Uid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
            )
            .unwrap();
        }
    }

    impl Drop for TestProc {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn process_identity_rejects_reused_pid_wrong_uid_and_dead_process() {
        let proc = TestProc::new();
        proc.process(42, 99_014, 123, "S");
        let original = ProcessIdentity::read_at(&proc.0, 42, 99_014, "boot-a")
            .unwrap()
            .unwrap();
        assert_eq!(original.start_time, 123);
        assert!(ProcessIdentity::read_at(&proc.0, 42, 99_015, "boot-a")
            .unwrap()
            .is_none());
        proc.process(42, 99_014, 124, "S");
        assert_ne!(
            ProcessIdentity::read_at(&proc.0, 42, 99_014, "boot-a")
                .unwrap()
                .unwrap(),
            original
        );
        proc.process(42, 99_014, 123, "Z");
        assert!(ProcessIdentity::read_at(&proc.0, 42, 99_014, "boot-a")
            .unwrap()
            .is_none());
        assert!(ProcessIdentity::read_at(&proc.0, 43, 99_014, "boot-a")
            .unwrap()
            .is_none());
    }

    #[test]
    fn uid_lookup_anchors_helpers_to_oldest_live_process_and_boot() {
        let proc = TestProc::new();
        proc.process(42, 99_014, 123, "S");
        proc.process(43, 99_014, 124, "S");
        proc.process(40, 199_014, 100, "S");
        let original = ProcessIdentity::for_uid_at(&proc.0, 99_014).unwrap();
        assert_eq!(original.pid, 42);
        assert_eq!(
            ProcessIdentity::for_uid_at(&proc.0, 199_014).unwrap().pid,
            40
        );
        fs::write(proc.0.join("sys/kernel/random/boot_id"), "boot-b").unwrap();
        assert_ne!(
            ProcessIdentity::for_uid_at(&proc.0, 99_014).unwrap(),
            original
        );
        assert!(ProcessIdentity::for_uid_at(&proc.0, -1).is_err());
    }

    #[test]
    fn stat_parser_rejects_malformed_or_different_process_records() {
        assert!(parse_stat("42 (name) S 0", 42).is_err());
        assert!(parse_stat("not a process", 42).is_err());
        assert!(parse_stat("43 (name) S 0", 42).is_err());
    }

    #[test]
    fn process_reader_matches_the_current_kernel_process_identity() {
        let uid = effective_uid(&fs::read_to_string("/proc/self/status").unwrap()).unwrap();
        let current = ProcessIdentity::for_pid(i64::from(std::process::id()), uid)
            .unwrap()
            .unwrap();
        assert_eq!(current.uid, uid);
        assert!(!current.boot_id.is_empty());
        assert!(ProcessIdentity::for_uid(-1).is_err());
    }
}
