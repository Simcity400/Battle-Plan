
use crate::{MapAuthorityAdmittedInput, MapAuthorityGap, Result};
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

#[cfg(windows)]
pub fn execute_foundation_content_preflight(
    resolver: &crate::GameFileResolver,
    admission: &replay_core::ValidatedContentAdmission,
    content_authority_sha256: &str,
) -> Result<FoundationContentExecutionReceipt> {
    validate_sha256(content_authority_sha256, "shipping content authority")?;
    let (map_width, map_height) = foundation_map_dimensions(resolver, admission)?;
    let request =
        resolver.foundation_content_request(admission, map_width, map_height, false, 0)?;
    let resolved = resolver.resolve_foundation_content(&request, admission)?;
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewerOwnedMapState {
    pub sky_enabled: bool,
    pub unit_models_enabled: bool,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub camera: Option<crate::ViewerMapCameraState>,
    pub observer_country_id: Option<String>,
    pub map_mode_id: String,
    pub map_mode_selection_epoch: u64,
    pub selected_object_ids: Vec<String>,
    pub input_revision: u64,
    pub animation_time_f32_bits: u32,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationContentExecutionReceipt {
    pub schema: String,
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
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
    pub map_authority_admitted: &'a [MapAuthorityAdmittedInput],
    assigned_stage_schedule: Option<&'a portable_render_stream::AssignedStageSchedule>,
    pub product_modset_sha256: &'a str,
    pub effective_vfs_manifest_sha256: &'a str,
    pub foundation_content_operation_sha256: &'a str,
    pub viewer_state_sha256: &'a str,
    pub prepared_terrain: Option<&'a crate::PreparedTerrainFrame>,
    pub prepared_political: Option<&'a crate::PreparedPoliticalFrame>,
    pub prepared_units: Option<&'a crate::PreparedXmap13Frame>,
    pub prepared_weather: Option<&'a crate::PreparedXmap19WeatherFrame>,
    pub weather_particles_bound: bool,
    /// The map objects XMAP12 placed earlier in this same frame; `None` when
    /// XMAP12 did not vouch this hour, which is not a measured-empty placement.
    pub prepared_map_objects: Option<&'a crate::PreparedXmap12RenderFrame>,
    pub xmap15_units:
        Option<&'a dyn crate::production_features::xmap15::Xmap15UnitIdentityProvider>,
    pub xmap13_motion: Option<&'a crate::Xmap13MotionSample>,
    pub snow_mud_phase: Option<&'a crate::xmap07_snow_mud_phase::Xmap07SnowMudPhaseSample>,
    pub xmap15_selection: &'a crate::production_features::xmap15::Xmap15ViewerSelection,
}

#[cfg(test)]
impl<'a> FoundationFeatureContext<'a> {
    pub(crate) fn with_test_schedule(mut self, schedule: &'a portable_render_stream::AssignedStageSchedule) -> Self {
        self.assigned_stage_schedule = Some(schedule);
        self
    }

    /// Test-only constructor for driving a feature adapter's owned-input
    /// accounting through its real `prepare()` entry: the private schedule
    /// field stays `None` and every hash is blank, which is exactly the shape
    /// where only the accounting and selection stages are reachable.
    pub(crate) fn new_for_owned_accounting_tests(
        world_frame: &'a WorldFrame,
        viewer: &'a ViewerOwnedMapState,
        map_authority_gaps: &'a [MapAuthorityGap],
        map_authority_admitted: &'a [MapAuthorityAdmittedInput],
        xmap15_selection: &'a crate::production_features::xmap15::Xmap15ViewerSelection,
    ) -> Self {
        Self {
            world_frame,
            viewer,
            map_authority_gaps,
            map_authority_admitted,
            assigned_stage_schedule: None,
            product_modset_sha256: "",
            effective_vfs_manifest_sha256: "",
            foundation_content_operation_sha256: "",
            viewer_state_sha256: "",
            prepared_terrain: None,
            prepared_political: None,
            prepared_units: None,
            prepared_map_objects: None,
            prepared_weather: None,
            weather_particles_bound: false,
            xmap15_units: None,
            xmap15_selection,
            xmap13_motion: None,
            snow_mud_phase: None,
        }
    }
}

impl FoundationFeatureContext<'_> {
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
    pub prepared_terrain: Option<crate::PreparedTerrainFrame>,
    pub prepared_political: Option<crate::PreparedPoliticalFrame>,
    pub prepared_thin_borders: Option<crate::PreparedThinBorderFrame>,
    pub prepared_lighting: Option<crate::PreparedXmap09LightingFrame>,
    pub prepared_weather: Option<crate::PreparedXmap19WeatherFrame>,
    pub prepared_rivers_straits: Option<crate::PreparedRiversStraitsFrame>,
    pub prepared_water: Option<crate::PreparedWaterFrame>,
    pub prepared_units: Option<crate::PreparedXmap13Frame>,
    pub prepared_combat: Option<crate::PreparedXmap14Frame>,
    pub prepared_orders: Option<crate::production_features::xmap15::PreparedXmap15Orders>,
    pub prepared_rail_supply: Option<crate::PreparedXmap16Frame>,
    pub prepared_map_modes: Option<crate::PreparedMapModeFrame>,
    /// Per-element doubts this family raised while preparing, at element grain.
    ///
    /// This is the additive counterpart of `consumed_gap_ids`. A gap says the RECORDING
    /// declared a hole; these rows say this producer could not vouch one element of an
    /// otherwise complete layer, and they let it exclude that element instead of refusing the
    /// whole family. A family that has nothing to except leaves the list empty, which is
    /// exactly the "absent rows = fully vouched" reading the envelope already has.
    ///
    /// Two producers deliberately do not use this field. XMAP12's rows travel on its prepared
    /// frame, because they are validated against the placement population that frame carries.
    /// XMAP06's standing exceptions are derived from its prepared partition in
    /// [`collect_element_diagnostics`], because they are a function of what that partition
    /// classified rather than something the adapter decides.
    pub element_diagnostics: Vec<portable_render_stream::ElementDiagnostic>,
    pub consumed_gap_ids: Vec<String>,
}

impl FoundationFeatureOutput {
    /// A family that carries no executable payload of its own -- it is either
    /// refusing, not applicable, or waiting on draw time. Adapters that do
    /// produce a payload fill the matching field in afterwards.
    pub fn without_payload(disposition: FamilyDisposition, consumed_gap_ids: Vec<String>) -> Self {
        Self {
            disposition,
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_lighting: None,
            prepared_weather: None,
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_rail_supply: None,
            prepared_map_modes: None,
            element_diagnostics: Vec::new(),
            consumed_gap_ids,
        }
    }
}

pub trait PreparedEntityManagerOwner {
    fn manager(&self) -> std::sync::Arc<std::sync::Mutex<crate::particle_runtime::ViewerEntityManager>>;
    fn owner(&self) -> crate::particle_runtime::EntityManagerOwner;
    fn frame(&self) -> crate::particle_runtime::EntityManagerFrame;
    fn advance_root(&mut self, root: &str, scene: u32,
        random: &mut crate::particle_runtime::ViewerParticleSeed) -> Result<()>;
    fn finish(&mut self) -> Result<()>;
    fn refuse(&mut self, reason: &str);
}

pub(crate) fn dispatch_prepared_entity_owners(owners: &mut [Box<dyn PreparedEntityManagerOwner + '_>]) -> Result<()> {
    use crate::particle_runtime::EntityManagerDispatch;
    let Some(first) = owners.first() else { return Ok(()); };
    let manager = first.manager();
    let frame = first.frame();
    let outcome: Result<()> = (|| {
        if owners.iter().any(|owner| !std::sync::Arc::ptr_eq(&manager, &owner.manager())
            || owner.frame() != frame) {
            return Err("entity_manager.prepared_owners.disagree".into());
        }
        let identities: Vec<_> = owners.iter().map(|owner| owner.owner()).collect();
        {
            let mut callbacks: Vec<_> = owners.iter_mut().map(|owner|
                move |root: &str, scene: u32, random: &mut crate::particle_runtime::ViewerParticleSeed|
                    owner.advance_root(root, scene, random)).collect();
            let mut dispatch: Vec<_> = identities.iter().zip(callbacks.iter_mut()).map(|(owner, callback)|
                EntityManagerDispatch { owner: *owner, prepared_for: frame, advance_root: callback }).collect();
            manager.lock().map_err(|_| "entity_manager.lock_poisoned")?
                .advance_frame(frame, &mut dispatch)?;
        }
        for owner in owners.iter_mut() { owner.finish()?; }
        Ok(())
    })();
    if let Err(error) = &outcome { refuse_prepared_entity_owners(owners, &error.to_string()); }
    outcome
}

fn refuse_prepared_entity_owners(owners: &mut [Box<dyn PreparedEntityManagerOwner + '_>], reason: &str) {
    // Finalization can fail after another owner materialized; revoke every publication.
    for owner in owners.iter_mut() {
        if let Ok(mut manager) = owner.manager().lock() {
            manager.refuse_frame(owner.frame(), crate::particle_runtime::EntityManagerFrameRefusal::Finalization(reason.into()));
        }
        owner.refuse(reason);
    }
}

pub trait ProductionMapFeatureAdapter {
    fn family(&self) -> MapVisualFamily;
    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput>;
    fn prepare_entity_manager_population(&self, _context: &FoundationFeatureContext<'_>) -> Result<()> {
        Ok(())
    }
    // Taking an already prepared owner must not construct roots or consume the manager cursor.
    fn take_prepared_entity_manager_owner(&self) -> Result<Option<Box<dyn PreparedEntityManagerOwner + '_>>> {
        Ok(None)
    }
    fn weather_particles_bound(&self, _time: f32, _identity: &str) -> Result<bool> {
        Ok(false)
    }
    fn visit_particles_for_draw(
        &self,
        _presentation_time: f32,
        _camera: crate::particle_runtime::ParticleSortCamera,
        _visit: &mut dyn FnMut(&str, &crate::particle_authored::ParticleDrawCapacity, crate::particle_runtime::ParticleGpuView<'_>) -> Result<()>,
    ) -> Result<()> {
        Err("PARTICLE_DRAW retained authored runtime provider unavailable".into())
    }
}

/// The one error an adapter may raise that is an ANSWER rather than a defect:
/// this family cannot be vouched from the admitted content, the recorded hour,
/// or an upstream family that itself refused. It is recorded as
/// [`FamilyDisposition::Unavailable`] and the remaining families are still
/// prepared; `seal_readiness` is what refuses the frame.
///
/// Every other error out of an adapter is a contract violation and still aborts
/// the preparation. Raising this type for one would file a defect as a content
/// outcome.
#[derive(Debug)]
pub struct FamilyContentRefusal {
    gap_id: String,
    reason: String,
}

impl FamilyContentRefusal {
    /// `gap_id` is owner-scoped (`"XMAP17.country_name_runtime_variable"`) for
    /// the same reason a draw contract is: an unscoped id would let one
    /// family's refusal be read as another's.
    pub fn new(gap_id: impl Into<String>, reason: impl Into<String>) -> Box<dyn std::error::Error> {
        Box::new(Self {
            gap_id: gap_id.into(),
            reason: reason.into(),
        })
    }

    fn into_gap(self, family: MapVisualFamily, hour: i64) -> Result<FoundationGap> {
        let owner_prefix = format!("{}.", family.owner_packet());
        if !self.gap_id.starts_with(&owner_prefix)
            || self.gap_id.len() <= owner_prefix.len()
            || self.gap_id.trim() != self.gap_id
            || self.reason.trim().is_empty()
        {
            return Err(format!(
                "{family:?} raised a content refusal whose gap id {:?} is not scoped to its owner packet, or whose reason is blank",
                self.gap_id
            )
            .into());
        }
        let witness_sha256 = Sha256::hex(
            format!("{}\n{hour}\n{hour}\n{}", self.gap_id, self.reason).as_bytes(),
        );
        Ok(FoundationGap {
            gap_id: self.gap_id,
            first_unavailable_hour: hour,
            last_unavailable_hour: hour,
            witness_sha256,
            reason: self.reason,
        })
    }
}

impl std::fmt::Display for FamilyContentRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.reason, self.gap_id)
    }
}

impl std::error::Error for FamilyContentRefusal {}

/// One family's preparation, once a content refusal has been separated from a
/// contract violation.
#[derive(Debug)]
enum FamilyPreparation<T> {
    Prepared(T),
    Refused(FoundationGap),
}

fn classify_family_preparation<T>(
    family: MapVisualFamily,
    hour: i64,
    outcome: Result<T>,
) -> Result<FamilyPreparation<T>> {
    match outcome {
        Ok(value) => Ok(FamilyPreparation::Prepared(value)),
        Err(error) => match error.downcast::<FamilyContentRefusal>() {
            Ok(refusal) => Ok(FamilyPreparation::Refused(refusal.into_gap(family, hour)?)),
            Err(error) => Err(error),
        },
    }
}

#[derive(Default)]
pub struct ProductionMapFeatureAdapters<'a> {
    xmap13_motion: Option<crate::Xmap13MotionSample>,
    slots: BTreeMap<MapVisualFamily, Box<dyn ProductionMapFeatureAdapter + 'a>>,
    xmap11: Option<crate::production_features::xmap11::Xmap11TreesAdapter>,
    xmap12: Option<crate::production_features::xmap12::Xmap12CitiesBuildingsMapObjectsAdapter>,
    xmap17: Option<crate::production_features::xmap17::Xmap17LabelAdapter>,
    assigned_stage_schedule: Option<portable_render_stream::AssignedStageSchedule>,
}

impl<'a> ProductionMapFeatureAdapters<'a> {
    fn collect_prepared_entity_owners(&self) -> Result<Vec<Box<dyn PreparedEntityManagerOwner + '_>>> {
        let mut owners = Vec::new();
        let mut failure = None;
        for adapter in self.slots.values() {
            match adapter.take_prepared_entity_manager_owner() {
                Ok(Some(owner)) => owners.push(owner),
                Ok(None) => {},
                Err(error) => { if failure.is_none() { failure = Some(error); } }
            }
        }
        if let Some(error) = failure {
            refuse_prepared_entity_owners(&mut owners, &error.to_string());
            return Err(error);
        }
        Ok(owners)
    }

    fn prepare_lighting_entity_population(&self, context: &FoundationFeatureContext<'_>, preparation: Result<()>) -> Result<()> {
        let preparation = preparation.and_then(|()| match self.get(MapVisualFamily::LightingShadows) {
            Some(lighting) => lighting.prepare_entity_manager_population(context),
            None => Ok(()),
        });
        let mut owners = self.collect_prepared_entity_owners()?;
        if let Err(error) = preparation {
            refuse_prepared_entity_owners(&mut owners, &error.to_string());
            return Err(error);
        }
        dispatch_prepared_entity_owners(&mut owners)
    }

    pub fn visit_particles_for_draw(
        &self,
        presentation_time: f32,
        camera: crate::particle_runtime::ParticleSortCamera,
        visit: &mut dyn FnMut(&str, &crate::particle_authored::ParticleDrawCapacity, crate::particle_runtime::ParticleGpuView<'_>) -> Result<()>,
    ) -> Result<()> {
        self.get(MapVisualFamily::LightingShadows)
            .ok_or("PARTICLE_DRAW lighting runtime provider unavailable")?
            .visit_particles_for_draw(presentation_time, camera, visit)
    }

    pub fn set_xmap13_motion(&mut self, sample: Option<crate::Xmap13MotionSample>) {
        self.xmap13_motion = sample;
    }

    pub fn register(&mut self, adapter: Box<dyn ProductionMapFeatureAdapter + 'a>) -> Result<()> {
        let family = adapter.family();
        if self.already_registered(family) {
            return Err(format!("duplicate production adapter for {family:?}").into());
        }
        self.slots.insert(family, adapter);
        Ok(())
    }

    fn get(&self, family: MapVisualFamily) -> Option<&dyn ProductionMapFeatureAdapter> {
        self.slots.get(&family).map(|adapter| adapter.as_ref())
    }

    fn register_xmap12(
        &mut self,
        adapter: crate::production_features::xmap12::Xmap12CitiesBuildingsMapObjectsAdapter,
    ) -> Result<()> {
        if self.already_registered(MapVisualFamily::CitiesBuildingsMapObjects) {
            return Err("duplicate production adapter for CitiesBuildingsMapObjects".into());
        }
        self.xmap12 = Some(adapter);
        Ok(())
    }

    fn register_xmap11(
        &mut self,
        adapter: crate::production_features::xmap11::Xmap11TreesAdapter,
    ) -> Result<()> {
        if self.already_registered(MapVisualFamily::Trees) {
            return Err("duplicate production adapter for Trees".into());
        }
        self.xmap11 = Some(adapter);
        Ok(())
    }

    fn register_xmap17(
        &mut self,
        adapter: crate::production_features::xmap17::Xmap17LabelAdapter,
    ) -> Result<()> {
        if self.already_registered(MapVisualFamily::LabelsMapText) {
            return Err("duplicate production adapter for LabelsMapText".into());
        }
        self.xmap17 = Some(adapter);
        Ok(())
    }

    /// Distinct meshes the XMAP12 adapter has packed and the geometry bytes
    /// resident behind them; `None` when the family has no adapter.
    pub fn xmap12_resident_geometry(&self) -> Option<Result<(usize, usize)>> {
        self.xmap12
            .as_ref()
            .map(|adapter| adapter.resident_geometry())
    }

    /// Registered by EITHER route. Three families are held in a dedicated slot
    /// and the family loop prepares that one instead of a slot adapter, so a
    /// family registered both ways would be prepared twice and one answer
    /// discarded.
    fn already_registered(&self, family: MapVisualFamily) -> bool {
        self.slots.contains_key(&family)
            || match family {
                MapVisualFamily::Trees => self.xmap11.is_some(),
                MapVisualFamily::CitiesBuildingsMapObjects => self.xmap12.is_some(),
                MapVisualFamily::LabelsMapText => self.xmap17.is_some(),
                _ => false,
            }
    }

    /// Whether the family's OWN packet adapter is registered - the one that can
    /// produce its executable payload, which a slot adapter for a dedicated
    /// family is not.
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

    pub fn prepare_with_xmap12_inputs(
        &self,
        world_frame: &WorldFrame,
        viewer: &ViewerOwnedMapState,
        manifest: &ContentManifest,
        content_receipt: &FoundationContentExecutionReceipt,
        previous_building_visual: Option<&crate::Xmap12BuildingVisualFrame>,
        previous_fixed_runtime: Option<&crate::Xmap12FixedRuntimeState>,
        snow_mud_phase: Option<&crate::xmap07_snow_mud_phase::Xmap07SnowMudPhaseSample>,
    ) -> Result<PreparedFoundationFrame> {
        let selection = crate::production_features::xmap15::Xmap15ViewerSelection::default();
        prepare_foundation_frame_inner(
            world_frame,
            viewer,
            manifest,
            content_receipt,
            self,
            previous_building_visual,
            previous_fixed_runtime,
            &selection,
        snow_mud_phase,
        )
    }

    pub fn prepare_with_xmap12_and_xmap15_inputs(
        &self,
        world_frame: &WorldFrame,
        viewer: &ViewerOwnedMapState,
        manifest: &ContentManifest,
        content_receipt: &FoundationContentExecutionReceipt,
        previous_building_visual: Option<&crate::Xmap12BuildingVisualFrame>,
        previous_fixed_runtime: Option<&crate::Xmap12FixedRuntimeState>,
        selection: &crate::production_features::xmap15::Xmap15ViewerSelection,
        snow_mud_phase: Option<&crate::xmap07_snow_mud_phase::Xmap07SnowMudPhaseSample>,
    ) -> Result<PreparedFoundationFrame> {
        prepare_foundation_frame_inner(
            world_frame,
            viewer,
            manifest,
            content_receipt,
            self,
            previous_building_visual,
            previous_fixed_runtime,
            selection,
            snow_mud_phase,
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

fn foundation_feature_adapter_core(
    resolver: &crate::GameFileResolver,
    admission: &replay_core::ValidatedContentAdmission,
    terrain_options: crate::TerrainLodCullOptions,
) -> Result<ProductionMapFeatureAdapters<'static>> {
    let mut adapters = ProductionMapFeatureAdapters::default();
    let entity_manager = std::sync::Arc::new(std::sync::Mutex::new(
        crate::particle_runtime::ViewerEntityManager::default(),
    ));
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
        crate::production_features::xmap09::Xmap09LightingAdapter::with_entity_manager(
            resolver, entity_manager.clone(),
        )?,
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
        crate::production_features::xmap13::Xmap13Adapter::with_entity_manager(
            resolver, admission, entity_manager,
        )?,
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
    // The last three families all execute at draw time and already produce a
    // draw receipt, so registering them makes the readiness graph report the
    // REAL state instead of "the packet never promoted an adapter".
    adapters.register(Box::new(
        crate::production_features::xmap10::Xmap10GradingPostFxAdapter,
    ))?;
    adapters.register(Box::new(
        crate::production_features::xmap20::Xmap20EffectsAdapter,
    ))?;
    adapters.register(Box::new(
        crate::production_features::xmap21::Xmap21MapFurnitureAdapter,
    ))?;
    Ok(adapters)
}

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
            AssignedStageEdge::new(border_early, rivers.clone()),
            rivers_before_straits,
            AssignedStageEdge::new(rivers, rail_execute.clone()),
            AssignedStageEdge::new(rail_execute, trees.clone()),
            AssignedStageEdge::new(trees, water_surface.clone()),
            AssignedStageEdge::new(water_surface.clone(), border_late.clone()),
            AssignedStageEdge::new(border_late, straits.clone()),
            AssignedStageEdge::new(straits, supply_arrow_execute.clone()),
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
    // EXCLUDE AND NAME, never a blank frame (owner ruling): if this install's
    // authored content cannot build the combat-indicator layer, the layer is
    // registered as typed-unavailable with the refusal carried verbatim, and
    // the rest of the map still draws. Propagating that refusal here destroyed
    // the whole adapter graph, so one vanilla sprite that omits an optional key
    // cost the entire picture -- measured on t83, which reached D3D11 and the
    // localisation corpus and then produced no pixel at all.
    adapters.register(Box::new(
        crate::production_features::xmap14::Xmap14Adapter::new_or_unavailable(
            resolver,
            admission,
            localisation_language,
        ),
    ))?;
    adapters.register_xmap17(crate::production_features::xmap17::Xmap17LabelAdapter::new(
        resolver,
        admission,
        localisation_language,
    )?)?;
    Ok(adapters)
}

/// What one family's preparation cost this hour: its wall time and the
/// process's committed private bytes once it had answered. The footprint is
/// measured per family so a frame's peak is attributed rather than inferred
/// from where a dump died.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyPreparationCost {
    pub family: MapVisualFamily,
    pub wall_ms: u64,
    pub private_bytes_after: u64,
    pub peak_private_bytes_after: u64,
}

/// This process's committed private bytes now and at its peak
/// (`PagefileUsage` / `PeakPagefileUsage` of `PROCESS_MEMORY_COUNTERS`).
#[cfg(windows)]
pub fn process_private_bytes() -> Option<(u64, u64)> {
    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(
            process: isize,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }
    let mut counters = ProcessMemoryCounters {
        cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
        page_fault_count: 0,
        peak_working_set_size: 0,
        working_set_size: 0,
        quota_peak_paged_pool_usage: 0,
        quota_paged_pool_usage: 0,
        quota_peak_non_paged_pool_usage: 0,
        quota_non_paged_pool_usage: 0,
        pagefile_usage: 0,
        peak_pagefile_usage: 0,
    };
    // SAFETY: the counters struct is the documented PROCESS_MEMORY_COUNTERS
    // layout and `cb` carries its size; the pseudo-handle needs no release.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    (ok != 0).then(|| {
        (
            counters.pagefile_usage as u64,
            counters.peak_pagefile_usage as u64,
        )
    })
}

#[cfg(not(windows))]
pub fn process_private_bytes() -> Option<(u64, u64)> {
    None
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedFoundationFrame {
    pub envelope: FoundationRenderEnvelope,
    pub terrain: crate::PreparedTerrainFrame,
    pub political: crate::PreparedPoliticalFrame,
    pub thin_borders: Option<crate::PreparedThinBorderFrame>,
    pub lighting: Option<crate::PreparedXmap09LightingFrame>,
    pub trees: Option<crate::PreparedTreeFrame>,
    pub xmap12: Option<crate::PreparedXmap12RenderFrame>,
    /// This frame's derived per-placement building presentation. Feed it back as the next
    /// frame's `previous_building_visual`: it is the destroyed-mesh selector's previous-frame
    /// memory, the engine's `slot+0x3a`.
    pub xmap12_building_visual: Option<crate::Xmap12BuildingVisualFrame>,
    /// This frame's derived ambient/fixed lifecycle. The caller carries it into
    /// the next frame the way it carries the building visual: an ambient object
    /// the viewer's camera has spawned stays spawned.
    pub xmap12_fixed_runtime: Option<crate::Xmap12FixedRuntimeState>,
    pub weather: Option<crate::PreparedXmap19WeatherFrame>,
    pub rivers_straits: Option<crate::PreparedRiversStraitsFrame>,
    pub water: crate::PreparedWaterFrame,
    pub units: Option<crate::PreparedXmap13Frame>,
    pub xmap13_motion: Option<crate::Xmap13MotionSample>,
    pub combat: Option<crate::PreparedXmap14Frame>,
    pub orders: Option<crate::production_features::xmap15::PreparedXmap15Frame>,
    pub order_inputs: Option<crate::Xmap15PlanProvider>,
    pub rail_supply: Option<crate::PreparedXmap16Frame>,
    pub map_modes: crate::PreparedMapModeFrame,
    pub labels: Option<crate::PreparedMapLabelFrame>,
    /// One row per family in `MapVisualFamily::ALL` order, present or not.
    pub family_costs: Vec<FamilyPreparationCost>,
    /// Every localisation miss the map families named this hour (XMAP17
    /// labels drawn as their raw key, XMAP14 progress text); the app logs them.
    pub localisation_misses: replay_core::LocalisationMisses,
}

/// How much the XMAP12 payload carries this hour: the counts a footprint row
/// is divided by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Xmap12PayloadSummary {
    pub draws: usize,
    pub l03_rows: usize,
    pub element_diagnostics: usize,
    pub building_rows: usize,
}

impl PreparedFoundationFrame {
    /// `None` when the family did not compose this hour.
    pub fn xmap12_payload_summary(&self) -> Option<Xmap12PayloadSummary> {
        let frame = self.xmap12.as_ref()?;
        Some(Xmap12PayloadSummary {
            draws: frame.ordered_draws.len(),
            l03_rows: frame.ordered_l03_rows.len(),
            element_diagnostics: frame.element_diagnostics.len(),
            building_rows: self
                .xmap12_building_visual
                .as_ref()
                .map_or(0, |visual| visual.rows.len()),
        })
    }
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

pub fn prepare_foundation_frame(
    world_frame: &WorldFrame,
    viewer: &ViewerOwnedMapState,
    manifest: &ContentManifest,
    content_receipt: &FoundationContentExecutionReceipt,
    adapters: &ProductionMapFeatureAdapters<'_>,
    snow_mud_phase: Option<&crate::xmap07_snow_mud_phase::Xmap07SnowMudPhaseSample>,
) -> Result<PreparedFoundationFrame> {
    let selection = crate::production_features::xmap15::Xmap15ViewerSelection::default();
    prepare_foundation_frame_inner(
        world_frame,
        viewer,
        manifest,
        content_receipt,
        adapters,
        None,
        None,
        &selection,
        snow_mud_phase,
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
        FamilyDisposition::Unavailable { .. }
        | FamilyDisposition::NotApplicable { .. }
        | FamilyDisposition::DeferredToDraw { .. } => {
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
    content_receipt: &FoundationContentExecutionReceipt,
    adapters: &ProductionMapFeatureAdapters<'_>,
    xmap12_previous_building_visual: Option<&crate::Xmap12BuildingVisualFrame>,
    xmap12_previous_fixed_runtime: Option<&crate::Xmap12FixedRuntimeState>,
    xmap15_selection: &crate::production_features::xmap15::Xmap15ViewerSelection,
    snow_mud_phase: Option<&crate::xmap07_snow_mud_phase::Xmap07SnowMudPhaseSample>,
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
    // The map authority is derived HERE, from the frame and manifest this
    // preparation was handed and from nothing a caller could hand it beside
    // them. It used to arrive as a parameter, which let an admission derived
    // from another frame reach the family loop - a gap filed for a row this
    // frame measures, or an admitted row this frame does not carry - and a
    // family gating on the frame (XMAP12's lifecycle token, XMAP19's weather
    // token) would then disagree with the gaps it was told to consume. The
    // content receipt above is bound to the manifest by comparison; this is
    // bound to the frame by construction.
    let map_authority_projection =
        crate::map_authority::map_authority_projection_from_world_frame(world_frame, manifest)?;
    let map_admission =
        crate::map_authority::admit_map_authority_projection(&map_authority_projection)?;
    let viewer_state_sha256 = Sha256::hex(&serde_json::to_vec(viewer)?);

    let mut consumed = BTreeSet::new();
    let mut localisation_misses = replay_core::LocalisationMisses::default();
    let mut adapter_element_diagnostics: Vec<portable_render_stream::ElementDiagnostic> =
        Vec::new();
    let mut families = Vec::with_capacity(MapVisualFamily::ALL.len());
    let mut family_costs = Vec::with_capacity(MapVisualFamily::ALL.len());
    let mut prepared_terrain = None;
    let mut prepared_political = None;
    let mut prepared_thin_borders = None;
    let mut prepared_rivers_straits = None;
    let mut prepared_lighting = None;
    let mut prepared_trees = None;
    let mut prepared_xmap12 = None;
    let mut prepared_xmap12_building_visual = None;
    let mut prepared_xmap12_fixed_runtime = None;
    let mut prepared_weather: Option<crate::PreparedXmap19WeatherFrame> = None;
    let mut prepared_water = None;
    let mut prepared_units = None;
    let mut prepared_combat = None;
    let mut prepared_orders = None;
    let mut order_inputs = None;
    let mut prepared_rail_supply = None;
    let mut prepared_labels = None;
    let mut prepared_map_modes = None;
    let mut early_weather: Option<FamilyPreparation<FoundationFeatureOutput>> = None;
    let mut entity_preparation_refusal: Option<String> = None;
    let mut entity_preparation_cost = [std::time::Duration::ZERO; 2];
    for family in MapVisualFamily::ALL {
        let steps = match family {
            MapVisualFamily::UnitsCounters => [Some((family, false)), None, None],
            MapVisualFamily::LightingShadows => [Some((family, false)),
                Some((MapVisualFamily::UnitsCounters, true)), Some((family, true))],
            _ => [Some((family, true)), None, None],
        };
        for (family, publish) in steps.into_iter().flatten() {
        let family_started = std::time::Instant::now();
        let weather_preparation = if family == MapVisualFamily::WeatherAtmosphere {
            early_weather.take()
        } else {
            None
        };
        let mut context = FoundationFeatureContext {
            world_frame,
            viewer,
            map_authority_gaps: map_admission.gaps(),
            map_authority_admitted: map_admission.admitted(),
            assigned_stage_schedule: adapters.assigned_stage_schedule.as_ref(),
            product_modset_sha256: &manifest.product_modset_sha256,
            effective_vfs_manifest_sha256: &manifest.effective_vfs_manifest_sha256,
            foundation_content_operation_sha256: &content_receipt.deterministic_sha256,
            viewer_state_sha256: &viewer_state_sha256,
            prepared_terrain: prepared_terrain.as_ref(),
            prepared_political: prepared_political.as_ref(),
            prepared_units: prepared_units.as_ref(),
            prepared_map_objects: prepared_xmap12.as_ref(),
            prepared_weather: prepared_weather.as_ref(),
            weather_particles_bound: false,
            xmap15_units: prepared_units.as_ref().map(|frame| {
                &frame.identity_provider
                    as &dyn crate::production_features::xmap15::Xmap15UnitIdentityProvider
            }),
            xmap15_selection,
            xmap13_motion: adapters.xmap13_motion.as_ref(),
            snow_mud_phase,
        };
        // A family that cannot vouch is recorded and the loop goes on; only a
        // contract violation stops the preparation here.
        let mut content_refusal: Option<FoundationGap> = None;
        let hour = world_frame.hour_index();
        // A local macro rather than a closure: the four call sites prepare four
        // different payload types, which one monomorphic closure cannot take.
        macro_rules! prepared_or_refusal {
            ($outcome:expr) => {
                match classify_family_preparation(family, hour, $outcome)? {
                    FamilyPreparation::Prepared(value) => Some(value),
                    FamilyPreparation::Refused(gap) => {
                        content_refusal = Some(gap);
                        None
                    }
                }
            };
        }
        if !publish && family == MapVisualFamily::UnitsCounters {
            if let Some(adapter) = adapters.get(family) {
                if let Err(error) = adapter.prepare_entity_manager_population(&context) {
                    entity_preparation_refusal = Some(error.to_string());
                }
            }
            entity_preparation_cost[0] = family_started.elapsed();
            continue;
        }
        if publish && matches!(family, MapVisualFamily::UnitsCounters | MapVisualFamily::LightingShadows) {
            if let Some(reason) = &entity_preparation_refusal {
                prepared_or_refusal!(Err::<(), _>(FamilyContentRefusal::new(
                    format!("{}.entity_manager_frame_unavailable", family.owner_packet()), reason.clone())));
            }
        }
        let xmap12_preparation = if family == MapVisualFamily::CitiesBuildingsMapObjects {
            match adapters.xmap12.as_ref() {
                Some(adapter) => prepared_or_refusal!(adapter.prepare(
                    &context,
                    xmap12_previous_building_visual,
                    xmap12_previous_fixed_runtime,
                )),
                None => None,
            }
        } else {
            None
        };
        let xmap11_preparation = if family == MapVisualFamily::Trees {
            match adapters.xmap11.as_ref() {
                Some(adapter) => prepared_or_refusal!(adapter.prepare(&context)),
                None => None,
            }
        } else {
            None
        };
        let xmap17_preparation = if family == MapVisualFamily::LabelsMapText {
            match adapters.xmap17.as_ref() {
                Some(adapter) => prepared_or_refusal!(adapter.prepare(&context)),
                None => None,
            }
        } else {
            None
        };
        // Weather setters precede entity events; all owners publish after common dispatch.
        if !publish && family == MapVisualFamily::LightingShadows {
            let preparation: Result<()> = (|| {
                if let Some(reason) = &entity_preparation_refusal {
                    return Err(reason.clone().into());
                }
                if let Some(adapter) = adapters.get(MapVisualFamily::WeatherAtmosphere) {
                    early_weather = Some(classify_family_preparation(
                        MapVisualFamily::WeatherAtmosphere, hour, adapter.prepare(&context))?);
                }
                Ok(())
            })();
            context.prepared_weather = match early_weather.as_ref() {
                Some(FamilyPreparation::Prepared(output)) => output.prepared_weather.as_ref(),
                _ => None,
            };
            if let Err(error) = adapters.prepare_lighting_entity_population(&context, preparation) {
                entity_preparation_refusal = Some(error.to_string());
            }
            entity_preparation_cost[1] = family_started.elapsed();
            continue;
        }
        if publish && matches!(family, MapVisualFamily::UnitsCounters | MapVisualFamily::LightingShadows) {
            context.prepared_weather = match early_weather.as_ref() {
                Some(FamilyPreparation::Prepared(output)) => output.prepared_weather.as_ref(),
                _ => None,
            };
        }
        if family == MapVisualFamily::ParticlesEffects {
            if let (Some(weather), Some(lighting)) =
                (prepared_weather.as_ref(), adapters.get(MapVisualFamily::LightingShadows))
            {
                context.weather_particles_bound = prepared_or_refusal!(
                    lighting.weather_particles_bound(
                        f32::from_bits(viewer.animation_time_f32_bits),
                        &weather.deterministic_sha256,
                    ).map_err(|error| FamilyContentRefusal::new(
                        "XMAP20.weather_particle_binding_unavailable", error.to_string(),
                    ))
                ).unwrap_or(false);
            }
        }
        let generic_preparation = if content_refusal.is_some() {
            None
        } else {
    match weather_preparation {
                Some(FamilyPreparation::Prepared(output)) => Some(output),
                Some(FamilyPreparation::Refused(gap)) => {
                    content_refusal = Some(gap);
                    None
                }
                None => match adapters.get(family) {
                    Some(adapter) => prepared_or_refusal!(adapter.prepare(&context)),
                    None => None,
                },
            }
        };
        let disposition = if let Some(gap) = content_refusal {
            FamilyDisposition::Unavailable { gap }
        } else if let Some(xmap11) = xmap11_preparation {
            if prepared_trees.replace(xmap11.prepared).is_some() {
                return Err("foundation produced duplicate XMAP11 tree payload".into());
            }
            let output = xmap11.output;
            adapter_element_diagnostics.extend(output.element_diagnostics);
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
            localisation_misses.absorb(xmap17.localisation_misses);
            if prepared_labels.replace(xmap17.prepared).is_some() {
                return Err("foundation produced duplicate XMAP17 label payload".into());
            }
            let output = xmap17.output;
            adapter_element_diagnostics.extend(output.element_diagnostics);
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
            match (xmap12_preparation, generic_preparation) {
                (Some(xmap12), _) => {
                    // A present frame owes its payload AND both carried states,
                    // and an absent one owes none of them. The adapter answers
                    // with one field holding all three, so the only thing left
                    // to check here is that the answer agrees with the
                    // disposition beside it.
                    if !xmap12.disposition_agrees_with_payload() {
                        return Err(
                            "CitiesBuildingsMapObjects disposition and prepared payload disagree"
                                .into(),
                        );
                    }
                    if let Some(prepared) = xmap12.present {
                        // The two inputs this layer HANDS the adapter, checked
                        // against the answer that came back. Both fail silently
                        // when they go missing on the way through, and neither
                        // shows up as an error anywhere else.
                        //
                        // The DLC set is read through the lifecycle row's own
                        // decision, the one the adapter's admission token was
                        // minted from: a present payload on an hour that row
                        // refuses is a defect, not an empty set to compare
                        // against.
                        let supplied_dlc_names =
                            crate::map_authority::ambient_lifecycle_session_dlcs(
                                context.world_frame,
                            )
                            .map_err(|refused| {
                                format!(
                                    "present XMAP12 lifecycle on an hour whose session DLC set is {refused:?}"
                                )
                            })?;
                        crate::xmap12_fixed_runtime_consumed_its_inputs(
                            xmap12_previous_fixed_runtime,
                            &supplied_dlc_names,
                            &prepared.fixed_runtime,
                        )?;
                        prepared_xmap12_building_visual = Some(prepared.building_visual);
                        prepared_xmap12_fixed_runtime = Some(prepared.fixed_runtime);
                        if prepared_xmap12.replace(prepared.prepared).is_some() {
                            return Err("foundation produced duplicate XMAP12 payload".into());
                        }
                    }
                    let output = xmap12.output;
                    adapter_element_diagnostics.extend(output.element_diagnostics);
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
                (None, Some(output)) => {
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
                    // XMAP20 has no foundation-time payload to carry: an effect
                    // packet's operation identity binds live D3D scene state, so
                    // the family either defers under its draw contract or refuses.
                    // Vouching here would be a claim it cannot have checked.
                    if family == MapVisualFamily::ParticlesEffects
                        && matches!(
                            &output.disposition,
                            FamilyDisposition::Present { .. }
                                | FamilyDisposition::AuthenticatedEmpty { .. }
                        )
                    {
                        return Err(
                            "ParticlesEffects vouched at foundation time, where its draw-time \
                             inputs do not exist"
                                .into(),
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
                        orders.plans.validate(world_frame.hour_index())?;
                        if family != MapVisualFamily::OrdersFrontsArrows
                            || order_inputs.replace(orders.plans).is_some()
                        {
                            return Err(
                            "foundation adapter produced duplicate or non-XMAP15 orders payload"
                                .into(),
                        );
                        }
                        prepared_orders = orders.native;
                    }
                    if family == MapVisualFamily::OrdersFrontsArrows
                        && prepared_orders.is_none()
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
                    // XMAP21 has no foundation-time payload either: every
                    // furniture icon is projected through the live camera and
                    // viewport.
                    if family == MapVisualFamily::MapFurnitureIcons
                        && matches!(
                            &output.disposition,
                            FamilyDisposition::Present { .. }
                                | FamilyDisposition::AuthenticatedEmpty { .. }
                        )
                    {
                        return Err(
                            "MapFurnitureIcons vouched at foundation time, where its camera and \
                             viewport do not exist"
                                .into(),
                        );
                    }
                    adapter_element_diagnostics.extend(output.element_diagnostics);
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
        let (private_bytes_after, peak_private_bytes_after) =
            process_private_bytes().unwrap_or((0, 0));
        family_costs.push(FamilyPreparationCost {
            family,
            wall_ms: u64::try_from((family_started.elapsed() + match family {
                MapVisualFamily::UnitsCounters => entity_preparation_cost[0],
                MapVisualFamily::LightingShadows => entity_preparation_cost[1],
                _ => std::time::Duration::ZERO,
            }).as_millis())?,
            private_bytes_after,
            peak_private_bytes_after,
        });
        }
    }

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

    // Per-element diagnostics ride the envelope alongside the family rows: one collection
    // point, so a family that gains an unvouched element adds its list here and needs no new
    // channel. Rows under a family that refused wholesale are dropped - that family already
    // said everything through its gap, and a row underneath it would claim an element that
    // was never composed.
    let element_diagnostics = collect_element_diagnostics(
        prepared_xmap12.as_ref(),
        prepared_thin_borders.as_ref(),
        adapter_element_diagnostics,
        &families,
    )?;
    let envelope = FoundationRenderEnvelope {
        schema: FOUNDATION_RENDER_SCHEMA.into(),
        hour_index: world_frame.hour_index(),
        content_manifest_sha256: manifest_sha256.clone(),
        product_modset_sha256: manifest.product_modset_sha256.clone(),
        effective_vfs_manifest_sha256: manifest.effective_vfs_manifest_sha256.clone(),
        foundation_content_operation_sha256: content_receipt.deterministic_sha256.clone(),
        families,
        element_diagnostics,
    };
    envelope.validate()?;
    let envelope = FoundationRenderEnvelope::decode_json(&envelope.encode_json()?)
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    // These three have no absent form - the rest of the preparation reads their
    // payloads directly - so an absent one names the gap it recorded, not the
    // symptom.
    let terrain = prepared_terrain
        .ok_or_else(|| mandatory_family_absence(&envelope, MapVisualFamily::Terrain))?;
    terrain.validate()?;
    let political = prepared_political.ok_or_else(|| {
        mandatory_family_absence(&envelope, MapVisualFamily::PoliticalFillGradients)
    })?;
    political.validate()?;
    let map_modes = prepared_map_modes
        .ok_or_else(|| mandatory_family_absence(&envelope, MapVisualFamily::MapModes))?;
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
            // XMAP06 derives its border classification in the replay layer
            // (owner ruling 2026-08-21), so it has no unavailable disposition
            // any more: there is no recorded carrier that an hour can be missing.
            FamilyDisposition::Present { payload_sha256, .. }
                if payload_sha256 == &thin_borders.deterministic_sha256 => {}
            _ => {
                return Err(
                    "foundation XMAP06 family row disagrees with its prepared geometry".into(),
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
        let row = envelope
            .families
            .iter()
            .find(|row| row.family == MapVisualFamily::LabelsMapText)
            .ok_or("foundation envelope omitted LabelsMapText")?;
        match (&row.disposition, prepared_labels) {
            // A refusing XMAP17 carries no payload, exactly as XMAP14 and XMAP15
            // already do; the frame does not seal, `seal_readiness` says so.
            (FamilyDisposition::Unavailable { .. }, None) => None,
            (FamilyDisposition::Present { payload_sha256, .. }, Some(labels))
                if payload_sha256 == &labels.deterministic_sha256 =>
            {
                labels.validate()?;
                Some(labels)
            }
            _ => {
                return Err("foundation XMAP17 family row disagrees with executable payload".into())
            }
        }
    } else {
        if prepared_labels.is_some() {
            return Err("foundation produced XMAP17 labels without registering its adapter".into());
        }
        None
    };
    Ok(PreparedFoundationFrame {
        localisation_misses: {
            if let Some(combat) = &prepared_combat {
                localisation_misses.absorb(combat.localisation_misses.clone());
            }
            localisation_misses
        },
        envelope,
        terrain,
        political,
        thin_borders,
        lighting: prepared_lighting,
        trees: prepared_trees,
        xmap12: prepared_xmap12,
        xmap12_building_visual: prepared_xmap12_building_visual,
        xmap12_fixed_runtime: prepared_xmap12_fixed_runtime,
        weather: prepared_weather,
        rivers_straits: prepared_rivers_straits,
        water,
        units: prepared_units,
        xmap13_motion: adapters.xmap13_motion.clone(),
        combat: prepared_combat,
        orders: prepared_orders,
        order_inputs,
        rail_supply: prepared_rail_supply,
        labels,
        map_modes,
        family_costs,
    })
}

/// Why a family the rest of the preparation reads directly has no payload.
fn mandatory_family_absence(
    envelope: &FoundationRenderEnvelope,
    family: MapVisualFamily,
) -> Box<dyn std::error::Error> {
    let owner = family.owner_packet();
    match envelope
        .families
        .iter()
        .find(|row| row.family == family)
        .map(|row| &row.disposition)
    {
        Some(FamilyDisposition::Unavailable { gap }) => format!(
            "{owner} refuses the mandatory {family:?} family, so this frame has no foundation: \
             {} ({})",
            gap.reason, gap.gap_id
        ),
        _ => format!("foundation {owner} adapter produced no executable {family:?} payload"),
    }
    .into()
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

/// Gather every family's per-element diagnostics into the envelope's single list.
///
/// This is the one place the channel is assembled, from three sources: the rows any adapter
/// put on its `FoundationFeatureOutput`, XMAP12's rows on its prepared frame, and XMAP06's
/// standing exceptions derived from its prepared partition. A family that gains an unvouched
/// element adds its list here; nothing else in the foundation changes, and no family is
/// refused because one of its elements is doubtful.
fn collect_element_diagnostics(
    xmap12: Option<&crate::PreparedXmap12RenderFrame>,
    thin_borders: Option<&crate::PreparedThinBorderFrame>,
    adapter_rows: Vec<portable_render_stream::ElementDiagnostic>,
    families: &[FamilyCoverageRow],
) -> Result<Vec<portable_render_stream::ElementDiagnostic>> {
    let mut rows = adapter_rows;
    if let Some(xmap12) = xmap12 {
        rows.extend(xmap12.element_diagnostics.iter().cloned());
    }
    if let Some(thin_borders) = thin_borders {
        rows.extend(crate::production_features::xmap06::xmap06_family_exceptions(
            thin_borders,
        )?);
    }
    rows.retain(|row| {
        families.iter().any(|family_row| {
            family_row.family == row.family
                && !matches!(family_row.disposition, FamilyDisposition::Unavailable { .. })
        })
    });
    portable_render_stream::collapse_element_diagnostics(rows).map_err(Into::into)
}

/// Which packet owns one authority input, straight from the owner table.
///
/// Public so a reader never has to keep a second copy of the packet universe:
/// an input assigned to a packet nobody wrote down still answers here.
pub fn map_authority_owner_packet(input_id: &str) -> Option<&'static str> {
    owner_packet_for_authority_gap(input_id)
}

/// What an authority artifact prints where an owner would go when the table
/// assigns none, so a reader can recognise the case instead of guessing.
pub const MAP_AUTHORITY_OWNER_UNCLASSIFIED: &str = "owner-unclassified";

/// The identity of the WHOLE authority set: every required input paired with the
/// packet that owns it, in the product's own order, as one digest.
///
/// One definition, so the digest an artifact declares and the digest a reader
/// pins are the same fact rather than two hand-kept copies. A reader that
/// recomputes it over the rows it actually parsed cannot be handed a truncated
/// artifact, a repeated row, or a row whose owner was edited, and then write a
/// census over a set nobody measured.
pub fn map_authority_set_identity() -> String {
    let mut preimage = String::new();
    for input in crate::map_authority::MAP_AUTHORITY_REQUIRED_INPUTS {
        preimage.push_str(input);
        preimage.push('\t');
        preimage
            .push_str(map_authority_owner_packet(input).unwrap_or(MAP_AUTHORITY_OWNER_UNCLASSIFIED));
        preimage.push('\n');
    }
    Sha256::hex(preimage.as_bytes())
}

/// The identity of the shipped a729 authority set, pinned.
///
/// The lane's per-family recording-input census carries this same digest and
/// refuses an artifact that is a census of any other set. Growing or re-owning
/// [`crate::map_authority::MAP_AUTHORITY_REQUIRED_INPUTS`] must move this
/// constant, which is what makes that reader's pin honest rather than stale.
pub const MAP_AUTHORITY_SET_IDENTITY_A729: &str =
    "2d8e802a5ae4259bb04fcc6340ab45fc1d380b03e5943d6dce786daec88c7cfd";

/// Every XMAP00 authority input one packet owns, derived from the owner table
/// rather than restated.
///
/// A family that keeps its own list of owned inputs can silently disagree
/// with the table the global accounting uses: XMAP21 listed two of the four
/// inputs the table assigns it and then reported that the recording "admits
/// every XMAP21 authority input" on an hour where `mapmode.resource_deposits`
/// was unavailable.
pub fn map_authority_inputs_owned_by(packet: &str) -> Vec<&'static str> {
    crate::map_authority::MAP_AUTHORITY_REQUIRED_INPUTS
        .iter()
        .copied()
        .filter(|input| owner_packet_for_authority_gap(input) == Some(packet))
        .collect()
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
        || input_id.starts_with("map_furniture.")
    {
        // NOT `mapmode.raids` or `mapmode.resource_deposits`. This table claimed
        // both for XMAP21 while the LEDGER
        // (`MAP_AUTHORITY_OWNER_PROVEN_INPUTS`) records `mapmode.raids` as
        // XMAP18's, and XMAP18's own owned list claims both. XMAP21 read this
        // table for its owned set, so once its adapter stopped being a standing
        // refusal and began consuming its gaps, two families claimed one gap
        // and the whole frame refused:
        //
        //   MapFurnitureIcons claimed an unknown or already-consumed authority
        //   gap mapmode.resource_deposits
        //
        // which is what the r4 dump printed. XMAP21 READS raids and resource
        // deposits to place icons; the AUTHORITY over a `mapmode.*` input is
        // XMAP18's, and reading is not owning.
        // `the_owner_table_and_the_ledger_name_the_same_owner` keeps the two
        // from drifting again.
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
            // The fallback cache wants "no family refuses", NOT "every family
            // has already vouched". A family that defers to draw time has not
            // refused: it is answered per-frame when the frame is actually
            // drawn, and the seal still makes it prove that. Caching on
            // Gating on "already vouched" here would mean a permanently-deferred family kept
            // the cache empty forever and silently turned every later stale
            // frame into an outright refusal.
            if !candidate.has_refusing_family() {
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

    struct FrameOwner {
        manager: std::sync::Arc<std::sync::Mutex<crate::particle_runtime::ViewerEntityManager>>,
        id: crate::particle_runtime::EntityManagerOwner,
        frame: crate::particle_runtime::EntityManagerFrame,
        root: &'static str,
        order: std::rc::Rc<std::cell::RefCell<Vec<(&'static str, u32)>>>,
        published: std::rc::Rc<std::cell::Cell<bool>>,
        fail_finish: bool,
    }
    impl PreparedEntityManagerOwner for FrameOwner {
        fn manager(&self) -> std::sync::Arc<std::sync::Mutex<crate::particle_runtime::ViewerEntityManager>> { self.manager.clone() }
        fn owner(&self) -> crate::particle_runtime::EntityManagerOwner { self.id }
        fn frame(&self) -> crate::particle_runtime::EntityManagerFrame { self.frame }
        fn advance_root(&mut self, root: &str, _: u32, random: &mut crate::particle_runtime::ViewerParticleSeed) -> Result<()> {
            assert_eq!(root, self.root);
            self.order.borrow_mut().push((self.root, random.draw()));
            Ok(())
        }
        fn finish(&mut self) -> Result<()> {
            if self.fail_finish { return Err("fixture materialization failure".into()); }
            self.published.set(true);
            Ok(())
        }
        fn refuse(&mut self, _: &str) { self.published.set(false); }
    }

    fn frame_owners() -> [Box<FrameOwner>; 2] {
        use crate::particle_runtime::*;
        let manager = std::sync::Arc::new(std::sync::Mutex::new(ViewerEntityManager::default()));
        let order = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        ["first", "second"].map(|root| {
            let id = EntityManagerOwner::default();
            manager.lock().unwrap().parts_mut().1.apply(id, &mut vec![EntityRootChange::Register {
                id: root.into(), scene_mask: 7, update_mask: 6,
            }]);
            Box::new(FrameOwner { manager: manager.clone(), id, root, order: order.clone(),
                frame: EntityManagerFrame { presentation_time_f32_bits: 1.0f32.to_bits(),
                    delta_seconds_f32_bits: 0.5f32.to_bits(), cameras: Some([true; 3]) },
                published: Default::default(), fail_finish: false })
        })
    }

    #[test]
    fn common_owner_finalization_failure_revokes_earlier_materialization_and_retains_draws() {
        let [first, mut second] = frame_owners();
        let manager = first.manager.clone();
        let order = first.order.clone();
        let first_publication = first.published.clone();
        let second_publication = second.published.clone();
        second.fail_finish = true;
        let mut owners: Vec<Box<dyn PreparedEntityManagerOwner>> = vec![first, second];
        assert!(dispatch_prepared_entity_owners(&mut owners).unwrap_err().to_string().contains("materialization"));
        assert_eq!(order.borrow().iter().map(|(root, _)| *root).collect::<Vec<_>>(),
            ["second", "first", "second", "first", "second", "first"]);
        assert_eq!(manager.lock().unwrap().random_state().counter, 6);
        assert!(!first_publication.get() && !second_publication.get());
        assert!(!manager.lock().unwrap().frame_is_published(1.0f32.to_bits()));
        assert!(dispatch_prepared_entity_owners(&mut owners).is_err());
        assert_eq!(manager.lock().unwrap().random_state().counter, 6);
        assert_eq!(order.borrow().len(), 6);
    }

    #[test]
    fn later_adapter_collection_failure_revokes_taken_owner_without_draws() {
        struct Adapter {
            family: MapVisualFamily,
            pending: std::cell::RefCell<Option<Box<FrameOwner>>>,
            fail_take: bool,
            taken: std::rc::Rc<std::cell::Cell<u32>>,
        }
        impl ProductionMapFeatureAdapter for Adapter {
            fn family(&self) -> MapVisualFamily { self.family }
            fn prepare(&self, _: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
                panic!("owner collection must not run family preparation")
            }
            fn take_prepared_entity_manager_owner(&self) -> Result<Option<Box<dyn PreparedEntityManagerOwner + '_>>> {
                self.taken.set(self.taken.get() + 1);
                if self.fail_take { return Err("fixture later owner collection failure".into()); }
                Ok(self.pending.borrow_mut().take().map(|owner| owner as Box<dyn PreparedEntityManagerOwner>))
            }
        }
        let [first, second] = frame_owners();
        let manager = first.manager.clone();
        let frame = first.frame;
        let order = first.order.clone();
        let publication = first.published.clone();
        publication.set(true);
        let taken = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut adapters = ProductionMapFeatureAdapters::default();
        adapters.register(Box::new(Adapter { family: MapVisualFamily::LightingShadows,
            pending: std::cell::RefCell::new(Some(first)), fail_take: false, taken: taken.clone() })).unwrap();
        adapters.register(Box::new(Adapter { family: MapVisualFamily::UnitsCounters,
            pending: std::cell::RefCell::new(Some(second)), fail_take: true, taken: taken.clone() })).unwrap();
        let error = adapters.collect_prepared_entity_owners().err().unwrap();
        assert!(error.to_string().contains("later owner collection failure"));
        assert_eq!(taken.get(), 2);
        assert!(order.borrow().is_empty());
        assert!(!publication.get());
        let mut manager = manager.lock().unwrap();
        assert_eq!(manager.random_state().counter, 0);
        assert!(!manager.frame_is_published(frame.presentation_time_f32_bits));
        for frame in [frame, crate::particle_runtime::EntityManagerFrame {
            presentation_time_f32_bits: 2.0f32.to_bits(), ..frame
        }] {
            let refusal = manager.advance_frame(frame, &mut []).unwrap_err();
            assert!(refusal.to_string().contains("later owner collection failure"));
            assert_eq!(manager.random_state().counter, 0);
        }
    }

    #[test]
    fn later_population_failure_drains_both_owners_and_requires_explicit_recovery() {
        use crate::particle_runtime::*;
        struct Adapter {
            family: MapVisualFamily,
            fail: std::rc::Rc<std::cell::Cell<bool>>,
            pending: std::rc::Rc<std::cell::RefCell<Option<Box<FrameOwner>>>>,
        }
        impl ProductionMapFeatureAdapter for Adapter {
            fn family(&self) -> MapVisualFamily { self.family }
            fn prepare(&self, _: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
                panic!("failed population must not publish a family")
            }
            fn prepare_entity_manager_population(&self, context: &FoundationFeatureContext<'_>) -> Result<()> {
                assert!(self.pending.borrow().is_some());
                assert_eq!(context.viewer.animation_time_f32_bits, 1.0f32.to_bits());
                if self.fail.get() { return Err("fixture lighting population preparation failed".into()); }
                Ok(())
            }
            fn take_prepared_entity_manager_owner(&self) -> Result<Option<Box<dyn PreparedEntityManagerOwner + '_>>> {
                Ok(self.pending.borrow_mut().take().map(|owner| owner as Box<dyn PreparedEntityManagerOwner>))
            }
        }
        let [first, second] = frame_owners();
        let manager = first.manager.clone();
        let frame = first.frame;
        let order = first.order.clone();
        let identities = [first.id, second.id];
        let publications = [first.published.clone(), second.published.clone()];
        for publication in &publications { publication.set(true); }
        let pending = [first, second].map(|owner| std::rc::Rc::new(std::cell::RefCell::new(Some(owner))));
        let world = WorldFrame::from_raw_scheduled(replay_core::RawFrame {
            hour_index: 79, date: None, save_version: None, schema_version: None, rest: Default::default(),
        }, None, None, None, replay_core::BuildSchedule::Serial);
        let viewer = ViewerOwnedMapState { sky_enabled: true, unit_models_enabled: true,
            viewport_width: 512, viewport_height: 512, camera: None, observer_country_id: None,
            map_mode_id: "political".into(), map_mode_selection_epoch: 0, selected_object_ids: vec![],
            input_revision: 0, animation_time_f32_bits: 1.0f32.to_bits(), frame_delta_seconds_f32_bits: 0.5f32.to_bits() };
        let selection = crate::production_features::xmap15::Xmap15ViewerSelection::default();
        let context = FoundationFeatureContext::new_for_owned_accounting_tests(&world, &viewer, &[], &[], &selection);
        let fail = std::rc::Rc::new(std::cell::Cell::new(true));
        let mut adapters = ProductionMapFeatureAdapters::default();
        for (family, pending) in [MapVisualFamily::UnitsCounters, MapVisualFamily::LightingShadows]
            .into_iter().zip(&pending) {
            adapters.register(Box::new(Adapter { family, fail: fail.clone(), pending: pending.clone() })).unwrap();
        }
        let error = adapters.prepare_lighting_entity_population(&context, Ok(())).unwrap_err();
        assert!(error.to_string().contains("lighting population preparation"));
        assert!(pending.iter().all(|pending| pending.borrow().is_none()));
        assert!(publications.iter().all(|publication| !publication.get()));
        assert!(order.borrow().is_empty());
        let next = EntityManagerFrame { presentation_time_f32_bits: 2.0f32.to_bits(), ..frame };
        {
            let mut manager = manager.lock().unwrap();
            assert!(!manager.frame_is_published(frame.presentation_time_f32_bits));
            assert!(!manager.frame_is_published(next.presentation_time_f32_bits));
            assert!(manager.advance_frame(next, &mut []).unwrap_err().to_string()
                .contains("lighting population preparation"));
            assert_eq!(manager.random_state().counter, 0);
            assert_eq!(manager.reset_frame_publication(), Err(EntityManagerFrameRefusal::RegisteredRootsOnReset));
            for identity in identities { manager.remove_owner(identity); }
            manager.reset_frame_publication().unwrap();
        }
        for ((pending, id), root) in pending.iter().zip(identities).zip(["first", "second"]) {
            manager.lock().unwrap().parts_mut().1.apply(id, &mut vec![EntityRootChange::Register {
                id: root.into(), scene_mask: 7, update_mask: 6,
            }]);
            *pending.borrow_mut() = Some(Box::new(FrameOwner { manager: manager.clone(), id,
                frame: next, root, order: order.clone(), published: publications[usize::from(root == "second")].clone(),
                fail_finish: false }));
        }
        fail.set(false);
        adapters.prepare_lighting_entity_population(&context, Ok(())).unwrap();
        assert!(pending.iter().all(|pending| pending.borrow().is_none()));
        assert!(publications.iter().all(|publication| publication.get()));
        assert!(manager.lock().unwrap().frame_is_published(next.presentation_time_f32_bits));
        assert_eq!(manager.lock().unwrap().random_state().counter, 6);
        assert_eq!(order.borrow().len(), 6);
    }

    #[test]
    fn common_owner_dispatch_rejects_independent_manager_before_any_draw() {
        let [first, mut second] = frame_owners();
        let manager = first.manager.clone();
        second.manager = std::sync::Arc::new(std::sync::Mutex::new(crate::particle_runtime::ViewerEntityManager::default()));
        let other = second.manager.clone();
        let mut owners: Vec<Box<dyn PreparedEntityManagerOwner>> = vec![first, second];
        assert!(dispatch_prepared_entity_owners(&mut owners).unwrap_err().to_string().contains("disagree"));
        for manager in [manager, other] {
            assert_eq!(manager.lock().unwrap().random_state().counter, 0);
            assert!(!manager.lock().unwrap().frame_is_published(1.0f32.to_bits()));
        }
    }

    /// kills: a broken PROCESS_MEMORY_COUNTERS binding - a wrong `cb` or
    /// layout fails the call and every footprint row would read zero.
    #[cfg(windows)]
    #[test]
    fn the_process_footprint_reads_a_live_nonzero_commit() {
        let (private_bytes, peak_private_bytes) =
            process_private_bytes().expect("K32GetProcessMemoryInfo answers for this process");
        assert!(
            private_bytes > 0 && peak_private_bytes >= private_bytes,
            "{private_bytes} {peak_private_bytes}"
        );
    }

    /// kills: a cost row measured over less than the family's whole iteration
    /// (the timer started after the context is built, or the row pushed before
    /// the family's coverage row), a second push that would attribute one
    /// family's cost to another, and a row count that no longer follows
    /// `MapVisualFamily::ALL`.
    #[test]
    fn every_family_cost_row_spans_its_whole_preparation() {
        let source = include_str!("production_foundation.rs").replace("\r\n", "\n");
        let body = source
            .split_once("fn prepare_foundation_frame_inner(")
            .expect("the family loop must exist")
            .1
            .split_once("\n/// Why a family the rest of the preparation reads directly")
            .expect("the family loop must end")
            .0;
        let (before_loop, loop_and_after) = body
            .split_once("    for family in MapVisualFamily::ALL {\n")
            .expect("the family loop must iterate MapVisualFamily::ALL");
        assert!(
            before_loop.contains("let mut family_costs = Vec::with_capacity(MapVisualFamily::ALL.len());"),
            "the cost rows are not sized to the family list"
        );
        let phase_body = loop_and_after.split_once("for (family, publish) in steps.into_iter().flatten() {")
            .expect("every preparation phase is timed").1;
        let first_statement = phase_body
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("//"))
            .expect("the loop has statements");
        assert_eq!(
            first_statement, "let family_started = std::time::Instant::now();",
            "the family timer does not start at the top of the iteration"
        );
        assert_eq!(
            loop_and_after.matches("family_costs.push(FamilyPreparationCost {").count(),
            1,
            "a family's cost is pushed from more than one place"
        );
        let coverage_row = loop_and_after
            .find("families.push(FamilyCoverageRow {")
            .expect("the coverage row is pushed");
        let cost_row = loop_and_after
            .find("family_costs.push(FamilyPreparationCost {")
            .expect("the cost row is pushed");
        let loop_end = loop_and_after
            .find("\n    }\n")
            .expect("the loop closes");
        assert!(
            coverage_row < cost_row && cost_row < loop_end,
            "the cost row is not the last thing the iteration records"
        );
        assert!(
            loop_and_after[cost_row..loop_end].contains("family_started.elapsed()"),
            "the cost row does not read the timer started at the top"
        );
        let loop_body = &loop_and_after[..loop_end];
        assert_eq!(
            loop_body.matches("family_started").count(),
            4,
            "the timer must have one start, two retained phase costs, and one publication read"
        );
        // After the coverage row the iteration is exactly the two measurement
        // statements, verbatim: nothing can run between the timer's read and
        // the row that records it, and nothing after the row.
        let tail = loop_body[coverage_row..]
            .split_once("});")
            .expect("the coverage row closes")
            .1;
        assert_eq!(
            tail.split_whitespace().collect::<Vec<_>>().join(" "),
            concat!(
                "let (private_bytes_after, peak_private_bytes_after) = ",
                "process_private_bytes().unwrap_or((0, 0)); ",
                "family_costs.push(FamilyPreparationCost { family, ",
                "wall_ms: u64::try_from((family_started.elapsed() + match family { ",
                "MapVisualFamily::UnitsCounters => entity_preparation_cost[0], ",
                "MapVisualFamily::LightingShadows => entity_preparation_cost[1], ",
                "_ => std::time::Duration::ZERO, }).as_millis())?, ",
                "private_bytes_after, peak_private_bytes_after, }); }"
            ),
            "the iteration's tail is not the two measurement statements, verbatim"
        );
        assert!(
            loop_and_after.contains("family_costs,\n    })"),
            "the rows do not leave with the prepared frame"
        );
    }

    /// kills: recording every adapter error as a content gap (a defect filed as
    /// an outcome); dropping the owner scope from a refusal's gap id; accepting
    /// a blank cause; and widening the hour window a single hour's refusal
    /// claims.
    #[test]
    fn only_a_typed_content_refusal_becomes_an_owner_scoped_gap() {
        let hour = 17_030_998i64;
        let family = MapVisualFamily::LabelsMapText;

        match classify_family_preparation(family, hour, Ok(7u32)).unwrap() {
            FamilyPreparation::Prepared(value) => assert_eq!(value, 7),
            FamilyPreparation::Refused(gap) => panic!("a success refused: {}", gap.reason),
        }

        let refusal = classify_family_preparation::<u32>(
            family,
            hour,
            Err(FamilyContentRefusal::new(
                "XMAP17.unbound_runtime_variable",
                "no operand binds OVERLORDADJ",
            )),
        )
        .unwrap();
        let FamilyPreparation::Refused(gap) = refusal else {
            panic!("a content refusal was taken for a prepared family");
        };
        assert_eq!(gap.gap_id, "XMAP17.unbound_runtime_variable");
        assert_eq!(gap.reason, "no operand binds OVERLORDADJ");
        assert_eq!(gap.first_unavailable_hour, hour);
        assert_eq!(gap.last_unavailable_hour, hour);
        assert_eq!(gap.witness_sha256.len(), 64);
        assert!(gap
            .witness_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        // The witness has to move with what is being claimed, or two different
        // refusals would carry one witness.
        let other = classify_family_preparation::<u32>(
            family,
            hour,
            Err(FamilyContentRefusal::new(
                "XMAP17.unbound_runtime_variable",
                "a different cause",
            )),
        )
        .unwrap();
        let FamilyPreparation::Refused(other) = other else {
            panic!("a content refusal was taken for a prepared family");
        };
        assert_ne!(other.witness_sha256, gap.witness_sha256);

        // Anything that is not a content refusal is a contract violation and
        // still stops the preparation, with its own words.
        let contract: Box<dyn std::error::Error> =
            "XMAP17 admitted content belongs to another product modset".into();
        let error = classify_family_preparation::<u32>(family, hour, Err(contract)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "XMAP17 admitted content belongs to another product modset"
        );

        // A refusal that is not scoped to the family's own owner packet, or
        // that says nothing, is itself a defect.
        for (gap_id, reason) in [
            ("XMAP04.unbound_runtime_variable", "wrong owner"),
            ("XMAP17.", "empty suffix"),
            ("unbound_runtime_variable", "no owner at all"),
            ("XMAP17.unbound_runtime_variable", "   "),
        ] {
            let error = classify_family_preparation::<u32>(
                family,
                hour,
                Err(FamilyContentRefusal::new(gap_id, reason)),
            )
            .unwrap_err()
            .to_string();
            assert!(
                error.contains("is not scoped to its owner packet, or whose reason is blank"),
                "{gap_id:?}/{reason:?} was admitted: {error}"
            );
        }
    }

    /// kills: letting one family be registered twice. The family loop prepares
    /// the generic slot adapter for every family, so one also held in a
    /// dedicated slot would be prepared twice and one answer discarded.
    #[test]
    fn a_family_cannot_be_registered_twice_by_either_route() {
        struct Fixture(MapVisualFamily);
        impl ProductionMapFeatureAdapter for Fixture {
            fn family(&self) -> MapVisualFamily {
                self.0
            }

            fn prepare(
                &self,
                _context: &FoundationFeatureContext<'_>,
            ) -> Result<FoundationFeatureOutput> {
                unreachable!("registration fixture is never prepared")
            }
        }

        for family in MapVisualFamily::ALL {
            let mut adapters = ProductionMapFeatureAdapters::default();
            adapters.register(Box::new(Fixture(family))).unwrap();
            assert!(adapters.already_registered(family), "{family:?}");
            let error = adapters
                .register(Box::new(Fixture(family)))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("duplicate production adapter"),
                "{family:?}: {error}"
            );
        }

        // The dedicated-slot direction, which the loop above cannot reach: one
        // predicate covers both, and each dedicated family names itself in it.
        let source = include_str!("production_foundation.rs");
        let predicate = source
            .split_once("fn already_registered(&self, family: MapVisualFamily) -> bool {")
            .expect("the registration predicate must exist")
            .1
            .split_once("\n    }")
            .expect("the registration predicate must end")
            .0;
        assert!(predicate.contains("self.slots.contains_key(&family)"));
        for (family, slot) in [
            ("MapVisualFamily::Trees", "self.xmap11"),
            ("MapVisualFamily::CitiesBuildingsMapObjects", "self.xmap12"),
            ("MapVisualFamily::LabelsMapText", "self.xmap17"),
        ] {
            assert!(
                predicate.contains(&format!("{family} => {slot}.is_some()")),
                "{family} is not covered by the registration predicate"
            );
        }
    }

    /// The two places that name an input's owner must name the SAME owner.
    ///
    /// There were two: this file's `owner_packet_for_authority_gap`, which the
    /// global gap accounting and every family's owned-input derivation read, and
    /// `MAP_AUTHORITY_OWNER_PROVEN_INPUTS` in `map_authority`, which is the
    /// ledger. They disagreed about `mapmode.raids` from 2026-08-06 until the r4
    /// frame dump refused a whole frame over it, because nothing compared them:
    /// the table said XMAP21, the ledger says XMAP18, and XMAP18's own owned
    /// list said XMAP18 too. It stayed invisible while XMAP21's adapter was a
    /// standing refusal that consumed no gaps.
    ///
    /// kills: reintroducing an owner in either place that the other does not
    /// agree with. A gap has exactly one consumer, so two owners is a frame that
    /// cannot seal.
    #[test]
    fn the_owner_table_and_the_ledger_name_the_same_owner() {
        let mut compared = 0usize;
        for proven in crate::map_authority::MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
            let table = map_authority_owner_packet(proven.input_id).unwrap_or_else(|| {
                panic!(
                    "the owner table assigns no packet to {}, which the ledger owner-proves",
                    proven.input_id
                )
            });
            assert_eq!(
                table, proven.owner_packet,
                "the owner table says {} owns {} and the ledger says {}",
                table, proven.input_id, proven.owner_packet
            );
            compared += 1;
        }
        assert!(
            compared >= 20,
            "only {compared} owner-proven inputs were compared; the ledger cannot have \
             shrunk this far without someone noticing"
        );

        // The ledger only lists owner-PROVEN inputs, so the loop above cannot
        // see a disagreement about an unproven one - and
        // `mapmode.resource_deposits`, half of the original defect, is exactly
        // that. The other half of the partition is each family's own declared
        // list. XMAP18's is the only hand-written one left; XMAP21 derives from
        // the table, and no other family keeps one (Codex review, round 11).
        assert_eq!(
            crate::production_features::xmap18::OWNED_AUTHORITY_INPUTS.to_vec(),
            map_authority_inputs_owned_by("XMAP18"),
            "XMAP18's declared owned-input list and the owner table disagree"
        );

        // And every required input has exactly ONE owner that lists it back. A
        // gap has one consumer; an input the table cannot place has none, and
        // whichever family reached it first would consume a gap nobody assigned.
        for input in crate::map_authority::MAP_AUTHORITY_REQUIRED_INPUTS {
            let owner = map_authority_owner_packet(input)
                .unwrap_or_else(|| panic!("the owner table places no packet on {input}"));
            assert!(
                map_authority_inputs_owned_by(owner).contains(input),
                "{input} is owned by {owner}, which does not list it back"
            );
        }

        // THE WHOLE PARTITION, WRITTEN OUT. Everything above compares the table
        // with something derived from the table, or with the ledger, which
        // covers only the owner-PROVEN two thirds. Re-owning an unproven input
        // and re-pinning the identity passes all of it, because every consumer
        // reads the same changed table (Codex review, round 11). This is the
        // independent statement: 44 rows, written here, compared with the
        // table's answer for each. Moving an owner means editing this list, on
        // purpose, in a diff a reader can see.
        const EXPECTED_OWNERS: [(&str, &str); 44] = [
            ("combat.land_active_epoch", "XMAP14"),
            ("combat.land_terminal_detail", "XMAP14"),
            ("combat.naval_air_epoch", "XMAP14"),
            ("control.controller_by_province", "XMAP05"),
            ("control.controller_by_state", "XMAP05"),
            ("control.owner_by_state", "XMAP05"),
            ("country.current_capital_state", "XMAP21"),
            ("country.d18_complete_roster", "XMAP05"),
            ("country.presentation_identity", "XMAP05"),
            ("diplomacy.faction_roster", "XMAP18"),
            ("diplomacy.pair_relations", "XMAP18"),
            ("effects.semantic_lifecycle", "XMAP20"),
            ("fog.province_visibility_by_pov", "XMAP05"),
            ("fog.spotted_land_by_pov", "XMAP05"),
            ("fog.spotted_taskforces_by_pov", "XMAP05"),
            ("fog.spotted_wings_by_pov", "XMAP05"),
            ("lighting.point_light_lifecycle", "XMAP09"),
            ("logistics.rail_hub_frames", "XMAP16"),
            ("logistics.rail_topology_state", "XMAP16"),
            ("map_furniture.current_victory_points", "XMAP21"),
            ("mapmode.air_superiority", "XMAP18"),
            ("mapmode.infrastructure_state", "XMAP18"),
            ("mapmode.intelligence", "XMAP18"),
            ("mapmode.naval_supremacy", "XMAP18"),
            ("mapmode.occupation_resistance_compliance", "XMAP18"),
            ("mapmode.raids", "XMAP18"),
            ("mapmode.resource_deposits", "XMAP18"),
            ("mapmode.scripted_inputs", "XMAP18"),
            ("mapmode.supply_state", "XMAP16"),
            ("objects.building_state_damage", "XMAP12"),
            ("objects.city_population", "XMAP12"),
            ("objects.generic_population_lifecycle", "XMAP12"),
            ("orders.army_battleplans_semantic", "XMAP15"),
            ("orders.theatre_army_hierarchy", "XMAP13"),
            ("session.player_country_set", "XMAP18"),
            ("terrain.snow_mud_semantic", "XMAP07"),
            ("units.air_base_location", "XMAP13"),
            ("units.divisions", "XMAP13"),
            ("units.taskforces_fleets", "XMAP13"),
            ("units.wings", "XMAP13"),
            ("weather.province_snow_accumulator", "XMAP07"),
            ("weather.region_gameplay", "XMAP19"),
            ("weather.visual_membership_lifecycle", "XMAP19"),
            ("world.calendar_time", "XMAP09"),
        ];
        let expected: std::collections::BTreeMap<&str, &str> =
            EXPECTED_OWNERS.into_iter().collect();
        assert_eq!(
            expected.len(),
            EXPECTED_OWNERS.len(),
            "the written-out partition repeats an input"
        );
        let shipped: std::collections::BTreeSet<&str> =
            crate::map_authority::MAP_AUTHORITY_REQUIRED_INPUTS
                .iter()
                .copied()
                .collect();
        assert_eq!(
            shipped,
            expected.keys().copied().collect::<std::collections::BTreeSet<_>>(),
            "the written-out partition is not over the shipped input set"
        );
        for (input, owner) in expected {
            assert_eq!(
                map_authority_owner_packet(input),
                Some(owner),
                "{input} moved owner without this list moving with it"
            );
        }
    }

    /// The pinned authority-set identity is the identity of the set the product
    /// actually ships.
    ///
    /// The identity is what lets a reader of an authority artifact refuse a
    /// census over a set nobody measured - but only while the pin it compares
    /// against is current. An artifact edited in both its rows AND its declared
    /// identity agrees with itself; the pin is the independent half, and a pin
    /// nothing keeps honest goes stale the first time an input is added or
    /// re-owned.
    ///
    /// kills: growing or re-owning `MAP_AUTHORITY_REQUIRED_INPUTS` while the
    /// pinned digest stays put, which would leave every reader written against
    /// the old set silently accepting the new one.
    #[test]
    fn the_pinned_authority_identity_is_the_shipped_set() {
        // The preimage, built HERE rather than by asking the function under
        // test. Comparing `map_authority_set_identity()` with the pin alone is
        // tautological the moment the function is reimplemented to return the
        // pin: the set could then be re-owned freely (Codex review, round 6).
        // This is the independent oracle; the production function is compared
        // to IT, and the pin is compared to it too.
        let independent = {
            let mut preimage = String::new();
            for input in crate::map_authority::MAP_AUTHORITY_REQUIRED_INPUTS {
                preimage.push_str(input);
                preimage.push('\t');
                preimage.push_str(match owner_packet_for_authority_gap(input) {
                    Some(owner) => owner,
                    None => MAP_AUTHORITY_OWNER_UNCLASSIFIED,
                });
                preimage.push('\n');
            }
            Sha256::hex(preimage.as_bytes())
        };

        assert_eq!(
            crate::map_authority::MAP_AUTHORITY_REQUIRED_INPUTS.len(),
            44,
            "the authority set changed size; re-pin MAP_AUTHORITY_SET_IDENTITY_A729 and every \
             reader written against it"
        );
        assert_eq!(
            independent, MAP_AUTHORITY_SET_IDENTITY_A729,
            "the shipped (input, owner) set no longer hashes to the pinned identity; re-pin it \
             and every reader written against it"
        );
        assert_eq!(
            map_authority_set_identity(),
            independent,
            "the product's own identity function no longer describes the set it ships"
        );

        // The identity is a function of the OWNERS too, not just the input
        // names: re-owning one input alone has to move it, or an artifact whose
        // owner column was edited would still pass a reader's recomputation.
        let mut reowned = String::new();
        for input in crate::map_authority::MAP_AUTHORITY_REQUIRED_INPUTS {
            reowned.push_str(input);
            reowned.push('\t');
            reowned.push_str(if **input == *"country.current_capital_state" {
                "XMAP20"
            } else {
                map_authority_owner_packet(input).unwrap_or(MAP_AUTHORITY_OWNER_UNCLASSIFIED)
            });
            reowned.push('\n');
        }
        assert_ne!(
            Sha256::hex(reowned.as_bytes()),
            independent,
            "a re-owned input must not hash to the shipped identity"
        );
    }

    /// No shipped adapter carries a STANDING refusal any more.
    ///
    /// XMAP20 and XMAP21 used to be registered through constructors
    /// (`carrier_absent()` / `authority_absent()`) that could never vouch, so no
    /// frame this product composed could seal on ANY recording or hour. Both now
    /// decide from the recording in `prepare()`, which is what lets a recording
    /// that carries the inputs get past them.
    ///
    /// kills: reintroducing a constructor-shaped refusal in the shipped set, and
    /// letting either family's blocker stop being owner-scoped or lose its cause.
    #[test]
    fn no_shipped_adapter_carries_a_standing_refusal() {
        // The shipped set is built in two halves, and both register adapters.
        let source = include_str!("production_foundation.rs");
        let after_core = source
            .split_once("fn foundation_feature_adapter_core(")
            .expect("the production adapter set must exist")
            .1;
        let core = format!(
            "{}{}",
            after_core
                .split_once("\npub fn ")
                .expect("adapter core must end")
                .0,
            after_core
                .split_once("pub fn product_feature_adapters_with_assigned_stage_schedule(")
                .expect("the shipped adapter set must exist")
                .1
                .split_once("\n#[derive(")
                .expect("the shipped adapter set must end")
                .0
        );
        assert_eq!(
            core.matches("_absent()").count(),
            0,
            "a shipped adapter is registered through a constructor that can never vouch"
        );
        // XMAP14's refusal is CONDITIONAL - it refuses only when this install's
        // content cannot build the layer - so it is not a standing refusal and is
        // counted separately.
        assert_eq!(core.matches("new_or_unavailable(").count(), 1);

        // Both former standing refusals still refuse with an owner-scoped,
        // caused gap when their inputs are missing - the refusal moved into
        // prepare(), it did not disappear.
        let hour = 17_030_998i64;
        let refusals = [
            (
                MapVisualFamily::ParticlesEffects,
                "XMAP20.test_unused_gap",
                crate::production_features::xmap20::Xmap20EffectsAdapter::deferred_to_draw_output(),
            ),
            (
                MapVisualFamily::MapFurnitureIcons,
                "XMAP21.map_furniture_engine_derivations_owed",
                crate::production_features::xmap21::Xmap21MapFurnitureAdapter::derivation_owed_output(
                    hour,
                ),
            ),
        ];
        for (family, gap_id, output) in &refusals {
            match &output.disposition {
                FamilyDisposition::DeferredToDraw { draw_contract_id, .. } => {
                    assert_eq!(*family, MapVisualFamily::ParticlesEffects);
                    assert_eq!(draw_contract_id, crate::XMAP20_EFFECT_DRAW_CONTRACT);
                }
                FamilyDisposition::Unavailable { gap } => {
                    assert_eq!(&gap.gap_id, gap_id, "{family:?}");
                    assert!(!gap.reason.trim().is_empty(), "{family:?} refuses silently");
                }
                other => panic!("{family:?} must still be able to refuse: {other:?}"),
            }
        }

        // Every OTHER family vouching does not rescue the frame, and the seal
        // says which one stopped it.
        let envelope = FoundationRenderEnvelope {
            schema: FOUNDATION_RENDER_SCHEMA.into(),
            hour_index: hour,
            content_manifest_sha256: "1".repeat(64),
            product_modset_sha256: "2".repeat(64),
            effective_vfs_manifest_sha256: "3".repeat(64),
            foundation_content_operation_sha256: "4".repeat(64),
            families: MapVisualFamily::ALL
                .into_iter()
                .map(|family| FamilyCoverageRow {
                    family,
                    owner_packet: family.owner_packet().into(),
                    disposition: refusals
                        .iter()
                        .find(|(refusing_family, _, _)| *refusing_family == family)
                        .map(|(_, _, output)| output.disposition.clone())
                        .unwrap_or(FamilyDisposition::AuthenticatedEmpty {
                            producer_registration_sha256: "5".repeat(64),
                            enumerator_witness_sha256: "6".repeat(64),
                        }),
                })
                .collect(),
            element_diagnostics: Vec::new(),
        };
        assert_eq!(envelope.gaps().count(), 1);
        let refusal = envelope
            .seal_readiness(&std::collections::BTreeSet::new())
            .unwrap_err();
        assert!(refusal.contains("ParticlesEffects"), "{refusal}");
        let particle_contract = std::collections::BTreeSet::from([
            FoundationRenderEnvelope::draw_contract_key(
                MapVisualFamily::ParticlesEffects, crate::XMAP20_EFFECT_DRAW_CONTRACT,
            ),
        ]);
        assert!(envelope.seal_readiness(&particle_contract).unwrap_err().contains("MapFurnitureIcons"));
    }

    /// The map authority the family loop reads is derived from the frame being
    /// prepared, inside the preparation, and no entry takes one from a caller.
    ///
    /// kills: reintroducing an admission parameter on any preparation entry -
    /// the seam by which an admission derived from another frame reached the
    /// family loop and disagreed with the frame-gated tokens (Codex round 1,
    /// lane WP3) - and deriving the admission from anything but the frame and
    /// manifest the preparation was handed.
    #[test]
    fn the_map_authority_is_derived_from_the_frame_being_prepared_and_never_handed_in() {
        let source = include_str!("production_foundation.rs").replace("\r\n", "\n");
        let inner = source
            .split_once("fn prepare_foundation_frame_inner(")
            .expect("the inner preparation must exist")
            .1;
        let (signature, body) = inner
            .split_once(") -> Result<PreparedFoundationFrame> {")
            .expect("the inner preparation must have a signature");
        assert!(
            !signature.contains("MapAuthorityAdmission"),
            "prepare_foundation_frame_inner takes an admission it did not derive"
        );
        let before_loop = body
            .split_once("for family in MapVisualFamily::ALL {")
            .expect("the family loop must exist")
            .0;
        assert!(
            before_loop.contains(
                "map_authority_projection_from_world_frame(world_frame, manifest)?"
            ),
            "the projection is not derived from the frame and manifest being prepared"
        );
        assert!(
            before_loop.contains("admit_map_authority_projection(&map_authority_projection)?"),
            "the admission is not derived from that projection"
        );
        for entry in [
            "pub fn prepare_with_xmap12_inputs(",
            "pub fn prepare_with_xmap12_and_xmap15_inputs(",
            "pub fn prepare_foundation_frame(",
        ] {
            let signature = source
                .split_once(entry)
                .unwrap_or_else(|| panic!("{entry} must exist"))
                .1
                .split_once(") -> Result<PreparedFoundationFrame> {")
                .unwrap_or_else(|| panic!("{entry} must have a signature"))
                .0;
            assert!(
                !signature.contains("MapAuthorityAdmission"),
                "{entry} takes an admission it did not derive"
            );
        }
    }

    #[test]
    fn every_adapter_preparation_in_the_family_loop_is_accounted() {
        let source = include_str!("production_foundation.rs").lines().collect::<Vec<_>>().join("\n");
        let body = source
            .split_once("fn prepare_foundation_frame_inner(")
            .expect("the family loop must exist")
            .1
            .split_once("\n/// Why a family the rest of the preparation reads directly")
            .expect("the family loop must end")
            .0;
        assert_eq!(
            body.matches(".prepare(").count(),
            5,
            "the family loop prepares a number of adapters this test does not account for"
        );
        assert_eq!(
            body.matches("prepared_or_refusal!(").count(),
            6,
            "an adapter preparation bypasses the content-refusal accounting"
        );
        let arm = body
            .split_once("macro_rules! prepared_or_refusal {")
            .expect("the accounting macro must exist")
            .1
            .split_once("\n        }")
            .expect("the accounting macro must end")
            .0;
        assert!(arm.contains("classify_family_preparation(family, hour, $outcome)?"));
        assert!(arm.contains("content_refusal = Some(gap)"));
        // The refusal arm RECORDS and falls through: a `return` here would stop
        // the preparation on the first family that cannot answer, and every
        // string counted above would still be present.
        for stop in ["return", "break", "continue"] {
            assert!(
                !arm.contains(stop),
                "the refusal arm {stop}s instead of recording the family and going on"
            );
        }
        // ...and what the loop then does with a recorded refusal: one row, and
        // on to the next family.
        let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains(
                concat!(
                    "let disposition = if let Some(gap) = content_refusal {",
                    " FamilyDisposition::Unavailable { gap } } else if"
                )
            ),
            "a recorded refusal no longer becomes this family's row"
        );

        assert!(flat.contains("early_weather = Some(classify_family_preparation( MapVisualFamily::WeatherAtmosphere, hour, adapter.prepare(&context))?);"));
        assert!(flat.contains("let weather_preparation = if family == MapVisualFamily::WeatherAtmosphere { early_weather.take() } else { None };"));
        assert!(flat.contains("Some(FamilyPreparation::Prepared(output)) => Some(output)"));
        assert!(flat.contains("Some(FamilyPreparation::Refused(gap)) => { content_refusal = Some(gap); None }"));

        // Registration is what keeps that per-family preparation single.
        let register = source
            .split_once("pub fn register(&mut self")
            .expect("registration must exist")
            .1
            .split_once("
    }")
            .expect("registration must end")
            .0;
        assert!(register.contains("if self.already_registered(family) {"));
        for (route, family) in [
            ("fn register_xmap11(", "MapVisualFamily::Trees"),
            ("fn register_xmap12(", "MapVisualFamily::CitiesBuildingsMapObjects"),
            ("fn register_xmap17(", "MapVisualFamily::LabelsMapText"),
        ] {
            let body = source
                .split_once(route)
                .unwrap_or_else(|| panic!("{route} must exist"))
                .1
                .split_once("
    }")
                .unwrap_or_else(|| panic!("{route} must end"))
                .0;
            assert!(
                body.contains(&format!("if self.already_registered({family}) {{")),
                "{route} no longer refuses a family the other route already holds"
            );
        }
    }

    /// The dump's own path, end to end at envelope grain: three families refuse,
    /// and ONE preparation names all three instead of the first.
    ///
    /// kills: reporting only the first refusal; letting a refusing hour be
    /// served from the stale-frame cache; and treating a draw-time deferral as
    /// a refusal.
    #[test]
    fn one_refusing_hour_names_every_refusing_family_at_once() {
        let hour = 17_030_998i64;
        let refusals = [
            (
                MapVisualFamily::LabelsMapText,
                "XMAP17.unbound_runtime_variable",
            ),
            (
                MapVisualFamily::ParticlesEffects,
                "XMAP20.test_unused_gap",
            ),
            (
                MapVisualFamily::MapFurnitureIcons,
                "XMAP21.map_furniture_authority_absent",
            ),
        ];
        let envelope = FoundationRenderEnvelope {
            schema: FOUNDATION_RENDER_SCHEMA.into(),
            hour_index: hour,
            content_manifest_sha256: "1".repeat(64),
            product_modset_sha256: "2".repeat(64),
            effective_vfs_manifest_sha256: "3".repeat(64),
            foundation_content_operation_sha256: "4".repeat(64),
            families: MapVisualFamily::ALL
                .into_iter()
                .map(|family| FamilyCoverageRow {
                    family,
                    owner_packet: family.owner_packet().into(),
                    disposition: match refusals.iter().find(|(refusing, _)| *refusing == family) {
                        Some((_, gap_id)) => {
                            let reason = format!("{gap_id} cannot be vouched");
                            FamilyDisposition::Unavailable {
                                gap: FoundationGap {
                                    gap_id: (*gap_id).into(),
                                    first_unavailable_hour: hour,
                                    last_unavailable_hour: hour,
                                    witness_sha256: Sha256::hex(gap_id.as_bytes()),
                                    reason,
                                },
                            }
                        }
                        None if family == MapVisualFamily::GradingPostFx => {
                            FamilyDisposition::DeferredToDraw {
                                producer_registration_sha256: "5".repeat(64),
                                draw_contract_id: "XMAP10.postfx".into(),
                                reason: "answered per frame".into(),
                            }
                        }
                        None => FamilyDisposition::AuthenticatedEmpty {
                            producer_registration_sha256: "6".repeat(64),
                            enumerator_witness_sha256: "7".repeat(64),
                        },
                    },
                })
                .collect(),
            element_diagnostics: Vec::new(),
        };
        envelope.validate().unwrap();

        // This is the product route a dump takes: playback admission, then the
        // freshness requirement that turns the outcome into the CLI's error.
        let mut playback = FoundationPlaybackState::default();
        let outcome = playback
            .admit_allowing_xmap16_unavailable(envelope.clone())
            .unwrap();
        let unavailable = require_fresh_foundation_frame(outcome).unwrap_err();
        let message = unavailable.to_string();
        for (family, gap_id) in refusals {
            assert!(message.contains(gap_id), "{family:?} is missing from {message}");
        }
        assert!(!message.contains("XMAP10.postfx"), "a deferral is not a refusal: {message}");

        // A refusing hour never enters the stale-frame cache, so a later hour
        // cannot be served this one's frame.
        let vouched = FoundationRenderEnvelope {
            families: MapVisualFamily::ALL
                .into_iter()
                .map(|family| FamilyCoverageRow {
                    family,
                    owner_packet: family.owner_packet().into(),
                    disposition: FamilyDisposition::AuthenticatedEmpty {
                        producer_registration_sha256: "6".repeat(64),
                        enumerator_witness_sha256: "7".repeat(64),
                    },
                })
                .collect(),
            hour_index: hour + 1,
            ..envelope.clone()
        };
        assert!(matches!(
            playback.admit_allowing_xmap16_unavailable(vouched).unwrap(),
            FoundationPlaybackOutcome::Fresh(_)
        ));
    }

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
        assert_eq!(topology.ordered_edges().len(), 19);
        assert!(border_early.assigned_ordinal < rivers.assigned_ordinal);
        assert!(rivers.assigned_ordinal < straits.assigned_ordinal);
        assert!(rivers.assigned_ordinal < rail_execute.assigned_ordinal);
        assert!(rail_execute.assigned_ordinal < trees.assigned_ordinal);
        assert!(trees.assigned_ordinal < water_surface.assigned_ordinal);
        assert!(water_surface.assigned_ordinal < border_late.assigned_ordinal);
        assert!(border_late.assigned_ordinal < straits.assigned_ordinal);
        assert!(straits.assigned_ordinal < supply_arrow_execute.assigned_ordinal);
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
        assert_eq!(straits.assigned_ordinal, 7);
        assert_eq!(rail_execute.assigned_ordinal, 3);
        assert_eq!(trees.assigned_ordinal, 4);
        assert_eq!(water_surface.assigned_ordinal, 5);
        assert_eq!(border_late.assigned_ordinal, 6);
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
                    rivers.identity(),
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
                    straits.identity(),
                ),
                portable_render_stream::AssignedStageEdge::new(
                    straits.identity(),
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
            6
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

    /// The two map-mode inputs XMAP21 READS are owned by XMAP18.
    ///
    /// This test used to assert the opposite, and that is why the defect
    /// survived: the table said XMAP21, the ledger and XMAP18's own list said
    /// XMAP18, and the only test that looked pinned the table's answer instead
    /// of comparing the two. It stayed green for the whole time XMAP21's
    /// adapter was a standing refusal that consumed no gaps, and the r4 frame
    /// dump was what finally refused a frame over it:
    ///
    ///   MapFurnitureIcons claimed an unknown or already-consumed authority gap
    ///   mapmode.resource_deposits
    ///
    /// kills: giving either input back to XMAP21. XMAP21 reads raids and
    /// resource deposits to place icons; authority over a `mapmode.*` input is
    /// XMAP18's, and reading is not owning.
    #[test]
    fn the_two_map_mode_inputs_xmap21_reads_are_owned_by_xmap18() {
        for input in ["mapmode.raids", "mapmode.resource_deposits"] {
            assert_eq!(
                owner_packet_for_authority_gap(input),
                Some("XMAP18"),
                "{input} is a map-mode input"
            );
        }
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

    // The regression this lane exists for. Before it, XMAP10/20/21 had NO
    // registered adapter, so prepare_foundation_frame_inner fell to its
    // (None, None) arm and told the readiness graph all three had "not promoted
    // the viewer-owned production adapter". That was false: all three execute at
    // draw time.
    //
    // kills: DELETING ANY OF THE THREE REGISTRATIONS. The earlier version of
    // this test hand-built dispositions and stayed green with all three
    // registration lines removed (Codex review, round 1), so it now reads the
    // production registration site itself -- the same source-guard idiom
    // xmap00_map_authority.rs uses for this function.
    #[test]
    fn the_production_adapter_set_registers_all_three_late_families() {
        let source = include_str!("production_foundation.rs");
        let core = source
            .split_once("fn foundation_feature_adapter_core(")
            .expect("the production adapter set must exist")
            .1
            .split_once("
pub fn ")
            .expect("adapter core must end")
            .0;
        for registration in [
            "xmap10::Xmap10GradingPostFxAdapter",
            "xmap20::Xmap20EffectsAdapter",
            "xmap21::Xmap21MapFurnitureAdapter",
        ] {
            assert!(
                core.contains(registration),
                "the production adapter set no longer registers {registration}"
            );
        }

        // ...and each adapter claims exactly the family it is registered for,
        // so a registration cannot be silently pointed at the wrong row.
        let adapters: [(&dyn ProductionMapFeatureAdapter, MapVisualFamily); 3] = [
            (
                &crate::production_features::xmap10::Xmap10GradingPostFxAdapter,
                MapVisualFamily::GradingPostFx,
            ),
            (
                &crate::production_features::xmap20::Xmap20EffectsAdapter,
                MapVisualFamily::ParticlesEffects,
            ),
            (
                &crate::production_features::xmap21::Xmap21MapFurnitureAdapter,
                MapVisualFamily::MapFurnitureIcons,
            ),
        ];
        let mut registry = ProductionMapFeatureAdapters::default();
        for (adapter, family) in adapters {
            assert_eq!(adapter.family(), family);
            assert!(registry.get(family).is_none(), "{family:?} promoted early");
        }
        registry
            .register(Box::new(
                crate::production_features::xmap10::Xmap10GradingPostFxAdapter,
            ))
            .unwrap();
        assert!(registry.get(MapVisualFamily::GradingPostFx).is_some());
    }

    // The real dispositions, from the real adapters -- and none of them may
    // still read as "no adapter", which is the false statement this lane
    // deleted.
    //
    // kills: reverting either of the two remaining blockers to the generic
    // unimplemented gap.
    #[test]
    fn the_late_families_report_their_real_blocker_not_a_missing_adapter() {
        let hour = 5;
        let unimplemented = unimplemented_adapter_gap(MapVisualFamily::GradingPostFx, hour);
        assert!(unimplemented.gap_id.contains("adapter_unimplemented"));

        let dispositions = [
            (
                MapVisualFamily::ParticlesEffects,
                crate::production_features::xmap20::Xmap20EffectsAdapter::deferred_to_draw_output()
                .disposition,
            ),
            (
                MapVisualFamily::MapFurnitureIcons,
                crate::production_features::xmap21::Xmap21MapFurnitureAdapter::derivation_owed_output(
                    hour,
                )
                .disposition,
            ),
        ];
        for (family, disposition) in dispositions {
            if let FamilyDisposition::DeferredToDraw { draw_contract_id, .. } = &disposition {
                assert_eq!(family, MapVisualFamily::ParticlesEffects);
                assert_eq!(draw_contract_id, crate::XMAP20_EFFECT_DRAW_CONTRACT);
                continue;
            }
            let FamilyDisposition::Unavailable { gap } = &disposition else {
                panic!("{family:?} must name a real blocker while it cannot vouch");
            };
            assert!(
                !gap.gap_id.contains("adapter_unimplemented"),
                "{family:?} still reports a missing adapter"
            );
            assert!(
                gap.gap_id.starts_with(family.owner_packet()),
                "{family:?} gap {} is not owner-scoped",
                gap.gap_id
            );
            assert!(!gap.reason.trim().is_empty());
        }

        // XMAP10 defers, because its TIMING problem is permanent: the grade
        // needs live per-frame D3D identities and the prior frame's temporal
        // receipt. It no longer ALSO refuses on a missing exposure authority --
        // that row was retired once the exposure law was proved to be a pure
        // function of the frames the viewer draws for itself, exactly as it is
        // for a real client (see `crate::luminance_law`).
        //
        // kills: reintroducing an authority gate here, which would put the
        // family back to refusing every frame.
        use crate::production_features::xmap10::Xmap10GradingPostFxAdapter;
        let deferred = Xmap10GradingPostFxAdapter::deferred_disposition();
        let FamilyDisposition::DeferredToDraw {
            draw_contract_id,
            reason,
            ..
        } = &deferred
        else {
            panic!("XMAP10 answers at draw time, never at foundation time");
        };
        assert_eq!(
            draw_contract_id,
            crate::production_features::xmap10::XMAP10_POSTFX_DRAW_CONTRACT
        );
        assert!(reason.contains("temporal-adaptation receipt"), "{reason}");
    }

    #[test]
    fn a_frame_still_refuses_while_xmap20_and_xmap21_have_nothing_to_vouch_with() {
        let hour = 5;
        let mut envelope = FoundationRenderEnvelope {
            schema: FOUNDATION_RENDER_SCHEMA.into(),
            hour_index: hour,
            content_manifest_sha256: "1".repeat(64),
            product_modset_sha256: "2".repeat(64),
            effective_vfs_manifest_sha256: "3".repeat(64),
            foundation_content_operation_sha256: "4".repeat(64),
            families: MapVisualFamily::ALL
                .into_iter()
                .map(|family| FamilyCoverageRow {
                    family,
                    owner_packet: family.owner_packet().into(),
                    disposition: FamilyDisposition::AuthenticatedEmpty {
                        producer_registration_sha256: "5".repeat(64),
                        enumerator_witness_sha256: "6".repeat(64),
                    },
                })
                .collect(),
            element_diagnostics: Vec::new(),
        };
        let deferred = crate::production_features::xmap10::Xmap10GradingPostFxAdapter::deferred_disposition();
        envelope.families[MapVisualFamily::ALL
            .iter()
            .position(|family| *family == MapVisualFamily::GradingPostFx)
            .expect("every family is in ALL")]
        .disposition = deferred;
        let discharged = std::collections::BTreeSet::from([
            FoundationRenderEnvelope::draw_contract_key(
                MapVisualFamily::GradingPostFx,
                crate::production_features::xmap10::XMAP10_POSTFX_DRAW_CONTRACT,
            ),
        ]);
        // XMAP10 alone is satisfiable at draw time.
        envelope.seal_readiness(&discharged).unwrap();
        // ...and a deferral is not a vouch: with nothing discharged the same
        // envelope refuses.
        assert!(envelope
            .seal_readiness(&std::collections::BTreeSet::new())
            .is_err());
        assert!(!envelope.has_refusing_family(), "nothing refused yet");

        // ...but the two refusing families still stop the frame, and no amount
        // of draw-time discharging changes that.
        envelope.families[MapVisualFamily::ALL
            .iter()
            .position(|family| *family == MapVisualFamily::ParticlesEffects)
            .expect("every family is in ALL")]
        .disposition = FamilyDisposition::Unavailable {
            gap: FoundationGap {
                gap_id: "XMAP20.test_unavailable_runtime".into(),
                first_unavailable_hour: hour,
                last_unavailable_hour: hour,
                witness_sha256: "a".repeat(64),
                reason: "test runtime provider unavailable".into(),
            },
        };
        let refusal = envelope.seal_readiness(&discharged).unwrap_err();
        assert!(refusal.contains("refuses this frame"), "{refusal}");
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
            element_diagnostics: Vec::new(),
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
            element_diagnostics: Vec::new(),
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

#[cfg(test)]
mod recovered_light_census;
