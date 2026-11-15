//! Which tables the recorder summarizes and how ([`replay_columnar::summary`]): exactly what the
//! dashboard's declarations read ([`dashboard_declare::summaries`]). The schema digest does not
//! cover this list: a reader takes what each block's index names.

use replay_columnar::summary::Shape;
use std::io;

/// Every summarized table and its pieces' shape, ascending by table.
pub fn registry() -> io::Result<Vec<(usize, Shape)>> {
    #[allow(unused_mut, reason = "only the crate's tests add to the declarations")]
    let mut registry = dashboard_declare::summaries()?;
    #[cfg(test)]
    rank_in_tests(&mut registry)?;
    Ok(registry)
}

/// How many keys the crate's tests rank Countries' political power by.
#[cfg(test)]
pub(crate) const TEST_RANK: u8 = 4;

/// The crate's tests rank Countries' political power: a ranked column of a recorded table, which
/// a synthetic journal can hold (a derived table's rows are its writer's to write).
#[cfg(test)]
fn rank_in_tests(registry: &mut [(usize, Shape)]) -> io::Result<()> {
    use replay_columnar::summary::Source;
    let table = crate::rows::TableId::Countries;
    let power = dashboard_declare::field(table, "political_power_raw")?;
    let column = registry
        .iter_mut()
        .find(|(held, _)| *held == table.index())
        .and_then(|(_, shape)| match shape {
            Shape::Values(columns) => columns.iter_mut().find(|column| matches!(column.source, Source::Cell(cell) if cell == power)),
            _ => None,
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Countries' political power is not summarized"))?;
    column.rank = column.rank.max(TEST_RANK);
    Ok(())
}
