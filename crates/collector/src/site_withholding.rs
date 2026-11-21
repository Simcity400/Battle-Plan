use crate::read_accounting;
use replay_data::rows::TableId;
use replay_frame::frame::{FrameInput, TableView};
use std::io;

pub(crate) fn apply(source: &mut FrameInput) -> io::Result<()> {
    for table in TableId::ALL {
        if let Some(key) = read_accounting::withheld(table) {
            source.tables[table.index()] = TableView::Unavailable {
                reason: engine_allocation::string(
                    engine_sites::located()
                        .and_then(|located| located.message(key))
                        .unwrap_or("a game site this table reads is withheld on this build"),
                )?,
            }
            .into();
        }
    }
    Ok(())
}
