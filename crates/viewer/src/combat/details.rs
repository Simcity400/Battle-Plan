//! Format-22 operands override legacy gaps, without substituting a save or live state.
use super::{
    INLINE_FLAG, Selection, Values, Widget,
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
/// CCombatantEntry (1415BA240) names a committed division "name 0x12TAG" for
/// the attacker and "0x12TAG name" for the defender; the reserve entry
/// (1415BAC10) uses the bare name. The country is the division's owner
/// (+0x1DC when positive, else +0x1D8), recorded as `effective_owner`. The
/// 140C79AC0 alternative owner branch is not recorded: UNVERIFIED for units
/// whose +0x3B8 parent reports state 2.
fn entry_name(row: &LandCombatDivisionRow, committed: bool, flag: Option<&String>) -> String {
    let name = bytes(&row.name);
    match flag.filter(|_| committed) {
        Some(flag) if row.side == 0 => format!("{name} {INLINE_FLAG}{flag}"),
        Some(flag) => format!("{INLINE_FLAG}{flag} {name}"),
        None => name,
    }
}
fn entry(
    row: &LandCombatDivisionRow,
    committed: bool,
    bonus: Option<i64>,
    flag: Option<&String>,
    mut values: Values,
) -> Values {
    text(&mut values, "name", entry_name(row, committed, flag));
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

/// 1415C5F10 formats the selected commander's skill through the
/// `UNIT_LEADER_SKILL` key (`$SKILL|Y$` in the pinned English files), so the
/// digit carries the key's colour code rather than the font's colour.
fn skill_text(template: &str, skill: i32) -> String {
    let Some(start) = template.find("$SKILL") else {
        return template.to_owned();
    };
    let rest = &template[start + "$SKILL".len()..];
    let Some(end) = rest.find('$') else {
        return template.to_owned();
    };
    let value = match rest[..end]
        .strip_prefix('|')
        .and_then(|format| format.chars().find(char::is_ascii_alphabetic))
    {
        Some(colour) => format!("§{colour}{skill}§!"),
        None => skill.to_string(),
    };
    format!("{}{value}{}", &template[..start], &rest[end + 1..])
}

/// The side whose `*_leader_initiative_overlay_glow` 1415C8950 shows. 1412990E0
/// scores each side by its selected commander's skill (140C0A4A0, zero without
/// one) and adds the `RECON_SKILL_IMPACT` define to the side whose largest
/// reconnaissance (1413CCA80: stat +0xD0 over the full +0x20 roster, from zero)
/// is strictly greater. The higher score glows; a tie shows neither.
/// pairL_land_danzig.png: Popov 4 against Leese 5, reconnaissance 0 against
/// 2.0, so the defender glows.
fn initiative_side(skills: [i32; 2], reconnaissance: [i64; 2], impact: i32) -> Option<usize> {
    let mut scores = skills;
    match reconnaissance[0].cmp(&reconnaissance[1]) {
        std::cmp::Ordering::Greater => scores[0] = scores[0].wrapping_add(impact),
        std::cmp::Ordering::Less => scores[1] = scores[1].wrapping_add(impact),
        std::cmp::Ordering::Equal => (),
    }
    match scores[0].cmp(&scores[1]) {
        std::cmp::Ordering::Greater => Some(0),
        std::cmp::Ordering::Less => Some(1),
        std::cmp::Ordering::Equal => None,
    }
}

// A recorded role identifies the section independently of LandCombatDetails.
// The full-side ordinal is NOT the committed/reserve GUI order, which is the
// CLandCombatant +0xC8/+0xE0 array order LandCombatDetails copies. The ordinal
// order is only the fallback the limitation registry states when that table
// is missing (formats 14 and 22).
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

/// CLandCombatant vtable+F8 (1413CD7E0): true once two of the side's units
/// (CCombatant+0x20, the full LandCombats roster in order) stand in different
/// provinces. A unit's province is its location (+0x1F0), or its movement
/// source (+0x1F8) when it stands in the combat province and has one.
/// CLandBorderWarCombatant's slot (14011CD10) always returns false.
fn several_directions(
    combat: &replay::rows::LandCombatRow,
    divisions: &BTreeMap<(u8, u64), &LandCombatDivisionRow>,
    movement: &BTreeMap<u64, u32>,
) -> Option<bool> {
    let mut first = None;
    for unit in &combat.attackers {
        let row = divisions.get(&(0, *unit))?;
        let province = if row.province == combat.province {
            match *movement.get(unit)? {
                0 => row.province,
                source => source,
            }
        } else {
            row.province
        };
        match first {
            None => first = Some(province),
            Some(seen) if seen != province => return Some(true),
            Some(_) => (),
        }
    }
    Some(false)
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
    let movement: BTreeMap<_, _> = rows::<tables::DivisionMovement>(frame)?
        .rows()
        .iter()
        .map(|row| (row.division, row.source))
        .collect();
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
        if document.icons.combat.has_sprite(&terrain) {
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
        let old: BTreeMap<_, _> = std::mem::take(&mut out.entries[side]).into_iter().collect();
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
                        flags.get(&row.context.effective_owner),
                        widgets,
                    );
                } else {
                    out.warnings.push(format!("LandCombatDivisions: missing combat {id} side {side} division {unit}; entry values remain unknown."));
                }
                if committed
                    && let Some(modifier) =
                        modifier.and_then(|m| m.divisions.iter().find(|m| m.division == *unit))
                    && let Some(row) = divisions.get(&(side as u8, *unit))
                {
                    let opponents: Option<Vec<_>> = detail.map_or(Some(Vec::new()), |detail| {
                        detail.sides[1 - side]
                            .committed
                            .iter()
                            .map(|enemy| divisions.get(&(1 - side as u8, *enemy)).copied())
                            .collect()
                    });
                    if detail.is_some() && opponents.is_none() {
                        out.warnings.push(format!("Land division icons: an opposing committed LandCombatDivisions row of combat {id} is missing; derived frames from armour onwards are omitted."));
                    }
                    icons(
                        &mut widgets,
                        "modifier_icons_container/list",
                        "modifier_icon_entry",
                        super::division_icons::frames(
                            modifier.entries.iter().map(|m| m.id),
                            detail.map(|detail| (detail, side)),
                            row,
                            opponents.as_deref(),
                        )
                        .into_iter(),
                    );
                }
                out.entries[target].push((*unit, widgets));
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
                    skill_text(
                        document
                            .names
                            .localisation
                            .get("UNIT_LEADER_SKILL")
                            .unwrap_or("$SKILL$"),
                        value.commander_skill,
                    ),
                );
                flag(
                    &mut out.values,
                    &format!("{prefix}_flag"),
                    value.commander_country,
                    flags,
                );
            } else {
                // 1415C5F10 writes an empty skill beside UNIT_LEADER_NO_LEADER.
                text(&mut out.values, &format!("{prefix}_leader_skill"), "");
                sprite(
                    &mut out.values,
                    &format!("{prefix}_leader"),
                    "GFX_no_combat_leader".into(),
                );
            }
            // 1415C5F10 shows the HQ badge (attacker/defender_leader_army_hq_icon)
            // exactly for the recorded HQ unit; zero hides it for any reason and
            // the committed-membership test stays native.
            if value.hq_unit != 0 && value.committed.contains(&value.hq_unit) {
                show(&mut out.values, &format!("{prefix}_leader_army_hq_icon"));
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
            // 1415C9860's order. Frame 6 is attacker-only and asks the
            // combatant's vtable+F8, which for CLandCombatant is 1413CD7E0.
            for (mask, kind) in [
                (0x8000, 2),
                (0x4000, 1),
                (0x200, 0),
                (0x800, 3),
                (0x1000, 4),
                (0x20, 5),
                (0, 6),
                (0x40000, 7),
                (0x800000, 9),
            ] {
                if mask == 0 {
                    if side == 0 {
                        match combat.map(|combat| {
                            several_directions(combat, &divisions, &movement)
                        }) {
                            Some(Some(true)) => kinds.push(kind),
                            Some(Some(false)) => (),
                            _ => out.warnings.push("Land attacker multiple-direction icon: an attacker's LandCombatDivisions location or DivisionMovement source is missing; the icon is omitted rather than guessed.".into()),
                        }
                    }
                } else if modifier.flags & mask != 0 {
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
    if let (Some(detail), Some(combat)) = (detail, combat) {
        let reconnaissance = |side: u8, roster: &[u64]| {
            roster.iter().try_fold(0_i64, |best, unit| {
                Some(best.max(divisions.get(&(side, *unit))?.reconnaissance))
            })
        };
        match (
            reconnaissance(0, &combat.attackers),
            reconnaissance(1, &combat.defenders),
        ) {
            (Some(attacker), Some(defender)) => {
                let skills = detail.sides.each_ref().map(|side| side.commander_skill);
                if let Some(side) =
                    initiative_side(skills, [attacker, defender], detail.recon_skill_impact)
                {
                    show(
                        &mut out.values,
                        &format!(
                            "{}_leader_initiative_overlay_glow",
                            ["attacker", "defender"][side]
                        ),
                    );
                }
            }
            _ => out.warnings.push("Land initiative glow: a participant's LandCombatDivisions reconnaissance is missing; neither glow is drawn rather than guessed.".into()),
        }
    }
    if detail.is_none() {
        out.warnings.push("LandCombatDetails unavailable: recorded LandCombatDivisions names, HP/org bars, commitment roles and defence/breakthrough remain bound independently, as do LandCombatModifiers icons. Side attack bonuses, selected commanders, tactics, CAS, maximum width and heading remain unknown; division commanders do not substitute for selected-side commanders.".into());
    }
    out.warnings.push("Format-22 land: available row-level names, bars, roles and modifier kinds are independent of the parent-detail table. Defence/breakthrough uses recorded scales and truncates to whole units, as observed in reference07. Side-dependent attack, selected commander skill, CAS, the committed/reserve array order and the derived division icons require LandCombatDetails; icons 14/15 and the HQ marker additionally require format-25 rows and stay absent on older files. Reinforcement and modifier tooltips, learning traits and possible-tactics menus remain unported; unsupported widgets stay empty/dash.".into());
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
        let active = entry(&row, true, Some(10_000), None, Values::new());
        let reserve = entry(&row, false, Some(10_000), None, Values::new());
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
        let active = entry(&row, true, None, None, Values::new());
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
    fn committed_names_carry_the_owner_flag_on_the_side_the_native_entry_writes() {
        let mut row = LandCombatDivisionRow {
            name: b"144ya Strelkovaya Diviziya".to_vec(),
            ..Default::default()
        };
        let flag = "SOV_communism".to_string();
        assert_eq!(
            entry_name(&row, true, Some(&flag)),
            format!("144ya Strelkovaya Diviziya {INLINE_FLAG}SOV_communism")
        );
        assert_eq!(
            entry_name(&row, false, Some(&flag)),
            "144ya Strelkovaya Diviziya"
        );
        row.side = 1;
        assert_eq!(
            entry_name(&row, true, Some(&flag)),
            format!("{INLINE_FLAG}SOV_communism 144ya Strelkovaya Diviziya")
        );
        assert_eq!(entry_name(&row, true, None), "144ya Strelkovaya Diviziya");
    }

    #[test]
    fn skill_uses_the_localised_colour() {
        // pairL_land_danzig.png: yellow 4 and 5 from UNIT_LEADER_SKILL.
        assert_eq!(skill_text("$SKILL|Y$", 5), "§Y5§!");
        assert_eq!(skill_text("Skill $SKILL$", 4), "Skill 4");
    }

    #[test]
    fn initiative_adds_the_reconnaissance_define_before_comparing_skills() {
        // pairL_land_danzig.png at 60828493: the defender glows.
        assert_eq!(initiative_side([4, 5], [0, 200_000], 5), Some(1));
        // Reconnaissance can carry the lower skill past the higher one.
        assert_eq!(initiative_side([4, 5], [200_000, 0], 5), Some(0));
        // A tie in the score shows neither glow; equal reconnaissance adds nothing.
        assert_eq!(initiative_side([3, 3], [100_000, 100_000], 5), None);
        assert_eq!(initiative_side([0, 0], [0, 1], 0), None);
    }

    #[test]
    fn several_directions_compare_locations_or_the_source_in_the_combat_province() {
        let combat = replay::rows::LandCombatRow {
            province: 362,
            attackers: vec![1, 2],
            ..Default::default()
        };
        let rows = [
            LandCombatDivisionRow {
                division: 1,
                province: 9263,
                ..Default::default()
            },
            LandCombatDivisionRow {
                division: 2,
                province: 389,
                ..Default::default()
            },
        ];
        let mut divisions: BTreeMap<_, _> = rows.iter().map(|r| ((0, r.division), r)).collect();
        let mut movement = BTreeMap::new();
        // pairJ_land_5.png: 144ya at 9263 and 2-p Morit at 389 show the icon.
        assert_eq!(
            several_directions(&combat, &divisions, &movement),
            Some(true)
        );
        let entered = [
            LandCombatDivisionRow {
                division: 1,
                province: 362,
                ..Default::default()
            },
            LandCombatDivisionRow {
                division: 2,
                province: 9263,
                ..Default::default()
            },
        ];
        divisions = entered.iter().map(|r| ((0, r.division), r)).collect();
        assert_eq!(several_directions(&combat, &divisions, &movement), None);
        movement.insert(1, 9263);
        assert_eq!(
            several_directions(&combat, &divisions, &movement),
            Some(false)
        );
        movement.insert(1, 0);
        assert_eq!(
            several_directions(&combat, &divisions, &movement),
            Some(true)
        );
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
