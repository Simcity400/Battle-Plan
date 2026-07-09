//! Decisions-panel chrome (U2 wave — UI_COVERAGE_MATRIX §1 "Decisions" row,
//! REPLAY_PRODUCTION_PLAN §6a).
//!
//! the game renders **all** decisions/missions through a single small chrome panel
//! (`interface/countrydecisionview.gui`): a scrolling grid of *category tabs*
//! (`category_header`) each holding *decision cards* (`decision_item` /
//! `targeted_decision_item` / `timed_decision_item` /
//! `on_map_decision_locator_item`). The card *content* is data-driven from the
//! script (`common/decisions/*.txt` + `common/decisions/categories/*.txt`) and
//! its localisation/GFX. This module does two file-derived jobs and one binding
//! seam:
//!
//!   1. **Layout** ([`parse_decision_layout`]) — parse the shipped `.gui` via the
//!      shared [`crate::gui`] parser and surface the chrome windows + card
//!      templates and their sub-elements (icon / name / cost-timer / flag).
//!   2. **Decision-def → card mapping** ([`parse_decisions`] +
//!      [`resolve_decisions`]) — parse every decision definition to
//!      `id → {category, name-loc-key, icon-sprite}` and resolve those against
//!      the recording's localisation + sprite registry (the `N resolve / total`
//!      gate).
//!   3. **Binding** ([`decision_card_binding`]) — fold the per-decision dynamic
//!      STATE (available / active / on-cooldown / cost / timer) onto a card.
//!
//! ## Data label discipline (Constraint 3)
//!
//!   * **FILE** — the layout, the `id → name-key` law (loc key == decision id,
//!     or the decision's own `name =` override key), the icon-sprite naming law
//!     (`GFX_decision_<icon>` / category `GFX_decision_category_<icon>`), and the
//!     card-icon fail-closed default (`GFX_decision_unknown`, the `decision_item`
//!     `icon` element's shipped `spriteType`). All cited from shipped files.
//!   * **CAPTURE / NEW** — the per-decision dynamic STATE (which decisions are
//!     available/active/on-cooldown, their remaining timers and PP/other costs)
//!     is *simulation* state, one of the inventory's 14 NEW streams
//!     (REPLAY_DATA_REQUIREMENTS §9c / EXTRACTION_NEEDS A18). It is not yet a
//!     `replay_core` view, so [`decision_card_binding`] takes it as an explicit
//!     input and renders the **fail-closed absent form** (dynamic fields `None`)
//!     when no state is supplied.
//!   * **UNKNOWN** — cost/timer NUMBER formatting (PP glyph, +/- colouring,
//!     day/week rendering) is engine-computed; any supplied text is carried raw
//!     (never fabricated).

use crate::assets::AssetResolver;
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, LayoutSet};
use crate::loc::Localisation;
use crate::paradox_text::{parse_file, Value};
use crate::Result;
use std::collections::{BTreeMap, BTreeSet};

// ===========================================================================
// FILE constants — shipped paths + element names + naming laws
// ===========================================================================

/// The decisions-panel chrome layout (FILE).
pub const DECISION_VIEW_GUI: &str = "interface/countrydecisionview.gui";
/// Directory of per-country decision-definition scripts (FILE).
pub const DECISIONS_DIR: &str = "common/decisions";
/// Sub-directory of decision *category* definitions (FILE).
pub const DECISION_CATEGORIES_DIR: &str = "common/decisions/categories";

/// Root panel window (FILE: `countrydecisionview.gui`).
pub const ROOT_WINDOW: &str = "countrydecisionview";
/// The scrolling gridbox that holds the category/decision rows.
pub const DECISION_GRID: &str = "decision_grid";
/// The category-tab header template.
pub const CATEGORY_HEADER_WINDOW: &str = "category_header";

/// The four decision-card templates, in shipped declaration order. Each renders
/// one decision instance; which template a decision uses is classified from its
/// script fields ([`classify_template`]).
pub const CARD_TEMPLATES: [&str; 4] = [
    "decision_item",
    "targeted_decision_item",
    "timed_decision_item",
    "on_map_decision_locator_item",
];

/// Card sub-element names the binding fills (FILE). The `on_map` locator card
/// names its text element `name_text` and its icon `decision_icon`, so the icon
/// probe accepts either spelling.
pub const CARD_NAME_TEXT: &str = "name_text";
pub const CARD_COST_TIMER_TEXT: &str = "cost_and_timer_text";
pub const CARD_TARGET_FLAG: &str = "target_flag";

/// Fail-closed card icon when a decision declares no `icon` (FILE: the
/// `decision_item`/`icon` element ships `spriteType = "GFX_decision_unknown"`).
pub const DECISION_UNKNOWN_ICON: &str = "GFX_decision_unknown";

/// GFX name prefix for a decision `icon = <token>` (FILE naming law, verified
/// against `interface/decisions.gfx` + `countrydecisionview.gfx`: 601 shipped
/// `GFX_decision_*` sprites).
const DECISION_ICON_PREFIX: &str = "GFX_decision_";
/// GFX name prefix for a category `icon = <token>` (FILE: 112 shipped
/// `GFX_decision_category_*` sprites).
const CATEGORY_ICON_PREFIX: &str = "GFX_decision_category_";

// ===========================================================================
// Layout
// ===========================================================================

/// One decision-card template's sub-element presence (FILE, from the `.gui`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionCardTemplate {
    pub name: String,
    pub present: bool,
    pub has_icon: bool,
    pub has_name_text: bool,
    pub has_cost_and_timer_text: bool,
    pub has_target_flag: bool,
    /// The card `icon`/`decision_icon` element's declared default sprite.
    pub icon_default_sprite: Option<String>,
}

/// The parsed decisions-panel chrome (layout only; no state).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionPanelLayout {
    pub source_file: String,
    pub has_root_window: bool,
    pub has_decision_grid: bool,
    pub has_category_header: bool,
    /// The four card templates, in [`CARD_TEMPLATES`] order.
    pub cards: Vec<DecisionCardTemplate>,
    pub recoveries: usize,
}

impl DecisionPanelLayout {
    /// True when every gated chrome window + all four card templates parsed, and
    /// each card carries at least an icon + a name text element. This is the
    /// layout gate the binding relies on.
    pub fn is_complete(&self) -> bool {
        self.has_root_window
            && self.has_decision_grid
            && self.has_category_header
            && self.cards.len() == CARD_TEMPLATES.len()
            && self
                .cards
                .iter()
                .all(|card| card.present && card.has_icon && card.has_name_text)
    }
}

/// Find the first icon-bearing sprite on a card sub-element named `icon` or
/// `decision_icon`; returns `(present, default_sprite)`.
fn card_icon(card: &GuiElement) -> (bool, Option<String>) {
    for icon_name in ["icon", "decision_icon"] {
        let mut hits = Vec::new();
        card.find_by_name(icon_name, &mut hits);
        if let Some(el) = hits.into_iter().find(|e| {
            e.element_type.eq_ignore_ascii_case("iconType")
                || e.element_type.eq_ignore_ascii_case("buttonType")
        }) {
            return (true, el.sprite_refs.first().cloned());
        }
    }
    (false, None)
}

/// True when the card has a direct-or-nested child element named `name`.
fn has_named_child(card: &GuiElement, name: &str) -> bool {
    let mut hits = Vec::new();
    card.find_by_name(name, &mut hits);
    hits.iter().any(|el| !std::ptr::eq(*el, card))
}

fn card_template(layouts: &LayoutSet, name: &str) -> DecisionCardTemplate {
    let Some(card) = layouts
        .find_by_name(name)
        .into_iter()
        .find(|el| el.element_type.eq_ignore_ascii_case("containerWindowType"))
    else {
        return DecisionCardTemplate {
            name: name.to_string(),
            ..DecisionCardTemplate::default()
        };
    };
    let (has_icon, icon_default_sprite) = card_icon(card);
    DecisionCardTemplate {
        name: name.to_string(),
        present: true,
        has_icon,
        has_name_text: has_named_child(card, CARD_NAME_TEXT),
        has_cost_and_timer_text: has_named_child(card, CARD_COST_TIMER_TEXT),
        has_target_flag: has_named_child(card, CARD_TARGET_FLAG),
        icon_default_sprite,
    }
}

/// Parse the decisions-panel chrome layout from the shipped `.gui` via the
/// shared parser (reuse; no bare joins — the resolver owns the mod/vanilla root
/// order).
pub fn parse_decision_layout(resolver: &AssetResolver) -> Result<DecisionPanelLayout> {
    let resolved = resolver.resolve_file(DECISION_VIEW_GUI)?;
    let doc = parse_file(&resolved.path)?;
    let mut layouts = LayoutSet::default();
    layouts.add_document(&resolved.rel_path, &doc);

    let has_window = |name: &str, kind: &str| {
        layouts
            .find_by_name(name)
            .into_iter()
            .any(|el| el.element_type.eq_ignore_ascii_case(kind))
    };

    let cards = CARD_TEMPLATES
        .iter()
        .map(|name| card_template(&layouts, name))
        .collect();

    Ok(DecisionPanelLayout {
        source_file: resolved.rel_path,
        has_root_window: has_window(ROOT_WINDOW, "containerWindowType"),
        has_decision_grid: has_window(DECISION_GRID, "gridBoxType"),
        has_category_header: has_window(CATEGORY_HEADER_WINDOW, "containerWindowType"),
        cards,
        recoveries: doc.recoveries.len(),
    })
}

// ===========================================================================
// Decision + category definitions (script)
// ===========================================================================

/// A decision *category* (the tab chrome keys off this).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionCategoryDef {
    pub id: String,
    /// The bare `icon = <token>` (resolved to a sprite by [`category_icon_sprite`]).
    pub icon_token: Option<String>,
    /// A full `picture = GFX_...` sprite (takes precedence over `icon`).
    pub picture: Option<String>,
    pub source_file: String,
}

/// One decision definition (the card content source).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionDef {
    pub id: String,
    /// The owning category (the top-level block key in the decision file).
    pub category: String,
    /// A `name = <key>` loc-key override, when the decision declares one.
    pub name_override: Option<String>,
    /// The decision's own `icon = <token>`, when declared.
    pub icon_token: Option<String>,
    /// Which card template renders this decision ([`classify_template`]).
    pub template: String,
    pub source_file: String,
}

/// The full parsed decision inventory + parse diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionSet {
    pub categories: BTreeMap<String, DecisionCategoryDef>,
    pub decisions: Vec<DecisionDef>,
    /// Top-level blocks in decision files whose key is NOT a known category
    /// (the game requires decision-file top-level == category; expected empty).
    pub orphan_category_blocks: BTreeSet<String>,
    pub category_file_count: usize,
    pub decision_file_count: usize,
    pub recoveries: usize,
}

/// Card-template classifier (FILE heuristic from the decision's declared fields):
///   * a `targets` / `target_root_trigger` / `target_trigger` field → targeted;
///   * else a `days_remove` / `days_mission_timeout` timer field → timed;
///   * else the plain `decision_item`.
///
/// `on_map_decision_locator_item` is an on-map locator surface, not selected by
/// a script field, so it is never auto-assigned here.
pub fn classify_template(decision: &Value) -> &'static str {
    let has = |key: &str| decision.first_value_ci(key).is_some();
    if has("targets") || has("target_root_trigger") || has("target_trigger") {
        "targeted_decision_item"
    } else if has("days_remove") || has("days_mission_timeout") {
        "timed_decision_item"
    } else {
        "decision_item"
    }
}

/// Resolve an `icon = <token>` to its GFX sprite name (FILE law): a token that
/// already starts with `GFX_` is used verbatim, else it is prefixed.
fn token_to_sprite(token: &str, prefix: &str) -> String {
    let token = token.trim();
    if token.to_ascii_uppercase().starts_with("GFX_") {
        token.to_string()
    } else {
        format!("{prefix}{token}")
    }
}

/// The category-tab icon sprite (FILE): `picture` if present, else the prefixed
/// `icon` token, else `None` (the tab falls back to `GFX_decision_category_generic`
/// in the shipped layout, but that default is a *layout* value, not a def value).
pub fn category_icon_sprite(category: &DecisionCategoryDef) -> Option<String> {
    if let Some(picture) = &category.picture {
        return Some(token_to_sprite(picture, CATEGORY_ICON_PREFIX));
    }
    category
        .icon_token
        .as_ref()
        .map(|token| token_to_sprite(token, CATEGORY_ICON_PREFIX))
}

/// The decision-card icon sprite (FILE): the decision's own `icon` token
/// (prefixed), else the fail-closed layout default [`DECISION_UNKNOWN_ICON`].
pub fn decision_icon_sprite(def: &DecisionDef) -> String {
    match &def.icon_token {
        Some(token) => token_to_sprite(token, DECISION_ICON_PREFIX),
        None => DECISION_UNKNOWN_ICON.to_string(),
    }
}

/// The decision name loc key (FILE): the `name =` override key when declared,
/// else the decision id (the game's default: the id IS the loc key).
pub fn decision_name_loc_key(def: &DecisionDef) -> String {
    def.name_override.clone().unwrap_or_else(|| def.id.clone())
}

/// The decision description/tooltip loc key (FILE convention `<id>_desc`).
pub fn decision_desc_loc_key(def: &DecisionDef) -> String {
    format!("{}_desc", def.id)
}

fn is_categories_path(rel: &str) -> bool {
    rel.replace('\\', "/")
        .to_ascii_lowercase()
        .contains("/categories/")
}

/// Parse every decision *category* definition under
/// `common/decisions/categories/`.
pub fn parse_decision_categories(
    resolver: &AssetResolver,
) -> Result<BTreeMap<String, DecisionCategoryDef>> {
    let mut out = BTreeMap::new();
    let files = resolver.list_files_recursive(DECISION_CATEGORIES_DIR, Some("txt"))?;
    for file in files {
        let doc = parse_file(&file.path)?;
        for entry in doc.root.fields() {
            let Some(id) = entry.key.as_deref() else {
                continue;
            };
            if entry.value.as_object().is_none() || id.starts_with('@') {
                continue;
            }
            out.entry(id.to_string())
                .or_insert_with(|| DecisionCategoryDef {
                    id: id.to_string(),
                    icon_token: entry.value.first_scalar_ci("icon").map(str::to_string),
                    picture: entry.value.first_scalar_ci("picture").map(str::to_string),
                    source_file: file.rel_path.clone(),
                });
        }
    }
    Ok(out)
}

/// Parse the full decision inventory: every `category = { decision = { .. } }`
/// block in `common/decisions/*.txt` (excluding the `categories/` sub-dir),
/// cross-referenced against the parsed category set.
pub fn parse_decisions(resolver: &AssetResolver) -> Result<DecisionSet> {
    let categories = parse_decision_categories(resolver)?;
    let mut set = DecisionSet {
        category_file_count: resolver
            .list_files_recursive(DECISION_CATEGORIES_DIR, Some("txt"))?
            .len(),
        categories: categories.clone(),
        ..DecisionSet::default()
    };

    let files = resolver.list_files_recursive(DECISIONS_DIR, Some("txt"))?;
    for file in files {
        if is_categories_path(&file.rel_path) {
            continue;
        }
        set.decision_file_count += 1;
        let doc = parse_file(&file.path)?;
        set.recoveries += doc.recoveries.len();
        for cat_entry in doc.root.fields() {
            let Some(category) = cat_entry.key.as_deref() else {
                continue;
            };
            let Some(_) = cat_entry.value.as_object() else {
                continue; // scalars (e.g. @vars) are not category blocks
            };
            if category.starts_with('@') {
                continue;
            }
            if !categories.contains_key(category) {
                set.orphan_category_blocks.insert(category.to_string());
            }
            for dec_entry in cat_entry.value.fields() {
                let Some(id) = dec_entry.key.as_deref() else {
                    continue;
                };
                let Some(_) = dec_entry.value.as_object() else {
                    continue; // scalars inside a category (e.g. @vars) aren't decisions
                };
                if id.starts_with('@') {
                    continue;
                }
                set.decisions.push(DecisionDef {
                    id: id.to_string(),
                    category: category.to_string(),
                    name_override: dec_entry.value.first_scalar_ci("name").map(str::to_string),
                    icon_token: dec_entry.value.first_scalar_ci("icon").map(str::to_string),
                    template: classify_template(&dec_entry.value).to_string(),
                    source_file: file.rel_path.clone(),
                });
            }
        }
    }
    Ok(set)
}

// ===========================================================================
// Resolution (the N-resolve/total gate)
// ===========================================================================

/// Counts of how much of the decision inventory resolves against the recording's
/// localisation + sprite registry. Every field is an exact count (the gate).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionResolutionReport {
    pub total_decisions: usize,
    /// Decisions whose name loc key exists in the localisation.
    pub name_key_resolved: usize,
    /// Decisions whose `<id>_desc` tooltip key exists.
    pub desc_key_resolved: usize,
    /// Decisions that declare an `icon` field.
    pub icon_declared: usize,
    /// Of the declared icons, how many resolve to a real sprite.
    pub icon_sprite_resolved: usize,
    /// Decisions whose category is a known category definition.
    pub category_known: usize,
    /// Per-card-template decision counts.
    pub template_histogram: BTreeMap<String, usize>,

    pub total_categories: usize,
    /// Categories whose id resolves as a loc key (the tab name).
    pub category_name_resolved: usize,
    /// Categories whose tab icon/picture resolves to a real sprite.
    pub category_icon_resolved: usize,
    /// Categories declaring neither icon nor picture (tab falls back to the
    /// layout default `GFX_decision_category_generic`).
    pub category_icon_absent: usize,
}

/// Resolve the whole [`DecisionSet`] against the recording's localisation +
/// sprite registry.
pub fn resolve_decisions(
    set: &DecisionSet,
    loc: &Localisation,
    registry: &SpriteRegistry,
) -> DecisionResolutionReport {
    let mut report = DecisionResolutionReport {
        total_decisions: set.decisions.len(),
        total_categories: set.categories.len(),
        ..DecisionResolutionReport::default()
    };

    for def in &set.decisions {
        if loc.get(&decision_name_loc_key(def)).is_some() {
            report.name_key_resolved += 1;
        }
        if loc.get(&decision_desc_loc_key(def)).is_some() {
            report.desc_key_resolved += 1;
        }
        if def.icon_token.is_some() {
            report.icon_declared += 1;
            if registry.get(&decision_icon_sprite(def)).is_some() {
                report.icon_sprite_resolved += 1;
            }
        }
        if set.categories.contains_key(&def.category) {
            report.category_known += 1;
        }
        *report
            .template_histogram
            .entry(def.template.clone())
            .or_insert(0) += 1;
    }

    for category in set.categories.values() {
        if loc.get(&category.id).is_some() {
            report.category_name_resolved += 1;
        }
        match category_icon_sprite(category) {
            Some(sprite) => {
                if registry.get(&sprite).is_some() {
                    report.category_icon_resolved += 1;
                }
            }
            None => report.category_icon_absent += 1,
        }
    }

    report
}

// ===========================================================================
// Binding (decision-state stream → card) — fail-closed
// ===========================================================================

/// A decision's dynamic availability in the recording (the NEW "decision state"
/// stream). `Unknown` is the fail-closed value when the stream carries a
/// decision but not a decodable status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecisionAvailability {
    /// Available to take (passes `available`).
    Available,
    /// Visible but not currently available.
    Unavailable,
    /// Currently active / running (timed or mission decisions).
    Active,
    /// Taken and on cooldown (`days_re_enable`).
    OnCooldown,
    #[default]
    Unknown,
}

/// One decision's dynamic STATE as the NEW recording stream must carry it
/// (REPLAY_DATA_REQUIREMENTS §9c / EXTRACTION_NEEDS A18). This is the shape
/// the binding consumes; it is NOT yet a `replay_core` view. `cost_text` /
/// `timer_text` are raw engine strings (UNKNOWN formatting → carried verbatim).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionStateEntry {
    pub decision_id: String,
    pub availability: DecisionAvailability,
    /// The card `cost_and_timer_text` value (PP/other cost), raw.
    pub cost_text: Option<String>,
    /// The card `timer_text` value (remaining days), raw.
    pub timer_text: Option<String>,
    /// The `target_flag` tag for a targeted decision.
    pub target_tag: Option<String>,
}

/// One populated decision card: FILE-derived static chrome (id/category/name-
/// key/icon/template) plus the fail-closed dynamic state from the stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionCardBinding {
    // --- FILE-derived static chrome (always present) ---
    pub decision_id: String,
    pub category: String,
    pub name_loc_key: String,
    pub icon_sprite: String,
    pub template: String,
    // --- dynamic state (None == fail-closed absent) ---
    pub availability: Option<DecisionAvailability>,
    pub cost_and_timer_text: Option<String>,
    pub timer_text: Option<String>,
    pub target_tag: Option<String>,
}

/// Build a decision card for `def`, folding in an optional [`DecisionStateEntry`].
/// The static chrome is always filled from files; every dynamic field is `Some`
/// only when the state stream supplies it, else `None` (fail-closed — never a
/// fabricated "Available"/cost/timer).
pub fn decision_card_binding(
    def: &DecisionDef,
    state: Option<&DecisionStateEntry>,
) -> DecisionCardBinding {
    DecisionCardBinding {
        decision_id: def.id.clone(),
        category: def.category.clone(),
        name_loc_key: decision_name_loc_key(def),
        icon_sprite: decision_icon_sprite(def),
        template: def.template.clone(),
        availability: state.map(|s| s.availability),
        cost_and_timer_text: state.and_then(|s| s.cost_text.clone()),
        timer_text: state.and_then(|s| s.timer_text.clone()),
        target_tag: state.and_then(|s| s.target_tag.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paradox_text::parse_str;

    fn def(id: &str, icon: Option<&str>, name: Option<&str>) -> DecisionDef {
        DecisionDef {
            id: id.to_string(),
            category: "cat".to_string(),
            name_override: name.map(str::to_string),
            icon_token: icon.map(str::to_string),
            template: "decision_item".to_string(),
            source_file: "test.txt".to_string(),
        }
    }

    #[test]
    fn icon_token_prefixing_and_passthrough() {
        assert_eq!(
            decision_icon_sprite(&def("d", Some("generic_civil_support"), None)),
            "GFX_decision_generic_civil_support"
        );
        // Already a GFX_ sprite → verbatim.
        assert_eq!(
            decision_icon_sprite(&def("d", Some("GFX_decision_BRA_integralism"), None)),
            "GFX_decision_BRA_integralism"
        );
        // No icon → fail-closed layout default.
        assert_eq!(
            decision_icon_sprite(&def("d", None, None)),
            DECISION_UNKNOWN_ICON
        );
    }

    #[test]
    fn name_key_uses_override_then_id() {
        assert_eq!(
            decision_name_loc_key(&def("GER_mefo_bills", None, None)),
            "GER_mefo_bills"
        );
        assert_eq!(
            decision_name_loc_key(&def("d", None, Some("CUSTOM_KEY"))),
            "CUSTOM_KEY"
        );
        assert_eq!(
            decision_desc_loc_key(&def("GER_mefo_bills", None, None)),
            "GER_mefo_bills_desc"
        );
    }

    #[test]
    fn category_icon_prefers_picture_then_icon() {
        let with_picture = DecisionCategoryDef {
            id: "c".into(),
            icon_token: Some("generic_research".into()),
            picture: Some("GFX_decision_cat_picture_naval_treaties".into()),
            source_file: "c.txt".into(),
        };
        assert_eq!(
            category_icon_sprite(&with_picture).as_deref(),
            Some("GFX_decision_cat_picture_naval_treaties")
        );
        let icon_only = DecisionCategoryDef {
            picture: None,
            ..with_picture.clone()
        };
        assert_eq!(
            category_icon_sprite(&icon_only).as_deref(),
            Some("GFX_decision_category_generic_research")
        );
        let bare = DecisionCategoryDef {
            icon_token: None,
            ..icon_only
        };
        assert_eq!(category_icon_sprite(&bare), None);
    }

    #[test]
    fn template_classifier_reads_declared_fields() {
        let targeted = parse_str("d = { targets = { GER } }")
            .root
            .first_value_ci("d")
            .unwrap()
            .clone();
        assert_eq!(classify_template(&targeted), "targeted_decision_item");
        let timed = parse_str("d = { days_remove = 30 }")
            .root
            .first_value_ci("d")
            .unwrap()
            .clone();
        assert_eq!(classify_template(&timed), "timed_decision_item");
        let plain = parse_str("d = { cost = 50 }")
            .root
            .first_value_ci("d")
            .unwrap()
            .clone();
        assert_eq!(classify_template(&plain), "decision_item");
    }

    #[test]
    fn binding_is_fail_closed_without_state() {
        let card = decision_card_binding(&def("GER_mefo_bills", Some("steel"), None), None);
        // Static chrome present…
        assert_eq!(card.decision_id, "GER_mefo_bills");
        assert_eq!(card.name_loc_key, "GER_mefo_bills");
        assert_eq!(card.icon_sprite, "GFX_decision_steel");
        assert_eq!(card.template, "decision_item");
        // …dynamic state fail-closed absent (never fabricated).
        assert_eq!(card.availability, None);
        assert_eq!(card.cost_and_timer_text, None);
        assert_eq!(card.timer_text, None);
        assert_eq!(card.target_tag, None);
    }

    #[test]
    fn binding_folds_supplied_state() {
        let state = DecisionStateEntry {
            decision_id: "GER_mefo_bills".into(),
            availability: DecisionAvailability::Available,
            cost_text: Some("150".into()),
            timer_text: None,
            target_tag: None,
        };
        let card = decision_card_binding(&def("GER_mefo_bills", None, None), Some(&state));
        assert_eq!(card.availability, Some(DecisionAvailability::Available));
        assert_eq!(card.cost_and_timer_text.as_deref(), Some("150"));
        assert_eq!(card.icon_sprite, DECISION_UNKNOWN_ICON);
    }

    #[test]
    fn parse_decisions_two_level_structure() {
        // A category block wrapping two decisions; effect-block `name=`s inside
        // must NOT leak as a decision name override.
        let doc = parse_str(
            r#"
            my_category = {
                first_decision = {
                    icon = generic_civil_support
                    available = { always = yes }
                    complete_effect = { set_province_name = { name = "Leaks?" } }
                }
                second_decision = {
                    name = OVERRIDE_KEY
                    targets = { GER }
                }
            }
            "#,
        );
        let mut set = DecisionSet::default();
        let cats = BTreeMap::from([(
            "my_category".to_string(),
            DecisionCategoryDef {
                id: "my_category".into(),
                icon_token: None,
                picture: None,
                source_file: "c.txt".into(),
            },
        )]);
        // Simulate the inner loop of parse_decisions against the in-memory doc.
        for cat_entry in doc.root.fields() {
            let category = cat_entry.key.as_deref().unwrap();
            for dec_entry in cat_entry.value.fields() {
                let id = dec_entry.key.as_deref().unwrap();
                set.decisions.push(DecisionDef {
                    id: id.to_string(),
                    category: category.to_string(),
                    name_override: dec_entry.value.first_scalar_ci("name").map(str::to_string),
                    icon_token: dec_entry.value.first_scalar_ci("icon").map(str::to_string),
                    template: classify_template(&dec_entry.value).to_string(),
                    source_file: "d.txt".into(),
                });
            }
        }
        set.categories = cats;
        assert_eq!(set.decisions.len(), 2);
        let first = &set.decisions[0];
        assert_eq!(first.id, "first_decision");
        assert_eq!(first.name_override, None, "effect-block name must not leak");
        assert_eq!(first.icon_token.as_deref(), Some("generic_civil_support"));
        assert_eq!(first.template, "decision_item");
        let second = &set.decisions[1];
        assert_eq!(second.name_override.as_deref(), Some("OVERRIDE_KEY"));
        assert_eq!(second.template, "targeted_decision_item");
    }
}
