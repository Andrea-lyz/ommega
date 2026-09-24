//! Host-side tests for the software TA.
//!
//! Two of these are anchored to bytes captured from the live HAL of the OnePlus 13
//! (`getDeviceId` and `hasAskAlready` while its TA was dead), so any drift in the
//! reply encoder away from the vendor wire format fails here.

use crate::blob;
use crate::dispatch;
use crate::error::{SOTER_ERR_NO_KEY, SOTER_ERR_TA_UNAVAILABLE, SOTER_OK};
use crate::parcel::{self, Args, Reply};
use crate::state::TaState;

const UID: u32 = 10503;
const KNAME: &str = "ommega_default_key";

/// Builds a request parcel the way the generated client stubs do: strict-mode
/// policy word, interface token, then the arguments.
struct RequestBuilder {
    buf: Vec<u8>,
}

impl RequestBuilder {
    fn new(token: &str) -> Self {
        let mut builder = Self { buf: Vec::new() };
        builder.buf.extend_from_slice(&0i32.to_le_bytes());
        builder.string16(token)
    }

    fn i32(mut self, value: i32) -> Self {
        self.buf.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn i64(mut self, value: i64) -> Self {
        self.buf.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn string16(mut self, value: &str) -> Self {
        let units: Vec<u16> = value.encode_utf16().collect();
        self.buf.extend_from_slice(&(units.len() as i32).to_le_bytes());
        for unit in &units {
            self.buf.extend_from_slice(&unit.to_le_bytes());
        }
        self.buf.extend_from_slice(&0u16.to_le_bytes());
        while self.buf.len() % 4 != 0 {
            self.buf.push(0);
        }
        self
    }

    fn build(self) -> Vec<u8> {
        self.buf
    }
}

fn state() -> TaState {
    TaState::generate_local().expect("local TA state")
}

fn call(state: &mut TaState, tx: u32, request: &[u8]) -> Vec<u8> {
    let (token, mut args) = Args::parse_request(request).expect("request parses");
    assert!(Args::is_soter_token(&token), "unexpected token: {token}");
    dispatch::handle(state, tx, &mut args).expect("transaction belongs to the software TA")
}

fn word(bytes: &[u8], index: usize) -> i32 {
    let start = index * 4;
    i32::from_le_bytes(bytes[start..start + 4].try_into().expect("word"))
}

/// Reads `[status][rc][presence][size][byte[] buffer][int32 field]`.
fn parse_buffer_return(reply: &[u8]) -> Vec<u8> {
    let mut args = Args::new(reply);
    assert_eq!(args.read_i32(), Some(0), "status header");
    assert_eq!(args.read_i32(), Some(SOTER_OK), "return code");
    assert_eq!(args.read_i32(), Some(1), "out parameter is present");
    let size = args.read_i32().expect("size");
    let buffer = args.read_byte_array().expect("buffer");
    assert_eq!(args.read_i32(), Some(0), "buffer field");
    assert_eq!(size as usize, 4 + 4 + ((buffer.len() + 3) & !3) + 4);
    buffer
}

fn counter_of(blob_bytes: &[u8]) -> u64 {
    let (json, _) = blob::parse(blob_bytes).expect("blob structure");
    let value: serde_json::Value = serde_json::from_slice(json).expect("blob JSON");
    value["counter"].as_u64().expect("counter")
}

fn json_field(blob_bytes: &[u8], field: &str) -> String {
    let (json, _) = blob::parse(blob_bytes).expect("blob structure");
    let value: serde_json::Value = serde_json::from_slice(json).expect("blob JSON");
    value[field].as_str().expect("string field").to_string()
}

#[test]
fn reply_bytes_match_captured_hal_failures() {
    // getDeviceId with a dead TA: status ok, rc -20, a present but empty
    // SoterBufferReturn.
    let mut reply = Reply::ok();
    reply.i32(SOTER_ERR_TA_UNAVAILABLE);
    reply.buffer_return(&[], 0);
    assert_eq!(
        hex::encode(reply.into_bytes()),
        "00000000ecffffff010000000c0000000000000000000000"
    );

    // hasAskAlready for a uid with no ASK: status ok, rc -5.
    let mut reply = Reply::ok();
    reply.i32(SOTER_ERR_NO_KEY);
    assert_eq!(hex::encode(reply.into_bytes()), "00000000fbffffff");
}

#[test]
fn request_parsing_matches_client_layout() {
    let request = RequestBuilder::new(parcel::SOTER_INTERFACE)
        .i32(UID as i32)
        .string16(KNAME)
        .build();
    let (token, mut args) = Args::parse_request(&request).expect("parse");
    assert_eq!(token, parcel::SOTER_INTERFACE);
    assert!(Args::is_soter_token(&token));
    assert_eq!(args.read_u32(), Some(UID));
    assert_eq!(args.read_string16().as_deref(), Some(KNAME));

    // the `<descriptor>/default` spelling is accepted too
    let request = RequestBuilder::new(parcel::SOTER_INTERFACE_INSTANCE).build();
    let (token, _) = Args::parse_request(&request).expect("parse");
    assert!(Args::is_soter_token(&token));
}

#[test]
fn ask_blob_signature_verifies_under_the_device_key() {
    let mut state = state();
    assert_eq!(state.generate_ask(UID), SOTER_OK);
    let (code, blob_bytes) = state.export_ask(UID);
    assert_eq!(code, SOTER_OK);

    let (json, signature) = blob::parse(&blob_bytes).expect("blob structure");
    assert_eq!(signature.len(), blob::SIGNATURE_LEN);
    let device_key = blob::public_key_from_pem(&state.attk.public_pem).expect("device key");
    assert!(blob::verify(&device_key, json, signature));

    let text = String::from_utf8(json.to_vec()).expect("utf8 json");
    let position = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("{needle} in {text}"));
    assert!(position("\"pub_key\"") < position("\"cpu_id\""));
    assert!(position("\"cpu_id\"") < position("\"counter\""));
    assert!(position("\"counter\"") < position("\"uid\""));
    assert!(!text.contains("certs"), "{text}");
}

#[test]
fn auth_blob_and_sign_result_verify_along_the_chain() {
    let mut state = state();
    assert_eq!(state.generate_ask(UID), SOTER_OK);
    assert_eq!(state.generate_auth(UID, KNAME), SOTER_OK);

    let (code, auth_blob) = state.export_auth(UID, KNAME);
    assert_eq!(code, SOTER_OK);
    let (auth_json, auth_signature) = blob::parse(&auth_blob).expect("auth blob");
    let ask_public = blob::public_key_from_pem(
        &state.uids[&UID].ask.as_ref().expect("ask").public_pem,
    )
    .expect("ask public key");
    assert!(blob::verify(&ask_public, auth_json, auth_signature));

    let (code, session) = state.init_sign(UID, KNAME, "challenge-1");
    assert_eq!(code, SOTER_OK);
    assert!(session > 0);

    let (code, result_blob) = state.finish_sign(session);
    assert_eq!(code, SOTER_OK);
    let (result_json, result_signature) = blob::parse(&result_blob).expect("result blob");
    let auth_public =
        blob::public_key_from_pem(&state.uids[&UID].auth[KNAME].public_pem).expect("auth key");
    assert!(blob::verify(&auth_public, result_json, result_signature));

    let text = String::from_utf8(result_json.to_vec()).expect("utf8 json");
    for field in [
        "raw", "fid", "counter", "tee_n", "tee_v", "fp_n", "fp_v", "cpu_id", "uid",
    ] {
        assert!(text.contains(&format!("\"{field}\"")), "{text}");
    }
    assert!(text.contains("\"raw\":\"challenge-1\""), "{text}");
}

#[test]
fn counters_are_strictly_increasing_per_uid() {
    let mut state = state();
    let other = 10349;
    assert_eq!(state.generate_ask(UID), SOTER_OK);
    assert_eq!(state.generate_ask(other), SOTER_OK);

    let (_, first) = state.export_ask(UID);
    let (_, second) = state.export_ask(UID);
    let (_, independent) = state.export_ask(other);
    assert_eq!(counter_of(&second), counter_of(&first) + 1);
    assert!(counter_of(&independent) > 0);
    assert_ne!(counter_of(&independent), counter_of(&second));
}

#[test]
fn ledger_tracks_presence_and_removal() {
    let mut state = state();
    assert_eq!(state.has_ask(UID), SOTER_ERR_NO_KEY);
    assert_eq!(state.export_ask(UID).0, SOTER_ERR_NO_KEY);
    assert_eq!(state.generate_auth(UID, KNAME), SOTER_ERR_NO_KEY, "no ASK yet");

    assert_eq!(state.generate_ask(UID), SOTER_OK);
    assert_eq!(state.has_ask(UID), SOTER_OK);
    assert_eq!(state.generate_auth(UID, KNAME), SOTER_OK);
    assert_eq!(state.has_auth(UID, KNAME), SOTER_OK);

    assert_eq!(state.remove_auth(UID, KNAME), SOTER_OK);
    assert_eq!(state.has_auth(UID, KNAME), SOTER_ERR_NO_KEY);
    assert_eq!(state.remove_auth(UID, KNAME), SOTER_ERR_NO_KEY);

    assert_eq!(state.remove_all_uid(UID), SOTER_OK);
    assert_eq!(state.has_ask(UID), SOTER_ERR_NO_KEY);
    assert_eq!(state.remove_all_uid(UID), SOTER_ERR_NO_KEY);
    assert_eq!(state.has_ask(999_999), SOTER_ERR_NO_KEY);
}

#[test]
fn sign_sessions_are_single_use() {
    let mut state = state();
    assert_eq!(state.generate_ask(UID), SOTER_OK);
    assert_eq!(state.generate_auth(UID, KNAME), SOTER_OK);

    let (code, session) = state.init_sign(UID, KNAME, "challenge");
    assert_eq!(code, SOTER_OK);
    assert_eq!(state.finish_sign(session).0, SOTER_OK);
    assert_ne!(state.finish_sign(session).0, SOTER_OK, "session is consumed");
    assert_ne!(state.finish_sign(4242).0, SOTER_OK, "unknown session");
    assert_ne!(state.init_sign(UID, "missing", "challenge").0, SOTER_OK);

    // removing the key invalidates a session that was already opened
    let (_, session) = state.init_sign(UID, KNAME, "challenge");
    assert_eq!(state.remove_all_uid(UID), SOTER_OK);
    assert_ne!(state.finish_sign(session).0, SOTER_OK);
}

#[test]
fn device_id_matches_the_cpu_id_inside_the_blobs() {
    let mut state = state();
    let (code, id) = state.device_id();
    assert_eq!(code, SOTER_OK);
    let id = String::from_utf8(id).expect("ascii device id");
    assert_eq!(id, state.cpu_id);
    assert_eq!(id.len(), 32);
    assert!(id.starts_with("00000000"));

    assert_eq!(state.generate_ask(UID), SOTER_OK);
    let (_, ask_blob) = state.export_ask(UID);
    assert_eq!(json_field(&ask_blob, "cpu_id"), id);
}

#[test]
fn state_json_round_trip_preserves_the_ledger() {
    let mut state = state();
    assert_eq!(state.generate_ask(UID), SOTER_OK);
    assert_eq!(state.generate_auth(UID, KNAME), SOTER_OK);
    let (_, before) = state.export_auth(UID, KNAME);

    let encoded = state.to_json();
    let mut restored = TaState::from_json(&encoded).expect("round trip");
    assert_eq!(restored.cpu_id, state.cpu_id);
    assert_eq!(restored.has_ask(UID), SOTER_OK);
    assert_eq!(restored.has_auth(UID, KNAME), SOTER_OK);
    let (code, after) = restored.export_auth(UID, KNAME);
    assert_eq!(code, SOTER_OK);
    assert_eq!(json_field(&before, "pub_key"), json_field(&after, "pub_key"));
    assert!(counter_of(&after) > counter_of(&before));
}

#[test]
fn dispatch_serves_the_java_layer_and_leaves_attk_alone() {
    let mut state = state();
    let token = parcel::SOTER_INTERFACE;

    assert!(dispatch::handles(dispatch::TX_GENERATE_ASK));
    assert!(!dispatch::handles(dispatch::TX_VERIFY_ATTK));
    assert!(!dispatch::handles(dispatch::TX_EXPORT_ATTK));
    assert!(!dispatch::handles(dispatch::TX_GENERATE_ATTK));

    let reply = call(
        &mut state,
        dispatch::TX_HAS_ASK,
        &RequestBuilder::new(token).i32(UID as i32).build(),
    );
    assert_eq!(word(&reply, 1), SOTER_ERR_NO_KEY, "no ASK yet");

    let reply = call(
        &mut state,
        dispatch::TX_GENERATE_ASK,
        &RequestBuilder::new(token).i32(UID as i32).build(),
    );
    assert_eq!(word(&reply, 0), 0);
    assert_eq!(word(&reply, 1), SOTER_OK);

    let reply = call(
        &mut state,
        dispatch::TX_EXPORT_ASK,
        &RequestBuilder::new(token).i32(UID as i32).build(),
    );
    assert_eq!(word(&reply, 2), 1, "out parameter present");
    let exported = parse_buffer_return(&reply);
    let (json, signature) = blob::parse(&exported).expect("blob");
    let device_key = blob::public_key_from_pem(&state.attk.public_pem).expect("device key");
    assert!(blob::verify(&device_key, json, signature));

    let reply = call(
        &mut state,
        dispatch::TX_GENERATE_AUTH,
        &RequestBuilder::new(token)
            .i32(UID as i32)
            .string16(KNAME)
            .build(),
    );
    assert_eq!(word(&reply, 1), SOTER_OK);

    let reply = call(
        &mut state,
        dispatch::TX_INIT_SIGN,
        &RequestBuilder::new(token)
            .i32(UID as i32)
            .string16(KNAME)
            .string16("challenge")
            .build(),
    );
    assert_eq!(word(&reply, 0), 0, "status header");
    assert_eq!(word(&reply, 1), 1, "return value present");
    assert_eq!(word(&reply, 2), 16, "SoterInitReturn size");
    assert_eq!(word(&reply, 3), SOTER_OK);
    let session = i64::from_le_bytes(reply[16..24].try_into().expect("session"));
    assert!(session > 0);

    let reply = call(
        &mut state,
        dispatch::TX_FINISH_SIGN,
        &RequestBuilder::new(token).i64(session).build(),
    );
    let result = parse_buffer_return(&reply);
    let (json, signature) = blob::parse(&result).expect("result blob");
    let auth_key =
        blob::public_key_from_pem(&state.uids[&UID].auth[KNAME].public_pem).expect("auth key");
    assert!(blob::verify(&auth_key, json, signature));

    let reply = call(
        &mut state,
        dispatch::TX_GET_DEVICE_ID,
        &RequestBuilder::new(token).build(),
    );
    let id = parse_buffer_return(&reply);
    assert_eq!(String::from_utf8(id).expect("ascii"), state.cpu_id);

    let reply = call(
        &mut state,
        dispatch::TX_REMOVE_ALL_UID_KEY,
        &RequestBuilder::new(token).i32(UID as i32).build(),
    );
    assert_eq!(word(&reply, 1), SOTER_OK);
    let reply = call(
        &mut state,
        dispatch::TX_HAS_ASK,
        &RequestBuilder::new(token).i32(UID as i32).build(),
    );
    assert_eq!(word(&reply, 1), SOTER_ERR_NO_KEY);

    // a truncated request must not be passed through in silence
    let reply = call(&mut state, dispatch::TX_HAS_ASK, &RequestBuilder::new(token).build());
    assert_ne!(word(&reply, 0), 0, "exception status");
}

