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

There is no sender-only or receiver-only role. This device is always available as a playback destination; remote devices become available after pairing.

No web dashboard. No account. No streaming service integration.
Just choose a device and play something.

## Why ura?

Maybe your desktop has the good speakers.
Maybe an old laptop sits under your monitor.
Maybe a small Linux box handles audio for the room.
Sometimes you just want the machine in front of you to play the music.

With `ura`, all of those are the same operation: select a playback device.

- Play YouTube and YouTube Music URLs on this device or a paired Linux peer
- Play immediately or add tracks to the queue
- Pause, resume, stop, loop, inspect status, and view history through the same selected device
- Pair peers with a temporary six-digit code
- Switch devices with an interactive terminal selector
- Keep `mpv` and its IPC interface local to each node

## Quick start

Start the local node:

```console
$ ura serve
```

With no paired peers, this device is selected automatically:

```console
$ ura device list
Selected device: desktop

Devices:
     NAME               TYPE         ADDRESS
  *  desktop            this device  -

$ ura play "https://youtu.be/..."
```

The URL plays through this machine's `mpv` instance.

## Pair another device

On the other Linux machine, expose its peer API and open pairing:

```console
$ ura serve --bind 0.0.0.0:8765
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

The selection persists, so normal playback commands do not need a destination argument:

```console
$ ura play "https://youtu.be/..."
$ ura pause
$ ura status
```

`--to <NAME>` remains available as a one-shot override when you do not want to change the selected device.

## Node model

Local commands are sent to the local `ura` node first. The node resolves the selected destination:

- **this device** → the node's own local playback backend
- **peer** → the command is forwarded to that paired node

Peer HTTP requests never resolve the receiving node's selected device. They always operate on that node's own `mpv` instance. This prevents accidental multi-hop forwarding and keeps destination selection local to the initiating node.

## Current compatibility

The existing pairing protocol, bearer-token authentication, HTTP peer API, history database, and legacy direct receiver overrides are still supported while the node model is introduced. Existing remote-only controller configurations continue to work when no local `ura` node is running.
