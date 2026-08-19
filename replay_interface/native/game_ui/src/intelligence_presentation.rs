
use crate::assets::AssetResolver;
use crate::compose::{resolve_element_rect, ComposeOptions, ComposeOutput, Compositor, Rect};
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, LayoutSet};
use crate::gui_scene_util::{
    load_named_element, pair, prefix_element_names, promote_show_position, set_position_upper_left,
};
use crate::politics_state::PoliticsTooltipPayloads as IntelligenceTooltipPayloads;
use crate::tooltip_compositor::{
    AdmittedTooltipSource, RowAvailability, ScreenRect, TooltipLineKind, TooltipOwner,
    TooltipPayload, TooltipRow,
};
use crate::widget_layout::{gridbox_layout, GridBoxLayout};
use crate::{GameUiError, Result};
use replay_core::{
    IntelligenceCountryPresentationExact, IntelligencePresentationExact, IntelligenceTooltipExact,
    ViewStatus, WorldFrame,
};
use std::collections::{BTreeMap, BTreeSet};

pub const AGENCY_GUI: &str = "interface/countryintelligenceagencyview.gui";
pub const OPERATIVE_GUI: &str = "interface/operativeleader.gui";
pub const LEDGER_GUI: &str = "interface/countryintelledger.gui";
pub const OPERATION_GUI: &str = "interface/operationoverview.gui";
pub const AGENCY_ROOT: &str = "countryintelligenceagencyview";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IntelligenceViewerSurface {
    #[default]
    Agency,
    Operations,
    Cryptology,
    IntelLedger,
    OperationOverview,
}

impl IntelligenceViewerSurface {
    pub fn next(self) -> Self {
        match self {
            Self::Agency => Self::Operations,
            Self::Operations => Self::Cryptology,
            Self::Cryptology => Self::IntelLedger,
            Self::IntelLedger => Self::OperationOverview,
            Self::OperationOverview => Self::Agency,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntelligenceViewerState {
    pub panel_open: bool,
    pub country_tag: Option<String>,
    pub surface: IntelligenceViewerSurface,
    pub row_scroll: usize,
}

impl IntelligenceViewerState {
    pub fn toggle(&mut self, selected_country: Option<&str>) {
        let Some(tag) = selected_country.filter(|tag| !tag.trim().is_empty()) else {
            self.close();
            return;
        };
        if self.panel_open && self.country_tag.as_deref() == Some(tag) {
            self.close();
        } else {
            *self = Self {
                panel_open: true,
                country_tag: Some(tag.to_string()),
                ..Self::default()
            };
        }
    }

    pub fn reconcile_country(&mut self, selected_country: Option<&str>) {
        let Some(tag) = selected_country.filter(|tag| !tag.trim().is_empty()) else {
            self.close();
            return;
        };
        if self.panel_open && self.country_tag.as_deref() != Some(tag) {
            *self = Self {
                panel_open: true,
                country_tag: Some(tag.to_string()),
                ..Self::default()
            };
        }
    }

    pub fn close(&mut self) {
        *self = Self::default();
    }

    pub fn select_surface(&mut self, surface: IntelligenceViewerSurface) {
        self.surface = surface;
        self.row_scroll = 0;
    }

    pub fn wheel(&mut self, signed_units: i32, row_count: usize) {
        if signed_units > 0 {
            self.row_scroll = self.row_scroll.saturating_sub(signed_units as usize);
        } else if signed_units < 0 {
            self.row_scroll = self
                .row_scroll
                .saturating_add(signed_units.unsigned_abs() as usize)
                .min(row_count.saturating_sub(1));
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntelligenceAdmissionError {
    CarrierUnavailable(ViewStatus),
    ContentManifestMismatch,
    CountryUnavailable,
    ViewerMismatch,
}

pub fn admit_intelligence_presentation<'a>(
    frame: &'a WorldFrame,
    selected_country: &str,
    viewer: &IntelligenceViewerState,
    expected_content_manifest_sha256: &str,
) -> std::result::Result<
    (
        &'a IntelligencePresentationExact,
        &'a IntelligenceCountryPresentationExact,
    ),
    IntelligenceAdmissionError,
> {
    let view = frame.intelligence_presentation_exact();
    let carrier = view
        .presentation
        .as_ref()
        .ok_or(IntelligenceAdmissionError::CarrierUnavailable(view.status))?;
    if carrier.content_manifest_sha256 != expected_content_manifest_sha256 {
        return Err(IntelligenceAdmissionError::ContentManifestMismatch);
    }
    if !viewer.panel_open || viewer.country_tag.as_deref() != Some(selected_country) {
        return Err(IntelligenceAdmissionError::ViewerMismatch);
    }
    let row = carrier
        .rows
        .iter()
        .find(|row| row.country_tag == selected_country)
        .ok_or(IntelligenceAdmissionError::CountryUnavailable)?;
    Ok((carrier, row))
}

#[derive(Clone, Debug)]
pub struct IntelligenceAuthoredLayout {
    agency_root: GuiElement,
    ledger_root: GuiElement,
    operation_root: GuiElement,
    upgrade_template: GuiElement,
    operative_window: GuiElement,
    operative_template: GuiElement,
    operation_template: GuiElement,
    operation_operative_template: GuiElement,
    country_template: GuiElement,
    upgrade_grid: GridBoxLayout,
    operation_grid: GridBoxLayout,
    country_grid: GridBoxLayout,
    operative_grid: GridBoxLayout,
    overview_operative_grid: GridBoxLayout,
    agency_panel_rect: Rect,
    agency_close_rect: Rect,
    agency_operations_tab_rect: Rect,
    agency_crypto_tab_rect: Rect,
    ledger_panel_rect: Rect,
    operation_panel_rect: Rect,
    operation_close_rect: Rect,
}

impl IntelligenceAuthoredLayout {
    pub fn load(resolver: &AssetResolver, width: usize, height: usize) -> Result<Self> {
        let mut cache = BTreeMap::<String, LayoutSet>::new();
        let mut agency_root = load_named_element(&mut cache, resolver, AGENCY_GUI, AGENCY_ROOT)?;
        promote_show_position(&mut agency_root);
        let mut ledger_root = load_named_element(
            &mut cache,
            resolver,
            LEDGER_GUI,
            "country_intel_ledger_view",
        )?;
        promote_show_position(&mut ledger_root);
        let mut operation_root = load_named_element(
            &mut cache,
            resolver,
            OPERATION_GUI,
            "operation_overview_window",
        )?;
        promote_show_position(&mut operation_root);
        let upgrade_template =
            load_named_element(&mut cache, resolver, AGENCY_GUI, "upgrade_button")?;
        let operative_window = load_named_element(
            &mut cache,
            resolver,
            OPERATIVE_GUI,
            "operative_leader_window",
        )?;
        let operative_template = load_named_element(
            &mut cache,
            resolver,
            OPERATIVE_GUI,
            "operative_leader_portrait_window",
        )?;
        let operation_template =
            load_named_element(&mut cache, resolver, AGENCY_GUI, "operation_view_entry")?;
        let operation_operative_template = load_named_element(
            &mut cache,
            resolver,
            OPERATION_GUI,
            "operation_operative_leader_portrait_window",
        )?;
        let country_template =
            load_named_element(&mut cache, resolver, AGENCY_GUI, "cryptology_country_entry")?;
        let options = ComposeOptions::new(width, height);
        let viewport = Rect::new(0, 0, width as i32, height as i32);
        let agency_panel_rect =
            resolve_element_rect(&agency_root, viewport, (550, height), &options);
        let (upgrade, upgrade_parent) = descendant_with_parent_rect(
            &agency_root,
            "upgrades_list",
            agency_panel_rect,
            &options,
        )?;
        let (operation, operation_parent) = descendant_with_parent_rect(
            &agency_root,
            "operations_grid",
            agency_panel_rect,
            &options,
        )?;
        let (country, country_parent) =
            descendant_with_parent_rect(&agency_root, "country_list", agency_panel_rect, &options)?;
        let operative_rect = resolve_element_rect(&operative_window, viewport, (0, 0), &options);
        let (operative, operative_parent) = descendant_with_parent_rect(
            &operative_window,
            "operatives_box",
            operative_rect,
            &options,
        )?;
        let agency_close_rect =
            element_rect(&agency_root, "close_button", agency_panel_rect, &options)?;
        let agency_operations_tab_rect = element_rect(
            &agency_root,
            "operations_tab_button",
            agency_panel_rect,
            &options,
        )?;
        let agency_crypto_tab_rect = element_rect(
            &agency_root,
            "crypto_tab_button",
            agency_panel_rect,
            &options,
        )?;
        let upgrade_grid = gridbox_layout(upgrade, upgrade_parent, &options)?;
        let operation_grid = gridbox_layout(operation, operation_parent, &options)?;
        let country_grid = gridbox_layout(country, country_parent, &options)?;
        let operative_grid = gridbox_layout(operative, operative_parent, &options)?;
        let ledger_panel_rect = resolve_element_rect(&ledger_root, viewport, (0, 0), &options);
        let operation_panel_rect =
            resolve_element_rect(&operation_root, viewport, (0, 0), &options);
        let operation_close_rect =
            element_rect(&operation_root, "btn_close", operation_panel_rect, &options)?;
        let (overview_operatives, overview_parent) = descendant_with_parent_rect(
            &operation_root,
            "operatives_grid",
            operation_panel_rect,
            &options,
        )?;
        let overview_operative_grid =
            gridbox_layout(overview_operatives, overview_parent, &options)?;
        Ok(Self {
            agency_root,
            ledger_root,
            operation_root,
            upgrade_template,
            operative_window,
            operative_template,
            operation_template,
            operation_operative_template,
            country_template,
            upgrade_grid,
            operation_grid,
            country_grid,
            operative_grid,
            overview_operative_grid,
            agency_panel_rect,
            agency_close_rect,
            agency_operations_tab_rect,
            agency_crypto_tab_rect,
            ledger_panel_rect,
            operation_panel_rect,
            operation_close_rect,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntelligenceInputRegion {
    pub panel: [i32; 4],
    pub close: Option<[i32; 4]>,
    pub operations_tab: Option<[i32; 4]>,
    pub crypto_tab: Option<[i32; 4]>,
    pub row_count: usize,
}

impl IntelligenceInputRegion {
    pub fn panel_contains(self, x: i32, y: i32) -> bool {
        contains(self.panel, x, y)
    }
    pub fn close_contains(self, x: i32, y: i32) -> bool {
        self.close.is_some_and(|rect| contains(rect, x, y))
    }
    pub fn operations_contains(self, x: i32, y: i32) -> bool {
        self.operations_tab.is_some_and(|rect| contains(rect, x, y))
    }
    pub fn crypto_contains(self, x: i32, y: i32) -> bool {
        self.crypto_tab.is_some_and(|rect| contains(rect, x, y))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelligenceTooltipHitTarget {
    pub rect: ScreenRect,
    pub clip_rect: ScreenRect,
    pub payloads: IntelligenceTooltipPayloads,
}

pub fn intelligence_tooltip_payloads_at(
    targets: &[IntelligenceTooltipHitTarget],
    point: (i32, i32),
) -> Option<&IntelligenceTooltipPayloads> {
    targets.iter().rev().find_map(|target| {
        target
            .rect
            .intersection(target.clip_rect)
            .filter(|rect| rect.contains_half_open(point))
            .map(|_| &target.payloads)
    })
}

#[derive(Clone, Debug)]
pub struct IntelligenceComposeOutput {
    pub output: ComposeOutput,
    pub authored_root: String,
    pub input_region: IntelligenceInputRegion,
    pub displayed_row_ids: Vec<String>,
    pub tooltip_targets: Vec<IntelligenceTooltipHitTarget>,
}

pub fn render_intelligence(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    presentation: &IntelligenceCountryPresentationExact,
    producer_manifest_sha256: &str,
    viewer: &IntelligenceViewerState,
    width: usize,
    height: usize,
) -> Result<IntelligenceComposeOutput> {
    let layout = IntelligenceAuthoredLayout::load(resolver, width, height)?;
    let mut options = ComposeOptions::new(width, height);
    let (
        root,
        authored_root,
        panel,
        close,
        operations_tab,
        crypto_tab,
        displayed_row_ids,
        row_count,
    ) = match viewer.surface {
        IntelligenceViewerSurface::Agency
        | IntelligenceViewerSurface::Operations
        | IntelligenceViewerSurface::Cryptology => {
            let mut root = layout.agency_root.clone();
            configure_agency_chrome(&mut options, registry, presentation)?;
            let (displayed_row_ids, row_count) = match viewer.surface {
                IntelligenceViewerSurface::Agency => {
                    options.hidden_elements.insert("agency_operations".into());
                    options.hidden_elements.insert("agency_crypto".into());
                    append_upgrades(&mut root, &layout, registry, presentation, &mut options)?;
                    append_operatives(&mut root, &layout, registry, presentation, &mut options)?;
                    let mut ids = presentation
                        .upgrades
                        .iter()
                        .map(|row| row.stable_item_id.clone())
                        .collect::<Vec<_>>();
                    ids.extend(
                        presentation
                            .operatives
                            .iter()
                            .map(|row| row.stable_item_id.clone()),
                    );
                    (ids, 0)
                }
                IntelligenceViewerSurface::Operations => {
                    options.hidden_elements.insert("agency_branches".into());
                    options.hidden_elements.insert("agency_agents".into());
                    options.hidden_elements.insert("agency_crypto".into());
                    append_operations(&mut root, &layout, registry, presentation, &mut options)?;
                    (
                        presentation
                            .operations
                            .iter()
                            .map(|row| row.stable_item_id.clone())
                            .collect(),
                        0,
                    )
                }
                IntelligenceViewerSurface::Cryptology => {
                    options.hidden_elements.insert("agency_branches".into());
                    options.hidden_elements.insert("agency_agents".into());
                    options.hidden_elements.insert("agency_operations".into());
                    let ids = append_cipher_rows(
                        &mut root,
                        &layout,
                        registry,
                        presentation,
                        viewer,
                        &mut options,
                    )?;
                    (ids, presentation.ciphers.len())
                }
                _ => unreachable!("agency surface match is exhaustive"),
            };
            (
                root,
                AGENCY_ROOT,
                layout.agency_panel_rect,
                Some(layout.agency_close_rect),
                Some(layout.agency_operations_tab_rect),
                Some(layout.agency_crypto_tab_rect),
                displayed_row_ids,
                row_count,
            )
        }
        IntelligenceViewerSurface::IntelLedger => {
            let (root, ids) = build_ledger_surface(&layout, presentation, viewer, &mut options)?;
            (
                root,
                "country_intel_ledger_view",
                layout.ledger_panel_rect,
                None,
                None,
                None,
                ids,
                presentation.intel_levels.len(),
            )
        }
        IntelligenceViewerSurface::OperationOverview => {
            let (root, ids) = build_operation_overview_surface(
                &layout,
                registry,
                presentation,
                viewer,
                &mut options,
            )?;
            (
                root,
                "operation_overview_window",
                layout.operation_panel_rect,
                Some(layout.operation_close_rect),
                None,
                None,
                ids,
                presentation.operations.len(),
            )
        }
    };
    let wrapper = wrapper_root(width, height, root)?;
    let output = Compositor::new(resolver, registry).compose(&wrapper, &options);
    let tooltip_targets = intelligence_tooltip_targets(
        resolver,
        &output,
        presentation,
        producer_manifest_sha256,
        viewer,
    )?;
    Ok(IntelligenceComposeOutput {
        output,
        authored_root: authored_root.into(),
        input_region: IntelligenceInputRegion {
            panel: rect_array(panel),
            close: close.map(rect_array),
            operations_tab: operations_tab.map(rect_array),
            crypto_tab: crypto_tab.map(rect_array),
            row_count,
        },
        displayed_row_ids,
        tooltip_targets,
    })
}

fn configure_agency_chrome(
    options: &mut ComposeOptions,
    registry: &SpriteRegistry,
    presentation: &IntelligenceCountryPresentationExact,
) -> Result<()> {
    let agency = &presentation.agency;
    require_sprite(registry, &agency.icon_sprite)?;
    set_text(options, "agency_name", &agency.name.final_text);
    set_text(
        options,
        "agency_defense_level",
        &agency.defense_text.final_text,
    );
    set_text(
        options,
        "agency_crypto_defense_level",
        &agency.defense_text.final_text,
    );
    set_text(
        options,
        "total_operative_text",
        &agency.capacity_text.final_text,
    );
    options
        .sprite_name_overrides
        .insert("agency_logo".into(), agency.icon_sprite.clone());
    options.progress_overrides.insert(
        "progressbar_content".into(),
        agency.upgrade_progress_raw as f32 / agency.upgrade_progress_denominator_raw as f32,
    );
    for name in [
        "spymaster",
        "ai_factory_allocation",
        "creation_button",
        "cancel_creation_button",
        "cancel_upgrade",
        "add_new_container",
        "activate_button_container",
        "delete_button",
    ] {
        options.hidden_elements.insert(name.into());
    }
    if agency.created {
        options
            .hidden_elements
            .insert("before_creation_panel".into());
        options
            .hidden_elements
            .insert("upgrades_before_agency".into());
        options
            .hidden_elements
            .insert("operatives_before_agency".into());
    } else {
        for name in [
            "agency_branches",
            "agency_agents",
            "agency_operations",
            "agency_crypto",
        ] {
            options.hidden_elements.insert(name.into());
        }
    }
    Ok(())
}

fn append_upgrades(
    root: &mut GuiElement,
    layout: &IntelligenceAuthoredLayout,
    registry: &SpriteRegistry,
    presentation: &IntelligenceCountryPresentationExact,
    options: &mut ComposeOptions,
) -> Result<()> {
    let grid = descendant_mut_exact(root, "upgrades_list")?;
    for (index, row) in presentation.upgrades.iter().enumerate() {
        require_sprite(registry, &row.icon_sprite)?;
        let mut clone = layout.upgrade_template.clone();
        let prefix = format!("xui14_upgrade_{index}_");
        let (x, y) = grid_origin(&layout.upgrade_grid, index)?;
        set_position_upper_left(&mut clone, x, y);
        prefix_element_names(&mut clone, &prefix);
        set_text(
            options,
            &format!("{prefix}agency_branches_title"),
            &row.display.final_text,
        );
        options
            .sprite_name_overrides
            .insert(format!("{prefix}upgrade_icon"), row.icon_sprite.clone());
        grid.children.push(clone);
    }
    Ok(())
}

fn append_operations(
    root: &mut GuiElement,
    layout: &IntelligenceAuthoredLayout,
    registry: &SpriteRegistry,
    presentation: &IntelligenceCountryPresentationExact,
    options: &mut ComposeOptions,
) -> Result<()> {
    let grid = descendant_mut_exact(root, "operations_grid")?;
    for (index, row) in presentation.operations.iter().enumerate() {
        require_sprite(registry, &row.icon_sprite)?;
        let mut clone = layout.operation_template.clone();
        let prefix = format!("xui14_operation_{index}_");
        let (x, y) = grid_origin(&layout.operation_grid, index)?;
        set_position_upper_left(&mut clone, x, y);
        prefix_element_names(&mut clone, &prefix);
        set_text(
            options,
            &format!("{prefix}operation_name_text"),
            &row.display.final_text,
        );
        set_text(
            options,
            &format!("{prefix}operation_duration_text"),
            &row.status_text.final_text,
        );
        set_text(
            options,
            &format!("{prefix}network_strength_text"),
            &row.target_text.final_text,
        );
        options
            .sprite_name_overrides
            .insert(format!("{prefix}operation_icon"), row.icon_sprite.clone());
        options
            .hidden_elements
            .insert(format!("{prefix}container_launch_operation"));
        grid.children.push(clone);
    }
    Ok(())
}

fn append_operatives(
    root: &mut GuiElement,
    layout: &IntelligenceAuthoredLayout,
    registry: &SpriteRegistry,
    presentation: &IntelligenceCountryPresentationExact,
    options: &mut ComposeOptions,
) -> Result<()> {
    let mut window = layout.operative_window.clone();
    prefix_element_names(&mut window, "xui14_operatives_");
    let grid = descendant_mut_exact(&mut window, "xui14_operatives_operatives_box")?;
    for (index, row) in presentation.operatives.iter().enumerate() {
        require_sprite(registry, &row.portrait_sprite)?;
        let mut clone = layout.operative_template.clone();
        let prefix = format!("xui14_operative_{index}_");
        let (x, y) = grid_origin(&layout.operative_grid, index)?;
        set_position_upper_left(&mut clone, x, y);
        prefix_element_names(&mut clone, &prefix);
        options
            .hidden_elements
            .insert(format!("{prefix}operative_skill_level"));
        options
            .sprite_name_overrides
            .insert(format!("{prefix}portrait_btn"), row.portrait_sprite.clone());
        if !row.captured {
            options
                .hidden_elements
                .insert(format!("{prefix}leader_captured"));
        }
        grid.children.push(clone);
    }
    root.children.push(window);
    Ok(())
}

fn append_cipher_rows(
    root: &mut GuiElement,
    layout: &IntelligenceAuthoredLayout,
    registry: &SpriteRegistry,
    presentation: &IntelligenceCountryPresentationExact,
    viewer: &IntelligenceViewerState,
    options: &mut ComposeOptions,
) -> Result<Vec<String>> {
    let grid = descendant_mut_exact(root, "country_list")?;
    let mut ids = Vec::new();
    let count = presentation.ciphers.len();
    for local in 0..count.saturating_sub(viewer.row_scroll) {
        let source = viewer.row_scroll + local;
        let row = &presentation.ciphers[source];
        require_sprite(registry, &row.flag_sprite)?;
        let mut clone = layout.country_template.clone();
        let prefix = format!("xui14_cipher_{source}_");
        let (x, y) = grid_origin(&layout.country_grid, local)?;
        set_position_upper_left(&mut clone, x, y);
        prefix_element_names(&mut clone, &prefix);
        set_text(
            options,
            &format!("{prefix}name"),
            &row.target_text.final_text,
        );
        set_text(
            options,
            &format!("{prefix}cryptology_state"),
            &row.progress_text.final_text,
        );
        options.progress_overrides.insert(
            format!("{prefix}decrytion_bar"),
            row.decryption_points_raw as f32 / row.progress_denominator_raw as f32,
        );
        options
            .sprite_name_overrides
            .insert(format!("{prefix}flag"), row.flag_sprite.clone());
        for action in [
            "start_stop_button",
            "activate_button_container",
            "delete_button",
        ] {
            options.hidden_elements.insert(format!("{prefix}{action}"));
        }
        ids.push(row.stable_item_id.clone());
        grid.children.push(clone);
    }
    Ok(ids)
}

fn build_ledger_surface(
    layout: &IntelligenceAuthoredLayout,
    presentation: &IntelligenceCountryPresentationExact,
    viewer: &IntelligenceViewerState,
    options: &mut ComposeOptions,
) -> Result<(GuiElement, Vec<String>)> {
    let mut root = layout.ledger_root.clone();
    for name in ["economy_panel", "army_panel", "navy_panel", "air_panel"] {
        options.hidden_elements.insert(name.into());
    }
    for (frame, prefix) in [
        ("economy_tab_frame", "xui14_ledger_civilian_"),
        ("army_tab_frame", "xui14_ledger_army_"),
        ("navy_tab_frame", "xui14_ledger_navy_"),
        ("air_tab_frame", "xui14_ledger_air_"),
    ] {
        prefix_named_subtree(&mut root, frame, prefix)?;
        options
            .hidden_elements
            .insert(format!("{prefix}tab_button"));
    }
    let Some(row) = presentation.intel_levels.get(viewer.row_scroll) else {
        options.hidden_elements.insert("country_info".into());
        return Ok((root, Vec::new()));
    };
    set_text(
        options,
        "xui14_ledger_civilian_intel",
        &row.civilian_text.final_text,
    );
    set_text(
        options,
        "xui14_ledger_army_intel",
        &row.army_text.final_text,
    );
    set_text(
        options,
        "xui14_ledger_navy_intel",
        &row.navy_text.final_text,
    );
    set_text(options, "xui14_ledger_air_intel", &row.air_text.final_text);
    Ok((root, vec![row.stable_item_id.clone()]))
}

fn build_operation_overview_surface(
    layout: &IntelligenceAuthoredLayout,
    registry: &SpriteRegistry,
    presentation: &IntelligenceCountryPresentationExact,
    viewer: &IntelligenceViewerState,
    options: &mut ComposeOptions,
) -> Result<(GuiElement, Vec<String>)> {
    let mut root = layout.operation_root.clone();
    for name in [
        "resources_grid_container",
        "outlook_container",
        "phases_container",
        "container_bottom",
        "reward_container",
        "btn_location",
    ] {
        options.hidden_elements.insert(name.into());
    }
    let Some(operation) = presentation.operations.get(viewer.row_scroll) else {
        for name in [
            "container_target_info",
            "duration_container",
            "operatives_container",
        ] {
            options.hidden_elements.insert(name.into());
        }
        return Ok((root, Vec::new()));
    };
    require_sprite(registry, &operation.icon_sprite)?;
    set_text(
        options,
        "operation_name_text",
        &operation.display.final_text,
    );
    options
        .hidden_elements
        .insert("operation_progress_text".into());
    set_text(
        options,
        "operation_duration_text",
        &operation.status_text.final_text,
    );
    set_text(options, "flag_name", &operation.target_text.final_text);
    options
        .sprite_name_overrides
        .insert("operation_icon".into(), operation.icon_sprite.clone());
    options.progress_overrides.insert(
        "progressbar_content".into(),
        operation.progress_raw as f32 / operation.progress_denominator_raw as f32,
    );
    if let Some(flag) = operation
        .target_country
        .as_deref()
        .and_then(|tag| intelligence_target_flag(presentation, tag))
    {
        require_sprite(registry, flag)?;
        options
            .sprite_name_overrides
            .insert("btn_target_flag".into(), flag.to_string());
    } else {
        options.hidden_elements.insert("btn_target_flag".into());
    }
    let grid = descendant_mut_exact(&mut root, "operatives_grid")?;
    for (local, operative_id) in operation.assigned_operative_ids.iter().enumerate() {
        let operative = presentation
            .operatives
            .iter()
            .find(|row| row.operative_id == *operative_id)
            .ok_or_else(|| GameUiError::missing("XUI14 overview operative join changed"))?;
        require_sprite(registry, &operative.portrait_sprite)?;
        let mut clone = layout.operation_operative_template.clone();
        let prefix = format!("xui14_overview_operative_{local}_");
        let (x, y) = grid_origin(&layout.overview_operative_grid, local)?;
        set_position_upper_left(&mut clone, x, y);
        prefix_element_names(&mut clone, &prefix);
        options.sprite_name_overrides.insert(
            format!("{prefix}portrait_btn"),
            operative.portrait_sprite.clone(),
        );
        if !operative.captured {
            options
                .hidden_elements
                .insert(format!("{prefix}leader_captured"));
        }
        grid.children.push(clone);
    }
    Ok((root, vec![operation.stable_item_id.clone()]))
}

fn intelligence_target_flag<'a>(
    presentation: &'a IntelligenceCountryPresentationExact,
    tag: &str,
) -> Option<&'a str> {
    presentation
        .ciphers
        .iter()
        .find(|row| row.target_country == tag)
        .map(|row| row.flag_sprite.as_str())
        .or_else(|| {
            presentation
                .intel_levels
                .iter()
                .find(|row| row.target_country == tag)
                .map(|row| row.flag_sprite.as_str())
        })
        .or_else(|| {
            presentation
                .networks
                .iter()
                .find(|row| row.target_country == tag)
                .map(|row| row.flag_sprite.as_str())
        })
}

fn prefix_named_subtree(root: &mut GuiElement, name: &str, prefix: &str) -> Result<()> {
    let subtree = descendant_mut_exact(root, name)?;
    prefix_element_names(subtree, prefix);
    Ok(())
}

fn intelligence_tooltip_targets(
    resolver: &AssetResolver,
    output: &ComposeOutput,
    presentation: &IntelligenceCountryPresentationExact,
    producer_manifest_sha256: &str,
    viewer: &IntelligenceViewerState,
) -> Result<Vec<IntelligenceTooltipHitTarget>> {
    let tooltip_root_name = crate::tooltip_compositor::authored_tooltip_shell_name(resolver)?;
    let viewport = ScreenRect::new(
        0,
        0,
        i32::try_from(output.image.width)
            .map_err(|_| GameUiError::parse("XUI14 tooltip viewport width overflow"))?,
        i32::try_from(output.image.height)
            .map_err(|_| GameUiError::parse("XUI14 tooltip viewport height overflow"))?,
    );
    let mut targets = Vec::new();
    let mut owner_tokens = BTreeSet::new();
    let mut push = |source_file: &str,
                    source_root: &str,
                    name: &str,
                    stable_id: &str,
                    tooltip: &IntelligenceTooltipExact|
     -> Result<()> {
        let Some(rect) = output
            .debug_rects
            .iter()
            .rev()
            .find(|rect| rect.name == name && rect.w > 0 && rect.h > 0)
        else {
            return Err(GameUiError::missing(format!(
                "XUI14 tooltip owner {name:?} was not composed"
            )));
        };
        let token = format!(
            "xui14:{}:{}:{:?}:{}",
            presentation.effective_root_identity_sha256,
            presentation.country_tag,
            viewer.surface,
            stable_id,
        );
        if !owner_tokens.insert(token.clone()) {
            return Err(GameUiError::missing(
                "XUI14 tooltip owner identity is duplicated",
            ));
        }
        let owner = TooltipOwner {
            source_file: source_file.into(),
            root_window: source_root.into(),
            element_path: rect.child_path.clone(),
            engine_instance_token: token,
        };
        let payloads = intelligence_tooltip_payloads(
            owner,
            &tooltip_root_name,
            tooltip,
            &presentation.source_record_identity_sha256,
            stable_id,
            producer_manifest_sha256,
        )?;
        targets.push(IntelligenceTooltipHitTarget {
            rect: ScreenRect::new(rect.x, rect.y, rect.w, rect.h),
            clip_rect: viewport,
            payloads,
        });
        Ok(())
    };

    match viewer.surface {
        IntelligenceViewerSurface::Agency => {
            let agency = &presentation.agency;
            push(
                AGENCY_GUI,
                AGENCY_ROOT,
                "agency_logo",
                &agency.agency_instance_id,
                &agency.tooltip,
            )?;
            for (index, row) in presentation.upgrades.iter().enumerate() {
                push(
                    AGENCY_GUI,
                    "upgrade_button",
                    &format!("xui14_upgrade_{index}_upgrade_icon"),
                    &row.stable_item_id,
                    &row.tooltip,
                )?;
            }
            for (index, row) in presentation.operatives.iter().enumerate() {
                push(
                    OPERATIVE_GUI,
                    "operative_leader_portrait_window",
                    &format!("xui14_operative_{index}_portrait_btn"),
                    &row.stable_item_id,
                    &row.tooltip,
                )?;
            }
        }
        IntelligenceViewerSurface::Operations => {
            for (index, row) in presentation.operations.iter().enumerate() {
                push(
                    AGENCY_GUI,
                    "operation_view_entry",
                    &format!("xui14_operation_{index}_operation_bg"),
                    &row.stable_item_id,
                    &row.tooltip,
                )?;
            }
        }
        IntelligenceViewerSurface::Cryptology => {
            for (index, row) in presentation
                .ciphers
                .iter()
                .enumerate()
                .skip(viewer.row_scroll)
            {
                push(
                    AGENCY_GUI,
                    "cryptology_country_entry",
                    &format!("xui14_cipher_{index}_bg"),
                    &row.stable_item_id,
                    &row.tooltip,
                )?;
            }
        }
        IntelligenceViewerSurface::IntelLedger => {}
        IntelligenceViewerSurface::OperationOverview => {
            if let Some(row) = presentation.operations.get(viewer.row_scroll) {
                push(
                    OPERATION_GUI,
                    "operation_overview_window",
                    "operation_icon",
                    &row.stable_item_id,
                    &row.tooltip,
                )?;
                for (local, operative_id) in row.assigned_operative_ids.iter().enumerate() {
                    let operative = presentation
                        .operatives
                        .iter()
                        .find(|candidate| candidate.operative_id == *operative_id)
                        .ok_or_else(|| {
                            GameUiError::missing("XUI14 overview tooltip operative join changed")
                        })?;
                    push(
                        OPERATION_GUI,
                        "operation_operative_leader_portrait_window",
                        &format!("xui14_overview_operative_{local}_portrait_btn"),
                        &operative.stable_item_id,
                        &operative.tooltip,
                    )?;
                }
            }
        }
    }
    Ok(targets)
}

fn intelligence_tooltip_payloads(
    owner: TooltipOwner,
    tooltip_root_name: &str,
    tooltip: &IntelligenceTooltipExact,
    source_record_identity: &str,
    stable_item_id: &str,
    producer_manifest_sha256: &str,
) -> Result<IntelligenceTooltipPayloads> {
    let source = |field: &str| AdmittedTooltipSource::PacketSemantic {
        packet_id: "XUI14".into(),
        producer_identity: format!("{source_record_identity}:{stable_item_id}"),
        field: field.into(),
        manifest_sha256: producer_manifest_sha256.into(),
    };
    let mut rows = Vec::with_capacity(1);
    let mut push_row = |field: String, text: String| {
        let ordinal = rows.len() as u32;
        rows.push(TooltipRow {
            source_ordinal: ordinal,
            line_kind: TooltipLineKind::Scripted,
            source_identity: format!("{stable_item_id}/{field}"),
            evaluation_context: format!("xui14-record={source_record_identity}"),
            availability: RowAvailability::Resolved,
            text,
            admitted_sources: vec![source(&field)],
        });
    };
    push_row(
        format!("tooltip.primary:{}", tooltip.primary.authored_identity),
        tooltip.primary.final_text.clone(),
    );
    let primary = TooltipPayload {
        owner: owner.clone(),
        tooltip_root_name: tooltip_root_name.into(),
        rows: rows.clone(),
    };
    primary.validate()?;
    let delayed = if tooltip
        .delayed
        .as_ref()
        .is_some_and(|delayed| !delayed.final_text.is_empty())
    {
        let mut delayed_rows = rows;
        if let Some(delayed) = tooltip
            .delayed
            .as_ref()
            .filter(|delayed| !delayed.final_text.is_empty())
        {
            let field = format!("tooltip.delayed:{}", delayed.authored_identity);
            delayed_rows.push(TooltipRow {
                source_ordinal: delayed_rows.len() as u32,
                line_kind: TooltipLineKind::Scripted,
                source_identity: format!("{stable_item_id}/{field}"),
                evaluation_context: format!("xui14-record={source_record_identity}"),
                availability: RowAvailability::Resolved,
                text: delayed.final_text.clone(),
                admitted_sources: vec![source(&field)],
            });
        }
        let payload = TooltipPayload {
            owner,
            tooltip_root_name: tooltip_root_name.into(),
            rows: delayed_rows,
        };
        payload.validate()?;
        Some(payload)
    } else {
        None
    };
    Ok(IntelligenceTooltipPayloads { primary, delayed })
}

#[cfg(test)]
mod tooltip_tests {
    use super::*;
    use replay_core::IntelligenceTextExact;

    fn text(identity: &str, value: &str) -> IntelligenceTextExact {
        IntelligenceTextExact {
            authored_identity: identity.into(),
            final_text: value.into(),
            final_text_sha256: "0".repeat(64),
        }
    }

    #[test]
    fn xui14_tooltip_payload_retains_only_exact_owner_primary_and_delayed() {
        let owner = TooltipOwner {
            source_file: AGENCY_GUI.into(),
            root_window: AGENCY_ROOT.into(),
            element_path: vec![1, 2],
            engine_instance_token: "xui14:test-owner".into(),
        };
        let payloads = intelligence_tooltip_payloads(
            owner,
            "tooltip",
            &IntelligenceTooltipExact {
                primary: text("operative/primary", "Operative"),
                delayed: Some(text("operative/delayed", "Exact delayed details")),
            },
            &"a".repeat(64),
            "operative:7",
            &"b".repeat(64),
        )
        .expect("exact XUI14 tooltip payload");

        assert_eq!(payloads.primary.rows.len(), 1);
        assert_eq!(payloads.delayed.as_ref().unwrap().rows.len(), 2);
        assert_eq!(payloads.primary.rows[0].text, "Operative");
        let delayed = payloads.delayed.as_ref().unwrap();
        assert!(delayed
            .rows
            .iter()
            .any(|row| row.text == "Exact delayed details"));
        assert!(delayed.rows.iter().all(|row| {
            !row.text.contains("<measured-empty>") && !row.text.contains("state_id=")
        }));
        for row in &payloads.primary.rows {
            assert!(row.admitted_sources.iter().all(|source| matches!(
                source,
                AdmittedTooltipSource::PacketSemantic { packet_id, .. } if packet_id == "XUI14"
            )));
        }
    }
}

fn grid_origin(layout: &GridBoxLayout, index: usize) -> Result<(i32, i32)> {
    let horizontal = layout.add_horizontal.unwrap_or(true);
    let (column, row) = if horizontal {
        layout
            .max_slots_x
            .filter(|value| *value > 0)
            .map_or((index, 0), |columns| (index % columns, index / columns))
    } else {
        layout
            .max_slots_y
            .filter(|value| *value > 0)
            .map_or((0, index), |rows| (index / rows, index % rows))
    };
    if layout.max_slots_x.is_some_and(|max| column >= max)
        || layout.max_slots_y.is_some_and(|max| row >= max)
    {
        return Err(GameUiError::missing(
            "XUI14 authored grid capacity exhausted",
        ));
    }
    let absolute = layout.explicit_coordinate_origin((column as i32, row as i32))?;
    Ok((absolute.0 - layout.rect.x(), absolute.1 - layout.rect.y()))
}

fn descendant_with_parent_rect<'a>(
    root: &'a GuiElement,
    wanted: &str,
    root_rect: Rect,
    options: &ComposeOptions,
) -> Result<(&'a GuiElement, Rect)> {
    fn walk<'a>(
        current: &'a GuiElement,
        wanted: &str,
        parent: Rect,
        options: &ComposeOptions,
        hits: &mut Vec<(&'a GuiElement, Rect)>,
    ) {
        if current.name == wanted {
            hits.push((current, parent));
        }
        let rect = resolve_element_rect(current, parent, (0, 0), options);
        for child in &current.children {
            walk(child, wanted, rect, options, hits);
        }
    }
    let mut hits = Vec::new();
    for child in &root.children {
        walk(child, wanted, root_rect, options, &mut hits);
    }
    if hits.len() != 1 {
        return Err(GameUiError::missing(format!(
            "XUI14 expected one {wanted}, found {}",
            hits.len()
        )));
    }
    Ok(hits[0])
}

fn element_rect(
    root: &GuiElement,
    wanted: &str,
    root_rect: Rect,
    options: &ComposeOptions,
) -> Result<Rect> {
    let (element, parent) = descendant_with_parent_rect(root, wanted, root_rect, options)?;
    Ok(resolve_element_rect(element, parent, (0, 0), options))
}

fn descendant_mut_exact<'a>(root: &'a mut GuiElement, wanted: &str) -> Result<&'a mut GuiElement> {
    let count = count(root, wanted);
    if count != 1 {
        return Err(GameUiError::missing(format!(
            "XUI14 expected one {wanted}, found {count}"
        )));
    }
    find_mut(root, wanted).ok_or_else(|| GameUiError::missing(wanted))
}

fn count(root: &GuiElement, wanted: &str) -> usize {
    usize::from(root.name == wanted)
        + root
            .children
            .iter()
            .map(|child| count(child, wanted))
            .sum::<usize>()
}

fn find_mut<'a>(root: &'a mut GuiElement, wanted: &str) -> Option<&'a mut GuiElement> {
    if root.name == wanted {
        return Some(root);
    }
    root.children
        .iter_mut()
        .find_map(|child| find_mut(child, wanted))
}

fn require_sprite(registry: &SpriteRegistry, sprite: &str) -> Result<()> {
    registry
        .get(sprite)
        .map(|_| ())
        .ok_or_else(|| GameUiError::missing(format!("XUI14 effective GFX lacks {sprite:?}")))
}

fn set_text(options: &mut ComposeOptions, name: &str, text: impl Into<String>) {
    options.text_overrides.insert(name.into(), text.into());
}

fn wrapper_root(width: usize, height: usize, child: GuiElement) -> Result<GuiElement> {
    Ok(GuiElement {
        element_type: "containerWindowType".into(),
        name: "xui14_intelligence_root".into(),
        position: None,
        size: Some(pair(
            i32::try_from(width).map_err(|_| GameUiError::parse("XUI14 width overflow"))?,
            i32::try_from(height).map_err(|_| GameUiError::parse("XUI14 height overflow"))?,
        )),
        orientation: Some("UPPER_LEFT".into()),
        sprite_refs: Vec::new(),
        font: None,
        text: None,
        format: None,
        children: vec![child],
        raw: crate::paradox_text::Value::Object(Vec::new()),
    })
}

fn rect_array(rect: Rect) -> [i32; 4] {
    [rect.x(), rect.y(), rect.w(), rect.h()]
}
fn contains(rect: [i32; 4], x: i32, y: i32) -> bool {
    x >= rect[0]
        && y >= rect[1]
        && x < rect[0].saturating_add(rect[2])
        && y < rect[1].saturating_add(rect[3])
}
