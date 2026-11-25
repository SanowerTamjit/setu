#!/usr/bin/env bash
# Gathers one platform's build outputs into OUT_DIR with release names.
#
#   collect.sh <version> <platform> <cli-binary> [out-dir]
#
# version   e.g. nightly-20261120-ab12cd3
# platform  e.g. linux-x86_64, macos-universal, windows-x86_64
# The desktop installers are found under target/**/release/bundle; the CLI
# binary is packed with the license and README (zip on Windows, tar.gz
# elsewhere). Names: Setu-<version>-<platform>.<ext>, setu-cli-<version>-<platform>.<ext>
set -euo pipefail

version=$1
platform=$2
cli=$3
out=${4:-release-out}
mkdir -p "$out"

found=0
while IFS= read -r -d '' file; do
  name=$(basename "$file")
  case "$name" in
    *.AppImage) ext=AppImage ;;
    *.deb) ext=deb ;;
    *.rpm) ext=rpm ;;
    *.dmg) ext=dmg ;;
    *.msi) ext=msi ;;
    *-setup.exe) ext=setup.exe ;;
    *) continue ;;
  esac
  cp "$file" "$out/Setu-$version-$platform.$ext"
  echo "desktop: $name -> Setu-$version-$platform.$ext"
  found=$((found + 1))
done < <(find target -path '*/release/bundle/*' -type f -print0)

if [ "$found" -eq 0 ]; then
  echo "error: no desktop installers found under target/**/release/bundle" >&2
  exit 1
fi

stage=$(mktemp -d)
pkg="setu-cli-$version-$platform"
mkdir -p "$stage/$pkg"
cp "$cli" LICENSE README.md "$stage/$pkg/"
cp docs/user/cli.md "$stage/$pkg/CLI.md"
if [[ "$cli" == *.exe ]]; then
  (cd "$stage" && 7z a -tzip -bso0 "$pkg.zip" "$pkg")
  mv "$stage/$pkg.zip" "$out/"
  echo "cli: $out/$pkg.zip"
else
  chmod +x "$stage/$pkg/$(basename "$cli")"
  tar -C "$stage" -czf "$out/$pkg.tar.gz" "$pkg"
  echo "cli: $out/$pkg.tar.gz"
fi
rm -rf "$stage"
