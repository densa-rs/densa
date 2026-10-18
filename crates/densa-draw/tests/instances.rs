//! Instance layout and the fixed-capacity buffer.

use densa_draw::{Color, Instance, InstanceBuffer};

#[test]
fn instance_layout_matches_the_shader() {
    assert_eq!(std::mem::size_of::<Instance>(), 48);
    let solid = Instance::solid([1.0, 2.0, 3.0, 4.0], Color::linear(0.1, 0.2, 0.3, 1.0));
    assert_eq!(
        solid.color,
        [0.1, 0.2, 0.3, 1.0],
        "the shader reads linear RGBA"
    );
    assert!(!solid.is_glyph());
    let glyph = Instance::glyph([0.0; 4], [0.0, 0.0, 0.5, 0.5], Color::WHITE);
    assert!(glyph.is_glyph());
    assert_eq!(bytemuck::bytes_of(&solid).len(), 48);
}

#[test]
fn buffer_fills_to_capacity_then_counts_drops() {
    let mut buf = InstanceBuffer::with_capacity(2);
    let item = Instance::solid([0.0; 4], Color::WHITE);
    assert!(buf.push(item));
    assert!(buf.push(item));
    assert!(!buf.push(item));
    assert_eq!(buf.len(), 2);
    assert_eq!(buf.capacity(), 2);
    assert_eq!(buf.dropped(), 1);
    assert_eq!(buf.as_slice().len(), 2);

    buf.clear();
    assert!(buf.is_empty());
    assert_eq!(buf.dropped(), 0, "clear starts a new frame");
}
