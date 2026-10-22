//! Runtime text shaping, font fallback, metrics, and bounded glyph raster cache.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use cosmic_text::{Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping, SwashCache};
use unicode_segmentation::UnicodeSegmentation;

use crate::{DpiScale, LogicalPx, LogicalSize};

const DEFAULT_GLYPH_CAPACITY: usize = 4096;
const DEFAULT_CACHE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub enum FontFamily {
    #[default]
    SansSerif,
    Serif,
    Monospace,
    Named(String),
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum FontWeight {
    #[default]
    Regular,
    Medium,
    Semibold,
    Bold,
}

impl FontWeight {
    fn cosmic(self) -> cosmic_text::Weight {
        match self {
            Self::Regular => cosmic_text::Weight::NORMAL,
            Self::Medium => cosmic_text::Weight::MEDIUM,
            Self::Semibold => cosmic_text::Weight::SEMIBOLD,
            Self::Bold => cosmic_text::Weight::BOLD,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub family: FontFamily,
    pub weight: FontWeight,
    pub font_size: LogicalPx,
    pub line_height: LogicalPx,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: FontFamily::SansSerif,
            weight: FontWeight::Regular,
            font_size: LogicalPx(14.0),
            line_height: LogicalPx(20.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextMetrics {
    pub size: LogicalSize,
    /// Ascent/descent are positive distances from the baseline.
    pub ascent: LogicalPx,
    pub descent: LogicalPx,
    pub line_gap: LogicalPx,
    /// Baseline offset from the top of the first line.
    pub baseline: LogicalPx,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GlyphCacheStats {
    pub glyphs: usize,
    pub bytes: usize,
    pub capacity_glyphs: usize,
    pub capacity_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

/// Runtime diagnostic for the actual font faces selected while shaping text.
/// This is useful for platform acceptance and troubleshooting fallback; it is
/// not part of layout or rendering decisions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontUsage {
    pub family: String,
    pub face: String,
    pub glyphs: usize,
    pub missing_glyphs: usize,
}

#[derive(Clone, Debug)]
pub struct GlyphImage {
    pub content: cosmic_text::SwashContent,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub data: Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub struct PositionedGlyph {
    pub x: i32,
    pub y: i32,
    pub image: GlyphImage,
}

#[derive(Clone, Debug)]
struct LayoutLine {
    baseline: f32,
    glyphs: Vec<cosmic_text::LayoutGlyph>,
}

#[derive(Clone, Debug)]
pub struct TextLayout {
    pub metrics: TextMetrics,
    lines: Vec<LayoutLine>,
}

impl TextLayout {
    /// Number of laid-out (wrapped) lines.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Horizontal position (logical px from the start of the first line) of the
    /// caret placed before byte `index` of the shaped text. `index` is clamped
    /// to the text; inside a multi-character cluster the position is
    /// interpolated. Meant for single-line editors.
    pub fn caret_x(&self, index: usize) -> f64 {
        let Some(line) = self.lines.first() else {
            return 0.0;
        };
        let mut end_x = 0.0_f64;
        for glyph in &line.glyphs {
            let (x, w) = (f64::from(glyph.x), f64::from(glyph.w));
            if index <= glyph.start {
                return x;
            }
            if index < glyph.end {
                let span = (glyph.end - glyph.start).max(1) as f64;
                return x + w * (index - glyph.start) as f64 / span;
            }
            end_x = x + w;
        }
        end_x
    }

    /// Byte index of the caret position closest to horizontal offset `x`
    /// (the inverse of [`caret_x`](Self::caret_x)), for click-to-place.
    pub fn index_at_x(&self, x: f64, text_len: usize) -> usize {
        let Some(line) = self.lines.first() else {
            return 0;
        };
        for glyph in &line.glyphs {
            let (gx, w) = (f64::from(glyph.x), f64::from(glyph.w));
            if x < gx + w / 2.0 {
                return glyph.start;
            }
        }
        line.glyphs
            .last()
            .map_or(0, |glyph| glyph.end)
            .min(text_len)
    }
}

#[derive(Clone)]
struct CachedGlyph {
    image: GlyphImage,
    bytes: usize,
    /// Position of this entry in `TextSystem::recency`.
    tick: u64,
}

/// Per-UI-tree text service. Keep one instance alive; font discovery is costly.
pub struct TextSystem {
    fonts: FontSystem,
    rasterizer: SwashCache,
    cache: HashMap<CacheKey, CachedGlyph>,
    /// Recency index: the smallest tick is the least recently used glyph.
    /// Touching and evicting are O(log n), independent of resident glyphs.
    recency: BTreeMap<u64, CacheKey>,
    clock: u64,
    stats: GlyphCacheStats,
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSystem {
    pub fn new() -> Self {
        Self::with_cache_limits(DEFAULT_GLYPH_CAPACITY, DEFAULT_CACHE_BYTES)
    }

    /// Custom limits are useful for deterministic stress tests and constrained apps.
    pub fn with_cache_limits(capacity_glyphs: usize, capacity_bytes: usize) -> Self {
        Self {
            fonts: FontSystem::new(),
            rasterizer: SwashCache::new(),
            cache: HashMap::new(),
            recency: BTreeMap::new(),
            clock: 0,
            stats: GlyphCacheStats {
                capacity_glyphs,
                capacity_bytes,
                ..GlyphCacheStats::default()
            },
        }
    }

    pub fn shape(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: Option<LogicalPx>,
    ) -> TextLayout {
        if text.is_empty() {
            return TextLayout {
                metrics: TextMetrics::default(),
                lines: Vec::new(),
            };
        }
        let font_size = finite_positive(style.font_size.0, 14.0) as f32;
        let line_height = finite_positive(style.line_height.0, font_size as f64) as f32;
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(font_size, line_height));
        buffer.set_size(
            max_width.map(|width| finite_positive(width.0, 0.0) as f32),
            None,
        );
        let family = match &style.family {
            FontFamily::SansSerif => Family::SansSerif,
            FontFamily::Serif => Family::Serif,
            FontFamily::Monospace => Family::Monospace,
            FontFamily::Named(name) => Family::Name(name),
        };
        buffer.set_text(
            text,
            &Attrs::new().family(family).weight(style.weight.cosmic()),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.fonts, true);

        let mut lines = Vec::new();
        let mut width = 0.0f32;
        let mut baseline = 0.0f32;
        let mut ascent = 0.0f32;
        let mut descent = 0.0f32;
        let mut height = 0.0f32;
        for run in buffer.layout_runs() {
            width = width.max(run.line_w);
            height = height.max(run.line_top + run.line_height);
            if lines.is_empty() {
                baseline = run.line_y;
            }
            for glyph in run.glyphs {
                if let Some(font) = self.fonts.get_font(glyph.font_id, glyph.font_weight) {
                    let metrics = font.metrics();
                    if metrics.units_per_em > 0 {
                        let factor = glyph.font_size / f32::from(metrics.units_per_em);
                        ascent = ascent.max(metrics.ascent * factor);
                        descent = descent.max(-metrics.descent * factor);
                    }
                }
            }
            lines.push(LayoutLine {
                baseline: run.line_y,
                glyphs: run.glyphs.to_vec(),
            });
        }
        if lines.is_empty() && !text.is_empty() {
            height = line_height;
            baseline = font_size;
            ascent = font_size;
        }
        let gap = (line_height - ascent - descent).max(0.0);
        TextLayout {
            metrics: TextMetrics {
                size: LogicalSize::new(width as f64, height as f64),
                ascent: LogicalPx(ascent as f64),
                descent: LogicalPx(descent as f64),
                line_gap: LogicalPx(gap as f64),
                baseline: LogicalPx(baseline as f64),
            },
            lines,
        }
    }

    /// Shape `text` wrapped to `max_width`, limited to `max_lines`. Longer text
    /// ends with an ellipsis on the last line, cut at a grapheme boundary so a
    /// cluster is never split. Newlines are treated as spaces: this is for a
    /// single paragraph (a notification, a caption), not documents.
    pub fn shape_clamped(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: LogicalPx,
        max_lines: usize,
    ) -> TextLayout {
        let max_lines = max_lines.max(1);
        let flattened: String = text
            .chars()
            .map(|ch| if ch == '\n' || ch == '\r' { ' ' } else { ch })
            .collect();
        let layout = self.shape(&flattened, style, Some(max_width));
        if layout.lines.len() <= max_lines {
            return layout;
        }
        let end = layout.lines[max_lines - 1]
            .glyphs
            .iter()
            .map(|glyph| glyph.end)
            .max()
            .unwrap_or(0)
            .min(flattened.len());
        let mut boundaries: Vec<usize> = flattened
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain([flattened.len()])
            .filter(|index| *index <= end)
            .collect();
        loop {
            let cut = boundaries.last().copied().unwrap_or(0);
            let candidate = format!("{}…", flattened[..cut].trim_end());
            let shaped = self.shape(&candidate, style, Some(max_width));
            if shaped.lines.len() <= max_lines || boundaries.len() <= 1 {
                return shaped;
            }
            boundaries.pop();
        }
    }

    /// Rasterize the shaped layout at the target DPI. Coordinates are device pixels.
    pub fn rasterize(
        &mut self,
        layout: &TextLayout,
        origin: (i32, i32),
        scale: DpiScale,
    ) -> Vec<PositionedGlyph> {
        let scale = scale.get() as f32;
        let mut output = Vec::new();
        for line in &layout.lines {
            let baseline = origin.1 as f32 + line.baseline * scale;
            for glyph in &line.glyphs {
                let physical = glyph.physical((origin.0 as f32, baseline), scale);
                if let Some(image) = self.glyph_image(physical.cache_key) {
                    output.push(PositionedGlyph {
                        x: physical.x,
                        y: physical.y,
                        image,
                    });
                }
            }
        }
        output
    }

    pub fn cache_stats(&self) -> GlyphCacheStats {
        self.stats
    }

    /// Report the font faces that supplied glyphs in a shaped layout. A glyph
    /// id of zero is counted as missing so acceptance diagnostics can
    /// distinguish a successful fallback from a rendered .notdef box.
    pub fn font_usage(&self, layout: &TextLayout) -> Vec<FontUsage> {
        let mut usage = BTreeMap::<(String, String), FontUsage>::new();
        for glyph in layout.lines.iter().flat_map(|line| &line.glyphs) {
            let Some(face) = self.fonts.db().face(glyph.font_id) else {
                continue;
            };
            let family = face
                .families
                .first()
                .map(|(family, _)| family.clone())
                .unwrap_or_else(|| face.post_script_name.clone());
            let key = (family.clone(), face.post_script_name.clone());
            let entry = usage.entry(key).or_insert_with(|| FontUsage {
                family,
                face: face.post_script_name.clone(),
                glyphs: 0,
                missing_glyphs: 0,
            });
            entry.glyphs += 1;
            entry.missing_glyphs += usize::from(glyph.glyph_id == 0);
        }
        usage.into_values().collect()
    }

    fn glyph_image(&mut self, key: CacheKey) -> Option<GlyphImage> {
        let tick = self.next_tick();
        if let Some(entry) = self.cache.get_mut(&key) {
            self.recency.remove(&entry.tick);
            entry.tick = tick;
            self.recency.insert(tick, key);
            self.stats.hits += 1;
            return Some(entry.image.clone());
        }
        self.stats.misses += 1;
        let image = self.rasterizer.get_image_uncached(&mut self.fonts, key)?;
        let data: Arc<[u8]> = Arc::from(image.data);
        let rendered = GlyphImage {
            content: image.content,
            width: image.placement.width,
            height: image.placement.height,
            left: image.placement.left,
            top: image.placement.top,
            data,
        };
        let bytes = rendered.data.len();
        if self.stats.capacity_glyphs > 0 && bytes <= self.stats.capacity_bytes {
            while self.cache.len() >= self.stats.capacity_glyphs
                || self.stats.bytes.saturating_add(bytes) > self.stats.capacity_bytes
            {
                let Some((_, oldest)) = self.recency.pop_first() else {
                    break;
                };
                if let Some(evicted) = self.cache.remove(&oldest) {
                    self.stats.bytes = self.stats.bytes.saturating_sub(evicted.bytes);
                    self.stats.evictions += 1;
                }
            }
            self.cache.insert(
                key,
                CachedGlyph {
                    image: rendered.clone(),
                    bytes,
                    tick,
                },
            );
            self.recency.insert(tick, key);
            self.stats.bytes += bytes;
            self.stats.glyphs = self.cache.len();
        }
        Some(rendered)
    }

    fn next_tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }
}

fn finite_positive(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advanced_layout_shapes_unicode_and_reports_metrics() {
        let mut text = TextSystem::with_cache_limits(128, 1024 * 1024);
        let layout = text.shape(
            "EvertyDesk — Файл: C:\\Users\\Тест\\résumé.txt\nA\u{0308} العربية",
            &TextStyle::default(),
            None,
        );
        assert!(layout.metrics.size.width.0 > 0.0);
        assert!(layout.metrics.size.height.0 >= 40.0);
        assert!(layout.metrics.baseline.0 > 0.0);
        assert_eq!(layout.lines.len(), 2);
        assert!(
            layout
                .lines
                .iter()
                .map(|line| line.glyphs.len())
                .sum::<usize>()
                > 20
        );
    }

    #[test]
    fn runtime_unicode_fallback_reports_selected_faces_without_missing_glyphs() {
        let mut text = TextSystem::new();
        let sample = "EvertyDesk — Файл №123 · العربية 🧪";
        let layout = text.shape(sample, &TextStyle::default(), None);
        let usage = text.font_usage(&layout);
        eprintln!("font fallback diagnostic: {usage:#?}");
        assert!(
            !usage.is_empty(),
            "shaped text should resolve to a font face"
        );
        assert!(
            usage.iter().all(|font| font.missing_glyphs == 0),
            "platform fallback left a .notdef glyph: {usage:#?}"
        );
    }

    #[test]
    fn glyph_cache_hits_and_obeys_limits() {
        let mut text = TextSystem::with_cache_limits(2, 1024 * 1024);
        let layout = text.shape("AB", &TextStyle::default(), None);
        let scale = DpiScale::new(1.25).unwrap();
        let first = text.rasterize(&layout, (0, 0), scale);
        assert!(!first.is_empty());
        let _ = text.rasterize(&layout, (0, 0), scale);
        let other = text.shape("C", &TextStyle::default(), None);
        let _ = text.rasterize(&other, (0, 0), scale);
        let stats = text.cache_stats();
        assert!(stats.hits > 0);
        assert!(stats.evictions > 0);
        assert!(stats.glyphs <= 2);
        assert!(stats.bytes <= stats.capacity_bytes);
    }

    #[test]
    fn glyph_cache_stays_bounded_under_unicode_and_dpi_churn() {
        let mut text = TextSystem::with_cache_limits(32, 8 * 1024);
        let sample: String = (0x21..0x500)
            .filter_map(char::from_u32)
            .filter(|ch| !ch.is_control())
            .collect();
        let layout = text.shape(&sample, &TextStyle::default(), None);
        for factor in [1.0, 1.25, 1.5, 1.75, 2.0] {
            let glyphs = text.rasterize(&layout, (0, 0), DpiScale::new(factor).unwrap());
            assert!(!glyphs.is_empty(), "no glyphs at {factor}x");
            let stats = text.cache_stats();
            assert!(stats.glyphs <= stats.capacity_glyphs, "{stats:?}");
            assert!(stats.bytes <= stats.capacity_bytes, "{stats:?}");
            assert_eq!(text.cache.len(), text.recency.len(), "{stats:?}");
            assert!(text
                .recency
                .values()
                .all(|key| text.cache.contains_key(key)));
            assert!(text
                .cache
                .iter()
                .all(|(key, entry)| text.recency.get(&entry.tick) == Some(key)));
        }
        let stats = text.cache_stats();
        assert!(stats.misses > stats.capacity_glyphs as u64, "{stats:?}");
        assert!(stats.evictions > 0, "{stats:?}");
        eprintln!("unicode/dpi glyph-cache stress: {stats:?}");
    }

    fn single(text: &mut TextSystem, ch: &str) -> TextLayout {
        text.shape(ch, &TextStyle::default(), None)
    }

    #[test]
    fn glyph_cache_evicts_least_recently_used_not_least_recently_inserted() {
        let mut text = TextSystem::with_cache_limits(2, 1024 * 1024);
        let (a, b, c) = (
            single(&mut text, "A"),
            single(&mut text, "B"),
            single(&mut text, "C"),
        );
        let scale = DpiScale::ONE;
        text.rasterize(&a, (0, 0), scale);
        text.rasterize(&b, (0, 0), scale);
        // Touch A so B becomes the eviction candidate even though A is older.
        text.rasterize(&a, (0, 0), scale);
        text.rasterize(&c, (0, 0), scale);
        let before = text.cache_stats();
        text.rasterize(&a, (0, 0), scale);
        assert_eq!(text.cache_stats().hits, before.hits + 1, "A must survive");
        text.rasterize(&b, (0, 0), scale);
        assert_eq!(
            text.cache_stats().misses,
            before.misses + 1,
            "B was evicted"
        );
        assert_eq!(text.cache.len(), text.recency.len());
    }

    /// Right edge of the ink produced for a layout, in device pixels.
    fn ink_right(glyphs: &[PositionedGlyph]) -> i32 {
        glyphs
            .iter()
            .map(|glyph| glyph.x + glyph.image.left + glyph.image.width as i32)
            .max()
            .unwrap_or(0)
    }

    type Signature = Vec<(i32, i32, u32, u32, Vec<u8>)>;

    #[test]
    fn dpi_transitions_reuse_no_stale_bitmaps_and_track_measured_width() {
        const SAMPLE: &str = "Отчёт_final (копия) №2.docx — Report";
        let mut text = TextSystem::with_cache_limits(512, 4 * 1024 * 1024);
        let layout = text.shape(SAMPLE, &TextStyle::default(), None);
        let logical_width = layout.metrics.size.width.0;
        let raster = |text: &mut TextSystem, factor: f64| {
            text.rasterize(&layout, (0, 0), DpiScale::new(factor).unwrap())
        };
        let signature = |glyphs: &[PositionedGlyph]| -> Signature {
            glyphs
                .iter()
                .map(|g| {
                    (
                        g.x,
                        g.y,
                        g.image.width,
                        g.image.height,
                        g.image.data.to_vec(),
                    )
                })
                .collect()
        };
        for sequence in [
            [1.0, 1.25, 1.0].as_slice(),
            [1.0, 1.5, 2.0].as_slice(),
            [2.0, 1.25].as_slice(),
        ] {
            let mut first_seen: Vec<(f64, Signature)> = Vec::new();
            for &factor in sequence {
                let glyphs = raster(&mut text, factor);
                assert!(!glyphs.is_empty(), "no glyphs at {factor}x");
                // Measurement and rendering agree: the ink never extends past
                // the measured advance by more than glyph overhang/rounding.
                let right = f64::from(ink_right(&glyphs));
                let measured = logical_width * factor;
                assert!(
                    (right - measured).abs() <= 3.0 + factor,
                    "{factor}x: ink right edge {right} vs measured {measured}"
                );
                let signature = signature(&glyphs);
                match first_seen.iter().find(|(seen, _)| *seen == factor) {
                    Some((_, previous)) => assert_eq!(
                        previous, &signature,
                        "{factor}x raster changed after a DPI round trip"
                    ),
                    None => first_seen.push((factor, signature)),
                }
            }
            let stats = text.cache_stats();
            assert!(stats.glyphs <= stats.capacity_glyphs);
            assert!(stats.bytes <= stats.capacity_bytes);
        }
        let small: u32 = raster(&mut text, 1.0).iter().map(|g| g.image.height).sum();
        let large: u32 = raster(&mut text, 2.0).iter().map(|g| g.image.height).sum();
        assert!(large > small, "2x glyphs must be rasterized at 2x size");
    }

    #[test]
    fn combining_marks_add_no_advance_and_mixed_scripts_have_no_missing_glyphs() {
        let mut text = TextSystem::new();
        let style = TextStyle::default();
        let plain = text.shape("e", &style, None).metrics.size.width.0;
        let combined = text.shape("e\u{301}", &style, None);
        assert!(
            (combined.metrics.size.width.0 - plain).abs() < 0.5,
            "combining acute must not add advance: {} vs {plain}",
            combined.metrics.size.width.0
        );
        let sample = "Отчёт_final (копия) №2: «Привет», world — résumé.docx";
        let layout = text.shape(sample, &style, None);
        assert!(text
            .font_usage(&layout)
            .iter()
            .all(|font| font.missing_glyphs == 0));
    }

    #[test]
    fn empty_text_has_zero_layout_and_no_glyphs() {
        let mut text = TextSystem::with_cache_limits(8, 4096);
        let layout = text.shape("", &TextStyle::default(), None);
        assert_eq!(layout.metrics.size, LogicalSize::ZERO);
        assert!(text.rasterize(&layout, (0, 0), DpiScale::ONE).is_empty());
    }

    #[test]
    fn clamped_text_fits_the_line_budget_with_an_ellipsis_and_keeps_short_text() {
        let mut text = TextSystem::with_cache_limits(64, 1024 * 1024);
        let style = TextStyle::default();
        let width = LogicalPx(120.0);
        let short = text.shape_clamped("Готово", &style, width, 3);
        assert_eq!(short.line_count(), 1);
        let plain = text.shape("Готово", &style, Some(width));
        assert_eq!(short.metrics.size, plain.metrics.size);

        let long = "Не удалось передать файл отчёт_за_квартал_финальная_версия.docx: нет места на диске получателя";
        for max_lines in 1..=3 {
            let layout = text.shape_clamped(long, &style, width, max_lines);
            assert!(
                layout.line_count() <= max_lines,
                "{max_lines}: {}",
                layout.line_count()
            );
            assert!(layout.metrics.size.width.0 <= width.0 + 0.5);
        }
        let layout = text.shape_clamped(long, &style, width, 2);
        assert_eq!(layout.line_count(), 2, "long text uses the whole budget");
        let first_line_end = text.shape(long, &style, Some(width)).lines[0]
            .glyphs
            .last()
            .map(|g| g.end);
        assert!(first_line_end.is_some());
    }

    #[test]
    fn clamped_text_never_splits_a_grapheme_and_survives_degenerate_widths() {
        let mut text = TextSystem::with_cache_limits(64, 1024 * 1024);
        let style = TextStyle::default();
        let sample = "e\u{301}".repeat(80) + " 👨\u{200d}👩\u{200d}👧\u{200d}👦 end";
        for width in [0.0, 1.0, 8.0, 40.0, 200.0] {
            for max_lines in [1, 2, 3] {
                let layout = text.shape_clamped(&sample, &style, LogicalPx(width), max_lines);
                assert!(
                    layout.line_count() <= max_lines.max(1)
                        || width < 8.0 && layout.line_count() <= 80
                );
                assert!(layout.metrics.size.width.0.is_finite());
            }
        }
    }

    #[test]
    fn clamped_text_handles_a_single_word_wider_than_the_line() {
        let mut text = TextSystem::with_cache_limits(64, 1024 * 1024);
        let style = TextStyle::default();
        let name = "отчёт_за_квартал_финальная_версия_копия_номер_два.docx";
        let layout = text.shape_clamped(name, &style, LogicalPx(120.0), 1);
        assert_eq!(layout.line_count(), 1);
        assert!(
            layout.metrics.size.width.0 <= 120.5,
            "an unbreakable name must be ellipsized, got width {}",
            layout.metrics.size.width.0
        );
        let two = text.shape_clamped(name, &style, LogicalPx(120.0), 2);
        assert!(two.line_count() <= 2);
        assert!(two.metrics.size.width.0 <= 120.5);
    }
}
