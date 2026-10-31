#!/bin/bash
# Installs a Linux package in a clean container, checks that everything in the
# manifest is there and that the installed libraries and the standalone app
# find every library they link, then removes the package and checks that
# everything is gone. Run it as root in a container with the repository
# mounted: Ubuntu for a .deb, Fedora for an .rpm.
# Usage: installer/test-linux.sh <deb|rpm> <package> <manifest.txt>
set -euo pipefail
kind=$1
package=$(realpath "$2")
manifest=$(realpath "$3")
failed=0

each_path() {
  while IFS= read -r path; do
    "$@" "$path" || failed=1
  done < "$manifest"
}
is_there() { [ -e "$1" ] || { echo "missing after install: $1"; return 1; }; }
is_gone() { [ ! -e "$1" ] || { echo "left after uninstall: $1"; return 1; }; }

case $kind in
  deb)
    apt-get update -qq
    apt-get install -y -qq "$package"
    ;;
  rpm) dnf install -y -q "$package" ;;
esac
each_path is_there

for binary in /usr/bin/reagent /usr/lib/clap/reagent.clap /usr/lib/vst3/reagent.vst3/Contents/*-linux/reagent.so; do
  if ldd "$binary" | grep "not found"; then
    echo "$binary links a library the package doesn't bring"
    failed=1
  fi
done

case $kind in
  deb) apt-get remove -y -qq reagent ;;
  rpm) dnf remove -y -q reagent ;;
esac
each_path is_gone
is_gone /usr/share/loombridge || failed=1
exit "$failed"
