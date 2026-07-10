# Architecture

Status: Current behavior.

`ura` is a small personal audio receiver. The runtime path is:

```text
browser extension / CLI
  -> HTTP API with bearer-token auth
  -> ura serve
  -> local mpv JSON IPC socket
  -> audio-only playback
```

The receiver starts and supervises `mpv`, exposes a small HTTP API, validates
incoming URLs, sends playback commands to `mpv`, observes `mpv` playback events,
and records history in SQLite.

## CLI

The same binary provides receiver and controller commands:

```bash
ura serve
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
ura config init
ura token generate
```

Controller commands resolve a destination from `--to <name>` or the configured
`selected_device`, then call the receiver HTTP API with that device's URL and
bearer token. Legacy `receiver_url` and `token` config is still read when no
`[[devices]]` entries exist. `URA_RECEIVER_URL`, `URA_TOKEN`,
`--receiver-url`, and `--token` remain compatibility overrides.

`ura pair` without an address is a receiver-side command. It connects to the
local control socket for the already-running `ura serve` process and starts a
temporary pairing session. `ura pair <ADDRESS>` is a controller-side command
that normalizes the receiver address, claims the pairing session, stores the
issued token through the same config path as `ura device add`, and can update
`selected_device`.

## Browser Extension

The Firefox extension in `extension/` reads the current tab URL and sends it to
the configured receiver. Extension settings are:

- receiver URL
- token
- default action: `play` or `queue`

The extension uses `Authorization: Bearer <token>` and sends either
`POST /v1/play` or `POST /v1/enqueue`.

## HTTP Receiver

`ura serve`:

1. validates the configured token
2. checks that `mpv` and `yt-dlp` are available in `PATH`
3. creates the runtime directory for the `mpv` IPC socket
4. opens the SQLite history database
5. starts `mpv` with audio-only options
6. starts the HTTP API on the configured bind address

The default bind address is `127.0.0.1:8765`. LAN or Tailscale exposure requires
an explicit `--bind`, `URA_BIND`, or `bind` setting.

## API

All API routes require:

```text
Authorization: Bearer <token>
```

Authentication first checks active authorized-device token hashes in SQLite.
Successful authorized-device authentication updates `last_seen_at` at most once
per 60 seconds. If no authorized-device token matches, the receiver checks the
legacy configured token as a compatibility fallback. Failed authentication does
not update `last_seen_at`.

Current endpoints:

```text
GET  /v1/pair/info
POST /v1/pair/claim
POST /v1/play
POST /v1/enqueue
POST /v1/control
GET  /v1/status
GET  /v1/history
```

`GET /v1/pair/info` and `POST /v1/pair/claim` are unauthenticated only while a
pairing session is active. They do not weaken authentication for the normal
playback, control, status, or history endpoints, which remain bearer-protected.

`POST /v1/play` replaces current playback. `POST /v1/enqueue` appends to the
mpv playlist and starts playback when appropriate. Both accept JSON with a
`url` field and optional `source` field.

`POST /v1/control` supports these command values:

```text
toggle
stop
pause
resume
loop-off
loop-one
loop-queue
loop-status
```

`GET /v1/status` returns the receiver's observed playback state with normalized
metadata and loop status. `GET /v1/history` returns stored track history ordered
by recent playback.

## mpv JSON IPC

The receiver starts `mpv` with:

- `--idle=yes`
- `--no-video`
- `--force-window=no`
- `--terminal=no`
- `--input-ipc-server=<XDG_RUNTIME_DIR>/ura/mpv.sock`
- `--ytdl-format=bestaudio/best`

The IPC endpoint is a local Unix socket. It is never exposed over TCP.

Playback uses `mpv` JSON IPC commands. User-supplied URLs are sent as JSON
command arguments and are not passed through a shell.

The receiver keeps a persistent JSON IPC observer connection open for structured
events and property changes. It observes:

```text
media-title
duration
metadata
path
pause
idle-active
playlist-pos
playlist-count
time-pos
```

Normal playback commands use request IDs so command responses are not confused
with asynchronous events. Unknown events and properties are ignored. Malformed
IPC messages are logged by the observer and do not stop the receiver when the
observer can keep reading.

On `file-loaded`, the receiver reads a coherent metadata snapshot from `mpv` and
associates the loaded item with a pending play or queue request. Later
`property-change` events can enrich the same current-track metadata. The
receiver does not parse human-readable `mpv` logs and does not run a second
blocking `yt-dlp` extraction for metadata.

Pending playback association falls back to queue order when mpv reports a
resolved media path that differs from the submitted URL. This assumes the mpv
playlist is not reordered outside ura.

Normalized metadata fields are:

```text
source_url
playback_path
title
artist
uploader
album
duration_seconds
thumbnail_url
```

Metadata keys from `mpv` are matched case-insensitively. Empty values do not
replace useful existing values. If `media-title` is only the source URL or
playback path, it is treated as a display fallback rather than authoritative
title metadata.

## SQLite History

Playback history is stored in SQLite. The current schema has `tracks` and
`plays` tables. `tracks` stores the original source URL, the URL passed to
`mpv`, optional metadata fields, timestamps, and play count. `plays` stores
individual play events and their optional source label. `authorized_devices`
stores receiver-side controller credentials using SHA-256 token hashes,
creation time, coarse `last_seen_at`, and optional revocation time; plaintext
tokens are not stored.

For current YouTube URL playback, `source_url` is the URL received from the
caller and `play_url` is the validated URL passed to `mpv`.

The receiver records a play when `mpv` emits `file-loaded`, not merely when the
HTTP request is accepted. This keeps failed loads out of successful playback
history. Metadata enrichment updates the existing `tracks` row for the canonical
source URL without rewriting the historical play time. Replaying the same URL
increments play count and can refresh missing metadata.

## Pairing Manager And Control Socket

`ura serve` owns a `PairingManager` shared by the local control socket and the
pairing HTTP endpoints. The manager tracks the current code, expiry, remaining
attempts, claim-in-progress state, and final completion state under one
synchronized owner.

The local administrative socket is:

```text
$XDG_RUNTIME_DIR/ura/control.sock
```

It is a Unix socket created under a restrictive runtime directory and removed on
receiver shutdown. It accepts local JSON commands:

```json
{"command":"pair_start"}
{"command":"pair_status"}
{"command":"pair_cancel"}
```

The pairing code is only returned over this local socket and printed by
`ura pair` on the receiver. There is no network endpoint that can start
pairing.

Successful HTTP pairing uses the same authorized-device database operation as
`ura device authorize <name>`. The controller stores the returned token through
the same remote-device config operation used by manual device addition.

## XDG Paths

Current default paths:

```text
config:
  $XDG_CONFIG_HOME/ura/config.toml
  fallback: ~/.config/ura/config.toml

database:
  $XDG_DATA_HOME/ura/ura.db
  fallback: ~/.local/share/ura/ura.db

runtime socket:
  $XDG_RUNTIME_DIR/ura/mpv.sock
  $XDG_RUNTIME_DIR/ura/control.sock
```

`XDG_RUNTIME_DIR` is required for the local runtime sockets.

## Configuration

The controller configuration can store multiple remote devices:

```toml
selected_device = "kamo"

[[devices]]
name = "kamo"
url = "http://192.168.1.23:8765"
token = "..."
```

The receiver still reads `bind` from `config.toml`:

```toml
bind = "127.0.0.1:8765"
```

Legacy flat `token` and `receiver_url` fields remain readable when no
`[[devices]]` entries exist.

Destination priority for controller commands is:

1. command-local `--to <name>`
2. `selected_device`
3. legacy flat `receiver_url` and `token`

`--receiver-url`, `--token`, `URA_RECEIVER_URL`, and `URA_TOKEN` remain
compatibility overrides.

Configuration priority for `ura serve` is:

1. CLI flags
2. environment variables: `URA_BIND`, `URA_TOKEN`
3. `config.toml`
4. default bind address when bind is omitted

Receiver tokens must be at least 32 characters and cannot be empty or
`change-me`.

## Loop Behavior

The CLI exposes:

```bash
ura loop
ura loop off
ura loop track
ura loop queue
ura loop status
```

`ura loop` reads current loop status and toggles current-track looping. The
receiver control API uses `loop-one` internally for current-track looping.

Current loop modes map to `mpv` properties:

```text
off:
  loop-file = no
  loop-playlist = no

track:
  loop-file = inf
  loop-playlist = no

queue:
  loop-file = no
  loop-playlist = inf
```

Custom `mpv` loop property combinations can be reported as custom status. A-B
looping and numeric loop counts are not implemented.

## URL Validation

The receiver accepts only `http://` and `https://` URLs and only supported
YouTube video URL forms:

```text
https://www.youtube.com/watch?v=...
https://music.youtube.com/watch?v=...
https://youtu.be/...
```

Playlist URLs and playlist context are rejected or stripped before playback,
depending on whether a valid video URL remains. Unsupported schemes, malformed
authorities, credentials in authorities, unknown hosts, and empty video IDs are
rejected with clear HTTP 400 errors.
