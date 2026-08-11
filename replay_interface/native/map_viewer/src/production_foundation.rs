//! XMAP01 viewer-owned production foundation.
//!
//! The normal product route enters here with one semantic [`WorldFrame`],
//! viewer-local presentation state, an admitted XMAP00 gap projection, and the
//! exact XMAP01+XMAP02 content-operation receipt. Recorder-selected
//! presentation is not an argument and cannot be recovered through this API.

use crate::{MapAuthorityAdmission, MapAuthorityGap, Result};
use portable_render_stream::{
    FamilyCoverageRow, FamilyDisposition, FoundationGap, FoundationRenderEnvelope, MapVisualFamily,
    FOUNDATION_RENDER_SCHEMA,
};
use replay_core::{ContentManifest, WorldFrame};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const FOUNDATION_CONTENT_EXECUTION_RECEIPT_SCHEMA: &str =
    "hoi4.foundation-content-execution.v1";

/// Resolve the engine-proven XMAP02 operation from admitted content and
/// execute it on a real D3D11 device. Map dimensions come from the effective
/// `map/provinces.bmp` winner; no vanilla dimensions or tuple are supplied by
/// the caller.
#[cfg(windows)]
pub fn execute_foundation_content_preflight(
    resolver: &crate::GameFileResolver,
    admission: &replay_core::ValidatedContentAdmission,
    content_authority_sha256: &str,
) -> Result<FoundationContentExecutionReceipt> {
    validate_sha256(content_authority_sha256, "shipping content authority")?;
    let (map_width, map_height) = foundation_map_dimensions(resolver, admission)?;
    // exact-a729's live DX11 branch is false; max_texture_dimension is unused
    // on that branch and remains zero rather than a guessed device cap.
    let request =
        resolver.foundation_content_request(admission, map_width, map_height, false, 0)?;
    let resolved = resolver.resolve_foundation_content(&request, admission)?;
    // Exercise the portable XMAP02 identity codec in the real caller before
    // any host resource is created.
    let bytes = serde_json::to_vec(&resolved)?;
    let decoded: portable_content_manifest::foundation::ResolvedFoundationContent =
        serde_json::from_slice(&bytes)?;
    decoded
        .validate()
        .map_err(|error| format!("foundation content container round-trip: {error}"))?;
    if decoded != resolved {
        return Err("foundation content container round-trip changed identity".into());
    }
    crate::execute_foundation_content_operation(resolver, &decoded, content_authority_sha256)
}

pub fn foundation_map_dimensions(
    resolver: &crate::GameFileResolver,
    admission: &replay_core::ValidatedContentAdmission,
) -> Result<(u32, u32)> {
    let provinces = admission
        .require_used_content("map/provinces.bmp")
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let bytes = resolver.read_admitted_used_content(provinces)?;
    bmp_dimensions(&bytes)
}

pub(crate) fn bmp_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    if bytes.get(..2) != Some(b"BM") || bytes.len() < 26 {
        return Err("effective map/provinces.bmp has no complete BMP header".into());
    }
    let width = i32::from_le_bytes(
        bytes[18..22]
            .try_into()
            .map_err(|_| "BMP width is truncated")?,
    );
    let height = i32::from_le_bytes(
        bytes[22..26]
            .try_into()
            .map_err(|_| "BMP height is truncated")?,
    );
    if width <= 0 || height == 0 || height == i32::MIN {
        return Err("effective map/provinces.bmp dimensions are invalid".into());
    }
    Ok((width as u32, height.unsigned_abs()))
}

/// Viewer-local state. No member can be sourced from the recorder's camera,
/// selected Present, map-mode selection, hover, GUI, or input state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewerOwnedMapState {
    pub viewport_width: u32,
    pub viewport_height: u32,
    /// XMAP03 viewer-owned camera payload. Recorder camera carriers cannot be
    /// converted into this type. `None` is a typed unavailable state.
    pub camera: Option<crate::ViewerMapCameraState>,
    pub observer_country_id: Option<String>,
    pub map_mode_id: String,
    /// Viewer-owned monotone epoch incremented only when `map_mode_id`
    /// changes. Camera, selection, and playback revisions cannot masquerade
    /// as a map-mode transition.
    pub map_mode_selection_epoch: u64,
    pub selected_object_ids: Vec<String>,
    pub input_revision: u64,
    /// Viewer/render-clock seconds consumed by animated map shaders. This is
    /// never sourced from the recorder and is carried as exact f32 bits so a
    /// prepared frame cannot silently change due to host float conversion.
    pub animation_time_f32_bits: u32,
    /// Viewer-local elapsed real time for exact presentation recurrences.
    /// This is never recorder timing.
    pub frame_delta_seconds_f32_bits: u32,
}

impl ViewerOwnedMapState {
    pub fn validate(&self) -> Result<()> {
        if self.viewport_width == 0 || self.viewport_height == 0 {
            return Err("viewer-owned map viewport has zero extent".into());
        }
        if self.map_mode_id.trim().is_empty() {
            return Err("viewer-owned map mode is blank".into());
        }
        let animation_time = f32::from_bits(self.animation_time_f32_bits);
        if !animation_time.is_finite() || animation_time < 0.0 {
            return Err("viewer-owned map animation time is invalid".into());
        }
        let frame_delta_seconds = self.frame_delta_seconds();
        if !frame_delta_seconds.is_finite() || frame_delta_seconds < 0.0 {
            return Err("viewer-owned frame delta is negative or non-finite".into());
        }
        if let Some(camera) = &self.camera {
            camera.validate()?;
            if camera.viewport_width != self.viewport_width
                || camera.viewport_height != self.viewport_height
                || camera.input_revision != self.input_revision
            {
                return Err(
                    "viewer map camera payload disagrees with viewer viewport/input revision"
                        .into(),
                );
            }
        }
        let mut selected = BTreeSet::new();
        if self
            .selected_object_ids
            .iter()
            .any(|value| value.trim().is_empty() || !selected.insert(value.as_str()))
        {
            return Err("viewer-owned map selection is blank or duplicated".into());
        }
        Ok(())
    }

    pub fn frame_delta_seconds(&self) -> f32 {
        f32::from_bits(self.frame_delta_seconds_f32_bits)
    }
}

/// Receipt for the real reusable content preflight. XMAP02 resolves every
/// identity through the effective VFS; XMAP01 compiles/loads the exact program
/// and creates the exact GPU resource/descriptor state before issuing this
/// receipt. It is not a draw and contains no fabricated map pixels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationContentExecutionReceipt {
    pub schema: String,
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    /// Canonical XMAP02 authority derived from the recording manifest,
    /// admitted effective VFS, and actual mounted resolver.
    pub content_authority_sha256: String,
    pub resolved_operation_sha256: String,
    pub ordered_program_stage_sha256: Vec<String>,
    pub resource_identity_sha256: String,
    pub descriptor_identity_sha256: String,
    pub compiled_or_loaded_programs: u32,
    pub created_resources: u32,
    pub created_descriptors: u32,
    pub deterministic_sha256: String,
}

impl FoundationContentExecutionReceipt {
    pub fn seal(mut self) -> Result<Self> {
        self.deterministic_sha256.clear();
        self.validate_body()?;
        self.deterministic_sha256 = Sha256::hex(&serde_json::to_vec(&self)?);
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body()?;
        validate_sha256(&self.deterministic_sha256, "foundation receipt")?;
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("foundation content execution receipt digest mismatch".into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        if self.schema != FOUNDATION_CONTENT_EXECUTION_RECEIPT_SCHEMA {
            return Err("foundation content execution receipt schema mismatch".into());
        }
        validate_sha256(&self.content_manifest_sha256, "content manifest")?;
        validate_sha256(&self.product_modset_sha256, "product modset")?;
        validate_sha256(&self.effective_vfs_manifest_sha256, "effective VFS")?;
        validate_sha256(&self.content_authority_sha256, "shipping content authority")?;
        validate_sha256(
            &self.resolved_operation_sha256,
            "resolved content operation",
        )?;
        validate_sha256(&self.resource_identity_sha256, "foundation resource")?;
        validate_sha256(&self.descriptor_identity_sha256, "foundation descriptor")?;
        if self.ordered_program_stage_sha256.is_empty()
            || self.compiled_or_loaded_programs == 0
            || self.created_resources == 0
            || self.created_descriptors == 0
        {
            return Err(
                "foundation content operation did not compile/load a program and create its resource/descriptor"
                    .into(),
            );
        }
        for identity in &self.ordered_program_stage_sha256 {
            validate_sha256(identity, "foundation program stage")?;
        }
        Ok(())
    }
}

pub struct FoundationFeatureContext<'a> {
    pub world_frame: &'a WorldFrame,
    pub viewer: &'a ViewerOwnedMapState,
    pub map_authority_gaps: &'a [MapAuthorityGap],
    assigned_stage_schedule: Option<&'a portable_render_stream::AssignedStageSchedule>,
    pub product_modset_sha256: &'a str,
    pub effective_vfs_manifest_sha256: &'a str,
    pub foundation_content_operation_sha256: &'a str,
    pub viewer_state_sha256: &'a str,
    /// Earlier XMAP04 output in canonical family order. Later map-space
    /// consumers reuse its exact admitted map-copy set rather than deriving a
    /// second camera-footprint approximation.
    pub prepared_terrain: Option<&'a crate::PreparedTerrainFrame>,
    /// Earlier promoted family output in the canonical family order. XMAP13
    /// consumes XMAP05's exact visibility classification through this typed
    /// dependency; it never constructs a second FoW view.
    pub prepared_political: Option<&'a crate::PreparedPoliticalFrame>,
    /// Earlier same-hour XMAP13 output in canonical family order. XMAP14 uses
    /// this typed identity provider for complete participant joins.
    pub prepared_units: Option<&'a crate::PreparedXmap13Frame>,
    /// Exact current-hour XMAP13 identity authority consumed by XMAP15. It is
    /// derived from `prepared_units` in canonical family order and is never a
    /// recorder-selected presentation carrier.
    pub xmap15_units:
        Option<&'a dyn crate::production_features::xmap15::Xmap15UnitIdentityProvider>,
    pub xmap15_selection: &'a crate::production_features::xmap15::Xmap15ViewerSelection,
}

impl FoundationFeatureContext<'_> {
    /// Require the authenticated assignment for one exact packet-owned stage.
    ///
    /// The private adapter-core constructor carries no schedule while the
    /// product graph is being assembled. The public product constructor
    /// installs the sealed schedule before registering any stage-consuming
    /// adapter; this lookup never substitutes a packet-local ordinal.
    pub fn require_assigned_stage(
        &self,
        stage_name: &str,
        operation_name: &str,
    ) -> Result<&portable_render_stream::AssignedStage> {
        self.assigned_stage_schedule
            .ok_or_else(|| {
                format!(
                    "foundation has no assigned-stage schedule for required {stage_name}/{operation_name}"
                )
            })?
            .require(stage_name, operation_name)
            .map_err(Into::into)
    }
}

pub struct FoundationFeatureOutput {
    pub disposition: FamilyDisposition,
    /// XMAP04's capture-independent terrain geometry for the shipping
    /// foundation consumer. Only the Terrain family may populate this field.
    pub prepared_terrain: Option<crate::PreparedTerrainFrame>,
    /// XMAP05's replay-epoch political, GradientBorder, IntelMap, and
    /// presentation-independent visibility payload. Only
    /// PoliticalFillGradients may populate it; FogOfWar binds its companion
    /// digest from the same shared producer.
    pub prepared_political: Option<crate::PreparedPoliticalFrame>,
    /// XMAP06's replay-epoch thin-border geometry and admitted material
    /// identities. Only ThinBorders may populate this field. Exact draw order
    /// and fade remain separate fail-closed draw-contract concerns. The
    /// geometry may therefore accompany a typed Unavailable disposition.
    pub prepared_thin_borders: Option<crate::PreparedThinBorderFrame>,
    /// XMAP09's exact semantic lighting resources and viewer-camera shadow
    /// handoff. Only the LightingShadows family may populate this field.
    pub prepared_lighting: Option<crate::PreparedXmap09LightingFrame>,
    /// XMAP19's presentation-independent weather membership, authored content
    /// joins, exact camera visibility/playback law, and temporal epoch input.
    /// Only WeatherAtmosphere may populate this field.
    pub prepared_weather: Option<crate::PreparedXmap19WeatherFrame>,
    /// XMAP08's executable river geometry, exact adjacency records, and
    /// compiled river/strait programs.
    pub prepared_rivers_straits: Option<crate::PreparedRiversStraitsFrame>,
    /// XMAP07's executable water/ice program, runtime SnowMud texture,
    /// refraction target contract, quad, and temporal payload.
    pub prepared_water: Option<crate::PreparedWaterFrame>,
    /// XMAP20's unconditional lifecycle/content/packet preparation. Only the
    /// ParticlesEffects family may populate it.
    pub prepared_effects: Option<crate::PreparedXmap20Frame>,
    /// XMAP13's presentation-independent unit/entity preparation. Only
    /// UnitsCounters may populate it.
    pub prepared_units: Option<crate::PreparedXmap13Frame>,
    /// XMAP14's viewer-projected, effective-content-authenticated combat
    /// indicator compositor payload. Only CombatMarkers may populate it.
    pub prepared_combat: Option<crate::PreparedXmap14Frame>,
    /// XMAP15's exact current-hour orders/fronts/movement state. Only
    /// OrdersFrontsArrows may populate it.
    pub prepared_orders: Option<crate::production_features::xmap15::PreparedXmap15Frame>,
    /// XMAP16's presentation-independent rail/supply payload. Only RailSupply
    /// may populate this field.
    pub prepared_rail_supply: Option<crate::PreparedXmap16Frame>,
    /// XMAP18's viewer-owned selection and exact composition authority. Only
    /// MapModes may populate this field.
    pub prepared_map_modes: Option<crate::PreparedMapModeFrame>,
    /// Every unresolved XMAP00 input the adapter consumed or converted to its
    /// own exact unavailable state. The dispatch rejects unclaimed gaps.
    pub consumed_gap_ids: Vec<String>,
}

/// One promoted packet adapter. Later XMAP packets implement this trait in
/// their predeclared feature module and register exactly their owned family.
pub trait ProductionMapFeatureAdapter {
    fn family(&self) -> MapVisualFamily;
    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput>;
}

#[derive(Default)]
pub struct ProductionMapFeatureAdapters<'a> {
    slots: BTreeMap<MapVisualFamily, Box<dyn ProductionMapFeatureAdapter + 'a>>,
    xmap11: Option<crate::production_features::xmap11::Xmap11TreesAdapter>,
    xmap12: Option<crate::production_features::xmap12::Xmap12CitiesBuildingsMapObjectsAdapter>,
    xmap17: Option<crate::production_features::xmap17::Xmap17LabelAdapter>,
    assigned_stage_schedule: Option<portable_render_stream::AssignedStageSchedule>,
}

impl<'a> ProductionMapFeatureAdapters<'a> {
    pub fn register(&mut self, adapter: Box<dyn ProductionMapFeatureAdapter + 'a>) -> Result<()> {
        let family = adapter.family();
        if (family == MapVisualFamily::Trees && self.xmap11.is_some())
            || (family == MapVisualFamily::CitiesBuildingsMapObjects && self.xmap12.is_some())
        {
            return Err(format!("duplicate production adapter for {family:?}").into());
        }
        if self.slots.insert(family, adapter).is_some() {
            return Err(format!("duplicate production adapter for {family:?}").into());
        }
        Ok(())
    }

    fn get(&self, family: MapVisualFamily) -> Option<&dyn ProductionMapFeatureAdapter> {
        self.slots.get(&family).map(|adapter| adapter.as_ref())
    }

    fn register_xmap12(
        &mut self,
        adapter: crate::production_features::xmap12::Xmap12CitiesBuildingsMapObjectsAdapter,
    ) -> Result<()> {
        if self
            .slots
            .contains_key(&MapVisualFamily::CitiesBuildingsMapObjects)
            || self.xmap12.is_some()
        {
            return Err("duplicate production adapter for CitiesBuildingsMapObjects".into());
        }
        self.xmap12 = Some(adapter);
        Ok(())
    }

    fn register_xmap11(
        &mut self,
        adapter: crate::production_features::xmap11::Xmap11TreesAdapter,
    ) -> Result<()> {
        if self.slots.contains_key(&MapVisualFamily::Trees) || self.xmap11.is_some() {
            return Err("duplicate production adapter for Trees".into());
        }
        self.xmap11 = Some(adapter);
        Ok(())
    }

    fn register_xmap17(
        &mut self,
        adapter: crate::production_features::xmap17::Xmap17LabelAdapter,
    ) -> Result<()> {
        if self.slots.contains_key(&MapVisualFamily::LabelsMapText) || self.xmap17.is_some() {
            return Err("duplicate production adapter for LabelsMapText".into());
        }
        self.xmap17 = Some(adapter);
        Ok(())
    }

    fn has(&self, family: MapVisualFamily) -> bool {
        if family == MapVisualFamily::Trees {
            self.xmap11.is_some()
        } else if family == MapVisualFamily::CitiesBuildingsMapObjects {
            self.xmap12.is_some()
        } else if family == MapVisualFamily::LabelsMapText {
            self.xmap17.is_some()
        } else {
            self.slots.contains_key(&family)
        }
    }

    /// Prepare with the exact optional XMAP12 per-frame carriers. Absence is
    /// represented by XMAP12's typed-unavailable family row; it is never
    /// replaced with recorder presentation, a prior-hour carrier, or a
    /// synthesized default.
    pub fn prepare_with_xmap12_inputs(
        &self,
        world_frame: &WorldFrame,
        viewer: &ViewerOwnedMapState,
        manifest: &ContentManifest,
        map_admission: &MapAuthorityAdmission,
        content_receipt: &FoundationContentExecutionReceipt,
        building_visual: Option<&crate::Xmap12BuildingVisualFrame>,
        fixed_runtime: Option<&crate::Xmap12FixedRuntimeState>,
    ) -> Result<PreparedFoundationFrame> {
        let selection = crate::production_features::xmap15::Xmap15ViewerSelection::default();
        prepare_foundation_frame_inner(
            world_frame,
            viewer,
            manifest,
            map_admission,
            content_receipt,
            self,
            building_visual,
            fixed_runtime,
            &selection,
        )
    }

    /// Prepare the current product graph with exact XMAP12 carriers and an
    /// explicit viewer-owned XMAP15 plan selection. The selection is never
    /// recovered from recorder presentation.
    pub fn prepare_with_xmap12_and_xmap15_inputs(
        &self,
        world_frame: &WorldFrame,
        viewer: &ViewerOwnedMapState,
        manifest: &ContentManifest,
        map_admission: &MapAuthorityAdmission,
        content_receipt: &FoundationContentExecutionReceipt,
        building_visual: Option<&crate::Xmap12BuildingVisualFrame>,
        fixed_runtime: Option<&crate::Xmap12FixedRuntimeState>,
        selection: &crate::production_features::xmap15::Xmap15ViewerSelection,
    ) -> Result<PreparedFoundationFrame> {
        prepare_foundation_frame_inner(
            world_frame,
            viewer,
            manifest,
            map_admission,
            content_receipt,
            self,
            building_visual,
            fixed_runtime,
            selection,
        )
    }

    fn install_assigned_stage_schedule(
        &mut self,
        topology: &portable_render_stream::AssignedStageTopology,
        schedule: portable_render_stream::AssignedStageSchedule,
    ) -> Result<()> {
        schedule
            .validate_against(topology)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        if self.assigned_stage_schedule.is_some() {
            return Err("production adapters already have an assigned-stage schedule".into());
        }
        self.assigned_stage_schedule = Some(schedule);
        Ok(())
    }
}

/// Assemble the schedule-independent adapter core. This is private so callers
/// cannot mistake an incomplete intermediate graph for the promoted product
/// graph.
fn foundation_feature_adapter_core(
    resolver: &crate::GameFileResolver,
    admission: &replay_core::ValidatedContentAdmission,
    terrain_options: crate::TerrainLodCullOptions,
) -> Result<ProductionMapFeatureAdapters<'static>> {
    let mut adapters = ProductionMapFeatureAdapters::default();
    adapters.register(Box::new(
        crate::production_features::xmap03::Xmap03CameraAdapter,
    ))?;
    adapters.register(Box::new(
        crate::production_features::xmap04::Xmap04TerrainAdapter::new(
            resolver,
            admission,
            terrain_options,
        )?,
    ))?;
    let (political, fow) = crate::production_features::xmap05::adapters(resolver, admission)?;
    adapters.register(Box::new(political))?;
    adapters.register(Box::new(fow))?;
    adapters.register(Box::new(
        crate::production_features::xmap09::Xmap09LightingAdapter,
    ))?;
    adapters.register_xmap11(crate::production_features::xmap11::Xmap11TreesAdapter::new(
        resolver,
        admission,
        terrain_options,
    )?)?;
    adapters.register_xmap12(
        crate::production_features::xmap12::Xmap12CitiesBuildingsMapObjectsAdapter::new(
            resolver,
            admission,
            terrain_options,
        )?,
    )?;
    adapters.register(Box::new(
        crate::production_features::xmap19::Xmap19WeatherAdapter::new(resolver, admission),
    ))?;
    adapters.register(Box::new(
        crate::production_features::xmap07::Xmap07WaterAdapter::new(
            resolver,
            admission,
            terrain_options,
        )?,
    ))?;
    adapters.register(Box::new(
        crate::production_features::xmap13::Xmap13Adapter::new(resolver, admission)?,
    ))?;
    adapters.register(Box::new(
        crate::production_features::xmap15::Xmap15OrdersAdapter::new(resolver, admission),
    ))?;
    adapters.register(Box::new(
        crate::production_features::xmap16::Xmap16RailSupplyAdapter::new(resolver, admission)?,
    ))?;
    adapters.register(Box::new(
        crate::production_features::xmap18::Xmap18MapModeAdapter::new(resolver, admission)?,
    ))?;
    Ok(adapters)
}

/// Seal the complete assigned-stage topology consumed by the promoted product
/// foundation. Packet-local topology helpers remain packet evidence; this
/// neutral aggregate owns only the product census and the proven stage edges.
/// Exact-a729's core-map caller places XMAP08 rivers/straits after XMAP06's
/// early-border draws, XMAP16 rails before XMAP11 trees before XMAP07 water, XMAP06 late borders
/// after water, and XMAP16 supply arrows after that complete core-map block.
/// XMAP20 particles follow the supply-arrow manager. XMAP15's map-scene orders
/// precede XMAP13's map-icon-manager counters. Exact-a729's map-icon manager
/// then paints XMAP14 combat buckets 9/10/11 after counter bucket 1 and before
/// XMAP16 hub bucket 26.
/// XMAP16 hubs and XMAP21 furniture
/// do not have one unconditional edge: exact map-icon buckets put VP/resource
/// furniture before hubs and raid furniture after hubs. XMAP18's authenticated
/// furniture-state handoff orders XMAP21 prepare before XMAP21 execute without
/// inventing a cross-family draw edge. Lighting and weather preparation
/// otherwise retain only their independently proven dependencies.
pub fn product_foundation_assigned_stage_topology(
) -> Result<portable_render_stream::AssignedStageTopology> {
    use crate::production_features::{
        xmap06, xmap07, xmap08, xmap09, xmap13, xmap14, xmap15, xmap16, xmap17, xmap19, xmap20,
    };
    use portable_render_stream::{AssignedStageEdge, AssignedStageIdentity};

    let border_early = AssignedStageIdentity::new(
        xmap06::XMAP06_EARLY_STAGE_NAME,
        xmap06::XMAP06_EARLY_OPERATION_NAME,
    );
    let border_late = AssignedStageIdentity::new(
        xmap06::XMAP06_LATE_STAGE_NAME,
        xmap06::XMAP06_LATE_OPERATION_NAME,
    );
    let lighting_prepare = AssignedStageIdentity::new(
        xmap09::XMAP09_PREPARE_STAGE_NAME,
        xmap09::XMAP09_PREPARE_OPERATION_NAME,
    );
    let shadow_execute = AssignedStageIdentity::new(
        xmap09::XMAP09_EXECUTE_STAGE_NAME,
        xmap09::XMAP09_EXECUTE_OPERATION_NAME,
    );
    let weather_prepare = AssignedStageIdentity::new(
        xmap19::XMAP19_ASSIGNED_STAGE_NAME,
        xmap19::XMAP19_ASSIGNED_OPERATION_NAME,
    );
    let water_surface = AssignedStageIdentity::new(
        xmap07::XMAP07_ASSIGNED_STAGE_NAME,
        xmap07::XMAP07_ASSIGNED_OPERATION_NAME,
    );
    let trees = AssignedStageIdentity::new(
        crate::XMAP11_TREE_STAGE_NAME,
        crate::XMAP11_TREE_OPERATION_NAME,
    );
    let unit_entities = AssignedStageIdentity::new(
        xmap13::XMAP13_UNIT_ENTITY_STAGE_NAME,
        xmap13::XMAP13_UNIT_ENTITY_OPERATION_NAME,
    );
    let counter_quads = AssignedStageIdentity::new(
        xmap13::XMAP13_COUNTER_QUAD_STAGE_NAME,
        xmap13::XMAP13_COUNTER_QUAD_OPERATION_NAME,
    );
    let combat_indicators = AssignedStageIdentity::new(
        xmap14::XMAP14_COMBAT_STAGE_NAME,
        xmap14::XMAP14_COMBAT_OPERATION_NAME,
    );
    let orders = AssignedStageIdentity::new(
        xmap15::XMAP15_ASSIGNED_STAGE_NAME,
        xmap15::XMAP15_ASSIGNED_OPERATION_NAME,
    );
    let labels_prepare = AssignedStageIdentity::new(
        xmap17::XMAP17_PREPARE_STAGE,
        xmap17::XMAP17_PREPARE_OPERATION,
    );
    let labels_execute = AssignedStageIdentity::new(
        xmap17::XMAP17_EXECUTE_STAGE,
        xmap17::XMAP17_EXECUTE_OPERATION,
    );
    let particles = AssignedStageIdentity::new(
        xmap20::XMAP20_ASSIGNED_STAGE_NAME,
        xmap20::XMAP20_ASSIGNED_OPERATION_NAME,
    );
    let [rivers, straits] = xmap08::xmap08_assigned_stage_identities();
    let [rivers_before_straits] = xmap08::xmap08_assigned_stage_edges();
    let rail_execute = AssignedStageIdentity::new(
        crate::XMAP16_RAIL_STAGE_NAME,
        crate::XMAP16_RAIL_OPERATION_ID,
    );
    let supply_arrow_execute = AssignedStageIdentity::new(
        crate::XMAP16_SUPPLY_ARROW_OPERATION_ID,
        crate::XMAP16_SUPPLY_ARROW_OPERATION_NAME,
    );
    let hub_execute = AssignedStageIdentity::new(
        xmap16::XMAP16_HUB_STAGE_NAME,
        xmap16::XMAP16_HUB_OPERATION_NAME,
    );
    let furniture_state = AssignedStageIdentity::new("map_mode", "map_mode.furniture_state");
    let furniture_prepare = AssignedStageIdentity::new(
        crate::XMAP21_PREPARE_STAGE_NAME,
        crate::XMAP21_PREPARE_OPERATION_NAME,
    );
    let furniture_execute = AssignedStageIdentity::new(
        crate::XMAP21_EXECUTE_STAGE_NAME,
        crate::XMAP21_EXECUTE_OPERATION_NAME,
    );

    portable_render_stream::AssignedStageTopology::seal(
        vec![
            border_early.clone(),
            rivers.clone(),
            straits.clone(),
            rail_execute.clone(),
            trees.clone(),
            water_surface.clone(),
            border_late.clone(),
            supply_arrow_execute.clone(),
            lighting_prepare.clone(),
            shadow_execute.clone(),
            weather_prepare,
            unit_entities,
            counter_quads.clone(),
            combat_indicators.clone(),
            orders.clone(),
            labels_prepare.clone(),
            labels_execute.clone(),
            particles.clone(),
            hub_execute.clone(),
            furniture_state.clone(),
            furniture_prepare.clone(),
            furniture_execute.clone(),
        ],
        vec![
            AssignedStageEdge::new(border_early, rivers),
            rivers_before_straits,
            AssignedStageEdge::new(straits, rail_execute.clone()),
            AssignedStageEdge::new(rail_execute, trees.clone()),
            AssignedStageEdge::new(trees, water_surface.clone()),
            AssignedStageEdge::new(water_surface.clone(), border_late.clone()),
            AssignedStageEdge::new(border_late, supply_arrow_execute.clone()),
            AssignedStageEdge::new(lighting_prepare, shadow_execute),
            AssignedStageEdge::new(supply_arrow_execute.clone(), particles.clone()),
            AssignedStageEdge::new(supply_arrow_execute, labels_prepare.clone()),
            AssignedStageEdge::new(labels_prepare, labels_execute.clone()),
            AssignedStageEdge::new(labels_execute, particles.clone()),
            AssignedStageEdge::new(particles, hub_execute.clone()),
            AssignedStageEdge::new(orders, counter_quads.clone()),
            AssignedStageEdge::new(counter_quads, combat_indicators.clone()),
            AssignedStageEdge::new(combat_indicators, hub_execute),
            AssignedStageEdge::new(furniture_state, furniture_prepare.clone()),
            AssignedStageEdge::new(furniture_prepare, furniture_execute.clone()),
        ],
    )
    .map_err(Into::into)
}

/// Construct the product adapter graph with the authenticated schedule required
/// by any packet that emits assigned-stage operations. The neutral provider
/// validates the exact census and dependency graph before an adapter can
/// observe an ordinal.
pub fn product_feature_adapters_with_assigned_stage_schedule(
    resolver: &crate::GameFileResolver,
    admission: &replay_core::ValidatedContentAdmission,
    terrain_options: crate::TerrainLodCullOptions,
    topology: &portable_render_stream::AssignedStageTopology,
    schedule: portable_render_stream::AssignedStageSchedule,
    localisation_language: &str,
) -> Result<ProductionMapFeatureAdapters<'static>> {
    let mut adapters = foundation_feature_adapter_core(resolver, admission, terrain_options)?;
    adapters.install_assigned_stage_schedule(topology, schedule)?;
    adapters.register(Box::new(
        crate::production_features::xmap08::Xmap08RiversStraitsAdapter::new(
            resolver,
            admission,
            terrain_options.high_gfx_shaders,
            terrain_options.texture_quality,
        )?,
    ))?;
    adapters.register(Box::new(
        crate::production_features::xmap06::Xmap06ThinBorderAdapter::new(resolver, admission)?,
    ))?;
    adapters.register(Box::new(
        crate::production_features::xmap14::Xmap14Adapter::new(
            resolver,
            admission,
            localisation_language,
        )?,
    ))?;
    adapters.register_xmap17(crate::production_features::xmap17::Xmap17LabelAdapter::new(
        resolver,
        admission,
        localisation_language,
    )?)?;
    Ok(adapters)
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedFoundationFrame {
    pub envelope: FoundationRenderEnvelope,
    pub terrain: crate::PreparedTerrainFrame,
    pub political: crate::PreparedPoliticalFrame,
    /// Populated by the public product adapter constructor, which installs the
    /// authenticated schedule before registering XMAP06. `None` remains only
    /// for lower-level adapter-graph tests and cannot be produced by the
    /// promoted product constructor.
    pub thin_borders: Option<crate::PreparedThinBorderFrame>,
    pub lighting: Option<crate::PreparedXmap09LightingFrame>,
    /// XMAP11's effective-content and viewer-derived tree census. Present and
    /// authenticated-empty family rows both retain the complete prepared frame.
    pub trees: Option<crate::PreparedTreeFrame>,
    /// XMAP12's exact current city/building/ambient draw census and unchanged
    /// L03 caster rows. `None` is valid only alongside XMAP12's explicit
    /// typed-unavailable family disposition.
    pub xmap12: Option<crate::PreparedXmap12RenderFrame>,
    /// `None` is the exact typed-unavailable state when the independent
    /// weather visual-epoch carrier has not been recorded.
    pub weather: Option<crate::PreparedXmap19WeatherFrame>,
    /// Prepared XMAP08 provider payload. It is populated only by the
    /// schedule-bearing product graph.
    pub rivers_straits: Option<crate::PreparedRiversStraitsFrame>,
    pub water: crate::PreparedWaterFrame,
    pub effects: Option<crate::PreparedXmap20Frame>,
    pub units: Option<crate::PreparedXmap13Frame>,
    pub combat: Option<crate::PreparedXmap14Frame>,
    pub orders: Option<crate::production_features::xmap15::PreparedXmap15Frame>,
    pub rail_supply: Option<crate::PreparedXmap16Frame>,
    /// XMAP18's viewer-selected map-mode composition. The product adapter graph
    /// always registers XMAP18, so this is mandatory on product frames.
    pub map_modes: crate::PreparedMapModeFrame,
    /// XMAP17's semantic, effective-content-derived country/state map-name
    /// program and glyph geometry. Product construction requires this payload;
    /// lower-level adapter tests may omit the dedicated adapter.
    pub labels: Option<crate::PreparedMapLabelFrame>,
}

pub enum Xmap16FoundationState<'a> {
    Present(&'a crate::PreparedXmap16Frame),
    Unavailable(&'a portable_render_stream::FoundationGap),
}

pub enum Xmap15FoundationState<'a> {
    Present(&'a crate::PreparedXmap15Frame),
    AuthenticatedEmpty(&'a crate::PreparedXmap15Frame),
    Unavailable(&'a portable_render_stream::FoundationGap),
}

impl PreparedFoundationFrame {
    pub fn xmap15_orders_state(&self) -> Result<Xmap15FoundationState<'_>> {
        self.envelope
            .validate()
            .map_err(|error| format!("XMAP15 foundation envelope: {error}"))?;
        let row = self
            .envelope
            .families
            .iter()
            .find(|row| row.family == MapVisualFamily::OrdersFrontsArrows)
            .ok_or("XMAP15 foundation envelope omitted OrdersFrontsArrows")?;
        match (&row.disposition, self.orders.as_ref()) {
            (FamilyDisposition::Present { .. }, Some(prepared)) => {
                validate_xmap15_foundation_disposition(&row.disposition, prepared)?;
                Ok(Xmap15FoundationState::Present(prepared))
            }
            (FamilyDisposition::AuthenticatedEmpty { .. }, Some(prepared)) => {
                validate_xmap15_foundation_disposition(&row.disposition, prepared)?;
                Ok(Xmap15FoundationState::AuthenticatedEmpty(prepared))
            }
            (FamilyDisposition::Unavailable { gap }, None) => {
                Ok(Xmap15FoundationState::Unavailable(gap))
            }
            _ => Err("XMAP15 foundation disposition disagrees with executable payload".into()),
        }
    }

    pub fn require_xmap15_orders(&self) -> Result<&crate::PreparedXmap15Frame> {
        match self.xmap15_orders_state()? {
            Xmap15FoundationState::Present(prepared)
            | Xmap15FoundationState::AuthenticatedEmpty(prepared) => Ok(prepared),
            Xmap15FoundationState::Unavailable(gap) => Err(format!(
                "XMAP15 foundation has no executable orders payload: {}",
                gap.reason
            )
            .into()),
        }
    }

    pub fn xmap16_rail_supply_state(&self) -> Result<Xmap16FoundationState<'_>> {
        self.envelope
            .validate()
            .map_err(|error| format!("XMAP16 foundation envelope: {error}"))?;
        let row = self
            .envelope
            .families
            .iter()
            .find(|row| row.family == MapVisualFamily::RailSupply)
            .ok_or("XMAP16 foundation envelope omitted RailSupply")?;
        match (&row.disposition, self.rail_supply.as_ref()) {
            (FamilyDisposition::Present { .. }, Some(prepared)) => {
                prepared.validate()?;
                validate_xmap16_foundation_disposition(&row.disposition, prepared)?;
                Ok(Xmap16FoundationState::Present(prepared))
            }
            (FamilyDisposition::Unavailable { gap }, None) => {
                Ok(Xmap16FoundationState::Unavailable(gap))
            }
            _ => Err("XMAP16 foundation disposition disagrees with executable payload".into()),
        }
    }

    pub fn require_xmap16_rail_supply(&self) -> Result<&crate::PreparedXmap16Frame> {
        match self.xmap16_rail_supply_state()? {
            Xmap16FoundationState::Present(prepared) => Ok(prepared),
            Xmap16FoundationState::Unavailable(gap) => Err(format!(
                "XMAP16 foundation has no executable rail/supply payload: {}",
                gap.reason
            )
            .into()),
        }
    }
}

fn validate_xmap16_foundation_disposition(
    disposition: &FamilyDisposition,
    prepared: &crate::PreparedXmap16Frame,
) -> Result<()> {
    let FamilyDisposition::Present {
        payload_schema,
        producer_registration_sha256,
        payload_sha256,
        ordered_operations,
        ordered_used_content,
    } = disposition
    else {
        return Err("XMAP16 foundation disposition is not present".into());
    };
    let stages = [
        &prepared.rail_stage,
        &prepared.supply_arrow_stage,
        &prepared.hub_stage,
    ];
    let resources_sha256 = crate::production_features::xmap16::xmap16_resource_order_sha256(
        &prepared.ordered_used_content,
    );
    if payload_schema != crate::XMAP16_RAIL_SUPPLY_SCHEMA
        || payload_sha256 != &prepared.deterministic_sha256
        || ordered_used_content != &prepared.ordered_used_content
    {
        return Err("XMAP16 foundation disposition drifted from executable payload".into());
    }
    validate_xmap16_operation_rows(
        producer_registration_sha256,
        &resources_sha256,
        ordered_operations,
        stages,
    )
}

fn validate_xmap16_operation_rows(
    producer_registration_sha256: &str,
    resources_sha256: &str,
    ordered_operations: &[portable_render_stream::FoundationOperationIdentity],
    stages: [&portable_render_stream::AssignedStage; 3],
) -> Result<()> {
    if producer_registration_sha256
        != crate::production_features::xmap16::xmap16_producer_registration_sha256()
    {
        return Err("XMAP16 foundation producer registration is not the exact a729 program".into());
    }
    if ordered_operations.len() != stages.len() {
        return Err("XMAP16 foundation operation census is incomplete".into());
    }
    let mut target_sha256 = None;
    for (operation, stage) in ordered_operations.iter().zip(stages) {
        stage
            .validate()
            .map_err(|error| format!("XMAP16 foundation assigned stage: {error}"))?;
        if operation.operation_id != stage.operation_name
            || operation.global_order != stage.assigned_ordinal
            || operation.program_sha256 != producer_registration_sha256
            || operation.ordered_resources_sha256 != resources_sha256
            || target_sha256
                .as_ref()
                .is_some_and(|target| target != &operation.target_sha256)
        {
            return Err("XMAP16 foundation operation identity drifted from prepared stages".into());
        }
        target_sha256.get_or_insert_with(|| operation.target_sha256.clone());
    }
    Ok(())
}

/// Execute every predeclared adapter slot and construct the complete portable
/// family manifest. Missing slots are typed `Unavailable`; no family can
/// silently disappear from preparation.
pub fn prepare_foundation_frame(
    world_frame: &WorldFrame,
    viewer: &ViewerOwnedMapState,
    manifest: &ContentManifest,
    map_admission: &MapAuthorityAdmission,
    content_receipt: &FoundationContentExecutionReceipt,
    adapters: &ProductionMapFeatureAdapters<'_>,
) -> Result<PreparedFoundationFrame> {
    let selection = crate::production_features::xmap15::Xmap15ViewerSelection::default();
    prepare_foundation_frame_inner(
        world_frame,
        viewer,
        manifest,
        map_admission,
        content_receipt,
        adapters,
        None,
        None,
        &selection,
    )
}

fn validate_xmap15_foundation_disposition(
    disposition: &FamilyDisposition,
    prepared: &crate::PreparedXmap15Frame,
) -> Result<()> {
    prepared.validate()?;
    match disposition {
        FamilyDisposition::Present {
            payload_schema,
            producer_registration_sha256,
            payload_sha256,
            ordered_operations,
            ordered_used_content,
        } => {
            let resources_sha256 = crate::production_features::xmap15::xmap15_used_content_sha256(
                &prepared.portable.ordered_used_content,
            )?;
            let operation = ordered_operations
                .first()
                .ok_or("XMAP15 present disposition omitted its assigned operation")?;
            let expected_operation = prepared
                .assigned_stage
                .foundation_operation_identity(
                    crate::production_features::xmap15::xmap15_registration_sha256(),
                    resources_sha256,
                    prepared.foundation_target_sha256.clone(),
                )
                .map_err(|error| format!("XMAP15 foundation assigned operation: {error}"))?;
            if prepared.portable.ordered_rows.is_empty()
                || payload_schema != portable_render_stream::XMAP15_STATE_FIRST_SCHEMA
                || producer_registration_sha256
                    != &crate::production_features::xmap15::xmap15_registration_sha256()
                || payload_sha256 != &prepared.portable.deterministic_sha256
                || ordered_operations.len() != 1
                || operation != &expected_operation
                || ordered_used_content != &prepared.portable.ordered_used_content
            {
                return Err("XMAP15 present disposition drifted from executable payload".into());
            }
        }
        FamilyDisposition::AuthenticatedEmpty {
            producer_registration_sha256,
            enumerator_witness_sha256,
        } => {
            if !prepared.portable.ordered_rows.is_empty()
                || producer_registration_sha256
                    != &crate::production_features::xmap15::xmap15_registration_sha256()
                || enumerator_witness_sha256 != &prepared.portable.enumerator_witness_sha256
            {
                return Err(
                    "XMAP15 authenticated-empty disposition drifted from executable payload".into(),
                );
            }
        }
        FamilyDisposition::Unavailable { .. } | FamilyDisposition::NotApplicable { .. } => {
            return Err(
                "XMAP15 executable payload was paired with a non-executable disposition".into(),
            )
        }
    }
    Ok(())
}

fn prepare_foundation_frame_inner(
    world_frame: &WorldFrame,
    viewer: &ViewerOwnedMapState,
    manifest: &ContentManifest,
    map_admission: &MapAuthorityAdmission,
    content_receipt: &FoundationContentExecutionReceipt,
    adapters: &ProductionMapFeatureAdapters<'_>,
    xmap12_building_visual: Option<&crate::Xmap12BuildingVisualFrame>,
    xmap12_fixed_runtime: Option<&crate::Xmap12FixedRuntimeState>,
    xmap15_selection: &crate::production_features::xmap15::Xmap15ViewerSelection,
) -> Result<PreparedFoundationFrame> {
    viewer.validate()?;
    content_receipt.validate()?;
    let manifest_sha256 = manifest
        .manifest_digest_sha256
        .as_ref()
        .ok_or("foundation rendering requires a finalized content manifest")?;
    if content_receipt.content_manifest_sha256 != *manifest_sha256
        || content_receipt.product_modset_sha256 != manifest.product_modset_sha256
        || content_receipt.effective_vfs_manifest_sha256 != manifest.effective_vfs_manifest_sha256
    {
        return Err("foundation content receipt belongs to different replay content".into());
    }
    let viewer_state_sha256 = Sha256::hex(&serde_json::to_vec(viewer)?);

    let mut consumed = BTreeSet::new();
    let mut families = Vec::with_capacity(MapVisualFamily::ALL.len());
    let mut prepared_terrain = None;
    let mut prepared_political = None;
    let mut prepared_thin_borders = None;
    let mut prepared_rivers_straits = None;
    let mut prepared_lighting = None;
    let mut prepared_trees = None;
    let mut prepared_xmap12 = None;
    let mut prepared_weather = None;
    let mut prepared_water = None;
    let mut prepared_effects = None;
    let mut prepared_units = None;
    let mut prepared_combat = None;
    let mut prepared_orders = None;
    let mut prepared_rail_supply = None;
    let mut prepared_labels = None;
    let mut prepared_map_modes = None;
    for family in MapVisualFamily::ALL {
        let context = FoundationFeatureContext {
            world_frame,
            viewer,
            map_authority_gaps: map_admission.gaps(),
            assigned_stage_schedule: adapters.assigned_stage_schedule.as_ref(),
            product_modset_sha256: &manifest.product_modset_sha256,
            effective_vfs_manifest_sha256: &manifest.effective_vfs_manifest_sha256,
            foundation_content_operation_sha256: &content_receipt.deterministic_sha256,
            viewer_state_sha256: &viewer_state_sha256,
            prepared_terrain: prepared_terrain.as_ref(),
            prepared_political: prepared_political.as_ref(),
            prepared_units: prepared_units.as_ref(),
            xmap15_units: prepared_units.as_ref().map(|frame| {
                &frame.identity_provider
                    as &dyn crate::production_features::xmap15::Xmap15UnitIdentityProvider
            }),
            xmap15_selection,
        };
        let xmap12_preparation = if family == MapVisualFamily::CitiesBuildingsMapObjects {
            adapters
                .xmap12
                .as_ref()
                .map(|adapter| {
                    adapter.prepare(&context, xmap12_building_visual, xmap12_fixed_runtime)
                })
                .transpose()?
        } else {
            None
        };
        let xmap11_preparation = if family == MapVisualFamily::Trees {
            adapters
                .xmap11
                .as_ref()
                .map(|adapter| adapter.prepare(&context))
                .transpose()?
        } else {
            None
        };
        let xmap17_preparation = if family == MapVisualFamily::LabelsMapText {
            adapters
                .xmap17
                .as_ref()
                .map(|adapter| adapter.prepare(&context))
                .transpose()?
        } else {
            None
        };
        let disposition = if let Some(xmap11) = xmap11_preparation {
            if prepared_trees.replace(xmap11.prepared).is_some() {
                return Err("foundation produced duplicate XMAP11 tree payload".into());
            }
            let output = xmap11.output;
            for gap_id in output.consumed_gap_ids {
                if !map_admission
                    .gaps()
                    .iter()
                    .any(|gap| gap.input_id == gap_id)
                    || !consumed.insert(gap_id.clone())
                {
                    return Err(format!(
                        "{family:?} claimed an unknown or already-consumed authority gap {gap_id}"
                    )
                    .into());
                }
            }
            output.disposition
        } else if let Some(xmap17) = xmap17_preparation {
            if prepared_labels.replace(xmap17.prepared).is_some() {
                return Err("foundation produced duplicate XMAP17 label payload".into());
            }
            let output = xmap17.output;
            for gap_id in output.consumed_gap_ids {
                if !map_admission
                    .gaps()
                    .iter()
                    .any(|gap| gap.input_id == gap_id)
                    || !consumed.insert(gap_id.clone())
                {
                    return Err(format!(
                        "{family:?} claimed an unknown or already-consumed authority gap {gap_id}"
                    )
                    .into());
                }
            }
            output.disposition
        } else {
            match (xmap12_preparation, adapters.get(family)) {
                (Some(xmap12), _) => {
                    if let Some(payload) = xmap12.prepared {
                        if prepared_xmap12.replace(payload).is_some() {
                            return Err("foundation produced duplicate XMAP12 payload".into());
                        }
                    } else if matches!(
                        &xmap12.output.disposition,
                        FamilyDisposition::Present { .. }
                    ) {
                        return Err(
                        "present CitiesBuildingsMapObjects family omitted its executable payload"
                            .into(),
                    );
                    }
                    let output = xmap12.output;
                    for gap_id in output.consumed_gap_ids {
                        if !map_admission
                            .gaps()
                            .iter()
                            .any(|gap| gap.input_id == gap_id)
                            || !consumed.insert(gap_id.clone())
                        {
                            return Err(format!(
                            "{family:?} claimed an unknown or already-consumed authority gap {gap_id}"
                        )
                        .into());
                        }
                    }
                    output.disposition
                }
                (None, Some(adapter)) => {
                    let output = adapter.prepare(&context)?;
                    if let Some(terrain) = output.prepared_terrain {
                        if family != MapVisualFamily::Terrain
                            || prepared_terrain.replace(terrain).is_some()
                        {
                            return Err(
                            "foundation adapter produced duplicate or non-terrain terrain payload"
                                .into(),
                        );
                        }
                    } else if family == MapVisualFamily::Terrain
                        && matches!(&output.disposition, FamilyDisposition::Present { .. })
                    {
                        return Err(
                            "present Terrain family omitted its executable draw payload".into()
                        );
                    }
                    if let Some(political) = output.prepared_political {
                        if family != MapVisualFamily::PoliticalFillGradients
                            || prepared_political.replace(political).is_some()
                        {
                            return Err(
                            "foundation adapter produced duplicate or non-political XMAP05 payload"
                                .into(),
                        );
                        }
                    } else if family == MapVisualFamily::PoliticalFillGradients
                        && matches!(&output.disposition, FamilyDisposition::Present { .. })
                    {
                        return Err(
                            "present PoliticalFillGradients family omitted its executable payload"
                                .into(),
                        );
                    }
                    if let Some(thin_borders) = output.prepared_thin_borders {
                        if family != MapVisualFamily::ThinBorders
                            || prepared_thin_borders.replace(thin_borders).is_some()
                        {
                            return Err(
                            "foundation adapter produced duplicate or non-XMAP06 border payload"
                                .into(),
                        );
                        }
                    } else if family == MapVisualFamily::ThinBorders
                        && matches!(&output.disposition, FamilyDisposition::Present { .. })
                    {
                        return Err(
                            "present ThinBorders family omitted its executable geometry payload"
                                .into(),
                        );
                    }
                    if let Some(lighting) = output.prepared_lighting {
                        if family != MapVisualFamily::LightingShadows
                            || prepared_lighting.replace(lighting).is_some()
                        {
                            return Err(
                            "foundation adapter produced duplicate or non-XMAP09 lighting payload"
                                .into(),
                        );
                        }
                    } else if family == MapVisualFamily::LightingShadows
                        && matches!(&output.disposition, FamilyDisposition::Present { .. })
                    {
                        return Err(
                            "present LightingShadows family omitted its executable handoff payload"
                                .into(),
                        );
                    }
                    if let Some(weather) = output.prepared_weather {
                        if family != MapVisualFamily::WeatherAtmosphere
                            || prepared_weather.replace(weather).is_some()
                        {
                            return Err(
                            "foundation adapter produced duplicate or non-weather XMAP19 payload"
                                .into(),
                        );
                        }
                    } else if family == MapVisualFamily::WeatherAtmosphere
                        && matches!(&output.disposition, FamilyDisposition::Present { .. })
                    {
                        return Err(
                            "present WeatherAtmosphere family omitted its executable payload"
                                .into(),
                        );
                    }
                    if let Some(rivers_straits) = output.prepared_rivers_straits {
                        if family != MapVisualFamily::RiversStraits
                            || prepared_rivers_straits.replace(rivers_straits).is_some()
                        {
                            return Err(
                            "foundation adapter produced duplicate or non-XMAP08 rivers/straits payload"
                                .into(),
                        );
                        }
                    } else if family == MapVisualFamily::RiversStraits
                        && matches!(&output.disposition, FamilyDisposition::Present { .. })
                    {
                        return Err(
                            "present RiversStraits family omitted its executable draw payload"
                                .into(),
                        );
                    }
                    if let Some(water) = output.prepared_water {
                        if family != MapVisualFamily::WaterIce
                            || prepared_water.replace(water).is_some()
                        {
                            return Err(
                                "foundation adapter produced duplicate or non-water XMAP07 payload"
                                    .into(),
                            );
                        }
                    } else if family == MapVisualFamily::WaterIce
                        && matches!(&output.disposition, FamilyDisposition::Present { .. })
                    {
                        return Err("present WaterIce family omitted its executable payload".into());
                    }
                    if let Some(effects) = output.prepared_effects {
                        if family != MapVisualFamily::ParticlesEffects
                            || prepared_effects.replace(effects).is_some()
                        {
                            return Err(
                            "foundation adapter produced duplicate or non-XMAP20 effects payload"
                                .into(),
                        );
                        }
                    } else if family == MapVisualFamily::ParticlesEffects
                        && matches!(&output.disposition, FamilyDisposition::Present { .. })
                    {
                        return Err(
                            "present ParticlesEffects family omitted its executable payload".into(),
                        );
                    }
                    if let Some(units) = output.prepared_units {
                        if family != MapVisualFamily::UnitsCounters
                            || prepared_units.replace(units).is_some()
                        {
                            return Err(
                                "foundation adapter produced duplicate or non-XMAP13 unit payload"
                                    .into(),
                            );
                        }
                    } else if family == MapVisualFamily::UnitsCounters
                        && matches!(&output.disposition, FamilyDisposition::Present { .. })
                    {
                        return Err(
                            "present UnitsCounters family omitted its executable payload".into(),
                        );
                    }
                    if let Some(combat) = output.prepared_combat {
                        if family != MapVisualFamily::CombatMarkers
                            || prepared_combat.replace(combat).is_some()
                        {
                            return Err(
                            "foundation adapter produced duplicate or non-XMAP14 combat payload"
                                .into(),
                        );
                        }
                    } else if family == MapVisualFamily::CombatMarkers
                        && matches!(
                            &output.disposition,
                            FamilyDisposition::Present { .. }
                                | FamilyDisposition::AuthenticatedEmpty { .. }
                        )
                    {
                        return Err(
                        "present/authenticated-empty CombatMarkers family omitted its executable payload"
                            .into(),
                    );
                    }
                    if let Some(orders) = output.prepared_orders {
                        if family != MapVisualFamily::OrdersFrontsArrows
                            || prepared_orders.replace(orders).is_some()
                        {
                            return Err(
                            "foundation adapter produced duplicate or non-XMAP15 orders payload"
                                .into(),
                        );
                        }
                    } else if family == MapVisualFamily::OrdersFrontsArrows
                        && matches!(
                            &output.disposition,
                            FamilyDisposition::Present { .. }
                                | FamilyDisposition::AuthenticatedEmpty { .. }
                        )
                    {
                        return Err(
                        "present/authenticated-empty OrdersFrontsArrows family omitted its executable payload"
                            .into(),
                    );
                    }
                    if let Some(rail_supply) = output.prepared_rail_supply {
                        if family != MapVisualFamily::RailSupply
                            || prepared_rail_supply.replace(rail_supply).is_some()
                        {
                            return Err(
                                "foundation adapter produced duplicate or non-XMAP16 rail payload"
                                    .into(),
                            );
                        }
                    } else if family == MapVisualFamily::RailSupply
                        && matches!(&output.disposition, FamilyDisposition::Present { .. })
                    {
                        return Err(
                            "present RailSupply family omitted its executable payload".into()
                        );
                    }
                    if let Some(map_modes) = output.prepared_map_modes {
                        if family != MapVisualFamily::MapModes
                            || prepared_map_modes.replace(map_modes).is_some()
                        {
                            return Err(
                                "foundation adapter produced duplicate or non-XMAP18 map-mode payload"
                                    .into(),
                            );
                        }
                    } else if family == MapVisualFamily::MapModes
                        && matches!(&output.disposition, FamilyDisposition::Present { .. })
                    {
                        return Err("present MapModes family omitted its executable payload".into());
                    }
                    for gap_id in output.consumed_gap_ids {
                        if !map_admission
                            .gaps()
                            .iter()
                            .any(|gap| gap.input_id == gap_id)
                            || !consumed.insert(gap_id.clone())
                        {
                            return Err(format!(
                            "{family:?} claimed an unknown or already-consumed authority gap {gap_id}"
                        )
                        .into());
                        }
                    }
                    output.disposition
                }
                (None, None) => FamilyDisposition::Unavailable {
                    gap: unimplemented_adapter_gap(family, world_frame.hour_index()),
                },
            }
        };
        families.push(FamilyCoverageRow {
            family,
            owner_packet: family.owner_packet().into(),
            disposition,
        });
    }

    // A promoted adapter set may not strand an XMAP00 semantic gap outside the
    // exhaustive family projection. While a slot is unimplemented, the
    // canonical slot gap subsumes its owner's unproved semantic inputs.
    for authority_gap in map_admission.gaps() {
        if consumed.contains(&authority_gap.input_id) {
            continue;
        }
        let owner = owner_packet_for_authority_gap(&authority_gap.input_id)
            .ok_or_else(|| format!("unclassified map-authority gap {}", authority_gap.input_id))?;
        if !families.iter().any(|row| {
            row.owner_packet == owner
                && matches!(row.disposition, FamilyDisposition::Unavailable { .. })
        }) {
            return Err(format!(
                "map-authority gap {} is an ordinary omission from an otherwise present adapter",
                authority_gap.input_id
            )
            .into());
        }
    }

    let envelope = FoundationRenderEnvelope {
        schema: FOUNDATION_RENDER_SCHEMA.into(),
        hour_index: world_frame.hour_index(),
        content_manifest_sha256: manifest_sha256.clone(),
        product_modset_sha256: manifest.product_modset_sha256.clone(),
        effective_vfs_manifest_sha256: manifest.effective_vfs_manifest_sha256.clone(),
        // The sealed receipt binds the XMAP02 canonical shipping authority,
        // resolved operation, compiled stages, and created resource/descriptor.
        foundation_content_operation_sha256: content_receipt.deterministic_sha256.clone(),
        families,
    };
    envelope.validate()?;
    // The real wire codec is exercised here, not only in a test helper.
    let envelope = FoundationRenderEnvelope::decode_json(&envelope.encode_json()?)
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let terrain =
        prepared_terrain.ok_or("foundation Terrain adapter produced no executable draw payload")?;
    terrain.validate()?;
    let political = prepared_political
        .ok_or("foundation XMAP05 adapter produced no executable political payload")?;
    political.validate()?;
    let map_modes =
        prepared_map_modes.ok_or("foundation XMAP18 adapter produced no map-mode payload")?;
    map_modes.validate()?;
    if map_modes.composition == crate::MapModeComposition::Supply {
        let rail_supply = envelope
            .families
            .iter()
            .find(|row| row.family == MapVisualFamily::RailSupply)
            .ok_or("foundation envelope omitted RailSupply")?;
        if !matches!(
            &rail_supply.disposition,
            FamilyDisposition::Present { .. } | FamilyDisposition::AuthenticatedEmpty { .. }
        ) {
            return Err(
                "XMAP18 Supply mode requires XMAP16's promoted mode-neutral rail/supply payload"
                    .into(),
            );
        }
    }
    if matches!(
        map_modes.composition,
        crate::MapModeComposition::ResourceFurniture | crate::MapModeComposition::RaidFurniture
    ) {
        let furniture = envelope
            .families
            .iter()
            .find(|row| row.family == MapVisualFamily::MapFurnitureIcons)
            .ok_or("foundation envelope omitted MapFurnitureIcons")?;
        if !matches!(
            &furniture.disposition,
            FamilyDisposition::Present { .. } | FamilyDisposition::AuthenticatedEmpty { .. }
        ) {
            return Err(
                "XMAP18 resource/raid mode requires XMAP21's promoted map-furniture payload".into(),
            );
        }
    }
    let map_modes_row = envelope
        .families
        .iter()
        .find(|row| row.family == MapVisualFamily::MapModes)
        .ok_or("foundation envelope omitted MapModes")?;
    if !matches!(
        &map_modes_row.disposition,
        FamilyDisposition::Present { payload_sha256, .. }
            if payload_sha256 == &map_modes.deterministic_sha256
    ) {
        return Err("foundation XMAP18 family row disagrees with executable payload".into());
    }
    let water =
        prepared_water.ok_or("foundation XMAP07 adapter produced no executable water payload")?;
    water.validate()?;
    let political_row = envelope
        .families
        .iter()
        .find(|row| row.family == MapVisualFamily::PoliticalFillGradients)
        .ok_or("foundation envelope omitted PoliticalFillGradients")?;
    let fow_row = envelope
        .families
        .iter()
        .find(|row| row.family == MapVisualFamily::FogOfWar)
        .ok_or("foundation envelope omitted FogOfWar")?;
    if !matches!(
        &political_row.disposition,
        FamilyDisposition::Present { payload_sha256, .. }
            if payload_sha256 == &political.political_family_sha256
    ) || !matches!(
        &fow_row.disposition,
        FamilyDisposition::Present { payload_sha256, .. }
            if payload_sha256 == &political.fow_family_sha256
    ) {
        return Err("foundation XMAP05 family rows disagree with executable payload".into());
    }
    let thin_borders = if adapters.get(MapVisualFamily::ThinBorders).is_some() {
        let thin_borders = prepared_thin_borders
            .ok_or("foundation XMAP06 adapter produced no executable border geometry payload")?;
        thin_borders.validate()?;
        let thin_border_row = envelope
            .families
            .iter()
            .find(|row| row.family == MapVisualFamily::ThinBorders)
            .ok_or("foundation envelope omitted ThinBorders")?;
        match &thin_border_row.disposition {
            FamilyDisposition::Present { payload_sha256, .. }
                if payload_sha256 == &thin_borders.deterministic_sha256 => {}
            FamilyDisposition::Unavailable { gap }
                if gap.gap_id
                    == crate::production_features::xmap06::XMAP06_RUNTIME_BUCKET_GAP_ID => {}
            _ => {
                return Err(
                    "foundation XMAP06 family row disagrees with its prepared geometry/blocker"
                        .into(),
                )
            }
        }
        Some(thin_borders)
    } else {
        if prepared_thin_borders.is_some() {
            return Err(
                "foundation produced XMAP06 geometry without registering its adapter".into(),
            );
        }
        None
    };
    if let Some(lighting) = &prepared_lighting {
        lighting.validate()?;
    }
    if let Some(trees) = &prepared_trees {
        trees.validate()?;
        let row = envelope
            .families
            .iter()
            .find(|row| row.family == MapVisualFamily::Trees)
            .ok_or("foundation envelope omitted Trees")?;
        let disposition_matches = match &row.disposition {
            FamilyDisposition::Present { payload_sha256, .. } => {
                !trees.ordered_main_draw_batch_indices.is_empty()
                    && payload_sha256 == &trees.prepared_frame_sha256
            }
            FamilyDisposition::AuthenticatedEmpty { .. } => {
                trees.ordered_main_draw_batch_indices.is_empty()
            }
            _ => false,
        };
        if !disposition_matches
            || trees.hour_index != world_frame.hour_index()
            || trees.replay_frame_identity_sha256 != terrain.replay_frame_identity_sha256
            || trees.viewer_state_sha256 != viewer_state_sha256
            || trees.map_copy_receipt_set_sha256 != terrain.map_copy_receipt_set_sha256()?
        {
            return Err("foundation XMAP11 family row/payload/replay/viewer identity is not current"
                .into());
        }
    } else if adapters.has(MapVisualFamily::Trees) {
        return Err("foundation XMAP11 adapter omitted its complete prepared frame".into());
    }
    if let Some(xmap12) = &prepared_xmap12 {
        xmap12.validate()?;
        let row = envelope
            .families
            .iter()
            .find(|row| row.family == MapVisualFamily::CitiesBuildingsMapObjects)
            .ok_or("foundation envelope omitted CitiesBuildingsMapObjects")?;
        if !matches!(
            &row.disposition,
            FamilyDisposition::Present { payload_sha256, .. }
                if payload_sha256 == &xmap12.deterministic_sha256
        ) || xmap12.replay_frame_identity_sha256 != terrain.replay_frame_identity_sha256
            || xmap12.viewer_state_sha256
                != viewer
                    .camera
                    .as_ref()
                    .ok_or("present XMAP12 payload has no viewer camera")?
                    .deterministic_sha256
        {
            return Err(
                "foundation XMAP12 family row/payload/replay/viewer identity is not current".into(),
            );
        }
    } else if adapters.has(MapVisualFamily::CitiesBuildingsMapObjects) {
        let row = envelope
            .families
            .iter()
            .find(|row| row.family == MapVisualFamily::CitiesBuildingsMapObjects)
            .ok_or("foundation envelope omitted CitiesBuildingsMapObjects")?;
        if !matches!(&row.disposition, FamilyDisposition::Unavailable { .. }) {
            return Err(
                "foundation XMAP12 omitted its payload without an explicit unavailable disposition"
                    .into(),
            );
        }
    }
    if let Some(weather) = &prepared_weather {
        weather.validate()?;
        let weather_row = envelope
            .families
            .iter()
            .find(|row| row.family == MapVisualFamily::WeatherAtmosphere)
            .ok_or("foundation envelope omitted WeatherAtmosphere")?;
        if !matches!(
            &weather_row.disposition,
            FamilyDisposition::Present { payload_sha256, .. }
                if payload_sha256 == &weather.deterministic_sha256
        ) {
            return Err("foundation XMAP19 family row disagrees with executable payload".into());
        }
    }
    if let Some(rivers_straits) = &prepared_rivers_straits {
        rivers_straits.validate()?;
    }
    let water_row = envelope
        .families
        .iter()
        .find(|row| row.family == MapVisualFamily::WaterIce)
        .ok_or("foundation envelope omitted WaterIce")?;
    if !matches!(
        &water_row.disposition,
        FamilyDisposition::Present { payload_sha256, .. }
            if payload_sha256 == &water.deterministic_sha256
    ) {
        return Err("foundation XMAP07 family row disagrees with executable payload".into());
    }
    let rail_row = envelope
        .families
        .iter()
        .find(|row| row.family == MapVisualFamily::RailSupply)
        .ok_or("foundation envelope omitted RailSupply")?;
    match (&rail_row.disposition, &prepared_rail_supply) {
        (FamilyDisposition::Present { .. }, Some(rail_supply)) => {
            rail_supply.validate()?;
            validate_xmap16_foundation_disposition(&rail_row.disposition, rail_supply)?;
        }
        (FamilyDisposition::Unavailable { .. }, None) => {}
        _ => return Err("foundation XMAP16 family row disagrees with executable payload".into()),
    }
    let combat_row = envelope
        .families
        .iter()
        .find(|row| row.family == MapVisualFamily::CombatMarkers)
        .ok_or("foundation envelope omitted CombatMarkers")?;
    match (&combat_row.disposition, &prepared_combat) {
        (FamilyDisposition::Present { payload_sha256, .. }, Some(combat))
            if payload_sha256 == &combat.deterministic_sha256 && !combat.indicators.is_empty() =>
        {
            combat.validate()?;
        }
        (
            FamilyDisposition::AuthenticatedEmpty {
                enumerator_witness_sha256,
                ..
            },
            Some(combat),
        ) if combat.indicators.is_empty()
            && enumerator_witness_sha256 == &combat.enumerator_witness_sha256 =>
        {
            combat.validate()?;
        }
        (FamilyDisposition::Unavailable { .. }, None) => {}
        _ => {
            return Err(
                "foundation XMAP14 family row disagrees with its executable combat payload".into(),
            )
        }
    }
    let orders_row = envelope
        .families
        .iter()
        .find(|row| row.family == MapVisualFamily::OrdersFrontsArrows)
        .ok_or("foundation envelope omitted OrdersFrontsArrows")?;
    match (&orders_row.disposition, &prepared_orders) {
        (FamilyDisposition::Present { payload_sha256, .. }, Some(orders))
            if payload_sha256 == &orders.portable.deterministic_sha256
                && !orders.portable.ordered_rows.is_empty() =>
        {
            orders.validate()?;
        }
        (
            FamilyDisposition::AuthenticatedEmpty {
                enumerator_witness_sha256,
                ..
            },
            Some(orders),
        ) if orders.portable.ordered_rows.is_empty()
            && enumerator_witness_sha256 == &orders.portable.enumerator_witness_sha256 =>
        {
            orders.validate()?;
        }
        (FamilyDisposition::Unavailable { .. }, None) => {}
        _ => {
            return Err(
                "foundation XMAP15 family row disagrees with its executable orders payload".into(),
            )
        }
    }
    let labels = if adapters.has(MapVisualFamily::LabelsMapText) {
        let labels = prepared_labels
            .ok_or("foundation XMAP17 adapter produced no executable label payload")?;
        labels.validate()?;
        let row = envelope
            .families
            .iter()
            .find(|row| row.family == MapVisualFamily::LabelsMapText)
            .ok_or("foundation envelope omitted LabelsMapText")?;
        if !matches!(
            &row.disposition,
            FamilyDisposition::Present { payload_sha256, .. }
                if payload_sha256 == &labels.deterministic_sha256
        ) {
            return Err("foundation XMAP17 family row disagrees with executable payload".into());
        }
        Some(labels)
    } else {
        if prepared_labels.is_some() {
            return Err("foundation produced XMAP17 labels without registering its adapter".into());
        }
        None
    };
    Ok(PreparedFoundationFrame {
        envelope,
        terrain,
        political,
        thin_borders,
        lighting: prepared_lighting,
        trees: prepared_trees,
        xmap12: prepared_xmap12,
        weather: prepared_weather,
        rivers_straits: prepared_rivers_straits,
        water,
        effects: prepared_effects,
        units: prepared_units,
        combat: prepared_combat,
        orders: prepared_orders,
        rail_supply: prepared_rail_supply,
        labels,
        map_modes,
    })
}

fn unimplemented_adapter_gap(family: MapVisualFamily, hour: i64) -> FoundationGap {
    let owner = family.owner_packet();
    let gap_id = format!("{owner}.adapter_unimplemented");
    let reason = format!(
        "{owner} has not promoted the {:?} viewer-owned production adapter",
        family
    );
    let witness_sha256 = Sha256::hex(format!("{gap_id}\n{hour}\n{hour}\n{reason}").as_bytes());
    FoundationGap {
        gap_id,
        first_unavailable_hour: hour,
        last_unavailable_hour: hour,
        witness_sha256,
        reason,
    }
}

fn owner_packet_for_authority_gap(input_id: &str) -> Option<&'static str> {
    if input_id.starts_with("combat.") {
        Some("XMAP14")
    } else if input_id.starts_with("control.")
        || input_id == "country.d18_complete_roster"
        || input_id == "country.presentation_identity"
        || input_id.starts_with("fog.")
    {
        Some("XMAP05")
    } else if input_id == "country.current_capital_state"
        || input_id == "mapmode.raids"
        || input_id == "mapmode.resource_deposits"
        || input_id.starts_with("map_furniture.")
    {
        Some("XMAP21")
    } else if input_id == "mapmode.supply_state" {
        Some("XMAP16")
    } else if input_id.starts_with("diplomacy.")
        || input_id.starts_with("mapmode.")
        || input_id == "session.player_country_set"
    {
        Some("XMAP18")
    } else if input_id.starts_with("effects.") {
        Some("XMAP20")
    } else if input_id.starts_with("lighting.") || input_id == "world.calendar_time" {
        Some("XMAP09")
    } else if input_id.starts_with("logistics.") {
        Some("XMAP16")
    } else if input_id.starts_with("objects.") {
        Some("XMAP12")
    } else if input_id == "orders.army_battleplans_semantic" {
        Some("XMAP15")
    } else if input_id == "orders.theatre_army_hierarchy" || input_id.starts_with("units.") {
        Some("XMAP13")
    } else if input_id.starts_with("postfx.") {
        Some("XMAP10")
    } else if input_id == "terrain.snow_mud_semantic"
        || input_id == "weather.province_snow_accumulator"
    {
        Some("XMAP07")
    } else if input_id.starts_with("weather.") {
        Some("XMAP19")
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FoundationPlaybackOutcome {
    Fresh(FoundationRenderEnvelope),
    Stale {
        requested_hour: i64,
        rendered_hour: i64,
        gaps: Vec<FoundationGap>,
        frame: FoundationRenderEnvelope,
    },
    Unavailable {
        requested_hour: i64,
        gaps: Vec<FoundationGap>,
    },
}

#[derive(Default)]
pub struct FoundationPlaybackState {
    last_complete: Option<FoundationRenderEnvelope>,
}

impl FoundationPlaybackState {
    pub fn admit(
        &mut self,
        candidate: FoundationRenderEnvelope,
    ) -> Result<FoundationPlaybackOutcome> {
        self.admit_with_nonblocking_family(candidate, None)
    }

    /// Product playback may continue with an exact current-hour foundation
    /// when RailSupply alone is typed unavailable. No prior-hour XMAP16 state
    /// is reused, and every other unavailable family remains a fatal blocker
    /// at [`require_fresh_foundation_frame`].
    pub fn admit_allowing_xmap16_unavailable(
        &mut self,
        candidate: FoundationRenderEnvelope,
    ) -> Result<FoundationPlaybackOutcome> {
        self.admit_with_nonblocking_family(candidate, Some(MapVisualFamily::RailSupply))
    }

    fn admit_with_nonblocking_family(
        &mut self,
        candidate: FoundationRenderEnvelope,
        nonblocking_family: Option<MapVisualFamily>,
    ) -> Result<FoundationPlaybackOutcome> {
        candidate.validate()?;
        let gaps = candidate
            .gaps()
            .filter(|(family, _)| Some(**family) != nonblocking_family)
            .map(|(_, gap)| gap.clone())
            .collect::<Vec<_>>();
        if gaps.is_empty() {
            if candidate.is_complete() {
                self.last_complete = Some(candidate.clone());
            }
            return Ok(FoundationPlaybackOutcome::Fresh(candidate));
        }
        if let Some(last) = &self.last_complete {
            if last.content_manifest_sha256 == candidate.content_manifest_sha256
                && last.product_modset_sha256 == candidate.product_modset_sha256
                && last.effective_vfs_manifest_sha256 == candidate.effective_vfs_manifest_sha256
                && last.foundation_content_operation_sha256
                    == candidate.foundation_content_operation_sha256
            {
                return Ok(FoundationPlaybackOutcome::Stale {
                    requested_hour: candidate.hour_index,
                    rendered_hour: last.hour_index,
                    gaps,
                    frame: last.clone(),
                });
            }
        }
        Ok(FoundationPlaybackOutcome::Unavailable {
            requested_hour: candidate.hour_index,
            gaps,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundationProductUnavailable {
    pub code: &'static str,
    pub requested_hour: i64,
    pub gap_ids: Vec<String>,
}

impl std::fmt::Display for FoundationProductUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}: requested hour {} is unavailable (gaps={})",
            self.code,
            self.requested_hour,
            self.gap_ids.join(",")
        )
    }
}

impl std::error::Error for FoundationProductUnavailable {}

pub fn require_fresh_foundation_frame(
    outcome: FoundationPlaybackOutcome,
) -> std::result::Result<FoundationRenderEnvelope, FoundationProductUnavailable> {
    match outcome {
        FoundationPlaybackOutcome::Fresh(frame) => Ok(frame),
        FoundationPlaybackOutcome::Stale {
            requested_hour,
            gaps,
            ..
        }
        | FoundationPlaybackOutcome::Unavailable {
            requested_hour,
            gaps,
        } => Err(FoundationProductUnavailable {
            code: "FOUNDATION_MAP_UNAVAILABLE",
            requested_hour,
            gap_ids: gaps.into_iter().map(|gap| gap.gap_id).collect(),
        }),
    }
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase SHA-256").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_foundation_topology_seals_packet_chains_and_independent_stages() {
        use crate::production_features::{
            xmap06, xmap07, xmap08, xmap09, xmap13, xmap14, xmap15, xmap16, xmap17, xmap19, xmap20,
        };

        let topology = product_foundation_assigned_stage_topology().unwrap();
        let schedule =
            portable_render_stream::AssignedStageSchedule::from_topology(&topology).unwrap();
        schedule.validate_against(&topology).unwrap();

        let border_early = schedule
            .require(
                xmap06::XMAP06_EARLY_STAGE_NAME,
                xmap06::XMAP06_EARLY_OPERATION_NAME,
            )
            .unwrap();
        let border_late = schedule
            .require(
                xmap06::XMAP06_LATE_STAGE_NAME,
                xmap06::XMAP06_LATE_OPERATION_NAME,
            )
            .unwrap();
        let lighting_prepare = schedule
            .require(
                xmap09::XMAP09_PREPARE_STAGE_NAME,
                xmap09::XMAP09_PREPARE_OPERATION_NAME,
            )
            .unwrap();
        let shadow_execute = schedule
            .require(
                xmap09::XMAP09_EXECUTE_STAGE_NAME,
                xmap09::XMAP09_EXECUTE_OPERATION_NAME,
            )
            .unwrap();
        let weather_prepare = schedule
            .require(
                xmap19::XMAP19_ASSIGNED_STAGE_NAME,
                xmap19::XMAP19_ASSIGNED_OPERATION_NAME,
            )
            .unwrap();
        let water_surface = schedule
            .require(
                xmap07::XMAP07_ASSIGNED_STAGE_NAME,
                xmap07::XMAP07_ASSIGNED_OPERATION_NAME,
            )
            .unwrap();
        let trees = schedule
            .require(
                crate::XMAP11_TREE_STAGE_NAME,
                crate::XMAP11_TREE_OPERATION_NAME,
            )
            .unwrap();
        let rivers = schedule
            .require(
                xmap08::XMAP08_RIVER_STAGE_NAME,
                xmap08::XMAP08_RIVER_OPERATION_ID,
            )
            .unwrap();
        let straits = schedule
            .require(
                xmap08::XMAP08_STRAIT_STAGE_NAME,
                xmap08::XMAP08_STRAIT_OPERATION_ID,
            )
            .unwrap();
        let unit_entities = schedule
            .require(
                xmap13::XMAP13_UNIT_ENTITY_STAGE_NAME,
                xmap13::XMAP13_UNIT_ENTITY_OPERATION_NAME,
            )
            .unwrap();
        let counter_quads = schedule
            .require(
                xmap13::XMAP13_COUNTER_QUAD_STAGE_NAME,
                xmap13::XMAP13_COUNTER_QUAD_OPERATION_NAME,
            )
            .unwrap();
        let combat_indicators = schedule
            .require(
                xmap14::XMAP14_COMBAT_STAGE_NAME,
                xmap14::XMAP14_COMBAT_OPERATION_NAME,
            )
            .unwrap();
        let orders = schedule
            .require(
                xmap15::XMAP15_ASSIGNED_STAGE_NAME,
                xmap15::XMAP15_ASSIGNED_OPERATION_NAME,
            )
            .unwrap();
        let particles = schedule
            .require(
                xmap20::XMAP20_ASSIGNED_STAGE_NAME,
                xmap20::XMAP20_ASSIGNED_OPERATION_NAME,
            )
            .unwrap();
        let labels_prepare = schedule
            .require(
                xmap17::XMAP17_PREPARE_STAGE,
                xmap17::XMAP17_PREPARE_OPERATION,
            )
            .unwrap();
        let labels_execute = schedule
            .require(
                xmap17::XMAP17_EXECUTE_STAGE,
                xmap17::XMAP17_EXECUTE_OPERATION,
            )
            .unwrap();
        let rail_execute = schedule
            .require(
                crate::XMAP16_RAIL_STAGE_NAME,
                crate::XMAP16_RAIL_OPERATION_ID,
            )
            .unwrap();
        let supply_arrow_execute = schedule
            .require(
                crate::XMAP16_SUPPLY_ARROW_OPERATION_ID,
                crate::XMAP16_SUPPLY_ARROW_OPERATION_NAME,
            )
            .unwrap();
        let hub_execute = schedule
            .require(
                xmap16::XMAP16_HUB_STAGE_NAME,
                xmap16::XMAP16_HUB_OPERATION_NAME,
            )
            .unwrap();
        let furniture_state = schedule
            .require("map_mode", "map_mode.furniture_state")
            .unwrap();
        let furniture_prepare = schedule
            .require(
                crate::XMAP21_PREPARE_STAGE_NAME,
                crate::XMAP21_PREPARE_OPERATION_NAME,
            )
            .unwrap();
        let furniture_execute = schedule
            .require(
                crate::XMAP21_EXECUTE_STAGE_NAME,
                crate::XMAP21_EXECUTE_OPERATION_NAME,
            )
            .unwrap();

        assert_eq!(topology.ordered_stages().len(), 22);
        assert_eq!(topology.ordered_edges().len(), 18);
        assert!(border_early.assigned_ordinal < rivers.assigned_ordinal);
        assert!(rivers.assigned_ordinal < straits.assigned_ordinal);
        assert!(straits.assigned_ordinal < rail_execute.assigned_ordinal);
        assert!(rail_execute.assigned_ordinal < trees.assigned_ordinal);
        assert!(trees.assigned_ordinal < water_surface.assigned_ordinal);
        assert!(water_surface.assigned_ordinal < border_late.assigned_ordinal);
        assert!(border_late.assigned_ordinal < supply_arrow_execute.assigned_ordinal);
        assert!(supply_arrow_execute.assigned_ordinal < particles.assigned_ordinal);
        assert!(supply_arrow_execute.assigned_ordinal < labels_prepare.assigned_ordinal);
        assert!(labels_prepare.assigned_ordinal < labels_execute.assigned_ordinal);
        assert!(labels_execute.assigned_ordinal < particles.assigned_ordinal);
        assert!(particles.assigned_ordinal < hub_execute.assigned_ordinal);
        assert!(orders.assigned_ordinal < counter_quads.assigned_ordinal);
        assert!(counter_quads.assigned_ordinal < combat_indicators.assigned_ordinal);
        assert!(combat_indicators.assigned_ordinal < hub_execute.assigned_ordinal);
        assert!(furniture_state.assigned_ordinal < furniture_prepare.assigned_ordinal);
        assert!(furniture_prepare.assigned_ordinal < furniture_execute.assigned_ordinal);
        assert!(lighting_prepare.assigned_ordinal < shadow_execute.assigned_ordinal);
        assert_eq!(rivers.assigned_ordinal, 2);
        assert_eq!(straits.assigned_ordinal, 3);
        assert_eq!(rail_execute.assigned_ordinal, 4);
        assert_eq!(trees.assigned_ordinal, 5);
        assert_eq!(water_surface.assigned_ordinal, 6);
        assert_eq!(border_late.assigned_ordinal, 7);
        assert_eq!(supply_arrow_execute.assigned_ordinal, 8);
        assert_eq!(weather_prepare.assigned_ordinal, 11);
        assert_eq!(unit_entities.assigned_ordinal, 12);
        assert_eq!(orders.assigned_ordinal, 13);
        assert_eq!(counter_quads.assigned_ordinal, 14);
        assert_eq!(combat_indicators.assigned_ordinal, 15);
        assert_eq!(labels_prepare.assigned_ordinal, 16);
        assert_eq!(labels_execute.assigned_ordinal, 17);
        assert_eq!(particles.assigned_ordinal, 18);
        assert_eq!(hub_execute.assigned_ordinal, 19);
        assert_eq!(furniture_state.assigned_ordinal, 20);
        assert_eq!(furniture_prepare.assigned_ordinal, 21);
        assert_eq!(furniture_execute.assigned_ordinal, 22);
        assert_ne!(weather_prepare.assigned_ordinal, 0);
        assert_ne!(hub_execute.assigned_ordinal, 0);
        assert_eq!(
            topology.ordered_edges(),
            &[
                portable_render_stream::AssignedStageEdge::new(
                    border_early.identity(),
                    rivers.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    rivers.identity(),
                    straits.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    straits.identity(),
                    rail_execute.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    rail_execute.identity(),
                    trees.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    trees.identity(),
                    water_surface.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    water_surface.identity(),
                    border_late.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    border_late.identity(),
                    supply_arrow_execute.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    lighting_prepare.identity(),
                    shadow_execute.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    supply_arrow_execute.identity(),
                    particles.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    supply_arrow_execute.identity(),
                    labels_prepare.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    labels_prepare.identity(),
                    labels_execute.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    labels_execute.identity(),
                    particles.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    particles.identity(),
                    hub_execute.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    orders.identity(),
                    counter_quads.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    counter_quads.identity(),
                    combat_indicators.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    combat_indicators.identity(),
                    hub_execute.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    furniture_state.identity(),
                    furniture_prepare.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    furniture_prepare.identity(),
                    furniture_execute.identity(),
                ),
            ]
        );
    }

    #[test]
    fn xmap16_operation_rows_are_bound_to_the_three_prepared_assignments() {
        use crate::production_features::xmap16;

        let topology = product_foundation_assigned_stage_topology().unwrap();
        let schedule =
            portable_render_stream::AssignedStageSchedule::from_topology(&topology).unwrap();
        let stages = [
            schedule
                .require(
                    crate::XMAP16_RAIL_STAGE_NAME,
                    crate::XMAP16_RAIL_OPERATION_ID,
                )
                .unwrap(),
            schedule
                .require(
                    crate::XMAP16_SUPPLY_ARROW_OPERATION_ID,
                    crate::XMAP16_SUPPLY_ARROW_OPERATION_NAME,
                )
                .unwrap(),
            schedule
                .require(
                    xmap16::XMAP16_HUB_STAGE_NAME,
                    xmap16::XMAP16_HUB_OPERATION_NAME,
                )
                .unwrap(),
        ];
        let registration =
            crate::production_features::xmap16::xmap16_producer_registration_sha256();
        let resources = "b".repeat(64);
        let target = "c".repeat(64);
        let operations = stages
            .iter()
            .map(|stage| {
                stage
                    .foundation_operation_identity(
                        registration.clone(),
                        resources.clone(),
                        target.clone(),
                    )
                    .unwrap()
            })
            .collect::<Vec<_>>();
        validate_xmap16_operation_rows(&registration, &resources, &operations, stages).unwrap();

        let mut wrong_owner = operations.clone();
        wrong_owner[1].program_sha256 = "d".repeat(64);
        assert!(
            validate_xmap16_operation_rows(&registration, &resources, &wrong_owner, stages,)
                .is_err()
        );

        let forged_registration = "d".repeat(64);
        let forged_operations = stages
            .iter()
            .map(|stage| {
                stage
                    .foundation_operation_identity(
                        forged_registration.clone(),
                        resources.clone(),
                        target.clone(),
                    )
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert!(validate_xmap16_operation_rows(
            &forged_registration,
            &resources,
            &forged_operations,
            stages,
        )
        .is_err());

        let mut wrong_target = operations;
        wrong_target[2].target_sha256 = "e".repeat(64);
        assert!(
            validate_xmap16_operation_rows(&registration, &resources, &wrong_target, stages,)
                .is_err()
        );
    }

    #[test]
    fn packet_stage_lookup_requires_the_authenticated_schedule() {
        let topology = product_foundation_assigned_stage_topology().unwrap();
        let schedule =
            portable_render_stream::AssignedStageSchedule::from_topology(&topology).unwrap();
        let mut adapters = ProductionMapFeatureAdapters::default();
        assert!(adapters.assigned_stage_schedule.is_none());
        adapters
            .install_assigned_stage_schedule(&topology, schedule)
            .unwrap();
        assert_eq!(
            adapters
                .assigned_stage_schedule
                .as_ref()
                .unwrap()
                .require(
                    crate::production_features::xmap06::XMAP06_LATE_STAGE_NAME,
                    crate::production_features::xmap06::XMAP06_LATE_OPERATION_NAME,
                )
                .unwrap()
                .assigned_ordinal,
            7
        );

        let original_schedule = adapters.assigned_stage_schedule.clone().unwrap();
        let other_topology = portable_render_stream::AssignedStageTopology::seal(
            vec![portable_render_stream::AssignedStageIdentity::new(
                "water_surface",
                "water.surface.draw",
            )],
            Vec::new(),
        )
        .unwrap();
        let other_schedule =
            portable_render_stream::AssignedStageSchedule::from_topology(&other_topology).unwrap();
        assert!(adapters
            .install_assigned_stage_schedule(&other_topology, other_schedule)
            .unwrap_err()
            .to_string()
            .contains("already"));
        assert_eq!(
            adapters.assigned_stage_schedule.as_ref(),
            Some(&original_schedule)
        );
    }

    #[test]
    fn all_authority_gap_ids_have_one_feature_owner() {
        for gap in crate::MAP_AUTHORITY_REQUIRED_INPUTS {
            assert!(
                owner_packet_for_authority_gap(gap).is_some(),
                "unclassified {gap}"
            );
        }
    }

    #[test]
    fn resource_and_raid_authority_remains_xmap21_owned() {
        assert_eq!(
            owner_packet_for_authority_gap("mapmode.raids"),
            Some("XMAP21")
        );
        assert_eq!(
            owner_packet_for_authority_gap("mapmode.resource_deposits"),
            Some("XMAP21")
        );
    }

    #[test]
    fn promoted_registry_contains_camera_and_terrain_only() {
        struct FixtureTerrainAdapter;
        impl ProductionMapFeatureAdapter for FixtureTerrainAdapter {
            fn family(&self) -> MapVisualFamily {
                MapVisualFamily::Terrain
            }

            fn prepare(
                &self,
                _context: &FoundationFeatureContext<'_>,
            ) -> Result<FoundationFeatureOutput> {
                unreachable!("registry-shape fixture is never prepared")
            }
        }
        let mut adapters = ProductionMapFeatureAdapters::default();
        adapters
            .register(Box::new(
                crate::production_features::xmap03::Xmap03CameraAdapter,
            ))
            .unwrap();
        adapters.register(Box::new(FixtureTerrainAdapter)).unwrap();
        assert!(adapters.get(MapVisualFamily::CameraView).is_some());
        assert!(adapters.get(MapVisualFamily::Terrain).is_some());
        for family in MapVisualFamily::ALL {
            if !matches!(
                family,
                MapVisualFamily::CameraView | MapVisualFamily::Terrain
            ) {
                assert!(adapters.get(family).is_none(), "{family:?} promoted early");
            }
        }
    }

    #[test]
    fn stale_state_never_relabels_the_unavailable_hour_as_fresh() {
        let family_rows = |hour, unavailable_family: Option<MapVisualFamily>| {
            MapVisualFamily::ALL
                .into_iter()
                .map(|family| FamilyCoverageRow {
                    family,
                    owner_packet: family.owner_packet().into(),
                    disposition: if unavailable_family == Some(family) {
                        FamilyDisposition::Unavailable {
                            gap: unimplemented_adapter_gap(family, hour),
                        }
                    } else {
                        FamilyDisposition::NotApplicable {
                            applicability_witness_sha256: "a".repeat(64),
                            reason: "fixture".into(),
                        }
                    },
                })
                .collect()
        };
        let frame = |hour, unavailable_family| FoundationRenderEnvelope {
            schema: FOUNDATION_RENDER_SCHEMA.into(),
            hour_index: hour,
            content_manifest_sha256: "b".repeat(64),
            product_modset_sha256: "c".repeat(64),
            effective_vfs_manifest_sha256: "d".repeat(64),
            foundation_content_operation_sha256: "e".repeat(64),
            families: family_rows(hour, unavailable_family),
        };
        let mut state = FoundationPlaybackState::default();
        assert!(matches!(
            state.admit(frame(10, None)).unwrap(),
            FoundationPlaybackOutcome::Fresh(_)
        ));
        let outcome = state
            .admit(frame(11, Some(MapVisualFamily::Terrain)))
            .unwrap();
        match outcome {
            FoundationPlaybackOutcome::Stale {
                requested_hour,
                rendered_hour,
                frame,
                ..
            } => {
                assert_eq!(requested_hour, 11);
                assert_eq!(rendered_hour, 10);
                assert_eq!(frame.hour_index, 10);
            }
            other => panic!("expected stale, got {other:?}"),
        }
    }

    #[test]
    fn product_playback_allows_only_current_xmap16_typed_unavailability() {
        let frame = |hour, unavailable_family: MapVisualFamily| FoundationRenderEnvelope {
            schema: FOUNDATION_RENDER_SCHEMA.into(),
            hour_index: hour,
            content_manifest_sha256: "b".repeat(64),
            product_modset_sha256: "c".repeat(64),
            effective_vfs_manifest_sha256: "d".repeat(64),
            foundation_content_operation_sha256: "e".repeat(64),
            families: MapVisualFamily::ALL
                .into_iter()
                .map(|family| FamilyCoverageRow {
                    family,
                    owner_packet: family.owner_packet().into(),
                    disposition: if family == unavailable_family {
                        FamilyDisposition::Unavailable {
                            gap: FoundationGap {
                                gap_id: format!("fixture.{}", family.owner_packet()),
                                first_unavailable_hour: hour,
                                last_unavailable_hour: hour,
                                witness_sha256: "f".repeat(64),
                                reason: if family == MapVisualFamily::RailSupply {
                                    "schema v1/v2 predates state-first XMAP16".into()
                                } else {
                                    "fixture blocking family".into()
                                },
                            },
                        }
                    } else {
                        FamilyDisposition::NotApplicable {
                            applicability_witness_sha256: "a".repeat(64),
                            reason: "fixture".into(),
                        }
                    },
                })
                .collect(),
        };

        let mut state = FoundationPlaybackState::default();
        let legacy = require_fresh_foundation_frame(
            state
                .admit_allowing_xmap16_unavailable(frame(20, MapVisualFamily::RailSupply))
                .unwrap(),
        )
        .expect("schema-v1/v2 XMAP16 gap reaches current product frame");
        assert!(matches!(
            legacy
                .families
                .iter()
                .find(|row| row.family == MapVisualFamily::RailSupply)
                .map(|row| &row.disposition),
            Some(FamilyDisposition::Unavailable { .. })
        ));

        let blocking = state
            .admit_allowing_xmap16_unavailable(frame(21, MapVisualFamily::Terrain))
            .unwrap();
        let error = require_fresh_foundation_frame(blocking)
            .expect_err("non-XMAP16 foundation gaps remain fatal");
        assert_eq!(error.code, "FOUNDATION_MAP_UNAVAILABLE");
        assert_eq!(error.gap_ids, vec!["fixture.XMAP04"]);
    }
}
