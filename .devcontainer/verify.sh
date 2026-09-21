#!/usr/bin/env bash
set -euo pipefail
cd /workspaces/popper

pnpm typecheck
pnpm test
pnpm build:web

# Native Rust checks, tests and packaging run in the Windows GitHub workflow.
echo 'Frontend verification passed. Run the GitHub Windows workflow for native verification and packaging.'
