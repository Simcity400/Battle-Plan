//! Selected prepared state enters the ordinary decoder at its saved hour.
use super::*;

impl FrameDecoder {
    /// Restore a selected preparation accumulator, including acquisition provenance.
    /// The next update still follows the ordinary branch and baseline rules.
    pub fn resume_for_tables(tables: &[TableId], mut frame: DecodedFrame) -> io::Result<Self> {
        let mut decoder = Self::for_tables(tables);
        let selected = decoder
            .selected
            .as_ref()
            .expect("table selection initialized");
        for table in TableId::ALL {
            let view = &frame.tables[table.index()];
            match &**view {
                TableView::Complete(rows) => validate_complete(table, rows, frame.hour)?,
                TableView::Truncated { rows, missing } => {
                    validate_rows(table, rows)?;
                    if *missing == 0 {
                        return Err(invalid("truncated checkpoint has no missing rows"));
                    }
                }
                TableView::Unavailable { reason } => {
                    if reason.is_empty() {
                        return Err(invalid("unavailable checkpoint needs a reason"));
                    }
                }
                TableView::NotRecorded { .. } => {}
            }
            if view
                .sampled_at()
                .is_some_and(|hour| hour > frame.hour || !matches!(&**view, TableView::Complete(_)))
            {
                return Err(invalid("invalid checkpoint sample hour"));
            }
            if !selected[table.index()] {
                frame.tables[table.index()] = not_selected();
            }
        }
        decoder.current = Some(frame);
        Ok(decoder)
    }

    /// The saved or most recently reconstructed hour.
    pub fn current(&self) -> Option<&DecodedFrame> {
        self.current.as_ref()
    }
}
