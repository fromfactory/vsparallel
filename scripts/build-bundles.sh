#!/usr/bin/env sh
set -eu

VSPARALLEL_REPOSITORY_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$VSPARALLEL_REPOSITORY_DIR"

# Host bundle tools must not load GTK/GIO modules injected by VS Code's Snap.
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

# Keep local usernames and checkout paths out of Rust panic-location strings.
if [ -n "${RUSTFLAGS-}" ] && [ -z "${CARGO_ENCODED_RUSTFLAGS-}" ]; then
  printf '%s\n' \
    'RUSTFLAGS is set; convert it to CARGO_ENCODED_RUSTFLAGS before building release bundles.' >&2
  exit 2
fi
VSPARALLEL_FLAG_SEPARATOR=$(printf '\037')
VSPARALLEL_RELEASE_FLAGS=${CARGO_ENCODED_RUSTFLAGS-}
if [ -n "$VSPARALLEL_RELEASE_FLAGS" ]; then
  VSPARALLEL_RELEASE_FLAGS=$VSPARALLEL_RELEASE_FLAGS$VSPARALLEL_FLAG_SEPARATOR
fi
VSPARALLEL_RELEASE_FLAGS=$VSPARALLEL_RELEASE_FLAGS--remap-path-prefix=$VSPARALLEL_REPOSITORY_DIR=/src/vsparallel
if [ -n "${HOME-}" ]; then
  VSPARALLEL_RELEASE_FLAGS=$VSPARALLEL_RELEASE_FLAGS$VSPARALLEL_FLAG_SEPARATOR--remap-path-prefix=$HOME=/build
fi
CARGO_ENCODED_RUSTFLAGS=$VSPARALLEL_RELEASE_FLAGS
export CARGO_ENCODED_RUSTFLAGS

exec cargo tauri build --ci "$@" -- --locked
