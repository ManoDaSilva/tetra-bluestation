//! Counters and reports for the T1 receiver measurement.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use tetra_config::bluestation::{CfgT1Test, T1OutputFormat};

use crate::t1test::ber::{BURST_BITS, BurstFit};

/// Upper limits (inclusive) of the errors-per-burst histogram bins. The last bin is everything above
pub const HIST_LIMITS: [u32; 4] = [0, 4, 16, 64];

fn hist_bin(errors: u32) -> usize {
    HIST_LIMITS.iter().position(|&l| errors <= l).unwrap_or(HIST_LIMITS.len())
}

/// Raw counts. Used both per report interval and over the whole run
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct T1Counters {
    /// Uplink bursts that could have arrived on the measured timeslots (frames 1-17)
    pub expected_bursts: u64,
    /// Bursts the PHY detected on the measured timeslots
    pub detected_bursts: u64,
    /// Detected bursts that match the PRBS (see `FIT_MAX_ERRORS`). Only these count towards the bit totals
    pub received_bursts: u64,
    pub bits: u64,
    pub bit_errors: u64,
    /// Received bursts with at least one bit error
    pub error_bursts: u64,
    /// Bit errors in the first and second half of the burst. A skew points to timing or frequency drift
    pub errors_first_half: u64,
    pub errors_second_half: u64,
    /// Received bursts by errors per burst: 0, 1-4, 5-16, 17-64, more than 64
    pub errors_per_burst: [u64; 5],
}

impl T1Counters {
    pub fn add_received(&mut self, fit: &BurstFit) {
        self.received_bursts += 1;
        self.bits += BURST_BITS as u64;
        self.bit_errors += fit.errors as u64;
        self.errors_first_half += fit.errors_first_half as u64;
        self.errors_second_half += fit.errors_second_half as u64;
        if fit.errors > 0 {
            self.error_bursts += 1;
        }
        self.errors_per_burst[hist_bin(fit.errors)] += 1;
    }

    pub fn add(&mut self, other: &T1Counters) {
        self.expected_bursts += other.expected_bursts;
        self.detected_bursts += other.detected_bursts;
        self.received_bursts += other.received_bursts;
        self.bits += other.bits;
        self.bit_errors += other.bit_errors;
        self.error_bursts += other.error_bursts;
        self.errors_first_half += other.errors_first_half;
        self.errors_second_half += other.errors_second_half;
        for (a, b) in self.errors_per_burst.iter_mut().zip(other.errors_per_burst) {
            *a += b;
        }
    }
}

fn ratio(num: u64, den: u64) -> Option<f64> {
    (den > 0).then(|| num as f64 / den as f64)
}

/// Counters plus the rates derived from them
#[derive(Debug, Clone, Serialize)]
pub struct T1Stats {
    #[serde(flatten)]
    pub counters: T1Counters,
    /// Bit error ratio over the received bursts
    pub ber: Option<f64>,
    /// Share of received bursts with at least one bit error
    pub bler: Option<f64>,
    /// Detected over expected bursts
    pub detection_rate: Option<f64>,
    /// Received over expected bursts. Bursts that were not received are not in the BER
    pub receive_rate: Option<f64>,
}

impl T1Stats {
    pub fn new(c: &T1Counters) -> Self {
        Self {
            counters: c.clone(),
            ber: ratio(c.bit_errors, c.bits),
            bler: ratio(c.error_bursts, c.received_bursts),
            detection_rate: ratio(c.detected_bursts, c.expected_bursts),
            receive_rate: ratio(c.received_bursts, c.expected_bursts),
        }
    }
}

/// State of the uplink signal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LinkState {
    /// Nothing detected recently
    #[serde(rename = "NO_SIGNAL")]
    NoSignal,
    /// Bursts detected, but none matched the PRBS recently
    #[serde(rename = "SEARCH")]
    Search,
    /// Bursts matched the PRBS recently
    #[serde(rename = "LOCK")]
    Lock,
}

impl LinkState {
    fn as_str(&self) -> &'static str {
        match self {
            LinkState::NoSignal => "NO SIGNAL",
            LinkState::Search => "SEARCH",
            LinkState::Lock => "LOCK",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum VerdictResult {
    /// No bits received yet
    #[serde(rename = "NO_DATA")]
    NoData,
    /// Fewer than `min_bits` received
    #[serde(rename = "RUNNING")]
    Running,
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "FAIL")]
    Fail,
}

impl VerdictResult {
    fn as_str(&self) -> &'static str {
        match self {
            VerdictResult::NoData => "NO DATA",
            VerdictResult::Running => "RUNNING",
            VerdictResult::Pass => "PASS",
            VerdictResult::Fail => "FAIL",
        }
    }
}

/// Pass or fail against the configured BER limit, once enough bits are in
#[derive(Debug, Clone, Serialize)]
pub struct T1Verdict {
    pub limit_percent: f64,
    pub min_bits: u64,
    /// Bits received as a share of `min_bits`, capped at 100
    pub settled_percent: f64,
    pub result: VerdictResult,
}

impl T1Verdict {
    pub fn new(cfg: &CfgT1Test, total: &T1Counters) -> Option<Self> {
        let limit_percent = cfg.ber_limit_percent?;
        let min_bits = cfg.min_bits.unwrap_or(0);
        let settled_percent = if min_bits == 0 {
            100.0
        } else {
            (total.bits as f64 / min_bits as f64 * 100.0).min(100.0)
        };
        let ber_percent = ratio(total.bit_errors, total.bits).map(|b| b * 100.0);
        let result = match ber_percent {
            None => VerdictResult::NoData,
            Some(_) if total.bits < min_bits => VerdictResult::Running,
            Some(b) if b <= limit_percent => VerdictResult::Pass,
            Some(_) => VerdictResult::Fail,
        };
        Some(Self {
            limit_percent,
            min_bits,
            settled_percent,
            result,
        })
    }
}

/// One report, for the console or for a script
#[derive(Debug, Clone, Serialize)]
pub struct T1Report {
    /// True for the report printed at the end of a run
    #[serde(rename = "final")]
    pub is_final: bool,
    pub elapsed_s: f64,
    pub state: LinkState,
    /// Uplink timeslots that are measured
    pub slots: Vec<u8>,
    /// Counts since the previous report
    pub interval: T1Stats,
    /// Counts since the start
    pub total: T1Stats,
    /// Present when `ber_limit_percent` is set
    pub verdict: Option<T1Verdict>,
}

impl T1Report {
    pub fn build(cfg: &CfgT1Test, elapsed_s: f64, state: LinkState, interval: &T1Counters, total: &T1Counters, is_final: bool) -> Self {
        Self {
            is_final,
            elapsed_s,
            state,
            slots: measured_slots(cfg),
            interval: T1Stats::new(interval),
            total: T1Stats::new(total),
            verdict: T1Verdict::new(cfg, total),
        }
    }

    /// The report in the configured output format
    pub fn render(&self, format: T1OutputFormat) -> String {
        match format {
            T1OutputFormat::Console => self.to_console_line(),
            T1OutputFormat::Json => serde_json::to_string(self).expect("T1Report serializes"),
        }
    }

    pub fn to_console_line(&self) -> String {
        let t = &self.total;
        let c = &t.counters;
        let h = &c.errors_per_burst;
        let slots: Vec<String> = self.slots.iter().map(|s| s.to_string()).collect();
        let mut line = format!(
            "{} | t {:>6.1}s | {} | slot {} | bursts exp {} det {} rx {} | bits {} errs {} | BER {} (last {}) | BLER {} | errs/burst [0:{} 1-4:{} 5-16:{} 17-64:{} >64:{}] | halves {}/{}",
            if self.is_final { "T1 FINAL" } else { "T1" },
            self.elapsed_s,
            self.state.as_str(),
            slots.join(","),
            c.expected_bursts,
            c.detected_bursts,
            c.received_bursts,
            c.bits,
            c.bit_errors,
            fmt_ber(t.ber),
            fmt_ber(self.interval.ber),
            fmt_percent(t.bler),
            h[0],
            h[1],
            h[2],
            h[3],
            h[4],
            c.errors_first_half,
            c.errors_second_half,
        );
        if let Some(v) = &self.verdict {
            line.push_str(&format!(
                " | limit {} settled {:.0}% {}",
                fmt_ber(Some(v.limit_percent / 100.0)),
                v.settled_percent,
                v.result.as_str()
            ));
        }
        line
    }
}

fn fmt_ber(v: Option<f64>) -> String {
    match v {
        Some(0.0) => "0".to_string(),
        Some(v) => format!("{:.2e}", v),
        None => "n/a".to_string(),
    }
}

fn fmt_percent(v: Option<f64>) -> String {
    v.map_or("n/a".to_string(), |v| format!("{:.1}%", v * 100.0))
}

/// Uplink timeslots that the config measures
pub fn measured_slots(cfg: &CfgT1Test) -> Vec<u8> {
    if cfg.measure_all_slots {
        vec![1, 2, 3, 4]
    } else {
        vec![cfg.ul_timeslot]
    }
}

/// State shared between the T1 entity and the main thread, so the final report can be printed after the stack stops
pub struct T1Shared {
    pub cfg: CfgT1Test,
    pub started: Instant,
    pub total: T1Counters,
    pub state: LinkState,
}

pub type T1SharedHandle = Arc<Mutex<T1Shared>>;

impl T1Shared {
    /// Report over the whole run
    pub fn final_report(&self) -> T1Report {
        T1Report::build(
            &self.cfg,
            self.started.elapsed().as_secs_f64(),
            self.state,
            &self.total,
            &self.total,
            true,
        )
    }

    /// Process exit code for scripts: 0 pass, 1 fail, 2 no usable measurement.
    /// Without a configured BER limit, any received bits count as a successful run.
    pub fn exit_code(&self) -> i32 {
        match T1Verdict::new(&self.cfg, &self.total) {
            Some(v) => match v.result {
                VerdictResult::Pass => 0,
                VerdictResult::Fail => 1,
                VerdictResult::NoData | VerdictResult::Running => 2,
            },
            None => {
                if self.total.bits > 0 {
                    0
                } else {
                    2
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fit(errors: u32, first: u32) -> BurstFit {
        BurstFit {
            phase: 0,
            errors,
            errors_first_half: first,
            errors_second_half: errors - first,
        }
    }

    fn counters() -> T1Counters {
        let mut c = T1Counters {
            expected_bursts: 12,
            detected_bursts: 11,
            ..Default::default()
        };
        for (e, f) in [(0, 0), (0, 0), (2, 1), (10, 4), (70, 30), (0, 0), (0, 0), (0, 0), (0, 0), (0, 0)] {
            c.add_received(&fit(e, f));
        }
        c
    }

    #[test]
    fn counters_and_derived_rates() {
        let c = counters();
        assert_eq!(c.received_bursts, 10);
        assert_eq!(c.bits, 4320);
        assert_eq!(c.bit_errors, 82);
        assert_eq!(c.error_bursts, 3);
        assert_eq!((c.errors_first_half, c.errors_second_half), (35, 47));
        assert_eq!(c.errors_per_burst, [7, 1, 1, 0, 1]);

        let s = T1Stats::new(&c);
        assert!((s.ber.unwrap() - 82.0 / 4320.0).abs() < 1e-12);
        assert!((s.bler.unwrap() - 0.3).abs() < 1e-12);
        assert!((s.detection_rate.unwrap() - 11.0 / 12.0).abs() < 1e-12);
        assert!((s.receive_rate.unwrap() - 10.0 / 12.0).abs() < 1e-12);
        assert!(T1Stats::new(&T1Counters::default()).ber.is_none());
    }

    #[test]
    fn counters_add() {
        let mut a = counters();
        a.add(&counters());
        assert_eq!((a.bits, a.bit_errors, a.expected_bursts), (8640, 164, 24));
        assert_eq!(a.errors_per_burst, [14, 2, 2, 0, 2]);
    }

    fn cfg(limit: Option<f64>, min_bits: Option<u64>) -> CfgT1Test {
        CfgT1Test {
            ber_limit_percent: limit,
            min_bits,
            ..Default::default()
        }
    }

    #[test]
    fn verdict_progression() {
        assert!(T1Verdict::new(&cfg(None, None), &counters()).is_none());

        // 82 errors in 4320 bits is a BER of 1.9 %
        let c = counters();
        let v = T1Verdict::new(&cfg(Some(2.0), Some(10_000)), &c).unwrap();
        assert_eq!(v.result, VerdictResult::Running);
        assert!((v.settled_percent - 43.2).abs() < 1e-9);

        assert_eq!(T1Verdict::new(&cfg(Some(2.0), Some(4000)), &c).unwrap().result, VerdictResult::Pass);
        assert_eq!(T1Verdict::new(&cfg(Some(1.5), Some(4000)), &c).unwrap().result, VerdictResult::Fail);
        assert_eq!(T1Verdict::new(&cfg(Some(1.5), None), &c).unwrap().result, VerdictResult::Fail);
        assert_eq!(
            T1Verdict::new(&cfg(Some(1.5), Some(4000)), &T1Counters::default()).unwrap().result,
            VerdictResult::NoData
        );
    }

    #[test]
    fn exit_codes() {
        let make = |limit, min_bits, total: T1Counters| T1Shared {
            cfg: cfg(limit, min_bits),
            started: Instant::now(),
            total,
            state: LinkState::Lock,
        };
        assert_eq!(make(Some(2.0), Some(4000), counters()).exit_code(), 0);
        assert_eq!(make(Some(1.0), Some(4000), counters()).exit_code(), 1);
        assert_eq!(make(Some(2.0), Some(10_000), counters()).exit_code(), 2);
        assert_eq!(make(None, None, counters()).exit_code(), 0);
        assert_eq!(make(None, None, T1Counters::default()).exit_code(), 2);
    }

    #[test]
    fn console_line_has_the_numbers() {
        let c = counters();
        let r = T1Report::build(&cfg(Some(2.0), Some(10_000)), 12.34, LinkState::Lock, &c, &c, false);
        let line = r.render(T1OutputFormat::Console);
        assert!(line.starts_with("T1 | t   12.3s | LOCK | slot 1 |"), "{}", line);
        assert!(line.contains("bursts exp 12 det 11 rx 10"), "{}", line);
        assert!(line.contains("bits 4320 errs 82"), "{}", line);
        assert!(line.contains("BER 1.90e-2"), "{}", line);
        assert!(line.contains("BLER 30.0%"), "{}", line);
        assert!(line.contains("[0:7 1-4:1 5-16:1 17-64:0 >64:1]"), "{}", line);
        assert!(line.contains("halves 35/47"), "{}", line);
        assert!(line.contains("limit 2.00e-2 settled 43% RUNNING"), "{}", line);

        let fin = T1Report::build(&cfg(None, None), 1.0, LinkState::NoSignal, &c, &c, true);
        assert!(fin.to_console_line().starts_with("T1 FINAL | t    1.0s | NO SIGNAL |"));
    }

    #[test]
    fn json_line_is_machine_readable() {
        let c = counters();
        let mut config = cfg(Some(2.0), Some(4000));
        config.measure_all_slots = true;
        let r = T1Report::build(&config, 5.0, LinkState::Search, &c, &c, true);
        let v: serde_json::Value = serde_json::from_str(&r.render(T1OutputFormat::Json)).unwrap();
        assert_eq!(v["final"], true);
        assert_eq!(v["state"], "SEARCH");
        assert_eq!(v["slots"], serde_json::json!([1, 2, 3, 4]));
        assert_eq!(v["total"]["bits"], 4320);
        assert_eq!(v["total"]["bit_errors"], 82);
        assert_eq!(v["total"]["errors_per_burst"], serde_json::json!([7, 1, 1, 0, 1]));
        assert!((v["total"]["ber"].as_f64().unwrap() - 82.0 / 4320.0).abs() < 1e-12);
        assert_eq!(v["verdict"]["result"], "PASS");
        assert_eq!(v["verdict"]["min_bits"], 4000);
        // No data: rates are null, not NaN
        let empty = T1Report::build(
            &config,
            0.0,
            LinkState::NoSignal,
            &T1Counters::default(),
            &T1Counters::default(),
            false,
        );
        let v: serde_json::Value = serde_json::from_str(&empty.render(T1OutputFormat::Json)).unwrap();
        assert!(v["total"]["ber"].is_null());
        assert_eq!(v["verdict"]["result"], "NO_DATA");
    }
}
