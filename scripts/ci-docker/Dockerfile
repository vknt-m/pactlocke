# The Linux job of .github/workflows/ci.yml, as an image: the Rust version
# rust-toolchain.toml pins (scripts/ci-docker.sh passes it in), clippy,
# rustfmt and the packages the Linux job installs. The script builds it once
# per Rust version and reuses it.
ARG RUST_VERSION
FROM rust:${RUST_VERSION}-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        cmake libasound2-dev libudev-dev libxkbcommon-dev libxkbcommon-x11-0 libwayland-dev libx11-dev \
        libxcursor-dev libxrandr-dev libxi-dev mesa-vulkan-drivers xvfb xauth \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt
