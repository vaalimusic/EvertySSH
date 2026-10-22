use std::{hint::black_box, time::Instant};

use everty_ui::{layout, Constraints, LayoutId, LayoutNode, LogicalRect, LogicalSize};

const NODE_COUNT: usize = 10_000;
const WARMUP_ITERATIONS: usize = 20;
const SAMPLES: usize = 100;

fn main() {
    let root = LayoutNode::stack(
        LayoutId(0),
        (1..=NODE_COUNT)
            .map(|id| LayoutNode::intrinsic(LayoutId(id as u64), LogicalSize::new(96.0, 24.0)))
            .collect(),
    );
    let bounds = LogicalRect::from_xywh(0.0, 0.0, 1280.0, 820.0);

    for _ in 0..WARMUP_ITERATIONS {
        run_layout(&root, bounds);
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        run_layout(&root, bounds);
        samples.push(start.elapsed().as_secs_f64() * 1_000.0);
    }
    samples.sort_by(f64::total_cmp);

    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let p50 = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    println!("EvertyUI layout microbenchmark");
    println!(
        "target={}-{} profile=release",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("shape=Stack with {NODE_COUNT} intrinsic leaves (tree built outside timed region)");
    println!("operation=measure + layout + materialize LayoutResult boxes");
    println!("warmup={WARMUP_ITERATIONS} samples={SAMPLES}");
    println!("mean={mean:.3} ms p50={p50:.3} ms p95={p95:.3} ms");
}

fn run_layout(root: &LayoutNode, bounds: LogicalRect) {
    let measured = root.measure(Constraints::UNBOUNDED);
    let result = layout(root, bounds);
    black_box((measured, result.iter().count()));
}

fn percentile(sorted_samples: &[f64], percentile: f64) -> f64 {
    let index = ((sorted_samples.len() - 1) as f64 * percentile).round() as usize;
    sorted_samples[index]
}
