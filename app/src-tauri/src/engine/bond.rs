//! Multi-device bonding: making a download leave on a chosen adapter.
//!
//! Without a bound socket the routing table picks one interface for every
//! stream, so "bonding" would be a label on a single NIC's throughput. Binding
//! the source address is what Windows uses to select the interface, and it
//! needs no administrator rights — unlike `SO_BINDTODEVICE`, which names the
//! interface directly but is not available to an unprivileged process here.

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{
    Buffers, ConnectProxyConnector, ConnectionDetails, Connector, Either, LazyBuffers,
    NativeTlsConnector, NextTimeout, Transport,
};
use ureq::{Agent, Error};

/// One route a worker can be handed.
///
/// The colour and label come from the `networks` row rather than being invented
/// here, so a block's stripe in the Sucker Map matches the card it came from.
#[derive(Clone, Debug)]
pub struct Route {
    /// `networks.id`. None for the OS default route, which bonds nothing.
    pub network_id: Option<i64>,
    /// Colour id shared with the Networks deck, e.g. `n2`.
    pub color: String,
    /// Label for the Connections table, e.g. `Wi-Fi`.
    pub label: String,
    /// Source address to bind. None lets the routing table decide.
    pub bind: Option<IpAddr>,
}

impl Route {
    /// The unbonded case: one route, no binding, whatever Windows would have
    /// picked anyway. Used when fewer than two adapters are enabled, so a
    /// machine with a single NIC never takes the bonding path.
    ///
    /// Its colour is the engine's "I cannot say which device carried this" id,
    /// not `n1`: the attempt row for an unbonded block stores a NULL network,
    /// and the map reads that back as unattributed. Painting the live block with
    /// a real device's colour would make the same byte change colour the moment
    /// the job was re-read from SQLite.
    ///
    /// `job::attribute_egress` may replace that colour and store an id before
    /// any byte moves, but only once the routing table has named the interface —
    /// so the two always change together.
    pub fn default_route() -> Route {
        Route {
            network_id: None,
            color: super::job::live::UNATTRIBUTED.to_string(),
            label: "Default route".to_string(),
            bind: None,
        }
    }

    /// The agent this route transfers with, built once per download.
    ///
    /// Not one agent re-bound per request: ureq pools sockets by origin, so a
    /// shared pool would hand a "Wi-Fi" attempt a socket that left through
    /// Ethernet. Each route gets its own agent, and workers clone it, which
    /// keeps the pool per route rather than per thread.
    pub fn agent(&self) -> Agent {
        self.agent_with(None)
    }

    /// The same agent under a browser session's identity. The UA belongs to the
    /// agent rather than the request because Cloudflare checks it against the
    /// cookie on every hit, and a per-request override would have to be repeated
    /// at each of the four places this app builds a request.
    ///
    /// With a proxy configured the CONNECT hop is the bound socket, not the
    /// origin: `ConnectProxyConnector` asks the rest of this chain for a
    /// connection to the proxy with the proxy stripped from the config it
    /// re-enters on, so `BoundConnector` binds the source address to that
    /// hop and the proxy is reached on this route's own adapter. Measured, not
    /// inferred — see `design/proxy-and-http-auth.md` §4.7.
    pub fn agent_with(&self, user_agent: Option<&str>) -> Agent {
        let config = super::config_session(user_agent);
        match self.bind {
            None => config.new_agent(),
            Some(local_ip) => Agent::with_parts(
                config,
                ConnectProxyConnector::default()
                    .chain(BoundConnector { local_ip })
                    .chain(NativeTlsConnector::default()),
                DefaultResolver::default(),
            ),
        }
    }
}

/// The source address the routing table would use for this URL's server.
///
/// Nothing is sent: a connected UDP socket makes the stack look up a route and
/// record a peer, and `local_addr` then reports the address that lookup chose.
/// One `send` would be needed for a packet to leave, and there is none, so this
/// costs a syscall rather than traffic. It is the OS's own answer, not a guess
/// from a metric — the same answer `bind` would have forced us to choose, but
/// without binding, which is what keeps localhost targets reachable.
pub fn egress_source(url: &str) -> Option<std::net::IpAddr> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

    let dest = server_addr(url)?;
    let any = if dest.is_ipv4() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
    } else {
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)
    };
    let socket = UdpSocket::bind(any).ok()?;
    socket.connect(dest).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// `host:port` of a URL as a socket address, the same authority split
/// `filename_from_url` uses. IPv4 first: the adapter table this is matched
/// against carries IPv4 addresses.
fn server_addr(url: &str) -> Option<std::net::SocketAddr> {
    use std::net::ToSocketAddrs;

    let (scheme, rest) = match url.split_once("://") {
        Some(split) => split,
        None => ("http", url),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map(|(_, host)| host).unwrap_or(authority);
    let without_port = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.parse::<u16>().is_ok() => host,
        _ => authority,
    };
    let host = without_port.trim_matches(|c| c == '[' || c == ']');
    if host.is_empty() {
        return None;
    }
    let port = match authority.rsplit_once(':') {
        Some((_, port)) if port.parse::<u16>().is_ok() => port.parse::<u16>().ok()?,
        _ => {
            if scheme == "https" {
                443
            } else {
                80
            }
        }
    };
    let found: Vec<std::net::SocketAddr> = (host, port).to_socket_addrs().ok()?.collect();
    found
        .iter()
        .find(|addr| addr.is_ipv4())
        .copied()
        .or_else(|| found.first().copied())
}

/// A connector that opens every socket on one adapter's own source address.
///
/// Generic over what it may receive: with no proxy the chain hands it nothing
/// and it connects the origin itself; behind a `ConnectProxyConnector` it is
/// handed that connector's tunnel and passes it through untouched, because the
/// socket the tunnel sits on was already bound by the same code, one frame
/// earlier.
#[derive(Debug)]
struct BoundConnector {
    local_ip: IpAddr,
}

impl<In: Transport> Connector<In> for BoundConnector {
    type Out = Either<In, BoundTransport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, Error> {
        if chained.is_some() {
            // Something earlier in the chain already made the connection.
            return Ok(chained.map(Either::A));
        }

        let config = details.config;
        let budget = details.timeout.not_zero().map(|d| *d);
        let started = Instant::now();
        let mut last_err: Option<io::Error> = None;

        for addr in &details.addrs {
            // A bound socket only speaks its own family. An IPv6 answer to a
            // host that also has A records would leave on a different route
            // than the one this worker was given.
            if !family_matches(self.local_ip, *addr) {
                continue;
            }
            // The whole connect shares one deadline, spread over the addresses
            // of this family, the same way the stock connector spreads it.
            let remaining = match budget {
                None => None,
                Some(total) => {
                    let left = total.saturating_sub(started.elapsed());
                    if left.is_zero() {
                        break;
                    }
                    Some(left)
                }
            };
            match bind_connect(*addr, self.local_ip, remaining) {
                Ok(stream) => {
                    if config.no_delay() {
                        stream.set_nodelay(true)?;
                    }
                    let buffers = LazyBuffers::new(
                        config.input_buffer_size(),
                        config.output_buffer_size(),
                    );
                    return Ok(Some(Either::B(BoundTransport::new(stream, buffers))));
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::TimedOut {
                        return Err(Error::Timeout(details.timeout.reason));
                    }
                    last_err = Some(e);
                }
            }
        }

        Err(match last_err {
            Some(e) => e.into(),
            // Nothing resolved to this family, or the deadline went first.
            None => Error::ConnectionFailed,
        })
    }
}

/// Whether a resolved address may be reached from this bound source: the two
/// families must agree, or the socket would leave on another adapter's route.
fn family_matches(local: IpAddr, remote: SocketAddr) -> bool {
    local.is_ipv4() == remote.ip().is_ipv4()
}

/// Bind to `local_ip` (any port) and connect, with an optional deadline.
fn bind_connect(
    remote: SocketAddr,
    local_ip: IpAddr,
    budget: Option<Duration>,
) -> io::Result<TcpStream> {
    let socket = Socket::new(Domain::for_address(remote), Type::STREAM, Some(Protocol::TCP))?;
    socket.bind(&SockAddr::from(SocketAddr::new(local_ip, 0)))?;
    let target = SockAddr::from(remote);
    match budget {
        Some(d) => socket.connect_timeout(&target, d)?,
        None => socket.connect(&target)?,
    }
    Ok(TcpStream::from(socket))
}

/// A bound socket presented to ureq as a `Transport`.
///
/// ureq keeps its own `TcpTransport` private, so a bespoke connector supplies
/// its own — the pattern in ureq's `mpsc-transport` example. The semantics
/// mirror the stock one: a blocking socket with per-operation timeouts, writes
/// of the whole requested amount, and interrupted reads retried against the
/// remaining budget.
#[derive(Debug)]
struct BoundTransport {
    stream: TcpStream,
    buffers: LazyBuffers,
    timeout_read: Option<Duration>,
    timeout_write: Option<Duration>,
}

impl BoundTransport {
    fn new(stream: TcpStream, buffers: LazyBuffers) -> Self {
        BoundTransport {
            stream,
            buffers,
            timeout_read: None,
            timeout_write: None,
        }
    }

    /// Only call `set_*_timeout` when the value actually moved; these are
    /// per-request hot paths and the timeout rarely changes.
    fn apply_read_timeout(&mut self, per: Option<Duration>) -> io::Result<()> {
        if per != self.timeout_read {
            self.stream.set_read_timeout(per)?;
            self.timeout_read = per;
        }
        Ok(())
    }

    fn apply_write_timeout(&mut self, per: Option<Duration>) -> io::Result<()> {
        if per != self.timeout_write {
            self.stream.set_write_timeout(per)?;
            self.timeout_write = per;
        }
        Ok(())
    }
}

impl Transport for BoundTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        if timeout.after.is_zero() {
            return Err(Error::Timeout(timeout.reason));
        }
        let per = timeout.not_zero().map(|d| *d);
        self.apply_write_timeout(per)?;

        let output = &self.buffers.output()[..amount];
        match (&self.stream).write_all(output) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => Err(Error::Timeout(timeout.reason)),
            Err(e) => Err(e.into()),
        }
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        if timeout.after.is_zero() {
            return Err(Error::Timeout(timeout.reason));
        }
        let per = timeout.not_zero().map(|d| *d);
        self.apply_read_timeout(per)?;

        let input = self.buffers.input_append_buf();
        let stream = &mut self.stream;
        let started = Instant::now();
        let read = loop {
            match stream.read(input) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                    let Some(total) = per else {
                        return Err(e.into());
                    };
                    let left = total.saturating_sub(started.elapsed());
                    if left.is_zero() {
                        return Err(Error::Timeout(timeout.reason));
                    }
                    stream.set_read_timeout(Some(left))?;
                }
                result => break result,
            }
        };

        let amount = match read {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                return Err(Error::Timeout(timeout.reason))
            }
            Err(e) => return Err(e.into()),
        };
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        probe(&self.stream).unwrap_or(false)
    }
}

/// Is this pooled socket still usable? Non-blocking peek: a byte waiting means
/// the server talked out of turn, an error means it is gone.
fn probe(stream: &TcpStream) -> io::Result<bool> {
    stream.set_nonblocking(true)?;
    let mut peek = stream;
    let mut buf = [0u8];
    let open = match peek.read(&mut buf) {
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => true,
        _ => false,
    };
    stream.set_nonblocking(false)?;
    Ok(open)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr, TcpListener};
    use std::sync::{Arc, Mutex};

    /// Loopback answers on every address in 127.0.0.0/8, so a second address in
    /// it stands in for a second NIC when the machine only has one route.
    const SECOND: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

    /// One-shot HTTP/1.1 server on 127.0.0.1 that records who actually called.
    fn serve_one(response: &'static str) -> (u16, Arc<Mutex<Vec<SocketAddr>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        let seen: Arc<Mutex<Vec<SocketAddr>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                // The source address is known the moment the socket is accepted;
                // waiting for a request first would make the plumbing-only test
                // hang on a client that never speaks.
                if let Ok(peer) = stream.peer_addr() {
                    recorder.lock().unwrap().push(peer);
                }
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (port, seen)
    }

    /// The accept loop runs on its own thread, so a socket that is already
    /// connected is not necessarily recorded yet.
    fn await_source(seen: &Arc<Mutex<Vec<SocketAddr>>>) -> IpAddr {
        for _ in 0..200 {
            if let Some(peer) = seen.lock().unwrap().first() {
                return peer.ip();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the server never saw the connection");
    }

    #[test]
    fn binding_changes_the_source_address() {
        let (port, seen) = serve_one("HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
        let remote = SocketAddr::from(([127, 0, 0, 1], port));
        let stream = bind_connect(remote, SECOND, Some(Duration::from_secs(5))).expect("connect");
        assert_eq!(
            stream.local_addr().unwrap().ip(),
            SECOND,
            "the socket left on the address we asked for"
        );
        assert_eq!(await_source(&seen), SECOND, "and the far end agrees");
    }

    #[test]
    fn a_bound_agent_transfers_a_real_response() {
        let payload = "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nbytes";
        let (port, seen) = serve_one(payload);
        let route = Route {
            network_id: Some(7),
            color: "n2".to_string(),
            label: "Loopback B".to_string(),
            bind: Some(SECOND),
        };
        let mut res = route
            .agent()
            .get(format!("http://127.0.0.1:{port}/file"))
            .call()
            .expect("request over the bound route");
        assert_eq!(res.body_mut().read_to_string().unwrap(), "bytes");
        assert_eq!(seen.lock().unwrap().first().unwrap().ip(), SECOND);
    }

    #[test]
    fn an_unbound_route_still_works() {
        let (port, seen) = serve_one(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
        );
        let mut res = Route::default_route()
            .agent()
            .get(format!("http://127.0.0.1:{port}/file"))
            .call()
            .expect("default route request");
        assert_eq!(res.body_mut().read_to_string().unwrap(), "ok");
        assert_eq!(
            seen.lock().unwrap().first().unwrap().ip(),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            "unbound means the stack picks, and loopback is all it can pick"
        );
    }

    #[test]
    fn a_bound_source_only_uses_its_own_family() {
        let v4 = SocketAddr::from(([127, 0, 0, 1], 80));
        let v6 = SocketAddr::from((Ipv6Addr::LOCALHOST, 80));
        let local = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
        assert!(family_matches(local, v4));
        assert!(
            !family_matches(local, v6),
            "a v4-bound route must not answer for a v6 target, or the bytes \
             leave on whatever interface owns v6"
        );
    }
}
