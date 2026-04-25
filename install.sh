#!/usr/bin/env bash
# SPDX-License-Identifier: PMPL-1.0-or-later
# Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
#
# coord-tui installer — sets up BoJ local-coord-mcp + coord-tui on a new machine.
#
# Run from the boj-server repo root:
#   bash coord-tui/install.sh
# Or from anywhere with a full path:
#   bash /path/to/boj-server/coord-tui/install.sh
#
# What this script does:
#   1. Builds the Zig coord adapter (local-coord-mcp, port 7745)
#   2. Builds the Rust coord-tui binary
#   3. Installs both to ~/.local/bin/
#   4. Installs a systemd user service to keep the adapter running
#   5. Installs shell hooks (~/.config/coord-tui/coord-hooks.sh)
#   6. Sources the hooks from ~/.bashrc and ~/.zshrc
#
# Requirements: cargo (Rust), zig, systemd (optional but recommended)

set -euo pipefail

# ── Paths ──────────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BOJ_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
ADAPTER_DIR="$BOJ_ROOT/cartridges/local-coord-mcp/adapter"
LOCAL_BIN="${HOME}/.local/bin"
SYSTEMD_DIR="${HOME}/.config/systemd/user"
HOOKS_DEST="${HOME}/.config/coord-tui/coord-hooks.sh"
SERVICE_FILE="$SYSTEMD_DIR/local-coord-mcp.service"

# ── Helpers ────────────────────────────────────────────────────────────────────

say()  { printf '\e[1;36m▶\e[0m  %s\n' "$*"; }
ok()   { printf '\e[1;32m✓\e[0m  %s\n' "$*"; }
warn() { printf '\e[1;33m!\e[0m  %s\n' "$*"; }
die()  { printf '\e[1;31m✗\e[0m  %s\n' "$*" >&2; exit 1; }

require() {
    command -v "$1" >/dev/null 2>&1 \
        || die "Required tool not found: $1 — please install it first."
}

patch_rc() {
    local rc="$1"
    local marker="# coord-tui hooks"
    grep -qF "$marker" "$rc" 2>/dev/null && return
    printf '\n%s\n[ -f "%s" ] && source "%s"\n' \
        "$marker" "$HOOKS_DEST" "$HOOKS_DEST" >> "$rc"
    ok "Patched $rc"
}

# ── Preflight ──────────────────────────────────────────────────────────────────

printf '\n'
say "BoJ coord-tui installer"
printf '\n'

require cargo
require zig

[ -d "$ADAPTER_DIR" ] \
    || die "Coord adapter not found at $ADAPTER_DIR — is boj-server fully cloned?"
[ -f "$SCRIPT_DIR/Cargo.toml" ] \
    || die "coord-tui Cargo.toml not found at $SCRIPT_DIR — something is wrong."

# ── Build Zig adapter ──────────────────────────────────────────────────────────

say "Building Zig coord adapter…"
(cd "$ADAPTER_DIR" && zig build -Doptimize=ReleaseSafe)
ADAPTER_BIN="$ADAPTER_DIR/zig-out/bin/local_coord_adapter"
[ -f "$ADAPTER_BIN" ] \
    || die "Zig build succeeded but binary not found at $ADAPTER_BIN"
ok "Zig adapter: $ADAPTER_BIN"

# ── Build coord-tui ────────────────────────────────────────────────────────────

say "Building coord-tui (Rust)…"
(cd "$SCRIPT_DIR" && cargo build --release --quiet)
TUI_BIN="$SCRIPT_DIR/target/release/coord-tui"
[ -f "$TUI_BIN" ] \
    || die "Cargo build succeeded but binary not found at $TUI_BIN"
ok "coord-tui: $TUI_BIN"

# ── Install binaries ───────────────────────────────────────────────────────────

say "Installing symlinks to $LOCAL_BIN…"
mkdir -p "$LOCAL_BIN"
ln -sf "$TUI_BIN"     "$LOCAL_BIN/coord-tui"
ln -sf "$ADAPTER_BIN" "$LOCAL_BIN/local_coord_adapter"
ok "coord-tui  →  $LOCAL_BIN/coord-tui"
ok "adapter    →  $LOCAL_BIN/local_coord_adapter"

if ! printf '%s' "$PATH" | grep -qF "$LOCAL_BIN"; then
    warn "$LOCAL_BIN is not on your PATH. Add to your shell rc:"
    warn "  export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

# ── Systemd user service ───────────────────────────────────────────────────────

say "Installing systemd user service…"
mkdir -p "$SYSTEMD_DIR"
cat > "$SERVICE_FILE" <<EOF
# SPDX-License-Identifier: PMPL-1.0-or-later
[Unit]
Description=BoJ local-coord-mcp adapter (AI multi-instance coordination)
After=network.target

[Service]
ExecStart=${ADAPTER_BIN}
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
EOF
ok "Service: $SERVICE_FILE"

if command -v systemctl >/dev/null 2>&1; then
    systemctl --user daemon-reload
    systemctl --user enable --now local-coord-mcp
    ok "Service enabled and started (port 7745)"
else
    warn "systemctl not available — start the adapter manually before using coord-tui:"
    warn "  $ADAPTER_BIN &"
fi

# ── BoJ REST server service ────────────────────────────────────────────────────

say "Installing BoJ REST server service (port 7700)…"

BOJ_REST_SERVICE_SRC="$BOJ_ROOT/elixir/boj-rest.service"
BOJ_REST_SERVICE_DST="$SYSTEMD_DIR/boj-rest.service"

if [ ! -f "$BOJ_REST_SERVICE_SRC" ]; then
    warn "boj-rest.service template not found at $BOJ_REST_SERVICE_SRC — skipping."
else
    mkdir -p "${HOME}/.local/share/boj-server"
    BOJ_ROOT="$BOJ_ROOT" HOME="$HOME" envsubst < "$BOJ_REST_SERVICE_SRC" > "$BOJ_REST_SERVICE_DST"
    ok "Service: $BOJ_REST_SERVICE_DST"

    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user daemon-reload
        systemctl --user enable --now boj-rest
        ok "Service enabled and started (port 7700)"
    else
        warn "systemctl not available — start the server manually:"
        warn "  cd $BOJ_ROOT/elixir && MIX_ENV=dev mix run --no-halt"
    fi
fi

# ── Shell hooks ────────────────────────────────────────────────────────────────

say "Installing shell hooks…"
mkdir -p "$(dirname "$HOOKS_DEST")"
cp "$SCRIPT_DIR/shell/coord-hooks.sh" "$HOOKS_DEST"
ok "Hooks: $HOOKS_DEST"

for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
    [ -f "$rc" ] && patch_rc "$rc"
done

# ── Done ───────────────────────────────────────────────────────────────────────

printf '\n'
ok "Installation complete."
printf '\n'
printf '  Services running:\n'
printf '    boj-rest        — Elixir REST server on :7700 (112 cartridges)\n'
printf '    local-coord-mcp — Zig coord adapter on :7745\n'
printf '\n'
printf '  Open a new shell (or run: source %s)\n' "$HOOKS_DEST"
printf '  then launch a tool to auto-register:\n\n'
printf '    claude          # registers + sets window title to "claude [<peer_id>]"\n'
printf '    gemini          # same for Gemini\n'
printf '    cursor          # same for Cursor / Vibe\n'
printf '    codex           # same for Codex\n'
printf '    coord-tui       # interactive TUI (see all peers + claims)\n'
printf '\n'
printf '  Check services:\n'
printf '    systemctl --user status boj-rest\n'
printf '    systemctl --user status local-coord-mcp\n'
printf '    curl http://localhost:7700/health\n'
printf '\n'
