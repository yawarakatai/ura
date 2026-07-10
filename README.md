# ura

> **Play it over there.**

Send music from your browser or terminal to the Linux machine connected to your speakers.

`ura` turns any Linux device into a lightweight remote audio receiver.
Open a YouTube video on your laptop, send it with one click, and let another machine play the audio through `mpv`.

```text
your laptop                         your speakers
┌──────────────────┐               ┌──────────────────┐
│ Firefox / ura CLI│ ─── send ───▶ │ ura + mpv        │
│                  │               │ Linux receiver   │
└──────────────────┘               └──────────────────┘
```

No web dashboard. No account. No streaming service integration.
Just pair your devices and send something to play.

## Why ura?

Maybe your desktop has the good speakers.
Maybe an old laptop sits under your monitor.
Maybe a small Linux box handles audio for the room.

With `ura`, you can control that machine without opening a remote shell or moving between devices.

- Send YouTube and YouTube Music URLs remotely
- Play immediately or add tracks to the queue
- Control playback from the CLI or Firefox
- Pair devices with a temporary six-digit code
- Switch between multiple receivers
- Keep `mpv` and its IPC interface local to the receiver

## Quick start

On the Linux machine connected to your speakers:

```console
$ ura serve --bind 0.0.0.0:8765
```

Open pairing:

```console
$ ura pair
```

Then, from another device:

```console
$ ura pair 192.168.1.23
$ ura play "https://youtu.be/..."
```

That is it. The controlling device does not need to run a background service.
