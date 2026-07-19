use crate::assets::{AssetResolver, ResolvedFile};
use crate::definition_admission::{load_effective_definition_catalog, ResolvedGuiDefinitionFile};
use crate::gfx::SpriteRegistry;
use crate::loc::Localisation;
use crate::paradox_text::{parse_file, Value};
use crate::{GameUiError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPanel {
    pub window_id: String,
    pub name: String,
    pub element_type: String,
    pub source_file: String,
    pub mod_added_file: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptedLocalisationDefinition {
    pub name: String,
    pub source_file: String,
    pub candidate_keys: Vec<String>,
    pub requires_captured_runtime_value: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModUiDiscovery {
    pub schema: String,
    pub gui_files: usize,
    pub gfx_files: usize,
    pub localisation_files: usize,
    pub scripted_localisation_files: usize,
    pub mod_added_gui_files: usize,
    pub overridden_gui_files: usize,
    pub sprite_definitions: usize,
    pub localisation_keys: usize,
    pub panels: Vec<DiscoveredPanel>,
    pub scripted_localisation: Vec<ScriptedLocalisationDefinition>,
    pub production_admitted: bool,
}

pub fn discover_runtime_mod_ui(game_root: &Path, userdata: &Path) -> Result<ModUiDiscovery> {
    let resolver = AssetResolver::from_runtime_userdir(game_root, userdata)?;
    let gui_files = resolver.list_files_recursive("interface", Some("gui"))?;
    let gfx_files = resolver.list_files_recursive("interface", Some("gfx"))?;
    let loc_files = resolver.list_files_recursive("localisation/english", Some("yml"))?;
    let scripted_files =
        resolver.list_files_recursive("common/scripted_localisation", Some("txt"))?;
    let rows = gui_files
        .iter()
        .map(|row| ResolvedGuiDefinitionFile::new(&row.rel_path, &row.path))
        .collect();
    let catalog = load_effective_definition_catalog(rows)?;
    let unknown = catalog
        .rejected_windows
        .iter()
        .filter(|row| row.unsupported.iter().any(|item| item.contains("Unknown(")))
        .map(|row| format!("{}:{}:{:?}", row.source_file, row.name, row.unsupported))
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(GameUiError::parse(format!(
            "unknown effective widget constructs: {unknown:?}"
        )));
    }
    let mut registry = SpriteRegistry::new();
    for file in &gfx_files {
        registry.parse_file(&file.path, &file.rel_path)?;
    }
    let localisation = Localisation::load_english(&resolver)?;
    let mut scripted_localisation = Vec::new();
    for file in &scripted_files {
        scripted_localisation.extend(parse_scripted_localisation(file)?);
    }
    let panels = catalog
        .admitted_windows
        .iter()
        .map(|row| DiscoveredPanel {
            window_id: row.window_id.clone(),
            name: row.name.clone(),
            element_type: row.element_type.clone(),
            source_file: row.source_file.clone(),
            mod_added_file: is_mod_added(game_root, &row.source_file, &gui_files),
        })
        .collect::<Vec<_>>();
    let mod_added_gui_files = gui_files
        .iter()
        .filter(|row| !game_root.join(rel_path(&row.rel_path)).is_file())
        .count();
    let overridden_gui_files = gui_files
        .iter()
        .filter(|row| {
            let vanilla = game_root.join(rel_path(&row.rel_path));
            vanilla.is_file() && row.path != vanilla
        })
        .count();
    Ok(ModUiDiscovery {
        schema: "hoi4-xui24-mod-ui-discovery-v1".to_string(),
        gui_files: gui_files.len(),
        gfx_files: gfx_files.len(),
        localisation_files: loc_files.len(),
        scripted_localisation_files: scripted_files.len(),
        mod_added_gui_files,
        overridden_gui_files,
        sprite_definitions: registry.len(),
        localisation_keys: localisation.len(),
        panels,
        scripted_localisation,
        production_admitted: true,
    })
}

fn parse_scripted_localisation(file: &ResolvedFile) -> Result<Vec<ScriptedLocalisationDefinition>> {
    let doc = parse_file(&file.path)?;
    if !doc.recoveries.is_empty() {
        return Err(GameUiError::parse(format!(
            "scripted-localisation parse recovery in {}: {:?}",
            file.rel_path, doc.recoveries
        )));
    }
    let mut out = Vec::new();
    for entry in doc.root.fields() {
        if !entry
            .key
            .as_deref()
            .is_some_and(|key| key.eq_ignore_ascii_case("defined_text"))
        {
            return Err(GameUiError::parse(format!(
                "unknown scripted-localisation top-level construct in {}: {:?}",
                file.rel_path, entry.key
            )));
        }
        let block = entry.value.as_object().ok_or_else(|| {
            GameUiError::parse(format!(
                "defined_text is not an object in {}",
                file.rel_path
            ))
        })?;
        let name = entry.value.first_scalar_ci("name").ok_or_else(|| {
            GameUiError::parse(format!("defined_text lacks name in {}", file.rel_path))
        })?;
        let mut keys = BTreeSet::new();
        for field in block {
            let Some(key) = field.key.as_deref() else {
                return Err(GameUiError::parse(format!(
                    "anonymous defined_text field in {}",
                    file.rel_path
                )));
            };
            if key.eq_ignore_ascii_case("name") {
                continue;
            }
            if !key.eq_ignore_ascii_case("text") {
                return Err(GameUiError::parse(format!(
                    "unknown defined_text construct {key} in {}",
                    file.rel_path
                )));
            }
            let text = match &field.value {
                Value::Object(_) => &field.value,
                _ => {
                    return Err(GameUiError::parse(format!(
                        "defined_text text is not an object in {}",
                        file.rel_path
                    )))
                }
            };
            for child in text.fields() {
                collect_text_candidates(child, file, &mut keys)?;
            }
        }
        if keys.is_empty() {
            return Err(GameUiError::parse(format!(
                "defined_text has no localization_key candidate in {}",
                file.rel_path
            )));
        }
        out.push(ScriptedLocalisationDefinition {
            name: name.to_string(),
            source_file: file.rel_path.clone(),
            candidate_keys: keys.into_iter().collect(),
            requires_captured_runtime_value: true,
        });
    }
    Ok(out)
}

fn collect_text_candidates(
    field: &crate::paradox_text::Entry,
    file: &ResolvedFile,
    keys: &mut BTreeSet<String>,
) -> Result<()> {
    let Some(key) = field.key.as_deref() else {
        return Err(GameUiError::parse(format!(
            "anonymous scripted-localisation text field in {}",
            file.rel_path
        )));
    };
    if key.eq_ignore_ascii_case("trigger") {
        if !matches!(field.value, Value::Object(_)) {
            return Err(GameUiError::parse(format!(
                "scripted-localisation trigger is not an object in {}",
                file.rel_path
            )));
        }
        return Ok(());
    }
    if key.eq_ignore_ascii_case("localization_key") {
        let value = field.value.as_scalar().ok_or_else(|| {
            GameUiError::parse(format!(
                "scripted-localisation localization_key is not scalar in {}",
                file.rel_path
            ))
        })?;
        keys.insert(value.to_string());
        return Ok(());
    }
    if !key.eq_ignore_ascii_case("random_list") {
        return Err(GameUiError::parse(format!(
            "unknown scripted-localisation text construct {key} in {}",
            file.rel_path
        )));
    }
    let rows = field.value.as_object().ok_or_else(|| {
        GameUiError::parse(format!(
            "scripted-localisation random_list is not an object in {}",
            file.rel_path
        ))
    })?;
    for row in rows {
        let Some(row_key) = row.key.as_deref() else {
            return Err(GameUiError::parse(format!(
                "anonymous scripted-localisation random_list row in {}",
                file.rel_path
            )));
        };
        if row_key.eq_ignore_ascii_case("seed") {
            if row.value.as_scalar().is_none() {
                return Err(GameUiError::parse(format!(
                    "scripted-localisation random_list seed is not scalar in {}",
                    file.rel_path
                )));
            }
            continue;
        }
        if row_key.parse::<u64>().is_err() {
            return Err(GameUiError::parse(format!(
                "unknown scripted-localisation random_list construct {row_key} in {}",
                file.rel_path
            )));
        }
        let weighted = row.value.as_object().ok_or_else(|| {
            GameUiError::parse(format!(
                "scripted-localisation random_list weighted row is not an object in {}",
                file.rel_path
            ))
        })?;
        if weighted.len() != 1 || !weighted[0].key_matches("localization_key") {
            return Err(GameUiError::parse(format!(
                "unknown scripted-localisation random_list weighted row in {}",
                file.rel_path
            )));
        }
        let candidate = weighted[0].value.as_scalar().ok_or_else(|| {
            GameUiError::parse(format!(
                "scripted-localisation random_list localization_key is not scalar in {}",
                file.rel_path
            ))
        })?;
        keys.insert(candidate.to_string());
    }
    Ok(())
}

fn is_mod_added(game_root: &Path, rel: &str, files: &[ResolvedFile]) -> bool {
    files
        .iter()
        .find(|row| row.rel_path.eq_ignore_ascii_case(rel))
        .is_some_and(|row| !row.path.starts_with(game_root))
}

fn rel_path(rel: &str) -> std::path::PathBuf {
    rel.split('/')
        .fold(std::path::PathBuf::new(), |path, part| path.join(part))
}
