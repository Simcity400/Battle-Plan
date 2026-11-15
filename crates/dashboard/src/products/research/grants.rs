//! A country's research grant inventory, from its owners' `ResearchModifiers` rows of the
//! inventory scope at a read; names are localised from the bound content.
use replay_data::rows::ResearchModifierRow as R;
use serde::Serialize;
use ts_rs::TS;

/// The `ResearchModifiers` index of a country's grant inventory rows.
pub(super) const INVENTORY_INDEX: u32 = u32::MAX - 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GrantRestriction {
    pub token: String,
    pub name: Option<String>,
    pub database_index: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchGrant {
    pub id: i32,
    pub ordinal: u32,
    pub kind: String,
    pub token: String,
    pub name: Option<String>,
    pub uses: i32,
    pub active_claims: Option<i32>,
    pub speed_bonus_raw: Option<String>,
    pub ahead_reduction_raw: Option<String>,
    pub cost_reduction_raw: Option<String>,
    pub technologies: Vec<GrantRestriction>,
    pub categories: Vec<GrantRestriction>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GrantInventory {
    pub next_id: Option<i32>,
    pub grants: Vec<ResearchGrant>,
    /// Why the inventory is not known at the hour, when it is not.
    pub unavailable: Option<String>,
}

impl GrantInventory {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self { next_id: None, grants: Vec::new(), unavailable: Some(reason.into()) }
    }
}

/// Whether `row` names a restriction, whose token [`assemble`] spells.
pub(super) fn spells(row: &R) -> bool {
    matches!(row.kind, R::GRANT_TECHNOLOGY | R::GRANT_CATEGORY)
}

/// The inventory one country's grant rows hold, in their table order, every name unlocalised;
/// `token` spells a restriction's token.
pub(super) fn assemble(selected: &[&R], mut token: impl FnMut(u32) -> Result<String, String>) -> Result<GrantInventory, String> {
    let mut counters = selected.iter().filter(|row| row.kind == R::GRANT_COUNTER);
    let counter = counters.next().ok_or("Research grant inventory is not recorded")?;
    if counters.next().is_some() {
        return Err("Research grant inventory has multiple country counters".into());
    }
    let next_id = i32::try_from(counter.raw).map_err(|_| "Research grant counter is outside its signed width")?;
    let mut grants = Vec::new();
    let mut parents = selected.iter().enumerate().filter(|(_, row)| matches!(row.kind, R::SPEED_GRANT | R::COST_GRANT)).peekable();
    while let Some((position, row)) = parents.next() {
        let end = parents.peek().map_or(selected.len(), |(next, _)| *next);
        let children = &selected[position + 1..end];
        let cost = row.kind == R::COST_GRANT;
        let own = |value: &&&R, kind: u8| value.kind == kind && value.source_id == row.source_id && value.count == u32::from(cost);
        let mut usage = children.iter().filter(|value| own(value, R::GRANT_USES));
        let usage_row = usage.next().ok_or("Research grant uses are not recorded")?;
        if usage.next().is_some() {
            return Err("Research grant has multiple use records".into());
        }
        let mut restrictions = |kind: u8| -> Result<Vec<GrantRestriction>, String> {
            children
                .iter()
                .filter(|value| own(value, kind))
                .map(|value| {
                    let spelling = token(u32::try_from(value.raw).map_err(|_| "Research grant token is outside its native width")?)?;
                    let database_index = if kind == R::GRANT_TECHNOLOGY {
                        Some(i32::try_from(value.auxiliary).map_err(|_| "Research grant technology index is outside its native width")?)
                    } else {
                        None
                    };
                    Ok(GrantRestriction { name: None, token: spelling, database_index })
                })
                .collect()
        };
        let technologies = restrictions(R::GRANT_TECHNOLOGY)?;
        let categories = restrictions(R::GRANT_CATEGORY)?;
        let spelling = String::from_utf8(row.name.clone()).map_err(|_| "Research grant name has invalid text")?;
        grants.push(ResearchGrant {
            id: row.source_id as i32,
            ordinal: row.count,
            kind: if cost { "cost" } else { "speed" }.into(),
            name: None,
            token: spelling,
            uses: i32::try_from(usage_row.raw).map_err(|_| "Research grant uses are outside their signed width")?,
            active_claims: if cost { None } else { Some(i32::try_from(usage_row.auxiliary).map_err(|_| "Research grant claims are outside their signed width")?) },
            speed_bonus_raw: (!cost).then(|| row.raw.to_string()),
            ahead_reduction_raw: (!cost).then(|| row.auxiliary.to_string()),
            cost_reduction_raw: cost.then(|| row.raw.to_string()),
            technologies,
            categories,
        });
    }
    Ok(GrantInventory { next_id: Some(next_id), grants, unavailable: None })
}

/// `inventory` with each grant's and restriction's name localised.
pub(super) fn localised(mut inventory: GrantInventory, localise: &dyn Fn(&str) -> Option<String>) -> GrantInventory {
    for grant in &mut inventory.grants {
        for restriction in grant.technologies.iter_mut().chain(grant.categories.iter_mut()) {
            restriction.name = localise(&restriction.token);
        }
        grant.name = localise(&grant.token);
    }
    inventory
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: u8, source_id: u32, count: u32, raw: i64, auxiliary: i64, name: &[u8]) -> R {
        R { country: 11, index: INVENTORY_INDEX, kind, source_id, count, raw, auxiliary, name: name.to_vec(), ..Default::default() }
    }

    #[test]
    fn grant_inventory_keeps_repeated_ids_in_row_order_and_spells_restrictions() {
        let rows = [
            row(R::GRANT_COUNTER, 0, 0, -7, 0, b""),
            row(R::SPEED_GRANT, 7, 0, 10, 20_000, b"mod_a"),
            row(R::GRANT_USES, 7, 0, 3, 2, b""),
            row(R::GRANT_TECHNOLOGY, 7, 0, 41, 9, b""),
            row(R::SPEED_GRANT, 7, 1, 20, 0, b"mod_b"),
            row(R::GRANT_USES, 7, 0, 1, 0, b""),
            row(R::COST_GRANT, 7, 1, 30_000, 0, b"mod_c"),
            row(R::GRANT_USES, 7, 1, 2, 0, b""),
            row(R::GRANT_CATEGORY, 7, 1, 42, 0, b""),
        ];
        let selected: Vec<&R> = rows.iter().collect();
        let inventory = assemble(&selected, |id| Ok(format!("token_{id}"))).unwrap();
        assert_eq!(inventory.next_id, Some(-7));
        assert_eq!(inventory.grants.iter().map(|grant| (grant.token.as_str(), grant.uses)).collect::<Vec<_>>(), [("mod_a", 3), ("mod_b", 1), ("mod_c", 2)]);
        assert_eq!(inventory.grants[0].active_claims, Some(2));
        assert_eq!(inventory.grants[0].technologies, [GrantRestriction { token: "token_41".into(), name: None, database_index: Some(9) }]);
        assert_eq!(inventory.grants[2].active_claims, None);
        assert_eq!(inventory.grants[2].cost_reduction_raw.as_deref(), Some("30000"));
        assert_eq!(inventory.grants[2].categories[0].token, "token_42");
        let named = localised(inventory, &|token| Some(format!("<{token}>")));
        assert_eq!(named.grants[1].name.as_deref(), Some("<mod_b>"));
        assert_eq!(named.grants[2].categories[0].name.as_deref(), Some("<token_42>"));
        // A token the recording does not spell refuses the whole inventory by name, never a part.
        assert_eq!(assemble(&selected, |id| Err(format!("Research grant restriction token {id} is not recorded"))).unwrap_err(), "Research grant restriction token 41 is not recorded");
        assert!(assemble(&selected[1..], |id| Ok(id.to_string())).is_err());
    }
}
