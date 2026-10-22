//! Shared geometry expressed in explicit logical or physical pixel units.

use crate::DpiScale;

/// A coordinate or extent in DPI-independent layout pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct LogicalPx(pub f64);

/// A coordinate or extent in device pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct PhysicalPx(pub f64);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LogicalPoint {
    pub x: LogicalPx,
    pub y: LogicalPx,
}

impl LogicalPoint {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f64, y: f64) -> Self {
        Self {
            x: LogicalPx(x),
            y: LogicalPx(y),
        }
    }

    pub fn to_physical(self, scale: DpiScale) -> PhysicalPoint {
        PhysicalPoint::new(scale.to_physical(self.x).0, scale.to_physical(self.y).0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhysicalPoint {
    pub x: PhysicalPx,
    pub y: PhysicalPx,
}

impl PhysicalPoint {
    pub const fn new(x: f64, y: f64) -> Self {
        Self {
            x: PhysicalPx(x),
            y: PhysicalPx(y),
        }
    }

    pub fn to_logical(self, scale: DpiScale) -> LogicalPoint {
        LogicalPoint::new(scale.to_logical(self.x).0, scale.to_logical(self.y).0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LogicalSize {
    pub width: LogicalPx,
    pub height: LogicalPx,
}

impl LogicalSize {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(width: f64, height: f64) -> Self {
        Self {
            width: LogicalPx(width),
            height: LogicalPx(height),
        }
    }

    pub fn is_empty(self) -> bool {
        self.width.0 <= 0.0 || self.height.0 <= 0.0
    }

    pub fn to_physical(self, scale: DpiScale) -> PhysicalSize {
        PhysicalSize::new(
            scale.to_physical(self.width).0,
            scale.to_physical(self.height).0,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhysicalSize {
    pub width: PhysicalPx,
    pub height: PhysicalPx,
}

impl PhysicalSize {
    pub const fn new(width: f64, height: f64) -> Self {
        Self {
            width: PhysicalPx(width),
            height: PhysicalPx(height),
        }
    }

    pub fn to_logical(self, scale: DpiScale) -> LogicalSize {
        LogicalSize::new(
            scale.to_logical(self.width).0,
            scale.to_logical(self.height).0,
        )
    }
}

/// Insets are logical distances from each edge.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: LogicalPx,
    pub top: LogicalPx,
    pub right: LogicalPx,
    pub bottom: LogicalPx,
}

impl Insets {
    pub const ZERO: Self = Self::all(0.0);

    pub const fn all(value: f64) -> Self {
        Self {
            left: LogicalPx(value),
            top: LogicalPx(value),
            right: LogicalPx(value),
            bottom: LogicalPx(value),
        }
    }

    pub const fn symmetric(horizontal: f64, vertical: f64) -> Self {
        Self {
            left: LogicalPx(horizontal),
            top: LogicalPx(vertical),
            right: LogicalPx(horizontal),
            bottom: LogicalPx(vertical),
        }
    }
}

/// Half-open logical rectangle: left/top included, right/bottom excluded.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LogicalRect {
    pub origin: LogicalPoint,
    pub size: LogicalSize,
}

impl LogicalRect {
    pub const ZERO: Self = Self::from_xywh(0.0, 0.0, 0.0, 0.0);

    pub const fn new(origin: LogicalPoint, size: LogicalSize) -> Self {
        Self { origin, size }
    }

    pub const fn from_xywh(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self::new(LogicalPoint::new(x, y), LogicalSize::new(width, height))
    }

    pub fn left(self) -> f64 {
        self.origin.x.0
    }
    pub fn top(self) -> f64 {
        self.origin.y.0
    }
    pub fn right(self) -> f64 {
        self.left() + self.size.width.0
    }
    pub fn bottom(self) -> f64 {
        self.top() + self.size.height.0
    }

    pub fn contains(self, point: LogicalPoint) -> bool {
        point.x.0 >= self.left()
            && point.y.0 >= self.top()
            && point.x.0 < self.right()
            && point.y.0 < self.bottom()
    }

    pub fn intersects(self, other: Self) -> bool {
        self.left() < other.right()
            && other.left() < self.right()
            && self.top() < other.bottom()
            && other.top() < self.bottom()
    }

    pub fn inset(self, insets: Insets) -> Self {
        // Non-finite insets are treated as zero rather than poisoning the
        // geometry with NaN.
        let finite = |value: f64| if value.is_finite() { value } else { 0.0 };
        let left = self.left() + finite(insets.left.0);
        let top = self.top() + finite(insets.top.0);
        let right = (self.right() - finite(insets.right.0)).max(left);
        let bottom = (self.bottom() - finite(insets.bottom.0)).max(top);
        Self::from_xywh(left, top, right - left, bottom - top)
    }

    pub fn expand(self, insets: Insets) -> Self {
        let left = self.left() - insets.left.0;
        let top = self.top() - insets.top.0;
        let right = self.right() + insets.right.0;
        let bottom = self.bottom() + insets.bottom.0;
        Self::from_xywh(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
    }

    pub fn translate(self, delta: LogicalPoint) -> Self {
        Self::new(
            LogicalPoint::new(self.left() + delta.x.0, self.top() + delta.y.0),
            self.size,
        )
    }

    pub fn clip(self, bounds: Self) -> Self {
        let left = self.left().max(bounds.left());
        let top = self.top().max(bounds.top());
        let right = self.right().min(bounds.right()).max(left);
        let bottom = self.bottom().min(bounds.bottom()).max(top);
        Self::from_xywh(left, top, right - left, bottom - top)
    }

    pub fn to_physical(self, scale: DpiScale) -> PhysicalRect {
        PhysicalRect::from_xywh(
            scale.to_physical(self.origin.x).0,
            scale.to_physical(self.origin.y).0,
            scale.to_physical(self.size.width).0,
            scale.to_physical(self.size.height).0,
        )
    }
}

/// Half-open rectangle in physical device pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhysicalRect {
    pub origin: PhysicalPoint,
    pub size: PhysicalSize,
}

impl PhysicalRect {
    pub const fn from_xywh(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: PhysicalPoint::new(x, y),
            size: PhysicalSize::new(width, height),
        }
    }

    pub fn to_logical(self, scale: DpiScale) -> LogicalRect {
        LogicalRect::from_xywh(
            scale.to_logical(self.origin.x).0,
            scale.to_logical(self.origin.y).0,
            scale.to_logical(self.size.width).0,
            scale.to_logical(self.size.height).0,
        )
    }
}

/// Min/max layout constraints. Max is inclusive; `None` means unbounded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Constraints {
    pub min: LogicalSize,
    pub max: Option<LogicalSize>,
}

impl Constraints {
    pub const UNBOUNDED: Self = Self {
        min: LogicalSize::ZERO,
        max: None,
    };

    pub const fn new(min: LogicalSize, max: Option<LogicalSize>) -> Self {
        Self { min, max }
    }

    pub fn constrain(self, size: LogicalSize) -> LogicalSize {
        let max_width = self
            .max
            .map_or(f64::INFINITY, |max| max.width.0.max(self.min.width.0));
        let max_height = self
            .max
            .map_or(f64::INFINITY, |max| max.height.0.max(self.min.height.0));
        LogicalSize::new(
            size.width.0.max(self.min.width.0).min(max_width),
            size.height.0.max(self.min.height.0).min(max_height),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangles_use_half_open_bounds_and_intersection() {
        let rect = LogicalRect::from_xywh(10.0, 20.0, 30.0, 40.0);
        assert!(rect.contains(LogicalPoint::new(10.0, 20.0)));
        assert!(!rect.contains(LogicalPoint::new(40.0, 20.0)));
        assert!(rect.intersects(LogicalRect::from_xywh(39.0, 59.0, 2.0, 2.0)));
        assert!(!rect.intersects(LogicalRect::from_xywh(40.0, 20.0, 2.0, 2.0)));
    }

    #[test]
    fn inset_expand_translate_and_clip_are_deterministic() {
        let rect = LogicalRect::from_xywh(10.0, 20.0, 100.0, 80.0);
        assert_eq!(
            rect.inset(Insets::symmetric(4.0, 6.0)),
            LogicalRect::from_xywh(14.0, 26.0, 92.0, 68.0)
        );
        assert_eq!(
            rect.expand(Insets::all(2.0)),
            LogicalRect::from_xywh(8.0, 18.0, 104.0, 84.0)
        );
        assert_eq!(
            rect.translate(LogicalPoint::new(-5.0, 7.0)),
            LogicalRect::from_xywh(5.0, 27.0, 100.0, 80.0)
        );
        assert_eq!(
            rect.clip(LogicalRect::from_xywh(50.0, 50.0, 100.0, 100.0)),
            LogicalRect::from_xywh(50.0, 50.0, 60.0, 50.0)
        );
        assert_eq!(rect.inset(Insets::all(100.0)).size, LogicalSize::ZERO);
    }

    #[test]
    fn constraints_clamp_to_min_and_max() {
        let constraints = Constraints::new(
            LogicalSize::new(20.0, 10.0),
            Some(LogicalSize::new(80.0, 50.0)),
        );
        assert_eq!(
            constraints.constrain(LogicalSize::new(5.0, 100.0)),
            LogicalSize::new(20.0, 50.0)
        );
        assert_eq!(
            Constraints::UNBOUNDED.constrain(LogicalSize::new(25.0, 35.0)),
            LogicalSize::new(25.0, 35.0)
        );
    }
}
