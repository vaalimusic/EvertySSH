//! Minimal single-popup state for menus and anchored transient panels.

use crate::{InputEvent, InputState, Key, LogicalPoint, LogicalRect, LogicalSize, WidgetId};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PopupId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PopupPointerResult {
    None,
    Inside,
    DismissedOutside,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopupState {
    pub id: PopupId,
    pub anchor: LogicalRect,
    pub bounds: LogicalRect,
    /// Widget that held keyboard focus when the popup opened.
    pub restore_focus: Option<WidgetId>,
}

/// Place a `size` panel next to `anchor` inside `window`: below the anchor when
/// it fits, otherwise above it, always clamped into the window (a panel larger
/// than the window is shrunk to it). Shared by popups and tooltips so both
/// follow one placement rule.
pub fn anchored_placement(
    anchor: LogicalRect,
    size: LogicalSize,
    window: LogicalRect,
    gap: f64,
) -> LogicalRect {
    let width = size.width.0.max(0.0).min(window.size.width.0);
    let height = size.height.0.max(0.0).min(window.size.height.0);
    let max_x = window.right() - width;
    let max_y = window.bottom() - height;
    let x = anchor.left().clamp(window.left(), max_x);
    let below = anchor.bottom() + gap;
    let y = if below + height <= window.bottom() {
        below.max(window.top())
    } else {
        (anchor.top() - gap - height).clamp(window.top(), max_y)
    };
    LogicalRect::from_xywh(x, y, width, height)
}

/// What a popup did with an input event; `Consumed` events must not reach the
/// widgets underneath.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PopupEventResult {
    /// No popup is open, or the event is not the popup's business.
    Ignored,
    /// Pointer press inside the popup; the popup's own widgets handle it.
    Inside,
    /// The popup closed and the triggering event is consumed (Escape, or a
    /// press outside, which must not also click the control underneath).
    Dismissed,
    /// The popup closed but the event still applies normally (focus loss).
    ClosedPassThrough,
}

/// Owns the currently open popup. Painting and hit regions are emitted by the
/// application in `PaintLayer::Popup`; this state only defines identity,
/// anchor geometry and dismissal behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PopupManager {
    active: Option<PopupState>,
}

impl PopupManager {
    pub fn open(&mut self, id: PopupId, anchor: LogicalRect, bounds: LogicalRect) {
        self.active = Some(PopupState {
            id,
            anchor,
            bounds,
            restore_focus: None,
        });
    }

    /// Open a popup of `size` placed by [`anchored_placement`] and remember
    /// the currently focused widget so closing can give focus back to it.
    pub fn open_anchored(
        &mut self,
        id: PopupId,
        anchor: LogicalRect,
        size: LogicalSize,
        window: LogicalRect,
        gap: f64,
        input: &InputState,
    ) {
        self.active = Some(PopupState {
            id,
            anchor,
            bounds: anchored_placement(anchor, size, window, gap),
            restore_focus: input.focused(),
        });
    }

    /// Re-place the open popup after a window resize or DPI change so it stays
    /// attached to its anchor and inside the window.
    pub fn relayout(&mut self, window: LogicalRect, gap: f64) {
        if let Some(popup) = self.active.as_mut() {
            popup.bounds = anchored_placement(popup.anchor, popup.bounds.size, window, gap);
        }
    }

    /// Route one input event. Escape and a press outside dismiss (and consume
    /// the event); window focus loss closes without consuming. Closing
    /// restores keyboard focus to the widget that had it before the popup.
    pub fn handle(&mut self, event: &InputEvent, input: &mut InputState) -> PopupEventResult {
        if self.active.is_none() {
            return PopupEventResult::Ignored;
        }
        let result = match event {
            InputEvent::Key {
                key: Key::Escape,
                pressed: true,
                ..
            } => PopupEventResult::Dismissed,
            InputEvent::PointerDown(_) => {
                let inside = self
                    .active
                    .is_some_and(|popup| popup.bounds.contains(input.pointer));
                if inside {
                    return PopupEventResult::Inside;
                }
                PopupEventResult::Dismissed
            }
            InputEvent::WindowFocused(false) => PopupEventResult::ClosedPassThrough,
            _ => return PopupEventResult::Ignored,
        };
        self.close_restoring_focus(input);
        result
    }

    /// Close and hand keyboard focus back to the pre-popup widget, if it can
    /// still take it.
    pub fn close_restoring_focus(&mut self, input: &mut InputState) -> bool {
        let Some(popup) = self.close() else {
            return false;
        };
        if let Some(widget) = popup.restore_focus {
            input.focus.focus(Some(widget));
        }
        true
    }

    pub fn active(&self) -> Option<PopupState> {
        self.active
    }

    pub fn close(&mut self) -> Option<PopupState> {
        self.active.take()
    }

    /// Outside pointer-down dismisses and consumes that click, preventing it
    /// from activating an underlying control in the same event dispatch.
    pub fn pointer_down(&mut self, point: LogicalPoint) -> PopupPointerResult {
        let Some(popup) = self.active else {
            return PopupPointerResult::None;
        };
        if popup.bounds.contains(point) {
            PopupPointerResult::Inside
        } else {
            self.active = None;
            PopupPointerResult::DismissedOutside
        }
    }

    pub fn escape(&mut self) -> bool {
        self.close().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_uses_shared_bounds_and_dismisses_on_outside_click() {
        let mut popup = PopupManager::default();
        let anchor = LogicalRect::from_xywh(40.0, 20.0, 30.0, 24.0);
        let bounds = LogicalRect::from_xywh(40.0, 48.0, 120.0, 96.0);
        popup.open(PopupId(9), anchor, bounds);
        assert_eq!(popup.active().unwrap().anchor, anchor);
        assert_eq!(
            popup.pointer_down(LogicalPoint::new(50.0, 60.0)),
            PopupPointerResult::Inside
        );
        assert_eq!(
            popup.pointer_down(LogicalPoint::new(20.0, 20.0)),
            PopupPointerResult::DismissedOutside
        );
        assert!(popup.active().is_none());
        assert_eq!(
            popup.pointer_down(LogicalPoint::new(50.0, 60.0)),
            PopupPointerResult::None
        );
    }

    #[test]
    fn escape_closes_active_popup_only() {
        let mut popup = PopupManager::default();
        assert!(!popup.escape());
        popup.open(
            PopupId(3),
            LogicalRect::from_xywh(0.0, 0.0, 20.0, 20.0),
            LogicalRect::from_xywh(0.0, 20.0, 100.0, 100.0),
        );
        assert!(popup.escape());
        assert!(!popup.escape());
    }

    #[test]
    fn anchored_placement_flips_clamps_and_shrinks_into_the_window() {
        let window = LogicalRect::from_xywh(10.0, 20.0, 200.0, 120.0);
        let size = LogicalSize::new(80.0, 40.0);
        // Fits below.
        let below = anchored_placement(
            LogicalRect::from_xywh(30.0, 30.0, 20.0, 20.0),
            size,
            window,
            4.0,
        );
        assert_eq!(below, LogicalRect::from_xywh(30.0, 54.0, 80.0, 40.0));
        // No room below: flips above the anchor.
        let above = anchored_placement(
            LogicalRect::from_xywh(30.0, 110.0, 20.0, 20.0),
            size,
            window,
            4.0,
        );
        assert_eq!(above, LogicalRect::from_xywh(30.0, 66.0, 80.0, 40.0));
        // Anchor near the right edge: shifted left so the panel stays visible.
        let right = anchored_placement(
            LogicalRect::from_xywh(200.0, 30.0, 20.0, 20.0),
            size,
            window,
            4.0,
        );
        assert_eq!(right.right(), window.right());
        // Anchor outside the window and a panel larger than the window.
        let huge = anchored_placement(
            LogicalRect::from_xywh(-500.0, 900.0, 20.0, 20.0),
            LogicalSize::new(1000.0, 1000.0),
            window,
            4.0,
        );
        assert_eq!(huge, window);
        let zero = anchored_placement(LogicalRect::ZERO, size, LogicalRect::ZERO, 4.0);
        assert_eq!(zero.size, LogicalSize::ZERO);
    }

    fn key_down(key: Key) -> InputEvent {
        InputEvent::Key {
            key,
            pressed: true,
            modifiers: crate::Modifiers::default(),
        }
    }

    #[test]
    fn escape_outside_press_and_focus_loss_dismiss_and_restore_focus() {
        let window = LogicalRect::from_xywh(0.0, 0.0, 300.0, 200.0);
        let anchor = LogicalRect::from_xywh(20.0, 20.0, 40.0, 24.0);
        for closing in [
            InputEvent::Key {
                key: Key::Escape,
                pressed: true,
                modifiers: crate::Modifiers::default(),
            },
            InputEvent::PointerDown(crate::MouseButton::Primary),
            InputEvent::WindowFocused(false),
        ] {
            let mut input = InputState::default();
            input
                .focus
                .set_tab_order([(WidgetId(1), true, true), (WidgetId(2), true, true)]);
            input.set_focus(Some(WidgetId(2)));
            input.pointer = LogicalPoint::new(290.0, 190.0);
            let mut popup = PopupManager::default();
            popup.open_anchored(
                PopupId(1),
                anchor,
                LogicalSize::new(100.0, 60.0),
                window,
                4.0,
                &input,
            );
            // Focus moved into the popup content while it was open.
            input.set_focus(Some(WidgetId(1)));
            let result = popup.handle(&closing, &mut input);
            assert!(popup.active().is_none(), "{closing:?} must close the popup");
            assert_eq!(
                input.focused(),
                Some(WidgetId(2)),
                "{closing:?} must restore focus"
            );
            let expected = if matches!(closing, InputEvent::WindowFocused(_)) {
                PopupEventResult::ClosedPassThrough
            } else {
                PopupEventResult::Dismissed
            };
            assert_eq!(result, expected);
        }
    }

    #[test]
    fn inside_press_keeps_the_popup_and_unrelated_events_are_ignored() {
        let window = LogicalRect::from_xywh(0.0, 0.0, 300.0, 200.0);
        let mut input = InputState::default();
        let mut popup = PopupManager::default();
        assert_eq!(
            popup.handle(&key_down(Key::Escape), &mut input),
            PopupEventResult::Ignored
        );
        popup.open_anchored(
            PopupId(1),
            LogicalRect::from_xywh(20.0, 20.0, 40.0, 24.0),
            LogicalSize::new(100.0, 60.0),
            window,
            4.0,
            &input,
        );
        let bounds = popup.active().unwrap().bounds;
        input.pointer = LogicalPoint::new(bounds.left() + 5.0, bounds.top() + 5.0);
        assert_eq!(
            popup.handle(
                &InputEvent::PointerDown(crate::MouseButton::Primary),
                &mut input
            ),
            PopupEventResult::Inside
        );
        for event in [
            key_down(Key::Character('a')),
            InputEvent::PointerMoved(LogicalPoint::new(1.0, 1.0)),
            InputEvent::WindowFocused(true),
        ] {
            assert_eq!(popup.handle(&event, &mut input), PopupEventResult::Ignored);
            assert!(popup.active().is_some());
        }
    }

    #[test]
    fn relayout_keeps_the_popup_attached_and_inside_a_resized_window() {
        let input = InputState::default();
        let mut popup = PopupManager::default();
        let anchor = LogicalRect::from_xywh(180.0, 20.0, 40.0, 24.0);
        popup.open_anchored(
            PopupId(1),
            anchor,
            LogicalSize::new(100.0, 60.0),
            LogicalRect::from_xywh(0.0, 0.0, 400.0, 300.0),
            4.0,
            &input,
        );
        assert_eq!(popup.active().unwrap().bounds.left(), 180.0);
        let smaller = LogicalRect::from_xywh(0.0, 0.0, 220.0, 70.0);
        popup.relayout(smaller, 4.0);
        let bounds = popup.active().unwrap().bounds;
        assert_eq!(bounds.right(), smaller.right());
        assert!(bounds.bottom() <= smaller.bottom() && bounds.top() >= smaller.top());
    }
}
