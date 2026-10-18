//! Color: construction, sRGB conversion, alpha, array round trips.

use densa_core::Color;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn linear_components_are_stored_as_given() {
    let c = Color::linear(0.1, 0.2, 0.3, 0.4);
    assert_eq!((c.r, c.g, c.b, c.a), (0.1, 0.2, 0.3, 0.4));
}

#[test]
fn srgb_hex_converts_to_linear_and_is_opaque() {
    assert_eq!(Color::srgb(0x000000), Color::BLACK);
    assert_eq!(Color::srgb(0xFFFFFF), Color::WHITE);
    let c = Color::srgb(0x800000);
    assert!(close(c.r, 0.2158), "{}", c.r);
    assert_eq!((c.g, c.b, c.a), (0.0, 0.0, 1.0));
}

#[test]
fn srgb_round_trips_through_8_bit_channels() {
    for hex in [0x000000, 0x0D1117, 0x58A6FF, 0xC9D1D9, 0xF85149, 0xFFFFFF] {
        assert_eq!(Color::srgb(hex).to_srgb_hex(), hex, "{hex:06X}");
    }
}

#[test]
fn srgb8_matches_srgb_hex() {
    assert_eq!(Color::srgb8(0x58, 0xA6, 0xFF), Color::srgb(0x58A6FF));
}

#[test]
fn with_alpha_replaces_only_alpha() {
    let c = Color::srgb(0x3FB950).with_alpha(0.25);
    assert_eq!(c.a, 0.25);
    assert_eq!(c.with_alpha(1.0), Color::srgb(0x3FB950));
}

#[test]
fn alpha_is_clamped_to_the_unit_range() {
    assert_eq!(Color::WHITE.with_alpha(2.0).a, 1.0);
    assert_eq!(Color::WHITE.with_alpha(-1.0).a, 0.0);
    assert_eq!(
        Color::WHITE.with_alpha(f32::NAN).a,
        0.0,
        "NaN is transparent, not poison"
    );
}

#[test]
fn converts_to_and_from_arrays() {
    let c = Color::linear(0.1, 0.2, 0.3, 0.4);
    assert_eq!(c.to_array(), [0.1, 0.2, 0.3, 0.4]);
    assert_eq!(Color::from([0.1, 0.2, 0.3, 0.4]), c);
    let a: [f32; 4] = c.into();
    assert_eq!(a, [0.1, 0.2, 0.3, 0.4]);
}

#[test]
fn transparent_is_the_default() {
    assert_eq!(Color::default(), Color::TRANSPARENT);
    assert_eq!(Color::TRANSPARENT.a, 0.0);
}

#[test]
fn layout_is_four_contiguous_floats() {
    assert_eq!(std::mem::size_of::<Color>(), 16);
    assert_eq!(std::mem::align_of::<Color>(), 4);
}
