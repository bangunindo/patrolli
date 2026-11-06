// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! TCP-MD5 signatures (RFC 2385) for passive BGP sessions, on Linux.
//!
//! A key is set per peer address on the listening socket; the connections it
//! accepts inherit it, and the kernel drops the segments of a peer whose key
//! does not match (a peer with no key set connects without signatures).
//! An IPv4 peer of an IPv6 socket is given as a v4-mapped address, which the
//! kernel stores as an IPv4 key; an IPv6 peer cannot have a key on an IPv4
//! socket (configuration refuses it).

use socket2::{SockAddr, Socket};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::os::fd::AsRawFd;

/// `struct tcp_md5sig` of `linux/tcp.h`; libc defines the option but not the
/// struct on Linux.
#[repr(C)]
struct TcpMd5Sig {
    tcpm_addr: libc::sockaddr_storage,
    tcpm_flags: u8,
    tcpm_prefixlen: u8,
    tcpm_keylen: u16,
    tcpm_ifindex: libc::c_int,
    tcpm_key: [u8; libc::TCP_MD5SIG_MAXKEYLEN],
}

/// Sets `key` for `peer` on `socket`, a bound socket: a listener's keys are
/// inherited by the connections it accepts.
pub fn set_key(socket: &Socket, peer: IpAddr, key: &[u8]) -> io::Result<()> {
    if key.is_empty() || key.len() > libc::TCP_MD5SIG_MAXKEYLEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "tcp md5 key must be 1 to 80 bytes",
        ));
    }
    let v6_socket = socket.local_addr()?.is_ipv6();
    let peer = match (peer.to_canonical(), v6_socket) {
        (IpAddr::V4(v4), true) => IpAddr::V6(v4.to_ipv6_mapped()),
        (IpAddr::V6(_), false) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an ipv6 peer needs an ipv6 socket",
            ));
        }
        (ip, _) => ip,
    };
    let mut storage = SockAddr::from(SocketAddr::new(peer, 0)).as_storage();
    // SAFETY: `SockAddrStorage` is `repr(transparent)` over the platform's
    // `sockaddr_storage`, which is what we read it as.
    let addr = *unsafe { storage.view_as::<libc::sockaddr_storage>() };
    let mut sig = TcpMd5Sig {
        tcpm_addr: addr,
        tcpm_flags: 0,
        tcpm_prefixlen: 0,
        tcpm_keylen: key.len() as u16,
        tcpm_ifindex: 0,
        tcpm_key: [0; libc::TCP_MD5SIG_MAXKEYLEN],
    };
    sig.tcpm_key[..key.len()].copy_from_slice(key);
    // SAFETY: `sig` is a fully initialised `struct tcp_md5sig` that outlives
    // the call, and the length passed is its size.
    let rc = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::IPPROTO_TCP,
            libc::TCP_MD5SIG,
            (&raw const sig).cast(),
            size_of::<TcpMd5Sig>() as libc::socklen_t,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use socket2::{Domain, Protocol, Type};
    use std::time::Duration;

    fn listener(addr: &str) -> Socket {
        let addr: SocketAddr = addr.parse().unwrap();
        let s = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP)).unwrap();
        s.bind(&addr.into()).unwrap();
        s.listen(8).unwrap();
        s
    }

    fn client(key: Option<&[u8]>, to: SocketAddr) -> io::Result<Socket> {
        let s = Socket::new(Domain::for_address(to), Type::STREAM, Some(Protocol::TCP))?;
        if let Some(key) = key {
            // a client socket takes the key of the address it connects to
            s.bind(&SocketAddr::new(to.ip(), 0).into())?;
            set_key(&s, to.ip(), key)?;
        }
        s.connect_timeout(&to.into(), Duration::from_millis(500))?;
        Ok(s)
    }

    #[test]
    fn a_peer_with_the_key_connects_and_one_without_does_not() {
        let server = listener("127.0.0.1:0");
        set_key(&server, "127.0.0.1".parse().unwrap(), b"s3cret").unwrap();
        let to = server.local_addr().unwrap().as_socket().unwrap();
        client(Some(b"s3cret"), to).expect("signed connection");
        assert!(client(None, to).is_err(), "an unsigned syn is dropped");
        assert!(
            client(Some(b"wrong"), to).is_err(),
            "a wrong key is dropped"
        );
    }

    #[test]
    fn keys_follow_the_socket_family() {
        let v4 = listener("127.0.0.1:0");
        assert!(set_key(&v4, "::1".parse().unwrap(), b"k").is_err());
        assert!(set_key(&v4, "127.0.0.1".parse().unwrap(), &[]).is_err());
        assert!(set_key(&v4, "127.0.0.1".parse().unwrap(), &[0; 81]).is_err());
        if let Ok(v6) = std::panic::catch_unwind(|| listener("[::1]:0")) {
            set_key(&v6, "192.0.2.1".parse().unwrap(), b"k").expect("v4-mapped");
            set_key(&v6, "2001:db8::1".parse().unwrap(), b"k").unwrap();
        }
    }
}
