
use super::*;
use replay_data::rows::{CountryAppearanceRow, CountryRow};

pub fn warm_initial_flags(frame: &DecodedFrame, assets: &CounterAssets) -> io::Result<()> {
    countries(frame)?.warm_flags(assets);
    Ok(())
}

fn countries(frame: &DecodedFrame) -> io::Result<Countries> {
    core::<tables::CountryLinks>(frame)?;
    let source = core::<tables::Countries>(frame)?;
    let country_rows = source
        .iter()
        .map(|country| (country.canonical_slot, country))
        .collect();
    let appearances = rows::<tables::CountryAppearance>(frame)?
        .rows()
        .iter()
        .map(|row| (row.country, row))
        .collect();
    Ok(Countries {
        tags: source
            .iter()
            .map(|country| {
                (
                    country.canonical_slot,
                    String::from_utf8_lossy(&country.tag).into_owned(),
                )
            })
            .collect(),
        flag_tags: flag_tags(&country_rows, &appearances),
        groups: ideology_groups(frame, &country_rows)?,
        ..Default::default()
    })
}

pub(super) fn flag_tags(
    countries: &BTreeMap<u32, &CountryRow>,
    appearances: &BTreeMap<u32, &CountryAppearanceRow>,
) -> BTreeMap<u32, FlagTags> {
    countries
        .iter()
        .filter_map(|(&slot, country)| {
            let row = appearances.get(&country.id)?;
            Some((
                slot,
                (
                    std::str::from_utf8(&row.cosmetic_tag)
                        .map(str::to_owned)
                        .map_err(|_| "country cosmetic tag is not UTF-8".into()),
                    content_reader_country_names::country_names::original_tag(country, row)
                        .map(str::to_owned),
                ),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests;
