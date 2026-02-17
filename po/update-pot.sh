#!/bin/bash
# Regenerate rustconn.pot from source files
#
# Usage: ./po/update-pot.sh
#
# Requires: xgettext (from gettext package)
# Install: sudo apt install gettext

set -e

DOMAIN="rustconn"
POTFILE="po/${DOMAIN}.pot"
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')

echo "Extracting translatable strings for ${DOMAIN} v${VERSION}..."

# Extract from Rust source files (gettext/ngettext calls via gettextrs)
# Use --language=C because xgettext doesn't natively support Rust,
# but the gettext("...") call syntax is identical to C.
xgettext \
    --language=C \
    --from-code=UTF-8 \
    --keyword=gettext \
    --keyword=ngettext:1,2 \
    --keyword=pgettext:1c,2 \
    --keyword=npgettext:1c,2,3 \
    --keyword=i18n \
    --keyword=i18n_f \
    --keyword=ni18n:1,2 \
    --keyword=ni18n_f:1,2 \
    --add-comments=Translators \
    --package-name="${DOMAIN}" \
    --package-version="${VERSION}" \
    --msgid-bugs-address="https://github.com/totoshko88/RustConn/issues" \
    --copyright-holder="Anton Isaiev" \
    --output="${POTFILE}" \
    $(find rustconn/src -name '*.rs' -type f | sort)

# Extract from desktop file and merge
xgettext \
    --from-code=UTF-8 \
    --join-existing \
    --output="${POTFILE}" \
    rustconn/assets/io.github.totoshko88.RustConn.desktop

# Extract from metainfo XML (AppStream)
if command -v itstool &> /dev/null; then
    itstool -i /usr/share/its/appdata.its -o "${POTFILE}" \
        rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml 2>/dev/null || true
fi

echo "Generated ${POTFILE}"
echo "Strings: $(grep -c '^msgid ' "${POTFILE}") entries"
