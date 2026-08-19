use crate::{
    border_diffuse_source, border_fade_windows_from_graphics_defines, latin1_bytes_to_string,
    synthesize_border_world_mesh_with_keying, AssignedStage, BorderChain, BorderFadeWindow,
    BorderFadeWindows, BorderMaterial, BorderWorldMesh, FoundationFeatureContext,
    FoundationFeatureOutput, GameFileResolver, KeyingTables, ProductionMapFeatureAdapter, Result,
    BORDER_ADJACENCIES_CSV_SOURCE, BORDER_DEFAULT_MAP_SOURCE, BORDER_DEFINITION_CSV_SOURCE,
    BORDER_HEIGHTMAP_BMP_SOURCE, BORDER_HISTORY_STATES_DIR, BORDER_PROVINCES_BMP_SOURCE,
    BORDER_SHADER_SOURCE, BORDER_VERTEX_STRIDE_BYTES,
};
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{FamilyDisposition, FoundationGap, MapVisualFamily};
use replay_core::{Leaf, ValidatedContentAdmission, ViewStatus, WorldFrame};
use serde::Deserialize;
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

pub const XMAP06_THIN_BORDER_SCHEMA: &str = "hoi4.xmap06.thin-border-prepared.a729.v7";
pub const XMAP06_BORDER_CB2_BYTES: u32 = 32;
pub const XMAP06_RUNTIME_BUCKET_GAP_ID: &str = "XMAP06.runtime_bucket_carrier_unavailable";
pub const XMAP06_EARLY_STAGE_NAME: &str = "early_borders";
pub const XMAP06_EARLY_OPERATION_NAME: &str = "thin-borders.early.draw";
pub const XMAP06_LATE_STAGE_NAME: &str = "late_borders";
pub const XMAP06_LATE_OPERATION_NAME: &str = "thin-borders.late.draw";
pub const XMAP06_BUILTIN_MAP_MODE_DEFAULT: &str = "builtin:MAPMODE_DEFAULT";
pub const XMAP06_BUILTIN_MAP_MODE_RESOURCES: &str = "builtin:MAPMODE_RESOURCES";
pub const XMAP06_BUILTIN_MAP_MODE_INFRASTRUCTURE: &str = "builtin:MAPMODE_INFRASTRUCTURE";
pub const XMAP06_BUILTIN_MAP_MODE_MANPOWER: &str = "builtin:MAPMODE_MANPOWER";

const BORDER_GRAPHICS_DEFINES: &str = "common/defines/00_graphics.lua";
const BORDER_PLATFORM_HEADER: &str = "gfx/FX/defines_hlsl_dx11.fxh";
const XMAP06_RUNTIME_CARRIER_KEY: &str = "thin_border_runtime_a729";
const XMAP06_RUNTIME_CARRIER_AUTHORITY: &str = "exact_a729_cpdxmap_border_draw_payload_v4";
const XMAP06_A729_BINARY_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const XMAP06_BUCKET_WRITER_RVA: u32 = 0x0126_a450;
const XMAP06_PALETTE_WRITER_RVA: u32 = 0x0126_6aa0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedThinBorderFadeWindow {
    pub near_f32_bits: u32,
    pub far_f32_bits: u32,
}

impl PreparedThinBorderFadeWindow {
    fn from_window(window: BorderFadeWindow) -> Self {
        Self {
            near_f32_bits: window.near.to_bits(),
            far_f32_bits: window.far.to_bits(),
        }
    }

    fn validate(self, label: &str) -> Result<()> {
        let near = f32::from_bits(self.near_f32_bits);
        let far = f32::from_bits(self.far_f32_bits);
        if !near.is_finite() || !far.is_finite() || near < 0.0 || far <= near {
            return Err(format!(
                "XMAP06 admitted {label} fade window is invalid: near={near} far={far}"
            )
            .into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedThinBorderFadeWindows {
    pub province: PreparedThinBorderFadeWindow,
    pub state: PreparedThinBorderFadeWindow,
}

impl PreparedThinBorderFadeWindows {
    fn from_windows(windows: BorderFadeWindows) -> Self {
        Self {
            province: PreparedThinBorderFadeWindow::from_window(windows.province),
            state: PreparedThinBorderFadeWindow::from_window(windows.state),
        }
    }

    fn validate(self) -> Result<()> {
        self.province.validate("province")?;
        self.state.validate("state")
    }

    pub fn transparency(self, material: BorderMaterial, camera_height: f32) -> Result<f32> {
        if !camera_height.is_finite() {
            return Err("XMAP06 border camera height is not finite".into());
        }
        let window = match material {
            BorderMaterial::Province => Some(self.province),
            BorderMaterial::State => Some(self.state),
            BorderMaterial::Country
            | BorderMaterial::Impassable
            | BorderMaterial::SeaCoast
            | BorderMaterial::SeaRegion => None,
        };
        let Some(window) = window else {
            return Ok(1.0);
        };
        let near = f32::from_bits(window.near_f32_bits);
        let far = f32::from_bits(window.far_f32_bits);
        let clamped = camera_height.clamp(near, far);
        Ok(1.0 - (clamped - near) / (far - near))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedThinBorderDrawPhase {
    Early,
    Late,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedThinBorderBucketBank {
    Base,
    Secondary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedThinBorderBucket {
    pub bucket_index: u32,
    pub phase: PreparedThinBorderDrawPhase,
    pub bank: PreparedThinBorderBucketBank,
    pub material: BorderMaterial,
}

pub const XMAP06_MATERIAL_ORDER: [BorderMaterial; 6] = BorderMaterial::ALL;

pub const XMAP06_DRAW_BUCKETS: [PreparedThinBorderBucket; 12] = [
    PreparedThinBorderBucket {
        bucket_index: 0,
        phase: PreparedThinBorderDrawPhase::Early,
        bank: PreparedThinBorderBucketBank::Base,
        material: BorderMaterial::Province,
    },
    PreparedThinBorderBucket {
        bucket_index: 1,
        phase: PreparedThinBorderDrawPhase::Early,
        bank: PreparedThinBorderBucketBank::Base,
        material: BorderMaterial::State,
    },
    PreparedThinBorderBucket {
        bucket_index: 2,
        phase: PreparedThinBorderDrawPhase::Early,
        bank: PreparedThinBorderBucketBank::Base,
        material: BorderMaterial::Country,
    },
    PreparedThinBorderBucket {
        bucket_index: 3,
        phase: PreparedThinBorderDrawPhase::Early,
        bank: PreparedThinBorderBucketBank::Base,
        material: BorderMaterial::Impassable,
    },
    PreparedThinBorderBucket {
        bucket_index: 4,
        phase: PreparedThinBorderDrawPhase::Late,
        bank: PreparedThinBorderBucketBank::Base,
        material: BorderMaterial::SeaCoast,
    },
    PreparedThinBorderBucket {
        bucket_index: 5,
        phase: PreparedThinBorderDrawPhase::Late,
        bank: PreparedThinBorderBucketBank::Base,
        material: BorderMaterial::SeaRegion,
    },
    PreparedThinBorderBucket {
        bucket_index: 6,
        phase: PreparedThinBorderDrawPhase::Late,
        bank: PreparedThinBorderBucketBank::Secondary,
        material: BorderMaterial::Province,
    },
    PreparedThinBorderBucket {
        bucket_index: 7,
        phase: PreparedThinBorderDrawPhase::Late,
        bank: PreparedThinBorderBucketBank::Secondary,
        material: BorderMaterial::State,
    },
    PreparedThinBorderBucket {
        bucket_index: 8,
        phase: PreparedThinBorderDrawPhase::Late,
        bank: PreparedThinBorderBucketBank::Secondary,
        material: BorderMaterial::Country,
    },
    PreparedThinBorderBucket {
        bucket_index: 9,
        phase: PreparedThinBorderDrawPhase::Late,
        bank: PreparedThinBorderBucketBank::Secondary,
        material: BorderMaterial::Impassable,
    },
    PreparedThinBorderBucket {
        bucket_index: 10,
        phase: PreparedThinBorderDrawPhase::Late,
        bank: PreparedThinBorderBucketBank::Secondary,
        material: BorderMaterial::SeaCoast,
    },
    PreparedThinBorderBucket {
        bucket_index: 11,
        phase: PreparedThinBorderDrawPhase::Late,
        bank: PreparedThinBorderBucketBank::Secondary,
        material: BorderMaterial::SeaRegion,
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedThinBorderClass {
    pub material: BorderMaterial,
    pub material_ordinal: u32,
    pub texture_content: UsedContentKey,
    pub index_count: u32,
    pub indices: Arc<[u8]>,
    pub indices_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedThinBorderBucketDraw {
    pub province_low: u32,
    pub province_high: u32,
    pub pair_record_ordinal: u32,
    pub bucket_draw_ordinal: u32,
    pub first_index: u32,
    pub index_count: u32,
    pub selector_reason_code_raw: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedThinBorderBucketGeometry {
    pub bucket: PreparedThinBorderBucket,
    pub index_count: u32,
    pub indices: Arc<[u8]>,
    pub indices_sha256: String,
    pub ordered_draws: Vec<PreparedThinBorderBucketDraw>,
    pub ordered_draws_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedThinBorderRuntimePartition {
    pub viewer_country_or_global: String,
    pub viewer_map_mode_id: String,
    pub province_fade_uses_viewer_map_mode: bool,
    pub province_fade_alternate_suppressed: bool,
    pub province_fade_suppressed: bool,
    pub carrier_payload_sha256: String,
    pub selection_palette_f32_bits: Vec<[u32; 4]>,
    pub selection_palette_sha256: String,
    pub buckets: Vec<PreparedThinBorderBucketGeometry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedThinBorderFrame {
    pub schema: String,
    pub hour_index: i64,
    pub early_stage: AssignedStage,
    pub late_stage: AssignedStage,
    pub map_width: u32,
    pub map_height: u32,
    pub owner_epoch_sha256: String,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub fade_windows: PreparedThinBorderFadeWindows,
    pub material_order: [BorderMaterial; 6],
    pub draw_buckets: [PreparedThinBorderBucket; 12],
    pub vertex_stride_bytes: u32,
    pub cb2_byte_width: u32,
    pub vertex_count: u32,
    pub vertices: Arc<[u8]>,
    pub vertices_sha256: String,
    pub classes: Vec<PreparedThinBorderClass>,
    pub runtime_partition: Option<PreparedThinBorderRuntimePartition>,
    pub deterministic_sha256: String,
}

impl PreparedThinBorderFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP06_THIN_BORDER_SCHEMA
            || self.map_width == 0
            || self.map_height == 0
            || self.vertex_stride_bytes != BORDER_VERTEX_STRIDE_BYTES as u32
            || self.cb2_byte_width != XMAP06_BORDER_CB2_BYTES
            || self.vertex_count == 0
            || self.vertices.len()
                != (self.vertex_count as usize)
                    .checked_mul(BORDER_VERTEX_STRIDE_BYTES)
                    .ok_or("XMAP06 vertex byte length overflow")?
            || self.vertices_sha256 != Sha256::hex(&self.vertices)
        {
            return Err("XMAP06 prepared border geometry is incomplete".into());
        }
        validate_sha256(&self.owner_epoch_sha256, "XMAP06 owner epoch")?;
        validate_sha256(&self.vertices_sha256, "XMAP06 vertex buffer")?;
        validate_assigned_stage_pair(&self.early_stage, &self.late_stage)?;
        self.fade_windows.validate()?;
        validate_static_draw_contract(&self.material_order, &self.draw_buckets)?;

        let mut used_paths = BTreeSet::new();
        for content in &self.ordered_used_content {
            content
                .validate()
                .map_err(|error| format!("XMAP06 used-content identity: {error}"))?;
            if !used_paths.insert(content.normalized_game_relative_path.as_str()) {
                return Err("XMAP06 used-content path is duplicated".into());
            }
        }
        if self.ordered_used_content.is_empty() {
            return Err("XMAP06 used-content closure is empty".into());
        }
        if self.ordered_used_content.windows(2).any(|window| {
            window[0].normalized_game_relative_path >= window[1].normalized_game_relative_path
        }) {
            return Err("XMAP06 used-content closure is not canonically ordered".into());
        }

        if self.classes.len() != BorderMaterial::ALL.len() {
            return Err("XMAP06 material-class census is incomplete".into());
        }
        let mut total_indices = 0u64;
        for (ordinal, (class, expected_material)) in
            self.classes.iter().zip(self.material_order).enumerate()
        {
            if class.material != expected_material
                || class.material_ordinal != ordinal as u32
                || class.texture_content.normalized_game_relative_path
                    != border_diffuse_source(class.material)
                || class.indices.len()
                    != (class.index_count as usize)
                        .checked_mul(std::mem::size_of::<u32>())
                        .ok_or("XMAP06 index byte length overflow")?
                || class.indices_sha256 != Sha256::hex(&class.indices)
            {
                return Err("XMAP06 material-class payload is invalid".into());
            }
            class
                .texture_content
                .validate()
                .map_err(|error| format!("XMAP06 material content identity: {error}"))?;
            if !self.ordered_used_content.contains(&class.texture_content) {
                return Err(
                    "XMAP06 material content is absent from the admitted content closure".into(),
                );
            }
            validate_sha256(&class.indices_sha256, "XMAP06 material index buffer")?;
            for bytes in class.indices.chunks_exact(4) {
                let index = u32::from_le_bytes(
                    bytes
                        .try_into()
                        .map_err(|_| "XMAP06 index buffer row is truncated")?,
                );
                if index >= self.vertex_count {
                    return Err("XMAP06 material index references a missing vertex".into());
                }
            }
            total_indices = total_indices
                .checked_add(class.index_count as u64)
                .ok_or("XMAP06 index census overflow")?;
        }
        if total_indices == 0 {
            return Err("XMAP06 synthesized no border indices".into());
        }
        if let Some(partition) = &self.runtime_partition {
            if !valid_viewer_context_key(&partition.viewer_country_or_global) {
                return Err("XMAP06 runtime partition viewer context is invalid".into());
            }
            let expected_suppression = xmap06_province_fade_suppressed(
                &partition.viewer_map_mode_id,
                partition.province_fade_uses_viewer_map_mode,
                partition.province_fade_alternate_suppressed,
            )?;
            if partition.province_fade_suppressed != expected_suppression {
                return Err(
                    "XMAP06 runtime partition froze or corrupted province-fade suppression".into(),
                );
            }
            validate_sha256(
                &partition.carrier_payload_sha256,
                "XMAP06 runtime bucket carrier",
            )?;
            validate_sha256(
                &partition.selection_palette_sha256,
                "XMAP06 selection palette",
            )?;
            if partition.selection_palette_f32_bits.is_empty()
                || partition.selection_palette_f32_bits.first() != Some(&[0; 4])
                || partition.selection_palette_sha256
                    != selection_palette_sha256(&partition.selection_palette_f32_bits)
            {
                return Err("XMAP06 selection palette payload is invalid".into());
            }
            if partition.buckets.len() != XMAP06_DRAW_BUCKETS.len() {
                return Err("XMAP06 runtime bucket census is incomplete".into());
            }
            let mut partition_indices = 0u64;
            let mut partition_draw_keys = BTreeSet::new();
            for (geometry, expected) in partition.buckets.iter().zip(XMAP06_DRAW_BUCKETS) {
                if geometry.bucket != expected
                    || geometry.indices.len()
                        != (geometry.index_count as usize)
                            .checked_mul(std::mem::size_of::<u32>())
                            .ok_or("XMAP06 bucket index byte length overflow")?
                    || geometry.indices_sha256 != Sha256::hex(&geometry.indices)
                    || geometry.ordered_draws_sha256
                        != prepared_bucket_draws_sha256(geometry.bucket, &geometry.ordered_draws)
                {
                    return Err("XMAP06 runtime bucket payload is invalid".into());
                }
                validate_sha256(&geometry.indices_sha256, "XMAP06 bucket index buffer")?;
                validate_sha256(
                    &geometry.ordered_draws_sha256,
                    "XMAP06 bucket ordered draws",
                )?;
                let mut next_first_index = 0u32;
                let mut previous_reason = None;
                for (ordinal, draw) in geometry.ordered_draws.iter().enumerate() {
                    let draw_key = (
                        draw.province_low,
                        draw.province_high,
                        draw.pair_record_ordinal,
                    );
                    if draw.province_low == 0
                        || draw.province_low >= draw.province_high
                        || draw.bucket_draw_ordinal != ordinal as u32
                        || draw.first_index != next_first_index
                        || draw.index_count == 0
                        || draw.index_count % 3 != 0
                        || draw.selector_reason_code_raw as usize
                            >= partition.selection_palette_f32_bits.len()
                        || previous_reason
                            .is_some_and(|reason| reason > draw.selector_reason_code_raw)
                        || !partition_draw_keys.insert(draw_key)
                    {
                        return Err("XMAP06 runtime bucket ordered draw is invalid".into());
                    }
                    next_first_index = next_first_index
                        .checked_add(draw.index_count)
                        .ok_or("XMAP06 runtime bucket draw range overflow")?;
                    previous_reason = Some(draw.selector_reason_code_raw);
                }
                if next_first_index != geometry.index_count {
                    return Err("XMAP06 runtime bucket draws do not cover its index buffer".into());
                }
                for bytes in geometry.indices.chunks_exact(4) {
                    let index = u32::from_le_bytes(
                        bytes
                            .try_into()
                            .map_err(|_| "XMAP06 bucket index row is truncated")?,
                    );
                    if index >= self.vertex_count {
                        return Err("XMAP06 bucket index references a missing vertex".into());
                    }
                }
                partition_indices = partition_indices
                    .checked_add(geometry.index_count as u64)
                    .ok_or("XMAP06 runtime bucket index census overflow")?;
            }
            if partition_indices > total_indices {
                return Err(
                    "XMAP06 runtime bucket partition duplicates synthesized geometry".into(),
                );
            }
        }
        validate_sha256(&self.deterministic_sha256, "XMAP06 prepared border frame")?;
        if self.deterministic_sha256 != prepared_thin_border_digest(self)? {
            return Err("XMAP06 prepared border digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawThinBorderRuntimeCarrier {
    schema_version: u32,
    authority: String,
    source_binary_sha256: String,
    producer_function_rva: u32,
    palette_producer_function_rva: u32,
    hour_index: i64,
    complete: bool,
    contexts_sha256: String,
    contexts: BTreeMap<String, RawThinBorderRuntimeContext>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawThinBorderRuntimeContext {
    province_fade_uses_viewer_map_mode: bool,
    province_fade_alternate_suppressed: bool,
    palette_entry_count: u32,
    selection_palette_f32_bits: Vec<[u32; 4]>,
    payload_sha256: String,
    records: Vec<RawThinBorderRuntimeRecord>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawThinBorderRuntimeRecord {
    province_low: u32,
    province_high: u32,
    pair_record_ordinal: u32,
    base_material_ordinal: Option<u8>,
    secondary_bank: bool,
    bucket_draw_ordinal: Option<u32>,
    selector_reason_code_raw: u8,
}

#[derive(Clone)]
struct AdmittedThinBorderRuntimeRecord {
    key: (u32, u32, u32),
    base_material: Option<BorderMaterial>,
    secondary_bank: bool,
    bucket_draw_ordinal: Option<u32>,
    selector_reason_code_raw: u8,
}

struct AdmittedThinBorderRuntimeCarrier {
    viewer_country_or_global: String,
    province_fade_uses_viewer_map_mode: bool,
    province_fade_alternate_suppressed: bool,
    payload_sha256: String,
    selection_palette_f32_bits: Vec<[u32; 4]>,
    selection_palette_sha256: String,
    records: Vec<AdmittedThinBorderRuntimeRecord>,
}

#[derive(Clone)]
struct Xmap06Content {
    resolver: GameFileResolver,
    ordered_used_content: Vec<UsedContentKey>,
    material_content: BTreeMap<BorderMaterial, UsedContentKey>,
    fade_windows: PreparedThinBorderFadeWindows,
}

impl Xmap06Content {
    fn admit(resolver: &GameFileResolver, admission: &ValidatedContentAdmission) -> Result<Self> {
        let translation_unit =
            crate::shader_compile::effect_translation_unit_files(resolver, BORDER_SHADER_SOURCE)?;
        let effective_states =
            resolver.audit_effective_dir_files(BORDER_HISTORY_STATES_DIR, Some("txt"))?;
        if effective_states.is_empty() {
            return Err("XMAP06 effective history/states census is empty".into());
        }

        let mut required = BTreeSet::from([
            BORDER_DEFAULT_MAP_SOURCE.to_string(),
            BORDER_DEFINITION_CSV_SOURCE.to_string(),
            BORDER_PROVINCES_BMP_SOURCE.to_string(),
            BORDER_HEIGHTMAP_BMP_SOURCE.to_string(),
            BORDER_ADJACENCIES_CSV_SOURCE.to_string(),
            BORDER_GRAPHICS_DEFINES.to_string(),
            BORDER_PLATFORM_HEADER.to_string(),
        ]);
        required.extend(translation_unit.into_iter().map(|(path, _)| path));
        required.extend(effective_states);
        required.extend(
            BorderMaterial::ALL
                .into_iter()
                .map(border_diffuse_source)
                .map(str::to_string),
        );

        let mut ordered_used_content = Vec::with_capacity(required.len());
        for path in required {
            let content = admission
                .require_used_content(&path)
                .map_err(|error| format!("XMAP06 requires admitted {path}: {error}"))?
                .clone();
            resolver.read_admitted_used_content(&content)?;
            ordered_used_content.push(content);
        }
        ordered_used_content.sort_by(|left, right| {
            left.normalized_game_relative_path
                .cmp(&right.normalized_game_relative_path)
        });
        let graphics_content = ordered_used_content
            .iter()
            .find(|content| content.normalized_game_relative_path == BORDER_GRAPHICS_DEFINES)
            .ok_or("XMAP06 admitted content closure omitted graphics defines")?;
        let graphics_bytes = resolver.read_admitted_used_content(graphics_content)?;
        let fade_windows = PreparedThinBorderFadeWindows::from_windows(
            border_fade_windows_from_graphics_defines(&latin1_bytes_to_string(&graphics_bytes))?,
        );
        fade_windows.validate()?;

        let material_content = BorderMaterial::ALL
            .into_iter()
            .map(|material| {
                let path = border_diffuse_source(material);
                let content = admission
                    .require_used_content(path)
                    .map_err(|error| format!("XMAP06 requires admitted {path}: {error}"))?
                    .clone();
                Ok((material, content))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;

        Ok(Self {
            resolver: resolver.clone(),
            ordered_used_content,
            material_content,
            fade_windows,
        })
    }
}

struct Xmap06Runtime {
    cache_key: Option<String>,
    prepared: Option<PreparedThinBorderFrame>,
}

pub(crate) struct Xmap06ThinBorderAdapter {
    content: Arc<Xmap06Content>,
    runtime: Mutex<Xmap06Runtime>,
}

impl Xmap06ThinBorderAdapter {
    pub(crate) fn new(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
    ) -> Result<Self> {
        Ok(Self {
            content: Arc::new(Xmap06Content::admit(resolver, admission)?),
            runtime: Mutex::new(Xmap06Runtime {
                cache_key: None,
                prepared: None,
            }),
        })
    }

    fn prepare_geometry(
        &self,
        context: &FoundationFeatureContext<'_>,
    ) -> Result<PreparedThinBorderFrame> {
        let early_stage = context
            .require_assigned_stage(XMAP06_EARLY_STAGE_NAME, XMAP06_EARLY_OPERATION_NAME)?
            .clone();
        let late_stage = context
            .require_assigned_stage(XMAP06_LATE_STAGE_NAME, XMAP06_LATE_OPERATION_NAME)?
            .clone();
        validate_assigned_stage_pair(&early_stage, &late_stage)?;
        let (keying, owner_epoch_sha256) = admitted_owner_epoch(context.world_frame)?;
        let runtime_carrier = admit_runtime_bucket_carrier(
            context.world_frame,
            context.viewer.observer_country_id.as_deref(),
        )?;
        let runtime_identity = runtime_carrier
            .as_ref()
            .map(|carrier| carrier.payload_sha256.as_str())
            .unwrap_or("absent");
        let cache_key = xmap06_cache_key(
            context.world_frame.hour_index(),
            &owner_epoch_sha256,
            runtime_identity,
            &context.viewer.map_mode_id,
            &early_stage.assignment_sha256,
            &late_stage.assignment_sha256,
        );
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "XMAP06 runtime cache lock is poisoned")?;
        if runtime.cache_key.as_deref() == Some(cache_key.as_str()) {
            return runtime
                .prepared
                .clone()
                .ok_or_else(|| "XMAP06 cache identity has no payload".into());
        }

        let mesh = synthesize_border_world_mesh_with_keying(&self.content.resolver, &keying)?;
        let prepared = prepare_border_frame(
            context.world_frame.hour_index(),
            early_stage,
            late_stage,
            owner_epoch_sha256,
            self.content.ordered_used_content.clone(),
            &self.content.material_content,
            self.content.fade_windows,
            &mesh,
            &context.viewer.map_mode_id,
            runtime_carrier.as_ref(),
        )?;
        runtime.cache_key = Some(cache_key);
        runtime.prepared = Some(prepared.clone());
        Ok(prepared)
    }
}

fn xmap06_cache_key(
    hour_index: i64,
    owner_epoch_sha256: &str,
    runtime_identity: &str,
    viewer_map_mode_id: &str,
    early_assignment_sha256: &str,
    late_assignment_sha256: &str,
) -> String {
    Sha256::hex(
        format!(
            "XMAP06_CACHE_V6\0{hour_index}\0{owner_epoch_sha256}\0{runtime_identity}\0{viewer_map_mode_id}\0{early_assignment_sha256}\0{late_assignment_sha256}"
        )
        .as_bytes(),
    )
}

impl ProductionMapFeatureAdapter for Xmap06ThinBorderAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::ThinBorders
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        let prepared = self.prepare_geometry(context)?;
        let disposition = if let Some(partition) = &prepared.runtime_partition {
            let shader_sha256 = prepared
                .ordered_used_content
                .iter()
                .find(|content| content.normalized_game_relative_path == BORDER_SHADER_SOURCE)
                .ok_or("XMAP06 prepared closure omitted border.shader")?
                .payload_sha256
                .clone();
            if partition.buckets.len() != 12 {
                return Err("XMAP06 runtime partition omitted scheduled buckets".into());
            }
            let early_resources_sha256 = phase_resources_sha256(&prepared, 0, 4)?;
            let late_resources_sha256 = phase_resources_sha256(&prepared, 4, 12)?;
            let target_sha256 =
                Sha256::hex(format!("{}\0{}", prepared.map_width, prepared.map_height).as_bytes());
            let early_operation = prepared
                .early_stage
                .foundation_operation_identity(
                    shader_sha256.clone(),
                    early_resources_sha256,
                    target_sha256.clone(),
                )
                .map_err(|error| format!("XMAP06 early foundation operation: {error}"))?;
            let late_operation = prepared
                .late_stage
                .foundation_operation_identity(shader_sha256, late_resources_sha256, target_sha256)
                .map_err(|error| format!("XMAP06 late foundation operation: {error}"))?;
            FamilyDisposition::Present {
                payload_schema: XMAP06_THIN_BORDER_SCHEMA.into(),
                producer_registration_sha256: Sha256::hex(
                    b"XMAP06_THIN_BORDER_PRODUCTION_ADAPTER_V6",
                ),
                payload_sha256: prepared.deterministic_sha256.clone(),
                ordered_operations: vec![early_operation, late_operation],
                ordered_used_content: prepared.ordered_used_content.clone(),
            }
        } else {
            FamilyDisposition::Unavailable {
                gap: runtime_bucket_carrier_gap(
                    context.world_frame.hour_index(),
                    &prepared.deterministic_sha256,
                ),
            }
        };
        Ok(FoundationFeatureOutput {
            disposition,
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: Some(prepared),
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_lighting: None,
            prepared_rail_supply: None,
            prepared_weather: None,
            prepared_effects: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_map_modes: None,
            consumed_gap_ids: Vec::new(),
        })
    }
}

fn phase_resources_sha256(
    prepared: &PreparedThinBorderFrame,
    first_bucket: usize,
    end_bucket: usize,
) -> Result<String> {
    let partition = prepared
        .runtime_partition
        .as_ref()
        .ok_or("XMAP06 phase resources require a runtime partition")?;
    let buckets = partition
        .buckets
        .get(first_bucket..end_bucket)
        .ok_or("XMAP06 phase resource bucket range is invalid")?;
    let mut identity = b"XMAP06_ASSIGNED_PHASE_RESOURCES_V4\0".to_vec();
    identity.extend_from_slice(partition.viewer_country_or_global.as_bytes());
    identity.push(0);
    identity.extend_from_slice(partition.viewer_map_mode_id.as_bytes());
    identity.push(0);
    identity.push(u8::from(partition.province_fade_uses_viewer_map_mode));
    identity.push(u8::from(partition.province_fade_alternate_suppressed));
    identity.push(u8::from(partition.province_fade_suppressed));
    identity.extend_from_slice(partition.selection_palette_sha256.as_bytes());
    for bucket in buckets {
        let class = prepared
            .classes
            .iter()
            .find(|class| class.material == bucket.bucket.material)
            .ok_or("XMAP06 phase resources omitted a material class")?;
        identity.extend_from_slice(&bucket.bucket.bucket_index.to_le_bytes());
        identity.extend_from_slice(bucket.indices_sha256.as_bytes());
        identity.extend_from_slice(bucket.ordered_draws_sha256.as_bytes());
        identity.extend_from_slice(class.texture_content.payload_sha256.as_bytes());
    }
    Ok(Sha256::hex(&identity))
}

fn validate_assigned_stage_pair(early: &AssignedStage, late: &AssignedStage) -> Result<()> {
    early
        .validate()
        .map_err(|error| format!("XMAP06 early-stage assignment: {error}"))?;
    late.validate()
        .map_err(|error| format!("XMAP06 late-stage assignment: {error}"))?;
    if early.stage_name != XMAP06_EARLY_STAGE_NAME
        || early.operation_name != XMAP06_EARLY_OPERATION_NAME
        || late.stage_name != XMAP06_LATE_STAGE_NAME
        || late.operation_name != XMAP06_LATE_OPERATION_NAME
        || early.topology_sha256 != late.topology_sha256
        || early.assigned_ordinal >= late.assigned_ordinal
    {
        return Err("XMAP06 assigned-stage pair is invalid or misordered".into());
    }
    Ok(())
}

fn admitted_owner_epoch(frame: &WorldFrame) -> Result<(KeyingTables, String)> {
    if frame.owner_by_state_status() != ViewStatus::Present {
        return Err(format!(
            "XMAP06 owner_by_state is {:?}, not Present",
            frame.owner_by_state_status()
        )
        .into());
    }
    let mut owners = BTreeMap::new();
    for (&state_id, owner) in frame.owner_by_state() {
        match owner {
            Leaf::Measured(tag) if !tag.trim().is_empty() => {
                owners.insert(state_id, tag.clone());
            }
            Leaf::Measured(_) | Leaf::Absent | Leaf::Unobtained => {
                return Err(format!(
                    "XMAP06 owner_by_state row {state_id} is empty, absent, or unobtained"
                )
                .into());
            }
        }
    }
    if owners.is_empty() {
        return Err("XMAP06 owner_by_state census is empty".into());
    }
    let mut identity = b"XMAP06_OWNER_EPOCH_V1\0".to_vec();
    identity.extend_from_slice(&frame.hour_index().to_le_bytes());
    identity.extend_from_slice(&serde_json::to_vec(&owners)?);
    let owner_epoch_sha256 = Sha256::hex(&identity);
    let keying = KeyingTables::from_world_frame(frame);
    if keying.state_owners != owners {
        return Err("XMAP06 owner keying projection changed the admitted census".into());
    }
    Ok((keying, owner_epoch_sha256))
}

fn admit_runtime_bucket_carrier(
    frame: &WorldFrame,
    viewer_country: Option<&str>,
) -> Result<Option<AdmittedThinBorderRuntimeCarrier>> {
    let Some(map) = frame.domain("map") else {
        return Ok(None);
    };
    let map = map
        .as_object()
        .ok_or("XMAP06 map domain is present but is not an object")?;
    let Some(value) = map.get(XMAP06_RUNTIME_CARRIER_KEY) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let raw: RawThinBorderRuntimeCarrier = serde_json::from_value(value.clone())
        .map_err(|error| format!("XMAP06 runtime bucket carrier is malformed: {error}"))?;
    if raw.schema_version != 4
        || raw.authority != XMAP06_RUNTIME_CARRIER_AUTHORITY
        || raw.source_binary_sha256 != XMAP06_A729_BINARY_SHA256
        || raw.producer_function_rva != XMAP06_BUCKET_WRITER_RVA
        || raw.palette_producer_function_rva != XMAP06_PALETTE_WRITER_RVA
        || raw.hour_index != frame.hour_index()
        || !raw.complete
        || raw.contexts.is_empty()
    {
        return Err("XMAP06 runtime bucket carrier authority/header was rejected".into());
    }
    let roster = frame.d18_country_roster();
    if roster.status != ViewStatus::Present {
        return Err(format!(
            "XMAP06 requires the complete D18 viewer-country roster, found {:?}",
            roster.status
        )
        .into());
    }
    let expected_contexts = std::iter::once("global".to_owned())
        .chain(roster.entries.into_iter().map(|entry| entry.tag))
        .collect::<BTreeSet<_>>();
    let actual_contexts = raw.contexts.keys().cloned().collect::<BTreeSet<_>>();
    if actual_contexts != expected_contexts {
        return Err("XMAP06 runtime carrier does not cover the complete D18 viewer roster".into());
    }
    validate_sha256(&raw.contexts_sha256, "XMAP06 runtime carrier contexts")?;
    let selected_key = selected_viewer_context_key(viewer_country)?;
    let mut contexts_identity = b"XMAP06_RUNTIME_CONTEXTS_V2\0".to_vec();
    contexts_identity.extend_from_slice(&(raw.contexts.len() as u64).to_le_bytes());
    let mut selected = None;
    for (viewer_country_or_global, context) in raw.contexts {
        if !valid_viewer_context_key(&viewer_country_or_global)
            || (context.province_fade_uses_viewer_map_mode
                && context.province_fade_alternate_suppressed)
            || context.palette_entry_count == 0
            || context.palette_entry_count as usize != context.selection_palette_f32_bits.len()
            || context.selection_palette_f32_bits.first() != Some(&[0; 4])
            || context.records.is_empty()
            || context.records.len() > 200_000
        {
            return Err("XMAP06 runtime context authority/header was rejected".into());
        }
        validate_sha256(&context.payload_sha256, "XMAP06 runtime context payload")?;
        let mut previous_key = None;
        let mut admitted = Vec::with_capacity(context.records.len());
        for row in context.records {
            let key = (row.province_low, row.province_high, row.pair_record_ordinal);
            if row.province_low == 0
                || row.province_low >= row.province_high
                || previous_key.is_some_and(|previous| previous >= key)
            {
                return Err(
                    "XMAP06 runtime carrier record keys are invalid, duplicated, or unsorted"
                        .into(),
                );
            }
            let base_material = row
                .base_material_ordinal
                .map(material_from_ordinal)
                .transpose()?;
            if base_material.is_some() != row.bucket_draw_ordinal.is_some()
                || (base_material.is_none()
                    && (row.secondary_bank || row.selector_reason_code_raw != 0))
                || base_material.is_some()
                    && row.selector_reason_code_raw as usize
                        >= context.selection_palette_f32_bits.len()
            {
                return Err("XMAP06 runtime carrier record draw state is invalid".into());
            }
            admitted.push(AdmittedThinBorderRuntimeRecord {
                key,
                base_material,
                secondary_bank: row.secondary_bank,
                bucket_draw_ordinal: row.bucket_draw_ordinal,
                selector_reason_code_raw: row.selector_reason_code_raw,
            });
            previous_key = Some(key);
        }
        let computed = runtime_carrier_payload_sha256(
            &viewer_country_or_global,
            context.province_fade_uses_viewer_map_mode,
            context.province_fade_alternate_suppressed,
            &context.selection_palette_f32_bits,
            &admitted,
        );
        if computed != context.payload_sha256 {
            return Err("XMAP06 runtime context payload digest mismatch".into());
        }
        contexts_identity.extend_from_slice(viewer_country_or_global.as_bytes());
        contexts_identity.push(0);
        contexts_identity.extend_from_slice(context.payload_sha256.as_bytes());
        if viewer_country_or_global == selected_key {
            selected = Some(AdmittedThinBorderRuntimeCarrier {
                viewer_country_or_global,
                province_fade_uses_viewer_map_mode: context.province_fade_uses_viewer_map_mode,
                province_fade_alternate_suppressed: context.province_fade_alternate_suppressed,
                payload_sha256: context.payload_sha256,
                selection_palette_sha256: selection_palette_sha256(
                    &context.selection_palette_f32_bits,
                ),
                selection_palette_f32_bits: context.selection_palette_f32_bits,
                records: admitted,
            });
        }
    }
    if Sha256::hex(&contexts_identity) != raw.contexts_sha256 {
        return Err("XMAP06 runtime carrier contexts digest mismatch".into());
    }
    selected.map(Some).ok_or_else(|| {
        format!("XMAP06 runtime carrier has no selected viewer context {selected_key}").into()
    })
}

fn selected_viewer_context_key(viewer_country: Option<&str>) -> Result<String> {
    let key = viewer_country.unwrap_or("global");
    if !valid_viewer_context_key(key) {
        return Err(format!("XMAP06 selected viewer context {key:?} is invalid").into());
    }
    Ok(key.to_owned())
}

fn valid_viewer_context_key(key: &str) -> bool {
    key == "global"
        || !key.is_empty()
            && key.len() <= 32
            && key
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

pub fn xmap06_province_fade_suppressed(
    viewer_map_mode_id: &str,
    uses_viewer_map_mode: bool,
    alternate_suppressed: bool,
) -> Result<bool> {
    if uses_viewer_map_mode && alternate_suppressed {
        return Err(
            "XMAP06 province-fade carrier selected both mutually-exclusive engine branches".into(),
        );
    }
    if alternate_suppressed {
        return Ok(true);
    }
    if !uses_viewer_map_mode {
        return Ok(false);
    }
    xmap06_viewer_map_mode_suppresses_province_fade(viewer_map_mode_id)
}

fn xmap06_viewer_map_mode_suppresses_province_fade(map_mode_id: &str) -> Result<bool> {
    if map_mode_id == "viewer.map_mode.unselected" {
        debug_assert_eq!(XMAP06_BUILTIN_MAP_MODE_DEFAULT, "builtin:MAPMODE_DEFAULT");
        return Ok(false);
    }
    if let Some(key) = map_mode_id.strip_prefix("builtin:MAPMODE_") {
        if !valid_map_mode_key_suffix(key) {
            return Err("XMAP06 viewer built-in map-mode identity is malformed".into());
        }
        return Ok(matches!(
            map_mode_id,
            XMAP06_BUILTIN_MAP_MODE_RESOURCES
                | XMAP06_BUILTIN_MAP_MODE_INFRASTRUCTURE
                | XMAP06_BUILTIN_MAP_MODE_MANPOWER
        ));
    }
    if let Some(key) = map_mode_id.strip_prefix("scripted:MAPMODE_") {
        if !valid_map_mode_key_suffix(key) {
            return Err("XMAP06 viewer scripted map-mode identity is malformed".into());
        }
        return Ok(false);
    }
    if let Some(token) = map_mode_id.strip_prefix("viewer.map_mode.") {
        if token.starts_with("scripted:") {
            crate::ViewerMapModeSelection::parse(map_mode_id)?;
            return Ok(false);
        }
        let selection = crate::ViewerMapModeSelection::parse(map_mode_id)?;
        return Ok(matches!(
            selection,
            crate::ViewerMapModeSelection::Hardcoded(
                crate::HardcodedMapMode::Resources
                    | crate::HardcodedMapMode::Infrastructure
                    | crate::HardcodedMapMode::Manpower
            )
        ));
    }
    Err(format!(
        "XMAP06 viewer map-mode identity {map_mode_id:?} lacks an exact built-in/scripted namespace"
    )
    .into())
}

fn valid_map_mode_key_suffix(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn selection_palette_sha256(palette: &[[u32; 4]]) -> String {
    let mut identity = b"XMAP06_SELECTION_PALETTE_V1\0".to_vec();
    identity.extend_from_slice(&(palette.len() as u64).to_le_bytes());
    for color in palette {
        for component in color {
            identity.extend_from_slice(&component.to_le_bytes());
        }
    }
    Sha256::hex(&identity)
}

fn runtime_carrier_payload_sha256(
    viewer_country_or_global: &str,
    province_fade_uses_viewer_map_mode: bool,
    province_fade_alternate_suppressed: bool,
    palette: &[[u32; 4]],
    records: &[AdmittedThinBorderRuntimeRecord],
) -> String {
    let mut identity = b"XMAP06_RUNTIME_DRAW_PAYLOAD_V4\0".to_vec();
    identity.extend_from_slice(viewer_country_or_global.as_bytes());
    identity.push(0);
    identity.push(u8::from(province_fade_uses_viewer_map_mode));
    identity.push(u8::from(province_fade_alternate_suppressed));
    identity.extend_from_slice(selection_palette_sha256(palette).as_bytes());
    for row in records {
        identity.extend_from_slice(&row.key.0.to_le_bytes());
        identity.extend_from_slice(&row.key.1.to_le_bytes());
        identity.extend_from_slice(&row.key.2.to_le_bytes());
        identity.push(
            row.base_material
                .map(|material| material as u8)
                .unwrap_or(u8::MAX),
        );
        identity.push(u8::from(row.secondary_bank));
        identity.extend_from_slice(&row.bucket_draw_ordinal.unwrap_or(u32::MAX).to_le_bytes());
        identity.push(row.selector_reason_code_raw);
    }
    Sha256::hex(&identity)
}

fn material_from_ordinal(ordinal: u8) -> Result<BorderMaterial> {
    BorderMaterial::ALL
        .get(ordinal as usize)
        .copied()
        .ok_or_else(|| {
            format!("XMAP06 runtime carrier material ordinal {ordinal} is invalid").into()
        })
}

fn prepare_runtime_partition(
    mesh: &BorderWorldMesh,
    viewer_map_mode_id: &str,
    carrier: &AdmittedThinBorderRuntimeCarrier,
) -> Result<PreparedThinBorderRuntimePartition> {
    prepare_runtime_partition_from_parts(&mesh.chains, &mesh.indices, viewer_map_mode_id, carrier)
}

fn prepare_runtime_partition_from_parts(
    chains: &[BorderChain],
    mesh_indices: &[u32],
    viewer_map_mode_id: &str,
    carrier: &AdmittedThinBorderRuntimeCarrier,
) -> Result<PreparedThinBorderRuntimePartition> {
    let mut next_ordinal_by_pair = BTreeMap::<(u32, u32), u32>::new();
    let mut expected_records = Vec::with_capacity(chains.len());
    for chain in chains {
        let pair = (
            chain.province_a.min(chain.province_b),
            chain.province_a.max(chain.province_b),
        );
        let ordinal = next_ordinal_by_pair.entry(pair).or_default();
        expected_records.push(((pair.0, pair.1, *ordinal), chain));
        *ordinal = ordinal
            .checked_add(1)
            .ok_or("XMAP06 pair record ordinal overflow")?;
    }
    expected_records.sort_by_key(|(key, _)| *key);
    if expected_records.len() != carrier.records.len() {
        return Err(format!(
            "XMAP06 runtime carrier record census {} does not match synthesized census {}",
            carrier.records.len(),
            expected_records.len()
        )
        .into());
    }

    let mut bucket_records = (0..XMAP06_DRAW_BUCKETS.len())
        .map(|_| Vec::<(&BorderChain, &AdmittedThinBorderRuntimeRecord)>::new())
        .collect::<Vec<_>>();
    for ((expected_key, chain), row) in expected_records.iter().zip(&carrier.records) {
        if *expected_key != row.key {
            return Err(format!(
                "XMAP06 runtime carrier key {:?} does not match synthesized key {:?}",
                row.key, expected_key
            )
            .into());
        }
        let Some(material) = row.base_material else {
            continue;
        };
        if material != chain.material {
            return Err(format!(
                "XMAP06 runtime carrier material {} disagrees with synthesized {} at {:?}",
                material.as_str(),
                chain.material.as_str(),
                expected_key
            )
            .into());
        }
        let bucket_index = if row.secondary_bank {
            6 + material as usize
        } else {
            material as usize
        };
        bucket_records[bucket_index].push((chain, row));
    }

    let buckets = XMAP06_DRAW_BUCKETS
        .into_iter()
        .zip(bucket_records)
        .map(|(bucket, mut records)| {
            records.sort_by_key(|(_, row)| row.bucket_draw_ordinal);
            let mut indices = Vec::<u32>::new();
            let mut ordered_draws = Vec::with_capacity(records.len());
            for (expected_ordinal, (chain, row)) in records.into_iter().enumerate() {
                if row.bucket_draw_ordinal != Some(expected_ordinal as u32) {
                    return Err(format!(
                        "XMAP06 runtime bucket {} draw ordinals are not contiguous",
                        bucket.bucket_index
                    )
                    .into());
                }
                let start = chain.first_index as usize;
                let end = start
                    .checked_add(chain.index_count() as usize)
                    .ok_or("XMAP06 runtime partition chain range overflow")?;
                let run = mesh_indices
                    .get(start..end)
                    .ok_or("XMAP06 runtime partition chain range exceeds mesh")?;
                let first_index: u32 = indices
                    .len()
                    .try_into()
                    .map_err(|_| "XMAP06 runtime bucket first index exceeds u32")?;
                let index_count: u32 = run
                    .len()
                    .try_into()
                    .map_err(|_| "XMAP06 runtime bucket draw index count exceeds u32")?;
                indices.extend_from_slice(run);
                ordered_draws.push(PreparedThinBorderBucketDraw {
                    province_low: row.key.0,
                    province_high: row.key.1,
                    pair_record_ordinal: row.key.2,
                    bucket_draw_ordinal: expected_ordinal as u32,
                    first_index,
                    index_count,
                    selector_reason_code_raw: row.selector_reason_code_raw,
                });
            }
            let bytes: Arc<[u8]> = indices
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>()
                .into();
            let ordered_draws_sha256 = prepared_bucket_draws_sha256(bucket, &ordered_draws);
            Ok(PreparedThinBorderBucketGeometry {
                bucket,
                index_count: (bytes.len() / 4)
                    .try_into()
                    .map_err(|_| "XMAP06 runtime bucket index count exceeds u32")?,
                indices_sha256: Sha256::hex(&bytes),
                indices: bytes,
                ordered_draws,
                ordered_draws_sha256,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(PreparedThinBorderRuntimePartition {
        viewer_country_or_global: carrier.viewer_country_or_global.clone(),
        viewer_map_mode_id: viewer_map_mode_id.to_owned(),
        province_fade_uses_viewer_map_mode: carrier.province_fade_uses_viewer_map_mode,
        province_fade_alternate_suppressed: carrier.province_fade_alternate_suppressed,
        province_fade_suppressed: xmap06_province_fade_suppressed(
            viewer_map_mode_id,
            carrier.province_fade_uses_viewer_map_mode,
            carrier.province_fade_alternate_suppressed,
        )?,
        carrier_payload_sha256: carrier.payload_sha256.clone(),
        selection_palette_f32_bits: carrier.selection_palette_f32_bits.clone(),
        selection_palette_sha256: carrier.selection_palette_sha256.clone(),
        buckets,
    })
}

fn prepared_bucket_draws_sha256(
    bucket: PreparedThinBorderBucket,
    draws: &[PreparedThinBorderBucketDraw],
) -> String {
    let mut identity = b"XMAP06_PREPARED_BUCKET_DRAWS_V2\0".to_vec();
    identity.extend_from_slice(&bucket.bucket_index.to_le_bytes());
    identity.extend_from_slice(&(draws.len() as u64).to_le_bytes());
    for draw in draws {
        identity.extend_from_slice(&draw.province_low.to_le_bytes());
        identity.extend_from_slice(&draw.province_high.to_le_bytes());
        identity.extend_from_slice(&draw.pair_record_ordinal.to_le_bytes());
        identity.extend_from_slice(&draw.bucket_draw_ordinal.to_le_bytes());
        identity.extend_from_slice(&draw.first_index.to_le_bytes());
        identity.extend_from_slice(&draw.index_count.to_le_bytes());
        identity.push(draw.selector_reason_code_raw);
    }
    Sha256::hex(&identity)
}

fn prepare_border_frame(
    hour_index: i64,
    early_stage: AssignedStage,
    late_stage: AssignedStage,
    owner_epoch_sha256: String,
    ordered_used_content: Vec<UsedContentKey>,
    material_content: &BTreeMap<BorderMaterial, UsedContentKey>,
    fade_windows: PreparedThinBorderFadeWindows,
    mesh: &BorderWorldMesh,
    viewer_map_mode_id: &str,
    runtime_carrier: Option<&AdmittedThinBorderRuntimeCarrier>,
) -> Result<PreparedThinBorderFrame> {
    let vertices: Arc<[u8]> = mesh.to_shader_vertex_layout_bytes()?.into();
    let grouped = group_indices_by_material(&mesh.chains, &mesh.indices)?;
    let classes = BorderMaterial::ALL
        .into_iter()
        .enumerate()
        .map(|(ordinal, material)| {
            let indices: Arc<[u8]> = grouped
                .get(&material)
                .ok_or("XMAP06 material grouping omitted a class")?
                .iter()
                .flat_map(|index| index.to_le_bytes())
                .collect::<Vec<_>>()
                .into();
            Ok(PreparedThinBorderClass {
                material,
                material_ordinal: ordinal as u32,
                texture_content: material_content
                    .get(&material)
                    .ok_or("XMAP06 material content closure omitted a class")?
                    .clone(),
                index_count: (indices.len() / 4)
                    .try_into()
                    .map_err(|_| "XMAP06 material index count exceeds u32")?,
                indices_sha256: Sha256::hex(&indices),
                indices,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let runtime_partition = runtime_carrier
        .map(|carrier| prepare_runtime_partition(mesh, viewer_map_mode_id, carrier))
        .transpose()?;
    let mut prepared = PreparedThinBorderFrame {
        schema: XMAP06_THIN_BORDER_SCHEMA.into(),
        hour_index,
        early_stage,
        late_stage,
        map_width: mesh
            .map_width
            .try_into()
            .map_err(|_| "XMAP06 map width exceeds u32")?,
        map_height: mesh
            .map_height
            .try_into()
            .map_err(|_| "XMAP06 map height exceeds u32")?,
        owner_epoch_sha256,
        ordered_used_content,
        fade_windows,
        material_order: XMAP06_MATERIAL_ORDER,
        draw_buckets: XMAP06_DRAW_BUCKETS,
        vertex_stride_bytes: BORDER_VERTEX_STRIDE_BYTES as u32,
        cb2_byte_width: XMAP06_BORDER_CB2_BYTES,
        vertex_count: mesh
            .vertices
            .len()
            .try_into()
            .map_err(|_| "XMAP06 vertex count exceeds u32")?,
        vertices_sha256: Sha256::hex(&vertices),
        vertices,
        classes,
        runtime_partition,
        deterministic_sha256: String::new(),
    };
    prepared.deterministic_sha256 = prepared_thin_border_digest(&prepared)?;
    prepared.validate()?;
    Ok(prepared)
}

fn group_indices_by_material(
    chains: &[BorderChain],
    mesh_indices: &[u32],
) -> Result<BTreeMap<BorderMaterial, Vec<u32>>> {
    let mut grouped = BorderMaterial::ALL
        .into_iter()
        .map(|material| (material, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let mut next_index = 0usize;
    for chain in chains {
        let start = chain.first_index as usize;
        if start != next_index {
            return Err(
                "XMAP06 chain index ranges overlap, contain a gap, or are out of order".into(),
            );
        }
        let end = start
            .checked_add(chain.index_count() as usize)
            .ok_or("XMAP06 chain index range overflow")?;
        let run = mesh_indices
            .get(start..end)
            .ok_or("XMAP06 chain index range exceeds synthesized mesh")?;
        grouped
            .get_mut(&chain.material)
            .ok_or("XMAP06 chain has an unknown material")?
            .extend_from_slice(run);
        next_index = end;
    }
    let grouped_count = grouped.values().try_fold(0usize, |total, run| {
        total
            .checked_add(run.len())
            .ok_or("XMAP06 grouped index count overflow")
    })?;
    if next_index != mesh_indices.len() || grouped_count != mesh_indices.len() {
        return Err("XMAP06 chains do not account for every synthesized index".into());
    }
    Ok(grouped)
}

fn prepared_thin_border_digest(prepared: &PreparedThinBorderFrame) -> Result<String> {
    let mut identity = b"XMAP06_PREPARED_THIN_BORDER_V7\0".to_vec();
    identity.extend_from_slice(prepared.schema.as_bytes());
    identity.extend_from_slice(&prepared.hour_index.to_le_bytes());
    identity.extend_from_slice(&serde_json::to_vec(&prepared.early_stage)?);
    identity.extend_from_slice(&serde_json::to_vec(&prepared.late_stage)?);
    identity.extend_from_slice(&prepared.map_width.to_le_bytes());
    identity.extend_from_slice(&prepared.map_height.to_le_bytes());
    identity.extend_from_slice(prepared.owner_epoch_sha256.as_bytes());
    identity.extend_from_slice(&prepared.fade_windows.province.near_f32_bits.to_le_bytes());
    identity.extend_from_slice(&prepared.fade_windows.province.far_f32_bits.to_le_bytes());
    identity.extend_from_slice(&prepared.fade_windows.state.near_f32_bits.to_le_bytes());
    identity.extend_from_slice(&prepared.fade_windows.state.far_f32_bits.to_le_bytes());
    for material in prepared.material_order {
        identity.extend_from_slice(&(material as u32).to_le_bytes());
    }
    for bucket in prepared.draw_buckets {
        identity.extend_from_slice(&bucket.bucket_index.to_le_bytes());
        identity.push(match bucket.phase {
            PreparedThinBorderDrawPhase::Early => 0,
            PreparedThinBorderDrawPhase::Late => 1,
        });
        identity.push(match bucket.bank {
            PreparedThinBorderBucketBank::Base => 0,
            PreparedThinBorderBucketBank::Secondary => 1,
        });
        identity.extend_from_slice(&(bucket.material as u32).to_le_bytes());
    }
    identity.extend_from_slice(&prepared.vertex_stride_bytes.to_le_bytes());
    identity.extend_from_slice(&prepared.cb2_byte_width.to_le_bytes());
    identity.extend_from_slice(&prepared.vertex_count.to_le_bytes());
    identity.extend_from_slice(prepared.vertices_sha256.as_bytes());
    identity.extend_from_slice(&serde_json::to_vec(&prepared.ordered_used_content)?);
    for class in &prepared.classes {
        identity.extend_from_slice(&(class.material as u32).to_le_bytes());
        identity.extend_from_slice(&class.material_ordinal.to_le_bytes());
        identity.extend_from_slice(&class.index_count.to_le_bytes());
        identity.extend_from_slice(class.indices_sha256.as_bytes());
        identity.extend_from_slice(class.texture_content.payload_sha256.as_bytes());
    }
    if let Some(partition) = &prepared.runtime_partition {
        identity.push(1);
        identity.extend_from_slice(partition.viewer_country_or_global.as_bytes());
        identity.push(0);
        identity.extend_from_slice(partition.viewer_map_mode_id.as_bytes());
        identity.push(0);
        identity.push(u8::from(partition.province_fade_uses_viewer_map_mode));
        identity.push(u8::from(partition.province_fade_alternate_suppressed));
        identity.push(u8::from(partition.province_fade_suppressed));
        identity.extend_from_slice(partition.carrier_payload_sha256.as_bytes());
        identity.extend_from_slice(partition.selection_palette_sha256.as_bytes());
        for bucket in &partition.buckets {
            identity.extend_from_slice(&bucket.bucket.bucket_index.to_le_bytes());
            identity.extend_from_slice(&bucket.index_count.to_le_bytes());
            identity.extend_from_slice(bucket.indices_sha256.as_bytes());
            identity.extend_from_slice(bucket.ordered_draws_sha256.as_bytes());
        }
    } else {
        identity.push(0);
    }
    Ok(Sha256::hex(&identity))
}

fn runtime_bucket_carrier_gap(hour: i64, geometry_sha256: &str) -> FoundationGap {
    let witness_sha256 = Sha256::hex(
        format!("{XMAP06_RUNTIME_BUCKET_GAP_ID}\n{hour}\n{hour}\n{geometry_sha256}\n{reason}")
            .as_bytes(),
    );
    FoundationGap {
        gap_id: XMAP06_RUNTIME_BUCKET_GAP_ID.into(),
        first_unavailable_hour: hour,
        last_unavailable_hour: hour,
        witness_sha256,
        reason: reason.into(),
    }
}

fn validate_static_draw_contract(
    material_order: &[BorderMaterial; 6],
    draw_buckets: &[PreparedThinBorderBucket; 12],
) -> Result<()> {
    if *material_order != XMAP06_MATERIAL_ORDER {
        return Err("XMAP06 six-material ordinal/order differs from exact-a729".into());
    }
    if *draw_buckets != XMAP06_DRAW_BUCKETS {
        return Err("XMAP06 two-phase twelve-bucket order differs from exact-a729".into());
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || value.bytes().all(|byte| byte == b'0')
    {
        return Err(format!("{label} is not a canonical nonzero SHA-256 identity").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BorderRank, BorderWaterClass};

    fn chain(material: BorderMaterial, first_index: u32, rows: u32) -> BorderChain {
        BorderChain {
            province_a: 1,
            province_b: 2,
            state_a: Some(1),
            state_b: Some(2),
            rank: BorderRank::State,
            water_class: BorderWaterClass::LandLand,
            material,
            closed: false,
            manhattan_length: rows - 2,
            rows,
            first_vertex: 0,
            first_index,
        }
    }

    #[test]
    fn material_grouping_preserves_each_chain_run_without_declaring_draw_order() {
        let first = chain(BorderMaterial::State, 0, 3);
        let second = chain(BorderMaterial::Province, 12, 3);
        let indices = (0..24).collect::<Vec<_>>();
        let grouped = group_indices_by_material(&[first, second], &indices).unwrap();
        assert_eq!(
            grouped.get(&BorderMaterial::State).unwrap(),
            &(0..12).collect::<Vec<_>>()
        );
        assert_eq!(
            grouped.get(&BorderMaterial::Province).unwrap(),
            &(12..24).collect::<Vec<_>>()
        );
        assert!(grouped.get(&BorderMaterial::Country).unwrap().is_empty());
    }

    #[test]
    fn material_grouping_rejects_unaccounted_or_out_of_range_indices() {
        let only = chain(BorderMaterial::State, 0, 3);
        assert!(group_indices_by_material(&[only.clone()], &(0..13).collect::<Vec<_>>()).is_err());
        let out_of_range = chain(BorderMaterial::State, 2, 3);
        assert!(group_indices_by_material(&[out_of_range], &(0..12).collect::<Vec<_>>()).is_err());
    }

    #[test]
    fn effective_graphics_fade_windows_preserve_exact_f32_words() {
        let source = "\
            NGraphics = {\n\
                PROVINCE_BORDER_FADE_NEAR = 200.25,\n\
                PROVINCE_BORDER_FADE_FAR = 300.5,\n\
                STATE_BORDER_FADE_NEAR = 400.75,\n\
                STATE_BORDER_FADE_FAR = 500.125,\n\
            }\n";
        let parsed = border_fade_windows_from_graphics_defines(source).unwrap();
        let prepared = PreparedThinBorderFadeWindows::from_windows(parsed);
        prepared.validate().unwrap();
        assert_eq!(prepared.province.near_f32_bits, 200.25f32.to_bits());
        assert_eq!(prepared.province.far_f32_bits, 300.5f32.to_bits());
        assert_eq!(prepared.state.near_f32_bits, 400.75f32.to_bits());
        assert_eq!(prepared.state.far_f32_bits, 500.125f32.to_bits());

        let invalid = PreparedThinBorderFadeWindow {
            near_f32_bits: 500.0f32.to_bits(),
            far_f32_bits: 400.0f32.to_bits(),
        };
        assert!(invalid.validate("test").is_err());
    }

    #[test]
    fn prepared_fade_transparency_matches_exact_bucket_windows() {
        let windows = PreparedThinBorderFadeWindows {
            province: PreparedThinBorderFadeWindow {
                near_f32_bits: 200.0f32.to_bits(),
                far_f32_bits: 300.0f32.to_bits(),
            },
            state: PreparedThinBorderFadeWindow {
                near_f32_bits: 400.0f32.to_bits(),
                far_f32_bits: 500.0f32.to_bits(),
            },
        };
        assert_eq!(
            windows
                .transparency(BorderMaterial::Province, 200.0)
                .unwrap(),
            1.0
        );
        assert_eq!(
            windows
                .transparency(BorderMaterial::Province, 250.0)
                .unwrap(),
            0.5
        );
        assert_eq!(
            windows
                .transparency(BorderMaterial::Province, 300.0)
                .unwrap(),
            0.0
        );
        assert_eq!(
            windows.transparency(BorderMaterial::State, 450.0).unwrap(),
            0.5
        );
        for material in [
            BorderMaterial::Country,
            BorderMaterial::Impassable,
            BorderMaterial::SeaCoast,
            BorderMaterial::SeaRegion,
        ] {
            assert_eq!(windows.transparency(material, 50_000.0).unwrap(), 1.0);
        }
        assert!(windows
            .transparency(BorderMaterial::Province, f32::NAN)
            .is_err());
    }

    #[test]
    fn assigned_phase_pair_comes_only_from_the_authenticated_schedule() {
        let early =
            crate::AssignedStageIdentity::new(XMAP06_EARLY_STAGE_NAME, XMAP06_EARLY_OPERATION_NAME);
        let late =
            crate::AssignedStageIdentity::new(XMAP06_LATE_STAGE_NAME, XMAP06_LATE_OPERATION_NAME);
        let topology = crate::AssignedStageTopology::seal(
            vec![early.clone(), late.clone()],
            vec![crate::AssignedStageEdge::new(early, late)],
        )
        .unwrap();
        let schedule = crate::AssignedStageSchedule::from_topology(&topology).unwrap();
        let early = schedule
            .require(XMAP06_EARLY_STAGE_NAME, XMAP06_EARLY_OPERATION_NAME)
            .unwrap();
        let late = schedule
            .require(XMAP06_LATE_STAGE_NAME, XMAP06_LATE_OPERATION_NAME)
            .unwrap();
        validate_assigned_stage_pair(early, late).unwrap();

        let mut tampered = early.clone();
        tampered.assigned_ordinal = late.assigned_ordinal;
        assert!(validate_assigned_stage_pair(&tampered, late).is_err());
    }

    #[test]
    fn static_contract_has_exact_material_ordinals_and_two_submit_phases() {
        validate_static_draw_contract(&XMAP06_MATERIAL_ORDER, &XMAP06_DRAW_BUCKETS).unwrap();
        assert_eq!(XMAP06_MATERIAL_ORDER, BorderMaterial::ALL);
        assert_eq!(
            XMAP06_DRAW_BUCKETS
                .iter()
                .filter(|bucket| bucket.phase == PreparedThinBorderDrawPhase::Early)
                .count(),
            4
        );
        assert_eq!(
            XMAP06_DRAW_BUCKETS
                .iter()
                .filter(|bucket| bucket.phase == PreparedThinBorderDrawPhase::Late)
                .count(),
            8
        );
        assert_eq!(
            XMAP06_DRAW_BUCKETS
                .iter()
                .filter(|bucket| bucket.bank == PreparedThinBorderBucketBank::Secondary)
                .map(|bucket| bucket.material)
                .collect::<Vec<_>>(),
            BorderMaterial::ALL
        );

        let mut changed = XMAP06_DRAW_BUCKETS;
        changed.swap(4, 5);
        assert!(validate_static_draw_contract(&XMAP06_MATERIAL_ORDER, &changed).is_err());
    }

    #[test]
    fn captured_vertex_and_cb2_sizes_are_part_of_the_prepared_contract() {
        assert_eq!(BORDER_VERTEX_STRIDE_BYTES, 16);
        assert_eq!(XMAP06_BORDER_CB2_BYTES, 32);
    }

    #[test]
    fn viewer_context_keys_follow_the_complete_d18_tag_contract() {
        assert_eq!(selected_viewer_context_key(None).unwrap(), "global");
        assert_eq!(
            selected_viewer_context_key(Some("DYN_001")).unwrap(),
            "DYN_001"
        );
        assert!(!valid_viewer_context_key(""));
        assert!(!valid_viewer_context_key("ger"));
        assert!(!valid_viewer_context_key("TAG-WITH-DASH"));
        assert!(!valid_viewer_context_key(&"A".repeat(33)));
    }

    #[test]
    fn recorded_runtime_selector_partitions_each_chain_once_without_duplication() {
        let chains = vec![
            chain(BorderMaterial::State, 0, 3),
            chain(BorderMaterial::Province, 12, 3),
            chain(BorderMaterial::Country, 24, 3),
        ];
        let records = vec![
            AdmittedThinBorderRuntimeRecord {
                key: (1, 2, 0),
                base_material: Some(BorderMaterial::State),
                secondary_bank: false,
                bucket_draw_ordinal: Some(0),
                selector_reason_code_raw: 1,
            },
            AdmittedThinBorderRuntimeRecord {
                key: (1, 2, 1),
                base_material: Some(BorderMaterial::Province),
                secondary_bank: true,
                bucket_draw_ordinal: Some(0),
                selector_reason_code_raw: 7,
            },
            AdmittedThinBorderRuntimeRecord {
                key: (1, 2, 2),
                base_material: None,
                secondary_bank: false,
                bucket_draw_ordinal: None,
                selector_reason_code_raw: 0,
            },
        ];
        let mut palette = vec![[0; 4]; 8];
        palette[1] = [1, 2, 3, 4];
        palette[7] = [7, 8, 9, 10];
        let carrier = AdmittedThinBorderRuntimeCarrier {
            viewer_country_or_global: "global".into(),
            province_fade_uses_viewer_map_mode: true,
            province_fade_alternate_suppressed: false,
            payload_sha256: runtime_carrier_payload_sha256(
                "global", true, false, &palette, &records,
            ),
            selection_palette_sha256: selection_palette_sha256(&palette),
            selection_palette_f32_bits: palette,
            records,
        };
        let partition = prepare_runtime_partition_from_parts(
            &chains,
            &(0..36).collect::<Vec<_>>(),
            "viewer.map_mode.unselected",
            &carrier,
        )
        .unwrap();
        assert_eq!(partition.buckets.len(), 12);
        assert_eq!(partition.buckets[1].index_count, 12);
        assert_eq!(partition.buckets[6].index_count, 12);
        assert_eq!(partition.viewer_country_or_global, "global");
        assert!(!partition.province_fade_suppressed);
        assert_eq!(
            partition
                .buckets
                .iter()
                .map(|bucket| bucket.index_count)
                .sum::<u32>(),
            24,
            "hidden records are omitted and admitted records are never duplicated"
        );
    }

    #[test]
    fn runtime_partition_obeys_authenticated_post_sort_order_and_alternate_branch() {
        let chains = vec![
            chain(BorderMaterial::Province, 0, 3),
            chain(BorderMaterial::Province, 12, 3),
        ];
        let records = vec![
            AdmittedThinBorderRuntimeRecord {
                key: (1, 2, 0),
                base_material: Some(BorderMaterial::Province),
                secondary_bank: false,
                bucket_draw_ordinal: Some(1),
                selector_reason_code_raw: 1,
            },
            AdmittedThinBorderRuntimeRecord {
                key: (1, 2, 1),
                base_material: Some(BorderMaterial::Province),
                secondary_bank: false,
                bucket_draw_ordinal: Some(0),
                selector_reason_code_raw: 0,
            },
        ];
        let palette = vec![[0; 4]; 8];
        let original_digest =
            runtime_carrier_payload_sha256("GER", false, true, &palette, &records);
        let carrier = AdmittedThinBorderRuntimeCarrier {
            viewer_country_or_global: "GER".into(),
            province_fade_uses_viewer_map_mode: false,
            province_fade_alternate_suppressed: true,
            payload_sha256: original_digest.clone(),
            selection_palette_sha256: selection_palette_sha256(&palette),
            selection_palette_f32_bits: palette,
            records: records.clone(),
        };
        let partition = prepare_runtime_partition_from_parts(
            &chains,
            &(0..24).collect::<Vec<_>>(),
            XMAP06_BUILTIN_MAP_MODE_RESOURCES,
            &carrier,
        )
        .unwrap();
        let draws = &partition.buckets[0].ordered_draws;
        assert_eq!(
            draws
                .iter()
                .map(|draw| draw.pair_record_ordinal)
                .collect::<Vec<_>>(),
            vec![1, 0]
        );
        assert_eq!(carrier.viewer_country_or_global, "GER");
        assert!(partition.province_fade_suppressed);

        let mut changed = records;
        changed[0].selector_reason_code_raw = 2;
        assert_ne!(
            original_digest,
            runtime_carrier_payload_sha256(
                "GER",
                false,
                true,
                &carrier.selection_palette_f32_bits,
                &changed,
            )
        );
    }

    #[test]
    fn runtime_selector_rejects_material_or_record_identity_drift() {
        let chains = vec![chain(BorderMaterial::State, 0, 3)];
        let palette = vec![[0; 4], [1, 2, 3, 4]];
        let carrier = AdmittedThinBorderRuntimeCarrier {
            viewer_country_or_global: "global".into(),
            province_fade_uses_viewer_map_mode: true,
            province_fade_alternate_suppressed: false,
            payload_sha256: Sha256::hex(b"test carrier"),
            selection_palette_sha256: selection_palette_sha256(&palette),
            selection_palette_f32_bits: palette,
            records: vec![AdmittedThinBorderRuntimeRecord {
                key: (1, 2, 0),
                base_material: Some(BorderMaterial::Country),
                secondary_bank: false,
                bucket_draw_ordinal: Some(0),
                selector_reason_code_raw: 1,
            }],
        };
        assert!(prepare_runtime_partition_from_parts(
            &chains,
            &(0..12).collect::<Vec<_>>(),
            "viewer.map_mode.unselected",
            &carrier
        )
        .is_err());

        let mut wrong_key = carrier;
        wrong_key.records[0].key = (1, 3, 0);
        assert!(prepare_runtime_partition_from_parts(
            &chains,
            &(0..12).collect::<Vec<_>>(),
            "viewer.map_mode.unselected",
            &wrong_key
        )
        .is_err());
    }

    #[test]
    fn viewer_map_mode_suppression_is_exact_and_scripted_collision_safe() {
        for map_mode_id in [
            XMAP06_BUILTIN_MAP_MODE_RESOURCES,
            XMAP06_BUILTIN_MAP_MODE_INFRASTRUCTURE,
            XMAP06_BUILTIN_MAP_MODE_MANPOWER,
        ] {
            assert!(xmap06_province_fade_suppressed(map_mode_id, true, false).unwrap());
        }
        assert!(!xmap06_province_fade_suppressed("builtin:MAPMODE_COUNTRY", true, false,).unwrap());
        assert!(
            !xmap06_province_fade_suppressed("scripted:MAPMODE_RESOURCES", true, false,).unwrap()
        );
        assert!(
            xmap06_province_fade_suppressed("viewer.map_mode.unselected", false, true,).unwrap()
        );
        assert!(xmap06_province_fade_suppressed("not-a-map-mode", false, true).unwrap());
        assert!(
            xmap06_province_fade_suppressed("viewer.map_mode.unselected", true, true,).is_err()
        );
        for map_mode_id in [
            "viewer.map_mode.resources",
            "viewer.map_mode.infrastructure",
            "viewer.map_mode.manpower",
        ] {
            assert!(xmap06_province_fade_suppressed(map_mode_id, true, false).unwrap());
        }
        assert!(
            !xmap06_province_fade_suppressed("viewer.map_mode.political", true, false,).unwrap()
        );
        assert!(!xmap06_province_fade_suppressed(
            "viewer.map_mode.scripted:resources",
            true,
            false,
        )
        .unwrap());
        assert!(xmap06_province_fade_suppressed("MAPMODE_RESOURCES", true, false).is_err());
    }

    #[test]
    fn viewer_map_mode_switch_invalidates_xmap06_cache_identity() {
        let common = |map_mode_id| {
            xmap06_cache_key(
                7,
                &"a".repeat(64),
                &"b".repeat(64),
                map_mode_id,
                &"c".repeat(64),
                &"d".repeat(64),
            )
        };
        assert_ne!(
            common(XMAP06_BUILTIN_MAP_MODE_DEFAULT),
            common(XMAP06_BUILTIN_MAP_MODE_RESOURCES),
        );
        assert_ne!(
            common(XMAP06_BUILTIN_MAP_MODE_RESOURCES),
            common("scripted:MAPMODE_RESOURCES"),
        );
    }
}
