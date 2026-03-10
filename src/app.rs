use crate::config;
use crate::monitor::Monitor;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
};
use objc2_foundation::{NSObject, NSString, NSTimer};
use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Shared application state accessible from ObjC callbacks
pub struct AppState {
    pub monitor: Monitor,
    pub last_config_mtime: Option<SystemTime>,
    pub status_item: Option<Retained<NSStatusItem>>,
}

// Thread-safe shared state
pub type SharedState = Arc<Mutex<AppState>>;

// Global state for ObjC callbacks (needed because ObjC selectors can't capture Rust closures)
thread_local! {
    static GLOBAL_STATE: RefCell<Option<SharedState>> = const { RefCell::new(None) };
}

pub fn set_global_state(state: SharedState) {
    GLOBAL_STATE.with(|s| {
        *s.borrow_mut() = Some(state);
    });
}

fn with_state<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut AppState) -> R,
{
    GLOBAL_STATE.with(|s| {
        let borrowed = s.borrow();
        let state = borrowed.as_ref()?;
        let mut locked = state.lock().ok()?;
        Some(f(&mut locked))
    })
}

// Define the AppDelegate class
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "AppDelegate"]
    #[thread_kind = AllocAnyThread]
    pub struct AppDelegate;

    impl AppDelegate {
        #[unsafe(method(onTick:))]
        fn on_tick(&self, _timer: *mut NSTimer) {
            with_state(|state| {
                // Check for config changes
                let current_mtime = config::config_mtime();
                if current_mtime != state.last_config_mtime {
                    if let Ok(new_config) = config::load_config() {
                        log::info!("Config reloaded");
                        state.monitor.update_config(new_config);
                        state.last_config_mtime = current_mtime;
                    }
                }

                // Run one monitoring tick
                let actions = state.monitor.tick();
                if actions > 0 {
                    log::info!("Took {} actions this tick", actions);
                }
            });
        }

        #[unsafe(method(onQuit:))]
        fn on_quit(&self, _sender: *mut NSObject) {
            let app = NSApplication::sharedApplication(unsafe { MainThreadMarker::new_unchecked() });
            app.terminate(None);
        }

        #[unsafe(method(onToggleEnabled:))]
        fn on_toggle_enabled(&self, _sender: *mut NSObject) {
            with_state(|state| {
                if let Ok(mut cfg) = config::load_config() {
                    cfg.enabled = !cfg.enabled;
                    let _ = config::save_config(&cfg);
                    state.monitor.update_config(cfg);
                    log::info!("Toggled enabled state");
                }
            });
        }

        #[unsafe(method(onEditRules:))]
        fn on_edit_rules(&self, _sender: *mut NSObject) {
            let path = config::config_path();
            let path_str = path.to_string_lossy().to_string();
            std::process::Command::new("open")
                .arg("-t")
                .arg(&path_str)
                .spawn()
                .ok();
        }

        #[unsafe(method(onReloadConfig:))]
        fn on_reload_config(&self, _sender: *mut NSObject) {
            with_state(|state| {
                if let Ok(cfg) = config::load_config() {
                    state.monitor.update_config(cfg);
                    state.last_config_mtime = config::config_mtime();
                    log::info!("Config manually reloaded");
                }
            });
        }
    }
);

pub fn setup_app(mtm: MainThreadMarker, shared_state: SharedState) {
    let app = NSApplication::sharedApplication(mtm);

    // Set as accessory app (menu bar only, no dock icon)
    app.setActivationPolicy(
        objc2_app_kit::NSApplicationActivationPolicy::Accessory,
    );

    // Create the delegate
    let delegate: Retained<AppDelegate> = unsafe {
        let alloc = AppDelegate::alloc();
        let partial = alloc.set_ivars(());
        msg_send![super(partial), init]
    };

    // Create status bar item
    let status_bar = NSStatusBar::systemStatusBar();
    let status_item = status_bar.statusItemWithLength(-1.0); // NSVariableStatusItemLength

    // Set the title (text in menu bar)
    if let Some(button) = status_item.button(mtm) {
        let title = NSString::from_str("I'll Allow It");
        button.setTitle(&title);
    }

    // Build the menu
    let menu = build_menu(mtm, &delegate);
    status_item.setMenu(Some(&menu));

    // Store status item in state
    {
        let mut state = shared_state.lock().unwrap();
        state.status_item = Some(status_item);
    }

    // Set up polling timer
    unsafe {
        let sel = objc2::sel!(onTick:);
        let timer = NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
            0.5,
            &delegate,
            sel,
            None,
            true,
        );

        // Prevent timer from being cleaned up
        std::mem::forget(timer);
    }

    // Keep delegate alive
    std::mem::forget(delegate);

    // Run the app
    app.run();
}

fn build_menu(mtm: MainThreadMarker, delegate: &AppDelegate) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);

    // Get a reference to the delegate as AnyObject for setTarget
    let delegate_obj: &AnyObject = unsafe { std::mem::transmute::<&AppDelegate, &AnyObject>(delegate) };

    // Status item (disabled, informational)
    add_menu_item(&menu, mtm, "Status: Monitoring...", None, None, false);

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    add_menu_item(&menu, mtm, "Enabled", Some(objc2::sel!(onToggleEnabled:)), Some(delegate_obj), true);

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    add_menu_item(&menu, mtm, "Edit Rules...", Some(objc2::sel!(onEditRules:)), Some(delegate_obj), true);
    add_menu_item(&menu, mtm, "Reload Config", Some(objc2::sel!(onReloadConfig:)), Some(delegate_obj), true);

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    add_menu_item(&menu, mtm, "Quit I'll Allow It", Some(objc2::sel!(onQuit:)), Some(delegate_obj), true);

    menu
}

fn add_menu_item(
    menu: &NSMenu,
    mtm: MainThreadMarker,
    title: &str,
    action: Option<objc2::runtime::Sel>,
    target: Option<&AnyObject>,
    enabled: bool,
) {
    let ns_title = NSString::from_str(title);
    let key_equiv = NSString::from_str("");
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &ns_title,
            action,
            &key_equiv,
        )
    };
    if let Some(t) = target {
        unsafe { item.setTarget(Some(t)) };
    }
    if !enabled {
        item.setEnabled(false);
    }
    menu.addItem(&item);
}
