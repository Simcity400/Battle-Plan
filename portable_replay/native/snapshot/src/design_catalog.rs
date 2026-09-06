use crate::sha256::Sha256;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const A27_LIVE_CATALOG_SCHEMA: &str = "extractor.a27.native_catalog.a729.v4";
pub const A27_LIVE_CATALOG_DESIGNER_SCHEMA: &str = "extractor.a27.native_designer_catalog.a729.v5";
pub const A27_LIVE_CATALOG_TIMELINE_SCHEMA: &str =
    "extractor.a27.live_heap_catalog_timeline.a729.v2";
pub const PORTABLE_DESIGN_CATALOG_SCHEMA: &str = "hoi4.portable-design-catalog.v4";
pub const PORTABLE_ARTIFACT_PATH: &str = "static/design_catalog_live/a27_live_catalog.json";
const DATE_EPOCH_RAW: u64 = 43_799_999;
const EXACT_A729_PROGRAM: &str = "hoi4.exe 1.19.2.0.a729";
const EXACT_A729_SHA256: &str = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const XUI13_CLASSIFIER_MANIFEST_SHA256: &str =
    "733bfeb07165754e30cec9d8395f244b0515f92bc431baff5d236b51b4f70df9";
const XUI13_CONTROLLER_MANIFEST_SHA256: &str =
    "8f50fd0c83ea637c3a9464c7061a7f6c118fda9195e4cccd88ae12a3abc765c3";

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
    pub design_id: String,
    pub catalog_scan_ordinal: u64,
    pub equipment_definition_token: String,
    pub equipment_token_raw: u32,
    pub item_icon_override: DesignItemIconObservation,
    pub serialized_variant_flag_2b9d: DesignByteObservation,
    pub record: DesignRecord,
    pub model: VariantModelInputs,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeDesignerInputs {
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

impl ObserverDesignerFamily {
    fn token(self) -> &'static str {
        match self {
            Self::Ship => "ship",
            Self::Tank => "tank",
            Self::Plane => "plane",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverDesignerCandidate {
    pub record: DesignRecord,
    pub designer_window_tokens: Vec<String>,
    pub icon_token: Option<String>,
    pub icon_authority: String,
    pub mission_mask: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverDesignerCountryFilterRow {
    pub country_tag: String,
    pub candidate_designer_window_tokens: Vec<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverDesignerCatalog {
    pub source_record_id: String,
    pub anchor_design_id: String,
    pub selected_country_tag: String,
    pub family: ObserverDesignerFamily,
    pub country_filter_rows: Vec<ObserverDesignerCountryFilterRow>,
    pub ordered: Vec<ObserverDesignerCandidate>,
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
            if artifact.source_schema != A27_LIVE_CATALOG_SCHEMA
                && artifact.source_schema != A27_LIVE_CATALOG_DESIGNER_SCHEMA
            {
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
        let mut exact_native_catalog = value.clone();
        strip_relocation_fields(&mut exact_native_catalog);
        let raw: RawCatalog = serde_json::from_value(value)
            .map_err(|error| format!("typed A27 live catalog projection: {error}"))?;
        if raw.schema != A27_LIVE_CATALOG_SCHEMA && raw.schema != A27_LIVE_CATALOG_DESIGNER_SCHEMA {
            return Err(format!(
                "A27 live catalog schema {:?} != {:?}",
                raw.schema, A27_LIVE_CATALOG_DESIGNER_SCHEMA
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
                        value: stat.raw_i32 as f64 / stat.scale as f64,
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

    #[allow(clippy::too_many_arguments)]
    pub fn observer_designer_catalog_at_hour(
        &self,
        hour: u64,
        anchor_design_id: &str,
        selected_country_tag: &str,
        family: ObserverDesignerFamily,
        our_filter_enabled: bool,
        foreign_filter_enabled: bool,
        obsolete_filter_enabled: bool,
        excluded_country_tags: &[String],
    ) -> Result<ObserverDesignerCatalog, String> {
        if self.schema != A27_LIVE_CATALOG_DESIGNER_SCHEMA {
            return Err(format!(
                "XUI13 requires {A27_LIVE_CATALOG_DESIGNER_SCHEMA}; recording carries {}",
                self.schema
            ));
        }
        if selected_country_tag.trim().is_empty() {
            return Err("XUI13 selected-country identity is empty".into());
        }
        let exact = self.exact_native_at_hour(hour)?;
        let raw: RawCatalog = serde_json::from_value(exact)
            .map_err(|error| format!("typed XUI13 native frame: {error}"))?;
        validate_raw_catalog(&raw)?;
        if !raw
            .countries
            .iter()
            .any(|country| country.tag == selected_country_tag)
        {
            return Err(format!(
                "XUI13 selected country {selected_country_tag:?} is absent from A27"
            ));
        }
        let mut excluded_country_set = BTreeSet::new();
        for tag in excluded_country_tags {
            if !raw.countries.iter().any(|country| country.tag == *tag)
                || !excluded_country_set.insert(tag.as_str())
            {
                return Err(format!(
                    "XUI13 excluded country filter {tag:?} is invalid/duplicate"
                ));
            }
        }
        let resolved_anchor_design_id = if anchor_design_id.trim().is_empty() {
            raw.variants.iter().find_map(|row| {
                row.definition
                    .designer_classification
                    .as_ref()
                    .filter(|classification| {
                        classification
                            .family_membership
                            .iter()
                            .any(|candidate| candidate == family.token())
                    })
                    .map(|_| format!("{}:{}", row.id, row.runtime_type))
            })
        } else {
            None
        };
        if anchor_design_id.trim().is_empty() && resolved_anchor_design_id.is_none() {
            if !excluded_country_set.is_empty() {
                return Err(
                    "XUI13 measured-empty family cannot select absent country-filter rows".into(),
                );
            }
            let source_record_id = self
                .temporal_deltas
                .iter()
                .find(|delta| delta.capture_hour_index == hour)
                .map(|delta| delta.source_record_id.clone())
                .unwrap_or_else(|| self.source_record_id.clone());
            return Ok(ObserverDesignerCatalog {
                source_record_id,
                anchor_design_id: String::new(),
                selected_country_tag: selected_country_tag.to_string(),
                family,
                country_filter_rows: Vec::new(),
                ordered: Vec::new(),
            });
        }
        let anchor_design_id = resolved_anchor_design_id
            .as_deref()
            .unwrap_or(anchor_design_id);
        let anchor = raw
            .variants
            .iter()
            .find(|row| format!("{}:{}", row.id, row.runtime_type) == anchor_design_id)
            .ok_or_else(|| format!("XUI13 anchor {anchor_design_id:?} is absent"))?;
        let anchor_classification = anchor
            .definition
            .designer_classification
            .as_ref()
            .ok_or_else(|| "XUI13 anchor lacks designer classification".to_string())?;
        if !anchor_classification
            .family_membership
            .iter()
            .any(|candidate| candidate == family.token())
        {
            return Err(format!(
                "XUI13 anchor {anchor_design_id:?} is not a {}-designer definition",
                family.token()
            ));
        }
        let context = raw
            .designer_controller_contexts
            .iter()
            .find(|context| context.equipment_token_raw == anchor.definition.equipment_token_raw)
            .ok_or_else(|| "XUI13 anchor has no designer controller context".to_string())?;
        let eligible = context
            .eligible_country_tags
            .get(family.token())
            .ok_or_else(|| "XUI13 controller context omits the selected family".to_string())?
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let admitted_countries = raw
            .countries
            .iter()
            .filter(|country| {
                country.tag == selected_country_tag || eligible.contains(country.tag.as_str())
            })
            .map(|country| country.registry_index)
            .collect::<BTreeSet<_>>();
        let mut occurrence_order = BTreeMap::new();
        for row in &raw.variants {
            for occurrence in &row.source_occurrences {
                if !admitted_countries.contains(&occurrence.source_country_registry_index) {
                    continue;
                }
                let list_ordinal = match occurrence.source_list.as_str() {
                    "A" => 0_u8,
                    "B" => 1_u8,
                    _ => return Err("XUI13 occurrence has an invalid source list".into()),
                };
                let key = (
                    occurrence.source_country_registry_index,
                    list_ordinal,
                    occurrence.source_list_ordinal,
                );
                if occurrence_order.insert(key, row).is_some() {
                    return Err("XUI13 occurrence order contains a duplicate native slot".into());
                }
            }
        }
        let mut filter_country_witnesses = BTreeMap::<String, Vec<Vec<String>>>::new();
        let mut filter_seen_designs = BTreeSet::new();
        if foreign_filter_enabled {
            for row in occurrence_order.values() {
                let design_id = format!("{}:{}", row.id, row.runtime_type);
                if !filter_seen_designs.insert(design_id.clone()) {
                    continue;
                }
                let classification = row
                    .definition
                    .designer_classification
                    .as_ref()
                    .ok_or_else(|| format!("XUI13 row {design_id} lacks classification"))?;
                if classification.controller_candidate_base
                    && classification
                        .family_membership
                        .iter()
                        .any(|candidate| candidate == family.token())
                    && row.selected_country_tag != selected_country_tag
                {
                    filter_country_witnesses
                        .entry(row.selected_country_tag.clone())
                        .or_default()
                        .push(classification.designer_window_tokens.clone());
                }
            }
        }
        let mut filter_countries = raw
            .countries
            .iter()
            .filter(|country| filter_country_witnesses.contains_key(country.tag.as_str()))
            .collect::<Vec<_>>();
        filter_countries.sort_by_key(|country| country.designer_pointer_order_ordinal);
        let country_filter_rows = filter_countries
            .into_iter()
            .map(|country| ObserverDesignerCountryFilterRow {
                country_tag: country.tag.clone(),
                candidate_designer_window_tokens: filter_country_witnesses
                    .remove(country.tag.as_str())
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        if excluded_country_set.iter().any(|tag| {
            !country_filter_rows
                .iter()
                .any(|candidate| candidate.country_tag == *tag)
        }) {
            return Err(
                "XUI13 country filter selects a country absent from the controller strip".into(),
            );
        }
        let records = self
            .records_at_hour(hour)?
            .into_iter()
            .map(|record| (record.design_id.clone(), record))
            .collect::<BTreeMap<_, _>>();
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::new();
        for row in occurrence_order.into_values() {
            let design_id = format!("{}:{}", row.id, row.runtime_type);
            if !seen.insert(design_id.clone()) {
                continue;
            }
            let classification = row
                .definition
                .designer_classification
                .as_ref()
                .ok_or_else(|| format!("XUI13 row {design_id} lacks classification"))?;
            if !classification.controller_candidate_base
                || !classification
                    .family_membership
                    .iter()
                    .any(|candidate| candidate == family.token())
            {
                continue;
            }
            let is_our = row.selected_country_tag == selected_country_tag;
            if (!our_filter_enabled && is_our)
                || (!foreign_filter_enabled && (row.origin_override_tag.is_some() || !is_our))
                || (!obsolete_filter_enabled && row.obsolete)
                || excluded_country_set.contains(row.selected_country_tag.as_str())
            {
                continue;
            }
            let visual = row
                .definition
                .designer_row_visual
                .as_ref()
                .ok_or_else(|| format!("XUI13 row {design_id} lacks row visual"))?;
            ordered.push(ObserverDesignerCandidate {
                record: records
                    .get(&design_id)
                    .ok_or_else(|| format!("XUI13 row {design_id} has no portable projection"))?
                    .clone(),
                designer_window_tokens: classification.designer_window_tokens.clone(),
                icon_token: visual.icon_token.clone(),
                icon_authority: visual.icon_authority.clone(),
                mission_mask: visual.mission_mask,
            });
        }
        let source_record_id = self
            .temporal_deltas
            .iter()
            .find(|delta| delta.capture_hour_index == hour)
            .map(|delta| delta.source_record_id.clone())
            .unwrap_or_else(|| self.source_record_id.clone());
        Ok(ObserverDesignerCatalog {
            source_record_id,
            anchor_design_id: anchor_design_id.to_string(),
            selected_country_tag: selected_country_tag.to_string(),
            family,
            country_filter_rows,
            ordered,
        })
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
                design_id, catalog_scan_ordinal: row.catalog_scan_ordinal,
                equipment_definition_token: row.definition.archetype_token,
                equipment_token_raw: row.definition.equipment_token_raw,
                item_icon_override: row.definition.item_icon_override,
                serialized_variant_flag_2b9d: row.definition.serialized_variant_flag_2b9d,
                record, model,
            })
        }).collect::<Result<Vec<_>, String>>()?;
        let source_record_id = self.temporal_deltas.iter()
            .find(|delta| delta.capture_hour_index == hour)
            .map(|delta| delta.source_record_id.clone()).unwrap_or_else(|| self.source_record_id.clone());
        Ok(NativeDesignerInputs {
            runtime_dlc_mask: raw.runtime_dlc_mask,
            source_record_id, hour, variants,
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
        if (self.schema != A27_LIVE_CATALOG_SCHEMA
            && self.schema != A27_LIVE_CATALOG_DESIGNER_SCHEMA)
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
    if contains_relocation_field(value) {
        return Err(
            "portable A27 native authority contains process-local relocation fields".into(),
        );
    }
    let schema = value.get("schema").and_then(Value::as_str);
    if (schema != Some(A27_LIVE_CATALOG_SCHEMA) && schema != Some(A27_LIVE_CATALOG_DESIGNER_SCHEMA))
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
    let mut designer_pointer_ordinals = BTreeSet::new();
    for (ordinal, country) in raw.countries.iter().enumerate() {
        if country.registry_index != ordinal as u64
            || country.tag.trim().is_empty()
            || !country_tags.insert(country.tag.as_str())
            || (raw.schema == A27_LIVE_CATALOG_DESIGNER_SCHEMA
                && (!designer_pointer_ordinals.insert(
                    country.designer_pointer_order_ordinal.ok_or_else(|| {
                        "A27 v3 country lacks designer pointer-order ordinal".to_string()
                    })?,
                )))
        {
            return Err("A27 live catalog has an invalid country registry".into());
        }
    }
    if raw.schema == A27_LIVE_CATALOG_DESIGNER_SCHEMA
        && designer_pointer_ordinals != (0..raw.countries.len() as u64).collect::<BTreeSet<_>>()
    {
        return Err("A27 v3 country pointer-order ordinals are not a permutation".into());
    }
    if raw.schema == A27_LIVE_CATALOG_DESIGNER_SCHEMA {
        let registry_order = raw
            .countries
            .iter()
            .map(|country| country.tag.as_str())
            .collect::<Vec<_>>();
        let registry_tags = registry_order.iter().copied().collect::<BTreeSet<_>>();
        let expected_families = ["plane", "ship", "tank"]
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut context_tokens = BTreeSet::new();
        for context in &raw.designer_controller_contexts {
            if context.equipment_token_raw == 0
                || !context_tokens.insert(context.equipment_token_raw)
                || context.authority != "exact_a729_equipment_designer_country_table"
                || context.controller_manifest_sha256 != XUI13_CONTROLLER_MANIFEST_SHA256
                || context.country_registry_count != raw.country_count
                || context.table_values_sha256.len() != 64
                || !context
                    .table_values_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                || context
                    .threshold_raw
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>()
                    != expected_families
                || context
                    .eligible_country_tags
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>()
                    != expected_families
            {
                return Err("A27 designer controller context is incomplete".into());
            }
            for tags in context.eligible_country_tags.values() {
                if tags.iter().any(|tag| !registry_tags.contains(tag.as_str()))
                    || tags.windows(2).any(|pair| {
                        registry_order
                            .iter()
                            .position(|tag| *tag == pair[0].as_str())
                            .zip(
                                registry_order
                                    .iter()
                                    .position(|tag| *tag == pair[1].as_str()),
                            )
                            .is_none_or(|(left, right)| left >= right)
                    })
                {
                    return Err("A27 designer eligibility is not registry ordered".into());
                }
            }
        }
        if context_tokens.is_empty() {
            return Err("A27 designer controller context census is empty".into());
        }
    } else if !raw.designer_controller_contexts.is_empty() {
        return Err("legacy A27 schema contains unversioned designer contexts".into());
    }
    if raw.completeness.owner != raw.division_templates.iter().all(|row|row.owner_tag.is_some()) {
        return Err("A27 owner completeness disagrees with measured template membership".into());
    }
    let mut template_slots = BTreeSet::new();
    for row in &raw.division_templates {
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
        if raw.schema == A27_LIVE_CATALOG_DESIGNER_SCHEMA {
            let classification = row
                .definition
                .designer_classification
                .as_ref()
                .ok_or_else(|| format!("A27 variant {} lacks designer classification", row.id))?;
            let visual = row
                .definition
                .designer_row_visual
                .as_ref()
                .ok_or_else(|| format!("A27 variant {} lacks designer row visual", row.id))?;
            let families = classification
                .family_membership
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            let valid_families = ["ship", "tank", "plane"]
                .into_iter()
                .collect::<BTreeSet<_>>();
            if classification.authority != "exact_a729_equipment_definition_classifiers"
                || classification.classifier_manifest_sha256 != XUI13_CLASSIFIER_MANIFEST_SHA256
                || families.len() != classification.family_membership.len()
                || !families.is_subset(&valid_families)
                || classification.definition_chain_depth == 0
                || classification.definition_chain_depth as usize
                    != classification.normalized_category_masks_hex.len()
                || classification.definition_chain_depth as usize
                    != classification.designer_window_tokens.len()
                || classification
                    .designer_window_tokens
                    .iter()
                    .any(|token| token.trim().is_empty())
                || classification
                    .normalized_category_masks_hex
                    .iter()
                    .any(|mask| {
                        mask.len() != 16 || !mask.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                || classification.controller_availability_mask_hex.len() != 16
                || !classification
                    .controller_availability_mask_hex
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                || visual.authority != "exact_a729_designer_equipment_variant_item"
            {
                return Err(format!(
                    "A27 variant {} has invalid designer authority",
                    row.id
                ));
            }
            match visual.icon_entry_kind {
                0 if visual
                    .icon_token
                    .as_deref()
                    .is_some_and(|token| !token.is_empty()) =>
                {
                    if visual.icon_authority != "embedded_string"
                        || !utf8_hex_matches(
                            visual.icon_token.as_deref(),
                            &visual.icon_token_utf8_hex,
                        )
                    {
                        return Err(format!("A27 variant {} has invalid embedded icon", row.id));
                    }
                }
                0 => {
                    if visual.icon_authority != "item_database_fallback"
                        || visual.icon_token.is_some()
                        || !visual.icon_token_utf8_hex.is_empty()
                    {
                        return Err(format!("A27 variant {} has invalid fallback icon", row.id));
                    }
                }
                1 => {
                    if visual.icon_authority != "embedded_resource_pointer_unportable"
                        || visual.icon_token.is_some()
                        || !visual.icon_token_utf8_hex.is_empty()
                    {
                        return Err(format!("A27 variant {} has invalid resource icon", row.id));
                    }
                }
                _ => {
                    return Err(format!("A27 variant {} has invalid icon kind", row.id));
                }
            }
            if classification.controller_candidate_base
                && !families.is_empty()
                && (visual.icon_entry_kind != 0
                    || visual.icon_authority != "embedded_string"
                    || visual
                        .icon_token
                        .as_deref()
                        .is_none_or(|token| token.is_empty()))
            {
                return Err(format!(
                    "A27 variant {} has a non-portable controller-visible icon",
                    row.id
                ));
            }
            let _ = (
                classification.generic_equipment_predicate,
                classification.dlc_feature_bits_raw,
                classification.controller_candidate_base,
                visual.mission_mask,
            );
        } else if row.definition.designer_classification.is_some()
            || row.definition.designer_row_visual.is_some()
        {
            return Err(format!(
                "legacy A27 variant {} contains unversioned designer fields",
                row.id
            ));
        }
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
    if raw.schema == A27_LIVE_CATALOG_DESIGNER_SCHEMA {
        let variant_tokens = raw
            .variants
            .iter()
            .map(|row| row.definition.equipment_token_raw)
            .collect::<BTreeSet<_>>();
        let context_tokens = raw
            .designer_controller_contexts
            .iter()
            .map(|context| context.equipment_token_raw)
            .collect::<BTreeSet<_>>();
        if variant_tokens != context_tokens {
            return Err("A27 designer contexts do not exactly cover variant definitions".into());
        }
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
    #[serde(default)]
    designer_controller_contexts: Vec<RawDesignerControllerContext>,
    division_templates: Vec<RawDivisionTemplate>,
    variants: Vec<RawVariant>,
}

#[derive(Clone, Deserialize)]
struct RawDesignerControllerContext {
    equipment_token_raw: u32,
    authority: String,
    controller_manifest_sha256: String,
    country_registry_count: u64,
    table_values_sha256: String,
    threshold_raw: BTreeMap<String, i64>,
    eligible_country_tags: BTreeMap<String, Vec<String>>,
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
    explicit_entity: DesignTextObservation,
    item_icon_override: DesignItemIconObservation,
    serialized_variant_flag_2b9d: DesignByteObservation,
    archetype_token: String,
    archetype_token_utf8_hex: String,
    cached_stats: Vec<RawStat>,
    cached_stats_complete: bool,
    display_name: Option<String>,
    display_name_utf8_hex: String,
    #[serde(default)]
    designer_classification: Option<RawDesignerClassification>,
    #[serde(default)]
    designer_row_visual: Option<RawDesignerRowVisual>,
    equipment_token_raw: u32,
    modules: Vec<RawModule>,
    modules_complete: bool,
    upgrades: Vec<RawUpgrade>,
    upgrades_complete: bool,
    variant_name: Option<String>,
    variant_name_utf8_hex: String,
}

#[derive(Clone, Deserialize)]
struct RawDesignerClassification {
    authority: String,
    classifier_manifest_sha256: String,
    family_membership: Vec<String>,
    generic_equipment_predicate: bool,
    definition_chain_depth: u64,
    designer_window_tokens: Vec<String>,
    dlc_feature_bits_raw: u64,
    controller_availability_mask_hex: String,
    controller_candidate_base: bool,
    normalized_category_masks_hex: Vec<String>,
}

#[derive(Clone, Deserialize)]
struct RawDesignerRowVisual {
    authority: String,
    mission_mask: u32,
    icon_entry_kind: u32,
    icon_authority: String,
    icon_token: Option<String>,
    icon_token_utf8_hex: String,
}

#[derive(Deserialize)]
struct RawModule {
    module_definition_handle: u32,
    module_token: String,
    module_token_utf8_hex: String,
    slot_handle: u64,
    source_ordinal: u32,
}

#[derive(Deserialize)]
struct RawStat {
    raw_i32: i64,
    scale: i64,
    source_ordinal: u32,
    stat_id: String,
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
                    "archetype_token": "light_tank_chassis_1",
                    "archetype_token_utf8_hex": "6c696768745f74616e6b5f636861737369735f31",
                    "cached_stats": [{
                        "raw_i32": 250000,
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

    fn designer_fixture() -> Vec<u8> {
        let mut value: Value = serde_json::from_slice(&fixture()).unwrap();
        value["schema"] = A27_LIVE_CATALOG_DESIGNER_SCHEMA.into();
        value["country_count"] = 3.into();
        value["countries"] = serde_json::json!([
            {"registry_index": 0, "tag": "GER", "designer_pointer_order_ordinal": 2},
            {"registry_index": 1, "tag": "ITA", "designer_pointer_order_ordinal": 1},
            {"registry_index": 2, "tag": "USA", "designer_pointer_order_ordinal": 0}
        ]);
        value["designer_controller_contexts"] = serde_json::json!([{
            "equipment_token_raw": 71,
            "authority": "exact_a729_equipment_designer_country_table",
            "controller_manifest_sha256": XUI13_CONTROLLER_MANIFEST_SHA256,
            "country_registry_count": 3,
            "table_values_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
            "threshold_raw": {"plane": 0, "ship": 0, "tank": 0},
            "eligible_country_tags": {
                "plane": ["ITA", "USA"],
                "ship": ["ITA", "USA"],
                "tank": ["ITA", "USA"]
            }
        }]);
        let base = value["variants"][0].clone();
        let mut our = base.clone();
        our["id"] = 7.into();
        our["catalog_scan_ordinal"] = 0.into();
        our["first_ui_discovery_ordinal"] = 0.into();
        our["source_occurrences"] = serde_json::json!([{
            "source_country_tag": "GER",
            "source_country_registry_index": 0,
            "source_list": "A",
            "source_list_ordinal": 0
        }]);
        let mut foreign = base.clone();
        foreign["id"] = 9.into();
        foreign["catalog_scan_ordinal"] = 1.into();
        foreign["creator_tag"] = "ITA".into();
        foreign["owner_tag"] = "ITA".into();
        foreign["selected_country_tag"] = "ITA".into();
        foreign["first_ui_discovery_ordinal"] = 1.into();
        foreign["source_occurrences"] = serde_json::json!([
            {
                "source_country_tag": "GER",
                "source_country_registry_index": 0,
                "source_list": "A",
                "source_list_ordinal": 1
            },
            {
                "source_country_tag": "ITA",
                "source_country_registry_index": 1,
                "source_list": "B",
                "source_list_ordinal": 0
            }
        ]);
        let mut obsolete = base;
        obsolete["id"] = 10.into();
        obsolete["catalog_scan_ordinal"] = 2.into();
        obsolete["creator_tag"] = "USA".into();
        obsolete["owner_tag"] = "USA".into();
        obsolete["selected_country_tag"] = "USA".into();
        obsolete["obsolete"] = true.into();
        obsolete["first_ui_discovery_ordinal"] = 2.into();
        obsolete["source_occurrences"] = serde_json::json!([{
            "source_country_tag": "USA",
            "source_country_registry_index": 2,
            "source_list": "A",
            "source_list_ordinal": 0
        }]);
        for row in [&mut our, &mut foreign, &mut obsolete] {
            row["definition"]["designer_classification"] = serde_json::json!({
                "authority": "exact_a729_equipment_definition_classifiers",
                "classifier_manifest_sha256": XUI13_CLASSIFIER_MANIFEST_SHA256,
                "family_membership": ["ship", "tank", "plane"],
                "generic_equipment_predicate": false,
                "definition_chain_depth": 1,
                "designer_window_tokens": ["light_tank_chassis_1"],
                "dlc_feature_bits_raw": 16,
                "controller_availability_mask_hex": "00000080004001c0",
                "controller_candidate_base": true,
                "normalized_category_masks_hex": ["0000000000000004"]
            });
            row["definition"]["designer_row_visual"] = serde_json::json!({
                "authority": "exact_a729_designer_equipment_variant_item",
                "mission_mask": 5,
                "icon_entry_kind": 0,
                "icon_authority": "embedded_string",
                "icon_token": "GFX_test_equipment",
                "icon_token_utf8_hex": "4746585f746573745f65717569706d656e74"
            });
        }
        value["variants"] = Value::Array(vec![our, foreign, obsolete]);
        serde_json::to_vec(&value).unwrap()
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
    fn xui13_reconstructs_native_country_list_order_dedup_and_filters() {
        let catalog = PortableDesignCatalog::decode_a27_live_json(&designer_fixture()).unwrap();
        let all = catalog
            .observer_designer_catalog_at_hour(
                900,
                "7:70",
                "GER",
                ObserverDesignerFamily::Tank,
                true,
                true,
                true,
                &[],
            )
            .unwrap();
        assert_eq!(
            all.ordered
                .iter()
                .map(|row| row.record.design_id.as_str())
                .collect::<Vec<_>>(),
            ["7:70", "9:70", "10:70"]
        );
        assert_eq!(
            all.country_filter_rows
                .iter()
                .map(|row| row.country_tag.as_str())
                .collect::<Vec<_>>(),
            ["USA", "ITA"]
        );
        assert_eq!(
            all.ordered[0].icon_token.as_deref(),
            Some("GFX_test_equipment")
        );
        assert_eq!(all.ordered[0].mission_mask, 5);

        let ours = catalog
            .observer_designer_catalog_at_hour(
                900,
                "7:70",
                "GER",
                ObserverDesignerFamily::Tank,
                true,
                false,
                true,
                &[],
            )
            .unwrap();
        assert_eq!(
            ours.ordered
                .iter()
                .map(|row| row.record.design_id.as_str())
                .collect::<Vec<_>>(),
            ["7:70"]
        );
        assert!(ours.country_filter_rows.is_empty());
        let active_foreign = catalog
            .observer_designer_catalog_at_hour(
                900,
                "7:70",
                "GER",
                ObserverDesignerFamily::Tank,
                false,
                true,
                false,
                &[],
            )
            .unwrap();
        assert_eq!(
            active_foreign
                .ordered
                .iter()
                .map(|row| row.record.design_id.as_str())
                .collect::<Vec<_>>(),
            ["9:70"]
        );
        let without_usa = catalog
            .observer_designer_catalog_at_hour(
                900,
                "7:70",
                "GER",
                ObserverDesignerFamily::Tank,
                true,
                true,
                true,
                &["USA".into()],
            )
            .unwrap();
        assert_eq!(
            without_usa
                .ordered
                .iter()
                .map(|row| row.record.design_id.as_str())
                .collect::<Vec<_>>(),
            ["7:70", "9:70"]
        );
    }

    #[test]
    fn xui13_typed_route_refuses_legacy_v2_without_breaking_legacy_decode() {
        let catalog = PortableDesignCatalog::decode_a27_live_json(&fixture()).unwrap();
        assert_eq!(catalog.records_at_hour(900).unwrap().len(), 2);
        let error = catalog
            .observer_designer_catalog_at_hour(
                900,
                "7:70",
                "GER",
                ObserverDesignerFamily::Tank,
                true,
                true,
                true,
                &[],
            )
            .unwrap_err();
        assert!(error.contains(A27_LIVE_CATALOG_DESIGNER_SCHEMA));
    }

    #[test]
    fn xui13_distinguishes_complete_measured_empty_from_missing_anchor() {
        let mut value: Value = serde_json::from_slice(&designer_fixture()).unwrap();
        for row in value["variants"].as_array_mut().unwrap() {
            row["definition"]["designer_classification"]["family_membership"] =
                serde_json::json!(["tank"]);
        }
        let catalog =
            PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&value).unwrap())
                .unwrap();

        let measured_empty = catalog
            .observer_designer_catalog_at_hour(
                900,
                "",
                "GER",
                ObserverDesignerFamily::Ship,
                true,
                true,
                true,
                &[],
            )
            .unwrap();
        assert!(measured_empty.ordered.is_empty());
        assert!(measured_empty.country_filter_rows.is_empty());
        assert!(measured_empty.anchor_design_id.is_empty());

        let resolved_anchor = catalog
            .observer_designer_catalog_at_hour(
                900,
                "",
                "GER",
                ObserverDesignerFamily::Tank,
                true,
                true,
                true,
                &[],
            )
            .unwrap();
        assert_eq!(resolved_anchor.anchor_design_id, "7:70");
        assert!(!resolved_anchor.ordered.is_empty());
    }

    #[test]
    fn xui13_refuses_an_anchor_from_a_different_designer_family() {
        let mut value: Value = serde_json::from_slice(&designer_fixture()).unwrap();
        value["variants"][0]["definition"]["designer_classification"]["family_membership"] =
            serde_json::json!(["ship"]);
        let catalog =
            PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&value).unwrap())
                .unwrap();
        let error = catalog
            .observer_designer_catalog_at_hour(
                900,
                "7:70",
                "GER",
                ObserverDesignerFamily::Tank,
                true,
                true,
                true,
                &[],
            )
            .unwrap_err();
        assert!(error.contains("not a tank-designer definition"));
    }

    #[test]
    fn xui13_v3_refuses_nonportable_icons_before_controller_admission() {
        let mut value: Value = serde_json::from_slice(&designer_fixture()).unwrap();
        let visual = &mut value["variants"][0]["definition"]["designer_row_visual"];
        visual["icon_token"] = Value::Null;
        visual["icon_token_utf8_hex"] = "".into();
        visual["icon_authority"] = "item_database_fallback".into();
        assert!(
            PortableDesignCatalog::decode_a27_live_json(&serde_json::to_vec(&value).unwrap())
                .unwrap_err()
                .contains("non-portable controller-visible icon")
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
}
