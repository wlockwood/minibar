/// SplitMix64: tiny, fast, and fully deterministic. Training needs
/// reproducible noise, not cryptographic quality.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Independent stream for `(seed, stream)`, so a candidate's noise never
    /// depends on how many other candidates were drawn before it.
    pub fn for_stream(seed: u64, stream: u64) -> Self {
        let mut mixer = Self::new(seed ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        // Discard one output so adjacent stream ids decorrelate.
        mixer.next_u64();
        mixer
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn uniform(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `[-1, 1)`.
    pub fn signed(&mut self) -> f32 {
        self.uniform() * 2.0 - 1.0
    }

    /// Standard normal via Box–Muller.
    pub fn gaussian(&mut self) -> f32 {
        let u1 = self.uniform().max(f32::MIN_POSITIVE);
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_reproducible_and_distinct() {
        let a: Vec<u64> = (0..4)
            .scan(Rng::for_stream(7, 1), |r, _| Some(r.next_u64()))
            .collect();
        let b: Vec<u64> = (0..4)
            .scan(Rng::for_stream(7, 1), |r, _| Some(r.next_u64()))
            .collect();
        let c: Vec<u64> = (0..4)
            .scan(Rng::for_stream(7, 2), |r, _| Some(r.next_u64()))
            .collect();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn gaussian_has_roughly_unit_moments() {
        let mut rng = Rng::new(11);
        let samples: Vec<f32> = (0..20_000).map(|_| rng.gaussian()).collect();
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        let variance =
            samples.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / samples.len() as f32;
        assert!(mean.abs() < 0.03, "mean {mean}");
        assert!((variance - 1.0).abs() < 0.05, "variance {variance}");
    }
}
