use std::collections::HashMap;

use serde::Deserialize;
use toml::Value;

/// Settings for the BS T1 test mode (`stack_mode = "BsT1"`).
#[derive(Debug, Clone)]
pub struct CfgT1Test {
    /// Interval between console report lines, in milliseconds
    pub report_interval_ms: u32,
    /// Uplink timeslot (1-4) on which the test set transmits its T1 signal
    pub ul_timeslot: u8,
    /// Stop after this many seconds. Runs until Ctrl+C if unset
    pub duration_s: Option<u32>,
}

impl Default for CfgT1Test {
    fn default() -> Self {
        Self {
            report_interval_ms: 1000,
            ul_timeslot: 1,
            duration_s: None,
        }
    }
}

#[derive(Deserialize)]
pub struct CfgT1TestDto {
    pub report_interval_ms: Option<u32>,
    pub ul_timeslot: Option<u8>,
    pub duration_s: Option<u32>,

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
    dst.duration_s = src.duration_s;
    dst
}
