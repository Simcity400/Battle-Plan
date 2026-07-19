//! Pointer-free A27 live design catalog carried once per portable replay.
//!
//! Domain 59 remains the authored game-file template/equipment-definition
//! artifact. It does not contain live stable ids, owners, lifecycle, modules,
//! upgrades, or UI order and is never relabelled as this catalog.

use crate::sha256::Sha256;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const A27_LIVE_CATALOG_SCHEMA: &str = "extractor.a27.live_heap_catalog.a729.v2";
pub const PORTABLE_DESIGN_CATALOG_SCHEMA: &str = "hoi4.portable-design-catalog.v1";
pub const PORTABLE_ARTIFACT_PATH: &str = "static/design_catalog_live/a27_live_catalog.json";

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
    /// Complete live A27 catalog in its captured heap-scan order. This order is
    /// provenance only; it is never exposed as engine UI display order.
    pub records: Vec<DesignRecord>,
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
}

impl PortableDesignCatalog {
    pub fn decode_a27_live_json(bytes: &[u8]) -> Result<Self, String> {
        let value = crate::parse_json_strict(bytes)
            .map_err(|error| format!("strict A27 live catalog parse: {error}"))?;
        if value.get("schema").and_then(serde_json::Value::as_str)
            == Some(PORTABLE_DESIGN_CATALOG_SCHEMA)
        {
            let artifact: PortableArtifact = serde_json::from_value(value)
                .map_err(|error| format!("typed portable A27 catalog projection: {error}"))?;
            if artifact.source_schema != A27_LIVE_CATALOG_SCHEMA {
                return Err("portable A27 catalog source schema mismatch".into());
            }
            let catalog = Self {
                schema: artifact.source_schema,
                source_record_id: artifact.source_record_id,
                source_sha256: artifact.source_sha256,
                records: artifact.records,
            };
            catalog.validate_portable()?;
            return Ok(catalog);
        }
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
            records,
        })
    }

    pub fn encode_portable_json(&self) -> Result<Vec<u8>, String> {
        self.validate_portable()?;
        serde_json::to_vec(&PortableArtifact {
            schema: PORTABLE_DESIGN_CATALOG_SCHEMA.to_string(),
            source_schema: self.schema.clone(),
            source_record_id: self.source_record_id.clone(),
            source_sha256: self.source_sha256.clone(),
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
        Ok(())
    }

    pub fn get(&self, design_id: &str) -> Option<&DesignRecord> {
        self.records
            .iter()
            .find(|record| record.design_id == design_id)
    }
}

#[derive(Serialize, Deserialize)]
struct PortableArtifact {
    schema: String,
    source_schema: String,
    source_record_id: String,
    source_sha256: String,
    records: Vec<DesignRecord>,
}

#[derive(Deserialize)]
struct RawCatalog {
    schema: String,
    full_replacement: bool,
    completeness: RawCompleteness,
    division_templates: Vec<RawDivisionTemplate>,
    variants: Vec<RawVariant>,
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
    definition: RawDivisionDefinition,
    definition_complete: bool,
    id: u32,
    obsolete: bool,
    owner_tag: String,
    #[serde(rename = "type")]
    runtime_type: u32,
}

#[derive(Deserialize)]
struct RawDivisionDefinition {
    name: Option<String>,
}

#[derive(Deserialize)]
struct RawVariant {
    catalog_scan_ordinal: u64,
    definition: RawVariantDefinition,
    definition_complete: bool,
    id: u32,
    obsolete: bool,
    owner_tag: String,
    #[serde(rename = "type")]
    runtime_type: u32,
}

#[derive(Deserialize)]
struct RawVariantDefinition {
    archetype_token: String,
    cached_stats: Vec<RawStat>,
    cached_stats_complete: bool,
    display_name: Option<String>,
    modules: Vec<RawModule>,
    modules_complete: bool,
    upgrades: Vec<RawUpgrade>,
    upgrades_complete: bool,
    variant_name: Option<String>,
}

#[derive(Deserialize)]
struct RawModule {
    module_token: String,
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
    source_ordinal: u32,
    upgrade_handle: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema": A27_LIVE_CATALOG_SCHEMA,
            "full_replacement": true,
            "completeness": {
                "definitions": true,
                "lifecycle": true,
                "membership": true,
                "owner": true,
                "ui_occurrences": true
            },
            "division_templates": [{
                "catalog_scan_ordinal": 0,
                "definition": { "name": "Infantry" },
                "definition_complete": true,
                "id": 8,
                "obsolete": false,
                "owner_tag": "GER",
                "type": 52
            }],
            "variants": [{
                "catalog_scan_ordinal": 0,
                "definition": {
                    "archetype_token": "light_tank_chassis_1",
                    "cached_stats": [{
                        "raw_i32": 250000,
                        "scale": 100000,
                        "source_ordinal": 0,
                        "stat_id": "armor"
                    }],
                    "cached_stats_complete": true,
                    "display_name": "Panzer",
                    "modules": [{
                        "module_token": "tank_gasoline_engine",
                        "slot_handle": 91,
                        "source_ordinal": 0
                    }],
                    "modules_complete": true,
                    "upgrades": [{
                        "level": 2,
                        "source_ordinal": 0,
                        "upgrade_handle": 44
                    }],
                    "upgrades_complete": true,
                    "variant_name": null
                },
                "definition_complete": true,
                "id": 7,
                "obsolete": false,
                "owner_tag": "GER",
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
        assert!(!serde_json::to_string(&catalog).unwrap().contains("pointer"));
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
}
