//! Retained wing-name text. Current capture failures never fall back; older
//! prefixes can use the authenticated active-raid projection shared by
//! effect descriptions.

use crate::catalogue::Refusals;
use replay::{
    frame::DecodedFrame,
    rows::{AirWingRow, RaidRow, RaidTooltipOperandRow, Recorded, tables},
};
use std::collections::{HashMap, HashSet};

#[cfg(test)]
mod tests;

pub(super) fn recorded_names(
    frame: &DecodedFrame,
    requested: &HashSet<u64>,
    refusals: &mut Refusals,
) -> HashMap<u64, Vec<u8>> {
    let mut names = HashMap::new();
    if requested.is_empty() {
        return names;
    }
    let wings = match frame.complete::<tables::AirWings>() {
        Ok(rows) => rows,
        Err(why) => {
            for reference in requested {
                refusals.note(why.to_string(), reference.to_string());
            }
            return names;
        }
    };
    let mut selected: HashMap<u64, Option<&AirWingRow>> = HashMap::new();
    for wing in wings.iter().filter(|wing| requested.contains(&wing.id)) {
        selected
            .entry(wing.id)
            .and_modify(|row| *row = None)
            .or_insert(Some(wing));
    }
    let mut legacy = HashSet::new();
    for &reference in requested {
        let reason = match selected.get(&reference) {
            None => Some(format!("AirWings lacks WING {reference:#x}")),
            Some(None) => Some("AirWings has duplicate wing references".into()),
            Some(Some(wing)) => match &wing.name {
                Recorded::Value(Some(bytes)) => {
                    names.insert(reference, bytes.clone());
                    None
                }
                Recorded::Value(None) => Some(
                    "Air wing stored name capture failed: guarded read or name arena refusal"
                        .into(),
                ),
                Recorded::NotRecorded => {
                    legacy.insert(reference);
                    None
                }
            },
        };
        if let Some(reason) = reason {
            refusals.note(reason, reference.to_string());
        }
    }
    for (reference, result) in raid_names(frame, &legacy) {
        match result {
            Ok(bytes) => {
                names.insert(reference, bytes);
            }
            Err(why) => refusals.note(why, reference.to_string()),
        }
    }
    names
}

pub(crate) fn raid_name(frame: &DecodedFrame, reference: u64) -> Result<Vec<u8>, String> {
    raid_names(frame, &HashSet::from([reference]))
        .remove(&reference)
        .expect("requested wing name")
}

fn raid_names(
    frame: &DecodedFrame,
    requested: &HashSet<u64>,
) -> HashMap<u64, Result<Vec<u8>, String>> {
    let mut names = HashMap::new();
    if requested.is_empty() {
        return names;
    }
    let raids = match frame.complete::<tables::Raids>() {
        Ok(rows) => rows,
        Err(why) => {
            for &reference in requested {
                names.insert(reference, Err(why.to_string()));
            }
            return names;
        }
    };
    let mut selected: HashMap<u64, Vec<&RaidRow>> = HashMap::new();
    let mut ids = HashSet::new();
    for raid in raids.iter().filter(|raid| {
        raid.air_wing != 0
            && requested.contains(&raid.air_wing)
            && raid.phase != engine::craid_instance::PHASE_ENDED
    }) {
        selected.entry(raid.air_wing).or_default().push(raid);
        ids.insert(u64::from(raid.id) << 32 | u64::from(raid.reference_type));
    }
    let operands = frame.complete::<tables::RaidTooltipOperands>();
    let mut by_raid: HashMap<u64, Vec<&RaidTooltipOperandRow>> = HashMap::new();
    if let Ok(rows) = &operands {
        for row in rows.iter().filter(|row| ids.contains(&row.raid)) {
            by_raid.entry(row.raid).or_default().push(row);
        }
    }
    for &reference in requested {
        let result = (|| {
            let selected = selected.get(&reference).ok_or_else(|| {
                format!("WING {reference:#x} name outside selected active raids is not recorded")
            })?;
            operands.as_ref().map_err(|why| why.to_string())?;
            let mut name = None;
            for raid in selected {
                let id = u64::from(raid.id) << 32 | u64::from(raid.reference_type);
                let bytes =
                    raid_bytes(raid, reference, by_raid.get(&id).map_or(&[], Vec::as_slice))?;
                if name.as_ref().is_some_and(|previous| previous != &bytes) {
                    return Err(format!(
                        "active-raid WING {reference:#x} name (raid {id:#x}): selected raids recorded different names for the same wing"
                    ));
                }
                name = Some(bytes);
            }
            name.ok_or_else(|| format!("WING {reference:#x} name is not recorded"))
        })();
        names.insert(reference, result);
    }
    names
}

// The name selection resolves an Army before the wing. Retained wing names
// have no source marker; callers display these bytes with stored_text.
fn raid_bytes(
    raid: &RaidRow,
    reference: u64,
    rows: &[&RaidTooltipOperandRow],
) -> Result<Vec<u8>, String> {
    use engine::raid_tooltip_operands::{
        ARMY_RESOLVED, NAME, REFERENCES, UNREADABLE, WING_NAME, WING_RESOLVED, wing_name_error,
    };
    let id = u64::from(raid.id) << 32 | u64::from(raid.reference_type);
    let fail = |why: &str| format!("active-raid WING {reference:#x} name (raid {id:#x}): {why}");
    if rows.iter().any(|row| row.kind == UNREADABLE) {
        return Err(fail("raid operands were unreadable when recorded"));
    }
    let value = |kind: u32, key: u32| -> Result<i64, String> {
        let mut values = rows.iter().filter(|row| row.kind == kind && row.key == key);
        let row = values
            .next()
            .ok_or_else(|| fail(&format!("operand {kind}:{key:#x} is not recorded")))?;
        if values.next().is_some() {
            return Err(fail(&format!("operand {kind}:{key:#x} is duplicated")));
        }
        Ok(row.raw)
    };
    if value(REFERENCES, 0)? as u64 != raid.unit || value(REFERENCES, 1)? as u64 != reference {
        return Err(fail("naming reference differs from recorded raid"));
    }
    if value(NAME, ARMY_RESOLVED)? != 0 {
        return Err(fail(
            "wing name was not selected: native Army-before-wing branch",
        ));
    }
    if value(NAME, WING_RESOLVED)? == 0 {
        return Err(fail("wing reference did not resolve when recorded"));
    }
    let length = value(WING_NAME, u32::MAX)?;
    if length < 0 {
        return Err(fail(wing_name_error(length)));
    }
    let length = usize::try_from(length).map_err(|_| fail("name length overflows"))?;
    if length.div_ceil(8) > rows.len() {
        return Err(fail("name byte chunks were not recorded in full"));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve(length)
        .map_err(|why| fail(&why.to_string()))?;
    for (index, row) in rows
        .iter()
        .filter(|row| row.kind == WING_NAME && row.key != u32::MAX)
        .enumerate()
    {
        if index >= length.div_ceil(8) || usize::try_from(row.key).ok() != Some(index) {
            return Err(fail(
                "name byte chunks are duplicate, extra or out of order",
            ));
        }
        bytes.extend_from_slice(&row.raw.to_le_bytes()[..(length - bytes.len()).min(8)]);
    }
    if bytes.len() != length {
        return Err(fail("name byte chunks were not recorded in full"));
    }
    Ok(bytes)
}
