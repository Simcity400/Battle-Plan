
use crate::assets::AssetResolver;
use crate::compose::{
    resolve_element_rect, ComposeOptions, ComposeOutput, Compositor, Rect, UiAssetCache,
};
use crate::dds::RgbaImage;
use crate::gfx::SpriteRegistry;
use crate::gui::{parse_measure, GuiElement};
use crate::gui_scene_util::{
    load_named_element, prefix_element_names, promote_show_position, set_position_upper_left,
};
use crate::loc::Localisation;
use crate::occupation_binding::{
    AdmittedOccupationSurface, GarrisonActivityRow, GarrisonEquipmentLossRow,
    OccupationStatePresentation, COMBAT_LOG_GUI, COUNTRY_OCCUPATION_GUI, COUNTRY_STATE_GUI,
    GARRISON_LOG_GUI,
};
use crate::observer_collection::plan_authenticated_smooth_list_rects;
use crate::widget_layout::gridbox_layout;
use crate::{GameUiError, Result};
use replay_core::{
    OccupationUiActivityPresentationExact, OccupationUiControlFrameLawExact,
    OccupationUiControllerPresentationExact, OccupationUiCountryGroupPresentationExact,
    OccupationUiEquipmentPresentationExact, OccupationUiMonthProjectionExact,
    OccupationUiStatePresentationExact, OccupationUiStateTemplateExact,
    OccupationUiStatusWidgetExact, ViewStatus, WorldFrame,
};
use std::collections::{BTreeMap, BTreeSet};

pub const XUI16_GUI_SOURCES: [&str; 4] = [
    COUNTRY_OCCUPATION_GUI,
    GARRISON_LOG_GUI,
    COMBAT_LOG_GUI,
    COUNTRY_STATE_GUI,
];

const COUNTRY_ROOT: &str = "countryoccupationview";
const COUNTRY_ROW: &str = "occupied_territory_country_entry";
const STATE_ROW: &str = "occupied_territory_state_entry";
const STATE_ROW_NO_RESISTANCE: &str = "occupied_territory_state_entry_no_resistance";
const RESISTANCE_STATUS: &str = "medium_resistance_status_container";
const COMPLIANCE_STATUS: &str = "tiny_compliance_status_container";
const GARRISON_ROOT: &str = "garrison_log_view";
const ACTIVITY_ROW: &str = "resistance_attack_item";
const EQUIPMENT_ROW: &str = "combat_log_enemy_loss_item";
const EQUIPMENT_COLLECTION_PATH: [&str; 3] = ["equipment_tab", "subwindow", "equipments_list"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OccupationPanel {
    Closed,
    Territories,
    GarrisonActivities,
    GarrisonEquipment,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OccupationTerritoryMetric {
    #[default]
    Resistance,
    Compliance,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationViewerState {
    pub panel: OccupationPanel,
    pub selected_country: String,
    pub expanded_country_tags: BTreeSet<String>,
    pub territory_sort_mode: u8,
    pub territory_metric: OccupationTerritoryMetric,
    pub show_non_resisting: bool,
    pub garrison_months: u8,
    pub territory_scroll_row: usize,
    pub activity_scroll_row: usize,
    pub equipment_scroll_row: usize,
    pub garrison_drag_position: Option<[i32; 2]>,
}

impl Default for OccupationViewerState {
    fn default() -> Self {
        Self {
            panel: OccupationPanel::Closed,
            selected_country: String::new(),
            expanded_country_tags: BTreeSet::new(),
            territory_sort_mode: 0,
            territory_metric: OccupationTerritoryMetric::Resistance,
            show_non_resisting: false,
            garrison_months: 12,
            territory_scroll_row: 0,
            activity_scroll_row: 0,
            equipment_scroll_row: 0,
            garrison_drag_position: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OccupationPixelRejection {
    IncompleteCarrier(String),
    SourceJoin(String),
    AuthoredLayout(String),
    ViewerInput(String),
    DynamicBinding(String),
    IncompleteComposition(String),
}

impl std::fmt::Display for OccupationPixelRejection {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncompleteCarrier(value)
            | Self::SourceJoin(value)
            | Self::AuthoredLayout(value)
            | Self::ViewerInput(value)
            | Self::DynamicBinding(value)
            | Self::IncompleteComposition(value) => out.write_str(value),
        }
    }
}

impl std::error::Error for OccupationPixelRejection {}

#[derive(Clone, Debug, PartialEq)]
pub struct OccupationPixelOutput {
    pub image: RgbaImage,
    pub report: crate::compose::ComposeReport,
    pub authored_root: String,
    pub source_record_id: String,
    pub materialized_row_ids: Vec<String>,
    pub applied_scroll_row: usize,
    pub input_region: OccupationInputRegion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OccupationInputRegion {
    pub panel: OccupationPanel,
    pub panel_rect: [i32; 4],
    pub scroll_rect: [i32; 4],
}

impl OccupationInputRegion {
    pub fn contains_panel_point(self, x: i32, y: i32) -> bool {
        contains_rect_point(self.panel_rect, x, y)
    }

    pub fn contains_scroll_point(self, x: i32, y: i32) -> bool {
        contains_rect_point(self.scroll_rect, x, y)
    }
}

fn contains_rect_point(rect: [i32; 4], x: i32, y: i32) -> bool {
    let [left, top, width, height] = rect;
    width > 0
        && height > 0
        && x >= left
        && y >= top
        && i64::from(x) < i64::from(left) + i64::from(width)
        && i64::from(y) < i64::from(top) + i64::from(height)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AppliedScroll {
    row: usize,
    pixels: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GarrisonEquipmentCollectionPlan {
    clip: Rect,
    scroll: AppliedScroll,
    row_rects: Vec<Rect>,
}

impl GarrisonEquipmentCollectionPlan {
    fn visible_row_rect(&self, ordinal: usize) -> Option<Rect> {
        self.row_rects.get(ordinal).copied().filter(|rect| {
            intersects_vertical(rect.y(), rect.h(), self.clip.y(), self.clip.h())
        })
    }

    fn input_region(&self, root_rect: Rect) -> OccupationInputRegion {
        OccupationInputRegion {
            panel: OccupationPanel::GarrisonEquipment,
            panel_rect: [root_rect.x(), root_rect.y(), root_rect.w(), root_rect.h()],
            scroll_rect: [self.clip.x(), self.clip.y(), self.clip.w(), self.clip.h()],
        }
    }
}

#[derive(Clone, Copy)]
struct AdmittedOccupationPixels<'a> {
    presentation: &'a OccupationUiControllerPresentationExact,
    controls: &'a OccupationUiControlFrameLawExact,
}

#[derive(Clone, Debug)]
pub struct OccupationPixelCompositor {
    country_root: GuiElement,
    country_row: GuiElement,
    state_row: GuiElement,
    state_row_no_resistance: GuiElement,
    resistance_status: GuiElement,
    compliance_status: GuiElement,
    resistance_modifier: GuiElement,
    garrison_root: GuiElement,
    activity_row: GuiElement,
    equipment_row: GuiElement,
}

impl OccupationPixelCompositor {
    pub fn load(resolver: &AssetResolver) -> Result<Self> {
        let mut cache = BTreeMap::new();
        let mut country_root =
            load_named_element(&mut cache, resolver, COUNTRY_OCCUPATION_GUI, COUNTRY_ROOT)?;
        promote_show_position(&mut country_root);
        Ok(Self {
            country_root,
            country_row: load_named_element(
                &mut cache,
                resolver,
                COUNTRY_OCCUPATION_GUI,
                COUNTRY_ROW,
            )?,
            state_row: load_named_element(&mut cache, resolver, COUNTRY_OCCUPATION_GUI, STATE_ROW)?,
            state_row_no_resistance: load_named_element(
                &mut cache,
                resolver,
                COUNTRY_OCCUPATION_GUI,
                STATE_ROW_NO_RESISTANCE,
            )?,
            resistance_status: load_named_element(
                &mut cache,
                resolver,
                COUNTRY_OCCUPATION_GUI,
                RESISTANCE_STATUS,
            )?,
            compliance_status: load_named_element(
                &mut cache,
                resolver,
                COUNTRY_OCCUPATION_GUI,
                COMPLIANCE_STATUS,
            )?,
            resistance_modifier: load_named_element(
                &mut cache,
                resolver,
                COUNTRY_OCCUPATION_GUI,
                "occupation_modifier_entry",
            )?,
            garrison_root: load_named_element(
                &mut cache,
                resolver,
                GARRISON_LOG_GUI,
                GARRISON_ROOT,
            )?,
            activity_row: load_named_element(&mut cache, resolver, GARRISON_LOG_GUI, ACTIVITY_ROW)?,
            equipment_row: load_named_element(&mut cache, resolver, COMBAT_LOG_GUI, EQUIPMENT_ROW)?,
        })
    }

    pub fn compose(
        &self,
        resolver: &AssetResolver,
        registry: &SpriteRegistry,
        localisation: &Localisation,
        cache: &mut UiAssetCache,
        frame: &WorldFrame,
        surface: &AdmittedOccupationSurface,
        viewer: &OccupationViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<Option<OccupationPixelOutput>, OccupationPixelRejection> {
        if viewer.panel == OccupationPanel::Closed {
            return Ok(None);
        }
        let exact = admit_exact_pixels(frame, surface, viewer)?;
        let output = match viewer.panel {
            OccupationPanel::Closed => Ok(None),
            OccupationPanel::Territories => self
                .compose_territories(
                    resolver,
                    registry,
                    localisation,
                    cache,
                    surface,
                    exact,
                    viewer,
                    viewport_width,
                    viewport_height,
                )
                .map(Some),
            OccupationPanel::GarrisonActivities | OccupationPanel::GarrisonEquipment => self
                .compose_garrison(
                    resolver,
                    registry,
                    localisation,
                    cache,
                    surface,
                    exact,
                    viewer,
                    viewport_width,
                    viewport_height,
                )
                .map(Some),
        }?;
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn compose_territories(
        &self,
        resolver: &AssetResolver,
        registry: &SpriteRegistry,
        localisation: &Localisation,
        cache: &mut UiAssetCache,
        surface: &AdmittedOccupationSurface,
        exact: AdmittedOccupationPixels<'_>,
        viewer: &OccupationViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<OccupationPixelOutput, OccupationPixelRejection> {
        let mut base = self.country_root.clone();
        let mut dynamic = self.country_root.clone();
        let mut options =
            ComposeOptions::new(viewport_width, viewport_height).with_localisation(localisation);
        bind_country_root(
            registry,
            &dynamic,
            "xui16_root_",
            &exact.presentation.root,
            exact.controls,
            viewer,
            &mut options,
        )?;
        let root_rect = resolve_element_rect(
            &dynamic,
            Rect::new(0, 0, viewport_width as i32, viewport_height as i32),
            (0, 0),
            &options,
        );
        let clip = descendant_rect(
            &dynamic,
            root_rect,
            &[
                "occupation_territories_list_container_bottom_margin",
                "occupation_territories_list_container",
                "occupation_territories_list",
            ],
            &options,
        )?;
        prefix_element_names(&mut base, "xui16_root_");
        prefix_element_names(&mut dynamic, "xui16_root_");
        let state_by_id = surface
            .states()
            .iter()
            .map(|row| (row.state_id, row))
            .collect::<BTreeMap<_, _>>();
        let country_order = exact
            .presentation
            .territory_country_orders
            .iter()
            .find(|order| {
                order.sort_mode == viewer.territory_sort_mode
                    && order.show_non_resisting == viewer.show_non_resisting
            })
            .ok_or_else(|| {
                OccupationPixelRejection::IncompleteCarrier(format!(
                    "XUI16 has no exact territory order for sort {} / nonresisting {}",
                    viewer.territory_sort_mode, viewer.show_non_resisting
                ))
            })?
            .occupied_country_tags
            .as_slice();
        let country_h = authored_extent(&self.country_row, "country row")?.1;
        let state_grid_y = named_position_y(&self.country_row, "state_grid")?;
        let state_grid_add = named_position_y(&self.country_row, "state_grid_size_add")?;
        let mut territory_row_offsets = Vec::new();
        let mut territory_content_height = 0_i32;
        for country_tag in country_order {
            let group = exact
                .presentation
                .country_groups
                .get(country_tag)
                .ok_or_else(|| {
                    OccupationPixelRejection::SourceJoin(format!(
                        "XUI16 territory order names absent country group {country_tag}"
                    ))
                })?;
            territory_row_offsets.push(territory_content_height);
            let expanded = viewer
                .expanded_country_tags
                .contains(&group.occupied_country_tag);
            let state_order = exact
                .presentation
                .state_orders_by_occupied_country
                .get(country_tag)
                .and_then(|orders| {
                    orders
                        .iter()
                        .find(|order| order.sort_mode == viewer.territory_sort_mode)
                })
                .map(|order| order.state_ids.as_slice())
                .ok_or_else(|| {
                    OccupationPixelRejection::IncompleteCarrier(format!(
                        "XUI16 country {} lacks sort {} state order",
                        group.occupied_country_tag, viewer.territory_sort_mode
                    ))
                })?;
            let mut state_height_sum = 0_i32;
            if expanded {
                let mut state_y = territory_content_height
                    .checked_add(state_grid_y)
                    .ok_or_else(|| {
                        OccupationPixelRejection::AuthoredLayout(
                            "XUI16 territory row offset overflow".into(),
                        )
                    })?;
                for state_id in state_order.iter().filter(|state_id| {
                    viewer.show_non_resisting
                        || exact
                            .presentation
                            .state_presentations
                            .get(state_id)
                            .is_some_and(|row| {
                                row.template == OccupationUiStateTemplateExact::Resisting60
                            })
                }) {
                    let exact_state = exact
                        .presentation
                        .state_presentations
                        .get(state_id)
                        .ok_or_else(|| {
                            OccupationPixelRejection::IncompleteCarrier(format!(
                                "XUI16 state {state_id} lacks template authority"
                            ))
                        })?;
                    let state_h =
                        authored_extent(self.state_template(exact_state.template)?, "state row")?.1;
                    territory_row_offsets.push(state_y);
                    state_y = state_y.checked_add(state_h).ok_or_else(|| {
                        OccupationPixelRejection::AuthoredLayout(
                            "XUI16 territory row offset overflow".into(),
                        )
                    })?;
                    state_height_sum = state_height_sum.checked_add(state_h).ok_or_else(|| {
                        OccupationPixelRejection::AuthoredLayout(
                            "XUI16 territory content height overflow".into(),
                        )
                    })?;
                }
            }
            let group_extra = if expanded {
                state_grid_add
                    .checked_add(state_height_sum)
                    .ok_or_else(|| {
                        OccupationPixelRejection::AuthoredLayout(
                            "XUI16 territory group height overflow".into(),
                        )
                    })?
            } else {
                0
            };
            territory_content_height = territory_content_height
                .checked_add(country_h)
                .and_then(|height| height.checked_add(group_extra))
                .ok_or_else(|| {
                    OccupationPixelRejection::AuthoredLayout(
                        "XUI16 territory content height overflow".into(),
                    )
                })?;
        }
        let territory_scroll = authored_variable_row_scroll_offset(
            viewer.territory_scroll_row,
            &territory_row_offsets,
            territory_content_height,
            clip.h(),
        )?;
        let mut content_y = -territory_scroll.pixels;
        let mut materialized = Vec::new();

        for country_tag in country_order {
            let group = exact
                .presentation
                .country_groups
                .get(country_tag)
                .ok_or_else(|| {
                    OccupationPixelRejection::SourceJoin(format!(
                        "XUI16 territory order names absent country group {country_tag}"
                    ))
                })?;
            let expanded = viewer
                .expanded_country_tags
                .contains(&group.occupied_country_tag);
            let state_order = exact
                .presentation
                .state_orders_by_occupied_country
                .get(country_tag)
                .and_then(|orders| {
                    orders
                        .iter()
                        .find(|order| order.sort_mode == viewer.territory_sort_mode)
                })
                .map(|order| order.state_ids.as_slice())
                .ok_or_else(|| {
                    OccupationPixelRejection::IncompleteCarrier(format!(
                        "XUI16 country {} lacks sort {} state order",
                        group.occupied_country_tag, viewer.territory_sort_mode
                    ))
                })?;
            let state_extents = state_order
                .iter()
                .filter(|state_id| {
                    viewer.show_non_resisting
                        || exact
                            .presentation
                            .state_presentations
                            .get(state_id)
                            .is_some_and(|row| {
                                row.template == OccupationUiStateTemplateExact::Resisting60
                            })
                })
                .map(|state_id| {
                    let exact_state = exact
                        .presentation
                        .state_presentations
                        .get(state_id)
                        .ok_or_else(|| {
                            OccupationPixelRejection::IncompleteCarrier(format!(
                                "XUI16 state {state_id} lacks template authority"
                            ))
                        })?;
                    let template = self.state_template(exact_state.template)?;
                    Ok((*state_id, authored_extent(template, "state row")?.1))
                })
                .collect::<std::result::Result<Vec<_>, OccupationPixelRejection>>()?;
            let group_extra = if expanded {
                state_extents
                    .iter()
                    .try_fold(state_grid_add, |height, (_, row_height)| {
                        height.checked_add(*row_height).ok_or_else(|| {
                            OccupationPixelRejection::AuthoredLayout(
                                "XUI16 country group height overflow".into(),
                            )
                        })
                    })?
            } else {
                0
            };
            let group_h = country_h.checked_add(group_extra).ok_or_else(|| {
                OccupationPixelRejection::AuthoredLayout(
                    "XUI16 country group height overflow".into(),
                )
            })?;
            if intersects_vertical(content_y, group_h, 0, clip.h()) {
                let prefix = format!("xui16_country_{}_", group.occupied_country_tag);
                let mut row = self.country_row.clone();
                set_position_upper_left(
                    &mut row,
                    clip.x() - root_rect.x(),
                    clip.y() - root_rect.y() + content_y,
                );
                attach_resistance_status(
                    registry,
                    &mut row,
                    &self.resistance_status,
                    &self.resistance_modifier,
                    "resistance_status_pos",
                    &group.resistance_status,
                    &prefix,
                    "resistance_status_",
                    &mut options,
                )?;
                attach_compliance_status(
                    registry,
                    &mut row,
                    &self.compliance_status,
                    "compliance_status_pos",
                    &group.compliance_status,
                    &prefix,
                    "compliance_status_",
                    &mut options,
                )?;
                bind_country_presentation(
                    resolver,
                    registry,
                    &row,
                    &prefix,
                    group,
                    expanded,
                    &mut options,
                )?;
                prefix_element_names(&mut row, &prefix);
                dynamic.children.push(row);
                materialized.push(format!("country:{}", group.occupied_country_tag));
            }
            if expanded {
                let mut state_y = content_y + state_grid_y;
                for (state_id, state_h) in state_extents {
                    let row_y = state_y;
                    if intersects_vertical(row_y, state_h, 0, clip.h()) {
                        let state = state_by_id.get(&state_id).ok_or_else(|| {
                            OccupationPixelRejection::SourceJoin(format!(
                                "XUI16 group {} names absent state {state_id}",
                                group.occupied_country_tag
                            ))
                        })?;
                        let exact_state = exact
                            .presentation
                            .state_presentations
                            .get(&state_id)
                            .ok_or_else(|| {
                                OccupationPixelRejection::IncompleteCarrier(format!(
                                    "XUI16 state {state_id} has no exact widget binding"
                                ))
                            })?;
                        let prefix = format!("xui16_state_{state_id}_");
                        let mut row = self.state_template(exact_state.template)?.clone();
                        bind_state_semantics(&mut row, state);
                        if exact_state.template == OccupationUiStateTemplateExact::Resisting60 {
                            attach_resistance_status(
                                registry,
                                &mut row,
                                &self.resistance_status,
                                &self.resistance_modifier,
                                "resistance_status_pos",
                                exact_state.resistance_status.as_ref().ok_or_else(|| {
                                    OccupationPixelRejection::IncompleteCarrier(format!(
                                        "XUI16 state {state_id} lacks exact resistance status"
                                    ))
                                })?,
                                &prefix,
                                "resistance_status_",
                                &mut options,
                            )?;
                            attach_compliance_status(
                                registry,
                                &mut row,
                                &self.compliance_status,
                                "compliance_status_pos",
                                exact_state.compliance_status.as_ref().ok_or_else(|| {
                                    OccupationPixelRejection::IncompleteCarrier(format!(
                                        "XUI16 state {state_id} lacks exact compliance status"
                                    ))
                                })?,
                                &prefix,
                                "compliance_status_",
                                &mut options,
                            )?;
                        }
                        set_position_upper_left(
                            &mut row,
                            clip.x() - root_rect.x(),
                            clip.y() - root_rect.y() + row_y,
                        );
                        bind_state_presentation(
                            registry,
                            &row,
                            &prefix,
                            exact_state,
                            &mut options,
                        )?;
                        prefix_element_names(&mut row, &prefix);
                        dynamic.children.push(row);
                        materialized.push(format!("state:{state_id}"));
                    }
                    state_y = state_y.checked_add(state_h).ok_or_else(|| {
                        OccupationPixelRejection::AuthoredLayout(
                            "XUI16 state materialization height overflow".into(),
                        )
                    })?;
                }
            }
            content_y = content_y.checked_add(group_h).ok_or_else(|| {
                OccupationPixelRejection::AuthoredLayout("XUI16 content height overflow".into())
            })?;
        }
        if content_y + territory_scroll.pixels != territory_content_height {
            return Err(OccupationPixelRejection::AuthoredLayout(
                "XUI16 territory geometry changed between scroll and materialization passes".into(),
            ));
        }
        let (image, report) =
            compose_clipped_pair(resolver, registry, cache, &base, &dynamic, &options, clip)?;
        complete(&report)?;
        Ok(OccupationPixelOutput {
            image,
            report,
            authored_root: COUNTRY_ROOT.into(),
            source_record_id: surface.source_record_id().into(),
            materialized_row_ids: materialized,
            applied_scroll_row: territory_scroll.row,
            input_region: OccupationInputRegion {
                panel: OccupationPanel::Territories,
                panel_rect: [root_rect.x(), root_rect.y(), root_rect.w(), root_rect.h()],
                scroll_rect: [clip.x(), clip.y(), clip.w(), clip.h()],
            },
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn compose_garrison(
        &self,
        resolver: &AssetResolver,
        registry: &SpriteRegistry,
        localisation: &Localisation,
        cache: &mut UiAssetCache,
        surface: &AdmittedOccupationSurface,
        exact: AdmittedOccupationPixels<'_>,
        viewer: &OccupationViewerState,
        viewport_width: usize,
        viewport_height: usize,
    ) -> std::result::Result<OccupationPixelOutput, OccupationPixelRejection> {
        let month = exact
            .presentation
            .month_projections
            .iter()
            .find(|month| month.months == viewer.garrison_months)
            .ok_or_else(|| {
                OccupationPixelRejection::IncompleteCarrier(format!(
                    "XUI16 has no exact {}-month projection",
                    viewer.garrison_months
                ))
            })?;
        let mut base = self.garrison_root.clone();
        let mut dynamic = self.garrison_root.clone();
        disambiguate_garrison_empty_roles(&mut base)?;
        disambiguate_garrison_empty_roles(&mut dynamic)?;
        if let Some([x, y]) = viewer.garrison_drag_position {
            set_position_upper_left(&mut base, x, y);
            set_position_upper_left(&mut dynamic, x, y);
        }
        let mut options =
            ComposeOptions::new(viewport_width, viewport_height).with_localisation(localisation);
        let active_equipment = viewer.panel == OccupationPanel::GarrisonEquipment;
        options.hidden_elements.insert(format!(
            "xui16_garrison_{}",
            if active_equipment {
                "resistance_activities_tab"
            } else {
                "equipment_tab"
            }
        ));
        bind_garrison_root(
            registry,
            &dynamic,
            &month,
            exact.controls,
            active_equipment,
            "xui16_garrison_",
            &mut options,
        )?;
        let root_rect = resolve_element_rect(
            &dynamic,
            Rect::new(0, 0, viewport_width as i32, viewport_height as i32),
            (0, 0),
            &options,
        );
        let equipment_order = month
            .equipment_losses
            .iter()
            .map(|loss| format!("equipment:{}:{}", loss.equipment_id, loss.equipment_type))
            .collect::<Vec<_>>();
        let equipment_plan = if active_equipment {
            Some(plan_garrison_equipment_collection(
                &mut base,
                &mut dynamic,
                root_rect,
                authored_extent(&self.equipment_row, "equipment row")?,
                equipment_order.len(),
                viewer.equipment_scroll_row,
                &options,
            )?)
        } else {
            None
        };
        let (clip, ordered, pitch, scroll, template) = if active_equipment {
            (
                equipment_plan
                    .as_ref()
                    .expect("active equipment resolves its exact collection plan")
                    .clip,
                equipment_order.as_slice(),
                authored_extent(&self.equipment_row, "equipment row")?.1,
                equipment_plan
                    .as_ref()
                    .expect("active equipment resolves its exact collection plan")
                    .scroll,
                &self.equipment_row,
            )
        } else {
            let activity_clip = descendant_rect(
                &dynamic,
                root_rect,
                &["resistance_activities_tab", "activities_list"],
                &options,
            )?;
            let activity_pitch = authored_slot_height(&dynamic, "activities_list")?;
            (
                activity_clip,
                month.ordered_activity_ids.as_slice(),
                activity_pitch,
                authored_row_scroll_offset(
                    viewer.activity_scroll_row,
                    month.ordered_activity_ids.len(),
                    activity_pitch,
                    activity_clip.h(),
                )?,
                &self.activity_row,
            )
        };
        prefix_element_names(&mut base, "xui16_garrison_");
        prefix_element_names(&mut dynamic, "xui16_garrison_");
        let activities = surface
            .activities()
            .iter()
            .map(|row| (row.stable_id.as_str(), row))
            .collect::<BTreeMap<_, _>>();
        let equipment = surface
            .equipment_losses()
            .iter()
            .map(|row| (row.stable_id.as_str(), row))
            .collect::<BTreeMap<_, _>>();
        let mut materialized = Vec::new();
        for (ordinal, stable_id) in ordered.iter().enumerate() {
            let row_rect = match equipment_plan.as_ref() {
                Some(plan) => match plan.visible_row_rect(ordinal) {
                    Some(rect) => rect,
                    None => continue,
                },
                None => {
                    let y = -scroll.pixels
                        + pitch
                            * i32::try_from(ordinal).map_err(|_| {
                                OccupationPixelRejection::AuthoredLayout(
                                    "XUI16 row ordinal exceeds i32".into(),
                                )
                            })?;
                    Rect::new(clip.x(), clip.y() + y, clip.w(), pitch)
                }
            };
            if !intersects_vertical(row_rect.y(), row_rect.h(), clip.y(), clip.h()) {
                continue;
            }
            let prefix = format!("xui16_log_{ordinal}_");
            let mut row = template.clone();
            if active_equipment {
                let semantic = equipment.get(stable_id.as_str()).ok_or_else(|| {
                    OccupationPixelRejection::SourceJoin(format!(
                        "XUI16 month projection names absent equipment row {stable_id}"
                    ))
                })?;
                bind_equipment_semantics(&mut row, semantic);
                let presentation = exact
                    .presentation
                    .equipment_presentations
                    .iter()
                    .find(|row| {
                        format!("equipment:{}:{}", row.equipment_id, row.equipment_type)
                            == *stable_id
                    })
                    .ok_or_else(|| {
                        OccupationPixelRejection::IncompleteCarrier(format!(
                            "XUI16 equipment row {stable_id} has no exact presentation"
                        ))
                    })?;
                let amount_lost_text = month
                    .equipment_losses
                    .iter()
                    .find(|row| {
                        format!("equipment:{}:{}", row.equipment_id, row.equipment_type)
                            == *stable_id
                    })
                    .map(|row| row.amount_lost_text.as_str())
                    .ok_or_else(|| {
                        OccupationPixelRejection::IncompleteCarrier(format!(
                            "XUI16 equipment row {stable_id} has no month aggregate text"
                        ))
                    })?;
                bind_equipment_presentation(
                    registry,
                    &row,
                    &prefix,
                    presentation,
                    amount_lost_text,
                    &mut options,
                )?;
            } else {
                let semantic = activities.get(stable_id.as_str()).ok_or_else(|| {
                    OccupationPixelRejection::SourceJoin(format!(
                        "XUI16 month projection names absent activity row {stable_id}"
                    ))
                })?;
                bind_activity_semantics(&mut row, semantic);
                let presentation = exact
                    .presentation
                    .activity_presentations
                    .get(stable_id)
                    .ok_or_else(|| {
                        OccupationPixelRejection::IncompleteCarrier(format!(
                            "XUI16 activity row {stable_id} has no exact presentation"
                        ))
                    })?;
                bind_activity_presentation(resolver, &row, &prefix, presentation, &mut options)?;
            }
            set_position_upper_left(
                &mut row,
                row_rect.x() - root_rect.x(),
                row_rect.y() - root_rect.y(),
            );
            prefix_element_names(&mut row, &prefix);
            dynamic.children.push(row);
            materialized.push(stable_id.clone());
        }
        let content_height = pitch
            .checked_mul(i32::try_from(ordered.len()).map_err(|_| {
                OccupationPixelRejection::AuthoredLayout(
                    "XUI16 materialized row count exceeds i32".into(),
                )
            })?)
            .ok_or_else(|| {
                OccupationPixelRejection::AuthoredLayout("XUI16 log height overflow".into())
            })?;
        validate_scroll(
            scroll.pixels,
            content_height,
            clip.h(),
            if active_equipment {
                "equipment"
            } else {
                "activity"
            },
        )?;
        let (image, report) =
            compose_clipped_pair(resolver, registry, cache, &base, &dynamic, &options, clip)?;
        complete(&report)?;
        Ok(OccupationPixelOutput {
            image,
            report,
            authored_root: GARRISON_ROOT.into(),
            source_record_id: surface.source_record_id().into(),
            materialized_row_ids: materialized,
            applied_scroll_row: scroll.row,
            input_region: equipment_plan.as_ref().map_or_else(
                || OccupationInputRegion {
                    panel: viewer.panel,
                    panel_rect: [root_rect.x(), root_rect.y(), root_rect.w(), root_rect.h()],
                    scroll_rect: [clip.x(), clip.y(), clip.w(), clip.h()],
                },
                |plan| plan.input_region(root_rect),
            ),
        })
    }

    fn state_template(
        &self,
        template: OccupationUiStateTemplateExact,
    ) -> std::result::Result<&GuiElement, OccupationPixelRejection> {
        match template {
            OccupationUiStateTemplateExact::Resisting60 => Ok(&self.state_row),
            OccupationUiStateTemplateExact::NoResistance30 => Ok(&self.state_row_no_resistance),
        }
    }
}

fn admit_exact_pixels<'a>(
    frame: &'a WorldFrame,
    surface: &AdmittedOccupationSurface,
    viewer: &OccupationViewerState,
) -> std::result::Result<AdmittedOccupationPixels<'a>, OccupationPixelRejection> {
    if !surface.is_product_authority() {
        return Err(OccupationPixelRejection::SourceJoin(
            "XUI16 pixel compositor rejects fixture-only occupation surfaces".into(),
        ));
    }
    if viewer.selected_country.trim().is_empty()
        || !(1..=12).contains(&viewer.garrison_months)
        || viewer.territory_sort_mode > 4
    {
        return Err(OccupationPixelRejection::ViewerInput(
            "XUI16 viewer country/month/sort is invalid".into(),
        ));
    }
    // A STORED SELECTION outlives the frame that vouched for it: this hour's
    // census decides whether it names a country at all.
    if frame.named_country(&viewer.selected_country).is_none() {
        return Err(OccupationPixelRejection::ViewerInput(
            "XUI16 viewer country is a key two D18 ids reach, which names no country".into(),
        ));
    }

    let view = frame.occupation_ui_supplement_exact();
    if view.status != ViewStatus::Present {
        return Err(OccupationPixelRejection::IncompleteCarrier(format!(
            "XUI16 exact supplement status is {:?}",
            view.status
        )));
    }
    let supplement = view.supplement.as_ref().ok_or_else(|| {
        OccupationPixelRejection::IncompleteCarrier(
            "XUI16 Present supplement has no payload".into(),
        )
    })?;
    if !supplement.complete || supplement.source_record_id != surface.source_record_id() {
        return Err(OccupationPixelRejection::SourceJoin(
            "XUI16 exact supplement is incomplete or differs from the admitted semantic surface"
                .into(),
        ));
    }
    let frame_hour = u64::try_from(frame.hour_index()).map_err(|_| {
        OccupationPixelRejection::SourceJoin("XUI16 WorldFrame hour is negative".into())
    })?;
    if supplement.frame_hour != frame_hour {
        return Err(OccupationPixelRejection::SourceJoin(
            "XUI16 exact supplement hour differs from the current WorldFrame".into(),
        ));
    }
    let controller = supplement
        .controllers
        .get(&viewer.selected_country)
        .ok_or_else(|| {
            OccupationPixelRejection::SourceJoin(format!(
                "XUI16 supplement has no viewer-selected controller {}",
                viewer.selected_country
            ))
        })?;
    if controller.controller_country_tag != viewer.selected_country
        || controller.controller_country_tag_db_id == 0
    {
        return Err(OccupationPixelRejection::SourceJoin(
            "XUI16 selected controller identity is invalid".into(),
        ));
    }

    let semantic_states = surface
        .states()
        .iter()
        .map(|row| (row.state_id, row))
        .collect::<BTreeMap<_, _>>();
    if semantic_states.len() != surface.states().len()
        || semantic_states
            .values()
            .any(|row| row.occupier_tag != viewer.selected_country)
        || controller
            .presentation
            .state_presentations
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != semantic_states.keys().copied().collect::<BTreeSet<_>>()
    {
        return Err(OccupationPixelRejection::SourceJoin(
            "XUI16 exact state presentation does not cover the admitted selected-controller states"
                .into(),
        ));
    }
    for (state_id, exact) in &controller.presentation.state_presentations {
        let semantic = semantic_states.get(state_id).ok_or_else(|| {
            OccupationPixelRejection::SourceJoin(format!(
                "XUI16 exact state {state_id} is absent from the admitted surface"
            ))
        })?;
        let expected = if semantic.resistance == 0.0 {
            OccupationUiStateTemplateExact::NoResistance30
        } else {
            OccupationUiStateTemplateExact::Resisting60
        };
        if exact.template != expected {
            return Err(OccupationPixelRejection::SourceJoin(format!(
                "XUI16 state {state_id} template disagrees with the admitted resistance split"
            )));
        }
    }

    let activity_ids = surface
        .activities()
        .iter()
        .map(|row| row.stable_id.as_str())
        .collect::<BTreeSet<_>>();
    if activity_ids.len() != surface.activities().len()
        || controller
            .presentation
            .activity_presentations
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != activity_ids
        || surface
            .activities()
            .iter()
            .any(|row| row.controller_tag != viewer.selected_country)
    {
        return Err(OccupationPixelRejection::SourceJoin(
            "XUI16 exact activity presentation does not cover the admitted selected controller"
                .into(),
        ));
    }

    let semantic_equipment = surface
        .equipment_losses()
        .iter()
        .map(|row| (row.equipment_id, row.equipment_type))
        .collect::<BTreeSet<_>>();
    let exact_equipment = controller
        .presentation
        .equipment_presentations
        .iter()
        .map(|row| (row.equipment_id, row.equipment_type))
        .collect::<BTreeSet<_>>();
    if semantic_equipment.len() != surface.equipment_losses().len()
        || exact_equipment.len() != controller.presentation.equipment_presentations.len()
        || semantic_equipment != exact_equipment
    {
        return Err(OccupationPixelRejection::SourceJoin(
            "XUI16 exact equipment presentation does not cover the admitted loss rows".into(),
        ));
    }

    Ok(AdmittedOccupationPixels {
        presentation: &controller.presentation,
        controls: &supplement.control_frame_law,
    })
}

fn bounded_text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 16_384
}

fn bind_state_semantics(root: &mut GuiElement, state: &OccupationStatePresentation) {
    set_authored_text(root, "state_name", &state.text.state_name);
}

fn bind_activity_semantics(root: &mut GuiElement, row: &GarrisonActivityRow) {
    set_authored_text(root, "location", &row.location_text);
    set_authored_text(root, "date", &row.date_text);
}

fn bind_equipment_semantics(root: &mut GuiElement, row: &GarrisonEquipmentLossRow) {
    set_authored_text(root, "eq_name", &row.display_name);
}

fn set_authored_text(root: &mut GuiElement, name: &str, text: &str) {
    if let Some(element) = find_named_mut(root, name) {
        element.text = Some(text.to_owned());
    }
}

fn attach_resistance_status(
    registry: &SpriteRegistry,
    row: &mut GuiElement,
    template: &GuiElement,
    modifier_template: &GuiElement,
    position_name: &str,
    presentation: &OccupationUiStatusWidgetExact,
    row_prefix: &str,
    status_prefix: &str,
    options: &mut ComposeOptions,
) -> std::result::Result<(), OccupationPixelRejection> {
    if !bounded_text(&presentation.progress_text) || presentation.modifier_icons.len() > 4_096 {
        return Err(OccupationPixelRejection::IncompleteCarrier(format!(
            "XUI16 {position_name} resistance status is empty or exceeds bounds"
        )));
    }
    let anchor = find_named(row, position_name).ok_or_else(|| {
        OccupationPixelRejection::AuthoredLayout(format!(
            "XUI16 state row lacks authored {position_name}"
        ))
    })?;
    let mut status = template.clone();
    status.position = anchor.position.clone();
    let target_position_y = named_position_y(&status, "resistance_target")?;
    let target = find_named_mut(&mut status, "resistance_target").ok_or_else(|| {
        OccupationPixelRejection::AuthoredLayout(
            "XUI16 resistance status lacks authored resistance_target".into(),
        )
    })?;
    let target_authority = presentation.target.as_ref().ok_or_else(|| {
        OccupationPixelRejection::IncompleteCarrier(
            "XUI16 resistance status lacks an exact authored target coordinate".into(),
        )
    })?;
    set_position_upper_left(target, target_authority.authored_local_x, target_position_y);
    let status_extent = authored_extent(&status, "resistance status")?;
    let modifier_container =
        find_named_mut(&mut status, "modifier_container").ok_or_else(|| {
            OccupationPixelRejection::AuthoredLayout(
                "XUI16 resistance status lacks authored modifier_container".into(),
            )
        })?;
    let modifier_layout = gridbox_layout(
        modifier_container,
        Rect::new(0, 0, status_extent.0, status_extent.1),
        options,
    )
    .map_err(OccupationPixelRejection::from)?;
    for (ordinal, modifier) in presentation.modifier_icons.iter().enumerate() {
        let mut icon = modifier_template.clone();
        let slot = modifier_layout
            .slot_rect(ordinal)
            .map_err(OccupationPixelRejection::from)?;
        set_position_upper_left(
            &mut icon,
            slot.x() - modifier_layout.rect.x(),
            slot.y() - modifier_layout.rect.y(),
        );
        let modifier_prefix = format!("modifier_{ordinal}_");
        exact_sprite_frame(
            registry,
            options,
            &format!("{row_prefix}{status_prefix}{modifier_prefix}"),
            "icon",
            &modifier.icon_sprite,
            modifier.frame,
        )?;
        prefix_element_names(&mut icon, &modifier_prefix);
        modifier_container.children.push(icon);
    }
    authored_frame(
        &status,
        registry,
        options,
        &format!("{row_prefix}{status_prefix}"),
        "progress",
        presentation.progress_frame,
        true,
    )?;
    exact_text(
        options,
        &format!("{row_prefix}{status_prefix}"),
        "progress_text",
        &presentation.progress_text,
    )?;
    prefix_element_names(&mut status, status_prefix);
    row.children.push(status);
    Ok(())
}

fn attach_compliance_status(
    registry: &SpriteRegistry,
    row: &mut GuiElement,
    template: &GuiElement,
    position_name: &str,
    presentation: &OccupationUiStatusWidgetExact,
    row_prefix: &str,
    status_prefix: &str,
    options: &mut ComposeOptions,
) -> std::result::Result<(), OccupationPixelRejection> {
    if !bounded_text(&presentation.progress_text) {
        return Err(OccupationPixelRejection::IncompleteCarrier(format!(
            "XUI16 {position_name} compliance status text is empty or exceeds bounds"
        )));
    }
    let anchor = find_named(row, position_name).ok_or_else(|| {
        OccupationPixelRejection::AuthoredLayout(format!(
            "XUI16 state row lacks authored {position_name}"
        ))
    })?;
    let mut status = template.clone();
    status.position = anchor.position.clone();
    authored_frame(
        &status,
        registry,
        options,
        &format!("{row_prefix}{status_prefix}"),
        "progress",
        presentation.progress_frame,
        true,
    )?;
    exact_text(
        options,
        &format!("{row_prefix}{status_prefix}"),
        "progress_text",
        &presentation.progress_text,
    )?;
    prefix_element_names(&mut status, status_prefix);
    row.children.push(status);
    Ok(())
}

fn require_roles(
    root: &GuiElement,
    roles: &[&str],
) -> std::result::Result<(), OccupationPixelRejection> {
    for role in roles {
        if find_named(root, role).is_none() {
            return Err(OccupationPixelRejection::DynamicBinding(format!(
                "XUI16 authored template lacks required role {role:?}"
            )));
        }
    }
    Ok(())
}

fn exact_text(
    options: &mut ComposeOptions,
    prefix: &str,
    role: &str,
    value: &str,
) -> std::result::Result<(), OccupationPixelRejection> {
    if value.trim().is_empty() {
        return Err(OccupationPixelRejection::DynamicBinding(format!(
            "XUI16 exact text for {role:?} is empty"
        )));
    }
    options
        .text_overrides
        .insert(format!("{prefix}{role}"), value.to_owned());
    Ok(())
}

fn exact_texture(
    resolver: &AssetResolver,
    options: &mut ComposeOptions,
    prefix: &str,
    role: &str,
    texture: &str,
) -> std::result::Result<(), OccupationPixelRejection> {
    if texture.trim().is_empty() {
        return Err(OccupationPixelRejection::DynamicBinding(format!(
            "XUI16 exact texture for {role:?} is empty"
        )));
    }
    let decoded = resolver.decode_texture(texture).map_err(|error| {
        OccupationPixelRejection::DynamicBinding(format!(
            "XUI16 exact texture {texture:?} for {role:?} did not decode: {error}"
        ))
    })?;
    options
        .sprite_overrides
        .insert(format!("{prefix}{role}"), decoded.image);
    Ok(())
}

fn exact_sprite(
    registry: &SpriteRegistry,
    options: &mut ComposeOptions,
    prefix: &str,
    role: &str,
    sprite: &str,
) -> std::result::Result<(), OccupationPixelRejection> {
    if sprite.trim().is_empty() || registry.get(sprite).is_none() {
        return Err(OccupationPixelRejection::DynamicBinding(format!(
            "XUI16 exact sprite {sprite:?} for {role:?} is absent from the admitted effective VFS"
        )));
    }
    options
        .sprite_name_overrides
        .insert(format!("{prefix}{role}"), sprite.to_owned());
    Ok(())
}

fn authored_frame(
    root: &GuiElement,
    registry: &SpriteRegistry,
    options: &mut ComposeOptions,
    prefix: &str,
    role: &str,
    frame: u32,
    one_based: bool,
) -> std::result::Result<(), OccupationPixelRejection> {
    let element = find_named(root, role).ok_or_else(|| {
        OccupationPixelRejection::DynamicBinding(format!(
            "XUI16 authored template lacks framed role {role:?}"
        ))
    })?;
    let sprite = element
        .sprite_refs
        .first()
        .map(String::as_str)
        .ok_or_else(|| {
            OccupationPixelRejection::DynamicBinding(format!(
                "XUI16 authored role {role:?} lacks a sprite"
            ))
        })?;
    let definition = registry.get(sprite).ok_or_else(|| {
        OccupationPixelRejection::DynamicBinding(format!(
            "XUI16 authored sprite {sprite:?} for {role:?} is absent from the admitted effective VFS"
        ))
    })?;
    let index = if one_based {
        frame.checked_sub(1).ok_or_else(|| {
            OccupationPixelRejection::IncompleteCarrier(format!(
                "XUI16 one-based frame for {role:?} is zero"
            ))
        })?
    } else {
        frame
    };
    if index >= definition.no_of_frames.max(1) {
        return Err(OccupationPixelRejection::DynamicBinding(format!(
            "XUI16 frame {frame} for {role:?} exceeds admitted sprite {sprite:?} frame_count {}",
            definition.no_of_frames.max(1)
        )));
    }
    options.frame_overrides.insert(
        format!("{prefix}{role}"),
        usize::try_from(index).map_err(|_| {
            OccupationPixelRejection::IncompleteCarrier(format!(
                "XUI16 frame for {role:?} exceeds usize"
            ))
        })?,
    );
    Ok(())
}

fn exact_sprite_frame(
    registry: &SpriteRegistry,
    options: &mut ComposeOptions,
    prefix: &str,
    role: &str,
    sprite: &str,
    one_based_frame: u32,
) -> std::result::Result<(), OccupationPixelRejection> {
    exact_sprite(registry, options, prefix, role, sprite)?;
    let definition = registry.get(sprite).ok_or_else(|| {
        OccupationPixelRejection::DynamicBinding(format!(
            "XUI16 exact sprite {sprite:?} for {role:?} is absent"
        ))
    })?;
    let index = one_based_frame.checked_sub(1).ok_or_else(|| {
        OccupationPixelRejection::IncompleteCarrier(format!(
            "XUI16 one-based exact sprite frame for {role:?} is zero"
        ))
    })?;
    if index >= definition.no_of_frames.max(1) {
        return Err(OccupationPixelRejection::DynamicBinding(format!(
            "XUI16 frame {one_based_frame} for dynamic sprite {sprite:?} exceeds frame_count {}",
            definition.no_of_frames.max(1)
        )));
    }
    options.frame_overrides.insert(
        format!("{prefix}{role}"),
        usize::try_from(index).map_err(|_| {
            OccupationPixelRejection::IncompleteCarrier(format!(
                "XUI16 dynamic frame for {role:?} exceeds usize"
            ))
        })?,
    );
    Ok(())
}

fn visibility(options: &mut ComposeOptions, prefix: &str, role: &str, visible: bool) {
    if !visible {
        options.hidden_elements.insert(format!("{prefix}{role}"));
    }
}

fn bind_country_presentation(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    root: &GuiElement,
    prefix: &str,
    presentation: &OccupationUiCountryGroupPresentationExact,
    expanded: bool,
    options: &mut ComposeOptions,
) -> std::result::Result<(), OccupationPixelRejection> {
    const ROLES: [&str; 15] = [
        "flag",
        "country_name",
        "occupied_state_count",
        "manpower",
        "industrial_capacity",
        "resources",
        "selected_law",
        "select_law_icon",
        "garrison_template_icon",
        "template_need_text",
        "bar_str",
        "release_button",
        "return_territory_button",
        "invalid_default_law",
        "state_info_container",
    ];
    require_roles(root, &ROLES)?;
    exact_texture(
        resolver,
        options,
        prefix,
        "flag",
        &presentation.flag_texture,
    )?;
    for (role, value) in [
        ("country_name", presentation.country_name.as_str()),
        (
            "occupied_state_count",
            presentation.occupied_state_count_text.as_str(),
        ),
        ("manpower", presentation.manpower_text.as_str()),
        (
            "industrial_capacity",
            presentation.industrial_capacity_text.as_str(),
        ),
        ("resources", presentation.resources_text.as_str()),
        ("selected_law", presentation.selected_law_text.as_str()),
        (
            "template_need_text",
            presentation.garrison_template_need_text.as_str(),
        ),
    ] {
        exact_text(options, prefix, role, value)?;
    }
    exact_sprite(
        registry,
        options,
        prefix,
        "garrison_template_icon",
        &presentation.garrison_template_icon_sprite,
    )?;
    authored_frame(
        root,
        registry,
        options,
        prefix,
        "select_law_icon",
        presentation.selected_law_icon_frame,
        true,
    )?;
    authored_frame(
        root,
        registry,
        options,
        prefix,
        "bar_str",
        presentation.garrison_strength_frame,
        true,
    )?;
    for (role, visible, enabled) in [
        (
            "release_button",
            presentation.release_visible,
            presentation.release_enabled,
        ),
        (
            "return_territory_button",
            presentation.return_territory_visible,
            presentation.return_territory_enabled,
        ),
    ] {
        if visible && !enabled {
            authored_frame(root, registry, options, prefix, role, 4, true)?;
        }
    }
    for (role, shown) in [
        ("release_button", presentation.release_visible),
        (
            "return_territory_button",
            presentation.return_territory_visible,
        ),
        (
            "invalid_default_law",
            presentation.invalid_default_law_visible,
        ),
        ("state_info_container", expanded),
    ] {
        visibility(options, prefix, role, shown);
    }
    Ok(())
}

fn bind_country_root(
    registry: &SpriteRegistry,
    root: &GuiElement,
    prefix: &str,
    presentation: &replay_core::OccupationUiControllerRootPresentationExact,
    controls: &OccupationUiControlFrameLawExact,
    viewer: &OccupationViewerState,
    options: &mut ComposeOptions,
) -> std::result::Result<(), OccupationPixelRejection> {
    require_roles(
        root,
        &[
            "garrison_template_icon",
            "template_need_text",
            "selected_law",
            "select_law_icon",
            "invalid_default_law",
            "foreign_support",
            "foreign_support_amount",
            "resistance_tab_button",
            "compliance_tab_button",
            "show_non_resisting_container_checkbox",
            "sort_button_0",
            "sort_button_1",
            "sort_button_2",
            "sort_button_3",
            "sort_button_4",
        ],
    )?;
    exact_sprite(
        registry,
        options,
        prefix,
        "garrison_template_icon",
        &presentation.default_garrison_template_icon_sprite,
    )?;
    exact_text(
        options,
        prefix,
        "template_need_text",
        &presentation.default_garrison_template_need_text,
    )?;
    exact_text(
        options,
        prefix,
        "selected_law",
        &presentation.selected_default_law_text,
    )?;
    authored_frame(
        root,
        registry,
        options,
        prefix,
        "select_law_icon",
        presentation.selected_default_law_icon_frame,
        true,
    )?;
    visibility(
        options,
        prefix,
        "invalid_default_law",
        presentation.invalid_default_law_visible,
    );
    visibility(
        options,
        prefix,
        "foreign_support",
        presentation.foreign_support_visible,
    );
    if presentation.foreign_support_visible {
        exact_text(
            options,
            prefix,
            "foreign_support_amount",
            &presentation.foreign_support_amount_text,
        )?;
    }
    let sort_frames = controls
        .territory_sort_button_frames
        .get(&viewer.territory_sort_mode)
        .ok_or_else(|| {
            OccupationPixelRejection::IncompleteCarrier(format!(
                "XUI16 control law lacks sort-mode {} frames",
                viewer.territory_sort_mode
            ))
        })?;
    for (button, frame) in sort_frames.iter().enumerate() {
        authored_frame(
            root,
            registry,
            options,
            prefix,
            &format!("sort_button_{button}"),
            *frame,
            false,
        )?;
    }
    authored_frame(
        root,
        registry,
        options,
        prefix,
        "show_non_resisting_container_checkbox",
        controls.show_non_resisting_checkbox_frames[usize::from(viewer.show_non_resisting)],
        false,
    )?;
    let metric_index = match viewer.territory_metric {
        OccupationTerritoryMetric::Resistance => 0,
        OccupationTerritoryMetric::Compliance => 1,
    };
    for (role, frame) in [
        (
            "resistance_tab_button",
            controls.territory_metric_button_frames[metric_index][0],
        ),
        (
            "compliance_tab_button",
            controls.territory_metric_button_frames[metric_index][1],
        ),
    ] {
        authored_frame(root, registry, options, prefix, role, frame, false)?;
    }
    Ok(())
}

fn bind_state_presentation(
    registry: &SpriteRegistry,
    root: &GuiElement,
    prefix: &str,
    presentation: &OccupationUiStatePresentationExact,
    options: &mut ComposeOptions,
) -> std::result::Result<(), OccupationPixelRejection> {
    match presentation.template {
        OccupationUiStateTemplateExact::NoResistance30 => {
            if presentation.garrison_template_icon_sprite.is_some()
                || presentation.garrison_template_need_text.is_some()
                || presentation.garrison_strength_frame.is_some()
                || presentation.selected_law_icon_frame.is_some()
            {
                return Err(OccupationPixelRejection::SourceJoin(
                    "XUI16 no-resistance state carries resisting-row chrome".into(),
                ));
            }
        }
        OccupationUiStateTemplateExact::Resisting60 => {
            require_roles(
                root,
                &[
                    "garrison_template_icon",
                    "template_need_text",
                    "bar_str",
                    "select_law_icon",
                ],
            )?;
            let icon = presentation
                .garrison_template_icon_sprite
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    OccupationPixelRejection::IncompleteCarrier(
                        "XUI16 resisting state lacks garrison icon".into(),
                    )
                })?;
            let need = presentation
                .garrison_template_need_text
                .as_deref()
                .ok_or_else(|| {
                    OccupationPixelRejection::IncompleteCarrier(
                        "XUI16 resisting state lacks garrison need text".into(),
                    )
                })?;
            exact_sprite(registry, options, prefix, "garrison_template_icon", icon)?;
            exact_text(options, prefix, "template_need_text", need)?;
            authored_frame(
                root,
                registry,
                options,
                prefix,
                "bar_str",
                presentation.garrison_strength_frame.ok_or_else(|| {
                    OccupationPixelRejection::IncompleteCarrier(
                        "XUI16 resisting state lacks garrison strength frame".into(),
                    )
                })?,
                true,
            )?;
            authored_frame(
                root,
                registry,
                options,
                prefix,
                "select_law_icon",
                presentation.selected_law_icon_frame.ok_or_else(|| {
                    OccupationPixelRejection::IncompleteCarrier(
                        "XUI16 resisting state lacks occupation-law icon frame".into(),
                    )
                })?,
                true,
            )?;
        }
    }
    Ok(())
}

fn bind_garrison_root(
    registry: &SpriteRegistry,
    root: &GuiElement,
    month: &OccupationUiMonthProjectionExact,
    controls: &OccupationUiControlFrameLawExact,
    active_equipment: bool,
    prefix: &str,
    options: &mut ComposeOptions,
) -> std::result::Result<(), OccupationPixelRejection> {
    require_roles(
        root,
        &[
            "title_losses_in_manpower",
            "title_losses_in_equipment",
            "total_equipment_lost",
            "resistance_activities_tab_button",
            "equipment_tab_button",
            "activity_empty_msg",
            "equipment_empty_msg",
        ],
    )?;
    exact_text(
        options,
        prefix,
        "title_losses_in_manpower",
        &month.total_manpower_lost_text,
    )?;
    exact_text(
        options,
        prefix,
        "title_losses_in_equipment",
        &month.total_equipment_lost_text,
    )?;
    exact_text(
        options,
        prefix,
        "total_equipment_lost",
        &month.equipment_tab_total_text,
    )?;
    exact_text(
        options,
        prefix,
        "activity_empty_msg",
        &month.empty_activity_text,
    )?;
    exact_text(
        options,
        prefix,
        "equipment_empty_msg",
        &month.empty_equipment_text,
    )?;
    let tab_index = usize::from(active_equipment);
    for (role, frame) in [
        (
            "resistance_activities_tab_button",
            controls.garrison_tab_button_frames[tab_index][0],
        ),
        (
            "equipment_tab_button",
            controls.garrison_tab_button_frames[tab_index][1],
        ),
    ] {
        authored_frame(root, registry, options, prefix, role, frame, false)?;
    }
    visibility(
        options,
        prefix,
        "activity_empty_msg",
        !active_equipment && month.ordered_activity_ids.is_empty(),
    );
    visibility(
        options,
        prefix,
        "equipment_empty_msg",
        active_equipment && month.equipment_losses.is_empty(),
    );
    Ok(())
}

fn bind_activity_presentation(
    resolver: &AssetResolver,
    root: &GuiElement,
    prefix: &str,
    presentation: &OccupationUiActivityPresentationExact,
    options: &mut ComposeOptions,
) -> std::result::Result<(), OccupationPixelRejection> {
    require_roles(
        root,
        &[
            "attack_blocked",
            "garrison_losses",
            "lost_manpower",
            "lost_equipment",
            "flag",
        ],
    )?;
    exact_text(
        options,
        prefix,
        "attack_blocked",
        &presentation.attack_result_text,
    )?;
    exact_text(
        options,
        prefix,
        "lost_manpower",
        &presentation.lost_manpower_text,
    )?;
    exact_text(
        options,
        prefix,
        "lost_equipment",
        &presentation.lost_equipment_text,
    )?;
    exact_texture(
        resolver,
        options,
        prefix,
        "flag",
        &presentation.occupied_country_flag_texture,
    )?;
    visibility(
        options,
        prefix,
        "garrison_losses",
        presentation.garrison_losses_visible,
    );
    visibility(
        options,
        prefix,
        "attack_blocked",
        presentation.attack_result_visible,
    );
    Ok(())
}

fn bind_equipment_presentation(
    registry: &SpriteRegistry,
    root: &GuiElement,
    prefix: &str,
    presentation: &OccupationUiEquipmentPresentationExact,
    amount_lost_text: &str,
    options: &mut ComposeOptions,
) -> std::result::Result<(), OccupationPixelRejection> {
    require_roles(root, &["num_loss_in_combat", "equipment_icon"])?;
    exact_text(options, prefix, "num_loss_in_combat", amount_lost_text)?;
    exact_sprite(
        registry,
        options,
        prefix,
        "equipment_icon",
        &presentation.equipment_icon_sprite,
    )
}

fn descendant_rect(
    root: &GuiElement,
    root_rect: Rect,
    path: &[&str],
    options: &ComposeOptions,
) -> std::result::Result<Rect, OccupationPixelRejection> {
    descendant_element_with_parent_rect(root, root_rect, path, options)
        .map(|(_, rect, _)| rect)
}

fn descendant_element_with_parent_rect<'a>(
    root: &'a GuiElement,
    root_rect: Rect,
    path: &[&str],
    options: &ComposeOptions,
) -> std::result::Result<(&'a GuiElement, Rect, Rect), OccupationPixelRejection> {
    if path.is_empty() {
        return Err(OccupationPixelRejection::AuthoredLayout(
            "XUI16 authored descendant path is empty".into(),
        ));
    }
    let mut element = root;
    let mut rect = root_rect;
    let mut parent_rect = root_rect;
    for name in path {
        parent_rect = rect;
        element = element
            .children
            .iter()
            .find(|child| child.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                OccupationPixelRejection::AuthoredLayout(format!(
                    "XUI16 authored path lacks {name:?}"
                ))
            })?;
        rect = resolve_element_rect(element, rect, (0, 0), options);
    }
    if rect.w() <= 0 || rect.h() <= 0 {
        return Err(OccupationPixelRejection::AuthoredLayout(
            "XUI16 authored clip has non-positive extent".into(),
        ));
    }
    if parent_rect.w() <= 0 || parent_rect.h() <= 0 {
        return Err(OccupationPixelRejection::AuthoredLayout(
            "XUI16 authored collection parent has non-positive extent".into(),
        ));
    }
    Ok((element, rect, parent_rect))
}

fn overwrite_descendant_height(
    root: &mut GuiElement,
    path: &[&str],
    height: i32,
) -> std::result::Result<(), OccupationPixelRejection> {
    if height <= 0 {
        return Err(OccupationPixelRejection::AuthoredLayout(
            "XUI16 controller collection height is not positive".into(),
        ));
    }
    let mut element = root;
    for name in path {
        element = element
            .children
            .iter_mut()
            .find(|child| child.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                OccupationPixelRejection::AuthoredLayout(format!(
                    "XUI16 authored path lacks {name:?}"
                ))
            })?;
    }
    let size = element.size.as_mut().ok_or_else(|| {
        OccupationPixelRejection::AuthoredLayout(
            "XUI16 controller collection has no authored size".into(),
        )
    })?;
    size.y = parse_measure(&height.to_string());
    Ok(())
}

fn plan_garrison_equipment_collection(
    base: &mut GuiElement,
    dynamic: &mut GuiElement,
    root_rect: Rect,
    row_extent: (i32, i32),
    row_count: usize,
    requested_scroll_row: usize,
    options: &ComposeOptions,
) -> std::result::Result<GarrisonEquipmentCollectionPlan, OccupationPixelRejection> {
    let (_, _, parent_rect) = descendant_element_with_parent_rect(
        dynamic,
        root_rect,
        &EQUIPMENT_COLLECTION_PATH,
        options,
    )?;
    overwrite_descendant_height(base, &EQUIPMENT_COLLECTION_PATH, parent_rect.h())?;
    overwrite_descendant_height(dynamic, &EQUIPMENT_COLLECTION_PATH, parent_rect.h())?;
    let (collection, clip, _) = descendant_element_with_parent_rect(
        dynamic,
        root_rect,
        &EQUIPMENT_COLLECTION_PATH,
        options,
    )?;
    let scroll = authored_row_scroll_offset(
        requested_scroll_row,
        row_count,
        row_extent.1,
        clip.h(),
    )?;
    let sizes = vec![row_extent; row_count];
    let row_rects = plan_authenticated_smooth_list_rects(
        collection,
        clip,
        &sizes,
        scroll.pixels,
        options,
    )?;
    Ok(GarrisonEquipmentCollectionPlan {
        clip,
        scroll,
        row_rects,
    })
}

fn disambiguate_garrison_empty_roles(
    root: &mut GuiElement,
) -> std::result::Result<(), OccupationPixelRejection> {
    rename_descendant_role(
        root,
        &["resistance_activities_tab", "empty_msg"],
        "activity_empty_msg",
    )?;
    rename_descendant_role(
        root,
        &["equipment_tab", "subwindow", "empty_msg"],
        "equipment_empty_msg",
    )
}

fn rename_descendant_role(
    root: &mut GuiElement,
    path: &[&str],
    replacement: &str,
) -> std::result::Result<(), OccupationPixelRejection> {
    let mut element = root;
    for name in path {
        element = element
            .children
            .iter_mut()
            .find(|child| child.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                OccupationPixelRejection::AuthoredLayout(format!(
                    "XUI16 authored path lacks {name:?}"
                ))
            })?;
    }
    element.name = replacement.to_owned();
    Ok(())
}

fn authored_extent(
    element: &GuiElement,
    label: &str,
) -> std::result::Result<(i32, i32), OccupationPixelRejection> {
    let size = element.size.as_ref().ok_or_else(|| {
        OccupationPixelRejection::AuthoredLayout(format!("{label} has no authored size"))
    })?;
    Ok((
        parse_authored_fixed_pixels(&size.x.raw).ok_or_else(|| {
            OccupationPixelRejection::AuthoredLayout(format!("{label} width is not fixed positive"))
        })?,
        parse_authored_fixed_pixels(&size.y.raw).ok_or_else(|| {
            OccupationPixelRejection::AuthoredLayout(format!(
                "{label} height is not fixed positive"
            ))
        })?,
    ))
}

fn parse_authored_fixed_pixels(raw: &str) -> Option<i32> {
    raw.trim().parse::<i32>().ok().filter(|value| *value > 0)
}

fn authored_slot_height(
    root: &GuiElement,
    name: &str,
) -> std::result::Result<i32, OccupationPixelRejection> {
    let row = find_named(root, name).ok_or_else(|| {
        OccupationPixelRejection::AuthoredLayout(format!(
            "XUI16 authored collection {name:?} is absent"
        ))
    })?;
    let slot = row
        .raw
        .first_value_ci("slotsize")
        .and_then(crate::gui::parse_measure_pair)
        .ok_or_else(|| {
            OccupationPixelRejection::AuthoredLayout(format!(
                "XUI16 authored collection {name:?} lacks slotsize"
            ))
        })?;
    slot.y
        .raw
        .parse::<i32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            OccupationPixelRejection::AuthoredLayout(format!(
                "XUI16 authored collection {name:?} slot height is not positive fixed pixels"
            ))
        })
}

fn named_position_y(
    root: &GuiElement,
    name: &str,
) -> std::result::Result<i32, OccupationPixelRejection> {
    find_named(root, name)
        .and_then(|row| row.position.as_ref())
        .and_then(|position| position.y.raw.parse::<i32>().ok())
        .ok_or_else(|| {
            OccupationPixelRejection::AuthoredLayout(format!(
                "XUI16 authored position {name:?} has no fixed y"
            ))
        })
}

fn find_named<'a>(root: &'a GuiElement, name: &str) -> Option<&'a GuiElement> {
    if root.name.eq_ignore_ascii_case(name) {
        return Some(root);
    }
    root.children
        .iter()
        .find_map(|child| find_named(child, name))
}

fn find_named_mut<'a>(root: &'a mut GuiElement, name: &str) -> Option<&'a mut GuiElement> {
    if root.name.eq_ignore_ascii_case(name) {
        return Some(root);
    }
    root.children
        .iter_mut()
        .find_map(|child| find_named_mut(child, name))
}

fn authored_row_scroll_offset(
    requested_row: usize,
    row_count: usize,
    pitch: i32,
    viewport_height: i32,
) -> std::result::Result<AppliedScroll, OccupationPixelRejection> {
    if pitch <= 0 || viewport_height <= 0 {
        return Err(OccupationPixelRejection::AuthoredLayout(
            "XUI16 authored scroll geometry is not positive".into(),
        ));
    }
    let row_count_i32 = i32::try_from(row_count).map_err(|_| {
        OccupationPixelRejection::AuthoredLayout(
            "XUI16 authored scroll row count exceeds i32".into(),
        )
    })?;
    let content_height = pitch.checked_mul(row_count_i32).ok_or_else(|| {
        OccupationPixelRejection::AuthoredLayout(
            "XUI16 authored scroll content height overflow".into(),
        )
    })?;
    let max_pixels = content_height.saturating_sub(viewport_height).max(0);
    let max_row =
        usize::try_from((i64::from(max_pixels) + i64::from(pitch) - 1) / i64::from(pitch))
            .unwrap_or(usize::MAX)
            .min(row_count.saturating_sub(1));
    let applied_row = requested_row.min(max_row);
    let requested = pitch
        .checked_mul(i32::try_from(applied_row).map_err(|_| {
            OccupationPixelRejection::AuthoredLayout(
                "XUI16 authored scroll ordinal exceeds i32".into(),
            )
        })?)
        .ok_or_else(|| {
            OccupationPixelRejection::AuthoredLayout("XUI16 authored scroll offset overflow".into())
        })?;
    Ok(AppliedScroll {
        row: applied_row,
        pixels: requested.min(max_pixels),
    })
}

fn authored_variable_row_scroll_offset(
    requested_row: usize,
    row_offsets: &[i32],
    content_height: i32,
    viewport_height: i32,
) -> std::result::Result<AppliedScroll, OccupationPixelRejection> {
    if content_height < 0
        || viewport_height <= 0
        || row_offsets.iter().any(|offset| *offset < 0)
        || row_offsets.windows(2).any(|pair| pair[0] > pair[1])
    {
        return Err(OccupationPixelRejection::AuthoredLayout(
            "XUI16 authored variable-row scroll geometry is invalid".into(),
        ));
    }
    let max_pixels = content_height.saturating_sub(viewport_height).max(0);
    let max_row = row_offsets
        .iter()
        .position(|offset| *offset >= max_pixels)
        .unwrap_or_else(|| row_offsets.len().saturating_sub(1));
    let applied_row = requested_row.min(max_row);
    let requested = row_offsets.get(applied_row).copied().unwrap_or(0);
    Ok(AppliedScroll {
        row: applied_row,
        pixels: requested.min(max_pixels),
    })
}

fn validate_scroll(
    scroll: i32,
    content_height: i32,
    viewport_height: i32,
    label: &str,
) -> std::result::Result<(), OccupationPixelRejection> {
    let max = content_height.saturating_sub(viewport_height).max(0);
    if scroll < 0 || scroll > max {
        return Err(OccupationPixelRejection::ViewerInput(format!(
            "XUI16 {label} scroll {scroll} is outside authored content range 0..={max}"
        )));
    }
    Ok(())
}

fn intersects_vertical(y: i32, h: i32, top: i32, clip_h: i32) -> bool {
    i64::from(y) < i64::from(top) + i64::from(clip_h)
        && i64::from(y) + i64::from(h) > i64::from(top)
}

fn compose_clipped_pair(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    cache: &mut UiAssetCache,
    base: &GuiElement,
    dynamic: &GuiElement,
    options: &ComposeOptions,
    clip: Rect,
) -> std::result::Result<(RgbaImage, crate::compose::ComposeReport), OccupationPixelRejection> {
    let ComposeOutput {
        mut image,
        report: _,
        ..
    } = Compositor::with_cache(resolver, registry, cache).compose(base, options);
    let ComposeOutput {
        image: dynamic_image,
        report,
        ..
    } = Compositor::with_cache(resolver, registry, cache).compose(dynamic, options);
    copy_rect(&mut image, &dynamic_image, clip)?;
    Ok((image, report))
}

fn copy_rect(
    destination: &mut RgbaImage,
    source: &RgbaImage,
    rect: Rect,
) -> std::result::Result<(), OccupationPixelRejection> {
    if destination.width != source.width || destination.height != source.height {
        return Err(OccupationPixelRejection::IncompleteComposition(
            "XUI16 clipping passes differ in image extent".into(),
        ));
    }
    let left = rect.x().max(0) as usize;
    let top = rect.y().max(0) as usize;
    let right =
        (i64::from(rect.x()) + i64::from(rect.w())).clamp(0, destination.width as i64) as usize;
    let bottom =
        (i64::from(rect.y()) + i64::from(rect.h())).clamp(0, destination.height as i64) as usize;
    for y in top..bottom {
        let start = (y * destination.width + left) * 4;
        let end = (y * destination.width + right) * 4;
        destination.pixels[start..end].copy_from_slice(&source.pixels[start..end]);
    }
    Ok(())
}

fn complete(
    report: &crate::compose::ComposeReport,
) -> std::result::Result<(), OccupationPixelRejection> {
    if !report.missing_sprites.is_empty()
        || !report.undecodable_sprites.is_empty()
        || !report.missing_fonts.is_empty()
        || !report.missing_glyphs.is_empty()
        || !report.unresolved_text_inputs.is_empty()
    {
        return Err(OccupationPixelRejection::IncompleteComposition(format!(
            "XUI16 composition is incomplete: missing_sprites={:?}; undecodable={:?}; \
             missing_fonts={:?}; missing_glyphs={:?}; unresolved_text={:?}",
            report.missing_sprites,
            report.undecodable_sprites,
            report.missing_fonts,
            report.missing_glyphs,
            report.unresolved_text_inputs
        )));
    }
    Ok(())
}

impl From<GameUiError> for OccupationPixelRejection {
    fn from(value: GameUiError) -> Self {
        Self::AuthoredLayout(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::parse_gui_document;
    use crate::paradox_text::parse_str;

    fn parsed_root(source: &str) -> GuiElement {
        let document = parse_str(&format!("guiTypes = {{ {source} }}"));
        parse_gui_document(&document)
            .into_iter()
            .next()
            .expect("one GUI root")
    }

    #[test]
    fn viewer_month_range_is_exactly_the_authored_slider_range() {
        let state = OccupationViewerState::default();
        assert_eq!(state.garrison_months, 12);
        assert!((1..=12).contains(&state.garrison_months));
    }

    #[test]
    fn garrison_equipment_template_is_the_exact_constructor_branch() {
        assert_eq!(EQUIPMENT_ROW, "combat_log_enemy_loss_item");
    }

    #[test]
    fn clipping_intersection_keeps_partial_rows() {
        assert!(intersects_vertical(-10, 45, 0, 100));
        assert!(intersects_vertical(99, 45, 0, 100));
        assert!(!intersects_vertical(100, 45, 0, 100));
        assert!(!intersects_vertical(-45, 45, 0, 100));
    }

    #[test]
    fn authored_extent_refuses_percent_fraction_and_expression_substitutes() {
        assert_eq!(parse_authored_fixed_pixels("60"), Some(60));
        assert_eq!(parse_authored_fixed_pixels(" 30 "), Some(30));
        assert_eq!(parse_authored_fixed_pixels("100%"), None);
        assert_eq!(parse_authored_fixed_pixels("1.5"), None);
        assert_eq!(parse_authored_fixed_pixels("gui_width - 4"), None);
        assert_eq!(parse_authored_fixed_pixels("0"), None);
        assert_eq!(parse_authored_fixed_pixels("-1"), None);
    }

    #[test]
    fn authored_scroll_reports_the_stable_bottom_clamp_row() {
        let fixed =
            authored_row_scroll_offset(usize::MAX, 10, 45, 100).expect("fixed authored geometry");
        assert_eq!(
            fixed,
            AppliedScroll {
                row: 8,
                pixels: 350
            }
        );

        let variable =
            authored_variable_row_scroll_offset(usize::MAX, &[0, 60, 120, 180], 240, 100)
                .expect("variable authored geometry");
        assert_eq!(
            variable,
            AppliedScroll {
                row: 3,
                pixels: 140
            }
        );
    }

    #[test]
    fn garrison_equipment_controller_override_drives_rows_clip_and_input_region() {
        let mut root = parsed_root(
            r#"containerWindowType = {
                name = "garrison_log_view"
                size = { x = 600 y = 400 }
                gridBoxType = {
                    name = "equipments_list"
                    size = { x = 20 y = 2 }
                    slotsize = { x = 20 y = 2 }
                }
                containerWindowType = {
                    name = "equipment_tab"
                    position = { x = 0 y = 50 }
                    size = { x = 600 y = 300 }
                    containerWindowType = {
                        name = "subwindow"
                        position = { x = 10 y = 60 }
                        size = { x = 580 y = 100 }
                        smoothListBoxType = {
                            name = "equipments_list"
                            position = { x = 0 y = 0 }
                            size = { x = 580 y = 1 }
                            scrollbarType = "standardlistbox_slider"
                        }
                    }
                }
            }"#,
        );
        let options = ComposeOptions::new(800, 600);
        let root_rect = resolve_element_rect(
            &root,
            Rect::new(0, 0, 800, 600),
            (0, 0),
            &options,
        );
        let (_, authored, parent) = descendant_element_with_parent_rect(
            &root,
            root_rect,
            &EQUIPMENT_COLLECTION_PATH,
            &options,
        )
        .expect("exact equipment path");
        assert_eq!(authored.h(), 1);
        assert_eq!(parent.h(), 100);

        let mut base = root.clone();
        let plan = plan_garrison_equipment_collection(
            &mut base,
            &mut root,
            root_rect,
            (580, 45),
            3,
            0,
            &options,
        )
        .expect("shipping garrison equipment collection plan");
        let (collection, clip, resolved_parent) = descendant_element_with_parent_rect(
            &root,
            root_rect,
            &EQUIPMENT_COLLECTION_PATH,
            &options,
        )
        .expect("controller-resolved exact equipment path");
        assert_eq!(collection.element_type.to_ascii_lowercase(), "smoothlistboxtype");
        assert_eq!(clip.h(), resolved_parent.h());
        assert_eq!(clip, Rect::new(10, 110, 580, 100));
        assert_eq!(plan.clip, clip);
        let materialized = (0..3)
            .filter_map(|ordinal| plan.visible_row_rect(ordinal))
            .count();
        assert_eq!(materialized, 3, "the third row is partially visible");

        let input = plan.input_region(root_rect);
        assert!(input.contains_scroll_point(clip.x(), clip.y() + 99));
        assert!(!input.contains_scroll_point(clip.x(), clip.y() + 100));
    }
}
