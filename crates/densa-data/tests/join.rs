//! Lookup joins, checked against a join recomputed from plain maps after every change.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, Int64Type};
use arrow_array::{Array, ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use densa_data::{CompareOp, DataError, Filter, LiveTable, LookupJoin, Scalar, ViewSpec};

fn left_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("sym", DataType::Utf8, true),
        Field::new("qty", DataType::Int64, true),
    ]))
}

fn right_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("sym", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("px", DataType::Float64, true),
    ]))
}

fn join() -> (LookupJoin, LiveTable) {
    let join = LookupJoin::new(left_schema(), "id", "sym", right_schema(), "sym").unwrap();
    let out = join.table().unwrap();
    (join, out)
}

fn lefts(rows: &[(i64, Option<&str>, i64)]) -> RecordBatch {
    let ids: ArrayRef = Arc::new(Int64Array::from_iter_values(rows.iter().map(|r| r.0)));
    let syms: ArrayRef = Arc::new(StringArray::from_iter(rows.iter().map(|r| r.1)));
    let qty: ArrayRef = Arc::new(Int64Array::from_iter_values(rows.iter().map(|r| r.2)));
    RecordBatch::try_new(left_schema(), vec![ids, syms, qty]).unwrap()
}

fn rights(rows: &[(&str, &str, f64)]) -> RecordBatch {
    let syms: ArrayRef = Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.0)));
    let names: ArrayRef = Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.1)));
    let px: ArrayRef = Arc::new(Float64Array::from_iter_values(rows.iter().map(|r| r.2)));
    RecordBatch::try_new(right_schema(), vec![syms, names, px]).unwrap()
}

/// An output row: id, sym, qty, name, px.
type Row = (i64, Option<String>, i64, Option<String>, Option<f64>);

/// The output's rows, by id.
fn rows(out: &LiveTable) -> Vec<Row> {
    let mut rows = Vec::new();
    for batch in out.snapshot().unwrap() {
        let ids = batch.column(0).as_primitive::<Int64Type>();
        let syms = batch.column(1).as_string::<i32>();
        let qty = batch.column(2).as_primitive::<Int64Type>();
        let names = batch.column(3).as_string::<i32>();
        let px = batch.column(4).as_primitive::<Float64Type>();
        for i in 0..batch.num_rows() {
            let text = |a: &arrow_array::StringArray| a.is_valid(i).then(|| a.value(i).to_owned());
            rows.push((
                ids.value(i),
                text(syms),
                qty.value(i),
                text(names),
                px.is_valid(i).then(|| px.value(i)),
            ));
        }
    }
    rows.sort_by_key(|r| r.0);
    rows
}

#[test]
fn rows_get_the_columns_of_the_row_they_name() {
    let (mut join, mut out) = join();
    let names: Vec<&str> = join
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    assert_eq!(names, ["id", "sym", "qty", "name", "px"]);
    join.upsert_right(&mut out, rights(&[("A", "Alpha", 1.5)]))
        .unwrap();
    assert!(out.is_empty(), "no left rows yet");
    let changes = join
        .upsert_left(
            &mut out,
            lefts(&[(1, Some("A"), 10), (2, Some("B"), 20), (3, None, 30)]),
        )
        .unwrap();
    assert_eq!(changes.inserted, 3);
    assert_eq!(
        rows(&out),
        [
            (1, Some("A".into()), 10, Some("Alpha".into()), Some(1.5)),
            (2, Some("B".into()), 20, None, None),
            (3, None, 30, None, None),
        ]
    );
    let changes = join
        .upsert_right(
            &mut out,
            rights(&[("B", "Beta", 2.0), ("A", "Alpha", 1.75)]),
        )
        .unwrap();
    assert_eq!(changes.updated, 2, "rows 1 and 2 joined again");
    assert_eq!(rows(&out)[0].4, Some(1.75));
    assert_eq!(rows(&out)[1].3.as_deref(), Some("Beta"));
    join.delete_right(&mut out, &StringArray::from(vec!["A"]))
        .unwrap();
    assert_eq!(
        rows(&out)[0],
        (1, Some("A".into()), 10, None, None),
        "A is gone: nulls"
    );
    assert_eq!(join.right().len(), 1);
}

#[test]
fn a_left_row_that_names_another_key_follows_it() {
    let (mut join, mut out) = join();
    join.upsert_right(&mut out, rights(&[("A", "Alpha", 1.0), ("B", "Beta", 2.0)]))
        .unwrap();
    join.upsert_left(&mut out, lefts(&[(1, Some("A"), 1)]))
        .unwrap();
    join.upsert_left(&mut out, lefts(&[(1, Some("B"), 1)]))
        .unwrap();
    let changes = join
        .upsert_right(&mut out, rights(&[("A", "Alpha 2", 1.0)]))
        .unwrap();
    assert_eq!(
        changes.updated, 0,
        "no row names A any more: none is joined again"
    );
    assert_eq!(
        rows(&out)[0].3.as_deref(),
        Some("Beta"),
        "A's change is not row 1's"
    );
    join.upsert_right(&mut out, rights(&[("B", "Beta 2", 2.0)]))
        .unwrap();
    assert_eq!(rows(&out)[0].3.as_deref(), Some("Beta 2"));
}

#[test]
fn left_rows_removed_from_the_output_are_left_alone() {
    let (mut join, mut out) = join();
    join.upsert_left(&mut out, lefts(&[(1, Some("A"), 1), (2, Some("A"), 2)]))
        .unwrap();
    out.delete(&Int64Array::from(vec![1])).unwrap();
    let changes = join
        .upsert_right(&mut out, rights(&[("A", "Alpha", 1.0)]))
        .unwrap();
    assert_eq!(
        (changes.inserted, changes.updated),
        (0, 1),
        "row 1 does not come back"
    );
    assert_eq!(rows(&out).len(), 1);
}

#[test]
fn another_output_table_is_indexed_anew() {
    let (mut join, mut out) = join();
    // Row ids differ between the tables: out's A is its second row, other's first names B.
    join.upsert_left(&mut out, lefts(&[(1, Some("B"), 1), (2, Some("A"), 2)]))
        .unwrap();
    let mut other = join.table().unwrap();
    join.upsert_left(&mut other, lefts(&[(5, Some("B"), 5)]))
        .unwrap();
    join.upsert_right(&mut other, rights(&[("B", "Beta", 1.0)]))
        .unwrap();
    assert_eq!(rows(&other)[0].3.as_deref(), Some("Beta"));
    join.upsert_right(&mut out, rights(&[("A", "Alpha", 1.0)]))
        .unwrap();
    assert_eq!(
        rows(&out)[1].3.as_deref(),
        Some("Alpha"),
        "each table indexed when it comes"
    );
    assert_eq!(rows(&out)[0].3, None, "out's B was joined before Beta came");
}

#[test]
fn the_output_is_a_table_like_any_other() {
    let (mut join, mut out) = join();
    let matched = out
        .add_view(ViewSpec::new().filter(Filter::is_not_null("name")))
        .unwrap();
    let big = out
        .add_view(ViewSpec::new().filter(Filter::compare("qty", CompareOp::Gt, Scalar::Int(5))))
        .unwrap();
    join.upsert_left(&mut out, lefts(&[(1, Some("A"), 1), (2, Some("B"), 9)]))
        .unwrap();
    assert_eq!(
        out.view(matched).unwrap().len(),
        0,
        "an inner join is a view of the output"
    );
    join.upsert_right(&mut out, rights(&[("B", "Beta", 2.0)]))
        .unwrap();
    assert_eq!(out.view(matched).unwrap().len(), 1);
    assert_eq!(out.view(big).unwrap().len(), 1);
}

#[test]
fn what_cannot_be_joined_is_refused_with_the_reason() {
    let ints = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("sym", DataType::Int64, true),
    ]));
    let error = LookupJoin::new(ints, "id", "sym", right_schema(), "sym").unwrap_err();
    assert_eq!(error, DataError::JoinKey("sym".into()));
    assert!(error.to_string().contains("same kind of value"), "{error}");
    let clash = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("sym", DataType::Utf8, true),
        Field::new("name", DataType::Utf8, true),
    ]));
    let error = LookupJoin::new(clash, "id", "sym", right_schema(), "sym").unwrap_err();
    assert_eq!(error, DataError::DuplicateName("name".into()));
    assert!(error.to_string().contains("join"), "{error}");
    for (key, on, right_key) in [
        ("nope", "sym", "sym"),
        ("id", "nope", "sym"),
        ("id", "sym", "nope"),
    ] {
        let error = LookupJoin::new(left_schema(), key, on, right_schema(), right_key).unwrap_err();
        assert_eq!(error, DataError::UnknownColumn("nope".into()));
    }
    let error = LookupJoin::new(left_schema(), "qty", "sym", right_schema(), "px").unwrap_err();
    assert_eq!(error, DataError::UnsupportedKey("px".into()));
}

#[test]
fn bad_rows_and_tables_change_nothing() {
    let (mut join, mut out) = join();
    join.upsert_left(&mut out, lefts(&[(1, Some("A"), 1)]))
        .unwrap();
    let keyed_otherwise = LiveTable::new(join.schema().clone(), "sym").unwrap();
    let mut keyed_otherwise = keyed_otherwise;
    assert_eq!(
        join.upsert_left(&mut keyed_otherwise, lefts(&[(2, Some("A"), 2)]))
            .unwrap_err(),
        DataError::JoinTable,
        "the join's columns, but keyed by another"
    );
    let mut stranger = LiveTable::new(left_schema(), "id").unwrap();
    let error = join
        .upsert_left(&mut stranger, lefts(&[(2, Some("A"), 2)]))
        .unwrap_err();
    assert_eq!(error, DataError::JoinTable);
    assert!(error.to_string().contains("LookupJoin::table"), "{error}");
    assert_eq!(
        join.upsert_right(&mut stranger, rights(&[("A", "x", 1.0)]))
            .unwrap_err(),
        DataError::JoinTable
    );
    assert!(join.right().is_empty(), "nothing taken in");
    assert_eq!(
        join.upsert_left(&mut out, rights(&[("A", "x", 1.0)]))
            .unwrap_err(),
        DataError::ColumnMismatch { column: 0 }
    );
    let null_key = RecordBatch::try_new(
        join.right().schema().clone(),
        vec![
            Arc::new(StringArray::from(vec![None::<&str>])) as ArrayRef,
            Arc::new(StringArray::from(vec!["x"])),
            Arc::new(Float64Array::from(vec![1.0])),
        ],
    )
    .unwrap();
    assert_eq!(
        join.upsert_right(&mut out, null_key).unwrap_err(),
        DataError::NullKey { row: 0 }
    );
    assert_eq!(
        join.delete_right(&mut out, &Int64Array::from(vec![1]))
            .unwrap_err(),
        DataError::KeyType
    );
    assert_eq!(rows(&out), [(1, Some("A".into()), 1, None, None)]);
}

/// A tiny random walk, the same for a seed everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const SYMS: [&str; 8] = ["A", "B", "C", "D", "E", "F", "G", "H"];

/// The join recomputed from plain maps.
fn expected(
    left: &BTreeMap<i64, (Option<String>, i64)>,
    right: &BTreeMap<String, (String, f64)>,
) -> Vec<Row> {
    left.iter()
        .map(|(&id, (sym, qty))| {
            let found = sym.as_ref().and_then(|s| right.get(s));
            (
                id,
                sym.clone(),
                *qty,
                found.map(|r| r.0.clone()),
                found.map(|r| r.1),
            )
        })
        .collect()
}

#[test]
fn the_output_matches_a_join_recomputed_after_every_change() {
    for seed in 1..=12u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let (mut join, mut out) = join();
        let mut left: BTreeMap<i64, (Option<String>, i64)> = BTreeMap::new();
        let mut right: BTreeMap<String, (String, f64)> = BTreeMap::new();
        for step in 0..600 {
            match rng.below(10) {
                0..=4 => {
                    let rows: Vec<(i64, Option<&str>, i64)> = (0..1 + rng.below(6))
                        .map(|_| {
                            let id = rng.below(30) as i64;
                            let sym = (rng.below(9) < 8).then(|| SYMS[rng.below(8) as usize]);
                            (id, sym, rng.below(1000) as i64)
                        })
                        .collect();
                    join.upsert_left(&mut out, lefts(&rows)).unwrap();
                    for (id, sym, qty) in rows {
                        left.insert(id, (sym.map(str::to_owned), qty));
                    }
                }
                5..=6 => {
                    let rows: Vec<(String, String, f64)> = (0..1 + rng.below(4))
                        .map(|_| {
                            let sym = SYMS[rng.below(8) as usize].to_owned();
                            (
                                sym,
                                format!("n{}", rng.below(100)),
                                rng.below(500) as f64 / 4.0,
                            )
                        })
                        .collect();
                    let borrowed: Vec<(&str, &str, f64)> = rows
                        .iter()
                        .map(|(s, n, p)| (s.as_str(), n.as_str(), *p))
                        .collect();
                    join.upsert_right(&mut out, rights(&borrowed)).unwrap();
                    for (sym, name, px) in rows {
                        right.insert(sym, (name, px));
                    }
                }
                7 => {
                    let gone: Vec<&str> = (0..1 + rng.below(3))
                        .map(|_| SYMS[rng.below(8) as usize])
                        .collect();
                    join.delete_right(&mut out, &StringArray::from(gone.clone()))
                        .unwrap();
                    for sym in gone {
                        right.remove(sym);
                    }
                }
                8 => {
                    let gone: Vec<i64> = (0..1 + rng.below(3))
                        .map(|_| rng.below(30) as i64)
                        .collect();
                    out.delete(&Int64Array::from(gone.clone())).unwrap();
                    for id in gone {
                        left.remove(&id);
                    }
                }
                _ => out.compact().unwrap(),
            }
            assert_eq!(
                rows(&out),
                expected(&left, &right),
                "seed {seed}, step {step}"
            );
        }
    }
}

#[test]
fn a_stream_whose_oldest_rows_leave_stays_joined() {
    // Rows keep coming and the oldest leave (as with a window), so row ids are reused by rows
    // naming other keys: the index gathers stale entries, outgrows the table and is rebuilt.
    let (mut join, mut out) = join();
    let syms: Vec<String> = (0..300).map(|i| format!("S{i}")).collect();
    for i in 0..6_000i64 {
        let sym = syms[(i * 7 % 300) as usize].as_str();
        join.upsert_left(&mut out, lefts(&[(i, Some(sym), i)]))
            .unwrap();
        if i >= 40 {
            out.delete(&Int64Array::from(vec![i - 40])).unwrap();
        }
    }
    assert_eq!(out.len(), 40);
    let named: Vec<(String, String, f64)> = syms
        .iter()
        .map(|s| (s.clone(), format!("name of {s}"), 1.0))
        .collect();
    let borrowed: Vec<(&str, &str, f64)> = named
        .iter()
        .map(|(s, n, p)| (s.as_str(), n.as_str(), *p))
        .collect();
    let changes = join.upsert_right(&mut out, rights(&borrowed)).unwrap();
    assert_eq!(
        (changes.inserted, changes.updated),
        (0, 40),
        "only the live rows, once each"
    );
    for (id, sym, _, name, _) in rows(&out) {
        assert_eq!(name, Some(format!("name of {}", sym.unwrap())), "row {id}");
    }
}

#[test]
fn a_right_batch_of_too_few_columns_is_refused_whatever_the_key_column() {
    // The right key is the last column: reading it before the columns are checked would
    // panic on a batch without it.
    let right = Arc::new(Schema::new(vec![
        Field::new("name", DataType::Utf8, true),
        Field::new("px", DataType::Float64, true),
        Field::new("sym", DataType::Utf8, false),
    ]));
    let mut join = LookupJoin::new(left_schema(), "id", "sym", right, "sym").unwrap();
    let mut out = join.table().unwrap();
    let short =
        RecordBatch::try_from_iter([("name", Arc::new(StringArray::from(vec!["x"])) as ArrayRef)])
            .unwrap();
    let error = join.upsert_right(&mut out, short).unwrap_err();
    assert_eq!(
        error,
        DataError::ColumnCount {
            expected: 3,
            found: 1
        }
    );
    assert!(join.right().is_empty());
}

#[test]
fn a_right_batch_repeating_a_key_joins_each_row_once_with_its_last() {
    let (mut join, mut out) = join();
    join.upsert_left(
        &mut out,
        lefts(&[(1, Some("A"), 1), (2, Some("A"), 2), (3, Some("B"), 3)]),
    )
    .unwrap();
    let changes = join
        .upsert_right(
            &mut out,
            rights(&[
                ("A", "first", 1.0),
                ("B", "beta", 2.0),
                ("A", "last", 3.0),
                ("A", "x", 4.0),
            ]),
        )
        .unwrap();
    assert_eq!(changes.updated, 3, "each row once");
    let names: Vec<Option<String>> = rows(&out).into_iter().map(|r| r.3).collect();
    assert_eq!(
        names,
        [Some("x".into()), Some("x".into()), Some("beta".into())]
    );
}

#[test]
fn rows_a_window_lets_go_are_not_joined_again() {
    let left = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("sym", DataType::Utf8, true),
        Field::new("t", DataType::Int64, true),
    ]));
    let mut join = LookupJoin::new(Arc::clone(&left), "id", "sym", right_schema(), "sym").unwrap();
    let mut out = join.table().unwrap();
    out.set_window(Some(densa_data::Window::span("t", 10.0)))
        .unwrap();
    let at = |id: i64, t: i64| {
        RecordBatch::try_new(
            Arc::clone(&left),
            vec![
                Arc::new(Int64Array::from(vec![id])) as ArrayRef,
                Arc::new(StringArray::from(vec!["A"])),
                Arc::new(Int64Array::from(vec![t])),
            ],
        )
        .unwrap()
    };
    for (id, t) in [(1, 0), (2, 5), (3, 12)] {
        join.upsert_left(&mut out, at(id, t)).unwrap();
    }
    assert_eq!(out.len(), 2, "t = 0 is more than 10 older than 12");
    let changes = join
        .upsert_right(&mut out, rights(&[("A", "Alpha", 1.0)]))
        .unwrap();
    assert_eq!(
        (changes.inserted, changes.updated, changes.deleted),
        (0, 2, 0)
    );
    assert_eq!(out.len(), 2);
}

#[test]
fn keys_join_across_integer_widths_but_not_across_kinds() {
    let right = |key: DataType| {
        Arc::new(Schema::new(vec![
            Field::new("k", key, false),
            Field::new("v", DataType::Utf8, true),
        ]))
    };
    let left = |on: DataType| {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("ref", on, true),
        ]))
    };
    assert!(LookupJoin::new(
        left(DataType::Int32),
        "id",
        "ref",
        right(DataType::Int64),
        "k"
    )
    .is_ok());
    assert!(LookupJoin::new(
        left(DataType::UInt8),
        "id",
        "ref",
        right(DataType::UInt64),
        "k"
    )
    .is_ok());
    assert!(LookupJoin::new(
        left(DataType::LargeUtf8),
        "id",
        "ref",
        right(DataType::Utf8View),
        "k"
    )
    .is_ok());
    for (on, key) in [
        (DataType::Int64, DataType::UInt64),
        (DataType::Utf8, DataType::Int64),
        (
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            DataType::Utf8,
        ),
    ] {
        let error = LookupJoin::new(left(on.clone()), "id", "ref", right(key), "k").unwrap_err();
        assert_eq!(error, DataError::JoinKey("ref".into()), "{on}");
    }

    // An Int32 reference finds an Int64 key.
    let mut join = LookupJoin::new(
        left(DataType::Int32),
        "id",
        "ref",
        right(DataType::Int64),
        "k",
    )
    .unwrap();
    let mut out = join.table().unwrap();
    let rights = RecordBatch::try_new(
        join.right().schema().clone(),
        vec![
            Arc::new(Int64Array::from(vec![7])) as ArrayRef,
            Arc::new(StringArray::from(vec!["seven"])),
        ],
    )
    .unwrap();
    join.upsert_right(&mut out, rights).unwrap();
    let lefts = RecordBatch::try_new(
        left(DataType::Int32),
        vec![
            Arc::new(Int64Array::from(vec![1])) as ArrayRef,
            Arc::new(arrow_array::Int32Array::from(vec![7])),
        ],
    )
    .unwrap();
    join.upsert_left(&mut out, lefts).unwrap();
    let batch = &out.snapshot().unwrap()[0];
    assert_eq!(batch.column(2).as_string::<i32>().value(0), "seven");
}

#[test]
fn right_columns_of_any_text_layout_come_through() {
    use arrow_array::types::Int8Type;
    use arrow_array::{DictionaryArray, LargeStringArray, StringViewArray};
    let right = Arc::new(Schema::new(vec![
        Field::new("sym", DataType::Utf8, false),
        Field::new("large", DataType::LargeUtf8, true),
        Field::new("view", DataType::Utf8View, true),
        Field::new(
            "dict",
            DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::Utf8)),
            true,
        ),
    ]));
    let mut join = LookupJoin::new(left_schema(), "id", "sym", Arc::clone(&right), "sym").unwrap();
    let mut out = join.table().unwrap();
    for (sym, text) in [("A", "alpha"), ("B", "beta"), ("A", "alpha 2")] {
        let dict: DictionaryArray<Int8Type> = vec![text].into_iter().collect();
        let rows = RecordBatch::try_new(
            Arc::clone(&right),
            vec![
                Arc::new(StringArray::from(vec![sym])) as ArrayRef,
                Arc::new(LargeStringArray::from(vec![text])),
                Arc::new(StringViewArray::from(vec![text])),
                Arc::new(dict),
            ],
        )
        .unwrap();
        join.upsert_right(&mut out, rows).unwrap();
    }
    join.upsert_left(
        &mut out,
        lefts(&[(1, Some("A"), 1), (2, Some("B"), 2), (3, Some("C"), 3)]),
    )
    .unwrap();
    let mut seen = Vec::new();
    for batch in out.snapshot().unwrap() {
        let ids = batch.column(0).as_primitive::<Int64Type>();
        let large = batch.column(3).as_string::<i64>();
        let view = batch.column(4).as_string_view();
        let dict = batch.column(5).as_dictionary::<Int8Type>();
        let dict_values = dict.values().as_string::<i32>();
        for i in 0..batch.num_rows() {
            let text = |valid: bool, value: &str| valid.then(|| value.to_owned());
            seen.push((
                ids.value(i),
                text(large.is_valid(i), large.value(i)),
                text(view.is_valid(i), view.value(i)),
                dict.is_valid(i)
                    .then(|| dict_values.value(dict.keys().value(i) as usize).to_owned()),
            ));
        }
    }
    seen.sort();
    let both = |t: &str| (Some(t.to_owned()), Some(t.to_owned()), Some(t.to_owned()));
    let expect = |id: i64, t: Option<&str>| {
        let (a, b, c) = t.map_or((None, None, None), both);
        (id, a, b, c)
    };
    assert_eq!(
        seen,
        [
            expect(1, Some("alpha 2")),
            expect(2, Some("beta")),
            expect(3, None)
        ]
    );
}

#[test]
fn a_right_table_of_many_batches_joins_from_the_ones_it_reads() {
    let (mut join, mut out) = join();
    for i in 0..300 {
        let sym = SYMS[i % 8];
        join.upsert_right(&mut out, rights(&[(sym, &format!("n{i}"), i as f64)]))
            .unwrap();
    }
    assert!(join.right().batches().len() > 8, "one batch per upsert");
    let lefts_rows: Vec<(i64, Option<&str>, i64)> = (0..16)
        .map(|i| (i, Some(SYMS[(i % 8) as usize]), i))
        .collect();
    join.upsert_left(&mut out, lefts(&lefts_rows)).unwrap();
    for (id, _, _, name, px) in rows(&out) {
        let last = (0..300).rev().find(|i| i % 8 == (id % 8) as usize).unwrap();
        assert_eq!(
            (name, px),
            (Some(format!("n{last}")), Some(last as f64)),
            "row {id}"
        );
    }
}

#[test]
fn empty_batches_change_nothing() {
    let (mut join, mut out) = join();
    assert_eq!(
        join.upsert_left(&mut out, lefts(&[])).unwrap(),
        Default::default()
    );
    assert_eq!(
        join.upsert_right(&mut out, rights(&[])).unwrap(),
        Default::default()
    );
    join.upsert_left(&mut out, lefts(&[(1, Some("A"), 1)]))
        .unwrap();
    assert_eq!(
        join.upsert_left(&mut out, lefts(&[])).unwrap(),
        Default::default()
    );
    let none: &[&str] = &[];
    assert_eq!(
        join.delete_right(&mut out, &StringArray::from(none.to_vec()))
            .unwrap(),
        Default::default()
    );
    assert_eq!(rows(&out).len(), 1);
}

#[test]
fn left_rows_may_carry_nulls_where_the_left_columns_say_none() {
    // Like a LiveTable, the join takes nulls in any column but the key.
    let strict = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("sym", DataType::Utf8, false),
        Field::new("qty", DataType::Int64, false),
    ]));
    let mut join = LookupJoin::new(strict, "id", "sym", right_schema(), "sym").unwrap();
    let mut out = join.table().unwrap();
    join.upsert_left(&mut out, lefts(&[(1, None, 1), (2, Some("A"), 2)]))
        .unwrap();
    join.upsert_right(&mut out, rights(&[("A", "Alpha", 1.0)]))
        .unwrap();
    assert_eq!(rows(&out)[0], (1, None, 1, None, None));
    assert_eq!(rows(&out)[1].3.as_deref(), Some("Alpha"));
}
