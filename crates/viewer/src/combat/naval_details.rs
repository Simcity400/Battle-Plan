//! Native naval member grouping and retained ship-result categories (format 22).
use super::{
    Selection, Values, Widget,
    binding::{Bindings, NavalEntry, fill, hide, sprite, text},
};
use crate::{
    document::Document,
    map::{perspective::Vision, tables::rows},
};
use replay::{
    frame::DecodedFrame,
    rows::{NavalCombatShipRow, NavalResultShip, tables},
};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    io,
};

pub(super) fn bind(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    selection: Selection,
    flags: &BTreeMap<u32, String>,
    vision: &Vision,
) -> io::Result<()> {
    match selection {
        Selection::Naval(id) => active(out, document, frame, id, flags, vision),
        Selection::NavalResult(id) => result(out, document, frame, id, flags),
        Selection::Land(_) => Ok(()),
    }
}

// 14195ED00: state is an enum, not a mask. Live formation flags use the
// 140C2BAD0/BAE0/BAF0 accessors; cached formation_type alone is insufficient.
fn column(row: &NavalCombatShipRow) -> Option<&'static str> {
    match row.state {
        32 => Some("sunk"),
        4 | 16 => Some("reserve"),
        1 | 2 | 8 if row.live_convoy != 0 => Some("carrier"),
        1 | 2 | 8 if row.live_ship != 0 => {
            let flags = row.formation_flags;
            if flags & (1 << 7) != 0 {
                Some(if row.detection_state == 0 {
                    "hidden"
                } else {
                    "visible"
                })
            } else if flags & (1 << 8) != 0 {
                Some("screen")
            } else if flags & (1 << 39) != 0 || row.is_convoy != 0 {
                Some("carrier")
            } else {
                Some("battle")
            }
        }
        _ => None,
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupKey {
    side: u8,
    column: &'static str,
    state: u32,
    formation: i32,
    subunit: i32,
    convoy: u8,
    owner: u32,
    damaged: bool,
    unknown: bool,
}
fn key(row: &NavalCombatShipRow, owner: u32, unknown: bool) -> Option<GroupKey> {
    let column = column(row)?;
    // 1417D9730 collapses all unseen incoming entries in their box. Otherwise
    // it compares cached formation/subunit/convoy, country aliases and damage.
    Some(GroupKey {
        side: row.side,
        column,
        state: if unknown {
            0
        } else if row.state == 1 {
            2
        } else {
            row.state
        },
        formation: if unknown { 0 } else { row.formation_type },
        subunit: if unknown { 0 } else { row.subunit_index },
        convoy: if unknown { 0 } else { row.is_convoy },
        owner: if unknown { 0 } else { owner },
        damaged: !unknown
            && (row.live_ship != 0 || row.live_convoy != 0)
            && row.strength < row.maximum_hit_points,
        unknown,
    })
}

fn active(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
    flags: &BTreeMap<u32, String>,
    vision: &Vision,
) -> io::Result<()> {
    let table = rows::<tables::NavalCombatShips>(frame)?;
    if table.rows().is_empty() && !table.is_complete() {
        return Ok(());
    }
    let mut members: Vec<_> = table.rows().iter().filter(|r| r.combat == id).collect();
    // The native forward traversal and retained-member list are separate. Do
    // not drop source1 when its ship has already left NavalCombatParticipants.
    members.sort_by_key(|r| {
        (
            r.member_source,
            r.group_index,
            r.member_index,
            r.battle_slot,
        )
    });
    let aliases: BTreeMap<_, _> = rows::<tables::CountryLinks>(frame)?
        .rows()
        .iter()
        .map(|r| (r.id, r.canonical_slot))
        .collect();
    let mut lookup = BTreeMap::new();
    let mut groups: Vec<(GroupKey, Vec<&NavalCombatShipRow>)> = Vec::new();
    for row in members {
        let unknown = row.state == 4
            && row.live_ship != 0
            && !vision.sees_combat(aliases.get(&row.owner).copied().into_iter());
        let Some(key) = key(
            row,
            aliases.get(&row.owner).copied().unwrap_or(row.owner),
            unknown,
        ) else {
            out.warnings.push(format!(
                "NavalCombatShips slot {}: state {} has no drawable live combat box; omitted.",
                row.battle_slot, row.state
            ));
            continue;
        };
        let next = groups.len();
        let index = *lookup.entry(key.clone()).or_insert(next);
        if index == next {
            groups.push((key, Vec::new()));
        }
        groups[index].1.push(row);
    }
    // 1417D9C60 -> 1417CC070 sorts each combat box after grouping. More
    // numerous groups precede smaller ones; the final native pointer tie is
    // transient, so retain recorded insertion order for otherwise equal keys.
    groups.sort_by_key(|(key, members)| {
        (
            key.side,
            key.column,
            Reverse(key.convoy),
            Reverse(key.formation),
            Reverse(key.subunit),
            Reverse(members.len()),
            key.damaged,
            Reverse(members.iter().map(|r| r.battle_slot).max().unwrap_or(0)),
        )
    });
    out.naval_entries.clear();
    let mut heights = BTreeMap::<(u8, &str), f32>::new();
    for (key, members) in groups {
        let first = members[0];
        let name = if key.unknown {
            "GFX_navalcombat_ship_icon_unknown".into()
        } else {
            format!(
                "GFX_navalcombat_ship_icon_{}",
                String::from_utf8_lossy(&first.sprite)
            )
        };
        let Some(asset) = document.icons.combat.sprites.get(&name) else {
            out.warnings.push(format!(
                "NavalCombatShips slot {}: declared sprite {name} is unavailable; row omitted.",
                first.battle_slot
            ));
            continue;
        };
        let mut values = Values::new();
        for widget in [
            "icon_hidden",
            "escape_progress",
            "escape_icon_left",
            "escape_icon_right",
            "pride",
            "bar_str",
            "bar_org",
            "desc_left",
            "desc_right",
        ] {
            hide(&mut values, widget);
        }
        let escaping: Vec<_> = members.iter().filter(|r| r.state == 8).collect();
        let reversed = !escaping.is_empty() || members.iter().any(|r| r.state == 16);
        values.insert(
            "icon".into(),
            Widget {
                sprite: Some(name),
                frame: Some(u32::from(key.side) ^ u32::from(reversed)),
                ..Widget::default()
            },
        );
        if !key.unknown {
            descriptions(
                &mut values,
                first.owner,
                members.len(),
                key.side,
                asset.size[0],
                flags,
            );
            let live: Vec<_> = members
                .iter()
                .filter(|r| r.live_ship != 0 || r.live_convoy != 0)
                .collect();
            if !live.is_empty() && key.column != "sunk" {
                let ratio = live
                    .iter()
                    .map(|r| fixed_ratio(r.strength, r.maximum_hit_points.max(100)))
                    .sum::<i128>()
                    / live.len() as i128;
                bar(
                    &mut values,
                    "bar_str",
                    ratio as f32 / 100_000.0,
                    asset.size[0],
                    document,
                );
                if let Some(total) = live.iter().try_fold(0_i128, |sum, row| {
                    Some(sum + fixed_ratio(row.organisation, organisation_maximum(row)?))
                }) {
                    let ratio = total / live.len() as i128;
                    bar(
                        &mut values,
                        "bar_org",
                        ratio as f32 / 100_000.0,
                        asset.size[0],
                        document,
                    );
                }
            }
            if key.column == "hidden" {
                show(&mut values, "icon_hidden");
                center_icon(
                    &mut values,
                    "icon_hidden",
                    "GFX_navalcombat_ship_hidden",
                    asset.size[0],
                    document,
                );
            }
            if members.iter().any(|r| r.pride != 0) {
                show(&mut values, "pride");
                center_icon(
                    &mut values,
                    "pride",
                    "GFX_NV_icon_pride_small",
                    asset.size[0],
                    document,
                );
            }
        }
        if !escaping.is_empty() {
            let ratio = escaping
                .iter()
                .map(|r| i128::from(r.escape_progress))
                .sum::<i128>()
                / escaping.len() as i128;
            bar(
                &mut values,
                "escape_progress",
                ratio as f32 / 100_000.0,
                asset.size[0],
                document,
            );
            show(
                &mut values,
                if key.side == 0 {
                    "escape_icon_left"
                } else {
                    "escape_icon_right"
                },
            );
        }
        let wnd = if key.side == 0 { "left" } else { "right" };
        let grid = match key.column {
            "hidden" | "visible" => {
                format!("battle_window/middle_wnd/{wnd}_wnd/grid_{}", key.column)
            }
            "sunk" => format!("bottom_wnd/{wnd}_wnd/grid"),
            "reserve" => format!("battle_window/outside_wnd/{wnd}_wnd/grid"),
            column => format!("battle_window/top_wnd/{wnd}_wnd/grid_{column}"),
        };
        let height = heights.entry((key.side, key.column)).or_default();
        let offset_x = if key.column == "sunk" {
            0.0
        } else {
            100.0 - asset.size[0] + ((asset.size[0] - 1.0) * 0.5).trunc()
        };
        out.naval_entries.push(NavalEntry {
            grid,
            offset: [offset_x, *height],
            size: asset.size,
            values,
        });
        *height += asset.size[1] + 3.0 + if escaping.is_empty() { 0.0 } else { 8.0 };
    }
    out.warnings
        .retain(|w| !w.starts_with("NavalCombatShips: live HP/org bars"));
    out.warnings.push("Format-22 naval rows: native state/formation/reveal boxes, source0 and retained source1 slots, normalized-state/subunit/formation/country/damaged grouping, copied sprites, native sorted group counts, mean strength/organisation and escape progress are bound. Organisation maxima apply recorded modifiers46/47/27e with default-leader and HQ sources; missing terms in complete captured sources mean zero. UNVERIFIED: native carrier classification also queries embedded equipment flags, pride tint, sunk-strip layout, incoming-entry relation edge cases and remaining flag/text geometry need native operand and per-widget attribution beyond reference07.".into());
    Ok(())
}
fn fixed_ratio(value: i64, maximum: i64) -> i128 {
    i128::from(value) * 100_000 / i128::from(maximum)
}
fn organisation_maximum(row: &NavalCombatShipRow) -> Option<i64> {
    if row.live_ship == 0 {
        return (row.live_convoy != 0 && row.base_organisation > 0)
            .then_some(row.base_organisation);
    }
    // 140C29DF0 -> 140C28730 calls 140C29020 with commander=0, which
    // selects the task force's default commander rather than combat leader.
    // The rest of the captured sources, including HQ12..15, still contribute.
    let mut context = row.weapon_context.clone();
    context.sources.retain(|s| !matches!(s.scope, 9 | 10));
    for source in &mut context.sources {
        source.scope = match source.scope {
            31 => 9,
            32 => 10,
            other => other,
        };
    }
    let modifier = |key| super::naval_environment::modifier(&context, key);
    let base = row.base_organisation.wrapping_add(modifier(0x46)?);
    let factor = 100_000_i64
        .wrapping_add(modifier(0x47)?)
        .wrapping_add(modifier(0x27e)?);
    Some((base.wrapping_mul(factor) / 100_000).max(100))
}
fn show(values: &mut Values, key: &str) {
    values.entry(key.into()).or_default().hidden = false;
}
fn bar(values: &mut Values, key: &str, value: f32, width: f32, document: &Document) {
    show(values, key);
    fill(values, key, value.clamp(0.0, 1.0));
    let name = match key {
        "bar_str" => "GFX_navalcombat_bar_str",
        "bar_org" => "GFX_navalcombat_bar_org",
        _ => "GFX_naval_combat_escape_bar",
    };
    center_icon(values, key, name, width, document);
}
fn center_icon(values: &mut Values, key: &str, name: &str, width: f32, document: &Document) {
    if let Some(asset) = document.icons.combat.sprites.get(name) {
        // CENTER_UP already contributes width/2. Native retains authored x.
        values.entry(key.into()).or_default().offset[0] =
            (width / 2.0).trunc() - width / 2.0 - (asset.size[0] / 2.0).trunc();
    }
}
fn descriptions(
    values: &mut Values,
    owner: u32,
    count: usize,
    side: u8,
    width: f32,
    flags: &BTreeMap<u32, String>,
) {
    values.insert(
        if side == 0 { "desc_right" } else { "desc_left" }.into(),
        Widget {
            text: Some(count.to_string()),
            text_width: Some(9.0 * count.to_string().len() as f32),
            offset: if side == 0 {
                [width - 8.0, 0.0]
            } else {
                [-4.0, 0.0]
            },
            ..Widget::default()
        },
    );
    if let Some(flag) = flags.get(&owner) {
        values.insert(
            if side == 0 { "desc_left" } else { "desc_right" }.into(),
            Widget {
                flag: Some(flag.clone()),
                text: Some(String::new()),
                size: Some([9.0, 6.0]),
                offset: if side == 0 {
                    [-4.0, 6.0]
                } else {
                    [width - 8.0, 6.0]
                },
                ..Widget::default()
            },
        );
    }
}

fn repeated(values: &mut Values, key: &str, template: String, entries: Vec<Values>) {
    values.insert(
        key.into(),
        Widget {
            entry_template: Some(template),
            entries,
            ..Widget::default()
        },
    );
}
fn result(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
    flags: &BTreeMap<u32, String>,
) -> io::Result<()> {
    let table = rows::<tables::NavalCombatResultDetails>(frame)?;
    let Some(detail) = table.rows().iter().find(|r| r.result == id) else {
        return Ok(());
    };
    for (side, row) in detail.sides.iter().enumerate() {
        let label = if side == 0 { "attacker" } else { "defender" };
        let root = if side == 0 {
            "attacking_side"
        } else {
            "defending_side"
        };
        let mut survivor_keys = BTreeMap::new();
        let mut survivors: Vec<Vec<&NavalResultShip>> = Vec::new();
        let mut loss_keys = BTreeMap::new();
        let mut losses: Vec<Vec<&NavalResultShip>> = Vec::new();
        for ship in &row.ships {
            // 1417C4C30/141DF27E0/141DEBD30: survivor subunit+formation;
            // losses use copied sprite, with convoys in a separate list.
            if ship.sunk_by.is_empty() {
                let next = survivors.len();
                let index = *survivor_keys
                    .entry((ship.subunit_index, ship.formation_type))
                    .or_insert(next);
                if index == next {
                    survivors.push(Vec::new());
                }
                survivors[index].push(ship);
            } else {
                let next = losses.len();
                let index = *loss_keys
                    .entry((ship.is_convoy, ship.sprite.as_slice()))
                    .or_insert(next);
                if index == next {
                    losses.push(Vec::new());
                }
                losses[index].push(ship);
            }
        }
        let mut entries = Vec::new();
        for group in survivors {
            let mut values = Values::new();
            let name = format!(
                "GFX_unit_{}_icon_medium_black",
                String::from_utf8_lossy(&group[0].sprite)
            );
            if document.icons.combat.sprites.contains_key(&name) {
                sprite(&mut values, "fleet", name);
            } else {
                hide(&mut values, "fleet");
            }
            text(&mut values, "number", group.len().to_string());
            result_flags(&mut values, &group, flags);
            entries.push(values);
        }
        repeated(
            &mut out.values,
            &format!("{root}/survivors/survivors_grid"),
            format!("naval_combat_results_{label}_survivor_entry"),
            entries,
        );
        let mut entries = Vec::new();
        for group in losses {
            let mut values = Values::new();
            let name = format!(
                "GFX_navalcombat_ship_icon_{}",
                String::from_utf8_lossy(&group[0].sprite)
            );
            if document.icons.combat.sprites.contains_key(&name) {
                values.insert(
                    "equipment".into(),
                    Widget {
                        sprite: Some(name),
                        frame: Some(side as u32),
                        ..Widget::default()
                    },
                );
            } else {
                hide(&mut values, "equipment");
            }
            if !group.iter().any(|s| s.pride != 0) {
                hide(&mut values, "pride");
            }
            let mut countries = BTreeMap::new();
            for ship in group {
                *countries.entry(ship.owner).or_insert(0usize) += 1;
            }
            let counts = countries
                .into_iter()
                .map(|(owner, count)| {
                    let mut values = Values::new();
                    text(&mut values, "number", count.to_string());
                    if let Some(flag) = flags.get(&owner) {
                        values.insert(
                            "country_flag".into(),
                            Widget {
                                flag: Some(flag.clone()),
                                ..Widget::default()
                            },
                        );
                    } else {
                        hide(&mut values, "country_flag");
                    }
                    values
                })
                .collect();
            repeated(
                &mut values,
                "lost_ships_grid",
                format!("naval_combat_results_{label}_lost_ship_entry"),
                counts,
            );
            entries.push(values);
        }
        repeated(
            &mut out.values,
            &format!("{root}/lost/lost_line_grid"),
            format!("naval_combat_results_{label}_lost_line_entry"),
            entries,
        );
    }
    out.warnings.retain(|w| {
        !w.starts_with("Combat results:") && !w.starts_with("NavalCombatResultDetails side")
    });
    out.warnings.push("Format-22 retained results: copied sunk_by classifies survivors/losses; native subunit/formation survivor and sprite/convoy loss groups, recorded counts, copied sprites and country flags are bound. UNVERIFIED: country ordering, representative survivor sprite choice, plane equipment-icon joins, outcome perspective, detail hit text and result scrolling require native binder completion and a retained-result capture. No active-combat disappearance is treated as a result.".into());
    Ok(())
}
fn result_flags(values: &mut Values, ships: &[&NavalResultShip], flags: &BTreeMap<u32, String>) {
    for key in [
        "country_flag_left",
        "country_flag_right",
        "country_flag_center",
    ] {
        hide(values, key);
    }
    let mut owners = Vec::new();
    let mut seen = BTreeSet::new();
    for ship in ships {
        if seen.insert(ship.owner) {
            owners.push(ship.owner);
        }
    }
    let slots: &[&str] = if owners.len() == 1 {
        &["country_flag_center"]
    } else {
        &["country_flag_left", "country_flag_right"]
    };
    for (owner, key) in owners.into_iter().zip(slots) {
        if let Some(flag) = flags.get(&owner) {
            values.insert(
                (*key).into(),
                Widget {
                    flag: Some(flag.clone()),
                    size: Some([12.0, 8.0]),
                    ..Widget::default()
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn organisation_uses_default_leader_and_recorded_hq_terms() {
        use replay::rows::{NavalShipModifierSource, NavalShipModifierTerm};
        let mut row = NavalCombatShipRow {
            live_ship: 1,
            base_organisation: 3_000_000,
            ..Default::default()
        };
        row.weapon_context.sources = vec![
            NavalShipModifierSource {
                scope: 9,
                terms: vec![NavalShipModifierTerm {
                    key: 0x47,
                    raw: 50_000,
                    ..Default::default()
                }],
                ..Default::default()
            },
            NavalShipModifierSource {
                scope: 14,
                index: 0,
                terms: vec![NavalShipModifierTerm {
                    key: 0x47,
                    raw: 10_000,
                    ..Default::default()
                }],
                ..Default::default()
            },
            NavalShipModifierSource {
                scope: 31,
                terms: vec![NavalShipModifierTerm {
                    key: 0x46,
                    raw: 200_000,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ];
        assert_eq!(organisation_maximum(&row), Some(3_520_000));
        row.weapon_context.sources.pop();
        assert_eq!(organisation_maximum(&row), Some(3_300_000));
    }
    #[test]
    fn reveal_and_exact_state_select_boxes() {
        let mut row = NavalCombatShipRow {
            state: 1,
            live_ship: 1,
            formation_flags: 1 << 7,
            ..Default::default()
        };
        assert_eq!(column(&row), Some("hidden"));
        row.detection_state = 2;
        assert_eq!(column(&row), Some("visible"));
        row.state = 32;
        row.live_ship = 0;
        assert_eq!(column(&row), Some("sunk"));
        row.state = 33;
        assert_eq!(column(&row), None);
    }
    #[test]
    fn grouping_normalizes_state_but_preserves_damage() {
        let mut row = NavalCombatShipRow {
            state: 1,
            live_ship: 1,
            strength: 100,
            maximum_hit_points: 100,
            ..Default::default()
        };
        let healthy = key(&row, 7, false);
        row.state = 2;
        assert!(healthy == key(&row, 7, false));
        row.strength = 99;
        assert!(healthy != key(&row, 7, false));
        row.state = 4;
        let unknown = key(&row, 7, true);
        row.subunit_index = 42;
        row.owner = 8;
        row.strength = 100;
        assert!(unknown == key(&row, 8, true));
    }
}
