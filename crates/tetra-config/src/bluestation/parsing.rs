use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use serde::Deserialize;
use toml::Value;

use crate::bluestation::{CellInfoDto, CfgControlDto, NetInfoDto, apply_control_patch, cell_dto_to_cfg, net_dto_to_cfg};

use super::config::{StackConfig, StackMode};

/// Stack modes as written in the config file. `BsT1` is not a stack mode for the rest of the stack;
/// it is parsed into `StackMode::Bs` plus a `t1_test` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
enum StackModeDto {
    Bs,
    Ms,
    Mon,
    BsT1,
}
use super::sec_brew::{CfgBrewDto, apply_brew_patch};
use super::sec_t1test::{CfgT1TestDto, apply_t1test_patch};
use super::sec_telemetry::{CfgTelemetryDto, apply_telemetry_patch};
use super::{PhyIoDto, phy_dto_to_cfg};

/// Build `StackConfig` from a TOML configuration file
pub fn from_toml_str(toml_str: &str) -> Result<StackConfig, Box<dyn std::error::Error>> {
    let root: TomlConfigRoot = toml::from_str(toml_str)?;

    // Various sanity checks
    let expected_config_version = "0.6";
    if !root.config_version.eq(expected_config_version) {
        return Err(format!(
            "Unrecognized config_version: {}, expect {}",
            root.config_version, expected_config_version
        )
        .into());
    }
    if !root.extra.is_empty() {
        return Err(format!("Unrecognized top-level fields: {:?}", sorted_keys(&root.extra)).into());
    }

    if !root.phy_io.extra.is_empty() {
        return Err(format!("Unrecognized fields: phy_io::{:?}", sorted_keys(&root.phy_io.extra)).into());
    }
    if let Some(ref soapy) = root.phy_io.soapysdr {
        let extra_keys = sorted_keys(&soapy.extra);
        let extra_keys_filtered = extra_keys
            .iter()
            .filter(|key| !(key.starts_with("rx_gain_") || key.starts_with("tx_gain_")))
            .collect::<Vec<&&str>>();
        if !extra_keys_filtered.is_empty() {
            return Err(format!("Unrecognized fields: phy_io.soapysdr::{:?}", extra_keys_filtered).into());
        }
    }
    if !root.net_info.extra.is_empty() {
        return Err(format!("Unrecognized fields in net_info: {:?}", sorted_keys(&root.net_info.extra)).into());
    }
    if !root.cell_info.extra.is_empty() {
        return Err(format!("Unrecognized fields in cell_info: {:?}", sorted_keys(&root.cell_info.extra)).into());
    }

    // Optional brew section
    if let Some(ref brew) = root.brew {
        if !brew.extra.is_empty() {
            return Err(format!("Unrecognized fields in brew config: {:?}", sorted_keys(&brew.extra)).into());
        }
    }

    // Optional telemetry section
    if let Some(ref telemetry) = root.telemetry {
        if !telemetry.extra.is_empty() {
            return Err(format!("Unrecognized fields in telemetry config: {:?}", sorted_keys(&telemetry.extra)).into());
        }
    }

    // Optional t1_test section
    if root.t1_test.is_some() && root.stack_mode != StackModeDto::BsT1 {
        return Err("t1_test section is only valid with stack_mode = \"BsT1\"".into());
    }
    if let Some(ref t1) = root.t1_test {
        if !t1.extra.is_empty() {
            return Err(format!("Unrecognized fields in t1_test config: {:?}", sorted_keys(&t1.extra)).into());
        }
    }

    // Build config from required and optional values
    let mut cfg = StackConfig {
        stack_mode: match root.stack_mode {
            StackModeDto::Bs | StackModeDto::BsT1 => StackMode::Bs,
            StackModeDto::Ms => StackMode::Ms,
            StackModeDto::Mon => StackMode::Mon,
        },
        debug_log: root.debug_log,
        phy_io: phy_dto_to_cfg(root.phy_io),
        net: net_dto_to_cfg(root.net_info),
        cell: cell_dto_to_cfg(root.cell_info),
        brew: None,
        telemetry: None,
        control: None,
        t1_test: None,
    };

    if let Some(brew) = root.brew {
        cfg.brew = Some(apply_brew_patch(brew));
    }

    if let Some(telemetry) = root.telemetry {
        cfg.telemetry = Some(apply_telemetry_patch(telemetry)?);
    }

    if root.stack_mode == StackModeDto::BsT1 {
        // The section is optional in T1 mode; defaults apply when absent
        cfg.t1_test = Some(root.t1_test.map(apply_t1test_patch).unwrap_or_default());
    }

    if let Some(command) = root.command {
        cfg.control = Some(apply_control_patch(command)?);
    }

    Ok(cfg)
}

/// Build `SharedConfig` from any reader.
pub fn from_reader<R: Read>(reader: R) -> Result<StackConfig, Box<dyn std::error::Error>> {
    let mut contents = String::new();
    let mut reader = BufReader::new(reader);
    reader.read_to_string(&mut contents)?;
    from_toml_str(&contents)
}

/// Build `SharedConfig` from a file path.
pub fn from_file<P: AsRef<Path>>(path: P) -> Result<StackConfig, Box<dyn std::error::Error>> {
    let f = File::open(path)?;
    let r = BufReader::new(f);
    let cfg = from_reader(r)?;
    Ok(cfg)
}

fn sorted_keys(map: &HashMap<String, Value>) -> Vec<&str> {
    let mut v: Vec<&str> = map.keys().map(|s| s.as_str()).collect();
    v.sort_unstable();
    v
}

/// ----------------------- DTOs for input shape -----------------------

#[derive(Deserialize)]
struct TomlConfigRoot {
    config_version: String,
    stack_mode: StackModeDto,
    debug_log: Option<String>,

    phy_io: PhyIoDto,
    net_info: NetInfoDto,
    cell_info: CellInfoDto,

    brew: Option<CfgBrewDto>,
    telemetry: Option<CfgTelemetryDto>,
    command: Option<CfgControlDto>,
    t1_test: Option<CfgT1TestDto>,

    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const T1_CONFIG: &str = include_str!("../../../../example_config/config_t1.toml");

    #[test]
    fn t1_example_config_parses_to_bs_with_t1_section() {
        let cfg = from_toml_str(T1_CONFIG).unwrap();
        assert_eq!(cfg.stack_mode, StackMode::Bs);
        let t1 = cfg.t1_test.as_ref().expect("t1_test must be set");
        assert_eq!(t1.report_interval_ms, 1000);
        assert_eq!(t1.ul_timeslot, 1);
        assert_eq!(t1.duration_s, None);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn t1_section_is_optional_in_t1_mode() {
        let toml = T1_CONFIG.replace("[t1_test]", "[unused_removed]");
        // Section removed entirely
        let start = toml.find("[unused_removed]").unwrap();
        let end = toml.find("[phy_io]").unwrap();
        let toml = format!("{}{}", &toml[..start], &toml[end..]);
        let cfg = from_toml_str(&toml).unwrap();
        assert_eq!(cfg.t1_test.unwrap().report_interval_ms, 1000);
    }

    #[test]
    fn t1_section_rejected_without_t1_mode() {
        let toml = T1_CONFIG.replace("stack_mode = \"BsT1\"", "stack_mode = \"Bs\"");
        assert!(from_toml_str(&toml).is_err());
    }

    #[test]
    fn t1_mode_rejects_network_features() {
        let toml = format!("{}\n[telemetry]\nhost = \"localhost\"\nport = 1234\n", T1_CONFIG);
        let cfg = from_toml_str(&toml).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn t1_mode_rejects_bad_values() {
        let cfg = from_toml_str(&T1_CONFIG.replace("ul_timeslot = 1", "ul_timeslot = 5")).unwrap();
        assert!(cfg.validate().is_err());
        let cfg = from_toml_str(&T1_CONFIG.replace("report_interval_ms = 1000", "report_interval_ms = 10")).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn t1_mode_rejects_unknown_fields() {
        assert!(from_toml_str(&T1_CONFIG.replace("ul_timeslot = 1", "ul_timeslot = 1\nbogus = 2")).is_err());
    }
}
