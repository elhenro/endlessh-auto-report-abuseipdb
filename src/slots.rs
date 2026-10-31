//! how many connections the tarpit holds: overall, per network and per site

use crate::ip::{is_reserved, network, site};
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
    reserved: usize,
    networks: HashMap<IpAddr, usize>,
    sites: HashMap<IpAddr, usize>,
}

/// room left for the worker's api sockets and event files when clients ran the process out of fds
const FD_HEADROOM: usize = 16;

/// a site holds this many networks' worth of clients at most: a routed block must not take every slot
const NETWORKS_PER_SITE: usize = 8;

fn held(counts: &HashMap<IpAddr, usize>, key: IpAddr) -> usize {
    counts.get(&key).copied().unwrap_or(0)
}

fn forget(counts: &mut HashMap<IpAddr, usize>, key: IpAddr) {
    if let Entry::Occupied(mut n) = counts.entry(key) {
        *n.get_mut() -= 1;
        if *n.get() == 0 {
            n.remove();
        }
    }
}

impl Slots {
    pub fn new(max: usize, per_network: usize) -> Self {
        Self(Mutex::new(State {
            max,
            per_network,
            total: 0,
            reserved: 0,
            networks: HashMap::new(),
            sites: HashMap::new(),
        }))
    }

    pub fn full(&self) -> bool {
        let s = self.0.lock().expect("slots lock");
        s.total >= s.max
    }

    /// false: this source holds its share already, so one host cannot starve everyone else
    pub fn acquire(&self, ip: IpAddr) -> bool {
        let s = &mut *self.0.lock().expect("slots lock");
        if is_reserved(ip) {
            // behind a docker proxy every visitor looks like one gateway ip, so no share per address.
            // half the slots stay out of reach: nobody can tell these apart or report them
            if s.reserved >= (s.max / 2).max(1) {
                return false;
            }
            s.reserved += 1;
        } else {
            let (net, block) = (network(ip), site(ip));
            if held(&s.networks, net) >= s.per_network
                || held(&s.sites, block) >= s.per_network.saturating_mul(NETWORKS_PER_SITE)
            {
                return false;
            }
            *s.networks.entry(net).or_default() += 1;
            *s.sites.entry(block).or_default() += 1;
        }
        s.total += 1;
        true
    }

    pub fn release(&self, ip: IpAddr) {
        let s = &mut *self.0.lock().expect("slots lock");
        s.total -= 1;
        if is_reserved(ip) {
            s.reserved -= 1;
        } else {
            forget(&mut s.networks, network(ip));
            forget(&mut s.sites, site(ip));
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
    fn a_routed_block_cannot_take_every_slot() {
        let s = Slots::new(10_000, 2);
        let v6: Vec<_> = (0..16)
            .map(|n| ip(&format!("2a00:1450:0:{:x}::1", n / 2)))
            .collect();
        assert!(
            v6.iter().all(|&a| s.acquire(a)),
            "8 /64s of one /48, 2 each"
        );
        assert!(!s.acquire(ip("2a00:1450:0:ff::1")), "a fresh /64, same /48");
        assert!(s.acquire(ip("2a00:1450:1::1")), "the next /48");

        let v4: Vec<_> = (0..16).map(|n| ip(&format!("8.8.8.{}", n / 2))).collect();
        assert!(v4.iter().all(|&a| s.acquire(a)));
        assert!(!s.acquire(ip("8.8.8.200")), "a fresh address, same /24");
        assert!(s.acquire(ip("8.8.9.1")), "the next /24");

        s.release(v4[0]);
        assert!(
            s.acquire(ip("8.8.8.200")),
            "a freed slot counts for the site too"
        );
    }

    #[test]
    fn private_addresses_have_no_share_but_cannot_take_everything() {
        let s = Slots::new(100, 1);
        for _ in 0..50 {
            assert!(s.acquire(ip("172.17.0.1")), "not limited per address");
        }
        assert!(!s.acquire(ip("172.17.0.1")), "half of all slots");
        assert!(!s.acquire(ip("fd00::1")), "any reserved source");
        assert!(s.acquire(ip("8.8.8.8")), "public visitors still get in");
        s.release(ip("8.8.8.8"));
        for _ in 0..50 {
            s.release(ip("172.17.0.1"));
        }
        assert!(!s.full());
        assert!(s.acquire(ip("172.17.0.1")));
        assert!(Slots::new(1, 1).acquire(ip("127.0.0.1")), "a tiny cap");
    }

    #[test]
    fn counts_up_to_the_cap_and_frees_entries() {
        let s = Slots::new(2, 5);
        assert!(s.acquire(ip("8.8.8.8")) && !s.full());
        assert!(s.acquire(ip("1.1.1.1")) && s.full());
        s.release(ip("8.8.8.8"));
        s.release(ip("1.1.1.1"));
        assert!(!s.full());
        let state = s.0.lock().unwrap();
        assert!(
            state.networks.is_empty() && state.sites.is_empty(),
            "no entry leaks"
        );
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
