//! where the tarpit listens: the socket systemd passed in, else a dual-stack socket

use crate::legacy::Family;
use socket2::{Domain, Protocol, SockRef, Socket, Type};
use std::{
    env, fs, io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener},
};

/// the smallest receive buffer: less kernel memory per connection, and it slows the sender.
/// on the listener too, so on linux a queued connection holds about 1 KB instead of 126 KB
pub const RECV_BUFFER: usize = 1;

/// a client that never reads pins whatever is queued for it. without this that reaches about 47 KB
/// per client, and 4096 of them get the container killed at its 128 MB limit. linux doubles this to
/// its minimum, 4608; macos takes it literally, and much less would cut every banner line short
pub const SEND_BUFFER: usize = 2304;

/// the listener systemd passed in (socket activation), else a socket on `port`: dual-stack unless asked for one family
pub fn listen(port: u16, family: Option<Family>) -> io::Result<TcpListener> {
    if let Some(l) = inherited() {
        // a closed or non-socket fd 3 fails here instead of as an endless accept error loop
        l.local_addr()?;
        let s = SockRef::from(&l);
        s.set_recv_buffer_size(RECV_BUFFER)?;
        s.set_send_buffer_size(SEND_BUFFER)?;
        return Ok(l);
    }
    let v4 = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
    let v6 = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
    let bind = |domain: Domain, addr: SocketAddr| -> io::Result<TcpListener> {
        let s = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
        s.set_reuse_address(true)?;
        if domain == Domain::IPV6 {
            s.set_only_v6(family == Some(Family::V6))?;
        }
        s.bind(&addr.into())?;
        s.set_recv_buffer_size(RECV_BUFFER)?;
        s.set_send_buffer_size(SEND_BUFFER)?;
        s.listen(1024)?;
        Ok(s.into())
    };
    match family {
        Some(Family::V4) => bind(Domain::IPV4, v4),
        Some(Family::V6) => bind(Domain::IPV6, v6),
        None => bind(Domain::IPV6, v6).or_else(|e| {
            // ipv6 off, or the port taken on v6 only. net.ipv6.bindv6only is not a cause: set_only_v6 overrides it
            log!("warning: no ipv6 listener on port {port} ({e}), trying ipv4 only");
            bind(Domain::IPV4, v4)
        }),
    }
}

#[allow(unsafe_code)] // from_raw_fd on the fd systemd hands us; checked via LISTEN_PID/LISTEN_FDS
fn inherited() -> Option<TcpListener> {
    use std::os::fd::FromRawFd;
    let pid: u32 = env::var("LISTEN_PID").ok()?.parse().ok()?;
    let fds: u32 = env::var("LISTEN_FDS").ok()?.parse().ok()?;
    // SAFETY: with LISTEN_PID == our pid, fd 3 is the first socket systemd passed and nobody else owns it
    (pid == std::process::id() && fds >= 1).then(|| unsafe { TcpListener::from_raw_fd(3) })
}

/// docker HEALTHCHECK: `port` in LISTEN state per /proc. no connection, so it never shows up as a visitor
pub fn is_listening(port: u16) -> bool {
    let local = format!(":{port:04X}");
    ["/proc/net/tcp", "/proc/net/tcp6"].iter().any(|path| {
        fs::read_to_string(path).is_ok_and(|table| {
            table.lines().skip(1).any(|line| {
                // sl local_address rem_address st ...
                let f: Vec<_> = line.split_whitespace().collect();
                f.len() > 3 && f[1].ends_with(&local) && f[3] == "0A"
            })
        })
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::net::TcpStream;

    #[test]
    fn accepted_sockets_inherit_the_small_buffers() {
        let l = listen(0, None).unwrap();
        let port = l.local_addr().unwrap().port();
        let _client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (accepted, _) = l.accept().unwrap();
        // the kernel rounds up to 2304 and 4608; its defaults are far above these
        let s = SockRef::from(&accepted);
        let (recv, send) = (s.recv_buffer_size().unwrap(), s.send_buffer_size().unwrap());
        assert!(recv <= 4096 && send <= 8192, "{recv} {send}");
    }
}
