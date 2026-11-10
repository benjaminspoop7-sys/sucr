# Proxy support and HTTP auth — scope

**Status: scoped, steps 1 – 5 closed, step 6 closed as deferred, step 7 closed, step 8 closed as the user's
first `DECIDED` in this slice.** Written 2026-11-04, the same hour the size budget was ruled at
**5 MB** (`scripts/verify-m0.mjs:31`), so for the first time in this project the constraint on a transport
feature is not bytes — see §5. **Later the same day §6 step 0 ran to completion** in
`D:\RE\_cap-harness\proxy-spike` — zero product bytes committed, `engine/bond.rs` reverted verbatim — and
every claim §4.1-§4.2 marked *unverified* is now measured; §4.7 is the ledger of that run, and step 1b added
the launched-exe ledger to §6. §8 now carries
**chosen** answers to Q1–Q6 with the evidence under each; none of those six is `DECIDED`, which only the user
writes — the slice's one `DECIDED` is step 8's ruling of 2026-11-05, quoted verbatim there. **Steps 1a, 1b, 2, 3, 4 and 5 are implemented and proven in the shipped exe; step 6 is closed as deferred
(its own usage precondition checked against evidence, §6 step 6); step 7 is closed — the credential wording is
kind-aware in the shipped exe and both launched-exe suites re-pass against it, at 0 B (§6 step 7); step 8 is the
user's ruling that a clean slate means the secrets too, landed at 0 B and proven by a fifteenth leg (§6 step 8).
What the slice
still does not have is named in §6 step 5's "Not covered" and §7: proxy Digest, SOCKS5, the registry-backed system
read and an `https://` origin.** Step 2
is the one that makes the proxy honest rather than merely functional — Q4's R2 is in force and measured, and
the one gap it opened (the Networks pane's totals omit proxied bytes — **closed 2026-11-05**, see the end of
§4.3 and `design/unattributed-bytes.md`) was recorded in §4.3 and §6 rather than
buried. Step 3 is the credential: one `credentials` row under `scheme='proxy'` (Q5's corollary, so the URL
setting keeps carrying no secret), sent **pre-send inside the CONNECT** rather than after a challenge, and a
typed error whose both halves — `proxy-auth` for a 407, `proxy-failed` for a 403 — have now been observed
leaving the exe rather than only out of a unit test. It is also the first step of this slice to need a **new
IPC channel** (`proxy:set-auth`, 33 → 34), so the four gate snapshot sites moved with it.

---

## 1. Why this slice next

The Plexo gap study measured two absences that Sucr shares with the app it replaces: **no proxy
configuration and no HTTP authentication**. Everything else Sucr either has or deliberately does not want.
A corporate user behind a mandatory proxy cannot download a single byte today; a user whose files sit on a
Basic-auth or Digest-auth server (appliances, NAS, internal mirrors, a lot of FTP-over-HTTP bridges) is in
the same position. Both are *foundational* rather than *featured*: they change what the engine can reach.

The reason this needs a scope document before code, and the reason it is not a two-hour patch, is §4.3:
a proxy moves the meaning of every promise Feature 1 (per-network limits, the Sucker Map, per-adapter
attribution) just made. Feature 1 charges bytes to the interface that would carry them to *the origin*.
Introduce a proxy without re-pointing that and the app will draw a confident, wrong picture of the network.

---

## 2. What is actually true today (`grep` at scoping time, with locators; five rows since superseded by this slice)

The short version: **"no proxy support" is not quite right, and the wrong half is the dangerous one.**

> **This table is the tree as it stood at scoping time, and five of its rows have since been changed by this
> very slice.** Finding 1 (`proxy` appears 0 times) and finding 2 (no 407, no `Authorization`, no typed proxy
> error) are what steps 1a/1b and step 3 replaced; the dangerous half of finding 3 — *no UI, no status, no way
> for the user to see or override it* — is closed by the Settings card and `via()`, though the env read itself
> is unchanged (`Mode::System` is still `Proxy::try_from_env()`); finding 12's "33-channel list" is **34**
> since step 3 added `proxy:set-auth`; and finding 5's silent ignore is closed too — step 0 priced putting
> `ConnectProxyConnector` at the head of the bonded chain (+512 B) and step 1a shipped it. Findings 4 and 6-11
> still describe the shipped app, including finding 6: the schema slot step 3 filled (`scheme='proxy'`, one row
> per proxy authority) is exactly the one that was waiting. Read §6 as the running diff against this table.

| # | Finding | Evidence |
|---|---|---|
| 1 | The word `proxy` appears **0 times** in the Rust source and **0 times** in the renderer. No UI, no setting, no code path. | `grep -rin proxy src-tauri/src --include=*.rs \| wc -l` → `0`; same over `src/` → `0` |
| 2 | No 401, no 407, no `WWW-Authenticate`, no `Authorization` anywhere in the engine. A `401` from an origin falls into the generic bucket and the job dies with `server answered 401`. | the same grep over `Authorization\|401\|407\|WWW-Authenticate` returned only `TcpStream`/`connect` hits; generic bucket at `engine/mod.rs:253-271`, `engine/job.rs:2933`, `engine/job.rs:3219` |
| 3 | **But** every agent is built from `Agent::config_builder()`, and ureq 3.5's `Config::default()` sets `proxy: Proxy::try_from_env()`. So an `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` in the environment **already re-routes Sucr's default-route downloads** — with no UI, no status, and no way for the user to see or override it. | `engine/mod.rs:135`; ureq `src/config.rs:240-241` (`builder()` → `Config::default()`), `:1073` (`proxy: Proxy::try_from_env()`); env list at `src/proxy.rs:233-240` |
| 4 | That is dormant on this box, not absent: no proxy variables are set here, so behaviour today is direct. | `env \| grep -i proxy` → empty |
| 5 | **The bonded path cannot see a proxy at all.** `Route::agent_with()` builds `Agent::with_parts(config, BoundConnector.chain(NativeTlsConnector), DefaultResolver)`. ureq's CONNECT-proxy connector exists only in `DefaultConnector`'s chain, so a proxy in the config is silently ignored for every bound route — including the env one from finding 3. | `engine/bond.rs:74-84`; ureq `src/unversioned/transport/mod.rs:384-410` (chain order: Socks → ConnectProxy → Tcp → Tls) |
| 6 | **The schema was designed for this feature and is waiting.** `credentials.scheme` already CHECK-accepts `'basic'`, `'digest'`, `'ntlm'`, `'proxy'`, `'cookie'`, `'header-secret'`; `downloads` already has a `proxy_profile_id` column. | `src-tauri/schema.sql:35-44`, `:83` |
| 7 | Those slots are currently dead weight, like `schedules`: `header_secrets`, `proxy_profile_id`, `dupe_index`, `media_captures`, `grabber_projects` and `schedules` each have **0** references in non-test Rust. Only `net_usage` is live (16). | counted with `grep -rn "$t" src --include=*.rs \| grep -v tests \| wc -l` |
| 8 | Adding proxy or auth credentials needs **no schema version**: one row per host under a new `scheme` value is exactly what `credentials` is for. | `db.rs:278-284` (the comment that says so), `:425-427` (the insert) |
| 9 | There is **one** config choke point and **three** live request-building sites. `config_ua()` is the only place a `Config` is assembled, and it feeds the transfer agents, the bonded agents and the add-bar link check. | `engine/mod.rs:130-152`; consumers `engine/bond.rs:75`, `ipc.rs:791`; requests at `engine/mod.rs:197-206` (probe), `engine/job.rs:2890-2900` (ranged worker), `engine/job.rs:3211-3216` (single stream) |
| 10 | An identity already rides that thread: a pasted `Cookie` is an `Option<&str>` carried from the probe into every later request of the job. The User-Agent went the other way — onto the *agent* — precisely because per-request would repeat at every build site. | `engine/job.rs:2385`, `:2442`, `:2528`, `:2747`, `:2878`, `:3215`, `:3605`; the reasoning at `engine/bond.rs:70-73` |
| 11 | The settings-to-engine mirror pattern exists and is three lines to copy: an atomic static, a setter, a call from `settings_set`, plus a read at boot. | `engine/mod.rs:89-93`, `ipc.rs:939-943`, `lib.rs:382` |
| 12 | `SETTING_KEYS` is a fixed array of 17 (`ipc.rs:875`) and the gate pins both the key count and the 33-channel list, so a new key is a cheap edit and a new *channel* is the expensive one. | `ipc.rs:875-893`; `scripts/verify-m0.mjs:55` (`check('33 channels', …)`) |

Note on finding 3 that is worth keeping: `Proxy` reports whether it came from the environment
(`proxy.rs:319 is_from_env`), so the app can name the source honestly instead of just obeying it.

---

## 3. The two transfer paths, in one picture

```
             config_ua()  engine/mod.rs:130   <- the one place a proxy belongs
                  |
    +-------------+----------------+
    |                              |
 default route                 bound route
 config.new_agent()       Agent::with_parts(config,
 engine/mod.rs:158          BoundConnector.chain(NativeTls))
 ureq DefaultConnector      engine/bond.rs:78-82
   = Socks -> ConnectProxy   ureq user chain
     -> Tcp -> Tls            = Bound -> Tls
   (proxy honoured)          (proxy IGNORED -- finding 5)
```

Used by: the one-byte probe (`engine/mod.rs:195`), the ranged workers (`engine/job.rs:2890`), the
single-stream retry (`engine/job.rs:3211`), the link refresh (`engine/job.rs:3605`) and the add-bar link
check (`ipc.rs:791`). Any design has to land on both branches or it is a half-feature with a silent
asymmetry — which is what finding 3+5 mean the app *already* is, by accident.

---

## 4. The decision surface

### 4.1 Where a proxy attaches

**Option A — set `Config.proxy` only.** One line inside `config_ua()`. The default-route path gains HTTP
CONNECT and (with the feature flag) SOCKS5; the bonded path keeps ignoring proxies. Cheap, and it makes
the current accidental behaviour deliberate — but the asymmetry is the worst property of the status quo,
because it means the answer to "is this download going through my proxy?" depends on whether bonding
happened to pick a route. **Verdict: not an end state. Useful only as the spike in §6 step 0.**

**Option B — one proxy setting honoured on both paths.** Add ureq's exported `ConnectProxyConnector`
(`transport/mod.rs:50`) to the bonded chain. The chaining rules say the shape is
`ConnectProxyConnector.chain(BoundConnector).chain(NativeTlsConnector::default())`: a connector that is
handed an already-open transport passes it through (`connect.rs:33-36`, `chain.rs:18-27`), and the CONNECT
connector opens its *own* socket by calling back into the pool with the proxy stripped from the config
(`connect.rs:66-83`, `config.rs:265 clone_without_proxy`), so that inner call reaches `BoundConnector` and
the socket to the proxy is bound to the route's NIC. TLS-to-the-origin then wraps the tunnel, which is the
same order the default chain uses. **This is the shape I recommend, and it must be proven by a compile and
a local-proxy spike, not accepted from source reading** — §6 step 0 exists for exactly that.

**Step 0 verdict: Option B works, and the source reading was right.** The chain above compiled against
Sucr's exact dependency set with **one** signature change — `BoundConnector` goes from `impl Connector`
(`type Out = Either<(), BoundTransport>`) to `impl<In: Transport> Connector<In>`
(`type Out = Either<In, BoundTransport>`). Its existing `if chained.is_some()` passthrough guard was
already correct and needed no edit. The bound socket did reach the proxy through the chosen NIC: the proxy
logged the control connection's peer as `127.0.0.2`, the second-address stand-in `engine/bond.rs:368`
already uses, and the negative control (a `192.168.1.2` bind against a loopback proxy) died with
`os error 10049`. Full leg table in §4.7.

**Option C — "use the system proxy" mode.** Two sub-variants, both cheap in code, different in linkage:
- *C1* enable ureq's `win-system-proxy` (ureq's own `Cargo.toml:114`: `win-system-proxy = ["dep:winreg"]`),
  which reads `…\Internet Settings` through `winreg`. `winreg 0.56`
  is in `Cargo.lock` but only as a dependency of `embed-resource`, a **build**-dependency
  (`Cargo.lock:671`), so it is not linked into the exe today — this feature would make it a real linked
  crate for the first time. The standing rule says lockfile membership is not linkage: measure it.
- *C2* read the same registry keys (or ask WinHTTP via `WinHttpReadProxyForCurrentProcess`) through the
  `windows` 0.62 / `windows-sys` 0.61 crates **already linked** (`Cargo.toml:35-50`), adding a feature
  module and no crate.

**Verdict: C2 if a system mode is wanted at all.** It also answers WPAD/PAC for free, which C1 does not.

### 4.2 Which proxy protocols

| Protocol | Cost | Notes |
|---|---|---|
| HTTP CONNECT (`http://proxy:3128`) | in core, no feature flag | `ProxyProtocol::is_connect()` covers `Http` and `Https` (`proxy.rs:52-54`). `Proxy::new("http://user:pass@host:port")` parses a single settings string (`proxy.rs:159`), and `Proxy::builder(..).no_proxy(..)` gives a bypass list without touching the environment (`proxy.rs:438`). **Two API traps step 1 must not trip on, both read and one exercised in §4.7:** `no_proxy()` takes **one expression per call** and does not split on commas — a bypass list is a loop, not a string (the *env* path does split, `proxy.rs:580`); and `Proxy::new()` cannot carry a bypass at all, so a manual proxy with a bypass list must go through the builder, which takes `ProxyProtocol` + host + port + user + pass, not a URI. |
| HTTPS-to-proxy | same code path | **measured, and it has a trap.** It works (`https://sucr:pw1@127.0.0.1:8900` tunnelled a GET fine), but only when `TlsConfig::builder().provider(TlsProvider::NativeTls)` is set **explicitly**. `TlsProvider`'s `#[default]` is Rustls (`tls/mod.rs:24-33`) and `TlsConfig::default()` copies it (`:369-382`), and `NativeTlsConnector::connect` passes the transport through untouched when the provider is not NativeTls (`native_tls.rs:48-51`) — so a bespoke chain with a default `TlsConfig` sends a **cleartext CONNECT to the TLS port** and fails as an opaque "proxy server did not respond". Sucr already avoids this at `engine/mod.rs:139-146`; any step-1 code that builds its own `TlsConfig` must not regress it. The proxy confirmed the hop was real (`tlsHop:true`). |
| SOCKS5 / SOCKS5h | **feature flag with no dependency** — ureq's `Cargo.toml:112` is `socks-proxy = []`, the handshake is hand-written in `transport/socks.rs` | Local resolution for `socks5`, remote for `socks5h` (`proxy.rs:57-62`). `SocksConnector` is exported (`transport/mod.rs:60`), so Option B's chain shape works symmetrically. |
| PAC / WPAD | **out of scope** | needs a JS engine or a partial PAC parser; the OS already resolves it (Option C2). |
| Forward proxy for `http://` (absolute-URI request line) | **not implemented by ureq at all** | `connect_proxy_uri()` gates on `is_connect()` (`config.rs:251-257`), so an `http://` target through an `http://` proxy is still tunnelled with `CONNECT host:80`. Older or strict proxies sometimes refuse CONNECT on port 80. The only alternative is hand-writing the request-line rewrite inside the bonded transport — real bytes, real risk, and it makes the two paths diverge again. **Step 0 measured both sides of this: the harness proxy accepted `CONNECT 127.0.0.1:8898` and tunnelled it (200, 4096 B), and a second instance configured to refuse that port made the failure legible as `CONNECT proxy failed: proxy server responded 403/403` — a `ConnectProxyFailed`, never a `Response`. Recommendation stands: tunnel. What a refusal costs the user is now a UI question (§4.5), not an unknown.** |

### 4.3 What a proxy does to Feature 1 — the part that must not be an afterthought

Feature 1's attribution asks the OS which local address would carry traffic **to the URL's server** and
charges that adapter: `attribute_egress()` (`engine/job.rs:836-858`) → `egress_source(url)`
(`engine/bond.rs:95-145`, a route lookup resolved against `server_addr(url)`) → `networks` row → the
bucket the cap paces and the allowance spends → the colour on the Sucker Map.

Under a proxy, **no byte ever travels that route.** The sockets go to the proxy's address; for an HTTP
proxy the origin is not even resolved locally (`config.rs:819`: that phase is skipped when the proxy
resolves the target). So with a proxy active:

- the map colours chunks by a route that carried nothing;
- a per-network rate cap paces a bucket on the wrong adapter, i.e. it limits the wrong interface and lets
  the real one run free — the exact class of untruth Feature 1 step 4 was built to remove;
- an allowance shuts a route that is not the one consuming the data;
- `network.metered_guard` excludes the wrong adapter from the pool.

None of that is a rendering bug. It is the engine charging the wrong party, and it is why "just set
`Config.proxy`" (Option A) is not a shippable slice even though it is the smallest one.

**Step 0 demonstrated the charge-to-the-wrong-party, in one run.** `egress_source("https://example.com/")`
— Sucr's attribution call verbatim in shape — answered **`192.168.1.2`** (the WiFi adapter, the route to
the *origin*), while the proxied bytes for that same target left from **`127.0.0.2`** and the proxy logged
the peer as `127.0.0.2`. Asking the same function about the proxy itself,
`egress_source("http://sucr:pw1@127.0.0.1:8899")`, answered `127.0.0.1`. So the defect is not a matter of
degree: `egress_source` takes no proxy argument and structurally cannot name the hop that carries the
bytes. That is the measurement behind Q4, and the reason R2 is the honest first slice.

Three honest resolutions:

- **R1 — re-point attribution at the proxy.** When a proxy is active for this URL, resolve and attribute
  against `server_addr(proxy_uri)` instead of `server_addr(url)`, and label the card "via
  `proxy.example:3128`". For a *LAN* proxy each route still genuinely reaches the proxy through its own
  NIC, so bonding keeps its meaning. Correct, and the most work.
- **R2 — declare bonding and a manual proxy mutually exclusive for the first slice.** Bonding off, one
  route, attribution honestly "default route via proxy". Cheap, defensible, and it is what most download
  managers effectively do.
- **R3 — keep attributing against the origin and say nothing.** Not offered; it is a false instrument.

**Recommendation: ship R2 as the slice that lands, and design it so R1 is an additive change** (the
attribution question becomes one function taking a different authority, not a rework). Q4 in §8 asks the
user to rule which is the promise. **R2 landed as step 2, and the "additive" half held:** the authority is
one call — `proxy::egress(url)` — asked once per job in `job.rs`, and `Proxied` currently answers "default
route, charge nobody". R1 is that one arm resolving `server_addr(proxy_uri)` and calling `attribute_egress`
with the result instead of skipping it; no call site, colour, or deck rule has to move.

Two step-0 measurements sharpen what R2 actually costs, and both argue it is smaller than it reads:

- **R2 is not absolute, because `NO_PROXY` bypasses before the proxy connector runs.** The bypass leg —
  a manual proxy plus `.no_proxy("127.0.0.1")`, target `http://127.0.0.1:8898/file` — produced **no
  CONNECT at all** (the check is inside `ConnectProxyConnector`, `connect.rs:49-57`) and the origin saw the
  request arrive from the **bound** address `127.0.0.2`, direct. So "bonding off under a manual proxy" is
  really "bonding off for proxied hosts": a bypass entry still gets its own NIC, which is exactly the
  shape a LAN-proxy user wants and the reason R1 stays worth doing later.
- **One tunnel carries many streams, so R2's single-route proxy path does not multiply CONNECTs.** Two GETs
  from clones of one `Agent` (`Agent` is `#[derive(Clone)]` over an `Arc<Config>` + `Arc<ConnectionPool>`,
  `agent.rs:89-92` — the way `engine/job.rs` hands its workers one pooled agent) came back with **one**
  logged CONNECT and 8192 B total: the tunnel is pooled and reused across clones. What was *counted* is two
  requests and one CONNECT; that a 13-stream job behaves the same way was **inferred from the pool keying,
  and step 1b measured something different again — so the pooling question is open, not settled.** The two
  proxied legs (`manual-typed`, `system-resolves-env`) opened **12 to 14 tunnels for one 32 MiB job while
  `streams_final` was 8 every time** — neither one CONNECT per job, which is what step 0's clone test
  suggested, nor one per stream. The count also moves between identical runs, so it is not a clean function
  of the job at all; the honest reading is *at least one tunnel per range request, plus whatever the ramp
  and the re-opens add*, with the exact split **unknown**. The cost claim that survives either explanation:
  R2's CONNECT overhead scales with the work of a job, not with the job, so against a proxy that counts
  CONNECTs a 32 MiB download is ~13 and a large one more; and step 3's credential write has to authenticate
  each of those tunnels, not one per job. This bullet carried a per-stream causal sentence in its first
  draft and retracted it once the tunnel counter sat beside `streams_final`.
- **Repetitions of the same legs agree, which is what makes them evidence.** Five invocations of the driver:
  the first crashed on a harness bug (the `const` re-declaration above), the second ran all seven legs but
  failed five of them on my own wrong denominator, and the last three passed everything — two against the
  exe before the `--smoke` check and one against the rebuild after it. The proxied legs counted 12, 14, 14,
  12 tunnels (`system-resolves-env`) and 13, 13, 14, 13 (`manual-typed`) across those four completed runs,
  with `streams_final` 8 and `verifiedBytes` exactly 33,554,432 every single time. Timings are not quoted:
  the poll granularity is 250 ms and these are single-shot reads, not rotated medians.
- **Step 2 measured the collapse and the silence, in the shipped exe — and found a third thing the four
  bullets above did not predict.** On a proxied leg the whole 32 MiB lands on `networkId: null` (map stripes
  `{"n0": 33554432}`) while `network:list` shows **both** adapters growing by **0**, and the deck reads
  `0 B of 32.0 MB attributed`; on the bypass leg of the same run the same file lands on the Ethernet stripe,
  Ethernet grows by the full 33,554,432 B, and the deck reads `of 32.0 MB attributed` with **0** tunnels. So
  R2's two claims — bonding collapses to one default route, and nothing is charged to any adapter — are
  measured, not inferred, and the per-URL half of the correction above is measured with them (§6 step 2).
  The third thing: **the Networks pane's own totals are adapter sums, so they under-report by exactly the
  proxied bytes.** §4.3 predicted the cap, the allowance and the map going wrong; it did not predict that
  the usage ledger itself would quietly omit a class of traffic. A deck note and a sentence on the pane say
  so; the numbers still do not include what no adapter carried. Fixing that means unattributed bytes in the
  usage payload ≈ a new rusqlite read path ≈ **5 KB**, which is its own ruling — see §6 step 2 and the
  open-defects record.
- **Closed 2026-11-05, and the two halves of that sentence are worth separating.** *"The numbers still do
  not include what no adapter carried"* is false of the shipped build and is retracted, not amended:
  `network:list` now answers `{ adapters, unattributed }` from a second ledger (`unattributed_usage`,
  schema **v4**), the pane adds it to Downloaded Today/Week/Month/Lifetime, and `#net-desc` names the
  amount that belongs to no adapter. *"≈ 5 KB"* was a prediction and was a little high: measured
  **+3 584 B** (exe 4 569 600 → **4 573 184 B**). No byte was folded into an adapter row — the bypass leg
  and the extended engine test both show `net_usage` unmoved. Full record:
  [`design/unattributed-bytes.md`](file:///D:/RE/Sucr/design/unattributed-bytes.md), including the
  counterfactual that re-measured the gap on the retained previous binary (67 108 864 B invisible; the
  pane stopped at `32.0 MB`, now reads `96.0 MB`).

### 4.4 Origin authentication

| Scheme | Verdict | Why |
|---|---|---|
| **Basic (401)** | in, first | The 401 arrives as a normal `Response` because Sucr sets `http_status_as_error(false)` (`engine/mod.rs:150`), so `www-authenticate` is readable with no plumbing change. Encoding needs base64, already linked through ureq's CONNECT credential path (`connect.rs:121`) — either declared as a direct dependency at the version already in `Cargo.lock`, or ~15 hand-written lines to keep the graph untouched. Store under `scheme='basic'`, no schema change. |
| **Digest (401)** | **deferred — step 6 closed as "precondition unmet", 2026-11-04** | MD5 and SHA-256 are **already linked** for checksum verification (`md5::Md5` at `engine/mod.rs:680`, `sha2` at `:19`), so the crypto is free. What is not free: challenge parsing (`realm`/`nonce`/`opaque`/`qop`/`algorithm`), `HA1`/`HA2`, `qop=auth` and `auth-int`, stale-nonce retry, and a CSPRNG for `cnonce` — available from the already-linked `windows` crate (`BCryptGenRandom`), no new dep. This is the single biggest code chunk in the feature and the one most likely to be wrong in a way that only shows up against a real server. The step's own trigger was "if and only if Basic's real usage shows servers that need it", and there is no usage to consult: no telemetry, no field report, and no real host offering Digest alone has ever been surveyed here (the step-4 fixture is the only one, and it was built for the naming). §6 step 6 carries the re-entry condition. The 401 sentence already names the offer, so a Digest-only host is told *what* it asked for rather than answered silently. |
| **Bearer / custom header** | small, worth it, and already half-provisioned | A `Authorization: Bearer …` paste is the same shape as the existing cookie-paste feature and would cover most API-ish hosts and the "header-secret" scheme `schema.sql:38` names. Cheap. **Shipped as step 5.** The paste goes out verbatim — no base64, no padding repair — and `schema.sql:38` needed nothing, which is finding 6 paid off: the whole feature was one field, one predicate and one switch. A custom *named* header (anything other than `Authorization`) is still out: a request carries one `Authorization`, so the store holds one credential per host and the kind written second owns it. |
| **NTLM** | **out** | In the schema CHECK, but it is a multi-round, DES/MD4-based handshake needing a crate the graph does not have. Named cost, low payoff for a download manager. Say so in the UI rather than fail mysteriously. |
| **Kerberos / Negotiate (SSO)** | **out** | Same reasoning, larger. |
| **Proxy auth (407)** | Basic only, and see §4.5 | ureq writes `Proxy-Authorization: Basic` for CONNECT from the `Proxy` credentials (`connect.rs:116-126`) — so proxy Basic is **free** with Option B. Proxy **Digest** for CONNECT is **unreachable without forking ureq's connector**: nothing in the chain sends a second CONNECT. That is a hard limit of this design, and it should be stated in the docs rather than discovered by a user — **step 6 verified it against the crate rather than reasoning about it: zero `digest` matches in the whole `ureq-3.5.0` tree, one `Proxy-Authorization` write site, and a CONNECT reader that fails on any non-200 instead of retrying with a challenge.** **Shipped as step 3, and this row's claim is now measured, not read:** the exe put the header *inside* the first CONNECT on every one of 11 tunnels (`refused: []`, no challenge round) and never sent a credential to a proxy it was not keyed to. The Digest limit is documented in §7 and enforced by the store accepting Basic only. |

Where the origin header **rides** — two shapes, and the choice matters later:

- **H1 a `Middleware` on the config** (ureq `config.rs:744`, `pub mod middleware` with no feature gate at
  `lib.rs:613`): one place, automatically covers probe, both transfer paths, the refresh and the link
  check, and is the same decision the app already made for the User-Agent (`bond.rs:70-73`) for the same
  reason. Constraint: middleware is per-agent, so a per-host secret means either one agent per host
  identity (the app already builds one agent per download, and one per route) or resolving lazily inside
  the middleware from the request URI — which works and is the obvious shape.
- **H2 extend the identity thread** the way `cookie: Option<&str>` is extended today (`job.rs:2385` etc.):
  three extra header calls, no new concept, but it must be repeated everywhere a request is built — and
  the app's own comment says that is the pattern it avoided for the UA.

**Recommendation: H1.** It is what the code already concluded once.

Storage and at-rest honesty: `credentials` rows hold the secret in plaintext, and `db.rs:278-288` says
so openly with a threat model (same file as the library, same user's profile, anyone who can read it can
read the browser's cookie store). That argument covers passwords exactly as well as it covers
`cf_clearance` — but it is a *ruling on record*, not encryption, and it is one of the held defects. Q5 asks
whether to settle it now (one mechanism, two scheme kinds) or re-affirm the plaintext position deliberately.
Also worth naming: `username` is currently overloaded — for `scheme='cookie'` it stores the User-Agent the
cookie was copied with (`db.rs:280-283`), so a `'basic'` row must not inherit that convention.

### 4.5 Surfacing (this is where the feature earns its keep)

- A **401 from the origin** is a `Response` with a status — the existing `Probe.status` field carries it
  (`engine/mod.rs:223`) and `probe_link` already returns `{ok, code, status, message, session}`
  (`ipc.rs:792-809`). New codes: `needs-auth` (+ whether `www-authenticate` offered Basic or Digest).
- A **407 from the proxy is not a Response at all.** ureq raises
  `Error::ConnectProxyFailed("proxy server responded 407/Proxy Authentication Required")`
  (`connect.rs:150-155`), and Sucr flattens every ureq error into `code: "http"` with the Display string
  (`engine/mod.rs:81-85`). So the app would show a download that "failed: http" with a sentence inside it.
  The fix is a typed branch: match `ConnectProxyFailed`, emit `code: "proxy-auth"`, status 407 — and the
  same for `Timeout`/`HostNotFound` if the pattern is opened once.
  **Step 0 measured this and corrected two things about the sentence above.** The variant is
  `ConnectProxyFailed(String)` (`error.rs:165`) — **the status code exists only inside that string**, so a
  typed branch cannot read `407` numerically the way `Probe.status` reads an origin 401; and the strings
  that actually arrived from this box were `CONNECT proxy failed: proxy server responded 407/407` (no
  credentials) and `.../403/403` (a proxy refusing `CONNECT` to the origin's port). The reason-phrase field
  came back as the digits, not "Proxy Authentication Required" / "Forbidden", which is observed but not
  chased — the design consequence stands either way: **do not put that Display string in front of a user,
  and do not plan to split 407 from 403 on a status object.** Either sniff the leading digits of the string
  or ship one honest `proxy-failed` code that covers refused-and-unauthenticated, and let the UI say which
  it is only when it can prove it.
  **Step 3 chose digit-sniffing, and it is a narrow, honest branch** (`engine/mod.rs:96-111
  proxy_connect_error`): strip the literal prefix `proxy server responded `, take the field up to the first
  `/`, parse it as `u16`; the code is `proxy-auth` **iff** that parse yields exactly `407`, otherwise
  `proxy-failed`, and `.with_status(status)` is attached only when the parse succeeded. The message is
  `the proxy refused the tunnel: {reason}` — the reason is included because it is the only place the status
  exists, and the user needs the number, but the *code* never depends on ureq's phrasing: a ureq that rewords
  its string degrades to `proxy-failed` with no status, which is the safe direction (a generic proxy failure,
  never a credential accusation). What the shipped exe actually printed, from the two refusals §6 step 3 ran:
  `the proxy refused the tunnel: proxy server responded 407/407` with `errorCode: "proxy-auth"`, and
  `...: proxy server responded 403/403` with `errorCode: "proxy-failed"` — both codes therefore observed
  leaving the product, not only out of `engine::tests`. So §4.5's warning was right that no status *object*
  is readable, and the split that shipped keys off the digits alone, which is exactly as much of a
  407-vs-403 distinction as this payload can prove.
- The **add bar's link check** should say which of the two it is, because "the link is greyed out" with a
  proxy-unreachable reason and with a wrong password are different user actions. The greyed-out Download
  path already distinguishes `blocked` from `http-status` (`engine/mod.rs:253-271`), so there is precedent.
- A **status line naming the proxy** (manual / system / env) with the source, because finding 3 means the
  app can currently be proxied for a reason the user cannot see. `Proxy::is_from_env()` exists (`proxy.rs:319`).
- **Step 3's credential line: the name travels, the secret does not.** The card answers one of three things —
  no credential stored, a pair *in use* for the authority it was typed against, or a pair *stored, not in use*
  because the address no longer names that proxy — and it prints the username and the `host:port` only. The
  secret has no path out: it is not in that sentence, not in the password box after a save (the box is a
  **replacement field**, so a stored pair reads back empty), and not in `app:get-info` — which is the payload
  the page can ask for wholesale, and was grepped for it. `applies` is decided by the engine's own
  `auth_host()`, not by the renderer comparing two strings, so the card cannot claim a credential is live when
  the mode is not `manual`.
- The **network card** should say `via <proxy>` when attribution is re-pointed, or say bonding is off because
  a manual proxy is set — whichever R1/R2 wins.
- `history_events` needs kinds for auth-challenged and proxy-failed transfers. 30 kinds existed at scoping
  time and the build counts them (`app-design-plan-v2-engine.md` §10 M0 row). **Step 2 added the 31st,
  `proxy`, and the count is enforced, not folklore: `scripts/build.mjs` scans every `.rs` (excluding
  `tests.rs`) for `history(…)` kind literals and fails the build when a kind has no `case '…':` in
  `formatEvent()`, printing `history kinds: 31 written by the engine, all formatted by the UI`.** A working
  note raised during step 2 contradicted all of that — roughly 14 kinds, no `CHECK`, no gate — and was
  tested against the build output before it could be written here. The note was wrong; the scoping-time
  claim stands, and this is recorded because the wrong note was the closer call to becoming a "correction".

### 4.7 Step 0, measured (2026-11-04)

Harness: `D:\RE\_cap-harness\proxy-spike` — `src/main.rs` (Sucr's `bind_connect` / `family_matches` /
`BoundTransport` / `probe` / `server_addr` / `egress_source` copied verbatim, `BoundConnector` genericised),
`proxy.mjs` (authenticating CONNECT proxy, logs the **peer of its own control connection** — that peer is
the witness), `origin.mjs`, `drive.mjs` (starts two proxies: 8899/8900 permissive, 8901/8902 with
`REFUSE_PORT=8898`, plus the origin on 8898), self-signed `certs/spike-*.pem` for the TLS-proxy hop.
Reproduce with `cargo build --release --offline && node drive.mjs`. Nothing inside `app/` was changed;
`engine/bond.rs` was edited only to take the §5 measurement and is byte-identical to
`bond.rs.baseline` (sha256 `f263ced3…`).

| Leg | Result | What it proves |
|---|---|---|
| `baseline-direct` (bound, no proxy) | 200, 4096 B; origin peer `127.0.0.2`, no proxy entry | the control: the bound chain still works with no proxy in the config |
| `bonded-http` (`http://sucr:pw1@127.0.0.1:8899` → `http://127.0.0.1:8898/file`) | 200, 4096 B; **proxy peer `127.0.0.2`**, `auth:true user:sucr`, `connect:"127.0.0.1:8898"`; origin peer `127.0.0.1` (dialed by the proxy) | **the bound socket reaches the proxy through the chosen NIC**, and `CONNECT` to a plain `http://` target on a non-443 port is tunnelled fine |
| `bonded-http-hostname` (target `http://localhost:8898`) | 200; proxy saw `connect:"localhost:8898"` | `resolve_target()` false ⇒ the CONNECT carries the **hostname**, so the proxy does the DNS, not the app |
| `unbound-http` (default chain, `config.new_agent()`) | 200; proxy peer `127.0.0.1` | Option B's two paths behave the same; the only difference is which source the proxy hop leaves on |
| `no-creds-407` (proxy URI without userinfo) | `CONNECT proxy failed: proxy server responded 407/407`; proxy `reject:"407"` | §4.4's free proxy-Basic claim and §4.5's "407 is never a Response", both in one line |
| `wrong-bind` (bind `192.168.1.2` → loopback proxy) | `io: The requested address is not valid in its context. (os error 10049)` | the negative control for the peer evidence — a bind that cannot reach the proxy fails loudly, so `127.0.0.2` at the proxy was not an accident |
| `https-proxy` (`https://sucr:pw1@127.0.0.1:8900`) | 200, 4096 B; proxy `tlsHop:true`, peer `127.0.0.2` | the TLS-to-proxy hop is real and bound. **Only after setting `provider(NativeTls)` explicitly** — with the default `TlsConfig` the leg failed as "proxy server did not respond" because a cleartext CONNECT went to the TLS port (§4.2) |
| `bonded-https-target` (`https://example.com/`) | 200, 577 B; proxy `connect:"example.com:443"` | TLS wraps the tunnel for an `https` origin and **verified normally against the platform store** — no `disable_verification` needed on this leg |
| `unbound-https-target` | 200, 577 B | same, on the default chain |
| `twice` (two GETs from clones of one `Agent`) | 8192 B and **one** CONNECT logged | the tunnel is pooled and reused across agent clones (§4.3) |
| `bypass` (manual proxy + `.no_proxy("127.0.0.1")`) | 200, 4096 B, **no proxy entry at all**, origin peer `127.0.0.2` | a bypass entry falls through to `BoundConnector` and keeps its own NIC — R2 is not absolute (§4.3) |
| `port-refused` (strict proxy, `REFUSE_PORT=8898`) | `CONNECT proxy failed: proxy server responded 403/403`; strict proxy `reject:"403-port8898"` | the `CONNECT host:port` compatibility risk is a legible failure, not a silent one (§4.2) |
| `egress-src` × 4 | `https://example.com/` → `192.168.1.2`; the proxy URI → `127.0.0.1`; the loopback origin → `127.0.0.1` | Feature 1's attribution call cannot name the proxy hop (§4.3, the Q4 evidence) |

Not tested, and still read-only claims: SOCKS5 (`socks-proxy` was not enabled in the spike), the system-proxy
registry path (C1/C2 — no option was implemented, only the manual mode was exercised), origin 401 Basic/Digest
(no origin-auth code exists yet), and any real corporate proxy. §9 keeps those in the unproven column.

---

## 5. What it costs, against the budget that was just ruled

Headroom at the ruling: exe **4,508,672 B**, new ceiling **5,242,880 B** → **734,208 B**. The size question
is no longer "does it fit" but "is it worth the grams" — and the standing discipline (price before
accepting, name the number) is unchanged.

Measured cost map from Feature 1, applied to the pieces above:

| Piece | Expected | Basis |
|---|---|---|
| `Config.proxy` wired into `config_ua()`; proxy mirror static + setter + boot read | **measured: +5,120 B for the whole engine half of step 1** (4,508,672 → **4,513,792**) | Landed 2026-11-04: `engine/proxy.rs` (the mirror, `resolve`, `via`, and the two validators the boundary calls), the one `.proxy(...)` line in `config_ua`, the three `bond.rs` chain edits, three keys and their validator arms in `ipc.rs`, `db::proxy_settings`, and the boot read in `lib.rs`. The chain alone was +512 B (next row), so **≈4.6 KB is new code plus the ureq proxy paths it finally instantiates** — the same band as "a new rusqlite read path ≈ 5 KB" (**that band is retired** — see the step-2-gap row at the end of this table: two DB-writing paths landed later measured +2,048 B and +3,584 B, so the band was a new *engine module*, never the database), and ten times what the spike priced, because the spike only ever measured the chain. The twelve new tests are not in that number: `cfg(test)` code never reaches the release target. The timeout mirror (`mod.rs:89-93`, `ipc.rs:939-943`, boot read at `lib.rs:382-386`) was the template, and it transferred: one static the engine reads, one setter, two call sites. |
| `ConnectProxyConnector` added to the bonded chain | **measured: +512 B** (4,508,672 → **4,509,184**, then back to 4,508,672 after the revert) | this is the row step 0 existed to price. The change is three edits in `engine/bond.rs` — import the connector, put it at the head of the chain, and genericise `BoundConnector` to `impl<In: Transport> Connector<In>` with `type Out = Either<In, BoundTransport>` — and the whole of it costs half a kilobyte. The feared "generic instantiation is KB-scale" did not bite here: the genericisation replaced an existing impl rather than adding one. |
| `socks-proxy` feature | **no new crate** (ureq's own `Cargo.toml:112` is `socks-proxy = []`; ours is `ureq = { version = "3", default-features = false, features = ["native-tls"] }` at `app/src-tauri/Cargo.toml:30`, so enabling it is one string) | its cost is the SOCKS handshake monomorphised against both chains |
| System-proxy via `winreg` (C1) | a **newly linked crate** — `winreg` is build-graph only today (`Cargo.lock:671`) | measure before choosing; C2 avoids the question |
| Proxy panel in Settings (mode select, URL, bypass) | **measured: +3,584 B** (4,513,792 → **4,517,376**), and **+2,560 B** more for its `--smoke` check (→ **4,519,936**) | Landed 2026-11-04 as §6 step 1b: the card markup, one `proxyVia` field on the existing `app:get-info` payload, and the renderer's defaults/`syncProxyVia`/`renderProxyVia`/`commitProxyField`. **Zero new CSS and zero new channels** — the row is all existing classes (`settings-card`, `seg-control`, `settings-path-box`, `settings-note` + `[data-tone="error"]`), and the engine's answer rides the same carrier as `globalSpeedLimit`, so the 33-channel gate never moved. The estimate was ≈ 5 KB "for the last renderer surface"; it came in 30% under, and the gap is explained by what this surface did *not* add — no new component, no new subscription, no new stylesheet rule. The gate's own check costs more per line than the panel did per line: the smoke driver is bundled into the embedded renderer, so a UI assertion is a product byte. |
| 17 → 20 setting keys, one `settings:set` reuse (no new channel) | **measured: inside the +5,120 B above; no channel churn, no gate churn** | Landed: `SETTING_KEYS` is one array (`ipc.rs:875`) and the gate pins the *channel* count (33) but not the key count, so a key is cheap. A *new channel* is what breaks the four snapshot sites — avoided by staying inside `settings:*`. |
| Basic-auth challenge parse + prompt + store + retry | ≈ 1 – 3 KB | string parsing on already-linked types; base64 already in the graph |
| Digest (RFC 7616) | **estimate never confirmed — step 6 deferred the scheme, so it was never written and never `stat`ed: assume 4 – 8 KB if it is ever opened** | MD5/SHA-256 already linked (`Cargo.toml:23`, `:28`); the cost is the implementation, plus `BCryptGenRandom`. The estimate in this row is the only honest thing to say about Digest's cost, because the one number a byte-cost table wants — what a linked Digest actually measures — does not exist. §6 step 6 records why the scheme was not built to find out. |
| Typed proxy/407 error path | ≈ 0.5 – 1 KB | one match arm in `From<ureq::Error>` and new `code` strings |
| Attribution re-point (R1) | small code, **large blast radius** | one function's argument changes meaning; the caps, the allowance and the map all read through it |
| Step 2 — per-URL egress, the proxied-route seam, the durable `proxy` row, three renderer surfaces, the Networks witness in the gate | **measured: +2,048 B** (4,519,936 → **4,521,984**) | Landed 2026-11-04. One link, not four: the step was built as one tree and `stat`ed once, so the engine / renderer / gate split below is **not measured** — do not cite a sub-number that was never linked alone. What the 2 KB buys, against the rows above: an `Egress` enum plus `egress()` and a runtime `Uri` parse in `engine/proxy.rs`; one `if egress.is_proxied()` in `job.rs` and one `history(…)` write; a credited-sum in `paintRoutes`, one sentence builder in `#net-desc`, one `formatEvent` case; and the polling `netNoteHas()` in the bundled smoke driver, which is a product byte like every other UI assertion. **Cheaper than the 1b panel alone (+3,584 B)**, and the reason is the same one §4.3 kept running into: nothing new was *drawn*. No new channel (the gate stayed at 33), no new CSS, no new crate, no new table — the row, the colour `n0` and the "Not attributed" label were Feature 1's, and the deck note is a text change to an element that already existed. The 31st `history` kind is inside this number too, which is the concrete price of the §4.5 claim that a new kind is not free: it costs the `case` arm the build gate demands. |
| Step 3 — the proxy credential: one `credentials` row, the engine half that puts it in the CONNECT, the typed error, the card's two boxes, and the channel | **measured: +12,800 B** (4,521,984 → **4,534,784**), headroom **708,096 B** under the ruled 5 MiB | Landed 2026-11-04. One link again, so the engine / renderer / gate split is **not measured** — the same discipline as the step-2 row. What the 12.8 KB buys: `db::{set,clear,find}_proxy_credential` over the `scheme='proxy'` slot finding 6 said was waiting (no schema version, as finding 8 predicted); `engine/proxy.rs`'s `auth_host()` + `Credential` mirror so the pair reaches `Proxy::new` at build time; `proxy_connect_error` (§4.5) turning `ConnectProxyFailed` into `proxy-auth` / `proxy-failed` instead of `http`; the credential row on the Settings card (username + password boxes, a **replacement field**, three answer states) with its `--smoke` assertions; and **one new IPC channel** — `proxy:set-auth`, 33 → 34, which is the change class §2 finding 12 called expensive: the four snapshot sites (`ipc.rs`'s own uniqueness test, both `verify-m0.mjs` literals, and the derived renderer surface) all had to move together. That channel is why this step costs 6× what step 2 cost, and it is the first time in this slice the expensive kind of edit was the right one: a secret must not ride `settings:*`, because `settings:list` echoes every stored value to the page (§8 Q5's corollary). For scale: still 0 new crates, 0 new tables, 0 new CSS, and 0 new `history` kinds — the refusal legs reuse step 2's `proxy` row. |
| Step 4 — origin Basic: the middleware that pre-sends it, the store, the card, the link-check distinction | **measured: +27,648 B** (4,534,784 → **4,562,432**), headroom **680,448 B** | Landed 2026-11-04, one link. The expensive part is the change class §2 finding 12 named: **a second credential channel** (`origin:set-auth`, 34 → 35) with its own card, rows and probe-path changes, plus 8 new middleware/db tests. §5's "≈ 1 – 3 KB for challenge parse + prompt + store + retry" row above was an order-of-magnitude estimate and it was low by ~10×: what the step actually paid for was a new surface on the Settings page and a new IPC channel, not the base64. |
| Step 5 — bearer / header-secret on the *same* channel | **measured: +5,120 B** (4,562,432 → **4,567,552**), headroom **675,328 B** | Landed 2026-11-04. 5× cheaper than step 4 on the same feature because it added neither a channel nor a store nor a card: one optional 4th argument, one `Kind` enum, one hidden username box, and the token path through code `config_ua` already built. Two of those 5.1 KB are the sentence reworded mid-step (§9 records the false claim the proof caught); the rest is the branch. |
| Step 6 — Digest | **0 B — no code written; the step was a source check** | `ureq-3.5.0` searched whole-tree (zero `digest` matches) and `connect.rs:124`/`:147-155` read. §6 step 6 records the deferral and its re-entry condition. The exe measured **4,567,552 B** before and after, unchanged, because nothing was linked. |
| Step 7 — the credential wording | **measured: 0 B** (4,567,552 → **4,567,552**) | Landed 2026-11-04. Nine strings across six sites reworded (engine `format!` arms, two renderer verdicts, the inspector ternary, one `index.html` placeholder), the exe relinked and `stat`ed: identical size. Same finding as step 5's reword — a string edit inside an existing arm or an existing element is free at the 512 B section granularity — and the contents are proved changed, not assumed, by binary string probe (§9). What this step *did* cost is the harness: `auth-run.mjs` asserts these exact sentences, so 87 checks became 88 and all 14 legs were re-run. |
| Step 8 — reset-all forgets the credentials | **measured: 0 B** (4,567,552 → **4,567,552**) | Landed 2026-11-05. Renderer-only on purpose: the two forgets already exist as shipped write paths (`proxy:set-auth("","")` and `origin:set-auth(link,"","")`), so the button calls them instead of a new channel being built for it. That keeps `CHANNELS` at 35, the gate's `check(` count at 50, and the write honest — the same commands a Forget row fires, so the reset cannot diverge from the panel in what it drops. The only new string is the note sentence ("All preferences and stored credentials reset to defaults."), which joins step 3's 148 B disclosure, step 5's reword and step 7's nine strings as edits absorbed by the 512 B section padding. |
| Step 2's named gap — unattributed bytes in the usage payload | **measured: +3,584 B** (4,569,600 → **4,573,184**), headroom **669,696 B** | Landed 2026-11-05, one link, and it beat the ≈ 5 KB this table predicted while adding *more* than that prediction assumed: schema **v3 → v4**, a 17th table (`unattributed_usage`), two new `db.rs` functions (a write and a read over a shared `usage_windows` helper), one new engine booking in the `network_id == None` branch, one added field on the `network:list` answer, and the pane's sentence. No 36th channel, no new setting key, no new CSS, no new `history` kind — so none of the four snapshot sites moved. **This is the row that retires "a new rusqlite read path ≈ 5 KB" as a price for a DB path**: read together with the boot reconcile (+2,048 B, also a DB-writing path), the band was always *a new engine module*, not the database. Record: [`design/unattributed-bytes.md`](file:///D:/RE/Sucr/design/unattributed-bytes.md). |

Nothing in scope here is MB-scale. The honest reading of §5 is that **bytes are not the constraint on this
feature, and should not be cited as the reason to defer any part of it.** The constraints are correctness
against real proxies (§4.2, §4.3) and the size of the auth surface (§4.4).

How to price each row when the slicing starts: **`stat` the exe, never compare hashes.** Measured on
2026-11-04, five relinks of this crate produced five different SHA256s at an identical 4,508,672 B — 20
bytes of per-link noise (a random CodeView/PDB GUID plus the PE `TimeDateStamp`) — so a hash delta proves
nothing while the size is stable. See the M0 row of `app-design-plan-v2-engine.md` §10.

---

## 6. Slicing, each step with its own gate

0. **Spike (no product bytes committed). CLOSED 2026-11-04 — all four questions answered, results in §4.7.**
   It answered: the bound socket does reach the proxy through the chosen NIC (proxy peer `127.0.0.2`, with an
   `os error 10049` negative control); a plain `http://` target survives `CONNECT host:80` (200, 4096 B) and
   a proxy that refuses it fails as `ConnectProxyFailed 403`; an `https://` proxy works **only** with an
   explicit `NativeTls` provider; and the chain instantiation costs **+512 B**. The gate to step 1 is passed.
   What step 0 did *not* cover is listed at the end of §4.7 (SOCKS5, the registry/system modes, origin auth).
1. **Proxy setting, both paths, no credentials.** `network.proxy_mode` ∈ `direct | system | manual`,
   `network.proxy_url`, `network.proxy_bypass`. Explicit `direct` must beat the environment (finding 3),
   which means setting `.proxy(None)` deliberately rather than leaving the default. Card/status names the
   source: manual / system / env / none. Two measured constraints this step must respect: any chain it
   builds carries `TlsConfig::builder().provider(NativeTls)` (§4.2 — the default Rustls provider silently
   sends cleartext to an `https` proxy port), and `network.proxy_bypass` is a **loop of `no_proxy()` calls**,
   one entry each, since the builder does not split on commas (§4.2). Also note the coupling the M7 CLI row
   already assumes: that flag set lists `--proxy` and `--proxy-user/-password`
   (`app-design-plan-v2-engine.md` §10 M7). A global setting and a per-invocation flag are not in conflict —
   the flag is an override for one run — but `downloads.proxy_profile_id` (finding 6, a dead slot today) is
   the *third* shape, per-URL storage. Q6 chose global, so step 1 should leave `proxy_profile_id` dead and
   say so, rather than half-wiring it because the column exists.
   **1a LANDED 2026-11-04 (engine + settings, +5,120 B).** Every acceptance criterion above is met and
   measured: `config_ua` writes `.proxy(...)` on all three modes, so `direct` beats the environment on both
   transfer paths — finding 3 is fixed, not merely named; `system` is `Proxy::try_from_env()`, with the
   registry read still deferred as C2's own slice; `manual` builds through `Proxy::builder` with one
   `no_proxy()` call per bypass entry; the bonded chain is
   `ConnectProxyConnector::default().chain(BoundConnector).chain(NativeTlsConnector::default())`; and the
   engine answers "where do these bytes leave?" with `proxy::via()`, which is what step 1b's status line
   reads. 166 tests pass (`--test-threads=1`), typecheck clean, `verify-m0` green at 4.30 MB.
   Three things the writing settled that the scope could not. The boundary **refuses** a proxy URL carrying
   userinfo or a non-CONNECT scheme instead of storing it (Q5's corollary, enforced where it is typed).
   A bypass list is refused when an entry cannot match anything, because ureq's own parser falls through to
   "exact host" for anything unknown: `10.0.0.0/8` is never dropped, it is kept as a host nothing can ever
   be named, which is the silent version of the very defect this slice exists to remove. And `via()` keeps
   the mode in its own text when the mode resolved to nothing (`manual direct`), because a label that said
   only `direct` would hide a misconfiguration.
   **1b LANDED 2026-11-04 and is proven in the shipped binary (+3,584 B panel, +2,560 B gate check →
   4,519,936 B).** The Settings row
   is the three controls and nothing else — all existing classes, no new CSS, no new channel: the card's
   egress line reads `app:get-info.proxyVia`, which is `proxy::via()` evaluated in `ipc.rs`, so the page
   shows the engine's answer rather than recomposing the three stored keys (`system` is a question for the
   environment, which the renderer cannot see). Proof: `D:\RE\_cap-harness\proxy-run.mjs`, seven legs against
   one launched exe (`window.sucr.*` over CDP, its own `--data-dir`, real `.click()` on the panel nodes)
   driving `proxy-spike/proxy.mjs` — now tallying bytes per tunnel — and `server.mjs` as the origin. Report
   `D:\RE\_cap-harness\proxy-report-2026-11-04T18-46-32-545Z.json`, `allPass: true`, 32 MiB per leg, every
   row `verifiedBytes` exact:
   - **`direct-default`** — launched with `HTTP_PROXY` in its environment and no stored mode: `via()` says
     `direct`, **0** tunnels, all bytes arrive. Finding 3 is closed in the exe, not just in the unit test:
     an environment proxy no longer moves Sucr's bytes without the user saying so.
   - **`system-resolves-env`** — one click on System: `system 127.0.0.1:8127`, **14** tunnels, all dialling
     `192.168.1.2:8097`, 34,082,014 B counted leaving the origin *through the proxy*.
   - **`manual-typed`** — the same from a typed string: `manual 127.0.0.1:8127`, **13** tunnels,
     35,556,153 B across. Bytes through the proxy, the row's `verifiedBytes` exact, and a 32 MiB file on disk
     is the proof: the tunnel tally exceeds one file because the surplus tunnels re-served ranges, so treat
     `down` as a floor on what crossed, not as the file size. §4.3's pooled-tunnel expectation is **not**
     confirmed by these counts — see the correction there.
   - **`manual-with-bypass`** — bypass `192.168.1.2` with the proxy still resolved and shown: **0** tunnels,
     all bytes arrive. This is the measurement the step-1b card sentence rests on.
   - **`refused-url` / `refused-bypass`** — credentials in the address and a CIDR range are each refused in
     the validator's own words on the card, `data-tone="error"`, the stored key unchanged, and the box keeps
     what was typed so it can be fixed. No download is attempted.
   - **`cleared-url`** — empty the address while the mode stays `manual`: `via()` says `manual direct`, the
     line says "found nothing to use", the stored `proxy_url` is null and `proxy_mode` is still `manual`, and
     the download lands direct. The misconfiguration names itself.
   Two harness lessons, both in the driver now: a `Runtime.evaluate` script that declares a top-level `const`
   runs **once** — the second call of the same helper throws `Identifier 'n' has already been declared`, so
   every multi-statement evaluation is an IIFE; and the origin's served-bytes counter is a lower bound, not
   an equality, because the add-bar's 1-byte probe and the re-open handshakes are counted in it — 32.00 MiB of
   app bytes against 34.38 MiB served on a proxied leg.
   **The panel is also a gate check now (+2,560 B, `M0 GATE: PASS` at 4,519,936 B).** `verify-m0` drives the
   same surface the shipped app shows: it clicks Manual and waits for a line the boot state cannot already be
   showing, types an address through the real `change` handler and waits for the engine's answer
   (`Egress: manual 127.0.0.1:8199. Proxied hosts use it; …`), stores a bypass entry, then asks for the
   credentials refusal and reads `data-tone="error"`, the validator's own words, the still-editable box, and
   the untouched stored address. It ends by clicking Direct and clearing both boxes, and asserts the table
   reads `direct / null / null` — every write settled by reading `settings.list` back rather than by a sleep,
   because the change handlers return nothing to await and a proxy left stored would move the engine mirror
   under every check that runs after this one.
   What this run does **not** cover, and `verdict` in that report says so: with one enabled adapter the app
   builds no bonded chain at all, so these legs exercise `Config.proxy` on the default route and the
   `ConnectProxyConnector` head over `BoundConnector` is still proven only by step 0's spike and the unit
   tests; the registry-backed system proxy (C1/C2), proxy credentials and SOCKS are untouched — steps 3 and §7.
2. **Truth under a proxy. Chosen: R2, built so R1 stays additive** (§8 Q4). Concretely: a *manual* proxy
   takes the default route and the card says so; a host matched by `proxy_bypass` keeps its bound route,
   because §4.7 proved the bypass falls through to `BoundConnector` before any proxy socket is opened. So
   the sentence on the card is per-URL, not global — "via `proxy.example:3128`, default route" for proxied
   hosts and normal bonding for bypassed ones. This step is what makes the feature honest rather than merely
   functional, and it must not be deferred past step 1 by more than one release.
   **2 LANDED 2026-11-04 (+2,048 B → 4,521,984 B; the proof legs are `…T19-49-06Z`).** The seam is one
   question asked once per job. `engine/proxy.rs` answers it with `Egress { Unproxied, Proxied(via),
   Bypassed(via) }` and `egress(url)` — a runtime `url.parse::<Uri>()` (ureq re-exports `http::Uri`, so no
   new dependency) fed to the same mirror the card reads, plus `accepts_bypass` for the list; and
   `job.rs::download()` does `if egress.is_proxied() { routes = vec![Route::default_route()] }`. **For
   proxied hosts only** — that is §4.3's `NO_PROXY` correction kept, and `manual-with-bypass` is the leg that
   proves it: the bypassed host still gets its bound route while the proxy stays resolved and displayed on
   the card. When proxied, `attribute_egress` is skipped, so every block of the job carries
   `network_id: None` → the `UNATTRIBUTED` colour `n0` Feature 1 already had, and no adapter bucket is
   charged, so a cap cannot pace an interface that carries nothing and an allowance cannot shut one.
   The engine also writes one durable `history(…, "proxy", {via, state})` row per start whenever
   `egress.via()` is `Some` — `state` `proxied` or `bypassed` — and **no row at all** when nothing resolved,
   which is why the control leg asserts emptiness rather than a `direct` row. That is the 31st `history`
   kind (§4.5), gate-enforced.
   Three renderer edits, on top of the `n0` "Not attributed" path that already existed: `paintRoutes` now
   sums only the *credited* rows against the whole-job denominator, so the deck reads
   **`0 B of 32.0 MB attributed`** where it previously would have read `of 32.0 MB attributed` while
   crediting nothing; `#net-desc` in the Networks pane gains a per-mode sentence ("A proxy carries new links
   (`manual 127.0.0.1:8127`): their bytes are booked to no adapter, so the totals above and the caps on each
   card do not count or pace them. A host on the bypass list keeps its own route." — **this is the wording
   step 2 shipped; the unattributed-ledger change replaced the false half of it**, now "…their bytes belong
   to no adapter, so the caps and allowances on each card do not count or pace them", with the lifetime
   figure appended when the ledger is non-empty), deliberately **not** on
   the zero-adapter branch, whose totals come from `sumLibrary` and are a different claim; and `formatEvent`
   has the `case 'proxy'` line ("Via proxy" / "Proxy bypassed"). The Networks witness is in the `verify-m0`
   conjunction because that pane's own numbers are the sum of the adapter rows and a proxied link records
   nothing on any row — without the sentence, that view under-reports every proxied byte silently.
   *(That last clause held until 2026-11-05; the pane now adds the unattributed ledger to its totals, so the
   sentence explains a figure that is present rather than covering one that is missing — see §5's final row
   and `design/unattributed-bytes.md`.)*
   **Proven in the shipped exe**, seven legs, `allPass: true`, every row `verifiedBytes` 33,554,432 and
   `streams_final` 8 (`D:\RE\_cap-harness\proxy-report-2026-11-04T19-49-06-895Z.json`). The legs are ordered
   so the attribution story is a *contrast*, not four isolated reads — credited, then none, then credited:
   - **`direct-default`** — the control. Map stripes `{"n1": 33554432}`; `network:list` delta Ethernet
     **+33,554,432**, Tailscale 0; `proxy` rows `[]`; deck `of 32.0 MB attributed` with the 100% row on
     `n1` ("Default route").
   - **`system-resolves-env`** — `via: system 127.0.0.1:8127`. Stripes `{"n0": 33554432}`; **both** adapters
     grew **0**; one `proxy` row `{proxied, "system 127.0.0.1:8127"}`; deck `0 B of 32.0 MB attributed`, 100%
     row on `n0` "Not attributed" while the Ethernet row is still listed as `enabled · no bytes on this job
     yet`. 12 tunnels, 32.63 MiB counted leaving the origin through the proxy.
   - **`manual-typed`** — the same ledger from a typed address: `{"n0": …}`, both deltas 0, one `proxied` row
     for `manual 127.0.0.1:8127`, deck `0 B of 32.0 MB attributed · not measured`, 14 tunnels. The suffix is
     the live/non-live flag, not a different claim: the deck was read after the job closed, and the assertion
     is on the credited-over-denominator part only.
   - **`manual-with-bypass`** — bypass `192.168.1.2`, proxy still resolved and shown: stripes back to
     `{"n1": …}`, Ethernet **+33,554,432**, one `proxy` row with `state: "bypassed"`, deck
     `of 32.0 MB attributed · not measured` with the 100% row on `n1` "Ethernet", **0** tunnels. A global
     "bonding off whenever a proxy is set" would fail exactly this leg, which is why it is an assertion and
     not a comment.
   - **`cleared-url`** — mode `manual` with an empty address: `{"n1": …}`, Ethernet +33,554,432, `proxy` rows
     `[]`. The no-proxy-control-after-misconfiguration case, back on the credited side.
   - `refused-url` / `refused-bypass` — unchanged from step 1b, no job run.
   **The gap this step names and does not close:** under a proxy, the Networks pane's Downloaded
   Today / Week / Month / Lifetime are adapter sums, so they under-report by exactly the proxied bytes. The
   deck says so in words; the numbers do not include what no adapter carried. Closing it properly means
   unattributed bytes in the usage payload, i.e. a new rusqlite read path — **≈ 5 KB** on Feature 1's cost
   map — and that is a separate ruling, not a remainder of this one. Recorded in the open-defects list, not
   left as a footnote.
   - **CLOSED 2026-11-05 — the two sentences above are false of the shipped build and are retracted, not
     amended.** "The numbers do not include what no adapter carried" no longer holds: the pane's four totals
     add the `unattributed_usage` ledger alongside the adapter sums, and the deck sentence now names the
     amount instead of only warning. Measured **+3 584 B**, not ≈ 5 KB (§5's last row). What the step got
     right and still stands: it is a *separate* ruling from step 2, and the fix is a second ledger in the
     payload — **not** folding `n0` into the adapter sums, which would charge a byte to an interface that
     did not carry it. Proof: `proxy-run.mjs` 13 legs green on 4 573 184 B, with `unattributed_usage` growing
     exactly 33 554 432 B on each proxied leg and 0 B on the attributed ones, plus a counterfactual on the
     retained previous binary. Record: `design/unattributed-bytes.md`.
   Three harness lessons, all now commented in `proxy-run.mjs`. `.card[data-id=1]` is **not a valid CSS
   selector** (a numeric attribute value must be quoted), so cards are matched by `Number(c.dataset['id'])`.
   An assertion that a `proxy` row exists on the unproxied control leg is wrong by design — `egress` returns
   `Unproxied`, `via()` is `None`, nothing is written — so `attributionChecks` takes three modes
   (`proxied` / `bypassed` / `unproxied`) and the third expects zero rows. And the deck read raced the page
   twice: the inspector's async `fillRoutesDeck` repaints *after* the click returns, so the first version
   reported the previous leg's note; the fix blanks the stale `.attrib-note` before clicking and polls, and
   it must test selection through the DOM attribute (`.card[aria-selected="true"]?.dataset.id`) rather than
   by node identity, because `renderRows()` replaces the card nodes — a captured node reference is never the
   selected one, which is how the second attempt returned a null deck on all seven legs.
3. **Proxy credentials, Basic. CLOSED 2026-11-04 — shipped and proven in the launched exe.**
   What the scope left to decide, and what step 3 chose:
   - **Storage.** One `credentials` row, `scheme='proxy'`, keyed to the proxy authority as the engine names it
     (`host:port`, taken off the parsed ureq `Proxy`, so a written-out default port and a bare host cannot key
     two rows — `https://secure.corp` and `https://secure.corp:443` name the same credential). Finding 6
     predicted the schema needed no version for this and it was right: the CHECK already admitted `'proxy'`, and
     `db.rs:1570-1577` now asserts exactly that. No `settings` key anywhere near it, because `settings:list` echoes
     every stored value to the page — Q5's corollary, enforced by a new channel rather than a new key.
   - **Which path the credential applies to: `manual` only.** `auth_host()` returns `None` unless the mode is
     `manual`, so a system- or env-resolved proxy is used exactly as the environment describes it, including its
     own userinfo if it has any. This is a constraint, not a preference: attaching a pair to an env-resolved
     `Proxy` means rebuilding it with `Proxy::new`, and ureq does not export `NoProxy`, so the rebuild would drop
     the environment's `NO_PROXY` — a credential fix bought with a bypass regression. The card says so in words
     (`only a manual proxy takes a typed credential`) instead of pretending the pair is live.
   - **Host-match stranding.** A stored pair whose authority no longer equals the resolved proxy is *stored, not
     in use*: it is never sent to the other proxy, and the row survives an address edit so moving back restores
     it. A password riding to an unrelated host is the failure this rule exists to prevent, and the
     `auth-pair-stranded` leg proves the opposite happens: `credentialled: 0`, `authUsers: [null]`, a 407.
   - **The pair commits as one field**, because ureq sends it as one header; both boxes empty is the forget, a
     refusal keeps what was typed, a username is required whenever a secret is present, and the caps are the two
     constants at `db.rs:300-301` (256 B username / 1024 B secret). At this step `resetAllSettings` cleared
     every stored setting — the proxy trio with it — and never called `proxy.setAuth`, so a reset-to-defaults
     left a stored pair in place: forgetting a secret was not a default. That was a choice this step made, and
     it was flagged for a ruling — **which came on 2026-11-05 and reversed it:** the user ruled that a clean
     slate means the secrets too, and §6 step 8 is that ruling landed, so as of that build the button forgets
     the proxy pair and every origin credential.
   - **The typed error** is §4.5's digit-sniffing branch, with both halves now measured: `proxy-auth` (407) and
     `proxy-failed` (403). The scope's warning — *must not promise a 407-vs-403 split it cannot read* — is
     honoured by construction: only the literal digits decide the code, and an unparseable or reworded payload
     degrades to `proxy-failed` with **no** status rather than guessing.
   **Proven in the shipped exe** (`D:\RE\_cap-harness\proxy-run.mjs`, one app process for the whole life of the
   credential — absent, typed, wrong, stranded, forgotten, refused — never a restart; four servers now: the
   Range origin, an open proxy, an authenticating copy, and a third open-but-unwilling instance that answers 403
   to `CONNECT` for the origin's own port):
   - **`auth-asked-nothing-stored`** — manual proxy at the 407 fixture, nothing stored: `errorCode
     "proxy-auth"`, hint `the proxy refused the tunnel: proxy server responded 407/407`, 1 CONNECT asked and
     **0** credentialled, `refused: ["407"]`, 0 tunnels, the origin served **0 B**, both adapters `grew 0`, one
     durable `proxy` row `state: "proxied"`, and the card reading `No credential stored. A proxy that asks for
     one refuses the link.`
   - **`auth-pair-typed-in-card`** — `sucr` / `pw1` typed through the page: **every** CONNECT (10-14 across
     runs, at `streams_final` 8) carried the header with `passMatch: [true]` and
     `refused: []` — the pre-send shape §4.4 predicted, no challenge round — 32 MiB finished, stripes
     `{"n0": 33554432}`, deck `0 B of 32.0 MB attributed` — the step-2 attribution rule holds with a
     credential in play — the password box back to `passLen 0` after the save, and the secret absent from the
     answer line **and** from the whole `app:get-info` payload.
   - **`auth-wrong-password`** — the pair *is* sent and refused for its second half: `credentialled 1`,
     `authUsers ["sucr"]`, `passMatch [false]`, `refused ["407-pass"]`, and the app calls it the same
     `proxy-auth`, because that is what the digits say. The wrong half is not echoed anywhere. This leg closes
     the §9 item that said a wrong password was unproven — as distinct from an absent header.
   - **`auth-pair-stranded`** and **`auth-pair-forgotten`** — the two ways a pair stops applying: moved address
     (`applies: false`, "stored, not in use", nothing travels) and empty pair (back to the first leg's 407
     refusal from the other side, `stored: false`, without a restart to notice).
   - **`proxy-refuses-port`** — the other half of the split: an **open** proxy (no credential asked, none
     stored) that answers 403 to `CONNECT` for the origin's port. `errorCode "proxy-failed"`, hint `...: proxy
     server responded 403/403`, `refused ["403-port8097"]`, `credentialled 0`, 0 tunnels, nothing credited. A
     build that coded every `ConnectProxyFailed` as `proxy-auth` — or that sniffed for "40" — fails this leg,
     and it is the leg §4.5 said must not be promised without measurement. It is now the sixth step-3 leg, so
     both codes have left the exe, not only the unit test.
   Three harness traps found on the way, all commented in the driver. A **stale bypass entry is invisible on the
   surface**: step 2's `manual-with-bypass` leg leaves `192.168.1.2` exempt, so a credential leg would run
   direct, still show the proxy in `via()`, and prove nothing — the bypass is cleared once before the legs and
   every refusal leg now asserts `proxy_bypass === null` first. Second, the fixture must print **one line per
   CONNECT as it parsed it** (`head: true`) *before* it decides: counting `allow` lines loses every refused
   request (`credentialled` read 0 on legs where the header had been sent), and filtering on "not allow, not
   reject" double-counts, because an accepted CONNECT also emits an `allow` line and a `tunnel: closed` line
   (24 asks against 12 tunnels). Third, `waitForAuthNote` cannot see this leg: changing only the secret leaves
   the credential sentence exactly as the previous leg wrote it, so the wrong-password leg sampled the box
   while it still held 16 bytes — hence `waitForCredentialReleased`, which is bounded and returns the stale read
   instead of throwing, so a box that never releases fails one leg rather than aborting the run.
   **Not covered by this step, and still open:** proxy Digest (§4.4's hard limit), SOCKS5, the registry-backed
   system read, all origin auth, and every real corporate proxy. The deck's `· not measured` suffix appeared on
   some legs and not others across runs — a pre-existing race between the harness's deck read and the
   inspector's final repaint, unrelated to step 3; the credited-over-denominator half of that line is stable.
4. **Origin Basic + the paste/prompt surface.** `scheme='basic'`, one row per host, `Authorization`
   injected through the middleware (H1), the link check distinguishing `needs-auth`.
   **4 LANDED 2026-11-04 (+27,648 B → 4,562,432 B; 680,448 B under the ruled 5 MB ceiling, re-derived from
   5,242,880 − 4,562,432 rather than remembered; the proof legs are
   `D:\RE\_cap-harness\auth-report-2026-11-04T21-57-58-346Z.json`).** What the scope left to decide, and
   what step 4 chose:
   *(Reading note: the sentences quoted through the rest of this step — "this link needs a password", "the
   stored password was refused", "Add a password for this host" — are what the exe said **as step 4 measured it**.
   Step 7 reworded nine of them to the cover noun "credential" and re-ran every leg, so the current prose is
   §6 step 7 and the current assertions are `auth-run.mjs`; the behaviour each quote witnesses is unchanged.)*
   - **Keying: the host alone.** `engine::host_key(url)` lowercases and strips port, userinfo and path, and
     the middleware looks a request up by `uri().host()` — the same rule on both sides, so a pair written
     from `http://127.0.0.1:9/package.zip` answers for every port and path of `127.0.0.1`. The consequence is
     named in the card's own words rather than left to be discovered: a pair travels to a whole host, and
     pre-emptive Basic over plain `http://` is base64-readable on the wire. It also decides what the
     launched-exe isolation leg had to be: **two hosts, never two ports** — a pair saved for `192.168.1.2`
     proving nothing about `127.0.0.1` is the claim, and a port-keyed store would pass the same leg by
     accident.
   - **Injection: pre-send, one attachment site.** `engine::auth::middleware()` is the only place an origin
     `Authorization` is written, carried by every agent `config_ua()` builds, so both 401 sites get it without
     either one knowing about the store. A request that already carries the header is left alone
     (`authorization_for(host, already)` returns `None` when `already`), so nothing here overwrites a header
     another layer put there. `redirect_auth_headers(RedirectAuthHeaders::SameHost)` (`engine/mod.rs:186`) is
     set *because* of the host-only key: ureq's default `Never` would drop the pair on the first hop, which is
     a different rule from the one the store applies. Both are test-proven in Rust (`the_pair_is_on_the_request_as_it_leaves`,
     `a_same_host_redirect_keeps_the_pair`).
   - **The two readings come from the engine, not from the page.** `probe_link` and `job.rs::single_attempt`
     each ask `engine::auth::has(&host)` — the mirror, not the database — and put the answer in the payload as
     `auth: bool`. That is the only thing that lets the link check say *"the stored password was refused"*
     rather than *"this link needs a password"*, and the renderer's `verdictLine` switches on that boolean and
     invents nothing. The old unreachable `case 401` under `http-status` was deleted when `needs-auth` arrived,
     so a 401 has exactly one code path.
   - **A 401 names what the host offered.** `engine/mod.rs:308-322` reads `www-authenticate` through
     `auth_schemes()` — a fixed list of six names matched by substring, deliberately not a comma split, because
     `Basic realm="Files, internal"` would otherwise read `internal"` as a scheme — and says
     `server needs a password (offers basic)` when Basic is on the menu and
     `server needs a password, and offers digest; this app sends only Basic` when it is not. An offer this app
     cannot read is reported as no scheme, never as a scheme it might not be. **Step 5 reworded both halves of
     that match** — the sentence above is what step 4's leg 7 measured, and it is no longer true of the binary:
     Basic and Bearer are now named as offers it can answer, and the disclaimer is reserved for the rest.
   - **One channel, and the forget rides it.** `origin:set-auth(link, username, password)` (the 35th) does
     save, replace and forget: both halves empty is the forget, so a per-row Forget button needs no second
     channel and no new list read. `app_info` gained `originAuths` (host, username, **secretBytes**, createdAt)
     instead of a `origin:list` channel; `db.rs:433 list_origin_auths` selects `length(secret_blob)` and the
     write read-path never selects the blob at all, so the secret has no route to the page. `put_origin_auth`
     deletes only that host's `scheme='basic'` row — a site write cannot wipe a proxy credential stored against
     the same authority. The schema CHECK at `schema.sql:38` already admitted `'basic'`, so finding 6's
     "no schema version owed" holds here too. Caps (256 B username / 1024 B secret) live in `db.rs` only, and
     the nameless-password refusal explains itself: *a password needs a username: the two go out as one header,
     and a password without a name before it is never sent*.
   - **The surface is two cards and one bridge, no new CSS.** A Settings › Network *Site credentials* card
     between the proxy card and the browser sessions one (rows reusing `.session-row`, empty state
     `No site credentials stored.`), the add-bar line for the verdict, and — for a job that died on a 401 — an
     inspector button that reads `Add a password for this host` or `Replace the password for this host`
     depending on `originFor()`, which opens the card with the host prefilled. The password box is a
     replacement field: a stored pair reads back empty, same rule as the proxy credential and the session
     cookie. The disclosure sentence is the one Q5 asked for, repeated here because the file is the same file.
   **Proven in the shipped exe** — `auth-run.mjs`, eight legs against one launched `sucr.exe` (own
   `--data-dir`, CDP 9347, real `.click()` on the card and the add bar), plus a restart, against two instances
   of `server.mjs` now able to answer 401:
   - **`absent pair, link check`** — 1 request, verdict `absent`, 1 challenge; the line reads *This link needs
     a password (401).* with the engine's `(offers basic)` on its `title`, and Download is disabled.
   - **`pair typed through the card`** — `sucr`/`pw1` typed into the real boxes and committed by clicking Save:
     note *Saved for 127.0.0.1; every request to that host carries it now.*, one row keyed `127.0.0.1` reading
     `sucr · 3 B password`, the password box back to 0 bytes, the secret absent from the row **and** from the
     whole `app:get-info` payload, and the same link re-checked with **0 challenges across 2 accepted
     requests** — the probe path pre-sends.
   - **`32 MiB with the pair pre-sent`** — the transfer path: `finished`, `verifiedBytes` 33,554,432, a 32 MiB
     file on disk, `streams_final` 8, and the fixture's own account of **13 ranged requests, 13 `ok`, 0
     challenges**. This is the leg §4.4's H1 prediction could only assert from library source: no stream ever
     waited for a 401. (Served 34,340,866 B against a 33,554,432 B file — step 1b's caveat holds: the fixture's
     counter is a floor, ranges were re-served.)
   - **`wrong pair refused`** — the same username with the wrong secret: the link check says *The password
     stored for this host was refused (401).* (the `auth: true` reading, not a guess made in the page),
     Download stays disabled, the job ends `needs-attention` / `errorCode needs-auth` after **1 `mismatch`
     request with no file published**, and the card offers `Replace the password for this host` beside the line
     *A credential is saved for 127.0.0.1. Resume this job to send it again with that pair.*
   - **`forgotten from the row`** — the Forget button on the row: *Dropped the credential for 127.0.0.1.*, no
     row with a button left, `originAuths` empty, and the link back to *This link needs a password (401).* with
     both requests `absent`.
   - **`a pair answers for its host only`** — saved for `192.168.1.2`, then both addresses of the *same file on
     the same port* probed in one leg: the LAN address checks out, the loopback address still reads *needs a
     password*; fixture `1 ok / 2 absent / 2 challenges`. Host keying measured, not assumed.
   - **`a digest-only offer is named`** — a second fixture that always answers 401 offering Digest: the line is
     the needs-a-password reading, its `title` carries *and offers digest; this app sends only Basic*, Download
     disabled, and the fixture saw **1 `digest-only` challenge and 0 accepted requests** — the app never
     attempted a scheme it does not implement.
   - **`survives a restart`** — the harness instance was killed and relaunched on the same `--data-dir`:
     `originAuths` came back carrying `192.168.1.2/sucr` with the secret still absent from the payload, and the
     same link checked out with **0 challenges**. So `engine::auth::configure(rows)` at boot is measured, not
     merely written — a credential that only lived in the mirror would have failed exactly here.
   Four harness traps, all commented in the driver now. **Download has a second gate besides the link**: a
   save-to folder that does not exist also greys it out (`addCheck.extraBlock = () => addFolderEnsure.missing
   !== null`), so the first run's `the same link now checks out` leg failed on a disabled button while the
   engine had already accepted the link twice (`…T21-55-27-428Z.json`); the fix is to point `#add-folder` at
   the run's own output directory, wait out its 400 ms debounce, and record `folder` +
   `folderNoticeHidden` in every probe read so a disabled Download is never read as the link's doing. Second,
   **re-opening the add bar re-probes the URL still in the box** (`openAdd(true)` → `if (url) void runCheck(...)`),
   which is why several legs count two probes for one leg — the extra requests are the app's own, and each one
   carries the credential state the leg claims. Third, the empty state shares the `.session-row` class with a
   real row, so "no rows left" must count only rows carrying a Forget button. Fourth, a backtick inside an
   in-page comment terminates the driver's own template literal at parse time.
   The clean run's `verdict` raises six flags step 3 could not have written: `originBasicPreSent`,
   `originBasicOnBothErrorSites`, `refusedPairDistinguished`, `hostKeyedIsolation`, `survivesRestart`,
   `digestOfferNamed` — each one is a leg's assertion, not a summary. It adds three more as `false`, because
   the rule that governs a flag is that the outcome has been *observed leaving the exe*: `bearerCovered`
   (step 5), `digestCovered` (step 6) and `httpsOriginCovered` (no fixture here is TLS). `digestOfferNamed`
   and `digestCovered` are deliberately two flags and not one: the first is the app naming an offer it will not
   honour, measured; the second would be a working Digest computation, which no code in this app contains.
   **Not covered by this step:** bearer / header-secret paste (step 5), Digest (step 6), an `https://` origin
   (both fixtures are loopback `http://`, so the "readable on the wire" sentence in the card is the honest
   statement of what was *not* tested here), and `resetAllSettings` — which at this point cleared neither the
   proxy credential nor this one. Both are forgotten by the button now; see §6 step 8, the user's ruling of
   2026-11-05.
5. **Bearer / header-secret paste.** **LANDED 2026-11-04 (+5,120 B → 4,567,552 B; 675,328 B under the ruled
   5 MB ceiling; proof legs are `…T22-49-08Z` and the re-run `…T22-56-54Z`, 14 legs / 87 checks green).**
   §2 finding 6 paid for itself here: the second scheme needed no schema version, because `schema.sql:38`'s
   CHECK already admitted `'header-secret'`.
   - **The store grew a kind, not a table.** `engine::auth::Kind { Basic, Bearer }` with `scheme()` /
     `from_scheme()` mapping to `'basic'` / `'header-secret'`; `Credential { kind, username, secret }`;
     `usable(kind, username, secret)` is the single rule for "can this become a header at all" (a token needs
     only the secret, a pair needs both), so the two kinds differ in one field and one predicate, not in two
     parallel paths. `authorization_for` emits `Basic ${b64}` or `Bearer ${secret}` — a token goes out
     **verbatim**, no base64, no padding repair. `put_origin_auth(conn, host, kind, …)` still keys on
     `engine::host_key` and still deletes only `scheme IN ('basic','header-secret')`, so a site write can never
     wipe a `'cookie'` row or a proxy credential. One credential per host, and now measured rather than
     asserted: a request carries one `Authorization`, not two.
   - **No 36th channel.** `origin:set-auth(link, username, password, kind)` — `kind` optional, accepting
     `"basic"` or `"header-secret"`, refusing anything else with `origin:set-auth kind expects basic or
     header-secret, not {other}`. A new channel moves four snapshot sites (gate mechanics), and this feature
     needed none of that. `app_info.originAuths` entries gained `kind`; the secret still has no read path —
     `list_origin_auths` selects `length(secret_blob)` and nothing selects the blob.
   - **The surface is the same card with a switch.** `#origin-kind` is a `.seg-control` (`data-val`,
     `aria-pressed`) and deliberately carries no `data-set`, because that attribute is the settings-write hook;
     it binds locally and decides which boxes the one Save button commits. In token mode the username box is
     hidden — `[hidden] { display: none !important; }` already existed at `app.css:35`, so that cost 0 CSS
     bytes — and the row reads `12 B bearer token` where a pair row reads `sucr · 3 B password`. The secret box
     stays a replacement field for either kind.
   - **`has()` and `auth` stayed kind-agnostic on purpose**, which is what let step 4's two-reading UX extend to
     tokens for free: "this link needs a credential" and "the credential stored for this host was refused" are
     the same pair of sentences whatever the host was offered.
   - **What the proof then caught.** On a host that offers `Bearer`, the engine's own message still read *this
     app sends only Basic* — a claim the same binary contradicted two legs later when it sent a token and got
     32 MiB. `engine/mod.rs:308-322` now names Basic and Bearer alike as offers it can answer
     (`server needs a password (offers bearer)`) and reserves the disclaimer for what it cannot:
     *this app sends Basic or a bearer token*. The reword was measured, not assumed — relink left the exe at
     **4,567,552 B unchanged**, so a longer `format!` literal inside an existing match arm cost 0 B at the
     section's granularity, and the binary string-probes 1 hit for the new sentence, 0 for the old.
   - **Proven in the shipped exe** — `auth-run.mjs`, six bearer legs added to step 4's eight, all against the
     launched `sucr.exe` with its own `--data-dir` over CDP 9347, real clicks on the switch, the boxes and the
     Forget button, and a third `server.mjs` instance running `REQUIRE_BEARER=…`:
     `bearer host, nothing stored` (verdict `absent`, Download disabled, and the offer named without a
     disclaimer); `token typed through the card` (username box hidden, row reads `12 B bearer token`, the token
     absent from every row **and** from the whole `app:get-info` payload, `kind: header-secret` with
     `username: ""`, the LAN pair untouched, then `finished` at 33,554,432 B verified over 11 requests with
     **0 challenges** and `streams_final` 8); `wrong token refused` (*refused (401)*, not *needs*, `mismatch`
     only, `errorCode needs-auth`, no file); `the kind written second owns the host` (one row per host through
     a pair→token→pair→token flip, and the bearer host answers only the kind written last); `forgotten,
     whichever kind it was` (the row's Forget clears a token row on the same host-only call, and the neighbour
     host stands); `a token survives a restart` (after a kill and a fresh process, the library reads
     `{header-secret, "", 12}` back and the link checks out with 0 challenges).
   **Not covered by this step:** answering a Digest offer (step 6 — this proves only that a Digest-only host is
   *named* honestly), an `https://` origin, and `resetAllSettings`, which at that point cleared neither the proxy
   credential nor this one — both are forgotten by it now (§6 step 8).
   **Also open, and deliberately not changed here — the wording audit, re-counted against the source after the
   step-5 edit rather than from memory:** the prose that calls *every* credential a password is **nine strings at
   six sites**, not three — `main.ts:3827` (`Add a password for this host` / `Replace the password for this host`),
   `main.ts:4405` and `:4406` (`The password stored for this host was refused (401).` / `This link needs a password
   (401).`), `engine/job.rs:3271` and `:3273` (`the stored password was refused` / `server needs a password`), and
   `engine/mod.rs:317`, `:320`, `:322` (the three link-check offer sentences this step just reworded).
   **Already kind-aware, so step 7 does not owe them:** the secret box's placeholder and `aria-label`
   (`bearer token, exactly as pasted` / `Token for that host`, `setOriginKind`), the saved-row meta grammar
   (`12 B bearer token` vs `sucr · 3 B password`, `renderOrigins`), and the write path's own validation, which
   branches on a local `noun` and guards the `a password needs a username` refusal behind `!token`
   (`ipc.rs:1238-1258`) — so that message can never be shown for a token. The proxy box's `password` placeholder is
   correct: the proxy takes only Basic.
   The bearer legs assert the six-site sentences as they now stand, so the proof covers the **current** prose, not a
   token-aware one; step 7 owns that decision, and rewording means re-running `auth-run.mjs`, not only the gate.
   **Step 7 took that decision and closed it the same day** — all nine strings reworded in source, the exe relinked
   at 0 B, and the bearer legs re-pointed and re-run (§6 step 7, §9).

6. **Digest**, if and only if Basic's real usage shows servers that need it. Deliberately last.
   **Closed as deferred, with the precondition checked rather than assumed (2026-11-04).** Two halves, and they
   resolve differently:
   - **Proxy/CONNECT Digest is unreachable in the vendored ureq, not merely unwritten.** Whole-tree search of
     `ureq-3.5.0` returns **zero** matches for `digest`; the one credential the connector can write is
     `Proxy-Authorization: Basic` at `connect.rs:124`, built unconditionally from `Proxy::username()/password()`.
     And the reader loop that takes the CONNECT answer breaks on the **first** response and maps any non-200 to
     `Error::ConnectProxyFailed` (`connect.rs:147-155`) — so there is no 407-challenge round in the tunnel at all:
     a wrong-or-absent proxy credential is a hard failure, never a retry with a digest. Reaching it would mean
     forking the connector and re-issuing a second CONNECT, which is a different transport path from the one every
     step of this slice has been measured on. This is why §7 lists it as out of scope rather than as TODO.
   - **Origin Digest is implementable but not owed.** The crypto is genuinely free — `md-5 = "0.10"` and
     `sha2 = "0.10"` are direct dependencies (`Cargo.toml:23`, `:28`), used by the checksum verifier at
     `engine/mod.rs:680` and `:682` — and a 401 arrives as a readable `Response` because
     `http_status_as_error(false)`, so the challenge could be parsed and answered on the existing middleware.
     What is missing is the *trigger the step wrote for itself*: "if and only if Basic's real usage shows servers
     that need it". This app has no usage telemetry and no field reports, and the only Digest-only host this
     project has ever met is the fixture step 4 built to prove the naming — **no real host has been surveyed, so
     this is an absence of evidence, not evidence of absence**, which is what §9's own non-assertions say and why
     the trigger was written as a usage condition rather than a deadline. What the deferral costs the user today
     is one sentence: on a digest-only host the 401 is **named** in its own words (`server needs a credential, and
     offers digest; this app sends Basic or a bearer token`) and nothing is sent, which the step-4 fixture records
     as 0 accepted requests. Building the biggest and most-likely-wrong chunk in the feature to erase that
     sentence would be designing against an invented requirement — the error §9's `A2` flag keeps naming.
   - **Re-entry condition, stated so a future reader does not re-litigate it:** origin Digest becomes owed when a
     real host answers 401 with `WWW-Authenticate: Digest` and nothing Basic-shaped, and the user has been shown
     the named-offer sentence on it. What is already paid for, so the step is cheaper than it looks: the crypto is
     linked (`md5::Md5` at `engine/mod.rs:680`, `sha2` at `:19`); `schema.sql:38`'s CHECK list has named `'digest'`
     since before this app could use it, so **no schema version is owed** — the same finding that made bearer cost
     one field and one switch; `engine::auth::Kind` already has the from-scheme/shape seam, and `basic_pair`
     (`auth.rs:156-160`) was left public with a comment naming this step, because Digest's `HA1` is the same
     `user:password` string. What is still all cost is the challenge parse, `qop`, the stale-nonce retry and a
     `cnonce` CSPRNG — the 4 – 8 KB estimate of §5, which no measurement in this project has yet confirmed or
     denied.
7. **Docs, gate count, and launched-exe proof.** New `history` kinds recorded; `verify-m0.mjs` checks
   extended (proxy mode validated in `validate_setting`, channel count if it moved); the feature proven in
   the shipped exe over CDP against the harness proxy — the standard every slice has been held to.
   **7 LANDED 2026-11-04 (0 B — exe 4,567,552 B before and after; both launched-exe suites re-pass against
   the same build).** Two halves, and the first one turned out to owe nothing:
   - **The bookkeeping half was already true, so it was verified rather than done.** No `history` kind was added
     by steps 3 – 7, so the build gate still prints `history kinds: 31`; no channel was added after step 4, so
     `app:get-info` still answers `channelCount: 35` (the harness reads it as a preflight assertion, not a log
     line); the setting-key count is unchanged at 20; and `verify-m0.mjs` still has exactly **50** `check(`
     sites, counted with `grep -c` rather than remembered. The only *new* assertion in this slice is the one
     step 7 added to `auth-run.mjs` — that the inspector's own line uses the cover noun — which is a harness
     byte, not a product byte.
   - **The wording half: the nine strings, and the grammar chosen for them.** Chosen under "do as you see fit",
     and the rule is one line: **a site says "credential" when it cannot know the kind, and says the exact kind
     when it can.** So —
     `engine/mod.rs:319`, `:322`, `:324` (was `:317`/`:320`/`:322`): the three link-check offer sentences now
     read `server needs a credential (offers {s})`, `server needs a credential, and offers {s}; this app sends
     Basic or a bearer token`, `server needs a credential`. That function parses `www-authenticate`, which says
     nothing about what the user stored, so it is the definition of a kind-blind site.
     `engine/job.rs:3272`, `:3274` (was `:3271`/`:3273`): `the stored credential was refused` / `server needs a
     credential` — the same pair, distinguished by the middleware's own record of whether a credential rode the
     failed request, still not by guessing a kind.
     `main.ts:3829`/`:3831`/`:3832` (was `:3827`, one branch): the inspector button grew from two answers to
     three — `Add a credential for this host` when the host has no row, then `Replace the token for this host`
     or `Replace the password for this host` chosen from `held.kind`. This is the case that proves the rule is
     not "always say credential": the kind *is* knowable here, so the button says which secret it will replace.
     `main.ts:4411`/`:4412` (was `:4405`/`:4406`): the link-check verdict `The credential stored for this host
     was refused (401).` / `This link needs a credential (401).`, switching on the probe's `auth` boolean only.
     `index.html:587`: the host box's placeholder — `host, or the link that needs a credential`.
     **Deliberately not touched, because each already names the right kind locally:** `ipc.rs`'s
     `a password needs a username: the two go out as one header…`, which is guarded behind `!token` and so can
     never be shown about a token; the length refusal, which branches on a local `noun`; the saved-row meta
     (`sucr · 3 B password` vs `12 B bearer token`); and the **proxy** box's `password` placeholder, which is
     correct because the proxy takes only Basic (`engine/proxy.rs` has one credential shape, and §6 step 6 says
     why a second is unreachable).
   - **Cost of the reword: 0 B, and the claim is proved against the linked binary, not the source.** `stat`
     before the edit and after the relink both read **4,567,552 B**. String-only edits inside existing match
     arms and existing DOM attributes stay inside the 512 B section padding the previous link already carried —
     the same finding step 3's 148 B disclosure sentence and step 5's engine reword each produced. Because a
     size match alone cannot distinguish "free" from "not linked", the contents were probed in the exe:
     **present** — `server needs a credential (offers`, `this app sends Basic or a bearer token`,
     `The credential stored for this host was refused (401).`, `A credential is saved for ${l.host}`,
     `Add a credential for this host`, `Replace the token for this host`, `or the link that needs a credential`;
     **zero matches** — `server needs a password`, `this app sends only Basic`, `the stored password was refused`,
     `This link needs a password`, `needs a password`, `Add a password for this host`.
     `Replace the password for this host` still hits, and should: it is the Basic branch of the ternary.
   - **The gates the reword had to pass, in order:** `npm run typecheck` clean → `cargo test --release --lib --
     --test-threads=1` **196 passed, 0 failed, 6 ignored** in 50.71 s → `npm run verify` **M0 GATE: PASS (exe
     4.36 MB)**. Then the two launched-exe suites, both against the *rebuilt* exe: `auth-run.mjs` re-pointed to
     the new prose first (14 legs, **88 checks** — one assertion added — `allLegsPass: true`,
     `auth-report-2026-11-04T23-29-35-575Z.json`), and `proxy-run.mjs`, its needles unchanged, **13 legs, `allPass: true`**
     (`proxy-report-2026-11-04T23-31-33-568Z.json`) — re-run not because its needles moved but because the engine's
     401 arms and the shared refusal prose were just edited under it. The auth report carries no secret:
     `pw1` appears **0** times, the bearer token's `eyJ` prefix **0**, and the only credential facts recorded are
     a username, a byte length and a `kind`.
   - **Why the harness moved before the exe did:** these nine sentences are asserted verbatim by `auth-run.mjs`,
     so a reworded string with a stale assertion is worse than the old wording — it makes a green run mean
     nothing. The needles were re-pointed, the run re-passed, and only then was the wording claimed shipped.
   **What step 7 does not close:** `resetAllSettings` cleared neither credential — that gap is what step 8 turns
   into a ruling rather than leaving a held defect; no `https://` origin fixture; no SOCKS5; no registry-backed
   system proxy read; Digest deferred per step 6.
8. **Reset-all forgets the credentials — `DECIDED`.** The user ruled on 2026-11-05, in their own words:
   *"resetAllSettings should clear credentials, go ahead"*. This is the slice's first `DECIDED`; Q1–Q6 stay
   **chosen** under delegation (§8). Nothing about the ruling is inferred here: the words, the date, and the
   button it names are the whole of it.
   - **What was wrong.** The button nullled every setting key and stopped. The credential stores are *not*
     setting keys — deliberately, because `settings:list` echoes every stored value to the page and a password in
     `settings` would cross the bridge on every Settings open (Q5's corollary) — so a clean slate left three
     kinds of secret standing in `credentials`: the proxy's Basic pair, an origin pair, an origin token. Harmless
     in effect, invisible in the panel afterwards, and exactly the shape a "reset" button must not leave.
   - **The design choice: call the two forgets, do not build a third path.** Both already exist as the shipped
     write path for an empty pair — `proxy:set-auth("", "")` (`ipc.rs:1164-1172`, which clears the row *and*
     `engine::proxy::set_auth`) and `origin:set-auth(link, "", "")` (`ipc.rs:1241-1248`, which clears the host's
     row *and* `engine::auth::set`, and `auth.rs:113-127` **removes** the key once the pair is unusable rather
     than storing an empty one). A third channel that deleted both tables in Rust would have been a second
     implementation of forgetting, free to drift from the rows a user clicks. So the renderer reads the hosts
     first (`appInfo.originAuths`, because the secret itself is never in the page and the host is the whole key),
     nulls the keys, then fires one proxy forget and one forget per host. Cost of that decision: 0 product bytes,
     `CHANNELS` unmoved at 35, the gate's `check(` count unmoved at 50, and no new write path to prove.
   - **What it says.** `resetAllSettings` (`main.ts:2563`) ends on the shipped note *"All preferences and stored
     credentials reset to defaults."* — the sentence is the claim, so it had to change with the behaviour, and
     `auth-run.mjs` asserts it verbatim. The click handler (`main.ts:2977-2978`) and the button markup
     (`index.html:695`) are untouched.
   - **Proven in the exe, and only in the exe: leg 15 of `auth-run.mjs`.** Fifteen launched-exe legs now, this
     one new: stage a manual proxy and store a pair for it **through the panel**, save an origin pair and an
     origin token **through the card**, read both stores and the engine's own summary, click the real
     `#set-reset-all` in the System pillar, then read again and *use the app* — two link checks, one per scheme,
     each of which must answer `This link needs a credential (401).` with the fixture tallying `absent` only. A
     table that emptied while the mirror kept sending would show up there as an accepted request, which is the
     only failure mode of this feature a user would actually feel. 18 checks, all green: both stores empty
     (`afterList.keys` `[]`, `proxyAuth.stored` false), both kinds dropped and not just the pair (before
     `basic,header-secret`), no Forget row left on the card, the proxy panel back to four empty fields with the
     `password` placeholder, the settings table holding no `8199`, `app:get-info` echoing no secret either
     (`echo: false` before *and* after — the witness of a secret is a witness that it is not there), and the two
     live link checks reading anonymous.
   - **Cost: 0 B, proved linked.** `stat` reads **4,567,552 B** before and after the relink, and a size match
     alone cannot tell "free" from "not linked", so the exe was probed: the new sentence **1** hit, the old
     `All preferences reset to defaults.` **0** hits. The renderer is the newest file in the tree and still older
     than the binary (`main.ts` 05:22:04 vs `sucr.exe` 05:23:16), so the embedded bundle is the one under test.
   - **The gates, in order:** `npm run typecheck` clean → `npm run build` relink in 1 m 03 s → `cargo test
     --release --lib -- --test-threads=1` **196 passed, 0 failed, 6 ignored** (50.92 s) → `npm run verify`
     **M0 GATE: PASS (exe 4.36 MB)**. Then both launched-exe suites on the same build: `auth-run.mjs` **15 legs,
     106 checks, `allLegsPass: true`** (`auth-report-2026-11-05T00-08-40-062Z.json`), and `proxy-run.mjs`
     re-run as the regression sweep it is — the shared proxy panel and its credential boxes sit under this
     button now — **13 legs, `allPass: true`** (`proxy-report-2026-11-05T00-10-04-967Z.json`). Neither report
     carries a secret: `pw1` **0** hits, the token prefix `eyJ` **0**.
   - **Two failed runs, both the harness's fault, and the fixes are the durable lesson.** Run 1 failed five
     reset checks: the fixture had written `network.proxy_mode`/`network.proxy_url` *through the bridge*, so the
     page's own `storedSettings` never held those keys, `Object.keys(storedSettings)` skipped them, and the link
     checks then routed through a dead manual proxy the reset had not cleared. **The product was right; the
     fixture had built a shape no user can reach.** Fixed by `ui.proxyStage`, which presses the segmented control
     and fires a real `change` off the URL box exactly as `proxy-run.mjs` stages a proxy — the driver rule
     "nothing behind the panel's back", now written down in this suite too. Run 3 failed one *setup* check for a
     different reason: it asserted `passwordBytes === 3` off `app:get-info`, and the proxy summary is
     `{stored, username, host, applies}` (`ipc.rs:1130-1145`) — the byte count exists only on the
     `proxy.set-auth` **reply**, which the page consumes internally on its own commit and a driver that clicks
     real nodes never sees. The assertion now claims what the engine actually answers: stored, for
     `127.0.0.1:8199`, and `applies === true` — the engine saying this pair is the proxy in front of the app,
     which is the stronger setup claim for a reset test anyway.
   - **Held rule this step had to respect:** the ladder never crosses the bridge. No harness output, report or
     design quote carries a password or a token; the boxes are replacement fields and the only length recorded is
     in a `check` label for a value that is never stored.

---

## 7. Explicitly not in this scope

NTLM, Kerberos/Negotiate, PAC/WPAD parsing, SOCKS4/4A, per-proxy connection pooling beyond ureq's own pool,
Proxy-Digest for CONNECT (unreachable without forking the connector, §4.4 — checked against the vendored crate at
step 6, not assumed: `ureq-3.5.0` has zero `digest` matches anywhere in its tree and exactly one
`Proxy-Authorization` write site, `connect.rs:124`), TLS-to-origin pinning, `-k`
style insecure skip (the M7 CLI row already rules it out, `app-design-plan-v2-engine.md` §10 M7), and any
effect on the app's **own** WebView traffic — the setting in this document governs engine traffic only.

---

## 8. The six questions, each with an answer on the table

The user ruled the budget and said to scope proxy + auth, then delegated the rest ("do as you see fit"), so
each answer below is **chosen by this agent and carries the measurement it rests on**. None of them is
`DECIDED` — only the user writes that, and any of these can be overwritten. Where the evidence changed the
recommendation as written on first pass, that is said.

- **Q1 — System proxy: yes or no?** **Chosen: yes, and through C2** (the already-linked `windows` 0.62, not
  ureq's `win-system-proxy`). Evidence for the "not C1" half is firm: `win-system-proxy = ["dep:winreg"]`
  (ureq `Cargo.toml:114`) and `winreg` is in `Cargo.lock` only as a **build**-dependency of
  `embed-resource` (`Cargo.lock:671`), so C1 newly links a crate. C2 also answers WPAD for free. What is
  **not** measured: nothing in step 0 touched the registry — the spike exercised `manual` only. So the C2
  branch is the one §4.7 lists as still read-only, and step 1's system mode is where it gets proven.
- **Q2 — Accept CONNECT for `http://` targets, ureq-style?** **Chosen: yes.** Step 0 measured both halves:
  the harness proxy tunnelled `CONNECT 127.0.0.1:8898` and `CONNECT localhost:8898` to a 200 with the full
  body, and a second instance set to refuse that port produced a legible
  `CONNECT proxy failed: proxy server responded 403/403` rather than a hang or a silent fallback. So the
  compatibility risk is real, bounded, and *reportable* — which is the only thing an absolute-URI
  hand-written request line would have bought, at real bytes and a second divergent path.
- **Q3 — Is origin Digest in this feature?** **Chosen: deferred, deliberately last (step 6), and opened only
  if Basic's real usage shows servers that need it.** Unchanged from the recommendation; step 0 said nothing
  about it, because no origin-auth code exists yet and the spike tested *proxy* Basic (407), not origin
  Basic (401). **Step 3 changed nothing here either** — it shipped the *proxy* credential and the typed proxy
  error, and left every origin-scheme row untouched; what it did close is the other §9 unknown in this area: a
  **wrong** proxy password is now measured, not inferred (it is the same `ConnectProxyFailed` shape, refused
  with the same 407, and the app calls it `proxy-auth` because that is what the digits say). **Step 4 changed
  the half of this row that was about Basic, not the half about Digest:** origin Basic now exists, is pre-sent
  through the middleware, and is measured in the shipped exe — so this ruling is now the narrower and more
  honest thing it always was, *Digest deferred past a working Basic*, with no origin-auth absence left to
  describe. The digest-leg measurement is the first sight of what a Digest-only server looks like to this app:
  the offer is named in the 401's own words, nothing is sent, and the fixture records 0 accepted requests —
  which is the cost of the deferral stated as a number rather than as a worry. **Step 6 ran the precondition and
  the answer is still no, now on evidence rather than on a hunch** (`md5`/`sha2` linked, `'digest'` already in
  `schema.sql:38`'s CHECK list so no schema version is owed, but no usage exists that could show a server needing
  it), and the same step found the *proxy* half of Digest architecturally unreachable in ureq 3.5.0 — zero `digest`
  matches in the whole crate, one `Proxy-Authorization: Basic` write site (`connect.rs:124`), and a CONNECT reader
  that fails the tunnel on any non-200 instead of retrying with a challenge (`connect.rs:147-155`). That half is
  therefore out of scope permanently, not deferred: §6 step 6 and §7 both say so.
- **Q4 — Re-point attribution at the proxy (R1) or turn bonding off (R2)?** **Chosen: R2 first, built so R1
  is additive**, and the card must state which is happening — per URL, not globally. Step 0 is what turned
  this from a preference into a finding: `egress_source("https://example.com/")` returned `192.168.1.2`
  while those bytes left from `127.0.0.2`, and the same function pointed at the proxy URI itself returned
  `127.0.0.1`. The attribution call takes no proxy argument, so it cannot name the hop that carries the
  bytes; R1 means changing what it resolves against, which is the additive rework the design keeps room for.
  Two measured softeners: `proxy_bypass` hosts keep their bound route (no CONNECT at all — step 1b proved it
  in the shipped exe too), and R2 costs no honesty per host. It does **not** cost "one CONNECT per job": the
  step-1b legs opened 12-14 tunnels with `streams_final` at 8 (§4.3), so the pooled-tunnel reuse step 0 saw
  between two manual clones does not describe a real job. **Step 2 shipped the per-URL half and measured it
  in the exe:** the bypass leg carries the full 32 MiB on the Ethernet stripe with 0 tunnels while the proxy
  stays resolved and displayed, and the proxied legs carry it on `n0` with both adapters at +0 bytes (§6
  step 2). The card's sentence is therefore per-URL in fact, not in intention.
- **Q5 — Credentials at rest.** **Chosen: re-affirm the plaintext position on the record now** — one explicit
  sentence in the settings UI, inheriting `db.rs:278-288`'s threat model — **and put encryption behind its
  own measured slice.** A DPAPI (`CryptProtectData`) wrapper stays the candidate worth pricing because it
  needs no crate. Step 0 adds nothing here, but it did add a reason to be careful: the proxy password is
  pasted into a *setting* (`network.proxy_url` carries userinfo in ureq's own model, `Proxy::new("http://
  user:pass@host:port")`), and a setting is stored in the same plaintext table as everything else — the
  cookie threat model in `db.rs:278-288` was never written about passwords. Chosen corollary: **proxy
  credentials go in `credentials` under `scheme='proxy'`, not inside the proxy URL setting**, and that
  setting must refuse userinfo. The reason is not secrecy — both are plaintext at rest — it is that the
  proxy URL is what §4.5's status line and card name out loud (`via proxy.example:3128`), and a secret
  living inside it would be echoed to the screen and into every history line.
  **Step 3 shipped both halves of that corollary, and the second half of Q5 with them.** The pair lives in
  `credentials` under `scheme='proxy'` and reaches the engine through `proxy:set-auth`, never through
  `network.proxy_url` — which the boundary refuses userinfo on, measured by the `refused-url` leg since step
  1b. The **one explicit sentence in the settings UI** this ruling asked for now exists on the credential row
  ("What you type is saved in this app's library file exactly as written, not encrypted — the same file as your
  saved sessions"), and it was read back out of the live page over CDP rather than trusted from the source
  file. Its measured cost was **0 B**: the exe stayed at exactly 4,534,784 B across that relink, so those 148
  bytes of HTML landed in alignment padding the previous link was already carrying. Two facts this step
  surfaces rather than settles: the sentence says *not encrypted* because that is the position `db.rs:286`
  takes for cookies and Q5 re-affirmed for passwords — encryption stays behind its own measured DPAPI slice;
  and `resetAllSettings` cleared the proxy trio but **not** the credential, because forgetting a secret was not
  a default. That was a choice this agent made inside "do as you see fit", flagged as the kind of thing worth a
  ruling if the user read it differently — and the user did read it differently: on 2026-11-05 they ruled
  *"resetAllSettings should clear credentials, go ahead"*, which is `DECIDED` and landed as §6 step 8. The
  encryption position above is unaffected by that ruling.
- **Q6 — Where the proxy lives in the UI.** **Chosen: global in the Settings stage only, and the docs say
  so.** Per-adapter proxies multiply §4.3 — with R2 the whole premise of the card is "a manual proxy means
  the default route", and per-NIC proxies would reintroduce exactly the per-route attribution honesty R2
  defers. The card's job in this slice is to *name* the proxy and say bonding is off, not to configure one.

---

## 9. Evidence ledger

**Commands run for this document** (all against `D:\RE\Sucr\app` and the vendored ureq at
`C:\Users\Benjamin\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\ureq-3.5.0`): `grep -rin proxy`
over `src-tauri/src` and `src` (0 and 0); `grep` for `Authorization|401|407|WWW-Authenticate` in the
engine; `env | grep -i proxy` (empty); `grep -n 'CREATE TABLE' schema.sql` (16 tables); per-slot reference
counts (§2 finding 7); ureq `config.rs` / `proxy.rs` / `transport/mod.rs` / `transport/connect.rs` /
`transport/chain.rs` / `Cargo.toml` read at the cited lines; `Cargo.lock` `winreg` provenance. Path note:
ureq's transport modules live under `src/unversioned/transport/` in that crate — this document cites them
as `transport/mod.rs`, `transport/connect.rs`, `transport/chain.rs`, `transport/socks.rs` without the
`unversioned` segment, and every one of those line numbers was re-checked against the file at
`src/unversioned/transport/…` on 2026-11-04.

**Commands run for step 0 (2026-11-04, later the same day)** — every claim in §4.1, §4.2, §4.3, §4.5 and §5
marked *measured* rests on these, and the raw lines are in §4.7: `cargo build --release --offline` then
`node drive.mjs` in `D:\RE\_cap-harness\proxy-spike`, whose legs print one JSON line each while `proxy.mjs`
logs the peer of every control connection and `origin.mjs` logs who dialled it; `openssl req -x509`
for `certs/spike-*.pem`; the §5 byte row from `stat` on `target/release/sucr.exe` at
`D:\RE\Sucr\app\src-tauri` across three builds — baseline **4,508,672**, with the three `engine/bond.rs`
edits **4,509,184**, and after `cp bond.rs.baseline` back **4,508,672** with the sha256 matching
(`f263ced3…`). Before the measurement, `tasklist` proved no `sucr.exe` was running and `find` proved no
source file was newer than the exe. `winreg` linkage was re-derived by reading `Cargo.lock:671` and ureq's
`Cargo.toml:112,114`, not by building.

**Commands run for step 1 (2026-11-04, same day)** — the §5 panel rows and every "measured in the shipped
exe" claim in §6 step 1 rest on these. In `D:\RE\Sucr\app`: `npm run build`, `npm run typecheck`,
`cargo test --release --lib -- --test-threads=1`, `npm run verify`, and `stat -c %s
target/release/sucr.exe` after each link — the four sizes in §5 are **4,508,672** (pre-feature baseline),
**4,513,792** (1a), **4,517,376** (1b panel), **4,519,936** (1b `--smoke` check), and the 0 B cost of the
`ipc.rs` doc-comment reword was observed as an unchanged 4,517,376 across that link. `tasklist` proved no
`sucr.exe` was running before each relink and the running window was never restarted.
`D:\RE\_cap-harness\proxy-run.mjs` was invoked **five times**; the reports are
`proxy-report-2026-11-04T18-45-11-765Z.json` (aborted before any leg — `const` collision in the driver),
`…T18-45-48-357Z.json` (`verdict.allPass: false`, five legs failing on my own origin-byte denominator),
then `…T18-46-32-545Z.json`, `…T18-48-19-768Z.json` and `…T18-57-12-359Z.json`, all seven legs passing on
the shipped exe — the first two against `bytes.exe: 4517376`, the last against **4519936** after the smoke
check was added, so the proof was re-run on the final tree. Those files hold the raw evidence: `launched`
(pid, own `--data-dir`, CDP port 9347, and the `envHttpProxy` actually given to the process), `preflight`
(proxy and origin ports, `requireAuth: 0`, the 33,554,432-byte fixture), one `legs[]` entry per case with
`settingsBefore` / `settingsAfter` read from the table, the `via` the engine returned, the card note text
and tone, the screenshot name, and — for legs that ran a job — `job.proxy` (`marks`, `connects`, `dials`,
`peers`, `refused`, `up`, `down`) plus `job.streamsFinal` and `job.verifiedBytes`. The counts quoted in §6
come from those fields, and `verdict` records the caveats in the last row below.

**Commands run for step 2 (2026-11-04, same session)** — every "measured in the shipped exe" claim in §6
step 2, §4.3's fifth bullet and §8 Q4's closing sentence rest on these. In `D:\RE\Sucr\app`: `npm run build`,
`npm run typecheck`, `cargo test --release --lib -- --test-threads=1` (**169 pass, 6 ignored**), `npm run
verify` → `M0 GATE: PASS (exe 4.31 MB)`, and `stat -c %s target/release/sucr.exe` → **4,521,984 B**;
`tasklist` proved no `sucr.exe` was running before the relink and the user's window was never restarted. The
build output prints `history kinds: 31 written by the engine, all formatted by the UI`, which is the §4.5
gate observed rather than quoted.
`D:\RE\_cap-harness\proxy-run.mjs` was invoked **four more times** for this step, and the three before the
clean one are kept because each names a different mistake of mine rather than of the app:
`…T19-42-23-075Z.json` (aborted on the first leg — an invalid `.card[data-id=1]` selector thrown by
`querySelector`), `…T19-43-04-269Z.json` (seven legs, four failing: three on assertions I had written wrong
and one showing a **deck read one leg stale**, the page repainting after my poll returned), and
`…T19-45-33-529Z.json` (a `deck` of `null` on every leg, because the node-identity guard I used to fix that
race could never match: `renderRows()` replaces the card nodes). The clean run is
`proxy-report-2026-11-04T19-49-06-895Z.json`: `allPass: true`, `failed: []`, and the new
`attributionCovered: true` in `verdict`. Its per-leg fields are the numbers in §6 step 2 — `job.stripes`
(the durable map, summed per `networkId`), `job.netUsageDelta` (`network:list` read before and after the
job, per adapter), `job.proxyHistory` (the `history:tail` rows filtered to `kind === 'proxy'`), and
`job.deck` (`{note, rows}` read off the opened inspector, `.attrib-note` text plus each `.attrib-row`
`{net, label, sub}`). The harness grew three readers for this — `netUsage`, `usageDelta`, `proxyRows`, and
`ui.deck` — and `attributionChecks` with its three modes; all of it stays in `D:\RE\_cap-harness`, and the
only product-side test code this step added is the `netNoteHas()` polling inside the bundled smoke driver.

**Commands run for step 3 (2026-11-04, same session)** — every "measured in the shipped exe" claim in §6 step 3,
§4.4's proxy row, §4.5's typed-error paragraph and §8 Q5's closing paragraph rest on these. In `D:\RE\Sucr\app`:
`npm run build`, `npm run typecheck` (clean), `cargo test --release --lib -- --test-threads=1` → **184 pass,
6 ignored** (15 of them step 3's, countable by name: three `db::tests` credential rows, eight
`engine::proxy::tests` about `auth_host`, the authority key, the bypass and the secret never reaching a label,
three `engine::tests` about the typed error, one `ipc::tests` keying-and-not-echoing), `npm run verify` →
**M0 GATE: PASS (exe 4.32 MB)**, and `stat -c %s target/release/sucr.exe` → **4,534,784 B** both before and
after the disclosure sentence was added, which is how that sentence's 0 B was measured. The build still prints
`history kinds: 31` — step 3 added none. `tasklist` proved no `sucr.exe` was running before the relink and the
user's window was never restarted.
`D:\RE\_cap-harness\proxy-run.mjs` was invoked **seven times** for this slice, and the runs that failed are kept
because each names a mistake of mine rather than of the app. Five of them belong to step 3:
`…T20-38-12-820Z.json` (four credential legs red for one harness cause — step 2's `manual-with-bypass` leg
leaves `192.168.1.2` on the bypass list, so the target never reached the authenticating fixture at all; the fix
is one clear before the legs plus a `proxy_bypass === null` assertion inside `credentialRefusalChecks`),
`…T20-40-24-117Z.json` (the same legs red a second way: `credentialled`/`authUsers` were derived from the
fixture's `allow` lines, which a refusal never prints — so the wrong-password leg could not see the header it
had sent), `…T20-42-52-197Z.json` (the `head` witness existed but the tally still filtered on
"not allow, not reject", which both double-counted — 24 asks against 12 tunnels — and still missed refused
CONNECTs), then `…T20-44-47-140Z.json` (the wrong-password leg sampling a password box that still held 16
bytes, because `waitForAuthNote` cannot see a change to only the secret) and the clean
`…T20-46-01-296Z.json` (`allPass: true`, 12 legs, `proxyCredentialsCovered: true`). Two runs came after that:
`…T20-53-17-014Z.json` added the third fixture and the `proxy-refuses-port` leg — the first time
`proxy-failed` left the exe — and `…T21-03-25-687Z.json` re-ran all 13 legs against the rebuilt binary so the
cited proof matches the shipped bytes and the new disclosure sentence is read from the live page.
`verdict` carries `attributionCovered`, `proxyCredentialsCovered` and now `proxyErrorCodeSplitCovered` as
`true`, with `bondedChainCovered`, `multiAdapterCollapseCovered`, `digestCovered`, `socksCovered` and
`systemRegistryCovered` still `false`. Per-leg fields this step reads: `job.proxy` gained `asks`,
`credentialled`, `authUsers`, `passMatch` and `refused` — `passMatch` is only ever *"does this equal the pair
this fixture expects"*, never the secret itself — and `preflight.refusingProxy` records the third fixture's
port and `refusePort`.

**Commands run for step 4 (2026-11-04, same session)** — every "measured in the shipped exe" claim in §6
step 4, §4.4's origin row and §4.5's two-reading paragraph rest on these. In `D:\RE\Sucr\app`: `npm run build`,
`npm run typecheck` (clean), `cargo test --release --lib -- --test-threads=1` → **192 pass, 6 ignored** (step 3
left 184; `engine/auth.rs` carries six of the eight new ones — the RFC base64 vectors, the `user:password`
encoding, the per-host store answering nothing else, a request that already carries `Authorization` being left
alone, the pair being on the request *as it leaves*, and a SameHost redirect keeping it), `npm run verify` →
**M0 GATE: PASS (exe 4.35 MB)** with the new check *a site credential stores by click, keys by host, never
echoes, and forgets one host at a time* reporting all eleven fields true (`line, keyed, listed, bytesKept,
noEcho, boxesEmpty, refused, refusedLeftNothing, dropped, isolation, cleared`), and `stat -c %s
target/release/sucr.exe` → **4,534,784 B before, 4,562,432 B after (+27,648 B)**. The baseline was established
without reverting: the pre-step-4 exe string-probes for zero hits of `origin:set-auth`, `Site credentials` and
`origin-save`, and `find -newer` showed no other source had moved. `tasklist` proved no `sucr.exe` was running
before the relink and the user's window was never restarted. The gate's three literals moved with the new
channel, as gate mechanics say they must: `CHANNELS.len()` 34 → 35 in Rust, and in `verify-m0.mjs` the channel
count, the domains list (`origin`) and the per-domain methods list (`origin: ['setAuth']`).
`D:\RE\_cap-harness\auth-run.mjs` was invoked **twice**: `…T21-55-27-428Z.json` failed one leg and only one,
on the harness's own blind spot (the missing-save-to-folder gate, above), and
`…T21-57-58-346Z.json` is the clean run — `verdict.allLegsPass: true`, eight legs, `bytes.exe: 4562432`, run
against the final binary. Per-leg fields the step reads: `line` (`text`, `title` as the engine wrote it,
`submitDisabled`, `folder`, `folderNoticeHidden`), `rows` / `emptyRowText` off the live card, `list.keys` and
`list.echo` from `app:get-info`, `job` (`status`, `verifiedBytes`, `errorCode`, `errorHint`, `streamsFinal`,
`filesOnDisk`), `card.buttons` / `card.lines` from the opened inspector, and `fixture` — `requests`, `ranged`,
`served`, `challenges`, `authed`, `verdicts` (per-request `absent` / `mismatch` / `ok` / `digest-only`) and
`refused`, which is deliberately a list of `{auth, status, bytes}` and never the header, because the exact
string would be the secret. The fixture grew `REQUIRE_BASIC=user:pw`, `REQUIRE_DIGEST=1`, and the
`challenges` / `authed` counters with `/__reset`; `challenges === 0` across a 13-request, 8-stream, 32 MiB
transfer is the whole pre-send claim, measured on the host side. Its `verdict` carries the six step-4 flags
named in §6 (`originBasicPreSent`, `originBasicOnBothErrorSites`, `refusedPairDistinguished`,
`hostKeyedIsolation`, `survivesRestart`, `digestOfferNamed`) plus the three left `false` for the same reason
every earlier flag was withheld — the outcome has not been observed leaving the exe: `bearerCovered`,
`digestCovered`, `httpsOriginCovered`.

**Commands run for step 5 (2026-11-04, same session)** — every "measured in the shipped exe" claim in §6 step 5
and §4.4's Bearer row rest on these. In `D:\RE\Sucr\app`: `npm run typecheck` (clean), `cargo test --release
--lib -- --test-threads=1` → **196 pass, 6 ignored** (step 4 left 192), `npm run verify` → **M0 GATE: PASS
(exe 4.36 MB)** with the site-credential check now ANDing seven token fields into its conjunction
(`tokenUserBoxHidden, tokenKindListed, tokenReplacedPair, tokenNamedInRow, tokenNoEcho, tokenDropped`, beside
step 4's eleven), and `stat -c %s target/release/sucr.exe` → **4,562,432 B before, 4,567,552 B after
(+5,120 B)**, leaving **675,328 B** under the ruled 5 MB ceiling. The build still prints `history kinds: 31` —
step 5 added none — and `CHANNELS.len()` stayed **35**, which is the point of spending the write on an optional
fourth argument instead of a channel: a 36th would have moved four snapshot sites plus two gate literals. The
`check(` count in `verify-m0.mjs` is unchanged at **50** for the same reason. `tasklist` proved no `sucr.exe`
was running before either relink and the user's window was never restarted.
`D:\RE\_cap-harness\auth-run.mjs` was invoked **three times**. The first aborted on leg 2 with
`ReferenceError: kind is not defined` inside the in-page function (`…T22-48-17Z.json`): `ui.credential` returns
`kind` as an object **shorthand**, and a shorthand resolves in the *page's* scope, where the driver's Node-side
`kind` never existed — the fix is `kind: ${JSON.stringify(kind)}`, and it is a trap for every future helper that
passes a driver-side value into a template literal. `…T22-49-08Z.json` then ran all 14 legs green (86 checks)
against the step-5 binary, and **that run is what caught the false sentence**: the bearer host's `title` read
*this app sends only Basic* while leg 10 of the same run downloaded 32 MiB with a `Bearer` header. The reword
in `engine/mod.rs:308-322` was relinked and measured at **4,567,552 B — unchanged**, so a longer `format!`
literal in an existing match arm costs 0 B; proven in the binary by string probe (1 hit for
`this app sends Basic or a bearer token`, 0 for `this app sends only Basic`). `…T22-56-54Z.json` re-ran every
leg against that rebuilt binary with one assertion added: `verdict.allLegsPass: true`, **14 legs, 87 checks,
0 failed**, `bearerCovered: true`, and the two engine sentences read from the live page —
`server needs a password (offers bearer)` on the bearer host,
`server needs a password, and offers digest; this app sends Basic or a bearer token` on the Digest-only one.
The fixture grew `REQUIRE_BEARER=token`, a third `server.mjs` instance answering 401 with
`www-authenticate: Bearer realm="sucr-harness"` and the same per-request verdicts (`absent` / `mismatch` / `ok`
/ `digest-only`), with `/__stats` reporting only the *shape* of the requirement. No secret is printed anywhere:
the driver's `pass` echo guard refuses a witness that contains the pair or the token, and both `app:get-info`
and every row were searched for all four test secrets.

**Commands run for step 6 (2026-11-04, same session)** — the reachability half, and it is a source check, not a
build: `grep -ril digest` over the whole vendored `ureq-3.5.0` tree → **0 files, 0 matches**; `grep -rn
"Proxy-Authorization" src/` → exactly one site, `connect.rs:124`; the CONNECT answer loop read at
`connect.rs:90-159` (single `response.status()` match, `StatusCode::OK` opens the tunnel, every other status is
`Error::ConnectProxyFailed` at `:147-155` — no challenge round exists to answer a 407 with). The deps half:
`grep -n "md-5\|sha2" Cargo.toml` → `md-5 = "0.10"` at `:23`, `sha2 = "0.10"` at `:28`; `schema.sql:38` CHECK list
read directly → `('basic', 'digest', 'ntlm', 'proxy', 'cookie', 'header-secret')`, which is what makes the
deferral cheap to reverse. The usage half has no command behind it, and that is the finding: this app collects no
usage telemetry, so nothing in the built product could have answered "do servers need Digest", and no real
Digest-only host was ever contacted. **Step 6 wrote no product code, so it moved no bytes — the exe is unchanged
by this step, and the 4 – 8 KB in §5 stays an estimate.**

**Commands run for step 7 (2026-11-04, same session)** — the wording is a string edit, so the ledger here is
mostly measurement. `npm run typecheck` → clean, the two banner lines only.
`npm run build` (with `CARGO_HOME`/`RUSTUP_HOME` unset) → `Finished release profile in 1m 04s`,
`build-app: … sucr.exe · 4461 KB (4.36 MB)`; `stat -c %s src-tauri/target/release/sucr.exe` →
**4567552** both immediately before and immediately after that link, which is the 0 B claim. The claim then had
to be separated from "the edit did not compile into anything", so the linked binary was searched, not the
source — every probe below is `rg -a` over `sucr.exe` itself, with NULs stripped for display:
**present** `server needs a credential (offers`, `server needs a credential, and offers`,
`this app sends Basic or a bearer token`, `the stored credential was refused`,
`The credential stored for this host was refused (401).`, `A credential is saved for ${l.host}`,
`Add a credential for this host`, `Replace the token for this host`, `Replace the password for this host`,
`or the link that needs a credential`, `a password needs a username`;
**absent (0 matches)** `server needs a password`, `this app sends only Basic`, `the stored password was refused`,
`This link needs a password`, `needs a password`, `Add a password for this host`, `stored password`.
Two of those hits are the point of the exercise rather than an oversight: `Replace the password for this host`
and `a password needs a username` are the two places the kind *is* known locally, so the rule says they should
still read "password" — a probe that returned zero for them would have proved a blunder, not a fix.
`cargo test --release --lib -- --test-threads=1` → **196 passed; 0 failed; 6 ignored** (50.71 s).
`npm run verify` → **M0 GATE: PASS (exe 4.36 MB)**; `grep -c 'check(' scripts/verify-m0.mjs` → **50**, unchanged,
and the pre-built "a site credential stores by click…" leg passed without its needles being touched, which is
how the §6 claim "no gate check asserts the nine sentences" was checked rather than remembered.
Then the two launched-exe suites against the rebuilt binary: `node auth-run.mjs` → three fixtures on 8098/8099/
8100, three app processes on their own `--data-dir`, **14 legs, `allLegsPass: true`**, 88 `checks` total across
the legs (5+9+7+7+6+4+4+4+4+13+6+8+6+5), `digestCovered: false`, `httpsOriginCovered: false` —
`auth-report-2026-11-04T23-29-35-575Z.json`; `node proxy-run.mjs` → **13 legs, `allPass: true`**,
`attributionCovered` / `proxyCredentialsCovered` / `proxyErrorCodeSplitCovered` true and `digestCovered` /
`socksCovered` / `systemRegistryCovered` false — `proxy-report-2026-11-04T23-31-33-568Z.json`.
Secrets searched in the auth report: `pw1` **0**, `eyJ` (the token's prefix) **0**, `sucr:pw1` **0**; the 26 hits
for `sucr` are the username in row meta, `sucr.exe`/`sucr.db` paths and a harness `.sucrpart` filename.
Both drivers stopped their own app processes and deleted their staging; `tasklist` shows no `sucr.exe`, and the
user's own window was never launched or touched by this step.
*Ledger note:* the step-5 paragraph above records the live-page sentences as that run read them
(`server needs a password (offers bearer)`); the same two legs in this run read
`This link needs a credential (401).` with `server needs a credential (offers bearer)` beneath it, and the
Digest-only leg reads the disclaimer in its new words. That is the wording change being measured, not a second
behaviour.

**Commands run for step 8 (2026-11-05, the user's ruling)** — the change is one renderer function
(`resetAllSettings`, `src/renderer/main.ts:2563` — `:2473` as written on 2026-11-05; the global download
window inserted `windowRuleText()` and the Settings panel above it, and *line 2473 is now a different function*)
and one new harness leg, so the ledger is a byte measurement plus
three launched runs. `npm run typecheck` → clean. `npm run build` (with `CARGO_HOME`/`RUSTUP_HOME` unset) →
relink in 1 m 03 s; `stat -c %s src-tauri/target/release/sucr.exe` → **4567552** immediately before and again
immediately after, the 0 B claim. Because a size match cannot tell "free" from "not linked", the linked binary
itself was searched: `grep -a -c 'All preferences and stored credentials reset to defaults.' sucr.exe` → **1**,
`grep -a -c 'All preferences reset to defaults\.' sucr.exe` → **0**. And because the tree is shared, the build was
dated rather than assumed: `stat -c '%y %n' src/renderer/main.ts src-tauri/target/release/sucr.exe` → the renderer
at **05:22:04**, the exe at **05:23:16**, so the embedded bundle under test is the edited one.
`cargo test --release --lib -- --test-threads=1` → **196 passed; 0 failed; 6 ignored** (50.92 s) — unchanged,
which is the point of choosing the existing forget paths: no Rust was added, so no test was owed. `npm run verify`
→ **M0 GATE: PASS (exe 4.36 MB)**; `grep -c 'check(' scripts/verify-m0.mjs` → **50** and `CHANNELS` → **35**, both
unmoved, and the in-app `--smoke` driver was read to confirm no stale assertion pointed at the old sentence before
it was replaced.
Then the launched-exe proof, three times, and the first two are the record's value: `node auth-run.mjs` →
**15 legs**, leg 15 `reset-all clears credentials` failing **5** checks
(`auth-report-2026-11-04T23-58-34-970Z.json`) — the fixture, not the product: the proxy had been staged through
the bridge, so the page's `storedSettings` never held `network.proxy_mode` and the reset's own loop could not
clear a shape a user cannot make. `ui.proxyStage` presses the segmented control and fires a real `change` off the
URL box, the way `proxy-run.mjs` has staged a proxy since step 1b. Second run → leg 15 failing **1** check
(`auth-report-2026-11-05T00-04-18-927Z.json`), and again the assertion was wrong rather than the app: it read
`passwordBytes` off `app:get-info`, a field the proxy summary does not have (`ipc.rs:1130-1145` is
`stored`/`username`/`host`/`applies`; the byte count lives only on the `proxy.set-auth` reply). Re-pointed to
`stored` + `username` + `host === 127.0.0.1:8199` + `applies === true`, and `node --check auth-run.mjs` → clean
(one trap recorded in the driver: a JS comment *inside* a `cdp.evaluate` template must carry no backticks — they
close the outer literal). Third run → **15 legs, `allLegsPass: true`**, **106 checks** total
(5+9+7+7+6+4+4+4+4+13+6+8+6+5+**18**), `resetAllClearsCredentials: true`, `digestCovered: false`,
`httpsOriginCovered: false` — `auth-report-2026-11-05T00-08-40-062Z.json`. The sibling suite was re-run as the
regression sweep it is, because the reset button now clears the panel its own legs stage: `node proxy-run.mjs` →
**13 legs, `allPass: true`** (`proxy-report-2026-11-05T00-10-04-967Z.json`), its needles untouched.
Secrets searched in the auth report: `pw1` **0**, `eyJ` **0**. Both drivers stopped their own app instances and
deleted their staging; `tasklist /FI "IMAGENAME eq sucr.exe"` → **no tasks running**, and the user's own window was
never launched or touched by this step.

**Still read from library source, therefore not proven in this app** (step 0 narrowed six items to five;
step 1 closed none of them, and step 3 closed exactly one — the *wrong* proxy password, now measured against
an authenticating fixture rather than inferred from an absent header. What remains: that SOCKS5 works through
the same chain shape — `SocksConnector` was never enabled or exercised; that the system-proxy registry read
behaves as C2 describes — `Mode::System` is still `Proxy::try_from_env()`, and no registry code was written;
that origin Digest is answered rather than only named — step 4 proved the *naming* of the offer and the
refusal to send anything but Basic, not a working `Digest` computation. Step 4 closed the fifth item, the one
this list carried longest: that origin 401 Basic works through `config_ua()` + a middleware — §4.4's H1 shape
is now compiled *and* measured, pre-send, in the shipped exe. Every
§5 byte estimate except the chain row and the five measured step totals is still an estimate.)
`verdict` states the same list as flags: `bondedChainCovered`, `digestCovered`,
`socksCovered`, `systemRegistryCovered` are all `false` in every run — with one enabled device the app
builds no bonded chain at all, so the proxied legs exercise `config.proxy` on the default route only, and
the `ConnectProxyConnector` head of the bonded chain is proven by the step 0 spike and the unit tests, not
by the launched app. Step 2 changed two flags in that same `verdict`: `attributionCovered: true` (the
stripes / `network:list` delta / durable row / deck quadruple is asserted on all four job legs), and a new
`multiAdapterCollapseCovered: false`, because the collapse R2 chose is only *really* a collapse when there
are ≥2 routes to collapse — with one enabled adapter the pool is already a single default route, so the
launched legs prove the *no-credit* half of R2 and the unit tests still own the multi-route half. Turning
that flag true means enabling the user's disabled Tailscale adapter, which is their machine's state to
change, not a proof to manufacture; the same run's `netUsageDelta` already shows that adapter as a listed
row with `grew: 0`, so the shape of the claim is visible even where it is not exercised. Step 3 raised two
more in the same `verdict`: `proxyCredentialsCovered: true` (five legs over one app process — absent, typed,
wrong, stranded, forgotten — with the pair committed through the page and the secret witnessed absent from
both the answer line and the whole `app:get-info` payload), and a new `proxyErrorCodeSplitCovered: true`,
which exists only because §4.5 forbade promising that split unread: `proxy-auth` and `proxy-failed` have both
now been observed leaving the exe, not only out of `engine::tests`.

**Assertions this document does not make:** no throughput estimate for proxied transfers, no claim about
which corporate proxies demand Digest, and no claim that any real host reachable from this machine needs
auth — none of that was measured here. The feature's *need* comes from the gap analysis (proxy and HTTP
auth absent in both Plexo and Sucr); its *behaviour* now comes from §4.7, which is a **loopback harness**,
not a corporate proxy: everything above was measured against a proxy written for this spike, on one
machine, with `127.0.0.2` standing in for a second NIC. A real proxy's TLS trust chain, its keep-alive
timing, its handling of `Proxy-Connection: Keep-Alive`, and its behaviour under 13 simultaneous tunnels
are all outside what step 0 could show.
