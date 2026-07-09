# ura

`ura` is a tiny LAN/Tailscale audio caster for Linux and NixOS.

It lets you send a YouTube URL from a CLI or browser extension to a receiver
machine. The receiver plays audio only through `mpv`.

`ura` is not Chromecast-compatible. It does not support Spotify, DRM playback,
ad blocking, public internet exposure, phone apps, account login, or cloud sync.

## Architecture

```text
browser extension / CLI
  -> HTTP API with bearer token auth
  -> ura receive
  -> local mpv JSON IPC socket
  -> audio-only playback
```

The mpv IPC socket is local only and is never exposed over TCP.

## Development

Use the Nix dev shell:

```bash
nix develop
cargo build
cargo test
```

The dev shell includes Rust, `mpv`, `yt-dlp`, and `sqlite`.

## Configuration

Client commands read:

```text
$XDG_CONFIG_HOME/ura/config.toml
~/.config/ura/config.toml
```

Example:

```toml
receiver_url = "http://127.0.0.1:8765"
token = "change-me"
```

CLI flags override config:

```bash
ura --receiver-url http://127.0.0.1:8765 --token change-me status
```

## Receiver

Start a local-only receiver:

```bash
ura receive --token "change-me"
```

Expose it on a LAN or Tailscale address only when you intend to:

```bash
ura receive --bind 100.x.y.z:8765 --token "change-me"
```

The default bind address is `127.0.0.1:8765`.

## CLI

```bash
ura play "https://youtu.be/..."
ura enqueue "https://youtu.be/..."
ura toggle
ura stop
ura status
ura history
```

Loop controls:

```bash
ura loop off
ura loop one
ura loop queue
ura loop status
```

## Browser Extension

Load `extension/` temporarily in Firefox during development.

Open the extension options and set:

- receiver URL
- token
- default action: `play` or `enqueue`

Click the toolbar button to send the current tab URL to the receiver.

## Data

`ura` stores playback history in SQLite at:

```text
$XDG_DATA_HOME/ura/ura.db
~/.local/share/ura/ura.db
```

For the MVP, metadata such as title, uploader, duration, and thumbnail may be
empty. YouTube extraction is left to `mpv` and `yt-dlp`.

## Security Notes

- The HTTP API requires `Authorization: Bearer <token>`.
- The default bind address is localhost only.
- LAN/Tailscale exposure requires explicit `--bind`.
- Only YouTube-style `http://` or `https://` URLs are accepted.
- Playlist expansion is not implemented.
- mpv IPC remains a local Unix socket.
