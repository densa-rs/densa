//! Time windows: which rows a window keeps, the spans it accepts, and a model-based run of
//! random upserts and deletes checked against a plain recomputation after every step.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use arrow_array::types::Int64Type;
use arrow_array::{
    Array, ArrayRef, Date32Array, Decimal128Array, Float64Array, Int64Array, ListArray,
    RecordBatch, StringArray, TimestampMillisecondArray, TimestampSecondArray, UInt64Array,
};
use densa_data::{Aggregate, DataError, GroupSpec, LiveTable, Order, Scalar, ViewSpec, Window};

/// A batch of `id` (Int64 keys) and `time` (the given array).
fn batch(ids: &[i64], time: ArrayRef) -> RecordBatch {
    RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from(ids.to_vec())) as ArrayRef),
        ("time", time),
    ])
    .unwrap()
}

fn millis(ids: &[i64], times: &[Option<i64>]) -> RecordBatch {
    batch(
        ids,
        Arc::new(TimestampMillisecondArray::from(times.to_vec())),
    )
}

fn floats(ids: &[i64], times: &[Option<f64>]) -> RecordBatch {
    batch(ids, Arc::new(Float64Array::from(times.to_vec())))
}

/// A table of `first`'s schema with `window`, holding `first`'s rows.
fn windowed(first: RecordBatch, window: Window) -> LiveTable {
    let mut table = LiveTable::new(first.schema(), "id").unwrap();
    table.set_window(Some(window)).unwrap();
    table.upsert(first).unwrap();
    table
}

/// Keys the tests use: below this.
const KEYS: i64 = 3_000;

/// The live keys, in order.
fn keys(table: &LiveTable) -> Vec<i64> {
    let mut keys: Vec<i64> = (0..KEYS)
        .filter(|&k| table.record_of_key(&Scalar::Int(k)).is_some())
        .collect();
    keys.sort_unstable();
    keys
}

#[test]
fn rows_exactly_a_span_old_stay_and_older_ones_go() {
    let window = Window::duration("time", Duration::from_secs(10));
    let mut table = windowed(millis(&[1, 2, 3], &[Some(0), Some(1), Some(5_000)]), window);
    assert_eq!(table.len(), 3);
    let changes = table.upsert(millis(&[4], &[Some(10_000)])).unwrap();
    assert_eq!(
        (changes.inserted, changes.deleted),
        (1, 0),
        "row 1 is exactly 10 s old"
    );
    let changes = table.upsert(millis(&[5], &[Some(10_001)])).unwrap();
    assert_eq!((changes.inserted, changes.deleted), (1, 1));
    assert_eq!(keys(&table), [2, 3, 4, 5]);
    assert_eq!(table.window().map(Window::column), Some("time"));
}

#[test]
fn a_row_already_too_old_is_counted_in_and_out() {
    let mut table = windowed(
        millis(&[1], &[Some(60_000)]),
        Window::duration("time", Duration::from_secs(1)),
    );
    let changes = table.upsert(millis(&[2], &[Some(0)])).unwrap();
    assert_eq!((changes.inserted, changes.deleted), (1, 1));
    assert_eq!(keys(&table), [1]);
}

#[test]
fn an_update_moves_a_row_in_time() {
    let mut table = windowed(
        millis(&[1, 2], &[Some(0), Some(5_000)]),
        Window::duration("time", Duration::from_secs(10)),
    );
    // Row 1 moves to the newest time; row 2 is now 10.5 s older than it.
    let changes = table.upsert(millis(&[1], &[Some(15_500)])).unwrap();
    assert_eq!((changes.updated, changes.deleted), (1, 1));
    assert_eq!(keys(&table), [1]);
}

#[test]
fn rows_without_a_time_stay_and_do_not_move_the_end() {
    let mut table = windowed(
        floats(
            &[1, 2, 3, 4, 5],
            &[
                Some(0.0),
                None,
                Some(f64::NAN),
                Some(f64::INFINITY),
                Some(5.0),
            ],
        ),
        Window::span("time", 10.0),
    );
    assert_eq!(table.len(), 5, "the end is 5: infinity is no time");
    table.upsert(floats(&[6], &[Some(10.5)])).unwrap();
    assert_eq!(keys(&table), [2, 3, 4, 5, 6]);
    table
        .upsert(floats(&[7], &[Some(f64::NEG_INFINITY)]))
        .unwrap();
    assert_eq!(keys(&table), [2, 3, 4, 5, 6], "-inf is older than any end");
}

#[test]
fn many_rows_at_minus_infinity_go_and_the_end_is_the_newest_number() {
    // Several blocks of -inf before the numbers, so the searches cross blocks.
    for (minus, numbers) in [(2_500, 1), (2_500, 400), (300, 2_000), (5, 3)] {
        let ids: Vec<i64> = (0..minus + numbers).collect();
        let times: Vec<Option<f64>> = (0..minus)
            .map(|_| Some(f64::NEG_INFINITY))
            .chain((0..numbers).map(|i| Some(i as f64)))
            .collect();
        let mut table = LiveTable::new(floats(&[], &[]).schema(), "id").unwrap();
        table.set_window(Some(Window::span("time", 10.0))).unwrap();
        let changes = table.upsert(floats(&ids, &times)).unwrap();
        let kept = numbers.min(11) as usize;
        assert_eq!(
            changes.deleted,
            ids.len() - kept,
            "{minus} -inf, {numbers} numbers"
        );
        assert_eq!(table.len(), kept);
    }
}

#[test]
fn rows_at_minus_infinity_alone_have_no_end_to_go_by() {
    let table = windowed(
        floats(
            &[1, 2, 3],
            &[Some(f64::NEG_INFINITY), None, Some(f64::INFINITY)],
        ),
        Window::span("time", 1.0),
    );
    assert_eq!(table.len(), 3);
}

#[test]
fn a_table_of_no_times_keeps_everything() {
    let table = windowed(
        floats(&[1, 2], &[None, Some(f64::NAN)]),
        Window::span("time", 0.0),
    );
    assert_eq!(table.len(), 2);
}

#[test]
fn deleting_the_newest_row_waits_for_the_next_upsert() {
    let mut table = windowed(
        millis(&[1, 2], &[Some(0), Some(5_000)]),
        Window::duration("time", Duration::from_secs(5)),
    );
    table.delete(&Int64Array::from(vec![2])).unwrap();
    table.upsert(millis(&[3], &[Some(4_000)])).unwrap();
    assert_eq!(keys(&table), [1, 3], "the end is now 4 s");
}

#[test]
fn a_window_set_later_deletes_at_once_and_none_keeps_everything_again() {
    let mut table = LiveTable::new(millis(&[], &[]).schema(), "id").unwrap();
    table
        .upsert(millis(&[1, 2, 3], &[Some(0), Some(500), Some(2_000)]))
        .unwrap();
    let changes = table
        .set_window(Some(Window::duration("time", Duration::from_secs(1))))
        .unwrap();
    assert_eq!(changes.deleted, 2);
    assert_eq!(keys(&table), [3]);
    assert_eq!(table.set_window(None).unwrap().deleted, 0);
    assert!(table.window().is_none());
    table.upsert(millis(&[4], &[Some(100_000)])).unwrap();
    assert_eq!(keys(&table), [3, 4]);
}

#[test]
fn spans_round_down_to_whole_units() {
    let seconds = |ids: &[i64], times: &[i64]| {
        batch(ids, Arc::new(TimestampSecondArray::from(times.to_vec())))
    };
    let mut table = windowed(
        seconds(&[1, 2], &[0, 1]),
        Window::duration("time", Duration::from_millis(1_999)),
    );
    table.upsert(seconds(&[3], &[2])).unwrap();
    assert_eq!(keys(&table), [2, 3], "1.999 s keeps 1 s of whole seconds");
}

#[test]
fn dates_count_whole_days() {
    let dates = |ids: &[i64], days: &[i32]| batch(ids, Arc::new(Date32Array::from(days.to_vec())));
    let day = Duration::from_secs(86_400);
    let mut table = windowed(dates(&[1, 2], &[0, 1]), Window::duration("time", day * 2));
    table.upsert(dates(&[3], &[3])).unwrap();
    assert_eq!(keys(&table), [2, 3]);
    let mut table = windowed(
        dates(&[1], &[0]),
        Window::duration("time", day - Duration::from_secs(1)),
    );
    table.upsert(dates(&[2], &[1])).unwrap();
    assert_eq!(
        keys(&table),
        [2],
        "less than a day keeps the newest day alone"
    );
}

#[test]
fn integers_unsigned_and_decimals_take_a_span_in_their_units() {
    let unsigned = |ids: &[i64], v: &[u64]| batch(ids, Arc::new(UInt64Array::from(v.to_vec())));
    let mut table = windowed(
        unsigned(&[1, 2, 4], &[0, u64::MAX - 10, u64::MAX - 11]),
        Window::span("time", 10.9),
    );
    table.upsert(unsigned(&[3], &[u64::MAX])).unwrap();
    assert_eq!(keys(&table), [2, 3], "10.9 keeps 10 whole units, not 11");

    let ints = |ids: &[i64], v: &[i64]| batch(ids, Arc::new(Int64Array::from(v.to_vec())));
    // A span a float error short of a whole number is that number.
    let table = windowed(ints(&[1, 2], &[0, 29]), Window::span("time", 0.29 * 100.0));
    assert_eq!(keys(&table), [1, 2]);
    let mut table = windowed(ints(&[1, 2], &[i64::MIN, 0]), Window::span("time", 1e30));
    table.upsert(ints(&[3], &[i64::MAX])).unwrap();
    assert_eq!(
        keys(&table),
        [1, 2, 3],
        "a span past every difference keeps all"
    );

    // Two decimal places: 1.5 is 150 hundredths.
    let decimals = |ids: &[i64], v: &[i128]| {
        let array = Decimal128Array::from(v.to_vec())
            .with_precision_and_scale(10, 2)
            .unwrap();
        batch(ids, Arc::new(array))
    };
    let mut table = windowed(decimals(&[1, 2], &[0, 50]), Window::span("time", 1.5));
    table.upsert(decimals(&[3], &[200])).unwrap();
    assert_eq!(keys(&table), [2, 3]);
    // 0.29 is 28.999999999999996 hundredths in floats: still a span of 29.
    let table = windowed(decimals(&[1, 2], &[0, 29]), Window::span("time", 0.29));
    assert_eq!(keys(&table), [1, 2]);
    let table = windowed(decimals(&[1, 2], &[0, 30]), Window::span("time", 0.29));
    assert_eq!(keys(&table), [2]);

    // The widest decimals are further apart than i128 holds: as far apart as can be.
    let widest = 10_i128.pow(38) - 1;
    let wide = |ids: &[i64], v: &[i128]| {
        let array = Decimal128Array::from(v.to_vec())
            .with_precision_and_scale(38, 0)
            .unwrap();
        batch(ids, Arc::new(array))
    };
    let table = windowed(
        wide(&[1, 2], &[-widest, widest]),
        Window::span("time", 1e40),
    );
    assert_eq!(keys(&table), [1, 2]);
    let table = windowed(
        wide(&[1, 2], &[-widest, widest]),
        Window::span("time", 1e30),
    );
    assert_eq!(keys(&table), [2]);
}

#[test]
fn every_time_unit_converts_the_span() {
    use arrow_array::{Date64Array, Time64MicrosecondArray, TimestampNanosecondArray};
    let second = Duration::from_secs(1);
    // Rows a second apart: the middle one is exactly the span old and stays.
    let cases: [ArrayRef; 3] = [
        Arc::new(Date64Array::from(vec![0, 1_000, 2_000])),
        Arc::new(Time64MicrosecondArray::from(vec![0, 1_000_000, 2_000_000])),
        Arc::new(TimestampNanosecondArray::from(vec![
            0,
            1_000_000_000,
            2_000_000_000,
        ])),
    ];
    for times in cases {
        let table = windowed(batch(&[1, 2, 3], times), Window::duration("time", second));
        assert_eq!(
            keys(&table),
            [2, 3],
            "{:?}",
            table.schema().field(1).data_type()
        );
    }
}

#[test]
fn a_span_that_does_not_fit_the_column_changes_nothing() {
    let mut table = windowed(
        millis(&[1], &[Some(0)]),
        Window::duration("time", Duration::ZERO),
    );
    let refused = [
        (
            Window::span("time", 5.0),
            DataError::WindowSpan("time".into()),
        ),
        (
            Window::duration("id", Duration::from_secs(1)),
            DataError::WindowSpan("id".into()),
        ),
        (Window::span("id", -1.0), DataError::WindowSpan("id".into())),
        (
            Window::span("id", f64::NAN),
            DataError::WindowSpan("id".into()),
        ),
        (
            Window::span("nowhere", 1.0),
            DataError::UnknownColumn("nowhere".into()),
        ),
    ];
    for (window, error) in refused {
        assert_eq!(
            table.set_window(Some(window.clone())),
            Err(error),
            "{window:?}"
        );
    }
    assert_eq!(
        table.window(),
        Some(&Window::duration("time", Duration::ZERO))
    );

    let text = RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from(vec![1])) as ArrayRef),
        ("name", Arc::new(StringArray::from(vec!["a"])) as ArrayRef),
    ])
    .unwrap();
    let mut table = LiveTable::new(text.schema(), "id").unwrap();
    let error = table
        .set_window(Some(Window::span("name", 1.0)))
        .unwrap_err();
    assert_eq!(error, DataError::WindowSpan("name".into()));
    assert!(error.to_string().contains("\"name\""), "{error}");

    let lists = ListArray::from_iter_primitive::<Int64Type, _, _>([Some(vec![Some(1)])]);
    let mut table = LiveTable::new(batch(&[1], Arc::new(lists)).schema(), "id").unwrap();
    let error = table
        .set_window(Some(Window::span("time", 1.0)))
        .unwrap_err();
    assert_eq!(error, DataError::UnsupportedColumn("time".into()));
}

#[test]
fn views_groups_and_records_follow_the_window() {
    let mut table = LiveTable::new(millis(&[], &[]).schema(), "id").unwrap();
    let newest_first = table
        .add_view(ViewSpec::new().sort_by("time", Order::Descending))
        .unwrap();
    let count = table
        .add_groups(GroupSpec::new().aggregate(Aggregate::count().named("rows")))
        .unwrap();
    table
        .set_window(Some(Window::duration("time", Duration::from_secs(3))))
        .unwrap();
    table
        .upsert(millis(
            &[1, 2, 3, 4],
            &[Some(0), Some(1_000), Some(2_000), Some(3_000)],
        ))
        .unwrap();
    let old = table.record_of_key(&Scalar::Int(1)).unwrap();
    table.upsert(millis(&[5], &[Some(4_000)])).unwrap();
    assert!(table.record_of_key(&Scalar::Int(1)).is_none());
    assert!(
        table.row_of(old).is_none(),
        "the expired key's record ended"
    );

    let view = table.view(newest_first).unwrap();
    let times: Vec<i64> = (0..view.len())
        .map(|i| {
            let at = view.row(i).unwrap();
            let column = table.batches()[at.batch].column(1);
            let times = column.as_any().downcast_ref::<TimestampMillisecondArray>();
            times.unwrap().value(at.row)
        })
        .collect();
    assert_eq!(times, [4_000, 3_000, 2_000, 1_000]);
    let groups = table.groups(count).unwrap().to_batch().unwrap();
    let rows = groups.column(0).as_any().downcast_ref::<UInt64Array>();
    assert_eq!(rows.map(|r| r.value(0)), Some(4));

    // A key that expired can come back as a new record.
    table.upsert(millis(&[1], &[Some(4_000)])).unwrap();
    assert_eq!(keys(&table), [1, 2, 3, 4, 5]);
}

#[test]
fn a_clone_keeps_its_own_window() {
    let mut table = windowed(
        millis(&[1, 2], &[Some(0), Some(1_000)]),
        Window::duration("time", Duration::from_secs(1)),
    );
    let mut copy = table.clone();
    copy.upsert(millis(&[3], &[Some(2_000)])).unwrap();
    assert_eq!(keys(&copy), [2, 3]);
    assert_eq!(keys(&table), [1, 2]);
    table.set_window(None).unwrap();
    assert!(copy.window().is_some());
}

/// Deterministic xorshift, so a failure reproduces.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

/// A float time: mostly numbers marching forward from `now`, some nulls, NaN and
/// infinities, some far behind.
fn random_time(rng: &mut Rng, now: f64) -> Option<f64> {
    match rng.below(40) {
        0 => None,
        1 => Some(f64::NAN),
        2 => Some(f64::INFINITY),
        3 | 4 => Some(f64::NEG_INFINITY),
        5 => Some(now - 500.0),
        _ => Some(now + rng.below(20) as f64 - 10.0),
    }
}

fn expired(time: Option<f64>, newest: f64, span: f64) -> bool {
    time.is_some_and(|t| newest - t > span)
}

/// Random upserts (repeating keys within a batch), deletes, window changes and
/// compactions; after every step the table's keys, a sorted view and a count match a model.
#[test]
fn random_changes_match_a_model() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut table = LiveTable::new(floats(&[], &[]).schema(), "id").unwrap();
    let by_time = table
        .add_view(ViewSpec::new().sort_by("time", Order::Ascending))
        .unwrap();
    let mut model: BTreeMap<i64, Option<f64>> = BTreeMap::new();
    let mut span = Some(30.0);
    table
        .set_window(span.map(|s| Window::span("time", s)))
        .unwrap();
    let mut now = 0.0;
    for step in 0..1_500 {
        let deleted_before = model.len();
        let mut expect_deleted = None;
        match rng.below(20) {
            0 => {
                let ids: Vec<i64> = (0..rng.below(4))
                    .map(|_| rng.below(KEYS as u64) as i64)
                    .collect();
                for id in &ids {
                    model.remove(id);
                }
                table.delete(&Int64Array::from(ids)).unwrap();
            }
            1 => {
                span = match rng.below(3) {
                    0 => None,
                    _ => Some(rng.below(60) as f64),
                };
                let changes = table
                    .set_window(span.map(|s| Window::span("time", s)))
                    .unwrap();
                apply(&mut model, span);
                expect_deleted = Some(changes.deleted);
            }
            2 => table.compact().unwrap(),
            _ => {
                now += rng.below(3) as f64;
                // Now and then a batch of thousands, so the table spans several blocks.
                let most = match rng.below(20) {
                    0 => 2_500,
                    1..=3 => 300,
                    _ => 20,
                };
                let n = 1 + rng.below(most) as usize;
                let ids: Vec<i64> = (0..n).map(|_| rng.below(KEYS as u64) as i64).collect();
                let times: Vec<Option<f64>> = (0..n).map(|_| random_time(&mut rng, now)).collect();
                for (id, time) in ids.iter().zip(&times) {
                    model.insert(*id, *time);
                }
                let changes = table.upsert(floats(&ids, &times)).unwrap();
                let before = model.len();
                apply(&mut model, span);
                assert_eq!(changes.deleted, before - model.len(), "step {step}");
            }
        }
        if let Some(deleted) = expect_deleted {
            assert_eq!(deleted, deleted_before - model.len(), "step {step}");
        }
        assert_eq!(
            keys(&table),
            model.keys().copied().collect::<Vec<_>>(),
            "step {step}"
        );
        let view = table.view(by_time).unwrap();
        assert_eq!(view.len(), model.len(), "step {step}");
        let times: Vec<Option<f64>> = (0..view.len())
            .map(|i| {
                let at = view.row(i).unwrap();
                let column = table.batches()[at.batch].column(1);
                let column = column.as_any().downcast_ref::<Float64Array>().unwrap();
                column.is_valid(at.row).then(|| column.value(at.row))
            })
            .collect();
        let mut expected: Vec<Option<f64>> = model.values().copied().collect();
        expected.sort_by(|a, b| match (a, b) {
            (Some(a), Some(b)) => a.total_cmp(b),
            (None, None) => std::cmp::Ordering::Equal,
            (None, _) => std::cmp::Ordering::Greater,
            (_, None) => std::cmp::Ordering::Less,
        });
        let bits = |v: &[Option<f64>]| v.iter().map(|t| t.map(f64::to_bits)).collect::<Vec<_>>();
        assert_eq!(bits(&times), bits(&expected), "step {step}");
    }
}

/// Removes the model's rows past a window of `span` (if any).
fn apply(model: &mut BTreeMap<i64, Option<f64>>, span: Option<f64>) {
    let Some(span) = span else { return };
    let newest = model
        .values()
        .flatten()
        .copied()
        .filter(|t| t.is_finite())
        .reduce(f64::max);
    if let Some(newest) = newest {
        model.retain(|_, time| !expired(*time, newest, span));
    }
}
