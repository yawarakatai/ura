# Architecture

Status: Current behavior.

`ura` is a small personal audio receiver. The runtime path is:

```text
browser extension / CLI
  -> HTTP API with bearer-token auth
  -> ura receive
  -> local mpv JSON IPC socket
  -> audio-only playback
```

The receiver starts and supervises `mpv`, exposes a small HTTP API, validates
incoming URLs, sends playback commands to `mpv`, and records history in SQLite.

## CLI

The same binary provides receiver and controller commands:

```bash
ura receive
ura play "https://youtu.be/..."
ura enqueue "https://youtu.be/..."
ura toggle
ura stop
ura status
ura history
ura loop
ura loop off
ura loop track
ura loop queue
ura loop status
ura config init
ura token generate
```

Controller commands read the receiver URL and token from configuration,
environment variables, or CLI flags, then call the receiver HTTP API.

## Browser Extension

The Firefox extension in `extension/` reads the current tab URL and sends it to
the configured receiver. Extension settings are:

- receiver URL
- token
- default action: `play` or `enqueue`

The extension uses `Authorization: Bearer <token>` and sends either
`POST /v1/play` or `POST /v1/enqueue`.

## HTTP Receiver

`ura receive`:

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

Current endpoints:

```text
POST /v1/play
POST /v1/enqueue
POST /v1/control
GET  /v1/status
GET  /v1/history
```

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

`GET /v1/status` returns basic `mpv` status fields. `GET /v1/history` returns
stored track history ordered by recent playback.

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

## SQLite History

Playback history is stored in SQLite. The current schema has `tracks` and
`plays` tables. `tracks` stores the original source URL, the URL passed to
`mpv`, optional metadata fields, timestamps, and play count. `plays` stores
individual play events and their optional source label.

For current YouTube URL playback, `source_url` is the URL received from the
caller and `play_url` is the validated URL passed to `mpv`.

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
```

`XDG_RUNTIME_DIR` is required for the `mpv` IPC socket.

## Configuration

The current configuration file is `config.toml` with these fields:

```toml
token = "a-random-token-of-at-least-32-characters"
receiver_url = "http://127.0.0.1:8765"
bind = "127.0.0.1:8765"
```

`ura config init` creates the file, generates a token, writes the default
receiver URL and bind address, and prints the created path without printing the
full token.

Configuration priority for controller commands is:

1. CLI flags
2. environment variables: `URA_RECEIVER_URL`, `URA_TOKEN`
3. `config.toml`

Configuration priority for `ura receive` is:

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
