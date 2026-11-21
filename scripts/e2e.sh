#!/usr/bin/env bash
# The end-to-end suite: the real binary and the real board in Chromium and WebKit, inside the
# pinned Playwright container, so fonts and rendering are the same on every machine.
#
#   scripts/e2e.sh                  run the suite
#   scripts/e2e.sh --update         make the screenshot baselines again (see docs/testing.md)
#   scripts/e2e.sh -g "usage"       anything else goes to `playwright test`
#
# Needs docker, and node for the board's build when face/dist is missing. Two containers are
# started and both are removed when they stop: a Rust one builds the Linux binary (with the
# fixed-clock feature the tests need), a Playwright one runs the tests. Nothing touches your
# real ~/.claude, ~/.codex, ~/.gemini or ~/.agent-eye: every test uses temporary homes.
set -euo pipefail
cd "$(dirname "$0")/.."

pw=$(sed -n 's/.*"@playwright\/test": *"\([0-9.]*\)".*/\1/p' e2e/package.json)
[ -n "$pw" ] || { echo "no pinned @playwright/test in e2e/package.json" >&2; exit 1; }
rust_image=rust:1.89-bookworm
pw_image=mcr.microsoft.com/playwright:v${pw}-noble

args=()
for a in "$@"; do
  case "$a" in
    --update) args+=(--update-snapshots) ;;
    *) args+=("$a") ;;
  esac
done

cleanup() { docker rm -f eye-e2e-build eye-e2e-run >/dev/null 2>&1 || true; }
trap cleanup EXIT

# The board is embedded in the binary, so it is built first (it does not depend on the platform).
if [ ! -f face/dist/index.html ]; then
  (cd face && npm ci --ignore-scripts && npm run build)
fi

user=$(id -u):$(id -g)
mkdir -p target/e2e-cargo

echo "== building the Linux binary ($rust_image)"
docker run --rm --name eye-e2e-build --user "$user" \
  -v "$PWD:/work" -w /work \
  -e HOME=/tmp -e CARGO_HOME=/work/target/e2e-cargo -e CARGO_TARGET_DIR=/work/target/e2e-linux \
  "$rust_image" \
  cargo build --locked --features fixed-clock --bin agent-eye --example fixture_home

echo "== running the suite ($pw_image)"
docker run --rm --init --ipc=host --name eye-e2e-run --user "$user" \
  -v "$PWD:/work" -w /work/e2e \
  -e HOME=/tmp -e CI="${CI:-}" \
  "$pw_image" \
  bash -c 'npm ci --ignore-scripts --no-audit --no-fund >/dev/null && exec npx playwright test "$@"' _ ${args[@]+"${args[@]}"}
