//! Shared Everty Light design tokens; widgets consume tokens, not ad-hoc sizes.

use crate::LogicalPx;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self([red, green, blue, alpha])
    }

    /// Linear blend of the RGB channels towards `other` by `amount` (0..=1);
    /// alpha is kept. Used to derive hover/pressed shades from a base token.
    pub fn mix(self, other: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let blend = |from: u8, to: u8| {
            (f32::from(from) + (f32::from(to) - f32::from(from)) * amount)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Self([
            blend(self.0[0], other.0[0]),
            blend(self.0[1], other.0[1]),
            blend(self.0[2], other.0[2]),
            self.0[3],
        ])
    }

    /// Same colour with its alpha scaled by `factor` (0..=1).
    pub fn with_opacity(self, factor: f32) -> Self {
        let mut color = self;
        color.0[3] = (f32::from(self.0[3]) * factor.clamp(0.0, 1.0)).round() as u8;
        color
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeMetrics {
    pub font_xs: LogicalPx,
    pub font_sm: LogicalPx,
    pub font_md: LogicalPx,
    pub font_lg: LogicalPx,
    pub font_title: LogicalPx,
    pub line_height_sm: LogicalPx,
    pub line_height_md: LogicalPx,
    pub line_height_lg: LogicalPx,
    pub space_xs: LogicalPx,
    pub space_sm: LogicalPx,
    pub space_md: LogicalPx,
    pub space_lg: LogicalPx,
    pub control_height_sm: LogicalPx,
    pub control_height_md: LogicalPx,
    pub border_width: LogicalPx,
    pub radius_sm: LogicalPx,
    pub radius_md: LogicalPx,
    /// Cards and windows' inner panels.
    pub radius_lg: LogicalPx,
    pub focus_width: LogicalPx,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeColors {
    pub window: Color,
    pub surface: Color,
    pub surface_hover: Color,
    pub surface_pressed: Color,
    pub border: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_disabled: Color,
    pub accent: Color,
    pub selection: Color,
    pub focus: Color,
    /// Status colours for notifications, indicators and destructive actions.
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    /// Dims whatever is behind a modal surface.
    pub scrim: Color,
    /// Foreground on `accent`/`danger` fills.
    pub text_on_accent: Color,
}

/// Theme values are immutable and owned by the application/window tree.
/// Adding a future dark theme changes a token set, not widget code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub metrics: ThemeMetrics,
    pub colors: ThemeColors,
    pub disabled_opacity: f32,
}

impl Theme {
    pub const EVERTY_LIGHT: Self = Self {
        metrics: ThemeMetrics {
            font_xs: LogicalPx(12.0),
            font_sm: LogicalPx(13.0),
            font_md: LogicalPx(14.0),
            font_lg: LogicalPx(16.0),
            font_title: LogicalPx(24.0),
            line_height_sm: LogicalPx(18.0),
            line_height_md: LogicalPx(20.0),
            line_height_lg: LogicalPx(24.0),
            space_xs: LogicalPx(4.0),
            space_sm: LogicalPx(8.0),
            space_md: LogicalPx(12.0),
            space_lg: LogicalPx(20.0),
            control_height_sm: LogicalPx(28.0),
            control_height_md: LogicalPx(36.0),
            border_width: LogicalPx(1.0),
            radius_sm: LogicalPx(8.0),
            radius_md: LogicalPx(10.0),
            radius_lg: LogicalPx(14.0),
            focus_width: LogicalPx(2.0),
        },
        colors: ThemeColors {
            window: Color::rgba(246, 247, 249, 255),
            surface: Color::rgba(255, 255, 255, 255),
            surface_hover: Color::rgba(237, 241, 247, 255),
            surface_pressed: Color::rgba(224, 231, 241, 255),
            border: Color::rgba(205, 212, 222, 255),
            text_primary: Color::rgba(27, 35, 47, 255),
            text_secondary: Color::rgba(85, 96, 112, 255),
            text_disabled: Color::rgba(143, 151, 162, 255),
            accent: Color::rgba(40, 103, 190, 255),
            selection: Color::rgba(216, 232, 252, 255),
            focus: Color::rgba(57, 127, 219, 255),
            success: Color::rgba(33, 140, 84, 255),
            warning: Color::rgba(184, 120, 12, 255),
            danger: Color::rgba(196, 44, 56, 255),
            scrim: Color::rgba(18, 26, 38, 140),
            text_on_accent: Color::rgba(255, 255, 255, 255),
        },
        disabled_opacity: 0.55,
    };
}

impl Default for Theme {
    fn default() -> Self {
        Self::EVERTY_LIGHT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everty_light_has_positive_and_consistent_metrics() {
        let theme = std::hint::black_box(Theme::EVERTY_LIGHT);
        let metrics = theme.metrics;
        assert!(metrics.font_xs.0 > 0.0 && metrics.font_xs.0 < metrics.font_md.0);
        assert!(metrics.font_md.0 < metrics.font_lg.0 && metrics.font_lg.0 < metrics.font_title.0);
        assert!(metrics.line_height_md.0 >= metrics.font_md.0);
        assert!(metrics.control_height_md.0 >= metrics.line_height_md.0);
        assert!(theme.disabled_opacity > 0.0 && theme.disabled_opacity < 1.0);
    }

    #[test]
    fn theme_has_distinct_interaction_and_focus_tokens() {
        let colors = Theme::EVERTY_LIGHT.colors;
        assert_ne!(colors.surface, colors.surface_hover);
        assert_ne!(colors.surface_hover, colors.surface_pressed);
        assert_ne!(colors.focus, colors.border);
    }

    #[test]
    fn mix_and_opacity_keep_channels_in_range_and_alpha_semantics() {
        let a = Color::rgba(200, 100, 0, 255);
        let black = Color::rgba(0, 0, 0, 255);
        assert_eq!(a.mix(black, 0.0), a);
        assert_eq!(a.mix(black, 1.0), black);
        assert_eq!(a.mix(black, 0.5), Color::rgba(100, 50, 0, 255));
        assert_eq!(a.mix(black, 9.0), black, "amount is clamped");
        assert_eq!(a.with_opacity(0.5).0[3], 128);
        assert_eq!(a.with_opacity(-1.0).0[3], 0);
        assert_eq!(a.with_opacity(2.0), a);
    }

    #[test]
    fn status_tokens_are_distinct_and_readable_on_the_surface() {
        let colors = Theme::EVERTY_LIGHT.colors;
        let status = [colors.success, colors.warning, colors.danger, colors.accent];
        for (index, a) in status.iter().enumerate() {
            for b in &status[index + 1..] {
                assert_ne!(a, b);
            }
        }
        // Status colours are used for text and thin strokes on `surface`:
        // require WCAG-style contrast >= 3:1 (large text / UI components).
        let luminance = |color: Color| {
            let channel = |value: u8| {
                let value = f64::from(value) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(color.0[0])
                + 0.7152 * channel(color.0[1])
                + 0.0722 * channel(color.0[2])
        };
        let surface = luminance(colors.surface);
        for color in status {
            let ratio = (surface + 0.05) / (luminance(color) + 0.05);
            assert!(ratio >= 3.0, "{color:?} has contrast {ratio:.2} on surface");
        }
        assert!(colors.scrim.0[3] < 255, "scrim must stay translucent");
    }
}
