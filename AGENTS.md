# AGENTS.md

## Project

`ura` is a small personal audio node for Linux/NixOS. Each machine can play a
supported YouTube URL locally and can route locally initiated playback commands
to a paired ura peer.

Keep the project small, boring, explicit, and useful.

## Architecture Summary

```text
Firefox / ura CLI
        │
        ▼
  local ura node
        │ selected destination
    ┌───┴────┐
    ▼        ▼
   self     peer
    │        │
   mpv   ura node → mpv
```

The long-running node exposes:

- `$XDG_RUNTIME_DIR/ura/node.sock` for local CLI routing
- `127.0.0.1:8766` for the loopback-only Firefox control API
- the configured peer HTTP bind, default `127.0.0.1:8765`
- local mpv JSON IPC and playback history in SQLite

## Target Platform And Language

- Primary platform: Linux and NixOS.
- Implementation language: Rust.
- Runtime dependencies: `mpv` and `yt-dlp`.
- Use the existing Nix flake and prefer `nix develop -c <command>` for commands
  that need the development environment.

Do not add Windows or macOS support unless explicitly requested.

## Important Invariants

- Only locally initiated commands may resolve the selected destination.
- Requests received through the peer HTTP API always control that node's own
  playback backend and must never be forwarded through its selected peer.
- This device is an implicit destination and must not require self-pairing or a
  fake remote-device entry.
- CLI and Firefox share destination state through the local ura node; do not
  create a second browser-owned peer list or selected-device state.
- The Firefox control API remains loopback-only. LAN/Tailscale exposure applies
  only to the explicitly configured peer API.
- `mpv` IPC remains local-only and must not be exposed over TCP.
- `mpv` runtime state and metadata come from structured JSON IPC events and
  properties, not parsed human-readable logs.
- User-supplied URLs are never passed through a shell.
- Supported media URLs remain allowlisted.
- Use XDG paths for config, data, and runtime files.
- Runtime behavior stays simple and explicit; playback commands must not
  secretly daemonize a second ura process behind systemd/Home Manager.
- Do not add public-internet exposure features.
- Authenticated HTTP control routes require bearer-token authentication.
- LAN or Tailscale binding remains explicit; default peer bind is local-only.
- Preserve compatibility deliberately rather than letting legacy
  controller/receiver concepts leak into new core abstractions.
- Code changes must include a numbered verification checklist.

## Code Style

- Keep changes scoped to the user's request.
- Prefer small incremental changes over large rewrites.
- Understand nearby code before editing.
- Prefer clear, readable, self-documenting Rust.
- Avoid premature abstraction and excessive fragmentation.
- Keep core routing logic as pure as practical and I/O at the edges.
- Prefer explicit error handling over panics for recoverable errors.
- Do not add dependencies unless clearly justified and discussed first.
- Update documentation when changing user-visible behavior, CLI flags, config
  formats, APIs, setup steps, or examples.

## Pairing Direction

Pairing protocol v1 is still asymmetric at the credential level. Do not confuse
that compatibility detail with the product architecture: ura machines are nodes,
not permanent sender/receiver roles.

A future symmetric protocol may add stable node identity and mutual trust, but
it must preserve the routing invariant that peer requests terminate at the
receiving node.

## Non-Goals

Do not implement these unless explicitly requested later:

- Chromecast compatibility
- Spotify direct playback
- DRM bypass
- ad blocking or ad bypass features
- YouTube downloading/ripping features
- video playback UI
- web dashboard
- phone app
- multi-user permissions
- remote public internet exposure
- playlist import/export or expansion
- recommendation engine
- account login or cloud sync
- Windows/macOS support

## Document Pointers

Architecture/API/data changes:
  read [docs/architecture.md](docs/architecture.md)

Pairing/auth changes:
  read [docs/pairing-auth.md](docs/pairing-auth.md)

Development/testing changes:
  read [docs/development.md](docs/development.md)

Nix/systemd/deployment changes:
  read [docs/deployment.md](docs/deployment.md)

## Verification

For implementation tasks, run the relevant checks and include a numbered
verification checklist. At minimum consider:

1. `cargo fmt --check`
2. `cargo clippy --locked --all-targets -- -D warnings`
3. `cargo test --locked`
4. `node --test extension/lib.test.js` when extension behavior is touched
5. manual command examples when routing/playback behavior changes
6. no unsupported feature was accidentally added

Use the full local gate when appropriate:

```bash
nix develop -c ./scripts/verify.sh
```

If validation cannot be run, report the exact command that was skipped and why.
