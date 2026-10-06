//! Country-owned goal references and their serialized payloads.

use super::{block, bytes, field, integer_list, invalid, null_permitted_handle, object_id};
use crate::parser::Item;
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    /// Every saved country, including an omitted (empty) owned-goal roster.
    pub owned_goals: BTreeMap<Vec<u8>, Vec<u64>>,
    /// Diplomacy +0x80 owns serialized goal objects independently of +0x68.
    pub goals: BTreeMap<u64, Goal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Goal {
    pub recipient: Vec<u8>,
    pub goal_type: Vec<u8>,
    pub states: Vec<u32>,
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    let mut pack = Pack::default();
    let countries = field(items, b"countries")?
        .ok_or_else(|| invalid("country rosters save lacks countries"))?;
    for country in block(countries)? {
        let tag = country
            .key
            .as_ref()
            .filter(|tag| !tag.is_empty())
            .ok_or_else(|| invalid("country roster has no tag"))?;
        let fields = block(&country.value)?;
        let mut owned = Vec::new();
        if let Some(diplomacy) = field(fields, b"diplomacy")? {
            let diplomacy = block(diplomacy)?;
            // 140D32BD0 emits +0x68/count+0x74 only when nonempty, in native
            // order, as available_wargoals (0x337C), wargoal (0x2CE2) refs.
            if let Some(roster) = field(diplomacy, b"available_wargoals")? {
                for entry in block(roster)? {
                    if entry.key.as_deref() != Some(b"wargoal") {
                        return Err(invalid("owned wargoal roster has an unknown entry"));
                    }
                    owned.push(null_permitted_handle(&entry.value)?);
                }
            }
            // 140D32BD0 emits +0x80/count+0x8C under wargoals (0x331D),
            // with each object's +0x30 token as its dynamic child key.
            if let Some(goals) = field(diplomacy, b"wargoals")? {
                for item in block(goals)? {
                    item.key
                        .as_ref()
                        .filter(|key| !key.is_empty())
                        .ok_or_else(|| invalid("unkeyed or empty serialized wargoals child"))?;
                    let payload = block(&item.value)?;
                    // 1415CE720 -> 1415CE460 writes actor/recipient/type
                    // always; states (0x2E3B) only for a nonzero count.
                    // type is [+0x40]+8, not the child key (+0x30):
                    // 1415CBDD0 initializes both from the definition, but
                    // 1415CD530 can replace +0x40 without changing +0x30.
                    super::scalar(
                        field(payload, b"wargoaldata_actor")?
                            .ok_or_else(|| invalid("wargoal lacks actor"))?,
                    )?;
                    let goal = Goal {
                        recipient: bytes(payload, b"wargoaldata_recipient")?
                            .ok_or_else(|| invalid("wargoal lacks recipient"))?,
                        goal_type: bytes(payload, b"type")?
                            .ok_or_else(|| invalid("wargoal lacks type"))?,
                        states: integer_list(payload, b"states")?,
                    };
                    if pack.goals.insert(object_id(payload)?, goal).is_some() {
                        return Err(invalid("duplicate serialized wargoal handle"));
                    }
                }
            }
        }
        if pack.owned_goals.insert(tag.clone(), owned).is_some() {
            return Err(invalid("duplicate country roster tag"));
        }
    }
    Ok(pack)
}

#[cfg(test)]
mod tests;
