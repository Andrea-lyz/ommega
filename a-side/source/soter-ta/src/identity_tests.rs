//! Regression coverage for identity selection, rotation and full relay sessions.
use crate::{
    blob,
    dispatch::{self as d, Outcome, Request},
    error::*,
    ffi,
    identities::{self, Record, Selection, Snapshot, Store},
    platform::Platform,
    remote,
    state::{KeyPair, TaState},
};
use std::{
    cell::Cell,
    ffi::CString,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
};
const UID: u32 = 11001;
fn base() -> TaState {
    static BASE: OnceLock<Vec<u8>> = OnceLock::new();
    TaState::from_json(BASE.get_or_init(|| TaState::generate_local().unwrap().to_json())).unwrap()
}
fn local(id: &str) -> Record {
    Record {
        id: id.into(),
        name: id.into(),
        kind: "local".into(),
        ..Record::default()
    }
}
fn relay(id: &str, url: &str) -> Record {
    Record {
        id: id.into(),
        name: id.into(),
        kind: "relay".into(),
        url: url.into(),
        token: "mock-token".into(),
        device_id: "mock-device".into(),
        ..Record::default()
    }
}
fn policy(r: Record) -> Snapshot {
    Snapshot::memory(Store {
        default: r.id.clone(),
        identities: vec![r],
        ..Store::default()
    })
}
fn config() -> remote::Config {
    remote::Config {
        enabled: true,
        url: "https://global.invalid".into(),
        token: "mock-token".into(),
        device_id: "mock-device".into(),
        ..remote::Config::default()
    }
}
fn fixture() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "soter-regression-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn init(p: &std::path::Path) {
    let path = p.join("state.json");
    base().store(&path).unwrap();
    let arg = CString::new(path.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { ffi::soterta_init(arg.as_ptr()) }, 0);
}
fn session(o: Option<Outcome>) -> u64 {
    let Some(Outcome::Init { code: 0, session }) = o else {
        panic!("expected init")
    };
    session
}
fn seed(state: &mut TaState, pair: &KeyPair) {
    let entry = state.uids.entry(UID).or_default();
    entry.ask = Some(pair.clone());
    entry.auth.insert("auth".into(), pair.clone());
}
#[test]
fn named_relay_finishes_at_the_original_server_with_global_on_or_off() {
    let _guard = ffi::TEST_LOCK.lock().unwrap();
    let dir = fixture();
    for global in [false, true] {
        init(&dir);
        let policy = policy(relay("a", "https://identity-a.invalid"));
        let h = session(
            ffi::answer_with_policy(
                d::TX_INIT_SIGN,
                Request::InitSign {
                    uid: UID,
                    kname: "auth".into(),
                    challenge: "challenge".into(),
                },
                Ok(if global {
                    config()
                } else {
                    remote::Config::default()
                }),
                global,
                &policy,
                "",
                |c, _, _, _| {
                    assert_eq!(c.url, "https://identity-a.invalid");
                    Ok(Some(Outcome::Init {
                        code: 0,
                        session: 7,
                    }))
                },
            )
            .unwrap(),
        );
        let out = ffi::answer_with_policy(
            d::TX_FINISH_SIGN,
            Request::Session(h),
            Ok(if global {
                config()
            } else {
                remote::Config::default()
            }),
            global,
            &policy,
            "",
            |c, _, r, _| {
                assert_eq!(c.url, "https://identity-a.invalid");
                assert_eq!(r, &Request::Session(7));
                Ok(Some(Outcome::Buffer {
                    code: 0,
                    data: vec![1],
                    field: 1,
                }))
            },
        )
        .unwrap();
        assert!(matches!(out, Some(Outcome::Buffer { code: 0, .. })));
    }
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn local_identity_overrides_global_relay_and_changed_session_never_forwards() {
    let _guard = ffi::TEST_LOCK.lock().unwrap();
    let dir = fixture();
    init(&dir);
    let out = ffi::answer_with_policy(
        d::TX_HAS_ASK,
        Request::Uid(UID),
        Ok(config()),
        true,
        &policy(local("a")),
        "",
        |_, _, _, _| panic!("local identity was sent to relay"),
    )
    .unwrap();
    assert_eq!(out, Some(Outcome::Code(SOTER_ERR_NO_KEY)));
    let a = policy(relay("r", "https://identity-a.invalid"));
    let h = session(
        ffi::answer_with_policy(
            d::TX_INIT_SIGN,
            Request::InitSign {
                uid: UID,
                kname: "auth".into(),
                challenge: "challenge".into(),
            },
            Ok(config()),
            true,
            &a,
            "",
            |_, _, _, _| {
                Ok(Some(Outcome::Init {
                    code: 0,
                    session: 7,
                }))
            },
        )
        .unwrap(),
    );
    let b = policy(relay("r", "https://identity-b.invalid"));
    let called = Cell::new(false);
    assert!(ffi::answer_with_policy(
        d::TX_FINISH_SIGN,
        Request::Session(h),
        Ok(config()),
        true,
        &b,
        "",
        |_, _, _, _| {
            called.set(true);
            Ok(None)
        }
    )
    .is_err());
    assert!(!called.get());
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn named_relay_never_silently_becomes_a_local_identity() {
    let _guard = ffi::TEST_LOCK.lock().unwrap();
    let dir = fixture();
    init(&dir);
    let p = policy(relay("a", "https://identity-a.invalid"));
    assert!(ffi::answer_with_policy(
        d::TX_HAS_ASK,
        Request::Uid(UID),
        Ok(remote::Config::default()),
        false,
        &p,
        "",
        |_, _, _, _| Ok(None)
    )
    .is_err());
    let called = Cell::new(false);
    assert_eq!(
        ffi::answer_with_policy(
            d::TX_HAS_ASK,
            Request::Uid(UID),
            Ok(remote::Config::default()),
            false,
            &p,
            "",
            |_, _, _, _| {
                called.set(true);
                Ok(Some(Outcome::Code(0)))
            }
        )
        .unwrap(),
        Some(Outcome::Code(0))
    );
    assert!(called.get());
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn old_session_is_revoked_and_counters_survive_switching_back() {
    let mut s = base();
    let pair = s.attk.clone();
    let a = Selection::Key {
        id: "a".into(),
        cpu_id: "a".repeat(32),
        pair: pair.clone(),
    };
    let b = Selection::Key {
        id: "b".into(),
        cpu_id: "b".repeat(32),
        pair: pair.clone(),
    };
    s.prepare_identity(Some(UID), &a).unwrap();
    seed(&mut s, &pair);
    let (_, ask) = s.export_ask(UID);
    let (j, _) = blob::parse(&ask).unwrap();
    let first = serde_json::from_slice::<serde_json::Value>(j).unwrap()["counter"]
        .as_u64()
        .unwrap();
    assert!(
        first >= 1_700_000_000,
        "the counter must start above any value this device already served, got {first}"
    );
    let (_, handle) = s.init_sign(
        UID,
        "auth",
        "old challenge",
        Platform {
            boot_ms: Some(1000),
            bio_mark: Some(1u64 << 32),
        },
    );
    s.prepare_identity(Some(UID), &b).unwrap();
    seed(&mut s, &pair);
    assert_eq!(
        s.finish_sign(
            handle,
            Platform {
                boot_ms: Some(1100),
                bio_mark: Some((1u64 << 32) + 1)
            }
        )
        .0,
        SOTER_ERR_NO_SESSION
    );
    s.prepare_identity(Some(UID), &a).unwrap();
    seed(&mut s, &pair);
    let (_, ask) = s.export_ask(UID);
    let (j, _) = blob::parse(&ask).unwrap();
    let second = serde_json::from_slice::<serde_json::Value>(j).unwrap()["counter"]
        .as_u64()
        .unwrap();
    assert!(
        second > first,
        "switching back must not rewind the counter: {second} <= {first}"
    );
}
#[test]
fn material_rotation_same_id_invalidates_ask_auth_and_sessions() {
    let mut s = base();
    let pair = s.attk.clone();
    let a = Selection::Key {
        id: "same".into(),
        cpu_id: "a".repeat(32),
        pair: pair.clone(),
    };
    s.prepare_identity(Some(UID), &a).unwrap();
    seed(&mut s, &pair);
    let (_, h) = s.init_sign(UID, "auth", "challenge", Platform::default());
    let b = Selection::Key {
        id: "same".into(),
        cpu_id: "b".repeat(32),
        pair,
    };
    s.prepare_identity(Some(UID), &b).unwrap();
    assert_eq!(s.has_ask(UID), SOTER_ERR_NO_KEY);
    assert_eq!(s.has_auth(UID, "auth"), SOTER_ERR_NO_AUTH_KEY);
    assert!(!s.sessions.contains_key(&h));
}
#[test]
fn bad_configuration_does_not_mutate_valid_material() {
    let _guard = ffi::TEST_LOCK.lock().unwrap();
    let dir = fixture();
    let mut state = base();
    let pair = state.attk.clone();
    seed(&mut state, &pair);
    let path = dir.join("state.json");
    state.store(&path).unwrap();
    let raw = state.to_json();
    let arg = CString::new(path.to_str().unwrap()).unwrap();
    unsafe {
        ffi::soterta_init(arg.as_ptr());
    }
    let p = policy(Record {
        id: "bad".into(),
        name: "incomplete".into(),
        kind: "key".into(),
        cpu_id: "b".repeat(32),
        ..Record::default()
    });
    assert!(ffi::answer_with_policy(
        d::TX_HAS_ASK,
        Request::Uid(UID),
        Ok(config()),
        true,
        &p,
        "",
        |_, _, _, _| panic!("must not call network")
    )
    .is_err());
    assert_eq!(fs::read(&path).unwrap(), raw);
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn missing_policy_after_configuration_was_seen_fails_closed() {
    let _guard = ffi::TEST_LOCK.lock().unwrap();
    let dir = fixture();
    init(&dir);
    let p = Snapshot::memory(Store::default());
    ffi::answer_with_policy(
        d::TX_HAS_ASK,
        Request::Uid(UID),
        Ok(remote::Config::default()),
        false,
        &p,
        "",
        |_, _, _, _| panic!(),
    )
    .unwrap();
    assert!(ffi::answer_with_policy(
        d::TX_HAS_ASK,
        Request::Uid(UID),
        Ok(remote::Config::default()),
        false,
        &Snapshot::absent(),
        "",
        |_, _, _, _| panic!()
    )
    .is_err());
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn metadata_uses_selected_default_and_key_validation_requires_private_components() {
    let mut s = base();
    let pair = s.attk.clone();
    let selected = Selection::Key {
        id: "a".into(),
        cpu_id: "a".repeat(32),
        pair: pair.clone(),
    };
    s.prepare_identity(None, &selected).unwrap();
    assert_eq!(hex::encode(s.device_id().1), "a".repeat(32));
    let (_, attk) = s.export_attk();
    let (j, sig) = blob::parse(&attk).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(j).unwrap()["cpu_id"],
        "a".repeat(32)
    );
    assert!(blob::verify(&pair.public().unwrap(), j, sig));
    assert!(identities::validate_key(&pair.public_pem).is_err());
    assert!(identities::validate_key(&pair.private_pem).is_ok());
}

#[test]
fn policy_appearance_migration_and_inflight_edits_reject_stale_results() {
    let _guard = ffi::TEST_LOCK.lock().unwrap();
    let dir = fixture();
    let primary = dir.join("policy.json");
    let legacy = dir.join("legacy.json");
    let absent = Snapshot::load_at(&primary, &legacy).unwrap();
    let store = Store {
        default: "r".into(),
        identities: vec![relay("r", "https://identity-a.invalid")],
        ..Store::default()
    };
    fs::write(&legacy, serde_json::to_vec(&store).unwrap()).unwrap();
    assert!(absent.unchanged().is_err());
    let old = Snapshot::load_at(&primary, &legacy).unwrap();
    fs::write(&primary, serde_json::to_vec(&store).unwrap()).unwrap();
    assert!(old.unchanged().is_err());
    let live = Snapshot::load_at(&primary, &legacy).unwrap();
    init(&dir);
    let result = ffi::answer_with_policy(
        d::TX_INIT_SIGN,
        Request::InitSign {
            uid: UID,
            kname: "auth".into(),
            challenge: "offline".into(),
        },
        Ok(remote::Config::default()),
        false,
        &live,
        "",
        |_, _, _, _| {
            fs::write(&primary, b"{}").unwrap();
            Ok(Some(Outcome::Init {
                code: 0,
                session: 7,
            }))
        },
    );
    assert!(result.is_err());
    assert!(Snapshot::load_at(&primary, &legacy).is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn uid_less_metadata_follows_the_default_identity_and_never_relays() {
    let _guard = ffi::TEST_LOCK.lock().unwrap();
    let dir = fixture();
    init(&dir);
    // A device identity that is not the ledger one: the default identity is
    // what every application without its own assignment uses.
    let policy = Snapshot::memory(Store {
        default: "local-a".into(),
        identities: vec![local("local-a")],
        ..Store::default()
    });
    let called = Cell::new(false);
    let out = ffi::answer_with_policy(
        d::TX_GET_DEVICE_ID,
        Request::None,
        Ok(config()),
        true,
        &policy,
        "",
        |_, _, _, _| {
            called.set(true);
            Ok(None)
        },
    )
    .unwrap();
    assert!(!called.get(), "metadata must not be relayed");
    let Some(Outcome::Buffer { code: 0, data, .. }) = out else {
        panic!("expected a device id");
    };
    let served = hex::encode(&data);
    assert_ne!(
        served,
        base().cpu_id,
        "the default identity owns its own id"
    );
    assert!(served.starts_with("00000000"), "unexpected id {served}");
    // The identity material the answer came from is now in the ledger.
    let ledger = TaState::load(&dir.join("state.json")).unwrap();
    assert_eq!(
        ledger.identities["local-a"].cpu_id, served,
        "getDeviceId must report the default identity"
    );
    fs::remove_dir_all(dir).unwrap();
}
