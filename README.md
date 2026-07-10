# ura

`ura` is a tiny audio caster for Linux and NixOS. Send a YouTube URL from the
CLI or the browser extension to a receiver machine, and the receiver plays it as
audio only through `mpv`.

`ura` is a personal receiver, not a Chromecast clone. It does not provide public
internet exposure, account login, phone apps, Spotify playback, DRM playback, or
cloud sync.

## Current Functionality

- run a local HTTP receiver with bearer-token authentication
- play or queue supported YouTube URLs
- control pause/resume, stop, and loop mode
- report basic `mpv` status
- store playback history in SQLite
- send the current tab from a minimal Firefox extension

Supported media URLs are currently allowlisted to:

- `https://www.youtube.com/watch?...`
- `https://music.youtube.com/watch?...`
- `https://youtu.be/...`

## Requirements

- Linux
- `mpv`
- `yt-dlp`
- SQLite

The Nix development shell includes the needed runtime tools.

## Quick Start

```bash
nix develop
cargo run -- config init
cargo run -- serve
```

In another terminal:

```bash
cargo run -- play "https://youtu.be/..."
cargo run -- status
```

Installed binary usage is the same without `cargo run --`:

```bash
ura config init
ura serve
ura play "https://youtu.be/..."
```

## CLI

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
ura device list
ura device add kamo 192.168.1.23 --token <token>
ura device select kamo
ura device remove kamo
ura device authorize desuwa
ura device revoke desuwa
ura config init
ura token generate
```

`ura loop` toggles current-track looping. `ura loop track` loops the current
track, `ura loop queue` loops the playback queue, and `ura loop off` disables
both.

Multiple receivers can be configured manually:

```bash
ura device authorize desuwa
ura device add kamo 192.168.1.23 --token <shown-token>
ura device select kamo
ura play --to kamo "https://youtu.be/..."
```

Pairing-code setup is planned but not implemented.

## Browser Extension

Load `extension/` temporarily in Firefox, open the extension options, and set:

- receiver URL
- token
- default action: `play` or `queue`

Click the toolbar button to send the current tab URL to the configured receiver.

## Security And Network

The receiver defaults to `127.0.0.1:8765`. Bind to a LAN or Tailscale address
only when you intend to expose the receiver to that network:

```bash
ura serve --bind 192.168.1.20:8765
```

The HTTP API requires `Authorization: Bearer <token>`. The `mpv` IPC socket is a
local Unix socket and is not exposed over TCP.

## More Documentation

- [Architecture](docs/architecture.md)
- [Planned pairing/authentication design](docs/pairing-auth.md)
- [Development and validation](docs/development.md)
- [Deployment and operation](docs/deployment.md)
