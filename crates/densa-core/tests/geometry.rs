//! Point and Rect: construction, edges, exact tiling, emptiness, NaN.

use densa_core::{Point, Rect};

#[test]
fn points_offset_and_convert() {
    let p = Point::new(1.0, 2.0);
    assert_eq!(p.offset(3.0, -1.0), Point::new(4.0, 1.0));
    assert_eq!(Point::from([1.0, 2.0]), p);
    assert_eq!(p.to_array(), [1.0, 2.0]);
    assert_eq!(Point::ORIGIN, Point::new(0.0, 0.0));
}

#[test]
fn rects_expose_their_edges_and_size() {
    let r = Rect::new(10.0, 20.0, 30.0, 40.0);
    assert_eq!(
        (r.left(), r.top(), r.right(), r.bottom()),
        (10.0, 20.0, 40.0, 60.0)
    );
    assert_eq!((r.width(), r.height()), (30.0, 40.0));
    assert_eq!(r.to_xywh(), [10.0, 20.0, 30.0, 40.0]);
    assert_eq!(Rect::from_edges(10.0, 20.0, 40.0, 60.0), r);
}

#[test]
fn rects_from_reversed_edges_are_normalized() {
    assert_eq!(
        Rect::from_edges(40.0, 60.0, 10.0, 20.0),
        Rect::from_edges(10.0, 20.0, 40.0, 60.0)
    );
}

#[test]
fn containment_is_half_open() {
    let r = Rect::new(0.0, 0.0, 10.0, 10.0);
    assert!(r.contains(Point::new(0.0, 0.0)));
    assert!(r.contains(Point::new(9.99, 9.99)));
    assert!(!r.contains(Point::new(10.0, 5.0)), "right edge is outside");
    assert!(!r.contains(Point::new(5.0, -0.01)));
}

#[test]
fn rects_sharing_an_edge_tile_exactly_even_after_translation() {
    // 0.1 + (0.7 - 0.1) is not exactly 0.7 in f32: origin-plus-size storage would leave a
    // seam or an overlap here.
    let a = Rect::from_edges(0.1, 0.0, 0.7, 1.0);
    let b = Rect::from_edges(0.7, 0.0, 1.3, 1.0);
    for shift in [Point::ORIGIN, Point::new(0.3, 0.0), Point::new(1e4, 7.0)] {
        let (a, b) = (a.translate(shift), b.translate(shift));
        let edge = Point::new(a.right(), 0.5 + shift.y);
        assert_eq!(a.right(), b.left());
        assert!(!a.contains(edge) && b.contains(edge), "shift {shift:?}");
    }
}

#[test]
fn degenerate_rects_are_empty_and_contain_nothing() {
    let zero = Rect::new(5.0, 5.0, 0.0, 10.0);
    assert!(zero.is_empty());
    assert!(!zero.contains(Point::new(5.0, 6.0)));
    assert!(!Rect::new(0.0, 0.0, 1.0, 1.0).is_empty());
}

#[test]
fn nan_edges_make_an_empty_rect_instead_of_a_different_one() {
    let r = Rect::from_edges(f32::NAN, 0.0, 5.0, 1.0);
    assert!(r.is_empty());
    assert!(!r.contains(Point::new(4.0, 0.5)));
    assert!(!Rect::new(0.0, 0.0, 1.0, 1.0).contains(Point::new(f32::NAN, 0.5)));
}

#[test]
fn rects_translate() {
    let r = Rect::new(1.0, 2.0, 3.0, 4.0);
    assert_eq!(
        r.translate(Point::new(10.0, 20.0)),
        Rect::new(11.0, 22.0, 3.0, 4.0)
    );
}
