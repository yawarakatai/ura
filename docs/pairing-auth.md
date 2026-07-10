# Pairing And Authentication

Status: Implemented for CLI receiver-screen pairing.

`ura` supports per-device bearer tokens, multiple configured receivers, and
receiver-screen pairing through `ura pair`.

## Threat Model

`ura` is for personal use on a trusted local network or private overlay network.
Pairing protects against casual unauthorized control by requiring visibility of
the receiver terminal where the six-digit code is shown.

Pairing is not a defense for hostile networks or public internet exposure.
Plain HTTP remains visible to anyone who can observe the network path. Use a
trusted tunnel or reverse proxy if transport encryption is required.

## Trust Model

The receiver screen is the trusted out-of-band channel. The six-digit code is
valid for 120 seconds and allows creation of one new random bearer token. The
long-lived bearer token is not derived from the code.

There is no IP-based authorization. A controller remains authorized by its
bearer token even if its address changes.

## CLI Pairing

On the receiver:

```bash
ura pair
```

This contacts the already-running local `ura serve` process through
`$XDG_RUNTIME_DIR/ura/control.sock`, starts one temporary pairing session, and
prints the receiver address, six-digit code, and expiry. It waits until pairing
succeeds, expires, is cancelled, or reaches five invalid attempts.

On the controller:

```bash
ura pair 192.168.1.23
```

The controller normalizes the receiver address, checks `/v1/pair/info`, prompts
for the code, prompts for this controller's device name when needed, prompts for
the local receiver alias, submits the claim, stores the returned token in
`config.toml`, and optionally selects the receiver.

Non-interactive use:

```bash
ura pair 192.168.1.23 \
  --code 482913 \
  --device-name desuwa \
  --name kamo \
  --select
```

## Pairing API

Pairing adds two unauthenticated endpoints that work only while a pairing
session is active:

```text
GET  /v1/pair/info
POST /v1/pair/claim
```

`GET /v1/pair/info` returns minimal state:

```json
{
  "pairing": true,
  "receiver_name": "kamo",
  "expires_in": 93
}
```

Inactive or expired pairing returns `403` with:

```json
{
  "error": "pairing_not_active"
}
```

`POST /v1/pair/claim` accepts:

```json
{
  "code": "482913",
  "device_name": "desuwa"
}
```

On success it returns the plaintext bearer token exactly once:

```json
{
  "protocol_version": 1,
  "receiver_name": "kamo",
  "token": "<new-random-bearer-token>"
}
```

The API does not return the pairing code, token hash, database IDs, authorized
device list, or receiver database details.

## Lifecycle

Pairing behavior:

- exactly one active pairing session per receiver
- six decimal digits, including leading zeroes
- 120-second lifetime
- five invalid six-digit attempts
- malformed codes do not consume attempts
- successful claim consumes and closes the session
- timeout, attempt exhaustion, cancellation, and receiver shutdown close it
- starting a new session invalidates the previous one

The receiver first verifies the code, then registers the supplied device name.
Duplicate active authorized-device names return `409`.

## Token Model

Normal API requests use:

```text
Authorization: Bearer <token>
```

Receiver-side authorized devices live in SQLite and store SHA-256 token hashes,
not plaintext tokens. Pairing and `ura device authorize <name>` use the same
internal authorization path to create a random token with at least 256 bits of
entropy and store only its hash.

Controller-side remote receivers are stored in config as `[[devices]]` entries.
`ura pair <ADDRESS>` reuses the same config update behavior as
`ura device add <name> <address> --token <token>` and can also set
`selected_device`.

Legacy flat `receiver_url` and `token` config remains readable when no
`[[devices]]` entries exist.

## Browser Extension

The browser extension pairing UI is still planned. The protocol is intentionally
plain JSON over:

```text
GET /v1/pair/info
POST /v1/pair/claim
```

so the extension can later pair without CLI-specific assumptions.

## Explicit Non-Goals

Pairing does not implement HTTPS, mDNS discovery, browser-extension pairing UI,
IP allowlists, PAKE, HMAC request signing, QR codes, public-internet exposure,
Chromecast compatibility, Spotify direct playback, or DRM bypass.
