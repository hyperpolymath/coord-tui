# SPDX-License-Identifier: MPL-2.0
# Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
#
# BoJ coord-tui — shell launcher hooks and helper commands
#
# Source from ~/.bashrc / ~/.zshrc (the installer does this automatically):
#   [ -f "$HOME/.config/coord-tui/coord-hooks.sh" ] \
#       && source "$HOME/.config/coord-tui/coord-hooks.sh"
#
# What this gives you:
#   - Auto-registration + window title when you type: claude / gemini / cursor / codex / vibe
#   - coord-peers     — list all active peers (no TUI needed)
#   - coord-claims    — list all active task claims
#   - coord-claim     — claim a task from the command line
#   - coord-status    — set your status from the command line
#   - coord-whoami    — print your current peer ID

# ── Internal helpers ──────────────────────────────────────────────────────────

_coord_env() {
    local env_file="$HOME/.cache/coord-tui/peer.env"
    [ -f "$env_file" ] && source "$env_file"
}

_coord_token() {
    _coord_env
    printf '%s' "${BOJ_COORD_TOKEN:-}"
}

_coord_post() {
    local tool="$1"; shift
    local payload="$1"
    curl -sf -X POST "http://127.0.0.1:7745/tools/${tool}" \
        -H "Content-Type: application/json" \
        -d "$payload" 2>/dev/null
}

_coord_auto_register() {
    local kind="$1"
    # Registers silently, writes ~/.cache/coord-tui/peer.env, sets window title.
    # Falls through silently if the adapter is not running.
    coord-tui --id --kind "$kind" 2>/dev/null || true
    _coord_env
    # Fallback title set in case the binary already emitted it before the env loaded.
    [ -n "${BOJ_COORD_PEER_ID:-}" ] \
        && printf '\033]0;%s [%s]\007' "$kind" "$BOJ_COORD_PEER_ID"
}

# ── Tool launchers (auto-register on invocation) ──────────────────────────────

claude() { _coord_auto_register claude;  command claude  "$@"; }
gemini() { _coord_auto_register gemini;  command gemini  "$@"; }
cursor() { _coord_auto_register cursor;  command cursor  "$@"; }
codex()  { _coord_auto_register openai;  command codex   "$@"; }
vibe()   { _coord_auto_register vibe;    command vibe    "$@"; }

# ── Convenience commands (work in any shell without the TUI) ──────────────────

# List all active peers.
coord-peers() {
    local tok; tok="$(_coord_token)"
    if [ -z "$tok" ]; then
        echo "Not registered — run: coord-tui --id --kind claude" >&2; return 1
    fi
    _coord_post coord_list_peers "{\"token\":\"$tok\"}" \
        | python3 -c "
import sys, json
d = json.load(sys.stdin)
peers = d.get('peers', [])
print(f'  {len(peers)} peer(s):')
for p in peers:
    status = p.get('status','') or '—'
    print(f'  {p[\"peer_id\"]:30s}  {p[\"kind\"]:8s}  {status}')
" 2>/dev/null || echo "(no peers or adapter not running)"
}

# List all active task claims.
coord-claims() {
    local tok; tok="$(_coord_token)"
    if [ -z "$tok" ]; then
        echo "Not registered — run: coord-tui --id --kind claude" >&2; return 1
    fi
    _coord_post coord_list_claims "{\"token\":\"$tok\"}" \
        | python3 -c "
import sys, json
d = json.load(sys.stdin)
claims = d.get('active_claims', [])
if not claims:
    print('  No active claims.')
else:
    print(f'  {len(claims)} claim(s):')
    for c in claims:
        print(f'  {c[\"task\"]:40s}  holder={c.get(\"holder\",\"?\")}')
" 2>/dev/null || echo "(adapter not running)"
}

# Claim a task: coord-claim hypatia/my-task
coord-claim() {
    local task="${1:?Usage: coord-claim <task-name>}"
    local tok; tok="$(_coord_token)"
    if [ -z "$tok" ]; then
        echo "Not registered — run: coord-tui --id --kind claude" >&2; return 1
    fi
    local result
    result=$(_coord_post coord_claim_task \
        "{\"token\":\"$tok\",\"task\":\"$task\"}" 2>/dev/null)
    echo "$result" | python3 -c "
import sys, json
d = json.load(sys.stdin)
if d.get('success'):
    msg = d.get('message','')
    if msg == 'granted':
        print(f'  ✓ Claimed: $task')
    else:
        print(f'  ✗ {msg}')
else:
    print(f'  ✗ {d.get(\"error\",\"unknown error\")}')
" 2>/dev/null || echo "  ✗ Failed (adapter not running?)"
}

# Set your status: coord-status "doing the thing"
coord-status() {
    local status="${1:?Usage: coord-status <status text>}"
    local tok; tok="$(_coord_token)"
    if [ -z "$tok" ]; then
        echo "Not registered — run: coord-tui --id --kind claude" >&2; return 1
    fi
    _coord_post coord_status \
        "{\"token\":\"$tok\",\"status\":$(python3 -c "import json,sys; print(json.dumps('$status'))")}" \
        | python3 -c "
import sys, json
d = json.load(sys.stdin)
print('  ✓ Status set.' if d.get('success') else '  ✗ Failed.')
" 2>/dev/null
}

# Print your current peer ID and token.
coord-whoami() {
    _coord_env
    if [ -n "${BOJ_COORD_PEER_ID:-}" ]; then
        echo "  Peer:  ${BOJ_COORD_PEER_ID}"
        echo "  Token: ${BOJ_COORD_TOKEN:0:8}… (truncated)"
    else
        echo "  Not registered. Run: coord-tui --id --kind claude"
    fi
}
