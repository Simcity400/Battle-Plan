//! Saved pool identity and the transport wing's independently captured raid link.
use std::{collections::BTreeMap, io};

use installed_content::{tokens::Token, unit_leader::SubUnits};
use installed_content_save as save;
use replay::{
    frame::DecodedFrame,
    rows::{AirWingRow, Recorded, tables},
};

use super::super::{
    ContentRead, Report, Spellings, TokenNames, invalid, required_context, reverse_spellings,
};

/// Subunit initialization sets the name to
/// static token 0x165. Registration
/// sequence binds it to `none`; the wing-pool writer writes that token for a null subtype.
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
    // Build the native spelling join once, rather than scanning the complete
    // dictionary again for every wing in the same pool.
    let spellings = tokens
        .as_ref()
        .map_err(ToString::to_string)
        .map(reverse_spellings);
    for (id, expected) in saved {
        // The ordinary wing comparison owns missing/extra census differences.
        let Some(actual) = wings.get(&id) else {
            continue;
        };
        let label = format!("air wing type={} id={}", id as u32, id >> 32);
        let mut admitted = None;
        // A content failure must not suppress a separately captured live-raid
        // difference. Keep this field's refusal in its own comparison group.
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
            // Token insertion can reuse a token registered earlier
            // under different case. The wing-pool writer writes its native reverse spelling,
            // not this database's first authored key spelling.
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
    let transport = installed_content::equipment::category_mask("air_transport")
        .expect("the transport category exists");
    Ok(Some(mask & transport != 0))
}

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
            match admitted {
                Some(true) => report.difference(format!("{label}.raid_reference: guarded read or registry/memo refusal"), "unavailable", expected.raid),
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
