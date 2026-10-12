pub const SOCKET: &str = "/data/misc/keystore/ommega/rpc.sock";
/// Bounded connections within one session; Binder identities stay shared.
pub const MAX_OUTGOING_CONNECTIONS: u32 = 4;
pub const WIRE_MAX_VERSION: u32 = 2;

pub const SERVICE: &str = "ommega";
pub const AUTHORIZATION_SERVICE: &str = "ommega_authorization";
pub const MAINTENANCE_SERVICE: &str = "ommega_maintenance";
pub const METRICS_SERVICE: &str = "ommega_metrics";
