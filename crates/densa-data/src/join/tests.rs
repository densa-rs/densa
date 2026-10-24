use super::*;
use arrow_array::{Int64Array, StringArray};
use arrow_schema::DataType;

fn schemas() -> (SchemaRef, SchemaRef) {
    let left = Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("sym", DataType::Utf8, true),
    ]);
    let right = Schema::new(vec![
        Field::new("sym", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, true),
    ]);
    (Arc::new(left), Arc::new(right))
}

#[test]
fn the_index_stays_within_twice_the_table_as_rows_turn_over() {
    let (left, right) = schemas();
    let mut join = LookupJoin::new(Arc::clone(&left), "id", "sym", right, "sym").unwrap();
    let mut out = join.table().unwrap();
    let mut rebuilt = false;
    for i in 0..20_000i64 {
        // Each new row reuses the id of the row that just left, naming another key.
        let sym = format!("S{}", i % 997);
        let rows = RecordBatch::try_new(
            Arc::clone(&left),
            vec![
                Arc::new(Int64Array::from(vec![i])),
                Arc::new(StringArray::from(vec![sym])),
            ],
        )
        .unwrap();
        join.upsert_left(&mut out, rows).unwrap();
        if i >= 10 {
            out.delete(&Int64Array::from(vec![i - 10])).unwrap();
        }
        let bound = MIN_REBUILD.max(out.len() * 2) + 1;
        assert!(join.entries <= bound, "{} entries at {i}", join.entries);
        let counted: usize = join.refs.values().map(FxHashSet::len).sum();
        assert_eq!(counted, join.entries, "the count is the index's size");
        rebuilt |= join.entries < 20;
    }
    assert!(rebuilt, "the index was rebuilt along the way");
}

#[test]
fn entries_found_stale_while_joining_again_leave_the_index() {
    let (left, right) = schemas();
    let mut join =
        LookupJoin::new(Arc::clone(&left), "id", "sym", Arc::clone(&right), "sym").unwrap();
    let mut out = join.table().unwrap();
    let rows = |id: i64, sym: &str| {
        RecordBatch::try_new(
            Arc::clone(&left),
            vec![
                Arc::new(Int64Array::from(vec![id])) as ArrayRef,
                Arc::new(StringArray::from(vec![sym])),
            ],
        )
        .unwrap()
    };
    join.upsert_left(&mut out, rows(1, "A")).unwrap();
    join.upsert_left(&mut out, rows(1, "B")).unwrap();
    assert_eq!(join.entries, 2, "A's entry is stale, kept until A is used");
    let names = RecordBatch::try_new(
        Arc::clone(&right),
        vec![
            Arc::new(StringArray::from(vec!["A"])) as ArrayRef,
            Arc::new(StringArray::from(vec!["Alpha"])),
        ],
    )
    .unwrap();
    join.upsert_right(&mut out, names).unwrap();
    assert_eq!(join.entries, 1, "found stale and dropped");
    assert!(
        !join.refs.contains_key(&Owned::Str("A".into())),
        "its emptied bucket too"
    );
}
