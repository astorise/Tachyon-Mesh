#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 5 ]; then
  echo 'Usage: stage-linux-desktop-release.sh VERSION TAG DEB APPIMAGE OUTPUT_DIR' >&2
  exit 2
fi
version=$1
tag=$2
package=$3
appimage=$4
output_dir=$5

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]
test "$tag" = "v${version}"
[[ "$package" == *.deb && "$appimage" == *.AppImage ]]
test -f "$package"
test -f "$appimage"
expected=$version
if [[ "$version" == *-* ]]; then
  expected=${version/-/\~}
fi
test "$(dpkg-deb --field "$package" Version)" = "$expected"
mkdir -p "$output_dir"
deb="$output_dir/tachyon-studio_${version}_amd64.deb"
image="$output_dir/tachyon-studio_${version}_amd64.AppImage"
cp -- "$package" "$deb"
cp -- "$appimage" "$image"
printf 'deb=%s\nimage=%s\n' "$deb" "$image"
