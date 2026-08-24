#!/usr/bin/env bash
# Install the local pre-commit guard against committing the vendor library.
#
# CI catches a leak after the fact; the hook catches it before it enters
# history, which is the difference between "unstage the file" and "rewrite the
# branch". Run once per clone.
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
hook="$(git rev-parse --git-path hooks/pre-commit)"

if [ -e "$hook" ] && ! grep -q 'check-no-vendor-blob' "$hook" 2>/dev/null; then
    echo "A pre-commit hook already exists and does not call the guard:" >&2
    echo "  $hook" >&2
    echo "Add this line to it manually:" >&2
    echo '  "$(git rev-parse --show-toplevel)"/scripts/ci/check-no-vendor-blob.sh' >&2
    exit 1
fi

mkdir -p "$(dirname "$hook")"
cat > "$hook" <<'HOOK'
#!/usr/bin/env sh
# Block commits that would add reMarkable's proprietary libqsgepaper.so.
"$(git rev-parse --show-toplevel)"/scripts/ci/check-no-vendor-blob.sh
HOOK
chmod +x "$hook"

echo "Installed pre-commit guard at ${hook#"$root"/}"
