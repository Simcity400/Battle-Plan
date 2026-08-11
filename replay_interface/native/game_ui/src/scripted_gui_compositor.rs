//! Fail-closed XUI22 authored-root composition.
//!
//! Scripted-GUI definitions are attached to contextual parents by authored
//! content. The compositor therefore requires the owning parent extent and
//! never substitutes the physical viewport.

use crate::compose::{
    resolve_element_rect, ComposeOptions, ComposeReport, Compositor, Rect, UiAssetCache,
};
use crate::dds::RgbaImage;
use crate::decision_chrome::DecisionSet;
use crate::definition_admission::GuiDefinitionCatalog;
use crate::gfx::SpriteRegistry;
use crate::gui::GuiElement;
use crate::loc::Localisation;
use crate::observer_panels::{
    ExactPresentationState, ScriptedGuiObserverPanel, ScriptedGuiObserverRow,
    ScriptedGuiRuntimeAuthorityState,
};
use crate::scripted_gui_runtime::{EvaluatedProperty, Truth};
use crate::{GameUiError, Result};
use replay_core::ValidatedContentAdmission;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptedGuiInteractionState {
    pub root_enabled: bool,
    pub visual_enabled: BTreeMap<String, bool>,
    pub click_enabled: BTreeMap<String, bool>,
    pub right_click_enabled: BTreeMap<String, bool>,
    /// Effect names are semantic callback names, not inferred widget names.
    pub effect_enabled: BTreeMap<String, bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComposedScriptedGuiRoot {
    pub manager_order_index: u32,
    pub gui_id: String,
    pub window_name: String,
    pub source_file: String,
    pub image: RgbaImage,
    pub report: ComposeReport,
    pub interaction: ScriptedGuiInteractionState,
}

/// A root ready for insertion into XUI19's exact decision-category child
/// stream. XUI19 owns viewport placement, clipping, scrolling and the final
/// child cursor; this value retains only parent-relative authored geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedDecisionCategoryScriptedGuiRoot {
    pub category_id: String,
    pub category_source_file: String,
    pub manager_order_index: u32,
    pub gui_id: String,
    pub window_name: String,
    pub definition_source_file: String,
    pub root_source_file: String,
    pub root: GuiElement,
    pub options: ComposeOptions,
    pub local_rect: [i32; 4],
    pub interaction: ScriptedGuiInteractionState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedDecisionCategoryParent {
    pub category_id: String,
    pub expanded: bool,
}

fn namespace_element(
    element: &mut GuiElement,
    namespace: &str,
    names: &mut BTreeMap<String, String>,
) -> Result<()> {
    let original = element.name.clone();
    if names.contains_key(&original) {
        return Err(GameUiError::parse(format!(
            "XUI22 authored element name {original:?} is not unique"
        )));
    }
    let renamed = format!("{original}__{namespace}");
    names.insert(original, renamed.clone());
    element.name = renamed;
    for child in &mut element.children {
        namespace_element(child, namespace, names)?;
    }
    Ok(())
}

fn namespace_map<V>(
    values: &mut BTreeMap<String, V>,
    names: &BTreeMap<String, String>,
    label: &str,
) -> Result<()> {
    let mut renamed = BTreeMap::new();
    for (name, value) in std::mem::take(values) {
        let namespaced = names.get(&name).ok_or_else(|| {
            GameUiError::missing(format!(
                "XUI22 {label} override names no authored element {name:?}"
            ))
        })?;
        if renamed.insert(namespaced.clone(), value).is_some() {
            return Err(GameUiError::parse(format!(
                "XUI22 {label} overrides collide after namespacing"
            )));
        }
    }
    *values = renamed;
    Ok(())
}

fn namespace_set(
    values: &mut BTreeSet<String>,
    names: &BTreeMap<String, String>,
    label: &str,
) -> Result<()> {
    let mut renamed = BTreeSet::new();
    for name in std::mem::take(values) {
        let namespaced = names.get(&name).ok_or_else(|| {
            GameUiError::missing(format!(
                "XUI22 {label} override names no authored element {name:?}"
            ))
        })?;
        if !renamed.insert(namespaced.clone()) {
            return Err(GameUiError::parse(format!(
                "XUI22 {label} overrides collide after namespacing"
            )));
        }
    }
    *values = renamed;
    Ok(())
}

fn merge_map<K: Ord, V>(
    target: &mut BTreeMap<K, V>,
    source: BTreeMap<K, V>,
    label: &str,
) -> Result<()> {
    for (key, value) in source {
        if target.insert(key, value).is_some() {
            return Err(GameUiError::parse(format!(
                "XUI22 {label} override collides with the owning decision compositor"
            )));
        }
    }
    Ok(())
}

/// Namespace and transactionally merge one prepared root into XUI19's shared
/// composition options. `list_y` is supplied by XUI19 after the exact
/// preceding category children; the returned height is the authored resolved
/// amount by which XUI19 must advance following children.
pub fn merge_prepared_decision_category_root(
    mut prepared: PreparedDecisionCategoryScriptedGuiRoot,
    namespace: &str,
    list_y: i32,
    owner_options: &mut ComposeOptions,
) -> Result<(GuiElement, i32, ScriptedGuiInteractionState)> {
    if namespace.is_empty()
        || !namespace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(GameUiError::parse(
            "XUI22 decision insertion namespace is invalid",
        ));
    }
    let mut names = BTreeMap::new();
    namespace_element(&mut prepared.root, namespace, &mut names)?;
    namespace_map(&mut prepared.options.text_overrides, &names, "text")?;
    namespace_map(&mut prepared.options.position_overrides, &names, "position")?;
    namespace_map(
        &mut prepared.options.position_x_overrides,
        &names,
        "x-position",
    )?;
    namespace_map(
        &mut prepared.options.position_y_overrides,
        &names,
        "y-position",
    )?;
    namespace_map(
        &mut prepared.options.sprite_name_overrides,
        &names,
        "sprite-name",
    )?;
    namespace_map(&mut prepared.options.sprite_overrides, &names, "sprite")?;
    namespace_map(&mut prepared.options.frame_overrides, &names, "frame")?;
    namespace_map(
        &mut prepared.options.progress_overrides,
        &names,
        "progress",
    )?;
    namespace_map(
        &mut prepared.options.runtime_text_by_element,
        &names,
        "runtime-text",
    )?;
    namespace_set(&mut prepared.options.hidden_elements, &names, "visibility")?;
    namespace_map(
        &mut prepared.interaction.visual_enabled,
        &names,
        "enabled interaction",
    )?;
    namespace_map(
        &mut prepared.interaction.click_enabled,
        &names,
        "click-enabled interaction",
    )?;
    namespace_map(
        &mut prepared.interaction.right_click_enabled,
        &names,
        "right-click-enabled interaction",
    )?;

    let mut staged = owner_options.clone();
    let [authored_x, authored_y, _, root_height] = prepared.local_rect;
    let root_name = prepared.root.name.clone();
    let mut root_x = authored_x;
    let mut root_y = authored_y;
    if let Some([x, y]) = prepared.options.position_overrides.remove(&root_name) {
        if prepared.options.position_x_overrides.remove(&root_name).is_some()
            || prepared.options.position_y_overrides.remove(&root_name).is_some()
        {
            return Err(GameUiError::parse(
                "XUI22 root carries conflicting full and axis-specific positions",
            ));
        }
        root_x = x;
        root_y = y;
    } else {
        if let Some(x) = prepared.options.position_x_overrides.remove(&root_name) {
            root_x = x;
        }
        if let Some(y) = prepared.options.position_y_overrides.remove(&root_name) {
            root_y = y;
        }
    }
    let positioned_y = list_y
        .checked_add(root_y)
        .ok_or_else(|| GameUiError::parse("XUI22 decision insertion y overflowed"))?;
    if staged
        .position_overrides
        .insert(root_name, [root_x, positioned_y])
        .is_some()
    {
        return Err(GameUiError::parse(
            "XUI22 root position collides with the owning decision compositor",
        ));
    }

    merge_map(&mut staged.text_overrides, prepared.options.text_overrides, "text")?;
    merge_map(
        &mut staged.position_overrides,
        prepared.options.position_overrides,
        "position",
    )?;
    merge_map(
        &mut staged.position_x_overrides,
        prepared.options.position_x_overrides,
        "x-position",
    )?;
    merge_map(
        &mut staged.position_y_overrides,
        prepared.options.position_y_overrides,
        "y-position",
    )?;
    merge_map(
        &mut staged.sprite_name_overrides,
        prepared.options.sprite_name_overrides,
        "sprite-name",
    )?;
    merge_map(
        &mut staged.sprite_overrides,
        prepared.options.sprite_overrides,
        "sprite",
    )?;
    merge_map(&mut staged.frame_overrides, prepared.options.frame_overrides, "frame")?;
    merge_map(
        &mut staged.progress_overrides,
        prepared.options.progress_overrides,
        "progress",
    )?;
    merge_map(
        &mut staged.runtime_text_by_element,
        prepared.options.runtime_text_by_element,
        "runtime-text",
    )?;
    for hidden in prepared.options.hidden_elements {
        if !staged.hidden_elements.insert(hidden) {
            return Err(GameUiError::parse(
                "XUI22 visibility override collides with the owning decision compositor",
            ));
        }
    }
    *owner_options = staged;
    Ok((prepared.root, root_height, prepared.interaction))
}

/// Join complete d84 rows to exact rendered XUI19 category parents. A runtime
/// row cannot select its parent; the effective FILE `scripted_gui` field is
/// the only admitted identity join.
pub fn prepare_decision_category_scripted_gui_roots(
    content_admission: &ValidatedContentAdmission,
    catalog: &GuiDefinitionCatalog,
    decisions: &DecisionSet,
    panel: &ScriptedGuiObserverPanel,
    rendered_categories: &[RenderedDecisionCategoryParent],
    parent_width: usize,
    parent_height: usize,
) -> Result<Vec<PreparedDecisionCategoryScriptedGuiRoot>> {
    prepare_decision_category_scripted_gui_roots_with_admission(
        &mut |path| {
            content_admission
                .require_used_content(path)
                .map(|_| ())
                .map_err(GameUiError::missing)
        },
        catalog,
        decisions,
        panel,
        rendered_categories,
        parent_width,
        parent_height,
    )
}

fn prepare_decision_category_scripted_gui_roots_with_admission(
    require_admitted: &mut impl FnMut(&str) -> Result<()>,
    catalog: &GuiDefinitionCatalog,
    decisions: &DecisionSet,
    panel: &ScriptedGuiObserverPanel,
    rendered_categories: &[RenderedDecisionCategoryParent],
    parent_width: usize,
    parent_height: usize,
) -> Result<Vec<PreparedDecisionCategoryScriptedGuiRoot>> {
    require_ready_panel(panel)?;
    let parent = checked_parent_rect(parent_width, parent_height)?;

    let mut rendered_ids = BTreeSet::new();
    for category in rendered_categories {
        if !rendered_ids.insert(category.category_id.as_str()) {
            return Err(GameUiError::parse(format!(
                "XUI22 rendered decision category {:?} is duplicated",
                category.category_id
            )));
        }
        if !decisions.categories.contains_key(&category.category_id) {
            return Err(GameUiError::missing(format!(
                "XUI22 rendered decision category {:?} is absent from effective FILE definitions",
                category.category_id
            )));
        }
    }

    let mut rows_by_id = BTreeMap::new();
    for row in &panel.rows {
        if rows_by_id.insert(row.gui_id.as_str(), row).is_some() {
            return Err(GameUiError::parse(format!(
                "XUI22 d84 selected-country rows duplicate definition {:?}",
                row.gui_id
            )));
        }
    }

    let mut bound_visible_ids = BTreeSet::new();
    let mut outputs = Vec::new();
    for rendered in rendered_categories {
        if !rendered.expanded {
            continue;
        }
        let category = &decisions.categories[&rendered.category_id];
        let Some(gui_id) = category.scripted_gui.as_deref() else {
            continue;
        };
        if gui_id.trim().is_empty() {
            return Err(GameUiError::parse(format!(
                "XUI22 category {:?} has an empty scripted_gui binding",
                rendered.category_id
            )));
        }
        let Some(row) = rows_by_id.get(gui_id).copied() else {
            // This is exact only because require_ready_panel requires the
            // authenticated post-owner current-manager snapshot. Absence from
            // that complete engine container is measured uninstantiated, not
            // a missing member of an invented Cartesian product.
            continue;
        };
        match row.evaluation.visible {
            Truth::False => continue,
            Truth::Unavailable => {
                return Err(GameUiError::missing(format!(
                    "XUI22 visibility is unavailable for {gui_id:?}"
                )))
            }
            Truth::True => {}
        }
        if row.context_type.as_deref() != Some("decision_category") {
            return Err(GameUiError::parse(format!(
                "XUI22 category {:?} binds {gui_id:?} with non-decision context {:?}",
                rendered.category_id, row.context_type
            )));
        }
        if !bound_visible_ids.insert(gui_id) {
            return Err(GameUiError::missing(format!(
                "XUI22 visible definition {gui_id:?} resolves to more than one rendered category"
            )));
        }
        require_admitted(&category.source_file)?;
        require_admitted(&row.source_file)?;

        let (window_name, admitted_root) = unique_admitted_root(catalog, row)?;
        require_admitted(&admitted_root.source_file)?;
        let options = options_for_row(&admitted_root.root, row, parent_width, parent_height)?;
        let rect = resolve_element_rect(&admitted_root.root, parent, (0, 0), &options);
        if rect.w() <= 0 || rect.h() <= 0 {
            return Err(GameUiError::parse(format!(
                "XUI22 authored root {window_name:?} has non-positive parent geometry"
            )));
        }
        outputs.push(PreparedDecisionCategoryScriptedGuiRoot {
            category_id: rendered.category_id.clone(),
            category_source_file: category.source_file.clone(),
            manager_order_index: row.manager_order_index,
            gui_id: gui_id.to_string(),
            window_name: window_name.to_string(),
            definition_source_file: row.source_file.clone(),
            root_source_file: admitted_root.source_file.clone(),
            root: admitted_root.root.clone(),
            options,
            local_rect: [rect.x(), rect.y(), rect.w(), rect.h()],
            interaction: interaction_for_row(row)?,
        });
    }

    for row in &panel.rows {
        if row.context_type.as_deref() == Some("decision_category")
            && row.evaluation.visible == Truth::True
            && !bound_visible_ids.contains(row.gui_id.as_str())
        {
            let has_authored_parent = decisions
                .categories
                .values()
                .any(|category| category.scripted_gui.as_deref() == Some(row.gui_id.as_str()));
            if !has_authored_parent {
                return Err(GameUiError::missing(format!(
                    "XUI22 visible definition {:?} has no effective category binding",
                    row.gui_id
                )));
            }
            // A collapsed or absent rendered parent is measured inactive. It
            // must not be relocated into another parent or overlaid globally.
        }
    }
    Ok(outputs)
}

const XUI22_AUTHORED_TOOLTIP_PROPERTIES: [&str; 2] = [
    "pdx_tooltip",
    "pdx_tooltip_delayed",
];

const XUI22_DISABLED_TOOLTIP_PROPERTIES: [&str; 2] = [
    "pdx_disabled_tooltip",
    "pdx_disabled_tooltip_delayed",
];

const XUI22_UNSUPPORTED_TOOLTIP_PROPERTIES: [&str; 5] = [
    "context_aware_tooltip",
    "bound_tooltip",
    "tooltip",
    "tooltiptext",
    "delayedtooltiptext",
];

fn xui22_has_authored_field(element: &GuiElement, key: &str) -> bool {
    element.raw.fields().iter().any(|entry| entry.key_matches(key))
}

fn xui22_effect_owner(effect_name: &str) -> Option<String> {
    let mut owner = effect_name.strip_suffix("_click")?.to_string();
    loop {
        let mut stripped = false;
        for modifier in ["_right", "_alt", "_control", "_shift"] {
            if let Some(prefix) = owner.strip_suffix(modifier) {
                owner = prefix.to_string();
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    (!owner.is_empty()).then_some(owner)
}

fn xui22_dynamic_widgets(
    root: &GuiElement,
    namespace: &str,
    interaction: &ScriptedGuiInteractionState,
) -> Result<Vec<crate::production_renderer::ObserverDecisionCategoryDynamicWidget>> {
    fn visit(
        element: &GuiElement,
        path: &mut Vec<u32>,
        paint_ordinal: &mut u32,
        namespace: &str,
        interaction: &ScriptedGuiInteractionState,
        effects_by_owner: &BTreeMap<String, Vec<String>>,
        output: &mut Vec<crate::production_renderer::ObserverDecisionCategoryDynamicWidget>,
    ) -> Result<()> {
        let ordinal = *paint_ordinal;
        *paint_ordinal = paint_ordinal
            .checked_add(1)
            .ok_or_else(|| GameUiError::parse("XUI22 authored paint order overflowed"))?;
        if let Some(mechanism) = XUI22_UNSUPPORTED_TOOLTIP_PROPERTIES
            .iter()
            .find(|key| xui22_has_authored_field(element, key))
        {
            return Err(GameUiError::missing(format!(
                "XUI22 authored widget {:?} uses unsupported tooltip mechanism {mechanism}",
                element.name
            )));
        }
        if let Some(mechanism) = XUI22_DISABLED_TOOLTIP_PROPERTIES
            .iter()
            .find(|key| xui22_has_authored_field(element, key))
        {
            return Err(GameUiError::missing(format!(
                "XUI22 authored widget {:?} uses {mechanism} but d84 carries no exact a729 vtable +0x208 ordinary/disabled tooltip selector",
                element.name
            )));
        }
        let has_authored_tooltip = XUI22_AUTHORED_TOOLTIP_PROPERTIES
            .iter()
            .any(|key| xui22_has_authored_field(element, key));
        let visual_gate = interaction.visual_enabled.get(&element.name).copied();
        let click_enabled = interaction.click_enabled.get(&element.name).copied();
        let right_click_enabled = interaction
            .right_click_enabled
            .get(&element.name)
            .copied();
        let effect_names = effects_by_owner
            .get(&element.name)
            .cloned()
            .unwrap_or_default();
        if has_authored_tooltip
            || visual_gate.is_some()
            || click_enabled.is_some()
            || right_click_enabled.is_some()
            || !effect_names.is_empty()
        {
            output.push(
                crate::production_renderer::ObserverDecisionCategoryDynamicWidget {
                    paint_ordinal: ordinal,
                    authored_child_path: path.clone(),
                    authored_name: element.name.clone(),
                    composed_name: format!("{}__{}", element.name, namespace),
                    element_type: element.element_type.clone(),
                    has_authored_tooltip,
                    alternate_tooltip_selected:
                        crate::tooltip_compositor::RecordedFact::Unknown {
                            reason: "XUI22 d84 does not carry a729 vtable +0x208; alternate declarations are refused"
                                .into(),
                        },
                    visual_enabled: visual_gate,
                    click_enabled,
                    right_click_enabled,
                    effect_names,
                },
            );
        }
        for (index, child) in element.children.iter().enumerate() {
            path.push(u32::try_from(index).map_err(|_| {
                GameUiError::parse("XUI22 authored child ordinal exceeds u32")
            })?);
            visit(
                child,
                path,
                paint_ordinal,
                namespace,
                interaction,
                effects_by_owner,
                output,
            )?;
            path.pop();
        }
        Ok(())
    }

    let mut effects_by_owner = BTreeMap::<String, Vec<String>>::new();
    for effect_name in interaction.effect_enabled.keys() {
        let owner = xui22_effect_owner(effect_name).ok_or_else(|| {
            GameUiError::parse(format!(
                "XUI22 effect {effect_name:?} has no exact authored click owner"
            ))
        })?;
        effects_by_owner
            .entry(owner)
            .or_default()
            .push(effect_name.clone());
    }
    let mut output = Vec::new();
    let mut paint_ordinal = 0;
    visit(
        root,
        &mut Vec::new(),
        &mut paint_ordinal,
        namespace,
        interaction,
        &effects_by_owner,
        &mut output,
    )?;
    Ok(output)
}

/// Supply exactly one XUI22 child to XUI19's authenticated category callback.
/// XUI19 owns list placement, clipping, scrolling, z-order and Presented
/// lifecycle. This adapter owns only the effective FILE/d84/authored-root join
/// and namespaces the child options before returning it to that parent.
pub fn supply_xui19_decision_category_child(
    content_admission: &ValidatedContentAdmission,
    catalog: &GuiDefinitionCatalog,
    panel: &ScriptedGuiObserverPanel,
    request: &crate::production_renderer::ObserverDecisionCategoryComposeRequest,
    owner_options: &mut ComposeOptions,
) -> Result<crate::production_renderer::ObserverDecisionCategoryDynamicOutcome> {
    let Some(provider_id) = request.provider_id.as_deref() else {
        return Ok(crate::production_renderer::ObserverDecisionCategoryDynamicOutcome::NotBound);
    };
    if provider_id.trim().is_empty() || request.category_source_file.trim().is_empty() {
        return Err(GameUiError::parse(
            "XUI22 XUI19 request has an empty effective provider/source identity",
        ));
    }
    if !request.expanded {
        return Err(GameUiError::parse(
            "XUI22 provider callback was invoked for a collapsed category",
        ));
    }

    // The request is constructed only from XUI19's already parsed effective
    // category definition. Reconstructing this one-row DecisionSet preserves
    // the existing provider's exact FILE join without reparsing content inside
    // every category callback.
    let decisions = DecisionSet {
        categories: BTreeMap::from([(
            request.category_id.clone(),
            crate::decision_chrome::DecisionCategoryDef {
                id: request.category_id.clone(),
                icon_token: None,
                picture: None,
                scripted_gui: Some(provider_id.to_string()),
                has_custom_icons: false,
                custom_icons: Vec::new(),
                source_file: request.category_source_file.clone(),
            },
        )]),
        ..DecisionSet::default()
    };
    let rendered = [RenderedDecisionCategoryParent {
        category_id: request.category_id.clone(),
        expanded: true,
    }];
    let mut prepared_roots = prepare_decision_category_scripted_gui_roots(
        content_admission,
        catalog,
        &decisions,
        panel,
        &rendered,
        request.parent_width,
        request.parent_height,
    )?;
    if prepared_roots.len() > 1 {
        return Err(GameUiError::parse(
            "XUI22 one category produced more than one scripted root",
        ));
    }
    let Some(prepared) = prepared_roots.pop() else {
        // A complete d84 manager snapshot proves the bound provider is not
        // instantiated or is measured invisible for this country/hour.
        return Ok(
            crate::production_renderer::ObserverDecisionCategoryDynamicOutcome::MeasuredInactive,
        );
    };
    let category_source_file = prepared.category_source_file.clone();
    let definition_source_file = prepared.definition_source_file.clone();
    let authored_root_source_file = prepared.root_source_file.clone();
    let manager_order_index = prepared.manager_order_index;
    let width = prepared.local_rect[2];
    let namespace = format!("xui22_{}", request.stable_namespace);
    let widgets = xui22_dynamic_widgets(&prepared.root, &namespace, &prepared.interaction)?;
    let window_name = prepared.window_name.clone();
    let (element, height, interaction) =
        merge_prepared_decision_category_root(prepared, &namespace, 0, owner_options)?;
    let [x, y] = owner_options
        .position_overrides
        .remove(&element.name)
        .ok_or_else(|| GameUiError::missing("XUI22 merged root lost its parent-relative position"))?;
    if width <= 0 || height <= 0 {
        return Err(GameUiError::parse(
            "XUI22 merged root has non-positive authored geometry",
        ));
    }
    Ok(
        crate::production_renderer::ObserverDecisionCategoryDynamicOutcome::Inserted(
            crate::production_renderer::ObserverDecisionCategoryDynamicChild {
                element,
                local_rect: [x, y, width, height],
                category_source_file,
                provider_id: provider_id.to_string(),
                definition_source_file,
                authored_root_source_file,
                window_name,
                manager_order_index,
                interaction: crate::production_renderer::ObserverDecisionCategoryInteraction {
                    root_enabled: interaction.root_enabled,
                    visual_enabled: interaction.visual_enabled,
                    click_enabled: interaction.click_enabled,
                    right_click_enabled: interaction.right_click_enabled,
                    effect_enabled: interaction.effect_enabled,
                },
                widgets,
            },
        ),
    )
}

/// Compose visible roots in authenticated manager order for an exact supplied
/// contextual parent. This is a reusable provider; it is not a shipping route
/// until the owning panel supplies activation, placement and extent.
pub fn compose_scripted_gui_panel(
    resolver: &crate::assets::AssetResolver,
    registry: &SpriteRegistry,
    localisation: &Localisation,
    content_admission: &ValidatedContentAdmission,
    catalog: &GuiDefinitionCatalog,
    panel: &ScriptedGuiObserverPanel,
    parent_width: usize,
    parent_height: usize,
    asset_cache: &mut UiAssetCache,
) -> Result<Vec<ComposedScriptedGuiRoot>> {
    require_ready_panel(panel)?;
    checked_parent_rect(parent_width, parent_height)?;
    let mut outputs = Vec::new();
    for row in &panel.rows {
        match row.evaluation.visible {
            Truth::False => continue,
            Truth::Unavailable => {
                return Err(GameUiError::missing(format!(
                    "XUI22 visibility is unavailable for {:?}",
                    row.gui_id
                )))
            }
            Truth::True => {}
        }
        let (window_name, admitted_root) = unique_admitted_root(catalog, row)?;
        content_admission
            .require_used_content(&admitted_root.source_file)
            .map_err(GameUiError::missing)?;
        let mut options = options_for_row(&admitted_root.root, row, parent_width, parent_height)?
            .with_localisation(localisation);
        options.runtime_view_id = Some(format!("xui22/{}/{}", panel.country_tag, row.gui_id));
        let mut compositor = Compositor::with_cache(resolver, registry, asset_cache);
        let composed = compositor.compose(&admitted_root.root, &options);
        require_complete_report(&composed.report, &row.gui_id)?;
        outputs.push(ComposedScriptedGuiRoot {
            manager_order_index: row.manager_order_index,
            gui_id: row.gui_id.clone(),
            window_name: window_name.to_string(),
            source_file: admitted_root.source_file.clone(),
            image: composed.image,
            report: composed.report,
            interaction: interaction_for_row(row)?,
        });
    }
    Ok(outputs)
}

fn require_ready_panel(panel: &ScriptedGuiObserverPanel) -> Result<()> {
    if panel.runtime_authority != ScriptedGuiRuntimeAuthorityState::Present
        || panel.presentation != ExactPresentationState::Ready
    {
        return Err(GameUiError::missing(
            "XUI22 composition requires a complete admitted d84 current-manager snapshot",
        ));
    }
    Ok(())
}

fn checked_parent_rect(width: usize, height: usize) -> Result<Rect> {
    if width == 0 || height == 0 {
        return Err(GameUiError::parse("XUI22 contextual parent extent is empty"));
    }
    let width = i32::try_from(width)
        .map_err(|_| GameUiError::parse("XUI22 parent width exceeds i32"))?;
    let height = i32::try_from(height)
        .map_err(|_| GameUiError::parse("XUI22 parent height exceeds i32"))?;
    Ok(Rect::new(0, 0, width, height))
}

fn unique_admitted_root<'a, 'b>(
    catalog: &'a GuiDefinitionCatalog,
    row: &'b ScriptedGuiObserverRow,
) -> Result<(
    &'b str,
    &'a crate::definition_admission::EffectiveGuiRootDefinition,
)> {
    let window_name = row.window_name.as_deref().ok_or_else(|| {
        GameUiError::missing(format!(
            "XUI22 definition {:?} has no authored window binding",
            row.gui_id
        ))
    })?;
    let roots = catalog
        .effective_roots
        .iter()
        .filter(|root| root.admitted && root.root.name == window_name)
        .collect::<Vec<_>>();
    if roots.len() != 1 {
        return Err(GameUiError::missing(format!(
            "XUI22 authored window {window_name:?} resolves to {} admitted roots",
            roots.len()
        )));
    }
    Ok((window_name, roots[0]))
}

fn require_complete_report(report: &ComposeReport, gui_id: &str) -> Result<()> {
    if !report.missing_sprites.is_empty()
        || !report.undecodable_sprites.is_empty()
        || !report.missing_fonts.is_empty()
        || !report.missing_glyphs.is_empty()
        || !report.unresolved_loc_keys.is_empty()
        || !report.unresolved_text_inputs.is_empty()
    {
        return Err(GameUiError::missing(format!(
            "XUI22 composition for {gui_id:?} is incomplete: missing_sprites={:?}; undecodable_sprites={:?}; missing_fonts={:?}; missing_glyphs={:?}; unresolved_localisation={:?}; unresolved_runtime_text={:?}",
            report.missing_sprites,
            report.undecodable_sprites,
            report.missing_fonts,
            report.missing_glyphs,
            report.unresolved_loc_keys,
            report.unresolved_text_inputs,
        )));
    }
    Ok(())
}

fn options_for_row(
    root: &GuiElement,
    row: &ScriptedGuiObserverRow,
    parent_width: usize,
    parent_height: usize,
) -> Result<ComposeOptions> {
    let mut options =
        ComposeOptions::new(parent_width, parent_height).with_root_size(parent_width, parent_height);
    let element_counts = named_element_counts(root);

    for (trigger_name, truth) in &row.evaluation.triggers {
        if let Some(element_name) = trigger_name.strip_suffix("_visible") {
            require_unique_element(&element_counts, element_name, trigger_name)?;
            match truth {
                Truth::True => {}
                Truth::False => {
                    options.hidden_elements.insert(element_name.to_string());
                }
                Truth::Unavailable => {
                    return Err(GameUiError::missing(format!(
                        "XUI22 trigger {trigger_name:?} is unavailable"
                    )))
                }
            }
        } else if let Some(element_name) = trigger_name.strip_suffix("_right_click_enabled") {
            require_unique_element(&element_counts, element_name, trigger_name)?;
        } else if let Some(element_name) = trigger_name.strip_suffix("_click_enabled") {
            require_unique_element(&element_counts, element_name, trigger_name)?;
        } else if let Some(element_name) = trigger_name.strip_suffix("_enabled") {
            require_unique_element(&element_counts, element_name, trigger_name)?;
        } else {
            return Err(GameUiError::parse(format!(
                "XUI22 trigger {trigger_name:?} has no generic widget binding"
            )));
        }
    }

    for (key, value) in &row.evaluation.properties {
        let Some((element_name, property_name)) = key.rsplit_once('.') else {
            return Err(GameUiError::parse(format!(
                "XUI22 property key {key:?} is not element.property"
            )));
        };
        require_unique_element(&element_counts, element_name, key)?;
        match (property_name.to_ascii_lowercase().as_str(), value) {
            ("image", EvaluatedProperty::String { value, .. }) if !value.trim().is_empty() => {
                options
                    .sprite_name_overrides
                    .insert(element_name.to_string(), value.clone());
            }
            ("frame", value) => {
                options
                    .frame_overrides
                    .insert(element_name.to_string(), exact_one_based_frame(value, key)?);
            }
            ("x", value) => {
                options
                    .position_x_overrides
                    .insert(element_name.to_string(), exact_position(value, key)?);
            }
            ("y", value) => {
                options
                    .position_y_overrides
                    .insert(element_name.to_string(), exact_position(value, key)?);
            }
            (_, EvaluatedProperty::Unavailable) => {
                return Err(GameUiError::missing(format!(
                    "XUI22 property {key:?} is unavailable"
                )))
            }
            _ => {
                return Err(GameUiError::parse(format!(
                    "XUI22 property {key:?} has no exact generic compositor law"
                )))
            }
        }
    }
    Ok(options)
}

fn exact_one_based_frame(value: &EvaluatedProperty, key: &str) -> Result<usize> {
    let value = exact_integral_number(value, key, 1.0, usize::MAX as f64)?;
    Ok(value as usize - 1)
}

fn exact_position(value: &EvaluatedProperty, key: &str) -> Result<i32> {
    Ok(exact_integral_number(value, key, i32::MIN as f64, i32::MAX as f64)? as i32)
}

fn exact_integral_number(
    value: &EvaluatedProperty,
    key: &str,
    minimum: f64,
    maximum: f64,
) -> Result<f64> {
    let value = match value {
        EvaluatedProperty::Number { value } => *value,
        EvaluatedProperty::String { value, .. } => value.trim().parse::<f64>().map_err(|_| {
            GameUiError::parse(format!("XUI22 numeric property {key:?} is not a number"))
        })?,
        EvaluatedProperty::Unavailable => {
            return Err(GameUiError::missing(format!(
                "XUI22 property {key:?} is unavailable"
            )))
        }
    };
    if !value.is_finite() || value.fract() != 0.0 || value < minimum || value > maximum {
        return Err(GameUiError::parse(format!(
            "XUI22 numeric property {key:?} is not an in-range integer"
        )));
    }
    Ok(value)
}

fn interaction_for_row(row: &ScriptedGuiObserverRow) -> Result<ScriptedGuiInteractionState> {
    let root_enabled = exact_truth(row.evaluation.enabled, "root enabled")?;
    let mut visual_enabled = BTreeMap::new();
    let mut click_enabled = BTreeMap::new();
    let mut right_click_enabled = BTreeMap::new();
    for (name, truth) in &row.evaluation.triggers {
        if let Some(element) = name.strip_suffix("_right_click_enabled") {
            if right_click_enabled
                .insert(element.to_string(), exact_truth(*truth, name)?)
                .is_some()
            {
                return Err(GameUiError::parse(format!(
                    "XUI22 duplicate right-click-enabled widget {element:?}"
                )));
            }
        } else if let Some(element) = name.strip_suffix("_click_enabled") {
            if click_enabled
                .insert(element.to_string(), exact_truth(*truth, name)?)
                .is_some()
            {
                return Err(GameUiError::parse(format!(
                    "XUI22 duplicate click-enabled widget {element:?}"
                )));
            }
        } else if let Some(element) = name.strip_suffix("_enabled") {
            if visual_enabled
                .insert(element.to_string(), exact_truth(*truth, name)?)
                .is_some()
            {
                return Err(GameUiError::parse(format!(
                    "XUI22 duplicate enabled widget {element:?}"
                )));
            }
        }
    }
    let effect_enabled = row
        .evaluation
        .effect_enabled
        .iter()
        .map(|(name, truth)| Ok((name.clone(), exact_truth(*truth, name)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(ScriptedGuiInteractionState {
        root_enabled,
        visual_enabled,
        click_enabled,
        right_click_enabled,
        effect_enabled,
    })
}

fn exact_truth(value: Truth, label: &str) -> Result<bool> {
    match value {
        Truth::True => Ok(true),
        Truth::False => Ok(false),
        Truth::Unavailable => Err(GameUiError::missing(format!(
            "XUI22 {label} result is unavailable"
        ))),
    }
}

fn named_element_counts(root: &GuiElement) -> BTreeMap<String, usize> {
    fn visit(element: &GuiElement, counts: &mut BTreeMap<String, usize>) {
        *counts.entry(element.name.clone()).or_default() += 1;
        for child in &element.children {
            visit(child, counts);
        }
    }
    let mut counts = BTreeMap::new();
    visit(root, &mut counts);
    counts
}

fn require_unique_element(
    counts: &BTreeMap<String, usize>,
    element_name: &str,
    binding_name: &str,
) -> Result<()> {
    match counts.get(element_name).copied() {
        Some(1) => Ok(()),
        Some(count) => Err(GameUiError::parse(format!(
            "XUI22 binding {binding_name:?} resolves to {count} authored elements"
        ))),
        None => Err(GameUiError::missing(format!(
            "XUI22 binding {binding_name:?} names no authored element"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision_chrome::DecisionCategoryDef;
    use crate::definition_admission::EffectiveGuiRootDefinition;
    use crate::gui::{GuiElement, LayoutSet, Measure, MeasurePair};
    use crate::observer_panels::ObserverSemanticGap;
    use crate::scripted_gui_runtime::ScriptedGuiEvaluation;

    fn element(name: &str, children: Vec<GuiElement>) -> GuiElement {
        GuiElement {
            element_type: "iconType".into(),
            name: name.into(),
            position: None,
            size: None,
            orientation: None,
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children,
            raw: crate::paradox_text::Value::Object(Vec::new()),
        }
    }

    fn row() -> ScriptedGuiObserverRow {
        ScriptedGuiObserverRow {
            manager_order_index: 4,
            gui_id: "panel".into(),
            source_file: "common/scripted_guis/panel.txt".into(),
            context_type: Some("decision_category".into()),
            window_name: Some("panel_window".into()),
            evaluation: ScriptedGuiEvaluation {
                visible: Truth::True,
                enabled: Truth::True,
                triggers: BTreeMap::from([
                    ("icon_visible".into(), Truth::False),
                    ("icon_click_enabled".into(), Truth::True),
                ]),
                properties: BTreeMap::from([
                    (
                        "icon.image".into(),
                        EvaluatedProperty::String {
                            value: "GFX_exact".into(),
                            captured: true,
                        },
                    ),
                    ("icon.frame".into(), EvaluatedProperty::Number { value: 2.0 }),
                    ("icon.x".into(), EvaluatedProperty::Number { value: 7.0 }),
                ]),
                effect_enabled: BTreeMap::from([("icon_click".into(), Truth::False)]),
            },
            presentation: ExactPresentationState::Ready,
        }
    }

    fn measure(raw: &str, is_percent: bool) -> Measure {
        Measure {
            raw: raw.into(),
            value: Some(raw.into()),
            is_percent,
        }
    }

    fn prepared_fixture() -> (
        GuiDefinitionCatalog,
        DecisionSet,
        ScriptedGuiObserverPanel,
        Vec<RenderedDecisionCategoryParent>,
        BTreeSet<String>,
    ) {
        let mut root = element("panel_window", vec![element("icon", vec![])]);
        root.position = Some(MeasurePair {
            x: measure("11", false),
            y: measure("7", false),
        });
        root.size = Some(MeasurePair {
            x: measure("100", true),
            y: measure("30", false),
        });
        let category_source = "common/decisions/categories/fixture.txt".to_string();
        let definition_source = "common/scripted_guis/panel.txt".to_string();
        let root_source = "interface/fixture.gui".to_string();
        let catalog = GuiDefinitionCatalog {
            layouts: LayoutSet::default(),
            files: Vec::new(),
            admitted_windows: Vec::new(),
            rejected_windows: Vec::new(),
            effective_roots: vec![EffectiveGuiRootDefinition {
                source_file: root_source.clone(),
                window_id: "panel_window".into(),
                root,
                admitted: true,
                unsupported: BTreeSet::new(),
            }],
        };
        let decisions = DecisionSet {
            categories: BTreeMap::from([(
                "fixture_category".into(),
                DecisionCategoryDef {
                    id: "fixture_category".into(),
                    icon_token: None,
                    picture: None,
                    scripted_gui: Some("panel".into()),
                    has_custom_icons: false,
                    custom_icons: Vec::new(),
                    source_file: category_source.clone(),
                },
            )]),
            ..DecisionSet::default()
        };
        let panel = ScriptedGuiObserverPanel {
            country_tag: "USA".into(),
            registry_schema: "scripted_gui_registry_v1".into(),
            effective_file_count: 1,
            runtime_authority: ScriptedGuiRuntimeAuthorityState::Present,
            rows: vec![row()],
            presentation: ExactPresentationState::Ready,
        };
        let rendered = vec![RenderedDecisionCategoryParent {
            category_id: "fixture_category".into(),
            expanded: true,
        }];
        let admitted = BTreeSet::from([category_source, definition_source, root_source]);
        (catalog, decisions, panel, rendered, admitted)
    }

    fn merge_fixture() -> PreparedDecisionCategoryScriptedGuiRoot {
        let root = element("panel_window", vec![element("icon", vec![])]);
        let mut options = ComposeOptions::new(502, 700);
        options.position_x_overrides.insert("panel_window".into(), 11);
        options.position_y_overrides.insert("panel_window".into(), 7);
        options
            .sprite_name_overrides
            .insert("icon".into(), "GFX_exact".into());
        options.hidden_elements.insert("icon".into());
        PreparedDecisionCategoryScriptedGuiRoot {
            category_id: "fixture_category".into(),
            category_source_file: "common/decisions/categories/fixture.txt".into(),
            manager_order_index: 4,
            gui_id: "panel".into(),
            window_name: "panel_window".into(),
            definition_source_file: "common/scripted_guis/panel.txt".into(),
            root_source_file: "interface/fixture.gui".into(),
            root,
            options,
            local_rect: [0, 0, 502, 30],
            interaction: ScriptedGuiInteractionState {
                root_enabled: true,
                visual_enabled: BTreeMap::new(),
                click_enabled: BTreeMap::from([("icon".into(), true)]),
                right_click_enabled: BTreeMap::new(),
                effect_enabled: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn exact_widget_bindings_map_without_visual_defaults() {
        let root = element("panel_window", vec![element("icon", vec![])]);
        let row = row();
        let options = options_for_row(&root, &row, 200, 100).unwrap();
        assert!(options.hidden_elements.contains("icon"));
        assert_eq!(options.sprite_name_overrides["icon"], "GFX_exact");
        assert_eq!(options.frame_overrides["icon"], 1);
        assert_eq!(options.position_x_overrides["icon"], 7);
        let interaction = interaction_for_row(&row).unwrap();
        assert!(interaction.root_enabled);
        assert!(interaction.click_enabled["icon"]);
        assert!(!interaction.effect_enabled["icon_click"]);
    }

    #[test]
    fn unknown_property_has_no_generic_fallback() {
        let root = element("panel_window", vec![element("icon", vec![])]);
        let mut row = row();
        row.evaluation.properties = BTreeMap::from([(
            "icon.opacity".into(),
            EvaluatedProperty::Number { value: 0.5 },
        )]);
        assert!(options_for_row(&root, &row, 200, 100)
            .unwrap_err()
            .to_string()
            .contains("no exact generic compositor law"));
    }

    #[test]
    fn duplicate_authored_element_identity_refuses_binding() {
        let root = element(
            "panel_window",
            vec![element("icon", vec![]), element("icon", vec![])],
        );
        assert!(options_for_row(&root, &row(), 200, 100)
            .unwrap_err()
            .to_string()
            .contains("2 authored elements"));
    }

    #[test]
    fn incomplete_panel_cannot_enter_any_pixel_provider() {
        let panel = ScriptedGuiObserverPanel {
            country_tag: "USA".into(),
            registry_schema: "test".into(),
            effective_file_count: 1,
            runtime_authority: ScriptedGuiRuntimeAuthorityState::Present,
            rows: vec![row()],
            presentation: ExactPresentationState::blocked([
                ObserverSemanticGap::D84ScriptedGuiManagerSnapshot,
            ]),
        };
        assert!(require_ready_panel(&panel).is_err());
    }

    #[test]
    fn category_insertion_namespaces_and_advances_authored_height() {
        let mut owner = ComposeOptions::new(1920, 1080);
        let (root, height, interaction) = merge_prepared_decision_category_root(
            merge_fixture(),
            "xui22_category_3",
            20,
            &mut owner,
        )
        .unwrap();
        assert_eq!(root.name, "panel_window__xui22_category_3");
        assert_eq!(height, 30);
        assert_eq!(owner.position_overrides[&root.name], [11, 27]);
        assert_eq!(
            owner.sprite_name_overrides["icon__xui22_category_3"],
            "GFX_exact"
        );
        assert!(owner.hidden_elements.contains("icon__xui22_category_3"));
        assert!(interaction.click_enabled["icon__xui22_category_3"]);
    }

    #[test]
    fn category_insertion_is_transactional_on_late_collision() {
        let mut owner = ComposeOptions::new(1920, 1080);
        owner
            .hidden_elements
            .insert("icon__xui22_category_3".into());
        let before = owner.clone();
        assert!(merge_prepared_decision_category_root(
            merge_fixture(),
            "xui22_category_3",
            20,
            &mut owner,
        )
        .is_err());
        assert_eq!(owner, before);
    }

    #[test]
    fn category_provider_joins_file_runtime_and_authored_root() {
        let (catalog, decisions, panel, rendered, admitted) = prepared_fixture();
        let prepared = prepare_decision_category_scripted_gui_roots_with_admission(
            &mut |path| {
                admitted
                    .contains(path)
                    .then_some(())
                    .ok_or_else(|| GameUiError::missing(format!("unadmitted test path {path:?}")))
            },
            &catalog,
            &decisions,
            &panel,
            &rendered,
            502,
            700,
        )
        .unwrap();
        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].category_id, "fixture_category");
        assert_eq!(prepared[0].gui_id, "panel");
        assert_eq!(prepared[0].local_rect, [11, 7, 502, 30]);
    }

    #[test]
    fn descendant_targets_keep_authored_and_namespaced_presented_identities() {
        let mut root = element("panel_window", vec![element("icon", vec![])]);
        root.children[0].raw = crate::paradox_text::parse_str(
            "pdx_tooltip = EXACT_TOOLTIP",
        )
        .root;
        let interaction = ScriptedGuiInteractionState {
            root_enabled: true,
            visual_enabled: BTreeMap::new(),
            click_enabled: BTreeMap::from([("icon".into(), false)]),
            right_click_enabled: BTreeMap::new(),
            effect_enabled: BTreeMap::from([("icon_click".into(), false)]),
        };
        let widgets = xui22_dynamic_widgets(&root, "xui22_xui19_category_2", &interaction)
            .expect("exact descendants are admitted");
        assert_eq!(widgets.len(), 1);
        let widget = &widgets[0];
        assert_eq!(widget.authored_child_path, [0]);
        assert_eq!(widget.authored_name, "icon");
        assert_eq!(widget.composed_name, "icon__xui22_xui19_category_2");
        assert!(widget.has_authored_tooltip);
        assert!(matches!(
            &widget.alternate_tooltip_selected,
            crate::tooltip_compositor::RecordedFact::Unknown { .. }
        ));
        assert_eq!(widget.click_enabled, Some(false));
        assert_eq!(widget.effect_names, ["icon_click"]);
    }

    #[test]
    fn disabled_tooltip_without_exact_a729_selector_fails_closed() {
        let mut root = element("panel_window", vec![element("icon", vec![])]);
        root.children[0].raw = crate::paradox_text::parse_str(
            "pdx_tooltip = EXACT_TOOLTIP pdx_disabled_tooltip = EXACT_DISABLED",
        )
        .root;
        let error = xui22_dynamic_widgets(
            &root,
            "xui22_xui19_category_2",
            &ScriptedGuiInteractionState {
                root_enabled: true,
                visual_enabled: BTreeMap::new(),
                click_enabled: BTreeMap::from([("icon".into(), false)]),
                right_click_enabled: BTreeMap::new(),
                effect_enabled: BTreeMap::new(),
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("vtable +0x208 ordinary/disabled tooltip selector"));
    }

    #[test]
    fn unsupported_custom_tooltip_provider_fails_closed() {
        let mut root = element("panel_window", vec![element("icon", vec![])]);
        root.children[0].raw = crate::paradox_text::parse_str("tooltip = CUSTOM_PROVIDER").root;
        let error = xui22_dynamic_widgets(
            &root,
            "xui22_xui19_category_2",
            &ScriptedGuiInteractionState {
                root_enabled: true,
                visual_enabled: BTreeMap::new(),
                click_enabled: BTreeMap::new(),
                right_click_enabled: BTreeMap::new(),
                effect_enabled: BTreeMap::new(),
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("unsupported tooltip mechanism"));
    }
}
