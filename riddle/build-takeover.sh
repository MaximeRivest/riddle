#!/bin/bash
# Build riddle in TAKEOVER mode (links libquill.so + vendor Qt/qsgepaper).
#
# Must link with the ferrari SDK's gcc — the Ubuntu cross-linker's glibc (2.34)
# can't resolve GLIBC_2.36/2.38 symbols the device's Qt libs require. The SDK
# gcc ships the matching glibc 2.38 sysroot.
set -euo pipefail
cd "$(dirname "$0")"

SDK=~/rm-sdk-5.7.119
ENV=$(ls "$SDK"/environment-setup-* | head -n1)
unset LD_LIBRARY_PATH          # SDK env refuses to source otherwise
source "$ENV"                  # sets CC=aarch64-remarkable-linux-gcc ... --sysroot=...

# Ensure quill's build artifacts exist (libquill.so + vendor/libqsgepaper.so).
if [ ! -f ../quill/build/libquill.so ]; then
    echo "building quill first..."
    ( cd ../quill && ./build.sh )
fi

# Point cargo's aarch64 linker at the SDK gcc. $CC includes the -mcpu/-sysroot
# flags as one string; cargo wants a single program, so wrap it.
cat > /tmp/riddle-sdk-cc.sh <<EOF
#!/bin/bash
exec $CC "\$@"
EOF
chmod +x /tmp/riddle-sdk-cc.sh

export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=/tmp/riddle-sdk-cc.sh
# rustc still targets aarch64-unknown-linux-gnu (glibc); the SDK gcc just links.

cargo build --release --target aarch64-unknown-linux-gnu --features takeover "$@"

# The windowed (default-feature) build shares the same output path and would
# clobber this one. Copy the takeover binary to a distinct name so the two
# never collide.
OUT=target/aarch64-unknown-linux-gnu/release
cp "$OUT/riddle" "$OUT/riddle-takeover"
echo "built: $OUT/riddle-takeover (takeover; libquill-linked)"
