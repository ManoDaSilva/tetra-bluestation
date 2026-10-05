use std::time::{Duration, Instant};

use tetra_config::bluestation::{CfgT1Test, SharedConfig};
use tetra_core::tetra_entities::TetraEntity;
use tetra_core::{PhyBlockNum, Sap, TdmaTime, TrainingSequence};
use tetra_saps::{SapMsg, SapMsgInner};

use crate::t1test::diag::{self, PAYLOAD_BITS};
use crate::t1test::dl_gen::{T1DlGen, T1DlParams};
use crate::umac::subcomp::bs_sched::MACSCHED_TX_AHEAD;
use crate::{MessageQueue, TetraEntityTrait};

/// Entity driving the stack in BS T1 test mode.
///
/// It sits where LmacBs normally sits: the PHY sends its uplink bursts here and takes its downlink
/// slots from here, so the PHY needs no changes. For that reason it registers as `TetraEntity::Lmac`.
pub struct T1TestBs {
    cfg: CfgT1Test,
    dltime: TdmaTime,
    dl_gen: T1DlGen,

    started: Instant,
    last_report: Instant,
    ticks: u64,
    /// Uplink bursts detected by the PHY, per uplink timeslot (index 0 is timeslot 1)
    ul_bursts_per_slot: [u64; 4],
    /// Uplink bursts detected by the PHY, per training sequence: [normal 1, normal 2, extended]
    ul_bursts_per_train: [u64; 3],

    /// First half of a burst with two separate blocks, until the second half arrives
    pending_blk1: Option<[u8; PAYLOAD_BITS / 2]>,
    /// Number of detected bursts analyzed against the PRBS so far
    diag_count: u32,
}

/// Number of detected uplink bursts that get a diagnostic line
const DIAG_MAX_BURSTS: u32 = 30;

impl T1TestBs {
    pub fn new(config: SharedConfig) -> Self {
        let stack_cfg = config.config();
        let cfg = stack_cfg.t1_test.clone().expect("t1_test config must be set in BsT1 stack mode");
        let dl_gen = T1DlGen::new(T1DlParams::from_config(&stack_cfg));
        tracing::info!("T1TestBs: initialized, ul_timeslot {}", cfg.ul_timeslot);

        let now = Instant::now();
        Self {
            cfg,
            dltime: TdmaTime::default(),
            dl_gen,
            started: now,
            last_report: now,
            ticks: 0,
            ul_bursts_per_slot: [0; 4],
            ul_bursts_per_train: [0; 3],
            pending_blk1: None,
            diag_count: 0,
        }
    }

    /// Builds the downlink slot for the timeslot the PHY transmits next
    fn build_dl_slot(&mut self, ts: TdmaTime) -> SapMsg {
        let slot = self.dl_gen.build_slot(ts.add_timeslots(MACSCHED_TX_AHEAD as i32));
        SapMsg {
            sap: Sap::TpSap,
            src: TetraEntity::Lmac,
            dest: TetraEntity::Phy,
            msg: SapMsgInner::TpUnitdataReq(slot),
        }
    }

    fn rx_tp_ind(&mut self, message: SapMsg) {
        let SapMsgInner::TpUnitdataInd(prim) = message.msg else {
            panic!("T1TestBs: unexpected primitive from PHY");
        };

        // Uplink bursts arrive two timeslots after the downlink slot they were sent in
        let ul_time = self.dltime.add_timeslots(-2);
        self.ul_bursts_per_slot[ul_time.t as usize - 1] += 1;
        match prim.train_type {
            TrainingSequence::NormalTrainSeq1 => self.ul_bursts_per_train[0] += 1,
            TrainingSequence::NormalTrainSeq2 => self.ul_bursts_per_train[1] += 1,
            TrainingSequence::ExtendedTrainSeq => self.ul_bursts_per_train[2] += 1,
            _ => {}
        }

        // Collect the 432 payload bits of the burst and run the diagnostic on the first few
        if self.diag_count >= DIAG_MAX_BURSTS {
            return;
        }
        let mut block = prim.block;
        let payload = match prim.block_num {
            PhyBlockNum::Both if block.get_len() == PAYLOAD_BITS => {
                let mut p = [0u8; PAYLOAD_BITS];
                block.to_bitarr(&mut p);
                Some(p)
            }
            PhyBlockNum::Block1 if block.get_len() == PAYLOAD_BITS / 2 => {
                let mut p = [0u8; PAYLOAD_BITS / 2];
                block.to_bitarr(&mut p);
                self.pending_blk1 = Some(p);
                None
            }
            PhyBlockNum::Block2 if block.get_len() == PAYLOAD_BITS / 2 => self.pending_blk1.take().map(|b1| {
                let mut p = [0u8; PAYLOAD_BITS];
                p[..PAYLOAD_BITS / 2].copy_from_slice(&b1);
                block.to_bitarr(&mut p[PAYLOAD_BITS / 2..]);
                p
            }),
            _ => None,
        };
        if let Some(payload) = payload {
            self.diag_count += 1;
            let fits = diag::analyze(&payload, self.dl_gen.scrambling_code());
            let list: Vec<String> = fits.iter().map(|f| format!("{} {}", f.hypothesis, f.errors)).collect();
            let best = fits.iter().min_by_key(|f| f.errors).unwrap();
            println!(
                "T1 diag {}/{} | f{} ts{} | train {:?} | errors vs PRBS of 432 bits: [{}] | best {} phase {}",
                self.diag_count,
                DIAG_MAX_BURSTS,
                ul_time.f,
                ul_time.t,
                prim.train_type,
                list.join(", "),
                best.hypothesis,
                best.phase
            );
        }
    }

    fn print_report(&mut self) {
        let s = &self.ul_bursts_per_slot;
        let t = &self.ul_bursts_per_train;
        println!(
            "T1 | t {:>6.1}s | ticks {} | UL bursts per slot [{} {} {} {}] | per train seq [n1 {} n2 {} ext {}] | measurement not implemented yet",
            self.started.elapsed().as_secs_f32(),
            self.ticks,
            s[0],
            s[1],
            s[2],
            s[3],
            t[0],
            t[1],
            t[2],
        );
    }
}

impl TetraEntityTrait for T1TestBs {
    fn entity(&self) -> TetraEntity {
        TetraEntity::Lmac
    }

    fn rx_prim(&mut self, _queue: &mut MessageQueue, message: SapMsg) {
        match message.sap {
            Sap::TpSap => self.rx_tp_ind(message),
            _ => panic!("T1TestBs: unexpected SAP {:?}", message.sap),
        }
    }

    fn tick_start(&mut self, _queue: &mut MessageQueue, ts: TdmaTime) {
        self.dltime = ts;
    }

    fn tick_end(&mut self, queue: &mut MessageQueue, ts: TdmaTime) -> bool {
        self.ticks += 1;
        queue.push_back(self.build_dl_slot(ts));

        if self.last_report.elapsed() >= Duration::from_millis(self.cfg.report_interval_ms as u64) {
            self.last_report = Instant::now();
            self.print_report();
        }
        false
    }
}
