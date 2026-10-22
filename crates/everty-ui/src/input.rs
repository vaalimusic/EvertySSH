//! OS-neutral input routing. Platform adapters convert native events to these
//! logical events; widgets receive one target, never a broadcast.

use std::collections::{BTreeSet, HashSet};

use crate::{LogicalPoint, LogicalPxValue, PaintLayer};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WidgetId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MouseButton {
    Primary,
    Secondary,
    Middle,
    Other(u16),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Key {
    Tab,
    Escape,
    Enter,
    Space,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Backspace,
    Delete,
    Character(char),
    Other(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ImeEvent {
    Start,
    Update {
        text: String,
        selection: Option<(usize, usize)>,
    },
    Commit(String),
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    PointerMoved(LogicalPoint),
    PointerDown(MouseButton),
    PointerUp(MouseButton),
    /// A higher layer took pointer ownership; discard capture without activation.
    PointerCancel,
    Wheel {
        delta: (LogicalPxValue, LogicalPxValue),
        trackpad: bool,
    },
    Key {
        key: Key,
        pressed: bool,
        modifiers: Modifiers,
    },
    ModifiersChanged(Modifiers),
    Text(String),
    Ime(ImeEvent),
    WindowFocused(bool),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitRegion {
    pub id: WidgetId,
    pub rect: crate::LogicalRect,
    pub layer: PaintLayer,
    /// Later regions in a layer are painted on top of earlier ones.
    pub paint_order: u64,
    pub enabled: bool,
    pub focusable: bool,
    pub cursor: crate::CursorIcon,
}

impl HitRegion {
    /// Restrict the region to the part of it that is actually painted. Content
    /// scrolled partly out of a clip must not receive pointer input in the
    /// clipped-away area, where another widget is visible instead.
    pub fn clipped_to(mut self, clip: crate::LogicalRect) -> Self {
        self.rect = self.rect.clip(clip);
        self
    }

    pub fn hit_test(point: LogicalPoint, regions: &[Self]) -> Option<Self> {
        regions
            .iter()
            .filter(|region| region.enabled && region.rect.contains(point))
            .max_by_key(|region| (region.layer, region.paint_order))
            .copied()
    }
}

/// Resolve the local UI cursor from the same z-ordered hit regions used for
/// pointer input. The platform adapter maps this semantic icon to a native
/// cursor; remote-desktop cursor rendering is outside this API.
pub fn resolve_cursor(point: LogicalPoint, regions: &[HitRegion]) -> crate::CursorIcon {
    HitRegion::hit_test(point, regions)
        .map(|region| region.cursor)
        .unwrap_or_default()
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoutedEvent {
    pub target: Option<WidgetId>,
    pub event: InputEvent,
    /// True for toolkit-owned navigation such as Tab traversal.
    pub consumed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FocusManager {
    eligible: Vec<WidgetId>,
    focused: Option<WidgetId>,
}

impl FocusManager {
    pub fn set_tab_order(&mut self, order: impl IntoIterator<Item = (WidgetId, bool, bool)>) {
        self.eligible.clear();
        let mut seen = HashSet::new();
        for (id, focusable, enabled) in order {
            if focusable && enabled && seen.insert(id) {
                self.eligible.push(id);
            }
        }
        if self.focused.is_some_and(|focused| !seen.contains(&focused)) {
            self.focused = None;
        }
    }

    pub fn focused(&self) -> Option<WidgetId> {
        self.focused
    }

    pub fn focus(&mut self, id: Option<WidgetId>) -> bool {
        if id.is_none() || id.is_some_and(|id| self.eligible.contains(&id)) {
            self.focused = id;
            true
        } else {
            false
        }
    }

    fn focus_from_pointer(&mut self, id: WidgetId) {
        self.focused = Some(id);
    }

    pub fn focus_next(&mut self, reverse: bool) -> Option<WidgetId> {
        if self.eligible.is_empty() {
            self.focused = None;
            return None;
        }
        let current = self
            .focused
            .and_then(|id| self.eligible.iter().position(|item| *item == id));
        let next = match (current, reverse) {
            (Some(0), true) | (None, true) => self.eligible.len() - 1,
            (Some(index), true) => index - 1,
            (Some(index), false) => (index + 1) % self.eligible.len(),
            (None, false) => 0,
        };
        self.focused = Some(self.eligible[next]);
        self.focused
    }

    pub fn remove(&mut self, id: WidgetId) {
        self.eligible.retain(|item| *item != id);
        if self.focused == Some(id) {
            self.focused = None;
        }
    }
}

/// Counts consecutive clicks (1 = single, 2 = double, ...). The platform adapter
/// supplies a monotonic timestamp, so the logic is deterministic and testable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClickTracker {
    pub interval: std::time::Duration,
    /// Maximum pointer travel, in logical pixels, between clicks of one series.
    pub slop: f64,
    last: Option<(std::time::Duration, LogicalPoint, MouseButton, u32)>,
}

impl Default for ClickTracker {
    fn default() -> Self {
        Self {
            interval: std::time::Duration::from_millis(500),
            slop: 4.0,
            last: None,
        }
    }
}

impl ClickTracker {
    /// Record a button press and return its click count within the series.
    pub fn register(
        &mut self,
        now: std::time::Duration,
        point: LogicalPoint,
        button: MouseButton,
    ) -> u32 {
        let count = match self.last {
            Some((at, last_point, last_button, count))
                if last_button == button
                    && now >= at
                    && now - at <= self.interval
                    && (point.x.0 - last_point.x.0).hypot(point.y.0 - last_point.y.0)
                        <= self.slop =>
            {
                count.saturating_add(1)
            }
            _ => 1,
        };
        self.last = Some((now, point, button, count));
        count
    }

    /// Forget the current series (focus loss, DPI change, popup dismissal).
    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[derive(Clone, Debug, Default)]
pub struct InputState {
    pub pointer: LogicalPoint,
    pub buttons_down: BTreeSet<MouseButton>,
    pub modifiers: Modifiers,
    pub pointer_capture: Option<WidgetId>,
    pub hovered: Option<WidgetId>,
    pub focus: FocusManager,
    pub window_focused: bool,
    pub ime_active: bool,
}

impl InputState {
    pub fn focused(&self) -> Option<WidgetId> {
        self.focus.focused()
    }

    pub fn set_focus(&mut self, id: Option<WidgetId>) {
        self.focus.focus(id);
    }

    pub fn capture_pointer(&mut self, id: WidgetId) {
        self.pointer_capture = Some(id);
    }

    pub fn release_pointer(&mut self, id: WidgetId) -> bool {
        if self.pointer_capture == Some(id) {
            self.pointer_capture = None;
            true
        } else {
            false
        }
    }

    pub fn route(&mut self, event: InputEvent, regions: &[HitRegion]) -> RoutedEvent {
        let mut consumed = false;
        let mut target = None;
        match &event {
            InputEvent::PointerMoved(position) => {
                self.pointer = *position;
                self.hovered = HitRegion::hit_test(*position, regions).map(|region| region.id);
                target = self.pointer_capture.or(self.hovered);
            }
            InputEvent::PointerDown(button) => {
                self.buttons_down.insert(*button);
                target = self
                    .pointer_capture
                    .or_else(|| HitRegion::hit_test(self.pointer, regions).map(|region| region.id));
                if let Some(region) = HitRegion::hit_test(self.pointer, regions) {
                    if region.focusable {
                        self.focus.focus_from_pointer(region.id);
                    }
                }
            }
            InputEvent::PointerUp(button) => {
                target = self
                    .pointer_capture
                    .or_else(|| HitRegion::hit_test(self.pointer, regions).map(|region| region.id));
                self.buttons_down.remove(button);
                if self.buttons_down.is_empty() {
                    self.pointer_capture = None;
                }
            }
            InputEvent::PointerCancel => {
                self.buttons_down.clear();
                self.pointer_capture = None;
                self.hovered = None;
            }
            InputEvent::Wheel { .. } => {
                target = HitRegion::hit_test(self.pointer, regions).map(|region| region.id);
            }
            InputEvent::Key {
                key: Key::Tab,
                pressed: true,
                modifiers,
            } => {
                if modifiers.control || modifiers.alt || modifiers.meta {
                    target = self.focus.focused();
                } else {
                    target = self.focus.route_tab(*modifiers);
                    consumed = target.is_some();
                }
            }
            InputEvent::Key { modifiers, .. } => {
                self.modifiers = *modifiers;
                target = self.focus.focused();
            }
            InputEvent::ModifiersChanged(modifiers) => self.modifiers = *modifiers,
            InputEvent::Text(_) => target = self.focus.focused(),
            InputEvent::Ime(ime) => {
                match ime {
                    ImeEvent::Start => self.ime_active = true,
                    ImeEvent::End => self.ime_active = false,
                    ImeEvent::Update { .. } | ImeEvent::Commit(_) => {}
                }
                target = self.focus.focused();
            }
            InputEvent::WindowFocused(focused) => {
                self.window_focused = *focused;
                if !focused {
                    // Key-up and button-up events are not delivered while the
                    // window is unfocused, so nothing may stay "held".
                    self.pointer_capture = None;
                    self.buttons_down.clear();
                    self.hovered = None;
                    self.modifiers = Modifiers::default();
                }
            }
        }
        RoutedEvent {
            target,
            event,
            consumed,
        }
    }

    /// Call after the focused widget had its chance to handle a key. Remote
    /// input is eligible only when UI did not consume it and the viewport owns
    /// focus; the caller then forwards the original key event if this is true.
    pub fn remote_key_fallback_allowed(ui_consumed: bool, remote_viewport_focused: bool) -> bool {
        !ui_consumed && remote_viewport_focused
    }
}

impl FocusManager {
    pub fn route_tab(&mut self, modifiers: Modifiers) -> Option<WidgetId> {
        self.focus_next(modifiers.shift)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogicalRect;

    fn region(id: u64, x: f64, layer: PaintLayer, order: u64) -> HitRegion {
        HitRegion {
            id: WidgetId(id),
            rect: LogicalRect::from_xywh(x, 0.0, 20.0, 20.0),
            layer,
            paint_order: order,
            enabled: true,
            focusable: true,
            cursor: crate::CursorIcon::Default,
        }
    }

    #[test]
    fn hit_test_uses_top_paint_layer_and_order_and_skips_disabled() {
        let top = region(2, 0.0, PaintLayer::Popup, 0);
        let mut disabled = region(3, 0.0, PaintLayer::Overlay, 10);
        disabled.enabled = false;
        let regions = [region(1, 0.0, PaintLayer::Content, 99), top, disabled];
        assert_eq!(
            HitRegion::hit_test(LogicalPoint::new(5.0, 5.0), &regions)
                .unwrap()
                .id,
            WidgetId(2)
        );
    }

    #[test]
    fn cursor_resolver_uses_the_same_topmost_enabled_hit_region() {
        let mut content = region(1, 0.0, PaintLayer::Content, 0);
        content.cursor = crate::CursorIcon::Pointer;
        let mut popup = region(2, 0.0, PaintLayer::Popup, 0);
        popup.cursor = crate::CursorIcon::ResizeHorizontal;
        let point = LogicalPoint::new(5.0, 5.0);
        assert_eq!(
            resolve_cursor(point, &[content, popup]),
            crate::CursorIcon::ResizeHorizontal
        );

        let mut disabled_popup = popup;
        disabled_popup.enabled = false;
        assert_eq!(
            resolve_cursor(point, &[content, disabled_popup]),
            crate::CursorIcon::Pointer
        );
        assert_eq!(
            resolve_cursor(LogicalPoint::new(50.0, 50.0), &[content]),
            crate::CursorIcon::Default
        );
    }

    #[test]
    fn capture_routes_drag_outside_and_mouse_up_releases_it() {
        let regions = [region(1, 0.0, PaintLayer::Content, 0)];
        let mut input = InputState::default();
        input.route(
            InputEvent::PointerMoved(LogicalPoint::new(5.0, 5.0)),
            &regions,
        );
        input.focus.set_tab_order([(WidgetId(1), true, true)]);
        input.route(InputEvent::PointerDown(MouseButton::Primary), &regions);
        assert_eq!(input.focused(), Some(WidgetId(1)));
        assert_eq!(input.focus.focused(), Some(WidgetId(1)));
        input.capture_pointer(WidgetId(1));
        let moved = input.route(
            InputEvent::PointerMoved(LogicalPoint::new(100.0, 100.0)),
            &regions,
        );
        assert_eq!(moved.target, Some(WidgetId(1)));
        let up = input.route(InputEvent::PointerUp(MouseButton::Primary), &regions);
        assert_eq!(up.target, Some(WidgetId(1)));
        assert_eq!(input.pointer_capture, None);
    }

    #[test]
    fn pointer_cancel_releases_capture_without_stealing_keyboard_focus() {
        let regions = [region(1, 0.0, PaintLayer::Content, 0)];
        let mut input = InputState::default();
        input.focus.set_tab_order([(WidgetId(1), true, true)]);
        input.route(
            InputEvent::PointerMoved(LogicalPoint::new(5.0, 5.0)),
            &regions,
        );
        input.route(InputEvent::PointerDown(MouseButton::Primary), &regions);
        input.capture_pointer(WidgetId(1));
        assert_eq!(input.focused(), Some(WidgetId(1)));

        input.route(InputEvent::PointerCancel, &regions);
        assert!(input.buttons_down.is_empty());
        assert_eq!(input.pointer_capture, None);
        assert_eq!(input.hovered, None);
        assert_eq!(input.focused(), Some(WidgetId(1)));
    }

    #[test]
    fn focus_tab_and_remote_keyboard_fallback_are_explicit() {
        let mut input = InputState::default();
        input.focus.set_tab_order([
            (WidgetId(1), true, true),
            (WidgetId(2), false, true),
            (WidgetId(3), true, true),
        ]);
        assert_eq!(
            input.focus.route_tab(Modifiers::default()),
            Some(WidgetId(1))
        );
        assert_eq!(
            input.focus.route_tab(Modifiers::default()),
            Some(WidgetId(3))
        );
        assert_eq!(
            input.focus.route_tab(Modifiers {
                shift: true,
                ..Modifiers::default()
            }),
            Some(WidgetId(1))
        );
        let tab = input.route(
            InputEvent::Key {
                key: Key::Tab,
                pressed: true,
                modifiers: Modifiers::default(),
            },
            &[],
        );
        assert!(tab.consumed);
        assert_eq!(tab.target, Some(WidgetId(3)));
        let key = input.route(
            InputEvent::Key {
                key: Key::Character('a'),
                pressed: true,
                modifiers: Modifiers::default(),
            },
            &[],
        );
        assert_eq!(key.target, Some(WidgetId(3)));
        assert!(!InputState::remote_key_fallback_allowed(true, true));
        assert!(!InputState::remote_key_fallback_allowed(false, false));
        assert!(InputState::remote_key_fallback_allowed(false, true));
    }

    #[test]
    fn ime_and_window_focus_loss_are_preserved_and_cleanup_capture() {
        let mut input = InputState::default();
        input.focus.set_tab_order([(WidgetId(7), true, true)]);
        input.set_focus(Some(WidgetId(7)));
        let ime = ImeEvent::Update {
            text: "тест".to_owned(),
            selection: Some((1, 3)),
        };
        assert_eq!(
            input.route(InputEvent::Ime(ime.clone()), &[]).event,
            InputEvent::Ime(ime)
        );
        input.capture_pointer(WidgetId(7));
        input.buttons_down.insert(MouseButton::Primary);
        input.route(InputEvent::Ime(ImeEvent::Start), &[]);
        assert!(input.ime_active);
        input.route(InputEvent::WindowFocused(false), &[]);
        assert_eq!(input.pointer_capture, None);
        assert!(input.buttons_down.is_empty());
    }

    /// Tiny deterministic generator so the property tests are reproducible.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, bound: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % bound
        }
    }

    #[test]
    fn hit_test_order_equals_paint_order_for_random_overlapping_scenes() {
        use crate::{Color, PaintBuilder, PaintOp};
        let layers = [
            PaintLayer::Background,
            PaintLayer::Content,
            PaintLayer::Floating,
            PaintLayer::Popup,
            PaintLayer::Tooltip,
            PaintLayer::Overlay,
        ];
        let mut random = Lcg(0x5eed);
        for scene in 0..300 {
            let mut builder = PaintBuilder::new();
            let mut regions = Vec::new();
            let mut enabled = Vec::new();
            let clip = LogicalRect::from_xywh(5.0, 5.0, 60.0, 50.0);
            for id in 0..8u64 {
                let layer = layers[random.next(layers.len() as u64) as usize];
                let rect = LogicalRect::from_xywh(
                    random.next(50) as f64,
                    random.next(40) as f64,
                    5.0 + random.next(40) as f64,
                    5.0 + random.next(30) as f64,
                );
                let is_clipped = random.next(3) == 0;
                builder.set_layer(layer);
                if is_clipped {
                    builder.push_clip(clip);
                }
                let order = builder.paint_order();
                // Distinct alpha doubles as the widget identity in the paint list.
                builder.fill_rect(rect, Color::rgba(0, 0, 0, 10 + id as u8), 0.0);
                if is_clipped {
                    builder.pop_clip().unwrap();
                }
                let mut region = HitRegion {
                    id: WidgetId(id),
                    rect,
                    layer,
                    paint_order: order,
                    enabled: random.next(5) != 0,
                    focusable: true,
                    cursor: crate::CursorIcon::Default,
                };
                if is_clipped {
                    region = region.clipped_to(clip);
                }
                enabled.push(region.enabled);
                regions.push(region);
            }
            let mut paint = builder.finish();
            paint.sort_by_layer();
            for y in 0..60 {
                for x in 0..100 {
                    let point = LogicalPoint::new(f64::from(x) + 0.25, f64::from(y) + 0.25);
                    let topmost_painted_enabled = paint
                        .commands()
                        .iter()
                        .filter_map(|command| {
                            let PaintOp::FillRect { rect, color, .. } = command.op else {
                                return None;
                            };
                            let id = usize::from(color.0[3] - 10);
                            let visible = command.clip.map_or(rect, |clip| rect.clip(clip));
                            (visible.contains(point) && enabled[id]).then_some(WidgetId(id as u64))
                        })
                        .next_back();
                    assert_eq!(
                        HitRegion::hit_test(point, &regions).map(|region| region.id),
                        topmost_painted_enabled,
                        "scene {scene} at ({x},{y})"
                    );
                }
            }
        }
    }

    #[test]
    fn window_focus_loss_clears_held_modifiers_buttons_and_capture() {
        let mut input = InputState::default();
        input.route(
            InputEvent::ModifiersChanged(Modifiers {
                shift: true,
                control: true,
                ..Modifiers::default()
            }),
            &[],
        );
        assert!(input.modifiers.shift);
        input.route(InputEvent::WindowFocused(false), &[]);
        assert_eq!(input.modifiers, Modifiers::default());
        // A key-up lost while unfocused must not leave Shift latched for Tab.
        input
            .focus
            .set_tab_order([(WidgetId(1), true, true), (WidgetId(2), true, true)]);
        input.route(InputEvent::WindowFocused(true), &[]);
        let tab = input.route(
            InputEvent::Key {
                key: Key::Tab,
                pressed: true,
                modifiers: input.modifiers,
            },
            &[],
        );
        assert_eq!(tab.target, Some(WidgetId(1)), "forward traversal expected");
    }

    #[test]
    fn keyboard_text_and_ime_reach_only_the_focused_widget_never_the_hovered_one() {
        let hovered = region(1, 0.0, PaintLayer::Content, 0);
        let regions = [hovered];
        let mut input = InputState::default();
        input
            .focus
            .set_tab_order([(WidgetId(1), true, true), (WidgetId(9), true, true)]);
        input.route(
            InputEvent::PointerMoved(LogicalPoint::new(5.0, 5.0)),
            &regions,
        );
        assert_eq!(input.hovered, Some(WidgetId(1)));
        let events = [
            InputEvent::Key {
                key: Key::Character('x'),
                pressed: true,
                modifiers: Modifiers::default(),
            },
            InputEvent::Key {
                key: Key::Escape,
                pressed: true,
                modifiers: Modifiers::default(),
            },
            InputEvent::Text("я".to_owned()),
            InputEvent::Ime(ImeEvent::Commit("я".to_owned())),
        ];
        for event in &events {
            assert_eq!(
                input.route(event.clone(), &regions).target,
                None,
                "no focus: {event:?} must not fall through to the hovered widget"
            );
        }
        input.set_focus(Some(WidgetId(9)));
        for event in &events {
            assert_eq!(
                input.route(event.clone(), &regions).target,
                Some(WidgetId(9))
            );
        }
    }

    #[test]
    fn popup_layer_wins_hit_tests_and_pointer_capture_survives_overlap() {
        let content = region(1, 0.0, PaintLayer::Content, 5);
        let popup = region(2, 10.0, PaintLayer::Popup, 0);
        let regions = [content, popup];
        let mut input = InputState::default();
        input.route(
            InputEvent::PointerMoved(LogicalPoint::new(15.0, 5.0)),
            &regions,
        );
        assert_eq!(input.hovered, Some(WidgetId(2)));
        input.route(
            InputEvent::PointerMoved(LogicalPoint::new(5.0, 5.0)),
            &regions,
        );
        input.route(InputEvent::PointerDown(MouseButton::Primary), &regions);
        input.capture_pointer(WidgetId(1));
        // Dragging over the popup keeps delivering to the capturing widget.
        let moved = input.route(
            InputEvent::PointerMoved(LogicalPoint::new(15.0, 5.0)),
            &regions,
        );
        assert_eq!(moved.target, Some(WidgetId(1)));
        assert_eq!(input.hovered, Some(WidgetId(2)));
        let up = input.route(InputEvent::PointerUp(MouseButton::Primary), &regions);
        assert_eq!(up.target, Some(WidgetId(1)));
        assert_eq!(input.pointer_capture, None);
    }

    #[test]
    fn tab_order_is_deduplicated_in_linear_time_for_large_trees() {
        let mut focus = FocusManager::default();
        let order = (0..20_000u64)
            .map(|id| (WidgetId(id % 10_000), true, true))
            .chain([(WidgetId(7), false, true), (WidgetId(8), true, false)]);
        focus.set_tab_order(order);
        assert_eq!(focus.focus_next(false), Some(WidgetId(0)));
        assert_eq!(focus.focus_next(true), Some(WidgetId(9_999)));
    }

    #[test]
    fn click_tracker_counts_double_clicks_by_time_distance_and_button() {
        use std::time::Duration;
        let mut tracker = ClickTracker::default();
        let at = LogicalPoint::new(10.0, 10.0);
        let ms = Duration::from_millis;
        assert_eq!(tracker.register(ms(0), at, MouseButton::Primary), 1);
        assert_eq!(tracker.register(ms(300), at, MouseButton::Primary), 2);
        assert_eq!(tracker.register(ms(600), at, MouseButton::Primary), 3);
        assert_eq!(
            tracker.register(ms(1200), at, MouseButton::Primary),
            1,
            "too slow"
        );
        assert_eq!(
            tracker.register(
                ms(1300),
                LogicalPoint::new(30.0, 10.0),
                MouseButton::Primary
            ),
            1,
            "moved too far"
        );
        assert_eq!(
            tracker.register(
                ms(1400),
                LogicalPoint::new(30.0, 10.0),
                MouseButton::Secondary
            ),
            1
        );
        assert_eq!(
            tracker.register(
                ms(1500),
                LogicalPoint::new(30.0, 10.0),
                MouseButton::Secondary
            ),
            2
        );
        tracker.reset();
        assert_eq!(
            tracker.register(
                ms(1550),
                LogicalPoint::new(30.0, 10.0),
                MouseButton::Secondary
            ),
            1
        );
    }
}
