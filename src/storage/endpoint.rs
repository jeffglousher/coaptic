//! [`Endpoint`]: portable UDP peer identity, sidecar to datagram slots.

use core::fmt;

/// UDP peer address stored beside a datagram slot, not in its byte buffer.
///
/// A datagram slot holds CoAP message bytes (the UDP payload). [`Endpoint`] is
/// sidecar metadata for that slot. See `design.md` and `knowledge/memory.md`.
///
/// This type does not use [`std::net`] on the default `no_std` path. The `std`
/// feature adds conversions. IPv6 flowinfo and scope id are not stored; they
/// are zero when converting to [`std::net::SocketAddrV6`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Endpoint {
    /// IPv4 address and UDP port (network-order octets).
    V4([u8; 4], u16),
    /// IPv6 address and UDP port (network-order octets).
    V6([u8; 16], u16),
}

impl Endpoint {
    /// IPv4 endpoint.
    #[must_use]
    pub const fn v4(addr: [u8; 4], port: u16) -> Self {
        Self::V4(addr, port)
    }

    /// IPv6 endpoint.
    #[must_use]
    pub const fn v6(addr: [u8; 16], port: u16) -> Self {
        Self::V6(addr, port)
    }

    /// UDP port.
    #[must_use]
    pub const fn port(self) -> u16 {
        match self {
            Self::V4(_, port) | Self::V6(_, port) => port,
        }
    }

    /// IPv4 octets and port, if this is [`Self::V4`].
    #[must_use]
    pub const fn as_ipv4(self) -> Option<([u8; 4], u16)> {
        match self {
            Self::V4(addr, port) => Some((addr, port)),
            Self::V6(_, _) => None,
        }
    }

    /// IPv6 octets and port, if this is [`Self::V6`].
    #[must_use]
    pub const fn as_ipv6(self) -> Option<([u8; 16], u16)> {
        match self {
            Self::V6(addr, port) => Some((addr, port)),
            Self::V4(_, _) => None,
        }
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::V4(addr, port) => {
                write!(f, "{}.{}.{}.{}:{port}", addr[0], addr[1], addr[2], addr[3])
            }
            Self::V6(addr, port) => {
                write!(f, "[")?;
                for (i, chunk) in addr.chunks_exact(2).enumerate() {
                    if i > 0 {
                        write!(f, ":")?;
                    }
                    let group = u16::from_be_bytes([chunk[0], chunk[1]]);
                    write!(f, "{group:x}")?;
                }
                write!(f, "]:{port}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl From<std::net::SocketAddrV4> for Endpoint {
    fn from(addr: std::net::SocketAddrV4) -> Self {
        Self::V4(addr.ip().octets(), addr.port())
    }
}

#[cfg(feature = "std")]
impl From<std::net::SocketAddrV6> for Endpoint {
    fn from(addr: std::net::SocketAddrV6) -> Self {
        Self::V6(addr.ip().octets(), addr.port())
    }
}

#[cfg(feature = "std")]
impl From<std::net::SocketAddr> for Endpoint {
    fn from(addr: std::net::SocketAddr) -> Self {
        match addr {
            std::net::SocketAddr::V4(v4) => Self::from(v4),
            std::net::SocketAddr::V6(v6) => Self::from(v6),
        }
    }
}

#[cfg(feature = "std")]
impl From<Endpoint> for std::net::SocketAddr {
    fn from(endpoint: Endpoint) -> Self {
        match endpoint {
            Endpoint::V4(octets, port) => {
                std::net::SocketAddr::V4(std::net::SocketAddrV4::new(octets.into(), port))
            }
            Endpoint::V6(octets, port) => {
                std::net::SocketAddr::V6(std::net::SocketAddrV6::new(octets.into(), port, 0, 0))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Endpoint;

    #[test]
    fn equality_same_v4() {
        let a = Endpoint::v4([192, 0, 2, 1], 5683);
        let b = Endpoint::V4([192, 0, 2, 1], 5683);
        assert_eq!(a, b);
        assert_eq!(a.port(), 5683);
        assert_eq!(a.as_ipv4(), Some(([192, 0, 2, 1], 5683)));
        assert_eq!(a.as_ipv6(), None);
    }

    #[test]
    fn inequality_port_and_family() {
        let v4 = Endpoint::v4([192, 0, 2, 1], 5683);
        assert_ne!(v4, Endpoint::v4([192, 0, 2, 1], 5684));
        assert_ne!(v4, Endpoint::v4([192, 0, 2, 2], 5683));
        let v6 = Endpoint::v6([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], 5683);
        assert_ne!(v4, v6);
        assert_eq!(v6.as_ipv6().map(|(_, p)| p), Some(5683));
        assert_eq!(v6.as_ipv4(), None);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn display_v4_and_v6() {
        extern crate alloc;
        use alloc::string::ToString;

        assert_eq!(
            Endpoint::v4([192, 0, 2, 1], 5683).to_string(),
            "192.0.2.1:5683"
        );
        let loopback = Endpoint::v6([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], 5683);
        assert_eq!(loopback.to_string(), "[0:0:0:0:0:0:0:1]:5683");
    }

    #[cfg(feature = "std")]
    #[test]
    fn std_socket_addr_roundtrip() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

        let v4 = Endpoint::v4([192, 0, 2, 10], 5683);
        let sock: SocketAddr = v4.into();
        assert_eq!(sock, SocketAddr::from((Ipv4Addr::new(192, 0, 2, 10), 5683)));
        assert_eq!(Endpoint::from(sock), v4);
        assert_eq!(
            Endpoint::from(SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 5683)),
            v4
        );

        let v6 = Endpoint::v6(
            [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            5684,
        );
        let sock6: SocketAddr = v6.into();
        assert_eq!(
            sock6,
            SocketAddr::new(
                IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
                5684
            )
        );
        assert_eq!(Endpoint::from(sock6), v6);

        let scoped = SocketAddrV6::new(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1), 5683, 7, 2);
        let stripped = Endpoint::from(scoped);
        assert_eq!(
            stripped,
            Endpoint::v6([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], 5683)
        );
        let back: SocketAddr = stripped.into();
        match back {
            SocketAddr::V6(v) => {
                assert_eq!(v.flowinfo(), 0);
                assert_eq!(v.scope_id(), 0);
            }
            SocketAddr::V4(_) => panic!("expected v6"),
        }
    }
}
