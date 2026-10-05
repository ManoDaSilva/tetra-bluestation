//! PRBS generator for the T1 test signal.
//!
//! EN 300 394-1 clause 5.3.2 specifies a repeating pseudo random sequence of 511 bits according to
//! ITU-T O.153. That is the maximal-length 2^9-1 sequence with generator polynomial x^9 + x^5 + 1.

/// Length of the PRBS in bits
pub const PRBS_PERIOD: usize = 511;

#[derive(Debug, Clone)]
pub struct Prbs511 {
    /// 9-bit shift register. Never all-zero
    state: u16,
}

impl Default for Prbs511 {
    fn default() -> Self {
        Self::new()
    }
}

impl Prbs511 {
    pub fn new() -> Self {
        Self { state: 0x1FF }
    }

    /// Next bit of the sequence
    pub fn next_bit(&mut self) -> u8 {
        let out = ((self.state >> 8) & 1) as u8;
        let fb = ((self.state >> 8) ^ (self.state >> 4)) & 1;
        self.state = ((self.state << 1) | fb) & 0x1FF;
        out
    }

    pub fn fill(&mut self, out: &mut [u8]) {
        for b in out.iter_mut() {
            *b = self.next_bit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_is_511_and_balanced() {
        let mut p = Prbs511::new();
        let mut seq = [0u8; PRBS_PERIOD];
        p.fill(&mut seq);
        // A maximal-length 9-bit sequence has 256 ones and 255 zeros per period
        assert_eq!(seq.iter().map(|&b| b as usize).sum::<usize>(), 256);
        // The sequence repeats after exactly 511 bits
        let mut again = [0u8; PRBS_PERIOD];
        p.fill(&mut again);
        assert_eq!(seq, again);
        // And not after any shorter shift
        for shift in 1..PRBS_PERIOD {
            let same = (0..PRBS_PERIOD).all(|i| seq[i] == seq[(i + shift) % PRBS_PERIOD]);
            assert!(!same, "sequence repeats with shift {}", shift);
        }
    }
}
