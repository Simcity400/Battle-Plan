
use replay_columnar::summary::Shape;
use std::io;

pub fn registry() -> io::Result<Vec<(usize, Shape)>> {
    #[allow(unused_mut, reason = "only the crate's tests add to the declarations")]
    let mut registry = dashboard_declare::summaries()?;
    #[cfg(any(test, feature = "test-support"))]
    rank_in_tests(&mut registry)?;
    Ok(registry)
}

#[cfg(any(test, feature = "test-support"))]
pub const TEST_RANK: u8 = 4;

#[cfg(any(test, feature = "test-support"))]
fn rank_in_tests(registry: &mut [(usize, Shape)]) -> io::Result<()> {
    use replay_columnar::summary::Source;
    let table = replay_data::rows::TableId::Countries;
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
