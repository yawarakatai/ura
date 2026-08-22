# Architecture

Status: Current `0.3.0` behavior.

`ura` is a small Linux audio node. Each machine can play audio itself and can
route local commands to a paired peer. There is no sender-only or receiver-only
role in the user-facing model.

```text
Firefox / ura CLI
        │
        ▼
┌──────────────────┐
│ local ura node   │
└────────┬─────────┘
         │ selected destination
    ┌────┴──────────────┐
    ▼                   ▼
 this device          paired peer
    │                   │
 local HTTP           peer HTTP
    │                   │
   mpv             ura node → mpv
```

The important routing invariant is:

> Only locally initiated commands resolve the selected destination. Requests
> received through the peer HTTP API always operate on that node's own playback
> backend and are never forwarded again.

This prevents routing loops and accidental multi-hop playback.

## Runtime Components

`ura daemon` supervises three control surfaces around one local playback backend:

```text
$XDG_RUNTIME_DIR/ura/node.sock
  local CLI routing and device selection

127.0.0.1:8766
  loopback-only browser control API

<configured bind>:8765
  authenticated peer HTTP API
```

The same process also starts and supervises `mpv`, observes structured mpv JSON
IPC events, records playback history in SQLite, and owns pairing state.

The default peer bind is `127.0.0.1:8765`. LAN or Tailscale exposure requires an
explicit `--bind`, `URA_BIND`, or `bind` setting. The browser control API is
always bound separately to `127.0.0.1:8766`; exposing the peer API never exposes
browser routing controls to the LAN.

## Destinations

The local node exposes two destination kinds:

```text
ThisDevice
Peer
```

This device is implicit and is never stored as a peer. Its display name comes
from `[local].name` when configured, otherwise the local hostname-derived
default is used.

Remote peers are currently stored as `[[devices]]` entries containing name,
address, and bearer token. Internally the selected destination resolves to
`Destination::SelfNode` or `Destination::Peer`.

When no remote device is selected, this device is the default destination.
Legacy flat `receiver_url` pointing at loopback is treated as this device rather
than being shown as a duplicate peer.

## CLI

The same binary provides node, playback, pairing, and device commands:

```bash
ura daemon
ura play "https://youtu.be/..."
ura queue "https://youtu.be/..."
ura pause
ura resume
ura toggle
ura stop
ura status
ura history
ura loop
ura loop off
ura loop track
ura loop queue
ura loop status
ura pair
ura pair 192.168.1.23
ura device list
ura device select
ura config init
ura token generate
```

Normal local commands first use the local node socket. The local node resolves
`--to <name>` when supplied, otherwise it resolves the persisted selected
device. `--to` is a one-shot override and does not change the stored selection.

`ura device select` without a name opens an interactive selector with arrow-key
navigation and text filtering. `ura device select <name>` remains suitable for
scripts.

For compatibility, a controller-only invocation with no running local node can
still talk directly to an explicitly configured remote peer. Selecting this
device without a running local node fails with an actionable error because local
playback requires the long-running node and mpv backend.

## Local Node Socket

Local CLI routing uses:

```text
$XDG_RUNTIME_DIR/ura/node.sock
```

The socket is created under the user's restrictive XDG runtime directory and is
mode `0600`. Requests are small JSON messages for play, queue, control, status,
history, device listing, and device selection.

The node socket is the authoritative local routing boundary. A request arriving
here may resolve the selected peer. A request arriving from the peer HTTP API
does not enter this routing boundary.

## Browser Control API

Firefox does not store or contact remote peers directly in `0.3.0`. It talks
only to:

```text
http://127.0.0.1:8766
```

Authenticated routes are:

```text
POST /v1/play
POST /v1/enqueue
POST /v1/control
GET  /v1/status
GET  /v1/history
GET  /v1/devices
POST /v1/select
```

These routes call the local node socket, so Firefox and the CLI share the same
selected destination.

The browser API authenticates only against active authorized-device token hashes
in SQLite. Firefox is authorized once through the existing six-digit pairing
flow and stores only its local-node credential plus extension preferences.
Remote peer addresses and peer credentials remain owned by ura.

For setup, the browser API also exposes local proxies for:

```text
GET  /v1/pair/info
POST /v1/pair/claim
```

They proxy the already-running node's pairing session; they do not create a new
pairing mechanism or expose a way to start pairing over HTTP.

The Firefox manifest host permission is restricted to localhost.

## Peer HTTP API

The peer API remains on the configured `ura daemon` bind address. It is the
network-facing playback API used by paired ura nodes.

Authenticated routes are:

```text
POST /v1/play
POST /v1/enqueue
POST /v1/control
GET  /v1/status
GET  /v1/history
```

Pairing routes are available without bearer authentication only while a pairing
session is active:

```text
GET  /v1/pair/info
POST /v1/pair/claim
```

Normal peer authentication first checks active authorized-device token hashes in
SQLite. Successful authorized-device authentication updates `last_seen_at` at
most once per 60 seconds. The legacy configured receiver token remains a
compatibility fallback.

A peer request always controls the receiving node's own mpv instance. It never
consults that node's selected destination.

## Pairing

`ura pair` without an address is local administrative behavior. It connects to:

```text
$XDG_RUNTIME_DIR/ura/control.sock
```

and starts one temporary pairing session in the running node. The six-digit code
is returned only through this local Unix socket and printed to the terminal.
There is no network endpoint that can start pairing.

`ura pair <ADDRESS>` claims a pairing session on another node and stores the
returned credential as a peer.

The current network pairing protocol is version 1 and remains asymmetric at the
credential level: the controlling node stores a plaintext bearer token for the
peer, while the receiving node stores only its hash. A future protocol can make
peer identity/trust symmetric without changing the routing invariant described
above.

## mpv JSON IPC

Each node starts `mpv` with audio-only options including:

```text
--idle=yes
--no-video
--force-window=no
--terminal=no
--input-ipc-server=<XDG_RUNTIME_DIR>/ura/mpv.sock
--ytdl-format=bestaudio/best
```

The IPC endpoint is a local Unix socket and is never exposed over TCP.
User-supplied URLs are encoded as JSON command arguments and are never passed
through a shell.

The receiver backend keeps a persistent JSON IPC observer connection and
observes structured properties including media title, duration, metadata, path,
pause, idle state, playlist position/count, and playback position.

On `file-loaded`, ura associates the loaded item with a pending play or queue
request and records the successful play. Later property changes can enrich the
same current-track metadata. Human-readable mpv logs are not parsed for runtime
state.

## SQLite

The default database is:

```text
$XDG_DATA_HOME/ura/ura.db
fallback: ~/.local/share/ura/ura.db
```

Playback history uses `tracks` and `plays` tables. Authorized local/peer clients
are stored in `authorized_devices` using SHA-256 token hashes, creation time,
coarse `last_seen_at`, and optional revocation time. Plaintext issued tokens are
not stored by the node that authorizes them.

## XDG Paths

```text
config:
  $XDG_CONFIG_HOME/ura/config.toml
  fallback: ~/.config/ura/config.toml

database:
  $XDG_DATA_HOME/ura/ura.db
  fallback: ~/.local/share/ura/ura.db

runtime:
  $XDG_RUNTIME_DIR/ura/mpv.sock
  $XDG_RUNTIME_DIR/ura/control.sock
  $XDG_RUNTIME_DIR/ura/node.sock
```

`XDG_RUNTIME_DIR` is required for local runtime sockets.

## Configuration

Example node configuration with one peer:

```toml
bind = "127.0.0.1:8765"
selected_device = "living-room"

[local]
name = "desktop"

[[devices]]
name = "living-room"
url = "http://192.168.1.23:8765"
token = "..."
```

Selecting this device removes the persisted remote `selected_device`; self is
implicit rather than serialized as another `[[devices]]` entry.

Device mutation preserves unrelated top-level node settings such as `bind` and
legacy receiver token fields.

Legacy flat `receiver_url` and `token` remain readable for compatibility.

## URL Validation

The playback backend accepts only supported YouTube video URLs using `http://`
or `https://`, including:

```text
https://www.youtube.com/watch?v=...
https://music.youtube.com/watch?v=...
https://youtu.be/...
```

Unsupported schemes, malformed authorities, credentials in authorities, unknown
hosts, and empty video IDs are rejected. Playlist context is stripped when a
valid video remains; standalone playlist expansion is not implemented.
