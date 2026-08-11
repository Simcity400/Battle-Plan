use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const PROVINCE_COLOR_MAP_WIDTH: usize = 2816;
pub const PROVINCE_COLOR_MAP_HEIGHT: usize = 1024;
pub const PROVINCE_COLOR_MAP_DXGI_FORMAT: u32 = DXGI_FORMAT_B8G8R8A8_UNORM_SRGB;
pub const PROVINCE_SECONDARY_COLOR_MAP_SHADER_SLOT: u32 = 6;

const PROVINCE_SECONDARY_CAPTURE_JSON: &str = "province_secondary_color_map_capture.json";
const PROVINCE_SECONDARY_SYNTHETIC_FILE: &str = "province_secondary_color_map_synthesized.rgba";
const PROVINCE_OWNER_SYNTHETIC_FILE: &str = "province_owner_color_map_synthesized.rgba";
const PROVINCE_SECONDARY_DIFF_HEATMAP_FILE: &str = "province_secondary_color_map_diff.bmp";
const POLITICAL_COLOR_STATS_FILE: &str = "political_color_stats.json";

#[derive(Clone, Debug)]
pub struct PoliticalColorSynthesis {
    pub width: usize,
    pub height: usize,
    pub owner_color_bytes: Vec<u8>,
    pub secondary_color_bytes: Vec<u8>,
    pub descriptor: PoliticalColorTextureDescriptor,
    pub source: PoliticalColorSynthesisSource,
    pub law: PoliticalColorCompositionLaw,
    class_by_texel: Vec<PoliticalColorTexelClass>,
}

/// Political topology consumed by the exact-a729 GradientBorder producer.
/// Region zero is outside/no-controller. Every non-zero region is a stable
/// controller tag for the current replay epoch.
#[derive(Clone, Debug)]
pub(crate) struct GradientBorderPoliticalInputs {
    /// Controller region indexed by exact province database id. The engine
    /// constructs edge records on the full-resolution province-id raster
    /// before reducing the completed layer for the terrain texture.
    pub(crate) province_regions: Vec<u32>,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) regions: Vec<u32>,
    pub(crate) land_mask: Vec<bool>,
    pub(crate) region_colors: Vec<[u8; 3]>,
    /// Full-resolution province-raster pixel area per controller region. This
    /// is the exact field consumed by the a729 country thickness callback.
    pub(crate) region_areas: Vec<u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct Xmap05RasterInputs {
    pub(crate) map_width: usize,
    pub(crate) map_height: usize,
    pub(crate) province_ids: Vec<u16>,
    pub(crate) province_is_eligible_land: Vec<bool>,
    pub(crate) gradient: GradientBorderPoliticalInputs,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoliticalColorTextureDescriptor {
    pub symbol: String,
    pub format: String,
    pub dxgi_format: u32,
    pub width: usize,
    pub height: usize,
    pub byte_len: usize,
    pub row_pitch: usize,
    pub mip_count: u32,
    pub shader_slot: u32,
    pub fnv1a64: String,
    pub sampler: PoliticalColorSamplerDescriptor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoliticalColorSamplerDescriptor {
    pub mag_filter: String,
    pub min_filter: String,
    pub mip_filter: String,
    pub address_u: String,
    pub address_v: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoliticalColorSynthesisSource {
    pub definition_csv: PathBuf,
    pub provinces_bmp: PathBuf,
    pub colors_txt: Option<PathBuf>,
    pub state_files: usize,
    pub definition_csv_fnv1a64: String,
    pub provinces_bmp_fnv1a64: String,
    pub colors_txt_fnv1a64: Option<String>,
    pub country_color_count: usize,
    pub state_count: usize,
    pub province_to_state_count: usize,
    pub state_owner_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoliticalColorCompositionLaw {
    pub captured_symbol: String,
    pub texture_space: String,
    pub static_primary_source: String,
    pub static_secondary_source: String,
    pub owner_controller_rule: String,
    pub sea_unowned_rule: String,
    pub impassable_rule: String,
    pub dynamic_inputs_needed: Vec<String>,
    pub shader_sources: Vec<String>,
    pub file_sources: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoliticalColorComparisonReport {
    pub capture_dir: PathBuf,
    pub captured_file: PathBuf,
    pub generated_descriptor: PoliticalColorTextureDescriptor,
    pub captured_descriptor: PoliticalColorTextureDescriptor,
    pub byte_exact: bool,
    pub byte_mismatches: usize,
    pub texel_mismatches: usize,
    pub texel_match_percent: f64,
    pub channel_mismatches: [usize; 4],
    pub class_counts: BTreeMap<String, usize>,
    pub mismatch_class_counts: BTreeMap<String, usize>,
    pub mismatch_samples: Vec<PoliticalColorMismatchSample>,
    pub all_captured_texels_transparent_zero: bool,
    pub all_synthesized_texels_transparent_zero: bool,
    /// True when both the synthesized and captured secondary maps are fully
    /// transparent. A byte-exact / 100% match in this state is a *vacuous*
    /// agreement (transparent == transparent); it does NOT validate the
    /// province-id -> occupation-stripe mapping, because no non-transparent
    /// texel was ever exercised. Downstream readers must not treat a vacuous
    /// match as proof of the political color law.
    pub validation_is_vacuous: bool,
    pub static_occupied_texels: usize,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoliticalColorMismatchSample {
    pub x: usize,
    pub y: usize,
    pub class: String,
    pub generated: [u8; 4],
    pub captured: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoliticalColorDumpPaths {
    pub synthesized_secondary_rgba: PathBuf,
    pub synthesized_owner_rgba: PathBuf,
    pub diff_heatmap_bmp: PathBuf,
    pub stats_json: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
struct ProvinceSecondaryColorCaptureJson {
    schema: String,
    symbol: String,
    file: String,
    width: usize,
    height: usize,
    byte_len: usize,
    fnv1a64: String,
    format: String,
    source_dxgi_format: u32,
    shader_resource_slot: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PoliticalColorTexelClass {
    SeaOrLake,
    UnknownProvince,
    NoState,
    NoOwner,
    Impassable,
    OwnerControllerEqual,
    StaticControllerDiffers,
}

impl PoliticalColorTexelClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::SeaOrLake => "sea_or_lake_transparent",
            Self::UnknownProvince => "unknown_province_transparent",
            Self::NoState => "no_state_transparent",
            Self::NoOwner => "no_owner_transparent",
            Self::Impassable => "impassable_transparent",
            Self::OwnerControllerEqual => "owner_controller_equal_transparent",
            Self::StaticControllerDiffers => "static_controller_differs_secondary_color",
        }
    }
}

/// Per-state owner/controller for one moment. In the static path both come
/// from the 1936 `history/states` initial block; in the A5 timeline path they
/// are filled from the `WorldTimeline` `owner_by_state` / `control_by_state`
/// views (see [`crate::keying::KeyingTables`]). `None` means "not keyed for
/// this state at this moment" — the fill law falls back / fails closed, never
/// invents a tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitialStateControl {
    pub owner: Option<String>,
    pub controller: Option<String>,
}

pub fn synthesize_political_colors(resolver: &GameFileResolver) -> Result<PoliticalColorSynthesis> {
    synthesize_political_colors_impl(resolver, None)
}

/// A5 timeline-keyed political fill: identical to [`synthesize_political_colors`]
/// but the owner-by-state, state controller, and per-tag cosmetic color come
/// from a scrubbed [`KeyingTables`] (built from a `WorldTimeline` frame) instead
/// of the static 1936 history parse. The province raster, definitions, state
/// topology (province->state, impassable), and BASE country colors are still
/// file-derived (they are not recording state); only the political keying is
/// swapped. On the 1936 baseline moment this yields byte-identical output to the
/// static path when the timeline carries the same owner/controller/cosmetic
/// (the A5 t0 gate).
pub fn synthesize_political_colors_with_keying(
    resolver: &GameFileResolver,
    keying: &KeyingTables,
) -> Result<PoliticalColorSynthesis> {
    synthesize_political_colors_impl(resolver, Some(keying))
}

fn validate_xmap05_state_authority(
    effective_state_ids: &BTreeSet<u32>,
    keying: &KeyingTables,
    province_controllers: &BTreeMap<u32, String>,
    country_roster: &BTreeSet<String>,
) -> Result<()> {
    let owner_state_ids = keying.state_owners.keys().copied().collect::<BTreeSet<_>>();
    let controller_state_ids = keying
        .state_controls
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    if owner_state_ids != *effective_state_ids || controller_state_ids != *effective_state_ids {
        return Err(format!(
            "XMAP05 measured state census does not equal the effective history/states census (effective={}, owners={}, controllers={})",
            effective_state_ids.len(),
            owner_state_ids.len(),
            controller_state_ids.len()
        )
        .into());
    }
    for (role, tag) in keying
        .state_owners
        .values()
        .map(|tag| ("state owner", tag))
        .chain(keying.state_controls.values().flat_map(|control| {
            control
                .owner
                .iter()
                .map(|tag| ("state control owner", tag))
                .chain(
                    control
                        .controller
                        .iter()
                        .map(|tag| ("state controller", tag)),
                )
        }))
        .chain(
            province_controllers
                .values()
                .map(|tag| ("province controller", tag)),
        )
    {
        if !country_roster.contains(tag) {
            return Err(
                format!("XMAP05 {role} tag {tag} is absent from the complete D18 roster").into(),
            );
        }
    }
    Ok(())
}

fn validate_xmap05_province_census(
    eligible_provinces: &BTreeSet<u32>,
    province_controllers: &BTreeMap<u32, String>,
) -> Result<()> {
    let measured_provinces = province_controllers
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    if measured_provinces != *eligible_provinces {
        return Err(format!(
            "XMAP05 measured province-controller census does not equal the effective eligible-land census (effective={}, measured={})",
            eligible_provinces.len(),
            measured_provinces.len()
        )
        .into());
    }
    Ok(())
}

/// Resolve the complete file-derived XMAP05 raster topology for one replay
/// epoch. Political ownership/control comes from `keying`; province identity,
/// state membership, land eligibility, and country colours come from the
/// effective VFS. No captured presentation texture participates.
pub(crate) fn xmap05_raster_inputs_with_keying(
    resolver: &GameFileResolver,
    keying: &KeyingTables,
    province_controllers: &BTreeMap<u32, String>,
    country_roster: &BTreeSet<String>,
) -> Result<Xmap05RasterInputs> {
    let definition_csv = resolver.resolve_required("map/definition.csv", "definition.csv")?;
    let provinces_bmp = resolver.resolve_required("map/provinces.bmp", "provinces.bmp")?;
    let state_paths = resolver.resolve_dir_files("history/states", Some("txt"))?;
    let (definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let provinces = load_provinces_bmp(&provinces_bmp, &color_to_id)?;
    if provinces.width == 0
        || provinces.height == 0
        || provinces.width % 2 != 0
        || provinces.height % 2 != 0
    {
        return Err(format!(
            "map/provinces.bmp dimensions {}x{} cannot produce half-resolution GradientBorder resources",
            provinces.width, provinces.height
        )
        .into());
    }
    let (states, province_to_state, _) = load_states(&state_paths)?;
    let country_colors =
        country_colors_from_game_resolver_with_cosmetics(resolver, &keying.cosmetic_by_tag)?;
    let effective_state_ids = states.keys().copied().collect::<BTreeSet<_>>();
    validate_xmap05_state_authority(
        &effective_state_ids,
        keying,
        province_controllers,
        country_roster,
    )?;

    // XMAP05 owns both state- and province-control authority. the game's normal
    // political/GradientBorder path is state-controller keyed, while the
    // independent province mirror is consumed by country-label placement.
    // Admit the shared political resource only when the two controller views
    // describe the same land topology. A province-level divergence is not
    // guessed into a state colour or silently ignored.
    let eligible_provinces = definitions
        .iter()
        .filter_map(|(&province_id, definition)| {
            (definition.province_type == "land" && province_to_state.contains_key(&province_id))
                .then_some(province_id)
        })
        .collect::<BTreeSet<_>>();
    validate_xmap05_province_census(&eligible_provinces, province_controllers)?;
    for &province_id in &eligible_provinces {
        let state_id = province_to_state[&province_id];
        let state_controller = keying
            .state_controls
            .get(&state_id)
            .and_then(|control| control.controller.as_deref())
            .or_else(|| keying.state_owners.get(&state_id).map(String::as_str))
            .ok_or_else(|| {
                format!(
                    "XMAP05 state {state_id} has no measured owner/controller for land province {province_id}"
                )
            })?;
        let province_controller = province_controllers.get(&province_id).ok_or_else(|| {
            format!(
                "XMAP05 province-control census omits land province {province_id} in state {state_id}"
            )
        })?;
        if province_controller != state_controller {
            return Err(format!(
                "XMAP05 province {province_id} controller {province_controller} disagrees with state {state_id} controller {state_controller}"
            )
            .into());
        }
    }

    let max_province = definitions
        .keys()
        .copied()
        .chain(provinces.ids.iter().copied())
        .max()
        .unwrap_or(0);
    let mut province_is_eligible_land = vec![false; usize::try_from(max_province)? + 1];
    let mut province_regions = vec![0u32; usize::try_from(max_province)? + 1];
    for (&province_id, definition) in &definitions {
        if definition.province_type == "land" {
            province_is_eligible_land[usize::try_from(province_id)?] = true;
        }
    }
    let province_ids = provinces
        .ids
        .iter()
        .map(|value| {
            u16::try_from(*value).map_err(|_| -> Box<dyn std::error::Error> {
                format!(
                    "province database id {value} exceeds exact-a729 IntelMap u16 coverage storage"
                )
                .into()
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut tag_regions = BTreeMap::<String, u32>::new();
    let mut region_colors = vec![[0u8; 3]];
    let max_state = states.keys().copied().max().unwrap_or(0) as usize;
    let mut state_regions = vec![0u32; max_state + 1];
    for &state_id in states.keys() {
        let Some(control) = keying.state_controls.get(&state_id) else {
            continue;
        };
        let controller = control
            .controller
            .as_deref()
            .or(control.owner.as_deref())
            .or_else(|| keying.state_owners.get(&state_id).map(String::as_str));
        let Some(controller) = controller else {
            continue;
        };
        let raw_color = require_country_color(&country_colors, controller)?;
        let region = if let Some(region) = tag_regions.get(controller) {
            *region
        } else {
            let region = u32::try_from(region_colors.len())
                .map_err(|_| "GradientBorder controller region id overflow")?;
            tag_regions.insert(controller.to_string(), region);
            region_colors.push(raw_color);
            region
        };
        state_regions[state_id as usize] = region;
    }

    let width = provinces.width / 2;
    let height = provinces.height / 2;
    let mut regions = vec![0u32; width * height];
    let mut land_mask = vec![false; width * height];
    let mut region_areas = vec![0u32; region_colors.len()];
    for &province_id in &provinces.ids {
        let Some(&state_id) = province_to_state.get(&province_id) else {
            continue;
        };
        let region = state_regions.get(state_id as usize).copied().unwrap_or(0);
        province_regions[usize::try_from(province_id)?] = region;
        if region != 0 {
            region_areas[region as usize] = region_areas[region as usize]
                .checked_add(1)
                .ok_or("GradientBorder country pixel area overflow")?;
        }
    }
    for y in 0..height {
        for x in 0..width {
            let texel = y * width + x;
            // The exact half-resolution lookup samples the center-selected
            // source pixel: x=2*x+1 and, after the engine's output-row flip,
            // y=map_height-2*y-2. No 2x2 vote or tie-break exists.
            let source = xmap05_half_res_source_index(provinces.width, provinces.height, x, y);
            let province_id = provinces.ids[source];
            let state_id = province_to_state.get(&province_id).copied().unwrap_or(0);
            if state_id == 0 {
                continue;
            }
            land_mask[texel] = true;
            regions[texel] = state_regions.get(state_id as usize).copied().unwrap_or(0);
        }
    }

    Ok(Xmap05RasterInputs {
        map_width: provinces.width,
        map_height: provinces.height,
        province_ids,
        province_is_eligible_land,
        gradient: GradientBorderPoliticalInputs {
            province_regions,
            width,
            height,
            regions,
            land_mask,
            region_colors,
            region_areas,
        },
    })
}

pub(crate) fn xmap05_half_res_source_index(
    map_width: usize,
    map_height: usize,
    x: usize,
    y: usize,
) -> usize {
    let source_x = x * 2 + 1;
    let source_y = map_height - (y + 1) * 2;
    source_y * map_width + source_x
}

/// Shared body of the static and A5 timeline-keyed synthesis entry points.
/// Everything file-derived (province raster, definitions, state topology,
/// dimension law) is identical between the two paths; only the political
/// keying source (owner/controller/cosmetic) is selected by `keying`.
fn synthesize_political_colors_impl(
    resolver: &GameFileResolver,
    keying: Option<&KeyingTables>,
) -> Result<PoliticalColorSynthesis> {
    let definition_csv = resolver.resolve_required("map/definition.csv", "definition.csv")?;
    let provinces_bmp = resolver.resolve_required("map/provinces.bmp", "provinces.bmp")?;
    let state_paths = resolver.resolve_dir_files("history/states", Some("txt"))?;
    let colors_txt = resolver.resolve("common/countries/colors.txt")?;

    let (definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let provinces = load_provinces_bmp(&provinces_bmp, &color_to_id)?;
    if provinces.width == 0
        || provinces.height == 0
        || provinces.width % 2 != 0
        || provinces.height % 2 != 0
    {
        return Err(format!(
            "map/provinces.bmp dimensions {}x{} cannot produce the exact half-resolution political texture",
            provinces.width,
            provinces.height,
        )
        .into());
    }
    let texture_width = provinces.width / 2;
    let texture_height = provinces.height / 2;

    let (states, province_to_state, static_owners) = load_states(&state_paths)?;
    let static_controls;
    let static_country_colors;
    let keyed_country_colors;
    let (state_owners, state_controls, country_colors): (
        &BTreeMap<u32, String>,
        &BTreeMap<u32, InitialStateControl>,
        &BTreeMap<String, [u8; 3]>,
    ) = match keying {
        None => {
            static_controls = load_initial_state_controls(&state_paths)?;
            static_country_colors = load_country_colors_from_game_resolver(resolver)?;
            (&static_owners, &static_controls, &static_country_colors)
        }
        Some(keying) => {
            // Topology (province->state, impassable) is not recording state; keep it
            // file-derived. Keying (owner/controller/cosmetic) comes from the timeline.
            keyed_country_colors = country_colors_from_game_resolver_with_cosmetics(
                resolver,
                &keying.cosmetic_by_tag,
            )?;
            (
                &keying.state_owners,
                &keying.state_controls,
                &keyed_country_colors,
            )
        }
    };

    let (owner_color_bytes, secondary_color_bytes, class_by_texel) = render_political_texels(
        texture_width,
        texture_height,
        &definitions,
        &provinces,
        &province_to_state,
        &states,
        state_owners,
        state_controls,
        country_colors,
    )?;

    let descriptor = PoliticalColorTextureDescriptor {
        symbol: PROVINCE_SECONDARY_COLOR_MAP_SYMBOL.to_string(),
        format: RUNTIME_RGBA8_FORMAT.to_string(),
        dxgi_format: PROVINCE_COLOR_MAP_DXGI_FORMAT,
        width: texture_width,
        height: texture_height,
        byte_len: secondary_color_bytes.len(),
        row_pitch: texture_width * 4,
        mip_count: 1,
        shader_slot: PROVINCE_SECONDARY_COLOR_MAP_SHADER_SLOT,
        fnv1a64: fnv1a64_bytes(&secondary_color_bytes),
        sampler: province_secondary_color_sampler(),
    };

    Ok(PoliticalColorSynthesis {
        width: texture_width,
        height: texture_height,
        owner_color_bytes,
        secondary_color_bytes,
        descriptor,
        source: PoliticalColorSynthesisSource {
            definition_csv: definition_csv.clone(),
            provinces_bmp: provinces_bmp.clone(),
            colors_txt: colors_txt.clone(),
            state_files: state_paths.len(),
            definition_csv_fnv1a64: fnv1a64_file(&definition_csv)?,
            provinces_bmp_fnv1a64: fnv1a64_file(&provinces_bmp)?,
            colors_txt_fnv1a64: colors_txt
                .as_ref()
                .map(|path| fnv1a64_file(path))
                .transpose()?,
            country_color_count: country_colors.len(),
            state_count: states.len(),
            province_to_state_count: province_to_state.len(),
            state_owner_count: state_owners.len(),
        },
        law: political_color_composition_law(),
        class_by_texel,
    })
}

/// The shared per-texel political-fill loop used by both the static and the
/// A5 timeline-keyed entry points, so the two paths are byte-identical by
/// construction when fed the same keying. Returns
/// `(owner_color_bytes, secondary_color_bytes, class_by_texel)`.
fn render_political_texels(
    texture_width: usize,
    texture_height: usize,
    definitions: &BTreeMap<u32, ProvinceDefinition>,
    provinces: &BmpIds,
    province_to_state: &BTreeMap<u32, u32>,
    states: &BTreeMap<u32, StateMeta>,
    state_owners: &BTreeMap<u32, String>,
    state_controls: &BTreeMap<u32, InitialStateControl>,
    country_colors: &BTreeMap<String, [u8; 3]>,
) -> Result<(Vec<u8>, Vec<u8>, Vec<PoliticalColorTexelClass>)> {
    let mut owner_color_bytes = vec![0u8; texture_width * texture_height * 4];
    let mut secondary_color_bytes = vec![0u8; texture_width * texture_height * 4];
    let mut class_by_texel =
        vec![PoliticalColorTexelClass::UnknownProvince; texture_width * texture_height];
    // The per-texel result depends only on the province id (~13-17k distinct
    // values across ~2.9M texels), so memoize per province id. The cache is
    // filled LAZILY in raster order: only province ids an actual texel samples
    // are ever evaluated, which keeps output bytes, class_by_texel, and error
    // behavior (including which fail-closed error fires first) exactly
    // identical to the unmemoized loop.
    let mut colors_by_province =
        BTreeMap::<u32, (PoliticalColorTexelClass, Option<[u8; 4]>, Option<[u8; 4]>)>::new();

    for y in 0..texture_height {
        for x in 0..texture_width {
            let texel_index = y * texture_width + x;
            let byte_offset = texel_index * 4;
            let (source_x, source_y) = political_source_pixel_for_texel(
                x,
                y,
                provinces.width,
                provinces.height,
                texture_width,
                texture_height,
            );
            let province_id = provinces.ids[source_y * provinces.width + source_x];
            let (class, owner_color, secondary_color) = match colors_by_province.get(&province_id) {
                Some(cached) => *cached,
                None => {
                    let computed = political_texel_colors(
                        province_id,
                        definitions,
                        province_to_state,
                        states,
                        state_owners,
                        state_controls,
                        country_colors,
                    )?;
                    colors_by_province.insert(province_id, computed);
                    computed
                }
            };
            class_by_texel[texel_index] = class;
            if let Some(color) = owner_color {
                owner_color_bytes[byte_offset..byte_offset + 4].copy_from_slice(&color);
            }
            if let Some(color) = secondary_color {
                secondary_color_bytes[byte_offset..byte_offset + 4].copy_from_slice(&color);
            }
        }
    }
    Ok((owner_color_bytes, secondary_color_bytes, class_by_texel))
}

pub fn compare_political_colors_to_capture(
    resolver: &GameFileResolver,
    capture_dir: &Path,
) -> Result<PoliticalColorComparisonReport> {
    let synthesized = synthesize_political_colors(resolver)?;
    let (report, _captured_bytes) = compare_synthesized_to_capture(&synthesized, capture_dir)?;
    Ok(report)
}

/// Core of the capture comparison: takes an already-built synthesis so callers
/// that need both the synthesis output and the comparison (the dump path) run
/// the 2816x1024 political fill exactly once. Owns ALL capture-side
/// validations (byte length vs synthesized, byte length vs capture JSON,
/// FNV-1a64 integrity, captured descriptor law) and runs them fail-closed
/// before the texel comparison. Returns the validated captured bytes alongside
/// the report so downstream consumers (diff heatmap) never re-read the capture
/// through an unvalidated path.
fn compare_synthesized_to_capture(
    synthesized: &PoliticalColorSynthesis,
    capture_dir: &Path,
) -> Result<(PoliticalColorComparisonReport, Vec<u8>)> {
    let (capture_meta, captured_file, captured_bytes) =
        load_captured_province_secondary_color_map(capture_dir)?;
    if captured_bytes.len() != synthesized.secondary_color_bytes.len() {
        return Err(format!(
            "{} has {} bytes, generated {} has {} bytes",
            captured_file.display(),
            captured_bytes.len(),
            PROVINCE_SECONDARY_COLOR_MAP_SYMBOL,
            synthesized.secondary_color_bytes.len()
        )
        .into());
    }
    if captured_bytes.len() != capture_meta.byte_len {
        return Err(format!(
            "{} has {} bytes, capture JSON declares {}",
            captured_file.display(),
            captured_bytes.len(),
            capture_meta.byte_len
        )
        .into());
    }
    let captured_fnv = fnv1a64_bytes(&captured_bytes);
    if captured_fnv != capture_meta.fnv1a64 {
        return Err(format!(
            "{} FNV-1a64 is {}, capture JSON declares {}",
            captured_file.display(),
            captured_fnv,
            capture_meta.fnv1a64
        )
        .into());
    }
    let captured_descriptor = PoliticalColorTextureDescriptor {
        symbol: capture_meta.symbol,
        format: capture_meta.format,
        dxgi_format: capture_meta.source_dxgi_format,
        width: capture_meta.width,
        height: capture_meta.height,
        byte_len: capture_meta.byte_len,
        row_pitch: capture_meta.width * 4,
        mip_count: 1,
        shader_slot: capture_meta.shader_resource_slot,
        fnv1a64: capture_meta.fnv1a64,
        sampler: province_secondary_color_sampler(),
    };
    validate_captured_province_secondary_descriptor(&captured_descriptor)?;

    let mut byte_mismatches = 0usize;
    let mut texel_mismatches = 0usize;
    let mut channel_mismatches = [0usize; 4];
    let mut mismatch_class_counts = BTreeMap::<String, usize>::new();
    let mut samples = Vec::new();
    for y in 0..synthesized.height {
        for x in 0..synthesized.width {
            let texel_index = y * synthesized.width + x;
            let offset = texel_index * 4;
            let generated = [
                synthesized.secondary_color_bytes[offset],
                synthesized.secondary_color_bytes[offset + 1],
                synthesized.secondary_color_bytes[offset + 2],
                synthesized.secondary_color_bytes[offset + 3],
            ];
            let captured = [
                captured_bytes[offset],
                captured_bytes[offset + 1],
                captured_bytes[offset + 2],
                captured_bytes[offset + 3],
            ];
            let mut differs = false;
            for channel in 0..4 {
                if generated[channel] != captured[channel] {
                    differs = true;
                    byte_mismatches += 1;
                    channel_mismatches[channel] += 1;
                }
            }
            if differs {
                texel_mismatches += 1;
                let class = synthesized.class_by_texel[texel_index].as_str().to_string();
                *mismatch_class_counts.entry(class.clone()).or_insert(0) += 1;
                if samples.len() < 32 {
                    samples.push(PoliticalColorMismatchSample {
                        x,
                        y,
                        class,
                        generated,
                        captured,
                    });
                }
            }
        }
    }

    let total_texels = synthesized.width * synthesized.height;
    let texel_match_percent =
        ((total_texels - texel_mismatches) as f64 * 100.0) / total_texels as f64;
    let class_counts = class_counts(&synthesized.class_by_texel);
    let static_occupied_texels = class_counts
        .get(PoliticalColorTexelClass::StaticControllerDiffers.as_str())
        .copied()
        .unwrap_or_default();
    let all_captured_texels_transparent_zero = secondary_map_all_transparent(&captured_bytes);
    let all_synthesized_texels_transparent_zero =
        secondary_map_all_transparent(&synthesized.secondary_color_bytes);
    let validation_is_vacuous =
        all_captured_texels_transparent_zero && all_synthesized_texels_transparent_zero;
    let mut notes = Vec::new();
    if all_captured_texels_transparent_zero {
        notes.push(
            "captured ProvinceSecondaryColorMap is all transparent zero: the game wrote no occupation stripes into this SRV for the captured frame"
                .to_string(),
        );
    }
    if validation_is_vacuous {
        notes.push(
            "VACUOUS MATCH: synthesized and captured secondary maps are both fully transparent, so a byte-exact / 100% texel match does NOT validate the province-id -> occupation-stripe mapping; no non-transparent texel was exercised"
                .to_string(),
        );
    }
    if static_occupied_texels == 0 {
        notes.push(
            "file-derived initial (1936-start) state controllers match owners for all sampled land texels, so the synthesized static secondary color remains transparent; ongoing-save controller/occupation state is not available from static files"
                .to_string(),
        );
    }

    let report = PoliticalColorComparisonReport {
        capture_dir: capture_dir.to_path_buf(),
        captured_file,
        generated_descriptor: synthesized.descriptor.clone(),
        captured_descriptor,
        byte_exact: byte_mismatches == 0,
        byte_mismatches,
        texel_mismatches,
        texel_match_percent,
        channel_mismatches,
        class_counts,
        mismatch_class_counts,
        mismatch_samples: samples,
        all_captured_texels_transparent_zero,
        all_synthesized_texels_transparent_zero,
        validation_is_vacuous,
        static_occupied_texels,
        notes,
    };
    Ok((report, captured_bytes))
}

pub fn dump_political_color_comparison(
    resolver: &GameFileResolver,
    capture_dir: &Path,
    out_dir: &Path,
) -> Result<(PoliticalColorComparisonReport, PoliticalColorDumpPaths)> {
    let synthesized = synthesize_political_colors(resolver)?;
    let (report, captured_bytes) = compare_synthesized_to_capture(&synthesized, capture_dir)?;
    fs::create_dir_all(out_dir)?;
    let secondary_path = out_dir.join(PROVINCE_SECONDARY_SYNTHETIC_FILE);
    let owner_path = out_dir.join(PROVINCE_OWNER_SYNTHETIC_FILE);
    let heatmap_path = out_dir.join(PROVINCE_SECONDARY_DIFF_HEATMAP_FILE);
    let stats_path = out_dir.join(POLITICAL_COLOR_STATS_FILE);
    write_rgba_raw(&secondary_path, &synthesized.secondary_color_bytes)?;
    write_rgba_raw(&owner_path, &synthesized.owner_color_bytes)?;
    write_diff_heatmap_bmp(
        &heatmap_path,
        synthesized.width,
        synthesized.height,
        &synthesized.secondary_color_bytes,
        &captured_bytes,
    )?;
    write_json(&stats_path, &report)?;
    Ok((
        report,
        PoliticalColorDumpPaths {
            synthesized_secondary_rgba: secondary_path,
            synthesized_owner_rgba: owner_path,
            diff_heatmap_bmp: heatmap_path,
            stats_json: stats_path,
        },
    ))
}

pub fn political_color_composition_law() -> PoliticalColorCompositionLaw {
    PoliticalColorCompositionLaw {
        captured_symbol: PROVINCE_SECONDARY_COLOR_MAP_SYMBOL.to_string(),
        texture_space: "map-space texture sampled at Input.uv2; captured dimensions are half MAP_SIZE_X/Y (2816x1024 for MAP_SIZE 5632x2048), with source BMP rows flipped to match uv2.y = 1 - map_y/MAP_SIZE_Y".to_string(),
        static_primary_source: "owner/controller tags from initial history/states owner/controller assignments resolved to common/countries colors; emitted separately as an unvalidated owner-color diagnostic texture".to_string(),
        static_secondary_source: "transparent when controller == owner; owner color with alpha only for static initial controller/owner differences".to_string(),
        owner_controller_rule: "initial controller defaults to owner; dated history blocks are ignored for the 1936 captured start".to_string(),
        sea_unowned_rule: "sea, lake, unknown province, no-state, and no-owner texels are transparent in the synthesized secondary mask".to_string(),
        impassable_rule: "impassable land is transparent in the secondary mask; primary political gray is not validated by the captured secondary SRV".to_string(),
        dynamic_inputs_needed: vec![
            "runtime controller changes after 1936".to_string(),
            "province-level occupation and resistance/compliance ownership state".to_string(),
            "cosmetic tag changes and civil-war/ideology color variants after game start".to_string(),
            "primary political fill SRV or render-target evidence; live_PAN2 only captured ProvinceSecondaryColorMap".to_string(),
        ],
        shader_sources: vec![
            "gfx/FX/pdxmap.shader:68-76 declares ProvinceSecondaryColorMap at sampler Index 6 with Linear/Wrap".to_string(),
            "gfx/FX/pdxmap.shader:327-328 applies secondary_color_mask(diffuse.rgb, ..., Input.uv2, ProvinceSecondaryColorMap, ...)".to_string(),
            "gfx/FX/standardfuncsgfx.fxh:979-987 samples TexMaskSampler at vUV and lerps to vColorMask.rgb by CalculateOccupationMask(vUV) * alpha".to_string(),
            "gfx/FX/standardfuncsgfx.fxh:1065-1073 contains the separate TerrainIDMap tile-index decode law; ProvinceSecondaryColorMap is not indexed through TerrainIDMap in the shader".to_string(),
        ],
        file_sources: vec![
            "common/countries/colors.txt color overrides".to_string(),
            "common/country_tags/*.txt country tag file bindings".to_string(),
            "common/countries/*.txt fallback country color assignments".to_string(),
            "history/states/*.txt initial owner/controller/provinces".to_string(),
            "map/definition.csv province type and map/provinces.bmp province id at map-space texels".to_string(),
        ],
    }
}

pub(crate) fn political_texel_colors(
    province_id: u32,
    definitions: &BTreeMap<u32, ProvinceDefinition>,
    province_to_state: &BTreeMap<u32, u32>,
    states: &BTreeMap<u32, StateMeta>,
    state_owners: &BTreeMap<u32, String>,
    state_controls: &BTreeMap<u32, InitialStateControl>,
    country_colors: &BTreeMap<String, [u8; 3]>,
) -> Result<(PoliticalColorTexelClass, Option<[u8; 4]>, Option<[u8; 4]>)> {
    let Some(definition) = definitions.get(&province_id) else {
        return Ok((PoliticalColorTexelClass::UnknownProvince, None, None));
    };
    if definition.province_type == "sea" || definition.province_type == "lake" {
        return Ok((PoliticalColorTexelClass::SeaOrLake, None, None));
    }
    let Some(state_id) = province_to_state.get(&province_id).copied() else {
        return Ok((PoliticalColorTexelClass::NoState, None, None));
    };
    let Some(state) = states.get(&state_id) else {
        return Ok((PoliticalColorTexelClass::NoState, None, None));
    };
    if state.impassable {
        return Ok((PoliticalColorTexelClass::Impassable, None, None));
    }
    let owner = state_controls
        .get(&state_id)
        .and_then(|control| control.owner.as_deref())
        .or_else(|| state_owners.get(&state_id).map(String::as_str));
    let Some(owner) = owner else {
        return Ok((PoliticalColorTexelClass::NoOwner, None, None));
    };
    let owner_color = rgba_opaque(require_country_color(country_colors, owner)?);
    let controller = state_controls
        .get(&state_id)
        .and_then(|control| control.controller.as_deref())
        .unwrap_or(owner);
    if controller == owner {
        return Ok((
            PoliticalColorTexelClass::OwnerControllerEqual,
            Some(owner_color),
            None,
        ));
    }
    let controller_color = rgba_opaque(require_country_color(country_colors, controller)?);
    Ok((
        PoliticalColorTexelClass::StaticControllerDiffers,
        Some(controller_color),
        Some(owner_color),
    ))
}

fn require_country_color(country_colors: &BTreeMap<String, [u8; 3]>, tag: &str) -> Result<[u8; 3]> {
    country_colors.get(tag).copied().ok_or_else(|| {
        format!("missing country color for tag {tag}; political synthesis fails closed").into()
    })
}

fn rgba_opaque(rgb: [u8; 3]) -> [u8; 4] {
    [rgb[0], rgb[1], rgb[2], 255]
}

fn load_initial_state_controls(
    state_files: &[(String, PathBuf)],
) -> Result<BTreeMap<u32, InitialStateControl>> {
    let mut out = BTreeMap::new();
    for (_, path) in state_files {
        let text = strip_comments(&read_latin1(path)?);
        let Some(state_id) = parse_u32_value(&text, "id") else {
            continue;
        };
        let initial_history = extract_block_after(&text, "history", 0)
            .map(|(inner, _)| initial_history_without_dated_blocks(&inner))
            .unwrap_or_default();
        out.insert(
            state_id,
            InitialStateControl {
                owner: parse_word_value(&initial_history, "owner"),
                controller: parse_word_value(&initial_history, "controller"),
            },
        );
    }
    Ok(out)
}

fn initial_history_without_dated_blocks(history: &str) -> String {
    let mut out = Vec::new();
    let mut depth = 0i32;
    for line in history.lines() {
        let trimmed = line.trim_start();
        if depth == 0 && looks_like_date_assignment(trimmed) {
            break;
        }
        out.push(line);
        depth += line.bytes().filter(|b| *b == b'{').count() as i32;
        depth -= line.bytes().filter(|b| *b == b'}').count() as i32;
        depth = depth.max(0);
    }
    out.join("\n")
}

fn looks_like_date_assignment(line: &str) -> bool {
    let bytes = line.as_bytes();
    if bytes.len() < 10 || !bytes[0].is_ascii_digit() {
        return false;
    }
    let mut idx = 0usize;
    let mut dots = 0usize;
    while idx < bytes.len() {
        let b = bytes[idx];
        if b.is_ascii_digit() {
            idx += 1;
        } else if b == b'.' {
            dots += 1;
            idx += 1;
        } else {
            break;
        }
    }
    if dots < 2 {
        return false;
    }
    let rest = &line[idx..];
    rest.trim_start().starts_with('=')
}

fn political_source_pixel_for_texel(
    texel_x: usize,
    texel_y: usize,
    map_width: usize,
    map_height: usize,
    texture_width: usize,
    texture_height: usize,
) -> (usize, usize) {
    let x = (((texel_x as f64 + 0.5) * map_width as f64) / texture_width as f64).floor() as usize;
    let unflipped_y =
        (((texel_y as f64 + 0.5) * map_height as f64) / texture_height as f64).floor() as usize;
    let y = map_height - 1 - unflipped_y.min(map_height - 1);
    (x.min(map_width - 1), y)
}

fn province_secondary_color_sampler() -> PoliticalColorSamplerDescriptor {
    PoliticalColorSamplerDescriptor {
        mag_filter: "Linear".to_string(),
        min_filter: "Linear".to_string(),
        mip_filter: "Linear".to_string(),
        address_u: "Wrap".to_string(),
        address_v: "Wrap".to_string(),
    }
}

fn load_captured_province_secondary_color_map(
    capture_dir: &Path,
) -> Result<(ProvinceSecondaryColorCaptureJson, PathBuf, Vec<u8>)> {
    let capture_dir = require_dir(capture_dir, "ProvinceSecondaryColorMap capture directory")?;
    let capture_json = capture_dir.join(PROVINCE_SECONDARY_CAPTURE_JSON);
    let capture_meta: ProvinceSecondaryColorCaptureJson =
        serde_json::from_slice(&fs::read(&capture_json)?)?;
    if capture_meta.schema != "battle-plan-runtime-texture-capture-v1" {
        return Err(format!(
            "{} has unsupported schema {}",
            capture_json.display(),
            capture_meta.schema
        )
        .into());
    }
    if capture_meta.symbol != PROVINCE_SECONDARY_COLOR_MAP_SYMBOL {
        return Err(format!(
            "{} captures {}, expected {}",
            capture_json.display(),
            capture_meta.symbol,
            PROVINCE_SECONDARY_COLOR_MAP_SYMBOL
        )
        .into());
    }
    let captured_file = capture_dir.join(&capture_meta.file);
    let captured_bytes = fs::read(&captured_file)?;
    Ok((capture_meta, captured_file, captured_bytes))
}

fn validate_captured_province_secondary_descriptor(
    descriptor: &PoliticalColorTextureDescriptor,
) -> Result<()> {
    if descriptor.symbol != PROVINCE_SECONDARY_COLOR_MAP_SYMBOL {
        return Err(format!(
            "captured political color symbol is {}, expected {}",
            descriptor.symbol, PROVINCE_SECONDARY_COLOR_MAP_SYMBOL
        )
        .into());
    }
    if descriptor.width != PROVINCE_COLOR_MAP_WIDTH
        || descriptor.height != PROVINCE_COLOR_MAP_HEIGHT
    {
        return Err(format!(
            "captured ProvinceSecondaryColorMap dimensions are {}x{}, expected {}x{}",
            descriptor.width,
            descriptor.height,
            PROVINCE_COLOR_MAP_WIDTH,
            PROVINCE_COLOR_MAP_HEIGHT
        )
        .into());
    }
    if descriptor.dxgi_format != PROVINCE_COLOR_MAP_DXGI_FORMAT {
        return Err(format!(
            "captured ProvinceSecondaryColorMap source DXGI format is {}, expected {}",
            descriptor.dxgi_format, PROVINCE_COLOR_MAP_DXGI_FORMAT
        )
        .into());
    }
    if descriptor.shader_slot != PROVINCE_SECONDARY_COLOR_MAP_SHADER_SLOT
        && descriptor.shader_slot != 15
        && descriptor.shader_slot != 3
    {
        return Err(format!(
            "captured ProvinceSecondaryColorMap shader slot is {}, expected pdxmap slot 6 (or pass-specific tree/water slots 15/3)",
            descriptor.shader_slot
        )
        .into());
    }
    Ok(())
}

/// Fail-closed test for a fully transparent secondary color map: every RGBA
/// texel must be exactly `[0,0,0,0]`. A non-multiple-of-4 length is treated as
/// non-transparent so a malformed buffer can never masquerade as "all zero".
fn secondary_map_all_transparent(bytes: &[u8]) -> bool {
    bytes.len() % 4 == 0 && bytes.chunks_exact(4).all(|px| px == [0u8, 0, 0, 0])
}

fn class_counts(classes: &[PoliticalColorTexelClass]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for class in classes {
        *out.entry(class.as_str().to_string()).or_insert(0) += 1;
    }
    out
}

fn write_diff_heatmap_bmp(
    path: &Path,
    width: usize,
    height: usize,
    generated: &[u8],
    captured: &[u8],
) -> Result<()> {
    if generated.len() != captured.len() || generated.len() != width * height * 4 {
        return Err(format!(
            "{} diff inputs do not match {}x{} RGBA",
            path.display(),
            width,
            height
        )
        .into());
    }
    let mut rgb = vec![0u8; width * height * 3];
    for idx in 0..width * height {
        let offset = idx * 4;
        let out = idx * 3;
        if generated[offset..offset + 4] == captured[offset..offset + 4] {
            rgb[out..out + 3].copy_from_slice(&[0, 0, 0]);
        } else {
            rgb[out..out + 3].copy_from_slice(&[255, 0, 0]);
        }
    }
    write_rgb24_bmp(path, width, height, &rgb)
}

fn write_rgb24_bmp(path: &Path, width: usize, height: usize, pixels: &[u8]) -> Result<()> {
    if pixels.len() != width * height * 3 {
        return Err(format!(
            "{} RGB24 pixel length is {}, expected {}",
            path.display(),
            pixels.len(),
            width * height * 3
        )
        .into());
    }
    let stride = (width * 3 + 3) & !3;
    let pixel_bytes = stride * height;
    let file_size = 14 + 40 + pixel_bytes;
    let mut file = fs::File::create(path)?;
    file.write_all(b"BM")?;
    file.write_all(&(file_size as u32).to_le_bytes())?;
    file.write_all(&[0u8; 4])?;
    file.write_all(&(54u32).to_le_bytes())?;
    file.write_all(&(40u32).to_le_bytes())?;
    file.write_all(&(width as i32).to_le_bytes())?;
    file.write_all(&(height as i32).to_le_bytes())?;
    file.write_all(&(1u16).to_le_bytes())?;
    file.write_all(&(24u16).to_le_bytes())?;
    file.write_all(&(0u32).to_le_bytes())?;
    file.write_all(&(pixel_bytes as u32).to_le_bytes())?;
    file.write_all(&(2835i32).to_le_bytes())?;
    file.write_all(&(2835i32).to_le_bytes())?;
    file.write_all(&(0u32).to_le_bytes())?;
    file.write_all(&(0u32).to_le_bytes())?;
    let padding = vec![0u8; stride - width * 3];
    for y_from_bottom in 0..height {
        let y = height - 1 - y_from_bottom;
        for x in 0..width {
            let offset = (y * width + x) * 3;
            let r = pixels[offset];
            let g = pixels[offset + 1];
            let b = pixels[offset + 2];
            file.write_all(&[b, g, r])?;
        }
        file.write_all(&padding)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::fs;

    #[test]
    fn colors_txt_known_tags_match_file_law() {
        let root = temp_test_dir("political_colors_known_tags");
        fs::create_dir_all(&root).expect("create fixture dir");
        let path = root.join("colors.txt");
        fs::write(
            &path,
            r#"
            GER = {
                color = HSV { 0.1 0.15 0.4 }
                color_ui = rgb { 138 155 116 }
            }
            SOV = { color = rgb { 125 13 24 } }
            USA = {
                color = rgb { 20 133 237 }
                color_ui = rgb { 87 160 255 }
            }
            "#,
        )
        .expect("write fixture colors");
        let colors = load_colors_txt_overrides(&path).expect("parse colors");
        assert_eq!(colors["GER"], [102, 96, 87]);
        assert_eq!(colors["SOV"], [125, 13, 24]);
        assert_eq!(colors["USA"], [20, 133, 237]);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn province_color_texel_uses_map_space_center_sample() {
        assert_eq!(political_source_pixel_for_texel(0, 0, 8, 4, 4, 2), (1, 2));
        assert_eq!(political_source_pixel_for_texel(1, 0, 8, 4, 4, 2), (3, 2));
        assert_eq!(political_source_pixel_for_texel(3, 1, 8, 4, 4, 2), (7, 0));
    }

    #[test]
    fn unknown_country_color_fails_closed() {
        let colors = BTreeMap::new();
        let err = require_country_color(&colors, "XYZ").expect_err("unknown tag must fail");
        assert!(err
            .to_string()
            .contains("missing country color for tag XYZ"));
    }

    #[test]
    fn all_transparent_detection_is_fail_closed() {
        // A genuinely all-zero RGBA buffer is transparent.
        assert!(secondary_map_all_transparent(&[0u8; 16]));
        assert!(secondary_map_all_transparent(&[]));
        // Any single non-zero channel (including a non-zero alpha) is not transparent.
        assert!(!secondary_map_all_transparent(&[0, 0, 0, 1]));
        assert!(!secondary_map_all_transparent(&[7, 0, 0, 0]));
        let mut buf = vec![0u8; 4096];
        buf[2050] = 255; // one stray texel deep in the buffer
        assert!(!secondary_map_all_transparent(&buf));
        // A malformed (non-multiple-of-4) buffer must never report transparent.
        assert!(!secondary_map_all_transparent(&[0u8; 5]));
        assert!(!secondary_map_all_transparent(&[0u8; 3]));
    }

    #[test]
    fn vacuous_match_requires_both_sides_transparent() {
        // Both transparent -> vacuous (a 100% match here proves nothing).
        let captured = vec![0u8; 4096];
        let synth = vec![0u8; 4096];
        let cap0 = secondary_map_all_transparent(&captured);
        let syn0 = secondary_map_all_transparent(&synth);
        assert!(cap0 && syn0);
        assert!(cap0 && syn0, "both transparent must flag vacuous");
        // A non-transparent captured map makes the comparison non-vacuous even
        // if the synthesized side is transparent.
        let mut captured_nz = vec![0u8; 4096];
        captured_nz[7] = 128;
        assert!(!secondary_map_all_transparent(&captured_nz));
        assert!(!(secondary_map_all_transparent(&captured_nz) && syn0));
    }

    fn land_definition() -> ProvinceDefinition {
        ProvinceDefinition {
            rgb: [0, 0, 0],
            province_type: "land".to_string(),
            coastal: false,
            terrain: "plains".to_string(),
            continent: 1,
        }
    }

    fn memoized_render_fixture() -> (
        BTreeMap<u32, ProvinceDefinition>,
        BmpIds,
        BTreeMap<u32, u32>,
        BTreeMap<u32, StateMeta>,
        BTreeMap<u32, String>,
        BTreeMap<u32, InitialStateControl>,
        BTreeMap<String, [u8; 3]>,
    ) {
        let mut definitions = BTreeMap::new();
        definitions.insert(1, land_definition());
        let mut sea = land_definition();
        sea.province_type = "sea".to_string();
        definitions.insert(2, sea);
        definitions.insert(3, land_definition());
        // Province id 0 is deliberately absent: unknown-province texels.
        let provinces = BmpIds {
            width: 4,
            height: 2,
            ids: vec![1, 2, 3, 0, 1, 2, 3, 0],
            unknown_pixels: 0,
        };
        let mut province_to_state = BTreeMap::new();
        province_to_state.insert(1, 10);
        province_to_state.insert(3, 30);
        let mut states = BTreeMap::new();
        states.insert(10, StateMeta::default());
        states.insert(30, StateMeta::default());
        let mut state_owners = BTreeMap::new();
        state_owners.insert(10, "GER".to_string());
        let mut state_controls = BTreeMap::new();
        state_controls.insert(
            10,
            InitialStateControl {
                owner: None,
                controller: None,
            },
        );
        state_controls.insert(
            30,
            InitialStateControl {
                owner: Some("POL".to_string()),
                controller: Some("GER".to_string()),
            },
        );
        let mut country_colors = BTreeMap::new();
        country_colors.insert("GER".to_string(), [1, 2, 3]);
        country_colors.insert("POL".to_string(), [4, 5, 6]);
        (
            definitions,
            provinces,
            province_to_state,
            states,
            state_owners,
            state_controls,
            country_colors,
        )
    }

    #[test]
    fn memoized_render_matches_per_texel_law() {
        let (
            definitions,
            provinces,
            province_to_state,
            states,
            state_owners,
            state_controls,
            country_colors,
        ) = memoized_render_fixture();
        let (owner_bytes, secondary_bytes, class_by_texel) = render_political_texels(
            PROVINCE_COLOR_MAP_WIDTH,
            PROVINCE_COLOR_MAP_HEIGHT,
            &definitions,
            &provinces,
            &province_to_state,
            &states,
            &state_owners,
            &state_controls,
            &country_colors,
        )
        .expect("render succeeds");
        // Sampled texels must match the unmemoized per-texel law exactly. The
        // prime stride keeps the test fast while covering every province band
        // (each band is 704 texels wide) and both vertical halves.
        for y in (0..PROVINCE_COLOR_MAP_HEIGHT).step_by(53) {
            for x in (0..PROVINCE_COLOR_MAP_WIDTH).step_by(53) {
                let texel_index = y * PROVINCE_COLOR_MAP_WIDTH + x;
                let offset = texel_index * 4;
                let (source_x, source_y) = political_source_pixel_for_texel(
                    x,
                    y,
                    provinces.width,
                    provinces.height,
                    PROVINCE_COLOR_MAP_WIDTH,
                    PROVINCE_COLOR_MAP_HEIGHT,
                );
                let province_id = provinces.ids[source_y * provinces.width + source_x];
                let (class, owner_color, secondary_color) = political_texel_colors(
                    province_id,
                    &definitions,
                    &province_to_state,
                    &states,
                    &state_owners,
                    &state_controls,
                    &country_colors,
                )
                .expect("per-texel law succeeds");
                assert_eq!(class_by_texel[texel_index], class, "class at {x},{y}");
                let expected_owner = owner_color.unwrap_or([0, 0, 0, 0]);
                let expected_secondary = secondary_color.unwrap_or([0, 0, 0, 0]);
                assert_eq!(
                    owner_bytes[offset..offset + 4],
                    expected_owner,
                    "owner bytes at {x},{y}"
                );
                assert_eq!(
                    secondary_bytes[offset..offset + 4],
                    expected_secondary,
                    "secondary bytes at {x},{y}"
                );
            }
        }
        // Spot-check that each fixture class actually appeared (the fixture
        // exercises equal-owner, sea, occupied, and unknown provinces).
        let counts = class_counts(&class_by_texel);
        assert_eq!(
            counts.values().sum::<usize>(),
            PROVINCE_COLOR_MAP_WIDTH * PROVINCE_COLOR_MAP_HEIGHT
        );
        assert!(counts.contains_key(PoliticalColorTexelClass::OwnerControllerEqual.as_str()));
        assert!(counts.contains_key(PoliticalColorTexelClass::SeaOrLake.as_str()));
        assert!(counts.contains_key(PoliticalColorTexelClass::StaticControllerDiffers.as_str()));
        assert!(counts.contains_key(PoliticalColorTexelClass::UnknownProvince.as_str()));
    }

    #[test]
    fn memoized_render_propagates_fail_closed_country_color_error() {
        let (
            definitions,
            provinces,
            province_to_state,
            states,
            state_owners,
            state_controls,
            mut country_colors,
        ) = memoized_render_fixture();
        // Province 3's state owner is POL; removing its color must refuse the
        // whole render, exactly as the unmemoized loop did.
        country_colors.remove("POL");
        let err = render_political_texels(
            PROVINCE_COLOR_MAP_WIDTH,
            PROVINCE_COLOR_MAP_HEIGHT,
            &definitions,
            &provinces,
            &province_to_state,
            &states,
            &state_owners,
            &state_controls,
            &country_colors,
        )
        .expect_err("missing country color must fail closed");
        assert!(err
            .to_string()
            .contains("missing country color for tag POL"));
    }

    #[test]
    fn dated_history_blocks_do_not_affect_1936_controller() {
        let history = r#"
            owner = CHI
            victory_points = {
                10076 3
            }
            1938.10.25 = {
                controller = JAP
            }
        "#;
        let initial = initial_history_without_dated_blocks(history);
        assert_eq!(parse_word_value(&initial, "owner").as_deref(), Some("CHI"));
        assert_eq!(parse_word_value(&initial, "controller"), None);
    }

    #[test]
    fn xmap05_half_resolution_selects_the_center_pixel_without_a_vote() {
        // For a 4x4 source, output (0,0) selects source (1,2). A 2x2 block
        // may contain any three other identities; they never participate in a
        // majority or numeric tie-break.
        assert_eq!(xmap05_half_res_source_index(4, 4, 0, 0), 2 * 4 + 1);
        assert_eq!(xmap05_half_res_source_index(4, 4, 1, 1), 0 * 4 + 3);
    }

    #[test]
    fn xmap05_censuses_reject_missing_extra_and_foreign_authority() {
        let states = BTreeSet::from([1, 2]);
        let roster = BTreeSet::from(["AAA".to_string(), "BBB".to_string()]);
        let mut keying = KeyingTables {
            state_owners: BTreeMap::from([(1, "AAA".into()), (2, "BBB".into())]),
            state_controls: BTreeMap::from([
                (
                    1,
                    InitialStateControl {
                        owner: Some("AAA".into()),
                        controller: Some("AAA".into()),
                    },
                ),
                (
                    2,
                    InitialStateControl {
                        owner: Some("BBB".into()),
                        controller: Some("BBB".into()),
                    },
                ),
            ]),
            cosmetic_by_tag: BTreeMap::new(),
        };
        let mut provinces = BTreeMap::from([(10, "AAA".into()), (20, "BBB".into())]);
        validate_xmap05_state_authority(&states, &keying, &provinces, &roster).unwrap();
        validate_xmap05_province_census(&BTreeSet::from([10, 20]), &provinces).unwrap();

        keying.state_controls.remove(&2);
        assert!(
            validate_xmap05_state_authority(&states, &keying, &provinces, &roster)
                .unwrap_err()
                .to_string()
                .contains("state census")
        );
        keying.state_controls.insert(
            2,
            InitialStateControl {
                owner: Some("BBB".into()),
                controller: Some("BBB".into()),
            },
        );

        provinces.insert(30, "AAA".into());
        assert!(
            validate_xmap05_province_census(&BTreeSet::from([10, 20]), &provinces)
                .unwrap_err()
                .to_string()
                .contains("province-controller census")
        );
        provinces.remove(&30);

        provinces.insert(20, "ZZZ".into());
        assert!(
            validate_xmap05_state_authority(&states, &keying, &provinces, &roster)
                .unwrap_err()
                .to_string()
                .contains("absent from the complete D18 roster")
        );
    }
}
