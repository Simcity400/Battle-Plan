//! Full-save alias projection from the current authenticated physical census.

use std::io;

use installed_content::save::{ActualCountry, CountryEmissionIdentity, Save, SaveDocument};
use replay::{
    cadence::{self, Cadence},
    frame::DecodedFrame,
    rows::{Recorded, TableId, tables},
};

use super::{country_context, invalid};

pub(super) fn parse(document: &SaveDocument, frame: &DecodedFrame) -> io::Result<Save> {
    match layout(frame) {
        Ok(Some(layout)) => document.project(&layout)?.parse(),
        Ok(None) => document.parse(),
        Err(reason) if document.has_repeated_country_tags()? => Err(invalid(format!(
            "full repeated-country save requires its physical census: {reason}"
        ))),
        // Unique standalone/historical saves retain their ordinary full parse.
        // Their unavailable census remains unavailable to the named comparisons;
        // no occurrence ledger or duplicate admission is manufactured here.
        Err(_) => document.parse(),
    }
}

pub(super) fn country_items<'a>(
    document: &'a SaveDocument,
    frame: &DecodedFrame,
) -> io::Result<Vec<&'a installed_content::parser::Item>> {
    match layout(frame) {
        Ok(Some(layout)) => document.representative_country_items(&layout),
        Ok(None) if document.has_repeated_country_tags()? => Err(invalid(
            "full repeated-country save requires its physical census",
        )),
        Ok(None) => Ok(document.original_country_items()?.iter().collect()),
        Err(reason) if document.has_repeated_country_tags()? => Err(invalid(format!(
            "full repeated-country save requires its physical census: {reason}"
        ))),
        Err(_) => Ok(document.original_country_items()?.iter().collect()),
    }
}

fn layout(frame: &DecodedFrame) -> io::Result<Option<Vec<CountryEmissionIdentity>>> {
    if cadence::cadence(TableId::ReservedCountryNaming) != Cadence::Hourly {
        return Err(invalid(
            "physical country emission census is not a current hourly table",
        ));
    }
    let rows = frame.complete::<tables::ReservedCountryNaming>()?;
    let [reserved] = rows else {
        return Err(invalid(
            "physical country emission census requires one reserved row",
        ));
    };
    let array = match &reserved.country_array {
        Recorded::NotRecorded => return Ok(None),
        Recorded::Value(None) => return Err(invalid("physical country emission census refused")),
        Recorded::Value(Some(array)) => array,
    };
    if array.first() != Some(&0) {
        return Err(invalid(
            "physical country emission census lacks its authenticated reserved entry",
        ));
    }
    let countries = country_context(frame)?;
    for country in countries.by_tag.values() {
        if array.get(country.canonical_slot as usize) != Some(&country.id) {
            return Err(invalid(
                "physical country census differs from its canonical Countries roster",
            ));
        }
    }
    let mut result = Vec::with_capacity(array.len().saturating_sub(1));
    for (physical_slot, &own_id) in array.iter().enumerate().skip(1) {
        let (actual, tag) = if own_id == 0 {
            // country_roster::capture records zero here only after exact
            // pointer equality with the authenticated shared reserved root.
            (ActualCountry::Reserved, reserved.tag.as_slice())
        } else {
            let country = countries.links.get(&own_id).ok_or_else(|| {
                invalid("physical country emission has an unresolved positive own ID")
            })?;
            if country.id != own_id || country.canonical_slot == 0 {
                return Err(invalid(
                    "physical country emission own ID differs from its canonical object",
                ));
            }
            (
                ActualCountry::Canonical(country.canonical_slot),
                country.tag.as_slice(),
            )
        };
        result.push(CountryEmissionIdentity {
            physical_slot: u32::try_from(physical_slot)
                .map_err(|_| invalid("physical country emission census exceeds u32"))?,
            own_id,
            actual,
            tag: tag.to_vec(),
        });
    }
    Ok(Some(result))
}

#[cfg(test)]
#[path = "country_emissions/tests.rs"]
mod tests;
