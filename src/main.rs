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
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--diagnose") {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
        return run_diagnose();
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("Starting I'll Allow It...");

    let config = config::load_config()?;
    log::info!(
        "Config loaded: {} rules, enabled={}",
        config.rules.len(),
        config.enabled
    );

    if !accessibility::is_accessibility_trusted() {
        log::warn!("Accessibility permission not granted. Requesting...");
        accessibility::check_accessibility_with_prompt(true);
        log::info!("Please grant Accessibility permission in System Settings and restart the app.");
    }

    let monitor = Monitor::new(config);

    let state = Arc::new(Mutex::new(app::AppState {
        monitor,
        last_config_mtime: config::config_mtime(),
        status_item: None,
    }));

    app::set_global_state(state.clone());

    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    app::setup_app(mtm, state);

    Ok(())
}

/// Diagnostic mode: dump what the app sees.
fn run_diagnose() -> Result<()> {
    println!("=== I'll Allow It - Diagnostic Mode ===\n");

    // 1. Accessibility
    let trusted = accessibility::is_accessibility_trusted();
    println!("Accessibility trusted: {}", trusted);
    if !trusted {
        println!("ERROR: Not trusted! Grant permission in System Settings > Privacy & Security > Accessibility");
        println!("Then re-run this diagnostic.");
        return Ok(());
    }

    // 2. Config
    let config = config::load_config()?;
    println!("\nConfig: enabled={}, vscode_enabled={}, rules={}, default={}",
        config.enabled, config.vscode_enabled, config.rules.len(), config.default_action);

    // 3. Claude processes
    let mut system = sysinfo::System::new();
    let claude_procs = process::find_claude_processes(&mut system);
    println!("\n--- Claude Processes ({}) ---", claude_procs.len());
    for proc in &claude_procs {
        println!("  claude PID {} -> parent {} ({})", proc.pid, proc.parent_app_pid, proc.parent_app_name);

        println!("  Scanning accessibility tree for parent PID {}...", proc.parent_app_pid);
        match accessibility::scan_app_windows(proc.parent_app_pid as i32) {
            Ok(scan) => {
                println!("  Found {} text nodes, {} buttons", scan.texts.len(), scan.buttons.len());
                for btn in &scan.buttons {
                    println!("    BUTTON: {:?}", btn.title);
                }
                // Show some text context
                let relevant: Vec<&String> = scan.texts.iter()
                    .filter(|t| t.contains("Allow") || t.contains("Deny") || t.contains("Claude") || t.contains("Run"))
                    .collect();
                if !relevant.is_empty() {
                    println!("  Relevant text fragments:");
                    for t in &relevant {
                        println!("    {:?}", if t.len() > 120 { &t[..120] } else { t });
                    }
                }
            }
            Err(e) => println!("  ERROR: {}", e),
        }

        // Also dump the raw tree
        println!("\n  Accessibility tree dump:");
        match accessibility::dump_tree(proc.parent_app_pid as i32) {
            Ok(tree) => {
                for line in tree.lines().take(60) {
                    println!("  {}", line);
                }
                if tree.lines().count() > 60 {
                    println!("  ... (truncated)");
                }
            }
            Err(e) => println!("  ERROR: {}", e),
        }
    }

    // 4. Direct scan of Claude.app main process
    println!("\n--- Direct Claude.app Scan ---");
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    for (pid, proc) in system.processes() {
        let name = proc.name().to_string_lossy().to_string();
        if name == "Claude" {
            let exe = proc.exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            if exe.contains("Contents/MacOS/Claude") {
                println!("Main Claude.app: PID {}", pid);
                match accessibility::scan_app_windows(pid.as_u32() as i32) {
                    Ok(scan) => {
                        println!("  {} text nodes, {} buttons", scan.texts.len(), scan.buttons.len());
                        for btn in &scan.buttons {
                            println!("  BUTTON: {:?}", btn.title);
                        }
                    }
                    Err(e) => println!("  ERROR: {}", e),
                }
                match accessibility::dump_tree(pid.as_u32() as i32) {
                    Ok(tree) => {
                        println!("  Tree:");
                        for line in tree.lines().take(80) {
                            println!("    {}", line);
                        }
                    }
                    Err(e) => println!("  Tree ERROR: {}", e),
                }
            }
        }
    }

    // 5. VSCode
    let vscode_procs = process::find_vscode_processes(&mut system);
    println!("\n--- VSCode Processes ({}) ---", vscode_procs.len());
    for proc in &vscode_procs {
        println!("  PID {} ({})", proc.pid, proc.app_name);
        match accessibility::scan_app_windows(proc.pid as i32) {
            Ok(scan) => {
                println!("  {} buttons found", scan.buttons.len());
                for btn in &scan.buttons {
                    println!("    BUTTON: {:?}", btn.title);
                }
            }
            Err(e) => println!("  ERROR: {}", e),
        }
    }

    println!("\n=== Diagnostic complete ===");
    Ok(())
}
