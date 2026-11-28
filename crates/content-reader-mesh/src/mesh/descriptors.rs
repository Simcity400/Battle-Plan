mod frozen_entities;

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_layers::{ResolvedLayers, script_files::ScriptPath};
use content_reader_parser::{crt, parser};

use super::{Material, SubMesh, Transform, invalid, parse};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshSetting {
    pub name: String,
    pub index: i32,
    pub diffuse: Option<String>,
    pub normal: Option<String>,
    pub specular: Option<String>,
    pub shader: Option<String>,
    pub shader_file: Option<String>,
}

impl Default for MeshSetting {
    fn default() -> Self {
        Self {
            name: String::new(),
            index: -1,
            diffuse: None,
            normal: None,
            specular: None,
            shader: None,
            shader_file: None,
        }
    }
}

impl MeshSetting {
    pub fn applies(&self, shape: &str, child_index: usize) -> bool {
        self.name.is_empty()
            || (self.name == shape
                && (self.index == -1 || usize::try_from(self.index).ok() == Some(child_index)))
    }
}

pub const DEFAULT_CULL_DISTANCE_SQUARED: f32 = 430.0 * 430.0;

#[derive(Clone, Debug, PartialEq)]
pub struct MeshDefinition {
    pub name: String,
    pub file: String,
    pub scale: f32,
    pub settings: Vec<MeshSetting>,
    pub animations: Vec<(String, String)>,
    pub cull_distance_squared: f32,
}

impl Default for MeshDefinition {
    fn default() -> Self {
        Self {
            name: String::new(),
            file: String::new(),
            scale: 0.0,
            settings: Vec::new(),
            animations: Vec::new(),
            cull_distance_squared: DEFAULT_CULL_DISTANCE_SQUARED,
        }
    }
}

impl MeshDefinition {
    pub fn material(&self, shape: &str, sub: &SubMesh) -> Material {
        let mut material = sub.material.clone().unwrap_or_default();
        if let Some(setting) = self
            .settings
            .iter()
            .find(|s| s.applies(shape, sub.child_index))
        {
            for (target, source) in [
                (&mut material.diffuse, &setting.diffuse),
                (&mut material.normal, &setting.normal),
                (&mut material.specular, &setting.specular),
            ] {
                if let Some(value) = source.as_ref().filter(|value| !value.is_empty()) {
                    target.clone_from(value);
                }
            }
            for (target, source) in [
                (&mut material.shader, &setting.shader),
                (&mut material.shader_file, &setting.shader_file),
            ] {
                if let Some(value) = source.as_ref().filter(|value| !value.is_empty()) {
                    *target = Some(value.clone());
                }
            }
        }
        material
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attachment {
    pub name: String,
    pub joint: String,
    pub entity: String,
}

impl Attachment {
    pub fn handle(&self) -> &str {
        if self.name.is_empty() {
            &self.joint
        } else {
            &self.name
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EntityDefinition {
    pub name: String,
    pub mesh: Option<String>,
    pub unbound_mesh: Option<String>,
    pub clone: Option<String>,
    pub scale: Option<f32>,
    pub cull_radius: Option<f32>,
    pub attachments: Vec<Attachment>,
    pub default_state: Option<String>,
    pub states: Vec<EntityState>,
    pub get_state_from_parent: Option<bool>,
    pub locators: Vec<(String, Locator)>,
    pub game_data: Option<Result<EntityGameData, String>>,
    pub sources: Vec<ScriptPath>,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EntityGameData {
    pub texture_anim_speed: f32,
    pub atlas_size: [i32; 2],
}

impl Default for EntityGameData {
    fn default() -> Self {
        Self {
            texture_anim_speed: 0.,
            atlas_size: [1, 1],
        }
    }
}

fn entity_game_data(items: &[parser::Item]) -> Option<Result<EntityGameData, String>> {
    let fields: Vec<_> = items
        .iter()
        .filter(|i| i.key.as_deref() == Some(b"game_data"))
        .filter_map(|i| i.value.items())
        .flatten()
        .collect();
    if fields.is_empty() {
        return None;
    }
    let mut data = EntityGameData::default();
    for field in fields {
        match field.key.as_deref() {
            Some(b"texture_anim_speed") => {
                let Some(speed) = field
                    .value
                    .scalar_str()
                    .and_then(|v| crt::scan_f32(v.as_bytes()))
                else {
                    return Some(Err(
                        "entity texture_anim_speed is not a literal float".into()
                    ));
                };
                data.texture_anim_speed = speed;
            }
            Some(b"atlas_size") => {
                let Some(values) = field.value.items() else {
                    return Some(Err("entity atlas_size is not an integer vector".into()));
                };
                if values.len() > 2 {
                    return Some(Err(
                        "entity atlas_size leaves its two native integer fields".into(),
                    ));
                }
                for (index, value) in values.iter().enumerate() {
                    if value
                        .key
                        .as_deref()
                        .is_some_and(|key| key != [b"x".as_slice(), b"y".as_slice()][index])
                    {
                        return Some(Err(
                            "entity atlas_size component labels are out of native order".into(),
                        ));
                    }
                    let Some(value) = value
                        .value
                        .scalar_str()
                        .and_then(|v| crt::scan_i32(v.as_bytes()).stored())
                    else {
                        return Some(Err(
                            "entity atlas_size component is not a literal integer".into()
                        ));
                    };
                    data.atlas_size[index] = value;
                }
            }
            _ => {}
        }
    }
    Some(Ok(data))
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Locator {
    pub transform: Transform,
    pub parent_joint: Option<String>,
    pub origin_mesh: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EntityState {
    pub name: String,
    pub animation: String,
    pub looping: bool,
    pub next_state: Option<String>,
    pub chance: f32,
    pub chance_weight: Option<i32>,
    pub blend: f32,
    pub speed: f32,
    pub start: f32,
    pub random_start: f32,
    pub propagate: Vec<(String, String)>,
    pub events: Vec<EntityEvent>,
    pub naval_hitmiss_effects: Vec<NavalHitMissEffect>,
    pub naval_hitmiss_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NavalHitMissEffect {
    pub time: f32,
    pub kind: u32,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EntityEvent {
    pub id: Option<String>,
    pub time: f32,
    pub node: String,
    pub light: Option<String>,
    pub particle: Option<String>,
    pub sound: Option<String>,
    pub keep_particle: bool,
    pub trigger_once: bool,
}

fn state_naval_hitmiss_effects(
    items: &[parser::Item],
) -> (Vec<NavalHitMissEffect>, Option<String>) {
    let mut effects = Vec::new();
    let mut error = None;
    for data in items.iter().filter(|item| item.keyword_is("game_data")) {
        let parser::Value::Block(fields) = &data.value else {
            error.get_or_insert_with(|| "state game_data is not a native block".into());
            continue;
        };
        for field in fields {
            if !field.keyword_is("hitmiss_effect") {
                if fully_consumed_literal(field) {
                    continue;
                }
                error.get_or_insert_with(|| {
                    format!(
                        "state game_data has unexpected field {}",
                        String::from_utf8_lossy(field.key.as_deref().unwrap_or_default())
                    )
                });
                continue;
            }
            let parser::Value::Block(record) = &field.value else {
                error.get_or_insert_with(|| "state hitmiss_effect is not a native block".into());
                continue;
            };
            let mut effect = NavalHitMissEffect::default();
            for member in record {
                let time = member.keyword_is("time");
                let kind = member.keyword_is("type");
                if !time && !kind {
                    if !fully_consumed_literal(member) {
                        error.get_or_insert_with(|| {
                            "state hitmiss_effect has an unsettled unknown member".into()
                        });
                    }
                    continue;
                }
                let Some(value) = member.value.scalar() else {
                    error.get_or_insert_with(|| {
                        "state hitmiss_effect time/type has an unconsumed block".into()
                    });
                    continue;
                };
                if !member.quoted() && value.starts_with(b"@") {
                    error.get_or_insert_with(|| {
                        "state hitmiss_effect time/type needs an unresolved script variable".into()
                    });
                    continue;
                }
                if time {
                    if let Some(value) = crt::scan_f32(value) {
                        effect.time = value;
                    }
                } else {
                    let value = parser::read_string(value);
                    if value.eq_ignore_ascii_case(b"small") {
                        effect.kind = 0;
                    } else if value.eq_ignore_ascii_case(b"big") {
                        effect.kind = 1;
                    }
                }
            }
            effects.push(effect);
        }
    }
    (effects, error)
}

fn fully_consumed_literal(item: &parser::Item) -> bool {
    item.key
        .as_deref()
        .is_some_and(|key| item.key_quoted || !key.starts_with(b"@"))
        && literal_value(item).is_some()
}

fn literal_value(item: &parser::Item) -> Option<&[u8]> {
    let value = item.value.scalar()?;
    (item.quoted() || !value.starts_with(b"@")).then_some(value)
}

fn unsettled_state_data(fields: &[parser::Item]) -> Option<&'static str> {
    for field in fields {
        if field.keyword_is("name") || field.keyword_is("animation") {
            if literal_value(field).is_none() {
                return Some("state name/animation input has unsettled native consumption");
            }
        } else if field.keyword_is("game_data") {
            let parser::Value::Block(data) = &field.value else {
                return Some("state game_data carrier is not a native block");
            };
            for member in data {
                if !member.keyword_is("hitmiss_effect") {
                    if !fully_consumed_literal(member) {
                        return Some("state game_data member has unsettled native skip");
                    }
                    continue;
                }
                let parser::Value::Block(record) = &member.value else {
                    return Some("hitmiss_effect carrier is not a native block");
                };
                for value in record {
                    if value.keyword_is("time") || value.keyword_is("type") {
                        if literal_value(value).is_none() {
                            return Some(
                                "hitmiss_effect time/type input has unsettled native consumption",
                            );
                        }
                    } else if !fully_consumed_literal(value) {
                        return Some("hitmiss_effect member has unsettled native consumption");
                    }
                }
            }
        }
    }
    None
}

fn state_text(items: &[parser::Item], key: &str, error: &mut Option<String>) -> String {
    let mut stored: &[u8] = b"";
    for field in items.iter().filter(|item| item.keyword_is(key)) {
        let Some(value) = field.value.scalar() else {
            error.get_or_insert_with(|| format!("state {key} has an unconsumed block"));
            stored = b"";
            continue;
        };
        if !field.quoted() && value.starts_with(b"@") {
            error.get_or_insert_with(|| format!("state {key} needs an unresolved script variable"));
            stored = b"";
            continue;
        }
        let value = parser::c_string(value);
        stored = &value[..value.len().min(255)];
    }
    match std::str::from_utf8(stored) {
        Ok(value) => value.to_owned(),
        Err(_) => {
            error.get_or_insert_with(|| format!("state {key} native 255-byte text is not UTF-8"));
            String::new()
        }
    }
}

#[cfg(test)]
#[path = "descriptors/naval_hitmiss_tests.rs"]
mod naval_hitmiss_tests;
#[cfg(test)]
#[path = "descriptors/naval_locator_tests.rs"]
mod naval_locator_tests;

fn number(items: &[parser::Item], key: &[u8], fallback: f32) -> f32 {
    text(items, key)
        .and_then(|v| crt::scan_f32(v.as_bytes()))
        .unwrap_or(fallback)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnimationDefinition {
    pub name: String,
    pub file: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMesh<'a> {
    pub definition: &'a MeshDefinition,
    pub scale: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Descriptors {
    pub meshes: BTreeMap<String, MeshDefinition>,
    pub entities: BTreeMap<String, EntityDefinition>,
    pub animations: BTreeMap<String, AnimationDefinition>,
}

#[derive(Clone, Copy)]
enum EntityRows<'a> {
    Raw(&'a BTreeMap<String, EntityDefinition>),
    Packed(&'a frozen_entities::Job<'a>),
    Prepared(&'a frozen_entities::Job<'a>),
}

impl<'a> EntityRows<'a> {
    fn get(self, name: &str) -> Option<&'a EntityDefinition> {
        match self {
            Self::Raw(rows) => rows.get(name),
            Self::Packed(job) => job.get(name),
            Self::Prepared(job) => job.prepared(name),
        }
    }

    fn contains(self, name: &str) -> bool {
        match self {
            Self::Raw(rows) => rows.contains_key(name),
            Self::Packed(job) | Self::Prepared(job) => job.contains(name),
        }
    }

    fn len(self) -> usize {
        match self {
            Self::Raw(rows) => rows.len(),
            Self::Packed(job) | Self::Prepared(job) => job.len(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct DescriptorView<'a> {
    pub meshes: &'a BTreeMap<String, MeshDefinition>,
    pub animations: &'a BTreeMap<String, AnimationDefinition>,
    entities: EntityRows<'a>,
    aliases: Option<&'a BTreeMap<String, EntityDefinition>>,
}

impl<'a> From<&'a Descriptors> for DescriptorView<'a> {
    fn from(descriptors: &'a Descriptors) -> Self {
        Self {
            meshes: &descriptors.meshes,
            animations: &descriptors.animations,
            entities: EntityRows::Raw(&descriptors.entities),
            aliases: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InheritedEntity<'a> {
    pub mesh: Option<&'a str>,
    pub unbound_mesh: Option<&'a str>,
    pub scale: Option<f32>,
    pub cull_radius: Option<f32>,
    pub default_state: Option<&'a str>,
    pub attachments: Vec<&'a Attachment>,
    pub states: BTreeMap<&'a str, Vec<&'a EntityState>>,
    pub get_state_from_parent: Option<bool>,
    pub locators: BTreeMap<&'a str, &'a Locator>,
    pub game_data: Option<&'a Result<EntityGameData, String>>,
    pub sources: Vec<&'a ScriptPath>,
}

impl Descriptors {
    pub fn inherit(&self, name: &str) -> Option<InheritedEntity<'_>> {
        DescriptorView::from(self).inherit(name)
    }

    pub fn resolve_entity(&self, name: &str) -> Option<ResolvedEntity<'_>> {
        DescriptorView::from(self).resolve_entity(name)
    }
}

impl<'a> DescriptorView<'a> {
    pub fn prepared(self) -> Self {
        match self.entities {
            EntityRows::Packed(job) => Self { entities: EntityRows::Prepared(job), ..self },
            _ => self,
        }
    }

    pub fn with_aliases(self, aliases: &'a BTreeMap<String, EntityDefinition>) -> Self {
        Self { aliases: Some(aliases), ..self }
    }

    pub fn entity(self, name: &str) -> Option<&'a EntityDefinition> {
        self.aliases.and_then(|aliases| aliases.get(name)).or_else(|| self.entities.get(name))
    }

    pub fn has_entity(self, name: &str) -> bool {
        self.aliases.is_some_and(|aliases| aliases.contains_key(name)) || self.entities.contains(name)
    }

    pub fn entity_count(self) -> usize {
        self.entities.len() + self.aliases.map_or(0, BTreeMap::len)
    }

    pub fn inherit(self, name: &str) -> Option<InheritedEntity<'a>> {
        let mut out = InheritedEntity::default();
        let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
        let mut current = self.entity(name)?;
        let mut chain = Vec::new();
        for _ in 0..=self.entity_count() {
            chain.push(current);
            out.sources.extend(&current.sources);
            out.mesh = out.mesh.or(current.mesh.as_deref());
            out.unbound_mesh = out.unbound_mesh.or(current.unbound_mesh.as_deref());
            out.scale = out.scale.or(current.scale);
            if current.clone.is_none() {
                out.cull_radius = current.cull_radius;
            }
            out.game_data = out.game_data.or(current.game_data.as_ref());
            out.default_state = out.default_state.or(current.default_state.as_deref());
            out.get_state_from_parent = out.get_state_from_parent.or(current.get_state_from_parent);
            for (name, locator) in &current.locators {
                out.locators.entry(name).or_insert(locator);
            }
            let mut local = BTreeMap::<&str, Vec<&EntityState>>::new();
            for state in &current.states {
                local.entry(&state.name).or_default().push(state);
            }
            for (name, variants) in local {
                out.states.entry(name).or_insert(variants);
            }
            let Some(parent) = current.clone.as_deref() else {
                break;
            };
            if seen.insert(parent, ()).is_some() {
                break;
            }
            let Some(parent) = self.entity(parent) else {
                break;
            };
            current = parent;
        }
        for link in chain.into_iter().rev() {
            for attachment in &link.attachments {
                match out
                    .attachments
                    .iter()
                    .position(|held| same_slot(held, attachment))
                {
                    Some(at) => out.attachments[at] = attachment,
                    None => out.attachments.push(attachment),
                }
            }
        }
        Some(out)
    }

    pub fn resolve_entity(self, name: &str) -> Option<ResolvedEntity<'a>> {
        let Some(inherited) = self.inherit(name) else {
            let definition = self.meshes.get(name)?;
            return Some(ResolvedEntity {
                definition,
                scale: definition.scale,
                attachments: Vec::new(),
                animation: None,
            });
        };
        let definition = self.meshes.get(inherited.mesh?)?;
        let state = inherited.default_state.unwrap_or(DEFAULT_STATE);
        let animation = inherited
            .states
            .get(state)
            .and_then(|variants| variants.first())
            .and_then(|state| {
                let id = &state.animation;
                definition
                    .animations
                    .iter()
                    .find(|(declared, _)| declared == id)
            })
            .and_then(|(_, name)| self.animations.get(name.as_str()));
        Some(ResolvedEntity {
            definition,
            scale: inherited.scale.unwrap_or(1.0) * definition.scale,
            attachments: inherited.attachments,
            animation,
        })
    }
}

pub const DEFAULT_STATE: &str = "idle";

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedEntity<'a> {
    pub definition: &'a MeshDefinition,
    pub scale: f32,
    pub attachments: Vec<&'a Attachment>,
    pub animation: Option<&'a AnimationDefinition>,
}

impl Descriptors {
    pub fn resolve(&self, name: &str) -> Option<ResolvedMesh<'_>> {
        DescriptorView::from(self).resolve(name)
    }
}

impl<'a> DescriptorView<'a> {
    pub fn resolve(self, name: &str) -> Option<ResolvedMesh<'a>> {
        self.resolve_entity(name).map(|entity| ResolvedMesh {
            definition: entity.definition,
            scale: entity.scale,
        })
    }
}

pub fn parse_descriptors(bytes: &[u8], path: &str) -> io::Result<Descriptors> {
    let document = parser::parse(bytes)?;
    settled_state_carriers(&document, path)?;
    let (object_items, _) = read_object_items(bytes, path)?;
    let (entity_items, _) = read_entity_items(bytes, path)?;
    let mut out = Descriptors::default();
    let _ = collect(&document, &object_items, &entity_items, path, &mut out);
    Ok(out)
}

fn settled_state_carriers(document: &parser::Document, path: &str) -> io::Result<()> {
    for item in document
        .items
        .iter()
        .filter(|item| item.keyword_is("entity"))
        .filter_map(|item| item.value.items())
        .flatten()
        .filter(|item| item.keyword_is("state"))
    {
        let parser::Value::Block(fields) = &item.value else {
            return Err(invalid(format!(
                "{path}: scalar/tagged entity state carrier leaves the native file reader alignment unsupported"
            )));
        };
        if let Some(reason) = unsettled_state_data(fields) {
            return Err(invalid(format!(
                "{path}: {reason}; descriptor file refused"
            )));
        }
    }
    for entity in document
        .items
        .iter()
        .filter(|item| item.keyword_is("entity"))
    {
        let Some(fields) = entity.value.items() else {
            continue;
        };
        for field in fields
            .iter()
            .filter(|field| field.keyword_is("cull_radius"))
        {
            if literal_value(field).is_none() {
                let name = last_text(fields, "name").unwrap_or_default();
                return Err(invalid(format!(
                    "{path}: entity {name} cull_radius variable/block/tagged carrier has unsupported native consumption; descriptor file refused"
                )));
            }
        }
    }
    Ok(())
}

fn check_native_read(
    read: content_reader_script::native::Read,
    path: &str,
) -> io::Result<(Vec<parser::Item>, Vec<String>)> {
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("{path}: {cause}; descriptor file refused")));
    }
    let misreads = read
        .misreads
        .into_iter()
        .map(|misread| format!("{path}: {misread}"))
        .collect();
    Ok((read.items, misreads))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ObjectNode {
    Top,
    ObjectTypes,
    Mesh,
    MeshSettings,
    MeshAnimation,
    MeshVariant,
    NamedObject,
}

fn object_dispatch(
    node: ObjectNode,
    field: &content_reader_script::native::Field<'_>,
) -> content_reader_script::native::Shape<ObjectNode> {
    use content_reader_script::native::Shape;
    match node {
        ObjectNode::Top => match field.keyword().as_deref() {
            Some(b"objecttypes") => Shape::Object(ObjectNode::ObjectTypes),
            _ => Shape::Skip,
        },
        ObjectNode::ObjectTypes => match field.keyword().as_deref() {
            Some(b"pdxmesh") => Shape::Object(ObjectNode::Mesh),
            Some(b"arrowtype" | b"animatedmaptext" | b"pdxparticle") => {
                Shape::Object(ObjectNode::NamedObject)
            }
            _ => Shape::Skip,
        },
        ObjectNode::Mesh => match field.keyword().as_deref() {
            Some(b"meshsettings") => Shape::Object(ObjectNode::MeshSettings),
            Some(b"animation") => Shape::Object(ObjectNode::MeshAnimation),
            Some(b"variant") => Shape::Object(ObjectNode::MeshVariant),
            Some(b"name" | b"file" | b"scale" | b"cull_distance") => Shape::One,
            _ => Shape::Skip,
        },
        ObjectNode::MeshSettings => match field.keyword().as_deref() {
            Some(
                b"name"
                | b"index"
                | b"texture_diffuse"
                | b"texture_normal"
                | b"texture_specular"
                | b"shader"
                | b"shader_file",
            ) => Shape::One,
            _ => Shape::Skip,
        },
        ObjectNode::MeshAnimation => match field.keyword().as_deref() {
            Some(b"id" | b"type") => Shape::One,
            _ => Shape::Skip,
        },
        ObjectNode::MeshVariant => match field.keyword().as_deref() {
            Some(b"weight" | b"pdxmesh") => Shape::One,
            _ => Shape::Skip,
        },
        ObjectNode::NamedObject => match field.keyword().as_deref() {
            Some(b"name") => Shape::One,
            _ => Shape::Skip,
        },
    }
}

fn read_object_items(bytes: &[u8], path: &str) -> io::Result<(Vec<parser::Item>, Vec<String>)> {
    let mut dispatch = object_dispatch;
    let read = content_reader_script::native::read(
        bytes,
        content_reader_script::native::Mode::SkipMark,
        content_reader_script::native::Top::File(ObjectNode::Top),
        None,
        &mut dispatch,
    );
    check_native_read(read, path)
}

fn recover_object_names(bytes: &[u8]) -> BTreeSet<String> {
    let explored = content_reader_script::native::recover_declared_names(
        bytes,
        content_reader_script::native::Mode::SkipMark,
        ObjectNode::Top,
        |node| match node {
            ObjectNode::Mesh => Some(content_reader_script::native::NamePolicy::FirstExact),
            ObjectNode::NamedObject => Some(content_reader_script::native::NamePolicy::LastKeyword),
            _ => None,
        },
        |node, field| content_reader_script::native::RecoveryShape::Exact(object_dispatch(node, field)),
    );
    recovered_utf8_names(explored.names)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum EntityNode {
    Top,
    Entity,
    State,
    StateEvent,
    StateEventSound,
    StatePropagate,
    StateGameData,
    HitMissEffect,
    Attach,
    Locator,
    EntityGameData,
}

fn entity_dispatch(
    node: EntityNode,
    field: &content_reader_script::native::Field<'_>,
) -> content_reader_script::native::Shape<EntityNode> {
    use content_reader_script::native::Shape;
    match node {
        EntityNode::Top => match field.keyword().as_deref() {
            Some(b"entity") => Shape::Object(EntityNode::Entity),
            _ => Shape::Skip,
        },
        EntityNode::Entity => match field.keyword().as_deref() {
            Some(b"state") => Shape::Object(EntityNode::State),
            Some(b"attach") => Shape::Object(EntityNode::Attach),
            Some(b"locator") => Shape::Object(EntityNode::Locator),
            Some(b"game_data") => Shape::Object(EntityNode::EntityGameData),
            Some(
                b"name"
                | b"pdxmesh"
                | b"clone"
                | b"scale"
                | b"cull_radius"
                | b"default_state"
                | b"get_state_from_parent"
                | b"version",
            ) => Shape::One,
            _ => Shape::Skip,
        },
        EntityNode::State => match field.keyword().as_deref() {
            Some(b"event") => Shape::Object(EntityNode::StateEvent),
            Some(b"propagate_state") => Shape::Object(EntityNode::StatePropagate),
            Some(b"game_data") => Shape::Object(EntityNode::StateGameData),
            Some(
                b"name"
                | b"animation"
                | b"looping"
                | b"next_state"
                | b"chance"
                | b"animation_blend_time"
                | b"animation_speed"
                | b"animation_start_time"
                | b"animation_random_start_time",
            ) => Shape::One,
            _ => Shape::Skip,
        },
        EntityNode::StateEvent => match field.keyword().as_deref() {
            Some(b"sound") => Shape::Object(EntityNode::StateEventSound),
            Some(
                b"id"
                | b"time"
                | b"node"
                | b"light"
                | b"particle"
                | b"keep_particle"
                | b"trigger_once",
            ) => Shape::One,
            _ => Shape::Skip,
        },
        EntityNode::StateEventSound => match field.keyword().as_deref() {
            Some(b"soundeffect") => Shape::One,
            _ => Shape::Skip,
        },
        EntityNode::StatePropagate | EntityNode::Attach => {
            if field.opens() {
                Shape::Skip
            } else {
                Shape::One
            }
        }
        EntityNode::StateGameData => match field.keyword().as_deref() {
            Some(b"hitmiss_effect") => Shape::Object(EntityNode::HitMissEffect),
            _ => Shape::Skip,
        },
        EntityNode::HitMissEffect => match field.keyword().as_deref() {
            Some(b"time" | b"type") => Shape::One,
            _ => Shape::Skip,
        },
        EntityNode::Locator => match field.keyword().as_deref() {
            Some(b"position" | b"rotation") => Shape::ByKey,
            Some(b"name" | b"parent_joint" | b"scale") => Shape::One,
            _ => Shape::Skip,
        },
        EntityNode::EntityGameData => match field.keyword().as_deref() {
            Some(b"atlas_size") => Shape::ByKey,
            Some(b"texture_anim_speed") => Shape::One,
            _ => Shape::Skip,
        },
    }
}

fn read_entity_items(bytes: &[u8], path: &str) -> io::Result<(Vec<parser::Item>, Vec<String>)> {
    let mut dispatch = entity_dispatch;
    let read = content_reader_script::native::read(
        bytes,
        content_reader_script::native::Mode::SkipMark,
        content_reader_script::native::Top::File(EntityNode::Top),
        None,
        &mut dispatch,
    );
    check_native_read(read, path)
}

fn entity_recovery_dispatch(
    node: EntityNode,
    field: &content_reader_script::native::Field<'_>,
) -> content_reader_script::native::RecoveryShape<EntityNode> {
    use content_reader_script::native::{RecoveryShape, Shape};
    match node {
        EntityNode::Entity => match field.keyword().as_deref() {
            Some(b"state") if !field.opens() => RecoveryShape::ObjectOrSkip(EntityNode::State),
            Some(b"cull_radius") => RecoveryShape::OneOrSkip,
            _ => RecoveryShape::Exact(entity_dispatch(node, field)),
        },
        EntityNode::State => match field.keyword().as_deref() {
            Some(b"name" | b"animation") => RecoveryShape::OneOrSkip,
            Some(b"game_data") => RecoveryShape::ObjectOrSkip(EntityNode::StateGameData),
            _ => RecoveryShape::Exact(entity_dispatch(node, field)),
        },
        EntityNode::StateGameData => match field.keyword().as_deref() {
            Some(b"hitmiss_effect") => RecoveryShape::ObjectOrSkip(EntityNode::HitMissEffect),
            _ => RecoveryShape::Exact(entity_dispatch(node, field)),
        },
        EntityNode::HitMissEffect => match field.keyword().as_deref() {
            Some(b"time" | b"type") => RecoveryShape::OneOrSkip,
            _ => RecoveryShape::Exact(Shape::Skip),
        },
        _ => RecoveryShape::Exact(entity_dispatch(node, field)),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum NamedBlockNode {
    Top,
    Body,
}

fn recover_named_block(
    bytes: &[u8],
    block_key: &str,
    with_file: bool,
    policy: content_reader_script::native::NamePolicy,
) -> BTreeSet<String> {
    let explored = content_reader_script::native::recover_declared_names(
        bytes,
        content_reader_script::native::Mode::SkipMark,
        NamedBlockNode::Top,
        |node| matches!(node, NamedBlockNode::Body).then_some(policy),
        |node, field| match node {
            NamedBlockNode::Top => {
                if field.keyword_is(block_key) {
                    content_reader_script::native::RecoveryShape::Exact(content_reader_script::native::Shape::Object(
                        NamedBlockNode::Body,
                    ))
                } else {
                    content_reader_script::native::RecoveryShape::Exact(content_reader_script::native::Shape::Skip)
                }
            }
            NamedBlockNode::Body => {
                if field.keyword_is("name") || (with_file && field.keyword_is("file")) {
                    content_reader_script::native::RecoveryShape::Exact(content_reader_script::native::Shape::One)
                } else {
                    content_reader_script::native::RecoveryShape::Exact(content_reader_script::native::untraced(field))
                }
            }
        },
    );
    recovered_utf8_names(explored.names)
}

fn recover_asset_names(bytes: &[u8]) -> BTreeSet<String> {
    let entities = content_reader_script::native::recover_declared_names(
        bytes,
        content_reader_script::native::Mode::SkipMark,
        EntityNode::Top,
        |node| {
            matches!(node, EntityNode::Entity).then_some(content_reader_script::native::NamePolicy::LastKeyword)
        },
        entity_recovery_dispatch,
    );
    let mut names = recovered_utf8_names(entities.names);
    names.extend(recover_named_block(
        bytes,
        "animation",
        true,
        content_reader_script::native::NamePolicy::FirstExact,
    ));
    names.extend(recover_named_block(
        bytes,
        "light",
        false,
        content_reader_script::native::NamePolicy::LastKeyword,
    ));
    names
}

fn recovered_utf8_names(raw_names: BTreeSet<Vec<u8>>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for raw in raw_names {
        let Ok(text) = std::str::from_utf8(&raw) else {
            continue;
        };
        let name = unquote(text);
        if !name.is_empty() {
            names.insert(name);
        }
    }
    names
}

fn collect(
    document: &parser::Document,
    object_items: &[parser::Item],
    entity_items: &[parser::Item],
    path: &str,
    out: &mut Descriptors,
) -> Vec<String> {
    let directory = path.rsplit_once('/').map_or("", |(head, _)| head);
    let mut replaced = Vec::new();
    for item in object_items {
        if item.keyword_is("objectTypes") {
            for block in item.value.items().unwrap_or(&[]) {
                if block.keyword_is("pdxmesh")
                    && let Some(definition) = mesh_definition(&block.value)
                    && let Some(old) = out.meshes.insert(definition.name.clone(), definition)
                {
                    replaced.push(old.name);
                }
            }
        }
    }
    for item in entity_items {
        if item.keyword_is("entity")
            && let Some(definition) = entity_definition(&item.value)
            && let Some(old) = out.entities.insert(definition.name.clone(), definition)
        {
            replaced.push(old.name);
        }
    }
    for item in &document.items {
        if item.key.as_deref() == Some(b"animation")
            && let Some(definition) = animation_definition(&item.value, directory)
            && let Some(old) = out.animations.insert(definition.name.clone(), definition)
        {
            replaced.push(old.name);
        }
    }
    replaced
}

pub fn balanced_blocks(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let (mut depth, mut quoted, mut comment, mut escaped) = (0usize, false, false, false);
    for &b in bytes {
        if comment {
            out.push(b);
            if b == b'\n' {
                comment = false;
            }
            continue;
        }
        if quoted {
            out.push(b);
            if b == b'"' && !escaped {
                quoted = false;
            }
            escaped = b == b'\\' && !escaped;
            continue;
        }
        match b {
            b'#' => comment = true,
            b'"' => quoted = true,
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
            }
            _ => {}
        }
        out.push(b);
    }
    out.push(b'\n');
    out.extend(std::iter::repeat_n(b'}', depth));
    out
}

#[derive(Clone, Debug, Default)]
pub struct Roster {
    pub descriptors: RosterDescriptors,
    pub lights: BTreeMap<String, Vec<parser::Item>>,
    pub logged: Vec<String>,
    pub unreadable: Vec<String>,
    pub unreadable_names: BTreeMap<String, String>,
    pub clones: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct RosterDescriptors {
    pub meshes: BTreeMap<String, MeshDefinition>,
    pub animations: BTreeMap<String, AnimationDefinition>,
    entities: frozen_entities::Entities,
    attachments: BTreeMap<String, Box<[String]>>,
}

impl RosterDescriptors {
    fn freeze(&mut self, definitions: BTreeMap<String, EntityDefinition>) {
        let view = DescriptorView {
            meshes: &self.meshes,
            animations: &self.animations,
            entities: EntityRows::Raw(&definitions),
            aliases: None,
        };
        self.attachments = definitions.iter().filter_map(|(name, entity)| {
            if entity.clone.is_none() && entity.attachments.is_empty() {
                return None;
            }
            let links: Box<[String]> = view.inherit(name)?.attachments.into_iter()
                .map(|attachment| attachment.entity.clone())
                .collect();
            (!links.is_empty()).then(|| (name.clone(), links))
        }).collect();
        self.entities = frozen_entities::Entities::freeze(definitions.into_iter());
    }
}

impl From<Descriptors> for RosterDescriptors {
    fn from(descriptors: Descriptors) -> Self {
        let Descriptors { meshes, animations, entities } = descriptors;
        let mut registered = Self { meshes, animations, ..Self::default() };
        registered.freeze(entities);
        registered
    }
}

pub struct DescriptorJob<'a> {
    roster: &'a Roster,
    entities: frozen_entities::Job<'a>,
}

impl DescriptorJob<'_> {
    pub fn view(&self) -> DescriptorView<'_> {
        DescriptorView {
            meshes: &self.roster.descriptors.meshes,
            animations: &self.roster.descriptors.animations,
            entities: EntityRows::Packed(&self.entities),
            aliases: None,
        }
    }

    pub fn entities(&self) -> impl Iterator<Item = &EntityDefinition> {
        self.entities.iter()
    }

    pub fn refused_declaration(&self, entity: &str) -> Option<String> {
        self.roster.refused_below(self.view(), entity, &BTreeSet::new())
    }
}

const OBJECTS: [&str; 4] = ["pdxmesh", "arrowType", "animatedmaptext", "pdxparticle"];

const ANIMATION_NAME: usize = 63;
const ENTITY_NAME: usize = 255;

struct ParsedFile {
    document: parser::Document,
    items: Vec<parser::Item>,
    overlay_fallback_names: Option<BTreeSet<String>>,
}

impl Roster {
    pub fn descriptor_job(&self) -> DescriptorJob<'_> {
        DescriptorJob { roster: self, entities: self.descriptors.entities.job() }
    }

    pub fn has_entity(&self, name: &str) -> bool {
        self.descriptors.entities.contains(name)
    }

    pub fn entity_names(&self) -> impl Iterator<Item = &str> {
        self.descriptors.entities.names()
    }

    pub fn mesh_definitions(&self) -> &BTreeMap<String, MeshDefinition> {
        &self.descriptors.meshes
    }

    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut roster = Self::default();
        let interface = layers.descriptor_paths("interface", &[".gfx"])?;
        let gfx = layers.descriptor_paths("gfx", &[".gfx", ".asset"])?;
        let mut objects = BTreeMap::new();
        for path in crate::particles::object_file_order(&interface, &gfx) {
            if let Some(parsed) = roster.parse(
                layers,
                &path,
                &OBJECTS,
                read_object_items,
                recover_object_names,
            )? {
                let registered = add_objects(&parsed.items, &mut objects);
                if let Some(overlay_names) = parsed.overlay_fallback_names {
                    roster.record_overlay_refusals(overlay_names, &registered, &[], &path.text());
                }
            }
        }
        roster.descriptors.meshes = objects
            .into_iter()
            .filter_map(|(name, mesh)| Some((name, mesh?)))
            .collect();
        let mut animations = BTreeMap::<String, Vec<AnimationDefinition>>::new();
        let mut entities = Entities::default();
        for file in content_reader_defines::defines::load_order("gfx/", ".asset", &gfx) {
            let Some(parsed) = roster.parse(
                layers,
                &file,
                &["entity", "animation", "light"],
                read_entity_items,
                recover_asset_names,
            )?
            else {
                continue;
            };
            let path = file.text();
            let directory = file.folder_text();
            let mut registered = BTreeSet::new();
            for item in &parsed.document.items {
                if item.keyword_is("animation") {
                    let Some(animation) =
                        animation_definition(&item.value, directory.unwrap_or_default())
                    else {
                        continue;
                    };
                    if directory.is_none() {
                        return Err(invalid(format!(
                            "{path}: animation {} names its file from this file's folder, whose name isn't UTF-8",
                            animation.name
                        )));
                    }
                    if animation.name.len() > ANIMATION_NAME {
                        return Err(invalid(format!(
                            "{path}: animation {} is longer than the {ANIMATION_NAME} bytes the engine keeps",
                            animation.name
                        )));
                    }
                    registered.insert(animation.name.clone());
                    animations
                        .entry(animation.name.clone())
                        .or_default()
                        .push(animation);
                } else if item.keyword_is("light")
                    && let Some(fields) = item.value.items()
                {
                    let name = last_text(fields, "name").unwrap_or_default();
                    if name.len() > ANIMATION_NAME {
                        return Err(invalid(format!(
                            "{path}: light {name} is longer than the {ANIMATION_NAME} bytes the engine keeps"
                        )));
                    }
                    if !name.is_empty() {
                        registered.insert(name.clone());
                    }
                    roster.lights.insert(name, fields.to_vec());
                }
            }
            let mut file_clones = Vec::new();
            for item in parsed.items {
                if item.keyword_is("entity") {
                    if let Some(fields) = item.value.items()
                        && let Some(name) = last_text(fields, "name").filter(|name| !name.is_empty())
                        && let Some(parent) =
                            last_text(fields, "clone").filter(|parent| !parent.is_empty())
                    {
                        file_clones.push((name, parent));
                    }
                    if let Some(name) = entities.declare(&item.value, &file, &mut roster)? {
                        registered.insert(name);
                    }
                }
            }
            if let Some(overlay_names) = parsed.overlay_fallback_names {
                roster.record_overlay_refusals(overlay_names, &registered, &file_clones, &path);
            }
        }
        for (name, declarations) in animations {
            let animation = loaded_animation(layers, declarations)?;
            roster.descriptors.animations.insert(name, animation);
        }
        roster.descriptors.freeze(
            entities.0.into_iter().map(|(name, (definition, _))| (name, definition)).collect(),
        );
        Ok(roster)
    }

    fn record_overlay_refusals(
        &mut self,
        overlay_names: BTreeSet<String>,
        registered: &BTreeSet<String>,
        file_clones: &[(String, String)],
        path: &str,
    ) {
        let mut newly_refused: BTreeSet<String> = overlay_names
            .into_iter()
            .filter(|name| !registered.contains(name))
            .collect();
        for (child, parent) in file_clones {
            if newly_refused.contains(parent) {
                newly_refused.insert(child.clone());
            }
        }
        for name in newly_refused {
            self.unreadable_names
                .entry(name)
                .or_insert_with(|| path.to_owned());
        }
    }

    fn parse(
        &mut self,
        layers: &ResolvedLayers,
        file: &ScriptPath,
        words: &[&str],
        native: fn(&[u8], &str) -> io::Result<(Vec<parser::Item>, Vec<String>)>,
        recover: fn(&[u8]) -> BTreeSet<String>,
    ) -> io::Result<Option<ParsedFile>> {
        let text = file.text();
        let path: &str = &text;
        let bytes = match file.read_uncovered(layers) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.unreadable.push(format!("{path} ({error})"));
                return Ok(None);
            }
        };
        if !words.iter().any(|word| {
            bytes
                .windows(word.len())
                .any(|window| window.eq_ignore_ascii_case(word.as_bytes()))
        }) {
            return Ok(None);
        }
        let parse_tree = |bytes: &[u8]| {
            parser::parse(bytes).or_else(|_| parser::parse(&balanced_blocks(bytes)))
        };
        let validate = |bytes: &[u8], document: &parser::Document| -> io::Result<_> {
            settled_state_carriers(document, path)?;
            native(bytes, path)
        };
        let (error, parsed) = match parse_tree(&bytes) {
            Ok(document) => match validate(&bytes, &document) {
                Ok((items, misreads)) => {
                    self.logged.extend(misreads);
                    return Ok(Some(ParsedFile {
                        document,
                        items,
                        overlay_fallback_names: None,
                    }));
                }
                Err(error) => (error, Some(document)),
            },
            Err(error) => (error, None),
        };
        if layers.supplied_by_visual_mod(file)
            && let Some(recorded) = layers.without_overlays()
            && let Ok(recorded_bytes) = file.read_uncovered(&recorded)
            && let Ok(document) = parse_tree(&recorded_bytes)
            && let Ok((items, misreads)) = validate(&recorded_bytes, &document)
        {
            layers.note_overlay_refusal(format!("descriptor {path} ({error})"));
            let overlay_names = match &parsed {
                Some(_) => recover(&bytes),
                None => {
                    let mut overlay_names = lexical_names(&bytes);
                    overlay_names.extend(recover(&bytes));
                    overlay_names
                }
            };
            self.logged.extend(misreads);
            return Ok(Some(ParsedFile {
                document,
                items,
                overlay_fallback_names: Some(overlay_names),
            }));
        }
        let names = match &parsed {
            Some(_) => recover(&bytes),
            None => {
                let mut names = lexical_names(&bytes);
                names.extend(recover(&bytes));
                names
            }
        };
        for name in names {
            self.unreadable_names
                .entry(name)
                .or_insert_with(|| path.to_owned());
        }
        self.unreadable.push(format!("{path} ({error})"));
        Ok(None)
    }

    pub fn unreadable(&self, name: &str) -> Option<String> {
        self.unreadable_names
            .get(name)
            .map(|path| format!("may be declared in {path}, which does not parse"))
    }

    pub fn unreadable_attachment(&self, root: &str) -> Option<(&str, &str)> {
        if self.unreadable_names.is_empty() {
            return None;
        }
        let mut pending = vec![root];
        let mut seen = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !seen.insert(name) {
                continue;
            }
            if let Some((name, path)) = self.unreadable_names.get_key_value(name) {
                return Some((name, path));
            }
            if let Some(links) = self.descriptors.attachments.get(name) {
                pending.extend(links.iter().map(String::as_str));
            }
        }
        None
    }

    pub fn refused_declaration(&self, entity: &str) -> Option<String> {
        self.descriptor_job().refused_declaration(entity)
    }

    fn refused_below(
        &self,
        descriptors: DescriptorView<'_>,
        entity: &str,
        ancestors: &BTreeSet<String>,
    ) -> Option<String> {
        let mut ancestors = ancestors.clone();
        if !ancestors.insert(entity.to_owned()) {
            return Some(format!("attachment cycle through {entity}"));
        }
        if let Some(flaw) = self.unreadable(entity) {
            return Some(format!("it {flaw}"));
        }
        let inherited = descriptors.inherit(entity)?;
        if let Some(state) = inherited
            .states
            .values()
            .flatten()
            .find(|state| !(state.speed.is_finite() && state.speed > 0.))
        {
            return Some(format!(
                "state {} has unsupported non-positive animation_speed {}",
                state.name, state.speed
            ));
        }
        if let Some(name) = inherited.mesh {
            if let Some(flaw) = self.unreadable(name) {
                return Some(format!("its mesh {name} {flaw}"));
            }
            if let Some(definition) = descriptors.meshes.get(name) {
                for state in inherited.states.values().flatten() {
                    if let Some((_, animation)) = definition
                        .animations
                        .iter()
                        .find(|(declared, _)| *declared == state.animation)
                        && let Some(flaw) = self.unreadable(animation)
                    {
                        return Some(format!("its animation {animation} {flaw}"));
                    }
                }
            }
        }
        for attachment in &inherited.attachments {
            if let Some(reason) = self.refused_below(descriptors, &attachment.entity, &ancestors) {
                return Some(format!(
                    "its attached entity {} is refused: {reason}",
                    attachment.entity
                ));
            }
        }
        None
    }
}

fn add_objects(
    items: &[parser::Item],
    objects: &mut BTreeMap<String, Option<MeshDefinition>>,
) -> BTreeSet<String> {
    let mut registered = BTreeSet::new();
    for item in items {
        if !item.keyword_is("objectTypes") {
            continue;
        }
        for object in item.value.items().unwrap_or(&[]) {
            if object.keyword_is(OBJECTS[0]) {
                if let Some(mesh) = mesh_definition(&object.value) {
                    registered.insert(mesh.name.clone());
                    objects.insert(mesh.name.clone(), Some(mesh));
                }
            } else if OBJECTS[1..].iter().any(|kind| object.keyword_is(kind))
                && let Some(name) = object
                    .value
                    .items()
                    .and_then(|fields| last_text(fields, "name"))
            {
                if !name.is_empty() {
                    registered.insert(name.clone());
                }
                objects.insert(name, None);
            }
        }
    }
    registered
}

fn loaded_animation(
    layers: &ResolvedLayers,
    mut declarations: Vec<AnimationDefinition>,
) -> io::Result<AnimationDefinition> {
    let last = declarations.pop().expect("a name has a declaration");
    if declarations.iter().all(|earlier| earlier.file == last.file) {
        return Ok(last);
    }
    for declaration in std::iter::once(&last).chain(declarations.iter().rev()) {
        let bytes = match layers.read_uncovered(&declaration.file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if parse(&bytes)
            .is_ok_and(|root| root.child("info").is_some() && root.child("samples").is_some())
        {
            return Ok(declaration.clone());
        }
    }
    Ok(last)
}

#[derive(Default)]
struct Entities(BTreeMap<String, (EntityDefinition, i32)>);

impl Entities {
    fn declare(
        &mut self,
        value: &parser::Value,
        path: &ScriptPath,
        roster: &mut Roster,
    ) -> io::Result<Option<String>> {
        let Some(fields) = value.items() else {
            return Ok(None);
        };
        let Some(name) = last_text(fields, "name").filter(|name| !name.is_empty()) else {
            return Ok(None);
        };
        let clone = last_text(fields, "clone").filter(|parent| !parent.is_empty());
        let mesh = last_text(fields, "pdxmesh");
        for text in std::iter::once(&name).chain(&clone).chain(&mesh) {
            if text.len() > ENTITY_NAME {
                return Err(invalid(format!(
                    "{path}: entity {name} names {text}, longer than the {ENTITY_NAME} bytes the engine keeps"
                )));
            }
        }
        let version = fields
            .iter()
            .rev()
            .find(|item| item.keyword_is("version"))
            .and_then(|item| item.value.scalar())
            .map_or(0, content_reader_parser::fixed5::decimal_i32);
        if let Some(file) = clone
            .as_ref()
            .and_then(|parent| roster.unreadable_names.get(parent))
        {
            let file = file.clone();
            roster.unreadable_names.entry(name.clone()).or_insert(file);
        }
        let parent = match &clone {
            None => None,
            Some(parent) => match self.0.get(parent) {
                Some(registered) => Some(registered.clone()),
                None => {
                    roster.logged.push(format!(
                        "{name} in {path}: the engine drops it, finding no entity {parent} to clone yet"
                    ));
                    return Ok(None);
                }
            },
        };
        let registered = self.0.get(&name);
        let stored = registered.map(|(_, stored)| *stored);
        if stored.is_some_and(|stored| version < stored) {
            return Ok(None);
        }
        let Some(mut definition) = entity_definition(value) else {
            return Ok(None);
        };
        roster
            .logged
            .extend(definition.states.iter().filter_map(|state| {
                state.naval_hitmiss_error.as_ref().map(|reason| {
                    format!(
                        "{name} in {path}: state {:?} is unsupported: {reason}",
                        state.name
                    )
                })
            }));
        definition.name = name.clone();
        definition.sources.insert(0, path.clone());
        definition.clone = None;
        let held = parent.as_ref().or(registered).map(|(held, _)| held);
        let kept = held.and_then(|held| held.mesh.clone());
        let mut unbound = held.and_then(|held| held.unbound_mesh.clone());
        definition.mesh = match mesh.filter(|mesh| !mesh.is_empty()) {
            Some(mesh) if roster.descriptors.meshes.contains_key(&mesh) => Some(mesh),
            Some(mesh) => {
                roster.logged.push(format!(
                    "{name} in {path}: no pdxmesh {mesh} is registered, so the engine keeps its mesh binding"
                ));
                unbound = Some(mesh);
                kept
            }
            None => kept,
        };
        definition.unbound_mesh = definition.mesh.is_none().then_some(unbound).flatten();
        for (_, locator) in &mut definition.locators {
            locator.origin_mesh = definition.mesh.clone();
        }
        let entry = match parent {
            Some((parent, version)) => (overlay(parent, definition), version),
            None => (definition, stored.unwrap_or(version)),
        };
        match clone {
            Some(parent) => roster.clones.insert(name.clone(), parent),
            None => roster.clones.remove(&name),
        };
        self.0.insert(name.clone(), entry);
        Ok(Some(name))
    }
}

fn overlay(parent: EntityDefinition, child: EntityDefinition) -> EntityDefinition {
    let mut attachments = parent.attachments;
    merge_attachments(&mut attachments, child.attachments);
    let locators = parent.locators;
    let own: BTreeSet<String> = child
        .states
        .iter()
        .map(|state| state.name.clone())
        .collect();
    let mut states = child.states;
    states.extend(
        parent
            .states
            .into_iter()
            .filter(|state| !own.contains(&state.name)),
    );
    EntityDefinition {
        name: child.name,
        mesh: child.mesh.or(parent.mesh),
        unbound_mesh: child.unbound_mesh,
        clone: None,
        scale: child.scale.or(parent.scale),
        cull_radius: parent.cull_radius,
        attachments,
        default_state: child.default_state.or(parent.default_state),
        states,
        get_state_from_parent: child.get_state_from_parent.or(parent.get_state_from_parent),
        locators,
        game_data: child.game_data.or(parent.game_data),
        sources: child.sources.into_iter().chain(parent.sources).collect(),
    }
}

fn merge_attachments(held: &mut Vec<Attachment>, declared: Vec<Attachment>) {
    for attachment in declared {
        match held.iter().position(|slot| same_slot(slot, &attachment)) {
            Some(at) => held[at] = attachment,
            None => held.push(attachment),
        }
    }
}

fn same_slot(held: &Attachment, declared: &Attachment) -> bool {
    held.name == declared.name && (!held.name.is_empty() || held.joint == declared.joint)
}

pub(super) fn last_text(fields: &[parser::Item], key: &str) -> Option<String> {
    fields
        .iter()
        .rev()
        .find(|item| item.keyword_is(key))
        .and_then(|item| item.value.scalar_str())
        .map(unquote)
}

pub fn lexical_names(bytes: &[u8]) -> BTreeSet<String> {
    parser::scalars(bytes)
        .iter()
        .filter_map(|word| std::str::from_utf8(word).ok())
        .map(unquote)
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct TextureIndex {
    paths: BTreeMap<Vec<u8>, ScriptPath>,
}

impl TextureIndex {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let paths = layers.descriptor_paths("gfx/models", &[".dds", ".tga"])?;
        Ok(Self::new(content_reader_defines::defines::load_order("gfx/models/", "", &paths)))
    }

    pub fn new(paths: impl IntoIterator<Item = ScriptPath>) -> Self {
        let mut index = Self::default();
        for path in paths {
            let bytes = path.as_bytes();
            if !(bytes.ends_with(b".dds") || bytes.ends_with(b".tga")) {
                continue;
            }
            let basename = bytes
                .rsplit(|byte| *byte == b'/')
                .next()
                .unwrap_or(bytes)
                .to_vec();
            index.paths.entry(basename).or_insert(path);
        }
        index
    }

    pub fn resolve(&self, directory: &str, authored: &str) -> io::Result<String> {
        let key = authored.replacen(".tga", ".dds", 1);
        match self.paths.get(key.as_bytes()) {
            Some(path) => path.to_str().map(str::to_owned).ok_or_else(|| {
                invalid(format!(
                    "texture {authored} resolves to {path}, whose path isn't UTF-8 text"
                ))
            }),
            None if directory.is_empty() => Ok(authored.to_owned()),
            None => Ok(format!("{directory}/{authored}")),
        }
    }
}

fn animation_definition(value: &parser::Value, directory: &str) -> Option<AnimationDefinition> {
    let items = value.items()?;
    let name = unquote(text(items, b"name")?);
    let file = unquote(text(items, b"file")?);
    if name.is_empty() || file.is_empty() {
        return None;
    }
    Some(AnimationDefinition {
        name,
        file: if directory.is_empty() {
            file
        } else {
            format!("{directory}/{file}")
        },
    })
}

fn mesh_definition(value: &parser::Value) -> Option<MeshDefinition> {
    let items = value.items()?;
    let scale = items
        .iter()
        .filter(|item| item.keyword_is("scale"))
        .fold(1.0, |held, item| {
            item.value.scalar().and_then(crt::scan_f32).unwrap_or(held)
        });
    let name = unquote(text(items, b"name")?);
    let file = unquote(text(items, b"file")?)
        .trim_start_matches('/')
        .to_owned();
    if name.is_empty() || file.is_empty() {
        return None;
    }
    let settings = items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"meshsettings"))
        .filter_map(|item| mesh_setting(&item.value))
        .collect();
    let animations = items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"animation"))
        .filter_map(|item| {
            let block = item.value.items()?;
            Some((unquote(text(block, b"id")?), unquote(text(block, b"type")?)))
        })
        .collect();
    let cull_distance_squared = items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"cull_distance"))
        .fold(DEFAULT_CULL_DISTANCE_SQUARED, |squared, item| {
            let distance = item
                .value
                .scalar_str()
                .and_then(|value| crt::scan_f32(value.as_bytes()))
                .unwrap_or(squared);
            distance * distance
        });
    Some(MeshDefinition {
        name,
        file,
        scale,
        settings,
        animations,
        cull_distance_squared,
    })
}

fn mesh_setting(value: &parser::Value) -> Option<MeshSetting> {
    let items = value.items()?;
    let mut setting = MeshSetting::default();
    for item in items {
        let Some(value) = item.value.scalar_str() else {
            continue;
        };
        match item.key.as_deref() {
            Some(b"name") => setting.name = unquote(value),
            Some(b"index") => {
                if let content_reader_parser::crt::Scanned::Stored(index) = content_reader_parser::crt::scan_i32(value.as_bytes()) {
                    setting.index = index;
                }
            }
            Some(b"texture_diffuse") => setting.diffuse = Some(unquote(value)),
            Some(b"texture_normal") => setting.normal = Some(unquote(value)),
            Some(b"texture_specular") => setting.specular = Some(unquote(value)),
            Some(b"shader") => setting.shader = Some(unquote(value)),
            Some(b"shader_file") => setting.shader_file = Some(unquote(value)),
            _ => {}
        }
    }
    Some(setting)
}

fn entity_definition(value: &parser::Value) -> Option<EntityDefinition> {
    let items = value.items()?;
    let scale = items
        .iter()
        .filter(|item| item.keyword_is("scale"))
        .fold(None, |held, item| {
            Some(
                item.value
                    .scalar()
                    .and_then(crt::scan_f32)
                    .unwrap_or_else(|| held.unwrap_or(1.0)),
            )
        });
    let name = unquote(text(items, b"name")?);
    if name.is_empty() {
        return None;
    }
    let attachments = items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"attach"))
        .filter_map(|item| {
            let block = item.value.items()?;
            let (joint, entity) = block.iter().find_map(|field| {
                let key = field.key.as_deref()?;
                if key == b"name" {
                    return None;
                }
                Some((
                    unquote(std::str::from_utf8(key).ok()?),
                    unquote(field.value.scalar_str()?),
                ))
            })?;
            Some(Attachment {
                name: text(block, b"name").map(unquote).unwrap_or_default(),
                joint,
                entity,
            })
        })
        .filter(|attachment| !attachment.joint.is_empty() && !attachment.entity.is_empty())
        .collect();
    let states = items
        .iter()
        .filter(|item| item.keyword_is("state"))
        .filter_map(|item| {
            let parser::Value::Block(block) = &item.value else {
                return None;
            };
            let (naval_hitmiss_effects, mut naval_hitmiss_error) =
                state_naval_hitmiss_effects(block);
            let name = state_text(block, "name", &mut naval_hitmiss_error);
            let animation = state_text(block, "animation", &mut naval_hitmiss_error);
            Some(EntityState {
                name,
                animation,
                looping: parser::read_bool_field(block, "looping", true),
                next_state: text(block, b"next_state").map(unquote),
                chance: number(block, b"chance", 1.0),
                chance_weight: Some(state_chance_weight(block)),
                blend: number(block, b"animation_blend_time", 0.0),
                speed: number(block, b"animation_speed", 1.0),
                start: number(block, b"animation_start_time", 0.0),
                random_start: number(block, b"animation_random_start_time", 0.0),
                propagate: block
                    .iter()
                    .filter(|i| i.key.as_deref() == Some(b"propagate_state"))
                    .filter_map(|i| i.value.items())
                    .flatten()
                    .filter_map(|i| {
                        Some((
                            String::from_utf8_lossy(i.key.as_deref()?).into_owned(),
                            unquote(i.value.scalar_str()?),
                        ))
                    })
                    .collect(),
                events: block
                    .iter()
                    .filter(|i| i.key.as_deref() == Some(b"event"))
                    .filter_map(|i| i.value.items())
                    .map(|event| EntityEvent {
                        id: last_text(event, "id"),
                        time: number(event, b"time", 0.0),
                        node: text(event, b"node").map(unquote).unwrap_or_default(),
                        light: text(event, b"light").map(unquote),
                        particle: text(event, b"particle").map(unquote),
                        sound: event
                            .iter()
                            .filter(|item| item.keyword_is("sound"))
                            .filter_map(|item| item.value.items())
                            .filter_map(|fields| last_text(fields, "soundeffect"))
                            .next_back(),
                        keep_particle: parser::read_bool_field(event, "keep_particle", false),
                        trigger_once: parser::read_bool_field(event, "trigger_once", false),
                    })
                    .collect(),
                naval_hitmiss_effects,
                naval_hitmiss_error,
            })
        })
        .collect();
    Some(EntityDefinition {
        name,
        game_data: entity_game_data(items),
        sources: Vec::new(),
        mesh: text(items, b"pdxmesh").map(unquote),
        unbound_mesh: None,
        clone: text(items, b"clone").map(unquote),
        scale,
        cull_radius: Some(
            items
                .iter()
                .filter(|item| item.keyword_is("cull_radius"))
                .fold(1.0, |held, item| {
                    item.value.scalar().and_then(crt::scan_f32).unwrap_or(held)
                }),
        ),
        attachments,
        default_state: text(items, b"default_state").map(unquote),
        get_state_from_parent: items
            .iter()
            .any(|item| item.keyword_is("get_state_from_parent"))
            .then(|| parser::read_bool_field(items, "get_state_from_parent", false)),
        locators: items
            .iter()
            .filter(|i| i.key.as_deref() == Some(b"locator"))
            .filter_map(|i| {
                let fields = i.value.items()?;
                let vector = |key: &[u8]| {
                    let values = fields
                        .iter()
                        .find(|i| i.key.as_deref() == Some(key))
                        .and_then(|i| i.value.items())
                        .unwrap_or(&[])
                        .iter()
                        .filter_map(|i| crt::scan_f32(i.value.scalar_str()?.as_bytes()))
                        .collect::<Vec<_>>();
                    std::array::from_fn::<_, 3, _>(|i| values.get(i).copied().unwrap_or(0.0))
                };
                let r = vector(b"rotation").map(|v| v.to_radians() * 0.5);
                let x = Transform::from_trs([0.0; 3], [r[0].sin(), 0.0, 0.0, r[0].cos()], 1.0);
                let y = Transform::from_trs([0.0; 3], [0.0, r[1].sin(), 0.0, r[1].cos()], 1.0);
                let z = Transform::from_trs([0.0; 3], [0.0, 0.0, r[2].sin(), r[2].cos()], 1.0);
                let scale = text(fields, b"scale")
                    .and_then(|value| crt::scan_f32(value.as_bytes()))
                    .unwrap_or(1.0);
                let mut transform = y.compose(&x).compose(&z).compose(&Transform::from_trs(
                    [0.0; 3],
                    [0.0, 0.0, 0.0, 1.0],
                    scale,
                ));
                transform.translation = vector(b"position");
                Some((
                    unquote(text(fields, b"name")?),
                    Locator {
                        transform,
                        parent_joint: text(fields, b"parent_joint").map(unquote),
                        origin_mesh: None,
                    },
                ))
            })
            .collect(),
        states,
    })
}

fn text<'a>(items: &'a [parser::Item], key: &[u8]) -> Option<&'a str> {
    items
        .iter()
        .find(|item| item.key.as_deref() == Some(key))
        .and_then(|item| item.value.scalar_str())
}

fn state_chance_weight(items: &[parser::Item]) -> i32 {
    items
        .iter()
        .filter(|item| item.keyword_is("chance"))
        .map(|item| {
            let value = item
                .value
                .scalar()
                .and_then(content_reader_parser::fixed5::script_fixed5)
                .unwrap_or(0);
            if value > 0 { value as i32 } else { 0 }
        })
        .next_back()
        .unwrap_or(0)
}

fn unquote(text: &str) -> String {
    text.trim().trim_matches('"').to_string()
}

#[cfg(test)]
mod naval_chance_tests;
