use super::*;
use crate::production_foundation::PreparedEntityManagerOwner;
use crate::xmap09_light_lifecycle::{
    DerivedLightRows, PreparedLightEntityFrame, ViewerLightLifecycle, Xmap09PlacedEntity,
};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xmap13ModelObjectRefusal {
    ContentLease,
    PublicationUnavailable,
    PublicationGuard(String),
    PreparedFrame,
    CandidateUnavailable,
    CategoryUnavailable,
    AnimationConstruction(String),
    AnimatedBoundsUnavailable,
    Collision(crate::pdx_mesh::NativePdxCollisionRefusal),
    Ray(crate::native_model_selection::NativeCollisionRefusal),
}

enum PublishedModelBounds {
    Static {
        animation: Arc<crate::xmap09_light_lifecycle::MeshAnimationAdmission>,
        library: Arc<crate::Xmap12MeshLibrary>,
        declaration_source: UsedContentKey,
        declaration_line: usize,
    },
    Unavailable(Xmap13ModelObjectRefusal),
}

pub(crate) struct Xmap13PublishedModelObject {
    identity: Xmap13UnitIdentity,
    root: String,
    normal_mesh: Arc<crate::xmap12_pdxmesh::AdmittedNormalMesh>,
    mesh_source: UsedContentKey,
    object_matrix: [f32; 16],
    authored_mesh_scale: f32,
    bounds: PublishedModelBounds,
}

impl Xmap13PublishedModelObject {
    pub(crate) fn root(&self) -> &str { &self.root }
    pub(crate) fn mesh(&self) -> &Arc<crate::Xmap12SharedMesh> { self.normal_mesh.mesh() }
    pub(crate) fn mesh_source(&self) -> &UsedContentKey { &self.mesh_source }
    pub(crate) fn object_matrix(&self) -> [f32; 16] { self.object_matrix }
    pub(crate) fn authored_mesh_scale(&self) -> f32 { self.authored_mesh_scale }

    pub(crate) fn geometry_hit(&self, origin: [f32; 3], direction: [f32; 3])
        -> std::result::Result<bool, Xmap13ModelObjectRefusal>
    {
        let collision = self.mesh().native_collision(self.authored_mesh_scale)
            .map_err(Xmap13ModelObjectRefusal::Collision)?;
        if let Some(collision) = collision.as_ref().filter(|row| !row.indices.is_empty()) {
            return crate::native_model_selection::model_geometry_hit(self.object_matrix, origin, direction,
                &collision.scaled_positions, &collision.indices, None)
                .map_err(Xmap13ModelObjectRefusal::Ray);
        }
        let local_bounds = match &self.bounds {
            PublishedModelBounds::Static { .. } => self.normal_mesh.local_bounds(),
            PublishedModelBounds::Unavailable(reason) => return Err(reason.clone()),
        };
        crate::native_model_selection::model_geometry_hit(self.object_matrix, origin, direction,
            &[], &[], Some(local_bounds)).map_err(Xmap13ModelObjectRefusal::Ray)
    }
}

pub(crate) enum Xmap13ModelFallbackSlot<'a> {
    KnownNull,
    Present(&'a Xmap13PublishedModelObject),
    Unavailable(Xmap13ModelObjectRefusal),
}

pub(crate) enum Xmap13ModelPrimary<'a> {
    Present(&'a Xmap13PublishedModelObject),
    KnownAbsent { fallback: [Xmap13ModelFallbackSlot<'a>; 4] },
    Unavailable(Xmap13ModelObjectRefusal),
}

impl Xmap13ModelPrimary<'_> {
    pub(crate) fn geometry_hit(self, origin: [f32; 3], direction: [f32; 3])
        -> std::result::Result<bool, Xmap13ModelObjectRefusal>
    {
        match self {
            Self::Present(object) => object.geometry_hit(origin, direction),
            Self::Unavailable(reason) => Err(reason),
            Self::KnownAbsent { fallback } => {
                for slot in fallback {
                    match slot {
                        Xmap13ModelFallbackSlot::KnownNull => {},
                        Xmap13ModelFallbackSlot::Unavailable(reason) => return Err(reason),
                        Xmap13ModelFallbackSlot::Present(object) => {
                            if object.geometry_hit(origin, direction)? { return Ok(true); }
                        },
                    }
                }
                Ok(false)
            },
        }
    }
}

struct PublishedLandObjects {
    owner: std::sync::Weak<()>,
    lease: crate::mod_resolver::ContentAdmissionLease,
    prepared_digest: String,
    objects: BTreeMap<(u32, u32), Xmap13PublishedModelObject>,
    published: Arc<DerivedLightRows>,
}

#[derive(Clone)]
pub(crate) struct Xmap13PublishedLandObjects(Arc<PublishedLandObjects>);

#[derive(Clone)]
pub struct Xmap13ModelHitPublication {
    objects: Xmap13PublishedLandObjects,
    resolver: GameFileResolver,
    admission: ValidatedContentAdmission,
}

impl Xmap13ModelHitPublication {
    pub fn prepared_frame_sha256(&self) -> &str {
        &self.objects.0.prepared_digest
    }

    pub fn geometry_hit(
        &self,
        prepared: &PreparedXmap13Frame,
        identity: &Xmap13UnitIdentity,
        province: u32,
        category: u32,
        world_origin: [f32; 3],
        world_direction: [f32; 3],
    ) -> std::result::Result<bool, Xmap13ModelObjectRefusal> {
        prepared.validate().map_err(|_| Xmap13ModelObjectRefusal::PreparedFrame)?;
        if prepared.deterministic_sha256 != self.prepared_frame_sha256() {
            return Err(Xmap13ModelObjectRefusal::PreparedFrame);
        }
        self.objects.with_primary(
            &self.resolver, &self.admission, identity, province, category,
            |primary| primary.geometry_hit(world_origin, world_direction),
        )
    }
}

impl Xmap13PublishedLandObjects {
    fn validate(&self, resolver: &GameFileResolver, admission: &ValidatedContentAdmission)
        -> std::result::Result<(), Xmap13ModelObjectRefusal>
    {
        if resolver.content_admission_lease().as_ref() != Some(&self.0.lease)
            || self.0.lease.manifest_digest_sha256() != Some(admission.manifest_digest_sha256()) {
            return Err(Xmap13ModelObjectRefusal::ContentLease);
        }
        if self.0.owner.upgrade().is_none() {
            return Err(Xmap13ModelObjectRefusal::PublicationUnavailable);
        }
        for object in self.0.objects.values() {
            if object.normal_mesh.admission_lease() != &self.0.lease {
                return Err(Xmap13ModelObjectRefusal::ContentLease);
            }
            if let PublishedModelBounds::Static { animation, library, declaration_source, declaration_line } = &object.bounds {
                let view = animation.checked(resolver, admission, library)
                    .map_err(|error| Xmap13ModelObjectRefusal::AnimationConstruction(error.to_string()))?;
                if view.declaration_source != declaration_source || view.declaration_line != *declaration_line
                    || view.mesh_source != &object.mesh_source || !Arc::ptr_eq(view.mesh, object.mesh())
                    || !matches!(view.construction,
                        crate::xmap09_light_lifecycle::MeshAnimationConstruction::NotInvokedEmptyLookup
                        | crate::xmap09_light_lifecycle::MeshAnimationConstruction::LoaderReturnedNull)
                {
                    return Err(Xmap13ModelObjectRefusal::AnimationConstruction(
                        "static bounds differ from retained construction".into()));
                }
            }
            self.0.published.root_animation(resolver, admission, &object.root)
                .map_err(|error| Xmap13ModelObjectRefusal::PublicationGuard(error.to_string()))?;
        }
        Ok(())
    }

    pub(crate) fn with_primary<T>(&self, resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission, identity: &Xmap13UnitIdentity,
        province: u32, category: u32,
        visit: impl FnOnce(Xmap13ModelPrimary<'_>) -> std::result::Result<T, Xmap13ModelObjectRefusal>)
        -> std::result::Result<T, Xmap13ModelObjectRefusal>
    {
        self.validate(resolver, admission)?;
        let primary = if category != 0 {
            Xmap13ModelPrimary::Unavailable(Xmap13ModelObjectRefusal::CategoryUnavailable)
        } else {
            match self.0.objects.get(&(province, category)) {
                Some(object) if &object.identity == identity => Xmap13ModelPrimary::Present(object),
                _ => Xmap13ModelPrimary::Unavailable(Xmap13ModelObjectRefusal::CandidateUnavailable),
            }
        };
        let result = visit(primary);
        self.validate(resolver, admission)?;
        result
    }
}

#[derive(Clone)]
pub(super) struct LandModelCandidate {
    pub ordinal: usize,
    pub identity: Xmap13UnitIdentity,
    pub chain: crate::Xmap12ResolvedEntityChain,
    pub state: String,
    pub placement: Xmap13HelperPlacement,
}

pub struct Xmap13UnitParticlePublication {
    owner: crate::xmap09_light_lifecycle::RetainedEntityEffects,
    viewer: crate::ViewerOwnedMapState,
    prepared_sha256: String,
    unbound: Vec<portable_render_stream::ElementDiagnostic>,
}

impl Xmap13UnitParticlePublication {
    fn check(&self, context: &FoundationFeatureContext<'_>) -> Result<()> {
        let prepared = context.prepared_units.ok_or("XMAP13 particle prepared units unavailable")?;
        if context.viewer != &self.viewer || prepared.deterministic_sha256 != self.prepared_sha256 {
            return Err("XMAP13 particle publication belongs to another viewer/frame".into());
        }
        prepared.validate()?;
        let world = derived_frame_digest(context.world_frame)?;
        if prepared.semantic_input_sha256 !=
            derived::population_digest_from_world(&world, context.xmap13_motion)? {
            return Err("XMAP13 particle publication belongs to another world/motion input".into());
        }
        context.shared_entity_effects.ok_or("entity_manager.effects.publication_unavailable")?
            .check_owner(&self.owner, f32::from_bits(context.viewer.animation_time_f32_bits))
    }

    pub(crate) fn with_diagnostics<T>(&self, context: &FoundationFeatureContext<'_>,
        visit: impl FnOnce(&[portable_render_stream::ElementDiagnostic]) -> Result<T>,
    ) -> Result<T> {
        self.check(context)?;
        let result = visit(&self.unbound);
        self.check(context)?;
        result
    }
}

pub(super) struct LandRuntime {
    lifecycle: ViewerLightLifecycle,
    refused: bool,
    motion_lifetime: Option<String>,
    drawn_provinces: BTreeSet<u32>,
    slots: BTreeMap<u32, instance::LandModelSlot>,
}

impl LandRuntime {
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    #[cfg(test)]
    pub(super) fn animation_admission(&self, province: u32)
        -> Option<&std::result::Result<Arc<crate::xmap09_light_lifecycle::MeshAnimationAdmission>, String>>
    {
        self.slots.get(&province)?.animation_admission()
    }

    pub(super) fn new(manager: Arc<Mutex<crate::particle_runtime::ViewerEntityManager>>) -> Self {
        Self {
            lifecycle: ViewerLightLifecycle::with_entity_manager(manager).with_checked_effect_fields(),
            refused: false,
            motion_lifetime: None,
            drawn_provinces: BTreeSet::new(),
            slots: BTreeMap::new(),
        }
    }
}

pub(super) struct PendingLandPopulation {
    population: Xmap13DerivedPopulation,
    candidates: Vec<LandModelCandidate>,
    viewer: crate::ViewerOwnedMapState,
    prepared: PreparedLightEntityFrame,
}

pub(super) struct FinishedLandPopulation {
    population: Xmap13DerivedPopulation,
    candidates: Vec<LandModelCandidate>,
    viewer: crate::ViewerOwnedMapState,
    published: Arc<DerivedLightRows>,
    object_owner: Arc<()>,
    objects: Option<Xmap13PublishedLandObjects>,
    particle_visibility: Option<BTreeMap<String, std::result::Result<[bool; 3], String>>>,
}

struct LandLifecycleGuard<'a>(MutexGuard<'a, LandRuntime>);

impl std::ops::Deref for LandLifecycleGuard<'_> {
    type Target = ViewerLightLifecycle;
    fn deref(&self) -> &Self::Target { &self.0.lifecycle }
}

impl std::ops::DerefMut for LandLifecycleGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.0.lifecycle }
}

struct PreparedLandOwner<'a> {
    adapter: &'a Xmap13Adapter,
    runtime: MutexGuard<'a, LandRuntime>,
    pending: Option<PendingLandPopulation>,
    frame: crate::particle_runtime::EntityManagerFrame,
}

impl PreparedEntityManagerOwner for PreparedLandOwner<'_> {
    fn manager(&self) -> Arc<Mutex<crate::particle_runtime::ViewerEntityManager>> {
        self.runtime.lifecycle.entity_manager()
    }
    fn owner(&self) -> crate::particle_runtime::EntityManagerOwner {
        self.runtime.lifecycle.manager_owner()
    }
    fn frame(&self) -> crate::particle_runtime::EntityManagerFrame {
        self.frame
    }
    fn advance_root(
        &mut self,
        root: &str,
        _: u32,
        random: &mut crate::particle_runtime::ViewerParticleSeed,
    ) -> Result<()> {
        self.runtime.lifecycle.advance_prepared_root(
            &self.adapter.content.resolver,
            &self.pending.as_ref().ok_or("XMAP13 owner already finished")?.prepared,
            root, random,
        )
    }
    fn advance_effect(
        &mut self,
        instance: &str,
        phase: crate::particle_runtime::EntityEffectPhase,
    ) -> Result<()> {
        self.runtime.lifecycle.advance_prepared_effect(
            &self.adapter.content.resolver,
            &self.pending.as_ref().ok_or("XMAP13 owner already finished")?.prepared,
            instance,
            phase,
        )
    }
    fn finish(&mut self) -> Result<()> {
        let pending = self.pending.take().ok_or("XMAP13 owner already finished")?;
        let published = self
            .runtime
            .lifecycle
            .finish_entity_frame(&self.adapter.content.resolver, pending.prepared)?;
        *self
            .adapter
            .finished_land
            .lock()
            .map_err(|_| "XMAP13 publication lock poisoned")? = Some(FinishedLandPopulation {
            population: pending.population,
            candidates: pending.candidates,
            viewer: pending.viewer,
            published: Arc::new(published),
            object_owner: Arc::new(()),
            objects: None,
            particle_visibility: None,
        });
        Ok(())
    }
    fn refuse(&mut self, reason: &str) {
        self.runtime.lifecycle.refuse_entity_publication(reason);
        self.runtime.refused = true;
        self.runtime.drawn_provinces.clear();
        if let Ok(mut published) = self.adapter.finished_land.lock() {
            *published = None;
        }
    }
}

fn root_id(province: u32) -> String {
    format!("xmap13:province:{province}:kind:0")
}

fn replace_refusal(
    population: &mut Xmap13DerivedPopulation,
    ordinal: usize,
    cause: &str,
    detail: String,
) {
    if let Some(row) = population
        .element_diagnostics
        .iter_mut()
        .find(|row| row.element_id == format!("xmap13:division:source:{ordinal}"))
    {
        row.cause_id = cause.into();
        row.detail = detail;
        row.outcome = portable_render_stream::ElementOutcome::ExcludedUnknownInput;
    }
}

impl Xmap13Adapter {
    pub(super) fn require_scalar_land_mesh(&self, chain: &crate::Xmap12ResolvedEntityChain) -> Result<()> {
        let assets = self.entity_assets.get_or_init(||
            crate::xmap09_light_lifecycle::EntityAssets::load(&self.content.resolver)
                .map_err(|reason| reason.to_string())).as_ref().map_err(|reason| reason.clone())?;
        let mesh = assets.resolved_pdxmesh_declaration(
            &self.content.resolver, &self.content.admission, &chain.pdxmesh.name)?
            .ok_or("XMAP13 selected mesh declaration is unavailable")?;
        if mesh.binding != &chain.pdxmesh || mesh.source != &chain.pdxmesh_source {
            return Err("XMAP13 selected mesh differs from the retained catalog winner".into());
        }
        for field in mesh.declaration.value.fields() {
            if !["name", "file", "scale", "animation"].iter().any(|key| field.key_matches(key)) {
                return Err(format!("XMAP13 mesh field {:?} requires an unbound mesh variant or settings selector", field.key).into());
            }
        }
        let mut name = chain.entity.name.as_str();
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(name.to_owned()) { return Err("XMAP13 entity clone cycle".into()); }
            let resolved = assets.resolved_binding(&self.content.resolver, &self.content.admission, name)?
                .ok_or("XMAP13 entity has no admitted scalar mesh binding")?;
            let declaration = assets.entity_declarations(&self.content.resolver, &self.content.admission)?
                .find(|(source, entry)| **source == resolved.declaration_source
                    && entry.line == resolved.binding.block_line
                    && entry.value.first_scalar_ci("name") == Some(name))
                .map(|(_, entry)| entry).ok_or("XMAP13 entity declaration ordinal is unavailable")?;
            for field in declaration.value.fields() {
                if !["name", "pdxmesh", "clone", "scale", "default_state", "state"].iter()
                    .any(|key| field.key_matches(key)) {
                    return Err(format!("XMAP13 entity field {:?} is outside the proven scalar-mesh definition", field.key).into());
                }
            }
            let mut meshes = declaration.value.values_ci("pdxmesh");
            if let Some(mesh) = meshes.next() {
                if meshes.next().is_some() || mesh.as_scalar() != Some(chain.pdxmesh.name.as_str()) {
                    return Err("XMAP13 weighted or competing mesh requires the retained variation selector".into());
                }
            }
            match declaration.value.first_scalar_ci("clone") {
                Some(base) => name = base,
                None => return if declaration.value.first_scalar_ci("pdxmesh") == Some(chain.pdxmesh.name.as_str()) {
                    Ok(())
                } else { Err("XMAP13 entity has no witnessed scalar mesh".into()) },
            }
        }
    }
    pub(super) fn stage_land_population(
        &self,
        context: &FoundationFeatureContext<'_>,
    ) -> Result<()> {
        *self
            .finished_land
            .lock()
            .map_err(|_| "XMAP13 publication lock poisoned")? = None;
        let mut pending = self
            .pending_land
            .lock()
            .map_err(|_| "XMAP13 pending lock poisoned")?;
        if pending.is_some() {
            return Err("XMAP13 population was not dispatched".into());
        }
        let mut runtime = self
            .land_runtime
            .lock()
            .map_err(|_| "XMAP13 lifecycle lock poisoned")?;
        let lifetime = context.xmap13_motion.map(Xmap13MotionSample::lifetime);
        let recreated = lifetime.is_some() && lifetime != runtime.motion_lifetime.as_deref();
        if recreated {
            runtime.lifecycle.reset_population();
            runtime.slots.clear();
            runtime.drawn_provinces.clear();
            runtime.refused = false;
            runtime.motion_lifetime = lifetime.map(str::to_owned);
        } else if runtime.refused {
            return Err(
                "XMAP13 entity publication requires an explicit viewer helper discontinuity".into(),
            );
        }
        let result = (|| -> Result<PendingLandPopulation> {
            let mut population = self
                .content
                .derive_population(context.world_frame, context.xmap13_motion)?;
            let mut counts = BTreeMap::<u32, usize>::new();
            let mut locations_complete = context.world_frame.division_units_status()
                == ViewStatus::Present
                && context
                    .world_frame
                    .domain("divisions")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|rows| rows.len() == context.world_frame.division_units().len());
            for unit in context.world_frame.division_units() {
                match unit.location_province {
                    Leaf::Measured(province) if province > 0 => {
                        *counts.entry(province).or_default() += 1
                    }
                    _ => locations_complete = false,
                }
            }
            let mut candidates = Vec::new();
            let source_candidates = std::mem::take(&mut population.land_candidates);
            for candidate in &source_candidates {
                let province = candidate.placement.anchor_province_id;
                let reason = if !context.viewer.unit_models_enabled {
                    Some(
                        "land models disabled; counter grouping and symbol inputs remain unbound"
                            .to_owned(),
                    )
                } else if !locations_complete || counts.get(&province) != Some(&1) {
                    Some("province representative requires the complete native ordered competing-unit census".to_owned())
                } else {
                    let eligibility = (|| -> Result<bool> {
                        let unit = &context.world_frame.division_units()[candidate.ordinal];
                        let owner = unit
                            .owner
                            .measured()
                            .ok_or("XMAP13 model owner unavailable")?;
                        match context.viewer.observer_country_id.as_deref() {
                            Some(observer) => Ok(context.world_frame.country_handles_equal(
                                context.world_frame.country_identity_for_tag(owner.tag())?,
                                context.world_frame.country_identity_for_tag(observer)?,
                            )?),
                            None => Ok(true),
                        }
                    })();
                    match eligibility {
                        Ok(true) => None,
                        Ok(false) => Some(
                            "foreign model eligibility requires the native visibility threshold"
                                .to_owned(),
                        ),
                        Err(reason) => Some(reason.to_string()),
                    }
                };
                let reason = reason.or_else(|| self.require_scalar_land_mesh(&candidate.chain)
                    .err().map(|reason| reason.to_string()));
                if let Some(reason) = reason {
                    replace_refusal(
                        &mut population,
                        candidate.ordinal,
                        if context.viewer.unit_models_enabled {
                            "unit-model-input-unavailable"
                        } else {
                            "unit-counter-input-unavailable"
                        },
                        reason,
                    );
                } else {
                    candidates.push(candidate.clone());
                }
            }
            let active = candidates
                .iter()
                .map(|candidate| candidate.placement.anchor_province_id)
                .collect::<BTreeSet<_>>();
            if context.viewer.unit_models_enabled
                && runtime.slots.keys().any(|province| {
                    !active.contains(province)
                        && (!locations_complete || counts.contains_key(province))
                })
            {
                return Err("XMAP13 retained model has an unresolved current population; explicit viewer recreation is required".into());
            }
            runtime
                .slots
                .retain(|province, _| active.contains(province));
            let mut placed = Vec::new();
            let mut offsets = BTreeMap::new();
            for candidate in &candidates {
                let province = candidate.placement.anchor_province_id;
                let previous_land_draw = runtime.drawn_provinces.contains(&province);
                let slot = runtime
                    .slots
                    .entry(province)
                    .or_insert_with(|| instance::LandModelSlot::new(true));
                slot.refresh(&candidate.identity, Some(&candidate.chain), previous_land_draw)?;
                if slot.animation_admission().is_none() {
                    let receipt: Result<Arc<crate::xmap09_light_lifecycle::MeshAnimationAdmission>> = (|| {
                        let assets = self.entity_assets.get().ok_or("XMAP13 model catalog unavailable")?
                            .as_ref().map_err(|reason| reason.clone())?;
                        assets.mesh_animation_admission(&self.content.resolver, &self.content.admission,
                            &candidate.chain.pdxmesh.name, &candidate.chain.pdxmesh_source,
                            &candidate.chain.mesh_source, &self.content.library)
                    })();
                    slot.bind_animation_admission(&candidate.chain, receipt.map_err(|error| error.to_string()))?;
                }
                slot.advance(
                    context.viewer.frame_delta_seconds(),
                    *self.popup_speed.as_ref().map_err(|reason| reason.clone())?,
                )?;
                let scale = candidate.chain.entity.scale * candidate.chain.pdxmesh.scale;
                if !scale.is_finite() || scale <= 0.0 {
                    return Err("XMAP13 authored model scale invalid".into());
                }
                let matrix = crate::xmap12_pdxmesh::scaled_world_matrix(
                    slot.matrix(&candidate.identity, &candidate.placement)?,
                    scale,
                )?
                .map(f32::to_bits);
                let root = root_id(province);
                offsets.insert(root.clone(), 5.0);
                placed.push(Xmap09PlacedEntity {
                    id: root,
                    entity_token: candidate.chain.entity.name.clone(),
                    state: Some(candidate.state.clone()),
                    animate: true,
                    world_matrix_f32_bits: matrix,
                });
            }
            let prepared = runtime.lifecycle.prepare_entity_frame_with_state_offsets(
                &self.content.resolver,
                &placed,
                context.viewer,
                None,
                &offsets,
            )?;
            Ok(PendingLandPopulation {
                population,
                candidates,
                viewer: context.viewer.clone(),
                prepared,
            })
        })();
        match result {
            Ok(prepared) => {
                *pending = Some(prepared);
                Ok(())
            }
            Err(error) => {
                runtime
                    .lifecycle
                    .refuse_entity_publication(&error.to_string());
                runtime.slots.clear();
                runtime.drawn_provinces.clear();
                runtime.refused = true;
                Err(error)
            }
        }
    }

    pub(super) fn take_land_owner(
        &self,
    ) -> Result<Option<Box<dyn PreparedEntityManagerOwner + '_>>> {
        let pending = self
            .pending_land
            .lock()
            .map_err(|_| "XMAP13 pending lock poisoned")?
            .take();
        let Some(pending) = pending else {
            return Ok(None);
        };
        let frame = pending.prepared.frame;
        Ok(Some(Box::new(PreparedLandOwner {
            adapter: self,
            runtime: self
                .land_runtime
                .lock()
                .map_err(|_| "XMAP13 lifecycle lock poisoned")?,
            pending: Some(pending),
            frame,
        })))
    }

    pub(crate) fn model_hit_publication(
        &self,
        context: &FoundationFeatureContext<'_>,
        prepared: &PreparedXmap13Frame,
    ) -> std::result::Result<Xmap13ModelHitPublication, Xmap13ModelObjectRefusal> {
        Ok(Xmap13ModelHitPublication {
            objects: self.published_land_objects(context, prepared)?,
            resolver: self.content.resolver.clone(),
            admission: self.content.admission.clone(),
        })
    }

    pub(super) fn retained_land_effects(&self, time: f32)
        -> Result<Option<crate::xmap09_light_lifecycle::RetainedEntityEffects>>
    {
        let finished = self.finished_land.lock().map_err(|_| "XMAP13 publication lock poisoned")?;
        let Some(finished) = finished.as_ref() else { return Ok(None); };
        Ok(Some(crate::xmap09_light_lifecycle::RetainedEntityEffects::new(
            finished.published.clone(), &self.content.resolver, &self.content.admission, time,
        )?))
    }

    fn checked_land_particle_visibility(&self, time: f32, context: Option<&FoundationFeatureContext<'_>>)
        -> Result<BTreeMap<String, [bool; 3]>>
    {
        let finished = self.finished_land.lock().map_err(|_| "XMAP13 publication lock poisoned")?;
        let finished = finished.as_ref().ok_or("XMAP13 particle publication unavailable")?;
        crate::xmap09_light_lifecycle::RetainedEntityEffects::new(
            finished.published.clone(), &self.content.resolver, &self.content.admission, time,
        )?;
        if let Some(context) = context {
            if finished.viewer != *context.viewer { return Err("XMAP13 particle viewer differs".into()); }
            finished.population.validate_census(context.world_frame)?;
        }
        let visibility = finished.particle_visibility.as_ref()
            .ok_or("XMAP13 particle materialization unavailable")?;
        let mut roots = BTreeMap::new();
        for candidate in &finished.candidates {
            let root = root_id(candidate.placement.anchor_province_id);
            finished.published.root_animation(&self.content.resolver, &self.content.admission, &root)?;
            let mask = visibility.get(&root).ok_or("XMAP13 particle root visibility unavailable")?
                .as_ref().map_err(|reason| format!("XMAP13 particle root {root}: {reason}"))?;
            if roots.insert(root, *mask).is_some() {
                return Err("XMAP13 particle root duplicated".into());
            }
        }
        if roots.len() != visibility.len() {
            return Err("XMAP13 particle visibility has foreign roots".into());
        }
        Ok(roots)
    }

    pub(super) fn retained_land_particle_owner(
        &self, time: f32, cameras: [crate::particle_runtime::ParticleSortCamera; 3],
    ) -> Result<Option<Box<dyn crate::production_foundation::RetainedParticleDrawOwner + '_>>> {
        let roots = self.checked_land_particle_visibility(time, None)?;
        let runtime = self.land_runtime.lock().map_err(|_| "XMAP13 lifecycle lock poisoned")?;
        Ok(Some(Box::new(crate::xmap09_light_lifecycle::RetainedLifecycleParticleOwner::new(
            LandLifecycleGuard(runtime), time, cameras, roots,
        )?)))
    }

    pub(super) fn land_particle_publication(&self, context: &FoundationFeatureContext<'_>)
        -> Result<Option<Xmap13UnitParticlePublication>>
    {
        self.checked_land_particle_visibility(f32::from_bits(context.viewer.animation_time_f32_bits), Some(context))?;
        let prepared = context.prepared_units.ok_or("XMAP13 particle prepared units unavailable")?;
        self.published_land_objects(context, prepared)
            .map_err(|reason| format!("XMAP13 particle model publication: {reason:?}"))?;
        let finished = self.finished_land.lock().map_err(|_| "XMAP13 publication lock poisoned")?;
        let finished = finished.as_ref().ok_or("XMAP13 particle publication unavailable")?;
        let owner = crate::xmap09_light_lifecycle::RetainedEntityEffects::new(
            finished.published.clone(), &self.content.resolver, &self.content.admission,
            f32::from_bits(context.viewer.animation_time_f32_bits),
        )?;
        let mut unbound = Vec::new();
        let mut refuse = |branch: &str, count: u64, detail: String| {
            unbound.push(portable_render_stream::ElementDiagnostic::family_exception(
                MapVisualFamily::ParticlesEffects, branch, count, detail));
        };
        if context.world_frame.division_units_status() != ViewStatus::Present {
            refuse("unit-division-event-phase-unbound", 0, "division census is unavailable".into());
        } else if context.viewer.unit_models_enabled {
            for (index, _) in context.world_frame.division_units().iter().enumerate() {
                if !finished.candidates.iter().any(|candidate| candidate.ordinal == index) {
                    let reason = finished.population.element_diagnostics.iter()
                        .find(|row| row.element_id == format!("xmap13:division:source:{index}"))
                        .map(|row| row.detail.as_str()).unwrap_or("division model eligibility unavailable");
                    refuse(&format!("unit-division-{index}-event-phase-unbound"), 1, reason.into());
                }
            }
        }
        for (class, status, count) in [
            ("task-force", context.world_frame.fleets_status(), context.world_frame.fleets().len()),
            ("air-wing", context.world_frame.wings_status(), context.world_frame.wings().len()),
        ] {
            if status != ViewStatus::Present || count != 0 {
                refuse(&format!("unit-{class}-event-phase-unbound"), u64::try_from(count)?,
                    format!("{class} retained entity event binding is unavailable ({status:?})"));
            }
        }
        Ok(Some(Xmap13UnitParticlePublication {
            owner, viewer: finished.viewer.clone(),
            prepared_sha256: prepared.deterministic_sha256.clone(), unbound,
        }))
    }

    pub(crate) fn published_land_objects(&self, context: &FoundationFeatureContext<'_>,
        prepared: &PreparedXmap13Frame)
        -> std::result::Result<Xmap13PublishedLandObjects, Xmap13ModelObjectRefusal>
    {
        prepared.validate().map_err(|_| Xmap13ModelObjectRefusal::PreparedFrame)?;
        let finished = self.finished_land.lock()
            .map_err(|_| Xmap13ModelObjectRefusal::PublicationUnavailable)?;
        let finished = finished.as_ref().ok_or(Xmap13ModelObjectRefusal::PublicationUnavailable)?;
        let objects = finished.objects.as_ref().ok_or(Xmap13ModelObjectRefusal::PublicationUnavailable)?;
        if finished.viewer != *context.viewer
            || finished.population.deterministic_sha256 != prepared.semantic_input_sha256
            || objects.0.prepared_digest != prepared.deterministic_sha256 {
            return Err(Xmap13ModelObjectRefusal::PreparedFrame);
        }
        finished.population.validate_census(context.world_frame)
            .map_err(|_| Xmap13ModelObjectRefusal::PreparedFrame)?;
        objects.validate(&self.content.resolver, &self.content.admission)?;
        Ok(objects.clone())
    }

    pub(super) fn append_finished_land_models(
        &self,
        context: &FoundationFeatureContext<'_>,
        visibility: &crate::PreparedXmap05Visibility,
        prepared: &mut PreparedXmap13Frame,
    ) -> Result<()> {
        let mut finished = self
            .finished_land
            .lock()
            .map_err(|_| "XMAP13 publication lock poisoned")?;
        let Some(finished) = finished.as_mut() else {
            return Ok(());
        };
        finished.objects = None;
        finished.particle_visibility = None;
        finished.object_owner = Arc::new(());
        if finished.viewer != *context.viewer
            || finished.population.deterministic_sha256 != prepared.semantic_input_sha256
        {
            return Err("XMAP13 model publication belongs to another viewer/frame".into());
        }
        finished.population.validate_census(context.world_frame)?;
        prepared.element_diagnostics = finished.population.element_diagnostics.clone();
        let mut runtime = self
            .land_runtime
            .lock()
            .map_err(|_| "XMAP13 lifecycle lock poisoned")?;
        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP13 model camera unavailable")?;
        let mut used = prepared
            .ordered_used_content
            .iter()
            .map(|key| (key.normalized_game_relative_path.clone(), key.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut drawn_provinces = BTreeSet::new();
        let mut objects = BTreeMap::new();
        let mut particle_visibility = BTreeMap::new();
        for candidate in &finished.candidates {
            let root = root_id(candidate.placement.anchor_province_id);
            let slot = runtime
                .slots
                .get(&candidate.placement.anchor_province_id)
                .ok_or("XMAP13 retained model slot missing")?;
            let result = self.materialize_finished_land_model(
                context,
                visibility,
                candidate,
                slot,
                &root,
                &finished.published,
                u32::try_from(prepared.native_entity_draws.len())?,
            );
            let (model, unit, draw, object) = match result {
                Ok(Some(result)) => result,
                Ok(None) => {
                    particle_visibility.insert(root.clone(), Ok([false; 3]));
                    if let Some(row) = prepared.element_diagnostics.iter_mut().find(|row| {
                        row.element_id == format!("xmap13:division:source:{}", candidate.ordinal)
                    }) {
                        row.cause_id = "unit-counter-input-unavailable".into();
                        row.detail = "model is outside the current camera partitions; counter grouping/symbol operands remain unavailable".into();
                    }
                    continue;
                }
                Err(error) => {
                    particle_visibility.insert(root.clone(), Err(error.to_string()));
                    if let Some(row) = prepared.element_diagnostics.iter_mut().find(|row| {
                        row.element_id == format!("xmap13:division:source:{}", candidate.ordinal)
                    }) {
                        row.cause_id = "unit-model-content-unavailable".into();
                        row.detail = error.to_string();
                        row.outcome = portable_render_stream::ElementOutcome::ExcludedUnknownInput;
                    }
                    continue;
                }
            };
            particle_visibility.insert(root.clone(), Ok(std::array::from_fn(|pass| draw.partition_mask & (1 << pass) != 0)));
            for key in [
                &candidate.chain.entity_source,
                &candidate.chain.entity_pdxmesh_source,
                &candidate.chain.pdxmesh_source,
                &candidate.chain.mesh_source,
                &candidate.chain.shader_source,
            ]
            .into_iter()
            .chain(candidate.chain.entity_scale_source.iter())
            .chain(model.helper_content.iter())
            .chain(unit.ordered_texture_content.iter())
            .chain(unit.animation_asset.iter())
            .chain(object.normal_mesh.sources().iter())
            {
                self.content.admit_content(key)?;
                if used
                    .get(&key.normalized_game_relative_path)
                    .is_some_and(|old| old != key)
                {
                    return Err("XMAP13 model content conflict".into());
                }
                used.insert(key.normalized_game_relative_path.clone(), key.clone());
            }
            prepared.element_diagnostics.retain(|row| {
                row.element_id != format!("xmap13:division:source:{}", candidate.ordinal)
            });
            prepared
                .element_diagnostics
                .push(portable_render_stream::ElementDiagnostic {
                    family: MapVisualFamily::UnitsCounters,
                    owner_packet: "XMAP13".into(),
                    element_id: format!("{}:counter", unit.identity.canonical_id()),
                    source_ordinal: Some(u32::try_from(candidate.ordinal)?),
                    outcome: portable_render_stream::ElementOutcome::ExcludedUnknownInput,
                    part: String::new(),
                    cause_id: "unit-counter-input-unavailable".into(),
                    element_count: 1,
                    detail:
                        "model publication does not establish the counter grouping/symbol operands"
                            .into(),
                });
            if let Some(hit) = project_hit_target(
                &camera.production_camera()?,
                context.viewer.viewport_width,
                context.viewer.viewport_height,
                &unit,
            ) {
                prepared.hit_targets.push(hit);
            }
            if unit.casts_light_depth {
                prepared.light_depth_casters.push(draw.l03_caster_row()?);
            }
            objects.insert((candidate.placement.anchor_province_id, 0), object);
            prepared.prepared_model_poses.push(model);
            prepared.visible_units.push(unit.clone());
            prepared.identity_provider.ordered_units.push(unit);
            prepared.native_entity_draws.push(draw);
            drawn_provinces.insert(candidate.placement.anchor_province_id);
            finished.published.root_animation(
                &self.content.resolver,
                &self.content.admission,
                &root,
            )?;
        }
        prepared.ordered_used_content = used.into_values().collect();
        prepared.identity_provider.census_sha256 = ordered_identity_digest(
            prepared
                .identity_provider
                .ordered_units
                .iter()
                .map(|unit| &unit.identity),
        )?;
        prepared.identity_provider.admission_sha256 = Sha256::hex(
            format!(
                "{}\0{}\0{}\0{}",
                prepared.semantic_input_sha256,
                prepared.identity_provider.census_sha256,
                prepared.identity_provider.formation_census_sha256,
                used_content_digest(&prepared.ordered_used_content)?
            )
            .as_bytes(),
        );
        validate_derived_census(
            context.world_frame,
            prepared
                .identity_provider
                .ordered_units
                .iter()
                .map(|unit| &unit.identity),
            &prepared.element_diagnostics,
        )?;
        prepared.deterministic_sha256 = prepared_digest(prepared)?;
        prepared.validate()?;
        let lease = self.content.resolver.content_admission_lease()
            .ok_or("XMAP13 model publication has no content lease")?;
        finished.objects = Some(Xmap13PublishedLandObjects(Arc::new(PublishedLandObjects {
            owner: Arc::downgrade(&finished.object_owner), lease,
            prepared_digest: prepared.deterministic_sha256.clone(), objects,
            published: finished.published.clone(),
        })));
        finished.particle_visibility = Some(particle_visibility);
        runtime.drawn_provinces = drawn_provinces;
        Ok(())
    }

    fn materialize_finished_land_model(
        &self,
        context: &FoundationFeatureContext<'_>,
        visibility: &crate::PreparedXmap05Visibility,
        candidate: &LandModelCandidate,
        slot: &instance::LandModelSlot,
        root: &str,
        published: &DerivedLightRows,
        visible_order: u32,
    ) -> Result<
        Option<(
            PreparedXmap13ModelPose,
            PreparedXmap13Unit,
            crate::Xmap12PdxMeshMaterialization,
            Xmap13PublishedModelObject,
        )>,
    > {
        let mut model =
            self.published_land_model_pose(root, &candidate.identity, &candidate.chain, published)?;
        model.helper_placement = Some(candidate.placement.clone());
        model.helper_content = context
            .xmap13_motion
            .ok_or("XMAP13 model helper sample unavailable")?
            .geometry_sources()
            .to_vec();
        let request = slot
            .request(visible_order, &candidate.chain, &model)?
            .ok_or("XMAP13 model slot disabled")?;
        let camera = context
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP13 model camera unavailable")?;
        let draw = match crate::materialize_xmap12_pdxmesh(
            &self.content.resolver,
            &self.content.admission,
            &self.content.library,
            camera,
            &request,
        )? {
            crate::Xmap12MaterializeOutcome::Visible(draw) => *draw,
            crate::Xmap12MaterializeOutcome::HiddenByCamera => return Ok(None),
            crate::Xmap12MaterializeOutcome::AnimatedSkinUndecoded => {
                return Err("XMAP13 published palette was not materialized".into())
            }
        };
        let active =
            published.root_animation(&self.content.resolver, &self.content.admission, root)?;
        let assets = self
            .entity_assets
            .get()
            .ok_or("XMAP13 model catalog unavailable")?
            .as_ref()
            .map_err(|reason| reason.clone())?;
        let (entity, _) = assets.definition(
            &self.content.resolver,
            &self.content.admission,
            &candidate.chain.entity.name,
            &candidate.chain.entity_source,
        )?;
        let state = active
            .state_ordinal
            .and_then(|ordinal| entity.states.get(ordinal))
            .ok_or("XMAP13 published state unavailable")?;
        let authored_state_events = published.with_root_effects(
            &self.content.resolver, &self.content.admission, root, |events, _bindings| {
                if events != state.events.as_slice() {
                    return Err("XMAP13 retained state effects differ from selected definition".into());
                }
                let mut rows = Vec::new();
                for (index, event) in events.iter().enumerate() {
                    if event.entity.is_some() {
                        return Err("XMAP13 unit event-child identity projection unavailable".into());
                    }
                    if let Some(token) = &event.particle {
                        rows.push(Xmap13AuthoredStateEvent {
                            authored_event_index: u32::try_from(index)?,
                            authored_effect_index: 0,
                            effect_kind: Xmap13AuthoredEffectKind::Particle,
                            effect_token: token.clone(),
                            node: event.node.clone(),
                            event_time_f32_bits: event.time.map(f32::to_bits),
                            keep_particle: Some(event.keep_particle),
                            trigger_once: Some(event.trigger_once),
                        });
                    }
                }
                Ok(rows)
            },
        )?;
        let mesh = self.content.library.shared_mesh(
            &self.content.resolver,
            &self.content.admission,
            &candidate.chain.mesh_source,
        )?;
        let declaration = assets.resolved_pdxmesh_declaration(
            &self.content.resolver, &self.content.admission, &candidate.chain.pdxmesh.name)?
            .ok_or("XMAP13 published mesh declaration unavailable")?;
        let normal_mesh = self.content.library.normal_mesh_admission(
            &self.content.resolver, &self.content.admission, &declaration,
            &candidate.chain.mesh_source, &candidate.chain.shader_source,
            [camera.map_width, camera.map_height])?;
        if !Arc::ptr_eq(&mesh, normal_mesh.mesh()) {
            return Err("XMAP13 normal rows belong to another shared mesh".into());
        }
        let bounds = (|| -> std::result::Result<PublishedModelBounds, Xmap13ModelObjectRefusal> {
            let receipt = slot.animation_admission()
                .ok_or_else(|| Xmap13ModelObjectRefusal::AnimationConstruction("retained model construction unavailable".into()))?
                .as_ref().map_err(|reason| Xmap13ModelObjectRefusal::AnimationConstruction(reason.clone()))?;
            let view = receipt.checked(&self.content.resolver, &self.content.admission, &self.content.library)
                .map_err(|error| Xmap13ModelObjectRefusal::AnimationConstruction(error.to_string()))?;
            if view.declaration_source != declaration.source || view.declaration_line != declaration.declaration.line
                || view.mesh_source != &candidate.chain.mesh_source || !Arc::ptr_eq(view.mesh, &mesh)
            {
                return Err(Xmap13ModelObjectRefusal::AnimationConstruction("retained construction differs from published mesh".into()));
            }
            match view.construction {
                crate::xmap09_light_lifecycle::MeshAnimationConstruction::NotInvokedEmptyLookup
                | crate::xmap09_light_lifecycle::MeshAnimationConstruction::LoaderReturnedNull => {
                    Ok(PublishedModelBounds::Static { animation: receipt.clone(), library: self.content.library.clone(),
                        declaration_source: declaration.source.clone(), declaration_line: declaration.declaration.line })
                },
                crate::xmap09_light_lifecycle::MeshAnimationConstruction::Constructed { .. } =>
                    Err(Xmap13ModelObjectRefusal::AnimatedBoundsUnavailable),
            }
        })().unwrap_or_else(PublishedModelBounds::Unavailable);
        let drawable = mesh.drawable_model()?;
        let casts_light_depth = normal_mesh.casts_light_depth();
        let lod_bounds = xmap13_lod_bounds(drawable);
        let (min, max) = lod_bounds
            .get(draw.selected_lod as usize)
            .and_then(|bounds| *bounds)
            .ok_or("XMAP13 drawn model has no authored LOD bounds")?;
        let L03SemanticInputRow::PdxmeshObject(caster) = draw.l03_caster_row()? else {
            return Err("XMAP13 model produced another caster class".into());
        };
        let owner = context.world_frame.division_units()[candidate.ordinal]
            .owner
            .measured()
            .ok_or("XMAP13 model owner unavailable")?
            .tag()
            .to_owned();
        let (disclosure, visibility_reason) = match &candidate.identity {
            Xmap13UnitIdentity::Division { type_id, stable_id } => {
                visibility.land_disclosure(*type_id, *stable_id)
            }
            _ => return Err("XMAP13 land publication has another unit class".into()),
        };
        if !matches!(
            disclosure,
            ExactUnitDisclosure::MapVisible | ExactUnitDisclosure::ExactDetails
        ) {
            return Err("XMAP13 model has no current visible political identity".into());
        }
        let (animation_name, animation_asset) = match &model.pose.clip {
            Xmap13UnitClip::Authored {
                animation_name,
                animation_asset,
            } => (Some(animation_name.clone()), Some(animation_asset.clone())),
            Xmap13UnitClip::Unanimated { .. } => (None, None),
        };
        let unit = PreparedXmap13Unit {
            identity: candidate.identity.clone(),
            semantic_source_id: format!("xmap13:division:source:{}", candidate.ordinal),
            entity_instance_id: u64::from(candidate.placement.anchor_province_id) << 8,
            owner_country: owner,
            anchor_province_id: candidate.placement.anchor_province_id,
            entity_asset: model.entity_asset.clone(),
            model_registry: model.model_registry.clone(),
            mesh: model.mesh.clone(),
            shader_source: draw.shader_source.clone(),
            animation_asset,
            ordered_texture_content: draw
                .ordered_texture_sources
                .iter()
                .map(|source| source.content.clone())
                .collect(),
            entity_name: model.entity_name.clone(),
            model_name: model.model_name.clone(),
            state_name: model.state_name.clone(),
            material_variant: draw
                .submeshes
                .iter()
                .map(|submesh| submesh.material_shader.as_str())
                .collect::<Vec<_>>()
                .join("\0"),
            animation_name,
            animation_phase_f32_bits: model.pose.animation_phase_f32_bits,
            animation_speed_f32_bits: model.pose.animation_speed_f32_bits,
            pose: model.pose.clone(),
            authored_event_block_count: u32::try_from(state.events.len())?,
            authored_state_events,
            task_force_members: None,
            counter: None,
            world_matrix_f32_bits: draw.world_matrix_f32_bits,
            disclosure,
            visibility_reason,
            selected: context
                .viewer
                .selected_object_ids
                .contains(&candidate.identity.canonical_id()),
            selected_lod_index: draw.selected_lod,
            selected_shape_indices: draw
                .submeshes
                .iter()
                .map(|submesh| submesh.group_index)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            selected_mesh: model.mesh.normalized_game_relative_path.clone(),
            groups: caster.groups,
            skinned: draw.object_draw.render.skinned,
            skin_palette_sha256: model.pose.skin_palette_sha256.clone(),
            skin_palette_3200_bytes: model.pose.skin_palette_3200_bytes.clone(),
            partition_mask: draw.partition_mask,
            casts_light_depth,
            local_aabb_min_f32_bits: min.map(f32::to_bits),
            local_aabb_max_f32_bits: max.map(f32::to_bits),
        };
        published.root_animation(&self.content.resolver, &self.content.admission, root)?;
        let object = Xmap13PublishedModelObject {
            identity: candidate.identity.clone(), root: root.to_owned(), normal_mesh,
            mesh_source: candidate.chain.mesh_source.clone(),
            object_matrix: crate::xmap12_pdxmesh::scaled_world_matrix(
                request.world_matrix_f32_bits, candidate.chain.entity.scale)?,
            authored_mesh_scale: candidate.chain.pdxmesh.scale,
            bounds,
        };
        Ok(Some((model, unit, draw, object)))
    }
}
