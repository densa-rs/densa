//! A table's live rows as plain record batches, for whatever else reads Arrow.

use std::sync::Arc;

use arrow_array::builder::BooleanBufferBuilder;
use arrow_array::{BooleanArray, RecordBatch};
use arrow_select::filter::filter_record_batch;

use crate::{DataError, LiveTable};

impl LiveTable {
    /// The live rows (the latest values of each key) as record batches, all with the table's
    /// [schema](Self::schema) (stored batches keep the schema they arrived with, whose
    /// columns may not be nullable), to hand to something that reads Arrow: a SQL engine, a
    /// file writer, another thread (the table itself is used from one thread at a time). The
    /// columns of stored batches whose rows are all live are shared, not copied; the others
    /// are filtered down to their live rows. Rows come in no particular order.
    ///
    /// A table without live rows gives no batches: take the schema from
    /// [`schema`](Self::schema). A table fed many small batches gives as many small ones;
    /// [`compact`](Self::compact) first to have them merged (and superseded rows dropped).
    ///
    /// The batches are a snapshot: later upserts and deletes do not change them. Taking one
    /// allocates: a bit per stored row of partly live batches, and their live rows' copies.
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
        // A mask, a bit per row, only for the batches that are partly live.
        let mut masks: Vec<Option<BooleanBufferBuilder>> = batches
            .iter()
            .zip(&live)
            .map(|(batch, &count)| {
                let rows = batch.num_rows();
                (count > 0 && count < rows).then(|| {
                    let mut mask = BooleanBufferBuilder::new(rows);
                    mask.append_n(rows, false);
                    mask
                })
            })
            .collect();
        for row in self.live_rows() {
            if let Some(mask) = &mut masks[row.batch] {
                mask.set_bit(row.row, true);
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
                    Some(mut mask) => {
                        let mask = BooleanArray::new(mask.finish(), None);
                        filter_record_batch(batch, &mask).map_err(DataError::arrow)?
                    }
                };
                RecordBatch::try_new(Arc::clone(self.schema()), live.columns().to_vec())
                    .map_err(DataError::arrow)
            })
            .collect()
    }
}
