#!/usr/bin/env bash
# Writes SHA256SUMS for every file in a folder (default release-out).
set -euo pipefail
dir=${1:-release-out}
cd "$dir"
rm -f SHA256SUMS
if command -v sha256sum >/dev/null; then
  sha256sum -- * > SHA256SUMS.tmp
else
  shasum -a 256 -- * > SHA256SUMS.tmp
fi
mv SHA256SUMS.tmp SHA256SUMS
cat SHA256SUMS
