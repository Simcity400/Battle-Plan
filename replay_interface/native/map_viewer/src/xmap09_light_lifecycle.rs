use crate::particle_authored::{entity_token_hash, EntityAttachmentCatalog, ParticleDefinitions};
use crate::particle_runtime::{
    ParticleAction, ParticleHandle, ParticleInstanceUpdate, ParticleLightBinding, ParticleRuntime,
    ViewerEntityManager, ViewerParticleSeed,
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

#[derive(Debug)]
pub(crate) struct ResolvedEntityBinding {
    pub(crate) binding: BuildingEntityBinding,
    pub(crate) declaration_source: portable_content_manifest::UsedContentKey,
    pub(crate) pdxmesh_source: portable_content_manifest::UsedContentKey,
    pub(crate) scale_source: Option<portable_content_manifest::UsedContentKey>,
}

pub(crate) struct SelectedEntityClip {
    pub(crate) name: String,
    pub(crate) source: portable_content_manifest::UsedContentKey,
    pub(crate) animation: Arc<PdxAnimData>,
}

pub(crate) struct ResolvedPdxmeshDeclaration<'a> {
    pub(crate) binding: &'a BuildingPdxmeshBinding,
    pub(crate) source: &'a portable_content_manifest::UsedContentKey,
    pub(crate) declaration: &'a portable_content_manifest::paradox_text::Entry,
}

pub(crate) struct EntityAssets {
    content_lease: Option<crate::mod_resolver::ContentAdmissionLease>,
    entity_sources: BTreeMap<String, portable_content_manifest::UsedContentKey>,
    bindings: BTreeMap<String, std::sync::OnceLock<std::result::Result<Option<ResolvedEntityBinding>, String>>>,
    binding_declarations: Vec<(portable_content_manifest::UsedContentKey, Vec<portable_content_manifest::paradox_text::Entry>)>,
    models: Mutex<BTreeMap<String, Arc<PdxModel>>>,
    animations: Mutex<BTreeMap<String, Arc<PdxAnimData>>>,
    poses: BTreeMap<String, BuildingEntityPose>,
    static_attachments: EntityAttachmentCatalog,
    meshes: BTreeMap<String, BuildingPdxmeshBinding>,
    mesh_sources: BTreeMap<String, portable_content_manifest::UsedContentKey>,
    mesh_declarations: Vec<(portable_content_manifest::UsedContentKey, Vec<portable_content_manifest::paradox_text::Entry>)>,
    mesh_clips: BTreeMap<String, BTreeMap<String, String>>,
    clips: BTreeMap<String, String>,
    templates: Vec<Xmap09LightTemplate>,
}

impl EntityAssets {
    pub(crate) fn resolved_binding(
        &self,
        resolver: &GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        name: &str,
    ) -> Result<Option<&ResolvedEntityBinding>> {
        self.check_admission(resolver, admission)?;
        let Some(cached) = self.bindings.get(name) else { return Ok(None) };
        let resolved = cached.get_or_init(|| self.resolve_binding(name).map_err(|error| error.to_string()))
            .as_ref().map_err(|reason| format!("entity binding {name}: {reason}"))?;
        let Some(resolved) = resolved.as_ref() else { return Ok(None) };
        let source = &resolved.declaration_source;
        if resolved.binding.source != source.normalized_game_relative_path
            || self.entity_sources.get(name) != Some(source) {
            return Err(format!("entity {name} binding differs from its admitted namespace winner").into());
        }
        for source in [Some(source), Some(&resolved.pdxmesh_source), resolved.scale_source.as_ref()]
            .into_iter().flatten() {
            if admission.require_used_content(&source.normalized_game_relative_path)? != source {
                return Err(format!("entity {name} binding field belongs to another content admission").into());
            }
        }
        Ok(Some(resolved))
    }

    pub(crate) fn resolved_pdxmesh(
        &self, resolver: &GameFileResolver, admission: &replay_core::ValidatedContentAdmission, name: &str,
    ) -> Result<Option<(&BuildingPdxmeshBinding, &portable_content_manifest::UsedContentKey)>> {
        self.check_admission(resolver, admission)?;
        let Some(binding) = self.meshes.get(name) else { return Ok(None) };
        let source = self.mesh_sources.get(name).ok_or("pdxmesh declaration source is unavailable")?;
        if binding.source != source.normalized_game_relative_path
            || admission.require_used_content(&source.normalized_game_relative_path)? != source {
            return Err(format!("pdxmesh {name} binding differs from its admitted namespace winner").into());
        }
        Ok(Some((binding, source)))
    }

    pub(crate) fn resolved_pdxmesh_declaration(
        &self, resolver: &GameFileResolver, admission: &replay_core::ValidatedContentAdmission,
        name: &str,
    ) -> Result<Option<ResolvedPdxmeshDeclaration<'_>>> {
        let Some((binding, source)) = self.resolved_pdxmesh(resolver, admission, name)? else {
            return Ok(None);
        };
        let mut winner = None;
        let mut variants = None;
        for (key, declarations) in &self.mesh_declarations {
            for entry in declarations {
                if entry.value.first_scalar_ci("name") != Some(name) { continue; }
                let current = entity_parser_text(entry.value.fields().iter()
                    .filter(|field| field.key_matches("variant"))).0;
                if variants.as_ref().is_some_and(|previous| previous != &current) {
                    return Err(format!("pdxmesh {name} has conflicting admitted variant declarations").into());
                }
                variants = Some(current);
                if key == source && entry.line == binding.block_line {
                    winner = Some(entry);
                }
            }
        }
        let declaration = winner.ok_or("pdxmesh declaration differs from its admitted binding")?;
        Ok(Some(ResolvedPdxmeshDeclaration { binding, source, declaration }))
    }

    pub(crate) fn entity_declarations<'a>(
        &'a self, resolver: &GameFileResolver, admission: &replay_core::ValidatedContentAdmission,
    ) -> Result<impl Iterator<Item = (&'a portable_content_manifest::UsedContentKey,
        &'a portable_content_manifest::paradox_text::Entry)> + 'a> {
        self.check_admission(resolver, admission)?;
        for (source, _) in &self.binding_declarations {
            if admission.require_used_content(&source.normalized_game_relative_path)? != source {
                return Err("entity declaration belongs to another content admission".into());
            }
        }
        Ok(self.binding_declarations.iter().flat_map(|(source, entries)|
            entries.iter().map(move |entry| (source, entry))))
    }

    fn resolve_binding(&self, name: &str) -> Result<Option<ResolvedEntityBinding>> {
        let mut needed = BTreeSet::from([name.to_owned()]);
        loop {
            let before = needed.len();
            for (_, entries) in &self.binding_declarations {
                for entry in entries {
                    let value = &entry.value;
                    if value.first_scalar_ci("name").is_some_and(|name| needed.contains(name))
                        && value.first_scalar_ci("pdxmesh").is_none() {
                        if let Some(base) = value.first_scalar_ci("clone") { needed.insert(base.to_owned()); }
                    }
                }
            }
            if before == needed.len() { break; }
        }
        let mut text = String::new();
        let mut source_lines = Vec::new();
        // Preserve XMAP12's last-file winner while retaining all competing declarations.
        for (source, entries) in self.binding_declarations.iter().rev() {
            let entries = entries.iter().filter(|entry| entry.value.first_scalar_ci("name")
                .is_some_and(|name| needed.contains(name)));
            let (part, lines) = entity_parser_text(entries);
            text.push_str(&part);
            source_lines.extend(lines.into_iter().map(|line| (source, line)));
        }
        let locate = |line: usize| -> Result<(&portable_content_manifest::UsedContentKey, usize)> {
            source_lines.get(line.checked_sub(1).ok_or("entity binding line is zero")?)
                .copied().ok_or_else(|| "entity binding line has no admitted source".into())
        };
        let Some(mut binding) = parse_building_entity_bindings(&text, "authenticated entity namespace")?
            .into_iter().find(|binding| binding.name == name) else { return Ok(None) };
        let (source, block_line) = locate(binding.block_line)?;
        let (pdxmesh_source, pdxmesh_line) = locate(binding.pdxmesh_line)?;
        let scale = binding.scale_line.map(locate).transpose()?;
        binding.source = source.normalized_game_relative_path.clone();
        binding.block_line = block_line;
        binding.name_line = locate(binding.name_line)?.1;
        binding.pdxmesh_line = pdxmesh_line;
        binding.scale_line = scale.as_ref().map(|(_, line)| *line);
        Ok(Some(ResolvedEntityBinding {
            binding, declaration_source: source.clone(), pdxmesh_source: pdxmesh_source.clone(),
            scale_source: scale.map(|(source, _)| source.clone()),
        }))
    }

    fn check_admission(
        &self,
        resolver: &GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
    ) -> Result<()> {
        let lease = resolver.content_admission_lease();
        if self.content_lease.is_none() || self.content_lease != lease
            || lease.as_ref().and_then(|lease| lease.manifest_digest_sha256())
                != Some(admission.manifest_digest_sha256()) {
            return Err("entity definition belongs to another content admission".into());
        }
        Ok(())
    }

    pub(crate) fn definition(
        &self,
        resolver: &GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        name: &str,
        source: &portable_content_manifest::UsedContentKey,
    ) -> Result<(&BuildingEntityPose, crate::particle_authored::EntityAttachments)> {
        self.check_admission(resolver, admission)?;
        if admission.require_used_content(&source.normalized_game_relative_path)? != source {
            return Err("entity definition belongs to another content admission".into());
        }
        if self.entity_sources.get(name) != Some(source) {
            return Err(format!("entity {name} definition differs from its admitted namespace winner").into());
        }
        let pose = self.poses.get(name)
            .ok_or_else(|| format!("entity {name} has no admitted pose"))?;
        Ok((pose, self.static_attachments.resolve(name)?))
    }

    pub(crate) fn selected_state_clip(
        &self,
        resolver: &GameFileResolver,
        admission: &replay_core::ValidatedContentAdmission,
        name: &str,
        source: &portable_content_manifest::UsedContentKey,
        state_ordinal: usize,
    ) -> Result<Option<SelectedEntityClip>> {
        let (pose, _) = self.definition(resolver, admission, name, source)?;
        self.clip_for_state(resolver, pose, state_ordinal)
    }

    fn clip_for_state(
        &self, resolver: &GameFileResolver, pose: &BuildingEntityPose, state_ordinal: usize,
    ) -> Result<Option<SelectedEntityClip>> {
        let state = pose.states.get(state_ordinal).ok_or("entity selected state is unavailable")?;
        let Some(animation) = state.animation.as_ref() else { return Ok(None) };
        let mesh = pose.pdxmesh.as_ref().ok_or("animated entity has no pdxmesh")?;
        let name = self.mesh_clips.get(mesh).and_then(|clips| clips.get(animation))
            .ok_or("entity selected animation binding is unavailable")?;
        let file = self.clips.get(name).ok_or("entity selected animation file is unavailable")?;
        let source = resolver.admitted_used_content_key(file)?
            .ok_or_else(|| format!("entity selected animation asset not admitted: {file}"))?;
        Ok(Some(SelectedEntityClip {
            name: name.clone(), source, animation: self.animation(resolver, file)?,
        }))
    }

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

    pub(crate) fn load(resolver: &GameFileResolver) -> Result<Self> {
        let content_lease = resolver.content_admission_lease()
            .ok_or("entity assets require authenticated content admission")?;
        let mut entity_sources = BTreeMap::new();
        let mut text = String::new();
        let mut binding_declarations = Vec::new();
        let mut clips = BTreeMap::new();
        let mut meshes: BTreeMap<String, BuildingPdxmeshBinding> = BTreeMap::new();
        let mut mesh_sources = BTreeMap::new();
        let mut mesh_declarations = Vec::new();
        let mut static_attachments = EntityAttachmentCatalog::default();
        for extension in ["asset", "gfx"] {
            for key in resolver.require_complete_admitted_dir(
                "",
                Some(extension),
                true,
                "XMAP09 asset events",
            )? {
                let source = latin1_bytes_to_string(&resolver.read_admitted_used_content(&key)?);
                let (mut document, _) = if extension == "asset" {
                    portable_content_manifest::paradox_text::parse_asset_str(&source)
                        .map_err(|e| format!("{}: {e:?}", key.normalized_game_relative_path))?
                } else {
                    (portable_content_manifest::paradox_text::parse_str_strict(&source)
                        .map_err(|e| format!("{}: {e:?}", key.normalized_game_relative_path))?,
                        std::borrow::Cow::Borrowed(source.as_str()))
                };
                portable_content_manifest::paradox_text::expand_file_scoped_variables(&mut document)
                    .map_err(|error| format!("{}: {error}", key.normalized_game_relative_path))?;
                fn retain_meshes(
                    entries: &[portable_content_manifest::paradox_text::Entry],
                    out: &mut Vec<portable_content_manifest::paradox_text::Entry>,
                ) {
                    for entry in entries {
                        if entry.key_matches("pdxmesh") && entry.value.as_object().is_some() {
                            out.push(entry.clone());
                        } else {
                            retain_meshes(entry.value.fields(), out);
                        }
                    }
                }
                let mut declarations = Vec::new();
                retain_meshes(document.fields(), &mut declarations);
                mesh_declarations.push((key.clone(), declarations));
                let (source, source_lines) = entity_parser_text(document.fields().iter());
                for entity in document.values_ci("entity") {
                    if let Some(name) = entity.fields().iter().rev()
                        .find(|entry| entry.key_matches("name")).and_then(|entry| entry.value.as_scalar()) {
                        static_attachments.insert(name, entity);
                        entity_sources.insert(name.to_owned(), key.clone());
                    }
                }
                for mut mesh in parse_building_pdxmesh_bindings(&source, &key.normalized_game_relative_path)? {
                    let locate = |line: usize| source_lines.get(line.saturating_sub(1)).copied()
                        .ok_or("pdxmesh binding line has no admitted source");
                    mesh.block_line = locate(mesh.block_line)?;
                    mesh.name_line = locate(mesh.name_line)?;
                    mesh.file_line = locate(mesh.file_line)?;
                    mesh.scale_line = mesh.scale_line.map(locate).transpose()?;
                    if meshes.get(&mesh.name).is_some_and(|previous| !previous.semantically_eq(&mesh)) {
                        return Err(format!("pdxmesh {} has conflicting admitted definitions", mesh.name).into());
                    }
                    mesh_sources.insert(mesh.name.clone(), key.clone());
                    meshes.insert(mesh.name.clone(), mesh);
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
                binding_declarations.push((key, document.fields().iter()
                    .filter(|entry| entry.key_matches("entity")).cloned().collect()));
            }
        }
        let bindings = entity_sources.keys().map(|name| (name.clone(), std::sync::OnceLock::new())).collect();
        Ok(Self {
            content_lease: Some(content_lease),
            entity_sources,
            bindings,
            binding_declarations,
            models: Mutex::new(BTreeMap::new()),
            animations: Mutex::new(BTreeMap::new()),
            poses: parse_building_entity_poses(&text, "authenticated asset namespace")?
                .into_iter()
                .map(|p| (p.entity_name.clone(), p))
                .collect(),
            meshes,
            mesh_sources,
            mesh_declarations,
            mesh_clips: parse_pdxmesh_animation_bindings(&text, "authenticated asset namespace")?,
            clips,
            static_attachments,
            templates: parse_light_templates(&text, "authenticated asset namespace")?,
        })
    }
}

fn entity_parser_text<'a>(
    entries: impl Iterator<Item = &'a portable_content_manifest::paradox_text::Entry>,
) -> (String, Vec<usize>) {
    use portable_content_manifest::paradox_text::{Entry, Scalar, Value};
    fn quoted(text: &mut String, value: &str) {
        text.push('"');
        for ch in value.chars() {
            match ch {
                '\\' => text.push_str("\\\\"),
                '"' => text.push_str("\\\""),
                '\n' => text.push_str("\\n"),
                '\r' => text.push_str("\\r"),
                '\t' => text.push_str("\\t"),
                ch => text.push(ch),
            }
        }
        text.push('"');
    }
    fn emit(entry: &Entry, text: &mut String, lines: &mut Vec<usize>) {
        lines.push(entry.line);
        if let Some(key) = &entry.key {
            if key.is_empty() || key.chars().any(|ch| ch.is_whitespace() || "{}=#\"".contains(ch)) {
                quoted(text, key);
            } else { text.push_str(key); }
            text.push_str(" = ");
        }
        match &entry.value {
            Value::Scalar(Scalar::Bare(value)) => text.push_str(value),
            Value::Scalar(Scalar::Quoted(value)) => quoted(text, value),
            Value::Object(entries) => {
                text.push_str("{\n");
                for entry in entries { emit(entry, text, lines); }
                lines.push(entry.line);
                text.push('}');
            }
        }
        text.push('\n');
    }
    let (mut text, mut lines) = (String::new(), Vec::new());
    for entry in entries { emit(entry, &mut text, &mut lines); }
    (text, lines)
}

#[derive(Clone)]
struct EntityAnimation {
    active_state: Option<String>,
    state_index: Option<usize>,
    include_particle_only_events: bool,
    next_state: Option<String>,
    authored_speed: f64,
    scale: f32,
    token: String,
    requested_state: Option<String>,
    requested_offset_range: Option<u32>,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChildKind {
    Static { variant_hash: u32 },
    Event,
}

#[derive(Clone)]
struct ChildEntity {
    kind: ChildKind,
    parent: String,
    event: BuildingEntityEvent,
    life: f32,
    ordinal: u64,
}

struct EntityManagerRegistration {
    manager: std::sync::Weak<Mutex<ViewerEntityManager>>,
    owner: crate::particle_runtime::EntityManagerOwner,
}

impl Drop for EntityManagerRegistration {
    fn drop(&mut self) {
        if let Some(manager) = self.manager.upgrade() {
            if let Ok(mut manager) = manager.lock() { manager.remove_owner(self.owner); };
        }
    }
}

#[derive(Default)]
pub(crate) struct ViewerLightLifecycle {
    include_particle_only_events: bool,
    assets: Option<Arc<EntityAssets>>,
    particle_definitions: Option<ParticleDefinitions>,
    content_lease: Option<crate::mod_resolver::ContentAdmissionLease>,
    asset_refusal: Option<Xmap09LightCensusRefusal>,
    entities: BTreeMap<String, EntityAnimation>,
    root_registration: Vec<String>,
    children: BTreeMap<String, ChildEntity>,
    instances: Vec<FiredInstance>,
    next_instance: u64,
    next_particle_only_instance: u64,
    next_static_instance: u64,
    entity_manager: Arc<Mutex<ViewerEntityManager>>,
    manager_owner: crate::particle_runtime::EntityManagerOwner,
    root_changes: Vec<crate::particle_runtime::EntityRootChange>,
    manager_registration: Option<EntityManagerRegistration>,
    last_presentation_time: Option<f32>,
    publication_refusal: Option<Xmap09LightCensusRefusal>,
    publication_lease: Option<Arc<()>>,
    weather_binding: Option<(u32, String)>,
}

pub(crate) struct PreparedLightEntityFrame {
    placed: Vec<Xmap09PlacedEntity>,
    root_indices: BTreeMap<String, usize>,
    assets: Arc<EntityAssets>,
    pub(crate) frame: crate::particle_runtime::EntityManagerFrame,
    advance: bool,
    weather_identity: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EntityAnimationReadout<'a> {
    pub(crate) entity_token: &'a str,
    pub(crate) state_ordinal: Option<usize>,
    pub(crate) playhead_seconds: f64,
    pub(crate) authored_speed: f64,
    pub(crate) entity_speed: f64,
}

pub(crate) struct DerivedLightRows {
    manager: std::sync::Weak<Mutex<ViewerEntityManager>>,
    publication: (u32, u64),
    publication_lease: std::sync::Weak<()>,
    entities: Vec<(String, Arc<Vec<BuildingEntityEvent>>, [f32; 16])>,
    entity_indices: BTreeMap<String, usize>,
    instances: Vec<(String, String, usize, Xmap09LightInstanceClock)>,
    particle_bindings: BTreeMap<String, ParticleLightBinding>,
    pub(crate) authored_particles: Vec<crate::production_features::xmap20::Xmap20AuthoredParticleBinding>,
    poses: BTreeMap<String, EntityAnimation>,
    children: BTreeMap<String, ChildEntity>,
    assets: Arc<EntityAssets>,
}

impl DerivedLightRows {
    pub(crate) fn root_animation(
        &self, resolver: &GameFileResolver, admission: &replay_core::ValidatedContentAdmission, id: &str,
    ) -> Result<EntityAnimationReadout<'_>> {
        self.assets.check_admission(resolver, admission)?;
        let _lease = self.publication_lease.upgrade().ok_or("entity_manager.frame.unpublished")?;
        let manager = self.manager.upgrade().ok_or("entity_manager.frame.retired")?;
        if manager.lock().map_err(|_| "entity_manager.lock_poisoned")?
            .publication_revision(self.publication.0) != Some(self.publication.1) {
            return Err("entity_manager.frame.unpublished".into());
        }
        if self.children.contains_key(id) || !self.entity_indices.contains_key(id) {
            return Err(format!("entity_manager.root_animation.{id}: root is unavailable").into());
        }
        let state = self.poses.get(id).ok_or("entity_manager.root_animation.unavailable")?;
        Ok(EntityAnimationReadout {
            entity_token: &state.token, state_ordinal: state.state_index,
            playhead_seconds: state.playhead, authored_speed: state.authored_speed,
            entity_speed: state.speed,
        })
    }

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
        assets: &EntityAssets,
        resolver: &GameFileResolver,
        require_pose: bool,
        include_particle_only_events: bool,
        random: &mut ViewerParticleSeed,
    ) -> Result<Self> {
        Self::load_with_offset_range(row, assets, resolver, require_pose, include_particle_only_events, None, random)
    }

    fn load_with_offset_range(
        row: &Xmap09PlacedEntity,
        assets: &EntityAssets,
        resolver: &GameFileResolver,
        require_pose: bool,
        include_particle_only_events: bool,
        offset_range: Option<f32>,
        random: &mut ViewerParticleSeed,
    ) -> Result<Self> {
        let pose = assets
            .poses
            .get(&row.entity_token)
            .ok_or_else(|| format!("entity {} is unavailable", row.entity_token))?;
        let requested = row.state.as_deref().or(pose.default_state.as_deref());
        let authored = assets.static_attachments.resolve(&row.entity_token)?;
        let (index, start) = match requested {
            Some(requested) => authored.enter_state_with_offset_range(pose, requested, offset_range, random)?,
            None => (None, 0.0),
        };
        let state = index.map(|index| &pose.states[index]);
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
        let mut out = Self {
            active_state: state.map(|state| state.name.clone()),
            state_index: state.and_then(|state| pose.states.iter().position(|candidate| std::ptr::eq(candidate, state))),
            include_particle_only_events,
            next_state: state.and_then(|s| s.next_state.clone()),
            authored_speed: state.and_then(|s| s.animation_speed).unwrap_or(1.0) as f64,
            scale: entity_scale * mesh_scale,
            token: row.entity_token.clone(),
            requested_state: row.state.clone(),
            requested_offset_range: offset_range.map(f32::to_bits),
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
        if has_lights || require_pose || out.next_state.is_some() {
            if let Some(mesh_name) = pose.pdxmesh.as_ref() {
                let mesh = assets.meshes.get(mesh_name)
                    .ok_or("light pdxmesh binding is unavailable")?;
                out.model = Some(assets.model(resolver, &mesh.mesh)?);
            }
        }
        if let Some(index) = index {
            if let Some(selected) = assets.clip_for_state(resolver, pose, index)? {
                out.length = xmap13_clip_length_seconds(&selected.animation) as f64;
                out.clip = Some(selected.animation);
            }
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
        if !self.entity_manager.lock().map_err(|_| "entity_manager.lock_poisoned")?
            .frame_is_published(presentation_time.to_bits()) {
            return Err("entity_manager.frame.unpublished".into());
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
        Self::with_entity_manager(Arc::new(Mutex::new(ViewerEntityManager::default())))
    }

    pub(crate) fn with_entity_manager(entity_manager: Arc<Mutex<ViewerEntityManager>>) -> Self {
        Self { include_particle_only_events: true, entity_manager, ..Default::default() }
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
            .filter(|(_, child)| child.parent == id && child.kind == ChildKind::Event && child.life < 0.0)
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
        if let Some(index) = self.root_registration.iter().position(|root| root == id) {
            self.root_registration.swap_remove(index);
            self.root_changes.push(crate::particle_runtime::EntityRootChange::Remove(id.to_owned()));
        }
        self.entities.remove(id);
        self.instances.retain(|i| i.entity_id != id);
    }

    fn instantiate_entity(
        &mut self,
        row: &Xmap09PlacedEntity,
        assets: &EntityAssets,
        resolver: &GameFileResolver,
        ancestry: &mut Vec<String>,
        random: &mut ViewerParticleSeed,
    ) -> Result<()> {
        self.instantiate_entity_with_offset_range(row, assets, resolver, ancestry, None, random)
    }

    fn instantiate_entity_with_offset_range(
        &mut self,
        row: &Xmap09PlacedEntity,
        assets: &EntityAssets,
        resolver: &GameFileResolver,
        ancestry: &mut Vec<String>,
        offset_range: Option<f32>,
        random: &mut ViewerParticleSeed,
    ) -> Result<()> {
        if ancestry.contains(&row.entity_token) {
            return Err(format!(
                "recursive static entity attachment: {ancestry:?} -> {}",
                row.entity_token
            )
            .into());
        }
        let attachments = assets.static_attachments.resolve(&row.entity_token)?;
        let mut require_pose = false;
        ancestry.push(row.entity_token.clone());
        for (group_index, group) in attachments.groups.iter().enumerate() {
            let Some(variant_index) = group.select(|| random.draw())? else { continue; };
            let variant = &group.variants[variant_index];
            let variant_hash = if variant.name.is_empty() { 0 } else { entity_token_hash(&variant.name)? };
            for (child_index, (node, token)) in variant.children.iter().enumerate() {
                require_pose |= !node.is_empty();
                let ordinal = self.next_static_instance;
                self.next_static_instance = ordinal
                    .checked_add(1)
                    .ok_or("static attachment identity exhausted")?;
                let child_id = format!(
                    "{}:attach:{group_index}:{variant_index}:{child_index}:{ordinal}",
                    row.id
                );
                let child = Xmap09PlacedEntity {
                    id: child_id.clone(),
                    entity_token: token.clone(),
                    state: None,
                    animate: true,
                    world_matrix_f32_bits: IDENTITY.map(f32::to_bits),
                };
                self.instantiate_entity(&child, assets, resolver, ancestry, random)?;
                self.children.insert(
                    child_id,
                    ChildEntity {
                        kind: ChildKind::Static { variant_hash },
                        parent: row.id.clone(),
                        event: BuildingEntityEvent {
                            particle: None,
                            keep_particle: false,
                            entity: None,
                            life: None,
                            time: None,
                            node: Some(node.clone()),
                            light: None,
                            trigger_once: false,
                        },
                        life: -1.0,
                        ordinal,
                    },
                );
            }
        }
        ancestry.pop();
        // 142273fb0 enters the default after recursive construction; 141276c20's
        // requested-state setter follows that complete constructor on the same cursor.
        let mut constructor = row.clone();
        constructor.state = None;
        let state = EntityAnimation::load(&constructor, assets, resolver, require_pose, self.include_particle_only_events, random)?;
        self.entities.insert(row.id.clone(), state);
        self.propagate_static_state(&row.id, assets, resolver, random)?;
        if let Some(requested) = row.state.as_deref() {
            let hash = entity_token_hash(requested)?;
            let present = assets.poses[&row.entity_token].states.iter()
                .map(|state| entity_token_hash(&state.name))
                .collect::<Result<Vec<_>>>()?.contains(&hash);
            if !present {
                let retained = self.entities.get_mut(&row.id).unwrap();
                retained.requested_state = row.state.clone();
                retained.requested_offset_range = offset_range.map(f32::to_bits);
                return Ok(());
            }
            let state = EntityAnimation::load_with_offset_range(row, assets, resolver, require_pose, self.include_particle_only_events, offset_range, random)?;
            self.leave_state(&row.id);
            self.entities.insert(row.id.clone(), state);
            self.propagate_static_state_with_offset_range(&row.id, assets, resolver, offset_range, random)?;
        }
        Ok(())
    }

    fn propagate_static_state(
        &mut self,
        id: &str,
        assets: &EntityAssets,
        resolver: &GameFileResolver,
        random: &mut ViewerParticleSeed,
    ) -> Result<()> {
        self.propagate_static_state_with_offset_range(id, assets, resolver, None, random)
    }

    fn propagate_static_state_with_offset_range(
        &mut self,
        id: &str,
        assets: &EntityAssets,
        resolver: &GameFileResolver,
        offset_range: Option<f32>,
        random: &mut ViewerParticleSeed,
    ) -> Result<()> {
        let mut children = self
            .children
            .iter()
            .filter_map(|(child_id, child)| {
                if child.parent != id {
                    return None;
                }
                match child.kind {
                    ChildKind::Static { variant_hash } => {
                        Some((child.ordinal, child_id.clone(), variant_hash))
                    }
                    ChildKind::Event => None,
                }
            })
            .collect::<Vec<_>>();
        if children.is_empty() {
            return Ok(());
        }
        children.sort_by_key(|child| child.0);
        let parent = &self.entities[id];
        let Some(state_index) = parent.state_index else {
            return Ok(());
        };
        let attachments = assets.static_attachments.resolve(&parent.token)?;
        let (name, mappings) = attachments
            .state_propagation
            .get(state_index)
            .filter(|(name, _)| Some(name) == parent.active_state.as_ref())
            .ok_or("static attachment state and shared entity pose ordinals disagree")?;
        let fallback = entity_token_hash(name)?;
        for (_, child_id, variant_hash) in children {
            let child = &self.entities[&child_id];
            if assets
                .static_attachments
                .resolve(&child.token)?
                .parent_state_override
                .is_some()
            {
                return Err(format!(
                    "entity {} get_state_from_parent registrar token is not admitted",
                    child.token
                )
                .into());
            }
            let target = mappings
                .iter()
                .find(|(key, _)| *key == variant_hash)
                .or_else(|| mappings.iter().find(|(key, _)| *key == 0))
                .map(|(_, target)| *target)
                .unwrap_or(fallback);
            let child = &self.entities[&child_id];
            let pose = &assets.poses[&child.token];
            let mut requested = None;
            for state in &pose.states {
                if entity_token_hash(&state.name)? == target {
                    requested = Some(state.name.clone());
                    break;
                }
            }
            let Some(requested) = requested else {
                continue;
            };
            let row = Xmap09PlacedEntity {
                id: child_id.clone(),
                entity_token: child.token.clone(),
                state: Some(requested),
                animate: true,
                world_matrix_f32_bits: IDENTITY.map(f32::to_bits),
            };
            let mut next = EntityAnimation::load_with_offset_range(
                &row,
                assets,
                resolver,
                self.requires_retained_pose(&child_id),
                self.include_particle_only_events,
                offset_range,
                random,
            )?;
            next.speed = child.speed;
            next.visible = child.visible;
            next.requested_state = child.requested_state.clone();
            next.requested_offset_range = child.requested_offset_range;
            self.leave_state(&child_id);
            self.entities.insert(child_id.clone(), next);
            self.propagate_static_state_with_offset_range(&child_id, assets, resolver, offset_range, random)?;
        }
        Ok(())
    }

    fn requires_retained_pose(&self, id: &str) -> bool {
        self.instances
            .iter()
            .any(|instance| instance.entity_id == id && instance.event.keep_particle)
            || self.children.values().any(|child| {
                child.parent == id
                    && (matches!(child.kind, ChildKind::Static { .. }) || child.life >= 0.0)
                    && child
                        .event
                        .node
                        .as_deref()
                        .is_some_and(|node| !node.is_empty())
            })
    }

    fn static_descendants(&self, id: &str) -> BTreeSet<String> {
        let mut ids = BTreeSet::from([id.to_owned()]);
        let mut pending = vec![id.to_owned()];
        while let Some(parent) = pending.pop() {
            for (child_id, child) in &self.children {
                if child.parent == parent && matches!(child.kind, ChildKind::Static { .. }) {
                    ids.insert(child_id.clone());
                    pending.push(child_id.clone());
                }
            }
        }
        ids
    }

    fn set_static_speed(&mut self, id: &str, speed: f64) {
        for child in self.static_descendants(id) {
            self.entities.get_mut(&child).unwrap().speed = speed;
        }
    }

    fn advance_entity(
        &mut self,
        id: &str,
        delta: f64,
        resolver: &GameFileResolver,
        assets: &EntityAssets,
        ancestry: &mut Vec<String>,
        random: &mut ViewerParticleSeed,
    ) -> Result<()> {
        let existing = self
            .children
            .iter()
            .filter(|(_, c)| c.parent == id && c.kind == ChildKind::Event)
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
        let mut index = 0;
        while index < self.instances.len() {
            let instance = &self.instances[index];
            if instance.entity_id != id {
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
        if self.entities[id].visible {
            for instance in self.instances.iter_mut().filter(|instance| instance.entity_id == id) {
                instance.clock.advance(delta, true)?;
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
                let ordinal = self.next_instance;
                self.next_instance = ordinal.checked_add(1).ok_or("entity event identity exhausted")?;
                self.instantiate_entity(&row, assets, resolver, &mut Vec::new(), random)?;
                self.set_static_speed(&child_id, speed);
                self.children.insert(
                    child_id,
                    ChildEntity {
                        kind: ChildKind::Event,
                        parent: id.into(),
                        event: event.clone(),
                        life,
                        ordinal,
                    },
                );
            }
        }
        let mut children = self
            .children
            .iter()
            .filter(|(_, c)| c.parent == id)
            .map(|(id, c)| (c.kind == ChildKind::Event, c.ordinal, id.clone()))
            .collect::<Vec<_>>();
        children.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
        for (_, _, child) in children {
            ancestry.push(self.entities[&child].token.clone());
            self.advance_entity(&child, delta, resolver, assets, ancestry, random)?;
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
                let require_pose = self.requires_retained_pose(id);
                let mut next = EntityAnimation::load(&row, assets, resolver, require_pose, self.include_particle_only_events, random)?;
                next.speed = speed;
                next.visible = state.visible;
                next.requested_state = requested_state;
                self.leave_state(id);
                self.entities.insert(id.into(), next);
                self.propagate_static_state(id, assets, resolver, random)?;
            }
        }
        Ok(())
    }

    pub(crate) fn weather_particles_bound(&self, time: f32, identity: &str) -> bool {
        self.weather_binding.as_ref().is_some_and(|(bits, hash)| *bits == time.to_bits() && hash == identity)
            && self.entity_manager.lock().is_ok_and(|manager| manager.frame_is_published(time.to_bits()))
    }

    pub(crate) fn reset_population(&mut self) {
        if let Ok(mut manager) = self.entity_manager.lock() {
            manager.remove_owner(self.manager_owner);
            let _ = manager.reset_frame_publication();
        }
        self.publication_lease = None;
        self.root_changes.clear();
        self.entities.clear();
        self.root_registration.clear();
        self.children.clear();
        self.instances.clear();
        self.next_instance = 0;
        self.next_particle_only_instance = 0;
        self.next_static_instance = 0;
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
        let prepared = self.prepare_entity_frame(resolver, placed, viewer, weather)?;
        let manager = Arc::clone(&self.entity_manager);
        let owner = self.manager_owner;
        let frame = prepared.frame;
        let mut update = |id: &str, _scene, random: &mut ViewerParticleSeed|
            self.advance_prepared_root(resolver, &prepared, id, random);
        let result = manager.lock().map_err(|_| missing("entity_manager.lock_poisoned"))?
            .advance_frame(frame, &mut [crate::particle_runtime::EntityManagerDispatch {
                owner, prepared_for: frame, advance_root: &mut update,
            }]);
        if let Err(error) = result {
            let refusal = self.publication_refusal.clone().unwrap_or_else(|| missing(error.to_string()));
            self.publication_refusal = Some(refusal.clone());
            self.last_presentation_time = None;
            self.weather_binding = None;
            return Err(refusal);
        }
        self.finish_entity_frame(resolver, prepared)
    }

    pub(crate) fn prepare_entity_frame(
        &mut self,
        resolver: &GameFileResolver,
        placed: &[Xmap09PlacedEntity],
        viewer: &ViewerOwnedMapState,
        weather: Option<&crate::PreparedXmap19WeatherFrame>,
    ) -> std::result::Result<PreparedLightEntityFrame, Xmap09LightCensusRefusal> {
        self.prepare_entity_frame_with_state_offsets(resolver, placed, viewer, weather, &BTreeMap::new())
    }

    pub(crate) fn prepare_entity_frame_with_state_offsets(
        &mut self,
        resolver: &GameFileResolver,
        placed: &[Xmap09PlacedEntity],
        viewer: &ViewerOwnedMapState,
        weather: Option<&crate::PreparedXmap19WeatherFrame>,
        requested_offset_ranges: &BTreeMap<String, f32>,
    ) -> std::result::Result<PreparedLightEntityFrame, Xmap09LightCensusRefusal> {
        if let Some(refusal) = &self.publication_refusal { return Err(refusal.clone()); }
        self.weather_binding = None;
        self.publication_lease = None;
        self.prepare_entity_population(resolver, placed, viewer, weather, requested_offset_ranges).map_err(|refusal| {
            self.last_presentation_time = None;
            self.publication_refusal = Some(refusal.clone());
            if let Ok(mut manager) = self.entity_manager.lock() {
                manager.refuse_frame(crate::particle_runtime::EntityManagerFrame {
                    presentation_time_f32_bits: viewer.animation_time_f32_bits,
                    delta_seconds_f32_bits: viewer.frame_delta_seconds_f32_bits,
                    cameras: None,
                }, crate::particle_runtime::EntityManagerFrameRefusal::Preparation(refusal.to_string()));
            }
            refusal
        })
    }

    fn prepare_entity_population(
        &mut self,
        resolver: &GameFileResolver,
        placed: &[Xmap09PlacedEntity],
        viewer: &ViewerOwnedMapState,
        weather: Option<&crate::PreparedXmap19WeatherFrame>,
        requested_offset_ranges: &BTreeMap<String, f32>,
    ) -> std::result::Result<PreparedLightEntityFrame, Xmap09LightCensusRefusal> {
        for (id, range) in requested_offset_ranges {
            if !range.is_finite() || !placed.iter().any(|row| row.id == *id && row.state.is_some()) {
                return Err(missing(format!("entity_manager.state_offset.{id}")));
            }
        }
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
            match EntityAssets::load(resolver) {
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
        let manager = Arc::clone(&self.entity_manager);
        let mut manager = manager.lock().map_err(|_| missing("entity_manager.poisoned"))?;
        self.manager_registration.get_or_insert_with(|| EntityManagerRegistration {
            manager: Arc::downgrade(&self.entity_manager), owner: self.manager_owner,
        });
        let (random, scenes) = manager.parts_mut();
        scenes.apply(self.manager_owner, &mut self.root_changes);
        for row in placed {
            let offset_range = requested_offset_ranges.get(&row.id).copied();
            if self
                .entities
                .get(&row.id)
                .is_none_or(|e| e.token != row.entity_token || e.requested_state != row.state
                    || e.requested_offset_range != offset_range.map(f32::to_bits))
            {
                let same_entity = self
                    .entities
                    .get(&row.id)
                    .is_some_and(|e| e.token == row.entity_token);
                let mut update = || -> Result<()> {
                    if same_entity {
                        if let Some(requested) = row.state.as_deref() {
                            let hash = entity_token_hash(requested)?;
                            let present = assets.poses[&row.entity_token].states.iter()
                                .map(|state| entity_token_hash(&state.name)).collect::<Result<Vec<_>>>()?.contains(&hash);
                            if !present {
                                let retained = self.entities.get_mut(&row.id).unwrap();
                                retained.requested_state = row.state.clone();
                                retained.requested_offset_range = offset_range.map(f32::to_bits);
                                return Ok(());
                            }
                        }
                        let previous = &self.entities[&row.id];
                        let mut state = EntityAnimation::load_with_offset_range(row, &assets, resolver,
                            self.requires_retained_pose(&row.id), self.include_particle_only_events, offset_range, random)?;
                        state.speed = previous.speed;
                        state.visible = previous.visible;
                        self.leave_state(&row.id);
                        self.entities.insert(row.id.clone(), state);
                        self.propagate_static_state_with_offset_range(&row.id, &assets, resolver, offset_range, random)?;
                    } else {
                        self.remove_entity(&row.id);
                        self.root_registration.push(row.id.clone());
                        self.root_changes.push(crate::particle_runtime::EntityRootChange::Register {
                            id: row.id.clone(), scene_mask: 7, update_mask: 6,
                        });
                        self.instantiate_entity_with_offset_range(row, &assets, resolver, &mut Vec::new(), offset_range, random)?;
                    }
                    Ok(())
                };
                let result = update();
                scenes.apply(self.manager_owner, &mut self.root_changes);
                result.map_err(|error| match error.downcast::<Xmap09LightCensusRefusal>() {
                    Ok(refusal) => *refusal,
                    Err(error) => missing(format!("assets.light_events.{}: {error}", row.entity_token)),
                })?;
            }
        }
        if let Some(weather) = weather {
            for (id, alpha) in &controls {
                if let Some(alpha) = alpha {
                    self.entities.get_mut(*id).unwrap().visible = *alpha > 0.0;
                    let opacity_ids = self.static_descendants(id);
                    for instance in self.instances.iter_mut().filter(|instance| opacity_ids.contains(&instance.entity_id)) {
                        if let Some(particle) = &mut instance.particle {
                            let refusal = |refusal| Xmap09LightCensusRefusal::ParticleInput {
                                instance: instance.id.clone(), refusal,
                            };
                            if instance.entity_id == *id {
                                particle.runtime.set_visible(particle.handle, *alpha > 0.0).map_err(refusal)?;
                            }
                            if *alpha > 0.0 {
                                particle.runtime.set_opacity(particle.handle, *alpha).map_err(refusal)?;
                            }
                        }
                    }
                }
            }
            for (id, _) in &controls {
                self.set_static_speed(id, f64::from(f32::from_bits(weather.playback_rate_f32_bits)));
            }
        }
        let cameras = match &viewer.camera {
            Some(camera) => crate::particle_runtime::entity_scene_cameras(camera)
                .map_err(|error| missing(format!("entity_manager.camera: {error}")))?,
            None if scenes.requires_camera(u32::MAX) => return Err(missing("entity_manager.camera.absent")),
            None => [false; 3],
        };
        drop(manager);
        let placed = population.into_owned();
        let root_indices = placed.iter().enumerate().map(|(index, row)| (row.id.clone(), index)).collect();
        Ok(PreparedLightEntityFrame {
            placed, root_indices, assets, advance,
            frame: crate::particle_runtime::EntityManagerFrame {
                presentation_time_f32_bits: now.to_bits(), delta_seconds_f32_bits: delta.to_bits(),
                cameras: viewer.camera.as_ref().map(|_| cameras),
            },
            weather_identity: weather.map(|weather| weather.deterministic_sha256.clone()),
        })
    }

    pub(crate) fn entity_manager(&self) -> Arc<Mutex<ViewerEntityManager>> {
        Arc::clone(&self.entity_manager)
    }

    pub(crate) fn refuse_entity_publication(&mut self, reason: &str) {
        self.publication_lease = None;
        if self.publication_refusal.is_none() { self.publication_refusal = Some(missing(reason)); }
        self.last_presentation_time = None;
        self.weather_binding = None;
    }

    pub(crate) fn manager_owner(&self) -> crate::particle_runtime::EntityManagerOwner {
        self.manager_owner
    }

    pub(crate) fn advance_prepared_root(
        &mut self, resolver: &GameFileResolver, prepared: &PreparedLightEntityFrame,
        id: &str, random: &mut ViewerParticleSeed,
    ) -> Result<()> {
        let index = prepared.root_indices.get(id).ok_or_else(|| missing(format!("entity_manager.root.{id}")))?;
        let row = &prepared.placed[*index];
        if !row.animate { return Ok(()); }
        self.advance_entity(id, f32::from_bits(prepared.frame.delta_seconds_f32_bits) as f64,
            resolver, &prepared.assets, &mut vec![row.entity_token.clone()], random)
            .map_err(|error| {
                let refusal = match error.downcast::<Xmap09LightCensusRefusal>() {
                    Ok(refusal) => *refusal,
                    Err(error) => missing(format!("assets.light_events.{}: {error}", row.entity_token)),
                };
                self.publication_refusal = Some(refusal.clone());
                Box::new(refusal) as Box<dyn std::error::Error>
            })
    }

    pub(crate) fn finish_entity_frame(
        &mut self, resolver: &GameFileResolver, prepared: PreparedLightEntityFrame,
    ) -> std::result::Result<DerivedLightRows, Xmap09LightCensusRefusal> {
        let frame = prepared.frame;
        if !self.entity_manager.lock().map_err(|_| missing("entity_manager.lock_poisoned"))?
            .frame_is_published(frame.presentation_time_f32_bits) {
            return Err(missing("entity_manager.frame.unpublished"));
        }
        let weather = prepared.weather_identity.clone();
        match self.materialize_entity_frame(resolver, prepared) {
            Ok(rows) => {
                self.last_presentation_time = Some(f32::from_bits(frame.presentation_time_f32_bits));
                self.weather_binding = weather.map(|identity| (frame.presentation_time_f32_bits, identity));
                Ok(rows)
            }
            Err(refusal) => {
                self.publication_refusal = Some(refusal.clone());
                self.last_presentation_time = None;
                self.weather_binding = None;
                if let Ok(mut manager) = self.entity_manager.lock() {
                    manager.refuse_frame(frame, crate::particle_runtime::EntityManagerFrameRefusal::Finalization(refusal.to_string()));
                }
                Err(refusal)
            }
        }
    }

    fn materialize_entity_frame(
        &mut self, resolver: &GameFileResolver, prepared: PreparedLightEntityFrame,
    ) -> std::result::Result<DerivedLightRows, Xmap09LightCensusRefusal> {
        let PreparedLightEntityFrame { placed, assets, frame, advance, .. } = prepared;
        let now = f32::from_bits(frame.presentation_time_f32_bits);
        let delta = f32::from_bits(frame.delta_seconds_f32_bits);
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
        let revision = self.entity_manager.lock().map_err(|_| missing("entity_manager.lock_poisoned"))?
            .publication_revision(frame.presentation_time_f32_bits)
            .ok_or_else(|| missing("entity_manager.frame.unpublished"))?;
        let publication_lease = Arc::new(());
        self.publication_lease = Some(publication_lease.clone());
        let mut rows = DerivedLightRows {
            publication_lease: Arc::downgrade(&publication_lease),
            manager: Arc::downgrade(&self.entity_manager),
            publication: (frame.presentation_time_f32_bits, revision),
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

    #[test]
    fn shared_entity_definition_resolves_cross_file_clone_with_admitted_identity() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[
            ("gfx/a_clone.asset", br#"entity = { name = clone clone = base }"#),
            ("gfx/z_base.asset", br#"
entity = {
 name = base
 pdxmesh = mesh
 default_state = idle
 state = { name = idle chance = 0 time_offset = 20 }
 state = { name = idle chance = 1 time_offset = { 1 2 } }
 attach = { "" = attached }
}
entity = { name = attached }
"#),
        ]);
        let assets = EntityAssets::load(&content.resolver).unwrap();
        let source = content.admission.require_used_content("gfx/a_clone.asset").unwrap();
        let (pose, attachments) = assets.definition(&content.resolver, &content.admission,
            "clone", source).unwrap();
        assert_eq!(pose.entity_name, "clone");
        assert_eq!(pose.pdxmesh.as_deref(), Some("mesh"));
        assert_eq!(pose.default_state.as_deref(), Some("idle"));
        assert_eq!(pose.states.len(), 2);
        assert_eq!(attachments.state_weights, [("idle".into(), 0), ("idle".into(), 100_000)]);
        assert_eq!(attachments.groups[0].variants[0].children, [("".into(), "attached".into())]);
        let mut expected_random = ViewerParticleSeed { seed: 1, counter: 0 };
        expected_random.draw();
        let expected_start = 1.0 + expected_random.draw() as f32 * f32::from_bits(0x30000000) * 2.0;
        let mut random = ViewerParticleSeed { seed: 1, counter: 0 };
        let (index, start) = attachments.enter_state(pose, "idle", &mut random).unwrap();
        assert_eq!(index, Some(1));
        assert_eq!(start.to_bits(), expected_start.to_bits());
        assert_eq!(random, expected_random);
        assert_eq!(random.counter, 2);
        let base_source = content.admission.require_used_content("gfx/z_base.asset").unwrap();
        assert!(assets.definition(&content.resolver, &content.admission, "clone", base_source)
            .err().unwrap().to_string().contains("namespace winner"));
        let mut changed_source = source.clone();
        changed_source.payload_bytes += 1;
        assert!(assets.definition(&content.resolver, &content.admission, "clone", &changed_source)
            .err().unwrap().to_string().contains("another content admission"));
        let other = Fixture::new(REGISTRATION, "particle={name=other}");
        assert!(assets.definition(&other.resolver, &content.admission, "clone", source)
            .err().unwrap().to_string().contains("another content admission"));
        assert!(assets.definition(&content.resolver, &other.admission, "clone", source)
            .err().unwrap().to_string().contains("another content admission"));
        assert_eq!(random.counter, 2);
    }

    #[test]
    fn shared_resolved_binding_preserves_clone_lines_winner_and_lease() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[
            ("gfx/a_clone.asset", b"entity = {\n name = clone\n clone = base\n scale = 3\n}\nentity={name=meshless}\nentity={name=inherited clone=base}\n"),
            ("gfx/b_same.asset", b"entity={name=same pdxmesh=mesh scale=2}\n"),
            ("gfx/z_base.asset", b"\nentity = {\n name = base\n pdxmesh = mesh\n scale = 2\n}\nentity={name=same pdxmesh=mesh scale=2}\n"),
        ]);
        let assets = EntityAssets::load(&content.resolver).unwrap();
        let resolved = assets.resolved_binding(&content.resolver, &content.admission, "clone").unwrap().unwrap();
        let (binding, source) = (&resolved.binding, &resolved.declaration_source);
        assert_eq!(binding, &BuildingEntityBinding {
            name: "clone".into(), pdxmesh: "mesh".into(), scale: 3.0,
            source: "gfx/a_clone.asset".into(), block_line: 1, name_line: 2,
            pdxmesh_line: 4, scale_line: Some(4),
        });
        assert_eq!(source, content.admission.require_used_content("gfx/a_clone.asset").unwrap());
        assert_eq!(&resolved.pdxmesh_source, content.admission.require_used_content("gfx/z_base.asset").unwrap());
        assert_eq!(resolved.scale_source.as_ref(), Some(source));
        let (pose, _) = assets.definition(&content.resolver, &content.admission, "clone", source).unwrap();
        assert_eq!(pose.pdxmesh.as_deref(), Some(binding.pdxmesh.as_str()));
        assert_eq!(pose.scale, Some(binding.scale));
        let same = assets.resolved_binding(&content.resolver, &content.admission, "same").unwrap().unwrap();
        let (same, winner) = (&same.binding, &same.declaration_source);
        assert_eq!(winner, content.admission.require_used_content("gfx/z_base.asset").unwrap());
        assert_eq!(same.source, winner.normalized_game_relative_path);
        assert_eq!((same.block_line, same.name_line, same.pdxmesh_line, same.scale_line), (7, 7, 7, Some(7)));
        let inherited = assets.resolved_binding(&content.resolver, &content.admission, "inherited").unwrap().unwrap();
        assert_eq!(inherited.declaration_source.normalized_game_relative_path, "gfx/a_clone.asset");
        assert_eq!(inherited.pdxmesh_source.normalized_game_relative_path, "gfx/z_base.asset");
        assert_eq!(inherited.scale_source.as_ref().unwrap().normalized_game_relative_path, "gfx/z_base.asset");
        assert_eq!((inherited.binding.block_line, inherited.binding.pdxmesh_line, inherited.binding.scale_line), (7, 4, Some(5)));
        assert_eq!(inherited.binding.scale, 2.0);
        for name in ["meshless", "absent"] {
            assert!(assets.resolved_binding(&content.resolver, &content.admission, name).unwrap().is_none());
        }
        let other = Fixture::new(REGISTRATION, "particle={name=other}");
        assert!(assets.resolved_binding(&other.resolver, &content.admission, "clone").unwrap_err().to_string().contains("another content admission"));
        assert!(assets.resolved_binding(&content.resolver, &other.admission, "clone").unwrap_err().to_string().contains("another content admission"));
    }

    #[test]
    fn resolved_bindings_expand_variables_per_file_and_isolate_unrelated_binding_failure() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[
            ("gfx/a.asset", br#"@mesh = "mesh_a"
@size = 2.5
entity = {
 name = a
 pdxmesh = @mesh
 scale = @size
}
entity = { name = bad pdxmesh = mesh_bad scale = invalid }
"#),
            ("gfx/z.asset", br#"@mesh = "mesh_z"
@size = 3.5
entity = {
 name = z
 pdxmesh = @mesh
 scale = @size
}
"#),
        ]);
        let assets = EntityAssets::load(&content.resolver).unwrap();
        for (name, mesh, scale, path) in [("a", "mesh_a", 2.5, "gfx/a.asset"), ("z", "mesh_z", 3.5, "gfx/z.asset")] {
            let resolved = assets.resolved_binding(&content.resolver, &content.admission, name).unwrap().unwrap();
            assert_eq!(resolved.binding.pdxmesh, mesh);
            assert_eq!(resolved.binding.scale, scale);
            assert_eq!((resolved.binding.block_line, resolved.binding.name_line, resolved.binding.pdxmesh_line, resolved.binding.scale_line), (3, 4, 5, Some(6)));
            assert_eq!(resolved.declaration_source.normalized_game_relative_path, path);
            assert_eq!(resolved.pdxmesh_source, resolved.declaration_source);
            assert_eq!(resolved.scale_source.as_ref(), Some(&resolved.declaration_source));
            let (pose, _) = assets.definition(&content.resolver, &content.admission, name, &resolved.declaration_source).unwrap();
            assert_eq!(pose.pdxmesh.as_deref(), Some(mesh));
            assert_eq!(pose.scale, Some(scale));
        }
        let failure = assets.resolved_binding(&content.resolver, &content.admission, "bad").unwrap_err().to_string();
        assert!(failure.contains("entity binding bad"));
        assert!(failure.contains("scale is not a float"));
        assert!(assets.resolved_binding(&content.resolver, &content.admission, "a").unwrap().is_some());
        assert!(assets.resolved_binding(&content.resolver, &content.admission, "z").unwrap().is_some());
    }

    #[test]
    fn shared_mesh_and_animation_declarations_keep_the_admitted_source() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[
            ("gfx/entity.asset", br#"
entity={name=base pdxmesh=mesh animation={name=idle}}
entity={name=clone clone=base state={name=walk animation=walking}}
"#),
            ("gfx/a.gfx", br#"objectTypes={pdxmesh={name=mesh file="gfx/models/one.mesh" scale=2}}"#),
            ("gfx/z.gfx", br#"objectTypes = {
 pdxmesh = {
  name = mesh
  file = "gfx/models/one.mesh"
  scale = 2
 }
}
"#),
        ]);
        let assets = EntityAssets::load(&content.resolver).unwrap();
        let (mesh, source) = assets.resolved_pdxmesh(&content.resolver, &content.admission, "mesh").unwrap().unwrap();
        assert_eq!(mesh.mesh, "gfx/models/one.mesh");
        assert_eq!(mesh.scale, 2.0);
        assert_eq!((mesh.block_line, mesh.name_line, mesh.file_line, mesh.scale_line), (2, 3, 4, Some(5)));
        assert_eq!(mesh.source, "gfx/z.gfx");
        assert_eq!(source, content.admission.require_used_content("gfx/z.gfx").unwrap());
        assert!(assets.resolved_pdxmesh(&content.resolver, &content.admission, "absent").unwrap().is_none());
        let declarations = assets.entity_declarations(&content.resolver, &content.admission).unwrap().collect::<Vec<_>>();
        let (base_source, base) = declarations.iter().find(|(_, entry)| entry.value.first_scalar_ci("name") == Some("base")).unwrap();
        assert_eq!(*base_source, content.admission.require_used_content("gfx/entity.asset").unwrap());
        assert_eq!(base.line, 2);
        assert_eq!(base.value.first_value_ci("animation").unwrap().first_scalar_ci("name"), Some("idle"));
        let (_, clone) = declarations.iter().find(|(_, entry)| entry.value.first_scalar_ci("name") == Some("clone")).unwrap();
        assert_eq!(clone.value.first_scalar_ci("clone"), Some("base"));
        assert_eq!(clone.value.first_value_ci("state").unwrap().first_scalar_ci("animation"), Some("walking"));
        let other = Fixture::new(REGISTRATION, "particle={name=other}");
        assert!(assets.resolved_pdxmesh(&other.resolver, &content.admission, "mesh").is_err());
        assert!(assets.entity_declarations(&other.resolver, &content.admission).is_err());
    }

    #[test]
    fn shared_mesh_variants_follow_cross_file_clone_and_expanded_declaration() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let files: &[(&str, &[u8])] = &[
            ("gfx/a_clone.asset", b"entity={name=clone clone=base}\n"),
            ("gfx/z_base.asset", b"entity={name=base pdxmesh=mesh}\n"),
            ("gfx/variants.gfx", br#"@alternate = alternate
@weight = 2
objectTypes = {
 pdxmesh = {
  name = mesh
  file = "gfx/shared.mesh"
  variant = { pdxmesh = ignored pdxmesh = @alternate weight = 1 weight = @weight }
  variant = { pdxmesh = alternate weight = 3 }
 }
 pdxmesh = { name = alternate file = "gfx/shared.mesh" }
}
"#),
        ];
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", files);
        let assets = EntityAssets::load(&content.resolver).unwrap();
        let entity = assets.resolved_binding(&content.resolver, &content.admission, "clone")
            .unwrap().unwrap();
        assert_eq!(entity.declaration_source.normalized_game_relative_path, "gfx/a_clone.asset");
        assert_eq!(entity.pdxmesh_source.normalized_game_relative_path, "gfx/z_base.asset");
        let selected = assets.resolved_pdxmesh_declaration(
            &content.resolver, &content.admission, &entity.binding.pdxmesh).unwrap().unwrap();
        assert_eq!(selected.source, content.admission.require_used_content("gfx/variants.gfx").unwrap());
        assert_eq!(selected.binding.name, "mesh");
        assert_eq!(selected.binding.block_line, selected.declaration.line);
        let variants = selected.declaration.value.fields().iter()
            .filter(|field| field.key_matches("variant")).collect::<Vec<_>>();
        assert_eq!(variants.iter().map(|field| field.line).collect::<Vec<_>>(), [7, 8]);
        assert_eq!(variants[0].value.fields().iter()
            .map(|field| (field.key.as_deref(), field.value.as_scalar())).collect::<Vec<_>>(),
            [(Some("pdxmesh"), Some("ignored")), (Some("pdxmesh"), Some("alternate")),
             (Some("weight"), Some("1")), (Some("weight"), Some("2"))]);
        assert_eq!(variants[1].value.first_scalar_ci("weight"), Some("3"));
        let singleton = assets.resolved_pdxmesh_declaration(
            &content.resolver, &content.admission, "alternate").unwrap().unwrap();
        assert_eq!(singleton.declaration.value.values_ci("variant").count(), 0);
        assert!(assets.resolved_pdxmesh_declaration(
            &content.resolver, &content.admission, "absent").unwrap().is_none());
        let other = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", files);
        assert!(assets.resolved_pdxmesh_declaration(&other.resolver, &content.admission, "mesh").is_err());
        let different = Fixture::new(REGISTRATION, "particle={name=different}");
        assert!(assets.resolved_pdxmesh_declaration(&content.resolver, &different.admission, "mesh").is_err());
    }

    #[test]
    fn conflicting_mesh_variants_refuse_only_the_requested_mesh() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[
            ("gfx/a.gfx", br#"objectTypes={pdxmesh={name=mesh file="gfx/shared.mesh"
 variant={pdxmesh=alternate weight=1}}}"#),
            ("gfx/z.gfx", br#"objectTypes={pdxmesh={name=mesh file="gfx/shared.mesh"
 variant={pdxmesh=alternate weight=2}}
 pdxmesh={name=unrelated file="gfx/unrelated.mesh"}}"#),
        ]);
        let assets = EntityAssets::load(&content.resolver).unwrap();
        let error = assets.resolved_pdxmesh_declaration(&content.resolver, &content.admission, "mesh")
            .err().unwrap().to_string();
        assert!(error.contains("conflicting admitted variant declarations"), "{error}");
        assert!(assets.resolved_pdxmesh_declaration(
            &content.resolver, &content.admission, "unrelated").unwrap().is_some());
    }

    #[test]
    fn constructor_default_then_requested_state_share_cursor_and_publish_retained_clock() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[
            ("gfx/entity.asset", br#"
entity={name=entity default_state=boot
 state={name=boot chance=1 time_offset={10 1}}
 state={name=boot chance=1 time_offset={20 1}}
 state={name=idle chance=1 time_offset={30 1}}
 state={name=idle chance=1 time_offset={40 1}}
}
"#),
        ]);
        let assets = Arc::new(EntityAssets::load(&content.resolver).unwrap());
        let (_, mut placed, viewer) = fixture(1);
        placed[0].state = Some("idle".into());
        let mut expected = ViewerParticleSeed { seed: 1, counter: 0 };
        let definition = &assets.poses["entity"];
        let attachments = assets.static_attachments.resolve("entity").unwrap();
        let (boot_index, _) = attachments.enter_state(definition, "boot", &mut expected).unwrap();
        let (idle_index, start) = attachments.enter_state(definition, "idle", &mut expected).unwrap();
        assert_eq!(expected.counter, 4);
        assert_ne!(boot_index, idle_index);
        let mut shortcut = ViewerParticleSeed { seed: 1, counter: 0 };
        let (_, shortcut_start) = attachments.enter_state(definition, "idle", &mut shortcut).unwrap();
        assert_ne!(start.to_bits(), shortcut_start.to_bits());
        let manager = Arc::new(Mutex::new(ViewerEntityManager::new(ViewerParticleSeed { seed: 1, counter: 0 })));
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        lifecycle.entity_manager = manager.clone();
        lifecycle.assets = Some(assets);
        lifecycle.content_lease = content.resolver.content_admission_lease();
        let rows = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        let observed = rows.root_animation(&content.resolver, &content.admission, &placed[0].id).unwrap();
        let other = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[
            ("gfx/entity.asset", br#"
entity={name=entity default_state=boot
 state={name=boot chance=1 time_offset={10 1}}
 state={name=boot chance=1 time_offset={20 1}}
 state={name=idle chance=1 time_offset={30 1} animation=other_clip}
 state={name=idle chance=1 time_offset={40 1} animation=other_clip}
}
"#),
        ]);
        let other_assets = EntityAssets::load(&other.resolver).unwrap();
        let other_pose = &other_assets.poses["entity"];
        assert_eq!(other_pose.states[idle_index.unwrap()].name, "idle");
        assert_eq!(other_pose.states[idle_index.unwrap()].animation.as_deref(), Some("other_clip"));
        for (resolver, admission) in [(&other.resolver, &other.admission),
            (&content.resolver, &other.admission), (&other.resolver, &content.admission)] {
            assert!(rows.root_animation(resolver, admission, &placed[0].id).unwrap_err()
                .to_string().contains("another content admission"));
        }
        assert_eq!(observed.state_ordinal, idle_index);
        assert_eq!(observed.playhead_seconds, start as f64 + 0.5);
        assert_eq!((observed.authored_speed, observed.entity_speed), (1.0, 1.0));
        assert_eq!(manager.lock().unwrap().random_state(), expected);
        assert_eq!(rows.root_animation(&content.resolver, &content.admission, &placed[0].id).unwrap(), observed);
        let replay = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(replay.root_animation(&content.resolver, &content.admission, &placed[0].id).unwrap(), observed);
        assert!(rows.root_animation(&content.resolver, &content.admission, &placed[0].id).is_err());
        assert_eq!(manager.lock().unwrap().random_state(), expected);
        let frame = crate::particle_runtime::EntityManagerFrame {
            presentation_time_f32_bits: viewer.animation_time_f32_bits,
            delta_seconds_f32_bits: viewer.frame_delta_seconds_f32_bits, cameras: None,
        };
        manager.lock().unwrap().refuse_frame(frame, crate::particle_runtime::EntityManagerFrameRefusal::Finalization("later owner".into()));
        assert!(replay.root_animation(&content.resolver, &content.admission, &placed[0].id).unwrap_err().to_string().contains("unpublished"));
        lifecycle.reset_population();
        let recovered = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert!(recovered.root_animation(&content.resolver, &content.admission, &placed[0].id).is_ok());
        assert!(rows.root_animation(&content.resolver, &content.admission, &placed[0].id).is_err());
        assert_eq!(manager.lock().unwrap().random_state().counter, 8);
    }

    #[test]
    fn requested_offset_override_reaches_child_setters_on_the_same_manager_cursor() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        for requested_range in [0.0, 5.0] {
            let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[
                ("gfx/entity.asset", br#"
entity={name=entity default_state=boot attach={""=child}
 state={name=boot time_offset={10 1}}
 state={name=idle time_offset={20 2}}
}
entity={name=child default_state=idle state={name=idle time_offset={30 100}}}
"#),
            ]);
            let assets = Arc::new(EntityAssets::load(&content.resolver).unwrap());
            let (_, mut placed, viewer) = fixture(1);
            placed[0].state = Some("idle".into());
            let root = assets.static_attachments.resolve("entity").unwrap();
            let child = assets.static_attachments.resolve("child").unwrap();
            let mut expected = ViewerParticleSeed { seed: 1, counter: 0 };
            expected.draw();
            child.enter_state(&assets.poses["child"], "idle", &mut expected).unwrap();
            root.enter_state(&assets.poses["entity"], "boot", &mut expected).unwrap();
            let (root_index, root_start) = root.enter_state_with_offset_range(
                &assets.poses["entity"], "idle", Some(requested_range), &mut expected).unwrap();
            let (_, child_start) = child.enter_state_with_offset_range(
                &assets.poses["child"], "idle", Some(requested_range), &mut expected).unwrap();
            assert_eq!(expected.counter, 5);
            let manager = Arc::new(Mutex::new(ViewerEntityManager::new(ViewerParticleSeed { seed: 1, counter: 0 })));
            let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
            lifecycle.entity_manager = manager.clone();
            lifecycle.assets = Some(assets);
            lifecycle.content_lease = content.resolver.content_admission_lease();
            let ranges = BTreeMap::from([(placed[0].id.clone(), requested_range)]);
            let prepared = lifecycle.prepare_entity_frame_with_state_offsets(
                &content.resolver, &placed, &viewer, None, &ranges).unwrap();
            assert_eq!(manager.lock().unwrap().random_state(), expected);
            let child_id = lifecycle.children.keys().next().unwrap().clone();
            assert_eq!(lifecycle.entities[&child_id].playhead, child_start as f64);
            let frame = prepared.frame;
            let owner = lifecycle.manager_owner;
            let mut update = |id: &str, _scene, random: &mut ViewerParticleSeed|
                lifecycle.advance_prepared_root(&content.resolver, &prepared, id, random);
            manager.lock().unwrap().advance_frame(frame, &mut [crate::particle_runtime::EntityManagerDispatch {
                owner, prepared_for: frame, advance_root: &mut update,
            }]).unwrap();
            let rows = lifecycle.finish_entity_frame(&content.resolver, prepared).unwrap();
            let readout = rows.root_animation(&content.resolver, &content.admission, &placed[0].id).unwrap();
            assert_eq!(readout.state_ordinal, root_index);
            assert_eq!(readout.playhead_seconds, root_start as f64 + 0.5);
            assert!(rows.root_animation(&content.resolver, &content.admission, &child_id).unwrap_err().to_string().contains("root is unavailable"));
            assert_eq!(manager.lock().unwrap().random_state(), expected);
            let mut absent_random = expected;
            let absent = root.enter_state_with_offset_range(
                &rows.assets.poses["entity"], "absent", Some(5.0), &mut absent_random).unwrap();
            assert_eq!(absent, (None, 0.0));
            assert_eq!(absent_random, expected);
        }
    }

    #[test]
    fn constructor_without_authored_default_does_not_invent_idle_entry() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[
            ("gfx/entity.asset", br#"entity={name=entity state={name=idle time_offset={1 2}}}"#),
        ]);
        let assets = EntityAssets::load(&content.resolver).unwrap();
        let (_, mut placed, _) = fixture(1);
        placed[0].state = None;
        let mut random = ViewerParticleSeed { seed: 1, counter: 0 };
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        lifecycle.instantiate_entity(&placed[0], &assets, &content.resolver, &mut Vec::new(), &mut random).unwrap();
        assert_eq!(lifecycle.entities[&placed[0].id].state_index, None);
        assert_eq!(random.counter, 0);
        placed[0].state = Some("missing".into());
        lifecycle.instantiate_entity(&placed[0], &assets, &content.resolver, &mut Vec::new(), &mut random).unwrap();
        assert_eq!(lifecycle.entities[&placed[0].id].state_index, None);
        assert_eq!(random.counter, 0);
    }

    fn fixture_attachment_catalog() -> EntityAttachmentCatalog {
        let mut catalog = EntityAttachmentCatalog::default();
        let (document, _) = portable_content_manifest::paradox_text::parse_asset_str(
            "entity={name=entity} entity={name=child}").unwrap();
        for entity in document.values_ci("entity") {
            catalog.insert(entity.first_scalar_ci("name").unwrap(), entity);
        }
        catalog
    }

    fn fixture(
        count: usize,
    ) -> (
        ViewerLightLifecycle,
        Vec<Xmap09PlacedEntity>,
        ViewerOwnedMapState,
    ) {
        let templates = parse_light_templates("light = {\n name = flash\n color = { r = 1 g = 1 b = 1 }\n intensity = 1\n radius = 8\n falloff = 1\n position = { x = 1 y = 0 z = 0 }\n}", "fixture.asset").unwrap();
        let mut lifecycle = ViewerLightLifecycle {
            assets: Some(Arc::new(EntityAssets {
                content_lease: None,
                entity_sources: BTreeMap::new(),
                bindings: BTreeMap::new(),
                binding_declarations: Vec::new(),
                models: Mutex::new(BTreeMap::new()),
                animations: Mutex::new(BTreeMap::new()),
                poses: BTreeMap::new(),
                static_attachments: fixture_attachment_catalog(),
                meshes: BTreeMap::new(),
                mesh_sources: BTreeMap::new(),
                mesh_declarations: Vec::new(),
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
                lifecycle.root_registration.push(id.clone());
                lifecycle.root_changes.push(crate::particle_runtime::EntityRootChange::Register { id: id.clone(), scene_mask: 7, update_mask: 6 });
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
            unit_models_enabled: true,
            sky_enabled: true,            viewport_width: 1,
            viewport_height: 1,
            camera: Some(crate::particle_runtime::scene_camera_fixture(100_000)),
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
        viewer.animation_time_f32_bits = 2.5f32.to_bits();
        lifecycle.derive_light_instance_rows(&content.resolver, &[], &viewer, None).unwrap();
        assert!(lifecycle.instances.is_empty());
        assert!(!lifecycle.weather_particles_bound(2.5, &middle.deterministic_sha256));
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
    fn unavailable_static_child_is_named_only_when_its_parent_is_instantiated() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}",
            &[("gfx/entity.asset", b"entity={\n name=unbound\n attach={\"\"=child}\n}\nentity={\n name=empty\n}\n")]);
        let (_, mut placed, viewer) = fixture(1);
        let mut lifecycle = ViewerLightLifecycle::default();
        placed[0].entity_token = "empty".into();
        lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        placed[0].entity_token = "unbound".into();
        let error = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).err().unwrap();
        assert!(format!("{error:?}").contains("child attachment source is unavailable"));
    }

    #[test]
    fn static_children_feed_real_particle_draw_in_reverse_order_before_event_children() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=8 duration=-1 emission=4 life=20}}",
            &[("gfx/entity.asset", br#"
entity = {
 name = entity
 scale = 2
 default_state = idle
 attach = { name = lamps "" = first "" = second }
 state = { name = idle event = { time = 0 entity = event_child } }
 state = { name = stopped propagate_state = { lamps = quiet } }
}
entity = { name = first scale = 3 default_state = idle
 state = { name = idle event = { time = 0 light = flash particle = paired } }
 state = { name = quiet }
}
entity = { name = second scale = 4 default_state = idle
 state = { name = idle event = { time = 0 particle = paired } }
 state = { name = quiet }
}
entity = { name = event_child default_state = idle
 state = { name = idle event = { time = 0 particle = paired } }
}
light = {
 name = flash
 color = { r = 1 g = 1 b = 1 }
 intensity = 1
 radius = 8
 falloff = 1
 position = { x = 1 y = 0 z = 0 }
}
"#)]);
        let (_, mut placed, mut viewer) = fixture(1);
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        let rows = lifecycle
            .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
            .unwrap();
        assert_eq!(lifecycle.children.len(), 3);
        assert_eq!(
            lifecycle
                .instances
                .iter()
                .map(|instance| lifecycle.entities[&instance.entity_id].token.as_str())
                .collect::<Vec<_>>(),
            ["second", "first", "event_child"]
        );
        let static_ids = lifecycle
            .children
            .iter()
            .filter(|(_, child)| matches!(child.kind, ChildKind::Static { .. }))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        assert_eq!(static_ids.len(), 2);
        assert_eq!(rows.authored_particles.len(), 3);
        let census = rows
            .with_rows(|rows| admit_light_census(Ok(rows), [0., 0., 0.125, 0.125]))
            .unwrap();
        assert_eq!(census.counts.admitted_instances, 1);
        assert_eq!(
            census.lights[0]
                .position_radius_f32_bits
                .map(f32::from_bits),
            [16.0 + 2.0 * 3.0, 0.0, 16.0, 8.0]
        );
        let camera = crate::particle_runtime::ParticleSortCamera {
            position_f32_bits: [0; 3],
            forward_f32_bits: [0, 0, 1.0f32.to_bits()],
        };
        let mut records = 0;
        let mut draws = Vec::new();
        lifecycle
            .visit_particles_for_draw(0.5, camera, &mut |id, capacity, view| {
                assert_eq!(capacity.particles, 8);
                let binding = rows
                    .authored_particles
                    .iter()
                    .find(|binding| binding.instance_id == id)
                    .unwrap();
                assert_eq!(
                    view.world_matrix_f32_bits(),
                    binding.particle_phase.world_matrix_f32_bits
                );
                for subsystem in view.root_subsystems() {
                    for record in subsystem.records() {
                        record?;
                        records += 1;
                    }
                }
                draws.push(id.to_owned());
                Ok(())
            })
            .unwrap();
        assert_eq!(draws.len(), 3);
        assert!(records > 0);
        for child_id in &static_ids {
            let world = rows.world(child_id).unwrap();
            let scale = if lifecycle.entities[child_id].token == "first" {
                6.0
            } else {
                8.0
            };
            assert_eq!([world[0], world[5], world[10]], [scale; 3]);
            assert_eq!([world[3], world[7], world[11]], [16.0, 0.0, 16.0]);
        }
        lifecycle.set_static_speed(&placed[0].id, 0.25);
        for child_id in &static_ids {
            assert_eq!(lifecycle.entities[child_id].speed, 0.25);
        }
        let event_id = lifecycle
            .children
            .iter()
            .find(|(_, child)| child.kind == ChildKind::Event)
            .unwrap()
            .0;
        assert_eq!(lifecycle.entities[event_id].speed, 1.0);
        placed[0].state = Some("stopped".into());
        viewer.animation_time_f32_bits = 1.0f32.to_bits();
        let rows = lifecycle
            .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
            .unwrap();
        assert_eq!(lifecycle.next_static_instance, 2);
        assert_eq!(
            lifecycle.children.keys().cloned().collect::<Vec<_>>(),
            static_ids
        );
        for child_id in &static_ids {
            assert_eq!(
                lifecycle.entities[child_id].active_state.as_deref(),
                Some("quiet")
            );
        }
        assert!(rows.authored_particles.is_empty());
        assert!(lifecycle.instances.is_empty());
    }

    #[test]
    fn nested_groups_consume_depth_first_random_draws_before_later_parent_groups() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=8 duration=-1 emission=4 life=20}}",
            &[("gfx/entity.asset", br#"
entity = { name = entity default_state = idle
 group = { name = first attach = { "" = nested } }
 group = { name = second attach = { chance = 1 "" = left } attach = { chance = 1 "" = right } }
 state = { name = idle }
}
entity = { name = nested default_state = idle
 group = { attach = { chance = 1 "" = deep } attach = { chance = 1 "" = shallow } }
 state = { name = idle }
}
entity = { name = deep default_state = idle attach = { "" = deep_leaf } state = { name = idle } }
entity = { name = deep_leaf default_state = idle state = { name = idle event = { time = 0 particle = paired } } }
entity = { name = shallow default_state = idle state = { name = idle event = { time = 0 particle = paired } } }
entity = { name = left default_state = idle state = { name = idle event = { time = 0 particle = paired } } }
entity = { name = right default_state = idle state = { name = idle event = { time = 0 particle = paired } } }
"#)]);
        let (_, placed, viewer) = fixture(1);
        let assets = Arc::new(EntityAssets::load(&content.resolver).unwrap());
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        lifecycle.assets = Some(assets.clone());
        lifecycle.content_lease = content.resolver.content_admission_lease();
        let mut random = ViewerParticleSeed { seed: 1, counter: 0 };
        let mut witness = random;
        assert_eq!([witness.draw(), witness.draw(), witness.draw(), witness.draw()],
            [1425195803, 388003433, 1418269600, 1125234208]);
        lifecycle.root_registration.push(placed[0].id.clone());
                lifecycle.root_changes.push(crate::particle_runtime::EntityRootChange::Register { id: placed[0].id.clone(), scene_mask: 7, update_mask: 6 });
        lifecycle.instantiate_entity(&placed[0], &assets, &content.resolver, &mut Vec::new(), &mut random).unwrap();
        assert_eq!(random.counter, 4);
        assert_eq!(lifecycle.next_static_instance, 4);
        assert_eq!(lifecycle.entities.values().map(|state| state.token.as_str()).collect::<BTreeSet<_>>(),
            BTreeSet::from(["entity", "nested", "deep", "deep_leaf", "right"]));
        let rows = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        let camera = crate::particle_runtime::ParticleSortCamera { position_f32_bits: [0; 3], forward_f32_bits: [0, 0, 1.0f32.to_bits()] };
        let mut drawn = Vec::new();
        lifecycle.visit_particles_for_draw(0.5, camera, &mut |id, _, view| {
            let binding = rows.authored_particles.iter().find(|binding| binding.instance_id == id).unwrap();
            drawn.push(rows.poses[&binding.entity_id].token.clone());
            let records = view.root_subsystems().next().unwrap().records()
                .collect::<std::result::Result<Vec<_>, _>>()?;
            assert!(!records.is_empty());
            Ok(())
        }).unwrap();
        assert_eq!(drawn, ["right", "deep_leaf"]);
    }

    #[test]
    fn child_default_state_and_offset_consume_shared_draws_before_parent_next_group() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        for (offset, count, expected) in [("0", 3, "right"), ("{ 0 1 }", 4, "left")] {
            let source = format!(r#"
entity = {{ name = entity default_state = root
 group = {{ attach = {{ "" = child }} }}
 group = {{ attach = {{ chance = 55 "" = left }} attach = {{ chance = 45 "" = right }} }}
 state = {{ name = root }}
 state = {{ name = change propagate_state = idle }}
}}
entity = {{ name = child default_state = idle
 state = {{ name = idle chance = 1 time_offset = {offset} event = {{ time = 0 particle = paired }} }}
 state = {{ name = unrelated }}
 state = {{ name = idle chance = 1 event = {{ time = 0.1 particle = paired }} }}
}}
entity = {{ name = left default_state = idle state = {{ name = idle event = {{ time = 0 particle = paired }} }} }}
entity = {{ name = right default_state = idle state = {{ name = idle event = {{ time = 0 particle = paired }} }} }}
"#);
            let content = Fixture::with_extra_files(REGISTRATION,
                "particle={name=effect subsystem={name=s max_amount=8 duration=-1 emission=4 life=20}}",
                &[("gfx/entity.asset", source.as_bytes())]);
            let (_, mut placed, viewer) = fixture(1);
            placed[0].state = None;
            let assets = Arc::new(EntityAssets::load(&content.resolver).unwrap());
            let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
            lifecycle.assets = Some(assets.clone());
            lifecycle.content_lease = content.resolver.content_admission_lease();
            let mut random = ViewerParticleSeed { seed: 1, counter: 0 };
            lifecycle.root_registration.push(placed[0].id.clone());
                lifecycle.root_changes.push(crate::particle_runtime::EntityRootChange::Register { id: placed[0].id.clone(), scene_mask: 7, update_mask: 6 });
            lifecycle.instantiate_entity(&placed[0], &assets, &content.resolver, &mut Vec::new(), &mut random).unwrap();
            assert_eq!(random.counter, count);
            let child_id = lifecycle.entities.iter().find(|(_, state)| state.token == "child").unwrap().0.clone();
            assert_eq!(lifecycle.entities[&child_id].state_index, Some(0));
            assert_eq!(lifecycle.entities[&child_id].playhead,
                if count == 4 { (1418269600u32 as f32 * f32::from_bits(0x30000000)) as f64 } else { 0.0 });
            assert!(lifecycle.entities.values().any(|state| state.token == expected));
            assert!(!lifecycle.entities.values().any(|state| state.token == if expected == "left" { "right" } else { "left" }));
            lifecycle.entity_manager = Arc::new(Mutex::new(ViewerEntityManager::new(random)));
            let rows = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
            let camera = crate::particle_runtime::ParticleSortCamera {
                position_f32_bits: [0; 3], forward_f32_bits: [0, 0, 1.0f32.to_bits()],
            };
            let mut drawn = Vec::new();
            lifecycle.visit_particles_for_draw(0.5, camera, &mut |id, _, view| {
                let binding = rows.authored_particles.iter().find(|binding| binding.instance_id == id).unwrap();
                drawn.push(rows.poses[&binding.entity_id].token.clone());
                let records = view.root_subsystems().next().unwrap().records()
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                assert!(!records.is_empty());
                Ok(())
            }).unwrap();
            assert_eq!(drawn.first().map(String::as_str), Some(expected));
            assert_eq!(lifecycle.entity_manager.lock().unwrap().random_state().counter, count);
            let mut witness = ViewerParticleSeed { seed: 1, counter: count };
            let selected = usize::from(witness.draw() >= 0x4000_0000);
            if selected == 0 && count == 4 { witness.draw(); }
            placed[0].state = Some("change".into());
            lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
            assert_eq!(lifecycle.entities[&child_id].state_index, Some(selected * 2));
            assert_eq!(lifecycle.entity_manager.lock().unwrap().random_state().counter, witness.counter);
        }
    }

    #[test]
    fn two_roots_share_static_state_and_event_draws_without_identity_reseeding() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let source = r#"
entity = { name = entity default_state = root
 group = { attach = { chance = 55 "" = left } attach = { chance = 45 "" = right } }
 state = { name = root chance = 1 event = { time = 0 entity = event_child } }
 state = { name = root chance = 1 event = { time = 0 entity = event_child } }
}
entity = { name = left default_state = idle
 state = { name = idle chance = 1 event = { time = 0 particle = paired } }
 state = { name = idle chance = 1 event = { time = 0 particle = paired } }
}
entity = { name = right default_state = idle
 state = { name = idle chance = 1 event = { time = 0 particle = paired } }
 state = { name = idle chance = 1 event = { time = 0 particle = paired } }
}
entity = { name = event_child default_state = idle
 state = { name = idle chance = 3 event = { time = 0 particle = paired } }
 state = { name = idle chance = 2 event = { time = 0 particle = paired } }
}
"#;
        let content = Fixture::with_extra_files(REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=8 duration=-1 emission=4 life=20}}",
            &[("gfx/entity.asset", source.as_bytes())]);
        let (_, placed, viewer) = fixture(2);
        let manager = Arc::new(Mutex::new(ViewerEntityManager::new(ViewerParticleSeed { seed: 1, counter: 0 })));
        let mut lifecycle = ViewerLightLifecycle::with_entity_manager(Arc::clone(&manager));
        let rows = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(manager.lock().unwrap().random_state(), ViewerParticleSeed { seed: 1, counter: 8 });
        assert_eq!(lifecycle.entities[&placed[0].id].state_index, Some(1));
        assert_eq!(lifecycle.entities[&placed[1].id].state_index, Some(0));
        let mut children = lifecycle.children.iter().map(|(id, child)| (
            child.parent.clone(), child.kind == ChildKind::Event,
            lifecycle.entities[id].token.clone(), lifecycle.entities[id].state_index,
        )).collect::<Vec<_>>();
        children.sort();
        assert_eq!(children, vec![
            (placed[0].id.clone(), false, "right".into(), Some(0)),
            (placed[0].id.clone(), true, "event_child".into(), Some(0)),
            (placed[1].id.clone(), false, "left".into(), Some(1)),
            (placed[1].id.clone(), true, "event_child".into(), Some(1)),
        ]);
        let camera = crate::particle_runtime::ParticleSortCamera {
            position_f32_bits: [0; 3], forward_f32_bits: [0, 0, 1.0f32.to_bits()],
        };
        let mut drawn = Vec::new();
        lifecycle.visit_particles_for_draw(0.5, camera, &mut |id, _, view| {
            let binding = rows.authored_particles.iter().find(|binding| binding.instance_id == id).unwrap();
            drawn.push(rows.poses[&binding.entity_id].token.clone());
            assert!(!view.root_subsystems().next().unwrap().records()
                .collect::<std::result::Result<Vec<_>, _>>()?.is_empty());
            Ok(())
        }).unwrap();
        assert_eq!(drawn, ["left", "event_child", "right", "event_child"]);
        let same = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(same.authored_particles, rows.authored_particles);
        assert_eq!(manager.lock().unwrap().random_state().counter, 8);
    }

    #[test]
    fn wrap_scene_visits_repeat_actual_root_events_on_one_manager_stream() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=8 duration=-1 emission=4 life=20}}",
            &[("gfx/entity.asset", br#"
entity = { name = entity default_state = idle state = { name = idle
 event = { time = 0.25 entity = event_child }
 event = { time = 0.75 entity = event_child }
 event = { time = 1.25 entity = event_child }
} }
entity = { name = event_child default_state = idle
 state = { name = idle chance = 1 event = { time = 0 particle = paired } }
 state = { name = idle chance = 1 event = { time = 0 particle = paired } }
}
"#)]);
        let (_, placed, mut viewer) = fixture(2);
        viewer.camera = Some(crate::particle_runtime::scene_camera_fixture(1));
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        let rows = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(lifecycle.entity_manager.lock().unwrap().random_state().counter, 6);
        for row in &placed { assert_eq!(lifecycle.entities[&row.id].playhead, 1.5); }
        let mut children = lifecycle.children.iter().map(|(id, child)|
            (child.ordinal, child.parent.clone(), lifecycle.entities[id].state_index.unwrap())).collect::<Vec<_>>();
        children.sort();
        assert_eq!(children.iter().map(|(_, parent, state)| (parent.as_str(), *state)).collect::<Vec<_>>(),
            [(placed[1].id.as_str(),1),(placed[0].id.as_str(),0),
             (placed[1].id.as_str(),1),(placed[0].id.as_str(),1),
             (placed[1].id.as_str(),1),(placed[0].id.as_str(),0)]);
        let mut drawn = Vec::new();
        lifecycle.visit_particles_for_draw(0.5, crate::particle_runtime::ParticleSortCamera {
            position_f32_bits: [0;3], forward_f32_bits: [0,0,1.0f32.to_bits()],
        }, &mut |id, _, view| {
            let binding = rows.authored_particles.iter().find(|binding| binding.instance_id == id).unwrap();
            drawn.push(rows.children[&binding.entity_id].parent.clone());
            assert!(!view.root_subsystems().next().unwrap().records().collect::<std::result::Result<Vec<_>, _>>()?.is_empty());
            Ok(())
        }).unwrap();
        assert_eq!(drawn, [placed[1].id.clone(),placed[0].id.clone(),placed[1].id.clone(),
            placed[0].id.clone(),placed[1].id.clone(),placed[0].id.clone()]);
        let repeated = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(rows.authored_particles, repeated.authored_particles);
        assert_eq!(lifecycle.entity_manager.lock().unwrap().random_state().counter, 6);
        lifecycle.reset_population();
        lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(lifecycle.entity_manager.lock().unwrap().random_state().counter, 12);
    }

    #[test]
    fn root_registration_survives_row_reordering_and_swap_removal_before_event_draws() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=8 duration=-1 emission=4 life=20}}",
            &[("gfx/entity.asset", br#"
entity = { name = entity
 state = { name = quiet }
 state = { name = active event = { time = 0 entity = event_child } }
}
entity = { name = event_child default_state = idle
 state = { name = idle chance = 3 event = { time = 0 particle = paired } }
 state = { name = idle chance = 2 event = { time = 0 particle = paired } }
}
"#)]);
        let (_, mut placed, mut viewer) = fixture(3);
        for row in &mut placed { row.state = Some("quiet".into()); }
        let original = placed.clone();
        let manager = Arc::new(Mutex::new(ViewerEntityManager::default()));
        let mut lifecycle = ViewerLightLifecycle::with_entity_manager(Arc::clone(&manager));
        lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(manager.lock().unwrap().random_state().counter, 0);
        placed.reverse();
        lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(lifecycle.root_registration, original.iter().map(|row| row.id.clone()).collect::<Vec<_>>());
        let mut added = original[1].clone();
        added.id = "new-root".into();
        placed = vec![original[2].clone(), added.clone(), original[0].clone()];
        for row in &mut placed { row.state = Some("active".into()); }
        viewer.animation_time_f32_bits = 1.0f32.to_bits();
        let rows = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(lifecycle.root_registration, [original[0].id.clone(), original[2].id.clone(), added.id.clone()]);
        let state = |parent: &str| {
            let (id, _) = lifecycle.children.iter().find(|(_, child)| child.parent == parent).unwrap();
            lifecycle.entities[id].state_index
        };
        assert_eq!((state(&added.id), state(&original[2].id), state(&original[0].id)),
            (Some(1), Some(0), Some(1)));
        assert_eq!(manager.lock().unwrap().random_state().counter, 3);
        let camera = crate::particle_runtime::ParticleSortCamera {
            position_f32_bits: [0; 3], forward_f32_bits: [0, 0, 1.0f32.to_bits()],
        };
        let mut drawn = Vec::new();
        lifecycle.visit_particles_for_draw(1.0, camera, &mut |id, _, view| {
            let binding = rows.authored_particles.iter().find(|binding| binding.instance_id == id).unwrap();
            drawn.push(rows.children[&binding.entity_id].parent.clone());
            assert!(!view.root_subsystems().next().unwrap().records()
                .collect::<std::result::Result<Vec<_>, _>>()?.is_empty());
            Ok(())
        }).unwrap();
        assert_eq!(drawn, [added.id, original[2].id.clone(), original[0].id.clone()]);
        placed.reverse();
        let repeated = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(repeated.authored_particles, rows.authored_particles);
        assert_eq!(manager.lock().unwrap().random_state().counter, 3);
        lifecycle.reset_population();
        assert!(lifecycle.root_registration.is_empty());
        assert_eq!(manager.lock().unwrap().random_state().counter, 3);
    }

    #[test]
    fn shared_manager_retains_history_across_roots_population_resets_and_consumers() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[("gfx/entity.asset", br#"
entity = { name = entity default_state = idle
 state = { name = idle chance = 1 }
 state = { name = idle chance = 1 }
}
"#)]);
        let (_, mut placed, mut viewer) = fixture(2);
        viewer.frame_delta_seconds_f32_bits = 0;
        let initial = ViewerParticleSeed { seed: 1, counter: 0 };
        let manager = Arc::new(Mutex::new(ViewerEntityManager::new(initial)));
        let mut first = ViewerLightLifecycle::with_entity_manager(Arc::clone(&manager));
        let mut second = ViewerLightLifecycle::with_entity_manager(Arc::clone(&manager));
        first.derive_light_instance_rows(&content.resolver, &placed[..1], &viewer, None).unwrap();
        assert_eq!(first.entities[&placed[0].id].state_index, Some(1));
        first.reset_population();
        second.derive_light_instance_rows(&content.resolver, &placed[1..], &viewer, None).unwrap();
        assert_eq!(second.entities[&placed[1].id].state_index, Some(0));
        assert_eq!(manager.lock().unwrap().random_state().counter, 2);
        second.derive_light_instance_rows(&content.resolver, &placed[1..], &viewer, None).unwrap();
        assert_eq!(manager.lock().unwrap().random_state().counter, 2);
        second.reset_population();
        first.derive_light_instance_rows(&content.resolver, &[], &viewer, None).unwrap();
        first.reset_population();
        assert_eq!(manager.lock().unwrap().random_state().counter, 2);
        first.derive_light_instance_rows(&content.resolver, &placed[..1], &viewer, None).unwrap();
        assert_eq!(manager.lock().unwrap().random_state().counter, 3);
        first.reset_population();
        assert_eq!(manager.lock().unwrap().random_state().counter, 3);
        first.derive_light_instance_rows(&content.resolver, &placed[..1], &viewer, None).unwrap();
        assert_eq!(manager.lock().unwrap().random_state().counter, 4);
        placed[0].id = "identity-is-not-a-seed".into();
        let restarted = Arc::new(Mutex::new(ViewerEntityManager::new(initial)));
        let mut replay = ViewerLightLifecycle::with_entity_manager(Arc::clone(&restarted));
        replay.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(replay.entities[&placed[0].id].state_index, Some(1));
        assert_eq!(replay.entities[&placed[1].id].state_index, Some(0));
        assert_eq!(restarted.lock().unwrap().random_state().counter, 2);
        assert_eq!(manager.lock().unwrap().random_state().counter, 4);
    }

    #[test]
    fn missing_camera_refuses_registered_scene_updates_and_requires_explicit_recovery() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[("gfx/entity.asset", br#"
entity = { name = entity default_state = idle state = { name = idle chance = 1 } state = { name = idle chance = 1 } }
"#)]);
        let (_, placed, mut viewer) = fixture(1);
        let camera = viewer.camera.take();
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        let refusal = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).err().unwrap();
        assert_eq!(refusal, Xmap09LightCensusRefusal::MissingDomain { domain: "entity_manager.camera.absent".into() });
        assert_eq!(lifecycle.entity_manager.lock().unwrap().random_state().counter, 1);
        viewer.camera = camera;
        assert_eq!(lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).err().unwrap(), refusal);
        lifecycle.reset_population();
        lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(lifecycle.entity_manager.lock().unwrap().random_state().counter, 2);
        assert_eq!(lifecycle.entities[&placed[0].id].playhead, 0.5);
    }

    #[test]
    fn separately_owned_roots_share_one_actual_event_and_particle_submission_pass() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=8 duration=-1 emission=4 life=20}}",
            &[("gfx/entity.asset", br#"
entity = { name = entity default_state = idle
 state = { name = idle chance = 1 event = { time = 0 entity = child } }
 state = { name = idle chance = 1 event = { time = 0 entity = child } }
}
entity = { name = child default_state = idle
 state = { name = idle chance = 3 event = { time = 0 particle = paired } }
 state = { name = idle chance = 2 event = { time = 0 particle = paired } }
}
"#)]);
        let (_, placed, viewer) = fixture(2);
        let manager = Arc::new(Mutex::new(ViewerEntityManager::default()));
        let mut first = ViewerLightLifecycle::with_entity_manager(Arc::clone(&manager));
        let mut second = ViewerLightLifecycle::with_entity_manager(Arc::clone(&manager));
        let fp = first.prepare_entity_frame(&content.resolver, &placed[..1], &viewer, None).unwrap();
        let sp = second.prepare_entity_frame(&content.resolver, &placed[1..], &viewer, None).unwrap();
        assert_eq!((first.entities[&placed[0].id].state_index, second.entities[&placed[1].id].state_index), (Some(1),Some(0)));
        assert_eq!(manager.lock().unwrap().random_state().counter, 2);
        struct PreparedOwner<'a> {
            lifecycle: &'a mut ViewerLightLifecycle,
            resolver: &'a GameFileResolver,
            prepared: Option<PreparedLightEntityFrame>,
            frame: crate::particle_runtime::EntityManagerFrame,
            rows: &'a mut Option<DerivedLightRows>,
        }
        impl crate::production_foundation::PreparedEntityManagerOwner for PreparedOwner<'_> {
            fn manager(&self) -> Arc<Mutex<ViewerEntityManager>> { self.lifecycle.entity_manager() }
            fn owner(&self) -> crate::particle_runtime::EntityManagerOwner { self.lifecycle.manager_owner() }
            fn frame(&self) -> crate::particle_runtime::EntityManagerFrame { self.frame }
            fn advance_root(&mut self, root: &str, _: u32, random: &mut ViewerParticleSeed) -> Result<()> {
                self.lifecycle.advance_prepared_root(self.resolver, self.prepared.as_ref().unwrap(), root, random)
            }
            fn finish(&mut self) -> Result<()> {
                *self.rows = Some(self.lifecycle.finish_entity_frame(self.resolver, self.prepared.take().unwrap())?);
                Ok(())
            }
            fn refuse(&mut self, reason: &str) { self.lifecycle.refuse_entity_publication(reason); }
        }
        let (mut frows, mut srows) = (None, None);
        {
            let frame = fp.frame;
            let mut owners: Vec<Box<dyn crate::production_foundation::PreparedEntityManagerOwner + '_>> = vec![
                Box::new(PreparedOwner { lifecycle: &mut first, resolver: &content.resolver, prepared: Some(fp), frame, rows: &mut frows }),
                Box::new(PreparedOwner { lifecycle: &mut second, resolver: &content.resolver, prepared: Some(sp), frame, rows: &mut srows }),
            ];
            crate::production_foundation::dispatch_prepared_entity_owners(&mut owners).unwrap();
        }
        assert_eq!(manager.lock().unwrap().random_state().counter, 4);
        let first_child = first.children.keys().next().unwrap();
        let second_child = second.children.keys().next().unwrap();
        assert_eq!((first.entities[first_child].state_index, second.entities[second_child].state_index), (Some(0),Some(1)));
        for (owner, rows) in [(&mut first, frows.unwrap()), (&mut second, srows.unwrap())] {
            assert_eq!(rows.authored_particles.len(), 1);
            let mut records = 0;
            owner.visit_particles_for_draw(0.5, crate::particle_runtime::ParticleSortCamera {
                position_f32_bits: [0;3], forward_f32_bits: [0,0,1.0f32.to_bits()],
            }, &mut |_, _, view| {
                records += view.root_subsystems().next().unwrap().records().collect::<std::result::Result<Vec<_>, _>>()?.len();
                Ok(())
            }).unwrap();
            assert!(records > 0);
        }
        assert_eq!(manager.lock().unwrap().random_state().counter, 4);
    }

    #[test]
    fn failed_root_update_keeps_shared_draw_history_and_refuses_same_phase_retry() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[("gfx/entity.asset", br#"
entity = { name = entity default_state = idle
 state = { name = idle chance = 1 event = { time = 0 entity = missing_child } }
 state = { name = idle chance = 1 event = { time = 0 entity = missing_child } }
 state = { name = quiet }
}
"#)]);
        let (_, mut placed, viewer) = fixture(2);
        let manager = Arc::new(Mutex::new(ViewerEntityManager::default()));
        let mut lifecycle = ViewerLightLifecycle::with_entity_manager(Arc::clone(&manager));
        let first = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).err().unwrap();
        assert!(format!("{first:?}").contains("missing_child"));
        assert_eq!(manager.lock().unwrap().random_state().counter, 2);
        let retry = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).err().unwrap();
        assert_eq!(retry, first);
        let camera = crate::particle_runtime::ParticleSortCamera {
            position_f32_bits: [0; 3], forward_f32_bits: [0, 0, 1.0f32.to_bits()],
        };
        assert!(lifecycle.visit_particles_for_draw(0.5, camera, &mut |_, _, _| {
            panic!("failed publication reached draw")
        }).unwrap_err().to_string().contains("reset_population required"));
        assert_eq!(manager.lock().unwrap().random_state().counter, 2);
        for row in &mut placed { row.state = Some("quiet".into()); }
        lifecycle.reset_population();
        lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
        assert_eq!(manager.lock().unwrap().random_state().counter, 4);
        assert!(lifecycle.publication_refusal.is_none());
    }

    #[test]
    fn state_selection_preserves_zero_default_signed_weights_and_identical_duplicates() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        for (chance, expected) in [("", 1), ("chance = 42949.67295", 1), ("chance = 010", 0)] {
            let source = format!("entity = {{ name = entity default_state = idle state = {{ name = idle {chance} }} state = {{ name = idle }} }}");
            let content = Fixture::with_extra_files(REGISTRATION, "particle={name=effect}",
                &[("gfx/entity.asset", source.as_bytes())]);
            let assets = EntityAssets::load(&content.resolver).unwrap();
            let (_, placed, _) = fixture(1);
            let mut random = ViewerParticleSeed { seed: 1, counter: 0 };
            let state = EntityAnimation::load(&placed[0], &assets, &content.resolver, false, false, &mut random).unwrap();
            assert_eq!(random.counter, 1);
            assert_eq!(state.state_index, Some(expected));
        }
        let weights = vec![("idle".into(), i32::MAX); 4];
        assert_eq!(crate::particle_authored::native_state_index(&weights, 0x7fff_ffff).unwrap(), 3);
        assert_eq!(crate::particle_authored::native_state_index(&weights, 0x4000_0000).unwrap(), 2);
    }

    #[test]
    fn weather_opacity_reaches_static_particles_but_not_event_children() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=8 duration=-1 emission=4 life=20 color={a=255} texture={file=gfx/weather.dds shader=Weather}}}",
            &[("gfx/entity.asset", br#"
entity = { name = weather default_state = idle attach = { "" = static_child }
 state = { name = idle event = { time = 0.25 particle = paired } event = { time = 0.25 entity = event_child } }
}
entity = { name = static_child default_state = idle state = { name = idle event = { time = 0.25 particle = paired } } }
entity = { name = event_child default_state = idle state = { name = idle event = { time = 0.25 particle = paired } } }
"#), ("gfx/weather.shader", b"Effect Weather = { VertexShader = VS PixelShader = PS }"),
                ("gfx/weather.dds", b"lifecycle fixture: texture is not decoded by this consumer")]);
        let weather = weather_frame(&content.resolver, 1300.0);
        let (_, _, mut viewer) = fixture(0);
        viewer.animation_time_f32_bits = 2.0f32.to_bits();
        viewer.frame_delta_seconds_f32_bits = 2.0f32.to_bits();
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        lifecycle
            .derive_light_instance_rows(&content.resolver, &[], &viewer, Some(&weather))
            .unwrap();
        assert_eq!(lifecycle.instances.len(), 6);
        viewer.animation_time_f32_bits = 2.5f32.to_bits();
        viewer.frame_delta_seconds_f32_bits = 0.5f32.to_bits();
        lifecycle
            .derive_light_instance_rows(&content.resolver, &[], &viewer, Some(&weather))
            .unwrap();
        let mut static_count = 0;
        let mut event_count = 0;
        for instance in &lifecycle.instances {
            let expected = match lifecycle
                .children
                .get(&instance.entity_id)
                .map(|child| child.kind)
            {
                Some(ChildKind::Event) => {
                    event_count += 1;
                    255
                }
                Some(ChildKind::Static { .. }) => {
                    static_count += 1;
                    128
                }
                None => 128,
            };
            let particle = instance.particle.as_ref().unwrap();
            let record = particle
                .runtime
                .gpu_view(particle.handle)
                .unwrap()
                .root_subsystems()
                .next()
                .unwrap()
                .records()
                .next()
                .unwrap()
                .unwrap();
            assert_eq!(record.0[35], expected, "{}", instance.id);
        }
        assert_eq!((static_count, event_count), (2, 2));
    }

    #[test]
    fn zero_weight_static_variant_does_not_require_its_missing_node_or_entity() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(
            REGISTRATION,
            "particle={name=effect}",
            &[(
                "gfx/entity.asset",
                br#"
entity = { name = entity group = {
 attach = { chance = 0 unavailable_node = unavailable_entity }
 attach = { chance = 1 "" = child }
} }
entity = { name = child }
"#,
            )],
        );
        let (_, placed, viewer) = fixture(1);
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        lifecycle
            .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
            .unwrap();
        assert_eq!(lifecycle.children.len(), 1);
        let child = lifecycle.children.keys().next().unwrap();
        assert_eq!(lifecycle.entities[child].token, "child");
        assert!(lifecycle.entities[&placed[0].id].model.is_none());
    }

    #[test]
    fn static_attachment_cycle_poison_survives_retry_and_draw_until_reset() {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let content = Fixture::with_extra_files(
            REGISTRATION,
            "particle={name=effect}",
            &[(
                "gfx/entity.asset",
                br#"entity={name=entity attach={""=child}} entity={name=child attach={""=entity}}"#,
            )],
        );
        let (_, placed, viewer) = fixture(1);
        let mut lifecycle = ViewerLightLifecycle::for_authored_particles();
        let first = lifecycle
            .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
            .err()
            .unwrap();
        assert!(format!("{first:?}").contains("recursive static entity attachment"));
        assert_eq!(lifecycle.next_static_instance, 2);
        assert_eq!(
            lifecycle
                .derive_light_instance_rows(&content.resolver, &placed, &viewer, None)
                .err(),
            Some(first)
        );
        let camera = crate::particle_runtime::ParticleSortCamera {
            position_f32_bits: [0; 3],
            forward_f32_bits: [0, 0, 1.0f32.to_bits()],
        };
        assert!(lifecycle
            .visit_particles_for_draw(0.5, camera, &mut |_, _, _| panic!("poisoned draw"))
            .is_err());
        lifecycle.reset_population();
        assert!(lifecycle.entities.is_empty());
        assert!(lifecycle.publication_refusal.is_none());
        assert_eq!(lifecycle.next_static_instance, 0);
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
        lifecycle.assets = Some(Arc::new(EntityAssets::load(&content.resolver).unwrap()));
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
        Arc::make_mut(&mut lifecycle.entities.get_mut("placed-000").unwrap().events)[0].node =
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
        assert!(rows.poses["placed-000"].attach(0).is_err());
        assert_eq!(rows.instances[0].1, "placed-064");
        assert_eq!(rows.instances[64].1, "placed-000");
        let (mut first_missing, placed, viewer) = fixture(65);
        Arc::make_mut(&mut first_missing.entities.get_mut("placed-064").unwrap().events)[0].node =
            Some("missing".into());
        let rows = first_missing.derive_light_instance_rows(&resolver, &placed, &viewer, None).unwrap();
        assert!(matches!(rows.with_rows(|rows| admit_light_census(Ok(rows), [0., 0., 0.125, 0.125])),
            Err(Xmap09LightCensusRefusal::MissingDomain { domain })
                if domain.contains("placed-064") && domain.contains("light attach node missing")));
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
        let first = EntityAnimation::load(&placed[0], assets, &resolver, false, false, &mut ViewerParticleSeed { seed: 1, counter: 0 }).unwrap();
        assert!(first.playhead >= 0.2f32 as f64 && first.playhead <= 0.5);
        assert_eq!(first.elapsed, 0.0);
        assert_eq!(
            EntityAnimation::load(&placed[0], assets, &resolver, false, false, &mut ViewerParticleSeed { seed: 1, counter: 0 })
                .unwrap()
                .playhead,
            first.playhead
        );
        placed[0].id.push_str("different");
        assert_eq!(
            EntityAnimation::load(&placed[0], assets, &resolver, false, false, &mut ViewerParticleSeed { seed: 1, counter: 0 })
                .unwrap()
                .playhead,
            first.playhead
        );
    }

    fn eventless_clip_fixture() -> crate::particle_authored::test_support::Fixture {
        use crate::particle_authored::test_support::{Fixture, REGISTRATION};
        let mut clip = crate::pdx_mesh::test_fixtures::synthetic_anim_bytes();
        let fps = clip.windows(4).enumerate().filter_map(|(index, bytes)|
            (bytes == 15.0f32.to_le_bytes()).then_some(index)).collect::<Vec<_>>();
        assert_eq!(fps.len(), 1);
        clip[fps[0]..fps[0] + 4].copy_from_slice(&1.0f32.to_le_bytes());
        let mesh = crate::pdx_mesh::test_fixtures::synthetic_mesh_bytes(0.0);
        Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[
            ("gfx/model.mesh", &mesh), ("gfx/test.anim", &clip),
            ("gfx/entity.asset", br#"
@animation_id = idle
@clip_name = shared_clip
@clip_file = test.anim
pdxmesh = { name = mesh file = "gfx/model.mesh"
 animation = { id = @animation_id type = @clip_name }
}
animation = { name = @clip_name file = @clip_file }
entity = { name = entity pdxmesh = mesh default_state = boot
 state = { name = boot animation = @animation_id animation_speed = 2 }
 state = { name = steady animation = @animation_id animation_speed = 2 }
 state = { name = run animation = @animation_id animation_speed = 2 looping = no next_state = done }
 state = { name = done animation = @animation_id animation_speed = 2 }
 state = { name = still }
 state = { name = missing animation = unbound }
}
"#),
        ])
    }

    #[test]
    fn shared_selected_clip_uses_expanded_catalog_and_checks_content_lease() {
        let content = eventless_clip_fixture();
        let assets = EntityAssets::load(&content.resolver).unwrap();
        let source = content.admission.require_used_content("gfx/entity.asset").unwrap();
        let pose = &assets.poses["entity"];
        let index = |name: &str| pose.states.iter().position(|state| state.name == name).unwrap();
        let selected = assets.selected_state_clip(&content.resolver, &content.admission,
            "entity", source, index("boot")).unwrap().unwrap();
        assert_eq!(selected.name, "shared_clip");
        assert_eq!(&selected.source, content.admission.require_used_content("gfx/test.anim").unwrap());
        assert_eq!(xmap13_clip_length_seconds(&selected.animation), 2.0);
        let next = assets.selected_state_clip(&content.resolver, &content.admission,
            "entity", source, index("done")).unwrap().unwrap();
        assert!(Arc::ptr_eq(&selected.animation, &next.animation));
        assert!(assets.selected_state_clip(&content.resolver, &content.admission,
            "entity", source, index("still")).unwrap().is_none());
        assert!(assets.selected_state_clip(&content.resolver, &content.admission,
            "entity", source, index("missing")).err().unwrap().to_string().contains("animation binding"));
        assert!(assets.selected_state_clip(&content.resolver, &content.admission,
            "entity", source, pose.states.len()).err().unwrap().to_string().contains("selected state"));
        let other = eventless_clip_fixture();
        assert!(assets.selected_state_clip(&other.resolver, &content.admission,
            "entity", source, index("boot")).err().unwrap().to_string().contains("another content admission"));
        let foreign = crate::particle_authored::test_support::Fixture::new(
            crate::particle_authored::test_support::REGISTRATION, "particle={name=different}");
        assert!(assets.selected_state_clip(&content.resolver, &foreign.admission,
            "entity", source, index("boot")).err().unwrap().to_string().contains("another content admission"));
        let mut wrong_source = source.clone();
        wrong_source.payload_bytes += 1;
        assert!(assets.selected_state_clip(&content.resolver, &content.admission,
            "entity", &wrong_source, index("boot")).is_err());
    }

    #[test]
    fn eventless_selected_clip_clock_survives_default_requested_and_next_state_publication() {
        let content = eventless_clip_fixture();
        let assets = Arc::new(EntityAssets::load(&content.resolver).unwrap());
        let source = content.admission.require_used_content("gfx/entity.asset").unwrap();
        let (_, mut placed, mut viewer) = fixture(1);
        let mut lifecycle = ViewerLightLifecycle::default();
        for (requested, time, delta, expected_state, expected_playhead) in [
            (None, 0.25f32, 0.25f32, "boot", 0.5),
            (Some("steady"), 0.5, 0.25, "steady", 0.5),
            (Some("run"), 0.75, 0.25, "run", 0.5),
            (Some("run"), 1.5, 0.75, "done", 0.0),
            (Some("run"), 1.75, 0.25, "done", 0.5),
        ] {
            placed[0].state = requested.map(str::to_owned);
            viewer.animation_time_f32_bits = time.to_bits();
            viewer.frame_delta_seconds_f32_bits = delta.to_bits();
            let rows = lifecycle.derive_light_instance_rows(&content.resolver, &placed, &viewer, None).unwrap();
            let published = rows.root_animation(&content.resolver, &content.admission, &placed[0].id).unwrap();
            let state = &lifecycle.entities[&placed[0].id];
            assert_eq!(state.active_state.as_deref(), Some(expected_state));
            assert!(state.events.is_empty());
            assert_eq!(state.length, 2.0);
            assert_eq!(published.authored_speed, 2.0);
            assert_eq!(published.playhead_seconds, expected_playhead, "{expected_state} at {time}");
            let selected = assets.selected_state_clip(&content.resolver, &content.admission,
                "entity", source, published.state_ordinal.unwrap()).unwrap().unwrap();
            assert_eq!(xmap13_clip_length_seconds(&selected.animation) as f64, state.length);
            assert!(state.clip.is_some());
            if expected_state != "run" { assert!(state.model.is_none()); }
            assert!(lifecycle.instances.is_empty());
        }
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
            active_state: Some("idle".into()),
            state_index: Some(0),
            include_particle_only_events: false,
            next_state: None,
            authored_speed: 1.0,
            scale: 2.0,
            token: "entity".into(),
            requested_state: None,
            requested_offset_range: None,
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
            assets: Some(Arc::new(EntityAssets {
                content_lease: None,
                entity_sources: BTreeMap::new(),
                bindings: BTreeMap::new(),
                binding_declarations: Vec::new(),
                models: Mutex::new(BTreeMap::new()),
                animations: Mutex::new(BTreeMap::new()),
                poses: BTreeMap::new(),
                static_attachments: fixture_attachment_catalog(),
                meshes: BTreeMap::new(),
                mesh_sources: BTreeMap::new(),
                mesh_declarations: Vec::new(),
                mesh_clips: BTreeMap::new(),
                clips: BTreeMap::new(),
                templates,
            })),
            entities: BTreeMap::from([("placed".into(), animation(true, false))]),
            root_registration: vec!["placed".into()],
            root_changes: vec![crate::particle_runtime::EntityRootChange::Register { id: "placed".into(), scene_mask: 7, update_mask: 6 }],
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
            unit_models_enabled: true,
            sky_enabled: true,            viewport_width: 1,
            viewport_height: 1,
            camera: Some(crate::particle_runtime::scene_camera_fixture(100_000)),
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
