//! Combat window operands from recorded simulation state and matching content.
use super::{
    INLINE_FLAG, Selection, Values, Widget,
    binding::{
        Bindings, FILL, FLAG, SPRITE, TEXT, TextIcons, builtin, country_flag, displayable, fill,
        hide, text, token,
    },
    land_details::{Condition, conditions},
    template_icons,
    tooltips::{hover_text, set_hover},
};
use crate::{
    document::Document,
    map::tables::{Available, rows},
};
use installed_content::{
    division_names::{Database, Unresolved},
    localisation::{Argument, Localisation},
};
use replay::{
    frame::DecodedFrame,
    rows::{LandCombatDivisionRow, TableSchema, tables},
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

/// The land window facts decided once per bind, here and nowhere else: the
/// terrain picture (1415C7520) and each side's flag (1415C9860). Each binds
/// through [`Bindings::bind`], so a refusal hides the widget.
fn pictures(
    out: &mut Bindings,
    frame: &DecodedFrame,
    id: u64,
    terrain: &BTreeMap<u32, String>,
    has_sprite: &dyn Fn(&str) -> bool,
    flags: &BTreeMap<u32, String>,
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
        // 1415C9860 hands the country 1415C5F10 returns to the flag widget
        // (country_icon_presentation_functions::render 140B35990): the
        // selected commander's country (+0x120), else the first entry of the
        // side's primary country set (+0x38, count +0x48), else the attacked
        // province's controller (+0x188). The recorder keeps that country as
        // `commander_country` with or without a commander.
        let flag = match detail {
            Some(detail) => country_flag(flags, detail.sides[side].commander_country),
            None => Err(format!(
                "LandCombatDetails has no row for the combat ({}), so the side country 1415C5F10 selects (the commander's, else the first primary country, else the province controller) is unknown",
                absence(&details)
            )),
        }
        .map_err(|reason| format!("Land combat {id} {prefix} flag is hidden: {reason}."));
        out.bind(&format!("{prefix}_flag"), FLAG, flag);
    }
    Ok(())
}

/// 1415C7520's terrain picture: `GFX_terrain_<category>` of the combat
/// province, with `_winter` when that province's weather snow
/// (SWeatherPerProvince+0x40 through 140F04D50, recorded as
/// `WeatherProvinces.snow`) is positive. Missing weather is not zero snow:
/// without the province's row summer and winter are undecided. Natively a
/// missing sprite is logged ("Missing spriteType") and the widget keeps
/// whatever it showed before; the viewer has no such earlier native state,
/// so it refuses by name.
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
        refused(format!("the content lacks sprite {name} (1415C7520)"))
    }
}
/// Why a table this hour holds no row for a key, from the table's state.
fn absence<T: TableSchema>(table: &Available<'_, T>) -> String {
    match table {
        Available::Complete(_) => "the complete table lacks it".to_owned(),
        Available::Truncated { missing, .. } => {
            format!("the table is truncated, {missing} rows unread")
        }
        Available::Unavailable { reason } => format!("the table is unavailable: {reason}"),
    }
}
// Native signed qword multiplication precedes the fixed5 division.
fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / 100_000
}
fn fraction(value: i64, maximum: i64) -> f32 {
    ((value as f64 / maximum.max(100) as f64).clamp(0.0, 1.0)) as f32
}
/// Why a division's name cannot be rendered.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Unnamed {
    CustomNotText,
    /// The name holds a control the painter does not draw ([`displayable`]).
    Control(String),
    /// CArmy's constructor 140C5DD30 builds its member with 1409B3070 (type
    /// 0), and SetTemplate/SelectFakeIntelDivisionTemplate bind it to a
    /// template. ResolveNameGroup 1409B7290's other types (1 ship/variant, 2
    /// operative, 3 railway gun, 4 the other constructors' unassigned
    /// default) have no content form for a division; only a save whose
    /// `division_name` type was edited (read verbatim by 1409BB290) yields one.
    ForeignType(i32),
    Unresolved(Unresolved),
}

impl std::fmt::Display for Unnamed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CustomNotText => f.write_str("the custom name is not UTF-8"),
            Self::Control(reason) => write!(f, "the name holds {reason}"),
            Self::ForeignType(kind) => write!(
                f,
                "the name type is {kind}, a ship, operative or railway-gun naming form no division content produces"
            ),
            Self::Unresolved(reason) => write!(f, "{reason}"),
        }
    }
}

/// 1409B5640: the stored name where the game shows it, which is exactly
/// where the capture records one (with +0xA9 clear, or set and a null
/// group), else the selected historical name of a type-zero member from the
/// content's name groups.
fn division_name(row: &LandCombatDivisionRow, names: &Database) -> Result<String, Unnamed> {
    if !row.name_custom.is_empty() {
        return std::str::from_utf8(&row.name_custom)
            .map(str::to_owned)
            .map_err(|_| Unnamed::CustomNotText);
    }
    if row.name_type != 0 {
        return Err(Unnamed::ForeignType(row.name_type));
    }
    names
        .resolve(&row.name_group, row.name_order, row.name_ordered != 0)
        .map_err(Unnamed::Unresolved)
}

/// [`division_name`] as the entry draws it: the text path (1422894D0) reads
/// its control bytes, so one the painter does not draw is refused.
fn shown_name(
    row: &LandCombatDivisionRow,
    names: &Database,
    icons: &impl TextIcons,
) -> Result<String, Unnamed> {
    division_name(row, names).and_then(|name| displayable(icons, name).map_err(Unnamed::Control))
}

/// One warning per cause for a combat's divisions whose names are
/// unavailable, naming each division with its group and ordinal.
fn unnamed_warnings<'a>(
    combat: u64,
    rows: impl Iterator<Item = &'a LandCombatDivisionRow>,
    names: &Database,
    icons: &impl TextIcons,
) -> Vec<String> {
    let mut causes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        if let Err(cause) = shown_name(row, names, icons) {
            causes.entry(cause.to_string()).or_default().push(format!(
                "division {} (group {}, {} {})",
                row.division,
                row.name_group.escape_ascii(),
                if row.name_ordered != 0 {
                    "ordered"
                } else {
                    "unordered"
                },
                row.name_order
            ));
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

/// CCombatantEntry (1415BA240) names a committed division "name 0x12TAG" for
/// the attacker and "0x12TAG name" for the defender; the reserve entry
/// (1415BAC10) uses the bare name.
fn entry_name(name: String, side: u8, committed: bool, flag: Option<&String>) -> String {
    match flag.filter(|_| committed) {
        Some(flag) if side == 0 => format!("{name} {INLINE_FLAG}{flag}"),
        Some(flag) => format!("{INLINE_FLAG}{flag} {name}"),
        None => name,
    }
}

/// An entry's strength and organisation bars; without the division's
/// condition both are hidden and the refusal counted. A bar whose maximum is
/// unknown is hidden alone; `status_gap` says why `DivisionStatus` lacks it.
fn condition_bars(
    out: &mut Bindings,
    values: &mut Values,
    row: &LandCombatDivisionRow,
    committed: bool,
    bars: Result<Condition, String>,
    status_gap: &str,
) {
    let bars = match bars {
        Ok(bars) => bars,
        Err(refusal) => return out.withhold_in(values, &["str_bar", "org_bar"], refusal),
    };
    let unbounded = |bar: &str, maximum: &str| {
        format!(
            "Land combat {} division {}: its DivisionStatus row is missing ({status_gap}), so its {maximum} maximum is unknown; its {bar} bar is hidden.",
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
        strength.map(|strength| {
            if committed {
                (strength * row.strength_combat_factor as f32 / 100_000.0).clamp(0.0, 1.0)
            } else {
                strength
            }
        }),
    );
    let organisation = bars
        .max_organisation
        .map(|maximum| fraction(bars.organisation, maximum))
        .ok_or_else(|| unbounded("organisation", "organisation"));
    out.bind_in(values, "org_bar", FILL, organisation);
}

fn entry(
    row: &LandCombatDivisionRow,
    name: Option<String>,
    committed: bool,
    bonus: Option<i64>,
    flag: Option<&String>,
    mut values: Values,
) -> Values {
    // An unavailable name is cleared and hidden; `unnamed_warnings` reports
    // it once per cause with every division it covers.
    let name_widget = values.entry("name".into()).or_default();
    name_widget.hidden = name.is_none();
    name_widget.text = name.map(|name| entry_name(name, row.side, committed, flag));
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
                // 1415C7E00 sets each stat box through Fixed5ToString
                // (1424B6200) at precision 0, so the whole part is
                // truncated: Reference07 / pairJ_land_5, 369.58320 * .99000,
                // shows 365, not 366.
                text(
                    &mut values,
                    key,
                    installed_content::fixed5::fixed5_to_string(raw, 0),
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

/// A global modifier hover: its text (`None` for a frame without one) and
/// delayed text, each or why it is refused.
type ModifierHover = (
    Result<Option<String>, String>,
    Option<Result<String, String>>,
);

/// 1415C0AE0 ignores the hovered child and dispatches by the global modifier
/// strip's frame. Static descriptions come from the localisation; border
/// conflict factors are the side's recorded simulation operands.
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
                "BORDER_CONFLICT_MODIFIER",
                &[("VAL", Argument::Fixed5(modifier))],
            ) {
                Ok(line) => main.push_str(&line),
                Err(reason) => return (Err(reason), None),
            }
        }
        for (value, key) in [
            (terrain, "BORDER_CONFLICT_TERRAIN_FACTOR"),
            (dig_in, "BORDER_CONFLICT_DIG_IN_FACTOR"),
        ] {
            if value == 100_000 {
                continue;
            }
            let entry = match builtin(
                icons,
                loc,
                "BORDER_CONFLICT_FACTOR_ENTRY",
                &[("VAL", Argument::Fixed5(value))],
            ) {
                Ok(entry) => entry,
                Err(reason) => return (Err(reason), None),
            };
            // 1415C0AE0 wraps the entry in the control byte 0x11 and a colour
            // with string appends, outside the localiser's buffer.
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
            "BM_FORT_MODIFIER",
            dlc_mask.map(|mask| {
                // 1401AE640's feature ID 0x23 maps to the saved DLC mask's
                // bit 7, not to a bit numbered 0x23.
                if mask & (1 << 7) != 0 {
                    "BM_FORT_MODIFIER_DESC_WITH_RAILWAY_GUN"
                } else {
                    "BM_FORT_MODIFIER_DESC"
                }
            }),
        ),
        1 => (
            "BM_ENEMY_AIR_SUPERIORITY",
            Some("BM_ENEMY_AIR_SUPERIORITY_DESC"),
        ),
        2 => ("BM_AIR_SUPPORT", Some("BM_AIR_SUPPORT_DESC")),
        3 => ("BM_SHORE_BOMBARD", Some("BM_SHORE_BOMBARD_DESC")),
        4 => (
            "BM_RAILWAY_GUN_BOMBARD",
            Some("BM_RAILWAY_GUN_BOMBARD_DESC"),
        ),
        5 => ("BM_STACKING", Some("BM_STACKING_DESC")),
        6 => (
            "BM_MULTIPLE_DIRECTIONS",
            Some("BM_MULTIPLE_DIRECTIONS_DESC"),
        ),
        7 => ("BM_DECRYPTION", Some("BM_DECRYPTION_DESC")),
        9 => (
            "BM_ACTIVE_CODE_CRACKING_BONUSES",
            Some("BM_ACTIVE_CODE_CRACKING_BONUSES_DESC"),
        ),
        _ => return (Ok(None), None),
    };
    // The BM_ keys are the executable's: each must be in the localisation,
    // and a missing or unsupported one is refused by name, never shown as
    // its own text.
    let main = builtin(icons, loc, key, &[])
        .and_then(|name| builtin(icons, loc, "HEADER", &[("KEY", Argument::Text(&name))]))
        .map(Some);
    let delayed = description.map(|key| builtin(icons, loc, key, &[]));
    (main, delayed)
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
/// (CCombatant+0x20, the full LandCombats roster in order) have different
/// direction keys. CLandBorderWarCombatant's slot (14011CD10) always returns
/// false.
fn several_directions(
    combat: &replay::rows::LandCombatRow,
    border_war: bool,
    divisions: &BTreeMap<(u8, u64), &LandCombatDivisionRow>,
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

/// Whether the combat is a CLandBorderWarCombat, whose type token 0x39A5
/// (1413D8470) selects both the window title (1415C7520) and the combatant
/// class above. Only a border war records border values on its modifier
/// rows, so without any row for the combat the class stays undecided.
pub(super) fn border_war(rows: &[replay::rows::LandCombatModifierRow], id: u64) -> Option<bool> {
    rows.iter()
        .find(|row| row.combat == id)
        .map(|row| row.border_values.is_some())
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
    let marker = rows::<tables::LandCombatSides>(frame)?;
    let marker = marker.rows().iter().find(|row| row.combat == id);
    let features = rows::<tables::SimulationFeatures>(frame)?;
    let dlc_mask = match features.rows() {
        [feature] if features.is_complete() => Some(feature.dlc_mask),
        _ => None,
    };
    let bars = conditions(frame)?;
    let division_gap = absence(&rows::<tables::Divisions>(frame)?);
    let status_gap = absence(&rows::<tables::DivisionStatus>(frame)?);
    let exiles = template_icons::Inputs::read(frame)?;
    let content = document.icons.combat.land_defines.as_ref();
    if let Err(reason) = content {
        out.warnings.push(format!("Land combat defines are unavailable ({reason}); derived division icons and the initiative glow are omitted."));
    }
    let content = content.ok();
    let names = &document.icons.combat.division_names;
    out.warnings.extend(unnamed_warnings(
        id,
        divisions.values().copied(),
        names,
        &document.icons.combat,
    ));
    // Undecided entry flags and exile badges by widget and cause, with the
    // committed divisions each cause leaves undecided: one warning per cause.
    let mut undecided: BTreeMap<(&str, String), Vec<u64>> = BTreeMap::new();
    let movement: BTreeMap<_, _> = rows::<tables::DivisionMovement>(frame)?
        .rows()
        .iter()
        .map(|row| (row.division, row.source))
        .collect();
    let has_sprite = |name: &str| document.icons.combat.has_sprite(name);
    pictures(
        out,
        frame,
        id,
        &document.counters.combat_terrain,
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
    // Committed entries whose icon list ended early, per cause.
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
                    let flag = if committed {
                        let cause = match exiles.entry_owner(row) {
                            Ok(owner) => match flags.get(&owner) {
                                Some(flag) => Ok(flag),
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
                        shown_name(row, names, &document.icons.combat).ok(),
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
                    condition_bars(out, &mut widgets, row, committed, condition, &status_gap);
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
                    let exile = match (detail, content) {
                        (Some(detail), Some(content)) => {
                            let [occupier, core] =
                                exiles.frames(row, detail, &content.exile_against_occupier);
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
                        row,
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
                    out.refuse(match modifier {
                        None => format!(
                            "Land combat {id} side {side} division modifier icons are omitted: no LandCombatModifiers row records the side."
                        ),
                        Some(_) => format!(
                            "Land combat {id} division {unit}: its side's LandCombatModifiers row does not list it; its modifier icons are omitted."
                        ),
                    });
                }
                out.entries[target].push((*unit, widgets));
            }
            let (kind, label) = if committed {
                ("committed", "COMBAT_COMMITED_LABEL")
            } else {
                ("reserves", "COMBAT_RESERVES_LABEL")
            };
            // 1415C9860: COUNT is the side's committed (+0xD4) or reserve
            // (+0xEC) count as a string argument (Int32ToString 1424B56A0,
            // LocalizeWithMovedStringArgument 1402E30F0). An unknown count
            // takes the viewer's unknown dash.
            let count = if detail.is_some() || (roles_known && divisions_complete) {
                ids.len().to_string()
            } else {
                "—".into()
            };
            let text = builtin(
                &document.icons.combat,
                &document.names.localisation,
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
            // 1415C5F10: a commander's name is its localised name token
            // (140C16FB0 into 142230F20); no commander is UNIT_LEADER_NO_LEADER.
            let name = if value.commander == 0 {
                builtin(
                    &document.icons.combat,
                    &document.names.localisation,
                    "UNIT_LEADER_NO_LEADER",
                    &[],
                )
            } else {
                token(
                    &document.icons.combat,
                    &document.names.localisation,
                    &value.commander_name_token,
                )
            }
            .map_err(|reason| {
                format!("Land combat {id} {prefix} commander name is hidden: {reason}.")
            });
            out.bind(&format!("{prefix}_leader_name"), TEXT, name);
            // The side flag is bound in `pictures`, with or without a
            // commander.
            if value.commander != 0 {
                // 1415C5F10: UNIT_LEADER_SKILL with the integer argument SKILL
                // (Localisation_FormatValue 1402E2E60, type 5).
                let skill = builtin(
                    &document.icons.combat,
                    &document.names.localisation,
                    "UNIT_LEADER_SKILL",
                    &[("SKILL", Argument::Int32(value.commander_skill))],
                )
                .map_err(|reason| {
                    format!("Land combat {id} {prefix} commander skill is hidden: {reason}.")
                });
                out.bind(&format!("{prefix}_leader_skill"), TEXT, skill);
            } else {
                // 1415C5F10 writes an empty skill beside UNIT_LEADER_NO_LEADER
                // and sets the sprite it names on the portrait button.
                out.bind(&format!("{prefix}_leader_skill"), TEXT, Ok(String::new()));
                let name = "GFX_no_combat_leader";
                let portrait = if document.icons.combat.has_sprite(name) {
                    Ok(name.to_owned())
                } else {
                    Err(format!(
                        "Land combat {id} {prefix} commander portrait is hidden: the content lacks sprite {name} (1415C5F10)."
                    ))
                };
                out.bind(&format!("{prefix}_leader"), SPRITE, portrait);
            }
            // The badge is hidden in `Bindings::land` and shown only here.
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
                        &document.icons.combat,
                        &document.names.localisation,
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
                Some(best.max(divisions.get(&(side, *unit))?.reconnaissance))
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
            _ => out.warnings.push("Land initiative glow: a participant's LandCombatDivisions reconnaissance is missing; neither glow is drawn rather than guessed.".into()),
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
    out.warnings.push("Land window: names, bars, roles and modifier kinds bind without the parent-detail table. Defence/breakthrough uses recorded scales and truncates to whole units, as observed in reference07. Side-dependent attack, selected commander skill, CAS, the committed/reserve array order and the derived division icons require LandCombatDetails. Reinforcement and modifier tooltips, learning traits and possible-tactics menus remain unported; unsupported widgets stay empty/dash.".into());
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
        // The window recomputes positioning, screening and anti-air from live operands on
        // every refresh; the combat's stored copies lag them.
        let combat_ships = rows::<tables::NavalCombatShips>(frame)?;
        let ship_models = rows::<tables::ShipModels>(frame)?;
        let air_bases = rows::<tables::AirBases>(frame)?;
        let live_ships = rows::<tables::Ships>(frame)?;
        let defines = document.icons.combat.naval_defines.as_ref();
        // One ShipModels lookup per refresh serves surface detection, carrier
        // air, screening and anti-air.
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
        // The recomputations read fixed modifier keys from the recorded
        // accumulators; a recording whose definitions spell them differently
        // is refused by name, and the affected values are omitted.
        let mut admitted = |family| match replay::fixed_layout::validate_recording(frame, family) {
            Ok(()) => true,
            Err(reason) => {
                out.warnings.push(reason);
                false
            }
        };
        let positioning_admitted = admitted(replay::fixed_layout::NAVAL_POSITIONING);
        let anti_air_admitted = admitted(replay::fixed_layout::NAVAL_ANTI_AIR);
        let positioning = if positioning_admitted {
            super::naval_positioning::positioning_values(
                detail,
                combat_ships.is_complete().then_some(combat_ships.rows()),
                models,
                air_bases.is_complete().then_some(air_bases.rows()),
                live_ships.is_complete().then_some(live_ships.rows()),
                dense,
                &document.icons.combat.naval_air_groups,
                defines,
            )
        } else {
            [None; 2]
        };
        for (side, value) in detail.sides.iter().enumerate() {
            let label = if side == 0 { "attacker" } else { "defender" };
            let wnd = if side == 0 { "left" } else { "right" };
            // 1417D9C60: FormatFixed5Percent at precision 0 times 100
            // (1417DA098, 1417DA0C1), of the screening the window recomputes.
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
                            installed_content::fixed5::fixed5_percent(raw, 0, true),
                        );
                    }
                }
                Err(refusal) => out.withhold(
                    &[screening_keys[0].as_str(), screening_keys[1].as_str()],
                    refusal,
                ),
            }
            // 1417D9C60: positioning as the screening (1417DA0F8), anti-air
            // through Fixed5ToString at precision 1 (1417DA133), truncating.
            out.bind(
                &format!("header_container/{wnd}_wnd/stat_positioning"),
                TEXT,
                positioning[side]
                    .map(|raw| installed_content::fixed5::fixed5_percent(raw, 0, true))
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
                .map(|raw| installed_content::fixed5::fixed5_to_string(raw, 1))
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
            let flag = value
                .summary
                .countries
                .first()
                .ok_or_else(|| "NavalCombatDetails lists no country for the side".to_owned())
                .and_then(|country| country_flag(flags, *country))
                .map_err(|reason| format!("Naval combat {id} {label} flag is hidden: {reason}."));
            out.bind(&format!("header_container/flag_{label}"), FLAG, flag);
            let name_key = format!("header_container/leader_{label}_name");
            if value.commander == 0 {
                // 1417DCE40 (from RefreshCombatDetails 1417DA2F0).
                let name = builtin(
                    &document.icons.combat,
                    &document.names.localisation,
                    "UNIT_LEADER_NO_LEADER",
                    &[],
                )
                .map_err(|reason| {
                    format!("Naval combat {id} {label} commander name is hidden: {reason}.")
                });
                out.bind(&name_key, TEXT, name);
                // portrait button; land's GFX_no_combat_leader is not used.
                let sprite = "GFX_leader_unknown";
                let portrait = if document.icons.combat.has_sprite(sprite) {
                    Ok(sprite.to_owned())
                } else {
                    Err(format!(
                        "Naval combat {id} {label} commander portrait is hidden: the content lacks sprite {sprite} (1417DCE40)."
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
                    .and_then(|name| displayable(&document.icons.combat, name.to_owned()))
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
                // The skills box stays hidden from `Bindings::naval`.
                out.withhold(
                    &[&name_key],
                    format!(
                        "Naval combat {id} {label} commander name and skills are hidden: NavalCombatDetails holds no inputs for commander {}.",
                        value.commander
                    ),
                );
            }
        }
        out.warnings.push("Naval: progress, recomputed screening, positioning and AA, ordered side flags and commander name/skills are bound. Air groups, perspective-dependent progress skin and tooltips remain unported; the environment binder supplies recorded weather and weapon summaries. Phase has no separate scalar widget in navalcombat.gui; retreat/engagement operands are not relabelled as a guessed phase.".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::binding::NoIcons;

    /// The BM_ keys are the executable's: a missing one is refused, never
    /// shown as its own text, and the delayed key is named. Border factors
    /// wrap their entry in the stored colour byte.
    #[test]
    fn executable_modifier_keys_must_exist() {
        let loc: Localisation = [
            ("HEADER", "$KEY$"),
            ("BM_STACKING", "Stacking"),
            ("BORDER_CONFLICT_FACTOR_ENTRY", "$VAL|%0$"),
            ("BORDER_CONFLICT_TERRAIN_FACTOR", "Terrain $VAL$"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let (main, delayed) = global_modifier_tooltip(&NoIcons, &loc, 5, None, None);
        assert_eq!(main, Ok(Some("Stacking".to_owned())));
        // A missing top-level key shows its own bytes (142230940).
        assert_eq!(delayed, Some(Ok("BM_STACKING_DESC".to_owned())));
        let (main, _) = global_modifier_tooltip(&NoIcons, &loc, 7, None, None);
        assert_eq!(main, Ok(Some("BM_DECRYPTION".to_owned())));
        let (main, delayed) =
            global_modifier_tooltip(&NoIcons, &loc, 8, None, Some([0, 100_000, 80_000]));
        assert_eq!(main, Ok(Some("Terrain \u{11}R80%\u{11}!".to_owned())));
        assert_eq!(delayed, None);
        let (main, _) = global_modifier_tooltip(&NoIcons, &loc, 8, None, None);
        assert!(main.unwrap_err().contains("border-war factors"));
    }
    const HALF: Condition = Condition {
        strength: 50_000,
        max_strength: Some(100_000),
        organisation: 30_000,
        max_organisation: Some(60_000),
    };

    #[test]
    fn committed_strength_and_side_attack_use_distinct_native_operands() {
        let row = LandCombatDivisionRow {
            strength_combat_factor: 50_000,
            scales: [200_000, 50_000, 150_000, 100_000],
            stat_values: [8_000_000, 6_000_000, 0, 10_000_000, 2_000_000],
            ..Default::default()
        };
        let entry = |committed| {
            let mut values = entry(&row, None, committed, Some(10_000), None, Values::new());
            condition_bars(
                &mut Bindings::default(),
                &mut values,
                &row,
                committed,
                Ok(HALF),
                "",
            );
            values
        };
        let active = entry(true);
        let reserve = entry(false);
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
            strength_combat_factor: 100_000,
            scales: [99_000, 99_000, 100_000, 100_000],
            stat_values: [36_958_320, 7_635_582, 833, 23_127_045, 5_736_750],
            ..Default::default()
        };
        let mut out = Bindings::default();
        let mut active = entry(
            &row,
            Some("169th 'Madison' Division".into()),
            true,
            None,
            None,
            Values::new(),
        );
        condition_bars(&mut out, &mut active, &row, true, Ok(HALF), "");
        assert_eq!(
            active["name"].text.as_deref(),
            Some("169th 'Madison' Division")
        );
        assert_eq!(active["str_bar"].fill, Some(0.5));
        assert_eq!(active["org_bar"].fill, Some(0.5));
        assert_eq!(active["defense"].text.as_deref(), Some("365"));
        assert_eq!(active["soft_attack"].text, None);
        assert_eq!(active["hard_attack"].text, None);
        // The same entry bound again without its name or condition keeps
        // neither earlier value: both are cleared and hidden, the bars'
        // refusal counted once per bind.
        let mut stale = entry(&row, None, true, None, None, active);
        for _ in 0..2 {
            condition_bars(
                &mut out,
                &mut stale,
                &row,
                true,
                Err("Land combat 5 division 9: bars refused.".into()),
                "",
            );
        }
        assert!(stale["name"].hidden && stale["name"].text.is_none());
        for bar in ["str_bar", "org_bar"] {
            assert!(stale[bar].hidden && stale[bar].fill.is_none(), "{bar}");
        }
        assert_eq!(
            out.refused()
                .get("Land combat 5 division 9: bars refused.")
                .copied(),
            Some(2)
        );
    }

    /// A division with its Divisions row but no DivisionStatus row keeps a
    /// condition; each bar is hidden only for its own missing maximum, and
    /// the refusal names the table's state.
    #[test]
    fn a_missing_status_row_hides_each_bar_by_its_own_maximum() {
        use replay::rows::DivisionRow;
        let mut frame = window_frame(None, None);
        *frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                id: 9,
                hit_points: 50_000,
                organisation: 30_000,
                ..Default::default()
            }]));
        *frame.tables[TableId::DivisionStatus.index()] = TableView::Truncated {
            rows: TableRows::DivisionStatus(Vec::new()),
            missing: 2,
        };
        let bars = conditions(&frame).unwrap();
        let status_gap = absence(&rows::<tables::DivisionStatus>(&frame).unwrap());
        let row = LandCombatDivisionRow {
            combat: COMBAT,
            division: 9,
            ..Default::default()
        };
        let mut out = Bindings::default();
        let mut values = Values::new();
        condition_bars(
            &mut out,
            &mut values,
            &row,
            false,
            Ok(bars[&9]),
            &status_gap,
        );
        for bar in ["str_bar", "org_bar"] {
            assert!(values[bar].hidden && values[bar].fill.is_none(), "{bar}");
        }
        assert!(refused(
            &out,
            "Land combat 62 division 9: its DivisionStatus row is missing (the table is truncated, 2 rows unread), so its hit points maximum is unknown; its strength bar is hidden."
        ));
        assert!(refused(
            &out,
            "so its organisation maximum is unknown; its organisation bar is hidden."
        ));
        // A known maximum binds its own bar although the other is missing.
        let mut values = Values::new();
        let strength_only = Condition {
            max_strength: Some(100_000),
            ..bars[&9]
        };
        condition_bars(
            &mut out,
            &mut values,
            &row,
            false,
            Ok(strength_only),
            &status_gap,
        );
        assert_eq!(values["str_bar"].fill, Some(0.5));
        assert!(!values["str_bar"].hidden);
        assert!(values["org_bar"].hidden && values["org_bar"].fill.is_none());
    }

    #[test]
    fn committed_names_carry_the_owner_flag_on_the_side_the_native_entry_writes() {
        let name = || "144ya Strelkovaya Diviziya".to_owned();
        let flag = "SOV_communism".to_string();
        assert_eq!(
            entry_name(name(), 0, true, Some(&flag)),
            format!("144ya Strelkovaya Diviziya {INLINE_FLAG}SOV_communism")
        );
        assert_eq!(
            entry_name(name(), 0, false, Some(&flag)),
            "144ya Strelkovaya Diviziya"
        );
        assert_eq!(
            entry_name(name(), 1, true, Some(&flag)),
            format!("{INLINE_FLAG}SOV_communism 144ya Strelkovaya Diviziya")
        );
        assert_eq!(
            entry_name(name(), 1, true, None),
            "144ya Strelkovaya Diviziya"
        );
    }

    #[test]
    fn names_prefer_a_custom_name_then_the_selected_group_in_the_content() {
        let names = installed_content::division_names::parse(
            br#"SOV_INF_01 = { ordered = { 144 = { "%dya Strelkovaya Diviziya" } } }"#,
        )
        .unwrap();
        let mut row = LandCombatDivisionRow {
            name_group: b"SOV_INF_01".to_vec(),
            name_order: 144,
            name_ordered: 1,
            ..Default::default()
        };
        assert_eq!(
            division_name(&row, &names).as_deref(),
            Ok("144ya Strelkovaya Diviziya")
        );
        row.name_custom = b"Guards".to_vec();
        assert_eq!(division_name(&row, &names).as_deref(), Ok("Guards"));
        // A programmatic member with a group: the capture records no stored
        // name, and the group's entry is shown.
        row.name_programmatic = 1;
        row.name_custom.clear();
        assert_eq!(
            division_name(&row, &names).as_deref(),
            Ok("144ya Strelkovaya Diviziya")
        );
        // With a null group the capture records the stored name, which
        // 1409B5640 shows.
        row.name_group.clear();
        row.name_custom = b"1-ya Strelkovaya".to_vec();
        assert_eq!(
            division_name(&row, &names).as_deref(),
            Ok("1-ya Strelkovaya")
        );
        // Without one the game builds the legacy name.
        row.name_custom.clear();
        assert_eq!(
            division_name(&row, &names),
            Err(Unnamed::Unresolved(Unresolved::NoGroup))
        );
        row.name_group = b"SOV_INF_01".to_vec();
        row.name_type = 1;
        assert_eq!(division_name(&row, &names), Err(Unnamed::ForeignType(1)));
    }

    /// Each cause is one warning naming every division it leaves unnamed,
    /// with its group and ordinal.
    #[test]
    fn unnamed_divisions_are_reported_by_cause_with_their_identities() {
        let names = installed_content::division_names::parse(
            br#"INF = { ordered = { 1 = { "First" } } unordered = { loose = { "Loose" } } }"#,
        )
        .unwrap();
        let row = |division, group: &[u8], order, ordered: u8| LandCombatDivisionRow {
            division,
            name_group: group.to_vec(),
            name_order: order,
            name_ordered: ordered,
            ..Default::default()
        };
        let rows = [
            row(1, b"INF", 1, 1),
            row(2, b"INF", 0, 1),
            row(3, b"INF", 2, 1),
            row(4, b"INF", 1, 0),
            row(5, b"GER\xff", 1, 1),
            row(6, b"ABSENT", 3, 1),
            LandCombatDivisionRow {
                name_custom: b"\xffGuards".to_vec(),
                ..row(7, b"INF", 1, 1)
            },
            LandCombatDivisionRow {
                name_type: 2,
                ..row(8, b"INF", 1, 1)
            },
            // A player's name holding the stored flag control 0x12.
            LandCombatDivisionRow {
                name_custom: b"\x12GER Guards".to_vec(),
                ..row(10, b"INF", 1, 1)
            },
        ];
        struct NoIcons;
        impl TextIcons for NoIcons {
            fn icon(&self, _: &str) -> Option<(u32, i64)> {
                None
            }
        }
        let warnings = unnamed_warnings(9, rows.iter(), &names, &NoIcons);
        let expected = [
            (Unnamed::CustomNotText, "division 7 (group INF, ordered 1)"),
            (Unnamed::ForeignType(2), "division 8 (group INF, ordered 1)"),
            (
                Unnamed::Unresolved(Unresolved::GroupAbsent),
                "division 5 (group GER\\xff, ordered 1); division 6 (group ABSENT, ordered 3)",
            ),
            (
                Unnamed::Unresolved(Unresolved::NoFallback),
                "division 3 (group INF, ordered 2); division 4 (group INF, unordered 1)",
            ),
            (
                Unnamed::Unresolved(Unresolved::UnassignedOrder),
                "division 2 (group INF, ordered 0)",
            ),
            (
                Unnamed::Control(
                    "the inline flag 0x12 \"GER\" (an authored `@`), which the viewer does not draw"
                        .into(),
                ),
                "division 10 (group INF, ordered 1)",
            ),
        ];
        assert_eq!(warnings.len(), expected.len(), "{warnings:#?}");
        for (cause, divisions) in expected {
            let count = divisions.split("; ").count();
            let warning = format!(
                "Land combat 9: {count} division names are unavailable because {cause}: {divisions}."
            );
            assert!(warnings.contains(&warning), "{warning}\n{warnings:#?}");
        }
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
            several_directions(&combat, false, &divisions, &movement),
            Some(true)
        );
        // A border war never has several directions.
        assert_eq!(
            several_directions(&combat, true, &divisions, &movement),
            Some(false)
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
        assert_eq!(
            several_directions(&combat, false, &divisions, &movement),
            None
        );
        movement.insert(1, 9263);
        assert_eq!(
            several_directions(&combat, false, &divisions, &movement),
            Some(false)
        );
        movement.insert(1, 0);
        assert_eq!(
            several_directions(&combat, false, &divisions, &movement),
            Some(true)
        );
    }

    #[test]
    fn a_missing_modifier_row_leaves_the_combat_class_undecided() {
        use replay::rows::LandCombatModifierRow;
        let row = |combat, side, border_values| LandCombatModifierRow {
            combat,
            side,
            border_values,
            ..Default::default()
        };
        assert_eq!(border_war(&[], 5), None);
        assert_eq!(border_war(&[row(4, 0, Some([0; 3]))], 5), None);
        // Either side's row decides; a projected-out side does not matter.
        assert_eq!(border_war(&[row(5, 1, None)], 5), Some(false));
        assert_eq!(
            border_war(
                &[row(4, 0, None), row(5, 1, Some([0, 100_000, 100_000]))],
                5
            ),
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

    use replay::{
        frame::{Metrics, TableView},
        rows::{
            LandCombatDetailsRow, LandCombatRow, LandCombatSideDetails, TableId, TableRows,
            WeatherProvinceRow,
        },
    };

    const COMBAT: u64 = 62;
    const PROVINCE: u32 = 3_000;

    fn window_frame(
        detail: Option<LandCombatDetailsRow>,
        weather: Option<TableView>,
    ) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 0,
            hour: 7,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "not recorded".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::LandCombats.index()] =
            TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
                id: COMBAT,
                province: PROVINCE,
                ..Default::default()
            }]));
        *frame.tables[TableId::LandCombatDetails.index()] =
            TableView::Complete(TableRows::LandCombatDetails(detail.into_iter().collect()));
        if let Some(weather) = weather {
            *frame.tables[TableId::WeatherProvinces.index()] = weather;
        }
        frame
    }

    fn snow(snow: i64) -> Option<TableView> {
        Some(TableView::Complete(TableRows::WeatherProvinces(vec![
            WeatherProvinceRow {
                id: PROVINCE,
                snow,
                ..Default::default()
            },
        ])))
    }

    /// The window as an earlier binding left it: a first-member flag on each
    /// side and the summer terrain picture.
    fn stale() -> Bindings {
        let mut out = Bindings::default();
        for key in ["attacker_flag", "defender_flag"] {
            out.bind(key, FLAG, Ok("SOV".into()));
        }
        out.bind("terrain_picture", SPRITE, Ok("GFX_terrain_plains".into()));
        out
    }

    fn bind_pictures(frame: &DecodedFrame, sprites: &[&str]) -> Bindings {
        let terrain = BTreeMap::from([(PROVINCE, "plains".to_owned())]);
        let flags = BTreeMap::from([(7, "GER".to_owned())]);
        let has_sprite = |name: &str| sprites.contains(&name);
        let mut out = stale();
        pictures(&mut out, frame, COMBAT, &terrain, &has_sprite, &flags).unwrap();
        out
    }

    fn refused(out: &Bindings, fragment: &str) -> bool {
        out.refused()
            .keys()
            .any(|refusal| refusal.contains(fragment))
    }

    /// 1415C5F10 returns the side's country with or without a commander, and
    /// 1415C9860 binds the flag from it: without a commander the recorded
    /// first primary country (or province controller) replaces the earlier
    /// first-member flag; a country without artwork hides the flag.
    #[test]
    fn a_side_without_a_commander_takes_its_recorded_country_flag() {
        let detail = LandCombatDetailsRow {
            combat: COMBAT,
            province: PROVINCE,
            sides: [
                LandCombatSideDetails {
                    commander: 0,
                    commander_country: 7,
                    ..Default::default()
                },
                LandCombatSideDetails {
                    commander: 0,
                    commander_country: 9,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let out = bind_pictures(
            &window_frame(Some(detail), snow(0)),
            &["GFX_terrain_plains"],
        );
        let attacker = &out.values["attacker_flag"];
        assert_eq!(attacker.flag.as_deref(), Some("GER"));
        assert!(!attacker.hidden);
        let defender = &out.values["defender_flag"];
        assert_eq!(defender.flag, None, "the earlier SOV flag is cleared");
        assert!(defender.hidden);
        assert!(refused(
            &out,
            "Land combat 62 defender flag is hidden: no combat flag resolves for country 9"
        ));
        // Without LandCombatDetails the side country is unknown: neither the
        // earlier flag nor a first-member guess is kept.
        let out = bind_pictures(&window_frame(None, snow(0)), &["GFX_terrain_plains"]);
        for key in ["attacker_flag", "defender_flag"] {
            assert!(out.values[key].hidden && out.values[key].flag.is_none());
        }
        assert!(refused(
            &out,
            "attacker flag is hidden: LandCombatDetails has no row for the combat"
        ));
    }

    /// 1415C7520 adds `_winter` for positive province snow. A missing winter
    /// sprite or a missing weather row hides the picture and names the gap,
    /// rather than keeping the earlier summer picture or assuming no snow.
    #[test]
    fn the_terrain_picture_is_decided_once_and_hidden_on_refusal() {
        let both = ["GFX_terrain_plains", "GFX_terrain_plains_winter"];
        let picture = |out: &Bindings| {
            let widget = &out.values["terrain_picture"];
            (widget.sprite.clone(), widget.hidden)
        };
        let out = bind_pictures(&window_frame(None, snow(0)), &both);
        assert_eq!(picture(&out), (Some("GFX_terrain_plains".into()), false));
        let out = bind_pictures(&window_frame(None, snow(1)), &both);
        assert_eq!(
            picture(&out),
            (Some("GFX_terrain_plains_winter".into()), false)
        );
        // Snow with no winter sprite: the summer picture bound earlier goes.
        let out = bind_pictures(&window_frame(None, snow(1)), &["GFX_terrain_plains"]);
        assert_eq!(picture(&out), (None, true));
        assert!(refused(
            &out,
            "Land combat 62 terrain picture is hidden: the content lacks sprite GFX_terrain_plains_winter (1415C7520)."
        ));
        // Missing weather is not zero snow, whatever the table's state.
        for (weather, state) in [
            (None, "the table is unavailable: not recorded"),
            (
                Some(TableView::Complete(TableRows::WeatherProvinces(Vec::new()))),
                "the complete table lacks it",
            ),
            (
                Some(TableView::Truncated {
                    rows: TableRows::WeatherProvinces(Vec::new()),
                    missing: 4,
                }),
                "the table is truncated, 4 rows unread",
            ),
        ] {
            let out = bind_pictures(&window_frame(None, weather), &both);
            assert_eq!(picture(&out), (None, true), "{state}");
            assert!(
                refused(
                    &out,
                    &format!(
                        "WeatherProvinces has no row for province 3000 ({state}), so its snow and the summer or `_winter` picture are unknown"
                    )
                ),
                "{:?}",
                out.refused()
            );
        }
    }

    /// A value bound after a refusal shows the widget again; a repeated
    /// refusal is counted, not repeated.
    #[test]
    fn a_binding_after_a_refusal_shows_the_widget_again() {
        let mut out = Bindings::default();
        out.bind("name", TEXT, Ok("early".into()));
        out.bind("name", TEXT, Err("name refused".into()));
        out.bind("name", TEXT, Err("name refused".into()));
        assert!(out.values["name"].hidden && out.values["name"].text.is_none());
        assert_eq!(out.refused().get("name refused").copied(), Some(2));
        out.bind("name", TEXT, Ok("late".into()));
        assert!(!out.values["name"].hidden);
        assert_eq!(out.values["name"].text.as_deref(), Some("late"));
    }
}
