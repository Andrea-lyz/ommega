//! Which transaction codes the platform's own keystore2 stub implements.
//!
//! The injector hands out its own IKeystoreSecurityLevel and IKeystoreOperation
//! objects. Those objects are generated from the AIDL this module was built
//! against, so on an older platform they would be asked for transaction codes
//! whose own stub does not implement them at all - a stock AIDL stub fails such
//! a transaction at the transport level, before any code runs.
//!
//! Version tables for the service interface live in crate::identify. This module
//! answers the same question for the interfaces that have no such table, by
//! asking the local keystore2 stub which codes it implements. Only the
//! in-process object is authoritative: a remote proxy reports transport failures
//! that must not be read as "not implemented", so a probe that cannot reach the
//! local object leaves every code implemented, which is the behavior of a build
//! without this gate.

use std::sync::{LazyLock, Mutex};

use log::info;
use rsbinder::{hub, Parcel, SIBinder, StatusCode, Transactable};

use crate::identify;

/// The keystore2 service instance that owns the security-level objects.
const KEYSTORE2_SERVICE_NAME: &str = "android.system.keystore2.IKeystoreService/default";
/// SecurityLevel::TRUSTED_ENVIRONMENT; every device has this level.
const SECURITY_LEVEL_TEE: i32 = 0;

/// Verdict of one probe transaction against a local stub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProbeVerdict {
    /// The stub dispatched the code: any argument or permission failure keeps
    /// the code alive, because the stub reached its own argument parsing.
    Implemented,
    /// The stub has no transaction by that code.
    Unimplemented,
    /// The probe could not be interpreted; treat the code as implemented.
    Inconclusive,
}

static UNIMPLEMENTED_SECURITY_LEVEL_CODES: LazyLock<Mutex<Option<Vec<u32>>>> =
    LazyLock::new(|| Mutex::new(None));

/// True when the platform stub behind this device implements `code` on the
/// security-level interface. Resolved once; any probe failure keeps every code
/// implemented.
pub(super) fn platform_implements_security_level_code(code: u32) -> bool {
    let mut cache = UNIMPLEMENTED_SECURITY_LEVEL_CODES
        .lock()
        .expect("platform surface cache poisoned");
    if cache.is_none() {
        let unsupported = probe_unimplemented_security_level_codes();
        if !unsupported.is_empty() {
            info!(
                "event=platform_surface security-level codes this platform does not implement: {:?}; leaving them to the system stub",
                unsupported
            );
        }
        *cache = Some(unsupported);
    }
    !cache
        .as_ref()
        .expect("platform surface cache is populated")
        .contains(&code)
}

fn probe_unimplemented_security_level_codes() -> Vec<u32> {
    let Some(object) = real_security_level_object() else {
        return Vec::new();
    };
    let Some(stub) = object.as_transactable() else {
        return Vec::new();
    };
    identify::security_level_codes()
        .into_iter()
        .filter(|code| {
            probe_local_code(stub, identify::KEYSTORE_SECURITY_LEVEL_INTERFACE, *code)
                == ProbeVerdict::Unimplemented
        })
        .collect()
}

/// The in-process security-level object of the platform's own keystore2 service.
///
/// The injector runs inside the process that owns that object, so the handle
/// resolves to the local implementation. A proxy, or a service that is not up
/// yet, yields None and leaves every code implemented.
fn real_security_level_object() -> Option<SIBinder> {
    let service = hub::try_get_service(KEYSTORE2_SERVICE_NAME)
        .ok()
        .flatten()?;
    let stub = service.as_transactable()?;
    let mut data = Parcel::new();
    data.write(&identify::KEYSTORE_SERVICE_INTERFACE.to_string())
        .ok()?;
    data.write(&SECURITY_LEVEL_TEE).ok()?;
    let mut reply = Parcel::new();
    stub.transact(
        identify::SERVICE_GET_SECURITY_LEVEL_CODE,
        &mut data,
        &mut reply,
    )
    .ok()?;
    reply.set_data_position(0);
    let security_level: SIBinder = reply.read().ok()?;
    Some(security_level)
}

/// Ask a local stub whether it implements `code`.
///
/// The parcel carries only the interface token, so a stub that knows the code
/// fails while reading arguments and never runs the operation.
fn probe_local_code(stub: &dyn Transactable, interface: &str, code: u32) -> ProbeVerdict {
    let mut data = Parcel::new();
    if data.write(&interface.to_string()).is_err() {
        return ProbeVerdict::Inconclusive;
    }
    let mut reply = Parcel::new();
    match stub.transact(code, &mut data, &mut reply) {
        Ok(()) => {
            reply.set_data_position(0);
            match reply.read::<i32>() {
                Ok(status) if status == i32::from(StatusCode::UnknownTransaction) => {
                    ProbeVerdict::Unimplemented
                }
                _ => ProbeVerdict::Implemented,
            }
        }
        Err(status) if is_missing_code(status) => ProbeVerdict::Unimplemented,
        Err(_) => ProbeVerdict::Implemented,
    }
}

/// Status codes a stub uses for a transaction it does not have.
fn is_missing_code(status: StatusCode) -> bool {
    // Only the dedicated "no such transaction" status counts. Every other
    // failure keeps the code implemented, so an unexpected stub answer can only
    // preserve the previous behavior.
    status == StatusCode::UnknownTransaction
}

#[cfg(test)]
mod tests;
