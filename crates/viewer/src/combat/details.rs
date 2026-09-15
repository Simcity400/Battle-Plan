//! Format-22 operands override legacy gaps, without substituting a save or live state.
use super::{
    Selection, Values, Widget,
    binding::{Bindings, fill, hide, sprite, text},
};
use crate::{document::Document, map::tables::rows};
use replay::{
    frame::DecodedFrame,
    rows::{LandCombatDivisionRow, tables},
};
use std::{collections::BTreeMap, io};

pub(super) fn bind(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    selection: Selection,
    flags: &BTreeMap<u32, String>,
) -> io::Result<()> {
    match selection {
        Selection::Land(id) => {
            land(out, document, frame, id, flags)?;
            super::land_details::bind(out, document, frame, id)
        }
        Selection::Naval(id) => naval(out, document, frame, id, flags),
        Selection::NavalResult(_) => Ok(()),
    }
}

fn show(values: &mut Values, key: &str) {
    values.entry(key.into()).or_default().hidden = false;
}
fn flag(values: &mut Values, key: &str, country: u32, flags: &BTreeMap<u32, String>) {
    if let Some(flag) = flags.get(&country) {
        values.entry(key.into()).or_default().flag = Some(flag.clone());
    }
}
fn bytes(value: &[u8]) -> String {
    String::from_utf8_lossy(value).into_owned()
}
// Native signed qword multiplication precedes the fixed5 division.
fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / 100_000
}
fn fraction(value: i64, maximum: i64) -> f32 {
    ((value as f64 / maximum.max(100) as f64).clamp(0.0, 1.0)) as f32
}
fn entry(
    row: &LandCombatDivisionRow,
    committed: bool,
    bonus: Option<i64>,
    mut values: Values,
) -> Values {
    text(&mut values, "name", bytes(&row.name));
    let strength = fraction(row.strength, row.max_strength);
    fill(
        &mut values,
        "str_bar",
        if committed {
            (strength * row.strength_combat_factor as f32 / 100_000.0).clamp(0.0, 1.0)
        } else {
            strength
        },
    );
    fill(
        &mut values,
        "org_bar",
        fraction(row.organisation, row.max_organisation),
    );
    if committed {
        // 1415C7E00 -> 140BECA30/140BECB50, then side bonus and cached stats.
        let attack = bonus.map(|bonus| {
            mul(
                mul(row.scales[2], row.scales[0]),
                bonus.wrapping_add(100_000),
            )
        });
        let defence = mul(row.scales[3], row.scales[1]);
        for (key, raw) in [
            (
                "soft_attack",
                attack.map(|attack| mul(attack, row.stat_values[3])),
            ),
            (
                "hard_attack",
                attack.map(|attack| mul(attack, row.stat_values[4])),
            ),
            (
                "defense",
                Some(mul(defence, row.stat_values[usize::from(row.side == 0)])),
            ),
        ] {
            if let Some(raw) = raw {
                // Reference07 / pairJ_land_5: 369.58320 * .99000 displays
                // 365, not366. The native integer widget truncates fixed5.
                text(&mut values, key, (raw / 100_000).to_string());
            } else {
                values.entry(key.into()).or_default().text = None;
            }
        }
    }
    values
}
fn icons(values: &mut Values, key: &str, template: &str, frames: impl Iterator<Item = u32>) {
    values.insert(
        key.into(),
        Widget {
            entry_template: Some(template.into()),
            entries: frames
                .map(|frame| {
                    BTreeMap::from([(
                        "modifier_icon".into(),
                        Widget {
                            frame: Some(frame),
                            ..Widget::default()
                        },
                    )])
                })
                .collect(),
            ..Widget::default()
        },
    );
}

// A recorded role identifies the section independently of LandCombatDetails.
// The full-side ordinal is NOT the committed/reserve GUI order. Preserve it
// only as the explicitly unverified fallback when those ordered lists failed.
fn role_rosters<'a>(
    divisions: impl IntoIterator<Item = &'a LandCombatDivisionRow>,
    side: u8,
) -> [Vec<u64>; 2] {
    let mut divisions: Vec<_> = divisions
        .into_iter()
        .filter(|row| row.side == side)
        .collect();
    divisions.sort_by_key(|row| row.ordinal);
    let mut groups: [Vec<u64>; 2] = Default::default();
    for row in divisions {
        match row.role {
            1 => groups[0].push(row.division),
            2 => groups[1].push(row.division),
            _ => (),
        }
    }
    groups
}

fn land(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
    flags: &BTreeMap<u32, String>,
) -> io::Result<()> {
    let details = rows::<tables::LandCombatDetails>(frame)?;
    let detail = details.rows().iter().find(|r| r.combat == id);
    let legacy = rows::<tables::LandCombats>(frame)?;
    let combat = legacy.rows().iter().find(|r| r.id == id);
    let divisions = rows::<tables::LandCombatDivisions>(frame)?;
    let divisions_complete = divisions.is_complete();
    let divisions: BTreeMap<_, _> = divisions
        .rows()
        .iter()
        .filter(|r| r.combat == id)
        .map(|r| ((r.side, r.division), r))
        .collect();
    let modifiers = rows::<tables::LandCombatModifiers>(frame)?;
    if detail.is_none()
        && divisions.is_empty()
        && !modifiers.rows().iter().any(|row| row.combat == id)
    {
        return Ok(());
    }
    out.warnings
        .retain(|s| !s.starts_with("Combat UI: LandCombatDetails/"));
    if let Some(detail) = detail {
        let snow = rows::<tables::WeatherProvinces>(frame)?
            .rows()
            .iter()
            .find(|r| r.id == detail.province)
            .is_some_and(|r| r.snow > 0);
        let terrain = format!(
            "GFX_terrain_{}{}",
            bytes(&detail.terrain_name),
            if snow { "_winter" } else { "" }
        );
        if document.icons.combat.sprites.contains_key(&terrain) {
            sprite(&mut out.values, "terrain_picture", terrain);
        }
    }
    for side in 0..2 {
        let prefix = if side == 0 { "attacker" } else { "defender" };
        let old_ids = combat.map(|c| {
            if side == 0 {
                &c.attackers
            } else {
                &c.defenders
            }
        });
        let old: BTreeMap<_, _> = old_ids
            .into_iter()
            .flatten()
            .copied()
            .zip(std::mem::take(&mut out.entries[side]))
            .collect();
        let modifier = modifiers
            .rows()
            .iter()
            .find(|r| r.combat == id && r.side == side as u8);
        let fallback = if divisions.is_empty() {
            [old_ids.cloned().unwrap_or_default(), Vec::new()]
        } else {
            role_rosters(divisions.values().copied(), side as u8)
        };
        let [committed_ids, reserve_ids] = detail.map_or([&fallback[0], &fallback[1]], |detail| {
            [&detail.sides[side].committed, &detail.sides[side].reserves]
        });
        let roles_known = detail.is_some() || !divisions.is_empty();
        for (committed, ids) in [(true, committed_ids), (false, reserve_ids)] {
            let target = side + if committed { 0 } else { 2 };
            for unit in ids {
                let mut widgets = old.get(unit).cloned().unwrap_or_default();
                hide(&mut widgets, "unit_button");
                if !old.contains_key(unit) {
                    hide(&mut widgets, "unit_icon");
                }
                if let Some(row) = divisions.get(&(side as u8, *unit)) {
                    widgets = entry(
                        row,
                        committed,
                        detail.map(|detail| {
                            if side == 0 {
                                detail.attacker_bonus
                            } else {
                                detail.defender_bonus
                            }
                        }),
                        widgets,
                    );
                } else {
                    out.warnings.push(format!("LandCombatDivisions: missing combat {id} side {side} division {unit}; entry values remain unknown."));
                }
                if committed
                    && let Some(modifier) =
                        modifier.and_then(|m| m.divisions.iter().find(|m| m.division == *unit))
                {
                    icons(
                        &mut widgets,
                        "modifier_icons_container/list",
                        "modifier_icon_entry",
                        modifier.entries.iter().filter_map(|m| {
                            [2, 7, 10, 13, 27, 8, 17, 16]
                                .iter()
                                .position(|id| *id == m.id)
                                .map(|n| n as u32)
                        }),
                    );
                }
                out.entries[target].push(widgets);
            }
            let (kind, label) = if committed {
                ("committed", "COMBAT_COMMITED_LABEL")
            } else {
                ("reserves", "COMBAT_RESERVES_LABEL")
            };
            let fallback = if committed {
                "$COUNT$ Divisions"
            } else {
                "$COUNT$ Reserves"
            };
            let label = document
                .names
                .localisation
                .get(label)
                .unwrap_or(fallback)
                .replace(
                    "$COUNT$",
                    &if detail.is_some() || (roles_known && divisions_complete) {
                        ids.len().to_string()
                    } else {
                        "—".into()
                    },
                );
            text(
                &mut out.values,
                &format!("{prefix}_window/{prefix}_{kind}_label"),
                label,
            );
        }
        if let Some(detail) = detail {
            let value = &detail.sides[side];
            text(
                &mut out.values,
                &format!("{prefix}_leader_name"),
                if value.commander == 0 {
                    document
                        .names
                        .localisation
                        .get("UNIT_LEADER_NO_LEADER")
                        .unwrap_or("No Commander")
                        .to_owned()
                } else {
                    bytes(&value.commander_name)
                },
            );
            if value.commander != 0 {
                text(
                    &mut out.values,
                    &format!("{prefix}_leader_skill"),
                    value.commander_skill.to_string(),
                );
                flag(
                    &mut out.values,
                    &format!("{prefix}_flag"),
                    value.commander_country,
                    flags,
                );
            } else {
                sprite(
                    &mut out.values,
                    &format!("{prefix}_leader"),
                    "GFX_no_combat_leader".into(),
                );
            }
            if !detail.sides[0].air_counts.is_empty() || !detail.sides[1].air_counts.is_empty() {
                for key in ["air_icon", "air_count"] {
                    show(&mut out.values, &format!("{prefix}_{key}"));
                }
                text(
                    &mut out.values,
                    &format!("{prefix}_air_count"),
                    value
                        .air_counts
                        .iter()
                        .copied()
                        .fold(0_i32, i32::wrapping_add)
                        .to_string(),
                );
            }
        }
        if let Some(modifier) = modifier {
            let mut kinds = Vec::new();
            if modifier.indicator != 0 {
                kinds.push(8);
            }
            for (mask, kind) in [
                (0x8000, 2),
                (0x4000, 1),
                (0x200, 0),
                (0x800, 3),
                (0x1000, 4),
                (0x20, 5),
                (0x40000, 7),
                (0x800000, 9),
            ] {
                if modifier.flags & mask != 0 {
                    kinds.push(kind);
                }
            }
            icons(
                &mut out.values,
                &format!("{prefix}_window/modifier_icons_container/list"),
                "global_modifier_icon_entry",
                kinds.into_iter(),
            );
        }
    }
    if detail.is_none() {
        out.warnings.push("LandCombatDetails unavailable: recorded LandCombatDivisions names, HP/org bars, commitment roles and defence/breakthrough remain bound independently, as do LandCombatModifiers icons. UNVERIFIED: rows follow full-side ordinal within each role; exact GUI ordering requires the missing committed/reserve lists. Side attack bonuses, selected commanders, tactics, CAS, maximum width and heading remain unknown; division commanders do not substitute for selected-side commanders.".into());
    }
    out.warnings.push("Format-22 land: available row-level names, bars, roles and modifier kinds are independent of the parent-detail table. Defence/breakthrough uses recorded scales and truncates to whole units, as observed in reference07. Side-dependent attack, selected commander skill, CAS and exact ordered lists require LandCombatDetails. Derived armour/fuel/multiple-direction modifier icons, initiative/HQ markers, reinforcement and modifier tooltips, learning traits and possible-tactics menus remain unported; unsupported widgets stay empty/dash.".into());
    Ok(())
}

fn naval(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
    flags: &BTreeMap<u32, String>,
) -> io::Result<()> {
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    if let Some(detail) = details.rows().iter().find(|r| r.combat == id) {
        out.warnings
            .retain(|s| !s.starts_with("Combat UI: NavalCombatDetails/"));
        fill(
            &mut out.values,
            "header_container/progress_attacker",
            detail.progress as f32 / 100_000.0,
        );
        for (side, value) in detail.sides.iter().enumerate() {
            let label = if side == 0 { "attacker" } else { "defender" };
            let wnd = if side == 0 { "left" } else { "right" };
            for (key, raw) in [
                ("capital", value.capital_screening),
                ("carrier", value.carrier_screening),
            ] {
                text(
                    &mut out.values,
                    &format!("{wnd}_wnd_{key}_screening_efficiency"),
                    format!("{}%", raw / 1000),
                );
            }
            text(
                &mut out.values,
                &format!("header_container/{wnd}_wnd/stat_positioning"),
                format!("{}%", value.positioning / 1000),
            );
            text(
                &mut out.values,
                &format!("header_container/{wnd}_wnd/stat_anti_air"),
                format!("{:.1}", value.anti_air as f64 / 100_000.0),
            );
            if let Some(country) = value.summary.countries.first() {
                flag(
                    &mut out.values,
                    &format!("header_container/flag_{label}"),
                    *country,
                    flags,
                );
            }
            if value.commander == 0 {
                text(
                    &mut out.values,
                    &format!("header_container/leader_{label}_name"),
                    document
                        .names
                        .localisation
                        .get("UNIT_LEADER_NO_LEADER")
                        .unwrap_or("No Commander"),
                );
                sprite(
                    &mut out.values,
                    &format!("header_container/leader_{label}"),
                    "GFX_no_combat_leader".into(),
                );
            } else if let Some(commander) = &value.commander_inputs {
                show(&mut out.values, &format!("header_container/{label}_stats"));
                text(
                    &mut out.values,
                    &format!("header_container/leader_{label}_name"),
                    bytes(&commander.name),
                );
                for (index, stat) in ["attack", "defense", "maneuvering", "coordination"]
                    .iter()
                    .enumerate()
                {
                    text(
                        &mut out.values,
                        &format!("header_container/{label}_stats/leader_stat_{stat}_text"),
                        commander.skills[index + 1].to_string(),
                    );
                }
            }
        }
        out.warnings.push("Format-22 naval: progress, screening, positioning, AA, ordered side flags and commander name/skills are bound. Air groups, perspective-dependent progress skin and tooltips remain unported; the environment binder supplies recorded weather and weapon summaries. Phase has no separate scalar widget in navalcombat.gui; retreat/engagement operands are not relabelled as a guessed phase.".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn committed_strength_and_side_attack_use_distinct_native_operands() {
        let row = LandCombatDivisionRow {
            strength: 50_000,
            max_strength: 100_000,
            strength_combat_factor: 50_000,
            organisation: 30_000,
            max_organisation: 60_000,
            scales: [200_000, 50_000, 150_000, 100_000],
            stat_values: [8_000_000, 6_000_000, 0, 10_000_000, 2_000_000],
            ..Default::default()
        };
        let active = entry(&row, true, Some(10_000), Values::new());
        let reserve = entry(&row, false, Some(10_000), Values::new());
        assert_eq!(active["str_bar"].fill, Some(0.25));
        assert_eq!(reserve["str_bar"].fill, Some(0.5));
        assert_eq!(active["org_bar"].fill, Some(0.5));
        assert_eq!(active["soft_attack"].text.as_deref(), Some("330"));
        assert_eq!(active["hard_attack"].text.as_deref(), Some("66"));
        assert_eq!(active["defense"].text.as_deref(), Some("30"));
        assert_eq!(fraction(50, 0), 0.5); // Native maximum-stat clamp is 100 raw.
    }

    #[test]
    fn missing_parent_keeps_recorded_row_values_and_truncates_live_defence() {
        // Reference07, hour60828492: 169th 'Madison' Division shows365.
        let row = LandCombatDivisionRow {
            side: 1,
            name: b"169th 'Madison' Division".to_vec(),
            strength: 50_000,
            max_strength: 100_000,
            strength_combat_factor: 100_000,
            organisation: 30_000,
            max_organisation: 60_000,
            scales: [99_000, 99_000, 100_000, 100_000],
            stat_values: [36_958_320, 7_635_582, 833, 23_127_045, 5_736_750],
            ..Default::default()
        };
        let active = entry(&row, true, None, Values::new());
        assert_eq!(
            active["name"].text.as_deref(),
            Some("169th 'Madison' Division")
        );
        assert_eq!(active["str_bar"].fill, Some(0.5));
        assert_eq!(active["org_bar"].fill, Some(0.5));
        assert_eq!(active["defense"].text.as_deref(), Some("365"));
        assert_eq!(active["soft_attack"].text, None);
        assert_eq!(active["hard_attack"].text, None);
    }

    #[test]
    fn fallback_roles_use_full_side_ordinal_without_inventing_commitment() {
        let rows = [
            LandCombatDivisionRow {
                division: 90,
                role: 1,
                ordinal: 3,
                ..Default::default()
            },
            LandCombatDivisionRow {
                division: 80,
                role: 2,
                ordinal: 0,
                ..Default::default()
            },
            LandCombatDivisionRow {
                division: 70,
                role: 0,
                ordinal: 1,
                ..Default::default()
            },
            LandCombatDivisionRow {
                division: 60,
                role: 1,
                ordinal: 2,
                ..Default::default()
            },
            LandCombatDivisionRow {
                division: 50,
                side: 1,
                role: 1,
                ordinal: 0,
                ..Default::default()
            },
        ];
        assert_eq!(role_rosters(&rows, 0), [vec![60, 90], vec![80]]);
        assert_eq!(role_rosters(&rows, 1), [vec![50], vec![]]);
    }
}
