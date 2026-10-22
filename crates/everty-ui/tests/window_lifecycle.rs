//! Model-level multi-window lifecycle stress. No native window is created:
//! each model window owns exactly what a native one owns in the toolkit
//! (UiWindow state/input and a private raster target) while the font system and
//! its bounded glyph cache are shared. The native counterpart is the Gallery's
//! `--window-lifecycle-cycles=N` runtime fixture.

use std::collections::HashSet;

use everty_ui::{
    Button, DpiScale, HitRegion, InputEvent, Label, LogicalRect, MouseButton, PaintBuilder,
    PaintLayer, PhysicalSize, Renderer, Rgba8Renderer, TextSystem, Theme, WidgetEvent, WidgetId,
    WindowId, WindowRegistry,
};

const SCALES: [f64; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];

struct ModelWindow {
    raster: Rgba8Renderer,
    button: Button,
    rect: LogicalRect,
}

impl ModelWindow {
    fn new(id: WindowId) -> Self {
        Self {
            raster: Rgba8Renderer::new(),
            button: Button::new(WidgetId(100 + id.0), "Открыть файл"),
            rect: LogicalRect::from_xywh(20.0, 20.0, 160.0, 36.0),
        }
    }

    fn hit(&self) -> HitRegion {
        self.button.hit_region(self.rect, PaintLayer::Content, 0)
    }

    fn paint(&mut self, registry: &WindowRegistry, id: WindowId, text: &mut TextSystem) {
        let window = registry.get(id).expect("window is open");
        let theme = Theme::EVERTY_LIGHT;
        let viewport = window.state.logical_size;
        let mut builder = PaintBuilder::with_viewport(LogicalRect::from_xywh(
            0.0,
            0.0,
            viewport.width.0,
            viewport.height.0,
        ));
        builder.set_layer(PaintLayer::Content);
        self.button.paint(
            self.rect,
            self.button.state(&window.input),
            text,
            theme,
            &mut builder,
        );
        Label::new(WidgetId(1), "Файл — report_финал.docx").paint(
            LogicalRect::from_xywh(20.0, 80.0, 220.0, 24.0),
            text,
            theme,
            &mut builder,
        );
        self.raster
            .render(
                &builder.finish(),
                text,
                window.state.scale,
                window.state.physical_size,
            )
            .unwrap();
    }
}

fn physical(size: (f64, f64), factor: f64) -> PhysicalSize {
    PhysicalSize::new((size.0 * factor).round(), (size.1 * factor).round())
}

#[test]
fn create_render_interact_destroy_100_times_leaves_nothing_behind() {
    let mut text = TextSystem::with_cache_limits(256, 1024 * 1024);
    let mut registry = WindowRegistry::default();
    let mut seen_ids = HashSet::new();
    let mut activations = 0;

    for cycle in 0..100usize {
        let factor = SCALES[cycle % SCALES.len()];
        let scale = DpiScale::new(factor).unwrap();
        let id = registry.open(physical((400.0, 200.0), factor), scale);
        assert!(seen_ids.insert(id), "window id {id:?} was reused");
        let mut model = ModelWindow::new(id);
        registry
            .get_mut(id)
            .unwrap()
            .input
            .focus
            .set_tab_order([(model.button.id, true, true)]);
        model.paint(&registry, id, &mut text);

        // Interact: hover, press, release inside -> one semantic activation.
        let point = everty_ui::LogicalPoint::new(40.0, 30.0);
        for event in [
            InputEvent::PointerMoved(point),
            InputEvent::PointerDown(MouseButton::Primary),
            InputEvent::PointerUp(MouseButton::Primary),
        ] {
            let routed = registry.dispatch(id, event, &[model.hit()]).unwrap();
            let window = registry.get_mut(id).unwrap();
            if model
                .button
                .handle(routed.target, &routed.event, &mut window.input, model.rect)
                == Some(WidgetEvent::Activated(model.button.id))
            {
                activations += 1;
            }
        }
        model.paint(&registry, id, &mut text);

        // Mid-life DPI + size change, then render again at the new metrics.
        let next = SCALES[(cycle + 2) % SCALES.len()];
        registry
            .get_mut(id)
            .unwrap()
            .update_metrics(physical((420.0, 220.0), next), DpiScale::new(next).unwrap());
        model.paint(&registry, id, &mut text);
        let expected = physical((420.0, 220.0), next);
        assert_eq!(
            model.raster.pixels().len(),
            (expected.width.0 * expected.height.0 * 4.0) as usize,
            "raster target follows its own window's physical size"
        );

        // Destroy: state is returned and gone; late events find no window.
        let closed = registry.close(id).expect("window was open");
        assert!(closed.input.pointer_capture.is_none());
        assert!(registry
            .dispatch(id, InputEvent::PointerMoved(point), &[])
            .is_none());
        assert_eq!(registry.len(), 0);
    }

    assert_eq!(activations, 100);
    assert_eq!(seen_ids.len(), 100);
    assert!(registry.is_empty());
    let stats = text.cache_stats();
    assert!(stats.glyphs <= stats.capacity_glyphs, "{stats:?}");
    assert!(stats.bytes <= stats.capacity_bytes, "{stats:?}");
    assert!(
        stats.hits > stats.misses,
        "shared glyph cache is reused: {stats:?}"
    );
}

#[test]
fn concurrent_windows_keep_independent_scale_focus_capture_and_redraw_state() {
    let mut registry = WindowRegistry::default();
    let a = registry.open(physical((400.0, 200.0), 1.25), DpiScale::new(1.25).unwrap());
    let b = registry.open(physical((400.0, 200.0), 2.0), DpiScale::new(2.0).unwrap());
    let button = Button::new(WidgetId(7), "Action");
    let rect = LogicalRect::from_xywh(0.0, 0.0, 100.0, 36.0);
    let hit = button.hit_region(rect, PaintLayer::Content, 0);
    for id in [a, b] {
        registry
            .get_mut(id)
            .unwrap()
            .input
            .focus
            .set_tab_order([(button.id, true, true)]);
    }
    registry.get_mut(a).unwrap().state.take_redraw_request();
    registry.get_mut(b).unwrap().state.take_redraw_request();

    registry
        .dispatch(
            a,
            InputEvent::PointerMoved(everty_ui::LogicalPoint::new(5.0, 5.0)),
            &[hit],
        )
        .unwrap();
    registry
        .dispatch(a, InputEvent::PointerDown(MouseButton::Primary), &[hit])
        .unwrap();
    registry
        .get_mut(a)
        .unwrap()
        .input
        .capture_pointer(button.id);

    let (wa, wb) = (registry.get(a).unwrap(), registry.get(b).unwrap());
    assert_eq!(wa.input.focused(), Some(button.id));
    assert_eq!(wa.input.pointer_capture, Some(button.id));
    assert_eq!(wb.input.focused(), None);
    assert_eq!(wb.input.pointer_capture, None);
    assert_eq!(wb.input.hovered, None);
    assert!(wa.state.needs_redraw && !wb.state.needs_redraw);
    assert_ne!(wa.state.scale, wb.state.scale);
    assert_eq!(wa.state.logical_size, wb.state.logical_size);

    // Focus loss in one window does not disturb the other.
    registry
        .dispatch(a, InputEvent::WindowFocused(false), &[])
        .unwrap();
    assert_eq!(registry.get(a).unwrap().input.pointer_capture, None);
    registry
        .get_mut(b)
        .unwrap()
        .input
        .capture_pointer(button.id);
    registry
        .dispatch(a, InputEvent::WindowFocused(false), &[])
        .unwrap();
    assert_eq!(
        registry.get(b).unwrap().input.pointer_capture,
        Some(button.id)
    );
}
