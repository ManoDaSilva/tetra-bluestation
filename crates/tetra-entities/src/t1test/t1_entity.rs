use std::time::{Duration, Instant};

use tetra_config::bluestation::{CfgT1Test, SharedConfig};
use tetra_core::tetra_entities::TetraEntity;
use tetra_core::{Sap, TdmaTime};
use tetra_saps::{SapMsg, SapMsgInner};

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
    ul_bursts_total: u64,
    ul_bursts_on_slot: u64,
}

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
            ul_bursts_total: 0,
            ul_bursts_on_slot: 0,
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
