//! Series: the visible part by binary search, and the extent for scaling an axis.

use densa_chart::{
    candle_extent, candles_within, points_within, y_extent, Axis, Candle, Candles, Points,
};

#[test]
fn the_points_within_an_axis_are_found_ends_included() {
    let points: Vec<(f64, f64)> = (0..10).map(|x| (f64::from(x), 0.0)).collect();
    assert_eq!(points_within(&points[..], &Axis::number(2.0, 5.0)), 2..6);
    assert_eq!(points_within(&points[..], &Axis::number(2.5, 4.5)), 3..5);
    assert_eq!(points_within(&points[..], &Axis::number(-5.0, -1.0)), 0..0);
    assert_eq!(
        points_within(&points[..], &Axis::number(20.0, 30.0)),
        10..10
    );
    assert_eq!(
        points_within(&points[..], &Axis::number(-1.0, 100.0)),
        0..10
    );
    let empty: [(f64, f64); 0] = [];
    assert_eq!(points_within(&empty[..], &Axis::number(0.0, 1.0)), 0..0);
}

#[test]
fn repeated_x_values_are_all_found() {
    let points = [(1.0, 0.0), (2.0, 0.0), (2.0, 1.0), (2.0, 2.0), (3.0, 0.0)];
    assert_eq!(points_within(&points[..], &Axis::number(2.0, 2.5)), 1..4);
}

#[test]
fn side_by_side_slices_are_as_long_as_the_shorter() {
    let xs = [0.0, 1.0, 2.0, 3.0];
    let ys = [5.0, 6.0, 7.0];
    let points = (&xs[..], &ys[..]);
    assert_eq!(points.len(), 3);
    assert!(!points.is_empty());
    assert_eq!((points.x(2), points.y(2)), (2.0, 7.0));
    assert_eq!(points_within(&points, &Axis::number(1.0, 10.0)), 1..3);
}

#[test]
fn the_extent_skips_gaps() {
    let points = [
        (0.0, 3.0),
        (1.0, f64::NAN),
        (2.0, -1.0),
        (3.0, 8.0),
        (4.0, 100.0),
    ];
    assert_eq!(y_extent(&points[..], 0..4), Some((-1.0, 8.0)));
    assert_eq!(y_extent(&points[..], 1..2), None);
    assert_eq!(y_extent(&points[..], 0..0), None);
}

#[test]
fn candles_are_found_and_measured_from_low_to_high() {
    let candles = [
        (0.0, Candle::new(10.0, 12.0, 9.0, 11.0)),
        (60.0, Candle::new(11.0, 15.0, 10.5, 14.0)),
        (120.0, Candle::new(f64::NAN, 99.0, 0.0, 1.0)),
        (180.0, Candle::new(14.0, 14.5, 7.0, 8.0)),
    ];
    assert_eq!(candles.len(), 4);
    assert!(!candles.is_empty());
    assert_eq!(candles_within(&candles[..], &Axis::time(30.0, 200.0)), 1..4);
    assert_eq!(candle_extent(&candles[..], 0..4), Some((7.0, 15.0)));
    assert_eq!(
        candle_extent(&candles[..], 2..3),
        None,
        "a broken candle counts not"
    );
    assert_eq!(candles.x(1), 60.0);
    assert_eq!(candles.candle(3).close, 8.0);
    assert!(!Candle::new(1.0, f64::INFINITY, 0.0, 1.0).is_finite());
}

#[test]
fn an_extent_reads_only_what_there_is() {
    let points = [(0.0, 3.0), (1.0, 4.0)];
    assert_eq!(y_extent(&points[..], 1..10), Some((4.0, 4.0)));
    assert_eq!(y_extent(&points[..], 5..10), None);
    let candles = [(0.0, Candle::new(1.0, 2.0, 0.5, 1.5))];
    assert_eq!(candle_extent(&candles[..], 0..7), Some((0.5, 2.0)));
}
