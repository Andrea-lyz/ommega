use super::*;

fn node(ptr: libc::c_ulong) -> LocalBinderTarget {
    LocalBinderTarget {
        ptr,
        cookie: ptr + 0x1000,
    }
}

const APP_UID: i64 = 10_680;
const SYSTEM_UID: i64 = 1_000;

#[test]
fn service_node_rejects_a_maintenance_token_after_the_service_binding() {
    // The U probe: a maintenance interface token sent to the IKeystoreService node.
    let target = node(0x1000_0000);
    assert_eq!(
        observe_request(Some(target), KeystoreInterface::Service, APP_UID),
        BindingDecision::Proceed
    );
    assert_eq!(
        observe_request(Some(target), KeystoreInterface::Maintenance, APP_UID),
        BindingDecision::ForeignToken {
            bound: Some(KeystoreInterface::Service)
        }
    );
    // The rejected request must not disturb the binding.
    assert_eq!(
        observe_request(Some(target), KeystoreInterface::Service, APP_UID),
        BindingDecision::Proceed
    );
}

#[test]
fn maintenance_node_still_serves_the_maintenance_interface() {
    let target = node(0x2000_0000);
    assert_eq!(
        observe_request(Some(target), KeystoreInterface::Maintenance, SYSTEM_UID),
        BindingDecision::Proceed
    );
    assert_eq!(
        observe_request(Some(target), KeystoreInterface::Maintenance, SYSTEM_UID),
        BindingDecision::Proceed
    );
    assert_eq!(
        observe_request(Some(target), KeystoreInterface::Service, APP_UID),
        BindingDecision::ForeignToken {
            bound: Some(KeystoreInterface::Maintenance)
        }
    );
}

#[test]
fn application_uid_cannot_claim_a_system_interface_node() {
    let target = node(0x3000_0000);
    assert_eq!(
        observe_request(Some(target), KeystoreInterface::Maintenance, APP_UID),
        BindingDecision::ForeignToken { bound: None }
    );
    // The refused claim leaves the node free for its real interface.
    assert_eq!(
        observe_request(Some(target), KeystoreInterface::Service, APP_UID),
        BindingDecision::Proceed
    );
    assert_eq!(
        observe_request(Some(target), KeystoreInterface::Authorization, SYSTEM_UID),
        BindingDecision::ForeignToken {
            bound: Some(KeystoreInterface::Service)
        }
    );
}

#[test]
fn authorization_node_binds_for_system_callers() {
    let target = node(0x4000_0000);
    assert_eq!(
        observe_request(Some(target), KeystoreInterface::Authorization, SYSTEM_UID),
        BindingDecision::Proceed
    );
    assert_eq!(
        observe_request(Some(target), KeystoreInterface::Authorization, SYSTEM_UID),
        BindingDecision::Proceed
    );
}

#[test]
fn security_level_and_operation_nodes_accept_application_callers() {
    let security_level = node(0x5000_0000);
    let operation = node(0x6000_0000);
    assert_eq!(
        observe_request(
            Some(security_level),
            KeystoreInterface::SecurityLevel,
            APP_UID
        ),
        BindingDecision::Proceed
    );
    assert_eq!(
        observe_request(Some(operation), KeystoreInterface::Operation, APP_UID),
        BindingDecision::Proceed
    );
    assert_eq!(
        observe_request(Some(security_level), KeystoreInterface::Operation, APP_UID),
        BindingDecision::ForeignToken {
            bound: Some(KeystoreInterface::SecurityLevel)
        }
    );
}

#[test]
fn transactions_without_a_node_keep_the_previous_behavior() {
    assert_eq!(
        observe_request(None, KeystoreInterface::Maintenance, APP_UID),
        BindingDecision::Proceed
    );
}

#[test]
fn interface_classes_cover_the_known_keystore_tokens() {
    for interface in identify::KNOWN_KEYSTORE_INTERFACES {
        assert!(
            KeystoreInterface::from_token(interface).is_some(),
            "{interface} must have an interface class"
        );
    }
    assert_eq!(
        KeystoreInterface::from_token(identify::KEYSTORE_SERVICE_INTERFACE),
        Some(KeystoreInterface::Service)
    );
    assert_eq!(
        KeystoreInterface::from_token(identify::KEYSTORE_MAINTENANCE_INTERFACE),
        Some(KeystoreInterface::Maintenance)
    );
    assert_eq!(
        KeystoreInterface::from_token(identify::KEYSTORE_AUTHORIZATION_INTERFACE),
        Some(KeystoreInterface::Authorization)
    );
    assert_eq!(
        KeystoreInterface::from_token(identify::KEYSTORE_SECURITY_LEVEL_INTERFACE),
        Some(KeystoreInterface::SecurityLevel)
    );
    assert_eq!(
        KeystoreInterface::from_token(identify::KEYSTORE_OPERATION_INTERFACE),
        Some(KeystoreInterface::Operation)
    );
    assert_eq!(KeystoreInterface::from_token("android.window.IFoo"), None);
}
