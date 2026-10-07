#!/usr/bin/env bash
set -euo pipefail
cd /workspaces/popper

# Only change ownership of container volume roots, never the host source tree.
sudo chown "$(id -u):$(id -g)" /workspaces/popper/node_modules /home/node/.cache

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
echo 'Popper ready. Run: bash .devcontainer/verify.sh or pnpm dev:web --host 0.0.0.0'

# ==================== SSH 配置 ====================
# 准备 SSH 目录和配置文件权限
SSH_DIR="${HOME}/.ssh"
SSH_CONFIG="${SSH_DIR}/config"

mkdir -p "${SSH_DIR}"
chmod 700 "${SSH_DIR}"

touch "${SSH_CONFIG}"
chmod 600 "${SSH_CONFIG}"

# 配置块标记，用于重复执行时更新已有配置
BEGIN_MARKER="# >>> devcontainer github ssh 443 >>>"
END_MARKER="# <<< devcontainer github ssh 443 <<<"

# 删除上一次由本脚本写入的配置块
if grep -qF "${BEGIN_MARKER}" "${SSH_CONFIG}"; then
    sed -i "\|${BEGIN_MARKER}|,\|${END_MARKER}|d" "${SSH_CONFIG}"
fi

# 将 GitHub SSH 连接配置为使用 443 端口
cat >> "${SSH_CONFIG}" <<'EOF'

# >>> devcontainer github ssh 443 >>>
Host github.com
    HostName ssh.github.com
    Port 443
# <<< devcontainer github ssh 443 <<<
EOF

echo "GitHub SSH configuration updated:"
echo "  github.com -> ssh.github.com:443"
# ================== SSH 配置结束 ==================
