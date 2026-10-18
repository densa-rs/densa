//! Formatting Arrow values into grid cells, multi-batch row lookup, schema checks, columns.

use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{
    ArrayRef, BooleanArray, Date32Array, Decimal128Array, DictionaryArray,
    DurationMillisecondArray, Float64Array, Int64Array, LargeStringArray, RecordBatch, StringArray,
    StringViewArray, Time64MicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    UInt8Array,
};
use arrow_schema::{DataType, Field, Schema};
use densa_arrow::{ArrowTable, CellState, TableError};
use densa_grid::{Align, CellText, GridSource, Palette};

fn batch(columns: Vec<(&str, ArrayRef)>) -> RecordBatch {
    RecordBatch::try_from_iter(columns).unwrap()
}

/// The text of every row of column 0 in a one-column table.
fn texts(array: ArrayRef) -> Vec<String> {
    let table = ArrowTable::from_batches([batch(vec![("c", array)])]).unwrap();
    (0..table.rows())
        .map(|row| {
            let mut text = CellText::new();
            table.write_cell(row, 0, &mut text);
            text.as_str().to_owned()
        })
        .collect()
}

mod values {
    use super::*;

    #[test]
    fn integers_and_unsigned_integers() {
        assert_eq!(
            texts(Arc::new(Int64Array::from(vec![0, -42, i64::MAX]))),
            ["0", "-42", "9223372036854775807"]
        );
        assert_eq!(texts(Arc::new(UInt8Array::from(vec![255]))), ["255"]);
    }

    #[test]
    fn floats_print_their_shortest_exact_form_unless_decimals_are_set() {
        let array: ArrayRef = Arc::new(Float64Array::from(vec![1.5, 0.1, -2.0]));
        assert_eq!(texts(Arc::clone(&array)), ["1.5", "0.1", "-2"]);

        let mut table = ArrowTable::from_batches([batch(vec![("c", array)])]).unwrap();
        table.set_decimals(0, Some(2)).unwrap();
        let mut text = CellText::new();
        table.write_cell(2, 0, &mut text);
        assert_eq!(text.as_str(), "-2.00");
    }

    #[test]
    fn decimals_keep_their_scale() {
        let array = Decimal128Array::from(vec![12345, -5, 0])
            .with_precision_and_scale(10, 2)
            .unwrap();
        assert_eq!(texts(Arc::new(array)), ["123.45", "-0.05", "0.00"]);
        let integral = Decimal128Array::from(vec![7])
            .with_precision_and_scale(10, 0)
            .unwrap();
        assert_eq!(texts(Arc::new(integral)), ["7"]);
        let whole = Decimal128Array::from(vec![7])
            .with_precision_and_scale(10, -2)
            .unwrap();
        assert_eq!(texts(Arc::new(whole)), ["700"]);
    }

    #[test]
    fn booleans_and_every_string_layout() {
        assert_eq!(
            texts(Arc::new(BooleanArray::from(vec![true, false]))),
            ["true", "false"]
        );
        assert_eq!(
            texts(Arc::new(StringArray::from(vec!["a", "bé"]))),
            ["a", "bé"]
        );
        assert_eq!(
            texts(Arc::new(LargeStringArray::from(vec!["large"]))),
            ["large"]
        );
        assert_eq!(
            texts(Arc::new(StringViewArray::from(vec!["view"]))),
            ["view"]
        );
    }

    #[test]
    fn dictionary_strings_print_their_values() {
        let array: DictionaryArray<Int32Type> =
            vec!["north", "south", "north"].into_iter().collect();
        assert_eq!(texts(Arc::new(array)), ["north", "south", "north"]);
    }

    #[test]
    fn dates_times_and_timestamps_in_iso_form() {
        assert_eq!(
            texts(Arc::new(Date32Array::from(vec![0, 19_000, -1]))),
            ["1970-01-01", "2022-01-08", "1969-12-31"]
        );
        let ms = TimestampMillisecondArray::from(vec![1_700_000_000_123]);
        assert_eq!(texts(Arc::new(ms)), ["2023-11-14 22:13:20.123"]);
        let utc = TimestampNanosecondArray::from(vec![-1]).with_timezone("UTC");
        assert_eq!(texts(Arc::new(utc)), ["1969-12-31 23:59:59.999999999Z"]);
        let time = Time64MicrosecondArray::from(vec![3_723_000_004]);
        assert_eq!(texts(Arc::new(time)), ["01:02:03.000004"]);
        let duration = DurationMillisecondArray::from(vec![1500]);
        assert_eq!(texts(Arc::new(duration)), ["1500ms"]);
    }

    #[test]
    fn dates_before_the_epoch_and_coarse_times() {
        let ms = arrow_array::Date64Array::from(vec![-1, 86_400_000]);
        assert_eq!(texts(Arc::new(ms)), ["1969-12-31", "1970-01-02"]);
        let seconds = arrow_array::Time32SecondArray::from(vec![86_399]);
        assert_eq!(texts(Arc::new(seconds)), ["23:59:59"]);
        let utc = arrow_array::TimestampSecondArray::from(vec![0]).with_timezone("+00:00");
        assert_eq!(texts(Arc::new(utc)), ["1970-01-01 00:00:00Z"]);
        let nanos = arrow_array::DurationNanosecondArray::from(vec![-7]);
        assert_eq!(texts(Arc::new(nanos)), ["-7ns"]);
    }

    #[test]
    fn a_dictionary_entry_with_a_null_value_is_null() {
        let values = StringArray::from(vec![Some("x"), None]);
        let keys = arrow_array::Int32Array::from(vec![0, 1]);
        let array = DictionaryArray::<Int32Type>::try_new(keys, Arc::new(values)).unwrap();
        let table =
            ArrowTable::from_batches([batch(vec![("c", Arc::new(array) as ArrayRef)])]).unwrap();
        let mut text = CellText::new();
        assert_eq!(table.write_cell(0, 0, &mut text), CellState::Value);
        text.clear();
        assert_eq!(table.write_cell(1, 0, &mut text), CellState::Null);
    }

    #[test]
    fn nulls_print_as_null_and_report_it() {
        let table = ArrowTable::from_batches([batch(vec![(
            "c",
            Arc::new(Int64Array::from(vec![Some(1), None])) as ArrayRef,
        )])])
        .unwrap();
        let mut text = CellText::new();
        assert_eq!(table.write_cell(0, 0, &mut text), CellState::Value);
        text.clear();
        assert_eq!(table.write_cell(1, 0, &mut text), CellState::Null);
        assert_eq!(text.as_str(), "null");
    }

    #[test]
    fn values_a_cell_cannot_show_become_a_hash_never_a_partial_value() {
        let floats: ArrayRef = Arc::new(Float64Array::from(vec![1e300, f64::NAN, f64::INFINITY]));
        let mut table = ArrowTable::from_batches([batch(vec![("c", floats)])]).unwrap();
        let mut text = CellText::new();
        assert_eq!(table.write_cell(0, 0, &mut text), CellState::Invalid);
        assert_eq!(text.as_str(), "#");

        table.set_decimals(0, Some(2)).unwrap();
        text.clear();
        assert_eq!(table.write_cell(1, 0, &mut text), CellState::Value);
        assert_eq!(text.as_str(), "NaN");
        text.clear();
        table.write_cell(2, 0, &mut text);
        assert_eq!(text.as_str(), "inf");
    }

    #[test]
    fn a_time_of_day_outside_a_day_is_invalid() {
        let time = arrow_array::Time32SecondArray::from(vec![-1, 86_400]);
        assert_eq!(texts(Arc::new(time)), ["#", "#"]);
    }

    #[test]
    fn zero_with_a_negative_decimal_scale_is_plain_zero() {
        let array = Decimal128Array::from(vec![0])
            .with_precision_and_scale(10, -3)
            .unwrap();
        assert_eq!(texts(Arc::new(array)), ["0"]);
    }

    #[test]
    fn years_before_year_zero_keep_four_digits_after_the_sign() {
        let date = Date32Array::from(vec![-719_529]); // the day before 0000-01-01
        assert_eq!(texts(Arc::new(date)), ["-0001-12-31"]);
    }

    #[test]
    fn a_null_typed_column_shows_null() {
        let table = ArrowTable::from_batches([batch(vec![(
            "c",
            Arc::new(arrow_array::NullArray::new(2)) as ArrayRef,
        )])])
        .unwrap();
        let mut text = CellText::new();
        assert_eq!(table.write_cell(1, 0, &mut text), CellState::Null);
    }

    #[test]
    fn unsupported_types_print_a_marker() {
        let list = arrow_array::ListArray::from_iter_primitive::<arrow_array::types::Int32Type, _, _>(
            vec![Some(vec![Some(1)])],
        );
        let table =
            ArrowTable::from_batches([batch(vec![("c", Arc::new(list) as ArrayRef)])]).unwrap();
        let mut text = CellText::new();
        assert_eq!(table.write_cell(0, 0, &mut text), CellState::Unsupported);
        assert_eq!(text.as_str(), "?");
    }

    #[test]
    fn text_wider_than_its_column_is_cut_with_an_ellipsis() {
        let long = "a".repeat(200);
        let mut table = ArrowTable::from_batches([batch(vec![(
            "c",
            Arc::new(StringArray::from(vec![long.as_str(), "short"])) as ArrayRef,
        )])])
        .unwrap();
        table.set_column_chars(0, 6).unwrap();
        let mut text = CellText::new();
        table.write_cell(0, 0, &mut text);
        assert_eq!(text.as_str(), "aaaaa…");
        text.clear();
        table.write_cell(1, 0, &mut text);
        assert_eq!(text.as_str(), "short");
    }
}

mod tables {
    use super::*;

    fn ints(values: Vec<i64>) -> RecordBatch {
        batch(vec![("n", Arc::new(Int64Array::from(values)) as ArrayRef)])
    }

    #[test]
    fn rows_span_every_batch_in_order() {
        let table =
            ArrowTable::from_batches([ints(vec![1, 2]), ints(vec![]), ints(vec![3])]).unwrap();
        assert_eq!(table.rows(), 3);
        let mut text = CellText::new();
        table.write_cell(2, 0, &mut text);
        assert_eq!(text.as_str(), "3");
    }

    #[test]
    fn batches_can_be_appended_while_streaming() {
        let schema = ints(vec![]).schema();
        let mut table = ArrowTable::new(schema);
        assert_eq!(table.rows(), 0);
        table.push(ints(vec![10])).unwrap();
        table.push(ints(vec![20, 30])).unwrap();
        assert_eq!(table.rows(), 3);
        let mut text = CellText::new();
        table.write_cell(1, 0, &mut text);
        assert_eq!(text.as_str(), "20");
    }

    #[test]
    fn a_batch_with_another_schema_is_rejected() {
        let mut table = ArrowTable::from_batches([ints(vec![1])]).unwrap();
        let other = batch(vec![(
            "n",
            Arc::new(StringArray::from(vec!["x"])) as ArrayRef,
        )]);
        assert_eq!(
            table.push(other),
            Err(TableError::ColumnMismatch { column: 0 })
        );
        let wider = batch(vec![
            ("n", Arc::new(Int64Array::from(vec![1])) as ArrayRef),
            ("m", Arc::new(Int64Array::from(vec![2])) as ArrayRef),
        ]);
        assert_eq!(
            table.push(wider),
            Err(TableError::ColumnCount {
                expected: 1,
                found: 2
            })
        );
        assert_eq!(table.rows(), 1);
    }

    #[test]
    fn a_width_set_before_the_first_rows_is_kept() {
        let mut table = ArrowTable::new(ints(vec![]).schema());
        table.set_column_chars(0, 12).unwrap();
        table.push(ints(vec![1])).unwrap();
        assert_eq!(table.columns()[0].chars, 12);
    }

    #[test]
    fn widths_are_clamped_to_what_a_cell_can_hold() {
        let mut table = ArrowTable::from_batches([ints(vec![1])]).unwrap();
        table.set_column_chars(0, usize::MAX).unwrap();
        assert_eq!(table.columns()[0].chars as usize, densa_grid::CELL_BYTES);
        table.set_column_chars(0, 0).unwrap();
        assert_eq!(table.columns()[0].chars, 1);
    }

    #[test]
    fn a_view_scrolled_past_any_row_shows_nothing_without_overflowing() {
        let table = ArrowTable::from_batches([ints(vec![1])]).unwrap();
        let view = table.view(usize::MAX, &Palette::new());
        let mut text = CellText::new();
        view.cell(5, 0, &mut text);
        assert!(text.is_empty());
    }

    #[test]
    fn a_table_needs_a_batch_to_know_its_schema() {
        let none: [RecordBatch; 0] = [];
        assert_eq!(
            ArrowTable::from_batches(none).unwrap_err(),
            TableError::NoBatches
        );
    }

    #[test]
    fn columns_are_titled_by_field_sized_by_content_and_numbers_align_right() {
        let table = ArrowTable::from_batches([batch(vec![
            (
                "id",
                Arc::new(Int64Array::from(vec![1, 123_456])) as ArrayRef,
            ),
            (
                "name",
                Arc::new(StringArray::from(vec!["a", "abc"])) as ArrayRef,
            ),
        ])])
        .unwrap();
        let columns = table.columns();
        assert_eq!(columns[0].title, "id");
        assert_eq!(columns[0].chars, 6);
        assert_eq!(columns[0].align, Align::Right);
        assert_eq!(columns[1].title, "name");
        assert_eq!(columns[1].chars, 4);
        assert_eq!(columns[1].align, Align::Left);
    }

    #[test]
    fn numbers_dates_and_times_align_right_text_and_booleans_left() {
        let field = |name: &str, data_type: DataType| Field::new(name, data_type, true);
        let dictionary = DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8));
        let schema = Arc::new(Schema::new(vec![
            field("f", DataType::Float32),
            field("d", DataType::Decimal128(10, 2)),
            field(
                "t",
                DataType::Timestamp(arrow_schema::TimeUnit::Second, None),
            ),
            field("day", DataType::Date32),
            field("u", DataType::UInt16),
            field("s", DataType::Utf8),
            field("b", DataType::Boolean),
            field("k", dictionary),
        ]));
        let aligns: Vec<Align> = ArrowTable::new(schema)
            .columns()
            .iter()
            .map(|c| c.align)
            .collect();
        use Align::{Left, Right};
        assert_eq!(
            aligns,
            [Right, Right, Right, Right, Right, Left, Left, Left]
        );
    }

    #[test]
    fn settings_for_a_missing_column_are_errors() {
        let mut table = ArrowTable::from_batches([ints(vec![1])]).unwrap();
        assert_eq!(
            table.set_decimals(3, None),
            Err(TableError::NoSuchColumn(3))
        );
        assert_eq!(
            table.set_column_chars(3, 4),
            Err(TableError::NoSuchColumn(3))
        );
        let mut text = CellText::new();
        assert_eq!(table.write_cell(0, 3, &mut text), CellState::OutOfRange);
        assert_eq!(table.write_cell(5, 0, &mut text), CellState::OutOfRange);
        assert!(text.is_empty());
    }

    #[test]
    fn the_first_rows_size_the_columns_and_later_batches_do_not_move_them() {
        let strings =
            |values: Vec<&str>| batch(vec![("c", Arc::new(StringArray::from(values)) as ArrayRef)]);
        let mut table =
            ArrowTable::from_batches([strings(vec![]), strings(vec!["héllo"])]).unwrap();
        assert_eq!(
            table.columns()[0].chars,
            5,
            "sized by characters, from the first rows"
        );
        table.push(strings(vec!["a much longer value"])).unwrap();
        assert_eq!(table.columns()[0].chars, 5);
    }

    #[test]
    fn an_empty_table_sizes_columns_by_title_and_type() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Int32, true),
            Field::new("long title", DataType::Utf8, true),
        ]));
        let table = ArrowTable::new(schema);
        let columns = table.columns();
        assert!(columns[0].chars >= 1);
        assert_eq!(columns[1].chars, 10);
    }

    #[test]
    fn a_view_starts_at_its_first_row_and_colors_nulls() {
        let table = ArrowTable::from_batches([batch(vec![(
            "n",
            Arc::new(Int64Array::from(vec![Some(1), Some(2), None])) as ArrayRef,
        )])])
        .unwrap();
        let palette = Palette::new();
        let view = table.view(1, &palette);
        assert_eq!(view.rows(), 2);
        let mut text = CellText::new();
        let style = view.cell(0, 0, &mut text);
        assert_eq!(text.as_str(), "2");
        assert_eq!(style.color, palette.text);
        text.clear();
        let style = view.cell(1, 0, &mut text);
        assert_eq!(style.color, palette.dim_text);
        assert_eq!(table.view(10, &palette).rows(), 0);
    }
}
