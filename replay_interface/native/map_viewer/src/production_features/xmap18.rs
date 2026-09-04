
use crate::{
    FoundationFeatureContext, FoundationFeatureOutput, GameFileResolver, HardcodedMapMode,
    ProductionMapFeatureAdapter, Result, ViewerMapModeSelection, XMAP18_A729_EXECUTABLE_SHA256,
    XMAP18_REGISTRY_GHIDRA_MANIFEST_SHA256,
};
use portable_content_manifest::{
    paradox_text::{decode_text_diagnosed, destructive_losses, parse_str, Value},
    UsedContentKey,
};
use portable_render_stream::{FamilyDisposition, FoundationOperationIdentity, MapVisualFamily};
use replay_core::{Leaf, ValidatedContentAdmission, ViewStatus};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;

#[cfg(test)]
use portable_content_manifest::paradox_text::destructive_event_count;
use std::collections::{BTreeMap, BTreeSet};

pub const XMAP18_MAP_MODE_SCHEMA: &str = "hoi4.xmap18.map-mode-composition.a729.v1";
pub const XMAP18_FILL_SOURCE_SCHEMA: &str = "hoi4.xmap18.map-mode-fill-source.a729.v1";
pub const XMAP18_RUNTIME_INPUT_SCHEMA: &str = "hoi4.xmap18.raw-map-mode-primitives.a729.v4";
pub const XMAP18_SCRIPTED_RUNTIME_INPUT_SCHEMA: &str =
    "hoi4.xmap18.engine-evaluated-scripted-scopes.a729.v2";
pub const XMAP18_MAX_RUNTIME_JSON_BYTES_PER_HOUR: usize = 256 * 1024;
pub const XMAP18_MAX_SCRIPTED_RUNTIME_JSON_BYTES_PER_HOUR: usize = 16 * 1024 * 1024;

const STATIC_CONTENT: [&str; 2] = ["common/countries/colors.txt", "map/definition.csv"];
const STATIC_DIRS: [(&str, Option<&str>); 10] = [
    ("common/defines", Some("lua")),
    ("common/buildings", Some("txt")),
    ("common/terrain", Some("txt")),
    ("history/states", Some("txt")),
    ("history/countries", Some("txt")),
    ("common/country_tags", Some("txt")),
    ("common/countries", Some("txt")),
    ("common/ideologies", Some("txt")),
    ("map/strategicregions", Some("txt")),
    ("common/map_modes", Some("txt")),
];
/// Every map-authority input the XMAP00 ledger gives XMAP18 as owner packet,
/// gapped or admitted.
///
/// This used to be a gap-ONLY list that `prepare` treated as a hard error when
/// an entry was missing from `map_authority_gaps`. That is the defect XMAP13
/// already fixed for its own rows: an owner-proven input reports ADMITTED, not
/// gapped, so the moment any owned row was proven the whole foundation graph
/// aborted. Two entries here — `diplomacy.faction_roster` and
/// `mapmode.occupation_resistance_compliance` — were ALREADY owner-proven, so
/// this abort was live, not hypothetical: `mapmode.occupation_resistance_compliance`
/// is ADMITTED on 13 of the 14 hours sampled across `captures/oz_tip`, and every
/// one of those hours aborted XMAP18 before this was found (lane PL,
/// 2026-08-21; extractor/native/runs/pl_xmap18/witness_final_stride40.json).
///
/// `mapmode.raids`, `mapmode.resource_deposits` and `diplomacy.pair_relations`
/// were missing from the old list entirely; the accounting is only honest if it
/// names every row the ledger assigns to this packet.
/// Visible to `production_foundation`'s owner-agreement test, which compares
/// this list against what the owner table derives for XMAP18. The ledger only
/// lists owner-PROVEN inputs, so the ledger comparison cannot see a
/// disagreement about an unproven one; this list can (Codex review, round 11).
pub(crate) const OWNED_AUTHORITY_INPUTS: [&str; 11] = [
    "diplomacy.faction_roster",
    "diplomacy.pair_relations",
    "mapmode.air_superiority",
    "mapmode.infrastructure_state",
    "mapmode.intelligence",
    "mapmode.naval_supremacy",
    "mapmode.occupation_resistance_compliance",
    "mapmode.raids",
    "mapmode.resource_deposits",
    "mapmode.scripted_inputs",
    "session.player_country_set",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapModeComposition {
    Political,
    StrategicAir,
    StrategicNavy,
    Intelligence,
    StateOwner,
    Supply,
    Resistance,
    Compliance,
    ResourceFurniture,
    Diplomacy,
    Factions,
    Players,
    Infrastructure,
    Manpower,
    Ideology,
    Terrain,
    RaidFurniture,
    ScriptedTwoLayer,
}

impl MapModeComposition {
    pub const fn token(self) -> &'static str {
        match self {
            Self::Political => "political",
            Self::StrategicAir => "strategic_air",
            Self::StrategicNavy => "strategic_navy",
            Self::Intelligence => "intelligence",
            Self::StateOwner => "state_owner",
            Self::Supply => "supply",
            Self::Resistance => "resistance",
            Self::Compliance => "compliance",
            Self::ResourceFurniture => "resource_furniture",
            Self::Diplomacy => "diplomacy",
            Self::Factions => "factions",
            Self::Players => "players",
            Self::Infrastructure => "infrastructure",
            Self::Manpower => "manpower",
            Self::Ideology => "ideology",
            Self::Terrain => "terrain",
            Self::RaidFurniture => "raid_furniture",
            Self::ScriptedTwoLayer => "scripted_two_layer",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineMapModeConfiguration {
    /// are authenticated below instead of inventing a sentinel ABI value.
    pub packed_layer_word: Option<u64>,
    pub fill_channels: Vec<(u8, u8)>,
    pub fill_update_kinds: Vec<u8>,
    pub manager_state_0x50: Option<u32>,
    pub manager_state_0x54: Option<u32>,
    pub layer0_outline_cutoff_define: Option<String>,
    pub layer0_camera_distance_override_define: Option<String>,
    pub layer0_flag_0x80: bool,
    pub manager_flag_0x49: bool,
    pub scripted_definition_sha256: Option<String>,
    pub scripted_layers: Vec<String>,
    pub scripted_far_text: Option<String>,
    pub scripted_near_text: Option<String>,
    pub scripted_update_daily: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineProvinceColorWrite {
    pub province_id: u32,
    pub packed_bgra: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap18OwnerResourceRole {
    Xmap05ProvincePrimary,
    Xmap05ProvinceSecondary,
    Xmap05GradientBorderChannel1,
    Xmap05GradientBorderChannel2,
    Xmap05IntelMap,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap18OwnerResourceRef {
    pub role: Xmap18OwnerResourceRole,
    pub descriptor_sha256: String,
    pub payload_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineFillUpdate {
    pub update_kind: u8,
    pub ordered_writes: Vec<EngineProvinceColorWrite>,
    pub expected_write_keys_sha256: String,
    pub ordered_row_set_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Xmap18RuntimeInput {
    schema: String,
    complete: bool,
    hour_index: i64,
    effective_vfs_manifest_sha256: String,
    observer_country_ids: Vec<String>,
    strategic_region_ids: Vec<u32>,
    state_ids: Vec<u32>,
    air_cells: Vec<Xmap18AirCell>,
    naval_cells: Vec<Xmap18NavalCell>,
    operative_strength_scale: i64,
    operative_cells: Vec<Xmap18OperativeCell>,
    projected_packed_bytes: u64,
    deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Xmap18AirCell {
    observer_index: u32,
    strategic_region_index: u32,
    friendly_weight_raw: i64,
    enemy_weight_raw: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Xmap18NavalCell {
    observer_index: u32,
    strategic_region_index: u32,
    control_code_raw: u8,
    contested_code_raw: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Xmap18OperativeCell {
    observer_index: u32,
    state_index: u32,
    strength_raw: i64,
    quiet_raw: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Xmap18ScriptedRuntimeInput {
    schema: String,
    complete: bool,
    hour_index: i64,
    effective_vfs_manifest_sha256: String,
    observer_country_ids: Vec<String>,
    state_ids: Vec<u32>,
    scripted_mode_ids: Vec<String>,
    layer_payloads: Vec<Xmap18ScriptedLayerPayload>,
    entries: Vec<Xmap18ScriptedScopeEntry>,
    projected_packed_bytes: u64,
    deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Xmap18ScriptedScopeEntry {
    observer_index: u32,
    scripted_mode_index: u32,
    top_payload_index: u32,
    bottom_payload_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Xmap18ScriptedLayerPayload {
    scope_kind: Xmap18ScriptedScopeKind,
    rows: Vec<Xmap18ScriptedScopeResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Xmap18ScriptedScopeKind {
    None,
    Country,
    State,
    StateController,
    HardcodedPolitical,
    HardcodedStates,
    HardcodedDiplomacy,
    HardcodedPlayers,
    HardcodedFactions,
    HardcodedIdeology,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Xmap18ScriptedScopeResult {
    scope_index: u32,
    controller_index: Option<u32>,
    packed_bgra: u32,
    gradient_center_f32_bits: Option<u32>,
    gradient_thickness_f32_bits: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MapModeFillSource {
    OwnerResources {
        resources: Vec<Xmap18OwnerResourceRef>,
    },
    ModeOverride {
        ordered_updates: Vec<EngineFillUpdate>,
    },
    RawSemanticMatrix { runtime_matrix_sha256: String },
    EngineEvaluatedScriptedScopes { runtime_scopes_sha256: String },
}

impl MapModeFillSource {
    fn validate(&self) -> Result<()> {
        match self {
            Self::OwnerResources { resources } => {
                if resources.is_empty() {
                    return Err("XMAP18 configuration-only mode has no owner resources".into());
                }
                let mut roles = BTreeSet::new();
                for resource in resources {
                    if !roles.insert(resource.role) {
                        return Err("XMAP18 owner resource role is duplicated".into());
                    }
                    validate_digest(&resource.descriptor_sha256, "owner resource descriptor")?;
                    validate_digest(&resource.payload_sha256, "owner resource payload")?;
                }
            }
            Self::ModeOverride { ordered_updates } => {
                if ordered_updates.is_empty() {
                    return Err("XMAP18 mode override has no engine update producer".into());
                }
                if ordered_updates
                    .iter()
                    .all(|update| update.ordered_writes.is_empty())
                {
                    return Err("XMAP18 mode override has no CFillBuffer writes".into());
                }
                for update in ordered_updates {
                    let mut keys = BTreeSet::new();
                    for write in &update.ordered_writes {
                        if write.province_id == 0 || !keys.insert(write.province_id) {
                            return Err(
                                "XMAP18 mode override has an invalid or duplicate write key".into(),
                            );
                        }
                    }
                    if update.expected_write_keys_sha256 != write_key_digest(&update.ordered_writes)
                        || update.ordered_row_set_sha256 != write_row_digest(&update.ordered_writes)
                    {
                        return Err("XMAP18 mode override row-set identity mismatch".into());
                    }
                }
            }
            Self::RawSemanticMatrix {
                runtime_matrix_sha256,
            } => validate_digest(runtime_matrix_sha256, "raw semantic matrix")?,
            Self::EngineEvaluatedScriptedScopes {
                runtime_scopes_sha256,
            } => validate_digest(runtime_scopes_sha256, "scripted runtime scopes")?,
        }
        Ok(())
    }

    fn validate_against_xmap05(&self, political: &crate::PreparedPoliticalFrame) -> Result<()> {
        self.validate()?;
        let Self::OwnerResources { resources } = self else {
            return Ok(());
        };
        let expected = xmap05_owner_resources(political);
        if resources != &expected {
            return Err("XMAP18 owner resources disagree with the prepared XMAP05 frame".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMapModeFrame {
    pub schema: String,
    pub hour_index: i64,
    pub viewer_selection: String,
    pub engine_mode_code: u32,
    pub visible_button: Option<u8>,
    pub composition: MapModeComposition,
    pub engine_configuration: EngineMapModeConfiguration,
    pub fill_source: MapModeFillSource,
    pub render_political: crate::PreparedPoliticalFrame,
    pub observer_country_id: Option<String>,
    pub effective_scripted_mode_ids: Vec<String>,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub semantic_witness_sha256: String,
    pub deterministic_sha256: String,
}

impl PreparedMapModeFrame {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP18_MAP_MODE_SCHEMA
            || self.viewer_selection.trim().is_empty()
            || self.ordered_used_content.is_empty()
            || self.semantic_witness_sha256.len() != 64
            || self.deterministic_sha256 != prepared_digest(self)
        {
            return Err("XMAP18 prepared map-mode frame is incomplete".into());
        }
        self.fill_source.validate()?;
        self.render_political.validate()?;
        if self.render_political.hour_index != self.hour_index {
            return Err("XMAP18 render resources have a different replay hour".into());
        }
        if let MapModeFillSource::ModeOverride { ordered_updates } = &self.fill_source {
            let update_kinds = ordered_updates
                .iter()
                .map(|update| update.update_kind)
                .collect::<Vec<_>>();
            if update_kinds != self.engine_configuration.fill_update_kinds {
                return Err(
                    "XMAP18 mode override boundaries disagree with engine update order".into(),
                );
            }
        }
        let selection = ViewerMapModeSelection::parse(&self.viewer_selection)?;
        let scripted_index = match &selection {
            ViewerMapModeSelection::Scripted(id) => self
                .effective_scripted_mode_ids
                .iter()
                .position(|candidate| candidate == id)
                .map(u32::try_from)
                .transpose()?,
            ViewerMapModeSelection::Hardcoded(_) => None,
        };
        if selection.engine_code(scripted_index)? != self.engine_mode_code
            || selection.visible_button() != self.visible_button
        {
            return Err("XMAP18 selection, engine code, and visible button disagree".into());
        }
        match selection {
            ViewerMapModeSelection::Hardcoded(mode) => {
                if self.engine_configuration != hardcoded_engine_configuration(mode) {
                    return Err(
                        "XMAP18 engine layer configuration disagrees with exact a729".into(),
                    );
                }
            }
            ViewerMapModeSelection::Scripted(_) => {
                let configuration = &self.engine_configuration;
                if configuration.packed_layer_word.is_some()
                    || !configuration.fill_channels.is_empty()
                    || !configuration.fill_update_kinds.is_empty()
                    || configuration.manager_state_0x50.is_some()
                    || configuration.manager_state_0x54.is_some()
                    || configuration.layer0_outline_cutoff_define.is_some()
                    || configuration
                        .layer0_camera_distance_override_define
                        .is_some()
                    || configuration.layer0_flag_0x80
                    || configuration.manager_flag_0x49
                    || configuration.scripted_layers.len() != 2
                    || !configuration.scripted_layers[0].starts_with("top:")
                    || !configuration.scripted_layers[1].starts_with("bottom:")
                    || configuration.scripted_far_text.is_none()
                    || configuration.scripted_near_text.is_none()
                    || configuration.scripted_update_daily.is_none()
                {
                    return Err("XMAP18 scripted engine layer configuration is incomplete".into());
                }
                validate_digest(
                    configuration
                        .scripted_definition_sha256
                        .as_deref()
                        .ok_or("XMAP18 scripted definition identity is absent")?,
                    "scripted definition",
                )?;
            }
        }
        let mut paths = BTreeSet::new();
        for content in &self.ordered_used_content {
            content
                .validate()
                .map_err(|error| format!("XMAP18 used-content identity: {error}"))?;
            if !paths.insert(content.normalized_game_relative_path.as_str()) {
                return Err("XMAP18 used-content path is duplicated".into());
            }
        }
        Ok(())
    }

    pub fn xmap16_supply_view_active(&self) -> Result<bool> {
        self.validate()?;
        Ok(self.composition == MapModeComposition::Supply)
    }

    pub fn xmap16_observer_country_id(&self) -> Result<Option<&str>> {
        self.validate()?;
        Ok(self.observer_country_id.as_deref())
    }

    pub fn xmap17_text_selection(&self) -> Result<(u8, u8, bool)> {
        self.validate()?;
        if !matches!(
            ViewerMapModeSelection::parse(&self.viewer_selection)?,
            ViewerMapModeSelection::Scripted(_)
        ) {
            return Ok((0, 0, true));
        }
        let selector = |value: Option<&str>, label: &str| -> Result<u8> {
            Ok(
                match value.ok_or_else(|| format!("XMAP18 scripted {label} selector is absent"))? {
                    "country" => 0,
                    "faction" => 1,
                    "state" => 2,
                    "player" => 3,
                    "none" => 4,
                    other => {
                        return Err(format!(
                            "XMAP18 scripted {label} selector {other:?} is invalid"
                        )
                        .into())
                    }
                },
            )
        };
        Ok((
            selector(
                self.engine_configuration.scripted_near_text.as_deref(),
                "near_text",
            )?,
            selector(
                self.engine_configuration.scripted_far_text.as_deref(),
                "far_text",
            )?,
            self.engine_configuration
                .scripted_update_daily
                .ok_or("XMAP18 scripted update_daily is absent")?,
        ))
    }
}

#[derive(Clone)]
pub(crate) struct Xmap18MapModeAdapter {
    resolver: GameFileResolver,
    ordered_used_content: Vec<UsedContentKey>,
    effective_scripted_mode_ids: Vec<String>,
}

impl Xmap18MapModeAdapter {
    pub(crate) fn new(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
    ) -> Result<Self> {
        let mut required = BTreeSet::new();
        required.extend(STATIC_CONTENT.map(str::to_string));
        for (directory, extension) in STATIC_DIRS {
            required.extend(
                resolver
                    .require_complete_admitted_dir(
                        directory,
                        extension,
                        true,
                        "XMAP18 effective authored namespace",
                    )?
                    .into_iter()
                    .map(|row| row.normalized_game_relative_path),
            );
        }
        let mut ordered_used_content = Vec::with_capacity(required.len());
        for path in required {
            let content = admission
                .require_used_content(&path)
                .map_err(|error| format!("XMAP18 requires admitted {path}: {error}"))?
                .clone();
            resolver.read_admitted_used_content(&content)?;
            ordered_used_content.push(content);
        }
        let effective_scripted_mode_ids = effective_scripted_definitions(resolver)?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        Ok(Self {
            resolver: resolver.clone(),
            ordered_used_content,
            effective_scripted_mode_ids,
        })
    }
}

#[cfg(test)]
impl Xmap18MapModeAdapter {
    /// Test-only constructor: a resolver over an arbitrary existing directory
    /// and empty content, enough to reach `prepare()`'s owned-input accounting
    /// and selection stages, which touch neither field.
    fn new_for_owned_accounting_tests() -> Self {
        Self {
            resolver: GameFileResolver::with_mod_paths(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
                Vec::new(),
            )
            .expect("test resolver over the crate directory"),
            ordered_used_content: Vec::new(),
            effective_scripted_mode_ids: Vec::new(),
        }
    }
}

impl ProductionMapFeatureAdapter for Xmap18MapModeAdapter {
    fn family(&self) -> MapVisualFamily {
        MapVisualFamily::MapModes
    }

    fn prepare(&self, context: &FoundationFeatureContext<'_>) -> Result<FoundationFeatureOutput> {
        // Owned-input accounting first, and identically whether a row is gapped
        // or admitted. See `OWNED_AUTHORITY_INPUTS`.
        let consumed_gap_ids = crate::account_owned_map_authority_inputs(
            "XMAP18",
            &OWNED_AUTHORITY_INPUTS,
            context.map_authority_gaps,
            context.map_authority_admitted,
        )?;
        let selection = ViewerMapModeSelection::parse(&context.viewer.map_mode_id)?;
        let (composition, scripted_index) = match &selection {
            ViewerMapModeSelection::Hardcoded(mode) => {
                validate_hardcoded_inputs(*mode, context)?;
                (hardcoded_composition(*mode), None)
            }
            ViewerMapModeSelection::Scripted(id) => {
                let index = self
                    .effective_scripted_mode_ids
                    .iter()
                    .position(|candidate| candidate == id)
                    .ok_or_else(|| {
                        format!(
                            "viewer selected scripted map mode {id}, but it is absent from the effective VFS"
                        )
                    })?;
                validate_scripted_inputs(context)?;
                (
                    MapModeComposition::ScriptedTwoLayer,
                    Some(u32::try_from(index)?),
                )
            }
        };
        let semantic_witness_sha256 = semantic_witness(context, &selection, composition)?;
        let base_political = context
            .prepared_political
            .ok_or("XMAP18 lost the prepared XMAP05 resource authority")?;
        let (render_political, raw_matrix_sha256) = prepare_render_political(
            &self.resolver,
            &selection,
            &self.effective_scripted_mode_ids,
            context,
            base_political,
        )?;
        let fill_source =
            prepare_fill_source(&selection, &render_political, raw_matrix_sha256.as_deref())?;
        let mut prepared = PreparedMapModeFrame {
            schema: XMAP18_MAP_MODE_SCHEMA.into(),
            hour_index: context.world_frame.hour_index(),
            viewer_selection: selection.viewer_token(),
            engine_mode_code: selection.engine_code(scripted_index)?,
            visible_button: selection.visible_button(),
            composition,
            engine_configuration: match &selection {
                ViewerMapModeSelection::Hardcoded(mode) => hardcoded_engine_configuration(*mode),
                ViewerMapModeSelection::Scripted(id) => {
                    scripted_engine_configuration(&self.resolver, id)?
                }
            },
            fill_source,
            render_political,
            observer_country_id: context.viewer.observer_country_id.clone(),
            effective_scripted_mode_ids: self.effective_scripted_mode_ids.clone(),
            ordered_used_content: self.ordered_used_content.clone(),
            semantic_witness_sha256,
            deterministic_sha256: String::new(),
        };
        prepared.deterministic_sha256 = prepared_digest(&prepared);
        prepared.validate()?;
        prepared
            .fill_source
            .validate_against_xmap05(&prepared.render_political)?;
        Ok(FoundationFeatureOutput {
            disposition: FamilyDisposition::Present {
                payload_schema: XMAP18_MAP_MODE_SCHEMA.into(),
                producer_registration_sha256: registration_digest(),
                payload_sha256: prepared.deterministic_sha256.clone(),
                ordered_operations: vec![FoundationOperationIdentity {
                    operation_id: "map-mode.select-and-compose".into(),
                    global_order: 16,
                    program_sha256: registration_digest(),
                    ordered_resources_sha256: prepared.semantic_witness_sha256.clone(),
                    target_sha256: prepared.deterministic_sha256.clone(),
                }],
                ordered_used_content: prepared.ordered_used_content.clone(),
            },
            prepared_terrain: None,
            prepared_political: None,
            prepared_thin_borders: None,
            prepared_lighting: None,
            prepared_weather: None,
            prepared_rivers_straits: None,
            prepared_water: None,
            prepared_units: None,
            prepared_combat: None,
            prepared_orders: None,
            prepared_rail_supply: None,
            prepared_map_modes: Some(prepared),
            element_diagnostics: Vec::new(),
            consumed_gap_ids,
        })
    }
}

fn effective_directory_source(
    resolver: &GameFileResolver,
    directory: &str,
    extension: Option<&str>,
    label: &str,
) -> Result<String> {
    let files = resolver.resolve_dir_files_recursive(directory, extension)?;
    if files.is_empty() {
        return Err(format!("XMAP18 {label} directory {directory} is empty").into());
    }
    let mut source = String::new();
    for (relative, path) in files.iter().rev() {
        source.push_str("\n-- XMAP18 effective source: ");
        source.push_str(relative);
        source.push('\n');
        source.push_str(&crate::read_latin1(path)?);
        source.push('\n');
    }
    Ok(source)
}

#[derive(Clone, Copy)]
enum ScriptedLayerKind {
    None,
    Country,
    State,
    StateController,
    Hardcoded(HardcodedMapMode),
}

#[derive(Clone, Copy)]
struct ScriptedLayerSpec {
    band: crate::production_features::xmap05::Xmap05MapModeBand,
    kind: ScriptedLayerKind,
    rgba: Option<[u8; 4]>,
}

fn prepare_scripted_render(
    resolver: &GameFileResolver,
    selection: &ViewerMapModeSelection,
    context: &FoundationFeatureContext<'_>,
    base: &crate::PreparedPoliticalFrame,
) -> Result<(crate::PreparedPoliticalFrame, Option<String>)> {
    let ViewerMapModeSelection::Scripted(id) = selection else {
        return Err("XMAP18 scripted renderer received a hardcoded selection".into());
    };
    let value = context
        .world_frame
        .domain("map_mode_scripted_runtime")
        .ok_or_else(|| {
            format!("XMAP18 scripted mode {id} requires engine-evaluated runtime data")
        })?;
    let runtime = decode_bounded_scripted_runtime_input(value)?;
    let definitions = effective_scripted_definitions(resolver)?;
    validate_scripted_runtime(&runtime, resolver, context, &definitions)?;
    let observer = context
        .viewer
        .observer_country_id
        .as_deref()
        .ok_or_else(|| format!("XMAP18 scripted mode {id} requires a viewer country"))?;
    let observer_index = runtime
        .observer_country_ids
        .iter()
        .position(|candidate| candidate == observer)
        .ok_or("XMAP18 scripted runtime has no selected observer")?;
    let scripted_mode_index = runtime
        .scripted_mode_ids
        .iter()
        .position(|candidate| candidate == id)
        .ok_or("XMAP18 scripted runtime has no selected mode")?;
    let entry = runtime
        .entries
        .iter()
        .find(|entry| {
            entry.observer_index as usize == observer_index
                && entry.scripted_mode_index as usize == scripted_mode_index
        })
        .ok_or("XMAP18 scripted runtime lost a complete scope entry")?;
    let definition = definitions
        .iter()
        .find_map(|(candidate, definition)| (candidate == id).then_some(definition))
        .ok_or("XMAP18 selected scripted definition disappeared")?;
    let expected_kinds = scripted_runtime_scope_kinds(id, definition)?;
    let payload_indices = [entry.top_payload_index, entry.bottom_payload_index];
    let bands = [
        crate::production_features::xmap05::Xmap05MapModeBand::Top,
        crate::production_features::xmap05::Xmap05MapModeBand::Bottom,
    ];
    let mut rendered = base.clone();
    for ((payload_index, expected_kind), band) in
        payload_indices.into_iter().zip(expected_kinds).zip(bands)
    {
        let payload = runtime
            .layer_payloads
            .get(usize::try_from(payload_index)?)
            .ok_or("XMAP18 scripted entry names an absent layer payload")?;
        if payload.scope_kind != expected_kind {
            return Err(
                "XMAP18 scripted runtime scope kind differs from effective definition".into(),
            );
        }
        rendered = materialize_scripted_scope_payload(
            resolver, context, &rendered, base, band, payload, &runtime,
        )?;
    }
    Ok((rendered, Some(runtime.deterministic_sha256)))
}

fn scripted_runtime_scope_kinds(
    id: &str,
    definition: &Value,
) -> Result<[Xmap18ScriptedScopeKind; 2]> {
    let parse = |name: &str| -> Result<Xmap18ScriptedScopeKind> {
        let Some(layer) = definition.first_value_ci(name) else {
            return Ok(Xmap18ScriptedScopeKind::None);
        };
        let layer_type = layer
            .first_scalar_ci("type")
            .ok_or_else(|| format!("XMAP18 scripted mode {id} {name} layer has no type"))?;
        Ok(match layer_type {
            "none" => Xmap18ScriptedScopeKind::None,
            "country" => Xmap18ScriptedScopeKind::Country,
            "state" => Xmap18ScriptedScopeKind::State,
            "state_controller" => Xmap18ScriptedScopeKind::StateController,
            "game_map_mode_country" => Xmap18ScriptedScopeKind::HardcodedPolitical,
            "game_map_mode_states" => Xmap18ScriptedScopeKind::HardcodedStates,
            "game_map_mode_diplomacy" => Xmap18ScriptedScopeKind::HardcodedDiplomacy,
            "game_map_mode_players" => Xmap18ScriptedScopeKind::HardcodedPlayers,
            "game_map_mode_factions" => Xmap18ScriptedScopeKind::HardcodedFactions,
            "game_map_mode_ideology" => Xmap18ScriptedScopeKind::HardcodedIdeology,
            other => {
                return Err(format!(
                    "XMAP18 scripted mode {id} {name} has unsupported layer type {other}"
                )
                .into())
            }
        })
    };
    let kinds = [parse("top")?, parse("bottom")?];
    Ok(kinds)
}

fn materialize_scripted_scope_payload(
    resolver: &GameFileResolver,
    context: &FoundationFeatureContext<'_>,
    composite: &crate::PreparedPoliticalFrame,
    authority_base: &crate::PreparedPoliticalFrame,
    band: crate::production_features::xmap05::Xmap05MapModeBand,
    payload: &Xmap18ScriptedLayerPayload,
    runtime: &Xmap18ScriptedRuntimeInput,
) -> Result<crate::PreparedPoliticalFrame> {
    let delegate = match payload.scope_kind {
        Xmap18ScriptedScopeKind::HardcodedPolitical => Some(HardcodedMapMode::Political),
        Xmap18ScriptedScopeKind::HardcodedStates => Some(HardcodedMapMode::States),
        Xmap18ScriptedScopeKind::HardcodedDiplomacy => Some(HardcodedMapMode::Diplomacy),
        Xmap18ScriptedScopeKind::HardcodedPlayers => Some(HardcodedMapMode::Players),
        Xmap18ScriptedScopeKind::HardcodedFactions => Some(HardcodedMapMode::Factions),
        Xmap18ScriptedScopeKind::HardcodedIdeology => Some(HardcodedMapMode::Ideology),
        _ => None,
    };
    if let Some(mode) = delegate {
        if !payload.rows.is_empty() {
            return Err("XMAP18 hardcoded scripted payload contains evaluated scope rows".into());
        }
        validate_hardcoded_inputs(mode, context)?;
        let (delegate, _) = prepare_render_political(
            resolver,
            &ViewerMapModeSelection::Hardcoded(mode),
            &[],
            context,
            authority_base,
        )?;
        return crate::production_features::xmap05::xmap05_copy_engine_evaluated_band(
            composite, &delegate, band,
        );
    }
    if payload.scope_kind == Xmap18ScriptedScopeKind::None {
        if !payload.rows.is_empty() {
            return Err("XMAP18 none scripted payload contains evaluated scope rows".into());
        }
    }

    let state_paths = resolver.resolve_dir_files_recursive("history/states", Some("txt"))?;
    let (_, province_to_state, _) = crate::load_states(&state_paths)?;
    let (_, controllers, _) =
        crate::production_features::xmap05::admitted_epoch_keying(context.world_frame)?;
    let mut fills = province_to_state
        .keys()
        .map(|province_id| (*province_id, None))
        .collect::<BTreeMap<_, _>>();
    let mut gradients = BTreeMap::new();
    for (ordinal, row) in payload.rows.iter().enumerate() {
        let region_key = format!("scripted-scope:{ordinal}");
        let rgba = row.packed_bgra.to_le_bytes();
        let gradient = match (
            row.gradient_center_f32_bits,
            row.gradient_thickness_f32_bits,
        ) {
            (None, None) => None,
            (Some(center), Some(thickness)) => Some(
                crate::production_features::xmap05::Xmap05EvaluatedGradient {
                    center: f32::from_bits(center),
                    thickness: f32::from_bits(thickness),
                },
            ),
            _ => return Err("XMAP18 scripted scope has a partial gradient result".into()),
        };
        gradients.insert(region_key.clone(), gradient);
        let state_id = match payload.scope_kind {
            Xmap18ScriptedScopeKind::State | Xmap18ScriptedScopeKind::StateController => Some(
                *runtime
                    .state_ids
                    .get(usize::try_from(row.scope_index)?)
                    .ok_or("XMAP18 scripted row names an absent state index")?,
            ),
            _ => None,
        };
        let country = match payload.scope_kind {
            Xmap18ScriptedScopeKind::Country => Some(
                runtime
                    .observer_country_ids
                    .get(usize::try_from(row.scope_index)?)
                    .ok_or("XMAP18 scripted row names an absent country index")?,
            ),
            Xmap18ScriptedScopeKind::StateController => Some(
                runtime
                    .observer_country_ids
                    .get(usize::try_from(row.controller_index.ok_or(
                        "XMAP18 state_controller row has no controller index",
                    )?)?)
                    .ok_or("XMAP18 state_controller row names an absent controller")?,
            ),
            _ => None,
        };
        for (&province_id, &province_state) in &province_to_state {
            let in_scope = match payload.scope_kind {
                Xmap18ScriptedScopeKind::Country => controllers.get(&province_id) == country,
                Xmap18ScriptedScopeKind::State => Some(province_state) == state_id,
                Xmap18ScriptedScopeKind::StateController => {
                    Some(province_state) == state_id && controllers.get(&province_id) == country
                }
                Xmap18ScriptedScopeKind::None => false,
                _ => unreachable!(),
            };
            if in_scope
                && fills
                    .insert(province_id, Some((region_key.clone(), rgba)))
                    .flatten()
                    .is_some()
            {
                return Err("XMAP18 scripted scope results overlap on one province".into());
            }
        }
    }
    crate::production_features::xmap05::xmap05_engine_evaluated_scripted_layer_derivative(
        resolver, context, composite, band, &fills, &gradients,
    )
}

fn effective_scripted_definitions(resolver: &GameFileResolver) -> Result<Vec<(String, Value)>> {
    let mut definitions = Vec::<(String, Value)>::new();
    for (relative, path) in resolver.resolve_dir_files_recursive("common/map_modes", Some("txt"))? {
        let bytes = std::fs::read(&path)?;
        let (source, losses) = decode_text_diagnosed(&bytes);
        if !destructive_losses(&losses).is_empty() {
            return Err(
                format!("XMAP18 scripted source {relative} has destructive decode loss").into(),
            );
        }
        let document = parse_str(&source);
        if !document.recoveries.is_empty() {
            return Err(
                format!("XMAP18 scripted source {relative} required parser recovery").into(),
            );
        }
        for database in document.root.values_ci("scripted_map_modes") {
            let entries = database.as_object().ok_or_else(|| {
                format!("XMAP18 scripted_map_modes in {relative} is not an object")
            })?;
            for entry in entries {
                let id = entry
                    .key
                    .as_deref()
                    .ok_or_else(|| {
                        format!("XMAP18 scripted_map_modes in {relative} has an unkeyed entry")
                    })?
                    .trim();
                if id.is_empty() || !matches!(entry.value, Value::Object(_)) {
                    return Err(format!(
                        "XMAP18 scripted map-mode definition {id} in {relative} has no object body"
                    )
                    .into());
                }
                insert_effective_scripted_definition(
                    &mut definitions,
                    id.to_string(),
                    entry.value.clone(),
                );
            }
        }
    }
    Ok(definitions)
}

fn insert_effective_scripted_definition(
    definitions: &mut Vec<(String, Value)>,
    id: String,
    definition: Value,
) {
    if let Some(index) = definitions
        .iter()
        .position(|(candidate, _)| candidate == &id)
    {
        definitions.remove(index);
    }
    definitions.push((id, definition));
}

fn scripted_definition_from_resolver(
    resolver: &GameFileResolver,
    id: &str,
) -> Result<(Value, String)> {
    let definition = effective_scripted_definitions(resolver)?
        .into_iter()
        .find_map(|(candidate, definition)| (candidate == id).then_some(definition))
        .ok_or_else(|| format!("XMAP18 scripted mode {id} is absent"))?;
    let mut canonical = b"XMAP18_SCRIPTED_DEFINITION_AST_A729_V2\0".to_vec();
    canonical.extend_from_slice(id.as_bytes());
    canonical_value(&definition, &mut canonical);
    Ok((definition, Sha256::hex(&canonical)))
}

fn scripted_layer_specs(id: &str, definition: &Value) -> Result<Vec<ScriptedLayerSpec>> {
    let entries = definition
        .as_object()
        .ok_or_else(|| format!("XMAP18 scripted mode {id} is not an object"))?;
    let mut seen = BTreeSet::new();
    for entry in entries {
        let key = entry
            .key
            .as_deref()
            .ok_or_else(|| format!("XMAP18 scripted mode {id} has an unkeyed field"))?;
        let canonical_key = key.to_ascii_lowercase();
        if !matches!(canonical_key.as_str(), "top" | "bottom") || !seen.insert(canonical_key) {
            return Err(format!(
                "XMAP18 scripted mode {id} has unsupported or duplicate field {key}"
            )
            .into());
        }
    }
    let specs = [
        (
            "top",
            crate::production_features::xmap05::Xmap05MapModeBand::Top,
        ),
        (
            "bottom",
            crate::production_features::xmap05::Xmap05MapModeBand::Bottom,
        ),
    ]
    .into_iter()
    .map(|(name, band)| parse_scripted_layer(id, name, band, definition.first_value_ci(name)))
    .collect::<Result<Vec<_>>>()?;
    Ok(specs)
}

fn canonical_value(value: &Value, bytes: &mut Vec<u8>) {
    match value {
        Value::Scalar(portable_content_manifest::paradox_text::Scalar::Bare(value)) => {
            bytes.push(b'b');
            bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        Value::Scalar(portable_content_manifest::paradox_text::Scalar::Quoted(value)) => {
            bytes.push(b'q');
            bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        Value::Object(entries) => {
            bytes.push(b'{');
            bytes.extend_from_slice(&(entries.len() as u64).to_le_bytes());
            for entry in entries {
                match &entry.key {
                    Some(key) => {
                        bytes.push(b'k');
                        bytes.extend_from_slice(&(key.len() as u64).to_le_bytes());
                        bytes.extend_from_slice(key.as_bytes());
                    }
                    None => bytes.push(b'u'),
                }
                canonical_value(&entry.value, bytes);
            }
            bytes.push(b'}');
        }
    }
}

fn parse_scripted_layer(
    id: &str,
    name: &str,
    band: crate::production_features::xmap05::Xmap05MapModeBand,
    value: Option<&Value>,
) -> Result<ScriptedLayerSpec> {
    let Some(value) = value else {
        return Ok(ScriptedLayerSpec {
            band,
            kind: ScriptedLayerKind::None,
            rgba: None,
        });
    };
    let fields = value
        .as_object()
        .ok_or_else(|| format!("XMAP18 scripted mode {id} {name} layer is not an object"))?;
    let mut field_names = BTreeSet::new();
    for field in fields {
        let key = field
            .key
            .as_deref()
            .ok_or_else(|| format!("XMAP18 scripted mode {id} {name} has an unkeyed field"))?;
        let canonical_key = key.to_ascii_lowercase();
        if !matches!(canonical_key.as_str(), "type" | "thickness" | "color")
            || !field_names.insert(canonical_key)
        {
            return Err(format!(
                "XMAP18 scripted mode {id} {name} has unsupported or duplicate field {key}"
            )
            .into());
        }
    }
    let layer_type = value
        .first_scalar_ci("type")
        .ok_or_else(|| format!("XMAP18 scripted mode {id} {name} layer has no type"))?;
    let kind = match layer_type {
        "none" => ScriptedLayerKind::None,
        "country" => ScriptedLayerKind::Country,
        "state" => ScriptedLayerKind::State,
        "game_map_mode_country" => ScriptedLayerKind::Hardcoded(HardcodedMapMode::Political),
        "game_map_mode_states" => ScriptedLayerKind::Hardcoded(HardcodedMapMode::States),
        "game_map_mode_diplomacy" => ScriptedLayerKind::Hardcoded(HardcodedMapMode::Diplomacy),
        "game_map_mode_players" => ScriptedLayerKind::Hardcoded(HardcodedMapMode::Players),
        "game_map_mode_factions" => ScriptedLayerKind::Hardcoded(HardcodedMapMode::Factions),
        "game_map_mode_ideology" => ScriptedLayerKind::Hardcoded(HardcodedMapMode::Ideology),
        "state_controller" => ScriptedLayerKind::StateController,
        other => {
            return Err(format!("XMAP18 scripted mode {id} has unsupported type {other}").into())
        }
    };
    if matches!(
        kind,
        ScriptedLayerKind::None | ScriptedLayerKind::Hardcoded(_)
    ) {
        if field_names.len() != 1 || !field_names.contains("type") {
            return Err(format!(
                "XMAP18 scripted mode {id} {name} hardcoded/none layer accepts only type"
            )
            .into());
        }
        return Ok(ScriptedLayerSpec {
            band,
            kind,
            rgba: None,
        });
    }
    if !matches!(value.first_scalar_ci("thickness"), None | Some("no")) {
        return Err(
            format!("XMAP18 scripted mode {id} requires raw target/thickness primitives").into(),
        );
    }
    let color = value
        .first_value_ci("color")
        .ok_or_else(|| format!("XMAP18 scripted mode {id} {name} layer has no color program"))?;
    Ok(ScriptedLayerSpec {
        band,
        kind,
        rgba: Some(parse_constant_scripted_color(id, color)?),
    })
}

fn parse_constant_scripted_color(id: &str, color: &Value) -> Result<[u8; 4]> {
    let mut channels = BTreeMap::<String, f32>::new();
    let mut saw_always = false;
    for entry in color.fields() {
        let statement = entry
            .key
            .as_deref()
            .ok_or_else(|| format!("XMAP18 scripted mode {id} color has an unkeyed statement"))?;
        if statement.eq_ignore_ascii_case("always")
            && entry.value.as_scalar() == Some("yes")
            && !saw_always
        {
            saw_always = true;
            continue;
        }
        if !entry.key_matches("set_temp_variable") {
            return Err(format!(
                "XMAP18 scripted mode {id} color program needs a raw primitive for an unsupported statement"
            )
            .into());
        }
        let fields = entry
            .value
            .as_object()
            .ok_or("XMAP18 scripted set_temp_variable is not an object")?;
        let mut inserted = false;
        for field in fields {
            let key = field
                .key
                .as_deref()
                .ok_or("XMAP18 scripted set_temp_variable has an unkeyed field")?;
            let canonical_key = key.to_ascii_lowercase();
            if !matches!(canonical_key.as_str(), "red" | "green" | "blue" | "alpha") {
                return Err(format!(
                    "XMAP18 scripted mode {id} set_temp_variable has unsupported field {key}"
                )
                .into());
            }
            let value = field
                .value
                .as_scalar()
                .and_then(|value| value.parse::<f32>().ok())
                .ok_or("XMAP18 scripted colour channel is not a numeric literal")?;
            if channels.insert(canonical_key, value).is_some() {
                return Err("XMAP18 scripted colour channel is assigned more than once".into());
            }
            inserted = true;
        }
        if !inserted {
            return Err(format!(
                "XMAP18 scripted mode {id} set_temp_variable is not a constant colour assignment"
            )
            .into());
        }
    }
    let values = ["red", "green", "blue", "alpha"].map(|key| {
        channels
            .get(key)
            .copied()
            .ok_or_else(|| format!("XMAP18 scripted mode {id} does not assign {key}"))
    });
    let values = values
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if values
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(format!("XMAP18 scripted mode {id} colour is outside normalized RGBA").into());
    }
    Ok(std::array::from_fn(|index| {
        (values[index] * 255.0).round() as u8
    }))
}

fn scripted_engine_configuration(
    resolver: &GameFileResolver,
    id: &str,
) -> Result<EngineMapModeConfiguration> {
    let (definition, definition_sha256) = scripted_definition_from_resolver(resolver, id)?;
    let scripted_layers = scripted_layer_identities(id, &definition);
    let (scripted_far_text, scripted_near_text, scripted_update_daily) =
        scripted_mode_level_configuration(id, &definition)?;
    Ok(EngineMapModeConfiguration {
        packed_layer_word: None,
        fill_channels: Vec::new(),
        fill_update_kinds: Vec::new(),
        manager_state_0x50: None,
        manager_state_0x54: None,
        layer0_outline_cutoff_define: None,
        layer0_camera_distance_override_define: None,
        layer0_flag_0x80: false,
        manager_flag_0x49: false,
        scripted_definition_sha256: Some(definition_sha256),
        scripted_layers,
        scripted_far_text: Some(scripted_far_text),
        scripted_near_text: Some(scripted_near_text),
        scripted_update_daily: Some(scripted_update_daily),
    })
}

fn scripted_mode_level_configuration(
    id: &str,
    definition: &Value,
) -> Result<(String, String, bool)> {
    let text_selector = |key: &str| -> Result<String> {
        let values = definition
            .fields()
            .iter()
            .filter(|entry| entry.key_matches(key))
            .collect::<Vec<_>>();
        let value = values
            .last()
            .map(|entry| {
                entry
                    .value
                    .as_scalar()
                    .ok_or_else(|| format!("XMAP18 scripted mode {id} {key} is not scalar"))
            })
            .transpose()?
            .unwrap_or("none")
            .to_ascii_lowercase();
        if !matches!(
            value.as_str(),
            "none" | "country" | "state" | "faction" | "player"
        ) {
            return Err(
                format!("XMAP18 scripted mode {id} has invalid {key} selector {value}").into(),
            );
        }
        Ok(value)
    };
    let updates = definition
        .fields()
        .iter()
        .filter(|entry| entry.key_matches("update_daily"))
        .collect::<Vec<_>>();
    let update_daily = match updates.last().map(|entry| entry.value.as_scalar()) {
        None => true,
        Some(Some(value)) if value.eq_ignore_ascii_case("yes") => true,
        Some(Some(value)) if value.eq_ignore_ascii_case("no") => false,
        _ => return Err(format!("XMAP18 scripted mode {id} update_daily is not yes/no").into()),
    };
    Ok((
        text_selector("far_text")?,
        text_selector("near_text")?,
        update_daily,
    ))
}

fn scripted_layer_identities(id: &str, definition: &Value) -> Vec<String> {
    match scripted_layer_specs(id, definition) {
        Ok(specs) => specs
            .iter()
            .map(|spec| {
                let band = match spec.band {
                    crate::production_features::xmap05::Xmap05MapModeBand::Top => "top",
                    crate::production_features::xmap05::Xmap05MapModeBand::Bottom => "bottom",
                };
                let kind = match spec.kind {
                    ScriptedLayerKind::None => "none".to_string(),
                    ScriptedLayerKind::Country => format!(
                        "country:{:02x}{:02x}{:02x}{:02x}",
                        spec.rgba.expect("country scripted layers have a colour")[0],
                        spec.rgba.expect("country scripted layers have a colour")[1],
                        spec.rgba.expect("country scripted layers have a colour")[2],
                        spec.rgba.expect("country scripted layers have a colour")[3]
                    ),
                    ScriptedLayerKind::State => format!(
                        "state:{:02x}{:02x}{:02x}{:02x}",
                        spec.rgba.expect("state scripted layers have a colour")[0],
                        spec.rgba.expect("state scripted layers have a colour")[1],
                        spec.rgba.expect("state scripted layers have a colour")[2],
                        spec.rgba.expect("state scripted layers have a colour")[3]
                    ),
                    ScriptedLayerKind::StateController => {
                        "state_controller:engine_evaluated".to_string()
                    }
                    ScriptedLayerKind::Hardcoded(mode) => format!("hardcoded:{}", mode.token()),
                };
                format!("{band}:{kind}")
            })
            .collect::<Vec<_>>(),
        Err(_) => vec![
            "top:engine_evaluated".to_string(),
            "bottom:engine_evaluated".to_string(),
        ],
    }
}

fn prepare_render_political(
    resolver: &GameFileResolver,
    selection: &ViewerMapModeSelection,
    _effective_scripted_mode_ids: &[String],
    context: &FoundationFeatureContext<'_>,
    base: &crate::PreparedPoliticalFrame,
) -> Result<(crate::PreparedPoliticalFrame, Option<String>)> {
    if matches!(selection, ViewerMapModeSelection::Scripted(_)) {
        return prepare_scripted_render(resolver, selection, context, base);
    }
    let ViewerMapModeSelection::Hardcoded(mode) = selection else {
        unreachable!();
    };
    if matches!(mode, HardcodedMapMode::Political | HardcodedMapMode::Supply) {
        return Ok((base.clone(), None));
    }
    if *mode == HardcodedMapMode::Terrain {
        let fills =
            crate::map_mode_terrain_resource::terrain_province_rgba_from_resolver(resolver)?;
        return Ok((
            crate::production_features::xmap05::xmap05_terrain_map_mode_derivative(
                resolver, context, base, &fills,
            )?,
            None,
        ));
    }
    if *mode == HardcodedMapMode::Diplomacy {
        return Ok((
            crate::production_features::xmap05::xmap05_diplomacy_configuration_derivative(
                resolver, base,
            )?,
            None,
        ));
    }
    if matches!(
        mode,
        HardcodedMapMode::StrategicAir
            | HardcodedMapMode::StrategicNavy
            | HardcodedMapMode::Operatives
    ) {
        return raw_runtime_layers(resolver, *mode, context, base);
    }
    if matches!(mode, HardcodedMapMode::Resources | HardcodedMapMode::Raids) {
        return Ok((base.clone(), None));
    }

    let definition_csv =
        resolver.resolve_required("map/definition.csv", "XMAP18 province definitions")?;
    let (definitions, _) = crate::load_definition_csv(&definition_csv)?;
    let state_paths = resolver.resolve_dir_files_recursive("history/states", Some("txt"))?;
    let (states, province_to_state, _) = crate::load_states(&state_paths)?;
    let eligible = definitions
        .iter()
        .filter_map(|(&province_id, definition)| {
            (definition.province_type == "land" && province_to_state.contains_key(&province_id))
                .then_some(province_id)
        })
        .collect::<BTreeSet<_>>();
    let mut fills = eligible
        .iter()
        .map(|province_id| (*province_id, None))
        .collect::<BTreeMap<_, _>>();

    match mode {
        HardcodedMapMode::Resistance | HardcodedMapMode::Compliance => {
            let graphics = effective_directory_source(
                resolver,
                "common/defines",
                Some("lua"),
                "XMAP18 graphics defines",
            )?;
            let palette = crate::occupation_palette_from_source(&graphics)?;
            let occupation_mode = if *mode == HardcodedMapMode::Resistance {
                crate::OccupationMapMode::Resistance
            } else {
                crate::OccupationMapMode::Compliance
            };
            for province_id in &eligible {
                let state_id = province_to_state[province_id];
                let rgba = match context.world_frame.occupation(state_id) {
                    Some(row) => {
                        let level = match occupation_mode {
                            crate::OccupationMapMode::Resistance => &row.resistance,
                            crate::OccupationMapMode::Compliance => &row.compliance,
                        };
                        let Leaf::Measured(level) = level else {
                            return Err(format!(
                                "XMAP18 occupation level for state {state_id} is not measured"
                            )
                            .into());
                        };
                        crate::occupation_mapmode_color(&palette, occupation_mode, *level as f32)
                            .ok_or_else(|| {
                                format!("XMAP18 occupation colour failed for state {state_id}")
                            })?
                    }
                    None => palette.non_initialized,
                };
                fills.insert(
                    *province_id,
                    Some((format!("occupation-state:{state_id}"), unit_rgba(rgba)?)),
                );
            }
        }
        HardcodedMapMode::Manpower => {
            let source = effective_directory_source(
                resolver,
                "common/defines",
                Some("lua"),
                "XMAP18 manpower defines",
            )?;
            let maximum = crate::parse_lua_number_define(&source, "MAP_MODE_MANPOWER_RANGE_MAX")?;
            if !maximum.is_finite() || maximum <= 0.0 {
                return Err("XMAP18 manpower range maximum is not positive".into());
            }
            let from = normalized_color4(&source, "MAP_MODE_MANPOWER_RANGE_COLOR_FROM")?;
            let to = normalized_color4(&source, "MAP_MODE_MANPOWER_RANGE_COLOR_TO")?;
            apply_state_rgba_fills_f32(
                &eligible,
                &province_to_state,
                &mut fills,
                "manpower",
                |state_id| {
                    let manpower = states.get(&state_id)?.manpower as f32;
                    let t = (manpower / maximum).clamp(0.0, 1.0);
                    Some(std::array::from_fn(|index| {
                        from[index] + (to[index] - from[index]) * t
                    }))
                },
            )?;
        }
        HardcodedMapMode::States => {
            let owners = measured_owner_by_state(context)?;
            let colors = crate::country_colors_from_game_resolver_with_session_metadata(
                resolver,
                context.world_frame.session_metadata(),
            )?;
            let state_fills = crate::states_admin_fill(&owners, &colors);
            apply_state_rgba_fills_u8(
                &eligible,
                &province_to_state,
                &mut fills,
                "state-owner",
                |state_id| state_fills.get(&state_id).and_then(|row| row.fill_rgba),
            )?;
        }
        HardcodedMapMode::Players => {
            let owners = measured_owner_by_state(context)?;
            let colors = crate::country_colors_from_game_resolver_with_session_metadata(
                resolver,
                context.world_frame.session_metadata(),
            )?;
            let graphics = effective_directory_source(
                resolver,
                "common/defines",
                Some("lua"),
                "XMAP18 player-mode defines",
            )?;
            let defines = crate::admin_mapmode_defines_from_source(&graphics)?;
            let Leaf::Measured(players) = &context.world_frame.session_metadata().player_countries
            else {
                return Err("XMAP18 player-country set is not measured".into());
            };
            // A repeated TAG is co-op, not a defect.
            //
            // This used to error on `!player_tags.insert(tag)`. But what this
            // mode paints is set MEMBERSHIP — `players_fill` asks only whether a
            // state's owner is in `player_tags` — and two humans sharing one
            // country is an ordinary multiplayer state the D57 producer emits
            // deliberately (it sorts on `(country_tag, tag_db_id, human_name)`
            // precisely because the country alone is not a key). Erroring on it
            // refused to draw a legitimate co-op game over a duplicate that
            // folds into the same answer (lane PL, 2026-08-21).
            //
            // A blank tag is still a defect: it is not a country.
            let mut player_tags = BTreeSet::new();
            for player in players {
                let Leaf::Measured(tag) = &player.tag else {
                    return Err("XMAP18 player-country row has no measured tag".into());
                };
                if tag.trim().is_empty() {
                    return Err("XMAP18 player-country set has a blank tag".into());
                }
                player_tags.insert(tag.tag().to_string());
            }
            let state_fills = crate::players_fill(&owners, &player_tags, &colors, &defines);
            apply_state_rgba_fills_u8(
                &eligible,
                &province_to_state,
                &mut fills,
                "players",
                |state_id| state_fills.get(&state_id).and_then(|row| row.fill_rgba),
            )?;
        }
        HardcodedMapMode::Ideology => {
            let owners = measured_owner_by_state(context)?;
            let mut ruling = BTreeMap::new();
            for (tag, identity) in context.world_frame.country_identities() {
                let Leaf::Measured(ideology) = &identity.ruling_ideology else {
                    return Err(format!(
                        "XMAP18 ruling ideology for country {tag} is not measured"
                    )
                    .into());
                };
                if ideology.trim().is_empty() {
                    return Err(format!("XMAP18 ruling ideology for country {tag} is blank").into());
                }
                ruling.insert(tag.clone(), ideology.clone());
            }
            let colors = crate::ideology_colors_from_resolver(resolver)?;
            let graphics = effective_directory_source(
                resolver,
                "common/defines",
                Some("lua"),
                "XMAP18 ideology-mode defines",
            )?;
            let defines = crate::admin_mapmode_defines_from_source(&graphics)?;
            let state_fills = crate::ideology_fill(&owners, &ruling, &colors, &defines);
            apply_state_rgba_fills_u8(
                &eligible,
                &province_to_state,
                &mut fills,
                "ideology",
                |state_id| state_fills.get(&state_id).and_then(|row| row.fill_rgba),
            )?;
        }
        HardcodedMapMode::Infrastructure => {
            let graphics = effective_directory_source(
                resolver,
                "common/defines",
                Some("lua"),
                "XMAP18 infrastructure defines",
            )?;
            let colors = crate::infra_gradient_colors_from_source(&graphics)?;
            let buildings = effective_directory_source(
                resolver,
                "common/buildings",
                Some("txt"),
                "XMAP18 building definitions",
            )?;
            let state_max = crate::parse_infrastructure_state_max(&buildings)
                .ok_or("XMAP18 effective building database has no infrastructure state_max")?;
            let inputs =
                infrastructure_level_inputs(context.world_frame, states.keys().copied())?;
            let state_fills = crate::infra_state_tints(&inputs, state_max, &colors);
            apply_state_rgba_fills_f32(
                &eligible,
                &province_to_state,
                &mut fills,
                "infrastructure",
                |state_id| state_fills.get(&state_id).and_then(|row| row.fill_rgba),
            )?;
        }
        HardcodedMapMode::Factions => {
            let owners = measured_owner_by_state(context)?;
            let colors = crate::country_colors_from_game_resolver_with_session_metadata(
                resolver,
                context.world_frame.session_metadata(),
            )?;
            let graphics = effective_directory_source(
                resolver,
                "common/defines",
                Some("lua"),
                "XMAP18 faction-mode defines",
            )?;
            let defines = crate::factions_defines_from_source(&graphics)?;
            let diplomacy = context.world_frame.diplomacy();
            let Leaf::Measured(roster) = &diplomacy.faction_members else {
                return Err("XMAP18 faction roster is not measured".into());
            };
            let mut factions = Vec::with_capacity(roster.len());
            for (faction_id, members) in roster {
                let mut leader = None;
                for (tag, country) in &diplomacy.countries {
                    if !matches!(&country.faction_name, Leaf::Measured(name) if name == faction_id)
                    {
                        continue;
                    }
                    match &country.faction_is_leader {
                        Leaf::Measured(true) if leader.replace(tag.clone()).is_some() => {
                            return Err(format!(
                                "XMAP18 faction {faction_id} has multiple measured leaders"
                            )
                            .into())
                        }
                        Leaf::Measured(_) => {}
                        _ => {
                            return Err(format!(
                                "XMAP18 faction leadership for country {tag} is not measured"
                            )
                            .into())
                        }
                    }
                }
                if leader.is_none() {
                    return Err(
                        format!("XMAP18 faction {faction_id} has no measured leader").into(),
                    );
                }
                factions.push(crate::FactionInput {
                    faction_id: faction_id.clone(),
                    leader,
                    members: members.clone(),
                });
            }
            let memberships = Some(
                crate::country_faction_memberships(diplomacy.faction_roster_status(), &factions)
                    .ok_or("XMAP18 faction roster failed exact membership validation")?,
            );
            apply_state_rgba_fills_u8(
                &eligible,
                &province_to_state,
                &mut fills,
                "factions",
                |state_id| {
                    let owner = owners.get(&state_id)?;
                    crate::faction_fill(
                        &crate::membership_for(owner, &memberships),
                        &colors,
                        &defines,
                    )
                    .map(|mut rgba| {
                        rgba[3] = u8::MAX;
                        rgba
                    })
                },
            )?;
        }
        _ => {
            return Err(format!(
                "XMAP18 {} mode has no complete authenticated renderer composition",
                mode.token()
            )
            .into())
        }
    }
    Ok((
        crate::production_features::xmap05::xmap05_map_mode_derivative(
            resolver,
            context,
            base,
            hardcoded_render_band(*mode),
            hardcoded_render_mask(*mode),
            &fills,
        )?,
        None,
    ))
}

/// The infrastructure level for every state the effective VFS defines, COUNTED
/// rather than stopped at the first refusal, and refusing by state id.
pub(crate) fn infrastructure_level_inputs(
    frame: &replay_core::WorldFrame,
    vfs_state_ids: impl Iterator<Item = u32>,
) -> Result<BTreeMap<u32, crate::InfraLevelInput>> {
    const NAMED_REFUSALS: usize = 8;
    let mut inputs = BTreeMap::new();
    let mut unmeasured_scope = Vec::new();
    let mut unmeasured_level = Vec::new();
    let mut expected = 0usize;
    for state_id in vfs_state_ids {
        expected += 1;
        if !crate::map_authority::xmap18_state_building_scope_is_whole(frame, state_id) {
            unmeasured_scope.push(state_id);
            continue;
        }
        if !crate::map_authority::xmap18_infrastructure_level_is_measured(frame, state_id) {
            unmeasured_level.push(state_id);
            continue;
        }
        let level = frame
            .buildings_by_state()
            .get(&state_id)
            .and_then(|state| state.state_buildings.get("infrastructure"))
            .and_then(|row| match row.level {
                Leaf::Measured(level) => Some(level),
                _ => None,
            })
            .unwrap_or(0.0);
        inputs.insert(state_id, crate::InfraLevelInput { level: Some(level) });
    }
    if inputs.len() != expected {
        let named = |states: &[u32]| {
            states
                .iter()
                .take(NAMED_REFUSALS)
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        };
        return Err(format!(
            "XMAP18 infrastructure joined {} of the {expected} states the effective VFS defines: \
             {} carry no whole D48 building scope ({}), {} carry an unmeasured infrastructure level ({})",
            inputs.len(),
            unmeasured_scope.len(),
            named(&unmeasured_scope),
            unmeasured_level.len(),
            named(&unmeasured_level),
        )
        .into());
    }
    Ok(inputs)
}

fn raw_runtime_layers(
    resolver: &GameFileResolver,
    mode: HardcodedMapMode,
    context: &FoundationFeatureContext<'_>,
    base: &crate::PreparedPoliticalFrame,
) -> Result<(crate::PreparedPoliticalFrame, Option<String>)> {
    let value = context
        .world_frame
        .domain("map_mode_runtime")
        .ok_or_else(|| format!("XMAP18 {} requires raw map-mode matrices", mode.token()))?;
    let runtime = decode_bounded_runtime_input(value)?;
    if runtime.hour_index != context.world_frame.hour_index() {
        return Err("XMAP18 raw map-mode matrix belongs to a different replay hour".into());
    }
    if runtime.effective_vfs_manifest_sha256 != context.effective_vfs_manifest_sha256 {
        return Err("XMAP18 raw map-mode matrix belongs to a different effective VFS".into());
    }
    // COMPLETE, not partial: the runtime's observer census is compared for
    // EQUALITY against this list, which is an absence claim about the country
    // set ("no observer outside these"). A roster short by a country the frame
    // carries cannot back it, and the refusal names the countries.
    let roster = context.world_frame.d18_country_roster();
    let expected_observers = match roster.complete() {
        Ok(entries) => entries
            .iter()
            .map(|entry| entry.tag.clone())
            .collect::<Vec<_>>(),
        Err(refusal) => {
            return Err(format!(
                "XMAP18 requires the complete current-hour D18 country roster: {refusal}"
            )
            .into())
        }
    };
    if expected_observers.is_empty()
        || runtime.observer_country_ids != expected_observers
        || !runtime
            .observer_country_ids
            .windows(2)
            .all(|window| window[0] < window[1])
    {
        return Err("XMAP18 raw matrix observer census differs from complete D18 order".into());
    }
    let region_paths = resolver.resolve_dir_files_recursive("map/strategicregions", Some("txt"))?;
    let (region_to_provinces, _, _) = crate::load_strategic_regions(&region_paths)?;
    let expected_regions = region_to_provinces.keys().copied().collect::<Vec<_>>();
    if runtime.strategic_region_ids != expected_regions
        || !runtime
            .strategic_region_ids
            .windows(2)
            .all(|window| window[0] < window[1])
    {
        return Err("XMAP18 raw matrix region census differs from the effective VFS".into());
    }
    let state_paths = resolver.resolve_dir_files_recursive("history/states", Some("txt"))?;
    let (states, province_to_state, _) = crate::load_states(&state_paths)?;
    let expected_states = states.keys().copied().collect::<Vec<_>>();
    if runtime.state_ids != expected_states
        || !runtime
            .state_ids
            .windows(2)
            .all(|window| window[0] < window[1])
    {
        return Err("XMAP18 raw matrix state census differs from the effective VFS".into());
    }
    validate_sparse_runtime(&runtime)?;
    let projected = projected_runtime_bytes(&runtime)?;
    if runtime.projected_packed_bytes != projected
        || runtime.deterministic_sha256 != raw_runtime_digest(&runtime)
    {
        return Err("XMAP18 raw matrix projection or deterministic identity mismatch".into());
    }
    let observer = context
        .viewer
        .observer_country_id
        .as_deref()
        .ok_or_else(|| format!("XMAP18 {} requires a viewer country", mode.token()))?;
    let observer_index = runtime
        .observer_country_ids
        .iter()
        .position(|candidate| candidate == observer)
        .ok_or("XMAP18 raw matrix has no selected observer")?;
    let mut fills = BTreeMap::<u32, Option<(String, [u8; 4])>>::new();

    match mode {
        HardcodedMapMode::StrategicAir => {
            let graphics = effective_directory_source(
                resolver,
                "common/defines",
                Some("lua"),
                "XMAP18 strategic-air defines",
            )?;
            let colors = crate::strategic_air_colors_from_source(&graphics)?;
            for (region_index, region_id) in runtime.strategic_region_ids.iter().enumerate() {
                let (friendly, enemy) = air_weights_for(&runtime, observer_index, region_index);
                let friendly = friendly as i128;
                let enemy = enemy as i128;
                let color = if friendly == 0 && enemy == 0 {
                    colors.neutral
                } else {
                    let ratio_fixed5 = friendly
                        .checked_mul(100_000)
                        .and_then(|value| value.checked_div(friendly + enemy))
                        .ok_or("XMAP18 strategic-air ratio overflow")?;
                    if ratio_fixed5 > 60_000 {
                        colors.good
                    } else if ratio_fixed5 < 40_000 {
                        colors.bad
                    } else {
                        colors.average
                    }
                };
                let rgba = unit_rgba(color)?;
                for province_id in region_to_provinces
                    .get(region_id)
                    .ok_or("XMAP18 strategic-air region lost its province set")?
                {
                    if fills
                        .insert(
                            *province_id,
                            Some((format!("air-region:{region_id}"), rgba)),
                        )
                        .is_some()
                    {
                        return Err("XMAP18 strategic-air regions overlap on one province".into());
                    }
                }
            }
        }
        HardcodedMapMode::StrategicNavy => {
            let regions = crate::naval_region_set_from_resolver(resolver)?;
            for (region_index, region_id) in runtime.strategic_region_ids.iter().enumerate() {
                let (control, contested) = naval_codes_for(&runtime, observer_index, region_index);
                let Some(zone) = regions.zones.get(region_id) else {
                    if (control, contested) == (0, 0) {
                        continue;
                    }
                    return Err(format!(
                        "XMAP18 strategic region {region_id} has non-default naval codes but no sea geometry"
                    )
                    .into());
                };
                let rgba = [control, 0, 0, contested];
                for province_id in &zone.sea_provinces {
                    if fills
                        .insert(
                            *province_id,
                            Some((format!("naval-region:{region_id}"), rgba)),
                        )
                        .is_some()
                    {
                        return Err("XMAP18 naval regions overlap on one province".into());
                    }
                }
            }
        }
        HardcodedMapMode::Operatives => {
            let graphics = effective_directory_source(
                resolver,
                "common/defines",
                Some("lua"),
                "XMAP18 operative-network defines",
            )?;
            let normal_low =
                normalized_color4(&graphics, "MAP_MODE_INTEL_NETWORK_STRENGTH_COLOR_LOW")?;
            let normal_high =
                normalized_color4(&graphics, "MAP_MODE_INTEL_NETWORK_STRENGTH_COLOR_HIGH")?;
            let quiet_low =
                normalized_color4(&graphics, "MAP_MODE_INTEL_NETWORK_STRENGTH_QUIET_COLOR_LOW")?;
            let quiet_high = normalized_color4(
                &graphics,
                "MAP_MODE_INTEL_NETWORK_STRENGTH_QUIET_COLOR_HIGH",
            )?;
            fills.extend(
                province_to_state
                    .keys()
                    .map(|province_id| (*province_id, None)),
            );
            for cell in runtime
                .operative_cells
                .iter()
                .filter(|cell| cell.observer_index as usize == observer_index)
            {
                let state_id = runtime.state_ids[cell.state_index as usize];
                let strength = cell.strength_raw;
                let quiet = cell.quiet_raw == 1;
                let (low, high) = if quiet {
                    (quiet_low, quiet_high)
                } else {
                    (normal_low, normal_high)
                };
                let factor = strength as f64 / runtime.operative_strength_scale as f64;
                let rgba = unit_rgba(std::array::from_fn(|index| {
                    (f64::from(low[index])
                        + (f64::from(high[index]) - f64::from(low[index])) * factor)
                        as f32
                }))?;
                for (&province_id, &province_state_id) in &province_to_state {
                    if province_state_id == state_id {
                        fills.insert(
                            province_id,
                            Some((format!("operative-network-state:{state_id}"), rgba)),
                        );
                    }
                }
            }
        }
        _ => return Err("XMAP18 raw matrix requested for a non-matrix mode".into()),
    }

    let rendered = if mode == HardcodedMapMode::StrategicNavy {
        crate::production_features::xmap05::xmap05_naval_dominance_derivative(
            resolver, context, base, &fills,
        )?
    } else {
        crate::production_features::xmap05::xmap05_evaluated_map_mode_derivative(
            resolver,
            context,
            base,
            match mode {
                HardcodedMapMode::Operatives => {
                    crate::production_features::xmap05::Xmap05MapModeBand::Bottom
                }
                _ => crate::production_features::xmap05::Xmap05MapModeBand::Top,
            },
            match mode {
                HardcodedMapMode::Operatives => {
                    crate::production_features::xmap05::Xmap05MapModeMask::StateDistance
                }
                _ => crate::production_features::xmap05::Xmap05MapModeMask::Flat,
            },
            &fills,
        )?
    };
    Ok((rendered, Some(runtime.deterministic_sha256)))
}

fn decode_bounded_runtime_input(value: &serde_json::Value) -> Result<Xmap18RuntimeInput> {
    let serialized_bytes = serde_json::to_vec(value)?;
    if serialized_bytes.len() > XMAP18_MAX_RUNTIME_JSON_BYTES_PER_HOUR {
        return Err(format!(
            "XMAP18 raw map-mode carrier is {} bytes, above the {} byte production ceiling",
            serialized_bytes.len(),
            XMAP18_MAX_RUNTIME_JSON_BYTES_PER_HOUR
        )
        .into());
    }
    let runtime: Xmap18RuntimeInput = serde_json::from_value(value.clone())
        .map_err(|error| format!("XMAP18 raw map-mode matrix schema: {error}"))?;
    if runtime.schema != XMAP18_RUNTIME_INPUT_SCHEMA || !runtime.complete {
        return Err("XMAP18 raw map-mode matrix is not a complete a729 carrier".into());
    }
    Ok(runtime)
}

fn decode_bounded_scripted_runtime_input(
    value: &serde_json::Value,
) -> Result<Xmap18ScriptedRuntimeInput> {
    let serialized_bytes = serde_json::to_vec(value)?;
    if serialized_bytes.len() > XMAP18_MAX_SCRIPTED_RUNTIME_JSON_BYTES_PER_HOUR {
        return Err(format!(
            "XMAP18 scripted runtime carrier is {} bytes, above the {} byte production ceiling",
            serialized_bytes.len(),
            XMAP18_MAX_SCRIPTED_RUNTIME_JSON_BYTES_PER_HOUR
        )
        .into());
    }
    let runtime: Xmap18ScriptedRuntimeInput = serde_json::from_value(value.clone())
        .map_err(|error| format!("XMAP18 scripted runtime scope schema: {error}"))?;
    if runtime.schema != XMAP18_SCRIPTED_RUNTIME_INPUT_SCHEMA || !runtime.complete {
        return Err("XMAP18 scripted runtime scopes are not a complete a729 carrier".into());
    }
    Ok(runtime)
}

fn validate_scripted_runtime(
    runtime: &Xmap18ScriptedRuntimeInput,
    resolver: &GameFileResolver,
    context: &FoundationFeatureContext<'_>,
    effective_definitions: &[(String, Value)],
) -> Result<()> {
    if runtime.hour_index != context.world_frame.hour_index()
        || runtime.effective_vfs_manifest_sha256 != context.effective_vfs_manifest_sha256
    {
        return Err("XMAP18 scripted runtime belongs to a different hour or effective VFS".into());
    }
    // COMPLETE, not partial: the runtime's observer census is compared for
    // EQUALITY against this list, which is an absence claim about the country
    // set ("no observer outside these"). A roster short by a country the frame
    // carries cannot back it, and the refusal names the countries.
    let roster = context.world_frame.d18_country_roster();
    let expected_observers = match roster.complete() {
        Ok(entries) => entries
            .iter()
            .map(|entry| entry.tag.clone())
            .collect::<Vec<_>>(),
        Err(refusal) => {
            return Err(format!(
                "XMAP18 requires the complete current-hour D18 country roster: {refusal}"
            )
            .into())
        }
    };
    if expected_observers.is_empty()
        || runtime.observer_country_ids != expected_observers
        || !runtime
            .observer_country_ids
            .windows(2)
            .all(|window| window[0] < window[1])
    {
        return Err(
            "XMAP18 scripted runtime observer census differs from complete D18 order".into(),
        );
    }
    let expected_scripted_mode_ids = effective_definitions
        .iter()
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    if runtime.scripted_mode_ids != expected_scripted_mode_ids
        || runtime
            .scripted_mode_ids
            .iter()
            .any(|id| id.trim().is_empty())
    {
        return Err("XMAP18 scripted runtime mode census differs from the effective VFS".into());
    }
    let state_paths = resolver.resolve_dir_files_recursive("history/states", Some("txt"))?;
    let (states, _, _) = crate::load_states(&state_paths)?;
    if runtime.state_ids != states.keys().copied().collect::<Vec<_>>()
        || !runtime
            .state_ids
            .windows(2)
            .all(|window| window[0] < window[1])
    {
        return Err("XMAP18 scripted runtime state census differs from the effective VFS".into());
    }
    let expected_entry_count = runtime
        .observer_country_ids
        .len()
        .checked_mul(runtime.scripted_mode_ids.len())
        .ok_or("XMAP18 scripted runtime census overflow")?;
    if runtime.entries.len() != expected_entry_count {
        return Err("XMAP18 scripted runtime is not a complete observer x mode census".into());
    }
    if runtime.layer_payloads.is_empty() {
        return Err("XMAP18 scripted runtime has no semantic payload dictionary".into());
    }
    let mut used_payloads = BTreeSet::new();
    let mut next_first_use = 0u32;
    for (ordinal, entry) in runtime.entries.iter().enumerate() {
        let expected_observer = ordinal / runtime.scripted_mode_ids.len();
        let expected_mode = ordinal % runtime.scripted_mode_ids.len();
        if entry.observer_index as usize != expected_observer
            || entry.scripted_mode_index as usize != expected_mode
        {
            return Err("XMAP18 scripted scope entry key is not canonical".into());
        }
        for payload_index in [entry.top_payload_index, entry.bottom_payload_index] {
            if usize::try_from(payload_index)? >= runtime.layer_payloads.len() {
                return Err("XMAP18 scripted entry names an absent payload".into());
            }
            if used_payloads.insert(payload_index) {
                if payload_index != next_first_use {
                    return Err(
                        "XMAP18 scripted payload dictionary is not in first-use order".into(),
                    );
                }
                next_first_use = next_first_use
                    .checked_add(1)
                    .ok_or("XMAP18 scripted payload index overflow")?;
            }
        }
    }
    if used_payloads.len() != runtime.layer_payloads.len() {
        return Err("XMAP18 scripted payload dictionary contains an unused row".into());
    }
    validate_scripted_runtime_scope_kinds(runtime, effective_definitions)?;
    for (index, payload) in runtime.layer_payloads.iter().enumerate() {
        if runtime.layer_payloads[..index].contains(payload) {
            return Err("XMAP18 scripted payload dictionary contains a duplicate".into());
        }
        validate_scripted_scope_payload(payload, runtime)?;
    }
    if runtime.projected_packed_bytes != projected_scripted_runtime_bytes(runtime)?
        || runtime.deterministic_sha256 != scripted_runtime_digest(runtime)
    {
        return Err("XMAP18 scripted runtime projection or deterministic identity mismatch".into());
    }
    Ok(())
}

fn validate_scripted_runtime_scope_kinds(
    runtime: &Xmap18ScriptedRuntimeInput,
    effective_definitions: &[(String, Value)],
) -> Result<()> {
    if runtime.scripted_mode_ids.len() != effective_definitions.len() {
        return Err("XMAP18 scripted runtime mode definitions are incomplete".into());
    }
    for entry in &runtime.entries {
        let mode_index = usize::try_from(entry.scripted_mode_index)?;
        let (mode_id, definition) = effective_definitions
            .get(mode_index)
            .ok_or("XMAP18 scripted entry names an absent effective definition")?;
        if runtime.scripted_mode_ids.get(mode_index) != Some(mode_id) {
            return Err("XMAP18 scripted runtime mode definition order differs".into());
        }
        let expected = scripted_runtime_scope_kinds(mode_id, definition)?;
        for (payload_index, expected_kind) in [entry.top_payload_index, entry.bottom_payload_index]
            .into_iter()
            .zip(expected)
        {
            let payload = runtime
                .layer_payloads
                .get(usize::try_from(payload_index)?)
                .ok_or("XMAP18 scripted entry names an absent layer payload")?;
            if payload.scope_kind != expected_kind {
                return Err(format!(
                    "XMAP18 scripted runtime scope kind differs from effective definition for mode {mode_id}"
                )
                .into());
            }
        }
    }
    Ok(())
}

fn validate_scripted_scope_payload(
    payload: &Xmap18ScriptedLayerPayload,
    runtime: &Xmap18ScriptedRuntimeInput,
) -> Result<()> {
    let semantic = matches!(
        payload.scope_kind,
        Xmap18ScriptedScopeKind::Country
            | Xmap18ScriptedScopeKind::State
            | Xmap18ScriptedScopeKind::StateController
    );
    if !semantic && !payload.rows.is_empty() {
        return Err("XMAP18 none/hardcoded scripted payload contains semantic rows".into());
    }
    let mut prior = None;
    for row in &payload.rows {
        let key = (row.scope_index, row.controller_index.unwrap_or(u32::MAX));
        if prior.is_some_and(|prior| prior >= key) {
            return Err("XMAP18 scripted scope rows are duplicated or out of order".into());
        }
        prior = Some(key);
        match payload.scope_kind {
            Xmap18ScriptedScopeKind::Country => {
                if row.controller_index.is_some()
                    || usize::try_from(row.scope_index)? >= runtime.observer_country_ids.len()
                {
                    return Err("XMAP18 scripted country scope index is invalid".into());
                }
            }
            Xmap18ScriptedScopeKind::State => {
                if row.controller_index.is_some()
                    || usize::try_from(row.scope_index)? >= runtime.state_ids.len()
                {
                    return Err("XMAP18 scripted state scope index is invalid".into());
                }
            }
            Xmap18ScriptedScopeKind::StateController => {
                if usize::try_from(row.scope_index)? >= runtime.state_ids.len()
                    || usize::try_from(
                        row.controller_index
                            .ok_or("XMAP18 scripted state_controller row has no controller")?,
                    )? >= runtime.observer_country_ids.len()
                {
                    return Err("XMAP18 scripted state_controller scope index is invalid".into());
                }
            }
            _ => unreachable!(),
        }
        match (
            row.gradient_center_f32_bits,
            row.gradient_thickness_f32_bits,
        ) {
            (None, None) => {}
            (Some(center), Some(thickness))
                if f32::from_bits(center).is_finite()
                    && f32::from_bits(center) >= 0.0
                    && f32::from_bits(thickness).is_finite()
                    && f32::from_bits(thickness) > 0.0 => {}
            _ => return Err("XMAP18 scripted scope gradient parameters are invalid".into()),
        }
    }
    Ok(())
}

fn projected_scripted_runtime_bytes(runtime: &Xmap18ScriptedRuntimeInput) -> Result<u64> {
    let census_bytes = runtime
        .observer_country_ids
        .iter()
        .chain(runtime.scripted_mode_ids.iter())
        .try_fold(0u64, |sum, id| {
            sum.checked_add(u64::try_from(id.len()).ok()?.checked_add(1)?)
        })
        .ok_or("XMAP18 scripted runtime census size projection overflow")?;
    let census_bytes = census_bytes
        .checked_add(
            u64::try_from(runtime.state_ids.len())?
                .checked_mul(4)
                .ok_or("XMAP18 scripted state census size overflow")?,
        )
        .ok_or("XMAP18 scripted census size overflow")?;
    let entry_bytes = u64::try_from(runtime.entries.len())?
        .checked_mul(16)
        .ok_or("XMAP18 scripted entry size overflow")?;
    runtime.layer_payloads.iter().try_fold(
        census_bytes
            .checked_add(entry_bytes)
            .ok_or("XMAP18 scripted projection overflow")?,
        |sum, payload| {
            sum.checked_add(1)
                .and_then(|bytes| {
                    bytes.checked_add(u64::try_from(payload.rows.len()).ok()?.checked_mul(20)?)
                })
                .ok_or_else(|| "XMAP18 scripted runtime size projection overflow".into())
        },
    )
}

fn air_weights_for(
    runtime: &Xmap18RuntimeInput,
    observer_index: usize,
    strategic_region_index: usize,
) -> (i64, i64) {
    runtime
        .air_cells
        .iter()
        .find(|cell| {
            cell.observer_index as usize == observer_index
                && cell.strategic_region_index as usize == strategic_region_index
        })
        .map_or((0, 0), |cell| {
            (cell.friendly_weight_raw, cell.enemy_weight_raw)
        })
}

fn naval_codes_for(
    runtime: &Xmap18RuntimeInput,
    observer_index: usize,
    strategic_region_index: usize,
) -> (u8, u8) {
    runtime
        .naval_cells
        .iter()
        .find(|cell| {
            cell.observer_index as usize == observer_index
                && cell.strategic_region_index as usize == strategic_region_index
        })
        .map_or((0, 0), |cell| {
            (cell.control_code_raw, cell.contested_code_raw)
        })
}

fn validate_sparse_runtime(runtime: &Xmap18RuntimeInput) -> Result<()> {
    let observers = runtime.observer_country_ids.len();
    let regions = runtime.strategic_region_ids.len();
    let states = runtime.state_ids.len();
    let mut previous = None;
    for cell in &runtime.air_cells {
        let key = (cell.observer_index, cell.strategic_region_index);
        if previous.is_some_and(|candidate| candidate >= key)
            || cell.observer_index as usize >= observers
            || cell.strategic_region_index as usize >= regions
            || cell.friendly_weight_raw < 0
            || cell.enemy_weight_raw < 0
            || (cell.friendly_weight_raw == 0 && cell.enemy_weight_raw == 0)
        {
            return Err("XMAP18 sparse air cells are not canonical".into());
        }
        previous = Some(key);
    }
    previous = None;
    for cell in &runtime.naval_cells {
        let key = (cell.observer_index, cell.strategic_region_index);
        if previous.is_some_and(|candidate| candidate >= key)
            || cell.observer_index as usize >= observers
            || cell.strategic_region_index as usize >= regions
            || !matches!(cell.control_code_raw, 0 | 128 | 255)
            || !matches!(cell.contested_code_raw, 0 | 128 | 255)
            || (cell.control_code_raw == 0 && cell.contested_code_raw == 0)
        {
            return Err("XMAP18 sparse naval cells are not canonical".into());
        }
        previous = Some(key);
    }
    if runtime.operative_strength_scale <= 0 {
        return Err("XMAP18 operative strength scale is not positive".into());
    }
    previous = None;
    for cell in &runtime.operative_cells {
        let key = (cell.observer_index, cell.state_index);
        if previous.is_some_and(|candidate| candidate >= key)
            || cell.observer_index as usize >= observers
            || cell.state_index as usize >= states
            || !(0..=runtime.operative_strength_scale).contains(&cell.strength_raw)
            || cell.quiet_raw > 1
        {
            return Err("XMAP18 sparse operative cells are not canonical".into());
        }
        previous = Some(key);
    }
    Ok(())
}

fn projected_runtime_bytes(runtime: &Xmap18RuntimeInput) -> Result<u64> {
    let observer_bytes = runtime
        .observer_country_ids
        .iter()
        .try_fold(0u64, |sum, id| {
            sum.checked_add(u64::try_from(id.len()).ok()?.checked_add(1)?)
        })
        .ok_or("XMAP18 observer census size projection overflow")?;
    observer_bytes
        .checked_add(
            u64::try_from(runtime.strategic_region_ids.len())?
                .checked_mul(4)
                .ok_or("XMAP18 region census size projection overflow")?,
        )
        .and_then(|bytes| {
            bytes.checked_add(
                u64::try_from(runtime.state_ids.len())
                    .ok()?
                    .checked_mul(4)?,
            )
        })
        .and_then(|bytes| bytes.checked_add(8))
        .and_then(|bytes| {
            bytes.checked_add(
                u64::try_from(runtime.air_cells.len())
                    .ok()?
                    .checked_mul(24)?,
            )
        })
        .and_then(|bytes| {
            bytes.checked_add(
                u64::try_from(runtime.naval_cells.len())
                    .ok()?
                    .checked_mul(10)?,
            )
        })
        .and_then(|bytes| {
            bytes.checked_add(
                u64::try_from(runtime.operative_cells.len())
                    .ok()?
                    .checked_mul(17)?,
            )
        })
        .ok_or_else(|| "XMAP18 compact carrier size projection overflow".into())
}

fn hardcoded_render_band(
    mode: HardcodedMapMode,
) -> crate::production_features::xmap05::Xmap05MapModeBand {
    use crate::production_features::xmap05::Xmap05MapModeBand::{Bottom, Top};
    match mode {
        HardcodedMapMode::Resistance | HardcodedMapMode::Compliance => Bottom,
        HardcodedMapMode::States
        | HardcodedMapMode::Ideology
        | HardcodedMapMode::Factions
        | HardcodedMapMode::Players => Top,
        HardcodedMapMode::Infrastructure | HardcodedMapMode::Manpower => Top,
        _ => unreachable!("only materialized hardcoded modes request a render band"),
    }
}

fn hardcoded_render_mask(
    mode: HardcodedMapMode,
) -> crate::production_features::xmap05::Xmap05MapModeMask {
    use crate::production_features::xmap05::Xmap05MapModeMask::{
        CountryDistance, Flat, StateDistance,
    };
    match mode {
        HardcodedMapMode::States => StateDistance,
        HardcodedMapMode::Factions | HardcodedMapMode::Players | HardcodedMapMode::Ideology => {
            CountryDistance
        }
        HardcodedMapMode::Resistance
        | HardcodedMapMode::Compliance
        | HardcodedMapMode::Infrastructure
        | HardcodedMapMode::Manpower => Flat,
        _ => unreachable!("only materialized hardcoded modes request a render mask"),
    }
}

fn measured_owner_by_state(
    context: &FoundationFeatureContext<'_>,
) -> Result<BTreeMap<u32, String>> {
    let mut owners = BTreeMap::new();
    for (&state_id, owner) in context.world_frame.owner_by_state() {
        let Leaf::Measured(owner) = owner else {
            return Err(format!("XMAP18 owner for state {state_id} is not measured").into());
        };
        if owner.trim().is_empty() {
            return Err(format!("XMAP18 owner for state {state_id} is blank").into());
        }
        owners.insert(state_id, owner.tag().to_string());
    }
    Ok(owners)
}

fn apply_state_rgba_fills_u8(
    eligible: &BTreeSet<u32>,
    province_to_state: &BTreeMap<u32, u32>,
    fills: &mut BTreeMap<u32, Option<(String, [u8; 4])>>,
    region_prefix: &str,
    mut color: impl FnMut(u32) -> Option<[u8; 4]>,
) -> Result<()> {
    for province_id in eligible {
        let state_id = province_to_state[province_id];
        let rgba = color(state_id).ok_or_else(|| {
            format!("XMAP18 {region_prefix} mode has no fill for state {state_id}")
        })?;
        fills.insert(
            *province_id,
            Some((format!("{region_prefix}:{state_id}"), rgba)),
        );
    }
    Ok(())
}

fn apply_state_rgba_fills_f32(
    eligible: &BTreeSet<u32>,
    province_to_state: &BTreeMap<u32, u32>,
    fills: &mut BTreeMap<u32, Option<(String, [u8; 4])>>,
    region_prefix: &str,
    mut color: impl FnMut(u32) -> Option<[f32; 4]>,
) -> Result<()> {
    for province_id in eligible {
        let state_id = province_to_state[province_id];
        let rgba = color(state_id).ok_or_else(|| {
            format!("XMAP18 {region_prefix} mode has no fill for state {state_id}")
        })?;
        fills.insert(
            *province_id,
            Some((format!("{region_prefix}:{state_id}"), unit_rgba(rgba)?)),
        );
    }
    Ok(())
}

fn unit_rgba(rgba: [f32; 4]) -> Result<[u8; 4]> {
    if rgba
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err("XMAP18 file-derived colour is outside normalized RGBA".into());
    }
    Ok(std::array::from_fn(|index| {
        (rgba[index] * 255.0).round() as u8
    }))
}

fn normalized_color4(source: &str, key: &str) -> Result<[f32; 4]> {
    let values = crate::map_furniture::parse_lua_number_list(source, key)
        .ok_or_else(|| format!("XMAP18 graphics defines omit {key}"))?;
    if values.len() != 4 {
        return Err(format!("XMAP18 {key} has {} values instead of four", values.len()).into());
    }
    let color = [values[0], values[1], values[2], values[3]];
    unit_rgba(color)?;
    Ok(color)
}

fn prepare_fill_source(
    selection: &ViewerMapModeSelection,
    political: &crate::PreparedPoliticalFrame,
    runtime_payload_sha256: Option<&str>,
) -> Result<MapModeFillSource> {
    political.validate()?;
    if let Some(runtime_payload_sha256) = runtime_payload_sha256 {
        return Ok(match selection {
            ViewerMapModeSelection::Scripted(_) => {
                MapModeFillSource::EngineEvaluatedScriptedScopes {
                    runtime_scopes_sha256: runtime_payload_sha256.to_string(),
                }
            }
            ViewerMapModeSelection::Hardcoded(_) => MapModeFillSource::RawSemanticMatrix {
                runtime_matrix_sha256: runtime_payload_sha256.to_string(),
            },
        });
    }
    match selection {
        ViewerMapModeSelection::Scripted(_) | ViewerMapModeSelection::Hardcoded(_) => {
            Ok(MapModeFillSource::OwnerResources {
                resources: xmap05_owner_resources(political),
            })
        }
    }
}

fn xmap05_owner_resources(
    political: &crate::PreparedPoliticalFrame,
) -> Vec<Xmap18OwnerResourceRef> {
    [
        (
            Xmap18OwnerResourceRole::Xmap05ProvincePrimary,
            &political.province_primary,
        ),
        (
            Xmap18OwnerResourceRole::Xmap05ProvinceSecondary,
            &political.province_secondary,
        ),
        (
            Xmap18OwnerResourceRole::Xmap05GradientBorderChannel1,
            &political.gradient_border_channel1,
        ),
        (
            Xmap18OwnerResourceRole::Xmap05GradientBorderChannel2,
            &political.gradient_border_channel2,
        ),
        (
            Xmap18OwnerResourceRole::Xmap05IntelMap,
            &political.intel_map,
        ),
    ]
    .into_iter()
    .map(|(role, texture)| Xmap18OwnerResourceRef {
        role,
        descriptor_sha256: texture.descriptor.descriptor_sha256.clone(),
        payload_sha256: texture.descriptor.payload_sha256.clone(),
    })
    .collect()
}

fn hardcoded_composition(mode: HardcodedMapMode) -> MapModeComposition {
    match mode {
        HardcodedMapMode::Political => MapModeComposition::Political,
        HardcodedMapMode::StrategicAir => MapModeComposition::StrategicAir,
        HardcodedMapMode::StrategicNavy => MapModeComposition::StrategicNavy,
        HardcodedMapMode::Operatives => MapModeComposition::Intelligence,
        HardcodedMapMode::States => MapModeComposition::StateOwner,
        HardcodedMapMode::Supply => MapModeComposition::Supply,
        HardcodedMapMode::Resistance => MapModeComposition::Resistance,
        HardcodedMapMode::Compliance => MapModeComposition::Compliance,
        HardcodedMapMode::Resources => MapModeComposition::ResourceFurniture,
        HardcodedMapMode::Diplomacy => MapModeComposition::Diplomacy,
        HardcodedMapMode::Factions => MapModeComposition::Factions,
        HardcodedMapMode::Players => MapModeComposition::Players,
        HardcodedMapMode::Infrastructure => MapModeComposition::Infrastructure,
        HardcodedMapMode::Manpower => MapModeComposition::Manpower,
        HardcodedMapMode::Ideology => MapModeComposition::Ideology,
        HardcodedMapMode::Terrain => MapModeComposition::Terrain,
        HardcodedMapMode::Raids => MapModeComposition::RaidFurniture,
    }
}

fn hardcoded_engine_configuration(mode: HardcodedMapMode) -> EngineMapModeConfiguration {
    let (
        packed_layer_word,
        fill_channels,
        fill_update_kinds,
        manager_state_0x50,
        manager_state_0x54,
    ) = match mode {
        HardcodedMapMode::Political => (
            0x0000_0000_0000_0005,
            vec![(1, 4)],
            vec![0],
            Some(7),
            Some(0),
        ),
        HardcodedMapMode::StrategicAir => (
            0xffff_ffff_0000_0005,
            Vec::new(),
            vec![13],
            Some(3),
            Some(3),
        ),
        HardcodedMapMode::StrategicNavy => (
            0x0000_0000_0000_0005,
            vec![(1, 0)],
            vec![13],
            Some(3),
            Some(3),
        ),
        HardcodedMapMode::Operatives => {
            (0x0000_000a_0000_0000, vec![(0, 0)], Vec::new(), None, None)
        }
        HardcodedMapMode::States => (
            0x0000_0002_0000_0000,
            vec![(0, 0), (1, 0)],
            Vec::new(),
            None,
            None,
        ),
        HardcodedMapMode::Supply => (
            0x0000_0003_0000_0004,
            vec![(0, 0)],
            vec![0],
            Some(7),
            Some(0),
        ),
        HardcodedMapMode::Resistance => (
            0x0000_000c_0000_0000,
            vec![(0, 0), (1, 0)],
            vec![0, 12],
            Some(2),
            Some(0),
        ),
        HardcodedMapMode::Compliance => (
            0x0000_000c_0000_0000,
            vec![(0, 0), (1, 1)],
            vec![12],
            Some(2),
            Some(0),
        ),
        HardcodedMapMode::Resources => (
            0xffff_ffff_0000_0000,
            vec![(0, 0)],
            vec![5],
            Some(2),
            Some(0),
        ),
        HardcodedMapMode::Diplomacy => (0xffff_ffff_0000_0001, Vec::new(), Vec::new(), None, None),
        HardcodedMapMode::Factions => (
            0xffff_ffff_0000_0000,
            vec![(0, 1)],
            vec![0],
            Some(6),
            Some(6),
        ),
        HardcodedMapMode::Players => (
            0xffff_ffff_0000_0000,
            vec![(0, 2)],
            vec![0],
            Some(5),
            Some(5),
        ),
        HardcodedMapMode::Infrastructure => {
            (0x0000_0000_0000_0007, vec![(1, 0)], Vec::new(), None, None)
        }
        HardcodedMapMode::Manpower => (0x0000_0000_0000_0008, vec![(1, 0)], Vec::new(), None, None),
        HardcodedMapMode::Ideology => (0xffff_ffff_0000_0000, vec![(0, 3)], Vec::new(), None, None),
        HardcodedMapMode::Terrain => (0x0000_0000_0000_0009, vec![(1, 0)], Vec::new(), None, None),
        HardcodedMapMode::Raids => (
            0xffff_ffff_0000_0000,
            vec![(0, 0)],
            vec![0],
            Some(0),
            Some(0),
        ),
    };
    let diplomacy = mode == HardcodedMapMode::Diplomacy;
    EngineMapModeConfiguration {
        packed_layer_word: Some(packed_layer_word),
        fill_channels,
        fill_update_kinds,
        manager_state_0x50,
        manager_state_0x54,
        layer0_outline_cutoff_define: diplomacy
            .then(|| "GRADIENT_BORDERS_OUTLINE_CUTOFF_DIPLOMACY".into()),
        layer0_camera_distance_override_define: diplomacy
            .then(|| "GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_DIPLOMACY".into()),
        layer0_flag_0x80: diplomacy,
        manager_flag_0x49: diplomacy,
        scripted_definition_sha256: None,
        scripted_layers: Vec::new(),
        scripted_far_text: None,
        scripted_near_text: None,
        scripted_update_daily: None,
    }
}

fn validate_hardcoded_inputs(
    mode: HardcodedMapMode,
    context: &FoundationFeatureContext<'_>,
) -> Result<()> {
    let frame = context.world_frame;
    let present = |status: ViewStatus, label: &str| -> Result<()> {
        if status == ViewStatus::Present {
            Ok(())
        } else {
            Err(format!("XMAP18 {label} is {status:?}, not Present").into())
        }
    };
    match mode {
        HardcodedMapMode::Political
        | HardcodedMapMode::States
        | HardcodedMapMode::Terrain
        | HardcodedMapMode::Manpower => {
            present(frame.owner_by_state_status(), "owner-by-state authority")?
        }
        HardcodedMapMode::StrategicAir
        | HardcodedMapMode::StrategicNavy
        | HardcodedMapMode::Operatives => {
            require_observer(context, mode.token())?;
        }
        HardcodedMapMode::Supply => {
        }
        HardcodedMapMode::Resistance | HardcodedMapMode::Compliance => {
            present(frame.occupation_status(), "occupation map-mode authority")?
        }
        HardcodedMapMode::Factions => {
            present(frame.diplomacy().status, "faction authority")?;
            if !matches!(&frame.diplomacy().faction_members, Leaf::Measured(_)) {
                return Err("XMAP18 complete faction roster is not measured".into());
            }
        }
        HardcodedMapMode::Players => {
            // The player LEAF, not the combined `session_metadata` root.
            //
            // That root is `combine_status` over `session` and
            // `world.scenario`, and the exact D57 carrier this mode consumes
            // lives in NEITHER: the producer publishes `players_by_country`
            // from its own sidecar and can do so on an hour where the
            // pause/speed section is unavailable. Requiring the root refused to
            // draw a fully measured player set for a reason that has nothing to
            // do with it — and, once `session.player_country_set` became
            // owner-proven off the leaf, made the authority row admit an input
            // this consumer then rejected (lane PL, 2026-08-21).
            if !matches!(
                &frame.session_metadata().player_countries,
                Leaf::Measured(_)
            ) {
                return Err("XMAP18 player-country set is not measured".into());
            }
        }
        HardcodedMapMode::Infrastructure => {
            present(frame.buildings_status(), "infrastructure-state authority")?
        }
        HardcodedMapMode::Ideology => present(
            frame.country_identities_status(),
            "country ideology authority",
        )?,
        HardcodedMapMode::Resources | HardcodedMapMode::Raids | HardcodedMapMode::Diplomacy => {}
    }
    Ok(())
}

fn validate_scripted_inputs(context: &FoundationFeatureContext<'_>) -> Result<()> {
    require_observer(context, "scripted")?;
    Ok(())
}

fn require_observer(context: &FoundationFeatureContext<'_>, mode: &str) -> Result<()> {
    let observer = context
        .viewer
        .observer_country_id
        .as_deref()
        .ok_or_else(|| format!("XMAP18 {mode} map mode requires a viewer country"))?;
    // PARTIAL: this is one country's PRESENCE, not a census. A resolved entry
    // for the viewer's country is proof enough that the country exists and is
    // named, whatever happened to any other country this hour. The excluded
    // keys still ride the refusal, because "absent from D18" and "in the frame
    // but unnameable" are different facts and only the second is the recorder's.
    let roster = context.world_frame.d18_country_roster();
    let partial = roster
        .partial()
        .map_err(|refusal| format!("XMAP18 viewer country {observer}: {refusal}"))?;
    if !partial.resolved.iter().any(|entry| entry.tag == observer) {
        let unnamed = if partial.excluded.is_empty() {
            String::new()
        } else {
            format!(
                " ({} emitted country key(s) carry no D18 identity this hour: {})",
                partial.excluded.len(),
                partial.excluded.join(",")
            )
        };
        return Err(
            format!("XMAP18 viewer country {observer} is absent from D18{unnamed}").into(),
        );
    }
    Ok(())
}

fn semantic_witness(
    context: &FoundationFeatureContext<'_>,
    selection: &ViewerMapModeSelection,
    composition: MapModeComposition,
) -> Result<String> {
    let mut bytes = b"XMAP18_SEMANTIC_WITNESS_V1\0".to_vec();
    bytes.extend_from_slice(&context.world_frame.hour_index().to_le_bytes());
    bytes.extend_from_slice(selection.viewer_token().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(composition.token().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(
        context
            .viewer
            .observer_country_id
            .as_deref()
            .unwrap_or("<omniscient>")
            .as_bytes(),
    );
    bytes.push(0);
    for domain in [
        "map",
        "states",
        "countries",
        "naval_air",
        "diplomacy",
        "construction",
        "occupation",
        "raids",
        "session",
        "scripted_gui",
        "map_mode_runtime",
    ] {
        if let Some(value) = context.world_frame.domain(domain) {
            bytes.extend_from_slice(domain.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(&serde_json::to_vec(value)?);
            bytes.push(0);
        }
    }
    Ok(Sha256::hex(&bytes))
}

fn registration_digest() -> String {
    Sha256::hex(
        format!(
            "XMAP18\0{XMAP18_MAP_MODE_SCHEMA}\0{XMAP18_A729_EXECUTABLE_SHA256}\0{XMAP18_REGISTRY_GHIDRA_MANIFEST_SHA256}"
        )
        .as_bytes(),
    )
}

fn prepared_digest(prepared: &PreparedMapModeFrame) -> String {
    let mut bytes = b"XMAP18_PREPARED_V1\0".to_vec();
    bytes.extend_from_slice(&prepared.hour_index.to_le_bytes());
    bytes.extend_from_slice(prepared.viewer_selection.as_bytes());
    bytes.extend_from_slice(&prepared.engine_mode_code.to_le_bytes());
    bytes.push(prepared.visible_button.unwrap_or_default());
    bytes.extend_from_slice(prepared.composition.token().as_bytes());
    match prepared.engine_configuration.packed_layer_word {
        Some(word) => {
            bytes.push(1);
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        None => bytes.push(0),
    }
    for (channel, variant) in &prepared.engine_configuration.fill_channels {
        bytes.push(*channel);
        bytes.push(*variant);
    }
    bytes.push(
        u8::try_from(prepared.engine_configuration.fill_update_kinds.len())
            .expect("XMAP18 has fewer than 256 update producers"),
    );
    bytes.extend_from_slice(&prepared.engine_configuration.fill_update_kinds);
    bytes.extend_from_slice(
        &prepared
            .engine_configuration
            .manager_state_0x50
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    bytes.push(0);
    bytes.extend_from_slice(
        prepared
            .engine_configuration
            .scripted_definition_sha256
            .as_deref()
            .unwrap_or("<hardcoded>")
            .as_bytes(),
    );
    for define in [
        prepared
            .engine_configuration
            .layer0_outline_cutoff_define
            .as_deref(),
        prepared
            .engine_configuration
            .layer0_camera_distance_override_define
            .as_deref(),
    ] {
        bytes.push(0);
        bytes.extend_from_slice(define.unwrap_or("<none>").as_bytes());
    }
    bytes.push(u8::from(prepared.engine_configuration.layer0_flag_0x80));
    bytes.push(u8::from(prepared.engine_configuration.manager_flag_0x49));
    for layer in &prepared.engine_configuration.scripted_layers {
        bytes.push(0);
        bytes.extend_from_slice(layer.as_bytes());
    }
    for selector in [
        prepared.engine_configuration.scripted_far_text.as_deref(),
        prepared.engine_configuration.scripted_near_text.as_deref(),
    ] {
        bytes.push(0);
        bytes.extend_from_slice(selector.unwrap_or("<hardcoded>").as_bytes());
    }
    bytes.push(match prepared.engine_configuration.scripted_update_daily {
        Some(false) => 0,
        Some(true) => 1,
        None => 2,
    });
    bytes.extend_from_slice(
        &prepared
            .engine_configuration
            .manager_state_0x54
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    bytes.push(0);
    bytes.extend_from_slice(
        &serde_json::to_vec(&prepared.fill_source)
            .expect("serializing XMAP18 fill source cannot fail"),
    );
    bytes.push(0);
    bytes.extend_from_slice(prepared.render_political.deterministic_sha256.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(
        prepared
            .observer_country_id
            .as_deref()
            .unwrap_or("<omniscient>")
            .as_bytes(),
    );
    for id in &prepared.effective_scripted_mode_ids {
        bytes.push(0);
        bytes.extend_from_slice(id.as_bytes());
    }
    for content in &prepared.ordered_used_content {
        bytes.push(0);
        bytes.extend_from_slice(content.normalized_game_relative_path.as_bytes());
        bytes.extend_from_slice(content.source_sha256.as_bytes());
        bytes.extend_from_slice(content.payload_sha256.as_bytes());
        bytes.extend_from_slice(&content.payload_bytes.to_le_bytes());
    }
    bytes.extend_from_slice(prepared.semantic_witness_sha256.as_bytes());
    Sha256::hex(&bytes)
}

fn validate_digest(value: &str, label: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("XMAP18 {label} digest is not SHA-256 hex").into());
    }
    Ok(())
}

fn write_key_digest(rows: &[EngineProvinceColorWrite]) -> String {
    let mut bytes = b"XMAP18_CFILLBUFFER_WRITE_KEYS_A729_V1\0".to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.province_id.to_le_bytes());
    }
    Sha256::hex(&bytes)
}

fn write_row_digest(rows: &[EngineProvinceColorWrite]) -> String {
    let mut bytes = b"XMAP18_CFILLBUFFER_WRITES_A729_V1\0".to_vec();
    for row in rows {
        bytes.extend_from_slice(&row.province_id.to_le_bytes());
        bytes.extend_from_slice(&row.packed_bgra.to_le_bytes());
    }
    Sha256::hex(&bytes)
}

fn raw_runtime_digest(runtime: &Xmap18RuntimeInput) -> String {
    let mut bytes = b"XMAP18_RAW_MAP_MODE_PRIMITIVES_A729_V4\0".to_vec();
    bytes.extend_from_slice(runtime.schema.as_bytes());
    bytes.push(u8::from(runtime.complete));
    bytes.extend_from_slice(&runtime.hour_index.to_le_bytes());
    bytes.extend_from_slice(runtime.effective_vfs_manifest_sha256.as_bytes());
    bytes.extend_from_slice(&(runtime.observer_country_ids.len() as u64).to_le_bytes());
    for observer in &runtime.observer_country_ids {
        bytes.push(0);
        bytes.extend_from_slice(observer.as_bytes());
    }
    bytes.push(0xff);
    bytes.extend_from_slice(&(runtime.strategic_region_ids.len() as u64).to_le_bytes());
    for region_id in &runtime.strategic_region_ids {
        bytes.extend_from_slice(&region_id.to_le_bytes());
    }
    bytes.extend_from_slice(&(runtime.state_ids.len() as u64).to_le_bytes());
    for state_id in &runtime.state_ids {
        bytes.extend_from_slice(&state_id.to_le_bytes());
    }
    bytes.extend_from_slice(&(runtime.air_cells.len() as u64).to_le_bytes());
    for cell in &runtime.air_cells {
        bytes.extend_from_slice(&cell.observer_index.to_le_bytes());
        bytes.extend_from_slice(&cell.strategic_region_index.to_le_bytes());
        bytes.extend_from_slice(&cell.friendly_weight_raw.to_le_bytes());
        bytes.extend_from_slice(&cell.enemy_weight_raw.to_le_bytes());
    }
    bytes.extend_from_slice(&(runtime.naval_cells.len() as u64).to_le_bytes());
    for cell in &runtime.naval_cells {
        bytes.extend_from_slice(&cell.observer_index.to_le_bytes());
        bytes.extend_from_slice(&cell.strategic_region_index.to_le_bytes());
        bytes.push(cell.control_code_raw);
        bytes.push(cell.contested_code_raw);
    }
    bytes.extend_from_slice(&runtime.operative_strength_scale.to_le_bytes());
    bytes.extend_from_slice(&(runtime.operative_cells.len() as u64).to_le_bytes());
    for cell in &runtime.operative_cells {
        bytes.extend_from_slice(&cell.observer_index.to_le_bytes());
        bytes.extend_from_slice(&cell.state_index.to_le_bytes());
        bytes.extend_from_slice(&cell.strength_raw.to_le_bytes());
        bytes.push(cell.quiet_raw);
    }
    bytes.extend_from_slice(&runtime.projected_packed_bytes.to_le_bytes());
    Sha256::hex(&bytes)
}

fn scripted_runtime_digest(runtime: &Xmap18ScriptedRuntimeInput) -> String {
    let mut bytes = b"XMAP18_ENGINE_EVALUATED_SCRIPTED_SCOPES_A729_V2\0".to_vec();
    bytes.extend_from_slice(runtime.schema.as_bytes());
    bytes.push(u8::from(runtime.complete));
    bytes.extend_from_slice(&runtime.hour_index.to_le_bytes());
    bytes.extend_from_slice(runtime.effective_vfs_manifest_sha256.as_bytes());
    bytes.extend_from_slice(&(runtime.observer_country_ids.len() as u64).to_le_bytes());
    for observer in &runtime.observer_country_ids {
        bytes.push(0);
        bytes.extend_from_slice(observer.as_bytes());
    }
    bytes.extend_from_slice(&(runtime.state_ids.len() as u64).to_le_bytes());
    for state_id in &runtime.state_ids {
        bytes.extend_from_slice(&state_id.to_le_bytes());
    }
    bytes.push(0xfe);
    bytes.extend_from_slice(&(runtime.scripted_mode_ids.len() as u64).to_le_bytes());
    for mode in &runtime.scripted_mode_ids {
        bytes.push(0);
        bytes.extend_from_slice(mode.as_bytes());
    }
    bytes.push(0xff);
    bytes.extend_from_slice(&(runtime.layer_payloads.len() as u64).to_le_bytes());
    for payload in &runtime.layer_payloads {
        bytes.push(match payload.scope_kind {
            Xmap18ScriptedScopeKind::None => 0,
            Xmap18ScriptedScopeKind::Country => 1,
            Xmap18ScriptedScopeKind::State => 2,
            Xmap18ScriptedScopeKind::StateController => 3,
            Xmap18ScriptedScopeKind::HardcodedPolitical => 4,
            Xmap18ScriptedScopeKind::HardcodedStates => 5,
            Xmap18ScriptedScopeKind::HardcodedDiplomacy => 6,
            Xmap18ScriptedScopeKind::HardcodedPlayers => 7,
            Xmap18ScriptedScopeKind::HardcodedFactions => 8,
            Xmap18ScriptedScopeKind::HardcodedIdeology => 9,
        });
        bytes.extend_from_slice(&(payload.rows.len() as u64).to_le_bytes());
        for row in &payload.rows {
            bytes.extend_from_slice(&row.scope_index.to_le_bytes());
            bytes.extend_from_slice(&row.controller_index.unwrap_or(u32::MAX).to_le_bytes());
            bytes.extend_from_slice(&row.packed_bgra.to_le_bytes());
            bytes.extend_from_slice(
                &row.gradient_center_f32_bits
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            bytes.extend_from_slice(
                &row.gradient_thickness_f32_bits
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
        }
    }
    bytes.extend_from_slice(&(runtime.entries.len() as u64).to_le_bytes());
    for entry in &runtime.entries {
        bytes.extend_from_slice(&entry.observer_index.to_le_bytes());
        bytes.extend_from_slice(&entry.scripted_mode_index.to_le_bytes());
        bytes.extend_from_slice(&entry.top_payload_index.to_le_bytes());
        bytes.extend_from_slice(&entry.bottom_payload_index.to_le_bytes());
    }
    bytes.extend_from_slice(&runtime.projected_packed_bytes.to_le_bytes());
    Sha256::hex(&bytes)
}

#[cfg(test)]
fn scripted_definition_ids(bytes: &[u8], path: &str) -> Result<Vec<String>> {
    let (source, losses) = decode_text_diagnosed(bytes);
    let destructive = destructive_losses(&losses);
    if !destructive.is_empty() {
        return Err(format!(
            "XMAP18 scripted map-mode source {path} lost {} byte sequence(s) while decoding",
            destructive_event_count(&losses)
        )
        .into());
    }
    let document = parse_str(&source);
    if !document.recoveries.is_empty() {
        let first = &document.recoveries[0];
        return Err(format!(
            "XMAP18 scripted map-mode source {path} required {} parser recovery event(s); first at {}:{}: {}",
            document.recoveries.len(),
            first.line,
            first.column,
            first.message
        )
        .into());
    }
    let mut ids = Vec::new();
    for database in document.root.values_ci("scripted_map_modes") {
        let Value::Object(definitions) = database else {
            return Err(format!("XMAP18 scripted_map_modes in {path} is not an object").into());
        };
        for definition in definitions {
            let id = definition
                .key
                .as_deref()
                .ok_or_else(|| {
                    format!(
                        "XMAP18 scripted_map_modes in {path} has an unkeyed entry at {}:{}",
                        definition.line, definition.column
                    )
                })?
                .trim();
            if id.is_empty() || !matches!(definition.value, Value::Object(_)) {
                return Err(format!(
                    "XMAP18 scripted map-mode definition in {path} at {}:{} has no object body",
                    definition.line, definition.column
                )
                .into());
            }
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_with_states(states: serde_json::Value) -> std::sync::Arc<replay_core::WorldFrame> {
        use replay_core::{RawFrame, ReplaySource, SourceError, WorldTimeline};
        struct Source(serde_json::Value);
        impl ReplaySource for Source {
            fn game_id(&self) -> &str {
                "xmap18-infrastructure-join"
            }
            fn hours(&self) -> &[i64] {
                &[7]
            }
            fn warnings(&self) -> &[String] {
                &[]
            }
            fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
                let mut rest = serde_json::Map::new();
                rest.insert("states".into(), self.0.clone());
                Ok(RawFrame {
                    hour_index,
                    date: None,
                    save_version: None,
                    schema_version: None,
                    rest,
                })
            }
        }
        let timeline = WorldTimeline::open(Box::new(Source(states))).expect("timeline");
        timeline.frame(7).expect("frame")
    }

    fn d48_state(level: Option<u32>) -> serde_json::Value {
        let rows = match level {
            Some(level) => serde_json::json!({
                "type_0x00002fab": {
                    "level": level,
                    "healthy_level": level,
                    "damaged": false,
                    "token": "infrastructure"
                }
            }),
            None => serde_json::json!({}),
        };
        serde_json::json!({
            "construction_buildings_state_damage_live": {
                "source": "C",
                "state_buildings": rows
            }
        })
    }

    /// COUNTED: every position in a four-state VFS list is broken in turn, so a
    /// loop that read a prefix, a suffix or any fixed count fails here.
    #[test]
    fn the_infrastructure_join_counts_every_state_the_vfs_defines() {
        let vfs: Vec<u32> = vec![11, 12, 13, 14];

        let whole = serde_json::json!({
            "11": d48_state(Some(3)),
            "12": d48_state(None),
            "13": d48_state(Some(1)),
            "14": d48_state(Some(5)),
        });
        let frame = frame_with_states(whole.clone());
        let inputs =
            infrastructure_level_inputs(&frame, vfs.iter().copied()).expect("whole scopes join");
        assert_eq!(inputs.len(), 4);
        assert_eq!(inputs[&12].level, Some(0.0), "a measured-empty scope is zero");
        assert_eq!(inputs[&14].level, Some(5.0));

        for broken in &vfs {
            for shape in [
                serde_json::json!({}),
                serde_json::json!({"construction_buildings_state_damage_live": {"source": "C"}}),
                serde_json::json!({
                    "construction_buildings_state_damage_live": {
                        "source": "recorder", "state_buildings": {}
                    }
                }),
            ] {
                let mut states = whole.clone();
                states[broken.to_string()] = shape;
                let frame = frame_with_states(states);
                let error = infrastructure_level_inputs(&frame, vfs.iter().copied())
                    .expect_err("a state without a whole D48 scope must refuse")
                    .to_string();
                assert!(
                    error.contains("joined 3 of the 4 states") && error.contains(&broken.to_string()),
                    "the refusal must count and name state {broken}: {error}"
                );
            }
            // A state the VFS defines and the frame does not carry at all.
            let mut states = whole.clone();
            states.as_object_mut().expect("object").remove(&broken.to_string());
            let frame = frame_with_states(states);
            assert!(infrastructure_level_inputs(&frame, vfs.iter().copied()).is_err());
        }
    }

    #[test]
    fn effective_scripted_database_preserves_file_order() {
        let source = br#"
            ignored = { }
            scripted_map_modes = {
                first_mode = {
                    color = { type = country }
                }
                second_mode = {
                    color = { type = state }
                }
            }
        "#;
        assert_eq!(
            scripted_definition_ids(source, "common/map_modes/test.txt").unwrap(),
            ["first_mode", "second_mode"]
        );
    }

    #[test]
    fn malformed_scripted_database_fails_closed() {
        assert!(scripted_definition_ids(
            b"scripted_map_modes = { bad = { color = { }",
            "common/map_modes/bad.txt"
        )
        .is_err());
    }

    fn scripted_definition(source: &str, id: &str) -> Value {
        let document = parse_str(source);
        assert!(document.recoveries.is_empty());
        let definition = document
            .root
            .values_ci("scripted_map_modes")
            .flat_map(Value::fields)
            .find(|entry| entry.key.as_deref() == Some(id))
            .expect("fixture scripted definition")
            .value
            .clone();
        definition
    }

    #[test]
    fn scripted_constant_layers_are_file_bound_and_ordered() {
        let definition = scripted_definition(
            r#"scripted_map_modes = {
                exact_mode = {
                    top = {
                        type = country
                        color = {
                            set_temp_variable = { red = 0 green = 0 blue = 1 alpha = 1 }
                        }
                    }
                    bottom = {
                        type = state
                        thickness = no
                        color = {
                            always = yes
                            set_temp_variable = { red = 1 green = 0 blue = 0 alpha = 0.5 }
                        }
                    }
                }
            }"#,
            "exact_mode",
        );
        let specs = scripted_layer_specs("exact_mode", &definition).unwrap();
        assert!(matches!(specs[0].kind, ScriptedLayerKind::Country));
        assert_eq!(specs[0].rgba, Some([0, 0, 255, 255]));
        assert!(matches!(specs[1].kind, ScriptedLayerKind::State));
        assert_eq!(specs[1].rgba, Some([255, 0, 0, 128]));
    }

    #[test]
    fn scripted_mixed_hardcoded_layer_is_ordered_and_unknown_color_field_fails_closed() {
        let mixed = scripted_definition(
            r#"scripted_map_modes = {
                mixed = {
                    top = { type = game_map_mode_country }
                    bottom = { type = state color = { set_temp_variable = { red = 1 green = 0 blue = 0 alpha = 1 } } }
                }
            }"#,
            "mixed",
        );
        let mixed = scripted_layer_specs("mixed", &mixed).unwrap();
        assert!(matches!(
            mixed[0].kind,
            ScriptedLayerKind::Hardcoded(HardcodedMapMode::Political)
        ));
        assert!(matches!(mixed[1].kind, ScriptedLayerKind::State));

        let unknown = scripted_definition(
            r#"scripted_map_modes = {
                unknown = {
                    top = { type = country color = { set_temp_variable = { red = 1 green = 0 blue = 0 alpha = 1 hidden = 7 } } }
                }
            }"#,
            "unknown",
        );
        assert!(scripted_layer_specs("unknown", &unknown).is_err());
    }

    #[test]
    fn scripted_effective_database_is_later_definition_wins() {
        let first = scripted_definition(
            "scripted_map_modes = { exact = { top = { type = none } } }",
            "exact",
        );
        let later = scripted_definition(
            "scripted_map_modes = { exact = { bottom = { type = none } } }",
            "exact",
        );
        let mut definitions = Vec::new();
        insert_effective_scripted_definition(&mut definitions, "exact".into(), first);
        insert_effective_scripted_definition(&mut definitions, "exact".into(), later.clone());
        assert_eq!(definitions, vec![("exact".into(), later)]);
    }

    #[test]
    fn scripted_definition_identity_covers_every_accepted_ast_field() {
        let without_thickness = scripted_definition(
            "scripted_map_modes = { exact = { top = { type = country color = { set_temp_variable = { red = 1 green = 0 blue = 0 alpha = 1 } } } } }",
            "exact",
        );
        let with_thickness = scripted_definition(
            "scripted_map_modes = { exact = { top = { type = country thickness = no color = { set_temp_variable = { red = 1 green = 0 blue = 0 alpha = 1 } } } } }",
            "exact",
        );
        scripted_layer_specs("exact", &without_thickness).unwrap();
        scripted_layer_specs("exact", &with_thickness).unwrap();
        let digest = |value: &Value| {
            let mut bytes = Vec::new();
            canonical_value(value, &mut bytes);
            Sha256::hex(&bytes)
        };
        assert_ne!(digest(&without_thickness), digest(&with_thickness));
    }

    #[test]
    fn scripted_dynamic_targets_use_engine_runtime_instead_of_disappearing() {
        let definition = scripted_definition(
            r#"scripted_map_modes = {
                dynamic_mode = {
                    top = {
                        type = country
                        targets = { GER }
                        color = { set_temp_variable = { red = 1 green = 1 blue = 1 alpha = 1 } }
                    }
                }
            }"#,
            "dynamic_mode",
        );
        assert!(parse_scripted_layer(
            "dynamic_mode",
            "top",
            crate::production_features::xmap05::Xmap05MapModeBand::Top,
            definition.first_value_ci("top"),
        )
        .is_err());
        assert_eq!(
            scripted_layer_identities("dynamic_mode", &definition),
            ["top:engine_evaluated", "bottom:engine_evaluated"]
        );
        let mut canonical = Vec::new();
        canonical_value(&definition, &mut canonical);
        assert!(
            !canonical.is_empty(),
            "the complete dynamic definition is authenticated"
        );
    }

    #[test]
    fn scripted_mode_level_fields_apply_exact_defaults_and_values() {
        let explicit = scripted_definition(
            r#"scripted_map_modes = {
                exact = {
                    top = { type = none }
                    far_text = faction
                    near_text = state
                    update_daily = no
                }
            }"#,
            "exact",
        );
        assert_eq!(
            scripted_mode_level_configuration("exact", &explicit).unwrap(),
            ("faction".into(), "state".into(), false)
        );
        let defaults = scripted_definition(
            "scripted_map_modes = { exact = { top = { type = none } } }",
            "exact",
        );
        assert_eq!(
            scripted_mode_level_configuration("exact", &defaults).unwrap(),
            ("none".into(), "none".into(), true)
        );
        let overrides = scripted_definition(
            "scripted_map_modes = { exact = { far_text = country far_text = player update_daily = no update_daily = yes } }",
            "exact",
        );
        assert_eq!(
            scripted_mode_level_configuration("exact", &overrides).unwrap(),
            ("player".into(), "none".into(), true)
        );
    }

    fn scripted_runtime_fixture() -> Xmap18ScriptedRuntimeInput {
        let mut runtime = Xmap18ScriptedRuntimeInput {
            schema: XMAP18_SCRIPTED_RUNTIME_INPUT_SCHEMA.into(),
            complete: true,
            hour_index: 42,
            effective_vfs_manifest_sha256: "a".repeat(64),
            observer_country_ids: vec!["ENG".into()],
            state_ids: vec![10],
            scripted_mode_ids: vec!["dynamic_mode".into()],
            layer_payloads: vec![
                Xmap18ScriptedLayerPayload {
                    scope_kind: Xmap18ScriptedScopeKind::Country,
                    rows: vec![Xmap18ScriptedScopeResult {
                        scope_index: 0,
                        controller_index: None,
                        packed_bgra: 0x4433_2211,
                        gradient_center_f32_bits: Some(2.0f32.to_bits()),
                        gradient_thickness_f32_bits: Some(10.0f32.to_bits()),
                    }],
                },
                Xmap18ScriptedLayerPayload {
                    scope_kind: Xmap18ScriptedScopeKind::None,
                    rows: Vec::new(),
                },
            ],
            entries: vec![Xmap18ScriptedScopeEntry {
                observer_index: 0,
                scripted_mode_index: 0,
                top_payload_index: 0,
                bottom_payload_index: 1,
            }],
            projected_packed_bytes: 0,
            deterministic_sha256: String::new(),
        };
        runtime.projected_packed_bytes = projected_scripted_runtime_bytes(&runtime).unwrap();
        runtime.deterministic_sha256 = scripted_runtime_digest(&runtime);
        runtime
    }

    #[test]
    fn scripted_runtime_is_compact_canonical_and_scope_exact() {
        let runtime = scripted_runtime_fixture();
        assert_eq!(runtime.projected_packed_bytes, 59);
        assert_eq!(
            runtime.deterministic_sha256,
            scripted_runtime_digest(&runtime)
        );
        let value = serde_json::to_value(&runtime).unwrap();
        assert_eq!(
            decode_bounded_scripted_runtime_input(&value).unwrap(),
            runtime
        );
        validate_scripted_scope_payload(&runtime.layer_payloads[0], &runtime).unwrap();
        validate_scripted_scope_payload(&runtime.layer_payloads[1], &runtime).unwrap();
    }

    #[test]
    fn scripted_runtime_rejects_partial_gradient_or_wrong_scope_tuple() {
        let mut runtime = scripted_runtime_fixture();
        runtime.layer_payloads[0].rows[0].gradient_thickness_f32_bits = None;
        assert!(validate_scripted_scope_payload(&runtime.layer_payloads[0], &runtime).is_err());
        runtime = scripted_runtime_fixture();
        runtime.layer_payloads[0].rows[0].controller_index = Some(0);
        assert!(validate_scripted_scope_payload(&runtime.layer_payloads[0], &runtime).is_err());
    }

    #[test]
    fn scripted_runtime_rejects_wrong_kind_in_unselected_mode() {
        let mut runtime = scripted_runtime_fixture();
        runtime.scripted_mode_ids.push("unselected_mode".into());
        runtime.layer_payloads.push(Xmap18ScriptedLayerPayload {
            scope_kind: Xmap18ScriptedScopeKind::State,
            rows: vec![Xmap18ScriptedScopeResult {
                scope_index: 0,
                controller_index: None,
                packed_bgra: 0x4433_2211,
                gradient_center_f32_bits: None,
                gradient_thickness_f32_bits: None,
            }],
        });
        runtime.entries.push(Xmap18ScriptedScopeEntry {
            observer_index: 0,
            scripted_mode_index: 1,
            top_payload_index: 2,
            bottom_payload_index: 1,
        });
        let definitions = vec![
            (
                "dynamic_mode".into(),
                scripted_definition(
                    "scripted_map_modes = { dynamic_mode = { top = { type = country } bottom = { type = none } } }",
                    "dynamic_mode",
                ),
            ),
            (
                "unselected_mode".into(),
                scripted_definition(
                    "scripted_map_modes = { unselected_mode = { top = { type = country } bottom = { type = none } } }",
                    "unselected_mode",
                ),
            ),
        ];
        assert!(
            validate_scripted_runtime_scope_kinds(&runtime, &definitions)
                .unwrap_err()
                .to_string()
                .contains("unselected_mode")
        );
    }

    #[test]
    fn scripted_runtime_serialized_production_ceiling_is_predecode() {
        let oversized = serde_json::json!({
            "padding": "x".repeat(XMAP18_MAX_SCRIPTED_RUNTIME_JSON_BYTES_PER_HOUR)
        });
        assert!(decode_bounded_scripted_runtime_input(&oversized)
            .unwrap_err()
            .to_string()
            .contains("production ceiling"));
    }

    fn raw_runtime_fixture() -> Xmap18RuntimeInput {
        let mut runtime = Xmap18RuntimeInput {
            schema: XMAP18_RUNTIME_INPUT_SCHEMA.into(),
            complete: true,
            hour_index: 42,
            effective_vfs_manifest_sha256: "a".repeat(64),
            observer_country_ids: vec!["ENG".into(), "GER".into()],
            strategic_region_ids: vec![1, 2],
            state_ids: vec![10, 20],
            air_cells: vec![
                Xmap18AirCell {
                    observer_index: 0,
                    strategic_region_index: 0,
                    friendly_weight_raw: 1,
                    enemy_weight_raw: 4,
                },
                Xmap18AirCell {
                    observer_index: 0,
                    strategic_region_index: 1,
                    friendly_weight_raw: 2,
                    enemy_weight_raw: 3,
                },
                Xmap18AirCell {
                    observer_index: 1,
                    strategic_region_index: 0,
                    friendly_weight_raw: 3,
                    enemy_weight_raw: 2,
                },
                Xmap18AirCell {
                    observer_index: 1,
                    strategic_region_index: 1,
                    friendly_weight_raw: 4,
                    enemy_weight_raw: 1,
                },
            ],
            naval_cells: vec![
                Xmap18NavalCell {
                    observer_index: 0,
                    strategic_region_index: 0,
                    control_code_raw: 255,
                    contested_code_raw: 0,
                },
                Xmap18NavalCell {
                    observer_index: 0,
                    strategic_region_index: 1,
                    control_code_raw: 128,
                    contested_code_raw: 128,
                },
                Xmap18NavalCell {
                    observer_index: 1,
                    strategic_region_index: 0,
                    control_code_raw: 0,
                    contested_code_raw: 255,
                },
                Xmap18NavalCell {
                    observer_index: 1,
                    strategic_region_index: 1,
                    control_code_raw: 255,
                    contested_code_raw: 0,
                },
            ],
            operative_strength_scale: 100_000,
            operative_cells: vec![
                Xmap18OperativeCell {
                    observer_index: 0,
                    state_index: 1,
                    strength_raw: 50_000,
                    quiet_raw: 0,
                },
                Xmap18OperativeCell {
                    observer_index: 1,
                    state_index: 0,
                    strength_raw: 100_000,
                    quiet_raw: 1,
                },
                Xmap18OperativeCell {
                    observer_index: 1,
                    state_index: 1,
                    strength_raw: 0,
                    quiet_raw: 0,
                },
            ],
            projected_packed_bytes: 0,
            deterministic_sha256: String::new(),
        };
        runtime.projected_packed_bytes = projected_runtime_bytes(&runtime).unwrap();
        runtime.deterministic_sha256 = raw_runtime_digest(&runtime);
        runtime
    }

    #[test]
    fn raw_matrix_is_compact_colour_free_and_identity_bound() {
        let runtime = raw_runtime_fixture();
        let json = serde_json::to_string(&runtime).unwrap();
        assert_eq!(
            json.matches("ENG").count(),
            1,
            "observer ids are not repeated per cell"
        );
        assert!(!json.contains("packed_bgra"));
        assert_eq!(
            runtime.projected_packed_bytes,
            8 + 8 + 8 + 8 + 4 * 24 + 4 * 10 + 3 * 17
        );
        assert!(
            serde_json::to_vec(&runtime).unwrap().len() <= XMAP18_MAX_RUNTIME_JSON_BYTES_PER_HOUR
        );
        validate_sparse_runtime(&runtime).unwrap();
        assert_eq!(runtime.deterministic_sha256, raw_runtime_digest(&runtime));
        let value = serde_json::to_value(&runtime).unwrap();
        assert_eq!(decode_bounded_runtime_input(&value).unwrap(), runtime);
    }

    #[test]
    fn raw_runtime_observer_switch_selects_different_sparse_rows_and_exact_defaults() {
        let runtime = raw_runtime_fixture();
        assert_eq!(air_weights_for(&runtime, 0, 0), (1, 4));
        assert_eq!(air_weights_for(&runtime, 1, 0), (3, 2));
        assert_eq!(naval_codes_for(&runtime, 0, 1), (128, 128));
        assert_eq!(naval_codes_for(&runtime, 1, 1), (255, 0));
        assert_eq!(air_weights_for(&runtime, 9, 9), (0, 0));
        assert_eq!(naval_codes_for(&runtime, 9, 9), (0, 0));
    }

    #[test]
    fn raw_runtime_serialized_production_ceiling_is_enforced_before_schema_decode() {
        let oversized = serde_json::json!({
            "padding": "x".repeat(XMAP18_MAX_RUNTIME_JSON_BYTES_PER_HOUR)
        });
        assert!(decode_bounded_runtime_input(&oversized)
            .unwrap_err()
            .to_string()
            .contains("production ceiling"));
    }

    #[test]
    fn diplomacy_is_configuration_only_and_uses_exact_layer_zero_defines() {
        let configuration = hardcoded_engine_configuration(HardcodedMapMode::Diplomacy);
        assert_eq!(configuration.packed_layer_word, Some(0xffff_ffff_0000_0001));
        assert!(configuration.fill_channels.is_empty());
        assert!(configuration.fill_update_kinds.is_empty());
        assert_eq!(
            configuration.layer0_outline_cutoff_define.as_deref(),
            Some("GRADIENT_BORDERS_OUTLINE_CUTOFF_DIPLOMACY")
        );
        assert_eq!(
            configuration
                .layer0_camera_distance_override_define
                .as_deref(),
            Some("GRADIENT_BORDERS_CAMERA_DISTANCE_OVERRIDE_DIPLOMACY")
        );
        assert!(configuration.layer0_flag_0x80);
        assert!(configuration.manager_flag_0x49);
    }

    #[test]
    fn raw_matrix_identity_rejects_partial_or_stale_content() {
        let runtime = raw_runtime_fixture();
        let mut partial = runtime.clone();
        partial.air_cells[0].enemy_weight_raw = -1;
        assert!(validate_sparse_runtime(&partial).is_err());
        let mut stale = runtime.clone();
        stale.hour_index += 1;
        assert_ne!(stale.deterministic_sha256, raw_runtime_digest(&stale));
        let mut invalid_naval = runtime;
        invalid_naval.naval_cells[0].control_code_raw = 127;
        assert!(validate_sparse_runtime(&invalid_naval).is_err());
    }

    /// kills: restoring the gap-only owned-input loop that aborted `prepare()`
    /// whenever an owned authority row was ADMITTED instead of gapped (the
    /// pre-PL production abort: two owned rows are owner-proven and occupation
    /// admits on live hours, so the whole map-mode family died).
    ///
    /// This drives `prepare()`'s exact accounting call with XMAP18's exact
    /// owned list: admitted owned rows must be accepted silently, gapped owned
    /// rows must come back as consumed gap ids, and nothing may error.
    #[test]
    fn owned_input_accounting_accepts_admitted_rows_and_consumes_gaps() {
        use crate::map_authority::{MapAuthorityGap, MapAuthorityAdmittedInput};
        use crate::map_authority::MapAuthorityRequirementDisposition;

        let admitted_ids = ["mapmode.raids", "session.player_country_set"];
        let admitted: Vec<MapAuthorityAdmittedInput> = admitted_ids
            .iter()
            .map(|id| MapAuthorityAdmittedInput {
                input_id: (*id).to_string(),
                owner_packet: "XMAP18".to_string(),
                payload_schema: "test".to_string(),
                cadence: "hourly".to_string(),
                identity_order: "test".to_string(),
                source: crate::MapAuthorityAdmissionSource::RecordedCarrier,
                derivation: None,
            })
            .collect();
        let gaps: Vec<MapAuthorityGap> = OWNED_AUTHORITY_INPUTS
            .iter()
            .filter(|id| !admitted_ids.contains(id))
            .map(|id| MapAuthorityGap {
                input_id: (*id).to_string(),
                disposition: MapAuthorityRequirementDisposition::Unavailable,
                reason: "test gap".to_string(),
            })
            .collect();

        let consumed = crate::account_owned_map_authority_inputs(
            "XMAP18",
            &OWNED_AUTHORITY_INPUTS,
            &gaps,
            &admitted,
        )
        .expect("admitted owned rows must not abort the family");
        let expected: Vec<String> = OWNED_AUTHORITY_INPUTS
            .iter()
            .filter(|id| !admitted_ids.contains(id))
            .map(|id| (*id).to_string())
            .collect();
        assert_eq!(consumed, expected);

        // The two defended failure shapes stay errors: an owned row that is
        // both admitted and gapped, and an owned row missing from both sides.
        let mut both = gaps.clone();
        both.push(MapAuthorityGap {
            input_id: "mapmode.raids".to_string(),
            disposition: MapAuthorityRequirementDisposition::Unavailable,
            reason: "test gap".to_string(),
        });
        assert!(crate::account_owned_map_authority_inputs(
            "XMAP18",
            &OWNED_AUTHORITY_INPUTS,
            &both,
            &admitted,
        )
        .is_err());
        assert!(crate::account_owned_map_authority_inputs(
            "XMAP18",
            &OWNED_AUTHORITY_INPUTS,
            &gaps,
            &admitted[..1],
        )
        .is_err());
    }

    /// kills: reverting `Xmap18MapModeAdapter::prepare` to the old gap-only
    /// owned-input loop. That loop aborted the whole map-mode family whenever
    /// an owned authority row was ADMITTED instead of gapped (the pre-PL
    /// production abort), and it could not detect a row that was both admitted
    /// and gapped. Both arms here drive `prepare()` itself, so a revert of the
    /// wiring — not just of the shared helper — flips them.
    #[test]
    fn prepare_owned_accounting_accepts_admitted_rows_and_rejects_double_booking() {
        use crate::map_authority::{
            MapAuthorityAdmittedInput, MapAuthorityGap, MapAuthorityRequirementDisposition,
        };
        use crate::production_features::xmap15::Xmap15ViewerSelection;
        use crate::production_foundation::{FoundationFeatureContext, ViewerOwnedMapState};
        use replay_core::{RawFrame, ReplaySource, SourceError, WorldTimeline};

        struct EmptySource;
        impl ReplaySource for EmptySource {
            fn game_id(&self) -> &str {
                "xmap18-owned-accounting-seam"
            }
            fn hours(&self) -> &[i64] {
                &[7]
            }
            fn warnings(&self) -> &[String] {
                &[]
            }
            fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
                Ok(RawFrame {
                    hour_index,
                    date: None,
                    save_version: None,
                    schema_version: None,
                    rest: serde_json::Map::new(),
                })
            }
        }

        let timeline = WorldTimeline::open(Box::new(EmptySource)).expect("timeline");
        let frame = timeline.frame(7).expect("frame");
        let viewer = ViewerOwnedMapState {
            viewport_width: 16,
            viewport_height: 16,
            camera: None,
            observer_country_id: None,
            // Deliberately NOT a valid selection: both arms must resolve
            // before any real map-mode work is reachable.
            map_mode_id: "not_a_mode".to_string(),
            map_mode_selection_epoch: 0,
            selected_object_ids: Vec::new(),
            input_revision: 0,
            animation_time_f32_bits: 0,
            frame_delta_seconds_f32_bits: 0,
        };
        let selection = Xmap15ViewerSelection {
            selected_plan: None,
        };
        let adapter = Xmap18MapModeAdapter::new_for_owned_accounting_tests();

        let admitted = vec![MapAuthorityAdmittedInput {
            input_id: "mapmode.raids".to_string(),
            owner_packet: "XMAP18".to_string(),
            payload_schema: "test".to_string(),
            cadence: "hourly".to_string(),
            identity_order: "test".to_string(),
            source: crate::MapAuthorityAdmissionSource::RecordedCarrier,
            derivation: None,
        }];
        let gaps: Vec<MapAuthorityGap> = OWNED_AUTHORITY_INPUTS
            .iter()
            .filter(|id| **id != "mapmode.raids")
            .map(|id| MapAuthorityGap {
                input_id: (*id).to_string(),
                disposition: MapAuthorityRequirementDisposition::Unavailable,
                reason: "test gap".to_string(),
            })
            .collect();

        // Arm 1: an admitted owned row must pass accounting; the first error
        // `prepare()` may report is the (deliberately bogus) selection.
        let context = FoundationFeatureContext::new_for_owned_accounting_tests(
            &frame, &viewer, &gaps, &admitted, &selection,
        );
        let error = match adapter.prepare(&context) {
            Ok(_) => panic!("the bogus selection must fail"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("unknown viewer map-mode token"),
            "prepare() failed BEFORE selection parsing — the owned-input \
             accounting rejected an admitted row again: {error}"
        );

        // Arm 2: a row that is both admitted and gapped must abort in
        // accounting, before selection parsing is ever reached.
        let mut double_booked = gaps.clone();
        double_booked.push(MapAuthorityGap {
            input_id: "mapmode.raids".to_string(),
            disposition: MapAuthorityRequirementDisposition::Unavailable,
            reason: "test gap".to_string(),
        });
        let context = FoundationFeatureContext::new_for_owned_accounting_tests(
            &frame, &viewer, &double_booked, &admitted, &selection,
        );
        let error = match adapter.prepare(&context) {
            Ok(_) => panic!("double booking must abort"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("both admitted and gapped"),
            "expected the accounting abort, got: {error}"
        );
    }
}
