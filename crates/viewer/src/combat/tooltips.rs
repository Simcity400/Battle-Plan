//! Land combat hover text rebuilt from simulation operands and matching content.
//!
//! Combat details supply tactic, terrain, width and commander hovers.
use super::{
    HoverText, Selection, Values,
    binding::{Bindings, TextIcons, builtin, token},
    land_details::{Conditions, condition_gap, conditions, division_width},
};
use crate::{
    document::Document,
    map::{counters::NativeWindows, perspective::Vision},
    table_access::rows,
};
use installed_content::{
    gui::native::{self, Element},
    localisation::{Argument, Localisation},
    unit_leader::Skills,
};
use replay::{
    frame::DecodedFrame,
    rows::{LandCombatDetailsRow, LandCombatSideDetails, LandCombatSideRow, tables},
};
use std::{collections::BTreeMap, io};

use super::tactics::{Content as Tactics, Effects};

#[derive(Debug, Default)]
pub(super) struct Content {
    skills: Skills,
    warnings: Vec<String>,
}

impl Content {
    /// `skills` are `common/unit_leader`'s, read with the commander content
    /// (their files' readers also hold the traits). Tactic effects come from
    /// the one tactic registry ([`Tactics`]).
    pub(super) fn load(skills: Skills) -> Self {
        Self {
            warnings: skills.warnings().to_vec(),
            skills,
        }
    }

    pub(super) fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// the game rebuilds the six combat sums after tactic changes and load.
/// Organisation terms come from content; the two recorded damage sums
/// prove the current side membership still matches that rebuild. The
/// error names the missing operand.
pub(super) fn organisation_effects(
    tactics: &Tactics,
    detail: &LandCombatDetailsRow,
    sides: Option<&LandCombatSideRow>,
) -> Result<[i64; 2], String> {
    let sides = sides.ok_or("the combat's LandCombatSides row is missing")?;
    if sides.combat != detail.combat {
        return Err(format!(
            "the LandCombatSides row is combat {}'s, not {}'s",
            sides.combat, detail.combat
        ));
    }
    // the game scopes each side to its first linked-set country. An empty
    // set during teardown cannot authenticate the previous effects.
    for (side, primary) in [
        ("attacker", &sides.attacker_primary),
        ("defender", &sides.defender_primary),
    ] {
        match primary.first() {
            None => return Err(format!("the {side}'s primary country set is empty")),
            Some(0) => return Err(format!("the {side}'s first primary country is null")),
            Some(_) => {}
        }
    }
    let mut damage = [0_i64; 2];
    let mut organisation = [0_i64; 2];
    for (index, side) in detail.sides.iter().enumerate() {
        if side.tactic_kind <= 1 || countered(side, &detail.sides[1 - index]) {
            continue;
        }
        if side.tactic_kind != 2 {
            return Err(format!(
                "side {index}'s tactic kind {} is not a combat tactic",
                side.tactic_kind
            ));
        }
        let tactic = tactics.effects(side.tactic_id, &side.tactic_token)?;
        damage[0] = damage[0].wrapping_add(tactic.attacker);
        damage[1] = damage[1].wrapping_add(tactic.defender);
        organisation[0] = organisation[0].wrapping_add(tactic.attacker_org);
        organisation[1] = organisation[1].wrapping_add(tactic.defender_org);
    }
    if damage != [detail.attacker_bonus, detail.defender_bonus] {
        return Err(format!(
            "the content tactics' damage sums {damage:?} differ from the recorded {:?}, so the side membership they were rebuilt for is not this hour's",
            [detail.attacker_bonus, detail.defender_bonus]
        ));
    }
    Ok(organisation)
}

/// the game tests the other pointer, not whether it is a real tactic.
/// Kind 1 is a present null tactic (id 0); only kind 0 is an absent pointer.
pub(super) fn countered(side: &LandCombatSideDetails, other: &LandCombatSideDetails) -> bool {
    side.tactic_kind != 0
        && other.tactic_kind != 0
        && side.tactic_counter_id == other.tactic_id as u32
}

/// Trim newline, tab, carriage return and space from both ends of tooltip
/// text. Other Unicode whitespace remains. An empty result gives no hover.
pub(super) fn hover_text(text: &str) -> Option<HoverText> {
    let text = text.trim_matches(['\n', '\t', '\r', ' ']);
    (!text.is_empty()).then(|| text.into())
}

/// The one hover setter: both strings of `key`'s widget, which the dynamic
/// builder then owns. A refused hover is `None` and clears what the widget
/// held, so an authored `pdx_tooltip` never stands in for a refused dynamic
/// one, whichever binds first; the caller counts the refusal.
pub(super) fn set_hover(
    values: &mut Values,
    key: impl Into<String>,
    text: Option<HoverText>,
    description: Option<HoverText>,
) {
    let widget = values.entry(key.into()).or_default();
    widget.tooltip = text;
    widget.tooltip_description = description;
    widget.hover_owned = true;
}

/// Clears the dynamically owned hovers `keys` a builder cannot make at all,
/// so none keeps its authored seed.
pub(super) fn refuse_hovers<K: Into<String>>(
    values: &mut Values,
    keys: impl IntoIterator<Item = K>,
) {
    for key in keys {
        set_hover(values, key, None, None);
    }
}

/// The land hovers the game sets on every update of one side.
fn side_hovers(prefix: &str) -> [String; 9] {
    [
        "tactic",
        "countered_tactic_overlay",
        "leader_skill",
        "leader_skill_bg",
        "leader_army_hq_icon",
        "leader_initiative_overlay_glow",
        "air_text",
        "air_count",
        "air_icon",
    ]
    .map(|suffix| format!("{prefix}_{suffix}"))
}

/// [`hover_text`] of a built hover, or `None` with its refusal counted.
pub(super) fn counted_hover(
    text: Result<String, impl std::fmt::Display>,
    refused: &mut BTreeMap<String, usize>,
) -> Option<HoverText> {
    match text {
        Ok(text) => hover_text(&text),
        Err(reason) => {
            *refused.entry(reason.to_string()).or_default() += 1;
            None
        }
    }
}

/// What the authored seed reads: the localiser, the native text icons and the
/// combat windows an entry template names.
struct Gui<'a> {
    icons: &'a dyn TextIcons,
    loc: &'a Localisation,
    windows: &'a NativeWindows,
}

// GUI field reading stores hash, not a text lookup key.
fn authored_key<'a>(
    hash: u64,
    key: &'a [u8],
    path: &str,
    field: &str,
) -> Result<Option<&'a str>, String> {
    if hash == 0 && key.is_empty() {
        return Ok(None);
    }
    if native::tooltip_hash(key) != hash {
        return Err(format!(
            "authored_tooltip_hash_mismatch: {path}'s {field} hash {hash:#018x} does not match its stored key {key:?}"
        ));
    }
    if hash == 0 {
        return Ok(None);
    }
    std::str::from_utf8(installed_content::parser::c_string(key))
        .map(Some)
        .map_err(|_| {
            format!("authored_tooltip_undecodable_key: {path}'s {field} key {key:?} is not UTF-8")
        })
}

fn authored_hover(
    gui: &Gui<'_>,
    hash: u64,
    key: &[u8],
    path: &str,
    field: &str,
    refused: &mut BTreeMap<String, usize>,
) -> Option<HoverText> {
    let token = match authored_key(hash, key, path, field) {
        Ok(Some(token)) => token,
        Ok(None) => return None,
        Err(reason) => {
            *refused.entry(reason).or_default() += 1;
            return None;
        }
    };
    counted_hover(builtin(gui.icons, gui.loc, token, &[]), refused)
}

/// Authored `pdx_tooltip`/`pdx_tooltip_delayed` keys of every widget, the
/// text it is created with: a widget whose hovers a dynamic builder owns
/// keeps the builder's text or refusal. Missing localisation keeps the key's
/// text; an invalid stored hash/key or text [`builtin`] cannot produce is
/// counted in `refused`, immediate or delayed alike. Button shortcuts remain
/// an independent tail even when a dynamic builder owns the text.
fn authored(
    gui: &Gui<'_>,
    node: &Element,
    path: &str,
    values: &mut Values,
    refused: &mut BTreeMap<String, usize>,
) {
    for child in node.nested() {
        let key = if path.is_empty() {
            child.name().into_owned()
        } else {
            format!("{path}/{}", child.name())
        };
        // Native lookup folds ASCII case; keep the producer's path and hover ownership.
        let key = values
            .get_key_value(&key)
            .map(|(path, _)| path)
            .or_else(|| values.keys().find(|path| path.eq_ignore_ascii_case(&key)))
            .cloned()
            .unwrap_or(key);
        let owned = values.get(&key).is_some_and(|widget| widget.hover_owned);
        if !owned && (child.common.tooltip != 0 || !child.common.tooltip_key.is_empty()) {
            values.entry(key.clone()).or_default().tooltip = authored_hover(
                gui,
                child.common.tooltip,
                &child.common.tooltip_key,
                &key,
                "pdx_tooltip",
                refused,
            );
        }
        if !owned
            && (child.common.delayed_tooltip != 0 || !child.common.delayed_tooltip_key.is_empty())
        {
            values.entry(key.clone()).or_default().tooltip_description = authored_hover(
                gui,
                child.common.delayed_tooltip,
                &child.common.delayed_tooltip_key,
                &key,
                "pdx_tooltip_delayed",
                refused,
            );
        }
        if let Some(shortcut) = child.button().and_then(|button| button.shortcuts.first()) {
            values.entry(key.clone()).or_default().tooltip_shortcut =
                counted_hover(shortcut_tooltip(gui, shortcut, &key), refused);
        }
        authored(gui, child, &key, values, refused);
        if let Some(name) = values
            .get(&key)
            .and_then(|widget| widget.entry_template.clone())
            && let Some(widget) = values.get_mut(&key)
        {
            for (index, entry) in widget.entries.iter_mut().enumerate() {
                let selected = widget.entry_templates.get(&index).unwrap_or(&name);
                let Some(template) = gui.windows.gui_type(selected.as_bytes()) else {
                    *refused.entry(format!("{key}'s entry template {selected} is not in the combat interface files, so its authored hovers are not bound")).or_default() += 1;
                    continue;
                };
                authored(gui, template, "", entry, refused);
            }
        }
    }
}

/// Authored text and shortcuts for every constructed combat window and entry.
pub(super) fn bind_authored(
    out: &mut Bindings,
    icons: &dyn TextIcons,
    loc: &Localisation,
    windows: &NativeWindows,
    selection: Selection,
) {
    let gui = Gui {
        icons,
        loc,
        windows,
    };
    let mut refused = BTreeMap::new();
    let name = super::gui::window_name(selection);
    match gui.windows.gui_type(name.as_bytes()) {
        Some(window) => authored(&gui, window, "", &mut out.values, &mut refused),
        None => out.refuse(format!(
            "Combat GUI {name}: the content declares no window, so its authored hovers are unavailable"
        )),
    }
    if matches!(selection, Selection::Land(_)) {
        // the game selects the division entry templates.
        for (side, entries) in out.entries.iter_mut().enumerate() {
            let name = match side {
                0 => "division_combat_attacker_entry",
                1 => "division_combat_defender_entry",
                _ => "reserve_combat_entry",
            };
            authored_entries(
                &gui,
                name,
                entries.iter_mut().map(|(_, entry)| entry),
                &mut refused,
            );
        }
        // the game constructs the tactics window and entry templates.
        match gui.windows.gui_type(b"tactics_list_view") {
            Some(window) => authored(&gui, window, "", &mut out.tactics.values, &mut refused),
            None => out.refuse(
                "Combat GUI tactics_list_view: the content declares no window, so its authored hovers are unavailable".into(),
            ),
        }
        authored_entries(
            &gui,
            "tactics_list_entry",
            out.tactics.entries.iter_mut(),
            &mut refused,
        );
    } else {
        // the game constructs ship entries for live combat and results.
        authored_entries(
            &gui,
            "navalcombat_ship_entry",
            out.naval_entries.iter_mut().map(|row| &mut row.values),
            &mut refused,
        );
    }
    for (reason, count) in refused {
        for _ in 0..count {
            out.refuse(reason.clone());
        }
    }
}

fn authored_entries<'a>(
    gui: &Gui<'_>,
    name: &str,
    entries: impl Iterator<Item = &'a mut Values>,
    refused: &mut BTreeMap<String, usize>,
) {
    let template = gui.windows.gui_type(name.as_bytes());
    for entry in entries {
        match template {
            Some(window) => authored(gui, window, "", entry, refused),
            None => {
                let reason = format!(
                    "Combat GUI {name}: the content declares no entry template, so its authored hovers are unavailable"
                );
                *refused.entry(reason).or_default() += 1;
            }
        }
    }
}

/// A hover's immediate text and, when it has one, its delayed text or why
/// that is absent.
type Hover = (String, Option<Result<String, String>>);

/// The null tactic has a localised constructor name. An absent top-level
/// key displays as its own text, including the active null-tactic singleton.
const NULL_TACTIC_NAME: &str = "nullCombatTactic";
pub(super) const HEADER: &str = "HEADER";

/// The tactic a side's hover describes.
#[derive(Clone, Copy, Debug)]
pub(super) enum Described<'a> {
    /// A content tactic: its recorded database id and token.
    Content { id: i32, token: &'a [u8] },
    /// A present null tactic (recorded kind 1) uses the ordinary header,
    /// effect and delayed path with its default name and zero effects.
    Null,
}

/// Build the immediate text, then `TACTIC_DESC` in the delayed slot.
/// Only an absent tactic pointer (kind 0) has `NO_TACTIC` alone.
fn tactic_tooltip(
    icons: &dyn TextIcons,
    loc: &Localisation,
    tactics: &Tactics,
    side: &LandCombatSideDetails,
    other: &LandCombatSideDetails,
    attacker: bool,
) -> Result<Hover, String> {
    let described = match side.tactic_kind {
        0 => return Ok((builtin(icons, loc, NO_TACTIC, &[])?, None)),
        1 => Described::Null,
        2 => Described::Content {
            id: side.tactic_id,
            token: &side.tactic_token,
        },
        kind => {
            return Err(format!(
                "the recorded tactic kind {kind} is none of the absent pointer (0), present null tactic (1) and content tactic (2)"
            ));
        }
    };
    tactic_description(
        icons,
        loc,
        tactics,
        described,
        attacker,
        countered(side, other),
    )
    .map(|(text, description)| (text, Some(description)))
}

pub(super) fn tactic_description(
    icons: &dyn TextIcons,
    loc: &Localisation,
    tactics: &Tactics,
    described: Described<'_>,
    attacker: bool,
    countered: bool,
) -> Result<(String, Result<String, String>), String> {
    let name = match described {
        Described::Content { token: tactic, .. } => token(icons, loc, tactic)?,
        Described::Null => token(icons, loc, NULL_TACTIC_NAME.as_bytes())?,
    };
    let mut text = builtin(icons, loc, HEADER, &[("KEY", Argument::Text(&name))])?;
    if countered {
        text.push_str(&builtin(icons, loc, TACTIC_CURRENTLY_COUNTERED, &[])?);
    } else {
        // The null tactic's six effects are zero.
        let tactic = match described {
            Described::Content { id, token } => tactics.effects(id, token)?,
            Described::Null => Effects::default(),
        };
        for (value, label, favorable_side, width) in [
            (tactic.width, TACTICS_COMBATWIDTH, true, true),
            (
                tactic.movement,
                TACTICS_MOVEMENTSPEED_ATTACKER,
                attacker,
                false,
            ),
            (tactic.attacker, TACTICS_DAMAGE_ATTACKER, attacker, false),
            (
                tactic.attacker_org,
                TACTICS_ATTACKER_ORG_DAMAGE_MODIFIER,
                attacker,
                false,
            ),
            (tactic.defender, TACTICS_DAMAGE_DEFENDER, !attacker, false),
            // Defender organisation damage uses the attacker colour preference;
            // defender strength damage uses the defender preference.
            (
                tactic.defender_org,
                TACTICS_DEFENDER_ORG_DAMAGE_MODIFIER,
                attacker,
                false,
            ),
        ] {
            if value == 0 {
                continue;
            }
            let format = if width {
                TACTICS_WIDTH_VALUE
            } else if (value > 0) == favorable_side {
                TACTICS_VALUE_POSITIVE
            } else {
                TACTICS_VALUE_NEGATIVE
            };
            text.push_str(&builtin(icons, loc, label, &[])?);
            text.push(' ');
            text.push_str(&builtin(
                icons,
                loc,
                format,
                &[("VALUE", Argument::Fixed5(value))],
            )?);
            text.push('\n');
        }
    }
    Ok((text, builtin(icons, loc, TACTIC_DESC, &[])))
}

/// The leader skill hover. `levels` is whether the content's skill levels
/// hold: a block under a registered modifier before them in their file
/// leaves them unknown ([`installed_content::unit_leader::Conditional`]).
fn skill_tooltip(
    icons: &dyn TextIcons,
    loc: &Localisation,
    skill: i32,
    experience: i64,
    next: Option<Option<i32>>,
    ours: bool,
    levels: Result<(), &str>,
) -> Result<String, String> {
    if !ours {
        return builtin(
            icons,
            loc,
            LEADER_SKILL_DESC,
            &[("S", Argument::Int32(skill))],
        );
    }
    levels.map_err(|reason| format!("the content's leader skill levels are unknown: {reason}"))?;
    let next = next.ok_or_else(|| {
        format!("the content's leader skill levels do not define skill {skill}'s next level")
    })?;
    let Some(required_experience) = next else {
        return builtin(
            icons,
            loc,
            LEADER_SKILL_AT_CAP,
            &[("SKILL", Argument::Int32(skill))],
        );
    };
    let ratio = super::fixed::fraction(
        experience,
        i64::from(required_experience) * super::fixed::ONE,
    )
    .ok_or_else(|| format!("the experience fraction: {}", super::fixed::TRAP))?;
    builtin(
        icons,
        loc,
        LEADER_SKILL_WITH_PROGRESS,
        &[
            ("SKILL", Argument::Int32(skill)),
            ("PROGRESS", Argument::Fixed5(ratio)),
        ],
    )
}

/// The CAS summary the game builds for a side's air text, count and icon:
/// the side's own planes, then what the opposing side's anti-air did to them.
struct Cas<'a> {
    own: &'a LandCombatSideDetails,
    opponent: &'a LandCombatSideDetails,
    attacking: bool,
}

/// The CAS hover, or why it is refused: the opposing division, operand or
/// localisation key that failed.
fn cas_tooltip(
    icons: &dyn TextIcons,
    loc: &Localisation,
    cas: Cas<'_>,
    conditions: &Conditions,
    condition_gap: &dyn Fn(u64) -> String,
    widths: Result<i64, String>,
    defines: &super::defines::LandDefines,
) -> Result<(String, Result<String, String>), String> {
    let planes = cas
        .own
        .air_counts
        .iter()
        .fold(0_i32, |sum, count| sum.wrapping_add(*count));
    // The opponent's committed units' current strength and organisation.
    let (strength, organisation) = cas.opponent.committed.iter().try_fold(
        (0_i64, 0_i64),
        |(strength, organisation), unit| {
            let condition = conditions
                .get(unit)
                .ok_or_else(|| format!("opposing committed {}", condition_gap(*unit)))?;
            Ok::<_, String>((
                strength.wrapping_add(condition.strength),
                organisation.wrapping_add(condition.organisation),
            ))
        },
    )?;
    // the anti-air defence times the reduction factor,
    // capped at the maximum reduction.
    let defence = engine::csub_unit_definition::movement_air_defense_value(
        cas.opponent.anti_air_attack,
        defines.air_superiority[0],
        defines.air_superiority[1],
    )
    .map_err(|reason| format!("the opposing anti-air defence cannot be evaluated: {reason}"))?;
    let effectiveness =
        (defence.wrapping_mul(defines.anti_air[0]) / 100_000).min(defines.anti_air[1]);
    let side = builtin(
        icons,
        loc,
        if cas.attacking { ATTACKING } else { DEFENDING },
        &[],
    )?;
    let text = builtin(
        icons,
        loc,
        LAND_COMBAT_CAS_INFO_TOOLTIP,
        &[
            ("NUM", Argument::Int32(planes)),
            ("SIDE", Argument::Text(&side)),
            ("STR", Argument::Fixed5(cas.opponent.damage[0])),
            ("STR_MAX", Argument::Fixed5(strength)),
            ("ORG", Argument::Fixed5(cas.opponent.damage[1])),
            ("ORG_MAX", Argument::Fixed5(organisation)),
            ("KILL", Argument::Int32(cas.opponent.air_kills as i32)),
            ("STR_PREVENT", Argument::Fixed5(cas.opponent.damage[4])),
            ("ORG_PREVENT", Argument::Fixed5(cas.opponent.damage[5])),
            ("AA_EFFECTIVENESS", Argument::Fixed5(effectiveness)),
        ],
    )?;
    // The opponent's committed width bounds the planes that fit.
    let detail = widths.and_then(|width| {
        builtin(
            icons,
            loc,
            LAND_COMBAT_CAS_INFO_TOOLTIP_DETAIL,
            &[(
                "MAX_PLANES",
                Argument::Int32(super::fixed::round(super::fixed::mul(
                    width,
                    defines.planes_per_width,
                ))),
            )],
        )
    });
    Ok((text, detail))
}

/// combat width hover for the combat's province, or the operand
/// or key that failed.
fn width_tooltip(document: &Document, province: u32) -> Result<String, String> {
    let (icons, loc) = (&document.icons.combat, &document.names.localisation);
    let terrain = document
        .counters
        .combat_terrain
        .get(&province)
        .ok_or_else(|| format!("province {province} has no terrain category in the content"))?;
    let [width, support] = document
        .counters
        .combat_widths(terrain)
        .ok_or_else(|| format!("terrain {terrain} is not a content terrain category"))?;
    let mut text = builtin(icons, loc, COMBAT_COMBAT_WIDTH_DESC, &[])?;
    text.push_str("\n\n");
    let terrain_name = token(icons, loc, terrain.as_bytes())?;
    text.push_str(&builtin(
        icons,
        loc,
        HEADER,
        &[("KEY", Argument::Text(&terrain_name))],
    )?);
    text.push_str(&builtin(
        icons,
        loc,
        TERRAIN_COMBAT_WIDTH,
        &[("VALUE", Argument::Fixed5(*width))],
    )?);
    text.push('\n');
    text.push_str(&builtin(
        icons,
        loc,
        TERRAIN_ADDITIONAL_WIDTH,
        &[("VALUE", Argument::Fixed5(*support))],
    )?);
    Ok(text)
}

pub(super) fn bind(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    selection: Selection,
    vision: &Vision,
) -> io::Result<()> {
    if let Selection::Naval(id) = selection {
        return naval_hovers(out, document, frame, id, vision);
    }
    let Selection::Land(id) = selection else {
        return Ok(());
    };
    let assets = &document.icons.combat;
    let mut refused = BTreeMap::new();
    let detail_rows = rows::<tables::LandCombatDetails>(frame)?;
    let Some(detail) = detail_rows.rows().iter().find(|row| row.combat == id) else {
        out.warnings.push(format!(
            "Land combat {id}: no LandCombatDetails row, so the tactic, commander skill, HQ badge, CAS, balance and combat width hovers are omitted."
        ));
        refuse_hovers(
            &mut out.values,
            [
                "tactics_list_button",
                "combat_width",
                "combat_progress",
                "land_battle_progress_frame",
            ],
        );
        for prefix in ["attacker", "defender"] {
            refuse_hovers(&mut out.values, side_hovers(prefix));
        }
        for (key, count) in refused {
            out.warnings.push(format!(
                "Land combat {id}: {count} hovers are omitted: {key}."
            ));
        }
        return Ok(());
    };
    let (icons, loc) = (assets, &document.names.localisation);
    let list = counted_hover(builtin(icons, loc, TACTICS_LIST_DESC, &[]), &mut refused);
    set_hover(&mut out.values, "tactics_list_button", list, None);
    let width = match width_tooltip(document, detail.province) {
        Ok(text) => hover_text(&text),
        Err(reason) => {
            out.warnings.push(format!(
                "Land combat {id}: the combat width hover is omitted: {reason}."
            ));
            None
        }
    };
    set_hover(&mut out.values, "combat_width", width, None);
    let links = rows::<tables::CountryLinks>(frame)?;
    let divisions = rows::<tables::LandCombatDivisions>(frame)?;
    let divisions: BTreeMap<_, _> = divisions
        .rows()
        .iter()
        .filter(|row| row.combat == id)
        .map(|row| (row.division, row))
        .collect();
    let conditions = conditions(frame)?;
    let condition_gap = condition_gap(frame)?;
    let outlook = (|| {
        let combat_rows = rows::<tables::LandCombats>(frame).map_err(|why| why.to_string())?;
        let combat = combat_rows
            .rows()
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("LandCombats lacks combat {id}"))?;
        let side_rows = rows::<tables::LandCombatSides>(frame).map_err(|why| why.to_string())?;
        let sides = side_rows
            .rows()
            .iter()
            .find(|row| row.combat == id)
            .ok_or_else(|| format!("LandCombatSides lacks combat {id}"))?;
        let aliases = links
            .rows()
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let member = super::land_marker_tooltips::membership(sides, vision.ours(), &aliases)?;
        let scores = super::land_details::side_scores(
            detail,
            divisions.values().copied(),
            &conditions,
            [&combat.attackers, &combat.defenders],
            document.counters.defines.land_combat.scores,
        )
        .map_err(|why| format!("survival estimate: {why:?}"))?;
        super::land_marker_tooltips::outlook(icons, loc, scores, member)
    })()
    .map_err(|why| format!("Land combat {id} balance hover is omitted: {why}"));
    land_progress_hovers(&mut out.values, icons, loc, outlook, &mut refused);
    // Only when the skill levels depend on the modifier registry: read
    // again with the recorded one; a condition it cannot settle refuses.
    let settled;
    let (skills, levels) = if assets.tooltips.skills.conditions().is_empty() {
        (&assets.tooltips.skills, Ok(()))
    } else {
        let definitions = super::modifier_text::Definitions::read(frame, &document.modifier_names)?;
        settled = assets.commander_tooltips.settled(definitions.as_ref());
        let levels = super::modifier_text::Definitions::settle(
            definitions.as_ref(),
            settled.skills.conditions(),
        );
        (&settled.skills, levels)
    };
    for (index, prefix) in ["attacker", "defender"].into_iter().enumerate() {
        let side = &detail.sides[index];
        let opponent = &detail.sides[1 - index];
        // The tactic and its countered overlay share the builder and both of
        // its slots, for the attacker and the defender alike.
        if side.tactic_kind == 1 && loc.get(NULL_TACTIC_NAME).is_none() {
            out.warnings.push(format!(
                "Land combat {id} {prefix} tactic: the content has no localisation {NULL_TACTIC_NAME}, used by the null tactic name; the game shows the key itself, and so does the hover."
            ));
        }
        let (text, description) = match tactic_tooltip(
            icons,
            loc,
            &assets.tactics,
            side,
            opponent,
            index == 0,
        ) {
            Ok((text, description)) => {
                let description = match description {
                    Some(Ok(description)) => hover_text(&description),
                    Some(Err(reason)) => {
                        out.warnings.push(format!(
                                "Land combat {id} {prefix} tactic tooltip: its delayed description is omitted: {reason}."
                            ));
                        None
                    }
                    None => None,
                };
                (hover_text(&text), description)
            }
            Err(reason) => {
                out.warnings.push(format!(
                    "Land combat {id} {prefix} tactic tooltip is omitted: {reason}."
                ));
                (None, None)
            }
        };
        for suffix in ["tactic", "countered_tactic_overlay"] {
            set_hover(
                &mut out.values,
                format!("{prefix}_{suffix}"),
                text.clone(),
                description.clone(),
            );
        }
        if side.commander == 0 {
            // No commander: the game builds no skill or badge hover.
            refuse_hovers(
                &mut out.values,
                ["leader_skill", "leader_skill_bg", "leader_army_hq_icon"]
                    .map(|suffix| format!("{prefix}_{suffix}")),
            );
        } else {
            // the game shows the progress only for the viewing country's
            // commander (a country equality): a link this hour cannot
            // resolve decides neither.
            let skill = match crate::table_access::country_slot(&links, side.commander_country) {
                Ok(slot) => skill_tooltip(
                    icons,
                    loc,
                    side.commander_skill,
                    side.commander_experience,
                    skills.next_required_experience(side.commander_skill, side.commander_category),
                    slot == Some(vision.ours()),
                    levels.as_ref().copied().map_err(String::as_str),
                ),
                Err(reason) => Err(format!(
                    "whether the commander is the viewing country's, whose commander alone shows the skill progress, is unknown: {reason}"
                )),
            };
            let skill = match skill {
                Ok(text) => hover_text(&text),
                Err(reason) => {
                    out.warnings.push(format!(
                        "Land combat {id} {prefix} leader skill tooltip is omitted: {reason}."
                    ));
                    None
                }
            };
            for suffix in ["leader_skill", "leader_skill_bg"] {
                set_hover(
                    &mut out.values,
                    format!("{prefix}_{suffix}"),
                    skill.clone(),
                    None,
                );
            }
            let badge = token(icons, loc, &side.commander_name_token)
                .map_err(|refusal| format!("the commander's recorded name: {refusal}"))
                .and_then(|name| {
                    builtin(
                        icons,
                        loc,
                        ARMY_HQ_BADGE_TOOLTIP,
                        &[("NAME", Argument::Text(&name))],
                    )
                });
            let badge = counted_hover(badge, &mut refused);
            set_hover(
                &mut out.values,
                format!("{prefix}_leader_army_hq_icon"),
                badge,
                None,
            );
        }
        let glow = format!("{prefix}_leader_initiative_overlay_glow");
        let text = if out.values.get(&glow).is_some_and(|widget| !widget.hidden) {
            counted_hover(
                builtin(icons, loc, LEADER_HAS_INITIATIVE, &[]),
                &mut refused,
            )
        } else {
            // A hidden glow carries no hover.
            None
        };
        set_hover(&mut out.values, glow, text, None);
        let defines = match &assets.land_defines {
            Ok(defines) => defines,
            Err(reason) => {
                out.warnings.push(format!(
                    "Land combat {id} {prefix} CAS tooltip is omitted: the land combat defines are unavailable ({reason})."
                ));
                refuse_hovers(
                    &mut out.values,
                    ["air_text", "air_count", "air_icon"]
                        .map(|suffix| format!("{prefix}_{suffix}")),
                );
                continue;
            }
        };
        let widths = opponent.committed.iter().try_fold(0_i64, |sum, unit| {
            let row = divisions.get(unit).ok_or_else(|| {
                format!("opposing committed division {unit} has no LandCombatDivisions row")
            })?;
            let width = division_width(row).ok_or_else(|| {
                format!("opposing committed division {unit}'s width modifiers have an unsupported eligibility callback")
            })?;
            Ok(sum.wrapping_add(width))
        });
        let cas = cas_tooltip(
            icons,
            loc,
            Cas {
                own: side,
                opponent,
                attacking: index == 0,
            },
            &conditions,
            &condition_gap,
            widths,
            defines,
        );
        let (text, detail) = match cas {
            Ok((text, detail)) => {
                let detail = match detail {
                    Ok(detail) => hover_text(&detail),
                    Err(reason) => {
                        out.warnings.push(format!(
                            "Land combat {id} {prefix} CAS tooltip: its delayed detail is omitted: {reason}."
                        ));
                        None
                    }
                };
                (hover_text(&text), detail)
            }
            Err(reason) => {
                out.warnings.push(format!(
                    "Land combat {id} {prefix} CAS tooltip is omitted: {reason}."
                ));
                (None, None)
            }
        };
        for suffix in ["air_text", "air_count", "air_icon"] {
            set_hover(
                &mut out.values,
                format!("{prefix}_{suffix}"),
                text.clone(),
                detail.clone(),
            );
        }
    }
    for (key, count) in refused {
        out.warnings.push(format!(
            "Land combat {id}: {count} hovers are omitted: {key}."
        ));
    }
    Ok(())
}

/// Naval progress and stat hovers dispatch side stats before checking
/// progress widgets.
fn naval_hovers(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
    vision: &Vision,
) -> io::Result<()> {
    let assets = &document.icons.combat;
    let loc = &document.names.localisation;
    let mut refused = BTreeMap::new();
    if assets
        .native_windows
        .gui_type(b"navalcombat_ship_entry")
        .is_some()
    {
        for row in &mut out.naval_entries {
            naval_pride_hover(&mut row.values, assets, loc, id, &mut refused);
        }
    }
    // both progress icons receive this same dynamic tooltip,
    // including the hidden icon. The localiser resolves the executable key
    // through the recorded content stack; an authored seed cannot replace it.
    naval_progress_hovers(&mut out.values, assets, loc, id, &mut refused);
    // the game uses the same callback for each text and its background.
    // Anti-air has no breakdown; the other stats append an operand-dependent
    // delayed body and are handled with their calculations separately.
    naval_anti_air_hovers(&mut out.values, assets, loc, id, &mut refused);
    for (reason, count) in refused {
        for _ in 0..count {
            out.refuse(reason.clone());
        }
    }
    naval_commander_hovers(out, document, frame, id, vision)?;
    naval_flag_hovers(out, document, frame, id, vision)?;
    naval_carrier_penalty_hovers(out, document, id)?;
    super::naval_summary_tooltips::bind(out, document, frame, id)?;
    super::naval_ship_tooltips::bind(out, document, frame, id)?;
    Ok(())
}

// Use the first authored shortcut, trimmed and converted into keycaps,
// then localise SHORTCUT and KEY.
fn shortcut_tooltip(gui: &Gui<'_>, shortcut: &[u8], path: &str) -> Result<String, String> {
    let shortcut = std::str::from_utf8(shortcut)
        .map_err(|_| format!("Combat button {path}'s shortcut is not UTF-8"))?;
    let glyphs = shortcut_keycaps(shortcut);
    if glyphs.is_empty() {
        return Ok(String::new());
    }
    // the game displays a missing localisation key itself.
    builtin(
        gui.icons,
        gui.loc,
        SHORTCUT,
        &[("KEY", Argument::Text(&glyphs))],
    )
}

pub(super) fn shortcut_keycaps(shortcut: &str) -> String {
    // Trim only these bytes and preserve component indices.
    let shortcut = match shortcut.trim_matches(['\n', '\t', '\r', ' ']) {
        "," => "COMMA",
        "." => "PERIOD",
        shortcut => shortcut,
    };
    if shortcut.is_empty() {
        return String::new();
    }
    if shortcut.len() < 3 {
        return format!("\u{13}{shortcut} ");
    }
    let mut glyphs = String::new();
    for (index, key) in shortcut.split('+').enumerate() {
        let key = key.trim_matches(['\n', '\t', '\r', ' ']);
        if !key.is_empty() {
            if index != 0 {
                glyphs.push('+');
            }
            glyphs.push('\u{13}');
            glyphs.push_str(key);
            glyphs.push(' ');
        }
    }
    glyphs
}

fn naval_anti_air_hovers(
    values: &mut Values,
    icons: &dyn TextIcons,
    loc: &Localisation,
    id: u64,
    refused: &mut BTreeMap<String, usize>,
) {
    for side in ["left", "right"] {
        let text = counted_hover(
            builtin(icons, loc, COMBAT_STAT_ANTI_AIR, &[]).map_err(|reason| {
                format!("Naval combat {id} {side} anti-air hover is omitted: {reason}")
            }),
            refused,
        );
        for key in ["stat_anti_air", "stat_anti_air_bg"] {
            set_hover(
                values,
                format!("header_container/{side}_wnd/{key}"),
                text.clone(),
                None,
            );
        }
    }
}

fn carrier_penalty_text(
    icons: &dyn TextIcons,
    loc: &Localisation,
    multiplier: i64,
    enemy_fleet: i32,
    minimum: i32,
) -> Result<String, String> {
    // executable argument names and order: PERC is a fixed5
    // actual sortie multiplier, then the weighted opposing count and floor.
    builtin(
        icons,
        loc,
        CARRIER_STACK_PENALTY_DESC,
        &[
            ("PERC", Argument::Fixed5(multiplier)),
            ("ENEMY_FLEET", Argument::Int32(enemy_fleet)),
            ("MINIMUM", Argument::Int32(minimum)),
        ],
    )
}

fn naval_carrier_penalty_hovers(
    out: &mut Bindings,
    document: &Document,
    id: u64,
) -> io::Result<()> {
    for (side, label) in ["attacker", "defender"].into_iter().enumerate() {
        let key = format!("header_container/carrier_penalty_{label}");
        let result = out.naval_penalties[side]
            .clone()
            .unwrap_or_else(|| Err("the carrier laurel was not evaluated".into()))
            .and_then(|(multiplier, enemy, minimum)| {
                carrier_penalty_text(
                    &document.icons.combat,
                    &document.names.localisation,
                    multiplier,
                    enemy,
                    minimum,
                )
            });
        match result {
            Ok(text) => set_hover(&mut out.values, key, hover_text(&text), None),
            Err(reason) => {
                set_hover(&mut out.values, key, None, None);
                out.refuse(format!(
                    "Naval combat {id} {label} carrier-penalty hover is withheld: {reason}."
                ));
            }
        }
    }
    Ok(())
}

fn naval_flag_country(
    detail: &replay::rows::NavalCombatDetailsRow,
    aliases: &BTreeMap<u32, u32>,
    viewer: u32,
) -> Result<u32, String> {
    // Both flag hovers select the name from the first attacker country,
    // regardless of the hovered flag. Only the delayed arguments swap sides.
    let members = &detail.sides[0].summary.countries;
    let Some(&first) = members.first() else {
        return Err("the side lists no country".into());
    };
    let side =
        super::tactics::viewing_side(members, "attacker", aliases, viewer).map_err(|_| {
            let missing = members
                .iter()
                .copied()
                .find(|id| !aliases.contains_key(id))
                .unwrap_or(first);
            format!("naval flag side country {missing} has no CountryLinks row")
        })?;
    if side == 0 {
        Ok(viewer)
    } else {
        aliases
            .get(&first)
            .copied()
            .ok_or_else(|| format!("first side country {first} has no CountryLinks row"))
    }
}

/// both flags' immediate line is the selected country's live
/// display name wrapped in the native H colour; the game builds its delayed
/// summary from the side history and source-token map.
fn naval_flag_hovers(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
    vision: &Vision,
) -> io::Result<()> {
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
        return Ok(());
    };
    let links = rows::<tables::CountryLinks>(frame)?;
    let countries = rows::<tables::Countries>(frame)?;
    let aliases: BTreeMap<_, _> = links
        .rows()
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let names = installed_content::country_names::CountryNames::read(frame, &document.names.localisation);
    let viewing_tag = super::text::viewing_tag(countries.rows(), vision);
    let tokens = crate::table_access::TokenNames::new(frame).map_err(|error| error.to_string());
    let sub_units = document.icons.combat.commander_tooltips.sub_units();
    for (side, label) in ["attacker", "defender"].into_iter().enumerate() {
        let key = format!("header_container/flag_{label}");
        let main = naval_flag_country(detail, &aliases, vision.ours())
            .and_then(|country| names.name(country))
            .and_then(|name| {
                super::binding::displayable(&document.icons.combat, format!("\u{11}H{name}\u{11}!"))
            });
        let delayed = (|| {
            let tokens = tokens.as_ref().map_err(ToString::to_string)?;
            let sub_units = sub_units.map_err(str::to_owned)?;
            super::naval_summary_tooltips::flag_description(
                &detail.sides[side],
                &detail.sides[1 - side],
                tokens,
                sub_units,
                viewing_tag,
                &document.icons.combat,
                &document.names.localisation,
            )
        })();
        let delayed_hover = match delayed {
            Ok(text) => hover_text(&text),
            Err(reason) => {
                out.refuse(format!(
                    "Naval combat {id} {label} flag delayed damage summary is withheld: {reason}."
                ));
                None
            }
        };
        match main {
            Ok(main) => set_hover(&mut out.values, key, hover_text(&main), delayed_hover),
            Err(reason) => {
                set_hover(&mut out.values, key, None, delayed_hover);
                out.refuse(format!(
                    "Naval combat {id} {label} flag hover is withheld: {reason}."
                ));
            }
        }
    }
    Ok(())
}

fn naval_pride_hover(
    values: &mut Values,
    icons: &dyn TextIcons,
    loc: &Localisation,
    id: u64,
    refused: &mut BTreeMap<String, usize>,
) {
    // ship icon hover dispatches the pride icon before its
    // stored member hover. A hidden pride widget has no hit target.
    if values.get("pride").is_some_and(|widget| !widget.hidden) {
        let text = counted_hover(
            builtin(icons, loc, NAVY_CONTAINS_PRIDE_OF_FLEET_WITHOUT_NAME, &[]).map_err(|reason| {
                format!("Naval combat {id} ship pride hover is withheld: {reason}")
            }),
            refused,
        );
        set_hover(values, "pride", text, None);
    }
}

fn land_progress_hovers(
    values: &mut Values,
    icons: &dyn TextIcons,
    loc: &Localisation,
    outlook: Result<String, String>,
    refused: &mut BTreeMap<String, usize>,
) {
    // both names share outlook and the delayed explanation.
    let main = counted_hover(outlook, refused);
    let delayed = counted_hover(builtin(icons, loc, COMBAT_PROGRESS_DESC, &[]), refused);
    for key in ["combat_progress", "land_battle_progress_frame"] {
        set_hover(values, key, main.clone(), delayed.clone());
    }
}

fn naval_progress_hovers(
    values: &mut Values,
    icons: &dyn TextIcons,
    loc: &Localisation,
    id: u64,
    refused: &mut BTreeMap<String, usize>,
) {
    // Naval survival scores are zero on either side. Neither 120% comparison
    // wins, and rounded duration is always zero days.
    let main = counted_hover(
        super::land_marker_tooltips::outlook(icons, loc, [0, 0], [false; 2])
            .map_err(|reason| format!("Naval combat {id} progress hover is omitted: {reason}")),
        refused,
    );
    let delayed = counted_hover(
        builtin(icons, loc, COMBAT_PROGRESS_DESC, &[]).map_err(|reason| {
            format!("Naval combat {id} delayed progress hover is omitted: {reason}")
        }),
        refused,
    );
    for key in ["progress_attacker", "progress_defender"] {
        set_hover(
            values,
            format!("header_container/{key}"),
            main.clone(),
            delayed.clone(),
        );
    }
}

/// the game builds the portrait's HEADER and description from the recorded
/// commander tokens, appending learning progress only for the viewing
/// country. It uses the same learning formatter as land portrait.
fn naval_commander_hovers(
    out: &mut Bindings,
    document: &Document,
    frame: &DecodedFrame,
    id: u64,
    vision: &Vision,
) -> io::Result<()> {
    let details = rows::<tables::NavalCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
        out.refuse(format!(
            "Naval combat {id} commander hovers are withheld: NavalCombatDetails has no combat row."
        ));
        return Ok(());
    };
    let links = rows::<tables::CountryLinks>(frame)?;
    let countries = rows::<tables::Countries>(frame)?;
    let viewing_tag = super::text::viewing_tag(countries.rows(), vision);
    let icons = &document.icons.combat;
    let loc = &document.names.localisation;
    for (index, name) in ["attacker", "defender"].into_iter().enumerate() {
        let key = format!("header_container/leader_{name}");
        let side = &detail.sides[index];
        if side.commander == 0 {
            set_hover(&mut out.values, key, None, None);
            continue;
        }
        let Some(commander) = side.commander_inputs.as_ref() else {
            set_hover(&mut out.values, key, None, None);
            out.refuse(format!("Naval combat {id} {name} commander hover is withheld: NavalCombatDetails has no commander inputs."));
            continue;
        };
        let owner = crate::table_access::country_slot(&links, commander.country)
            .map(|slot| slot == Some(vision.ours()));
        let tooltip = naval_commander_portrait(icons, loc, commander, owner, viewing_tag);
        match tooltip {
            Ok((main, delayed)) => set_hover(
                &mut out.values,
                key,
                hover_text(&main),
                hover_text(&delayed),
            ),
            Err(reason) => {
                set_hover(&mut out.values, key, None, None);
                out.refuse(format!(
                    "Naval combat {id} {name} commander hover is withheld: {reason}."
                ));
            }
        }
    }
    Ok(())
}

fn naval_commander_portrait(
    icons: &dyn TextIcons,
    loc: &Localisation,
    commander: &replay::rows::NavalCommanderInputs,
    own_country: Result<bool, String>,
    viewing_tag: Result<&str, &str>,
) -> Result<(String, String), String> {
    if let Err(reason) = &own_country
        && !commander.learning_traits.is_empty()
    {
        return Err(format!(
            "whether learning traits belong to the viewing country is unresolved: {reason}"
        ));
    }
    let progress = (own_country == Ok(true)).then_some((
        commander.learning_traits.as_slice(),
        commander.learning_threshold,
        viewing_tag,
    ));
    super::commander_tooltips::portrait(
        icons,
        loc,
        &commander.name,
        &commander.description,
        (commander.category, commander.skills[0]),
        progress,
        false,
    )
}

// Shared by the land skill builder and naval components.
pub(super) const LEADER_SKILL_DESC: &str = "LEADER_SKILL_DESC";

crate::combat::text::tooltip_keys! {
    // tooltip updates.
    SHORTCUT = "SHORTCUT",
    COMBAT_PROGRESS_DESC = "COMBAT_PROGRESS_DESC",
    COMBAT_STAT_ANTI_AIR = "combat_stat_anti_air",
    NAVY_CONTAINS_PRIDE_OF_FLEET_WITHOUT_NAME = "NAVY_CONTAINS_PRIDE_OF_FLEET_WITHOUT_NAME",
    CARRIER_STACK_PENALTY_DESC = "CARRIER_STACK_PENALTY_DESC",
    NO_TACTIC = "NO_TACTIC",
    TACTIC_CURRENTLY_COUNTERED = "TACTIC_CURRENTLY_COUNTERED",
    TACTICS_COMBATWIDTH = "TACTICS_COMBATWIDTH",
    TACTICS_MOVEMENTSPEED_ATTACKER = "TACTICS_MOVEMENTSPEED_ATTACKER",
    TACTICS_DAMAGE_ATTACKER = "TACTICS_DAMAGE_ATTACKER",
    TACTICS_ATTACKER_ORG_DAMAGE_MODIFIER = "TACTICS_ATTACKER_ORG_DAMAGE_MODIFIER",
    TACTICS_DAMAGE_DEFENDER = "TACTICS_DAMAGE_DEFENDER",
    TACTICS_DEFENDER_ORG_DAMAGE_MODIFIER = "TACTICS_DEFENDER_ORG_DAMAGE_MODIFIER",
    TACTICS_WIDTH_VALUE = "TACTICS_WIDTH_VALUE",
    TACTICS_VALUE_POSITIVE = "TACTICS_VALUE_POSITIVE",
    TACTICS_VALUE_NEGATIVE = "TACTICS_VALUE_NEGATIVE",
    TACTIC_DESC = "TACTIC_DESC",
    LEADER_SKILL_AT_CAP = "LEADER_SKILL_AT_CAP",
    LEADER_SKILL_WITH_PROGRESS = "LEADER_SKILL_WITH_PROGRESS",
    ATTACKING = "ATTACKING",
    DEFENDING = "DEFENDING",
    LAND_COMBAT_CAS_INFO_TOOLTIP = "LAND_COMBAT_CAS_INFO_TOOLTIP",
    LAND_COMBAT_CAS_INFO_TOOLTIP_DETAIL = "LAND_COMBAT_CAS_INFO_TOOLTIP_DETAIL",
    COMBAT_COMBAT_WIDTH_DESC = "COMBAT_COMBAT_WIDTH_DESC",
    TERRAIN_COMBAT_WIDTH = "TERRAIN_COMBAT_WIDTH",
    TERRAIN_ADDITIONAL_WIDTH = "TERRAIN_ADDITIONAL_WIDTH",
    TACTICS_LIST_DESC = "TACTICS_LIST_DESC",
    ARMY_HQ_BADGE_TOOLTIP = "ARMY_HQ_BADGE_TOOLTIP",
    LEADER_HAS_INITIATIVE = "LEADER_HAS_INITIATIVE",
}

#[cfg(test)]
mod tests;
