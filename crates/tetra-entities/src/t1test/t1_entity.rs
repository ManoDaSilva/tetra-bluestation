use std::time::{Duration, Instant};

use tetra_config::bluestation::{CfgT1Test, SharedConfig};
use tetra_core::tetra_entities::TetraEntity;
use tetra_core::{BitBuffer, BurstType, Sap, TdmaTime, TrainingSequence};
use tetra_saps::tp::TpUnitdataReqSlot;
use tetra_saps::{SapMsg, SapMsgInner};

use crate::{MessageQueue, TetraEntityTrait};

/// Bit counts of a full normal burst block pair and of the broadcast block
const NDB_FULL_BLK_BITS: usize = 432;
const BBK_BITS: usize = 30;

/// Entity driving the stack in BS T1 test mode.
///
/// It sits where LmacBs normally sits: the PHY sends its uplink bursts here and takes its downlink
/// slots from here, so the PHY needs no changes. For that reason it registers as `TetraEntity::Lmac`.
pub struct T1TestBs {
    cfg: CfgT1Test,
    dltime: TdmaTime,

    started: Instant,
    last_report: Instant,
    ticks: u64,
    ul_bursts_total: u64,
    ul_bursts_on_slot: u64,
}

impl T1TestBs {
    pub fn new(config: SharedConfig) -> Self {
        let cfg = config
            .config()
            .t1_test
            .clone()
            .expect("t1_test config must be set in BsT1 stack mode");
        tracing::info!("T1TestBs: initialized, ul_timeslot {}", cfg.ul_timeslot);

        let now = Instant::now();
        Self {
            cfg,
            dltime: TdmaTime::default(),
            started: now,
            last_report: now,
            ticks: 0,
            ul_bursts_total: 0,
            ul_bursts_on_slot: 0,
        }
    }

    /// Phase 1 placeholder downlink: an all-zero normal burst. This keeps the PHY timing loop running
    /// but is not a valid TETRA downlink. Replaced by sync and T1 content in phase 2.
    fn build_placeholder_dl_slot(&self) -> SapMsg {
        SapMsg {
            sap: Sap::TpSap,
            src: TetraEntity::Lmac,
            dest: TetraEntity::Phy,
            msg: SapMsgInner::TpUnitdataReq(TpUnitdataReqSlot {
                train_type: TrainingSequence::NormalTrainSeq1,
                burst_type: BurstType::NDB,
                bbk: Some(BitBuffer::new(BBK_BITS)),
                blk1: Some(BitBuffer::new(NDB_FULL_BLK_BITS)),
                blk2: None,
            }),
        }
    }

    fn rx_tp_ind(&mut self, message: SapMsg) {
        let SapMsgInner::TpUnitdataInd(_prim) = message.msg else {
            panic!("T1TestBs: unexpected primitive from PHY");
        };

        // Uplink bursts arrive two timeslots after the downlink slot they were sent in
        let ul_time = self.dltime.add_timeslots(-2);
        self.ul_bursts_total += 1;
        if ul_time.t == self.cfg.ul_timeslot {
            self.ul_bursts_on_slot += 1;
        }
    }

    fn print_report(&mut self) {
        println!(
            "T1 | t {:>6.1}s | ticks {} | UL bursts {} (slot {}: {}) | measurement not implemented yet",
            self.started.elapsed().as_secs_f32(),
            self.ticks,
            self.ul_bursts_total,
            self.cfg.ul_timeslot,
            self.ul_bursts_on_slot
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

    fn tick_end(&mut self, queue: &mut MessageQueue, _ts: TdmaTime) -> bool {
        self.ticks += 1;
        queue.push_back(self.build_placeholder_dl_slot());

        if self.last_report.elapsed() >= Duration::from_millis(self.cfg.report_interval_ms as u64) {
            self.last_report = Instant::now();
            self.print_report();
        }
        false
    }
}
