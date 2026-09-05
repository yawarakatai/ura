# Pairing And Authentication

Status: Implemented for ura `0.4.0` using pairing protocol version 1.

`ura` uses temporary six-digit pairing sessions to issue long-lived bearer
credentials. The user-facing model is node/peer based, while protocol v1 is
still asymmetric at the credential level for compatibility.

## Threat Model

`ura` is intended for personal use on a trusted local network or private overlay
network. Pairing protects against casual unauthorized control by requiring
visibility of the terminal where the six-digit code is shown.

Pairing is not a defense for hostile networks or public internet exposure.
The peer API currently uses plain HTTP. Use a trusted tunnel or private overlay
if transport confidentiality is required; public-internet exposure is not a
supported deployment model.

The browser-facing control API is separate and binds only to `127.0.0.1:8766`.

## Trust Model

A running ura node owns the pairing session. The six-digit code:

- is valid for 120 seconds
- allows creation of one new random bearer credential
- has five valid-format incorrect attempts
- is never used as the long-lived credential

There is no IP-based authorization. An authorized client remains authorized by
its bearer token even if its address changes.

## Starting Pairing

On the node being authorized:

```bash
ura pair
```

This contacts the already-running node through:

```text
$XDG_RUNTIME_DIR/ura/control.sock
```

and starts a temporary pairing session. The code is returned only over this
local Unix socket and printed to the terminal. No HTTP endpoint can start a
pairing session.

## Pairing Another Ura Node

From another node:

```bash
ura pair 192.168.1.23
```

The CLI normalizes the address, checks `/v1/pair/info`, prompts for the code and
names when necessary, claims the session, and stores the returned token as a
peer credential.

Non-interactive use remains available:

```bash
ura pair 192.168.1.23 \
  --code 482913 \
  --node-name desktop \
  --name living-room \
  --select
```

In protocol v1, this establishes control in one direction: the initiating node
stores a token that authorizes it to control the paired node. The receiving node
stores only the token hash. Symmetric node identity/trust is intentionally left
for a future pairing protocol version.

## Firefox Pairing

Firefox `0.4.0` no longer pairs independently with every remote peer. It is
authorized once against the local ura node.

Setup is:

```bash
ura daemon
ura pair
```

Then enter the displayed six-digit code in the extension options.

The extension communicates only with the loopback API at:

```text
http://127.0.0.1:8766
```

The local API proxies `/v1/pair/info` and `/v1/pair/claim` to the same pairing
session owned by the local node. This is not a separate pairing mechanism: it
issues a normal authorized-client token through the same SQLite credential
store used by the peer API.

After pairing, Firefox stores only:

```json
{
  "nodeToken": "...",
  "defaultAction": "play",
  "localDeviceName": "Firefox"
}
```

Remote peer addresses, remote peer tokens, and the selected playback destination
are not owned by the extension. They remain ura node state. This means CLI and
Firefox device selection cannot drift apart.

Legacy extension settings are migrated only when they contain a localhost
credential. Old remote peer tokens are deliberately not treated as a local
node credential.

## Pairing API

Pairing protocol v1 uses:

```text
GET  /v1/pair/info
POST /v1/pair/claim
```

On the peer API these endpoints are unauthenticated only while a pairing session
is active. The loopback browser API provides proxies for the same two operations.

`GET /v1/pair/info` returns:

```json
{
  "pairing": true,
  "receiver_name": "kamo",
  "expires_in": 93
}
```

The `receiver_name` field keeps its protocol-v1 name for compatibility even
though the product model now calls it a node.

Inactive or expired pairing returns an error such as:

```json
{
  "error": "pairing_not_active"
}
```

`POST /v1/pair/claim` accepts:

```json
{
  "code": "482913",
  "device_name": "desktop"
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
device list, or database details.

## Pairing Lifecycle

- exactly one active pairing session per node
- six decimal digits, including leading zeroes
- 120-second lifetime
- five incorrect six-digit attempts
- malformed codes do not consume attempts
- successful claim consumes and closes the session
- timeout, attempt exhaustion, cancellation, and node shutdown close it
- starting a new session invalidates the previous one
- duplicate active authorized-client names are rejected

## Token Storage

Normal authenticated API requests use:

```text
Authorization: Bearer <token>
```

The authorizing node stores SHA-256 token hashes in SQLite, not plaintext tokens.
The `authorized_devices` table also records creation time, coarse
`last_seen_at`, and optional revocation time.

A node controlling a remote peer currently stores the peer-issued plaintext
token in its local configuration:

```toml
[[devices]]
name = "living-room"
url = "http://192.168.1.23:8765"
token = "..."
```

Firefox stores the plaintext credential issued by its local node in
`browser.storage.local` and never receives remote peer credentials in the new
model.

## Authentication Boundaries

There are two HTTP authentication surfaces:

### Peer API

The network-facing peer API checks:

1. active authorized-client token hashes in SQLite
2. the legacy configured peer API token as a compatibility fallback

### Browser Control API

The loopback-only browser API checks only active authorized-client token hashes
in SQLite. It does not accept the legacy peer API token as an implicit browser
credential.

The browser API remains bound to `127.0.0.1` even when the peer API is exposed
on LAN or Tailscale addresses.

## Revocation

Existing authorization commands remain available:

```bash
ura device authorize <name>
ura device revoke <name>
```

Revoking the credential used by Firefox causes subsequent browser requests to
return unauthorized; the extension then asks the user to pair it again.

## Explicit Non-Goals

The current pairing design does not implement HTTPS, public-internet exposure,
mDNS discovery, PAKE, HMAC request signing, QR codes, accounts, multi-user
permissions, Chromecast compatibility, Spotify direct playback, or DRM bypass.

A future protocol-v2 design may add stable node identity and symmetric peer
trust, but it should preserve the current routing invariant: network peer
requests always terminate at the receiving node rather than being forwarded
through its selected destination.
