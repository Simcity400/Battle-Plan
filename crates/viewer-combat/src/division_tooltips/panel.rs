use super::*;
use replay_data::{rows::DivisionPanelStatsRow, rows::DivisionStatusRow};
mod attrition;

fn cached_value(
    status: &DivisionStatusRow,
    panel: Option<&DivisionPanelStatsRow>,
    index: usize,
    selector: usize,
) -> Option<i64> {
    if selector == 0x30 {
        status
            .movement_stats
            .require("DivisionStatus.movement_stats")
            .ok()?
            .as_ref()
            .map(|stats| stats.air_defense)
    } else {
        panel?.panel_stats[index]
    }
}

fn cached_text(
    icons: &dyn TextIcons,
    loc: &Localisation,
    status: &DivisionStatusRow,
    panel: Option<&DivisionPanelStatsRow>,
) -> (String, Vec<String>) {
    let metadata = content_reader_script::equipment_stats::stat_metadata();
    let mut text = String::new();
    let mut refused = Vec::new();
    let stored = DivisionPanelStatsRow::PANEL_STAT_SELECTORS
        .iter()
        .copied()
        .enumerate();
    for (index, selector) in stored
        .clone()
        .take(11)
        .chain(std::iter::once((usize::MAX, 0x30)))
        .chain(stored.skip(11))
    {
        if selector == 0x3e {
            continue;
        }
        let Some((_, label, _)) = metadata.get(selector) else {
            refused.push(format!(
                "deployed division stat selector {selector} has no executable metadata"
            ));
            continue;
        };
        let line = cached_value(status, panel, index, selector)
            .ok_or_else(|| format!("deployed division {label} cache unavailable"))
            .and_then(|value| {
                Ok(format!(
                    "{}: {}",
                    builtin(icons, loc, label, &[])?,
                    builtin(
                        icons,
                        loc,
                        &format!("{label}_VALUE"),
                        &[("VALUE", Argument::Fixed5(value))]
                    )?
                ))
            });
        match line {
            Ok(line) => {
                text.push_str(&line);
                text.push('\n');
            }
            Err(why) => refused.push(why),
        }
    }
    (text, refused)
}
pub(super) fn text(
    content: &StaticContent,
    frame: &DecodedFrame,
    id: u64,
) -> Result<(String, Vec<String>), String> {
    let rows = frame
        .complete::<tables::DivisionStatus>()
        .map_err(|why| why.to_string())?;
    let status = rows
        .binary_search_by_key(&id, |row| row.division)
        .ok()
        .map(|at| &rows[at])
        .ok_or("deployed division panel status unavailable")?;
    let (icons, loc) = (content.assets.as_ref(), &content.localisation);
    let panel = frame
        .complete::<tables::DivisionPanelStats>()
        .ok()
        .and_then(|rows| {
            rows.binary_search_by_key(&id, |row| row.division)
                .ok()
                .map(|at| &rows[at])
        });
    let (mut text, mut refused) = cached_text(icons, loc, status, panel);
    let units = frame
        .complete::<tables::Divisions>()
        .map_err(|why| why.to_string())?;
    let unit = units
        .binary_search_by_key(&id, |row| row.id)
        .ok()
        .map(|at| &units[at])
        .ok_or("deployed division unavailable")?;
    let terms = replay_derive::eta_derive::frame_terms::FrameTerms::new(frame);
    match terms.panel_width(unit).and_then(|value| {
        Ok(format!(
            "{}: {}",
            builtin(icons, loc, "COMBAT_WIDTH", &[])?,
            content_reader_parser::fixed5::fixed5_to_string(value, 2)
        ))
    }) {
        Ok(line) => {
            text.push_str(&line);
            text.push('\n');
        }
        Err(why) => refused.push(why),
    }
    let fuel = status
        .movement_stats
        .require("DivisionStatus.movement_stats")
        .ok()
        .and_then(|stats| stats.as_ref())
        .map(|stats| stats.fuel_rate);
    if let Some(fuel) = fuel {
        let label = content_reader_script::equipment_stats::stat_metadata()
            .get(0x10)
            .ok_or("fuel stat metadata unavailable")?
            .1;
        text.push_str(&builtin(icons, loc, label, &[])?);
        text.push_str(": ");
        text.push_str(&builtin(
            icons,
            loc,
            &format!("{label}_VALUE"),
            &[("VALUE", Argument::Fixed5(fuel))],
        )?);
    } else {
        refused.push("deployed division fuel consumption cache unavailable".into());
    }
    let speed = (|| {
        let movement_content = content
            .movement
            .as_ref()
            .ok_or("deployed speed content unavailable")?;
        let units = frame
            .complete::<tables::Divisions>()
            .map_err(|why| why.to_string())?;
        let unit = units
            .binary_search_by_key(&id, |row| row.id)
            .ok()
            .map(|at| &units[at])
            .ok_or("deployed division unavailable")?;
        let movement = frame
            .complete::<tables::DivisionMovement>()
            .map_err(|why| why.to_string())?;
        let movement = movement
            .binary_search_by_key(&id, |row| row.division)
            .ok()
            .map(|at| &movement[at])
            .ok_or("deployed movement unavailable")?;
        replay_derive::eta_derive::frame_terms::FrameTerms::new(frame).panel_speed(
            unit,
            movement,
            movement_content.as_ref(),
            &movement_content.defines,
            combat_speed(content, frame, movement),
        )
    })();
    match speed.and_then(|value| stat_line(icons, loc, 0x3e, value)) {
        Ok(line) => {
            text.push('\n');
            text.push_str(&line);
        }
        Err(why) => refused.push(why),
    }
    match attrition::fresh(content, frame, unit, status).and_then(|value| {
        builtin(
            icons,
            loc,
            "ATTRITION_TOTAL",
            &[("TOT", Argument::Fixed5(value))],
        )
    }) {
        Ok(line) => {
            text.push('\n');
            text.push_str(&line);
        }
        Err(why) => refused.push(format!("fresh deployed attrition: {why}")),
    }
    match sources(content, frame, status) {
        Ok((lines, reasons)) => {
            refused.extend(reasons);
            text.push('\n');
            text.push_str(&lines);
        }
        Err(why) => refused.push(format!("deployed stat modifier sources: {why}")),
    }
    Ok((text, refused))
}

fn stat_line(
    icons: &dyn TextIcons,
    loc: &Localisation,
    selector: usize,
    value: i64,
) -> Result<String, String> {
    let label = content_reader_script::equipment_stats::stat_metadata()
        .get(selector)
        .ok_or("stat metadata unavailable")?
        .1;
    Ok(format!(
        "{}: {}",
        builtin(icons, loc, label, &[])?,
        builtin(
            icons,
            loc,
            &format!("{label}_VALUE"),
            &[("VALUE", Argument::Fixed5(value))]
        )?
    ))
}

fn combat_speed(
    content: &StaticContent,
    frame: &DecodedFrame,
    movement: &replay_data::rows::DivisionMovementRow,
) -> Result<i64, String> {
    if movement.combat_count == 0
        || movement
            .withdrawal
            .require("DivisionMovement.withdrawal")?
            .is_some_and(|value| value != 0)
    {
        return Ok(100_000);
    }
    let refs = movement
        .combat_references
        .require("DivisionMovement.combat_references")?
        .as_ref()
        .ok_or("deployed speed combat references unavailable")?;
    let combats = frame
        .complete::<tables::LandCombats>()
        .map_err(|why| why.to_string())?;
    let rows = frame
        .complete::<tables::LandCombatDetails>()
        .map_err(|why| why.to_string())?;
    let sides = frame
        .complete::<tables::LandCombatSides>()
        .map_err(|why| why.to_string())?;
    for reference in &refs.entries {
        let members = combats
            .binary_search_by_key(&reference.combat, |row| row.id)
            .ok()
            .map(|at| &combats[at])
            .ok_or("deployed speed referenced combat membership unavailable")?;
        if !members.attackers.contains(&movement.division)
            && !members.defenders.contains(&movement.division)
        {
            continue;
        }
        let detail = rows
            .binary_search_by_key(&reference.combat, |row| row.combat)
            .ok()
            .map(|at| &rows[at])
            .ok_or("deployed speed referenced combat unavailable")?;
        let membership = sides
            .binary_search_by_key(&reference.combat, |row| row.combat)
            .ok()
            .map(|at| &sides[at]);
        let tactics = &content.assets.tactics;
        organisation_effects(tactics, detail, membership)?;
        let mut value = if reference.kind == 0 {
            content
                .assets
                .land_defines
                .as_ref()
                .map_err(Clone::clone)?
                .combat_speed
        } else {
            0
        };
        for (index, side) in detail.sides.iter().enumerate() {
            if side.tactic_kind != 2
                || super::super::tooltips::countered(side, &detail.sides[1 - index])
            {
                continue;
            }
            value = value.wrapping_add(
                tactics
                    .effects(side.tactic_id, &side.tactic_token)?
                    .movement,
            );
        }
        return Ok(value);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::Recorded;
    #[test]
    fn every_deployed_cache_column_joins_the_panel_table_and_preserves_refusal() {
        let status = DivisionStatusRow::default();
        let mut panel = DivisionPanelStatsRow::default();
        for (index, selector) in DivisionPanelStatsRow::PANEL_STAT_SELECTORS
            .iter()
            .copied()
            .enumerate()
        {
            panel.panel_stats[index] = Some(i64::MIN + index as i64);
            assert_eq!(
                cached_value(&status, Some(&panel), index, selector),
                panel.panel_stats[index]
            );
            assert_eq!(cached_value(&status, None, index, selector), None);
            panel.panel_stats[index] = None;
            assert_eq!(cached_value(&status, Some(&panel), index, selector), None);
        }
    }

    #[test]
    fn the_panel_leaves_out_the_stored_suppression_word() {
        assert!(
            !DivisionPanelStatsRow::PANEL_STAT_SELECTORS
                .contains(&DivisionPanelStatsRow::SUPPRESSION_SELECTOR)
        );
    }

    #[test]
    fn anti_air_reuses_the_existing_raw_movement_operand_and_preserves_refusal() {
        let mut status = DivisionStatusRow {
            movement_stats: Recorded::Value(Some(replay_data::rows::DivisionMovementStats {
                air_defense: i64::MIN,
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(!DivisionPanelStatsRow::PANEL_STAT_SELECTORS.contains(&0x30));
        assert_eq!(
            cached_value(&status, None, usize::MAX, 0x30),
            Some(i64::MIN)
        );
        status.movement_stats = Recorded::Value(None);
        assert_eq!(cached_value(&status, None, usize::MAX, 0x30), None);
        status.movement_stats = Recorded::NotRecorded;
        assert_eq!(cached_value(&status, None, usize::MAX, 0x30), None);
    }
}

fn sources(
    content: &StaticContent,
    frame: &DecodedFrame,
    status: &DivisionStatusRow,
) -> Result<(String, Vec<String>), String> {
    use crate::modifier_text::{Block, Definitions};
    use replay_data::rows::ModifierBreakdownNode;
    use replay_derive::modifier_registry::Registry;
    let definitions = Definitions::read(frame, &content.modifier_names)
        .map_err(|why| why.to_string())?
        .map_err(|why| why.to_string())?;
    let registry = Registry::from_frame(frame)?;
    let metadata = frame
        .complete::<tables::ModifierDefinitions>()
        .map_err(|why| why.to_string())?;
    let rows = frame
        .complete::<tables::DivisionPanelSources>()
        .map_err(|why| why.to_string())?;
    let selected = status
        .panel_source()
        .ok_or("selected commander source identity unavailable")?;
    let mut text = String::new();
    let mut refused = Vec::new();
    if selected[1] == u64::MAX {
        refused.push("deployed selected commander identity unavailable".into());
    }
    for (kind, owner) in std::iter::once((0, selected[0]))
        .chain((selected[2] != 0).then_some((selected[1] as u8, selected[2])))
    {
        let result = (|| {
            let start = rows.partition_point(|row| (row.kind, row.owner) < (kind, owner));
            let end = rows.partition_point(|row| (row.kind, row.owner) <= (kind, owner));
            let nodes = &rows[start..end];
            if nodes.is_empty() {
                return Err(format!("shared modifier source {kind}/{owner} is absent"));
            }
            let mut remaining = 1u64;
            for (ordinal, row) in nodes.iter().enumerate() {
                let node = row
                    .node
                    .as_ref()
                    .ok_or_else(|| String::from_utf8_lossy(&row.reason).into_owned())?;
                if row.ordinal as usize != ordinal || remaining == 0 {
                    return Err("shared modifier source preorder is incomplete".into());
                }
                remaining = remaining - 1 + u64::from(node.children);
            }
            if remaining != 0 {
                return Err("shared modifier source preorder is incomplete".into());
            }
            let root = nodes[0].node.as_ref().unwrap();
            let mut output = String::new();
            let keys = source_keys(nodes);
            for key in keys {
                let part = (|| {
                    let raw = root
                        .values
                        .binary_search_by_key(&key, |value| value.key)
                        .ok()
                        .map_or(0, |at| root.values[at].raw);
                    let definition = metadata
                        .binary_search_by_key(&key, |row| row.index)
                        .ok()
                        .map(|at| &metadata[at])
                        .ok_or_else(|| {
                            format!("division source modifier {} has no definition", key)
                        })?;
                    if definition.category_mask & 2 == 0 && !panel_context_key(key) {
                        return Ok(String::new());
                    }
                    let block = Block {
                        values: BTreeMap::from([(key, raw)]),
                        ..Default::default()
                    };
                    let heading = block
                        .describe(
                            &definitions,
                            content.assets.as_ref(),
                            &content.localisation,
                            100_000,
                            "",
                            u32::MAX,
                        )
                        .map_err(|why| why.to_string())?;
                    let mut tree = Vec::with_capacity(nodes.len());
                    for row in nodes {
                        let node = row.node.as_ref().unwrap();
                        let raw = node
                            .values
                            .binary_search_by_key(&key, |v| v.key)
                            .ok()
                            .map_or(0, |at| node.values[at].raw);
                        tree.push(ModifierBreakdownNode {
                            children: node.children,
                            value: registry.effective(registry.indexed(key)?, raw)?,
                            name: node.name.clone(),
                        });
                    }
                    let body = crate::naval_trees::breakdown(
                        &tree,
                        definition,
                        100_000,
                        "  ",
                        &content.recorded_labels,
                        content.assets.as_ref(),
                        &content.localisation,
                        true,
                    )?;
                    let mut line = String::new();
                    if !heading.is_empty() {
                        line.push_str(&heading);
                        line.push('\n');
                    }
                    line.push_str(&body);
                    line.push('\n');
                    Ok::<_, String>(line)
                })();
                match part {
                    Ok(line) => output.push_str(&line),
                    Err(why) => refused.push(format!(
                        "deployed modifier source {kind}/{owner}/{key}: {why}"
                    )),
                }
            }
            Ok::<_, String>(output)
        })();
        match result {
            Ok(lines) => text.push_str(&lines),
            Err(why) => refused.push(format!("deployed modifier source {kind}/{owner}: {why}")),
        }
    }
    Ok((text, refused))
}
fn panel_context_key(key: u32) -> bool {
    use engine_executable::modifier_keys::*;
    [
        ATTRITION,
        UNIT_UPKEEP_ATTRITION_FACTOR,
        HEAT_ATTRITION_FACTOR,
        WINTER_ATTRITION_FACTOR,
        ARMY_SPEED_FACTOR,
        ARMY_ARMOR_SPEED_FACTOR,
        ARMY_ATTACK_SPEED_FACTOR,
        OUT_OF_SUPPLY_FACTOR,
        SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
        COMBAT_WIDTH_FACTOR,
    ]
    .contains(&key)
}

fn source_keys(
    nodes: &[replay_data::rows::SharedDivisionPanelSourceRow],
) -> std::collections::BTreeSet<u32> {
    nodes
        .iter()
        .filter_map(|row| row.node.as_ref())
        .flat_map(|node| node.values.iter().map(|value| value.key))
        .collect()
}
#[cfg(test)]
mod source_tests {
    use super::*;
    use replay_data::{
        rows::DivisionPanelSourceNode, rows::DivisionPanelSourceRow, rows::DivisionPanelSourceValue,
    };
    #[test]
    fn cancelling_leaves_remain_explainable_when_the_aggregate_has_no_key() {
        let row = |ordinal, values| {
            std::sync::Arc::new(DivisionPanelSourceRow {
                ordinal,
                node: Some(DivisionPanelSourceNode {
                    values,
                    ..Default::default()
                }),
                ..Default::default()
            })
        };
        let nodes = [
            row(0, Vec::new()),
            row(1, vec![DivisionPanelSourceValue { key: 79, raw: 4000 }]),
            row(
                2,
                vec![DivisionPanelSourceValue {
                    key: 79,
                    raw: -4000,
                }],
            ),
        ];
        assert_eq!(source_keys(&nodes), std::collections::BTreeSet::from([79]));
    }
}
