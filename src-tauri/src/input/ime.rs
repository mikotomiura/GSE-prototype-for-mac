use windows::{
    core::*, 
    Win32::System::Com::*, 
    Win32::UI::Accessibility::*
};

// Use lazy_static or thread_local for UIA object to avoid re-creation overhead?
// For now, simpler implementation: Initialize COM in the worker thread loop once.

pub struct ImeMonitor {
    automation: Option<IUIAutomation>,
}

impl ImeMonitor {
    pub fn new() -> Self {
        unsafe {
            // Ensure COM is initialized on this thread
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            
            // Try to create the automation object.
            let automation: Result<IUIAutomation> = CoCreateInstance(
                &CUIAutomation, 
                None, 
                CLSCTX_INPROC_SERVER
            );

            if let Ok(auto) = automation {
                Self { automation: Some(auto) }
            } else {
                // Not returning error to keep thread running, just logging
                // eprintln!("Failed to create IUIAutomation"); 
                Self { automation: None }
            }
        }
    }

    pub fn is_candidate_window_open(&self) -> bool {
        if let Some(auto) = &self.automation {
            unsafe {
                // Get focused element
                if let Ok(element) = auto.GetFocusedElement() {
                    // Check properties
                    if let Ok(class_name) = element.CurrentClassName() {
                        let name = class_name.to_string();
                        if name.contains("Candidate") || name.contains("Ime") {
                            return true;
                        }
                    }
                    
                    if let Ok(name_bstr) = element.CurrentName() {
                        let name = name_bstr.to_string();
                        if name.contains("候補") { 
                            return true; 
                        }
                    }
                }
            }
        }
        false
    }
}

// Thread-local helper for cleaner integration
pub fn is_ime_active_check() -> bool {
    // This is a rough check. Real-time polling of UI Automation can be CPU intensive.
    // We should limit this call frequency.
    
    // Static instance per thread?
    // Using a new instance every time is bad.
    // Let's assume the caller manages the ImeMonitor instance.
    false
}
