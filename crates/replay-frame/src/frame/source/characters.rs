use super::*;
use replay_data::rows::{CharacterRow, tables};

#[derive(Clone)]
pub struct SharedCharacters(SharedView);
impl std::fmt::Debug for SharedCharacters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedCharacters")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}
impl SharedCharacters {
    pub fn rows(&self) -> Rows<'_, CharacterRow> {
        rows(&self.0)
    }
    pub fn status(&self) -> OperandStatus<'_> {
        OperandStatus::for_table(&self.0, TableId::Characters)
    }
}
impl From<SharedView> for SharedCharacters {
    fn from(view: SharedView) -> Self {
        Self(view)
    }
}
pub(super) fn rows(view: &TableView) -> Rows<'_, CharacterRow> {
    match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
            rows.rows::<tables::Characters>().unwrap_or_default().into()
        }
        _ => Rows::empty(),
    }
}
