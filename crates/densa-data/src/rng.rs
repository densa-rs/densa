/// wyrand: a tiny, fast, non-cryptographic PRNG (Wang Yi, public domain).
///
/// Used for deterministic synthetic feeds; never for anything security-related.
#[derive(Debug, Clone)]
pub struct WyRand {
    state: u64,
}

impl WyRand {
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0xa076_1d64_78bd_642f);
        let t = u128::from(self.state) * u128::from(self.state ^ 0xe703_7ed1_a0b4_28db);
        ((t >> 64) as u64) ^ (t as u64)
    }

    /// Uniform value in `0..n` via Lemire's multiply-shift.
    ///
    /// Bias is below `n / 2^32`, negligible for symbol selection. Returns 0 when `n == 0`.
    #[inline]
    pub fn below(&mut self, n: u32) -> u32 {
        (((self.next_u64() >> 32) * u64::from(n)) >> 32) as u32
    }
}
