# Rust relay server protocol notes

## B battery telemetry

Battery metadata uses the existing authenticated `GET /api/b/poll/` heartbeat;
there is no extra endpoint, database migration or persistent telemetry file.
Required device identity, token validation, concurrency guard, task routing and
200-task/204-empty responses are unchanged. Optional query fields:

| Field | Valid values | Missing, empty or malformed |
| --- | --- | --- |
| `battery_percent` | decimal integer 0 through 100, inclusive | unknown |
| `battery_charging` | literal `true` or `false` (active charging) | unknown |

Attributes are validated independently. Bad battery metadata does not reject a
valid task poll and is never clamped or defaulted to zero/false. A poll replaces
the previous battery snapshot, so an old relay, B APK or failed sysfs read cannot
keep a last-good value appearing current. The B Rust relay samples read-only
primary-battery sysfs at most once every five seconds and includes supported
values on its usual polls. `Full` is not active charging. It never invokes
HAL/TEE or a shell to obtain battery data.

`GET /api/status/` publishes the following additive JSON properties in each
`online_b_devices` row. The authenticated `GET /api/admin/devices/` response
also includes them in `connected_b_devices`:

```json
{
  "device_id": "configured-b-device",
  "battery_percent": 0,
  "battery_charging": false,
  "battery_updated_at_ms": 1793220000000
}
```

- `battery_percent` is an integer in 0..100 or JSON null; 0 is a real reading.
- `battery_charging` is a boolean or JSON null, independently of percent.
- `battery_updated_at_ms` is the server's Unix-millisecond receipt time when at
  least one value is valid; it is null when neither is valid. It is captured once
  before a poll waits, never refreshed by queue notifications/timeouts, and never
  taken from the B clock. It is not an exact sysfs measurement timestamp: the B
  sample may already be up to five seconds old when the poll is sent.

Consumers must use the exact configured B device ID, check the receipt timestamp
against `server_time_ms`, and show unknown for missing/stale reports instead of
substituting another online device. Existing online-device expiry (120 seconds)
continues to apply; offline devices are omitted. Missing telemetry on older
servers/relays remains unknown. The public response still omits machine IDs,
credentials, keys and certificates. Older B clients may omit both fields and
older servers may ignore the added query parameters.

Both updated B relay and server binaries must be explicitly deployed before
battery values arrive. This change does not restart, deploy to or probe devices;
server restart also discards the in-memory snapshots until the next poll.

## B token device binding

A DB-issued B token serves only the device ids an admin bound to it
(`api_token.allowed_device_ids`, comma separated; set when generating the token
or with `POST /api/admin/tokens/devices/` `{id, allowed_device_ids}`). The
column is added automatically on first use; if the DB user cannot `ALTER`,
add it manually: `ALTER TABLE api_token ADD COLUMN allowed_device_ids
VARCHAR(1024) NOT NULL DEFAULT ''`. `b/poll`, `b/result`,
`b/upload_keybox_identity` and `b/revoke_server_identity` answer `403` for a
`device_id` the token is not bound to, so an unbound B token serves nothing.
The static `RELAY_TOKEN` (the owner's credential) still serves any device.
`b/poll` also requires a non-empty `machine_id` (`400` otherwise).

`/api/card/query/` (by contact) returns only masked tokens and order ids; the
full token is returned only by `/api/card/order/status/` for the random order id.

## Card lottery

`/api/card/lottery/` allows 3 draws per client per UTC day. IPv4 (including
IPv4-mapped IPv6) is counted per address; other IPv6 addresses are counted per
/64 prefix, so rotating addresses inside one allocation does not add draws.
At most 5 draws win per UTC day across all clients; once reached, later draws
still count but do not win.

## Admin keybox DeviceID

An uploaded `keybox.xml` `DeviceID`, and any admin-supplied `device_id` for a
stored identity, must be 1-128 characters from `A-Z a-z 0-9 . _ -`; anything
else is rejected with `400`. Admin console device buttons carry the id in
`data-*` attributes instead of inline script handlers.

## Rate limits

`RELAY_DATA_RATE_LIMIT_REQUESTS` (default `20000`) is the work budget for the
relay data plane: A task endpoints and `POST /api/b/result/`. `GET /api/b/poll/`
has its own budget, `RELAY_POLL_RATE_LIMIT_REQUESTS` (default `0`, disabled), so
idle heartbeats cannot exhaust the work budget. The B Rust relay holds idle
polls for up to 20 seconds; immediate empty responses retain an 80ms floor.
The default also permits older clients that use short polling.
Both values are per token per `RELAY_RATE_LIMIT_WINDOW` seconds (default
`3600`) and can be set in the process environment or the working-directory
`.env` file.

Admin login allows 20 attempts per minute per IP and 5 per IP+username (`429`
beyond that); the username enters the key only as a hash of its first 256
bytes. Every limiter map drops keys whose window has emptied and holds at most
4096 keys, evicting the least recently used when full; keys over 128 bytes are
stored as hashes.

## Low-latency relay transport

`timeout` on `GET /api/b/poll/` is an idle hold duration, capped at 120 seconds.
A task arrival wakes only the requested device's waiting polls. Wildcard work
can wake any device; queue locking still permits exactly one claim per task.
Notifications are enabled before checking the queue so arrivals cannot be lost
in a check-to-sleep race. Cancelled polls release their waiter; a later poll
checks already queued work before waiting. Reclaimed work notifies the
appropriate poll. Lost attest/agree/SOTER assignments fail with execution
state unknown; the server does not replay them or accept a late result for
the failed assignment.

Physical `attest`/`profile`/`decrypt`/`agree` waits at most 25 seconds (or
`RELAY_WAIT_RESULT_TIMEOUT` if it is lower). B may claim the task only during
the first 80% of that wait and must start executing it by 90%: the poll
response carries the remaining budget as `start_within_ms`, and B reports an
expiry error instead of entering the HAL once it has passed. A retry with
the same request ID keeps the original deadlines; unclaimed or reclaimed
work past the claim deadline is expired before dispatch; reaching the HTTP
deadline does not interrupt work already claimed by B. `sign`/`delete` keep
the 10-second wait and cancel unclaimed work.

A result is published through that task's watch channel, not broadcast to B
polls. An identical resend of an already-recorded result from the assigned
device is acknowledged as a duplicate (200) instead of a 409, so a B retry
that lost its first ack is not misreported as an upload failure. Both the
HTTP and Rustls HTTPS listeners enable `TCP_NODELAY`; TLS, authentication,
request IDs, assignment checks and result contents are unchanged.
Reverse proxies must allow a 20-second hold (use a read timeout of at least
30 seconds); no buffering or retry setting may replay a stateful task.

At debug level, `relay_claim` records monotonic `queue_us`; `relay_complete`
records `assigned_us` and `total_us`, all correlated by task ID. Queue time
starts at task creation; assigned time includes B transport, dispatch, execution
and result arrival. Do not sum nested timers or subtract timestamps across
devices. These are in-memory/log observations, not a new database schema.

## Safe local validation

From this directory, `cargo test --lib` exercises the parser, queue and mock
handler round trips without a database, network listener or device. Focused
battery tests can be selected with `cargo test --lib battery`. The B source
workspace's Android fmt/clippy/no-run gates remain required for Android build
acceptance; host-only tests do not establish on-device sysfs readability.

The isolated transport benchmark uses the actual server binary plus HTTP A/B
software stubs with fixed 20ms work, never a device or real TEE:

```sh
python3 tests/link_bench.py target/x86_64-unknown-linux-musl/release/relay_rs --samples 60
python3 tests/link_bench.py target/x86_64-unknown-linux-musl/release/relay_rs --samples 60 --tls
```

Both strategies use the same candidate server to isolate the 80ms polling
cadence. TLS mode generates a temporary loopback-only certificate with OpenSSL
and verifies it with a fixture CA. The benchmark starts only its own ephemeral
127.0.0.1 listener and stops it on exit. It does not measure WAN, hardware time,
A-side chain verification or Keystore storage, and makes no T04 claim.
