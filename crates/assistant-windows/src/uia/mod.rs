//! Thin wrapper around the root `IUIAutomation` object.

use assistant_core::AdapterError;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Descendants,
};

use crate::error::platform;

/// Wraps the UIA root object and exposes the operations W0 needs.
pub struct UiaClient {
    automation: IUIAutomation,
}

impl UiaClient {
    /// Create the UIA root object. COM must already be initialized on this
    /// thread (see [`crate::com::ComGuard`]).
    pub fn new() -> Result<Self, AdapterError> {
        // SAFETY: `CUIAutomation` is a registered in-proc COM server.
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| platform("CoCreateInstance(CUIAutomation)", e))?;
        Ok(Self { automation })
    }

    /// Return the UI element that currently has keyboard focus.
    pub fn focused_element(&self) -> Result<IUIAutomationElement, AdapterError> {
        // SAFETY: FFI call into UIA; returned pointer is owned by us.
        unsafe { self.automation.GetFocusedElement() }.map_err(|e| platform("GetFocusedElement", e))
    }

    /// Return the UIA root element for an existing native window.
    pub(crate) fn element_from_hwnd(
        &self,
        hwnd: isize,
    ) -> Result<IUIAutomationElement, AdapterError> {
        // SAFETY: the caller obtained this HWND from GetForegroundWindow.
        unsafe {
            self.automation
                .ElementFromHandle(HWND(hwnd as *mut std::ffi::c_void))
        }
        .map_err(|e| platform("ElementFromHandle", e))
    }

    /// Snapshot all descendants in the control tree. QQ scanning is polling,
    /// so each call owns a short-lived array and never keeps stale COM elements.
    pub(crate) fn descendants(
        &self,
        root: &IUIAutomationElement,
    ) -> Result<Vec<IUIAutomationElement>, AdapterError> {
        // SAFETY: valid automation object and root element.
        let condition = unsafe { self.automation.CreateTrueCondition() }
            .map_err(|e| platform("CreateTrueCondition", e))?;
        // SAFETY: condition and root belong to this UIA client.
        let array = unsafe { root.FindAll(TreeScope_Descendants, &condition) }
            .map_err(|e| platform("FindAll(descendants)", e))?;
        // SAFETY: valid UIA array.
        let len = unsafe { array.Length() }.map_err(|e| platform("ElementArray.Length", e))?;
        let mut elements = Vec::with_capacity((len as usize).min(4096));
        for index in 0..len.min(4096) {
            // SAFETY: index is within the reported array length.
            let element = unsafe { array.GetElement(index) }
                .map_err(|e| platform("ElementArray.GetElement", e))?;
            elements.push(element);
        }
        Ok(elements)
    }
}
