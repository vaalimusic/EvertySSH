//! Platform-neutral delayed tooltip state and placement.

use std::time::{Duration, Instant};

use crate::popup::anchored_placement;
use crate::{LogicalRect, LogicalSize, Theme, WidgetId};

#[derive(Clone, Debug, PartialEq)]
pub struct TooltipTarget {
    pub owner: WidgetId,
    pub anchor: LogicalRect,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TooltipPlacement {
    pub owner: WidgetId,
    pub text: String,
    pub bounds: LogicalRect,
}

/// Shared hover timer and window-clamped tooltip placement. Text shaping and
/// painting remain the caller's responsibility so this state stays renderer- and
/// platform-independent.
#[derive(Clone, Debug)]
pub struct TooltipManager {
    delay: Duration,
    target: Option<TooltipTarget>,
    hovered_since: Option<Instant>,
    shown: bool,
    /// The delay elapsed but there was nothing to show (empty text, empty
    /// window). Counts as handled so the host does not keep requesting
    /// redraws for a tooltip that can never appear.
    suppressed: bool,
}

impl Default for TooltipManager {
    fn default() -> Self {
        Self::new(Duration::from_millis(500))
    }
}

impl TooltipManager {
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            target: None,
            hovered_since: None,
            shown: false,
            suppressed: false,
        }
    }

    /// Set the current topmost tooltip candidate. Moving the same target keeps
    /// its hover timer, while changing owner/text starts a new delay.
    pub fn set_target(&mut self, now: Instant, target: Option<TooltipTarget>) {
        let same_target =
            self.target
                .as_ref()
                .zip(target.as_ref())
                .is_some_and(|(current, next)| {
                    current.owner == next.owner && current.text == next.text
                });
        if !same_target {
            self.hovered_since = target.as_ref().map(|_| now);
            self.shown = false;
            self.suppressed = false;
        }
        self.target = target;
    }

    pub fn target(&self) -> Option<&TooltipTarget> {
        self.target.as_ref()
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        if self.shown || self.suppressed {
            return None;
        }
        self.hovered_since
            .and_then(|since| since.checked_add(self.delay))
            .filter(|deadline| *deadline > now)
    }

    pub fn is_due(&self, now: Instant) -> bool {
        !self.shown
            && !self.suppressed
            && self
                .hovered_since
                .is_some_and(|since| now.saturating_duration_since(since) >= self.delay)
            && self.target.is_some()
    }

    pub fn placement(
        &mut self,
        now: Instant,
        window: LogicalRect,
        measured_text: LogicalSize,
        theme: Theme,
    ) -> Option<TooltipPlacement> {
        let target = self.target.as_ref()?;
        if self
            .hovered_since
            .is_none_or(|since| now.saturating_duration_since(since) < self.delay)
        {
            return None;
        }
        if target.text.is_empty() || window.size.is_empty() {
            self.suppressed = true;
            return None;
        }

        let padding = theme.metrics.space_sm.0;
        let gap = theme.metrics.space_xs.0;
        let bounds = anchored_placement(
            target.anchor,
            LogicalSize::new(
                measured_text.width.0 + padding * 2.0,
                measured_text.height.0 + padding * 2.0,
            ),
            window,
            gap,
        );
        self.shown = true;
        Some(TooltipPlacement {
            owner: target.owner,
            text: target.text.clone(),
            bounds,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(owner: u64, x: f64, y: f64) -> TooltipTarget {
        TooltipTarget {
            owner: WidgetId(owner),
            anchor: LogicalRect::from_xywh(x, y, 24.0, 24.0),
            text: "Tooltip text".to_owned(),
        }
    }

    #[test]
    fn tooltip_waits_for_hover_delay_and_resets_for_new_target() {
        let now = Instant::now();
        let mut tooltip = TooltipManager::default();
        let window = LogicalRect::from_xywh(0.0, 0.0, 320.0, 200.0);
        let measured = LogicalSize::new(80.0, 18.0);
        tooltip.set_target(now, Some(target(1, 20.0, 20.0)));
        assert_eq!(
            tooltip.next_deadline(now),
            Some(now + Duration::from_millis(500))
        );
        assert!(!tooltip.is_due(now + Duration::from_millis(499)));
        assert!(tooltip
            .placement(
                now + Duration::from_millis(499),
                window,
                measured,
                Theme::default()
            )
            .is_none());
        assert!(tooltip.is_due(now + Duration::from_millis(500)));
        assert!(tooltip
            .next_deadline(now + Duration::from_millis(500))
            .is_none());
        assert!(tooltip
            .placement(
                now + Duration::from_millis(500),
                window,
                measured,
                Theme::default()
            )
            .is_some());

        tooltip.set_target(now + Duration::from_secs(1), Some(target(2, 20.0, 20.0)));
        assert!(tooltip
            .placement(
                now + Duration::from_millis(1499),
                window,
                measured,
                Theme::default()
            )
            .is_none());
        assert!(tooltip
            .placement(
                now + Duration::from_millis(1500),
                window,
                measured,
                Theme::default()
            )
            .is_some());
        tooltip.set_target(now, None);
        assert!(tooltip.next_deadline(now).is_none());
    }

    #[test]
    fn tooltip_flips_above_and_clamps_inside_window() {
        let now = Instant::now();
        let mut tooltip = TooltipManager::default();
        let window = LogicalRect::from_xywh(10.0, 20.0, 200.0, 120.0);
        let measured = LogicalSize::new(100.0, 22.0);
        tooltip.set_target(now, Some(target(7, 185.0, 112.0)));
        let placed = tooltip
            .placement(
                now + Duration::from_millis(500),
                window,
                measured,
                Theme::default(),
            )
            .unwrap();
        assert!(placed.bounds.left() >= window.left());
        assert!(placed.bounds.right() <= window.right());
        assert!(placed.bounds.top() >= window.top());
        assert!(placed.bounds.bottom() <= window.bottom());
        assert!(placed.bounds.bottom() < 112.0);
        assert_eq!(placed.owner, WidgetId(7));
    }

    #[test]
    fn a_tooltip_that_cannot_be_shown_stops_requesting_redraws() {
        let now = Instant::now();
        let due = now + Duration::from_millis(600);
        let window = LogicalRect::from_xywh(0.0, 0.0, 320.0, 200.0);
        let measured = LogicalSize::new(80.0, 18.0);
        // Empty text: the host loop must not spin on `is_due`.
        let mut tooltip = TooltipManager::default();
        let mut empty = target(1, 20.0, 20.0);
        empty.text.clear();
        tooltip.set_target(now, Some(empty));
        assert!(tooltip.is_due(due));
        assert!(tooltip
            .placement(due, window, measured, Theme::default())
            .is_none());
        assert!(!tooltip.is_due(due), "handled: no further redraw requests");
        assert!(tooltip.next_deadline(due).is_none());
        // Zero-size window (minimized): same, and a new target starts afresh.
        let mut tooltip = TooltipManager::default();
        tooltip.set_target(now, Some(target(1, 20.0, 20.0)));
        assert!(tooltip
            .placement(due, LogicalRect::ZERO, measured, Theme::default())
            .is_none());
        assert!(!tooltip.is_due(due));
        tooltip.set_target(due, Some(target(2, 20.0, 20.0)));
        assert!(tooltip.is_due(due + Duration::from_millis(500)));
    }
}
