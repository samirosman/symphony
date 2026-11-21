//! AppKit backend. A local event monitor sees every mouse/tablet event for
//! the app before the window does, and passes each one through untouched.
//!
//! AppKit raises an exception (which would abort us) if tablet-only fields
//! are read from a plain mouse event, so every accessor is gated on the
//! event's type and subtype.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSEvent, NSEventMask, NSEventSubtype, NSEventType, NSPointingDeviceType, NSView,
};

use crate::{DeviceInfo, PenError, PenSample, Phase, Tool};

#[derive(Default)]
struct Shared {
    samples: Vec<PenSample>,
    /// Device last announced by a proximity event.
    pen_tool: Option<Tool>,
    device: DeviceInfo,
    down: bool,
}

pub struct Backend {
    shared: Arc<Mutex<Shared>>,
    monitor: Retained<AnyObject>,
}

impl Backend {
    pub fn attach(ns_view: NonNull<c_void>) -> Result<Self, PenError> {
        MainThreadMarker::new().ok_or(PenError::NotMainThread)?;
        // SAFETY: raw-window-handle guarantees ns_view is a live NSView.
        let view: Retained<NSView> =
            unsafe { Retained::retain(ns_view.as_ptr().cast()) }.ok_or(PenError::NoNativeView)?;

        // Deliver every sample instead of merging them per display frame.
        NSEvent::setMouseCoalescingEnabled(false);

        let shared = Arc::new(Mutex::new(Shared::default()));
        let sink = shared.clone();
        let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit hands us a valid event for the duration of the call.
            let e = unsafe { event.as_ref() };
            if let Ok(mut s) = sink.lock() {
                handle(e, &view, &mut s);
            }
            event.as_ptr()
        });

        let mask = NSEventMask::LeftMouseDown
            | NSEventMask::LeftMouseUp
            | NSEventMask::LeftMouseDragged
            | NSEventMask::MouseMoved
            | NSEventMask::TabletPoint
            | NSEventMask::TabletProximity;
        // SAFETY: the block returns the event it was given (valid pointer).
        let monitor =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) }
                .ok_or(PenError::MonitorFailed)?;
        Ok(Backend { shared, monitor })
    }

    pub fn drain(&self) -> Vec<PenSample> {
        self.shared
            .lock()
            .map(|mut s| std::mem::take(&mut s.samples))
            .unwrap_or_default()
    }

    pub fn has_pending(&self) -> bool {
        self.shared
            .lock()
            .map(|s| !s.samples.is_empty())
            .unwrap_or(false)
    }

    pub fn device(&self) -> DeviceInfo {
        self.shared
            .lock()
            .map(|s| s.device.clone())
            .unwrap_or_default()
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        // SAFETY: monitor came from addLocalMonitor…
        unsafe { NSEvent::removeMonitor(&self.monitor) };
    }
}

fn handle(e: &NSEvent, view: &NSView, s: &mut Shared) {
    let ty = e.r#type();
    if ty == NSEventType::TabletProximity {
        proximity(e, s);
        return;
    }

    let is_mouse = ty == NSEventType::LeftMouseDown
        || ty == NSEventType::LeftMouseUp
        || ty == NSEventType::LeftMouseDragged
        || ty == NSEventType::MouseMoved;
    if !is_mouse && ty != NSEventType::TabletPoint {
        return;
    }

    // Only events for our window (proximity is app-wide, handled above).
    let mtm = MainThreadMarker::from(view);
    let ours = match (e.window(mtm), view.window()) {
        (Some(a), Some(b)) => Retained::as_ptr(&a) == Retained::as_ptr(&b),
        _ => false,
    };
    if !ours {
        return;
    }

    let subtype = if is_mouse {
        e.subtype()
    } else {
        NSEventSubtype::TabletPoint
    };
    if subtype == NSEventSubtype::TabletProximity {
        proximity(e, s);
    }
    let tablet = subtype == NSEventSubtype::TabletPoint;

    let phase = if ty == NSEventType::LeftMouseDown {
        s.down = true;
        Phase::Down
    } else if ty == NSEventType::LeftMouseUp {
        s.down = false;
        Phase::Up
    } else if ty == NSEventType::LeftMouseDragged || (ty == NSEventType::TabletPoint && s.down) {
        Phase::Move
    } else {
        Phase::Hover
    };

    let p = view.convertPoint_fromView(e.locationInWindow(), None);
    let y = if view.isFlipped() {
        p.y
    } else {
        view.bounds().size.height - p.y
    };

    let mut sample = PenSample {
        x: p.x,
        y,
        pressure: 0.0,
        tilt_x: 0.0,
        tilt_y: 0.0,
        rotation: 0.0,
        tangential: 0.0,
        phase,
        tool: Tool::Mouse,
        buttons: 0,
        time: e.timestamp(),
    };

    if tablet {
        let tilt = e.tilt();
        sample.pressure = e.pressure();
        sample.tilt_x = tilt.x as f32;
        // AppKit tilt y is positive toward the top of the tablet; flip so
        // +y matches screen-down like our coordinates.
        sample.tilt_y = -tilt.y as f32;
        sample.rotation = e.rotation();
        sample.tangential = e.tangentialPressure();
        sample.buttons = e.buttonMask().bits() as u32;
        sample.tool = s.pen_tool.unwrap_or(Tool::Pen);
    } else if ty != NSEventType::MouseMoved {
        sample.pressure = e.pressure();
    }
    if phase == Phase::Up {
        sample.pressure = 0.0;
    }

    s.samples.push(sample);
}

fn proximity(e: &NSEvent, s: &mut Shared) {
    let entering = e.isEnteringProximity();
    s.device = DeviceInfo {
        vendor_id: e.vendorID() as u64,
        tablet_id: e.tabletID() as u64,
        unique_id: e.uniqueID(),
        capability_mask: e.capabilityMask() as u64,
        in_proximity: entering,
    };
    s.pen_tool = if entering {
        Some(if e.pointingDeviceType() == NSPointingDeviceType::Eraser {
            Tool::Eraser
        } else {
            Tool::Pen
        })
    } else {
        None
    };
}
