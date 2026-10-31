//! is this visitor worth a report? a bare connect proves little: a web page or a url fetcher can be
//! steered at any host, and it is whoever runs it that gets reported. an ssh client says so in its
//! first bytes, so those are looked at and left in place: peeked, compared with one fixed string,
//! never consumed and never parsed.

use std::{io::ErrorKind, net::TcpStream, time::Duration};

/// how long a client gets to say what it is. openssh, libssh, libssh2 and paramiko all speak first
pub(crate) const HELLO_WAIT: Duration = Duration::from_secs(10);

const SSH: &[u8] = b"SSH-";

/// None: cannot tell from these bytes yet
fn judge(first: &[u8]) -> Option<bool> {
    if first.is_empty() || !SSH.starts_with(first) {
        return Some(false); // left without a word, or speaks something else
    }
    (first.len() == SSH.len()).then_some(true)
}

/// Some(true): an ssh client. None: ask again on the next tick
pub(crate) fn verdict(stream: &TcpStream, waited: Duration) -> Option<bool> {
    let mut first = [0u8; SSH.len()];
    let seen = match stream.peek(&mut first) {
        Ok(n) => judge(&first[..n]),
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => None,
        Err(_) => Some(false),
    };
    seen.or((waited >= HELLO_WAIT).then_some(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        net::{TcpListener, TcpStream},
        thread,
        time::Duration,
    };

    #[test]
    fn first_bytes_decide() {
        assert_eq!(judge(b"SSH-"), Some(true));
        for not_yet in [&b"S"[..], b"SS", b"SSH"] {
            assert_eq!(judge(not_yet), None, "{not_yet:?}");
        }
        for other in [
            &b""[..],
            b"GET ",
            b"\x16\x03\x01\x02",
            b"OPTI",
            b"ssh-",
            b"SSH_",
            b"X",
        ] {
            assert_eq!(judge(other), Some(false), "{other:?}");
        }
    }

    /// a connected pair: (client side, tarpit side)
    fn pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (s, _) = l.accept().unwrap();
        s.set_nonblocking(true).unwrap();
        (c, s)
    }

    fn settled(s: &TcpStream, waited: Duration) -> Option<bool> {
        for _ in 0..200 {
            if let Some(v) = verdict(s, waited) {
                return Some(v);
            }
            thread::sleep(Duration::from_millis(5));
        }
        None
    }

    #[test]
    fn verdict_on_real_sockets() {
        let early = Duration::ZERO;
        let (mut c, s) = pair();
        c.write_all(b"SSH-2.0-x\r\n").unwrap();
        assert_eq!(settled(&s, early), Some(true));
        let mut left = [0u8; 16];
        assert_eq!(s.peek(&mut left).unwrap(), 11, "still unread");

        let (mut c, s) = pair();
        c.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!(settled(&s, early), Some(false));

        let (c, s) = pair();
        drop(c);
        assert_eq!(settled(&s, early), Some(false), "gone without a word");

        let (_c, s) = pair();
        assert_eq!(verdict(&s, early), None, "silent, still in the wait");
        assert_eq!(verdict(&s, HELLO_WAIT), Some(false), "silent to the end");

        let (mut c, s) = pair();
        c.write_all(b"SS").unwrap();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(verdict(&s, early), None, "may still become SSH-");
        assert_eq!(verdict(&s, HELLO_WAIT), Some(false), "it did not");
    }
}
