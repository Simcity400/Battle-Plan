//! Inventory projection for the requested country and recorded hour. Dynamic
//! grant facts come from rows; their spelling and presentation use bound content.
use super::*;
use replay_data::rows::ResearchModifierRow as R;

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GrantRestriction {
    pub token: String,
    pub name: Option<String>,
    pub database_index: Option<i32>,
}

#[derive(Clone, Debug, Serialize, TS)]
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

#[derive(Clone, Debug, Serialize, TS)]
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

pub(super) const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::ResearchModifiers,
    TableId::SliceReads,
    TableId::LexicalTokens,
];

pub(super) fn inventory(
    frame: &DecodedFrame,
    tag: &str,
    mut localise: impl FnMut(&str) -> Option<String>,
) -> GrantInventory {
    fn read(
        frame: &DecodedFrame,
        tag: &str,
        localise: &mut impl FnMut(&str) -> Option<String>,
    ) -> Result<GrantInventory, String> {
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
        let token = |id: u32| -> Result<String, String> {
            let row = tokens
                .iter()
                .find(|row| row.token == id)
                .ok_or_else(|| format!("Research grant restriction token {id} is not recorded"))?;
            String::from_utf8(row.literal.clone())
                .map_err(|_| format!("Research grant restriction token {id} has invalid text"))
        };
        let selected: Vec<_> = rows
            .iter()
            .filter(|row| {
                row.index == engine_research::research_modifiers::grants::INVENTORY_INDEX
                    && crate::metadata::link_slot(links, row.country) == Some(slot)
            })
            .collect();
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
                children.iter().filter(|value| value.kind == kind && value.source_id == row.source_id
                    && value.count == u32::from(cost)).map(|value| {
                    let spelling = token(u32::try_from(value.raw).map_err(|_| "Research grant token is outside its native width")?)?;
                    Ok(GrantRestriction { name: localise(&spelling), token: spelling,
                        database_index: if kind == R::GRANT_TECHNOLOGY { Some(i32::try_from(value.auxiliary)
                            .map_err(|_| "Research grant technology index is outside its native width")?) } else { None } })
                }).collect()
            };
            let technologies = restrictions(R::GRANT_TECHNOLOGY)?;
            let categories = restrictions(R::GRANT_CATEGORY)?;
            let spelling = String::from_utf8(row.name.clone())
                .map_err(|_| "Research grant name has invalid text")?;
            grants.push(ResearchGrant {
                id: row.source_id as i32,
                ordinal: row.count,
                kind: if cost { "cost" } else { "speed" }.into(),
                name: localise(&spelling),
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
    read(frame, tag, &mut localise).unwrap_or_else(GrantInventory::unavailable)
}

/// A bounded owner-read proof, including empty-slot countries and every intervening hour.
pub(super) fn at_hour(
    source: &Source,
    branch: u64,
    hour: i32,
    tag: &str,
    cancelled: &AtomicBool,
    localise: impl FnMut(&str) -> Option<String>,
) -> io::Result<GrantInventory> {
    let frames = crate::prepared::query::progressive::read_window(
        source,
        branch,
        hour.saturating_sub(replay_data::cadence::STAGGER_PERIOD - 1),
        hour,
        TABLES,
        cancelled,
    )?;
    let mut identity = None;
    let mut previous = None;
    let mut admitted = false;
    for frame in &frames {
        let current = frame
            .complete::<tables::Countries>()
            .ok()
            .zip(frame.complete::<tables::CountryLinks>().ok())
            .and_then(|(countries, links)| {
                crate::metadata::tagged_country(countries, links, tag).found()
            })
            .map(|(country, slot)| (country.id, slot));
        if current != identity
            || previous.and_then(|last: i32| last.checked_add(1)) != Some(frame.hour)
        {
            admitted = false;
        }
        identity = current;
        previous = Some(frame.hour);
        let read = current.and_then(|(id, _)| {
            frame
                .complete::<tables::SliceReads>()
                .ok()
                .and_then(|reads| {
                    replay_data::sliced::read_hour(reads, TableId::ResearchModifiers, id)
                })
        });
        if read == Some(frame.hour) {
            admitted = true;
        }
        let owner_reads_match = current.is_some_and(|(_, slot)| {
            let Some((links, reads, rows)) = frame
                .complete::<tables::CountryLinks>()
                .ok()
                .zip(frame.complete::<tables::SliceReads>().ok())
                .zip(frame.complete::<tables::ResearchModifiers>().ok())
                .map(|((links, reads), rows)| (links, reads, rows))
            else {
                return false;
            };
            rows.iter()
                .filter(|row| {
                    row.index == engine_research::research_modifiers::grants::INVENTORY_INDEX
                        && crate::metadata::link_slot(links, row.country) == Some(slot)
                })
                .all(|row| {
                    replay_data::sliced::read_hour(reads, TableId::ResearchModifiers, row.country)
                        == read
                })
        });
        if !owner_reads_match {
            admitted = false;
        }
        if read.is_none_or(|read| {
            read > frame.hour
                || frame.hour.saturating_sub(read) >= replay_data::cadence::STAGGER_PERIOD
        }) || frame.complete::<tables::ResearchModifiers>().is_err()
        {
            admitted = false;
        }
    }
    Ok(
        match frames.last().filter(|frame| frame.hour == hour && admitted) {
            Some(frame) => inventory(frame, tag, localise),
            None => GrantInventory::unavailable(
                "Research grants lack a continuous supplying read at this hour",
            ),
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{CAMPAIGN, file_source, records, source};
    use replay::container::Replay;
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    #[test]
    fn gap_research_grants_grant_inventory_preserves_repeated_ids_and_requires_its_supplying_read()
    {
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
        *frame.tables[TableId::SliceReads.index()] =
            TableView::Complete(TableRows::SliceReads(vec![
                SliceReadRow {
                    table: TableId::ResearchModifiers.index() as u32,
                    owner: 9001,
                    sample_hour: 24,
                },
                SliceReadRow {
                    table: TableId::ResearchModifiers.index() as u32,
                    owner: 11,
                    sample_hour: 24,
                },
            ]));
        if let TableView::Complete(TableRows::SliceReads(reads)) =
            &mut *frame.tables[TableId::SliceReads.index()]
        {
            reads.sort_by_key(|row| (row.table, row.owner));
        }
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(Vec::new()));
        let (source, root) =
            file_source(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap());
        let result = at_hour(&source, 1, 24, "GER", &AtomicBool::new(false), |_| None).unwrap();
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
        assert!(
            at_hour(&source, 1, 25, "GER", &AtomicBool::new(false), |_| None)
                .unwrap()
                .unavailable
                .is_some()
        );
        drop(source);
        std::fs::remove_dir_all(root).unwrap();
    }
}
