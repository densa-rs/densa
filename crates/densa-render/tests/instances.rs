use densa_render::{srgb, Instance, InstanceBuffer};

#[test]
fn instance_layout_matches_the_shader() {
    assert_eq!(std::mem::size_of::<Instance>(), 48);
    let solid = Instance::solid([1.0, 2.0, 3.0, 4.0], [0.1, 0.2, 0.3, 1.0]);
    assert!(!solid.is_glyph());
    let glyph = Instance::glyph([0.0; 4], [0.0, 0.0, 0.5, 0.5], [1.0; 4]);
    assert!(glyph.is_glyph());
    assert_eq!(bytemuck::bytes_of(&solid).len(), 48);
}

#[test]
fn buffer_fills_to_capacity_then_counts_drops() {
    let mut buf = InstanceBuffer::with_capacity(2);
    let item = Instance::solid([0.0; 4], [1.0; 4]);
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

#[test]
fn srgb_to_linear_conversion() {
    assert_eq!(srgb(0x000000, 1.0), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(srgb(0xFFFFFF, 0.5), [1.0, 1.0, 1.0, 0.5]);
    let [r, g, b, _] = srgb(0x800000, 1.0);
    assert!((r - 0.2158).abs() < 1e-3, "{r}");
    assert_eq!((g, b), (0.0, 0.0));
}
