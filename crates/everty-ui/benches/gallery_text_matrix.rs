use std::{collections::BTreeSet, time::Instant};

use everty_ui::{DpiScale, FontWeight, TextStyle, TextSystem};

const PASSES: usize = 3;

struct Sample {
    page: &'static str,
    text: &'static str,
    size: f64,
    weight: FontWeight,
}

const SAMPLES: &[Sample] = &[
    Sample {
        page: "Typography",
        text: "12 px English Русский 0123456789",
        size: 12.0,
        weight: FontWeight::Regular,
    },
    Sample {
        page: "Typography",
        text: "… — × · « » № ° •",
        size: 13.0,
        weight: FontWeight::Regular,
    },
    Sample {
        page: "Typography",
        text: r"C:\Program Files\EvertyDesk",
        size: 14.0,
        weight: FontWeight::Regular,
    },
    Sample {
        page: "Typography",
        text: "/home/артур/Загрузки résumé.txt",
        size: 16.0,
        weight: FontWeight::Regular,
    },
    Sample {
        page: "Typography",
        text: "Файлы  Settings  gjpq  АРТУР  №123",
        size: 18.0,
        weight: FontWeight::Bold,
    },
    Sample {
        page: "Typography",
        text: "Regular / Bold / A\u{0308} / العربية",
        size: 24.0,
        weight: FontWeight::Bold,
    },
    Sample {
        page: "Buttons",
        text: "Activate semantic button",
        size: 14.0,
        weight: FontWeight::Regular,
    },
    Sample {
        page: "Buttons",
        text: "Длинная русская подпись кнопки",
        size: 14.0,
        weight: FontWeight::Medium,
    },
    Sample {
        page: "Buttons",
        text: "Disabled · Hover · Pressed · Focused",
        size: 14.0,
        weight: FontWeight::Regular,
    },
    Sample {
        page: "Layout",
        text: "Row  Column  Stack  Padding  Align",
        size: 14.0,
        weight: FontWeight::Regular,
    },
    Sample {
        page: "Layout",
        text: "Local pane  —  Remote pane",
        size: 16.0,
        weight: FontWeight::Medium,
    },
    Sample {
        page: "Layout",
        text: "Flexible content with minimum constraints",
        size: 18.0,
        weight: FontWeight::Regular,
    },
    Sample {
        page: "Clipping",
        text: "A long filename clipped at the viewport edge…",
        size: 14.0,
        weight: FontWeight::Regular,
    },
    Sample {
        page: "Clipping",
        text: "Отчёт_日本語_über_длинное_имя_файла.log",
        size: 16.0,
        weight: FontWeight::Regular,
    },
    Sample {
        page: "Clipping",
        text: "tooltip and breadcrumb ellipsis measurement",
        size: 18.0,
        weight: FontWeight::Regular,
    },
];

fn main() {
    let mut text = TextSystem::new();
    let pages = SAMPLES
        .iter()
        .map(|sample| sample.page)
        .collect::<BTreeSet<_>>();
    println!("EvertyUI representative Gallery text/cache matrix");
    println!("pages={pages:?} dpi=100,125,150,175,200% passes={PASSES}");
    println!(
        "cache_limit={} glyphs / {} bytes",
        text.cache_stats().capacity_glyphs,
        text.cache_stats().capacity_bytes
    );

    for percent in [100, 125, 150, 175, 200] {
        let scale = DpiScale::new(f64::from(percent) / 100.0).expect("valid DPI scale");
        let before = text.cache_stats();
        let started = Instant::now();
        let mut shaped_glyphs = 0usize;
        let mut missing_glyphs = 0usize;
        let mut rasterized_images = 0usize;

        for _ in 0..PASSES {
            for sample in SAMPLES {
                let style = TextStyle {
                    font_size: everty_ui::LogicalPx(sample.size),
                    line_height: everty_ui::LogicalPx(sample.size * 1.4),
                    weight: sample.weight,
                    ..TextStyle::default()
                };
                let layout = text.shape(sample.text, &style, None);
                for face in text.font_usage(&layout) {
                    shaped_glyphs += face.glyphs;
                    missing_glyphs += face.missing_glyphs;
                }
                rasterized_images += text.rasterize(&layout, (0, 0), scale).len();
            }
        }

        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let after = text.cache_stats();
        let hits = after.hits - before.hits;
        let misses = after.misses - before.misses;
        let requests = hits + misses;
        let hit_rate = if requests == 0 {
            0.0
        } else {
            hits as f64 * 100.0 / requests as f64
        };
        let page_count = pages.len();
        println!(
            "dpi={percent}% pages={page_count} samples={} passes={PASSES} elapsed={elapsed_ms:.2}ms shaped_glyphs={shaped_glyphs} rasterized_images={rasterized_images} missing_glyphs={missing_glyphs} cache_glyphs={}/{} cache_bytes={}/{} hits={hits} misses={misses} hit_rate={hit_rate:.1}% evictions={}",
            SAMPLES.len(),
            after.glyphs,
            after.capacity_glyphs,
            after.bytes,
            after.capacity_bytes,
            after.evictions,
        );
        assert_eq!(
            missing_glyphs, 0,
            "fallback left missing glyphs at {percent}% DPI"
        );
        assert!(after.glyphs <= after.capacity_glyphs);
        assert!(after.bytes <= after.capacity_bytes);
        assert!(
            after.hits > before.hits,
            "repeat pass should hit the glyph cache at {percent}% DPI"
        );
        assert!(rasterized_images > 0);
    }
}
