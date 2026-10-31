//! takes connections off the listener, within the slot limits

use crate::{
    listen::RECV_BUFFER,
    slots::Slots,
    tarpit::{now_unix, Client, Emitter, Event, Params},
    throttle::Throttled,
};
use socket2::SockRef;
use std::{
    net::TcpListener,
    sync::{mpsc::Sender, Arc},
    thread,
    time::{Duration, Instant},
};

// no io::ErrorKind for these two; the numbers are the same on linux and macos
const ENFILE: i32 = 23;
const EMFILE: i32 = 24;

pub(crate) fn accept_loop(
    listener: TcpListener,
    p: Params,
    slots: Arc<Slots>,
    out: Sender<Client>,
    mut ev: Emitter,
) {
    let (mut errors, mut refused) = (Throttled::default(), Throttled::default());
    loop {
        // at the cap, leave new connections in the kernel backlog instead of accepting
        if slots.full() {
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        let (stream, peer) = match listener.accept() {
            Ok(x) => x,
            Err(e) => {
                if matches!(e.raw_os_error(), Some(ENFILE | EMFILE)) {
                    let cap = slots.shrink();
                    errors.hit(|_| {
                        log!(
                            "out of file descriptors, holding at most {cap} clients until restart: \
                             raise LimitNOFILE or ulimits, or lower TARPIT_MAX_CLIENTS"
                        )
                    });
                } else {
                    errors.hit(|n| log!("accept failed ({n} times): {e}"));
                }
                thread::sleep(Duration::from_millis(100));
                continue;
            }
        };
        let _ = SockRef::from(&stream).set_recv_buffer_size(RECV_BUFFER); // macos does not inherit it
        if stream.set_nonblocking(true).is_err() {
            continue;
        }
        let ip = peer.ip().to_canonical();
        // no event: a client looping here must cost neither log lines nor api calls
        if !slots.acquire(ip) {
            refused.hit(|n| log!("over TARPIT_MAX_PER_IP, closed: {n}, last from {ip}"));
            continue;
        }
        ev.emit(Event::Accept {
            t: now_unix(),
            ip,
            port: peer.port(),
        });
        let now = Instant::now();
        if out
            .send(Client {
                stream,
                ip,
                since: now,
                next: now + p.delay,
                bytes: 0,
            })
            .is_err()
        {
            return;
        }
    }
}
