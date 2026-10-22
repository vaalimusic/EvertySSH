//! Reusable two-pane layout with pointer-captured splitter dragging.

use crate::{
    CursorIcon, HitRegion, InputEvent, InputState, LogicalRect, MainAxis, PaintLayer, WidgetId,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitPane {
    pub id: WidgetId,
    pub axis: MainAxis,
    pub ratio: f64,
    pub first_min: f64,
    pub second_min: f64,
    pub splitter_extent: f64,
    dragging: bool,
    /// Pointer position relative to the splitter centre when the drag began,
    /// so grabbing the bar's edge does not make it jump under the pointer.
    grab_offset: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitPaneLayout {
    pub first: LogicalRect,
    pub splitter: LogicalRect,
    pub second: LogicalRect,
}

impl SplitPane {
    pub fn new(id: WidgetId, axis: MainAxis, ratio: f64) -> Self {
        Self {
            id,
            axis,
            ratio: normalized_ratio(ratio),
            first_min: 0.0,
            second_min: 0.0,
            splitter_extent: 6.0,
            dragging: false,
            grab_offset: 0.0,
        }
    }

    pub fn with_min_sizes(mut self, first: f64, second: f64) -> Self {
        self.first_min = non_negative(first);
        self.second_min = non_negative(second);
        self
    }

    pub fn with_splitter_extent(mut self, extent: f64) -> Self {
        self.splitter_extent = non_negative(extent);
        self
    }

    pub fn layout(&self, bounds: LogicalRect) -> SplitPaneLayout {
        let total = match self.axis {
            MainAxis::Horizontal => bounds.size.width.0,
            MainAxis::Vertical => bounds.size.height.0,
        };
        let extent = self.splitter_extent.min(total.max(0.0));
        let available = (total - extent).max(0.0);
        let ratio = self.clamped_ratio(self.ratio, available);
        let first_extent = available * ratio;
        let second_extent = (available - first_extent).max(0.0);
        match self.axis {
            MainAxis::Horizontal => SplitPaneLayout {
                first: LogicalRect::from_xywh(
                    bounds.left(),
                    bounds.top(),
                    first_extent,
                    bounds.size.height.0,
                ),
                splitter: LogicalRect::from_xywh(
                    bounds.left() + first_extent,
                    bounds.top(),
                    extent,
                    bounds.size.height.0,
                ),
                second: LogicalRect::from_xywh(
                    bounds.left() + first_extent + extent,
                    bounds.top(),
                    second_extent,
                    bounds.size.height.0,
                ),
            },
            MainAxis::Vertical => SplitPaneLayout {
                first: LogicalRect::from_xywh(
                    bounds.left(),
                    bounds.top(),
                    bounds.size.width.0,
                    first_extent,
                ),
                splitter: LogicalRect::from_xywh(
                    bounds.left(),
                    bounds.top() + first_extent,
                    bounds.size.width.0,
                    extent,
                ),
                second: LogicalRect::from_xywh(
                    bounds.left(),
                    bounds.top() + first_extent + extent,
                    bounds.size.width.0,
                    second_extent,
                ),
            },
        }
    }

    /// Hit region for the splitter bar, widened by `grab_slop` logical pixels
    /// on both sides along the split axis so a thin bar stays easy to grab.
    /// Registering this region (rather than testing the pointer here) keeps
    /// the drag subject to the same z-order as every other widget.
    pub fn hit_region(
        &self,
        bounds: LogicalRect,
        grab_slop: f64,
        layer: PaintLayer,
        paint_order: u64,
    ) -> HitRegion {
        let bar = self.layout(bounds).splitter;
        let slop = non_negative(grab_slop);
        let (rect, cursor) = match self.axis {
            MainAxis::Horizontal => (
                LogicalRect::from_xywh(
                    bar.left() - slop,
                    bar.top(),
                    bar.size.width.0 + 2.0 * slop,
                    bar.size.height.0,
                ),
                CursorIcon::ResizeHorizontal,
            ),
            MainAxis::Vertical => (
                LogicalRect::from_xywh(
                    bar.left(),
                    bar.top() - slop,
                    bar.size.width.0,
                    bar.size.height.0 + 2.0 * slop,
                ),
                CursorIcon::ResizeVertical,
            ),
        };
        HitRegion {
            id: self.id,
            rect: rect.clip(bounds),
            layer,
            paint_order,
            enabled: true,
            focusable: false,
            cursor,
        }
    }

    /// `target` is the routed target of `event`: a drag starts only when the
    /// topmost hit region under the pointer is this splitter.
    pub fn handle(
        &mut self,
        event: &InputEvent,
        target: Option<WidgetId>,
        input: &mut InputState,
        bounds: LogicalRect,
    ) -> bool {
        match event {
            InputEvent::PointerDown(crate::MouseButton::Primary) if target == Some(self.id) => {
                let bar = self.layout(bounds).splitter;
                self.grab_offset = match self.axis {
                    MainAxis::Horizontal => {
                        input.pointer.x.0 - (bar.left() + bar.size.width.0 / 2.0)
                    }
                    MainAxis::Vertical => input.pointer.y.0 - (bar.top() + bar.size.height.0 / 2.0),
                };
                self.dragging = true;
                input.capture_pointer(self.id);
                true
            }
            InputEvent::PointerMoved(_) if self.dragging => {
                if input.pointer_capture != Some(self.id) {
                    // Capture was taken away (overlay, reset): the drag is over.
                    self.dragging = false;
                    return false;
                }
                let total = match self.axis {
                    MainAxis::Horizontal => bounds.size.width.0,
                    MainAxis::Vertical => bounds.size.height.0,
                };
                let coordinate = match self.axis {
                    MainAxis::Horizontal => input.pointer.x.0 - bounds.left(),
                    MainAxis::Vertical => input.pointer.y.0 - bounds.top(),
                } - self.grab_offset;
                let extent = self.splitter_extent.min(total.max(0.0));
                let available = (total - extent).max(0.0);
                if available > 0.0 {
                    self.ratio =
                        self.clamped_ratio((coordinate - extent / 2.0) / available, available);
                }
                true
            }
            InputEvent::PointerUp(crate::MouseButton::Primary) if self.dragging => {
                self.dragging = false;
                input.release_pointer(self.id);
                true
            }
            InputEvent::PointerCancel | InputEvent::WindowFocused(false) => {
                if self.dragging {
                    self.dragging = false;
                    input.release_pointer(self.id);
                }
                false
            }
            _ => false,
        }
    }

    pub fn dragging(&self) -> bool {
        self.dragging
    }

    fn clamped_ratio(&self, proposed: f64, available: f64) -> f64 {
        if available <= 0.0 {
            return 0.5;
        }
        let ratio_min = self.first_min / available;
        let ratio_max = 1.0 - self.second_min / available;
        let (minimum, maximum) = if ratio_min <= ratio_max {
            (ratio_min, ratio_max)
        } else {
            // The available extent cannot satisfy both minimums. Keep the first
            // pane at its requested minimum and avoid a negative second pane.
            (ratio_min.clamp(0.0, 1.0), ratio_min.clamp(0.0, 1.0))
        };
        normalized_ratio(proposed).clamp(minimum, maximum)
    }
}

fn normalized_ratio(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.5
    }
}

fn non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LogicalPoint, LogicalSize, MouseButton};

    #[test]
    fn pane_respects_minimums_and_bounds_ratio() {
        let split = SplitPane::new(WidgetId(1), MainAxis::Horizontal, 0.9)
            .with_min_sizes(120.0, 160.0)
            .with_splitter_extent(8.0);
        let layout = split.layout(LogicalRect::from_xywh(10.0, 20.0, 500.0, 300.0));
        assert!(layout.first.size.width.0 >= 120.0);
        assert!(layout.second.size.width.0 >= 160.0);
        assert_eq!(
            layout.first.size.width.0 + layout.splitter.size.width.0 + layout.second.size.width.0,
            500.0
        );
        assert_eq!(layout.splitter.left(), layout.first.right());
    }

    #[test]
    fn splitter_drag_captures_pointer_and_clamps_to_minimum() {
        let bounds = LogicalRect::from_xywh(0.0, 0.0, 400.0, 200.0);
        let mut split =
            SplitPane::new(WidgetId(5), MainAxis::Horizontal, 0.5).with_min_sizes(100.0, 100.0);
        let bar = split.layout(bounds).splitter;
        let mut input = InputState {
            pointer: LogicalPoint::new(bar.left() + 2.0, 50.0),
            ..InputState::default()
        };
        assert!(split.handle(
            &InputEvent::PointerDown(MouseButton::Primary),
            Some(split.id),
            &mut input,
            bounds
        ));
        assert_eq!(input.pointer_capture, Some(split.id));
        input.pointer = LogicalPoint::new(390.0, 50.0);
        let pointer = input.pointer;
        assert!(split.handle(
            &InputEvent::PointerMoved(pointer),
            Some(split.id),
            &mut input,
            bounds
        ));
        let layout = split.layout(bounds);
        assert!(layout.second.size.width.0 >= 100.0);
        assert!(split.handle(
            &InputEvent::PointerUp(MouseButton::Primary),
            Some(split.id),
            &mut input,
            bounds
        ));
        assert!(!split.dragging());
        assert_eq!(input.pointer_capture, None);
    }

    #[test]
    fn pointer_cancel_stops_splitter_drag() {
        let bounds = LogicalRect::from_xywh(0.0, 0.0, 400.0, 200.0);
        let mut split = SplitPane::new(WidgetId(6), MainAxis::Horizontal, 0.5);
        let bar = split.layout(bounds).splitter;
        let mut input = InputState {
            pointer: LogicalPoint::new(bar.left() + 2.0, 50.0),
            ..InputState::default()
        };
        split.handle(
            &InputEvent::PointerDown(MouseButton::Primary),
            Some(split.id),
            &mut input,
            bounds,
        );
        assert!(split.dragging());
        let canceled = input.route(InputEvent::PointerCancel, &[]);
        split.handle(&canceled.event, canceled.target, &mut input, bounds);
        assert!(!split.dragging());
        assert_eq!(input.pointer_capture, None);
    }

    #[test]
    fn zero_available_space_stays_well_formed() {
        let split =
            SplitPane::new(WidgetId(3), MainAxis::Vertical, f64::NAN).with_splitter_extent(10.0);
        let layout = split.layout(LogicalRect::from_xywh(1.0, 2.0, 8.0, 5.0));
        assert_eq!(layout.first.size, LogicalSize::new(8.0, 0.0));
        assert_eq!(layout.second.size, LogicalSize::new(8.0, 0.0));
    }

    fn hits_with_popup_over_bar(split: &SplitPane, bounds: LogicalRect) -> [HitRegion; 2] {
        let bar = split.hit_region(bounds, 3.0, PaintLayer::Content, 0);
        let popup = HitRegion {
            id: WidgetId(900),
            rect: bar.rect,
            layer: PaintLayer::Popup,
            paint_order: 0,
            enabled: true,
            focusable: false,
            cursor: CursorIcon::Default,
        };
        [bar, popup]
    }

    #[test]
    fn a_popup_over_the_splitter_receives_the_press_instead_of_the_drag() {
        let bounds = LogicalRect::from_xywh(0.0, 0.0, 400.0, 200.0);
        let mut split = SplitPane::new(WidgetId(5), MainAxis::Horizontal, 0.5);
        let hits = hits_with_popup_over_bar(&split, bounds);
        let mut input = InputState::default();
        let centre = hits[0].rect.left() + hits[0].rect.size.width.0 / 2.0;
        input.route(
            InputEvent::PointerMoved(crate::LogicalPoint::new(centre, 50.0)),
            &hits,
        );
        let down = input.route(InputEvent::PointerDown(crate::MouseButton::Primary), &hits);
        assert_eq!(down.target, Some(WidgetId(900)));
        assert!(!split.handle(&down.event, down.target, &mut input, bounds));
        assert!(!split.dragging());
        assert_eq!(input.pointer_capture, None);
        // Without the popup the same press starts the drag.
        let bar_only = [hits[0]];
        let down = input.route(
            InputEvent::PointerDown(crate::MouseButton::Primary),
            &bar_only,
        );
        assert!(split.handle(&down.event, down.target, &mut input, bounds));
        assert!(split.dragging());
    }

    #[test]
    fn grabbing_the_bar_edge_does_not_jump_the_splitter() {
        for axis in [MainAxis::Horizontal, MainAxis::Vertical] {
            let bounds = LogicalRect::from_xywh(10.0, 20.0, 400.0, 300.0);
            let mut split = SplitPane::new(WidgetId(5), axis, 0.4)
                .with_min_sizes(50.0, 50.0)
                .with_splitter_extent(8.0);
            let hit = split.hit_region(bounds, 3.0, PaintLayer::Content, 0);
            let before = split.layout(bounds).splitter;
            // Press 3px from the bar centre, on its edge, then move 25px.
            let (px, py, dx, dy) = match axis {
                MainAxis::Horizontal => (before.left() + 1.0, 100.0, 25.0, 0.0),
                MainAxis::Vertical => (100.0, before.top() + 1.0, 0.0, 25.0),
            };
            let mut input = InputState::default();
            input.route(
                InputEvent::PointerMoved(crate::LogicalPoint::new(px, py)),
                &[hit],
            );
            let down = input.route(InputEvent::PointerDown(crate::MouseButton::Primary), &[hit]);
            split.handle(&down.event, down.target, &mut input, bounds);
            assert_eq!(
                split.layout(bounds).splitter,
                before,
                "press alone must not move the bar"
            );
            let moved = input.route(
                InputEvent::PointerMoved(crate::LogicalPoint::new(px + dx, py + dy)),
                &[hit],
            );
            split.handle(&moved.event, moved.target, &mut input, bounds);
            let after = split.layout(bounds).splitter;
            let travelled = match axis {
                MainAxis::Horizontal => after.left() - before.left(),
                MainAxis::Vertical => after.top() - before.top(),
            };
            assert!(
                (travelled - 25.0).abs() < 1e-9,
                "{axis:?}: moved {travelled}"
            );
        }
    }

    #[test]
    fn drag_ends_when_capture_is_taken_away_without_a_cancel_event() {
        let bounds = LogicalRect::from_xywh(0.0, 0.0, 400.0, 200.0);
        let mut split = SplitPane::new(WidgetId(5), MainAxis::Horizontal, 0.5);
        let hit = split.hit_region(bounds, 3.0, PaintLayer::Content, 0);
        let mut input = InputState::default();
        input.route(
            InputEvent::PointerMoved(crate::LogicalPoint::new(hit.rect.left() + 3.0, 10.0)),
            &[hit],
        );
        let down = input.route(InputEvent::PointerDown(crate::MouseButton::Primary), &[hit]);
        split.handle(&down.event, down.target, &mut input, bounds);
        assert!(split.dragging());
        input.pointer_capture = None; // e.g. an overlay reset the input state
        let ratio = split.ratio;
        let moved = input.route(
            InputEvent::PointerMoved(crate::LogicalPoint::new(350.0, 10.0)),
            &[hit],
        );
        assert!(!split.handle(&moved.event, moved.target, &mut input, bounds));
        assert!(!split.dragging());
        assert_eq!(split.ratio, ratio, "a dead drag must not keep resizing");
    }

    #[test]
    fn panes_always_tile_the_bounds_even_in_tiny_or_huge_windows() {
        for axis in [MainAxis::Horizontal, MainAxis::Vertical] {
            for total in [0.0, 1.0, 5.0, 6.0, 7.0, 100.0, 359.0, 1e6] {
                for (first_min, second_min) in [(0.0, 0.0), (180.0, 220.0), (1e7, 1e7)] {
                    for ratio in [0.0, 0.42, 1.0, f64::NAN] {
                        let split = SplitPane::new(WidgetId(1), axis, ratio)
                            .with_min_sizes(first_min, second_min);
                        let bounds = match axis {
                            MainAxis::Horizontal => LogicalRect::from_xywh(3.0, 4.0, total, 50.0),
                            MainAxis::Vertical => LogicalRect::from_xywh(3.0, 4.0, 50.0, total),
                        };
                        let layout = split.layout(bounds);
                        let extent = |rect: LogicalRect| match axis {
                            MainAxis::Horizontal => rect.size.width.0,
                            MainAxis::Vertical => rect.size.height.0,
                        };
                        for rect in [layout.first, layout.splitter, layout.second] {
                            assert!(extent(rect) >= 0.0 && extent(rect).is_finite());
                        }
                        let sum =
                            extent(layout.first) + extent(layout.splitter) + extent(layout.second);
                        assert!(
                            (sum - total).abs() < 1e-6 * total.max(1.0),
                            "{axis:?} total {total} mins {first_min}/{second_min}: {sum}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn ratio_survives_shrinking_and_growing_the_window_and_dpi_changes() {
        let split = SplitPane::new(WidgetId(1), MainAxis::Horizontal, 0.7)
            .with_min_sizes(120.0, 120.0)
            .with_splitter_extent(6.0);
        let wide = LogicalRect::from_xywh(0.0, 0.0, 800.0, 100.0);
        let before = split.layout(wide);
        // A narrow window forces the minimums but must not rewrite the ratio.
        let narrow = split.layout(LogicalRect::from_xywh(0.0, 0.0, 260.0, 100.0));
        assert_eq!(narrow.first.size.width.0, 134.0);
        assert!(narrow.second.size.width.0 >= 120.0 - 1e-9);
        assert_eq!(split.ratio, 0.7);
        assert_eq!(
            split.layout(wide),
            before,
            "growing back restores the layout"
        );
        // The ratio is dimensionless: snapped panes tile at every scale.
        for factor in [1.0f64, 1.25, 1.5, 1.75, 2.0] {
            let scale = crate::DpiScale::new(factor).unwrap();
            let snapped_first = scale.snap_rect(before.first);
            let snapped_bar = scale.snap_rect(before.splitter);
            let snapped_second = scale.snap_rect(before.second);
            let edge = |logical: f64| (logical * factor).round();
            assert_eq!(
                edge(snapped_first.right()),
                edge(snapped_bar.left()),
                "{factor}x"
            );
            assert_eq!(
                edge(snapped_bar.right()),
                edge(snapped_second.left()),
                "{factor}x"
            );
        }
    }
}
