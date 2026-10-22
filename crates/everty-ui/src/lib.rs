//! Small, cross-platform UI foundation for Everty desktop products.
//!
//! This crate deliberately contains no session, protocol, filesystem, settings,
//! or product business logic. The initial foundation provides unit-aware
//! geometry, centralized logical/physical conversion, and shared visual tokens.
//! Platform integration and widgets are added in later, separately verified
//! layers.

#![forbid(unsafe_code)]

pub mod dpi;
pub mod geometry;
mod highlight;
pub mod input;
pub mod layout;
pub mod platform;
pub mod popup;
pub mod renderer;
pub mod scroll;
pub mod split_pane;
pub mod text;
pub mod text_field;
pub mod theme;
pub mod tooltip;
pub mod virtual_list;
pub mod widgets;
pub mod window;

pub use dpi::{DpiScale, DpiScaleError};
pub use geometry::{
    Constraints, Insets, LogicalPoint, LogicalPx, LogicalRect, LogicalSize, PhysicalPoint,
    PhysicalPx, PhysicalRect, PhysicalSize,
};
pub use highlight::ItemHighlights;
pub use input::{
    resolve_cursor, ClickTracker, FocusManager, HitRegion, ImeEvent, InputEvent, InputState, Key,
    Modifiers, MouseButton, RoutedEvent, WidgetId,
};
pub use layout::{
    layout, Alignment, FlexChild, LayoutBox, LayoutId, LayoutKind, LayoutNode, LayoutResult,
    MainAxis,
};
pub use popup::{
    anchored_placement, PopupEventResult, PopupId, PopupManager, PopupPointerResult, PopupState,
};
pub use renderer::{
    logical_viewport, raster_rect, BitmapIcon, IconId, LogicalPxValue, PaintBuilder, PaintCommand,
    PaintError, PaintLayer, PaintList, PaintOp, Renderer, Rgba8Renderer,
};
pub use scroll::{scrollbar_thumb, ScrollArea, ScrollAreaLayout, ScrollAxis, ScrollState};
pub use split_pane::{SplitPane, SplitPaneLayout};
pub use text::{
    FontFamily, FontUsage, FontWeight, GlyphCacheStats, GlyphImage, PositionedGlyph, TextLayout,
    TextMetrics, TextStyle, TextSystem,
};
pub use text_field::{caret_visible, Movement, TextEdit, TextField};
pub use theme::{Color, Theme, ThemeColors, ThemeMetrics};
pub use tooltip::{TooltipManager, TooltipPlacement, TooltipTarget};
pub use virtual_list::VirtualList;
pub use widgets::{
    Button, ButtonState, ButtonVariant, HorizontalTextAlign, IconButton, Label, WidgetEvent,
};
pub use window::{
    CursorIcon, UiClock, UiWindow, WindowId, WindowMetricsChange, WindowRegistry, WindowState,
};

/// Current crate version, useful to Gallery diagnostics without importing
/// package metadata at runtime.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Keep the shared toolkit independent of product code and OS-specific UI
/// branches. Native implementations, when needed, belong under `src/platform`.
#[cfg(test)]
mod boundary_tests {
    use std::path::{Path, PathBuf};

    fn rust_sources(directory: &Path, result: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let file_type = entry.file_type().unwrap();
            if file_type.is_dir() {
                rust_sources(&path, result);
            } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
                result.push(path);
            }
        }
    }

    #[test]
    fn shared_source_has_no_product_dependencies_or_platform_conditionals() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(!manifest.contains("evertydesk-core"));
        assert!(!manifest.contains("evertydesk-desktop-next"));
        // The toolkit stays independent of the retained GUI stack, the GPU
        // backend and the media/transport layers.
        for forbidden in [
            ["ic", "ed"].concat(),
            ["wg", "pu"].concat(),
            ["ev", "rt"].concat(),
            ["vali", "ure"].concat(),
            ["hb", "bu"].concat(),
        ] {
            assert!(
                !manifest.to_ascii_lowercase().contains(&forbidden),
                "everty-ui manifest must not mention {forbidden}"
            );
        }

        let source_root = root.join("src");
        let mut files = Vec::new();
        rust_sources(&source_root, &mut files);
        let os_marker = ["target", "_os"].concat();
        let family_marker = ["target", "_family"].concat();
        let core_crate = ["evertydesk", "_core"].concat();
        let desktop_crate = ["evertydesk", "_desktop_next"].concat();
        let transport_markers = [
            ["ev", "rt"].concat(),
            ["vali", "ure"].concat(),
            ["hb", "bu"].concat(),
        ];
        for file in files {
            let relative = file.strip_prefix(&source_root).unwrap();
            if relative
                .components()
                .next()
                .is_some_and(|component| component.as_os_str() == "platform")
            {
                continue;
            }

            let source = std::fs::read_to_string(&file).unwrap();
            assert!(
                !source.contains(&os_marker) && !source.contains(&family_marker),
                "platform conditional leaked into shared UI module: {}",
                relative.display()
            );
            assert!(
                !source.contains(&core_crate) && !source.contains(&desktop_crate),
                "product dependency leaked into shared UI module: {}",
                relative.display()
            );
            let lowered = source.to_ascii_lowercase();
            for marker in &transport_markers {
                assert!(
                    !lowered.contains(marker.as_str()),
                    "transport/codec dependency '{marker}' leaked into: {}",
                    relative.display()
                );
            }
        }
    }
}
