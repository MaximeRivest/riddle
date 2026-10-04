#!/bin/bash
# Launch the diary in full-takeover mode: stop xochitl, run riddle against the
# vendor e-ink engine (instant ink), ALWAYS restore xochitl on exit.
#
# Exit the diary: 5-finger tap or SIGTERM; the power button sleeps/wakes it. Escape hatch if
# anything wedges: ssh rm 'systemctl start xochitl'.

# Keep one owner of the display. Reject a duplicate before installing cleanup
# traps, so it cannot restart xochitl beneath an already-running diary.
exec 9>/run/riddle-takeover.lock
flock -n 9 || { echo "The diary is already open" >&2; exit 1; }

restore() {
    if [ -n "${CHATGPT_PID:-}" ]; then
        kill "$CHATGPT_PID" 2>/dev/null || true
        wait "$CHATGPT_PID" 2>/dev/null || true
    fi
    [ -n "${REMAGIC_SESSION:-}" ] && return
    rm -f /tmp/epframebuffer.lock
    systemctl start xochitl
}
# Under the Remagic Home session host (REMAGIC_SESSION=1), xochitl is already
# stopped and the session owns its restore — skip our own stop/restart.
trap restore EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Resolve our own install directory so the bundle works wherever it lives
# (e.g. /home/root/xovi/exthome/appload/riddle/ when installed via AppLoad).
HERE=$(cd "$(dirname "$0")" && pwd)

# Oracle config: put your API key in oracle.env next to this script, e.g.
#   RIDDLE_OPENAI_KEY=sk-...
#   RIDDLE_OPENAI_BASE=https://api.openai.com/v1     # optional
#   RIDDLE_OPENAI_MODEL=gpt-4o-mini                  # optional
# Without it, riddle falls back to the pi backend (if pi is installed).
if [ -f "$HERE/oracle.env" ]; then
    set -a; . "$HERE/oracle.env"; set +a
fi

# Optional direct ChatGPT plan backend. Credentials stay in the companion;
# the diary sees only a loopback relay token, never an OpenAI access token.
if [ "${RIDDLE_CHATGPT:-0}" = 1 ]; then
    AUTH_DIR="$HERE/chatgpt-auth"
    [ -x "$HERE/riddle-chatgpt" ] || { echo "riddle-chatgpt is not installed" >&2; exit 1; }
    [ -f "$AUTH_DIR/credentials.json" ] || { echo "Sign in with ChatGPT first; see docs/CHATGPT.md" >&2; exit 1; }
    "$HERE/riddle-chatgpt" serve --state-dir "$AUTH_DIR" \
        --listen 127.0.0.1:17843 --model "${RIDDLE_CHATGPT_MODEL:-}" &
    CHATGPT_PID=$!
    for attempt in $(seq 1 10); do
        kill -0 "$CHATGPT_PID" 2>/dev/null || { echo "ChatGPT companion failed to start" >&2; exit 1; }
        wget -q -O /dev/null http://127.0.0.1:17843/health && break
        sleep 1
    done
    wget -q -O /dev/null http://127.0.0.1:17843/health || { echo "ChatGPT companion is not ready" >&2; exit 1; }
    export RIDDLE_OPENAI_KEY="$(cat "$AUTH_DIR/local-token")"
    export RIDDLE_OPENAI_BASE=http://127.0.0.1:17843/v1
fi

if [ -z "${REMAGIC_SESSION:-}" ]; then
    systemctl stop xochitl
fi
rm -f /tmp/epframebuffer.lock      # stale EPD lock blocks the engine
[ -z "${REMAGIC_SESSION:-}" ] && sleep 1

cd "$HERE"
# libquill.so ships in this bundle; libqsgepaper.so (reMarkable's proprietary
# engine) comes from the device's own scenegraph plugin dir. We search the
# bundle first, then a standalone /home/root/quill install, then the plugin dir.
LD_LIBRARY_PATH="$HERE:/home/root/quill:/usr/lib/plugins/scenegraph" \
    PAPERTERM_SHELL= HOME=/home/root \
    "$HERE/riddle"
echo "riddle-takeover: diary closed ($?), restoring xochitl"
