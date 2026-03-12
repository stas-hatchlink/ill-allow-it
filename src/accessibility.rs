#![allow(unsafe_op_in_unsafe_fn)]

use anyhow::{anyhow, Result};
use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use std::ffi::c_void;
use std::ptr;

// Accessibility API types
type AXUIElementRef = *mut c_void;
type AXError = i32;
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;

const K_AX_ERROR_SUCCESS: AXError = 0;

unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    fn CFArrayGetCount(array: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: isize) -> *const c_void;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFRelease(cf: CFTypeRef);
}

// Accessibility attribute keys
fn ax_focused_window() -> CFString { CFString::new("AXFocusedWindow") }
fn ax_windows() -> CFString { CFString::new("AXWindows") }
fn ax_children() -> CFString { CFString::new("AXChildren") }
fn ax_role() -> CFString { CFString::new("AXRole") }
fn ax_role_description() -> CFString { CFString::new("AXRoleDescription") }
fn ax_value() -> CFString { CFString::new("AXValue") }
fn ax_title() -> CFString { CFString::new("AXTitle") }
fn ax_description() -> CFString { CFString::new("AXDescription") }
fn ax_identifier() -> CFString { CFString::new("AXIdentifier") }
fn ax_press() -> CFString { CFString::new("AXPress") }

/// Check if the current process has accessibility permissions.
pub fn is_accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Check if trusted, and optionally prompt the user to grant permission.
pub fn check_accessibility_with_prompt(prompt: bool) -> bool {
    unsafe {
        let key = CFString::new("AXTrustedCheckOptionPrompt");
        let value = if prompt {
            core_foundation::boolean::CFBoolean::true_value()
        } else {
            core_foundation::boolean::CFBoolean::false_value()
        };
        let keys_raw = [key.as_CFTypeRef()];
        let values_raw = [value.as_CFTypeRef()];
        let dict = core_foundation::dictionary::CFDictionary::<CFString, core_foundation::boolean::CFBoolean>::from_CFType_pairs(
            &[(key, value.clone())],
        );
        let _ = (keys_raw, values_raw);
        AXIsProcessTrustedWithOptions(dict.as_CFTypeRef())
    }
}

/// A button found in the accessibility tree.
#[derive(Debug, Clone)]
pub struct AXButton {
    /// The title/label of the button
    pub title: String,
    /// The element reference (opaque pointer, valid only during the search scope)
    element: usize, // stored as usize for the pointer
}

/// Result of scanning an app's windows for buttons.
#[derive(Debug)]
pub struct WindowScan {
    /// All text found in the window (for context/logging)
    pub texts: Vec<String>,
    /// Buttons found with their titles
    pub buttons: Vec<AXButton>,
}

/// Scan all windows of an application for text and buttons.
/// Searches the focused window first, then all windows.
pub fn scan_app_windows(pid: i32) -> Result<WindowScan> {
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return Err(anyhow!("Failed to create AXUIElement for pid {}", pid));
        }

        let mut scan = WindowScan {
            texts: Vec::new(),
            buttons: Vec::new(),
        };

        // Try focused window first
        let mut focused: CFTypeRef = ptr::null();
        let err = AXUIElementCopyAttributeValue(
            app,
            ax_focused_window().as_CFTypeRef() as CFStringRef,
            &mut focused,
        );
        if err == K_AX_ERROR_SUCCESS && !focused.is_null() {
            collect_elements(focused as AXUIElementRef, &mut scan, 0, 20);
            CFRelease(focused);
        }

        // If we didn't find buttons in focused window, scan all windows
        if scan.buttons.is_empty() {
            let mut windows: CFTypeRef = ptr::null();
            let err = AXUIElementCopyAttributeValue(
                app,
                ax_windows().as_CFTypeRef() as CFStringRef,
                &mut windows,
            );
            if err == K_AX_ERROR_SUCCESS && !windows.is_null() {
                let count = CFArrayGetCount(windows as CFArrayRef);
                for i in 0..count {
                    let win = CFArrayGetValueAtIndex(windows as CFArrayRef, i);
                    if !win.is_null() {
                        collect_elements(win as AXUIElementRef, &mut scan, 0, 20);
                        if !scan.buttons.is_empty() {
                            break; // Found buttons, stop scanning
                        }
                    }
                }
                CFRelease(windows);
            }
        }

        CFRelease(app as CFTypeRef);
        Ok(scan)
    }
}

/// Click a button by its stored element reference.
pub fn click_button(button: &AXButton) -> Result<()> {
    unsafe {
        let element = button.element as AXUIElementRef;
        let err = AXUIElementPerformAction(
            element,
            ax_press().as_CFTypeRef() as CFStringRef,
        );
        if err != K_AX_ERROR_SUCCESS {
            return Err(anyhow!("AXPress failed with error {}", err));
        }
    }
    Ok(())
}

/// Recursively walk the AX element tree, collecting text and buttons.
unsafe fn collect_elements(
    element: AXUIElementRef,
    scan: &mut WindowScan,
    depth: usize,
    max_depth: usize,
) {
    if depth >= max_depth || element.is_null() {
        return;
    }

    // Get element role
    let role = get_string_attribute(element, &ax_role());

    // Collect text from any element
    for attr in &[ax_value(), ax_title(), ax_description()] {
        if let Some(text) = get_string_attribute(element, attr) {
            if !text.is_empty() {
                scan.texts.push(text);
            }
        }
    }

    // Check if this is a clickable button
    if let Some(ref role_str) = role {
        if role_str == "AXButton" || role_str == "AXLink" {
            if let Some(title) = get_string_attribute(element, &ax_title()) {
                if !title.is_empty() {
                    scan.buttons.push(AXButton {
                        title,
                        element: element as usize,
                    });
                }
            }
            // Also check AXDescription and AXValue for button label
            if let Some(desc) = get_string_attribute(element, &ax_description()) {
                if !desc.is_empty() {
                    // Only add if we don't already have this button by title
                    let already_found = scan.buttons.iter().any(|b| b.element == element as usize);
                    if !already_found {
                        scan.buttons.push(AXButton {
                            title: desc,
                            element: element as usize,
                        });
                    }
                }
            }
        }
    }

    // Recurse into children
    let mut children: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(
        element,
        ax_children().as_CFTypeRef() as CFStringRef,
        &mut children,
    );

    if err == K_AX_ERROR_SUCCESS && !children.is_null() {
        let count = CFArrayGetCount(children as CFArrayRef);
        for i in 0..count {
            let child = CFArrayGetValueAtIndex(children as CFArrayRef, i);
            if !child.is_null() {
                collect_elements(child as AXUIElementRef, scan, depth + 1, max_depth);
            }
        }
        CFRelease(children);
    }
}

/// Read all visible text from the focused window of an application.
pub fn read_window_text(pid: i32) -> Result<String> {
    let scan = scan_app_windows(pid)?;
    Ok(scan.texts.join("\n"))
}

/// Get a string attribute from an AX element.
unsafe fn get_string_attribute(element: AXUIElementRef, attribute: &CFString) -> Option<String> {
    let mut value: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(
        element,
        attribute.as_CFTypeRef() as CFStringRef,
        &mut value,
    );

    if err != K_AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }

    let type_id = CFGetTypeID(value);
    let string_type_id = CFStringGetTypeID();
    if type_id != string_type_id {
        CFRelease(value);
        return None;
    }

    let cf_string = CFString::wrap_under_create_rule(value as *const _);
    Some(cf_string.to_string())
}

/// Dump the accessibility tree for diagnostic purposes.
pub fn dump_tree(pid: i32) -> Result<String> {
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return Err(anyhow!("Failed to create AXUIElement for pid {}", pid));
        }

        let mut output = String::new();

        // Try focused window
        let mut focused: CFTypeRef = ptr::null();
        let err = AXUIElementCopyAttributeValue(
            app,
            ax_focused_window().as_CFTypeRef() as CFStringRef,
            &mut focused,
        );

        if err == K_AX_ERROR_SUCCESS && !focused.is_null() {
            dump_element(focused as AXUIElementRef, &mut output, 0, 10);
            CFRelease(focused);
        } else {
            output.push_str(&format!("No focused window (AXError {})\n", err));

            // Try all windows
            let mut windows: CFTypeRef = ptr::null();
            let err = AXUIElementCopyAttributeValue(
                app,
                ax_windows().as_CFTypeRef() as CFStringRef,
                &mut windows,
            );
            if err == K_AX_ERROR_SUCCESS && !windows.is_null() {
                let count = CFArrayGetCount(windows as CFArrayRef);
                output.push_str(&format!("{} window(s) found\n", count));
                for i in 0..count.min(3) {
                    output.push_str(&format!("\n--- Window {} ---\n", i));
                    let win = CFArrayGetValueAtIndex(windows as CFArrayRef, i);
                    if !win.is_null() {
                        dump_element(win as AXUIElementRef, &mut output, 0, 10);
                    }
                }
                CFRelease(windows);
            } else {
                output.push_str("No windows found\n");
            }
        }

        CFRelease(app as CFTypeRef);
        Ok(output)
    }
}

/// Dump a single AX element and its children for diagnostics.
unsafe fn dump_element(element: AXUIElementRef, output: &mut String, depth: usize, max_depth: usize) {
    if depth >= max_depth || element.is_null() {
        return;
    }

    let indent = "  ".repeat(depth);
    let role = get_string_attribute(element, &ax_role()).unwrap_or_default();
    let title = get_string_attribute(element, &ax_title()).unwrap_or_default();
    let value = get_string_attribute(element, &ax_value()).unwrap_or_default();
    let desc = get_string_attribute(element, &ax_description()).unwrap_or_default();
    let ident = get_string_attribute(element, &ax_identifier()).unwrap_or_default();
    let role_desc = get_string_attribute(element, &ax_role_description()).unwrap_or_default();

    // Only print elements that have something interesting
    if !role.is_empty() || !title.is_empty() || !value.is_empty() {
        output.push_str(&format!("{}[{}]", indent, role));
        if !title.is_empty() { output.push_str(&format!(" title={:?}", truncate(&title, 80))); }
        if !value.is_empty() { output.push_str(&format!(" value={:?}", truncate(&value, 80))); }
        if !desc.is_empty() { output.push_str(&format!(" desc={:?}", truncate(&desc, 80))); }
        if !ident.is_empty() { output.push_str(&format!(" id={:?}", ident)); }
        if !role_desc.is_empty() && role_desc != role { output.push_str(&format!(" roleDesc={:?}", role_desc)); }
        output.push('\n');
    }

    // Recurse
    let mut children: CFTypeRef = ptr::null();
    let err = AXUIElementCopyAttributeValue(
        element,
        ax_children().as_CFTypeRef() as CFStringRef,
        &mut children,
    );
    if err == K_AX_ERROR_SUCCESS && !children.is_null() {
        let count = CFArrayGetCount(children as CFArrayRef);
        for i in 0..count {
            let child = CFArrayGetValueAtIndex(children as CFArrayRef, i);
            if !child.is_null() {
                dump_element(child as AXUIElementRef, output, depth + 1, max_depth);
            }
        }
        CFRelease(children);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max])
    } else {
        s.to_string()
    }
}
