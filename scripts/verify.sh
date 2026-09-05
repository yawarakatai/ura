#!/bin/sh
set -eu

for tool in mpv yt-dlp sqlite3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 1
    fi
done

cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --locked --all-targets
nix flake check
