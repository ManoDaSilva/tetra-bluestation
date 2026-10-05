use std::collections::HashMap;

use serde::Deserialize;
use toml::Value;

/// Format of the periodic report lines
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum T1OutputFormat {
    /// One human readable line per interval
    Console,
    /// One JSON object per line, for scripts
    Json,
}

/// Settings for the BS T1 test mode (`stack_mode = "BsT1"`).
#[derive(Debug, Clone)]
pub struct CfgT1Test {
    /// Interval between report lines, in milliseconds
    pub report_interval_ms: u32,
    /// Uplink timeslot (1-4) on which the test set transmits its T1 signal
    pub ul_timeslot: u8,
    /// Also measure on the other three timeslots. The test set sends the same type 7 signal there,
    /// so this gives four times the bits per second
    pub measure_all_slots: bool,
    /// Stop after this many seconds. Runs until Ctrl+C if unset
    pub duration_s: Option<u32>,
    /// Report format
    pub output: T1OutputFormat,
    /// BER limit in percent for a pass or fail verdict. EN 300 394-1 Table A.5 lists limits per test case
    pub ber_limit_percent: Option<f64>,
    /// Number of bits to measure before the verdict counts. Table A.5 lists the minimum per test case
    pub min_bits: Option<u64>,
}

impl Default for CfgT1Test {
    fn default() -> Self {
        Self {
            report_interval_ms: 1000,
            ul_timeslot: 1,
            measure_all_slots: false,
            duration_s: None,
            output: T1OutputFormat::Console,
            ber_limit_percent: None,
            min_bits: None,
        }
    }
}

#[derive(Deserialize)]
pub struct CfgT1TestDto {
    pub report_interval_ms: Option<u32>,
    pub ul_timeslot: Option<u8>,
    pub measure_all_slots: Option<bool>,
    pub duration_s: Option<u32>,
    pub output: Option<T1OutputFormat>,
    pub ber_limit_percent: Option<f64>,
    pub min_bits: Option<u64>,

    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

pub fn apply_t1test_patch(src: CfgT1TestDto) -> CfgT1Test {
    let mut dst = CfgT1Test::default();
    if let Some(v) = src.report_interval_ms {
        dst.report_interval_ms = v;
    }
    if let Some(v) = src.ul_timeslot {
        dst.ul_timeslot = v;
    }
    if let Some(v) = src.measure_all_slots {
        dst.measure_all_slots = v;
    }
    if let Some(v) = src.output {
        dst.output = v;
    }
    dst.duration_s = src.duration_s;
    dst.ber_limit_percent = src.ber_limit_percent;
    dst.min_bits = src.min_bits;
    dst
}
