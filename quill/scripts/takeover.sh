#!/bin/bash
# Run a quill takeover app safely: stop xochitl, own the panel, and ALWAYS
# bring xochitl back — on app exit, crash, or this script dying.
# Usage: takeover.sh /home/root/quill/scribble

APP="${1:-/home/root/quill/scribble}"

restore() {
    rm -f /tmp/epframebuffer.lock
    systemctl start xochitl
}
trap restore EXIT INT TERM

systemctl stop xochitl
# xochitl's userspace EPD lock can linger; a stale one blocks the engine.
rm -f /tmp/epframebuffer.lock
sleep 1

cd "$(dirname "$APP")"
LD_LIBRARY_PATH=/home/root/quill "$APP"
echo "takeover: app exited ($?), restoring xochitl"
