# Deployment

Status: Current `0.3.0` behavior.

This document covers running `ura daemon` as the long-lived local ura node.
Tailscale or another private overlay can be used for peer traffic, but is not a
project requirement.

## Why The Node Is Long-Lived

Playback commands do not daemonize ura implicitly. The long-running node owns:

- the local mpv process and JSON IPC socket
- the local CLI routing socket
- the loopback-only Firefox control API on `127.0.0.1:8766`
- the optional network-facing peer API
- pairing state and playback history

For normal desktop use, run it as a systemd user service rather than starting
`ura daemon` manually for every playback command.

## systemd User Service

`contrib/systemd/` contains examples:

- `contrib/systemd/ura.service`
- `contrib/systemd/ura.env.example`

Install them into user configuration:

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
```

Follow logs with:

```bash
journalctl --user -u ura.service -f
```

The service runs:

```ini
ExecStart=ura daemon
```

The optional `~/.config/ura/ura.env` file is for process environment such as
`RUST_LOG`. Node bind/token settings are normally read from `config.toml`.

## PATH And Runtime Dependencies

The user service must be able to find:

- `ura`
- `mpv`
- `yt-dlp`

`ura daemon` validates its runtime dependencies at startup and mpv uses `yt-dlp`
for supported media URLs.

## NixOS And Home Manager

The flake exports a Home Manager module:

```nix
{
  imports = [ inputs.ura.homeManagerModules.default ];

  services.ura = {
    enable = true;
  };
}
```

With the module enabled, the local ura node starts as a systemd user service, so
normal commands such as `ura play` do not require manually running `ura daemon`.

The module installs ura and adds mpv and yt-dlp to the service PATH.
`services.ura.package`, `services.ura.bind`, and `services.ura.logLevel` can be
overridden.

## Local-Only Default

The default peer HTTP bind is:

```text
127.0.0.1:8765
```

This is sufficient for local playback and Firefox use. Firefox talks to the
separate browser control listener:

```text
127.0.0.1:8766
```

The browser listener is not configurable in `0.3.0` and remains loopback-only.
It is never widened when the peer API is exposed.

## Exposing A Peer

To let another ura node pair/control this machine, explicitly expose the peer
API on an appropriate trusted interface:

```bash
ura daemon --bind 192.168.1.20:8765
```

or configure:

```toml
bind = "192.168.1.20:8765"
```

Then restart the user service:

```bash
systemctl --user restart ura.service
```

A Tailscale address can be used the same way:

```toml
bind = "100.x.y.z:8765"
```

The selected address/port must be reachable through the local firewall from the
peer that will connect. Do not expose ura's peer API directly to the public
internet.

Using `0.0.0.0:8765` is convenient for local development but exposes the peer API
on every IPv4 interface. Prefer a specific LAN/private-overlay address for a
persistent deployment when practical.

## Pairing Peers

The node must already be running before pairing:

```bash
ura pair
```

This command talks to the local administrative Unix socket at:

```text
$XDG_RUNTIME_DIR/ura/control.sock
```

It starts a temporary pairing session without restarting the node.

From another ura node:

```bash
ura pair 192.168.1.20
```

The default peer port `8765` can be omitted. A custom peer port must be included
in the address.

After pairing, select the playback destination with:

```bash
ura device select
```

## Firefox Authorization

Firefox is authorized against its local ura node, not against every remote peer.

1. Ensure the local node/user service is running.
2. Run `ura pair` on the same machine.
3. Enter the six-digit code in the extension options.

The extension then accesses only `127.0.0.1:8766`. Remote peer credentials stay
inside ura configuration and are never copied into new extension state.

Revoking Firefox's authorized-device credential causes the extension to request
pairing again.

## Compatibility Credentials

Authorized devices are stored in the SQLite database under `XDG_DATA_HOME` or
`~/.local/share/ura/ura.db` as token hashes.

The legacy top-level receiver token and `receiver_url` settings remain readable
for compatibility:

```toml
token = "..."
receiver_url = "http://192.168.1.20:8765"
```

The legacy receiver token is accepted only by the peer HTTP compatibility path;
the loopback browser API authenticates against authorized-device credentials.

## Troubleshooting

Useful service commands:

```bash
systemctl --user status ura.service
journalctl --user -u ura.service -e
journalctl --user -u ura.service -f
systemctl --user restart ura.service
systemctl --user stop ura.service
```

Check runtime tools:

```bash
which ura
which mpv
which yt-dlp
```

If `ura play` reports that this device is selected but the local node is not
running, start the user service:

```bash
systemctl --user start ura.service
```

During repository development, start it in another terminal with:

```bash
cargo run -- serve
```

If ura reports that an mpv or ura runtime socket is already in use, stop the
existing node before starting another copy.

If Firefox reports the local node as unreachable, verify that the node is
running and that `127.0.0.1:8766` is not occupied by another process.

## Deployment Verification Checklist

1. Start/enable the local node service.
2. Confirm `ura device list` includes this device and reports the expected
   selection.
3. Play a short supported URL on this device.
4. If remote control is needed, bind the peer API to the intended trusted
   interface and verify firewall reachability.
5. Pair a second ura node and switch between self/peer with `ura device select`.
6. Verify a peer request terminates at the receiving node even if that node has
   another selected destination.
7. Pair Firefox locally and confirm CLI/extension selection is shared.
8. Confirm the browser API is reachable only through loopback and the mpv IPC
   socket remains Unix-local.
9. Confirm no plaintext credentials appear in normal logs or device listings.
