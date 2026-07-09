# AGENTS.md

## Project

This repository is for `ura`, a tiny LAN/Tailscale audio caster for NixOS/Linux.

`ura` lets a user send a YouTube URL from one device, such as a browser extension or CLI, to another Linux device. The receiving device plays the URL as audio only using `mpv`.

The project should stay small, boring, and useful.

## Core concept

`ura` is not a Chromecast clone.

It is a small personal audio receiver:

```text
browser extension / CLI
  -> HTTP API
  -> ura receive
  -> mpv JSON IPC
  -> audio-only playback
```

The receiver runs on the machine connected to speakers or headphones.

## Naming

Use:

```bash
ura receive
ura play <url>
ura enqueue <url>
ura toggle
ura stop
ura status
ura history
```

Prefer `receive` over `recv`.

## Target platform

Primary target:

- Linux
- NixOS
- local network or Tailscale network
- desktop/server-style usage

Do not add Windows or macOS support unless explicitly requested later.

## Implementation language

Use Rust.

Prefer simple, maintainable Rust over clever abstractions.

Recommended crates:

- `clap` for CLI parsing
- `tokio` for async runtime
- `axum` for the HTTP API
- `serde` / `serde_json` for JSON
- `rusqlite` for SQLite
- `anyhow` for application errors
- `directories` or `dirs` for XDG paths

Avoid adding large frameworks unless there is a clear reason.

## Runtime dependencies

The receiver may depend on external programs:

- `mpv`
- `yt-dlp`

Do not reimplement YouTube extraction. Let `mpv` and `yt-dlp` handle URL playback.

## Data locations

Follow XDG paths.

Suggested defaults:

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

## Receiver behavior

`ura receive` should:

1. start or connect to an `mpv` instance
2. expose a small HTTP API
3. require token authentication
4. accept YouTube URLs
5. play audio only
6. save playback history to SQLite
7. keep mpv IPC local only

The mpv IPC socket must not be exposed directly over the network.

## HTTP API

Keep the API small.

Required endpoints:

```text
POST /v1/play
POST /v1/enqueue
POST /v1/control
GET  /v1/status
GET  /v1/history
```

Authentication:

```text
Authorization: Bearer <token>
```

Example play request:

```json
{
  "url": "https://www.youtube.com/watch?v=...",
  "source": "browser-extension"
}
```

Example control request:

```json
{
  "command": "toggle"
}
```

Supported control commands for the MVP:

```text
toggle
stop
pause
resume
```

## CLI behavior

The same binary should provide both receiver and client commands.

The client commands should call the configured receiver HTTP API.

Example:

```bash
ura play "https://youtu.be/..."
ura enqueue "https://youtu.be/..."
ura toggle
ura stop
ura status
ura history
```

## Loop behavior

Looping is a first-class feature.

Support these loop modes:

```text
off
one
queue
```

Meaning:

```text
off:
  disable both current-track and queue looping

one:
  loop the current track forever

queue:
  loop the mpv playlist / queue forever
```

Use mpv properties:

```text
loop-file
loop-playlist
```

Mapping:

```text
off:
  loop-file = no
  loop-playlist = no

one:
  loop-file = inf
  loop-playlist = no

queue:
  loop-file = no
  loop-playlist = inf
```

CLI commands:

```bash
ura loop off
ura loop one
ura loop queue
ura loop status
```

HTTP control commands should support:

```text
loop-off
loop-one
loop-queue
loop-status
```

Do not implement A-B loop in the MVP.

Do not implement numeric loop counts in the MVP.

The CLI should read receiver URL and token from config.

## Browser extension

The browser extension should be minimal.

MVP behavior:

- user clicks toolbar button
- extension reads the current tab URL
- extension sends it to `POST /v1/play`
- extension shows success or failure

Required settings:

- receiver URL
- token
- default action: `play` or `enqueue`

Do not build a complex browser UI initially.

## Database

Use SQLite.

MVP tables:

```sql
tracks
plays
```

Suggested schema:

```sql
CREATE TABLE IF NOT EXISTS tracks (
  id INTEGER PRIMARY KEY,
  source_kind TEXT NOT NULL,
  source_url TEXT NOT NULL UNIQUE,
  play_url TEXT NOT NULL,
  title TEXT,
  uploader TEXT,
  duration INTEGER,
  thumbnail_url TEXT,
  created_at TEXT NOT NULL,
  last_played_at TEXT,
  play_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS plays (
  id INTEGER PRIMARY KEY,
  track_id INTEGER NOT NULL,
  played_at TEXT NOT NULL,
  source TEXT,
  FOREIGN KEY(track_id) REFERENCES tracks(id)
);
```

For YouTube URLs in the MVP:

```text
source_url = original URL received from user
play_url   = same URL passed to mpv
```

Keep `source_url` and `play_url` separate so future Spotify-to-YouTube resolution can be added later.

## URL support

MVP should support:

- `https://www.youtube.com/watch?...`
- `https://youtu.be/...`
- `https://music.youtube.com/watch?...`

For now, reject unknown URLs with a clear error.

Do not support Spotify in the MVP.

## Security

Security matters because the receiver exposes a network API.

Rules:

1. require bearer token auth
2. default bind address should be `127.0.0.1:8765`
3. LAN/Tailscale exposure must require explicit `--bind`
4. never expose mpv IPC directly over TCP
5. reject unsupported URL schemes
6. do not execute shell commands with unsanitized user input
7. prefer direct process arguments over shell strings

Accept only `http://` and `https://` URLs in the MVP.

## Non-goals

Do not implement these unless explicitly requested later:

- Chromecast compatibility
- Spotify direct playback
- Spotify Premium / Connect integration
- DRM bypass
- ad blocking or ad bypass features
- YouTube download/ripping features
- video playback UI
- web dashboard
- phone app
- mDNS discovery
- multi-user permissions
- remote public internet exposure
- playlist import/export
- YouTube playlist expansion
- music recommendation engine
- account login
- cloud sync
- Windows/macOS support

## Style

Keep code simple.

Prefer:

- small modules
- explicit error messages
- easy local testing
- boring data formats
- minimal global state

Avoid:

- over-engineering
- background magic
- unnecessary async complexity
- large UI frameworks
- hidden network behavior

## Verification

Each implementation task should include a numbered verification checklist.

At minimum, verify:

1. `cargo fmt`
2. `cargo clippy`
3. `cargo test`
4. manual command examples
5. no unsupported feature was accidentally added
