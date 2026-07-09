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

`~/.config/ura/config.toml` is the main ura configuration file for both the
receiver and client commands.

```text
$XDG_CONFIG_HOME/ura/config.toml
~/.config/ura/config.toml
```

Example:

```toml
token = "paste-a-random-token-of-at-least-32-chars"
receiver_url = "http://127.0.0.1:8765"
bind = "127.0.0.1:8765"
```

Create a local config with:

```bash
ura config init
```

This writes `~/.config/ura/config.toml`, generates a random token, and prints
the created path without printing the full token. Use `--force` to replace an
existing config, `--receiver-url` to write a non-default receiver URL, `--bind`
to write the receiver bind address, and `--token` when you want to reuse an
existing token.

For Tailscale use from another device, initialize with the Tailscale address:

```bash
ura config init --receiver-url http://100.x.y.z:8765 --bind 100.x.y.z:8765
```

Generate a token without writing files:

```bash
ura token generate
```

Priority order is CLI flags, then environment variables, then `config.toml`,
then safe defaults where available. CLI flags still work:

```bash
ura --receiver-url http://127.0.0.1:8765 --token "paste-a-random-token-of-at-least-32-chars" status
ura receive --bind 127.0.0.1:8765
ura --config ~/.config/ura/config.toml status
ura --config ~/.config/ura/config.toml receive
```

## Receiver

Start a local-only receiver:

```bash
ura receive
```

Expose it on a LAN or Tailscale address only when you intend to:

```bash
ura receive --bind 100.x.y.z:8765
```

`ura receive` reads `token` and `bind` from `config.toml`. The default bind
address is `127.0.0.1:8765` when config omits `bind`. Receiver tokens must be
at least 32 characters and cannot be empty or `change-me`.

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

ura config init
$EDITOR ~/.config/ura/config.toml

systemctl --user daemon-reload
systemctl --user enable --now ura.service
systemctl --user status ura.service
journalctl --user -u ura.service -f
```

For an existing service, update `config.toml`, then restart:

```bash
ura config init --force
systemctl --user restart ura.service
ura status
```

The systemd service reads token and bind settings from
`~/.config/ura/config.toml`. The optional `~/.config/ura/ura.env` file is only
for process environment such as `RUST_LOG`.

If you want to load `~/.config/ura/ura.env` into an interactive shell, use
`set -a` so the assignments are exported to child processes:

```bash
set -a
source ~/.config/ura/ura.env
set +a
```

The example service runs:

```ini
ExecStart=ura receive
```

If `ura` is not in the PATH seen by systemd user services, install it into your
profile or replace `ExecStart` with the absolute path to the binary. `mpv` and
`yt-dlp` must also be available in the service environment.

The default receiver bind address remains `127.0.0.1:8765`. To bind explicitly
to a Tailscale or LAN address, set `bind` in `config.toml`, then reload and
restart:

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
ura loop
ura loop off
ura loop track
ura loop queue
ura loop status
```

`ura loop` toggles current-track looping on or off. `ura loop track` loops the
current track forever. `ura loop queue` loops the mpv playlist queue. `ura loop
off` disables both current-track and queue looping.

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
ura config init --force
RUST_LOG=ura=info cargo run -- receive
```

From another shell, run:

```bash
cargo run -- play 'https://www.youtube.com/watch?v=ynsLjv1AyEg'
cargo run -- status
cargo run -- toggle
cargo run -- loop track
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
- Generate a strong token with `ura token generate` or `openssl rand -base64 32`.
- Receiver and client commands must use the same `token` from `config.toml`.
- Receiver tokens must be at least 32 characters; empty tokens and `change-me`
  are rejected.
- The default bind address is localhost only.
- The default localhost bind is safest.
- LAN/Tailscale exposure requires explicit `--bind`.
- Bind to a Tailscale address explicitly when controlling playback from another
  device.
- Tailscale is recommended for cross-device use.
- Do not expose the receiver directly to the public internet.
- HTTP bearer tokens are not encrypted on plain LAN HTTP.
- Only YouTube-style `http://` or `https://` URLs are accepted.
- Playlist expansion is not implemented.
- mpv IPC remains a local Unix socket.
