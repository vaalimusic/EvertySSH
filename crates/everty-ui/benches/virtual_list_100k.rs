use std::{hint::black_box, time::Instant};

use everty_ui::{LogicalPoint, LogicalPx, LogicalRect, LogicalSize, ScrollState, VirtualList};

const ITEM_COUNT: usize = 100_000;
const VIEWPORT: LogicalSize = LogicalSize::new(1280.0, 820.0);
const WARMUP_ITERATIONS: usize = 20;
const SAMPLES: usize = 100;

fn main() {
    let list = VirtualList::new(ITEM_COUNT, LogicalPx(24.0));
    let mut scroll = ScrollState::default();
    scroll.set_metrics(VIEWPORT, list.content_size(LogicalPx(VIEWPORT.width.0)));
    scroll.set_offset(LogicalPoint::new(0.0, 1_200_000.0));
    let viewport = LogicalRect::from_xywh(0.0, 0.0, VIEWPORT.width.0, VIEWPORT.height.0);
    let visible_count = list.visible_range(&scroll).len();

    for _ in 0..WARMUP_ITERATIONS {
        run_frame(&list, &scroll, viewport);
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        run_frame(&list, &scroll, viewport);
        samples.push(start.elapsed().as_secs_f64() * 1_000.0);
    }
    samples.sort_by(f64::total_cmp);

    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    println!("EvertyUI virtual-list microbenchmark");
    println!(
        "target={}-{} profile=release",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!(
        "model={ITEM_COUNT} fixed-height rows viewport={:.0}x{:.0} logical visible_plus_overscan={visible_count}",
        VIEWPORT.width.0,
        VIEWPORT.height.0
    );
    println!("operation=visible_range + materialize visible row rectangles");
    println!("warmup={WARMUP_ITERATIONS} samples={SAMPLES}");
    println!(
        "mean={mean:.6} ms p50={:.6} ms p95={:.6} ms",
        percentile(&samples, 0.50),
        percentile(&samples, 0.95)
    );
}

fn run_frame(list: &VirtualList, scroll: &ScrollState, viewport: LogicalRect) {
    let visible = list.visible_range(scroll);
    let rows = visible
        .filter_map(|index| list.item_rect(index, viewport, scroll))
        .collect::<Vec<_>>();
    black_box(rows);
}

fn percentile(sorted_samples: &[f64], percentile: f64) -> f64 {
    let index = ((sorted_samples.len() - 1) as f64 * percentile).round() as usize;
    sorted_samples[index]
}
