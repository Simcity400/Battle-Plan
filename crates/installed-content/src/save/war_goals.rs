//! CWarGoal::Serialize@1415CE720 -> goal data@1415CE460.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarGoal {
    pub actor: Vec<u8>,
    pub recipient: Vec<u8>,
    pub kind: Vec<u8>,
    pub states: Vec<u32>,
    /// Goal+0x60, saved as liberate (0x30D0), in native order.
    pub countries: Vec<Vec<u8>>,
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
            kind: required_bytes(body, b"type")?,
            states,
            countries,
        };
        if out.insert(handle, goal).is_some() {
            return Err(invalid("duplicate full war goal handle in save"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordered_null_repeated_references_join_to_mod_goal_objects() {
        let doc=crate::parser::parse(br#"first_wargoals={wargoal={id=3 type=4713} wargoal={id=0 type=0} wargoal={id=3 type=4713}}
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
        let bad = crate::parser::parse(b"first_wargoals={not_a_goal={id=3 type=4713}}").unwrap();
        assert!(references(&bad.items, b"first_wargoals").is_err());
    }
}
