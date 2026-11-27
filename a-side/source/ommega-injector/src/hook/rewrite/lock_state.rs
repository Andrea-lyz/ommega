use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};

use zeroize::Zeroizing;

use super::*;

pub(super) type RetainedLockState = BTreeMap<i32, RetainedUser>;

pub(super) static RETAINED_LOCK_STATE: LazyLock<Mutex<RetainedLockState>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

#[derive(Clone)]
pub(super) struct RetainedUser {
    pub unlock_secret: Option<(Zeroizing<Vec<u8>>, CallerInfo)>,
    pub phase: Phase,
}

#[derive(Clone)]
pub(super) enum Phase {
    Unknown,
    Unlocked {
        caller: CallerInfo,
    },
    Locked {
        unlocking_sids: Vec<i64>,
        weak_unlock_enabled: bool,
        weak_expired: bool,
        non_lskf_expired: bool,
        caller: CallerInfo,
    },
}

impl Default for RetainedUser {
    fn default() -> Self {
        Self {
            unlock_secret: None,
            phase: Phase::Unknown,
        }
    }
}

pub(super) fn retain_authorization(
    state: &mut RetainedLockState,
    request: &ParsedAuthorizationRequest,
    caller: &CallerInfo,
) {
    match request {
        ParsedAuthorizationRequest::OnDeviceUnlocked { user_id, password } => {
            let entry = state.entry(*user_id).or_default();
            if let Some(password) = password {
                entry.unlock_secret = Some((Zeroizing::new(password.clone()), caller.clone()));
            }
            entry.phase = Phase::Unlocked {
                caller: caller.clone(),
            };
        }
        ParsedAuthorizationRequest::OnDeviceLocked {
            user_id,
            unlocking_sids,
            weak_unlock_enabled,
        } => {
            state.entry(*user_id).or_default().phase = Phase::Locked {
                unlocking_sids: unlocking_sids.clone(),
                weak_unlock_enabled: *weak_unlock_enabled,
                weak_expired: false,
                non_lskf_expired: false,
                caller: caller.clone(),
            };
        }
        ParsedAuthorizationRequest::OnWeakUnlockMethodsExpired { user_id } => {
            if let Some(RetainedUser {
                phase:
                    Phase::Locked {
                        weak_expired: expired,
                        ..
                    },
                ..
            }) = state.get_mut(user_id)
            {
                *expired = true;
            }
        }
        ParsedAuthorizationRequest::OnNonLskfUnlockMethodsExpired { user_id } => {
            if let Some(RetainedUser {
                phase:
                    Phase::Locked {
                        non_lskf_expired: expired,
                        ..
                    },
                ..
            }) = state.get_mut(user_id)
            {
                *expired = true;
            }
        }
        ParsedAuthorizationRequest::OnUserStorageLocked { user_id } => {
            state.remove(user_id);
        }
        ParsedAuthorizationRequest::AddAuthToken { .. }
        | ParsedAuthorizationRequest::GetAuthTokensForCredStore { .. }
        | ParsedAuthorizationRequest::GetLastAuthTime { .. } => {}
    }
}

pub(super) fn retain_maintenance(
    state: &mut RetainedLockState,
    request: &ParsedMaintenanceRequest,
    caller: &CallerInfo,
) {
    match request {
        ParsedMaintenanceRequest::InitUserSuperKeys {
            user_id, password, ..
        } => {
            state.entry(*user_id).or_default().unlock_secret =
                Some((Zeroizing::new(password.clone()), caller.clone()));
        }
        ParsedMaintenanceRequest::OnUserPasswordChanged { user_id, password } => {
            state.entry(*user_id).or_default().unlock_secret = password
                .as_ref()
                .map(|password| (Zeroizing::new(password.clone()), caller.clone()));
        }
        ParsedMaintenanceRequest::OnUserAdded { user_id }
        | ParsedMaintenanceRequest::OnUserRemoved { user_id } => {
            state.remove(user_id);
        }
        ParsedMaintenanceRequest::DeleteAllKeys => state.clear(),
        _ => {}
    }
}

pub(super) fn replay_requests(
    state: &RetainedLockState,
) -> Vec<(CallerInfo, ParsedAuthorizationRequest)> {
    let mut replay = Vec::new();
    for (&user_id, retained) in state {
        let secret = retained
            .unlock_secret
            .as_ref()
            .map(|(secret, caller)| (secret.as_slice().to_vec(), caller.clone()));
        if let Some((secret, caller)) = &secret {
            replay.push((
                caller.clone(),
                ParsedAuthorizationRequest::OnDeviceUnlocked {
                    user_id,
                    password: Some(secret.clone()),
                },
            ));
        }
        match &retained.phase {
            Phase::Unlocked { caller } => {
                if secret.is_none() {
                    replay.push((
                        caller.clone(),
                        ParsedAuthorizationRequest::OnDeviceUnlocked {
                            user_id,
                            password: None,
                        },
                    ));
                }
            }
            Phase::Locked {
                unlocking_sids,
                weak_unlock_enabled,
                weak_expired,
                non_lskf_expired,
                caller,
            } => {
                replay.push((
                    caller.clone(),
                    ParsedAuthorizationRequest::OnDeviceLocked {
                        user_id,
                        unlocking_sids: unlocking_sids.clone(),
                        weak_unlock_enabled: *weak_unlock_enabled,
                    },
                ));
                if *weak_expired {
                    replay.push((
                        caller.clone(),
                        ParsedAuthorizationRequest::OnWeakUnlockMethodsExpired { user_id },
                    ));
                }
                if *non_lskf_expired {
                    replay.push((
                        caller.clone(),
                        ParsedAuthorizationRequest::OnNonLskfUnlockMethodsExpired { user_id },
                    ));
                }
            }
            Phase::Unknown => {
                if let Some((_, caller)) = secret {
                    replay.push((
                        caller,
                        ParsedAuthorizationRequest::OnDeviceLocked {
                            user_id,
                            unlocking_sids: Vec::new(),
                            weak_unlock_enabled: false,
                        },
                    ));
                }
            }
        }
    }
    replay
}

#[cfg(test)]
pub(super) fn reset_retained_lock_state_for_tests() {
    RETAINED_LOCK_STATE
        .lock()
        .expect("retained lock state poisoned")
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller(uid: i64) -> CallerInfo {
        CallerInfo {
            uid,
            sid: String::new(),
            pid: uid + 1,
        }
    }

    #[test]
    fn retained_unlock_lock_and_expiry_replay_in_order() {
        let mut state = RetainedLockState::new();
        let caller = caller(1000);
        retain_maintenance(
            &mut state,
            &ParsedMaintenanceRequest::InitUserSuperKeys {
                user_id: 10,
                password: vec![1, 2],
                allow_existing: false,
            },
            &caller,
        );
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnDeviceLocked {
                user_id: 10,
                unlocking_sids: vec![7],
                weak_unlock_enabled: true,
            },
            &caller,
        );
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnWeakUnlockMethodsExpired { user_id: 10 },
            &caller,
        );
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnNonLskfUnlockMethodsExpired { user_id: 10 },
            &caller,
        );
        let replay = replay_requests(&state);
        assert!(matches!(
            replay.as_slice(),
            [
                (
                    _,
                    ParsedAuthorizationRequest::OnDeviceUnlocked {
                        password: Some(_),
                        ..
                    }
                ),
                (_, ParsedAuthorizationRequest::OnDeviceLocked { .. }),
                (
                    _,
                    ParsedAuthorizationRequest::OnWeakUnlockMethodsExpired { .. }
                ),
                (
                    _,
                    ParsedAuthorizationRequest::OnNonLskfUnlockMethodsExpired { .. }
                )
            ]
        ));
    }

    #[test]
    fn unknown_user_with_secret_is_conservatively_locked_and_removal_drops_state() {
        let mut state = RetainedLockState::new();
        let caller = caller(1000);
        retain_maintenance(
            &mut state,
            &ParsedMaintenanceRequest::OnUserPasswordChanged {
                user_id: 11,
                password: Some(vec![3]),
            },
            &caller,
        );
        let replay = replay_requests(&state);
        assert!(matches!(
            replay.as_slice(),
            [
                (
                    _,
                    ParsedAuthorizationRequest::OnDeviceUnlocked {
                        password: Some(_),
                        ..
                    }
                ),
                (_, ParsedAuthorizationRequest::OnDeviceLocked { .. })
            ]
        ));
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnUserStorageLocked { user_id: 11 },
            &caller,
        );
        assert!(replay_requests(&state).is_empty());
    }

    #[test]
    fn authorization_retention_rules_track_unlock_lock_expiry_and_tokens() {
        let mut state = RetainedLockState::new();
        let caller = caller(1000);
        retain_maintenance(
            &mut state,
            &ParsedMaintenanceRequest::InitUserSuperKeys {
                user_id: 10,
                password: vec![9],
                allow_existing: false,
            },
            &caller,
        );
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnDeviceUnlocked {
                user_id: 10,
                password: None,
            },
            &caller,
        );
        assert!(state[&10].unlock_secret.is_some());
        assert!(matches!(state[&10].phase, Phase::Unlocked { .. }));
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnWeakUnlockMethodsExpired { user_id: 10 },
            &caller,
        );
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnNonLskfUnlockMethodsExpired { user_id: 10 },
            &caller,
        );
        assert!(matches!(state[&10].phase, Phase::Unlocked { .. }));

        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnDeviceLocked {
                user_id: 10,
                unlocking_sids: vec![1],
                weak_unlock_enabled: false,
            },
            &caller,
        );
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnWeakUnlockMethodsExpired { user_id: 10 },
            &caller,
        );
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnNonLskfUnlockMethodsExpired { user_id: 10 },
            &caller,
        );
        let Phase::Locked {
            weak_expired,
            non_lskf_expired,
            ..
        } = state[&10].phase
        else {
            panic!("user should be locked")
        };
        assert!(weak_expired && non_lskf_expired);
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnDeviceLocked {
                user_id: 10,
                unlocking_sids: vec![2],
                weak_unlock_enabled: true,
            },
            &caller,
        );
        let Phase::Locked {
            weak_expired,
            non_lskf_expired,
            ..
        } = state[&10].phase
        else {
            panic!("user should remain locked")
        };
        assert!(!weak_expired && !non_lskf_expired);

        let before = replay_requests(&state).len();
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::AddAuthToken {
                auth_token: Default::default(),
            },
            &caller,
        );
        assert_eq!(replay_requests(&state).len(), before);
    }

    #[test]
    fn maintenance_retention_rules_clear_or_remove_user_state() {
        let mut state = RetainedLockState::new();
        let caller = caller(1000);
        for user in [10, 11, 12] {
            retain_maintenance(
                &mut state,
                &ParsedMaintenanceRequest::OnUserPasswordChanged {
                    user_id: user,
                    password: Some(vec![user as u8]),
                },
                &caller,
            );
        }
        retain_maintenance(
            &mut state,
            &ParsedMaintenanceRequest::OnUserPasswordChanged {
                user_id: 10,
                password: None,
            },
            &caller,
        );
        assert!(state[&10].unlock_secret.is_none());
        retain_maintenance(
            &mut state,
            &ParsedMaintenanceRequest::OnUserAdded { user_id: 10 },
            &caller,
        );
        assert!(!state.contains_key(&10));
        retain_maintenance(
            &mut state,
            &ParsedMaintenanceRequest::OnUserRemoved { user_id: 11 },
            &caller,
        );
        assert!(!state.contains_key(&11));
        retain_maintenance(
            &mut state,
            &ParsedMaintenanceRequest::DeleteAllKeys,
            &caller,
        );
        assert!(state.is_empty());
    }

    #[test]
    fn replay_requests_cover_unlocked_users_order_and_locked_values() {
        let mut state = RetainedLockState::new();
        let caller = caller(1000);
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnDeviceUnlocked {
                user_id: 20,
                password: None,
            },
            &caller,
        );
        let replay = replay_requests(&state);
        assert!(matches!(
            replay.as_slice(),
            [(
                _,
                ParsedAuthorizationRequest::OnDeviceUnlocked {
                    user_id: 20,
                    password: None
                }
            )]
        ));

        retain_maintenance(
            &mut state,
            &ParsedMaintenanceRequest::OnUserPasswordChanged {
                user_id: 20,
                password: Some(vec![4, 5]),
            },
            &caller,
        );
        let replay = replay_requests(&state);
        let [(
            _,
            ParsedAuthorizationRequest::OnDeviceUnlocked {
                user_id: 20,
                password: Some(secret),
            },
        )] = replay.as_slice()
        else {
            panic!("unlocked user with secret should replay only the secret unlock")
        };
        assert_eq!(secret, &[4, 5]);

        state.clear();
        for user in [30, 10] {
            retain_authorization(
                &mut state,
                &ParsedAuthorizationRequest::OnDeviceUnlocked {
                    user_id: user,
                    password: None,
                },
                &caller,
            );
        }
        let replay = replay_requests(&state);
        let users: Vec<i32> = replay
            .iter()
            .map(|(_, request)| match request {
                ParsedAuthorizationRequest::OnDeviceUnlocked { user_id, .. } => *user_id,
                _ => panic!("unexpected replay request"),
            })
            .collect();
        assert_eq!(users, [10, 30]);

        state.clear();
        retain_authorization(
            &mut state,
            &ParsedAuthorizationRequest::OnDeviceLocked {
                user_id: 40,
                unlocking_sids: vec![11, 12],
                weak_unlock_enabled: true,
            },
            &caller,
        );
        let replay = replay_requests(&state);
        let [(
            _,
            ParsedAuthorizationRequest::OnDeviceLocked {
                unlocking_sids,
                weak_unlock_enabled,
                ..
            },
        )] = replay.as_slice()
        else {
            panic!("locked user should replay one lock event")
        };
        assert_eq!(unlocking_sids, &[11, 12]);
        assert!(*weak_unlock_enabled);
    }
}
