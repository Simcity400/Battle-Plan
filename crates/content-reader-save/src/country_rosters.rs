
use super::{block, bytes, field, integer_list, invalid, null_permitted_handle, object_id};
use content_reader_parser::parser::Item;
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub owned_goals: BTreeMap<Vec<u8>, Vec<u64>>,
    pub goals: BTreeMap<u64, Goal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Goal {
    pub recipient: Vec<u8>,
    pub goal_type: Vec<u8>,
    pub states: Vec<u32>,
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    parse_with_countries(items, super::CountriesView::from_root(items)?)
}

pub(super) fn parse_with_countries(
    _items: &[Item],
    countries: super::CountriesView<'_>,
) -> io::Result<Pack> {
    let mut pack = Pack::default();
    if !countries.is_present() {
        return Err(invalid("country rosters save lacks countries"));
    }
    for country in countries.iter() {
        let tag = country
            .key
            .as_ref()
            .filter(|tag| !tag.is_empty())
            .ok_or_else(|| invalid("country roster has no tag"))?;
        let fields = block(&country.value)?;
        let mut owned = Vec::new();
        if let Some(diplomacy) = field(fields, b"diplomacy")? {
            let diplomacy = block(diplomacy)?;
            if let Some(roster) = field(diplomacy, b"available_wargoals")? {
                for entry in block(roster)? {
                    if entry.key.as_deref() != Some(b"wargoal") {
                        return Err(invalid("owned wargoal roster has an unknown entry"));
                    }
                    owned.push(null_permitted_handle(&entry.value)?);
                }
            }
            if let Some(goals) = field(diplomacy, b"wargoals")? {
                for item in block(goals)? {
                    item.key
                        .as_ref()
                        .filter(|key| !key.is_empty())
                        .ok_or_else(|| invalid("unkeyed or empty serialized wargoals child"))?;
                    let payload = block(&item.value)?;
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
