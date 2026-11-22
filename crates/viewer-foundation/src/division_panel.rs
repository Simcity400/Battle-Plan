use replay_data::{rows::ModifierBreakdownNode, rows::tables};
use replay_derive::modifier_registry::Registry;
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
pub fn operand(
    frame: &DecodedFrame,
    kind: u8,
    owner: u64,
    ordinal: u32,
    key: u32,
) -> Result<i64, String> {
    let rows = frame
        .complete::<tables::DivisionPanelSources>()
        .map_err(|why| why.to_string())?;
    let row = rows
        .binary_search_by_key(&(kind, owner, ordinal), |row| {
            (row.kind, row.owner, row.ordinal)
        })
        .ok()
        .map(|at| &rows[at])
        .ok_or_else(|| format!("attrition operand {kind}/{owner}/{ordinal}/{key} absent"))?;
    let node = row
        .node
        .as_ref()
        .ok_or_else(|| String::from_utf8_lossy(&row.reason).into_owned())?;
    Ok(node
        .values
        .binary_search_by_key(&key, |value| value.key)
        .ok()
        .map_or(0, |at| node.values[at].raw))
}
pub fn modifier(
    frame: &DecodedFrame,
    registry: &Registry<'_>,
    kind: u8,
    owner: u64,
    key: u32,
) -> Result<i64, String> {
    registry.effective(registry.indexed(key)?, operand(frame, kind, owner, 0, key)?)
}
pub fn state(frame: &DecodedFrame, state: u32, country: u32, key: u32) -> Result<i64, String> {
    operand(frame, 4, ((state as u64) << 32) | country as u64, 0, key)
}
pub fn country_identity(frame: &DecodedFrame, raw: u32) -> Result<u64, String> {
    if raw == 0 {
        return Ok(0);
    }
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?;
    let slot = links
        .binary_search_by_key(&raw, |row| row.id)
        .ok()
        .map(|at| links[at].canonical_slot)
        .ok_or("attrition owner country link unavailable")?;
    if slot == 0 {
        return Ok(0);
    }
    frame
        .complete::<tables::Countries>()
        .map_err(|why| why.to_string())?
        .iter()
        .find(|row| row.canonical_slot == slot)
        .map(|row| row.id as u64)
        .ok_or_else(|| "attrition owner country identity unavailable".into())
}
pub fn equipment(base: i64, values: impl IntoIterator<Item = (i64, i64)>) -> i64 {
    let sum = values
        .into_iter()
        .filter(|(quantity, _)| *quantity != 0)
        .fold(0i64, |sum, (_, bonus)| sum.wrapping_add(bonus));
    base.wrapping_add(sum.max(base.wrapping_neg()))
}
pub fn at_war(frame: &DecodedFrame, links: &BTreeMap<u32, u32>, raw: u32) -> Result<bool, String> {
    let slot = links
        .get(&raw)
        .copied()
        .ok_or("attrition logical owner unresolved")?;
    let countries = frame
        .complete::<tables::CountryWars>()
        .map_err(|why| why.to_string())?;
    if countries.iter().any(|row| {
        links.get(&row.country) == Some(&slot) && row.enemies_read != 0 && !row.enemies.is_empty()
    }) {
        return Ok(true);
    }
    Ok(frame
        .complete::<tables::WarOwnedRelations>()
        .map_err(|why| why.to_string())?
        .iter()
        .any(|row| links.get(&row.owner) == Some(&slot)))
}

pub fn held_bonus(frame: &DecodedFrame, division: u64, terrain: u32) -> Result<i64, String> {
    let sources = frame
        .complete::<tables::DivisionPanelSources>()
        .map_err(|why| why.to_string())?;
    let start = sources.partition_point(|row| (row.kind, row.owner) < (5, division));
    let end = sources.partition_point(|row| (row.kind, row.owner) <= (5, division));
    let rows = &sources[start..end];
    let summary = rows
        .first()
        .filter(|row| row.ordinal == 0)
        .ok_or("held variant membership unavailable")?;
    summary
        .node
        .as_ref()
        .ok_or_else(|| String::from_utf8_lossy(&summary.reason).into_owned())?;
    rows.iter().skip(1).try_fold(0i64, |sum, row| {
        let node = row.node.as_ref().ok_or("held variant membership refused")?;
        let handle = node
            .values
            .first()
            .ok_or("held variant identity unavailable")?
            .raw as u64;
        Ok(sum.wrapping_add(operand(frame, 3, handle, terrain, 0)?))
    })
}
pub fn state_amount(
    frame: &DecodedFrame,
    state_id: u32,
    owner: u32,
    controller: u32,
) -> Result<i64, String> {
    use engine_executable::modifier_keys as keys;
    let links: BTreeMap<_, _> = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let mut value = state(frame, state_id, owner, keys::ATTRITION)?;
    let friendly = replay_derive::visibility::event_shared(frame, &links, owner, controller, true)
        .map_err(str::to_owned)?;
    if friendly {
        value = value.wrapping_add(state(
            frame,
            state_id,
            owner,
            keys::ATTRITION_FOR_CONTROLLER,
        )?);
    } else if (owner as i32) > 0 {
        let slot = |raw| {
            if raw == 0 {
                Ok(0)
            } else {
                links
                    .get(&raw)
                    .copied()
                    .ok_or_else(|| format!("attrition country {raw} unresolved"))
            }
        };
        let wars = crate::visibility::Relations::read(frame, &links).map_err(str::to_owned)?;
        if wars.at_war(slot(owner)?, slot(controller)?) {
            value = value.wrapping_add(state(frame, state_id, owner, keys::ENEMY_ATTRITION)?);
        }
    }
    Ok(value)
}
pub fn tree(
    frame: &DecodedFrame,
    kind: u8,
    owner: u64,
    key: u32,
) -> Result<Vec<ModifierBreakdownNode>, String> {
    let rows = frame
        .complete::<tables::DivisionPanelSources>()
        .map_err(|why| why.to_string())?;
    let start = rows.partition_point(|row| (row.kind, row.owner) < (kind, owner));
    let end = rows.partition_point(|row| (row.kind, row.owner) <= (kind, owner));
    if start == end {
        return Err(format!("shared modifier source {kind}/{owner} unavailable"));
    }
    let registry = Registry::from_frame(frame)?;
    rows[start..end]
        .iter()
        .enumerate()
        .map(|(ordinal, row)| {
            if row.ordinal as usize != ordinal {
                return Err("shared modifier source preorder incomplete".into());
            }
            let node = row
                .node
                .as_ref()
                .ok_or_else(|| String::from_utf8_lossy(&row.reason).into_owned())?;
            let raw = node
                .values
                .binary_search_by_key(&key, |v| v.key)
                .ok()
                .map_or(0, |at| node.values[at].raw);
            Ok(ModifierBreakdownNode {
                children: node.children,
                value: registry.effective(registry.indexed(key)?, raw)?,
                name: node.name.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::DivisionPanelSourceNode, rows::DivisionPanelSourceRow,
        rows::DivisionPanelSourceValue, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::TableView;
    use std::sync::Arc;
    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Default::default(),
            tables: TableId::ALL.map(|id| TableView::Complete(TableRows::empty(id)).into()),
        }
    }
    fn row(kind: u8, owner: u64, ordinal: u32, raw: i64) -> Arc<DivisionPanelSourceRow> {
        Arc::new(DivisionPanelSourceRow {
            kind,
            owner,
            ordinal,
            node: Some(DivisionPanelSourceNode {
                values: vec![DivisionPanelSourceValue { key: 0, raw }],
                ..Default::default()
            }),
            ..Default::default()
        })
    }
    #[test]
    fn held_membership_repeats_are_not_quantity_weighted_and_a_missing_variant_is_named() {
        let mut frame = frame();
        *frame.tables[TableId::DivisionPanelSources.index()] =
            TableView::Complete(TableRows::DivisionPanelSources(vec![
                row(3, 70, 2, 4_000),
                row(5, 9, 0, 0),
                row(5, 9, 2, 70),
                row(5, 9, 7, 70),
            ]));
        assert_eq!(held_bonus(&frame, 9, 2), Ok(8_000));
        assert!(held_bonus(&frame, 9, 3).unwrap_err().contains("3/70/3/0"));
        assert_eq!(equipment(3_000, [(1, -10_000)]), 0);
        assert_eq!(equipment(3_000, [(0, 9_000), (-5, 2_000)]), 5_000);
    }
    #[test]
    fn mode_zero_war_gate_keeps_retired_owned_relations_and_native_enemy_count() {
        use replay_data::{rows::CountryWarRow, rows::WarOwnedRelationRow};
        let mut frame = frame();
        let links = BTreeMap::from([(7, 3)]);
        assert_eq!(at_war(&frame, &links, 7), Ok(false));
        *frame.tables[TableId::WarOwnedRelations.index()] =
            TableView::Complete(TableRows::WarOwnedRelations(vec![WarOwnedRelationRow {
                owner: 7,
                retired: 1,
                ..Default::default()
            }]));
        assert_eq!(at_war(&frame, &links, 7), Ok(true));
        *frame.tables[TableId::WarOwnedRelations.index()] =
            TableView::Complete(TableRows::WarOwnedRelations(Vec::new()));
        *frame.tables[TableId::CountryWars.index()] =
            TableView::Complete(TableRows::CountryWars(vec![CountryWarRow {
                country: 7,
                enemies_read: 1,
                enemies: vec![0],
                ..Default::default()
            }]));
        assert_eq!(at_war(&frame, &links, 7), Ok(true));
    }
}
