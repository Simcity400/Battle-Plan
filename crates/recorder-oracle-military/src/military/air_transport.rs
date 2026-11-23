use std::{collections::BTreeMap, io};

use content_reader_parser::tokens::Token;
use content_reader_script::sub_units::SubUnits;
use content_reader_save as save;
use replay_data::{rows::AirWingRow, rows::Recorded, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::super::{
    ContentRead, Report, Spellings, TokenNames, invalid, required_context, reverse_spellings,
};

const NULL_POOL_TYPE: &[u8] = b"none";

pub fn compare(
    frame: &DecodedFrame,
    save: &save::Save,
    sub_units: &ContentRead<'_, SubUnits>,
    tokens: &io::Result<TokenNames<'_>>,
    report: &mut Report,
) -> io::Result<()> {
    let mut wings = BTreeMap::new();
    for wing in frame.complete::<tables::AirWings>()? {
        if wing.id == 0 || wings.insert(wing.id, wing).is_some() {
            return Err(invalid("null or duplicate full air wing handle"));
        }
    }
    let mut saved = BTreeMap::new();
    for wing in &save.air.wings {
        if saved.insert(wing.id, wing).is_some() {
            return Err(invalid("duplicate saved air wing in the air pack"));
        }
    }
    let spellings = tokens
        .as_ref()
        .map_err(ToString::to_string)
        .map(reverse_spellings);
    for (id, expected) in saved {
        let Some(actual) = wings.get(&id) else {
            continue;
        };
        let label = format!("air wing type={} id={}", id as u32, id >> 32);
        let mut admitted = None;
        report.attempt(&format!("{label}.pool_subunit"), |report| {
            admitted = pool(actual, expected, sub_units, &spellings, &label, report)?;
            Ok(())
        });
        raid(actual, expected, admitted, &label, report);
    }
    Ok(())
}

fn pool(
    actual: &AirWingRow,
    expected: &save::air::Wing,
    sub_units: &ContentRead<'_, SubUnits>,
    spellings: &Result<Spellings<'_>, String>,
    label: &str,
    report: &mut Report,
) -> io::Result<Option<bool>> {
    let index = match actual.pool_subunit {
        Recorded::NotRecorded => {
            report.limitation(
                "air wing pool sub-unit index not recorded by this format",
                format!("{label}.pool_subunit"),
            );
            return Ok(None);
        }
        Recorded::Value(None) => {
            report.difference(
                format!("{label}.pool_subunit: guarded identity or memo refusal"),
                "unavailable",
                &expected.pool_type,
            );
            return Ok(None);
        }
        Recorded::Value(Some(index)) => index,
    };
    let content = if index == 0 {
        None
    } else {
        let sub_units = required_context("verified sub-unit database", sub_units)?;
        let name = sub_units
            .name(index)
            .ok_or_else(|| {
                invalid(format!(
                    "{label}: pool sub-unit index {index} is absent from recorded content"
                ))
            })?
            .map_err(invalid)?;
        Some((sub_units, name))
    };
    let pool_type = match content {
        None => NULL_POOL_TYPE,
        Some((_, name)) => {
            let identity = Token::c_str(name.as_bytes());
            if identity.is_rejected() {
                return Err(invalid(format!(
                    "{label}: pool sub-unit has an unsupported rejected lexer token"
                )));
            }
            let literal = required_context("recorded lexical dictionary", spellings)?
                .get(&identity).ok_or_else(|| {
                invalid(format!(
                    "{label}: pool sub-unit token is absent from the recorded lexical dictionary"
                ))
            })?;
            literal.ok_or_else(|| {
                invalid(format!(
                    "{label}: pool sub-unit token has ambiguous recorded lexical identities"
                ))
            })?
        }
    };
    report.compared("AirWings.pool_subunit", 1);
    if pool_type != expected.pool_type {
        report.difference(
            format!("{label}.pool_subunit spelling"),
            pool_type,
            &expected.pool_type,
        );
    }
    if actual.mission_mask != 0
        || actual.eligibility_veto != 0
        || actual.eligibility_counter > actual.eligibility_limit
    {
        return Ok(Some(false));
    }
    let Some((sub_units, name)) = content else {
        return Ok(Some(false));
    };
    let mask = sub_units.category(name.as_bytes()).map_err(invalid)?;
    let transport = content_reader_script::equipment_stats::category_mask("air_transport")
        .expect("the transport category exists");
    Ok(Some(mask & transport != 0))
}

const RAID_NOT_RECORDED: &str = "air wing raid reference not recorded at this hour: the wing writers were not observed or one ran across the read, or the reference failed its guarded read or raid identity check";

fn raid(
    actual: &AirWingRow,
    expected: &save::air::Wing,
    admitted: Option<bool>,
    label: &str,
    report: &mut Report,
) {
    let reference = match &actual.raid_reference {
        Recorded::NotRecorded => {
            report.limitation(
                "air wing raid reference not recorded by this format",
                format!("{label}.raid_reference"),
            );
            return;
        }
        Recorded::Value(None) => {
            if actual.air_base != 0 {
                match actual.assigned_to_raid() {
                    Some(false) => {
                        report.compared("AirWings.raid_reference", 1);
                        if let Some(saved) = expected.raid {
                            report.difference(
                                format!("{label}.raid_reference.resolves"),
                                false,
                                saved,
                            );
                        }
                    }
                    Some(true) => {
                        report.compared("AirWings.raid_reference", 1);
                        report.difference(
                            format!("{label}.raid_reference: certified as resolving, recorded without its reference"),
                            "none",
                            expected.raid,
                        );
                    }
                    None => report.limitation(RAID_NOT_RECORDED, format!("{label}.raid_reference")),
                }
                return;
            }
            match admitted {
                Some(true) => report.limitation(RAID_NOT_RECORDED, format!("{label}.raid_reference")),
                Some(false) => report.limitation("raid-reference capture is limited to transport wings with zero mission mask/veto and counter <= limit; this saved raid link is outside that capture demand", format!("{label}.raid_reference: saved {:?}", expected.raid)),
                None => report.limitation("raid-reference capture demand cannot be certified without the independently compared pool sub-unit index", format!("{label}.raid_reference")),
            }
            return;
        }
        Recorded::Value(Some(reference)) => reference,
    };
    report.compared("AirWings.raid_reference", 1);
    match expected.raid {
        Some(saved) => {
            if reference.raw != saved {
                report.difference(format!("{label}.raid_reference.raw"), reference.raw, saved);
            }
            if !reference.resolves {
                report.difference(format!("{label}.raid_reference.resolves"), false, true);
            }
        }
        None if reference.resolves => report.difference(format!("{label}.raid_reference"), reference.raw, "omitted: the full reference must not resolve"),
        None => report.limitation("the air-wing writer omits null and stale raid references; a save certifies non-resolution but not their exact raw bits. A matched independent native read witness is required for those bits", format!("{label}.raid_reference.raw={}", reference.raw)),
    }
}

#[cfg(test)]
mod tests;
