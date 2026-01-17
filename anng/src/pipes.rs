use crate::Socket;
use core::ffi::CStr;
use core::ffi::c_char;
use core::ffi::c_int;
use core::fmt;
use core::mem::MaybeUninit;
use core::net::Ipv4Addr;
use core::net::Ipv6Addr;
use core::net::SocketAddr;
use core::net::SocketAddrV4;
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
    /// Retrieve the local address that this listener is listening on.
    pub fn local_addr(&self) -> io::Result<Addr> {
        // SAFETY: these options are valid for listeners, the listener is valid, we use the
        // appropriate typed pointers to each type.
        let addr = unsafe {
            let mut addr = MaybeUninit::<nng_sys::nng_sockaddr>::uninit();
            let errno = nng_sys::nng_listener_get_addr(
                self.listener,
                nng_sys::NNG_OPT_LOCADDR as *const _ as *const c_char,
                addr.as_mut_ptr(),
            );
            let errno = u32::try_from(errno).expect("errno is never negative");
            if errno == nng_sys::NNG_ENOTSUP {
                return Err(io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    "listener transport does not support NNG_OPT_LOCADDR",
                ));
            }
            assert_eq!(
                errno, 0,
                "all listed error conditions of nng_listener_get_addr are impossible given arguments"
            );
            addr.assume_init()
        };

        Ok(Addr::from_nng(addr).expect("LOCADDR on listener is never UNSPEC if errno == 0"))
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

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Addr {
    /// Address for intraprocess communication.
    Inproc {
        /// This field holds an arbitrary C string, which is the name of the address.
        ///
        /// The string must be NUL terminated, but no other restrictions exist.
        name: CString,
    },
    /// Address for interprocess communication.
    Ipc {
        /// This field holds the C string corresponding to path name where the IPC socket is
        /// located.
        ///
        /// For systems using UNIX domain sockets, this will be a path name in the file system,
        /// where the UNIX domain socket is located. For Windows systems, this is the path name of
        /// the Named Pipe, without the leading \\.pipe\ portion, which will be automatically
        /// added.
        path: CString,
    },
    /// Address for TCP/IP (v4) communication.
    Inet(SocketAddrV4),
    /// Address for TCP/IP (v6) communication.
    Inet6(SocketAddrV6),
    /// Address for ZeroTier transport.
    #[non_exhaustive]
    Zt,
    /// Address for an abstract UNIX domain socket.
    ///
    /// Abstract sockets are only supported on Linux at present. These sockets have a name that is
    /// simply an array of bytes, with no special meaning. Abstract sockets also have no presence
    /// in the file system, do not honor any permissions, and are automatically cleaned up by the
    /// operating system when no longer in use.
    Abstract {
        /// This field holds the name of the abstract socket.
        ///
        /// The bytes of name can have any value, including zero.
        ///
        /// The name does not include the leading NUL byte used on Linux to discriminate between
        /// abstract and path name sockets.
        name: Box<[u8]>,
    },
}

impl Addr {
    /// Returns `None` if address is not initialized (ie, family is `NNG_AF_UNSPEC`).
    // NOTE(jon): not From, since that'd make nng_sys be part of the public API.
    pub(crate) fn from_nng(addr: nng_sys::nng_sockaddr) -> Option<Self> {
        // SAFETY: first field of every union member is a u16, so s_family is always okay to access
        let s_family = u32::from(unsafe { addr.s_family });
        // SAFETY: cast is valid since s_family is always one of the listed variants
        match unsafe { core::mem::transmute::<u32, nng_sys::nng_sockaddr_family>(s_family) } {
            nng_sys::nng_sockaddr_family::NNG_AF_UNSPEC => None,
            nng_sys::nng_sockaddr_family::NNG_AF_INET => {
                // SAFETY: we've checked the family
                let addr = unsafe { &addr.s_in };
                let sa_addr = u32::from_be(addr.sa_addr);
                let sa_port = u16::from_be(addr.sa_port);
                let ip = Ipv4Addr::from_bits(sa_addr);
                Some(Self::Inet(SocketAddrV4::new(ip, sa_port)))
            }
            nng_sys::nng_sockaddr_family::NNG_AF_INET6 => {
                // SAFETY: we've checked the family
                let addr = unsafe { &addr.s_in6 };
                let sa_port = u16::from_be(addr.sa_port);
                let ip = Ipv6Addr::from_bits(u128::from_be_bytes(addr.sa_addr));
                Some(Self::Inet6(SocketAddrV6::new(
                    ip,
                    sa_port,
                    0,
                    addr.sa_scope,
                )))
            }
            nng_sys::nng_sockaddr_family::NNG_AF_IPC => {
                // SAFETY: we've checked the family
                let addr = unsafe { &addr.s_ipc };
                // SAFETY: sa_path is guaranteed to be a C-style string, and we stop using this
                // reference before we drop the `nng_sockaddr`.
                let path = unsafe { CStr::from_ptr(addr.sa_path.as_ptr()) };
                Some(Self::Ipc {
                    path: path.to_owned(),
                })
            }
            nng_sys::nng_sockaddr_family::NNG_AF_ABSTRACT => {
                // SAFETY: we've checked the family
                let addr = unsafe { &addr.s_abstract };
                let name = &addr.sa_name[..usize::from(addr.sa_len)];
                Some(Self::Abstract {
                    name: Vec::from(name).into_boxed_slice(),
                })
            }
            nng_sys::nng_sockaddr_family::NNG_AF_INPROC => {
                // SAFETY: we've checked the family
                let addr = unsafe { &addr.s_inproc };
                // SAFETY: sa_name is guaranteed to be a C-style string, and we stop using this
                // reference before we drop the `nng_sockaddr`.
                let name = unsafe { CStr::from_ptr(addr.sa_name.as_ptr()) };
                Some(Self::Inproc {
                    name: name.to_owned(),
                })
            }
            _family => {
                unimplemented!("support for family {s_family} has not yet been added")
            }
        }
    }
}

impl fmt::Display for Addr {
    /// Format trait for an empty format, `{}`.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Addr::Inproc { name } => {
                // inproc addresses are guaranteed to be a text string according
                // to https://nng.nanomsg.org/man/v1.10.0/nng_inproc.7.html
                write!(f, "inproc://{}", name.to_string_lossy())
            }
            Addr::Ipc { path } => {
                // abstract addresses are handled in the `Addr::Abstract` variant.
                // see the `Socket Address` section of
                // https://nng.nanomsg.org/man/v1.10.0/nng_ipc.7.html
                // so these are always simple paths
                write!(f, "ipc://{}", path.to_string_lossy())
            }
            Addr::Inet(addr) => write!(f, "tcp://{addr}"),
            Addr::Inet6(addr) => write!(f, "tcp://{addr}"),
            Addr::Abstract { name } => {
                write!(f, "abstract://")?;
                for b in name.as_ref() {
                    if b.is_ascii_graphic() {
                        write!(f, "{}", *b as char)?;
                    } else {
                        // conform to https://nng.nanomsg.org/man/v1.10.0/nng_ipc.7.html
                        // and prefix the hex value with `%`
                        write!(f, "%{:02x}", b)?;
                    }
                }
                Ok(())
            }
            // format in URI scheme for consistency with the other variants
            Addr::Zt => write!(f, "zt://<todo>"),
        }
    }
}

impl<Protocol> TcpListener<'_, Protocol> {
    /// Retrieve the local address that this TCP listener is listening on.
    // NOTE(jon): this is entirely just a convenience wrapper to give `SocketAddr` (and to validate
    // `BOUND_PORT`).
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        let addr = self
            .0
            .local_addr()
            .expect("TCP supports LOCADDR and doesn't yield UNSPEC");

        // SAFETY: this option is valid for TCP listeners, the listener is valid, we use the
        // appropriate typed pointers to the type.
        let port = unsafe {
            let mut port = MaybeUninit::<c_int>::uninit();
            let errno = nng_sys::nng_listener_get_int(
                self.0.listener,
                nng_sys::NNG_OPT_TCP_BOUND_PORT as *const _ as *const c_char,
                port.as_mut_ptr(),
            );
            assert_eq!(
                errno, 0,
                "all listed error conditions of nng_listener_get_int are impossible given arguments"
            );
            port.assume_init()
        };

        match addr {
            Addr::Inet(v4) => {
                assert_eq!(port, i32::from(v4.port()));
                Ok(SocketAddr::V4(v4))
            }
            Addr::Inet6(v6) => {
                assert_eq!(port, i32::from(v6.port()));
                Ok(SocketAddr::V6(v6))
            }
            addr => {
                unreachable!(
                    "tcp:// listeners should always be associated with INET family, not {addr}",
                )
            }
        }
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
        let listener =
            crate::protocols::add_listener_to_socket(self.inner.socket, &url, |listener| {
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
    use rstest::rstest;
    use std::ffi::CString;
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};

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

    #[test]
    fn test_from_nng_unspec() {
        let nng_addr = nng_sys::nng_sockaddr {
            s_family: nng_sys::nng_sockaddr_family::NNG_AF_UNSPEC as u16,
        };
        assert_eq!(Addr::from_nng(nng_addr), None);
    }

    #[test]
    fn test_from_nng_inet() {
        let sockaddr_in = nng_sys::nng_sockaddr_in {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_INET as u16,
            sa_port: 8080u16.to_be(),       // Network byte order
            sa_addr: 0x7F000001u32.to_be(), // 127.0.0.1 in network byte order
        };

        let nng_addr = nng_sys::nng_sockaddr { s_in: sockaddr_in };

        let addr = Addr::from_nng(nng_addr).unwrap();
        let Addr::Inet(v4) = addr else {
            panic!("Expected Inet address, got: {addr:?}");
        };

        assert_eq!(v4.ip(), &Ipv4Addr::new(127, 0, 0, 1));
        assert_eq!(v4.port(), 8080);
    }

    #[test]
    fn test_from_nng_inet6() {
        let sockaddr_in6 = nng_sys::nng_sockaddr_in6 {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_INET6 as u16,
            sa_port: 9090u16.to_be(),
            sa_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], // ::1 in bytes
            sa_scope: 42,
        };

        let nng_addr = nng_sys::nng_sockaddr {
            s_in6: sockaddr_in6,
        };

        let addr = Addr::from_nng(nng_addr).unwrap();
        let Addr::Inet6(v6) = addr else {
            panic!("Expected Inet6 address, got: {addr:?}");
        };

        assert_eq!(v6.ip(), &Ipv6Addr::LOCALHOST);
        assert_eq!(v6.port(), 9090);
        assert_eq!(v6.scope_id(), 42);
    }

    #[test]
    fn test_from_nng_ipc() {
        let mut sockaddr_ipc = nng_sys::nng_sockaddr_ipc {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_IPC as u16,
            sa_path: [0; 128], // NNG_MAXADDRLEN is typically 128
        };

        // Set a path - must be null-terminated C string
        let path = c"/tmp/test.sock";
        let path_bytes = path.to_bytes_with_nul();
        for (i, &byte) in path_bytes.iter().enumerate() {
            sockaddr_ipc.sa_path[i] = byte as _;
        }

        let nng_addr = nng_sys::nng_sockaddr {
            s_ipc: sockaddr_ipc,
        };

        let addr = Addr::from_nng(nng_addr).unwrap();
        let Addr::Ipc { path: path_cstring } = addr else {
            panic!("Expected IPC address, got: {addr:?}");
        };

        assert_eq!(path_cstring.to_str().unwrap(), "/tmp/test.sock");
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

        let addr = Addr::from_nng(nng_addr).unwrap();
        let Addr::Inproc { name: name_cstring } = addr else {
            panic!("Expected Inproc address, got: {addr:?}");
        };

        assert_eq!(name_cstring.to_str().unwrap(), "test-inproc");
    }

    #[test]
    fn test_from_nng_abstract() {
        let mut sockaddr_abstract = nng_sys::nng_sockaddr_abstract {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_ABSTRACT as u16,
            sa_len: 10,
            sa_name: [0; 107], // Based on NNG source
        };

        // Abstract sockets can have arbitrary bytes, including nulls
        let name_bytes = b"test\0sock\x01";
        sockaddr_abstract.sa_len = name_bytes.len() as u16;
        sockaddr_abstract.sa_name[..name_bytes.len()].copy_from_slice(name_bytes);

        let nng_addr = nng_sys::nng_sockaddr {
            s_abstract: sockaddr_abstract,
        };

        let addr = Addr::from_nng(nng_addr).unwrap();
        let Addr::Abstract { name: name_box } = addr else {
            panic!("Expected Abstract address, got: {addr:?}");
        };

        assert_eq!(&name_box[..], b"test\0sock\x01");
    }

    #[test]
    fn test_from_nng_inet_edge_cases() {
        // Test with port 0
        let sockaddr_in = nng_sys::nng_sockaddr_in {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_INET as u16,
            sa_port: 0u16.to_be(),
            sa_addr: 0x00000000u32.to_be(), // 0.0.0.0
        };

        let nng_addr = nng_sys::nng_sockaddr { s_in: sockaddr_in };

        let addr = Addr::from_nng(nng_addr).unwrap();
        let Addr::Inet(v4) = addr else {
            panic!("Expected Inet address, got: {addr:?}");
        };

        assert_eq!(v4.ip(), &Ipv4Addr::new(0, 0, 0, 0));
        assert_eq!(v4.port(), 0);
    }

    #[test]
    fn test_from_nng_empty_ipc_path() {
        let mut sockaddr_ipc = nng_sys::nng_sockaddr_ipc {
            sa_family: nng_sys::nng_sockaddr_family::NNG_AF_IPC as u16,
            sa_path: [0; 128],
        };

        // Just null terminator
        sockaddr_ipc.sa_path[0] = 0;

        let nng_addr = nng_sys::nng_sockaddr {
            s_ipc: sockaddr_ipc,
        };

        let addr = Addr::from_nng(nng_addr).unwrap();
        let Addr::Ipc { path: path_cstring } = addr else {
            panic!("Expected IPC address, got: {addr:?}");
        };

        assert_eq!(path_cstring.to_str().unwrap(), "");
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

        let addr = Addr::from_nng(nng_addr).unwrap();
        let Addr::Abstract { name: name_box } = addr else {
            panic!("Expected Abstract address, got: {addr:?}");
        };

        assert_eq!(name_box.len(), 0);
    }

    #[rstest]
    #[case(
        Addr::Inet(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
        "tcp://127.0.0.1:8080",
        "inet localhost"
    )]
    #[case(
        Addr::Inet(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 8080)),
        "tcp://0.0.0.0:8080",
        "inet any"
    )]
    #[case(
        Addr::Inet6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 8080, 0, 0)),
        "tcp://[::1]:8080",
        "inet6 localhost"
    )]
    #[case(
        Addr::Inet6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 8080, 0, 0)),
        "tcp://[::]:8080",
        "inet6 any"
    )]
    #[case(Addr::Inproc { name: CString::new("myapp").unwrap() }, "inproc://myapp", "inproc printable")]
    #[case(Addr::Inproc { name: CString::new("my-app_123").unwrap() }, "inproc://my-app_123", "inproc with special chars")]
    #[case(Addr::Ipc { path: CString::new("/tmp/mysocket").unwrap() }, "ipc:///tmp/mysocket", "ipc printable")]
    #[case(Addr::Abstract { name: b"myapp".to_vec().into() }, "abstract://myapp", "abstract printable")]
    #[case(Addr::Abstract { name: vec![0x00, b'a', b'p', b'p'].into() }, "abstract://%00app", "abstract with null prefix")]
    #[case(Addr::Abstract { name: vec![0x00, b'a', b'p', b'p', 0xFF, b'!'].into() }, "abstract://%00app%ff!", "abstract mixed content")]
    #[case(Addr::Abstract { name: vec![0x00, 0x01, 0xFF, 0xFE].into() }, "abstract://%00%01%ff%fe", "abstract all non printable")]
    #[case(Addr::Abstract { name: vec![].into() }, "abstract://", "abstract empty")]
    #[case(Addr::Abstract { name: b"Hello-World_123!".to_vec().into() }, "abstract://Hello-World_123!", "hex name helper all printable")]
    #[case(Addr::Abstract { name: b"hello world".to_vec().into() }, "abstract://hello%20world", "hex name helper whitespace encoded")]
    #[case(Addr::Abstract { name: vec![b'a', b'\t', b'b', b'\n', b'c'].into() }, "abstract://a%09b%0ac", "hex name helper tab and newline")]
    fn test_addr_to_string(#[case] addr: Addr, #[case] expected: &str, #[case] description: &str) {
        let addr = addr.to_string();
        assert_eq!(addr, expected, "{description}");
    }
}
