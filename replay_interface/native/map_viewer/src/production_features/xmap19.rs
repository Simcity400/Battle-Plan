
use crate::{
    entity_default_state_events, entity_definition_count, particle_asset_defines,
    particle_asset_definition_count, particle_asset_shader_effects, particle_asset_texture_files,
    pdxparticle_registration_count, pdxparticle_type, weather_distance_fog_defines_from_source,
    weather_distance_fog_factor, weather_effects_from_source, weather_emitters_from_source,
    weather_fade_defines_from_source, weather_selected_candidate_index, weather_visibility_law,
    FoundationFeatureContext, FoundationFeatureOutput, GameFileResolver,
    ProductionMapFeatureAdapter, Result, WeatherEffectBinding, WeatherPositionSize, WeatherType,
    FOG_CONSTANTS_SOURCE, GRAPHICS_DEFINES_SOURCE, WEATHER_EFFECTS_SOURCE,
    WEATHER_POSITIONS_SOURCE,
};
use portable_content_manifest::UsedContentKey;
use portable_render_stream::{FamilyDisposition, FoundationGap, MapVisualFamily};
use replay_core::{Leaf, ValidatedContentAdmission, ViewStatus, WeatherRegionRow};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const XMAP19_WEATHER_SCHEMA: &str = "hoi4.xmap19.weather-atmosphere.a729.v2";
pub const XMAP19_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const XMAP19_A729_PRODUCER_RVA: &str = "0x0166ec50";
pub const XMAP19_ASSIGNED_STAGE_NAME: &str = "xmap19.weather.atmosphere.prepare";
pub const XMAP19_ASSIGNED_OPERATION_NAME: &str = "weather.atmosphere.prepare";
const WEATHER_AUTHORITY_GAPS: [&str; 2] = [
    "weather.region_gameplay",
    "weather.visual_membership_lifecycle",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedWeatherAuthoredEvent {
    pub authored_event_index: u32,
    pub ordered_particle_effects: Vec<PreparedWeatherParticleEffect>,
    pub node_token: Option<String>,
    pub time_f32_bits: Option<u32>,
    pub keep_particle: Option<bool>,
    pub trigger_once: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedWeatherParticleEffect {
    pub authored_effect_index: u32,
    pub particle_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedWeatherEntityProgram {
    pub entity_name: String,
    pub entity_definition_source: UsedContentKey,
    pub default_state_name: String,
    pub ordered_events: Vec<PreparedWeatherAuthoredEvent>,
    pub ordered_particle_names: Vec<String>,
    pub ordered_particle_types: Vec<String>,
    pub ordered_particle_registration_sources: Vec<UsedContentKey>,
    pub ordered_particle_asset_paths: Vec<String>,
    pub ordered_particle_shader_effects: Vec<Vec<String>>,
    pub ordered_particle_shader_paths: Vec<Vec<String>>,
    pub ordered_texture_paths: Vec<String>,
    pub ordered_content: Vec<UsedContentKey>,
}

impl PreparedWeatherEntityProgram {
    fn validate(&self) -> Result<()> {
        if self.entity_name.trim().is_empty()
            || self.default_state_name.trim().is_empty()
            || self.ordered_events.is_empty()
            || self.ordered_particle_names.is_empty()
            || self.ordered_particle_names.len() != self.ordered_particle_types.len()
            || self.ordered_particle_types.len() != self.ordered_particle_registration_sources.len()
            || self.ordered_particle_types.len() != self.ordered_particle_asset_paths.len()
            || self.ordered_particle_asset_paths.len() != self.ordered_particle_shader_effects.len()
            || self.ordered_particle_shader_effects.len()
                != self.ordered_particle_shader_paths.len()
            || self.ordered_texture_paths.is_empty()
            || self.ordered_content.is_empty()
        {
            return Err("XMAP19 entity program is incomplete".into());
        }
        let mut event_particles = Vec::new();
        for (expected_index, event) in self.ordered_events.iter().enumerate() {
            if event.authored_event_index != u32::try_from(expected_index)?
                || event
                    .node_token
                    .as_ref()
                    .is_some_and(|token| token.trim().is_empty())
                || event.time_f32_bits.is_some_and(|bits| {
                    let value = f32::from_bits(bits);
                    !value.is_finite() || value < 0.0
                })
            {
                return Err("XMAP19 authored entity event is invalid".into());
            }
            let mut prior_effect = None;
            for effect in &event.ordered_particle_effects {
                if effect.particle_token.trim().is_empty()
                    || prior_effect.is_some_and(|prior| effect.authored_effect_index <= prior)
                {
                    return Err("XMAP19 authored particle-effect order is invalid".into());
                }
                prior_effect = Some(effect.authored_effect_index);
                event_particles.push(effect.particle_token.clone());
            }
        }
        if event_particles != self.ordered_particle_names {
            return Err("XMAP19 entity particle census differs from all-event order".into());
        }
        let mut paths = BTreeSet::new();
        for content in &self.ordered_content {
            content
                .validate()
                .map_err(|error| format!("XMAP19 entity content identity: {error}"))?;
            if !paths.insert(
                content
                    .normalized_game_relative_path
                    .replace('\\', "/")
                    .to_ascii_lowercase(),
            ) {
                return Err("XMAP19 entity content path is duplicated".into());
            }
        }
        self.entity_definition_source
            .validate()
            .map_err(|error| format!("XMAP19 entity definition source: {error}"))?;
        if !self
            .ordered_content
            .iter()
            .any(|content| content == &self.entity_definition_source)
        {
            return Err("XMAP19 entity closure omits its real definition source".into());
        }
        for (index, registration) in self
            .ordered_particle_registration_sources
            .iter()
            .enumerate()
        {
            registration
                .validate()
                .map_err(|error| format!("XMAP19 particle registration source: {error}"))?;
            if !self
                .ordered_content
                .iter()
                .any(|content| content == registration)
                || self.ordered_particle_shader_effects[index].is_empty()
                || self.ordered_particle_shader_effects[index].len()
                    != self.ordered_particle_shader_paths[index].len()
                || self.ordered_particle_shader_effects[index]
                    .iter()
                    .any(|effect| effect.trim().is_empty())
                || self.ordered_particle_shader_paths[index]
                    .iter()
                    .any(|path| path.trim().is_empty())
            {
                return Err("XMAP19 entity shader/registration closure is incomplete".into());
            }
            for shader_path in &self.ordered_particle_shader_paths[index] {
                if !self.ordered_content.iter().any(|content| {
                    content
                        .normalized_game_relative_path
                        .eq_ignore_ascii_case(shader_path)
                }) {
                    return Err(
                        "XMAP19 entity closure omits a resolved particle shader source".into(),
                    );
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap19WeatherTrigger {
    pub semantic_source_id: String,
    pub source_content: UsedContentKey,
    pub entity_name: String,
    pub state_name: String,
    pub authored_event_index: u32,
    pub authored_effect_index: u32,
    pub particle_token: String,
    pub node_token: Option<String>,
    pub time_f32_bits: Option<u32>,
    pub keep_particle: Option<bool>,
    pub trigger_once: Option<bool>,
    pub owner_admission_sha256: String,
}

impl Xmap19WeatherTrigger {
    pub fn validate(&self) -> Result<()> {
        self.source_content
            .validate()
            .map_err(|error| format!("XMAP19 trigger source content: {error}"))?;
        if self.semantic_source_id.trim().is_empty()
            || self.entity_name.trim().is_empty()
            || self.state_name.trim().is_empty()
            || self.particle_token.trim().is_empty()
            || self
                .node_token
                .as_ref()
                .is_some_and(|node| node.trim().is_empty())
            || self.time_f32_bits.is_some_and(|bits| {
                let value = f32::from_bits(bits);
                !value.is_finite() || value < 0.0
            })
            || !is_lower_sha256(&self.owner_admission_sha256)
        {
            return Err("XMAP19 trigger receipt is incomplete".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedWeatherSelection {
    pub authored_weather_effect_index: u16,
    pub primary_source_id: String,
    pub fading_source_id: Option<String>,
    pub primary: PreparedWeatherEntityProgram,
    pub fading: Option<PreparedWeatherEntityProgram>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedWeatherPosition {
    pub source_ordinal: u32,
    pub strategic_region_id: u32,
    pub position_xyz_f32_bits: [u32; 3],
    pub size: String,
    pub airborne_runtime_code: u8,
    pub weather_effect_key: Option<String>,
    pub selection: Option<PreparedWeatherSelection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedWeatherRegionEpoch {
    pub strategic_region_id: u32,
    pub airborne_runtime_code: u8,
    pub transition_expiry_game_hour: i32,
    pub epoch_game_hour: i64,
    pub epoch_ordinal: u64,
    pub selection_seed_words: Option<[u32; 2]>,
    pub selection_seed_after_positions: Option<[u32; 2]>,
    pub authenticated_producer_instance_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedXmap19WeatherFrame {
    pub schema: String,
    pub hour_index: i64,
    pub source_sequence: u64,
    pub lifecycle_payload_sha256: String,
    pub camera_distance_f32_bits: u32,
    pub primary_visibility_alpha_f32_bits: u32,
    pub fading_visibility_alpha_f32_bits: u32,
    pub playback_rate_f32_bits: u32,
    pub distance_fog_color_f32_bits: [u32; 3],
    pub distance_fog_begin_f32_bits: u32,
    pub distance_fog_end_f32_bits: u32,
    pub distance_fog_max_f32_bits: u32,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub regions: Vec<PreparedWeatherRegionEpoch>,
    pub ordered_positions: Vec<PreparedWeatherPosition>,
    pub deterministic_sha256: String,
}

impl PreparedXmap19WeatherFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP19_WEATHER_SCHEMA
            || !is_lower_sha256(&self.lifecycle_payload_sha256)
            || self.regions.len() != 305
            || self.ordered_used_content.is_empty()
        {
            return Err("XMAP19 prepared frame is incomplete".into());
        }
        let distance = f32::from_bits(self.camera_distance_f32_bits);
        let primary_alpha = f32::from_bits(self.primary_visibility_alpha_f32_bits);
        let fading_alpha = f32::from_bits(self.fading_visibility_alpha_f32_bits);
        let playback = f32::from_bits(self.playback_rate_f32_bits);
        if !distance.is_finite()
            || distance < 0.0
            || !primary_alpha.is_finite()
            || !(0.0..=1.0).contains(&primary_alpha)
            || !fading_alpha.is_finite()
            || !(0.0..=1.0).contains(&fading_alpha)
            || !playback.is_finite()
            || !(0.0..=1.0).contains(&playback)
        {
            return Err("XMAP19 camera visibility/playback output is invalid".into());
        }
        let distance_fog = crate::WeatherDistanceFogDefines {
            color: self.distance_fog_color_f32_bits.map(f32::from_bits),
            begin: f32::from_bits(self.distance_fog_begin_f32_bits),
            end: f32::from_bits(self.distance_fog_end_f32_bits),
            max: f32::from_bits(self.distance_fog_max_f32_bits),
        };
        weather_distance_fog_factor(
            [0.0, distance_fog.end + 1.0, 0.0],
            [0.0, 0.0, 1.0],
            &distance_fog,
        )?;
        for (expected, region) in self.regions.iter().enumerate() {
            if region.strategic_region_id != expected as u32
                || !matches!(region.airborne_runtime_code, 0 | 2 | 3 | 4 | 5 | 7)
                || region.epoch_game_hour > self.hour_index
                || (region.airborne_runtime_code == 0) != region.selection_seed_words.is_none()
                || (region.airborne_runtime_code == 0)
                    != region.selection_seed_after_positions.is_none()
                || (region.airborne_runtime_code == 0
                    && !region.authenticated_producer_instance_ids.is_empty())
            {
                return Err("XMAP19 region epoch census is invalid".into());
            }
        }
        let mut producer_ids_by_region = BTreeMap::<u32, Vec<String>>::new();
        let mut selections_by_region = BTreeMap::<u32, u32>::new();
        for (expected_ordinal, position) in self.ordered_positions.iter().enumerate() {
            let epoch = self
                .regions
                .get(position.strategic_region_id as usize)
                .ok_or("XMAP19 position references an unknown region epoch")?;
            if position.source_ordinal != u32::try_from(expected_ordinal + 1)?
                || position.airborne_runtime_code != epoch.airborne_runtime_code
                || position
                    .position_xyz_f32_bits
                    .iter()
                    .any(|bits| !f32::from_bits(*bits).is_finite())
                || !matches!(position.size.as_str(), "big" | "small")
            {
                return Err("XMAP19 weather position identity/order is invalid".into());
            }
            match (&position.weather_effect_key, &position.selection) {
                (None, None) if position.airborne_runtime_code == 0 => {}
                (Some(key), Some(selection))
                    if weather_type_for_code(position.airborne_runtime_code)
                        .is_some_and(|weather| weather.key() == key) =>
                {
                    let expected_primary = weather_producer_instance_id(
                        position.strategic_region_id,
                        epoch.epoch_ordinal,
                        position.airborne_runtime_code,
                        position.source_ordinal,
                        "primary",
                    );
                    if selection.primary_source_id.trim().is_empty()
                        || selection.primary_source_id != expected_primary
                        || selection.fading_source_id.is_some() != selection.fading.is_some()
                    {
                        return Err("XMAP19 selected producer identity is invalid".into());
                    }
                    producer_ids_by_region
                        .entry(position.strategic_region_id)
                        .or_default()
                        .push(selection.primary_source_id.clone());
                    let selection_count = selections_by_region
                        .entry(position.strategic_region_id)
                        .or_default();
                    *selection_count = selection_count
                        .checked_add(1)
                        .ok_or("XMAP19 weather selection count overflow")?;
                    if let Some(fading_source_id) = &selection.fading_source_id {
                        let expected_fading = weather_producer_instance_id(
                            position.strategic_region_id,
                            epoch.epoch_ordinal,
                            position.airborne_runtime_code,
                            position.source_ordinal,
                            "fading",
                        );
                        if fading_source_id != &expected_fading {
                            return Err("XMAP19 fading producer identity is invalid".into());
                        }
                        producer_ids_by_region
                            .entry(position.strategic_region_id)
                            .or_default()
                            .push(fading_source_id.clone());
                    }
                    selection.primary.validate()?;
                    if let Some(fading) = &selection.fading {
                        fading.validate()?;
                    }
                }
                (Some(_), None) if position.airborne_runtime_code != 0 => {}
                _ => return Err("XMAP19 position weather resolution is inconsistent".into()),
            }
        }
        for region in &self.regions {
            if producer_ids_by_region
                .remove(&region.strategic_region_id)
                .unwrap_or_default()
                != region.authenticated_producer_instance_ids
            {
                return Err("XMAP19 prepared producer-instance census mismatch".into());
            }
            let expected_seed_after = region.selection_seed_words.map(|mut words| {
                words[0] = words[0].wrapping_add(
                    selections_by_region
                        .get(&region.strategic_region_id)
                        .copied()
                        .unwrap_or(0),
                );
                words
            });
            if region.selection_seed_after_positions != expected_seed_after {
                return Err("XMAP19 prepared selector advancement mismatch".into());
            }
        }
        let mut content_paths = BTreeSet::new();
        for content in &self.ordered_used_content {
            content
                .validate()
                .map_err(|error| format!("XMAP19 used-content identity: {error}"))?;
            if !content_paths.insert(
                content
                    .normalized_game_relative_path
                    .replace('\\', "/")
                    .to_ascii_lowercase(),
            ) {
                return Err("XMAP19 used-content path is duplicated".into());
            }
        }
        if self.deterministic_sha256 != prepared_weather_digest(self)? {
            return Err("XMAP19 prepared frame digest mismatch".into());
        }
        Ok(())
    }

    pub fn ordered_producer_instance_ids(&self) -> Vec<&str> {
        self.regions
            .iter()
            .flat_map(|region| {
                region
                    .authenticated_producer_instance_ids
                    .iter()
                    .map(String::as_str)
            })
            .collect()
    }

    pub fn weather_trigger(
        &self,
        producer_instance_id: &str,
        authored_event_index: u32,
        authored_effect_index: u32,
    ) -> Result<Xmap19WeatherTrigger> {
        self.validate()?;
        let mut selected_program = None;
        for position in &self.ordered_positions {
            let Some(selection) = &position.selection else {
                continue;
            };
            if selection.primary_source_id == producer_instance_id {
                selected_program = Some(&selection.primary);
                break;
            }
            if selection.fading_source_id.as_deref() == Some(producer_instance_id) {
                selected_program = selection.fading.as_ref();
                break;
            }
        }
        let program = selected_program.ok_or_else(|| {
            format!(
                "XMAP19 producer instance {producer_instance_id:?} is outside the authenticated census"
            )
        })?;
        let event = program
            .ordered_events
            .get(usize::try_from(authored_event_index)?)
            .filter(|event| event.authored_event_index == authored_event_index)
            .ok_or_else(|| {
                format!(
                    "XMAP19 producer instance {producer_instance_id:?} has no authored event {authored_event_index}"
                )
            })?;
        let particle_token = event
            .ordered_particle_effects
            .iter()
            .find(|effect| effect.authored_effect_index == authored_effect_index)
            .map(|effect| effect.particle_token.clone())
            .ok_or_else(|| {
                format!(
                    "XMAP19 producer instance {producer_instance_id:?} event {authored_event_index} has no particle effect {authored_effect_index}"
                )
            })?;
        let source_content = program.entity_definition_source.clone();
        let trigger = Xmap19WeatherTrigger {
            semantic_source_id: producer_instance_id.to_string(),
            source_content,
            entity_name: program.entity_name.clone(),
            state_name: program.default_state_name.clone(),
            authored_event_index,
            authored_effect_index,
            particle_token,
            node_token: event.node_token.clone(),
            time_f32_bits: event.time_f32_bits,
            keep_particle: event.keep_particle,
            trigger_once: event.trigger_once,
            owner_admission_sha256: self.deterministic_sha256.clone(),
        };
        trigger.validate()?;
        Ok(trigger)
    }
}

#[derive(Clone)]
struct VariantProgram {
    authored_weather_effect_index: u16,
    size: WeatherPositionSize,
    primary: PreparedWeatherEntityProgram,
    fading: Option<PreparedWeatherEntityProgram>,
}

#[derive(Clone)]
struct Xmap19Content {
    ordered_used_content: Vec<UsedContentKey>,
    effects: BTreeMap<WeatherType, Vec<VariantProgram>>,
    positions: crate::WeatherEmitterTable,
    defines: crate::WeatherFadeDefines,
    distance_fog: crate::WeatherDistanceFogDefines,
}

pub(crate) struct Xmap19WeatherAdapter {
    resolver: GameFileResolver,
    admission: ValidatedContentAdmission,
}

impl Xmap19WeatherAdapter {
    pub(crate) fn new(resolver: &GameFileResolver, admission: &ValidatedContentAdmission) -> Self {
        Self {
            resolver: resolver.clone(),
            admission: admission.clone(),
        }
    }
}

impl ProductionMapFeatureAdapter for Xmap19WeatherAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::WeatherAtmosphere
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        if context.world_frame.weather_status() != ViewStatus::Present
            || context.world_frame.weather_visual_lifecycle().status != ViewStatus::Present
        {
            return Ok(FoundationFeatureOutput {
                disposition: FamilyDisposition::Unavailable {
                    gap: unavailable_gap(context),
                },
                prepared_terrain: None,
                prepared_political: None,
                prepared_thin_borders: None,
                prepared_rivers_straits: None,
                prepared_water: None,
                prepared_lighting: None,
                prepared_rail_supply: None,
                prepared_weather: None,
                prepared_effects: None,
                prepared_units: None,
                prepared_combat: None,
                prepared_orders: None,
                prepared_map_modes: None,
                consumed_gap_ids: owned_gaps(context),
            });
        }
        let content = admit_content(&self.resolver, &self.admission)?;
        let prepared = prepare_weather(context, &content)?;
        let registration = registration_digest();
        let resources = Sha256::hex(
            &prepared
                .ordered_used_content
                .iter()
                .flat_map(|content| content.payload_sha256.bytes())
                .collect::<Vec<_>>(),
        );
        let target = Sha256::hex(
            format!(
                "{}\0{}\0{}",
                context.viewer.viewport_width,
                context.viewer.viewport_height,
                prepared.camera_distance_f32_bits
            )
            .as_bytes(),
        );
        let operation = context
            .require_assigned_stage(XMAP19_ASSIGNED_STAGE_NAME, XMAP19_ASSIGNED_OPERATION_NAME)?
            .foundation_operation_identity(registration.clone(), resources, target)
            .map_err(|error| format!("XMAP19 assigned-stage identity: {error}"))?;
        Ok(FoundationFeatureOutput {
            disposition: FamilyDisposition::Present {
                payload_schema: XMAP19_WEATHER_SCHEMA.into(),
                producer_registration_sha256: registration.clone(),
                payload_sha256: prepared.deterministic_sha256.clone(),
                ordered_operations: vec![operation],
                ordered_used_content: prepared.ordered_used_content.clone(),
            },
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_lighting: None,
            prepared_rail_supply: None,
            prepared_weather: Some(prepared),
            prepared_effects: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_map_modes: None,
            consumed_gap_ids: Vec::new(),
        })
    }
}

fn admit_content(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
) -> Result<Xmap19Content> {
    let weather_source = admitted_text(resolver, admission, WEATHER_EFFECTS_SOURCE)?;
    let positions_source = admitted_text(resolver, admission, WEATHER_POSITIONS_SOURCE)?;
    let defines_source = admitted_text(resolver, admission, GRAPHICS_DEFINES_SOURCE)?;
    let distance_fog_source = admitted_text(resolver, admission, FOG_CONSTANTS_SOURCE)?;
    let effects_table = weather_effects_from_source(&weather_source)?;
    let positions = weather_emitters_from_source(&positions_source)?;
    let defines = weather_fade_defines_from_source(&defines_source)?;
    let distance_fog = weather_distance_fog_defines_from_source(&distance_fog_source)?;

    let mut needed_entities = BTreeSet::new();
    for (_, binding) in effects_table.rows() {
        needed_entities.insert(binding.entity.clone());
        if let Some(fading) = &binding.fading_entity {
            needed_entities.insert(fading.clone());
        }
    }
    let entity_sources = if needed_entities.is_empty() {
        Vec::new()
    } else {
        admitted_effective_namespace(
            resolver,
            admission,
            "gfx/entities",
            "asset",
            "entity-definition",
        )?
    };
    let registration_sources = if needed_entities.is_empty() {
        Vec::new()
    } else {
        admitted_effective_namespace(
            resolver,
            admission,
            "gfx/entities",
            "gfx",
            "particle-registration",
        )?
    };
    let mut entity_particle_map = BTreeMap::<
        String,
        (
            UsedContentKey,
            String,
            Vec<PreparedWeatherAuthoredEvent>,
            Vec<(String, String, UsedContentKey)>,
        ),
    >::new();
    let mut needed_types = BTreeSet::new();
    for entity in needed_entities {
        let matches = entity_sources
            .iter()
            .filter(|source| entity_definition_count(&source.text, &entity) != 0)
            .collect::<Vec<_>>();
        let definition_count = matches
            .iter()
            .map(|source| entity_definition_count(&source.text, &entity))
            .sum::<usize>();
        if definition_count != 1 || matches.len() != 1 {
            return Err(format!(
                "XMAP19 entity {entity:?} resolved {definition_count} times across the effective namespace"
            )
            .into());
        }
        let entity_source = matches[0];
        let authored_state = entity_default_state_events(&entity_source.text, &entity)?;
        let authored_events = authored_state
            .ordered_events
            .into_iter()
            .map(|event| PreparedWeatherAuthoredEvent {
                authored_event_index: event.authored_event_index,
                ordered_particle_effects: event
                    .ordered_particle_effects
                    .into_iter()
                    .map(|effect| PreparedWeatherParticleEffect {
                        authored_effect_index: effect.authored_effect_index,
                        particle_token: effect.particle_token,
                    })
                    .collect(),
                node_token: event.node_token,
                time_f32_bits: event.time_f32_bits,
                keep_particle: event.keep_particle,
                trigger_once: event.trigger_once,
            })
            .collect::<Vec<_>>();
        let names = authored_events
            .iter()
            .flat_map(|event| {
                event
                    .ordered_particle_effects
                    .iter()
                    .map(|effect| effect.particle_token.clone())
            })
            .collect::<Vec<_>>();
        if names.is_empty() {
            return Err(format!("XMAP19 entity {entity:?} has no particle events").into());
        }
        let mut rows = Vec::new();
        for name in names {
            let matches = registration_sources
                .iter()
                .filter(|source| pdxparticle_registration_count(&source.text, &name) != 0)
                .collect::<Vec<_>>();
            let registration_count = matches
                .iter()
                .map(|source| pdxparticle_registration_count(&source.text, &name))
                .sum::<usize>();
            if registration_count != 1 || matches.len() != 1 {
                return Err(format!(
                    "XMAP19 particle registration {name:?} resolved {registration_count} times across the effective namespace"
                )
                .into());
            }
            let particle_type = pdxparticle_type(&matches[0].text, &name)
                .ok_or_else(|| format!("XMAP19 particle registration {name:?} has no type"))?;
            needed_types.insert(particle_type.clone());
            rows.push((name, particle_type, matches[0].content.clone()));
        }
        entity_particle_map.insert(
            entity,
            (
                entity_source.content.clone(),
                authored_state.state_name,
                authored_events,
                rows,
            ),
        );
    }

    let asset_sources = if needed_types.is_empty() {
        Vec::new()
    } else {
        admitted_effective_namespace(
            resolver,
            admission,
            "gfx/particles",
            "asset",
            "particle-definition",
        )?
    };
    let mut particle_assets =
        BTreeMap::<String, (String, UsedContentKey, Vec<String>, Vec<String>)>::new();
    let mut needed_shader_effects = BTreeSet::new();
    for particle_type in needed_types {
        let matches = asset_sources
            .iter()
            .filter(|source| particle_asset_defines(&source.text, &particle_type))
            .collect::<Vec<_>>();
        let definition_count = matches
            .iter()
            .map(|source| particle_asset_definition_count(&source.text, &particle_type))
            .sum::<usize>();
        if definition_count != 1 || matches.len() != 1 {
            return Err(format!(
                "XMAP19 particle definition {particle_type:?} resolved {} times",
                definition_count
            )
            .into());
        }
        let source = matches[0];
        let textures = particle_asset_texture_files(&source.text, &particle_type);
        let shader_effects = particle_asset_shader_effects(&source.text, &particle_type);
        if textures.is_empty() {
            return Err(format!("XMAP19 particle {particle_type:?} has no textures").into());
        }
        if shader_effects.is_empty() {
            return Err(format!("XMAP19 particle {particle_type:?} has no shader effects").into());
        }
        needed_shader_effects.extend(shader_effects.iter().cloned());
        particle_assets.insert(
            particle_type,
            (
                source.path.clone(),
                source.content.clone(),
                textures,
                shader_effects,
            ),
        );
    }

    let shader_sources = if needed_shader_effects.is_empty() {
        Vec::new()
    } else {
        admitted_effective_namespace(resolver, admission, "gfx/FX", "shader", "particle-shader")?
    };
    let parsed_shader_sources = shader_sources
        .iter()
        .map(|source| {
            crate::shader_compile::parse_effect_file(&source.text)
                .map(|parsed| (source, parsed))
                .map_err(|error| {
                    format!("XMAP19 parse effective shader {}: {error}", source.path).into()
                })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut shader_path_by_effect = BTreeMap::new();
    for effect in needed_shader_effects {
        let matches = parsed_shader_sources
            .iter()
            .filter(|(_, parsed)| parsed.effect(&effect).is_some())
            .collect::<Vec<_>>();
        let definition_count = matches
            .iter()
            .map(|(_, parsed)| parsed.effect_count(&effect))
            .sum::<usize>();
        if definition_count != 1 || matches.len() != 1 {
            return Err(format!(
                "XMAP19 particle shader Effect {effect:?} resolved {} times across the effective namespace",
                definition_count
            )
            .into());
        }
        let definition = matches[0]
            .1
            .effect(&effect)
            .ok_or("XMAP19 shader effect index drifted")?;
        if definition.vertex_shader.is_none() || definition.pixel_shader.is_none() {
            return Err(format!(
                "XMAP19 particle shader Effect {effect:?} does not define both stages"
            )
            .into());
        }
        shader_path_by_effect.insert(effect, matches[0].0.path.clone());
    }
    let mut shader_closure_by_path = BTreeMap::<String, Vec<UsedContentKey>>::new();
    for path in shader_path_by_effect.values() {
        if shader_closure_by_path.contains_key(path) {
            continue;
        }
        let mut closure = Vec::new();
        for (source_path, _) in
            crate::shader_compile::effect_translation_unit_files(resolver, path)?
        {
            let content = require_content(admission, &source_path)?;
            resolver.read_admitted_used_content(&content)?;
            push_content(&mut closure, content);
        }
        shader_closure_by_path.insert(path.clone(), closure);
    }

    let weather_content = require_content(admission, WEATHER_EFFECTS_SOURCE)?;
    let mut entity_programs = BTreeMap::new();
    for (entity, (entity_definition_source, default_state_name, ordered_events, particles)) in
        entity_particle_map
    {
        let mut asset_paths = Vec::new();
        let mut registration_sources = Vec::new();
        let mut shader_effect_rows = Vec::new();
        let mut shader_path_rows = Vec::new();
        let mut texture_paths = Vec::new();
        let mut ordered_content = vec![weather_content.clone(), entity_definition_source.clone()];
        for (_, particle_type, registration_source) in &particles {
            let (asset_path, asset_content, textures, shader_effects) = particle_assets
                .get(particle_type)
                .ok_or("XMAP19 particle asset index lost a required type")?;
            push_content(&mut ordered_content, registration_source.clone());
            registration_sources.push(registration_source.clone());
            push_content(&mut ordered_content, asset_content.clone());
            asset_paths.push(asset_path.clone());
            for texture in textures {
                push_content(&mut ordered_content, require_content(admission, texture)?);
                if !texture_paths.contains(texture) {
                    texture_paths.push(texture.clone());
                }
            }
            let shader_paths = shader_effects
                .iter()
                .map(|effect| {
                    shader_path_by_effect
                        .get(effect)
                        .cloned()
                        .ok_or_else(|| format!("XMAP19 lost shader path for Effect {effect:?}"))
                })
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            for shader_path in &shader_paths {
                for content in shader_closure_by_path
                    .get(shader_path)
                    .ok_or("XMAP19 lost particle shader translation-unit closure")?
                {
                    push_content(&mut ordered_content, content.clone());
                }
            }
            shader_effect_rows.push(shader_effects.clone());
            shader_path_rows.push(shader_paths);
        }
        let program = PreparedWeatherEntityProgram {
            entity_name: entity.clone(),
            entity_definition_source,
            default_state_name,
            ordered_events,
            ordered_particle_names: particles.iter().map(|row| row.0.clone()).collect(),
            ordered_particle_types: particles.iter().map(|row| row.1.clone()).collect(),
            ordered_particle_registration_sources: registration_sources,
            ordered_particle_asset_paths: asset_paths,
            ordered_particle_shader_effects: shader_effect_rows,
            ordered_particle_shader_paths: shader_path_rows,
            ordered_texture_paths: texture_paths,
            ordered_content,
        };
        program.validate()?;
        entity_programs.insert(entity, program);
    }

    let mut effects = BTreeMap::new();
    for weather in [
        WeatherType::RainLight,
        WeatherType::RainHeavy,
        WeatherType::Snow,
        WeatherType::Blizzard,
        WeatherType::Sandstorm,
    ] {
        let rows = effects_table
            .rows()
            .iter()
            .filter(|(candidate, _)| *candidate == weather)
            .enumerate()
            .map(|(index, (_, binding))| variant_program(index, binding, &entity_programs))
            .collect::<Result<Vec<_>>>()?;
        effects.insert(weather, rows);
    }
    let mut all_content = vec![
        require_content(admission, WEATHER_EFFECTS_SOURCE)?,
        require_content(admission, WEATHER_POSITIONS_SOURCE)?,
        require_content(admission, GRAPHICS_DEFINES_SOURCE)?,
        require_content(admission, FOG_CONSTANTS_SOURCE)?,
    ];
    for rows in effects.values() {
        for row in rows {
            for content in &row.primary.ordered_content {
                push_content(&mut all_content, content.clone());
            }
            if let Some(fading) = &row.fading {
                for content in &fading.ordered_content {
                    push_content(&mut all_content, content.clone());
                }
            }
        }
    }
    all_content.sort_by(|left, right| {
        left.normalized_game_relative_path
            .cmp(&right.normalized_game_relative_path)
    });
    Ok(Xmap19Content {
        ordered_used_content: all_content,
        effects,
        positions,
        defines,
        distance_fog,
    })
}

#[derive(Clone)]
struct AdmittedNamespaceSource {
    path: String,
    content: UsedContentKey,
    text: String,
}

fn admitted_effective_namespace(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    directory: &str,
    extension: &str,
    label: &str,
) -> Result<Vec<AdmittedNamespaceSource>> {
    let prefix = format!("{}/", directory.replace('\\', "/").to_ascii_lowercase());
    let suffix = format!(
        ".{}",
        extension.trim_start_matches('.').to_ascii_lowercase()
    );
    let effective = resolver.audit_effective_dir_files_recursive(directory, Some(extension))?;
    let effective_folded = effective
        .iter()
        .map(|path| path.replace('\\', "/").to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let admitted_folded = admission
        .required_paths()
        .filter_map(|path| {
            let folded = path.replace('\\', "/").to_ascii_lowercase();
            (folded.starts_with(&prefix) && folded.ends_with(&suffix)).then_some(folded)
        })
        .collect::<BTreeSet<_>>();
    if effective_folded != admitted_folded {
        return Err(format!(
            "XMAP19 {label} namespace is not admission-complete (effective {}, admitted {})",
            effective_folded.len(),
            admitted_folded.len()
        )
        .into());
    }
    effective
        .into_iter()
        .map(|path| {
            let content = require_content(admission, &path)?;
            let bytes = resolver.read_admitted_used_content(&content)?;
            let text = crate::latin1_bytes_to_string(&bytes);
            Ok(AdmittedNamespaceSource {
                path,
                content,
                text,
            })
        })
        .collect()
}

fn variant_program(
    index: usize,
    binding: &WeatherEffectBinding,
    entities: &BTreeMap<String, PreparedWeatherEntityProgram>,
) -> Result<VariantProgram> {
    let fading = match &binding.fading_entity {
        Some(entity) => Some(
            entities
                .get(entity)
                .cloned()
                .ok_or_else(|| format!("XMAP19 lost fading entity {entity}"))?,
        ),
        None => None,
    };
    Ok(VariantProgram {
        authored_weather_effect_index: u16::try_from(index)?,
        size: if binding.small {
            WeatherPositionSize::Small
        } else {
            WeatherPositionSize::Big
        },
        primary: entities
            .get(&binding.entity)
            .cloned()
            .ok_or_else(|| format!("XMAP19 lost primary entity {}", binding.entity))?,
        fading,
    })
}

fn prepare_weather(
    context: &FoundationFeatureContext<'_>,
    content: &Xmap19Content,
) -> Result<PreparedXmap19WeatherFrame> {
    let lifecycle = context.world_frame.weather_visual_lifecycle();
    let source_sequence = lifecycle
        .source_sequence
        .ok_or("XMAP19 lifecycle has no source sequence")?;
    let lifecycle_payload_sha256 = lifecycle
        .payload_sha256
        .clone()
        .ok_or("XMAP19 lifecycle has no payload digest")?;
    let camera = context
        .viewer
        .camera
        .as_ref()
        .ok_or("XMAP19 requires XMAP03 viewer-owned camera state")?;
    camera.validate()?;
    let camera_distance = camera.current_eye()[1];
    let visibility = weather_visibility_law(camera_distance, &content.defines)?;

    let mut regions = Vec::with_capacity(305);
    let mut positions = Vec::with_capacity(content.positions.emitters.len());
    for region_id in 0..305u32 {
        let gameplay = context
            .world_frame
            .weather(region_id)
            .ok_or_else(|| format!("XMAP19 D24 omitted strategic region {region_id}"))?;
        let epoch = lifecycle
            .by_region
            .get(&region_id)
            .ok_or_else(|| format!("XMAP19 lifecycle omitted strategic region {region_id}"))?;
        validate_epoch_join(
            gameplay,
            epoch.airborne_runtime_code,
            epoch.transition_expiry_game_hour,
        )?;
        let weather = weather_type_for_code(epoch.airborne_runtime_code);
        let mut seed = epoch.selection_seed_words;
        let mut expected_producer_ids = Vec::new();
        for (ordinal, emitter) in content.positions.emitters.iter().enumerate() {
            if emitter.region_id != region_id {
                continue;
            }
            let selection = if let Some(weather) = weather {
                let variants = content
                    .effects
                    .get(&weather)
                    .ok_or("XMAP19 content omitted an active weather vector")?;
                let has_big = variants
                    .iter()
                    .any(|variant| variant.size == WeatherPositionSize::Big);
                let class = match emitter.size {
                    WeatherPositionSize::Big if has_big => WeatherPositionSize::Big,
                    WeatherPositionSize::Big => WeatherPositionSize::Small,
                    WeatherPositionSize::Small => WeatherPositionSize::Small,
                };
                let candidates = variants
                    .iter()
                    .filter(|variant| variant.size == class)
                    .collect::<Vec<_>>();
                weather_selected_candidate_index(&mut seed, candidates.len())?
                    .map(|index| candidates[index])
                    .map(|variant| -> Result<PreparedWeatherSelection> {
                        let source_ordinal = u32::try_from(ordinal + 1)?;
                        let primary_source_id = weather_producer_instance_id(
                            region_id,
                            epoch.epoch_ordinal,
                            epoch.airborne_runtime_code,
                            source_ordinal,
                            "primary",
                        );
                        expected_producer_ids.push(primary_source_id.clone());
                        let fading_source_id = variant.fading.as_ref().map(|_| {
                            let identity = weather_producer_instance_id(
                                region_id,
                                epoch.epoch_ordinal,
                                epoch.airborne_runtime_code,
                                source_ordinal,
                                "fading",
                            );
                            expected_producer_ids.push(identity.clone());
                            identity
                        });
                        Ok(PreparedWeatherSelection {
                            authored_weather_effect_index: variant.authored_weather_effect_index,
                            primary_source_id,
                            fading_source_id,
                            primary: variant.primary.clone(),
                            fading: variant.fading.clone(),
                        })
                    })
                    .transpose()?
            } else {
                None
            };
            positions.push(PreparedWeatherPosition {
                source_ordinal: u32::try_from(ordinal + 1)?,
                strategic_region_id: region_id,
                position_xyz_f32_bits: emitter.position.map(f32::to_bits),
                size: match emitter.size {
                    WeatherPositionSize::Big => "big",
                    WeatherPositionSize::Small => "small",
                }
                .into(),
                airborne_runtime_code: epoch.airborne_runtime_code,
                weather_effect_key: weather.map(|value| value.key().to_string()),
                selection,
            });
        }
        if expected_producer_ids != epoch.producer_instance_ids {
            return Err(format!(
                "XMAP19 region {region_id} authenticated producer-instance census disagrees with file/seed-derived membership"
            )
            .into());
        }
        regions.push(PreparedWeatherRegionEpoch {
            strategic_region_id: region_id,
            airborne_runtime_code: epoch.airborne_runtime_code,
            transition_expiry_game_hour: epoch.transition_expiry_game_hour,
            epoch_game_hour: epoch.epoch_game_hour,
            epoch_ordinal: epoch.epoch_ordinal,
            selection_seed_words: epoch.selection_seed_words,
            selection_seed_after_positions: seed,
            authenticated_producer_instance_ids: epoch.producer_instance_ids.clone(),
        });
    }
    positions.sort_by_key(|position| position.source_ordinal);
    let mut prepared = PreparedXmap19WeatherFrame {
        schema: XMAP19_WEATHER_SCHEMA.into(),
        hour_index: context.world_frame.hour_index(),
        source_sequence,
        lifecycle_payload_sha256,
        camera_distance_f32_bits: camera_distance.to_bits(),
        primary_visibility_alpha_f32_bits: visibility.primary_alpha.to_bits(),
        fading_visibility_alpha_f32_bits: visibility.fading_alpha.to_bits(),
        playback_rate_f32_bits: visibility.playback_rate.to_bits(),
        distance_fog_color_f32_bits: content.distance_fog.color.map(f32::to_bits),
        distance_fog_begin_f32_bits: content.distance_fog.begin.to_bits(),
        distance_fog_end_f32_bits: content.distance_fog.end.to_bits(),
        distance_fog_max_f32_bits: content.distance_fog.max.to_bits(),
        ordered_used_content: content.ordered_used_content.clone(),
        regions,
        ordered_positions: positions,
        deterministic_sha256: String::new(),
    };
    prepared.deterministic_sha256 = prepared_weather_digest(&prepared)?;
    prepared.validate()?;
    Ok(prepared)
}

fn validate_epoch_join(gameplay: &WeatherRegionRow, code: u8, expiry: i32) -> Result<()> {
    let exact = |leaf: &Leaf<bool>, name: &str| -> Result<bool> {
        match leaf {
            Leaf::Measured(value) => Ok(*value),
            _ => Err(format!("XMAP19 D24 {name} is not measured").into()),
        }
    };
    let flags = [
        (2, exact(&gameplay.rain_light_active, "rain_light")?),
        (3, exact(&gameplay.rain_heavy_active, "rain_heavy")?),
        (4, exact(&gameplay.snow_active, "snow")?),
        (5, exact(&gameplay.blizzard_active, "blizzard")?),
        (7, exact(&gameplay.sandstorm_active, "sandstorm")?),
    ];
    let expected = flags
        .iter()
        .find(|(_, value)| *value)
        .map(|(code, _)| *code)
        .unwrap_or(0);
    let kind = match &gameplay.weather_kind {
        Leaf::Measured(value) => WeatherType::from_recording_kind(value)
            .ok_or("XMAP19 D24 weather kind is not an exact engine key")?,
        _ => return Err("XMAP19 D24 weather kind is not measured".into()),
    };
    if expected != code || weather_type_for_code(code).unwrap_or(WeatherType::NoPhenomenon) != kind
    {
        return Err("XMAP19 D24 and visual epoch airborne code disagree".into());
    }
    match &gameplay.transition_expiry_game_hour {
        Leaf::Measured(value) if *value == i64::from(expiry) => Ok(()),
        _ => Err("XMAP19 D24 and visual epoch transition expiry disagree".into()),
    }
}

fn weather_type_for_code(code: u8) -> Option<WeatherType> {
    match code {
        0 => None,
        2 => Some(WeatherType::RainLight),
        3 => Some(WeatherType::RainHeavy),
        4 => Some(WeatherType::Snow),
        5 => Some(WeatherType::Blizzard),
        7 => Some(WeatherType::Sandstorm),
        _ => None,
    }
}

fn weather_producer_instance_id(
    region_id: u32,
    epoch_ordinal: u64,
    code: u8,
    source_ordinal: u32,
    role: &str,
) -> String {
    format!("xmap19.weather:r{region_id}:e{epoch_ordinal}:c{code}:p{source_ordinal}:{role}")
}

fn admitted_text(
    resolver: &GameFileResolver,
    admission: &ValidatedContentAdmission,
    path: &str,
) -> Result<String> {
    let content = require_content(admission, path)?;
    let bytes = resolver.read_admitted_used_content(&content)?;
    Ok(crate::latin1_bytes_to_string(&bytes))
}

fn require_content(admission: &ValidatedContentAdmission, path: &str) -> Result<UsedContentKey> {
    admission
        .require_used_content(path)
        .map(Clone::clone)
        .map_err(|error| format!("XMAP19 requires admitted {path}: {error}").into())
}

fn push_content(out: &mut Vec<UsedContentKey>, content: UsedContentKey) {
    if !out.iter().any(|existing| {
        existing
            .normalized_game_relative_path
            .eq_ignore_ascii_case(&content.normalized_game_relative_path)
    }) {
        out.push(content);
    }
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn registration_digest() -> String {
    Sha256::hex(
        format!(
            "XMAP19\0{XMAP19_WEATHER_SCHEMA}\0{XMAP19_A729_EXECUTABLE_SHA256}\0{XMAP19_A729_PRODUCER_RVA}"
        )
        .as_bytes(),
    )
}

fn prepared_weather_digest(prepared: &PreparedXmap19WeatherFrame) -> Result<String> {
    let mut unsigned = prepared.clone();
    unsigned.deterministic_sha256.clear();
    Ok(Sha256::hex(&serde_json::to_vec(&unsigned)?))
}

fn owned_gaps(context: &FoundationFeatureContext<'_>) -> Vec<String> {
    WEATHER_AUTHORITY_GAPS
        .iter()
        .filter(|input| {
            context
                .map_authority_gaps
                .iter()
                .any(|gap| gap.input_id == **input)
        })
        .map(|input| (*input).to_string())
        .collect()
}

fn unavailable_gap(context: &FoundationFeatureContext<'_>) -> FoundationGap {
    let gap_id = "XMAP19.weather_inputs_unavailable".to_string();
    let reason = format!(
        "XMAP19 requires Present D24 weather gameplay and the exact presentation-independent {} carrier; selected Present is not an authority",
        replay_core::WEATHER_VISUAL_LIFECYCLE_SCHEMA
    );
    let hour = context.world_frame.hour_index();
    let witness_sha256 = Sha256::hex(
        format!(
            "{gap_id}\n{hour}\n{:?}\n{:?}\n{reason}",
            context.world_frame.weather_status(),
            context.world_frame.weather_visual_lifecycle().status
        )
        .as_bytes(),
    );
    FoundationGap {
        gap_id,
        first_unavailable_hour: hour,
        last_unavailable_hour: hour,
        witness_sha256,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gameplay(kind: &str, code: u8, expiry: i64) -> WeatherRegionRow {
        WeatherRegionRow {
            weather_kind: Leaf::Measured(kind.into()),
            intensity: Leaf::Measured(1.0),
            rain_light_active: Leaf::Measured(code == 2),
            rain_heavy_active: Leaf::Measured(code == 3),
            snow_active: Leaf::Measured(code == 4),
            blizzard_active: Leaf::Measured(code == 5),
            sandstorm_active: Leaf::Measured(code == 7),
            arctic_water_active: Leaf::Measured(false),
            transition_expiry_game_hour: Leaf::Measured(expiry),
            snow_level_0_255: Leaf::Absent,
            mud_level_0_255: Leaf::Absent,
            snow_mud_fade: Leaf::Absent,
            snow_mud_fade_from: Leaf::Absent,
            snow_mud_fade_to: Leaf::Absent,
        }
    }

    #[test]
    fn exact_epoch_join_rejects_kind_code_or_expiry_drift() {
        let rain = gameplay("rain_light", 2, 900);
        validate_epoch_join(&rain, 2, 900).expect("exact join");
        assert!(validate_epoch_join(&rain, 3, 900).is_err());
        assert!(validate_epoch_join(&rain, 2, 901).is_err());
        let alias = gameplay("rain", 2, 900);
        assert!(validate_epoch_join(&alias, 2, 900).is_err());
    }

    #[test]
    fn exact_epoch_join_uses_engine_named_flag_priority() {
        let mut overlap = gameplay("rain_light", 2, 900);
        overlap.rain_heavy_active = Leaf::Measured(true);
        overlap.snow_active = Leaf::Measured(true);
        validate_epoch_join(&overlap, 2, 900).expect("first active flag wins");
        assert!(validate_epoch_join(&overlap, 3, 900).is_err());
    }

    #[test]
    fn selector_consumes_only_nonempty_vectors() {
        let mut seed = Some([7, 11]);
        assert_eq!(
            weather_selected_candidate_index(&mut seed, 0).expect("empty"),
            None
        );
        assert_eq!(seed, Some([7, 11]));
        let expected = crate::weather_selection_word([7, 11]) as usize % 3;
        assert_eq!(
            weather_selected_candidate_index(&mut seed, 3).expect("selected"),
            Some(expected)
        );
        assert_eq!(seed, Some([8, 11]));
    }

    #[test]
    fn effective_namespace_decode_preserves_clausewitz_single_byte_text() {
        let decoded = crate::latin1_bytes_to_string(&[b'n', b'a', b'm', b'e', b'=', 0xe9]);
        assert_eq!(decoded, "name=é");
    }
}
