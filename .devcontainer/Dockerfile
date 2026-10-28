FROM node:22.22.0-bookworm-slim

ARG RUST_VERSION=1.97.1
ARG PNPM_VERSION=11.7.0

USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl git openssh-client sudo vim \
    && rm -rf /var/lib/apt/lists/* \
    && echo 'node ALL=(root) NOPASSWD:ALL' > /etc/sudoers.d/node \
    && chmod 0440 /etc/sudoers.d/node \
    && npm install --global pnpm@${PNPM_VERSION}

ENV CARGO_HOME=/home/node/.cargo \
    RUSTUP_HOME=/home/node/.rustup \
    PATH=/home/node/.cargo/bin:${PATH}

# 提前创建 codex-accounts 卷的挂载路径，并设置父目录权限，避免 Docker
# 自动创建 root 所有的父目录，导致 VS Code Server 在 postCreateCommand 执行前无法写入。
RUN mkdir -p /home/node/.vscode-server/data/User/globalStorage/wannanbigpig.codex-accounts-manager \
    && chown -R node:node /home/node/.vscode-server

USER node
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/popper-rustup.sh \
    && sh /tmp/popper-rustup.sh -y --profile minimal --no-modify-path \
        --default-toolchain ${RUST_VERSION} --component rustfmt --component rust-src \
    && rm /tmp/popper-rustup.sh \
    && mkdir -p /home/node/.cache/popper

WORKDIR /workspaces/popper
