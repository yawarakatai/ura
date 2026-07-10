# Development

Status: Current behavior.

Use the project Nix flake for development and validation commands that need the
expected toolchain or runtime tools.

## Development Shell

```bash
nix develop
```

The shell includes Rust tooling plus `mpv`, `yt-dlp`, and SQLite.

You can also run one command inside the shell:

```bash
nix develop -c cargo test
```

## Validation Commands

Run focused checks while developing:

```bash
nix develop -c cargo fmt --check
nix develop -c cargo clippy -- -D warnings
nix develop -c cargo test
nix develop -c nix flake check
```

The full local gate is:

```bash
nix develop -c ./scripts/verify.sh
```

`scripts/verify.sh` checks for `mpv`, `yt-dlp`, and `sqlite3`, then runs:

1. `cargo fmt --check`
2. `cargo clippy -- -D warnings`
3. `cargo test`
4. `cargo check --all-targets`
5. `nix flake check`

## Local Real-Audio Smoke Test

Use a real receiver and client in separate terminals:

```bash
nix develop
cargo run -- config init
cargo run -- receive
```

Then:

```bash
nix develop
cargo run -- play "https://youtu.be/..."
cargo run -- status
cargo run -- toggle
cargo run -- stop
```

Use a short known-good YouTube URL and verify that audio plays through the
receiver machine.

## Fake mpv Lifecycle Tests

For lifecycle work around receiver startup, shutdown, and socket handling, prefer
automated tests where possible. Existing tests cover stale socket removal, live
socket rejection, audio-only `mpv` startup arguments, API authentication, URL
validation, and client request construction.

When manually testing with a fake or wrapped `mpv`, keep the fake earlier in
`PATH` only for that shell and make sure it accepts the arguments used by
`ura receive`.

## Firefox Temporary Extension Testing

1. Open Firefox.
2. Go to `about:debugging#/runtime/this-firefox`.
3. Click "Load Temporary Add-on".
4. Select `extension/manifest.json`.
5. Open the extension options.
6. Set receiver URL, token, and default action.
7. Start `ura receive`.
8. Open a supported YouTube URL and click the toolbar button.

The extension uses local storage for settings and sends the current tab URL to
`/v1/play` or `/v1/enqueue`.

## Logging

`ura receive` initializes tracing with `RUST_LOG` support. If `RUST_LOG` is not
set, it defaults to `ura=info`.

Examples:

```bash
RUST_LOG=ura=debug cargo run -- receive
RUST_LOG=ura=info ura receive
```

Logs should avoid printing full secrets. URL validation logs sanitized host and
video-id context rather than full untrusted URLs.

## Quoting URLs

Always quote URLs that contain shell metacharacters such as `&`:

```bash
ura play "https://www.youtube.com/watch?v=abc123&list=ignored"
```

Without quotes, the shell may split the command before `ura` receives the full
URL.
