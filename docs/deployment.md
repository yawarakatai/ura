# Deployment

Status: Current behavior.

This document covers installing and running `ura serve` as a long-lived local
receiver. Tailscale can be used as an optional private network, but it is not a
project requirement.

## systemd User Service

`contrib/systemd/` contains example files for running `ura serve` as a systemd
user service:

- `contrib/systemd/ura.service`
- `contrib/systemd/ura.env.example`

Install the examples into user configuration:

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
ExecStart=ura serve
```

The optional `~/.config/ura/ura.env` file is for process environment such as
`RUST_LOG`; token and bind settings are read from `config.toml` unless supplied
another supported way.

If you want to source `ura.env` in an interactive shell, export the assignments:

```bash
set -a
source ~/.config/ura/ura.env
set +a
```

## PATH And Runtime Dependencies

The systemd user service must be able to find:

- `ura`
- `mpv`
- `yt-dlp`

If `ura` is not in the PATH seen by systemd user services, install it into your
profile or change `ExecStart` to the absolute path of the binary. `mpv` and
`yt-dlp` must also be available in the service environment because `ura serve`
checks them at startup and `mpv` uses `yt-dlp` for supported media URLs.

## NixOS And Home Manager

The flake exports a Home Manager module. Import it and enable the receiver with:

```nix
{
  imports = [ inputs.ura.homeManagerModules.default ];

  services.ura = {
    enable = true;
    bind = "0.0.0.0:8765";
  };
}
```

The module installs `ura` and adds `mpv` and `yt-dlp` to the service `PATH`.
`services.ura.package`, `services.ura.bind`, and `services.ura.logLevel` can be
overridden when needed.

## LAN Binding

The default receiver bind address is local-only:

```text
127.0.0.1:8765
```

To expose the receiver to another device on a LAN or private overlay network,
set an explicit bind address:

```bash
ura serve --bind 192.168.1.20:8765
```

For a systemd service, set `bind` in `~/.config/ura/config.toml`:

```toml
bind = "192.168.1.20:8765"
```

Then restart:

```bash
systemctl --user restart ura.service
```

Use a Tailscale address the same way when you intentionally want access over
your tailnet:

```toml
bind = "100.x.y.z:8765"
```

For another device to pair or control playback, the receiver HTTP port must be
reachable through the local firewall on the chosen LAN or private overlay
interface.

## Device Authorization

Receiver-side authorized devices live in the SQLite database under
`XDG_DATA_HOME` or `~/.local/share/ura/ura.db`. Once a controller has been
authorized with `ura device authorize <name>` and added on the controller with
`ura device add <name> <address> --token <token>`, the receiver configuration no
longer needs one shared controller token for that device.

The legacy `token` and `receiver_url` settings remain compatibility behavior:

```toml
token = "..."
receiver_url = "http://192.168.1.20:8765"
```

The legacy token is still accepted after authorized-device lookup fails. Keep it
only while older CLI or browser-extension settings still need it.

## Pairing

`ura serve` must already be running before local receiver-side pairing:

```bash
ura pair
```

The command talks to the running receiver through:

```text
$XDG_RUNTIME_DIR/ura/control.sock
```

The socket is local-only, created under `$XDG_RUNTIME_DIR/ura` with restrictive
directory permissions, and removed when the receiver shuts down. Pairing does
not require restarting `ura serve`.

On the controller, use the address printed by the receiver:

```bash
ura pair 192.168.1.20
```

The default port `8765` can be omitted. If the receiver uses a custom port, keep
it in the address.

## Troubleshooting With journalctl

Useful commands:

```bash
systemctl --user status ura.service
journalctl --user -u ura.service -e
journalctl --user -u ura.service -f
systemctl --user restart ura.service
systemctl --user stop ura.service
```

Check PATH-related failures from the same kind of environment that starts the
service:

```bash
which ura
which mpv
which yt-dlp
```

If the receiver reports that the `mpv` IPC socket is already in use, stop the
existing receiver before starting another one.

## External-Device Smoke Test

1. On the receiver machine, choose an explicit LAN or private overlay bind
   address.
2. Set `bind` in `~/.config/ura/config.toml`.
3. Start or restart `ura serve`.
4. Run `ura pair` on the receiver and note the shown code.
5. From the controller device, run `ura pair <address>`.
6. Confirm the new receiver is selected, or run `ura device select <name>`.
7. Run `ura status` or use the browser extension with its existing URL/token settings.
8. Send a short supported YouTube URL and verify that audio plays on the
   receiver machine.

Keep the receiver off public interfaces unless you are deliberately exposing it
inside a trusted private network.
