# AGENTS.md

## Project

`ura` is a small personal audio receiver for Linux/NixOS. A CLI or browser
extension sends a supported YouTube URL to `ura serve`; the receiver controls
`mpv` over local JSON IPC and plays audio only.

Keep the project small, boring, explicit, and useful.

## Architecture Summary

```text
browser extension / CLI
  -> HTTP API with bearer-token auth
  -> ura serve
  -> local mpv JSON IPC
  -> audio-only playback
```

The receiver also records playback history in SQLite and uses XDG paths for
config, data, and runtime socket locations.

## Target Platform And Language

- Primary platform: Linux and NixOS.
- Implementation language: Rust.
- Runtime dependencies: `mpv` and `yt-dlp`.
- Use the existing Nix flake and prefer `nix develop -c <command>` for commands
  that need the development environment.

Do not add Windows or macOS support unless explicitly requested.

## Important Invariants

- `mpv` IPC remains local-only and must not be exposed over TCP.
- `mpv` runtime state and metadata must come from structured JSON IPC events and
  properties, not parsed human-readable logs.
- User-supplied URLs are never passed through a shell.
- Supported media URLs remain allowlisted.
- Use XDG paths for config, data, and runtime files.
- Runtime behavior must stay simple and explicit.
- Do not add public-internet exposure features.
- The HTTP API requires bearer-token authentication.
- LAN or Tailscale binding must remain explicit; default bind is local-only.
- Code changes must include a numbered verification checklist.

## Code Style

- Keep changes scoped to the user's request.
- Prefer small incremental changes over large rewrites.
- Understand nearby code before editing.
- Prefer clear, readable, self-documenting Rust.
- Avoid premature abstraction and excessive fragmentation.
- Keep core logic as pure as practical and keep I/O at the edges.
- Prefer explicit error handling over panics for recoverable errors.
- Do not add dependencies unless clearly justified and discussed first.
- Update documentation when changing user-visible behavior, CLI flags, config
  formats, APIs, setup steps, or examples.

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

1. `cargo fmt`
2. `cargo clippy`
3. `cargo test`
4. manual command examples when behavior changes
5. no unsupported feature was accidentally added

Use the full local gate when appropriate:

```bash
nix develop -c ./scripts/verify.sh
```

If validation cannot be run, report the exact command that was skipped and why.
