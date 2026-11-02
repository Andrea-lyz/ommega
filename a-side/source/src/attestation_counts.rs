//! Persistent counters stored in the existing Ommega keystore database.
use std::{collections::BTreeMap, path::Path, time::Duration};

use rusqlite::{params, Connection, OpenFlags};

pub const DATABASE_PATH: &str = "/data/misc/keystore/ommega/data/keymaster.db";

/// Which producer a counter belongs to: the relay (B-side hardware attestation)
/// or this device's own software TA. The two never share rows, so the reader can
/// report the counters that match the active relay mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Remote,
    Local,
}

impl Source {
    fn count_table(self) -> &'static str {
        match self {
            Source::Remote => "ommega_attestation_count",
            Source::Local => "ommega_local_attestation_count",
        }
    }

    fn latest_table(self) -> &'static str {
        match self {
            Source::Remote => "ommega_attestation_latest",
            Source::Local => "ommega_local_attestation_latest",
        }
    }

    fn create_count(self) -> String {
        format!(
            "CREATE TABLE IF NOT EXISTS {} (
    package TEXT PRIMARY KEY NOT NULL,
    count INTEGER NOT NULL CHECK(count >= 0),
    shared INTEGER NOT NULL DEFAULT 0
)",
            self.count_table()
        )
    }

    fn create_latest(self) -> String {
        format!(
            "CREATE TABLE IF NOT EXISTS {} (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    latency_ms INTEGER,
    completed_at_ms INTEGER NOT NULL
)",
            self.latest_table()
        )
    }
}

#[derive(Debug, serde::Serialize, PartialEq, Eq)]
pub struct CountInfo {
    pub count: i64,
    pub shared: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct Snapshot {
    pub counts: Option<BTreeMap<String, CountInfo>>,
    pub last_b_latency_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct CountDelta {
    pub count: i64,
    pub shared: bool,
}

pub fn apply_batch_in_connection(
    source: Source,
    conn: &mut Connection,
    counts: &BTreeMap<String, CountDelta>,
    latest: Option<(Option<u64>, i64)>,
) -> rusqlite::Result<()> {
    if counts.is_empty() && latest.is_none() {
        return Ok(());
    }
    let transaction = conn.transaction()?;
    transaction.execute(&source.create_count(), [])?;
    transaction.execute(&source.create_latest(), [])?;
    for (package, delta) in counts {
        transaction.execute(
            &format!(
                "INSERT INTO {}(package, count, shared) VALUES (?1, ?2, ?3)
             ON CONFLICT(package) DO UPDATE SET count = min(count + excluded.count, 9007199254740991), shared = max(shared, excluded.shared)",
                source.count_table()
            ),
            params![package, delta.count, delta.shared],
        )?;
    }
    if let Some((latency_ms, completed_at_ms)) = latest {
        let latency = latency_ms.map(|value| value.min(9007199254740991) as i64);
        transaction.execute(
            &format!(
                "INSERT INTO {}(id, latency_ms, completed_at_ms) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET latency_ms=excluded.latency_ms, completed_at_ms=excluded.completed_at_ms
             WHERE excluded.completed_at_ms >= completed_at_ms",
                source.latest_table()
            ),
            params![latency, completed_at_ms],
        )?;
    }
    transaction.commit()
}

pub fn increment_in_connection(
    source: Source,
    conn: &mut Connection,
    packages: &[String],
    latency_ms: Option<u64>,
    completed_at_ms: i64,
) -> rusqlite::Result<()> {
    let mut counts = BTreeMap::new();
    for package in packages {
        counts.insert(
            package.clone(),
            CountDelta {
                count: 1,
                shared: packages.len() > 1,
            },
        );
    }
    if counts.is_empty() {
        return Ok(());
    }
    apply_batch_in_connection(source, conn, &counts, Some((latency_ms, completed_at_ms)))
}

pub fn increment_at(
    source: Source,
    path: &Path,
    packages: &[String],
    latency_ms: Option<u64>,
    completed_at_ms: i64,
) -> rusqlite::Result<()> {
    // READ_WRITE deliberately excludes CREATE: metrics never create a new DB.
    let mut conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(Duration::from_millis(250))?;
    increment_in_connection(source, &mut conn, packages, latency_ms, completed_at_ms)
}

pub fn apply_batch_at(
    source: Source,
    path: &Path,
    counts: &BTreeMap<String, CountDelta>,
    latest: Option<(Option<u64>, i64)>,
) -> rusqlite::Result<()> {
    let mut conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(Duration::from_millis(250))?;
    apply_batch_in_connection(source, &mut conn, counts, latest)
}

pub fn read_in_connection(
    source: Source,
    conn: &Connection,
) -> rusqlite::Result<BTreeMap<String, CountInfo>> {
    let mut statement = conn.prepare(&format!(
        "SELECT package, count, shared FROM {} WHERE count > 0 ORDER BY package LIMIT 16384",
        source.count_table()
    ))?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            CountInfo {
                count: row.get(1)?,
                shared: row.get(2)?,
            },
        ))
    })?;
    rows.collect()
}

pub fn read_at(source: Source, path: &Path) -> rusqlite::Result<Snapshot> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_millis(100))?;
    let latest: Option<(Option<i64>, i64)> = conn
        .query_row(
            &format!(
                "SELECT latency_ms, completed_at_ms FROM {} WHERE id=1",
                source.latest_table()
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    Ok(Snapshot {
        counts: read_in_connection(source, &conn).ok(),
        last_b_latency_ms: latest.and_then(|value| value.0),
        completed_at_ms: latest.map(|value| value.1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_increment_independently_and_survive_reopening_the_database() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        {
            let mut conn = Connection::open(temp.path()).unwrap();
            increment_in_connection(
                Source::Remote,
                &mut conn,
                &["com.example.one".into()],
                Some(100),
                1,
            )
            .unwrap();
            increment_in_connection(
                Source::Remote,
                &mut conn,
                &["com.example.two".into(), "com.example.three".into()],
                Some(200),
                2,
            )
            .unwrap();
            let latency: i64 = conn
                .query_row(
                    "SELECT latency_ms FROM ommega_attestation_latest WHERE id=1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(latency, 200);
            increment_in_connection(
                Source::Remote,
                &mut conn,
                &["com.example.one".into()],
                None,
                3,
            )
            .unwrap();
        }
        let conn = Connection::open(temp.path()).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT count FROM ommega_attestation_count WHERE package=?",
                ["com.example.one"],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        assert!(conn
            .query_row(
                "SELECT shared FROM ommega_attestation_count WHERE package=?",
                ["com.example.two"],
                |row| row.get::<_, bool>(0)
            )
            .unwrap());
        let counts = read_in_connection(Source::Remote, &conn).unwrap();
        assert_eq!(
            counts["com.example.one"],
            CountInfo {
                count: 2,
                shared: false
            }
        );
        assert_eq!(
            counts["com.example.two"],
            CountInfo {
                count: 1,
                shared: true
            }
        );
        let snapshot = read_at(Source::Remote, temp.path()).unwrap();
        assert_eq!(
            snapshot.last_b_latency_ms, None,
            "server-keybox success clears the previous B latency"
        );
        assert_eq!(snapshot.completed_at_ms, Some(3));
    }

    #[test]
    fn local_and_remote_counters_never_share_rows() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        {
            let mut conn = Connection::open(temp.path()).unwrap();
            increment_in_connection(
                Source::Remote,
                &mut conn,
                &["com.example.app".into()],
                Some(90),
                10,
            )
            .unwrap();
            increment_in_connection(
                Source::Local,
                &mut conn,
                &["com.example.app".into()],
                None,
                20,
            )
            .unwrap();
            increment_in_connection(
                Source::Local,
                &mut conn,
                &["com.example.app".into()],
                None,
                30,
            )
            .unwrap();
        }
        let conn = Connection::open(temp.path()).unwrap();
        let remote = read_in_connection(Source::Remote, &conn).unwrap();
        let local = read_in_connection(Source::Local, &conn).unwrap();
        assert_eq!(remote["com.example.app"].count, 1);
        assert_eq!(local["com.example.app"].count, 2);
        let remote_latest = read_at(Source::Remote, temp.path()).unwrap();
        let local_latest = read_at(Source::Local, temp.path()).unwrap();
        assert_eq!(
            remote_latest.last_b_latency_ms,
            Some(90),
            "the relay latency stays remote-only"
        );
        assert_eq!(remote_latest.completed_at_ms, Some(10));
        assert_eq!(local_latest.last_b_latency_ms, None);
        assert_eq!(local_latest.completed_at_ms, Some(30));
    }

    #[test]
    fn older_write_does_not_replace_the_latest_latency() {
        let mut conn = Connection::open_in_memory().unwrap();
        increment_in_connection(
            Source::Remote,
            &mut conn,
            &["com.example.app".into()],
            Some(100),
            20,
        )
        .unwrap();
        increment_in_connection(
            Source::Remote,
            &mut conn,
            &["com.example.app".into()],
            Some(500),
            10,
        )
        .unwrap();
        let latest: (i64, i64) = conn
            .query_row(
                "SELECT latency_ms, completed_at_ms FROM ommega_attestation_latest WHERE id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(latest, (100, 20));
        assert_eq!(
            read_in_connection(Source::Remote, &conn).unwrap()["com.example.app"].count,
            2
        );
    }
}
