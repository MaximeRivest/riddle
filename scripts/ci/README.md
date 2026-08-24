# CI guards

## `check-no-vendor-blob.sh`

Refuses to let reMarkable's proprietary `libqsgepaper.so` into the repository.

`quill/` is a clean-room MIT implementation that *links against* that library,
but the library itself is reMarkable's property. Every user extracts it from a
device they own — `quill/build.sh` pulls it over SSH — and we never
redistribute it. See `quill/README.md` and `quill/CLEANROOM.md` for the
clean-room boundary this protects.

`.gitignore` already covers `quill/vendor/` and `quill/build/`. This guard is
the backstop for the cases `.gitignore` does not cover:

| Case | Caught by |
|---|---|
| `git add -f quill/vendor/libqsgepaper.so` | filename match |
| Blob renamed and hidden elsewhere in the tree | ELF content scan |
| `quill/build/` output committed | path match |
| Blob already in a previous commit | CI history scan |

It deliberately **allows** documentation under `quill/vendor/` — that directory
legitimately holds a README telling users how to fetch the library themselves.
The library is matched by filename and by content, not by living in that
directory.

### Run it

```sh
./scripts/ci/check-no-vendor-blob.sh
```

### Install the pre-commit hook

```sh
./scripts/ci/install-hooks.sh
```

CI catches a leak after it is pushed; the hook catches it before it enters
history — the difference between `git restore --staged` and rewriting a branch.
Run once per clone. The hook installs into the shared `.git/hooks/`, so it
covers every worktree of this repository.

### In CI

`.github/workflows/no-vendor-blob.yml` runs the working-tree check on every
push and pull request, then scans full reachable history for any object named
`libqsgepaper.so` that is large enough to be the real library (~350KB).

### If it fires

```sh
git restore --staged <path>   # not yet committed
```

If the blob is already in history, it must be removed with a history rewrite
(`git filter-repo`) before the branch can ship publicly. Verified clean as of
the commit that added this guard.

## Downstream repositories

Any repository that vendors `quill/` — including a future g-pad — must carry
this guard and the matching `.gitignore` rules. The ignore rules do not travel
with copied source; this check is what makes the boundary enforceable rather
than a convention.
