//! XUI17 character, unit-leader, trait-tree, and ace production boundary.
//!
//! Exact live identity/stats come from replay_core's authenticated domain-47
//! and domain-46 sidecar views. Runtime presentation is a separate producer
//! input and is admitted only when it covers the live roster bijectively. This
//! module never treats recorder array order as screen order. Commander
//! assignment identity comes from the exact d28 OOB join, and experience is
//! exposed as an exact fixed-point rational using the effective game-file
//! skill cost rather than being recaptured as presentation data.

use crate::assets::AssetResolver;
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, LayoutSet};
use crate::paradox_text::{parse_file, Document};
use crate::{GameUiError, Result};
use replay_core::{
    AceRosterExactView, CharacterPresentationExactView, CharacterRosterExactView,
    CharacterTraitNodeState, UnitLeaderExactRow, ViewStatus,
};
use std::collections::{BTreeMap, BTreeSet};

pub const ARMY_LEADER_GUI: &str = "interface/unitleaderwindow.gui";
pub const ARMY_LEADER_WINDOW: &str = "armyleaderwindow";
pub const ARMY_LEADER_ROW: &str = "armyleaderentry";
pub const NAVY_LEADER_GUI: &str = "interface/navyleaderview.gui";
pub const NAVY_LEADER_WINDOW: &str = "navy_leader_window";
pub const NAVY_LEADER_ROW: &str = "navy_leader_portrait_window";
pub const ACES_GUI: &str = "interface/acesview.gui";
pub const ACES_WINDOW: &str = "acesview";
pub const ACE_ROW: &str = "acesview_entry";
pub const ARMY_TRAIT_TREE_GUI: &str = "interface/armytraittreewindow.gui";
pub const ARMY_TRAIT_TREE_WINDOW: &str = "unitleader_trait_tree_window";
pub const NAVY_TRAIT_TREE_GUI: &str = "interface/navytraittreewindow.gui";
pub const NAVY_TRAIT_TREE_WINDOW: &str = "navyleader_trait_tree_window";
pub const TRAIT_TREE_COMMON_GUI: &str = "interface/traittreewindow_common.gui";
pub const TRAIT_TREE_ROW: &str = "unit_leader_trait";
pub const LEADER_SKILLS_FILE: &str = "common/unit_leader/00_skills.txt";
pub const LEADER_EXPERIENCE_SCALE: i64 = 100_000;

#[derive(Clone, Copy)]
struct LayoutSpec {
    file: &'static str,
    element: &'static str,
    slots: &'static [&'static str],
}

const LAYOUT_SPECS: &[LayoutSpec] = &[
    LayoutSpec {
        file: ARMY_LEADER_GUI,
        element: ARMY_LEADER_WINDOW,
        slots: &["header_text", "leaders_grid", "close_button"],
    },
    LayoutSpec {
        file: ARMY_LEADER_GUI,
        element: ARMY_LEADER_ROW,
        slots: &["leader_button", "name", "skill", "traits_grid"],
    },
    LayoutSpec {
        file: NAVY_LEADER_GUI,
        element: NAVY_LEADER_WINDOW,
        slots: &["new_fleet_window", "portrait_btn", "fleets_box"],
    },
    LayoutSpec {
        file: NAVY_LEADER_GUI,
        element: NAVY_LEADER_ROW,
        slots: &["portrait_btn", "portrait_frame", "taskforce_count"],
    },
    LayoutSpec {
        file: ACES_GUI,
        element: ACES_WINDOW,
        slots: &["title", "close_button", "grid"],
    },
    LayoutSpec {
        file: ACES_GUI,
        element: ACE_ROW,
        slots: &[
            "ace_btn",
            "name",
            "modifiers",
            "current_wing_equipment_icon",
            "current_wing_name",
        ],
    },
    LayoutSpec {
        file: ARMY_TRAIT_TREE_GUI,
        element: ARMY_TRAIT_TREE_WINDOW,
        slots: &[
            "unit_leader",
            "name",
            "active_traits",
            "regular_traits",
            "tree",
        ],
    },
    LayoutSpec {
        file: NAVY_TRAIT_TREE_GUI,
        element: NAVY_TRAIT_TREE_WINDOW,
        slots: &[
            "unit_leader",
            "name",
            "active_traits",
            "regular_traits",
            "tree",
        ],
    },
    LayoutSpec {
        file: TRAIT_TREE_COMMON_GUI,
        element: TRAIT_TREE_ROW,
        slots: &["disabled_bg", "pickable_bg", "picked_bg", "icon", "name"],
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterLayoutGate {
    pub source_file: String,
    pub element: String,
    pub source_present: bool,
    pub parse_recoveries: usize,
    pub element_found: bool,
    pub slots_missing: Vec<String>,
}

impl CharacterLayoutGate {
    pub fn complete(&self) -> bool {
        self.source_present
            && self.parse_recoveries == 0
            && self.element_found
            && self.slots_missing.is_empty()
    }
}

fn descendant_names(element: &GuiElement, names: &mut BTreeSet<String>) {
    names.insert(element.name.to_ascii_lowercase());
    for child in &element.children {
        descendant_names(child, names);
    }
}

/// Parse the nine exact window/template roots through the mod-aware resolver.
pub fn inspect_character_layouts(resolver: &AssetResolver) -> Result<Vec<CharacterLayoutGate>> {
    let mut parsed: BTreeMap<&str, Option<LayoutSet>> = BTreeMap::new();
    for spec in LAYOUT_SPECS {
        if parsed.contains_key(spec.file) {
            continue;
        }
        let layout = match resolver.resolve_optional_file(spec.file)? {
            Some(file) => {
                let mut layout = LayoutSet::default();
                layout.parse_file(&file.path, &file.rel_path)?;
                Some(layout)
            }
            None => None,
        };
        parsed.insert(spec.file, layout);
    }
    Ok(LAYOUT_SPECS
        .iter()
        .map(|spec| {
            let Some(layout) = parsed.get(spec.file).and_then(Option::as_ref) else {
                return CharacterLayoutGate {
                    source_file: spec.file.to_string(),
                    element: spec.element.to_string(),
                    source_present: false,
                    parse_recoveries: 0,
                    element_found: false,
                    slots_missing: spec.slots.iter().map(|slot| slot.to_string()).collect(),
                };
            };
            let found = layout.find_by_name(spec.element).into_iter().next();
            let mut names = BTreeSet::new();
            if let Some(element) = found {
                descendant_names(element, &mut names);
            }
            CharacterLayoutGate {
                source_file: spec.file.to_string(),
                element: spec.element.to_string(),
                source_present: true,
                parse_recoveries: layout.stats.recovery_count,
                element_found: found.is_some(),
                slots_missing: spec
                    .slots
                    .iter()
                    .filter(|slot| !names.contains(&slot.to_ascii_lowercase()))
                    .map(|slot| slot.to_string())
                    .collect(),
            }
        })
        .collect())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaderPortraitContext {
    Army,
    Navy,
}

#[derive(Clone, Debug, Default)]
struct PortraitChoices(BTreeMap<String, BTreeSet<String>>);

/// Exact `common/characters/*.txt portraits.<context>.large` declarations.
#[derive(Clone, Debug, Default)]
pub struct CharacterPortraitCatalog {
    by_token: BTreeMap<String, PortraitChoices>,
}

impl CharacterPortraitCatalog {
    pub fn add_document(&mut self, document: &Document) {
        for characters in document.root.values_ci("characters") {
            for character in characters.fields() {
                let Some(token) = character.key.as_deref() else {
                    continue;
                };
                let Some(portraits) = character.value.first_value_ci("portraits") else {
                    continue;
                };
                for context in portraits.fields() {
                    let (Some(context), Some(gfx)) = (
                        context.key.as_deref(),
                        context.value.first_scalar_ci("large"),
                    ) else {
                        continue;
                    };
                    if !gfx.is_empty() {
                        self.by_token
                            .entry(token.to_string())
                            .or_default()
                            .0
                            .entry(context.to_ascii_lowercase())
                            .or_default()
                            .insert(gfx.to_string());
                    }
                }
            }
        }
    }

    pub fn load(resolver: &AssetResolver) -> Result<Self> {
        let mut out = Self::default();
        for file in resolver.list_files_recursive("common/characters", Some("txt"))? {
            out.add_document(&parse_file(&file.path)?);
        }
        Ok(out)
    }

    pub fn token_count(&self) -> usize {
        self.by_token.len()
    }

    pub fn resolve(
        &self,
        token: &str,
        context: LeaderPortraitContext,
        sprites: &SpriteRegistry,
    ) -> Option<(String, String)> {
        let choices = &self.by_token.get(token)?.0;
        let context = match context {
            LeaderPortraitContext::Army => "army",
            LeaderPortraitContext::Navy => "navy",
        };
        let gfx = if let Some(keys) = choices.get(context) {
            (keys.len() == 1)
                .then(|| keys.iter().next().map(String::as_str))
                .flatten()?
        } else {
            let unique: BTreeSet<&str> = choices
                .values()
                .flat_map(|keys| keys.iter().map(String::as_str))
                .collect();
            (unique.len() == 1)
                .then(|| unique.iter().next().copied())
                .flatten()?
        };
        let sprite = sprites.get(gfx)?;
        Some((gfx.to_string(), sprite.primary_texture()?.to_string()))
    }
}

/// Resolve the documented ace name+surname override before any random
/// country/culture/gender pool selection. No character normalization is
/// guessed: only the literal d46 name and surname components are tried.
pub fn resolve_named_ace_portrait(
    country_tag: &str,
    name: &str,
    surname: &str,
    sprites: &SpriteRegistry,
) -> Option<(String, String)> {
    if country_tag.is_empty() || name.is_empty() || surname.is_empty() {
        return None;
    }
    let country_specific = format!("GFX_{country_tag}_ace_{name}_{surname}");
    let generic = format!("GFX_ace_{name}_{surname}");
    for gfx in [country_specific, generic] {
        if let Some(sprite) = sprites.get(&gfx) {
            if let Some(texture) = sprite.primary_texture() {
                return Some((gfx, texture.to_string()));
            }
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LeaderSkillKind {
    Army,
    Navy,
}

/// Effective `leader_skills` costs keyed by engine leader kind and current
/// skill. Corps-commander and field-marshal costs must agree because d47's
/// army leader kind intentionally does not invent a rank.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LeaderSkillCostCatalog {
    costs: BTreeMap<(LeaderSkillKind, i64), i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderExperienceExact {
    pub experience_raw: i64,
    pub fixed_scale: i64,
    pub level_cost: i64,
    pub level_cost_raw: i64,
}

impl LeaderSkillCostCatalog {
    pub fn add_document(&mut self, document: &Document) -> Result<()> {
        if !document.recoveries.is_empty() {
            return Err(invalid("leader skill file required parser recovery"));
        }
        for skills in document.root.values_ci("leader_skills") {
            for entry in skills.fields() {
                let Some(level) = entry.key.as_deref().and_then(|key| key.parse::<i64>().ok())
                else {
                    continue;
                };
                let Some(kind) = entry.value.first_scalar_ci("type") else {
                    continue;
                };
                let Some(cost) = entry
                    .value
                    .first_scalar_ci("cost")
                    .and_then(|value| value.parse::<i64>().ok())
                else {
                    return Err(invalid(format!(
                        "leader skill {kind}/{level} has no integer cost"
                    )));
                };
                if cost <= 0 {
                    return Err(invalid(format!(
                        "leader skill {kind}/{level} has non-positive cost"
                    )));
                }
                let kind = match kind.to_ascii_lowercase().as_str() {
                    "navy" => Some(LeaderSkillKind::Navy),
                    "corps_commander" | "field_marshal" => Some(LeaderSkillKind::Army),
                    _ => None,
                };
                let Some(kind) = kind else { continue };
                match self.costs.insert((kind, level), cost) {
                    Some(previous) if previous != cost => {
                        return Err(invalid(format!(
                            "conflicting leader skill costs for {kind:?}/{level}: {previous} and {cost}"
                        )))
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    pub fn load(resolver: &AssetResolver) -> Result<Self> {
        let file = resolver
            .resolve_optional_file(LEADER_SKILLS_FILE)?
            .ok_or_else(|| invalid(format!("{LEADER_SKILLS_FILE} is absent")))?;
        let mut catalog = Self::default();
        catalog.add_document(&parse_file(&file.path)?)?;
        if catalog.costs.is_empty() {
            return Err(invalid("leader skill cost catalog is empty"));
        }
        Ok(catalog)
    }

    pub fn cost(&self, kind: LeaderSkillKind, level: i64) -> Option<i64> {
        self.costs.get(&(kind, level)).copied()
    }

    pub fn exact_experience(&self, row: &UnitLeaderExactRow) -> Result<LeaderExperienceExact> {
        let kind = match row.leader_kind.as_str() {
            "army" => LeaderSkillKind::Army,
            "navy" => LeaderSkillKind::Navy,
            other => return Err(invalid(format!("unsupported leader kind {other}"))),
        };
        let level_cost = self.cost(kind, row.skill).ok_or_else(|| {
            invalid(format!(
                "no effective leader skill cost for {kind:?}/{}",
                row.skill
            ))
        })?;
        let level_cost_raw = level_cost
            .checked_mul(LEADER_EXPERIENCE_SCALE)
            .ok_or_else(|| invalid("leader experience denominator overflow"))?;
        Ok(LeaderExperienceExact {
            experience_raw: row.experience_raw,
            fixed_scale: LEADER_EXPERIENCE_SCALE,
            level_cost,
            level_cost_raw,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraitNodeState {
    Picked,
    Pickable,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitNodePresentation {
    pub trait_token: String,
    pub state: TraitNodeState,
    /// `None` is explicit producer unavailability and must not render as zero.
    pub progress_raw: Option<i64>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderPresentationInput {
    pub unit_leader_id: u64,
    pub visible_ordinal: usize,
    /// Runtime formation display name. Identity/kind/reserve state comes from
    /// `UnitLeaderExactRow::assignment`; this field is absent for reserves.
    pub assignment_name: Option<String>,
    pub portrait_gfx: String,
    pub trait_nodes: Vec<TraitNodePresentation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcePresentationInput {
    pub ace_id: u64,
    pub visible_ordinal: usize,
    pub display_name: String,
    pub portrait_gfx: String,
    pub modifier_rows: Vec<String>,
    pub wing_display_name: Option<String>,
    pub wing_equipment_sprite: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CharacterPresentationInput {
    pub leaders: Vec<LeaderPresentationInput>,
    pub aces: Vec<AcePresentationInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedCharacterUi {
    pub leaders: Vec<LeaderPresentationInput>,
    pub aces: Vec<AcePresentationInput>,
}

/// Overlay the authenticated A39 leaves onto the remaining presentation
/// inputs. This function never manufactures nodes or text: a d82 observation
/// must join an already named trait node by `(owner, leader, trait_token)`.
/// D82 progress availability replaces the caller value as well, so a missing
/// producer remains `None` instead of inheriting an unproved zero.
pub fn apply_exact_a39_observations(
    owner_tag_db_id: u32,
    exact: &CharacterPresentationExactView,
    input: &CharacterPresentationInput,
) -> Result<CharacterPresentationInput> {
    if owner_tag_db_id == 0 {
        return Err(invalid("A39 owner_tag_db_id is zero"));
    }
    if exact.status != ViewStatus::Present {
        return Err(invalid("d82 character presentation is not Present"));
    }
    let presentation = exact
        .presentation
        .as_ref()
        .ok_or_else(|| invalid("d82 Present view omitted presentation"))?;
    let mut output = input.clone();
    let leader_index: BTreeMap<u64, usize> = output
        .leaders
        .iter()
        .enumerate()
        .map(|(index, row)| (row.unit_leader_id, index))
        .collect();
    if leader_index.len() != output.leaders.len() {
        return Err(invalid("presentation input contains duplicate leader identity"));
    }
    let mut observed_nodes = BTreeSet::new();
    for tree in presentation
        .trait_trees
        .iter()
        .filter(|tree| tree.owner_tag_db_id == owner_tag_db_id)
    {
        let Some(&leader_index) = leader_index.get(&tree.unit_leader_id) else {
            return Err(invalid(format!(
                "d82 observed unknown unit_leader_id {}",
                tree.unit_leader_id
            )));
        };
        let leader = &mut output.leaders[leader_index];
        for node in &tree.nodes {
            if !observed_nodes.insert((tree.unit_leader_id, node.trait_token.as_str())) {
                return Err(invalid(format!(
                    "d82 repeated trait node {} for leader {} across tree roots",
                    node.trait_token, tree.unit_leader_id
                )));
            }
            let target = leader
                .trait_nodes
                .iter_mut()
                .find(|candidate| candidate.trait_token == node.trait_token)
                .ok_or_else(|| {
                    invalid(format!(
                        "d82 trait node {} has no named presentation input for leader {}",
                        node.trait_token, tree.unit_leader_id
                    ))
                })?;
            target.state = match node.state {
                CharacterTraitNodeState::Blocked => TraitNodeState::Blocked,
                CharacterTraitNodeState::Pickable => TraitNodeState::Pickable,
                CharacterTraitNodeState::Picked => TraitNodeState::Picked,
            };
            target.progress_raw = node.progress_raw;
        }
    }

    let owner_aces = presentation
        .ace_portraits
        .iter()
        .filter(|ace| ace.owner_tag_db_id == owner_tag_db_id)
        .collect::<Vec<_>>();
    if owner_aces.len() != output.aces.len() {
        return Err(invalid("d82 owner ace portrait membership is not bijective"));
    }
    let ace_index: BTreeMap<u64, usize> = output
        .aces
        .iter()
        .enumerate()
        .map(|(index, row)| (row.ace_id, index))
        .collect();
    if ace_index.len() != output.aces.len() {
        return Err(invalid("presentation input contains duplicate ace identity"));
    }
    for ace in owner_aces {
        let Some(&index) = ace_index.get(&ace.ace_id) else {
            return Err(invalid(format!("d82 observed unknown ace_id {}", ace.ace_id)));
        };
        output.aces[index].visible_ordinal = usize::try_from(ace.visible_ordinal)
            .map_err(|_| invalid("d82 ace visible ordinal does not fit this platform"))?;
        output.aces[index].portrait_gfx = ace.portrait_gfx.clone();
    }
    Ok(output)
}

fn invalid(message: impl Into<String>) -> GameUiError {
    GameUiError::parse(format!("XUI17 presentation rejected: {}", message.into()))
}

fn nonempty(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(invalid(format!("{field} is empty")))
    } else {
        Ok(())
    }
}

/// Admit one complete runtime presentation snapshot against exact live rows.
pub fn admit_character_ui(
    roster: &CharacterRosterExactView,
    ace_roster: Option<&AceRosterExactView>,
    input: &CharacterPresentationInput,
) -> Result<AdmittedCharacterUi> {
    if roster.status != ViewStatus::Present {
        return Err(invalid("character roster is not Present"));
    }
    if !roster.command_assignments_complete {
        return Err(invalid("d28 command assignments are not complete"));
    }
    let live_leaders: BTreeMap<u64, _> = roster
        .unit_leaders
        .iter()
        .map(|row| (row.unit_leader_id, row))
        .collect();
    if live_leaders.len() != roster.unit_leaders.len() || input.leaders.len() != live_leaders.len()
    {
        return Err(invalid("leader membership is not bijective"));
    }
    let mut seen = BTreeSet::new();
    for row in &input.leaders {
        if !seen.insert(row.unit_leader_id) {
            return Err(invalid(format!(
                "duplicate unit_leader_id {}",
                row.unit_leader_id
            )));
        }
        let Some(live) = live_leaders.get(&row.unit_leader_id) else {
            return Err(invalid(format!(
                "unknown unit_leader_id {}",
                row.unit_leader_id
            )));
        };
        nonempty(&row.portrait_gfx, "leader portrait_gfx")?;
        match (&live.assignment, row.assignment_name.as_deref()) {
            (None, None) => {}
            (Some(_), Some(name)) if !name.trim().is_empty() => {}
            _ => {
                return Err(invalid(
                    "leader assignment display name contradicts exact d28 join",
                ))
            }
        }
        let mut node_ids = BTreeSet::new();
        for node in &row.trait_nodes {
            nonempty(&node.trait_token, "trait node token")?;
            if !node_ids.insert(node.trait_token.as_str()) {
                return Err(invalid(format!(
                    "duplicate trait node {}",
                    node.trait_token
                )));
            }
        }
        for owned in &live.traits {
            if !row
                .trait_nodes
                .iter()
                .any(|node| node.trait_token == *owned && node.state == TraitNodeState::Picked)
            {
                return Err(invalid(format!(
                    "owned trait {owned} is not represented as Picked for leader {}",
                    row.unit_leader_id
                )));
            }
        }
    }
    let mut leader_ordinals: Vec<_> = input
        .leaders
        .iter()
        .map(|row| row.visible_ordinal)
        .collect();
    leader_ordinals.sort_unstable();
    if leader_ordinals != (0..input.leaders.len()).collect::<Vec<_>>() {
        return Err(invalid(
            "leader visible ordinals are not a contiguous permutation",
        ));
    }

    let live_aces: BTreeMap<u64, _> = ace_roster
        .filter(|view| view.status == ViewStatus::Present)
        .map(|view| view.aces.iter().map(|row| (row.ace_id, row)).collect())
        .unwrap_or_default();
    if input.aces.len() != live_aces.len() {
        return Err(invalid("ace membership is not bijective"));
    }
    let mut seen_aces = BTreeSet::new();
    for row in &input.aces {
        if !seen_aces.insert(row.ace_id) || !live_aces.contains_key(&row.ace_id) {
            return Err(invalid(format!(
                "duplicate or unknown ace_id {}",
                row.ace_id
            )));
        }
        nonempty(&row.display_name, "ace display_name")?;
        nonempty(&row.portrait_gfx, "ace portrait_gfx")?;
        let live = live_aces[&row.ace_id];
        match (
            live.wing_id,
            row.wing_display_name.as_deref(),
            row.wing_equipment_sprite.as_deref(),
        ) {
            (0, None, None) => {}
            (id, Some(name), Some(sprite)) if id != 0 && !name.is_empty() && !sprite.is_empty() => {
            }
            _ => {
                return Err(invalid(format!(
                    "ace {} wing presentation is incomplete",
                    row.ace_id
                )))
            }
        }
    }
    let mut ace_ordinals: Vec<_> = input.aces.iter().map(|row| row.visible_ordinal).collect();
    ace_ordinals.sort_unstable();
    if ace_ordinals != (0..input.aces.len()).collect::<Vec<_>>() {
        return Err(invalid(
            "ace visible ordinals are not a contiguous permutation",
        ));
    }

    let mut leaders = input.leaders.clone();
    leaders.sort_by_key(|row| row.visible_ordinal);
    let mut aces = input.aces.clone();
    aces.sort_by_key(|row| row.visible_ordinal);
    Ok(AdmittedCharacterUi { leaders, aces })
}

/// Exact upstream leaves still required before a raw roster can be displayed.
pub fn required_character_presentation_inputs(
    roster: &CharacterRosterExactView,
    ace_roster: Option<&AceRosterExactView>,
) -> Vec<&'static str> {
    let mut needs = Vec::new();
    if !roster.unit_leaders.is_empty() {
        needs.extend([
            "unit_leader_visible_ordinal",
            "unit_leader_portrait_gfx",
            "unit_leader_trait_tree_node_state",
        ]);
        if roster
            .unit_leaders
            .iter()
            .any(|row| row.assignment.is_some())
        {
            needs.push("unit_leader_assignment_display_name");
        }
    }
    if ace_roster.is_some_and(|view| !view.aces.is_empty()) {
        needs.extend([
            "ace_visible_ordinal",
            "ace_display_name",
            "ace_portrait_gfx",
            "ace_modifier_rows",
            "ace_wing_display_name_and_equipment_sprite",
        ]);
    }
    needs
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{
        AceExactRow, UnitLeaderAssignmentExact, UnitLeaderAssignmentKind, UnitLeaderExactRow,
    };

    fn roster() -> CharacterRosterExactView {
        CharacterRosterExactView {
            status: ViewStatus::Present,
            command_assignments_complete: true,
            characters: Vec::new(),
            country_leader_roles: Vec::new(),
            active_rulers: Vec::new(),
            unit_leaders: vec![UnitLeaderExactRow {
                producer_ordinal: 0,
                unit_leader_id: 7,
                char_id: 2,
                leader_kind: "army".into(),
                leader_kind_raw: 0,
                skill: 4,
                experience_raw: 100,
                attack: 4,
                defense: 3,
                planning: 2,
                logistics: 1,
                maneuvering: 0,
                coordination: 0,
                traits: vec!["panzer_leader".into()],
                assignment: Some(UnitLeaderAssignmentExact {
                    kind: UnitLeaderAssignmentKind::Army,
                    formation_id: 55,
                }),
                token: "GER_test".into(),
                name: "Test".into(),
            }],
        }
    }

    fn aces() -> AceRosterExactView {
        AceRosterExactView {
            status: ViewStatus::Present,
            wing_assignment_complete: true,
            aces: vec![AceExactRow {
                ace_id: 9,
                country_ace_index: 0,
                name: "A".into(),
                surname: "B".into(),
                callsign: "C".into(),
                alive: true,
                handled: Some(false),
                kill_type_raw: Some(0),
                lifecycle: Some(replay_core::AceLifecycleState::Alive),
                wing_id: 4,
                killer_ace_id: 0,
                killer_country_tag_db_id: 0,
            }],
        }
    }

    fn complete_input() -> CharacterPresentationInput {
        CharacterPresentationInput {
            leaders: vec![LeaderPresentationInput {
                unit_leader_id: 7,
                visible_ordinal: 0,
                assignment_name: Some("2. Armee".into()),
                portrait_gfx: "GFX_portrait".into(),
                trait_nodes: vec![TraitNodePresentation {
                    trait_token: "panzer_leader".into(),
                    state: TraitNodeState::Picked,
                    progress_raw: Some(0),
                    reason: String::new(),
                }],
            }],
            aces: vec![AcePresentationInput {
                ace_id: 9,
                visible_ordinal: 0,
                display_name: "A 'C' B".into(),
                portrait_gfx: "GFX_ace".into(),
                modifier_rows: vec!["+10% agility".into()],
                wing_display_name: Some("JG 1".into()),
                wing_equipment_sprite: Some("GFX_fighter".into()),
            }],
        }
    }

    #[test]
    fn complete_snapshot_is_admitted_in_visible_order() {
        let admitted = admit_character_ui(&roster(), Some(&aces()), &complete_input()).unwrap();
        assert_eq!(admitted.leaders[0].unit_leader_id, 7);
        assert_eq!(admitted.aces[0].ace_id, 9);
    }

    #[test]
    fn missing_owned_trait_and_assignment_are_rejected() {
        let mut input = complete_input();
        input.leaders[0].trait_nodes.clear();
        assert!(admit_character_ui(&roster(), Some(&aces()), &input).is_err());
        let mut input = complete_input();
        input.leaders[0].assignment_name = None;
        assert!(admit_character_ui(&roster(), Some(&aces()), &input).is_err());
    }

    #[test]
    fn a39_exact_overlay_replaces_trait_state_and_owner_qualified_ace_portrait() {
        let exact = replay_core::CharacterPresentationExactView {
            status: ViewStatus::Present,
            presentation: Some(replay_core::CharacterPresentationExact {
                schema_version: 1,
                authority: "exact_a729_character_presentation".into(),
                elapsed_wall_ms: 1,
                frame_hour: 2,
                completeness_record_identity: "xui17:a729:refresh:3".into(),
                source_sequence: 3,
                trait_trees: vec![replay_core::CharacterTraitTreeObservation {
                    owner_tag_db_id: 17,
                    unit_leader_id: 7,
                    tree_root_offset: 0x120,
                    nodes: vec![replay_core::CharacterTraitNodeObservation {
                        source_ordinal: 0,
                        trait_token: "panzer_leader".into(),
                        state_raw: 0,
                        state: replay_core::CharacterTraitNodeState::Blocked,
                        progress_raw: None,
                    }],
                }],
                ace_portraits: vec![replay_core::CharacterAcePortraitObservation {
                    owner_tag_db_id: 17,
                    ace_id: 9,
                    visible_ordinal: 0,
                    portrait_gfx: "GFX_GER_ace_m_7".into(),
                }],
            }),
        };
        let overlaid = apply_exact_a39_observations(17, &exact, &complete_input()).unwrap();
        assert_eq!(overlaid.leaders[0].trait_nodes[0].state, TraitNodeState::Blocked);
        assert_eq!(overlaid.leaders[0].trait_nodes[0].progress_raw, None);
        assert_eq!(overlaid.aces[0].portrait_gfx, "GFX_GER_ace_m_7");
    }

    #[test]
    fn leader_experience_is_exact_fixed_point_over_effective_skill_cost() {
        let document = crate::paradox_text::parse_str(
            r#"leader_skills = {
                4 = { cost = 1000 type = corps_commander }
                4 = { cost = 1000 type = field_marshal }
                4 = { cost = 800 type = navy }
            }"#,
        );
        let mut catalog = LeaderSkillCostCatalog::default();
        catalog.add_document(&document).unwrap();
        let exact = catalog.exact_experience(&roster().unit_leaders[0]).unwrap();
        assert_eq!(exact.experience_raw, 100);
        assert_eq!(exact.fixed_scale, 100_000);
        assert_eq!(exact.level_cost, 1000);
        assert_eq!(exact.level_cost_raw, 100_000_000);
    }
}
