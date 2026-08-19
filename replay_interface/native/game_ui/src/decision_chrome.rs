
use crate::assets::AssetResolver;
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, LayoutSet};
use crate::loc::Localisation;
use crate::paradox_text::{parse_file, Value};
use crate::{GameUiError, Result};
use std::collections::{BTreeMap, BTreeSet};


pub const DECISION_VIEW_GUI: &str = "interface/countrydecisionview.gui";
pub const DECISIONS_DIR: &str = "common/decisions";
pub const DECISION_CATEGORIES_DIR: &str = "common/decisions/categories";

pub const ROOT_WINDOW: &str = "countrydecisionview";
pub const DECISION_GRID: &str = "decision_grid";
pub const CATEGORY_HEADER_WINDOW: &str = "category_header";
pub const CATEGORY_DESCRIPTION_WINDOW: &str = "decision_category_desc";
pub const CUSTOM_ICON_ITEM_WINDOW: &str = "custom_icon_item";

pub const CARD_TEMPLATES: [&str; 4] = [
    "decision_item",
    "targeted_decision_item",
    "timed_decision_item",
    "on_map_decision_locator_item",
];

pub const CARD_NAME_TEXT: &str = "name_text";
pub const CARD_COST_TIMER_TEXT: &str = "cost_and_timer_text";
pub const CARD_TARGET_FLAG: &str = "target_flag";

pub const DECISION_UNKNOWN_ICON: &str = "GFX_decision_unknown";

const DECISION_ICON_PREFIX: &str = "GFX_decision_";
const CATEGORY_ICON_PREFIX: &str = "GFX_decision_category_";


#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionCardTemplate {
    pub name: String,
    pub present: bool,
    pub has_icon: bool,
    pub has_name_text: bool,
    pub has_cost_and_timer_text: bool,
    pub has_target_flag: bool,
    pub icon_default_sprite: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionPanelLayout {
    pub source_file: String,
    pub has_root_window: bool,
    pub has_decision_grid: bool,
    pub has_category_header: bool,
    pub cards: Vec<DecisionCardTemplate>,
    pub recoveries: usize,
}

#[derive(Clone, Debug)]
pub struct DecisionAuthoredScene {
    pub source_file: String,
    pub root: GuiElement,
    pub decision_grid: GuiElement,
    pub category_header: GuiElement,
    pub category_description: GuiElement,
    pub custom_icon_item: GuiElement,
    pub card_templates: BTreeMap<String, GuiElement>,
}

impl DecisionPanelLayout {
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


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionCategoryDef {
    pub id: String,
    pub icon_token: Option<String>,
    pub picture: Option<String>,
    pub scripted_gui: Option<String>,
    pub has_custom_icons: bool,
    pub custom_icons: Vec<DecisionCategoryCustomIconDef>,
    pub source_file: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionCategoryCustomIconDef {
    pub source_ordinal: u32,
    pub tag: String,
    pub value_loc_key: String,
    pub desc_loc_key: String,
}

fn unique_named_kind(layouts: &LayoutSet, name: &str, kind: &str) -> Result<GuiElement> {
    let matches = layouts
        .find_by_name(name)
        .into_iter()
        .filter(|element| element.element_type.eq_ignore_ascii_case(kind))
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [element] => Ok(element.clone()),
        [] => Err(GameUiError::missing(format!(
            "{DECISION_VIEW_GUI}: required {kind} {name:?} is absent"
        ))),
        _ => Err(GameUiError::parse(format!(
            "{DECISION_VIEW_GUI}: required {kind} {name:?} is ambiguous in the effective file"
        ))),
    }
}

pub fn load_decision_authored_scene(resolver: &AssetResolver) -> Result<DecisionAuthoredScene> {
    let resolved = resolver.resolve_file(DECISION_VIEW_GUI)?;
    let doc = parse_file(&resolved.path)?;
    if !doc.recoveries.is_empty() {
        return Err(GameUiError::parse(format!(
            "{} required parser recovery",
            resolved.rel_path
        )));
    }
    let mut layouts = LayoutSet::default();
    layouts.add_document(&resolved.rel_path, &doc);
    let root = unique_named_kind(&layouts, ROOT_WINDOW, "containerWindowType")?;
    let decision_grid = unique_named_kind(&layouts, DECISION_GRID, "gridBoxType")?;
    let category_header =
        unique_named_kind(&layouts, CATEGORY_HEADER_WINDOW, "containerWindowType")?;
    let category_description =
        unique_named_kind(&layouts, CATEGORY_DESCRIPTION_WINDOW, "containerWindowType")?;
    let custom_icon_item =
        unique_named_kind(&layouts, CUSTOM_ICON_ITEM_WINDOW, "containerWindowType")?;
    let mut owned_grid = Vec::new();
    root.find_by_name(DECISION_GRID, &mut owned_grid);
    if owned_grid.len() != 1
        || !owned_grid[0]
            .element_type
            .eq_ignore_ascii_case("gridBoxType")
    {
        return Err(GameUiError::parse(
            "XUI19 countrydecisionview does not uniquely own decision_grid",
        ));
    }
    let card_templates = CARD_TEMPLATES
        .iter()
        .map(|name| {
            unique_named_kind(&layouts, name, "containerWindowType")
                .map(|element| ((*name).to_string(), element))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(DecisionAuthoredScene {
        source_file: resolved.rel_path,
        root,
        decision_grid,
        category_header,
        category_description,
        custom_icon_item,
        card_templates,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionDef {
    pub id: String,
    pub category: String,
    pub name_override: Option<String>,
    pub icon_token: Option<String>,
    pub template: String,
    pub source_file: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionSet {
    pub categories: BTreeMap<String, DecisionCategoryDef>,
    pub decisions: Vec<DecisionDef>,
    pub orphan_category_blocks: BTreeSet<String>,
    pub category_file_count: usize,
    pub decision_file_count: usize,
    pub recoveries: usize,
}

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

fn token_to_sprite(token: &str, prefix: &str) -> String {
    let token = token.trim();
    if token.to_ascii_uppercase().starts_with("GFX_") {
        token.to_string()
    } else {
        format!("{prefix}{token}")
    }
}

pub fn category_icon_sprite(category: &DecisionCategoryDef) -> Option<String> {
    if let Some(picture) = &category.picture {
        return Some(token_to_sprite(picture, CATEGORY_ICON_PREFIX));
    }
    category
        .icon_token
        .as_ref()
        .map(|token| token_to_sprite(token, CATEGORY_ICON_PREFIX))
}

pub fn decision_icon_sprite(def: &DecisionDef) -> String {
    match &def.icon_token {
        Some(token) => token_to_sprite(token, DECISION_ICON_PREFIX),
        None => DECISION_UNKNOWN_ICON.to_string(),
    }
}

pub fn decision_name_loc_key(def: &DecisionDef) -> String {
    def.name_override.clone().unwrap_or_else(|| def.id.clone())
}

pub fn decision_desc_loc_key(def: &DecisionDef) -> String {
    format!("{}_desc", def.id)
}

fn is_categories_path(rel: &str) -> bool {
    rel.replace('\\', "/")
        .to_ascii_lowercase()
        .contains("/categories/")
}

fn decision_category_def(
    id: &str,
    value: &Value,
    source_file: &str,
) -> Result<DecisionCategoryDef> {
    let mut custom_icons = Vec::new();
    if let Some(object) = value.as_object() {
        for (ordinal, entry) in object
            .iter()
            .filter(|entry| {
                entry
                    .key
                    .as_deref()
                    .is_some_and(|key| key.eq_ignore_ascii_case("custom_icon"))
            })
            .enumerate()
        {
            let block = entry.value.as_object().ok_or_else(|| {
                GameUiError::parse(format!(
                    "decision category {id:?} custom_icon #{ordinal} in {source_file:?} is not an object"
                ))
            })?;
            let scalar = |field: &str| {
                block
                    .iter()
                    .find(|entry| {
                        entry
                            .key
                            .as_deref()
                            .is_some_and(|key| key.eq_ignore_ascii_case(field))
                    })
                    .and_then(|entry| entry.value.as_scalar())
                    .ok_or_else(|| {
                    GameUiError::parse(format!(
                        "decision category {id:?} custom_icon #{ordinal} in {source_file:?} lacks {field:?}"
                    ))
                })
            };
            custom_icons.push(DecisionCategoryCustomIconDef {
                source_ordinal: u32::try_from(ordinal).map_err(|_| {
                    GameUiError::parse(format!(
                        "decision category {id:?} has too many custom_icon declarations"
                    ))
                })?,
                tag: scalar("tag")?.to_string(),
                value_loc_key: scalar("value")?.to_string(),
                desc_loc_key: scalar("desc")?.to_string(),
            });
        }
    }
    Ok(DecisionCategoryDef {
        id: id.to_string(),
        icon_token: value.first_scalar_ci("icon").map(str::to_string),
        picture: value.first_scalar_ci("picture").map(str::to_string),
        scripted_gui: value.first_scalar_ci("scripted_gui").map(str::to_string),
        has_custom_icons: !custom_icons.is_empty(),
        custom_icons,
        source_file: source_file.to_string(),
    })
}

fn insert_effective_decision_category(
    out: &mut BTreeMap<String, DecisionCategoryDef>,
    definition: DecisionCategoryDef,
) -> Result<()> {
    let id = definition.id.clone();
    match out.entry(id.clone()) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(definition);
            Ok(())
        }
        std::collections::btree_map::Entry::Occupied(previous) => Err(GameUiError::parse(format!(
            "decision category {id:?} is defined more than once in the effective VFS: {:?} and {:?}; no exact cross-file override law is admitted",
            previous.get().source_file,
            definition.source_file
        ))),
    }
}

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
            insert_effective_decision_category(
                &mut out,
                decision_category_def(id, &entry.value, &file.rel_path)?,
            )?;
        }
    }
    Ok(out)
}

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
    let mut decision_sources = BTreeMap::<String, String>::new();
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
                continue;
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
                    continue;
                };
                if id.starts_with('@') {
                    continue;
                }
                if let Some(previous) =
                    decision_sources.insert(id.to_string(), file.rel_path.clone())
                {
                    return Err(GameUiError::parse(format!(
                        "decision {id:?} is defined more than once in the effective VFS: {previous:?} and {:?}; no exact cross-file override law is admitted",
                        file.rel_path
                    )));
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


#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionResolutionReport {
    pub total_decisions: usize,
    pub name_key_resolved: usize,
    pub desc_key_resolved: usize,
    pub icon_declared: usize,
    pub icon_sprite_resolved: usize,
    pub category_known: usize,
    pub template_histogram: BTreeMap<String, usize>,

    pub total_categories: usize,
    pub category_name_resolved: usize,
    pub category_icon_resolved: usize,
    pub category_icon_absent: usize,
}

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


#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecisionAvailability {
    Available,
    Unavailable,
    Active,
    OnCooldown,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionStateEntry {
    pub decision_id: String,
    pub availability: DecisionAvailability,
    pub cost_text: Option<String>,
    pub timer_text: Option<String>,
    pub target_tag: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionCardBinding {
    pub decision_id: String,
    pub category: String,
    pub name_loc_key: String,
    pub icon_sprite: String,
    pub template: String,
    pub availability: Option<DecisionAvailability>,
    pub cost_and_timer_text: Option<String>,
    pub timer_text: Option<String>,
    pub target_tag: Option<String>,
}

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
        assert_eq!(
            decision_icon_sprite(&def("d", Some("GFX_decision_BRA_integralism"), None)),
            "GFX_decision_BRA_integralism"
        );
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
            scripted_gui: None,
            has_custom_icons: false,
            custom_icons: Vec::new(),
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
    fn nested_scripted_gui_never_overrides_top_level_binding() {
        let doc = parse_str(
            r#"
            USA_congress = {
                icon = usa_congress
                scripted_gui = usa_congress_decision_ui
                visible = { scripted_gui = nested_value_must_not_leak }
                custom_icon = {
                    tag = USA
                    value = usa_congress_value
                    desc = usa_congress_desc
                }
            }
            "#,
        );
        let value = doc.root.first_value_ci("USA_congress").unwrap();
        let category = decision_category_def(
            "USA_congress",
            value,
            "common/decisions/categories/USA_decision_categories.txt",
        )
        .unwrap();
        assert_eq!(
            category.scripted_gui.as_deref(),
            Some("usa_congress_decision_ui")
        );
        assert!(category.has_custom_icons);
        assert_eq!(category.custom_icons.len(), 1);
        assert_eq!(category.custom_icons[0].source_ordinal, 0);
        assert_eq!(category.custom_icons[0].tag, "USA");
        assert_eq!(category.custom_icons[0].value_loc_key, "usa_congress_value");
        assert_eq!(category.custom_icons[0].desc_loc_key, "usa_congress_desc");
    }

    #[test]
    fn malformed_custom_icon_is_rejected_instead_of_partially_rendered() {
        for body in [
            "custom_icon = USA",
            "custom_icon = { value = v desc = d }",
            "custom_icon = { tag = USA desc = d }",
            "custom_icon = { tag = USA value = v }",
        ] {
            let doc = parse_str(&format!("category = {{ {body} }}"));
            let error = decision_category_def(
                "category",
                doc.root.first_value_ci("category").unwrap(),
                "common/decisions/categories/test.txt",
            )
            .unwrap_err();
            assert!(error.to_string().contains("custom_icon"));
        }
    }

    #[test]
    fn duplicate_effective_category_id_is_rejected_without_override_guess() {
        let mut categories = BTreeMap::new();
        for source in ["first.txt", "second.txt"] {
            let result = insert_effective_decision_category(
                &mut categories,
                DecisionCategoryDef {
                    id: "duplicate".into(),
                    icon_token: None,
                    picture: None,
                    scripted_gui: None,
                    has_custom_icons: false,
                    custom_icons: Vec::new(),
                    source_file: source.into(),
                },
            );
            if source == "first.txt" {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().to_string().contains("defined more than once"));
            }
        }
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
    fn category_scripted_gui_binding_is_exact_and_non_recursive() {
        let doc = parse_str(
            r#"
            USA_congress = {
                icon = usa_congress
                scripted_gui = usa_congress_decision_ui
                visible = { scripted_gui = nested_value_must_not_leak }
            }
            "#,
        );
        let value = doc.root.first_value_ci("USA_congress").unwrap();
        let category = decision_category_def(
            "USA_congress",
            value,
            "common/decisions/categories/USA_decision_categories.txt",
        )
        .unwrap();
        assert_eq!(
            category.scripted_gui.as_deref(),
            Some("usa_congress_decision_ui")
        );
    }

    #[test]
    fn duplicate_effective_category_id_is_rejected_without_an_override_guess() {
        let mut categories = BTreeMap::new();
        insert_effective_decision_category(
            &mut categories,
            DecisionCategoryDef {
                id: "duplicate".into(),
                icon_token: None,
                picture: None,
                scripted_gui: Some("first".into()),
                has_custom_icons: false,
                custom_icons: Vec::new(),
                source_file: "common/decisions/categories/first.txt".into(),
            },
        )
        .unwrap();
        let error = insert_effective_decision_category(
            &mut categories,
            DecisionCategoryDef {
                id: "duplicate".into(),
                icon_token: None,
                picture: None,
                scripted_gui: Some("second".into()),
                has_custom_icons: false,
                custom_icons: Vec::new(),
                source_file: "common/decisions/categories/second.txt".into(),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("defined more than once"));
    }

    #[test]
    fn binding_is_fail_closed_without_state() {
        let card = decision_card_binding(&def("GER_mefo_bills", Some("steel"), None), None);
        assert_eq!(card.decision_id, "GER_mefo_bills");
        assert_eq!(card.name_loc_key, "GER_mefo_bills");
        assert_eq!(card.icon_sprite, "GFX_decision_steel");
        assert_eq!(card.template, "decision_item");
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
                scripted_gui: None,
                has_custom_icons: false,
                custom_icons: Vec::new(),
                source_file: "c.txt".into(),
            },
        )]);
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
