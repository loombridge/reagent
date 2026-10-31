#!/bin/bash
# Installs the macOS package, checks that everything in the manifest is there
# and the bundles' signatures hold, then runs the uninstaller it installed and
# checks that everything is gone. Needs sudo (CI runners have it).
# Usage: installer/test-macos.sh <package.pkg> <manifest.txt>
set -euo pipefail
package=$1
manifest=$2
failed=0

each_path() {
  while IFS= read -r path; do
    "$@" "$path" || failed=1
  done < "$manifest"
}
is_there() { [ -e "$1" ] || { echo "missing after install: $1"; return 1; }; }
is_gone() { [ ! -e "$1" ] || { echo "left after uninstall: $1"; return 1; }; }

sudo installer -pkg "$package" -target /
each_path is_there
for bundle in /Library/Audio/Plug-Ins/*/reagent.* /Applications/reagent.app; do
  codesign --verify --deep --strict "$bundle" || failed=1
done

sudo "/Library/Application Support/Loombridge/Reagent/uninstall.sh"
each_path is_gone
if pkgutil --pkg-info com.loombridge.reagent > /dev/null 2>&1; then
  echo "the package receipt is still there"
  failed=1
fi
exit "$failed"
