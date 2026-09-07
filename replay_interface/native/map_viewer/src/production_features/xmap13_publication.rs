use super::*;
use crate::production_foundation::PreparedEntityManagerOwner;
use crate::xmap09_light_lifecycle::{
    DerivedLightRows, PreparedLightEntityFrame, ViewerLightLifecycle, Xmap09PlacedEntity,
};
use std::sync::{Arc, Mutex, MutexGuard};

pub(super) fn land_model_casts_light_depth(
    model: &crate::PdxModel,
    effects: &crate::EffectFile,
) -> Result<bool> {
    let mut casts = false;
    for mesh in model.shapes.iter().flat_map(|shape| &shape.meshes) {
        if mesh.material.is_collision() {
            continue;
        }
        let shader = if mesh.skin.is_some() {
            format!("{}Skinned", mesh.material.shader)
        } else {
            mesh.material.shader.clone()
        };
        let effect = effects.effect(&shader).ok_or_else(|| {
            format!("XMAP13 whole-mesh shadow classification has no normal effect {shader}")
        })?;
        let rows = crate::xmap12_d3d_handoff::state_rows(
            effect.blend_state.as_deref(), &effects.blend_states, "BlendState");
        let mut blend_enabled = false;
        for (key, value) in rows {
            if key.eq_ignore_ascii_case("BlendEnable") {
                blend_enabled = crate::xmap12_d3d_handoff::yes_no(value, key)?;
            }
        }
        casts |= !blend_enabled;
    }
    Ok(casts)
}

#[derive(Clone)]
pub(super) struct LandModelCandidate {
    pub ordinal: usize,
    pub identity: Xmap13UnitIdentity,
    pub chain: crate::Xmap12ResolvedEntityChain,
    pub state: String,
    pub placement: Xmap13HelperPlacement,
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

    pub(super) fn new(manager: Arc<Mutex<crate::particle_runtime::ViewerEntityManager>>) -> Self {
        Self {
            lifecycle: ViewerLightLifecycle::with_entity_manager(manager),
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
    published: DerivedLightRows,
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
            &self
                .pending
                .as_ref()
                .ok_or("XMAP13 owner already finished")?
                .prepared,
            root,
            random,
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
            published,
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

    pub(super) fn append_finished_land_models(
        &self,
        context: &FoundationFeatureContext<'_>,
        visibility: &crate::PreparedXmap05Visibility,
        prepared: &mut PreparedXmap13Frame,
    ) -> Result<()> {
        let finished = self
            .finished_land
            .lock()
            .map_err(|_| "XMAP13 publication lock poisoned")?;
        let Some(finished) = finished.as_ref() else {
            return Ok(());
        };
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
            let (model, unit, draw) = match result {
                Ok(Some(result)) => result,
                Ok(None) => {
                    if let Some(row) = prepared.element_diagnostics.iter_mut().find(|row| {
                        row.element_id == format!("xmap13:division:source:{}", candidate.ordinal)
                    }) {
                        row.cause_id = "unit-counter-input-unavailable".into();
                        row.detail = "model is outside the current camera partitions; counter grouping/symbol operands remain unavailable".into();
                    }
                    continue;
                }
                Err(error) => {
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
        if !state.events.is_empty() {
            return Err("XMAP13 model state-event projection is not yet bound".into());
        }
        let mesh = self.content.library.shared_mesh(
            &self.content.resolver,
            &self.content.admission,
            &candidate.chain.mesh_source,
        )?;
        let effects = self.content.library.effect_file(
            &self.content.resolver,
            &self.content.admission,
            &candidate.chain.shader_source,
        )?;
        let drawable = mesh.drawable_model()?;
        let casts_light_depth = land_model_casts_light_depth(drawable, &effects)?;
        let bounds = xmap13_lod_bounds(drawable);
        let (min, max) = bounds
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
            authored_event_block_count: 0,
            authored_state_events: Vec::new(),
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
        Ok(Some((model, unit, draw)))
    }
}
