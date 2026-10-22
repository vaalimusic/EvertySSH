//! Reusable scroll state and viewport/content geometry.

use crate::{LogicalPoint, LogicalPx, LogicalRect, LogicalSize};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollState {
    pub offset: LogicalPoint,
    pub viewport: LogicalSize,
    pub content: LogicalSize,
}

impl ScrollState {
    pub fn set_metrics(&mut self, viewport: LogicalSize, content: LogicalSize) {
        self.viewport = non_negative_size(viewport);
        self.content = non_negative_size(content);
        self.clamp();
    }

    pub fn set_offset(&mut self, offset: LogicalPoint) {
        self.offset = offset;
        self.clamp();
    }

    /// Scroll by a wheel/trackpad delta. NaN components are ignored and
    /// infinite ones saturate at the corresponding end. Returns whether the
    /// offset changed, so callers can skip a redraw when already at a limit.
    pub fn scroll_by(&mut self, delta: LogicalPoint) -> bool {
        let before = self.offset;
        if !delta.x.0.is_nan() {
            self.offset.x.0 += delta.x.0;
        }
        if !delta.y.0.is_nan() {
            self.offset.y.0 += delta.y.0;
        }
        self.clamp();
        self.offset != before
    }

    pub fn ensure_visible(&mut self, item: LogicalRect) {
        if [item.left(), item.top(), item.right(), item.bottom()]
            .iter()
            .any(|value| value.is_nan())
        {
            return;
        }
        if item.left() < self.offset.x.0 {
            self.offset.x.0 = item.left();
        } else if item.right() > self.offset.x.0 + self.viewport.width.0 {
            self.offset.x.0 = item.right() - self.viewport.width.0;
        }
        if item.top() < self.offset.y.0 {
            self.offset.y.0 = item.top();
        } else if item.bottom() > self.offset.y.0 + self.viewport.height.0 {
            self.offset.y.0 = item.bottom() - self.viewport.height.0;
        }
        self.clamp();
    }

    pub fn max_offset(&self) -> LogicalPoint {
        LogicalPoint::new(
            (self.content.width.0 - self.viewport.width.0).max(0.0),
            (self.content.height.0 - self.viewport.height.0).max(0.0),
        )
    }

    pub fn clamp(&mut self) {
        let max = self.max_offset();
        self.offset = LogicalPoint::new(
            finite(self.offset.x.0).clamp(0.0, max.x.0),
            finite(self.offset.y.0).clamp(0.0, max.y.0),
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollAreaLayout {
    pub viewport: LogicalRect,
    pub content: LogicalRect,
    pub clip: LogicalRect,
    pub vertical_scrollbar: Option<LogicalRect>,
    pub horizontal_scrollbar: Option<LogicalRect>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollArea {
    pub state: ScrollState,
    pub scrollbar_extent: LogicalPx,
    pub minimum_thumb: LogicalPx,
}

impl Default for ScrollArea {
    fn default() -> Self {
        Self {
            state: ScrollState::default(),
            scrollbar_extent: LogicalPx(10.0),
            minimum_thumb: LogicalPx(18.0),
        }
    }
}

impl ScrollArea {
    pub fn layout(&mut self, bounds: LogicalRect, content_size: LogicalSize) -> ScrollAreaLayout {
        let bar = self.scrollbar_extent.0.max(0.0);
        let mut vertical = content_size.height.0 > bounds.size.height.0;
        let mut horizontal = content_size.width.0 > bounds.size.width.0;
        let mut viewport_size = bounds.size;
        if vertical {
            viewport_size.width.0 = (viewport_size.width.0 - bar).max(0.0);
        }
        if horizontal {
            viewport_size.height.0 = (viewport_size.height.0 - bar).max(0.0);
        }
        // A reserved scrollbar can induce overflow on the perpendicular axis.
        vertical |= content_size.height.0 > viewport_size.height.0;
        horizontal |= content_size.width.0 > viewport_size.width.0;
        viewport_size.width.0 = (bounds.size.width.0 - if vertical { bar } else { 0.0 }).max(0.0);
        viewport_size.height.0 =
            (bounds.size.height.0 - if horizontal { bar } else { 0.0 }).max(0.0);
        self.state.set_metrics(viewport_size, content_size);
        let viewport = LogicalRect::new(bounds.origin, viewport_size);
        let content = LogicalRect::from_xywh(
            viewport.left() - self.state.offset.x.0,
            viewport.top() - self.state.offset.y.0,
            content_size.width.0,
            content_size.height.0,
        );
        let vertical_scrollbar = vertical.then(|| {
            scrollbar_thumb(
                LogicalRect::from_xywh(viewport.right(), bounds.top(), bar, viewport.size.height.0),
                ScrollAxis::Vertical,
                LogicalPx(viewport.size.height.0),
                LogicalPx(content_size.height.0),
                LogicalPx(self.state.offset.y.0),
                self.minimum_thumb,
            )
        });
        let horizontal_scrollbar = horizontal.then(|| {
            scrollbar_thumb(
                LogicalRect::from_xywh(
                    bounds.left(),
                    viewport.bottom(),
                    viewport.size.width.0,
                    bar,
                ),
                ScrollAxis::Horizontal,
                LogicalPx(viewport.size.width.0),
                LogicalPx(content_size.width.0),
                LogicalPx(self.state.offset.x.0),
                self.minimum_thumb,
            )
        });
        ScrollAreaLayout {
            viewport,
            content,
            clip: viewport,
            vertical_scrollbar,
            horizontal_scrollbar,
        }
    }

    pub fn scroll_by(&mut self, delta: LogicalPoint) -> bool {
        self.state.scroll_by(delta)
    }
}

/// Thumb bounds in the same logical coordinate space as its track.
pub fn scrollbar_thumb(
    track: LogicalRect,
    axis: ScrollAxis,
    viewport_extent: LogicalPx,
    content_extent: LogicalPx,
    offset: LogicalPx,
    minimum_thumb: LogicalPx,
) -> LogicalRect {
    let (track_start, track_extent) = match axis {
        ScrollAxis::Horizontal => (track.left(), track.size.width.0),
        ScrollAxis::Vertical => (track.top(), track.size.height.0),
    };
    if track_extent <= 0.0 || content_extent.0 <= 0.0 {
        return match axis {
            ScrollAxis::Horizontal => {
                LogicalRect::from_xywh(track_start, track.top(), 0.0, track.size.height.0)
            }
            ScrollAxis::Vertical => {
                LogicalRect::from_xywh(track.left(), track_start, track.size.width.0, 0.0)
            }
        };
    }
    let thumb_extent = (track_extent * (viewport_extent.0 / content_extent.0))
        .max(minimum_thumb.0)
        .min(track_extent);
    let scrollable_content = (content_extent.0 - viewport_extent.0).max(0.0);
    let scrollable_track = (track_extent - thumb_extent).max(0.0);
    let thumb_start = track_start
        + if scrollable_content > 0.0 {
            offset.0.clamp(0.0, scrollable_content) / scrollable_content * scrollable_track
        } else {
            0.0
        };
    match axis {
        ScrollAxis::Horizontal => {
            LogicalRect::from_xywh(thumb_start, track.top(), thumb_extent, track.size.height.0)
        }
        ScrollAxis::Vertical => {
            LogicalRect::from_xywh(track.left(), thumb_start, track.size.width.0, thumb_extent)
        }
    }
}

fn non_negative_size(size: LogicalSize) -> LogicalSize {
    LogicalSize::new(non_negative(size.width.0), non_negative(size.height.0))
}

fn non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

/// NaN becomes 0; infinities are kept so `clamp` saturates them at an end
/// instead of snapping a "scroll to the end" request back to the start.
fn finite(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_offsets_clamp_and_ensure_visible() {
        let mut scroll = ScrollState::default();
        scroll.set_metrics(
            LogicalSize::new(100.0, 60.0),
            LogicalSize::new(500.0, 400.0),
        );
        scroll.set_offset(LogicalPoint::new(900.0, -20.0));
        assert_eq!(scroll.offset, LogicalPoint::new(400.0, 0.0));
        scroll.ensure_visible(LogicalRect::from_xywh(20.0, 100.0, 30.0, 20.0));
        assert_eq!(scroll.offset, LogicalPoint::new(20.0, 60.0));
        scroll.scroll_by(LogicalPoint::new(0.0, 1000.0));
        assert_eq!(scroll.offset.y.0, 340.0);
    }

    #[test]
    fn scrollbar_thumb_tracks_viewport_and_offset() {
        let track = LogicalRect::from_xywh(0.0, 0.0, 10.0, 100.0);
        let at_top = scrollbar_thumb(
            track,
            ScrollAxis::Vertical,
            LogicalPx(50.0),
            LogicalPx(200.0),
            LogicalPx(0.0),
            LogicalPx(8.0),
        );
        let at_bottom = scrollbar_thumb(
            track,
            ScrollAxis::Vertical,
            LogicalPx(50.0),
            LogicalPx(200.0),
            LogicalPx(150.0),
            LogicalPx(8.0),
        );
        assert_eq!(at_top, LogicalRect::from_xywh(0.0, 0.0, 10.0, 25.0));
        assert_eq!(at_bottom, LogicalRect::from_xywh(0.0, 75.0, 10.0, 25.0));
    }

    #[test]
    fn scroll_area_returns_shared_viewport_content_and_nested_clip_geometry() {
        let mut area = ScrollArea::default();
        let layout = area.layout(
            LogicalRect::from_xywh(10.0, 20.0, 200.0, 100.0),
            LogicalSize::new(400.0, 300.0),
        );
        assert_eq!(
            layout.viewport,
            LogicalRect::from_xywh(10.0, 20.0, 190.0, 90.0)
        );
        assert_eq!(layout.clip, layout.viewport);
        assert!(layout.vertical_scrollbar.is_some());
        assert!(layout.horizontal_scrollbar.is_some());
        area.scroll_by(LogicalPoint::new(15.0, 25.0));
        let scrolled = area.layout(
            LogicalRect::from_xywh(10.0, 20.0, 200.0, 100.0),
            LogicalSize::new(400.0, 300.0),
        );
        assert_eq!(scrolled.content.origin, LogicalPoint::new(-5.0, -5.0));
    }

    #[test]
    fn wheel_deltas_saturate_clamp_and_ignore_nan() {
        let mut scroll = ScrollState::default();
        scroll.set_metrics(
            LogicalSize::new(100.0, 100.0),
            LogicalSize::new(100.0, 1000.0),
        );
        assert!(scroll.scroll_by(LogicalPoint::new(0.0, 1e300)));
        assert_eq!(scroll.offset.y.0, 900.0, "huge delta clamps to the end");
        assert!(
            !scroll.scroll_by(LogicalPoint::new(0.0, 48.0)),
            "already at the end"
        );
        assert!(scroll.scroll_by(LogicalPoint::new(0.0, f64::NEG_INFINITY)));
        assert_eq!(scroll.offset.y.0, 0.0);
        assert!(scroll.scroll_by(LogicalPoint::new(0.0, f64::INFINITY)));
        assert_eq!(
            scroll.offset.y.0, 900.0,
            "+inf means the end, not the start"
        );
        assert!(!scroll.scroll_by(LogicalPoint::new(f64::NAN, f64::NAN)));
        assert_eq!(scroll.offset.y.0, 900.0, "NaN must not move the view");
        assert!(scroll.scroll_by(LogicalPoint::new(0.0, -200.5)));
        assert_eq!(scroll.offset.y.0, 699.5);
        assert!(
            !scroll.scroll_by(LogicalPoint::new(30.0, 0.0)),
            "no horizontal overflow"
        );
    }

    #[test]
    fn content_smaller_than_viewport_never_scrolls_and_resize_reclamps() {
        let mut area = ScrollArea::default();
        let bounds = LogicalRect::from_xywh(0.0, 0.0, 200.0, 100.0);
        let small = area.layout(bounds, LogicalSize::new(150.0, 60.0));
        assert!(small.vertical_scrollbar.is_none() && small.horizontal_scrollbar.is_none());
        assert_eq!(small.viewport, bounds);
        assert!(!area.scroll_by(LogicalPoint::new(0.0, 500.0)));
        assert_eq!(area.state.offset, LogicalPoint::ZERO);

        let large = LogicalSize::new(150.0, 400.0);
        area.layout(bounds, large);
        area.scroll_by(LogicalPoint::new(0.0, 1e9));
        let at_end = area.state.offset.y.0;
        assert_eq!(at_end, 400.0 - 100.0);
        // Growing the window shrinks the scroll range: the offset follows.
        let taller = area.layout(LogicalRect::from_xywh(0.0, 0.0, 200.0, 250.0), large);
        assert_eq!(area.state.offset.y.0, 150.0);
        assert_eq!(taller.content.bottom(), taller.viewport.bottom());
        // Zero-size and negative bounds stay well-formed.
        let tiny = area.layout(LogicalRect::from_xywh(0.0, 0.0, 0.0, -5.0), large);
        assert!(tiny.viewport.size.width.0 >= 0.0 && tiny.viewport.size.height.0 >= 0.0);
        assert!((0.0..=400.0).contains(&area.state.offset.y.0));
    }

    #[test]
    fn nested_scroll_areas_paint_only_inside_both_viewports() {
        use crate::{
            Color, DpiScale, PaintBuilder, PhysicalSize, Renderer, Rgba8Renderer, TextSystem,
        };
        let mut outer = ScrollArea::default();
        let outer_layout = outer.layout(
            LogicalRect::from_xywh(10.0, 10.0, 60.0, 40.0),
            LogicalSize::new(200.0, 200.0),
        );
        // The inner area extends beyond the outer viewport on the right/bottom.
        let mut inner = ScrollArea::default();
        let inner_layout = inner.layout(
            LogicalRect::from_xywh(
                outer_layout.content.left() + 30.0,
                outer_layout.content.top() + 20.0,
                120.0,
                100.0,
            ),
            LogicalSize::new(300.0, 300.0),
        );
        let mut paint = PaintBuilder::new();
        paint.push_clip(outer_layout.clip);
        paint.push_clip(inner_layout.clip);
        paint.fill_rect(inner_layout.content, Color::rgba(255, 0, 0, 255), 0.0);
        paint.pop_clip().unwrap();
        paint.pop_clip().unwrap();
        let paint = paint.finish();
        assert_eq!(
            paint.commands()[0].clip,
            Some(outer_layout.clip.clip(inner_layout.clip))
        );
        let mut renderer = Rgba8Renderer::new();
        renderer
            .render(
                &paint,
                &mut TextSystem::with_cache_limits(1, 1024),
                DpiScale::ONE,
                PhysicalSize::new(100.0, 80.0),
            )
            .unwrap();
        let visible = outer_layout.clip.clip(inner_layout.clip);
        for y in 0..80 {
            for x in 0..100 {
                let painted = renderer.pixels()[(y * 100 + x) * 4 + 3] != 0;
                let expected = visible.contains(LogicalPoint::new(x as f64 + 0.5, y as f64 + 0.5));
                assert_eq!(painted, expected, "pixel ({x},{y})");
            }
        }
    }
}
