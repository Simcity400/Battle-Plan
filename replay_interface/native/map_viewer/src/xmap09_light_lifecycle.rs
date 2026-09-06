use crate::particle_authored::ParticleDefinitions;
use crate::particle_runtime::{
    ParticleAction, ParticleHandle, ParticleInstanceUpdate, ParticleLightBinding, ParticleRuntime,
    ViewerParticleSeed,
};
use crate::production_features::xmap13_pose::{
    xmap13_clip_length_seconds, xmap13_model_skeleton, xmap13_node_transforms,
};
use crate::xmap09_light_census::*;
use crate::xmap09_light_template::{
    multiply_4x4, parse_light_templates, Xmap09LightInstanceClock, Xmap09LightTemplate,
};
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap09PlacedEntity {
    pub id: String,
    pub entity_token: String,
    pub state: Option<String>,
    pub animate: bool,
    pub world_matrix_f32_bits: [u32; 16],
}

fn missing(domain: impl Into<String>) -> Xmap09LightCensusRefusal {
    Xmap09LightCensusRefusal::MissingDomain {
        domain: domain.into(),
    }
}

pub fn placed_light_entities(
    cities: &PreparedXmap12CityFrame,
    buildings: &PreparedXmap12BuildingFrame,
    fixed: &PreparedXmap12FixedFrame,
    visual: &Xmap12BuildingVisualFrame,
) -> std::result::Result<Vec<Xmap09PlacedEntity>, Xmap09LightCensusRefusal> {
    let mut rows = Vec::new();
    for row in &cities.complete_population {
        match row.disposition {
            Xmap12CityDisposition::MissingEntity => continue,
            Xmap12CityDisposition::ExcludedUnvouched => {
                return Err(missing(format!(
                    "XMAP12.placed_entities.{}",
                    row.stable_typed_id
                )))
            }
            _ => {}
        }
        rows.push(Xmap09PlacedEntity {
            id: row.stable_typed_id.clone(),
            entity_token: row
                .entity_token
                .clone()
                .ok_or_else(|| missing(format!("XMAP12.entity.{}", row.stable_typed_id)))?,
            state: None,
            animate: true,
            world_matrix_f32_bits: row.world_matrix_f32_bits,
        });
    }
    for row in &buildings.complete_population {
        match row.disposition {
            Xmap12BuildingDisposition::HiddenByLevel => continue,
            Xmap12BuildingDisposition::ExcludedUnvouched => {
                return Err(missing(format!(
                    "XMAP12.placed_entities.{}",
                    row.stable_typed_id
                )))
            }
            _ => {}
        }
        let selected = visual
            .rows
            .iter()
            .find(|v| v.source_ordinal == row.source_ordinal)
            .filter(|v| !v.selected_entity_token.is_empty())
            .ok_or_else(|| missing(format!("XMAP12.entity.{}", row.stable_typed_id)))?;
        rows.push(Xmap09PlacedEntity {
            id: row.stable_typed_id.clone(),
            entity_token: selected.selected_entity_token.clone(),
            state: None,
            animate: true,
            world_matrix_f32_bits: row.world_matrix_f32_bits,
        });
    }
    for row in &fixed.complete_population {
        match row.disposition {
            Xmap12FixedDisposition::InactiveLifecycle => continue,
            Xmap12FixedDisposition::ExcludedUnvouched => {
                return Err(missing(format!(
                    "XMAP12.placed_entities.{}",
                    row.stable_typed_id
                )))
            }
            _ => {}
        }
        rows.push(Xmap09PlacedEntity {
            id: row.stable_typed_id.clone(),
            entity_token: row.entity_token.clone(),
            state: row.use_animation.then(|| row.animation_name.clone()),
            animate: row.use_animation,
            world_matrix_f32_bits: row.world_matrix_f32_bits,
        });
    }
    Ok(rows)
}

struct Assets {
    models: Mutex<BTreeMap<String, Arc<PdxModel>>>,
    animations: Mutex<BTreeMap<String, Arc<PdxAnimData>>>,
    poses: BTreeMap<String, BuildingEntityPose>,
    static_attachments: BTreeSet<String>,
    meshes: BTreeMap<String, BuildingPdxmeshBinding>,
    mesh_clips: BTreeMap<String, BTreeMap<String, String>>,
    clips: BTreeMap<String, String>,
    templates: Vec<Xmap09LightTemplate>,
}

impl Assets {
    fn model(&self, resolver: &GameFileResolver, path: &str) -> Result<Arc<PdxModel>> {
        let mut cache = self
            .models
            .lock()
            .map_err(|_| "light mesh cache is poisoned")?;
        if let Some(model) = cache.get(path) {
            return Ok(model.clone());
        }
        let model = Arc::new(extract_pdx_model(&parse_pdx_asset(&read_asset(
            resolver, path,
        )?)?)?);
        cache.insert(path.into(), model.clone());
        Ok(model)
    }

    fn animation(&self, resolver: &GameFileResolver, path: &str) -> Result<Arc<PdxAnimData>> {
        let mut cache = self
            .animations
            .lock()
            .map_err(|_| "light clip cache is poisoned")?;
        if let Some(clip) = cache.get(path) {
            return Ok(clip.clone());
        }
        let clip = Arc::new(extract_pdx_anim(&parse_pdx_asset(&read_asset(
            resolver, path,
        )?)?)?);
        cache.insert(path.into(), clip.clone());
        Ok(clip)
    }

    fn load(resolver: &GameFileResolver) -> Result<Self> {
        let mut text = String::new();
        let mut clips = BTreeMap::new();
        let mut static_attachments = BTreeSet::new();
        for extension in ["asset", "gfx"] {
            for key in resolver.require_complete_admitted_dir(
                "",
                Some(extension),
                true,
                "XMAP09 asset events",
            )? {
                let source = latin1_bytes_to_string(&resolver.read_admitted_used_content(&key)?);
                let (document, source) = if extension == "asset" {
                    portable_content_manifest::paradox_text::parse_asset_str(&source)
                        .map_err(|e| format!("{}: {e:?}", key.normalized_game_relative_path))?
                } else {
                    (portable_content_manifest::paradox_text::parse_str_strict(&source)
                        .map_err(|e| format!("{}: {e:?}", key.normalized_game_relative_path))?,
                        std::borrow::Cow::Borrowed(source.as_str()))
                };
                for entity in document.values_ci("entity") {
                    if entity.first_value_ci("attach").is_some() {
                        if let Some(name) = entity.first_scalar_ci("name") {
                            static_attachments.insert(name.to_owned());
                        }
                    }
                }
                for (name, file) in
                    parse_building_animation_bindings(&source, &key.normalized_game_relative_path)?
                {
                    clips.insert(
                        name,
                        crate::production_features::xmap13_pose::authored_clip_logical_path(
                            &key.normalized_game_relative_path,
                            &file,
                        ),
                    );
                }
                text.push_str(&source);
                text.push('\n');
            }
        }
        Ok(Self {
            models: Mutex::new(BTreeMap::new()),
            animations: Mutex::new(BTreeMap::new()),
            poses: parse_building_entity_poses(&text, "authenticated asset namespace")?
                .into_iter()
                .map(|p| (p.entity_name.clone(), p))
                .collect(),
            meshes: parse_building_pdxmesh_bindings(&text, "authenticated asset namespace")?
                .into_iter()
                .map(|m| (m.name.clone(), m))
                .collect(),
            mesh_clips: parse_pdxmesh_animation_bindings(&text, "authenticated asset namespace")?,
            clips,
            static_attachments,
            templates: parse_light_templates(&text, "authenticated asset namespace")?,
        })
    }
}

#[derive(Clone)]
struct EntityAnimation {
    include_particle_only_events: bool,
    next_state: Option<String>,
    authored_speed: f64,
    scale: f32,
    token: String,
    requested_state: Option<String>,
    events: Arc<Vec<BuildingEntityEvent>>,
    speed: f64,
    visible: bool,
    length: f64,
    looping: bool,
    elapsed: f64,
    playhead: f64,
    model: Option<Arc<PdxModel>>,
    clip: Option<Arc<PdxAnimData>>,
}

struct PairedParticle {
    runtime: ParticleRuntime,
    handle: ParticleHandle,
    binding: ParticleLightBinding,
}

struct FiredInstance {
    authored_event_index: usize,
    ordinal: u64,
    stop_unbounded: bool,
    particle: Option<PairedParticle>,
    id: String,
    entity_id: String,
    event: BuildingEntityEvent,
    clock: Xmap09LightInstanceClock,
}

const IDENTITY: [f32; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];

#[derive(Clone)]
struct ChildEntity {
    parent: String,
    event: BuildingEntityEvent,
    life: f32,
    ordinal: u64,
}

#[derive(Default)]
pub(crate) struct ViewerLightLifecycle {
    include_particle_only_events: bool,
    assets: Option<Arc<Assets>>,
    particle_definitions: Option<ParticleDefinitions>,
    content_lease: Option<crate::mod_resolver::ContentAdmissionLease>,
    asset_refusal: Option<Xmap09LightCensusRefusal>,
    entities: BTreeMap<String, EntityAnimation>,
    children: BTreeMap<String, ChildEntity>,
    instances: Vec<FiredInstance>,
    next_instance: u64,
    next_particle_only_instance: u64,
    last_presentation_time: Option<f32>,
    publication_refusal: Option<Xmap09LightCensusRefusal>,
    weather_binding: Option<(u32, String)>,
}

pub(crate) struct DerivedLightRows {
    entities: Vec<(String, Arc<Vec<BuildingEntityEvent>>, [f32; 16])>,
    entity_indices: BTreeMap<String, usize>,
    instances: Vec<(String, String, usize, Xmap09LightInstanceClock)>,
    particle_bindings: BTreeMap<String, ParticleLightBinding>,
    pub(crate) authored_particles: Vec<crate::production_features::xmap20::Xmap20AuthoredParticleBinding>,
    poses: BTreeMap<String, EntityAnimation>,
    children: BTreeMap<String, ChildEntity>,
    assets: Arc<Assets>,
}

impl DerivedLightRows {
    fn world(&self, id: &str) -> Result<[f32; 16]> {
        if let Some(child) = self.children.get(id) {
            let parent = self.world(&child.parent)?;
            let attach = self.poses[&child.parent].attach_event(&child.event)?;
            let mut scale = IDENTITY;
            for i in [0, 5, 10] {
                scale[i] = self.poses[id].scale;
            }
            Ok(multiply_4x4(multiply_4x4(parent, attach), scale))
        } else {
            self.entity_indices
                .get(id)
                .map(|index| self.entities[*index].2)
                .ok_or_else(|| format!("placed entity {id} is unavailable").into())
        }
    }

    pub(crate) fn with_rows<T>(&self, consumer: impl FnOnce(Xmap09LightCensusRows<'_>) -> T) -> T {
        let entities = self
            .entities
            .iter()
            .map(|(id, events, _)| Xmap09LightEntity {
                id,
                scene_mask: 6,
                events,
                placement: Some(IDENTITY),
            })
            .collect::<Vec<_>>();
        let instances = self
            .instances
            .iter()
            .map(
                |(id, entity_id, event_index, clock)| Xmap09FiredLightInstance {
                    id,
                    entity_id,
                    event_index: *event_index,
                    creator_rva: 0x0227_6040,
                    clock: Some(clock),
                    attach_transform: None,
                    paired_particle: self.particle_bindings.get(id).copied(),
                    enabled: Some(true),
                },
            )
            .collect::<Vec<_>>();
        let resolve_attach = |instance: &Xmap09FiredLightInstance<'_>| {
            self.world(instance.entity_id)
                .and_then(|world| {
                    Ok(multiply_4x4(
                        world,
                        self.poses[instance.entity_id].attach_event(
                            &self.entities[self.entity_indices[instance.entity_id]].1
                                [instance.event_index],
                        )?,
                    ))
                })
                .map_err(|error| missing(format!("assets.light_attach.{}: {error}", instance.id)))
        };
        consumer(Xmap09LightCensusRows {
            resolve_attach: Some(&resolve_attach),
            entities: &entities,
            templates: &self.assets.templates,
            fired_instances: &instances,
        })
    }
}

fn read_asset(resolver: &GameFileResolver, path: &str) -> Result<Vec<u8>> {
    let key = resolver
        .admitted_used_content_key(path)?
        .ok_or_else(|| format!("asset not admitted: {path}"))?;
    resolver.read_admitted_used_content(&key)
}

impl EntityAnimation {
    fn load(
        row: &Xmap09PlacedEntity,
        assets: &Assets,
        resolver: &GameFileResolver,
        require_pose: bool,
        include_particle_only_events: bool,
    ) -> Result<Self> {
        if assets.static_attachments.contains(&row.entity_token) {
            return Err(format!("entity {} static attachment selection/phase is unbound", row.entity_token).into());
        }
        let pose = assets
            .poses
            .get(&row.entity_token)
            .ok_or_else(|| format!("entity {} is unavailable", row.entity_token))?;
        let requested = row
            .state
            .as_deref()
            .or(pose.default_state.as_deref())
            .unwrap_or("idle");
        let state = match pose.state_choice(requested) {
            EntityStateChoice::One(state) => Some(state),
            EntityStateChoice::Undeclared => None,
            EntityStateChoice::Weighted(variants) => {
                use crate::production_features::xmap13_pose::{
                    xmap13_pose_seed, xmap13_variant_unit, xmap13_weighted_pick,
                };
                let weights = variants
                    .iter()
                    .map(|s| s.chance.unwrap())
                    .collect::<Vec<_>>();
                let index = xmap13_weighted_pick(
                    xmap13_variant_unit(xmap13_pose_seed(&row.id), requested),
                    &weights,
                )
                .ok_or("light state variant weights are invalid")?;
                Some(variants[index])
            }
            _ => {
                return Err(format!(
                    "entity {} state {requested} selection is unavailable",
                    row.entity_token
                )
                .into())
            }
        };
        let events = state.map(|s| s.events.clone()).unwrap_or_default();
        if events.iter().any(|e| {
            (e.light.is_some() || e.entity.is_some() || (include_particle_only_events && e.particle.is_some()))
                && (e.time.is_none_or(|t| !t.is_finite() || t < 0.0)
                    || e.life.is_some_and(|t| !t.is_finite()))
        }) {
            return Err("entity event clock is invalid".into());
        }
        let has_lights = events
            .iter()
            .any(|e| e.light.is_some() || e.entity.is_some() || (include_particle_only_events && e.particle.is_some()));
        let entity_scale = pose.scale.unwrap_or(1.0);
        let mesh_scale = match pose.pdxmesh.as_ref() {
            Some(name) => {
                assets
                    .meshes
                    .get(name)
                    .ok_or("pdxmesh scale is unavailable")?
                    .scale
            }
            None => 1.0,
        };
        let [mut start, range] = state.and_then(|s| s.time_offset).unwrap_or([0.0, 0.0]);
        if range > 0.0 {
            use crate::production_features::xmap13_pose::{xmap13_pose_seed, xmap13_variant_unit};
            let draw = (xmap13_variant_unit(
                xmap13_pose_seed(&row.id),
                &format!("{requested}:time_offset"),
            ) * 2147483648.0) as u32;
            start += draw as f32 * f32::from_bits(0x30000000) * range;
        }
        let mut out = Self {
            include_particle_only_events,
            next_state: state.and_then(|s| s.next_state.clone()),
            authored_speed: state.and_then(|s| s.animation_speed).unwrap_or(1.0) as f64,
            scale: entity_scale * mesh_scale,
            token: row.entity_token.clone(),
            requested_state: row.state.clone(),
            events: Arc::new(events),
            speed: 1.0,
            visible: true,
            length: state.and_then(|s| s.length).unwrap_or(-1.0) as f64,
            looping: state.and_then(|s| s.looping).unwrap_or(true),
            elapsed: 0.0,
            playhead: start as f64,
            model: None,
            clip: None,
        };
        if !out.scale.is_finite() || out.scale <= 0.0 {
            return Err("entity scale is invalid".into());
        }
        if state.and_then(|s| s.animation.as_ref()).is_none() && out.length < 0.0 {
            out.looping = false;
        }
        if !has_lights && !require_pose && out.next_state.is_none() {
            return Ok(out);
        }
        if let Some(mesh_name) = pose.pdxmesh.as_ref() {
            let mesh = assets
                .meshes
                .get(mesh_name)
                .ok_or("light pdxmesh binding is unavailable")?;
            out.model = Some(assets.model(resolver, &mesh.mesh)?);
        }
        if let Some(animation) = state.and_then(|s| s.animation.as_ref()) {
            let mesh_name = pose
                .pdxmesh
                .as_ref()
                .ok_or("animated entity has no pdxmesh")?;
            let clip_name = assets
                .mesh_clips
                .get(mesh_name)
                .and_then(|c| c.get(animation))
                .ok_or("light animation binding is unavailable")?;
            let file = assets
                .clips
                .get(clip_name)
                .ok_or("light animation file is unavailable")?;
            let clip = assets.animation(resolver, file)?;
            out.length = xmap13_clip_length_seconds(&clip) as f64;
            out.clip = Some(clip);
        } else if out.length < 0.0 {
            out.looping = false;
        }
        if !out.authored_speed.is_finite()
            || !out.speed.is_finite()
            || out.speed < 0.0
            || !out.length.is_finite()
            || (out.looping && out.length <= 0.0)
        {
            return Err("light entity animation clock is invalid".into());
        }
        Ok(out)
    }

    fn advance(&mut self, delta: f64) -> Vec<usize> {
        let previous = self.playhead;
        if self.visible {
            self.elapsed += delta;
        }
        self.playhead += delta
            * self.speed
            * if self.clip.is_some() {
                self.authored_speed
            } else {
                1.0
            };
        if self.looping {
            self.playhead %= self.length;
            if self.playhead < 0.0 {
                self.playhead += self.length;
            }
        } else if self.clip.is_some() {
            self.playhead = self.playhead.clamp(0.0, self.length);
        }
        if !self.visible {
            return Vec::new();
        }
        self.events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| {
                let time = event.time? as f64;
                if event.light.is_none() && event.entity.is_none()
                    && !(self.include_particle_only_events && event.particle.is_some())
                    || self.speed <= 0.0
                    || previous == self.playhead
                    || (event.trigger_once
                        && self.elapsed > self.length
                        && !(self.clip.is_some() && !self.looping))
                {
                    return None;
                }
                let crossed = if self.playhead < previous {
                    previous <= time || time < self.playhead
                } else {
                    previous <= time && time < self.playhead
                };
                crossed.then_some(index)
            })
            .collect()
    }

    fn attach(&self, event: usize) -> Result<[f32; 16]> {
        self.attach_event(&self.events[event])
    }

    fn attach_event(&self, event: &BuildingEntityEvent) -> Result<[f32; 16]> {
        if event.node.as_deref().is_none_or(str::is_empty) {
            return Ok([
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ]);
        }
        let model = self
            .model
            .as_ref()
            .ok_or("light attach mesh is unavailable")?;
        let skeleton = xmap13_model_skeleton(model.shapes.iter().map(|s| s.skeleton.as_slice()))?;
        let transforms =
            xmap13_node_transforms(skeleton, self.clip.as_deref(), self.playhead as f32)?;
        let node = event
            .node
            .as_deref()
            .ok_or("light attach node is unavailable")?;
        xmap09_attach_node_object_transform(node, &model.locators, &|name| {
            transforms.get(name).copied()
        })
        .ok_or_else(|| format!("light attach node {node} is unavailable").into())
    }
}

impl ViewerLightLifecycle {
    pub(crate) fn visit_particles_for_draw(
        &mut self,
        presentation_time: f32,
        camera: crate::particle_runtime::ParticleSortCamera,
        visit: &mut dyn FnMut(&str, &crate::particle_authored::ParticleDrawCapacity, crate::particle_runtime::ParticleGpuView<'_>) -> Result<()>,
    ) -> Result<()> {
        if let Some(refusal) = &self.publication_refusal {
            return Err(format!("PARTICLE_DRAW lifecycle update failed; reset_population required: {refusal:?}").into());
        }
        if self.last_presentation_time.map(f32::to_bits) != Some(presentation_time.to_bits()) {
            return Err("PARTICLE_DRAW retained lifecycle belongs to another presentation".into());
        }
        let presentation_id = presentation_time.to_bits() as u64;
        let capacity = self.particle_definitions.as_mut().map(ParticleDefinitions::draw_capacity).transpose()?;
        for instance in &mut self.instances {
            let Some(particle) = &mut instance.particle else { continue; };
            particle.runtime.sort_for_draw(particle.handle, presentation_id, camera)?;
            visit(&instance.id, capacity.as_ref().ok_or("PARTICLE_DRAW catalog disappeared")?, particle.runtime.gpu_view(particle.handle)?)?;
        }
        Ok(())
    }

    pub(crate) fn for_authored_particles() -> Self {
        Self { include_particle_only_events: true, ..Default::default() }
    }

    fn entity_update_reached<'a>(&'a self, mut id: &'a str) -> bool {
        while let Some(child) = self.children.get(id) {
            id = &child.parent;
            if !self.entities[id].visible {
                return false;
            }
        }
        true
    }

    fn leave_state(&mut self, id: &str) {
        self.instances
            .retain(|i| i.entity_id != id || i.event.keep_particle);
        for instance in &mut self.instances {
            if instance.entity_id == id {
                instance.stop_unbounded = true;
            }
        }
        let removed = self
            .children
            .iter()
            .filter(|(_, child)| child.parent == id && child.life < 0.0)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for child in removed {
            self.remove_entity(&child);
        }
    }

    fn remove_entity(&mut self, id: &str) {
        let children = self
            .children
            .iter()
            .filter(|(_, c)| c.parent == id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for child in children {
            self.remove_entity(&child);
        }
        self.children.remove(id);
        self.entities.remove(id);
        self.instances.retain(|i| i.entity_id != id);
    }

    fn advance_entity(
        &mut self,
        id: &str,
        delta: f64,
        resolver: &GameFileResolver,
        assets: &Assets,
        ancestry: &mut Vec<String>,
    ) -> Result<()> {
        let existing = self
            .children
            .iter()
            .filter(|(_, c)| c.parent == id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for child in existing {
            let life = &mut self.children.get_mut(&child).unwrap().life;
            if *life >= 0.0 {
                *life -= delta as f32;
                if *life < 0.0 {
                    self.remove_entity(&child);
                }
            }
        }
        let fired = self.entities.get_mut(id).unwrap().advance(delta);
        if !self.entities[id].visible {
            return Ok(());
        }
        for event_index in fired {
            let state = &self.entities[id];
            let event = state.events[event_index].clone();
            let speed = state.speed;
            if event.light.is_some() || (self.include_particle_only_events && event.particle.is_some()) {
                let (ordinal, instance_id) = if event.light.is_some() {
                    let ordinal = self.next_instance;
                    if ordinal >= 1u64 << 63 {
                        return Err("viewer light/entity instance identity exhausted".into());
                    }
                    self.next_instance += 1;
                    (ordinal, format!("{id}:light:{event_index}:{ordinal}"))
                } else {
                    let local = self.next_particle_only_instance;
                    if local >= 1u64 << 63 {
                        return Err("viewer particle-only instance identity exhausted".into());
                    }
                    self.next_particle_only_instance += 1;
                    (local | (1u64 << 63), format!("{id}:particle:{event_index}:{local}"))
                };
                self.instances.push(FiredInstance {
                    authored_event_index: event_index,
                    ordinal,
                    stop_unbounded: false,
                    particle: None,
                    id: instance_id,
                    entity_id: id.into(),
                    event: event.clone(),
                    clock: Xmap09LightInstanceClock::default(),
                });
            }
            if let Some(token) = event.entity.clone() {
                if ancestry.contains(&token) {
                    return Err(format!(
                        "recursive same-frame entity event: {ancestry:?} -> {token}"
                    )
                    .into());
                }
                let life = event.life.unwrap_or(-1.0);
                if !life.is_finite() {
                    return Err(format!("entity event life is invalid: {id}:{event_index}").into());
                }
                let child_id = format!("{id}:entity:{event_index}:{}", self.next_instance);
                let row = Xmap09PlacedEntity {
                    id: child_id.clone(),
                    entity_token: token,
                    state: None,
                    animate: true,
                    world_matrix_f32_bits: IDENTITY.map(f32::to_bits),
                };
                let mut child = EntityAnimation::load(&row, assets, resolver, false, self.include_particle_only_events)?;
                child.speed = speed;
                self.entities.insert(child_id.clone(), child);
                self.children.insert(
                    child_id,
                    ChildEntity {
                        parent: id.into(),
                        event: event.clone(),
                        life,
                        ordinal: self.next_instance,
                    },
                );
                self.next_instance += 1;
            }
        }
        let mut children = self
            .children
            .iter()
            .filter(|(_, c)| c.parent == id)
            .map(|(id, c)| (c.ordinal, id.clone()))
            .collect::<Vec<_>>();
        children.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, child) in children {
            ancestry.push(self.entities[&child].token.clone());
            self.advance_entity(&child, delta, resolver, assets, ancestry)?;
            ancestry.pop();
        }
        let state = &self.entities[id];
        if !state.looping && state.playhead >= state.length {
            if let Some(next) = state.next_state.clone() {
                if !assets.poses[&state.token]
                    .states
                    .iter()
                    .any(|s| s.name == next)
                {
                    return Err(
                        format!("entity {} next_state {next} is unavailable", state.token).into(),
                    );
                }
                let speed = state.speed;
                let requested_state = state.requested_state.clone();
                let row = Xmap09PlacedEntity {
                    id: format!("{id}:state:{}", self.next_instance),
                    entity_token: state.token.clone(),
                    state: Some(next),
                    animate: true,
                    world_matrix_f32_bits: IDENTITY.map(f32::to_bits),
                };
                self.next_instance += 1;
                let require_pose = self
                    .instances
                    .iter()
                    .any(|i| i.entity_id == id && i.event.keep_particle)
                    || self
                        .children
                        .values()
                        .any(|c| c.parent == id && c.life >= 0.0);
                let mut next = EntityAnimation::load(&row, assets, resolver, require_pose, self.include_particle_only_events)?;
                next.speed = speed;
                next.requested_state = requested_state;
                self.leave_state(id);
                self.entities.insert(id.into(), next);
            }
        }
        Ok(())
    }

    pub(crate) fn weather_particles_bound(&self, time: f32, identity: &str) -> bool {
        self.weather_binding.as_ref().is_some_and(|(bits, hash)| *bits == time.to_bits() && hash == identity)
    }

    pub(crate) fn reset_population(&mut self) {
        self.entities.clear();
        self.children.clear();
        self.instances.clear();
        self.next_instance = 0;
        self.next_particle_only_instance = 0;
        self.last_presentation_time = None;
        self.publication_refusal = None;
        self.weather_binding = None;
    }

    pub(crate) fn derive_light_instance_rows(
        &mut self,
        resolver: &GameFileResolver,
        placed: &[Xmap09PlacedEntity],
        viewer: &ViewerOwnedMapState,
        weather: Option<&crate::PreparedXmap19WeatherFrame>,
    ) -> std::result::Result<DerivedLightRows, Xmap09LightCensusRefusal> {
        if let Some(refusal) = &self.publication_refusal {
            return Err(refusal.clone());
        }
        self.weather_binding = None;
        match self.update_light_instance_rows(resolver, placed, viewer, weather) {
            Ok(rows) => {
                let now = f32::from_bits(viewer.animation_time_f32_bits);
                self.last_presentation_time = Some(now);
                self.weather_binding = weather.map(|frame| (now.to_bits(), frame.deterministic_sha256.clone()));
                Ok(rows)
            }
            Err(refusal) => {
                // Event clocks and native particle columns may already have advanced.
                self.last_presentation_time = None;
                self.weather_binding = None;
                self.publication_refusal = Some(refusal.clone());
                Err(refusal)
            }
        }
    }

    fn update_light_instance_rows(
        &mut self,
        resolver: &GameFileResolver,
        placed: &[Xmap09PlacedEntity],
        viewer: &ViewerOwnedMapState,
        weather: Option<&crate::PreparedXmap19WeatherFrame>,
    ) -> std::result::Result<DerivedLightRows, Xmap09LightCensusRefusal> {
        let mut population = std::borrow::Cow::Borrowed(placed);
        let mut controls = Vec::new();
        if let Some(weather) = weather {
            weather.validate().map_err(|e| missing(format!("XMAP19.weather_control: {e}")))?;
            let rotation = crate::particle_runtime::native_axis_matrix(None, [0.0, 1.0, 0.0], -0.0)
                .map_err(|e| missing(format!("XMAP19.weather_transform: {e}")))?;
            for position in &weather.ordered_positions {
                let Some(selection) = &position.selection else { continue };
                let mut world = IDENTITY;
                for row in 0..3 {
                    for column in 0..3 {
                        world[row * 4 + column] = rotation[row * 3 + column] * 1.0;
                    }
                }
                for axis in 0..3 {
                    world[12 + axis] = f32::from_bits(position.position_xyz_f32_bits[axis]);
                }
                for (id, program) in std::iter::once((&selection.primary_source_id, &selection.primary))
                    .chain(selection.fading_source_id.as_ref().zip(selection.fading.as_ref()))
                {
                    population.to_mut().push(Xmap09PlacedEntity {
                        id: id.clone(), entity_token: program.entity_name.clone(),
                        state: Some(program.default_state_name.clone()), animate: true,
                        world_matrix_f32_bits: world.map(f32::to_bits),
                    });
                }
                if let Some(id) = &selection.fading_source_id {
                    controls.push((id.as_str(), Some(f32::from_bits(weather.fading_visibility_alpha_f32_bits))));
                }
                controls.push((selection.primary_source_id.as_str(),
                    weather.primary_control_written.then(|| f32::from_bits(weather.primary_visibility_alpha_f32_bits))));
            }
        }
        let placed = population.as_ref();
        let now = f32::from_bits(viewer.animation_time_f32_bits);
        let delta = viewer.frame_delta_seconds();
        if !now.is_finite() || now < 0.0 || !delta.is_finite() || delta < 0.0 {
            return Err(missing("viewer.renderer_clock"));
        }
        let lease = resolver.content_admission_lease();
        if self.content_lease != lease {
            self.reset_population();
            self.assets = None;
            self.asset_refusal = None;
            self.particle_definitions = None;
            self.content_lease = lease;
        }
        if self.last_presentation_time.is_some_and(|last| now < last) {
            self.reset_population();
        }
        let advance = self.last_presentation_time != Some(now);
        if let Some(refusal) = &self.asset_refusal {
            return Err(refusal.clone());
        }
        if self.assets.is_none() {
            match Assets::load(resolver) {
                Ok(assets) => self.assets = Some(Arc::new(assets)),
                Err(error) => {
                    let refusal = missing(format!("assets.light_events: {error}"));
                    self.asset_refusal = Some(refusal.clone());
                    return Err(refusal);
                }
            }
        }
        let assets = self.assets.as_ref().unwrap().clone();
        let ids = placed
            .iter()
            .map(|r| r.id.as_str())
            .collect::<BTreeSet<_>>();
        if ids.len() != placed.len() {
            return Err(Xmap09LightCensusRefusal::InvalidJoin {
                instance: "XMAP12.placed_entities".into(),
            });
        }
        let removed = self
            .entities
            .keys()
            .filter(|id| !self.children.contains_key(*id) && !ids.contains(id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        for id in removed {
            self.remove_entity(&id);
        }
        for row in placed {
            if self
                .entities
                .get(&row.id)
                .is_none_or(|e| e.token != row.entity_token || e.requested_state != row.state)
            {
                let same_entity = self
                    .entities
                    .get(&row.id)
                    .is_some_and(|e| e.token == row.entity_token);
                let require_pose = same_entity
                    && (self
                        .instances
                        .iter()
                        .any(|i| i.entity_id == row.id && i.event.keep_particle)
                        || self
                            .children
                            .values()
                            .any(|c| c.parent == row.id && c.life >= 0.0));
                let state =
                    EntityAnimation::load(row, &assets, resolver, require_pose, self.include_particle_only_events).map_err(|e| {
                        missing(format!("assets.light_events.{}: {e}", row.entity_token))
                    })?;
                if same_entity {
                    self.leave_state(&row.id);
                } else {
                    self.remove_entity(&row.id);
                }
                self.entities.insert(row.id.clone(), state);
            }
        }
        if let Some(weather) = weather {
            for (id, alpha) in &controls {
                if let Some(alpha) = alpha {
                    self.entities.get_mut(*id).unwrap().visible = *alpha > 0.0;
                    for instance in self.instances.iter_mut().filter(|instance| instance.entity_id == *id) {
                        if let Some(particle) = &mut instance.particle {
                            let refusal = |refusal| Xmap09LightCensusRefusal::ParticleInput {
                                instance: instance.id.clone(), refusal,
                            };
                            particle.runtime.set_visible(particle.handle, *alpha > 0.0).map_err(refusal)?;
                            if *alpha > 0.0 {
                                particle.runtime.set_opacity(particle.handle, *alpha).map_err(refusal)?;
                            }
                        }
                    }
                }
            }
            for (id, _) in &controls {
                self.entities.get_mut(*id).unwrap().speed =
                    f64::from(f32::from_bits(weather.playback_rate_f32_bits));
            }
        }
        if advance {
            let mut index = 0;
            while index < self.instances.len() {
                let instance = &self.instances[index];
                if !self.entity_update_reached(&instance.entity_id) {
                    index += 1;
                    continue;
                }
                let finished = instance
                    .particle
                    .as_ref()
                    .map(|particle| {
                        particle
                            .runtime
                            .light_binding(particle.handle)
                            .map(|binding| binding.finished)
                            .map_err(|refusal| Xmap09LightCensusRefusal::ParticleInput {
                                instance: instance.id.clone(),
                                refusal,
                            })
                    })
                    .transpose()?
                    .unwrap_or(false);
                if finished {
                    self.instances.swap_remove(index);
                } else {
                    index += 1;
                }
            }
            let updated = self.entities.iter()
                .filter(|(id, state)| state.visible && self.entity_update_reached(id))
                .map(|(id, _)| id.clone()).collect::<BTreeSet<_>>();
            for instance in self.instances.iter_mut().filter(|instance| updated.contains(&instance.entity_id)) {
                instance
                    .clock
                    .advance(delta as f64, true)
                    .map_err(|e| missing(e.to_string()))?;
            }
            for row in placed {
                if !row.animate {
                    continue;
                }
                self.advance_entity(
                    &row.id,
                    delta as f64,
                    resolver,
                    &assets,
                    &mut vec![row.entity_token.clone()],
                )
                .map_err(|e| missing(format!("assets.light_events.{}: {e}", row.entity_token)))?;
            }
        }
        let mut entities = placed
            .iter()
            .map(|row| {
                let state = &self.entities[&row.id];
                let world = crate::xmap12_pdxmesh::scaled_world_matrix(
                    row.world_matrix_f32_bits,
                    state.scale,
                )
                .map_err(|_| Xmap09LightCensusRefusal::InvalidValue {
                    instance: row.id.clone(),
                })?;
                Ok((
                    row.id.clone(),
                    state.events.clone(),
                    std::array::from_fn(|i| world[(i % 4) * 4 + i / 4]),
                ))
            })
            .collect::<std::result::Result<Vec<_>, Xmap09LightCensusRefusal>>()?;
        entities.extend(
            self.children
                .keys()
                .map(|id| (id.clone(), self.entities[id].events.clone(), IDENTITY)),
        );
        let entity_indices = entities
            .iter()
            .enumerate()
            .map(|(index, row)| (row.0.clone(), index))
            .collect::<BTreeMap<_, _>>();
        let instances = self
            .instances
            .iter()
            .filter(|i| i.event.light.is_some())
            .map(|i| {
                let events = &mut entities[entity_indices[&i.entity_id]].1;
                let index = events
                    .iter()
                    .position(|e| *e == i.event)
                    .unwrap_or_else(|| {
                        Arc::make_mut(events).push(i.event.clone());
                        events.len() - 1
                    });
                (i.id.clone(), i.entity_id.clone(), index, i.clock)
            })
            .collect();
        let poses = self.entities.clone();
        let mut rows = DerivedLightRows {
            entities,
            entity_indices,
            instances,
            poses,
            children: self.children.clone(),
            assets,
            particle_bindings: BTreeMap::new(),
            authored_particles: Vec::new(),
        };
        for instance in &mut self.instances {
            let Some(name) = instance.event.particle.as_deref().filter(|s| !s.is_empty()) else {
                continue;
            };
            let refusal = |refusal: crate::particle_authored::ParticleInputRefusal| {
                Xmap09LightCensusRefusal::ParticleInput {
                    instance: instance.id.clone(),
                    refusal: refusal.event(instance.ordinal),
                }
            };
            let mut ancestor = instance.entity_id.as_str();
            let mut attachment_updates = rows.poses[ancestor].visible;
            while let Some(child) = rows.children.get(ancestor) {
                ancestor = &child.parent;
                attachment_updates &= rows.poses[ancestor].visible;
            }
            let frozen_world = if !attachment_updates {
                instance.particle.as_ref().map(|particle| particle.binding.world_matrix_f32_bits)
            } else {
                None
            };
            let world = if let Some(bits) = frozen_world {
                Ok(std::array::from_fn(|i| f32::from_bits(bits[(i % 4) * 4 + i / 4])))
            } else {
                rows
                .world(&instance.entity_id)
                .and_then(|world| {
                    Ok(multiply_4x4(
                        world,
                        rows.poses[&instance.entity_id].attach_event(&instance.event)?,
                    ))
                })
            }.map_err(|e| missing(format!("assets.light_attach.{}: {e}", instance.id)))?;
            let bits = std::array::from_fn(|i| world[(i % 4) * 4 + i / 4].to_bits());
            let speed = rows.poses[&instance.entity_id].speed as f32;
            if instance.particle.is_none() {
                if self.particle_definitions.is_none() {
                    self.particle_definitions =
                        Some(ParticleDefinitions::load(resolver).map_err(refusal)?);
                }
                let definitions = self.particle_definitions.as_mut().unwrap();
                let definition = definitions.resolve(name).map_err(refusal)?;
                let mut runtime =
                    ParticleRuntime::new(definitions.content_lease(), 1).map_err(refusal)?;
                let seed = crate::production_features::xmap13_pose::xmap13_pose_seed(&instance.id);
                let handle = runtime
                    .spawn(
                        instance.ordinal,
                        definition,
                        ViewerParticleSeed {
                            counter: 0,
                            seed: seed as u32 ^ (seed >> 32) as u32,
                        },
                        bits,
                        speed,
                    )
                    .map_err(refusal)?;
                let binding = runtime.light_binding(handle).map_err(refusal)?;
                instance.particle = Some(PairedParticle {
                    runtime,
                    handle,
                    binding,
                });
            }
            let particle = instance.particle.as_mut().unwrap();
            particle
                .runtime
                .refresh_attachment(particle.handle, bits, speed)
                .map_err(refusal)?;
            if advance {
                particle.binding = particle
                    .runtime
                    .light_binding(particle.handle)
                    .map_err(refusal)?;
                let updates = [ParticleInstanceUpdate {
                    handle: particle.handle,
                    parent_world_matrix_f32_bits: None,
                    inherited_speed: None,
                    action: if instance.stop_unbounded {
                        ParticleAction::StopUnboundedEmission
                    } else {
                        ParticleAction::Continue
                    },
                }];
                particle
                    .runtime
                    .advance(now.to_bits() as u64, delta, &updates)
                    .map_err(refusal)?;
                instance.stop_unbounded = false;
            } else {
                particle.binding.world_matrix_f32_bits = particle
                    .runtime
                    .light_binding(particle.handle)
                    .map_err(refusal)?
                    .world_matrix_f32_bits;
            }
            rows.authored_particles.push(crate::production_features::xmap20::Xmap20AuthoredParticleBinding {
                instance_id: instance.id.clone(),
                entity_id: instance.entity_id.clone(),
                authored_event_index: instance.authored_event_index,
                particle_token: name.to_owned(),
                paired_light: instance.event.light.clone(),
                light_phase: particle.binding,
                particle_phase: particle.runtime.light_binding(particle.handle).map_err(refusal)?,
            });
            rows.particle_bindings
                .insert(instance.id.clone(), particle.binding);
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(
        count: usize,
    ) -> (
        ViewerLightLifecycle,
        Vec<Xmap09PlacedEntity>,
        ViewerOwnedMapState,
    ) {
        let templates = parse_light_templates("light = {\n name = flash\n color = { r = 1 g = 1 b = 1 }\n intensity = 1\n radius = 8\n falloff = 1\n position = { x = 1 y = 0 z = 0 }\n}", "fixture.asset").unwrap();
        let mut lifecycle = ViewerLightLifecycle {
            assets: Some(Arc::new(Assets {
                models: Mutex::new(BTreeMap::new()),
                animations: Mutex::new(BTreeMap::new()),
                poses: BTreeMap::new(),
                static_attachments: BTreeSet::new(),
                meshes: BTreeMap::new(),
                mesh_clips: BTreeMap::new(),
                clips: BTreeMap::new(),
                templates,
            })),
            ..Default::default()
        };
        let placed = (0..count)
            .map(|i| {
                let id = format!("placed-{i:03}");
                let mut state = animation(false, false);
                Arc::make_mut(&mut state.events)[0].node = None;
                lifecycle.entities.insert(id.clone(), state);
                let mut world = IDENTITY;
                world[12] = 16.0 + (i % 8) as f32 * 48.0;
                world[14] = 16.0 + (i / 8) as f32 * 48.0;
                Xmap09PlacedEntity {
                    id,
                    entity_token: "entity".into(),
                    state: None,
                    animate: true,
                    world_matrix_f32_bits: world.map(f32::to_bits),
                }
            })
            .collect();
        let viewer = ViewerOwnedMapState {
            sky_enabled: true,            viewport_width: 1,
            viewport_height: 1,
            camera: None,
            observer_country_id: None,
            map_mode_id: "political".into(),
            map_mode_selection_epoch: 0,
            selected_object_ids: vec![],
            input_revision: 0,
            animation_time_f32_bits: 0.5f32.to_bits(),
            frame_delta_seconds_f32_bits: 0.5f32.to_bits(),
        };
        (lifecycle, placed, viewer)
    }

    fn paired_fixture(
        body: &str,
    ) -> (
        crate::particle_authored::test_support::Fixture,
        ViewerLightLifecycle,
        Vec<Xmap09PlacedEntity>,
        ViewerOwnedMapState,
    ) {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(
            REGISTRATION,
            &format!("particle={{name=effect {body}}}"),
            &[(
                "gfx/entity.asset",
                br#"
entity = {
    name = entity
    scale = 2
    default_state = idle
    state = {
        name = idle
        event = { time = 0.25 light = flash particle = paired keep_particle = yes }
    }
    state = { name = stopped }
}
light = {
    name = flash
    color = { r = 1 g = 1 b = 1 }
    intensity = 1
    radius = 8
    falloff = 1
    position = { x = 1 y = 0 z = 0 }
}
"#,
            )],
        );
        let (_, placed, viewer) = fixture(1);
        (content, ViewerLightLifecycle::default(), placed, viewer)
    }


    fn weather_frame(resolver: &GameFileResolver, distance: f32) -> PreparedXmap19WeatherFrame {
        use crate::production_features::xmap19::*;
        let content = resolver.require_complete_admitted_dir("", None, true, "weather fixture").unwrap();
        let key = |path: &str| content.iter().find(|key| key.normalized_game_relative_path == path).unwrap().clone();
        let program = PreparedWeatherEntityProgram {
            entity_name: "weather".into(),
            entity_definition_source: key("gfx/entity.asset"),
            default_state_name: "idle".into(),
            ordered_events: vec![PreparedWeatherAuthoredEvent {
                authored_event_index: 0,
                ordered_particle_effects: vec![PreparedWeatherParticleEffect {
                    authored_effect_index: 0, particle_token: "paired".into(),
                }],
                node_token: None, time_f32_bits: Some(0.25f32.to_bits()),
                keep_particle: None, trigger_once: None,
            }],
            ordered_particle_names: vec!["paired".into()],
            ordered_particle_types: vec!["effect".into()],
            ordered_particle_registration_sources: vec![key("gfx/particles.gfx")],
            ordered_particle_asset_paths: vec!["gfx/particles.asset".into()],
            ordered_particle_shader_effects: vec![vec!["Weather".into()]],
            ordered_particle_shader_paths: vec![vec!["gfx/weather.shader".into()]],
            ordered_texture_paths: vec!["gfx/weather.dds".into()],
            ordered_content: content.clone(),
        };
        let primary = weather_membership_producer_instance_id(0, 12, 2, 1, "primary");
        let fading = weather_membership_producer_instance_id(0, 12, 2, 1, "fading");
        let law = weather_visibility_law(distance, &VANILLA_WEATHER_FADE_DEFINES).unwrap();
        let mut frame = PreparedXmap19WeatherFrame {
            schema: XMAP19_WEATHER_SCHEMA.into(), hour_index: 1,
            viewer_visual_lifecycle_sha256: "a".repeat(64),
            camera_distance_f32_bits: distance.to_bits(),
            primary_control_written: law.primary_control_written,
            primary_visibility_alpha_f32_bits: law.primary_alpha.to_bits(),
            fading_visibility_alpha_f32_bits: law.fading_alpha.to_bits(),
            playback_rate_f32_bits: law.playback_rate.to_bits(),
            distance_fog_color_f32_bits: VANILLA_WEATHER_DISTANCE_FOG.color.map(f32::to_bits),
            distance_fog_begin_f32_bits: VANILLA_WEATHER_DISTANCE_FOG.begin.to_bits(),
            distance_fog_end_f32_bits: VANILLA_WEATHER_DISTANCE_FOG.end.to_bits(),
            distance_fog_max_f32_bits: VANILLA_WEATHER_DISTANCE_FOG.max.to_bits(),
            ordered_used_content: content,
            regions: (0..XMAP19_STRATEGIC_REGION_COUNT).map(|id| PreparedWeatherRegionEpoch {
                strategic_region_id: id, airborne_runtime_code: if id == 0 { 2 } else { 0 },
                transition_expiry_game_hour: 12, epoch_identity: 12,
                selection_seed_words: (id == 0).then_some([0, 1]),
                selection_seed_after_positions: (id == 0).then_some([1, 1]),
                derived_producer_instance_ids: if id == 0 { vec![primary.clone(), fading.clone()] } else { vec![] },
            }).collect(),
            ordered_positions: vec![PreparedWeatherPosition {
                source_ordinal: 1, strategic_region_id: 0,
                position_xyz_f32_bits: [7.0f32, 11.0, -13.0].map(f32::to_bits),
                size: "big".into(), airborne_runtime_code: 2, weather_effect_key: Some("rain_light".into()),
                selection: Some(PreparedWeatherSelection {
                    authored_weather_effect_index: 0, primary_source_id: primary,
                    fading_source_id: Some(fading), primary: program.clone(), fading: Some(program),
                }),
            }],
            deterministic_sha256: String::new(),
        };
        frame.deterministic_sha256 = snapshot::sha256::Sha256::hex(&serde_json::to_vec(&frame).unwrap());
        frame.validate().unwrap();
        frame
    }

    fn weather_content() -> crate::particle_authored::test_support::Fixture {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        Fixture::with_extra_files(REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=8 duration=-1 emission=4 life=20 color={a=255} texture={file=gfx/weather.dds shader=Weather}}}",
            &[("gfx/entity.asset", br#"
entity = {
 name = weather
 default_state = idle
 state = {
  name = idle
  event = { time = 0.25 particle = paired }
 }
}
"#), ("gfx/weather.shader", b"Effect Weather = { VertexShader = VS PixelShader = PS }"),
                ("gfx/weather.dds", b"lifecycle fixture: texture is not decoded by this consumer")])
    }

    #[test]
    fn foundation_weather_binding_failure_is_local_and_checks_exact_custody_once() {
        use crate::production_foundation::*;
        use crate::production_features::{xmap19, xmap20};
        use portable_render_stream::{FamilyDisposition, MapVisualFamily};
        use replay_core::{RawFrame, ReplaySource, SourceError, WorldTimeline};
        use std::{cell::{Cell, RefCell}, rc::Rc};
        struct Source;
        impl ReplaySource for Source {
            fn game_id(&self) -> &str { "weather-binding" }
            fn hours(&self) -> &[i64] { &[1] }
            fn warnings(&self) -> &[String] { &[] }
            fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
                Ok(RawFrame { hour_index, date: None, save_version: None, schema_version: None, rest: Default::default() })
            }
        }
        struct Boundary {
            family: MapVisualFamily,
            weather: PreparedXmap19WeatherFrame,
            time: u32,
            mode: Rc<Cell<u8>>,
            calls: Rc<RefCell<Vec<&'static str>>>,
        }
        impl ProductionMapFeatureAdapter for Boundary {
            fn family(&self) -> MapVisualFamily { self.family }
            fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
                match self.family {
                    MapVisualFamily::WeatherAtmosphere => {
                        self.calls.borrow_mut().push("weather");
                        self.weather.validate()?;
                        let topology = product_foundation_assigned_stage_topology()?;
                        let schedule = portable_render_stream::AssignedStageSchedule::from_topology(&topology)?;
                        let operation = schedule.require(xmap19::XMAP19_ASSIGNED_STAGE_NAME, xmap19::XMAP19_ASSIGNED_OPERATION_NAME)?
                            .foundation_operation_identity("a".repeat(64), "b".repeat(64), "c".repeat(64))?;
                        let mut output = FoundationFeatureOutput::without_payload(FamilyDisposition::Present {
                            payload_schema: self.weather.schema.clone(),
                            producer_registration_sha256: "a".repeat(64),
                            payload_sha256: self.weather.deterministic_sha256.clone(),
                            ordered_operations: vec![operation],
                            ordered_used_content: self.weather.ordered_used_content.clone(),
                        }, context.map_authority_gaps.iter().filter(|gap| matches!(gap.input_id.as_str(),
                            "weather.region_gameplay" | "weather.visual_membership_lifecycle"))
                            .map(|gap| gap.input_id.clone()).collect());
                        output.prepared_weather = Some(self.weather.clone());
                        Ok(output)
                    }
                    MapVisualFamily::LightingShadows => {
                        self.calls.borrow_mut().push("lighting");
                        assert_eq!(context.prepared_weather.unwrap().deterministic_sha256, self.weather.deterministic_sha256);
                        Err(FamilyContentRefusal::new("XMAP09.fixture", "boundary fixture has no lighting draw"))
                    }
                    MapVisualFamily::ParticlesEffects => {
                        self.calls.borrow_mut().push("particles");
                        let output = xmap20::Xmap20EffectsAdapter.prepare(context)?;
                        let causes = output.element_diagnostics.iter().map(|row| row.cause_id.as_str()).collect::<Vec<_>>();
                        assert!(causes.contains(&"unit-event-phase-unbound"));
                        assert_eq!(causes.contains(&"weather-event-phase-unbound"), self.mode.get() == 1);
                        Ok(output)
                    }
                    MapVisualFamily::MapFurnitureIcons => {
                        self.calls.borrow_mut().push("later-family");
                        Err(FamilyContentRefusal::new("XMAP21.fixture", "boundary fixture has no furniture"))
                    }
                    _ => unreachable!(),
                }
            }
            fn weather_particles_bound(&self, time: f32, identity: &str) -> Result<bool> {
                assert_eq!(self.family, MapVisualFamily::LightingShadows);
                assert_eq!(time.to_bits(), self.time);
                assert_eq!(identity, self.weather.deterministic_sha256);
                self.calls.borrow_mut().push("binding");
                match self.mode.get() {
                    0 => Err("retained weather custody unavailable".into()),
                    1 => Ok(false),
                    2 => Ok(true),
                    _ => unreachable!(),
                }
            }
        }
        let content = weather_content();
        let weather = weather_frame(&content.resolver, 300.0);
        let (_, _, viewer) = fixture(0);
        let timeline = WorldTimeline::open(Box::new(Source)).unwrap();
        let world = timeline.frame(1).unwrap();
        let mut manifest: replay_core::ContentManifest = serde_json::from_value(serde_json::json!({
            "schema": replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA,
            "product_modset_sha256": "1".repeat(64),
            "source_game": { "version": "1.19.2", "build": "a729",
                "executable_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad", "build_profile": "weather-binding" },
            "source_profile": { "profile_id": "weather-binding", "playset_id": "vanilla", "selection_sha256": "0".repeat(64) },
            "effective_vfs_manifest_sha256": "2".repeat(64), "ordered_layers": [], "dlc_selection": [],
            "localisation_language": "l_english", "required_settings": {}, "recorder_local_settings": {},
            "required_content": [], "witnessed_absent_content": [], "witnessed_content_namespaces": [], "manifest_digest_sha256": null
        })).unwrap();
        manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
        let manifest = manifest.with_computed_digest().unwrap();
        let receipt = FoundationContentExecutionReceipt {
            schema: FOUNDATION_CONTENT_EXECUTION_RECEIPT_SCHEMA.into(),
            content_manifest_sha256: manifest.manifest_digest_sha256.clone().unwrap(),
            product_modset_sha256: manifest.product_modset_sha256.clone(),
            effective_vfs_manifest_sha256: manifest.effective_vfs_manifest_sha256.clone(),
            content_authority_sha256: "3".repeat(64), resolved_operation_sha256: "4".repeat(64),
            ordered_program_stage_sha256: vec!["5".repeat(64)], resource_identity_sha256: "6".repeat(64),
            descriptor_identity_sha256: "7".repeat(64), compiled_or_loaded_programs: 1,
            created_resources: 1, created_descriptors: 1, deterministic_sha256: String::new(),
        }.seal().unwrap();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mode = Rc::new(Cell::new(0));
        let mut adapters = ProductionMapFeatureAdapters::default();
        for family in [MapVisualFamily::WeatherAtmosphere, MapVisualFamily::LightingShadows,
            MapVisualFamily::ParticlesEffects, MapVisualFamily::MapFurnitureIcons] {
            adapters.register(Box::new(Boundary { family, weather: weather.clone(), time: viewer.animation_time_f32_bits,
                mode: mode.clone(), calls: calls.clone() })).unwrap();
        }
        for outcome in 0..3 {
            mode.set(outcome);
            calls.borrow_mut().clear();
            let error = prepare_foundation_frame(&world, &viewer, &manifest, &receipt, &adapters, None)
                .err().expect("the fixture deliberately omits mandatory terrain").to_string();
            assert!(error.contains("XMAP04 refuses the mandatory Terrain"), "{error}");
            let expected = if outcome == 0 {
                vec!["weather", "lighting", "binding", "later-family"]
            } else {
                vec!["weather", "lighting", "binding", "particles", "later-family"]
            };
            assert_eq!(*calls.borrow(), expected);
        }
    }

    #[test]
    fn failed_event_update_refuses_retry_and_draw_until_explicit_reset() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let repaired = weather_content();
        let particles = read_asset(&repaired.resolver, "gfx/particles.asset").unwrap();
        let shader = read_asset(&repaired.resolver, "gfx/weather.shader").unwrap();
        let texture = read_asset(&repaired.resolver, "gfx/weather.dds").unwrap();
        let broken = Fixture::with_extra_files(REGISTRATION, std::str::from_utf8(&particles).unwrap(), &[
            ("gfx/entity.asset", br#"
entity = {
 name = weather
 default_state = idle
 state = {
  name = idle
  event = { time = 0.25 particle = paired }
  event = { time = 0.3 entity = missing_child }
 }
}
"#), ("gfx/weather.shader", &shader), ("gfx/weather.dds", &texture),
        ]);
        let weather = weather_frame(&broken.resolver, 300.0);
        let primary = &weather.ordered_positions[0].selection.as_ref().unwrap().primary_source_id;
        let (_, _, mut viewer) = fixture(0);
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        viewer.animation_time_f32_bits = 0.125f32.to_bits();
        viewer.frame_delta_seconds_f32_bits = 0.125f32.to_bits();
        lifecycle.derive_light_instance_rows(&broken.resolver, &[], &viewer, Some(&weather)).unwrap();
        assert!(lifecycle.weather_particles_bound(0.125, &weather.deterministic_sha256));

        viewer.animation_time_f32_bits = 0.5f32.to_bits();
        viewer.frame_delta_seconds_f32_bits = 0.375f32.to_bits();
        let first = lifecycle.derive_light_instance_rows(&broken.resolver, &[], &viewer, Some(&weather))
            .err().expect("the second crossed event has no authored child definition");
        assert!(matches!(&first, Xmap09LightCensusRefusal::MissingDomain { domain }
            if domain.contains("missing_child")), "{first:?}");
        assert_eq!(lifecycle.entities[primary].playhead, 0.5);
        assert_eq!(lifecycle.instances.len(), 1, "the earlier event has already mutated retained state");
        assert!(lifecycle.instances[0].particle.is_none());
        assert_eq!(lifecycle.last_presentation_time, None);
        assert!(!lifecycle.weather_particles_bound(0.125, &weather.deterministic_sha256));
        assert!(!lifecycle.weather_particles_bound(0.5, &weather.deterministic_sha256));
        let first_ordinal = lifecycle.next_particle_only_instance;
        let camera = crate::particle_runtime::ParticleSortCamera {
            position_f32_bits: [0; 3], forward_f32_bits: [0, 0, 1.0f32.to_bits()],
        };
        for phase in [0.5f32, 1.0, 0.0] {
            viewer.animation_time_f32_bits = phase.to_bits();
            assert_eq!(lifecycle.derive_light_instance_rows(&broken.resolver, &[], &viewer, Some(&weather))
                .err().unwrap(), first);
            assert_eq!(lifecycle.entities[primary].playhead, 0.5);
            assert_eq!(lifecycle.next_particle_only_instance, first_ordinal);
            assert_eq!(lifecycle.instances.len(), 1);
            let error = lifecycle.visit_particles_for_draw(phase, camera, &mut |_, _, _| {
                panic!("failed lifecycle state reached the draw boundary")
            }).unwrap_err().to_string();
            assert!(error.contains("reset_population required") && error.contains("missing_child"), "{error}");
            assert!(!lifecycle.weather_particles_bound(phase, &weather.deterministic_sha256));
        }
        let repaired_weather = weather_frame(&repaired.resolver, 300.0);
        viewer.animation_time_f32_bits = 0.5f32.to_bits();
        viewer.frame_delta_seconds_f32_bits = 0.5f32.to_bits();
        assert_eq!(lifecycle.derive_light_instance_rows(&repaired.resolver, &[], &viewer, Some(&repaired_weather))
            .err().unwrap(), first, "a new content lease must not implicitly publish partial state");
        lifecycle.reset_population();
        assert!(lifecycle.entities.is_empty() && lifecycle.instances.is_empty());
        assert!(lifecycle.publication_refusal.is_none());
        assert!(lifecycle.visit_particles_for_draw(0.5, camera, &mut |_, _, _| {
            panic!("reset alone cannot publish a frame")
        }).is_err());
        let rows = lifecycle.derive_light_instance_rows(&repaired.resolver, &[], &viewer, Some(&repaired_weather)).unwrap();
        assert_eq!(rows.authored_particles.len(), 1);
        assert!(lifecycle.weather_particles_bound(0.5, &repaired_weather.deterministic_sha256));
        let mut records = 0;
        lifecycle.visit_particles_for_draw(0.5, camera, &mut |_, capacity, view| {
            view.visit_draw_subsystems(capacity.packets as usize, false, |_, subsystem| {
                records += subsystem.records().collect::<std::result::Result<Vec<_>, _>>()?.len();
                Ok(())
            })?;
            Ok(())
        }).unwrap();
        assert!(records > 0, "explicit reset and valid inputs must restore real particle output");
    }

    #[test]
    fn weather_controls_drive_retained_particles_and_the_real_draw_visitor() {
        let content = weather_content();
        let (_, _, mut viewer) = fixture(0);
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        let first = weather_frame(&content.resolver, 300.0);
        let selection = first.ordered_positions[0].selection.as_ref().unwrap();
        let primary = selection.primary_source_id.clone();
        let fading = selection.fading_source_id.clone().unwrap();
        // At near zoom, the primary is untouched and the fading entity is hidden before events.
        lifecycle.derive_light_instance_rows(&content.resolver, &[], &viewer, Some(&first)).unwrap();
        assert!(lifecycle.weather_particles_bound(0.5, &first.deterministic_sha256));
        assert!(!lifecycle.weather_particles_bound(0.75, &first.deterministic_sha256));
        assert_eq!(lifecycle.instances.len(), 1);
        assert_eq!(lifecycle.instances[0].entity_id, primary);
        assert_eq!(lifecycle.entities[&fading].playhead, 0.5);
        assert_eq!(lifecycle.entities[&fading].elapsed, 0.0);
        let particle = lifecycle.instances[0].particle.as_ref().unwrap();
        let original = particle.runtime.gpu_view(particle.handle).unwrap().root_subsystems().next().unwrap()
            .records().collect::<std::result::Result<Vec<_>, _>>().unwrap();
        assert_eq!(original.len(), 2);
        assert!(original.iter().all(|record| record.0[35] == 255));
        assert_eq!(particle.binding.world_matrix_f32_bits[12..15], first.ordered_positions[0].position_xyz_f32_bits);

        let hidden = weather_frame(&content.resolver, 1600.0);
        viewer.animation_time_f32_bits = 1.0f32.to_bits();
        lifecycle.derive_light_instance_rows(&content.resolver, &[], &viewer, Some(&hidden)).unwrap();
        let particle = lifecycle.instances[0].particle.as_ref().unwrap();
        assert_eq!(original, particle.runtime.gpu_view(particle.handle).unwrap().root_subsystems().next().unwrap()
            .records().collect::<std::result::Result<Vec<_>, _>>().unwrap());
        let camera = crate::particle_runtime::ParticleSortCamera {
            position_f32_bits: [0; 3], forward_f32_bits: [0, 0, 1.0f32.to_bits()],
        };
        let mut draws = 0;
        lifecycle.visit_particles_for_draw(1.0, camera, &mut |_, cap, view| {
            draws += view.visit_draw_subsystems(cap.packets as usize, false, |_, _| Ok(()))?;
            Ok(())
        }).unwrap();
        assert_eq!(draws, 0);

        viewer.animation_time_f32_bits = 1.5f32.to_bits();
        lifecycle.derive_light_instance_rows(&content.resolver, &[], &viewer, Some(&first)).unwrap();
        assert!(!lifecycle.entities[&primary].visible, "near branch must not reset the retained primary flag");
        assert_eq!(lifecycle.entities[&primary].speed, 1.0);
        assert_eq!(lifecycle.entities[&primary].elapsed, 0.5);

        let middle = weather_frame(&content.resolver, 1300.0);
        viewer.animation_time_f32_bits = 2.0f32.to_bits();
        lifecycle.derive_light_instance_rows(&content.resolver, &[], &viewer, Some(&middle)).unwrap();
        assert!(lifecycle.entities[&primary].visible);
        assert!(lifecycle.entities[&fading].visible);
        assert_eq!(lifecycle.entities[&primary].speed, f64::from(0.15f32));
        let particle = lifecycle.instances[0].particle.as_ref().unwrap();
        let faded = particle.runtime.gpu_view(particle.handle).unwrap().root_subsystems().next().unwrap()
            .records().collect::<std::result::Result<Vec<_>, _>>().unwrap();
        assert_eq!(faded[0].0[35], 128);
        draws = 0;
        lifecycle.visit_particles_for_draw(2.0, camera, &mut |_, cap, view| {
            draws += view.visit_draw_subsystems(cap.packets as usize, false, |_, _| Ok(()))?;
            Ok(())
        }).unwrap();
        assert!(draws > 0);
        lifecycle.derive_light_instance_rows(&content.resolver, &[], &viewer, Some(&middle)).unwrap();
        let particle = lifecycle.instances[0].particle.as_ref().unwrap();
        assert_eq!(faded, particle.runtime.gpu_view(particle.handle).unwrap().root_subsystems().next().unwrap()
            .records().collect::<std::result::Result<Vec<_>, _>>().unwrap());
        lifecycle.derive_light_instance_rows(&content.resolver, &[], &viewer, None).unwrap();
        assert!(lifecycle.instances.is_empty());
        assert!(!lifecycle.weather_particles_bound(2.0, &middle.deterministic_sha256));
    }

    #[test]
    fn weather_opacity_setter_changes_existing_particles_but_is_not_inherited_by_later_births() {
        let content = weather_content();
        let weather = weather_frame(&content.resolver, 1300.0);
        let (_, _, mut viewer) = fixture(0);
        viewer.animation_time_f32_bits = 2.0f32.to_bits();
        viewer.frame_delta_seconds_f32_bits = 2.0f32.to_bits();
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        lifecycle.derive_light_instance_rows(&content.resolver, &[], &viewer, Some(&weather)).unwrap();
        assert_eq!(lifecycle.instances.len(), 2);
        for instance in &lifecycle.instances {
            let particle = instance.particle.as_ref().unwrap();
            let records = particle.runtime.gpu_view(particle.handle).unwrap().root_subsystems().next().unwrap()
                .records().collect::<std::result::Result<Vec<_>, _>>().unwrap();
            assert!(!records.is_empty());
            assert!(records.iter().all(|record| record.0[35] == 255));
        }
        viewer.animation_time_f32_bits = 2.5f32.to_bits();
        viewer.frame_delta_seconds_f32_bits = 0.5f32.to_bits();
        lifecycle.derive_light_instance_rows(&content.resolver, &[], &viewer, Some(&weather)).unwrap();
        for instance in &lifecycle.instances {
            let particle = instance.particle.as_ref().unwrap();
            let record = particle.runtime.gpu_view(particle.handle).unwrap().root_subsystems().next().unwrap()
                .records().next().unwrap().unwrap();
            assert_eq!(record.0[35], 128);
        }
    }

    #[test]
    fn an_unbound_static_attachment_is_named_only_when_its_entity_is_instantiated() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}",
            &[("gfx/entity.asset", b"entity={\n name=unbound\n attach={node=child}\n}\nentity={\n name=empty\n}\n")]);
        let (_, mut placed, viewer) = fixture(1);
        let mut lifecycle = ViewerLightLifecycle::default();
        placed[0].entity_token = "empty".into();
        lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        placed[0].entity_token = "unbound".into();
        let error = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).err().unwrap();
        assert!(format!("{error:?}").contains("unbound static attachment selection/phase"));
    }

    #[test]
    fn hidden_entity_advances_playhead_without_firing_missed_events_or_elapsed_time() {
        let mut state = animation(false, false);
        state.visible = false;
        assert!(state.advance(0.5).is_empty());
        assert_eq!(state.playhead, 0.5);
        assert_eq!(state.elapsed, 0.0);
        state.visible = true;
        assert!(state.advance(0.1).is_empty());
        assert_eq!(state.playhead, 0.6);
        assert_eq!(state.elapsed, 0.1);
    }

    #[test]
    fn particle_draw_borrows_the_same_paired_runtime_and_rejects_stale_phase() {
        let (content, mut lights, placed, viewer) =
            paired_fixture("subsystem={name=s max_amount=4 duration=-1 emission=2 life=5}");
        let rows = lights.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        let camera = crate::particle_runtime::ParticleSortCamera {
            position_f32_bits: [0; 3],
            forward_f32_bits: [0, 0, 1.0_f32.to_bits()],
        };
        let time = f32::from_bits(viewer.animation_time_f32_bits);
        let mut seen = Vec::new();
        lights.visit_particles_for_draw(time, camera, &mut |id, capacity, view| {
            assert_eq!(view.world_matrix_f32_bits(), rows.particle_bindings[id].world_matrix_f32_bits);
            assert_eq!(view.sorted_presentation_id(), Some(time.to_bits() as u64));
            assert_eq!(capacity.packets, 1);
            assert_eq!(capacity.particles, 4);
            seen.push(view.handle());
            Ok(())
        }).unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0], lights.instances[0].particle.as_ref().unwrap().handle);
        let before = lights.instances[0].particle.as_ref().unwrap().binding;
        lights.visit_particles_for_draw(time, camera, &mut |_, _, view| {
            assert_eq!(view.handle(), seen[0]);
            Ok(())
        }).unwrap();
        assert_eq!(lights.instances[0].particle.as_ref().unwrap().binding, before);
        let error = lights.visit_particles_for_draw(time + 0.25, camera, &mut |_, _, _| {
            panic!("stale draw must not expose particle state")
        }).unwrap_err();
        assert!(error.to_string().contains("another presentation"));
    }

    #[test]
    fn xmap20_authored_consumer_matches_the_paired_light_phase_and_seek_reset() {
        let (content, mut lights, placed, mut viewer) =
            paired_fixture("subsystem={name=s max_amount=4 duration=0 emission=2 life=0.5}");
        let mut effects = crate::Xmap20AuthoredEntityRuntime::default();
        let mut first = None;
        for frame in 1..=4 {
            viewer.animation_time_f32_bits = (frame as f32 * 0.5).to_bits();
            let light_rows = lights.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
            let effect_rows = effects.prepare(&content.resolver, &placed, &viewer).unwrap();
            assert_eq!(effect_rows.len(), light_rows.particle_bindings.len());
            for effect in &effect_rows {
                assert_eq!(effect.light_phase, light_rows.particle_bindings[&effect.instance_id]);
                assert_eq!(effect.authored_event_index, 0);
                assert_eq!(effect.particle_token, "paired");
                assert_eq!(effect.paired_light.as_deref(), Some("flash"));
                assert_eq!(effect.particle_phase.presentation_id, Some(viewer.animation_time_f32_bits as u64));
            }
            assert_eq!(effect_rows, effects.prepare(&content.resolver, &placed, &viewer).unwrap());
            if frame == 1 { first = Some(effect_rows); }
        }
        viewer.animation_time_f32_bits = 0.5f32.to_bits();
        assert_eq!(effects.prepare(&content.resolver, &placed, &viewer).unwrap(), first.unwrap());
    }

    #[test]
    fn particle_only_events_do_not_change_the_paired_consumers_seed_identity() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=4 duration=-1 emission=2 life=0.5}}",
            &[("gfx/entity.asset", br#"entity={
                name=entity default_state=idle state={name=idle
                    event={time=0 particle=paired}
                    event={time=0.25 particle=paired light=flash}
                }
            }"#)]);
        let (_, placed, mut viewer) = fixture(1);
        let mut lights = ViewerLightLifecycle::default();
        let mut effects = crate::Xmap20AuthoredEntityRuntime::default();
        for frame in 1..=3 {
            viewer.animation_time_f32_bits = (frame as f32 * 0.5).to_bits();
            let rows = effects.prepare(&content.resolver, &placed, &viewer).unwrap();
            assert_eq!(rows.len(), 2);
            assert_ne!(rows[0].light_phase.event_instance_id, rows[1].light_phase.event_instance_id);
            let paired = rows.iter().find(|row| row.paired_light.is_some()).unwrap();
            let light_rows = lights.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
            assert_eq!(light_rows.particle_bindings.len(), 1);
            assert_eq!(paired.light_phase, light_rows.particle_bindings[&paired.instance_id]);
        }
    }

    #[test]
    fn xmap20_particle_only_event_is_real_runtime_state_without_a_light_or_packet() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}",
            &[("gfx/entity.asset", br#"entity={
                name=entity scale=2 default_state=idle
                state={name=idle event={time=0.25 particle=paired}}
            }"#)]);
        let (_, placed, mut viewer) = fixture(1);
        let mut effects = crate::Xmap20AuthoredEntityRuntime::default();
        let first = effects.prepare(&content.resolver, &placed, &viewer).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].entity_id, "placed-000");
        assert_eq!(first[0].authored_event_index, 0);
        assert_eq!(first[0].particle_token, "paired");
        assert_eq!(first[0].paired_light, None);
        assert!(first[0].light_phase.finished);
        assert!(first[0].particle_phase.finished);
        assert_eq!(first[0].light_phase.world_matrix_f32_bits[12], 16f32.to_bits());
        assert_eq!(first[0].light_phase.world_matrix_f32_bits[14], 16f32.to_bits());
        viewer.animation_time_f32_bits = 1f32.to_bits();
        assert!(effects.prepare(&content.resolver, &placed, &viewer).unwrap().is_empty());
    }

    #[test]
    fn authored_paired_light_is_nonzero_and_cleanup_reads_the_previous_particle_phase() {
        let (content, mut lifecycle, placed, mut viewer) =
            paired_fixture("subsystem={name=s max_amount=4 duration=0 emission=2 life=0.5}");
        let mut counts = Vec::new();
        for frame in 1..=4 {
            viewer.animation_time_f32_bits = (frame as f32 * 0.5).to_bits();
            let rows = lifecycle
                .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
                .unwrap();
            let census = rows
                .with_rows(|r| admit_light_census(Ok(r), [0., 0., 0.125, 0.125]))
                .unwrap();
            counts.push(census.lights.len());
            if frame == 1 {
                assert_eq!(
                    census.lights[0]
                        .position_radius_f32_bits
                        .map(f32::from_bits),
                    [18., 0., 16., 8.]
                );
                let binding = rows.particle_bindings.values().next().unwrap();
                let mut world = placed[0].world_matrix_f32_bits;
                for i in [0, 5, 10] {
                    world[i] = 2f32.to_bits();
                }
                assert_eq!(binding.world_matrix_f32_bits, world);
                assert!(!binding.finished);
            }
        }
        assert_eq!(counts, [1, 1, 1, 0]);
        assert!(lifecycle.instances.is_empty());
    }

    #[test]
    fn repeated_presentation_and_seek_preserve_seeded_particle_light_phase() {
        let (content, mut lifecycle, placed, mut viewer) =
            paired_fixture("subsystem={name=s max_amount=4 duration=0 emission=2 life={0.5 0.25}}");
        let first = lifecycle
            .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
            .unwrap();
        let repeated = lifecycle
            .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
            .unwrap();
        assert_eq!(first.particle_bindings, repeated.particle_bindings);
        assert_eq!(lifecycle.instances.len(), 1);
        assert_eq!(
            lifecycle.instances[0]
                .particle
                .as_ref()
                .unwrap()
                .runtime
                .allocated_particle_capacity(),
            4
        );
        viewer.animation_time_f32_bits = 1f32.to_bits();
        lifecycle
            .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
            .unwrap();
        viewer.animation_time_f32_bits = 0.5f32.to_bits();
        let sought = lifecycle
            .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
            .unwrap();
        assert_eq!(first.particle_bindings, sought.particle_bindings);
        assert_eq!(lifecycle.instances.len(), 1);
    }

    #[test]
    fn kept_unbounded_pair_stops_on_requested_state_change_and_drains_before_cleanup() {
        let (content, mut lifecycle, mut placed, mut viewer) =
            paired_fixture("subsystem={name=s max_amount=4 duration=-1 emission=2 life=0.5}");
        lifecycle
            .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
            .unwrap();
        placed[0].state = Some("stopped".into());
        let mut counts = Vec::new();
        for frame in 2..=5 {
            viewer.animation_time_f32_bits = (frame as f32 * 0.5).to_bits();
            let rows = lifecycle
                .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
                .unwrap();
            counts.push(
                rows.with_rows(|r| admit_light_census(Ok(r), [0., 0., 0.125, 0.125]))
                    .unwrap()
                    .lights
                    .len(),
            );
        }
        assert_eq!(counts, [1, 1, 1, 0]);
    }

    #[test]
    fn empty_paired_roster_lights_its_creation_frame_then_releases_storage() {
        let (content, mut lifecycle, placed, mut viewer) = paired_fixture("");
        let first = lifecycle
            .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
            .unwrap();
        assert!(first.particle_bindings.values().next().unwrap().finished);
        assert_eq!(
            first
                .with_rows(|r| admit_light_census(Ok(r), [0., 0., 0.125, 0.125]))
                .unwrap()
                .lights
                .len(),
            1
        );
        viewer.animation_time_f32_bits = 1f32.to_bits();
        let next = lifecycle
            .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
            .unwrap();
        assert!(next.instances.is_empty());
        assert!(lifecycle.instances.is_empty());
    }

    #[test]
    fn missing_paired_registration_names_the_fired_event_and_input() {
        let (content, mut lifecycle, placed, viewer) = paired_fixture("");
        let mut definitions = content.definitions();
        assert!(definitions.resolve("absent").is_err());
        lifecycle.assets = Some(Arc::new(Assets::load(&content.resolver).unwrap()));
        lifecycle.content_lease = content.resolver.content_admission_lease();
        let pose = Arc::get_mut(lifecycle.assets.as_mut().unwrap())
            .unwrap()
            .poses
            .get_mut("entity")
            .unwrap();
        pose.states[0].events[0].particle = Some("absent".into());
        let error = match lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
        {
            Err(error) => error,
            Ok(_) => panic!("missing particle was admitted"),
        };
        assert!(
            matches!(error, Xmap09LightCensusRefusal::ParticleInput { instance, refusal }
            if instance == "placed-000:light:0:0" && refusal.event_instance_id == Some(0)
                && refusal.reason == crate::particle_authored::ParticleRefusalReason::MissingRegistration)
        );
    }

    #[test]
    fn paired_light_event_keeps_its_authored_particle_dependency() {
        let poses = parse_building_entity_poses(
            r#"
entity = {
    name = paired
    state = {
        name = idle
        event = { time = 0 light = flash particle = smoke }
    }
}
"#,
            "paired.asset",
        )
        .unwrap();
        let event = &poses[0].states[0].events[0];
        assert_eq!(event.particle.as_deref(), Some("smoke"));
        assert!(
            matches!(resolve_light_event(event, &[], &[], &[], &Xmap09LightInstanceClock::default()),
            Err(Some(Xmap09LightEventRefusal::PairedParticleRuntimeUnavailable { particle })) if particle == "smoke")
        );
    }

    #[test]
    fn prepared_light_rows_share_catalogs_events_and_cached_asset_payloads() {
        let (mut lifecycle, placed, viewer) = fixture(1);
        let resolver = GameFileResolver::vanilla(".").unwrap();
        let assets = lifecycle.assets.as_ref().unwrap().clone();
        let model = lifecycle.entities["placed-000"]
            .model
            .as_ref()
            .unwrap()
            .clone();
        assets
            .models
            .lock()
            .unwrap()
            .insert("admitted.mesh".into(), model.clone());
        let cached = assets.model(&resolver, "admitted.mesh").unwrap();
        assert!(Arc::ptr_eq(&cached, &model));
        let rows = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        assert!(Arc::ptr_eq(&rows.assets, &assets));
        assert!(Arc::ptr_eq(
            &rows.entities[0].1,
            &lifecycle.entities["placed-000"].events
        ));
        assert!(Arc::ptr_eq(
            &rows.poses["placed-000"].events,
            &lifecycle.entities["placed-000"].events
        ));
        assert_eq!(rows.entity_indices["placed-000"], 0);
    }

    #[test]
    fn viewer_census_retains_expired_lights_and_accepts_entity_origin_events() {
        let (mut lifecycle, placed, mut viewer) = fixture(1);
        Arc::get_mut(lifecycle.assets.as_mut().unwrap())
            .unwrap()
            .templates[0]
            .duration = 0.1;
        let resolver = GameFileResolver::vanilla(".").unwrap();
        let rows = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        let first = rows
            .with_rows(|rows| admit_light_census(Ok(rows), [0., 0., 0.125, 0.125]))
            .unwrap();
        assert_eq!(
            first.lights[0].position_radius_f32_bits.map(f32::from_bits),
            [18., 0., 16., 8.]
        );
        viewer.animation_time_f32_bits = 1f32.to_bits();
        let rows = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        let expired = rows
            .with_rows(|rows| admit_light_census(Ok(rows), [0., 0., 0.125, 0.125]))
            .unwrap();
        assert_eq!(expired.counts.fired_instances, 1);
        assert_eq!(expired.counts.admitted_instances, 0);
        assert_eq!(rows.instances[0].3.age_seconds(), 0.5);
    }

    #[test]
    fn viewer_census_resolves_attach_nodes_only_until_the_upload_cap() {
        let (mut lifecycle, placed, viewer) = fixture(65);
        Arc::make_mut(&mut lifecycle.entities.get_mut("placed-064").unwrap().events)[0].node =
            Some("missing".into());
        let resolver = GameFileResolver::vanilla(".").unwrap();
        let rows = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        let census = rows
            .with_rows(|rows| admit_light_census(Ok(rows), [0., 0., 0.125, 0.125]))
            .unwrap();
        assert_eq!(census.counts.fired_instances, 65);
        assert_eq!(census.counts.admitted_instances, 65);
        assert_eq!(census.counts.uploaded_lights, 64);
        assert!(rows.poses["placed-064"].attach(0).is_err());
    }

    #[test]
    fn viewer_census_follows_child_events_scale_and_authored_life() {
        let (mut lifecycle, placed, mut viewer) = fixture(1);
        let event =
            &mut Arc::make_mut(&mut lifecycle.entities.get_mut("placed-000").unwrap().events)[0];
        event.light = None;
        event.entity = Some("child".into());
        event.life = Some(0.25);
        let poses = parse_building_entity_poses("entity = {\n name = child\n scale = 3\n default_state = idle\n state = {\n name = idle\n event = { time = 0 light = flash }\n }\n}", "fixture.asset").unwrap();
        Arc::get_mut(lifecycle.assets.as_mut().unwrap())
            .unwrap()
            .poses = poses
            .into_iter()
            .map(|p| (p.entity_name.clone(), p))
            .collect();
        let resolver = GameFileResolver::vanilla(".").unwrap();
        let rows = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        let census = rows
            .with_rows(|rows| admit_light_census(Ok(rows), [0., 0., 0.125, 0.125]))
            .unwrap();
        assert_eq!(census.counts.placed_entities, 2);
        assert_eq!(census.counts.authored_light_events, 1);
        assert_eq!(census.counts.fired_instances, 1);
        assert_eq!(
            census.lights[0]
                .position_radius_f32_bits
                .map(f32::from_bits),
            [22., 0., 16., 8.]
        );
        assert_eq!(rows.poses[rows.instances[0].1.as_str()].playhead, 0.5);
        viewer.animation_time_f32_bits = 1f32.to_bits();
        let rows = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        assert_eq!(rows.entities.len(), 1);
        assert!(rows.instances.is_empty());
    }

    #[test]
    fn viewer_clip_speed_keeps_inherited_and_authored_operands_and_once_end_exception() {
        let mut state = animation(false, true);
        state.clip = Some(Arc::new(PdxAnimData {
            fps: 10.0,
            sample_count: 10,
            joints: vec![],
            samples_t: vec![],
            samples_q: vec![],
            samples_s: vec![],
        }));
        state.length = 1.0;
        state.speed = 1.0;
        state.authored_speed = 2.0;
        Arc::make_mut(&mut state.events)[0].time = Some(0.75);
        assert_eq!(state.advance(0.5), [0]);
        assert_eq!(state.playhead, 1.0);
        let mut end = state.clone();
        end.playhead = 0.8;
        end.elapsed = 0.8;
        end.authored_speed = 1.0;
        Arc::make_mut(&mut end.events)[0].time = Some(0.9);
        assert_eq!(end.advance(0.3), [0]);
        assert!(end.elapsed > end.length);
        assert_eq!(end.playhead, 1.0);
        assert!(end.advance(0.1).is_empty());
        let mut no_clip = end;
        no_clip.clip = None;
        no_clip.playhead = 0.8;
        assert!(no_clip.advance(0.3).is_empty());
    }

    #[test]
    fn viewer_reverse_clip_normalizes_time_before_testing_event_intervals() {
        let mut state = animation(true, false);
        state.clip = Some(Arc::new(PdxAnimData {
            fps: 10.0,
            sample_count: 10,
            joints: vec![],
            samples_t: vec![],
            samples_q: vec![],
            samples_s: vec![],
        }));
        state.authored_speed = -1.0;
        Arc::make_mut(&mut state.events)[0].time = Some(0.95);
        assert!(state.advance(0.1).is_empty());
        assert_eq!(state.playhead, 0.9);
    }

    #[test]
    fn viewer_state_entry_draws_only_within_the_authored_time_offset_range() {
        let (mut lifecycle, mut placed, _) = fixture(1);
        let poses = parse_building_entity_poses("entity = {\n name = entity\n default_state = idle\n state = { name = idle time_offset = { 0.2 0.3 } }\n}", "fixture.asset").unwrap();
        assert_eq!(poses[0].states[0].time_offset, Some([0.2, 0.3]));
        Arc::get_mut(lifecycle.assets.as_mut().unwrap())
            .unwrap()
            .poses = poses
            .into_iter()
            .map(|p| (p.entity_name.clone(), p))
            .collect();
        let resolver = GameFileResolver::vanilla(".").unwrap();
        let assets = lifecycle.assets.as_ref().unwrap();
        let first = EntityAnimation::load(&placed[0], assets, &resolver, false, false).unwrap();
        assert!(first.playhead >= 0.2f32 as f64 && first.playhead <= 0.5);
        assert_eq!(first.elapsed, 0.0);
        assert_eq!(
            EntityAnimation::load(&placed[0], assets, &resolver, false, false)
                .unwrap()
                .playhead,
            first.playhead
        );
        placed[0].id.push_str("different");
        assert_ne!(
            EntityAnimation::load(&placed[0], assets, &resolver, false, false)
                .unwrap()
                .playhead,
            first.playhead
        );
    }

    #[test]
    fn viewer_catalog_loads_transition_clips_offsets_and_empty_destination_states() {
        let root = crate::test_support::temp_test_dir("viewer-transition-catalog");
        std::fs::create_dir_all(root.join("gfx/models")).unwrap();
        let asset = "pdxmesh = {\n name = mesh\n file = \"gfx/models/test.mesh\"\n scale = 3\n animation = { id = intro type = intro_clip }\n}\nanimation = { name = intro_clip file = \"test.anim\" }\nentity = {\n name = entity\n pdxmesh = mesh\n scale = 2\n default_state = initial\n state = { name = initial animation = intro looping = no next_state = lit }\n state = { name = lit length = 1 looping = no next_state = off time_offset = 0.5 event = { time = 0.6 light = flash } }\n state = { name = off }\n}\nlight = {\n name = flash\n color = { r = 1 g = 1 b = 1 }\n intensity = 1\n radius = 8\n falloff = 1\n position = { x = 1 y = 0 z = 0 }\n}";
        std::fs::write(root.join("gfx/models/test.asset"), asset).unwrap();
        std::fs::write(
            root.join("gfx/models/test.mesh"),
            crate::pdx_mesh::test_fixtures::synthetic_mesh_bytes(0.0),
        )
        .unwrap();
        std::fs::write(
            root.join("gfx/models/test.anim"),
            crate::pdx_mesh::test_fixtures::synthetic_anim_bytes(),
        )
        .unwrap();
        let resolver = GameFileResolver::vanilla(&root).unwrap();
        let paths = [
            "gfx/models/test.asset",
            "gfx/models/test.mesh",
            "gfx/models/test.anim",
        ]
        .map(String::from)
        .into_iter()
        .collect();
        let (resolver, _) = crate::test_support::admitted_vanilla_resolver(
            &resolver,
            &paths,
            "viewer-transition-catalog",
        );
        let (_, placed, mut viewer) = fixture(1);
        let mut lifecycle = ViewerLightLifecycle::default();
        viewer.animation_time_f32_bits = 0.01f32.to_bits();
        viewer.frame_delta_seconds_f32_bits = 0.01f32.to_bits();
        lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        assert_eq!(
            lifecycle.entities["placed-000"].next_state.as_deref(),
            Some("lit")
        );
        assert!(lifecycle.entities["placed-000"].length > 0.1);
        viewer.animation_time_f32_bits = 0.21f32.to_bits();
        viewer.frame_delta_seconds_f32_bits = 0.2f32.to_bits();
        lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        assert_eq!(
            lifecycle.entities["placed-000"].next_state.as_deref(),
            Some("off")
        );
        assert_eq!(lifecycle.entities["placed-000"].playhead, 0.5);
        viewer.animation_time_f32_bits = 0.41f32.to_bits();
        let lit = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        let census = lit
            .with_rows(|rows| admit_light_census(Ok(rows), [0., 0., 0.125, 0.125]))
            .unwrap();
        assert_eq!(census.counts.fired_instances, 1);
        assert_eq!(
            census.lights[0]
                .position_radius_f32_bits
                .map(f32::from_bits),
            [22., 0., 16., 8.]
        );
        viewer.animation_time_f32_bits = 0.91f32.to_bits();
        viewer.frame_delta_seconds_f32_bits = 0.5f32.to_bits();
        let off = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        assert!(off.instances.is_empty());
        assert!(lifecycle.entities["placed-000"].events.is_empty());
        assert!(lifecycle.entities["placed-000"].next_state.is_none());
        assert!(!lifecycle.entities["placed-000"].looping);
    }

    #[test]
    fn viewer_census_enters_authored_next_state_and_resets_across_population_gaps() {
        let (mut lifecycle, placed, mut viewer) = fixture(1);
        let state = lifecycle.entities.get_mut("placed-000").unwrap();
        Arc::make_mut(&mut state.events).clear();
        state.length = 0.5;
        state.next_state = Some("lit".into());
        let poses = parse_building_entity_poses("entity = {\n name = entity\n scale = 2\n default_state = initial\n state = { name = initial length = 0.5 looping = no next_state = lit }\n state = { name = lit event = { time = 0 light = flash } }\n}", "fixture.asset").unwrap();
        assert_eq!(poses[0].states[0].next_state.as_deref(), Some("lit"));
        Arc::get_mut(lifecycle.assets.as_mut().unwrap())
            .unwrap()
            .poses = poses
            .into_iter()
            .map(|p| (p.entity_name.clone(), p))
            .collect();
        let resolver = GameFileResolver::vanilla(".").unwrap();
        let first = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        assert!(first.instances.is_empty());
        viewer.animation_time_f32_bits = 1f32.to_bits();
        let lit = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        assert_eq!(lit.instances.len(), 1);
        lifecycle.reset_population();
        viewer.animation_time_f32_bits = 1.5f32.to_bits();
        let restored = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        assert!(restored.instances.is_empty());
        assert_eq!(lifecycle.entities["placed-000"].playhead, 0.0);
        viewer.animation_time_f32_bits = 2f32.to_bits();
        let lit = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        assert_eq!(lit.instances.len(), 1);
        assert_eq!(lit.instances[0].3.age_seconds(), 0.0);
    }

    fn animation(looping: bool, once: bool) -> EntityAnimation {
        EntityAnimation {
            include_particle_only_events: false,
            next_state: None,
            authored_speed: 1.0,
            scale: 2.0,
            token: "entity".into(),
            requested_state: None,
            events: Arc::new(vec![BuildingEntityEvent {
                particle: None,
                keep_particle: false,
                entity: None,
                life: None,
                time: Some(0.25),
                node: Some("lamp".into()),
                light: Some("flash".into()),
                trigger_once: once,
            }]),
            speed: 1.0,
            visible: true,
            length: if looping { 1.0 } else { -1.0 },
            looping,
            elapsed: 0.0,
            playhead: 0.0,
            model: Some(Arc::new(PdxModel {
                lod_distances: vec![],
                shapes: vec![],
                locators: vec![PdxLocator {
                    name: "lamp".into(),
                    position: [3.0, 4.0, 5.0],
                    rotation_quat: [0.0, 0.0, 0.0, 1.0],
                    parent_bone: None,
                    transform_4x4: None,
                }],
            })),
            clip: None,
        }
    }

    #[test]
    fn viewer_events_use_half_open_playhead_intervals_and_unscaled_trigger_once_elapsed() {
        let mut state = animation(true, false);
        assert!(state.advance(0.25).is_empty());
        assert_eq!(state.advance(0.25), [0]);
        assert!(state.advance(0.5).is_empty());
        assert_eq!(state.advance(0.5), [0]);
        let mut once = animation(true, true);
        once.speed = 2.0;
        assert_eq!(once.advance(0.2), [0]);
        assert!(once.advance(0.4).is_empty());
        assert_eq!(once.advance(0.1), [0]);
        assert!(once.advance(1.0).is_empty());
        let mut idle = animation(false, false);
        assert_eq!(idle.advance(2.0), [0]);
        assert_eq!(idle.playhead, 2.0);
        assert!(idle.advance(10.0).is_empty());
    }

    #[test]
    fn viewer_light_instances_persist_with_f32_ages_and_xmap12_world_transforms() {
        let templates = parse_light_templates("light = {\n name = flash\n color = { r = 1 g = 1 b = 1 }\n intensity = 1\n radius = 8\n falloff = 1\n position = { x = 0 y = 0 z = 0 }\n}", "fixture.asset").unwrap();
        let mut lifecycle = ViewerLightLifecycle {
            assets: Some(Arc::new(Assets {
                models: Mutex::new(BTreeMap::new()),
                animations: Mutex::new(BTreeMap::new()),
                poses: BTreeMap::new(),
                static_attachments: BTreeSet::new(),
                meshes: BTreeMap::new(),
                mesh_clips: BTreeMap::new(),
                clips: BTreeMap::new(),
                templates,
            })),
            entities: BTreeMap::from([("placed".into(), animation(true, false))]),
            ..Default::default()
        };
        let mut world = [0.0f32; 16];
        for i in [0, 5, 10, 15] {
            world[i] = 1.0;
        }
        world[12] = 16.0;
        world[14] = 24.0;
        let placed = vec![Xmap09PlacedEntity {
            id: "placed".into(),
            entity_token: "entity".into(),
            state: None,
            animate: true,
            world_matrix_f32_bits: world.map(f32::to_bits),
        }];
        let mut viewer = ViewerOwnedMapState {
            sky_enabled: true,            viewport_width: 1,
            viewport_height: 1,
            camera: None,
            observer_country_id: None,
            map_mode_id: "political".into(),
            map_mode_selection_epoch: 0,
            selected_object_ids: vec![],
            input_revision: 0,
            animation_time_f32_bits: 0.5f32.to_bits(),
            frame_delta_seconds_f32_bits: 0.5f32.to_bits(),
        };
        let resolver = GameFileResolver::vanilla(".").unwrap();
        let rows = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        let census = rows
            .with_rows(|rows| admit_light_census(Ok(rows), [0.0, 0.0, 0.125, 0.125]))
            .unwrap();
        assert_eq!(census.counts.placed_entities, 1);
        assert_eq!(census.counts.fired_instances, 1);
        assert_eq!(
            census.lights[0]
                .position_radius_f32_bits
                .map(f32::from_bits),
            [22.0, 8.0, 34.0, 8.0]
        );
        viewer.animation_time_f32_bits = 1.0f32.to_bits();
        lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        viewer.animation_time_f32_bits = 1.5f32.to_bits();
        let rows = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer, None)
            .unwrap();
        assert_eq!(rows.instances.len(), 2);
        assert_eq!(rows.instances[0].3.age_seconds(), 1.0);
        assert_eq!(rows.instances[1].3.age_seconds(), 0.0);
        viewer.animation_time_f32_bits = 2.0f32.to_bits();
        let empty = lifecycle
            .derive_light_instance_rows(&resolver, &[], &viewer, None)
            .unwrap();
        assert!(empty.instances.is_empty());
        assert!(empty.entities.is_empty());
    }
}
