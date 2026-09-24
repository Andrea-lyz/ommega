//! Transaction dispatch: `(transaction code, request arguments)` to an answer.
//!
//! Codes come from the vendor AIDL stubs in `vendor.qti.hardware.soter-V1-ndk.so`.
//! Everything the Java `SoterService` and its clients use is answered here; the
//! ATTK trio and the AIDL metadata codes are left to the stock HAL.
//!
//! Two callers share the same core. The host tests and the wire captures go
//! through [`handle`], which parses a request parcel and renders reply bytes. The
//! device daemon goes through [`handle_request`]: it owns the `AParcel` (only the
//! process holding it can read or write one), so it hands over the arguments it
//! has already read and encodes the answer from [`Outcome`].

use crate::error::{
    SOTER_ERR_BAD_VALUE, SOTER_ERR_NO_KEY, SOTER_ERR_NO_SESSION, SOTER_ERR_TA_UNAVAILABLE,
};
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

/// The ATTK transactions, which no Soter client on this ROM calls.
pub const ATTK_TRANSACTIONS: [u32; 3] = [TX_EXPORT_ATTK, TX_GENERATE_ATTK, TX_VERIFY_ATTK];

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

/// Arguments of one request, as the client wrote them on the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// No arguments, `getDeviceId`.
    None,
    /// `int32 uid`.
    Uid(u32),
    /// `int32 uid, string kname`.
    UidKey { uid: u32, kname: String },
    /// `int32 uid, string kname, string challenge`.
    InitSign {
        uid: u32,
        kname: String,
        challenge: String,
    },
    /// `int64 session`.
    Session(u64),
}

/// What one handled transaction answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A bare `SoterErrorCode`.
    Code(i32),
    /// A `SoterErrorCode` plus the `SoterBufferReturn` out parameter.
    ///
    /// `field` is the trailing `int32` of that out parameter. A device with a
    /// working TA writes the length of `data` there and the Java layer reports
    /// it to applications as `exportDataLength` (B-side capture of a live TA,
    /// 2026-09-24: 809 for a 809-byte ASK blob, 16 for the 16-byte device id).
    Buffer {
        code: i32,
        data: Vec<u8>,
        field: i32,
    },
    /// The `SoterInitReturn` parcelable `initSign` returns by value.
    Init { code: i32, session: u64 },
    /// A binder-level exception: status header plus message.
    Exception { code: i32, message: String },
}

impl Outcome {
    /// Render the reply payload the way the generated stubs expect it.
    ///
    /// The status header is part of the payload because the AIDL server side
    /// writes it: a `0` word first, then the return value, then the out
    /// parameters.
    pub fn to_reply_bytes(&self) -> Vec<u8> {
        match self {
            Outcome::Code(code) => {
                let mut reply = Reply::ok();
                reply.i32(*code);
                reply.into_bytes()
            }
            Outcome::Buffer { code, data, field } => {
                let mut reply = Reply::ok();
                reply.i32(*code);
                reply.buffer_return(data, *field);
                reply.into_bytes()
            }
            Outcome::Init { code, session } => {
                let mut reply = Reply::ok();
                reply.init_return(*code, *session as i64);
                reply.into_bytes()
            }
            Outcome::Exception { code, message } => Reply::exception(*code, message).into_bytes(),
        }
    }
}

fn malformed() -> Vec<u8> {
    Reply::exception(SOTER_ERR_BAD_VALUE, "malformed Soter request").into_bytes()
}

/// Read the arguments of `tx` out of a request parcel.
fn parse_request(tx: u32, args: &mut Args) -> Option<Request> {
    Some(match tx {
        TX_GET_DEVICE_ID => Request::None,
        TX_HAS_ASK | TX_GENERATE_ASK | TX_EXPORT_ASK | TX_REMOVE_ALL_UID_KEY => {
            Request::Uid(args.read_u32()?)
        }
        TX_GENERATE_AUTH | TX_HAS_AUTH | TX_EXPORT_AUTH | TX_REMOVE_AUTH => Request::UidKey {
            uid: args.read_u32()?,
            kname: args.read_string16()?,
        },
        TX_INIT_SIGN => Request::InitSign {
            uid: args.read_u32()?,
            kname: args.read_string16()?,
            challenge: args.read_string16()?,
        },
        TX_FINISH_SIGN => Request::Session(args.read_i64()? as u64),
        _ => return None,
    })
}

/// Answer one transaction from arguments that are already parsed.
///
/// `None` means the transaction is not this TA's business; the caller decides
/// what to do with it.
pub fn handle_request(state: &mut TaState, tx: u32, request: Request) -> Option<Outcome> {
    Some(match (tx, request) {
        (TX_GET_DEVICE_ID, Request::None) => {
            let (code, data) = state.device_id();
            Outcome::Buffer {
                field: data.len() as i32,
                code,
                data,
            }
        }
        (TX_HAS_ASK, Request::Uid(uid)) => Outcome::Code(state.has_ask(uid)),
        (TX_GENERATE_ASK, Request::Uid(uid)) => Outcome::Code(state.generate_ask(uid)),
        (TX_EXPORT_ASK, Request::Uid(uid)) => {
            let (code, data) = state.export_ask(uid);
            Outcome::Buffer {
                field: data.len() as i32,
                code,
                data,
            }
        }
        (TX_GENERATE_AUTH, Request::UidKey { uid, kname }) => {
            Outcome::Code(state.generate_auth(uid, &kname))
        }
        (TX_HAS_AUTH, Request::UidKey { uid, kname }) => Outcome::Code(state.has_auth(uid, &kname)),
        (TX_EXPORT_AUTH, Request::UidKey { uid, kname }) => {
            let (code, data) = state.export_auth(uid, &kname);
            Outcome::Buffer {
                field: data.len() as i32,
                code,
                data,
            }
        }
        (TX_REMOVE_AUTH, Request::UidKey { uid, kname }) => {
            Outcome::Code(state.remove_auth(uid, &kname))
        }
        (TX_REMOVE_ALL_UID_KEY, Request::Uid(uid)) => Outcome::Code(state.remove_all_uid(uid)),
        (
            TX_INIT_SIGN,
            Request::InitSign {
                uid,
                kname,
                challenge,
            },
        ) => {
            let (code, session) = state.init_sign(uid, &kname, &challenge);
            Outcome::Init { code, session }
        }
        (TX_FINISH_SIGN, Request::Session(session)) => {
            let (code, data) = state.finish_sign(session);
            Outcome::Buffer {
                field: data.len() as i32,
                code,
                data,
            }
        }
        _ => return None,
    })
}

/// Build the reply for one transaction, or `None` when the transaction belongs
/// to the stock HAL and must be passed through untouched.
pub fn handle(state: &mut TaState, tx: u32, args: &mut Args) -> Option<Vec<u8>> {
    match parse_request(tx, args) {
        Some(request) => handle_request(state, tx, request).map(|outcome| outcome.to_reply_bytes()),
        None if handles(tx) => Some(malformed()),
        None => None,
    }
}

/// What the stock HAL answers while its TA is dead, per transaction code.
///
/// Captured on the OnePlus 13 on 2026-09-24 by calling the live HAL with
/// `service call vendor.qti.hardware.soter.ISoter/default <code> ...` for every
/// code; `tests::dead_replies_match_captured_hal_failures` pins them byte for
/// byte. `soterta-svc --mode=dead` answers exactly this, so a client that talks
/// to the daemon sees what it saw with a dead TA: no key (-5), no device id
/// (-20), no sign session (-1000), and an empty `SoterBufferReturn` out
/// parameter wherever the reply carries one.
///
/// `None` means the code is not part of this interface.
pub fn dead_reply(tx: u32) -> Option<Outcome> {
    let empty = |code: i32| Outcome::Buffer {
        code,
        data: Vec::new(),
        field: 0,
    };
    Some(match tx {
        TX_EXPORT_ASK | TX_EXPORT_AUTH => empty(SOTER_ERR_NO_KEY),
        TX_EXPORT_ATTK | TX_GET_DEVICE_ID => empty(SOTER_ERR_TA_UNAVAILABLE),
        TX_FINISH_SIGN => empty(SOTER_ERR_NO_SESSION),
        TX_GENERATE_ASK | TX_GENERATE_ATTK | TX_VERIFY_ATTK => {
            Outcome::Code(SOTER_ERR_TA_UNAVAILABLE)
        }
        TX_HAS_ASK | TX_HAS_AUTH | TX_GENERATE_AUTH | TX_REMOVE_ALL_UID_KEY | TX_REMOVE_AUTH => {
            Outcome::Code(SOTER_ERR_NO_KEY)
        }
        TX_INIT_SIGN => Outcome::Init {
            code: SOTER_ERR_NO_KEY,
            session: 0,
        },
        _ => return None,
    })
}
