# syntax=docker/dockerfile:1
# Development-only image: toolchain + cargo-watch for `just dev`.
# Not a production artifact — no runtime stage.
FROM rust:1.97.1-bookworm
# Match the repo's toolchain spec: rust-toolchain.toml demands clippy + rustfmt,
# which the official rust image's minimal profile omits. Copied before the
# prime build so the pinned toolchain — not the image default — compiles it;
# a toolchain bump rebuilds the prime with the new pin instead of silently
# serving stale artifacts built by the old one.
COPY rust-toolchain.toml ./
RUN rustup component add clippy rustfmt
# mold: the GNU linker dominates dev rebuilds; this cuts link time ~10x.
RUN apt-get update && apt-get install -y --no-install-recommends mold \
    && rm -rf /var/lib/apt/lists/*
# Dev-profile tuning for this image only (host builds are untouched):
# mold for linking, line-tables-only debuginfo (backtraces still resolve,
# variable inspection is lost) so codegen and link do less work per save.
ENV CARGO_BUILD_RUSTFLAGS="-C link-arg=-fuse-ld=mold" \
    CARGO_PROFILE_DEV_DEBUG="line-tables-only"
RUN cargo install cargo-watch --locked --version 8.5.3
WORKDIR /app

# Prime the dependency cache: copy manifests + stub sources first so the
# ~350-crate tree is compiled once and reused across rebuilds.
COPY Cargo.toml Cargo.lock ./
COPY migration/Cargo.toml ./migration/Cargo.toml
COPY tools/tidy/Cargo.toml ./tools/tidy/Cargo.toml
RUN mkdir -p src migration/src tools/tidy/src \
    && printf 'fn main() {}\n' > src/main.rs \
    && printf 'fn main() {}\n' > migration/src/main.rs \
    && printf '\n' > migration/src/lib.rs \
    && printf '\n' > tools/tidy/src/main.rs \
    && cargo build --workspace

# Build-id: toolchain + lockfile + profile fingerprint. dev-loop.sh compares
# this against a marker in the dev_target volume to detect artifacts built by
# an older image (toolchain bump, dep change, tuning change). This layer only
# invalidates when the prime layers above it do, so the id is always in sync
# with the image's baked target.
RUN { rustc --version; sha256sum Cargo.lock; printf '%s\n' "$CARGO_BUILD_RUSTFLAGS" "$CARGO_PROFILE_DEV_DEBUG"; } > /opt/rustweb-build-id

COPY scripts/dev-loop.sh /usr/local/bin/dev-loop
RUN chmod +x /usr/local/bin/dev-loop
