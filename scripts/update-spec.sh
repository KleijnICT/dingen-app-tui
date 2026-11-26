#!/bin/sh
# Refreshes the copy of the conformance test set from the main repository.
# Usage: scripts/update-spec.sh [path to dingen-app]
set -eu
main="${1:-../dingen-app}"
here="$(cd "$(dirname "$0")/.." && pwd)"
rm -rf "$here/spec/conformance"
mkdir -p "$here/spec"
cp -r "$main/spec/conformance" "$here/spec/conformance"
echo "Conformance test set version $(grep -m1 '"version"' "$here/spec/conformance/manifest.json" | tr -dc 0-9)"
