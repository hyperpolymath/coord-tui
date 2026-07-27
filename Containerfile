# SPDX-License-Identifier: MPL-2.0
#
# Containerfile — coord-tui
#
# Nix retirement note: Guix became the estate-primary packaging path on
# 2026-06-01; flake.nix is kept (it predates that ruling and Guix
# packaging is not yet wired up here) but no longer satisfies the
# governance container gate on its own — a sealed, buildable
# Containerfile is the accepted escape hatch. This file is a real,
# non-stub build: every dependency install below is an active RUN
# step, and the binary produced here is the actual `coord-tui` crate
# (rapid-setup TUI for BoJ local-coord-mcp).
#
# Toolchain: Rust, single-crate package, edition 2021 (see Cargo.toml).
# No rust-toolchain.toml pin exists in this repo to honour, so this
# uses Wolfi's rust-1.89 package (a recent stable release bundling
# both rustc and cargo; Wolfi does not ship a bare "cargo" package —
# `apk add cargo` fails with "no such package").
#
# Multi-stage build:
#   Stage 1: compile the coord-tui binary with cargo --release
#   Stage 2: copy the dynamically-linked release binary into a
#             minimal Chainguard glibc runtime image (Wolfi-built Rust
#             binaries are dynamically linked, so glibc-dynamic is
#             used, not the static runtime variant)
#
# Build:  podman build -t coord-tui-verify:latest -f Containerfile .
# Run:    podman run --rm -it coord-tui-verify:latest --help
# Seal:   podman build --no-cache -t coord-tui:sealed -f Containerfile .

# --- Stage 1: Build (Rust) ---
FROM cgr.dev/chainguard/wolfi-base:latest AS builder

# Rust toolchain (rustc + cargo, rust-1.89 bundles both) as packaged by Wolfi
RUN apk add --no-cache rust-1.89 gcc

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release && \
    cp target/release/coord-tui /build/coord-tui

# --- Stage 2: Runtime ---
FROM cgr.dev/chainguard/glibc-dynamic:latest

COPY --from=builder /build/coord-tui /usr/bin/coord-tui

USER nonroot

ENTRYPOINT ["/usr/bin/coord-tui"]
