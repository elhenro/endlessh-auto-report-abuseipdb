//! address filtering: what is pointless to report, and the operator's allowlist

use std::net::IpAddr;

pub fn is_reserved(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(a) => {
            let o = a.octets();
            a.is_unspecified()
                || a.is_loopback()
                || a.is_private()
                || a.is_link_local()
                || a.is_multicast()
                || a.is_documentation()
                || o[0] == 0
                || o[0] >= 240 // reserved + broadcast
                || (o[0] == 100 && o[1] & 0xc0 == 64) // cgnat 100.64/10
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // ietf protocol assignments
                || (o[0] == 198 && o[1] & 0xfe == 18) // benchmarking 198.18/15
        }
        IpAddr::V6(a) => {
            let s = a.segments();
            a.is_unspecified()
                || a.is_loopback()
                || a.is_multicast()
                || s[0] & 0xffc0 == 0xfe80 // link-local
                || s[0] & 0xfe00 == 0xfc00 // unique local
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        }
    }
}

fn bits(ip: IpAddr) -> (u128, u8) {
    match ip.to_canonical() {
        IpAddr::V4(a) => (u32::from(a).into(), 32),
        IpAddr::V6(a) => (a.into(), 128),
    }
}

/// what one actor controls: a single v4 address, or a whole v6 /64, where rotating the low bits is free
pub fn network(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(a) => IpAddr::V6((u128::from(a) & (u128::MAX << 64)).into()),
        v4 => v4,
    }
}

/// plain ips and cidr ranges, comma or space separated
#[derive(Default)]
pub struct AllowList(Vec<(IpAddr, u8)>);

impl AllowList {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut out = Vec::new();
        for entry in s
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|e| !e.is_empty())
        {
            let (addr, len) = entry
                .split_once('/')
                .map_or((entry, None), |(a, l)| (a, Some(l)));
            let ip: IpAddr = addr
                .parse()
                .map_err(|_| format!("ALLOWLIST: '{entry}' is not an ip or cidr"))?;
            let width = bits(ip).1;
            let prefix = match len {
                None => width,
                Some(l) => l
                    .parse::<u8>()
                    .ok()
                    .filter(|p| *p <= width)
                    .ok_or_else(|| format!("ALLOWLIST: bad prefix in '{entry}'"))?,
            };
            out.push((ip, prefix));
        }
        Ok(Self(out))
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let (v, width) = bits(ip);
        self.0.iter().any(|&(net, prefix)| {
            let (n, nw) = bits(net);
            nw == width && (prefix == 0 || (v ^ n) >> (width - prefix) == 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn reserved_ranges() {
        for r in [
            "10.1.1.1",
            "172.16.0.1",
            "172.31.255.1",
            "192.168.1.1",
            "100.64.0.1",
            "100.127.1.1",
            "127.0.0.1",
            "169.254.1.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "198.18.0.1",
            "203.0.113.9",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:10.0.0.1",
        ] {
            assert!(is_reserved(ip(r)), "{r} should be reserved");
        }
        for p in [
            "8.8.8.8",
            "172.32.0.1",
            "100.128.0.1",
            "223.1.1.1",
            "2a00:1450::1",
            "::ffff:8.8.8.8",
        ] {
            assert!(!is_reserved(ip(p)), "{p} should be public");
        }
    }

    #[test]
    fn network_groups_v6_by_64_and_keeps_v4_whole() {
        assert_eq!(network(ip("2a00:1450:1:2:3:4:5:6")), ip("2a00:1450:1:2::"));
        assert_eq!(network(ip("2a00:1450:1:2::9")), ip("2a00:1450:1:2::"));
        assert_ne!(network(ip("2a00:1450:1:3::9")), ip("2a00:1450:1:2::"));
        assert_eq!(network(ip("8.8.8.8")), ip("8.8.8.8"));
        assert_eq!(network(ip("::ffff:8.8.8.8")), ip("8.8.8.8"));
    }

    #[test]
    fn allowlist_matches_ips_and_ranges() {
        let a = AllowList::parse("1.2.3.4, 10.0.0.0/8 2001:db8:1::/48").unwrap();
        assert!(a.contains(ip("1.2.3.4")));
        assert!(a.contains(ip("::ffff:1.2.3.4")));
        assert!(!a.contains(ip("1.2.3.5")));
        assert!(a.contains(ip("10.255.0.1")));
        assert!(a.contains(ip("2001:db8:1:ffff::1")));
        assert!(!a.contains(ip("2001:db8:2::1")));
    }

    #[test]
    fn allowlist_rejects_garbage() {
        for bad in ["nope", "1.2.3.4/33", "1.2.3.4/x", "::1/129", "1.2.3"] {
            assert!(AllowList::parse(bad).is_err(), "{bad}");
        }
        assert!(AllowList::parse("").unwrap().0.is_empty());
        assert!(AllowList::parse("0.0.0.0/0")
            .unwrap()
            .contains(ip("8.8.8.8")));
    }
}
