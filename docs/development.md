# Development

Status: Current `0.4.0` behavior.

Use the project Nix flake for development and validation commands that need the
expected toolchain or runtime tools.

## Development Shell

```bash
nix develop
```

The shell includes Rust tooling plus `mpv`, `yt-dlp`, and SQLite.
You can also run one command inside the shell:

```bash
nix develop -c cargo test --locked
```

## Verification Checklist

For code changes, verify in this order:

1. `nix develop -c cargo fmt --check`
2. `nix develop -c cargo clippy --locked --all-targets -- -D warnings`
3. `nix develop -c cargo test --locked`
4. `nix develop -c cargo check --locked --all-targets`
5. `nix develop -c nix flake check`
6. Run the relevant manual smoke test when user-visible routing or playback changes.

The full local gate is:

```bash
nix develop -c ./scripts/verify.sh
```

## Local Node Smoke Test

The local node is long-running. During repository development, use separate
terminals:

```bash
# terminal A
nix develop
cargo run -- daemon
```

```bash
# terminal B
nix develop
cargo run -- device list
cargo run -- "https://youtu.be/..."
cargo run --
cargo run -- toggle
cargo run -- stop
```

With no peer selected, `device list` should show this device as selected and the
URL should play through this machine's mpv instance.

When testing an installed Home Manager configuration, use the user service
instead of manually starting a second node process.

## Self/Peer Routing Smoke Test

Use two machines or isolated environments. Verify the routing invariant:

1. Start `ura daemon` on node A and node B.
2. Expose node B's peer API explicitly when needed, for example
   `ura daemon --bind 0.0.0.0:8765`.
3. Pair A with B.
4. On A, select `This device` and confirm playback is local to A.
5. On A, select B and confirm the same `ura <URL>` command plays on B.
6. On B, select some other destination if available, then send a peer request
   from A to B and confirm it still terminates on B rather than being forwarded.
7. Switch A back to `This device` through `ura device select` and confirm the
   selection persists.

Playback commands must use the persisted selected device; there is no one-shot
destination override.

## Config Verification

Use an isolated configuration path in tests. Cover:

1. Empty config → this device is the implicit destination.
2. Legacy flat peer fields do not create a remote device.
3. Adding/selecting/removing peers preserves unrelated node settings such as
   `bind` and removes obsolete credential fields when rewriting the config.
4. Selecting this device does not serialize a fake self peer.
5. No token or token hash appears in normal `device list` output.

## Pairing Verification

For peer pairing, use isolated XDG paths for the nodes under test. Pairing should
cover:

1. `ura pair` starts a pairing session only through the local administrative
   Unix socket.
2. `ura pair <address>` claims the session and stores the returned peer token.
3. Cancellation, expiry, wrong-code exhaustion, and malformed-code behavior.
4. Normal bearer-protected endpoints remain protected while pairing is active.
5. Successful authorized-client authentication updates `last_seen_at` no more
   than once per 60 seconds.
6. Codes, plaintext credentials, token hashes, and Authorization headers do not
   appear in logs.

## Fake mpv Lifecycle Tests

For lifecycle work around startup, shutdown, and socket handling, prefer
automated tests where possible. Existing tests cover stale socket removal, live
socket rejection, audio-only mpv startup arguments, API authentication, URL
validation, and client request construction.

Runtime state and metadata must come from structured mpv JSON IPC events and
properties. Do not parse human-readable mpv logs.

## Metadata Smoke Test

For deterministic IPC work, use a local generated audio file with simple tags or
a controlled mpv wrapper. Verify `file-loaded`, `media-title`, `duration`, and
`metadata` events are handled without log parsing.

YouTube metadata smoke tests are manual because they depend on network/upstream
behavior. Confirm playback starts, then check bare `ura` status and `ura
history` show useful metadata and never print literal `null` for missing
display values.

## Logging

`ura daemon` initializes tracing with `RUST_LOG` support and defaults to
`ura=info`.

```bash
RUST_LOG=ura=debug cargo run -- daemon
RUST_LOG=ura=info ura daemon
```

Logs must not print secrets. URL validation should log sanitized host/video-id
context rather than full untrusted URLs.

## Quoting URLs

Always quote URLs containing shell metacharacters such as `&`:

```bash
ura "https://www.youtube.com/watch?v=abc123&list=ignored"
```

Without quotes, the shell may split the URL before ura receives it.
