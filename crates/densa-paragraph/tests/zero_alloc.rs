//! Proves drawing a laid-out paragraph every frame never touches the heap.

use densa_alloc::{is_active, measure, CountingAllocator};
use densa_draw::{Canvas, InstanceBuffer};
use densa_paragraph::{
    Color, Motion, ParagraphStyle, Point, Span, TextEditor, TextSelection, TextSystem,
};
use densa_text::{AtlasConfig, GlyphCache};
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

#[test]
fn drawing_a_paragraph_performs_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let mut system = TextSystem::new().unwrap();
    let text = "Crème brûlée, à la carte, wrapped across several lines of a narrow column.";
    let style = ParagraphStyle::new(14.0).with_max_width(Some(160.0));
    let spans = [Span::new(0.."Crème brûlée".len()).with_color(Color::linear(1.0, 0.5, 0.0, 1.0))];
    let paragraph = system.paragraph(text, &style, &spans).unwrap();
    assert!(paragraph.lines().len() > 2);

    let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
    let mut out = InstanceBuffer::with_capacity(paragraph.glyph_count());
    let draw = |cache: &mut GlyphCache, out: &mut InstanceBuffer| {
        let mut canvas = Canvas::begin(cache, out);
        for run in paragraph.runs() {
            canvas.draw_run(run, Point::new(10.0, 20.0));
        }
    };
    draw(&mut cache, &mut out); // first frame rasterizes
    cache.drain_updates().for_each(drop);

    let ((), stats) = measure(|| {
        for _ in 0..1_000 {
            draw(&mut cache, &mut out);
            black_box(out.as_slice());
        }
    });
    assert!(stats.is_zero(), "drawing allocated: {stats:?}");
    assert!(!out.is_empty());
    assert_eq!(out.dropped(), 0);
    assert_eq!(
        cache.drain_updates().count(),
        0,
        "warm frames upload nothing"
    );
}

#[test]
fn hit_testing_and_moving_selections_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let mut system = TextSystem::new().unwrap();
    let text = "Crème brûlée, à la carte, wrapped across several lines of a narrow column.";
    let style = ParagraphStyle::new(14.0).with_max_width(Some(160.0));
    let paragraph = system.paragraph(text, &style, &[]).unwrap();
    let motions = [
        Motion::Right,
        Motion::Down,
        Motion::WordRight,
        Motion::LineEnd,
        Motion::Up,
        Motion::Left,
        Motion::WordLeft,
        Motion::LineStart,
        Motion::TextEnd,
        Motion::TextStart,
    ];

    let ((), stats) = measure(|| {
        for step in 0..1_000u32 {
            let point = Point::new((step % 160) as f32, (step % 60) as f32);
            let mut selection = TextSelection::at_point(&paragraph, point);
            selection = selection.extend_to_point(&paragraph, point.offset(40.0, 20.0));
            for motion in motions {
                selection = selection.moved(&paragraph, motion, step % 2 == 0);
            }
            black_box(selection.caret_rect(&paragraph, 1.0));
            selection.for_each_rect(&paragraph, |rect| {
                black_box(rect);
            });
            black_box(TextSelection::word_at_point(&paragraph, point));
        }
    });
    assert!(stats.is_zero(), "selection work allocated: {stats:?}");
}

#[test]
fn moving_the_caret_and_querying_an_editor_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let mut system = TextSystem::new().unwrap();
    let text = "Crème brûlée, à la carte, wrapped across several lines of a narrow column.";
    let style = ParagraphStyle::new(14.0).with_max_width(Some(160.0));
    let mut editor = TextEditor::new(&mut system, text, style).unwrap();
    editor
        .set_composition(&mut system, "preedit", Some(7..7))
        .unwrap();

    let ((), stats) = measure(|| {
        black_box(editor.ime_area());
        editor.for_each_composition_rect(|rect| {
            black_box(rect);
        });
        for step in 0..1_000u32 {
            editor.move_caret(Motion::Left, step % 3 == 0);
            editor.move_caret(Motion::Down, false);
            let point = Point::new((step % 160) as f32, (step % 60) as f32);
            editor.set_selection(TextSelection::at_point(editor.paragraph(), point));
            black_box(editor.selected_text());
            black_box(editor.ime_area());
            black_box(editor.paragraph().runs().count());
        }
    });
    assert!(stats.is_zero(), "caret work allocated: {stats:?}");
}
