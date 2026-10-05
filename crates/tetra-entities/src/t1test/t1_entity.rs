use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tetra_config::bluestation::{CfgT1Test, SharedConfig};
use tetra_core::tetra_entities::TetraEntity;
use tetra_core::{PhyBlockNum, Sap, TdmaTime, TrainingSequence};
use tetra_saps::{SapMsg, SapMsgInner};

use crate::lmac::components::scrambler;
use crate::t1test::ber::{BURST_BITS, PrbsFitter};
use crate::t1test::dl_gen::{T1DlGen, T1DlParams};
use crate::t1test::report::{LinkState, T1Counters, T1Report, T1Shared, T1SharedHandle};
use crate::umac::subcomp::bs_sched::MACSCHED_TX_AHEAD;
use crate::{MessageQueue, TetraEntityTrait};

/// The test set sends the PRBS in frames 1-17. Frame 18 carries a control channel
const LAST_MEASURED_FRAME: u8 = 17;

/// A link state is reported as lock or signal for this many ticks after the last matching or detected burst (about 2 s)
const STATE_HOLD_TICKS: u64 = 142;

/// Entity driving the stack in BS T1 test mode.
///
/// It sits where LmacBs normally sits: the PHY sends its uplink bursts here and takes its downlink
/// slots from here, so the PHY needs no changes. For that reason it registers as `TetraEntity::Lmac`.
pub struct T1TestBs {
    cfg: CfgT1Test,
    dltime: TdmaTime,
    dl_gen: T1DlGen,
    fitter: PrbsFitter,

    started: Instant,
    last_report: Instant,
    ticks: u64,
    last_detected_tick: Option<u64>,
    last_received_tick: Option<u64>,

    /// Counts since the last report
    interval: T1Counters,
    /// Counts since the start, also published in `shared`
    total: T1Counters,
    shared: T1SharedHandle,
}

impl T1TestBs {
    pub fn new(config: SharedConfig) -> Self {
        let stack_cfg = config.config();
        let cfg = stack_cfg.t1_test.clone().expect("t1_test config must be set in BsT1 stack mode");
        let dl_gen = T1DlGen::new(T1DlParams::from_config(&stack_cfg));
        tracing::info!(
            "T1TestBs: initialized, ul_timeslot {}, all slots {}",
            cfg.ul_timeslot,
            cfg.measure_all_slots
        );

        let now = Instant::now();
        let shared = Arc::new(Mutex::new(T1Shared {
            cfg: cfg.clone(),
            started: now,
            total: T1Counters::default(),
            state: LinkState::NoSignal,
        }));
        Self {
            cfg,
            dltime: TdmaTime::default(),
            dl_gen,
            fitter: PrbsFitter::new(),
            started: now,
            last_report: now,
            ticks: 0,
            last_detected_tick: None,
            last_received_tick: None,
            interval: T1Counters::default(),
            total: T1Counters::default(),
            shared,
        }
    }

    /// Handle for reading the measurement from outside the stack, for example to print the final report
    pub fn handle(&self) -> T1SharedHandle {
        self.shared.clone()
    }

    /// True if an uplink burst at this time belongs to the measurement
    fn is_measured(&self, ul_time: TdmaTime) -> bool {
        ul_time.f <= LAST_MEASURED_FRAME && (self.cfg.measure_all_slots || ul_time.t == self.cfg.ul_timeslot)
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
        if !self.is_measured(ul_time) {
            return;
        }

        // Type 7 is a full-slot normal burst with training sequence 1: one block of 432 bits.
        // Anything else (half-slot blocks, control bursts) is not part of this signal
        let mut block = prim.block;
        if prim.train_type != TrainingSequence::NormalTrainSeq1 || prim.block_num != PhyBlockNum::Both || block.get_len() != BURST_BITS {
            return;
        }
        self.interval.detected_bursts += 1;
        self.last_detected_tick = Some(self.ticks);

        // TCH/7,2 is only scrambled: descramble and compare with the PRBS
        scrambler::tetra_scramb_bits(self.dl_gen.scrambling_code(), &mut block);
        let mut payload = [0u8; BURST_BITS];
        block.to_bitarr(&mut payload);

        let fit = self.fitter.fit(&payload);
        if fit.is_received() {
            self.interval.add_received(&fit);
            self.last_received_tick = Some(self.ticks);
        }
    }

    fn link_state(&self) -> LinkState {
        let recent = |t: Option<u64>| t.is_some_and(|t| self.ticks - t <= STATE_HOLD_TICKS);
        if recent(self.last_received_tick) {
            LinkState::Lock
        } else if recent(self.last_detected_tick) {
            LinkState::Search
        } else {
            LinkState::NoSignal
        }
    }

    /// Folds the interval into the totals, publishes them and prints a report
    fn report(&mut self) {
        self.total.add(&self.interval);
        let state = self.link_state();
        let report = T1Report::build(
            &self.cfg,
            self.started.elapsed().as_secs_f64(),
            state,
            &self.interval,
            &self.total,
            false,
        );
        println!("{}", report.render(self.cfg.output));
        self.interval = T1Counters::default();
        self.publish(state);
    }

    fn publish(&self, state: LinkState) {
        let mut shared = self.shared.lock().expect("T1 shared state");
        shared.total = self.total.clone();
        shared.state = state;
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
        // The uplink slot that the PHY reports in this tick
        if self.is_measured(ts.add_timeslots(-2)) {
            self.interval.expected_bursts += 1;
        }
    }

    fn tick_end(&mut self, queue: &mut MessageQueue, ts: TdmaTime) -> bool {
        self.ticks += 1;
        queue.push_back(self.build_dl_slot(ts));

        if self.last_report.elapsed() >= Duration::from_millis(self.cfg.report_interval_ms as u64) {
            self.last_report = Instant::now();
            self.report();
        }
        false
    }
}

impl Drop for T1TestBs {
    /// Publish whatever was counted since the last report, so the final report is complete
    fn drop(&mut self) {
        self.total.add(&self.interval);
        self.interval = T1Counters::default();
        let state = self.link_state();
        self.publish(state);
    }
}
