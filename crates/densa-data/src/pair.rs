//! Pair joins: each row of one table with each row of another holding the same value in their
//! join columns (a customer's orders with each of the customer's addresses, a symbol's trades
//! with each of its alerts), kept up to date as either side changes.

use std::sync::Arc;

use arrow_array::{new_null_array, Array, ArrayRef, RecordBatch, UInt64Array};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::join::{check_columns, nullable, Owned, Places};
use crate::key::{key_reader, KeyFn, KeyIndex, KeyRef};
use crate::{Changes, DataError, LiveTable, RowId, Window};

/// The right half of the pair id of a left row that pairs with no right row.
const NO_RIGHT: RowId = RowId::MAX;

/// Pairs written to the output in one batch when it is filled whole.
const FILL_CHUNK: usize = 1 << 16;

/// The capacity above which the rows holding a value are shrunk when a quarter of it is used.
const SHRINK_FROM: usize = 64;

/// The id of the output row pairing left row `left` with right row `right` (both row ids of
/// the join's own tables): stable while both rows live.
fn pair_id(left: RowId, right: RowId) -> u64 {
    (u64::from(left) << 32) | u64::from(right)
}

/// The rows of pair id `id`.
fn rows_of(id: u64) -> (RowId, RowId) {
    // Lossless: the halves of a pair id are row ids.
    ((id >> 32) as RowId, id as RowId)
}

/// One side of a pair join: its rows, kept by key, and which of them hold each value of its
/// join column.
#[derive(Debug)]
struct Side {
    table: LiveTable,
    /// The join column, and how its values are read (`None` for a type that cannot be joined
    /// on: refused when the join is made).
    column: usize,
    read: Option<KeyFn>,
    /// The rows (ids in `table`) holding each value; a row holding a null holds none.
    holding: FxHashMap<Owned, FxHashSet<RowId>>,
}

impl Side {
    fn new(schema: SchemaRef, key: &str, column: &str) -> Result<Self, DataError> {
        let (at, field) = schema
            .column_with_name(column)
            .ok_or_else(|| DataError::UnknownColumn(column.to_owned()))?;
        let read = key_reader(field.data_type());
        Ok(Self {
            table: LiveTable::new(schema, key)?,
            column: at,
            read,
            holding: FxHashMap::default(),
        })
    }

    /// The join column's type.
    fn column_type(&self) -> &DataType {
        self.table.schema().field(self.column).data_type()
    }

    /// The join value of live row `id`, if it holds one.
    fn value_of(&self, id: RowId) -> Option<Owned> {
        let at = self.table.slot_of(id)?;
        let column = self.table.batches()[at.batch].column(self.column).as_ref();
        (self.read?)(column, at.row).map(Owned::of)
    }

    /// The rows holding `value`.
    fn holding(&self, value: Option<&Owned>) -> impl Iterator<Item = RowId> + '_ {
        value
            .and_then(|value| self.holding.get(value))
            .into_iter()
            .flatten()
            .copied()
    }

    /// Whether any row holds `value`. (Not `holding(value).next()`: a set's iterator walks its
    /// buckets, as many as the most rows that ever held the value; `forget` removes empty sets.)
    fn holds(&self, value: Option<&Owned>) -> bool {
        value.is_some_and(|value| self.holding.contains_key(value))
    }

    /// Notes that row `id` holds `value`.
    fn note(&mut self, id: RowId, value: Option<Owned>) {
        if let Some(value) = value {
            self.holding.entry(value).or_default().insert(id);
        }
    }

    /// Notes that row `id` no longer holds `value`.
    fn forget(&mut self, id: RowId, value: Option<&Owned>) {
        let Some(value) = value else {
            return;
        };
        if let Some(rows) = self.holding.get_mut(value) {
            rows.remove(&id);
            if rows.is_empty() {
                self.holding.remove(value);
            } else if rows.capacity() > SHRINK_FROM && rows.len() * 4 < rows.capacity() {
                // Walking the rows of a value costs its buckets: keep them near its rows.
                rows.shrink_to_fit();
            }
        }
    }

    /// Each distinct key among `keys` (the side's key column of rows to upsert, or keys to
    /// delete), with its live row and join value as they are now.
    fn current(&self, keys: &dyn Array) -> Vec<(Owned, Option<RowId>, Option<Owned>)> {
        let read = key_reader(keys.data_type());
        let mut seen: FxHashSet<KeyRef<'_>> = FxHashSet::default();
        let mut current = Vec::new();
        for row in 0..keys.len() {
            let Some(key) = read.and_then(|read| read(keys, row)) else {
                continue;
            };
            if seen.insert(key) {
                let id = self.table.id_of(key);
                let value = id.and_then(|id| self.value_of(id));
                current.push((Owned::of(key), id, value));
            }
        }
        current
    }
}

/// Joins the rows of two live tables that hold the same value in their join columns: each
/// output row is a pair, a left row followed by the columns of a right row whose `by` column
/// holds what the left row's `on` column does (a customer's orders, each with every address of
/// the customer). A left row that pairs with no right row (none holds its value, or its value
/// is null) is in the output once, with nulls for the right columns: a left outer join. Either
/// join column may be any column of integers or text, a key or not, so a value can pair many
/// left rows with many right rows; to join each row to the one row its key names, use
/// [`LookupJoin`](crate::LookupJoin), which keeps one output row per left row.
///
/// The join keeps both sides' rows (fewer than 4,294,967,295 on each side: a pair id holds two
/// row ids of 32 bits). Rows go in and out through [`upsert_left`](Self::upsert_left),
/// [`delete_left`](Self::delete_left), [`upsert_right`](Self::upsert_right) and
/// [`delete_right`](Self::delete_right), each of which brings the output up to date: the pairs
/// a row leaves are deleted, the pairs it joins upserted (rewritten when its columns change).
/// The output is an ordinary [`LiveTable`] the application owns (made by
/// [`table`](Self::table)), keyed by a pair id (an unsigned integer column, `pair` unless a
/// side has a column of that name: [`pair_column`](Self::pair_column)), so views, groups and
/// grids work on it as on any table; view it with `Filter::is_not_null` on a right column to
/// keep only rows that paired. Give each join a table of its own. To keep only recent rows,
/// window the left rows with [`set_left_window`](Self::set_left_window): the rows it lets go
/// of leave with every pair they are in. A window on the output deletes pairs as on any table,
/// but not the left rows behind them (a pair it deleted comes back only when one of its rows
/// changes, to be deleted again if the window still excludes it). Do not upsert into the
/// output or delete from it otherwise.
///
/// A table this join did not write to last (the first call after [`table`](Self::table), one
/// used before another, or one an earlier call failed to bring up to date) is filled whole at
/// the next call: every pair upserted, rows of pairs that ended deleted, at a cost of every
/// pair. A value held by `m` left rows and `n` right rows makes `m × n` output rows, and a
/// change to one of those rows rewrites the pairs it is in. The join allocates as it works.
///
/// ```
/// use std::sync::Arc;
/// use densa_data::arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
/// use densa_data::PairJoin;
///
/// let orders = |ids: Vec<i64>, customers: Vec<&str>| {
///     RecordBatch::try_from_iter([
///         ("order", Arc::new(Int64Array::from(ids)) as ArrayRef),
///         ("customer", Arc::new(StringArray::from(customers)) as ArrayRef),
///     ])
/// };
/// let addresses = |ids: Vec<i64>, customers: Vec<&str>, cities: Vec<&str>| {
///     RecordBatch::try_from_iter([
///         ("address", Arc::new(Int64Array::from(ids)) as ArrayRef),
///         ("owner", Arc::new(StringArray::from(customers)) as ArrayRef),
///         ("city", Arc::new(StringArray::from(cities)) as ArrayRef),
///     ])
/// };
///
/// // Orders keyed by id, addresses keyed by id, joined where an address's owner is the
/// // order's customer.
/// let left = orders(vec![], vec![])?.schema();
/// let right = addresses(vec![], vec![], vec![])?.schema();
/// let mut join = PairJoin::new(left, "order", "customer", right, "address", "owner")?;
/// let mut paired = join.table()?; // order, customer, address, city, pair
///
/// join.upsert_left(&mut paired, orders(vec![1, 2], vec!["ann", "bob"])?)?;
/// // Ann has two addresses: her order is in the output twice; Bob's once, with nulls.
/// join.upsert_right(&mut paired, addresses(vec![10, 11], vec!["ann", "ann"], vec!["Oslo", "Rome"])?)?;
/// assert_eq!(paired.len(), 3);
/// // Bob gets an address: his null row becomes a pair.
/// join.upsert_right(&mut paired, addresses(vec![12], vec!["bob"], vec!["Lima"])?)?;
/// assert_eq!(paired.len(), 3);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// Not Clone: a copy would bring the same output table up to date from rows it does not see.
#[derive(Debug)]
pub struct PairJoin {
    left: Side,
    right: Side,
    /// The right columns in the output (all but `by`, whose value is in `on`), by index.
    right_columns: Vec<usize>,
    /// One null per left and per right output column, for gathers where a row is missing.
    left_nulls: Vec<ArrayRef>,
    right_nulls: Vec<ArrayRef>,
    schema: SchemaRef,
    /// The name of the pair column, the output's key (its last column).
    pair: String,
    /// The output table this join brought up to date last, by instance.
    filled: Option<u64>,
}

impl PairJoin {
    /// A join of rows of `left`'s columns (keyed by `left_key`) to rows of `right`'s columns
    /// (keyed by `right_key`) where the left column `on` and the right column `by` hold the
    /// same value. The output has the left columns, then the right ones but `by` (its value is
    /// in `on`), then the pair id.
    ///
    /// # Errors
    /// [`DataError::UnknownColumn`] for a key or join column its side does not have,
    /// [`DataError::UnsupportedKey`] for a key column of another type than integers or text,
    /// [`DataError::PairColumns`] unless `on` and `by` are both signed integers, both unsigned
    /// integers or both text (floats, dictionaries and other types are refused),
    /// [`DataError::DuplicateName`] if a left column and a right one other than `by` share a
    /// name.
    pub fn new(
        left: SchemaRef,
        left_key: &str,
        on: &str,
        right: SchemaRef,
        right_key: &str,
        by: &str,
    ) -> Result<Self, DataError> {
        let mut left = Side::new(nullable(fields(&left)), left_key, on)?;
        let right = Side::new(nullable(fields(&right)), right_key, by)?;
        // Keys are read from exactly the types an index keys by: both sides read their column.
        let joinable = KeyIndex::for_type(left.column_type())
            .is_some_and(|kind| kind.same_kind(right.column_type()));
        if !joinable {
            return Err(DataError::PairColumns {
                on: on.to_owned(),
                by: by.to_owned(),
            });
        }
        // Rows a left window lets go of are freed during an upsert: their values are kept so
        // their pairs can be found.
        if let Some(read) = left.read {
            left.table.keep_expired(left.column, read);
        }
        let right_schema = Arc::clone(right.table.schema());
        let right_columns: Vec<usize> = (0..right_schema.fields().len())
            .filter(|&i| i != right.column)
            .collect();
        let mut fields = fields(left.table.schema());
        for &column in &right_columns {
            let field = right_schema.field(column);
            if fields.iter().any(|f| f.name() == field.name()) {
                return Err(DataError::DuplicateName(field.name().clone()));
            }
            fields.push(field.clone());
        }
        let pair = unused_name(&fields, "pair");
        fields.push(Field::new(&pair, DataType::UInt64, false));
        let nulls = |schema: &Schema, columns: &mut dyn Iterator<Item = usize>| {
            columns
                .map(|c| new_null_array(schema.field(c).data_type(), 1))
                .collect::<Vec<_>>()
        };
        let left_schema = Arc::clone(left.table.schema());
        let left_nulls = nulls(&left_schema, &mut (0..left_schema.fields().len()));
        let right_nulls = nulls(&right_schema, &mut right_columns.iter().copied());
        Ok(Self {
            left,
            right,
            right_columns,
            left_nulls,
            right_nulls,
            schema: Arc::new(Schema::new(fields)),
            pair,
            filled: None,
        })
    }

    /// The output's columns: the left ones, then the right ones but `by`, every one nullable,
    /// then the pair id.
    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// The name of the pair column, the output's key: `pair`, or `pair_`, `pair__`, ... if a
    /// side has a column of that name.
    pub fn pair_column(&self) -> &str {
        &self.pair
    }

    /// An empty output table, keyed by the pair id: give it to every call of this join. Add
    /// views, groups or a window to it as to any table, or hand it to a grid.
    ///
    /// # Errors
    /// None in practice: the columns were checked when the join was made (a table is made
    /// anew, which checks them again).
    pub fn table(&self) -> Result<LiveTable, DataError> {
        LiveTable::new(Arc::clone(&self.schema), &self.pair)
    }

    /// The left rows, as the join holds them.
    pub fn left(&self) -> &LiveTable {
        &self.left.table
    }

    /// The right rows, as the join holds them.
    pub fn right(&self) -> &LiveTable {
        &self.right.table
    }

    /// Upserts `rows` (of the left columns) into the left rows, and brings `out` up to date:
    /// each of them paired with every right row holding its value (or once with nulls), the
    /// pairs it no longer joins deleted. The changes are `out`'s: its rows inserted, updated (a
    /// pair rewritten) and deleted.
    ///
    /// # Errors
    /// A column error if `rows` has other columns than the left ones,
    /// [`DataError::PairTable`] if `out` is not a table of this join's output, and the left
    /// rows' own errors ([`DataError::NullKey`], ...): nothing changes then.
    /// [`DataError::Arrow`] if Arrow cannot build the output rows (a dictionary column whose
    /// key type cannot index all the values) comes after the join took `rows`: `out` is filled
    /// whole at the next call.
    pub fn upsert_left(
        &mut self,
        out: &mut LiveTable,
        rows: RecordBatch,
    ) -> Result<Changes, DataError> {
        check_columns(self.left.table.schema(), &rows)?;
        self.check_output(out)?;
        let keys = Arc::clone(rows.column(self.left.table.key_column()));
        let before = self.left.current(keys.as_ref());
        self.left.table.upsert(rows)?;
        let mut stale: FxHashSet<u64> = FxHashSet::default();
        let mut fresh: Vec<(RowId, RowId)> = Vec::new();
        for (key, old_id, old_value) in before {
            if let Some(old_id) = old_id {
                stale.extend(self.pairs_of_left(old_id, old_value.as_ref()).map(id_of));
                self.left.forget(old_id, old_value.as_ref());
            }
            // Gone if the window let go of it at once.
            let Some(id) = self.left.table.id_of(key.key()) else {
                continue;
            };
            let value = self.left.value_of(id);
            fresh.extend(self.pairs_of_left(id, value.as_ref()));
            self.left.note(id, value);
        }
        self.drop_expired(&mut stale);
        self.bring_up(out, stale, fresh)
    }

    /// Keeps the left rows to `window` from now on (as [`LiveTable::set_window`] does for a
    /// table), deleting the rows already past it with every pair they are in; `None` keeps
    /// every left row again (rows already let go of stay deleted). Each
    /// [`upsert_left`](Self::upsert_left) then lets go of the rows past it, with their pairs:
    /// the window's end is the largest value among the left rows (data time, not the clock),
    /// and [`upsert_right`](Self::upsert_right), [`delete_left`](Self::delete_left) and
    /// [`delete_right`](Self::delete_right) never let rows go. The changes are `out`'s.
    ///
    /// A window on the output table would delete pairs but not the left rows behind them:
    /// those stay held, and their pairs are written again whenever a row of the other side
    /// holding their value changes. Keep a pair join's left rows recent with this instead.
    ///
    /// # Errors
    /// [`DataError::PairTable`] if `out` is not a table of this join's output, and what
    /// [`LiveTable::set_window`] refuses (an unknown left column, or a span that does not
    /// fit it): nothing changes then. [`DataError::Arrow`] as for
    /// [`upsert_left`](Self::upsert_left).
    pub fn set_left_window(
        &mut self,
        out: &mut LiveTable,
        window: Option<Window>,
    ) -> Result<Changes, DataError> {
        self.check_output(out)?;
        self.left.table.set_window(window)?;
        let mut stale: FxHashSet<u64> = FxHashSet::default();
        self.drop_expired(&mut stale);
        self.bring_up(out, stale, Vec::new())
    }

    /// Lets go of the left rows the window let go of: their pairs go to `stale`. A row of
    /// the batch just taken that the window let go of at once was never noted: its pairs
    /// (with its new value) are in no output, and deleting keys a table does not hold
    /// deletes nothing.
    fn drop_expired(&mut self, stale: &mut FxHashSet<u64>) {
        for (id, value) in self.left.table.take_expired() {
            stale.extend(self.pairs_of_left(id, value.as_ref()).map(id_of));
            self.left.forget(id, value.as_ref());
        }
    }

    /// Deletes the left rows of `keys` (an array of the left key's kind), and their pairs from
    /// `out`. The changes are `out`'s.
    ///
    /// # Errors
    /// [`DataError::KeyType`] if `keys` is not of the left key's kind and
    /// [`DataError::PairTable`] if `out` is not a table of this join's output: nothing changes
    /// then. [`DataError::Arrow`] as for [`upsert_left`](Self::upsert_left).
    pub fn delete_left(
        &mut self,
        out: &mut LiveTable,
        keys: &dyn Array,
    ) -> Result<Changes, DataError> {
        self.check_output(out)?;
        let before = self.left.current(keys);
        self.left.table.delete(keys)?;
        let mut stale: FxHashSet<u64> = FxHashSet::default();
        for (_, id, value) in before {
            let Some(id) = id else {
                continue;
            };
            stale.extend(self.pairs_of_left(id, value.as_ref()).map(id_of));
            self.left.forget(id, value.as_ref());
        }
        self.bring_up(out, stale, Vec::new())
    }

    /// Upserts `rows` (of the right columns) into the right rows, and brings `out` up to date:
    /// each of them paired with every left row holding its value, the pairs it no longer joins
    /// deleted, and the left rows that pair with no right row any more (or now pair with one)
    /// given (or rid of) their row with nulls. The changes are `out`'s.
    ///
    /// # Errors
    /// As for [`upsert_left`](Self::upsert_left), of the right columns.
    pub fn upsert_right(
        &mut self,
        out: &mut LiveTable,
        rows: RecordBatch,
    ) -> Result<Changes, DataError> {
        check_columns(self.right.table.schema(), &rows)?;
        self.check_output(out)?;
        let keys = Arc::clone(rows.column(self.right.table.key_column()));
        let before = self.right.current(keys.as_ref());
        // Which values right rows held before: a left row holding one that loses (or gains)
        // its last right row gains (or loses) its row with nulls. A value a row of the batch
        // holds now was held.
        let mut held: FxHashMap<Owned, bool> = before
            .iter()
            .filter_map(|(_, _, value)| Some((value.clone()?, true)))
            .collect();
        self.right.table.upsert(rows)?;
        let mut stale: FxHashSet<u64> = FxHashSet::default();
        let mut fresh: Vec<(RowId, RowId)> = Vec::new();
        for (key, old_id, old_value) in &before {
            let Some(id) = self.right.table.id_of(key.key()) else {
                continue;
            };
            let value = self.right.value_of(id);
            if let Some(old_id) = *old_id {
                // Pairs kept (the same value: the row's id is its key's) are fresh too, and
                // `apply` keeps them.
                let left = self.left.holding(old_value.as_ref());
                stale.extend(left.map(|left| pair_id(left, old_id)));
                self.right.forget(old_id, old_value.as_ref());
            }
            if let Some(value) = &value {
                // Before any row of this batch noted it: what right rows held before.
                held.entry(value.clone())
                    .or_insert_with(|| self.right.holds(Some(value)));
            }
            fresh.extend(self.left.holding(value.as_ref()).map(|left| (left, id)));
            self.right.note(id, value);
        }
        self.left_rows_alone(&held, &mut stale, &mut fresh);
        self.bring_up(out, stale, fresh)
    }

    /// Deletes the right rows of `keys` (an array of the right key's kind), and their pairs
    /// from `out`; left rows that pair with no right row any more get their row with nulls.
    /// The changes are `out`'s.
    ///
    /// # Errors
    /// As for [`delete_left`](Self::delete_left), of the right key.
    pub fn delete_right(
        &mut self,
        out: &mut LiveTable,
        keys: &dyn Array,
    ) -> Result<Changes, DataError> {
        self.check_output(out)?;
        let before = self.right.current(keys);
        self.right.table.delete(keys)?;
        let mut held: FxHashMap<Owned, bool> = FxHashMap::default();
        let mut stale: FxHashSet<u64> = FxHashSet::default();
        for (_, id, value) in &before {
            let (Some(id), Some(value)) = (*id, value) else {
                continue;
            };
            held.entry(value.clone()).or_insert(true);
            stale.extend(self.left.holding(Some(value)).map(|left| pair_id(left, id)));
            self.right.forget(id, Some(value));
        }
        let mut fresh = Vec::new();
        self.left_rows_alone(&held, &mut stale, &mut fresh);
        self.bring_up(out, stale, fresh)
    }

    /// The pairs of left row `id`, holding `value`: one with each right row holding it, or one
    /// with no right row.
    fn pairs_of_left(
        &self,
        id: RowId,
        value: Option<&Owned>,
    ) -> impl Iterator<Item = (RowId, RowId)> + '_ {
        let alone = (!self.right.holds(value)).then_some((id, NO_RIGHT));
        let rights = self.right.holding(value).map(move |right| (id, right));
        rights.chain(alone)
    }

    /// Whether pair id `id` is one of the join's pairs now.
    fn is_pair(&self, id: u64) -> bool {
        let (left, right) = rows_of(id);
        if self.left.table.slot_of(left).is_none() {
            return false;
        }
        let value = self.left.value_of(left);
        if right == NO_RIGHT {
            return !self.right.holds(value.as_ref());
        }
        // A right row that is gone has no value.
        value.is_some() && self.right.value_of(right) == value
    }

    /// For each value in `held` (whether right rows held it before a change), the rows of the
    /// left rows holding it that pair with nothing: added if no right row holds it now but one
    /// did, deleted if one does now but none did.
    fn left_rows_alone(
        &self,
        held: &FxHashMap<Owned, bool>,
        stale: &mut FxHashSet<u64>,
        fresh: &mut Vec<(RowId, RowId)>,
    ) {
        for (value, &before) in held {
            let now = self.right.holds(Some(value));
            let left = self.left.holding(Some(value));
            match (before, now) {
                (true, false) => fresh.extend(left.map(|left| (left, NO_RIGHT))),
                (false, true) => stale.extend(left.map(|left| pair_id(left, NO_RIGHT))),
                _ => {}
            }
        }
    }

    /// Brings `out` up to date with the join's rows, which have just changed: the pairs
    /// `stale` deleted and `fresh` upserted, or, for a table this join did not bring up to date
    /// last, every pair. After a failure, the table is filled whole at the next call.
    fn bring_up(
        &mut self,
        out: &mut LiveTable,
        stale: FxHashSet<u64>,
        fresh: Vec<(RowId, RowId)>,
    ) -> Result<Changes, DataError> {
        let brought = if self.filled == Some(out.instance()) {
            self.apply(out, stale, fresh)
        } else {
            self.fill(out)
        };
        self.filled = brought.is_ok().then(|| out.instance());
        brought
    }

    /// Deletes the pairs `stale` from `out` (those not in `fresh`) and upserts `fresh`: the
    /// rows of `fresh` are built first, so a failure to build them leaves `out` as it was.
    fn apply(
        &self,
        out: &mut LiveTable,
        mut stale: FxHashSet<u64>,
        fresh: Vec<(RowId, RowId)>,
    ) -> Result<Changes, DataError> {
        // Each change gives a pair once: `fresh` holds no pair twice.
        let kept: FxHashSet<u64> = fresh.iter().copied().map(id_of).collect();
        stale.retain(|id| !kept.contains(id));
        let rows = if fresh.is_empty() {
            None
        } else {
            Some(self.pairs(&fresh)?)
        };
        let mut changes = Changes::default();
        if !stale.is_empty() {
            let ids: UInt64Array = stale.into_iter().collect();
            changes = add(changes, out.delete(&ids)?);
        }
        if let Some(rows) = rows {
            changes = add(changes, out.upsert(rows)?);
        }
        Ok(changes)
    }

    /// Fills `out` with every pair, in batches of up to [`FILL_CHUNK`], after deleting the rows
    /// it holds of pairs that are not the join's now.
    fn fill(&self, out: &mut LiveTable) -> Result<Changes, DataError> {
        let pair_column = self.schema.fields().len() - 1;
        let ended: Vec<u64> = out
            .live_ids()
            .filter_map(|(_, at)| {
                let ids = out.batches()[at.batch].column(pair_column);
                let ids = ids.as_any().downcast_ref::<UInt64Array>()?;
                Some(ids.value(at.row)).filter(|&id| !self.is_pair(id))
            })
            .collect();
        let mut changes = Changes::default();
        for chunk in ended.chunks(FILL_CHUNK) {
            changes = add(changes, out.delete(&UInt64Array::from(chunk.to_vec()))?);
        }
        let mut pairs: Vec<(RowId, RowId)> = Vec::new();
        for (id, _) in self.left.table.live_ids() {
            let value = self.left.value_of(id);
            pairs.extend(self.pairs_of_left(id, value.as_ref()));
            if pairs.len() >= FILL_CHUNK {
                changes = add(changes, out.upsert(self.pairs(&pairs)?)?);
                pairs.clear();
            }
        }
        if !pairs.is_empty() {
            changes = add(changes, out.upsert(self.pairs(&pairs)?)?);
        }
        Ok(changes)
    }

    /// The output rows of `pairs` (left row, right row or [`NO_RIGHT`]).
    fn pairs(&self, pairs: &[(RowId, RowId)]) -> Result<RecordBatch, DataError> {
        let slot = |side: &Side, id: RowId| {
            (id != NO_RIGHT)
                .then(|| side.table.slot_of(id))
                .flatten()
                .map(|at| (at.batch, at.row))
        };
        let left = Places::of(pairs.iter().map(|&(id, _)| slot(&self.left, id)));
        let right = Places::of(pairs.iter().map(|&(_, id)| slot(&self.right, id)));
        let mut columns = Vec::with_capacity(self.schema.fields().len());
        for (column, null) in self.left_nulls.iter().enumerate() {
            let batches = self.left.table.batches();
            columns.push(left.gather(batches, column, Some(null.as_ref()))?);
        }
        for (&column, null) in self.right_columns.iter().zip(&self.right_nulls) {
            let batches = self.right.table.batches();
            columns.push(right.gather(batches, column, Some(null.as_ref()))?);
        }
        let ids: UInt64Array = pairs.iter().map(|&pair| id_of(pair)).collect();
        columns.push(Arc::new(ids));
        RecordBatch::try_new(Arc::clone(&self.schema), columns).map_err(DataError::arrow)
    }

    /// Refuses a table that is not laid out as this join's output.
    fn check_output(&self, out: &LiveTable) -> Result<(), DataError> {
        let ours = self.schema.fields();
        let same = out.key_column() == ours.len() - 1
            && out.schema().fields().len() == ours.len()
            && out
                .schema()
                .fields()
                .iter()
                .zip(ours.iter())
                .all(|(a, b)| a.name() == b.name() && a.data_type() == b.data_type());
        if same {
            Ok(())
        } else {
            Err(DataError::PairTable)
        }
    }
}

/// The id of a pair.
fn id_of((left, right): (RowId, RowId)) -> u64 {
    pair_id(left, right)
}

/// Two counts of changes, added.
fn add(a: Changes, b: Changes) -> Changes {
    Changes {
        inserted: a.inserted + b.inserted,
        updated: a.updated + b.updated,
        deleted: a.deleted + b.deleted,
    }
}

/// The fields of `schema`, owned.
fn fields(schema: &Schema) -> Vec<Field> {
    schema.fields().iter().map(|f| f.as_ref().clone()).collect()
}

/// `name`, or a name made of it that none of `fields` has.
fn unused_name(fields: &[Field], name: &str) -> String {
    let mut unused = name.to_owned();
    while fields.iter().any(|field| field.name() == &unused) {
        unused.push('_');
    }
    unused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_id_holds_both_rows_and_none_is_all_ones() {
        assert_eq!(pair_id(1, 2), (1 << 32) | 2);
        assert_eq!(pair_id(7, NO_RIGHT), (7 << 32) | 0xFFFF_FFFF);
        assert_eq!(rows_of(pair_id(7, NO_RIGHT)), (7, NO_RIGHT));
        assert_eq!(rows_of(pair_id(RowId::MAX, 3)), (RowId::MAX, 3));
        let fields = [Field::new("pair", DataType::Int64, true)];
        assert_eq!(unused_name(&fields, "pair"), "pair_");
        assert_eq!(unused_name(&fields, "other"), "other");
    }

    #[test]
    fn the_rows_of_a_value_shrink_as_they_go() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("k", DataType::Int64, false),
            Field::new("v", DataType::Utf8, true),
        ]));
        let mut side = Side::new(schema, "k", "v").unwrap();
        let hot = Owned::of(KeyRef::Str("hot"));
        for id in 0..10_000 {
            side.note(id, Some(hot.clone()));
        }
        for id in 10..10_000 {
            side.forget(id, Some(&hot));
        }
        let rows = &side.holding[&hot];
        assert_eq!(rows.len(), 10);
        assert!(rows.capacity() <= SHRINK_FROM * 4, "{}", rows.capacity());
        assert!(side.holds(Some(&hot)));
        for id in 0..10 {
            side.forget(id, Some(&hot));
        }
        assert!(!side.holds(Some(&hot)));
        assert!(side.holding.is_empty());
    }

    #[test]
    fn a_copy_of_the_left_rows_keeps_none_it_lets_go_of() {
        use arrow_array::{Int64Array, StringArray};
        let schema = Arc::new(Schema::new(vec![
            Field::new("k", DataType::Int64, false),
            Field::new("v", DataType::Utf8, true),
            Field::new("t", DataType::Int64, true),
        ]));
        let rows = |keys: Vec<i64>, times: Vec<i64>| {
            let values: Vec<&str> = keys.iter().map(|_| "x").collect();
            RecordBatch::try_new(
                Arc::clone(&schema),
                vec![
                    Arc::new(Int64Array::from(keys)),
                    Arc::new(StringArray::from(values)),
                    Arc::new(Int64Array::from(times)),
                ],
            )
            .unwrap()
        };
        let mut side = Side::new(Arc::clone(&schema), "k", "v").unwrap();
        side.table.keep_expired(side.column, side.read.unwrap());
        side.table
            .set_window(Some(Window::span("t", 10.0)))
            .unwrap();
        side.table.upsert(rows(vec![1, 2], vec![0, 100])).unwrap();
        assert_eq!(
            side.table.take_expired().len(),
            1,
            "the join's own table keeps them"
        );
        let mut copy = side.table.clone();
        copy.upsert(rows(vec![3], vec![200])).unwrap();
        assert_eq!(copy.len(), 1);
        assert!(copy.take_expired().is_empty(), "a copy keeps none");
    }
}
