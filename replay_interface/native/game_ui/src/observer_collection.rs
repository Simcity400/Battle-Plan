
use crate::compose::{
    measure_to_pixels_for_parent, ComposeOptions, ComposeOutput, ComposeReport, Compositor,
    Orientation, Rect,
};
use crate::dds::RgbaImage;
use crate::font::alpha_blend;
use crate::gui::{parse_measure_pair, GuiElement};
use crate::gui_scene_util::prefix_element_names;
use crate::paradox_text::{scalar_f32, scalar_i32};
use crate::widget_layout::{gridbox_layout, listbox_layout, GridBoxLayout, ListFlow};
use crate::{ErrorKind, GameUiError, Result};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverSemanticCollection {
    pub collection_instance_token: String,
    pub frame_hour: u64,
    pub context_country: Option<String>,
    pub complete: bool,
    pub rows: Vec<ObserverSemanticRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverSemanticRow {
    pub stable_item_id: String,
    pub source_ordinal: u32,
    pub subtype_id: String,
}

impl ObserverSemanticCollection {
    pub fn validate(&self, expected_hour: u64) -> Result<()> {
        nonempty(&self.collection_instance_token, "collection_instance_token")?;
        if self.frame_hour != expected_hour {
            return Err(fail("collection frame_hour differs from the replay frame"));
        }
        if self
            .context_country
            .as_deref()
            .is_some_and(|tag| tag.trim().is_empty())
        {
            return Err(fail("context_country is blank"));
        }
        if !self.complete {
            return Err(fail(
                "panel-specific semantic membership is not explicitly complete",
            ));
        }
        let mut ids = BTreeSet::new();
        for (ordinal, row) in self.rows.iter().enumerate() {
            let expected = u32::try_from(ordinal)
                .map_err(|_| fail("collection exceeds the u32 source-ordinal space"))?;
            if row.source_ordinal != expected {
                return Err(fail("source ordinals are not contiguous from zero"));
            }
            nonempty(&row.stable_item_id, "stable_item_id")?;
            nonempty(&row.subtype_id, "subtype_id")?;
            if !ids.insert(row.stable_item_id.as_str()) {
                return Err(fail("duplicate stable_item_id"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AuthoredCollectionSource<'a> {
    pub owning_window_id: &'a str,
    pub root_source_identity: &'a str,
    pub collection_element_id: &'a str,
    pub collection_element: &'a GuiElement,
    pub effective_row_template_identity: Option<&'a str>,
    pub effective_row_template: Option<&'a GuiElement>,
    pub parent_rect: Rect,
    pub grid_authority: Option<StandardGridAuthority>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardGridAuthority {
    Xui26FrontendExactCarrierOrder,
    /// XUI13 ship/tank/plane `acquired_variants_grid` generic append through
    Xui13AcquiredVariantsStandardAutoAppend,
    Xui13VariantCountryFilterStandardAutoAppend,
    Xui15CountryTradeStandardAutoAppend,
    Xui19DecisionStandardAutoAppend,
}

#[derive(Clone, Copy, Debug)]
pub struct AuthoredRowBinding<'a> {
    pub stable_item_id: &'a str,
    pub subtype_id: &'a str,
    pub element_source_identity: &'a str,
    pub instance_token: &'a str,
    pub template: &'a GuiElement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewerCollectionState {
    pub scroll_offset_pixels: (i32, i32),
    pub window_stack_ordinal: u32,
    pub root_paint_ordinal: u32,
    pub visible_clip: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObserverCollectionFamily {
    List,
    SmoothList,
    Grid,
    Overlap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CollectionPaintOrder {
    pub window_stack_ordinal: u32,
    pub root_paint_ordinal: u32,
    pub local_paint_ordinal: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlannedCollectionRow {
    stable_item_id: String,
    subtype_id: String,
    source_ordinal: u32,
    owning_window_id: String,
    root_source_identity: String,
    element_source_identity: String,
    instance_token: String,
    viewport_position_pixels: (i32, i32),
    final_rect: Rect,
    visible_clip: Rect,
    visible_rect: Option<Rect>,
    paint_order: CollectionPaintOrder,
    template: GuiElement,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverCollectionPlan {
    family: ObserverCollectionFamily,
    collection_instance_token: String,
    collection_element_id: String,
    collection_rect: Rect,
    visible_clip: Rect,
    content_extent_pixels: (i32, i32),
    viewport_size_pixels: (usize, usize),
    root_size_pixels: (usize, usize),
    rows: Vec<PlannedCollectionRow>,
}

#[derive(Clone, Copy, Debug)]
pub struct ObserverRowComposeInput<'a> {
    pub stable_item_id: &'a str,
    pub options: &'a ComposeOptions,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComposedObserverCollection {
    pub output: ComposeOutput,
    pub painted_rows: Vec<String>,
}

impl ObserverCollectionPlan {
    pub fn family(&self) -> ObserverCollectionFamily {
        self.family
    }

    pub fn collection_instance_token(&self) -> &str {
        &self.collection_instance_token
    }

    pub fn collection_element_id(&self) -> &str {
        &self.collection_element_id
    }

    pub fn collection_rect(&self) -> Rect {
        self.collection_rect
    }

    pub fn visible_clip(&self) -> Rect {
        self.visible_clip
    }

    pub fn content_extent_pixels(&self) -> (i32, i32) {
        self.content_extent_pixels
    }

    pub fn rows(&self) -> impl Iterator<Item = &PlannedCollectionRow> {
        self.rows.iter()
    }

    pub fn visible_rows(&self) -> impl Iterator<Item = &PlannedCollectionRow> {
        self.rows().filter(|row| row.visible_rect.is_some())
    }
}

impl PlannedCollectionRow {
    pub fn stable_item_id(&self) -> &str {
        &self.stable_item_id
    }

    pub fn subtype_id(&self) -> &str {
        &self.subtype_id
    }

    pub fn source_ordinal(&self) -> u32 {
        self.source_ordinal
    }

    pub fn owning_window_id(&self) -> &str {
        &self.owning_window_id
    }

    pub fn root_source_identity(&self) -> &str {
        &self.root_source_identity
    }

    pub fn element_source_identity(&self) -> &str {
        &self.element_source_identity
    }

    pub fn instance_token(&self) -> &str {
        &self.instance_token
    }

    pub fn viewport_position_pixels(&self) -> (i32, i32) {
        self.viewport_position_pixels
    }

    pub fn final_rect(&self) -> Rect {
        self.final_rect
    }

    pub fn visible_clip(&self) -> Rect {
        self.visible_clip
    }

    pub fn visible_rect(&self) -> Option<Rect> {
        self.visible_rect
    }

    pub fn paint_order(&self) -> CollectionPaintOrder {
        self.paint_order
    }
}

pub fn clone_rows_for_composition(
    plan: &ObserverCollectionPlan,
) -> Result<Vec<(PlannedCollectionRow, GuiElement)>> {
    plan.rows()
        .map(|row| Ok((row.clone(), row.template.clone())))
        .collect()
}

pub fn compose_observer_row_layers(
    compositor: &mut Compositor<'_>,
    plan: &ObserverCollectionPlan,
    inputs: &[ObserverRowComposeInput<'_>],
) -> Result<ComposedObserverCollection> {
    let input_by_id = inputs
        .iter()
        .map(|input| (input.stable_item_id, input))
        .collect::<BTreeMap<_, _>>();
    if input_by_id.len() != inputs.len() {
        return Err(fail("duplicate row compose input stable_item_id"));
    }
    if inputs.len() != plan.rows().count() {
        return Err(fail(
            "row compose inputs do not exactly cover planned semantic rows",
        ));
    }

    let mut output = ComposeOutput {
        image: RgbaImage {
            width: plan.viewport_size_pixels.0,
            height: plan.viewport_size_pixels.1,
            pixels: vec![
                0;
                plan.viewport_size_pixels
                    .0
                    .checked_mul(plan.viewport_size_pixels.1)
                    .and_then(|pixels| pixels.checked_mul(4))
                    .ok_or_else(|| fail("collection viewport buffer overflows"))?
            ],
        },
        report: ComposeReport::default(),
        debug_rects: Vec::new(),
    };
    let mut painted_rows = Vec::with_capacity(inputs.len());
    let viewport_rect = Rect::new(
        0,
        0,
        i32::try_from(plan.viewport_size_pixels.0)
            .map_err(|_| fail("viewport width overflows i32"))?,
        i32::try_from(plan.viewport_size_pixels.1)
            .map_err(|_| fail("viewport height overflows i32"))?,
    );
    let render_clip = intersection(plan.visible_clip, viewport_rect);
    let mut rows = plan.rows().collect::<Vec<_>>();
    rows.sort_by_key(|row| row.paint_order);
    for row in rows {
        let input = input_by_id
            .get(row.stable_item_id.as_str())
            .ok_or_else(|| fail("planned row has no panel compose input"))?;
        nonempty(input.stable_item_id, "compose input stable_item_id")?;
        if input.options.viewport_width == 0 || input.options.viewport_height == 0 {
            return Err(fail("row compose viewport is empty"));
        }
        if (input.options.viewport_width, input.options.viewport_height)
            != plan.viewport_size_pixels
            || (input.options.root_width, input.options.root_height) != plan.root_size_pixels
        {
            return Err(fail(
                "row compose coordinate space differs from the admitted plan",
            ));
        }
        let Some(render_clip) = render_clip else {
            continue;
        };
        let template = row.template.clone();
        let mut options = input.options.clone();
        prefix_row_options(&mut options, &format!("{}::", row.instance_token));
        options.viewport_width =
            usize::try_from(render_clip.w()).map_err(|_| fail("render clip width overflows"))?;
        options.viewport_height =
            usize::try_from(render_clip.h()).map_err(|_| fail("render clip height overflows"))?;
        options.position_overrides.insert(
            template.name.clone(),
            [
                row.final_rect
                    .x()
                    .checked_sub(render_clip.x())
                    .ok_or_else(|| fail("cropped row x overflows"))?,
                row.final_rect
                    .y()
                    .checked_sub(render_clip.y())
                    .ok_or_else(|| fail("cropped row y overflows"))?,
            ],
        );
        let mut layer = compositor.compose(&template, &options);
        let local_clip = Rect::new(0, 0, render_clip.w(), render_clip.h());
        let reaches_clip = layer.debug_rects.iter().any(|debug| {
            intersection(Rect::new(debug.x, debug.y, debug.w, debug.h), local_clip).is_some()
        });
        if !reaches_clip {
            continue;
        }
        for debug in &mut layer.debug_rects {
            debug.x = debug
                .x
                .checked_add(render_clip.x())
                .ok_or_else(|| fail("debug rect x overflows"))?;
            debug.y = debug
                .y
                .checked_add(render_clip.y())
                .ok_or_else(|| fail("debug rect y overflows"))?;
        }
        merge_cropped_layer(&mut output, layer, render_clip)?;
        painted_rows.push(row.stable_item_id.clone());
    }
    Ok(ComposedObserverCollection {
        output,
        painted_rows,
    })
}

fn prefix_row_options(options: &mut ComposeOptions, prefix: &str) {
    fn map_keys<T>(map: &mut BTreeMap<String, T>, prefix: &str) {
        *map = std::mem::take(map)
            .into_iter()
            .map(|(name, value)| (format!("{prefix}{name}"), value))
            .collect();
    }
    map_keys(&mut options.text_overrides, prefix);
    map_keys(&mut options.position_overrides, prefix);
    map_keys(&mut options.sprite_name_overrides, prefix);
    map_keys(&mut options.sprite_overrides, prefix);
    map_keys(&mut options.frame_overrides, prefix);
    map_keys(&mut options.progress_overrides, prefix);
    map_keys(&mut options.runtime_text_by_element, prefix);
    options.hidden_elements = std::mem::take(&mut options.hidden_elements)
        .into_iter()
        .map(|name| format!("{prefix}{name}"))
        .collect();
}

pub fn plan_observer_collection(
    expected_hour: u64,
    semantic: &ObserverSemanticCollection,
    source: AuthoredCollectionSource<'_>,
    bindings: &[AuthoredRowBinding<'_>],
    viewer: ViewerCollectionState,
    options: &ComposeOptions,
) -> Result<ObserverCollectionPlan> {
    semantic.validate(expected_hour)?;
    validate_source(source)?;
    positive_rect(viewer.visible_clip, "visible_clip")?;
    checked_rect(viewer.visible_clip, "visible_clip")?;
    validate_coordinate_space(options)?;
    let family = family(source.collection_element)?;
    if family == ObserverCollectionFamily::Grid {
        validate_grid_authority(source)?;
    }
    if family != ObserverCollectionFamily::Grid && source.grid_authority.is_some() {
        return Err(fail(
            "standard-grid authority witness does not match collection family",
        ));
    }
    let collection_rect = checked_collection_rect(source.collection_element, source.parent_rect)?;
    positive_rect(collection_rect, "collection_rect")?;
    checked_rect(collection_rect, "collection_rect")?;
    let visible_clip = viewer.visible_clip;

    let binding_by_id = validate_bindings(semantic, bindings)?;
    if family == ObserverCollectionFamily::Grid {
        validate_grid_row_templates(source, bindings)?;
    }
    let row_sizes = semantic
        .rows
        .iter()
        .map(|row| {
            let binding = binding_by_id
                .get(row.stable_item_id.as_str())
                .expect("validated exact key coverage");
            template_size(binding.template, collection_rect)
        })
        .collect::<Result<Vec<_>>>()?;

    let (unscrolled, content_extent_pixels, local_paint_ordinals) = match family {
        ObserverCollectionFamily::List => {
            let (rects, extent) = plan_list(
                source.collection_element,
                source.parent_rect,
                collection_rect,
                &row_sizes,
                options,
            )?;
            (rects, extent, source_paint_ordinals(row_sizes.len())?)
        }
        ObserverCollectionFamily::SmoothList => {
            let (rects, extent) = plan_smooth_list(
                source.collection_element,
                source.parent_rect,
                collection_rect,
                &row_sizes,
                options,
            )?;
            (rects, extent, source_paint_ordinals(row_sizes.len())?)
        }
        ObserverCollectionFamily::Grid => {
            let (rects, extent) = plan_grid(
                source.collection_element,
                source.parent_rect,
                &row_sizes,
                options,
            )?;
            (rects, extent, source_paint_ordinals(row_sizes.len())?)
        }
        ObserverCollectionFamily::Overlap => {
            plan_overlap(source.collection_element, collection_rect, &row_sizes)?
        }
    };
    if unscrolled.len() != semantic.rows.len() {
        return Err(fail("layout did not account for every semantic row"));
    }
    validate_scroll(
        family,
        source,
        source.parent_rect,
        options,
        collection_rect,
        content_extent_pixels,
        viewer.scroll_offset_pixels,
        viewer.visible_clip,
    )?;

    let mut rows = Vec::with_capacity(bindings.len());
    for ((row, rect), local_paint_ordinal) in semantic
        .rows
        .iter()
        .zip(unscrolled)
        .zip(local_paint_ordinals)
    {
        let binding = binding_by_id
            .get(row.stable_item_id.as_str())
            .expect("validated exact key coverage");
        let final_rect = translate_for_scroll(rect, viewer.scroll_offset_pixels)?;
        checked_rect(final_rect, "final row rect")?;
        let mut admitted_template = binding.template.clone();
        prefix_element_names(
            &mut admitted_template,
            &format!("{}::", binding.instance_token),
        );
        rows.push(PlannedCollectionRow {
            stable_item_id: row.stable_item_id.clone(),
            subtype_id: row.subtype_id.clone(),
            source_ordinal: row.source_ordinal,
            owning_window_id: source.owning_window_id.to_string(),
            root_source_identity: source.root_source_identity.to_string(),
            element_source_identity: binding.element_source_identity.to_string(),
            instance_token: binding.instance_token.to_string(),
            viewport_position_pixels: (final_rect.x(), final_rect.y()),
            final_rect,
            visible_clip,
            visible_rect: intersection(final_rect, visible_clip),
            paint_order: paint_order(viewer, local_paint_ordinal),
            template: admitted_template,
        });
    }

    Ok(ObserverCollectionPlan {
        family,
        collection_instance_token: semantic.collection_instance_token.clone(),
        collection_element_id: source.collection_element_id.to_string(),
        collection_rect,
        visible_clip,
        content_extent_pixels,
        viewport_size_pixels: (options.viewport_width, options.viewport_height),
        root_size_pixels: (options.root_width, options.root_height),
        rows,
    })
}

fn validate_source(source: AuthoredCollectionSource<'_>) -> Result<()> {
    nonempty(source.owning_window_id, "owning_window_id")?;
    nonempty(source.root_source_identity, "root_source_identity")?;
    nonempty(source.collection_element_id, "collection_element_id")?;
    positive_rect(source.parent_rect, "parent_rect")
}

fn validate_grid_authority(source: AuthoredCollectionSource<'_>) -> Result<()> {
    let normalized_root = normalize_identity(source.root_source_identity);
    match source.grid_authority {
        Some(StandardGridAuthority::Xui26FrontendExactCarrierOrder) => {
            let normalized_owner = normalize_identity(source.owning_window_id);
            let normalized_element = normalize_identity(source.collection_element_id);
            if !normalized_owner.contains("interface/")
                || !normalized_owner.contains(".gui#")
                || normalized_element.trim().is_empty()
                || !source
                    .collection_element
                    .element_type
                    .eq_ignore_ascii_case("gridBoxType")
            {
                return Err(fail(
                    "XUI26 standard-grid witness does not match an authenticated effective GUI collection",
                ));
            }
            Ok(())
        }
        Some(StandardGridAuthority::Xui13AcquiredVariantsStandardAutoAppend) => {
            validate_xui13_grid_root(source, &normalized_root)?;
            let normalized_element_id = normalize_identity(source.collection_element_id);
            let source_matches = normalized_element_id
                == format!(
                    "{normalized_root}/designer_intel_container/gridbox_container/acquired_variants_grid"
                );
            let element_matches = source
                .collection_element
                .name
                .eq_ignore_ascii_case("acquired_variants_grid")
                && source
                    .collection_element
                    .element_type
                    .eq_ignore_ascii_case("gridBoxType");
            if !source_matches || !element_matches {
                return Err(fail(
                    "XUI13 witness does not match an exact acquired_variants_grid hierarchy",
                ));
            }
            Ok(())
        }
        Some(StandardGridAuthority::Xui13VariantCountryFilterStandardAutoAppend) => {
            validate_xui13_grid_root(source, &normalized_root)?;
            let normalized_element_id = normalize_identity(source.collection_element_id);
            let source_matches = normalized_element_id
                == format!(
                    "{normalized_root}/designer_intel_container/country_filter_container/variant_country_filter_gridbox"
                );
            if !source_matches
                || !source
                    .collection_element
                    .name
                    .eq_ignore_ascii_case("variant_country_filter_gridbox")
                || !source
                    .collection_element
                    .element_type
                    .eq_ignore_ascii_case("gridBoxType")
            {
                return Err(fail(
                    "XUI13 witness does not match an exact variant_country_filter_gridbox hierarchy",
                ));
            }
            Ok(())
        }
        Some(StandardGridAuthority::Xui15CountryTradeStandardAutoAppend) => {
            let normalized_owner = normalize_identity(source.owning_window_id);
            let normalized_element_id = normalize_identity(source.collection_element_id);
            let source_matches = normalized_root == "interface/countrytradeview.gui#countrytradeview"
                && normalized_owner == normalized_root
                && matches!(
                    normalized_element_id.as_str(),
                    "interface/countrytradeview.gui#countrytradeview/resources/resources_grid"
                        | "interface/countrytradeview.gui#countrytradeview/filters/resources_grid"
                        | "interface/countrytradeview.gui#countrytradeview/country_filters/country_filters_grid"
                        | "interface/countrytradeview.gui#countrytradeview/trade/trade_grid"
            );
            let expected_name = normalized_element_id.rsplit('/').next().unwrap_or_default();
            if !source_matches
                || !source
                    .collection_element
                    .name
                    .eq_ignore_ascii_case(expected_name)
                || !source
                    .collection_element
                    .element_type
                    .eq_ignore_ascii_case("gridBoxType")
            {
                return Err(fail(
                    "XUI15 witness does not match an exact countrytradeview grid hierarchy",
                ));
            }
            Ok(())
        }
        Some(StandardGridAuthority::Xui19DecisionStandardAutoAppend) => {
            let root = normalize_identity(source.root_source_identity);
            let owner = normalize_identity(source.owning_window_id);
            let element = normalize_identity(source.collection_element_id);
            let source_matches = root
                .split('#')
                .next()
                .is_some_and(|path| path.ends_with("interface/countrydecisionview.gui"))
                && owner.ends_with("#countrydecisionview")
                && element.starts_with("interface/countrydecisionview.gui#countrydecisionview/")
                && element.ends_with("/decision_grid");
            if !source_matches
                || !source
                    .collection_element
                    .name
                    .eq_ignore_ascii_case("decision_grid")
                || !source
                    .collection_element
                    .element_type
                    .eq_ignore_ascii_case("gridBoxType")
            {
                return Err(fail(
                    "XUI19 witness does not match the exact countrydecisionview/decision_grid hierarchy",
                ));
            }
            Ok(())
        }
        None => Err(fail(
            "grid has no reviewed panel-specific standard-auto-append witness",
        )),
    }
}

fn validate_xui13_grid_root(
    source: AuthoredCollectionSource<'_>,
    normalized_root: &str,
) -> Result<()> {
    if normalize_identity(source.owning_window_id) != normalized_root {
        return Err(fail(
            "XUI13 owning window identity does not match the effective family root",
        ));
    }
    let (_, root_name) = split_effective_gui_identity(normalized_root)?;
    if ![
        "countryequipmentdesignerview",
        "tank_designer_view",
        "plane_designer_view",
    ]
    .contains(&root_name)
    {
        return Err(fail(
            "XUI13 witness does not name an authenticated designer family root",
        ));
    }
    Ok(())
}

fn validate_grid_row_templates(
    source: AuthoredCollectionSource<'_>,
    bindings: &[AuthoredRowBinding<'_>],
) -> Result<()> {
    if source.grid_authority == Some(StandardGridAuthority::Xui26FrontendExactCarrierOrder) {
        if bindings.is_empty() {
            return match (
                source.effective_row_template_identity,
                source.effective_row_template,
            ) {
                (Some(identity), Some(template))
                    if !identity.trim().is_empty()
                        && !template.name.trim().is_empty()
                        && template
                            .element_type
                            .eq_ignore_ascii_case("containerWindowType") =>
                {
                    Ok(())
                }
                _ => Err(fail(
                    "XUI26 measured-empty grid has no authenticated effective row template",
                )),
            };
        }
        for binding in bindings {
            if binding.element_source_identity.trim().is_empty()
                || binding.template.name.trim().is_empty()
                || !binding
                    .template
                    .element_type
                    .eq_ignore_ascii_case("containerWindowType")
            {
                return Err(fail(
                    "XUI26 row template is not an authenticated container root",
                ));
            }
        }
        return Ok(());
    }
    if source.grid_authority == Some(StandardGridAuthority::Xui19DecisionStandardAutoAppend) {
        let allowed = [
            "decision_item",
            "targeted_decision_item",
            "timed_decision_item",
            "on_map_decision_locator_item",
        ];
        match (
            source.effective_row_template_identity,
            source.effective_row_template,
        ) {
            (Some(identity), Some(template)) => {
                let name = template.name.to_ascii_lowercase();
                if !allowed.contains(&name.as_str())
                    || !normalize_identity(identity)
                        .ends_with(&format!("interface/countrydecisionview.gui#{name}"))
                    || !template
                        .element_type
                        .eq_ignore_ascii_case("containerWindowType")
                {
                    return Err(fail(
                        "XUI19 effective row-template authority is not one of the four engine-resolved authored templates",
                    ));
                }
            }
            (None, None) if !bindings.is_empty() => {}
            (None, None) => {
                return Err(fail(
                    "XUI19 empty grid has no effective row-template authority",
                ));
            }
            _ => {
                return Err(fail(
                    "effective row-template identity/tree authority is incomplete",
                ));
            }
        }
        for binding in bindings {
            let identity = normalize_identity(binding.element_source_identity);
            let name = binding.template.name.to_ascii_lowercase();
            if !allowed.contains(&name.as_str())
                || !identity.ends_with(&format!("interface/countrydecisionview.gui#{name}"))
                || !binding
                    .template
                    .element_type
                    .eq_ignore_ascii_case("containerWindowType")
            {
                return Err(fail(
                    "XUI19 decision row template is not one of the four engine-resolved authored templates",
                ));
            }
        }
        return Ok(());
    }
    let (expected_name, rejection, empty_rejection) = match source.grid_authority {
        Some(StandardGridAuthority::Xui13AcquiredVariantsStandardAutoAppend) => (
            "designer_equipment_entry",
            "XUI13 acquired-variant row template does not match the effective designer_equipment_entry",
            "XUI13 empty grid has no effective row-template authority",
        ),
        Some(StandardGridAuthority::Xui13VariantCountryFilterStandardAutoAppend) => (
            "designer_creator_entry",
            "XUI13 country-filter row template does not match the effective designer_creator_entry",
            "XUI13 empty grid has no effective row-template authority",
        ),
        Some(StandardGridAuthority::Xui15CountryTradeStandardAutoAppend) => {
            let element_id = normalize_identity(source.collection_element_id);
            let expected_name = if element_id.ends_with("/resources/resources_grid") {
                "resources_info_entry"
            } else if element_id.ends_with("/filters/resources_grid") {
                "resources_filter_entry"
            } else if element_id.ends_with("/country_filters/country_filters_grid") {
                "filter_entry"
            } else if element_id.ends_with("/trade/trade_grid") {
                "country_trade_entry"
            } else {
                return Err(fail("XUI15 grid row-template hierarchy is unknown"));
            };
            (
                expected_name,
                "XUI15 row template does not match the admitted grid hierarchy",
                "XUI15 empty grid has no effective row-template authority",
            )
        }
        Some(StandardGridAuthority::Xui19DecisionStandardAutoAppend) => unreachable!(),
        Some(StandardGridAuthority::Xui26FrontendExactCarrierOrder) => unreachable!(),
        None => return Err(fail("grid row templates have no authority witness")),
    };
    let (expected_identity, expected_template) = match (
        source.effective_row_template_identity,
        source.effective_row_template,
    ) {
        (Some(identity), Some(template)) => (identity, template),
        (None, None) => {
            let first = bindings.first().ok_or_else(|| fail(empty_rejection))?;
            (first.element_source_identity, first.template)
        }
        _ => {
            return Err(fail(
                "effective row-template identity/tree authority is incomplete",
            ))
        }
    };
    validate_effective_template(expected_identity, expected_template, expected_name)
        .map_err(|_| fail(rejection))?;
    let normalized_expected = normalize_identity(expected_identity);
    for binding in bindings {
        if normalize_identity(binding.element_source_identity) != normalized_expected
            || !binding.template.name.eq_ignore_ascii_case(expected_name)
            || !binding
                .template
                .element_type
                .eq_ignore_ascii_case("containerWindowType")
        {
            return Err(fail(rejection));
        }
    }
    Ok(())
}

fn normalize_identity(identity: &str) -> String {
    identity.replace('\\', "/").to_ascii_lowercase()
}

fn split_effective_gui_identity(identity: &str) -> Result<(&str, &str)> {
    let (source_file, root_name) = identity
        .rsplit_once('#')
        .ok_or_else(|| fail("effective GUI identity has no root-name separator"))?;
    if source_file.trim().is_empty()
        || root_name.trim().is_empty()
        || !source_file.ends_with(".gui")
    {
        return Err(fail("effective GUI identity is incomplete"));
    }
    Ok((source_file, root_name))
}

fn validate_effective_template(
    identity: &str,
    template: &GuiElement,
    expected_name: &str,
) -> Result<()> {
    let normalized = normalize_identity(identity);
    let (_, root_name) = split_effective_gui_identity(&normalized)?;
    if root_name != expected_name
        || !template.name.eq_ignore_ascii_case(expected_name)
        || !template
            .element_type
            .eq_ignore_ascii_case("containerWindowType")
    {
        return Err(fail("effective row-template identity/tree mismatch"));
    }
    Ok(())
}

fn checked_collection_rect(element: &GuiElement, parent: Rect) -> Result<Rect> {
    checked_rect(parent, "collection parent rect")?;
    let size = element
        .size
        .as_ref()
        .ok_or_else(|| fail("authored collection has no explicit size"))?;
    let width = measure_to_pixels_for_parent(&size.x, parent.w());
    let height = measure_to_pixels_for_parent(&size.y, parent.h());
    let (position_x, position_y) = element
        .position
        .as_ref()
        .map(|position| {
            (
                measure_to_pixels_for_parent(&position.x, parent.w()),
                measure_to_pixels_for_parent(&position.y, parent.h()),
            )
        })
        .unwrap_or((0, 0));
    let (anchor_x, anchor_y) = Orientation::parse(element.orientation.as_deref()).anchor(parent);
    let x = parent
        .x()
        .checked_add(anchor_x)
        .and_then(|value| value.checked_add(position_x))
        .ok_or_else(|| fail("collection x coordinate overflows"))?;
    let y = parent
        .y()
        .checked_add(anchor_y)
        .and_then(|value| value.checked_add(position_y))
        .ok_or_else(|| fail("collection y coordinate overflows"))?;
    let rect = Rect::new(x, y, width, height);
    positive_rect(rect, "collection_rect")?;
    checked_rect(rect, "collection_rect")?;
    Ok(rect)
}

fn validate_bindings<'a>(
    semantic: &ObserverSemanticCollection,
    bindings: &'a [AuthoredRowBinding<'a>],
) -> Result<BTreeMap<&'a str, &'a AuthoredRowBinding<'a>>> {
    if bindings.len() != semantic.rows.len() {
        return Err(fail(
            "authored row bindings do not exactly cover semantic membership",
        ));
    }
    let mut by_id = BTreeMap::new();
    let mut instance_tokens = BTreeSet::new();
    for binding in bindings {
        nonempty(binding.stable_item_id, "binding stable_item_id")?;
        nonempty(binding.subtype_id, "binding subtype_id")?;
        nonempty(binding.element_source_identity, "element_source_identity")?;
        nonempty(binding.instance_token, "instance_token")?;
        if by_id.insert(binding.stable_item_id, binding).is_some() {
            return Err(fail("duplicate authored binding stable_item_id"));
        }
        if !instance_tokens.insert(binding.instance_token) {
            return Err(fail("duplicate pointer-free row instance_token"));
        }
    }
    for row in &semantic.rows {
        let binding = by_id
            .get(row.stable_item_id.as_str())
            .ok_or_else(|| fail("semantic row has no authored template binding"))?;
        if binding.subtype_id != row.subtype_id {
            return Err(fail("panel subtype differs from authored template binding"));
        }
    }
    Ok(by_id)
}

fn family(element: &GuiElement) -> Result<ObserverCollectionFamily> {
    match element.element_type.to_ascii_lowercase().as_str() {
        "listboxtype" => Ok(ObserverCollectionFamily::List),
        "smoothlistboxtype" => Ok(ObserverCollectionFamily::SmoothList),
        "gridboxtype" => Ok(ObserverCollectionFamily::Grid),
        "overlappingelementsboxtype" => Ok(ObserverCollectionFamily::Overlap),
        _ => Err(fail("authored element is not a collection family")),
    }
}

fn template_size(template: &GuiElement, parent: Rect) -> Result<(i32, i32)> {
    let size = template
        .size
        .as_ref()
        .ok_or_else(|| fail("authored row template has no explicit size"))?;
    let resolved = (
        measure_to_pixels_for_parent(&size.x, parent.w()),
        measure_to_pixels_for_parent(&size.y, parent.h()),
    );
    if resolved.0 <= 0 || resolved.1 <= 0 {
        return Err(fail(
            "authored row template resolves to a non-positive size",
        ));
    }
    Ok(resolved)
}

fn plan_list(
    element: &GuiElement,
    parent: Rect,
    collection_rect: Rect,
    sizes: &[(i32, i32)],
    options: &ComposeOptions,
) -> Result<(Vec<Rect>, (i32, i32))> {
    validate_list_layout_properties(element)?;
    let layout = listbox_layout(element, parent, options)?;
    let border_inset = list_border_inset(element, collection_rect)?;
    let mut cursor = match layout.flow {
        ListFlow::Vertical => collection_rect
            .y()
            .checked_add(border_inset)
            .ok_or_else(|| fail("list border inset overflows y"))?,
        ListFlow::Horizontal => collection_rect
            .x()
            .checked_add(border_inset)
            .ok_or_else(|| fail("list border inset overflows x"))?,
    };
    let fixed_x = collection_rect
        .x()
        .checked_add(border_inset)
        .ok_or_else(|| fail("list border inset overflows x"))?;
    let fixed_y = collection_rect
        .y()
        .checked_add(border_inset)
        .ok_or_else(|| fail("list border inset overflows y"))?;
    let mut rects = Vec::with_capacity(sizes.len());
    for &(width, height) in sizes {
        cursor = cursor
            .checked_add(layout.spacing)
            .ok_or_else(|| fail("list spacing overflows"))?;
        let rect = match layout.flow {
            ListFlow::Vertical => Rect::new(fixed_x, cursor, width, height),
            ListFlow::Horizontal => Rect::new(cursor, fixed_y, width, height),
        };
        rects.push(rect);
        cursor = cursor
            .checked_add(match layout.flow {
                ListFlow::Vertical => height,
                ListFlow::Horizontal => width,
            })
            .ok_or_else(|| fail("list child extent overflows"))?;
    }
    let extent = bounding_extent(collection_rect, &rects)?;
    Ok((rects, extent))
}

fn list_border_inset(element: &GuiElement, collection_rect: Rect) -> Result<i32> {
    let Some(raw) = element.raw.first_value_ci("borderSize") else {
        return Ok(0);
    };
    let pair = parse_measure_pair(raw)
        .ok_or_else(|| fail("list borderSize is not an authenticated measure pair"))?;
    let inset = measure_to_pixels_for_parent(&pair.x, collection_rect.w());
    if inset < 0 {
        return Err(fail("list borderSize.x is negative"));
    }
    Ok(inset)
}

fn plan_smooth_list(
    element: &GuiElement,
    parent: Rect,
    collection_rect: Rect,
    sizes: &[(i32, i32)],
    options: &ComposeOptions,
) -> Result<(Vec<Rect>, (i32, i32))> {
    validate_list_layout_properties(element)?;
    let layout = listbox_layout(element, parent, options)?;
    let mut cursor = 0i32;
    let mut rects = Vec::with_capacity(sizes.len());
    for &(width, height) in sizes {
        let rect = match layout.flow {
            ListFlow::Vertical => Rect::new(
                collection_rect.x(),
                collection_rect
                    .y()
                    .checked_add(cursor)
                    .ok_or_else(|| fail("smooth-list y coordinate overflows"))?,
                width,
                height,
            ),
            ListFlow::Horizontal => Rect::new(
                collection_rect
                    .x()
                    .checked_add(cursor)
                    .ok_or_else(|| fail("smooth-list x coordinate overflows"))?,
                collection_rect.y(),
                width,
                height,
            ),
        };
        rects.push(rect);
        cursor = cursor
            .checked_add(match layout.flow {
                ListFlow::Vertical => height,
                ListFlow::Horizontal => width,
            })
            .ok_or_else(|| fail("smooth-list child extent overflows"))?;
    }
    let extent = bounding_extent(collection_rect, &rects)?;
    Ok((rects, extent))
}

fn validate_list_layout_properties(element: &GuiElement) -> Result<()> {
    if let Some(raw) = element.raw.first_value_ci("horizontal") {
        let raw = raw
            .as_scalar()
            .ok_or_else(|| fail("list horizontal is not an authenticated boolean"))?;
        parse_optional_bool(Some(raw), "list horizontal")?;
    }
    if let Some(raw) = element.raw.first_value_ci("spacing") {
        scalar_i32(raw).ok_or_else(|| fail("list spacing is not an authenticated integer"))?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverlapFormat {
    Center,
    Right,
    Left,
    Up,
    Down,
}

impl OverlapFormat {
    fn parse(raw: Option<&str>) -> Result<Self> {
        match raw.unwrap_or("left").trim().to_ascii_lowercase().as_str() {
            "center" => Ok(Self::Center),
            "right" => Ok(Self::Right),
            "left" => Ok(Self::Left),
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            _ => Err(fail("unknown overlap format")),
        }
    }

    fn reverse_coordinates(self) -> bool {
        matches!(self, Self::Right | Self::Up)
    }

    fn advances_horizontally(self) -> bool {
        matches!(self, Self::Center | Self::Right | Self::Left)
    }
}

fn plan_overlap(
    element: &GuiElement,
    collection_rect: Rect,
    sizes: &[(i32, i32)],
) -> Result<(Vec<Rect>, (i32, i32), Vec<u32>)> {
    let (format, spacing, first_on_top) = overlap_properties(element)?;
    if sizes.is_empty() {
        return Ok((Vec::new(), (0, 0), Vec::new()));
    }
    let count = sizes.len();
    let first = sizes[0];
    if count == 1
        && ((format.advances_horizontally() && collection_rect.w() < first.0)
            || (!format.advances_horizontally() && collection_rect.h() < first.1))
    {
        return Err(fail(
            "singleton overlap would enter engine zero-interval compression",
        ));
    }
    let mut horizontal_stride = spacing + first.0 as f32;
    let mut vertical_stride = spacing + first.1 as f32;
    if count > 1 {
        let intervals = (count - 1) as f32;
        if (collection_rect.w() as f32) < count as f32 * horizontal_stride - spacing {
            horizontal_stride = (collection_rect.w() as f32 - first.0 as f32) / intervals;
        }
        if (collection_rect.h() as f32) < count as f32 * vertical_stride - spacing {
            vertical_stride = (collection_rect.h() as f32 - first.1 as f32) / intervals;
        }
    }
    if !horizontal_stride.is_finite() || !vertical_stride.is_finite() {
        return Err(fail("overlap stride is not finite"));
    }

    let (base_x, base_y, step_x, step_y) = match format {
        OverlapFormat::Center => {
            let span = (count.saturating_sub(1) as f32) * horizontal_stride + first.0 as f32;
            let centered = checked_f32_to_i32(
                collection_rect.w() as f32 * 0.5 - (span + 0.5) * 0.5,
                "overlap center offset",
            )?;
            (
                collection_rect
                    .x()
                    .checked_add(centered)
                    .ok_or_else(|| fail("overlap centered x overflows"))?,
                collection_rect.y(),
                horizontal_stride,
                0.0,
            )
        }
        OverlapFormat::Left => (
            collection_rect.x(),
            collection_rect.y(),
            horizontal_stride,
            0.0,
        ),
        OverlapFormat::Down => (
            collection_rect.x(),
            collection_rect.y(),
            0.0,
            vertical_stride,
        ),
        OverlapFormat::Right => (
            collection_rect
                .x()
                .checked_add(collection_rect.w())
                .and_then(|value| value.checked_sub(first.0))
                .ok_or_else(|| fail("overlap right anchor overflows"))?,
            collection_rect.y(),
            horizontal_stride,
            0.0,
        ),
        OverlapFormat::Up => (
            collection_rect.x(),
            collection_rect
                .y()
                .checked_add(collection_rect.h())
                .and_then(|value| value.checked_sub(first.1))
                .ok_or_else(|| fail("overlap up anchor overflows"))?,
            0.0,
            vertical_stride,
        ),
    };

    let coordinate_source_order = if format.reverse_coordinates() {
        (0..count).rev().collect::<Vec<_>>()
    } else {
        (0..count).collect::<Vec<_>>()
    };
    let mut rects = vec![Rect::new(0, 0, 1, 1); count];
    for (coordinate_ordinal, source_ordinal) in coordinate_source_order.into_iter().enumerate() {
        let offset_x = overlap_rounded_step(coordinate_ordinal, step_x)?;
        let offset_y = overlap_rounded_step(coordinate_ordinal, step_y)?;
        let (width, height) = sizes[source_ordinal];
        rects[source_ordinal] = Rect::new(
            base_x
                .checked_add(offset_x)
                .ok_or_else(|| fail("overlap child x overflows"))?,
            base_y
                .checked_add(offset_y)
                .ok_or_else(|| fail("overlap child y overflows"))?,
            width,
            height,
        );
    }

    let paint_ordinals = if first_on_top {
        (0..count)
            .rev()
            .map(|ordinal| {
                u32::try_from(ordinal).map_err(|_| fail("overlap paint ordinal overflows u32"))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        source_paint_ordinals(count)?
    };
    let extent = bounding_extent(collection_rect, &rects)?;
    Ok((rects, extent, paint_ordinals))
}

pub(crate) fn plan_authenticated_overlap_rects(
    element: &GuiElement,
    collection_rect: Rect,
    sizes: &[(i32, i32)],
) -> Result<Vec<Rect>> {
    let (rects, _, _) = plan_overlap(element, collection_rect, sizes)?;
    Ok(rects)
}

pub(crate) fn plan_authenticated_smooth_list_rects(
    element: &GuiElement,
    authenticated_collection_rect: Rect,
    sizes: &[(i32, i32)],
    scroll_offset_pixels: i32,
    options: &ComposeOptions,
) -> Result<Vec<Rect>> {
    if family(element)? != ObserverCollectionFamily::SmoothList {
        return Err(fail(
            "authenticated smooth-list owner supplied another collection family",
        ));
    }
    positive_rect(
        authenticated_collection_rect,
        "authenticated smooth-list collection rect",
    )?;
    if scroll_offset_pixels < 0 {
        return Err(fail("authenticated smooth-list scroll is negative"));
    }
    if sizes
        .iter()
        .any(|(width, height)| *width <= 0 || *height <= 0)
    {
        return Err(fail("authenticated smooth-list row extent is not positive"));
    }
    let (rects, content_extent) = plan_smooth_list(
        element,
        authenticated_collection_rect,
        authenticated_collection_rect,
        sizes,
        options,
    )?;
    let layout = listbox_layout(element, authenticated_collection_rect, options)?;
    let (content, viewport) = match layout.flow {
        ListFlow::Vertical => (content_extent.1, authenticated_collection_rect.h()),
        ListFlow::Horizontal => (content_extent.0, authenticated_collection_rect.w()),
    };
    if scroll_offset_pixels > content.saturating_sub(viewport).max(0) {
        return Err(fail(
            "authenticated smooth-list scroll exceeds the content extent",
        ));
    }
    rects
        .into_iter()
        .map(|rect| match layout.flow {
            ListFlow::Vertical => translate_for_scroll(rect, (0, scroll_offset_pixels)),
            ListFlow::Horizontal => translate_for_scroll(rect, (scroll_offset_pixels, 0)),
        })
        .collect()
}

fn overlap_properties(element: &GuiElement) -> Result<(OverlapFormat, f32, bool)> {
    let format = match element.raw.first_value_ci("format") {
        Some(raw) => OverlapFormat::parse(Some(
            raw.as_scalar()
                .ok_or_else(|| fail("overlap format is not an authenticated scalar"))?,
        ))?,
        None => OverlapFormat::Left,
    };
    let spacing = match element.raw.first_value_ci("spacing") {
        Some(raw) => {
            scalar_f32(raw).ok_or_else(|| fail("overlap spacing is not an authenticated scalar"))?
        }
        None => 1.0,
    };
    if !spacing.is_finite() {
        return Err(fail("overlap spacing is not finite"));
    }
    let first_on_top = match element.raw.first_value_ci("first_on_top") {
        Some(raw) => parse_optional_bool(
            Some(
                raw.as_scalar()
                    .ok_or_else(|| fail("overlap first_on_top is not an authenticated boolean"))?,
            ),
            "overlap first_on_top",
        )?
        .ok_or_else(|| fail("overlap first_on_top was not admitted"))?,
        None => false,
    };
    Ok((format, spacing, first_on_top))
}

fn overlap_rounded_step(index: usize, stride: f32) -> Result<i32> {
    let index = index as f32;
    checked_f64_to_i32(
        (index * stride) as f64 + 0.5f64,
        "overlap rounded child offset",
    )
}

fn checked_f32_to_i32(value: f32, field: &str) -> Result<i32> {
    checked_f64_to_i32(value as f64, field)
}

fn checked_f64_to_i32(value: f64, field: &str) -> Result<i32> {
    if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
        return Err(fail(format!("{field} overflows i32")));
    }
    Ok(value as i32)
}

fn parse_optional_bool(raw: Option<&str>, field: &str) -> Result<Option<bool>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" => Ok(Some(true)),
        "no" | "false" => Ok(Some(false)),
        value => value
            .parse::<i32>()
            .map(|value| Some(value != 0))
            .map_err(|_| fail(format!("{field} is not an authenticated boolean"))),
    }
}

fn source_paint_ordinals(count: usize) -> Result<Vec<u32>> {
    (0..count)
        .map(|ordinal| {
            u32::try_from(ordinal).map_err(|_| fail("local paint ordinal overflows u32"))
        })
        .collect()
}

fn plan_grid(
    element: &GuiElement,
    parent: Rect,
    sizes: &[(i32, i32)],
    options: &ComposeOptions,
) -> Result<(Vec<Rect>, (i32, i32))> {
    let layout = gridbox_layout(element, parent, options)?;
    let mut rects = Vec::with_capacity(sizes.len());
    for (index, &(width, height)) in sizes.iter().enumerate() {
        let coordinate = grid_coordinate(&layout, index)?;
        let origin = layout.explicit_coordinate_origin(coordinate)?;
        rects.push(Rect::new(origin.0, origin.1, width, height));
    }
    let extent = bounding_extent(layout.rect, &rects)?;
    Ok((rects, extent))
}

fn grid_coordinate(layout: &GridBoxLayout, index: usize) -> Result<(i32, i32)> {
    let horizontal = layout.add_horizontal.unwrap_or(true);
    let (x, y) = if horizontal {
        match layout.max_slots_x.filter(|columns| *columns > 0) {
            Some(columns) => (index % columns, index / columns),
            None => (index, 0),
        }
    } else {
        match layout.max_slots_y.filter(|rows| *rows > 0) {
            Some(rows) => (index / rows, index % rows),
            None => (0, index),
        }
    };
    if layout
        .max_slots_x
        .is_some_and(|columns| columns > 0 && x >= columns)
        || layout.max_slots_y.is_some_and(|rows| rows > 0 && y >= rows)
    {
        return Err(fail("grid population exceeds authored maximum slots"));
    }
    Ok((
        i32::try_from(x).map_err(|_| fail("grid x coordinate overflows i32"))?,
        i32::try_from(y).map_err(|_| fail("grid y coordinate overflows i32"))?,
    ))
}

fn bounding_extent(origin: Rect, rects: &[Rect]) -> Result<(i32, i32)> {
    if rects.is_empty() {
        return Ok((0, 0));
    }
    let mut right = 0i32;
    let mut bottom = 0i32;
    for rect in rects {
        right = right.max(
            rect.x()
                .checked_sub(origin.x())
                .and_then(|value| value.checked_add(rect.w()))
                .ok_or_else(|| fail("collection content width overflows"))?,
        );
        bottom = bottom.max(
            rect.y()
                .checked_sub(origin.y())
                .and_then(|value| value.checked_add(rect.h()))
                .ok_or_else(|| fail("collection content height overflows"))?,
        );
    }
    Ok((right.max(0), bottom.max(0)))
}

fn translate_for_scroll(rect: Rect, scroll: (i32, i32)) -> Result<Rect> {
    Ok(Rect::new(
        rect.x()
            .checked_sub(scroll.0)
            .ok_or_else(|| fail("horizontal viewer scroll overflows"))?,
        rect.y()
            .checked_sub(scroll.1)
            .ok_or_else(|| fail("vertical viewer scroll overflows"))?,
        rect.w(),
        rect.h(),
    ))
}

fn validate_coordinate_space(options: &ComposeOptions) -> Result<()> {
    if options.viewport_width == 0
        || options.viewport_height == 0
        || options.root_width == 0
        || options.root_height == 0
    {
        return Err(fail("composition coordinate space is empty"));
    }
    for value in [
        options.viewport_width,
        options.viewport_height,
        options.root_width,
        options.root_height,
    ] {
        i32::try_from(value)
            .map_err(|_| fail("composition coordinate space exceeds engine i32 pixels"))?;
    }
    Ok(())
}

fn validate_scroll(
    family: ObserverCollectionFamily,
    source: AuthoredCollectionSource<'_>,
    parent: Rect,
    options: &ComposeOptions,
    collection_rect: Rect,
    content_extent: (i32, i32),
    scroll: (i32, i32),
    visible_clip: Rect,
) -> Result<()> {
    if scroll.0 < 0 || scroll.1 < 0 {
        return Err(fail("viewer scroll is negative"));
    }
    match family {
        ObserverCollectionFamily::Grid => {
            if scroll == (0, 0) {
                return Ok(());
            }
            if source.grid_authority
                != Some(StandardGridAuthority::Xui15CountryTradeStandardAutoAppend)
            {
                return Err(fail(
                    "grid has no authenticated intrinsic scrollbar; scroll its admitted parent",
                ));
            }
            let element_id = source
                .collection_element_id
                .replace('\\', "/")
                .to_ascii_lowercase();
            let (offset, cross_offset, extent, viewport) =
                if element_id.ends_with("/filters/resources_grid") {
                    (scroll.0, scroll.1, content_extent.0, visible_clip.w())
                } else if element_id.ends_with("/country_filters/country_filters_grid")
                    || element_id.ends_with("/trade/trade_grid")
                {
                    (scroll.1, scroll.0, content_extent.1, visible_clip.h())
                } else {
                    return Err(fail("XUI15 resources grid is not scrollable"));
                };
            if cross_offset != 0 {
                return Err(fail(
                    "XUI15 grid viewer scroll moves on the non-authored axis",
                ));
            }
            let maximum = extent.saturating_sub(viewport).max(0);
            if offset > maximum {
                return Err(fail("XUI15 grid viewer scroll exceeds the content extent"));
            }
        }
        ObserverCollectionFamily::Overlap => {
            if scroll != (0, 0) {
                return Err(fail("overlap collection has no intrinsic scrollbar"));
            }
        }
        ObserverCollectionFamily::List | ObserverCollectionFamily::SmoothList => {
            let layout = listbox_layout(source.collection_element, parent, options)?;
            let (offset, cross_offset, extent, viewport) = match layout.flow {
                ListFlow::Vertical => (scroll.1, scroll.0, content_extent.1, collection_rect.h()),
                ListFlow::Horizontal => (scroll.0, scroll.1, content_extent.0, collection_rect.w()),
            };
            if cross_offset != 0 {
                return Err(fail("list viewer scroll moves on the non-flow axis"));
            }
            let maximum = extent.saturating_sub(viewport).max(0);
            if offset > maximum {
                return Err(fail("list viewer scroll exceeds the content extent"));
            }
        }
    }
    Ok(())
}

fn paint_order(viewer: ViewerCollectionState, local: u32) -> CollectionPaintOrder {
    CollectionPaintOrder {
        window_stack_ordinal: viewer.window_stack_ordinal,
        root_paint_ordinal: viewer.root_paint_ordinal,
        local_paint_ordinal: local,
    }
}

fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    let left = a.x().max(b.x());
    let top = a.y().max(b.y());
    let right = a.x().checked_add(a.w())?.min(b.x().checked_add(b.w())?);
    let bottom = a.y().checked_add(a.h())?.min(b.y().checked_add(b.h())?);
    (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
}

fn merge_cropped_layer(target: &mut ComposeOutput, layer: ComposeOutput, clip: Rect) -> Result<()> {
    let clip_width = usize::try_from(clip.w()).map_err(|_| fail("clip width overflows"))?;
    let clip_height = usize::try_from(clip.h()).map_err(|_| fail("clip height overflows"))?;
    if layer.image.width != clip_width || layer.image.height != clip_height {
        return Err(fail("cropped row layer dimensions differ from its clip"));
    }
    let target_x = usize::try_from(clip.x()).map_err(|_| fail("clip x is outside viewport"))?;
    let target_y = usize::try_from(clip.y()).map_err(|_| fail("clip y is outside viewport"))?;
    for local_y in 0..clip_height {
        for local_x in 0..clip_width {
            let source_offset = (local_y * clip_width + local_x) * 4;
            let target_offset =
                ((target_y + local_y) * target.image.width + target_x + local_x) * 4;
            let src = &layer.image.pixels[source_offset..source_offset + 4];
            let dst = &mut target.image.pixels[target_offset..target_offset + 4];
            let blended = alpha_blend(
                [dst[0], dst[1], dst[2], dst[3]],
                [src[0], src[1], src[2], src[3]],
            );
            dst.copy_from_slice(&blended);
        }
    }
    merge_report(&mut target.report, layer.report);
    target.debug_rects.extend(layer.debug_rects);
    Ok(())
}

fn merge_report(target: &mut ComposeReport, source: ComposeReport) {
    target.drawn_elements = target.drawn_elements.saturating_add(source.drawn_elements);
    target.missing_sprites.extend(source.missing_sprites);
    target
        .undecodable_sprites
        .extend(source.undecodable_sprites);
    target.missing_fonts.extend(source.missing_fonts);
    for (font, glyphs) in source.missing_glyphs {
        target
            .missing_glyphs
            .entry(font)
            .or_default()
            .extend(glyphs);
    }
    target
        .unresolved_loc_keys
        .extend(source.unresolved_loc_keys);
    for (element, issues) in source.unresolved_text_inputs {
        target
            .unresolved_text_inputs
            .entry(element)
            .or_default()
            .extend(issues);
    }
}

fn positive_rect(rect: Rect, field: &str) -> Result<()> {
    if rect.w() <= 0 || rect.h() <= 0 {
        return Err(fail(format!("{field} must have positive size")));
    }
    Ok(())
}

fn checked_rect(rect: Rect, field: &str) -> Result<()> {
    rect.x()
        .checked_add(rect.w())
        .ok_or_else(|| fail(format!("{field} right edge overflows")))?;
    rect.y()
        .checked_add(rect.h())
        .ok_or_else(|| fail(format!("{field} bottom edge overflows")))?;
    Ok(())
}

fn nonempty(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(fail(format!("{field} must be nonempty")));
    }
    Ok(())
}

fn fail(message: impl Into<String>) -> GameUiError {
    GameUiError::new(
        ErrorKind::Other,
        format!("XUI03_FAIL_CLOSED: {}", message.into()),
    )
}

#[cfg(test)]
mod production_adapter_tests {
    use super::*;
    use crate::gui::parse_gui_document;
    use crate::paradox_text::parse_str;

    fn root(source: &str) -> GuiElement {
        let document = parse_str(&format!("guiTypes = {{ {source} }}"));
        parse_gui_document(&document)
            .into_iter()
            .next()
            .expect("one GUI root")
    }

    #[test]
    fn runtime_resolved_smooth_list_uses_exact_cumulative_rows_and_scroll() {
        let list = root(
            r#"smoothListBoxType = {
                name = "equipments_list"
                position = { x = 0 y = 0 }
                size = { x = 580 y = 1 }
                scrollbarType = "standardlistbox_slider"
            }"#,
        );
        let viewport = Rect::new(10, 20, 580, 100);
        let sizes = [(580, 45), (580, 45), (580, 45)];
        let rects = plan_authenticated_smooth_list_rects(
            &list,
            viewport,
            &sizes,
            35,
            &ComposeOptions::new(800, 600),
        )
        .expect("controller-resolved XUI16 smooth list");
        assert_eq!(
            rects,
            [
                Rect::new(10, -15, 580, 45),
                Rect::new(10, 30, 580, 45),
                Rect::new(10, 75, 580, 45),
            ]
        );
        assert!(plan_authenticated_smooth_list_rects(
            &list,
            viewport,
            &sizes,
            36,
            &ComposeOptions::new(800, 600),
        )
        .is_err());
    }

    #[test]
    fn runtime_resolved_smooth_list_rejects_family_and_extent_substitution() {
        let grid = root(
            r#"gridBoxType = {
                name = "equipments_list"
                size = { x = 580 y = 1 }
                slotsize = { x = 580 y = 45 }
            }"#,
        );
        let viewport = Rect::new(10, 20, 580, 100);
        assert!(plan_authenticated_smooth_list_rects(
            &grid,
            viewport,
            &[(580, 45)],
            0,
            &ComposeOptions::new(800, 600),
        )
        .is_err());
        let list = root(
            r#"smoothListBoxType = {
                name = "equipments_list"
                size = { x = 580 y = 1 }
            }"#,
        );
        assert!(plan_authenticated_smooth_list_rects(
            &list,
            viewport,
            &[(580, 0)],
            0,
            &ComposeOptions::new(800, 600),
        )
        .is_err());
    }
}
