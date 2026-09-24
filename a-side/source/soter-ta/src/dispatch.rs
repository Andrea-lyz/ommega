//! Transaction dispatch: `(transaction code, request arguments)` to reply bytes.
//!
//! Codes come from the vendor AIDL stubs in `vendor.qti.hardware.soter-V1-ndk.so`.
//! Everything the Java `SoterService` and its clients use is answered here; the
//! ATTK trio and the AIDL metadata codes are left to the stock HAL.

use crate::error::SOTER_ERR_BAD_VALUE;
use crate::parcel::{Args, Reply};
use crate::state::TaState;

pub const TX_EXPORT_ASK: u32 = 1;
pub const TX_EXPORT_ATTK: u32 = 2;
pub const TX_EXPORT_AUTH: u32 = 3;
pub const TX_FINISH_SIGN: u32 = 4;
pub const TX_GENERATE_ASK: u32 = 5;
pub const TX_GENERATE_ATTK: u32 = 6;
pub const TX_GENERATE_AUTH: u32 = 7;
pub const TX_GET_DEVICE_ID: u32 = 8;
pub const TX_HAS_ASK: u32 = 9;
pub const TX_HAS_AUTH: u32 = 10;
pub const TX_INIT_SIGN: u32 = 11;
pub const TX_REMOVE_ALL_UID_KEY: u32 = 12;
pub const TX_REMOVE_AUTH: u32 = 13;
pub const TX_VERIFY_ATTK: u32 = 14;

/// Transactions this TA answers.
///
/// `tx2 exportAttkPublicKey`, `tx6 generateAttkKeyPair` and `tx14 verifyAttkKeyPair`
/// are deliberately absent: the APK's Java stubs do not even declare them, so no
/// Soter client reaches them, and the stock HAL keeps answering them.
pub fn handles(tx: u32) -> bool {
    matches!(
        tx,
        TX_EXPORT_ASK
            | TX_EXPORT_AUTH
            | TX_FINISH_SIGN
            | TX_GENERATE_ASK
            | TX_GENERATE_AUTH
            | TX_GET_DEVICE_ID
            | TX_HAS_ASK
            | TX_HAS_AUTH
            | TX_INIT_SIGN
            | TX_REMOVE_ALL_UID_KEY
            | TX_REMOVE_AUTH
    )
}

fn malformed() -> Vec<u8> {
    Reply::exception(SOTER_ERR_BAD_VALUE, "malformed Soter request").into_bytes()
}

/// Build the reply for one transaction, or `None` when the transaction belongs
/// to the stock HAL and must be passed through untouched.
pub fn handle(state: &mut TaState, tx: u32, args: &mut Args) -> Option<Vec<u8>> {
    let reply = match tx {
        TX_GET_DEVICE_ID => {
            let (code, data) = state.device_id();
            let mut reply = Reply::ok();
            reply.i32(code);
            reply.buffer_return(&data, 0);
            reply
        }
        TX_HAS_ASK => {
            let Some(uid) = args.read_u32() else {
                return Some(malformed());
            };
            let mut reply = Reply::ok();
            reply.i32(state.has_ask(uid));
            reply
        }
        TX_GENERATE_ASK => {
            let Some(uid) = args.read_u32() else {
                return Some(malformed());
            };
            let mut reply = Reply::ok();
            reply.i32(state.generate_ask(uid));
            reply
        }
        TX_EXPORT_ASK => {
            let Some(uid) = args.read_u32() else {
                return Some(malformed());
            };
            let (code, blob) = state.export_ask(uid);
            let mut reply = Reply::ok();
            reply.i32(code);
            reply.buffer_return(&blob, 0);
            reply
        }
        TX_GENERATE_AUTH => {
            let (Some(uid), Some(kname)) = (args.read_u32(), args.read_string16()) else {
                return Some(malformed());
            };
            let mut reply = Reply::ok();
            reply.i32(state.generate_auth(uid, &kname));
            reply
        }
        TX_HAS_AUTH => {
            let (Some(uid), Some(kname)) = (args.read_u32(), args.read_string16()) else {
                return Some(malformed());
            };
            let mut reply = Reply::ok();
            reply.i32(state.has_auth(uid, &kname));
            reply
        }
        TX_EXPORT_AUTH => {
            let (Some(uid), Some(kname)) = (args.read_u32(), args.read_string16()) else {
                return Some(malformed());
            };
            let (code, blob) = state.export_auth(uid, &kname);
            let mut reply = Reply::ok();
            reply.i32(code);
            reply.buffer_return(&blob, 0);
            reply
        }
        TX_REMOVE_AUTH => {
            let (Some(uid), Some(kname)) = (args.read_u32(), args.read_string16()) else {
                return Some(malformed());
            };
            let mut reply = Reply::ok();
            reply.i32(state.remove_auth(uid, &kname));
            reply
        }
        TX_REMOVE_ALL_UID_KEY => {
            let Some(uid) = args.read_u32() else {
                return Some(malformed());
            };
            let mut reply = Reply::ok();
            reply.i32(state.remove_all_uid(uid));
            reply
        }
        TX_INIT_SIGN => {
            let (Some(uid), Some(kname), Some(challenge)) =
                (args.read_u32(), args.read_string16(), args.read_string16())
            else {
                return Some(malformed());
            };
            let (code, session) = state.init_sign(uid, &kname, &challenge);
            let mut reply = Reply::ok();
            reply.init_return(code, session as i64);
            reply
        }
        TX_FINISH_SIGN => {
            let Some(session) = args.read_i64() else {
                return Some(malformed());
            };
            let (code, blob) = state.finish_sign(session as u64);
            let mut reply = Reply::ok();
            reply.i32(code);
            reply.buffer_return(&blob, 0);
            reply
        }
        _ => return None,
    };
    Some(reply.into_bytes())
}

