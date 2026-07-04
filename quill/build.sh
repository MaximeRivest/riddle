#!/bin/bash
# Cross-build quill against the ferrari SDK (OS 3.26 toolchain).
# Prereq: ~/rm-sdk-3.26 installed; libqsgepaper.so pulled from the device into ./vendor/.
set -euo pipefail
cd "$(dirname "$0")"

SDK=~/rm-sdk-3.26
ENV=$(ls $SDK/environment-setup-* | head -n1)
# The SDK env script sets CC/CXX with target flags and $SDKTARGETSYSROOT.
source "$ENV"

mkdir -p build vendor
if [ ! -f vendor/libqsgepaper.so ]; then
    echo "pulling libqsgepaper.so from device..."
    scp -O rm:/usr/lib/plugins/scenegraph/libqsgepaper.so vendor/
fi

QTINC="$SDKTARGETSYSROOT/usr/include"

# libquill.so: epfb-re shim (QImage constructor interposition) + C ABI.
# Must be FIRST on consumers' link lines so its interposed symbols win.
$CXX -fPIC -shared -O2 \
    -I "$QTINC" -I "$QTINC/QtCore" -I "$QTINC/QtGui" \
    src/epfb.cpp src/quill_c.cpp \
    -L vendor -lqsgepaper \
    -o build/libquill.so

# scribble: the C1 latency demo.
$CC -O2 src/scribble.c \
    -L build -lquill \
    -L vendor -lqsgepaper \
    -lQt6Gui -lQt6Core -lstdc++ \
    -Wl,-rpath,/home/root/quill \
    -o build/scribble

echo "built: build/libquill.so build/scribble"
