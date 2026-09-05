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
    meshes: BTreeMap<String, BuildingPdxmeshBinding>,
    mesh_clips: BTreeMap<String, BTreeMap<String, String>>,
    clips: BTreeMap<String, String>,
    templates: Vec<Xmap09LightTemplate>,
}

impl Assets {
    fn model(&self, resolver: &GameFileResolver, path: &str) -> Result<Arc<PdxModel>> {
        let mut cache = self.models.lock().map_err(|_| "light mesh cache is poisoned")?;
        if let Some(model) = cache.get(path) {
            return Ok(model.clone());
        }
        let model = Arc::new(extract_pdx_model(&parse_pdx_asset(&read_asset(resolver, path)?)?)?);
        cache.insert(path.into(), model.clone());
        Ok(model)
    }

    fn animation(&self, resolver: &GameFileResolver, path: &str) -> Result<Arc<PdxAnimData>> {
        let mut cache = self.animations.lock().map_err(|_| "light clip cache is poisoned")?;
        if let Some(clip) = cache.get(path) {
            return Ok(clip.clone());
        }
        let clip = Arc::new(extract_pdx_anim(&parse_pdx_asset(&read_asset(resolver, path)?)?)?);
        cache.insert(path.into(), clip.clone());
        Ok(clip)
    }

    fn load(resolver: &GameFileResolver) -> Result<Self> {
        let mut text = String::new();
        let mut clips = BTreeMap::new();
        for extension in ["asset", "gfx"] {
            for key in resolver.require_complete_admitted_dir(
                "",
                Some(extension),
                true,
                "XMAP09 asset events",
            )? {
                let source = latin1_bytes_to_string(&resolver.read_admitted_used_content(&key)?);
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
            templates: parse_light_templates(&text, "authenticated asset namespace")?,
        })
    }
}

#[derive(Clone)]
struct EntityAnimation {
    next_state: Option<String>,
    authored_speed: f64,
    scale: f32,
    token: String,
    requested_state: Option<String>,
    events: Arc<Vec<BuildingEntityEvent>>,
    speed: f64,
    length: f64,
    looping: bool,
    elapsed: f64,
    playhead: f64,
    model: Option<Arc<PdxModel>>,
    clip: Option<Arc<PdxAnimData>>,
}

struct FiredInstance {
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
    assets: Option<Arc<Assets>>,
    asset_refusal: Option<Xmap09LightCensusRefusal>,
    entities: BTreeMap<String, EntityAnimation>,
    children: BTreeMap<String, ChildEntity>,
    instances: Vec<FiredInstance>,
    next_instance: u64,
    last_presentation_time: Option<f32>,
}

pub(crate) struct DerivedLightRows {
    entities: Vec<(String, Arc<Vec<BuildingEntityEvent>>, [f32; 16])>,
    entity_indices: BTreeMap<String, usize>,
    instances: Vec<(String, String, usize, Xmap09LightInstanceClock)>,
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
            self.entity_indices.get(id)
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
                            &self.entities[self.entity_indices[instance.entity_id]].1[instance.event_index],
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
    ) -> Result<Self> {
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
            (e.light.is_some() || e.entity.is_some())
                && (e.time.is_none_or(|t| !t.is_finite() || t < 0.0)
                    || e.life.is_some_and(|t| !t.is_finite()))
        }) {
            return Err("entity event clock is invalid".into());
        }
        let has_lights = events
            .iter()
            .any(|e| e.light.is_some() || e.entity.is_some());
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
            next_state: state.and_then(|s| s.next_state.clone()),
            authored_speed: state.and_then(|s| s.animation_speed).unwrap_or(1.0) as f64,
            scale: entity_scale * mesh_scale,
            token: row.entity_token.clone(),
            requested_state: row.state.clone(),
            events: Arc::new(events),
            speed: 1.0,
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
        self.elapsed += delta;
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
        self.events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| {
                let time = event.time? as f64;
                if event.light.is_none() && event.entity.is_none()
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
        for event_index in fired {
            let state = &self.entities[id];
            let event = state.events[event_index].clone();
            let speed = state.speed;
            if event.light.is_some() {
                self.instances.push(FiredInstance {
                    id: format!("{id}:light:{event_index}:{}", self.next_instance),
                    entity_id: id.into(),
                    event: event.clone(),
                    clock: Xmap09LightInstanceClock::default(),
                });
                self.next_instance += 1;
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
                let mut child = EntityAnimation::load(&row, assets, resolver, false)?;
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
                let mut next = EntityAnimation::load(&row, assets, resolver, require_pose)?;
                next.speed = speed;
                next.requested_state = requested_state;
                self.instances
                    .retain(|i| i.entity_id != id || i.event.keep_particle);
                let removed = self
                    .children
                    .iter()
                    .filter(|(_, c)| c.parent == id && c.life < 0.0)
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                for child in removed {
                    self.remove_entity(&child);
                }
                self.entities.insert(id.into(), next);
            }
        }
        Ok(())
    }

    pub(crate) fn reset_population(&mut self) {
        self.entities.clear();
        self.children.clear();
        self.instances.clear();
        self.last_presentation_time = None;
    }

    pub(crate) fn derive_light_instance_rows(
        &mut self,
        resolver: &GameFileResolver,
        placed: &[Xmap09PlacedEntity],
        viewer: &ViewerOwnedMapState,
    ) -> std::result::Result<DerivedLightRows, Xmap09LightCensusRefusal> {
        let now = f32::from_bits(viewer.animation_time_f32_bits);
        let delta = viewer.frame_delta_seconds();
        if !now.is_finite() || !delta.is_finite() || delta < 0.0 {
            return Err(missing("viewer.renderer_clock"));
        }
        if self.last_presentation_time.is_some_and(|last| now < last) {
            self.reset_population();
        }
        self.last_presentation_time = Some(now);
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
                self.remove_entity(&row.id);
                self.entities.insert(
                    row.id.clone(),
                    EntityAnimation::load(row, &assets, resolver, false).map_err(|e| {
                        missing(format!("assets.light_events.{}: {e}", row.entity_token))
                    })?,
                );
            }
        }
        {
            for instance in &mut self.instances {
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
        let entity_indices = entities.iter().enumerate()
            .map(|(index, row)| (row.0.clone(), index)).collect::<BTreeMap<_, _>>();
        let instances = self
            .instances
            .iter()
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
        Ok(DerivedLightRows {
            entities,
            entity_indices,
            instances,
            poses,
            children: self.children.clone(),
            assets,
        })
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

    #[test]
    fn paired_light_event_keeps_its_authored_particle_dependency() {
        let poses = parse_building_entity_poses(r#"
entity = {
    name = paired
    state = {
        name = idle
        event = { time = 0 light = flash particle = smoke }
    }
}
"#, "paired.asset").unwrap();
        let event = &poses[0].states[0].events[0];
        assert_eq!(event.particle.as_deref(), Some("smoke"));
        assert!(matches!(resolve_light_event(event, &[], &[], &[], &Xmap09LightInstanceClock::default()),
            Err(Some(Xmap09LightEventRefusal::PairedParticleRuntimeUnavailable { particle })) if particle == "smoke"));
    }

    #[test]
    fn prepared_light_rows_share_catalogs_events_and_cached_asset_payloads() {
        let (mut lifecycle, placed, viewer) = fixture(1);
        let resolver = GameFileResolver::vanilla(".").unwrap();
        let assets = lifecycle.assets.as_ref().unwrap().clone();
        let model = lifecycle.entities["placed-000"].model.as_ref().unwrap().clone();
        assets.models.lock().unwrap().insert("admitted.mesh".into(), model.clone());
        let cached = assets.model(&resolver, "admitted.mesh").unwrap();
        assert!(Arc::ptr_eq(&cached, &model));
        let rows = lifecycle.derive_light_instance_rows(&resolver, &placed, &viewer).unwrap();
        assert!(Arc::ptr_eq(&rows.assets, &assets));
        assert!(Arc::ptr_eq(&rows.entities[0].1, &lifecycle.entities["placed-000"].events));
        assert!(Arc::ptr_eq(&rows.poses["placed-000"].events, &lifecycle.entities["placed-000"].events));
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
            .derive_light_instance_rows(&resolver, &placed, &viewer)
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
            .derive_light_instance_rows(&resolver, &placed, &viewer)
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
        Arc::make_mut(&mut lifecycle.entities.get_mut("placed-064").unwrap().events)[0].node = Some("missing".into());
        let resolver = GameFileResolver::vanilla(".").unwrap();
        let rows = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer)
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
        let event = &mut Arc::make_mut(&mut lifecycle.entities.get_mut("placed-000").unwrap().events)[0];
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
            .derive_light_instance_rows(&resolver, &placed, &viewer)
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
            .derive_light_instance_rows(&resolver, &placed, &viewer)
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
        let first = EntityAnimation::load(&placed[0], assets, &resolver, false).unwrap();
        assert!(first.playhead >= 0.2f32 as f64 && first.playhead <= 0.5);
        assert_eq!(first.elapsed, 0.0);
        assert_eq!(
            EntityAnimation::load(&placed[0], assets, &resolver, false)
                .unwrap()
                .playhead,
            first.playhead
        );
        placed[0].id.push_str("different");
        assert_ne!(
            EntityAnimation::load(&placed[0], assets, &resolver, false)
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
        viewer.frame_delta_seconds_f32_bits = 0.01f32.to_bits();
        lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer)
            .unwrap();
        assert_eq!(
            lifecycle.entities["placed-000"].next_state.as_deref(),
            Some("lit")
        );
        assert!(lifecycle.entities["placed-000"].length > 0.1);
        viewer.frame_delta_seconds_f32_bits = 0.2f32.to_bits();
        lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer)
            .unwrap();
        assert_eq!(
            lifecycle.entities["placed-000"].next_state.as_deref(),
            Some("off")
        );
        assert_eq!(lifecycle.entities["placed-000"].playhead, 0.5);
        let lit = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer)
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
        viewer.frame_delta_seconds_f32_bits = 0.5f32.to_bits();
        let off = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer)
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
            .derive_light_instance_rows(&resolver, &placed, &viewer)
            .unwrap();
        assert!(first.instances.is_empty());
        viewer.animation_time_f32_bits = 1f32.to_bits();
        let lit = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer)
            .unwrap();
        assert_eq!(lit.instances.len(), 1);
        lifecycle.reset_population();
        viewer.animation_time_f32_bits = 1.5f32.to_bits();
        let restored = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer)
            .unwrap();
        assert!(restored.instances.is_empty());
        assert_eq!(lifecycle.entities["placed-000"].playhead, 0.0);
        viewer.animation_time_f32_bits = 2f32.to_bits();
        let lit = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer)
            .unwrap();
        assert_eq!(lit.instances.len(), 1);
        assert_eq!(lit.instances[0].3.age_seconds(), 0.0);
    }

    fn animation(looping: bool, once: bool) -> EntityAnimation {
        EntityAnimation {
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
            .derive_light_instance_rows(&resolver, &placed, &viewer)
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
            .derive_light_instance_rows(&resolver, &placed, &viewer)
            .unwrap();
        viewer.animation_time_f32_bits = 1.5f32.to_bits();
        let rows = lifecycle
            .derive_light_instance_rows(&resolver, &placed, &viewer)
            .unwrap();
        assert_eq!(rows.instances.len(), 2);
        assert_eq!(rows.instances[0].3.age_seconds(), 1.0);
        assert_eq!(rows.instances[1].3.age_seconds(), 0.0);
        viewer.animation_time_f32_bits = 2.0f32.to_bits();
        let empty = lifecycle
            .derive_light_instance_rows(&resolver, &[], &viewer)
            .unwrap();
        assert!(empty.instances.is_empty());
        assert!(empty.entities.is_empty());
    }
}
