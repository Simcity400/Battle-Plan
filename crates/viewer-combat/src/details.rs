//! Combat window operands from recorded simulation state and matching content.
use installed_content::{
    division_names::{Database, Unresolved},
    localisation::{Argument, Localisation},
};
use replay::{
    frame::DecodedFrame,
    rows::{LandCombatDivisionRow, TableSchema, tables},
};
use std::{collections::BTreeMap, io};
use viewer_foundation::tooltip_text::tooltip_keys;
use {
    super::INLINE_FLAG, super::Selection, super::Values, super::Widget, super::binding::Bindings,
    super::binding::FILL, super::binding::FLAG, super::binding::Flags, super::binding::SPRITE,
    super::binding::TEXT, super::binding::country_flag, super::binding::fill, super::binding::hide,
    super::binding::text, super::template_icons, super::tooltips::hover_text,
    super::tooltips::set_hover, viewer_foundation::land_combat_math::Condition,
    viewer_foundation::land_combat_math::conditions, viewer_foundation::tooltip_text::TextIcons,
    viewer_foundation::tooltip_text::builtin, viewer_foundation::tooltip_text::displayable,
    viewer_foundation::tooltip_text::token,
};
use {
    super::content::Content as StaticContent,
    viewer_foundation::map::{
        perspective::Vision,
        tables::{Available, rows},
    },
};

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
        Selection::NavalResult(_) => Ok(()),
    }
}

fn show(values: &mut Values, key: &str) {
    values.entry(key.into()).or_default().hidden = false;
}

/// The land window facts decided once per bind, here and nowhere else: the
/// terrain picture and each side's flag. Each binds
/// through [`Bindings::bind`], so a refusal hides the widget.
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
        // The flag widget uses the selected commander's country, else the first
        // primary country, else the attacked province's controller. The
        // recorder keeps that country as
        // `commander_country` with or without a commander.
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

/// terrain picture: `GFX_terrain_<category>` of the combat
/// province, with `_winter` when that province's weather snow
/// (`WeatherProvinces.snow`) is positive. Missing weather is not zero snow:
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
        refused(format!("the content lacks sprite {name}"))
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
// Signed 64-bit multiplication precedes the fixed5 division.
fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / 100_000
}
fn fraction(value: i64, maximum: i64) -> f32 {
    ((value as f64 / maximum.max(100) as f64).clamp(0.0, 1.0)) as f32
}
/// Why a division's name cannot be rendered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Unnamed {
    CustomNotText,
    StoredText(String),
    Unavailable(String),
    /// The name holds a control the native text layout does not draw ([`displayable`]).
    Control(String),
    /// Divisions use type-zero names bound to their template. Other name types
    /// (1 ship/variant, 2 operative, 3 railway gun, 4 unassigned) have no
    /// content form for a division; an edited save can still record one.
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

/// Use a recorded custom name unless it is programmatic with a resolved
/// name group. Otherwise use the selected name of a type-zero member from
/// the content's name groups.
fn division_name(
    row: &LandCombatDivisionRow,
    names: &Database,
    localisation: &Localisation,
    recorded_language: &str,
) -> Result<String, Unnamed> {
    division_name_operands(
        division_custom_name(
            &row.name_custom,
            row.name_programmatic,
            &row.name_group,
            localisation,
            recorded_language,
        )?,
        row.name_type,
        &row.name_group,
        row.name_order,
        row.name_ordered != 0,
        names,
    )
}

/// Player overrides are literals. Visible programmatic overrides are stored client text;
/// a resolved group instead generates the name, even if an override was recorded.
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

/// Historical generator-operand name, before the retained native cache was recorded.
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

/// [`division_name`] as the entry draws it: the text path reads
/// its control bytes, so one the native text layout does not draw is refused.
fn shown_name(
    row: &LandCombatDivisionRow,
    names: &Database,
    icons: &impl TextIcons,
    localisation: &Localisation,
    recorded_language: &str,
) -> Result<String, Unnamed> {
    division_name(row, names, localisation, recorded_language)
        .and_then(|name| displayable(icons, name).map_err(Unnamed::Control))
}

/// The live getter returns the retained cache, not a freshly generated name.
/// Only an explicitly unrecorded cache uses the old combat naming projection.
fn shown_recorded_name(
    row: &LandCombatDivisionRow,
    status: Option<&replay::rows::DivisionStatusRow>,
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
    if matches!(status.name, replay::rows::Recorded::NotRecorded) {
        return shown_name(row, names, icons, localisation, recorded_language);
    }
    dashboard_text::unit_names::army_name(status, localisation, recorded_language, || Ok(names))
        .map_err(Unnamed::Unavailable)
        .and_then(|name| displayable(icons, name).map_err(Unnamed::Control))
}

/// One warning per cause for a combat's divisions whose names are
/// unavailable, naming each division with its group and ordinal.
fn unnamed_warnings<'a>(
    combat: u64,
    rows: impl Iterator<Item = &'a LandCombatDivisionRow>,
    name: impl Fn(&LandCombatDivisionRow) -> Result<String, Unnamed>,
) -> Vec<String> {
    let mut causes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        if let Err(cause) = name(row) {
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

/// Committed entries put the inline flag after the attacker's name and
/// before the defender's name. Reserve entries use the bare name.
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
        // Division combat inputs, then side bonus and cached stats.
        // A row holding neither its numbers nor an older packet shows no
        // stat box.
        let (scales, stats) = (row.scales(), row.stat_values());
        let attack = bonus
            .zip(scales)
            .map(|(bonus, scales)| mul(mul(scales[2], scales[0]), bonus.wrapping_add(100_000)));
        let defence = scales.map(|scales| mul(scales[3], scales[1]));
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
                // The game sets each stat box through fixed-point formatting
                // at precision 0, so the whole part is
                // truncated rather than rounded.
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

/// The game ignores the hovered child and dispatches by the global modifier
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
            // The game wraps the entry in the control byte 0x11 and a colour
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
            BM_FORT_MODIFIER,
            dlc_mask.map(|mask| {
                // feature ID 0x23 maps to the saved DLC mask's
                // bit 7, not to a bit numbered 0x23.
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
    // The BM_ keys are the executable's: each must be in the localisation,
    // and a missing or unsupported one is refused by name, never shown as
    // its own text.
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

/// The side whose `*_leader_initiative_overlay_glow` is shown. Each side
/// scores its selected commander's skill, zero without one, plus the
/// `RECON_SKILL_IMPACT` define when its greatest full-roster reconnaissance
/// is strictly greater. The higher score glows; a tie shows neither.
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
// The full-side ordinal differs from the committed/reserve list order
// recorded in LandCombatDetails. The ordinal
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

/// True once two units in the side's full ordered LandCombats roster have
/// different direction keys. Border-war sides always return false.
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

/// One combat's LandCombatDivisionModifiers rows by side and division. A
/// division a side lists twice keeps its first row, as the side's member list
/// was searched before.
pub(super) fn division_modifiers(
    rows: &[replay::rows::LandCombatDivisionModifierRow],
    id: u64,
) -> BTreeMap<(u8, u64), &replay::rows::LandCombatDivisionModifierRow> {
    let mut members = BTreeMap::new();
    for row in rows.iter().filter(|row| row.combat == id) {
        members.entry((row.side, row.division)).or_insert(row);
    }
    members
}

/// Whether the combat is a border-war combat, whose type token 0x39A5
/// selects both the window title and the combatant
/// class above. Only a border war records border values on its modifier
/// rows, so without any row for the combat the class stays undecided.
pub(super) fn border_war(rows: &[replay::rows::LandCombatModifierRow], id: u64) -> Option<bool> {
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
    let divisions = rows::<tables::LandCombatDivisions>(frame)?;
    let divisions_complete = divisions.is_complete();
    let divisions: BTreeMap<_, _> = divisions
        .rows()
        .iter()
        .filter(|r| r.combat == id)
        .map(|r| ((r.side, r.division), r))
        .collect();
    let modifiers = rows::<tables::LandCombatModifiers>(frame)?;
    let members = rows::<tables::LandCombatDivisionModifiers>(frame)?;
    let member_gap = absence(&members);
    let members = division_modifiers(members.rows(), id);
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
    let shown = |row: &LandCombatDivisionRow| {
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
    // Undecided entry flags and exile badges by widget and cause, with the
    // committed divisions each cause leaves undecided: one warning per cause.
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
                    condition_bars(out, &mut widgets, row, committed, condition, &status_gap);
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
            // COUNT is the side's committed or reserve count as a string
            // argument (signed integer formatting,
            // string-argument localisation). An unknown count
            // takes the viewer's unknown dash.
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
            // A commander name localises its recorded token; no commander shows
            // UNIT_LEADER_NO_LEADER.
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
            // The side flag is bound in `pictures`, with or without a
            // commander.
            if value.commander != 0 {
                // UNIT_LEADER_SKILL with the integer argument SKILL
                // (value localisation, type 5).
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
                // The game writes an empty skill beside UNIT_LEADER_NO_LEADER
                // and sets the sprite it names on the portrait button.
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
            // Preserve strip order. Frame 6 is attacker-only and requires
            // several attack directions.
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
    // Hide the controlling-player button when neither primary side list
    // contains an equivalent viewing country.
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
        // Compare the viewing country with the defender primary list.
        // A match shows the defender skin;
        // otherwise the attacker skin. Both widgets receive the same progress.
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
        // The window recomputes positioning, screening and anti-air from live operands on
        // every refresh; the combat's stored copies lag them.
        let combat_ships = rows::<tables::NavalCombatShips>(frame)?;
        let ship_models = rows::<tables::ShipModels>(frame)?;
        let air_bases = rows::<tables::AirBases>(frame)?;
        let live_ships = rows::<tables::Ships>(frame)?;
        let defines = static_content.assets.naval_defines.as_ref();
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
                &static_content.assets.naval_air_groups,
                defines,
            )
        } else {
            [None; 2]
        };
        for (side, value) in detail.sides.iter().enumerate() {
            let label = if side == 0 { "attacker" } else { "defender" };
            let wnd = if side == 0 { "left" } else { "right" };
            // Recomputed screening uses fixed-point percentage formatting
            // at precision 0, times 100.
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
            // positioning as the screening, anti-air
            // through fixed-point formatting at precision 1, truncating.
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
                // Refresh the commander portrait with the combat details.
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
                // Set "GFX_leader_unknown" on the
                // portrait button; land's GFX_no_combat_leader is not used.
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
        out.warnings.push("Naval: the perspective progress skin, recomputed screening, positioning and AA, ordered side flags and commander name/skills are bound. Forward active air groups and their carrier-grid entries are bound from recorded variants and the stacked equipment registry. Hover capture gaps are reported as counted refusals beside their affected widgets. Phase has no separate scalar widget in navalcombat.gui; retreat/engagement operands are not relabelled as a guessed phase.".into());
    }
    Ok(())
}

/// Compare only the two primary country lists against the viewing country.
/// Allied vision does not admit the retreat button. Enable it unless the
/// matched side is already retreating; hide it when neither side matches.
fn naval_disengage_state(
    detail: &replay::rows::NavalCombatDetailsRow,
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
        // The game chooses the attacker first if both primary lists match.
        (Ok(true), _) => Ok(Some(enabled[0])),
        (Ok(false), Ok(true)) => Ok(Some(enabled[1])),
        // An unknown attacker cannot change visibility when the defender
        // matches, nor the state if both possible sides have the same state.
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

/// The game selects the defender skin when the viewing
/// country is in the defender primary list. Raw links resolve to canonical
/// slots before comparison (`tactics::viewing_side`).
fn naval_progress_skin(
    detail: &replay::rows::NavalCombatDetailsRow,
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
