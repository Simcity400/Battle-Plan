
use super::{
    Arc, CounterAssets, Countries, CountryIndex, DecodedFrame, core, ideology_groups, io, tables,
};
#[cfg(test)]
use super::{BTreeMap, Batch, Carry, Child, Kinds, LazyFlags, LazyTextures, MissionMemo, Need, Shown, Vision, draw_flag, hour_tables_profiled, rows, type_tint};

pub fn warm_initial_flags(frame: &DecodedFrame, assets: &CounterAssets) -> io::Result<()> {
    countries(frame)?.warm_flags(assets);
    Ok(())
}

fn countries(frame: &DecodedFrame) -> io::Result<Countries> {
    let index = Arc::new(CountryIndex::read(frame)?);
    let source = core::<tables::Countries>(frame)?;
    let country_rows = source
        .iter()
        .map(|country| (country.canonical_slot, country))
        .collect();
    Ok(Countries {
        index: Some(index),
        groups: ideology_groups(frame, &country_rows)?,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests;
