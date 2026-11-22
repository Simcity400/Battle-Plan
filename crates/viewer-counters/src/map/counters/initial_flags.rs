
use super::*;

pub fn warm_initial_flags(frame: &DecodedFrame, assets: &CounterAssets) -> io::Result<()> {
    countries(frame)?.warm_flags(assets);
    Ok(())
}

fn countries(frame: &DecodedFrame) -> io::Result<Countries> {
    let index = CountryIndex::read(frame)?;
    let source = core::<tables::Countries>(frame)?;
    let country_rows = source
        .iter()
        .map(|country| (country.canonical_slot, country))
        .collect();
    Ok(Countries {
        tags: index.tags().clone(),
        flag_tags: index.flag_tags().clone(),
        groups: ideology_groups(frame, &country_rows)?,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests;
