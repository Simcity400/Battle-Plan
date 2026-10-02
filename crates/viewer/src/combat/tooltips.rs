//! Land combat hover text rebuilt from simulation operands and matching content.
//!
//! Land dispatch: 1415C1A60; tactic text: 1415BC770 -> 1406B12D0; terrain
//! widths: 14142C180; commander skill: 140C0A4B0. Nothing here is read from
//! the recording client's interface or localised string caches.
use super::{
    HoverText, Selection, Values,
    binding::{Bindings, TextIcons, builtin, token},
    land_details::{Conditions, condition_gap, conditions, division_width},
    layout::{Node, string},
};
use crate::{document::Document, map::perspective::Vision, table_access::rows};
use installed_content::{
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

/// 141299190 rebuilds the six combat sums after tactic changes and load.
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
    // 141299340 scopes each side to its first linked-set country. An empty
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

/// 1406B3670 tests the other pointer, not whether it is a real tactic.
/// Kind 1 is a nonnull CNullCombatTactic (id 0); kind 0 alone is nullptr.
pub(super) fn countered(side: &LandCombatSideDetails, other: &LandCombatSideDetails) -> bool {
    side.tactic_kind != 0
        && other.tactic_kind != 0
        && side.tactic_counter_id == other.tactic_id as u32
}

/// CTooltip's text (1424B7400 trims `\n`, `\t`, `\r` and space from both
/// ends, not all Unicode whitespace). Nothing left is no hover, as the game
/// shows none for an empty string.
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

/// Fixed widget tooltip keys from their producers (1415C1A60, 1415BC770).
pub(crate) fn keys() -> impl Iterator<Item = std::borrow::Cow<'static, str>> {
    TOOLTIP_KEYS
        .iter()
        .chain(super::division_tooltips::TOOLTIP_KEYS)
        .chain(super::commander_tooltips::TOOLTIP_KEYS)
        .chain(super::details::TOOLTIP_KEYS)
        .copied()
        .chain([NULL_TACTIC_NAME, HEADER])
        .map(std::borrow::Cow::Borrowed)
        .chain(super::division_tooltips::description_keys().map(std::borrow::Cow::Owned))
}

/// Authored hovers from the same nodes the combat binder uses.
pub(crate) fn authored_assets(
    windows: &BTreeMap<String, Node>,
    localisation: &Localisation,
) -> crate::map::native_gui::Assets {
    fn collect(node: &Node, loc: &Localisation, assets: &mut crate::map::native_gui::Assets) {
        for child in &node.children {
            for field in ["pdx_tooltip", "pdx_tooltip_delayed"] {
                if let Some(key) = string(&child.items, field) {
                    assets.add_localised(loc, key.as_bytes());
                }
            }
            collect(child, loc, assets);
        }
    }
    let mut assets = crate::map::native_gui::Assets::default();
    for window in windows.values() {
        collect(window, localisation, &mut assets);
    }
    assets
}

/// The land hovers 1415C1A60 sets on every update of one side.
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

/// What the authored seed reads: the localiser, the painter's icons and the
/// combat windows an entry template names.
struct Gui<'a> {
    icons: &'a dyn TextIcons,
    loc: &'a Localisation,
    windows: &'a BTreeMap<String, Node>,
}

/// Authored `pdx_tooltip`/`pdx_tooltip_delayed` keys of every widget, the
/// text it is created with: a widget whose hovers a dynamic builder owns
/// keeps the builder's text or refusal. A key the localisation lacks, or
/// whose text the localiser or the painter cannot produce as the game does
/// ([`builtin`]), is counted in `refused` by key, immediate or delayed alike.
fn authored(
    gui: &Gui<'_>,
    node: &Node,
    path: &str,
    values: &mut Values,
    refused: &mut BTreeMap<String, usize>,
) {
    let (icons, loc) = (gui.icons, gui.loc);
    for child in &node.children {
        let key = if path.is_empty() {
            child.name.clone()
        } else {
            format!("{path}/{}", child.name)
        };
        let owned = values.get(&key).is_some_and(|widget| widget.hover_owned);
        if !owned && let Some(token) = string(&child.items, "pdx_tooltip") {
            values.entry(key.clone()).or_default().tooltip =
                counted_hover(builtin(icons, loc, token, &[]), refused);
        }
        if !owned && let Some(token) = string(&child.items, "pdx_tooltip_delayed") {
            values.entry(key.clone()).or_default().tooltip_description =
                counted_hover(builtin(icons, loc, token, &[]), refused);
        }
        authored(gui, child, &key, values, refused);
        if let Some(name) = values
            .get(&key)
            .and_then(|widget| widget.entry_template.clone())
        {
            let Some(template) = gui.windows.get(&name) else {
                *refused
                    .entry(format!(
                        "{key}'s entry template {name} is not in the combat interface files, so its entries' authored hovers are not bound"
                    ))
                    .or_default() += 1;
                continue;
            };
            if let Some(widget) = values.get_mut(&key) {
                for entry in &mut widget.entries {
                    authored(gui, template, "", entry, refused);
                }
            }
        }
    }
}

/// A hover's immediate text and, when it has one, its delayed text or why
/// that is absent.
type Hover = (String, Option<Result<String, String>>);

/// The name GetNullCombatTactic (1406B3BD0) constructs CNullCombatTactic
/// with (the string at 1427D02D8). CCombatTactic's constructor (1406B0A70)
/// stores its localisation at +0x30 (142230F20), which the name virtual
/// (vtable slot 1427D0100 -> 1406B35A0) returns. A top-level key the
/// localisation lacks is copied as its own text (142230940), so the game
/// then shows `nullCombatTactic`; the bind reports the missing key.
const NULL_TACTIC_NAME: &str = "nullCombatTactic";
pub(super) const HEADER: &str = "HEADER";

/// The tactic a side's hover describes.
#[derive(Clone, Copy, Debug)]
pub(super) enum Described<'a> {
    /// A content tactic: its recorded database id and token.
    Content { id: i32, token: &'a [u8] },
    /// CNullCombatTactic (recorded kind 1): not a null pointer, so 1415BC770
    /// takes the ordinary header, effect and delayed path with its
    /// constructor's name and values, which 1406B0A70 zeroes (+0x138..+0x160).
    Null,
}

/// 1415BC770: the immediate text, then `TACTIC_DESC` in the delayed slot
/// (its output+0x20). Only a null tactic pointer (kind 0) has `NO_TACTIC`
/// alone.
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
                "the recorded tactic kind {kind} is none of the null pointer (0), CNullCombatTactic (1) and a content tactic (2)"
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
        // CNullCombatTactic's six values are 0 (1406B0A70).
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
            // 1406B12D0 deliberately tests param3 for +160, unlike +148.
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

/// The CAS summary 1415C1A60 builds for a side's air text, count and icon:
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
    // The opponent's committed units' current strength and organisation
    // (CArmy vslots +0xF8/+0x120: +0x420, +0x428).
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
    // 141298090: the anti-air defence (141297FD0) times the reduction factor,
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
    // The opponent's committed width (141299610) bounds the planes that fit.
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

/// 14142C180's combat width hover for the combat's province, or the operand
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
    let Selection::Land(id) = selection else {
        return Ok(());
    };
    let assets = &document.icons.combat;
    let mut refused = BTreeMap::new();
    let gui = Gui {
        icons: assets,
        loc: &document.names.localisation,
        windows: &assets.windows,
    };
    match assets.windows.get("landcombatview") {
        Some(window) => authored(&gui, window, "", &mut out.values, &mut refused),
        None => out.warnings.push(format!(
            "Land combat {id}: the combat interface files declare no landcombatview window, so no authored hover is bound."
        )),
    }
    for (side, entries) in out.entries.iter_mut().enumerate() {
        let name = match side {
            0 => "division_combat_attacker_entry",
            1 => "division_combat_defender_entry",
            _ => "reserve_combat_entry",
        };
        match assets.windows.get(name) {
            Some(node) => {
                for (_, entry) in entries {
                    authored(&gui, node, "", entry, &mut refused);
                }
            }
            None if !entries.is_empty() => out.warnings.push(format!(
                "Land combat {id}: the combat interface files declare no {name} window, so {} entries' authored hovers are not bound.",
                entries.len()
            )),
            None => {}
        }
    }
    let detail_rows = rows::<tables::LandCombatDetails>(frame)?;
    let Some(detail) = detail_rows.rows().iter().find(|row| row.combat == id) else {
        out.warnings.push(format!(
            "Land combat {id}: no LandCombatDetails row, so the tactic, commander skill, HQ badge, CAS and combat width hovers are omitted."
        ));
        refuse_hovers(&mut out.values, ["tactics_list_button", "combat_width"]);
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
                "Land combat {id} {prefix} tactic: the content has no localisation {NULL_TACTIC_NAME}, which CNullCombatTactic's name (1406B3BD0) takes; the game shows the key itself (142230940), and so does the hover."
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
            // No commander: 1415C1A60 builds no skill or badge hover.
            refuse_hovers(
                &mut out.values,
                ["leader_skill", "leader_skill_bg", "leader_army_hq_icon"]
                    .map(|suffix| format!("{prefix}_{suffix}")),
            );
        } else {
            // 1415C1A60 shows the progress only for the viewing country's
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
                    "whether the commander is the viewing country's, whose commander alone shows the skill progress (1415C1A60), is unknown: {reason}"
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

crate::combat::text::tooltip_keys! {
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
    LEADER_SKILL_DESC = "LEADER_SKILL_DESC",
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
mod tests {
    use super::*;
    use crate::combat::binding::NoIcons;

    #[test]
    fn authored_tooltip_icons_follow_the_bound_nodes() {
        let items = installed_content::parser::parse(
            br#"name = w pdx_tooltip = OTHER
                buttonType = { name = b pdx_tooltip = MAIN pdx_tooltip_delayed = DELAYED }"#,
        )
        .unwrap()
        .items;
        let windows = [("w".into(), Node::new("containerWindowType", &items))]
            .into_iter()
            .collect();
        let localisation = loc(&[
            ("MAIN", "£main"),
            ("DELAYED", "£delayed"),
            ("OTHER", "£unused"),
        ]);
        let assets = authored_assets(&windows, &localisation);
        assert!(assets.sprites.contains(b"GFX_main".as_slice()));
        assert!(assets.sprites.contains(b"GFX_delayed".as_slice()));
        assert!(!assets.sprites.contains(b"GFX_unused".as_slice()));
    }

    /// The tactic registry read through the real parse
    /// (installed_content::combat_tactics), as the land hovers use it.
    fn tactics(text: &[u8]) -> Tactics {
        Tactics::from_database(installed_content::combat_tactics::parse(
            text,
            &installed_content::key_names::KeyNames::default(),
        ))
    }

    fn loc(entries: &[(&str, &str)]) -> Localisation {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    /// The hovers find the recorded tactic at its recorded ID (1406B0A70
    /// stores the database ID at +0x98), which must carry the recorded name;
    /// a repeated name is its own tactic, and a refused effect or an
    /// unproven entry names the tactic and why.
    #[test]
    fn recorded_tactics_find_their_registry_effects() {
        // Whether `x` names an ideology group is unknown while an ideology
        // file stops short.
        let mut names = installed_content::key_names::KeyNames::default();
        names.counts.ideologies_gap = Some("common/ideologies/a.txt is unknown from line 1".into());
        let registry = Tactics::from_database(installed_content::combat_tactics::parse(
            br#"@bonus = 0.25
                PHASES = { }
                tactic_a = { ATTACKER = @bonus Combat_Width = 0.1 attacker_movement_speed = 010 }
                tactic_b = { defender = @missing }
                tactic_a = { attacker = 0.5 }
                tactic_c = { defender_org_damage_modifier = 1.5e3 active = yes }
                tactic_d = { trigger = { x = { } } }"#,
            &names,
        ));
        use super::super::tactics::Effects;
        assert_eq!(
            registry.effects(2, b"tactic_a"),
            Ok(Effects {
                attacker: 25_000,
                width: 10_000,
                movement: 800_000,
                ..Effects::default()
            })
        );
        // The repeat is tactic 4, with its own values.
        assert_eq!(
            registry.effects(4, b"tactic_a").map(|t| t.attacker),
            Ok(50_000)
        );
        assert_eq!(registry.effects(1, b"PHASES"), Ok(Effects::default()));
        // 1424B8600 reads any text: an undefined reference is its own text
        // (0), and 'e' is a digit to it.
        assert_eq!(registry.effects(3, b"tactic_b"), Ok(Effects::default()));
        assert_eq!(
            registry.effects(5, b"tactic_c"),
            Ok(Effects {
                defender_org: 203_300,
                ..Effects::default()
            })
        );
        // A trigger block the port cannot settle: that tactic is unproven
        // and no later one is known.
        let unsettled = registry.effects(6, b"tactic_d").unwrap_err();
        assert!(
            unsettled.starts_with("tactic tactic_d's ID or fields cannot be read as the game reads them: combat_tactics.txt line 7: field `x`: LoadKey (1405434F0) reads it"),
            "{unsettled}"
        );
        let past = registry.effects(10, b"tactic_a").unwrap_err();
        assert!(
            past.starts_with("recorded tactic tactic_a's id 10 is past the 6 combat tactics read before combat_tactics.txt line 7"),
            "{past}"
        );
        assert_eq!(
            registry.effects(0, b"x").unwrap_err(),
            "recorded tactic x's id 0 is not a content tactic id"
        );
        assert_eq!(
            registry.effects(1, b"tactic_b"),
            Err("recorded tactic id 1 is tactic_b, but the content's tactic 1 is PHASES".into())
        );
    }

    fn tactic_a() -> Described<'static> {
        Described::Content {
            id: 1,
            token: b"tactic_a",
        }
    }

    /// CNullCombatTactic (kind 1) takes 1415BC770's ordinary path: the
    /// header of its constructor's localised name, no effect line (all zero,
    /// 1406B0A70), and TACTIC_DESC in the delayed slot; countered, the
    /// countered line.
    #[test]
    fn the_null_tactic_object_is_described_like_a_tactic() {
        let loc = loc(&[
            ("HEADER", "$KEY$\n"),
            ("nullCombatTactic", "No Tactic Chosen"),
            ("TACTIC_CURRENTLY_COUNTERED", "Countered"),
            ("TACTIC_DESC", "What tactics do"),
        ]);
        let content = tactics(b"");
        let null = LandCombatSideDetails {
            tactic_kind: 1,
            tactic_counter_id: 5,
            ..Default::default()
        };
        let other = LandCombatSideDetails {
            tactic_kind: 2,
            tactic_id: 5,
            ..Default::default()
        };
        assert_eq!(
            tactic_tooltip(
                &NoIcons,
                &loc,
                &content,
                &null,
                &LandCombatSideDetails::default(),
                true
            ),
            Ok((
                "No Tactic Chosen\n".into(),
                Some(Ok("What tactics do".into()))
            ))
        );
        assert_eq!(
            tactic_tooltip(&NoIcons, &loc, &content, &null, &other, true),
            Ok((
                "No Tactic Chosen\nCountered".into(),
                Some(Ok("What tactics do".into()))
            ))
        );
    }

    #[test]
    fn tactic_description_goes_to_the_delayed_slot() {
        let loc = loc(&[
            ("tactic_a", "Tactic A"),
            ("HEADER", "$KEY$\n"),
            ("TACTICS_DAMAGE_ATTACKER", "Attack"),
            ("TACTICS_VALUE_POSITIVE", "$VALUE|G=%0$"),
            ("TACTIC_CURRENTLY_COUNTERED", "Countered"),
            ("TACTIC_DESC", "What tactics do"),
            ("NO_TACTIC", "None"),
        ]);
        let content = tactics(b"tactic_a = { attacker = 0.2 }");
        assert_eq!(
            tactic_description(&NoIcons, &loc, &content, tactic_a(), true, false),
            Ok((
                "Tactic A\nAttack \u{11}G+20%\u{11}!\n".into(),
                Ok("What tactics do".into())
            ))
        );
        // The countered overlay shares both slots.
        assert_eq!(
            tactic_description(&NoIcons, &loc, &content, tactic_a(), true, true),
            Ok(("Tactic A\nCountered".into(), Ok("What tactics do".into())))
        );
        let none = LandCombatSideDetails::default();
        assert_eq!(
            tactic_tooltip(&NoIcons, &loc, &content, &none, &none, true),
            Ok(("None".into(), None))
        );
    }

    /// Every hover these builders cannot make names why, and a missing or
    /// undrawable delayed description leaves the immediate text standing.
    #[test]
    fn refused_hovers_name_their_reason() {
        let entries = |extra: &[(&str, &str)]| -> Localisation {
            [("tactic_a", "Tactic A"), ("HEADER", "$KEY$\n")]
                .iter()
                .chain(extra)
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect()
        };
        let content = tactics(b"tactic_a = { }");
        let missing =
            tactic_description(&NoIcons, &entries(&[]), &content, tactic_a(), true, false);
        assert!(
            // A missing top-level key shows its own bytes (142230940).
            matches!(&missing, Ok((_, Ok(text))) if text == "TACTIC_DESC"),
            "{missing:?}"
        );
        let flagged = entries(&[("TACTIC_DESC", "@GER")]);
        let flagged = tactic_description(&NoIcons, &flagged, &content, tactic_a(), true, false);
        assert!(
            matches!(&flagged, Ok((_, Err(reason))) if reason == "localisation TACTIC_DESC holds the inline flag 0x12 \"GER\" (an authored `@`), which the viewer does not draw")
        );
        let unknown = tactic_description(
            &NoIcons,
            &entries(&[]),
            &content,
            Described::Content {
                id: 1,
                token: b"tactic_b",
            },
            true,
            false,
        );
        assert_eq!(
            unknown,
            Err("recorded tactic id 1 is tactic_b, but the content's tactic 1 is tactic_a".into())
        );
        // CNullCombatTactic's name key, missing, shows as itself (142230940).
        let side = LandCombatSideDetails {
            tactic_kind: 1,
            ..Default::default()
        };
        let described = entries(&[("TACTIC_DESC", "What tactics do")]);
        assert_eq!(
            tactic_tooltip(
                &NoIcons,
                &described,
                &content,
                &side,
                &LandCombatSideDetails::default(),
                true
            ),
            Ok((
                "nullCombatTactic
"
                .into(),
                Some(Ok("What tactics do".into()))
            ))
        );
        let odd = LandCombatSideDetails {
            tactic_kind: 3,
            ..Default::default()
        };
        assert!(
            tactic_tooltip(&NoIcons, &entries(&[]), &content, &odd, &odd, true)
                .unwrap_err()
                .starts_with("the recorded tactic kind 3")
        );
        let loc = entries(&[]);
        assert!(skill_tooltip(&NoIcons, &loc, 3, 0, None, true, Ok(())).is_err());
        assert!(
            skill_tooltip(&NoIcons, &loc, 3, 0, Some(Some(0)), true, Ok(()))
                .unwrap()
                .contains("LEADER_SKILL_WITH_PROGRESS")
        );
        // A missing top-level key shows its own bytes (142230940).
        assert!(
            skill_tooltip(&NoIcons, &loc, 3, 0, Some(None), false, Ok(()))
                .unwrap()
                .contains("LEADER_SKILL_DESC")
        );
        assert_eq!(
            skill_tooltip(
                &NoIcons,
                &loc,
                3,
                0,
                Some(None),
                true,
                Err("x is registered")
            ),
            Err("the content's leader skill levels are unknown: x is registered".into())
        );
    }

    /// The authored `pdx_tooltip` is the text a widget is created with: a
    /// dynamic builder's text, or its refusal, replaces it whichever binds
    /// first (1415C1A60 sets its hovers on every update), and an unowned
    /// widget keeps the seed.
    #[test]
    fn authored_seeds_never_replace_a_dynamic_hover_or_its_refusal() {
        let document = installed_content::parser::parse(
            br#"containerWindowType = { name = view
                buttonType = { name = dynamic pdx_tooltip = AUTHORED }
                buttonType = { name = refused pdx_tooltip = AUTHORED pdx_tooltip_delayed = AUTHORED }
                buttonType = { name = plain pdx_tooltip = AUTHORED }
            }"#,
        )
        .unwrap();
        let window = Node::new(
            "containerWindowType",
            document.items[0].value.items().unwrap(),
        );
        let loc = loc(&[("AUTHORED", "Authored"), ("DYNAMIC", "Dynamic")]);
        let windows = BTreeMap::new();
        let gui = Gui {
            icons: &NoIcons,
            loc: &loc,
            windows: &windows,
        };
        let mut values = Values::new();
        set_hover(&mut values, "dynamic", hover_text("Dynamic"), None);
        let mut refused = BTreeMap::new();
        authored(&gui, &window, "", &mut values, &mut refused);
        // A refusal after the seed clears it too.
        refuse_hovers(&mut values, ["refused"]);
        let hover = |key: &str| {
            let widget = &values[key];
            (
                widget.tooltip.as_deref().map(str::to_owned),
                widget.tooltip_description.as_deref().map(str::to_owned),
            )
        };
        assert_eq!(hover("dynamic"), (Some("Dynamic".into()), None));
        assert_eq!(hover("refused"), (None, None));
        assert_eq!(hover("plain"), (Some("Authored".into()), None));
        assert!(refused.is_empty());
    }

    #[test]
    fn hover_text_trims_only_the_four_ascii_bytes() {
        assert_eq!(hover_text(" \r\n\tname\n ").as_deref(), Some("name"));
        assert_eq!(
            hover_text("\u{a0}name\u{a0}").as_deref(),
            Some("\u{a0}name\u{a0}")
        );
        assert_eq!(hover_text(" \r\n\t"), None);
    }

    #[test]
    fn null_tactic_object_can_counter_but_null_pointer_cannot() {
        let side = LandCombatSideDetails {
            tactic_kind: 2,
            tactic_counter_id: 0,
            ..Default::default()
        };
        let mut other = LandCombatSideDetails {
            tactic_kind: 1,
            tactic_id: 0,
            ..Default::default()
        };
        assert!(countered(&side, &other));
        other.tactic_kind = 0;
        assert!(!countered(&side, &other));
    }

    #[test]
    fn organisation_effects_follow_active_tactics_and_validate_damage_sums() {
        let content = tactics(
            b"tactic_a = { attacker = 0.2 attacker_org_damage_modifier = 0.15 \
              defender_org_damage_modifier = -0.05 }",
        );
        let mut detail = LandCombatDetailsRow {
            combat: 42,
            attacker_bonus: 20_000,
            ..Default::default()
        };
        let mut sides = LandCombatSideRow {
            combat: 42,
            attacker_primary: vec![1],
            defender_primary: vec![2],
            ..Default::default()
        };
        detail.sides[0].tactic_kind = 2;
        detail.sides[0].tactic_id = 1;
        detail.sides[0].tactic_token = b"tactic_a".to_vec();
        assert_eq!(
            organisation_effects(&content, &detail, Some(&sides)),
            Ok([15_000, -5_000])
        );
        detail.attacker_bonus = 0;
        assert!(
            organisation_effects(&content, &detail, Some(&sides))
                .unwrap_err()
                .contains("damage sums")
        );
        detail.sides[1].tactic_kind = 1;
        assert_eq!(
            organisation_effects(&content, &detail, Some(&sides)),
            Ok([0, 0])
        );
        assert_eq!(
            organisation_effects(&content, &detail, None),
            Err("the combat's LandCombatSides row is missing".into())
        );
        sides.defender_primary.clear();
        sides.defender_secondary.push(2);
        assert_eq!(
            organisation_effects(&content, &detail, Some(&sides)),
            Err("the defender's primary country set is empty".into())
        );
        sides.defender_primary.push(0);
        assert_eq!(
            organisation_effects(&content, &detail, Some(&sides)),
            Err("the defender's first primary country is null".into())
        );
        sides.defender_primary[0] = 2;
        sides.combat = 43;
        assert!(organisation_effects(&content, &detail, Some(&sides)).is_err());
    }

    #[test]
    fn cas_reports_the_opponents_air_defence_against_our_planes() {
        let loc = loc(&[
            ("ATTACKING", "attacking"),
            (
                "LAND_COMBAT_CAS_INFO_TOOLTIP",
                "$NUM$ $SIDE$ $STR|1$/$STR_MAX|1$ $ORG|1$/$ORG_MAX|1$ $KILL$ $STR_PREVENT|1$ $ORG_PREVENT|1$ $AA_EFFECTIVENESS|%0$",
            ),
            ("LAND_COMBAT_CAS_INFO_TOOLTIP_DETAIL", "max $MAX_PLANES$"),
        ]);
        let own = LandCombatSideDetails {
            air_counts: vec![100, 50],
            ..Default::default()
        };
        let opponent = LandCombatSideDetails {
            committed: vec![7],
            damage: [120_000, 350_000, 0, 0, 10_000, 20_000],
            air_kills: 3,
            ..Default::default()
        };
        let conditions = Conditions::from([(
            7,
            super::super::land_details::Condition {
                strength: 900_000,
                organisation: 6_000_000,
                ..Default::default()
            },
        )]);
        let defines = super::super::defines::LandDefines {
            air_superiority: [62_500_000, 75_000],
            anti_air: [75_000, 70_000],
            planes_per_width: 300_000,
            ..Default::default()
        };
        let cas = || Cas {
            own: &own,
            opponent: &opponent,
            attacking: true,
        };
        let gap = |unit: u64| {
            format!(
                "division {unit}'s condition is unknown: the DivisionStatus table has no row for it"
            )
        };
        // No anti-air: zero effectiveness; the width of 40 fits 120 planes.
        assert_eq!(
            cas_tooltip(
                &NoIcons,
                &loc,
                cas(),
                &conditions,
                &gap,
                Ok(4_000_000),
                &defines
            ),
            Ok((
                "150 attacking 1.2/9.0 3.5/60.0 3 0.1 0.2 0%".into(),
                Ok("max 120".into())
            ))
        );
        // The refusal names the opposing division and the table that lacks it.
        assert_eq!(
            cas_tooltip(&NoIcons, &loc, cas(), &Conditions::new(), &gap, Ok(0), &defines),
            Err("opposing committed division 7's condition is unknown: the DivisionStatus table has no row for it".into())
        );
        // A missing executable key shows its own bytes (142230940).
        let bare: Localisation = [("ATTACKING".to_owned(), "attacking".to_owned())]
            .into_iter()
            .collect();
        assert_eq!(
            cas_tooltip(&NoIcons, &bare, cas(), &conditions, &gap, Ok(0), &defines),
            Ok((
                "LAND_COMBAT_CAS_INFO_TOOLTIP".into(),
                Ok("LAND_COMBAT_CAS_INFO_TOOLTIP_DETAIL".into())
            ))
        );
    }
}
