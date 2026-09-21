#!/usr/bin/env bash
set -euo pipefail
cd /workspaces/textlens

pnpm typecheck
pnpm test
pnpm build:web

# Use the Linux host target, not the existing macOS/Windows package scripts.
cargo check --manifest-path src-tauri/Cargo.toml --all-targets --locked
dbus-run-session -- xvfb-run -a cargo test \
  --manifest-path src-tauri/Cargo.toml --all-targets --locked -- --test-threads=1
