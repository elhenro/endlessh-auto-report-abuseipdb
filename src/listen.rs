//! where the tarpit listens: the socket systemd passed in, else a dual-stack socket

use socket2::{Domain, Protocol, SockRef, Socket, Type};
use std::{
    env, fs, io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener},
};

/// the smallest receive buffer: less kernel memory per connection, and it slows the sender.
/// on the listener too, so on linux a queued connection holds about 1 KB instead of 126 KB
pub const RECV_BUFFER: usize = 1;

/// the listener systemd passed in (socket activation), else a dual-stack socket on `port`
pub fn listen(port: u16) -> io::Result<TcpListener> {
    if let Some(l) = inherited() {
        // a closed or non-socket fd 3 fails here instead of as an endless accept error loop
        l.local_addr()?;
        SockRef::from(&l).set_recv_buffer_size(RECV_BUFFER)?;
        return Ok(l);
    }
    let bind = |domain: Domain, addr: SocketAddr| -> io::Result<TcpListener> {
        let s = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
        s.set_reuse_address(true)?;
        if domain == Domain::IPV6 {
            s.set_only_v6(false)?;
        }
        s.bind(&addr.into())?;
        s.set_recv_buffer_size(RECV_BUFFER)?;
        s.listen(1024)?;
        Ok(s.into())
    };
    bind(Domain::IPV6, (Ipv6Addr::UNSPECIFIED, port).into()).or_else(|e| {
        // ipv6 off, or the port taken on v6 only. net.ipv6.bindv6only is not a cause: set_only_v6 overrides it
        log!("warning: no ipv6 listener on port {port} ({e}), trying ipv4 only");
        bind(Domain::IPV4, (Ipv4Addr::UNSPECIFIED, port).into())
    })
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
    fn accepted_sockets_inherit_the_small_buffer() {
        let l = listen(0).unwrap();
        let port = l.local_addr().unwrap().port();
        let _client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (accepted, _) = l.accept().unwrap();
        // the kernel rounds up to 2304; its default is far above this
        let size = SockRef::from(&accepted).recv_buffer_size().unwrap();
        assert!(size <= 4096, "{size}");
    }
}
