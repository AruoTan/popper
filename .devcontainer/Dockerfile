FROM node:22.22.0-bookworm

ARG RUST_VERSION=1.97.1
ARG PNPM_VERSION=11.7.0

USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential ca-certificates cmake curl file git nasm pkg-config \
        libssl-dev libgtk-3-dev libwebkit2gtk-4.1-dev \
        libayatana-appindicator3-dev librsvg2-dev patchelf \
        sudo unzip xauth xvfb dbus-x11 fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/* \
    && echo 'node ALL=(root) NOPASSWD:ALL' > /etc/sudoers.d/node \
    && chmod 0440 /etc/sudoers.d/node \
    && npm install --global pnpm@${PNPM_VERSION}

ENV CARGO_HOME=/home/node/.cargo \
    RUSTUP_HOME=/home/node/.rustup \
    PATH=/home/node/.cargo/bin:${PATH}

USER node
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/textlens-rustup.sh \
    && sh /tmp/textlens-rustup.sh -y --profile minimal --no-modify-path \
        --default-toolchain ${RUST_VERSION} --component rustfmt --component clippy \
    && rm /tmp/textlens-rustup.sh \
    && mkdir -p /home/node/.cache/textlens

WORKDIR /workspaces/textlens
