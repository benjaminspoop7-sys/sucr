// Where the engine's sockets leave from when the network needs a proxy.
//
// A mirror of three setting keys, not a store: the same shape as
// `CONNECT_TIMEOUT_SECS`, so no agent-building call site needs a database
// handle to answer "am I proxied?". One place turns the strings into a `Proxy`
// and every path reads that one answer, which is what keeps the bonded agent
// from drifting from the default one the way timeouts and TLS trust already do.

use std::sync::Mutex;

use ureq::http::Uri;
use ureq::{Proxy, ProxyProtocol};

/// `network.proxy_mode`. `Direct` is a decision, not the absence of one: ureq
/// seeds `Config::default()` from `HTTP_PROXY`, so leaving the proxy option
/// unset means "whatever the environment says". Writing `None` is the only way
/// to mean direct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Direct,
    System,
    Manual,
}

/// The values `network.proxy_mode` accepts, in the order Settings lists them.
pub const MODES: [&str; 3] = ["direct", "system", "manual"];

impl Mode {
    /// Anything unreadable is direct: a hand-edited database row must not
    /// become an invisible proxy, and must not stop the app from booting.
    fn parse(value: &str) -> Self {
        match value {
            "system" => Self::System,
            "manual" => Self::Manual,
            _ => Self::Direct,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::System => "system",
            Self::Manual => "manual",
        }
    }
}

/// The Basic pair the user typed for one proxy.
///
/// Keyed by the proxy's `host:port` rather than held blindly: `network.proxy_url`
/// is a free-text box, and a password that travelled to whatever was typed next
/// would be a leak this screen caused. A credential whose host no longer matches
/// the proxy in front of it is inert until it is typed again.
struct Auth {
    host: String,
    username: String,
    secret: String,
}

struct Setting {
    mode: Mode,
    url: String,
    bypass: String,
    auth: Option<Auth>,
}

static SETTING: Mutex<Setting> = Mutex::new(Setting {
    mode: Mode::Direct,
    url: String::new(),
    bypass: String::new(),
    auth: None,
});

fn lock() -> std::sync::MutexGuard<'static, Setting> {
    SETTING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Write the three keys. The boot read and `settings_set` are the only callers,
/// and both have the values as the user left them, so this stores the strings
/// and lets `resolve` read them back — the reason a bad URL costs a download
/// nothing rather than a panic.
///
/// It does not touch the credential, and that is not an oversight: the three keys
/// live in `settings` and the pair lives in `credentials`, one command writes each,
/// so changing the proxy URL cannot clear the password and clearing the password
/// cannot change the URL.
pub fn configure(mode: &str, url: &str, bypass: &str) {
    let mut st = lock();
    st.mode = Mode::parse(mode);
    st.url = url.trim().to_string();
    st.bypass = bypass.trim().to_string();
}

/// Write the stored proxy credential. An empty username means there is none — that
/// is how forgetting works, and the reason a lone password is never stored: ureq's
/// `ProxyBuilder::build` only writes userinfo inside `if let Some(username)`, so a
/// password with no name before it is dropped on the floor rather than sent.
pub fn set_auth(host: &str, username: &str, secret: &str) {
    let mut st = lock();
    st.auth = if username.is_empty() {
        None
    } else {
        Some(Auth {
            host: host.to_string(),
            username: username.to_string(),
            secret: secret.to_string(),
        })
    };
}

/// The `host:port` a credential has to be keyed to to be used now, or `None` when
/// there is no proxy this app builds. The write path needs it — it is what the row
/// gets stored against — and reading it back through the same `host()`/`port()`
/// pair that [`manual`] compares is what keeps the two agreeing.
///
/// Manual mode only, deliberately. A system or environment proxy can carry its own
/// userinfo and ureq sends it, but adding a typed pair to it means rebuilding the
/// `Proxy`, and the only public way to do that (`Proxy::new`) drops the
/// environment's `NO_PROXY` list — so the internal hosts the environment exempts
/// would start leaving through the proxy. A missing password is a louder failure
/// than a bypass that silently stopped working.
pub fn auth_host() -> Option<String> {
    let st = lock();
    if st.mode != Mode::Manual {
        return None;
    }
    resolved(&st).map(|proxy| authority(&proxy))
}

/// The authority half of [`label`], in the form a credential row is keyed to.
fn authority(proxy: &Proxy) -> String {
    format!("{}:{}", proxy.host(), proxy.port())
}

/// The proxy the next agent is built with. `None` means the bytes leave on the
/// route the operating system picks, and `config_ua` writes that as an explicit
/// `.proxy(None)` rather than an omitted option.
pub fn resolve() -> Option<Proxy> {
    let st = lock();
    resolved(&st)
}

/// `resolved`, without reaching for the lock a second time.
fn resolved(st: &Setting) -> Option<Proxy> {
    match st.mode {
        Mode::Direct => None,
        // The environment's own `NO_PROXY` travels inside the `Proxy` that
        // `try_from_env` returns, so `network.proxy_bypass` is not consulted
        // here: it belongs to the URL the user typed, not to the system's.
        Mode::System => Proxy::try_from_env(),
        Mode::Manual => manual(&st),
    }
}

/// What the UI names as the egress path, so a transfer can say the route it
/// actually used instead of the setting that was saved.
///
/// Only the authority is printed, never `Proxy::uri()`: that carries userinfo,
/// and an environment proxy with a password in it would land in a status line
/// and in whatever the renderer logs. A mode that resolved to nothing says
/// `direct`, because that is where the bytes go.
pub fn via() -> String {
    let st = lock();
    match resolved(&st) {
        Some(proxy) => label(&st, &proxy),
        None if st.mode == Mode::Direct => "direct".to_string(),
        // Where the bytes go, with the mode that failed to say so: a manual mode
        // with nothing readable in it is a misconfiguration, and a label that
        // only said "direct" would hide it.
        None => format!("{} direct", st.mode.as_str()),
    }
}

/// `mode host:port`, the words `via()` prints and `egress()` repeats.
fn label(st: &Setting, proxy: &Proxy) -> String {
    format!("{} {}:{}", st.mode.as_str(), proxy.host(), proxy.port())
}

/// Which path one URL's bytes take under this mirror.
///
/// The question is per URL, not per setting. ureq's CONNECT connector asks
/// `is_no_proxy` on the request URI before it opens any proxy socket
/// (`unversioned/transport/connect.rs:49-57`), so a host on the bypass list and
/// a host behind the proxy genuinely leave differently from the same machine.
/// This calls the same method on the same parsed URI, so the answer is the
/// transport's own and not a second guess beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Egress {
    /// Nothing resolved for this mode, so the bond — or the OS route under it —
    /// carries the URL as it always did.
    Unproxied,
    /// The proxy carries it. The string is `via()`'s own words.
    Proxied(String),
    /// A proxy resolved and this host is exempt from it, so the URL keeps the
    /// route it was given.
    Bypassed(String),
}

impl Egress {
    /// Whether the proxy, rather than a bonded adapter, is the thing carrying
    /// this URL's bytes.
    pub fn is_proxied(&self) -> bool {
        matches!(self, Self::Proxied(_))
    }

    /// The label to put on screen, or none when there is nothing to say.
    pub fn via(&self) -> Option<&str> {
        match self {
            Self::Unproxied => None,
            Self::Proxied(via) | Self::Bypassed(via) => Some(via),
        }
    }
}

/// `Egress` for a job URL.
pub fn egress(url: &str) -> Egress {
    let st = lock();
    let Some(proxy) = resolved(&st) else {
        return Egress::Unproxied;
    };
    let via = label(&st, &proxy);
    drop(st);
    // A URL this cannot parse is one `Agent::get` cannot request either, so the
    // transport never reaches the bypass check; leaving it unproxied keeps the
    // bond the caller built. Every write path stores an absolute http(s) URL
    // (`ipc.rs:523`, `engine/job.rs:3546`), which is the form that parses.
    let Ok(uri) = url.parse::<Uri>() else {
        return Egress::Unproxied;
    };
    if proxy.is_no_proxy(&uri) {
        Egress::Bypassed(via)
    } else {
        Egress::Proxied(via)
    }
}

/// A typed `http://host:port` or `https://host:port`, its bypass list, and the
/// credential when one is stored for exactly this proxy.
fn manual(st: &Setting) -> Option<Proxy> {
    let (protocol, host, port) = split(&st.url)?;
    let plain = build(protocol, &host, port, &st.bypass, None)?;
    let Some(auth) = st.auth.as_ref() else {
        return Some(plain);
    };
    if auth.host != authority(&plain) {
        return Some(plain);
    }
    // Rebuilt rather than patched: a `Proxy` has no setters, and the one public
    // constructor that takes the pair in a string, `Proxy::new`, puts the password
    // inside a `Uri` — the thing `Debug` prints and any log of the proxy can
    // format. The builder percent-encodes the pair into that URI and hands back
    // the decoded `username()`/`password()`, which is the form the CONNECT writer
    // base64s into `Proxy-Authorization`.
    //
    // A build that fails with the credential keeps the one without it: the proxy
    // answers 407 and the job says so, which is a visible refusal. Falling back to
    // no proxy at all would put the request on the open route instead.
    Some(build(protocol, &host, port, &st.bypass, Some(auth)).unwrap_or(plain))
}

/// One `Proxy` from the parts [`split`] read; `None` credentials builds the
/// anonymous proxy the settings screen alone can describe.
fn build(
    protocol: ProxyProtocol,
    host: &str,
    port: Option<u16>,
    bypass: &str,
    auth: Option<&Auth>,
) -> Option<Proxy> {
    let mut builder = Proxy::builder(protocol).host(host);
    // Skipping `.port()` leaves the protocol's own default (80, 443), which is
    // what a URL without a port says.
    if let Some(port) = port {
        builder = builder.port(port);
    }
    if let Some(auth) = auth {
        builder = builder.username(&auth.username).password(&auth.secret);
    }
    // One call per entry: `no_proxy` takes a single expression, and unlike the
    // environment's list this one is typed by hand, so the spaces are trimmed
    // rather than becoming part of a hostname that can never match.
    for entry in bypass.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        builder = builder.no_proxy(entry);
    }
    builder.build().ok()
}

/// Scheme and authority of a proxy URL; a bare `host:port` is http.
///
/// Userinfo is refused rather than ignored: credentials belong in their own
/// table, not in a URL that ends up on screen, and a proxy row that quietly
/// carries a password would look like the setting it replaced.
fn split(url: &str) -> Option<(ProxyProtocol, String, Option<u16>)> {
    let (scheme, rest) = match url.split_once("://") {
        Some(split) => split,
        None => ("http", url),
    };
    let protocol = match scheme {
        "http" => ProxyProtocol::Http,
        "https" => ProxyProtocol::Https,
        _ => return None,
    };
    // Anything past the authority is a path pasted from a PAC file.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        // An IPv6 literal keeps its brackets: `build` puts this string straight
        // into a URL, and `http://[::1]:3128` is what parses.
        Some((host, port)) if !host.is_empty() && port.parse::<u16>().is_ok() => {
            (host.to_string(), port.parse::<u16>().ok())
        }
        _ => (authority.to_string(), None),
    };
    Some((protocol, host, port))
}

/// Whether `configure` would read this as a proxy URL. The settings layer
/// refuses a value the engine cannot use, so a saved `network.proxy_url` is
/// always one that works: a refused save is loud, and a manual mode that
/// resolves to nothing would just quietly download over the default route.
pub fn accepts(url: &str) -> bool {
    split(url.trim()).is_some()
}

/// Whether every entry of a typed bypass list is a form that can match a host.
///
/// ureq never rejects an expression: its `NoProxyEntry` falls through to "exact
/// host" for anything it does not recognise, so a pasted CIDR range — the shape
/// a corporate bypass box invites — becomes a host name nothing can ever be, and
/// the internal traffic goes through the proxy while the settings row says it is
/// bypassed. Stated once, in the engine, next to the code that uses the list; if
/// ureq ever widens its grammar this refuses something that would have worked,
/// which is the loud direction.
pub fn accepts_bypass(list: &str) -> bool {
    list.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .all(shaped_like_a_host)
}

/// A host, an IP, or one `*` at either end — the only forms read as anything
/// other than a literal host name.
fn shaped_like_a_host(entry: &str) -> bool {
    // An IPv6 literal is the one shape that has to keep its colons, and its
    // brackets: `Uri::host()` reports a bracketed address back with them, which
    // is what a bypass entry is compared against.
    let bracketed = entry.starts_with('[') && entry.ends_with(']');
    if entry
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '/' | '@') || (c == ':' && !bracketed))
    {
        return false;
    }
    match entry.matches('*').count() {
        0 => true,
        // Anywhere but the ends, the star belongs to a prefix or suffix that can
        // only ever match a host containing a literal star.
        1 => entry.starts_with('*') || entry.ends_with('*'),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ureq::http::Uri;

    /// The mirror is process-wide and the engine tests share one binary: a test
    /// that left it on `manual` would make the bonding tests try to resolve a
    /// proxy that does not exist, and fail there instead of here.
    fn reset() {
        set_auth("", "", "");
        configure("direct", "", "");
    }

    #[test]
    fn direct_resolves_to_nothing() {
        configure("direct", "http://proxy.corp:3128", "");
        assert!(resolve().is_none());
        assert_eq!(via(), "direct");
        reset();
    }

    #[test]
    fn an_unreadable_mode_is_direct() {
        configure("socks", "http://proxy.corp:3128", "");
        assert!(resolve().is_none());
        reset();
    }

    #[test]
    fn manual_reads_host_port_and_default() {
        configure("manual", "http://proxy.corp:3128", "");
        let proxy = resolve().unwrap();
        assert_eq!(proxy.host(), "proxy.corp");
        assert_eq!(proxy.port(), 3128);

        configure("manual", "https://secure.corp", "");
        let proxy = resolve().unwrap();
        assert_eq!(proxy.host(), "secure.corp");
        assert_eq!(proxy.port(), 443);
        reset();
    }

    #[test]
    fn manual_keeps_an_ipv6_literal_bracketed() {
        configure("manual", "http://[::1]:8888", "");
        let proxy = resolve().unwrap();
        // Measured, not assumed: the brackets survive into the parsed URI, and
        // `host()` reports them back — which is what the status line prints.
        assert_eq!(proxy.host(), "[::1]");
        assert_eq!(proxy.port(), 8888);
        assert_eq!(proxy.uri().to_string(), "http://[::1]:8888/");
        reset();
    }

    #[test]
    fn userinfo_and_other_schemes_are_refused() {
        configure("manual", "http://user:pass@proxy.corp:3128", "");
        assert!(resolve().is_none());

        configure("manual", "socks5://proxy.corp:1080", "");
        assert!(resolve().is_none());

        configure("manual", "", "");
        assert!(resolve().is_none());
        // The label says where the bytes go and keeps the mode that failed to
        // provide a proxy, so an unusable row cannot look like a working one.
        assert_eq!(via(), "manual direct");
        reset();
    }

    #[test]
    fn the_url_validator_only_admits_what_the_engine_can_read() {
        for good in [
            "proxy.corp:3128",
            "http://proxy.corp",
            "https://proxy.corp:8443",
            "http://[::1]:8888",
        ] {
            assert!(accepts(good), "{good}");
        }
        for refused in [
            "",
            "http://user:pass@proxy.corp:3128",
            "socks5://proxy.corp:1080",
            "http://",
            "ftp://proxy.corp",
        ] {
            assert!(!accepts(refused), "{refused}");
        }
    }

    /// The shapes a bypass list may hold, measured against what ureq matches.
    #[test]
    fn a_bypass_entry_is_a_host_or_one_star_at_an_end() {
        for good in [
            "internal.corp",
            ".lab.example",
            "*.lab.example",
            "10.0.0.7",
            "[::1]",
            "internal.corp, .lab.example",
            "",
        ] {
            assert!(accepts_bypass(good), "{good}");
        }
        for refused in [
            // CIDR is the shape a corporate bypass box invites and the one that
            // silently matches nothing.
            "10.0.0.0/8",
            "in ternal.corp",
            "http://internal.corp",
            "*internal*",
            "*.a,*.b*",
        ] {
            assert!(!accepts_bypass(refused), "{refused}");
        }
    }

    /// Measured, not assumed: a bracketed IPv6 literal is what a bypass entry
    /// has to be, because that is the form the request URI reports its host in.
    #[test]
    fn a_bypass_entry_matches_an_ipv6_host_bracketed() {
        configure("manual", "http://proxy.corp:3128", "[::1]");
        let proxy = resolve().unwrap();
        assert!(proxy.is_no_proxy(&Uri::from_static("http://[::1]:8080/file")));
        assert!(!proxy.is_no_proxy(&Uri::from_static("http://example.com/x")));
        reset();
    }

    #[test]
    fn bypass_entries_hold_the_direct_route() {
        configure("manual", "http://proxy.corp:3128", "internal.corp, .lab.example");
        let proxy = resolve().unwrap();
        assert!(proxy.is_no_proxy(&Uri::from_static("http://internal.corp/file")));
        assert!(proxy.is_no_proxy(&Uri::from_static("http://a.lab.example/x")));
        assert!(!proxy.is_no_proxy(&Uri::from_static("http://example.com/x")));
        reset();
    }

    #[test]
    fn the_via_label_names_the_chosen_proxy() {
        configure("manual", "http://proxy.corp:3128", "");
        assert_eq!(via(), "manual proxy.corp:3128");
        reset();
    }

    /// The whole point of asking per URL: two links on one machine, one setting,
    /// and different answers — because that is what the transport then does.
    #[test]
    fn egress_answers_per_url() {
        configure("manual", "http://proxy.corp:3128", "internal.corp, .lab.example");
        let via = "manual proxy.corp:3128".to_string();
        assert_eq!(egress("https://example.com/a.bin"), Egress::Proxied(via.clone()));
        assert_eq!(egress("https://internal.corp/a.bin"), Egress::Bypassed(via.clone()));
        assert_eq!(egress("http://a.lab.example/x"), Egress::Bypassed(via));
        assert!(egress("https://example.com/a.bin").is_proxied());
        assert!(!egress("https://internal.corp/a.bin").is_proxied());
        // Explicit direct beats the environment for the same URL the line above
        // called proxied, and says nothing rather than naming a proxy.
        configure("direct", "", "");
        let quiet = egress("https://example.com/a.bin");
        assert_eq!(quiet, Egress::Unproxied);
        assert!(!quiet.is_proxied());
        assert_eq!(quiet.via(), None);
    }

    /// A job URL the parser refuses is a URL `Agent::get` also refuses, so there
    /// is no transfer to mislabel: the bond the caller built stays in place.
    #[test]
    fn egress_leaves_an_unparseable_url_unproxied() {
        configure("manual", "http://proxy.corp:3128", "");
        assert_eq!(egress("not a url"), Egress::Unproxied);
        assert_eq!(egress(""), Egress::Unproxied);
        reset();
    }

    /// The label a proxied job gets is host and port only. `Proxy::builder`
    /// cannot take userinfo through this door (`split` refuses it), but
    /// `try_from_env` can, so the rule is stated where both are read.
    #[test]
    fn egress_labels_no_password() {
        configure("manual", "http://proxy.corp:3128", "");
        let egress = egress("https://example.com/a.bin");
        let text = egress.via().unwrap_or_default().to_string();
        assert_eq!(text, "manual proxy.corp:3128");
        assert!(!text.contains('@'));
        reset();
    }

    #[test]
    fn a_credential_reaches_the_built_proxy() {
        configure("manual", "http://proxy.corp:3128", "");
        set_auth("proxy.corp:3128", "agent", "s3cret");
        let proxy = resolve().unwrap();
        assert_eq!(proxy.username(), Some("agent"));
        assert_eq!(proxy.password(), Some("s3cret"));
        reset();
    }

    /// The pair survives the percent-encode/decode round trip `build()` and
    /// `Proxy::new_with_flag` do between the builder and the CONNECT writer,
    /// because a password of that shape is a normal one and the alternative is a
    /// credential that quietly is not the one that was typed.
    #[test]
    fn a_secret_with_uri_reserved_bytes_round_trips() {
        configure("manual", "http://proxy.corp:3128", "");
        set_auth("proxy.corp:3128", "a%b:c@d", "p@ss/w:ord+1");
        let proxy = resolve().unwrap();
        assert_eq!(proxy.username(), Some("a%b:c@d"));
        assert_eq!(proxy.password(), Some("p@ss/w:ord+1"));
        // Not the wire form: what the CONNECT writer base64s is the decoded pair.
        assert!(!proxy.uri().to_string().contains("p@ss"));
        reset();
    }

    /// A credential is keyed to a proxy, not to the app: the row outlives the URL
    /// it was typed for, and sending its password to the next one is the leak this
    /// rule exists to prevent.
    #[test]
    fn a_credential_for_another_proxy_is_not_used() {
        configure("manual", "http://proxy.corp:3128", "");
        set_auth("other.corp:3128", "agent", "s3cret");
        let proxy = resolve().unwrap();
        assert_eq!(proxy.username(), None);
        assert_eq!(proxy.password(), None);

        // Same host, different port: the authority is the key, port included.
        set_auth("proxy.corp:3129", "agent", "s3cret");
        assert_eq!(resolve().unwrap().username(), None);

        // And a URL moved under a credential already stored leaves it inert.
        set_auth("proxy.corp:3128", "agent", "s3cret");
        assert!(resolve().unwrap().username().is_some());
        configure("manual", "http://proxy.corp:8080", "");
        assert_eq!(resolve().unwrap().username(), None);
        // `configure` did not clear it — the row is still the one the settings
        // screen shows, and re-typing the URL puts it back to work.
        configure("manual", "http://proxy.corp:3128", "");
        assert!(resolve().unwrap().username().is_some());
        reset();
    }

    /// A URL with no port is matched on the protocol's own default, which is what
    /// `auth_host` reports for it. Writing `secure.corp` in the key would strand
    /// every credential typed against a default-port proxy.
    #[test]
    fn a_default_port_authority_matches() {
        configure("manual", "https://secure.corp", "");
        assert_eq!(auth_host().as_deref(), Some("secure.corp:443"));
        set_auth("secure.corp:443", "agent", "s3cret");
        let proxy = resolve().unwrap();
        assert_eq!(proxy.username(), Some("agent"));
        assert_eq!(proxy.port(), 443);
        reset();
        assert_eq!(auth_host(), None);
    }

    #[test]
    fn auth_host_names_only_a_manual_proxy() {
        assert_eq!(auth_host(), None);
        configure("manual", "http://proxy.corp:3128", "");
        assert_eq!(auth_host().as_deref(), Some("proxy.corp:3128"));
        // A manual mode with an unusable URL has no proxy to key a credential to.
        configure("manual", "socks5://proxy.corp:1080", "");
        assert_eq!(auth_host(), None);
        // System mode resolves a proxy but is not one Sucr can add a credential to
        // without dropping the environment's own `NO_PROXY` (see `auth_host`).
        configure("system", "", "");
        assert_eq!(auth_host(), None);
        reset();
    }

    /// The rebuild is the risk this design carries: `Proxy::new` would have been
    /// one line and would have lost the bypass list. Checked here, because the
    /// credential path and the bypass path meet inside one builder.
    #[test]
    fn a_credential_keeps_the_bypass_list() {
        configure("manual", "http://proxy.corp:3128", "internal.corp, .lab.example");
        set_auth("proxy.corp:3128", "agent", "s3cret");
        let proxy = resolve().unwrap();
        assert_eq!(proxy.username(), Some("agent"));
        assert!(proxy.is_no_proxy(&Uri::from_static("http://internal.corp/file")));
        assert!(proxy.is_no_proxy(&Uri::from_static("http://a.lab.example/x")));
        assert!(!proxy.is_no_proxy(&Uri::from_static("http://example.com/x")));
        // And the per-URL answer the card prints did not change with it.
        assert_eq!(
            egress("http://internal.corp/file"),
            Egress::Bypassed("manual proxy.corp:3128".to_string())
        );
        reset();
    }

    /// Nothing on either label may carry the secret: `via()` is a status line and
    /// `egress()`'s string is written into the history table for good.
    #[test]
    fn a_secret_never_reaches_a_label() {
        configure("manual", "http://proxy.corp:3128", "");
        set_auth("proxy.corp:3128", "agent", "s3cret");
        assert_eq!(via(), "manual proxy.corp:3128");
        let text = egress("https://example.com/a.bin").via().unwrap_or("").to_string();
        assert_eq!(text, "manual proxy.corp:3128");
        assert!(!via().contains("s3cret"));
        assert!(!text.contains("s3cret"));
        assert!(!text.contains("agent"));
        reset();
    }

    /// Forgetting is an empty username, and it clears the pair even when it is
    /// called for a proxy the credential never matched.
    #[test]
    fn an_empty_username_forgets_the_credential() {
        configure("manual", "http://proxy.corp:3128", "");
        set_auth("proxy.corp:3128", "agent", "s3cret");
        assert!(resolve().unwrap().username().is_some());
        set_auth("", "", "");
        assert_eq!(resolve().unwrap().username(), None);
        reset();
    }
}
