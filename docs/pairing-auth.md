# Pairing And Authentication

Status: Planned; not yet implemented.

This document describes the decided next-generation pairing and authentication
design. The current implementation still uses a single shared bearer token from
`config.toml`; see [architecture.md](architecture.md) for current behavior.

## Threat Model

`ura` is for personal use on a trusted local network or private overlay network.
The pairing design protects against casual unauthorized control by another
device on the same network and avoids copying one shared long-lived secret to
every controller.

Expected impact is limited to controlling local audio playback and reading local
playback history exposed by the receiver API. The design is not intended for
hostile networks, public internet exposure, or high-value multi-user access
control.

## Trust Model

The receiver screen is the trusted out-of-band channel. Pairing requires
physical visibility of the receiver terminal or display where `ura pair` shows a
short code.

There is no IP-based authentication. Network location can help reach the
receiver, but it does not prove identity.

## Receiver Workflow: `ura pair`

Planned receiver flow:

1. The user runs `ura pair` on the receiver.
2. The receiver prints a six-digit pairing code.
3. The code is valid for 120 seconds.
4. The receiver accepts at most five failed attempts.
5. Pairing closes after success, timeout, or attempt exhaustion.
6. On success, the receiver creates a bearer token for that peer.
7. The receiver stores only a hash of the token.

The raw bearer token is shown or returned only during pairing and cannot be
recovered from receiver storage later.

## Controller Workflow: `ura pair <ADDRESS>`

Planned controller CLI flow:

1. The user runs `ura pair <receiver-address>`.
2. The controller normalizes the address.
3. The controller prompts for the six-digit code shown by the receiver.
4. On success, the controller stores the receiver URL and issued bearer token.

Controllers may store multiple receivers. Each receiver entry has its own URL
and bearer token.

## Planned Device Commands

These commands are planned and are not implemented in the current CLI:

```text
ura pair
ura pair <ADDRESS>
ura device list
ura device select <NAME>
ura device remove <NAME>
ura play --to <NAME> <URL>
ura queue --to <NAME> <URL>
```

`ura pair` opens pairing on the local receiver. `ura pair <ADDRESS>` pairs this
controller or extension with the specified receiver.

`ura device list` lists known remote receivers and devices authorized to control
the local receiver. `ura device select <NAME>` persistently selects the device
used when `--to` is omitted. `ura device remove <NAME>` removes a known remote
device or revokes a locally authorized device. `--to <NAME>` overrides the
selected destination for one command.

## Browser Extension Workflow

The browser extension must be able to pair without the CLI. Planned extension
flow:

1. The user enters or discovers the receiver address in extension options.
2. The extension normalizes the address.
3. The extension prompts for the six-digit code shown by `ura pair`.
4. On success, the extension stores the receiver URL and issued bearer token.

The extension should not require users to copy tokens manually once pairing is
implemented.

## Address Normalization

Planned address handling:

- accept hostnames, IPv4 addresses, and private overlay addresses
- add `http://` when no scheme is provided
- reject schemes other than `http://`
- add the default port `8765` when no port is provided
- preserve an explicit path prefix when present
- trim surrounding whitespace

Examples:

```text
receiver.local        -> http://receiver.local:8765
192.168.1.20          -> http://192.168.1.20:8765
100.x.y.z:9000        -> http://100.x.y.z:9000
http://host:8765/ura  -> http://host:8765/ura
```

## Token Model

Pairing issues per-device bearer tokens. A receiver can have multiple paired
devices, and each device has an independent token. A controller can store
multiple receiver devices. Internal implementation details may still use `peer`
where technically appropriate.

The receiver stores token hashes, not raw bearer tokens. Requests still use:

```text
Authorization: Bearer <token>
```

## Current Manual Bridge

Pairing-code setup is planned. The current bridge is manual per-device
authorization:

```bash
ura device authorize desuwa
ura device add kamo 192.168.1.23 --token <shown-token>
ura device select kamo
ura play --to kamo "https://youtu.be/..."
ura device remove kamo
ura device revoke desuwa
```

`ura pair` will automate creation and transfer of the same per-device bearer
credential that `device authorize` creates today.

## Legacy Configuration Compatibility

Legacy configuration uses one shared token in `config.toml`:

```toml
token = "..."
receiver_url = "http://127.0.0.1:8765"
bind = "127.0.0.1:8765"
```

Transition behavior:

- keep existing single-token configuration working during transition
- allow manual authorization and future pairing to create per-device credentials alongside the existing token
- provide a clear path to revoke or remove the legacy shared token later
- avoid breaking existing local-only users without an explicit migration step

## Rationale For Plain HTTP And Bearer Auth

The receiver is intended for local networks and private overlays, not the public
internet. Plain HTTP keeps setup simple for browsers, CLIs, and LAN-only
devices. Bearer tokens are easy for the current CLI and extension to use and are
compatible with the existing API.

Pairing improves token distribution, not transport security. Users who need
transport encryption should place `ura` behind a local trusted tunnel or reverse
proxy that they operate.

## Explicit Non-Goals

The planned pairing design does not include:

- HTTPS managed by `ura`
- mTLS
- HMAC request signing
- PAKE
- OAuth
- JWT
- Matter
- Chromecast compatibility
- mandatory mDNS
- public internet exposure
