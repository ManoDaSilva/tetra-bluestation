//! Diagnostic for uplink bursts that the PHY detects.
//!
//! Tests the received payload against the T1 PRBS under a few hypotheses about what the receive chain did
//! to the burst, so a wrong assumption (spectrum inversion, scrambling layout) shows up directly.

use tetra_core::BitBuffer;

use crate::lmac::components::scrambler;
use crate::t1test::prbs::{PRBS_PERIOD, Prbs511};

pub const PAYLOAD_BITS: usize = 432;

/// One combination of assumptions about the received burst
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hypothesis {
    /// The received spectrum is mirrored: bit 0 of every dibit is inverted
    pub inverted: bool,
    /// The burst carries two separately scrambled 216-bit blocks instead of one 432-bit block
    pub two_blocks: bool,
}

pub const HYPOTHESES: [Hypothesis; 4] = [
    Hypothesis {
        inverted: false,
        two_blocks: false,
    },
    Hypothesis {
        inverted: false,
        two_blocks: true,
    },
    Hypothesis {
        inverted: true,
        two_blocks: false,
    },
    Hypothesis {
        inverted: true,
        two_blocks: true,
    },
];

impl std::fmt::Display for Hypothesis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{}",
            if self.inverted { "inv" } else { "norm" },
            if self.two_blocks { "2blk" } else { "1blk" }
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Fit {
    pub hypothesis: Hypothesis,
    /// PRBS phase of the first payload bit
    pub phase: usize,
    /// Bit errors against the best matching PRBS phase
    pub errors: usize,
}

/// Turns received type-5 payload bits into type-1 bits under a hypothesis
pub fn undo_hypothesis(payload: &[u8; PAYLOAD_BITS], h: Hypothesis, scrambling_code: u32) -> [u8; PAYLOAD_BITS] {
    let mut bits = *payload;
    if h.inverted {
        for b in bits.iter_mut().step_by(2) {
            *b ^= 1;
        }
    }
    let mut out = [0u8; PAYLOAD_BITS];
    if h.two_blocks {
        for (i, chunk) in bits.chunks(PAYLOAD_BITS / 2).enumerate() {
            let mut buf = BitBuffer::from_bitarr(chunk);
            scrambler::tetra_scramb_bits(scrambling_code, &mut buf);
            buf.to_bitarr(&mut out[i * PAYLOAD_BITS / 2..(i + 1) * PAYLOAD_BITS / 2]);
        }
    } else {
        let mut buf = BitBuffer::from_bitarr(&bits);
        scrambler::tetra_scramb_bits(scrambling_code, &mut buf);
        buf.to_bitarr(&mut out);
    }
    out
}

/// PRBS reference of two periods, so any phase can be read as a contiguous window
fn prbs_reference() -> Vec<u8> {
    let mut p = Prbs511::new();
    let mut v = vec![0u8; PRBS_PERIOD + PAYLOAD_BITS];
    p.fill(&mut v);
    v
}

/// Best PRBS phase for the given bits and its error count
pub fn best_phase(bits: &[u8; PAYLOAD_BITS], reference: &[u8]) -> (usize, usize) {
    (0..PRBS_PERIOD)
        .map(|phase| {
            let errs = bits
                .iter()
                .zip(&reference[phase..phase + PAYLOAD_BITS])
                .filter(|(a, b)| a != b)
                .count();
            (phase, errs)
        })
        .min_by_key(|&(_, errs)| errs)
        .unwrap()
}

/// Fit of the payload under every hypothesis, in the order of `HYPOTHESES`
pub fn analyze(payload: &[u8; PAYLOAD_BITS], scrambling_code: u32) -> [Fit; 4] {
    let reference = prbs_reference();
    HYPOTHESES.map(|hypothesis| {
        let bits = undo_hypothesis(payload, hypothesis, scrambling_code);
        let (phase, errors) = best_phase(&bits, &reference);
        Fit { hypothesis, phase, errors }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SC: u32 = 0x1234_5679;

    /// Builds a received payload from the PRBS under a hypothesis (the inverse of `undo_hypothesis`)
    fn make_payload(h: Hypothesis, phase: usize) -> [u8; PAYLOAD_BITS] {
        let reference = prbs_reference();
        let mut t1 = [0u8; PAYLOAD_BITS];
        t1.copy_from_slice(&reference[phase..phase + PAYLOAD_BITS]);
        // Scrambling is its own inverse, so applying the same operation scrambles
        let mut bits = undo_hypothesis(&t1, Hypothesis { inverted: false, ..h }, SC);
        if h.inverted {
            for b in bits.iter_mut().step_by(2) {
                *b ^= 1;
            }
        }
        bits
    }

    #[test]
    fn picks_the_right_hypothesis_and_phase() {
        for (i, h) in HYPOTHESES.iter().enumerate() {
            let payload = make_payload(*h, 100 + 37 * i);
            let fits = analyze(&payload, SC);
            assert_eq!(fits[i].errors, 0, "{}", h);
            assert_eq!(fits[i].phase, 100 + 37 * i);
            for (j, f) in fits.iter().enumerate() {
                if j != i {
                    assert!(f.errors > 100, "{} should not fit {}: {} errors", f.hypothesis, h, f.errors);
                }
            }
        }
    }

    #[test]
    fn counts_bit_errors() {
        let h = HYPOTHESES[0];
        let mut payload = make_payload(h, 5);
        for i in [3, 50, 200, 431] {
            payload[i] ^= 1;
        }
        let fit = analyze(&payload, SC)[0];
        assert_eq!((fit.phase, fit.errors), (5, 4));
    }
}
