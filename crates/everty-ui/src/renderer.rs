//! Renderer-neutral paint contract. Backends batch these logical commands and
//! perform the one DPI conversion at rasterization time.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;

use crate::{Color, DpiScale, LogicalPoint, LogicalRect, LogicalSize, PhysicalSize, TextLayout};
use crate::{GlyphImage, TextSystem};

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum PaintLayer {
    #[default]
    Background,
    Content,
    Floating,
    Popup,
    Tooltip,
    Overlay,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IconId(pub u32);

#[derive(Clone, Debug)]
pub enum PaintOp {
    Image {
        rect: LogicalRect,
        width: u32,
        height: u32,
        rgba: Arc<[u8]>,
    },
    FillRect {
        rect: LogicalRect,
        color: Color,
        radius: LogicalPxValue,
    },
    StrokeRect {
        rect: LogicalRect,
        color: Color,
        width: LogicalPxValue,
        radius: LogicalPxValue,
    },
    Text {
        origin: LogicalPoint,
        layout: Arc<TextLayout>,
        color: Color,
    },
    Icon {
        id: IconId,
        rect: LogicalRect,
        tint: Color,
    },
}

/// A dimension in logical pixels, named separately to avoid backend ambiguity.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LogicalPxValue(pub f64);

#[derive(Clone, Debug)]
pub struct PaintCommand {
    pub layer: PaintLayer,
    /// Effective intersection of all active clips, if clipping is enabled.
    pub clip: Option<LogicalRect>,
    pub op: PaintOp,
}

#[derive(Clone, Debug, Default)]
pub struct PaintList {
    commands: Vec<PaintCommand>,
}

impl PaintList {
    pub fn commands(&self) -> &[PaintCommand] {
        &self.commands
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Stable layer ordering; insertion order is preserved inside each layer.
    pub fn sort_by_layer(&mut self) {
        self.commands.sort_by_key(|command| command.layer);
    }

    pub fn into_commands(self) -> Vec<PaintCommand> {
        self.commands
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaintError {
    ClipStackUnderflow,
}

#[derive(Default)]
pub struct PaintBuilder {
    commands: Vec<PaintCommand>,
    clips: Vec<LogicalRect>,
    layer: PaintLayer,
    base_clip_depth: usize,
}

impl PaintBuilder {
    /// Bounded, already decoded RGBA pixels. The renderer never performs image decoding.
    pub fn image(&mut self, rect: LogicalRect, width: u32, height: u32, rgba: Arc<[u8]>) -> bool {
        if width == 0
            || height == 0
            || width > 1024
            || height > 1024
            || rgba.len() != width as usize * height as usize * 4
        {
            return false;
        }
        self.push(PaintOp::Image {
            rect,
            width,
            height,
            rgba,
        });
        true
    }
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_viewport(viewport: LogicalRect) -> Self {
        Self {
            clips: vec![viewport],
            base_clip_depth: 1,
            ..Self::default()
        }
    }

    pub fn set_layer(&mut self, layer: PaintLayer) {
        self.layer = layer;
    }

    /// Paint-order stamp for the next command. Passing it to
    /// `HitRegion::paint_order` for a widget painted from this point keeps
    /// hit-testing and painting on one ordering.
    pub fn paint_order(&self) -> u64 {
        self.commands.len() as u64
    }

    pub fn push_clip(&mut self, rect: LogicalRect) {
        let effective = self.clips.last().map_or(rect, |parent| parent.clip(rect));
        self.clips.push(effective);
    }

    pub fn pop_clip(&mut self) -> Result<(), PaintError> {
        if self.clips.len() <= self.base_clip_depth {
            return Err(PaintError::ClipStackUnderflow);
        }
        self.clips
            .pop()
            .map(|_| ())
            .ok_or(PaintError::ClipStackUnderflow)
    }

    pub fn fill_rect(&mut self, rect: LogicalRect, color: Color, radius: f64) {
        self.push(PaintOp::FillRect {
            rect,
            color,
            radius: LogicalPxValue(non_negative(radius)),
        });
    }

    pub fn stroke_rect(&mut self, rect: LogicalRect, color: Color, width: f64, radius: f64) {
        self.push(PaintOp::StrokeRect {
            rect,
            color,
            width: LogicalPxValue(non_negative(width)),
            radius: LogicalPxValue(non_negative(radius)),
        });
    }

    pub fn text(&mut self, origin: LogicalPoint, layout: Arc<TextLayout>, color: Color) {
        self.push(PaintOp::Text {
            origin,
            layout,
            color,
        });
    }

    pub fn icon(&mut self, id: IconId, rect: LogicalRect, tint: Color) {
        self.push(PaintOp::Icon { id, rect, tint });
    }

    pub fn finish(self) -> PaintList {
        PaintList {
            commands: self.commands,
        }
    }

    fn push(&mut self, op: PaintOp) {
        self.commands.push(PaintCommand {
            layer: self.layer,
            clip: self.clips.last().copied(),
            op,
        });
    }
}

/// Backend boundary: apps submit the same paint model to the existing viewer
/// compositor or a future independent-window surface implementation.
pub trait Renderer {
    type Error;

    fn render(
        &mut self,
        paint: &PaintList,
        text_system: &mut TextSystem,
        scale: DpiScale,
        target_size: PhysicalSize,
    ) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug)]
pub struct BitmapIcon {
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA8 pixels in row-major order.
    pub rgba: Arc<[u8]>,
}

/// Raster capacity that is always kept; smaller targets never trigger a shrink.
const RETAINED_RASTER_BYTES: usize = 4 * 1024 * 1024;

/// CPU UI-layer rasterizer used by Gallery and the current Viewer overlay path.
/// The existing FrameRenderer remains responsible for GPU upload/composition.
pub struct Rgba8Renderer {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    icons: HashMap<IconId, BitmapIcon>,
    svg_icons: HashMap<IconId, resvg::usvg::Tree>,
    svg_cache: HashMap<(IconId, u32, u32), BitmapIcon>,
}

impl Default for Rgba8Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Rgba8Renderer {
    pub fn new() -> Self {
        Self {
            pixels: Vec::new(),
            width: 0,
            height: 0,
            icons: HashMap::new(),
            svg_icons: HashMap::new(),
            svg_cache: HashMap::new(),
        }
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn register_icon(&mut self, id: IconId, icon: BitmapIcon) -> Result<(), &'static str> {
        let expected = (icon.width as usize)
            .checked_mul(icon.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or("icon dimensions overflow")?;
        if icon.width == 0 || icon.height == 0 || icon.rgba.len() != expected {
            return Err("icon RGBA buffer dimensions do not match");
        }
        self.icons.insert(id, icon);
        self.svg_icons.remove(&id);
        self.svg_cache.retain(|(cached, _, _), _| *cached != id);
        Ok(())
    }

    /// Keep vector source; rasterize once per actual snapped physical size.
    /// Cache is bounded and shared by all users of this renderer, including DPI changes.
    pub fn register_svg_icon(&mut self, id: IconId, svg: &str) -> Result<(), &'static str> {
        let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default())
            .map_err(|_| "invalid SVG icon")?;
        self.svg_icons.insert(id, tree);
        self.icons.remove(&id);
        self.svg_cache.retain(|(cached, _, _), _| *cached != id);
        Ok(())
    }

    fn icon_at_size(&mut self, id: IconId, width: u32, height: u32) -> Option<BitmapIcon> {
        if let Some(icon) = self.icons.get(&id) {
            return Some(icon.clone());
        }
        let key = (id, width, height);
        if let Some(icon) = self.svg_cache.get(&key) {
            return Some(icon.clone());
        }
        // Icons are small UI assets; never allocate an unbounded raster on malformed geometry.
        if width == 0 || height == 0 || width > 512 || height > 512 {
            return None;
        }
        let tree = self.svg_icons.get(&id)?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
        resvg::render(
            tree,
            resvg::tiny_skia::Transform::from_scale(
                width as f32 / tree.size().width(),
                height as f32 / tree.size().height(),
            ),
            &mut pixmap.as_mut(),
        );
        let mut rgba = pixmap.take();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0))
                .min(255) as u8;
            }
        }
        let icon = BitmapIcon {
            width,
            height,
            rgba: Arc::from(rgba),
        };
        if self.svg_cache.len() >= 128 {
            self.svg_cache.clear();
        }
        self.svg_cache.insert(key, icon.clone());
        Some(icon)
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), &'static str> {
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or("render target dimensions overflow")?;
        self.width = width;
        self.height = height;
        self.pixels.resize(len, 0);
        // `Vec::resize` keeps the old capacity: a window that was once huge
        // would pin that memory for as long as the renderer lives. Release it
        // once the target is less than a quarter of what is reserved.
        if self.pixels.capacity() > RETAINED_RASTER_BYTES
            && self.pixels.capacity() / 4 > self.pixels.len()
        {
            self.pixels.shrink_to(self.pixels.len());
        }
        Ok(())
    }

    fn paint_command(
        &mut self,
        command: &PaintCommand,
        text_system: &mut TextSystem,
        scale: DpiScale,
    ) {
        match &command.op {
            PaintOp::Image {
                rect,
                width,
                height,
                rgba,
            } => {
                let physical = scale.snap_rect(*rect).to_physical(scale);
                let (left, top, right, bottom) = rect_to_bounds(physical, self.width, self.height);
                let (cx0, cy0, cx1, cy1) = command
                    .clip
                    .map(|r| {
                        rect_to_bounds(
                            scale.snap_rect(r).to_physical(scale),
                            self.width,
                            self.height,
                        )
                    })
                    .unwrap_or((0, 0, self.width as i32, self.height as i32));
                if physical.size.width.0 > 0.0 && physical.size.height.0 > 0.0 {
                    for y in top.max(cy0)..bottom.min(cy1) {
                        for x in left.max(cx0)..right.min(cx1) {
                            let sx = (((x as f64 + 0.5 - physical.origin.x.0)
                                / physical.size.width.0)
                                * (*width as f64))
                                .clamp(0.0, (*width - 1) as f64)
                                as usize;
                            let sy = (((y as f64 + 0.5 - physical.origin.y.0)
                                / physical.size.height.0)
                                * (*height as f64))
                                .clamp(0.0, (*height - 1) as f64)
                                as usize;
                            let i = (sy * (*width as usize) + sx) * 4;
                            self.blend_pixel(
                                x as u32,
                                y as u32,
                                Color::rgba(rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]),
                            );
                        }
                    }
                }
            }
            PaintOp::FillRect {
                rect,
                color,
                radius,
            } => {
                self.paint_rect(*rect, command.clip, scale, *color, radius.0, 0.0);
            }
            PaintOp::StrokeRect {
                rect,
                color,
                width,
                radius,
            } => self.paint_rect(*rect, command.clip, scale, *color, radius.0, width.0),
            PaintOp::Text {
                origin,
                layout,
                color,
            } => self.paint_text(*origin, layout, text_system, command.clip, scale, *color),
            PaintOp::Icon { id, rect, tint } => {
                self.paint_icon(*id, *rect, command.clip, scale, *tint);
            }
        }
    }

    fn paint_rect(
        &mut self,
        rect: LogicalRect,
        clip: Option<LogicalRect>,
        scale: DpiScale,
        color: Color,
        radius: f64,
        stroke: f64,
    ) {
        let snapped = scale.snap_rect(rect);
        let physical = snapped.to_physical(scale);
        let clip = clip.unwrap_or_else(|| {
            LogicalRect::from_xywh(
                0.0,
                0.0,
                scale.to_logical(crate::PhysicalPx(self.width as f64)).0,
                scale.to_logical(crate::PhysicalPx(self.height as f64)).0,
            )
        });
        let clip = scale.snap_rect(clip).to_physical(scale);
        let left = physical.origin.x.0.round() as i32;
        let top = physical.origin.y.0.round() as i32;
        let right = (physical.origin.x.0 + physical.size.width.0).round() as i32;
        let bottom = (physical.origin.y.0 + physical.size.height.0).round() as i32;
        let clip_left = clip.origin.x.0.round() as i32;
        let clip_top = clip.origin.y.0.round() as i32;
        let clip_right = (clip.origin.x.0 + clip.size.width.0).round() as i32;
        let clip_bottom = (clip.origin.y.0 + clip.size.height.0).round() as i32;
        let x0 = left.max(clip_left).max(0);
        let y0 = top.max(clip_top).max(0);
        let x1 = right.min(clip_right).min(self.width as i32);
        let y1 = bottom.min(clip_bottom).min(self.height as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (width, height) = (right - left, bottom - top);
        let radius_px = (radius * scale.get())
            .max(0.0)
            .min(f64::from(width.min(height)) / 2.0);
        let stroke_px = if stroke > 0.0 {
            (stroke * scale.get()).round().max(1.0) as i32
        } else {
            0
        };
        // Rows are covered by solid spans (interior, straight edges) plus
        // anti-aliased pixels in the corner zones. A stroke is the outer
        // rounded rectangle minus the same rectangle inset by the stroke
        // width (with a correspondingly smaller radius), so corners keep a
        // continuous border; coverage comes from the signed distance to the
        // arc at the pixel centre, which keeps the old "centre inside" rule
        // at the 50 % level.
        let shape = RoundedShape::new(width, height, radius_px, stroke_px);
        let zone = shape.zone;
        for y in y0..y1 {
            let row = y - top;
            let solid = |renderer: &mut Self, start: i32, end: i32| {
                let xa = (left + start).max(x0);
                let xb = (left + end).min(x1);
                if xa < xb {
                    renderer.fill_span(y, xa, xb, color);
                }
            };
            let full_row = stroke_px == 0 || row < stroke_px || row >= height - stroke_px;
            if radius_px <= 0.0 || !shape.is_band(row) {
                // Straight rows: the whole width, or the two border strips.
                if full_row {
                    solid(self, 0, width);
                } else {
                    solid(self, 0, stroke_px);
                    solid(self, width - stroke_px, width);
                }
                continue;
            }
            // A band row: anti-aliased corner zones plus a solid middle.
            let overlap = 2 * zone >= width;
            let evaluated: [(i32, i32); 2] = if overlap {
                [(0, width), (0, 0)]
            } else {
                [(0, zone), (width - zone, width)]
            };
            if !overlap && full_row {
                solid(self, zone, width - zone);
            }
            for (start, end) in evaluated {
                let xa = (left + start).max(x0);
                let xb = (left + end).min(x1);
                for x in xa..xb {
                    let coverage = shape.coverage(x - left, row);
                    if coverage > 0.0 {
                        let index = (y as usize * self.width as usize + x as usize) * 4;
                        blend_over(
                            &mut self.pixels[index..index + 4],
                            color.with_opacity(coverage as f32),
                        );
                    }
                }
            }
        }
    }

    /// Fill `[xa, xb)` of row `y`; the caller has clipped it to the target.
    fn fill_span(&mut self, y: i32, xa: i32, xb: i32, color: Color) {
        if color.0[3] == u8::MAX {
            let row_start = ((y as usize * self.width as usize) + xa as usize) * 4;
            let row_end = row_start + (xb - xa) as usize * 4;
            for pixel in self.pixels[row_start..row_end].as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&color.0);
            }
        } else if color.0[3] == 0 {
            // Fully transparent: nothing to paint.
        } else {
            let row_start = ((y as usize * self.width as usize) + xa as usize) * 4;
            let row_end = row_start + (xb - xa) as usize * 4;
            for pixel in self.pixels[row_start..row_end].as_chunks_mut::<4>().0 {
                if pixel[3] == 0 {
                    pixel.copy_from_slice(&color.0);
                } else {
                    blend_over(pixel, color);
                }
            }
        }
    }

    fn paint_text(
        &mut self,
        origin: LogicalPoint,
        layout: &TextLayout,
        text_system: &mut TextSystem,
        clip: Option<LogicalRect>,
        scale: DpiScale,
        color: Color,
    ) {
        let origin = origin.to_physical(scale);
        let origin_x = scale.snap_physical(origin.x).0.round() as i32;
        let origin_y = scale.snap_physical(origin.y).0.round() as i32;
        let glyphs = text_system.rasterize(layout, (origin_x, origin_y), scale);
        let clip = clip.map(|rect| {
            rect_to_bounds(
                scale.snap_rect(rect).to_physical(scale),
                self.width,
                self.height,
            )
        });
        for glyph in glyphs {
            self.paint_glyph(&glyph.image, glyph.x, glyph.y, clip, color);
        }
    }

    fn paint_glyph(
        &mut self,
        image: &GlyphImage,
        x: i32,
        y: i32,
        clip: Option<(i32, i32, i32, i32)>,
        color: Color,
    ) {
        let left = x + image.left;
        let top = y - image.top;
        let (clip_left, clip_top, clip_right, clip_bottom) =
            clip.unwrap_or((0, 0, self.width as i32, self.height as i32));
        // Visit only the glyph pixels that land inside the target and the clip.
        let gx0 = (clip_left.max(0) - left).max(0);
        let gx1 = (clip_right.min(self.width as i32) - left).min(image.width as i32);
        let gy0 = (clip_top.max(0) - top).max(0);
        let gy1 = (clip_bottom.min(self.height as i32) - top).min(image.height as i32);
        let bytes_per_pixel = match image.content {
            cosmic_text::SwashContent::Mask => 1,
            cosmic_text::SwashContent::SubpixelMask | cosmic_text::SwashContent::Color => 4,
        };
        for gy in gy0..gy1 {
            for gx in gx0..gx1 {
                let index = (gy as usize * image.width as usize + gx as usize) * bytes_per_pixel;
                if index + bytes_per_pixel > image.data.len() {
                    continue;
                }
                let source = match image.content {
                    cosmic_text::SwashContent::Mask => Color::rgba(
                        color.0[0],
                        color.0[1],
                        color.0[2],
                        ((u16::from(color.0[3]) * u16::from(image.data[index])) / 255) as u8,
                    ),
                    cosmic_text::SwashContent::SubpixelMask => {
                        let mean = image.data[index..index + 3]
                            .iter()
                            .map(|value| u16::from(*value))
                            .sum::<u16>()
                            / 3;
                        Color::rgba(
                            color.0[0],
                            color.0[1],
                            color.0[2],
                            ((u16::from(color.0[3]) * mean) / 255) as u8,
                        )
                    }
                    cosmic_text::SwashContent::Color => Color::rgba(
                        image.data[index],
                        image.data[index + 1],
                        image.data[index + 2],
                        image.data[index + 3],
                    ),
                };
                self.blend_pixel((left + gx) as u32, (top + gy) as u32, source);
            }
        }
    }

    fn paint_icon(
        &mut self,
        id: IconId,
        rect: LogicalRect,
        clip: Option<LogicalRect>,
        scale: DpiScale,
        tint: Color,
    ) {
        let destination = scale.snap_rect(rect).to_physical(scale);
        if destination.size.width.0 <= 0.0 || destination.size.height.0 <= 0.0 {
            return;
        }
        let Some(icon) = self.icon_at_size(
            id,
            destination.size.width.0.round() as u32,
            destination.size.height.0.round() as u32,
        ) else {
            return;
        };
        let bounds = rect_to_bounds(destination, self.width, self.height);
        let clip = clip.map(|rect| {
            rect_to_bounds(
                scale.snap_rect(rect).to_physical(scale),
                self.width,
                self.height,
            )
        });
        for y in bounds.1..bounds.3 {
            for x in bounds.0..bounds.2 {
                if clip.is_some_and(|clip| x < clip.0 || y < clip.1 || x >= clip.2 || y >= clip.3) {
                    continue;
                }
                let sx = ((x as f64 + 0.5 - destination.origin.x.0) / destination.size.width.0
                    * icon.width as f64
                    - 0.5)
                    .clamp(0.0, icon.width.saturating_sub(1) as f64);
                let sy = ((y as f64 + 0.5 - destination.origin.y.0) / destination.size.height.0
                    * icon.height as f64
                    - 0.5)
                    .clamp(0.0, icon.height.saturating_sub(1) as f64);
                let sample = sample_bitmap_icon_bilinear(&icon, sx, sy);
                let source = Color::rgba(
                    ((u16::from(sample[0]) * u16::from(tint.0[0])) / 255) as u8,
                    ((u16::from(sample[1]) * u16::from(tint.0[1])) / 255) as u8,
                    ((u16::from(sample[2]) * u16::from(tint.0[2])) / 255) as u8,
                    ((u16::from(sample[3]) * u16::from(tint.0[3])) / 255) as u8,
                );
                self.blend_pixel(x as u32, y as u32, source);
            }
        }
    }

    fn blend_pixel(&mut self, x: u32, y: u32, source: Color) {
        let index = ((y * self.width + x) * 4) as usize;
        self.blend_at(index, source);
    }

    /// Source-over of `source` onto the pixel at byte offset `index`.
    fn blend_at(&mut self, index: usize, source: Color) {
        blend_over(&mut self.pixels[index..index + 4], source);
    }
}

fn sample_bitmap_icon_bilinear(icon: &BitmapIcon, x: f64, y: f64) -> [u8; 4] {
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(icon.width - 1);
    let y1 = (y0 + 1).min(icon.height - 1);
    let tx = x - f64::from(x0);
    let ty = y - f64::from(y0);
    let pixel = |px: u32, py: u32| {
        let offset = (py as usize * icon.width as usize + px as usize) * 4;
        &icon.rgba[offset..offset + 4]
    };
    let top_left = pixel(x0, y0);
    let top_right = pixel(x1, y0);
    let bottom_left = pixel(x0, y1);
    let bottom_right = pixel(x1, y1);
    std::array::from_fn(|channel| {
        let top = f64::from(top_left[channel]) * (1.0 - tx) + f64::from(top_right[channel]) * tx;
        let bottom =
            f64::from(bottom_left[channel]) * (1.0 - tx) + f64::from(bottom_right[channel]) * tx;
        (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8
    })
}

/// Source-over of `source` onto one RGBA pixel (`pixel.len() == 4`), blending
/// in linear light. Onto a transparent pixel the result is the source itself,
/// the common case for a translucent fill over a fresh layer.
// Channel indexes address both the source colour and the pixel.
#[allow(clippy::needless_range_loop)]
// Kept out of line: the hot span fill handles the transparent case inline.
#[inline(never)]
fn blend_over(pixel: &mut [u8], source: Color) {
    let source_alpha_u8 = source.0[3];
    if source_alpha_u8 == 0 {
        return;
    }
    if source_alpha_u8 == u8::MAX || pixel[3] == 0 {
        pixel.copy_from_slice(&source.0);
        return;
    }
    let tables = color_tables();
    let source_alpha = f64::from(source_alpha_u8) / 255.0;
    if pixel[3] == u8::MAX {
        let inverse_alpha = 1.0 - source_alpha;
        for channel in 0..3 {
            let output_linear = tables.srgb_to_linear[source.0[channel] as usize] * source_alpha
                + tables.srgb_to_linear[pixel[channel] as usize] * inverse_alpha;
            pixel[channel] = linear_to_srgb(output_linear);
        }
        return;
    }
    let destination_alpha = f64::from(pixel[3]) / 255.0;
    let output_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    for channel in 0..3 {
        let source_linear = tables.srgb_to_linear[source.0[channel] as usize];
        let destination_linear = tables.srgb_to_linear[pixel[channel] as usize];
        let output_linear = (source_linear * source_alpha
            + destination_linear * destination_alpha * (1.0 - source_alpha))
            / output_alpha;
        pixel[channel] = linear_to_srgb(output_linear);
    }
    pixel[3] = (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}

impl Renderer for Rgba8Renderer {
    type Error = &'static str;

    fn render(
        &mut self,
        paint: &PaintList,
        text_system: &mut TextSystem,
        scale: DpiScale,
        target_size: PhysicalSize,
    ) -> Result<(), Self::Error> {
        let width = target_size.width.0.round().max(0.0) as u32;
        let height = target_size.height.0.round().max(0.0) as u32;
        self.resize(width, height)?;
        self.pixels.fill(0);
        let commands = paint.commands();
        if commands.is_sorted_by_key(|command| command.layer) {
            for command in commands {
                self.paint_command(command, text_system, scale);
            }
        } else {
            // Stable sort: insertion order is preserved inside each layer.
            let mut ordered: Vec<_> = commands.iter().collect();
            ordered.sort_by_key(|command| command.layer);
            for command in ordered {
                self.paint_command(command, text_system, scale);
            }
        }
        Ok(())
    }
}

/// A rounded rectangle (optionally a stroke ring) in physical pixels, with
/// anti-aliased coverage from the signed distance at each pixel centre.
struct RoundedShape {
    width: f64,
    height: f64,
    radius: f64,
    stroke: f64,
    /// Columns/rows from each edge in which the arcs (and their one-pixel
    /// anti-aliasing fringe) can lie.
    zone: i32,
    rows: i32,
}

impl RoundedShape {
    fn new(width: i32, height: i32, radius: f64, stroke_px: i32) -> Self {
        let zone = (radius.ceil() as i32).max(stroke_px) + 1;
        Self {
            width: f64::from(width),
            height: f64::from(height),
            radius,
            stroke: f64::from(stroke_px),
            zone,
            rows: height,
        }
    }

    /// Rows whose pixels need the signed-distance test: within a zone of the
    /// top or bottom edge.
    fn is_band(&self, row: i32) -> bool {
        row < self.zone || row >= self.rows - self.zone
    }

    /// Coverage in `0.0..=1.0` of the pixel at (`column`, `row`).
    fn coverage(&self, column: i32, row: i32) -> f64 {
        let x = f64::from(column) + 0.5;
        let y = f64::from(row) + 0.5;
        let outer = rounded_coverage(x, y, self.width, self.height, self.radius);
        if self.stroke <= 0.0 || outer <= 0.0 {
            return outer;
        }
        let inner_width = self.width - 2.0 * self.stroke;
        let inner_height = self.height - 2.0 * self.stroke;
        if inner_width <= 0.0 || inner_height <= 0.0 {
            return outer;
        }
        let inner = rounded_coverage(
            x - self.stroke,
            y - self.stroke,
            inner_width,
            inner_height,
            (self.radius - self.stroke).max(0.0),
        );
        outer * (1.0 - inner)
    }
}

/// Coverage of the pixel centred at (`x`, `y`) by a `width` x `height` rounded
/// rectangle at the origin: 1 well inside, 0 well outside, 0.5 on the edge.
fn rounded_coverage(x: f64, y: f64, width: f64, height: f64, radius: f64) -> f64 {
    let half_w = width / 2.0;
    let half_h = height / 2.0;
    let qx = (x - half_w).abs() - (half_w - radius);
    let qy = (y - half_h).abs() - (half_h - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let distance = outside + qx.max(qy).min(0.0) - radius;
    (0.5 - distance).clamp(0.0, 1.0)
}

#[cfg(test)]
fn rounded_contains(x: f64, y: f64, width: f64, height: f64, radius: f64) -> bool {
    if radius <= 0.0 {
        return true;
    }
    let cx = x.clamp(radius, width - radius);
    let cy = y.clamp(radius, height - radius);
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= radius * radius
}

fn rect_to_bounds(rect: crate::PhysicalRect, width: u32, height: u32) -> (i32, i32, i32, i32) {
    (
        rect.origin.x.0.round().max(0.0) as i32,
        rect.origin.y.0.round().max(0.0) as i32,
        (rect.origin.x.0 + rect.size.width.0)
            .round()
            .clamp(0.0, width as f64) as i32,
        (rect.origin.y.0 + rect.size.height.0)
            .round()
            .clamp(0.0, height as f64) as i32,
    )
}

struct ColorTables {
    srgb_to_linear: [f64; 256],
    linear_to_srgb: Box<[u8; 65_536]>,
}

fn color_tables() -> &'static ColorTables {
    static TABLES: OnceLock<ColorTables> = OnceLock::new();
    TABLES.get_or_init(|| ColorTables {
        srgb_to_linear: std::array::from_fn(|index| srgb_to_linear_precise(index as u8)),
        linear_to_srgb: Box::new(std::array::from_fn(|index| {
            linear_to_srgb_precise(index as f64 / 65_535.0)
        })),
    })
}

fn srgb_to_linear_precise(value: u8) -> f64 {
    let value = f64::from(value) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f64) -> u8 {
    let index = (value.clamp(0.0, 1.0) * 65_535.0).round() as usize;
    color_tables().linear_to_srgb[index]
}

fn linear_to_srgb_precise(value: f64) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let srgb = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Explicit conversions used by backends; all scene geometry remains logical.
pub fn raster_rect(rect: LogicalRect, scale: DpiScale) -> LogicalRect {
    scale.snap_rect(rect)
}

pub fn logical_viewport(physical: PhysicalSize, scale: DpiScale) -> LogicalSize {
    physical.to_logical(scale)
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
    #[test]
    fn rgba_image_respects_clip_alpha_and_dpi() {
        use super::*;
        let mut builder = PaintBuilder::with_viewport(LogicalRect::from_xywh(0.0, 0.0, 6.0, 6.0));
        builder.fill_rect(
            LogicalRect::from_xywh(0.0, 0.0, 6.0, 6.0),
            Color::rgba(0, 0, 0, 255),
            0.0,
        );
        builder.push_clip(LogicalRect::from_xywh(2.0, 2.0, 2.0, 2.0));
        assert!(!builder.image(LogicalRect::ZERO, 2, 2, Arc::from([0u8; 3])));
        assert!(builder.image(
            LogicalRect::from_xywh(0.0, 0.0, 6.0, 6.0),
            1,
            1,
            Arc::from([255u8, 0, 0, 128])
        ));
        builder.pop_clip().unwrap();
        let paint = builder.finish();
        for factor in [1.0, 1.5, 2.0] {
            let side = (6.0 * factor) as u32;
            let mut raster = Rgba8Renderer::new();
            raster
                .render(
                    &paint,
                    &mut TextSystem::new(),
                    DpiScale::new(factor).unwrap(),
                    PhysicalSize::new(side as f64, side as f64),
                )
                .unwrap();
            let inside = ((3.0 * factor) as usize * side as usize + (3.0 * factor) as usize) * 4;
            // A 50% linear red sample becomes 188 in sRGB.
            assert_eq!(&raster.pixels()[inside..inside + 4], &[188, 0, 0, 255]);
            assert_eq!(&raster.pixels()[..4], &[0, 0, 0, 255]);
        }
    }
    #[test]
    fn svg_icons_rasterize_at_physical_size_and_invalidate_replaced_source() {
        use super::*;
        let id = IconId(900);
        let mut renderer = Rgba8Renderer::new();
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M2 3 L22 21" stroke="white" stroke-width="2"/></svg>"#;
        renderer.register_svg_icon(id, svg).unwrap();
        for size in [24, 30, 36, 42, 48] {
            let icon = renderer.icon_at_size(id, size, size).unwrap();
            assert_eq!((icon.width, icon.height), (size, size));
            assert_eq!(icon.rgba.len(), size as usize * size as usize * 4);
            let again = renderer.icon_at_size(id, size, size).unwrap();
            assert!(Arc::ptr_eq(&icon.rgba, &again.rgba));
            assert!(icon
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0 && pixel[3] < 255));
        }
        assert_eq!(renderer.svg_cache.len(), 5);
        assert!(renderer.icon_at_size(id, 4096, 4096).is_none());
        renderer.register_svg_icon(id, svg).unwrap();
        assert!(renderer.svg_cache.is_empty());
        assert!(renderer.register_svg_icon(id, "invalid").is_err());
    }
    use super::*;

    #[test]
    fn bitmap_icon_upscaling_interpolates_instead_of_nearest_neighbor() {
        let icon = BitmapIcon {
            width: 2,
            height: 1,
            rgba: Arc::from([
                255, 255, 255, 0, // transparent source texel
                255, 255, 255, 255, // opaque source texel
            ]),
        };
        let interpolated = sample_bitmap_icon_bilinear(&icon, 0.5, 0.0);
        assert_eq!(interpolated, [255, 255, 255, 128]);
    }

    #[test]
    fn nested_clips_intersect_and_layer_order_is_stable() {
        let mut builder =
            PaintBuilder::with_viewport(LogicalRect::from_xywh(0.0, 0.0, 100.0, 100.0));
        builder.push_clip(LogicalRect::from_xywh(10.0, 10.0, 60.0, 60.0));
        builder.push_clip(LogicalRect::from_xywh(40.0, 20.0, 50.0, 50.0));
        builder.fill_rect(
            LogicalRect::from_xywh(0.0, 0.0, 100.0, 100.0),
            Color::rgba(0, 0, 0, 255),
            0.0,
        );
        builder.pop_clip().unwrap();
        builder.pop_clip().unwrap();
        builder.set_layer(PaintLayer::Popup);
        builder.fill_rect(LogicalRect::ZERO, Color::rgba(0, 0, 0, 255), 0.0);
        builder.set_layer(PaintLayer::Content);
        builder.fill_rect(LogicalRect::ZERO, Color::rgba(0, 0, 0, 255), 0.0);
        let mut paint = builder.finish();
        assert_eq!(
            paint.commands()[0].clip,
            Some(LogicalRect::from_xywh(40.0, 20.0, 30.0, 50.0))
        );
        paint.sort_by_layer();
        assert_eq!(paint.commands()[0].layer, PaintLayer::Background);
        assert_eq!(paint.commands()[1].layer, PaintLayer::Content);
        assert_eq!(paint.commands()[2].layer, PaintLayer::Popup);
    }

    #[test]
    fn viewport_conversion_and_snapping_remain_centralized() {
        let scale = DpiScale::new(1.5).unwrap();
        let viewport = logical_viewport(PhysicalSize::new(150.0, 75.0), scale);
        assert_eq!(viewport, LogicalSize::new(100.0, 50.0));
        assert_eq!(
            raster_rect(LogicalRect::from_xywh(1.0, 1.0, 2.0, 2.0), scale),
            LogicalRect::from_xywh(1.3333333333333333, 1.3333333333333333, 2.0, 2.0)
        );
    }

    #[test]
    fn clip_underflow_is_reported() {
        assert_eq!(
            PaintBuilder::new().pop_clip(),
            Err(PaintError::ClipStackUnderflow)
        );
    }

    #[test]
    fn viewport_clip_cannot_be_popped_by_nested_paint_code() {
        let mut builder = PaintBuilder::with_viewport(LogicalRect::from_xywh(0.0, 0.0, 10.0, 10.0));
        assert_eq!(builder.pop_clip(), Err(PaintError::ClipStackUnderflow));
        builder.push_clip(LogicalRect::from_xywh(2.0, 2.0, 4.0, 4.0));
        assert_eq!(builder.pop_clip(), Ok(()));
    }

    #[test]
    fn rgba_renderer_respects_clipping_and_composites_layers() {
        let mut builder = PaintBuilder::new();
        builder.set_layer(PaintLayer::Content);
        builder.push_clip(LogicalRect::from_xywh(1.0, 1.0, 2.0, 2.0));
        builder.fill_rect(
            LogicalRect::from_xywh(0.0, 0.0, 4.0, 4.0),
            Color::rgba(255, 0, 0, 255),
            0.0,
        );
        builder.pop_clip().unwrap();
        builder.set_layer(PaintLayer::Overlay);
        builder.fill_rect(
            LogicalRect::from_xywh(2.0, 2.0, 1.0, 1.0),
            Color::rgba(0, 0, 255, 255),
            0.0,
        );
        let paint = builder.finish();
        let mut renderer = Rgba8Renderer::new();
        let mut text = TextSystem::new();
        renderer
            .render(
                &paint,
                &mut text,
                DpiScale::ONE,
                PhysicalSize::new(4.0, 4.0),
            )
            .unwrap();
        let pixel = |x: usize, y: usize| &renderer.pixels()[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4];
        assert_eq!(pixel(0, 0), &[0, 0, 0, 0]);
        assert_eq!(pixel(1, 1), &[255, 0, 0, 255]);
        assert_eq!(pixel(2, 2), &[0, 0, 255, 255]);
    }

    #[test]
    fn shaped_layout_is_rasterized_by_the_same_text_system() {
        let mut text = TextSystem::new();
        let layout = Arc::new(text.shape("Everty Файл №7", &crate::TextStyle::default(), None));
        let mut builder = PaintBuilder::new();
        builder.text(
            LogicalPoint::new(2.0, 2.0),
            layout,
            Color::rgba(20, 30, 40, 255),
        );
        let paint = builder.finish();
        let mut renderer = Rgba8Renderer::new();
        renderer
            .render(
                &paint,
                &mut text,
                DpiScale::ONE,
                PhysicalSize::new(240.0, 48.0),
            )
            .unwrap();
        assert!(renderer
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] != 0));
        assert!(text.cache_stats().glyphs > 0);
    }

    fn render_opaque(
        size: (u32, u32),
        scale: f64,
        build: impl FnOnce(&mut PaintBuilder),
    ) -> (Vec<u8>, u32) {
        let mut builder = PaintBuilder::new();
        build(&mut builder);
        let mut renderer = Rgba8Renderer::new();
        let mut text = TextSystem::with_cache_limits(8, 4096);
        renderer
            .render(
                &builder.finish(),
                &mut text,
                DpiScale::new(scale).unwrap(),
                PhysicalSize::new(f64::from(size.0), f64::from(size.1)),
            )
            .unwrap();
        (renderer.pixels().to_vec(), size.0)
    }

    #[test]
    fn rounded_fill_is_antialiased_and_agrees_with_the_centre_rule_at_half_coverage() {
        for (width, height) in [(1u32, 1u32), (5, 9), (22, 22), (36, 14), (30, 30)] {
            for radius in [0.0f64, 0.5, 1.0, 2.5, 4.0, 7.0, 18.0] {
                let radius = radius.min(f64::from(width.min(height)) / 2.0);
                let (pixels, stride) = render_opaque((width + 4, height + 4), 1.0, |builder| {
                    builder.fill_rect(
                        LogicalRect::from_xywh(2.0, 2.0, f64::from(width), f64::from(height)),
                        Color::rgba(10, 20, 30, 255),
                        radius,
                    );
                });
                let mut partial = 0;
                for row in 0..height {
                    for column in 0..width {
                        let coverage = rounded_coverage(
                            f64::from(column) + 0.5,
                            f64::from(row) + 0.5,
                            f64::from(width),
                            f64::from(height),
                            radius,
                        );
                        let index = (((row + 2) * stride + column + 2) * 4 + 3) as usize;
                        let alpha = pixels[index];
                        assert!(
                            (f64::from(alpha) - coverage * 255.0).abs() <= 1.0,
                            "{width}x{height} r{radius} ({column},{row}): alpha {alpha} vs {coverage}"
                        );
                        if (coverage - 0.5).abs() > 1e-6 {
                            let inside = rounded_contains(
                                f64::from(column) + 0.5,
                                f64::from(row) + 0.5,
                                f64::from(width),
                                f64::from(height),
                                radius,
                            );
                            assert_eq!(alpha >= 128, inside, "{width}x{height} r{radius}");
                        }
                        if alpha != 0 && alpha != 255 {
                            partial += 1;
                        }
                    }
                }
                // Nothing leaks outside the rectangle.
                for y in 0..height + 4 {
                    for x in 0..width + 4 {
                        if x < 2 || y < 2 || x >= width + 2 || y >= height + 2 {
                            assert_eq!(pixels[((y * stride + x) * 4 + 3) as usize], 0);
                        }
                    }
                }
                if radius >= 4.0 && width.min(height) >= 14 {
                    assert!(partial >= 4, "r{radius} corners must have a soft fringe");
                }
            }
        }
    }

    #[test]
    fn rounded_stroke_is_one_continuous_ring_including_the_corners() {
        let red = Color::rgba(255, 0, 0, 255);
        for (radius, stroke) in [(6.0, 1.0), (6.0, 2.0), (3.0, 1.0), (10.0, 3.0), (0.0, 1.0)] {
            let (pixels, stride) = render_opaque((28, 28), 1.0, |builder| {
                builder.stroke_rect(
                    LogicalRect::from_xywh(3.0, 3.0, 22.0, 22.0),
                    red,
                    stroke,
                    radius,
                );
            });
            let painted = |x: i32, y: i32| {
                x >= 0
                    && y >= 0
                    && x < 28
                    && y < 28
                    && pixels[((y as u32 * stride + x as u32) * 4 + 3) as usize] != 0
            };
            let inside = |x: i32, y: i32, left: i32, size: i32, radius: f64| {
                (left..left + size).contains(&x)
                    && (left..left + size).contains(&y)
                    && rounded_contains(
                        f64::from(x - left) + 0.5,
                        f64::from(y - left) + 0.5,
                        f64::from(size),
                        f64::from(size),
                        radius,
                    )
            };
            let s = stroke as i32;
            for y in 0..28 {
                for x in 0..28 {
                    let outer = inside(x, y, 3, 22, radius);
                    let inner = x >= 3 + s
                        && y >= 3 + s
                        && inside(x - s, y - s, 3, 22 - 2 * s, (radius - stroke).max(0.0));
                    // Anti-aliasing softens the edge pixels but never opens a
                    // hole in the ring, and nothing is painted away from it.
                    if outer && !inner {
                        assert!(
                            painted(x, y),
                            "radius {radius} stroke {stroke} pixel ({x},{y})"
                        );
                    }
                    if painted(x, y) {
                        assert!(
                            (-1..=1).any(|dy| (-1..=1).any(|dx| inside(
                                x + dx,
                                y + dy,
                                3,
                                22,
                                radius
                            ))),
                            "stray pixel ({x},{y}) r{radius} s{stroke}"
                        );
                    }
                    if outer
                        && [(1, 0), (-1, 0), (0, 1), (0, -1)]
                            .iter()
                            .any(|(dx, dy)| !inside(x + dx, y + dy, 3, 22, radius))
                    {
                        assert!(
                            painted(x, y),
                            "gap in the ring at ({x},{y}) r{radius} s{stroke}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn abutting_rectangles_have_no_seam_or_overlap_at_every_required_scale() {
        for factor in [1.0f64, 1.25, 1.5, 1.75, 2.0] {
            // Edges deliberately land between device pixels.
            let split = 10.3;
            let (pixels, stride) = render_opaque(
                ((24.0 * factor).ceil() as u32, (6.0 * factor).ceil() as u32),
                factor,
                |builder| {
                    builder.fill_rect(
                        LogicalRect::from_xywh(0.0, 0.0, split, 5.0),
                        Color::rgba(255, 0, 0, 255),
                        0.0,
                    );
                    builder.fill_rect(
                        LogicalRect::from_xywh(split, 0.0, 24.0 - split, 5.0),
                        Color::rgba(0, 0, 255, 255),
                        0.0,
                    );
                },
            );
            let edge = (split * factor).round() as u32;
            let row = (2.0 * factor) as u32;
            for x in 0..(24.0 * factor).round() as u32 {
                let pixel = &pixels[((row * stride + x) * 4) as usize..][..4];
                let expected = if x < edge {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                };
                assert_eq!(pixel, expected, "{factor}x seam/overlap at x={x}");
            }
        }
    }

    #[test]
    fn color_lookup_tables_match_srgb_transfer_functions_within_one_code_value() {
        for byte in 0..=u8::MAX {
            assert_eq!(
                linear_to_srgb(color_tables().srgb_to_linear[byte as usize]),
                byte
            );
        }
        for step in 0..=65_535u32 {
            let value = f64::from(step) / 65_535.0;
            assert!(
                linear_to_srgb(value).abs_diff(linear_to_srgb_precise(value)) <= 1,
                "LUT exceeded one sRGB code value at linear input {value}"
            );
        }
    }

    #[test]
    fn partial_alpha_blend_stays_linear_light_for_opaque_and_transparent_destinations() {
        let mut renderer = Rgba8Renderer::new();
        renderer.resize(1, 1).unwrap();
        renderer.pixels.copy_from_slice(&[255, 0, 0, 255]);
        renderer.blend_pixel(0, 0, Color::rgba(0, 0, 255, 128));

        let alpha = 128.0 / 255.0;
        let expected = [
            linear_to_srgb_precise(
                srgb_to_linear_precise(0) * alpha + srgb_to_linear_precise(255) * (1.0 - alpha),
            ),
            0,
            linear_to_srgb_precise(
                srgb_to_linear_precise(255) * alpha + srgb_to_linear_precise(0) * (1.0 - alpha),
            ),
            255,
        ];
        for (actual, expected) in renderer.pixels.iter().zip(expected) {
            assert!(actual.abs_diff(expected) <= 1);
        }

        renderer.pixels.fill(0);
        renderer.blend_pixel(0, 0, Color::rgba(12, 34, 56, 128));
        assert_eq!(renderer.pixels, [12, 34, 56, 128]);
    }

    #[test]
    fn translucent_span_fill_equals_per_pixel_blend_on_mixed_destinations() {
        // Transparent, semi-transparent and opaque destination pixels in one row.
        let destination: [u8; 16] = [0, 0, 0, 0, 10, 200, 30, 90, 250, 5, 5, 255, 0, 0, 0, 0];
        for alpha in [1_u8, 90, 140, 254] {
            let color = Color::rgba(18, 26, 38, alpha);
            let mut spans = Rgba8Renderer::new();
            spans.resize(4, 1).unwrap();
            spans.pixels.copy_from_slice(&destination);
            spans.fill_span(0, 0, 4, color);
            let mut pixels = Rgba8Renderer::new();
            pixels.resize(4, 1).unwrap();
            pixels.pixels.copy_from_slice(&destination);
            for x in 0..4 {
                pixels.blend_pixel(x, 0, color);
            }
            assert_eq!(spans.pixels, pixels.pixels, "alpha {alpha}");
        }
        // A fully transparent fill changes nothing.
        let mut untouched = Rgba8Renderer::new();
        untouched.resize(4, 1).unwrap();
        untouched.pixels.copy_from_slice(&destination);
        untouched.fill_span(0, 0, 4, Color::rgba(1, 2, 3, 0));
        assert_eq!(untouched.pixels, destination);
    }

    #[test]
    fn raster_memory_is_released_after_a_huge_window_shrinks() {
        let mut renderer = Rgba8Renderer::new();
        let mut text = TextSystem::with_cache_limits(1, 1024);
        let paint = PaintBuilder::new().finish();
        let render = |renderer: &mut Rgba8Renderer, text: &mut TextSystem, size: f64| {
            renderer
                .render(&paint, text, DpiScale::ONE, PhysicalSize::new(size, size))
                .unwrap();
        };
        render(&mut renderer, &mut text, 2048.0);
        assert!(renderer.pixels.capacity() >= 2048 * 2048 * 4);
        render(&mut renderer, &mut text, 64.0);
        assert!(
            renderer.pixels.capacity() < 1024 * 1024,
            "capacity {} still pinned after shrinking to 64x64",
            renderer.pixels.capacity()
        );
        // Ordinary small resizes do not churn the allocation.
        render(&mut renderer, &mut text, 64.0);
        let stable = renderer.pixels.capacity();
        render(&mut renderer, &mut text, 48.0);
        assert_eq!(renderer.pixels.capacity(), stable);
    }
}
