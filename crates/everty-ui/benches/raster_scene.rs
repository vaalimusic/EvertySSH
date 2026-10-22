use std::{hint::black_box, sync::Arc, time::Instant};

use everty_ui::{
    Color, DpiScale, LogicalPoint, LogicalRect, PaintBuilder, PaintLayer, PhysicalSize, Renderer,
    Rgba8Renderer, TextStyle, TextSystem,
};

const WARMUP_ITERATIONS: usize = 5;
const SAMPLES: usize = 30;

fn main() {
    let mut text = TextSystem::new();
    for factor in [1.0, 1.25, 2.0] {
        let scale = DpiScale::new(factor).unwrap();
        let paint = gallery_like_scene(&mut text);
        let size = PhysicalSize::new((1280.0 * factor).round(), (820.0 * factor).round());
        let mut renderer = Rgba8Renderer::new();
        for _ in 0..WARMUP_ITERATIONS {
            renderer.render(&paint, &mut text, scale, size).unwrap();
        }
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let start = Instant::now();
            renderer
                .render(black_box(&paint), &mut text, scale, size)
                .unwrap();
            samples.push(start.elapsed().as_secs_f64() * 1_000.0);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "rgba8 raster 1280x820 logical @{factor}x ({} commands): mean={:.3} ms p50={:.3} ms p95={:.3} ms",
            paint.len(),
            samples.iter().sum::<f64>() / samples.len() as f64,
            samples[samples.len() / 2],
            samples[(samples.len() as f64 * 0.95) as usize - 1],
        );
    }
}

/// Window background, sidebar, a large rounded card with a border, striped
/// rows and text: the shape of a typical Gallery page.
fn gallery_like_scene(text: &mut TextSystem) -> everty_ui::PaintList {
    let viewport = LogicalRect::from_xywh(0.0, 0.0, 1280.0, 820.0);
    let mut builder = PaintBuilder::with_viewport(viewport);
    builder.set_layer(PaintLayer::Background);
    builder.fill_rect(viewport, Color::rgba(243, 245, 248, 255), 0.0);
    builder.set_layer(PaintLayer::Content);
    let card = LogicalRect::from_xywh(236.0, 10.0, 1032.0, 800.0);
    builder.fill_rect(card, Color::rgba(255, 255, 255, 255), 8.0);
    builder.stroke_rect(card, Color::rgba(206, 212, 222, 255), 1.0, 8.0);
    let style = TextStyle::default();
    for row in 0..24 {
        let y = 70.0 + row as f64 * 28.0;
        if row % 2 == 0 {
            builder.fill_rect(
                LogicalRect::from_xywh(260.0, y, 980.0, 28.0),
                Color::rgba(232, 236, 242, 255),
                3.0,
            );
        }
        let layout = Arc::new(text.shape(
            &format!("item-{row:06}   Файл записи   2.4 MB — report.docx"),
            &style,
            None,
        ));
        builder.text(
            LogicalPoint::new(268.0, y + 4.0),
            layout,
            Color::rgba(28, 32, 40, 255),
        );
    }
    builder.fill_rect(
        LogicalRect::from_xywh(300.0, 200.0, 400.0, 300.0),
        Color::rgba(62, 139, 218, 120),
        6.0,
    );
    builder.finish()
}
