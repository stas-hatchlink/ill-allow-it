use crate::types::ApprovalAction;
use anyhow::{anyhow, Result};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::ffi::c_void;

// CGEventPostToPid is not in the core-graphics crate, so we declare it via FFI
type CGEventRef = *mut c_void;

unsafe extern "C" {
    fn CGEventPostToPid(pid: i32, event: CGEventRef);
}

/// Send a keystroke corresponding to the given action to the target application.
pub fn send_keystroke(target_pid: i32, action: ApprovalAction) -> Result<()> {
    let keycode = match action.keycode() {
        Some(k) => k,
        None => return Ok(()), // Ignore action, nothing to send
    };
    let needs_cmd = action.needs_cmd();

    log::info!(
        "Sending keystroke {} (keycode 0x{:02x}, cmd={}) to pid {}",
        action,
        keycode,
        needs_cmd,
        target_pid
    );

    // Try direct post_to_pid first
    match send_key_to_pid(target_pid, keycode, needs_cmd) {
        Ok(()) => {
            log::debug!("Successfully sent keystroke via CGEventPostToPid");
            Ok(())
        }
        Err(e) => {
            log::warn!(
                "CGEventPostToPid failed ({}), trying HID fallback",
                e
            );
            send_key_via_hid(keycode, needs_cmd)
        }
    }
}

/// Send a key event directly to a process by PID using CGEventPostToPid.
fn send_key_to_pid(pid: i32, keycode: u16, cmd: bool) -> Result<()> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| anyhow!("Failed to create CGEventSource"))?;

    // Key down
    let key_down = CGEvent::new_keyboard_event(source.clone(), keycode, true)
        .map_err(|_| anyhow!("Failed to create key down event"))?;

    // Set Cmd modifier if needed (for "Always allow for session")
    if cmd {
        key_down.set_flags(CGEventFlags::CGEventFlagCommand);
    }

    // Key up
    let key_up = CGEvent::new_keyboard_event(source, keycode, false)
        .map_err(|_| anyhow!("Failed to create key up event"))?;
    if cmd {
        key_up.set_flags(CGEventFlags::CGEventFlagCommand);
    }

    unsafe {
        let down_ref: CGEventRef = std::mem::transmute_copy(&key_down);
        let up_ref: CGEventRef = std::mem::transmute_copy(&key_up);
        CGEventPostToPid(pid, down_ref);
        CGEventPostToPid(pid, up_ref);
    }

    Ok(())
}

/// Fallback: post the keystroke to the HID system (goes to frontmost app).
fn send_key_via_hid(keycode: u16, cmd: bool) -> Result<()> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| anyhow!("Failed to create CGEventSource"))?;

    let key_down = CGEvent::new_keyboard_event(source.clone(), keycode, true)
        .map_err(|_| anyhow!("Failed to create key down event"))?;
    if cmd {
        key_down.set_flags(CGEventFlags::CGEventFlagCommand);
    }
    key_down.post(CGEventTapLocation::HID);

    let key_up = CGEvent::new_keyboard_event(source, keycode, false)
        .map_err(|_| anyhow!("Failed to create key up event"))?;
    if cmd {
        key_up.set_flags(CGEventFlags::CGEventFlagCommand);
    }
    key_up.post(CGEventTapLocation::HID);

    log::info!("Sent keystroke via HID fallback");
    Ok(())
}
