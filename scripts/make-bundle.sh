#!/usr/bin/env bash
# Stage the AppLoad bundle into dist/riddle/, ready for `remagic publish`.
# Prereq: ./build-takeover.sh has produced the takeover binary.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=target/aarch64-unknown-linux-gnu/release/riddle-takeover
[ -f "$BIN" ] || { echo "build first: ./build-takeover.sh" >&2; exit 1; }
[ -f ../quill/build/libquill.so ] || { echo "missing ../quill/build/libquill.so" >&2; exit 1; }

rm -rf dist/riddle
mkdir -p dist/riddle
install -m 755 "$BIN" dist/riddle/riddle
install -m 755 ../quill/build/libquill.so dist/riddle/
install -m 755 scripts/appload-launch.sh scripts/riddle-takeover.sh dist/riddle/
install -m 644 external.manifest.json icon.png oracle.env.example settings.schema.json dist/riddle/

# Build separately with scripts/build-chatgpt.sh to include this optional backend.
if [ -f tools/chatgpt/dist/riddle-chatgpt-linux-arm64 ]; then
    install -m 755 tools/chatgpt/dist/riddle-chatgpt-linux-arm64 dist/riddle/riddle-chatgpt
fi

echo "staged: $(du -sh dist/riddle | cut -f1) in dist/riddle/"
echo "publish with: remagic publish dist/riddle -catalog-dir <remagic checkout>"
