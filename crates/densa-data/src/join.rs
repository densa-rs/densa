//! Lookup joins: each row of one table gets the columns of another table's row whose key it
//! names (a trade its instrument, an order its customer), kept up to date as either side
//! changes.

use std::sync::Arc;

use arrow_array::{new_null_array, Array, ArrayRef, RecordBatch};
use arrow_schema::{Field, Schema, SchemaRef};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::dictionary::interleave_column;
use crate::key::{key_reader, KeyFn, KeyIndex, KeyRef};
use crate::{Changes, DataError, LiveTable, RowId};

/// Index entries kept, at least, before the index is rebuilt from the output table: it is
/// rebuilt once it holds more than twice as many entries as the table has rows.
const MIN_REBUILD: usize = 1_024;

/// A right table's key, owned.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Owned {
    Int(i64),
    UInt(u64),
    Str(Box<str>),
}

impl Owned {
    fn of(key: KeyRef<'_>) -> Self {
        match key {
            KeyRef::Int(k) => Self::Int(k),
            KeyRef::UInt(k) => Self::UInt(k),
            KeyRef::Str(k) => Self::Str(k.into()),
        }
    }
}

/// Joins the rows of a live table to the rows of another, the right table, that they name:
/// each output row is a left row followed by the columns of the right row whose key is in the
/// left row's `on` column (nulls when there is none: a left outer join). The join keeps the
/// right table; the output is an ordinary [`LiveTable`] the application owns (made by
/// [`table`](Self::table)), so views, groups, windows and grids work on it as on any table.
///
/// Left rows go in through [`upsert_left`](Self::upsert_left), joined as they arrive; right
/// rows through [`upsert_right`](Self::upsert_right) and
/// [`delete_right`](Self::delete_right), which join again every output row naming a changed
/// right key. Remove left rows from the output table itself (`delete`, or a [`Window`]): the
/// join notices. To keep only rows that found a match, view the output with
/// `Filter::is_not_null` on a right column.
///
/// Rows reach the output only through [`upsert_left`](Self::upsert_left): delete from it,
/// window it, compact it, view and group it, but do not upsert into it directly, or those
/// rows are not joined again when their right row changes.
///
/// A right row's change rewrites every output row naming it, so a right key named by many
/// rows (one instrument of most trades) costs that many rows each time it changes. The join
/// allocates as it works (its index, the joined batches).
///
/// ```
/// use std::sync::Arc;
/// use densa_data::arrow_array::cast::AsArray;
/// use densa_data::arrow_array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
/// use densa_data::LookupJoin;
///
/// let trades = |ids: Vec<i64>, symbols: Vec<&str>, prices: Vec<f64>| {
///     RecordBatch::try_from_iter([
///         ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
///         ("symbol", Arc::new(StringArray::from(symbols)) as ArrayRef),
///         ("price", Arc::new(Float64Array::from(prices)) as ArrayRef),
///     ])
/// };
/// let instruments = |symbols: Vec<&str>, names: Vec<&str>| {
///     RecordBatch::try_from_iter([
///         ("symbol", Arc::new(StringArray::from(symbols)) as ArrayRef),
///         ("name", Arc::new(StringArray::from(names)) as ArrayRef),
///     ])
/// };
///
/// // Trades keyed by id name their instrument in `symbol`; instruments are keyed by symbol.
/// let left = trades(vec![], vec![], vec![])?.schema();
/// let right = instruments(vec![], vec![])?.schema();
/// let mut join = LookupJoin::new(left, "id", "symbol", right, "symbol")?;
/// let mut joined = join.table()?; // id, symbol, price, name
///
/// join.upsert_right(&mut joined, instruments(vec!["AAA"], vec!["Alpha"])?)?;
/// join.upsert_left(&mut joined, trades(vec![1, 2], vec!["AAA", "BBB"], vec![10.0, 20.0])?)?;
/// // BBB is not an instrument yet: trade 2's name is null until it is.
/// join.upsert_right(&mut joined, instruments(vec!["BBB"], vec!["Beta"])?)?;
///
/// let names: Vec<String> = joined
///     .snapshot()?
///     .iter()
///     .flat_map(|batch| batch.column(3).as_string::<i32>().iter().flatten().map(String::from).collect::<Vec<_>>())
///     .collect();
/// assert_eq!(names.len(), 2);
/// assert!(names.contains(&"Alpha".to_owned()) && names.contains(&"Beta".to_owned()));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// [`Window`]: crate::Window
// Not Clone: a copy would share the output table's index without seeing the other's rows.
#[derive(Debug)]
pub struct LookupJoin {
    /// The left rows' columns (every one nullable), and where their key and `on` are.
    left: SchemaRef,
    left_key: String,
    left_key_column: usize,
    left_key_read: KeyFn,
    on: usize,
    on_read: KeyFn,
    right: LiveTable,
    right_key_read: KeyFn,
    /// The right columns joined (every one but its key), by index in the right table.
    right_columns: Vec<usize>,
    /// One null per joined right column, for rows that name no right row.
    nulls: Vec<ArrayRef>,
    schema: SchemaRef,
    /// The output rows (by their row id in the output table) naming each right key. Entries
    /// go stale when output rows change or leave: they are checked when used, and dropped
    /// when the index is rebuilt.
    refs: FxHashMap<Owned, FxHashSet<RowId>>,
    entries: usize,
    /// The output table the index is of.
    indexed: Option<u64>,
}

impl LookupJoin {
    /// A join of rows of `left`'s columns (keyed by `left_key`) to the rows of a right table
    /// of `right`'s columns (keyed by `right_key`) whose key is in the left column `on`. The
    /// output has the left columns, then the right ones but the right key (its value is in
    /// `on`).
    ///
    /// # Errors
    /// [`DataError::UnknownColumn`] for a column neither side has,
    /// [`DataError::UnsupportedKey`] for a key column of another type than integers or text,
    /// [`DataError::JoinKey`] if `on` does not hold the right key's kind of value,
    /// [`DataError::DuplicateName`] if a left column and a right one share a name.
    pub fn new(
        left: SchemaRef,
        left_key: &str,
        on: &str,
        right: SchemaRef,
        right_key: &str,
    ) -> Result<Self, DataError> {
        let (left_key_column, key_field) = left
            .column_with_name(left_key)
            .ok_or_else(|| DataError::UnknownColumn(left_key.to_owned()))?;
        let left_key_read = key_reader(key_field.data_type())
            .ok_or_else(|| DataError::UnsupportedKey(left_key.to_owned()))?;
        let right = LiveTable::new(right, right_key)?;
        let right_schema = Arc::clone(right.schema());
        let right_key_type = right_schema.field(right.key_column()).data_type();
        let right_key_read = key_reader(right_key_type)
            .ok_or_else(|| DataError::UnsupportedKey(right_key.into()))?;
        let (on_column, on_field) = left
            .column_with_name(on)
            .ok_or_else(|| DataError::UnknownColumn(on.to_owned()))?;
        let on_read = key_reader(on_field.data_type())
            .filter(|_| {
                KeyIndex::for_type(on_field.data_type())
                    .is_some_and(|kind| kind.same_kind(right_key_type))
            })
            .ok_or_else(|| DataError::JoinKey(on.to_owned()))?;

        let left = nullable(left.fields().iter().map(|f| f.as_ref().clone()));
        let right_columns: Vec<usize> = (0..right_schema.fields().len())
            .filter(|&i| i != right.key_column())
            .collect();
        let mut fields: Vec<Field> = left.fields().iter().map(|f| f.as_ref().clone()).collect();
        for &column in &right_columns {
            let field = right_schema.field(column);
            if fields.iter().any(|f| f.name() == field.name()) {
                return Err(DataError::DuplicateName(field.name().clone()));
            }
            fields.push(field.clone());
        }
        let nulls = right_columns
            .iter()
            .map(|&c| new_null_array(right_schema.field(c).data_type(), 1))
            .collect();
        Ok(Self {
            left,
            left_key: left_key.to_owned(),
            left_key_column,
            left_key_read,
            on: on_column,
            on_read,
            right,
            right_key_read,
            right_columns,
            nulls,
            schema: nullable(fields),
            refs: FxHashMap::default(),
            entries: 0,
            indexed: None,
        })
    }

    /// The output's columns: the left ones, then the right ones but the right key; every one
    /// nullable.
    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// An empty output table, keyed by the left key: give it to every call of this join. Add
    /// views, groups or a window to it as to any table, or hand it to a grid.
    ///
    /// # Errors
    /// None in practice: the columns were checked when the join was made.
    pub fn table(&self) -> Result<LiveTable, DataError> {
        LiveTable::new(Arc::clone(&self.schema), &self.left_key)
    }

    /// The right rows, as the join holds them.
    pub fn right(&self) -> &LiveTable {
        &self.right
    }

    /// Joins `rows` (of the left columns) and upserts them into `out`, replacing the output
    /// rows of their keys. The changes are `out`'s.
    ///
    /// # Errors
    /// A column error if `rows` has other columns than the left ones,
    /// [`DataError::JoinTable`] if `out` is not a table of this join's output, and `out`'s own
    /// errors ([`DataError::NullKey`], ...). Nothing changes on error.
    pub fn upsert_left(
        &mut self,
        out: &mut LiveTable,
        rows: RecordBatch,
    ) -> Result<Changes, DataError> {
        check_columns(&self.left, &rows)?;
        self.check_output(out)?;
        let joined = self.joined(&rows)?;
        let changes = out.upsert(joined)?;
        if self.indexed == Some(out.instance()) {
            self.index(out, &rows);
            self.rebuild_if_stale(out);
        } else {
            self.rebuild(out);
        }
        Ok(changes)
    }

    /// Upserts `rows` (of the right columns) into the right table, then joins again every row
    /// of `out` naming one of their keys. The changes are `out`'s.
    ///
    /// # Errors
    /// The right table's errors for `rows` (other columns, a null key, ...) and
    /// [`DataError::JoinTable`] if `out` is not a table of this join's output: nothing changes
    /// then. [`DataError::Arrow`] if Arrow cannot join the rows again (a dictionary column
    /// whose key type cannot index all the values joined) comes after the right table took
    /// `rows`: the output rows naming those keys keep their old right columns until the keys
    /// change again.
    pub fn upsert_right(
        &mut self,
        out: &mut LiveTable,
        rows: RecordBatch,
    ) -> Result<Changes, DataError> {
        self.check_output(out)?;
        // The right table checks the columns before the key column is read.
        self.right.upsert(rows.clone())?;
        let keys = rows.column(self.right.key_column());
        self.rejoin(out, keys.as_ref(), self.right_key_read)
    }

    /// Deletes the right rows of `keys` (an array of the right key's kind), then joins again
    /// every row of `out` naming one of them: their right columns become null. The changes are
    /// `out`'s.
    ///
    /// # Errors
    /// [`DataError::KeyType`] if `keys` is not of the right key's kind and
    /// [`DataError::JoinTable`] if `out` is not a table of this join's output: nothing changes
    /// then. [`DataError::Arrow`] as for [`upsert_right`](Self::upsert_right).
    pub fn delete_right(
        &mut self,
        out: &mut LiveTable,
        keys: &dyn Array,
    ) -> Result<Changes, DataError> {
        self.check_output(out)?;
        let read = key_reader(keys.data_type()).ok_or(DataError::KeyType)?;
        self.right.delete(keys)?;
        self.rejoin(out, keys, read)
    }

    /// `rows` (of the left columns) with each one's right columns.
    fn joined(&self, rows: &RecordBatch) -> Result<RecordBatch, DataError> {
        let on = rows.column(self.on).as_ref();
        let found = (0..rows.num_rows()).map(|row| {
            let id = (self.on_read)(on, row).and_then(|key| self.right.id_of(key))?;
            self.right.slot_of(id).map(|at| (at.batch, at.row))
        });
        let places = Places::of(found);
        let mut columns = rows.columns().to_vec();
        for (&column, null) in self.right_columns.iter().zip(&self.nulls) {
            columns.push(places.gather(self.right.batches(), column, Some(null.as_ref()))?);
        }
        RecordBatch::try_new(Arc::clone(&self.schema), columns).map_err(DataError::arrow)
    }

    /// Joins again the rows of `out` naming the right keys in `keys` (read by `read`).
    fn rejoin(
        &mut self,
        out: &mut LiveTable,
        keys: &dyn Array,
        read: KeyFn,
    ) -> Result<Changes, DataError> {
        if self.indexed != Some(out.instance()) {
            self.rebuild(out);
        }
        let (on, on_read) = (self.on, self.on_read);
        let mut seen: FxHashSet<Owned> = FxHashSet::default();
        let mut ids: FxHashSet<RowId> = FxHashSet::default();
        for row in 0..keys.len() {
            let Some(key) = read(keys, row) else {
                continue;
            };
            // A batch often repeats a key (ticks of one symbol): each is handled once.
            let owned = Owned::of(key);
            if seen.contains(&owned) {
                continue;
            }
            if let Some(named) = self.refs.get_mut(&owned) {
                let before = named.len();
                named.retain(|&id| names(out, id, on, on_read) == Some(key));
                self.entries -= before - named.len();
                ids.extend(named.iter().copied());
                if named.is_empty() {
                    self.refs.remove(&owned);
                }
            }
            seen.insert(owned);
        }
        if ids.is_empty() {
            return Ok(Changes::default());
        }
        // Every id was just checked to be live.
        let places = Places::of(
            ids.iter()
                .map(|&id| out.slot_of(id).map(|at| (at.batch, at.row))),
        );
        let columns = (0..self.left.fields().len())
            .map(|column| places.gather(out.batches(), column, None))
            .collect::<Result<Vec<ArrayRef>, _>>()?;
        let rows =
            RecordBatch::try_new(Arc::clone(&self.left), columns).map_err(DataError::arrow)?;
        let joined = self.joined(&rows)?;
        out.upsert(joined)
    }

    /// Records which right key each of `rows` (just upserted into `out`) names.
    fn index(&mut self, out: &LiveTable, rows: &RecordBatch) {
        let keys = rows.column(self.left_key_column).as_ref();
        let on = rows.column(self.on).as_ref();
        for row in 0..rows.num_rows() {
            let id = (self.left_key_read)(keys, row).and_then(|key| out.id_of(key));
            if let (Some(id), Some(named)) = (id, (self.on_read)(on, row)) {
                if self.refs.entry(Owned::of(named)).or_default().insert(id) {
                    self.entries += 1;
                }
            }
        }
    }

    /// Indexes `out` anew: every live row and the right key it names.
    fn rebuild(&mut self, out: &LiveTable) {
        self.refs.clear();
        self.entries = 0;
        for (id, at) in out.live_ids() {
            let column = out.batches()[at.batch].column(self.on).as_ref();
            if let Some(named) = (self.on_read)(column, at.row) {
                self.refs.entry(Owned::of(named)).or_default().insert(id);
                self.entries += 1;
            }
        }
        self.indexed = Some(out.instance());
    }

    /// Rebuilds the index once stale entries (rows changed or gone) outnumber live rows.
    fn rebuild_if_stale(&mut self, out: &LiveTable) {
        if self.entries > MIN_REBUILD && self.entries > out.len().saturating_mul(2) {
            self.rebuild(out);
        }
    }

    /// Refuses a table that is not of this join's output.
    fn check_output(&self, out: &LiveTable) -> Result<(), DataError> {
        let same = out.key_column() == self.left_key_column
            && out.schema().fields().len() == self.schema.fields().len()
            && out
                .schema()
                .fields()
                .iter()
                .zip(self.schema.fields().iter())
                .all(|(a, b)| a.name() == b.name() && a.data_type() == b.data_type());
        if same {
            Ok(())
        } else {
            Err(DataError::JoinTable)
        }
    }
}

/// Rows to gather from stored batches: only the batches read are handed to Arrow, which may
/// size a dictionary column by all its inputs' values and holds the buffers of every input
/// (as `LiveTable`'s own gathers do). Places missing a row take a fill value.
struct Places {
    /// The stored batches read, in the order of their first use.
    batches: Vec<usize>,
    /// Each row's (source, row): a source past `batches` is the fill.
    indices: Vec<(usize, usize)>,
    missing: bool,
}

impl Places {
    fn of(found: impl Iterator<Item = Option<(usize, usize)>>) -> Self {
        let mut position: FxHashMap<usize, usize> = FxHashMap::default();
        let mut batches = Vec::new();
        let mut missing = false;
        let indices: Vec<Option<(usize, usize)>> = found
            .map(|place| {
                let Some((batch, row)) = place else {
                    missing = true;
                    return None;
                };
                let source = *position.entry(batch).or_insert_with(|| {
                    batches.push(batch);
                    batches.len() - 1
                });
                Some((source, row))
            })
            .collect();
        let fill = (batches.len(), 0);
        Self {
            indices: indices.into_iter().map(|i| i.unwrap_or(fill)).collect(),
            batches,
            missing,
        }
    }

    /// Column `column` of `stored` at these places, `fill` (one value) where a row is missing
    /// (a gather with nothing missing needs none).
    fn gather(
        &self,
        stored: &[RecordBatch],
        column: usize,
        fill: Option<&dyn Array>,
    ) -> Result<ArrayRef, DataError> {
        let mut sources: Vec<&dyn Array> = self
            .batches
            .iter()
            .map(|&batch| stored[batch].column(column).as_ref())
            .collect();
        if let Some(fill) = fill.filter(|_| self.missing || sources.is_empty()) {
            sources.push(fill);
        }
        interleave_column(&sources, &self.indices).map_err(DataError::arrow)
    }
}

/// The right key the live row `id` of `out` names, if it is live and names one.
fn names(out: &LiveTable, id: RowId, on: usize, read: KeyFn) -> Option<KeyRef<'_>> {
    let at = out.slot_of(id)?;
    read(out.batches()[at.batch].column(on).as_ref(), at.row)
}

/// A schema of `fields`, every one nullable.
fn nullable(fields: impl IntoIterator<Item = Field>) -> SchemaRef {
    let fields: Vec<Field> = fields.into_iter().map(|f| f.with_nullable(true)).collect();
    Arc::new(Schema::new(fields))
}

/// Refuses `rows` unless they have `schema`'s column names and data types.
fn check_columns(schema: &Schema, rows: &RecordBatch) -> Result<(), DataError> {
    let (ours, theirs) = (schema.fields(), rows.schema_ref().fields());
    if ours.len() != theirs.len() {
        return Err(DataError::ColumnCount {
            expected: ours.len(),
            found: theirs.len(),
        });
    }
    match ours
        .iter()
        .zip(theirs.iter())
        .position(|(a, b)| a.name() != b.name() || a.data_type() != b.data_type())
    {
        Some(column) => Err(DataError::ColumnMismatch { column }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests;
