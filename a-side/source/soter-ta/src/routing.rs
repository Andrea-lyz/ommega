//! Keep an application's ASK, AuthKey and sign sessions on one backend.

use std::collections::BTreeMap;

use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::dispatch::{self, Outcome, Request};
use crate::remote::request_uid;

const MAX_SESSIONS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Backend {
    Local,
    Remote,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Session {
    uid: u32,
    backend: Backend,
    handle: u64,
}

/// Stored with the existing ledger; no separate policy or credential file.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Routing {
    scope: String,
    /// A local-only verdict applies to the UID, not just the current operation.
    /// The bool records whether this client has received the local ASK.
    local_uids: BTreeMap<u32, bool>,
    sessions: BTreeMap<u64, Session>,
    next_session: u64,
}

#[derive(Debug)]
pub(crate) struct Route {
    pub backend: Backend,
    pub request: Request,
    pub uid: Option<u32>,
    session: Option<u64>,
}

impl Routing {
    pub fn configure(&mut self, scope: &str) {
        if self.scope != scope {
            self.scope = scope.to_owned();
            self.local_uids.clear();
            self.sessions.clear();
            // Never reuse a handle still held by a client of the old scope.
        }
    }

    pub fn is_scope(&self, scope: &str) -> bool {
        self.scope == scope
    }

    pub fn remote_was_active(&self) -> bool {
        !self.scope.is_empty() && self.scope != "local"
    }

    pub fn prepare(&self, relay: bool, tx: u32, request: Request) -> Result<Route, String> {
        if let Request::Session(handle) = request {
            let session = self
                .sessions
                .get(&handle)
                .ok_or("unknown SOTER sign session")?;
            if session.backend == Backend::Remote
                && (!relay || self.local_uids.contains_key(&session.uid))
            {
                return Err("SOTER sign session belongs to a different backend".into());
            }
            return Ok(Route {
                backend: session.backend,
                request: Request::Session(session.handle),
                uid: Some(session.uid),
                session: Some(handle),
            });
        }
        let uid = request_uid(&request);
        let local = !relay || uid.is_some_and(|uid| self.local_uids.contains_key(&uid));
        if relay && local {
            self.check_local_ask(tx, uid)?;
        }
        Ok(Route {
            backend: if local {
                Backend::Local
            } else {
                Backend::Remote
            },
            request,
            uid,
            session: None,
        })
    }

    fn check_local_ask(&self, tx: u32, uid: Option<u32>) -> Result<(), String> {
        if matches!(
            tx,
            dispatch::TX_HAS_AUTH
                | dispatch::TX_GENERATE_AUTH
                | dispatch::TX_EXPORT_AUTH
                | dispatch::TX_INIT_SIGN
        ) && uid.is_some_and(|uid| self.local_uids.get(&uid) == Some(&false))
        {
            return Err("SOTER local-only policy requires exporting the local ASK first".into());
        }
        Ok(())
    }

    pub fn local_only(&mut self, tx: u32, route: &mut Route) -> Result<(), String> {
        let uid = route
            .uid
            .ok_or("SOTER local-only reply has no caller UID")?;
        self.local_uids.entry(uid).or_insert(false);
        // A session belongs to the backend that opened it. Never reinterpret
        // a remote session number as a local ledger session.
        if route.session.is_some() {
            return Err("relay declined an existing SOTER sign session".into());
        }
        route.backend = Backend::Local;
        self.check_local_ask(tx, Some(uid))
    }

    pub fn complete(
        &mut self,
        tx: u32,
        route: &Route,
        outcome: Outcome,
    ) -> Result<Outcome, String> {
        if route.backend == Backend::Remote
            && route
                .uid
                .is_some_and(|uid| self.local_uids.contains_key(&uid))
        {
            return Err(
                "SOTER caller switched backend while the relay request was in flight".into(),
            );
        }
        let code = match &outcome {
            Outcome::Code(code) | Outcome::Buffer { code, .. } | Outcome::Init { code, .. } => {
                *code
            }
            Outcome::Exception { .. } => return Ok(outcome),
        };
        if let Some(handle) = route.session {
            // The local TA consumes its session even on authentication failure.
            // A remote vendor may allow another authentication attempt.
            if route.backend == Backend::Local || !matches!(code, -26 | -9) {
                self.sessions.remove(&handle);
            }
        }
        if code != 0 {
            return Ok(outcome);
        }
        if tx == dispatch::TX_EXPORT_ASK && route.backend == Backend::Local {
            if let Some(ready) = route.uid.and_then(|uid| self.local_uids.get_mut(&uid)) {
                *ready = true;
            }
        }
        if tx == dispatch::TX_REMOVE_ALL_UID_KEY {
            if let Some(uid) = route.uid {
                self.sessions.retain(|_, session| session.uid != uid);
                if let Some(ready) = self.local_uids.get_mut(&uid) {
                    *ready = false;
                }
            }
        }
        if let Outcome::Init { session, .. } = outcome {
            if self.next_session == 0 {
                // Keep old raw HAL handles from aliasing the first handles
                // issued after upgrading a ledger without routing metadata.
                self.next_session = (OsRng.next_u64() & ((1 << 62) - 1)) | (1 << 62);
            }
            self.next_session = self
                .next_session
                .checked_add(1)
                .filter(|handle| *handle <= i64::MAX as u64)
                .ok_or("SOTER session handle space exhausted")?;
            if self.sessions.len() >= MAX_SESSIONS {
                self.sessions.pop_first();
            }
            self.sessions.insert(
                self.next_session,
                Session {
                    uid: route.uid.ok_or("SOTER initSign has no UID")?,
                    backend: route.backend,
                    handle: session,
                },
            );
            return Ok(Outcome::Init {
                code,
                session: self.next_session,
            });
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dispatch::*;

    fn ask() -> Outcome {
        Outcome::Buffer {
            code: 0,
            data: b"ASK".to_vec(),
            field: 3,
        }
    }

    fn auth(uid: u32) -> Request {
        Request::UidKey {
            uid,
            kname: "payment".into(),
        }
    }

    fn init(uid: u32) -> Request {
        Request::InitSign {
            uid,
            kname: "payment".into(),
            challenge: "challenge".into(),
        }
    }

    fn session(outcome: Outcome) -> u64 {
        let Outcome::Init { code: 0, session } = outcome else {
            panic!("init failed")
        };
        session
    }

    #[test]
    fn remote_ask_then_local_auth_requires_a_local_ask_and_survives_reload() {
        let mut routes = Routing::default();
        routes.configure("server-one");
        let remote_ask = routes
            .prepare(true, TX_EXPORT_ASK, Request::Uid(42))
            .unwrap();
        routes.complete(TX_EXPORT_ASK, &remote_ask, ask()).unwrap();
        let mut auth = routes.prepare(true, TX_GENERATE_AUTH, auth(42)).unwrap();
        assert!(routes.local_only(TX_GENERATE_AUTH, &mut auth).is_err());
        let json = serde_json::to_string(&routes).unwrap();
        let mut routes: Routing = serde_json::from_str(&json).unwrap();
        assert!(routes.remote_was_active());
        routes.configure("server-one");
        assert!(routes
            .prepare(true, TX_GENERATE_AUTH, auth.request.clone())
            .is_err());
        assert_eq!(
            routes
                .prepare(true, TX_HAS_ASK, Request::Uid(42))
                .unwrap()
                .backend,
            Backend::Local
        );
        let local_ask = routes
            .prepare(true, TX_EXPORT_ASK, Request::Uid(42))
            .unwrap();
        assert_eq!(local_ask.backend, Backend::Local);
        routes.complete(TX_EXPORT_ASK, &local_ask, ask()).unwrap();
        let local_auth = routes
            .prepare(true, TX_GENERATE_AUTH, auth.request)
            .unwrap();
        assert_eq!(local_auth.backend, Backend::Local);
        assert_eq!(
            routes
                .prepare(true, TX_EXPORT_ASK, Request::Uid(43))
                .unwrap()
                .backend,
            Backend::Remote
        );
    }

    #[test]
    fn failed_ask_export_does_not_allow_authentication_keys() {
        let mut routes = Routing::default();
        let mut route = routes
            .prepare(true, TX_EXPORT_ASK, Request::Uid(42))
            .unwrap();
        routes.local_only(TX_EXPORT_ASK, &mut route).unwrap();
        routes
            .complete(
                TX_EXPORT_ASK,
                &route,
                Outcome::Buffer {
                    code: -5,
                    data: vec![],
                    field: 0,
                },
            )
            .unwrap();
        assert!(routes.prepare(true, TX_EXPORT_AUTH, auth(42)).is_err());
    }

    #[test]
    fn equal_vendor_session_numbers_never_cross_backends() {
        let mut routes = Routing::default();
        let remote = routes.prepare(true, TX_INIT_SIGN, init(42)).unwrap();
        let remote_handle = session(
            routes
                .complete(
                    TX_INIT_SIGN,
                    &remote,
                    Outcome::Init {
                        code: 0,
                        session: 7,
                    },
                )
                .unwrap(),
        );
        let mut local = routes
            .prepare(true, TX_EXPORT_ASK, Request::Uid(43))
            .unwrap();
        routes.local_only(TX_EXPORT_ASK, &mut local).unwrap();
        routes.complete(TX_EXPORT_ASK, &local, ask()).unwrap();
        let local = routes.prepare(true, TX_INIT_SIGN, init(43)).unwrap();
        let local_handle = session(
            routes
                .complete(
                    TX_INIT_SIGN,
                    &local,
                    Outcome::Init {
                        code: 0,
                        session: 7,
                    },
                )
                .unwrap(),
        );
        assert_ne!(remote_handle, local_handle);
        let routes: Routing =
            serde_json::from_str(&serde_json::to_string(&routes).unwrap()).unwrap();
        let remote = routes
            .prepare(true, TX_FINISH_SIGN, Request::Session(remote_handle))
            .unwrap();
        let local = routes
            .prepare(true, TX_FINISH_SIGN, Request::Session(local_handle))
            .unwrap();
        assert_eq!(remote.backend, Backend::Remote);
        assert_eq!(local.backend, Backend::Local);
        assert_eq!(remote.request, Request::Session(7));
        assert_eq!(local.request, Request::Session(7));
    }

    #[test]
    fn local_only_for_remote_finish_never_dispatches_to_the_local_ta() {
        let mut routes = Routing::default();
        let route = routes.prepare(true, TX_INIT_SIGN, init(42)).unwrap();
        let handle = session(
            routes
                .complete(
                    TX_INIT_SIGN,
                    &route,
                    Outcome::Init {
                        code: 0,
                        session: 7,
                    },
                )
                .unwrap(),
        );
        let mut finish = routes
            .prepare(true, TX_FINISH_SIGN, Request::Session(handle))
            .unwrap();
        assert!(routes.local_only(TX_FINISH_SIGN, &mut finish).is_err());
        assert!(routes
            .prepare(true, TX_FINISH_SIGN, Request::Session(handle))
            .is_err());
        assert!(routes
            .prepare(true, TX_FINISH_SIGN, Request::Session(12345))
            .is_err());
    }

    #[test]
    fn config_changes_invalidate_sessions_without_reusing_handles() {
        let mut routes = Routing::default();
        routes.configure("server-one");
        let route = routes.prepare(true, TX_INIT_SIGN, init(42)).unwrap();
        let first = session(
            routes
                .complete(
                    TX_INIT_SIGN,
                    &route,
                    Outcome::Init {
                        code: 0,
                        session: 7,
                    },
                )
                .unwrap(),
        );
        routes.configure("server-two");
        assert!(routes
            .prepare(true, TX_FINISH_SIGN, Request::Session(first))
            .is_err());
        let route = routes.prepare(true, TX_INIT_SIGN, init(42)).unwrap();
        let second = session(
            routes
                .complete(
                    TX_INIT_SIGN,
                    &route,
                    Outcome::Init {
                        code: 0,
                        session: 7,
                    },
                )
                .unwrap(),
        );
        assert_ne!(first, second);
    }

    #[test]
    fn remote_minus_26_is_preserved_without_a_local_fallback() {
        let mut routes = Routing::default();
        let route = routes.prepare(true, TX_INIT_SIGN, init(42)).unwrap();
        let handle = session(
            routes
                .complete(
                    TX_INIT_SIGN,
                    &route,
                    Outcome::Init {
                        code: 0,
                        session: 7,
                    },
                )
                .unwrap(),
        );
        let route = routes
            .prepare(true, TX_FINISH_SIGN, Request::Session(handle))
            .unwrap();
        let failed = Outcome::Buffer {
            code: -26,
            data: vec![],
            field: 0,
        };
        assert_eq!(
            routes
                .complete(TX_FINISH_SIGN, &route, failed.clone())
                .unwrap(),
            failed
        );
        assert_eq!(
            routes
                .prepare(true, TX_FINISH_SIGN, Request::Session(handle))
                .unwrap()
                .backend,
            Backend::Remote
        );
    }

    #[test]
    fn late_remote_replies_cannot_publish_material_after_a_local_verdict() {
        let mut routes = Routing::default();
        let remote_ask = routes
            .prepare(true, TX_EXPORT_ASK, Request::Uid(42))
            .unwrap();
        let mut auth = routes.prepare(true, TX_GENERATE_AUTH, auth(42)).unwrap();
        assert!(routes.local_only(TX_GENERATE_AUTH, &mut auth).is_err());
        assert!(routes.complete(TX_EXPORT_ASK, &remote_ask, ask()).is_err());
    }

    #[test]
    fn anonymous_local_only_cannot_change_the_device_identity() {
        let mut routes = Routing::default();
        let mut route = routes
            .prepare(true, TX_GET_DEVICE_ID, Request::None)
            .unwrap();
        assert!(routes.local_only(TX_GET_DEVICE_ID, &mut route).is_err());
    }
}
