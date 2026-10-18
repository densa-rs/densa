/// Identifies a widget in a [`Ui`](crate::Ui).
///
/// An id stays valid until its widget is removed. A removed widget's slot can hold another
/// widget later, under another generation, so an old id never reaches the new widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WidgetId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl WidgetId {
    pub(crate) fn slot(self) -> usize {
        self.index as usize
    }
}
