# Sucr Capture

The Sucr browser add-on. Manifest V3, Chrome/Edge and Firefox/Zen
(gecko id `capture@sucr.local`, minimum Firefox 128).

It carries a download to Sucr. On every browser download it asks Sucr to take
the link (default), adopt a finished file, or import the bytes the browser has
already written. Nothing leaves the machine: the add-on talks to the app over
loopback with a token the native-messaging host hands it, and it never reads the
cookie store, history or profile.

## Install

Build first: `./build.sh` → `dist/chrome` and `dist/firefox`.

Chromium (Chrome, Edge, Helium, Brave):

1. `chrome://extensions`
2. Developer mode on
3. Load unpacked → `extension/dist/chrome`

Firefox / Zen:

1. `about:debugging#/runtime/this-firefox`
2. Load Temporary Add-on → `extension/dist/firefox/manifest.json`

The app must have run once on this machine: its first real boot writes the host
manifests under `<data-dir>/native/` and the HKCU
`…\NativeMessagingHosts\app.sucr.capture` keys that point at them. After that
the add-on can start the app itself.

## Settings

The popup stores two values in `storage.local`:

- `enabled` — capture on or off
- `mode` — `take` (cancel the browser download, Sucr re-pulls it), `adopt` (let
  the browser finish, then move its file), `import` (pause the browser
  mid-transfer and continue from its bytes)

## Using it

- **Automatically.** With `take` on, a download in the browser is cancelled
  only after Sucr accepts it, and the same bytes land from Sucr.
- **Alt over a link.** Hold Alt while the pointer is on a link and a chip
  appears under it: click it, or just Alt+click the link itself. Sucr is asked
  to fetch it first; if the app cannot (a file behind the page's own session),
  the browser downloads it and Sucr adopts the finished file.
- **Hover media.** Resting the pointer on a picture, video or audio element
  offers it with the same chip, inside frames too. No right-click anywhere.
- **The toolbar popup** shows the connection, recent takes, and the mode.

## Files

| File | What it is |
| --- | --- |
| `manifest.json` | MV3 entry, action popup, icons, pinned Chrome key |
| `manifest.firefox.json` | Same, with the gecko id and background scripts |
| `background.js` | Door discovery, intercept/adopt/import, badge, popup messages |
| `content.js` | The in-page chip: Alt+hover on links, hover on media, Alt+click |
| `popup.html`, `popup.js` | The toolbar popup, four states |
| `tokens.css` | Verbatim copy of the app's tokens (`app/dist/renderer/tokens.css`) |
| `ui.css` | Buttons, popup, in-page panel, refusal notice |
| `icons/` | The octopus at 16, 32, 48 and 128 |
| `preview.html` | Every surface at real size, light and dark |
| `_shots/` | Rendered screenshots and the puppeteer script that makes them |

The popup takes `?state=connected|waking|unregistered|empty` and
`?theme=light|dark`, which is how `preview.html` shows all four states.
