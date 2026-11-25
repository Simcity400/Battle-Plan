use crate::{
    codec::invalid_static as invalid,
    format::FormatVersion,
    frame::TableView,
    rows::{self, CountryFuelRow, RowCodec, TableId, tables},
};
use std::io;

pub fn with_award_recipients(
    rows: &[CountryFuelRow],
    recipients: &TableView,
    format: FormatVersion,
    read_hour: i32,
) -> io::Result<Vec<CountryFuelRow>> {
    let mut rows = rows
        .iter()
        .map(RowCodec::try_clone)
        .collect::<io::Result<Vec<_>>>()?;
    if format.table_refusal(TableId::FuelAwardRecipients).is_some() {
        return Ok(rows);
    }
    let recipients = match recipients {
        TableView::Complete(recipients) => Ok(recipients
            .rows::<tables::FuelAwardRecipients>()
            .ok_or_else(|| invalid("fuel award recipients view holds another table's rows"))?),
        TableView::Truncated { .. } => Err("fuel award recipients are truncated"),
        TableView::Unavailable { .. } => Err("fuel award recipients are unavailable"),
        TableView::NotRecorded { .. } => Err("fuel award recipients are not recorded"),
    };
    rows::assemble_award_recipients(&mut rows, recipients, read_hour);
    Ok(rows)
}
