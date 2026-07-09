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

Run the full local verification gate with:

```bash
nix develop -c ./scripts/verify.sh
```

## Configuration

Client commands read:

```text
$XDG_CONFIG_HOME/ura/config.toml
~/.config/ura/config.toml
```

Example:

```toml
receiver_url = "http://127.0.0.1:8765"
token = "paste-a-random-token-of-at-least-32-chars"
```

CLI flags override config:

```bash
ura --receiver-url http://127.0.0.1:8765 --token "paste-a-random-token-of-at-least-32-chars" status
```

## Receiver

Start a local-only receiver:

```bash
ura receive --token "paste-a-random-token-of-at-least-32-chars"
```

Expose it on a LAN or Tailscale address only when you intend to:

```bash
ura receive --bind 100.x.y.z:8765 --token "paste-a-random-token-of-at-least-32-chars"
```

The default bind address is `127.0.0.1:8765`.
Receiver tokens must be at least 32 characters and cannot be empty or
`change-me`.

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

The extension requests `activeTab`, `storage`, `notifications`, and HTTP host
access so its background script can send requests to the configured receiver.
The token is sent only as `Authorization: Bearer ...`.

## Manual Smoke Tests

Use these checks before tagging or deploying the MVP. They are intentionally
manual because they verify local audio output, YouTube extraction through
`mpv`/`yt-dlp`, process shutdown, and Firefox temporary extension loading.

With `receiver_url` and a safe token configured, start the receiver in one
shell:

```bash
ura receive --bind 127.0.0.1:8765
```

From another shell, run:

```bash
ura play "https://youtu.be/<known-working-video-id>"
ura toggle
ura loop one
ura loop off
ura stop
ura history
```

Confirm audio plays without video, loop commands affect playback through the
receiver HTTP API, `history` returns the played URL, and Ctrl+C in the receiver
shell shuts down the HTTP server without leaving an `mpv` child process.

For Firefox:

1. Open `about:debugging`.
2. Go to "This Firefox".
3. Click "Load Temporary Add-on".
4. Select `extension/manifest.json`.
5. Configure the receiver URL and token in the extension options.
6. Open a YouTube video tab.
7. Click the extension button.
8. Confirm the receiver receives the request and audio playback starts.

Temporary Firefox add-ons are unloaded when Firefox exits, so repeat the load
step after restarting Firefox during development.

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
- Receiver tokens must be at least 32 characters; empty tokens and `change-me`
  are rejected.
- The default bind address is localhost only.
- LAN/Tailscale exposure requires explicit `--bind`.
- Only YouTube-style `http://` or `https://` URLs are accepted.
- Playlist expansion is not implemented.
- mpv IPC remains a local Unix socket.
