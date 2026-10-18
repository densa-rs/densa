//! Axes: ordering and widening, zoom and pan, round ticks and their labels.

use densa_chart::{Axis, AxisKind, Ticks};
use densa_fmt::{FmtBuf, FmtError};

/// 2026-10-19 09:00:00 UTC.
const MORNING: f64 = 1_792_400_400.0;

fn labels(axis: Axis, most: usize) -> Vec<String> {
    let ticks = axis.ticks(most);
    ticks
        .clone()
        .map(|value| {
            let mut text = FmtBuf::<32>::new();
            ticks.write_label(value, &mut text).unwrap();
            text.as_str().to_owned()
        })
        .collect()
}

#[test]
fn numbers_tick_at_one_two_or_five_times_a_power_of_ten() {
    let ticks: Vec<f64> = Axis::number(0.0, 10.0).ticks(5).collect();
    assert_eq!(ticks, [0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
    assert_eq!(Axis::number(0.0, 10.0).ticks(5).step(), 2.0);
    assert_eq!(Axis::number(0.0, 100.0).ticks(4).step(), 50.0);
    assert_eq!(Axis::number(0.0, 1.0).ticks(3).step(), 0.5);
    assert_eq!(Axis::number(0.0, 1.0).ticks(1).step(), 1.0);
    assert_eq!(Axis::number(0.0, 7.0).ticks(1).step(), 10.0);
}

#[test]
fn ticks_stay_within_the_axis() {
    assert_eq!(labels(Axis::number(98.7, 101.3), 5), ["99", "100", "101"]);
    assert_eq!(labels(Axis::number(-3.0, 3.0), 3), ["-2", "0", "2"]);
}

#[test]
fn labels_have_the_decimals_the_step_needs() {
    assert_eq!(
        labels(Axis::number(0.0, 0.2), 4),
        ["0.00", "0.05", "0.10", "0.15", "0.20"]
    );
    let tenths = labels(Axis::number(0.0, 1.0), 10);
    assert_eq!(tenths.len(), 11);
    assert_eq!(tenths[3], "0.3", "a power of ten needs no extra digit");
    assert_eq!(labels(Axis::number(0.0, 2_000.0), 2), ["0", "1000", "2000"]);
}

#[test]
fn values_are_put_in_order_and_a_single_value_is_widened() {
    let axis = Axis::number(5.0, -5.0);
    assert_eq!((axis.start(), axis.end()), (-5.0, 5.0));
    let flat = Axis::number(100.0, 100.0);
    assert_eq!((flat.start(), flat.end()), (99.0, 101.0));
    let zero = Axis::number(0.0, 0.0);
    assert_eq!((zero.start(), zero.end()), (-1.0, 1.0));
    let tiny = Axis::number(f64::from_bits(1), f64::from_bits(1));
    assert!(tiny.span() > 0.0, "a subnormal value still gets a span");
}

#[test]
fn values_that_are_not_finite_give_zero_to_one() {
    for (start, end) in [
        (f64::NAN, 3.0),
        (0.0, f64::INFINITY),
        (f64::NEG_INFINITY, 1.0),
    ] {
        let axis = Axis::number(start, end);
        assert_eq!((axis.start(), axis.end()), (0.0, 1.0));
    }
    let widest = Axis::number(-f64::MAX, f64::MAX);
    assert!(widest.span().is_finite() && widest.span() > 0.0);
}

#[test]
fn zooming_keeps_the_anchor_and_panning_moves_both_ends() {
    let axis = Axis::number(0.0, 10.0);
    let zoomed = axis.zoomed(0.5, 5.0);
    assert_eq!((zoomed.start(), zoomed.end()), (2.5, 7.5));
    let edge = axis.zoomed(2.0, 0.0);
    assert_eq!((edge.start(), edge.end()), (0.0, 20.0));
    for factor in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(axis.zoomed(factor, 5.0), axis, "factor {factor}");
    }
    assert_eq!(axis.zoomed(0.5, f64::NAN), axis);
    let moved = axis.shifted(3.0);
    assert_eq!((moved.start(), moved.end()), (3.0, 13.0));
    let padded = axis.padded(0.1);
    assert_eq!((padded.start(), padded.end()), (-1.0, 11.0));
    assert_eq!(axis.padded(-0.5), axis, "negative padding is none");
    assert_eq!(zoomed.kind(), AxisKind::Number);
    assert_eq!(Axis::time(0.0, 1.0).shifted(1.0).kind(), AxisKind::Time);
}

#[test]
fn fractions_run_from_start_to_end() {
    let axis = Axis::number(10.0, 20.0);
    assert_eq!(axis.fraction(10.0), 0.0);
    assert_eq!(axis.fraction(15.0), 0.5);
    assert_eq!(axis.fraction(30.0), 2.0);
    assert_eq!(axis.value_at(0.25), 12.5);
}

#[test]
fn an_hour_ticks_every_ten_minutes_as_hours_and_minutes() {
    let hour = Axis::time(MORNING, MORNING + 3_600.0);
    assert_eq!(hour.ticks(6).step(), 600.0);
    assert_eq!(
        labels(hour, 6),
        ["09:00", "09:10", "09:20", "09:30", "09:40", "09:50", "10:00"]
    );
    let day = Axis::time(MORNING, MORNING + 86_400.0);
    assert_eq!(day.ticks(5).step(), 21_600.0, "six hours");
    assert_eq!(labels(day, 5)[0], "12:00");
}

#[test]
fn seconds_and_fractions_show_when_the_step_is_that_fine() {
    let half_minute = Axis::time(MORNING, MORNING + 30.0);
    assert_eq!(half_minute.ticks(6).step(), 5.0);
    assert_eq!(labels(half_minute, 6)[1], "09:00:05");
    assert_eq!(Axis::time(MORNING, MORNING + 60.0).ticks(4).step(), 15.0);
    let two_seconds = Axis::time(MORNING, MORNING + 2.0);
    assert_eq!(
        labels(two_seconds, 4),
        [
            "09:00:00.0",
            "09:00:00.5",
            "09:00:01.0",
            "09:00:01.5",
            "09:00:02.0"
        ]
    );
    let millis = Axis::time(MORNING, MORNING + 0.01);
    assert_eq!(labels(millis, 2)[1], "09:00:00.005");
}

#[test]
fn days_are_labelled_as_dates() {
    let ten_days = Axis::time(MORNING, MORNING + 10.0 * 86_400.0);
    assert_eq!(ten_days.ticks(5).step(), 2.0 * 86_400.0);
    assert_eq!(
        labels(ten_days, 5),
        [
            "2026-10-20",
            "2026-10-22",
            "2026-10-24",
            "2026-10-26",
            "2026-10-28"
        ]
    );
    let before_year_zero = Axis::time(-62_500_000_000.0, -62_200_000_000.0);
    assert!(labels(before_year_zero, 3)[0].starts_with("-000"));
}

#[test]
fn a_label_that_does_not_fit_leaves_the_text_as_it_was() {
    let ticks: Ticks = Axis::time(MORNING, MORNING + 10.0 * 86_400.0).ticks(5);
    let mut text = FmtBuf::<6>::new();
    text.push_str("ab").unwrap();
    assert_eq!(
        ticks.write_label(MORNING, &mut text),
        Err(FmtError::Overflow)
    );
    assert_eq!(text.as_str(), "ab");
    let mut numbers = FmtBuf::<3>::new();
    let number_ticks = Axis::number(0.0, 1.0).ticks(10);
    assert_eq!(
        number_ticks.write_label(12.5, &mut numbers),
        Err(FmtError::Overflow)
    );
    assert_eq!(
        number_ticks.write_label(f64::NAN, &mut numbers),
        Err(FmtError::NotFinite)
    );
    assert_eq!(numbers.as_str(), "");
}

#[test]
fn ticks_too_far_from_zero_to_tell_apart_are_left_out() {
    let far = Axis::number(1e19, 1e19 + 4_096.0);
    assert_eq!(far.ticks(5).count(), 0);
}

#[test]
fn the_tick_count_is_known_up_front() {
    let ticks = Axis::number(0.0, 10.0).ticks(5);
    assert_eq!(ticks.size_hint(), (6, Some(6)));
    let mut ticks = ticks;
    ticks.next();
    assert_eq!(ticks.len(), 5);
}

#[test]
fn room_for_no_ticks_still_gives_the_coarsest() {
    let ticks: Vec<f64> = Axis::number(0.0, 10.0).ticks(0).collect();
    assert_eq!(ticks, [0.0, 10.0]);
}

#[test]
fn a_step_of_exactly_one_two_or_five_is_kept() {
    assert_eq!(Axis::number(0.0, 10.0).ticks(2).step(), 5.0);
    assert_eq!(Axis::number(0.0, 10.0).ticks(10).step(), 1.0);
}

#[test]
fn a_step_of_a_day_writes_dates_and_of_a_minute_no_seconds() {
    let week = Axis::time(MORNING, MORNING + 5.0 * 86_400.0);
    assert_eq!(week.ticks(5).step(), 86_400.0);
    assert_eq!(labels(week, 5)[0], "2026-10-20");
    let hour = Axis::time(MORNING, MORNING + 3_600.0);
    assert_eq!(hour.ticks(60).step(), 60.0);
    assert_eq!(labels(hour, 60)[1], "09:01");
}

#[test]
fn a_time_too_far_out_for_its_step_has_no_label() {
    let far = Axis::time(1e19, 1e19 + 86_400.0);
    let ticks = far.ticks(10);
    let value = ticks.clone().next().unwrap();
    let mut text = FmtBuf::<32>::new();
    assert_eq!(ticks.write_label(value, &mut text), Err(FmtError::Overflow));
    assert_eq!(text.as_str(), "");
}

#[test]
fn the_widest_axis_has_no_tick_it_cannot_place() {
    let widest = Axis::number(-f64::MAX, f64::MAX);
    for most in [0, 1, 2, 10] {
        assert!(widest.ticks(most).all(f64::is_finite), "{most}");
    }
}

#[test]
fn a_value_that_rounds_to_zero_has_no_sign() {
    let ticks = Axis::number(0.0, 1.0).ticks(10);
    let mut text = FmtBuf::<16>::new();
    ticks.write_label(-0.01, &mut text).unwrap();
    assert_eq!(text.as_str(), "0.0");
}

#[test]
fn huge_times_are_labelled_without_panicking() {
    for (start, end) in [
        (0.0, 1e300),
        (-1e30, 1e30),
        (-f64::MAX, f64::MAX),
        (1e24, 1e24 + 1e9),
    ] {
        let axis = Axis::time(start, end);
        for most in [1, 5, 50] {
            let ticks = axis.ticks(most);
            for value in ticks.clone() {
                let mut text = FmtBuf::<32>::new();
                let _ = ticks.write_label(value, &mut text);
            }
        }
    }
}
