#!/usr/bin/env sh
set -eu

REPOSITORY_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$REPOSITORY_DIR"

if ! command -v node >/dev/null 2>&1 || ! command -v npm >/dev/null 2>&1; then
  printf '%s\n' 'Node.js and npm are required to compile and test the TypeScript UI.' >&2
  exit 1
fi
if ! command -v shellcheck >/dev/null 2>&1; then
  printf '%s\n' 'ShellCheck is required to validate the project scripts.' >&2
  exit 1
fi

npm run check:ui

for script in scripts/*.sh; do
  sh -n "$script"
done
shellcheck --severity=warning scripts/*.sh

cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

./scripts/check-third-party-licenses.sh

python3 -m unittest scripts/test_create_update_manifest.py

node --check companion/extension.js
node companion/test/extension.test.js
