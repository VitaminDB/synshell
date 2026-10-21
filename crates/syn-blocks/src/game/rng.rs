//! Детерминированный генератор случайных чисел (xorshift64*).
//!
//! Без внешних крейтов: один и тот же seed даёт одну и ту же партию,
//! что нужно и тестам, и будущему «ежедневному испытанию».

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    /// Seed из системного времени (для обычной партии).
    pub fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x1234_5678);
        Self::new(nanos ^ 0xA5A5_5A5A_1234_8765)
    }

    pub fn state(&self) -> u64 {
        self.0
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Равномерно в `0..n` (0 при n == 0).
    #[inline]
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            self.next_u32() % n
        }
    }

    #[inline]
    pub fn index(&mut self, n: usize) -> usize {
        self.below(n as u32) as usize
    }

    /// Равномерно в `[0, 1)`.
    #[inline]
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Равномерно в `[lo, hi)`.
    #[inline]
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// Взвешенный выбор индекса: вероятность пропорциональна `weights[i]`.
    pub fn weighted(&mut self, weights: &[f32]) -> usize {
        let total: f32 = weights.iter().filter(|w| **w > 0.0).sum();
        if total <= 0.0 || weights.is_empty() {
            return 0;
        }
        let mut t = self.unit() * total;
        for (i, w) in weights.iter().enumerate() {
            if *w <= 0.0 {
                continue;
            }
            if t < *w {
                return i;
            }
            t -= *w;
        }
        weights.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let mut a = Rng::new(123);
        let mut b = Rng::new(123);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut c = Rng::new(124);
        assert_ne!(b.next_u64(), c.next_u64());
    }

    #[test]
    fn ranges_hold() {
        let mut r = Rng::new(42);
        for _ in 0..10_000 {
            assert!(r.below(7) < 7);
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
            let v = r.range(-2.0, 3.0);
            assert!((-2.0..3.0).contains(&v));
        }
    }

    #[test]
    fn weighted_respects_zero_and_bias() {
        let mut r = Rng::new(7);
        let w = [0.0, 1.0, 9.0];
        let mut hits = [0usize; 3];
        for _ in 0..20_000 {
            hits[r.weighted(&w)] += 1;
        }
        assert_eq!(hits[0], 0);
        assert!(hits[2] > hits[1] * 6, "{hits:?}");
    }
}
