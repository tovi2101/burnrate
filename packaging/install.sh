#!/usr/bin/env sh
set -eu

repo="tovi2101/burnrate"
install_dir="${HOME}/.local/bin"
api="https://api.github.com/repos/${repo}/releases/latest"

asset_url=$(curl -fsSL "$api" | sed -n 's/.*"browser_download_url": "\([^"]*\.AppImage\)".*/\1/p' | head -n 1)
if [ -z "$asset_url" ]; then
  echo "Burnrate: no AppImage found in the latest GitHub release" >&2
  exit 1
fi

mkdir -p "$install_dir"
tmp_file=$(mktemp "${TMPDIR:-/tmp}/burnrate.XXXXXX.AppImage")
trap 'rm -f "$tmp_file"' EXIT HUP INT TERM
curl -fL "$asset_url" -o "$tmp_file"
chmod +x "$tmp_file"
mv "$tmp_file" "$install_dir/burnrate"
trap - EXIT HUP INT TERM
echo "Installed Burnrate to $install_dir/burnrate"
