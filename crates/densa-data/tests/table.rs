//! Live tables: keys, upserts, deletes, compaction, and the errors of each.

use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::{Int32Type, UInt8Type};
use arrow_array::{
    Array, ArrayRef, BinaryViewArray, Decimal128Array, DictionaryArray, Float64Array, Int64Array,
    ListArray, RecordBatch, StringArray, StringViewArray, TimestampMillisecondArray, UInt8Array,
};
use arrow_schema::{DataType, Field, Schema};
use densa_data::{
    Changes, CompareOp, DataError, Filter, LiveTable, Order, RowRef, Scalar, TableOptions, ViewSpec,
};

fn batch(columns: Vec<(&str, ArrayRef)>) -> RecordBatch {
    RecordBatch::try_from_iter(columns).unwrap()
}

fn prices(ids: Vec<i64>, values: Vec<f64>) -> RecordBatch {
    batch(vec![
        ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
        ("price", Arc::new(Float64Array::from(values)) as ArrayRef),
    ])
}

/// The price stored at `at`.
fn price(table: &LiveTable, at: RowRef) -> f64 {
    let column = table.batches()[at.batch].column(1);
    column
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap()
        .value(at.row)
}

fn table() -> LiveTable {
    LiveTable::new(prices(vec![], vec![]).schema(), "id").unwrap()
}

mod keys {
    use super::*;

    #[test]
    fn the_key_must_be_an_existing_integer_or_string_column() {
        let schema = prices(vec![], vec![]).schema();
        assert_eq!(
            LiveTable::new(Arc::clone(&schema), "nope").unwrap_err(),
            DataError::UnknownColumn("nope".into())
        );
        assert_eq!(
            LiveTable::new(schema, "price").unwrap_err(),
            DataError::UnsupportedKey("price".into())
        );
    }

    #[test]
    fn upserting_a_new_key_inserts_and_a_known_key_updates() {
        let mut t = table();
        let changes: Changes = t.upsert(prices(vec![1, 2], vec![10.0, 20.0])).unwrap();
        assert_eq!((changes.inserted, changes.updated), (2, 0));
        let changes = t.upsert(prices(vec![2, 3], vec![21.0, 30.0])).unwrap();
        assert_eq!((changes.inserted, changes.updated), (1, 1));
        assert_eq!(t.len(), 3);
        assert_eq!(price(&t, t.find(&Scalar::Int(2)).unwrap()), 21.0);
    }

    #[test]
    fn the_last_row_for_a_key_in_one_batch_wins() {
        let mut t = table();
        let changes = t
            .upsert(prices(vec![7, 7, 7], vec![1.0, 2.0, 3.0]))
            .unwrap();
        assert_eq!(changes.inserted, 1);
        assert_eq!(t.len(), 1);
        assert_eq!(price(&t, t.find(&Scalar::Int(7)).unwrap()), 3.0);
    }

    #[test]
    fn string_keys_work_like_integer_keys() {
        let names = |v: Vec<&str>, p: Vec<f64>| {
            batch(vec![
                ("sym", Arc::new(StringArray::from(v)) as ArrayRef),
                ("price", Arc::new(Float64Array::from(p)) as ArrayRef),
            ])
        };
        let mut t = LiveTable::new(names(vec![], vec![]).schema(), "sym").unwrap();
        t.upsert(names(vec!["AAPL", "MSFT"], vec![1.0, 2.0]))
            .unwrap();
        t.upsert(names(vec!["AAPL"], vec![1.5])).unwrap();
        assert_eq!(t.len(), 2);
        let at = t.find(&Scalar::Str("AAPL".into())).unwrap();
        assert_eq!(price(&t, at), 1.5);
        assert_eq!(t.find(&Scalar::Str("NOPE".into())), None);
    }

    #[test]
    fn a_null_key_rejects_the_whole_batch() {
        let mut t = table();
        let ids: ArrayRef = Arc::new(Int64Array::from(vec![Some(1), None]));
        let values: ArrayRef = Arc::new(Float64Array::from(vec![1.0, 2.0]));
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("price", DataType::Float64, false),
        ]));
        let bad = RecordBatch::try_new(schema, vec![ids, values]).unwrap();
        assert_eq!(t.upsert(bad), Err(DataError::NullKey { row: 1 }));
        assert!(t.is_empty());
    }

    #[test]
    fn columns_are_matched_by_name_too() {
        let mut t = table();
        let renamed = batch(vec![
            ("id", Arc::new(Int64Array::from(vec![1])) as ArrayRef),
            ("cost", Arc::new(Float64Array::from(vec![1.0])) as ArrayRef),
        ]);
        assert_eq!(
            t.upsert(renamed),
            Err(DataError::ColumnMismatch { column: 1 })
        );
    }

    #[test]
    fn a_batch_with_other_columns_is_rejected() {
        let mut t = table();
        let other = batch(vec![(
            "id",
            Arc::new(Int64Array::from(vec![1])) as ArrayRef,
        )]);
        assert_eq!(
            t.upsert(other),
            Err(DataError::ColumnCount {
                expected: 2,
                found: 1
            })
        );
        let retyped = batch(vec![
            ("id", Arc::new(Int64Array::from(vec![1])) as ArrayRef),
            ("price", Arc::new(StringArray::from(vec!["x"])) as ArrayRef),
        ]);
        assert_eq!(
            t.upsert(retyped),
            Err(DataError::ColumnMismatch { column: 1 })
        );
    }

    #[test]
    fn deleting_counts_only_keys_that_existed() {
        let mut t = table();
        t.upsert(prices(vec![1, 2, 3], vec![1.0, 2.0, 3.0]))
            .unwrap();
        assert_eq!(
            t.delete(&Int64Array::from(vec![2, 9, 2])).unwrap().deleted,
            1
        );
        assert_eq!(t.len(), 2);
        assert_eq!(t.find(&Scalar::Int(2)), None);
        assert_eq!(
            t.delete(&StringArray::from(vec!["1"])),
            Err(DataError::KeyType)
        );
    }
}

mod compaction {
    use super::*;

    #[test]
    fn nulls_in_a_column_the_first_schema_called_non_nullable_survive_compaction() {
        let strict = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("price", DataType::Float64, false),
        ]));
        let mut t = LiveTable::new(strict, "id").unwrap();
        t.upsert(prices(vec![1, 2], vec![1.0, 2.0])).unwrap();
        let loose = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("price", DataType::Float64, true),
        ]));
        let with_null = RecordBatch::try_new(
            loose,
            vec![
                Arc::new(Int64Array::from(vec![2])) as ArrayRef,
                Arc::new(Float64Array::from(vec![None])) as ArrayRef,
            ],
        )
        .unwrap();
        t.upsert(with_null).unwrap();
        t.compact().unwrap();
        let at = t.find(&Scalar::Int(2)).unwrap();
        assert!(t.batches()[at.batch].column(1).is_null(at.row));
        assert!(t.compaction_error().is_none());
    }

    #[test]
    fn without_automatic_compaction_superseded_rows_stay_until_compact() {
        let options = TableOptions::default().with_auto_compact(false);
        let mut t =
            LiveTable::with_options(prices(vec![], vec![]).schema(), "id", options).unwrap();
        for round in 0..1_000 {
            t.upsert(prices((0..10).collect(), vec![f64::from(round); 10]))
                .unwrap();
        }
        assert_eq!(t.generation(), 0);
        assert_eq!(t.batches().len(), 1_000);
        t.compact().unwrap();
        assert_eq!(t.batches().len(), 1);
    }

    #[test]
    fn compaction_keeps_every_live_row_and_drops_superseded_ones() {
        let mut t = table();
        for round in 0..50 {
            t.upsert(prices(vec![1, 2, 3], vec![round as f64; 3]))
                .unwrap();
        }
        t.delete(&Int64Array::from(vec![3])).unwrap();
        let generation = t.generation();
        t.compact().unwrap();
        assert!(t.generation() > generation);
        let rows: usize = t.batches().iter().map(RecordBatch::num_rows).sum();
        assert_eq!(rows, 2);
        assert_eq!(price(&t, t.find(&Scalar::Int(1)).unwrap()), 49.0);
    }

    #[test]
    fn compaction_writes_several_batches_for_many_rows() {
        let mut t = table();
        let ids: Vec<i64> = (0..70_000).collect();
        let values: Vec<f64> = ids.iter().map(|&i| i as f64).collect();
        t.upsert(prices(ids, values)).unwrap();
        t.upsert(prices(vec![69_999], vec![-1.0])).unwrap();
        t.compact().unwrap();
        assert!(t.batches().len() >= 2);
        assert_eq!(price(&t, t.find(&Scalar::Int(69_998)).unwrap()), 69_998.0);
        assert_eq!(price(&t, t.find(&Scalar::Int(69_999)).unwrap()), -1.0);
    }

    #[test]
    fn many_superseded_rows_are_compacted_automatically() {
        let mut t = table();
        for round in 0..2_000 {
            let ids: Vec<i64> = (0..10).collect();
            t.upsert(prices(ids, vec![f64::from(round); 10])).unwrap();
        }
        let stored: usize = t.batches().iter().map(RecordBatch::num_rows).sum();
        assert!(stored < 20_000 / 2, "{stored} rows kept for 10 keys");
    }

    /// 200 rows whose `name` is a u8-keyed dictionary of 200 distinct strings, starting at
    /// key `first`; strings or string views.
    fn names(first: i64, views: bool) -> RecordBatch {
        let text: Vec<String> = (first..first + 200).map(|i| format!("n{i:03}")).collect();
        let values: ArrayRef = if views {
            Arc::new(StringViewArray::from_iter_values(text))
        } else {
            Arc::new(StringArray::from(text))
        };
        let keys = UInt8Array::from((0..200u8).collect::<Vec<_>>());
        let names = DictionaryArray::<UInt8Type>::try_new(keys, values).unwrap();
        batch(vec![
            (
                "id",
                Arc::new(Int64Array::from((first..first + 200).collect::<Vec<_>>())) as ArrayRef,
            ),
            ("name", Arc::new(names) as ArrayRef),
        ])
    }

    /// Rows `ids` whose `name` is a u8-keyed dictionary of 200 strings `{prefix}000..199`,
    /// row `i` pointing at string `ids[i] % 200`.
    fn coded(ids: Vec<i64>, prefix: &str, views: bool) -> RecordBatch {
        let text: Vec<String> = (0..200).map(|i| format!("{prefix}{i:03}")).collect();
        let values: ArrayRef = if views {
            Arc::new(StringViewArray::from_iter_values(text))
        } else {
            Arc::new(StringArray::from(text))
        };
        let keys = UInt8Array::from(ids.iter().map(|id| (id % 200) as u8).collect::<Vec<_>>());
        let names = DictionaryArray::<UInt8Type>::try_new(keys, values).unwrap();
        batch(vec![
            ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
            ("name", Arc::new(names) as ArrayRef),
        ])
    }

    #[test]
    fn compacting_rows_that_alternate_between_dictionaries_writes_few_batches() {
        for views in [false, true] {
            let mut t = LiveTable::new(coded(vec![], "a", views).schema(), "id").unwrap();
            t.upsert(coded((0..1_000).collect(), "a", views)).unwrap();
            // Every odd row moves to another dictionary: live rows alternate between batches.
            t.upsert(coded(
                (0..1_000).filter(|i| i % 2 == 1).collect(),
                "b",
                views,
            ))
            .unwrap();
            for _ in 0..2 {
                t.compact().unwrap();
                // The live rows use 200 distinct names (even keys of one dictionary, odd of the
                // other), which a u8 key indexes: one batch, though the inputs hold 400.
                assert_eq!(t.batches().len(), 1);
                for id in [0, 1, 998, 999] {
                    let at = t.find(&Scalar::Int(id)).unwrap();
                    let column = t.batches()[at.batch].column(1);
                    let names = column.as_any_dictionary();
                    let key = names.normalized_keys()[at.row];
                    let values = names.values();
                    let name = match values.as_string_view_opt() {
                        Some(v) => v.value(key).to_owned(),
                        None => values.as_string::<i32>().value(key).to_owned(),
                    };
                    let prefix = if id % 2 == 1 { "b" } else { "a" };
                    assert_eq!(name, format!("{prefix}{:03}", id % 200));
                }
            }
        }
    }

    #[test]
    fn compaction_refuses_rather_than_writing_batches_of_a_few_rows() {
        // Arrow sizes a dictionary of binary views by all of its inputs' values: 400 here,
        // whichever rows are read, so no split helps. Below 128 rows compaction gives up.
        let coded_binary = |ids: Vec<i64>, prefix: u8| {
            let values = BinaryViewArray::from_iter_values((0..200u8).map(|i| [prefix, i]));
            let keys = UInt8Array::from(ids.iter().map(|id| (id % 200) as u8).collect::<Vec<_>>());
            let names = DictionaryArray::<UInt8Type>::try_new(keys, Arc::new(values)).unwrap();
            batch(vec![
                ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
                ("name", Arc::new(names) as ArrayRef),
            ])
        };
        let options = TableOptions::default().with_auto_compact(false);
        let schema = coded_binary(vec![], 0).schema();
        let mut t = LiveTable::with_options(schema, "id", options).unwrap();
        t.upsert(coded_binary((0..1_000).collect(), 0)).unwrap();
        t.upsert(coded_binary((0..1_000).filter(|i| i % 2 == 1).collect(), 1))
            .unwrap();
        assert!(matches!(t.compact(), Err(DataError::Arrow(_))));
        assert_eq!(
            (t.generation(), t.batches().len()),
            (0, 2),
            "nothing changed"
        );
    }

    #[test]
    fn compaction_splits_rows_a_narrow_dictionary_key_cannot_index_in_one_batch() {
        for views in [false, true] {
            let mut t = LiveTable::new(names(0, views).schema(), "id").unwrap();
            let sorted = t
                .add_view(ViewSpec::new().sort_by("name", Order::Descending))
                .unwrap();
            t.upsert(names(0, views)).unwrap();
            t.upsert(names(200, views)).unwrap();
            // 400 distinct names: no u8-keyed dictionary holds them all.
            t.compact().unwrap();
            assert_eq!(t.generation(), 1);
            assert!(t.batches().len() >= 2);
            let name = |at: RowRef| {
                let column = t.batches()[at.batch].column(1);
                let names = column.as_any_dictionary();
                let values = names.values();
                let key = names.normalized_keys()[at.row];
                match values.as_string_view_opt() {
                    Some(v) => v.value(key).to_owned(),
                    None => values.as_string::<i32>().value(key).to_owned(),
                }
            };
            assert_eq!(name(t.find(&Scalar::Int(399)).unwrap()), "n399");
            assert_eq!(name(t.find(&Scalar::Int(7)).unwrap()), "n007");
            let view = t.view(sorted).unwrap();
            assert_eq!(view.len(), 400);
            assert_eq!(name(view.row(0).unwrap()), "n399");
            assert_eq!(name(view.row(399).unwrap()), "n000");
        }
    }
}

mod views {
    use super::*;

    /// The prices a view shows, in order.
    fn shown(t: &LiveTable, id: densa_data::ViewId) -> Vec<f64> {
        let view = t.view(id).unwrap();
        (0..view.len())
            .map(|i| price(t, view.row(i).unwrap()))
            .collect()
    }

    #[test]
    fn a_rejected_upsert_leaves_views_as_they_were() {
        let mut t = table();
        t.upsert(prices(vec![1, 2], vec![2.0, 1.0])).unwrap();
        let id = t
            .add_view(ViewSpec::new().sort_by("price", Order::Ascending))
            .unwrap();
        let bad = RecordBatch::try_from_iter([
            (
                "id",
                Arc::new(Int64Array::from(vec![Some(3), None])) as ArrayRef,
            ),
            (
                "price",
                Arc::new(Float64Array::from(vec![0.5, 0.25])) as ArrayRef,
            ),
        ])
        .unwrap();
        assert_eq!(t.upsert(bad), Err(DataError::NullKey { row: 1 }));
        assert_eq!(shown(&t, id), [1.0, 2.0]);
    }

    #[test]
    fn a_deleted_key_s_id_is_reused_without_disturbing_views() {
        let mut t = table();
        t.upsert(prices(vec![1, 2, 3], vec![3.0, 1.0, 2.0]))
            .unwrap();
        let id = t
            .add_view(ViewSpec::new().sort_by("price", Order::Ascending))
            .unwrap();
        t.delete(&Int64Array::from(vec![2])).unwrap();
        t.upsert(prices(vec![4], vec![2.5])).unwrap();
        assert_eq!(shown(&t, id), [2.0, 2.5, 3.0]);
        let late = t
            .add_view(ViewSpec::new().sort_by("price", Order::Descending))
            .unwrap();
        assert_eq!(shown(&t, late), [3.0, 2.5, 2.0]);
    }

    #[test]
    fn view_ids_belong_to_their_table_and_its_clones() {
        let mut a = table();
        let id = a.add_view(ViewSpec::new()).unwrap();
        let clone = a.clone();
        assert!(clone.view(id).is_some(), "a clone copies its views");
        assert_ne!(clone.instance(), a.instance());
        let mut b = table();
        assert!(b.view(id).is_none());
        assert!(!b.remove_view(id));
    }

    #[test]
    fn views_name_existing_columns_of_supported_types_with_matching_literals() {
        let mut t = table();
        assert_eq!(
            t.add_view(ViewSpec::new().sort_by("nope", Order::Ascending)),
            Err(DataError::UnknownColumn("nope".into()))
        );
        let filter = Filter::compare("price", CompareOp::Eq, Scalar::Str("x".into()));
        assert_eq!(
            t.add_view(ViewSpec::new().filter(filter)),
            Err(DataError::LiteralMismatch("price".into()))
        );

        let list = ListArray::from_iter_primitive::<Int32Type, _, _>(vec![Some(vec![Some(1)])]);
        let with_list = batch(vec![
            ("id", Arc::new(Int64Array::from(vec![1])) as ArrayRef),
            ("tags", Arc::new(list) as ArrayRef),
        ]);
        let mut t = LiveTable::new(with_list.schema(), "id").unwrap();
        assert_eq!(
            t.add_view(ViewSpec::new().sort_by("tags", Order::Ascending)),
            Err(DataError::UnsupportedColumn("tags".into()))
        );
    }

    #[test]
    fn a_new_view_shows_the_rows_already_in_the_table() {
        let mut t = table();
        t.upsert(prices(vec![1, 2, 3], vec![30.0, 10.0, 20.0]))
            .unwrap();
        let id = t
            .add_view(ViewSpec::new().sort_by("price", Order::Descending))
            .unwrap();
        let view = t.view(id).unwrap();
        let shown: Vec<f64> = (0..view.len())
            .map(|i| price(&t, view.row(i).unwrap()))
            .collect();
        assert_eq!(shown, [30.0, 20.0, 10.0]);
        assert_eq!(view.row(3), None);
    }

    #[test]
    fn a_removed_view_is_gone() {
        let mut t = table();
        let id = t.add_view(ViewSpec::new()).unwrap();
        assert!(t.remove_view(id));
        assert!(t.view(id).is_none());
        assert!(!t.remove_view(id));
    }

    #[test]
    fn decimals_timestamps_and_dictionary_strings_sort_and_filter() {
        let regions: DictionaryArray<Int32Type> =
            vec!["west", "east", "north"].into_iter().collect();
        let totals = Decimal128Array::from(vec![500, 150, 999])
            .with_precision_and_scale(10, 2)
            .unwrap();
        let rows = batch(vec![
            ("id", Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef),
            ("region", Arc::new(regions) as ArrayRef),
            ("total", Arc::new(totals) as ArrayRef),
            (
                "at",
                Arc::new(TimestampMillisecondArray::from(vec![30, 10, 20])) as ArrayRef,
            ),
        ]);
        let mut t = LiveTable::new(rows.schema(), "id").unwrap();
        t.upsert(rows).unwrap();
        let ids = |t: &LiveTable, spec: ViewSpec| -> Vec<i64> {
            let mut t2 = LiveTable::new(t.schema().clone(), "id").unwrap();
            for b in t.batches() {
                t2.upsert(b.clone()).unwrap();
            }
            let id = t2.add_view(spec).unwrap();
            let view = t2.view(id).unwrap();
            (0..view.len())
                .map(|i| {
                    let at = view.row(i).unwrap();
                    let column = t2.batches()[at.batch].column(0);
                    column
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .unwrap()
                        .value(at.row)
                })
                .collect()
        };
        assert_eq!(
            ids(&t, ViewSpec::new().sort_by("region", Order::Ascending)),
            [2, 3, 1]
        );
        assert_eq!(
            ids(&t, ViewSpec::new().sort_by("at", Order::Ascending)),
            [2, 3, 1]
        );
        // 5.00 and 9.99 are above 4.5; 1.50 is not.
        let above = Filter::compare("total", CompareOp::Gt, Scalar::Float(4.5));
        assert_eq!(
            ids(
                &t,
                ViewSpec::new()
                    .filter(above)
                    .sort_by("total", Order::Ascending)
            ),
            [1, 3]
        );
        let exact = Filter::compare("total", CompareOp::Eq, Scalar::Int(5));
        assert_eq!(ids(&t, ViewSpec::new().filter(exact)), [1]);
    }
}
