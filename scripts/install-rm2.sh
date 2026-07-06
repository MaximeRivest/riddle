#!/usr/bin/env bash
# riddle rM2 installer — one command from zero to the diary on a reMarkable 2.
#
#   Usage:   ./scripts/install-rm2.sh                    # USB (10.11.99.1)
#            RM_HOST=192.168.1.42 ./scripts/install-rm2.sh   # over Wi-Fi
#
# Unlike the Paper Pro, the rM2 needs NO developer mode: SSH is stock. Find the
# root password on the tablet under Settings > Help > Copyrights and licenses >
# GPLv3 Compliance, then run this. It installs, over SSH:
#   1. your SSH key (so you type that password once)
#   2. xovi + AppLoad (arm32, from asivery's official releases)
#   3. xovi-tripletap persistence (triple-press power = toggle xovi)
#   4. the riddle bundle from dist/rm2/riddle (build-rm2.sh output)
# and asks for your API key to write oracle.env.
#
# Everything is reversible: `ssh root@10.11.99.1 /home/root/xovi/stock`
# returns the stock UI; a reboot does too.
set -euo pipefail

RM_HOST="${RM_HOST:-10.11.99.1}"
XOVI_BUNDLE_TAG="${XOVI_BUNDLE_TAG:-v19-23052026}"   # asivery/rm-xovi-extensions
APPLOAD_TAG="${APPLOAD_TAG:-v0.5.3}"                  # asivery/rm-appload
XOVI_BUNDLE_URL="https://github.com/asivery/rm-xovi-extensions/releases/download/${XOVI_BUNDLE_TAG}/xovi-arm32.tar.gz"
APPLOAD_URL="https://github.com/asivery/rm-appload/releases/download/${APPLOAD_TAG}/appload-arm32.zip"
TRIPLETAP_INSTALL_URL="https://raw.githubusercontent.com/rmitchellscott/xovi-tripletap/main/install.sh"

HERE="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE="$HERE/riddle/dist/rm2/riddle"

say()  { printf '\033[1m== %s\033[0m\n' "$*"; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

# rM2 SSH quirks, seen in the wild:
#  - dropbear 2020.81 (OS 3.x) closes the connection if an RSA host key is
#    negotiated (broken RSA signing path) — so prefer ed25519 explicitly. A
#    stale ssh-rsa entry for 10.11.99.1 in known_hosts forces RSA and triggers
#    exactly this; clear it with: ssh-keygen -R 10.11.99.1
#  - older firmware offers only ssh-rsa, which modern clients refuse by
#    default — so keep ssh-rsa as an accepted fallback.
WORK="$(mktemp -d)"

# One multiplexed SSH connection for the whole install (ControlMaster): the
# password is typed at most once; every later command and copy rides it.
SSH_OPTS=(-o HostKeyAlgorithms=ssh-ed25519,ssh-rsa -o PubkeyAcceptedAlgorithms=+ssh-rsa
          -o ControlMaster=auto -o "ControlPath=$WORK/ssh-ctrl" -o ControlPersist=300)
rm_ssh() { ssh -n "${SSH_OPTS[@]}" "root@$RM_HOST" "$@"; }
rm_ssh_stdin() { ssh "${SSH_OPTS[@]}" "root@$RM_HOST" "$@"; }
rm_scp() { scp -O "${SSH_OPTS[@]}" "$@"; }
cleanup() { ssh "${SSH_OPTS[@]}" -O exit "root@$RM_HOST" 2>/dev/null; rm -rf "$WORK"; }
trap cleanup EXIT

# --- 0. riddle bundle must exist locally -------------------------------------
[ -x "$BUNDLE/riddle" ] || die "no rM2 bundle at $BUNDLE — run riddle/build-rm2.sh first"

# --- 1. reach the tablet, install our key, confirm it is an rM2 --------------
# BatchMode probe: succeeds only with key auth, so a password-only tablet
# (e.g. fresh from a factory reset) correctly falls through to ssh-copy-id.
if ! ssh -n "${SSH_OPTS[@]}" -o BatchMode=yes -o ConnectTimeout=5 "root@$RM_HOST" true 2>/dev/null; then
    say "Installing your SSH key (type the tablet's password once — Settings > Help > GPLv3)"
    [ -f "$HOME/.ssh/id_ed25519.pub" ] || [ -f "$HOME/.ssh/id_rsa.pub" ] \
        || die "no SSH key found; create one with: ssh-keygen -t ed25519"
    ssh-copy-id "${SSH_OPTS[@]}" "root@$RM_HOST" || die "cannot reach root@$RM_HOST"
fi
echo "   Heads-up: one install step restarts the tablet UI. If your tablet"
echo "   has a PIN, unlock it when the lock screen appears mid-install."
MACHINE="$(rm_ssh 'cat /sys/devices/soc0/machine 2>/dev/null' || true)"
case "$MACHINE" in
    "reMarkable 2.0") say "Found a reMarkable 2" ;;
    *) die "this installer is rM2-only; device says: '${MACHINE:-unknown}'" ;;
esac

# --- 2. xovi (loader + scripts + qt-resource-rebuilder) ----------------------
say "Installing xovi"
curl -fL --retry 3 -o "$WORK/xovi.tar.gz" "$XOVI_BUNDLE_URL"
rm_scp "$WORK/xovi.tar.gz" "root@$RM_HOST:/tmp/xovi.tar.gz"
rm_ssh 'tar -xzf /tmp/xovi.tar.gz -C /home/root && rm -f /tmp/xovi.tar.gz'
# Activate the message broker if this bundle ships it inactive (harmless).
rm_ssh '[ -f /home/root/xovi/inactive-extensions/xovi-message-broker.so ] && \
        mv -f /home/root/xovi/inactive-extensions/xovi-message-broker.so \
              /home/root/xovi/extensions.d/ 2>/dev/null; true'

# --- 3. AppLoad ---------------------------------------------------------------
say "Installing the AppLoad launcher"
curl -fL --retry 3 -o "$WORK/appload.zip" "$APPLOAD_URL"
rm_scp "$WORK/appload.zip" "root@$RM_HOST:/tmp/appload.zip"
# Only appload.so is a xovi extension; the qtfb shims live under AppLoad's own
# exthome dir (NOT extensions.d, or xovi tries to load them and errors).
rm_ssh 'cd /tmp && rm -rf appload-unz && mkdir appload-unz && \
        unzip -oq appload.zip -d appload-unz && \
        cp -f appload-unz/appload.so /home/root/xovi/extensions.d/ && \
        mkdir -p /home/root/xovi/exthome/appload && \
        if [ -d appload-unz/shims ]; then cp -rf appload-unz/shims /home/root/xovi/exthome/appload/; fi && \
        if [ -d appload-unz/exthome ]; then cp -rf appload-unz/exthome/. /home/root/xovi/exthome/; fi && \
        rm -rf appload-unz appload.zip'

# qt-resource-rebuilder wants a per-OS-version hashtable; AppLoad itself does
# not need it, so best-effort only.
rm_ssh 'test -x /home/root/xovi/rebuild_hashtable && /home/root/xovi/rebuild_hashtable </dev/null' \
    || echo "   (hashtable rebuild skipped — fine for riddle)"

# --- 4. persistence (triple-press the power button to toggle xovi) -----------
say "Installing xovi-tripletap persistence"
rm_ssh "wget -qO- '$TRIPLETAP_INSTALL_URL' | bash" \
    || echo "   tripletap didn't install; start xovi manually: ssh root@$RM_HOST /home/root/xovi/start"

# --- 5. riddle ----------------------------------------------------------------
say "Installing riddle"
rm_ssh 'mkdir -p /home/root/xovi/exthome/appload'
rm_scp -r "$BUNDLE" "root@$RM_HOST:/home/root/xovi/exthome/appload/"

if ! rm_ssh 'test -f /home/root/xovi/exthome/appload/riddle/oracle.env'; then
    printf 'API key for the oracle (OpenAI/OpenRouter/compatible; empty to skip): '
    read -r KEY
    if [ -n "$KEY" ]; then
        # Recognize the provider from the key so Enter-through-the-prompts
        # does the right thing (an sk-or- key on the OpenAI base 401s).
        case "$KEY" in
            sk-or-*) DEF_BASE="https://openrouter.ai/api/v1"; DEF_MODEL="openai/gpt-4o-mini" ;;
            *)       DEF_BASE="https://api.openai.com/v1";    DEF_MODEL="gpt-4o-mini" ;;
        esac
        printf 'API base URL [%s]: ' "$DEF_BASE"
        read -r BASE; BASE="${BASE:-$DEF_BASE}"
        printf 'Vision model [%s]: ' "$DEF_MODEL"
        read -r MODEL; MODEL="${MODEL:-$DEF_MODEL}"
        rm_ssh_stdin "cat > /home/root/xovi/exthome/appload/riddle/oracle.env" <<EOF
RIDDLE_OPENAI_KEY=$KEY
RIDDLE_OPENAI_BASE=$BASE
RIDDLE_OPENAI_MODEL=$MODEL
EOF
        say "Verifying the oracle (needs tablet Wi-Fi)"
        rm_ssh 'cd /home/root/xovi/exthome/appload/riddle && set -a && . ./oracle.env && set +a && \
                ./riddle --oracle-test icon.png' \
            || echo "   oracle test failed — check Wi-Fi/key/model, then re-run: riddle --oracle-test"
    fi
fi

# --- 6. start xovi now --------------------------------------------------------
say "Starting xovi"
rm_ssh 'systemd-run --unit=xovi-firststart --collect --service-type=oneshot /home/root/xovi/start 2>/dev/null' \
    || echo "   couldn't start now — triple-press the power button, or reboot"

cat <<EOF

  Done. On the tablet, open AppLoad and tap "The Diary".
  Write something, rest the pen ~3 s, and watch the page drink your ink.

  Toggle xovi:   triple-press the power button
  Stock UI:      ssh root@$RM_HOST /home/root/xovi/stock   (or reboot)
EOF
