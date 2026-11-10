// Origin credentials: one per host, either a Basic pair or a bearer token, put on the
// request before it leaves.
//
// A mirror, exactly like `engine::proxy` — the agent-building sites cannot hold a
// database handle, and the answer to "does this host have a credential?" has to be as
// cheap as the answer to "am I proxied?". The credential lives in `credentials` with
// `scheme='basic'` or `scheme='header-secret'`; this is the copy the engine reads.
//
// The header is written by a `Middleware` attached in `config_ua`, which is the one
// function every request in this app is built from — the pre-flight probe, both
// transfer paths, the refresh and the add bar's link check. Attaching it per request
// instead would mean repeating the lookup at each of those sites, which is the
// pattern the User-Agent already refused to follow.

use std::collections::BTreeMap;
use std::sync::Mutex;

use ureq::http::{HeaderValue, Request, Response};
use ureq::middleware::{Middleware, MiddlewareNext};
use ureq::{Body, SendBody};

/// Which header a stored credential turns into. One kind per host, not one of each: the
/// middleware can put exactly one `Authorization` on a request, so a host that is given a
/// token after a password has one credential, the token, and the password is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Basic,
    Bearer,
}

impl Kind {
    /// The `credentials.scheme` this kind is stored under. `header-secret` is the name the
    /// table's own CHECK already allows, which is why adding tokens needed no schema
    /// version — the word was in the list before anything could use it.
    pub fn scheme(self) -> &'static str {
        match self {
            Kind::Basic => "basic",
            Kind::Bearer => "header-secret",
        }
    }

    /// Read back from the stored name. Total by design: the queries that reach here filter
    /// to exactly the two schemes above, so a third word cannot arrive, and anything that
    /// is not a token is treated as a pair rather than as an error at the boot read.
    pub fn from_scheme(scheme: &str) -> Kind {
        match scheme {
            "header-secret" => Kind::Bearer,
            _ => Kind::Basic,
        }
    }
}

/// A credential the mirror holds. The length limits on both halves live with the store
/// ([`crate::db::MAX_ORIGIN_USERNAME_BYTES`] and its companion), not here: the write path
/// validates, and whatever reached this table already passed. The engine's answer to a
/// host whose credential cannot produce a header — a pair with no name, a token with no
/// token — is "no header", the same failure the proxy mirror chose rather than putting a
/// nameless password on the wire.
#[derive(Clone)]
struct Credential {
    kind: Kind,
    username: String,
    secret: String,
}

/// Whether this row can become a header at all. Kept as one rule so the boot read and a
/// live write cannot disagree about what a usable credential is.
fn usable(kind: Kind, username: &str, secret: &str) -> bool {
    match kind {
        Kind::Basic => !username.is_empty(),
        Kind::Bearer => !secret.is_empty(),
    }
}

/// Keyed by `engine::host_key`, i.e. the host without its port — the same
/// normalisation the pasted cookie sessions use, so one host has one identity in this
/// table. The consequence is worth stating rather than discovering: a password typed
/// for `example.com` is sent to every port on `example.com`, where a browser would
/// key it to the exact origin.
static STORE: Mutex<BTreeMap<String, Credential>> = Mutex::new(BTreeMap::new());

fn lock() -> std::sync::MutexGuard<'static, BTreeMap<String, Credential>> {
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Replace the whole store from the library. Called at boot, and the only thing that
/// makes a credential stored in a previous run real in this one.
///
/// A row that cannot become a header is skipped: a pair with no name, or a token with
/// nothing in it. Putting either on the wire would report the host as credentialled while
/// every request went out anonymous.
pub fn configure(rows: Vec<(String, Kind, String, String)>) {
    let mut store = lock();
    store.clear();
    for (host, kind, username, secret) in rows {
        if !usable(kind, &username, &secret) {
            continue;
        }
        store.insert(
            host,
            Credential {
                kind,
                username,
                secret,
            },
        );
    }
}

/// Write or forget one host's credential. An unusable one is the forget, which is the
/// same rule the proxy credential uses: the boxes are one field, and clearing them is how
/// a credential stops existing — whatever kind it was.
pub fn set(host: &str, kind: Kind, username: &str, secret: &str) {
    let mut store = lock();
    if !usable(kind, username, secret) {
        store.remove(host);
    } else {
        store.insert(
            host.to_string(),
            Credential {
                kind,
                username: username.to_string(),
                secret: secret.to_string(),
            },
        );
    }
}

/// Whether a host has a credential in the engine's own copy. The Settings row asks this so
/// it can say "the stored credential was refused" instead of "this needs a credential" —
/// two different user actions that a bare 401 cannot tell apart. Asked of the host, never
/// of the kind: a 401 says the identity was refused, not which header carried it.
pub fn has(host: &str) -> bool {
    lock().contains_key(host)
}

/// The header value a host earns, or `None`. `already` is whether the request carries
/// an `Authorization` of its own: a caller that put one there wins, so nothing in
/// this store ever overwrites a header someone else wrote.
pub fn authorization_for(host: &str, already: bool) -> Option<HeaderValue> {
    if already {
        return None;
    }
    let credential = lock().get(host).cloned()?;
    // `from_str` is the fallible step, and it only fails on characters a header
    // cannot hold — control bytes inside base64 are not reachable, so a `None` here
    // means the credential is unusable and the request goes out unauthenticated rather
    // than half-built.
    let value = match credential.kind {
        Kind::Basic => format!("Basic {}", basic_pair(&credential.username, &credential.secret)),
        Kind::Bearer => format!("Bearer {}", credential.secret),
    };
    HeaderValue::from_str(&value).ok()
}

/// `user:password` in Base64. Public because the Digest step will want the same
/// userinfo encoding under a different scheme.
pub fn basic_pair(username: &str, secret: &str) -> String {
    base64(format!("{username}:{secret}").as_bytes())
}

/// The one place the origin credential is attached. A non-capturing closure: the
/// state it needs is this module's own store, so every agent can carry it without
/// cloning anything per request.
pub fn middleware() -> impl Middleware {
    |request: Request<SendBody>, next: MiddlewareNext<'_>| -> Result<Response<Body>, ureq::Error> {
        let host = request.uri().host().map(str::to_ascii_lowercase);
        let already = request.headers().contains_key("authorization");
        let value = host.as_deref().and_then(|h| authorization_for(h, already));
        let request = match value {
            Some(value) => {
                let (mut parts, body) = request.into_parts();
                parts.headers.insert("authorization", value);
                Request::from_parts(parts, body)
            }
            None => request,
        };
        next.handle(request)
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard Base64 with padding (RFC 4648 §4), encode only.
///
/// Hand-written rather than a `base64` dependency on purpose: ureq already links that
/// crate for its own `Proxy-Authorization`, but it is not re-exported, so reaching for
/// it means declaring a direct dependency and instantiating its generic engine here.
/// This is the smaller of the two, and it has nothing to do but encode.
fn base64(input: &[u8]) -> String {
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = (u32::from(chunk[0])) << 16 | u32::from(b1) << 8 | u32::from(b2);
        out.push(ALPHABET[(n >> 18) as usize] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{channel, Receiver};
    use std::thread;

    use ureq::ResponseExt;

    use super::*;

    /// RFC 4648 §4's own vectors — the three padding cases at once.
    #[test]
    fn base64_matches_the_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_pair_encodes_as_user_colon_password() {
        assert_eq!(basic_pair("sucr", "pw1"), "c3VjcjpwdzE=");
        assert_eq!(basic_pair("", ""), "Og==");
        // Non-ASCII stays bytes-on-bytes: the header is latin-1, and a UTF-8 password
        // encodes as its UTF-8 bytes, which is what RFC 7617 tells the server to try.
        assert_eq!(basic_pair("é", "é"), "w6k6w6k=");
    }

    /// The storage contract, in one place: both names come from the table's own CHECK list,
    /// and the read-back is the write's inverse for each of them. A future third kind has
    /// to be added to both halves of this pair on purpose.
    #[test]
    fn each_kind_is_stored_under_the_scheme_that_names_it() {
        assert_eq!(Kind::Basic.scheme(), "basic");
        assert_eq!(Kind::Bearer.scheme(), "header-secret");
        assert_eq!(Kind::from_scheme("basic"), Kind::Basic);
        assert_eq!(Kind::from_scheme("header-secret"), Kind::Bearer);
    }

    #[test]
    fn the_store_answers_per_host_and_nothing_else() {
        configure(vec![
            (
                "a.example".to_string(),
                Kind::Basic,
                "agent".to_string(),
                "s3cret".to_string(),
            ),
            // no username: not a credential, so not loaded
            (
                "b.example".to_string(),
                Kind::Basic,
                String::new(),
                "orphan".to_string(),
            ),
            // a token row with nothing in the token is the same mistake in the other kind
            (
                "e.example".to_string(),
                Kind::Bearer,
                String::new(),
                String::new(),
            ),
        ]);
        assert!(has("a.example"));
        assert!(!has("b.example"));
        assert!(!has("c.example"));
        assert!(!has("e.example"), "an empty token is not a credential");

        let value = authorization_for("a.example", false).expect("header");
        assert_eq!(value.to_str().expect("ascii"), "Basic YWdlbnQ6czNjcmV0");
        // The secret does not come back out of the answer the UI reads.
        assert!(!has("b.example"), "an orphan row stays unused");

        set("c.example", Kind::Basic, "agent", "s3cret");
        assert!(has("c.example"));
        set("c.example", Kind::Basic, "", "");
        assert!(!has("c.example"), "both boxes empty is how forgetting works");
        assert!(authorization_for("c.example", false).is_none());
    }

    /// The two kinds on one store, and the header each one earns. A token is sent as it
    /// was pasted: this app does not base64 it, trim its padding or guess a scheme word —
    /// `Authorization: Bearer <paste>` is the whole rule, which is what an API host asking
    /// for a bearer token checks.
    #[test]
    fn a_token_goes_out_as_bearer_and_a_pair_as_basic() {
        configure(vec![
            (
                "pair.example".to_string(),
                Kind::Basic,
                "agent".to_string(),
                "s3cret".to_string(),
            ),
            (
                "api.example".to_string(),
                Kind::Bearer,
                String::new(),
                "ey.abc-123".to_string(),
            ),
        ]);
        assert_eq!(
            authorization_for("pair.example", false)
                .expect("basic")
                .to_str()
                .expect("ascii"),
            "Basic YWdlbnQ6czNjcmV0"
        );
        assert_eq!(
            authorization_for("api.example", false)
                .expect("bearer")
                .to_str()
                .expect("ascii"),
            "Bearer ey.abc-123"
        );
    }

    /// One credential per host is the store's rule, not a preference: the middleware can
    /// write one `Authorization`, so the second kind typed for a host replaces the first
    /// rather than competing with it — including when the replacement is an unusable one.
    #[test]
    fn the_second_kind_written_for_a_host_replaces_the_first() {
        set("swap.example", Kind::Basic, "agent", "s3cret");
        set("swap.example", Kind::Bearer, "", "tok");
        assert_eq!(
            authorization_for("swap.example", false)
                .expect("token")
                .to_str()
                .expect("ascii"),
            "Bearer tok"
        );
        // And the password is gone from the mirror, not merely hidden behind the token.
        set("swap.example", Kind::Basic, "", "");
        assert!(!has("swap.example"), "forgetting by host clears either kind");
    }

    #[test]
    fn a_request_that_already_carries_authorization_is_left_alone() {
        configure(vec![(
            "d.example".to_string(),
            Kind::Basic,
            "agent".to_string(),
            "x".to_string(),
        )]);
        assert!(authorization_for("d.example", true).is_none());
        assert!(authorization_for("d.example", false).is_some());
        configure(Vec::new());
        assert_eq!(authorization_for("d.example", false), None, "and boot clears it");
    }

    /// A one-shot HTTP server on loopback: it answers `count` connections, keeps the
    /// bytes each request arrived with, and answers with a bare body. Nothing here is
    /// timed, so no sleep is involved and no port is hard-coded.
    fn capture_requests(
        count: usize,
        respond: impl Fn(usize) -> String + Send + 'static,
    ) -> (u16, Receiver<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = channel();
        thread::spawn(move || {
            let mut seen = Vec::new();
            for (i, stream) in listener.incoming().take(count).enumerate() {
                let mut s = match stream {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                // The header block is what matters; the request line goes with it so a
                // failure names the path that was asked for.
                seen.push(text.split("\r\n\r\n").next().unwrap_or(text.as_str()).to_string());
                let _ = s.write_all(respond(i).as_bytes());
                let _ = s.flush();
            }
            let _ = tx.send(seen);
        });
        (port, rx)
    }

    const CLOSED: &str = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

    /// The shape step 4 rests on: the pair is on the request as it leaves, so a server
    /// that would have to challenge never gets the chance to. This is the same claim the
    /// proxy step measured for `Proxy-Authorization`, proven here at the byte level
    /// because the origin header is this app's own work, not ureq's.
    #[test]
    fn the_pair_is_on_the_request_as_it_leaves() {
        crate::engine::proxy::configure("direct", "", "");
        let (port, rx) = capture_requests(2, |_| CLOSED.to_string());
        let url = format!("http://127.0.0.1:{port}/a.bin");
        let host = crate::engine::host_key(&url);

        configure(vec![(
            host.clone(),
            Kind::Basic,
            "agent".to_string(),
            "s3cret".to_string(),
        )]);
        let agent = crate::engine::agent();
        assert_eq!(agent.get(&url).call().unwrap().status().as_u16(), 200);
        // And nothing, from the same store a moment later — so the header above is the
        // credential's doing and not something ureq put there on its own.
        configure(Vec::new());
        let _ = agent.get(&url).call().unwrap();

        let seen = rx.recv().expect("requests");
        assert_eq!(seen.len(), 2);
        assert!(
            seen[0].contains("authorization: Basic YWdlbnQ6czNjcmV0"),
            "the pair was not on the first request: {seen:?}"
        );
        assert!(
            !seen[1].to_ascii_lowercase().contains("authorization"),
            "an empty store still put a header on the wire: {seen:?}"
        );
    }

    /// The same claim as the pair, for the other kind: a pasted token reaches the wire
    /// exactly as it was typed. The token below is deliberately not Base64 — dots, dashes
    /// and a trailing `==` that belongs to nothing — so an accidental encoding step here
    /// would show up as a different byte string rather than pass silently.
    #[test]
    fn a_token_is_on_the_request_exactly_as_pasted() {
        crate::engine::proxy::configure("direct", "", "");
        let (port, rx) = capture_requests(1, |_| CLOSED.to_string());
        let url = format!("http://127.0.0.1:{port}/a.bin");
        let host = crate::engine::host_key(&url);

        configure(vec![(
            host.clone(),
            Kind::Bearer,
            String::new(),
            "eyJ.abc-12==".to_string(),
        )]);
        let agent = crate::engine::agent();
        assert_eq!(agent.get(&url).call().unwrap().status().as_u16(), 200);
        configure(Vec::new());

        let seen = rx.recv().expect("requests");
        assert!(
            seen[0].contains("authorization: Bearer eyJ.abc-12=="),
            "the token was not sent verbatim: {seen:?}"
        );
    }

    /// `SameHost` is set in `config_ua` for this reason, and the claim is the config
    /// comment's: the middleware runs once, before ureq's redirect loop, so the header
    /// reaching hop two is the redirect policy's answer, not this module's. If ureq ever
    /// changes that default, this test is where the difference shows up.
    #[test]
    fn a_same_host_redirect_keeps_the_pair() {
        crate::engine::proxy::configure("direct", "", "");
        let (port, rx) = capture_requests(2, move |i| {
            if i == 0 {
                format!("HTTP/1.1 302 Found\r\nlocation: /b.bin\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            } else {
                CLOSED.to_string()
            }
        });
        let url = format!("http://127.0.0.1:{port}/a.bin");
        configure(vec![(
            crate::engine::host_key(&url),
            Kind::Basic,
            "agent".to_string(),
            "s3cret".to_string(),
        )]);
        let agent = crate::engine::agent();
        let resp = agent.get(&url).call().unwrap();
        configure(Vec::new());

        assert_eq!(resp.status().as_u16(), 200, "the redirect was not followed");
        assert!(resp.get_uri().path() == "/b.bin", "landed on {resp:?}");
        let seen = rx.recv().expect("requests");
        assert!(
            seen[1].contains("authorization: Basic YWdlbnQ6czNjcmV0"),
            "the pair was dropped on the same-host hop: {seen:?}"
        );
    }
}
