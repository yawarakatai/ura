# ura

Send a YouTube URL to another Linux machine and play it as audio.

`ura` runs a small receiver on the machine connected to your speakers. Control
it from another device using the CLI or the Firefox extension.

```text
CLI / Firefox extension
        ↓ HTTP
    ura receiver
        ↓
  mpv + yt-dlp
        ↓
   audio output
```

## Features

- Play or queue YouTube URLs on another Linux device
- Pair devices using a six-digit code shown on the receiver
- Control playback from the CLI or Firefox
- Switch between multiple receivers
- Pause, resume, stop, and configure loop behavior
- View current playback status and history
- Run the receiver as a systemd user service

Supported URLs currently include:

- `youtube.com/watch`
- `music.youtube.com/watch`
- `youtu.be`

## Quick start

Start the receiver on the machine connected to your speakers:

```bash
ura serve --bind 0.0.0.0:8765
```

In another terminal on the receiver, open pairing:

```bash
ura pair
```

The command displays an address and a six-digit code.

On the controlling device:

```bash
ura pair 192.168.1.23
ura play "https://youtu.be/..."
```

The controlling device does not need to run a background service.

## Firefox extension

Load the extension from `extension/`, then open its options page.

On the receiver:

```bash
ura pair
```

Enter the displayed address and code in the extension. Once paired, open a
YouTube tab and click the toolbar button to send it to the selected receiver.

The extension supports multiple receivers and `play` or `queue` as the default
action.

## Commands

Playback:

```bash
ura play <url>
ura queue <url>
ura pause
ura resume
ura toggle
ura stop
ura status
ura history
```

Looping:

```bash
ura loop
ura loop off
ura loop track
ura loop queue
ura loop status
```

Devices:

```bash
ura pair [address]
ura device list
ura device select <name>
ura device remove <name>
ura device revoke <name>
```

Use a specific receiver without changing the selected device:

```bash
ura play --to kamo "https://youtu.be/..."
```

Run `ura --help` or `ura <command> --help` for the complete CLI reference.

## Requirements

A receiver needs:

- Linux
- `mpv`
- `yt-dlp`

A CLI controller only needs the `ura` binary. A Firefox-only controller does
not need the CLI installed.

## Network and security

`ura` uses plain HTTP with per-device bearer tokens. Pairing must be initiated
from the receiver and is confirmed with a short-lived code shown on its screen.

The protocol is intended for private networks. Do not expose the receiver
directly to the public internet.

The `mpv` IPC socket remains local to the receiver and is never exposed over
TCP.

## Development

Enter the development environment and run the complete verification gate:

```bash
nix develop
./scripts/verify.sh
```

## Documentation

- [Architecture](docs/architecture.md)
- [Pairing and authentication](docs/pairing-auth.md)
- [Development and validation](docs/development.md)
- [Deployment and operation](docs/deployment.md)
