//! Bit error counting for the T1 uplink signal.
//!
//! The test set sends a 511-bit PRBS in the 432 payload bits of every TCH/7,2 burst. TCH/7,2 has no
//! channel coding (EN 300 392-2 clause 8.3.1.3.4), so the descrambled payload is the PRBS itself and
//! every bit error is a direct channel bit error.
//!
//! The PRBS phase at the start of a burst is not known in advance (the test set may continue the sequence
//! across bursts or restart it). Each burst is therefore compared with all 511 phases and the best one is
//! taken. The comparison is done on packed 64-bit words so this stays cheap on a Raspberry Pi.

use crate::t1test::prbs::{PRBS_PERIOD, Prbs511};

/// Payload bits of a TCH/7,2 burst
pub const BURST_BITS: usize = 432;
/// Bits of the first half of the burst (block 1)
const HALF_BITS: usize = BURST_BITS / 2;
const WORDS: usize = BURST_BITS.div_ceil(64);

/// A burst counts as received when the best PRBS phase leaves at most this many bit errors (30 %).
/// Random data leaves about 185 errors at its best of 511 phases (mean 216, sigma 10), so a false match
/// is practically impossible at this threshold. Bursts above it are counted as not received.
pub const FIT_MAX_ERRORS: u32 = 130;

type Packed = [u64; WORDS];

/// Packs bits MSB first. Unused bits in the last word stay zero
fn pack(bits: &[u8]) -> Packed {
    let mut words = [0u64; WORDS];
    for (i, &b) in bits.iter().enumerate() {
        words[i / 64] |= ((b & 1) as u64) << (63 - i % 64);
    }
    words
}

/// Number of set bits in positions `start..end` of a packed burst
fn count_range(words: &Packed, start: usize, end: usize) -> u32 {
    let mut total = 0;
    for (w, &word) in words.iter().enumerate() {
        let lo = (w * 64).max(start);
        let hi = ((w + 1) * 64).min(end);
        if lo >= hi {
            continue;
        }
        // Bit i of the burst sits at position 63 - i % 64 of its word
        let width = hi - lo;
        let shift = 64 - (lo - w * 64) - width;
        let mask = if width == 64 { u64::MAX } else { ((1u64 << width) - 1) << shift };
        total += (word & mask).count_ones();
    }
    total
}

/// Result of comparing one burst with the PRBS
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BurstFit {
    /// PRBS phase of the first payload bit
    pub phase: usize,
    pub errors: u32,
    pub errors_first_half: u32,
    pub errors_second_half: u32,
}

impl BurstFit {
    /// True if the burst matches the PRBS well enough to count
    pub fn is_received(&self) -> bool {
        self.errors <= FIT_MAX_ERRORS
    }
}

pub struct PrbsFitter {
    /// The 432 bit window of the PRBS at each of the 511 phases
    windows: Vec<Packed>,
}

impl Default for PrbsFitter {
    fn default() -> Self {
        Self::new()
    }
}

impl PrbsFitter {
    pub fn new() -> Self {
        let mut reference = vec![0u8; PRBS_PERIOD + BURST_BITS];
        Prbs511::new().fill(&mut reference);
        let windows = (0..PRBS_PERIOD).map(|p| pack(&reference[p..p + BURST_BITS])).collect();
        Self { windows }
    }

    /// Finds the PRBS phase that matches the descrambled payload best
    pub fn fit(&self, payload: &[u8; BURST_BITS]) -> BurstFit {
        let rx = pack(payload);
        let mut best = (0, u32::MAX);
        for (phase, window) in self.windows.iter().enumerate() {
            let errors: u32 = rx.iter().zip(window).map(|(a, b)| (a ^ b).count_ones()).sum();
            if errors < best.1 {
                best = (phase, errors);
            }
        }
        let (phase, errors) = best;
        let mut diff = [0u64; WORDS];
        for (d, (a, b)) in diff.iter_mut().zip(rx.iter().zip(&self.windows[phase])) {
            *d = a ^ b;
        }
        let errors_first_half = count_range(&diff, 0, HALF_BITS);
        BurstFit {
            phase,
            errors,
            errors_first_half,
            errors_second_half: errors - errors_first_half,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_window(phase: usize) -> [u8; BURST_BITS] {
        let mut r = vec![0u8; PRBS_PERIOD + BURST_BITS];
        Prbs511::new().fill(&mut r);
        r[phase..phase + BURST_BITS].try_into().unwrap()
    }

    #[test]
    fn clean_burst_has_no_errors_at_any_phase() {
        let fitter = PrbsFitter::new();
        for phase in [0, 1, 3, 63, 64, 200, 510] {
            let fit = fitter.fit(&reference_window(phase));
            assert_eq!((fit.phase, fit.errors), (phase, 0));
            assert!(fit.is_received());
        }
    }

    #[test]
    fn errors_are_counted_and_split_by_half() {
        let fitter = PrbsFitter::new();
        let mut burst = reference_window(77);
        // 3 errors in the first half (including the first and last bit of it), 2 in the second
        for i in [0, 100, HALF_BITS - 1, HALF_BITS, BURST_BITS - 1] {
            burst[i] ^= 1;
        }
        let fit = fitter.fit(&burst);
        assert_eq!(fit.phase, 77);
        assert_eq!((fit.errors, fit.errors_first_half, fit.errors_second_half), (5, 3, 2));
    }

    #[test]
    fn heavy_errors_still_find_the_phase() {
        let fitter = PrbsFitter::new();
        let mut burst = reference_window(311);
        // 20 % bit errors, spread evenly
        for i in (0..BURST_BITS).step_by(5) {
            burst[i] ^= 1;
        }
        let fit = fitter.fit(&burst);
        assert_eq!(fit.phase, 311);
        assert!(fit.is_received());
    }

    #[test]
    fn random_data_is_not_received() {
        let fitter = PrbsFitter::new();
        // Pseudo random data from a simple LCG, unrelated to the PRBS
        let mut x: u32 = 12345;
        for _ in 0..200 {
            let mut burst = [0u8; BURST_BITS];
            for b in burst.iter_mut() {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                *b = (x >> 24) as u8 & 1;
            }
            assert!(!fitter.fit(&burst).is_received());
        }
    }

    #[test]
    fn count_range_matches_naive_count() {
        let bits: Vec<u8> = (0..BURST_BITS).map(|i| ((i * 7 + i / 3) % 2) as u8).collect();
        let packed = pack(&bits);
        for (start, end) in [(0, 432), (0, 216), (216, 432), (5, 70), (63, 65), (190, 220), (400, 432)] {
            let naive = bits[start..end].iter().filter(|&&b| b == 1).count() as u32;
            assert_eq!(count_range(&packed, start, end), naive, "{}..{}", start, end);
        }
    }
}
