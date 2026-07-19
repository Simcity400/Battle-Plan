//! C1 CPU-side construction for on-map land-counter quads.
//!
//! This module consumes the decoded counter asset/layout chain and a
//! renderer-neutral mirror of game_ui binding counter content. It never reads
//! recording bytes. Shipped-file reads are routed through GameFileResolver.
//!
//! Evidence split:
//! - FILE: interface/mapicons.gui:17-159; interface/mapicons.gfx:79-88,
//!   326-334,442-459; interface/unitview.gfx:419-423; and the division/NATO
//!   atlas at interface/subuniticons.gfx:182-231.
//! - DERIVED: natural sprite-frame dimensions, atlas UVs, the 90-degree bar
//!   transform, and fraction clipping are arithmetic consequences of FILE rows.
//! - CAPTURE: the 2026-07-10 whole-frame corpus proves one GUI pixel per GUI
//!   unit over h in [50.0000076, 566.1656494], close-zoom anchor rounding and
//!   offsets at the captured h=50 and h=79.797 bands, close-zoom backing/symbol
//!   frame 0, and a two-counter [0, 24] stack separation.
//! - UNKNOWN-fail-closed: the anchor interpolation between captured bands,
//!   engine stack-slot identity/order, grouped-counter aggregation at h>=90,
//!   and runtime tint/modulation are not promoted. The former FIT stack variant
//!   is retained only as a source-compatibility fail-closed sentinel.

use crate::{
    map_counter_sprites_from_resolver, on_map_division_counter_inventory_from_resolver,
    unit_counter_gui_layouts_from_resolver, unitview_counter_sprites_from_resolver,
    CounterSpriteAsset, GameFileResolver, GuiContainerLayout, GuiControlLayout, GuiPoint, Result,
    UvRect,
};
use std::collections::{BTreeMap, BTreeSet};

/// FILE: interface/mapicons.gfx:79-83.
pub const COUNTER_FRAME_SPRITE: &str = "GFX_onmap_unit_counter";
/// FILE: interface/mapicons.gfx:326-330.
pub const COUNTER_ORG_BAR_SPRITE: &str = "GFX_mapicon_unit_bar_org";
/// FILE: interface/mapicons.gfx:331-334.
pub const COUNTER_STRENGTH_BAR_SPRITE: &str = "GFX_mapicon_unit_bar_str";
/// FILE: interface/mapicons.gui:86,93.
pub const COUNTER_BAR_ROTATION_RADIANS_FILE: f32 = 1.5708;
/// FILE: interface/mapicons.gui:189-198.
pub const ACTIVE_ABILITY_ITEM_SIZE_GUI_FILE: [f32; 2] = [20.0, 20.0];
/// FILE: interface/mapicons.gui:193-197.
pub const ACTIVE_ABILITY_ICON_SCALE_FILE: f32 = 0.8;

/// CAPTURE: lower/upper camera heights for the repeated h=50 close-zoom anchor
/// band. `hoi4_frame35240` chunk 28061 and `hoi4_frame61841` chunk 14602 use
/// the same GUI transform and resolve to anchor-to-origin `[-38, 39]` pixels.
pub const COUNTER_CAPTURE_H50_MIN: f32 = 50.000_008;
pub const COUNTER_CAPTURE_H50_MAX: f32 = 50.115_116;
/// CAPTURE: lower/upper camera heights for the repeated war-cluster band.
/// `hoi4_frame76017` chunk 24055 through `hoi4_frame69242` chunk 31494 resolve
/// to anchor-to-origin `[-38, 28]` pixels.
pub const COUNTER_CAPTURE_H79_MIN: f32 = 79.796_92;
pub const COUNTER_CAPTURE_H79_MAX: f32 = 79.797_05;
/// CAPTURE: all sampled counter GUI transforms from h=50.0000076 through
/// h=566.1656494 use 2/1920 and 2/1080 orthographic diagonals with unchanged
/// 66x27 / 30x12 VBs, hence one screen pixel per GUI unit.
pub const COUNTER_CAPTURE_PIXELS_PER_GUI_UNIT: f32 = 1.0;
/// CAPTURE: the two visible close-zoom stack pairs are separated by 24 pixels:
/// `hoi4_frame69242` chunks 33696/33815 and `hoi4_frame69592` equivalents.
pub const COUNTER_CAPTURE_STACK_STEP_GUI: [f32; 2] = [0.0, 24.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CounterHonesty {
    File,
    Re,
    Capture,
    Derived,
    Fit,
}

/// Only Land is constructible in this wave. Naval and air frame/symbol laws
/// remain UNKNOWN-fail-closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CounterRenderDomain {
    Land,
    Naval,
    Air,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPointPx {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CounterBadgeSelection {
    pub kind: CounterBadgeKind,
    pub frame_index: u32,
    pub evidence: CounterHonesty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CounterFrameSelection {
    pub frame_index: u32,
    pub evidence: CounterHonesty,
}

/// FILE slots: interface/mapicons.gui:57-63,112-159.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CounterBadgeKind {
    NoOrders,
    AreaDefense,
    InvalidOrders,
    LowSupply,
    DigIn,
    Training,
    Action,
    Attrition,
    ActiveAbility,
}

impl CounterBadgeKind {
    fn control_name(self) -> &'static str {
        match self {
            Self::NoOrders => "no_orders",
            Self::AreaDefense => "defending_area",
            Self::InvalidOrders => "invalid_orders",
            Self::LowSupply => "low_supply",
            Self::DigIn => "onmap_digin_icon",
            Self::Training => "onmap_training_icon",
            Self::Action => "onmap_action_icon",
            Self::Attrition => "attrition",
            Self::ActiveAbility => "active_abilities",
        }
    }

    fn sprite_name(self) -> &'static str {
        match self {
            Self::NoOrders => "GFX_mapicon_unit_no_orders",
            Self::AreaDefense => "GFX_mapicon_unit_area_defense",
            Self::InvalidOrders => "GFX_mapicon_unit_invalid_orders",
            Self::LowSupply => "GFX_unitcounter_supply_status_strip",
            Self::DigIn => "GFX_onmap_digin_icon",
            Self::Training => "GFX_onmap_training_icon",
            Self::Action => "GFX_onmap_action_icon",
            Self::Attrition => "GFX_unit_attrition",
            Self::ActiveAbility => "GFX_onmap_unit_counter_active_ability",
        }
    }
}

/// Renderer-neutral mirror of bound content plus placement/camera results.
#[derive(Clone, Debug, PartialEq)]
pub struct CounterRenderInput {
    pub unit_id: String,
    pub province_id: u32,
    pub domain: CounterRenderDomain,
    pub visible: bool,
    /// UNKNOWN-fail-closed when absent.
    pub screen_anchor_px: Option<ScreenPointPx>,
    /// Three FILE backing frames exist, but runtime meaning is CAP6-gated.
    pub counter_frame: Option<CounterFrameSelection>,
    pub nato_symbol_key: Option<String>,
    /// The shipped two-frame symbol meaning is not established; no default.
    pub nato_symbol_frame: Option<CounterFrameSelection>,
    pub strength_fraction: Option<f32>,
    pub organisation_fraction: Option<f32>,
    pub badges: Vec<CounterBadgeSelection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterColorMode {
    Countries,
    Allegiance,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CounterPlayerMapping {
    pub player_country_tags: BTreeSet<String>,
    pub unit_owner_tags: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterDisplayOmissionReason {
    Item4PlayerCountryMappingUnavailable,
    UnitOwnerUnavailable,
    Cap6CounterColorModeUnavailable,
    Cap6CountriesTintFrameUnavailable,
    Cap6AllegianceTintFrameUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CounterDisplayOmission {
    pub unit_id: String,
    pub reason: CounterDisplayOmissionReason,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CounterDisplaySelection {
    pub inputs: Vec<CounterRenderInput>,
    pub gaps: Vec<CounterDisplayOmissionReason>,
    pub omissions: Vec<CounterDisplayOmission>,
}

/// Narrow product seam for the two counter display controls. The shipped
/// colour toggle means Countries versus Allegiance; both tint/frame branches
/// remain CAP6-blocked. Player-only mode additionally requires the item-4
/// player-country mapping and a measured owner for every admitted unit.
pub fn select_counter_display_inputs(
    inputs: &[CounterRenderInput],
    player_counters_only: bool,
    color_mode: Option<CounterColorMode>,
    player_mapping: Option<&CounterPlayerMapping>,
) -> CounterDisplaySelection {
    let mut selection = if !player_counters_only {
        CounterDisplaySelection {
            inputs: inputs.to_vec(),
            gaps: Vec::new(),
            omissions: Vec::new(),
        }
    } else if let Some(player_mapping) = player_mapping {
        let mut selection = CounterDisplaySelection::default();
        for input in inputs.iter().filter(|input| input.visible) {
            let Some(owner_tag) = player_mapping.unit_owner_tags.get(&input.unit_id) else {
                selection.omissions.push(CounterDisplayOmission {
                    unit_id: input.unit_id.clone(),
                    reason: CounterDisplayOmissionReason::UnitOwnerUnavailable,
                });
                continue;
            };
            if player_mapping.player_country_tags.contains(owner_tag) {
                selection.inputs.push(input.clone());
            }
        }
        selection
    } else {
        CounterDisplaySelection {
            inputs: Vec::new(),
            gaps: vec![CounterDisplayOmissionReason::Item4PlayerCountryMappingUnavailable],
            omissions: inputs
                .iter()
                .filter(|input| input.visible)
                .map(|input| CounterDisplayOmission {
                    unit_id: input.unit_id.clone(),
                    reason: CounterDisplayOmissionReason::Item4PlayerCountryMappingUnavailable,
                })
                .collect(),
        }
    };

    let color_gap = match color_mode {
        None => CounterDisplayOmissionReason::Cap6CounterColorModeUnavailable,
        Some(CounterColorMode::Countries) => {
            CounterDisplayOmissionReason::Cap6CountriesTintFrameUnavailable
        }
        Some(CounterColorMode::Allegiance) => {
            CounterDisplayOmissionReason::Cap6AllegianceTintFrameUnavailable
        }
    };
    selection.gaps.push(color_gap);
    let mut color_candidates = selection
        .inputs
        .iter()
        .filter(|input| input.visible)
        .map(|input| input.unit_id.clone())
        .collect::<Vec<_>>();
    if player_counters_only {
        color_candidates.extend(
            selection
                .omissions
                .iter()
                .filter(|row| {
                    matches!(
                        row.reason,
                        CounterDisplayOmissionReason::Item4PlayerCountryMappingUnavailable
                            | CounterDisplayOmissionReason::UnitOwnerUnavailable
                    )
                })
                .map(|row| row.unit_id.clone()),
        );
    }
    selection
        .omissions
        .extend(
            color_candidates
                .into_iter()
                .map(|unit_id| CounterDisplayOmission {
                    unit_id,
                    reason: color_gap,
                }),
        );
    selection.inputs.clear();
    selection
}

/// CAP6-owned screen law. FIT evidence is rejected for size/scale-vs-zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CounterScreenLaw {
    UnknownFailClosed,
    Evidenced {
        pixels_per_gui_unit: f32,
        anchor_to_origin_gui: [f32; 2],
        round_anchor_to_pixel: bool,
        default_counter_frame: Option<CounterFrameSelection>,
        default_nato_symbol_frame: Option<CounterFrameSelection>,
        evidence: CounterHonesty,
    },
}

impl CounterScreenLaw {
    pub fn evidenced(
        pixels_per_gui_unit: f32,
        anchor_to_origin_gui: [f32; 2],
        evidence: CounterHonesty,
    ) -> Result<Self> {
        if evidence == CounterHonesty::Fit {
            return Err("counter screen size/scale cannot use FIT evidence".into());
        }
        if !pixels_per_gui_unit.is_finite() || pixels_per_gui_unit <= 0.0 {
            return Err("counter pixels_per_gui_unit must be finite and positive".into());
        }
        if !anchor_to_origin_gui.iter().all(|value| value.is_finite()) {
            return Err("counter anchor_to_origin_gui must be finite".into());
        }
        Ok(Self::Evidenced {
            pixels_per_gui_unit,
            anchor_to_origin_gui,
            round_anchor_to_pixel: false,
            default_counter_frame: None,
            default_nato_symbol_frame: None,
            evidence,
        })
    }

    /// Public CAP6 handoff without widening the crate's root export surface.
    pub fn from_capture_close_zoom(assets: &CounterRenderAssets, camera_height: f32) -> Self {
        captured_counter_screen_law(assets, camera_height)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CounterStackingLaw {
    UnknownFailClosed,
    /// Legacy source-compatibility sentinel. The former FIT geometry is retired
    /// and this variant now fails closed for every stack index.
    FitVerticalByDeclaredContainerHeight,
    EvidencedOffsets {
        offsets_gui: Vec<[f32; 2]>,
        evidence: CounterHonesty,
    },
}

impl CounterStackingLaw {
    /// Public CAP6 handoff paired with [`CounterScreenLaw::from_capture_close_zoom`].
    pub fn from_capture_close_zoom(assets: &CounterRenderAssets, camera_height: f32) -> Self {
        captured_counter_stacking_law(assets, camera_height)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CounterRenderAssets {
    pub frame: CounterSpriteAsset,
    pub organisation_bar: CounterSpriteAsset,
    pub strength_bar: CounterSpriteAsset,
    pub land_symbols: BTreeMap<String, CounterSpriteAsset>,
    pub unresolved_land_symbol_keys: BTreeSet<String>,
    pub badge_sprites: BTreeMap<String, CounterSpriteAsset>,
    pub land_layout: GuiContainerLayout,
    pub zoomed_in_offset: GuiPoint,
}

/// FILE parser entrypoint. Every read goes through the active resolver.
pub fn counter_render_assets_from_resolver(
    resolver: &GameFileResolver,
) -> Result<CounterRenderAssets> {
    let mut map_sprites = map_counter_sprites_from_resolver(resolver)?;
    let mut unitview_sprites = unitview_counter_sprites_from_resolver(resolver)?;
    let division_inventory = on_map_division_counter_inventory_from_resolver(resolver)?;
    let layouts = unit_counter_gui_layouts_from_resolver(resolver)?;
    let zoomed_in_offset = layouts.zoomed_in_offset;

    let frame = map_sprites
        .remove(COUNTER_FRAME_SPRITE)
        .ok_or("missing FILE counter backing sprite")?;
    let organisation_bar = map_sprites
        .remove(COUNTER_ORG_BAR_SPRITE)
        .ok_or("missing FILE counter organisation-bar sprite")?;
    let strength_bar = map_sprites
        .remove(COUNTER_STRENGTH_BAR_SPRITE)
        .ok_or("missing FILE counter strength-bar sprite")?;

    let mut badge_sprites = BTreeMap::new();
    for kind in [
        CounterBadgeKind::NoOrders,
        CounterBadgeKind::AreaDefense,
        CounterBadgeKind::InvalidOrders,
        CounterBadgeKind::LowSupply,
        CounterBadgeKind::DigIn,
        CounterBadgeKind::Training,
        CounterBadgeKind::Action,
        CounterBadgeKind::Attrition,
        CounterBadgeKind::ActiveAbility,
    ] {
        let name = kind.sprite_name();
        if let Some(asset) = map_sprites
            .remove(name)
            .or_else(|| unitview_sprites.remove(name))
        {
            badge_sprites.insert(name.to_string(), asset);
        }
    }

    Ok(CounterRenderAssets {
        frame,
        organisation_bar,
        strength_bar,
        land_symbols: division_inventory
            .resolved
            .into_iter()
            .map(|asset| (asset.name.clone(), asset))
            .collect(),
        unresolved_land_symbol_keys: division_inventory
            .unresolved
            .into_iter()
            .map(|asset| asset.name)
            .collect(),
        badge_sprites,
        land_layout: layouts.land,
        zoomed_in_offset,
    })
}

/// CAPTURE close-zoom screen law, gated to the two actually observed anchor
/// bands and the resolver-loaded FILE geometry used by the capture. Heights in
/// the unsampled gaps, changed mod layouts, and non-finite inputs fail closed.
pub fn captured_counter_screen_law(
    assets: &CounterRenderAssets,
    camera_height: f32,
) -> CounterScreenLaw {
    let Some(layout_size) = assets.land_layout.size else {
        return CounterScreenLaw::UnknownFailClosed;
    };
    if layout_size.width != 76
        || layout_size.height != 24
        || assets.zoomed_in_offset != (GuiPoint { x: 0, y: 20 })
        || assets.frame.frame_width != 66
        || assets.frame.frame_height != 27
        || !camera_height.is_finite()
    {
        return CounterScreenLaw::UnknownFailClosed;
    }

    let anchor_y = if (COUNTER_CAPTURE_H50_MIN..=COUNTER_CAPTURE_H50_MAX).contains(&camera_height) {
        39.0
    } else if (COUNTER_CAPTURE_H79_MIN..=COUNTER_CAPTURE_H79_MAX).contains(&camera_height) {
        28.0
    } else {
        return CounterScreenLaw::UnknownFailClosed;
    };

    CounterScreenLaw::Evidenced {
        pixels_per_gui_unit: COUNTER_CAPTURE_PIXELS_PER_GUI_UNIT,
        // FILE/DERIVED x: half of the resolver-loaded 76-unit container width.
        // CAPTURE y: exact world-anchor projection / GUI-CB comparison.
        anchor_to_origin_gui: [-(layout_size.width as f32) * 0.5, anchor_y],
        // CAPTURE: all 35 h=79.797 type-0 matches first round the projected
        // anchor to a pixel, then apply the integer offset.
        round_anchor_to_pixel: true,
        // CAPTURE: close-zoom backing and every observed division strip use
        // frame zero; allegiance is carried separately by GUI modulation.
        default_counter_frame: Some(CounterFrameSelection {
            frame_index: 0,
            evidence: CounterHonesty::Capture,
        }),
        default_nato_symbol_frame: Some(CounterFrameSelection {
            frame_index: 0,
            evidence: CounterHonesty::Capture,
        }),
        evidence: CounterHonesty::Capture,
    }
}

/// CAPTURE close-zoom stack-offset law. Only two slots were observed; index 2
/// and above fail closed rather than extrapolating the 24-pixel step.
pub fn captured_counter_stacking_law(
    assets: &CounterRenderAssets,
    camera_height: f32,
) -> CounterStackingLaw {
    if matches!(
        captured_counter_screen_law(assets, camera_height),
        CounterScreenLaw::UnknownFailClosed
    ) {
        return CounterStackingLaw::UnknownFailClosed;
    }
    CounterStackingLaw::EvidencedOffsets {
        offsets_gui: vec![[0.0, 0.0], COUNTER_CAPTURE_STACK_STEP_GUI],
        evidence: CounterHonesty::Capture,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CounterQuadLayer {
    Frame,
    NatoSymbol,
    OrganisationBar,
    StrengthBar,
    Badge(CounterBadgeKind),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CounterQuadVertex {
    pub position_px: ScreenPointPx,
    pub uv: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct CounterQuad {
    pub layer: CounterQuadLayer,
    pub texture_file: String,
    pub source_file: String,
    pub source_line: usize,
    pub frame_index: u32,
    pub selection_honesty: CounterHonesty,
    /// Clockwise: top-left, top-right, bottom-right, bottom-left.
    pub vertices: [CounterQuadVertex; 4],
    pub geometry_honesty: CounterHonesty,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CounterRenderInstance {
    pub unit_id: String,
    pub province_id: u32,
    pub stack_index: usize,
    pub stack_count: usize,
    pub origin_px: ScreenPointPx,
    pub stack_offset_gui: [f32; 2],
    pub stacking_honesty: CounterHonesty,
    pub quads: Vec<CounterQuad>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CounterRenderOmissionReason {
    ScreenGeometryUnknown,
    ScreenAnchorUnknown,
    UnsupportedDomain,
    StackOffsetUnknown,
    CounterFrameSelectionUnknown,
    CounterFrameOutOfRange,
    NatoSymbolKeyUnknown,
    NatoSymbolFrameSelectionUnknown,
    NatoSymbolAssetUnavailable,
    NatoSymbolFrameOutOfRange,
    BarFractionUnknown(CounterQuadLayer),
    BarFractionOutOfRange(CounterQuadLayer),
    LayoutControlUnavailable(&'static str),
    BadgeAssetUnavailable(CounterBadgeKind),
    BadgeSelectionUnknown(CounterBadgeKind),
    BadgeFrameOutOfRange(CounterBadgeKind),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CounterRenderOmission {
    pub unit_id: String,
    pub reason: CounterRenderOmissionReason,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CounterRenderBatch {
    pub instances: Vec<CounterRenderInstance>,
    pub omissions: Vec<CounterRenderOmission>,
}

/// Resolver-decoded counter texture payload for the replay-app GPU handoff.
///
/// The path remains the normalized FILE identity carried by [`CounterQuad`];
/// callers cannot substitute arbitrary bytes. Resolution goes through the
/// active effective-VFS resolver and DDS decoding is shared with the map
/// renderer rather than duplicated in the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CounterTextureRgba {
    pub texture_file: String,
    pub width: usize,
    pub height: usize,
    pub rgba8: Vec<u8>,
}

pub fn counter_texture_rgba_from_resolver(
    resolver: &GameFileResolver,
    texture_file: &str,
) -> Result<CounterTextureRgba> {
    if !resolver.has_content_admission() {
        return Err(
            "counter GPU texture resolution requires an admission-bound effective VFS".into(),
        );
    }
    let path = resolver.resolve_required(texture_file, "counter GPU texture")?;
    let image = crate::image_load::load_dds_rgba(&path)?;
    Ok(CounterTextureRgba {
        texture_file: texture_file.to_string(),
        width: image.width,
        height: image.height,
        rgba8: image.pixels,
    })
}

/// Deterministic CPU construction. Visible inputs are grouped by province and
/// domain, then sorted by stable unit id before stack indices are assigned.
pub fn build_counter_instances(
    assets: &CounterRenderAssets,
    inputs: &[CounterRenderInput],
    screen_law: CounterScreenLaw,
    stacking_law: &CounterStackingLaw,
) -> CounterRenderBatch {
    let CounterScreenLaw::Evidenced {
        pixels_per_gui_unit,
        anchor_to_origin_gui,
        round_anchor_to_pixel,
        default_counter_frame,
        default_nato_symbol_frame,
        evidence,
    } = screen_law
    else {
        return CounterRenderBatch {
            instances: Vec::new(),
            omissions: inputs
                .iter()
                .filter(|input| input.visible)
                .map(|input| omission(input, CounterRenderOmissionReason::ScreenGeometryUnknown))
                .collect(),
        };
    };
    if evidence == CounterHonesty::Fit
        || !pixels_per_gui_unit.is_finite()
        || pixels_per_gui_unit <= 0.0
        || !anchor_to_origin_gui.iter().all(|value| value.is_finite())
    {
        return CounterRenderBatch {
            instances: Vec::new(),
            omissions: inputs
                .iter()
                .filter(|input| input.visible)
                .map(|input| omission(input, CounterRenderOmissionReason::ScreenGeometryUnknown))
                .collect(),
        };
    }

    let mut grouped = BTreeMap::<(u32, CounterRenderDomain), Vec<&CounterRenderInput>>::new();
    for input in inputs.iter().filter(|input| input.visible) {
        grouped
            .entry((input.province_id, input.domain))
            .or_default()
            .push(input);
    }
    for group in grouped.values_mut() {
        group.sort_by(|left, right| left.unit_id.cmp(&right.unit_id));
    }

    let mut batch = CounterRenderBatch::default();
    for group in grouped.into_values() {
        let stack_count = group.len();
        for (stack_index, input) in group.into_iter().enumerate() {
            if input.domain != CounterRenderDomain::Land {
                omit(
                    &mut batch,
                    input,
                    CounterRenderOmissionReason::UnsupportedDomain,
                );
                continue;
            }
            let Some(anchor) = input.screen_anchor_px else {
                omit(
                    &mut batch,
                    input,
                    CounterRenderOmissionReason::ScreenAnchorUnknown,
                );
                continue;
            };
            let Some((stack_offset_gui, stacking_honesty)) =
                stack_offset(assets, stacking_law, stack_index)
            else {
                omit(
                    &mut batch,
                    input,
                    CounterRenderOmissionReason::StackOffsetUnknown,
                );
                continue;
            };
            let anchor = if round_anchor_to_pixel {
                ScreenPointPx {
                    x: anchor.x.round(),
                    y: anchor.y.round(),
                }
            } else {
                anchor
            };
            let origin_px = ScreenPointPx {
                x: anchor.x + (anchor_to_origin_gui[0] + stack_offset_gui[0]) * pixels_per_gui_unit,
                y: anchor.y + (anchor_to_origin_gui[1] + stack_offset_gui[1]) * pixels_per_gui_unit,
            };
            let mut instance = CounterRenderInstance {
                unit_id: input.unit_id.clone(),
                province_id: input.province_id,
                stack_index,
                stack_count,
                origin_px,
                stack_offset_gui,
                stacking_honesty,
                quads: Vec::new(),
            };
            build_frame(
                assets,
                input,
                default_counter_frame,
                origin_px,
                pixels_per_gui_unit,
                &mut instance,
                &mut batch.omissions,
            );
            build_symbol(
                assets,
                input,
                default_nato_symbol_frame,
                origin_px,
                pixels_per_gui_unit,
                &mut instance,
                &mut batch.omissions,
            );
            build_bar(
                assets,
                input,
                origin_px,
                pixels_per_gui_unit,
                CounterQuadLayer::OrganisationBar,
                input.organisation_fraction,
                &assets.organisation_bar,
                "bar_org",
                &mut instance,
                &mut batch.omissions,
            );
            build_bar(
                assets,
                input,
                origin_px,
                pixels_per_gui_unit,
                CounterQuadLayer::StrengthBar,
                input.strength_fraction,
                &assets.strength_bar,
                "bar_str",
                &mut instance,
                &mut batch.omissions,
            );
            build_badges(
                assets,
                input,
                origin_px,
                pixels_per_gui_unit,
                &mut instance,
                &mut batch.omissions,
            );
            batch.instances.push(instance);
        }
    }
    batch
}

fn stack_offset(
    _assets: &CounterRenderAssets,
    law: &CounterStackingLaw,
    stack_index: usize,
) -> Option<([f32; 2], CounterHonesty)> {
    match law {
        CounterStackingLaw::UnknownFailClosed
        | CounterStackingLaw::FitVerticalByDeclaredContainerHeight => None,
        CounterStackingLaw::EvidencedOffsets {
            offsets_gui,
            evidence,
        } if *evidence != CounterHonesty::Fit => {
            offsets_gui.get(stack_index).copied().and_then(|offset| {
                offset
                    .iter()
                    .all(|value| value.is_finite())
                    .then_some((offset, *evidence))
            })
        }
        CounterStackingLaw::EvidencedOffsets { .. } => None,
    }
}

fn build_frame(
    assets: &CounterRenderAssets,
    input: &CounterRenderInput,
    default_selection: Option<CounterFrameSelection>,
    origin_px: ScreenPointPx,
    scale: f32,
    instance: &mut CounterRenderInstance,
    omissions: &mut Vec<CounterRenderOmission>,
) {
    let Some(selection) = input.counter_frame.or(default_selection) else {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::CounterFrameSelectionUnknown,
        ));
        return;
    };
    if selection.evidence == CounterHonesty::Fit {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::CounterFrameSelectionUnknown,
        ));
        return;
    }
    let Some(frame) = assets.frame.frame(selection.frame_index) else {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::CounterFrameOutOfRange,
        ));
        return;
    };
    let Some(position) = assets
        .land_layout
        .control("count_bg")
        .and_then(|control| control.position)
    else {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::LayoutControlUnavailable("count_bg.position"),
        ));
        return;
    };
    instance.quads.push(axis_aligned_quad(
        CounterQuadLayer::Frame,
        &assets.frame,
        selection.frame_index,
        selection.evidence,
        frame.uv,
        origin_px,
        [position.x as f32, position.y as f32],
        [frame.rect.width as f32, frame.rect.height as f32],
        scale,
    ));
}

fn build_symbol(
    assets: &CounterRenderAssets,
    input: &CounterRenderInput,
    default_selection: Option<CounterFrameSelection>,
    origin_px: ScreenPointPx,
    scale: f32,
    instance: &mut CounterRenderInstance,
    omissions: &mut Vec<CounterRenderOmission>,
) {
    let Some(key) = input.nato_symbol_key.as_deref() else {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::NatoSymbolKeyUnknown,
        ));
        return;
    };
    let Some(selection) = input.nato_symbol_frame.or(default_selection) else {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::NatoSymbolFrameSelectionUnknown,
        ));
        return;
    };
    if selection.evidence == CounterHonesty::Fit {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::NatoSymbolFrameSelectionUnknown,
        ));
        return;
    }
    let Some(asset) = assets.land_symbols.get(key) else {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::NatoSymbolAssetUnavailable,
        ));
        return;
    };
    let Some(frame) = asset.frame(selection.frame_index) else {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::NatoSymbolFrameOutOfRange,
        ));
        return;
    };
    let Some(position) = assets
        .land_layout
        .control("type")
        .and_then(|control| control.position)
    else {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::LayoutControlUnavailable("type.position"),
        ));
        return;
    };
    instance.quads.push(axis_aligned_quad(
        CounterQuadLayer::NatoSymbol,
        asset,
        selection.frame_index,
        selection.evidence,
        frame.uv,
        origin_px,
        [position.x as f32, position.y as f32],
        [frame.rect.width as f32, frame.rect.height as f32],
        scale,
    ));
}

#[allow(clippy::too_many_arguments)]
fn build_bar(
    assets: &CounterRenderAssets,
    input: &CounterRenderInput,
    origin_px: ScreenPointPx,
    scale: f32,
    layer: CounterQuadLayer,
    fraction: Option<f32>,
    asset: &CounterSpriteAsset,
    control_name: &'static str,
    instance: &mut CounterRenderInstance,
    omissions: &mut Vec<CounterRenderOmission>,
) {
    let Some(fraction) = fraction else {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::BarFractionUnknown(layer),
        ));
        return;
    };
    if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::BarFractionOutOfRange(layer),
        ));
        return;
    }
    let Some(control) = assets.land_layout.control(control_name) else {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::LayoutControlUnavailable(control_name),
        ));
        return;
    };
    let Some(position) = control.position else {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::LayoutControlUnavailable(control_name),
        ));
        return;
    };
    if control.rotation != Some(COUNTER_BAR_ROTATION_RADIANS_FILE) {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::LayoutControlUnavailable(control_name),
        ));
        return;
    }
    let Some(frame) = asset.frame(0) else {
        omissions.push(omission(
            input,
            CounterRenderOmissionReason::LayoutControlUnavailable(control_name),
        ));
        return;
    };

    // DERIVED from FILE: clockwise quarter-turn maps 3x17 to 17x3. Cropping
    // visible width maps to the source V axis.
    let cropped_v = frame.uv.v1 - (frame.uv.v1 - frame.uv.v0) * fraction;
    let local = [position.x as f32, position.y as f32];
    let size = [frame.rect.height as f32 * fraction, frame.rect.width as f32];
    let tl = screen_point(origin_px, local, scale);
    let tr = ScreenPointPx {
        x: tl.x + size[0] * scale,
        y: tl.y,
    };
    let br = ScreenPointPx {
        x: tr.x,
        y: tl.y + size[1] * scale,
    };
    let bl = ScreenPointPx { x: tl.x, y: br.y };
    instance.quads.push(CounterQuad {
        layer,
        texture_file: asset.texture_file.clone(),
        source_file: asset.source_file.clone(),
        source_line: asset.source_line,
        frame_index: 0,
        selection_honesty: CounterHonesty::File,
        vertices: [
            CounterQuadVertex {
                position_px: tl,
                uv: [frame.uv.u0, frame.uv.v1],
            },
            CounterQuadVertex {
                position_px: tr,
                uv: [frame.uv.u0, cropped_v],
            },
            CounterQuadVertex {
                position_px: br,
                uv: [frame.uv.u1, cropped_v],
            },
            CounterQuadVertex {
                position_px: bl,
                uv: [frame.uv.u1, frame.uv.v1],
            },
        ],
        geometry_honesty: CounterHonesty::Derived,
    });
}

fn build_badges(
    assets: &CounterRenderAssets,
    input: &CounterRenderInput,
    origin_px: ScreenPointPx,
    scale: f32,
    instance: &mut CounterRenderInstance,
    omissions: &mut Vec<CounterRenderOmission>,
) {
    let mut selections = input.badges.clone();
    selections.sort();
    selections.dedup();
    for selection in selections {
        let kind = selection.kind;
        if selection.evidence == CounterHonesty::Fit {
            omissions.push(omission(
                input,
                CounterRenderOmissionReason::BadgeSelectionUnknown(kind),
            ));
            continue;
        }
        let Some(asset) = assets.badge_sprites.get(kind.sprite_name()) else {
            omissions.push(omission(
                input,
                CounterRenderOmissionReason::BadgeAssetUnavailable(kind),
            ));
            continue;
        };
        let Some(frame) = asset.frame(selection.frame_index) else {
            omissions.push(omission(
                input,
                CounterRenderOmissionReason::BadgeFrameOutOfRange(kind),
            ));
            continue;
        };
        let Some(control) = assets.land_layout.control(kind.control_name()) else {
            omissions.push(omission(
                input,
                CounterRenderOmissionReason::LayoutControlUnavailable(kind.control_name()),
            ));
            continue;
        };
        let Some(position) = control.position else {
            omissions.push(omission(
                input,
                CounterRenderOmissionReason::LayoutControlUnavailable(kind.control_name()),
            ));
            continue;
        };
        let mut local = [position.x as f32, position.y as f32];
        let mut size = scaled_natural_size(control, frame.rect.width, frame.rect.height);
        if kind == CounterBadgeKind::ActiveAbility {
            size[0] *= ACTIVE_ABILITY_ICON_SCALE_FILE;
            size[1] *= ACTIVE_ABILITY_ICON_SCALE_FILE;
            // DERIVED from FILE orientation=center inside active_ability_item.
            local[0] += (ACTIVE_ABILITY_ITEM_SIZE_GUI_FILE[0] - size[0]) * 0.5;
            local[1] += (ACTIVE_ABILITY_ITEM_SIZE_GUI_FILE[1] - size[1]) * 0.5;
        }
        instance.quads.push(axis_aligned_quad(
            CounterQuadLayer::Badge(kind),
            asset,
            selection.frame_index,
            selection.evidence,
            frame.uv,
            origin_px,
            local,
            size,
            scale,
        ));
    }
}

fn scaled_natural_size(control: &GuiControlLayout, width: u32, height: u32) -> [f32; 2] {
    let mut size = [width as f32, height as f32];
    if let Some(control_scale) = control.scale {
        size[0] *= control_scale;
        size[1] *= control_scale;
    }
    size
}

#[allow(clippy::too_many_arguments)]
fn axis_aligned_quad(
    layer: CounterQuadLayer,
    asset: &CounterSpriteAsset,
    frame_index: u32,
    selection_honesty: CounterHonesty,
    uv: UvRect,
    origin_px: ScreenPointPx,
    local_gui: [f32; 2],
    size_gui: [f32; 2],
    scale: f32,
) -> CounterQuad {
    let tl = screen_point(origin_px, local_gui, scale);
    let tr = ScreenPointPx {
        x: tl.x + size_gui[0] * scale,
        y: tl.y,
    };
    let br = ScreenPointPx {
        x: tr.x,
        y: tl.y + size_gui[1] * scale,
    };
    let bl = ScreenPointPx { x: tl.x, y: br.y };
    CounterQuad {
        layer,
        texture_file: asset.texture_file.clone(),
        source_file: asset.source_file.clone(),
        source_line: asset.source_line,
        frame_index,
        selection_honesty,
        vertices: [
            CounterQuadVertex {
                position_px: tl,
                uv: [uv.u0, uv.v0],
            },
            CounterQuadVertex {
                position_px: tr,
                uv: [uv.u1, uv.v0],
            },
            CounterQuadVertex {
                position_px: br,
                uv: [uv.u1, uv.v1],
            },
            CounterQuadVertex {
                position_px: bl,
                uv: [uv.u0, uv.v1],
            },
        ],
        geometry_honesty: CounterHonesty::Derived,
    }
}

fn screen_point(origin_px: ScreenPointPx, local_gui: [f32; 2], scale: f32) -> ScreenPointPx {
    ScreenPointPx {
        x: origin_px.x + local_gui[0] * scale,
        y: origin_px.y + local_gui[1] * scale,
    }
}

fn omit(
    batch: &mut CounterRenderBatch,
    input: &CounterRenderInput,
    reason: CounterRenderOmissionReason,
) {
    batch.omissions.push(omission(input, reason));
}

fn omission(
    input: &CounterRenderInput,
    reason: CounterRenderOmissionReason,
) -> CounterRenderOmission {
    CounterRenderOmission {
        unit_id: input.unit_id.clone(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AtlasFrame, GuiPoint, GuiSize, PixelRect};

    fn asset(name: &str, width: u32, height: u32, frames: u32) -> CounterSpriteAsset {
        let frame_width = width / frames;
        CounterSpriteAsset {
            name: name.to_string(),
            texture_file: format!("gfx/{name}.dds"),
            no_of_frames: frames,
            frame_direction_y: false,
            texture_width: width,
            texture_height: height,
            dds_format: "synthetic".to_string(),
            mip_count: 1,
            frame_width,
            frame_height: height,
            frames: (0..frames)
                .map(|index| AtlasFrame {
                    index,
                    rect: PixelRect {
                        x: index * frame_width,
                        y: 0,
                        width: frame_width,
                        height,
                    },
                    uv: UvRect {
                        u0: index as f32 / frames as f32,
                        v0: 0.0,
                        u1: (index + 1) as f32 / frames as f32,
                        v1: 1.0,
                    },
                })
                .collect(),
            source_file: "synthetic.gfx".to_string(),
            source_line: 1,
        }
    }

    fn control(
        name: &str,
        x: i32,
        y: i32,
        rotation: Option<f32>,
        scale: Option<f32>,
    ) -> GuiControlLayout {
        GuiControlLayout {
            kind: "iconType".to_string(),
            name: name.to_string(),
            source_file: "synthetic.gui".to_string(),
            source_line: 1,
            sprite_type: None,
            quad_texture_sprite: None,
            position: Some(GuiPoint { x, y }),
            size: None,
            slot_size: None,
            max_width: None,
            max_height: None,
            font: None,
            rotation,
            scale,
            frame: None,
        }
    }

    fn assets() -> CounterRenderAssets {
        let symbol = asset("GFX_unit_infantry_icon_medium_white", 60, 12, 2);
        let no_orders = asset("GFX_mapicon_unit_no_orders", 8, 8, 1);
        CounterRenderAssets {
            frame: asset(COUNTER_FRAME_SPRITE, 198, 27, 3),
            organisation_bar: asset(COUNTER_ORG_BAR_SPRITE, 3, 17, 1),
            strength_bar: asset(COUNTER_STRENGTH_BAR_SPRITE, 3, 17, 1),
            land_symbols: [(symbol.name.clone(), symbol)].into_iter().collect(),
            unresolved_land_symbol_keys: BTreeSet::new(),
            badge_sprites: [(no_orders.name.clone(), no_orders)].into_iter().collect(),
            land_layout: GuiContainerLayout {
                name: "unit_counter".to_string(),
                source_file: "synthetic.gui".to_string(),
                source_line: 1,
                position: Some(GuiPoint { x: 0, y: 0 }),
                size: Some(GuiSize {
                    width: 76,
                    height: 24,
                }),
                controls: vec![
                    control("count_bg", 0, 0, None, None),
                    control("type", 3, 3, None, None),
                    control(
                        "bar_org",
                        14,
                        8,
                        Some(COUNTER_BAR_ROTATION_RADIANS_FILE),
                        None,
                    ),
                    control(
                        "bar_str",
                        14,
                        12,
                        Some(COUNTER_BAR_ROTATION_RADIANS_FILE),
                        None,
                    ),
                    control("no_orders", -10, 8, None, None),
                ],
            },
            zoomed_in_offset: GuiPoint { x: 0, y: 20 },
        }
    }

    fn input(unit_id: &str, province_id: u32) -> CounterRenderInput {
        CounterRenderInput {
            unit_id: unit_id.to_string(),
            province_id,
            domain: CounterRenderDomain::Land,
            visible: true,
            screen_anchor_px: Some(ScreenPointPx { x: 100.0, y: 50.0 }),
            counter_frame: Some(CounterFrameSelection {
                frame_index: 0,
                evidence: CounterHonesty::Derived,
            }),
            nato_symbol_key: Some("GFX_unit_infantry_icon_medium_white".to_string()),
            nato_symbol_frame: Some(CounterFrameSelection {
                frame_index: 1,
                evidence: CounterHonesty::Derived,
            }),
            strength_fraction: Some(0.25),
            organisation_fraction: Some(0.5),
            badges: vec![CounterBadgeSelection {
                kind: CounterBadgeKind::NoOrders,
                frame_index: 0,
                evidence: CounterHonesty::Derived,
            }],
        }
    }

    fn law(scale: f32, offset: [f32; 2]) -> CounterScreenLaw {
        CounterScreenLaw::evidenced(scale, offset, CounterHonesty::Derived)
            .expect("synthetic evidenced law")
    }

    fn single_stack_law() -> CounterStackingLaw {
        CounterStackingLaw::EvidencedOffsets {
            offsets_gui: vec![[0.0, 0.0]],
            evidence: CounterHonesty::Derived,
        }
    }

    #[test]
    fn unknown_screen_geometry_fails_closed_for_every_visible_input() {
        let batch = build_counter_instances(
            &assets(),
            &[input("b", 1), input("a", 1)],
            CounterScreenLaw::UnknownFailClosed,
            &CounterStackingLaw::UnknownFailClosed,
        );
        assert!(batch.instances.is_empty());
        assert_eq!(batch.omissions.len(), 2);
        assert!(batch
            .omissions
            .iter()
            .all(|row| row.reason == CounterRenderOmissionReason::ScreenGeometryUnknown));
    }

    #[test]
    fn known_input_builds_five_layers_with_exact_corners_and_uvs() {
        let batch = build_counter_instances(
            &assets(),
            &[input("unit", 7)],
            law(2.0, [1.0, -2.0]),
            &single_stack_law(),
        );
        assert_eq!(batch.instances.len(), 1);
        let instance = &batch.instances[0];
        assert_eq!(instance.origin_px, ScreenPointPx { x: 102.0, y: 46.0 });
        assert_eq!(instance.quads.len(), 5);

        let frame = &instance.quads[0];
        assert_eq!(frame.layer, CounterQuadLayer::Frame);
        assert_eq!(
            frame.vertices.map(|vertex| vertex.position_px),
            [
                ScreenPointPx { x: 102.0, y: 46.0 },
                ScreenPointPx { x: 234.0, y: 46.0 },
                ScreenPointPx { x: 234.0, y: 100.0 },
                ScreenPointPx { x: 102.0, y: 100.0 },
            ]
        );
        assert_eq!(frame.vertices[1].uv, [1.0 / 3.0, 0.0]);

        let symbol = &instance.quads[1];
        assert_eq!(symbol.layer, CounterQuadLayer::NatoSymbol);
        assert_eq!(
            symbol.vertices[0].position_px,
            ScreenPointPx { x: 108.0, y: 52.0 }
        );
        assert_eq!(
            symbol.vertices[2].position_px,
            ScreenPointPx { x: 168.0, y: 76.0 }
        );
        assert_eq!(symbol.vertices[0].uv, [0.5, 0.0]);
        assert_eq!(symbol.vertices[2].uv, [1.0, 1.0]);

        let org = &instance.quads[2];
        assert_eq!(org.layer, CounterQuadLayer::OrganisationBar);
        assert_eq!(
            org.vertices[0].position_px,
            ScreenPointPx { x: 130.0, y: 62.0 }
        );
        assert_eq!(
            org.vertices[2].position_px,
            ScreenPointPx { x: 147.0, y: 68.0 }
        );
        assert_eq!(org.vertices[0].uv, [0.0, 1.0]);
        assert_eq!(org.vertices[2].uv, [1.0, 0.5]);

        let strength = &instance.quads[3];
        assert_eq!(strength.layer, CounterQuadLayer::StrengthBar);
        assert_eq!(
            strength.vertices[2].position_px,
            ScreenPointPx { x: 138.5, y: 76.0 }
        );
        assert_eq!(strength.vertices[2].uv, [1.0, 0.75]);

        let badge = &instance.quads[4];
        assert_eq!(
            badge.layer,
            CounterQuadLayer::Badge(CounterBadgeKind::NoOrders)
        );
        assert_eq!(
            badge.vertices[0].position_px,
            ScreenPointPx { x: 82.0, y: 62.0 }
        );
        assert_eq!(
            badge.vertices[2].position_px,
            ScreenPointPx { x: 98.0, y: 78.0 }
        );
    }

    #[test]
    fn captured_h79_law_rounds_anchor_and_defaults_observed_frame_zero() {
        let assets = assets();
        let mut row = input("unit", 3380);
        row.screen_anchor_px = Some(ScreenPointPx {
            x: 1473.4388,
            y: 13.839,
        });
        row.counter_frame = None;
        row.nato_symbol_frame = None;
        let batch = build_counter_instances(
            &assets,
            &[row],
            captured_counter_screen_law(&assets, COUNTER_CAPTURE_H79_MAX),
            &captured_counter_stacking_law(&assets, COUNTER_CAPTURE_H79_MAX),
        );
        assert_eq!(batch.instances.len(), 1);
        assert_eq!(
            batch.instances[0].origin_px,
            ScreenPointPx { x: 1435.0, y: 42.0 }
        );
        assert_eq!(batch.instances[0].quads[0].frame_index, 0);
        assert_eq!(
            batch.instances[0].quads[0].selection_honesty,
            CounterHonesty::Capture
        );
        assert_eq!(batch.instances[0].quads[1].frame_index, 0);
        assert_eq!(batch.instances[0].quads[1].vertices[0].uv, [0.0, 0.0]);
    }

    #[test]
    fn captured_close_zoom_stack_has_only_two_evidenced_slots() {
        let assets = assets();
        let rows = [input("b", 1), input("a", 1), input("c", 1)];
        let batch = build_counter_instances(
            &assets,
            &rows,
            captured_counter_screen_law(&assets, COUNTER_CAPTURE_H79_MIN),
            &captured_counter_stacking_law(&assets, COUNTER_CAPTURE_H79_MIN),
        );
        assert_eq!(
            batch
                .instances
                .iter()
                .map(|row| (row.unit_id.as_str(), row.origin_px.y, row.stacking_honesty))
                .collect::<Vec<_>>(),
            vec![
                ("a", 78.0, CounterHonesty::Capture),
                ("b", 102.0, CounterHonesty::Capture),
            ]
        );
        assert_eq!(
            batch.omissions.last().map(|row| &row.reason),
            Some(&CounterRenderOmissionReason::StackOffsetUnknown)
        );
    }

    #[test]
    fn captured_screen_law_fails_closed_between_observed_height_bands() {
        let assets = assets();
        assert_eq!(
            captured_counter_screen_law(&assets, 60.0),
            CounterScreenLaw::UnknownFailClosed
        );
        assert_eq!(
            captured_counter_screen_law(&assets, 90.0),
            CounterScreenLaw::UnknownFailClosed
        );
        let mut changed = assets;
        changed.land_layout.size = Some(GuiSize {
            width: 77,
            height: 24,
        });
        assert_eq!(
            captured_counter_screen_law(&changed, COUNTER_CAPTURE_H50_MIN),
            CounterScreenLaw::UnknownFailClosed
        );
    }

    #[test]
    fn retired_fit_stack_fails_closed_instead_of_fabricating_offsets() {
        let batch = build_counter_instances(
            &assets(),
            &[input("b", 1), input("a", 1), input("c", 2)],
            law(1.0, [0.0, 0.0]),
            &CounterStackingLaw::FitVerticalByDeclaredContainerHeight,
        );
        assert!(batch.instances.is_empty());
        assert_eq!(batch.omissions.len(), 3);
        assert!(batch
            .omissions
            .iter()
            .all(|row| row.reason == CounterRenderOmissionReason::StackOffsetUnknown));
    }

    #[test]
    fn unknown_fields_and_invalid_fractions_emit_no_guesses() {
        let mut row = input("unit", 1);
        row.counter_frame = None;
        row.nato_symbol_frame = None;
        row.organisation_fraction = None;
        row.strength_fraction = Some(1.5);
        let batch =
            build_counter_instances(&assets(), &[row], law(1.0, [0.0, 0.0]), &single_stack_law());
        assert_eq!(batch.instances.len(), 1);
        assert_eq!(batch.instances[0].quads.len(), 1); // explicit badge only
        assert_eq!(batch.omissions.len(), 4);
        assert!(batch.omissions.iter().any(|row| {
            row.reason == CounterRenderOmissionReason::CounterFrameSelectionUnknown
        }));
        assert!(batch.omissions.iter().any(|row| {
            row.reason == CounterRenderOmissionReason::NatoSymbolFrameSelectionUnknown
        }));
        assert!(batch.omissions.iter().any(|row| {
            row.reason
                == CounterRenderOmissionReason::BarFractionUnknown(
                    CounterQuadLayer::OrganisationBar,
                )
        }));
        assert!(batch.omissions.iter().any(|row| {
            row.reason
                == CounterRenderOmissionReason::BarFractionOutOfRange(CounterQuadLayer::StrengthBar)
        }));
    }

    #[test]
    fn evidenced_stack_and_screen_laws_validate_without_defaults() {
        let batch = build_counter_instances(
            &assets(),
            &[input("a", 1), input("b", 1)],
            law(1.0, [0.0, 0.0]),
            &CounterStackingLaw::EvidencedOffsets {
                offsets_gui: vec![[3.0, 4.0]],
                evidence: CounterHonesty::Re,
            },
        );
        assert_eq!(batch.instances.len(), 1);
        assert_eq!(
            batch.instances[0].origin_px,
            ScreenPointPx { x: 103.0, y: 54.0 }
        );
        assert_eq!(
            batch.omissions.last().map(|row| &row.reason),
            Some(&CounterRenderOmissionReason::StackOffsetUnknown)
        );
        assert!(CounterScreenLaw::evidenced(1.0, [0.0, 0.0], CounterHonesty::Fit).is_err());
        assert!(
            CounterScreenLaw::evidenced(f32::NAN, [0.0, 0.0], CounterHonesty::Derived).is_err()
        );

        let bypassed_fit = CounterScreenLaw::Evidenced {
            pixels_per_gui_unit: 1.0,
            anchor_to_origin_gui: [0.0, 0.0],
            round_anchor_to_pixel: false,
            default_counter_frame: None,
            default_nato_symbol_frame: None,
            evidence: CounterHonesty::Fit,
        };
        let blocked = build_counter_instances(
            &assets(),
            &[input("a", 1)],
            bypassed_fit,
            &single_stack_law(),
        );
        assert!(blocked.instances.is_empty());
        assert_eq!(
            blocked.omissions[0].reason,
            CounterRenderOmissionReason::ScreenGeometryUnknown
        );
    }

    #[test]
    fn fitted_runtime_frame_selections_are_rejected() {
        let mut row = input("unit", 1);
        row.counter_frame.as_mut().expect("frame").evidence = CounterHonesty::Fit;
        row.nato_symbol_frame.as_mut().expect("symbol").evidence = CounterHonesty::Fit;
        row.badges[0].evidence = CounterHonesty::Fit;
        let batch =
            build_counter_instances(&assets(), &[row], law(1.0, [0.0, 0.0]), &single_stack_law());
        assert_eq!(batch.instances.len(), 1);
        assert_eq!(batch.instances[0].quads.len(), 2); // FILE-selected bar frames only
        assert_eq!(batch.omissions.len(), 3);
        assert!(batch.omissions.iter().any(|row| {
            row.reason == CounterRenderOmissionReason::CounterFrameSelectionUnknown
        }));
        assert!(batch.omissions.iter().any(|row| {
            row.reason == CounterRenderOmissionReason::NatoSymbolFrameSelectionUnknown
        }));
        assert!(batch.omissions.iter().any(|row| {
            row.reason
                == CounterRenderOmissionReason::BadgeSelectionUnknown(CounterBadgeKind::NoOrders)
        }));
    }

    #[test]
    fn player_only_filter_requires_item4_mapping_and_never_leaks_nonplayers() {
        let rows = [input("player", 1), input("ai", 2), input("unknown", 3)];
        let absent =
            select_counter_display_inputs(&rows, true, Some(CounterColorMode::Countries), None);
        assert!(absent.inputs.is_empty());
        assert_eq!(
            absent.gaps,
            vec![
                CounterDisplayOmissionReason::Item4PlayerCountryMappingUnavailable,
                CounterDisplayOmissionReason::Cap6CountriesTintFrameUnavailable,
            ]
        );
        assert_eq!(absent.omissions.len(), 6);

        let mapping = CounterPlayerMapping {
            player_country_tags: BTreeSet::from(["GER".to_string()]),
            unit_owner_tags: BTreeMap::from([
                ("player".to_string(), "GER".to_string()),
                ("ai".to_string(), "ITA".to_string()),
            ]),
        };
        let selected = select_counter_display_inputs(
            &rows,
            true,
            Some(CounterColorMode::Countries),
            Some(&mapping),
        );
        assert!(selected.inputs.is_empty());
        assert_eq!(
            selected.gaps,
            vec![CounterDisplayOmissionReason::Cap6CountriesTintFrameUnavailable]
        );
        assert!(selected.omissions.iter().any(|row| {
            row.unit_id == "unknown"
                && row.reason == CounterDisplayOmissionReason::UnitOwnerUnavailable
        }));
        assert!(selected.omissions.iter().any(|row| {
            row.unit_id == "player"
                && row.reason == CounterDisplayOmissionReason::Cap6CountriesTintFrameUnavailable
        }));
        assert!(!selected.omissions.iter().any(|row| row.unit_id == "ai"));
    }

    #[test]
    fn country_and_allegiance_colour_modes_are_distinct_cap6_omissions() {
        let rows = [input("unit", 1)];
        let unavailable = select_counter_display_inputs(&rows, false, None, None);
        assert!(unavailable.inputs.is_empty());
        assert_eq!(
            unavailable.gaps,
            vec![CounterDisplayOmissionReason::Cap6CounterColorModeUnavailable]
        );

        let countries =
            select_counter_display_inputs(&rows, false, Some(CounterColorMode::Countries), None);
        assert!(countries.inputs.is_empty());
        assert_eq!(
            countries.omissions[0].reason,
            CounterDisplayOmissionReason::Cap6CountriesTintFrameUnavailable
        );

        let allegiance =
            select_counter_display_inputs(&rows, false, Some(CounterColorMode::Allegiance), None);
        assert_eq!(
            allegiance.gaps,
            vec![CounterDisplayOmissionReason::Cap6AllegianceTintFrameUnavailable]
        );

        let empty =
            select_counter_display_inputs(&[], true, Some(CounterColorMode::Allegiance), None);
        assert_eq!(
            empty.gaps,
            vec![
                CounterDisplayOmissionReason::Item4PlayerCountryMappingUnavailable,
                CounterDisplayOmissionReason::Cap6AllegianceTintFrameUnavailable,
            ]
        );
        assert!(empty.omissions.is_empty());
    }

    #[test]
    fn vanilla_resolver_decodes_render_assets_and_builds_known_land_counter() {
        let game_dir = std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from("X:/SteamLibrary/steamapps/common/Hearts of Iron IV")
            });
        if !game_dir.is_dir() {
            eprintln!("SKIP: vanilla game dir missing at {}", game_dir.display());
            return;
        }
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let assets = counter_render_assets_from_resolver(&resolver).expect("render assets");
        assert_eq!(
            (assets.frame.frame_width, assets.frame.frame_height),
            (66, 27)
        );
        assert_eq!(
            (
                assets.organisation_bar.frame_width,
                assets.organisation_bar.frame_height,
            ),
            (3, 17)
        );
        assert!(assets
            .land_symbols
            .contains_key("GFX_unit_infantry_icon_medium_white"));
        assert_eq!(assets.unresolved_land_symbol_keys.len(), 1);
        assert_eq!(assets.badge_sprites.len(), 9);

        let mut row = input("division", 1000);
        row.organisation_fraction = None;
        row.strength_fraction = Some(0.5);
        let batch =
            build_counter_instances(&assets, &[row], law(1.0, [0.0, 0.0]), &single_stack_law());
        assert_eq!(batch.instances.len(), 1);
        assert_eq!(batch.instances[0].quads.len(), 4);
        assert_eq!(
            batch.instances[0]
                .quads
                .iter()
                .map(|quad| quad.layer)
                .collect::<Vec<_>>(),
            vec![
                CounterQuadLayer::Frame,
                CounterQuadLayer::NatoSymbol,
                CounterQuadLayer::StrengthBar,
                CounterQuadLayer::Badge(CounterBadgeKind::NoOrders),
            ]
        );
        assert_eq!(
            batch.instances[0].quads[0].vertices[2].position_px,
            ScreenPointPx { x: 166.0, y: 77.0 }
        );
        assert_eq!(batch.instances[0].quads[1].vertices[0].uv, [0.5, 0.0]);
        assert!(batch.omissions.iter().any(|row| {
            row.reason
                == CounterRenderOmissionReason::BarFractionUnknown(
                    CounterQuadLayer::OrganisationBar,
                )
        }));
    }
}
