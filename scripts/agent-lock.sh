#!/usr/bin/env bash
# Lightweight advisory lock so two autonomous agents (or a human and an
# agent) working in the same repo checkout at the same time notice each
# other before they collide, instead of after. Not an OS-level lock — it's
# a courtesy file both sides are expected to check, matching how kaptaind
# and Claude Code sessions already coexist in these repos.
#
# Motivation: this session hit exactly the collision this exists to catch —
# a live agent was actively editing dreamseq/src/groq.rs while a second
# session started editing the same file, discovered only because the file's
# content changed mid-edit. A stale-checked lock would have surfaced that
# before either side wrote anything.
#
# Usage:
#   agent-lock.sh acquire "<description of what you're about to do>"
#   agent-lock.sh status
#   agent-lock.sh release
#
# Exit codes: 0 = ok, 1 = a live lock is held by someone else, 2 = usage error.
set -euo pipefail

lock_path="${AGENT_LOCK_PATH:-.agent-lock.json}"
stale_after_seconds="${AGENT_LOCK_STALE_SECONDS:-3600}"

now() { date -u +%s; }
iso_now() { date -u +"%Y-%m-%dT%H:%M:%SZ"; }

read_field() {
    # $1 = json field name, reads $lock_path
    php -r '
        $data = json_decode(file_get_contents($argv[1]), true);
        echo $data[$argv[2]] ?? "";
    ' "$lock_path" "$1" 2>/dev/null || true
}

is_stale() {
    local acquired_epoch
    acquired_epoch="$(read_field acquired_epoch)"
    [[ -z "$acquired_epoch" ]] && return 0
    local age=$(( $(now) - acquired_epoch ))
    [[ "$age" -gt "$stale_after_seconds" ]]
}

cmd_status() {
    if [[ ! -f "$lock_path" ]]; then
        echo "no lock held"
        return 0
    fi
    echo "holder:      $(read_field holder)"
    echo "description: $(read_field description)"
    echo "acquired_at: $(read_field acquired_at)"
    if is_stale; then
        echo "status:      STALE (older than ${stale_after_seconds}s — safe to override)"
    else
        echo "status:      live"
    fi
}

cmd_acquire() {
    local description="${1:?agent-lock.sh acquire "<description>" — description is required}"
    if [[ -f "$lock_path" ]] && ! is_stale; then
        echo "A live lock is already held:" >&2
        cmd_status >&2
        echo "" >&2
        echo "Another agent (or a human) is already working here. Coordinate before proceeding — don't override a live lock." >&2
        return 1
    fi
    local holder
    holder="${AGENT_LOCK_HOLDER:-$(whoami)@$(hostname 2>/dev/null || echo unknown)-pid$$}"
    php -r '
        $data = ["holder" => $argv[1], "description" => $argv[2], "acquired_at" => $argv[3], "acquired_epoch" => (int) $argv[4]];
        file_put_contents($argv[5], json_encode($data, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES));
    ' "$holder" "$description" "$(iso_now)" "$(now)" "$lock_path"
    echo "Lock acquired by $holder: $description"
}

cmd_release() {
    if [[ ! -f "$lock_path" ]]; then
        echo "no lock held"
        return 0
    fi
    rm -f "$lock_path"
    echo "Lock released."
}

case "${1:-}" in
    acquire) shift; cmd_acquire "${1:-}" ;;
    status) cmd_status ;;
    release) cmd_release ;;
    *)
        echo "usage: agent-lock.sh {acquire \"<description>\"|status|release}" >&2
        exit 2
        ;;
esac
