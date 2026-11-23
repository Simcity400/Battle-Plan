use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarGoal {
    pub actor: Vec<u8>,
    pub recipient: Vec<u8>,
    pub kind: Vec<u8>,
    pub states: Vec<u32>,
    pub countries: Vec<Vec<u8>>,
    pub puppets: Vec<Vec<u8>>,
    pub expires: Option<SavedDate>,
    pub name: Vec<u8>,
}
pub(super) fn references(items: &[Item], key: &[u8]) -> io::Result<Vec<u64>> {
    let Some(value) = field(items, key)? else {
        return Ok(Vec::new());
    };
    block(value)?
        .iter()
        .map(|item| {
            if item.key.as_deref() != Some(b"wargoal") {
                return Err(invalid(
                    "war goal reference list contains a non-wargoal entry",
                ));
            }
            null_permitted_handle(&item.value)
        })
        .collect()
}
pub(super) fn objects(country: &[Item], out: &mut BTreeMap<u64, WarGoal>) -> io::Result<()> {
    let Some(diplomacy) = field(country, b"diplomacy")? else {
        return Ok(());
    };
    let Some(goals) = field(block(diplomacy)?, b"wargoals")? else {
        return Ok(());
    };
    for item in block(goals)? {
        let body = block(&item.value)?;
        let handle =
            typed_handle(field(body, b"id")?.ok_or_else(|| invalid("war goal lacks id"))?)?;
        let states = field(body, b"states")?
            .map(block)
            .transpose()?
            .unwrap_or_default()
            .iter()
            .map(|v| {
                if v.key.is_some() {
                    return Err(invalid("war goal states list contains a keyed value"));
                }
                integer::<u32>(
                    v.value
                        .scalar()
                        .ok_or_else(|| invalid("war goal state is not a number"))?,
                )
            })
            .collect::<io::Result<Vec<_>>>()?;
        let countries = field(body, b"liberate")?
            .map(block)
            .transpose()?
            .unwrap_or_default()
            .iter()
            .map(|v| {
                if v.key.is_some() {
                    return Err(invalid("war goal country list contains a keyed value"));
                }
                v.value
                    .scalar()
                    .map(Vec::from)
                    .ok_or_else(|| invalid("war goal country is not a tag"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        let goal = WarGoal {
            actor: required_bytes(body, b"wargoaldata_actor")?,
            recipient: required_bytes(body, b"wargoaldata_recipient")?,
            name: item
                .key
                .clone()
                .ok_or_else(|| invalid("war-goal object has no serialized name"))?,
            kind: required_bytes(body, b"type")?,
            states,
            countries,
            puppets: tag_list(body, b"puppets")?,
            expires: bytes(body, b"expire")?
                .as_deref()
                .map(SavedDate::parse)
                .transpose()?,
        };
        if out.insert(handle, goal).is_some() {
            return Err(invalid("duplicate full war goal handle in save"));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryInventory {
    pub goals: Vec<u64>,
    pub available: Vec<u64>,
    pub justifications: Vec<Justification>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Justification {
    pub ordinal: u32,
    pub actor: Vec<u8>,
    pub recipient: Vec<u8>,
    pub kind: Vec<u8>,
    pub total_cost: Option<Decimal>,
    pub daily_cost: Option<Decimal>,
    pub points: Option<Decimal>,
    pub start: Option<SavedDate>,
    pub end: Option<SavedDate>,
    pub pay_upfront: bool,
    pub threat: Option<Decimal>,
    pub states: Vec<u32>,
}
fn tag_list(body: &[Item], key: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    field(body, key)?
        .map(block)
        .transpose()?
        .unwrap_or_default()
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("war-goal country list contains a keyed entry"));
            }
            Ok(scalar(&item.value)?.to_vec())
        })
        .collect()
}
fn state_list(body: &[Item], key: &[u8]) -> io::Result<Vec<u32>> {
    field(body, key)?
        .map(block)
        .transpose()?
        .unwrap_or_default()
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("war-goal state list contains a keyed entry"));
            }
            Ok(integer::<i32>(scalar(&item.value)?)? as u32)
        })
        .collect()
}
pub(super) fn inventory(country: &[Item]) -> io::Result<CountryInventory> {
    let mut out = CountryInventory::default();
    if let Some(diplomacy) = field(country, b"diplomacy")? {
        let body = block(diplomacy)?;
        if let Some(goals) = field(body, b"wargoals")? {
            for goal in block(goals)? {
                out.goals.push(typed_handle(
                    field(block(&goal.value)?, b"id")?
                        .ok_or_else(|| invalid("war-goal object lacks id"))?,
                )?);
            }
        }
        out.available = references(body, b"available_wargoals")?;
    }
    if let Some(politics) = field(country, b"politics")?
        && let Some(activities) = field(block(politics)?, b"timed_activities")?
    {
        for (ordinal, activity) in block(activities)?.iter().enumerate() {
            if activity.key.as_deref() != Some(b"timed_wargoal") {
                continue;
            }
            let body = block(&activity.value)?;
            out.justifications.push(Justification {
                ordinal: u32::try_from(ordinal)
                    .map_err(|_| invalid("too many saved timed activities"))?,
                actor: required_bytes(body, b"owner")?,
                recipient: required_bytes(body, b"target")?,
                kind: required_bytes(body, b"wargoal")?,
                total_cost: decimal(body, b"total_cost")?,
                daily_cost: decimal(body, b"daily_cost")?,
                points: decimal(body, b"points")?,
                start: bytes(body, b"start_date")?
                    .as_deref()
                    .map(SavedDate::parse)
                    .transpose()?,
                end: bytes(body, b"end_date")?
                    .as_deref()
                    .map(SavedDate::parse)
                    .transpose()?,
                pay_upfront: field(body, b"pay_upfront")?
                    .map(boolean)
                    .transpose()?
                    .unwrap_or(false),
                threat: decimal(body, b"threat")?,
                states: state_list(body, b"states")?,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordered_null_repeated_references_join_to_mod_goal_objects() {
        let doc=content_reader_parser::parser::parse(br#"first_wargoals={wargoal={id=3 type=4713} wargoal={id=0 type=0} wargoal={id=3 type=4713}}
            diplomacy={wargoals={mod_claim={id={id=3 type=4713} wargoaldata_actor="ONE" wargoaldata_recipient="TWO" type=star_contest states={9 7 9} liberate={THREE ONE}}}}"#).unwrap();
        let handles = references(&doc.items, b"first_wargoals").unwrap();
        assert_eq!(handles, vec![(3_u64 << 32) | 4713, 0, (3_u64 << 32) | 4713]);
        let mut objects_by_id = BTreeMap::new();
        objects(&doc.items, &mut objects_by_id).unwrap();
        let goal = &objects_by_id[&handles[0]];
        assert_eq!(goal.kind, b"star_contest");
        assert_eq!(goal.states, [9, 7, 9]);
        assert_eq!(goal.countries, vec![b"THREE".to_vec(), b"ONE".to_vec()]);
        assert!(objects(&doc.items, &mut objects_by_id).is_err());
        let bad =
            content_reader_parser::parser::parse(b"first_wargoals={not_a_goal={id=3 type=4713}}").unwrap();
        assert!(references(&bad.items, b"first_wargoals").is_err());
    }
}

#[cfg(test)]
mod inventory_tests {
    use super::*;
    #[test]
    fn wargoal_inventory_keeps_used_and_unused_rosters_and_activity_ordinals() {
        let doc=content_reader_parser::parser::parse(br#"diplomacy={wargoals={custom={id={id=4 type=123} type=custom wargoaldata_actor=ONE wargoaldata_recipient=TWO states={7 8 7} puppets={THREE} expire=1940.1.1.1}} available_wargoals={wargoal={id=4 type=123}}}
        politics={timed_activities={other_activity={} timed_wargoal={owner=ONE target=TWO total_cost=90 daily_cost=1 points=17.5 pay_upfront=yes wargoal=custom states={8 7} threat=0.25}}}"#).unwrap();
        let saved = inventory(&doc.items).unwrap();
        assert_eq!(saved.goals, [4u64 << 32 | 123]);
        assert_eq!(saved.available, saved.goals);
        let j = &saved.justifications[0];
        assert_eq!(j.ordinal, 1);
        assert_eq!(j.points.as_ref().unwrap().raw, 1750000);
        assert!(j.pay_upfront);
        assert_eq!(j.states, [8, 7]);
        let mut objects_by_id = BTreeMap::new();
        objects(&doc.items, &mut objects_by_id).unwrap();
        let g = &objects_by_id[&saved.goals[0]];
        assert_eq!(g.states, [7, 8, 7]);
        assert_eq!(g.puppets, [b"THREE".to_vec()]);
        assert!(g.expires.is_some());
    }
}
