//! XUI20 authored-canvas and viewer-selection contract.
//!
//! The combat/loss packet spans five engine-owned windows.  This module
//! discovers their winning definitions from the effective VFS, retains the
//! exact authored templates they clone, binds viewer-owned selection to one
//! replay hour and one complete engine identity, and refuses composition
//! before a pixel is drawn while a visible semantic family is unavailable.
//!
//! It deliberately does not turn d55 damage accumulators into casualties,
//! aggregate d86 rows into combat results, infer survivors from the absence of
//! a sink row, or manufacture equipment/ship presentation identities.

use crate::assets::AssetResolver;
use crate::compose::{ComposeOptions, ComposeOutput, Compositor, DebugRect};
use crate::definition_admission::{
    load_effective_definition_catalog, EffectiveGuiRootDefinition, ResolvedGuiDefinitionFile,
};
use crate::gfx::SpriteRegistry;
use crate::gui::GuiElement;
use crate::gui_scene_util::set_position_upper_left;
use crate::tooltip_compositor::{
    authored_tooltip_shell_name, AdmittedTooltipSource, RowAvailability, ScreenRect,
    TooltipLineKind, TooltipOwner, TooltipPayload, TooltipRow,
};
use crate::{GameUiError, Result};
use replay_core::{
    CombatDynamicRowExact, CombatElementBindingExact, CombatElementPathExact, CombatPanelKindExact,
    CombatPanelPresentationExact, CombatScrollChannelExact, WorldFrame,
};
use std::collections::{BTreeMap, BTreeSet};

/// Engine-selected controller roots.  The first, second, and fifth are exact
/// a729 strings cited by the XUI20 Ghidra jobs; the remaining two are the
/// winning FILE definitions consumed by their engine controllers.
pub const LAND_COMBAT_ROOT: &str = replay_core::XUI20_LAND_COMBAT_ROOT;
pub const NAVAL_COMBAT_ROOT: &str = replay_core::XUI20_NAVAL_COMBAT_ROOT;
pub const COMBAT_LOG_ROOT: &str = replay_core::XUI20_COMBAT_LOG_ROOT;
pub const NAVAL_LOSSES_ROOT: &str = replay_core::XUI20_NAVAL_LOSSES_ROOT;
pub const NAVAL_RESULTS_ROOT: &str = replay_core::XUI20_NAVAL_RESULTS_ROOT;

pub const XUI20_CANVAS_ROOTS: &[&str] = &[
    LAND_COMBAT_ROOT,
    NAVAL_COMBAT_ROOT,
    COMBAT_LOG_ROOT,
    NAVAL_LOSSES_ROOT,
    NAVAL_RESULTS_ROOT,
];

/// Dynamic definitions cloned by the five controllers.  These are names in
/// the engine's one case-insensitive GUI_TYPE registry, not vanilla paths;
/// effective-VFS last-parsed-wins selection is applied before they are read.
pub const XUI20_DYNAMIC_TEMPLATES: &[&str] = replay_core::XUI20_DYNAMIC_TEMPLATE_NAMES;

#[derive(Clone, Debug, PartialEq)]
pub struct Xui20AuthoredElement {
    pub name: String,
    pub source_file: String,
    pub window_id: String,
    pub root: GuiElement,
    /// Constructs the current compositor cannot reproduce exactly.  They are
    /// retained as a completeness input rather than silently discarded.
    pub unsupported: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xui20AuthoredCanvasCatalog {
    roots: BTreeMap<String, Xui20AuthoredElement>,
    templates: BTreeMap<String, Xui20AuthoredElement>,
}

impl Xui20AuthoredCanvasCatalog {
    pub fn root(&self, name: &str) -> Option<&Xui20AuthoredElement> {
        self.roots.get(&name.to_ascii_lowercase())
    }

    pub fn template(&self, name: &str) -> Option<&Xui20AuthoredElement> {
        self.templates.get(&name.to_ascii_lowercase())
    }

    pub fn roots(&self) -> impl Iterator<Item = &Xui20AuthoredElement> {
        self.roots.values()
    }

    pub fn templates(&self) -> impl Iterator<Item = &Xui20AuthoredElement> {
        self.templates.values()
    }
}

fn authored(row: &EffectiveGuiRootDefinition) -> Xui20AuthoredElement {
    Xui20AuthoredElement {
        name: row.root.name.clone(),
        source_file: row.source_file.clone(),
        window_id: row.window_id.clone(),
        root: row.root.clone(),
        unsupported: row.unsupported.clone(),
    }
}

fn require_winner(
    winners: &BTreeMap<String, &EffectiveGuiRootDefinition>,
    name: &str,
) -> Result<Xui20AuthoredElement> {
    let row = winners.get(&name.to_ascii_lowercase()).ok_or_else(|| {
        GameUiError::missing(format!(
            "XUI20 effective GUI registry has no winning definition for {name}"
        ))
    })?;
    if !row
        .root
        .element_type
        .eq_ignore_ascii_case("containerWindowType")
    {
        return Err(GameUiError::parse(format!(
            "XUI20 definition {name} is {}, expected containerWindowType",
            row.root.element_type
        )));
    }
    Ok(authored(row))
}

/// Load the exact effective-VFS winners for every XUI20 root and clone
/// template.  No vanilla filename is used for selection: mods may replace a
/// same-path file or move a winning definition to another interface file.
pub fn load_xui20_authored_canvas_catalog(
    resolver: &AssetResolver,
) -> Result<Xui20AuthoredCanvasCatalog> {
    let files = resolver.list_files_recursive("interface", Some("gui"))?;
    if files.is_empty() {
        return Err(GameUiError::missing(
            "XUI20 effective VFS exposes no interface GUI definitions",
        ));
    }
    let catalog = load_effective_definition_catalog(
        files
            .into_iter()
            .map(|row| ResolvedGuiDefinitionFile::new(row.rel_path, row.path))
            .collect(),
    )?;
    let winners = catalog
        .effective_roots
        .iter()
        .map(|row| (row.root.name.to_ascii_lowercase(), row))
        .collect::<BTreeMap<_, _>>();

    let mut roots = BTreeMap::new();
    for name in XUI20_CANVAS_ROOTS {
        roots.insert(name.to_ascii_lowercase(), require_winner(&winners, name)?);
    }
    let mut templates = BTreeMap::new();
    for name in XUI20_DYNAMIC_TEMPLATES {
        templates.insert(name.to_ascii_lowercase(), require_winner(&winners, name)?);
    }
    Ok(Xui20AuthoredCanvasCatalog { roots, templates })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Xui20CombatLogTab {
    Combats,
    Equipment,
    Templates,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xui20ViewerPanel {
    LandCombat {
        combat_id: i64,
        generation: i64,
    },
    NavalCombat {
        combat_id: i64,
        generation: i64,
    },
    CombatLog {
        country_tag: String,
        tab: Xui20CombatLogTab,
    },
    NavalLosses {
        country_tag: String,
    },
    NavalResults {
        combat_id: i64,
        generation: i64,
    },
}

impl Xui20ViewerPanel {
    pub fn root_name(&self) -> &'static str {
        match self {
            Self::LandCombat { .. } => LAND_COMBAT_ROOT,
            Self::NavalCombat { .. } => NAVAL_COMBAT_ROOT,
            Self::CombatLog { .. } => COMBAT_LOG_ROOT,
            Self::NavalLosses { .. } => NAVAL_LOSSES_ROOT,
            Self::NavalResults { .. } => NAVAL_RESULTS_ROOT,
        }
    }

    pub fn country_tag(&self) -> Option<&str> {
        match self {
            Self::CombatLog { country_tag, .. } | Self::NavalLosses { country_tag } => {
                Some(country_tag)
            }
            _ => None,
        }
    }
}

/// Pure playback state.  The recorder's selected combat/window/scroll state
/// is never consulted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Xui20ViewerState {
    pub panel: Option<Xui20ViewerPanel>,
    /// The frame for which `panel` was selected.  A seek invalidates the
    /// selection until the viewer explicitly selects a new identity.
    pub selected_hour: Option<i64>,
    pub primary_scroll_px: i32,
    pub secondary_scroll_px: i32,
}

impl Xui20ViewerState {
    pub fn open(&mut self, panel: Xui20ViewerPanel, frame_hour: i64) {
        *self = Self {
            panel: Some(panel),
            selected_hour: Some(frame_hour),
            primary_scroll_px: 0,
            secondary_scroll_px: 0,
        };
    }

    pub fn close(&mut self) {
        *self = Self::default();
    }

    pub fn reset_for_seek(&mut self) {
        self.close();
    }

    pub fn wheel(
        &mut self,
        channel: CombatScrollChannelExact,
        notches: i32,
        wheel_px: i32,
        maximum_px: i32,
    ) -> Result<()> {
        if wheel_px < 0 || maximum_px < 0 {
            return Err(GameUiError::parse("XUI20 invalid scroll contract"));
        }
        let delta = notches
            .checked_mul(wheel_px)
            .ok_or_else(|| GameUiError::parse("XUI20 scroll delta overflow"))?;
        let target = match channel {
            CombatScrollChannelExact::Primary => &mut self.primary_scroll_px,
            CombatScrollChannelExact::Secondary => &mut self.secondary_scroll_px,
            CombatScrollChannelExact::None => {
                return Err(GameUiError::parse(
                    "XUI20 cannot wheel a non-scroll channel",
                ));
            }
        };
        *target = target.saturating_sub(delta).clamp(0, maximum_px);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xui20CanvasRoute {
    pub panel: Xui20ViewerPanel,
    pub root: Xui20AuthoredElement,
    pub presentation: CombatPanelPresentationExact,
    pub frame_hour: u64,
    pub capture_sequence: u64,
    pub record_sha256: String,
    pub content_manifest_sha256: String,
}

impl Xui20CanvasRoute {
    pub fn is_composable(&self) -> bool {
        self.presentation.complete && self.root.unsupported.is_empty()
    }

    /// Production completeness gate.  Call this before constructing a
    /// `Compositor`; a partial XUI20 panel must never reach a render target.
    pub fn require_composable(&self) -> std::result::Result<(), String> {
        if !self.root.unsupported.is_empty() {
            return Err(format!(
                "XUI20 authored root {} contains unsupported constructs {:?}",
                self.root.name, self.root.unsupported
            ));
        }
        if !self.presentation.complete {
            return Err("XUI20 strict panel presentation is incomplete".into());
        }
        Ok(())
    }
}

fn selected_kind(panel: &Xui20ViewerPanel) -> CombatPanelKindExact {
    match panel {
        Xui20ViewerPanel::LandCombat { .. } => CombatPanelKindExact::LandCombat,
        Xui20ViewerPanel::NavalCombat { .. } => CombatPanelKindExact::NavalCombat,
        Xui20ViewerPanel::CombatLog { .. } => CombatPanelKindExact::CombatLog,
        Xui20ViewerPanel::NavalLosses { .. } => CombatPanelKindExact::NavalLosses,
        Xui20ViewerPanel::NavalResults { .. } => CombatPanelKindExact::NavalResults,
    }
}

fn selected_tab(panel: &Xui20ViewerPanel) -> Option<replay_core::CombatLogTabExact> {
    match panel {
        Xui20ViewerPanel::CombatLog { tab, .. } => Some(match tab {
            Xui20CombatLogTab::Combats => replay_core::CombatLogTabExact::Combats,
            Xui20CombatLogTab::Equipment => replay_core::CombatLogTabExact::Equipment,
            Xui20CombatLogTab::Templates => replay_core::CombatLogTabExact::Templates,
        }),
        _ => None,
    }
}

fn scoped_binding_is_active(
    binding: &replay_core::CombatScopedBindingExact,
    selected_tab: Option<replay_core::CombatLogTabExact>,
) -> bool {
    binding.combat_log_tab.is_none() || binding.combat_log_tab == selected_tab
}

/// Direct texture reads are outside the authored `.gfx` dependency walk, so
/// the shipping renderer must explicitly admit every active one before the
/// compositor receives an asset resolver.
pub(crate) fn active_texture_paths(route: &Xui20CanvasRoute) -> BTreeSet<String> {
    let tab = selected_tab(&route.panel);
    route
        .presentation
        .root_bindings
        .iter()
        .filter(|binding| scoped_binding_is_active(binding, tab))
        .chain(
            route
                .presentation
                .dynamic_rows
                .iter()
                .filter(|row| row.combat_log_tab.is_none() || row.combat_log_tab == tab)
                .flat_map(|row| row.bindings.iter())
                .filter(|binding| scoped_binding_is_active(binding, tab)),
        )
        .filter_map(|binding| match &binding.binding {
            CombatElementBindingExact::Texture { texture_path, .. } => Some(texture_path.clone()),
            _ => None,
        })
        .collect()
}

fn selection_matches(
    viewer: &Xui20ViewerPanel,
    presentation: &CombatPanelPresentationExact,
) -> bool {
    if presentation.panel_kind != selected_kind(viewer) {
        return false;
    }
    match viewer {
        Xui20ViewerPanel::LandCombat {
            combat_id,
            generation,
        }
        | Xui20ViewerPanel::NavalCombat {
            combat_id,
            generation,
        }
        | Xui20ViewerPanel::NavalResults {
            combat_id,
            generation,
        } => {
            presentation.owner_country_tag.is_none()
                && presentation.combat_id == Some(*combat_id)
                && presentation.combat_generation == Some(*generation)
        }
        Xui20ViewerPanel::CombatLog { country_tag, .. }
        | Xui20ViewerPanel::NavalLosses { country_tag } => {
            presentation.owner_country_tag.as_deref() == Some(country_tag)
                && presentation.combat_id.is_none()
                && presentation.combat_generation.is_none()
        }
    }
}

/// Bind the viewer selection to the exact frame and authenticated authored
/// winner. A successful route is complete input for the strict XUI20
/// compositor; missing or mismatched producer data fails closed here.
pub fn admit_xui20_canvas_route(
    resolver: &AssetResolver,
    frame: &WorldFrame,
    viewer: &Xui20ViewerState,
    expected_content_manifest_sha256: &str,
) -> std::result::Result<Option<Xui20CanvasRoute>, String> {
    let Some(panel) = viewer.panel.as_ref() else {
        return Ok(None);
    };
    if viewer.selected_hour != Some(frame.hour_index()) {
        return Err(format!(
            "XUI20 viewer selection hour {:?} does not match replay hour {}",
            viewer.selected_hour,
            frame.hour_index()
        ));
    }
    if viewer.primary_scroll_px < 0 || viewer.secondary_scroll_px < 0 {
        return Err("XUI20 viewer scroll offsets are negative".into());
    }

    let carrier = frame
        .combat_presentation_exact()
        .presentation
        .as_ref()
        .ok_or_else(|| {
            format!(
                "XUI20 passive presentation carrier is {:?}",
                frame.combat_presentation_exact_status()
            )
        })?;
    if carrier.content_manifest_sha256 != expected_content_manifest_sha256 {
        return Err("XUI20 carrier content manifest does not match playback content".into());
    }
    let presentation = carrier
        .panels
        .iter()
        .find(|candidate| selection_matches(panel, candidate))
        .cloned()
        .ok_or_else(|| "XUI20 selected panel has no exact passive presentation".to_string())?;
    if viewer.primary_scroll_px > presentation.primary_scroll_max_px
        || viewer.secondary_scroll_px > presentation.secondary_scroll_max_px
    {
        return Err("XUI20 viewer scroll exceeds the exact panel range".into());
    }

    let catalog =
        load_xui20_authored_canvas_catalog(resolver).map_err(|error| error.to_string())?;
    let root = catalog.root(panel.root_name()).cloned().ok_or_else(|| {
        format!(
            "XUI20 root {} disappeared after catalog load",
            panel.root_name()
        )
    })?;
    let route = Xui20CanvasRoute {
        panel: panel.clone(),
        root,
        presentation,
        frame_hour: carrier.frame_hour,
        capture_sequence: carrier.capture_sequence,
        record_sha256: carrier.record_sha256.clone(),
        content_manifest_sha256: carrier.content_manifest_sha256.clone(),
    };
    route.require_composable()?;
    Ok(Some(route))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Xui20InputRegion {
    pub panel: ScreenRect,
    pub close: ScreenRect,
    pub primary_scroll: Option<ScreenRect>,
    pub primary_scroll_max_px: i32,
    pub primary_scroll_wheel_px: i32,
    pub secondary_scroll: Option<ScreenRect>,
    pub secondary_scroll_max_px: i32,
    pub secondary_scroll_wheel_px: i32,
}

impl Xui20InputRegion {
    pub fn scroll_channel_at(self, point: (i32, i32)) -> Option<CombatScrollChannelExact> {
        if self
            .secondary_scroll
            .is_some_and(|rect| rect.contains_half_open(point))
        {
            Some(CombatScrollChannelExact::Secondary)
        } else if self
            .primary_scroll
            .is_some_and(|rect| rect.contains_half_open(point))
        {
            Some(CombatScrollChannelExact::Primary)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xui20TooltipHitTarget {
    pub rect: ScreenRect,
    pub payload: TooltipPayload,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xui20RenderedSurface {
    pub output: ComposeOutput,
    pub input_region: Xui20InputRegion,
    pub tooltip_targets: Vec<Xui20TooltipHitTarget>,
    pub used_sources: BTreeSet<String>,
}

#[derive(Clone)]
struct PendingTooltip {
    element_name: String,
    element_path: Vec<u32>,
    primary: replay_core::CombatFinalTextExact,
    delayed: Option<replay_core::CombatFinalTextExact>,
    scroll_clip_channels: Vec<CombatScrollChannelExact>,
}

fn validate_path<'a>(
    root: &'a GuiElement,
    path: &CombatElementPathExact,
) -> Result<&'a GuiElement> {
    let mut current = root;
    for step in path {
        current = current
            .children
            .get(step.child_ordinal as usize)
            .ok_or_else(|| GameUiError::missing("XUI20 authored child ordinal is absent"))?;
        if current.name != step.authored_name {
            return Err(GameUiError::parse(format!(
                "XUI20 authored child identity mismatch: expected {:?}, got {:?}",
                step.authored_name, current.name
            )));
        }
    }
    Ok(current)
}

fn element_mut_by_indices<'a>(
    root: &'a mut GuiElement,
    path: &[u32],
) -> Result<&'a mut GuiElement> {
    let mut current = root;
    for ordinal in path {
        current = current
            .children
            .get_mut(*ordinal as usize)
            .ok_or_else(|| GameUiError::missing("XUI20 materialized child ordinal is absent"))?;
    }
    Ok(current)
}

fn prefix_tree(root: &mut GuiElement, prefix: &str) -> BTreeMap<Vec<u32>, String> {
    fn visit(
        element: &mut GuiElement,
        prefix: &str,
        path: &mut Vec<u32>,
        names: &mut BTreeMap<Vec<u32>, String>,
    ) {
        let suffix = if path.is_empty() {
            "root".into()
        } else {
            path.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join("_")
        };
        element.name = format!("{prefix}_{suffix}");
        names.insert(path.clone(), element.name.clone());
        for (index, child) in element.children.iter_mut().enumerate() {
            path.push(index as u32);
            visit(child, prefix, path, names);
            path.pop();
        }
    }
    let mut names = BTreeMap::new();
    visit(root, prefix, &mut Vec::new(), &mut names);
    names
}

fn path_indices(path: &CombatElementPathExact) -> Vec<u32> {
    path.iter().map(|step| step.child_ordinal).collect()
}

fn apply_bindings(
    bindings: &[replay_core::CombatScopedBindingExact],
    authored: &GuiElement,
    names: &BTreeMap<Vec<u32>, String>,
    selected_tab: Option<replay_core::CombatLogTabExact>,
    scroll_clip_channels: &[CombatScrollChannelExact],
    resolver: Option<&AssetResolver>,
    options: &mut ComposeOptions,
    pending_tooltips: &mut Vec<PendingTooltip>,
) -> Result<()> {
    let active_paths = bindings
        .iter()
        .filter(|scoped| scoped_binding_is_active(scoped, selected_tab))
        .map(|scoped| path_indices(scoped.binding.path()))
        .collect::<BTreeSet<_>>();
    for scoped in bindings {
        let binding = &scoped.binding;
        validate_path(authored, binding.path())?;
        let path = path_indices(binding.path());
        let name = names
            .get(&path)
            .ok_or_else(|| GameUiError::missing("XUI20 prefixed binding path is absent"))?;
        if scoped.combat_log_tab.is_some() && scoped.combat_log_tab != selected_tab {
            if !active_paths.contains(&path) {
                options.hidden_elements.insert(name.clone());
            }
            continue;
        }
        match binding {
            CombatElementBindingExact::Text { value, .. } => {
                options
                    .text_overrides
                    .insert(name.clone(), value.final_text.clone());
            }
            CombatElementBindingExact::Sprite { sprite_name, .. } => {
                options
                    .sprite_name_overrides
                    .insert(name.clone(), sprite_name.clone());
            }
            CombatElementBindingExact::Texture { texture_path, .. } => {
                let resolver = resolver.ok_or_else(|| {
                    GameUiError::missing("XUI20 exact texture binding requires an asset resolver")
                })?;
                let decoded = resolver.decode_texture(texture_path).map_err(|error| {
                    GameUiError::missing(format!(
                        "XUI20 exact texture {texture_path:?} for {name:?} did not decode: {error}"
                    ))
                })?;
                options.sprite_overrides.insert(name.clone(), decoded.image);
            }
            CombatElementBindingExact::Visible { visible, .. } => {
                if !visible {
                    options.hidden_elements.insert(name.clone());
                }
            }
            CombatElementBindingExact::Progress {
                numerator_raw,
                denominator_raw,
                ..
            } => {
                options.progress_overrides.insert(
                    name.clone(),
                    *numerator_raw as f32 / *denominator_raw as f32,
                );
            }
            CombatElementBindingExact::Color { rgba, .. } => {
                options.color_overrides.insert(name.clone(), *rgba);
            }
            CombatElementBindingExact::Frame { frame_index, .. } => {
                options
                    .frame_overrides
                    .insert(name.clone(), *frame_index as usize);
            }
        }
        if let Some(tooltip) = binding.tooltip() {
            pending_tooltips.push(PendingTooltip {
                element_name: name.clone(),
                element_path: path,
                primary: tooltip.primary.clone(),
                delayed: tooltip.delayed.clone(),
                scroll_clip_channels: scroll_clip_channels.to_vec(),
            });
        }
    }
    Ok(())
}

fn debug_rect(output: &ComposeOutput, name: &str) -> Result<ScreenRect> {
    let matches = output
        .debug_rects
        .iter()
        .filter(|rect| rect.name == name)
        .collect::<Vec<&DebugRect>>();
    match matches.as_slice() {
        [rect] if rect.w > 0 && rect.h > 0 => Ok(ScreenRect::new(rect.x, rect.y, rect.w, rect.h)),
        [] => Err(GameUiError::missing(format!(
            "XUI20 composed geometry {name:?} is absent"
        ))),
        _ => Err(GameUiError::parse(format!(
            "XUI20 composed geometry {name:?} is ambiguous"
        ))),
    }
}

fn copy_rect(
    source: &crate::dds::RgbaImage,
    destination: &mut crate::dds::RgbaImage,
    rect: ScreenRect,
) {
    let x0 = rect.x.max(0) as usize;
    let y0 = rect.y.max(0) as usize;
    let x1 = rect.x.saturating_add(rect.width).max(0) as usize;
    let y1 = rect.y.saturating_add(rect.height).max(0) as usize;
    let x1 = x1.min(source.width).min(destination.width);
    let y1 = y1.min(source.height).min(destination.height);
    for y in y0.min(y1)..y1 {
        let source_start = (y * source.width + x0.min(x1)) * 4;
        let source_end = (y * source.width + x1) * 4;
        let destination_start = (y * destination.width + x0.min(x1)) * 4;
        let destination_end = destination_start + source_end.saturating_sub(source_start);
        destination.pixels[destination_start..destination_end]
            .copy_from_slice(&source.pixels[source_start..source_end]);
    }
}

fn scroll_offset(row: &CombatDynamicRowExact, viewer: &Xui20ViewerState) -> i32 {
    match row.scroll_channel {
        CombatScrollChannelExact::None => 0,
        CombatScrollChannelExact::Primary => viewer.primary_scroll_px,
        CombatScrollChannelExact::Secondary => viewer.secondary_scroll_px,
    }
}

fn clip_tooltip_rect(
    mut rect: ScreenRect,
    scroll_clip_channels: &[CombatScrollChannelExact],
    primary_scroll: Option<ScreenRect>,
    secondary_scroll: Option<ScreenRect>,
) -> Result<Option<ScreenRect>> {
    for channel in scroll_clip_channels {
        let viewport = match channel {
            CombatScrollChannelExact::None => continue,
            CombatScrollChannelExact::Primary => primary_scroll.ok_or_else(|| {
                GameUiError::missing("XUI20 primary-scroll tooltip has no viewport")
            })?,
            CombatScrollChannelExact::Secondary => secondary_scroll.ok_or_else(|| {
                GameUiError::missing("XUI20 secondary-scroll tooltip has no viewport")
            })?,
        };
        let Some(clipped) = rect.intersection(viewport) else {
            return Ok(None);
        };
        rect = clipped;
    }
    Ok(Some(rect))
}

fn row_scroll_clip_channels(
    rows: &[CombatDynamicRowExact],
    row_ordinal: usize,
) -> Result<Vec<CombatScrollChannelExact>> {
    let mut current = Some(row_ordinal);
    let mut seen = BTreeSet::new();
    let mut reverse_channels = Vec::new();
    while let Some(ordinal) = current {
        if !seen.insert(ordinal) {
            return Err(GameUiError::parse("XUI20 dynamic row parent cycle"));
        }
        let row = rows
            .get(ordinal)
            .ok_or_else(|| GameUiError::missing("XUI20 dynamic row ancestor is absent"))?;
        if row.scroll_channel != CombatScrollChannelExact::None
            && !reverse_channels.contains(&row.scroll_channel)
        {
            reverse_channels.push(row.scroll_channel);
        }
        current = row.parent_row_ordinal.map(|parent| parent as usize);
    }
    reverse_channels.reverse();
    Ok(reverse_channels)
}

pub fn render_xui20_surface(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    route: &Xui20CanvasRoute,
    viewer: &Xui20ViewerState,
    viewport_width: usize,
    viewport_height: usize,
) -> Result<Xui20RenderedSurface> {
    route.require_composable().map_err(GameUiError::missing)?;
    let catalog = load_xui20_authored_canvas_catalog(resolver)?;
    let mut root = route.root.root.clone();
    let authored_root = root.clone();
    let root_names = prefix_tree(&mut root, "xui20_panel");
    let mut options = ComposeOptions::new(viewport_width, viewport_height);
    let mut pending_tooltips = Vec::new();
    let tab = selected_tab(&route.panel);
    apply_bindings(
        &route.presentation.root_bindings,
        &authored_root,
        &root_names,
        tab,
        &[],
        Some(resolver),
        &mut options,
        &mut pending_tooltips,
    )?;

    let mut row_roots = Vec::<Option<GuiElement>>::new();
    let mut row_authored_roots = Vec::<Option<GuiElement>>::new();
    let mut row_names = Vec::<BTreeMap<Vec<u32>, String>>::new();
    let mut primary_scroll_row_root_names = BTreeSet::new();
    let mut secondary_scroll_row_root_names = BTreeSet::new();
    let mut used_sources = BTreeSet::from([route.root.source_file.clone()]);
    for (row_ordinal, row) in route.presentation.dynamic_rows.iter().enumerate() {
        let template = catalog.template(&row.template_name).ok_or_else(|| {
            GameUiError::missing(format!("XUI20 template {:?}", row.template_name))
        })?;
        if !template.unsupported.is_empty() {
            return Err(GameUiError::parse(format!(
                "XUI20 template {} contains unsupported constructs {:?}",
                template.name, template.unsupported
            )));
        }
        if row.combat_log_tab.is_some() && row.combat_log_tab != tab {
            row_roots.push(None);
            row_authored_roots.push(None);
            row_names.push(BTreeMap::new());
            continue;
        }
        used_sources.insert(template.source_file.clone());
        let mut clone = template.root.clone();
        let authored = clone.clone();
        let names = prefix_tree(&mut clone, &format!("xui20_row_{}", row.source_ordinal));
        let root_name = names
            .get(&Vec::new())
            .cloned()
            .ok_or_else(|| GameUiError::missing("XUI20 row root name"))?;
        set_position_upper_left(
            &mut clone,
            row.position_px[0],
            row.position_px[1].saturating_sub(scroll_offset(row, viewer)),
        );
        apply_bindings(
            &row.bindings,
            &authored,
            &names,
            tab,
            &row_scroll_clip_channels(&route.presentation.dynamic_rows, row_ordinal)?,
            Some(resolver),
            &mut options,
            &mut pending_tooltips,
        )?;
        match row.scroll_channel {
            CombatScrollChannelExact::None => {}
            CombatScrollChannelExact::Primary => {
                primary_scroll_row_root_names.insert(root_name);
            }
            CombatScrollChannelExact::Secondary => {
                secondary_scroll_row_root_names.insert(root_name);
            }
        }
        row_roots.push(Some(clone));
        row_authored_roots.push(Some(authored));
        row_names.push(names);
    }
    for ordinal in (0..route.presentation.dynamic_rows.len()).rev() {
        let Some(clone) = row_roots[ordinal].take() else {
            continue;
        };
        let row = &route.presentation.dynamic_rows[ordinal];
        if let Some(parent) = row.parent_row_ordinal {
            let authored_parent = row_authored_roots
                .get(parent as usize)
                .and_then(Option::as_ref)
                .ok_or_else(|| {
                    GameUiError::missing("XUI20 nested row authored parent is unavailable")
                })?;
            let insertion_ordinal = validate_path(authored_parent, &row.parent_path)?
                .children
                .len();
            let parent_root = row_roots
                .get_mut(parent as usize)
                .and_then(Option::as_mut)
                .ok_or_else(|| GameUiError::missing("XUI20 nested row parent is unavailable"))?;
            element_mut_by_indices(parent_root, &path_indices(&row.parent_path))?
                .children
                .insert(insertion_ordinal, clone);
        } else {
            let insertion_ordinal = validate_path(&authored_root, &row.parent_path)?
                .children
                .len();
            element_mut_by_indices(&mut root, &path_indices(&row.parent_path))?
                .children
                .insert(insertion_ordinal, clone);
        }
    }

    let mut output = Compositor::new(resolver, registry).compose(&root, &options);
    let close_name = root_names
        .get(&path_indices(&route.presentation.close_control_path))
        .ok_or_else(|| GameUiError::missing("XUI20 close-control path"))?;
    let primary_name = route
        .presentation
        .primary_scroll_viewport_path
        .as_ref()
        .and_then(|path| root_names.get(&path_indices(path)));
    let secondary_name = route
        .presentation
        .secondary_scroll_viewport_path
        .as_ref()
        .and_then(|path| root_names.get(&path_indices(path)));
    let primary_scroll = primary_name
        .map(|name| debug_rect(&output, name))
        .transpose()?;
    let secondary_scroll = secondary_name
        .map(|name| debug_rect(&output, name))
        .transpose()?;
    if primary_scroll.is_some_and(|left| {
        secondary_scroll.is_some_and(|right| left.intersection(right).is_some())
    }) {
        return Err(GameUiError::parse(
            "XUI20 primary and secondary scroll viewports overlap",
        ));
    }
    if !primary_scroll_row_root_names.is_empty() && primary_scroll.is_none() {
        return Err(GameUiError::missing(
            "XUI20 primary-scroll rows have no authenticated viewport",
        ));
    }
    if !secondary_scroll_row_root_names.is_empty() && secondary_scroll.is_none() {
        return Err(GameUiError::missing(
            "XUI20 secondary-scroll rows have no authenticated viewport",
        ));
    }
    if !primary_scroll_row_root_names.is_empty() || !secondary_scroll_row_root_names.is_empty() {
        let mut static_options = options.clone();
        static_options
            .hidden_elements
            .extend(primary_scroll_row_root_names.iter().cloned());
        static_options
            .hidden_elements
            .extend(secondary_scroll_row_root_names.iter().cloned());
        let mut clipped = Compositor::new(resolver, registry).compose(&root, &static_options);
        if let Some(rect) = primary_scroll {
            let mut primary_options = options.clone();
            primary_options
                .hidden_elements
                .extend(secondary_scroll_row_root_names.iter().cloned());
            let primary_output =
                Compositor::new(resolver, registry).compose(&root, &primary_options);
            copy_rect(&primary_output.image, &mut clipped.image, rect);
        }
        if let Some(rect) = secondary_scroll {
            let mut secondary_options = options.clone();
            secondary_options
                .hidden_elements
                .extend(primary_scroll_row_root_names.iter().cloned());
            let secondary_output =
                Compositor::new(resolver, registry).compose(&root, &secondary_options);
            copy_rect(&secondary_output.image, &mut clipped.image, rect);
        }
        output.image = clipped.image;
    }
    let tooltip_root_name = authored_tooltip_shell_name(resolver)?;
    let instance_token = format!(
        "xui20:{}:{}:{}",
        route.frame_hour, route.capture_sequence, route.presentation.source_record_identity_sha256
    );
    let mut tooltip_targets = Vec::new();
    for (ordinal, pending) in pending_tooltips.into_iter().enumerate() {
        let rect = debug_rect(&output, &pending.element_name)?;
        let rect = clip_tooltip_rect(
            rect,
            &pending.scroll_clip_channels,
            primary_scroll,
            secondary_scroll,
        )?;
        let Some(rect) = rect else {
            continue;
        };
        let mut rows = vec![TooltipRow {
            source_ordinal: 0,
            line_kind: TooltipLineKind::Description,
            source_identity: pending.primary.authored_identity.clone(),
            evaluation_context: route.presentation.source_record_identity_sha256.clone(),
            availability: RowAvailability::Resolved,
            text: pending.primary.final_text,
            admitted_sources: vec![AdmittedTooltipSource::PacketSemantic {
                packet_id: "XUI20".into(),
                producer_identity: route.presentation.source_record_identity_sha256.clone(),
                field: "final_tooltip_text".into(),
                manifest_sha256: route.content_manifest_sha256.clone(),
            }],
        }];
        if let Some(delayed) = pending.delayed {
            rows.push(TooltipRow {
                source_ordinal: 1,
                line_kind: TooltipLineKind::Breakdown,
                source_identity: delayed.authored_identity,
                evaluation_context: route.presentation.source_record_identity_sha256.clone(),
                availability: RowAvailability::Resolved,
                text: delayed.final_text,
                admitted_sources: vec![AdmittedTooltipSource::PacketSemantic {
                    packet_id: "XUI20".into(),
                    producer_identity: route.presentation.source_record_identity_sha256.clone(),
                    field: "final_delayed_tooltip_text".into(),
                    manifest_sha256: route.content_manifest_sha256.clone(),
                }],
            });
        }
        let payload = TooltipPayload {
            owner: TooltipOwner {
                source_file: route.root.source_file.clone(),
                root_window: route.root.name.clone(),
                element_path: pending.element_path,
                engine_instance_token: format!("{instance_token}:{ordinal}"),
            },
            tooltip_root_name: tooltip_root_name.clone(),
            rows,
        };
        payload.validate()?;
        tooltip_targets.push(Xui20TooltipHitTarget { rect, payload });
    }
    let panel_name = root_names
        .get(&Vec::new())
        .ok_or_else(|| GameUiError::missing("XUI20 panel root name"))?;
    Ok(Xui20RenderedSurface {
        input_region: Xui20InputRegion {
            panel: debug_rect(&output, panel_name)?,
            close: debug_rect(&output, close_name)?,
            primary_scroll,
            primary_scroll_max_px: route.presentation.primary_scroll_max_px,
            primary_scroll_wheel_px: route.presentation.primary_scroll_wheel_px,
            secondary_scroll,
            secondary_scroll_max_px: route.presentation.secondary_scroll_max_px,
            secondary_scroll_wheel_px: route.presentation.secondary_scroll_wheel_px,
        },
        output,
        tooltip_targets,
        used_sources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "game_ui_xui20_{label}_{}_{}",
                std::process::id(),
                NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    fn root(name: &str, child: &str) -> String {
        format!(
            "guiTypes = {{ containerWindowType = {{ name = \"{name}\" size = {{ width = 10 height = 10 }} containerWindowType = {{ name = \"{child}\" size = {{ width = 2 height = 2 }} }} }} }}"
        )
    }

    fn complete_fixture() -> (TestDir, AssetResolver) {
        let temp = TestDir::new("complete");
        let interface = temp.path().join("interface");
        fs::create_dir_all(&interface).unwrap();
        let mut rows = Vec::new();
        for name in XUI20_CANVAS_ROOTS
            .iter()
            .chain(XUI20_DYNAMIC_TEMPLATES.iter())
        {
            rows.push(root(name, "child"));
        }
        fs::write(interface.join("xui20.gui"), rows.join("\n")).unwrap();
        fs::write(
            interface.join("core.gui"),
            "guiTypes = { textBoxType = { name = \"hoi4_tooltip\" textureFile = \"gfx/interface/tooltip.dds\" borderSize = { x = 1 y = 1 } font = \"hoi4_typewriter24\" maxWidth = 100 maxHeight = 100 } }",
        )
        .unwrap();
        let resolver = AssetResolver::new(vec![temp.path().to_path_buf()]);
        (temp, resolver)
    }

    #[test]
    fn catalog_retains_every_root_and_template() {
        let (_vanilla, resolver) = complete_fixture();
        let catalog = load_xui20_authored_canvas_catalog(&resolver).unwrap();
        assert_eq!(catalog.roots().count(), XUI20_CANVAS_ROOTS.len());
        assert_eq!(catalog.templates().count(), XUI20_DYNAMIC_TEMPLATES.len());
        for name in XUI20_CANVAS_ROOTS {
            assert_eq!(
                catalog.root(name).unwrap().source_file,
                "interface/xui20.gui"
            );
        }
    }

    #[test]
    fn moved_mod_definition_wins_by_engine_registry_order() {
        let (vanilla, _resolver) = complete_fixture();
        let mod_root = TestDir::new("override");
        fs::create_dir_all(mod_root.path().join("interface")).unwrap();
        fs::write(
            mod_root.path().join("interface/z_xui20_override.gui"),
            root(LAND_COMBAT_ROOT, "mod_child"),
        )
        .unwrap();
        let resolver = AssetResolver::new(vec![
            mod_root.path().to_path_buf(),
            vanilla.path().to_path_buf(),
        ]);
        let catalog = load_xui20_authored_canvas_catalog(&resolver).unwrap();
        let land = catalog.root(LAND_COMBAT_ROOT).unwrap();
        assert_eq!(
            land.source_file, "interface/z_xui20_override.gui",
            "selection follows effective registry winners, not vanilla filenames"
        );
        assert!(land
            .root
            .children
            .iter()
            .any(|child| child.name == "mod_child"));
    }

    #[test]
    fn missing_required_template_fails_closed() {
        let temp = TestDir::new("missing_template");
        let interface = temp.path().join("interface");
        fs::create_dir_all(&interface).unwrap();
        let roots = XUI20_CANVAS_ROOTS
            .iter()
            .map(|name| root(name, "child"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(interface.join("only_roots.gui"), roots).unwrap();
        let error = load_xui20_authored_canvas_catalog(&AssetResolver::new(vec![temp
            .path()
            .to_path_buf()]))
        .unwrap_err();
        assert!(error.to_string().contains("reserve_combat_entry"));
    }

    #[test]
    fn viewer_scroll_and_seek_reset_are_fail_closed() {
        let mut viewer = Xui20ViewerState::default();
        viewer.open(
            Xui20ViewerPanel::CombatLog {
                country_tag: "ENG".into(),
                tab: Xui20CombatLogTab::Combats,
            },
            91,
        );
        viewer
            .wheel(CombatScrollChannelExact::Primary, -3, 60, 120)
            .unwrap();
        assert_eq!(viewer.primary_scroll_px, 120);
        assert!(viewer
            .wheel(CombatScrollChannelExact::None, 1, 60, 120)
            .is_err());
        viewer.reset_for_seek();
        assert!(viewer.panel.is_none());
        assert_eq!(viewer.selected_hour, None);
        assert_eq!(viewer.primary_scroll_px, 0);
    }

    fn final_text(text: &str) -> replay_core::CombatFinalTextExact {
        replay_core::CombatFinalTextExact {
            authored_identity: format!("test:{text}"),
            final_text: text.into(),
            final_text_sha256: replay_core::final_text_sha256(text),
        }
    }

    fn color_binding(color: [u8; 4], tooltip: &str) -> replay_core::CombatScopedBindingExact {
        replay_core::CombatScopedBindingExact {
            combat_log_tab: None,
            binding: CombatElementBindingExact::Color {
                path: Vec::new(),
                rgba: color,
                tooltip: Some(replay_core::CombatTooltipExact {
                    primary: final_text(tooltip),
                    delayed: None,
                }),
            },
        }
    }

    fn texture_binding(
        tab: Option<replay_core::CombatLogTabExact>,
        texture_path: &str,
    ) -> replay_core::CombatScopedBindingExact {
        replay_core::CombatScopedBindingExact {
            combat_log_tab: tab,
            binding: CombatElementBindingExact::Texture {
                path: Vec::new(),
                texture_path: texture_path.into(),
                tooltip: None,
            },
        }
    }

    fn dynamic_row(
        source_ordinal: u32,
        parent_row_ordinal: Option<u32>,
        position_px: [i32; 2],
        binding: Option<replay_core::CombatScopedBindingExact>,
    ) -> CombatDynamicRowExact {
        let bindings = binding.into_iter().collect::<Vec<_>>();
        CombatDynamicRowExact {
            source_ordinal,
            parent_row_ordinal,
            layout_ordinal: source_ordinal,
            stable_identity_sha256: format!("{:064x}", source_ordinal + 1),
            parent_path: Vec::new(),
            template_name: XUI20_DYNAMIC_TEMPLATES[0].into(),
            effective_template_identity_sha256: "a".repeat(64),
            position_px,
            scroll_channel: CombatScrollChannelExact::None,
            combat_log_tab: None,
            expected_binding_count: bindings.len() as u32,
            bindings,
        }
    }

    #[test]
    fn inactive_combat_log_tabs_cannot_hide_the_selected_binding_at_the_same_path() {
        let (_temp, resolver) = complete_fixture();
        let catalog = load_xui20_authored_canvas_catalog(&resolver).unwrap();
        let authored = catalog.root(COMBAT_LOG_ROOT).unwrap().root.clone();
        let mut prefixed = authored.clone();
        let names = prefix_tree(&mut prefixed, "xui20_tab_test");
        let path = vec![replay_core::CombatElementPathStepExact {
            child_ordinal: 0,
            authored_name: "child".into(),
        }];
        let binding = |tab, text: &str| replay_core::CombatScopedBindingExact {
            combat_log_tab: Some(tab),
            binding: CombatElementBindingExact::Text {
                path: path.clone(),
                value: replay_core::CombatFinalTextExact {
                    authored_identity: format!("tab:{text}"),
                    final_text: text.into(),
                    final_text_sha256: replay_core::final_text_sha256(text),
                },
                tooltip: None,
            },
        };
        let bindings = vec![
            binding(replay_core::CombatLogTabExact::Combats, "combats"),
            binding(replay_core::CombatLogTabExact::Equipment, "equipment"),
            binding(replay_core::CombatLogTabExact::Templates, "templates"),
        ];
        let mut options = ComposeOptions::new(64, 64);
        apply_bindings(
            &bindings,
            &authored,
            &names,
            Some(replay_core::CombatLogTabExact::Equipment),
            &[],
            None,
            &mut options,
            &mut Vec::new(),
        )
        .unwrap();
        let child_name = names.get(&vec![0]).unwrap();
        assert_eq!(options.text_overrides.get(child_name).unwrap(), "equipment");
        assert!(!options.hidden_elements.contains(child_name));

        let mut inactive_only = ComposeOptions::new(64, 64);
        apply_bindings(
            &bindings[..1],
            &authored,
            &names,
            Some(replay_core::CombatLogTabExact::Equipment),
            &[],
            None,
            &mut inactive_only,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(inactive_only.hidden_elements.contains(child_name));
    }

    #[test]
    fn dynamic_source_order_paints_and_hit_tests_later_top_level_and_nested_siblings_last() {
        let (temp, _resolver) = complete_fixture();
        fs::write(
            temp.path().join("interface/z_order.gui"),
            format!(
                "guiTypes = {{\n containerWindowType = {{ name = \"{}\" size = {{ width = 40 height = 20 }} containerWindowType = {{ name = \"close\" size = {{ width = 2 height = 2 }} }} }}\n containerWindowType = {{ name = \"{}\" size = {{ width = 10 height = 10 }} }}\n}}",
                COMBAT_LOG_ROOT, XUI20_DYNAMIC_TEMPLATES[0]
            ),
        )
        .unwrap();
        let resolver = AssetResolver::new(vec![temp.path().to_path_buf()]);
        let catalog = load_xui20_authored_canvas_catalog(&resolver).unwrap();
        let rows = vec![
            dynamic_row(
                0,
                None,
                [0, 0],
                Some(color_binding([255, 0, 0, 255], "top-early")),
            ),
            dynamic_row(
                1,
                None,
                [0, 0],
                Some(color_binding([0, 255, 0, 255], "top-late")),
            ),
            dynamic_row(2, None, [15, 0], None),
            dynamic_row(
                3,
                Some(2),
                [0, 0],
                Some(color_binding([0, 0, 255, 255], "nested-early")),
            ),
            dynamic_row(
                4,
                Some(2),
                [0, 0],
                Some(color_binding([255, 255, 0, 255], "nested-late")),
            ),
        ];
        let route = Xui20CanvasRoute {
            panel: Xui20ViewerPanel::CombatLog {
                country_tag: "ENG".into(),
                tab: Xui20CombatLogTab::Combats,
            },
            root: catalog.root(COMBAT_LOG_ROOT).unwrap().clone(),
            presentation: CombatPanelPresentationExact {
                source_ordinal: 0,
                panel_kind: CombatPanelKindExact::CombatLog,
                owner_country_tag: Some("ENG".into()),
                combat_id: None,
                combat_generation: None,
                source_record_identity_sha256: "5".repeat(64),
                effective_root_identity_sha256: "6".repeat(64),
                close_control_path: vec![replay_core::CombatElementPathStepExact {
                    child_ordinal: 0,
                    authored_name: "close".into(),
                }],
                primary_scroll_viewport_path: None,
                primary_scroll_max_px: 0,
                primary_scroll_wheel_px: 0,
                secondary_scroll_viewport_path: None,
                secondary_scroll_max_px: 0,
                secondary_scroll_wheel_px: 0,
                expected_root_binding_count: 0,
                expected_dynamic_row_count: rows.len() as u32,
                root_bindings: Vec::new(),
                dynamic_rows: rows,
                complete: true,
            },
            frame_hour: 91,
            capture_sequence: 7,
            record_sha256: "7".repeat(64),
            content_manifest_sha256: "8".repeat(64),
        };
        let mut viewer = Xui20ViewerState::default();
        viewer.open(route.panel.clone(), 91);
        let rendered =
            render_xui20_surface(&resolver, &SpriteRegistry::new(), &route, &viewer, 64, 64)
                .unwrap();
        let pixel = |x: usize, y: usize| {
            let offset = (y * rendered.output.image.width + x) * 4;
            rendered.output.image.pixels[offset..offset + 4].to_vec()
        };
        assert_eq!(pixel(1, 1), [0, 255, 0, 255]);
        assert_eq!(pixel(16, 1), [255, 255, 0, 255]);
        let hit = |point| {
            rendered
                .tooltip_targets
                .iter()
                .rev()
                .find(|target| target.rect.contains_half_open(point))
                .unwrap()
                .payload
                .rows()[0]
                .text
                .as_str()
        };
        assert_eq!(hit((1, 1)), "top-late");
        assert_eq!(hit((16, 1)), "nested-late");

        let mut texture_route = route.clone();
        texture_route.presentation.root_bindings = vec![
            texture_binding(None, "gfx/flags/root_active.tga"),
            texture_binding(
                Some(replay_core::CombatLogTabExact::Equipment),
                "gfx/flags/root_inactive.tga",
            ),
        ];
        texture_route.presentation.dynamic_rows[0]
            .bindings
            .push(texture_binding(None, "gfx/flags/row_active.tga"));
        texture_route.presentation.dynamic_rows[1]
            .bindings
            .push(texture_binding(
                Some(replay_core::CombatLogTabExact::Templates),
                "gfx/flags/binding_inactive.tga",
            ));
        texture_route.presentation.dynamic_rows[2].combat_log_tab =
            Some(replay_core::CombatLogTabExact::Equipment);
        texture_route.presentation.dynamic_rows[2]
            .bindings
            .push(texture_binding(None, "gfx/flags/row_inactive.tga"));
        assert_eq!(
            active_texture_paths(&texture_route),
            BTreeSet::from([
                "gfx/flags/root_active.tga".into(),
                "gfx/flags/row_active.tga".into(),
            ])
        );
    }

    #[test]
    fn scroll_tooltip_rects_use_their_own_viewport_and_drop_hidden_geometry() {
        let primary = Some(ScreenRect::new(0, 0, 10, 10));
        let secondary = Some(ScreenRect::new(20, 0, 10, 10));
        assert_eq!(
            clip_tooltip_rect(
                ScreenRect::new(-5, 2, 10, 4),
                &[CombatScrollChannelExact::Primary],
                primary,
                secondary,
            )
            .unwrap(),
            Some(ScreenRect::new(0, 2, 5, 4))
        );
        assert_eq!(
            clip_tooltip_rect(
                ScreenRect::new(25, 8, 10, 6),
                &[CombatScrollChannelExact::Secondary],
                primary,
                secondary,
            )
            .unwrap(),
            Some(ScreenRect::new(25, 8, 5, 2))
        );
        assert_eq!(
            clip_tooltip_rect(
                ScreenRect::new(11, 0, 4, 4),
                &[CombatScrollChannelExact::Primary],
                primary,
                secondary,
            )
            .unwrap(),
            None
        );
        assert_eq!(
            clip_tooltip_rect(
                ScreenRect::new(0, 11, 4, 4),
                &[CombatScrollChannelExact::Secondary],
                primary,
                secondary,
            )
            .unwrap(),
            None
        );
        assert_eq!(
            clip_tooltip_rect(ScreenRect::new(100, 100, 3, 3), &[], primary, secondary,).unwrap(),
            Some(ScreenRect::new(100, 100, 3, 3))
        );
    }

    #[test]
    fn scroll_pixels_are_channel_specific_and_nested_tooltips_inherit_parent_clips() {
        let (temp, _resolver) = complete_fixture();
        fs::write(
            temp.path().join("interface/z_scroll_channels.gui"),
            format!(
                "guiTypes = {{\n containerWindowType = {{ name = \"{}\" size = {{ width = 40 height = 20 }} containerWindowType = {{ name = \"close\" size = {{ width = 2 height = 2 }} }} containerWindowType = {{ name = \"primary\" position = {{ x = 0 y = 0 }} size = {{ width = 10 height = 10 }} }} containerWindowType = {{ name = \"secondary\" position = {{ x = 20 y = 0 }} size = {{ width = 10 height = 10 }} }} }}\n containerWindowType = {{ name = \"{}\" size = {{ width = 30 height = 10 }} }}\n}}",
                COMBAT_LOG_ROOT, XUI20_DYNAMIC_TEMPLATES[0]
            ),
        )
        .unwrap();
        let resolver = AssetResolver::new(vec![temp.path().to_path_buf()]);
        let catalog = load_xui20_authored_canvas_catalog(&resolver).unwrap();
        let mut primary = dynamic_row(
            0,
            None,
            [0, 0],
            Some(color_binding([255, 0, 0, 255], "primary")),
        );
        primary.scroll_channel = CombatScrollChannelExact::Primary;
        let mut secondary = dynamic_row(
            1,
            None,
            [0, 0],
            Some(color_binding([0, 0, 255, 255], "secondary")),
        );
        secondary.scroll_channel = CombatScrollChannelExact::Secondary;
        let mut parent = dynamic_row(2, None, [0, 0], None);
        parent.scroll_channel = CombatScrollChannelExact::Primary;
        let nested = dynamic_row(
            3,
            Some(2),
            [0, 5],
            Some(color_binding([0, 255, 0, 255], "nested-none")),
        );
        let rows = vec![primary, secondary, parent, nested];
        assert_eq!(
            row_scroll_clip_channels(&rows, 3).unwrap(),
            [CombatScrollChannelExact::Primary]
        );
        let mut mixed = rows.clone();
        mixed[3].scroll_channel = CombatScrollChannelExact::Secondary;
        assert_eq!(
            row_scroll_clip_channels(&mixed, 3).unwrap(),
            [
                CombatScrollChannelExact::Primary,
                CombatScrollChannelExact::Secondary,
            ]
        );
        assert_eq!(
            clip_tooltip_rect(
                ScreenRect::new(0, 0, 30, 10),
                &row_scroll_clip_channels(&mixed, 3).unwrap(),
                Some(ScreenRect::new(0, 0, 10, 10)),
                Some(ScreenRect::new(20, 0, 10, 10)),
            )
            .unwrap(),
            None
        );
        let route = Xui20CanvasRoute {
            panel: Xui20ViewerPanel::CombatLog {
                country_tag: "ENG".into(),
                tab: Xui20CombatLogTab::Combats,
            },
            root: catalog.root(COMBAT_LOG_ROOT).unwrap().clone(),
            presentation: CombatPanelPresentationExact {
                source_ordinal: 0,
                panel_kind: CombatPanelKindExact::CombatLog,
                owner_country_tag: Some("ENG".into()),
                combat_id: None,
                combat_generation: None,
                source_record_identity_sha256: "5".repeat(64),
                effective_root_identity_sha256: "6".repeat(64),
                close_control_path: vec![replay_core::CombatElementPathStepExact {
                    child_ordinal: 0,
                    authored_name: "close".into(),
                }],
                primary_scroll_viewport_path: Some(vec![replay_core::CombatElementPathStepExact {
                    child_ordinal: 1,
                    authored_name: "primary".into(),
                }]),
                primary_scroll_max_px: 20,
                primary_scroll_wheel_px: 5,
                secondary_scroll_viewport_path: Some(vec![
                    replay_core::CombatElementPathStepExact {
                        child_ordinal: 2,
                        authored_name: "secondary".into(),
                    },
                ]),
                secondary_scroll_max_px: 20,
                secondary_scroll_wheel_px: 5,
                expected_root_binding_count: 0,
                expected_dynamic_row_count: rows.len() as u32,
                root_bindings: Vec::new(),
                dynamic_rows: rows,
                complete: true,
            },
            frame_hour: 91,
            capture_sequence: 7,
            record_sha256: "7".repeat(64),
            content_manifest_sha256: "8".repeat(64),
        };
        let mut viewer = Xui20ViewerState::default();
        viewer.open(route.panel.clone(), 91);
        let rendered =
            render_xui20_surface(&resolver, &SpriteRegistry::new(), &route, &viewer, 64, 64)
                .unwrap();
        let pixel = |x: usize, y: usize| {
            let offset = (y * rendered.output.image.width + x) * 4;
            rendered.output.image.pixels[offset..offset + 4].to_vec()
        };
        assert_eq!(pixel(1, 1), [255, 0, 0, 255]);
        assert_eq!(pixel(1, 6), [0, 255, 0, 255]);
        assert_eq!(pixel(21, 1), [0, 0, 255, 255]);
        assert_eq!(pixel(21, 6), [0, 0, 255, 255]);
        assert_eq!(pixel(15, 1), [0, 0, 0, 0]);

        let target = |text: &str| {
            rendered
                .tooltip_targets
                .iter()
                .find(|target| target.payload.rows()[0].text == text)
                .unwrap()
                .rect
        };
        assert_eq!(target("primary"), ScreenRect::new(0, 0, 10, 10));
        assert_eq!(target("secondary"), ScreenRect::new(20, 0, 10, 10));
        assert_eq!(target("nested-none"), ScreenRect::new(0, 5, 10, 5));
    }

    #[test]
    fn strict_complete_panel_reaches_pixels_and_exact_input_geometry() {
        let (_temp, resolver) = complete_fixture();
        let catalog = load_xui20_authored_canvas_catalog(&resolver).unwrap();
        let route = Xui20CanvasRoute {
            panel: Xui20ViewerPanel::CombatLog {
                country_tag: "ENG".into(),
                tab: Xui20CombatLogTab::Combats,
            },
            root: catalog.root(COMBAT_LOG_ROOT).unwrap().clone(),
            presentation: CombatPanelPresentationExact {
                source_ordinal: 0,
                panel_kind: CombatPanelKindExact::CombatLog,
                owner_country_tag: Some("ENG".into()),
                combat_id: None,
                combat_generation: None,
                source_record_identity_sha256: "5".repeat(64),
                effective_root_identity_sha256: "6".repeat(64),
                close_control_path: vec![replay_core::CombatElementPathStepExact {
                    child_ordinal: 0,
                    authored_name: "child".into(),
                }],
                primary_scroll_viewport_path: None,
                primary_scroll_max_px: 0,
                primary_scroll_wheel_px: 0,
                secondary_scroll_viewport_path: None,
                secondary_scroll_max_px: 0,
                secondary_scroll_wheel_px: 0,
                expected_root_binding_count: 0,
                expected_dynamic_row_count: 0,
                root_bindings: Vec::new(),
                dynamic_rows: Vec::new(),
                complete: true,
            },
            frame_hour: 91,
            capture_sequence: 7,
            record_sha256: "7".repeat(64),
            content_manifest_sha256: "8".repeat(64),
        };
        let mut viewer = Xui20ViewerState::default();
        viewer.open(route.panel.clone(), 91);
        let rendered =
            render_xui20_surface(&resolver, &SpriteRegistry::new(), &route, &viewer, 64, 64)
                .unwrap();
        assert_eq!(rendered.output.image.width, 64);
        assert_eq!(rendered.output.image.height, 64);
        assert_eq!(rendered.input_region.panel.width, 10);
        assert_eq!(rendered.input_region.close.width, 2);
        assert!(rendered.tooltip_targets.is_empty());
        assert_eq!(
            rendered.used_sources,
            BTreeSet::from(["interface/xui20.gui".into()])
        );
    }

    /// Evidence tier only: the installed game corpus is not a deterministic
    /// default-test prerequisite.
    #[test]
    #[ignore = "requires the installed the game authored GUI corpus"]
    fn installed_vanilla_xui20_roots_and_templates_are_composable_and_all_five_families_render() {
        let game_root = replay_test_gates::require_vanilla_game_root();
        let resolver = AssetResolver::new(vec![game_root]);
        let catalog = load_xui20_authored_canvas_catalog(&resolver)
            .expect("installed effective GUI registry must admit XUI20");
        for name in XUI20_CANVAS_ROOTS {
            let root = catalog
                .root(name)
                .unwrap_or_else(|| panic!("missing XUI20 root {name}"));
            assert!(
                root.unsupported.is_empty(),
                "installed XUI20 root {name} is not composable: {:?}",
                root.unsupported
            );
        }
        for name in XUI20_DYNAMIC_TEMPLATES {
            let template = catalog
                .template(name)
                .unwrap_or_else(|| panic!("missing XUI20 clone template {name}"));
            assert!(
                template.unsupported.is_empty(),
                "installed XUI20 template {name} is not composable: {:?}",
                template.unsupported
            );
        }

        let mut registry = SpriteRegistry::new();
        for file in resolver
            .list_files_recursive("interface", Some("gfx"))
            .expect("installed effective GFX files")
        {
            registry
                .parse_file(&file.path, &file.rel_path)
                .unwrap_or_else(|error| panic!("failed to parse {}: {error}", file.rel_path));
        }
        assert!(
            !registry.is_empty(),
            "installed effective GFX registry is empty"
        );

        fn first_descendant_path(root: &GuiElement) -> CombatElementPathExact {
            fn visit(
                element: &GuiElement,
                path: &mut CombatElementPathExact,
            ) -> Option<CombatElementPathExact> {
                for (ordinal, child) in element.children.iter().enumerate() {
                    path.push(replay_core::CombatElementPathStepExact {
                        child_ordinal: ordinal as u32,
                        authored_name: child.name.clone(),
                    });
                    if !child.name.is_empty() {
                        return Some(path.clone());
                    }
                    if let Some(found) = visit(child, path) {
                        return Some(found);
                    }
                    path.pop();
                }
                None
            }

            visit(root, &mut Vec::new()).expect("installed XUI20 root has a child control")
        }

        fn representative_bindings(
            root: &GuiElement,
            registry: &SpriteRegistry,
        ) -> Vec<replay_core::CombatScopedBindingExact> {
            fn visit(
                element: &GuiElement,
                registry: &SpriteRegistry,
                path: &mut CombatElementPathExact,
                bindings: &mut Vec<replay_core::CombatScopedBindingExact>,
            ) {
                if element.text.as_deref().is_some_and(|text| !text.is_empty()) {
                    let final_text = "XUI20";
                    bindings.push(replay_core::CombatScopedBindingExact {
                        combat_log_tab: None,
                        binding: CombatElementBindingExact::Text {
                            path: path.clone(),
                            value: replay_core::CombatFinalTextExact {
                                authored_identity: format!("installed:{}", element.name),
                                final_text: final_text.into(),
                                final_text_sha256: replay_core::final_text_sha256(final_text),
                            },
                            tooltip: None,
                        },
                    });
                }
                if element.sprite_refs.iter().any(|sprite_name| {
                    registry
                        .get(sprite_name)
                        .is_some_and(|sprite| sprite.primary_texture().is_none())
                }) {
                    // `interface/mapicons.gfx` explicitly declares flag-stripe
                    // sprites without texturefile because engine code selects
                    // the proper country flag. Exercise that exact production
                    // binding seam with an installed effective flag texture.
                    bindings.push(replay_core::CombatScopedBindingExact {
                        combat_log_tab: None,
                        binding: CombatElementBindingExact::Texture {
                            path: path.clone(),
                            texture_path: "gfx/flags/ENG_democratic.tga".into(),
                            tooltip: None,
                        },
                    });
                }
                for (ordinal, child) in element.children.iter().enumerate() {
                    path.push(replay_core::CombatElementPathStepExact {
                        child_ordinal: ordinal as u32,
                        authored_name: child.name.clone(),
                    });
                    visit(child, registry, path, bindings);
                    path.pop();
                }
            }

            let mut bindings = Vec::new();
            visit(root, registry, &mut Vec::new(), &mut bindings);
            bindings
        }

        fn template_family(name: &str) -> CombatPanelKindExact {
            if name.starts_with("naval_combat_results_") {
                CombatPanelKindExact::NavalResults
            } else if name.starts_with("combat_log_") {
                CombatPanelKindExact::CombatLog
            } else if name == "navalcombat_ship_entry" {
                CombatPanelKindExact::NavalCombat
            } else if matches!(
                name,
                "reserve_combat_entry"
                    | "division_combat_defender_entry"
                    | "division_combat_attacker_entry"
                    | "global_modifier_icon_entry"
                    | "modifier_icon_entry"
            ) {
                CombatPanelKindExact::LandCombat
            } else {
                CombatPanelKindExact::NavalLosses
            }
        }

        let content_manifest_sha256 = "a".repeat(64);
        let cases = [
            (
                CombatPanelKindExact::LandCombat,
                Xui20ViewerPanel::LandCombat {
                    combat_id: 11,
                    generation: 1,
                },
                None,
                Some(11),
                Some(1),
            ),
            (
                CombatPanelKindExact::NavalCombat,
                Xui20ViewerPanel::NavalCombat {
                    combat_id: 12,
                    generation: 1,
                },
                None,
                Some(12),
                Some(1),
            ),
            (
                CombatPanelKindExact::CombatLog,
                Xui20ViewerPanel::CombatLog {
                    country_tag: "ENG".into(),
                    tab: Xui20CombatLogTab::Combats,
                },
                Some("ENG".to_string()),
                None,
                None,
            ),
            (
                CombatPanelKindExact::NavalLosses,
                Xui20ViewerPanel::NavalLosses {
                    country_tag: "ENG".into(),
                },
                Some("ENG".to_string()),
                None,
                None,
            ),
            (
                CombatPanelKindExact::NavalResults,
                Xui20ViewerPanel::NavalResults {
                    combat_id: 13,
                    generation: 1,
                },
                None,
                Some(13),
                Some(1),
            ),
        ];
        let mut composed_templates = BTreeSet::new();
        for (
            source_ordinal,
            (panel_kind, panel, owner_country_tag, combat_id, combat_generation),
        ) in cases.into_iter().enumerate()
        {
            let root = catalog
                .root(panel_kind.root_name())
                .expect("installed XUI20 family root")
                .clone();
            let root_bindings = representative_bindings(&root.root, &registry);
            let dynamic_rows = XUI20_DYNAMIC_TEMPLATES
                .iter()
                .copied()
                .filter(|name| template_family(name) == panel_kind)
                .enumerate()
                .map(|(ordinal, name)| {
                    let template = catalog.template(name).expect("installed XUI20 template");
                    composed_templates.insert(name.to_ascii_lowercase());
                    let bindings = representative_bindings(&template.root, &registry);
                    CombatDynamicRowExact {
                        source_ordinal: ordinal as u32,
                        parent_row_ordinal: None,
                        layout_ordinal: ordinal as u32,
                        stable_identity_sha256: replay_core::effective_template_identity_sha256(
                            &content_manifest_sha256,
                            name,
                        ),
                        parent_path: Vec::new(),
                        template_name: name.into(),
                        effective_template_identity_sha256:
                            replay_core::effective_template_identity_sha256(
                                &content_manifest_sha256,
                                name,
                            ),
                        position_px: [0, 0],
                        scroll_channel: CombatScrollChannelExact::None,
                        combat_log_tab: None,
                        expected_binding_count: bindings.len() as u32,
                        bindings,
                    }
                })
                .collect::<Vec<_>>();
            let route = Xui20CanvasRoute {
                panel: panel.clone(),
                presentation: CombatPanelPresentationExact {
                    source_ordinal: source_ordinal as u32,
                    panel_kind,
                    owner_country_tag,
                    combat_id,
                    combat_generation,
                    source_record_identity_sha256: "b".repeat(64),
                    effective_root_identity_sha256: replay_core::effective_root_identity_sha256(
                        &content_manifest_sha256,
                        panel_kind,
                    ),
                    close_control_path: first_descendant_path(&root.root),
                    primary_scroll_viewport_path: None,
                    primary_scroll_max_px: 0,
                    primary_scroll_wheel_px: 0,
                    secondary_scroll_viewport_path: None,
                    secondary_scroll_max_px: 0,
                    secondary_scroll_wheel_px: 0,
                    expected_root_binding_count: root_bindings.len() as u32,
                    expected_dynamic_row_count: dynamic_rows.len() as u32,
                    root_bindings,
                    dynamic_rows,
                    complete: true,
                },
                root,
                frame_hour: 91,
                capture_sequence: 7,
                record_sha256: "c".repeat(64),
                content_manifest_sha256: content_manifest_sha256.clone(),
            };
            let mut viewer = Xui20ViewerState::default();
            viewer.open(panel, 91);
            let rendered = render_xui20_surface(&resolver, &registry, &route, &viewer, 1920, 1080)
                .unwrap_or_else(|error| {
                    panic!(
                        "installed XUI20 {:?} minimal render failed: {error}",
                        panel_kind
                    )
                });
            assert_eq!(rendered.output.image.width, 1920, "{panel_kind:?}");
            assert_eq!(rendered.output.image.height, 1080, "{panel_kind:?}");
            assert!(rendered.input_region.panel.width > 0, "{panel_kind:?}");
            assert!(rendered.input_region.panel.height > 0, "{panel_kind:?}");
            crate::production_renderer::validate_complete_compose_report(&rendered.output.report)
                .unwrap_or_else(|error| {
                    panic!("installed XUI20 {panel_kind:?} shipping completeness failed: {error}")
                });
        }
        assert_eq!(
            composed_templates,
            XUI20_DYNAMIC_TEMPLATES
                .iter()
                .map(|name| name.to_ascii_lowercase())
                .collect(),
            "every XUI20 dynamic template must cross the shipping completeness gate"
        );
    }
}
