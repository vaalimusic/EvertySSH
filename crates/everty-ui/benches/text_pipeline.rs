use std::{hint::black_box, time::Instant};

use everty_ui::{DpiScale, TextStyle, TextSystem};

const WARMUP_ITERATIONS: usize = 20;
const SAMPLES: usize = 100;
const SAMPLE_TEXT: &str =
    "EvertyDesk — Файлы №123 • C:\\Program Files\\EvertyDesk · /home/артур/Загрузки";

fn main() {
    repeated_text_layout();
    glyph_cache_warm_and_uncached();
    glyph_cache_full_hit_path();
}

/// A resident cache close to its default 4096-glyph capacity: the cost of a
/// cache hit must not grow with the number of resident glyphs.
fn glyph_cache_full_hit_path() {
    let style = TextStyle::default();
    let mut text = TextSystem::with_cache_limits(4096, 64 * 1024 * 1024);
    let filler: String = (0x0400..0x0500u32)
        .chain(0x0020..0x017Fu32)
        .chain(0x2000..0x2400u32)
        .filter_map(char::from_u32)
        .filter(|ch| !ch.is_control() && !ch.is_whitespace())
        .collect();
    let mut resident = 0;
    for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
        let layout = text.shape(&filler, &style, None);
        black_box(text.rasterize(&layout, (0, 0), DpiScale::new(scale).unwrap()));
        resident = text.cache_stats().glyphs;
    }
    let sample = text.shape(SAMPLE_TEXT, &style, None);
    let scale = DpiScale::new(2.0).unwrap();
    black_box(text.rasterize(&sample, (0, 0), scale));
    let samples = measure(|| {
        black_box(text.rasterize(&sample, (0, 0), scale));
    });
    report("glyph cache hit path with a full cache", &samples);
    println!("  resident_glyphs={resident}");
}

fn repeated_text_layout() {
    let mut text = TextSystem::new();
    let style = TextStyle::default();
    for _ in 0..WARMUP_ITERATIONS {
        black_box(text.shape(SAMPLE_TEXT, &style, None));
    }

    let samples = measure(|| {
        black_box(text.shape(black_box(SAMPLE_TEXT), black_box(&style), None));
    });
    report("repeated advanced text shaping", &samples);
}

fn glyph_cache_warm_and_uncached() {
    let style = TextStyle::default();
    let mut warm = TextSystem::new();
    let warm_layout = warm.shape(SAMPLE_TEXT, &style, None);
    black_box(warm.rasterize(&warm_layout, (0, 0), DpiScale::ONE));
    for _ in 0..WARMUP_ITERATIONS {
        black_box(warm.rasterize(&warm_layout, (0, 0), DpiScale::ONE));
    }
    let warm_samples = measure(|| {
        black_box(warm.rasterize(&warm_layout, (0, 0), DpiScale::ONE));
    });
    let warm_stats = warm.cache_stats();
    let hit_rate = if warm_stats.hits + warm_stats.misses == 0 {
        0.0
    } else {
        warm_stats.hits as f64 / (warm_stats.hits + warm_stats.misses) as f64 * 100.0
    };
    report("warm bounded glyph cache rasterization", &warm_samples);
    println!(
        "  glyphs={} bytes={} hits={} misses={} hit_rate={hit_rate:.2}% evictions={}",
        warm_stats.glyphs,
        warm_stats.bytes,
        warm_stats.hits,
        warm_stats.misses,
        warm_stats.evictions
    );

    let mut uncached = TextSystem::with_cache_limits(0, 0);
    let uncached_layout = uncached.shape(SAMPLE_TEXT, &style, None);
    for _ in 0..WARMUP_ITERATIONS {
        black_box(uncached.rasterize(&uncached_layout, (0, 0), DpiScale::ONE));
    }
    let uncached_samples = measure(|| {
        black_box(uncached.rasterize(&uncached_layout, (0, 0), DpiScale::ONE));
    });
    let uncached_stats = uncached.cache_stats();
    report(
        "glyph rasterization with bounded cache disabled",
        &uncached_samples,
    );
    println!(
        "  retained_glyphs={} misses={} evictions={}",
        uncached_stats.glyphs, uncached_stats.misses, uncached_stats.evictions
    );
    let usage = warm.font_usage(&warm_layout);
    println!(
        "  shaped_glyphs={} missing_glyphs={}",
        usage.iter().map(|face| face.glyphs).sum::<usize>(),
        usage.iter().map(|face| face.missing_glyphs).sum::<usize>()
    );
}

fn measure(mut operation: impl FnMut()) -> Vec<f64> {
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        operation();
        samples.push(start.elapsed().as_secs_f64() * 1_000.0);
    }
    samples.sort_by(f64::total_cmp);
    samples
}

fn report(name: &str, sorted_samples: &[f64]) {
    let mean = sorted_samples.iter().sum::<f64>() / sorted_samples.len() as f64;
    println!(
        "{name}: mean={mean:.3} ms p50={:.3} ms p95={:.3} ms (warmup={WARMUP_ITERATIONS}, samples={SAMPLES})",
        percentile(sorted_samples, 0.50),
        percentile(sorted_samples, 0.95),
    );
}

fn percentile(sorted_samples: &[f64], percentile: f64) -> f64 {
    let index = ((sorted_samples.len() - 1) as f64 * percentile).round() as usize;
    sorted_samples[index]
}
