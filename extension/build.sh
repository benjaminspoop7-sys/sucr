#!/usr/bin/env bash
# Packages the add-on for each browser. Chrome/Edge read manifest.json as-is;
# Firefox needs its own manifest and rejects background.service_worker, so the
# same source folder is copied twice and only the manifest differs.
set -euo pipefail
cd "$(dirname "$0")"
out="${1:-dist}"
rm -rf "$out"
files=(background.js content.js popup.html popup.js ui.css tokens.css icons)

mkdir -p "$out/chrome" "$out/firefox"
for f in "${files[@]}"; do
  cp -r "$f" "$out/chrome/"
  cp -r "$f" "$out/firefox/"
done
cp manifest.json "$out/chrome/manifest.json"
cp manifest.firefox.json "$out/firefox/manifest.json"
echo "chrome:  $out/chrome   (drag into chrome://extensions with Developer mode on)"
echo "firefox: $out/firefox  (about:debugging > This Firefox > Load Temporary Add-on > manifest.json)"
