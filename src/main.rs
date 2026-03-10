mod accessibility;
mod app;
mod config;
mod keystroke;
mod monitor;
mod process;
mod rules;
mod types;

use anyhow::Result;
use monitor::Monitor;
use objc2::MainThreadMarker;
use std::sync::{Arc, Mutex};

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("Starting I'll Allow It...");

    // Load config (creates default if not present)
    let config = config::load_config()?;
    log::info!(
        "Config loaded: {} rules, enabled={}",
        config.rules.len(),
        config.enabled
    );

    // Check accessibility permissions
    if !accessibility::is_accessibility_trusted() {
        log::warn!("Accessibility permission not granted. Requesting...");
        accessibility::check_accessibility_with_prompt(true);
        log::info!("Please grant Accessibility permission in System Settings and restart the app.");
    }

    // Create the monitor
    let monitor = Monitor::new(config);

    // Create shared state
    let state = Arc::new(Mutex::new(app::AppState {
        monitor,
        last_config_mtime: config::config_mtime(),
        status_item: None,
    }));

    // Store global state for ObjC callbacks
    app::set_global_state(state.clone());

    // Must run on main thread for AppKit
    let mtm = unsafe { MainThreadMarker::new_unchecked() };

    // This blocks — runs the NSApplication run loop
    app::setup_app(mtm, state);

    Ok(())
}
