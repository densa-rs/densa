//! Views: the live rows that pass a filter, in a sort order, kept up to date as rows change.

use std::cmp::Ordering;

use arrow_array::RecordBatch;
use arrow_schema::Schema;

use crate::filter::{resolve, Filter, Predicate};
use crate::sorted::{Entry, SortedRows};
use crate::table::Slot;
use crate::value::{compare, prefix, ValueFn};
use crate::{DataError, RowId, RowRef};

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Order {
    /// Smallest first.
    #[default]
    Ascending,
    /// Largest first.
    Descending,
}

/// What a view shows: an optional filter and sort columns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewSpec {
    sort: Vec<(String, Order)>,
    filter: Option<Filter>,
}

impl ViewSpec {
    /// Every row, in no particular order (stable while rows keep their keys).
    pub fn new() -> Self {
        Self::default()
    }

    /// Sorts by `column` in `order`, after the sort columns given before it. Nulls come last
    /// either way; floats sort `-inf`, numbers, `inf`, then NaN, with `-0.0` equal to `0.0`.
    #[must_use]
    pub fn sort_by(mut self, column: impl Into<String>, order: Order) -> Self {
        self.sort.push((column.into(), order));
        self
    }

    /// Shows only rows for which `filter` is true.
    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = Some(filter);
        self
    }
}

/// A sort column bound to its index and reader.
#[derive(Debug, Clone)]
pub(crate) struct SortColumn {
    column: usize,
    read: ValueFn,
    descending: bool,
}

impl SortColumn {
    pub(crate) fn ascending(column: usize, read: ValueFn) -> Self {
        Self {
            column,
            read,
            descending: false,
        }
    }
}

/// A view's compiled spec and its rows.
#[derive(Debug, Clone)]
pub(crate) struct ViewState {
    sort: Vec<SortColumn>,
    filter: Option<Predicate>,
    rows: SortedRows,
    /// Whether each row id is in the view.
    member: Vec<bool>,
}

/// The tables views and groups read from: its batches and where each row id lives.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rows<'a> {
    pub(crate) batches: &'a [RecordBatch],
    pub(crate) slots: &'a [Option<Slot>],
}

impl Rows<'_> {
    pub(crate) fn at(&self, id: RowId) -> RowRef {
        match self.slots.get(id as usize) {
            Some(Some(slot)) => slot.row_ref(),
            // Views and groups drop a row before its slot is freed, so every id they hold is
            // live.
            _ => unreachable!("row id {id} is held, but its slot is free"),
        }
    }
}

impl ViewState {
    pub(crate) fn compile(spec: &ViewSpec, schema: &Schema) -> Result<Self, DataError> {
        let sort = spec
            .sort
            .iter()
            .map(|(name, order)| {
                let (column, _, read) = resolve(schema, name)?;
                Ok(SortColumn {
                    column,
                    read,
                    descending: *order == Order::Descending,
                })
            })
            .collect::<Result<_, DataError>>()?;
        let filter = spec
            .filter
            .as_ref()
            .map(|f| Predicate::compile(f, schema))
            .transpose()?;
        Ok(Self {
            sort,
            filter,
            rows: SortedRows::default(),
            member: Vec::new(),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn get(&self, index: usize) -> Option<RowId> {
        self.rows.get(index)
    }

    /// Whether the row at `at` passes the filter (unknown does not).
    fn passes(&self, batches: &[RecordBatch], at: RowRef) -> bool {
        self.filter
            .as_ref()
            .is_none_or(|f| f.eval(&batches[at.batch], at.row) == Some(true))
    }

    /// Drops row `id`, whose values are at `at` (where they were when it was inserted).
    pub(crate) fn remove(&mut self, rows: Rows<'_>, id: RowId, at: RowRef) {
        if !self.member.get(id as usize).copied().unwrap_or(false) {
            return;
        }
        let probe = entry(&self.sort, rows.batches, id, at);
        let sort = &self.sort;
        let found = self
            .rows
            .remove(id, |other| entry_order(sort, rows, other, probe, at));
        debug_assert!(found, "a member row is in the sorted rows");
        self.member[id as usize] = false;
    }

    /// Adds row `id` at `at` if it passes the filter.
    pub(crate) fn insert(&mut self, rows: Rows<'_>, id: RowId, at: RowRef) {
        if !self.passes(rows.batches, at) {
            return;
        }
        let probe = entry(&self.sort, rows.batches, id, at);
        let sort = &self.sort;
        self.rows
            .insert(probe, |other| entry_order(sort, rows, other, probe, at));
        if self.member.len() <= id as usize {
            self.member.resize(id as usize + 1, false);
        }
        self.member[id as usize] = true;
    }

    /// Recomputes the view from every live row.
    pub(crate) fn rebuild(&mut self, rows: Rows<'_>) {
        let mut entries: Vec<Entry> = rows
            .slots
            .iter()
            .enumerate()
            .filter_map(|(id, slot)| Some((RowId::try_from(id).ok()?, slot.as_ref()?.row_ref())))
            .filter(|&(_, at)| self.passes(rows.batches, at))
            .map(|(id, at)| entry(&self.sort, rows.batches, id, at))
            .collect();
        entries.sort_unstable_by(|&a, &b| entry_order(&self.sort, rows, a, b, rows.at(b.id)));
        self.member.clear();
        self.member.resize(rows.slots.len(), false);
        for entry in &entries {
            self.member[entry.id as usize] = true;
        }
        self.rows.rebuild(&entries);
    }
}

/// Row `id`'s view entry: its id and the prefix of its first sort column (0 if unsorted).
pub(crate) fn entry(sort: &[SortColumn], batches: &[RecordBatch], id: RowId, at: RowRef) -> Entry {
    let key = sort.first().map_or(0, |first| {
        let value = (first.read)(batches[at.batch].column(first.column).as_ref(), at.row);
        prefix(value, first.descending)
    });
    Entry { key, id }
}

/// Orders entry `other` (a stored row) against `probe` (whose values are at `probe_at`):
/// by prefix, reading the rows' values only when prefixes tie.
pub(crate) fn entry_order(
    sort: &[SortColumn],
    rows: Rows<'_>,
    other: Entry,
    probe: Entry,
    probe_at: RowRef,
) -> Ordering {
    other.key.cmp(&probe.key).then_with(|| {
        row_order(
            sort,
            rows.batches,
            (other.id, rows.at(other.id)),
            (probe.id, probe_at),
        )
    })
}

/// Orders row `a` against row `b` (each an id and where its values are): by the sort
/// columns, then by id, so the order is total.
fn row_order(
    sort: &[SortColumn],
    batches: &[RecordBatch],
    a: (RowId, RowRef),
    b: (RowId, RowRef),
) -> Ordering {
    for key in sort {
        let read = |at: RowRef| (key.read)(batches[at.batch].column(key.column).as_ref(), at.row);
        let order = compare(read(a.1), read(b.1), key.descending);
        if order.is_ne() {
            return order;
        }
    }
    a.0.cmp(&b.0)
}
