//! Frame availability and admission for the typed fuel-demand join.
use crate::{
    codec::invalid_static as invalid,
    format::FormatVersion,
    frame::TableView,
    rows::{self, CountryFuelRow, RowCodec, TableId, tables},
};
use std::io;

/// One CountryFuel read's rows with each demand pack's lists in place.
/// Recordings before [`crate::rows::compatibility::FUEL_DEMAND_PARTS_FORMAT`]
/// hold them in the pack, as does any pack that still holds a list; a later
/// pack's lists are FuelDemandParts rows (`parts`, the same hour's view),
/// which must then be complete. A pack whose parts are missing or
/// inconsistent keeps empty lists and names why in its `reason`, on which
/// every consumer of the pack already refuses it.
pub fn with_demand_parts(
    rows: &[CountryFuelRow],
    parts: &TableView,
    format: FormatVersion,
) -> io::Result<Vec<CountryFuelRow>> {
    let mut rows = rows
        .iter()
        .map(RowCodec::try_clone)
        .collect::<io::Result<Vec<_>>>()?;
    if format.table_refusal(TableId::FuelDemandParts).is_some() {
        return Ok(rows);
    }
    let parts = match parts {
        TableView::Complete(parts) => Ok(parts
            .rows::<tables::FuelDemandParts>()
            .ok_or_else(|| invalid("FuelDemandParts view holds another table's rows"))?),
        TableView::Truncated { .. } => Err("fuel demand parts are truncated"),
        TableView::Unavailable { .. } => Err("fuel demand parts are unavailable"),
        TableView::NotRecorded { .. } => Err("fuel demand parts are not recorded"),
    };
    rows::assemble_demand_parts(&mut rows, parts);
    Ok(rows)
}

#[cfg(test)]
mod tests;
