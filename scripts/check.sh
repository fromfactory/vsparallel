#!/usr/bin/env sh
set -eu

REPOSITORY_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$REPOSITORY_DIR"

cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

if command -v node >/dev/null 2>&1; then
  node --check ui/theme-init.js
  node --check ui/app.js
  node --check ui/test/interface.test.js
  node --check ui/test/window-chrome.test.js
  node --check companion/extension.js
  node ui/test/interface.test.js
  node ui/test/window-chrome.test.js
  node companion/test/extension.test.js
elif [ -x /snap/code/current/usr/share/code/code ]; then
  ELECTRON_RUN_AS_NODE=1 /snap/code/current/usr/share/code/code --check ui/theme-init.js
  ELECTRON_RUN_AS_NODE=1 /snap/code/current/usr/share/code/code --check ui/app.js
  ELECTRON_RUN_AS_NODE=1 /snap/code/current/usr/share/code/code --check ui/test/interface.test.js
  ELECTRON_RUN_AS_NODE=1 /snap/code/current/usr/share/code/code --check ui/test/window-chrome.test.js
  ELECTRON_RUN_AS_NODE=1 /snap/code/current/usr/share/code/code --check companion/extension.js
  ELECTRON_RUN_AS_NODE=1 /snap/code/current/usr/share/code/code ui/test/interface.test.js
  ELECTRON_RUN_AS_NODE=1 /snap/code/current/usr/share/code/code ui/test/window-chrome.test.js
  ELECTRON_RUN_AS_NODE=1 /snap/code/current/usr/share/code/code companion/test/extension.test.js
else
  printf '%s\n' 'No Node-compatible runner found; JavaScript tests were skipped.' >&2
fi
