mod common;

use std::sync::{Arc, Mutex};

use tetra_config::bluestation::{CfgT1Test, SharedConfig};
use tetra_core::{BitBuffer, TdmaTime, TrainingSequence};
use tetra_entities::MessageRouter;
use tetra_entities::lmac::components::scrambler;
use tetra_entities::phy::components::slotter::bitseq;
use tetra_entities::phy::phy_bs::PhyBs;
use tetra_entities::t1test::T1TestBs;
use tetra_entities::t1test::prbs::Prbs511;
use tetra_entities::t1test::report::T1Counters;
use tetra_pdus::phy::traits::rxtx_dev::{RxBurstBits, RxSlotBits, RxTxDev, RxTxDevError, TxSlotBits};

use common::default_stack::default_test_config_bs;

/// RF device that records everything transmitted and receives nothing
struct RecordingDev {
    tx: Arc<Mutex<Vec<(TdmaTime, Vec<u8>)>>>,
}

impl RxTxDev for RecordingDev {
    fn rxtx_timeslot(&mut self, tx_slot: &[TxSlotBits]) -> Result<Vec<Option<RxSlotBits<'_>>>, RxTxDevError> {
        for s in tx_slot {
            if let Some(bits) = s.slot {
                self.tx.lock().unwrap().push((s.time, bits.to_vec()));
            }
        }
        Ok(vec![])
    }
}

#[test]
fn t1_stack_transmits_sync_in_frame18_slot1_and_tch72_elsewhere() {
    let mut stack_cfg = default_test_config_bs();
    stack_cfg.t1_test = Some(CfgT1Test::default());
    let cfg = SharedConfig::from_parts(stack_cfg, None);

    let tx = Arc::new(Mutex::new(Vec::new()));
    let mut router = MessageRouter::new(cfg.clone());
    router.register_entity(Box::new(PhyBs::new(cfg.clone(), RecordingDev { tx: tx.clone() })));
    router.register_entity(Box::new(T1TestBs::new(cfg.clone())));
    router.set_dl_time(TdmaTime::default());

    // Two multiframes
    let ticks = 2 * 18 * 4;
    router.run_stack(Some(ticks), None);

    let tx = tx.lock().unwrap();
    assert_eq!(tx.len(), ticks, "one slot transmitted per tick");

    // Reference payload for TCH/7,2 slots: PRBS in transmit order, scrambled with the cell code
    let c = cfg.config();
    let scramb = scrambler::tetra_scramb_get_init(c.net.mcc, c.net.mnc, c.cell.colour_code);
    let mut prbs = Prbs511::new();

    let mut num_sync = 0;
    for (i, (time, bits)) in tx.iter().enumerate() {
        if i > 0 {
            assert_eq!(time.diff(tx[i - 1].0), 1, "consecutive timeslots");
        }
        if time.f == 18 && time.t == 1 {
            num_sync += 1;
            assert_eq!(&bits[214..252], &bitseq::y[..], "sync training sequence at {}", time);
        } else {
            assert_eq!(&bits[244..266], &bitseq::n[..], "normal training sequence 1 at {}", time);

            // Payload sits in the two 216-bit blocks of the normal burst
            let mut payload = [0u8; 432];
            payload[..216].copy_from_slice(&bits[14..230]);
            payload[216..].copy_from_slice(&bits[282..498]);
            let mut blk = BitBuffer::from_bitarr(&payload);
            scrambler::tetra_scramb_bits(scramb, &mut blk);
            blk.to_bitarr(&mut payload);
            let mut expect = [0u8; 432];
            prbs.fill(&mut expect);
            assert_eq!(payload, expect, "TCH/7,2 PRBS payload at {}", time);
        }
    }
    assert_eq!(num_sync, 2, "one sync burst per multiframe");
}

/// RF device that reports a T1 uplink burst in every slot: a PRBS payload that continues from burst to burst,
/// scrambled with the cell code, with a fixed number of bit errors
struct UplinkDev {
    scrambling_code: u32,
    errors_per_burst: usize,
    /// PRBS phase of the next burst
    phase: usize,
    burst: Vec<u8>,
}

impl UplinkDev {
    fn new(scrambling_code: u32, errors_per_burst: usize) -> Self {
        Self {
            scrambling_code,
            errors_per_burst,
            phase: 0,
            burst: vec![0u8; 462],
        }
    }

    fn make_burst(&mut self) {
        let mut reference = vec![0u8; 511 + 432];
        Prbs511::new().fill(&mut reference);
        let mut payload = BitBuffer::from_bitarr(&reference[self.phase..self.phase + 432]);
        scrambler::tetra_scramb_bits(self.scrambling_code, &mut payload);
        let mut bits = [0u8; 432];
        payload.to_bitarr(&mut bits);
        // Spread the errors over both halves of the burst
        for i in 0..self.errors_per_burst {
            bits[(i * 97 + 5) % 432] ^= 1;
        }

        // Normal uplink burst: 4 head bits, block 1, training sequence 1, block 2, 4 tail bits
        self.burst[4..220].copy_from_slice(&bits[..216]);
        self.burst[220..242].copy_from_slice(&bitseq::n);
        self.burst[242..458].copy_from_slice(&bits[216..]);
        self.phase = (self.phase + 432) % 511;
    }
}

impl RxTxDev for UplinkDev {
    fn rxtx_timeslot(&mut self, tx_slot: &[TxSlotBits]) -> Result<Vec<Option<RxSlotBits<'_>>>, RxTxDevError> {
        self.make_burst();
        Ok(vec![Some(RxSlotBits {
            time: tx_slot[0].time,
            slot: RxBurstBits {
                train_type: TrainingSequence::NormalTrainSeq1,
                bits: &self.burst,
            },
            ..Default::default()
        })])
    }
}

/// Runs the T1 stack against `UplinkDev` and returns the final counters and the exit code
fn run_uplink(t1: CfgT1Test, errors_per_burst: usize, ticks: usize) -> (T1Counters, i32) {
    let mut stack_cfg = default_test_config_bs();
    stack_cfg.t1_test = Some(t1);
    let cfg = SharedConfig::from_parts(stack_cfg, None);
    let c = cfg.config();
    let scramb = scrambler::tetra_scramb_get_init(c.net.mcc, c.net.mnc, c.cell.colour_code);

    let mut router = MessageRouter::new(cfg.clone());
    router.register_entity(Box::new(PhyBs::new(cfg.clone(), UplinkDev::new(scramb, errors_per_burst))));
    let t1 = T1TestBs::new(cfg.clone());
    let handle = t1.handle();
    router.register_entity(Box::new(t1));
    router.set_dl_time(TdmaTime::default());
    router.run_stack(Some(ticks), None);
    drop(router);

    let shared = handle.lock().unwrap();
    (shared.total.clone(), shared.exit_code())
}

#[test]
fn t1_measures_ber_on_the_configured_slot() {
    let t1 = CfgT1Test {
        ber_limit_percent: Some(1.0),
        min_bits: Some(5000),
        ..Default::default()
    };
    // Three multiframes
    let (c, exit_code) = run_uplink(t1, 3, 3 * 72);

    // Slot 1, frames 1-17: 17 bursts per multiframe, none from frame 18
    assert!((50..=51).contains(&c.expected_bursts), "expected {}", c.expected_bursts);
    assert_eq!(c.detected_bursts, c.expected_bursts);
    assert_eq!(c.received_bursts, c.expected_bursts);
    assert_eq!(c.bits, 432 * c.received_bursts);
    assert_eq!(c.bit_errors, 3 * c.received_bursts);
    assert_eq!(c.error_bursts, c.received_bursts);
    assert_eq!(c.errors_per_burst, [0, c.received_bursts, 0, 0, 0]);
    // 3 errors in 432 bits is 0.69 %, under the 1 % limit, and more than 5000 bits were measured
    assert_eq!(exit_code, 0);
}

#[test]
fn t1_fails_when_ber_is_over_the_limit() {
    let t1 = CfgT1Test {
        ber_limit_percent: Some(0.5),
        min_bits: Some(5000),
        ..Default::default()
    };
    let (c, exit_code) = run_uplink(t1, 3, 3 * 72);
    assert!(c.bits > 5000);
    assert_eq!(exit_code, 1);
}

#[test]
fn t1_is_unsettled_before_min_bits() {
    let t1 = CfgT1Test {
        ber_limit_percent: Some(5.0),
        min_bits: Some(10_000_000),
        ..Default::default()
    };
    let (_, exit_code) = run_uplink(t1, 0, 2 * 72);
    assert_eq!(exit_code, 2);
}

#[test]
fn t1_measures_all_four_slots_when_asked() {
    let t1 = CfgT1Test {
        measure_all_slots: true,
        ..Default::default()
    };
    let (c, exit_code) = run_uplink(t1, 0, 3 * 72);

    // Frames 1-17 of every slot: 68 bursts per multiframe
    assert!((200..=204).contains(&c.expected_bursts), "expected {}", c.expected_bursts);
    assert_eq!(c.received_bursts, c.expected_bursts);
    assert_eq!(c.bit_errors, 0);
    assert_eq!(c.errors_per_burst[0], c.received_bursts);
    // No limit configured: any received bits count as success
    assert_eq!(exit_code, 0);
}
