//! Symphony pen input: one sample format for every tablet, with our own
//! backend per OS instead of whatever the window toolkit happens to expose.
//!
//! - macOS: reads AppKit tablet events directly (any tablet whose driver
//!   speaks the standard macOS tablet protocol: Wacom, Veikk, XP-Pen,
//!   Huion, Apple Pencil via Sidecar…). Mouse coalescing is turned off so
//!   every sample the tablet sends arrives.
//! - Linux on Wayland: the `tablet-v2` protocol for pens and `wl_pointer`
//!   for the mouse, on the toolkit's own connection (any tablet libinput
//!   supports: Wacom, Huion, XP-Pen, Gaomon…).
//! - Windows (Windows Ink + Wintab), Linux on X11 (XInput2): planned.

#[cfg(target_os = "linux")]
use raw_window_handle::RawDisplayHandle;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use raw_window_handle::RawWindowHandle;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod wayland;

/// Where the pointer is in the stroke lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// In range but not touching (pen hover, or mouse moving with no button).
    Hover,
    Down,
    Move,
    Up,
}

/// Which end of which device produced the sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Mouse,
    Pen,
    Eraser,
}

/// One input sample. Position is in logical points from the top-left of the
/// window's content area (multiply by the window scale factor for pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenSample {
    pub x: f64,
    pub y: f64,
    /// 0.0..=1.0. A mouse reports 1.0 while pressed.
    pub pressure: f32,
    /// -1.0..=1.0 on each axis; 0 = pen upright. 0 when the device has no tilt.
    pub tilt_x: f32,
    pub tilt_y: f32,
    /// Barrel rotation in degrees (art pens only).
    pub rotation: f32,
    /// Tangential pressure / airbrush wheel, -1.0..=1.0.
    pub tangential: f32,
    pub phase: Phase,
    pub tool: Tool,
    /// Raw pen button bits (tip, lower side, upper side).
    pub buttons: u32,
    /// Seconds, monotonic (system uptime on macOS).
    pub time: f64,
}

/// What the OS told us about the tablet on its last proximity event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    pub vendor_id: u64,
    pub tablet_id: u64,
    pub unique_id: u64,
    /// Vendor-defined capability bits (tilt, rotation…), reported as-is.
    pub capability_mask: u64,
    pub in_proximity: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PenError {
    /// No backend for this OS yet.
    Unsupported,
    /// The window handle isn't a native view we can read.
    NoNativeView,
    NotMainThread,
    MonitorFailed,
}

impl std::fmt::Display for PenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            PenError::Unsupported => "no pen backend for this OS yet",
            PenError::NoNativeView => "window has no native view",
            PenError::NotMainThread => "pen input must be attached on the main thread",
            PenError::MonitorFailed => "the OS refused the event monitor",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for PenError {}

/// Collects pen samples for one window. Drop it to stop listening.
pub struct PenInput {
    #[cfg(target_os = "macos")]
    backend: macos::Backend,
    #[cfg(target_os = "linux")]
    backend: wayland::Backend,
}

impl PenInput {
    /// Start listening to pen and mouse input on `window`.
    /// Must be called on the main thread.
    pub fn attach(window: &(impl HasWindowHandle + HasDisplayHandle)) -> Result<Self, PenError> {
        let handle = window.window_handle().map_err(|_| PenError::NoNativeView)?;
        #[cfg(target_os = "macos")]
        if let RawWindowHandle::AppKit(h) = handle.as_raw() {
            return Ok(PenInput {
                backend: macos::Backend::attach(h.ns_view)?,
            });
        }
        #[cfg(target_os = "linux")]
        {
            let display = window
                .display_handle()
                .map_err(|_| PenError::NoNativeView)?;
            if let (RawDisplayHandle::Wayland(d), RawWindowHandle::Wayland(w)) =
                (display.as_raw(), handle.as_raw())
            {
                return Ok(PenInput {
                    backend: wayland::Backend::attach(d.display, w.surface)?,
                });
            }
        }
        let _ = handle;
        Err(PenError::Unsupported)
    }

    /// Call `wake` from any thread whenever new samples arrive. Backends
    /// whose events don't pass through the window toolkit need this, or
    /// the app won't repaint until something else wakes it.
    pub fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        #[cfg(target_os = "linux")]
        self.backend.set_waker(Box::new(wake));
        #[cfg(not(target_os = "linux"))]
        let _ = wake;
    }

    /// Whether the window toolkit also receives the pen as a mouse. True on
    /// macOS, where AppKit sends mouse events for tablet input. False on
    /// Wayland: once the app reads tablet-v2, the compositor stops sending
    /// pointer events for the pen, so the UI only sees it if the app passes
    /// pen samples on as pointer events.
    pub fn toolkit_sees_pen(&self) -> bool {
        !cfg!(target_os = "linux")
    }

    /// Take every sample received since the last call, oldest first.
    pub fn drain(&self) -> Vec<PenSample> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        return self.backend.drain();
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        Vec::new()
    }

    pub fn has_pending(&self) -> bool {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        return self.backend.has_pending();
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        false
    }

    pub fn device(&self) -> DeviceInfo {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        return self.backend.device();
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        DeviceInfo::default()
    }
}
