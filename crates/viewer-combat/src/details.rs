use content_reader_division_names::{division_names::Database, division_names::Unresolved};
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{
    rows::LandCombatDivisionRow, rows::LandCombatMember, rows::TableSchema,
    rows::land_combat_members, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
use viewer_foundation::tooltip_text::tooltip_keys;
use super::{
    INLINE_FLAG,
    Selection,
    Values,
    Widget,
    binding::{Bindings, FILL, FLAG, Flags, SPRITE, TEXT, country_flag, fill, hide, text},
    template_icons,
    tooltips::{hover_text, set_hover},
};
use viewer_engagements::land_combat_math::{Condition, combat_rows, conditions, member_values};
use viewer_foundation::tooltip_text::{TextIcons, builtin, displayable, token};
use super::content::Content as StaticContent;
use viewer_foundation::{perspective::Vision, tables::{Available, rows}};

pub(super) fn bind(
    out: &mut Bindings,
    static_content: &StaticContent,
    frame: &DecodedFrame,
    selection: Selection,
    flags: &Flags,
    vision: &Vision,
) -> io::Result<()> {
    match selection {
        Selection::Land(id) => {
            land(out, static_content, frame, id, flags)?;
            super::land_details::bind(out, static_content, frame, id)
        }
        Selection::Naval(id) => naval(out, static_content, frame, id, flags, vision),
        Selection::NavalResult(_) | Selection::AirRegion(_) | Selection::State(_) | Selection::NavalRegion(_) => Ok(()),
    }
}

fn show(values: &mut Values, key: &str) {
    values.entry(key.into()).or_default().hidden = false;
}

fn pictures(
    out: &mut Bindings,
    frame: &DecodedFrame,
    id: u64,
    terrain: &BTreeMap<u32, String>,
    has_sprite: &dyn Fn(&str) -> bool,
    flags: &Flags,
) -> io::Result<()> {
    let details = rows::<tables::LandCombatDetails>(frame)?;
    let detail = details.rows().iter().find(|row| row.combat == id);
    let province = match detail {
        Some(detail) => Some(detail.province),
        None => rows::<tables::LandCombats>(frame)?
            .rows()
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.province),
    };
    let picture = terrain_picture(frame, id, province, terrain, has_sprite)?;
    out.bind("terrain_picture", SPRITE, picture);
    for (side, prefix) in ["attacker", "defender"].into_iter().enumerate() {
        let flag = match detail {
            Some(detail) => country_flag(flags, detail.sides[side].commander_country),
            None => Err(format!(
                "LandCombatDetails has no row for the combat ({}), so the side country the game selects (the commander's, else the first primary country, else the province controller) is unknown",
                absence(&details)
            )),
        }
        .map_err(|reason| format!("Land combat {id} {prefix} flag is hidden: {reason}."));
        out.bind(&format!("{prefix}_flag"), FLAG, flag);
    }
    Ok(())
}

fn terrain_picture(
    frame: &DecodedFrame,
    id: u64,
    province: Option<u32>,
    terrain: &BTreeMap<u32, String>,
    has_sprite: &dyn Fn(&str) -> bool,
) -> io::Result<Result<String, String>> {
    let refused = |reason: String| {
        Ok(Err(format!(
            "Land combat {id} terrain picture is hidden: {reason}."
        )))
    };
    let Some(province) = province else {
        return refused(
            "neither LandCombatDetails nor LandCombats records the combat's province".into(),
        );
    };
    let Some(category) = terrain.get(&province) else {
        return refused(format!(
            "province {province} has no terrain category in the content"
        ));
    };
    let weather = rows::<tables::WeatherProvinces>(frame)?;
    let Some(weather_row) = weather.rows().iter().find(|row| row.id == province) else {
        return refused(format!(
            "WeatherProvinces has no row for province {province} ({}), so its snow and the summer or `_winter` picture are unknown",
            absence(&weather)
        ));
    };
    let name = format!(
        "GFX_terrain_{category}{}",
        if weather_row.snow > 0 { "_winter" } else { "" }
    );
    if has_sprite(&name) {
        Ok(Ok(name))
    } else {
        refused(format!("the content lacks sprite {name}"))
    }
}
fn absence<T: TableSchema>(table: &Available<'_, T>) -> String {
    match table {
        Available::Complete(_) => "the complete table lacks it".to_owned(),
        Available::Truncated { missing, .. } => {
            format!("the table is truncated, {missing} rows unread")
        }
        Available::Unavailable { reason } => format!("the table is unavailable: {reason}"),
        Available::NotRecorded { mode } => {
            format!("the table is not recorded in a {mode} recording")
        }
    }
}
fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / 100_000
}
fn fraction(value: i64, maximum: i64) -> f32 {
    ((value as f64 / maximum.max(100) as f64).clamp(0.0, 1.0)) as f32
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Unnamed {
    CustomNotText,
    StoredText(String),
    Unavailable(String),
    Control(String),
    ForeignType(i32),
    Unresolved(Unresolved),
}

impl std::fmt::Display for Unnamed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CustomNotText => f.write_str("the custom name is not UTF-8"),
            Self::StoredText(reason) => f.write_str(reason),
            Self::Unavailable(reason) => f.write_str(reason),
            Self::Control(reason) => write!(f, "the name holds {reason}"),
            Self::ForeignType(kind) => write!(
                f,
                "the name type is {kind}, a ship, operative or railway-gun naming form no division content produces"
            ),
            Self::Unresolved(reason) => write!(f, "{reason}"),
        }
    }
}

fn division_name(
    row: &LandCombatMember<'_>,
    names: &Database,
    localisation: &Localisation,
    recorded_language: &str,
) -> Result<String, Unnamed> {
    let naming = &row.values.ok_or_else(|| Unnamed::Unavailable(UNRECORDED_NAMING.into()))?.naming;
    division_name_operands(
        division_custom_name(
            &naming.custom,
            naming.programmatic,
            &naming.group,
            localisation,
            recorded_language,
        )?,
        naming.kind,
        &naming.group,
        naming.order,
        naming.ordered != 0,
        names,
    )
}

const UNRECORDED_NAMING: &str =
    "its naming operands are in neither its LandCombatDivisions row nor its division's rows";

fn division_custom_name(
    custom: &[u8],
    programmatic: u8,
    group: &[u8],
    localisation: &Localisation,
    recorded_language: &str,
) -> Result<Option<String>, Unnamed> {
    if custom.is_empty() || (programmatic != 0 && !group.is_empty()) {
        return Ok(None);
    }
    if programmatic != 0 {
        return dashboard_text::scripted_text::stored_text(
            localisation,
            recorded_language,
            custom,
            "programmatic division name",
        )
        .map(Some)
        .map_err(Unnamed::StoredText);
    }
    std::str::from_utf8(custom)
        .map(|name| Some(name.to_owned()))
        .map_err(|_| Unnamed::CustomNotText)
}

fn division_name_operands(
    custom: Option<String>,
    kind: i32,
    group: &[u8],
    order: i32,
    ordered: bool,
    names: &Database,
) -> Result<String, Unnamed> {
    if let Some(custom) = custom {
        return Ok(custom);
    }
    if kind != 0 {
        return Err(Unnamed::ForeignType(kind));
    }
    names
        .resolve(group, order, ordered)
        .map_err(Unnamed::Unresolved)
}

fn shown_name(
    row: &LandCombatMember<'_>,
    names: &Database,
    icons: &impl TextIcons,
    localisation: &Localisation,
    recorded_language: &str,
) -> Result<String, Unnamed> {
    division_name(row, names, localisation, recorded_language)
        .and_then(|name| displayable(icons, name).map_err(Unnamed::Control))
}

fn shown_recorded_name(
    row: &LandCombatMember<'_>,
    status: Option<&replay_data::rows::DivisionStatusRow>,
    status_gap: &str,
    names: &Database,
    icons: &impl TextIcons,
    localisation: &Localisation,
    recorded_language: &str,
) -> Result<String, Unnamed> {
    let status = status.ok_or_else(|| {
        Unnamed::Unavailable(format!(
            "DivisionStatus has no cached-name row for division {} ({status_gap})",
            row.division
        ))
    })?;
    if matches!(status.name, replay_data::rows::Recorded::NotRecorded) {
        return shown_name(row, names, icons, localisation, recorded_language);
    }
    dashboard_text::unit_names::army_name(status, localisation, recorded_language, || Ok(names))
        .map_err(Unnamed::Unavailable)
        .and_then(|name| displayable(icons, name).map_err(Unnamed::Control))
}

fn unnamed_warnings<'a>(
    combat: u64,
    rows: impl Iterator<Item = LandCombatMember<'a>>,
    name: impl Fn(&LandCombatMember<'_>) -> Result<String, Unnamed>,
) -> Vec<String> {
    let mut causes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        if let Err(cause) = name(&row) {
            let division = match row.values {
                Some(values) => format!(
                    "division {} (group {}, {} {})",
                    row.division,
                    values.naming.group.escape_ascii(),
                    if values.naming.ordered != 0 {
                        "ordered"
                    } else {
                        "unordered"
                    },
                    values.naming.order
                ),
                None => format!("division {}", row.division),
            };
            causes
                .entry(cause.to_string())
                .or_default()
                .push(division);
        }
    }
    causes
        .into_iter()
        .map(|(cause, divisions)| {
            format!(
                "Land combat {combat}: {} division names are unavailable because {cause}: {}.",
                divisions.len(),
                divisions.join("; ")
            )
        })
        .collect()
}

fn entry_name(name: String, side: u8, committed: bool, flag: Option<&String>) -> String {
    match flag.filter(|_| committed) {
        Some(flag) if side == 0 => format!("{name} {INLINE_FLAG}{flag}"),
        Some(flag) => format!("{INLINE_FLAG}{flag} {name}"),
        None => name,
    }
}

fn condition_bars(
    out: &mut Bindings,
    values: &mut Values,
    row: &LandCombatMember<'_>,
    committed: bool,
    bars: Result<Condition, String>,
    panel_gap: &str,
) {
    let bars = match bars {
        Ok(bars) => bars,
        Err(refusal) => return out.withhold_in(values, &["str_bar", "org_bar"], refusal),
    };
    let unbounded = |bar: &str, maximum: &str| {
        format!(
            "Land combat {} division {}: DivisionPanelStats has no {maximum} maximum for it ({panel_gap}); its {bar} bar is hidden.",
            row.combat, row.division
        )
    };
    let strength = bars
        .max_strength
        .map(|maximum| fraction(bars.strength, maximum))
        .ok_or_else(|| unbounded("strength", "hit points"));
    out.bind_in(
        values,
        "str_bar",
        FILL,
        strength.and_then(|strength| {
            if !committed {
                return Ok(strength);
            }
            let factor = row.values.map(|values| values.strength_combat_factor).ok_or_else(|| {
                format!(
                    "Land combat {} division {}: its strength factor is in neither its LandCombatDivisions row nor its division's rows; its strength bar is hidden.",
                    row.combat, row.division
                )
            })?;
            Ok((strength * factor as f32 / 100_000.0).clamp(0.0, 1.0))
        }),
    );
    let organisation = bars
        .max_organisation
        .map(|maximum| fraction(bars.organisation, maximum))
        .ok_or_else(|| unbounded("organisation", "organisation"));
    out.bind_in(values, "org_bar", FILL, organisation);
}

fn entry(
    row: &LandCombatMember<'_>,
    name: Option<String>,
    committed: bool,
    bonus: Option<i64>,
    flag: Option<&String>,
    mut values: Values,
) -> Values {
    let name_widget = values.entry("name".into()).or_default();
    name_widget.hidden = name.is_none();
    name_widget.text = name.map(|name| entry_name(name, row.side, committed, flag));
    if committed {
        let (scales, stats) = (row.scales, row.values.map(|values| values.stat_values));
        let attack =
            bonus.map(|bonus| mul(mul(scales[2], scales[0]), bonus.wrapping_add(100_000)));
        let defence = Some(mul(scales[3], scales[1]));
        for (key, raw) in [
            (
                "soft_attack",
                attack
                    .zip(stats)
                    .map(|(attack, stats)| mul(attack, stats[3])),
            ),
            (
                "hard_attack",
                attack
                    .zip(stats)
                    .map(|(attack, stats)| mul(attack, stats[4])),
            ),
            (
                "defense",
                defence
                    .zip(stats)
                    .map(|(defence, stats)| mul(defence, stats[usize::from(row.side == 0)])),
            ),
        ] {
            if let Some(raw) = raw {
                text(
                    &mut values,
                    key,
                    content_reader_parser::fixed5::fixed5_to_string(raw, 0),
                );
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

type ModifierHover = (
    Result<Option<String>, String>,
    Option<Result<String, String>>,
);

fn global_modifier_tooltip(
    icons: &impl TextIcons,
    loc: &Localisation,
    frame: u32,
    dlc_mask: Option<u32>,
    border: Option<[i64; 3]>,
) -> ModifierHover {
    if frame == 8 {
        let Some([modifier, dig_in, terrain]) = border else {
            return (
                Err("the side's border-war factors are not recorded".to_owned()),
                None,
            );
        };
        let mut main = String::new();
        if modifier != 0 {
            match builtin(
                icons,
                loc,
                BORDER_CONFLICT_MODIFIER,
                &[("VAL", Argument::Fixed5(modifier))],
            ) {
                Ok(line) => main.push_str(&line),
                Err(reason) => return (Err(reason), None),
            }
        }
        for (value, key) in [
            (terrain, BORDER_CONFLICT_TERRAIN_FACTOR),
            (dig_in, BORDER_CONFLICT_DIG_IN_FACTOR),
        ] {
            if value == 100_000 {
                continue;
            }
            let entry = match builtin(
                icons,
                loc,
                BORDER_CONFLICT_FACTOR_ENTRY,
                &[("VAL", Argument::Fixed5(value))],
            ) {
                Ok(entry) => entry,
                Err(reason) => return (Err(reason), None),
            };
            let colour = if value < 100_000 { 'R' } else { 'H' };
            let coloured = format!("\u{11}{colour}{entry}\u{11}!");
            if !main.is_empty() && !main.ends_with('\n') {
                main.push('\n');
            }
            match builtin(icons, loc, key, &[("VAL", Argument::Text(&coloured))]) {
                Ok(line) => main.push_str(&line),
                Err(reason) => return (Err(reason), None),
            }
        }
        return (Ok((!main.is_empty()).then_some(main)), None);
    }
    let (key, description) = match frame {
        0 => (
            BM_FORT_MODIFIER,
            dlc_mask.map(|mask| {
                if mask & (1 << 7) != 0 {
                    BM_FORT_MODIFIER_DESC_WITH_RAILWAY_GUN
                } else {
                    BM_FORT_MODIFIER_DESC
                }
            }),
        ),
        1 => (
            BM_ENEMY_AIR_SUPERIORITY,
            Some(BM_ENEMY_AIR_SUPERIORITY_DESC),
        ),
        2 => (BM_AIR_SUPPORT, Some(BM_AIR_SUPPORT_DESC)),
        3 => (BM_SHORE_BOMBARD, Some(BM_SHORE_BOMBARD_DESC)),
        4 => (BM_RAILWAY_GUN_BOMBARD, Some(BM_RAILWAY_GUN_BOMBARD_DESC)),
        5 => (BM_STACKING, Some(BM_STACKING_DESC)),
        6 => (BM_MULTIPLE_DIRECTIONS, Some(BM_MULTIPLE_DIRECTIONS_DESC)),
        7 => (BM_DECRYPTION, Some(BM_DECRYPTION_DESC)),
        9 => (
            BM_ACTIVE_CODE_CRACKING_BONUSES,
            Some(BM_ACTIVE_CODE_CRACKING_BONUSES_DESC),
        ),
        _ => return (Ok(None), None),
    };
    let main = builtin(icons, loc, key, &[])
        .and_then(|name| {
            builtin(
                icons,
                loc,
                super::tooltips::HEADER,
                &[("KEY", Argument::Text(&name))],
            )
        })
        .map(Some);
    let delayed = description.map(|key| builtin(icons, loc, key, &[]));
    (main, delayed)
}

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

fn several_directions<D: std::ops::Deref<Target = LandCombatDivisionRow>>(
    combat: &replay_data::rows::LandCombatRow,
    border_war: bool,
    divisions: &BTreeMap<(u8, u64), D>,
    movement: &BTreeMap<u64, u32>,
) -> Option<bool> {
    if border_war {
        return Some(false);
    }
    let mut first = None;
    for unit in &combat.attackers {
        let row = divisions.get(&(0, *unit))?;
        let province = super::land_details::direction_key(row, combat.province, movement)?;
        match first {
            None => first = Some(province),
            Some(seen) if seen != province => return Some(true),
            Some(_) => (),
        }
    }
    Some(false)
}

pub(super) fn division_modifiers<'a>(
    rows: &'a [replay_data::rows::LandCombatDivisionModifierRow],
    members: &[LandCombatDivisionRow],
    id: u64,
) -> BTreeMap<(u8, u64), &'a replay_data::rows::LandCombatDivisionModifierRow> {
    let mut found = BTreeMap::new();
    for row in rows.iter().filter(|row| row.combat == id) {
        if let Some(division) = row.division_in(members) {
            found.entry((row.side, division)).or_insert(row);
        }
    }
    found
}

pub(super) fn border_war(
    rows: &[replay_data::rows::LandCombatModifierRow],
    id: u64,
) -> Option<bool> {
    rows.iter()
        .find(|row| row.combat == id)
        .map(|row| row.border_values.is_some())
}

fn land(
    out: &mut Bindings,
    static_content: &StaticContent,
    frame: &DecodedFrame,
    id: u64,
    flags: &Flags,
) -> io::Result<()> {
    let details = rows::<tables::LandCombatDetails>(frame)?;
    let detail = details.rows().iter().find(|r| r.combat == id);
    let legacy = rows::<tables::LandCombats>(frame)?;
    let combat = legacy.rows().iter().find(|r| r.id == id);
    let division_rows = rows::<tables::LandCombatDivisions>(frame)?;
    let divisions_complete = division_rows.is_complete();
    let combat_members = combat_rows(division_rows.rows(), id);
    let member_values = member_values(frame, combat_members)?;
    let divisions: BTreeMap<_, _> = land_combat_members(combat_members, &member_values)
        .map(|r| ((r.side, r.division), r))
        .collect();
    let modifiers = rows::<tables::LandCombatModifiers>(frame)?;
    let members = rows::<tables::LandCombatDivisionModifiers>(frame)?;
    let member_gap = absence(&members);
    let members = division_modifiers(members.rows(), combat_members, id);
    let marker = rows::<tables::LandCombatSides>(frame)?;
    let marker = marker.rows().iter().find(|row| row.combat == id);
    let features = rows::<tables::SimulationFeatures>(frame)?;
    let dlc_mask = match features.rows() {
        [feature] if features.is_complete() => Some(feature.dlc_mask),
        _ => None,
    };
    let bars = conditions(frame)?;
    let division_gap = absence(&rows::<tables::Divisions>(frame)?);
    let statuses = rows::<tables::DivisionStatus>(frame)?;
    let status_gap = absence(&statuses);
    let panel_gap = absence(&rows::<tables::DivisionPanelStats>(frame)?);
    let statuses: BTreeMap<_, _> = statuses
        .rows()
        .iter()
        .filter(|status| {
            divisions.contains_key(&(0, status.division))
                || divisions.contains_key(&(1, status.division))
        })
        .map(|status| (status.division, status))
        .collect();
    let exiles = template_icons::Inputs::read(frame)?;
    let content = static_content.assets.land_defines.as_ref();
    if let Err(reason) = content {
        out.warnings.push(format!("Land combat defines are unavailable ({reason}); derived division icons and the initiative glow are omitted."));
    }
    let content = content.ok();
    let names = &static_content.assets.division_names;
    let shown = |row: &LandCombatMember<'_>| {
        shown_recorded_name(
            row,
            statuses.get(&row.division).copied(),
            &status_gap,
            names,
            static_content.assets.as_ref(),
            &static_content.localisation,
            &static_content.recorded_language,
        )
    };
    out.warnings
        .extend(unnamed_warnings(id, divisions.values().copied(), shown));
    let mut undecided: BTreeMap<(&str, String), Vec<u64>> = BTreeMap::new();
    let movement: BTreeMap<_, _> = rows::<tables::DivisionMovement>(frame)?
        .rows()
        .iter()
        .map(|row| (row.division, row.source))
        .collect();
    let has_sprite = |name: &str| static_content.assets.has_sprite(name);
    pictures(
        out,
        frame,
        id,
        &static_content.terrain.combat_terrain,
        &has_sprite,
        flags,
    )?;
    if detail.is_none()
        && divisions.is_empty()
        && !modifiers.rows().iter().any(|row| row.combat == id)
    {
        return Ok(());
    }
    out.warnings
        .retain(|s| !s.starts_with("Combat UI: LandCombatDetails/"));
    let mut omitted = BTreeMap::new();
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
            role_rosters(divisions.values().map(|member| member.row), side as u8)
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
                    let flag = if committed {
                        let cause = match exiles.entry_owner(row.row) {
                            Ok(owner) => match flags.get(&owner) {
                                Some(Ok(flag)) => Ok(flag),
                                Some(Err(reason)) => Err(reason.clone()),
                                None => Err(format!(
                                    "no combat flag resolves for its owner country {owner}"
                                )),
                            },
                            Err(refused) => Err(refused.to_string()),
                        };
                        cause
                            .map_err(|cause| {
                                undecided
                                    .entry(("entry flag", cause))
                                    .or_default()
                                    .push(*unit)
                            })
                            .ok()
                    } else {
                        None
                    };
                    widgets = entry(
                        row,
                        shown(row).ok(),
                        committed,
                        detail.map(|detail| {
                            if side == 0 {
                                detail.attacker_bonus
                            } else {
                                detail.defender_bonus
                            }
                        }),
                        flag,
                        widgets,
                    );
                    let condition = bars.get(unit).copied().ok_or_else(|| {
                        format!(
                            "Land combat {id} division {unit}: its Divisions row is missing ({division_gap}); its strength and organisation bars are hidden."
                        )
                    });
                    condition_bars(out, &mut widgets, row, committed, condition, &panel_gap);
                } else {
                    out.warnings.push(format!("LandCombatDivisions: missing combat {id} side {side} division {unit}; entry values remain unknown."));
                }
                if committed
                    && let Some(modifier) = members.get(&(side as u8, *unit))
                    && let Some(row) = divisions.get(&(side as u8, *unit))
                {
                    let opponents: Option<Vec<_>> = detail.map_or(Some(Vec::new()), |detail| {
                        detail.sides[1 - side]
                            .committed
                            .iter()
                            .map(|enemy| divisions.get(&(1 - side as u8, *enemy)).copied())
                            .collect()
                    });
                    let exile = match (detail, content) {
                        (Some(detail), Some(content)) => {
                            let [occupier, core] =
                                exiles.frames(row.row, detail, &content.exile_against_occupier);
                            [
                                ("exile occupier badge (frame 14)", occupier),
                                ("exile core badge (frame 15)", core),
                            ]
                            .map(|(badge, frame)| {
                                frame
                                    .map_err(|refused| {
                                        undecided
                                            .entry((badge, refused.to_string()))
                                            .or_default()
                                            .push(*unit)
                                    })
                                    .ok()
                            })
                        }
                        _ => [None; 2],
                    };
                    let (frames, cut) = super::division_icons::frames(
                        modifier.entries.iter().map(|m| m.id),
                        detail.map(|detail| (detail, side)),
                        marker.map(|marker| marker.flags[side]),
                        *row,
                        opponents.as_deref(),
                        content,
                        exile,
                    );
                    if let Some(cause) = cut {
                        *omitted.entry(cause).or_insert(0_usize) += 1;
                    }
                    icons(
                        &mut widgets,
                        "modifier_icons_container/list",
                        "modifier_icon_entry",
                        frames.into_iter(),
                    );
                } else if committed && divisions.contains_key(&(side as u8, *unit)) {
                    out.refuse(format!(
                        "Land combat {id} division {unit}: no LandCombatDivisionModifiers row records it ({member_gap}); its modifier icons are omitted."
                    ));
                }
                out.entries[target].push((*unit, widgets));
            }
            let (kind, label) = if committed {
                ("committed", "COMBAT_COMMITED_LABEL")
            } else {
                ("reserves", "COMBAT_RESERVES_LABEL")
            };
            let count = if detail.is_some() || (roles_known && divisions_complete) {
                ids.len().to_string()
            } else {
                "—".into()
            };
            let text = builtin(
                static_content.assets.as_ref(),
                &static_content.localisation,
                label,
                &[("COUNT", Argument::Text(&count))],
            )
            .map_err(|reason| format!("Land combat {prefix} {kind} label is hidden: {reason}."));
            out.bind(
                &format!("{prefix}_window/{prefix}_{kind}_label"),
                TEXT,
                text,
            );
        }
        if let Some(detail) = detail {
            let value = &detail.sides[side];
            let name = if value.commander == 0 {
                builtin(
                    static_content.assets.as_ref(),
                    &static_content.localisation,
                    "UNIT_LEADER_NO_LEADER",
                    &[],
                )
            } else {
                token(
                    static_content.assets.as_ref(),
                    &static_content.localisation,
                    &value.commander_name_token,
                )
            }
            .map_err(|reason| {
                format!("Land combat {id} {prefix} commander name is hidden: {reason}.")
            });
            out.bind(&format!("{prefix}_leader_name"), TEXT, name);
            if value.commander != 0 {
                let skill = builtin(
                    static_content.assets.as_ref(),
                    &static_content.localisation,
                    "UNIT_LEADER_SKILL",
                    &[("SKILL", Argument::Int32(value.commander_skill))],
                )
                .map_err(|reason| {
                    format!("Land combat {id} {prefix} commander skill is hidden: {reason}.")
                });
                out.bind(&format!("{prefix}_leader_skill"), TEXT, skill);
            } else {
                out.bind(&format!("{prefix}_leader_skill"), TEXT, Ok(String::new()));
                let name = "GFX_no_combat_leader";
                let portrait = if static_content.assets.has_sprite(name) {
                    Ok(name.to_owned())
                } else {
                    Err(format!(
                        "Land combat {id} {prefix} commander portrait is hidden: the content lacks sprite {name}."
                    ))
                };
                out.bind(&format!("{prefix}_leader"), SPRITE, portrait);
            }
            match super::hq::unit(frame, value)? {
                Ok(0) => {}
                Ok(_) => show(&mut out.values, &format!("{prefix}_leader_army_hq_icon")),
                Err(reason) => out.withhold(
                    &[&format!("{prefix}_leader_army_hq_icon")],
                    format!("Land combat {id} {prefix} HQ badge is hidden: {reason}."),
                ),
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
        if let Some(marker) = marker {
            let mut kinds = Vec::new();
            if marker.indicator != 0 {
                kinds.push(8);
            }
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
                        match (border_war(modifiers.rows(), id), combat) {
                            (Some(border_war), Some(combat)) => match several_directions(
                                combat,
                                border_war,
                                &divisions,
                                &movement,
                            ) {
                                Some(true) => kinds.push(kind),
                                Some(false) => (),
                                None => out.warnings.push("Land attacker multiple-direction icon: an attacker's LandCombatDivisions location or DivisionMovement source is missing; the icon is omitted rather than guessed.".into()),
                            },
                            (None, _) => out.warnings.push(format!("Land combat {id} multiple-direction icon: no LandCombatModifiers row records whether this is a border war, so the combatant class and its direction test are undecided; the icon is omitted.")),
                            (_, None) => out.warnings.push(format!("Land combat {id} multiple-direction icon: the LandCombats roster is missing; the icon is omitted rather than guessed.")),
                        }
                    }
                } else if marker.flags[side] & mask != 0 {
                    kinds.push(kind);
                }
            }
            let key = format!("{prefix}_window/modifier_icons_container/list");
            icons(
                &mut out.values,
                &key,
                "global_modifier_icon_entry",
                kinds.iter().copied(),
            );
            if kinds.contains(&0) && dlc_mask.is_none() {
                out.refuse(format!(
                    "Land combat {id} {prefix} fort modifier: the SimulationFeatures row is unavailable, so its delayed hover text is omitted."
                ));
            }
            let border = modifier.and_then(|modifier| modifier.border_values);
            let mut refusals = Vec::new();
            if let Some(list) = out.values.get_mut(&key) {
                for (kind, entry) in kinds.iter().copied().zip(&mut list.entries) {
                    let (main, delayed) = global_modifier_tooltip(
                        static_content.assets.as_ref(),
                        &static_content.localisation,
                        kind,
                        dlc_mask,
                        border,
                    );
                    let main = main.unwrap_or_else(|reason| {
                        refusals.push(format!(
                            "Land combat {id} {prefix} modifier frame {kind} hover is omitted: {reason}."
                        ));
                        None
                    });
                    let delayed = delayed.and_then(|delayed| {
                        delayed
                            .map_err(|reason| {
                                refusals.push(format!(
                                    "Land combat {id} {prefix} modifier frame {kind} delayed hover text is omitted: {reason}."
                                ))
                            })
                            .ok()
                    });
                    set_hover(
                        entry,
                        "modifier_icon",
                        main.as_deref().and_then(hover_text),
                        delayed.as_deref().and_then(hover_text),
                    );
                }
            }
            for refusal in refusals {
                out.refuse(refusal);
            }
        } else {
            out.refuse(format!(
                "Land combat {id} {prefix} global modifier icons are omitted: its LandCombatSides row is missing."
            ));
        }
    }
    if let (Some(detail), Some(combat)) = (detail, combat) {
        let reconnaissance = |side: u8, roster: &[u64]| {
            roster.iter().try_fold(0_i64, |best, unit| {
                Some(best.max(divisions.get(&(side, *unit))?.values?.reconnaissance))
            })
        };
        match (
            reconnaissance(0, &combat.attackers),
            reconnaissance(1, &combat.defenders),
            content,
        ) {
            (Some(attacker), Some(defender), Some(content)) => {
                let skills = detail.sides.each_ref().map(|side| side.commander_skill);
                if let Some(side) =
                    initiative_side(skills, [attacker, defender], content.recon_skill_impact)
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
            (_, _, None) => (),
            _ => out.warnings.push("Land initiative glow: a participant's LandCombatDivisions row or its reconnaissance is missing; neither glow is drawn rather than guessed.".into()),
        }
    }
    for ((widget, cause), units) in &undecided {
        out.warnings.push(format!(
            "Land combat {id}: {} committed division {widget}(s) omitted because {cause}; divisions {units:?}.",
            units.len()
        ));
    }
    out.warnings
        .extend(super::division_icons::omission_warnings(id, &omitted));
    if detail.is_none() {
        out.warnings.push("LandCombatDetails unavailable: recorded LandCombatDivisions names, HP/org bars, commitment roles and defence/breakthrough remain bound independently, as do LandCombatModifiers icons. Side attack bonuses, selected commanders, tactics, CAS, maximum width and heading remain unknown; division commanders do not substitute for selected-side commanders.".into());
    }
    Ok(())
}

fn naval(
    out: &mut Bindings,
    static_content: &StaticContent,
    frame: &DecodedFrame,
    id: u64,
    flags: &Flags,
    vision: &Vision,
) -> io::Result<()> {
    hide(&mut out.values, "header_container/disengage");
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    if let Some(detail) = details.rows().iter().find(|r| r.combat == id) {
        out.warnings
            .retain(|s| !s.starts_with("Combat UI: NavalCombatDetails/"));
        let links = rows::<tables::CountryLinks>(frame)?;
        let aliases: BTreeMap<_, _> = links
            .rows()
            .iter()
            .map(|r| (r.id, r.canonical_slot))
            .collect();
        bind_naval_disengage(
            out,
            id,
            naval_disengage_state(detail, &aliases, vision.ours()),
        );
        match naval_progress_skin(detail, &aliases, vision.ours()) {
            Ok(shown) => bind_naval_progress(&mut out.values, shown, detail.progress),
            Err(reason) => out.withhold(
                &[
                    "header_container/progress_attacker",
                    "header_container/progress_defender",
                ],
                format!("Naval combat {id} progress skin is hidden: {reason}."),
            ),
        }
        let combat_ships = rows::<tables::NavalCombatShips>(frame)?;
        let ship_models = rows::<tables::ShipModels>(frame)?;
        let air_bases = rows::<tables::AirBases>(frame)?;
        let live_ships = rows::<tables::Ships>(frame)?;
        let defines = static_content.assets.naval_defines.as_ref();
        let models = ship_models
            .is_complete()
            .then(|| super::keyed::Keyed::new(ship_models.rows(), |model| model.ship))
            .flatten();
        let screening_inputs = if combat_ships.is_complete() {
            defines.zip(models)
        } else {
            None
        };
        let dense = rows::<tables::NavalCombatAirBaseWingHandles>(frame)?;
        let wings = rows::<tables::AirWings>(frame)?;
        let dense = (dense.is_complete() && wings.is_complete())
            .then(|| super::naval_air_groups::DenseBases::new(dense.rows(), wings.rows()))
            .flatten();
        let mut admitted =
            |family| match replay_derive::fixed_layout::validate_recording(frame, family) {
                Ok(()) => true,
                Err(reason) => {
                    out.warnings.push(reason);
                    false
                }
            };
        let positioning_admitted = admitted(replay_derive::fixed_layout::NAVAL_POSITIONING);
        let anti_air_admitted = admitted(replay_derive::fixed_layout::NAVAL_ANTI_AIR);
        let positioning = if positioning_admitted {
            super::naval_positioning::positioning_values(
                detail,
                combat_ships.is_complete().then_some(combat_ships.rows()),
                models,
                air_bases.is_complete().then_some(air_bases.rows()),
                live_ships.is_complete().then_some(live_ships.rows()),
                dense,
                &static_content.assets.naval_air_groups,
                defines,
            )
        } else {
            [None; 2]
        };
        for (side, value) in detail.sides.iter().enumerate() {
            let label = if side == 0 { "attacker" } else { "defender" };
            let wnd = if side == 0 { "left" } else { "right" };
            let screening = match &screening_inputs {
                Some((defines, models)) => super::naval_screening::calculate(
                    id,
                    side as u8,
                    value,
                    combat_ships.rows(),
                    models,
                    defines,
                )
                .map_err(|reason| {
                    format!("Naval combat {id} {label} screening is hidden: {reason}.")
                }),
                None => Err(format!(
                    "Naval combat {id} {label} screening is hidden: complete combat ships, ship models and naval defines are required."
                )),
            };
            let screening_keys =
                ["capital", "carrier"].map(|key| format!("{wnd}_wnd_{key}_screening_efficiency"));
            match screening {
                Ok(screening) => {
                    for (key, raw) in screening_keys
                        .iter()
                        .zip([screening.capital, screening.carrier])
                    {
                        text(
                            &mut out.values,
                            key,
                            content_reader_parser::fixed5::fixed5_percent(raw, 0, true),
                        );
                    }
                }
                Err(refusal) => out.withhold(
                    &[screening_keys[0].as_str(), screening_keys[1].as_str()],
                    refusal,
                ),
            }
            out.bind(
                &format!("header_container/{wnd}_wnd/stat_positioning"),
                TEXT,
                positioning[side]
                    .map(|raw| content_reader_parser::fixed5::fixed5_percent(raw, 0, true))
                    .ok_or_else(|| {
                        format!(
                            "Naval combat {id} {label} positioning is hidden: the positioning modifier keys are refused or a required combat, ship or carrier-air operand is unavailable."
                        )
                    }),
            );
            let anti_air = if let (true, Some(fresh), Some((defines, models))) =
                (anti_air_admitted, positioning[side], &screening_inputs)
            {
                super::naval_anti_air::calculate(
                    id,
                    side as u8,
                    fresh,
                    combat_ships.rows(),
                    defines.summary_fixed[13],
                    |ship| {
                        models
                            .get(ship)
                            .and_then(|row| row.counter.as_ref())
                            .map(|counter| counter.anti_air_base)
                    },
                )
                .map(|raw| content_reader_parser::fixed5::fixed5_to_string(raw, 1))
                .map_err(|reason| {
                    format!("Naval combat {id} {label} anti-air is hidden: {reason}.")
                })
            } else {
                Err(format!(
                    "Naval combat {id} {label} anti-air is hidden: admitted anti-air modifier keys, positioning, complete combat ships and ship models and naval defines are required."
                ))
            };
            out.bind(
                &format!("header_container/{wnd}_wnd/stat_anti_air"),
                TEXT,
                anti_air,
            );
            let flag = naval_flag_country(&value.summary.countries, &aliases, vision.ours())
                .and_then(|country| country_flag(flags, country))
                .map_err(|reason| format!("Naval combat {id} {label} flag is hidden: {reason}."));
            out.bind(&format!("header_container/flag_{label}"), FLAG, flag);
            let name_key = format!("header_container/leader_{label}_name");
            if value.commander == 0 {
                let name = builtin(
                    static_content.assets.as_ref(),
                    &static_content.localisation,
                    "UNIT_LEADER_NO_LEADER",
                    &[],
                )
                .map_err(|reason| {
                    format!("Naval combat {id} {label} commander name is hidden: {reason}.")
                });
                out.bind(&name_key, TEXT, name);
                let sprite = "GFX_leader_unknown";
                let portrait = if static_content.assets.has_sprite(sprite) {
                    Ok(sprite.to_owned())
                } else {
                    Err(format!(
                        "Naval combat {id} {label} commander portrait is hidden: the content lacks sprite {sprite}."
                    ))
                };
                out.bind(
                    &format!("header_container/leader_{label}"),
                    SPRITE,
                    portrait,
                );
            } else if let Some(commander) = &value.commander_inputs {
                show(&mut out.values, &format!("header_container/{label}_stats"));
                let name = std::str::from_utf8(&commander.name)
                    .map_err(|_| "it is not UTF-8".to_owned())
                    .and_then(|name| displayable(static_content.assets.as_ref(), name.to_owned()))
                    .map_err(|reason| {
                        format!("Naval combat {id} {label} commander name is hidden: {reason}.")
                    });
                out.bind(&name_key, TEXT, name);
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
            } else {
                out.withhold(
                    &[&name_key],
                    format!(
                        "Naval combat {id} {label} commander name and skills are hidden: NavalCombatDetails holds no inputs for commander {}.",
                        value.commander
                    ),
                );
            }
        }
        out.warnings.push("Naval: the perspective progress skin, recomputed screening, positioning and AA, ordered side flags and commander name/skills are bound. Forward active air groups and their carrier-grid entries are bound from recorded variants and the stacked equipment registry. Hover capture gaps are reported as counted refusals beside their affected widgets. Phase has no separate scalar widget in navalcombat.gui; retreat/engagement operands are not relabelled as a guessed phase.".into());
    }
    Ok(())
}

fn naval_flag_country(
    countries: &[u32],
    aliases: &BTreeMap<u32, u32>,
    viewer: u32,
) -> Result<u32, String> {
    let mut unresolved = None;
    for &country in countries {
        match aliases.get(&country) {
            Some(&slot) if slot == viewer && viewer != 0 => {
                return unresolved.map_or(Ok(country), Err);
            }
            Some(_) => (),
            None if country != 0 => {
                unresolved.get_or_insert_with(|| {
                    format!("primary country {country} has no CountryLinks row")
                });
            }
            None => (),
        }
    }
    if let Some(reason) = unresolved {
        return Err(reason);
    }
    countries
        .first()
        .copied()
        .ok_or_else(|| "NavalCombatDetails lists no country for the side".to_owned())
}

fn naval_disengage_state(
    detail: &replay_data::rows::NavalCombatDetailsRow,
    aliases: &BTreeMap<u32, u32>,
    viewing_slot: u32,
) -> Result<Option<bool>, String> {
    let [attacker, defender] = [0, 1].map(|side| {
        super::tactics::viewing_side(
            &detail.sides[side].summary.countries,
            ["attacker", "defender"][side],
            aliases,
            viewing_slot,
        )
        .map(|relative| relative == 0)
    });
    let enabled = detail.sides.each_ref().map(|side| side.retreating == 0);
    match (attacker, defender) {
        (Ok(true), _) => Ok(Some(enabled[0])),
        (Ok(false), Ok(true)) => Ok(Some(enabled[1])),
        (Err(_), Ok(true)) if enabled[0] == enabled[1] => Ok(Some(enabled[1])),
        (Ok(false), Ok(false)) => Ok(None),
        (Err(reason), _) | (_, Err(reason)) => Err(reason),
    }
}

fn bind_naval_disengage(out: &mut Bindings, id: u64, state: Result<Option<bool>, String>) {
    let key = "header_container/disengage";
    hide(&mut out.values, key);
    out.values.get_mut(key).unwrap().enabled = None;
    match state {
        Ok(Some(enabled)) => {
            show(&mut out.values, key);
            out.values.get_mut(key).unwrap().enabled = Some(enabled);
        }
        Ok(None) => (),
        Err(reason) => out.refuse(format!(
            "Naval combat {id} Disengage button is hidden: {reason}."
        )),
    }
}

fn naval_progress_skin(
    detail: &replay_data::rows::NavalCombatDetailsRow,
    aliases: &BTreeMap<u32, u32>,
    viewer: u32,
) -> Result<&'static str, String> {
    super::tactics::viewing_side(
        &detail.sides[1].summary.countries,
        "defender",
        aliases,
        viewer,
    )
    .map(|side| {
        if side == 0 {
            "progress_defender"
        } else {
            "progress_attacker"
        }
    })
}

fn bind_naval_progress(values: &mut Values, shown: &str, progress: i64) {
    let hidden = if shown == "progress_attacker" {
        "progress_defender"
    } else {
        "progress_attacker"
    };
    hide(values, &format!("header_container/{hidden}"));
    let key = format!("header_container/{shown}");
    show(values, &key);
    fill(values, &key, progress as f32 / 100_000.0);
    values.entry(key).or_default().progress = Some(super::fixed::round(progress.wrapping_mul(100)));
}

tooltip_keys! {
    BORDER_CONFLICT_MODIFIER = "BORDER_CONFLICT_MODIFIER",
    BORDER_CONFLICT_TERRAIN_FACTOR = "BORDER_CONFLICT_TERRAIN_FACTOR",
    BORDER_CONFLICT_DIG_IN_FACTOR = "BORDER_CONFLICT_DIG_IN_FACTOR",
    BORDER_CONFLICT_FACTOR_ENTRY = "BORDER_CONFLICT_FACTOR_ENTRY",
    BM_FORT_MODIFIER = "BM_FORT_MODIFIER",
    BM_FORT_MODIFIER_DESC_WITH_RAILWAY_GUN = "BM_FORT_MODIFIER_DESC_WITH_RAILWAY_GUN",
    BM_FORT_MODIFIER_DESC = "BM_FORT_MODIFIER_DESC",
    BM_ENEMY_AIR_SUPERIORITY = "BM_ENEMY_AIR_SUPERIORITY",
    BM_ENEMY_AIR_SUPERIORITY_DESC = "BM_ENEMY_AIR_SUPERIORITY_DESC",
    BM_AIR_SUPPORT = "BM_AIR_SUPPORT",
    BM_AIR_SUPPORT_DESC = "BM_AIR_SUPPORT_DESC",
    BM_SHORE_BOMBARD = "BM_SHORE_BOMBARD",
    BM_SHORE_BOMBARD_DESC = "BM_SHORE_BOMBARD_DESC",
    BM_RAILWAY_GUN_BOMBARD = "BM_RAILWAY_GUN_BOMBARD",
    BM_RAILWAY_GUN_BOMBARD_DESC = "BM_RAILWAY_GUN_BOMBARD_DESC",
    BM_STACKING = "BM_STACKING",
    BM_STACKING_DESC = "BM_STACKING_DESC",
    BM_MULTIPLE_DIRECTIONS = "BM_MULTIPLE_DIRECTIONS",
    BM_MULTIPLE_DIRECTIONS_DESC = "BM_MULTIPLE_DIRECTIONS_DESC",
    BM_DECRYPTION = "BM_DECRYPTION",
    BM_DECRYPTION_DESC = "BM_DECRYPTION_DESC",
    BM_ACTIVE_CODE_CRACKING_BONUSES = "BM_ACTIVE_CODE_CRACKING_BONUSES",
    BM_ACTIVE_CODE_CRACKING_BONUSES_DESC = "BM_ACTIVE_CODE_CRACKING_BONUSES_DESC",
}

#[cfg(test)]
mod tests;
