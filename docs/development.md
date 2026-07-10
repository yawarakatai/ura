# Development

Status: Current behavior.

Use the project Nix flake for development and validation commands that need the
expected toolchain or runtime tools.

## Development Shell

```bash
nix develop
```

The shell includes Rust tooling plus Node.js, `mpv`, `yt-dlp`, and SQLite.

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
nix develop -c node --test extension/lib.test.js
nix develop -c nix flake check
```

The full local gate is:

```bash
nix develop -c ./scripts/verify.sh
```

`scripts/verify.sh` checks for `mpv`, `node`, `yt-dlp`, and `sqlite3`, then runs:

1. `cargo fmt --check`
2. `cargo clippy -- -D warnings`
3. `cargo test`
4. `cargo check --all-targets`
5. `node --test extension/lib.test.js`
6. `nix flake check`

## Local Real-Audio Smoke Test

Use a real receiver and client in separate terminals:

```bash
nix develop
cargo run -- config init
cargo run -- serve
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

Metadata tests use fake `mpv` IPC events such as:

```json
{"event":"file-loaded"}
{"event":"property-change","id":1,"name":"media-title","data":"Example song"}
{"event":"property-change","id":2,"name":"duration","data":222.5}
{"event":"property-change","id":3,"name":"metadata","data":{"TITLE":"Example song","ARTIST":"Example artist"}}
```

The fake tests should cover persistent event consumption, command response
request IDs, metadata normalization, queue association, stale-status clearing,
and malformed or unknown events.

When manually testing with a fake or wrapped `mpv`, keep the fake earlier in
`PATH` only for that shell and make sure it accepts the arguments used by
`ura serve`.

## Local-Media Metadata Smoke Test

For a deterministic real-`mpv` smoke test that does not need the internet, use a
local generated audio file with simple tags, then play it through a temporary
test path or direct `mpv` wrapper when working on IPC behavior. Verify that the
observer receives `file-loaded`, `media-title`, `duration`, and `metadata`
without parsing logs.

YouTube metadata smoke tests are optional and manual because they depend on
network access and upstream availability. When running one, verify playback
starts without a second metadata command, then check that `ura status` and
`ura history` show a real title and never print literal `null`.

## Multi-Device/Auth Verification

Use isolated XDG directories before manual multi-device testing so local
configuration and receiver history are untouched. Verify `device add`, `device
select`, `device remove`, and `--to` destination override behavior. On the
receiver side, verify that `device authorize` prints a token once, authorized
tokens authenticate, `device revoke` rejects only the revoked token, the legacy
configured token still works, and no token or token hash appears in normal
output or logs. Successful authorized-device authentication updates
`last_seen_at` no more than once per 60 seconds.

## Pairing Verification

Use separate isolated XDG paths for receiver and controller, even when both run
under the same Unix user:

```bash
export XDG_CONFIG_HOME=/tmp/ura-pair-receiver/config
export XDG_DATA_HOME=/tmp/ura-pair-receiver/data
export XDG_RUNTIME_DIR=/tmp/ura-pair-receiver/runtime
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
```

Use a different `/tmp/ura-pair-controller/...` set for the controller shell.

Pairing tests should cover:

- fake receiver/client pairing through `GET /v1/pair/info` and
  `POST /v1/pair/claim`
- isolated controller config updates after a successful claim
- token secrecy checks: no code, plaintext token, token hash, or
  `Authorization` header in logs
- cancellation, expiry, wrong-code attempt exhaustion, and malformed-code cases
- normal bearer-protected endpoints remaining protected while pairing is active

For an end-to-end smoke test, start `ura serve` in the receiver environment, run
`ura pair` in another receiver-environment shell, then run
`ura pair <address>` from the controller environment and verify `ura status`
authenticates with the newly stored credential.

## Firefox Temporary Extension Testing

1. Open Firefox.
2. Go to `about:debugging#/runtime/this-firefox`.
3. Click "Load Temporary Add-on".
4. Select `extension/manifest.json`.
5. Start `ura serve`.
6. Run `ura pair` on the receiver and keep the six-digit code visible.
7. Open the extension options.
8. Enter the receiver address, pairing code, receiver name, and browser device
   name, then click Pair.
9. Confirm the receiver is saved and selected.
10. Pair a second receiver, switch between receivers, and remove one receiver.
11. Open a supported YouTube URL and click the toolbar button.
12. Confirm the selected receiver plays it.
13. Confirm no token appears in the UI, browser console, or notifications.
14. To verify legacy migration, pre-populate `browser.storage.local` with
    `receiverUrl` and `token`, reload the options page, and confirm a `default`
    receiver is created while the legacy fields remain present.

The extension uses local storage for receiver devices and sends the current tab
URL to `/v1/play` or `/v1/enqueue` on the selected receiver. If no receiver is
selected, the toolbar action shows a notification that tells the user to pair or
select one.

## Logging

`ura serve` initializes tracing with `RUST_LOG` support. If `RUST_LOG` is not
set, it defaults to `ura=info`.

Examples:

```bash
RUST_LOG=ura=debug cargo run -- serve
RUST_LOG=ura=info ura serve
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
