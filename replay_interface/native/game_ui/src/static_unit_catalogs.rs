
use replay_core::{Leaf, PackagedStaticArtifact, WorldFrame};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const TEMPLATES_EQUIPMENT_ARTIFACT: &str = "templates_equipment_catalog";
pub const SHIP_DEFINITIONS_ARTIFACT: &str = "ship_definitions_static";

const TEMPLATES_EQUIPMENT_SCHEMA: &str =
    "templates_equipment_catalog_live.static_artifact.v1";
const SHIP_DEFINITIONS_SCHEMA: &str = "ship_definitions_static.static_artifact.v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticCatalogIdentity {
    pub artifact_name: String,
    pub container_path: String,
    pub payload_sha256: String,
    pub content_manifest_digest_sha256: String,
}

impl StaticCatalogIdentity {
    fn from_artifact(artifact: &PackagedStaticArtifact) -> Self {
        Self {
            artifact_name: artifact.name().to_string(),
            container_path: artifact.path().to_string(),
            payload_sha256: artifact.sha256().to_string(),
            content_manifest_digest_sha256: artifact
                .content_manifest_digest_sha256()
                .to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StaticDivisionTemplate {
    pub name: String,
    pub division_names_group: Option<String>,
    pub source: String,
    pub source_sha256: String,
    pub ordinal_in_file: u64,
    pub regiment_count: u64,
    pub support_count: u64,
    pub raw: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StaticDefinition {
    pub token: String,
    pub wrapper_token: String,
    pub source: String,
    pub source_sha256: String,
    pub fields: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TemplatesEquipmentCatalog {
    pub identity: StaticCatalogIdentity,
    pub division_templates: Vec<StaticDivisionTemplate>,
    pub equipment_definitions: BTreeMap<String, Vec<StaticDefinition>>,
    pub unit_definitions: BTreeMap<String, Vec<StaticDefinition>>,
    templates_by_name: BTreeMap<String, Vec<usize>>,
}

impl TemplatesEquipmentCatalog {
    pub fn templates_named(&self, name: &str) -> impl Iterator<Item = &StaticDivisionTemplate> {
        self.templates_by_name
            .get(name)
            .into_iter()
            .flatten()
            .map(|index| &self.division_templates[*index])
    }

    pub fn equipment_named(&self, token: &str) -> &[StaticDefinition] {
        self.equipment_definitions
            .get(token)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn unit_named(&self, token: &str) -> &[StaticDefinition] {
        self.unit_definitions
            .get(token)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShipHullDefinition {
    pub id: String,
    pub archetype: Option<String>,
    pub localised_name: Option<String>,
    pub source_file: String,
    pub source_layer: String,
    pub default_modules: BTreeMap<String, String>,
    pub stats: BTreeMap<String, Value>,
    pub raw: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShipModuleDefinition {
    pub id: String,
    pub category: String,
    pub localised_name: Option<String>,
    pub source_file: String,
    pub source_layer: String,
    pub add_stats: BTreeMap<String, String>,
    pub multiply_stats: BTreeMap<String, String>,
    pub raw: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShipDefinitionsCatalog {
    pub identity: StaticCatalogIdentity,
    pub hulls: BTreeMap<String, ShipHullDefinition>,
    pub modules: BTreeMap<String, ShipModuleDefinition>,
}

impl ShipDefinitionsCatalog {
    pub fn hull(&self, definition: &str) -> Option<&ShipHullDefinition> {
        self.hulls.get(definition)
    }

    pub fn module(&self, token: &str) -> Option<&ShipModuleDefinition> {
        self.modules.get(token)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StaticUnitCatalogs {
    pub templates_equipment: TemplatesEquipmentCatalog,
    pub ships: ShipDefinitionsCatalog,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StaticUnitFrameBindings {
    pub measured_template_names: usize,
    pub authored_template_matches: usize,
    pub measured_equipment_archetypes: usize,
    pub authored_equipment_matches: usize,
    pub measured_ship_definitions: usize,
    pub authored_ship_hull_matches: usize,
    pub unresolved_template_names: BTreeSet<String>,
    pub unresolved_equipment_archetypes: BTreeSet<String>,
    pub unresolved_ship_definitions: BTreeSet<String>,
}

impl StaticUnitCatalogs {
    pub fn bind_frame(&self, frame: &WorldFrame) -> StaticUnitFrameBindings {
        let mut out = StaticUnitFrameBindings::default();
        for template in &frame.designs().division_templates {
            let Some(name) = template.name.measured() else {
                continue;
            };
            out.measured_template_names += 1;
            if self.templates_equipment.templates_named(name).next().is_some() {
                out.authored_template_matches += 1;
            } else {
                out.unresolved_template_names.insert(name.clone());
            }
        }

        for variant in frame.designs().equipment_variants.values() {
            let Some(archetype) = variant.archetype.measured() else {
                continue;
            };
            out.measured_equipment_archetypes += 1;
            if self
                .templates_equipment
                .equipment_named(archetype)
                .is_empty()
            {
                out.unresolved_equipment_archetypes
                    .insert(archetype.clone());
            } else {
                out.authored_equipment_matches += 1;
            }
        }

        for fleet in frame.fleets() {
            let Leaf::Measured(ships) = &fleet.ships else {
                continue;
            };
            for ship in ships {
                let Some(definition) = ship.definition.measured() else {
                    continue;
                };
                out.measured_ship_definitions += 1;
                if self.ships.hull(definition).is_some() {
                    out.authored_ship_hull_matches += 1;
                } else {
                    out.unresolved_ship_definitions.insert(definition.clone());
                }
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticUnitCatalogError {
    message: String,
}

impl StaticUnitCatalogError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for StaticUnitCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StaticUnitCatalogError {}

pub fn admit_static_unit_catalogs(
    templates_equipment: Option<&PackagedStaticArtifact>,
    ship_definitions: Option<&PackagedStaticArtifact>,
) -> Result<StaticUnitCatalogs, StaticUnitCatalogError> {
    let templates_equipment = templates_equipment.ok_or_else(|| {
        StaticUnitCatalogError::new(format!(
            "recording omits required static artifact {TEMPLATES_EQUIPMENT_ARTIFACT}"
        ))
    })?;
    let ship_definitions = ship_definitions.ok_or_else(|| {
        StaticUnitCatalogError::new(format!(
            "recording omits required static artifact {SHIP_DEFINITIONS_ARTIFACT}"
        ))
    })?;
    Ok(StaticUnitCatalogs {
        templates_equipment: parse_templates_equipment(templates_equipment)?,
        ships: parse_ship_definitions(ship_definitions)?,
    })
}

fn parse_templates_equipment(
    artifact: &PackagedStaticArtifact,
) -> Result<TemplatesEquipmentCatalog, StaticUnitCatalogError> {
    require_artifact_name(artifact, TEMPLATES_EQUIPMENT_ARTIFACT)?;
    let root_value = parse_root(artifact, TEMPLATES_EQUIPMENT_SCHEMA)?;
    let root = root_value
        .as_object()
        .expect("parse_root validated an object root");
    let catalog = object_field(root, "catalog", TEMPLATES_EQUIPMENT_ARTIFACT)?;
    let template_values = array_field(catalog, "division_templates", TEMPLATES_EQUIPMENT_ARTIFACT)?;
    let equipment_blocks =
        object_field(catalog, "equipment_definitions", TEMPLATES_EQUIPMENT_ARTIFACT)?;
    let unit_blocks = object_field(catalog, "unit_definitions", TEMPLATES_EQUIPMENT_ARTIFACT)?;
    let counts = object_field(root, "counts", TEMPLATES_EQUIPMENT_ARTIFACT)?;
    require_count(
        counts,
        "division_templates",
        template_values.len(),
        TEMPLATES_EQUIPMENT_ARTIFACT,
    )?;
    require_count(
        counts,
        "equipment_definitions",
        equipment_blocks.len(),
        TEMPLATES_EQUIPMENT_ARTIFACT,
    )?;
    require_count(
        counts,
        "unit_definitions",
        unit_blocks.len(),
        TEMPLATES_EQUIPMENT_ARTIFACT,
    )?;

    let mut division_templates = Vec::with_capacity(template_values.len());
    let mut templates_by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, value) in template_values.iter().enumerate() {
        let object = value.as_object().ok_or_else(|| {
            StaticUnitCatalogError::new(format!(
                "{TEMPLATES_EQUIPMENT_ARTIFACT}.catalog.division_templates[{index}] is not an object"
            ))
        })?;
        let name = nonempty_string(object, "name", TEMPLATES_EQUIPMENT_ARTIFACT)?;
        let source = nonempty_string(object, "source", TEMPLATES_EQUIPMENT_ARTIFACT)?;
        let source_sha256 =
            sha256_field(object, "source_sha256", TEMPLATES_EQUIPMENT_ARTIFACT)?;
        let ordinal_in_file =
            unsigned_field(object, "ordinal_in_file", TEMPLATES_EQUIPMENT_ARTIFACT)?;
        let regiment_count =
            unsigned_field(object, "regiment_count", TEMPLATES_EQUIPMENT_ARTIFACT)?;
        let support_count =
            unsigned_field(object, "support_count", TEMPLATES_EQUIPMENT_ARTIFACT)?;
        require_array_len(object, "regiments", regiment_count, TEMPLATES_EQUIPMENT_ARTIFACT)?;
        require_array_len(object, "support", support_count, TEMPLATES_EQUIPMENT_ARTIFACT)?;
        let division_names_group = optional_string(object, "division_names_group")?;
        templates_by_name
            .entry(name.clone())
            .or_default()
            .push(division_templates.len());
        division_templates.push(StaticDivisionTemplate {
            name,
            division_names_group,
            source,
            source_sha256,
            ordinal_in_file,
            regiment_count,
            support_count,
            raw: value.clone(),
        });
    }

    Ok(TemplatesEquipmentCatalog {
        identity: StaticCatalogIdentity::from_artifact(artifact),
        division_templates,
        equipment_definitions: flatten_definition_blocks(
            equipment_blocks,
            TEMPLATES_EQUIPMENT_ARTIFACT,
        )?,
        unit_definitions: parse_named_definition_blocks(
            unit_blocks,
            TEMPLATES_EQUIPMENT_ARTIFACT,
        )?,
        templates_by_name,
    })
}

fn flatten_definition_blocks(
    blocks: &Map<String, Value>,
    artifact: &str,
) -> Result<BTreeMap<String, Vec<StaticDefinition>>, StaticUnitCatalogError> {
    let mut definitions: BTreeMap<String, Vec<StaticDefinition>> = BTreeMap::new();
    for (map_key, value) in blocks {
        let block = value.as_object().ok_or_else(|| {
            StaticUnitCatalogError::new(format!(
                "{artifact}.catalog definition block {map_key:?} is not an object"
            ))
        })?;
        let wrapper_token = nonempty_string(block, "token", artifact)?;
        let source = nonempty_string(block, "source", artifact)?;
        let source_sha256 = sha256_field(block, "source_sha256", artifact)?;
        let fields = object_field(block, "fields", artifact)?;
        for (token, fields) in fields {
            if token.trim().is_empty() {
                return Err(StaticUnitCatalogError::new(format!(
                    "{artifact} definition block {map_key:?} contains an empty token"
                )));
            }
            definitions
                .entry(token.clone())
                .or_default()
                .push(StaticDefinition {
                    token: token.clone(),
                    wrapper_token: wrapper_token.clone(),
                    source: source.clone(),
                    source_sha256: source_sha256.clone(),
                    fields: fields.clone(),
                });
        }
    }
    Ok(definitions)
}

fn parse_named_definition_blocks(
    blocks: &Map<String, Value>,
    artifact: &str,
) -> Result<BTreeMap<String, Vec<StaticDefinition>>, StaticUnitCatalogError> {
    let mut definitions: BTreeMap<String, Vec<StaticDefinition>> = BTreeMap::new();
    for (map_key, value) in blocks {
        let block = value.as_object().ok_or_else(|| {
            StaticUnitCatalogError::new(format!(
                "{artifact}.catalog definition block {map_key:?} is not an object"
            ))
        })?;
        let token = nonempty_string(block, "token", artifact)?;
        let source = nonempty_string(block, "source", artifact)?;
        let source_sha256 = sha256_field(block, "source_sha256", artifact)?;
        let fields = object_field(block, "fields", artifact)?;
        if map_key != &token && !map_key.starts_with(&format!("{token}#")) {
            return Err(StaticUnitCatalogError::new(format!(
                "{artifact} definition block key {map_key:?} disagrees with token {token:?}"
            )));
        }
        definitions
            .entry(token.clone())
            .or_default()
            .push(StaticDefinition {
                token: token.clone(),
                wrapper_token: token,
                source,
                source_sha256,
                fields: Value::Object(fields.clone()),
            });
    }
    Ok(definitions)
}

fn parse_ship_definitions(
    artifact: &PackagedStaticArtifact,
) -> Result<ShipDefinitionsCatalog, StaticUnitCatalogError> {
    require_artifact_name(artifact, SHIP_DEFINITIONS_ARTIFACT)?;
    let root_value = parse_root(artifact, SHIP_DEFINITIONS_SCHEMA)?;
    let root = root_value
        .as_object()
        .expect("parse_root validated an object root");
    let hull_values = object_field(root, "ship_hulls", SHIP_DEFINITIONS_ARTIFACT)?;
    let module_values = object_field(root, "ship_modules", SHIP_DEFINITIONS_ARTIFACT)?;
    let meta = object_field(root, "meta", SHIP_DEFINITIONS_ARTIFACT)?;
    require_count(
        meta,
        "ship_hull_count",
        hull_values.len(),
        SHIP_DEFINITIONS_ARTIFACT,
    )?;
    require_count(
        meta,
        "ship_module_count",
        module_values.len(),
        SHIP_DEFINITIONS_ARTIFACT,
    )?;

    let mut hulls = BTreeMap::new();
    for (token, value) in hull_values {
        let row = value.as_object().ok_or_else(|| {
            StaticUnitCatalogError::new(format!(
                "{SHIP_DEFINITIONS_ARTIFACT}.ship_hulls.{token} is not an object"
            ))
        })?;
        let id = matching_id(row, token, "ship_hulls")?;
        let default_modules = string_map_field(
            row,
            "default_modules",
            SHIP_DEFINITIONS_ARTIFACT,
        )?;
        hulls.insert(
            token.clone(),
            ShipHullDefinition {
                id,
                archetype: optional_string(row, "archetype")?,
                localised_name: optional_string(row, "localised_name")?,
                source_file: nonempty_string(row, "source_file", SHIP_DEFINITIONS_ARTIFACT)?,
                source_layer: nonempty_string(row, "source_layer", SHIP_DEFINITIONS_ARTIFACT)?,
                default_modules,
                stats: value_map_field(row, "stats", SHIP_DEFINITIONS_ARTIFACT)?,
                raw: value.clone(),
            },
        );
    }

    let mut modules = BTreeMap::new();
    for (token, value) in module_values {
        let row = value.as_object().ok_or_else(|| {
            StaticUnitCatalogError::new(format!(
                "{SHIP_DEFINITIONS_ARTIFACT}.ship_modules.{token} is not an object"
            ))
        })?;
        modules.insert(
            token.clone(),
            ShipModuleDefinition {
                id: matching_id(row, token, "ship_modules")?,
                category: nonempty_string(row, "category", SHIP_DEFINITIONS_ARTIFACT)?,
                localised_name: optional_string(row, "localised_name")?,
                source_file: nonempty_string(row, "source_file", SHIP_DEFINITIONS_ARTIFACT)?,
                source_layer: nonempty_string(row, "source_layer", SHIP_DEFINITIONS_ARTIFACT)?,
                add_stats: optional_string_map_field(
                    row,
                    "add_stats",
                    SHIP_DEFINITIONS_ARTIFACT,
                )?,
                multiply_stats: optional_string_map_field(
                    row,
                    "multiply_stats",
                    SHIP_DEFINITIONS_ARTIFACT,
                )?,
                raw: value.clone(),
            },
        );
    }

    for (hull, definition) in &hulls {
        for (slot, module) in &definition.default_modules {
            if !module.is_empty()
                && module != "empty"
                && module != "inherit"
                && !modules.contains_key(module)
            {
                return Err(StaticUnitCatalogError::new(format!(
                    "{SHIP_DEFINITIONS_ARTIFACT} hull {hull:?} slot {slot:?} references unknown default module {module:?}"
                )));
            }
        }
    }

    Ok(ShipDefinitionsCatalog {
        identity: StaticCatalogIdentity::from_artifact(artifact),
        hulls,
        modules,
    })
}

fn parse_root(
    artifact: &PackagedStaticArtifact,
    schema: &str,
) -> Result<Value, StaticUnitCatalogError> {
    let value: Value = serde_json::from_slice(artifact.bytes()).map_err(|error| {
        StaticUnitCatalogError::new(format!(
            "parse packaged static artifact {} bytes: {error}",
            artifact.name()
        ))
    })?;
    let root = value.as_object().ok_or_else(|| {
        StaticUnitCatalogError::new(format!(
            "packaged static artifact {} root is not an object",
            artifact.name()
        ))
    })?;
    if root.get("schema").and_then(Value::as_str) != Some(schema) {
        return Err(StaticUnitCatalogError::new(format!(
            "packaged static artifact {} schema is not {schema}",
            artifact.name()
        )));
    }
    Ok(value)
}

fn require_artifact_name(
    artifact: &PackagedStaticArtifact,
    expected: &str,
) -> Result<(), StaticUnitCatalogError> {
    if artifact.name() != expected {
        return Err(StaticUnitCatalogError::new(format!(
            "static catalog consumer expected artifact {expected:?}, got {:?}",
            artifact.name()
        )));
    }
    Ok(())
}

fn object_field<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a Map<String, Value>, StaticUnitCatalogError> {
    object.get(field).and_then(Value::as_object).ok_or_else(|| {
        StaticUnitCatalogError::new(format!("{context}.{field} is not an object"))
    })
}

fn array_field<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a Vec<Value>, StaticUnitCatalogError> {
    object.get(field).and_then(Value::as_array).ok_or_else(|| {
        StaticUnitCatalogError::new(format!("{context}.{field} is not an array"))
    })
}

fn nonempty_string(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<String, StaticUnitCatalogError> {
    let value = object.get(field).and_then(Value::as_str).ok_or_else(|| {
        StaticUnitCatalogError::new(format!("{context}.{field} is not a string"))
    })?;
    if value.trim().is_empty() {
        return Err(StaticUnitCatalogError::new(format!(
            "{context}.{field} is empty"
        )));
    }
    Ok(value.to_string())
}

fn optional_string(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<String>, StaticUnitCatalogError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
        _ => Err(StaticUnitCatalogError::new(format!(
            "{field} is neither null nor a non-empty string"
        ))),
    }
}

fn unsigned_field(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<u64, StaticUnitCatalogError> {
    object.get(field).and_then(Value::as_u64).ok_or_else(|| {
        StaticUnitCatalogError::new(format!("{context}.{field} is not an unsigned integer"))
    })
}

fn sha256_field(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<String, StaticUnitCatalogError> {
    let value = nonempty_string(object, field, context)?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StaticUnitCatalogError::new(format!(
            "{context}.{field} is not a SHA-256 hex digest"
        )));
    }
    Ok(value.to_ascii_lowercase())
}

fn require_count(
    object: &Map<String, Value>,
    field: &str,
    actual: usize,
    context: &str,
) -> Result<(), StaticUnitCatalogError> {
    let expected = unsigned_field(object, field, context)?;
    if u64::try_from(actual).ok() != Some(expected) {
        return Err(StaticUnitCatalogError::new(format!(
            "{context}.{field} declares {expected}, parsed {actual}"
        )));
    }
    Ok(())
}

fn require_array_len(
    object: &Map<String, Value>,
    field: &str,
    expected: u64,
    context: &str,
) -> Result<(), StaticUnitCatalogError> {
    let values = array_field(object, field, context)?;
    if u64::try_from(values.len()).ok() != Some(expected) {
        return Err(StaticUnitCatalogError::new(format!(
            "{context}.{field} length {} disagrees with declared {expected}",
            values.len()
        )));
    }
    Ok(())
}

fn string_map_field(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<BTreeMap<String, String>, StaticUnitCatalogError> {
    let values = object_field(object, field, context)?;
    values
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_string()))
                .ok_or_else(|| {
                    StaticUnitCatalogError::new(format!(
                        "{context}.{field}.{key} is not a string"
                    ))
                })
        })
        .collect()
}

fn optional_string_map_field(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<BTreeMap<String, String>, StaticUnitCatalogError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(BTreeMap::new()),
        Some(Value::Object(_)) => string_map_field(object, field, context),
        _ => Err(StaticUnitCatalogError::new(format!(
            "{context}.{field} is not an object"
        ))),
    }
}

fn value_map_field(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<BTreeMap<String, Value>, StaticUnitCatalogError> {
    object_field(object, field, context).map(|values| {
        values
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    })
}

fn matching_id(
    object: &Map<String, Value>,
    token: &str,
    category: &str,
) -> Result<String, StaticUnitCatalogError> {
    let id = nonempty_string(object, "id", SHIP_DEFINITIONS_ARTIFACT)?;
    if id != token {
        return Err(StaticUnitCatalogError::new(format!(
            "{SHIP_DEFINITIONS_ARTIFACT}.{category}.{token} declares mismatched id {id:?}"
        )));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {

    use super::*;
    use replay_core::PackagedStaticArtifact;

    const SOURCE_SHA: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const CONTAINER_SHA: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const CONTENT_DIGEST: &str = "3333333333333333333333333333333333333333333333333333333333333333";

    fn artifact(name: &str, value: &Value) -> PackagedStaticArtifact {
        PackagedStaticArtifact::unauthenticated_for_consumer_tests(
            name,
            &format!("static/{name}.json"),
            CONTAINER_SHA,
            CONTENT_DIGEST,
            serde_json::to_vec(value).expect("fixture serialises"),
        )
    }

    fn templates_value() -> Value {
        serde_json::json!({
            "schema": TEMPLATES_EQUIPMENT_SCHEMA,
            "counts": {
                "division_templates": 1,
                "equipment_definitions": 1,
                "unit_definitions": 1
            },
            "catalog": {
                "division_templates": [{
                    "name": "Infanterie",
                    "division_names_group": "GER_INF_01",
                    "source": "history/units/GER_1936.txt",
                    "source_sha256": SOURCE_SHA,
                    "ordinal_in_file": 0,
                    "regiment_count": 2,
                    "support_count": 1,
                    "regiments": [
                        {"unit_type": "infantry", "x": "0", "y": "0"},
                        {"unit_type": "infantry", "x": "0", "y": "1"}
                    ],
                    "support": [{"unit_type": "engineer", "x": "0", "y": "0"}]
                }],
                "equipment_definitions": {
                    "equipments": {
                        "token": "equipments",
                        "source": "common/units/equipment/infantry.txt",
                        "source_sha256": SOURCE_SHA,
                        "fields": {
                            "infantry_equipment_1": {"build_cost_ic": "0.43"}
                        }
                    }
                },
                "unit_definitions": {
                    "infantry": {
                        "token": "infantry",
                        "source": "common/units/infantry.txt",
                        "source_sha256": SOURCE_SHA,
                        "fields": {"max_organisation": "60"}
                    }
                }
            }
        })
    }

    fn ships_value() -> Value {
        serde_json::json!({
            "schema": SHIP_DEFINITIONS_SCHEMA,
            "meta": {"ship_hull_count": 1, "ship_module_count": 1},
            "ship_hulls": {
                "ship_hull_light_1": {
                    "id": "ship_hull_light_1",
                    "archetype": "ship_hull_light",
                    "localised_name": null,
                    "source_file": "common/units/ship_hull_light.txt",
                    "source_layer": "vanilla",
                    "default_modules": {"fixed_ship_engine_slot": "light_engine_1"},
                    "stats": {"max_strength": 100}
                }
            },
            "ship_modules": {
                "light_engine_1": {
                    "id": "light_engine_1",
                    "category": "ship_engine",
                    "localised_name": null,
                    "source_file": "common/units/equipment/modules/ship_modules.txt",
                    "source_layer": "vanilla",
                    "add_stats": {"max_strength": "10"},
                    "multiply_stats": {}
                }
            }
        })
    }

    fn admit(templates: &Value) -> Result<StaticUnitCatalogs, StaticUnitCatalogError> {
        let templates = artifact(TEMPLATES_EQUIPMENT_ARTIFACT, templates);
        let ships = artifact(SHIP_DEFINITIONS_ARTIFACT, &ships_value());
        admit_static_unit_catalogs(Some(&templates), Some(&ships))
    }

    #[test]
    fn a_well_formed_pair_is_admitted_and_joins_by_name_and_token() {
        let catalogs = admit(&templates_value()).expect("the well-formed fixture is admitted");
        let templates = &catalogs.templates_equipment;
        assert_eq!(templates.identity.artifact_name, TEMPLATES_EQUIPMENT_ARTIFACT);
        assert_eq!(templates.division_templates.len(), 1);
        assert_eq!(
            templates
                .templates_named("Infanterie")
                .map(|row| row.regiment_count)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(templates.equipment_named("infantry_equipment_1").len(), 1);
        assert_eq!(
            templates.equipment_named("infantry_equipment_1")[0].wrapper_token,
            "equipments"
        );
        assert_eq!(templates.unit_named("infantry").len(), 1);
        assert!(templates.templates_named("Volksgrenadier").next().is_none());
        assert_eq!(catalogs.ships.hulls.len(), 1);
        assert_eq!(catalogs.ships.modules.len(), 1);
    }

    #[test]
    fn a_recording_without_the_templates_catalog_is_refused_by_name() {
        let ships = artifact(SHIP_DEFINITIONS_ARTIFACT, &ships_value());
        let error = admit_static_unit_catalogs(None, Some(&ships))
            .expect_err("a missing required artifact is a refusal");
        assert!(
            error.to_string().contains(TEMPLATES_EQUIPMENT_ARTIFACT),
            "the refusal must name the artifact: {error}"
        );
    }

    #[test]
    fn a_recording_without_the_ship_definitions_catalog_is_refused_by_name() {
        let templates = artifact(TEMPLATES_EQUIPMENT_ARTIFACT, &templates_value());
        let error = admit_static_unit_catalogs(Some(&templates), None)
            .expect_err("a missing required artifact is a refusal");
        assert!(
            error.to_string().contains(SHIP_DEFINITIONS_ARTIFACT),
            "the refusal must name the artifact: {error}"
        );
    }

    #[test]
    fn an_artifact_delivered_under_another_name_is_refused() {
        let templates = artifact("some_other_static_artifact", &templates_value());
        let ships = artifact(SHIP_DEFINITIONS_ARTIFACT, &ships_value());
        let error = admit_static_unit_catalogs(Some(&templates), Some(&ships))
            .expect_err("a renamed artifact is a refusal");
        assert!(
            error.to_string().contains(TEMPLATES_EQUIPMENT_ARTIFACT),
            "the refusal must name the artifact it expected: {error}"
        );
    }

    #[test]
    fn a_payload_declaring_another_schema_is_refused() {
        let mut value = templates_value();
        value["schema"] = Value::String("templates_equipment_catalog_live.static_artifact.v2".into());
        let error = admit(&value).expect_err("a foreign schema is a refusal");
        assert!(error.to_string().contains("schema"), "{error}");
    }

    #[test]
    fn counts_that_disagree_with_the_rows_are_refused() {
        for field in ["division_templates", "equipment_definitions", "unit_definitions"] {
            let mut value = templates_value();
            value["counts"][field] = serde_json::json!(7);
            let error = admit(&value)
                .expect_err("a count that disagrees with its rows is a refusal")
                .to_string();
            assert!(error.contains(field), "the refusal must name the field: {error}");
        }
    }

    #[test]
    fn a_template_whose_grid_disagrees_with_its_declared_count_is_refused() {
        let mut value = templates_value();
        value["catalog"]["division_templates"][0]["regiment_count"] = serde_json::json!(9);
        let error = admit(&value).expect_err("a grid/count disagreement is a refusal");
        assert!(error.to_string().contains("regiments"), "{error}");
    }

    #[test]
    fn a_unit_block_whose_key_disagrees_with_its_token_is_refused() {
        let mut value = templates_value();
        let block = value["catalog"]["unit_definitions"]["infantry"].clone();
        value["catalog"]["unit_definitions"] = serde_json::json!({"artillery": block});
        let error = admit(&value).expect_err("a key/token disagreement is a refusal");
        assert!(error.to_string().contains("token"), "{error}");
    }

    #[test]
    fn rows_of_the_wrong_shape_are_refused_one_by_one() {
        let mut not_an_object = templates_value();
        not_an_object["catalog"]["division_templates"][0] = serde_json::json!("Infanterie");
        assert!(admit(&not_an_object).is_err(), "a non-object template row");

        let mut bad_digest = templates_value();
        bad_digest["catalog"]["division_templates"][0]["source_sha256"] =
            serde_json::json!("not-a-digest");
        assert!(admit(&bad_digest).is_err(), "a source digest that is not one");

        let mut empty_token = templates_value();
        empty_token["catalog"]["equipment_definitions"]["equipments"]["fields"] =
            serde_json::json!({"   ": {"build_cost_ic": "0.43"}});
        assert!(admit(&empty_token).is_err(), "an empty definition token");

        let mut missing_name = templates_value();
        missing_name["catalog"]["division_templates"][0]
            .as_object_mut()
            .expect("template row is an object")
            .remove("name");
        assert!(admit(&missing_name).is_err(), "a template with no name");
    }

    #[test]
    fn bytes_that_are_not_json_are_refused() {
        let templates = PackagedStaticArtifact::unauthenticated_for_consumer_tests(
            TEMPLATES_EQUIPMENT_ARTIFACT,
            "static/templates_equipment_catalog.json",
            CONTAINER_SHA,
            CONTENT_DIGEST,
            b"this is not json".to_vec(),
        );
        let ships = artifact(SHIP_DEFINITIONS_ARTIFACT, &ships_value());
        let error = admit_static_unit_catalogs(Some(&templates), Some(&ships))
            .expect_err("unparseable bytes are a refusal");
        assert!(error.to_string().contains("parse"), "{error}");
    }
}
