#!/bin/bash
set -e

echo "=== amend commit to remove release.sh ==="
git add -A
git commit --amend --no-edit
echo "AMEND: OK"

echo "=== re-tag ==="
git tag -d v0.10.10
git tag -a v0.10.10 -m "Release 0.10.10"
echo "RETAG: OK"

echo "=== push ==="
git push origin main --tags
echo "PUSH: OK"

echo "=== cleanup ==="
rm -f fixup.sh
echo "=== DONE ==="
