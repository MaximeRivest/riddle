#!/bin/sh
# AppLoad entry point for reMarkable 1/2 (windowed qtfb mode).
HERE=$(cd "$(dirname "$0")" && pwd)
SHIM=/home/root/xovi/exthome/appload/shims/qtfb-shim-32bit.so
if [ -f "$HERE/oracle.env" ]; then
    set -a
    . "$HERE/oracle.env"
    set +a
fi
# Route stylus through AppLoad's qtfb input shim (xochitl owns the real Wacom).
if [ -f "$SHIM" ]; then
    export LD_PRELOAD="$SHIM"
    export QTFB_SHIM_FB=0
    export QTFB_SHIM_INPUT=1
    export QTFB_SHIM_MODEL=RM1
    export QTFB_SHIM_INPUT_MODE=RM1
fi
exec "$HERE/riddle"