#!/usr/bin/env bash
# Packages the add-on for each browser. Chrome/Edge read manifest.json as-is;
# Firefox needs its own manifest and rejects background.service_worker, so the
# same source folder is copied twice and only the manifest differs.
#
# A third flavour, `dev`, is the same add-on pointed at the development build's
# relay. It exists so a local build can be tested from its own browser profile
# without a single release registration being rewritten: the dev copy asks for
# `app.sucr.capture.dev`, and the installed add-on keeps asking for
# `app.sucr.capture`.
set -euo pipefail
cd "$(dirname "$0")"
out="${1:-dist}"
rm -rf "$out"
files=(background.js content.js popup.html popup.js ui.css tokens.css icons)

dev_host="app.sucr.capture.dev"
if ! grep -q "const HOST = 'app.sucr.capture';" background.js; then
  echo "build: background.js no longer carries the relay constant; refusing to build a dev copy that would point at the installed relay" >&2
  exit 1
fi

mkdir -p "$out/chrome" "$out/firefox"
for f in "${files[@]}"; do
  cp -r "$f" "$out/chrome/"
  cp -r "$f" "$out/firefox/"
done
cp manifest.json "$out/chrome/manifest.json"
cp manifest.firefox.json "$out/firefox/manifest.json"

mkdir -p "$out/dev"
cp -r "$out/chrome" "$out/dev/chrome"
cp -r "$out/firefox" "$out/dev/firefox"
for family in chrome firefox; do
  sed "s/const HOST = 'app.sucr.capture';/const HOST = '$dev_host';/" \
    background.js > "$out/dev/$family/background.js"
  sed -i 's/"name": "Sucr Capture"/"name": "Sucr Capture (dev)"/' "$out/dev/$family/manifest.json"
done
# The dev copy must not be able to stand in for the installed add-on. Without
# the pinned `key` its Chromium ID is derived from the folder it is loaded from,
# so it is a different extension even if both are loaded in one profile; the
# Gecko id is spelled out and moved for the same reason.
sed -i '/^  "key": /d' "$out/dev/chrome/manifest.json"
sed -i 's/"id": "capture@sucr.local"/"id": "capture-dev@sucr.local"/' "$out/dev/firefox/manifest.json"

echo "chrome:  $out/chrome   (drag into chrome://extensions with Developer mode on)"
echo "firefox: $out/firefox  (about:debugging > This Firefox > Load Temporary Add-on > manifest.json)"
echo "dev:     $out/dev      (loaded automatically by scripts/dev-browser.mjs)"
