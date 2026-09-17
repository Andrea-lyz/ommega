//! Binder-node / interface-token binding for the real keystore2 nodes.
//!
//! keystore2 publishes the service, authorization, and maintenance interfaces as
//! separate binder nodes (`IKeystoreService/default`,
//! `...IKeystoreAuthorization/default`, `...IKeystoreMaintenance/default`), and
//! every AOSP AIDL stub enforces its own interface token in `onTransact` before
//! dispatching. A transaction that carries a foreign token on a node therefore
//! fails inside the stub and never runs.
//!
//! The injector classifies inbound transactions by the interface token alone, so
//! a caller could otherwise have a maintenance- or authorization-classified
//! transaction executed on the service node. Remember the interface each node
//! has been observed to serve, and leave transactions to the system whenever the
//! token contradicts that binding: the node then keeps answering exactly what the
//! real stub would.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use crate::hook::binder::LocalBinderTarget;
use crate::identify;

/// `Process.FIRST_APPLICATION_UID`: every uid below it belongs to the system.
const FIRST_APPLICATION_UID: i64 = 10_000;

/// keystore2 interfaces the injector recognizes in inbound parcels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeystoreInterface {
    Service,
    Maintenance,
    Authorization,
    SecurityLevel,
    Operation,
}

impl KeystoreInterface {
    pub(super) fn from_token(token: &str) -> Option<Self> {
        match token {
            identify::KEYSTORE_SERVICE_INTERFACE => Some(Self::Service),
            identify::KEYSTORE_MAINTENANCE_INTERFACE => Some(Self::Maintenance),
            identify::KEYSTORE_AUTHORIZATION_INTERFACE => Some(Self::Authorization),
            identify::KEYSTORE_SECURITY_LEVEL_INTERFACE => Some(Self::SecurityLevel),
            identify::KEYSTORE_OPERATION_INTERFACE => Some(Self::Operation),
            _ => None,
        }
    }

    /// Interfaces an application process can legitimately address: the service
    /// node is published through `ServiceManager`, and the security-level and
    /// operation objects are handed out by it. The authorization and maintenance
    /// nodes are only reachable for the system components that hold them.
    fn app_reachable(self) -> bool {
        matches!(self, Self::Service | Self::SecurityLevel | Self::Operation)
    }
}

/// Outcome of matching a request's interface token against its binder node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BindingDecision {
    /// The token matches the interface this node serves, or establishes it.
    Proceed,
    /// The token contradicts the node: leave the transaction to the system.
    ForeignToken {
        /// Interface the node is known to serve, when it was already bound.
        bound: Option<KeystoreInterface>,
    },
}

static NODE_INTERFACES: LazyLock<Mutex<HashMap<LocalBinderTarget, KeystoreInterface>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Match a keystore request against the interface its binder node serves.
///
/// The first token seen on a node binds it, except that a system-interface token
/// (maintenance, authorization) from an application uid neither binds nor runs:
/// an application can never legitimately reach those nodes, so such a request is
/// left to the real stub, which rejects it before dispatch.
pub(super) fn observe_request(
    target: Option<LocalBinderTarget>,
    interface: KeystoreInterface,
    caller_uid: i64,
) -> BindingDecision {
    let Some(target) = target else {
        return BindingDecision::Proceed;
    };
    let mut nodes = NODE_INTERFACES.lock().expect("node interface map poisoned");
    match nodes.get(&target) {
        Some(bound) if *bound == interface => BindingDecision::Proceed,
        Some(bound) => BindingDecision::ForeignToken {
            bound: Some(*bound),
        },
        None if interface.app_reachable() || caller_uid < FIRST_APPLICATION_UID => {
            nodes.insert(target, interface);
            BindingDecision::Proceed
        }
        None => BindingDecision::ForeignToken { bound: None },
    }
}

#[cfg(test)]
mod tests;
