#!/bin/sh
# The equivalent of the web app's `npm run check` — run it after every change.
set -e
cd "$(dirname "$0")/.."
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
