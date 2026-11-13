# The test rig's images. `player` is a small Linux system each headless
# Pactlocke runs in, with tc for network shaping and every time zone; it is
# built for x86_64 and, under emulation, for ARM64. `rig` adds iroh's own
# relay server (iroh-relay, at the version Cargo.lock has) for the rig's
# local relay. Neither holds the app (scripts/rig.sh builds it into a
# volume) or any ROM (mounted read-only at run time).
ARG RUST_VERSION
FROM rust:${RUST_VERSION}-bookworm AS relay
ARG IROH_RELAY_VERSION
RUN cargo install iroh-relay --locked --version "${IROH_RELAY_VERSION}" --features server

FROM debian:bookworm-slim AS player
RUN apt-get update \
    && apt-get install -y --no-install-recommends iproute2 tzdata libasound2 libudev1 ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY player.sh cut.sh /rig/
RUN chmod +x /rig/player.sh /rig/cut.sh

FROM player AS rig
COPY --from=relay /usr/local/cargo/bin/iroh-relay /usr/local/bin/iroh-relay
