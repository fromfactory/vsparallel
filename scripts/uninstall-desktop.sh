#!/usr/bin/env sh
set -eu

INSTALL_PREFIX=${VSPARALLEL_INSTALL_PREFIX:-"${HOME}/.local"}
rm -f -- \
  "$INSTALL_PREFIX/bin/vsparallel" \
  "$INSTALL_PREFIX/share/applications/app.vsparallel.desktop" \
  "$INSTALL_PREFIX/share/icons/hicolor/16x16/apps/app.vsparallel.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/24x24/apps/app.vsparallel.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/32x32/apps/app.vsparallel.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/48x48/apps/app.vsparallel.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/64x64/apps/app.vsparallel.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/128x128/apps/app.vsparallel.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/256x256/apps/app.vsparallel.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/512x512/apps/app.vsparallel.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/16x16/apps/app.vsparallel.desktop.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/24x24/apps/app.vsparallel.desktop.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/32x32/apps/app.vsparallel.desktop.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/48x48/apps/app.vsparallel.desktop.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/64x64/apps/app.vsparallel.desktop.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/128x128/apps/app.vsparallel.desktop.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/256x256/apps/app.vsparallel.desktop.png" \
  "$INSTALL_PREFIX/share/icons/hicolor/512x512/apps/app.vsparallel.desktop.png" \
  "$INSTALL_PREFIX/share/doc/vsparallel/LICENSE" \
  "$INSTALL_PREFIX/share/doc/vsparallel/PRIVACY.md" \
  "$INSTALL_PREFIX/share/doc/vsparallel/THIRD_PARTY_LICENSES.html"
rmdir -- "$INSTALL_PREFIX/share/doc/vsparallel" 2>/dev/null || true
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$INSTALL_PREFIX/share/icons/hicolor" >/dev/null 2>&1 ||
    printf '%s\n' 'Warning: could not refresh the GTK icon cache.' >&2
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$INSTALL_PREFIX/share/applications" >/dev/null 2>&1 ||
    printf '%s\n' 'Warning: could not refresh the desktop entry cache.' >&2
fi
printf 'Removed the VSParallel launcher, desktop entry, and icons from %s\n' "$INSTALL_PREFIX"
