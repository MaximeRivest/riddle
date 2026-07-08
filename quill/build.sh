#!/bin/bash
# Cross-build quill against the ferrari SDK (OS 3.26 toolchain).
# Prereq: ~/rm-sdk-3.26 installed; libqsgepaper.so pulled from the device into ./vendor/.
set -euo pipefail
cd "$(dirname "$0")"

SDK=~/rm-sdk-3.26
ENV=$(ls $SDK/environment-setup-* | head -n1)
# The SDK env script sets CC/CXX with target flags and $SDKTARGETSYSROOT.
# It refuses to load when LD_LIBRARY_PATH is set.
unset LD_LIBRARY_PATH
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
    -L vendor -lqsgepaper -ldl \
    -o build/libquill.so

# scribble: the C1 latency demo.
$CC -O2 src/scribble.c \
    -L build -lquill \
    -L vendor -lqsgepaper -ldl \
    -lQt6Gui -lQt6Core -lstdc++ \
    -Wl,-rpath,/home/root/quill \
    -o build/scribble

# map_demo: static full-screen map + tiny partial-update footsteps.
$CC -O2 src/map_demo.c \
    -L build -lquill \
    -L vendor -lqsgepaper -ldl \
    -lQt6Gui -lQt6Core -lstdc++ \
    -Wl,-rpath,/home/root/quill \
    -o build/map_demo

# image_demo: render a PNG/JPEG/etc. through Qt's QImage loader.
$CXX -O2 \
    -I "$QTINC" -I "$QTINC/QtCore" -I "$QTINC/QtGui" \
    src/image_demo.cpp \
    -L build -lquill \
    -L vendor -lqsgepaper -ldl \
    -lQt6Gui -lQt6Core -lstdc++ \
    -Wl,-rpath,/home/root/quill \
    -o build/image_demo

# image_anim_demo: regional black/white fade animation experiment.
$CXX -O2 \
    -I "$QTINC" -I "$QTINC/QtCore" -I "$QTINC/QtGui" \
    src/image_anim_demo.cpp \
    -L build -lquill \
    -L vendor -lqsgepaper -ldl \
    -lQt6Gui -lQt6Core -lstdc++ \
    -Wl,-rpath,/home/root/quill \
    -o build/image_anim_demo

# gif_demo: dither animated GIF frames and partial-update changed regions.
$CXX -O2 \
    -I "$QTINC" -I "$QTINC/QtCore" -I "$QTINC/QtGui" \
    src/gif_demo.cpp \
    -L build -lquill \
    -L vendor -lqsgepaper -ldl \
    -lQt6Gui -lQt6Core -lstdc++ \
    -Wl,-rpath,/home/root/quill \
    -o build/gif_demo

# drawlab: no-AI live drawing experiments.
$CC -O2 src/drawlab.c \
    -L build -lquill \
    -L vendor -lqsgepaper -ldl \
    -lQt6Gui -lQt6Core -lstdc++ \
    -Wl,-rpath,/home/root/quill \
    -o build/drawlab

# home: Remagic Home takeover session launcher.
$CXX -O2 \
    -I "$QTINC" -I "$QTINC/QtCore" -I "$QTINC/QtGui" \
    src/home.cpp \
    -L build -lquill \
    -L vendor -lqsgepaper -ldl \
    -lQt6Gui -lQt6Core -lstdc++ \
    -Wl,-rpath,/home/root/quill \
    -o build/home

echo "built: build/libquill.so build/scribble build/map_demo build/image_demo build/image_anim_demo build/gif_demo build/drawlab build/home"
