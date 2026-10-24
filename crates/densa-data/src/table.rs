use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, RecordBatch};
use arrow_schema::{Field, Schema, SchemaRef};
use rustc_hash::FxHashMap;

use crate::dictionary::interleave_column;
use crate::group::{GroupState, Groups};
use crate::key::{key_reader, KeyFn, KeyIndex, KeyRef};
use crate::view::{Rows, ViewState};
use crate::window::WindowState;
use crate::{DataError, GroupSpec, RowId, Scalar, ViewSpec, Window};

/// Changing more than one row in this many rebuilds each view instead of patching it.
const REBUILD_RATIO: usize = 8;
/// Superseded rows tolerated before an automatic compaction (and only once they outnumber
/// the live rows).
const MIN_DEAD_ROWS: usize = 4_096;
/// Stored batches tolerated before an automatic compaction merges them.
const MAX_BATCHES: usize = 4_096;
/// Rows per batch written by compaction.
const COMPACT_ROWS: usize = 65_536;
/// Compaction splits a chunk Arrow cannot build into halves, down to this many rows. Any
/// dictionary key type indexes this many distinct strings, so a smaller chunk would not help.
const MIN_SPLIT_ROWS: usize = 128;
/// Row ids are `u32`: at most this many live keys.
const MAX_ROWS: usize = 1 << 32;

/// Hands out identities of tables, views and groups, unique within the process.
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

fn next_serial() -> u64 {
    // ORDERING: Relaxed. Only uniqueness matters; no other memory is published with it.
    NEXT_SERIAL.fetch_add(1, AtomicOrdering::Relaxed)
}

/// Where a row's values are: `table.batches()[batch]`, row `row`. Valid until the next
/// compaction, which [`LiveTable::upsert`] and [`LiveTable::delete`] may run: compare
/// [`LiveTable::generation`] before reusing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowRef {
    /// Index into [`LiveTable::batches`].
    pub batch: usize,
    /// Row within that batch.
    pub row: usize,
}

/// Where a live row id's values are, packed to 8 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Slot {
    batch: u32,
    row: u32,
}

impl Slot {
    pub(crate) fn row_ref(self) -> RowRef {
        RowRef {
            batch: self.batch as usize,
            row: self.row as usize,
        }
    }
}

/// What an upsert or delete did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Changes {
    /// Keys that were new.
    pub inserted: usize,
    /// Keys whose row was replaced.
    pub updated: usize,
    /// Keys removed.
    pub deleted: usize,
}

/// How a [`LiveTable`] manages its storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TableOptions {
    /// Whether [`LiveTable::upsert`] and [`LiveTable::delete`] compact the table when
    /// superseded rows outnumber live ones (at least 4,096 of them) or more than 4,096 batches
    /// were stored since the last compaction. Compaction copies every live row, a pause
    /// proportional to the table;
    /// without it, call [`LiveTable::compact`] when convenient. Default: on.
    pub auto_compact: bool,
}

impl Default for TableOptions {
    fn default() -> Self {
        Self { auto_compact: true }
    }
}

impl TableOptions {
    /// The options with automatic compaction on or off.
    #[must_use]
    pub fn with_auto_compact(mut self, auto_compact: bool) -> Self {
        self.auto_compact = auto_compact;
        self
    }
}

/// Identifies a view of one [`LiveTable`]. A clone of the table copies its views, so the id
/// of a view added before cloning works on both; one added afterwards, only on its table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewId {
    /// Unique to the view's addition.
    serial: u64,
    index: usize,
}

/// Identifies grouped aggregates of one [`LiveTable`] (and of clones made after they were
/// added, as for [`ViewId`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GroupsId {
    serial: u64,
    index: usize,
}

/// A record: one key's row for as long as the key is live, through updates, compactions and
/// view changes. Deleting the key ends the record; inserting the key again starts a new one.
/// Get one with [`LiveTable::record_of_key`] or [`View::record`], find its row with
/// [`LiveTable::row_of`] and its place in a view with [`View::position`].
///
/// It is a plain number ([`to_bits`](Self::to_bits)), so a widget can keep it as a selection
/// without borrowing the table. It names a record of the table it came from and of clones
/// made after; in any other table it may name an unrelated row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordId(u64);

impl RecordId {
    fn new(id: RowId, reuse: u32) -> Self {
        Self(u64::from(reuse) << 32 | u64::from(id))
    }

    /// The record as a number, for storing outside densa-data.
    #[must_use]
    pub const fn to_bits(self) -> u64 {
        self.0
    }

    /// The record that [`to_bits`](Self::to_bits) gave `bits` for. A number it did not give
    /// names no record, or an arbitrary one.
    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The row id, and how many times it had been freed when the record began.
    fn parts(self) -> (RowId, u32) {
        // Lossless: the low and high halves.
        (self.0 as u32, (self.0 >> 32) as u32)
    }
}

/// One planned row change: its id, where its values were (if it existed) and are now, and
/// its key if new.
type Change<'a> = (RowId, Option<Slot>, Slot, Option<KeyRef<'a>>);

/// A table of rows identified by a key column, updated by Arrow record batches, with views
/// (filtered, sorted) that follow every change.
///
/// Storage is log-structured: an upserted batch is kept as it is (its arrays shared, not
/// copied) and each key points at its latest row. Superseded rows are dropped by compaction,
/// automatically (see [`TableOptions`]) or with [`compact`](Self::compact). Every key has a
/// stable internal id, so compaction never reorders a view.
///
/// Views and grouped aggregates ([`GroupSpec`]) are maintained incrementally: changing `k`
/// rows costs each view about `k` sorted insertions and removals (a few binary searches over
/// contiguous memory plus moving one block of at most 1,024 entries), and each group spec as
/// much per min or max column plus `O(log groups)` per row, not a re-sort or a recount; a
/// change touching more than an eighth of the rows rebuilds them instead. Reading a view
/// ([`View::row`], `O(log n)`) never allocates; [`Groups::to_batch`] does.
///
/// Every column of the table is nullable, whatever the schema given says; batches must have
/// the table's column names and data types, in order.
#[derive(Debug)]
pub struct LiveTable {
    /// Unique to this table object; a clone gets its own.
    instance: u64,
    options: TableOptions,
    schema: SchemaRef,
    key_column: usize,
    key_read: KeyFn,
    keys: KeyIndex,
    batches: Vec<RecordBatch>,
    /// Where each row id's values are; `None` for a free id.
    slots: Vec<Option<Slot>>,
    free: Vec<RowId>,
    /// How many times each row id was freed (missing: never), so a record of a deleted key
    /// never names the key that reuses its id.
    reuses: Vec<u32>,
    /// Stored rows that are no longer live.
    dead: usize,
    /// Each view with the serial of its id.
    views: Vec<Option<(u64, ViewState)>>,
    groups: Vec<Option<(u64, GroupState)>>,
    generation: u64,
    /// The last automatic compaction's failure, until a compaction succeeds.
    compaction_error: Option<DataError>,
    /// Dead rows at which automatic compaction tries again after a failure.
    retry_at_dead: usize,
    /// Batches the last compaction wrote: automatic compaction counts the ones stored since.
    compacted_batches: usize,
    window: Option<WindowState>,
}

impl Clone for LiveTable {
    fn clone(&self) -> Self {
        Self {
            instance: next_serial(),
            options: self.options,
            schema: Arc::clone(&self.schema),
            key_column: self.key_column,
            key_read: self.key_read,
            keys: self.keys.clone(),
            batches: self.batches.clone(),
            slots: self.slots.clone(),
            free: self.free.clone(),
            reuses: self.reuses.clone(),
            dead: self.dead,
            views: self.views.clone(),
            groups: self.groups.clone(),
            generation: self.generation,
            compaction_error: self.compaction_error.clone(),
            retry_at_dead: self.retry_at_dead,
            compacted_batches: self.compacted_batches,
            window: self.window.clone(),
        }
    }
}

impl LiveTable {
    /// An empty table of `schema`, keyed by the column named `key` (integers or strings),
    /// with default [`TableOptions`].
    ///
    /// # Errors
    /// [`DataError::UnknownColumn`] or [`DataError::UnsupportedKey`].
    pub fn new(schema: SchemaRef, key: &str) -> Result<Self, DataError> {
        Self::with_options(schema, key, TableOptions::default())
    }

    /// An empty table of `schema`, keyed by the column named `key`, managed with `options`.
    ///
    /// # Errors
    /// [`DataError::UnknownColumn`] or [`DataError::UnsupportedKey`].
    pub fn with_options(
        schema: SchemaRef,
        key: &str,
        options: TableOptions,
    ) -> Result<Self, DataError> {
        let (key_column, field) = schema
            .column_with_name(key)
            .ok_or_else(|| DataError::UnknownColumn(key.to_owned()))?;
        let unsupported = || DataError::UnsupportedKey(key.to_owned());
        let key_read = key_reader(field.data_type()).ok_or_else(unsupported)?;
        let keys = KeyIndex::for_type(field.data_type()).ok_or_else(unsupported)?;
        Ok(Self {
            instance: next_serial(),
            options,
            schema: nullable(&schema),
            key_column,
            key_read,
            keys,
            batches: Vec::new(),
            slots: Vec::new(),
            free: Vec::new(),
            reuses: Vec::new(),
            dead: 0,
            views: Vec::new(),
            groups: Vec::new(),
            generation: 0,
            compaction_error: None,
            retry_at_dead: MIN_DEAD_ROWS,
            compacted_batches: 0,
            window: None,
        })
    }

    /// The table's schema (every column nullable).
    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// Distinguishes this table object from every other one in the process, clones
    /// included: something caching per table (a grid's formatters) compares it with
    /// [`generation`](Self::generation).
    pub fn instance(&self) -> u64 {
        self.instance
    }

    /// Live rows: one per key.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether there are no live rows.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The stored batches [`RowRef`]s point into. They also hold superseded rows until the
    /// next compaction, and keep the schema they arrived with (compacted ones have the
    /// table's); [`snapshot`](Self::snapshot) gives the live rows alone, in the table's.
    pub fn batches(&self) -> &[RecordBatch] {
        &self.batches
    }

    /// Where each live row is, in row id order.
    pub(crate) fn live_rows(&self) -> impl Iterator<Item = RowRef> + '_ {
        self.slots.iter().flatten().map(|slot| slot.row_ref())
    }

    /// Counts compactions: when it changes, batches were rewritten and earlier [`RowRef`]s
    /// are invalid (views stay valid).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Why the last automatic compaction failed, if it did and no compaction has succeeded
    /// since. The table stays correct, only larger; automatic compaction tries again once
    /// twice as many rows are superseded.
    pub fn compaction_error(&self) -> Option<&DataError> {
        self.compaction_error.as_ref()
    }

    /// Where the live row with key `key` is.
    pub fn find(&self, key: &Scalar) -> Option<RowRef> {
        let id = self.key_id(key)?;
        self.slots[id as usize].map(Slot::row_ref)
    }

    /// The record of key `key`, if it is live.
    pub fn record_of_key(&self, key: &Scalar) -> Option<RecordId> {
        let id = self.key_id(key)?;
        Some(RecordId::new(id, reuse(&self.reuses, id)))
    }

    /// Where the row of `record` is, or `None` if its key was deleted.
    pub fn row_of(&self, record: RecordId) -> Option<RowRef> {
        live_slot(&self.slots, &self.reuses, record).map(|(_, slot)| slot.row_ref())
    }

    /// The row id holding `key`, if it is live.
    pub(crate) fn id_of(&self, key: KeyRef<'_>) -> Option<RowId> {
        self.keys.get(key)
    }

    /// Where the row with id `id` is, if it is live.
    pub(crate) fn slot_of(&self, id: RowId) -> Option<RowRef> {
        let slot = (*self.slots.get(id as usize)?)?;
        Some(slot.row_ref())
    }

    /// Each live row's id and place.
    pub(crate) fn live_ids(&self) -> impl Iterator<Item = (RowId, RowRef)> + '_ {
        // Lossless: row ids are u32.
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(id, slot)| Some((id as RowId, (*slot)?.row_ref())))
    }

    /// The column index of the key.
    pub(crate) fn key_column(&self) -> usize {
        self.key_column
    }

    /// The row id of key `key`, if it is live.
    fn key_id(&self, key: &Scalar) -> Option<RowId> {
        let key = match (key, &self.keys) {
            (Scalar::Int(k), KeyIndex::Int(_)) => KeyRef::Int(*k),
            (Scalar::Int(k), KeyIndex::UInt(_)) => KeyRef::UInt(u64::try_from(*k).ok()?),
            (Scalar::UInt(k), KeyIndex::UInt(_)) => KeyRef::UInt(*k),
            (Scalar::UInt(k), KeyIndex::Int(_)) => KeyRef::Int(i64::try_from(*k).ok()?),
            (Scalar::Str(k), KeyIndex::Str(_)) => KeyRef::Str(k),
            _ => return None,
        };
        self.keys.get(key)
    }

    /// Inserts `batch`'s rows, replacing the rows of keys already present; within the batch
    /// the last row for a key wins. Then, with a [`Window`], deletes the rows it leaves
    /// behind (counted in [`Changes::deleted`]; a row already past it when it came counts as
    /// inserted and deleted). Views follow.
    ///
    /// # Errors
    /// A column error if `batch` has other column names or data types,
    /// [`DataError::NullKey`] if a row has no key, [`DataError::TooManyRows`] past 2^32 live
    /// keys or rows in a batch. Nothing changes on error.
    pub fn upsert(&mut self, batch: RecordBatch) -> Result<Changes, DataError> {
        self.check(&batch)?;
        let latest = self.latest_rows(&batch)?;
        if latest.is_empty() {
            return Ok(Changes::default());
        }
        let plan = self.plan(&batch, &latest)?;
        let updated = plan.iter().filter(|c| c.1.is_some()).count();
        let mut changes = Changes {
            inserted: plan.len() - updated,
            updated,
            deleted: 0,
        };
        let rebuild = plan.len().saturating_mul(REBUILD_RATIO) > self.len() + changes.inserted;

        self.batches.push(batch.clone());
        if !rebuild {
            self.drop_from_views(plan.iter().filter_map(|c| Some((c.0, c.1?))));
        }
        for &(id, old, at, key) in &plan {
            self.slots[id as usize] = Some(at);
            self.dead += usize::from(old.is_some());
            if let Some(key) = key {
                self.keys.insert(key, id);
            }
        }
        // Earlier rows of keys repeated within the batch were never live.
        self.dead += batch.num_rows() - plan.len();
        self.add_to_views(rebuild, plan.iter().map(|c| (c.0, c.2)));
        changes.deleted = self.expire();
        self.compact_if_due();
        Ok(changes)
    }

    /// Deletes the rows of `keys` (an array of the key column's kind; nulls and unknown keys
    /// are skipped). Views follow.
    ///
    /// ```
    /// # use std::sync::Arc;
    /// # use densa_data::arrow_array::{ArrayRef, Int64Array, RecordBatch};
    /// # use densa_data::LiveTable;
    /// # let ids: ArrayRef = Arc::new(Int64Array::from(vec![1, 2, 3]));
    /// # let rows = RecordBatch::try_from_iter([("id", ids)])?;
    /// # let mut table = LiveTable::new(rows.schema(), "id")?;
    /// # table.upsert(rows)?;
    /// // Keys go in an Arrow array of the key column's type: here an Int64 key.
    /// table.delete(&Int64Array::from(vec![1, 3]))?;
    /// assert_eq!(table.len(), 1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    /// [`DataError::KeyType`] if `keys` is not of the key column's kind: nothing changes.
    pub fn delete(&mut self, keys: &dyn Array) -> Result<Changes, DataError> {
        let read = key_reader(keys.data_type())
            .filter(|_| self.keys.same_kind(keys.data_type()))
            .ok_or(DataError::KeyType)?;
        let mut deleted = 0;
        for row in 0..keys.len() {
            let Some(id) = read(keys, row).and_then(|key| self.keys.remove(key)) else {
                continue;
            };
            if let Some(slot) = self.slots[id as usize] {
                self.drop_from_views(std::iter::once((id, slot)));
            }
            self.release(id);
            deleted += 1;
        }
        self.compact_if_due();
        Ok(Changes {
            deleted,
            ..Changes::default()
        })
    }

    /// Rewrites the stored batches with only the live rows, in batches of up to 65,536 rows
    /// (fewer where one batch cannot hold them: more distinct strings than a dictionary
    /// column's key type indexes, over 2 GiB of strings). Dictionary columns of strings are
    /// re-encoded with the strings their rows use. Views keep their order;
    /// [`generation`](Self::generation) increases.
    ///
    /// # Errors
    /// [`DataError::Arrow`] if Arrow cannot build a batch of even 128 of the rows; nothing
    /// changes.
    pub fn compact(&mut self) -> Result<(), DataError> {
        let live: Vec<(RowId, Slot)> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(id, slot)| Some((RowId::try_from(id).ok()?, (*slot)?)))
            .collect();
        let mut compacted = Vec::new();
        for chunk in live.chunks(COMPACT_ROWS) {
            self.gather_into(chunk, &mut compacted)?;
        }
        // The live rows, in order, fill the new batches in order.
        let places = compacted.iter().enumerate().flat_map(|(batch, rows)| {
            // Lossless: at most 2^32 live rows, so as many batches or rows in one.
            (0..rows.num_rows()).map(move |row| Slot {
                batch: batch as u32,
                row: row as u32,
            })
        });
        for (&(id, _), slot) in live.iter().zip(places) {
            self.slots[id as usize] = Some(slot);
        }
        self.compacted_batches = compacted.len();
        self.batches = compacted;
        self.dead = 0;
        self.generation += 1;
        self.compaction_error = None;
        self.retry_at_dead = MIN_DEAD_ROWS;
        Ok(())
    }

    /// Keeps the table to the rows within `window` from now on, deleting the rows already
    /// past it; `None` keeps every row again (rows already deleted stay deleted). Views
    /// follow.
    ///
    /// # Errors
    /// [`DataError::UnknownColumn`], [`DataError::UnsupportedColumn`], or
    /// [`DataError::WindowSpan`] if the span does not fit the column: nothing changes.
    pub fn set_window(&mut self, window: Option<Window>) -> Result<Changes, DataError> {
        let Some(window) = window else {
            self.window = None;
            return Ok(Changes::default());
        };
        let mut state = WindowState::compile(&window, &self.schema)?;
        state.rows.rebuild(Rows {
            batches: &self.batches,
            slots: &self.slots,
        });
        self.window = Some(state);
        let deleted = self.expire();
        self.compact_if_due();
        Ok(Changes {
            deleted,
            ..Changes::default()
        })
    }

    /// The table's window, if it has one.
    pub fn window(&self) -> Option<&Window> {
        self.window.as_ref().map(WindowState::spec)
    }

    /// Adds a view of the rows matching `spec`, filled from the current rows.
    ///
    /// # Errors
    /// [`DataError::UnknownColumn`], [`DataError::UnsupportedColumn`] or
    /// [`DataError::LiteralMismatch`] if the spec does not fit the schema.
    pub fn add_view(&mut self, spec: ViewSpec) -> Result<ViewId, DataError> {
        let mut view = ViewState::compile(&spec, &self.schema)?;
        view.rebuild(Rows {
            batches: &self.batches,
            slots: &self.slots,
        });
        let (serial, index) = store(&mut self.views, view);
        Ok(ViewId { serial, index })
    }

    /// Removes a view; `false` if it was already gone or belongs to another table. Its id
    /// never names another view (its storage may be reused).
    pub fn remove_view(&mut self, id: ViewId) -> bool {
        take(&mut self.views, id.index, id.serial)
    }

    /// The rows of view `id`, in order; `None` if it was removed or belongs to another
    /// table.
    pub fn view(&self, id: ViewId) -> Option<View<'_>> {
        let state = find(&self.views, id.index, id.serial)?;
        Some(View {
            state,
            batches: &self.batches,
            slots: &self.slots,
            reuses: &self.reuses,
        })
    }

    /// Adds grouped aggregates of the rows matching `spec`, filled from the current rows.
    ///
    /// # Errors
    /// [`DataError::UnknownColumn`] or [`DataError::UnsupportedColumn`] for a key or an
    /// aggregated column, [`DataError::NotNumeric`] for a sum or mean of a column that is
    /// not numeric, [`DataError::DuplicateName`] if two result columns share a name,
    /// [`DataError::LiteralMismatch`] for a filter that does not fit.
    pub fn add_groups(&mut self, spec: GroupSpec) -> Result<GroupsId, DataError> {
        let mut groups = GroupState::compile(&spec, &self.schema)?;
        groups.rebuild(Rows {
            batches: &self.batches,
            slots: &self.slots,
        });
        let (serial, index) = store(&mut self.groups, groups);
        Ok(GroupsId { serial, index })
    }

    /// Removes grouped aggregates; `false` if already gone or another table's. Their id
    /// never names other aggregates (their storage may be reused).
    pub fn remove_groups(&mut self, id: GroupsId) -> bool {
        take(&mut self.groups, id.index, id.serial)
    }

    /// The groups of `id`, in key order; `None` if removed or another table's.
    pub fn groups(&self, id: GroupsId) -> Option<Groups<'_>> {
        let state = find(&self.groups, id.index, id.serial)?;
        Some(Groups {
            state,
            rows: Rows {
                batches: &self.batches,
                slots: &self.slots,
            },
        })
    }

    fn check(&self, batch: &RecordBatch) -> Result<(), DataError> {
        let ours = self.schema.fields();
        let theirs = batch.schema_ref().fields();
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

    /// The last row of each distinct key in `batch`, in row order.
    fn latest_rows<'b>(
        &self,
        batch: &'b RecordBatch,
    ) -> Result<Vec<(usize, KeyRef<'b>)>, DataError> {
        let keys = batch.column(self.key_column).as_ref();
        let mut last: FxHashMap<KeyRef<'b>, usize> =
            FxHashMap::with_capacity_and_hasher(batch.num_rows(), Default::default());
        for row in 0..batch.num_rows() {
            let key = (self.key_read)(keys, row).ok_or(DataError::NullKey { row })?;
            last.insert(key, row);
        }
        let mut rows: Vec<(usize, KeyRef<'b>)> = last.into_iter().map(|(k, r)| (r, k)).collect();
        rows.sort_unstable_by_key(|&(row, _)| row);
        Ok(rows)
    }

    /// Each changed row's id, old and new place and new key. Checks every limit before
    /// taking any id, so an error leaves the table unchanged.
    fn plan<'b>(
        &mut self,
        batch: &RecordBatch,
        latest: &[(usize, KeyRef<'b>)],
    ) -> Result<Vec<Change<'b>>, DataError> {
        let segment = u32::try_from(self.batches.len()).map_err(|_| DataError::TooManyRows)?;
        u32::try_from(batch.num_rows()).map_err(|_| DataError::TooManyRows)?;
        let new_keys = latest
            .iter()
            .filter(|&&(_, key)| self.keys.get(key).is_none())
            .count();
        if new_keys > self.free.len() + (MAX_ROWS - self.slots.len()) {
            return Err(DataError::TooManyRows);
        }
        let mut plan = Vec::with_capacity(latest.len());
        for &(row, key) in latest {
            // Lossless: the batch has fewer than 2^32 rows (checked above).
            let at = Slot {
                batch: segment,
                row: row as u32,
            };
            plan.push(match self.keys.get(key) {
                Some(id) => (id, self.slots[id as usize], at, None),
                None => (self.allocate_id(), None, at, Some(key)),
            });
        }
        Ok(plan)
    }

    /// A free row id; [`plan`](Self::plan) checked that one exists.
    fn allocate_id(&mut self) -> RowId {
        if let Some(id) = self.free.pop() {
            return id;
        }
        self.slots.push(None);
        // Lossless: `plan` keeps the slot count at most 2^32.
        (self.slots.len() - 1) as RowId
    }

    /// Copies the live rows of `chunk` into new batches at the end of `out`: one if Arrow can
    /// build it, otherwise each half on its own, recursively, down to [`MIN_SPLIT_ROWS`]:
    /// fewer rows use fewer distinct strings, so they fit a narrow dictionary key type.
    fn gather_into(
        &self,
        chunk: &[(RowId, Slot)],
        out: &mut Vec<RecordBatch>,
    ) -> Result<(), DataError> {
        match self.gather(chunk) {
            Ok(batch) => {
                out.push(batch);
                Ok(())
            }
            Err(_) if chunk.len() > MIN_SPLIT_ROWS => {
                let (first, second) = chunk.split_at(chunk.len() / 2);
                self.gather_into(first, out)?;
                self.gather_into(second, out)
            }
            Err(error) => Err(error),
        }
    }

    /// The live rows of `chunk`, copied into one batch.
    fn gather(&self, chunk: &[(RowId, Slot)]) -> Result<RecordBatch, DataError> {
        // Only the batches the chunk reads go to Arrow, which may size a dictionary column by
        // all of its inputs' values.
        let mut position: FxHashMap<usize, usize> = FxHashMap::default();
        let mut sources = Vec::new();
        let indices: Vec<(usize, usize)> = chunk
            .iter()
            .map(|(_, slot)| {
                let batch = slot.batch as usize;
                let at = *position.entry(batch).or_insert_with(|| {
                    sources.push(batch);
                    sources.len() - 1
                });
                (at, slot.row as usize)
            })
            .collect();
        let columns = (0..self.schema.fields().len())
            .map(|column| {
                let arrays: Vec<&dyn Array> = sources
                    .iter()
                    .map(|&batch| self.batches[batch].column(column).as_ref())
                    .collect();
                interleave_column(&arrays, &indices)
            })
            .collect::<Result<Vec<ArrayRef>, _>>()
            .map_err(DataError::arrow)?;
        RecordBatch::try_new(Arc::clone(&self.schema), columns).map_err(DataError::arrow)
    }

    /// Deletes the rows past the window; how many.
    fn expire(&mut self) -> usize {
        let Some(window) = &self.window else {
            return 0;
        };
        let count = window.expired(Rows {
            batches: &self.batches,
            slots: &self.slots,
        });
        if count == 0 {
            return 0;
        }
        let gone: Vec<(RowId, Slot)> = (0..count)
            .filter_map(|index| {
                let id = window.rows.get(index)?;
                Some((id, self.slots[id as usize]?))
            })
            .collect();
        let rebuild = count.saturating_mul(REBUILD_RATIO) > self.len();
        if !rebuild {
            self.drop_from_views(gone.iter().copied());
        }
        for &(id, slot) in &gone {
            let at = slot.row_ref();
            let keys = self.batches[at.batch].column(self.key_column).as_ref();
            if let Some(key) = (self.key_read)(keys, at.row) {
                self.keys.remove(key);
            }
            self.release(id);
        }
        if rebuild {
            self.add_to_views(true, std::iter::empty());
        }
        gone.len()
    }

    /// Frees row `id`, already out of every view and group: its key's record ends.
    fn release(&mut self, id: RowId) {
        self.slots[id as usize] = None;
        self.free.push(id);
        if self.reuses.len() <= id as usize {
            self.reuses.resize(id as usize + 1, 0);
        }
        // Wrapping: an id freed 2^32 times could give an old record back; harmless odds.
        self.reuses[id as usize] = self.reuses[id as usize].wrapping_add(1);
        self.dead += 1;
    }

    /// Removes rows from every view, group and the window, each at the place its values were
    /// when it entered.
    fn drop_from_views(&mut self, rows: impl Iterator<Item = (RowId, Slot)> + Clone) {
        let source = Rows {
            batches: &self.batches,
            slots: &self.slots,
        };
        for (_, view) in self.views.iter_mut().flatten() {
            for (id, slot) in rows.clone() {
                view.remove(source, id, slot.row_ref());
            }
        }
        for (_, groups) in self.groups.iter_mut().flatten() {
            for (id, slot) in rows.clone() {
                groups.remove(source, id, slot.row_ref());
            }
        }
        if let Some(window) = &mut self.window {
            for (id, slot) in rows {
                window.rows.remove(source, id, slot.row_ref());
            }
        }
    }

    /// Adds changed rows to every view, group and the window, or rebuilds them.
    fn add_to_views(&mut self, rebuild: bool, rows: impl Iterator<Item = (RowId, Slot)> + Clone) {
        let source = Rows {
            batches: &self.batches,
            slots: &self.slots,
        };
        for (_, view) in self.views.iter_mut().flatten() {
            if rebuild {
                view.rebuild(source);
            } else {
                for (id, slot) in rows.clone() {
                    view.insert(source, id, slot.row_ref());
                }
            }
        }
        for (_, groups) in self.groups.iter_mut().flatten() {
            if rebuild {
                groups.rebuild(source);
            } else {
                for (id, slot) in rows.clone() {
                    groups.insert(source, id, slot.row_ref());
                }
            }
        }
        if let Some(window) = &mut self.window {
            if rebuild {
                window.rows.rebuild(source);
            } else {
                for (id, slot) in rows {
                    window.rows.insert(source, id, slot.row_ref());
                }
            }
        }
    }

    /// Compacts when due and enabled. A failure is recorded, not returned: the change that
    /// triggered it is applied and the table stays correct, only larger.
    fn compact_if_due(&mut self) {
        if !self.options.auto_compact {
            return;
        }
        let dead_majority = self.dead >= self.retry_at_dead && self.dead > self.len();
        // Counting only the batches stored since the last compaction: one that had to write
        // many small batches does not run again at every change.
        let added = self.batches.len().saturating_sub(self.compacted_batches);
        let too_many_batches = added > MAX_BATCHES && self.compaction_error.is_none();
        if !(dead_majority || too_many_batches) {
            return;
        }
        if let Err(error) = self.compact() {
            self.compaction_error = Some(error);
            self.retry_at_dead = self.dead.saturating_mul(2);
        }
    }
}

/// The entry at `index` if its serial is `serial`.
fn find<T>(entries: &[Option<(u64, T)>], index: usize, serial: u64) -> Option<&T> {
    match entries.get(index)? {
        Some((s, entry)) if *s == serial => Some(entry),
        _ => None,
    }
}

/// Stores `entry` under a new serial in the first free place (removed entries leave one),
/// so a model that replaces its view on every sort keeps one; returns the serial and index.
fn store<T>(entries: &mut Vec<Option<(u64, T)>>, entry: T) -> (u64, usize) {
    let serial = next_serial();
    match entries.iter().position(Option::is_none) {
        Some(index) => {
            entries[index] = Some((serial, entry));
            (serial, index)
        }
        None => {
            entries.push(Some((serial, entry)));
            (serial, entries.len() - 1)
        }
    }
}

/// Removes the entry at `index` if its serial is `serial`; whether it did.
fn take<T>(entries: &mut [Option<(u64, T)>], index: usize, serial: u64) -> bool {
    entries
        .get_mut(index)
        .and_then(|entry| entry.take_if(|(s, _)| *s == serial))
        .is_some()
}

/// `schema` with every field nullable: superseded rows and compaction never need a column
/// to be free of nulls, and batches may carry nulls the first batch's schema did not.
fn nullable(schema: &Schema) -> SchemaRef {
    let fields: Vec<Field> = schema
        .fields()
        .iter()
        .map(|f| f.as_ref().clone().with_nullable(true))
        .collect();
    Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone()))
}

/// The rows of a view, in its order. Cheap to get; reading never allocates.
///
/// A row is a [`RowRef`]: which of [`LiveTable::batches`] holds it, and where. Read its
/// values from that batch's columns:
///
/// ```
/// use std::sync::Arc;
/// use densa_data::arrow_array::cast::AsArray;
/// use densa_data::arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
/// use densa_data::{LiveTable, Order, ViewSpec};
///
/// let rows = RecordBatch::try_from_iter([
///     ("symbol", Arc::new(StringArray::from(vec!["AAA", "BBB"])) as ArrayRef),
///     ("price", Arc::new(Float64Array::from(vec![150.0, 90.0])) as ArrayRef),
/// ])?;
/// let mut table = LiveTable::new(rows.schema(), "symbol")?;
/// let cheapest = table.add_view(ViewSpec::new().sort_by("price", Order::Ascending))?;
/// table.upsert(rows)?;
///
/// let view = table.view(cheapest).unwrap();
/// let symbols: Vec<&str> = (0..view.len())
///     .filter_map(|i| view.row(i))
///     .map(|at| {
///         let batch = &table.batches()[at.batch];
///         // The column's Arrow type, as the table's schema has it (Utf8 here).
///         batch.column_by_name("symbol").unwrap().as_string::<i32>().value(at.row)
///     })
///     .collect();
/// assert_eq!(symbols, ["BBB", "AAA"]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy)]
pub struct View<'a> {
    state: &'a ViewState,
    batches: &'a [RecordBatch],
    slots: &'a [Option<Slot>],
    reuses: &'a [u32],
}

impl View<'_> {
    /// Rows in the view.
    pub fn len(&self) -> usize {
        self.state.len()
    }

    /// Whether the view is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Where the row at `index` (0 is first) is. `O(log n)`.
    pub fn row(&self, index: usize) -> Option<RowRef> {
        let id = self.state.get(index)?;
        self.slots
            .get(id as usize)
            .copied()
            .flatten()
            .map(Slot::row_ref)
    }

    /// The record of the row at `index`. `O(log n)`.
    pub fn record(&self, index: usize) -> Option<RecordId> {
        let id = self.state.get(index)?;
        Some(RecordId::new(id, reuse(self.reuses, id)))
    }

    /// Where `record`'s row is in the view, or `None` if it is not in it (filtered out, or
    /// its key deleted). `O(log n)`: binary searches comparing the row's sort values.
    pub fn position(&self, record: RecordId) -> Option<usize> {
        let (id, _) = live_slot(self.slots, self.reuses, record)?;
        let rows = Rows {
            batches: self.batches,
            slots: self.slots,
        };
        self.state.position(rows, id)
    }
}

/// How many times row id `id` was freed.
fn reuse(reuses: &[u32], id: RowId) -> u32 {
    reuses.get(id as usize).copied().unwrap_or(0)
}

/// `record`'s row id and slot, if the record's key is still live.
fn live_slot(slots: &[Option<Slot>], reuses: &[u32], record: RecordId) -> Option<(RowId, Slot)> {
    let (id, reused) = record.parts();
    let slot = (*slots.get(id as usize)?)?;
    (reuse(reuses, id) == reused).then_some((id, slot))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array};

    use super::*;

    fn ids(keys: Vec<i64>) -> RecordBatch {
        RecordBatch::try_from_iter([("id", Arc::new(Int64Array::from(keys)) as ArrayRef)]).unwrap()
    }

    #[test]
    fn replacing_a_view_again_and_again_keeps_one_place() {
        let mut table = LiveTable::new(ids(vec![]).schema(), "id").unwrap();
        let mut view = table.add_view(ViewSpec::new()).unwrap();
        for _ in 0..100 {
            let next = table.add_view(ViewSpec::new()).unwrap();
            table.remove_view(view);
            view = next;
        }
        assert!(table.views.len() <= 2, "{} places", table.views.len());
    }

    #[test]
    fn ids_of_deleted_keys_are_reused() {
        let mut table = LiveTable::new(ids(vec![]).schema(), "id").unwrap();
        for round in 0..100 {
            table
                .upsert(ids((0..10).map(|k| round * 10 + k).collect()))
                .unwrap();
            table
                .delete(&Int64Array::from(
                    (0..10).map(|k| round * 10 + k).collect::<Vec<_>>(),
                ))
                .unwrap();
        }
        assert!(
            table.slots.len() <= 10,
            "{} ids for 10 live keys",
            table.slots.len()
        );
    }

    #[test]
    fn a_failed_automatic_compaction_is_recorded_and_backs_off() {
        let mut table = LiveTable::new(ids(vec![]).schema(), "id").unwrap();
        table.dead = MIN_DEAD_ROWS;
        // A batch the table's schema cannot hold makes the next compaction fail.
        let other = RecordBatch::try_from_iter([(
            "other",
            Arc::new(arrow_array::StringArray::from(vec!["x"])) as ArrayRef,
        )])
        .unwrap();
        table.batches.push(other);
        table.slots.push(Some(Slot { batch: 0, row: 0 }));
        table.compact_if_due();
        assert!(table.compaction_error().is_some());
        assert_eq!(table.retry_at_dead, 2 * MIN_DEAD_ROWS);
    }

    #[test]
    fn automatic_compaction_counts_the_batches_stored_since_the_last_one() {
        let mut table = LiveTable::new(ids(vec![]).schema(), "id").unwrap();
        table.upsert(ids(vec![1])).unwrap();
        // As if the last compaction had to write 10 batches.
        table.compacted_batches = 10;
        while table.batches.len() < 10 + MAX_BATCHES {
            table.batches.push(ids(vec![]));
        }
        table.compact_if_due();
        assert_eq!(table.generation(), 0, "4,096 batches added");
        table.batches.push(ids(vec![]));
        table.compact_if_due();
        assert_eq!(table.generation(), 1, "4,097 batches added");
        assert_eq!(table.compacted_batches, 1);
    }
}
