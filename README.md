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

## Running as a systemd user service

`contrib/systemd/` contains example files for running `ura receive` as a
long-lived systemd user service. This is not a system-level service and does not
install a NixOS or Home Manager module.

Install the examples into your user config:

```bash
mkdir -p ~/.config/systemd/user
mkdir -p ~/.config/ura

cp contrib/systemd/ura.service ~/.config/systemd/user/ura.service
cp contrib/systemd/ura.env.example ~/.config/ura/ura.env

$EDITOR ~/.config/ura/ura.env

systemctl --user daemon-reload
systemctl --user enable --now ura.service
systemctl --user status ura.service
journalctl --user -u ura.service -f
```

Generate a private token before editing the env file:

```bash
openssl rand -base64 32
```

The example service runs:

```ini
ExecStart=ura receive
```

If `ura` is not in the PATH seen by systemd user services, install it into your
profile or replace `ExecStart` with the absolute path to the binary. `mpv` and
`yt-dlp` must also be available in the service environment.

The default receiver bind address remains `127.0.0.1:8765`. To bind explicitly
to a Tailscale or LAN address, edit the copied service and override
`ExecStart`, for example:

```ini
ExecStart=ura receive --bind 100.x.y.z:8765
```

Then reload and restart:

```bash
systemctl --user daemon-reload
systemctl --user restart ura.service
```

Useful service commands:

```bash
systemctl --user restart ura.service
systemctl --user stop ura.service
```

Troubleshooting:

```bash
journalctl --user -u ura.service -e
which ura
which mpv
which yt-dlp
playerctl -l
pgrep -a mpv
```

For Home Manager users, a conceptual service snippet looks like this. Replace
`ExecStart` with the actual path to your `ura` binary if `pkgs.ura` is not
available in your configuration.

```nix
systemd.user.services.ura = {
  Unit = {
    Description = "ura audio receiver";
    After = [ "default.target" ];
  };

  Service = {
    Type = "simple";
    Environment = [
      "URA_TOKEN=replace-with-a-random-token-at-least-32-chars"
      "RUST_LOG=ura=info"
    ];
    ExecStart = "${pkgs.ura}/bin/ura receive";
    Restart = "on-failure";
    RestartSec = 2;
  };

  Install = {
    WantedBy = [ "default.target" ];
  };
};
```

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
`mpv`/`yt-dlp`, request logging, process shutdown, and Firefox temporary
extension loading.

For a local real-device smoke test, use one shell for the receiver:

```bash
export URA_TOKEN="$(openssl rand -base64 32)"
export URA_RECEIVER_URL="http://127.0.0.1:8765"

RUST_LOG=ura=info cargo run -- receive
```

From another shell, run:

```bash
cargo run -- play 'https://www.youtube.com/watch?v=ynsLjv1AyEg'
cargo run -- status
cargo run -- toggle
cargo run -- loop one
cargo run -- loop off
cargo run -- stop
cargo run -- history
```

Confirm startup logs include the bind address, mpv IPC socket path, and mpv
child pid. Play, status, control, loop, stop, and history requests should
produce useful receiver logs without printing bearer tokens or raw
`Authorization` headers. Confirm audio plays without video, loop commands
affect playback through the receiver HTTP API, `history` returns the played URL,
and Ctrl+C in the receiver shell shuts down the HTTP server without leaving an
`mpv` child process.

Quote URLs containing `&` in shells such as zsh, otherwise the shell treats
parts of the URL as background commands.

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

Future improvements may include metadata extraction for richer history and
optional MPRIS integration through `mpv-mpris`.

## Security Notes

- The HTTP API requires `Authorization: Bearer <token>`.
- Keep the receiver token private.
- Generate a strong token with `openssl rand -base64 32`.
- Receiver tokens must be at least 32 characters; empty tokens and `change-me`
  are rejected.
- The default bind address is localhost only.
- The default localhost bind is safest.
- LAN/Tailscale exposure requires explicit `--bind`.
- Bind to a Tailscale address explicitly when controlling playback from another
  device.
- Do not expose the receiver directly to the public internet.
- HTTP bearer tokens are not encrypted on plain LAN HTTP.
- Only YouTube-style `http://` or `https://` URLs are accepted.
- Playlist expansion is not implemented.
- mpv IPC remains a local Unix socket.
