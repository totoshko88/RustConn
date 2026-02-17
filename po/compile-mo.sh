#!/bin/bash
# Compile .po files to .mo for local development
#
# Usage: ./po/compile-mo.sh
#
# This creates locale files under build-dir/locale/ so that
# LOCALEDIR=build-dir/locale cargo run
# picks up translations without a full install.
#
# Requires: msgfmt (from gettext package)
# Install: sudo apt install gettext

set -e

DOMAIN="rustconn"
OUT_DIR="${1:-build-dir/locale}"

echo "Compiling .po → .mo into ${OUT_DIR}/ ..."

count=0
for po_file in po/*.po; do
    [ -f "$po_file" ] || continue
    lang=$(basename "$po_file" .po)
    dest="${OUT_DIR}/${lang}/LC_MESSAGES"
    mkdir -p "$dest"
    msgfmt -o "${dest}/${DOMAIN}.mo" "$po_file"
    count=$((count + 1))
done

echo "Compiled ${count} locale(s) into ${OUT_DIR}/"
echo ""
echo "Run with translations:"
echo "  LOCALEDIR=${OUT_DIR} cargo run"
