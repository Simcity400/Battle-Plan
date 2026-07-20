//! Exact effective-VFS weather catalog for the frozen XN14/XN15 producer law.
//!
//! The catalog is built from the bytes selected by the effective VFS. Runtime
//! definition pointers never enter the portable document. A live observation
//! resolves only when the named-factory registration token recorded on every
//! primary/fading entity uniquely matches one authored row. Direct-definition
//! entities with no authenticated token remain unresolved and fail closed.

use super::{
    ExactWeatherCatalogResolver, RawEffectiveVfsWinnerIndexBinding, RawVfsContentIdentity,
    RawWeatherEntityPair, RawWeatherEntityRef, RawWeatherObservation,
    ResolvedWeatherCatalogRow,
};
use portable_content_manifest::{
    normalize_game_relative_path, ContentLayerKind, UsedContentKey,
};
use portable_render_stream::{
    RenderOutputRef, WeatherEntityContentJoin, WeatherInstance, WeatherPosition,
    WeatherPositionClass, WeatherResolution, WeatherVariantContentJoin,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const WEATHER_RESOURCE_CATALOG_SCHEMA: &str =
    "extractor.xn14-effective-vfs-weather-resource-catalog.v1";

const WEATHER_SOURCE: &str = "common/weather.txt";
const WEATHER_ENTITIES_SOURCE: &str = "gfx/entities/weather_entities.asset";
const PARTICLE_REGISTRATIONS_SOURCE: &str = "gfx/entities/particles.gfx";
const PARTICLE_SHADER_SOURCE: &str = "gfx/fx/particle.shader";
const WEATHER_POSITIONS_SOURCE: &str = "map/weatherpositions.txt";

/// One exact effective-VFS payload supplied to the catalog builder. The bytes
/// are re-hashed before parsing; a matching path or metadata row is not enough.
#[derive(Clone, Debug)]
pub struct WeatherCatalogSourcePayload {
    pub content: UsedContentKey,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WeatherCatalogPayloadRequest {
    pub content: UsedContentKey,
    pub byte_offset: u64,
    pub byte_count: u64,
    pub range_sha256: String,
}

/// Minimal adapter implemented by the selected-Present admission owner. A
/// payload is useful only together with its finalized `UsedContentKey`; the
/// resolver never opens an install path or accepts an inventory row as bytes.
pub trait AdmittedWeatherCatalogPayloads {
    fn payload(&self, content: &UsedContentKey) -> Option<&[u8]>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherCatalogPosition {
    pub source_line: u32,
    pub strategic_region_id: u32,
    pub position_xyz_f32_bits: [u32; 3],
    pub class: WeatherPositionClass,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherCodeResourceEntry {
    pub airborne_runtime_code: u8,
    pub weather_effect_token: Option<String>,
    pub ordered_variants: Vec<WeatherVariantContentJoin>,
}

/// Canonical static-bundle artifact. `catalog_sha256` covers the schema,
/// effective-VFS authority, complete code table, position source, all authored
/// variants and all ordered positions. It never contains a machine path or a
/// process/capture-local pointer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherResourceCatalogProvenance {
    pub schema: String,
    pub authority_manifest_sha256: String,
    pub weather_position_source_content: UsedContentKey,
    pub weather_codes: Vec<WeatherCodeResourceEntry>,
    pub ordered_positions: Vec<WeatherCatalogPosition>,
    pub catalog_sha256: String,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedWeatherCatalog {
    provenance: WeatherResourceCatalogProvenance,
    codes: BTreeMap<u8, WeatherCodeResourceEntry>,
    positions_by_region: BTreeMap<u32, Vec<WeatherCatalogPosition>>,
}

impl AuthenticatedWeatherCatalog {
    /// First-phase full-payload requests. The fixed XN14 roots are requested
    /// together with particle-definition assets and shader include candidates
    /// solely to discover the exact referenced closure. Unreferenced discovery
    /// files never enter the catalog provenance or portable replay.
    pub fn discovery_payload_requests(
        winner_index: &RawEffectiveVfsWinnerIndexBinding,
    ) -> Result<Vec<WeatherCatalogPayloadRequest>, String> {
        winner_index.validate()?;
        let mut paths = [
            WEATHER_SOURCE,
            WEATHER_ENTITIES_SOURCE,
            PARTICLE_REGISTRATIONS_SOURCE,
            PARTICLE_SHADER_SOURCE,
            WEATHER_POSITIONS_SOURCE,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
        paths.extend(
            winner_index
                .ordered_rows
                .iter()
                .map(|row| row.content.logical_path.as_str())
                .filter(|path| {
                    (path.starts_with("gfx/particles/") && path.ends_with(".asset"))
                        || (path.starts_with("gfx/fx/") && path.ends_with(".fxh"))
                })
                .map(str::to_owned),
        );
        payload_requests_for_paths(winner_index, paths)
    }

    /// Second-phase exact closure requests. Call only after every discovery
    /// request is admitted. The result removes unrelated discovery assets and
    /// adds only referenced particle definitions, textures, the particle
    /// shader, and its authored include closure.
    pub fn exact_payload_requests(
        winner_index: &RawEffectiveVfsWinnerIndexBinding,
        admitted: &impl AdmittedWeatherCatalogPayloads,
    ) -> Result<Vec<WeatherCatalogPayloadRequest>, String> {
        let discovery = Self::discovery_payload_requests(winner_index)?;
        let discovery_sources = sources_from_requests(&discovery, admitted)?;
        let paths = discover_exact_dependency_paths(&discovery_sources)?;
        payload_requests_for_paths(winner_index, paths)
    }

    /// Construct from the exact second-phase request set. Missing or partial
    /// admitted payloads reject; no install-root or winner-path fallback is
    /// available at this boundary.
    pub fn from_admitted_payloads(
        winner_index: &RawEffectiveVfsWinnerIndexBinding,
        admitted: &impl AdmittedWeatherCatalogPayloads,
    ) -> Result<Self, String> {
        let requests = Self::exact_payload_requests(winner_index, admitted)?;
        let sources = sources_from_requests(&requests, admitted)?;
        Self::from_effective_vfs_payloads(
            winner_index.authority_manifest_sha256.clone(),
            sources,
        )
    }

    /// Build the exact catalog from effective-VFS winners and their consumed
    /// payloads. Callers may provide a larger inventory; only the closed
    /// weather dependency graph enters the portable provenance document.
    pub fn from_effective_vfs_payloads(
        authority_manifest_sha256: String,
        sources: impl IntoIterator<Item = WeatherCatalogSourcePayload>,
    ) -> Result<Self, String> {
        require_sha256(&authority_manifest_sha256, "weather authority manifest")?;
        let mut payloads = BTreeMap::<String, WeatherCatalogSourcePayload>::new();
        for mut source in sources {
            source.content.validate()?;
            if source.content.effective_vfs_manifest_sha256 != authority_manifest_sha256 {
                return Err("weather source belongs to a different effective VFS".into());
            }
            let normalized = normalize_game_relative_path(
                &source.content.normalized_game_relative_path,
            )?;
            if normalized != source.content.normalized_game_relative_path {
                return Err("weather source path is not canonical".into());
            }
            if source.content.payload_bytes != source.payload.len() as u64
                || source.content.payload_sha256 != sha256(&source.payload)
            {
                return Err(format!(
                    "weather source payload disagrees with UsedContentKey: {normalized}"
                ));
            }
            source.content.normalized_game_relative_path = normalized.clone();
            if payloads.insert(normalized.clone(), source).is_some() {
                return Err(format!("weather source repeats effective path {normalized}"));
            }
        }

        let weather_text = required_clausewitz_text(&payloads, WEATHER_SOURCE)?;
        let entities_text = required_clausewitz_text(&payloads, WEATHER_ENTITIES_SOURCE)?;
        let registrations_text =
            required_clausewitz_text(&payloads, PARTICLE_REGISTRATIONS_SOURCE)?;
        required_source(&payloads, PARTICLE_SHADER_SOURCE)?;
        let positions_text = required_utf8(&payloads, WEATHER_POSITIONS_SOURCE)?;

        let authored = parse_weather_effects(&weather_text)?;
        let definitions = index_particle_definitions(&payloads)?;
        let mut codes = Vec::with_capacity(6);
        codes.push(WeatherCodeResourceEntry {
            airborne_runtime_code: 0,
            weather_effect_token: None,
            ordered_variants: Vec::new(),
        });
        for (runtime_code, weather_token) in [
            (2, "rain_light"),
            (3, "rain_heavy"),
            (4, "snow"),
            (5, "blizzard"),
            (7, "sandstorm"),
        ] {
            let rows = authored.get(weather_token).ok_or_else(|| {
                format!("common/weather.txt has no authored {weather_token} effects")
            })?;
            if rows.is_empty() {
                return Err(format!("weather effect vector {weather_token} is empty"));
            }
            let mut variants = Vec::with_capacity(rows.len());
            for row in rows {
                let primary = resolve_entity_content(
                    &payloads,
                    &entities_text,
                    &registrations_text,
                    &definitions,
                    &row.primary_entity,
                )?;
                let fading = row
                    .fading_entity
                    .as_deref()
                    .map(|token| {
                        resolve_entity_content(
                            &payloads,
                            &entities_text,
                            &registrations_text,
                            &definitions,
                            token,
                        )
                    })
                    .transpose()?;
                variants.push(WeatherVariantContentJoin {
                    authored_effect_index: row.authored_effect_index,
                    class: row.class,
                    resolved_variant_token: variant_token(
                        weather_token,
                        row.authored_effect_index,
                        row.class,
                    ),
                    primary,
                    fading,
                });
            }
            codes.push(WeatherCodeResourceEntry {
                airborne_runtime_code: runtime_code,
                weather_effect_token: Some(weather_token.into()),
                ordered_variants: variants,
            });
        }

        let ordered_positions = parse_weather_positions(&positions_text)?;
        let position_content = required_source(&payloads, WEATHER_POSITIONS_SOURCE)?
            .content
            .clone();
        let catalog_sha256 = catalog_sha256(
            &authority_manifest_sha256,
            &position_content,
            &codes,
            &ordered_positions,
        )?;
        Self::from_provenance(WeatherResourceCatalogProvenance {
            schema: WEATHER_RESOURCE_CATALOG_SCHEMA.into(),
            authority_manifest_sha256,
            weather_position_source_content: position_content,
            weather_codes: codes,
            ordered_positions,
            catalog_sha256,
        })
    }

    /// Decode a static-bundle catalog. Every content key and the complete
    /// catalog digest is revalidated before the resolver becomes usable.
    pub fn decode_authenticated(
        bytes: &[u8],
        expected_artifact_sha256: &str,
        expected_authority_manifest_sha256: &str,
    ) -> Result<Self, String> {
        require_sha256(
            expected_artifact_sha256,
            "weather static-bundle artifact",
        )?;
        require_sha256(
            expected_authority_manifest_sha256,
            "weather expected authority manifest",
        )?;
        if sha256(bytes) != expected_artifact_sha256 {
            return Err("weather static-bundle artifact SHA-256 is invalid".into());
        }
        let provenance: WeatherResourceCatalogProvenance = serde_json::from_slice(bytes)
            .map_err(|error| format!("parse weather resource catalog: {error}"))?;
        if provenance.authority_manifest_sha256 != expected_authority_manifest_sha256 {
            return Err("weather catalog authority differs from the pinned launch authority".into());
        }
        let canonical = serde_json::to_vec(&provenance)
            .map_err(|error| format!("re-encode weather resource catalog: {error}"))?;
        if canonical != bytes {
            return Err("weather static-bundle catalog is not canonical JSON".into());
        }
        Self::from_provenance(provenance)
    }

    pub fn provenance(&self) -> &WeatherResourceCatalogProvenance {
        &self.provenance
    }

    pub fn canonical_provenance_json(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&self.provenance)
            .map_err(|error| format!("encode weather resource catalog: {error}"))
    }

    fn from_provenance(
        provenance: WeatherResourceCatalogProvenance,
    ) -> Result<Self, String> {
        validate_provenance(&provenance)?;
        let codes = provenance
            .weather_codes
            .iter()
            .cloned()
            .map(|row| (row.airborne_runtime_code, row))
            .collect();
        let mut positions_by_region = BTreeMap::<u32, Vec<WeatherCatalogPosition>>::new();
        for position in &provenance.ordered_positions {
            positions_by_region
                .entry(position.strategic_region_id)
                .or_default()
                .push(position.clone());
        }
        Ok(Self {
            provenance,
            codes,
            positions_by_region,
        })
    }

    fn resolve_exact(&self, raw: &RawWeatherObservation) -> Option<ResolvedWeatherCatalogRow> {
        let code = self.codes.get(&raw.airborne_runtime_code)?;
        let positions = self
            .positions_by_region
            .get(&raw.strategic_region_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if positions.len() > u16::MAX as usize {
            return None;
        }

        if raw.airborne_runtime_code == 0 && !raw.ordered_live_pairs.is_empty() {
            return None;
        }
        let mut live_pairs = raw.ordered_live_pairs.iter();
        let mut selection_seed = raw.selection_seed_words;
        let mut resolved_positions = Vec::with_capacity(positions.len());
        let mut instances = Vec::new();
        let mut successful_factory = raw
            .ordered_factory_attempts
            .iter()
            .filter_map(|attempt| {
                attempt.returned_entity_instance_id.map(|id| {
                    (attempt.definition_ptr, id)
                })
            });

        for (ordinal, position) in positions.iter().enumerate() {
            let candidates = code
                .ordered_variants
                .iter()
                .filter(|variant| variant.class == position.class)
                .collect::<Vec<_>>();
            let selected = if raw.airborne_runtime_code == 0 {
                None
            } else {
                let seed = selection_seed.as_mut()?;
                let selection_word = super::weather_selection_word(*seed);
                seed[0] = seed[0].wrapping_add(1);
                (!candidates.is_empty())
                    .then(|| candidates[selection_word as usize % candidates.len()])
            };
            let Some(selected) = selected else {
                resolved_positions.push(explicit_empty_position(ordinal, position)?);
                continue;
            };

            let pair = live_pairs.next()?;
            if !variant_matches_pair(pair, selected) {
                return None;
            }
            if !factory_sequence_matches(pair, &mut successful_factory) {
                return None;
            }
            let mut effect_ids = Vec::new();
            append_effect_ids(pair.primary.as_ref(), &mut effect_ids);
            append_effect_ids(pair.fading.as_ref(), &mut effect_ids);
            for effect_instance_id in &effect_ids {
                instances.push(WeatherInstance {
                    effect_instance_id: *effect_instance_id,
                    position_ordinal: ordinal as u16,
                    class: position.class,
                    resolved_variant_token: selected.resolved_variant_token.clone(),
                    submitted_outputs: Vec::<RenderOutputRef>::new(),
                });
            }
            resolved_positions.push(WeatherPosition {
                position_ordinal: ordinal as u16,
                source_line: position.source_line,
                position_xyz_f32_bits: position.position_xyz_f32_bits,
                class: position.class,
                resolution: WeatherResolution::Instantiated,
                selected_authored_effect_index: Some(selected.authored_effect_index),
                primary_entity_instance_id: pair
                    .primary
                    .as_ref()
                    .map(|entity| entity.entity_instance_id.to_string()),
                fading_entity_instance_id: pair
                    .fading
                    .as_ref()
                    .map(|entity| entity.entity_instance_id.to_string()),
                ordered_effect_instance_ids: effect_ids
                    .iter()
                    .map(u64::to_string)
                    .collect(),
                resolved_variant_token: Some(selected.resolved_variant_token.clone()),
            });
        }
        if live_pairs.next().is_some() || successful_factory.next().is_some() {
            return None;
        }

        Some(ResolvedWeatherCatalogRow {
            weather_effect_token: code.weather_effect_token.clone(),
            weather_position_source_content: self
                .provenance
                .weather_position_source_content
                .clone(),
            ordered_variant_content: code.ordered_variants.clone(),
            ordered_positions: resolved_positions,
            ordered_weather_instances: instances,
        })
    }
}

impl ExactWeatherCatalogResolver for AuthenticatedWeatherCatalog {
    fn authority_manifest_sha256(&self) -> &str {
        &self.provenance.authority_manifest_sha256
    }

    fn resolve(&self, raw: &RawWeatherObservation) -> Option<ResolvedWeatherCatalogRow> {
        self.resolve_exact(raw)
    }
}

fn payload_requests_for_paths(
    winner_index: &RawEffectiveVfsWinnerIndexBinding,
    paths: impl IntoIterator<Item = String>,
) -> Result<Vec<WeatherCatalogPayloadRequest>, String> {
    winner_index.validate()?;
    paths
        .into_iter()
        .map(|path| {
            let normalized = normalize_game_relative_path(&path)?;
            let row = winner_index
                .ordered_rows
                .binary_search_by(|row| {
                    row.content
                        .logical_path
                        .as_str()
                        .cmp(normalized.as_str())
                })
                .ok()
                .map(|index| &winner_index.ordered_rows[index])
                .ok_or_else(|| {
                    format!("weather dependency has no effective-VFS winner: {normalized}")
                })?;
            let content = used_content_from_winner(winner_index, &row.content)?;
            Ok(WeatherCatalogPayloadRequest {
                byte_offset: 0,
                byte_count: content.payload_bytes,
                range_sha256: content.payload_sha256.clone(),
                content,
            })
        })
        .collect()
}

fn used_content_from_winner(
    winner_index: &RawEffectiveVfsWinnerIndexBinding,
    raw: &RawVfsContentIdentity,
) -> Result<UsedContentKey, String> {
    winner_index.authorize_content(raw)?;
    let logical_path = normalize_game_relative_path(&raw.logical_path)?;
    if logical_path != raw.logical_path {
        return Err("weather winner logical path is not canonical".into());
    }
    let layer_kind = match raw.layer_kind.as_str() {
        "base" => ContentLayerKind::Base,
        "integrated_dlc" => ContentLayerKind::IntegratedDlc,
        "dlc" => ContentLayerKind::Dlc,
        "mod" => ContentLayerKind::Mod,
        _ => return Err("weather winner has an unsupported layer kind".into()),
    };
    let content = UsedContentKey {
        effective_vfs_manifest_sha256: winner_index.authority_manifest_sha256.clone(),
        normalized_game_relative_path: logical_path,
        winning_layer_ordinal: raw.winning_layer_ordinal,
        layer_kind,
        layer_id: raw.layer_id.clone(),
        source_sha256: raw.source_sha256.clone(),
        payload_sha256: raw.payload_sha256.clone(),
        payload_bytes: raw.payload_bytes,
    };
    content.validate()?;
    Ok(content)
}

fn sources_from_requests(
    requests: &[WeatherCatalogPayloadRequest],
    admitted: &impl AdmittedWeatherCatalogPayloads,
) -> Result<Vec<WeatherCatalogSourcePayload>, String> {
    requests
        .iter()
        .map(|request| {
            if request.byte_offset != 0
                || request.byte_count != request.content.payload_bytes
                || request.range_sha256 != request.content.payload_sha256
            {
                return Err("weather catalog request is not an exact full payload".into());
            }
            let payload = admitted
                .payload(&request.content)
                .ok_or_else(|| {
                    format!(
                        "weather catalog payload was not admitted: {}",
                        request.content.normalized_game_relative_path
                    )
                })?
                .to_vec();
            if payload.len() as u64 != request.byte_count || sha256(&payload) != request.range_sha256
            {
                return Err(format!(
                    "weather catalog admitted payload changed: {}",
                    request.content.normalized_game_relative_path
                ));
            }
            Ok(WeatherCatalogSourcePayload {
                content: request.content.clone(),
                payload,
            })
        })
        .collect()
}

fn discover_exact_dependency_paths(
    discovery_sources: &[WeatherCatalogSourcePayload],
) -> Result<BTreeSet<String>, String> {
    let payloads = discovery_sources
        .iter()
        .cloned()
        .map(|source| {
            (
                source.content.normalized_game_relative_path.clone(),
                source,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let weather_text = required_clausewitz_text(&payloads, WEATHER_SOURCE)?;
    let entities_text = required_clausewitz_text(&payloads, WEATHER_ENTITIES_SOURCE)?;
    let registrations_text =
        required_clausewitz_text(&payloads, PARTICLE_REGISTRATIONS_SOURCE)?;
    let authored = parse_weather_effects(&weather_text)?;
    let definitions = index_particle_definitions(&payloads)?;
    let mut paths = [
        WEATHER_SOURCE,
        WEATHER_ENTITIES_SOURCE,
        PARTICLE_REGISTRATIONS_SOURCE,
        PARTICLE_SHADER_SOURCE,
        WEATHER_POSITIONS_SOURCE,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let mut shader_tokens = BTreeSet::new();
    for token in ["rain_light", "rain_heavy", "snow", "blizzard", "sandstorm"] {
        let rows = authored
            .get(token)
            .ok_or_else(|| format!("common/weather.txt has no authored {token} effects"))?;
        if rows.is_empty() {
            return Err(format!("weather effect vector {token} is empty"));
        }
        for row in rows {
            discover_entity_paths(
                &entities_text,
                &registrations_text,
                &definitions,
                &row.primary_entity,
                &mut paths,
                &mut shader_tokens,
            )?;
            if let Some(fading) = &row.fading_entity {
                discover_entity_paths(
                    &entities_text,
                    &registrations_text,
                    &definitions,
                    fading,
                    &mut paths,
                    &mut shader_tokens,
                )?;
            }
        }
    }
    paths.extend(particle_shader_closure(&payloads, &shader_tokens)?);
    Ok(paths)
}

fn discover_entity_paths(
    entities_text: &str,
    registrations_text: &str,
    definitions: &BTreeMap<String, Vec<ParticleDefinition>>,
    entity_token: &str,
    paths: &mut BTreeSet<String>,
    shader_tokens: &mut BTreeSet<String>,
) -> Result<(), String> {
    let entity_blocks = named_blocks(entities_text, "entity", entity_token);
    if entity_blocks.len() != 1 {
        return Err(format!(
            "weather entity {entity_token:?} resolved {} times",
            entity_blocks.len()
        ));
    }
    let particle_tokens = event_particle_tokens(&entity_blocks[0])?;
    if particle_tokens.is_empty() {
        return Err(format!("weather entity {entity_token:?} has no particle event"));
    }
    for particle_token in particle_tokens {
        let registrations = named_blocks(registrations_text, "pdxparticle", &particle_token);
        if registrations.len() != 1 {
            return Err(format!(
                "weather particle registration {particle_token:?} resolved {} times",
                registrations.len()
            ));
        }
        let definition_token = unique_scalar_field(&registrations[0], "type")?.ok_or_else(|| {
            format!("weather particle registration {particle_token:?} has no type")
        })?;
        let matched = definitions
            .get(&definition_token)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if matched.len() != 1 {
            return Err(format!(
                "weather particle definition {definition_token:?} resolved {} times",
                matched.len()
            ));
        }
        paths.insert(matched[0].source_path.clone());
        let textures = particle_textures(&matched[0].body)?;
        if textures.is_empty() {
            return Err(format!(
                "weather particle definition {definition_token:?} has no texture"
            ));
        }
        for (texture, shader) in textures {
            paths.insert(normalize_game_relative_path(&texture)?);
            if shader.trim().is_empty() {
                return Err(format!(
                    "weather particle definition {definition_token:?} has a blank shader"
                ));
            }
            shader_tokens.insert(shader);
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct AuthoredWeatherRow {
    authored_effect_index: u16,
    class: WeatherPositionClass,
    primary_entity: String,
    fading_entity: Option<String>,
}

#[derive(Clone, Debug)]
struct ParticleDefinition {
    source_path: String,
    body: String,
}

fn validate_provenance(value: &WeatherResourceCatalogProvenance) -> Result<(), String> {
    if value.schema != WEATHER_RESOURCE_CATALOG_SCHEMA {
        return Err("weather resource catalog schema is unsupported".into());
    }
    require_sha256(
        &value.authority_manifest_sha256,
        "weather authority manifest",
    )?;
    validate_content(
        &value.weather_position_source_content,
        &value.authority_manifest_sha256,
    )?;
    if value
        .weather_position_source_content
        .normalized_game_relative_path
        != WEATHER_POSITIONS_SOURCE
    {
        return Err("weather position source is not map/weatherpositions.txt".into());
    }
    let expected_codes = [0, 2, 3, 4, 5, 7];
    if value.weather_codes.len() != expected_codes.len() {
        return Err("weather resource catalog code count is not exact".into());
    }
    for (row, expected_code) in value.weather_codes.iter().zip(expected_codes) {
        if row.airborne_runtime_code != expected_code
            || row.weather_effect_token.as_deref() != weather_token(expected_code)
        {
            return Err("weather resource catalog code/token order is not exact".into());
        }
        if expected_code == 0 && !row.ordered_variants.is_empty() {
            return Err("weather code zero contains authored variants".into());
        }
        if expected_code != 0 && row.ordered_variants.is_empty() {
            return Err("nonzero weather code has no authored variants".into());
        }
        let mut variant_keys = BTreeSet::new();
        for (expected_index, variant) in row.ordered_variants.iter().enumerate() {
            if usize::from(variant.authored_effect_index) != expected_index
                || !variant_keys.insert((variant.class, variant.authored_effect_index))
                || variant.resolved_variant_token
                    != variant_token(
                        row.weather_effect_token.as_deref().unwrap_or(""),
                        variant.authored_effect_index,
                        variant.class,
                    )
            {
                return Err("weather authored variant identity is duplicated or invalid".into());
            }
            validate_entity_content(
                &variant.primary,
                &value.authority_manifest_sha256,
            )?;
            if let Some(fading) = &variant.fading {
                validate_entity_content(fading, &value.authority_manifest_sha256)?;
            }
        }
    }
    let mut last_line = 0;
    for position in &value.ordered_positions {
        if position.source_line <= last_line
            || position.strategic_region_id == 0
            || position
                .position_xyz_f32_bits
                .iter()
                .any(|bits| !f32::from_bits(*bits).is_finite())
        {
            return Err("weather position source/order/XYZ identity is invalid".into());
        }
        last_line = position.source_line;
    }
    let expected = catalog_sha256(
        &value.authority_manifest_sha256,
        &value.weather_position_source_content,
        &value.weather_codes,
        &value.ordered_positions,
    )?;
    if value.catalog_sha256 != expected {
        return Err("weather resource catalog SHA-256 is invalid".into());
    }
    Ok(())
}

fn validate_entity_content(
    entity: &WeatherEntityContentJoin,
    authority: &str,
) -> Result<(), String> {
    if entity.entity_token.trim().is_empty() || entity.ordered_content.is_empty() {
        return Err("weather entity content closure is empty".into());
    }
    let mut paths = BTreeSet::new();
    for content in &entity.ordered_content {
        validate_content(content, authority)?;
        if !paths.insert(content.normalized_game_relative_path.as_str()) {
            return Err("weather entity content closure repeats a path".into());
        }
    }
    for required in [
        WEATHER_SOURCE,
        WEATHER_ENTITIES_SOURCE,
        PARTICLE_REGISTRATIONS_SOURCE,
        PARTICLE_SHADER_SOURCE,
    ] {
        if !paths.contains(required) {
            return Err(format!(
                "weather entity content closure omits required source {required}"
            ));
        }
    }
    Ok(())
}

fn validate_content(content: &UsedContentKey, authority: &str) -> Result<(), String> {
    content.validate()?;
    if content.effective_vfs_manifest_sha256 != authority {
        return Err("weather content belongs to a different effective VFS".into());
    }
    Ok(())
}

fn catalog_sha256(
    authority: &str,
    position_source: &UsedContentKey,
    codes: &[WeatherCodeResourceEntry],
    positions: &[WeatherCatalogPosition],
) -> Result<String, String> {
    let bytes = serde_json::to_vec(&(
        WEATHER_RESOURCE_CATALOG_SCHEMA,
        authority,
        position_source,
        codes,
        positions,
    ))
    .map_err(|error| format!("encode weather catalog digest body: {error}"))?;
    Ok(sha256(&bytes))
}

fn weather_token(code: u8) -> Option<&'static str> {
    match code {
        0 => None,
        2 => Some("rain_light"),
        3 => Some("rain_heavy"),
        4 => Some("snow"),
        5 => Some("blizzard"),
        7 => Some("sandstorm"),
        _ => None,
    }
}

fn variant_token(weather: &str, index: u16, class: WeatherPositionClass) -> String {
    let class = match class {
        WeatherPositionClass::Big => "big",
        WeatherPositionClass::Small => "small",
    };
    // XN14V freezes the portable identity as weather:class:authored-index.
    // This is an identity join, not presentation text; changing delimiter or
    // component order would make production catalogs disagree with retained
    // membership/checkpoint fixtures despite selecting the same authored row.
    format!("{weather}:{class}:{index}")
}

fn explicit_empty_position(
    ordinal: usize,
    source: &WeatherCatalogPosition,
) -> Option<WeatherPosition> {
    Some(WeatherPosition {
        position_ordinal: u16::try_from(ordinal).ok()?,
        source_line: source.source_line,
        position_xyz_f32_bits: source.position_xyz_f32_bits,
        class: source.class,
        resolution: WeatherResolution::ExplicitEmpty,
        selected_authored_effect_index: None,
        primary_entity_instance_id: None,
        fading_entity_instance_id: None,
        ordered_effect_instance_ids: Vec::new(),
        resolved_variant_token: None,
    })
}

fn variant_matches_pair(
    pair: &RawWeatherEntityPair,
    candidate: &WeatherVariantContentJoin,
) -> bool {
    entity_matches(pair.primary.as_ref(), Some(&candidate.primary))
        && entity_matches(pair.fading.as_ref(), candidate.fading.as_ref())
}

fn entity_matches(
    raw: Option<&RawWeatherEntityRef>,
    expected: Option<&WeatherEntityContentJoin>,
) -> bool {
    match (raw, expected) {
        (None, None) => true,
        (Some(raw), Some(expected)) => raw
            .registration_token
            .as_deref()
            .is_some_and(|token| token == expected.entity_token),
        _ => false,
    }
}

fn factory_sequence_matches<'a>(
    pair: &RawWeatherEntityPair,
    successful: &mut impl Iterator<Item = (usize, u64)>,
) -> bool {
    for entity in [pair.primary.as_ref(), pair.fading.as_ref()]
        .into_iter()
        .flatten()
    {
        if successful.next() != Some((entity.definition_ptr, entity.entity_instance_id)) {
            return false;
        }
    }
    true
}

fn append_effect_ids(entity: Option<&RawWeatherEntityRef>, out: &mut Vec<u64>) {
    if let Some(entity) = entity {
        out.extend(entity.ordered_effect_instance_ids.iter().copied());
    }
}

fn resolve_entity_content(
    payloads: &BTreeMap<String, WeatherCatalogSourcePayload>,
    entities_text: &str,
    registrations_text: &str,
    definitions: &BTreeMap<String, Vec<ParticleDefinition>>,
    entity_token: &str,
) -> Result<WeatherEntityContentJoin, String> {
    let entity_blocks = named_blocks(entities_text, "entity", entity_token);
    if entity_blocks.len() != 1 {
        return Err(format!(
            "weather entity {entity_token:?} resolved {} times",
            entity_blocks.len()
        ));
    }
    let particle_tokens = event_particle_tokens(&entity_blocks[0])?;
    if particle_tokens.is_empty() {
        return Err(format!("weather entity {entity_token:?} has no particle event"));
    }

    let mut ordered_paths = Vec::<String>::new();
    let mut shader_tokens = BTreeSet::<String>::new();
    push_unique_path(&mut ordered_paths, WEATHER_SOURCE);
    push_unique_path(&mut ordered_paths, WEATHER_ENTITIES_SOURCE);
    push_unique_path(&mut ordered_paths, PARTICLE_REGISTRATIONS_SOURCE);
    for particle_token in particle_tokens {
        let registrations = named_blocks(registrations_text, "pdxparticle", &particle_token);
        if registrations.len() != 1 {
            return Err(format!(
                "weather particle registration {particle_token:?} resolved {} times",
                registrations.len()
            ));
        }
        let definition_token = unique_scalar_field(&registrations[0], "type")?.ok_or_else(|| {
            format!("weather particle registration {particle_token:?} has no type")
        })?;
        let matched = definitions.get(&definition_token).map(Vec::as_slice).unwrap_or(&[]);
        if matched.len() != 1 {
            return Err(format!(
                "weather particle definition {definition_token:?} resolved {} times",
                matched.len()
            ));
        }
        let definition = &matched[0];
        push_unique_path(&mut ordered_paths, &definition.source_path);
        let textures = particle_textures(&definition.body)?;
        if textures.is_empty() {
            return Err(format!(
                "weather particle definition {definition_token:?} has no texture"
            ));
        }
        for (texture, shader) in textures {
            if shader.trim().is_empty() {
                return Err(format!(
                    "weather particle definition {definition_token:?} has a blank shader"
                ));
            }
            let normalized = normalize_game_relative_path(&texture)?;
            required_source(payloads, &normalized)?;
            push_unique_path(&mut ordered_paths, &normalized);
            shader_tokens.insert(shader);
        }
    }
    for path in particle_shader_closure(payloads, &shader_tokens)? {
        push_unique_path(&mut ordered_paths, &path);
    }
    let ordered_content = ordered_paths
        .iter()
        .map(|path| Ok(required_source(payloads, path)?.content.clone()))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(WeatherEntityContentJoin {
        entity_token: entity_token.into(),
        ordered_content,
    })
}

fn required_source<'a>(
    payloads: &'a BTreeMap<String, WeatherCatalogSourcePayload>,
    path: &str,
) -> Result<&'a WeatherCatalogSourcePayload, String> {
    payloads
        .get(path)
        .ok_or_else(|| format!("weather catalog is missing effective source {path}"))
}

fn required_utf8(
    payloads: &BTreeMap<String, WeatherCatalogSourcePayload>,
    path: &str,
) -> Result<String, String> {
    std::str::from_utf8(&required_source(payloads, path)?.payload)
        .map(str::to_owned)
        .map_err(|error| format!("weather source {path} is not UTF-8/ASCII: {error}"))
}

fn required_clausewitz_text(
    payloads: &BTreeMap<String, WeatherCatalogSourcePayload>,
    path: &str,
) -> Result<String, String> {
    strip_clausewitz_comments(&required_utf8(payloads, path)?, path)
}

fn parse_weather_positions(source: &str) -> Result<Vec<WeatherCatalogPosition>, String> {
    let mut out = Vec::new();
    for (line_index, raw) in source.lines().enumerate() {
        let source_line = u32::try_from(line_index + 1)
            .map_err(|_| "weather position source line overflows u32")?;
        let row = raw.split('#').next().unwrap_or("").trim();
        if row.is_empty() {
            continue;
        }
        let fields = row.split(';').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 5 || fields.iter().any(|field| field.is_empty()) {
            return Err(format!(
                "map/weatherpositions.txt line {source_line}: expected five fields"
            ));
        }
        let strategic_region_id = fields[0].parse::<u32>().map_err(|error| {
            format!("map/weatherpositions.txt line {source_line}: bad region: {error}")
        })?;
        if strategic_region_id == 0 {
            return Err(format!(
                "map/weatherpositions.txt line {source_line}: region zero is invalid"
            ));
        }
        let mut xyz = [0u32; 3];
        for (index, raw) in fields[1..4].iter().enumerate() {
            let value = raw.parse::<f32>().map_err(|error| {
                format!("map/weatherpositions.txt line {source_line}: bad XYZ: {error}")
            })?;
            if !value.is_finite() {
                return Err(format!(
                    "map/weatherpositions.txt line {source_line}: XYZ is non-finite"
                ));
            }
            xyz[index] = value.to_bits();
        }
        let class = match fields[4] {
            "big" => WeatherPositionClass::Big,
            "small" => WeatherPositionClass::Small,
            other => {
                return Err(format!(
                    "map/weatherpositions.txt line {source_line}: unknown class {other:?}"
                ))
            }
        };
        out.push(WeatherCatalogPosition {
            source_line,
            strategic_region_id,
            position_xyz_f32_bits: xyz,
            class,
        });
    }
    if out.is_empty() {
        return Err("map/weatherpositions.txt contains no positions".into());
    }
    Ok(out)
}

fn parse_weather_effects(
    source: &str,
) -> Result<BTreeMap<String, Vec<AuthoredWeatherRow>>, String> {
    let block = braced_value(source, "weather_effects")
        .ok_or("common/weather.txt has no weather_effects block")?;
    let mut out = BTreeMap::new();
    let mut rest = block.as_str();
    while let Some((token, body, after)) = next_braced_entry(rest) {
        if ["rain_light", "rain_heavy", "snow", "blizzard", "sandstorm"]
            .contains(&token.as_str())
        {
            let mut rows = Vec::new();
            for (index, effect) in repeated_braced_values(&body, "weather_effect").enumerate() {
                let authored_effect_index = u16::try_from(index)
                    .map_err(|_| format!("weather effect vector {token} exceeds u16"))?;
                let primary_entity = unique_scalar_field(&effect, "entity")?.ok_or_else(|| {
                    format!("weather effect {token}#{index} has no entity")
                })?;
                let small = unique_scalar_field(&effect, "small")?
                    .map(|value| match value.as_str() {
                        "yes" => Ok(true),
                        "no" => Ok(false),
                        _ => Err(format!(
                            "weather effect {token}#{index} has invalid small value {value:?}"
                        )),
                    })
                    .transpose()?
                    .unwrap_or(false);
                rows.push(AuthoredWeatherRow {
                    authored_effect_index,
                    class: if small {
                        WeatherPositionClass::Small
                    } else {
                        WeatherPositionClass::Big
                    },
                    primary_entity,
                    fading_entity: unique_scalar_field(&effect, "fading_entity")?,
                });
            }
            if out.insert(token.clone(), rows).is_some() {
                return Err(format!("weather_effects repeats type {token}"));
            }
        }
        rest = after;
    }
    Ok(out)
}

fn index_particle_definitions(
    payloads: &BTreeMap<String, WeatherCatalogSourcePayload>,
) -> Result<BTreeMap<String, Vec<ParticleDefinition>>, String> {
    let mut out = BTreeMap::<String, Vec<ParticleDefinition>>::new();
    for (path, source) in payloads {
        if !path.starts_with("gfx/particles/") || !path.ends_with(".asset") {
            continue;
        }
        let text = std::str::from_utf8(&source.payload)
            .map_err(|error| format!("particle asset {path} is not UTF-8/ASCII: {error}"))?;
        let text = strip_clausewitz_comments(text, path)?;
        for body in repeated_braced_values(&text, "particle") {
            let Some(name) = unique_scalar_field(&body, "name")? else {
                continue;
            };
            out.entry(name).or_default().push(ParticleDefinition {
                source_path: path.clone(),
                body,
            });
        }
    }
    Ok(out)
}

fn named_blocks(source: &str, kind: &str, name: &str) -> Vec<String> {
    repeated_braced_values(source, kind)
        .filter(|body| {
            unique_scalar_field(body, "name")
                .ok()
                .flatten()
                .as_deref()
                == Some(name)
        })
        .collect()
}

fn event_particle_tokens(entity_body: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for event in repeated_braced_values(entity_body, "event") {
        if let Some(particle) = unique_scalar_field(&event, "particle")? {
            out.push(particle);
        }
    }
    Ok(out)
}

fn particle_textures(body: &str) -> Result<Vec<(String, String)>, String> {
    repeated_braced_values(body, "texture")
        .map(|texture| {
            let file = unique_scalar_field(&texture, "file")?
                .ok_or("particle texture has no file")?;
            let shader = unique_scalar_field(&texture, "shader")?
                .ok_or("particle texture has no shader")?;
            Ok((file, shader))
        })
        .collect()
}

fn particle_shader_closure(
    payloads: &BTreeMap<String, WeatherCatalogSourcePayload>,
    required_effects: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    if required_effects.is_empty() {
        return Err("weather particle closure has no shader effects".into());
    }
    let root = required_shader_text(payloads, PARTICLE_SHADER_SOURCE)?;
    for effect in required_effects {
        let matches = named_keyword_blocks(&root, "Effect", effect);
        if matches != 1 {
            return Err(format!(
                "weather particle shader effect {effect:?} resolved {matches} times"
            ));
        }
    }

    let mut ordered = Vec::new();
    let mut pending = vec![PARTICLE_SHADER_SOURCE.to_string()];
    let mut visited = BTreeSet::new();
    while let Some(path) = pending.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        ordered.push(path.clone());
        let source = required_shader_text(payloads, &path)?;
        let includes = repeated_braced_values(&source, "Includes").collect::<Vec<_>>();
        if includes.len() > 1 {
            return Err(format!("weather shader source {path} repeats Includes"));
        }
        let Some(includes) = includes.first() else {
            continue;
        };
        let parent = path.rsplit_once('/').map(|(parent, _)| parent).unwrap_or("");
        let mut resolved = Vec::new();
        for include in quoted_strings(includes)? {
            let candidate = if include.contains('/') {
                include
            } else if parent.is_empty() {
                include
            } else {
                format!("{parent}/{include}")
            };
            let normalized = normalize_game_relative_path(&candidate)?;
            required_source(payloads, &normalized)?;
            resolved.push(normalized);
        }
        // LIFO traversal with reverse push preserves the authored include order.
        pending.extend(resolved.into_iter().rev());
    }
    Ok(ordered)
}

fn named_keyword_blocks(source: &str, keyword: &str, name: &str) -> usize {
    let bytes = source.as_bytes();
    let mut count = 0usize;
    let mut from = 0usize;
    while let Some(found) = source[from..].find(keyword) {
        let index = from + found;
        let before_ok = index == 0
            || !source[..index]
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_alphanumeric() || ch == '_');
        let mut cursor = index + keyword.len();
        if !before_ok || !bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            from = cursor;
            continue;
        }
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let start = cursor;
        while bytes
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            cursor += 1;
        }
        if &source[start..cursor] != name {
            from = cursor.max(index + keyword.len());
            continue;
        }
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) == Some(&b'{') && balanced_body(source, cursor).is_some() {
            count += 1;
        }
        from = cursor.saturating_add(1);
    }
    count
}

fn quoted_strings(block: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut chars = block.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch != '"' {
            continue;
        }
        let mut value = String::new();
        let mut closed = false;
        while let Some((_, next)) = chars.next() {
            if next == '"' {
                closed = true;
                break;
            } else {
                value.push(next);
            }
        }
        if !closed {
            return Err("weather shader include list has an unterminated string".into());
        }
        if value.trim().is_empty() {
            return Err("weather shader include list has an empty source".into());
        }
        out.push(value);
    }
    Ok(out)
}

fn push_unique_path(out: &mut Vec<String>, path: &str) {
    if !out.iter().any(|existing| existing == path) {
        out.push(path.to_string());
    }
}

fn require_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        || value.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return Err(format!("{label} is not canonical lowercase SHA-256"));
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Remove Clausewitz `#` comments without changing line count or touching a
/// hash embedded in a quoted resource token. Keeping newlines intact is
/// important because `map/weatherpositions.txt` source-line identity is part
/// of the portable contract.
fn strip_clausewitz_comments(source: &str, path: &str) -> Result<String, String> {
    let mut out = String::with_capacity(source.len());
    let mut quoted = false;
    let mut escaped = false;
    let mut comment = false;
    for ch in source.chars() {
        if comment {
            if ch == '\n' || ch == '\r' {
                comment = false;
                out.push(ch);
            } else {
                out.push(' ');
            }
            continue;
        }
        if quoted {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            continue;
        }
        match ch {
            '"' => {
                quoted = true;
                out.push(ch);
            }
            '#' => {
                comment = true;
                out.push(' ');
            }
            _ => out.push(ch),
        }
    }
    if quoted {
        return Err(format!("weather source {path} has an unterminated quoted string"));
    }
    Ok(out)
}

fn required_shader_text(
    payloads: &BTreeMap<String, WeatherCatalogSourcePayload>,
    path: &str,
) -> Result<String, String> {
    let source = required_utf8(payloads, path)?;
    if !source.is_ascii() {
        return Err(format!(
            "weather shader source {path} is not ASCII and cannot be tokenized exactly"
        ));
    }
    strip_c_comments(&source, path)
}

/// Shader sources use C-style comments and `#` preprocessor directives. The
/// latter must remain source rather than being mistaken for Clausewitz text
/// comments while discovering includes and named `Effect` blocks.
fn strip_c_comments(source: &str, path: &str) -> Result<String, String> {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut index = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if line_comment {
            if byte == b'\n' || byte == b'\r' {
                line_comment = false;
                out.push(byte as char);
            } else {
                out.push(' ');
            }
            index += 1;
            continue;
        }
        if block_comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                out.push_str("  ");
                block_comment = false;
                index += 2;
            } else {
                out.push(if byte == b'\n' || byte == b'\r' {
                    byte as char
                } else {
                    ' '
                });
                index += 1;
            }
            continue;
        }
        if quoted {
            out.push(byte as char);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            quoted = true;
            out.push('"');
            index += 1;
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            out.push_str("  ");
            line_comment = true;
            index += 2;
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            out.push_str("  ");
            block_comment = true;
            index += 2;
        } else {
            out.push(byte as char);
            index += 1;
        }
    }
    if quoted || block_comment {
        return Err(format!(
            "weather shader source {path} has an unterminated string/comment"
        ));
    }
    Ok(out)
}

// Minimal balanced Clausewitz-text reader. It deliberately has no recovery or
// fallback behavior: malformed braces or missing exact tokens return None.
fn braced_value(source: &str, key: &str) -> Option<String> {
    let brace = find_key_brace(source, key)?;
    balanced_body(source, brace)
}

fn repeated_braced_values<'a>(
    source: &'a str,
    key: &'a str,
) -> impl Iterator<Item = String> + 'a {
    let mut from = 0usize;
    std::iter::from_fn(move || {
        while from < source.len() {
            let brace = find_key_brace(&source[from..], key)? + from;
            let body = balanced_body(source, brace)?;
            from = (brace + body.len() + 2).min(source.len());
            return Some(body);
        }
        None
    })
}

fn next_braced_entry(source: &str) -> Option<(String, String, &str)> {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let token = &source[start..index];
            let mut cursor = index;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if bytes.get(cursor) != Some(&b'=') {
                continue;
            }
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if bytes.get(cursor) != Some(&b'{') {
                continue;
            }
            let body = balanced_body(source, cursor)?;
            let after = (cursor + body.len() + 2).min(source.len());
            return Some((token.into(), body, &source[after..]));
        }
        index += 1;
    }
    None
}

fn unique_scalar_field(block: &str, key: &str) -> Result<Option<String>, String> {
    let count = assignment_count(block, key);
    if count > 1 {
        return Err(format!("weather definition repeats scalar field {key:?}"));
    }
    if count == 0 {
        return Ok(None);
    }
    scalar_field(block, key)
        .map(Some)
        .ok_or_else(|| format!("weather definition has malformed scalar field {key:?}"))
}

fn assignment_count(source: &str, key: &str) -> usize {
    let bytes = source.as_bytes();
    let mut count = 0usize;
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    while index < bytes.len() {
        if quoted {
            if escaped {
                escaped = false;
            } else if bytes[index] == b'\\' {
                escaped = true;
            } else if bytes[index] == b'"' {
                quoted = false;
            }
            index += 1;
            continue;
        }
        match bytes[index] {
            b'"' => {
                quoted = true;
                index += 1;
            }
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            byte if depth == 0 && (byte.is_ascii_alphanumeric() || byte == b'_') => {
                let start = index;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
                {
                    index += 1;
                }
                let token = &source[start..index];
                let mut cursor = index;
                while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                if token == key && bytes.get(cursor) == Some(&b'=') {
                    count += 1;
                }
            }
            _ => index += 1,
        }
    }
    count
}

fn scalar_field(block: &str, key: &str) -> Option<String> {
    let bytes = block.as_bytes();
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    while index < bytes.len() {
        if quoted {
            if escaped {
                escaped = false;
            } else if bytes[index] == b'\\' {
                escaped = true;
            } else if bytes[index] == b'"' {
                quoted = false;
            }
            index += 1;
            continue;
        }
        match bytes[index] {
            b'"' => {
                quoted = true;
                index += 1;
            }
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            byte if depth == 0 && (byte.is_ascii_alphanumeric() || byte == b'_') => {
                let start_key = index;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
                {
                    index += 1;
                }
                if &block[start_key..index] != key {
                    continue;
                }
                let mut cursor = index;
                while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                if bytes.get(cursor) != Some(&b'=') {
                    continue;
                }
                cursor += 1;
                while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                if bytes.get(cursor) == Some(&b'"') {
                    let start = cursor + 1;
                    let end = block[start..].find('"')? + start;
                    return Some(block[start..end].to_string());
                }
                if bytes.get(cursor) == Some(&b'{') {
                    return balanced_body(block, cursor).map(|value| value.trim().to_string());
                }
                let start = cursor;
                while cursor < bytes.len()
                    && !bytes[cursor].is_ascii_whitespace()
                    && bytes[cursor] != b'}'
                {
                    cursor += 1;
                }
                return (cursor > start)
                    .then(|| block[start..cursor].trim_matches(',').to_string());
            }
            _ => index += 1,
        }
    }
    None
}

fn find_key_brace(source: &str, key: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut from = 0usize;
    while let Some(found) = source[from..].find(key) {
        let index = from + found;
        let before_ok = index == 0
            || !source[..index]
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_alphanumeric() || ch == '_');
        let after_key = index + key.len();
        let after_ok = source[after_key..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_whitespace() || ch == '=');
        if before_ok && after_ok {
            let mut cursor = after_key;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if bytes.get(cursor) == Some(&b'=') {
                cursor += 1;
                while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                if bytes.get(cursor) == Some(&b'{') {
                    return Some(cursor);
                }
            }
        }
        from = after_key;
    }
    None
}

fn balanced_body(source: &str, brace: usize) -> Option<String> {
    let bytes = source.as_bytes();
    if bytes.get(brace) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut quoted = false;
    let mut escaped = false;
    for index in brace..bytes.len() {
        if quoted {
            if escaped {
                escaped = false;
            } else if bytes[index] == b'\\' {
                escaped = true;
            } else if bytes[index] == b'"' {
                quoted = false;
            }
            continue;
        }
        match bytes[index] {
            b'"' => quoted = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(source[brace + 1..index].to_string());
                }
            }
            _ => {}
        }
    }
    None
}
