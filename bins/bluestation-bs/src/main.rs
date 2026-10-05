use clap::Parser;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use tetra_core::tetra_entities::TetraEntity;
use tetra_entities::net_control::channel::build_all_control_links;
use tetra_entities::net_control::{
    CONTROL_HEARTBEAT_INTERVAL, CONTROL_HEARTBEAT_TIMEOUT, CONTROL_PROTOCOL_VERSION, CommandDispatcher, ControlWorker,
};

use tetra_config::bluestation::{PhyBackend, SharedConfig, StackConfig, parsing};
use tetra_core::{TdmaTime, debug};
use tetra_entities::MessageRouter;
use tetra_entities::net_brew::entity::BrewEntity;
use tetra_entities::net_brew::new_websocket_transport;
use tetra_entities::net_telemetry::worker::TelemetryWorker;
use tetra_entities::net_telemetry::{
    TELEMETRY_HEARTBEAT_INTERVAL, TELEMETRY_HEARTBEAT_TIMEOUT, TELEMETRY_PROTOCOL_VERSION, TelemetrySource, telemetry_channel,
};
use tetra_entities::network::transports::websocket::{WebSocketTransport, WebSocketTransportConfig};
use tetra_entities::{
    cmce::cmce_bs::CmceBs,
    llc::llc_bs_ms::Llc,
    lmac::lmac_bs::LmacBs,
    mle::mle_bs::MleBs,
    mm::mm_bs::MmBs,
    phy::{components::soapy_dev::RxTxDevSoapySdr, phy_bs::PhyBs},
    sndcp::sndcp_bs::Sndcp,
    t1test::{T1TestBs, report::T1SharedHandle},
    umac::umac_bs::UmacBs,
};

/// Load configuration file
fn load_config_from_toml(cfg_path: &str) -> StackConfig {
    match parsing::from_file(cfg_path) {
        Ok(c) => c,
        Err(e) => {
            println!("Failed to load configuration from {}: {}", cfg_path, e);
            std::process::exit(1);
        }
    }
}

fn start_telemetry_worker(cfg: SharedConfig, telemetry_source: TelemetrySource) -> thread::JoinHandle<()> {
    let config = cfg.config();
    let tcfg = config.telemetry.as_ref().unwrap();

    let custom_root_certs = tcfg.ca_cert.as_ref().map(|path| {
        let der_bytes = std::fs::read(path).unwrap_or_else(|e| {
            eprintln!("Failed to read CA certificate from '{}': {}", path, e);
            std::process::exit(1);
        });
        vec![rustls::pki_types::CertificateDer::from(der_bytes)]
    });

    let ws_config = WebSocketTransportConfig {
        host: tcfg.host.clone(),
        port: tcfg.port,
        use_tls: tcfg.use_tls,
        digest_auth_credentials: None,
        basic_auth_credentials: tcfg.credentials.clone(),
        endpoint_path: "/".to_string(),
        subprotocol: Some(TELEMETRY_PROTOCOL_VERSION.to_string()),
        user_agent: format!("BlueStation/{}", tetra_core::STACK_VERSION),
        heartbeat_interval: TELEMETRY_HEARTBEAT_INTERVAL,
        heartbeat_timeout: TELEMETRY_HEARTBEAT_TIMEOUT,
        custom_root_certs,
        extra_headers: Vec::new(),
    };

    thread::spawn(move || {
        let transport = WebSocketTransport::new(ws_config);
        let mut worker = TelemetryWorker::new(telemetry_source, transport);
        worker.run();
    })
}

fn start_control_worker(cfg: SharedConfig, command_dispatchers: HashMap<TetraEntity, CommandDispatcher>) -> thread::JoinHandle<()> {
    let config = cfg.config();
    let ccfg = config.control.as_ref().unwrap();

    let custom_root_certs = ccfg.ca_cert.as_ref().map(|path| {
        let der_bytes = std::fs::read(path).unwrap_or_else(|e| {
            eprintln!("Failed to read CA certificate from '{}': {}", path, e);
            std::process::exit(1);
        });
        vec![rustls::pki_types::CertificateDer::from(der_bytes)]
    });

    let ws_config = WebSocketTransportConfig {
        host: ccfg.host.clone(),
        port: ccfg.port,
        use_tls: ccfg.use_tls,
        digest_auth_credentials: None,
        basic_auth_credentials: ccfg.credentials.clone(),
        endpoint_path: "/".to_string(),
        subprotocol: Some(CONTROL_PROTOCOL_VERSION.to_string()),
        user_agent: format!("BlueStation/{}", tetra_core::STACK_VERSION),
        heartbeat_interval: CONTROL_HEARTBEAT_INTERVAL,
        heartbeat_timeout: CONTROL_HEARTBEAT_TIMEOUT,
        custom_root_certs,
        extra_headers: Vec::new(),
    };

    thread::spawn(move || {
        let transport = WebSocketTransport::new(ws_config);
        let mut worker = ControlWorker::new(command_dispatchers, transport);
        worker.run();
    })
}

/// Start base station stack
fn build_bs_stack(cfg: &mut SharedConfig) -> (MessageRouter, Option<TelemetrySource>, HashMap<TetraEntity, CommandDispatcher>) {
    let mut router = MessageRouter::new(cfg.clone());

    // Add suitable Phy component based on PhyIo type
    match cfg.config().phy_io.backend {
        PhyBackend::SoapySdr => {
            let rxdev = RxTxDevSoapySdr::new(cfg);
            let phy = PhyBs::new(cfg.clone(), rxdev);
            router.register_entity(Box::new(phy));
        }
        _ => {
            panic!("Unsupported PhyIo type: {:?}", cfg.config().phy_io.backend);
        }
    }

    // Build telemetry sink/source, if enabled
    let (tsink, tsource) = if cfg.config().telemetry.is_some() {
        let (a, b) = telemetry_channel();
        (Some(a), Some(b))
    } else {
        (None, None)
    };

    // Build control links, if enabled
    let (mut c_d, mut c_e) = if cfg.config().control.is_some() {
        build_all_control_links()
    } else {
        (HashMap::new(), HashMap::new())
    };

    // Add remaining components
    let lmac = LmacBs::new(cfg.clone());
    let umac = UmacBs::new(cfg.clone());
    let llc = Llc::new(cfg.clone());
    let mle = MleBs::new(cfg.clone());
    let mm = MmBs::new(cfg.clone(), tsink.clone(), c_e.remove(&TetraEntity::Mm));
    let sndcp = Sndcp::new(cfg.clone());
    let cmce = CmceBs::new(cfg.clone(), tsink.clone(), c_e.remove(&TetraEntity::Cmce));
    router.register_entity(Box::new(lmac));
    router.register_entity(Box::new(umac));
    router.register_entity(Box::new(llc));
    router.register_entity(Box::new(mle));
    router.register_entity(Box::new(mm));
    router.register_entity(Box::new(sndcp));
    router.register_entity(Box::new(cmce));

    // Drop all command links that were not given to a TetraEntity
    for (entity, dispatcher) in c_e.into_iter() {
        drop(dispatcher);
        c_d.remove(&entity);
    }

    // Register Brew entity if enabled
    if let Some(ref brew_cfg) = cfg.config().brew {
        let transport = new_websocket_transport(brew_cfg);
        let brew_entity = BrewEntity::new(cfg.clone(), transport);
        router.register_entity(Box::new(brew_entity));
        eprintln!(" -> Brew/TetraPack integration enabled");
    }

    // Init network time
    router.set_dl_time(TdmaTime::default());

    (router, tsource, c_d)
}

/// Start the BS T1 test mode stack: PHY and the T1 test entity only.
/// Brew, telemetry, control and the signalling entities are not built.
fn build_t1_stack(cfg: &SharedConfig) -> (MessageRouter, T1SharedHandle) {
    let mut router = MessageRouter::new(cfg.clone());

    match cfg.config().phy_io.backend {
        PhyBackend::SoapySdr => {
            let rxdev = RxTxDevSoapySdr::new(cfg);
            let phy = PhyBs::new(cfg.clone(), rxdev);
            router.register_entity(Box::new(phy));
        }
        _ => {
            panic!("Unsupported PhyIo type: {:?}", cfg.config().phy_io.backend);
        }
    }

    let t1 = T1TestBs::new(cfg.clone());
    let t1_handle = t1.handle();
    router.register_entity(Box::new(t1));
    eprintln!(" -> BS T1 test mode: network features disabled");

    router.set_dl_time(TdmaTime::default());
    (router, t1_handle)
}

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "TETRA BlueStation base station stack",
    long_about = "Runs the TETRA BlueStation base station stack using the provided TOML configuration files"
)]

struct Args {
    /// Config file (required)
    #[arg(help = "TOML config with network/cell parameters")]
    config: String,
}

/// Run the BS T1 test mode until Ctrl+C or until the configured duration has passed.
/// Prints a final report and exits with 0 (pass), 1 (fail) or 2 (no usable measurement)
fn run_t1_mode(cfg: &SharedConfig) -> ! {
    let (mut router, t1_handle) = build_t1_stack(cfg);

    let is_running = Arc::new(AtomicBool::new(true));
    let is_running_ctrlc = is_running.clone();
    ctrlc::set_handler(move || {
        is_running_ctrlc.store(false, Ordering::SeqCst);
    })
    .expect("failed to set Ctrl+C handler");

    if let Some(duration_s) = cfg.config().t1_test.as_ref().and_then(|t| t.duration_s) {
        let is_running_timer = is_running.clone();
        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_secs(duration_s as u64));
            is_running_timer.store(false, Ordering::SeqCst);
        });
    }

    router.run_stack(None, Some(is_running));

    // Dropping the router drops the T1 entity, which publishes its last counts
    drop(router);
    let shared = t1_handle.lock().expect("T1 shared state");
    println!("{}", shared.final_report().render(shared.cfg.output));
    std::process::exit(shared.exit_code());
}

fn main() {
    eprintln!("░▀█▀░█▀▀░▀█▀░█▀▄░█▀█░░░░░█▀▄░█░░░█░█░█▀▀░█▀▀░▀█▀░█▀█░▀█▀░▀█▀░█▀█░█▀█");
    eprintln!("░░█░░█▀▀░░█░░█▀▄░█▀█░▄▄▄░█▀▄░█░░░█░█░█▀▀░▀▀█░░█░░█▀█░░█░░░█░░█░█░█░█");
    eprintln!("░░▀░░▀▀▀░░▀░░▀░▀░▀░▀░░░░░▀▀░░▀▀▀░▀▀▀░▀▀▀░▀▀▀░░▀░░▀░▀░░▀░░▀▀▀░▀▀▀░▀░▀\n");
    eprintln!("  Wouter Bokslag / Midnight Blue");
    eprintln!("  https://github.com/MidnightBlueLabs/tetra-bluestation");
    eprintln!("  Version: {}", tetra_core::STACK_VERSION);

    // Parse command-line arguments
    let args = Args::parse();

    // Build immutable, cheaply clonable SharedConfig and build the base station stack
    let stack_cfg = load_config_from_toml(&args.config);
    let mut cfg = SharedConfig::from_parts(stack_cfg, None);

    let _log_guards = debug::setup_logging_default(cfg.config().debug_log.clone());

    if cfg.config().t1_test.is_some() {
        run_t1_mode(&cfg);
    }

    let (mut router, tsource, cdispatchers) = build_bs_stack(&mut cfg);

    // Start Telemetry and Control threads, if enabled
    if let Some(telemetry_source) = tsource {
        start_telemetry_worker(cfg.clone(), telemetry_source);
    };
    if cfg.config().control.is_some() {
        start_control_worker(cfg.clone(), cdispatchers);
    };

    // Set up Ctrl+C handler for graceful shutdown
    let is_running = Arc::new(AtomicBool::new(true));
    let is_running_clone = is_running.clone();
    ctrlc::set_handler(move || {
        is_running_clone.store(false, Ordering::SeqCst);
    })
    .expect("failed to set Ctrl+C handler");

    // Start the stack
    router.run_stack(None, Some(is_running));

    // router drops here → entities are dropped, networked entities disconnect.
}
