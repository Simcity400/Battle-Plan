
#[cfg(test)]
use replay_frame::frame::DecodedFrame;

use crate::catalogue::Refusals;
use replay_data::rows::{
    AirWingRow, RaidRow, RaidTooltipOperandRow, Recorded, TableId, mask, tables,
};
use replay_frame::frame::Tables;
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

#[cfg(test)]
mod tests;

pub fn recorded_names<F: Tables + ?Sized>(
    frame: &F,
    requested: &HashSet<u64>,
    refusals: &mut Refusals,
) -> HashMap<u64, Vec<u8>> {
    let mut names = HashMap::new();
    if requested.is_empty() {
        return names;
    }
    let wings = match frame.complete_rows::<tables::AirWings>(mask::<AirWingRow>(&["name"])) {
        Ok(rows) => rows,
        Err(why) => {
            for reference in requested {
                refusals.note(why.to_string(), reference.to_string());
            }
            return names;
        }
    };
    let mut legacy = HashSet::new();
    for &reference in requested {
        let wing = wings.get(u128::from(reference));
        let reason = match wing {
            Err(why) => Some(why.to_string()),
            Ok(None) => Some(format!("air wings lacks WING {reference:#x}")),
            Ok(Some(wing)) => match &wing.name {
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

pub(crate) fn raid_name<F: Tables + ?Sized>(
    frame: &F,
    reference: u64,
) -> Result<Vec<u8>, String> {
    raid_names(frame, &HashSet::from([reference]))
        .remove(&reference)
        .expect("requested wing name")
}

fn raid_names<F: Tables + ?Sized>(
    frame: &F,
    requested: &HashSet<u64>,
) -> HashMap<u64, Result<Vec<u8>, String>> {
    let mut names = HashMap::new();
    if requested.is_empty() {
        return names;
    }
    let read = mask::<RaidRow>(&["air_wing", "phase", "unit"]);
    let raids = match frame.complete_rows::<tables::Raids>(read) {
        Ok(rows) => rows,
        Err(why) => {
            for &reference in requested {
                names.insert(reference, Err(why.to_string()));
            }
            return names;
        }
    };
    let mut selected: HashMap<u64, Vec<Cow<'_, RaidRow>>> = HashMap::new();
    let mut ids = HashSet::new();
    for raid in raids.iter() {
        let raid = match raid {
            Ok(raid) => raid,
            Err(why) => {
                for &reference in requested {
                    names.insert(reference, Err(why.to_string()));
                }
                return names;
            }
        };
        if raid.air_wing != 0
            && requested.contains(&raid.air_wing)
            && raid.phase != engine_native_air::craid_instance::PHASE_ENDED
        {
            ids.insert(u64::from(raid.id) << 32 | u64::from(raid.reference_type));
            selected.entry(raid.air_wing).or_default().push(raid);
        }
    }
    let operands = frame
        .complete_rows::<tables::RaidTooltipOperands>(TableId::RaidTooltipOperands.full_mask());
    let mut by_raid: HashMap<u64, Vec<Cow<'_, RaidTooltipOperandRow>>> = HashMap::new();
    let mut unread = None;
    if let Ok(rows) = &operands {
        for row in rows.keyed(|key| ids.contains(&((key >> 32) as u64))) {
            match row {
                Ok(row) => by_raid.entry(row.raid).or_default().push(row),
                Err(why) => {
                    unread = Some(why.to_string());
                    break;
                }
            }
        }
    }
    for &reference in requested {
        let result = (|| {
            let selected = selected.get(&reference).ok_or_else(|| {
                format!("WING {reference:#x} name outside selected active raids is not recorded")
            })?;
            operands.as_ref().map_err(|why| why.to_string())?;
            if let Some(why) = &unread {
                return Err(why.clone());
            }
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

fn raid_bytes(
    raid: &RaidRow,
    reference: u64,
    rows: &[Cow<'_, RaidTooltipOperandRow>],
) -> Result<Vec<u8>, String> {
    use engine_native_air::raid_tooltip_operands::{
        ARMY_RESOLVED,
        NAME,
        REFERENCES,
        UNREADABLE,
        WING_NAME,
        WING_RESOLVED,
        wing_name_error,
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
