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
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    fn CFArrayGetCount(array: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: isize) -> *const c_void;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFRelease(cf: CFTypeRef);
}

// Accessibility attribute keys
fn ax_focused_window() -> CFString {
    CFString::new("AXFocusedWindow")
}

fn ax_children() -> CFString {
    CFString::new("AXChildren")
}

fn ax_value() -> CFString {
    CFString::new("AXValue")
}

fn ax_title() -> CFString {
    CFString::new("AXTitle")
}

fn ax_description() -> CFString {
    CFString::new("AXDescription")
}

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

        // Build the options dictionary manually via CF
        let keys_raw = [key.as_CFTypeRef()];
        let values_raw = [value.as_CFTypeRef()];
        let dict = core_foundation::dictionary::CFDictionary::<CFString, core_foundation::boolean::CFBoolean>::from_CFType_pairs(
            &[(key, value.clone())],
        );
        let _ = (keys_raw, values_raw); // suppress unused
        AXIsProcessTrustedWithOptions(dict.as_CFTypeRef())
    }
}

/// Read all visible text from the focused window of an application.
pub fn read_window_text(pid: i32) -> Result<String> {
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return Err(anyhow!("Failed to create AXUIElement for pid {}", pid));
        }

        // Get the focused window
        let mut window: CFTypeRef = ptr::null();
        let err = AXUIElementCopyAttributeValue(
            app,
            ax_focused_window().as_CFTypeRef() as CFStringRef,
            &mut window,
        );

        CFRelease(app as CFTypeRef);

        if err != K_AX_ERROR_SUCCESS || window.is_null() {
            return Err(anyhow!(
                "Failed to get focused window for pid {} (AXError {})",
                pid,
                err
            ));
        }

        // Recursively collect text from the window
        let mut texts = Vec::new();
        collect_text(window as AXUIElementRef, &mut texts, 0, 15);

        CFRelease(window);

        Ok(texts.join("\n"))
    }
}

/// Recursively walk the AX element tree and collect text values.
unsafe fn collect_text(
    element: AXUIElementRef,
    texts: &mut Vec<String>,
    depth: usize,
    max_depth: usize,
) {
    if depth >= max_depth || element.is_null() {
        return;
    }

    // Try to read string attributes
    for attr in &[ax_value(), ax_title(), ax_description()] {
        if let Some(text) = unsafe { get_string_attribute(element, attr) } {
            if !text.is_empty() {
                texts.push(text);
            }
        }
    }

    // Recurse into children
    let mut children: CFTypeRef = ptr::null();
    let err = unsafe {
        AXUIElementCopyAttributeValue(
            element,
            ax_children().as_CFTypeRef() as CFStringRef,
            &mut children,
        )
    };

    if err == K_AX_ERROR_SUCCESS && !children.is_null() {
        let count = unsafe { CFArrayGetCount(children as CFArrayRef) };

        for i in 0..count {
            let child = unsafe { CFArrayGetValueAtIndex(children as CFArrayRef, i) };
            if !child.is_null() {
                unsafe {
                    collect_text(child as AXUIElementRef, texts, depth + 1, max_depth);
                }
            }
        }

        unsafe { CFRelease(children) };
    }
}

/// Get a string attribute from an AX element.
unsafe fn get_string_attribute(element: AXUIElementRef, attribute: &CFString) -> Option<String> {
    let mut value: CFTypeRef = ptr::null();
    let err = unsafe {
        AXUIElementCopyAttributeValue(
            element,
            attribute.as_CFTypeRef() as CFStringRef,
            &mut value,
        )
    };

    if err != K_AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }

    // Verify it's actually a CFString before wrapping
    let type_id = unsafe { CFGetTypeID(value) };
    let string_type_id = unsafe { CFStringGetTypeID() };
    if type_id != string_type_id {
        unsafe { CFRelease(value) };
        return None;
    }

    let cf_string = unsafe { CFString::wrap_under_create_rule(value as *const _) };
    Some(cf_string.to_string())
}
