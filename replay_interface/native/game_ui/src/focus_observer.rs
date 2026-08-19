
use crate::assets::AssetResolver;
use crate::compose::{ComposeOptions, ComposeOutput, Compositor, DebugRect};
use crate::gfx::SpriteRegistry;
use crate::gui::{parse_measure, GuiElement, LayoutSet, Measure, MeasurePair};
use crate::loc::Localisation;
use crate::national_focus::{AdmittedNationalFocusPresentation, FocusNodeState};
use crate::{GameUiError, Result};
use snapshot::sha256::{to_hex, Sha256};
use std::collections::BTreeSet;
use std::fs;

pub const XUI10_GUI_SOURCE: &str = "interface/nationalfocusview.gui";
pub const XUI10_GUI_ROOT: &str = "nationalfocusview";
pub const XUI10_NODE_TEMPLATE: &str = "national_focus_item";
pub const XUI10_LINK_TEMPLATE: &str = "national_focus_link";
pub const XUI10_EXCLUSIVE_TEMPLATE: &str = "national_focus_exclusive_item";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatedFocusProgress {
    pub progress_raw: i64,
    pub effective_required_progress_raw: i64,
    pub progress_scale_raw: i64,
    pub display_progress_percent: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatedFocusNode {
    pub focus_id: String,
    pub source_ordinal: u32,
    pub canvas_position_pixels: [i32; 2],
    pub presentation_state_code: u8,
    pub display_progress_percent: u8,
    pub continuous_glow_visible: bool,
    pub highlight_glow_visible: bool,
    pub historical_marker_visible: bool,
    pub joint_viewing_country_tag: Option<String>,
    pub joint_viewing_flag_sprite: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatedFocusLinkTile {
    pub source_ordinal: u32,
    pub from_focus_id: String,
    pub to_focus_id: String,
    pub mutually_exclusive: bool,
    pub container_width_pixels: Option<i32>,
    pub canvas_position_pixels: [i32; 2],
    pub child_operations: Vec<EvaluatedFocusLinkChildOperation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatedFocusLinkChildOperation {
    pub child_name: String,
    pub visible: bool,
    pub sprite_name: String,
    pub frame_index: usize,
    pub local_position_pixels: [i32; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatedFocusScene {
    pub frame_hour: u64,
    pub country_tag: String,
    pub tree_id: String,
    pub effective_vfs_manifest_sha256: String,
    pub authored_gui_sha256: String,
    pub producer_complete: bool,
    pub expected_node_count: u32,
    pub expected_link_tile_count: u32,
    pub progress: Option<EvaluatedFocusProgress>,
    pub nodes: Vec<EvaluatedFocusNode>,
    pub link_tiles: Vec<EvaluatedFocusLinkTile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedFocusNode {
    pub focus_id: String,
    pub canvas_position_pixels: [i32; 2],
    pub state: FocusNodeState,
    pub background_sprite: String,
    pub icon_sprite: String,
    pub overlay_sprite: Option<String>,
    pub title_loc_key: String,
    pub display_progress_percent: u8,
    pub continuous_glow_visible: bool,
    pub highlight_glow_visible: bool,
    pub historical_marker_visible: bool,
    pub joint_viewing_country_tag: Option<String>,
    pub joint_viewing_flag_sprite: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedObserverFocusScene {
    frame_hour: u64,
    country_tag: String,
    tree_id: String,
    authored_gui_sha256: String,
    progress: Option<EvaluatedFocusProgress>,
    nodes: Vec<PreparedFocusNode>,
    link_tiles: Vec<EvaluatedFocusLinkTile>,
}

impl PreparedObserverFocusScene {
    pub fn frame_hour(&self) -> u64 {
        self.frame_hour
    }

    pub fn country_tag(&self) -> &str {
        &self.country_tag
    }

    pub fn tree_id(&self) -> &str {
        &self.tree_id
    }

    pub fn progress(&self) -> Option<&EvaluatedFocusProgress> {
        self.progress.as_ref()
    }

    pub fn nodes(&self) -> &[PreparedFocusNode] {
        &self.nodes
    }

    pub fn link_tiles(&self) -> &[EvaluatedFocusLinkTile] {
        &self.link_tiles
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredFocusLayout {
    pub source_rel_path: String,
    pub source_payload_sha256: String,
    pub root: GuiElement,
    pub node_template: GuiElement,
    pub link_template: GuiElement,
    pub exclusive_template: GuiElement,
    pub node_extent_pixels: [i32; 2],
    pub focus_spacing_pixels: [i32; 2],
    pub focus_center_pixels: [i32; 2],
    pub link_spacing_pixels: [i32; 2],
    pub link_offsets_pixels: [i32; 2],
    pub link_begin_pixels: [i32; 2],
    pub link_end_pixels: [i32; 2],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusViewerState {
    pub open: bool,
    pub pan_pixels: [i32; 2],
    pub selected_focus_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObserverFocusInputRegion {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ObserverFocusInputRegion {
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverFocusNodeRegion {
    pub focus_id: String,
    pub region: ObserverFocusInputRegion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverFocusProgressBinding {
    pub focus_id: String,
    pub display_progress_percent: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComposedObserverFocusSurface {
    pub output: ComposeOutput,
    pub input_region: ObserverFocusInputRegion,
    pub node_regions: Vec<ObserverFocusNodeRegion>,
    pub progress: Option<ObserverFocusProgressBinding>,
}

impl FocusViewerState {
    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.selected_focus_id = None;
    }

    pub fn reset_for_seek_or_country_change(&mut self) {
        *self = Self::default();
    }

    pub fn pan_by(&mut self, delta: [i32; 2]) -> Result<()> {
        self.pan_pixels = [
            self.pan_pixels[0]
                .checked_add(delta[0])
                .ok_or_else(|| GameUiError::parse("XUI10 viewer pan x overflow"))?,
            self.pan_pixels[1]
                .checked_add(delta[1])
                .ok_or_else(|| GameUiError::parse("XUI10 viewer pan y overflow"))?,
        ];
        Ok(())
    }

    pub fn select(&mut self, scene: &PreparedObserverFocusScene, id: Option<&str>) -> Result<()> {
        if let Some(id) = id {
            if !scene.nodes.iter().any(|node| node.focus_id == id) {
                return Err(GameUiError::missing(format!(
                    "XUI10 viewer selection {id:?} is not in the prepared scene"
                )));
            }
            self.selected_focus_id = Some(id.to_string());
        } else {
            self.selected_focus_id = None;
        }
        Ok(())
    }

    pub fn hit_test<'a>(
        &self,
        scene: &'a PreparedObserverFocusScene,
        layout: &AuthoredFocusLayout,
        canvas_point: [i32; 2],
    ) -> Option<&'a str> {
        let [width, height] = layout.node_extent_pixels;
        scene.nodes.iter().rev().find_map(|node| {
            let x = node.canvas_position_pixels[0].checked_add(self.pan_pixels[0])?;
            let y = node.canvas_position_pixels[1].checked_add(self.pan_pixels[1])?;
            (canvas_point[0] >= x
                && canvas_point[0] < x.checked_add(width)?
                && canvas_point[1] >= y
                && canvas_point[1] < y.checked_add(height)?)
            .then_some(node.focus_id.as_str())
        })
    }
}

pub fn prepare_observer_focus_scene(
    semantic: &AdmittedNationalFocusPresentation,
    evaluated: &EvaluatedFocusScene,
    expected_frame_hour: u64,
    expected_country_tag: &str,
    expected_effective_vfs_manifest_sha256: &str,
    expected_authored_gui_sha256: &str,
) -> Result<PreparedObserverFocusScene> {
    if !evaluated.producer_complete
        || evaluated.frame_hour != expected_frame_hour
        || evaluated.country_tag != expected_country_tag
        || evaluated.tree_id != semantic.tree_id
        || evaluated.effective_vfs_manifest_sha256 != expected_effective_vfs_manifest_sha256
        || evaluated.authored_gui_sha256 != expected_authored_gui_sha256
        || !valid_country_tag(&evaluated.country_tag)
        || !lowercase_sha256(&evaluated.effective_vfs_manifest_sha256)
        || !lowercase_sha256(&evaluated.authored_gui_sha256)
        || usize::try_from(evaluated.expected_node_count).ok() != Some(evaluated.nodes.len())
        || usize::try_from(evaluated.expected_link_tile_count).ok()
            != Some(evaluated.link_tiles.len())
        || evaluated.nodes.len() != semantic.nodes.len()
    {
        return Err(GameUiError::missing(
            "XUI10 evaluated scene identity or completeness disagrees with replay authority",
        ));
    }

    let mut nodes = Vec::with_capacity(semantic.nodes.len());
    for (ordinal, (base, row)) in semantic.nodes.iter().zip(&evaluated.nodes).enumerate() {
        let state = state_from_code(row.presentation_state_code)?;
        if row.source_ordinal as usize != ordinal
            || row.focus_id != base.id
            || state != base.state
            || row.display_progress_percent > 100
            || row.continuous_glow_visible != (state == FocusNodeState::Current)
            || (state == FocusNodeState::Completed && row.display_progress_percent != 100)
            || (matches!(
                state,
                FocusNodeState::Available | FocusNodeState::Unavailable
            ) && row.display_progress_percent != 0)
            || row
                .joint_viewing_country_tag
                .as_deref()
                .is_some_and(|tag| !valid_country_tag(tag))
            || row.joint_viewing_country_tag.is_some() != row.joint_viewing_flag_sprite.is_some()
            || row
                .joint_viewing_flag_sprite
                .as_deref()
                .is_some_and(|sprite| sprite.trim().is_empty())
        {
            return Err(GameUiError::missing(format!(
                "XUI10 evaluated node {ordinal} disagrees with D12/FILE identity or state"
            )));
        }
        nodes.push(PreparedFocusNode {
            focus_id: base.id.clone(),
            canvas_position_pixels: row.canvas_position_pixels,
            state,
            background_sprite: base.background_sprite.clone(),
            icon_sprite: base.icon_sprite.clone(),
            overlay_sprite: base.overlay_sprite.clone(),
            title_loc_key: base.title_loc_key.clone(),
            display_progress_percent: row.display_progress_percent,
            continuous_glow_visible: row.continuous_glow_visible,
            highlight_glow_visible: row.highlight_glow_visible,
            historical_marker_visible: row.historical_marker_visible,
            joint_viewing_country_tag: row.joint_viewing_country_tag.clone(),
            joint_viewing_flag_sprite: row.joint_viewing_flag_sprite.clone(),
        });
    }

    validate_progress(evaluated.progress.as_ref(), &nodes)?;
    let ids = nodes
        .iter()
        .map(|node| node.focus_id.as_str())
        .collect::<BTreeSet<_>>();
    for (ordinal, link) in evaluated.link_tiles.iter().enumerate() {
        if link.source_ordinal as usize != ordinal
            || link.from_focus_id == link.to_focus_id
            || !ids.contains(link.from_focus_id.as_str())
            || !ids.contains(link.to_focus_id.as_str())
        {
            return Err(GameUiError::missing(format!(
                "XUI10 evaluated link tile {ordinal} has invalid order, identity, or sprite"
            )));
        }
        validate_link_operations(link)?;
    }

    Ok(PreparedObserverFocusScene {
        frame_hour: evaluated.frame_hour,
        country_tag: evaluated.country_tag.clone(),
        tree_id: evaluated.tree_id.clone(),
        authored_gui_sha256: evaluated.authored_gui_sha256.clone(),
        progress: evaluated.progress.clone(),
        nodes,
        link_tiles: evaluated.link_tiles.clone(),
    })
}

pub fn load_authored_focus_layout(resolver: &AssetResolver) -> Result<AuthoredFocusLayout> {
    let resolved = resolver.resolve_file(XUI10_GUI_SOURCE)?;
    let bytes = fs::read(&resolved.path)?;
    let mut layouts = LayoutSet::default();
    layouts.parse_file(&resolved.path, &resolved.rel_path)?;
    if layouts.stats.recovery_count != 0 || layouts.stats.decode_loss_count != 0 {
        return Err(GameUiError::parse(
            "XUI10 authored GUI required parser recovery or destructive decoding",
        ));
    }
    let root = last_top_level(&layouts, XUI10_GUI_ROOT)?;
    let node_template = last_top_level(&layouts, XUI10_NODE_TEMPLATE)?;
    let link_template = last_top_level(&layouts, XUI10_LINK_TEMPLATE)?;
    let exclusive_template = last_top_level(&layouts, XUI10_EXCLUSIVE_TEMPLATE)?;
    validate_template_children(&link_template, &["link"], XUI10_LINK_TEMPLATE)?;
    validate_template_children(
        &exclusive_template,
        &["link1", "link2", "left", "right", "mid"],
        XUI10_EXCLUSIVE_TEMPLATE,
    )?;
    let node_extent_pixels = exact_size(&node_template, XUI10_NODE_TEMPLATE)?;
    Ok(AuthoredFocusLayout {
        source_rel_path: resolved.rel_path,
        source_payload_sha256: to_hex(&Sha256::digest(&bytes)),
        root,
        node_template,
        link_template,
        exclusive_template,
        node_extent_pixels,
        focus_spacing_pixels: exact_marker(&layouts, "focus_spacing")?,
        focus_center_pixels: exact_marker(&layouts, "national_focus_center")?,
        link_spacing_pixels: exact_marker(&layouts, "link_spacing")?,
        link_offsets_pixels: exact_marker(&layouts, "link_offsets")?,
        link_begin_pixels: exact_marker(&layouts, "link_begin")?,
        link_end_pixels: exact_marker(&layouts, "link_end")?,
    })
}

pub fn build_observer_focus_compose_scene(
    layout: &AuthoredFocusLayout,
    scene: &PreparedObserverFocusScene,
    viewer: &FocusViewerState,
    localisation: &Localisation,
    viewport_width: usize,
    viewport_height: usize,
) -> Result<Option<(GuiElement, ComposeOptions)>> {
    if !viewer.open {
        return Ok(None);
    }
    if scene.authored_gui_sha256 != layout.source_payload_sha256 {
        return Err(GameUiError::missing(
            "XUI10 scene GUI identity differs from effective authored layout",
        ));
    }
    let mut root = layout.root.clone();
    let grid = child_chain_mut(&mut root, &["tree", "grid_window", "grid"])?;
    grid.children.clear();
    let mut options =
        ComposeOptions::new(viewport_width, viewport_height).with_localisation(localisation);
    for unsupported in [
        "find_view",
        "zoom_slider_container",
        "techtree_stripes",
        "search_tooltip",
        "focus_tree_scripted_window_container",
        "continuous_focus_window",
        "tutorial_glow_1",
        "tutorial_glow_2",
        "tutorial_glow_3",
        "tutorial_glow_4",
    ] {
        options.hidden_elements.insert(unsupported.to_string());
    }

    for link in &scene.link_tiles {
        let prefix = format!("xui10_link_{}__", link.source_ordinal);
        let mut element = if link.mutually_exclusive {
            layout.exclusive_template.clone()
        } else {
            layout.link_template.clone()
        };
        if let Some(width) = link.container_width_pixels {
            let size = element.size.as_mut().ok_or_else(|| {
                GameUiError::missing("XUI10 evaluated link width has no authored container size")
            })?;
            size.x = parse_measure(&width.to_string());
        }
        set_position_with_pan(&mut element, link.canvas_position_pixels, viewer.pan_pixels)?;
        prefix_names(&mut element, &prefix);
        for operation in &link.child_operations {
            let child =
                direct_child_mut(&mut element, &format!("{prefix}{}", operation.child_name))?;
            child.position = Some(pair(
                operation.local_position_pixels[0],
                operation.local_position_pixels[1],
            ));
            let child_name = format!("{prefix}{}", operation.child_name);
            options
                .sprite_name_overrides
                .insert(child_name.clone(), operation.sprite_name.clone());
            options
                .frame_overrides
                .insert(child_name.clone(), operation.frame_index);
            if !operation.visible {
                options.hidden_elements.insert(child_name);
            }
        }
        grid.children.push(element);
    }
    for (ordinal, node) in scene.nodes.iter().enumerate() {
        let prefix = format!("xui10_node_{ordinal}__");
        let mut element = layout.node_template.clone();
        set_position_with_pan(&mut element, node.canvas_position_pixels, viewer.pan_pixels)?;
        prefix_names(&mut element, &prefix);
        options
            .text_overrides
            .insert(format!("{prefix}name"), node.title_loc_key.clone());
        options
            .sprite_name_overrides
            .insert(format!("{prefix}bg"), node.background_sprite.clone());
        options
            .sprite_name_overrides
            .insert(format!("{prefix}symbol"), node.icon_sprite.clone());
        match &node.overlay_sprite {
            Some(sprite) => {
                options
                    .sprite_name_overrides
                    .insert(format!("{prefix}overlay"), sprite.clone());
            }
            None => {
                options.hidden_elements.insert(format!("{prefix}overlay"));
            }
        }
        for (name, visible) in [
            ("continuous_glow", node.continuous_glow_visible),
            (
                "highlight_glow",
                node.highlight_glow_visible
                    || viewer.selected_focus_id.as_deref() == Some(node.focus_id.as_str()),
            ),
            ("historical", node.historical_marker_visible),
        ] {
            if !visible {
                options.hidden_elements.insert(format!("{prefix}{name}"));
            }
        }
        match node.joint_viewing_flag_sprite.as_ref() {
            Some(sprite) => {
                options
                    .sprite_name_overrides
                    .insert(format!("{prefix}viewing_flag"), sprite.clone());
            }
            None => {
                options
                    .hidden_elements
                    .insert(format!("{prefix}viewing_flag"));
                options
                    .hidden_elements
                    .insert(format!("{prefix}viewing_flag_border"));
            }
        }
        grid.children.push(element);
    }
    Ok(Some((root, options)))
}

pub fn render_observer_focus(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    layout: &AuthoredFocusLayout,
    scene: &PreparedObserverFocusScene,
    viewer: &FocusViewerState,
    localisation: &Localisation,
    viewport_width: usize,
    viewport_height: usize,
) -> Result<Option<ComposedObserverFocusSurface>> {
    let Some((root, options)) = build_observer_focus_compose_scene(
        layout,
        scene,
        viewer,
        localisation,
        viewport_width,
        viewport_height,
    )?
    else {
        return Ok(None);
    };
    let mut compositor = Compositor::new(resolver, registry);
    let output = compositor.compose(&root, &options);
    validate_complete_report(&output)?;
    let input_region = exact_debug_region(&output.debug_rects, "tree")?;
    let node_regions = scene
        .nodes
        .iter()
        .enumerate()
        .map(|(ordinal, node)| {
            Ok(ObserverFocusNodeRegion {
                focus_id: node.focus_id.clone(),
                region: exact_debug_region(
                    &output.debug_rects,
                    &format!("xui10_node_{ordinal}__{XUI10_NODE_TEMPLATE}"),
                )?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let progress = scene.progress.as_ref().map(|progress| {
        let focus_id = scene
            .nodes
            .iter()
            .find(|node| node.state == FocusNodeState::Current)
            .expect("validated progress has exactly one current focus")
            .focus_id
            .clone();
        ObserverFocusProgressBinding {
            focus_id,
            display_progress_percent: progress.display_progress_percent,
        }
    });
    Ok(Some(ComposedObserverFocusSurface {
        output,
        input_region,
        node_regions,
        progress,
    }))
}

fn validate_link_operations(link: &EvaluatedFocusLinkTile) -> Result<()> {
    let expected: &[&str] = if link.mutually_exclusive {
        if link.container_width_pixels.is_none_or(|width| width <= 0) {
            return Err(GameUiError::missing(
                "XUI10 exclusive link lacks its evaluated positive container width",
            ));
        }
        &["link1", "link2", "left", "right", "mid"]
    } else {
        if link.container_width_pixels.is_some() {
            return Err(GameUiError::missing(
                "XUI10 ordinary link supplied an unauthored width override",
            ));
        }
        &["link"]
    };
    if link.child_operations.len() != expected.len()
        || link
            .child_operations
            .iter()
            .zip(expected)
            .any(|(operation, expected_name)| {
                operation.child_name != *expected_name || operation.sprite_name.trim().is_empty()
            })
    {
        return Err(GameUiError::missing(
            "XUI10 evaluated link child operation census or order is invalid",
        ));
    }
    Ok(())
}

fn validate_template_children(
    element: &GuiElement,
    expected: &[&str],
    template: &str,
) -> Result<()> {
    let actual = element
        .children
        .iter()
        .map(|child| child.name.as_str())
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(GameUiError::missing(format!(
            "{XUI10_GUI_SOURCE}: {template} direct child census/order is {actual:?}, expected {expected:?}"
        )));
    }
    Ok(())
}

fn direct_child_mut<'a>(element: &'a mut GuiElement, name: &str) -> Result<&'a mut GuiElement> {
    let matches = element
        .children
        .iter()
        .enumerate()
        .filter(|(_, child)| child.name == name)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let [index] = matches.as_slice() else {
        return Err(GameUiError::missing(format!(
            "XUI10 composed template child {name:?} is absent or ambiguous"
        )));
    };
    Ok(&mut element.children[*index])
}

fn exact_debug_region(debug_rects: &[DebugRect], name: &str) -> Result<ObserverFocusInputRegion> {
    let matches = debug_rects
        .iter()
        .filter(|rect| rect.name == name)
        .collect::<Vec<_>>();
    let [rect] = matches.as_slice() else {
        return Err(GameUiError::missing(format!(
            "XUI10 composed input region {name:?} is absent or ambiguous"
        )));
    };
    if rect.w <= 0 || rect.h <= 0 {
        return Err(GameUiError::missing(format!(
            "XUI10 composed input region {name:?} is empty"
        )));
    }
    Ok(ObserverFocusInputRegion {
        x: rect.x,
        y: rect.y,
        width: rect.w,
        height: rect.h,
    })
}

fn validate_progress(
    progress: Option<&EvaluatedFocusProgress>,
    nodes: &[PreparedFocusNode],
) -> Result<()> {
    let active = nodes
        .iter()
        .filter(|node| node.state == FocusNodeState::Current)
        .collect::<Vec<_>>();
    if progress.is_some() != (active.len() == 1) {
        return Err(GameUiError::missing(
            "XUI10 progress tuple does not match the exact active-node census",
        ));
    }
    let Some(progress) = progress else {
        return Ok(());
    };
    if progress.progress_raw < 0
        || progress.effective_required_progress_raw <= 0
        || progress.progress_scale_raw <= 0
        || progress.display_progress_percent > 100
        || active[0].display_progress_percent != progress.display_progress_percent
    {
        return Err(GameUiError::missing(
            "XUI10 exact progress tuple is outside its domain",
        ));
    }
    let required = progress
        .effective_required_progress_raw
        .checked_mul(100_000)
        .map(|value| value / progress.progress_scale_raw)
        .filter(|value| *value > 0);
    let current = progress
        .progress_raw
        .checked_mul(100_000)
        .map(|value| value / progress.progress_scale_raw);
    let expected = required
        .zip(current)
        .and_then(|(required, current)| {
            current
                .checked_mul(10_000_000)
                .map(|value| (value, required))
        })
        .map(|(value, required)| (value / required) / 100_000)
        .filter(|value| (0..=100).contains(value));
    if expected != Some(i64::from(progress.display_progress_percent)) {
        return Err(GameUiError::missing(
            "XUI10 final percent disagrees with exact a729 integer truncation law",
        ));
    }
    Ok(())
}

fn validate_complete_report(output: &ComposeOutput) -> Result<()> {
    let report = &output.report;
    if report.drawn_elements == 0
        || !report.missing_sprites.is_empty()
        || !report.undecodable_sprites.is_empty()
        || !report.missing_fonts.is_empty()
        || !report.missing_glyphs.is_empty()
        || !report.unresolved_loc_keys.is_empty()
        || !report.unresolved_text_inputs.is_empty()
    {
        return Err(GameUiError::missing(
            "XUI10 compositor report is incomplete",
        ));
    }
    Ok(())
}

fn state_from_code(code: u8) -> Result<FocusNodeState> {
    match code {
        0 => Ok(FocusNodeState::Unavailable),
        1 => Ok(FocusNodeState::Completed),
        2 => Ok(FocusNodeState::Available),
        3 => Ok(FocusNodeState::Current),
        _ => Err(GameUiError::parse(
            "XUI10 evaluated state code is outside 0..=3",
        )),
    }
}

fn valid_country_tag(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn last_top_level(layouts: &LayoutSet, name: &str) -> Result<GuiElement> {
    layouts
        .documents
        .iter()
        .flat_map(|document| document.roots.iter())
        .rev()
        .find(|element| element.name.eq_ignore_ascii_case(name))
        .cloned()
        .ok_or_else(|| GameUiError::missing(format!("{XUI10_GUI_SOURCE}: top-level {name} absent")))
}

fn exact_marker(layouts: &LayoutSet, name: &str) -> Result<[i32; 2]> {
    let matches = layouts.find_by_name(name);
    let [element] = matches.as_slice() else {
        return Err(GameUiError::missing(format!(
            "{XUI10_GUI_SOURCE}: expected one marker {name}, found {}",
            matches.len()
        )));
    };
    exact_position(element, name)
}

fn exact_position(element: &GuiElement, name: &str) -> Result<[i32; 2]> {
    let position = element.position.as_ref().ok_or_else(|| {
        GameUiError::missing(format!("{XUI10_GUI_SOURCE}: {name} lacks position"))
    })?;
    Ok([
        exact_measure(&position.x, name)?,
        exact_measure(&position.y, name)?,
    ])
}

fn exact_size(element: &GuiElement, name: &str) -> Result<[i32; 2]> {
    let size = element
        .size
        .as_ref()
        .ok_or_else(|| GameUiError::missing(format!("{XUI10_GUI_SOURCE}: {name} lacks size")))?;
    let result = [exact_measure(&size.x, name)?, exact_measure(&size.y, name)?];
    if result[0] <= 0 || result[1] <= 0 {
        return Err(GameUiError::parse(format!(
            "{XUI10_GUI_SOURCE}: {name} has non-positive size"
        )));
    }
    Ok(result)
}

fn exact_measure(measure: &Measure, name: &str) -> Result<i32> {
    if measure.is_percent {
        return Err(GameUiError::parse(format!(
            "{XUI10_GUI_SOURCE}: {name} marker is not an exact integer"
        )));
    }
    measure.raw.parse::<i32>().map_err(|_| {
        GameUiError::parse(format!(
            "{XUI10_GUI_SOURCE}: {name} marker is not an exact integer"
        ))
    })
}

fn child_chain_mut<'a>(root: &'a mut GuiElement, names: &[&str]) -> Result<&'a mut GuiElement> {
    let mut current = root;
    for name in names {
        let matches = current
            .children
            .iter()
            .enumerate()
            .filter(|(_, child)| child.name.eq_ignore_ascii_case(name))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let [index] = matches.as_slice() else {
            return Err(GameUiError::missing(format!(
                "{XUI10_GUI_SOURCE}: path component {name} is absent or ambiguous"
            )));
        };
        current = &mut current.children[*index];
    }
    Ok(current)
}

fn set_position_with_pan(
    element: &mut GuiElement,
    position: [i32; 2],
    pan: [i32; 2],
) -> Result<()> {
    let x = position[0]
        .checked_add(pan[0])
        .ok_or_else(|| GameUiError::parse("XUI10 composed x position overflow"))?;
    let y = position[1]
        .checked_add(pan[1])
        .ok_or_else(|| GameUiError::parse("XUI10 composed y position overflow"))?;
    element.position = Some(pair(x, y));
    element.orientation = Some("UPPER_LEFT".to_string());
    Ok(())
}

fn prefix_names(element: &mut GuiElement, prefix: &str) {
    element.name = format!("{prefix}{}", element.name);
    for child in &mut element.children {
        prefix_names(child, prefix);
    }
}

fn pair(x: i32, y: i32) -> MeasurePair {
    MeasurePair {
        x: parse_measure(&x.to_string()),
        y: parse_measure(&y.to_string()),
    }
}
