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

/// RF device that reports an uplink burst with a given training sequence every few slots
struct UplinkDev {
    train_type: TrainingSequence,
    burst: Vec<u8>,
    tick: usize,
}

impl RxTxDev for UplinkDev {
    fn rxtx_timeslot(&mut self, tx_slot: &[TxSlotBits]) -> Result<Vec<Option<RxSlotBits<'_>>>, RxTxDevError> {
        self.tick += 1;
        if self.tick % 3 != 0 {
            return Ok(vec![]);
        }
        Ok(vec![Some(RxSlotBits {
            time: tx_slot[0].time,
            slot: RxBurstBits {
                train_type: self.train_type,
                bits: &self.burst,
            },
            ..Default::default()
        })])
    }
}

/// Uplink bursts of both normal training sequences are accepted and analyzed without error
#[test]
fn t1_stack_handles_detected_uplink_bursts() {
    for train_type in [TrainingSequence::NormalTrainSeq1, TrainingSequence::NormalTrainSeq2] {
        let mut stack_cfg = default_test_config_bs();
        stack_cfg.t1_test = Some(CfgT1Test::default());
        let cfg = SharedConfig::from_parts(stack_cfg, None);

        // 462 bit normal uplink burst filled with PRBS-like data
        let mut prbs = Prbs511::new();
        let mut burst = vec![0u8; 462];
        prbs.fill(&mut burst);

        let mut router = MessageRouter::new(cfg.clone());
        router.register_entity(Box::new(PhyBs::new(
            cfg.clone(),
            UplinkDev {
                train_type,
                burst,
                tick: 0,
            },
        )));
        router.register_entity(Box::new(T1TestBs::new(cfg.clone())));
        router.set_dl_time(TdmaTime::default());
        router.run_stack(Some(60), None);
    }
}
