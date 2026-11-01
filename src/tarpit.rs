//! the tarpit itself: accept connections and dribble random banner lines forever.
//! it never reads from a client, so nothing a client sends is ever parsed. the one look it
//! takes is a peek at the first bytes, see hello.rs.

use crate::{
    accept::accept_loop,
    banner::{banner, seed},
    hello,
    slots::Slots,
    throttle::Throttled,
};
use std::{
    collections::VecDeque,
    io::{self, Write},
    net::{IpAddr, TcpListener, TcpStream},
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub enum Event {
    /// sent once the client has shown what it is, so up to hello::HELLO_WAIT after `t`
    Accept {
        t: u64,
        ip: IpAddr,
        port: u16,
        ssh: bool,
    },
    Close {
        t: u64,
        ip: IpAddr,
        secs: f64,
        bytes: u64,
    },
}

#[derive(Clone, Copy)]
pub struct Params {
    pub delay: Duration,
    pub max_clients: usize,
    pub max_per_ip: usize,
    pub max_line: usize, // 3..=255
}

pub(crate) struct Client {
    pub stream: TcpStream,
    pub ip: IpAddr,
    pub port: u16,
    pub t: u64,
    pub since: Instant,
    pub next: Instant,
    pub bytes: u64,
    pub announced: bool,
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// starts the accept thread and returns the handle of the client loop thread
pub fn spawn(listener: TcpListener, p: Params, events: SyncSender<Event>) -> JoinHandle<()> {
    let slots = Arc::new(Slots::new(p.max_clients, p.max_per_ip));
    let (new_tx, new_rx) = mpsc::channel();
    let s = slots.clone();
    thread::Builder::new()
        .name("accept".into())
        .spawn(move || accept_loop(listener, p, s, new_tx))
        .expect("spawn accept thread");
    thread::Builder::new()
        .name("clients".into())
        .spawn(move || client_loop(new_rx, p, slots, Emitter::new(events)))
        .expect("spawn client thread")
}

/// never blocks the tarpit: a full queue drops the event and says so once a minute
struct Emitter {
    tx: SyncSender<Event>,
    dropped: Throttled,
}

impl Emitter {
    fn new(tx: SyncSender<Event>) -> Self {
        Self {
            tx,
            dropped: Throttled::default(),
        }
    }

    fn emit(&mut self, ev: Event) {
        if self.tx.try_send(ev).is_err() {
            self.dropped
                .hit(|n| log!("warning: worker queue full, dropped {n} events"));
        }
    }

    fn announce(&mut self, c: &mut Client, ssh: bool) {
        c.announced = true;
        self.emit(Event::Accept {
            t: c.t,
            ip: c.ip,
            port: c.port,
            ssh,
        });
    }
}

fn client_loop(incoming: Receiver<Client>, p: Params, slots: Arc<Slots>, mut ev: Emitter) {
    let mut queue: VecDeque<Client> = VecDeque::new();
    let mut rng = seed();
    loop {
        let now = Instant::now();
        while queue.front().is_some_and(|c| c.next <= now) {
            let mut c = queue.pop_front().expect("front checked");
            if !c.announced {
                if let Some(ssh) = hello::verdict(&c.stream, c.since.elapsed()) {
                    ev.announce(&mut c, ssh);
                }
            }
            match send_line(&mut c, p.max_line, &mut rng) {
                Ok(()) => {
                    c.next = now + p.delay;
                    enqueue(&mut queue, c);
                }
                Err(_) => {
                    if !c.announced {
                        ev.announce(&mut c, false);
                    }
                    slots.release(c.ip);
                    ev.emit(Event::Close {
                        t: now_unix(),
                        ip: c.ip,
                        secs: c.since.elapsed().as_secs_f64(),
                        bytes: c.bytes,
                    });
                }
            }
        }
        let got = match queue.front() {
            Some(c) => incoming.recv_timeout(c.next.saturating_duration_since(Instant::now())),
            None => incoming.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match got {
            Ok(c) => enqueue(&mut queue, c),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// keep the queue ordered by due time; new clients usually land at the back
fn enqueue(queue: &mut VecDeque<Client>, c: Client) {
    let at = queue.partition_point(|x| x.next <= c.next);
    queue.insert(at, c);
}

/// a full send buffer (client not reading) is fine; any other error means the client is gone
fn send_line(c: &mut Client, max_line: usize, rng: &mut u64) -> io::Result<()> {
    match c.stream.write(&banner(max_line, rng)) {
        Ok(n) => {
            c.bytes += n as u64;
            Ok(())
        }
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(())
        }
        Err(e) => Err(e),
    }
}
