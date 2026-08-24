#!/usr/bin/env bash
# Refuse to let reMarkable's proprietary display library into the repository.
#
# quill/ is a clean-room MIT implementation that LINKS AGAINST libqsgepaper.so,
# but that library is reMarkable's property. Every user extracts it from a
# device they own (quill/build.sh pulls it over SSH); we never redistribute it.
#
# .gitignore already covers quill/vendor/ and quill/build/, but a stray
# `git add -f`, a moved build directory, or a downstream repository that forgets
# the ignore rule would leak it. This check is the backstop that makes the leak
# hard to commit and impossible to push unnoticed.
#
# Run manually:  ./scripts/ci/check-no-vendor-blob.sh
# Install hook:  ./scripts/ci/install-hooks.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# Filenames that are always the vendor library, wherever they appear.
FORBIDDEN_NAME='libqsgepaper\.so'
# Build output that must never be tracked. quill/vendor/ is deliberately NOT
# listed: it legitimately holds documentation telling users how to fetch the
# library from their own device. The library itself is caught by name above
# and by the ELF content scan below.
FORBIDDEN_PATH='^quill/build/'
# Smallest plausible size for the real library (~350KB); skip smaller files
# when doing the expensive content scan.
MIN_BLOB_BYTES=102400

fail=0

report() {
    if [ "$fail" -eq 0 ]; then
        echo "ERROR: proprietary vendor artifacts must never be committed." >&2
        echo >&2
    fi
    fail=1
    echo "  $1" >&2
}

matches_forbidden() {
    grep -E "${FORBIDDEN_PATH}|(^|/)${FORBIDDEN_NAME}(\.[0-9.]+)?$" || true
}

# 1. Nothing currently tracked may match.
while IFS= read -r path; do
    [ -n "$path" ] && report "tracked: $path"
done < <(git ls-files | matches_forbidden)

# 2. Nothing staged may match. This is the pre-commit path: it catches the
#    blob before it ever enters history, where removing it means a rewrite.
if git rev-parse --verify --quiet HEAD >/dev/null 2>&1; then
    while IFS= read -r path; do
        [ -n "$path" ] && report "staged: $path"
    done < <(git diff --cached --name-only --diff-filter=ACMR | matches_forbidden)
fi

# 3. Content check: a renamed blob still carries the vendor's ELF symbols.
#    Catches `cp libqsgepaper.so quill/src/display.bin` and similar.
while IFS= read -r path; do
    [ -n "$path" ] || continue
    [ -f "$path" ] || continue
    size=$(wc -c < "$path" 2>/dev/null || echo 0)
    [ "$size" -lt "$MIN_BLOB_BYTES" ] && continue
    # ELF magic: 0x7F 'E' 'L' 'F'
    [ "$(head -c 4 "$path" | od -An -tx1 | tr -d ' \n')" = "7f454c46" ] || continue
    # NOTE: `grep -q` exits on first match, which SIGPIPEs `strings`; under
    # `set -o pipefail` that turns a successful match into a failed pipeline
    # and the check silently passes. Count matches instead so the whole
    # stream is consumed and the exit status reflects the search, not the pipe.
    hits=$(strings -a "$path" 2>/dev/null | grep -cE 'qsgepaper|EPFramebuffer' || true)
    if [ "${hits:-0}" -gt 0 ]; then
        report "vendor ELF content in tracked file: $path"
    fi
done < <(git ls-files)

if [ "$fail" -ne 0 ]; then
    cat >&2 <<'MSG'

quill/vendor/ and quill/build/ are gitignored on purpose.
Users obtain libqsgepaper.so from their own device via quill/build.sh.
See quill/README.md and quill/CLEANROOM.md for the clean-room boundary.

If a file is already staged, unstage it:  git restore --staged <path>
MSG
    exit 1
fi

echo "OK: no proprietary vendor artifacts tracked or staged."
