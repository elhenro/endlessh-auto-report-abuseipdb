//! the endless part: random printable lines that never look like an ssh banner

use std::time::{SystemTime, UNIX_EPOCH};

pub fn seed() -> u64 {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64);
    (ns ^ u64::from(std::process::id()) << 32) | 1
}

fn next_rand(s: &mut u64) -> u64 {
    // xorshift64*: only needs to look noisy, not be secret
    *s ^= *s >> 12;
    *s ^= *s << 25;
    *s ^= *s >> 27;
    s.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 16
}

/// random printable line of 3..=max_line bytes ending in CRLF; never starts "SSH-" (that would be the banner)
pub fn banner(max_line: usize, rng: &mut u64) -> Vec<u8> {
    let len = 3 + next_rand(rng) as usize % (max_line - 2);
    let mut line: Vec<u8> = (0..len - 2)
        .map(|_| 32 + (next_rand(rng) % 95) as u8)
        .collect();
    line.extend_from_slice(b"\r\n");
    if line.starts_with(b"SSH-") {
        line[0] = b'X';
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_is_bounded_printable_crlf() {
        let mut rng = seed();
        for max in [3, 4, 32, 255] {
            for _ in 0..2000 {
                let b = banner(max, &mut rng);
                assert!((3..=max).contains(&b.len()));
                assert!(b.ends_with(b"\r\n"));
                assert!(b[..b.len() - 2].iter().all(|c| (32..127).contains(c)));
                assert!(!b.starts_with(b"SSH-"));
            }
        }
    }
}
