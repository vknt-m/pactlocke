# The Linux job of .github/workflows/ci.yml, as an image: the Rust that CI's
# `stable` was on last (keep RUST_VERSION in step with it), clippy, rustfmt
# and the packages the Linux job installs. scripts/ci-docker.sh builds it once
# and reuses it.
ARG RUST_VERSION=1.100.0
FROM rust:${RUST_VERSION}-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        cmake libasound2-dev libxkbcommon-dev libxkbcommon-x11-0 libwayland-dev libx11-dev \
        libxcursor-dev libxrandr-dev libxi-dev mesa-vulkan-drivers xvfb xauth \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt
