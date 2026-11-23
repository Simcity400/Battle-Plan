
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;
use std::sync::Arc;
use super::ideology_groups;
use viewer_counter_kit::{CounterArt, Countries};
use viewer_foundation::country_index::CountryIndex;
use viewer_foundation::tables::core;
#[cfg(test)]
use std::collections::BTreeMap;

pub fn warm_initial_flags(frame: &DecodedFrame, assets: &CounterArt) -> io::Result<()> {
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
