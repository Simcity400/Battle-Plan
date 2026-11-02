//! A country's grant inventory: its dynamic facts come from its owners' ResearchModifiers rows
//! at a read its supplying read admits ([`super::emit`]); their names are localised from bound
//! content when a reply is made.
use super::*;
use replay_data::rows::ResearchModifierRow as R;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GrantRestriction {
    pub token: String,
    pub name: Option<String>,
    pub database_index: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GrantInventory {
    pub next_id: Option<i32>,
    pub grants: Vec<ResearchGrant>,
    pub unavailable: Option<String>,
}

impl GrantInventory {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            next_id: None,
            grants: Vec::new(),
            unavailable: Some(reason.into()),
        }
    }
}

/// Why a reply shows no inventory at an hour no supplying read admits.
pub(super) const UNADMITTED: &str = "Research grants lack a continuous supplying read at this hour";

/// Whether `row` belongs to a country's grant inventory.
pub(super) fn inventory_row(row: &R) -> bool {
    row.index == engine_research::research_modifiers::grants::INVENTORY_INDEX
}

/// Whether `row` names a restriction, whose token `assemble` spells.
pub(super) fn spells(row: &R) -> bool {
    matches!(row.kind, R::GRANT_TECHNOLOGY | R::GRANT_CATEGORY)
}

/// The inventory one slot's grant rows hold, in their table order, every name unlocalised;
/// `token` spells a restriction's token.
pub(super) fn assemble(
    selected: &[&R],
    mut token: impl FnMut(u32) -> Result<String, String>,
) -> Result<GrantInventory, String> {
    let mut counters = selected.iter().filter(|row| row.kind == R::GRANT_COUNTER);
    let counter = counters
        .next()
        .ok_or("Research grant inventory is not recorded")?;
    if counters.next().is_some() {
        return Err("Research grant inventory has multiple country counters".into());
    }
    let next_id = i32::try_from(counter.raw)
        .map_err(|_| "Research grant counter is outside its signed width")?;
    let mut grants = Vec::new();
    let mut parents = selected
        .iter()
        .enumerate()
        .filter(|(_, row)| matches!(row.kind, R::SPEED_GRANT | R::COST_GRANT))
        .peekable();
    while let Some((position, row)) = parents.next() {
        let end = parents.peek().map_or(selected.len(), |(next, _)| *next);
        let children = &selected[position + 1..end];
        let cost = row.kind == R::COST_GRANT;
        let mut usage = children.iter().filter(|value| {
            value.kind == R::GRANT_USES
                && value.source_id == row.source_id
                && value.count == u32::from(cost)
        });
        let usage_row = usage.next().ok_or("Research grant uses are not recorded")?;
        if usage.next().is_some() {
            return Err("Research grant has multiple use records".into());
        }
        let mut restrictions = |kind| -> Result<Vec<GrantRestriction>, String> {
            children
                .iter()
                .filter(|value| {
                    value.kind == kind
                        && value.source_id == row.source_id
                        && value.count == u32::from(cost)
                })
                .map(|value| {
                    let spelling = token(
                        u32::try_from(value.raw)
                            .map_err(|_| "Research grant token is outside its native width")?,
                    )?;
                    Ok(GrantRestriction {
                        name: None,
                        token: spelling,
                        database_index: if kind == R::GRANT_TECHNOLOGY {
                            Some(i32::try_from(value.auxiliary).map_err(|_| {
                                "Research grant technology index is outside its native width"
                            })?)
                        } else {
                            None
                        },
                    })
                })
                .collect()
        };
        let technologies = restrictions(R::GRANT_TECHNOLOGY)?;
        let categories = restrictions(R::GRANT_CATEGORY)?;
        let spelling =
            String::from_utf8(row.name.clone()).map_err(|_| "Research grant name has invalid text")?;
        grants.push(ResearchGrant {
            id: row.source_id as i32,
            ordinal: row.count,
            kind: if cost { "cost" } else { "speed" }.into(),
            name: None,
            token: spelling,
            uses: i32::try_from(usage_row.raw)
                .map_err(|_| "Research grant uses are outside their signed width")?,
            active_claims: if cost {
                None
            } else {
                Some(
                    i32::try_from(usage_row.auxiliary)
                        .map_err(|_| "Research grant claims are outside their signed width")?,
                )
            },
            speed_bonus_raw: (!cost).then(|| row.raw.to_string()),
            ahead_reduction_raw: (!cost).then(|| row.auxiliary.to_string()),
            cost_reduction_raw: cost.then(|| row.raw.to_string()),
            technologies,
            categories,
        });
    }
    Ok(GrantInventory {
        next_id: Some(next_id),
        grants,
        unavailable: None,
    })
}

/// `inventory` with each restriction's and grant's name localised, in the order the row read
/// names them.
pub(super) fn localised(
    mut inventory: GrantInventory,
    mut localise: impl FnMut(&str) -> Option<String>,
) -> GrantInventory {
    for grant in &mut inventory.grants {
        for restriction in grant
            .technologies
            .iter_mut()
            .chain(grant.categories.iter_mut())
        {
            restriction.name = localise(&restriction.token);
        }
        grant.name = localise(&grant.token);
    }
    inventory
}

/// The row frame's inventory of `tag`, as a supplying read at its hour shows it.
#[cfg(test)]
pub(super) fn inventory(
    frame: &DecodedFrame,
    tag: &str,
    localise: impl FnMut(&str) -> Option<String>,
) -> GrantInventory {
    fn read(frame: &DecodedFrame, tag: &str) -> Result<GrantInventory, String> {
        let countries = frame
            .complete::<tables::Countries>()
            .map_err(|e| e.to_string())?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?;
        let (_, slot) = crate::metadata::tagged_country(countries, links, tag)
            .found()
            .ok_or("Research grant country identity is unavailable")?;
        let rows = frame
            .complete::<tables::ResearchModifiers>()
            .map_err(|e| e.to_string())?;
        let tokens = frame
            .complete::<tables::LexicalTokens>()
            .map_err(|e| e.to_string())?;
        let selected: Vec<_> = rows
            .iter()
            .filter(|row| {
                inventory_row(row) && crate::metadata::link_slot(links, row.country) == Some(slot)
            })
            .collect();
        assemble(&selected, |id| {
            let row = tokens
                .iter()
                .find(|row| row.token == id)
                .ok_or_else(|| format!("Research grant restriction token {id} is not recorded"))?;
            String::from_utf8(row.literal.clone())
                .map_err(|_| format!("Research grant restriction token {id} has invalid text"))
        })
    }
    match read(frame, tag) {
        Ok(inventory) => localised(inventory, localise),
        Err(reason) => GrantInventory::unavailable(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::source;
    use replay_data::rows::*;
    use replay_frame::frame::TableView;

    #[test]
    fn grant_inventory_preserves_repeated_ids_in_row_order() {
        let mut frame = source(1, 24, None);
        let values = vec![
            R {
                kind: R::GRANT_COUNTER,
                raw: -7,
                ..Default::default()
            },
            R {
                kind: R::SPEED_GRANT,
                source_id: 7,
                name: b"mod_a".to_vec(),
                raw: 10,
                ..Default::default()
            },
            R {
                kind: R::GRANT_USES,
                source_id: 7,
                raw: 3,
                auxiliary: 2,
                ..Default::default()
            },
            R {
                kind: R::SPEED_GRANT,
                source_id: 7,
                count: 1,
                name: b"mod_b".to_vec(),
                raw: 20,
                ..Default::default()
            },
            R {
                kind: R::GRANT_USES,
                source_id: 7,
                raw: 1,
                ..Default::default()
            },
        ]
        .into_iter()
        .enumerate()
        .map(|(ordinal, mut row)| {
            row.country = 11;
            row.index = engine_research::research_modifiers::grants::INVENTORY_INDEX;
            row.ordinal = ordinal as u32;
            row
        })
        .collect();
        *frame.tables[TableId::ResearchModifiers.index()] =
            TableView::Complete(TableRows::ResearchModifiers(values));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(Vec::new()));
        let result = inventory(&frame, "GER", |token| Some(format!("<{token}>")));
        assert_eq!(result.next_id, Some(-7));
        assert!(result.unavailable.is_none(), "{:?}", result.unavailable);
        assert_eq!(
            result
                .grants
                .iter()
                .map(|grant| grant.uses)
                .collect::<Vec<_>>(),
            [3, 1]
        );
        assert_eq!(result.grants[1].token, "mod_b");
        assert_eq!(result.grants[1].name.as_deref(), Some("<mod_b>"));
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "synthetic".into(),
        };
        assert_eq!(
            inventory(&frame, "GER", |_| None).unavailable.as_deref(),
            Some("hour 24 LexicalTokens is unavailable: synthetic")
        );
    }
}
