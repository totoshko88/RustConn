#!/bin/bash
# Install RustConn desktop entry, icon, and locale files

set -e

# Determine install prefix
PREFIX="${PREFIX:-$HOME/.local}"

# Install icon
ICON_DIR="$PREFIX/share/icons/hicolor/scalable/apps"
mkdir -p "$ICON_DIR"
cp rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg "$ICON_DIR/"

# Install desktop file
DESKTOP_DIR="$PREFIX/share/applications"
mkdir -p "$DESKTOP_DIR"
cp rustconn/assets/io.github.totoshko88.RustConn.desktop "$DESKTOP_DIR/"

# Install locale files (if compiled .mo files exist)
if [ -d "po" ]; then
    for po_file in po/*.po; do
        [ -f "$po_file" ] || continue
        lang=$(basename "$po_file" .po)
        LOCALE_DIR="$PREFIX/share/locale/$lang/LC_MESSAGES"
        mkdir -p "$LOCALE_DIR"
        if command -v msgfmt &> /dev/null; then
            msgfmt -o "$LOCALE_DIR/rustconn.mo" "$po_file"
            echo "Installed locale: $lang"
        fi
    done
fi

# Update icon cache
if command -v gtk-update-icon-cache &> /dev/null; then
    gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
fi

echo "Desktop entry, icon, and locales installed to $PREFIX"
echo "You may need to log out and log back in for changes to take effect."
