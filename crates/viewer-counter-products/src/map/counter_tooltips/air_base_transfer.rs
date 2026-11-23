use content_reader_key_names::unit_leader::AirWings as _;
use super::{Arg, Context, Tooltip, localise, refuse};
use replay_data::{rows::AirBaseRow, rows::AirWingRow, rows::RaidSource, rows::tables};

const REASONS: [&str; 8] = [
    "AIR_BASE_RIGHT_CLICK_REASON_NOT_CARRIER_CAPABLE",
    "AIR_BASE_RIGHT_CLICK_REASON_ROCKET_SITE",
    "AIR_BASE_RIGHT_CLICK_REASON_ROCKET_SITE_ROCKET",
    "AIR_BASE_RIGHT_CLICK_REASON_SAME_BASE",
    "AIR_BASE_RIGHT_CLICK_REASON_SAME_BASE_ROCKET",
    "AIR_BASE_RIGHT_CLICK_REASON_NO_ACCESS",
    "",
    "AIR_BASE_RIGHT_CLICK_REASON_ASSIGNED_TO_RAID",
];

pub(super) fn append(context: &Context<'_>, base: u64, selected: &[u64], out: &mut Tooltip) {
    let result = (|| {
        let registrations = context
            .frame
            .complete::<tables::AirBaseRegistrations>()
            .map_err(|error| error.to_string())?;
        if !registrations
            .iter()
            .any(|row| row.base == base && row.slot == context.vision.ours())
        {
            return Ok(None);
        }
        let bases = context
            .frame
            .complete::<tables::AirBases>()
            .map_err(|error| error.to_string())?;
        let target = bases
            .iter()
            .find(|row| row.id == base)
            .ok_or("transfer target base unavailable")?;
        if selected.is_empty() {
            return Ok(Some(String::new()));
        }
        let wings = context
            .frame
            .complete::<tables::AirWings>()
            .map_err(|error| error.to_string())?;
        let mut counts = [0_i32; 9];
        for handle in selected {
            let Some(wing) = wings.iter().find(|wing| wing.id == *handle) else {
                continue;
            };
            let reason = if wing.air_base == base {
                if special(context, wing)? {
                    5
                } else if raid_reference(wing)?.resolves {
                    8
                } else {
                    4
                }
            } else {
                destination(context, wing, target, bases)?
            };
            counts[reason] = counts[reason].wrapping_add(1);
        }
        let mut all_special = true;
        if counts[0] == 0 {
            for handle in selected {
                if let Some(wing) = wings.iter().find(|wing| wing.id == *handle)
                    && !special(context, wing)?
                {
                    all_special = false;
                    break;
                }
            }
        }
        feedback(context.loc, selected.len(), all_special, counts).map(Some)
    })();
    match result {
        Ok(Some(text)) => match viewer_foundation::tooltip_text::displayable(context.icons, text) {
            Ok(text) => out.body = text,
            Err(why) => {
                out.body.clear();
                refuse(out, "selected Air-base transfer", why);
            }
        },
        Ok(None) => (),
        Err(why) => {
            out.body.clear();
            refuse(out, "selected Air-base transfer", why);
        }
    }
}

fn definition<'a>(context: &'a Context<'_>, wing: &AirWingRow) -> Result<(&'a str, u64), String> {
    let index = wing
        .pool_subunit
        .require("AirWings.pool_subunit")?
        .as_ref()
        .ok_or("selected wing pool definition unavailable")?;
    context
        .units
        .as_ref()
        .map_err(Clone::clone)?
        .subunit(*index)
}

fn special(context: &Context<'_>, wing: &AirWingRow) -> Result<bool, String> {
    Ok(definition(context, wing)?.1 & replay_data::rows::AIR_MISSION_ZERO_PLANE_FLAGS != 0)
}

fn raid_reference(wing: &AirWingRow) -> Result<replay_data::rows::AirWingRaidReference, String> {
    if wing.assigned_to_raid() == Some(false) {
        return Ok(replay_data::rows::AirWingRaidReference {
            raw: 0,
            resolves: false,
        });
    }
    wing.raid_reference
        .require("AirWings.raid_reference")?
        .as_ref()
        .copied()
        .ok_or_else(|| "selected wing stored raid identity unavailable".into())
}

fn destination(
    context: &Context<'_>,
    wing: &AirWingRow,
    target: &AirBaseRow,
    bases: &[AirBaseRow],
) -> Result<usize, String> {
    let raid = raid_reference(wing)?;
    if raid.resolves {
        let raids = context
            .frame
            .complete::<tables::Raids>()
            .map_err(|error| error.to_string())?;
        let raid = raids
            .iter()
            .find(|row| (u64::from(row.id) << 32 | u64::from(row.reference_type)) == raid.raw)
            .ok_or("selected wing resolved raid is absent from Raids")?;
        if source_province(context, &raid.source)? != base_province(context, target)? {
            return Ok(7);
        }
    }
    let owner = if wing.owner == 0 {
        0
    } else {
        context.slot(wing.owner)?
    };
    let registrations = context
        .frame
        .complete::<tables::AirBaseRegistrations>()
        .map_err(|error| error.to_string())?;
    let Some(registration) = registrations
        .iter()
        .find(|row| row.base == target.id && row.slot == owner)
    else {
        return Ok(6);
    };
    let current = bases
        .iter()
        .find(|base| base.id == wing.air_base)
        .ok_or("selected wing pool base is absent from AirBases")?;
    if target.kind == 1 || current.kind == 1 {
        return Ok(if special(context, wing)? { 3 } else { 2 });
    }
    if target.kind == 2 || current.kind == 2 {
        return Ok(7);
    }
    if target.state.is_none() {
        let (name, _) = definition(context, wing)?;
        let content = context.content.ok_or("selected wing content unavailable")?;
        let capable = if name.is_empty() {
            false
        } else {
            content
                .sub_units
                .as_ref()
                .map_err(Clone::clone)?
                .air_carrier_capable(name.as_bytes(), &content.equipment)?
        };
        if !capable {
            return Ok(1);
        }
    }
    if registration.capacity_raw <= 0 {
        return Ok(6);
    }
    if let Some(state) = target.state {
        let flags = context
            .frame
            .complete::<tables::StateFlags>()
            .map_err(|error| error.to_string())?;
        let flags = flags
            .iter()
            .find(|row| row.state == state)
            .ok_or("transfer target state flags unavailable")?;
        if flags.demilitarized != 0 {
            return Ok(6);
        }
    }
    Ok(0)
}

fn base_province(context: &Context<'_>, base: &AirBaseRow) -> Result<u32, String> {
    if base.state.is_none() {
        return ship_province(context, base.host);
    }
    if let Some(province) = base.province {
        return Ok(province);
    }
    if let Some(content) = context.content {
        let kind = match base.kind {
            1 => 1,
            2 => 2,
            _ => 0,
        };
        return content
            .air_base_first_provinces
            .get(&(base.state.unwrap(), kind))
            .copied()
            .transpose()
            .map(|province| province.unwrap_or(0))
            .map_err(str::to_owned);
    }
    context
        .frame
        .complete::<tables::AirReferenceWitnesses>()
        .map_err(|error| error.to_string())?
        .iter()
        .find(|row| row.handle == base.id)
        .and_then(|row| row.province)
        .ok_or_else(|| "transfer target's uncached state/kind province order unavailable".into())
}

fn ship_province(context: &Context<'_>, ship: u64) -> Result<u32, String> {
    if ship == 0 {
        return Ok(0);
    }
    let ships = context
        .frame
        .complete::<tables::Ships>()
        .map_err(|error| error.to_string())?;
    let Some(ship) = ships.iter().find(|row| row.id == ship) else {
        return Ok(0);
    };
    context
        .frame
        .complete::<tables::TaskForces>()
        .map_err(|error| error.to_string())?
        .iter()
        .find(|row| row.id == ship.task_force)
        .map(|row| row.province)
        .ok_or_else(|| "raid source ship has no task force".into())
}

fn source_province(context: &Context<'_>, source: &RaidSource) -> Result<u32, String> {
    if source.kind_present == 0 {
        return Ok(0);
    }
    Ok(match source.kind.ok_or("raid source kind unavailable")? {
        0..=2 => source.province,
        3 | 4 => {
            return ship_province(
                context,
                u64::from(source.ship_reference_id) << 32 | u64::from(source.ship_reference_type),
            );
        }
        5 => {
            let province = source.building_location as u32;
            context
                .frame
                .complete::<tables::Provinces>()
                .map_err(|error| error.to_string())?
                .iter()
                .find(|row| row.id == province)
                .map_or(0, |row| row.id)
        }
        _ => 0,
    })
}

fn feedback(
    loc: &super::Localisation,
    selected: usize,
    all_special: bool,
    counts: [i32; 9],
) -> Result<String, String> {
    if selected == 0 {
        return Ok(String::new());
    }
    let accepted = counts[0];
    let key = if accepted == 0 {
        if all_special {
            "AIR_BASE_RIGHT_CLICK_NONE_ROCKETS"
        } else {
            "AIR_BASE_RIGHT_CLICK_NONE"
        }
    } else {
        "AIR_BASE_RIGHT_CLICK"
    };
    let mut text = localise(loc, key, &[])? + "\n";
    if accepted > 0 && (accepted as usize) < selected {
        text.push_str(&(localise(loc, "AIR_BASE_RIGHT_CLICK_NOT_ALL", &[])? + "\n"));
    }
    if accepted == 0 || (accepted as usize) < selected {
        for (index, key) in REASONS.into_iter().enumerate() {
            let count = counts[index + 1];
            if count > 0 && !key.is_empty() {
                text.push_str(&(localise(loc, key, &[("COUNT", Arg::Int32(count))])? + "\n"));
            }
        }
        text.push('\n');
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transfer_feedback_keeps_native_reason_order_unlabelled_rejections_and_empty_assignment() {
        let mut loc = std::collections::BTreeMap::<String, String>::new();
        for key in [
            "AIR_BASE_RIGHT_CLICK",
            "AIR_BASE_RIGHT_CLICK_NONE",
            "AIR_BASE_RIGHT_CLICK_NONE_ROCKETS",
            "AIR_BASE_RIGHT_CLICK_NOT_ALL",
        ] {
            loc.insert(key.into(), key.into());
        }
        for key in REASONS.into_iter().filter(|key| !key.is_empty()) {
            loc.insert(key.into(), format!("{key} $COUNT$"));
        }
        let loc: super::super::Localisation = loc.into_iter().collect();
        assert_eq!(feedback(&loc, 0, true, [0; 9]).unwrap(), "");
        assert_eq!(
            feedback(&loc, 2, false, [2, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap(),
            "AIR_BASE_RIGHT_CLICK\n"
        );
        let mixed = feedback(&loc, 5, false, [1, 1, 0, 0, 2, 0, 0, 1, 0]).unwrap();
        assert_eq!(
            mixed,
            "AIR_BASE_RIGHT_CLICK\nAIR_BASE_RIGHT_CLICK_NOT_ALL\nAIR_BASE_RIGHT_CLICK_REASON_NOT_CARRIER_CAPABLE 1\nAIR_BASE_RIGHT_CLICK_REASON_SAME_BASE 2\n\n"
        );
        assert_eq!(
            feedback(&loc, 1, true, [0, 0, 0, 0, 0, 0, 0, 1, 0]).unwrap(),
            "AIR_BASE_RIGHT_CLICK_NONE_ROCKETS\n\n"
        );
    }
}
