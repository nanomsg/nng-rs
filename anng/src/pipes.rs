use crate::Socket;
use core::ffi::CStr;
use core::ffi::c_char;
use core::fmt;
use core::net::Ipv4Addr;
use core::net::Ipv6Addr;
use core::net::SocketAddr;
use core::net::SocketAddrV6;
use core::ops::Deref;
use std::ffi::CString;
use std::io;

/// A handle to a just-started NNG dialer.
///
/// Note that dropping this handle does **not** close the dialer.
#[allow(dead_code)]
pub struct Dialer<'socket, Protocol> {
    pub(crate) socket: &'socket Socket<Protocol>,
    pub(crate) dialer: nng_sys::nng_dialer,
}

impl<Protocol> fmt::Debug for Dialer<'_, Protocol> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dialer")
            .field("socket", &self.socket)
            .field("dialer", &self.dialer)
            .finish()
    }
}

/// A handle to a just-started NNG listener.
///
/// Note that dropping this handle does **not** close the listener.
#[allow(dead_code)]
pub struct Listener<'socket, Protocol> {
    pub(crate) socket: &'socket Socket<Protocol>,
    pub(crate) listener: nng_sys::nng_listener,
}

impl<Protocol> fmt::Debug for Listener<'_, Protocol> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("socket", &self.socket)
            .field("listener", &self.listener)
            .finish()
    }
}

impl<Protocol> Listener<'_, Protocol> {
    /// Retrieve the local address that this listener is listening on as a URL.
    ///
    /// The port (if any) will be the actual bound port (not 0 even if a ephemeral
    /// port was requested).
    pub fn local_addr(&self) -> io::Result<Url> {
        // Note on URL ownership: nng_listener_get_url returns a borrowed pointer to NNG's
        // internal URL structure (hence `const nng_url **`). This URL is owned by the
        // listener and will be freed when the listener closes. We do NOT call nng_url_free
        // here - that is only for URLs created by nng_url_parse().
        let mut urlp: *const nng_sys::nng_url = core::ptr::null();

        // SAFETY: listener is valid, urlp is valid for writing.
        let errno = unsafe { nng_sys::nng_listener_get_url(self.listener, &mut urlp) };

        // nng_listener_get_url can only fail with NNG_ENOENT if the listener ID is invalid
        // or the listener has been closed. See nni_listener_find in nng/src/core/listener.c.
        let url = match errno {
            0 => {
                debug_assert!(!urlp.is_null(), "URL pointer should not be null on success");
                urlp
            }
            x if x == nng_sys::nng_err::NNG_ENOENT.0 as i32 => {
                unreachable!("listener cannot be closed while &self is held")
            }
            _ => unreachable!(
                "nng_listener_get_url can only return NNG_OK or NNG_ENOENT, got {errno}"
            ),
        };

        // SAFETY: url is valid and returned by nng_listener_get_url.
        // Use NNG_MAXADDRSTRLEN (144 bytes) as the buffer size - this is the maximum URL
        // length defined by NNG (NNG_MAXADDRLEN + 16 for scheme). See nng/include/nng/nng.h.
        const NNG_MAXADDRSTRLEN: usize = 128 + 16; // NNG_MAXADDRLEN + 16
        let url_str = unsafe {
            let mut buf = [0u8; NNG_MAXADDRSTRLEN];
            let written = nng_sys::nng_url_sprintf(buf.as_mut_ptr() as *mut _, buf.len(), url);
            if written < 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "nng_url_sprintf failed",
                ));
            }
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "nng_url_sprintf returned empty URL",
                ));
            }
            let written = written as usize;
            assert!(
                written < NNG_MAXADDRSTRLEN,
                "URL length {written} exceeds NNG_MAXADDRSTRLEN"
            );

            std::str::from_utf8(&buf[..written])
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "URL is not valid UTF-8"))?
                .to_string()
        };

        Ok(Url(url_str))
    }
}

/// A handle to a just-started TCP-transport NNG listener.
///
/// Note that dropping this handle does **not** close the listener.
pub struct TcpListener<'socket, Protocol>(Listener<'socket, Protocol>);

impl<Protocol> fmt::Debug for TcpListener<'_, Protocol> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TcpListener").field(&self.0).finish()
    }
}

impl<'socket, Protocol> Deref for TcpListener<'socket, Protocol> {
    type Target = Listener<'socket, Protocol>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A URL representing an NNG address.
///
/// This is a simple wrapper around a URL string as returned by NNG.
/// The URL scheme indicates the transport type (e.g., `tcp://`, `ipc://`, `inproc://`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Url(String);

impl Url {
    /// Returns the URL string as a reference.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the URL scheme (e.g., "tcp", "ipc", "inproc").
    pub fn scheme(&self) -> &str {
        // All Url instances are constructed with a valid "scheme://..." format:
        // - from_nng() writes "{scheme}://..." via fmt::Write
        // - local_addr() uses nng_url_sprintf which produces valid URLs
        self.0
            .split_once("://")
            .expect("Url invariant violated: missing '://' separator")
            .0
    }

    /// Returns `None` if address is not initialized (ie, family is `NNG_AF_UNSPEC`).
    // NOTE(jon): not From, since that'd make nng_sys be part of the public API.
    pub(crate) fn from_nng(addr: &nng_sys::nng_sockaddr, scheme: &CStr) -> Option<Self> {
        use core::fmt::Write;
        let scheme = scheme.to_str().ok()?;

        // SAFETY: first field of every union member is a u16, so s_family is always okay to access
        let s_family = u32::from(unsafe { addr.s_family });

        // Pre-allocate with typical URL size to reduce reallocations.
        // 64 bytes covers most common cases:
        // - TCP IPv4 max: "tcp://255.255.255.255:65535" (27 bytes)
        // - TCP IPv6 max: "tcp://[ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff%4294967295]:65535" (64 bytes)
        // - IPC/inproc: typically short paths/names
        let mut url = String::with_capacity(64);

        // NOTE: The Display impls of Ipv4Addr, Ipv6Addr, and SocketAddrV6 are stable and
        // guaranteed to produce the canonical text representation (e.g., "127.0.0.1", "::1",
        // "[::1%42]:9090"), which matches NNG's expected URL format.
        match s_family {
            x if x == nng_sys::nng_sockaddr_family::NNG_AF_UNSPEC as u32 => return None,
            x if x == nng_sys::nng_sockaddr_family::NNG_AF_INET as u32 => {
                // SAFETY: we've checked the family
                let addr = unsafe { &addr.s_in };
                let sa_addr = u32::from_be(addr.sa_addr);
                let sa_port = u16::from_be(addr.sa_port);
                let ip = Ipv4Addr::from_bits(sa_addr);
                write!(url, "{scheme}://{ip}:{sa_port}")
                    .expect("fmt::Write for String is infallible");
            }
            x if x == nng_sys::nng_sockaddr_family::NNG_AF_INET6 as u32 => {
                // SAFETY: we've checked the family
                let addr = unsafe { &addr.s_in6 };
                let sa_port = u16::from_be(addr.sa_port);
                let ip = Ipv6Addr::from_bits(u128::from_be_bytes(addr.sa_addr));
                let sock_addr = SocketAddrV6::new(ip, sa_port, 0, addr.sa_scope);
                write!(url, "{scheme}://{sock_addr}").expect("fmt::Write for String is infallible");
            }
            x if x == nng_sys::nng_sockaddr_family::NNG_AF_IPC as u32 => {
                // SAFETY: we've checked the family
                let addr = unsafe { &addr.s_ipc };
                // SAFETY: sa_path is guaranteed to be a C-style string, and we stop using this
                // reference before we drop the `nng_sockaddr`.
                let path = unsafe { CStr::from_ptr(addr.sa_path.as_ptr()) };
                write!(url, "{scheme}://{}", path.to_string_lossy())
                    .expect("fmt::Write for String is infallible");
            }
            x if x == nng_sys::nng_sockaddr_family::NNG_AF_ABSTRACT as u32 => {
                // SAFETY: we've checked the family
                let addr = unsafe { &addr.s_abstract };
                let name = &addr.sa_name[..usize::from(addr.sa_len)];
                // Abstract socket URL encoding:
                // - ASCII graphic characters (0x21-0x7E) are written literally
                // - All other bytes (including space 0x20, null 0x00, and high bytes) are
                //   percent-encoded as %xx
                write!(url, "{scheme}://").expect("fmt::Write for String is infallible");
                for &b in name {
                    if b.is_ascii_graphic() {
                        url.push(b as char);
                    } else {
                        write!(url, "%{b:02x}").expect("fmt::Write for String is infallible");
                    }
                }
            }
            x if x == nng_sys::nng_sockaddr_family::NNG_AF_INPROC as u32 => {
                // SAFETY: we've checked the family
                let addr = unsafe { &addr.s_inproc };
                // SAFETY: sa_name is guaranteed to be a C-style string, and we stop using this
                // reference before we drop the `nng_sockaddr`.
                let name = unsafe { CStr::from_ptr(addr.sa_name.as_ptr()) };
                // TODO(flxo): use CStr::display once
                // https://github.com/rust-lang/rust/issues/139984 is merged.
                write!(url, "{scheme}://{}", name.to_string_lossy())
                    .expect("fmt::Write for String is infallible");
            }
            _ => {
                unimplemented!("support for address family {s_family} has not yet been added")
            }
        }

        Some(Self(url))
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Url> for String {
    fn from(url: Url) -> String {
        url.0
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PipeEnd {
    Local,
    Remote,
}

/// Reads the address of one end of a pipe and formats it as a URL using the pipe's scheme.
///
/// Returns `None` if the pipe has been closed, or if its transport does not report the address.
pub(crate) fn pipe_addr(pipe: nng_sys::nng_pipe, end: PipeEnd) -> Option<Url> {
    use core::mem::MaybeUninit;
    use nng_sys::nng_err;

    let (get_addr, fn_name): (unsafe extern "C" fn(_, _) -> _, _) = match end {
        PipeEnd::Local => (nng_sys::nng_pipe_self_addr, "nng_pipe_self_addr"),
        PipeEnd::Remote => (nng_sys::nng_pipe_peer_addr, "nng_pipe_peer_addr"),
    };

    let mut addr = MaybeUninit::<nng_sys::nng_sockaddr>::uninit();
    // SAFETY: get_addr is nng_pipe_self_addr or nng_pipe_peer_addr (selected above), both of
    // which accept any pipe ID, and addr is valid for writing.
    let errno = unsafe { get_addr(pipe, addr.as_mut_ptr()) };
    let addr = match errno {
        // SAFETY: nng_pipe_self_addr and nng_pipe_peer_addr initialize addr on success.
        nng_err::NNG_OK => unsafe { addr.assume_init() },
        nng_err::NNG_ENOTSUP => {
            tracing::warn!("Pipe does not support {fn_name}");
            return None;
        }
        nng_err::NNG_ENOENT => {
            // The pipe ID is stale - the pipe was closed before the `get_addr` call.
            return None;
        }
        err if err.0 & nng_err::NNG_ESYSERR.0 != 0 => {
            tracing::warn!(
                "{fn_name} returned a system error: {}",
                io::Error::from_raw_os_error((err.0 & !(nng_err::NNG_ESYSERR.0)) as i32)
            );
            return None;
        }
        _ => {
            unreachable!("{fn_name} documentation claims err \"{errno}\" is never returned");
        }
    };

    // Get the URL scheme of the endpoint (listener or dialer) that created this pipe.
    // The address (`nng_sockaddr`) only carries the address family, not the scheme.
    //
    // Note on ownership: NNG hands back a pointer into its static table of recognised
    // schemes (`nni_schemes` in nng/src/core/url.c), not into per-endpoint storage. It is
    // valid for the lifetime of the process and must not be freed.
    let mut scheme_ptr: *const c_char = core::ptr::null();
    // SAFETY: nng_pipe_get_scheme accepts any pipe ID, and scheme_ptr is valid for writing.
    let errno = unsafe { nng_sys::nng_pipe_get_scheme(pipe, &mut scheme_ptr) };
    match errno {
        nng_err::NNG_OK => {}
        // The pipe was closed between `get_addr` and here.
        nng_err::NNG_ENOENT => return None,
        // The pipe has neither a dialer nor a listener, so there is no scheme.
        nng_err::NNG_ENOTSUP => return None,
        _ => {
            unreachable!(
                "nng_pipe_get_scheme documentation claims err \"{errno}\" is never returned"
            );
        }
    }

    // SAFETY: nng_pipe_get_scheme writes `scheme_ptr` and returns NNG_OK only when the
    // scheme is non-null, so on this path it points to a valid C string owned by NNG.
    let scheme = unsafe { CStr::from_ptr(scheme_ptr) };

    Url::from_nng(&addr, scheme)
}

/// Identifies a connection (an NNG pipe). IDs may be reused once a connection closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PipeId(u32);

/// An open connection, as reported by [`Socket::connections`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub local_addr: Option<Url>,
    pub remote_addr: Option<Url>,
}

/// The connections currently open on a socket, as reported by [`Socket::connections`].
pub type Connections = std::collections::BTreeMap<PipeId, Connection>;

/// The sender behind [`Socket::connections`], which NNG passes to `connections_cb` as `arg`.
type ConnectionsSender = tokio::sync::watch::Sender<Connections>;

impl<Protocol> Socket<Protocol> {
    /// Returns a [`watch`](tokio::sync::watch) of the connections currently open on this socket,
    /// with their local and remote addresses.
    ///
    /// Covers both dialed (including reconnects) and accepted connections. Each one appears once
    /// it is added to the socket, before any messages are exchanged, and disappears once removed.
    /// A connection that opens and closes before the receiver looks may never be seen.
    ///
    /// Only connections established after the first call are tracked, so call this before
    /// dialing or listening. Later calls return another receiver for the same map. The watch
    /// closes when the socket is dropped.
    pub fn connections(&mut self) -> tokio::sync::watch::Receiver<Connections> {
        if let Some(tx) = &self.connections {
            return tx.subscribe();
        }

        let (tx, rx) = tokio::sync::watch::channel(Connections::new());

        let tx = self.connections.insert(Box::new(tx));
        let arg = &**tx as *const ConnectionsSender as *mut core::ffi::c_void;

        for event in [nng_sys::NNG_PIPE_EV_ADD_POST, nng_sys::NNG_PIPE_EV_REM_POST] {
            // SAFETY: socket is valid and not closed (socket is live until `self` drops), and
            //         `arg` outlives every callback (see `connections_cb`).
            let errno =
                unsafe { nng_sys::nng_pipe_notify(self.socket, event, Some(connections_cb), arg) };

            match errno {
                nng_sys::nng_err::NNG_OK => {}
                nng_sys::nng_err::NNG_ECLOSED => {
                    unreachable!("socket is still open since we have a reference to it");
                }
                _ => {
                    unreachable!(
                        "nng_pipe_notify documentation claims err \"{errno}\" is never returned"
                    );
                }
            }
        }

        rx
    }
}

/// Called by NNG on `NNG_PIPE_EV_ADD_POST` and `NNG_PIPE_EV_REM_POST` for each pipe of a socket
/// with [`Socket::connections`] enabled. NNG never reports ADD_POST after REM_POST for a pipe.
///
/// # Safety
///
/// `arg` must point to a live `watch::Sender<Connections>`. The `Socket` owns it and frees it
/// only after nng_socket_close returns, which waits until every pipe has been removed, and pipes
/// are removed only after their REM_POST callback has finished.
unsafe extern "C" fn connections_cb(
    pipe: nng_sys::nng_pipe,
    event: nng_sys::nng_pipe_ev,
    arg: *mut core::ffi::c_void,
) {
    // NOTE: NNG holds a global lock during this callback, so it must not block or touch the
    // socket. Updating a watch doesn't block or need a runtime.
    let result = std::panic::catch_unwind(|| {
        // SAFETY: see the function's safety comment.
        let tx = unsafe { &*(arg as *const ConnectionsSender) };
        // SAFETY: nng_pipe_id only reads the ID from the handle.
        let id = PipeId(unsafe { nng_sys::nng_pipe_id(pipe) } as u32);

        match event {
            nng_sys::NNG_PIPE_EV_ADD_POST => {
                let connection = Connection {
                    local_addr: pipe_addr(pipe, PipeEnd::Local),
                    remote_addr: pipe_addr(pipe, PipeEnd::Remote),
                };
                tx.send_modify(|connections| {
                    connections.insert(id, connection);
                });
            }
            nng_sys::NNG_PIPE_EV_REM_POST => {
                tx.send_if_modified(|connections| connections.remove(&id).is_some());
            }
            _ => tracing::warn!(?event, "unexpected pipe event"),
        }
    });
    if result.is_err() {
        tracing::error!("panic while updating connections; update dropped");
    }
}

/// Parses a TCP/TLS URL into a [`SocketAddr`].
///
/// Accepts URLs with schemes: `tcp://`, `tcp4://`, `tcp6://`, `tls+tcp://`, `tls+tcp4://`, `tls+tcp6://`
fn parse_tcp_url(url_str: &str) -> io::Result<SocketAddr> {
    // Valid TCP schemes from NNG source (nng/src/sp/transport/tcp/tcp.c):
    //   tcp, tcp4, tcp6
    // Valid TLS schemes from NNG source (nng/src/sp/transport/tls/tls.c):
    //   tls+tcp, tls+tcp4, tls+tcp6
    let (scheme, addr_part) = url_str.split_once("://").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("malformed URL: {url_str}"),
        )
    })?;
    match scheme {
        "tcp" | "tcp4" | "tcp6" | "tls+tcp" | "tls+tcp4" | "tls+tcp6" => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected URL scheme in {url_str}"),
            ));
        }
    }

    addr_part.parse::<SocketAddr>().map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to parse socket address from {addr_part}: {e}"),
        )
    })
}

impl<Protocol> TcpListener<'_, Protocol> {
    /// Retrieve the local address that this TCP listener is listening on.
    ///
    /// Parses the URL from [`Listener::local_addr()`] into a [`SocketAddr`].
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        // NOTE(flxo): NNG API asymmetry: There is no `nng_listener_get_addr()` to get the sockaddr
        // directly, even though `nng_dialer_get_addr()` exists for dialers. We must go through the URL:
        //   1. nng_listener_get_url() -> nng_url pointer
        //   2. nng_url_sprintf() -> URL string
        //   3. Parse string to extract address (done by callers like TcpListener::local_addr)
        let url = self
            .0
            .local_addr()
            .expect("TCP supports LOCADDR and doesn't yield UNSPEC");

        parse_tcp_url(url.as_str())
    }
}

impl<Protocol> Socket<Protocol> {
    /// Adds a listener to this socket that accepts TCP connections on the specified address.
    ///
    /// This allows the socket to accept incoming connections from remote endpoints.
    /// Multiple listeners can be added to the same socket to listen on different
    /// addresses or interfaces.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The URL format is invalid
    /// - The address is already in use
    /// - Permission denied for the requested address
    /// - Out of memory conditions
    pub async fn listen_tcp(
        &self,
        addr: SocketAddr,
        options: &TcpOptions,
    ) -> io::Result<TcpListener<'_, Protocol>> {
        // the Display impl of SocketAddr prints 1.2.3.4:5 or [ab:cd:ef]:5,
        // which matches what NNG expects.
        let url = CString::new(format!("tcp://{addr}")).expect("no null bytes in addr");
        let listener = crate::protocols::add_listener_to_socket(self, &url, |listener| {
            let &TcpOptions {
                no_delay,
                keep_alive,
                pipe: PipeOptions {},
            } = options;

            // SAFETY: these options are valid for TCP listeners, and the listener is valid.
            unsafe {
                nng_sys::nng_listener_set_bool(
                    listener,
                    nng_sys::NNG_OPT_TCP_NODELAY as *const _ as *const c_char,
                    no_delay,
                );
                nng_sys::nng_listener_set_bool(
                    listener,
                    nng_sys::NNG_OPT_TCP_KEEPALIVE as *const _ as *const c_char,
                    keep_alive,
                );
            }
            Ok(())
        })
        .await?;

        Ok(TcpListener(Listener {
            socket: self,
            listener,
        }))
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub struct PipeOptions {}

#[allow(clippy::derivable_impls)]
impl Default for PipeOptions {
    fn default() -> Self {
        Self {}
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub struct TcpOptions {
    /// This option is used to disable (or enable) the use of Nagle's algorithm for TCP connections.
    ///
    /// When true (the default), messages are sent immediately by the underlying TCP stream without
    /// waiting to gather more data.
    ///
    /// When false, Nagle's algorithm is enabled, and the TCP stream may wait briefly in attempt to
    /// coalesce messages. Nagle's algorithm is useful on low-bandwidth connections to reduce
    /// overhead, but it comes at a cost to latency.
    ///
    /// When used on a dialer or a listener, the value affects how newly created connections will
    /// be configured.
    pub no_delay: bool,

    /// This option is used to enable the sending of keep-alive messages on the underlying TCP
    /// stream. This option is false by default.
    ///
    /// When enabled, if no messages are seen for a period of time, then a zero length TCP message
    /// is sent with the ACK flag set in an attempt to tickle some traffic from the peer. If none
    /// is still seen (after some platform-specific number of retries and timeouts), then the
    /// remote peer is presumed dead, and the connection is closed.
    ///
    /// When used on a dialer or a listener, the value affects how newly created connections will
    /// be configured.
    ///
    /// This option has two purposes. First, it can be used to detect dead peers on an otherwise
    /// quiescent network. Second, it can be used to keep connection table entries in NAT and other
    /// middleware from being expiring due to lack of activity.
    pub keep_alive: bool,

    pub pipe: PipeOptions,
}

impl Default for TcpOptions {
    fn default() -> Self {
        Self {
            no_delay: true,
            keep_alive: false,
            pipe: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddrV4;

    #[tokio::test]
    async fn listen_tcp_local_addr() {
        let socket = crate::protocols::reqrep0::Rep0::socket().unwrap();
        let listener = socket
            .listen_tcp(
                std::net::SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &TcpOptions::default(),
            )
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(addr.ip().is_ipv4() && addr.ip().is_loopback(), "{addr:?}");
        assert_ne!(addr.port(), 0);
    }

    /// Test `Listener::local_addr()` returns a properly formatted URL.
    /// This tests the base method that uses `nng_url_sprintf`.
    #[tokio::test]
    async fn listener_local_addr_url() {
        let socket = crate::protocols::reqrep0::Rep0::socket().unwrap();
        let tcp_listener = socket
            .listen_tcp(
                std::net::SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
                &TcpOptions::default(),
            )
            .await
            .unwrap();

        // Access Listener::local_addr() via Deref, which returns Url
        let url: Url = Listener::local_addr(&tcp_listener).unwrap();

        // Verify URL format
        assert!(
            url.as_str().starts_with("tcp://127.0.0.1:"),
            "Expected tcp://127.0.0.1:PORT, got: {url}"
        );

        // Verify port is non-zero (ephemeral port was assigned)
        let port_str = url.as_str().strip_prefix("tcp://127.0.0.1:").unwrap();
        let port: u16 = port_str.parse().expect("port should be numeric");
        assert_ne!(port, 0);

        // Verify String conversion works
        let url_string: String = url.into();
        assert!(url_string.starts_with("tcp://"));
    }

    #[test]
    fn test_from_nng_unspec() {
        let nng_addr = nng_sys::nng_sockaddr {
            s_family: nng_sys::nng_sockaddr_family::NNG_AF_UNSPEC as u16,
        };
        assert_eq!(Url::from_nng(&nng_addr, c"tcp"), None);
    }

    #[test]
    fn test_from_nng_inet() {
        let sockaddr_in = nng_sys::nng_sockaddr_in {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_INET as u16,
            sa_port: 8080u16.to_be(),
            sa_addr: 0x7F000001u32.to_be(),
        };

        let nng_addr = nng_sys::nng_sockaddr { s_in: sockaddr_in };
        let url = Url::from_nng(&nng_addr, c"tcp").unwrap();

        assert_eq!(url.as_str(), "tcp://127.0.0.1:8080");
    }

    #[test]
    fn test_from_nng_inet6() {
        // Test without scope_id
        let sockaddr_in6 = nng_sys::nng_sockaddr_in6 {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_INET6 as u16,
            sa_port: 9090u16.to_be(),
            sa_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            sa_scope: 0,
        };

        let nng_addr = nng_sys::nng_sockaddr {
            s_in6: sockaddr_in6,
        };
        let url = Url::from_nng(&nng_addr, c"tcp").unwrap();
        assert_eq!(url.as_str(), "tcp://[::1]:9090");

        // Test with scope_id (required for link-local addresses like fe80::)
        let sockaddr_in6_scoped = nng_sys::nng_sockaddr_in6 {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_INET6 as u16,
            sa_port: 9090u16.to_be(),
            sa_addr: [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], // fe80::1
            sa_scope: 42,
        };

        let nng_addr_scoped = nng_sys::nng_sockaddr {
            s_in6: sockaddr_in6_scoped,
        };
        let url_scoped = Url::from_nng(&nng_addr_scoped, c"tcp").unwrap();
        assert_eq!(url_scoped.as_str(), "tcp://[fe80::1%42]:9090");
    }

    #[test]
    fn test_from_nng_ipc() {
        let mut sockaddr_ipc = nng_sys::nng_sockaddr_ipc {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_IPC as u16,
            sa_path: [0; 128],
        };

        let path = c"/tmp/test.sock";
        let path_bytes = path.to_bytes_with_nul();
        for (i, &byte) in path_bytes.iter().enumerate() {
            sockaddr_ipc.sa_path[i] = byte as _;
        }

        let nng_addr = nng_sys::nng_sockaddr {
            s_ipc: sockaddr_ipc,
        };
        let url = Url::from_nng(&nng_addr, c"ipc").unwrap();

        assert_eq!(url.as_str(), "ipc:///tmp/test.sock");
    }

    #[test]
    fn test_from_nng_inproc() {
        let mut sockaddr_inproc = nng_sys::nng_sockaddr_inproc {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_INPROC as u16,
            sa_name: [0; 128],
        };

        let name = c"test-inproc";
        let name_bytes = name.to_bytes_with_nul();
        for (i, &byte) in name_bytes.iter().enumerate() {
            sockaddr_inproc.sa_name[i] = byte as _;
        }

        let nng_addr = nng_sys::nng_sockaddr {
            s_inproc: sockaddr_inproc,
        };
        let url = Url::from_nng(&nng_addr, c"inproc").unwrap();

        assert_eq!(url.as_str(), "inproc://test-inproc");
    }

    #[test]
    fn test_from_nng_abstract() {
        let mut sockaddr_abstract = nng_sys::nng_sockaddr_abstract {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_ABSTRACT as u16,
            sa_len: 10,
            sa_name: [0; 107],
        };

        let name_bytes = b"test\0sock\x01";
        sockaddr_abstract.sa_len = name_bytes.len() as u16;
        sockaddr_abstract.sa_name[..name_bytes.len()].copy_from_slice(name_bytes);

        let nng_addr = nng_sys::nng_sockaddr {
            s_abstract: sockaddr_abstract,
        };
        let url = Url::from_nng(&nng_addr, c"abstract").unwrap();

        assert_eq!(url.as_str(), "abstract://test%00sock%01");
    }

    #[test]
    fn test_from_nng_inet_edge_cases() {
        let sockaddr_in = nng_sys::nng_sockaddr_in {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_INET as u16,
            sa_port: 0u16.to_be(),
            sa_addr: 0x00000000u32.to_be(),
        };

        let nng_addr = nng_sys::nng_sockaddr { s_in: sockaddr_in };
        let url = Url::from_nng(&nng_addr, c"tcp").unwrap();

        assert_eq!(url.as_str(), "tcp://0.0.0.0:0");
    }

    #[test]
    fn test_from_nng_empty_ipc_path() {
        let mut sockaddr_ipc = nng_sys::nng_sockaddr_ipc {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_IPC as u16,
            sa_path: [0; 128],
        };
        sockaddr_ipc.sa_path[0] = 0;

        let nng_addr = nng_sys::nng_sockaddr {
            s_ipc: sockaddr_ipc,
        };
        let url = Url::from_nng(&nng_addr, c"ipc").unwrap();

        assert_eq!(url.as_str(), "ipc://");
    }

    #[test]
    fn test_from_nng_zero_length_abstract() {
        let sockaddr_abstract = nng_sys::nng_sockaddr_abstract {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_ABSTRACT as u16,
            sa_len: 0,
            sa_name: [0; 107],
        };

        let nng_addr = nng_sys::nng_sockaddr {
            s_abstract: sockaddr_abstract,
        };
        let url = Url::from_nng(&nng_addr, c"abstract").unwrap();

        assert_eq!(url.as_str(), "abstract://");
    }

    #[test]
    fn test_url_into_string() {
        let url = Url("tcp://127.0.0.1:8080".to_string());
        let s: String = url.into();
        assert_eq!(s, "tcp://127.0.0.1:8080");
    }

    #[test]
    fn test_url_from_for_string() {
        let url = Url("ipc:///tmp/test.sock".to_string());
        let s = String::from(url);
        assert_eq!(s, "ipc:///tmp/test.sock");
    }

    #[test]
    fn test_url_scheme() {
        assert_eq!(Url("tcp://127.0.0.1:8080".to_string()).scheme(), "tcp");
        assert_eq!(Url("tcp4://127.0.0.1:8080".to_string()).scheme(), "tcp4");
        assert_eq!(Url("tcp6://[::1]:8080".to_string()).scheme(), "tcp6");
        assert_eq!(
            Url("tls+tcp://127.0.0.1:8080".to_string()).scheme(),
            "tls+tcp"
        );
        assert_eq!(Url("ipc:///tmp/test.sock".to_string()).scheme(), "ipc");
        assert_eq!(Url("inproc://test".to_string()).scheme(), "inproc");
        assert_eq!(Url("abstract://test".to_string()).scheme(), "abstract");
    }

    #[tokio::test]
    async fn listen_tcp_local_addr_ipv6() {
        use std::net::SocketAddrV6;

        let socket = crate::protocols::reqrep0::Rep0::socket().unwrap();
        let listener = socket
            .listen_tcp(
                SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0)),
                &TcpOptions::default(),
            )
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(addr.ip().is_ipv6() && addr.ip().is_loopback());
        assert_ne!(addr.port(), 0);
    }

    /// Test abstract socket URL encoding for edge cases:
    /// - Space (0x20) should be percent-encoded
    /// - High bytes (0xFF) should be percent-encoded
    #[test]
    fn test_from_nng_abstract_edge_cases() {
        // Test with space character (0x20 is not ascii_graphic)
        let mut sockaddr = nng_sys::nng_sockaddr_abstract {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_ABSTRACT as u16,
            sa_len: 0,
            sa_name: [0; 107],
        };

        let name_with_space = b"hello world";
        sockaddr.sa_len = name_with_space.len() as u16;
        sockaddr.sa_name[..name_with_space.len()].copy_from_slice(name_with_space);

        let nng_addr = nng_sys::nng_sockaddr {
            s_abstract: sockaddr,
        };
        let url = Url::from_nng(&nng_addr, c"abstract").unwrap();
        assert_eq!(url.as_str(), "abstract://hello%20world");

        // Test with high byte (0xFF)
        let mut sockaddr = nng_sys::nng_sockaddr_abstract {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_ABSTRACT as u16,
            sa_len: 0,
            sa_name: [0; 107],
        };

        let name_with_high_byte = b"test\xff";
        sockaddr.sa_len = name_with_high_byte.len() as u16;
        sockaddr.sa_name[..name_with_high_byte.len()].copy_from_slice(name_with_high_byte);

        let nng_addr = nng_sys::nng_sockaddr {
            s_abstract: sockaddr,
        };
        let url = Url::from_nng(&nng_addr, c"abstract").unwrap();
        assert_eq!(url.as_str(), "abstract://test%ff");
    }

    /// Test that parse_tcp_url handles all valid TCP/TLS schemes.
    #[test]
    fn test_parse_tcp_url_schemes() {
        use super::parse_tcp_url;

        // Test all TCP schemes (from nng/src/sp/transport/tcp/tcp.c)
        assert_eq!(
            parse_tcp_url("tcp://127.0.0.1:8080").unwrap(),
            "127.0.0.1:8080".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            parse_tcp_url("tcp4://192.168.1.1:9090").unwrap(),
            "192.168.1.1:9090".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            parse_tcp_url("tcp6://[::1]:8080").unwrap(),
            "[::1]:8080".parse::<SocketAddr>().unwrap()
        );

        // Test all TLS schemes (from nng/src/sp/transport/tls/tls.c)
        assert_eq!(
            parse_tcp_url("tls+tcp://10.0.0.1:443").unwrap(),
            "10.0.0.1:443".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            parse_tcp_url("tls+tcp4://10.0.0.2:443").unwrap(),
            "10.0.0.2:443".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            parse_tcp_url("tls+tcp6://[fe80::1]:443").unwrap(),
            "[fe80::1]:443".parse::<SocketAddr>().unwrap()
        );
    }

    /// Test that parse_tcp_url preserves IPv6 scope_id.
    #[test]
    fn test_parse_tcp_url_scope_id_preservation() {
        use super::parse_tcp_url;

        // Numeric scope ID should be preserved
        let addr = parse_tcp_url("tcp://[fe80::1%42]:9090").unwrap();
        assert_eq!(addr.to_string(), "[fe80::1%42]:9090");
        if let SocketAddr::V6(v6) = addr {
            assert_eq!(v6.scope_id(), 42);
        } else {
            panic!("Expected IPv6 address");
        }

        // Note: Interface name scope IDs (like %eth0) are not supported by SocketAddr::from_str
        // and will result in a parse error, which is correct behavior
        assert!(parse_tcp_url("tcp6://[fe80::1%eth0]:8080").is_err());
    }

    /// Test that parse_tcp_url returns appropriate errors.
    #[test]
    fn test_parse_tcp_url_errors() {
        use super::parse_tcp_url;

        // Unknown scheme
        let err = parse_tcp_url("ipc:///tmp/socket").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        // Malformed address
        let err = parse_tcp_url("tcp://not-an-address").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        // Empty address
        let err = parse_tcp_url("tcp://").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
