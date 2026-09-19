# Architecture

Status: Current `0.7.0` behavior.

`ura` is a small Linux audio node. Each machine can play audio itself and can
route local commands to a paired peer. Nodes do not have fixed sending or
receiving roles in the user-facing model.

```text
ura CLI
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

`ura daemon` supervises two control surfaces around one local playback backend:

```text
$XDG_RUNTIME_DIR/ura/node.sock
  local CLI routing and device selection

<configured bind>:8765
  authenticated peer HTTP API
```

The same process also starts and supervises `mpv`, observes structured mpv JSON
IPC events, records playback history in SQLite, and owns pairing state.

The default peer bind is `127.0.0.1:8765`. LAN or Tailscale exposure requires an
explicit `--bind`, `URA_BIND`, or `bind` setting.

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
Legacy flat `receiver_url` and top-level `token` fields are ignored; peers must
be represented by `[[devices]]` entries created through pairing or device
management.

## CLI

The same binary uses a URL-first CLI with a small command set:

```bash
ura                                # status
ura "https://youtu.be/..."         # play now
ura --loop "https://youtu.be/..."
ura pause
ura resume
ura stop
ura seek +30
ura seek -10
ura seek 12:30
ura history
ura history 3
ura history 3 --loop
ura device list
ura device select
ura pair
ura pair 192.168.1.23
ura daemon
```

A bare URL plays immediately, while `--loop` starts it with current-track
looping enabled. A normal play request disables an earlier track loop, so
looping does not leak into later playback.

Running `ura` without a URL or command shows status. `ura pause` is idempotent
and keeps the media loaded. `ura resume` resumes paused media immediately, or
reloads stopped media from its persisted checkpoint. `ura stop` preserves that
checkpoint before unloading the media. A naturally completed track clears its
checkpoint. Explicit pause/resume commands replace the former toggle operation.

`ura seek` accepts an absolute `SS`, `MM:SS`, or `HH:MM:SS` position. A leading
`+` or `-` makes it relative to the current position. Seeking requires an active
track and also works while paused.

`ura --help` is the only help entry point; an additional `help` subcommand is not
generated.

`ura history` shows individual play events newest first with one-based entry
numbers. `ura history <NUMBER>` submits the selected entry's original allowlisted
URL as a normal play command, while `--loop` starts it with current-track looping
enabled. Resuming a stopped track does not create another history event.

Playback, status, and history require the running local node and always use its
persisted selected device. The CLI has no one-shot destination override and no
direct-peer mode.

`ura device select` without a name opens an interactive selector with arrow-key
navigation and text filtering. `ura device select <name>` remains suitable for
scripts.

## Local Node Socket

Local CLI routing uses:

```text
$XDG_RUNTIME_DIR/ura/node.sock
```

The socket is created under the user's restrictive XDG runtime directory and is
mode `0600`. Requests are small JSON messages for play, seek, control, status,
history, device listing, and device selection.

The node socket is the authoritative local routing boundary. A request arriving
here may resolve the selected peer. A request arriving from the peer HTTP API
does not enter this routing boundary.

## Peer HTTP API

The peer API remains on the configured `ura daemon` bind address. It is the
network-facing playback API used by paired ura nodes.

Authenticated routes are:

```text
POST /v1/play
POST /v1/seek
POST /v1/control
GET  /v1/status
GET  /v1/history
```

`POST /v1/control` accepts the explicit `pause`, `resume`, and `stop` actions.
`POST /v1/seek` accepts finite `seconds` and `relative` fields. Resume and seek
terminate at the receiving node like all other peer playback operations.

Pairing routes are available without bearer authentication only while a pairing
session is active:

```text
GET  /v1/pair/info
POST /v1/pair/claim
```

Normal peer authentication first checks active authorized-client token hashes in
SQLite. Successful authorized-client authentication updates `last_seen_at` at
most once per 60 seconds. The legacy configured peer API token remains a
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
credential level: the initiating node stores a plaintext bearer token for the
peer, while the paired node stores only its hash. Pairing protocol v1 keeps the
wire field name `receiver_name` for compatibility; Rust internals call the
same value `node_name`.

A future protocol can make peer identity/trust symmetric without changing the
routing invariant described above.

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

The local playback backend keeps a persistent JSON IPC observer connection and
observes structured properties including media title, duration, metadata, path,
pause, idle state, and playback position.

On `file-loaded`, ura associates the loaded item with a pending play or resume
request. New plays are recorded in history; resume requests retain the existing
history event and seek after the file has loaded. The observer also reconciles
mpv's current structured-property snapshot when it connects, so a load completed
during observer startup or reconnection is not omitted. Later property changes
can enrich the same current-track metadata. Human-readable mpv logs are not
parsed for runtime state.

## SQLite

The default database is:

```text
$XDG_DATA_HOME/ura/ura.db
fallback: ~/.local/share/ura/ura.db
```

Playback history uses `tracks` and `plays` tables. History is returned as
individual play events in reverse chronological order, including repeated plays
of the same URL. CLI history numbers are derived from that order and are not
stored identifiers.

The singleton `resume_checkpoint` table stores the current source URL and latest
playback position. The observer updates it at five-second playback intervals and
file loads; pause and stop also save immediately. Manual stop and node shutdown
preserve it, while natural end clears it. Track display metadata is joined from
the existing `tracks` table.

Authorized local/peer clients are stored in the existing `authorized_devices`
table using SHA-256 token hashes, creation time, coarse `last_seen_at`, and
optional revocation time. The table name is retained as an on-disk schema
compatibility detail; the Rust API calls these records authorized clients.
Plaintext issued tokens are not stored by the node that authorizes them.

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
