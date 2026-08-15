#!/usr/bin/env sh
set -eu

REPOSITORY_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
INSTALL_PREFIX=${VSPARALLEL_INSTALL_PREFIX:-"${HOME}/.local"}

cd "$REPOSITORY_DIR"
if ! command -v npm >/dev/null 2>&1; then
  printf '%s\n' 'npm is required to compile the TypeScript UI.' >&2
  exit 1
fi
npm run build:ui
cargo build --release --locked --bin vsparallel
install -d \
  "$INSTALL_PREFIX/bin" \
  "$INSTALL_PREFIX/share/applications" \
  "$INSTALL_PREFIX/share/icons/hicolor/16x16/apps" \
  "$INSTALL_PREFIX/share/icons/hicolor/24x24/apps" \
  "$INSTALL_PREFIX/share/icons/hicolor/32x32/apps" \
  "$INSTALL_PREFIX/share/icons/hicolor/48x48/apps" \
  "$INSTALL_PREFIX/share/icons/hicolor/64x64/apps" \
  "$INSTALL_PREFIX/share/icons/hicolor/128x128/apps" \
  "$INSTALL_PREFIX/share/icons/hicolor/256x256/apps" \
  "$INSTALL_PREFIX/share/icons/hicolor/512x512/apps" \
  "$INSTALL_PREFIX/share/doc/vsparallel"
install -m 0755 target/release/vsparallel "$INSTALL_PREFIX/bin/vsparallel"
install -m 0644 \
  LICENSE \
  PRIVACY.md \
  THIRD_PARTY_LICENSES.html \
  "$INSTALL_PREFIX/share/doc/vsparallel/"
install -m 0644 \
  src-tauri/icons/16x16.png \
  "$INSTALL_PREFIX/share/icons/hicolor/16x16/apps/app.vsparallel.png"
install -m 0644 \
  src-tauri/icons/24x24.png \
  "$INSTALL_PREFIX/share/icons/hicolor/24x24/apps/app.vsparallel.png"
install -m 0644 \
  src-tauri/icons/32x32.png \
  "$INSTALL_PREFIX/share/icons/hicolor/32x32/apps/app.vsparallel.png"
install -m 0644 \
  src-tauri/icons/48x48.png \
  "$INSTALL_PREFIX/share/icons/hicolor/48x48/apps/app.vsparallel.png"
install -m 0644 \
  src-tauri/icons/64x64.png \
  "$INSTALL_PREFIX/share/icons/hicolor/64x64/apps/app.vsparallel.png"
install -m 0644 \
  src-tauri/icons/128x128.png \
  "$INSTALL_PREFIX/share/icons/hicolor/128x128/apps/app.vsparallel.png"
install -m 0644 \
  src-tauri/icons/256x256.png \
  "$INSTALL_PREFIX/share/icons/hicolor/256x256/apps/app.vsparallel.png"
install -m 0644 \
  src-tauri/icons/512x512.png \
  "$INSTALL_PREFIX/share/icons/hicolor/512x512/apps/app.vsparallel.png"

DESKTOP_TEMP=$(mktemp)
trap 'rm -f "$DESKTOP_TEMP"' EXIT HUP INT TERM
sed "s|@BINDIR@|$INSTALL_PREFIX/bin|g" packaging/app.vsparallel.desktop.in > "$DESKTOP_TEMP"
install -m 0644 "$DESKTOP_TEMP" "$INSTALL_PREFIX/share/applications/app.vsparallel.desktop"

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$INSTALL_PREFIX/share/icons/hicolor" >/dev/null 2>&1 ||
    printf '%s\n' 'Warning: could not refresh the GTK icon cache.' >&2
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$INSTALL_PREFIX/share/applications" >/dev/null 2>&1 ||
    printf '%s\n' 'Warning: could not refresh the desktop entry cache.' >&2
fi

printf 'Installed VSParallel to %s\n' "$INSTALL_PREFIX/bin/vsparallel"
