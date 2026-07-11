#!/bin/sh
# Build the windowed reMarkable 2 AppLoad binary.
set -eu
cd "$(dirname "$0")"

TARGET=armv7-unknown-linux-gnueabihf
if command -v cross >/dev/null 2>&1; then
    cross build --release --target "$TARGET"
else
    : "${CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER:=arm-linux-gnueabihf-gcc}"
    export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER
    cargo build --release --target "$TARGET"
fi

cp "target/$TARGET/release/riddle" riddle-rm2
echo "built: $(pwd)/riddle-rm2"
