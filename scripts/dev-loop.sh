#!/bin/sh
# Live-reload loop for the dev container (`just dev`).
#
# The dev image bakes in a compiled stub so all ~350 deps are prebuilt; the
# bind mounts shadow the stubs with real sources. The first start touches
# those sources so cargo sees them as newer than the baked artifacts — the
# same mtime pitfall the builder image handles with `touch` — otherwise the
# first run can silently serve the stub binary. (This bumps host mtimes;
# git tracks content, not mtimes, so the tree is unaffected.)
#
# A build-id gates this. The image records its toolchain + lockfile + profile
# fingerprint in /opt/rustweb-build-id; the dev_target volume records the id
# it was primed with:
#   - same id    → the volume's artifacts are newer than the sources, skip.
#   - no id      → freshly seeded volume, touch so the stubs are never served.
#   - other id   → the image rebuilt (toolchain/dep/profile bump) while the
#                  volume persisted. The shadowed image prime can't be
#                  re-seeded from inside the container, so warn once and let
#                  cargo rebuild lazily; `just remove` re-seeds on the next
#                  `just dev`.
PRIME_ID=$(cat /opt/rustweb-build-id 2>/dev/null || echo unknown)
VOLUME_ID=$(cat /app/target/.build-id 2>/dev/null || echo none)
if [ "$PRIME_ID" != "$VOLUME_ID" ]; then
    if [ "$VOLUME_ID" != none ]; then
        echo "dev: target cache built by an older image; rebuilding lazily. For a fast re-seed: just remove, then just dev" >&2
    fi
    find src migration tools -type f -exec touch {} +
    echo "$PRIME_ID" > /app/target/.build-id
fi
# Then watch the mounted paths and rerun (rebuild + restart) on any change,
# debounced so editor save bursts don't kill an in-flight build.
exec cargo watch -w src -w migration -w tools -w Cargo.toml -w Cargo.lock -w rust-toolchain.toml --delay 0.2 -x run
