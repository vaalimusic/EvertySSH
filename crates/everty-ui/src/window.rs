//! Platform-neutral per-window state. Native handles and surfaces stay in the
//! winit/backend adapter; renderer surfaces are owned per window there.

use std::collections::HashMap;
use std::time::Instant;

use crate::{DpiScale, InputState, PhysicalSize};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WindowId(pub u64);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CursorIcon {
    #[default]
    Default,
    Pointer,
    Text,
    ResizeHorizontal,
    ResizeVertical,
    Grab,
    Grabbing,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowMetricsChange {
    pub scale_changed: bool,
    pub size_changed: bool,
}

#[derive(Clone, Debug)]
pub struct WindowState {
    pub scale: DpiScale,
    pub physical_size: PhysicalSize,
    pub logical_size: crate::LogicalSize,
    pub focused: bool,
    pub needs_redraw: bool,
    pub cursor: CursorIcon,
    pub ime_enabled: bool,
    pub metrics_generation: u64,
}

impl WindowState {
    pub fn new(physical_size: PhysicalSize, scale: DpiScale) -> Self {
        Self {
            scale,
            physical_size,
            logical_size: physical_size.to_logical(scale),
            focused: false,
            needs_redraw: true,
            cursor: CursorIcon::Default,
            ime_enabled: false,
            metrics_generation: 0,
        }
    }

    pub fn update_metrics(
        &mut self,
        physical_size: PhysicalSize,
        scale: DpiScale,
    ) -> WindowMetricsChange {
        let change = WindowMetricsChange {
            scale_changed: self.scale != scale,
            size_changed: self.physical_size != physical_size,
        };
        if change.scale_changed || change.size_changed {
            self.scale = scale;
            self.physical_size = physical_size;
            self.logical_size = physical_size.to_logical(scale);
            self.metrics_generation = self.metrics_generation.saturating_add(1);
            self.request_redraw();
        }
        change
    }

    pub fn set_focused(&mut self, focused: bool) -> bool {
        if self.focused == focused {
            return false;
        }
        self.focused = focused;
        self.request_redraw();
        true
    }

    pub fn set_cursor(&mut self, cursor: CursorIcon) -> bool {
        if self.cursor == cursor {
            return false;
        }
        self.cursor = cursor;
        true
    }

    pub fn set_ime_enabled(&mut self, enabled: bool) -> bool {
        if self.ime_enabled == enabled {
            return false;
        }
        self.ime_enabled = enabled;
        true
    }

    pub fn request_redraw(&mut self) {
        self.needs_redraw = true;
    }

    pub fn take_redraw_request(&mut self) -> bool {
        std::mem::replace(&mut self.needs_redraw, false)
    }
}

#[derive(Clone, Debug)]
pub struct UiWindow {
    pub id: WindowId,
    pub state: WindowState,
    pub input: InputState,
}

impl UiWindow {
    pub fn new(id: WindowId, physical_size: PhysicalSize, scale: DpiScale) -> Self {
        Self {
            id,
            state: WindowState::new(physical_size, scale),
            input: InputState::default(),
        }
    }

    /// Keep pointer coordinates in the new logical space until the platform
    /// delivers its next physical pointer event. The last window-relative
    /// physical position is only an estimate if the OS also moved the cursor.
    pub fn update_metrics(
        &mut self,
        physical_size: PhysicalSize,
        scale: DpiScale,
    ) -> WindowMetricsChange {
        let old_scale = self.state.scale;
        let change = self.state.update_metrics(physical_size, scale);
        if change.scale_changed {
            self.input.pointer = self.input.pointer.to_physical(old_scale).to_logical(scale);
        }
        if change.scale_changed || change.size_changed {
            self.input.hovered = None;
        }
        change
    }
}

#[derive(Default)]
pub struct WindowRegistry {
    windows: HashMap<WindowId, UiWindow>,
    next_id: u64,
}

impl WindowRegistry {
    /// Create a window with a fresh id. Ids are never reused within a
    /// registry, so a late native event addressed to a closed window can never
    /// reach a newer one.
    pub fn open(&mut self, physical_size: PhysicalSize, scale: DpiScale) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id += 1;
        self.windows
            .insert(id, UiWindow::new(id, physical_size, scale));
        id
    }

    /// Close a window, returning its state. Everything the window owned
    /// (input, focus, capture, redraw demand) is dropped with it.
    pub fn close(&mut self, id: WindowId) -> Option<UiWindow> {
        self.windows.remove(&id)
    }

    /// Route an event to one window's input state. Unknown (closed) windows
    /// yield `None`; other windows are never touched.
    pub fn dispatch(
        &mut self,
        id: WindowId,
        event: crate::InputEvent,
        regions: &[crate::HitRegion],
    ) -> Option<crate::RoutedEvent> {
        let window = self.windows.get_mut(&id)?;
        window.state.request_redraw();
        Some(window.input.route(event, regions))
    }

    pub fn ids(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.windows.keys().copied()
    }

    pub fn insert(&mut self, window: UiWindow) -> Option<UiWindow> {
        self.windows.insert(window.id, window)
    }

    pub fn remove(&mut self, id: WindowId) -> Option<UiWindow> {
        self.windows.remove(&id)
    }

    pub fn get(&self, id: WindowId) -> Option<&UiWindow> {
        self.windows.get(&id)
    }

    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut UiWindow> {
        self.windows.get_mut(&id)
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

/// One monotonic clock is shared by the UI tree for tooltip/animation timing.
#[derive(Clone, Debug)]
pub struct UiClock {
    origin: Instant,
}

impl Default for UiClock {
    fn default() -> Self {
        Self::new()
    }
}

impl UiClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    pub fn elapsed(&self) -> std::time::Duration {
        self.origin.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_and_resize_update_only_window_metrics_and_request_redraw() {
        let mut state = WindowState::new(PhysicalSize::new(1200.0, 800.0), DpiScale::ONE);
        assert_eq!(state.logical_size, crate::LogicalSize::new(1200.0, 800.0));
        assert!(state.take_redraw_request());
        let changed = state.update_metrics(
            PhysicalSize::new(1500.0, 1000.0),
            DpiScale::new(1.25).unwrap(),
        );
        assert!(changed.scale_changed && changed.size_changed);
        assert_eq!(state.logical_size, crate::LogicalSize::new(1200.0, 800.0));
        assert_eq!(state.metrics_generation, 1);
        assert!(state.take_redraw_request());
        assert!(!state.take_redraw_request());
    }

    #[test]
    fn independent_windows_keep_separate_input_and_cursor_state() {
        let mut windows = WindowRegistry::default();
        let mut a = UiWindow::new(WindowId(1), PhysicalSize::new(100.0, 100.0), DpiScale::ONE);
        let mut b = UiWindow::new(
            WindowId(2),
            PhysicalSize::new(200.0, 100.0),
            DpiScale::new(2.0).unwrap(),
        );
        a.state.set_cursor(CursorIcon::Pointer);
        a.input.pointer_capture = Some(crate::WidgetId(4));
        b.state.set_cursor(CursorIcon::Text);
        windows.insert(a);
        windows.insert(b);
        assert_eq!(windows.len(), 2);
        assert_eq!(
            windows.get(WindowId(1)).unwrap().state.cursor,
            CursorIcon::Pointer
        );
        assert_eq!(
            windows.get(WindowId(2)).unwrap().state.cursor,
            CursorIcon::Text
        );
        assert_eq!(
            windows.get(WindowId(1)).unwrap().input.pointer_capture,
            Some(crate::WidgetId(4))
        );
        windows.remove(WindowId(1));
        assert_eq!(windows.len(), 1);
    }

    #[test]
    fn scale_change_rebases_pointer_and_invalidates_hover_without_losing_capture() {
        let mut window = UiWindow::new(
            WindowId(3),
            PhysicalSize::new(1500.0, 1000.0),
            DpiScale::new(1.25).unwrap(),
        );
        window.input.pointer = crate::LogicalPoint::new(100.0, 80.0);
        window.input.hovered = Some(crate::WidgetId(10));
        window.input.capture_pointer(crate::WidgetId(11));
        let change = window.update_metrics(
            PhysicalSize::new(2400.0, 1600.0),
            DpiScale::new(2.0).unwrap(),
        );
        assert!(change.scale_changed && change.size_changed);
        assert_eq!(window.input.pointer, crate::LogicalPoint::new(62.5, 50.0));
        assert_eq!(window.input.hovered, None);
        assert_eq!(window.input.pointer_capture, Some(crate::WidgetId(11)));
        assert_eq!(
            window.state.logical_size,
            crate::LogicalSize::new(1200.0, 800.0)
        );
        assert_eq!(window.state.metrics_generation, 1);
        assert!(window.input.release_pointer(crate::WidgetId(11)));
        let target = crate::HitRegion {
            id: crate::WidgetId(13),
            rect: crate::LogicalRect::from_xywh(55.0, 45.0, 20.0, 20.0),
            layer: crate::PaintLayer::Content,
            paint_order: 0,
            enabled: true,
            focusable: true,
            cursor: CursorIcon::Pointer,
        };
        let routed = window.input.route(
            crate::InputEvent::PointerDown(crate::MouseButton::Primary),
            &[target],
        );
        assert_eq!(routed.target, Some(target.id));
    }

    #[test]
    fn resize_invalidates_hover_but_unchanged_metrics_preserve_it() {
        let mut window = UiWindow::new(WindowId(4), PhysicalSize::new(800.0, 600.0), DpiScale::ONE);
        window.input.pointer = crate::LogicalPoint::new(30.0, 40.0);
        window.input.hovered = Some(crate::WidgetId(12));
        assert_eq!(
            window.update_metrics(PhysicalSize::new(800.0, 600.0), DpiScale::ONE),
            WindowMetricsChange::default()
        );
        assert_eq!(window.input.hovered, Some(crate::WidgetId(12)));
        let change = window.update_metrics(PhysicalSize::new(900.0, 600.0), DpiScale::ONE);
        assert!(change.size_changed && !change.scale_changed);
        assert_eq!(window.input.pointer, crate::LogicalPoint::new(30.0, 40.0));
        assert_eq!(window.input.hovered, None);
    }

    #[test]
    fn cursor_ime_and_focus_are_idempotent_state_changes() {
        let mut state = WindowState::new(PhysicalSize::new(10.0, 10.0), DpiScale::ONE);
        assert!(state.set_focused(true));
        assert!(!state.set_focused(true));
        assert!(state.set_cursor(CursorIcon::ResizeHorizontal));
        assert!(state.set_ime_enabled(true));
        assert!(!state.set_ime_enabled(true));
    }
}
