#!/bin/sh
# Cross-build riddle for the reMarkable 2 (windowed/qtfb mode only) and
# assemble a ready-to-scp AppLoad bundle in dist/rm2/riddle/.
#
# The rM2 is 32-bit ARM. We target musl and link statically so the binary is
# independent of the device's (old) glibc. Requires cargo-zigbuild + zig:
#   rustup target add armv7-unknown-linux-musleabihf
#   brew install zig cargo-zigbuild        # or: cargo install cargo-zigbuild
set -e
cd "$(dirname "$0")"

# Homebrew installs rustup keg-only (not symlinked into PATH); find it anyway.
if ! command -v cargo >/dev/null 2>&1; then
    for p in /opt/homebrew/opt/rustup/bin "$HOME/.cargo/bin"; do
        [ -x "$p/cargo" ] && PATH="$p:$PATH" && break
    done
fi
command -v cargo >/dev/null 2>&1 || {
    echo "cargo not found — install Rust first: https://rustup.rs" >&2; exit 1; }

TARGET=armv7-unknown-linux-musleabihf
export RUSTFLAGS="-C target-feature=+crt-static"

cargo zigbuild --release --target $TARGET --features rm2 "$@"

OUT=target/$TARGET/release
DIST=dist/rm2/riddle
rm -rf "$DIST"
mkdir -p "$DIST"

cp "$OUT/riddle" "$DIST/riddle"
cp scripts/appload-launch-windowed.sh "$DIST/appload-launch.sh"
chmod +x "$DIST/riddle" "$DIST/appload-launch.sh"
cp icon.png oracle.env.example "$DIST/"
cat > "$DIST/external.manifest.json" <<'EOF'
{
  "name": "The Diary",
  "application": "appload-launch.sh",
  "qtfb": true
}
EOF

echo
echo "Bundle ready: $DIST"
echo "Install:  scp -O -r dist/rm2/riddle root@10.11.99.1:/home/root/xovi/exthome/appload/"
echo "Then create oracle.env in that folder with your RIDDLE_OPENAI_KEY."
