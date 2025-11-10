use soapysdr;
use crate::entities::phy::{components::dsp_types, traits::rxtx_dev::RxTxDevError};
use super::dsp_types::*;

type StreamType = ComplexSample;

pub enum Mode {
    Bs,
    Monitor,
}

struct SdrSettings<'a> {
    /// Name used to print which SDR was detected
    pub name: &'a str,
    /// Receive and transmit sample rate for TMO BS.
    pub fs_bs: f64,
    /// Receive and transmit sample rate for TMO monitor.
    /// The sample rate needs to be high enough to receive
    /// both downlink and uplink at the same time.
    pub fs_monitor: f64,
    /// Receive antenna
    pub rx_ant:   Option<&'a str>,
    /// Transmit antenna
    pub tx_ant:   Option<&'a str>,
    /// Receive gains
    pub rx_gain: &'a[(&'a str, f64)],
    /// Transmit gains
    pub tx_gain: &'a[(&'a str, f64)],
}


/// Default settings for any other SDR
const SDR_DEFAULTS: SdrSettings = SdrSettings {
    name: "Unknown SDR device",
    fs_bs: 512e3,
    fs_monitor: 16384e3,
    rx_ant: None,
    tx_ant: None,
    rx_gain: &[],
    tx_gain: &[],
};


/// Settings for LimeSDR
const SDR_SETTINGS_LIME: SdrSettings = SdrSettings {
    name: "LimeSDR",
    rx_ant: Some("LNAL"),
    tx_ant: Some("BAND1"),
    rx_gain: &[
        ("LNA", 20.0),
        ("TIA", 10.0),
        ("PGA", 10.0),
    ],
    tx_gain: &[
        ("PAD",  52.0),
        ("IAMP",  3.0),
    ],
    ..SDR_DEFAULTS
};


/// Settings for LimeSDR Mini v2
const SDR_SETTINGS_LIMEMINI_V2: SdrSettings = SdrSettings {
    name: "LimeSDR Mini v2",
    rx_ant: Some("LNAW"),
    tx_ant: Some("BAND2"),
    rx_gain: &[
        ("TIA", 6.0),
        ("LNA", 18.0),
        ("PGA", 0.0),
    ],
    tx_gain: &[
        ("PAD",  30.0),
        ("IAMP",  6.0),
    ],
    ..SDR_DEFAULTS
};

/// Settings for SXceiver
const SDR_SETTINGS_SX: SdrSettings = SdrSettings {
    name: "SXceiver",
    fs_bs: 600e3,
    fs_monitor: 600e3, // monitoring is not really possible with SXceiver
    rx_ant: Some("RX"),
    tx_ant: Some("TX"),
    rx_gain: &[
        ("LNA", 42.0),
        ("PGA", 16.0),
    ],
    tx_gain: &[
        ("DAC",    9.0),
        ("MIXER", 30.0),
    ],
};

/// Settings for Ettus UHD USRP B200/B210
const SDR_SETTINGS_USRP_B200: SdrSettings = SdrSettings {
    name: "USRP B200/B210",
    rx_ant: Some("TX/RX"),
    tx_ant: Some("TX/RX"),
    
    rx_gain: &[
        ("PGA", 50.0),  // 0 - 76 dB
    ],
    tx_gain: &[
        ("PGA", 35.0),  // 0 - 76 dB
    ],
    ..SDR_DEFAULTS
};


pub struct RxResult {
    /// Number of samples read
    pub len: usize,
    /// Sample counter for the first sample read
    pub count: dsp_types::SampleCount,
}

pub struct SoapyIo {
    rx_ch:  usize,
    tx_ch:  usize,
    rx_fs: f64,
    tx_fs: f64,
    /// Timestamp for the first sample read from SDR.
    /// This is subtracted from all following timestamps,
    /// so that sample counter starts from 0 even if timestamp does not.
    initial_time: Option<i64>,
    rx_next_count: SampleCount,

    /// If false, timestamp of latest RX read is used to estimate
    /// current hardware time. This is used in case get_hardware_time
    /// is unacceptably slow, particularly with SoapyRemote.
    use_get_hardware_time: bool,

    dev: soapysdr::Device,
    /// Receive stream. None if receiving is disabled.
    rx:  Option<soapysdr::RxStream<StreamType>>,
    /// Transmit stream. None if transmitting is disabled.
    tx:  Option<soapysdr::TxStream<StreamType>>,
}

/// It is annoying to repeat error handling so do that in a macro.
/// ? could be used but then it could not print which SoapySDR call failed.
macro_rules! soapycheck {
    ($text:literal, $soapysdr_call:expr) => {
        match $soapysdr_call {
            Ok(ret) => { ret },
            Err(err) => {
                tracing::error!("SoapySDR: Failed to {}: {}", $text, err);
                return Err(err);
            }
        }
    }
}

impl SoapyIo {
    pub fn new(
        dev_args_str: &[(&str, &str)],
        rx_freq: Option<f64>,
        tx_freq: Option<f64>,
        mode: Mode
    ) -> Result<Self, soapysdr::Error> {
        let rx_ch = 0;
        let tx_ch = 0;
        let mut use_get_hardware_time = true;

        let mut dev_args = soapysdr::Args::new();
        for (key, value) in dev_args_str {
            dev_args.set(*key, *value);

            // get_hardware_time tends to be unacceptably slow
            // over SoapyRemote, so do not use it.
            // Maybe this is not a reliably way to detect use of SoapyRemote
            // in case SoapySDR selects it by default, but I do not know
            // a better way to detect it.
            if *key == "driver" && *value == "remote" {
                use_get_hardware_time = false;
            }
        }

        let dev = soapycheck!("open SoapySDR device",
            soapysdr::Device::new(dev_args));

        let rx_enabled = rx_freq.is_some();
        let tx_enabled = tx_freq.is_some();

        let sdr_settings = match(
            dev.driver_key()  .unwrap_or("".to_string()).as_str(),
            dev.hardware_key().unwrap_or("".to_string()).as_str()
        ) {
            (_, "LimeSDR-USB") => &SDR_SETTINGS_LIME,
            (_, "LimeSDR-Mini_v2") => &SDR_SETTINGS_LIMEMINI_V2,

            ("sx", _) => &SDR_SETTINGS_SX,

            ("b200", _) => &SDR_SETTINGS_USRP_B200,
            
            (_, _) => &SDR_DEFAULTS,
        };
        
        tracing::info!("Got driver key {} hardware_key {}, selecting settings for {}", 
                dev.driver_key()  .unwrap_or("-".to_string()).as_str(),
                dev.hardware_key().unwrap_or("-".to_string()).as_str(),
                sdr_settings.name);

        let fs = match mode {
            Mode::Bs => sdr_settings.fs_bs,
            Mode::Monitor => sdr_settings.fs_monitor,
        };
        let mut rx_fs: f64 = 0.0;
        if rx_enabled {
            soapycheck!("set RX sample rate",
                dev.set_sample_rate(soapysdr::Direction::Rx, rx_ch, fs));
            // Read the actual sample rate obtained and store it
            // to avoid having to read it again every time it is needed.
            rx_fs = soapycheck!("get RX sample rate",
                dev.sample_rate(soapysdr::Direction::Rx, rx_ch));
        }
        let mut tx_fs: f64 = 0.0;
        if tx_enabled {
            soapycheck!("set TX sample rate",
                dev.set_sample_rate(soapysdr::Direction::Tx, tx_ch, fs));
            tx_fs = soapycheck!("get TX sample rate",
                dev.sample_rate(soapysdr::Direction::Tx, tx_ch));
        }

        if rx_enabled {
            // If rx_enabled is true, we already know sdr_rx_freq is not None,
            // so unwrap is fine here.
            soapycheck!("set RX center frequency",
            dev.set_frequency(soapysdr::Direction::Rx, rx_ch, rx_freq.unwrap(), soapysdr::Args::new()));

            if let Some(ant) = sdr_settings.rx_ant{
                soapycheck!("set RX antenna",
                    dev.set_antenna(soapysdr::Direction::Rx, rx_ch, ant));
            }

            for (name, gain) in sdr_settings.rx_gain {
                soapycheck!("set RX gain",
                    dev.set_gain_element(soapysdr::Direction::Rx, rx_ch, *name, *gain));
            }
        }

        if tx_enabled {
            soapycheck!("set TX center frequency",
            dev.set_frequency(soapysdr::Direction::Tx, tx_ch, tx_freq.unwrap(), soapysdr::Args::new()));

            if let Some(ant) = sdr_settings.tx_ant {
                soapycheck!("set TX antenna",
                dev.set_antenna(soapysdr::Direction::Tx, tx_ch, ant));
            }

            for (name, gain) in sdr_settings.tx_gain {
                soapycheck!("set TX gain",
                    dev.set_gain_element(soapysdr::Direction::Tx, tx_ch, *name, *gain));
            }
        }

        // TODO: add stream arguments to SdrSettings.
        // Maybe they should be different for BS and monitor modes.
        // For example, the latency argument with LimeSDR should probably
        // be set for minimum latency for TMO BS
        // but for maximum throughput for TMO monitor.
        let mut rx_args = soapysdr::Args::new();
        let tx_args = soapysdr::Args::new();
        // hack to test the idea above, TODO properly
        match mode {
            Mode::Bs => {
                // Minimize latency
                rx_args.set("latency", "0");
            },
            Mode::Monitor => {
                // Maximize throughput with high sample rates
                rx_args.set("latency", "1");
            }
        };

        let mut rx = if rx_enabled {
            Some(soapycheck!("setup RX stream",
                dev.rx_stream_args(&[rx_ch], rx_args)))
        } else {
            None
        };
        let mut tx = if tx_enabled {
            Some(soapycheck!("setup TX stream",
                dev.tx_stream_args(&[tx_ch], tx_args)))
        } else {
            None
        };
        if let Some(rx) = &mut rx {
            soapycheck!("activate RX stream",
                rx.activate(None));
        }
        if let Some(tx) = &mut tx {
            soapycheck!("activate TX stream",
                tx.activate(None));
        }
        Ok(Self {
            rx_ch,
            tx_ch,
            rx_fs,
            tx_fs,
            initial_time: None,
            rx_next_count: 0,
            use_get_hardware_time,
            dev,
            rx,
            tx,
        })
    }

    pub fn receive(&mut self, buffer: &mut [StreamType]) -> Result<RxResult, RxTxDevError> {
        if let Some(rx) = &mut self.rx {
            match rx.read_with_stream_result(&mut [buffer], soapysdr::StreamFlags::default(), 1000000) {
                Ok(result) => {
                    if self.initial_time.is_none() {
                        if let Some(time) = result.time {
                            // If the first read did not include a timestamp but a following one does,
                            // compute actual initial timestamp value
                            // from the number of samples received so far.
                            self.initial_time = Some(time - soapysdr::ticks_to_time_ns(self.rx_next_count, self.rx_fs));
                            tracing::trace!("Set initial_time to {} ns", self.initial_time.unwrap());
                        };
                    }
                    let count = if let Some(time) = result.time {
                        // unwrap is fine since self.initial_time is already Some at this point.
                        soapysdr::time_ns_to_ticks(time - self.initial_time.unwrap(), self.rx_fs)
                    } else {
                        self.rx_next_count
                    };
                    // Store expected sample count for the next sample to be read.
                    // This is used in case timestamp is missing.
                    self.rx_next_count = count + result.len as SampleCount;
                    Ok(RxResult {
                        len: result.len,
                        count,
                    })
                },
                Err(_) => Err(RxTxDevError::RxReadError),
            }
        } else {
            // RX is disabled
            Err(RxTxDevError::RxReadError)
        }
    }

    pub fn transmit(&mut self, buffer: &[StreamType], count: Option<SampleCount>) -> Result<(), RxTxDevError> {
        if let Some(tx) = &mut self.tx {
            if let Some(initial_time) = self.initial_time {
                tx.write_all(&[buffer],
                    count.map(|count|
                        initial_time + soapysdr::ticks_to_time_ns(count, self.tx_fs)
                    ),
                    false, 1000000
                ).map_err(|_| RxTxDevError::RxReadError)
            } else {
                // initial_time is not available, so TX is not possible yet
                Err(RxTxDevError::RxReadError)
            }
        } else {
            // TX is disabled
            Err(RxTxDevError::RxReadError)
        }
    }

    pub fn current_time(&self) -> Result<i64, RxTxDevError> {
        self.dev.get_hardware_time(None).map_err(|_| RxTxDevError::RxReadError)
    }

    /// Current hardware time as RX sample count
    pub fn rx_current_count(&self) -> Result<SampleCount, RxTxDevError> {
        if !self.rx_enabled() { return Ok(0); }
        if self.use_get_hardware_time {
            Ok(soapysdr::time_ns_to_ticks(
                self.current_time()? - self.initial_time.unwrap_or(0),
                self.rx_fs
            ))
        } else {
            Ok(self.rx_next_count - 1)
        }
    }

    /// Current hardware time as TX sample count
    pub fn tx_current_count(&self) -> Result<SampleCount, RxTxDevError> {
        if !self.tx_enabled() { return Ok(0); }
        if self.use_get_hardware_time {
            Ok(soapysdr::time_ns_to_ticks(
                self.current_time()? - self.initial_time.unwrap_or(0),
                self.tx_fs
            ))
        } else {
            // Assumes equal RX and TX sample rates
            // and does not work if RX is disabled.
            // This is not a problem right now but could be fixed if needed.
            Ok(self.rx_next_count - 1)
        }
    }

    pub fn tx_possible(&self) -> bool {
        // initial_time is obtained from the first RX read (that includes a timestamp),
        // so prevent TX before it is available.
        self.tx_enabled() && self.initial_time.is_some()
    }

    pub fn rx_sample_rate(&self) -> f64 {
        self.rx_fs
    }

    pub fn tx_sample_rate(&self) -> f64 {
        self.tx_fs
    }

    pub fn rx_center_frequency(&self) -> Result<f64, soapysdr::Error> {
        self.dev.frequency(soapysdr::Direction::Rx, self.rx_ch)
    }

    pub fn tx_center_frequency(&self) -> Result<f64, soapysdr::Error> {
        self.dev.frequency(soapysdr::Direction::Tx, self.tx_ch)
    }

    pub fn rx_enabled(&self) -> bool {
        self.rx.is_some()
    }

    pub fn tx_enabled(&self) -> bool {
        self.tx.is_some()
    }
}
