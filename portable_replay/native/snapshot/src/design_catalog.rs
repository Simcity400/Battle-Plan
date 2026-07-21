//! Pointer-free A27 live design-catalog timeline carried once per portable
//! replay. The baseline and each temporal delta are authoritative only for
//! their explicitly sealed `capture_hour_index`.
//!
//! Domain 59 remains the authored game-file template/equipment-definition
//! artifact. It does not contain live stable ids, owners, lifecycle, modules,
//! upgrades, or UI order and is never relabelled as this catalog.

use crate::sha256::Sha256;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const A27_LIVE_CATALOG_SCHEMA: &str = "extractor.a27.live_heap_catalog.a729.v2";
pub const A27_LIVE_CATALOG_TIMELINE_SCHEMA: &str =
    "extractor.a27.live_heap_catalog_timeline.a729.v1";
pub const PORTABLE_DESIGN_CATALOG_SCHEMA: &str = "hoi4.portable-design-catalog.v3";
pub const PORTABLE_ARTIFACT_PATH: &str = "static/design_catalog_live/a27_live_catalog.json";
const DATE_EPOCH_RAW: u64 = 43_799_999;
const EXACT_A729_PROGRAM: &str = "hoi4.exe 1.19.2.0.a729";
const EXACT_A729_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayOrderSnapshot {
    pub source_record_id: String,
    pub completeness: FieldCompleteness,
    /// Exact engine-displayed design ids from first to last.
    pub ordered_design_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortableDesignCatalog {
    pub schema: String,
    pub source_record_id: String,
    pub source_sha256: String,
    /// Exact game hour at which the sealed full-replacement census was read.
    pub capture_hour_index: u64,
    /// Complete native A27 payload with only process-local relocation fields
    /// removed. This is the lossless portable authority; `records` is a
    /// deliberately smaller replay/UI projection.
    pub exact_native_catalog: Value,
    /// Ordered changes after the baseline census. Each delta is itself an
    /// authenticated complete-hour observation; tombstones are minted only by
    /// absence from the next coherent full replacement.
    pub temporal_deltas: Vec<PortableDesignCatalogDelta>,
    /// Complete live A27 catalog in its captured heap-scan order. This order is
    /// provenance only; it is never exposed as engine UI display order.
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
        if value.get("schema").and_then(Value::as_str)
            == Some(A27_LIVE_CATALOG_TIMELINE_SCHEMA)
        {
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
            if artifact
                .exact_native_catalog
                .get("schema")
                .and_then(Value::as_str)
                != Some(A27_LIVE_CATALOG_SCHEMA)
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
            || !raw.completeness.owner
            || !raw.completeness.ui_occurrences
        {
            return Err("A27 live catalog is not a complete full replacement".into());
        }
        validate_raw_catalog(&raw)?;
        let capture_hour_index = raw.authority.capture_hour_index;
        if raw
            .authority
            .current_date_raw
            .checked_sub(DATE_EPOCH_RAW)
            != Some(capture_hour_index)
        {
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
                || row.owner_tag.trim().is_empty()
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
                owner: Some(row.owner_tag),
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
                owner_completeness: FieldCompleteness::Complete,
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
                // A27 proves the common live runtime type 70. Subclassifying it
                // as ship/tank/aircraft would require a separate exact join to
                // authored definitions; generic EquipmentVariant is lossless.
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
            schema: A27_LIVE_CATALOG_SCHEMA.into(),
            source_record_id,
            source_sha256,
            capture_hour_index,
            exact_native_catalog,
            temporal_deltas: Vec::new(),
            records,
        })
    }

    fn decode_a27_timeline_value(value: Value) -> Result<Self, String> {
        let authority = value
            .get("authority")
            .and_then(Value::as_object)
            .ok_or_else(|| "A27 timeline authority is missing".to_string())?;
        if authority.get("program").and_then(Value::as_str)
            != Some(EXACT_A729_PROGRAM)
            || authority.get("program_sha256").and_then(Value::as_str)
                != Some(EXACT_A729_SHA256)
            || value.get("complete_sequence").and_then(Value::as_bool) != Some(true)
            || value.get("transport").and_then(Value::as_str)
                != Some("first_full_then_native_row_deltas")
            || value.get("fatal").is_some()
        {
            return Err("A27 timeline has unauthenticated/incomplete authority".into());
        }
        let captures = value
            .get("captures")
            .and_then(Value::as_array)
            .filter(|captures| !captures.is_empty())
            .ok_or_else(|| "A27 timeline contains no complete captures".to_string())?;
        if value.get("capture_count").and_then(Value::as_u64) != Some(captures.len() as u64)
            || value.get("first_capture_hour_index").and_then(Value::as_u64)
                != captures
                    .first()
                    .and_then(|capture| capture.get("capture_hour_index"))
                    .and_then(Value::as_u64)
            || value.get("last_capture_hour_index").and_then(Value::as_u64)
                != captures
                    .last()
                    .and_then(|capture| capture.get("capture_hour_index"))
                    .and_then(Value::as_u64)
        {
            return Err("A27 timeline summary disagrees with its capture sequence".into());
        }
        let total_rechecks = captures.iter().try_fold(0_u64, |total, capture| {
            capture
                .get("same_hour_rechecks")
                .and_then(Value::as_u64)
                .and_then(|count| total.checked_add(count))
        });
        if value.get("same_hour_recheck_count").and_then(Value::as_u64) != total_rechecks {
            return Err("A27 timeline recheck summary disagrees with its captures".into());
        }
        let decode_snapshot =
            |sequence: usize, capture: &Value, snapshot: &Value| -> Result<Self, String> {
            if capture.get("sequence").and_then(Value::as_u64) != Some(sequence as u64) {
                return Err(format!("A27 timeline capture {sequence} has a sequence gap"));
            }
            if capture.get("same_hour_rechecks").and_then(Value::as_u64) == Some(0)
                || capture.get("same_hour_rechecks").and_then(Value::as_u64).is_none()
            {
                return Err(format!(
                    "A27 timeline capture {sequence} lacks a same-hour mutation recheck"
                ));
            }
            let snapshot_bytes = serde_json::to_vec(&canonical_json_value(snapshot))
                .map_err(|error| format!("serialize A27 timeline capture {sequence}: {error}"))?;
            let snapshot_sha256 = Sha256::hex(&snapshot_bytes);
            if capture.get("snapshot_sha256").and_then(Value::as_str)
                != Some(snapshot_sha256.as_str())
            {
                return Err(format!("A27 timeline capture {sequence} hash mismatch"));
            }
            let recheck_hashes = capture
                .get("same_hour_recheck_sha256")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    format!("A27 timeline capture {sequence} lacks recheck hashes")
                })?;
            if capture.get("same_hour_rechecks").and_then(Value::as_u64)
                != Some(recheck_hashes.len() as u64)
                || recheck_hashes.iter().any(|hash| {
                    hash.as_str() != Some(snapshot_sha256.as_str())
                })
            {
                return Err(format!(
                    "A27 timeline capture {sequence} recheck hashes disagree"
                ));
            }
            let catalog = Self::decode_a27_live_json(&snapshot_bytes)?;
            if capture.get("capture_hour_index").and_then(Value::as_u64)
                != Some(catalog.capture_hour_index)
            {
                return Err(format!("A27 timeline capture {sequence} hour mismatch"));
            }
            Ok(catalog)
        };
        let mut previous_snapshot = captures[0]
            .get("snapshot")
            .cloned()
            .ok_or_else(|| "A27 timeline baseline has no full snapshot".to_string())?;
        if captures[0].get("delta").is_some() {
            return Err("A27 timeline baseline cannot be a delta".into());
        }
        let mut baseline = decode_snapshot(0, &captures[0], &previous_snapshot)?;
        let mut previous = baseline.clone();
        for (sequence, capture) in captures.iter().enumerate().skip(1) {
            if capture.get("snapshot").is_some() {
                return Err(format!(
                    "A27 timeline capture {sequence} repeats a full snapshot"
                ));
            }
            let delta = capture
                .get("delta")
                .ok_or_else(|| format!("A27 timeline capture {sequence} has no delta"))?;
            let current_snapshot = apply_raw_timeline_delta(&previous_snapshot, delta)?;
            let current = decode_snapshot(sequence, capture, &current_snapshot)?;
            if current.capture_hour_index != previous.capture_hour_index.saturating_add(1) {
                return Err(format!(
                    "A27 timeline capture {sequence} is not the next game hour"
                ));
            }
            baseline
                .temporal_deltas
                .push(build_temporal_delta(sequence as u64, &previous, &current)?);
            previous = current;
            previous_snapshot = current_snapshot;
        }
        baseline.validate_portable()?;
        Ok(baseline)
    }

    pub fn records_at_hour(&self, hour: u64) -> Result<Vec<DesignRecord>, String> {
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
                    ordered.push(
                        records
                            .remove(design_id)
                            .ok_or_else(|| {
                                format!(
                                    "A27 delta {} order references absent design {design_id}",
                                    delta.sequence
                                )
                            })?,
                    );
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
        Err(format!("A27 has no authenticated census for replay hour {hour}"))
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
        let catalog_last = self
            .temporal_deltas
            .last()
            .map(|delta| delta.capture_hour_index)
            .unwrap_or(self.capture_hour_index);
        if first != self.capture_hour_index
            || last != catalog_last
            || hours.len() != self.temporal_deltas.len().saturating_add(1)
            || hours
                .windows(2)
                .any(|pair| pair[1] != pair[0].saturating_add(1))
        {
            return Err(format!(
                "A27 catalog hours {}..{} do not exactly cover replay hours {first}..{last}",
                self.capture_hour_index, catalog_last
            ));
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
        validate_exact_native_catalog(&self.exact_native_catalog, self.capture_hour_index)?;
        let mut ids = BTreeSet::new();
        for record in &self.records {
            if record.design_id.trim().is_empty()
                || record.source_record_id.trim().is_empty()
                || !ids.insert(record.design_id.as_str())
                || record.definition_completeness != FieldCompleteness::Complete
                || record.owner_completeness != FieldCompleteness::Complete
                || record.lifecycle_completeness != FieldCompleteness::Complete
                || record.owner.as_deref().is_none_or(str::is_empty)
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
            return Err("portable A27 UI projection diverges from its exact native authority".into());
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

fn canonical_json_value(value: &Value) -> Value {
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

fn apply_raw_timeline_delta(previous: &Value, delta: &Value) -> Result<Value, String> {
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
                return Err(format!("A27 raw timeline duplicates/conflicts upsert {stable_id}"));
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

fn exact_rows(catalog: &PortableDesignCatalog, key: &str) -> Result<BTreeMap<String, Value>, String> {
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
                .is_none_or(|previous| {
                    !same_native_row_ignoring_catalog_ordinal(previous, row)
                })
                .then(|| row.clone())
        })
        .collect::<Vec<_>>();
    let template_upserts = current_templates
        .iter()
        .filter_map(|(id, row)| {
            previous_templates
                .get(id)
                .is_none_or(|previous| {
                    !same_native_row_ignoring_catalog_ordinal(previous, row)
                })
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
        record.source_record_id =
            format!("{source_record_id}:{family}:{}", record.design_id);
        for (ordinal, module) in record.modules.iter_mut().enumerate() {
            module.source_record_id =
                format!("{}:module:{ordinal}", record.source_record_id);
        }
        for (ordinal, stat) in record.stats.iter_mut().enumerate() {
            stat.source_record_id =
                format!("{}:stat:{ordinal}", record.source_record_id);
        }
        for (ordinal, upgrade) in record.upgrades.iter_mut().enumerate() {
            upgrade.source_record_id =
                format!("{}:upgrade:{ordinal}", record.source_record_id);
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
        return Err("portable A27 native authority contains process-local relocation fields".into());
    }
    if value.get("schema").and_then(Value::as_str) != Some(A27_LIVE_CATALOG_SCHEMA)
        || value.get("full_replacement").and_then(Value::as_bool) != Some(true)
    {
        return Err("portable A27 native authority has the wrong schema/replacement law".into());
    }
    let authority = value
        .get("authority")
        .and_then(Value::as_object)
        .ok_or_else(|| "portable A27 native authority is missing capture identity".to_string())?;
    if authority
        .get("capture_hour_index")
        .and_then(Value::as_u64)
        != Some(capture_hour_index)
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
        || !raw.completeness.owner
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
                || row.get("catalog_scan_ordinal").and_then(Value::as_u64)
                    != Some(ordinal as u64)
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
            || delta.capture_hour_index != previous_hour.saturating_add(1)
            || delta.source_record_id != format!("a27-live-catalog:{}", delta.source_sha256)
            || delta.source_sha256.len() != 64
            || !delta
                .source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || contains_relocation_field(&delta.native_frame_metadata)
        {
            return Err(format!("portable A27 delta {} identity is invalid", delta.sequence));
        }
        let authority = delta
            .native_frame_metadata
            .get("authority")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("portable A27 delta {} has no authority", delta.sequence))?;
        if authority
            .get("capture_hour_index")
            .and_then(Value::as_u64)
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
                return Err(format!("portable A27 delta {} duplicates an upsert", delta.sequence));
            }
        }
        for row in &delta.template_upserts {
            if !upsert_ids.insert(exact_row_id(row, 52)?) {
                return Err(format!("portable A27 delta {} duplicates an upsert", delta.sequence));
            }
        }
        let mut tombstones = BTreeSet::new();
        for id in delta
            .variant_tombstones
            .iter()
            .chain(delta.template_tombstones.iter())
        {
            if !tombstones.insert(id.clone())
                || upsert_ids.contains(id)
                || !ids.remove(id)
            {
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
            || delta.ordered_design_ids.iter().cloned().collect::<BTreeSet<_>>() != ids
            || delta.ordered_design_ids.len() != ids.len()
        {
            return Err(format!(
                "portable A27 delta {} projection/order diverges from native mutation",
                delta.sequence
            ));
        }
        let mut exact_frame = delta.native_frame_metadata.clone();
        let frame = exact_frame
            .as_object_mut()
            .ok_or_else(|| format!("portable A27 delta {} metadata is not an object", delta.sequence))?;
        let mut templates = Vec::new();
        let mut variants = Vec::new();
        for design_id in &delta.ordered_design_ids {
            let mut row = native_rows
                .get(design_id)
                .ok_or_else(|| format!("portable A27 delta {} order has no native row", delta.sequence))?
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
        let exact_bytes = serde_json::to_vec(&exact_frame)
            .map_err(|error| format!("serialize A27 delta {} exact frame: {error}", delta.sequence))?;
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
            return Err(format!("{label} has an invalid/duplicate placeholder ordinal"));
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
        return Err(format!("{label} does not account for every native pointer slot"));
    }
    Ok(())
}

fn validate_raw_catalog(raw: &RawCatalog) -> Result<(), String> {
    if raw.authority.program != EXACT_A729_PROGRAM
        || raw.authority.program_sha256 != EXACT_A729_SHA256
        || raw.authority.coherence != "double_read_all_roots_fields_and_membership"
        || raw.country_count != raw.countries.len() as u64
    {
        return Err("A27 live catalog lacks a whole-sweep seal/country census".into());
    }
    let mut country_tags = BTreeSet::new();
    for (ordinal, country) in raw.countries.iter().enumerate() {
        if country.registry_index != ordinal as u64
            || country.tag.trim().is_empty()
            || !country_tags.insert(country.tag.as_str())
        {
            return Err("A27 live catalog has an invalid country registry".into());
        }
    }
    for row in &raw.division_templates {
        let country = raw
            .countries
            .get(row.source_country_registry_index as usize)
            .ok_or_else(|| format!("A27 template {} has an invalid country source", row.id))?;
        if country.tag != row.source_country_tag
            || row.owner_tag != row.source_country_tag
            || row.definition.obsolete != row.obsolete
            || !utf8_hex_matches(row.definition.name.as_deref(), &row.definition.name_utf8_hex)
            || row.definition.regiment_grid_columns == 0
            || row.definition.regiment_grid_rows == 0
        {
            return Err(format!("A27 template {} has inconsistent native fields", row.id));
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
            return Err(format!("A27 variant {} has inconsistent native fields", row.id));
        }
        for (ordinal, module) in row.definition.modules.iter().enumerate() {
            if module.source_ordinal != ordinal as u32
                || module.module_token.trim().is_empty()
                || !utf8_hex_matches(
                    Some(&module.module_token),
                    &module.module_token_utf8_hex,
                )
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
                return Err(format!("A27 variant {} has an invalid cached-stat row", row.id));
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
                return Err(format!("A27 variant {} has an invalid UI occurrence", row.id));
            }
            let _ = occurrence.source_list_ordinal;
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
    records: Vec<DesignRecord>,
}

#[derive(Deserialize)]
struct RawCatalog {
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
    country_vector_ordinal: u64,
    definition: RawDivisionDefinition,
    definition_complete: bool,
    id: u32,
    obsolete: bool,
    owner_tag: String,
    source_country_registry_index: u64,
    source_country_tag: String,
    #[serde(rename = "type")]
    runtime_type: u32,
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
    source_occurrences: Vec<RawSourceOccurrence>,
    #[serde(rename = "type")]
    runtime_type: u32,
}

#[derive(Deserialize)]
struct RawSourceOccurrence {
    source_country_tag: String,
    source_country_registry_index: u64,
    source_list: String,
    source_list_ordinal: u64,
}

#[derive(Deserialize)]
struct RawVariantDefinition {
    archetype_token: String,
    archetype_token_utf8_hex: String,
    cached_stats: Vec<RawStat>,
    cached_stats_complete: bool,
    display_name: Option<String>,
    display_name_utf8_hex: String,
    equipment_token_raw: u32,
    modules: Vec<RawModule>,
    modules_complete: bool,
    upgrades: Vec<RawUpgrade>,
    upgrades_complete: bool,
    variant_name: Option<String>,
    variant_name_utf8_hex: String,
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
            "country_count": 1,
            "countries": [{ "registry_index": 0, "tag": "GER" }],
            "division_templates": [{
                "catalog_scan_ordinal": 0,
                "country_vector_ordinal": 0,
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
}
