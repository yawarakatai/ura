# Deployment

Status: Current `0.5.0` behavior.

This document covers running `ura daemon` as the long-lived local ura node.
Tailscale or another private overlay can be used for peer traffic, but is not a
project requirement.

## Why The Node Is Long-Lived

Playback commands do not daemonize ura implicitly. The long-running node owns:

- the local mpv process and JSON IPC socket
- the local CLI routing socket
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
playing a URL with `ura <URL>` does not require manually running `ura daemon`.

The module installs ura and adds mpv and yt-dlp to the service PATH.
`services.ura.package`, `services.ura.bind`, and `services.ura.logLevel` can be
overridden.

## Local-Only Default

The default peer HTTP bind is:

```text
127.0.0.1:8765
```

This is sufficient for local playback. The CLI communicates with the local
node through its Unix socket, while peer nodes use the configured peer API.

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

## Credentials

Authorized clients are stored in the SQLite database under `XDG_DATA_HOME` or
`~/.local/share/ura/ura.db` as token hashes. Peer credentials are stored in
`[[devices]]` entries. Legacy top-level `token` and `receiver_url` fields are no
longer used.

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

If `ura <URL>` reports that the local node is not running, start the user
service:

```bash
systemctl --user start ura.service
```

During repository development, start it in another terminal with:

```bash
cargo run -- daemon
```

If ura reports that an mpv or ura runtime socket is already in use, stop the
existing node before starting another copy.

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
7. Confirm the mpv IPC socket remains Unix-local.
8. Confirm no plaintext credentials appear in normal logs or device listings.
