//! The sole logical/physical coordinate conversion boundary.

use crate::geometry::{LogicalPx, LogicalRect, PhysicalPx, PhysicalRect};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DpiScale(f64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DpiScaleError {
    NotFinite,
    NotPositive,
}

impl DpiScale {
    pub const ONE: Self = Self(1.0);

    pub fn new(scale: f64) -> Result<Self, DpiScaleError> {
        if !scale.is_finite() {
            return Err(DpiScaleError::NotFinite);
        }
        if scale <= 0.0 {
            return Err(DpiScaleError::NotPositive);
        }
        Ok(Self(scale))
    }

    pub const fn get(self) -> f64 {
        self.0
    }

    pub fn to_physical(self, value: LogicalPx) -> PhysicalPx {
        PhysicalPx(value.0 * self.0)
    }

    pub fn to_logical(self, value: PhysicalPx) -> LogicalPx {
        LogicalPx(value.0 / self.0)
    }

    /// Snap a logical coordinate to the nearest device-pixel boundary, then
    /// return it in logical units for layout consumers.
    pub fn snap_logical(self, value: LogicalPx) -> LogicalPx {
        LogicalPx((value.0 * self.0).round() / self.0)
    }

    pub fn snap_physical(self, value: PhysicalPx) -> PhysicalPx {
        let _ = self;
        PhysicalPx(value.0.round())
    }

    /// Snap each edge independently so the physical rectangle has crisp,
    /// integer-aligned boundaries without accumulating width rounding error.
    pub fn snap_rect(self, rect: LogicalRect) -> LogicalRect {
        let left = self.snap_logical(rect.origin.x).0;
        let top = self.snap_logical(rect.origin.y).0;
        let right = self
            .snap_logical(LogicalPx(rect.origin.x.0 + rect.size.width.0))
            .0;
        let bottom = self
            .snap_logical(LogicalPx(rect.origin.y.0 + rect.size.height.0))
            .0;
        LogicalRect::from_xywh(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
    }

    pub fn snap_physical_rect(self, rect: PhysicalRect) -> PhysicalRect {
        let _ = self;
        let left = rect.origin.x.0.round();
        let top = rect.origin.y.0.round();
        let right = (rect.origin.x.0 + rect.size.width.0).round();
        let bottom = (rect.origin.y.0 + rect.size.height.0).round();
        PhysicalRect::from_xywh(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
    }
}

impl Default for DpiScale {
    fn default() -> Self {
        Self::ONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{LogicalPoint, LogicalSize, PhysicalPx};

    #[test]
    fn rejects_invalid_scale_factors() {
        assert_eq!(DpiScale::new(0.0), Err(DpiScaleError::NotPositive));
        assert_eq!(DpiScale::new(-1.0), Err(DpiScaleError::NotPositive));
        assert_eq!(DpiScale::new(f64::NAN), Err(DpiScaleError::NotFinite));
        assert_eq!(DpiScale::new(f64::INFINITY), Err(DpiScaleError::NotFinite));
    }

    #[test]
    fn roundtrips_required_dpi_factors_without_double_scaling() {
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            let dpi = DpiScale::new(scale).unwrap();
            let logical = LogicalPoint::new(128.0, 72.0);
            let physical = logical.to_physical(dpi);
            assert_eq!(physical.to_logical(dpi), logical);
            let size = LogicalSize::new(640.0, 480.0)
                .to_physical(dpi)
                .to_logical(dpi);
            assert_eq!(size, LogicalSize::new(640.0, 480.0));
        }
    }

    #[test]
    fn snapping_occurs_once_in_physical_space() {
        let dpi = DpiScale::new(1.5).unwrap();
        let snapped = dpi.snap_logical(LogicalPx(10.2));
        assert_eq!(dpi.to_physical(snapped), PhysicalPx(15.0));
        let rect = dpi.snap_rect(LogicalRect::from_xywh(0.2, 1.2, 10.4, 5.4));
        let physical = rect.to_physical(dpi);
        assert_eq!(physical, PhysicalRect::from_xywh(0.0, 2.0, 16.0, 8.0));
    }

    #[test]
    fn snapping_negative_coordinates_uses_nearest_device_pixel() {
        let dpi = DpiScale::new(1.25).unwrap();
        assert_eq!(
            dpi.to_physical(dpi.snap_logical(LogicalPx(-0.6))),
            PhysicalPx(-1.0)
        );
    }
}
