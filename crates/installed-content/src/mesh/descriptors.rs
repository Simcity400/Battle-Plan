use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use crate::{ResolvedLayers, crt, parser};

use super::{Material, SubMesh, Transform, invalid, parse};

/// One `meshsettings` block: a per (shape, material index) override of the
/// `.mesh`'s own textures and effect (section 1.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshSetting {
    /// The shape node name inside the `.mesh`.
    pub name: String,
    /// Raw child ordinal within the named shape; exactly -1 matches every ordinal.
    pub index: i32,
    pub diffuse: Option<String>,
    pub normal: Option<String>,
    pub specular: Option<String>,
    pub shader: Option<String>,
    pub shader_file: Option<String>,
}

impl Default for MeshSetting {
    fn default() -> Self {
        // SMeshData construction in 1422F8720; signed reader 142251AE0(+D4).
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
    /// 14224F250: an empty name matches immediately, regardless of index.
    pub fn applies(&self, shape: &str, child_index: usize) -> bool {
        self.name.is_empty()
            || (self.name == shape
                && (self.index == -1 || usize::try_from(self.index).ok() == Some(child_index)))
    }
}

/// One `pdxmesh` block of a `gfx/entities/*.gfx` file (section 1.4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshDefinition {
    pub name: String,
    /// The `.mesh` path, relative to the content root.
    pub file: String,
    /// `scale`, default 1.0.
    pub scale: f32,
    pub settings: Vec<MeshSetting>,
    /// `animation = { id = … type = … }`: the animation **name** this mesh
    /// plays for each state animation id, in file order.
    pub animations: Vec<(String, String)>,
}

impl MeshDefinition {
    /// 14224F250 selects the first matching row. 14224CC00/14224BB00 apply only
    /// nonempty texture/shader overrides; later matching rows are never merged.
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

/// One `attach` block of an `entity` (section 1.5).
///
/// The block is `attach = { name = "<own name>" <joint> = "<entity>" }`: the
/// `name` is the attachment's own handle, which `propagate_state` addresses and
/// which a `clone` overrides by, and the one other key is a joint of the
/// parent's skeleton verbatim — `Right_Hand_node`, `Root_node_2`, `vehicle`,
/// `infantry`, `horse`, `artillery`, `bike` are all joint names in the
/// matching `.mesh`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attachment {
    /// The attachment's own handle.
    pub name: String,
    /// The parent skeleton's joint it hangs from.
    pub joint: String,
    /// The entity attached there.
    pub entity: String,
}

impl Attachment {
    /// The name `propagate_state` addresses the attachment by: its own
    /// handle, or its joint when the block names no handle.
    pub fn handle(&self) -> &str {
        if self.name.is_empty() {
            &self.joint
        } else {
            &self.name
        }
    }
}

/// One `entity` block of a `gfx/entities/*.asset` file (section 1.5).
///
/// Definitions retain states and events; consumers own their playback clocks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityDefinition {
    pub name: String,
    /// `pdxmesh`, the key of a [`MeshDefinition`].
    pub mesh: Option<String>,
    /// Failed authored binding when no native mesh survives (`142272140`).
    pub unbound_mesh: Option<String>,
    /// `clone`, the entity this one inherits from before its own overrides.
    pub clone: Option<String>,
    pub scale: Option<f32>,
    /// Native held `cull_radius`, default 1. A clone keeps its parent's radius.
    pub cull_radius: Option<f32>,
    /// `attach`, in file order.
    pub attachments: Vec<Attachment>,
    /// `default_state`; the engine's own default is `idle`.
    pub default_state: Option<String>,
    /// `state = { name = … animation = … }` pairs, in file order. A state name
    /// occurs several times when the engine picks between weighted variants;
    /// the first is taken.
    pub states: Vec<EntityState>,
    pub get_state_from_parent: Option<bool>,
    pub locators: Vec<(String, Locator)>,
    pub game_data: Option<Result<EntityGameData, String>>,
}

///140B65040: allocated on the first entity-level game_data field.
#[derive(Clone, Copy, Debug, PartialEq)]
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
    // CReader::ReadFloat@1424AB9D0 and ReadScriptSignedInt32@1424AB970 use CRT scans.
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
                    //1402C7A50 accepts ordered x/y labels as well as bare values.
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
            _ => {} //Native logs unknown keys after allocating the defaults.
        }
    }
    Some(Ok(data))
}

/// One entity `locator`: a transform in entity space, or in the space of its
/// `parent_joint`. `SEntityLocatorReader::ReadMember` (`14233BE00`) reads
/// `name`, `position`, `rotation` (degrees), `scale` (default 1) and
/// `parent_joint`; the lexer registers `locator` as token 0x1FF and
/// `parent_joint` as 0x200 (`14008AA8B`..`14008AAE7`). `14227D8C0` resolves
/// the joint name; one the skeleton lacks is logged and the locator stays in
/// entity space. No vanilla entity names a joint.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Locator {
    pub transform: Transform,
    pub parent_joint: Option<String>,
    /// Registration mesh whose skeleton ordinal a clone copies unchanged.
    pub origin_mesh: Option<String>,
}

/// Authored state variants retain their order and events, including states with no clip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityState {
    pub name: String,
    pub animation: String,
    pub looping: bool,
    pub next_state: Option<String>,
    pub chance: f32,
    /// Native `14233CC10` Fixed5 chance, stored as the low i32 word.
    /// Parsed states retain the genuine omitted/failed-read zero. `None`
    /// preserves legacy manually constructed states' float weighting.
    pub chance_weight: Option<i32>,
    pub blend: f32,
    pub speed: f32,
    pub start: f32,
    pub random_start: f32,
    pub propagate: Vec<(String, String)>,
    pub events: Vec<EntityEvent>,
    /// State-mode `game_data.hitmiss_effect` rows (`140B65040`), in authored order.
    pub naval_hitmiss_effects: Vec<NavalHitMissEffect>,
    /// A named unsupported form. Consumers
    /// must refuse these timings instead of treating the retained prefix as complete.
    pub naval_hitmiss_error: Option<String>,
}

/// `SNavalHitMissDataReader::vf4@140B65260`: each appended row starts at zero.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NavalHitMissEffect {
    pub time: f32,
    /// Native `small` = 0 and `big` = 1; an unknown type retains the previous value.
    pub kind: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityEvent {
    /// Native callback name (`14233BB30`, dispatched by `14123ACB0`).
    pub id: Option<String>,
    pub time: f32,
    pub node: String,
    pub light: Option<String>,
    pub particle: Option<String>,
    /// Played by `1422701A3`; retained for consumers without audio.
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
                // 140B65040 reports an unexpected field. Its recovery
                // (1424AD270) does nothing for a consumed literal scalar;
                // CPersistent::Read logs/clears that error and continues.
                if fully_consumed_literal(field) {
                    continue;
                }
                // An unknown block skip resolves variables. The tree cannot
                // reproduce that table or an unresolved value's block expansion.
                error.get_or_insert_with(|| {
                    format!(
                        "state game_data has unexpected field {} (140B65040)",
                        String::from_utf8_lossy(field.key.as_deref().unwrap_or_default())
                    )
                });
                continue;
            }
            let parser::Value::Block(record) = &field.value else {
                error.get_or_insert_with(|| "state hitmiss_effect is not a native block".into());
                continue;
            };
            // 140B65040 zeroes one 8-byte row before reading and appending it.
            let mut effect = NavalHitMissEffect::default();
            for member in record {
                let time = member.keyword_is("time");
                let kind = member.keyword_is("type");
                if !time && !kind {
                    // 140B65260 ignores unknown scalar fields. An unconsumed
                    // block is read as subsequent keys by CPersistent::Read;
                    // unresolved variables can alter that same reader table.
                    if !fully_consumed_literal(member) {
                        error.get_or_insert_with(|| {
                            "state hitmiss_effect has an unsettled unknown member (140B65260)"
                                .into()
                        });
                    }
                    continue;
                }
                let Some(value) = member.value.scalar() else {
                    error.get_or_insert_with(|| {
                        "state hitmiss_effect time/type has an unconsumed block (140B65260)".into()
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
                    // ReadFloat@1424AB9D0 uses the shared CRT prefix scan;
                    // a conversion that stores nothing leaves the previous bits.
                    // Its scalar error recovery does not poison the following rows.
                    if let Some(value) = crt::scan_f32(value) {
                        effect.time = value;
                    }
                } else {
                    // ReadStringInto@1424ABB50 then String_ToLowerInPlace:
                    // exact whole-string comparisons, including quoted values.
                    // A reported LF becomes "Unreadable String", hence leaves
                    // the prior kind just as every other unknown type does.
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

/// Unknown scalar fields are fully consumed only when no script-variable
/// definition/reference can change the native token's kind or block recovery.
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

/// Structural consumption checks for the reached state text and naval-data
/// readers only. Encoding refusals and known scalar no-ops do not unsettle a file.
fn unsettled_state_data(fields: &[parser::Item]) -> Option<&'static str> {
    for field in fields {
        if field.keyword_is("name") || field.keyword_is("animation") {
            if literal_value(field).is_none() {
                return Some(
                    "state name/animation input has unsettled native consumption (14233CC10)",
                );
            }
        } else if field.keyword_is("game_data") {
            let parser::Value::Block(data) = &field.value else {
                return Some("state game_data carrier is not a native block (14233CC10)");
            };
            for member in data {
                if !member.keyword_is("hitmiss_effect") {
                    if !fully_consumed_literal(member) {
                        return Some(
                            "state game_data member has unsettled native skip (140B65040)",
                        );
                    }
                    continue;
                }
                let parser::Value::Block(record) = &member.value else {
                    return Some("hitmiss_effect carrier is not a native block (140B65040)");
                };
                for value in record {
                    if value.keyword_is("time") || value.keyword_is("type") {
                        if literal_value(value).is_none() {
                            return Some(
                                "hitmiss_effect time/type input has unsettled native consumption (140B65260)",
                            );
                        }
                    } else if !fully_consumed_literal(value) {
                        return Some(
                            "hitmiss_effect member has unsettled native consumption (140B65260)",
                        );
                    }
                }
            }
        }
    }
    None
}

/// SEntityStateReader::vf4@14233CC10 writes both name and animation with
/// strncpy(255), followed by a NUL. No whitespace trim or LF replacement.
fn state_text(items: &[parser::Item], key: &str, error: &mut Option<String>) -> String {
    let mut stored: &[u8] = b"";
    for field in items.iter().filter(|item| item.keyword_is(key)) {
        let Some(value) = field.value.scalar() else {
            error.get_or_insert_with(|| format!("state {key} has an unconsumed block (14233CC10)"));
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
    // Each native write replaces the stored bytes. An earlier unrepresentable
    // truncation must not refuse a later valid overwrite of the same field.
    match std::str::from_utf8(stored) {
        Ok(value) => value.to_owned(),
        Err(_) => {
            error.get_or_insert_with(|| {
                format!("state {key} native 255-byte text is not UTF-8 (14233CC10)")
            });
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

/// One `animation = { name = … file = … }` block of a `gfx/models/**/*.asset`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnimationDefinition {
    pub name: String,
    /// The `.anim` path, already resolved against the descriptor's own
    /// directory and relative to the content root.
    pub file: String,
}

/// What a name resolved to: the `.mesh` to load, its world scale and its
/// material overrides.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMesh<'a> {
    pub definition: &'a MeshDefinition,
    /// `entity.scale` times `pdxmesh.scale`.
    ///
    /// UNVERIFIED (section 1.5): whether the two compose or the entity's wins.
    /// In shipped data only one of the two is ever set for a map object, so on
    /// this content the two readings agree; every `mapobject_*` sets only the
    /// `pdxmesh` scale.
    pub scale: f32,
}

/// The `pdxmesh` and `entity` blocks of a set of descriptor files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Descriptors {
    pub meshes: BTreeMap<String, MeshDefinition>,
    pub entities: BTreeMap<String, EntityDefinition>,
    /// `animation` blocks, keyed by the name a `pdxmesh` refers to them by.
    pub animations: BTreeMap<String, AnimationDefinition>,
}

/// An entity with its `clone` chain already applied (section 1.5).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InheritedEntity<'a> {
    /// `pdxmesh`, from the nearest link that declares one.
    pub mesh: Option<&'a str>,
    /// Diagnostic name when the native binding stayed absent.
    pub unbound_mesh: Option<&'a str>,
    /// `scale`, from the nearest link that declares one.
    pub scale: Option<f32>,
    /// The ordinary ancestor's held radius: the native clone reader never
    /// sets the authorship byte that would allow a child to replace it.
    pub cull_radius: Option<f32>,
    /// `default_state`, from the nearest link that declares one.
    pub default_state: Option<&'a str>,
    /// Every attachment reachable through the chain, merged down it as each
    /// clone merges its own (`14227CEF0`): a declaration replaces the held
    /// one of its slot where it stands, a new slot is appended.
    pub attachments: Vec<&'a Attachment>,
    /// State name to animation id, the nearest declaration winning.
    pub states: BTreeMap<&'a str, Vec<&'a EntityState>>,
    pub get_state_from_parent: Option<bool>,
    pub locators: BTreeMap<&'a str, &'a Locator>,
    pub game_data: Option<&'a Result<EntityGameData, String>>,
}

impl Descriptors {
    /// Resolves an entity's `clone` chain into one flattened definition.
    ///
    /// **The engine's `clone` copies the whole entity and then applies the
    /// overrides**, so a field the child does not declare is the parent's even
    /// when the child declares some other field. The shipped data needs that
    /// reading and the frames confirm it: `GER_infantry_entity` is
    /// `clone = "infantry_rifle_entity"` plus its own `pdxmesh` and its own
    /// five `attach` handles, and it declares no `scale`, no `default_state`
    /// and no `idle` state; the game draws it at `infantry_rifle_entity`'s
    /// `scale = 0.8` (`pairD_in10.png`: the game's helmet is 31.7 device pixels
    /// across and the unscaled figure's 40) and plays that entity's idle
    /// animation. A field-by-field override also explains every `attach`
    /// handle in the file: `GER_artillery_entity` clones `artillery_entity` and
    /// redeclares exactly the two handles `artillery` and `infantry` that it
    /// wants to point elsewhere.
    ///
    /// The walk is bounded by the number of entities, so a cyclic `clone`
    /// terminates.
    pub fn inherit(&self, name: &str) -> Option<InheritedEntity<'_>> {
        let mut out = InheritedEntity::default();
        let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
        let mut current = self.entities.get(name)?;
        let mut chain = Vec::new();
        for _ in 0..=self.entities.len() {
            chain.push(current);
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
            let Some(parent) = self.entities.get(parent) else {
                break;
            };
            current = parent;
        }
        // From the farthest ancestor down, as each clone's merge left them.
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

    /// Resolves an entity name to its mesh, world scale, attachments and the
    /// animation its default state plays.
    ///
    /// A name that is a `pdxmesh` and not an entity resolves to that mesh with
    /// no attachments, which is what every `mapobject_<paletteIndex>` the tree
    /// layer asks for is.
    pub fn resolve_entity(&self, name: &str) -> Option<ResolvedEntity<'_>> {
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

/// The state an entity that declares no `default_state` is in.
pub const DEFAULT_STATE: &str = "idle";

/// What an entity name resolved to, with its `clone` chain applied.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedEntity<'a> {
    pub definition: &'a MeshDefinition,
    /// `entity.scale * pdxmesh.scale`.
    pub scale: f32,
    pub attachments: Vec<&'a Attachment>,
    /// The `.anim` the entity's default state plays, when one resolves.
    pub animation: Option<&'a AnimationDefinition>,
}

impl Descriptors {
    /// Resolves an entity or mesh name to its `.mesh` and world scale.
    ///
    /// **Entities are tried first.** The two tables share a namespace and the
    /// shipped content collides in it: `gfx/entities/buildings.gfx:4-8`
    /// declares a *pdxmesh* named `westerngfx_house_1_1` and
    /// `buildings.asset:6-10` declares an *entity* of the same name whose only
    /// content is `pdxmesh = "westerngfx_house_1_1"` and `scale = 1.4`. Taking
    /// the mesh first loses that scale, and the sprawl houses come out at 1.0
    /// against the game's 1.4 (or 1.8 and 2.0 for the taller rings). Every
    /// name that exists in only one table resolves the same either way, which
    /// includes every `mapobject_<paletteIndex>` the tree layer asks for
    /// (R3 section 3.1): those are pdxmesh names with no entity.
    ///
    /// An entity name follows its whole `clone` chain, field by field; see
    /// [`Descriptors::inherit`].
    pub fn resolve(&self, name: &str) -> Option<ResolvedMesh<'_>> {
        self.resolve_entity(name).map(|entity| ResolvedMesh {
            definition: entity.definition,
            scale: entity.scale,
        })
    }
}

/// Reads the blocks of one already-read descriptor file. `path` is the file's
/// own path, which an `animation` block's `file` resolves against.
pub fn parse_descriptors(bytes: &[u8], path: &str) -> io::Result<Descriptors> {
    let document = parser::parse(bytes)?;
    settled_state_carriers(&document, path)?;
    let mut out = Descriptors::default();
    let _ = collect(&document, path, &mut out);
    Ok(out)
}

/// Unported entity-field consumption can shift the surrounding native file
/// out of tree alignment. Refuse the file before registering any declaration;
/// representation-only failures remain scoped to the final stored state text.
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
                "{path}: scalar/tagged entity state carrier leaves the native file reader alignment unsupported (14233BF40)"
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
                    "{path}: entity {name} cull_radius variable/block/tagged carrier has unsupported native consumption (14233C3FC); descriptor file refused"
                )));
            }
        }
    }
    Ok(())
}

/// Adds one file's blocks; returns the names that replaced an earlier block.
fn collect(document: &parser::Document, path: &str, out: &mut Descriptors) -> Vec<String> {
    let directory = path.rsplit_once('/').map_or("", |(head, _)| head);
    let mut replaced = Vec::new();
    for item in &document.items {
        match item.key.as_deref() {
            Some(b"objectTypes") => {
                for block in item.value.items().unwrap_or(&[]) {
                    if block.key.as_deref() == Some(b"pdxmesh")
                        && let Some(definition) = mesh_definition(&block.value)
                        && let Some(old) = out.meshes.insert(definition.name.clone(), definition)
                    {
                        replaced.push(old.name);
                    }
                }
            }
            Some(b"entity") => {
                if let Some(definition) = entity_definition(&item.value)
                    && let Some(old) = out.entities.insert(definition.name.clone(), definition)
                {
                    replaced.push(old.name);
                }
            }
            Some(b"animation") => {
                if let Some(definition) = animation_definition(&item.value, directory)
                    && let Some(old) = out.animations.insert(definition.name.clone(), definition)
                {
                    replaced.push(old.name);
                }
            }
            _ => {}
        }
    }
    replaced
}

/// Drops unmatched closing braces and closes blocks left open at the end,
/// leaving strings and comments alone. Several shipped descriptors need it,
/// and the game loads them; callers use it only after a parse fails.
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

/// Every descriptor the engine loads, registered as the engine registers it.
///
/// Three registries, each filled in its own pass; no file name is derived from
/// an identifier, and stray braces are balanced as the engine's reader
/// tolerates them:
/// - Meshes. `SGfxFileReader::ReadToken` (`1422264C0`) reads each object of
///   the `.gfx` files in the particle catalog's order (startup, `interface`,
///   then `gfx`) and prepends it to its name's bucket (`1406B1110`); the
///   lookup (`142223D70`) takes the first exact match, so the last object of a
///   name wins. `142272140` binds an entity's `pdxmesh` only to an object of
///   type 0x180, so a name whose last object is an `arrowType`,
///   `animatedmaptext` or `pdxparticle` is no mesh.
/// - Animations and lights. The asset factory (`142218E80` from
///   `CApplication::Initialize` `142218780`, entity flag clear) reads every
///   `gfx/**/*.asset` in `Vfs_EnumerateRecursive` order, and `142338C30`
///   registers each `animation` whose file loads (`14224D620`: it opens and
///   holds `[info]` and `[samples]`, `14224D6A0`), a later one replacing the
///   clip (`142399290`), and each `light` (token 0x54, `SAnimatedLightReader`),
///   a later one of the name rewriting the record (`1423C96F0`).
/// - Entities. `LoadAssets` (`142218E80` from `CGameApplication::Initialize`
///   `1401806B0`, entity flag set) reads the same files again for `entity`
///   blocks only: see [`Entities`].
#[derive(Clone, Debug, Default)]
pub struct Roster {
    pub descriptors: Descriptors,
    /// Each `light` block's fields by name, the last declaration's.
    pub lights: BTreeMap<String, Vec<parser::Item>>,
    /// What entity registration logs: dropped clones and failed mesh bindings, each with why.
    pub logged: Vec<String>,
    /// Files that do not parse even when closed, with the error.
    pub unreadable: Vec<String>,
    /// Every word such a file holds ([`lexical_names`]), with the first
    /// such file. Which of its blocks the engine still registers is
    /// unverified, so each could be a declaration a consumer cannot see. A
    /// clone whose parent is such a word is one too, with the parent's file:
    /// the parent it copied may be that file's.
    pub unreadable_names: BTreeMap<String, String>,
}

/// The objects of the mesh registry (`1422264C0` tokens 0x180, 0x57, 0x10E
/// and 0x181); only the first is a mesh.
const OBJECTS: [&str; 4] = ["pdxmesh", "arrowType", "animatedmaptext", "pdxparticle"];

/// `14224D6A0` copies an `animation` name, and `1423C96F0` a light's, into 64
/// bytes, and `14233BF40` an
/// entity's name, `clone` and `pdxmesh` into 256, each ending with a NUL. A
/// longer name would be cut, which is not modelled.
const ANIMATION_NAME: usize = 63;
const ENTITY_NAME: usize = 255;

impl Roster {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut roster = Self::default();
        let interface = layers.descriptor_paths("interface", &[".gfx"])?;
        let gfx = layers.descriptor_paths("gfx", &[".gfx", ".asset"])?;
        let mut objects = BTreeMap::new();
        for path in crate::particles::object_file_order(&interface, &gfx) {
            if let Some(document) = roster.parse(layers, &path, &OBJECTS)? {
                add_objects(&document, &mut objects);
            }
        }
        roster.descriptors.meshes = objects
            .into_iter()
            .filter_map(|(name, mesh)| Some((name, mesh?)))
            .collect();
        let mut animations = BTreeMap::<String, Vec<AnimationDefinition>>::new();
        let mut entities = Entities::default();
        for path in crate::defines::load_order("gfx/", ".asset", gfx.iter().map(String::as_str)) {
            let Some(document) = roster.parse(layers, &path, &["entity", "animation", "light"])?
            else {
                continue;
            };
            let directory = path.rsplit_once('/').map_or("", |(head, _)| head);
            for item in &document.items {
                if item.keyword_is("animation") {
                    let Some(animation) = animation_definition(&item.value, directory) else {
                        continue;
                    };
                    if animation.name.len() > ANIMATION_NAME {
                        return Err(invalid(format!(
                            "{path}: animation {} is longer than the {ANIMATION_NAME} bytes the engine keeps",
                            animation.name
                        )));
                    }
                    animations
                        .entry(animation.name.clone())
                        .or_default()
                        .push(animation);
                } else if item.keyword_is("entity") {
                    entities.declare(&item.value, &path, &mut roster)?;
                } else if item.keyword_is("light")
                    && let Some(fields) = item.value.items()
                {
                    let name = last_text(fields, "name").unwrap_or_default();
                    if name.len() > ANIMATION_NAME {
                        return Err(invalid(format!(
                            "{path}: light {name} is longer than the {ANIMATION_NAME} bytes the engine keeps"
                        )));
                    }
                    roster.lights.insert(name, fields.to_vec());
                }
            }
        }
        for (name, declarations) in animations {
            let animation = loaded_animation(layers, declarations)?;
            roster.descriptors.animations.insert(name, animation);
        }
        roster.descriptors.entities = entities
            .0
            .into_iter()
            .map(|(name, (definition, _))| (name, definition))
            .collect();
        Ok(roster)
    }

    /// The file's document when it holds any of `words` (without case, as the
    /// lexer folds keywords), else `None`; a file that does not parse is
    /// recorded as unreadable.
    fn parse(
        &mut self,
        layers: &ResolvedLayers,
        path: &str,
        words: &[&str],
    ) -> io::Result<Option<parser::Document>> {
        let bytes = layers.read_uncovered(path)?;
        if !words.iter().any(|word| {
            bytes
                .windows(word.len())
                .any(|window| window.eq_ignore_ascii_case(word.as_bytes()))
        }) {
            return Ok(None);
        }
        match parser::parse(&bytes)
            .or_else(|_| parser::parse(&balanced_blocks(&bytes)))
            .and_then(|document| {
                settled_state_carriers(&document, path)?;
                Ok(document)
            }) {
            Ok(document) => Ok(Some(document)),
            Err(error) => {
                for name in lexical_names(&bytes) {
                    self.unreadable_names
                        .entry(name)
                        .or_insert_with(|| path.to_owned());
                }
                self.unreadable.push(format!("{path} ({error})"));
                Ok(None)
            }
        }
    }
}

/// Registers one `.gfx` file's objects: the latest object of a name is the one
/// found, a mesh or, for another kind, `None`.
fn add_objects(
    document: &parser::Document,
    objects: &mut BTreeMap<String, Option<MeshDefinition>>,
) {
    for item in document
        .items
        .iter()
        .filter(|item| item.keyword_is("objectTypes"))
    {
        for object in item.value.items().unwrap_or(&[]) {
            if object.keyword_is(OBJECTS[0]) {
                if let Some(mesh) = mesh_definition(&object.value) {
                    objects.insert(mesh.name.clone(), Some(mesh));
                }
            } else if OBJECTS[1..].iter().any(|kind| object.keyword_is(kind))
                && let Some(name) = object
                    .value
                    .items()
                    .and_then(|fields| last_text(fields, "name"))
            {
                objects.insert(name, None);
            }
        }
    }
}

/// The declaration of an animation name the engine keeps: the last whose file
/// loads (`14224D620`). Declarations of one file need no reading, and when
/// none loads no clip does either, so the last stands.
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

/// The entity registry as `LoadAssets` fills it, block by block, each name with
/// its definition and stored version.
///
/// A plain block is `create_or_reuse` (`14227D8C0`); a block with a `clone` is
/// `update` (`14227CEF0`) of a copy of the parent as registered at that moment
/// (`entity_definition_lookup` `14227E5F0`, exact name), and is dropped when no
/// parent is registered yet ("Couldn't find parent clone entity",
/// `142338C30`). Either way a name already registered keeps its definition when
/// the block's `version` (`ParseTokenInt32`) is lower than the stored one, and
/// is replaced otherwise. The stored version is the one the name was created
/// with, and a clone stores its parent's (`14226C110` and `14226D5E0` copy it;
/// neither path writes the block's).
#[derive(Default)]
struct Entities(BTreeMap<String, (EntityDefinition, i32)>);

impl Entities {
    fn declare(
        &mut self,
        value: &parser::Value,
        path: &str,
        roster: &mut Roster,
    ) -> io::Result<()> {
        let Some(fields) = value.items() else {
            return Ok(());
        };
        let Some(name) = last_text(fields, "name").filter(|name| !name.is_empty()) else {
            return Ok(());
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
            .map_or(0, crate::fixed5::decimal_i32);
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
                    return Ok(());
                }
            },
        };
        let registered = self.0.get(&name);
        let stored = registered.map(|(_, stored)| *stored);
        if stored.is_some_and(|stored| version < stored) {
            return Ok(());
        }
        let Some(mut definition) = entity_definition(value) else {
            return Ok(());
        };
        // An unsupported state name can prevent lookup of its timing list
        // altogether. Surface the refusal through the existing entity diagnostics.
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
        definition.clone = None;
        // 142272140 binds the named object only when it is a `pdxmesh`, and
        // otherwise logs and leaves the bindings as they are: the parent's
        // copy (14226C110, 14226D5E0) or the definition being redefined,
        // whose bindings create_or_reuse (14227D8C0) does not clear.
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
        self.0.insert(name, entry);
        Ok(())
    }
}

/// A clone as `14227CEF0` builds it: the parent's definition, each field the
/// block declares replacing the parent's as [`Descriptors::inherit`] reads a
/// chain, and its attachments merged ([`merge_attachments`]).
fn overlay(parent: EntityDefinition, child: EntityDefinition) -> EntityDefinition {
    let mut attachments = parent.attachments;
    merge_attachments(&mut attachments, child.attachments);
    //14227CEF0 copies the parent's ordered array and ignores child locators.
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
        //14227D0AF tests description+448, but token500 never sets that byte.
        cull_radius: parent.cull_radius,
        attachments,
        default_state: child.default_state.or(parent.default_state),
        states,
        get_state_from_parent: child.get_state_from_parent.or(parent.get_state_from_parent),
        locators,
        game_data: child.game_data.or(parent.game_data),
    }
}

/// `14227CEF0`'s attachment merge: each attachment the clone declares
/// replaces, where it stands, the held one of the same name (of the same
/// joint when both names are empty), and is appended otherwise.
fn merge_attachments(held: &mut Vec<Attachment>, declared: Vec<Attachment>) {
    for attachment in declared {
        match held.iter().position(|slot| same_slot(slot, &attachment)) {
            Some(at) => held[at] = attachment,
            None => held.push(attachment),
        }
    }
}

/// Whether a clone's attachment replaces a held one (`14227CEF0`): the same
/// name, and the same joint when both names are empty.
fn same_slot(held: &Attachment, declared: &Attachment) -> bool {
    held.name == declared.name && (!held.name.is_empty() || held.joint == declared.joint)
}

/// The text of the last `key` a block holds, as the reader's last write of
/// that field leaves it.
pub(super) fn last_text(fields: &[parser::Item], key: &str) -> Option<String> {
    fields
        .iter()
        .rev()
        .find(|item| item.keyword_is(key))
        .and_then(|item| item.value.scalar_str())
        .map(unquote)
}

/// Every name a descriptor file could declare, read without parsing it:
/// each scalar the shared lexer reads ([`parser::scalars`], which keeps
/// scanning past the error that made the parse fail), decoded and trimmed
/// as a parsed name is (`scalar_str`, then [`unquote`]).
pub fn lexical_names(bytes: &[u8]) -> BTreeSet<String> {
    parser::scalars(bytes)
        .iter()
        .filter_map(|word| std::str::from_utf8(word).ok())
        .map(unquote)
        .collect()
}

/// `InitTextureLookup(gfx/models)`: `142251F00` indexes every `.dds` and
/// `.tga` below `gfx/models` by its stored basename.
#[derive(Clone, Debug, Default)]
pub struct TextureIndex {
    paths: BTreeMap<String, String>,
}

impl TextureIndex {
    /// `142251F00` walks `gfx/models` with `Vfs_EnumerateRecursive`
    /// (`1424C7630`, no filter, any depth), in [`crate::defines::load_order`].
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let paths = layers.descriptor_paths("gfx/models", &[".dds", ".tga"])?;
        Ok(Self::new(crate::defines::load_order(
            "gfx/models/",
            "",
            paths.iter().map(String::as_str),
        )))
    }

    /// Indexes `paths` in enumeration order: each basename keeps the first
    /// file (the FNV-1a and byte-compare map of `142251F00`; a later file of
    /// that basename is only reported, `1422527C0`).
    pub fn new(paths: impl IntoIterator<Item = String>) -> Self {
        let mut index = Self::default();
        for path in paths {
            // The suffix test is byte-exact (ends_with 1424B6AB0).
            if !(path.ends_with(".dds") || path.ends_with(".tga")) {
                continue;
            }
            let basename = path.rsplit('/').next().unwrap_or(&path).to_owned();
            index.paths.entry(basename).or_insert(path);
        }
        index
    }

    /// The one path `14224CC00` requests for a material texture.
    /// `142251230` looks the authored string up byte-exactly after spelling
    /// its first `.tga` as `.dds`; a miss keeps the mesh directory plus the
    /// authored name.
    pub fn resolve(&self, directory: &str, authored: &str) -> String {
        let key = authored.replacen(".tga", ".dds", 1);
        match self.paths.get(&key) {
            Some(path) => path.clone(),
            None if directory.is_empty() => authored.to_owned(),
            None => format!("{directory}/{authored}"),
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
        // 142338C30 joins the descriptor's directory, `/` and the file,
        // whatever the file holds.
        file: if directory.is_empty() {
            file
        } else {
            format!("{directory}/{file}")
        },
    })
}

fn mesh_definition(value: &parser::Value) -> Option<MeshDefinition> {
    let items = value.items()?;
    let name = unquote(text(items, b"name")?);
    // PhysFS drops a path's leading slashes (`sanitizePlatformIndependentPath`),
    // so `/gfx/models/...`, as the shipped supply-train meshes spell it, is
    // the content-root path.
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
    Some(MeshDefinition {
        name,
        file,
        scale: text(items, b"scale")
            .and_then(|value| crt::scan_f32(value.as_bytes()))
            .unwrap_or(1.0),
        settings,
        animations,
    })
}

fn mesh_setting(value: &parser::Value) -> Option<MeshSetting> {
    let items = value.items()?;
    let mut setting = MeshSetting::default();
    // 142251AE0 writes fields in reader order; failed numeric conversions retain the field.
    for item in items {
        let Some(value) = item.value.scalar_str() else {
            continue;
        };
        match item.key.as_deref() {
            Some(b"name") => setting.name = unquote(value),
            Some(b"index") => {
                if let crate::crt::Scanned::Stored(index) = crate::crt::scan_i32(value.as_bytes()) {
                    setting.index = index;
                }
            }
            Some(b"texture_diffuse") => setting.diffuse = Some(unquote(value)),
            Some(b"texture_normal") => setting.normal = Some(unquote(value)),
            Some(b"texture_specular") => setting.specular = Some(unquote(value)),
            Some(b"shader") => setting.shader = Some(unquote(value)),
            // Token 0x2A6 at 14008C426; reader142251AE0 writes SMeshData+A8.
            Some(b"shader_file") => setting.shader_file = Some(unquote(value)),
            _ => {}
        }
    }
    Some(setting)
}

fn entity_definition(value: &parser::Value) -> Option<EntityDefinition> {
    let items = value.items()?;
    let name = unquote(text(items, b"name")?);
    if name.is_empty() {
        return None;
    }
    let attachments = items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"attach"))
        .filter_map(|item| {
            let block = item.value.items()?;
            // The joint is whichever key is not `name`; the shipped blocks
            // carry exactly those two.
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
            // Both public and layered registration paths preflight these
            // immediate carriers before projecting any declaration in the file.
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
                // 14233CC10: +0x21C, true by default (142281510).
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
                        // 14233BB30: +0x714/715.
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
        mesh: text(items, b"pdxmesh").map(unquote),
        unbound_mesh: None,
        clone: text(items, b"clone").map(unquote),
        scale: text(items, b"scale").and_then(|value| crt::scan_f32(value.as_bytes())),
        //1422813B0 initializes +44C to 1;14233C3FC scans into the held
        //word, so a failed later scan preserves its previous value.
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
        // 14233BF40: +0x414; +0x415 marks authorship even on failure.
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

/// Each native chance read starts with zero; only a positive Fixed5 result
/// is copied to the state's low word (`14233CC10`, `142281510`).
fn state_chance_weight(items: &[parser::Item]) -> i32 {
    items
        .iter()
        .filter(|item| item.keyword_is("chance"))
        .map(|item| {
            let value = item
                .value
                .scalar()
                .and_then(crate::fixed5::script_fixed5)
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
