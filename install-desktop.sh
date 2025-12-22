#!/bin/bash
# Install RustConn desktop entry and icon

set -e

# Determine install prefix
PREFIX="${PREFIX:-$HOME/.local}"

# Install icon
ICON_DIR="$PREFIX/share/icons/hicolor/scalable/apps"
mkdir -p "$ICON_DIR"
cp rustconn/assets/icons/hicolor/scalable/apps/org.rustconn.RustConn.svg "$ICON_DIR/"

# Install desktop file
DESKTOP_DIR="$PREFIX/share/applications"
mkdir -p "$DESKTOP_DIR"
cp rustconn/assets/org.rustconn.RustConn.desktop "$DESKTOP_DIR/"

# Update icon cache
if command -v gtk-update-icon-cache &> /dev/null; then
    gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
fi

echo "Desktop entry and icon installed to $PREFIX"
echo "You may need to log out and log back in for changes to take effect."
