//! COM lifetime management. UIA is a COM API, so COM must be initialized on the
//! thread that talks to it. [`ComGuard`] initializes the multithreaded
//! apartment (MTA) and balances it with `CoUninitialize` on drop.

use assistant_core::AdapterError;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// RAII guard: initialize COM (MTA) when the thread has no apartment yet, or
/// reuse an apartment already established by the host (for example Tauri's STA
/// UI thread). Only calls `CoUninitialize` when our call incremented COM's
/// per-thread initialization count.
pub struct ComGuard {
    should_uninitialize: bool,
}

impl ComGuard {
    /// Initialize COM in the multithreaded apartment for the current thread.
    pub fn new() -> Result<Self, AdapterError> {
        // SAFETY: `CoInitializeEx` is safe to call per thread; the matching
        // `CoUninitialize` is issued in `Drop`. S_FALSE (already initialized)
        // is not treated as an error.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            // The host initialized this thread as STA. Basic UI Automation
            // client calls are valid there; we must neither reinitialize nor
            // uninitialize the host-owned apartment.
            return Ok(Self {
                should_uninitialize: false,
            });
        }
        if hr.is_err() {
            return Err(AdapterError::Platform(format!(
                "CoInitializeEx failed: {hr:?}"
            )));
        }
        Ok(Self {
            should_uninitialize: true,
        })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.should_uninitialize {
            // SAFETY: balanced with the successful `CoInitializeEx` call in
            // `new` (including S_FALSE, which increments COM's init count).
            unsafe { CoUninitialize() };
        }
    }
}
