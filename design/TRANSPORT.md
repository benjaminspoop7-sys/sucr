# Sucr — TRANSPORT.md

**Status: measured 2026-11-03, not ratified.** Nothing here is `DECIDED`. Per the standing rule for
this project, only the user can promote an entry to `DECIDED`; this file records what was measured
and what it therefore rules out. **Rung: 3 — AS-IS + OBSERVED**, same ladder as `MOTION.md`.

It exists because the question "should Sucr speak HTTP/3 over QUIC?" had been answered twice from
inference — once by a memory of what a QUIC stack costs, once by a claim about the Windows OS
implementation that turned out to be false. Both numbers are measured here instead, in a throwaway
crate, at zero cost to the product.

## 0. The claim, and what replaced it

| | before | after |
|---|---|---|
| Cost of a QUIC stack | inferred "roughly a megabyte" | **measured** 1,308,160 B of new code over what is already linked |
| Windows has platform QUIC | **asserted, and wrong** | absent on this build, proven four ways (§4) |
| tokio would have to be added | asserted | **wrong** — tokio 1.54.1 is already compiled into `sucr.exe` |
| h3 is faster | inferred from protocol lore | **measured**: a wash on bulk (−2.8%), −20% on cold connection cost |

## 1. Method

Two scratch crates, `Temp/quic-probe` and `Temp/h1-probe`, both deleted after the numbers were
taken. Neither touched the app: no product file changed, no product dependency was added, and the
probe binaries were measured in their own directory.

- **Profile copied verbatim** from `src-tauri/Cargo.toml` — `opt-level = "z"`, `lto = true`,
  `codegen-units = 1`, `panic = "abort"`, `strip = true` — so the subtraction is honest. A debug
  or default-profile probe would have overstated the cost by an order of magnitude.
- **Subtraction baselines**, not totals: `base` (empty `main`) and `tokio_base` (runtime plus
  socket plumbing, no TLS, no protocol). Every figure below is a difference, because `sucr.exe`
  already pays for the base.
- **The h1 comparator is Sucr-exact**, in an isolated graph pinned to `ureq =3.5.0`, with the real
  configuration from `engine/mod.rs:130` (`config_ua`): `TlsProvider::NativeTls`,
  `RootCerts::PlatformVerifier`, `timeout_connect` 10 s, `max_redirects` 10,
  `http_status_as_error(false)`. Omitting the provider is not a harmless shortcut — ureq defaults
  to Rustls, which is not compiled in, and the probe panicked on the first HTTPS request exactly
  like the comment at `engine/mod.rs:137` warns.
- **Stack measured, not assumed**: `quinn 0.11.12` (default-features off, `rustls-ring` +
  `runtime-tokio`), `rustls 0.23.46` with the ring provider (aws-lc avoided on Windows MSVC),
  `h3 0.0.8`, `h3-quinn 0.0.10`, `rustls-native-certs 0.8.5`, `http 1`, `bytes 1`, `futures 0.3`,
  `tokio`, `socket2 0.6`.

## 2. Size — the load-bearing number

Stripped bytes of each probe binary:

| binary | contents | bytes |
|---|---|---|
| `base` | empty `main` | 109,568 |
| `tokio_base` | + tokio runtime, socket and TLS plumbing | 232,960 |
| `h1-probe` | + ureq 3.5.0 and native-tls (Sucr's stack) | 545,792 |
| `quinn_raw` | + quinn and rustls/ring, transport only | 1,386,496 |
| `h3_get` | + h3 and h3-quinn, a working HTTP/3 client | **1,541,120** |

Derived, against what `sucr.exe` (4,477,952 B) already links:

- the ureq stack costs **436,224 B** over the base;
- QUIC plus its own TLS provider costs **1,276,928 B**;
- the h3 layer alone is **154,624 B**;
- **new code over already-linked tokio = 1,308,160 B** (`h3_get` − `tokio_base`) — this is the
  realistic figure, because a bonded client cannot delete the h1 path: the server has to be found
  to have h3 first, and `alt-svc` is only ever a hint;
- **best-case net = 871,936 B**, and that case is one the app cannot build: it assumes h3 replaces
  ureq entirely with no fallback and no per-route binding;
- feature trimming is not a lever: quinn's defaults off vs on is **7,168 B**.

Against the 30,924 B left under the 4,508,876 B ceiling, that is **28× the budget at best, 42×
realistically**. Two more links would have to be freed before this is arguable, not one.

## 3. Performance — one real win, and it is mostly moot

Same box, same link, same target family, h1 and h3 legs interleaved to average out drift.

| test | h1 | h3 | read |
|---|---|---|---|
| A — 1.33 MB bulk, cold connection, ×6 | median **0.392 s** | median **0.381 s** | +2.8%, one h3 outlier at 0.588 s: **a wash** |
| C — connection cost only, 0-byte `generate_204`, ×6 | 0.105–0.119, median **0.1125 s** | 0.087–0.100, median **0.0895 s** | **−20%**, distributions non-overlapping ≈ **23 ms per cold connection** |
| D — 16 in parallel | 0.319 s (16 connections) | 0.310 s (1 connection, 16 streams) | **not usable** — see below |

Test D is confounded and is recorded as confounded: ureq sends `Accept-Encoding: gzip` and the h3
client did not, so the two legs moved 1,429,280 B and 1,496,422 B. Byte counts differ by 4.7%,
which is the same order as the "win", so no throughput claim is made from it. A/B were run with
matched byte counts.

Not measurable on this box, and not claimed: **head-of-line blocking under loss** (no traffic
shaper here), **0-RTT resume**, **connection migration**. Those three are the properties that make
QUIC different, not merely faster — and two of them are the reason one would actually want it.

Why the 23 ms is mostly moot for Sucr: `engine/bond.rs:62-65` gives every route its own agent, so
ureq's socket pool is per route and a connection is reused across segments. Handshakes are
once-per-route, not once-per-segment, so a per-connection saving of 23 ms is billed a handful of
times per download, not thousands.

## 4. The Windows route, closed by probing rather than by documentation

WinHTTP in windows-sys 0.61.2 does expose `WINHTTP_OPTION_ENABLE_HTTP_PROTOCOL`,
`WINHTTP_PROTOCOL_FLAG_HTTP3`, four `WINHTTP_OPTION_HTTP3_*` timeouts and
`WINHTTP_OPTION_QUIC_STATS`, and the docs for the RTT option say it configures **msquic**. On this
install (10.0.26200) the engine is not there:

- `quic.dll`, `msquic.dll`, `Windows.Networking.Quic*`: **no such files** in System32 — a recursive
  sweep returned only `ChtQuickDS.dll` and `QuickActionsDataModel.dll`, neither of which is QUIC;
- `winhttp.dll` (1,220,408 B): **zero** `quic` strings, and no `h3` ALPN token;
- no `msquic.h` anywhere in Windows Kit `10.0.26100.0`;
- windows-sys 0.61.2 exposes **no** networking-QUIC feature module at all, so the API cannot even
  be named from this crate's metadata (`Win32::Networking` has no `Quic` subtree).

A discarded false positive, recorded so it is not repeated: a `grep` for `H3` in `winhttp.dll`
matched `L$(H3`. Those are x86 opcode bytes, not a protocol name. Binary string sweeps need a
word boundary that a raw dump does not give.

Even if msquic were present, WinHTTP has **no option to bind a source address or choose an
interface**, so the OS path cannot carry per-NIC bonding — the one thing that makes Sucr's transfer
layer Sucr's. It would be a second, dumber transport, not a replacement.

## 5. The structural blocker, independent of size

Bonding is not a socket `bind()` bolted onto a client. It is implemented inside ureq's
`unversioned::transport` API — `Connector`, `Transport`, `Buffers`, `LazyBuffers`, `Either`,
`NextTimeout` — assembled at `engine/bond.rs:78-83` through `Agent::with_parts(config,
BoundConnector { local_ip }.chain(NativeTlsConnector::default()), DefaultResolver::default())`.
So "add h3" is not "add a second HTTP client"; it is **write a second custom transport** against
quinn's `SendStream`/`RecvStream` and re-implement the same pooling, timeout and TLS-trust
guarantees `config_ua` already gives the h1 path. The unbonded and bonded cases then have to be
kept byte-identical in `job::attribute_egress` and the Sucker Map's colour attribution, which is a
correctness surface, not a feature.

## 6. Verdict

**Do not do HTTP/3.** The size argument is no longer an estimate: 871,936 B at best, 1,308,160 B
realistically, against 30,924 B of headroom. The speed argument does not rescue it — bulk measured
a wash, and the single measured gain (23 ms per cold connection) is billed rarely because sockets
are pooled per route.

The trigger to re-open this is a **requirement**, not a number: if a spec ever asks for transfers
that survive packet loss or an interface handover mid-stream — the two properties QUIC uniquely
buys and this box could not demonstrate — then revisit, and revisit with a size budget first. A
throughput target is not a trigger: it measured as a wash.

## 7. Traps found while probing, recorded so the next one is shorter

- `h3::client::new(...)` returns `(Connection, SendRequest)` and the `Connection` is **not** a
  `Future` or a `Stream`: quinn's endpoint drives the socket. Keep both objects alive in scope or
  the request silently stalls.
- Every h3 request stream **must be `finish()`ed**. Without it Cloudflare accepts the request and
  never sends the body — which looks exactly like a slow link, and cost a whole sweep with no
  connect timeout to diagnose.
- `tokio::time::timeout(cap, rep(..))` must be **constructed inside** `block_on`, not passed to it:
  tokio registers the timer at construction, and constructing it outside panics with "there is no
  reactor running".
- Discover real h3 endpoints from the `alt-svc: h3=":443"` header of an ordinary h1 response.
  `speed.cloudflare.com` — the obvious choice — **serves no HTTP/3** (no `alt-svc`, and it answers
  TLS alert 28 / `crypto(28)`). Positive controls that do advertise `h3=":443"`: `www.cloudflare.
  com`, `www.google.com`, `www.facebook.com`, `www.instagram.com`, `www.youtube.com`,
  `www.akamai.com`, `www.qualcomm.com`.
- `curl` on this box offers only `http/1.1` in ALPN, so a curl probe can never prove h3 support.
- The crates.io sparse index path is by first letter for 1-2 char names (`2/h3`) and by first two
  letters for 3+ (`qu/in/quinn`); `crates.io/api` returns 403 without a User-Agent. The registry
  root on this machine is `H:\tools\cargo`, not `~/.cargo`, so read the real source of a crate
  there instead of guessing its API.
