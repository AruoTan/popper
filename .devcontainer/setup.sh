#!/usr/bin/env bash
set -euo pipefail
cd /workspaces/textlens

# Only change ownership of container volume roots, never the host source tree.
sudo chown "$(id -u):$(id -g)" /workspaces/textlens/node_modules /home/node/.cache/textlens

expected_pnpm="$(node -p "require('./package.json').packageManager")"
if [[ "pnpm@$(pnpm --version)" != "$expected_pnpm" ]]; then
  echo "pnpm mismatch: update PNPM_VERSION in .devcontainer/Dockerfile to $expected_pnpm and rebuild." >&2
  exit 1
fi

pnpm install --frozen-lockfile
node --version
pnpm --version
rustc --version
cargo --version
echo 'TextLens ready. Run: bash .devcontainer/verify.sh or pnpm dev:web --host 0.0.0.0'
