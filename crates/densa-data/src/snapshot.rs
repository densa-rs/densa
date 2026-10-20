//! A table's live rows as plain record batches, for whatever else reads Arrow.

use std::sync::Arc;

use arrow_array::{BooleanArray, RecordBatch};
use arrow_select::filter::filter_record_batch;

use crate::{DataError, LiveTable};

impl LiveTable {
    /// The live rows (the latest values of each key) as record batches, all with the table's
    /// [schema](Self::schema) (stored batches keep the schema they arrived with, whose
    /// columns may not be nullable), to hand to something that reads Arrow: a SQL engine, a
    /// file writer, another thread. The columns of stored batches whose rows are all live
    /// are shared, not copied; the others are filtered down to their live rows. The rows
    /// come in storage order, not a view's.
    ///
    /// The batches are a snapshot: later upserts and deletes do not change them. Taking one
    /// allocates, and copies the rows of partly superseded batches (fewer after
    /// [`compact`](Self::compact)).
    ///
    /// # Errors
    /// [`DataError::Arrow`] if Arrow cannot filter a batch or give it the table's schema
    /// (neither should happen to rows the table accepted).
    pub fn snapshot(&self) -> Result<Vec<RecordBatch>, DataError> {
        let batches = self.batches();
        let mut live = vec![0_usize; batches.len()];
        for row in self.live_rows() {
            live[row.batch] += 1;
        }
        // A mask only for the batches that are partly live.
        let mut masks: Vec<Option<Vec<bool>>> = batches
            .iter()
            .zip(&live)
            .map(|(batch, &count)| {
                (count > 0 && count < batch.num_rows()).then(|| vec![false; batch.num_rows()])
            })
            .collect();
        for row in self.live_rows() {
            if let Some(mask) = &mut masks[row.batch] {
                mask[row.row] = true;
            }
        }
        batches
            .iter()
            .zip(live)
            .zip(masks)
            .filter(|((_, count), _)| *count > 0)
            .map(|((batch, _), mask)| {
                let live = match mask {
                    None => batch.clone(),
                    Some(mask) => filter_record_batch(batch, &BooleanArray::from(mask))
                        .map_err(DataError::arrow)?,
                };
                RecordBatch::try_new(Arc::clone(self.schema()), live.columns().to_vec())
                    .map_err(DataError::arrow)
            })
            .collect()
    }
}
