use crate::sha256::Sha256;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const A27_LIVE_CATALOG_SCHEMA: &str = "extractor.a27.native_catalog.a729.v5";
pub const A27_LIVE_CATALOG_TIMELINE_SCHEMA: &str =
    "extractor.a27.live_heap_catalog_timeline.a729.v3";
pub const PORTABLE_DESIGN_CATALOG_SCHEMA: &str = "hoi4.portable-design-catalog.v5";
pub const PORTABLE_ARTIFACT_PATH: &str = "static/design_catalog_live/a27_live_catalog.json";
const DATE_EPOCH_RAW: u64 = 43_799_999;
const EXACT_A729_PROGRAM: &str = "hoi4.exe 1.19.2.0.a729";
const EXACT_A729_SHA256: &str = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DesignKind {
    DivisionTemplate,
    ShipVariant,
    TankVariant,
    AircraftVariant,
    EquipmentVariant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldCompleteness {
    Complete,
    NotApplicable,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesignLifecycleState {
    Active,
    Obsolete,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesignTextObservation {
    Measured { value: String, utf8_hex: String },
    Unavailable { reason: DesignFieldRefusal },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesignItemIconObservation {
    Measured { active: bool, kind: u32, token: DesignTextObservation },
    Unavailable { reason: DesignFieldRefusal },
}

pub const MAX_DESIGN_SLOT_IDENTITIES: usize = 256;
pub const MAX_DESIGN_SLOT_TOKEN_BYTES: usize = 160;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesignI32Observation {
    Measured { raw_i32: i32 },
    Unavailable { reason: DesignFieldRefusal },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignSlotIdentity {
    pub source_ordinal: u32,
    pub token_handle_raw: u32,
    pub token: DesignTextObservation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesignSlotIdentityObservation {
    Measured {
        archetype_token_raw: u32,
        archetype_token: DesignTextObservation,
        slots: Vec<DesignSlotIdentity>,
    },
    Unavailable { reason: DesignFieldRefusal },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesignByteObservation {
    Measured { raw_u8: u8 },
    Unavailable { reason: DesignFieldRefusal },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesignU32Observation {
    Measured { raw_u32: u32 },
    Unavailable { reason: DesignFieldRefusal },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesignI64Observation {
    Measured { raw_i64: i64 },
    Unavailable { reason: DesignFieldRefusal },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesignMissionOverrideObservation {
    Measured {
        raw_i64: i64,
        group_count: u32,
        group_key_prefix: Vec<u32>,
        stat_count: u32,
        stat_key_prefix: Vec<u32>,
    },
    GroupAbsent { group_count: u32, group_key_prefix: Vec<u32> },
    StatAbsent {
        group_count: u32,
        group_key_prefix: Vec<u32>,
        stat_count: u32,
        stat_key_prefix: Vec<u32>,
    },
    Unavailable { reason: DesignFieldRefusal },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignMissionStatOperand {
    pub group_key: u32,
    pub stat_key: u32,
    pub fallback: DesignI64Observation,
    pub override_lookup: DesignMissionOverrideObservation,
}

impl DesignMissionStatOperand {
    pub fn effective_raw_i64(&self) -> Result<i64, DesignFieldRefusal> {
        match &self.override_lookup {
            DesignMissionOverrideObservation::Measured { raw_i64, .. } => Ok(*raw_i64),
            DesignMissionOverrideObservation::Unavailable { reason } => Err(reason.clone()),
            DesignMissionOverrideObservation::GroupAbsent { .. }
            | DesignMissionOverrideObservation::StatAbsent { .. } => match &self.fallback {
                DesignI64Observation::Measured { raw_i64 } => Ok(*raw_i64),
                DesignI64Observation::Unavailable { reason } => Err(reason.clone()),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignFieldRefusal {
    pub kind: String,
    pub field: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignSourceOccurrence {
    pub source_country_tag: String,
    pub source_country_registry_index: u64,
    pub source_list: String,
    pub source_list_ordinal: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantModelInputs {
    pub creator_tag: String,
    pub origin_override_tag: Option<String>,
    pub explicit_entity: DesignTextObservation,
    pub owned: Vec<DesignSourceOccurrence>,
    pub developed: Vec<DesignSourceOccurrence>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeDesignerCountryInputs {
    pub registry_index: u64,
    pub tag: String,
    pub designer_pointer_order_ordinal: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeDesignerVariantInputs {
    pub slot_identities: DesignSlotIdentityObservation,
    pub append_variant_name: DesignByteObservation,
    /// Exact native label operands; independent of the display-only record name.
    pub display_name: DesignTextObservation,
    pub variant_name: DesignTextObservation,
    pub design_id: String,
    pub catalog_scan_ordinal: u64,
    pub equipment_definition_token: String,
    pub equipment_token_raw: u32,
    pub source_occurrences: Vec<DesignSourceOccurrence>,
    pub item_icon_override: DesignItemIconObservation,
    pub serialized_variant_flag_2b9d: DesignByteObservation,
    pub cached_stats: Vec<DesignFixedStat>,
    pub mission_stat_operands: Vec<DesignMissionStatOperand>,
    pub record: DesignRecord,
    pub model: VariantModelInputs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeDesignerTemplateInputs {
    pub design_id: String,
    pub template_id: u32,
    pub country_289a: DesignI32Observation,
    pub country_3abe: DesignI32Observation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeDesignerInputs {
    pub templates: Vec<NativeDesignerTemplateInputs>,
    pub runtime_dlc_mask: DesignU32Observation,
    pub source_record_id: String,
    pub hour: u64,
    pub countries: Vec<NativeDesignerCountryInputs>,
    pub variants: Vec<NativeDesignerVariantInputs>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignModule {
    pub slot_id: String,
    pub module_token: String,
    pub source_record_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignUpgrade {
    pub upgrade_id: String,
    pub level: i64,
    pub source_record_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignStat {
    pub stat_id: String,
    pub value: f64,
    pub source_record_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignRecord {
    pub design_id: String,
    pub source_record_id: String,
    pub kind: DesignKind,
    pub owner: Option<String>,
    pub name: Option<String>,
    pub archetype: Option<String>,
    pub lifecycle_state: Option<DesignLifecycleState>,
    pub modules: Vec<DesignModule>,
    pub stats: Vec<DesignStat>,
    pub upgrades: Vec<DesignUpgrade>,
    pub definition_completeness: FieldCompleteness,
    pub owner_completeness: FieldCompleteness,
    pub lifecycle_completeness: FieldCompleteness,
    pub module_completeness: FieldCompleteness,
    pub stat_completeness: FieldCompleteness,
    pub upgrade_completeness: FieldCompleteness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DivisionSubunitVector {
    Regiment,
    Support,
    RegimentalSupport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivisionSubunit {
    pub x: u32,
    pub y: u32,
    pub unit_type: String,
    pub source_vector: DivisionSubunitVector,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivisionComposition {
    pub regiment_grid_columns: u32,
    pub regiment_grid_rows: u32,
    pub regiments: Vec<DivisionSubunit>,
    pub support: Vec<DivisionSubunit>,
    pub regimental_support: Vec<DivisionSubunit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayOrderSnapshot {
    pub source_record_id: String,
    pub completeness: FieldCompleteness,
    pub ordered_design_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignCatalogGapCause {
    Unreadable, InvalidRoot, InvalidIdentity, InvalidString, InvalidVector,
    RegistryCountMismatch, Torn, HourLabelMismatch, BoundExceeded, NotObserved,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignCatalogGap {
    pub first_hour_index: u64,
    pub last_hour_index: u64,
    pub cause: DesignCatalogGapCause,
    pub witness: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortableDesignCatalog {
    pub schema: String,
    pub source_record_id: String,
    pub source_sha256: String,
    pub capture_hour_index: u64,
    pub exact_native_catalog: Value,
    pub temporal_deltas: Vec<PortableDesignCatalogDelta>,
    pub gaps: Vec<DesignCatalogGap>,
    pub records: Vec<DesignRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortableDesignCatalogDelta {
    pub sequence: u64,
    pub capture_hour_index: u64,
    pub source_record_id: String,
    pub source_sha256: String,
    pub native_frame_metadata: Value,
    pub variant_upserts: Vec<Value>,
    pub variant_tombstones: Vec<String>,
    pub template_upserts: Vec<Value>,
    pub template_tombstones: Vec<String>,
    pub record_upserts: Vec<DesignRecord>,
    pub ordered_design_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CurrentDesignCatalog {
    pub records: Vec<DesignRecord>,
    pub display_order: DisplayOrderSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserverDesignerFamily {
    Ship,
    Tank,
    Plane,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesignCatalogJoinError {
    CatalogUnavailable,
    ContextUnavailable,
    DuplicateContextId(u32),
    UnknownContextId(u32),
    ContextSelectedDivisionTemplate(u32),
    CatalogHourUnavailable(u64),
}

impl PortableDesignCatalog {
    pub fn decode_a27_live_json(bytes: &[u8]) -> Result<Self, String> {
        let value = crate::parse_json_strict(bytes)
            .map_err(|error| format!("strict A27 live catalog parse: {error}"))?;
        if value.get("schema").and_then(Value::as_str) == Some(A27_LIVE_CATALOG_TIMELINE_SCHEMA) {
            return Self::decode_a27_timeline_value(value);
        }
        if value.get("schema").and_then(serde_json::Value::as_str)
            == Some(PORTABLE_DESIGN_CATALOG_SCHEMA)
        {
            let artifact: PortableArtifact = serde_json::from_value(value)
                .map_err(|error| format!("typed portable A27 catalog projection: {error}"))?;
            if artifact.source_schema != A27_LIVE_CATALOG_SCHEMA {
                return Err("portable A27 catalog source schema mismatch".into());
            }
            if artifact.exact_native_catalog.is_null() {
                let catalog = Self {
                    schema: artifact.source_schema, source_record_id: artifact.source_record_id,
                    source_sha256: artifact.source_sha256, capture_hour_index: artifact.capture_hour_index,
                    exact_native_catalog: Value::Null, temporal_deltas: artifact.temporal_deltas,
                    gaps: artifact.gaps, records: artifact.records,
                };
                catalog.validate_portable()?;
                return Ok(catalog);
            }
            if artifact
                .exact_native_catalog
                .get("schema")
                .and_then(Value::as_str)
                != Some(artifact.source_schema.as_str())
            {
                return Err("portable A27 native authority schema mismatch".into());
            }
            let exact_bytes = serde_json::to_vec(&artifact.exact_native_catalog)
                .map_err(|error| format!("serialize exact portable A27 authority: {error}"))?;
            let mut catalog = Self::decode_a27_live_json(&exact_bytes)?;
            catalog.schema = artifact.source_schema;
            catalog.source_record_id = artifact.source_record_id;
            catalog.source_sha256 = artifact.source_sha256;
            catalog.capture_hour_index = artifact.capture_hour_index;
            catalog.exact_native_catalog = artifact.exact_native_catalog;
            catalog.temporal_deltas = artifact.temporal_deltas;
            catalog.gaps = artifact.gaps;
            rebase_record_ids(&mut catalog.records, &catalog.source_record_id);
            if artifact.records != catalog.records {
                return Err("portable A27 UI projection diverges from native authority".into());
            }
            catalog.validate_portable()?;
            return Ok(catalog);
        }
        reject_designer_outcomes(&value)?;
        let mut exact_native_catalog = value.clone();
        strip_relocation_fields(&mut exact_native_catalog);
        let raw: RawCatalog = serde_json::from_value(value)
            .map_err(|error| format!("typed A27 live catalog projection: {error}"))?;
        if raw.schema != A27_LIVE_CATALOG_SCHEMA {
            return Err(format!(
                "A27 live catalog schema {:?} != {:?}",
                raw.schema, A27_LIVE_CATALOG_SCHEMA
            ));
        }
        if !raw.full_replacement
            || !raw.completeness.definitions
            || !raw.completeness.lifecycle
            || !raw.completeness.membership
            || !raw.completeness.ui_occurrences
        {
            return Err("A27 live catalog is not a complete full replacement".into());
        }
        validate_raw_catalog(&raw)?;
        let capture_hour_index = raw.authority.capture_hour_index;
        if raw.authority.current_date_raw.checked_sub(DATE_EPOCH_RAW) != Some(capture_hour_index) {
            return Err("A27 live catalog date/hour identity is inconsistent".into());
        }
        let source_sha256 = Sha256::hex(bytes);
        let source_record_id = format!("a27-live-catalog:{source_sha256}");
        let mut records = Vec::with_capacity(
            raw.division_templates
                .len()
                .saturating_add(raw.variants.len()),
        );
        let mut ids = BTreeSet::new();

        for (ordinal, row) in raw.division_templates.into_iter().enumerate() {
            if row.catalog_scan_ordinal != ordinal as u64
                || row.runtime_type != 52
                || !row.definition_complete
            {
                return Err(format!(
                    "A27 division template ordinal {ordinal} has incomplete/wrong-typed identity"
                ));
            }
            let design_id = format!("{}:{}", row.id, row.runtime_type);
            if !ids.insert(design_id.clone()) {
                return Err(format!("A27 live catalog duplicates design id {design_id}"));
            }
            let record_id = format!("{source_record_id}:division:{design_id}");
            records.push(DesignRecord {
                design_id,
                source_record_id: record_id,
                kind: DesignKind::DivisionTemplate,
                owner: row.owner_tag.clone(),
                name: row.definition.name.filter(|name| !name.trim().is_empty()),
                archetype: None,
                lifecycle_state: Some(if row.obsolete {
                    DesignLifecycleState::Obsolete
                } else {
                    DesignLifecycleState::Active
                }),
                modules: Vec::new(),
                stats: Vec::new(),
                upgrades: Vec::new(),
                definition_completeness: FieldCompleteness::Complete,
                owner_completeness: if row.owner_tag.is_some() { FieldCompleteness::Complete } else { FieldCompleteness::Unknown },
                lifecycle_completeness: FieldCompleteness::Complete,
                module_completeness: FieldCompleteness::NotApplicable,
                stat_completeness: FieldCompleteness::NotApplicable,
                upgrade_completeness: FieldCompleteness::NotApplicable,
            });
        }

        for (ordinal, row) in raw.variants.into_iter().enumerate() {
            if row.catalog_scan_ordinal != ordinal as u64
                || row.runtime_type != 70
                || !row.definition_complete
                || !row.definition.modules_complete
                || !row.definition.cached_stats_complete
                || !row.definition.upgrades_complete
                || row.owner_tag.trim().is_empty()
                || row.definition.archetype_token.trim().is_empty()
            {
                return Err(format!(
                    "A27 equipment variant ordinal {ordinal} has incomplete/wrong-typed identity"
                ));
            }
            let design_id = format!("{}:{}", row.id, row.runtime_type);
            if !ids.insert(design_id.clone()) {
                return Err(format!("A27 live catalog duplicates design id {design_id}"));
            }
            let record_id = format!("{source_record_id}:variant:{design_id}");
            let mut module_slots = BTreeSet::new();
            let modules = row
                .definition
                .modules
                .into_iter()
                .map(|module| {
                    let slot_id = format!("handle:{}", module.slot_handle);
                    if module.module_token.trim().is_empty()
                        || !module_slots.insert(slot_id.clone())
                    {
                        return Err(format!(
                            "A27 variant {design_id} has invalid/duplicate module slot"
                        ));
                    }
                    Ok(DesignModule {
                        slot_id,
                        module_token: module.module_token,
                        source_record_id: format!("{record_id}:module:{}", module.source_ordinal),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut stat_ids = BTreeSet::new();
            let stats = row
                .definition
                .cached_stats
                .into_iter()
                .map(|stat| {
                    if stat.stat_id.trim().is_empty()
                        || stat.scale <= 0
                        || !stat_ids.insert(stat.stat_id.clone())
                    {
                        return Err(format!(
                            "A27 variant {design_id} has invalid/duplicate stat"
                        ));
                    }
                    Ok(DesignStat {
                        stat_id: stat.stat_id,
                        value: stat.raw_i64 as f64 / stat.scale as f64,
                        source_record_id: format!("{record_id}:stat:{}", stat.source_ordinal),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut upgrade_ids = BTreeSet::new();
            let upgrades = row
                .definition
                .upgrades
                .into_iter()
                .map(|upgrade| {
                    let upgrade_id = format!("handle:{}", upgrade.upgrade_handle);
                    if upgrade.level < 0 || !upgrade_ids.insert(upgrade_id.clone()) {
                        return Err(format!(
                            "A27 variant {design_id} has invalid/duplicate upgrade"
                        ));
                    }
                    Ok(DesignUpgrade {
                        upgrade_id,
                        level: upgrade.level,
                        source_record_id: format!("{record_id}:upgrade:{}", upgrade.source_ordinal),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let name = row
                .definition
                .display_name
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    row.definition
                        .variant_name
                        .filter(|value| !value.trim().is_empty())
                });
            records.push(DesignRecord {
                design_id,
                source_record_id: record_id,
                kind: DesignKind::EquipmentVariant,
                owner: Some(row.owner_tag),
                name,
                archetype: Some(row.definition.archetype_token),
                lifecycle_state: Some(if row.obsolete {
                    DesignLifecycleState::Obsolete
                } else {
                    DesignLifecycleState::Active
                }),
                modules,
                stats,
                upgrades,
                definition_completeness: FieldCompleteness::Complete,
                owner_completeness: FieldCompleteness::Complete,
                lifecycle_completeness: FieldCompleteness::Complete,
                module_completeness: FieldCompleteness::Complete,
                stat_completeness: FieldCompleteness::Complete,
                upgrade_completeness: FieldCompleteness::Complete,
            });
        }

        Ok(Self {
            schema: raw.schema,
            source_record_id,
            source_sha256,
            capture_hour_index,
            exact_native_catalog,
            temporal_deltas: Vec::new(),
            gaps: Vec::new(),
            records,
        })
    }

    fn decode_a27_timeline_value(value: Value) -> Result<Self, String> {
        let authority = value
            .get("authority")
            .and_then(Value::as_object)
            .ok_or("A27 timeline authority is missing")?;
        if authority.get("program").and_then(Value::as_str) != Some(EXACT_A729_PROGRAM)
            || authority.get("program_sha256").and_then(Value::as_str) != Some(EXACT_A729_SHA256)
            || value.get("complete_sequence").and_then(Value::as_bool) != Some(true)
            || value.get("transport").and_then(Value::as_str)
                != Some("first_full_then_native_row_deltas")
            || value.get("fatal").is_some()
        {
            return Err("A27 timeline has unauthenticated/incomplete authority".into());
        }
        let entries = value
            .get("captures")
            .and_then(Value::as_array)
            .filter(|entries| !entries.is_empty())
            .ok_or("A27 timeline has no outcomes")?;
        if value["capture_count"].as_u64() != Some(entries.len() as u64) {
            return Err("A27 timeline outcome count disagrees with sequence".into());
        }
        let first = value["first_capture_hour_index"]
            .as_u64()
            .ok_or("A27 timeline first hour missing")?;
        let last = value["last_capture_hour_index"]
            .as_u64()
            .ok_or("A27 timeline last hour missing")?;
        let mut expected = Some(first);
        let mut baseline: Option<Self> = None;
        let mut previous: Option<Self> = None;
        let mut previous_snapshot: Option<Value> = None;
        let mut gaps = Vec::new();
        let mut rechecks = 0u64;
        let mut after_gap = false;
        for (sequence, entry) in entries.iter().enumerate() {
            if entry["sequence"].as_u64() != Some(sequence as u64) {
                return Err(format!(
                    "A27 timeline capture {sequence} has a sequence gap"
                ));
            }
            if let Some(gap) = entry.get("gap") {
                if entry.get("snapshot").is_some()
                    || entry.get("delta").is_some()
                    || entry.get("snapshot_sha256").is_some()
                    || entry.get("same_hour_rechecks").is_some()
                {
                    return Err("A27 gap also claims a measurement".into());
                }
                let gap: DesignCatalogGap = serde_json::from_value(gap.clone())
                    .map_err(|error| format!("A27 typed gap: {error}"))?;
                if Some(gap.first_hour_index) != expected
                    || gap.first_hour_index > gap.last_hour_index
                    || gap.last_hour_index > last
                    || gap
                        .witness
                        .as_object()
                        .is_none_or(|witness| witness.is_empty())
                    || entry["capture_hour_index"].as_u64() != Some(gap.first_hour_index)
                {
                    return Err("A27 gap range/witness disagrees with sequence".into());
                }
                expected = gap.last_hour_index.checked_add(1);
                gaps.push(gap);
                after_gap = true;
                continue;
            }
            let full = previous_snapshot.is_none() || after_gap;
            let snapshot = if full {
                if entry.get("delta").is_some() {
                    return Err("A27 gap recovery requires a complete baseline".into());
                }
                entry
                    .get("snapshot")
                    .cloned()
                    .ok_or("A27 timeline baseline has no full snapshot")?
            } else {
                if entry.get("snapshot").is_some() {
                    return Err("A27 timeline repeats a full snapshot without a gap".into());
                }
                apply_a27_row_delta(
                    previous_snapshot.as_ref().unwrap(),
                    entry.get("delta").ok_or("A27 timeline delta missing")?,
                )?
            };
            let bytes = serde_json::to_vec(&canonical_json_value(&snapshot))
                .map_err(|error| error.to_string())?;
            let hash = Sha256::hex(&bytes);
            let checks = entry["same_hour_rechecks"]
                .as_u64()
                .filter(|count| *count > 0)
                .ok_or("A27 timeline lacks a same-hour mutation recheck")?;
            let hashes = entry["same_hour_recheck_sha256"]
                .as_array()
                .ok_or("A27 timeline lacks recheck hashes")?;
            if entry["snapshot_sha256"].as_str() != Some(hash.as_str())
                || hashes.len() as u64 != checks
                || hashes
                    .iter()
                    .any(|witness| witness.as_str() != Some(hash.as_str()))
            {
                return Err(format!(
                    "A27 timeline capture {sequence} hash/recheck mismatch"
                ));
            }
            rechecks = rechecks
                .checked_add(checks)
                .ok_or("A27 recheck count overflow")?;
            let current = Self::decode_a27_live_json(&bytes)?;
            if Some(current.capture_hour_index) != expected
                || current.capture_hour_index > last
                || entry["capture_hour_index"].as_u64() != Some(current.capture_hour_index)
            {
                return Err("A27 timeline census is not the next game hour".into());
            }
            expected = current.capture_hour_index.checked_add(1);
            if let Some(baseline) = baseline.as_mut() {
                let mut delta = build_temporal_delta(
                    baseline.temporal_deltas.len() as u64 + 1,
                    previous.as_ref().unwrap(),
                    &current,
                )?;
                if after_gap {
                    delta.variant_upserts = current.exact_native_catalog["variants"]
                        .as_array()
                        .unwrap()
                        .clone();
                    delta.template_upserts = current.exact_native_catalog["division_templates"]
                        .as_array()
                        .unwrap()
                        .clone();
                    delta.record_upserts = current.records.clone();
                }
                baseline.temporal_deltas.push(delta);
            } else {
                baseline = Some(current.clone());
            }
            previous = Some(current);
            previous_snapshot = Some(snapshot);
            after_gap = false;
        }
        if expected != last.checked_add(1)
            || value["same_hour_recheck_count"].as_u64() != Some(rechecks)
        {
            return Err("A27 timeline terminal/recheck summary disagrees with outcomes".into());
        }
        let mut catalog = match baseline {
            Some(catalog) => catalog,
            None => {
                let bytes = serde_json::to_vec(&canonical_json_value(&value))
                    .map_err(|error| error.to_string())?;
                let hash = Sha256::hex(&bytes);
                Self {
                    schema: A27_LIVE_CATALOG_SCHEMA.into(),
                    source_record_id: format!("a27-live-catalog:{hash}"),
                    source_sha256: hash,
                    capture_hour_index: first,
                    exact_native_catalog: Value::Null,
                    temporal_deltas: Vec::new(),
                    gaps: Vec::new(),
                    records: Vec::new(),
                }
            }
        };
        catalog.gaps = gaps;
        catalog.validate_portable()?;
        if catalog.coverage_range()? != (first, last) {
            return Err("A27 timeline coverage summary mismatch".into());
        }
        Ok(catalog)
    }

    fn refuse_gap(&self, hour: u64) -> Result<(), String> {
        if let Some(gap) = self
            .gaps
            .iter()
            .find(|gap| (gap.first_hour_index..=gap.last_hour_index).contains(&hour))
        {
            return Err(format!(
                "A27 catalog gap at hour {hour}: {:?}; witness {}",
                gap.cause, gap.witness
            ));
        }
        Ok(())
    }

    pub fn coverage_range(&self) -> Result<(u64, u64), String> {
        let mut spans = Vec::new();
        if !self.exact_native_catalog.is_null() {
            spans.push((self.capture_hour_index, self.capture_hour_index));
            spans.extend(
                self.temporal_deltas
                    .iter()
                    .map(|delta| (delta.capture_hour_index, delta.capture_hour_index)),
            );
        }
        let mut previous_gap = None;
        for gap in &self.gaps {
            if gap.first_hour_index > gap.last_hour_index
                || previous_gap.is_some_and(|last| gap.first_hour_index <= last)
                || gap
                    .witness
                    .as_object()
                    .is_none_or(|witness| witness.is_empty())
            {
                return Err("A27 gap range/order/witness is invalid".into());
            }
            previous_gap = Some(gap.last_hour_index);
            spans.push((gap.first_hour_index, gap.last_hour_index));
        }
        spans.sort_unstable();
        let first = spans
            .first()
            .ok_or("A27 catalog has no measured or gap hours")?
            .0;
        let mut last = spans[0].1;
        for &(start, end) in spans.iter().skip(1) {
            if last.checked_add(1) != Some(start) {
                return Err("A27 catalog overlaps or omits an hour".into());
            }
            last = end;
        }
        Ok((first, last))
    }

    pub fn records_at_hour(&self, hour: u64) -> Result<Vec<DesignRecord>, String> {
        self.refuse_gap(hour)?;
        if hour == self.capture_hour_index {
            return Ok(self.records.clone());
        }
        let mut records = self
            .records
            .iter()
            .cloned()
            .map(|record| (record.design_id.clone(), record))
            .collect::<std::collections::BTreeMap<_, _>>();
        for delta in &self.temporal_deltas {
            for design_id in delta
                .variant_tombstones
                .iter()
                .chain(delta.template_tombstones.iter())
            {
                records.remove(design_id);
            }
            for record in &delta.record_upserts {
                records.insert(record.design_id.clone(), record.clone());
            }
            if delta.capture_hour_index == hour {
                let mut ordered = Vec::with_capacity(delta.ordered_design_ids.len());
                for design_id in &delta.ordered_design_ids {
                    ordered.push(records.remove(design_id).ok_or_else(|| {
                        format!(
                            "A27 delta {} order references absent design {design_id}",
                            delta.sequence
                        )
                    })?);
                }
                if !records.is_empty() {
                    return Err(format!(
                        "A27 delta {} order omits reconstructed designs",
                        delta.sequence
                    ));
                }
                return Ok(ordered);
            }
            if delta.capture_hour_index > hour {
                break;
            }
        }
        Err(format!(
            "A27 has no authenticated census for replay hour {hour}"
        ))
    }

    fn exact_native_at_hour(&self, hour: u64) -> Result<Value, String> {
        self.refuse_gap(hour)?;
        if hour == self.capture_hour_index {
            return Ok(self.exact_native_catalog.clone());
        }
        let mut rows = exact_rows(self, "division_templates")?;
        rows.extend(exact_rows(self, "variants")?);
        for delta in &self.temporal_deltas {
            for design_id in delta
                .variant_tombstones
                .iter()
                .chain(delta.template_tombstones.iter())
            {
                rows.remove(design_id);
            }
            for row in delta
                .variant_upserts
                .iter()
                .chain(delta.template_upserts.iter())
            {
                rows.insert(
                    format!(
                        "{}:{}",
                        row.get("id").and_then(Value::as_u64).unwrap_or_default(),
                        row.get("type").and_then(Value::as_u64).unwrap_or_default()
                    ),
                    row.clone(),
                );
            }
            if delta.capture_hour_index == hour {
                let mut frame = delta.native_frame_metadata.clone();
                let object = frame.as_object_mut().ok_or_else(|| {
                    format!("A27 delta {} metadata is not an object", delta.sequence)
                })?;
                let mut templates = Vec::new();
                let mut variants = Vec::new();
                for design_id in &delta.ordered_design_ids {
                    let mut row = rows
                        .get(design_id)
                        .ok_or_else(|| {
                            format!(
                                "A27 delta {} order references absent design {design_id}",
                                delta.sequence
                            )
                        })?
                        .clone();
                    match row.get("type").and_then(Value::as_u64) {
                        Some(52) => {
                            row["catalog_scan_ordinal"] = (templates.len() as u64).into();
                            templates.push(row);
                        }
                        Some(70) => {
                            row["catalog_scan_ordinal"] = (variants.len() as u64).into();
                            variants.push(row);
                        }
                        _ => {
                            return Err(format!(
                                "A27 delta {} contains a wrong-typed row",
                                delta.sequence
                            ))
                        }
                    }
                }
                object.insert("division_templates".into(), Value::Array(templates));
                object.insert("variants".into(), Value::Array(variants));
                validate_exact_native_catalog(&frame, hour)?;
                return Ok(frame);
            }
            if delta.capture_hour_index > hour {
                break;
            }
        }
        Err(format!(
            "A27 has no authenticated census for replay hour {hour}"
        ))
    }

    pub fn native_row_delta_from(&self, previous: &Self) -> Result<Value, String> {
        let delta = build_temporal_delta(1, previous, self)?;
        Ok(serde_json::json!({
            "frame_metadata": delta.native_frame_metadata,
            "variant_upserts": delta.variant_upserts,
            "variant_tombstones": delta.variant_tombstones,
            "template_upserts": delta.template_upserts,
            "template_tombstones": delta.template_tombstones,
            "ordered_design_ids": delta.ordered_design_ids,
        }))
    }

    pub fn native_designer_inputs_at_hour(&self, hour: u64) -> Result<NativeDesignerInputs, String> {
        let exact = self.exact_native_at_hour(hour)?;
        let raw: RawCatalog = serde_json::from_value(exact)
            .map_err(|error| format!("A27 native designer inputs: {error}"))?;
        validate_raw_catalog(&raw)?;
        let mut records: BTreeMap<_, _> = self.records_at_hour(hour)?.into_iter()
            .map(|record| (record.design_id.clone(), record)).collect();
        let variants = raw.variants.into_iter().map(|row| {
            let design_id = format!("{}:{}", row.id, row.runtime_type);
            let record = records.remove(&design_id)
                .ok_or_else(|| format!("A27 native designer row {design_id} has no exact-hour record"))?;
            let model = row.model_inputs();
            Ok(NativeDesignerVariantInputs {
                slot_identities: row.definition.slot_identities,
                append_variant_name: row.definition.append_variant_name,
                // validate_raw_catalog checked both original text/hex pairs.
                // Native mandatory reads encode only a measured empty as null.
                display_name: DesignTextObservation::Measured {
                    value: row.definition.display_name.unwrap_or_default(),
                    utf8_hex: row.definition.display_name_utf8_hex,
                },
                variant_name: DesignTextObservation::Measured {
                    value: row.definition.variant_name.unwrap_or_default(),
                    utf8_hex: row.definition.variant_name_utf8_hex,
                },
                design_id, catalog_scan_ordinal: row.catalog_scan_ordinal,
                equipment_definition_token: row.definition.archetype_token,
                equipment_token_raw: row.definition.equipment_token_raw,
                source_occurrences: row.source_occurrences,
                item_icon_override: row.definition.item_icon_override,
                serialized_variant_flag_2b9d: row.definition.serialized_variant_flag_2b9d,
                cached_stats: row.definition.cached_stats,
                mission_stat_operands: row.definition.mission_stat_operands,
                record, model,
            })
        }).collect::<Result<Vec<_>, String>>()?;
        let source_record_id = self.temporal_deltas.iter()
            .find(|delta| delta.capture_hour_index == hour)
            .map(|delta| delta.source_record_id.clone()).unwrap_or_else(|| self.source_record_id.clone());
        Ok(NativeDesignerInputs {
            runtime_dlc_mask: raw.runtime_dlc_mask,
            source_record_id, hour, variants,
            templates: raw.division_templates.into_iter().map(|row| {
                let design_id = format!("{}:{}", row.id, row.runtime_type);
                let record = records.remove(&design_id)
                    .ok_or_else(|| format!("A27 native template {design_id} has no exact-hour record"))?;
                if row.runtime_type != 52 || record.kind != DesignKind::DivisionTemplate {
                    return Err("A27 native template projection has a wrong typed identity".into());
                }
                Ok(NativeDesignerTemplateInputs {
                    design_id, template_id: row.id,
                    country_289a: row.definition.country_289a,
                    country_3abe: row.definition.country_3abe,
                })
            }).collect::<Result<Vec<_>, String>>()?,
            countries: raw.countries.into_iter().map(|row| NativeDesignerCountryInputs {
                registry_index: row.registry_index, tag: row.tag,
                designer_pointer_order_ordinal: row.designer_pointer_order_ordinal,
            }).collect(),
        })
    }

    pub fn variant_model_inputs_at_hour(
        &self,
        hour: u64,
        variant_id: u32,
    ) -> Result<VariantModelInputs, String> {
        let exact = self.exact_native_at_hour(hour)?;
        let raw: RawCatalog = serde_json::from_value(exact)
            .map_err(|error| format!("A27 variant model inputs: {error}"))?;
        validate_raw_catalog(&raw)?;
        let row = raw.variants.into_iter().find(|row| row.id == variant_id)
            .ok_or_else(|| format!("A27 variant {variant_id} is absent at hour {hour}"))?;
        Ok(row.model_inputs())
    }

    pub fn division_composition_at_hour(
        &self,
        hour: u64,
    ) -> Result<BTreeMap<String, DivisionComposition>, String> {
        self.refuse_gap(hour)?;
        let mut rows = exact_rows(self, "division_templates")?;
        if hour == self.capture_hour_index {
            return project_division_compositions(rows);
        }
        for delta in &self.temporal_deltas {
            for design_id in &delta.template_tombstones {
                rows.remove(design_id);
            }
            for row in &delta.template_upserts {
                rows.insert(exact_row_id(row, 52)?, row.clone());
            }
            if delta.capture_hour_index == hour {
                return project_division_compositions(rows);
            }
            if delta.capture_hour_index > hour {
                break;
            }
        }
        Err(format!(
            "A27 has no authenticated census for replay hour {hour}"
        ))
    }

    pub fn validate_replay_hours(&self, hours: &[i64]) -> Result<(), String> {
        let first = hours
            .first()
            .copied()
            .and_then(|hour| u64::try_from(hour).ok())
            .ok_or_else(|| "A27 replay hour index is empty/negative".to_string())?;
        let last = hours
            .last()
            .copied()
            .and_then(|hour| u64::try_from(hour).ok())
            .ok_or_else(|| "A27 replay hour index is empty/negative".to_string())?;
        let (catalog_first,catalog_last)=self.coverage_range()?;
        if first!=catalog_first || last!=catalog_last
            || hours.len() as u64!=last.checked_sub(first).and_then(|span|span.checked_add(1)).ok_or("A27 replay hour span overflow")?
            || hours.windows(2).any(|pair|pair[0].checked_add(1)!=Some(pair[1])) {
            return Err(format!("A27 catalog hours {catalog_first}..{catalog_last} do not exactly cover replay hours {first}..{last}"));
        }
        Ok(())
    }

    pub fn encode_portable_json(&self) -> Result<Vec<u8>, String> {
        self.validate_portable()?;
        serde_json::to_vec(&PortableArtifact {
            schema: PORTABLE_DESIGN_CATALOG_SCHEMA.to_string(),
            source_schema: self.schema.clone(),
            source_record_id: self.source_record_id.clone(),
            source_sha256: self.source_sha256.clone(),
            capture_hour_index: self.capture_hour_index,
            exact_native_catalog: self.exact_native_catalog.clone(),
            temporal_deltas: self.temporal_deltas.clone(),
            gaps: self.gaps.clone(),
            records: self.records.clone(),
        })
        .map_err(|error| format!("serialize portable A27 catalog: {error}"))
    }

    fn validate_portable(&self) -> Result<(), String> {
        if self.schema != A27_LIVE_CATALOG_SCHEMA
            || self.source_record_id != format!("a27-live-catalog:{}", self.source_sha256)
            || self.source_sha256.len() != 64
            || !self
                .source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("portable A27 catalog source identity is invalid".into());
        }
        let coverage=self.coverage_range()?;
        if self.exact_native_catalog.is_null() {
            if !self.records.is_empty() || !self.temporal_deltas.is_empty() || self.gaps.is_empty()
                || self.capture_hour_index!=coverage.0 {
                return Err("A27 unavailable baseline also claims measured state".into());
            }
            return Ok(());
        }
        validate_exact_native_catalog(&self.exact_native_catalog, self.capture_hour_index)?;
        let mut ids = BTreeSet::new();
        for record in &self.records {
            let owner_valid = match (&record.owner, record.owner_completeness) {
                (Some(owner), FieldCompleteness::Complete) => !owner.is_empty(),
                (None, FieldCompleteness::Unknown) => record.kind == DesignKind::DivisionTemplate,
                _ => false,
            };
            if !owner_valid || record.design_id.trim().is_empty()
                || record.source_record_id.trim().is_empty()
                || !ids.insert(record.design_id.as_str())
                || record.definition_completeness != FieldCompleteness::Complete
                || record.lifecycle_completeness != FieldCompleteness::Complete
                || record.lifecycle_state.is_none()
            {
                return Err("portable A27 catalog contains an incomplete/duplicate record".into());
            }
            if record.kind == DesignKind::EquipmentVariant
                && (record.module_completeness != FieldCompleteness::Complete
                    || record.stat_completeness != FieldCompleteness::Complete
                    || record.upgrade_completeness != FieldCompleteness::Complete
                    || record.archetype.as_deref().is_none_or(str::is_empty))
            {
                return Err("portable A27 variant is incomplete".into());
            }
        }
        let exact_ids = exact_design_ids(&self.exact_native_catalog)?;
        if exact_ids
            != self
                .records
                .iter()
                .map(|record| record.design_id.clone())
                .collect::<Vec<_>>()
        {
            return Err(
                "portable A27 UI projection diverges from its exact native authority".into(),
            );
        }
        validate_temporal_deltas(self)?;
        Ok(())
    }

    pub fn get(&self, design_id: &str) -> Option<&DesignRecord> {
        self.records
            .iter()
            .find(|record| record.design_id == design_id)
    }
}

fn strip_relocation_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.retain(|key, _| {
                key != "pointer_identity"
                    && !key.ends_with("_pointer")
                    && !matches!(key.as_str(), "pid" | "module_base" | "game_state")
            });
            for child in object.values_mut() {
                strip_relocation_fields(child);
            }
        }
        Value::Array(array) => {
            for child in array {
                strip_relocation_fields(child);
            }
        }
        _ => {}
    }
}

pub fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut canonical = serde_json::Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonical_json_value(&object[key]));
            }
            Value::Object(canonical)
        }
        Value::Array(array) => Value::Array(array.iter().map(canonical_json_value).collect()),
        _ => value.clone(),
    }
}

pub fn apply_a27_row_delta(previous: &Value, delta: &Value) -> Result<Value, String> {
    let mut rows = BTreeMap::new();
    for key in ["division_templates", "variants"] {
        for row in previous
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("A27 previous timeline frame is missing {key}"))?
        {
            let id = row
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("A27 previous {key} row has no id"))?;
            let runtime_type = row
                .get("type")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("A27 previous {key} row has no type"))?;
            rows.insert(format!("{id}:{runtime_type}"), row.clone());
        }
    }
    let mut tombstones = BTreeSet::new();
    for key in ["variant_tombstones", "template_tombstones"] {
        for id in delta
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("A27 raw timeline delta is missing {key}"))?
        {
            let id = id
                .as_str()
                .ok_or_else(|| format!("A27 raw timeline {key} contains a non-string"))?;
            if !tombstones.insert(id.to_string()) || rows.remove(id).is_none() {
                return Err(format!("A27 raw timeline tombstones absent design {id}"));
            }
        }
    }
    let mut upserts = BTreeSet::new();
    for key in ["variant_upserts", "template_upserts"] {
        for row in delta
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("A27 raw timeline delta is missing {key}"))?
        {
            let id = row
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("A27 raw timeline {key} row has no id"))?;
            let runtime_type = row
                .get("type")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("A27 raw timeline {key} row has no type"))?;
            if (key == "variant_upserts" && runtime_type != 70)
                || (key == "template_upserts" && runtime_type != 52)
            {
                return Err(format!("A27 raw timeline {key} row has wrong type"));
            }
            let stable_id = format!("{id}:{runtime_type}");
            if tombstones.contains(&stable_id) || !upserts.insert(stable_id.clone()) {
                return Err(format!(
                    "A27 raw timeline duplicates/conflicts upsert {stable_id}"
                ));
            }
            rows.insert(stable_id, row.clone());
        }
    }
    let mut frame = delta
        .get("frame_metadata")
        .cloned()
        .ok_or_else(|| "A27 raw timeline delta has no frame metadata".to_string())?;
    let object = frame
        .as_object_mut()
        .ok_or_else(|| "A27 raw timeline frame metadata is not an object".to_string())?;
    if object.contains_key("variants") || object.contains_key("division_templates") {
        return Err("A27 raw timeline frame metadata contains catalog rows".into());
    }
    let order = delta
        .get("ordered_design_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| "A27 raw timeline delta has no complete order".to_string())?;
    let mut templates = Vec::new();
    let mut variants = Vec::new();
    for id in order {
        let id = id
            .as_str()
            .ok_or_else(|| "A27 raw timeline order contains a non-string".to_string())?;
        let mut row = rows
            .remove(id)
            .ok_or_else(|| format!("A27 raw timeline order references absent design {id}"))?;
        match row.get("type").and_then(Value::as_u64) {
            Some(52) => {
                row["catalog_scan_ordinal"] = (templates.len() as u64).into();
                templates.push(row);
            }
            Some(70) => {
                row["catalog_scan_ordinal"] = (variants.len() as u64).into();
                variants.push(row);
            }
            _ => return Err("A27 raw timeline order references wrong-typed design".into()),
        }
    }
    if !rows.is_empty() {
        return Err("A27 raw timeline order omits catalog designs".into());
    }
    object.insert("division_templates".into(), Value::Array(templates));
    object.insert("variants".into(), Value::Array(variants));
    Ok(frame)
}

fn exact_rows(
    catalog: &PortableDesignCatalog,
    key: &str,
) -> Result<BTreeMap<String, Value>, String> {
    catalog
        .exact_native_catalog
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("A27 exact authority is missing {key}"))?
        .iter()
        .cloned()
        .map(|row| {
            let id = row
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("A27 {key} row has no id"))?;
            let runtime_type = row
                .get("type")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("A27 {key} row has no type"))?;
            Ok((format!("{id}:{runtime_type}"), row))
        })
        .collect()
}

fn project_division_compositions(
    rows: BTreeMap<String, Value>,
) -> Result<BTreeMap<String, DivisionComposition>, String> {
    rows.into_iter()
        .map(|(design_id, row)| {
            let raw: RawDivisionTemplate = serde_json::from_value(row)
                .map_err(|error| format!("typed A27 division composition {design_id}: {error}"))?;
            if raw.runtime_type != 52 || format!("{}:{}", raw.id, raw.runtime_type) != design_id {
                return Err(format!(
                    "A27 division composition {design_id} has inconsistent identity"
                ));
            }
            let definition = raw.definition;
            let stride = definition.regiment_grid_rows;
            if stride == 0 || stride > i16::MAX as u32
                || definition.regiment_grid_columns == 0
                || definition.regiment_grid_columns > i16::MAX as u32
                || definition.regiment_pointer_count > i16::MAX as u32
                || definition.support_pointer_count > i16::MAX as u32
                || definition.regimental_support_pointer_count > i16::MAX as u32
            {
                return Err(format!("A27 division composition {design_id} exceeds the signed-short grid"));
            }
            let project = |rows: Vec<RawSubunit>, source_vector| -> Result<Vec<DivisionSubunit>, String> {
                rows.into_iter().map(|row| {
                    let expected = match source_vector {
                        DivisionSubunitVector::Regiment => (row.source_ordinal / stride, row.source_ordinal % stride),
                        DivisionSubunitVector::Support => (0, row.source_ordinal),
                        DivisionSubunitVector::RegimentalSupport => (row.source_ordinal, 0),
                    };
                    if (row.x, row.y) != expected
                        || (source_vector == DivisionSubunitVector::Regiment
                            && row.x >= definition.regiment_grid_columns)
                    {
                        return Err(format!("A27 division composition {design_id} {source_vector:?} coordinate differs from source ordinal {}", row.source_ordinal));
                    }
                    Ok(DivisionSubunit {
                        x: row.x, y: row.y, unit_type: row.unit_type, source_vector,
                    })
                }).collect()
            };
            let composition = DivisionComposition {
                regiment_grid_columns: definition.regiment_grid_columns,
                regiment_grid_rows: definition.regiment_grid_rows,
                regiments: project(definition.regiments, DivisionSubunitVector::Regiment)?,
                support: project(definition.support, DivisionSubunitVector::Support)?,
                regimental_support: project(
                    definition.regimental_support,
                    DivisionSubunitVector::RegimentalSupport,
                )?,
            };
            Ok((design_id, composition))
        })
        .collect()
}


fn same_native_row_ignoring_catalog_ordinal(left: &Value, right: &Value) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    if let Some(object) = left.as_object_mut() {
        object.remove("catalog_scan_ordinal");
    }
    if let Some(object) = right.as_object_mut() {
        object.remove("catalog_scan_ordinal");
    }
    left == right
}

fn build_temporal_delta(
    sequence: u64,
    previous: &PortableDesignCatalog,
    current: &PortableDesignCatalog,
) -> Result<PortableDesignCatalogDelta, String> {
    let previous_variants = exact_rows(previous, "variants")?;
    let current_variants = exact_rows(current, "variants")?;
    let previous_templates = exact_rows(previous, "division_templates")?;
    let current_templates = exact_rows(current, "division_templates")?;
    let variant_tombstones = previous_variants
        .keys()
        .filter(|id| !current_variants.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    let template_tombstones = previous_templates
        .keys()
        .filter(|id| !current_templates.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    let variant_upserts = current_variants
        .iter()
        .filter_map(|(id, row)| {
            previous_variants
                .get(id)
                .is_none_or(|previous| !same_native_row_ignoring_catalog_ordinal(previous, row))
                .then(|| row.clone())
        })
        .collect::<Vec<_>>();
    let template_upserts = current_templates
        .iter()
        .filter_map(|(id, row)| {
            previous_templates
                .get(id)
                .is_none_or(|previous| !same_native_row_ignoring_catalog_ordinal(previous, row))
                .then(|| row.clone())
        })
        .collect::<Vec<_>>();
    let changed_ids = variant_upserts
        .iter()
        .chain(template_upserts.iter())
        .map(|row| {
            format!(
                "{}:{}",
                row.get("id").and_then(Value::as_u64).unwrap_or_default(),
                row.get("type").and_then(Value::as_u64).unwrap_or_default()
            )
        })
        .collect::<BTreeSet<_>>();
    let record_upserts = current
        .records
        .iter()
        .filter(|record| changed_ids.contains(&record.design_id))
        .cloned()
        .collect::<Vec<_>>();
    let mut native_frame_metadata = current.exact_native_catalog.clone();
    if let Some(object) = native_frame_metadata.as_object_mut() {
        object.remove("variants");
        object.remove("division_templates");
    }
    Ok(PortableDesignCatalogDelta {
        sequence,
        capture_hour_index: current.capture_hour_index,
        source_record_id: current.source_record_id.clone(),
        source_sha256: current.source_sha256.clone(),
        native_frame_metadata,
        variant_upserts,
        variant_tombstones,
        template_upserts,
        template_tombstones,
        record_upserts,
        ordered_design_ids: current
            .records
            .iter()
            .map(|record| record.design_id.clone())
            .collect(),
    })
}

fn rebase_record_ids(records: &mut [DesignRecord], source_record_id: &str) {
    for record in records {
        let family = if record.kind == DesignKind::DivisionTemplate {
            "division"
        } else {
            "variant"
        };
        record.source_record_id = format!("{source_record_id}:{family}:{}", record.design_id);
        for (ordinal, module) in record.modules.iter_mut().enumerate() {
            module.source_record_id = format!("{}:module:{ordinal}", record.source_record_id);
        }
        for (ordinal, stat) in record.stats.iter_mut().enumerate() {
            stat.source_record_id = format!("{}:stat:{ordinal}", record.source_record_id);
        }
        for (ordinal, upgrade) in record.upgrades.iter_mut().enumerate() {
            upgrade.source_record_id = format!("{}:upgrade:{ordinal}", record.source_record_id);
        }
    }
}

fn contains_relocation_field(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, child)| {
            key == "pointer_identity"
                || key.ends_with("_pointer")
                || matches!(key.as_str(), "pid" | "module_base" | "game_state")
                || contains_relocation_field(child)
        }),
        Value::Array(array) => array.iter().any(contains_relocation_field),
        _ => false,
    }
}

fn validate_exact_native_catalog(value: &Value, capture_hour_index: u64) -> Result<(), String> {
    reject_designer_outcomes(value)?;
    if contains_relocation_field(value) {
        return Err(
            "portable A27 native authority contains process-local relocation fields".into(),
        );
    }
    let schema = value.get("schema").and_then(Value::as_str);
    if schema != Some(A27_LIVE_CATALOG_SCHEMA)
        || value.get("full_replacement").and_then(Value::as_bool) != Some(true)
    {
        return Err("portable A27 native authority has the wrong schema/replacement law".into());
    }
    let authority = value
        .get("authority")
        .and_then(Value::as_object)
        .ok_or_else(|| "portable A27 native authority is missing capture identity".to_string())?;
    if authority.get("capture_hour_index").and_then(Value::as_u64) != Some(capture_hour_index)
        || authority
            .get("current_date_raw")
            .and_then(Value::as_u64)
            .and_then(|raw| raw.checked_sub(DATE_EPOCH_RAW))
            != Some(capture_hour_index)
    {
        return Err("portable A27 native authority date/hour identity is inconsistent".into());
    }
    if authority.get("coherence").and_then(Value::as_str)
        != Some("double_read_all_roots_fields_and_membership")
    {
        return Err("portable A27 native authority lacks the whole-sweep coherence seal".into());
    }
    let countries = value
        .get("countries")
        .and_then(Value::as_array)
        .ok_or_else(|| "portable A27 native authority is missing countries".to_string())?;
    if value.get("country_count").and_then(Value::as_u64) != Some(countries.len() as u64) {
        return Err("portable A27 native authority country count is inconsistent".into());
    }
    for key in ["variants", "division_templates"] {
        if value.get(key).and_then(Value::as_array).is_none() {
            return Err(format!("portable A27 native authority is missing {key}"));
        }
    }
    let raw: RawCatalog = serde_json::from_value(value.clone())
        .map_err(|error| format!("typed exact portable A27 authority: {error}"))?;
    if !raw.full_replacement
        || !raw.completeness.definitions
        || !raw.completeness.lifecycle
        || !raw.completeness.membership
        || !raw.completeness.ui_occurrences
    {
        return Err("portable A27 native authority is incomplete".into());
    }
    validate_raw_catalog(&raw)?;
    Ok(())
}

fn exact_design_ids(value: &Value) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    for (key, expected_type) in [("division_templates", 52_u64), ("variants", 70_u64)] {
        let rows = value
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("portable A27 native authority is missing {key}"))?;
        for (ordinal, row) in rows.iter().enumerate() {
            let row = row
                .as_object()
                .ok_or_else(|| format!("portable A27 {key} row {ordinal} is not an object"))?;
            let id = row
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("portable A27 {key} row {ordinal} has no id"))?;
            let runtime_type = row
                .get("type")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("portable A27 {key} row {ordinal} has no type"))?;
            if runtime_type != expected_type
                || row.get("catalog_scan_ordinal").and_then(Value::as_u64) != Some(ordinal as u64)
            {
                return Err(format!(
                    "portable A27 {key} row {ordinal} has invalid type/ordinal"
                ));
            }
            ids.push(format!("{id}:{runtime_type}"));
        }
    }
    Ok(ids)
}

fn exact_row_id(row: &Value, expected_type: u64) -> Result<String, String> {
    let id = row
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "A27 delta row has no id".to_string())?;
    let runtime_type = row
        .get("type")
        .and_then(Value::as_u64)
        .ok_or_else(|| "A27 delta row has no type".to_string())?;
    if runtime_type != expected_type {
        return Err("A27 delta row has the wrong runtime type".into());
    }
    Ok(format!("{id}:{runtime_type}"))
}

fn validate_temporal_deltas(catalog: &PortableDesignCatalog) -> Result<(), String> {
    let mut ids = exact_design_ids(&catalog.exact_native_catalog)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut native_rows = exact_rows(catalog, "division_templates")?;
    native_rows.extend(exact_rows(catalog, "variants")?);
    let mut previous_hour = catalog.capture_hour_index;
    for (offset, delta) in catalog.temporal_deltas.iter().enumerate() {
        if delta.sequence != (offset + 1) as u64
            || delta.capture_hour_index <= previous_hour
            || delta.source_record_id != format!("a27-live-catalog:{}", delta.source_sha256)
            || delta.source_sha256.len() != 64
            || !delta
                .source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || contains_relocation_field(&delta.native_frame_metadata)
        {
            return Err(format!(
                "portable A27 delta {} identity is invalid",
                delta.sequence
            ));
        }
        let authority = delta
            .native_frame_metadata
            .get("authority")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("portable A27 delta {} has no authority", delta.sequence))?;
        if authority.get("capture_hour_index").and_then(Value::as_u64)
            != Some(delta.capture_hour_index)
            || authority.get("coherence").and_then(Value::as_str)
                != Some("double_read_all_roots_fields_and_membership")
            || delta.native_frame_metadata.get("variants").is_some()
            || delta
                .native_frame_metadata
                .get("division_templates")
                .is_some()
        {
            return Err(format!(
                "portable A27 delta {} metadata is not an exact sealed frame",
                delta.sequence
            ));
        }
        let mut upsert_ids = BTreeSet::new();
        for row in &delta.variant_upserts {
            if !upsert_ids.insert(exact_row_id(row, 70)?) {
                return Err(format!(
                    "portable A27 delta {} duplicates an upsert",
                    delta.sequence
                ));
            }
        }
        for row in &delta.template_upserts {
            if !upsert_ids.insert(exact_row_id(row, 52)?) {
                return Err(format!(
                    "portable A27 delta {} duplicates an upsert",
                    delta.sequence
                ));
            }
        }
        if previous_hour.checked_add(1)!=Some(delta.capture_hour_index)
            && upsert_ids != delta.ordered_design_ids.iter().cloned().collect::<BTreeSet<_>>() {
            return Err("A27 gap recovery delta omits complete baseline definitions".into());
        }
        let mut tombstones = BTreeSet::new();
        for id in delta
            .variant_tombstones
            .iter()
            .chain(delta.template_tombstones.iter())
        {
            if !tombstones.insert(id.clone()) || upsert_ids.contains(id) || !ids.remove(id) {
                return Err(format!(
                    "portable A27 delta {} has an invalid tombstone",
                    delta.sequence
                ));
            }
            native_rows.remove(id);
        }
        for row in delta
            .variant_upserts
            .iter()
            .chain(delta.template_upserts.iter())
        {
            native_rows.insert(
                format!(
                    "{}:{}",
                    row.get("id").and_then(Value::as_u64).unwrap_or_default(),
                    row.get("type").and_then(Value::as_u64).unwrap_or_default()
                ),
                row.clone(),
            );
        }
        ids.extend(upsert_ids.iter().cloned());
        let record_upsert_ids = delta
            .record_upserts
            .iter()
            .map(|record| record.design_id.clone())
            .collect::<BTreeSet<_>>();
        if record_upsert_ids != upsert_ids
            || delta.record_upserts.len() != upsert_ids.len()
            || delta
                .ordered_design_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                != ids
            || delta.ordered_design_ids.len() != ids.len()
        {
            return Err(format!(
                "portable A27 delta {} projection/order diverges from native mutation",
                delta.sequence
            ));
        }
        let mut exact_frame = delta.native_frame_metadata.clone();
        let frame = exact_frame.as_object_mut().ok_or_else(|| {
            format!(
                "portable A27 delta {} metadata is not an object",
                delta.sequence
            )
        })?;
        let mut templates = Vec::new();
        let mut variants = Vec::new();
        for design_id in &delta.ordered_design_ids {
            let mut row = native_rows
                .get(design_id)
                .ok_or_else(|| {
                    format!(
                        "portable A27 delta {} order has no native row",
                        delta.sequence
                    )
                })?
                .clone();
            match row.get("type").and_then(Value::as_u64) {
                Some(52) => {
                    row["catalog_scan_ordinal"] = (templates.len() as u64).into();
                    templates.push(row);
                }
                Some(70) => {
                    row["catalog_scan_ordinal"] = (variants.len() as u64).into();
                    variants.push(row);
                }
                _ => {
                    return Err(format!(
                        "portable A27 delta {} native row type is invalid",
                        delta.sequence
                    ))
                }
            }
        }
        frame.insert("division_templates".into(), Value::Array(templates));
        frame.insert("variants".into(), Value::Array(variants));
        validate_exact_native_catalog(&exact_frame, delta.capture_hour_index)?;
        let exact_bytes = serde_json::to_vec(&exact_frame).map_err(|error| {
            format!(
                "serialize A27 delta {} exact frame: {error}",
                delta.sequence
            )
        })?;
        let projected = PortableDesignCatalog::decode_a27_live_json(&exact_bytes)?;
        let mut expected_record_upserts = projected
            .records
            .into_iter()
            .filter(|record| upsert_ids.contains(&record.design_id))
            .collect::<Vec<_>>();
        rebase_record_ids(&mut expected_record_upserts, &delta.source_record_id);
        if expected_record_upserts != delta.record_upserts {
            return Err(format!(
                "portable A27 delta {} record projection is not native-derived",
                delta.sequence
            ));
        }
        catalog.records_at_hour(delta.capture_hour_index)?;
        previous_hour = delta.capture_hour_index;
    }
    Ok(())
}

fn utf8_hex_matches(value: Option<&str>, encoded: &str) -> bool {
    value
        .unwrap_or("")
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        == encoded
}

fn validate_subunits(
    label: &str,
    pointer_count: u32,
    placeholders: &[u32],
    rows: &[RawSubunit],
) -> Result<(), String> {
    let mut ordinals = BTreeSet::new();
    for ordinal in placeholders {
        if *ordinal >= pointer_count || !ordinals.insert(*ordinal) {
            return Err(format!(
                "{label} has an invalid/duplicate placeholder ordinal"
            ));
        }
    }
    for row in rows {
        if row.source_ordinal >= pointer_count
            || !ordinals.insert(row.source_ordinal)
            || row.unit_type.trim().is_empty()
            || !utf8_hex_matches(Some(&row.unit_type), &row.type_utf8_hex)
        {
            return Err(format!("{label} has an invalid/duplicate subunit row"));
        }
        let _ = (row.x, row.y, row.subunit_definition_handle);
    }
    if ordinals.len() != pointer_count as usize {
        return Err(format!(
            "{label} does not account for every native pointer slot"
        ));
    }
    Ok(())
}

fn reject_designer_outcomes(value: &Value) -> Result<(), String> {
    if value.get("designer_controller_contexts").is_some()
        || value.get("variants").and_then(Value::as_array).is_some_and(|rows| rows.iter().any(|row|
            row.get("definition").is_some_and(|definition|
                definition.get("designer_classification").is_some() || definition.get("designer_row_visual").is_some())))
    {
        return Err("A27 contains precomputed viewer-owned designer outcomes".into());
    }
    Ok(())
}

fn validate_mission_stat_operands(rows: &[DesignMissionStatOperand]) -> Result<(), String> {
    fn refusal(reason: &DesignFieldRefusal, lookup: bool) -> Result<(), String> {
        if reason.field.trim().is_empty()
            || !(reason.kind == "unreadable" || lookup && reason.kind == "invalid_vector") {
            return Err("invalid operand refusal".into());
        }
        Ok(())
    }
    fn prefix(count: u32, keys: &[u32], target: u32, found: bool) -> Result<(), String> {
        if count > i32::MAX as u32 || keys.len() > 4096 || keys.len() > count as usize {
            return Err("operand prefix/count exceeds its bounded observation".into());
        }
        if found {
            if keys.last() != Some(&target) || keys[..keys.len()-1].contains(&target) {
                return Err("operand prefix must end at the first matching key".into());
            }
        } else if keys.len() != count as usize || keys.contains(&target) {
            return Err("operand absence requires a complete prefix without the target".into());
        }
        Ok(())
    }
    if rows.len() != 2 || (rows[0].group_key, rows[0].stat_key) != (0x1000, 0x23)
        || (rows[1].group_key, rows[1].stat_key) != (0x2000, 0x24) {
        return Err("expected the two ordered mission/stat identities".into());
    }
    let mut observed_groups: Option<(u32, &[u32])> = None;
    for row in rows {
        if let DesignI64Observation::Unavailable { reason } = &row.fallback {
            refusal(reason, false)?;
        }
        let (count, keys) = match &row.override_lookup {
            DesignMissionOverrideObservation::Measured { group_count, group_key_prefix, stat_count, stat_key_prefix, .. } => {
                prefix(*group_count, group_key_prefix, row.group_key, true)?;
                prefix(*stat_count, stat_key_prefix, row.stat_key, true)?;
                (*group_count, group_key_prefix.as_slice())
            }
            DesignMissionOverrideObservation::GroupAbsent { group_count, group_key_prefix } => {
                prefix(*group_count, group_key_prefix, row.group_key, false)?;
                (*group_count, group_key_prefix.as_slice())
            }
            DesignMissionOverrideObservation::StatAbsent { group_count, group_key_prefix, stat_count, stat_key_prefix } => {
                prefix(*group_count, group_key_prefix, row.group_key, true)?;
                prefix(*stat_count, stat_key_prefix, row.stat_key, false)?;
                (*group_count, group_key_prefix.as_slice())
            }
            DesignMissionOverrideObservation::Unavailable { reason } => {
                refusal(reason, true)?;
                continue;
            }
        };
        if let Some((previous_count, previous_keys)) = observed_groups {
            if previous_count != count || previous_keys.iter().zip(keys).any(|(left, right)| left != right) {
                return Err("mission operands disagree on the same ordered group table".into());
            }
        }
        observed_groups = Some((count, keys));
    }
    Ok(())
}

fn validate_operand_refusal(reason: &DesignFieldRefusal) -> Result<(), String> {
    if !matches!(reason.kind.as_str(), "unreadable" | "torn" | "invalid_identity" | "bound_exceeded")
        || reason.field.is_empty() || reason.field.len() > 160 {
        return Err("A27 independent operand has an invalid refusal".into());
    }
    Ok(())
}

fn measured_slot_key(text: &DesignTextObservation) -> Result<&str, String> {
    let DesignTextObservation::Measured { value, utf8_hex } = text else {
        return Err("A27 measured slot census has unavailable text".into());
    };
    if value.is_empty() || value.len() > MAX_DESIGN_SLOT_TOKEN_BYTES
        || !utf8_hex_matches(Some(value), utf8_hex)
        || !value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':')) {
        return Err("A27 slot census has an invalid token identity".into());
    }
    Ok(value)
}

fn validate_slot_identity_observation(observation: &DesignSlotIdentityObservation) -> Result<(), String> {
    match observation {
        DesignSlotIdentityObservation::Unavailable { reason } => {
            if !matches!(reason.kind.as_str(),
                "unreadable" | "torn" | "invalid_identity" | "missing_typed_archetype" |
                "invalid_vector" | "row_bound" | "lexer_unavailable" | "invalid_token_id" |
                "invalid_string" | "text_bound" | "invalid_utf8" | "invalid_token_text" |
                "duplicate_identity" | "read_budget" | "allocation_budget" | "trace_bound" |
                "allocation_failed" | "address" | "closed")
                || reason.field.is_empty() || reason.field.len() > 160 {
                return Err("A27 slot census has an invalid refusal".into());
            }
        }
        DesignSlotIdentityObservation::Measured { archetype_token_raw, archetype_token, slots } => {
            if *archetype_token_raw == 0 || *archetype_token_raw > i32::MAX as u32 {
                return Err("A27 slot census has an invalid archetype key".into());
            }
            measured_slot_key(archetype_token)?;
            if slots.len() > MAX_DESIGN_SLOT_IDENTITIES {
                return Err("A27 copied slot census exceeds the bound".into());
            }
            let mut handles = BTreeSet::new();
            let mut keys = BTreeSet::new();
            for (ordinal, row) in slots.iter().enumerate() {
                if row.source_ordinal as usize != ordinal || row.token_handle_raw == 0
                    || row.token_handle_raw > i32::MAX as u32 || !handles.insert(row.token_handle_raw)
                    || !keys.insert(measured_slot_key(&row.token)?.to_ascii_lowercase()) {
                    return Err("A27 copied slot order/identity is inconsistent".into());
                }
            }
        }
    }
    Ok(())
}

fn validate_raw_catalog(raw: &RawCatalog) -> Result<(), String> {
    if let DesignU32Observation::Unavailable { reason } = &raw.runtime_dlc_mask {
        if reason.kind != "unreadable" || reason.field.is_empty() {
            return Err("A27 runtime_dlc_mask has an invalid refusal".into());
        }
    }
    if raw.authority.program != EXACT_A729_PROGRAM
        || raw.authority.program_sha256 != EXACT_A729_SHA256
        || raw.authority.coherence != "double_read_all_roots_fields_and_membership"
        || raw.country_count != raw.countries.len() as u64
    {
        return Err("A27 live catalog lacks a whole-sweep seal/country census".into());
    }
    let mut country_tags = BTreeSet::new();
    let mut developed_slots = BTreeSet::new();
    for (ordinal, country) in raw.countries.iter().enumerate() {
        if country.registry_index != ordinal as u64
            || country.tag.trim().is_empty()
            || !country_tags.insert(country.tag.as_str())

        {
            return Err("A27 live catalog has an invalid country registry".into());
        }
    }
    if raw.completeness.owner != raw.division_templates.iter().all(|row|row.owner_tag.is_some()) {
        return Err("A27 owner completeness disagrees with measured template membership".into());
    }
    let mut template_slots = BTreeSet::new();
    for row in &raw.division_templates {
        for input in [&row.definition.country_289a, &row.definition.country_3abe] {
            if let DesignI32Observation::Unavailable { reason } = input {
                validate_operand_refusal(reason)?;
            }
        }
        {
            let sources = &row.country_source_occurrences;
            for source in sources {
                let country = raw.countries.get(source.source_country_registry_index as usize)
                    .ok_or("A27 template membership has an invalid country")?;
                if country.tag != source.source_country_tag
                    || !template_slots.insert((source.source_country_registry_index, source.country_vector_ordinal)) {
                    return Err("A27 template membership has an invalid/duplicate country slot".into());
                }
            }
            let sole = if sources.len()==1 {sources.first()} else {None};
            if row.owner_tag.as_deref()!=sole.map(|source|source.source_country_tag.as_str())
                || row.source_country_tag.as_deref()!=sole.map(|source|source.source_country_tag.as_str())
                || row.source_country_registry_index!=sole.map(|source|source.source_country_registry_index)
                || row.country_vector_ordinal!=sole.map(|source|source.country_vector_ordinal) {
                return Err("A27 template owner projection differs from country membership".into());
            }
        }
        if row.owner_tag != row.source_country_tag
            || row.definition.obsolete != row.obsolete
            || !utf8_hex_matches(
                row.definition.name.as_deref(),
                &row.definition.name_utf8_hex,
            )
            || row.definition.regiment_grid_columns == 0
            || row.definition.regiment_grid_rows == 0
        {
            return Err(format!(
                "A27 template {} has inconsistent native fields",
                row.id
            ));
        }
        validate_subunits(
            &format!("A27 template {} regiments", row.id),
            row.definition.regiment_pointer_count,
            &row.definition.regiment_placeholder_ordinals,
            &row.definition.regiments,
        )?;
        validate_subunits(
            &format!("A27 template {} support", row.id),
            row.definition.support_pointer_count,
            &row.definition.support_placeholder_ordinals,
            &row.definition.support,
        )?;
        validate_subunits(
            &format!("A27 template {} regimental support", row.id),
            row.definition.regimental_support_pointer_count,
            &row.definition.regimental_support_placeholder_ordinals,
            &row.definition.regimental_support,
        )?;
        let _ = (
            row.country_vector_ordinal,
            row.definition.priority,
            row.definition.lock_regime_snapshot_raw,
            row.definition.is_locked,
        );
    }
    for row in &raw.variants {
        if let DesignByteObservation::Unavailable { reason } = &row.definition.append_variant_name {
            validate_operand_refusal(reason)?;
        }
        validate_slot_identity_observation(&row.definition.slot_identities)?;

        if row.creator_tag != row.owner_tag
            || row.selected_country_tag
                != row
                    .origin_override_tag
                    .as_deref()
                    .unwrap_or(row.creator_tag.as_str())
            || !utf8_hex_matches(
                Some(&row.definition.archetype_token),
                &row.definition.archetype_token_utf8_hex,
            )
            || !utf8_hex_matches(
                row.definition.display_name.as_deref(),
                &row.definition.display_name_utf8_hex,
            )
            || !utf8_hex_matches(
                row.definition.variant_name.as_deref(),
                &row.definition.variant_name_utf8_hex,
            )
            || (row.source_occurrences.is_empty() != row.first_ui_discovery_ordinal.is_none())
        {
            return Err(format!(
                "A27 variant {} has inconsistent native fields",
                row.id
            ));
        }
        validate_mission_stat_operands(&row.definition.mission_stat_operands)
            .map_err(|reason| format!("A27 variant {} mission_stat_operands: {reason}", row.id))?;
        for (ordinal, module) in row.definition.modules.iter().enumerate() {
            if module.source_ordinal != ordinal as u32
                || module.module_token.trim().is_empty()
                || !utf8_hex_matches(Some(&module.module_token), &module.module_token_utf8_hex)
            {
                return Err(format!("A27 variant {} has an invalid module row", row.id));
            }
            let _ = module.module_definition_handle;
        }
        for (ordinal, upgrade) in row.definition.upgrades.iter().enumerate() {
            if upgrade.source_ordinal != ordinal as u32
                || upgrade.level_raw_u64 & 0xff != upgrade.level as u64
            {
                return Err(format!("A27 variant {} has an invalid upgrade row", row.id));
            }
        }
        for (ordinal, stat) in row.definition.cached_stats.iter().enumerate() {
            if stat.source_ordinal != ordinal as u32
                || stat.stat_id.trim().is_empty()
                || stat.scale <= 0
            {
                return Err(format!(
                    "A27 variant {} has an invalid cached-stat row",
                    row.id
                ));
            }
        }
        for occurrence in &row.source_occurrences {
            let country = raw
                .countries
                .get(occurrence.source_country_registry_index as usize)
                .ok_or_else(|| format!("A27 variant {} has an invalid UI country", row.id))?;
            if country.tag != occurrence.source_country_tag
                || !matches!(occurrence.source_list.as_str(), "A" | "B")
            {
                return Err(format!(
                    "A27 variant {} has an invalid UI occurrence",
                    row.id
                ));
            }
            let _ = occurrence.source_list_ordinal;
        }
        {
            let entity = &row.definition.explicit_entity;
            match entity {
                DesignTextObservation::Measured { value, utf8_hex } => {
                    if value.len() > 4096 || !utf8_hex_matches(Some(value), utf8_hex) {
                        return Err(format!("A27 variant {} has an invalid explicit entity", row.id));
                    }
                }
                DesignTextObservation::Unavailable { reason } => {
                    if !matches!(reason.kind.as_str(), "unreadable" | "invalid_string")
                        || reason.field.is_empty() {
                        return Err(format!("A27 variant {} has an invalid entity refusal", row.id));
                    }
                }
            }
        }
        if let DesignByteObservation::Unavailable { reason } = &row.definition.serialized_variant_flag_2b9d {
            if reason.kind != "unreadable" || reason.field.is_empty() {
                return Err(format!("A27 variant {} has an invalid serialized_variant_flag_2b9d refusal", row.id));
            }
        }
        match &row.definition.item_icon_override {
            DesignItemIconObservation::Measured { kind, token, .. } => {
                if !matches!(kind, 0 | 1) {
                    return Err(format!("A27 variant {} has an invalid item icon kind", row.id));
                }
                match token {
                    DesignTextObservation::Measured { value, utf8_hex } => {
                        if value.len() > 4096 || !utf8_hex_matches(Some(value), utf8_hex) {
                            return Err(format!("A27 variant {} has an invalid item icon token", row.id));
                        }
                    }
                    DesignTextObservation::Unavailable { reason } => {
                        if !matches!(reason.kind.as_str(), "unreadable" | "invalid_string")
                            || reason.field.is_empty() {
                            return Err(format!("A27 variant {} has an invalid item icon token refusal", row.id));
                        }
                    }
                }
            }
            DesignItemIconObservation::Unavailable { reason } => {
                if !matches!(reason.kind.as_str(), "unreadable" | "invalid_identity")
                    || reason.field.is_empty() {
                    return Err(format!("A27 variant {} has an invalid item icon header refusal", row.id));
                }
            }
        }
        {
            let occurrences = &row.developed_source_occurrences;
            for occurrence in occurrences {
                let country = raw.countries.get(occurrence.source_country_registry_index as usize)
                    .ok_or("A27 developed membership country is outside the registry")?;
                if occurrence.source_country_tag != country.tag
                    || occurrence.source_list != "developed"
                    || !developed_slots.insert((occurrence.source_country_registry_index, occurrence.source_list_ordinal)) {
                    return Err("A27 developed membership has an invalid or duplicate source slot".into());
                }
            }
        }
        let _ = row.definition.equipment_token_raw;
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct PortableArtifact {
    schema: String,
    source_schema: String,
    source_record_id: String,
    source_sha256: String,
    capture_hour_index: u64,
    exact_native_catalog: Value,
    #[serde(default)]
    temporal_deltas: Vec<PortableDesignCatalogDelta>,
    gaps: Vec<DesignCatalogGap>,
    records: Vec<DesignRecord>,
}

#[derive(Deserialize)]
struct RawCatalog {
    runtime_dlc_mask: DesignU32Observation,
    schema: String,
    authority: RawAuthority,
    full_replacement: bool,
    completeness: RawCompleteness,
    country_count: u64,
    countries: Vec<RawCountry>,
    division_templates: Vec<RawDivisionTemplate>,
    variants: Vec<RawVariant>,
}

#[derive(Deserialize)]
struct RawAuthority {
    program: String,
    program_sha256: String,
    current_date_raw: u64,
    capture_hour_index: u64,
    coherence: String,
}

#[derive(Deserialize)]
struct RawCountry {
    registry_index: u64,
    tag: String,
    designer_pointer_order_ordinal: Option<u64>,
}

#[derive(Deserialize)]
struct RawCompleteness {
    definitions: bool,
    lifecycle: bool,
    membership: bool,
    owner: bool,
    ui_occurrences: bool,
}

#[derive(Deserialize)]
struct RawDivisionTemplate {
    catalog_scan_ordinal: u64,
    country_vector_ordinal: Option<u64>,
    definition: RawDivisionDefinition,
    definition_complete: bool,
    id: u32,
    obsolete: bool,
    owner_tag: Option<String>,
    source_country_registry_index: Option<u64>,
    source_country_tag: Option<String>,
    country_source_occurrences: Vec<TemplateSourceOccurrence>,
    #[serde(rename = "type")]
    runtime_type: u32,
}

#[derive(Deserialize)]
struct TemplateSourceOccurrence {
    source_country_tag: String,
    source_country_registry_index: u64,
    country_vector_ordinal: u64,
}

#[derive(Deserialize)]
struct RawDivisionDefinition {
    country_289a: DesignI32Observation,
    country_3abe: DesignI32Observation,
    name: Option<String>,
    name_utf8_hex: String,
    regiment_grid_columns: u32,
    regiment_grid_rows: u32,
    regiment_pointer_count: u32,
    regiment_placeholder_ordinals: Vec<u32>,
    regiments: Vec<RawSubunit>,
    support_pointer_count: u32,
    support_placeholder_ordinals: Vec<u32>,
    support: Vec<RawSubunit>,
    regimental_support_pointer_count: u32,
    regimental_support_placeholder_ordinals: Vec<u32>,
    regimental_support: Vec<RawSubunit>,
    priority: u32,
    lock_regime_snapshot_raw: u8,
    is_locked: bool,
    obsolete: bool,
}

#[derive(Deserialize)]
struct RawSubunit {
    source_ordinal: u32,
    x: u32,
    y: u32,
    subunit_definition_handle: u32,
    #[serde(rename = "type")]
    unit_type: String,
    type_utf8_hex: String,
}

#[derive(Deserialize)]
struct RawVariant {
    catalog_scan_ordinal: u64,
    creator_tag: String,
    definition: RawVariantDefinition,
    definition_complete: bool,
    first_ui_discovery_ordinal: Option<u64>,
    id: u32,
    obsolete: bool,
    origin_override_tag: Option<String>,
    owner_tag: String,
    selected_country_tag: String,
    source_occurrences: Vec<DesignSourceOccurrence>,
    developed_source_occurrences: Vec<DesignSourceOccurrence>,
    #[serde(rename = "type")]
    runtime_type: u32,
}

impl RawVariant {
    fn model_inputs(&self) -> VariantModelInputs {
        VariantModelInputs {
            creator_tag: self.creator_tag.clone(),
            origin_override_tag: self.origin_override_tag.clone(),
            explicit_entity: self.definition.explicit_entity.clone(),
            owned: self.source_occurrences.iter()
                .filter(|occurrence| occurrence.source_list == "A").cloned().collect(),
            developed: self.developed_source_occurrences.clone(),
        }
    }
}

#[derive(Deserialize)]
struct RawVariantDefinition {
    slot_identities: DesignSlotIdentityObservation,
    append_variant_name: DesignByteObservation,
    explicit_entity: DesignTextObservation,
    item_icon_override: DesignItemIconObservation,
    serialized_variant_flag_2b9d: DesignByteObservation,
    archetype_token: String,
    archetype_token_utf8_hex: String,
    cached_stats: Vec<DesignFixedStat>,
    mission_stat_operands: Vec<DesignMissionStatOperand>,
    cached_stats_complete: bool,
    #[serde(deserialize_with = "required_nullable_text")]
    display_name: Option<String>,
    display_name_utf8_hex: String,
    equipment_token_raw: u32,
    modules: Vec<RawModule>,
    modules_complete: bool,
    upgrades: Vec<RawUpgrade>,
    upgrades_complete: bool,
    #[serde(deserialize_with = "required_nullable_text")]
    variant_name: Option<String>,
    variant_name_utf8_hex: String,
}

// Explicit null is the producer's measured-empty representation. Requiring
// the field prevents an omitted Option from silently becoming measured empty.
fn required_nullable_text<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

#[derive(Deserialize)]
struct RawModule {
    module_definition_handle: u32,
    module_token: String,
    module_token_utf8_hex: String,
    slot_handle: u32,
    source_ordinal: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignFixedStat {
    pub raw_i64: i64,
    pub scale: i64,
    pub source_ordinal: u32,
    pub stat_id: String,
}

#[derive(Deserialize)]
struct RawUpgrade {
    level: i64,
    level_raw_u64: u64,
    source_ordinal: u32,
    upgrade_handle: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema": A27_LIVE_CATALOG_SCHEMA,
            "authority": {
                "program": EXACT_A729_PROGRAM,
                "program_sha256": EXACT_A729_SHA256,
                "current_date_raw": DATE_EPOCH_RAW + 900,
                "capture_hour_index": 900,
                "coherence": "double_read_all_roots_fields_and_membership"
            },
            "full_replacement": true,
            "completeness": {
                "definitions": true,
                "lifecycle": true,
                "membership": true,
                "owner": true,
                "ui_occurrences": true
            },
            "runtime_dlc_mask": {"status":"measured","raw_u32":0},
            "country_count": 1,
            "countries": [{ "registry_index": 0, "tag": "GER" }],
            "division_templates": [{
                "catalog_scan_ordinal": 0,
                "country_vector_ordinal": 0,
                "country_source_occurrences": [{"source_country_tag":"GER","source_country_registry_index":0,"country_vector_ordinal":0}],
                "definition": {
                    "country_289a": {"status":"measured","raw_i32":-1},
                    "country_3abe": {"status":"measured","raw_i32":0},
                    "name": "Infantry",
                    "name_utf8_hex": "496e66616e747279",
                    "regiment_grid_columns": 5,
                    "regiment_grid_rows": 5,
                    "regiment_pointer_count": 0,
                    "regiment_placeholder_ordinals": [],
                    "regiments": [],
                    "support_pointer_count": 0,
                    "support_placeholder_ordinals": [],
                    "support": [],
                    "regimental_support_pointer_count": 0,
                    "regimental_support_placeholder_ordinals": [],
                    "regimental_support": [],
                    "priority": 0,
                    "lock_regime_snapshot_raw": 0,
                    "is_locked": false,
                    "obsolete": false
                },
                "definition_complete": true,
                "id": 8,
                "obsolete": false,
                "owner_tag": "GER",
                "source_country_registry_index": 0,
                "source_country_tag": "GER",
                "type": 52
            }],
            "variants": [{
                "catalog_scan_ordinal": 0,
                "creator_tag": "GER",
                "definition": {
                    "append_variant_name": {"status":"measured","raw_u8":0},
                    "slot_identities": {"status":"measured","archetype_token_raw":71,
                        "archetype_token":{"status":"measured","value":"light_tank_chassis_1",
                            "utf8_hex":"6c696768745f74616e6b5f636861737369735f31"},
                        "slots":[{"source_ordinal":0,"token_handle_raw":91,
                            "token":{"status":"measured","value":"engine","utf8_hex":"656e67696e65"}}]},
                    "archetype_token": "light_tank_chassis_1",
                    "archetype_token_utf8_hex": "6c696768745f74616e6b5f636861737369735f31",
                    "mission_stat_operands": [
                        {"group_key":4096,"stat_key":35,"fallback":{"status":"measured","raw_i64":0},
                         "override_lookup":{"status":"group_absent","group_count":0,"group_key_prefix":[]}},
                        {"group_key":8192,"stat_key":36,"fallback":{"status":"measured","raw_i64":0},
                         "override_lookup":{"status":"group_absent","group_count":0,"group_key_prefix":[]}}
                    ],
                    "cached_stats": [{
                        "raw_i64": 250000,
                        "scale": 100000,
                        "source_ordinal": 0,
                        "stat_id": "armor"
                    }],
                    "cached_stats_complete": true,
                    "explicit_entity": {"status":"measured","value":"","utf8_hex":""},
                    "serialized_variant_flag_2b9d": {"status":"measured","raw_u8":0},
                    "item_icon_override": {"status":"measured","active":false,"kind":1,"token":{"status":"measured","value":"","utf8_hex":""}},
                    "display_name": "Panzer",
                    "display_name_utf8_hex": "50616e7a6572",
                    "equipment_token_raw": 71,
                    "modules": [{
                        "module_definition_handle": 92,
                        "module_token": "tank_gasoline_engine",
                        "module_token_utf8_hex": "74616e6b5f6761736f6c696e655f656e67696e65",
                        "slot_handle": 91,
                        "source_ordinal": 0
                    }],
                    "modules_complete": true,
                    "upgrades": [{
                        "level": 2,
                        "level_raw_u64": 2,
                        "source_ordinal": 0,
                        "upgrade_handle": 44
                    }],
                    "upgrades_complete": true,
                    "variant_name": null,
                    "variant_name_utf8_hex": ""
                },
                "definition_complete": true,
                "developed_source_occurrences": [],
                "first_ui_discovery_ordinal": null,
                "id": 7,
                "obsolete": false,
                "origin_override_tag": null,
                "owner_tag": "GER",
                "selected_country_tag": "GER",
                "source_occurrences": [],
                "type": 70
            }]
        }))
        .unwrap()
    }

    #[test]
    fn variant_model_inputs_preserve_creator_and_independent_origin_override() {
        let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
        raw["country_count"] = 2.into();
        raw["countries"].as_array_mut().unwrap().push(
            serde_json::json!({"registry_index": 1, "tag": "ITA"}),
        );
        for origin in [Some("ITA"), None] {
            raw["variants"][0]["origin_override_tag"] = serde_json::json!(origin);
            raw["variants"][0]["selected_country_tag"] = origin.unwrap_or("GER").into();
            let catalog = PortableDesignCatalog::decode_a27_live_json(
                &serde_json::to_vec(&raw).unwrap(),
            ).unwrap();
            let inputs = catalog.variant_model_inputs_at_hour(900, 7).unwrap();
            assert_eq!(inputs.creator_tag, "GER");
            assert_eq!(inputs.origin_override_tag.as_deref(), origin);
            let records = catalog.records_at_hour(900).unwrap();
            let variant = records.iter().find(|row| row.design_id == "7:70").unwrap();
            assert_eq!(variant.owner.as_deref(), Some("GER"));
        }
    }

    #[test]
    fn composition_preserves_native_placeholders_and_certifies_source_coordinates() {
        let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
        let definition = &mut raw["division_templates"][0]["definition"];
        definition["regiment_pointer_count"] = 8.into();
        definition["regiment_placeholder_ordinals"] = serde_json::json!([0, 1, 2, 3, 4, 5, 6]);
        definition["regiments"] = serde_json::json!([{
            "source_ordinal": 7, "x": 1, "y": 2,
            "subunit_definition_handle": 101, "type": "infantry",
            "type_utf8_hex": "696e66616e747279"
        }]);
        let decode = |raw: &Value| {
            PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(raw).unwrap()).unwrap()
        };
        let catalog = decode(&raw);
        let projected = catalog.division_composition_at_hour(900).unwrap();
        assert_eq!(projected["8:52"].regiments.len(), 1);
        assert_eq!(
            (
                projected["8:52"].regiments[0].x,
                projected["8:52"].regiments[0].y
            ),
            (1, 2)
        );
        assert!(projected["8:52"].support.is_empty());
        assert!(projected["8:52"].regimental_support.is_empty());
        let mut reordered = raw.clone();
        let definition = &mut reordered["division_templates"][0]["definition"];
        let mut zero_cell = definition["regiments"][0].clone();
        zero_cell["source_ordinal"] = 0.into();
        zero_cell["x"] = 0.into();
        zero_cell["y"] = 0.into();
        definition["regiments"]
            .as_array_mut()
            .unwrap()
            .push(zero_cell);
        definition["regiment_placeholder_ordinals"] = serde_json::json!([1, 2, 3, 4, 5, 6]);
        let reordered = decode(&reordered)
            .division_composition_at_hour(900)
            .unwrap();
        assert_eq!(
            reordered["8:52"]
                .regiments
                .iter()
                .map(|row| (row.x, row.y))
                .collect::<Vec<_>>(),
            [(1, 2), (0, 0)]
        );
        for (field, value) in [("x", 0), ("y", 1)] {
            let mut invalid = raw.clone();
            invalid["division_templates"][0]["definition"]["regiments"][0][field] = value.into();
            assert!(decode(&invalid)
                .division_composition_at_hour(900)
                .unwrap_err()
                .contains("coordinate differs"));
        }
        let mut invalid = raw.clone();
        invalid["division_templates"][0]["definition"]["regiment_grid_rows"] = 32768.into();
        assert!(decode(&invalid)
            .division_composition_at_hour(900)
            .unwrap_err()
            .contains("signed-short"));
        for vector in ["support", "regimental_support"] {
            let mut invalid = raw.clone();
            let definition = &mut invalid["division_templates"][0]["definition"];
            definition[format!("{vector}_pointer_count")] = 1.into();
            let mut row = definition["regiments"][0].clone();
            row["source_ordinal"] = 0.into();
            definition[vector] = serde_json::json!([row]);
            assert!(decode(&invalid)
                .division_composition_at_hour(900)
                .unwrap_err()
                .contains("coordinate differs"));
        }
        let mut empty = raw.clone();
        empty["division_templates"][0]["definition"]["regiment_placeholder_ordinals"] =
            serde_json::json!([0, 1, 2, 3, 4, 5, 6, 7]);
        empty["division_templates"][0]["definition"]["regiments"] = serde_json::json!([]);
        assert!(
            decode(&empty).division_composition_at_hour(900).unwrap()["8:52"]
                .regiments
                .is_empty()
        );
        for unavailable in [false, true] {
            let mut invalid = raw.clone();
            let definition = invalid["division_templates"][0]["definition"]
                .as_object_mut()
                .unwrap();
            if unavailable {
                definition.insert("regiments".into(), Value::Null);
            } else {
                definition.remove("regiments");
            }
            assert!(PortableDesignCatalog::decode_a27_live_json(
                &serde_json::to_vec(&invalid).unwrap()
            )
            .is_err());
        }
    }

    #[test]
    fn native_catalog_refuses_replaced_schema_and_precomputed_ui_outcomes() {
        let base: Value = serde_json::from_slice(&fixture()).unwrap();
        let mut old = base.clone();
        old["schema"] = "extractor.a27.native_designer_catalog.a729.v5".into();
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&old).unwrap()).unwrap_err().contains("schema"));
        for field in ["designer_classification", "designer_row_visual"] {
            for value in [Value::Null, serde_json::json!({"measured":true})] {
                let mut invalid = base.clone();
                invalid["variants"][0]["definition"][field] = value;
                assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&invalid).unwrap()).unwrap_err().contains("viewer-owned"));
            }
        }
        let mut invalid = base;
        invalid["designer_controller_contexts"] = serde_json::json!([]);
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&invalid).unwrap()).unwrap_err().contains("viewer-owned"));
    }

    #[test]
    fn a27_catalog_decodes_pointer_free_exact_records() {
        let bytes = fixture();
        let catalog = PortableDesignCatalog::decode_a27_live_json(&bytes).unwrap();
        assert_eq!(catalog.records.len(), 2);
        let variant = catalog.get("7:70").unwrap();
        assert_eq!(variant.kind, DesignKind::EquipmentVariant);
        assert_eq!(variant.modules[0].slot_id, "handle:91");
        assert_eq!(variant.stats[0].value, 2.5);
        let encoded = serde_json::to_string(&catalog).unwrap();
        for forbidden in [
            "pointer_identity",
            "equipment_definition_pointer",
            "subunit_definition_pointer",
            "module_base",
            "game_state",
            "\"pid\"",
        ] {
            assert!(!encoded.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn a27_catalog_rejects_partial_or_wrong_runtime_type() {
        let mut value: serde_json::Value = serde_json::from_slice(&fixture()).unwrap();
        value["completeness"]["lifecycle"] = false.into();
        assert!(
            PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&value).unwrap())
                .is_err()
        );
        value["completeness"]["lifecycle"] = true.into();
        value["variants"][0]["type"] = 59.into();
        assert!(
            PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&value).unwrap())
                .is_err()
        );
    }

    #[test]
    fn a27_timeline_reconstructs_exact_hour_tombstones() {
        let first: Value = serde_json::from_slice(&fixture()).unwrap();
        let mut second = first.clone();
        second["authority"]["current_date_raw"] = (DATE_EPOCH_RAW + 901).into();
        second["authority"]["capture_hour_index"] = 901_u64.into();
        second["variants"] = serde_json::json!([]);
        let first_bytes = serde_json::to_vec(&canonical_json_value(&first)).unwrap();
        let second_bytes = serde_json::to_vec(&canonical_json_value(&second)).unwrap();
        let mut second_metadata = second.clone();
        second_metadata.as_object_mut().unwrap().remove("variants");
        second_metadata
            .as_object_mut()
            .unwrap()
            .remove("division_templates");
        let timeline = serde_json::to_vec(&serde_json::json!({
            "schema": A27_LIVE_CATALOG_TIMELINE_SCHEMA,
            "authority": {
                "program": "hoi4.exe 1.19.2.0.a729",
                "program_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
            },
            "complete_sequence": true,
            "transport": "first_full_then_native_row_deltas",
            "capture_count": 2,
            "same_hour_recheck_count": 2,
            "first_capture_hour_index": 900,
            "last_capture_hour_index": 901,
            "captures": [
                {
                    "sequence": 0,
                    "capture_hour_index": 900,
                    "same_hour_rechecks": 1,
                    "same_hour_recheck_sha256": [Sha256::hex(&first_bytes)],
                    "snapshot_sha256": Sha256::hex(&first_bytes),
                    "snapshot": first
                },
                {
                    "sequence": 1,
                    "capture_hour_index": 901,
                    "same_hour_rechecks": 1,
                    "same_hour_recheck_sha256": [Sha256::hex(&second_bytes)],
                    "snapshot_sha256": Sha256::hex(&second_bytes),
                    "delta": {
                        "frame_metadata": second_metadata,
                        "variant_upserts": [],
                        "variant_tombstones": ["7:70"],
                        "template_upserts": [],
                        "template_tombstones": [],
                        "ordered_design_ids": ["8:52"]
                    }
                }
            ]
        }))
        .unwrap();
        let catalog = PortableDesignCatalog::decode_a27_live_json(&timeline).unwrap();
        assert!(catalog.validate_replay_hours(&[900, 901]).is_ok());
        assert!(catalog.validate_replay_hours(&[900]).is_err());
        assert_eq!(catalog.records_at_hour(900).unwrap().len(), 2);
        assert_eq!(catalog.records_at_hour(901).unwrap().len(), 1);
        assert_eq!(catalog.temporal_deltas[0].variant_tombstones, ["7:70"]);
        let portable = catalog.encode_portable_json().unwrap();
        let relocated = PortableDesignCatalog::decode_a27_live_json(&portable).unwrap();
        assert_eq!(relocated.records_at_hour(901).unwrap()[0].design_id, "8:52");
    }

    #[test]
    fn division_composition_projects_baseline_upsert_tombstone_and_all_vectors() {
        let mut first: Value = serde_json::from_slice(&fixture()).unwrap();
        let definition = &mut first["division_templates"][0]["definition"];
        definition["regiment_pointer_count"] = 8.into();
        definition["regiment_placeholder_ordinals"] = serde_json::json!([0,1,2,3,4,5,6]);
        definition["regiments"] = serde_json::json!([{
            "source_ordinal": 7,
            "x": 1,
            "y": 2,
            "subunit_definition_handle": 101,
            "type": "infantry",
            "type_utf8_hex": "696e66616e747279"
        }]);
        definition["support_pointer_count"] = 1.into();
        definition["support"] = serde_json::json!([{
            "source_ordinal": 0,
            "x": 0,
            "y": 0,
            "subunit_definition_handle": 102,
            "type": "support_artillery",
            "type_utf8_hex": "737570706f72745f617274696c6c657279"
        }]);
        definition["regimental_support_pointer_count"] = 1.into();
        definition["regimental_support"] = serde_json::json!([{
            "source_ordinal": 0,
            "x": 0,
            "y": 0,
            "subunit_definition_handle": 103,
            "type": "signal_company",
            "type_utf8_hex": "7369676e616c5f636f6d70616e79"
        }]);

        let mut second = first.clone();
        second["authority"]["current_date_raw"] = (DATE_EPOCH_RAW + 901).into();
        second["authority"]["capture_hour_index"] = 901_u64.into();
        second["division_templates"][0]["definition"]["regiment_grid_columns"] = 6.into();
        second["division_templates"][0]["definition"]["regiments"][0]["x"] = 4.into();
        second["division_templates"][0]["definition"]["regiments"][0]["source_ordinal"] = 22.into();
        second["division_templates"][0]["definition"]["regiment_pointer_count"] = 23.into();
        second["division_templates"][0]["definition"]["regiment_placeholder_ordinals"] = serde_json::json!((0..22).collect::<Vec<_>>());
        second["division_templates"][0]["definition"]["regiments"][0]["type"] = "motorized".into();
        second["division_templates"][0]["definition"]["regiments"][0]["type_utf8_hex"] =
            "6d6f746f72697a6564".into();

        let mut third = second.clone();
        third["authority"]["current_date_raw"] = (DATE_EPOCH_RAW + 902).into();
        third["authority"]["capture_hour_index"] = 902_u64.into();
        third["division_templates"] = serde_json::json!([]);

        let first_bytes = serde_json::to_vec(&canonical_json_value(&first)).unwrap();
        let second_bytes = serde_json::to_vec(&canonical_json_value(&second)).unwrap();
        let third_bytes = serde_json::to_vec(&canonical_json_value(&third)).unwrap();
        let mut second_metadata = second.clone();
        second_metadata.as_object_mut().unwrap().remove("variants");
        second_metadata
            .as_object_mut()
            .unwrap()
            .remove("division_templates");
        let mut third_metadata = third.clone();
        third_metadata.as_object_mut().unwrap().remove("variants");
        third_metadata
            .as_object_mut()
            .unwrap()
            .remove("division_templates");

        let timeline = serde_json::to_vec(&serde_json::json!({
            "schema": A27_LIVE_CATALOG_TIMELINE_SCHEMA,
            "authority": {
                "program": EXACT_A729_PROGRAM,
                "program_sha256": EXACT_A729_SHA256
            },
            "complete_sequence": true,
            "transport": "first_full_then_native_row_deltas",
            "capture_count": 3,
            "same_hour_recheck_count": 3,
            "first_capture_hour_index": 900,
            "last_capture_hour_index": 902,
            "captures": [
                {
                    "sequence": 0,
                    "capture_hour_index": 900,
                    "same_hour_rechecks": 1,
                    "same_hour_recheck_sha256": [Sha256::hex(&first_bytes)],
                    "snapshot_sha256": Sha256::hex(&first_bytes),
                    "snapshot": first
                },
                {
                    "sequence": 1,
                    "capture_hour_index": 901,
                    "same_hour_rechecks": 1,
                    "same_hour_recheck_sha256": [Sha256::hex(&second_bytes)],
                    "snapshot_sha256": Sha256::hex(&second_bytes),
                    "delta": {
                        "frame_metadata": second_metadata,
                        "variant_upserts": [],
                        "variant_tombstones": [],
                        "template_upserts": [second["division_templates"][0].clone()],
                        "template_tombstones": [],
                        "ordered_design_ids": ["8:52", "7:70"]
                    }
                },
                {
                    "sequence": 2,
                    "capture_hour_index": 902,
                    "same_hour_rechecks": 1,
                    "same_hour_recheck_sha256": [Sha256::hex(&third_bytes)],
                    "snapshot_sha256": Sha256::hex(&third_bytes),
                    "delta": {
                        "frame_metadata": third_metadata,
                        "variant_upserts": [],
                        "variant_tombstones": [],
                        "template_upserts": [],
                        "template_tombstones": ["8:52"],
                        "ordered_design_ids": ["7:70"]
                    }
                }
            ]
        }))
        .unwrap();
        let catalog = PortableDesignCatalog::decode_a27_live_json(&timeline).unwrap();

        let baseline = catalog.division_composition_at_hour(900).unwrap();
        let baseline = baseline.get("8:52").unwrap();
        assert_eq!(
            (baseline.regiment_grid_columns, baseline.regiment_grid_rows),
            (5, 5)
        );
        assert_eq!(
            baseline.regiments[0],
            DivisionSubunit {
                x: 1,
                y: 2,
                unit_type: "infantry".into(),
                source_vector: DivisionSubunitVector::Regiment,
            }
        );
        assert_eq!(
            baseline.support[0],
            DivisionSubunit {
                x: 0,
                y: 0,
                unit_type: "support_artillery".into(),
                source_vector: DivisionSubunitVector::Support,
            }
        );
        assert_eq!(
            baseline.regimental_support[0],
            DivisionSubunit {
                x: 0,
                y: 0,
                unit_type: "signal_company".into(),
                source_vector: DivisionSubunitVector::RegimentalSupport,
            }
        );

        let upserted = catalog.division_composition_at_hour(901).unwrap();
        let upserted = upserted.get("8:52").unwrap();
        assert_eq!(upserted.regiment_grid_columns, 6);
        assert_eq!(upserted.regiments[0].x, 4);
        assert_eq!(upserted.regiments[0].unit_type, "motorized");

        assert!(catalog
            .division_composition_at_hour(902)
            .unwrap()
            .is_empty());
    }
    #[test]
    fn native_designer_accessor_preserves_inputs_without_claiming_designer_outcomes() {
        let catalog = PortableDesignCatalog::decode_a27_live_json(&fixture()).unwrap();
        let inputs = catalog.native_designer_inputs_at_hour(900).unwrap();
        assert_eq!(catalog.schema, A27_LIVE_CATALOG_SCHEMA);
        assert_eq!(inputs.hour, 900);
        assert_eq!(inputs.source_record_id, catalog.source_record_id);
        assert_eq!(inputs.variants.len(), 1);
        let row = &inputs.variants[0];
        assert_eq!(row.design_id, "7:70");
        assert_eq!(row.record, catalog.records_at_hour(900).unwrap().into_iter().find(|r| r.design_id == "7:70").unwrap());
        assert_eq!(row.model, catalog.variant_model_inputs_at_hour(900, 7).unwrap());
        assert_eq!(row.equipment_definition_token, catalog.exact_native_catalog["variants"][0]["definition"]["archetype_token"].as_str().unwrap());
        assert_eq!(row.equipment_token_raw as u64, catalog.exact_native_catalog["variants"][0]["definition"]["equipment_token_raw"].as_u64().unwrap());
        assert_eq!(inputs.countries[0].registry_index, 0);
        assert_eq!(inputs.countries[0].tag, "GER");
        assert_eq!(inputs.countries[0].designer_pointer_order_ordinal, None);
        assert!(catalog.native_designer_inputs_at_hour(899).is_err());
        assert!(catalog.native_designer_inputs_at_hour(901).is_err());
    }

    #[test]
    fn native_designer_accessor_preserves_zero_pointer_ordinal_and_refuses_gap() {
        let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
        raw["countries"][0]["designer_pointer_order_ordinal"] = 0.into();
        let mut catalog = PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap();
        assert_eq!(catalog.native_designer_inputs_at_hour(900).unwrap().countries[0].designer_pointer_order_ordinal, Some(0));
        catalog.gaps.push(DesignCatalogGap { first_hour_index: 900, last_hour_index: 900,
            cause: DesignCatalogGapCause::Torn, witness: serde_json::json!({"test":"exact_gap"}) });
        assert!(catalog.native_designer_inputs_at_hour(900).unwrap_err().contains("Torn"));
    }

    fn item_icon_states() -> Vec<DesignItemIconObservation> {
        let measured = |active, kind, value: &str| DesignItemIconObservation::Measured {
            active, kind, token: DesignTextObservation::Measured {
                value: value.into(),
                utf8_hex: value.as_bytes().iter().map(|byte| format!("{byte:02x}")).collect(),
            },
        };
        vec![
            measured(false, 1, ""),
            measured(true, 0, "GFX_tank"),
            measured(true, 1, "."),
            measured(true, 1, "gfx/interface/t\u{e4}nk.dds"),
            DesignItemIconObservation::Measured {
                active: true, kind: 0,
                token: DesignTextObservation::Unavailable { reason: DesignFieldRefusal {
                    kind: "invalid_string".into(), field: "variant.item_icon_override.token".into(),
                } },
            },
            DesignItemIconObservation::Unavailable { reason: DesignFieldRefusal {
                kind: "invalid_identity".into(), field: "variant.item_icon_override.kind".into(),
            } },
        ]
    }

    #[test]
    fn item_icon_accessor_preserves_each_observation_and_exact_record_join() {
        let mut states = item_icon_states();
        states.push(DesignItemIconObservation::Measured { active: false, kind: 1,
            token: DesignTextObservation::Unavailable { reason: DesignFieldRefusal {
                kind: "unreadable".into(), field: "variant.item_icon_override.token".into(),
            } } });
        states.push(DesignItemIconObservation::Unavailable { reason: DesignFieldRefusal {
            kind: "unreadable".into(), field: "variant.item_icon_override.header".into(),
        } });
        for expected in states {
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw["variants"][0]["definition"]["item_icon_override"] = serde_json::to_value(&expected).unwrap();
            let catalog = PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap();
            let row = catalog.native_designer_inputs_at_hour(900).unwrap().variants.remove(0);
            assert_eq!(row.item_icon_override, expected);
            assert_eq!(row.design_id, "7:70");
            assert_eq!(row.record, catalog.get("7:70").unwrap().clone());
            assert_eq!(row.model, catalog.variant_model_inputs_at_hour(900, 7).unwrap());
            assert_eq!(catalog.exact_native_catalog["variants"][0]["definition"]["item_icon_override"], serde_json::to_value(expected).unwrap());
        }
    }

    #[test]
    fn item_icon_rejects_missing_malformed_and_cross_level_refusals() {
        let measured = serde_json::to_value(&item_icon_states()[0]).unwrap();
        let mut invalid = Vec::new();
        for (key, value) in [("kind", serde_json::json!(2)), ("kind", serde_json::json!(-1)),
            ("active", serde_json::json!(1)), ("extra", serde_json::json!(true))] {
            let mut icon = measured.clone();
            icon[key] = value;
            invalid.push(icon);
        }
        for (value, hex) in [("\u{e9}", "e9"), ("x", ""), ("", "00")] {
            let mut icon = measured.clone();
            icon["token"] = serde_json::json!({"status":"measured","value":value,"utf8_hex":hex});
            invalid.push(icon);
        }
        let mut oversized = measured.clone();
        oversized["token"] = serde_json::json!({"status":"measured","value":"x".repeat(4097),"utf8_hex":"78".repeat(4097)});
        invalid.push(oversized);
        for (kind, field) in [("invalid_identity", "token"), ("unreadable", ""), ("torn", "token")] {
            let mut icon = measured.clone();
            icon["token"] = serde_json::json!({"status":"unavailable","reason":{"kind":kind,"field":field}});
            invalid.push(icon);
        }
        for (kind, field) in [("invalid_string", "header"), ("invalid_identity", ""), ("bound_exceeded", "header")] {
            invalid.push(serde_json::json!({"status":"unavailable","reason":{"kind":kind,"field":field}}));
        }
        invalid.push(serde_json::json!({"status":"measured","active":true,"kind":1}));
        invalid.push(Value::Null);
        for icon in invalid {
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw["variants"][0]["definition"]["item_icon_override"] = icon.clone();
            assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err(), "{icon}");
        }
        let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
        raw["variants"][0]["definition"].as_object_mut().unwrap().remove("item_icon_override");
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap_err().contains("item_icon_override"));
        let mut boundary = measured;
        boundary["token"] = serde_json::json!({"status":"measured","value":"x".repeat(4096),"utf8_hex":"78".repeat(4096)});
        raw["variants"][0]["definition"]["item_icon_override"] = boundary;
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_ok());
    }

    #[test]
    fn item_icon_timeline_retains_deltas_rechecks_gap_and_full_recovery() {
        let states = item_icon_states();
        let mut captures = Vec::new();
        let mut previous = None;
        for (index, icon) in states.iter().chain(states.first()).enumerate() {
            let hour = 900 + index as u64 + u64::from(index == states.len());
            if index == states.len() {
                captures.push(serde_json::json!({"sequence":index,"capture_hour_index":hour-1,
                    "gap": DesignCatalogGap { first_hour_index:hour-1, last_hour_index:hour-1,
                        cause: DesignCatalogGapCause::Torn, witness:serde_json::json!({"field":"item_icon_override"}) }}));
            }
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw["authority"]["capture_hour_index"] = hour.into();
            raw["authority"]["current_date_raw"] = (DATE_EPOCH_RAW + hour).into();
            raw["variants"][0]["definition"]["item_icon_override"] = serde_json::to_value(icon).unwrap();
            let bytes = serde_json::to_vec(&canonical_json_value(&raw)).unwrap();
            let current = PortableDesignCatalog::decode_a27_live_json(&bytes).unwrap();
            let mut entry = serde_json::json!({"sequence":captures.len(),"capture_hour_index":hour,
                "same_hour_rechecks":1,"same_hour_recheck_sha256":[Sha256::hex(&bytes)],"snapshot_sha256":Sha256::hex(&bytes)});
            if index == 0 || index == states.len() {
                entry["snapshot"] = raw;
            } else {
                let delta = current.native_row_delta_from(previous.as_ref().unwrap()).unwrap();
                assert_eq!(delta["variant_upserts"].as_array().unwrap().len(), 1);
                assert_eq!(delta["variant_upserts"][0]["definition"]["item_icon_override"], serde_json::to_value(icon).unwrap());
                entry["delta"] = delta;
            }
            captures.push(entry);
            previous = Some(current);
        }
        let mut timeline = serde_json::json!({"schema":A27_LIVE_CATALOG_TIMELINE_SCHEMA,
            "authority":{"program":EXACT_A729_PROGRAM,"program_sha256":EXACT_A729_SHA256},
            "complete_sequence":true,"transport":"first_full_then_native_row_deltas",
            "capture_count":captures.len(),"same_hour_recheck_count":states.len()+1,
            "first_capture_hour_index":900,"last_capture_hour_index":907,"captures":captures});
        let catalog = PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&timeline).unwrap()).unwrap();
        let relocated = PortableDesignCatalog::decode_a27_live_json(&catalog.encode_portable_json().unwrap()).unwrap();
        for (index, expected) in states.iter().enumerate() {
            let inputs = relocated.native_designer_inputs_at_hour(900 + index as u64).unwrap();
            assert_eq!(&inputs.variants[0].item_icon_override, expected);
            assert_eq!(inputs.variants[0].record.source_record_id, format!("{}:variant:7:70", inputs.source_record_id));
        }
        assert!(relocated.native_designer_inputs_at_hour(906).unwrap_err().contains("Torn"));
        assert_eq!(relocated.native_designer_inputs_at_hour(907).unwrap().variants[0].item_icon_override, states[0]);
        assert!(relocated.native_designer_inputs_at_hour(908).is_err());
        timeline["captures"][1]["same_hour_recheck_sha256"][0] = "0".repeat(64).into();
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&timeline).unwrap()).is_err());
    }

    fn byte_states() -> Vec<DesignByteObservation> {
        let mut states: Vec<_> = [0, 1, 2, 128, 255].into_iter()
            .map(|raw_u8| DesignByteObservation::Measured { raw_u8 }).collect();
        states.push(DesignByteObservation::Unavailable { reason: DesignFieldRefusal {
            kind: "unreadable".into(), field: "variant.serialized_variant_flag_2b9d".into(),
        } });
        states
    }

    #[test]
    fn serialized_variant_flag_preserves_full_byte_and_unreadable() {
        let mut states: Vec<_> = (0..=255).map(|raw_u8| DesignByteObservation::Measured { raw_u8 }).collect();
        states.push(byte_states().pop().unwrap());
        for expected in states {
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw["variants"][0]["definition"]["serialized_variant_flag_2b9d"] = serde_json::to_value(&expected).unwrap();
            let catalog = PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap();
            let row = catalog.native_designer_inputs_at_hour(900).unwrap().variants.remove(0);
            assert_eq!(row.serialized_variant_flag_2b9d, expected);
            assert_eq!(row.design_id, "7:70");
            assert_eq!(row.record, catalog.get("7:70").unwrap().clone());
            assert_eq!(row.model, catalog.variant_model_inputs_at_hour(900, 7).unwrap());
            assert_eq!(catalog.exact_native_catalog["variants"][0]["definition"]["serialized_variant_flag_2b9d"], serde_json::to_value(expected).unwrap());
        }
    }

    #[test]
    fn serialized_variant_flag_rejects_missing_and_malformed_observations() {
        let mut invalid = vec![Value::Null, serde_json::json!({"status":"measured"}),
            serde_json::json!({"status":"unknown","raw_u8":0}),
            serde_json::json!({"status":"measured","raw_u8":0,"extra":true}),
            serde_json::json!({"status":"unavailable"})];
        for value in [serde_json::json!(-1), serde_json::json!(256), serde_json::json!(true),
            serde_json::json!(1.5), serde_json::json!("0"), Value::Null] {
            invalid.push(serde_json::json!({"status":"measured","raw_u8":value}));
        }
        for (kind, field) in [("torn", "byte"), ("bound_exceeded", "byte"),
            ("invalid_identity", "byte"), ("unreadable", "")] {
            invalid.push(serde_json::json!({"status":"unavailable","reason":{"kind":kind,"field":field}}));
        }
        invalid.push(serde_json::json!({"status":"unavailable","reason":{"kind":"unreadable","field":"byte","extra":0}}));
        for value in invalid {
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw["variants"][0]["definition"]["serialized_variant_flag_2b9d"] = value.clone();
            assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err(), "{value}");
        }
        let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
        raw["variants"][0]["definition"].as_object_mut().unwrap().remove("serialized_variant_flag_2b9d");
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap())
            .unwrap_err().contains("serialized_variant_flag_2b9d"));
    }

    #[test]
    fn serialized_variant_flag_2b9d_timeline_retains_deltas_rechecks_gap_and_full_recovery() {
        let states = byte_states();
        let mut captures = Vec::new();
        let mut previous = None;
        for (index, flag) in states.iter().chain(states.first()).enumerate() {
            let hour = 900 + index as u64 + u64::from(index == states.len());
            if index == states.len() {
                captures.push(serde_json::json!({"sequence":index,"capture_hour_index":hour-1,
                    "gap": DesignCatalogGap { first_hour_index:hour-1, last_hour_index:hour-1,
                        cause: DesignCatalogGapCause::Torn, witness:serde_json::json!({"field":"serialized_variant_flag_2b9d"}) }}));
            }
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw["authority"]["capture_hour_index"] = hour.into();
            raw["authority"]["current_date_raw"] = (DATE_EPOCH_RAW + hour).into();
            raw["variants"][0]["definition"]["serialized_variant_flag_2b9d"] = serde_json::to_value(flag).unwrap();
            let bytes = serde_json::to_vec(&canonical_json_value(&raw)).unwrap();
            let current = PortableDesignCatalog::decode_a27_live_json(&bytes).unwrap();
            let mut entry = serde_json::json!({"sequence":captures.len(),"capture_hour_index":hour,
                "same_hour_rechecks":1,"same_hour_recheck_sha256":[Sha256::hex(&bytes)],"snapshot_sha256":Sha256::hex(&bytes)});
            if index == 0 || index == states.len() {
                entry["snapshot"] = raw;
            } else {
                let delta = current.native_row_delta_from(previous.as_ref().unwrap()).unwrap();
                assert_eq!(delta["variant_upserts"].as_array().unwrap().len(), 1);
                assert_eq!(delta["variant_upserts"][0]["definition"]["serialized_variant_flag_2b9d"], serde_json::to_value(flag).unwrap());
                entry["delta"] = delta;
            }
            captures.push(entry);
            previous = Some(current);
        }
        let mut timeline = serde_json::json!({"schema":A27_LIVE_CATALOG_TIMELINE_SCHEMA,
            "authority":{"program":EXACT_A729_PROGRAM,"program_sha256":EXACT_A729_SHA256},
            "complete_sequence":true,"transport":"first_full_then_native_row_deltas",
            "capture_count":captures.len(),"same_hour_recheck_count":states.len()+1,
            "first_capture_hour_index":900,"last_capture_hour_index":907,"captures":captures});
        let catalog = PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&timeline).unwrap()).unwrap();
        let relocated = PortableDesignCatalog::decode_a27_live_json(&catalog.encode_portable_json().unwrap()).unwrap();
        for (index, expected) in states.iter().enumerate() {
            let inputs = relocated.native_designer_inputs_at_hour(900 + index as u64).unwrap();
            assert_eq!(&inputs.variants[0].serialized_variant_flag_2b9d, expected);
            assert_eq!(inputs.variants[0].record.source_record_id, format!("{}:variant:7:70", inputs.source_record_id));
        }
        assert!(relocated.native_designer_inputs_at_hour(906).unwrap_err().contains("Torn"));
        assert_eq!(relocated.native_designer_inputs_at_hour(907).unwrap().variants[0].serialized_variant_flag_2b9d, states[0]);
        assert!(relocated.native_designer_inputs_at_hour(908).is_err());
        timeline["captures"][1]["same_hour_recheck_sha256"][0] = "0".repeat(64).into();
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&timeline).unwrap()).is_err());
    }
    fn runtime_mask_states() -> Vec<DesignU32Observation> {
        let mut states: Vec<_> = [0, 1, 0x190, 0x80000000, u32::MAX].into_iter()
            .map(|raw_u32| DesignU32Observation::Measured { raw_u32 }).collect();
        states.push(DesignU32Observation::Unavailable { reason: DesignFieldRefusal {
            kind: "unreadable".into(), field: "runtime_dlc_mask".into(),
        } });
        states
    }

    #[test]
    fn runtime_dlc_mask_preserves_zero_reserved_bits_and_unreadable() {
        let mut states = runtime_mask_states();
        states.extend((0..32).map(|bit| DesignU32Observation::Measured { raw_u32: 1 << bit }));
        for expected in states {
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw["runtime_dlc_mask"] = serde_json::to_value(&expected).unwrap();
            let catalog = PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap();
            let inputs = catalog.native_designer_inputs_at_hour(900).unwrap();
            assert_eq!(inputs.runtime_dlc_mask, expected);
            assert_eq!(inputs.variants[0].record, catalog.get("7:70").unwrap().clone());
            assert_eq!(catalog.exact_native_catalog["runtime_dlc_mask"], serde_json::to_value(expected).unwrap());
        }
    }

    #[test]
    fn runtime_dlc_mask_rejects_missing_malformed_and_wrong_refusal_level() {
        let mut invalid = vec![Value::Null, serde_json::json!({"status":"measured"}),
            serde_json::json!({"status":"unknown","raw_u32":0}),
            serde_json::json!({"status":"measured","raw_u32":0,"extra":true}),
            serde_json::json!({"status":"unavailable"})];
        for value in [serde_json::json!(-1), serde_json::json!(4294967296u64), serde_json::json!(true),
            serde_json::json!(1.5), serde_json::json!("0"), Value::Null] {
            invalid.push(serde_json::json!({"status":"measured","raw_u32":value}));
        }
        for (kind, field) in [("torn", "mask"), ("bound_exceeded", "mask"),
            ("invalid_identity", "mask"), ("unreadable", "")] {
            invalid.push(serde_json::json!({"status":"unavailable","reason":{"kind":kind,"field":field}}));
        }
        invalid.push(serde_json::json!({"status":"unavailable","reason":{"kind":"unreadable","field":"mask","extra":0}}));
        for value in invalid {
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw["runtime_dlc_mask"] = value.clone();
            assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err(), "{value}");
        }
        let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
        raw.as_object_mut().unwrap().remove("runtime_dlc_mask");
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap())
            .unwrap_err().contains("runtime_dlc_mask"));
    }

    #[test]
    fn runtime_dlc_mask_timeline_retains_deltas_rechecks_gap_and_full_recovery() {
        let states = runtime_mask_states();
        let mut captures = Vec::new();
        let mut previous = None;
        for (index, flag) in states.iter().chain(states.first()).enumerate() {
            let hour = 900 + index as u64 + u64::from(index == states.len());
            if index == states.len() {
                captures.push(serde_json::json!({"sequence":index,"capture_hour_index":hour-1,
                    "gap": DesignCatalogGap { first_hour_index:hour-1, last_hour_index:hour-1,
                        cause: DesignCatalogGapCause::Torn, witness:serde_json::json!({"field":"runtime_dlc_mask"}) }}));
            }
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw["authority"]["capture_hour_index"] = hour.into();
            raw["authority"]["current_date_raw"] = (DATE_EPOCH_RAW + hour).into();
            raw["runtime_dlc_mask"] = serde_json::to_value(flag).unwrap();
            let bytes = serde_json::to_vec(&canonical_json_value(&raw)).unwrap();
            let current = PortableDesignCatalog::decode_a27_live_json(&bytes).unwrap();
            let mut entry = serde_json::json!({"sequence":captures.len(),"capture_hour_index":hour,
                "same_hour_rechecks":1,"same_hour_recheck_sha256":[Sha256::hex(&bytes)],"snapshot_sha256":Sha256::hex(&bytes)});
            if index == 0 || index == states.len() {
                entry["snapshot"] = raw;
            } else {
                let delta = current.native_row_delta_from(previous.as_ref().unwrap()).unwrap();
                assert!(delta["variant_upserts"].as_array().unwrap().is_empty());
                assert_eq!(delta["frame_metadata"]["runtime_dlc_mask"], serde_json::to_value(flag).unwrap());
                entry["delta"] = delta;
            }
            captures.push(entry);
            previous = Some(current);
        }
        let mut timeline = serde_json::json!({"schema":A27_LIVE_CATALOG_TIMELINE_SCHEMA,
            "authority":{"program":EXACT_A729_PROGRAM,"program_sha256":EXACT_A729_SHA256},
            "complete_sequence":true,"transport":"first_full_then_native_row_deltas",
            "capture_count":captures.len(),"same_hour_recheck_count":states.len()+1,
            "first_capture_hour_index":900,"last_capture_hour_index":907,"captures":captures});
        let catalog = PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&timeline).unwrap()).unwrap();
        let relocated = PortableDesignCatalog::decode_a27_live_json(&catalog.encode_portable_json().unwrap()).unwrap();
        let baseline_record = relocated.native_designer_inputs_at_hour(900).unwrap().variants[0].record.clone();
        for (index, expected) in states.iter().enumerate() {
            let inputs = relocated.native_designer_inputs_at_hour(900 + index as u64).unwrap();
            assert_eq!(&inputs.runtime_dlc_mask, expected);
            assert_eq!(inputs.variants[0].record, baseline_record);
            let records = relocated.records_at_hour(900 + index as u64).unwrap();
            assert_eq!(&inputs.variants[0].record, records.iter().find(|row| row.design_id == "7:70").unwrap());
            assert_eq!(inputs.source_record_id, format!("a27-live-catalog:{}",
                timeline["captures"][index]["snapshot_sha256"].as_str().unwrap()));
        }
        assert!(relocated.native_designer_inputs_at_hour(906).unwrap_err().contains("Torn"));
        let recovered = relocated.native_designer_inputs_at_hour(907).unwrap();
        assert_eq!(recovered.runtime_dlc_mask, states[0]);
        assert_eq!(recovered.variants[0].record.source_record_id, format!("{}:variant:7:70", recovered.source_record_id));
        assert_ne!(recovered.variants[0].record.source_record_id, baseline_record.source_record_id);
        assert_eq!(&recovered.variants[0].record,
            relocated.records_at_hour(907).unwrap().iter().find(|row| row.design_id == "7:70").unwrap());
        assert!(relocated.native_designer_inputs_at_hour(908).is_err());
        timeline["captures"][1]["same_hour_recheck_sha256"][0] = "0".repeat(64).into();
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&timeline).unwrap()).is_err());
    }
    #[test]
    fn cached_stats_preserve_full_signed_width_beyond_float_and_i32_precision() {
        let values = [i64::MIN, i64::MAX, -(1_i64<<32)-17, (1_i64<<32)+19,
            (1_i64<<53), (1_i64<<53)+1, -1, 0, 1, i32::MAX as i64 + 1];
        let expected: Vec<_> = values.into_iter().enumerate().map(|(ordinal,raw_i64)| DesignFixedStat {
            raw_i64, scale:100_000, source_ordinal:ordinal as u32, stat_id:format!("stat_{ordinal}"),
        }).collect();
        let mut raw: Value=serde_json::from_slice(&fixture()).unwrap();
        raw["variants"][0]["definition"]["cached_stats"]=serde_json::to_value(&expected).unwrap();
        let catalog=PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap();
        let relocated=PortableDesignCatalog::decode_a27_live_json(&catalog.encode_portable_json().unwrap()).unwrap();
        let input=relocated.native_designer_inputs_at_hour(900).unwrap();
        assert_eq!(input.variants[0].cached_stats,expected);
        assert_eq!(relocated.exact_native_catalog["variants"][0]["definition"]["cached_stats"],serde_json::to_value(&expected).unwrap());
        assert_ne!(input.variants[0].cached_stats[4],input.variants[0].cached_stats[5]);
        assert_ne!(input.variants[0].cached_stats[4].raw_i64,input.variants[0].cached_stats[5].raw_i64);
        assert_eq!(input.variants[0].record.stats[4].value,input.variants[0].record.stats[5].value,
            "display floats can coincide; exact native operands must not");
    }

    #[test]
    fn cached_stats_refuse_old_missing_mixed_or_non_i64_raw_values() {
        let raw:Value=serde_json::from_slice(&fixture()).unwrap();
        let base=raw["variants"][0]["definition"]["cached_stats"][0].clone();
        let mut invalid=Vec::new();
        let mut old=base.clone(); let value=old.as_object_mut().unwrap().remove("raw_i64").unwrap();
        old["raw_i32"]=value; invalid.push(old);
        let mut missing=base.clone();missing.as_object_mut().unwrap().remove("raw_i64");invalid.push(missing);
        let mut mixed=base.clone();mixed["raw_i32"]=0.into();invalid.push(mixed);
        for value in [Value::Null,serde_json::json!(true),serde_json::json!(1.5),serde_json::json!("1"),serde_json::json!(u64::MAX)] {
            let mut row=base.clone();row["raw_i64"]=value;invalid.push(row);
        }
        for row in invalid {
            let mut current=raw.clone();current["variants"][0]["definition"]["cached_stats"][0]=row.clone();
            assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&current).unwrap()).is_err(),"{row}");
        }
    }

    #[test]
    fn cached_stat_high_bits_survive_row_deltas_same_hour_rechecks_gaps_and_full_recovery() {
        let values=[(1_i64<<53),(1_i64<<53)+1,i64::MIN];
        let mut previous=None;let mut captures=Vec::new();
        for (index,raw_i64) in values.into_iter().enumerate() {
            let hour=900+index as u64+u64::from(index==2);
            if index==2 {
                captures.push(serde_json::json!({"sequence":captures.len(),"capture_hour_index":902,
                    "gap":DesignCatalogGap { first_hour_index:902,last_hour_index:902,cause:DesignCatalogGapCause::Torn,
                        witness:serde_json::json!({"field":"cached_stats"}) }}));
            }
            let mut raw:Value=serde_json::from_slice(&fixture()).unwrap();
            raw["authority"]["capture_hour_index"]=hour.into();raw["authority"]["current_date_raw"]=(DATE_EPOCH_RAW+hour).into();
            raw["variants"][0]["definition"]["cached_stats"][0]["raw_i64"]=raw_i64.into();
            let bytes=serde_json::to_vec(&canonical_json_value(&raw)).unwrap();
            let current=PortableDesignCatalog::decode_a27_live_json(&bytes).unwrap();
            let mut capture=serde_json::json!({"sequence":captures.len(),"capture_hour_index":hour,"same_hour_rechecks":1,
                "same_hour_recheck_sha256":[Sha256::hex(&bytes)],"snapshot_sha256":Sha256::hex(&bytes)});
            if index==1 {
                let delta=current.native_row_delta_from(previous.as_ref().unwrap()).unwrap();
                assert_eq!(delta["variant_upserts"].as_array().unwrap().len(),1,"high bits are not display-float equality");
                capture["delta"]=delta;
            } else { capture["snapshot"]=raw; }
            captures.push(capture);previous=Some(current);
        }
        let mut timeline=serde_json::json!({"schema":A27_LIVE_CATALOG_TIMELINE_SCHEMA,
            "authority":{"program":EXACT_A729_PROGRAM,"program_sha256":EXACT_A729_SHA256},
            "complete_sequence":true,"transport":"first_full_then_native_row_deltas","capture_count":4,
            "same_hour_recheck_count":3,"first_capture_hour_index":900,"last_capture_hour_index":903,"captures":captures});
        let catalog=PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&timeline).unwrap()).unwrap();
        let relocated=PortableDesignCatalog::decode_a27_live_json(&catalog.encode_portable_json().unwrap()).unwrap();
        for (hour,raw) in [(900,values[0]),(901,values[1]),(903,values[2])] {
            let inputs=relocated.native_designer_inputs_at_hour(hour).unwrap();
            assert_eq!(inputs.variants[0].cached_stats[0].raw_i64,raw);
            assert_eq!(inputs.variants[0].cached_stats[0].scale,100_000);
            assert_eq!(&inputs.variants[0].record,relocated.records_at_hour(hour).unwrap().iter().find(|row|row.design_id=="7:70").unwrap());
        }
        assert!(relocated.native_designer_inputs_at_hour(902).unwrap_err().contains("Torn"));
        assert!(relocated.native_designer_inputs_at_hour(904).is_err());
        timeline["captures"][1]["same_hour_recheck_sha256"][0]="0".repeat(64).into();
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&timeline).unwrap()).is_err());
    }

    fn mission_fixture(lookups: [Value; 2]) -> Value {
        let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
        for (index, lookup) in lookups.into_iter().enumerate() {
            raw["variants"][0]["definition"]["mission_stat_operands"][index]["override_lookup"] = lookup;
        }
        raw
    }

    #[test]
    fn mission_operands_keep_first_match_absence_unknown_and_signed_values_distinct() {
        let mut raw = mission_fixture([
            serde_json::json!({"status":"measured","raw_i64":i64::MAX,"group_count":4,
                "group_key_prefix":[9,4096],"stat_count":3,"stat_key_prefix":[8,35]}),
            serde_json::json!({"status":"stat_absent","group_count":4,
                "group_key_prefix":[9,4096,8192],"stat_count":2,"stat_key_prefix":[8,9]})]);
        raw["variants"][0]["definition"]["mission_stat_operands"][0]["fallback"] =
            serde_json::json!({"status":"unavailable","reason":{"kind":"unreadable","field":"variant+218"}});
        raw["variants"][0]["definition"]["mission_stat_operands"][1]["fallback"]["raw_i64"] = i64::MIN.into();
        let decode = |raw: &Value| PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(raw).unwrap()).unwrap();
        let catalog = decode(&raw);
        let portable = PortableDesignCatalog::decode_a27_live_json(&catalog.encode_portable_json().unwrap()).unwrap();
        let rows = portable.native_designer_inputs_at_hour(900).unwrap().variants.remove(0).mission_stat_operands;
        assert_eq!(rows[0].effective_raw_i64(), Ok(i64::MAX));
        assert_eq!(rows[1].effective_raw_i64(), Ok(i64::MIN));
        assert!(matches!(rows[0].fallback, DesignI64Observation::Unavailable { .. }));
        assert_eq!(serde_json::to_value(&rows).unwrap(), raw["variants"][0]["definition"]["mission_stat_operands"]);
        for kind in ["unreadable", "invalid_vector"] {
            raw["variants"][0]["definition"]["mission_stat_operands"][1]["override_lookup"] =
                serde_json::json!({"status":"unavailable","reason":{"kind":kind,"field":"mission_group_count"}});
            let row = decode(&raw).native_designer_inputs_at_hour(900).unwrap().variants.remove(0).mission_stat_operands.remove(1);
            assert_eq!(row.effective_raw_i64().unwrap_err().kind, kind);
        }
        raw["variants"][0]["definition"]["mission_stat_operands"][1]["override_lookup"] =
            serde_json::json!({"status":"group_absent","group_count":4,"group_key_prefix":[9,4096,4096,10]});
        assert_eq!(decode(&raw).native_designer_inputs_at_hour(900).unwrap().variants[0].mission_stat_operands[1].effective_raw_i64(), Ok(i64::MIN));
        // A later duplicate group is outside the first group's nested lookup authority.
        raw["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"] =
            serde_json::json!({"status":"stat_absent","group_count":4,"group_key_prefix":[9,4096],"stat_count":0,"stat_key_prefix":[]});
        assert_eq!(decode(&raw).native_designer_inputs_at_hour(900).unwrap().variants[0].mission_stat_operands[0].effective_raw_i64().unwrap_err().field,"variant+218");
    }

    #[test]
    fn mission_operands_reject_incomplete_or_contradictory_prefix_custody() {
        let valid = mission_fixture([
            serde_json::json!({"status":"measured","raw_i64":0,"group_count":3,"group_key_prefix":[4096],"stat_count":2,"stat_key_prefix":[35]}),
            serde_json::json!({"status":"measured","raw_i64":1,"group_count":3,"group_key_prefix":[4096,8192],"stat_count":1,"stat_key_prefix":[36]})]);
        for (field, value) in [
            ("group_count",serde_json::json!(-1)),("group_count",serde_json::json!(i32::MAX as u32+1)),
            ("group_count",serde_json::json!(0)),("group_count",serde_json::json!(4)),
            ("group_key_prefix",serde_json::json!([])),("group_key_prefix",serde_json::json!([4096,4096])),
            ("group_key_prefix",serde_json::json!([10,4096])),("group_key_prefix",serde_json::json!([8192])),
            ("stat_count",serde_json::json!(-1)),("stat_count",serde_json::json!(i32::MAX as u32+1)),
            ("stat_key_prefix",serde_json::json!([35,35])),("stat_key_prefix",serde_json::json!([])),
            ("stat_key_prefix",serde_json::json!([36])),("stat_key_prefix",serde_json::json!([1,2,35])),
            ("raw_i64",serde_json::json!(u64::MAX)),("raw_i64",Value::Null),
        ] {
            let mut raw=valid.clone();raw["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"][field]=value;
            assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err(),"accepted {field}: {raw}");
        }
        for lookup in [
            serde_json::json!({"status":"group_absent","group_count":3,"group_key_prefix":[1,2]}),
            serde_json::json!({"status":"group_absent","group_count":3,"group_key_prefix":[4096,2,3]}),
            serde_json::json!({"status":"stat_absent","group_count":3,"group_key_prefix":[4096],"stat_count":2,"stat_key_prefix":[1]}),
            serde_json::json!({"status":"stat_absent","group_count":3,"group_key_prefix":[4096],"stat_count":2,"stat_key_prefix":[1,35]}),
            serde_json::json!({"status":"unavailable","reason":{"kind":"Torn","field":"keys"}}),
            serde_json::json!({"status":"unavailable","reason":{"kind":"unreadable","field":""}}),
            serde_json::json!({"status":"unavailable","reason":{"kind":"unreadable","field":"keys"},"raw_i64":0}),
        ] {
            let mut raw=valid.clone();raw["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"]=lookup;
            assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err());
        }
        for field in ["group_key","stat_key","fallback","override_lookup"] {
            let mut raw=valid.clone();raw["variants"][0]["definition"]["mission_stat_operands"][0].as_object_mut().unwrap().remove(field);
            assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err());
        }
        let mut raw=valid.clone();raw["variants"][0]["definition"].as_object_mut().unwrap().remove("mission_stat_operands");
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err());
        for replacement in [Value::Null,serde_json::json!([]),serde_json::json!([valid["variants"][0]["definition"]["mission_stat_operands"][0]])] {
            let mut raw=valid.clone();raw["variants"][0]["definition"]["mission_stat_operands"]=replacement;
            assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err());
        }
        let mut raw=valid.clone();raw["variants"][0]["definition"]["mission_stat_operands"].as_array_mut().unwrap().swap(0,1);
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err());
        let mut raw=valid;raw["variants"][0]["definition"]["mission_stat_operands"][0]["fallback"]=
            serde_json::json!({"status":"unavailable","reason":{"kind":"invalid_vector","field":"fallback"}});
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err());
    }

    #[test]
    fn mission_operand_prefix_bound_is_inclusive_and_does_not_require_unread_suffix() {
        let mut raw:Value=serde_json::from_slice(&fixture()).unwrap();
        for index in 0..2 {
            raw["variants"][0]["definition"]["mission_stat_operands"][index]["override_lookup"]=
                serde_json::json!({"status":"group_absent","group_count":4096,"group_key_prefix":vec![7;4096]});
        }
        let decode=|raw:&Value|PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(raw).unwrap());
        assert!(decode(&raw).is_ok());
        raw["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"]=
            serde_json::json!({"status":"stat_absent","group_count":4096,"group_key_prefix":[4096],"stat_count":4096,"stat_key_prefix":vec![7;4096]});
        raw["variants"][0]["definition"]["mission_stat_operands"][1]["override_lookup"]["group_key_prefix"][0]=4096.into();
        assert!(decode(&raw).is_ok(),"later groups need not be read for the first matching lookup");
        raw["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"]["stat_count"]=4097.into();
        assert!(decode(&raw).is_err());
        raw["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"]=
            serde_json::json!({"status":"measured","group_count":i32::MAX,"group_key_prefix":[4096],
                "stat_count":i32::MAX,"stat_key_prefix":[35],"raw_i64":-1});
        raw["variants"][0]["definition"]["mission_stat_operands"][1]["override_lookup"]=
            serde_json::json!({"status":"measured","group_count":i32::MAX,"group_key_prefix":[4096,8192],
                "stat_count":i32::MAX,"stat_key_prefix":[36],"raw_i64":1});
        assert!(decode(&raw).is_ok(),"large observed count does not require reading a suffix after first match");
        raw["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"]["stat_count"]=(i32::MAX as u32+1).into();
        assert!(decode(&raw).is_err(),"signed negative native counts cannot become positive observations");
        raw["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"]["stat_count"]=5000.into();
        let mut keys=vec![7;4096];keys.push(35);
        raw["variants"][0]["definition"]["mission_stat_operands"][0]["override_lookup"]["stat_key_prefix"]=serde_json::json!(keys);
        assert!(decode(&raw).is_err(),"the operational bound applies to keys actually read");
    }

    #[test]
    fn mission_operand_row_deltas_rechecks_gap_and_recovery_retain_exact_authority() {
        let mut captures=Vec::new();let mut previous=None;
        for (hour,value) in [(900,Some(i64::MIN)),(901,Some(i64::MAX)),(902,None),(903,Some(0))] {
            let Some(value)=value else {
                captures.push(serde_json::json!({"sequence":captures.len(),"capture_hour_index":hour,
                    "gap":DesignCatalogGap { first_hour_index:hour,last_hour_index:hour,cause:DesignCatalogGapCause::Torn,
                        witness:serde_json::json!({"field":"mission group prefix"}) }}));
                continue;
            };
            let mut raw:Value=serde_json::from_slice(&fixture()).unwrap();
            raw["authority"]["capture_hour_index"]=hour.into();raw["authority"]["current_date_raw"]=(DATE_EPOCH_RAW+hour).into();
            raw["variants"][0]["definition"]["mission_stat_operands"][0]["fallback"]["raw_i64"]=value.into();
            let bytes=serde_json::to_vec(&canonical_json_value(&raw)).unwrap();
            let current=PortableDesignCatalog::decode_a27_live_json(&bytes).unwrap();
            let mut capture=serde_json::json!({"sequence":captures.len(),"capture_hour_index":hour,"same_hour_rechecks":1,
                "same_hour_recheck_sha256":[Sha256::hex(&bytes)],"snapshot_sha256":Sha256::hex(&bytes)});
            if hour==901 {
                let delta=current.native_row_delta_from(previous.as_ref().unwrap()).unwrap();
                assert_eq!(delta["variant_upserts"].as_array().unwrap().len(),1);capture["delta"]=delta;
            } else {capture["snapshot"]=raw;}
            captures.push(capture);previous=Some(current);
        }
        let mut timeline=serde_json::json!({"schema":A27_LIVE_CATALOG_TIMELINE_SCHEMA,
            "authority":{"program":EXACT_A729_PROGRAM,"program_sha256":EXACT_A729_SHA256},
            "complete_sequence":true,"transport":"first_full_then_native_row_deltas","capture_count":4,
            "same_hour_recheck_count":3,"first_capture_hour_index":900,"last_capture_hour_index":903,"captures":captures});
        let catalog=PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&timeline).unwrap()).unwrap();
        let portable=PortableDesignCatalog::decode_a27_live_json(&catalog.encode_portable_json().unwrap()).unwrap();
        for (hour,expected) in [(900,i64::MIN),(901,i64::MAX),(903,0)] {
            let input=portable.native_designer_inputs_at_hour(hour).unwrap();
            assert_eq!(input.variants[0].mission_stat_operands[0].effective_raw_i64(),Ok(expected));
            assert_eq!(input.variants[0].record,portable.records_at_hour(hour).unwrap().into_iter().find(|row|row.design_id=="7:70").unwrap());
        }
        assert!(portable.native_designer_inputs_at_hour(902).unwrap_err().contains("Torn"));
        assert!(portable.native_designer_inputs_at_hour(904).is_err());
        timeline["captures"][1]["same_hour_recheck_sha256"][0]="0".repeat(64).into();
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&timeline).unwrap()).is_err());
    }


    #[test]
    fn v5_independent_operands_are_mandatory_and_exactly_typed() {
        for (family, field) in [("variants", "slot_identities"), ("variants", "append_variant_name"),
            ("division_templates", "country_289a"), ("division_templates", "country_3abe")] {
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw[family][0]["definition"].as_object_mut().unwrap().remove(field);
            assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap())
                .unwrap_err().contains(field));
        }
        for signed in [i32::MIN, -1, 0, 1, i32::MAX] {
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw["division_templates"][0]["definition"]["country_289a"] =
                serde_json::json!({"status":"measured","raw_i32":signed});
            let catalog = PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap();
            let inputs = catalog.native_designer_inputs_at_hour(900).unwrap();
            assert_eq!(inputs.templates[0].country_289a, DesignI32Observation::Measured { raw_i32: signed });
            assert_eq!(inputs.templates[0].design_id, "8:52");
            assert_eq!(inputs.templates[0].template_id, 8);
        }
        for byte in 0..=u8::MAX {
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw["variants"][0]["definition"]["append_variant_name"] =
                serde_json::json!({"status":"measured","raw_u8":byte});
            let catalog = PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap();
            assert_eq!(catalog.native_designer_inputs_at_hour(900).unwrap().variants[0].append_variant_name,
                DesignByteObservation::Measured { raw_u8: byte });
        }
        for (family, field, value) in [
            ("division_templates", "country_289a", serde_json::json!({"status":"measured","raw_i32":2147483648u64})),
            ("division_templates", "country_3abe", serde_json::json!({"status":"measured","raw_i32":-2147483649i64})),
            ("variants", "append_variant_name", serde_json::json!({"status":"measured","raw_u8":256})),
            ("variants", "append_variant_name", serde_json::json!({"status":"measured","raw_u8":true})),
            ("division_templates", "country_289a", serde_json::json!({"status":"measured","raw_i32":0.0})),
        ] {
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            raw[family][0]["definition"][field] = value;
            assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err());
        }
    }

    #[test]
    fn v5_complete_slot_order_preserves_unassigned_sentinel_and_measured_empty() {
        let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
        raw["variants"][0]["definition"]["slot_identities"]["slots"] = serde_json::json!([
            {"source_ordinal":0,"token_handle_raw":500,"token":{"status":"measured","value":"primary","utf8_hex":"7072696d617279"}},
            {"source_ordinal":1,"token_handle_raw":357,"token":{"status":"measured","value":"none","utf8_hex":"6e6f6e65"}}]);
        let decode = |raw: &Value| PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(raw).unwrap()).unwrap();
        let input = decode(&raw).native_designer_inputs_at_hour(900).unwrap();
        let DesignSlotIdentityObservation::Measured { slots, .. } = &input.variants[0].slot_identities else { panic!() };
        assert_eq!(slots[1].token_handle_raw, 0x165);
        // The assignment still has raw 91: it did not choose census membership.
        assert_eq!(input.variants[0].record.modules[0].slot_id, "handle:91");
        raw["variants"][0]["definition"]["slot_identities"]["slots"] = serde_json::json!([]);
        let empty = decode(&raw).native_designer_inputs_at_hour(900).unwrap();
        assert!(matches!(&empty.variants[0].slot_identities,
            DesignSlotIdentityObservation::Measured { slots, .. } if slots.is_empty()));
        raw["variants"][0]["definition"]["slot_identities"] = serde_json::json!({
            "status":"unavailable","reason":{"kind":"lexer_unavailable","field":"lexer root"}});
        assert!(matches!(&decode(&raw).native_designer_inputs_at_hour(900).unwrap().variants[0].slot_identities,
            DesignSlotIdentityObservation::Unavailable { .. }));
    }

    #[test]
    fn v5_slot_grammar_rejects_partial_wrong_or_ambiguous_identity() {
        let baseline: Value = serde_json::from_slice(&fixture()).unwrap();
        for case in 0..9 {
            let mut raw = baseline.clone();
            let receipt = &mut raw["variants"][0]["definition"]["slot_identities"];
            match case {
                0 => { receipt.as_object_mut().unwrap().remove("slots"); }
                1 => { receipt["slots"][0]["source_ordinal"] = 1.into(); }
                2 => { receipt["slots"][0]["token_handle_raw"] = 0.into(); }
                3 => { receipt["slots"][0]["token"]["utf8_hex"] = "00".into(); }
                4 => { receipt["slots"][0]["token"] = serde_json::json!({"status":"unavailable","reason":{"kind":"unreadable","field":"token"}}); }
                5 => { let row = receipt["slots"][0].clone(); receipt["slots"].as_array_mut().unwrap().push(row); }
                6 => { receipt["slots"][0]["unexpected_hash"] = 91.into(); }
                7 => { receipt["slots"][0]["token_handle_raw"] = 0x1_0000_0000u64.into(); }
                _ => { receipt["slots"][0]["token"]["value"] = "x".repeat(161).into();
                    receipt["slots"][0]["token"]["utf8_hex"] = "78".repeat(161).into(); }
            }
            assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err(), "case {case}");
        }
        let mut raw = baseline;
        raw["schema"] = "extractor.a27.native_catalog.a729.v4".into();
        assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err());
    }

    #[test]
    fn v5_template_projection_requires_the_existing_exact_hour_record() {
        let mut catalog = PortableDesignCatalog::decode_a27_live_json(&fixture()).unwrap();
        catalog.records.retain(|row| row.design_id != "8:52");
        assert!(catalog.native_designer_inputs_at_hour(900).is_err());
        assert!(catalog.native_designer_inputs_at_hour(899).is_err());
        assert!(catalog.native_designer_inputs_at_hour(901).is_err());
        let mut wrong_kind = PortableDesignCatalog::decode_a27_live_json(&fixture()).unwrap();
        let variant_kind = wrong_kind.records.iter().find(|row| row.design_id == "7:70").unwrap().kind.clone();
        wrong_kind.records.iter_mut().find(|row| row.design_id == "8:52").unwrap().kind = variant_kind;
        assert!(wrong_kind.native_designer_inputs_at_hour(900).unwrap_err().contains("wrong typed identity"));
    }

    #[test]
    fn v5_each_operand_drives_existing_raw_deltas_gap_and_full_recovery() {
        let baseline: Value = serde_json::from_slice(&fixture()).unwrap();
        let mut states = vec![baseline.clone()];
        for step in 1..5 {
            let mut raw = states.last().unwrap().clone();
            match step {
                1 => raw["division_templates"][0]["definition"]["country_289a"]["raw_i32"] = i32::MIN.into(),
                2 => raw["division_templates"][0]["definition"]["country_3abe"]["raw_i32"] = i32::MAX.into(),
                3 => raw["variants"][0]["definition"]["append_variant_name"]["raw_u8"] = 255.into(),
                _ => raw["variants"][0]["definition"]["slot_identities"]["slots"][0]["token_handle_raw"] = 357.into(),
            }
            states.push(raw);
        }
        let mut captures = Vec::new();
        let mut previous = None;
        for (index, state) in states.iter().chain(states.first()).enumerate() {
            let hour = 900 + index as u64 + u64::from(index == states.len());
            if index == states.len() {
                captures.push(serde_json::json!({"sequence":index,"capture_hour_index":hour-1,
                    "gap": DesignCatalogGap { first_hour_index:hour-1,last_hour_index:hour-1,
                        cause:DesignCatalogGapCause::Torn,witness:serde_json::json!({"field":"A27 identity operands"}) }}));
            }
            let mut raw = state.clone();
            raw["authority"]["capture_hour_index"] = hour.into();
            raw["authority"]["current_date_raw"] = (DATE_EPOCH_RAW + hour).into();
            let bytes = serde_json::to_vec(&canonical_json_value(&raw)).unwrap();
            let current = PortableDesignCatalog::decode_a27_live_json(&bytes).unwrap();
            let mut entry = serde_json::json!({"sequence":captures.len(),"capture_hour_index":hour,
                "same_hour_rechecks":1,"same_hour_recheck_sha256":[Sha256::hex(&bytes)],"snapshot_sha256":Sha256::hex(&bytes)});
            if index == 0 || index == states.len() { entry["snapshot"] = raw; }
            else {
                let delta = current.native_row_delta_from(previous.as_ref().unwrap()).unwrap();
                assert_eq!(delta["template_upserts"].as_array().unwrap().len(), usize::from(index < 3));
                assert_eq!(delta["variant_upserts"].as_array().unwrap().len(), usize::from(index >= 3));
                entry["delta"] = delta;
            }
            captures.push(entry);
            previous = Some(current);
        }
        let timeline = serde_json::json!({"schema":A27_LIVE_CATALOG_TIMELINE_SCHEMA,
            "authority":{"program":EXACT_A729_PROGRAM,"program_sha256":EXACT_A729_SHA256},
            "complete_sequence":true,"transport":"first_full_then_native_row_deltas",
            "capture_count":captures.len(),"same_hour_recheck_count":6,
            "first_capture_hour_index":900,"last_capture_hour_index":906,"captures":captures});
        let catalog = PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&timeline).unwrap()).unwrap();
        let relocated = PortableDesignCatalog::decode_a27_live_json(&catalog.encode_portable_json().unwrap()).unwrap();
        for hour in 900..=904 {
            let input = relocated.native_designer_inputs_at_hour(hour).unwrap();
            assert_eq!(input.hour, hour);
            assert_eq!(input.templates[0].design_id, "8:52");
            let records = relocated.records_at_hour(hour).unwrap();
            assert_eq!(&input.variants[0].record,
                records.iter().find(|row| row.design_id == "7:70").unwrap());
            assert_eq!(input.source_record_id, format!("a27-live-catalog:{}",
                timeline["captures"][(hour - 900) as usize]["snapshot_sha256"].as_str().unwrap()));
        }
        assert!(relocated.native_designer_inputs_at_hour(905).unwrap_err().contains("Torn"));
        assert_eq!(relocated.native_designer_inputs_at_hour(906).unwrap().templates[0].country_289a,
            DesignI32Observation::Measured { raw_i32: -1 });
        assert!(relocated.native_designer_inputs_at_hour(907).is_err());
    }

    #[test]
    fn v5_label_projection_keeps_both_exact_text_observations() {
        // Native empty is null+empty hex; an explicit empty string is also
        // accepted by the existing exact pair validator. Neither is missing.
        for (display, variant) in [
            (None, None),
            (Some(""), Some("")),
            (Some("  "), Some("\t")),
            (Some("Display"), Some("Variant")),
            (Some(""), Some("Variant")),
            (None, Some("Variant")),
            (Some(" Véhicule "), Some("車両\t")),
        ] {
            let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
            for (field, value) in [("display_name", display), ("variant_name", variant)] {
                raw["variants"][0]["definition"][field] = serde_json::json!(value);
                raw["variants"][0]["definition"][format!("{field}_utf8_hex")] =
                    value.unwrap_or("").as_bytes().iter().map(|b| format!("{b:02x}"))
                        .collect::<String>().into();
            }
            let catalog = PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).unwrap();
            let relocated = PortableDesignCatalog::decode_a27_live_json(&catalog.encode_portable_json().unwrap()).unwrap();
            let inputs = relocated.native_designer_inputs_at_hour(900).unwrap();
            let row = &inputs.variants[0];
            for (observed, value, field) in [(&row.display_name, display, "display_name"),
                (&row.variant_name, variant, "variant_name")] {
                assert_eq!(observed, &DesignTextObservation::Measured {
                    value: value.unwrap_or("").into(),
                    utf8_hex: raw["variants"][0]["definition"][format!("{field}_utf8_hex")]
                        .as_str().unwrap().into(),
                });
            }
            let merged = display.filter(|value| !value.trim().is_empty())
                .or_else(|| variant.filter(|value| !value.trim().is_empty()));
            assert_eq!(row.record.name.as_deref(), merged);
            assert!(relocated.native_designer_inputs_at_hour(901).is_err());
        }
    }

    #[test]
    fn v5_label_projection_refuses_missing_or_mismatched_original_pairs() {
        for field in ["display_name", "variant_name"] {
            for case in 0..4 {
                let mut raw: Value = serde_json::from_slice(&fixture()).unwrap();
                let definition = raw["variants"][0]["definition"].as_object_mut().unwrap();
                let hex_field = format!("{field}_utf8_hex");
                match case {
                    0 => { definition.remove(field); }
                    1 => { definition.remove(&hex_field); }
                    2 => { definition.insert(field.into(), Value::Null);
                        definition.insert(hex_field, "41".into()); }
                    _ => { definition.insert(field.into(), "A".into());
                        definition.insert(hex_field, "42".into()); }
                }
                assert!(PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&raw).unwrap()).is_err(),
                    "{field} case {case}");
            }
        }
    }

    #[test]
    fn v5_label_text_change_survives_delta_when_record_name_is_unchanged() {
        let baseline: Value = serde_json::from_slice(&fixture()).unwrap();
        let mut changed = baseline.clone();
        changed["authority"]["capture_hour_index"] = 901.into();
        changed["authority"]["current_date_raw"] = (DATE_EPOCH_RAW + 901).into();
        changed["variants"][0]["definition"]["variant_name"] = " Mk II ".into();
        changed["variants"][0]["definition"]["variant_name_utf8_hex"] = "204d6b20494920".into();
        let decode = |v: &Value| PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(v).unwrap()).unwrap();
        let first = decode(&baseline);
        let second = decode(&changed);
        let old = first.native_designer_inputs_at_hour(900).unwrap();
        let new = second.native_designer_inputs_at_hour(901).unwrap();
        assert_eq!(old.variants[0].record.name, new.variants[0].record.name);
        assert_ne!(old.variants[0].variant_name, new.variants[0].variant_name);
        let delta = second.native_row_delta_from(&first).unwrap();
        assert_eq!(delta["variant_upserts"].as_array().unwrap().len(), 1);
        assert!(delta["template_upserts"].as_array().unwrap().is_empty());
        let reconstructed = decode(&apply_a27_row_delta(&baseline, &delta).unwrap());
        let inputs = reconstructed.native_designer_inputs_at_hour(901).unwrap();
        assert_eq!(inputs.variants[0].display_name, old.variants[0].display_name);
        assert_eq!(inputs.variants[0].variant_name, DesignTextObservation::Measured {
            value: " Mk II ".into(), utf8_hex: "204d6b20494920".into(),
        });
    }

}
