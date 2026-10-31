//! noisy conditions get counted and logged at most once a minute

use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Throttled {
    count: u64,
    last: Option<Instant>,
}

impl Throttled {
    /// calls `report` with the count so far: on the first hit, then once a minute
    pub fn hit(&mut self, report: impl FnOnce(u64)) {
        self.count += 1;
        let due = self
            .last
            .map_or(true, |l| l.elapsed() >= Duration::from_secs(60));
        if due {
            report(self.count);
            (self.count, self.last) = (0, Some(Instant::now()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_first_hit_then_holds_back() {
        let mut t = Throttled::default();
        let mut reported = Vec::new();
        for _ in 0..5 {
            t.hit(|n| reported.push(n));
        }
        assert_eq!(reported, [1]);
        t.last = Some(Instant::now() - Duration::from_secs(61));
        t.hit(|n| reported.push(n));
        assert_eq!(reported, [1, 5], "the held back hits are counted");
    }
}
