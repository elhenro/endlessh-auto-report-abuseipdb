//! how many connections the tarpit holds, overall and per network

use crate::ip::{is_reserved, network};
use std::{
    collections::{hash_map::Entry, HashMap},
    net::IpAddr,
    sync::Mutex,
};

pub struct Slots(Mutex<State>);

struct State {
    max: usize,
    per_network: usize,
    total: usize,
    networks: HashMap<IpAddr, usize>,
}

/// room left for the worker's api sockets and event files when clients ran the process out of fds
const FD_HEADROOM: usize = 16;

impl Slots {
    pub fn new(max: usize, per_network: usize) -> Self {
        Self(Mutex::new(State {
            max,
            per_network,
            total: 0,
            networks: HashMap::new(),
        }))
    }

    pub fn full(&self) -> bool {
        let s = self.0.lock().expect("slots lock");
        s.total >= s.max
    }

    /// false: this network holds its share already, so one host cannot starve everyone else.
    /// private addresses are exempt: behind a docker proxy every visitor looks like one gateway ip
    pub fn acquire(&self, ip: IpAddr) -> bool {
        let s = &mut *self.0.lock().expect("slots lock");
        if !is_reserved(ip) {
            let held = s.networks.entry(network(ip)).or_default();
            if *held >= s.per_network {
                return false;
            }
            *held += 1;
        }
        s.total += 1;
        true
    }

    pub fn release(&self, ip: IpAddr) {
        let s = &mut *self.0.lock().expect("slots lock");
        s.total -= 1;
        if let Entry::Occupied(mut held) = s.networks.entry(network(ip)) {
            *held.get_mut() -= 1;
            if *held.get() == 0 {
                held.remove();
            }
        }
    }

    /// out of fds: hold fewer clients until restart. returns the new cap
    pub fn shrink(&self) -> usize {
        let s = &mut *self.0.lock().expect("slots lock");
        s.max = s.total.saturating_sub(FD_HEADROOM).max(1);
        s.max
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn one_network_cannot_take_more_than_its_share() {
        let s = Slots::new(100, 2);
        assert!(s.acquire(ip("8.8.8.8")) && s.acquire(ip("8.8.8.8")));
        assert!(!s.acquire(ip("8.8.8.8")));
        assert!(s.acquire(ip("1.1.1.1")), "other networks are not affected");
        s.release(ip("8.8.8.8"));
        assert!(s.acquire(ip("8.8.8.8")), "a freed slot can be used again");
    }

    #[test]
    fn rotating_within_a_v6_64_does_not_help() {
        let s = Slots::new(100, 2);
        assert!(s.acquire(ip("2a00:1450::1")) && s.acquire(ip("2a00:1450::2")));
        assert!(!s.acquire(ip("2a00:1450::3")));
        assert!(s.acquire(ip("2a00:1450:0:1::1")), "next /64");
    }

    #[test]
    fn private_addresses_have_no_share() {
        let s = Slots::new(100, 1);
        for _ in 0..50 {
            assert!(s.acquire(ip("172.17.0.1")));
        }
        for _ in 0..50 {
            s.release(ip("172.17.0.1"));
        }
        assert!(!s.full());
    }

    #[test]
    fn counts_up_to_the_cap_and_frees_entries() {
        let s = Slots::new(2, 5);
        assert!(s.acquire(ip("8.8.8.8")) && !s.full());
        assert!(s.acquire(ip("1.1.1.1")) && s.full());
        s.release(ip("8.8.8.8"));
        s.release(ip("1.1.1.1"));
        assert!(!s.full());
        assert!(s.0.lock().unwrap().networks.is_empty(), "no entry leaks");
    }

    #[test]
    fn shrinking_leaves_room_for_other_fds_and_never_reaches_zero() {
        let s = Slots::new(1000, 1000);
        for _ in 0..100 {
            assert!(s.acquire(ip("8.8.8.8")));
        }
        assert_eq!(s.shrink(), 84);
        assert!(s.full());
        for _ in 0..100 {
            s.release(ip("8.8.8.8"));
        }
        assert_eq!(s.shrink(), 1);
        assert!(!s.full(), "still serves after everyone left");
    }
}
