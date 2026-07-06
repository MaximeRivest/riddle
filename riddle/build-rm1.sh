#!/usr/bin/env bash
# Cross-build riddle for reMarkable 1/2 (armv7, windowed AppLoad/qtfb).
set -euo pipefail
cd "$(dirname "$0")"

export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc

cargo build --release --target armv7-unknown-linux-gnueabihf --no-default-features --features rm1

OUT=dist-rm1
rm -rf "$OUT"
mkdir -p "$OUT"

cp target/armv7-unknown-linux-gnueabihf/release/riddle "$OUT/riddle"
cp external.manifest.rm1.json "$OUT/external.manifest.json"
cp scripts/appload-launch-rm1.sh "$OUT/appload-launch-rm1.sh"
cp oracle.env.example "$OUT/oracle.env.example"
cp icon.png "$OUT/icon.png"
chmod +x "$OUT/riddle" "$OUT/appload-launch-rm1.sh"

echo "Built $OUT/ — copy to the tablet with setup-rm1.ps1 or setup-rm1.sh"