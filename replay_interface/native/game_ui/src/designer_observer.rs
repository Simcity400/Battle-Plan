pub mod authored_inputs;
pub(crate) mod controller;
pub mod item_database;

use crate::assets::AssetResolver;
use crate::compose::{
    resolve_element_rect, ComposeOptions, ComposeOutput, ComposeReport, Compositor, Rect,
    UiAssetCache,
};
use crate::definition_admission::{
    load_effective_definition_catalog, EffectiveGuiRootDefinition, ResolvedGuiDefinitionFile,
};
use crate::design_catalog::{
    resolve_observer_designer_view_state, AdmittedObserverDesignerView,
    ObserverDesignerViewState,
};
use crate::font::alpha_blend;
use crate::gfx::SpriteRegistry;
use crate::gui::GuiElement;
use crate::gui_scene_util::{pair, prefix_element_names, set_position_upper_left};
use crate::loc::Localisation;
use crate::observer_collection::{
    compose_observer_row_layers, plan_observer_collection, plan_xui13_known_prefix, AuthoredCollectionSource,
    AuthoredRowBinding, ObserverRowComposeInput, ObserverSemanticCollection, ObserverSemanticRow,
    StandardGridAuthority, ViewerCollectionState,
};
use crate::paradox_text::Value;
use crate::readonly_design_peace::{designer_chrome, DesignerViewKind};
use crate::topbar::{flag_candidates_for_size, FlagSizeFolder};
use crate::widget_layout::gridbox_layout;
use crate::{GameUiError, Result};
use replay_core::NamedCountry;
use replay_core::{Leaf, WorldFrame};
use std::collections::{BTreeMap, BTreeSet};

pub const XUI13_SUPPORTED_FAMILIES: [DesignerViewKind; 3] = [
    DesignerViewKind::Ship,
    DesignerViewKind::Tank,
    DesignerViewKind::Plane,
];

const INTEL_CONTAINER: &str = "designer_intel_container";
const GRID_CONTAINER: &str = "gridbox_container";
const VARIANT_GRID: &str = "acquired_variants_grid";
const COUNTRY_FILTER_CONTAINER: &str = "country_filter_container";
const COUNTRY_FILTER_GRID: &str = "variant_country_filter_gridbox";
const VARIANT_ROW_TEMPLATE: &str = "designer_equipment_entry";
const COUNTRY_ROW_TEMPLATE: &str = "designer_creator_entry";
const MISSION_ROW_TEMPLATE: &str = "mission_grid_item";

#[derive(Clone, Debug)]
struct DesignerFamilyChrome {
    root_source_file: String,
    root: GuiElement,
    row_source_file: String,
    row_template: GuiElement,
    country_row_source_file: String,
    country_row_template: GuiElement,
    mission_source_file: String,
    mission_template: GuiElement,
}

#[derive(Clone, Debug)]
enum EffectiveDesignerWindow {
    Admitted {
        source_file: String,
    },
    Unsupported {
        source_file: String,
        constructs: BTreeSet<String>,
    },
    WrongRuntimeType,
}

#[derive(Clone, Debug)]
pub struct ObserverDesignerChromeCatalog {
    by_family: BTreeMap<DesignerViewKind, DesignerFamilyChrome>,
    effective_windows: BTreeMap<String, EffectiveDesignerWindow>,
    authored: std::result::Result<authored_inputs::DesignerAuthoredInputs, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverDesignerSurface {
    pub family: DesignerViewKind,
    pub owner_tag: String,
    pub authored_root: String,
    pub source_files: Vec<String>,
    pub ordered_design_ids: Vec<String>,
    pub visible_design_ids: Vec<String>,
    pub selected_design_id: Option<String>,
    pub ordered_country_filter_tags: Vec<String>,
    pub visible_country_filter_tags: Vec<String>,
    pub selected_country_filter_tags: Vec<String>,
    pub output: ComposeOutput,
    pub field_refusals: BTreeMap<String, String>,
    pub membership_complete: bool,
    pub country_filters_complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverDesignerEffectiveMembership {
    pub ordered_design_ids: Vec<String>,
    pub country_filter_tags: Vec<String>,
}

impl ObserverDesignerChromeCatalog {
    pub fn load(resolver: &AssetResolver) -> Result<Self> {
        let files = resolver.list_files_recursive("interface", Some("gui"))?;
        let catalog = load_effective_definition_catalog(
            files
                .into_iter()
                .map(|file| ResolvedGuiDefinitionFile::read(resolver, &file))
                .collect::<crate::Result<Vec<_>>>()?,
        )?;
        let row = effective_named_root(&catalog.effective_roots, VARIANT_ROW_TEMPLATE)?;
        let country_row = effective_named_root(&catalog.effective_roots, COUNTRY_ROW_TEMPLATE)?;
        let mission = effective_named_root(&catalog.effective_roots, MISSION_ROW_TEMPLATE)?;
        let effective_windows = index_effective_designer_windows(&catalog.effective_roots);
        let mut by_family = BTreeMap::new();
        for family in XUI13_SUPPORTED_FAMILIES {
            let root_name = designer_chrome(family).root_widget;
            let root = effective_named_root(&catalog.effective_roots, root_name)?;
            require_catalogue_tree(&root.root)?;
            by_family.insert(
                family,
                DesignerFamilyChrome {
                    root_source_file: root.source_file.clone(),
                    root: root.root.clone(),
                    row_source_file: row.source_file.clone(),
                    row_template: row.root.clone(),
                    country_row_source_file: country_row.source_file.clone(),
                    country_row_template: country_row.root.clone(),
                    mission_source_file: mission.source_file.clone(),
                    mission_template: mission.root.clone(),
                },
            );
        }
        Ok(Self {
            by_family,
            authored: authored_inputs::DesignerAuthoredInputs::with_windows(resolver, effective_windows.clone()).map_err(|error| error.to_string()),
            effective_windows,
        })
    }

    fn authored_inputs(&self) -> Result<&authored_inputs::DesignerAuthoredInputs> {
        self.authored.as_ref().map_err(|reason| GameUiError::missing(reason.clone()))
    }

    pub fn effective_country_filter_tags(
        &self,
        frame: &WorldFrame,
        owner_tag: NamedCountry<'_>,
        viewer: &ObserverDesignerViewState,
    ) -> Result<Vec<String>> {
        let (_, admitted) = resolve_observer_designer_view_state(self.authored_inputs()?, frame, owner_tag, viewer)
            .map_err(|error| {
                GameUiError::missing(format!("XUI13 catalogue admission rejected: {error:?}"))
            })?;
        if !admitted.country_filters_complete() {
            return Err(GameUiError::missing(admitted.field_refusals()["country_filters"].clone()));
        }
        Ok(self
            .effective_country_filter_rows(&admitted, owner_tag.tag())?
            .into_iter()
            .map(|(tag, _source_file)| tag)
            .collect())
    }

    pub fn effective_membership(
        &self,
        frame: &WorldFrame,
        owner_tag: NamedCountry<'_>,
        viewer: &ObserverDesignerViewState,
    ) -> Result<ObserverDesignerEffectiveMembership> {
        let (_, admitted) = resolve_observer_designer_view_state(self.authored_inputs()?, frame, owner_tag, viewer)
            .map_err(|error| {
                GameUiError::missing(format!("XUI13 catalogue admission rejected: {error:?}"))
            })?;
        if !admitted.membership_complete() || !admitted.country_filters_complete() {
            return Err(GameUiError::missing(format!("XUI13 exact action membership is unresolved: {:?}; {:?}",
                admitted.unresolved_suffix(), admitted.field_refusals())));
        }
        let mut ordered_design_ids = Vec::new();
        for candidate in admitted.ordered() {
            if designer_window_witness(
                &self.effective_windows,
                &candidate.designer_window_tokens,
                owner_tag.tag(),
            )?
            .is_some()
            {
                ordered_design_ids.push(candidate.record.design_id.clone());
            }
        }
        let country_filter_tags = self
            .effective_country_filter_rows(&admitted, owner_tag.tag())?
            .into_iter()
            .map(|(tag, _source_file)| tag)
            .collect();
        Ok(ObserverDesignerEffectiveMembership {
            ordered_design_ids,
            country_filter_tags,
        })
    }

    fn effective_country_filter_rows(
        &self,
        admitted: &AdmittedObserverDesignerView,
        owner_tag: &str,
    ) -> Result<Vec<(String, String)>> {
        let mut rows = Vec::new();
        for row in admitted.country_filter_rows() {
            let mut witness_source = None;
            for tokens in &row.candidate_designer_window_tokens {
                if let Some(source) =
                    designer_window_witness(&self.effective_windows, tokens, owner_tag)?
                {
                    witness_source = Some(source.to_string());
                    break;
                }
            }
            if let Some(source) = witness_source {
                rows.push((row.country_tag.clone(), source));
            }
        }
        Ok(rows)
    }

    pub fn render(
        &self,
        resolver: &AssetResolver,
        registry: &SpriteRegistry,
        localisation: &Localisation,
        frame: &WorldFrame,
        owner_tag: NamedCountry<'_>,
        viewer: &ObserverDesignerViewState,
        asset_cache: &mut UiAssetCache,
        viewport_width: usize,
        viewport_height: usize,
    ) -> Result<ObserverDesignerSurface> {
        let chrome = self.by_family.get(&viewer.family).ok_or_else(|| {
            GameUiError::missing(format!(
                "XUI13 has no equipment-designer compositor for {:?}",
                viewer.family
            ))
        })?;
        let (_, admitted) = resolve_observer_designer_view_state(self.authored_inputs()?, frame, owner_tag, viewer)
            .map_err(|error| {
                GameUiError::missing(format!("XUI13 catalogue admission rejected: {error:?}"))
            })?;
        let expected_hour = u64::try_from(frame.hour_index())
            .map_err(|_| GameUiError::missing("XUI13 cannot compose a negative replay hour"))?;
        let options =
            ComposeOptions::new(viewport_width, viewport_height).with_localisation(localisation);
        let viewport = Rect::new(
            0,
            0,
            i32::try_from(viewport_width)
                .map_err(|_| GameUiError::missing("XUI13 viewport width exceeds i32"))?,
            i32::try_from(viewport_height)
                .map_err(|_| GameUiError::missing("XUI13 viewport height exceeds i32"))?,
        );
        let root_rect = resolve_element_rect(&chrome.root, viewport, (0, 0), &options);
        let intel = direct_child(&chrome.root, INTEL_CONTAINER)?;
        let intel_rect = resolve_element_rect(intel, root_rect, (0, 0), &options);
        let grid_parent = direct_child(intel, GRID_CONTAINER)?;
        let grid_parent_rect = resolve_element_rect(grid_parent, intel_rect, (0, 0), &options);
        let grid = direct_child(grid_parent, VARIANT_GRID)?;
        let country_parent = direct_child(intel, COUNTRY_FILTER_CONTAINER)?;
        let country_parent_rect =
            resolve_element_rect(country_parent, intel_rect, (0, 0), &options);
        let country_grid = direct_child(country_parent, COUNTRY_FILTER_GRID)?;
        let mut membership_source_files = BTreeSet::new();
        let mut controller_rows = Vec::new();
        for candidate in admitted.ordered() {
            if let Some(source) = designer_window_witness(
                &self.effective_windows,
                &candidate.designer_window_tokens,
                owner_tag.tag(),
            )? {
                membership_source_files.insert(source.to_string());
                controller_rows.push(candidate);
            }
        }
        if let Some(selected) = admitted.selected() {
            if !controller_rows
                .iter()
                .any(|candidate| candidate.record.design_id == selected.record.design_id)
            {
                return Err(GameUiError::missing(format!(
                    "XUI13 selected row {} has no effective designer window",
                    selected.record.design_id
                )));
            }
        }

        let subtype = family_token(viewer.family);
        let semantic_rows = controller_rows
            .iter()
            .enumerate()
            .map(|(ordinal, candidate)| {
                Ok(ObserverSemanticRow {
                    stable_item_id: candidate.record.design_id.clone(),
                    source_ordinal: u32::try_from(ordinal)
                        .map_err(|_| GameUiError::missing("XUI13 row ordinal exceeds u32"))?,
                    subtype_id: subtype.to_string(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let semantic = ObserverSemanticCollection {
            collection_instance_token: format!(
                "xui13/{expected_hour}/{owner_tag}/{subtype}/catalogue"
            ),
            frame_hour: expected_hour,
            context_country: Some(owner_tag.to_string()),
            complete: admitted.membership_complete(),
            rows: semantic_rows,
        };
        let instance_tokens = controller_rows
            .iter()
            .map(|candidate| {
                format!(
                    "xui13/{expected_hour}/{owner_tag}/{subtype}/{}",
                    candidate.record.design_id
                )
            })
            .collect::<Vec<_>>();
        let row_templates = controller_rows
            .iter()
            .map(|candidate| {
                row_template_with_missions(
                    &chrome.row_template,
                    &chrome.mission_template,
                    candidate.missions.as_ref().ok().copied(),
                    viewport,
                    &options,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let element_source_identity =
            format!("{}#{}", chrome.row_source_file, VARIANT_ROW_TEMPLATE);
        let bindings = controller_rows
            .iter()
            .enumerate()
            .map(|(ordinal, candidate)| {
                Ok(AuthoredRowBinding {
                    stable_item_id: &candidate.record.design_id,
                    subtype_id: subtype,
                    element_source_identity: &element_source_identity,
                    instance_token: &instance_tokens[ordinal],
                    template: &row_templates[ordinal],
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let root_source_identity = format!(
            "{}#{}",
            chrome.root_source_file,
            designer_chrome(viewer.family).root_widget
        );
        let owning_window_id = root_source_identity.clone();
        let collection_element_id = format!(
            "{}/designer_intel_container/gridbox_container/acquired_variants_grid",
            root_source_identity
        );
        let source = |parent_rect| AuthoredCollectionSource {
            owning_window_id: &owning_window_id,
            root_source_identity: &root_source_identity,
            collection_element_id: &collection_element_id,
            collection_element: grid,
            effective_row_template_identity: Some(&element_source_identity),
            effective_row_template: Some(&chrome.row_template),
            parent_rect,
            grid_authority: Some(StandardGridAuthority::Xui13AcquiredVariantsStandardAutoAppend),
        };
        let collection_viewer = |visible_clip| ViewerCollectionState {
            scroll_offset_pixels: (0, 0),
            window_stack_ordinal: 0,
            root_paint_ordinal: 0,
            visible_clip,
        };
        let plan_rows = if admitted.membership_complete() { plan_observer_collection } else { plan_xui13_known_prefix };
        let unscrolled_plan = plan_rows(
            expected_hour,
            &semantic,
            source(grid_parent_rect),
            &bindings,
            collection_viewer(grid_parent_rect),
            &options,
        )?;
        let max_scroll = unscrolled_plan
            .content_extent_pixels()
            .1
            .saturating_sub(grid_parent_rect.h())
            .max(0);
        let wheel_factor = authored_scroll_wheel_factor(grid_parent)?;
        let requested_scroll_y = admitted
            .scroll_wheel_steps()
            .checked_mul(wheel_factor)
            .and_then(|pixels| i32::try_from(pixels).ok())
            .ok_or_else(|| GameUiError::missing("XUI13 authored row scroll overflows"))?;
        let known_scroll = prefix_scroll_offset(requested_scroll_y, max_scroll, admitted.membership_complete());
        let scroll_y = known_scroll.unwrap_or(0);
        let plan = if scroll_y == 0 {
            unscrolled_plan
        } else {
            let shifted_parent = Rect::new(
                grid_parent_rect.x(),
                grid_parent_rect
                    .y()
                    .checked_sub(scroll_y)
                    .ok_or_else(|| GameUiError::missing("XUI13 parent scroll overflows"))?,
                grid_parent_rect.w(),
                grid_parent_rect.h(),
            );
            plan_rows(
                expected_hour,
                &semantic,
                source(shifted_parent),
                &bindings,
                collection_viewer(grid_parent_rect),
                &options,
            )?
        };

        let mut base_root = chrome.root.clone();
        base_root.children.retain(|child| {
            child.name.eq_ignore_ascii_case("background")
                || child.name.eq_ignore_ascii_case(INTEL_CONTAINER)
                || child.name.eq_ignore_ascii_case("designer_title")
                || child.name.eq_ignore_ascii_case("blueprint_type_name")
        });
        uniquify_filter_children(&mut base_root)?;
        let mut base_options = options.clone();
        bind_selected_header(&admitted, &mut base_options);
        bind_filter_frames(viewer, &mut base_options);
        let mut base_compositor = Compositor::with_cache(resolver, registry, asset_cache);
        let mut output = base_compositor.compose(
            &wrapper(viewport_width, viewport_height, vec![base_root]),
            &base_options,
        );
        drop(base_compositor);

        let mut field_refusals = admitted.field_refusals().clone();
        if let Some(suffix) = admitted.unresolved_suffix() {
            field_refusals.insert("row_suffix".into(), format!("XUI13 unresolved suffix at country={} list={} ordinal={} design={}: {}",
                suffix.source_country_registry_index, suffix.source_list, suffix.source_list_ordinal, suffix.design_id, suffix.reason));
        }
        if known_scroll.is_none() {
            field_refusals.insert("row_scroll".into(), "XUI13 requested scroll clamp depends on the unresolved suffix extent".into());
        }
        let mut row_options = Vec::with_capacity(controller_rows.len());
        let mut flag_cache: BTreeMap<String, crate::dds::RgbaImage> = BTreeMap::new();
        for candidate in &controller_rows {
            let record = &candidate.record;
            let mut row_option = options.clone();
            let name = record
                .name
                .as_deref()
                .or(record.archetype.as_deref())
                .ok_or_else(|| GameUiError::missing("XUI13 row has no display identity"))?;
            let selected = admitted
                .selected()
                .is_some_and(|selected| selected.record.design_id == record.design_id);
            let name = if selected {
                format!("§H{name}§!")
            } else {
                format!("§W{name}§!")
            };
            row_option.text_overrides.insert("name".to_string(), name);
            let icon = match &candidate.icon {
                Ok(crate::design_catalog::DesignerRowIcon::Authored) => Ok(()),
                Ok(crate::design_catalog::DesignerRowIcon::Sprite(token)) => {
                    row_option.sprite_name_overrides.insert("equipment".into(), token.clone());
                    Ok(())
                }
                Ok(crate::design_catalog::DesignerRowIcon::Texture(path)) => resolver.decode_texture(path)
                    .map(|texture| { row_option.sprite_overrides.insert("equipment".into(), texture.image); })
                    .map_err(|error| error.to_string()),
                Err(reason) => Err(reason.clone()),
            };
            if let Err(reason) = icon {
                row_option.hidden_elements.insert("equipment".into());
                field_refusals.insert(format!("{}/equipment_icon", record.design_id), reason);
            }
            match &candidate.missions {
                Ok(mask) => for bit in mission_bits(*mask) {
                    row_option.frame_overrides.insert(format!("mission_{bit}_mission_icon"), bit as usize);
                },
                Err(reason) => {
                    row_option.hidden_elements.insert("missions".into());
                    field_refusals.insert(format!("{}/missions", record.design_id), reason.clone());
                }
            }
            let row_owner = candidate.flag_country_tag.as_str();
            let flag = match flag_cache.get(row_owner) {
                Some(flag) => Ok(flag.clone()),
                None => resolve_small_flag(resolver, frame, row_owner),
            };
            match flag {
                Ok(flag) => {
                    flag_cache.insert(row_owner.to_string(), flag.clone());
                    row_option.sprite_overrides.insert("creator_flag".into(), flag);
                }
                Err(reason) => {
                    row_option.hidden_elements.insert("creator_flag".into());
                    field_refusals.insert(format!("{}/flag", record.design_id), reason.to_string());
                }
            }
            row_options.push(row_option);
        }
        let inputs = controller_rows
            .iter()
            .zip(&row_options)
            .map(|(candidate, options)| ObserverRowComposeInput {
                stable_item_id: candidate.record.design_id.as_str(),
                options,
            })
            .collect::<Vec<_>>();
        let visible_design_ids = if known_scroll.is_some() {
            let mut row_compositor = Compositor::with_cache(resolver, registry, asset_cache);
            let rows = compose_observer_row_layers(&mut row_compositor, &plan, &inputs)?;
            blend(&mut output.image, &rows.output.image)?;
            merge_report(&mut output.report, rows.output.report);
            output.debug_rects.extend(rows.output.debug_rects);
            rows.painted_rows
        } else { Vec::new() };

        let (country_tags, visible_country_filter_tags) = if admitted.country_filters_complete() {
        let effective_country_rows = self.effective_country_filter_rows(&admitted, owner_tag.tag())?;
        let mut country_tags = Vec::with_capacity(effective_country_rows.len());
        for (tag, source_file) in effective_country_rows {
            membership_source_files.insert(source_file);
            country_tags.push(tag);
        }
        let excluded_country_filters = admitted
            .excluded_country_filter_tags()
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if excluded_country_filters
            .iter()
            .any(|excluded| !country_tags.iter().any(|tag| tag == excluded))
        {
            return Err(GameUiError::missing(
                "XUI13 excluded country filter has no effective-window row",
            ));
        }
        let country_semantic = ObserverSemanticCollection {
            collection_instance_token: format!(
                "xui13/{expected_hour}/{owner_tag}/{subtype}/country_filters"
            ),
            frame_hour: expected_hour,
            context_country: Some(owner_tag.to_string()),
            complete: true,
            rows: country_tags
                .iter()
                .enumerate()
                .map(|(ordinal, tag)| {
                    Ok(ObserverSemanticRow {
                        stable_item_id: tag.clone(),
                        source_ordinal: u32::try_from(ordinal).map_err(|_| {
                            GameUiError::missing("XUI13 country-filter ordinal exceeds u32")
                        })?,
                        subtype_id: "country".to_string(),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        };
        let country_instance_tokens = country_tags
            .iter()
            .map(|tag| format!("xui13/{expected_hour}/{owner_tag}/{subtype}/country_filter/{tag}"))
            .collect::<Vec<_>>();
        let country_element_source_identity = format!(
            "{}#{}",
            chrome.country_row_source_file, COUNTRY_ROW_TEMPLATE
        );
        let country_bindings = country_tags
            .iter()
            .enumerate()
            .map(|(ordinal, tag)| AuthoredRowBinding {
                stable_item_id: tag,
                subtype_id: "country",
                element_source_identity: &country_element_source_identity,
                instance_token: &country_instance_tokens[ordinal],
                template: &chrome.country_row_template,
            })
            .collect::<Vec<_>>();
        let country_collection_element_id = format!(
            "{root_source_identity}/designer_intel_container/country_filter_container/variant_country_filter_gridbox"
        );
        let country_source = |parent_rect| AuthoredCollectionSource {
            owning_window_id: &owning_window_id,
            root_source_identity: &root_source_identity,
            collection_element_id: &country_collection_element_id,
            collection_element: country_grid,
            effective_row_template_identity: Some(&country_element_source_identity),
            effective_row_template: Some(&chrome.country_row_template),
            parent_rect,
            grid_authority: Some(
                StandardGridAuthority::Xui13VariantCountryFilterStandardAutoAppend,
            ),
        };
        let country_unscrolled = plan_observer_collection(
            expected_hour,
            &country_semantic,
            country_source(country_parent_rect),
            &country_bindings,
            collection_viewer(country_parent_rect),
            &options,
        )?;
        let country_max_scroll = country_unscrolled
            .content_extent_pixels()
            .1
            .saturating_sub(country_parent_rect.h())
            .max(0);
        let country_wheel_factor = authored_scroll_wheel_factor(country_parent)?;
        let requested_country_scroll_y = admitted
            .country_scroll_wheel_steps()
            .checked_mul(country_wheel_factor)
            .and_then(|pixels| i32::try_from(pixels).ok())
            .ok_or_else(|| GameUiError::missing("XUI13 authored country scroll overflows"))?;
        let country_scroll_y = requested_country_scroll_y.min(country_max_scroll);
        let country_plan = if country_scroll_y == 0 {
            country_unscrolled
        } else {
            let shifted_parent = Rect::new(
                country_parent_rect.x(),
                country_parent_rect
                    .y()
                    .checked_sub(country_scroll_y)
                    .ok_or_else(|| GameUiError::missing("XUI13 country scroll overflows"))?,
                country_parent_rect.w(),
                country_parent_rect.h(),
            );
            plan_observer_collection(
                expected_hour,
                &country_semantic,
                country_source(shifted_parent),
                &country_bindings,
                collection_viewer(country_parent_rect),
                &options,
            )?
        };
        let mut country_options = Vec::with_capacity(country_tags.len());
        for tag in &country_tags {
            let mut country_option = options.clone();
            let flag = match flag_cache.get(tag) {
                Some(flag) => Ok(flag.clone()),
                None => resolve_small_flag(resolver, frame, tag),
            };
            match flag {
                Ok(flag) => {
                    flag_cache.insert(tag.clone(), flag.clone());
                    country_option.sprite_overrides.insert("creator_flag".into(), flag);
                }
                Err(reason) => {
                    country_option.hidden_elements.insert("creator_flag".into());
                    field_refusals.insert(format!("country_filters/{tag}/flag"), reason.to_string());
                }
            }
            let _selected = !excluded_country_filters.contains(tag.as_str());
            country_options.push(country_option);
        }
        let country_inputs = country_tags
            .iter()
            .zip(&country_options)
            .map(|(tag, options)| ObserverRowComposeInput {
                stable_item_id: tag,
                options,
            })
            .collect::<Vec<_>>();
        let mut country_compositor = Compositor::with_cache(resolver, registry, asset_cache);
        let countries =
            compose_observer_row_layers(&mut country_compositor, &country_plan, &country_inputs)?;
        blend(&mut output.image, &countries.output.image)?;
        merge_report(&mut output.report, countries.output.report);
        output.debug_rects.extend(countries.output.debug_rects);

            (country_tags, countries.painted_rows)
        } else { (Vec::new(), Vec::new()) };

        let selected_design_id = admitted
            .selected()
            .map(|candidate| candidate.record.design_id.clone());
        Ok(ObserverDesignerSurface {
            family: viewer.family,
            owner_tag: owner_tag.to_string(),
            authored_root: designer_chrome(viewer.family).root_widget.to_string(),
            source_files: {
                let mut files = vec![
                    chrome.root_source_file.clone(),
                    chrome.row_source_file.clone(),
                    chrome.country_row_source_file.clone(),
                    chrome.mission_source_file.clone(),
                ];
                files.sort();
                files.dedup();
                files.extend(membership_source_files);
                files.sort();
                files.dedup();
                files
            },
            ordered_design_ids: controller_rows
                .iter()
                .map(|candidate| candidate.record.design_id.clone())
                .collect(),
            visible_design_ids,
            selected_design_id,
            ordered_country_filter_tags: country_tags.clone(),
            visible_country_filter_tags,
            selected_country_filter_tags: country_tags
                .into_iter()
                .filter(|tag| !admitted.excluded_country_filter_tags().contains(tag))
                .collect(),
            output,
            field_refusals,
            membership_complete: admitted.membership_complete(),
            country_filters_complete: admitted.country_filters_complete(),
        })
    }
}

fn prefix_scroll_offset(requested: i32, prefix_max: i32, complete: bool) -> Option<i32> {
    if complete { Some(requested.min(prefix_max)) }
    else if requested <= prefix_max { Some(requested) }
    else { None }
}

fn index_effective_designer_windows(
    roots: &[EffectiveGuiRootDefinition],
) -> BTreeMap<String, EffectiveDesignerWindow> {
    let mut effective_windows = BTreeMap::new();
    for root in roots {
        let name = root.root.name.to_ascii_lowercase();
        let window = if !root
            .root
            .element_type
            .eq_ignore_ascii_case("containerWindowType")
        {
            EffectiveDesignerWindow::WrongRuntimeType
        } else if root.admitted {
            EffectiveDesignerWindow::Admitted {
                source_file: root.source_file.clone(),
            }
        } else {
            EffectiveDesignerWindow::Unsupported {
                source_file: root.source_file.clone(),
                constructs: root.unsupported.clone(),
            }
        };
        effective_windows.insert(name, window);
    }
    effective_windows
}

fn effective_named_root<'a>(
    roots: &'a [EffectiveGuiRootDefinition],
    name: &str,
) -> Result<&'a EffectiveGuiRootDefinition> {
    let root = roots
        .iter()
        .rev()
        .find(|root| root.root.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| GameUiError::missing(format!("effective GUI root {name:?} is absent")))?;
    if !root.admitted {
        return Err(GameUiError::missing(format!(
            "effective GUI root {name:?} is not admitted: {:?}",
            root.unsupported
        )));
    }
    Ok(root)
}

fn require_catalogue_tree(root: &GuiElement) -> Result<()> {
    require_element_type(root, "containerWindowType")?;
    let intel = direct_child(root, INTEL_CONTAINER)?;
    require_element_type(intel, "containerWindowType")?;
    for filter in [
        "our_filter_entry",
        "foreign_filter_entry",
        "obsolete_filter_entry",
    ] {
        direct_child(intel, filter)?;
    }
    let grid_parent = direct_child(intel, GRID_CONTAINER)?;
    require_element_type(grid_parent, "containerWindowType")?;
    require_element_type(direct_child(grid_parent, VARIANT_GRID)?, "gridBoxType")?;
    let country_parent = direct_child(intel, COUNTRY_FILTER_CONTAINER)?;
    require_element_type(country_parent, "containerWindowType")?;
    require_element_type(
        direct_child(country_parent, COUNTRY_FILTER_GRID)?,
        "gridBoxType",
    )?;
    Ok(())
}

fn require_element_type(element: &GuiElement, expected: &str) -> Result<()> {
    if !element.element_type.eq_ignore_ascii_case(expected) {
        return Err(GameUiError::missing(format!(
            "XUI13 authored element {:?} has type {:?}, expected {expected:?}",
            element.name, element.element_type
        )));
    }
    Ok(())
}

fn direct_child<'a>(parent: &'a GuiElement, name: &str) -> Result<&'a GuiElement> {
    let mut matches = parent
        .children
        .iter()
        .filter(|child| child.name.eq_ignore_ascii_case(name));
    let found = matches.next().ok_or_else(|| {
        GameUiError::missing(format!(
            "XUI13 authored parent {:?} has no direct child {name:?}",
            parent.name
        ))
    })?;
    if matches.next().is_some() {
        return Err(GameUiError::missing(format!(
            "XUI13 authored parent {:?} has duplicate direct child {name:?}",
            parent.name
        )));
    }
    Ok(found)
}

fn family_token(family: DesignerViewKind) -> &'static str {
    match family {
        DesignerViewKind::Equipment => "equipment",
        DesignerViewKind::Tank => "tank",
        DesignerViewKind::Plane => "plane",
        DesignerViewKind::Ship => "ship",
        DesignerViewKind::Division => "division",
        DesignerViewKind::VariantUpgrade => "variant_upgrade",
    }
}

fn designer_window_witness<'a>(
    effective_windows: &'a BTreeMap<String, EffectiveDesignerWindow>,
    equipment_tokens: &[String],
    owner_tag: &str,
) -> Result<Option<&'a str>> {
    let suffix = owner_tag.trim().to_ascii_lowercase();
    if suffix.is_empty() || equipment_tokens.is_empty() {
        return Err(GameUiError::missing(
            "XUI13 designer-window lookup has empty owner/token authority",
        ));
    }
    let tokens = equipment_tokens
        .iter()
        .take(2)
        .map(|token| {
            let token = token.trim().to_ascii_lowercase();
            if token.is_empty() {
                return Err(GameUiError::missing(
                    "XUI13 designer-window chain contains an empty token",
                ));
            }
            Ok(token)
        })
        .collect::<Result<Vec<_>>>()?;
    let candidates = tokens
        .iter()
        .map(|token| format!("equipment_designer_{token}_{suffix}"))
        .chain(
            tokens
                .iter()
                .map(|token| format!("equipment_designer_{token}")),
        );
    for candidate in candidates {
        match effective_windows.get(&candidate) {
            None => {}
            Some(EffectiveDesignerWindow::Admitted { source_file }) => {
                return Ok(Some(source_file))
            }
            Some(EffectiveDesignerWindow::Unsupported {
                source_file,
                constructs,
            }) => {
                return Err(GameUiError::missing(format!(
                    "XUI13 effective designer window {candidate:?} from {source_file:?} contains unsupported constructs {constructs:?}"
                )))
            }
            Some(EffectiveDesignerWindow::WrongRuntimeType) => {
            }
        }
    }
    Ok(None)
}

fn authored_scroll_wheel_factor(element: &GuiElement) -> Result<u32> {
    let factor = element
        .raw
        .first_value_ci("scroll_wheel_factor")
        .and_then(crate::paradox_text::scalar_u32)
        .ok_or_else(|| {
            GameUiError::missing(format!(
                "XUI13 authored scroll container {:?} has no integer scroll_wheel_factor",
                element.name
            ))
        })?;
    if factor == 0 {
        return Err(GameUiError::missing(format!(
            "XUI13 authored scroll container {:?} has zero scroll_wheel_factor",
            element.name
        )));
    }
    Ok(factor)
}

fn mission_bits(mask: u32) -> Vec<u32> {
    (0_u32..32).filter(|bit| *bit != 11 && mask & (1_u32 << bit) != 0).collect()
}

fn row_template_with_missions(
    row_template: &GuiElement,
    mission_template: &GuiElement,
    mission_mask: Option<u32>,
    viewport: Rect,
    options: &ComposeOptions,
) -> Result<GuiElement> {
    let mut row = row_template.clone();
    let row_rect = resolve_element_rect(&row, viewport, (0, 0), options);
    let missions = direct_child_mut(&mut row, "missions")?;
    let missions_rect = resolve_element_rect(missions, row_rect, (0, 0), options);
    let mission_grid = direct_child_mut(missions, "mission_grid")?;
    let layout = gridbox_layout(mission_grid, missions_rect, options)?;
    mission_grid.children.clear();
    let bits = match mission_mask { Some(mask) => mission_bits(mask), None => Vec::new() };
    let capacity = match (layout.max_slots_x.filter(|count| *count > 0), layout.max_slots_y.filter(|count| *count > 0)) {
        (Some(x), Some(y)) => x.saturating_mul(y),
        _ => usize::MAX,
    };
    for (ordinal, bit) in bits.into_iter().take(capacity).enumerate() {
        let coordinate = crate::observer_collection::grid_coordinate(&layout, ordinal)?;
        let origin = layout.explicit_coordinate_origin(coordinate)?;
        let mut mission = mission_template.clone();
        set_position_upper_left(
            &mut mission,
            origin
                .0
                .checked_sub(layout.rect.x())
                .ok_or_else(|| GameUiError::missing("XUI13 mission x position overflows"))?,
            origin
                .1
                .checked_sub(layout.rect.y())
                .ok_or_else(|| GameUiError::missing("XUI13 mission y position overflows"))?,
        );
        prefix_element_names(&mut mission, &format!("mission_{bit}_"));
        mission_grid.children.push(mission);
    }
    Ok(row)
}

fn direct_child_mut<'a>(parent: &'a mut GuiElement, name: &str) -> Result<&'a mut GuiElement> {
    let mut found = None;
    for (index, child) in parent.children.iter().enumerate() {
        if child.name.eq_ignore_ascii_case(name) {
            if found.replace(index).is_some() {
                return Err(GameUiError::missing(format!(
                    "XUI13 authored parent {:?} has duplicate direct child {name:?}",
                    parent.name
                )));
            }
        }
    }
    let index = found.ok_or_else(|| {
        GameUiError::missing(format!(
            "XUI13 authored parent {:?} has no direct child {name:?}",
            parent.name
        ))
    })?;
    Ok(&mut parent.children[index])
}

fn uniquify_filter_children(root: &mut GuiElement) -> Result<()> {
    let intel = direct_child_mut(root, INTEL_CONTAINER)?;
    for filter_name in [
        "our_filter_entry",
        "foreign_filter_entry",
        "obsolete_filter_entry",
    ] {
        let filter = direct_child_mut(intel, filter_name)?;
        for child in &mut filter.children {
            prefix_element_names(child, &format!("{filter_name}_"));
        }
    }
    Ok(())
}

fn bind_filter_frames(viewer: &ObserverDesignerViewState, options: &mut ComposeOptions) {
    for (name, enabled) in [
        ("our_filter_entry_Background", viewer.our_filter_enabled),
        (
            "foreign_filter_entry_Background",
            viewer.foreign_filter_enabled,
        ),
        (
            "obsolete_filter_entry_Background",
            viewer.obsolete_filter_enabled,
        ),
    ] {
        options
            .frame_overrides
            .insert(name.to_string(), usize::from(enabled));
    }
}

fn bind_selected_header(admitted: &AdmittedObserverDesignerView, options: &mut ComposeOptions) {
    match admitted.selected() {
        Some(selected) => {
            if let Some(name) = selected.record.name.as_ref() {
                options
                    .text_overrides
                    .insert("designer_title".to_string(), name.clone());
            }
            if let Some(archetype) = selected.record.archetype.as_ref() {
                options
                    .text_overrides
                    .insert("blueprint_type_name".to_string(), archetype.clone());
            }
        }
        None => {
            options.hidden_elements.insert("designer_title".to_string());
            options
                .hidden_elements
                .insert("blueprint_type_name".to_string());
        }
    }
}

fn resolve_small_flag(
    resolver: &AssetResolver,
    frame: &WorldFrame,
    owner_tag: &str,
) -> Result<crate::dds::RgbaImage> {
    let identity = frame.country_identity(owner_tag).ok_or_else(|| {
        GameUiError::missing(format!("XUI13 owner {owner_tag:?} has no country identity"))
    })?;
    let ideology = measured_or_absent(&identity.ruling_ideology, "ruling ideology")?;
    let cosmetic = crate::binding::admitted_cosmetic_override(&identity.cosmetic_tag)
        .map_err(|detail| GameUiError::missing(format!("XUI13 {detail}")))?;
    for candidate in flag_candidates_for_size(
        Some(owner_tag),
        ideology,
        cosmetic,
        None,
        FlagSizeFolder::Small,
    ) {
        if resolver.resolve_optional_file(&candidate)?.is_some() {
            return resolver
                .decode_texture(&candidate)
                .map(|decoded| decoded.image);
        }
    }
    Err(GameUiError::missing(format!(
        "XUI13 cannot resolve an exact small flag for {owner_tag}"
    )))
}

fn measured_or_absent<'a>(leaf: &'a Leaf<String>, field: &str) -> Result<Option<&'a str>> {
    match leaf {
        Leaf::Measured(value) => Ok(Some(value.as_str())),
        Leaf::Absent => Ok(None),
        Leaf::Unobtained => Err(GameUiError::missing(format!(
            "XUI13 {field} is captured but unobtained"
        ))),
    }
}

fn wrapper(width: usize, height: usize, children: Vec<GuiElement>) -> GuiElement {
    GuiElement {
        element_type: "containerWindowType".to_string(),
        name: "xui13_observer_designer_root".to_string(),
        position: None,
        size: Some(pair(width as i32, height as i32)),
        orientation: Some("UPPER_LEFT".to_string()),
        sprite_refs: Vec::new(),
        font: None,
        text: None,
        format: None,
        children,
        raw: Value::Object(Vec::new()),
    }
}

pub fn blend(base: &mut crate::dds::RgbaImage, overlay: &crate::dds::RgbaImage) -> Result<()> {
    if base.width != overlay.width || base.height != overlay.height {
        return Err(GameUiError::missing(
            "XUI13 overlay dimensions differ from the production HUD",
        ));
    }
    for (dst, src) in base
        .pixels
        .chunks_exact_mut(4)
        .zip(overlay.pixels.chunks_exact(4))
    {
        let out = alpha_blend(
            [dst[0], dst[1], dst[2], dst[3]],
            [src[0], src[1], src[2], src[3]],
        );
        dst.copy_from_slice(&out);
    }
    Ok(())
}

fn merge_report(base: &mut ComposeReport, next: ComposeReport) {
    base.drawn_elements += next.drawn_elements;
    base.missing_sprites.extend(next.missing_sprites);
    base.undecodable_sprites.extend(next.undecodable_sprites);
    base.missing_fonts.extend(next.missing_fonts);
    for (font, glyphs) in next.missing_glyphs {
        base.missing_glyphs.entry(font).or_default().extend(glyphs);
    }
    base.localisation_misses.absorb(next.localisation_misses);
    for (element, issues) in next.unresolved_text_inputs {
        base.unresolved_text_inputs
            .entry(element)
            .or_default()
            .extend(issues);
    }
}

pub fn validate_complete_designer_report(report: &ComposeReport) -> Result<()> {
    if report.drawn_elements == 0
        || !report.missing_sprites.is_empty()
        || !report.undecodable_sprites.is_empty()
        || !report.missing_fonts.is_empty()
        || !report.missing_glyphs.is_empty()
        || !report.unresolved_text_inputs.is_empty()
    {
        return Err(GameUiError::missing(format!(
            "XUI13 composition incomplete: drawn={}; missing_sprites={:?}; undecodable_sprites={:?}; missing_fonts={:?}; missing_glyphs={:?}; unresolved_text={:?}",
            report.drawn_elements,
            report.missing_sprites,
            report.undecodable_sprites,
            report.missing_fonts,
            report.missing_glyphs,
            report.unresolved_text_inputs,
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// A TAG this frame's census admits. A test that means to name a country says
    /// so here; a test about a contended key asks `named_country` and gets `None`.
    fn named<'tag>(
        frame: &'tag replay_core::WorldFrame,
        tag: &'tag str,
    ) -> replay_core::NamedCountry<'tag> {
        frame.named_country(tag).expect("census-admitted tag")
    }

    use super::*;
    use crate::production_renderer::ProductionUiRenderer;
    use map_viewer::{GameFileResolver, PlaybackEnvironment};
    use replay_core::{
        canonical_json_sha256, ContentLayerKind, ContentManifest, LayerStorageKind,
        PortableDesignCatalog, RawFrame, RecordedContentLayer, ReplaySource, RequiredContent,
        RequiredContentKind, SourceError, SourceGameIdentity, SourceProfileIdentity,
        UsedContentKey, WorldTimeline, A27_LIVE_CATALOG_SCHEMA,
        A27_LIVE_CATALOG_TIMELINE_SCHEMA,
    };
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    pub(super) struct TestRoot(pub(super) PathBuf);

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct DesignerSource {
        catalog: Arc<PortableDesignCatalog>,
        rest: serde_json::Map<String, serde_json::Value>,
        hours: Vec<i64>,
    }

    impl ReplaySource for DesignerSource {
        fn game_id(&self) -> &str {
            "xui13-production-fixture"
        }

        fn hours(&self) -> &[i64] {
            &self.hours
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn design_catalog(&self) -> Option<&Arc<PortableDesignCatalog>> {
            Some(&self.catalog)
        }

        fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
            Ok(RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: {
                    let mut rest = self.rest.clone();
                    if replay_core::COUNTRY_COMPOSED_MEMBERS
                        .iter()
                        .any(|member| rest.contains_key(*member))
                    {
                        rest.entry("tag_db_id_index").or_insert_with(
                            || serde_json::json!({"country_composition_version": 2}),
                        );
                    }
                    rest.into()
                },
            })
        }
    }

    pub(super) fn world_from_catalog(raw: serde_json::Value, rest: serde_json::Value) -> Arc<WorldFrame> {
        let catalog = Arc::new(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap());
        let timeline = WorldTimeline::open(Box::new(DesignerSource {
            catalog, rest: rest.as_object().unwrap().clone(), hours: vec![900],
        })).unwrap();
        timeline.frame(900).unwrap()
    }

    fn controller_root(name: &str) -> String {
        format!(
            r#"containerWindowType = {{
                name = "{name}" position = {{ x = 0 y = 0 }} size = {{ width = 160 height = 120 }}
                containerWindowType = {{
                    name = "designer_intel_container" position = {{ x = 0 y = 0 }} size = {{ width = 160 height = 120 }}
                    instantTextBoxType = {{ name = "localized_caption" position = {{ x = 40 y = 0 }} size = {{ width = 80 height = 12 }} font = "xui13_test" text = "XUI13_CAPTION" }}
                    containerWindowType = {{ name = "our_filter_entry" position = {{ x = 0 y = 0 }} size = {{ width = 8 height = 8 }} iconType = {{ name = "Background" size = {{ width = 8 height = 8 }} spriteType = "GFX_xui13_test" }} }}
                    containerWindowType = {{ name = "foreign_filter_entry" position = {{ x = 10 y = 0 }} size = {{ width = 8 height = 8 }} iconType = {{ name = "Background" size = {{ width = 8 height = 8 }} spriteType = "GFX_xui13_test" }} }}
                    containerWindowType = {{ name = "obsolete_filter_entry" position = {{ x = 20 y = 0 }} size = {{ width = 8 height = 8 }} iconType = {{ name = "Background" size = {{ width = 8 height = 8 }} spriteType = "GFX_xui13_test" }} }}
                    containerWindowType = {{
                        name = "gridbox_container" position = {{ x = 0 y = 16 }} size = {{ width = 120 height = 80 }} scroll_wheel_factor = 10
                        gridBoxType = {{ name = "acquired_variants_grid" size = {{ width = 120 height = 80 }} slotsize = {{ width = 40 height = 24 }} format = UPPER_LEFT max_slots_horizontal = 3 add_horizontal = yes }}
                    }}
                    containerWindowType = {{
                        name = "country_filter_container" position = {{ x = 122 y = 16 }} size = {{ width = 32 height = 80 }} scroll_wheel_factor = 8
                        gridBoxType = {{ name = "variant_country_filter_gridbox" size = {{ width = 32 height = 80 }} slotsize = {{ width = 16 height = 12 }} format = UPPER_LEFT max_slots_horizontal = 2 add_horizontal = yes }}
                    }}
                }}
            }}"#
        )
    }

    fn designer_gui() -> Vec<u8> {
        format!(
            r#"guiTypes = {{
                {}
                {}
                {}
                containerWindowType = {{
                    name = "designer_equipment_entry" size = {{ width = 40 height = 24 }}
                    instantTextBoxType = {{ name = "name" position = {{ x = 18 y = 0 }} size = {{ width = 22 height = 8 }} font = "xui13_test" text = "" }}
                    iconType = {{ name = "equipment" position = {{ x = 0 y = 0 }} size = {{ width = 8 height = 8 }} spriteType = "GFX_xui13_test" }}
                    iconType = {{ name = "creator_flag" position = {{ x = 9 y = 0 }} size = {{ width = 8 height = 8 }} spriteType = "GFX_xui13_test" }}
                    containerWindowType = {{ name = "missions" position = {{ x = 0 y = 10 }} size = {{ width = 40 height = 10 }} gridBoxType = {{ name = "mission_grid" size = {{ width = 40 height = 10 }} slotsize = {{ width = 8 height = 8 }} format = UPPER_LEFT max_slots_horizontal = 6 add_horizontal = yes }} }}
                }}
                containerWindowType = {{ name = "designer_creator_entry" size = {{ width = 16 height = 12 }} iconType = {{ name = "creator_flag" size = {{ width = 8 height = 8 }} spriteType = "GFX_xui13_test" }} }}
                containerWindowType = {{
                    name = "mission_grid_item" size = {{ width = 8 height = 8 }}
                    iconType = {{ name = "mission_icon" size = {{ width = 8 height = 8 }} spriteType = "GFX_xui13_test" }}
                }}
            }}"#,
            controller_root("countryequipmentdesignerview"),
            controller_root("tank_designer_view"),
            controller_root("plane_designer_view")
        )
        .into_bytes()
    }

    fn test_tga() -> Vec<u8> {
        vec![
            0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 32, 0x20, 20, 80, 180,
            255, 180, 80, 20, 255,
        ]
    }

    fn test_font() -> Vec<u8> {
        let mut rows = String::new();
        for character in " DESIGNERCATALOGPanzer".chars().collect::<BTreeSet<_>>() {
            rows.push_str(&format!(
                "char id={} x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n",
                u32::from(character)
            ));
        }
        format!(
            "info face=\"XUI13\" size=8 padding=0,0,0,0 spacing=0,0 outline=0\n\
             common lineHeight=8 base=7 scaleW=2 scaleH=1 pages=1\n\
             page id=0 file=\"xui13_font.tga\"\n{rows}"
        )
        .into_bytes()
    }

    pub(super) fn designer_catalog_value() -> serde_json::Value {
        let definition = json!({
        "archetype_token": "light_tank_chassis_1",
        "archetype_token_utf8_hex": "6c696768745f74616e6b5f636861737369735f31",
        "cached_stats": [],
        "cached_stats_complete": true,
        "explicit_entity": {"status":"measured","value":"","utf8_hex":""},
        "serialized_variant_flag_2b9d": {"status":"measured","raw_u8":0},
        "mission_stat_operands": [
            {"group_key":4096,"stat_key":35,"fallback":{"status":"measured","raw_i64":0},
             "override_lookup":{"status":"group_absent","group_count":0,"group_key_prefix":[]}},
            {"group_key":8192,"stat_key":36,"fallback":{"status":"measured","raw_i64":0},
             "override_lookup":{"status":"group_absent","group_count":0,"group_key_prefix":[]}}
        ],
        "item_icon_override": {"status":"measured","active":true,"kind":0,"token":{"status":"measured","value":"GFX_xui13_test","utf8_hex":"4746585f78756931335f74657374"}},
        "display_name": "Panzer",
        "display_name_utf8_hex": "50616e7a6572",
        "equipment_token_raw": 71,
        "modules": [],
        "modules_complete": true,
        "upgrades": [],
        "upgrades_complete": true,
        "variant_name": null,
        "variant_name_utf8_hex": ""
    });
        json!({
            "schema": A27_LIVE_CATALOG_SCHEMA,
            "authority": {
                "program": "hoi4.exe 1.19.2.0.a729",
                "program_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
                "current_date_raw": 43799999_u64 + 900,
                "capture_hour_index": 900,
                "coherence": "double_read_all_roots_fields_and_membership"
            },
            "full_replacement": true,
            "completeness": {
                "definitions": true,
                "lifecycle": true,
                "membership": true,
                "owner": true,
                "ui_occurrences": true
            },
            "runtime_dlc_mask": {"status":"measured","raw_u32":128},
            "country_count": 1,
            "countries": [{
                "registry_index": 0,
                "tag": "GER",
                "designer_pointer_order_ordinal": 0
            }],
            "division_templates": [],
            "variants": [{
                "catalog_scan_ordinal": 0,
                "creator_tag": "GER",
                "definition": definition,
                "definition_complete": true,
                "developed_source_occurrences": [],
                "first_ui_discovery_ordinal": 0,
                "id": 7,
                "obsolete": false,
                "origin_override_tag": null,
                "owner_tag": "GER",
                "selected_country_tag": "GER",
                "source_occurrences": [{
                    "source_country_tag": "GER",
                    "source_country_registry_index": 0,
                    "source_list": "A",
                    "source_list_ordinal": 0
                }],
                "type": 70
            }]
        })
    }

    fn designer_catalog() -> Arc<PortableDesignCatalog> {
        let bytes = serde_json::to_vec(&designer_catalog_value()).unwrap();
        Arc::new(PortableDesignCatalog::decode_a27_live_json(&bytes).unwrap())
    }

    fn two_hour_designer_catalog() -> Arc<PortableDesignCatalog> {
        let first = designer_catalog_value();
        let mut second = first.clone();
        second["authority"]["current_date_raw"] = (43_799_999_u64 + 901).into();
        second["authority"]["capture_hour_index"] = 901_u64.into();
        let mut replacement = first["variants"][0].clone();
        replacement["id"] = 8_u64.into();
        replacement["definition"]["display_name"] = "Tiger".into();
        replacement["definition"]["display_name_utf8_hex"] = "5469676572".into();
        second["variants"] = json!([replacement.clone()]);
        let mut second_metadata = second.clone();
        second_metadata
            .as_object_mut()
            .unwrap()
            .remove("variants");
        second_metadata
            .as_object_mut()
            .unwrap()
            .remove("division_templates");
        let first_sha = canonical_json_sha256(&first).unwrap();
        let second_sha = canonical_json_sha256(&second).unwrap();
        let timeline = serde_json::to_vec(&json!({
            "schema": A27_LIVE_CATALOG_TIMELINE_SCHEMA,
            "authority": {
                "program": "hoi4.exe 1.19.2.0.a729",
                "program_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
            },
            "complete_sequence": true,
            "transport": "first_full_then_native_row_deltas",
            "capture_count": 2,
            "same_hour_recheck_count": 2,
            "first_capture_hour_index": 900,
            "last_capture_hour_index": 901,
            "captures": [
                {
                    "sequence": 0,
                    "capture_hour_index": 900,
                    "same_hour_rechecks": 1,
                    "same_hour_recheck_sha256": [first_sha],
                    "snapshot_sha256": first_sha,
                    "snapshot": first
                },
                {
                    "sequence": 1,
                    "capture_hour_index": 901,
                    "same_hour_rechecks": 1,
                    "same_hour_recheck_sha256": [second_sha],
                    "snapshot_sha256": second_sha,
                    "delta": {
                        "frame_metadata": second_metadata,
                        "variant_upserts": [replacement],
                        "variant_tombstones": ["7:70"],
                        "template_upserts": [],
                        "template_tombstones": [],
                        "ordered_design_ids": ["8:70"]
                    }
                }
            ]
        }))
        .unwrap();
        Arc::new(PortableDesignCatalog::decode_a27_live_json(&timeline).unwrap())
    }

    pub(super) fn production_fixture() -> (TestRoot, Arc<WorldFrame>) {
        let root = std::env::temp_dir().join(format!(
            "xui13_production_fixture_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("common/units/equipment")).unwrap();
        fs::create_dir_all(root.join("common/defines")).unwrap();
        fs::write(root.join("common/units/equipment/designer.txt"),
            b"equipments = { light_tank_chassis_1 = { is_archetype = yes type = armor module_slots = { main = {} } } }").unwrap();
        fs::write(root.join("common/defines/designer.lua"),
            b"NDefines = { NIntel = { NAVY_SHIP_TYPE_COUNT_INTEL_MAX = 0.5, ARMY_TEMPLATE_UNIT_COUNT_INTEL_MAX = 0.5, AIR_WING_TYPE_COUNT_INTEL_MAX = 0.5 } }").unwrap();
        fs::create_dir_all(root.join("interface")).unwrap();
        fs::create_dir_all(root.join("gfx/interface")).unwrap();
        fs::create_dir_all(root.join("gfx/flags/small")).unwrap();
        fs::create_dir_all(root.join("gfx/fonts")).unwrap();
        fs::create_dir_all(root.join("localisation/english")).unwrap();
        fs::write(root.join("interface/designer.gui"), designer_gui()).unwrap();
        fs::write(
            root.join("interface/designer_window.gui"),
            br#"guiTypes = { containerWindowType = { name = "equipment_designer_light_tank_chassis_1" size = { width = 1 height = 1 } } }"#,
        )
        .unwrap();
        fs::write(
            root.join("interface/designer.gfx"),
            br#"spriteTypes = { spriteType = { name = "GFX_xui13_test" texturefile = "gfx/interface/xui13_test.tga" noOfFrames = 2 } }"#,
        )
        .unwrap();
        fs::write(root.join("gfx/interface/xui13_test.tga"), test_tga()).unwrap();
        fs::write(root.join("gfx/flags/small/GER.tga"), test_tga()).unwrap();
        fs::write(root.join("gfx/fonts/xui13_test.fnt"), test_font()).unwrap();
        fs::write(root.join("gfx/fonts/xui13_font.tga"), test_tga()).unwrap();
        fs::write(
            root.join("localisation/english/xui13_l_english.yml"),
            b"l_english:\n XUI13_CAPTION:0 \"DESIGNER CATALOG\"\n",
        )
        .unwrap();
        for (path, root_name) in [
            ("interface/topbar.gui", "topbar"),
            ("interface/minimap.gui", "mini_map_interface"),
            ("interface/mapmodes_interface.gui", "MapmodesInterface_Ingame"),
            ("interface/theatreselector.gui", "theatre_selection_window"),
        ] {
            fs::write(
                root.join(path),
                format!(
                    "guiTypes = {{ containerWindowType = {{ name = \"{root_name}\" }} }}"
                ),
            )
            .unwrap();
        }
        let rest = json!({
            "countries": {
                "GER": {
                    "cosmetic_tag": null,
                    "politics": {"ruling_party": "fascism"}
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();
        let timeline = WorldTimeline::open(Box::new(DesignerSource {
            catalog: designer_catalog(),
            rest,
            hours: vec![900],
        }))
        .unwrap();
        let frame = timeline.frame(900).unwrap();
        (TestRoot(root), frame)
    }

    pub(super) fn admitted_production_renderer(
        root: &PathBuf,
        excluded_asset: Option<&str>,
    ) -> ProductionUiRenderer {
        admitted_production_renderer_with_assets(root, excluded_asset, &[])
    }

    pub(super) fn admitted_production_renderer_with_assets(
        root: &PathBuf,
        excluded_asset: Option<&str>,
        additional_assets: &[&str],
    ) -> ProductionUiRenderer {
        let paths = [
            "common/units/equipment/designer.txt",
            "common/defines/designer.lua",
            "interface/designer.gui",
            "interface/designer_window.gui",
            "interface/designer.gfx",
            "interface/topbar.gui",
            "interface/minimap.gui",
            "interface/mapmodes_interface.gui",
            "interface/theatreselector.gui",
            "gfx/interface/xui13_test.tga",
            "gfx/flags/small/GER.tga",
            "gfx/fonts/xui13_test.fnt",
            "gfx/fonts/xui13_font.tga",
            "localisation/english/xui13_l_english.yml",
        ];
        let resolver = GameFileResolver::vanilla(root).unwrap();
        let resolved = resolver
            .resolve_used_content(paths.into_iter().chain(additional_assets.iter().copied()).map(str::to_string))
            .unwrap();
        let vfs = "e".repeat(64);
        let required_content = resolved
            .iter()
            .map(|row| RequiredContent {
                content_key: row.logical_path.clone(),
                kind: RequiredContentKind::Other,
                observation_count: 1,
                consumers: vec![format!("xui13/fixture/{}", row.sha256)],
                content: UsedContentKey {
                    effective_vfs_manifest_sha256: vfs.clone(),
                    stored_game_relative_path: row.source_relative_path.clone(),
                    normalized_game_relative_path: row.logical_path.clone(),
                    winning_layer_ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    source_sha256: row.sha256.clone(),
                    payload_sha256: row.sha256.clone(),
                    payload_bytes: row.byte_len,
                },
            })
            .collect::<Vec<_>>();
        let source_game = SourceGameIdentity {
            version: "1.19.2".into(),
            build: "a729".into(),
            executable_sha256: "8".repeat(64),
            build_profile: "release".into(),
        };
        let mut manifest = ContentManifest {
            schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
            product_modset_sha256: "9".repeat(64),
            source_game: source_game.clone(),
            source_profile: SourceProfileIdentity {
                profile_id: "xui13-profile".into(),
                playset_id: "xui13-playset".into(),
                selection_sha256: "0".repeat(64),
            },
            effective_vfs_manifest_sha256: vfs,
            ordered_layers: vec![RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                display_name: "Base".into(),
                layer_id: "base".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: true,
            }],
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            recorder_local_settings: BTreeMap::new(),
            required_content,
            witnessed_content_namespaces: Vec::new(),
            witnessed_absent_content: ["interface/core.gfx", "gfx/flags/small/GER_fascism.tga", "gfx/flags/small/ITA_fascism.tga"]
                .into_iter().map(|path| {
                    assert!(!root.join(path).exists(), "fixture absence must be observed: {path}");
                    path.to_ascii_lowercase()
                }).collect::<std::collections::BTreeSet<_>>().into_iter().collect(),
            manifest_digest_sha256: None,
        };
        let sealed = crate::test_admission::seal_vanilla_namespace(root, &manifest.effective_vfs_manifest_sha256);
        let mut required: BTreeMap<_, _> = manifest.required_content.into_iter()
            .map(|row| (row.content.normalized_game_relative_path.clone(), row)).collect();
        manifest.required_content = sealed.winners().iter().map(|content| {
            if let Some(row) = required.remove(&content.normalized_game_relative_path) {
                assert_eq!(&row.content, content, "fixture content changed before namespace seal");
                row
            } else {
                RequiredContent {
                    content_key: content.normalized_game_relative_path.clone(),
                    kind: RequiredContentKind::Other,
                    observation_count: 1,
                    consumers: vec![format!("xui13/fixture/{}", content.payload_sha256)],
                    content: content.clone(),
                }
            }
        }).collect();
        assert!(required.is_empty(), "required fixture content disappeared before namespace seal");
        manifest.bind_observed_namespace(sealed).unwrap();
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        manifest = manifest.with_computed_digest().unwrap();
        let inventory = resolver
            .playback_inventory(
                &manifest,
                PlaybackEnvironment {
                    source_game,
                    source_profile: manifest.source_profile.clone(),
                    dlc_selection: Vec::new(),
                    localisation_language: "l_english".into(),
                    settings: BTreeMap::new(),
                },
            )
            .unwrap();
        let admission = manifest.admit_playback(&inventory).unwrap();
        let resolver = resolver.with_content_admission(&admission).unwrap();
        let selected_rows = admission
            .required_paths()
            .filter(|path| {
                excluded_asset.is_none_or(|excluded| !path.eq_ignore_ascii_case(excluded))
            })
            .map(|path| admission.require_used_content(path).unwrap().clone())
            .collect::<Vec<_>>();
        let assets =
            AssetResolver::from_used_content_rows(&resolver, &admission, selected_rows).unwrap();
        ProductionUiRenderer::new(assets, &admission).unwrap()
    }

    #[test]
    fn optional_assets_require_absence_witness_and_retain_consumer_capability() {
        let (root, _) = production_fixture();
        let renderer = admitted_production_renderer(&root.0, None);
        let assets = renderer.resolver();
        assert!(assets.resolve_optional_file("gfx/flags/small/GER.tga").unwrap().is_some());
        assert!(assets.resolve_optional_file("interface/core.gfx").unwrap().is_none());
        assert!(assets.resolve_texture_file("gfx/interface/xui13_test.png")
            .unwrap_err().message.contains("unwitnessed"),
            "an admitted same-stem TGA cannot authorize skipping an unknown PNG");
        let unknown = "gfx/interface/unrecorded_icon.tga";
        assert!(assets.resolve_optional_file(unknown).unwrap_err().message.contains("unwitnessed"));
        fs::write(root.0.join(unknown), test_tga()).unwrap();
        assert!(assets.resolve_optional_file(unknown).unwrap_err().message.contains("unwitnessed"),
            "an unrelated local file cannot change recording authority");
        let omitted = admitted_production_renderer(&root.0, Some("gfx/flags/small/GER.tga"));
        assert!(omitted.resolver().resolve_optional_file("gfx/flags/small/GER.tga")
            .unwrap_err().message.contains("consumer capability"));
        assert!(assets.resolve_optional_file("").is_err());
    }

    #[test]
    fn supported_product_families_are_exact_equipment_designer_families() {
        assert_eq!(
            XUI13_SUPPORTED_FAMILIES,
            [
                DesignerViewKind::Ship,
                DesignerViewKind::Tank,
                DesignerViewKind::Plane,
            ]
        );
        assert!(!XUI13_SUPPORTED_FAMILIES.contains(&DesignerViewKind::Equipment));
        assert!(!XUI13_SUPPORTED_FAMILIES.contains(&DesignerViewKind::Division));
        assert!(!XUI13_SUPPORTED_FAMILIES.contains(&DesignerViewKind::VariantUpgrade));
    }

    #[test]
    fn hour_advance_reanchors_after_tombstone_and_clears_vanished_selection() {
        let (root, _) = production_fixture();
        let renderer = admitted_production_renderer(&root.0, None);
        let authored = authored_inputs::DesignerAuthoredInputs::load(renderer.resolver()).unwrap();
        let rest = json!({
            "countries": {
                "GER": {"politics": {"ruling_party": "fascism"}}
            }
        })
        .as_object()
        .unwrap()
        .clone();
        let timeline = WorldTimeline::open(Box::new(DesignerSource {
            catalog: two_hour_designer_catalog(),
            rest,
            hours: vec![900, 901],
        }))
        .unwrap();
        let mut viewer = ObserverDesignerViewState {
            family: DesignerViewKind::Tank,
            anchor_design_id: String::new(),
            anchor_catalogue_hour: -1,
            anchor_catalogue_owner_tag: crate::stored_country_selection::StoredCountrySelection::of("GER"),
            anchor_catalogue_family: DesignerViewKind::Tank,
            selected_design_id: None,
            our_filter_enabled: true,
            foreign_filter_enabled: true,
            obsolete_filter_enabled: true,
            excluded_country_filter_tags: Vec::new(),
            scroll_wheel_steps: 0,
            country_scroll_wheel_steps: 0,
        };
        let (resolved_900, admitted_900) =
            resolve_observer_designer_view_state(&authored, &timeline.frame(900).unwrap(), named(&timeline.frame(900).unwrap(), "GER"), &viewer)
                .unwrap();
        assert_eq!(resolved_900.anchor_design_id, "7:70");
        assert_eq!(admitted_900.ordered()[0].record.design_id, "7:70");

        viewer = resolved_900;
        viewer.selected_design_id = Some("7:70".into());
        let (resolved_901, admitted_901) =
            resolve_observer_designer_view_state(&authored, &timeline.frame(901).unwrap(), named(&timeline.frame(901).unwrap(), "GER"), &viewer)
                .unwrap();
        assert_eq!(resolved_901.anchor_design_id, "8:70");
        assert_eq!(resolved_901.anchor_catalogue_hour, 901);
        assert_eq!(resolved_901.selected_design_id, None);
        assert_eq!(admitted_901.ordered()[0].record.design_id, "8:70");
        assert!(admitted_901.selected().is_none());
    }

    #[test]
    fn production_compositor_renders_nonempty_and_complete_measured_empty_families() {
        let (root, frame) = production_fixture();
        let renderer = admitted_production_renderer(&root.0, None);
        let mut tank = ObserverDesignerViewState {
            family: DesignerViewKind::Tank,
            anchor_design_id: String::new(),
            anchor_catalogue_hour: -1,
            anchor_catalogue_owner_tag: crate::stored_country_selection::StoredCountrySelection::of("GER"),
            anchor_catalogue_family: DesignerViewKind::Tank,
            selected_design_id: None,
            our_filter_enabled: true,
            foreign_filter_enabled: true,
            obsolete_filter_enabled: true,
            excluded_country_filter_tags: Vec::new(),
            scroll_wheel_steps: 0,
            country_scroll_wheel_steps: 0,
        };
        let (resolved, _) =
            resolve_observer_designer_view_state(&authored_inputs::DesignerAuthoredInputs::load(renderer.resolver()).unwrap(), &frame, named(&frame, "GER"), &tank).unwrap();
        assert_eq!(resolved.anchor_design_id, "7:70");
        assert_eq!(resolved.anchor_catalogue_hour, 900);
        let surface = renderer
            .compose_xui13_designer_surface(
                &frame,
                named(&frame, "GER"),
                &tank,
                160,
                120,
            )
            .unwrap();
        assert_eq!(surface.ordered_design_ids, ["7:70"]);
        assert!(surface.field_refusals.is_empty());
        assert!(surface
            .source_files
            .contains(&"interface/designer.gui".to_string()));
        assert!(surface
            .source_files
            .contains(&"interface/designer_window.gui".to_string()));
        validate_complete_designer_report(&surface.output.report).unwrap();
        assert!(surface
            .output
            .image
            .pixels
            .chunks_exact(4)
            .any(|pixel| pixel[3] != 0));

        tank.family = DesignerViewKind::Ship;
        tank.anchor_catalogue_family = DesignerViewKind::Ship;
        let measured_empty = renderer
            .compose_xui13_designer_surface(
                &frame,
                named(&frame, "GER"),
                &tank,
                160,
                120,
            )
            .unwrap();
        assert!(measured_empty.ordered_design_ids.is_empty());
        assert!(measured_empty.visible_design_ids.is_empty());
        validate_complete_designer_report(&measured_empty.output.report).unwrap();

        let missing_window = admitted_production_renderer(
            &root.0,
            Some("interface/designer_window.gui"),
        );
        let omitted = missing_window
            .compose_xui13_designer_surface(&frame, named(&frame, "GER"), &resolved, 160, 120)
            .err().expect("omitted effective GUI must be refused");
        assert!(matches!(omitted, crate::production_renderer::ProductionUiRejection::IncompleteComposition(message) if message.contains("interface/designer_window.gui")));

        fs::write(
            root.0.join("interface/designer_window.gui"),
            br#"guiTypes = {
                containerWindowType = {
                    name = "equipment_designer_light_tank_chassis_1"
                    size = { width = 1 height = 1 }
                    modCustomType = { name = "unsupported_winner" }
                }
            }"#,
        )
        .unwrap();
        let unsupported = admitted_production_renderer(&root.0, None);
        let partial = unsupported
            .compose_xui13_designer_surface(&frame, named(&frame, "GER"), &resolved, 160, 120)
            .unwrap();
        assert!(partial.ordered_design_ids.is_empty());
        assert!(partial.visible_design_ids.is_empty());
        assert!(!partial.membership_complete);
        assert!(!partial.country_filters_complete);
        assert!(partial.output.report.drawn_elements > 0);
        let refusal = &partial.field_refusals["row_suffix"];
        assert!(refusal.contains("designer_window.gui"), "{refusal}");
        assert!(refusal.contains("modcustomtype"), "{refusal}");
        assert!(refusal.contains("design=7:70"), "{refusal}");
    }

    #[test]
    fn xui13_hides_only_the_flag_whose_cosmetic_field_was_never_carried() {
        let (root, _) = production_fixture();
        let rest = json!({
            "countries": {
                "GER": {"politics": {"ruling_party": "fascism"}}
            }
        })
        .as_object()
        .unwrap()
        .clone();
        let timeline = WorldTimeline::open(Box::new(DesignerSource {
            catalog: designer_catalog(),
            rest,
            hours: vec![900],
        }))
        .unwrap();
        let frame = timeline.frame(900).unwrap();
        let renderer = admitted_production_renderer(&root.0, None);
        let viewer = ObserverDesignerViewState {
            family: DesignerViewKind::Tank,
            anchor_design_id: String::new(),
            anchor_catalogue_hour: -1,
            anchor_catalogue_owner_tag: crate::stored_country_selection::StoredCountrySelection::of("GER"),
            anchor_catalogue_family: DesignerViewKind::Tank,
            selected_design_id: None,
            our_filter_enabled: true,
            foreign_filter_enabled: true,
            obsolete_filter_enabled: true,
            excluded_country_filter_tags: Vec::new(),
            scroll_wheel_steps: 0,
            country_scroll_wheel_steps: 0,
        };
        let (resolved, _) = resolve_observer_designer_view_state(&authored_inputs::DesignerAuthoredInputs::load(renderer.resolver()).unwrap(), &frame, named(&frame, "GER"), &viewer).unwrap();
        let surface = renderer
            .compose_xui13_designer_surface(&frame, named(&frame, "GER"), &resolved, 160, 120)
            .unwrap();
        assert_eq!(surface.ordered_design_ids, ["7:70"]);
        assert_eq!(surface.visible_design_ids, ["7:70"]);
        assert!(surface.membership_complete);
        assert!(surface.output.report.drawn_elements > 0);
        assert!(surface.field_refusals["7:70/flag"].contains("cosmetic tag was never carried"));
        assert!(!surface.field_refusals.contains_key("7:70/equipment_icon"));
    }

    #[test]
    fn prefix_scroll_clamp_is_independent_only_up_to_the_proven_extent() {
        for prefix_max in [0, 1, 24, 96] {
            for requested in [0, prefix_max, prefix_max + 1, prefix_max + 100] {
                let partial = prefix_scroll_offset(requested, prefix_max, false);
                if requested <= prefix_max {
                    assert_eq!(partial, Some(requested));
                    for suffix_max in [prefix_max, prefix_max + 1, prefix_max + 1000] {
                        assert_eq!(partial, prefix_scroll_offset(requested, suffix_max, true));
                    }
                } else {
                    assert_eq!(partial, None);
                    assert_ne!(prefix_scroll_offset(requested, prefix_max, true),
                        prefix_scroll_offset(requested, prefix_max + 1000, true));
                }
            }
        }
        assert_eq!(prefix_scroll_offset(0, 0, false), Some(0));
        assert_eq!(prefix_scroll_offset(1, 0, false), None);
    }

    #[test]
    fn report_gate_rejects_every_incomplete_pixel_channel() {
        let mut report = ComposeReport {
            drawn_elements: 1,
            ..ComposeReport::default()
        };
        assert!(validate_complete_designer_report(&report).is_ok());
        report.missing_sprites.insert("missing".into());
        assert!(validate_complete_designer_report(&report).is_err());
    }

    #[test]
    fn effective_window_witness_preserves_native_lookup_order_and_source() {
        let sources = [
            (
                "equipment_designer_parent_ger".to_string(),
                EffectiveDesignerWindow::Admitted {
                    source_file: "interface/parent_country.gui".to_string(),
                },
            ),
            (
                "equipment_designer_current".to_string(),
                EffectiveDesignerWindow::Admitted {
                    source_file: "interface/current.gui".to_string(),
                },
            ),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            designer_window_witness(&sources, &["current".into(), "parent".into()], "GER",)
                .unwrap(),
            Some("interface/parent_country.gui")
        );
    }

    #[test]
    fn unsupported_last_winner_refuses_instead_of_falling_back() {
        let roots = [
            EffectiveGuiRootDefinition {
                source_file: "interface/vanilla_current.gui".to_string(),
                window_id: "vanilla#1".to_string(),
                root: wrapper(1, 1, Vec::new()),
                admitted: true,
                unsupported: BTreeSet::new(),
            },
            EffectiveGuiRootDefinition {
                source_file: "interface/parent_country.gui".to_string(),
                window_id: "parent#1".to_string(),
                root: wrapper(1, 1, Vec::new()),
                admitted: true,
                unsupported: BTreeSet::new(),
            },
            EffectiveGuiRootDefinition {
                source_file: "interface/modded_current.gui".to_string(),
                window_id: "mod#1".to_string(),
                root: wrapper(1, 1, Vec::new()),
                admitted: false,
                unsupported: ["dropdownboxtype".to_string()].into_iter().collect(),
            },
        ]
        .map(|mut root| {
            root.root.name = if root.source_file.contains("parent") {
                "equipment_designer_parent_ger".to_string()
            } else {
                "equipment_designer_current_ger".to_string()
            };
            root
        });
        let sources = index_effective_designer_windows(&roots);
        let error = designer_window_witness(&sources, &["current".into(), "parent".into()], "GER")
            .expect_err("unsupported winning current-country root must fail closed");
        assert!(error.to_string().contains("modded_current.gui"));
        assert!(error.to_string().contains("dropdownboxtype"));
    }

    #[test]
    fn wrong_type_last_winner_suppresses_stale_root_and_tries_parent() {
        let roots = [
            EffectiveGuiRootDefinition {
                source_file: "interface/vanilla_current.gui".to_string(),
                window_id: "vanilla#1".to_string(),
                root: wrapper(1, 1, Vec::new()),
                admitted: true,
                unsupported: BTreeSet::new(),
            },
            EffectiveGuiRootDefinition {
                source_file: "interface/parent_country.gui".to_string(),
                window_id: "parent#1".to_string(),
                root: wrapper(1, 1, Vec::new()),
                admitted: true,
                unsupported: BTreeSet::new(),
            },
            EffectiveGuiRootDefinition {
                source_file: "interface/modded_current.gui".to_string(),
                window_id: "mod#1".to_string(),
                root: wrapper(1, 1, Vec::new()),
                admitted: true,
                unsupported: BTreeSet::new(),
            },
        ]
        .map(|mut root| {
            root.root.name = if root.source_file.contains("parent") {
                "equipment_designer_parent_ger".to_string()
            } else {
                "equipment_designer_current_ger".to_string()
            };
            if root.source_file.contains("modded") {
                root.root.element_type = "windowType".to_string();
            }
            root
        });
        let sources = index_effective_designer_windows(&roots);
        assert_eq!(
            designer_window_witness(&sources, &["current".into(), "parent".into()], "GER")
                .expect("wrong-typed current winner is a failed named lookup"),
            Some("interface/parent_country.gui")
        );
    }
}
