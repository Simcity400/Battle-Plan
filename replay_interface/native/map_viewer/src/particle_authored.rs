use crate::mod_resolver::{ContentAdmissionLease, GameFileResolver};
use portable_content_manifest::paradox_text::{self, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ParticleInputRefusal {
    pub event_instance_id: Option<u64>,
    pub definition: String,
    pub subsystem: Option<String>,
    pub input: String,
    pub reason: ParticleRefusalReason,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ParticleRefusalReason {
    ContentAdmissionRequired,
    AuthenticatedContentUnavailable(String),
    MalformedContent,
    MissingRegistration,
    ConflictingRegistration,
    MissingDefinition,
    ConflictingDefinition,
    MissingAuthoredInput,
    UnsupportedAuthoredInput,
    InvalidNumber,
    CapacityExceeded,
    WrongContentSession,
    DuplicateEventInstance,
    UnknownHandle,
    PresentationOutOfOrder,
    DuplicatePresentation,
    DuplicateNotification,
    ArithmeticOverflow,
}

impl std::fmt::Display for ParticleInputRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PARTICLE_INPUT_REFUSAL {:?}: {} / {:?} / {} / {:?}",
            self.reason, self.definition, self.subsystem, self.input, self.event_instance_id
        )
    }
}
impl std::error::Error for ParticleInputRefusal {}

impl ParticleInputRefusal {
    pub(crate) fn new(
        definition: &str,
        subsystem: Option<&str>,
        input: &str,
        reason: ParticleRefusalReason,
    ) -> Self {
        Self {
            event_instance_id: None,
            definition: definition.into(),
            subsystem: subsystem.map(str::to_owned),
            input: input.into(),
            reason,
        }
    }
    pub(crate) fn event(mut self, id: u64) -> Self {
        self.event_instance_id = Some(id);
        self
    }
}

pub(crate) type ParticleResult<T> = std::result::Result<T, ParticleInputRefusal>;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VariedScalar {
    pub base: f32,
    pub range: f32,
    base_constant: bool,
    range_constant: bool,
    pub base_animations: Vec<Arc<ParticleAnimation>>,
    pub range_animations: Vec<Arc<ParticleAnimation>>,
}
impl VariedScalar {
    pub(crate) fn plain(base: f32, range: f32) -> Self {
        Self {
            base,
            range,
            base_constant: base != 0.0,
            range_constant: range != 0.0,
            base_animations: Vec::new(),
            range_animations: Vec::new(),
        }
    }
    pub(crate) fn update_particle(
        &self,
        sampled: f32,
        current: f32,
        system_time: f32,
        age: f32,
        life: f32,
    ) -> f32 {
        // The update callback is a no-op unless the base has a non-initial track.
        if !self.has_particle_update() {
            return current;
        }
        let mut result = sampled;
        // 1424ee010 appends initial tracks before update tracks to the same evaluator list.
        for animation in self
            .base_animations
            .iter()
            .filter(|a| a.time == 0)
            .chain(self.base_animations.iter().filter(|a| a.time != 0))
        {
            let time = match animation.time {
                0 | 1 => system_time,
                2 => age,
                3 if life == 0.0
                    && animation.op
                        == crate::xmap09_light_template::Xmap09LightAnimationOp::Mul =>
                {
                    age
                }
                3 => age / life,
                _ => unreachable!(),
            };
            result = animation.apply(result, time);
        }
        result
    }

    pub(crate) fn has_particle_update(&self) -> bool {
        self.base_animations
            .iter()
            .any(|animation| animation.time != 0)
    }

    pub(crate) fn initial(&self, time: f32) -> (f32, f32, bool) {
        let mut base = if !self.base_constant && !self.base_animations.iter().any(|a| a.time == 0) {
            0.0
        } else {
            self.base
        };
        let mut range = self.range;
        // Callback selection sees initial tracks; the shared list later also holds update tracks.
        // Birth has no age/life pointers, so only initial and system clocks enter that callback.
        if self.base_animations.iter().any(|a| a.time == 0) {
            for animation in self
                .base_animations
                .iter()
                .filter(|a| a.time == 0)
                .chain(self.base_animations.iter().filter(|a| a.time == 1))
            {
                base = animation.apply(base, time);
            }
        }
        if self.range_animations.iter().any(|a| a.time == 0) {
            for animation in self
                .range_animations
                .iter()
                .filter(|a| a.time == 0)
                .chain(self.range_animations.iter().filter(|a| a.time == 1))
            {
                range = animation.apply(range, time);
            }
        }
        (
            base,
            range,
            self.range_constant || self.range_animations.iter().any(|a| a.time == 0),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ParticleEmitterShape {
    Point,
    Sphere([VariedScalar; 3]),
    Box([VariedScalar; 3]),
}

impl ParticleEmitterShape {
    pub(crate) fn properties(&self) -> Option<&[VariedScalar; 3]> {
        match self {
            Self::Point => None,
            Self::Sphere(properties) | Self::Box(properties) => Some(properties),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParticleForceKind {
    Planar,
    Point,
    Vortex,
    Friction,
    Turbulence,
    Spin,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthoredParticleForce {
    pub name: String,
    pub kind: ParticleForceKind,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub yaw: f32,
    pub division: i32,
    pub local_force: bool,
    pub amount: VariedScalar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleSort {
    None,
    Depth,
    Distance,
    Age,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticleTexture {
    pub file: String,
    pub shader: String,
    pub columns: i32,
    pub rows: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthoredSubsystem {
    pub name: String,
    pub capacity: usize,
    pub delay: VariedScalar,
    pub duration: VariedScalar,
    pub emission: VariedScalar,
    pub life: VariedScalar,
    pub pulse_duration: VariedScalar,
    pub pulse_silence: VariedScalar,
    pub emitter_angles: [VariedScalar; 2],
    pub velocity_angles_and_speed: [VariedScalar; 3],
    pub local_space: bool,
    pub trail: bool,
    pub billboard: bool,
    pub sort: ParticleSort,
    pub invert: bool,
    pub texture: Option<ParticleTexture>,
    pub shape: ParticleEmitterShape,
    pub origin: [VariedScalar; 3],
    pub spawn_draw_columns: [VariedScalar; 12],
    pub children: Vec<AuthoredSubsystem>,
    pub storage_capacity: usize,
    pub forces: Vec<Arc<AuthoredParticleForce>>,
    pub force_division: i32,
}

#[derive(Debug)]
pub struct AdmittedParticleDefinition {
    pub(crate) lease: ContentAdmissionLease,
    pub(crate) name: String,
    pub(crate) registration: String,
    definition_path: String,
    pub(crate) scale: f32,
    pub(crate) subsystems: Arc<[AuthoredSubsystem]>,
    pub(crate) turbulence_divisions: Arc<[i32]>,
}

impl AdmittedParticleDefinition {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn registration(&self) -> &str {
        &self.registration
    }
    pub fn definition_path(&self) -> &str {
        &self.definition_path
    }
    pub fn authored_capacity(&self) -> usize {
        self.subsystems.iter().map(|s| s.storage_capacity).sum()
    }
    pub fn authored_scale_bits(&self) -> u32 {
        self.scale.to_bits()
    }
}

#[derive(Clone)]
struct SourceDefinition {
    value: Value,
    path: String,
}

#[derive(Clone, Debug)]
pub struct ParticleDrawCapacity {
    pub(crate) lease: ContentAdmissionLease,
    pub packets: usize,
    pub particles: usize,
}

pub struct ParticleDefinitions {
    lease: ContentAdmissionLease,
    registrations: BTreeMap<String, Vec<SourceDefinition>>,
    definitions: BTreeMap<String, Vec<SourceDefinition>>,
    admitted: BTreeMap<String, Arc<AdmittedParticleDefinition>>,
    compiled: BTreeMap<String, (Arc<[AuthoredSubsystem]>, Arc<[i32]>)>,
    draw_capacity: Option<ParticleDrawCapacity>,
}

impl ParticleDefinitions {
    pub fn load(resolver: &GameFileResolver) -> ParticleResult<Self> {
        let lease = resolver.content_admission_lease().ok_or_else(|| {
            ParticleInputRefusal::new(
                "",
                None,
                "content_session",
                ParticleRefusalReason::ContentAdmissionRequired,
            )
        })?;
        let mut result = Self {
            lease,
            registrations: BTreeMap::new(),
            definitions: BTreeMap::new(),
            admitted: BTreeMap::new(),
            compiled: BTreeMap::new(),
            draw_capacity: None,
        };
        for extension in ["gfx", "asset"] {
            let keys = resolver
                .require_complete_admitted_dir("gfx", Some(extension), true, "particle definitions")
                .map_err(|e| {
                    ParticleInputRefusal::new(
                        "",
                        None,
                        extension,
                        ParticleRefusalReason::AuthenticatedContentUnavailable(e.to_string()),
                    )
                })?;
            for key in keys {
                let path = key.normalized_game_relative_path.clone();
                let bytes = resolver.read_admitted_used_content(&key).map_err(|e| {
                    ParticleInputRefusal::new(
                        "",
                        None,
                        &path,
                        ParticleRefusalReason::AuthenticatedContentUnavailable(e.to_string()),
                    )
                })?;
                let text = paradox_text::decode_text_strict(&bytes).map_err(|_| {
                    ParticleInputRefusal::new(
                        "",
                        None,
                        &path,
                        ParticleRefusalReason::MalformedContent,
                    )
                })?;
                let mut root = if extension == "asset" {
                    paradox_text::parse_asset_str(&text).map(|(root, _)| root)
                } else {
                    paradox_text::parse_str_strict(&text)
                }
                .map_err(|_| {
                    ParticleInputRefusal::new(
                        "",
                        None,
                        &path,
                        ParticleRefusalReason::MalformedContent,
                    )
                })?;
                paradox_text::expand_file_scoped_variables(&mut root).map_err(|_| {
                    ParticleInputRefusal::new(
                        "",
                        None,
                        &path,
                        ParticleRefusalReason::MalformedContent,
                    )
                })?;
                collect_definitions(&root, &path, &mut result)?;
            }
        }
        Ok(result)
    }

    pub fn resolve(
        &mut self,
        registration: &str,
    ) -> ParticleResult<Arc<AdmittedParticleDefinition>> {
        if let Some(definition) = self.admitted.get(registration) {
            return Ok(Arc::clone(definition));
        }
        let rows = self.registrations.get(registration).ok_or_else(|| {
            ParticleInputRefusal::new(
                registration,
                None,
                "pdxparticle",
                ParticleRefusalReason::MissingRegistration,
            )
        })?;
        if rows.len() != 1 {
            return Err(ParticleInputRefusal::new(
                registration,
                None,
                "pdxparticle",
                ParticleRefusalReason::ConflictingRegistration,
            ));
        }
        let row = &rows[0];
        let binding = &row.value;
        let name = required_scalar(binding, "type", registration, None)?;
        // Clone resolution changes the type and scale; it cannot be inferred from a name match.
        if binding.first_value_ci("clone").is_some() {
            return Err(ParticleInputRefusal::new(
                registration,
                None,
                "clone",
                ParticleRefusalReason::UnsupportedAuthoredInput,
            ));
        }
        let scale = if binding.first_value_ci("scale").is_some() {
            required_scalar(binding, "scale", registration, None)?
                .parse::<f32>()
                .map_err(|_| {
                    ParticleInputRefusal::new(
                        registration,
                        None,
                        "scale",
                        ParticleRefusalReason::InvalidNumber,
                    )
                })?
        } else {
            f32::from_bits(0x3f80_0000)
        };
        if !scale.is_finite() {
            return Err(ParticleInputRefusal::new(
                registration,
                None,
                "scale",
                ParticleRefusalReason::InvalidNumber,
            ));
        }
        let definitions = self.definitions.get(name).ok_or_else(|| {
            ParticleInputRefusal::new(
                name,
                None,
                "particle",
                ParticleRefusalReason::MissingDefinition,
            )
        })?;
        if definitions.len() != 1 {
            return Err(ParticleInputRefusal::new(
                name,
                None,
                "particle",
                ParticleRefusalReason::ConflictingDefinition,
            ));
        }
        let definition = if let Some((subsystems, turbulence_divisions)) = self.compiled.get(name) {
            AdmittedParticleDefinition {
                lease: self.lease.clone(),
                name: name.into(),
                registration: registration.into(),
                definition_path: definitions[0].path.clone(),
                scale,
                subsystems: Arc::clone(subsystems),
                turbulence_divisions: Arc::clone(turbulence_divisions),
            }
        } else {
            let definition = compile_definition(
                &definitions[0],
                registration,
                name,
                scale,
                self.lease.clone(),
            )?;
            self.compiled.insert(
                name.into(),
                (
                    Arc::clone(&definition.subsystems),
                    Arc::clone(&definition.turbulence_divisions),
                ),
            );
            definition
        };
        let definition = Arc::new(definition);
        self.admitted
            .insert(registration.to_owned(), Arc::clone(&definition));
        Ok(definition)
    }

    pub fn draw_capacity(&mut self) -> ParticleResult<ParticleDrawCapacity> {
        if let Some(capacity) = &self.draw_capacity {
            return Ok(capacity.clone());
        }
        let mut capacity = ParticleDrawCapacity {
            lease: self.lease.clone(),
            packets: 0,
            particles: 0,
        };
        for (name, sources) in &self.definitions {
            if sources.len() != 1 {
                return Err(ParticleInputRefusal::new(
                    name,
                    None,
                    "particle",
                    ParticleRefusalReason::ConflictingDefinition,
                ));
            }
            if !self.compiled.contains_key(name) {
                let definition =
                    compile_definition(&sources[0], "", name, 1.0, self.lease.clone())?;
                self.compiled.insert(
                    name.clone(),
                    (definition.subsystems, definition.turbulence_divisions),
                );
            }
            let systems = &self.compiled[name].0;
            let mut packets = 0;
            let mut particles = 0;
            for system in systems.iter() {
                count_draw_capacity(name, system, 1, &mut packets, &mut particles)?;
            }
            capacity.packets = capacity.packets.max(packets);
            capacity.particles = capacity.particles.max(particles);
        }
        self.draw_capacity = Some(capacity.clone());
        Ok(capacity)
    }

    pub fn content_lease(&self) -> ContentAdmissionLease {
        self.lease.clone()
    }
}

fn count_draw_capacity(
    definition: &str,
    system: &AuthoredSubsystem,
    parent_capacity: usize,
    packets: &mut usize,
    particles: &mut usize,
) -> ParticleResult<()> {
    let fail = || {
        ParticleInputRefusal::new(
            definition,
            Some(&system.name),
            "global_draw_capacity",
            ParticleRefusalReason::CapacityExceeded,
        )
    };
    let instances = parent_capacity
        .checked_mul(system.capacity)
        .ok_or_else(fail)?;
    *packets = packets
        .checked_add(parent_capacity)
        .filter(|count| *count <= i32::MAX as usize)
        .ok_or_else(fail)?;
    *particles = particles
        .checked_add(instances)
        .filter(|count| *count <= i32::MAX as usize / 56)
        .ok_or_else(fail)?;
    for child in &system.children {
        count_draw_capacity(definition, child, instances, packets, particles)?;
    }
    Ok(())
}

fn collect_definitions(
    value: &Value,
    path: &str,
    output: &mut ParticleDefinitions,
) -> ParticleResult<()> {
    for entry in value.fields() {
        if entry.key_matches("pdxparticle") || entry.key_matches("particle") {
            let name = required_scalar(&entry.value, "name", "", None)?.to_owned();
            let rows = if entry.key_matches("pdxparticle") {
                &mut output.registrations
            } else {
                &mut output.definitions
            };
            rows.entry(name).or_default().push(SourceDefinition {
                value: join_comma_terms(entry.value.clone()),
                path: path.into(),
            });
        } else if entry.key_matches("objectTypes") {
            collect_definitions(&entry.value, path, output)?;
        }
    }
    Ok(())
}

fn required_scalar<'a>(
    value: &'a Value,
    key: &str,
    definition: &str,
    subsystem: Option<&str>,
) -> ParticleResult<&'a str> {
    let mut fields = value.fields().iter().filter(|entry| entry.key_matches(key));
    let field = fields.next().ok_or_else(|| {
        ParticleInputRefusal::new(
            definition,
            subsystem,
            key,
            ParticleRefusalReason::MissingAuthoredInput,
        )
    })?;
    if fields.next().is_some() {
        return Err(ParticleInputRefusal::new(
            definition,
            subsystem,
            key,
            ParticleRefusalReason::MalformedContent,
        ));
    }
    field.value.as_scalar().ok_or_else(|| {
        ParticleInputRefusal::new(
            definition,
            subsystem,
            key,
            ParticleRefusalReason::MalformedContent,
        )
    })
}

fn varied(
    value: &Value,
    key: &str,
    definition: &str,
    subsystem: &str,
    animations: &AnimationCatalog,
) -> ParticleResult<VariedScalar> {
    varied_aliases(value, &[key], definition, subsystem, animations)
}

fn varied_aliases(
    value: &Value,
    keys: &[&str],
    definition: &str,
    subsystem: &str,
    animations: &AnimationCatalog,
) -> ParticleResult<VariedScalar> {
    let mut result = VariedScalar::plain(0.0, 0.0);
    for field in value
        .fields()
        .iter()
        .filter(|entry| keys.iter().any(|key| entry.key_matches(key)))
    {
        let key = field.key.as_deref().unwrap();
        let fail = |reason| ParticleInputRefusal::new(definition, Some(subsystem), key, reason);
        let tokens = scalar_tokens(&field.value)
            .ok_or_else(|| fail(ParticleRefusalReason::MalformedContent))?;
        if tokens.len() > 2 {
            return Err(fail(ParticleRefusalReason::UnsupportedAuthoredInput));
        }
        for (index, token) in tokens.into_iter().enumerate() {
            let (number, constant, chain) = if index == 0 {
                (
                    &mut result.base,
                    &mut result.base_constant,
                    &mut result.base_animations,
                )
            } else {
                (
                    &mut result.range,
                    &mut result.range_constant,
                    &mut result.range_animations,
                )
            };
            for term in token.split(',') {
                if term.is_empty() {
                    return Err(fail(ParticleRefusalReason::MalformedContent));
                }
                if let Ok(numeric) = term.parse::<f32>() {
                    if !numeric.is_finite() {
                        return Err(fail(ParticleRefusalReason::InvalidNumber));
                    }
                    let value = engine_varied_number(term)
                        .ok_or_else(|| fail(ParticleRefusalReason::InvalidNumber))?;
                    *number = value;
                    *constant |= value != 0.0;
                } else {
                    chain.push(animations.resolve(term, definition, subsystem)?);
                }
            }
        }
    }
    Ok(result)
}

// 1424b8600 retains extra fractional digits and wraps its integer intermediates.
// The varied reader converts the resulting signed fixed-1e5 token to f32 before division.
fn engine_varied_number(text: &str) -> Option<f32> {
    let unsigned = text
        .strip_prefix('-')
        .or_else(|| text.strip_prefix('+'))
        .unwrap_or(text);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if (whole.is_empty() && fraction.is_empty())
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let signed_whole = if whole.is_empty() {
        0
    } else {
        let signed = if text.starts_with('-') {
            format!("-{whole}")
        } else {
            whole.to_owned()
        };
        signed.parse::<i64>().ok()?
    };
    let mut part = fraction.bytes().fold(0_i64, |value, c| {
        value.wrapping_mul(10).wrapping_add((c - b'0') as i64)
    });
    for _ in fraction.len()..5 {
        part = part.wrapping_mul(10);
    }
    if text.starts_with('-') {
        part = part.wrapping_neg();
    }
    let product = part.wrapping_mul(100_000);
    let part = if product <= i64::MIN + 1 {
        0xffff_ffff_i64
    } else {
        product.wrapping_mul(100_000) / 10_000_000_000
    };
    Some(signed_whole.wrapping_mul(100_000).wrapping_add(part) as f32 / 100_000.0)
}

fn join_comma_terms(value: Value) -> Value {
    let Value::Object(entries) = value else {
        return value;
    };
    let mut output: Vec<paradox_text::Entry> = Vec::with_capacity(entries.len());
    for mut entry in entries {
        entry.value = join_comma_terms(entry.value);
        if entry.key.is_none() {
            if let (Some(previous), Value::Scalar(paradox_text::Scalar::Bare(term))) =
                (output.last_mut(), &entry.value)
            {
                if let Value::Scalar(paradox_text::Scalar::Bare(prefix)) = &mut previous.value {
                    if prefix.ends_with(',') || term.starts_with(',') {
                        prefix.push_str(term);
                        continue;
                    }
                }
            }
        }
        output.push(entry);
    }
    Value::Object(output)
}

fn compile_definition(
    source: &SourceDefinition,
    registration: &str,
    name: &str,
    scale: f32,
    lease: ContentAdmissionLease,
) -> ParticleResult<AdmittedParticleDefinition> {
    let animations = AnimationCatalog {
        source: &source.value,
        cache: std::cell::RefCell::new(BTreeMap::new()),
    };
    let mut forces = BTreeMap::new();
    let mut turbulence_divisions = Vec::new();
    for field in source
        .value
        .fields()
        .iter()
        .filter(|f| f.key_matches("force"))
    {
        let force = required_scalar(&field.value, "name", name, None)?;
        if forces.contains_key(force) {
            return Err(ParticleInputRefusal::new(
                name,
                None,
                force,
                ParticleRefusalReason::ConflictingDefinition,
            ));
        }
        let compiled = compile_force(&field.value, name, &animations)?;
        if compiled.kind == ParticleForceKind::Turbulence {
            turbulence_divisions.push(compiled.division);
        }
        forces.insert(force.to_owned(), Arc::new(compiled));
    }
    let mut subsystems = Vec::new();
    let mut capacity = 0_usize;
    for field in source.value.fields() {
        if field.key_matches("name") || field.key_matches("animation") || field.key_matches("force")
        {
            continue;
        }
        if !field.key_matches("subsystem") {
            return Err(ParticleInputRefusal::new(
                name,
                None,
                field.key.as_deref().unwrap_or(&source.path),
                ParticleRefusalReason::UnsupportedAuthoredInput,
            ));
        }
        let subsystem = compile_subsystem(&field.value, name, &animations, &forces)?;
        capacity = capacity
            .checked_add(subsystem.storage_capacity)
            .ok_or_else(|| {
                ParticleInputRefusal::new(
                    name,
                    Some(&subsystem.name),
                    "max_amount",
                    ParticleRefusalReason::CapacityExceeded,
                )
            })?;
        subsystems.push(subsystem);
    }
    Ok(AdmittedParticleDefinition {
        lease,
        name: name.into(),
        registration: registration.into(),
        definition_path: source.path.clone(),
        scale,
        subsystems: subsystems.into(),
        turbulence_divisions: turbulence_divisions.into(),
    })
}

fn compile_texture(
    body: &Value,
    definition: &str,
    subsystem: &str,
) -> ParticleResult<ParticleTexture> {
    let fail =
        |key: &str, reason| ParticleInputRefusal::new(definition, Some(subsystem), key, reason);
    if !matches!(body, Value::Object(_)) {
        return Err(fail("texture", ParticleRefusalReason::MalformedContent));
    }
    let mut texture = ParticleTexture {
        file: String::new(),
        shader: String::new(),
        columns: 1,
        rows: 1,
    };
    for field in body.fields() {
        let key = field
            .key
            .as_deref()
            .ok_or_else(|| fail("texture", ParticleRefusalReason::MalformedContent))?;
        let value = field
            .value
            .as_scalar()
            .ok_or_else(|| fail(key, ParticleRefusalReason::MalformedContent))?;
        match key.to_ascii_lowercase().as_str() {
            "file" | "shader" => {
                let (output, limit) = if field.key_matches("file") {
                    (&mut texture.file, 255)
                } else {
                    (&mut texture.shader, 63)
                };
                let bytes = value.as_bytes();
                let end = bytes
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(bytes.len())
                    .min(limit);
                *output = std::str::from_utf8(&bytes[..end])
                    .map_err(|_| fail(key, ParticleRefusalReason::UnsupportedAuthoredInput))?
                    .into();
            }
            "x" => {
                texture.columns = value
                    .parse()
                    .map_err(|_| fail(key, ParticleRefusalReason::InvalidNumber))?
            }
            "y" => {
                texture.rows = value
                    .parse()
                    .map_err(|_| fail(key, ParticleRefusalReason::InvalidNumber))?
            }
            _ => return Err(fail(key, ParticleRefusalReason::UnsupportedAuthoredInput)),
        }
    }
    Ok(texture)
}

fn compile_force(
    body: &Value,
    definition: &str,
    animations: &AnimationCatalog,
) -> ParticleResult<AuthoredParticleForce> {
    let name = required_scalar(body, "name", definition, None)?;
    let fail = |key: &str, reason| ParticleInputRefusal::new(definition, Some(name), key, reason);
    let mut seen = std::collections::BTreeSet::new();
    for field in body.fields() {
        let key = field
            .key
            .as_deref()
            .ok_or_else(|| fail("force", ParticleRefusalReason::MalformedContent))?;
        if !seen.insert(key.to_ascii_lowercase()) {
            return Err(fail(key, ParticleRefusalReason::MalformedContent));
        }
        if matches!(
            key.to_ascii_lowercase().as_str(),
            "name" | "type" | "yaw" | "division" | "local_force"
        ) && field.value.as_scalar().is_none()
        {
            return Err(fail(key, ParticleRefusalReason::MalformedContent));
        }
        if !matches!(
            key.to_ascii_lowercase().as_str(),
            "name"
                | "type"
                | "position"
                | "direction"
                | "yaw"
                | "division"
                | "local_force"
                | "amount"
        ) {
            return Err(fail(key, ParticleRefusalReason::UnsupportedAuthoredInput));
        }
    }
    let kind = match body
        .first_scalar_ci("type")
        .unwrap_or("planar")
        .to_ascii_lowercase()
        .as_str()
    {
        "planar" => ParticleForceKind::Planar,
        "point" => ParticleForceKind::Point,
        "vortex" => ParticleForceKind::Vortex,
        "friction" => ParticleForceKind::Friction,
        "turbulence" => ParticleForceKind::Turbulence,
        "spin" => ParticleForceKind::Spin,
        _ => {
            return Err(fail(
                "type",
                ParticleRefusalReason::UnsupportedAuthoredInput,
            ))
        }
    };
    let vector = |key: &str, default: [f32; 3]| -> ParticleResult<[f32; 3]> {
        let Some(value) = body.first_value_ci(key) else {
            return Ok(default);
        };
        if !matches!(value, Value::Object(_)) {
            return Err(fail(key, ParticleRefusalReason::MalformedContent));
        }
        let mut vector = [0.0; 3];
        let tokens = scalar_tokens(value)
            .ok_or_else(|| fail(key, ParticleRefusalReason::MalformedContent))?;
        if tokens.len() > 3 {
            return Err(fail(key, ParticleRefusalReason::MalformedContent));
        }
        for (output, token) in vector.iter_mut().zip(tokens) {
            *output = token
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| fail(key, ParticleRefusalReason::InvalidNumber))?;
        }
        Ok(vector)
    };
    if let Some(value) = body.first_value_ci("amount") {
        if scalar_tokens(value).is_none_or(|tokens| tokens.len() != 1) {
            return Err(fail("amount", ParticleRefusalReason::MalformedContent));
        }
    }
    Ok(AuthoredParticleForce {
        name: name.into(),
        kind,
        position: vector("position", [0.0; 3])?,
        direction: vector("direction", [0.0, 1.0, 0.0])?,
        yaw: body.first_scalar_ci("yaw").map_or(Ok(0.0), |value| {
            value
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| fail("yaw", ParticleRefusalReason::InvalidNumber))
        })?,
        division: body.first_scalar_ci("division").map_or(Ok(16), |value| {
            value
                .parse::<i32>()
                .map_err(|_| fail("division", ParticleRefusalReason::InvalidNumber))
        })?,
        local_force: match body.first_scalar_ci("local_force") {
            None | Some("yes") => true,
            Some("no") => false,
            _ => return Err(fail("local_force", ParticleRefusalReason::MalformedContent)),
        },
        amount: varied(body, "amount", definition, name, animations)?,
    })
}

fn compile_subsystem(
    body: &Value,
    name: &str,
    animations: &AnimationCatalog,
    forces: &BTreeMap<String, Arc<AuthoredParticleForce>>,
) -> ParticleResult<AuthoredSubsystem> {
    let subsystem = required_scalar(body, "name", name, None)?;
    let fail =
        |input: &str, reason| ParticleInputRefusal::new(name, Some(subsystem), input, reason);
    let count = required_scalar(body, "max_amount", name, Some(subsystem))?
        .parse::<i32>()
        .map_err(|_| fail("max_amount", ParticleRefusalReason::InvalidNumber))?;
    let count = usize::try_from(count)
        .map_err(|_| fail("max_amount", ParticleRefusalReason::InvalidNumber))?;
    let mut seen = std::collections::BTreeSet::new();
    for field in body.fields() {
        let key = field
            .key
            .as_deref()
            .ok_or_else(|| fail("", ParticleRefusalReason::MalformedContent))?;
        if !field.key_matches("childsystem") && !seen.insert(key.to_ascii_lowercase()) {
            return Err(fail(key, ParticleRefusalReason::MalformedContent));
        }
        match key.to_ascii_lowercase().as_str() {
            "force"
            | "size"
            | "mass"
            | "color"
            | "position"
            | "particle_yaw"
            | "particle_pitch"
            | "particle_roll"
            | "roll"
            | "rotation_speed"
            | "rotation_speed_yaw"
            | "rotation_speed_pitch"
            | "rotation_speed_roll"
            | "childsystem"
            | "name"
            | "max_amount"
            | "start"
            | "duration"
            | "emission"
            | "life"
            | "texture"
            | "emission_pulse_duration"
            | "emission_pulse_silence"
            | "emitter_yaw"
            | "emitter_pitch"
            | "velocity_yaw"
            | "velocity_pitch"
            | "velocity"
            | "sphere_emitter_radius"
            | "sphere_emitter_yaw"
            | "sphere_emitter_pitch"
            | "box_emitter_x"
            | "box_emitter_y"
            | "box_emitter_z" => {}
            "emitter_type"
                if matches!(field.value.as_scalar(), Some("point" | "sphere" | "box")) => {}
            "slave_particles" if field.value.as_scalar().is_some() => {}
            "sort" if field.value.as_scalar().is_some() => {}
            "local_space" | "trail" | "billboard" | "invert"
                if matches!(field.value.as_scalar(), Some("yes" | "no")) => {}
            _ => return Err(fail(key, ParticleRefusalReason::UnsupportedAuthoredInput)),
        }
    }
    let mut selected_forces = Vec::new();
    if let Some(value) = body.first_value_ci("force") {
        for token in scalar_tokens(value)
            .ok_or_else(|| fail("force", ParticleRefusalReason::MalformedContent))?
        {
            for force in token.split(',') {
                let Some(force) = forces.get(force) else {
                    return Err(fail(
                        &format!("force:{force}"),
                        ParticleRefusalReason::MissingAuthoredInput,
                    ));
                };
                selected_forces.push(Arc::clone(force));
            }
        }
    }
    let shape = match body.first_scalar_ci("emitter_type") {
        None | Some("point") => ParticleEmitterShape::Point,
        Some(kind @ ("sphere" | "box")) => {
            let properties = [
                varied_aliases(
                    body,
                    &["sphere_emitter_radius", "box_emitter_x"],
                    name,
                    subsystem,
                    animations,
                )?,
                varied_aliases(
                    body,
                    &["sphere_emitter_yaw", "box_emitter_y"],
                    name,
                    subsystem,
                    animations,
                )?,
                varied_aliases(
                    body,
                    &["sphere_emitter_pitch", "box_emitter_z"],
                    name,
                    subsystem,
                    animations,
                )?,
            ];
            if kind == "sphere" {
                ParticleEmitterShape::Sphere(properties)
            } else {
                ParticleEmitterShape::Box(properties)
            }
        }
        _ => unreachable!(),
    };
    let origin = vector_tracks(body, "position", false, name, subsystem, animations)?;
    let colors = vector_tracks(body, "color", true, name, subsystem, animations)?;
    let mut spawn_draw_columns = std::array::from_fn(|_| VariedScalar::plain(0.0, 0.0));
    for (index, key) in [
        (0, "size"),
        (1, "particle_yaw"),
        (2, "particle_pitch"),
        (4, "rotation_speed_yaw"),
        (5, "rotation_speed_pitch"),
        (7, "mass"),
    ] {
        spawn_draw_columns[index] = varied(body, key, name, subsystem, animations)?;
    }
    spawn_draw_columns[3] = varied_aliases(
        body,
        &["particle_roll", "roll"],
        name,
        subsystem,
        animations,
    )?;
    spawn_draw_columns[6] = varied_aliases(
        body,
        &["rotation_speed_roll", "rotation_speed"],
        name,
        subsystem,
        animations,
    )?;
    for (destination, color) in spawn_draw_columns[8..].iter_mut().zip(colors) {
        *destination = color;
    }
    let mut children = Vec::new();
    let mut child_capacity = 0_usize;
    for child in body
        .fields()
        .iter()
        .filter(|field| field.key_matches("childsystem"))
    {
        let child = compile_subsystem(&child.value, name, animations, forces)?;
        child_capacity = child_capacity
            .checked_add(child.storage_capacity)
            .ok_or_else(|| fail("childsystem", ParticleRefusalReason::CapacityExceeded))?;
        children.push(child);
    }
    let storage_capacity = child_capacity
        .checked_add(1)
        .and_then(|n| n.checked_mul(count))
        .ok_or_else(|| fail("childsystem", ParticleRefusalReason::CapacityExceeded))?;
    Ok(AuthoredSubsystem {
        name: subsystem.into(),
        capacity: count,
        delay: varied(body, "start", name, subsystem, animations)?,
        duration: varied(body, "duration", name, subsystem, animations)?,
        emission: varied(body, "emission", name, subsystem, animations)?,
        life: varied(body, "life", name, subsystem, animations)?,
        pulse_duration: varied(body, "emission_pulse_duration", name, subsystem, animations)?,
        pulse_silence: varied(body, "emission_pulse_silence", name, subsystem, animations)?,
        emitter_angles: [
            varied(body, "emitter_yaw", name, subsystem, animations)?,
            varied(body, "emitter_pitch", name, subsystem, animations)?,
        ],
        velocity_angles_and_speed: [
            varied(body, "velocity_pitch", name, subsystem, animations)?,
            varied(body, "velocity_yaw", name, subsystem, animations)?,
            varied(body, "velocity", name, subsystem, animations)?,
        ],
        local_space: body.first_scalar_ci("local_space") == Some("yes"),
        trail: body.first_scalar_ci("trail") == Some("yes"),
        billboard: body
            .first_scalar_ci("billboard")
            .map_or(body.first_scalar_ci("trail") != Some("yes"), |value| {
                value == "yes"
            }),
        sort: match body
            .first_scalar_ci("sort")
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None => ParticleSort::None,
            Some("depth") => ParticleSort::Depth,
            Some("distance") => ParticleSort::Distance,
            Some("age") => ParticleSort::Age,
            _ => {
                return Err(fail(
                    "sort",
                    ParticleRefusalReason::UnsupportedAuthoredInput,
                ))
            }
        },
        invert: body.first_scalar_ci("invert") == Some("yes"),
        texture: body
            .first_value_ci("texture")
            .map(|value| compile_texture(value, name, subsystem))
            .transpose()?,
        shape,
        origin: [origin[0].clone(), origin[1].clone(), origin[2].clone()],
        spawn_draw_columns,
        children,
        storage_capacity,
        forces: selected_forces,
        force_division: body
            .first_scalar_ci("slave_particles")
            .map_or(Ok(0), |value| {
                value
                    .parse::<i32>()
                    .map_err(|_| fail("slave_particles", ParticleRefusalReason::InvalidNumber))
            })?,
    })
}

fn vector_tracks(
    body: &Value,
    key: &str,
    alpha: bool,
    name: &str,
    subsystem: &str,
    animations: &AnimationCatalog,
) -> ParticleResult<Vec<VariedScalar>> {
    let mut output = vec![VariedScalar::plain(0.0, 0.0); if alpha { 4 } else { 3 }];
    let Some(value) = body.first_value_ci(key) else {
        return Ok(output);
    };
    let aliases: &[&[&str]] = &[&["z", "b"], &["y", "g"], &["x", "r"], &["w", "a", "alpha"]];
    if !matches!(value, Value::Object(_)) {
        return Err(ParticleInputRefusal::new(
            name,
            Some(subsystem),
            key,
            ParticleRefusalReason::MalformedContent,
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for field in value.fields() {
        let field_key = field.key.as_deref().unwrap_or("").to_ascii_lowercase();
        if !seen.insert(field_key)
            || !aliases
                .iter()
                .flat_map(|keys| keys.iter())
                .any(|key| field.key_matches(key))
        {
            return Err(ParticleInputRefusal::new(
                name,
                Some(subsystem),
                key,
                ParticleRefusalReason::UnsupportedAuthoredInput,
            ));
        }
    }
    for (index, names) in aliases.iter().take(output.len()).enumerate() {
        output[index] = varied_aliases(value, names, name, subsystem, animations)?;
    }
    Ok(output)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParticleAnimation {
    time: u8,
    start: f32,
    end: f32,
    repeat: bool,
    op: crate::xmap09_light_template::Xmap09LightAnimationOp,
    samples: [f32; 64],
}
impl ParticleAnimation {
    fn apply(&self, base: f32, time: f32) -> f32 {
        use crate::xmap09_light_template::Xmap09LightAnimationOp;
        let time = if self.repeat { time % self.end } else { time };
        let index = ((time - self.start) * 64.0) / (self.end - self.start);
        let sample = if index < 0.0 {
            self.samples[0]
        } else if index < 63.0 {
            let whole = index as usize;
            (self.samples[whole + 1] - self.samples[whole]) * (index - whole as f32)
                + self.samples[whole]
        } else {
            self.samples[63]
        };
        match self.op {
            Xmap09LightAnimationOp::Add => sample + base,
            Xmap09LightAnimationOp::Mul => sample * base,
            Xmap09LightAnimationOp::Abs => sample,
        }
    }
}

struct AnimationCatalog<'a> {
    source: &'a Value,
    cache: std::cell::RefCell<BTreeMap<String, Arc<ParticleAnimation>>>,
}
impl AnimationCatalog<'_> {
    fn resolve(
        &self,
        key: &str,
        definition: &str,
        subsystem: &str,
    ) -> ParticleResult<Arc<ParticleAnimation>> {
        if let Some(value) = self.cache.borrow().get(key) {
            return Ok(Arc::clone(value));
        }
        let fail = |reason| {
            ParticleInputRefusal::new(
                definition,
                Some(subsystem),
                &format!("animation:{key}"),
                reason,
            )
        };
        let mut matches =
            self.source.fields().iter().filter(|f| {
                f.key_matches("animation") && f.value.first_scalar_ci("name") == Some(key)
            });
        let value = &matches
            .next()
            .ok_or_else(|| fail(ParticleRefusalReason::MissingAuthoredInput))?
            .value;
        if matches.next().is_some() {
            return Err(fail(ParticleRefusalReason::ConflictingDefinition));
        }
        let mut seen = std::collections::BTreeSet::new();
        for field in value.fields() {
            let field_key = field
                .key
                .as_deref()
                .ok_or_else(|| fail(ParticleRefusalReason::MalformedContent))?
                .to_ascii_lowercase();
            if !seen.insert(field_key.clone()) {
                return Err(fail(ParticleRefusalReason::MalformedContent));
            }
            if !matches!(
                field_key.as_str(),
                "name"
                    | "time"
                    | "start"
                    | "duration"
                    | "repeat"
                    | "op"
                    | "minvalue"
                    | "maxvalue"
                    | "curve"
            ) {
                return Err(fail(ParticleRefusalReason::UnsupportedAuthoredInput));
            }
        }
        let number = |field| -> ParticleResult<f32> {
            let Some(field) = value.first_value_ci(field) else {
                return Ok(0.0);
            };
            let number = field
                .as_scalar()
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .ok_or_else(|| fail(ParticleRefusalReason::InvalidNumber))?;
            Ok(number)
        };
        use crate::xmap09_light_template::{Xmap09LightAnimation, Xmap09LightAnimationOp};
        let op = match value.first_scalar_ci("op") {
            None => Xmap09LightAnimationOp::Add,
            Some(op) => Xmap09LightAnimationOp::parse(&op.to_ascii_uppercase())
                .ok_or_else(|| fail(ParticleRefusalReason::UnsupportedAuthoredInput))?,
        };
        let repeat = match value.first_scalar_ci("repeat") {
            None | Some("no") => false,
            Some("yes") => true,
            _ => return Err(fail(ParticleRefusalReason::MalformedContent)),
        };
        let time = match value.first_scalar_ci("time") {
            Some("system") => 1,
            Some("life_abs") => 2,
            Some("life") => 3,
            _ => 0,
        };
        let mut curve = Vec::new();
        if let Some(value) = value.first_value_ci("curve") {
            let tokens = scalar_tokens(value)
                .ok_or_else(|| fail(ParticleRefusalReason::MalformedContent))?;
            if tokens.len() % 2 != 0 {
                return Err(fail(ParticleRefusalReason::MalformedContent));
            }
            for pair in tokens.chunks_exact(2) {
                curve.push([
                    pair[0]
                        .parse()
                        .map_err(|_| fail(ParticleRefusalReason::InvalidNumber))?,
                    pair[1]
                        .parse()
                        .map_err(|_| fail(ParticleRefusalReason::InvalidNumber))?,
                ]);
            }
        }
        let start = number("start")?;
        let duration = number("duration")?;
        let duration = if duration <= 0.0 { 1.0 } else { duration };
        let authoring = Xmap09LightAnimation {
            name: key.into(),
            start,
            duration,
            repeat,
            op,
            min_value: number("minValue")?,
            max_value: number("maxValue")?,
            curve,
        };
        let mut samples = [0.0; 64];
        for (index, sample) in samples.iter_mut().enumerate() {
            *sample = authoring
                .sample_curve_for_table(index as f32 / 63.0)
                .ok_or_else(|| fail(ParticleRefusalReason::UnsupportedAuthoredInput))?;
        }
        let end = start + duration;
        if !end.is_finite() || end == start || (repeat && end == 0.0) {
            return Err(fail(ParticleRefusalReason::InvalidNumber));
        }
        let animation = Arc::new(ParticleAnimation {
            time,
            start,
            end,
            repeat,
            op,
            samples,
        });
        self.cache
            .borrow_mut()
            .insert(key.into(), Arc::clone(&animation));
        Ok(animation)
    }
}

fn scalar_tokens(value: &Value) -> Option<Vec<&str>> {
    match value {
        Value::Scalar(_) => Some(vec![value.as_scalar()?]),
        Value::Object(entries) => entries
            .iter()
            .map(|e| {
                if e.key.is_none() {
                    e.value.as_scalar()
                } else {
                    None
                }
            })
            .collect(),
    }
}


#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EntityAttachmentVariant {
    pub name: String,
    pub weight: i32,
    pub children: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EntityAttachmentGroup {
    pub name: String,
    pub variants: Vec<EntityAttachmentVariant>,
}

impl EntityAttachmentGroup {
    pub(crate) fn select(&self, draw: impl FnOnce() -> u32) -> crate::Result<Option<usize>> {
        let total = self
            .variants
            .iter()
            .filter(|v| v.weight > 0)
            .fold(0u32, |sum, v| sum.wrapping_add(v.weight as u32));
        if total == 0 {
            return Ok(None);
        }
        let draw = draw();
        // 1424db730 sign-extends the total before its unsigned i64-to-double conversion.
        let total_double = (i64::from(total as i32) as u64) as f64;
        let mut target = f64::from(draw) * f64::from_bits(0x3e00_0000_0000_0000) * total_double;
        if target >= f64::from_bits(0x43e0_0000_0000_0000) {
            target -= f64::from_bits(0x43e0_0000_0000_0000);
        }
        let target = target as i64 as i32;
        let mut cumulative = 0i32;
        for (index, variant) in self.variants.iter().enumerate() {
            if variant.weight > 0 {
                cumulative = cumulative.wrapping_add(variant.weight);
                if target < cumulative {
                    return Ok(Some(index));
                }
            }
        }
        Err("entity attachment weighted selection reached the native no-index error".into())
    }

    fn replaces(&self, previous: &Self) -> bool {
        if self.name.is_empty() && previous.name.is_empty() {
            self.variants
                .first()
                .zip(previous.variants.first())
                .is_some_and(|(a, b)| !a.name.is_empty() && a.name == b.name)
        } else {
            self.name == previous.name
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EntityAttachments {
    pub groups: Vec<EntityAttachmentGroup>,
    pub state_propagation: Vec<(String, Vec<(u32, u32)>)>,
    pub state_weights: Vec<(String, i32)>,
    pub parent_state_override: Option<String>,
}

// 1424db8f0/1424db310 sum signed-positive weights in 64 bits; zero total uses
// the already-consumed draw for a uniform index, unlike attachment groups.
pub(crate) fn native_state_index(weights: &[(String, i32)], draw: u32) -> crate::Result<usize> {
    let count = i32::try_from(weights.len()).map_err(|_| "entity state count exceeds native i32")?;
    if count == 0 || draw > 0x7fff_ffff {
        return Err("entity state random inputs are invalid".into());
    }
    let unit = f64::from(draw) * f64::from_bits(0x3e00_0000_0000_0000);
    let total = weights.iter().filter(|(_, weight)| *weight > 0)
        .fold(0u64, |sum, (_, weight)| sum + *weight as u64);
    if total == 0 {
        return Ok((unit * f64::from(count)) as usize);
    }
    let target = (unit * total as f64) as u64;
    let mut cumulative = 0u64;
    for (index, (_, weight)) in weights.iter().enumerate() {
        if *weight > 0 {
            cumulative += *weight as u64;
            if target < cumulative { return Ok(index); }
        }
    }
    Err("entity state weighted selection reached the native no-index error".into())
}

impl EntityAttachments {
    pub(crate) fn enter_state(
        &self,
        pose: &crate::BuildingEntityPose,
        requested: &str,
        random: &mut crate::particle_runtime::ViewerParticleSeed,
    ) -> crate::Result<(Option<usize>, f32)> {
        self.enter_state_with_offset_range(pose, requested, None, random)
    }

    pub(crate) fn enter_state_with_offset_range(
        &self,
        pose: &crate::BuildingEntityPose,
        requested: &str,
        offset_range: Option<f32>,
        random: &mut crate::particle_runtime::ViewerParticleSeed,
    ) -> crate::Result<(Option<usize>, f32)> {
        if offset_range.is_some_and(|range| !range.is_finite()) {
            return Err("entity state requested offset range is nonfinite".into());
        }
        let hashes = pose.states.iter().map(|state| entity_token_hash(&state.name))
            .collect::<crate::Result<Vec<_>>>()?;
        let requested_hash = entity_token_hash(requested)?;
        let first = hashes.iter().position(|hash| *hash == requested_hash);
        let index = if let Some(first) = first {
            // The native stable hash sort makes every equal-key declaration contiguous.
            let indices = hashes.iter().enumerate().filter_map(|(index, hash)|
                (*hash == hashes[first]).then_some(index)).collect::<Vec<_>>();
            let index = if indices.len() == 1 { first } else {
                let weights = indices.iter().map(|index| {
                    self.state_weights.get(*index)
                        .filter(|(name, _)| *name == pose.states[*index].name)
                        .cloned().ok_or_else(|| "entity state weights and shared pose ordinals disagree".into())
                }).collect::<crate::Result<Vec<_>>>()?;
                indices[native_state_index(&weights, random.draw())?]
            };
            Some(index)
        } else { None };
        let Some(selected) = index else { return Ok((None, 0.0)); };
        let [mut start, authored_range] = pose.states[selected].time_offset
            .unwrap_or([0.0, 0.0]);
        // 1422755a0: bit 4 replaces the range only when the requested range is positive.
        let range = offset_range.filter(|range| *range > 0.0).unwrap_or(authored_range);
        if !start.is_finite() || !range.is_finite() {
            return Err("entity state time offset is nonfinite".into());
        }
        if range > 0.0 {
            start += random.draw() as f32 * f32::from_bits(0x30000000) * range;
        }
        if !start.is_finite() {
            return Err("entity state time offset arithmetic overflow".into());
        }
        Ok((index, start))
    }
}

#[derive(Clone, Debug, PartialEq)]
struct EntityAttachmentSource {
    base: Option<String>,
    own: EntityAttachments,
}

#[derive(Default)]
pub(crate) struct EntityAttachmentCatalog {
    sources: BTreeMap<String, std::result::Result<EntityAttachmentSource, String>>,
}

fn attachment_name(value: &str) -> String {
    value.chars().take_while(|c| *c != '\0').take(255).collect()
}

pub(crate) fn entity_token_hash(value: &str) -> crate::Result<u32> {
    let bytes = value
        .chars()
        .take_while(|c| *c != '\0')
        .map(|c| u8::try_from(u32::from(c)))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| "entity token is outside the authenticated byte encoding")?;
    Ok(crate::production_features::xmap20_derive::fnv1a32(&bytes))
}

impl EntityAttachmentCatalog {
    pub(crate) fn insert(&mut self, name: &str, entity: &Value) {
        self.sources
            .insert(name.into(), Self::read(entity).map_err(|e| e.to_string()));
    }

    fn read(entity: &Value) -> crate::Result<EntityAttachmentSource> {
        let mut own = EntityAttachments::default();
        let mut base = None;
        for field in entity.fields() {
            if field.key_matches("clone") {
                let name = attachment_name(
                    field
                        .value
                        .as_scalar()
                        .ok_or("attachment clone is not scalar")?,
                );
                base = (!name.is_empty()).then_some(name);
            } else if field.key_matches("get_state_from_parent") {
                own.parent_state_override = Some(
                    field
                        .value
                        .as_scalar()
                        .ok_or("attachment parent-state override is not scalar")?
                        .into(),
                );
            } else if field.key_matches("attach") {
                own.groups.push(EntityAttachmentGroup {
                    name: String::new(),
                    variants: vec![Self::variant(&field.value)?],
                });
            } else if field.key_matches("group") {
                if !matches!(&field.value, Value::Object(_)) {
                    return Err("attachment group is not an object".into());
                }
                let mut group = EntityAttachmentGroup {
                    name: String::new(),
                    variants: Vec::new(),
                };
                for entry in field.value.fields() {
                    if entry.key_matches("name") {
                        group.name = attachment_name(
                            entry
                                .value
                                .as_scalar()
                                .ok_or("attachment group name is not scalar")?,
                        );
                    } else if entry.key_matches("attach") {
                        group.variants.push(Self::variant(&entry.value)?);
                    } else {
                        return Err("attachment group field has no admitted reader branch".into());
                    }
                }
                own.groups.push(group);
            } else if field.key_matches("state") {
                let mut name = None;
                let mut mappings = Vec::new();
                let mut weight = 0;
                for entry in field.value.fields() {
                    if entry.key_matches("name") {
                        name = Some(attachment_name(
                            entry
                                .value
                                .as_scalar()
                                .ok_or("attachment state name is not scalar")?,
                        ));
                    } else if entry.key_matches("chance") {
                        let raw = crate::authored_fixed::parse_fixed5(entry.value.as_scalar()
                            .ok_or("entity state chance is not scalar")?)
                            .ok_or("entity state chance is not a native fixed token")?;
                        weight = if raw > 0 { raw as i32 } else { 0 };
                    } else if !entry.key_matches("propagate_state") {
                        continue;
                    } else if let Some(target) = entry.value.as_scalar() {
                        mappings.push((0, entity_token_hash(target)?));
                    } else {
                        for mapping in entry.value.fields() {
                            let key = mapping
                                .key
                                .as_deref()
                                .ok_or("propagate_state has no source name")?;
                            let target = mapping
                                .value
                                .as_scalar()
                                .ok_or("propagate_state target is not scalar")?;
                            mappings.push((entity_token_hash(key)?, entity_token_hash(target)?));
                        }
                    }
                }
                let name = name.ok_or("attachment state has no name")?;
                own.state_weights.push((name.clone(), weight));
                own.state_propagation.push((name, mappings));
            }
        }
        Ok(EntityAttachmentSource { base, own })
    }

    fn variant(body: &Value) -> crate::Result<EntityAttachmentVariant> {
        let mut out = EntityAttachmentVariant {
            name: String::new(),
            weight: 100_000,
            children: Vec::new(),
        };
        if !matches!(body, Value::Object(_)) {
            return Err("attachment variant is not an object".into());
        }
        for entry in body.fields() {
            let key = entry
                .key
                .as_deref()
                .ok_or("attachment variant has no field name")?;
            let value = entry
                .value
                .as_scalar()
                .ok_or("attachment variant field is not scalar")?;
            if entry.key_matches("name") {
                out.name = attachment_name(value);
            } else if entry.key_matches("chance") {
                let raw = crate::authored_fixed::parse_fixed5(value)
                    .ok_or("attachment chance is not a native fixed token")?;
                out.weight = if raw > 0 { (raw as i32).max(0) } else { 0 };
            } else {
                out.children.push((
                    key.split('\0').next().unwrap().into(),
                    value.split('\0').next().unwrap().into(),
                ));
            }
        }
        Ok(out)
    }

    pub(crate) fn resolve(&self, name: &str) -> crate::Result<EntityAttachments> {
        self.resolve_inner(name, &mut Vec::new())
    }

    fn resolve_inner(
        &self,
        name: &str,
        ancestry: &mut Vec<String>,
    ) -> crate::Result<EntityAttachments> {
        let source = self
            .sources
            .get(name)
            .ok_or_else(|| format!("entity {name} attachment source is unavailable"))?;
        if ancestry.iter().any(|value| value == name) {
            return Err(
                format!("recursive entity attachment clone: {ancestry:?} -> {name}").into(),
            );
        }
        let source = source
            .as_ref()
            .map_err(|e| format!("entity {name} attachments: {e}"))?;
        let Some(base) = &source.base else {
            return Ok(source.own.clone());
        };
        if !self.sources.contains_key(base) {
            return Err(
                format!("entity {name} attachment clone base {base} is unavailable").into(),
            );
        }
        ancestry.push(name.into());
        let mut resolved = self.resolve_inner(base, ancestry)?;
        ancestry.pop();
        for group in &source.own.groups {
            if let Some(index) = resolved
                .groups
                .iter()
                .position(|previous| group.replaces(previous))
            {
                resolved.groups[index] = group.clone();
            } else {
                resolved.groups.push(group.clone());
            }
        }
        resolved.state_weights.retain(|(name, _)| {
            !source.own.state_weights.iter().any(|(own, _)| own == name)
        });
        resolved.state_weights.extend(source.own.state_weights.iter().cloned());
        resolved.state_propagation.retain(|(name, _)| {
            !source
                .own
                .state_propagation
                .iter()
                .any(|(own, _)| own == name)
        });
        resolved
            .state_propagation
            .extend(source.own.state_propagation.iter().cloned());
        if source.own.parent_state_override.is_some() {
            resolved.parent_state_override = source.own.parent_state_override.clone();
        }
        Ok(resolved)
    }
}

#[cfg(test)]
mod entity_attachment_tests {
    use super::*;

    fn catalog(text: &str) -> EntityAttachmentCatalog {
        let (document, _) = portable_content_manifest::paradox_text::parse_asset_str(text).unwrap();
        let mut catalog = EntityAttachmentCatalog::default();
        for entity in document.values_ci("entity") {
            catalog.insert(entity.first_scalar_ci("name").unwrap(), entity);
        }
        catalog
    }

    #[test]
    fn clone_replaces_named_group_or_first_named_variant_and_keeps_native_ordinal() {
        let catalog = catalog(
            r#"
entity = { name = base
 group = { name = gear attach = { name = old root = old_entity } }
 attach = { name = smoke smoke_node = smoke_entity }
 attach = { root = unnamed_entity }
 state = { name = idle propagate_state = all_idle }
 state = { name = attack propagate_state = { smoke = firing } }
}
entity = { name = derived clone = base
 attach = { name = smoke chance = 0.5 other_node = replacement }
 group = { name = gear attach = { name = new root = new_entity } }
 attach = { root = additional_unnamed }
 state = { name = idle propagate_state = { smoke = quiet } }
}
entity = { name = chained clone = derived }
"#,
        );
        let base = catalog.resolve("base").unwrap();
        let derived = catalog.resolve("derived").unwrap();
        assert_eq!(base.groups.len(), 3);
        assert_eq!(derived.groups.len(), 4);
        assert_eq!(derived.groups[0].name, "gear");
        assert_eq!(derived.groups[0].variants[0].name, "new");
        assert_eq!(
            derived.groups[1].variants[0].children,
            [("other_node".into(), "replacement".into())]
        );
        assert_eq!(derived.groups[1].variants[0].weight, 50_000);
        assert_eq!(derived.groups[2], base.groups[2]);
        assert_eq!(
            derived.groups[3].variants[0].children[0].1,
            "additional_unnamed"
        );
        assert_eq!(
            derived
                .state_propagation
                .iter()
                .map(|s| s.0.as_str())
                .collect::<Vec<_>>(),
            ["attack", "idle"]
        );
        assert_eq!(
            derived.state_propagation[1].1,
            [(
                entity_token_hash("smoke").unwrap(),
                entity_token_hash("quiet").unwrap()
            )]
        );
        assert_eq!(catalog.resolve("chained").unwrap(), derived);
        assert_eq!(
            base.state_propagation[0].1,
            [(0, entity_token_hash("all_idle").unwrap())]
        );
    }

    #[test]
    fn repeated_entity_and_state_fields_overwrite_in_native_reader_order() {
        let catalog = catalog(
            r#"
entity = { name = base attach = { root = inherited } get_state_from_parent = no }
entity = { name = derived clone = missing clone = base
 get_state_from_parent = no get_state_from_parent = yes
 state = { name = old propagate_state = first name = final propagate_state = second }
}
entity = { name = direct clone = base clone = "" attach = { root = own } }
"#,
        );
        let derived = catalog.resolve("derived").unwrap();
        assert_eq!(derived.groups[0].variants[0].children[0].1, "inherited");
        assert_eq!(derived.parent_state_override.as_deref(), Some("yes"));
        assert_eq!(
            derived.state_propagation,
            [(
                "final".into(),
                vec![
                    (0, entity_token_hash("first").unwrap()),
                    (0, entity_token_hash("second").unwrap()),
                ]
            )]
        );
        let direct = catalog.resolve("direct").unwrap();
        assert_eq!(direct.groups.len(), 1);
        assert_eq!(direct.groups[0].variants[0].children[0].1, "own");
        assert_eq!(direct.parent_state_override, None);
    }

    #[test]
    fn integer_attachment_weights_preserve_reader_cast_clamp_and_fraction_law() {
        let catalog = catalog(
            r#"entity = { name = root group = {
 attach = { chance = 1.234567 root = a }
 attach = { chance = -0.2 root = b }
 attach = { chance = 21474.83648 root = c }
 attach = { chance = 42949.67297 root = d }
 attach = { root = e }
 attach = { chance = 010 root = octal }
 attach = { chance = 0x10 root = hex }
 attach = { chance = 9223372036854775808 root = saturated }
} }"#,
        );
        let program = catalog.resolve("root").unwrap();
        assert_eq!(
            program.groups[0]
                .variants
                .iter()
                .map(|v| v.weight)
                .collect::<Vec<_>>(),
            [123_456, 0, 0, 1, 100_000, 800_000, 1_600_000, 0]
        );
    }

    #[test]
    fn group_selection_draws_once_even_for_one_variant_but_not_zero_total() {
        let mut seed = crate::particle_runtime::ViewerParticleSeed {
            seed: 7,
            counter: 9,
        };
        let empty = EntityAttachmentGroup {
            name: String::new(),
            variants: Vec::new(),
        };
        assert_eq!(empty.select(|| seed.draw()).unwrap(), None);
        assert_eq!(seed.counter, 9);
        let group = EntityAttachmentGroup {
            name: String::new(),
            variants: vec![EntityAttachmentVariant {
                name: "only".into(),
                weight: 100_000,
                children: Vec::new(),
            }],
        };
        assert_eq!(group.select(|| seed.draw()).unwrap(), Some(0));
        assert_eq!(seed.counter, 10);
        let wrapped_zero = EntityAttachmentGroup {
            variants: vec![
                EntityAttachmentVariant {
                    weight: 1 << 30,
                    ..group.variants[0].clone()
                };
                4
            ],
            ..group.clone()
        };
        assert_eq!(wrapped_zero.select(|| seed.draw()).unwrap(), None);
        assert_eq!(seed.counter, 10);
        let leading_dead = EntityAttachmentGroup {
            variants: vec![
                EntityAttachmentVariant {
                    weight: 0,
                    ..group.variants[0].clone()
                },
                group.variants[0].clone(),
            ],
            ..group
        };
        assert_eq!(leading_dead.select(|| seed.draw()).unwrap(), Some(1));
        assert_eq!(seed.counter, 11);
    }

    #[test]
    fn weighted_index_matches_pinned_instruction_cases_including_sign_extended_totals() {
        let law: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../extractor/native/runs/paired_particle_viewer_runtime/STATIC_ATTACHMENT_LAW.json"
        )).unwrap();
        let cases = law["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 48);
        for case in cases {
            let group = EntityAttachmentGroup {
                name: String::new(),
                variants: case["weights"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|weight| EntityAttachmentVariant {
                        name: String::new(),
                        weight: weight.as_u64().unwrap() as i32,
                        children: Vec::new(),
                    })
                    .collect(),
            };
            let mut calls = 0;
            let result = group.select(|| {
                calls += 1;
                case["draw"].as_u64().unwrap() as u32
            });
            assert_eq!(
                calls != 0,
                case["consumes_draw"].as_bool().unwrap(),
                "{case}"
            );
            if case["total_u32"] != 0 && case["index"].is_null() {
                assert!(
                    result.unwrap_err().to_string().contains("native no-index"),
                    "{case}"
                );
            } else {
                assert_eq!(
                    result.unwrap(),
                    case["index"].as_u64().map(|x| x as usize),
                    "{case}"
                );
            }
        }
    }

    #[test]
    fn bad_unused_attachment_does_not_refuse_other_entities_or_hide_clone_cycles() {
        let catalog = catalog(
            r#"
entity = { name = valid attach = { root = child } }
entity = { name = bad group = nope }
entity = { name = cycle_a clone = cycle_b }
entity = { name = cycle_b clone = cycle_a }
"#,
        );
        assert_eq!(
            catalog.resolve("valid").unwrap().groups[0].variants[0].children[0].1,
            "child"
        );
        assert!(catalog
            .resolve("missing")
            .unwrap_err()
            .to_string()
            .contains("source is unavailable"));
        assert!(catalog
            .resolve("bad")
            .unwrap_err()
            .to_string()
            .contains("not an object"));
        assert!(catalog
            .resolve("cycle_a")
            .unwrap_err()
            .to_string()
            .contains("recursive"));
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    pub struct Fixture {
        root: PathBuf,
        pub resolver: GameFileResolver,
        pub admission: replay_core::ValidatedContentAdmission,
    }
    impl Fixture {
        pub fn new(registrations: &str, particles: &str) -> Self {
            Self::with_extra_files(registrations, particles, &[])
        }
        pub fn with_extra_files(
            registrations: &str,
            particles: &str,
            files: &[(&str, &[u8])],
        ) -> Self {
            let root = std::env::temp_dir().join(format!(
                "particle-authored-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(root.join("gfx")).unwrap();
            std::fs::write(root.join("gfx/particles.gfx"), registrations).unwrap();
            std::fs::write(root.join("gfx/particles.asset"), particles).unwrap();
            for (path, bytes) in files {
                let target = root.join(path);
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                std::fs::write(target, bytes).unwrap();
            }
            let raw = GameFileResolver::vanilla(&root).unwrap();
            let (resolver, admission) = crate::test_support::admitted_vanilla_resolver(
                &raw, &std::collections::BTreeSet::new(), "particle-authored-fixture");
            Self { root, resolver, admission }
        }
        pub fn definitions(&self) -> ParticleDefinitions {
            ParticleDefinitions::load(&self.resolver).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            portable_test_fs_retry::remove_dir_all(&self.root).unwrap();
        }
    }
    pub const REGISTRATION: &str =
        r#"objectTypes = { pdxparticle = { name = "paired" type = "effect" scale = 2 } }"#;
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn texture_descriptor_retains_native_dimensions_and_fixed_string_widths() {
        let file = "a".repeat(270);
        let shader = "b".repeat(80);
        let fixture = Fixture::new(REGISTRATION, &format!("particle={{name=effect subsystem={{name=s max_amount=1 texture={{file={file} shader={shader} x=4 y=8}} sort=age invert=yes}}}}"));
        let definition = fixture.definitions().resolve("paired").unwrap();
        let system = &definition.subsystems[0];
        let texture = system.texture.as_ref().unwrap();
        assert_eq!(texture.file, "a".repeat(255));
        assert_eq!(texture.shader, "b".repeat(63));
        assert_eq!((texture.columns, texture.rows), (4, 8));
        assert_eq!(system.sort, ParticleSort::Age);
        assert!(system.invert);
        let fixture = Fixture::new(
            REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=1 texture={file=unrequested.dds}}}",
        );
        let definition = fixture.definitions().resolve("paired").unwrap();
        let texture = definition.subsystems[0].texture.as_ref().unwrap();
        assert_eq!((texture.columns, texture.rows), (1, 1));
        assert!(texture.shader.is_empty());
        assert_eq!(definition.subsystems[0].sort, ParticleSort::None);
        assert!(!definition.subsystems[0].invert);
    }

    #[test]
    fn force_defaults_vectors_and_order_keep_native_operands() {
        let fixture = Fixture::new(REGISTRATION, "particle={name=effect force={name=default} force={name=wind type=planar position={1 2 3} direction={4 5 6} yaw=30 division=8 local_force=no amount=.150} subsystem={name=s max_amount=4 force={wind default wind} slave_particles=2}}");
        let definition = fixture.definitions().resolve("paired").unwrap();
        let system = &definition.subsystems[0];
        assert_eq!(system.force_division, 2);
        assert_eq!(
            system
                .forces
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            ["wind", "default", "wind"]
        );
        assert!(Arc::ptr_eq(&system.forces[0], &system.forces[2]));
        let wind = &system.forces[0];
        assert_eq!(wind.position, [1.0, 2.0, 3.0]);
        assert_eq!(wind.direction, [4.0, 5.0, 6.0]);
        assert_eq!(
            (wind.yaw, wind.division, wind.local_force),
            (30.0, 8, false)
        );
        assert_eq!(wind.amount.initial(0.0), (0.15, 0.0, false));
        let default = &system.forces[1];
        assert_eq!(default.kind, ParticleForceKind::Planar);
        assert_eq!(default.position, [0.0; 3]);
        assert_eq!(default.direction, [0.0, 1.0, 0.0]);
        assert_eq!(
            (default.yaw, default.division, default.local_force),
            (0.0, 16, true)
        );
        assert_eq!(default.amount.initial(0.0), (0.0, 0.0, false));
    }

    #[test]
    fn malformed_force_fields_never_become_defaults_or_variation_ranges() {
        for field in [
            "amount={1 2}",
            "local_force=maybe",
            "type={planar}",
            "yaw={1}",
            "direction=2",
        ] {
            let fixture = Fixture::new(REGISTRATION, &format!("particle={{name=effect force={{name=bad {field}}} subsystem={{name=s max_amount=1 force=bad}}}}"));
            let error = fixture.definitions().resolve("paired").unwrap_err();
            assert_eq!(error.subsystem.as_deref(), Some("bad"));
            assert_eq!(error.reason, ParticleRefusalReason::MalformedContent);
        }
    }

    #[test]
    fn authenticated_definitions_are_cached_and_unused_texture_is_not_required() {
        let fixture = Fixture::new(
            REGISTRATION,
            r#"particle={name=effect subsystem={name=smoke max_amount=4 duration=1 emission=2 life={1 0.25} texture={file="missing-unused.dds"}}}"#,
        );
        let mut definitions = fixture.definitions();
        let first = definitions.resolve("paired").unwrap();
        assert!(Arc::ptr_eq(&first, &definitions.resolve("paired").unwrap()));
        assert_eq!(first.authored_capacity(), 4);
        assert_eq!(first.authored_scale_bits(), 2.0_f32.to_bits());
        assert_eq!(first.subsystems[0].life, VariedScalar::plain(1.0, 0.25));
    }

    #[test]
    fn absent_registration_and_definition_are_distinct() {
        let fixture = Fixture::new(REGISTRATION, "particle={name=another}");
        let mut definitions = fixture.definitions();
        assert_eq!(
            definitions.resolve("absent").unwrap_err().reason,
            ParticleRefusalReason::MissingRegistration
        );
        let error = definitions.resolve("paired").unwrap_err();
        assert_eq!(error.definition, "effect");
        assert_eq!(error.reason, ParticleRefusalReason::MissingDefinition);
    }

    #[test]
    fn conflicting_names_never_select_an_arbitrary_definition() {
        let fixture = Fixture::new(
            &format!("{REGISTRATION} {REGISTRATION}"),
            "particle={name=effect}",
        );
        assert_eq!(
            fixture.definitions().resolve("paired").unwrap_err().reason,
            ParticleRefusalReason::ConflictingRegistration
        );
        let fixture = Fixture::new(
            REGISTRATION,
            "particle={name=effect} particle={name=effect}",
        );
        assert_eq!(
            fixture.definitions().resolve("paired").unwrap_err().reason,
            ParticleRefusalReason::ConflictingDefinition
        );
    }

    #[test]
    fn empty_roster_and_missing_authored_capacity_remain_distinct() {
        let fixture = Fixture::new(REGISTRATION, "particle={name=effect}");
        assert_eq!(
            fixture
                .definitions()
                .resolve("paired")
                .unwrap()
                .authored_capacity(),
            0
        );
        let fixture = Fixture::new(
            REGISTRATION,
            "particle={name=effect subsystem={name=smoke}}",
        );
        let error = fixture.definitions().resolve("paired").unwrap_err();
        assert_eq!(error.subsystem.as_deref(), Some("smoke"));
        assert_eq!(error.input, "max_amount");
        assert_eq!(error.reason, ParticleRefusalReason::MissingAuthoredInput);
    }

    #[test]
    fn trail_controls_the_implicit_billboard_but_explicit_billboard_wins() {
        for (fields, trail, billboard) in [
            ("", false, true),
            ("trail=no", false, true),
            ("trail=yes", true, false),
            ("trail=yes billboard=yes", true, true),
            ("billboard=yes trail=yes", true, true),
            ("trail=no billboard=no", false, false),
        ] {
            let fixture = Fixture::new(
                REGISTRATION,
                &format!("particle={{name=effect subsystem={{name=s max_amount=1 {fields}}}}}"),
            );
            let definition = fixture.definitions().resolve("paired").unwrap();
            assert_eq!(definition.subsystems[0].trail, trail, "{fields}");
            assert_eq!(definition.subsystems[0].billboard, billboard, "{fields}");
        }
    }

    #[test]
    fn invalid_fields_refuse_the_named_subsystem() {
        for field in [
            "life=1 life=2",
            "life=nan",
            "life=1e30",
            "collision=unbound",
            "position=1",
        ] {
            let fixture = Fixture::new(
                REGISTRATION,
                &format!("particle={{name=effect subsystem={{name=smoke max_amount=4 {field}}}}}"),
            );
            let error = fixture.definitions().resolve("paired").unwrap_err();
            assert_eq!(error.subsystem.as_deref(), Some("smoke"), "{field}");
        }
        let fixture = Fixture::new(
            REGISTRATION,
            "particle={name=effect subsystem={name=smoke max_amount=-1}}",
        );
        assert_eq!(
            fixture.definitions().resolve("paired").unwrap_err().reason,
            ParticleRefusalReason::InvalidNumber
        );
    }

    #[test]
    fn complete_content_admission_precedes_catalogue_reads() {
        let resolver = GameFileResolver::vanilla(std::env::temp_dir()).unwrap();
        assert!(matches!(
            ParticleDefinitions::load(&resolver),
            Err(ParticleInputRefusal {
                reason: ParticleRefusalReason::ContentAdmissionRequired,
                ..
            })
        ));
    }

    #[test]
    fn particle_draw_capacity_is_catalog_maximum_with_recursive_multiplicity() {
        let fixture = Fixture::new(REGISTRATION, "particle={name=effect subsystem={name=root max_amount=2 childsystem={name=child max_amount=3 childsystem={name=grandchild max_amount=4}}}} particle={name=other subsystem={name=a max_amount=40} subsystem={name=b max_amount=0}}");
        let mut definitions = fixture.definitions();
        let capacity = definitions.draw_capacity().unwrap();
        assert_eq!(capacity.packets, 1 + 2 + 6);
        assert_eq!(capacity.particles, 40);
        assert_eq!(capacity.lease, definitions.content_lease());
        assert_eq!(
            definitions.resolve("paired").unwrap().authored_capacity(),
            2 + 6 + 24
        );
        assert_eq!(definitions.draw_capacity().unwrap().packets, 9);
    }

    #[test]
    fn recursive_capacity_includes_every_ordered_child_slot() {
        let fixture = Fixture::new(REGISTRATION,
            "particle={name=effect subsystem={name=root max_amount=3 childsystem={name=first max_amount=2} childsystem={name=second max_amount=4}}}");
        let definition = fixture.definitions().resolve("paired").unwrap();
        assert_eq!(definition.authored_capacity(), 21);
        assert_eq!(
            definition.subsystems[0]
                .children
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    }

    #[test]
    fn inherited_registration_constructor_scale_is_one() {
        let fixture = Fixture::new(
            "pdxparticle={name=paired type=effect}",
            "particle={name=effect}",
        );
        assert_eq!(
            fixture
                .definitions()
                .resolve("paired")
                .unwrap()
                .authored_scale_bits(),
            0x3f80_0000
        );
    }

    #[test]
    fn fixed_token_conversion_keeps_engine_fraction_and_zero_rules() {
        assert_eq!(engine_varied_number("0.000001"), Some(0.00001));
        assert_eq!(engine_varied_number("-0.123456"), Some(-1.23456));
        assert_eq!(engine_varied_number("0.5"), Some(0.5));
        assert_eq!(engine_varied_number("-0.0").unwrap().to_bits(), 0);
        assert_eq!(
            engine_varied_number("167.77217").unwrap().to_bits(),
            (16777217_i64 as f32 / 100_000.0).to_bits()
        );
        assert_ne!(
            engine_varied_number("167.77217").unwrap().to_bits(),
            "167.77217".parse::<f32>().unwrap().to_bits()
        );
        assert!(engine_varied_number("1e6").is_none());
    }

    #[test]
    fn shared_shape_fields_append_in_authored_order_and_keep_range_callback() {
        let fixture = Fixture::new(REGISTRATION, "particle={name=effect subsystem={name=s max_amount=1 emitter_type=box sphere_emitter_radius={2 1} box_emitter_x=3 rotation_speed={0 2} rotation_speed_roll={0 0} position={x=1 y=2 z=3} color={r=1 g=2 b=3 a=4}}}");
        let definition = fixture.definitions().resolve("paired").unwrap();
        let s = &definition.subsystems[0];
        let shape = s.shape.properties().unwrap();
        assert_eq!(shape[0].initial(0.0), (3.0, 1.0, true));
        assert_eq!(s.spawn_draw_columns[6].initial(0.0), (0.0, 0.0, true));
        assert_eq!(
            s.origin.iter().map(|v| v.base).collect::<Vec<_>>(),
            [3.0, 2.0, 1.0]
        );
        assert_eq!(
            s.spawn_draw_columns[8..]
                .iter()
                .map(|v| v.base)
                .collect::<Vec<_>>(),
            [3.0, 2.0, 1.0, 4.0]
        );
    }

    #[test]
    fn once_curve_uses_post_read_duration_and_shares_prepared_table() {
        let fixture = Fixture::new(REGISTRATION, "particle={name=effect animation={name=fade minValue=2 maxValue=2 curve={0 0 1 1}} subsystem={name=s max_amount=1 life={1, fade 0.25} emission=1, fade}}");
        let definition = fixture.definitions().resolve("paired").unwrap();
        let s = &definition.subsystems[0];
        assert_eq!(s.life.initial(0.0), (3.0, 0.25, true));
        assert_eq!(s.emission.initial(0.0), (3.0, 0.0, false));
        assert_eq!(s.life.base_animations[0].end, 1.0);
        assert!(Arc::ptr_eq(
            &s.life.base_animations[0],
            &s.emission.base_animations[0]
        ));
    }

    #[test]
    fn initial_callback_includes_system_tracks_only_when_selected_by_initial_tracks() {
        let fixture = Fixture::new(REGISTRATION,
            "particle={name=effect animation={name=once minValue=2 maxValue=2 curve={0 0 1 1}} animation={name=system time=system minValue=4 maxValue=4 curve={0 0 1 1}} animation={name=age time=life_abs minValue=8 maxValue=8 curve={0 0 1 1}} subsystem={name=s max_amount=1 size={3,system,once,age 1,system,once} mass=3,system}");
        let definition = fixture.definitions().resolve("paired").unwrap();
        let columns = &definition.subsystems[0].spawn_draw_columns;
        assert_eq!(columns[0].initial(0.25), (9.0, 7.0, true));
        assert_eq!(columns[7].initial(0.25), (3.0, 0.0, false));
        assert_eq!(columns[0].update_particle(9.0, 9.0, 0.25, 0.0, 1.0), 23.0);
    }

    #[test]
    fn normalized_visual_tracks_preserve_the_zero_life_multiply_branch() {
        let fixture = Fixture::new(REGISTRATION,
            "particle={name=effect animation={name=mul time=life op=MUL minValue=2 maxValue=4 curve={0 0 1 1}}              animation={name=add time=life op=ADD minValue=2 maxValue=4 curve={0 0 1 1}}              subsystem={name=s max_amount=1 size=3,mul mass=3,add}}");
        let definition = fixture.definitions().resolve("paired").unwrap();
        let s = &definition.subsystems[0];
        assert_eq!(
            s.spawn_draw_columns[0].update_particle(3.0, 99.0, 5.0, 0.0, 0.0),
            6.0
        );
        assert_eq!(
            s.spawn_draw_columns[7].update_particle(3.0, 99.0, 5.0, 0.0, 0.0),
            7.0
        );
        assert_eq!(
            s.spawn_draw_columns[0].update_particle(3.0, 99.0, 5.0, 1.0, 1.0),
            12.0
        );
    }

    #[test]
    fn dynamic_curves_do_not_change_birth_life_or_consume_birth_randomness() {
        for time in ["system", "life_abs", "life"] {
            let fixture = Fixture::new(REGISTRATION, &format!("particle={{name=effect animation={{name=dynamic time={time} minValue=99 maxValue=99 curve={{0 0 1 1}}}} subsystem={{name=s max_amount=1 life={{2,dynamic 0,dynamic}}}}}}"));
            let definition = fixture.definitions().resolve("paired").unwrap();
            assert_eq!(
                definition.subsystems[0].life.initial(9.0),
                (2.0, 0.0, false)
            );
        }
    }

    #[test]
    fn all_authored_turbulence_forces_are_initialized_even_without_references() {
        let fixture = Fixture::new(REGISTRATION, "particle={name=effect force={name=noise type=turbulence} force={name=gravity} subsystem={name=s max_amount=1 force=gravity}}");
        assert_eq!(
            fixture
                .definitions()
                .resolve("paired")
                .unwrap()
                .turbulence_divisions
                .as_ref(),
            &[16]
        );
        let fixture = Fixture::new(
            REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=1 force=absent}}",
        );
        assert_eq!(
            fixture.definitions().resolve("paired").unwrap_err().input,
            "force:absent"
        );
    }

    #[test]
    fn registrations_share_one_compiled_roster_while_retaining_their_scales() {
        let fixture = Fixture::new("pdxparticle={name=paired type=effect scale=2} pdxparticle={name=alias type=effect scale=3}",
            "particle={name=effect subsystem={name=s max_amount=4}}");
        let mut definitions = fixture.definitions();
        let first = definitions.resolve("paired").unwrap();
        let second = definitions.resolve("alias").unwrap();
        assert!(Arc::ptr_eq(&first.subsystems, &second.subsystems));
        assert_ne!(first.authored_scale_bits(), second.authored_scale_bits());
    }

    #[test]
    fn authenticated_train_outer_eof_does_not_block_particle_resolution() {
        let train = include_bytes!("../tests/fixtures/ger_train_coal_equipment_1.asset");
        assert_eq!(train.len(), 348);
        let fixture = Fixture::with_extra_files(
            REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=4}}",
            &[(
                "gfx/models/units/trains/ger_train_coal_equipment_1.asset",
                train,
            )],
        );
        let mut definitions = fixture.definitions();
        assert_eq!(
            definitions.resolve("paired").unwrap().authored_capacity(),
            4
        );
        assert_eq!(
            definitions
                .resolve("GER_train_coal_equipment_1_entity")
                .unwrap_err()
                .reason,
            ParticleRefusalReason::MissingRegistration
        );
    }

    #[test]
    fn persistent_particle_eof_keeps_the_last_scalar_and_file_variable() {
        let fixture = Fixture::new(
            REGISTRATION,
            "@capacity=4 particle={name=effect subsystem={name=s max_amount=@capacity life=2 # }",
        );
        let definition = fixture.definitions().resolve("paired").unwrap();
        assert_eq!(definition.authored_capacity(), 4);
        assert_eq!(definition.subsystems[0].life, VariedScalar::plain(2.0, 0.0));
    }

    #[test]
    fn asset_root_close_terminates_before_conflicting_particle_declarations() {
        let fixture = Fixture::new(
            REGISTRATION,
            "particle={name=effect subsystem={name=s max_amount=4}} } particle={name=effect}",
        );
        assert_eq!(
            fixture
                .definitions()
                .resolve("paired")
                .unwrap()
                .authored_capacity(),
            4
        );
    }

    #[test]
    fn asset_admission_still_names_actual_parse_and_decode_damage() {
        for bytes in [
            &b"entity={name="[..],
            &b"entity={name=\"unterminated"[..],
            &b"\xef\xbb\xbfentity={name=\xff}"[..],
        ] {
            let path = "gfx/models/broken.asset";
            let fixture =
                Fixture::with_extra_files(REGISTRATION, "particle={name=effect}", &[(path, bytes)]);
            let error = ParticleDefinitions::load(&fixture.resolver).err().unwrap();
            assert_eq!(error.input, path);
            assert_eq!(error.reason, ParticleRefusalReason::MalformedContent);
        }
    }
}
