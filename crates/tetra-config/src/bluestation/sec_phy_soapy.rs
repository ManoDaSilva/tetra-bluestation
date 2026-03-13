use serde::Deserialize;
use std::collections::HashMap;
use toml::Value;

/// Configuration for different SDR hardware devices
#[derive(Debug, Clone)]
pub struct SoapySdrIoCfg {
    /// USRP B2xx series configuration (B200, B210)
    pub iocfg_usrpb2xx: Option<CfgUsrpB2xx>,

    /// LimeSDR configuration
    pub iocfg_limesdr: Option<CfgLimeSdr>,

    /// SXceiver configuration
    pub iocfg_sxceiver: Option<CfgSxCeiver>,
}

impl SoapySdrIoCfg {
    pub fn get_soapy_driver_name(&self) -> &'static str {
        if self.iocfg_usrpb2xx.is_some() {
            "uhd"
        } else if self.iocfg_limesdr.is_some() {
            "lime"
        } else if self.iocfg_sxceiver.is_some() {
            "sx"
        } else {
            "unknown"
        }
    }
}

impl Default for SoapySdrIoCfg {
    fn default() -> Self {
        Self {
            iocfg_usrpb2xx: None,
            iocfg_limesdr: None,
            iocfg_sxceiver: None,
        }
    }
}

/// Configuration for Ettus USRP B2xx series
#[derive(Debug, Clone, Deserialize)]
pub struct CfgUsrpB2xx {
    pub rx_ant: Option<String>,
    pub tx_ant: Option<String>,
    pub rx_gain_pga: Option<f64>,
    pub tx_gain_pga: Option<f64>,
}

/// Configuration for LimeSDR
#[derive(Debug, Clone, Deserialize)]
pub struct CfgLimeSdr {
    pub rx_ant: Option<String>,
    pub tx_ant: Option<String>,
    pub rx_gain_lna: Option<f64>,
    pub rx_gain_tia: Option<f64>,
    pub rx_gain_pga: Option<f64>,
    pub tx_gain_pad: Option<f64>,
    pub tx_gain_iamp: Option<f64>,
}

/// Configuration for SXceiver
#[derive(Debug, Clone, Deserialize)]
pub struct CfgSxCeiver {
    pub rx_ant: Option<String>,
    pub tx_ant: Option<String>,
    pub rx_gain_lna: Option<f64>,
    pub rx_gain_pga: Option<f64>,
    pub tx_gain_dac: Option<f64>,
    pub tx_gain_mixer: Option<f64>,
}

/// Polynomial pre-distortion configuration for PA linearisation.
///
/// The predistorter applies a memoryless, magnitude-based polynomial correction
/// to each complex baseband sample immediately before it is handed to the SDR:
///
///   y[n] = x[n] * P(|x[n]|)
///
/// where P(r) = c0 + c1*r + c2*r^2 + c3*r^3 + ...
///
/// The polynomial is evaluated in `|x|` (instantaneous sample magnitude)
/// using Horner's method.  Only the magnitude axis is corrected; the phase
/// of each sample is preserved.
///
/// **Pass-through (identity):** coefficients = [1.0]  →  y = x * 1.0
///
/// **Typical use:** measure your PA's AM/AM compression curve, fit the inverse
/// polynomial, and enter those coefficients here.  A mildly compressive PA
/// often needs something like [1.0, 0.0, -0.15, 0.05].
///
/// Omit the `[phy_io.soapysdr.predistortion]` section entirely to disable DPD
/// (equivalent to coefficients = [1.0] but with zero runtime overhead).
#[derive(Debug, Clone, Deserialize)]
pub struct CfgPredistortion {
    /// Polynomial coefficients [c0, c1, c2, ...].
    /// Must contain at least one element.
    /// c0 = 1.0 with all others 0.0 is a unity pass-through.
    pub coefficients: Vec<f32>,
}

impl Default for CfgPredistortion {
    fn default() -> Self {
        // Identity: y = x * 1.0
        Self {
            coefficients: vec![1.0],
        }
    }
}

/// SoapySDR configuration
#[derive(Debug, Clone)]
pub struct CfgSoapySdr {
    /// Uplink frequency in Hz
    pub ul_freq: f64,
    /// Downlink frequency in Hz
    pub dl_freq: f64,
    /// PPM frequency error correction
    pub ppm_err: f64,
    /// Hardware-specific I/O configuration
    pub io_cfg: SoapySdrIoCfg,
    /// Optional polynomial pre-distortion for PA linearisation.
    /// None means DPD is disabled (samples pass through unmodified).
    pub predistortion: Option<CfgPredistortion>,
}

impl CfgSoapySdr {
    /// Get corrected UL frequency with PPM error applied
    pub fn ul_freq_corrected(&self) -> (f64, f64) {
        let ppm = self.ppm_err;
        let err = (self.ul_freq / 1_000_000.0) * ppm;
        (self.ul_freq + err, err)
    }

    /// Get corrected DL frequency with PPM error applied
    pub fn dl_freq_corrected(&self) -> (f64, f64) {
        let ppm = self.ppm_err;
        let err = (self.dl_freq / 1_000_000.0) * ppm;
        (self.dl_freq + err, err)
    }
}

#[derive(Deserialize)]
pub struct SoapySdrDto {
    pub rx_freq: f64,
    pub tx_freq: f64,
    pub ppm_err: Option<f64>,

    pub iocfg_usrpb2xx: Option<UsrpB2xxDto>,
    pub iocfg_limesdr: Option<LimeSdrDto>,
    pub iocfg_sxceiver: Option<SXceiverDto>,

    /// Optional polynomial pre-distortion section.
    pub predistortion: Option<CfgPredistortion>,

    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Deserialize)]
pub struct UsrpB2xxDto {
    pub rx_ant: Option<String>,
    pub tx_ant: Option<String>,
    pub rx_gain_pga: Option<f64>,
    pub tx_gain_pga: Option<f64>,
}

#[derive(Deserialize)]
pub struct LimeSdrDto {
    pub rx_ant: Option<String>,
    pub tx_ant: Option<String>,
    pub rx_gain_lna: Option<f64>,
    pub rx_gain_tia: Option<f64>,
    pub rx_gain_pga: Option<f64>,
    pub tx_gain_pad: Option<f64>,
    pub tx_gain_iamp: Option<f64>,
}

#[derive(Deserialize)]
pub struct SXceiverDto {
    pub rx_ant: Option<String>,
    pub tx_ant: Option<String>,
    pub rx_gain_lna: Option<f64>,
    pub rx_gain_pga: Option<f64>,
    pub tx_gain_dac: Option<f64>,
    pub tx_gain_mixer: Option<f64>,
}
