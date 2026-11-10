# Sucr Capture — the session ride and the path to "any video/audio format"

Date: 2026-11-09. `writer:` this agent. Trigger: the standing ask to make YouTube
downloads "actually work" for every video/audio format the player offers.

Scope: Sucr's own engine and capture server under `D:\RE\Sucr\app\src-tauri\src`,
plus the browser add-on under `D:\RE\Sucr\extension`. Nothing here changes the
shipped exe; this is a design + evidence note.

## 1. The mechanism a browser-side interceptor has to use

**Predicate:** the design constraint follows from how the browser's own player
legs are authorised, not from any third-party client.

1. **The request that works is the one the browser already made.** A media leg
   carries an attested URL (`pot`/`sig` query values) and is only accepted in the
   context that minted it: the browser's `User-Agent`, `Cookie`, `Referer`,
   `Accept-Encoding` and per-segment `Range`.

2. **So a capture tool must watch the wire, not just clicks.** The extension needs
   `webRequest.onBeforeSendHeaders` (blocking) on the media host to read the real
   outgoing header set for the leg the player is using, plus the exact URL.

3. **Cookies come from the browser's own store.** `cookies.getAll` scoped to the
   media host is the fallback when a leg has not been observed yet. This requires
   the `cookies` permission and a host pattern, and it is a capability the add-on
   must declare, not discover.

4. **The transport from add-on to engine is Sucr's loopback door**, not a native
   port: the add-on already holds a token and posts JSON. Adding the header set to
   that payload is the whole change.

5. **DASH is a manifest, not a format.** A `.mpd` names every representation; a
   client that wants one playable high-resolution file must fetch the chosen
   picture and sound representations and combine them itself, because no single
   representation carries both.

6. **The page grabber does not need per-format parsing of the player object.** It
   can learn formats from the same network legs the player makes, and from one
   same-origin player-API call. Neither route re-signs anything.

**Conclusion.** The thing that makes a high-resolution Take work is a **session
ride**: take the request the trusted browser already made — attested URL plus the
exact `User-Agent`, `Cookie`, `Referer` and `Range` — and have Sucr's engine replay
it. DASH support then means "fetch every representation the browser proved
fetchable, then mux locally."

## 2. Why Sucr's two prior attempts both failed (and were not the right lever)

- **§21 (adopt the URL, let Sucr's cookieless engine fetch it):** the PO-token gate
  binds to the *client context*, not only the URL, so the cookieless engine still
  403s past the free window. Correct diagnosis, wrong tool: any client that fetches
  that URL without the browser's identity fails the same way.
- **§22 (let `chrome.downloads` fetch the captured URL):** play-then-take measured
  `chrome.downloads` of the captured attested URL = **0 bytes** (`SERVER_FORBIDDEN` /
  `NETWORK_FAILED`) — a downloader full-GET is a *different* request context than the
  player's ranged GET. Reverted in §23.

Neither attempt forwarded the browser's **headers+UA+cookies**. That forwarding —
not URL reuse, not a re-download — is the actual trick.

## 3. What Sucr already has (so this is cheaper than a cold build)

`writer:` this agent; `predicate:` read of the engine source.

- The engine **already replays a per-transfer `Cookie` + `Referer` + custom
  `User-Agent` on every leg.** `engine/mod.rs:246 probe(agent,url,referer,cookie)`
  sends `Referer` and `Cookie`; the agent is built from `config_ua`
  (`mod.rs:168`, the CF `cf_clearance` UA-binding path); `job.rs:635-636` reads
  `ua`+`cookie` from the host session and `job.rs:3533` puts `probe.cookie` on the
  range workers; `job.rs:3943` `agent_session(ua)`.
- The host session store exists: `db.rs:773 session_for(host) -> {cookie,user_agent}`,
  written by `db.rs:813 save_session(host,cookie,ua)`.
- The adaptive ladder is already grabbed page-side (§20 `ytRowFromPage`, ANDROID
  innertube → per-itag url + contentLength).

**The gap is only input + one capability, not the fetch engine:**

- **Gap A — nothing feeds the browser's live values.** `save_session` is reachable
  only from the Settings/pasted surface, NOT from the loopback capture server
  (`capture.rs:246-258` has no session route; `save_session` is called nowhere in
  `capture.rs`/`ipc.rs`). And `/add` (`capture.rs:399`) accepts only
  `url, filename, referer, folder` — no `cookie`, no `ua`. So the extension has no
  way to hand the engine the browser's live UA/cookie/headers at all.
- **Gap B — no DASH muxer.** No `dash`/`mpd`/`segment`/`mux`/`merge` anywhere in
  the engine. Sucr can download each itag row but cannot combine a video-only row
  with its audio row into one file, so "high-res A/V as one playable file" is out.

## 4. The solution, as two independent legs

### Leg A — session ride (fixes "stops partway" on gated adaptive; parity for a single tier)

The same principle, using Sucr's existing plumbing:

1. **Extension** re-adds a *header capture*, but this time the right one:
   `chrome.webRequest.onBeforeSendHeaders` (blocking) on `*://*.googlevideo.com/*`
   reads the browser's real `Cookie`, `User-Agent`, `Referer`, `Range`,
   `Accept-Encoding` for the leg the player is using, and the exact attested URL
   (`pot`/`sig` intact). Plus `cookies.getAll` on `.youtube.com` as a cookie
   fallback. (Permissions to restore: `webRequest`; add `cookies`.) 0 exe bytes.
2. **Hand-off:** two implementations —
   - **A1 (smaller, reuses store):** a new loopback route `POST /session` that calls
     the existing `save_session(host,cookie,ua)`; the extension posts the captured
     values for `googlevideo.com`, then `POST /add` with the live attested URL. The
     engine's already-working range replay rides the session. Cost ≈ the IPC door
     (~1 KB) + the route body.
   - **A2 (cleaner, per-download):** extend `/add` to accept optional `cookie` +
     `ua`, thread them to the job instead of the host store, so no host session is
     mutated. Cost ≈ the `add` body + threading through `job.rs` signatures.
   Either way the download fetch layer is unchanged — only the input path is new.
3. **Caveat, honest (the user's bench):** Leg A's *mechanism* is sound — it replays
   exactly what the browser already proved fetchable — but whether it completes a
   *closed-gate* web-player row cannot be proven on this box — this automation profile
   is bot-trimmed and has no warm signed-in session that emits a gated itag-bearing
   web-player URL to capture. The single test is the user's own browser: reload the
   card, play at the target quality, Take it; a gated row that stalled at ~2% should
   now complete.

### Leg B — "any video/audio format" = DASH adaptive + mux

To offer arbitrary high tiers (1080p/1440p/2160p, video-only) as a single A/V file:

1. Grab the video-only row **and** its matching audio row via Leg A's session ride
   (both rows come from the existing ladder grabber).
2. **Mux** them into one container. Sucr has no muxer. Options, cost-ordered:
   - **B1 (recommended):** a small pure-Rust fMP4→MP4 mux (e.g. the `mp4`/`mp4muxer`
     family, or hand-rolled `moov` assembly from the two init+media segments).
     YouTube's higher tiers are fragmented MP4, so this is an init-segment merge +
     `moov` rewrite — no re-encode, cheap. **Must measure** against the 5 MB ceiling
     (headroom was 578 560 B); a container-only muxer is usually a few tens of KB.
   - **B2:** bundle ffmpeg — rejected, blows the ceiling by orders of magnitude.
   - **B3 (cheapest, honest fallback):** ship the two rows as separate files
     (`.mp4` video + `.m4a` audio) with no merge. Zero mux cost, but not one
     playable file. Good enough to prove the bytes arrive; merging is then a later
     slice.

## 5. Recommendation and the decision owed

- **Leg A is the direct fix for the user's actual complaint** (a download that
  stops partway), reuses Sucr's working header/cookie/UA replay, and is a faithful
  session ride. It costs exe bytes and touches
  the shared tree (A1 or A2), so it is the user's `DECIDED`, not taken here.
- **Leg A1** is the lowest-risk build (new route + reuse `save_session`);
  **A2** avoids mutating the host session store for the browser's live cookie.
- **Leg B** is a separate, larger slice; **B3** first (prove both rows arrive via
  Leg A), then **B1** for a single merged file if the muxer fits the byte budget.
- **Standing rules honoured:** no PO-token/BotGuard attestation is forged — only
  the user's own already-granted session values are replayed, nothing is re-signed;
  the exe-byte + shared-tree edits stay behind `DECIDED`; `stat` gates size, never a
  hash.

**Gate:** this session spent **0 exe bytes** (`stat` = 4 665 344 B unchanged); it
only read Sucr's own source. **Owed:** the user's
`DECIDED` on which leg(s) to build (A1 / A2 / B3 / B1) and the reload of the Sucr
Capture card to pick up the reverted `dist/chrome` from §23.

## 6. Shipped: Leg A + Leg B1 — one merged file

Date: 2026-11-09. `writer:` this agent. Trigger: the user's `DECIDED` scope
"Leg A + Leg B1 (one merged file)". Both halves are built; the exe-byte cost is
measured with `stat`, never a hash.

**Leg A — the session ride (verified prior session, closed this session).** The
browser's live Cookie/UA/Referer (and its attested googlevideo stream URL) ride to
Sucr's engine, which replays that identity so a gated adaptive row completes
instead of stalling at the ~12-16 MiB free window. This is the session ride
described in §1. Engine + extension wiring is in place; the live gated-row
completion is the user's real-browser bench (`predicate:` not reproducible on this
bot-trim box — the PO-token gate 403s here).

**Leg B1 — a pure-Rust fMP4→MP4 muxer + a DASH pair→one-file orchestration.**
`evidence-noun:` code + tests on disk, exe re-measured.

- `engine/mux.rs` (`writer:` this agent): a container-only remuxer, no new crate.
  It parses each source's fragmented MP4 (`ftyp`+`moov`+repeated `moof`/`mdat`),
  descends `traf`→`tfhd`/`trun` (honouring `default_base_is_moof`, per-sample
  flags, `depends_on==0` sync, composition offsets), collects `Samp{dur,size,cto,
  sync,off}`, then writes a **non-fragmented** `ftyp`+`moov`+`mdat`: one contiguous
  chunk per track (`stsc` 1 entry, single `stco`), a two-pass `moov` (probe length,
  recompute absolute `stco` offsets, rebuild — only 4-byte `stco` values move, so
  the length is stable), `ctts` v1 (signed) only when any offset ≠ 0, `stss` only
  when sync is mixed. `stream_samples` seeks/skips between non-contiguous sample
  offsets and copies each sample's bytes.
- `capture.rs` `("POST","/mux")`: reads `{videoPath, audioPath?, outPath}`,
  resolves each source through `resolve_written` (absolute + on disk), requires
  an absolute `outPath`, and **refuses an output equal to a source** — the muxer
  truncates the destination before it streams the picture, so a collision would
  destroy the input. This guard is defense-in-depth; the extension's `mergedPath`
  is written so it can never hand back the picture path unchanged.
- `ipc.rs` `final_path`: each status row (`download_list` and `download_get`, the
  latter behind `/status`) now carries `"path"` = folder-or-downloads_dir joined
  with `filename`, so the add-on can name finished files to `/mux` without
  guessing a separator or a null folder.
- `extension/content.js`: the ladder keeps `muxed` + `ext` + `isVideo` on each row;
  a video-only **MP4** Take is paired with the best MP4/M4A audio (`pairAudio`) and
  sent as `op:'take-mux'`. A webm picture, or one already carrying audio, is not a
  muxer candidate and takes alone (honest scope: B1 joins MP4 containers only).
- `extension/background.js`: `takeMux` takes both legs through the engine (never
  probes — a cookieless probe would only see the free window and hand the pair to
  the browser, which the ride exists to avoid), opens the panel on the picture leg,
  then `finishMux` polls both `/status` to finished, reads each `path`, derives
  `outPath` with `mergedPath`, POSTs `/mux`, POSTs `/adopt` so the merged file
  enters the library as one finished row, and re-points the panel at it.

**What is proven here (evidence-noun: tests + oracle on this box).** The muxer
byte-correctness: the committed fixtures (`tests/data/{a,v}_{only,multi}.mp4`,
ffmpeg-generated) merge to a file `ffmpeg -v error -i out -f null -` decodes with
exit 0 and no errors, and `ffprobe` reports both tracks (h264 75 frames / 5.0 s,
aac 217 frames / 5.02 s), including multi-fragment reassembly (5 `moof` → 1
continuous track each). Unit tests in `engine/mux/tests.rs` assert container
(`ftyp`/`moov`/`mdat`), the vide+soun handler pair, the exact per-track sample
counts, and that `mdat`'s box size equals the file tail. Engine suite:
**230 passed / 0 failed / 7 ignored**, no warnings (`final_path` is referenced).

**What is NOT proven here (the user's real-browser bench).** That a gated adaptive
video+audio pair actually completes over the wire via the ride in a live browser
and lands as one playable file. This box is bot-trimmed (Google PO-token gate), so
the two network legs cannot be exercised locally. The orchestration, the `/mux`
route, the `/status.path` field and the collision guard are correct against the
loopback contract; the live pair→merge leg is owed by the user.

**Byte gate.** `stat` after the full Leg B1 engine half (muxer + `/mux` + tests +
`path` field + collision guard): **4 685 824 B**, headroom **557 056 B** under the
5 242 880 B ceiling. The collision-guard + `path` wiring moved the binary +512 B
(one quantum); the extension JS is 0 exe bytes.

**Standing rules honoured:** no attestation forged — only the user's own session
values replayed; `stat` gates size; the shared tree
(`app` + `extension`) was re-read before each edit to survive concurrent edits.
**Owed by the user:** reload the Sucr Capture card to pick up the rebuilt
`dist/chrome`/`dist/firefox`, and run the one real-browser leg (a high-res YouTube
Take → watch both legs finish → one merged `.mp4`).

## 7. Option B — squeeze the logged-in session (extension-only, 0 exe bytes)

Date: 2026-11-09. `writer:` this agent. Trigger: the user asked "why not just use
the desktop website grabbing", then chose **Option B** (keep it extension-only and
squeeze the logged-in-session path) over an app-sidecar yt-dlp route (rejected:
yt-dlp is Python and cannot run in an MV3 extension; an app sidecar breaks the 5 MB
ceiling and only clears the gate with its separate PO-token provider).

**Root cause the desktop question surfaced.** `predicate:` read of `offer()`→
`take()`→`rideFor()` in `extension/background.js` + `youtubeCandidates()` in
`content.js`. The session ride only carried an identity when the browser had made
a `*.googlevideo.com/*` request whose URL held a matching `id=` (`onBeforeSendHeaders`,
the only source of `ua`/`cookie`). Under **SABR** the web player may make no such
classic `/videoplayback?id=` leg, so `rideHeaders` stayed empty → `rideFor()`
returned null → the Take fell back to the engine's **cookieless, default-UA** pull →
read the ~12-16 MiB free window → 403 → `block-failed` at ~2%. The protected-row
label had the same flaw: `case 'probe'` fetched **cookieless**, so the ladder could
mislabel a row the riding engine would actually finish.

**Why the desktop-client idea cannot itself remove the wall (recorded so it is not
re-lit).** `predicate:` `content.js:662-664` — the watch page's own player response
holds **no** format URLs and the on-wire videoplayback URLs carry no per-format query
(SABR). So there is nothing on the desktop player to lift-and-replay; the ANDROID
innertube call is used precisely because it is the only endpoint still returning a
plain per-format ladder. The gate is server-side Proof-of-Origin on googlevideo,
keyed to attestation, not to which client minted the URL — switching clients narrows
the ladder, it does not open the gate. yt-dlp clears it only with its
`bgutil-ytdlp-pot-provider`; bare yt-dlp hits the same 403.

**What shipped this slice (all `evidence-noun:` code on disk, `node --check` clean,
exe `stat` = 4,685,824 B unchanged → 0 exe bytes):**

1. **Whole-session ride fallback.** A `lastGoogle` record now keeps the freshest
   `{ua,cookie,referer,at}` from ANY googlevideo leg. `rideFor(url)` returns an
   exact-`id` capture, else this whole-session capture (tagged `src:'last'`), within
   the 120 s TTL. One browser signs one googlevideo domain, so for a same-page format
   whose `id` never appeared on the wire, the freshest leg is a faithful stand-in.
   Scoped by `onGoogle(url)` so the YouTube cookie is **never** attached to a
   non-googlevideo Take and no session note mislabels a plain file.
2. **UA coherence.** `content.js` sends `navigator.userAgent` on `op:'take'`,
   `op:'take-mux'`, `op:'probe'`; `take()` rides the sniffed UA if present, else the
   page's own desktop UA. The token is minted by a same-origin desktop fetch, so
   replaying with that desktop UA is more coherent than the engine's default.
3. **Ride-true labels.** `case 'probe'` now fetches each range under the same
   identity the engine will ride (page UA + any fresh googlevideo cookie), so a row
   the riding engine can finish is no longer marked `protected`. **This is the honest
   self-report:** the ladder labels now predict the outcome under the real pull path,
   which answers the user's "I do not want to keep checking one by one."
4. **Session-ride note.** `take()` tags the result `ride:'id'|'last'|'none'`;
   `openPanel(...)` passes it and the panel appends `· riding your browser session`
   only when a ride engaged. Shows and the row still stalls → Google's gate, not a
   missing session. Never shows → the session was not found (play first / sign in).

**What is NOT proven here (the user's signed-in bench).** `predicate:` this box is
bot-trimmed — no warm signed-in session emits a gated itag-bearing web-player leg, so
whether a *gated* adaptive row now completes is **not** reproducible locally. This
slice removes the mechanical reasons it failed before (empty ride, wrong UA, cookieless
labels); it does not mint a PO token. A gated row whose PO binds to attestation rather
than headers can still 403 with the session riding — that is Google's ceiling.

**Owed by the user:** reload the Helium Sucr Capture card to pick up the rebuilt
`dist/chrome`, be **signed into YouTube**, play the target quality, then Take a high-res
row and read the panel: (a) does `· riding your browser session` appear, (b) does the
row finish or stall, (c) is it labelled protected in the ladder. That one test is now
informative by construction.

## 8. The desktop-context pull + direct-URL leg (2026-11-10)

`writer:` extension `content.js` / `background.js`; `predicate:` the ladder rows and their
`protected` labels; `evidence-noun:` the live job row in `%APPDATA%\Sucr\sucr.db` and the
ladder dump.

**What changed.** The ladder no longer asks for a mobile client. `background.js` now runs
one page-world injection (`world:'MAIN'`) and makes two same-origin `/youtubei/v1/player`
calls with the page's own `ytcfg` context:

1. the **desktop (WEB)** context — answers `status: OK` with every format the desktop
   player offers, but only the progressive muxed row carries `url`; adaptive rows are
   SABR ids;
2. the same context with the client fields swapped to **visionOS** (`1.02`,
   `Apple/RealityDevice17,1`, `visionOS 26.5.23O471`) and the page's `visitorData` in
   both `context.client.visitorData` and `X-Goog-Visitor-Id` — every adaptive itag comes
   back with a direct URL.

Rows are built from the desktop response and the URL leg fills each itag. The
`yt-ladders` message op returns the pair; `ytRowFromPage` (retry watcher) uses the same
visionOS request. The content script is in an isolated world, so `window.ytcfg` is
unreachable there — the page-world injection is not optional.

**Measured on `aqz-KE-bpKQ` (2026-11-10).** Bare visionOS returns `LOGIN_REQUIRED`
("Sign in to confirm you're not a bot") without the page's `visitorData`; with it:
`http 200`, `OK`, 32 adaptive rows all with URLs. Range probes of itag 315
(`clen 1 362 269 481`) answered **206 at offsets 0, 13 000 000 and 1 300 000 000**.
The shipped ladder then rendered **26 rows (22 video + 4 audio), 0 labelled protected**,
2160p60 VP9 first at `1.3 GB · itag 315` (screenshot `ladder-vision.png`, temp).

**End-to-end in the shipped engine.** A Take on that row produced download id 1
`Big Buck Bunny …-2160p60-vp9-315.webm`, `total_bytes 1 362 269 481`, `c=VISIONOS` in the
stored URL, and passed 1.09 GB — i.e. **the 12 MiB wall does not apply to this client**.
The wall documented in §2 was a property of the ANDROID client request, not of Sucr.

**Caveats.** The desktop response varies by moment: one call returned 1 progressive row
(itag 18), a later call `UNPLAYABLE` and 0 progressive rows — so a muxed row may be absent
from the ladder; when it is absent the list is adaptive-only. The extension must be
reloaded in the browser to pick this up (`dist/chrome`, extension version 0.1.1).

## 9. One Take, one file — container-copy joins for every video row (2026-11-10)

`writer:` extension `content.js` / `background.js`, engine `engine/webm.rs`,
`capture.rs`; `predicate:` the ladder row's click path and the merged file on disk;
`evidence-noun:` the merged file's own stream table (ffprobe) plus `stsz`/EBML
`Tracks` read back from the written file, and the job rows that remain afterwards.

**Rule.** The whole ladder is still listed and every video itag still has a Take, but a
Take on a *video-only* row now downloads its sound leg too and lands as ONE file. Audio
rows stay deliberate standalone audio downloads; a muxed row (itag 18/22) needs no join.

**Pairing (`pairAudio`, content.js).** A video-only row pairs with the best audio row of
its own container family — `mp4` (H.264/AV1) with `m4a`, `webm` (VP9) with `webm` — because
a container copy cannot cross families. `cands` is already bitrate-sorted, so the first
match is the best. A row that already carries its audio is taken alone.

**Both remuxers, picked by the output extension (`/mux`, capture.rs).**
- `.mp4` → `engine::mux` (the fMP4→MP4 muxer from Leg B1);
- `.webm` → `engine::webm`, new: EBML walk of both sources, a rebuilt `Info`
  (TimestampScale + Duration = the longer leg) and `Tracks` (two `TrackEntry`, numbered
  1 and 2, distinct `TrackUID`s), every Cluster copied verbatim, and only each audio
  block's track-number byte rewritten. Clusters are interleaved by `Timecode`, picture
  first on ties. No re-encode, no crate, no ffmpeg.

**Names.** `mergedPath` strips the ladder tail (`-<tier>-<codec>-<itag>`, with any
` (n)`) and keeps the title, so a 2160p row lands as `Title.webm` and the picture leg is
never handed back as its own output (both muxers truncate the output first). `/mux`
grows a ` (n)` between stem and extension when the name is taken (`unique_out`) and
returns the path it actually wrote; the extension adopts *that* path.

**One file in the library.** After a successful `/adopt`, `finishMux` posts the two leg
ids to the new `POST /drop`, which reuses `ipc::download_remove` (waits for the row to go
idle, deletes the row and its file). A refused adopt keeps both legs.

**Watched rows.** A `Watch` on a protected video row enrolls its sound itag beside the
picture (`pairItag`/`pairFilename` in the retry job). When the gate opens, `runRetry`
takes the picture, resolves the sound fresh from the page, takes it, and runs the same
`finishMux`.

**Proven (hermetic + real scale).** Four tests over two WebM fixtures (two tracks numbered
1/2, every cluster of both sources kept, a non-WebM source refused, a disagreeing
TimestampScale refused) plus the ignored ffprobe leg, which now accepts
`SUCR_WEBM_BENCH=<video>;<audio>;<out>`. Run against the real 4K capture
(`sec-315.webm` 101 447 017 B + `audio251.webm`), the Rust muxer wrote
102 190 380 B — the same size the Node prototype produced — and ffprobe/ffmpeg READ it as
VP9 3840×2160 + Opus 48 kHz, duration 40.008 s, **0 decode errors, 2 400 video and
2 001 audio packets**, identical to the sources. Byte cost: exe **4 700 160 B**
(+14 336 B), headroom 542 720 B under the 5 MB ceiling.

**Not yet run.** The live browser Take on a real page with the new binary — the running
`sucr.exe` is the previous image, so the door has no `/drop` until the app is restarted;
the extension needs a reload for 0.1.2.

**Superseded by §10 (2026-11-10).** The user rejected the shape, not the join: two rows
appearing in the library for one Take is exactly what they said they did not want
("The extension downloads 2 different parts. Audio, and video. I do not want that to
happen"). The `/adopt` + `/drop` pair still exists for other callers, but a Take never
uses it now.

## 10. One Take, one row — the engine fetches both streams (2026-11-10)

`writer:` engine `engine/job.rs` (`combined`, `join_streams`, `finish_merged`,
`reset_leg`), `schema.sql` + `db.rs` (`audio_url`, `part_path`), `ipc::download_add`,
`capture.rs /add`; extension `content.js` + `background.js` 0.1.3;
`predicate:` the rows in `downloads` and the files in the save folder after one Take;
`evidence-noun:` one `downloads` row whose `total_bytes` = the merged file's length,
one file on disk read back by ffprobe as two streams, no `*.sucrpart(.v)` left.

**Rule.** A Take on a video-only row is ONE `/add` carrying `audioUrl`. The engine
fetches the picture, keeps it at `<name>.<id>.sucrpart.v`, resets the leg, fetches the
sound under the same row with progress based at the picture's total, joins them with the
container-copy muxer `§9` already has, deletes both parts, and closes the row as one
finished file. One progress bar, one library entry, one file.

**Why the row, not two calls.** Two `/add`s cannot be hidden from the library without a
second destructive API (`/drop`) and a gap where both rows exist; and a worker-side
`/mux` needs both legs to finish before it can even start, so the panel lies about which
file is being fetched. Folding it into the job means the engine owns the whole
transaction: cancellation stops whichever leg is running, and a failure never publishes
a half file.

**Resume, per leg.** `part_path` marks a *proven* picture (length checked before the
rename). On a restart with that file present, the picture is not fetched again; the row's
staging path, validators and block plan are the *sound's* and resume on their own. A
restart during the picture resumes it from its own staging file (no `part_path` yet).
`reset_leg` clears blocks + transfer columns between legs and leaves `part_path` alone.

**Schema without a version bump.** `downloads.audio_url` and `downloads.part_path` are
added to `schema.sql` and to `repair_columns()`, which runs on every open and checks
`pragma_table_info` — the same route `ua`/`cookie` took.

**Names.** `mergedName()` strips the ladder tail in `content.js` and again in
`background.js` for watched rows, so the one file is `Title.mp4` / `Title.webm`;
`unique_path` gives `Title (1).mp4` when the name is taken.

**Watch.** `runRetry` resolves the sound row fresh from the page at the tick the gate
opens and takes picture + sound in one `audioUrl` call. A missing sound leaves the row
watched (a silent file is never the outcome); a missing sound *format* drops the watch.

**Proven.**
- `one_row_takes_a_dash_pair_and_publishes_the_joined_file` — real fMP4 fixtures over a
  loopback Range server: one row, one file, `ftyp` first, both `vide` and `soun` tracks,
  the parts folder holds exactly one file, `total_bytes` = `verified_bytes` = the merged
  length, `part_path`/`staging_path` NULL.
- `a_failed_sound_leg_keeps_the_picture_for_the_next_run` — sound point at a dead port:
  the picture stays on disk, and the next run with a live sound finishes with exactly one
  `pair-picture` event (the picture is not fetched twice).
- Suite: **236 passed / 0 failed / 8 ignored** (`cargo test --release
  -- --test-threads=1`).
- Launched binary (`--data-dir` harness, door driven with the add-on's own headers):
  `POST /add {url, audioUrl, filename}` → 200 with one id; `/status` → `finished`,
  `totalBytes` = `verifiedBytes` = 73 289; the save folder held exactly `one-take.mp4`;
  ffprobe read it as **h264 video + aac audio**; `select count(*) from downloads` = 1.

**Byte cost.** exe **4 708 864 B** (+8 704 B over §9's image), headroom **534 016 B**
under the 5 MB ceiling. Extension: 0 bytes of exe, dist rebuilt at 0.1.3.

**Owed.** The live browser Take on youtube.com with 0.1.3 loaded — the in-page probe
labels and the panel's ride note are unchanged by this slice, but nothing here has been
seen on a real gated row yet.
