#!/usr/bin/env sh
set -eu

REPOSITORY_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$REPOSITORY_DIR"

if ! command -v npm >/dev/null 2>&1; then
  printf '%s\n' 'npm is required to compile the TypeScript UI.' >&2
  exit 1
fi
npm run build:ui

# The VS Code Snap exports GTK/GDK/GIO lookup paths for the libraries bundled
# inside the Snap. A host-built Tauri binary must use the host modules instead.
if [ -n "${XDG_DATA_DIRS_VSCODE_SNAP_ORIG-}" ]; then
  XDG_DATA_DIRS=$XDG_DATA_DIRS_VSCODE_SNAP_ORIG
  export XDG_DATA_DIRS
  unset \
    GDK_PIXBUF_MODULEDIR \
    GDK_PIXBUF_MODULE_FILE \
    GIO_LAUNCHED_DESKTOP_FILE \
    GIO_LAUNCHED_DESKTOP_FILE_PID \
    GIO_MODULE_DIR \
    GTK_EXE_PREFIX \
    GTK_IM_MODULE_FILE \
    GTK_MODULES \
    GTK_PATH
fi

exec cargo run --locked --bin vsparallel
