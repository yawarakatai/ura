# ura

> **Play it here. Play it there.**

`ura` lets you choose which of your Linux devices should play a YouTube or YouTube Music URL.
Every machine running `ura` is a node: it can play audio itself and it can send playback to a paired peer.

```text
Firefox / ura CLI
        │
        ▼
┌──────────────────┐
│ local ura node   │
└────────┬─────────┘
         │ selected device
    ┌────┴──────────────┐
    ▼                   ▼
 this device          paired peer
    │                   │
   mpv             ura node → mpv
```

Nodes do not have fixed roles. This device is always available as a playback destination; remote devices become available after pairing.

No web dashboard. No account. No streaming service integration.
Just choose a device and play something.

## Why ura?

Maybe your desktop has the good speakers.
Maybe an old laptop sits under your monitor.
Maybe a small Linux box handles audio for the room.
Sometimes you just want the machine in front of you to play the music.

With `ura`, all of those are the same operation: select a playback device.

- Play YouTube and YouTube Music URLs on this device or a paired Linux peer
- Play immediately, loop one track, or add tracks to the queue
- Toggle pause/resume, stop, inspect status, and replay earlier tracks through the same selected device
- Pair peers with a temporary six-digit code
- Switch devices with an interactive terminal selector
- Share the same selected device between the CLI and Firefox extension
- Keep `mpv` and its IPC interface local to each node

## Quick start

Start the local node daemon:

```console
$ ura daemon
```

With no paired peers, this device is selected automatically:

```console
$ ura device list
Selected device: desktop

Devices:
     NAME               TYPE         ADDRESS
  *  desktop            this device  -

$ ura "https://youtu.be/..."
```

The URL plays through this machine's `mpv` instance.

When installed through the Home Manager module, the user service starts the node for you, so normal use does not require running `ura daemon` manually.

## Pair another device

On the other Linux machine, expose its peer API and open pairing:

```console
$ ura daemon --bind 0.0.0.0:8765
$ ura pair
```

On this machine:

```console
$ ura pair 192.168.1.23
```

Then choose where playback should go:

```console
$ ura device select

Select playback device
Filter:

> desktop            This device
  living-room        http://192.168.1.23:8765

↑/↓ move  type to filter  enter select  esc cancel
```

The selection persists, so playback commands do not need a destination argument:

```console
$ ura "https://youtu.be/..."
$ ura toggle
$ ura
```

A bare URL plays immediately. URL options modify that request, while commands
are reserved for independent actions and management:

```console
$ ura --queue "https://youtu.be/..."
$ ura --loop "https://youtu.be/..."
$ ura stop
```

Running `ura` without a URL or command shows the selected device's status.

## Replay from history

History is numbered newest first:

```console
$ ura history
   #  TIMESTAMP             TITLE                         SOURCE
   1  2026-07-10 17:35:00   Example song                  url
   2  2026-07-09 09:12:00   Another song                  url
```

Replay an entry by number. Omitting the number replays the latest entry:

```console
$ ura history replay 2
$ ura history replay
```

History and replay follow the same selected-device routing as other playback
commands.

## Firefox extension

The Firefox extension no longer connects to remote peers directly. It talks only to the local ura node on a loopback-only control API, and the local node decides where playback goes.

Authorize Firefox once:

```console
$ ura pair
```

Open the extension options, enter the six-digit code, and choose a browser name such as `Firefox`.
The extension stores only the credential for the local node. Remote peer addresses and peer credentials remain owned by ura itself.

After that, changing the destination from either interface changes the same node state:

```console
$ ura device select
```

The extension options show the same selected device and can switch it too. Clicking the toolbar button sends the current tab to whichever device is selected at that moment.

The browser-facing control API listens only on `127.0.0.1:8766`. The peer API may be exposed on the LAN with `--bind`, but the browser control API is never bound to that address.

## Node model

Local commands are sent to the local `ura` node first. The node resolves the selected destination:

- **this device** → the node's own local playback backend
- **peer** → the command is forwarded to that paired node

Peer HTTP requests never resolve the receiving node's selected device. They always operate on that node's own `mpv` instance. This prevents accidental multi-hop forwarding and keeps destination selection local to the initiating node.

Firefox follows the same rule: it submits commands to the local node rather than contacting the selected peer itself.

## Command model

Playback, status, and history always go through the running local node. Device
selection is persistent and shared with Firefox; there is no one-shot
per-command destination override or direct-peer CLI mode.
