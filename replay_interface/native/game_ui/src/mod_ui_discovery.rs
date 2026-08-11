use crate::assets::{AssetResolver, ResolvedFile};
use crate::definition_admission::{load_effective_definition_catalog, ResolvedGuiDefinitionFile};
use crate::gfx::SpriteRegistry;
use crate::loc::Localisation;
use crate::paradox_text::{destructive_event_count, diagnose_text_decode, parse_bytes, Value};
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
    /// True when the admitted effective winner comes from a mod layer.
    #[serde(default)]
    pub mod_layer_winner: bool,
    /// Historical raw-root evidence only. Product discovery cannot classify
    /// add versus override because losing layers are outside its capability.
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
    #[serde(default)]
    pub mod_layer_gui_files: usize,
    pub sprite_definitions: usize,
    pub localisation_keys: usize,
    pub panels: Vec<DiscoveredPanel>,
    pub scripted_localisation: Vec<ScriptedLocalisationDefinition>,
    /// `.gui` files whose parse needed Clausewitz error recovery, with counts.
    /// Reported, not disqualifying: vanilla the game 1.19.2 ships four
    /// brace-damaged `.gui` files (see
    /// `definition_admission::FileAdmission::parse_recoveries`), so a
    /// recovery cannot be the difference between admitted and not.
    pub recovery_parsed_gui_files: Vec<String>,
    /// `.gui` files whose BYTES were destroyed at decode, with event counts.
    ///
    /// Unlike `recovery_parsed_gui_files` this one DOES disqualify: a recovery
    /// means the parse resynchronized over ambiguous syntax and the shipped
    /// game itself needs it, whereas destroyed bytes mean content this
    /// discovery genuinely did not see. Measured on the shipped the game 1.19.2
    /// corpus this is empty (zero destructive decode events across 8318 files),
    /// so a non-empty list really does mean mod content was lost.
    #[serde(default)]
    pub decode_lossy_gui_files: Vec<String>,
    /// `.gfx` files whose BYTES were destroyed at decode, with event counts.
    /// Same contract as [`ModUiDiscovery::decode_lossy_gui_files`].
    #[serde(default)]
    pub decode_lossy_gfx_files: Vec<String>,
    /// Localisation `.yml` files whose BYTES were destroyed at decode.
    ///
    /// Deliberately NOT merged into
    /// [`ModUiDiscovery::localisation_parse_failures`], which means unreadable
    /// files and lines that could not be interpreted. Byte-level loss is a
    /// different layer and is invisible to that one: a U+FFFD substituted
    /// inside a quoted value leaves a perfectly well-formed `KEY: "value"`
    /// line, so the line parser reports nothing while the displayed string is
    /// not the file's string.
    #[serde(default)]
    pub decode_lossy_localisation_files: Vec<String>,
    /// Unreadable or line-damaged localisation files, carried through from
    /// `Localisation::parse_failures` instead of being discarded (R11).
    /// Measured: the shipped `localisation/english` corpus produces none
    /// across 206 files / 129,087 lines, so this list being non-empty really
    /// does mean localisation content was lost.
    pub localisation_parse_failures: Vec<String>,
    /// True only when no discovered input was lost. This used to be a literal
    /// `true` on every successful call (R11 C6) — a statement about the code
    /// path rather than about the content.
    pub production_admitted: bool,
}

/// Discover every admitted runtime UI definition for the playback viewer's
/// selected language.
///
/// This is the only product entry point. It accepts an already restricted,
/// manifest-bound `AssetResolver`; raw install and user-data paths are not
/// reachable here. Every file enumerated below is therefore an authenticated
/// winner in the replay's effective VFS, and an unadmitted transitive input is
/// unavailable rather than silently read from the machine.
pub fn discover_admitted_runtime_mod_ui(
    resolver: &AssetResolver,
    localisation_language: &str,
) -> Result<ModUiDiscovery> {
    if !resolver.has_portable_content_admission() {
        return Err(GameUiError::missing(
            "XUI24 product discovery requires a portable content-admission resolver",
        ));
    }
    discover_with_resolver(resolver, localisation_language, None)
}

/// Non-product evidence seam retained for the historical XUI24 fixture corpus.
///
/// Shipped playback must call [`discover_admitted_runtime_mod_ui`]. Keeping
/// this name explicit prevents a later product caller from accidentally
/// reintroducing raw launcher paths as a fallback.
pub fn discover_runtime_mod_ui_from_userdir_for_evidence(
    game_root: &Path,
    userdata: &Path,
) -> Result<ModUiDiscovery> {
    let resolver = AssetResolver::from_runtime_userdir(game_root, userdata)?;
    discover_with_resolver(&resolver, "l_english", Some(game_root))
}

fn discover_with_resolver(
    resolver: &AssetResolver,
    localisation_language: &str,
    evidence_game_root: Option<&Path>,
) -> Result<ModUiDiscovery> {
    let language_directory = localisation_directory(localisation_language)?;
    let gui_files = resolver.list_files_recursive("interface", Some("gui"))?;
    let gfx_files = resolver.list_files_recursive("interface", Some("gfx"))?;
    let loc_files = resolver
        .list_files_recursive(&format!("localisation/{language_directory}"), Some("yml"))?;
    let scripted_files =
        resolver.list_files_recursive("common/scripted_localisation", Some("txt"))?;
    if evidence_game_root.is_none() {
        if gui_files.is_empty() || gfx_files.is_empty() || loc_files.is_empty() {
            return Err(GameUiError::missing(
                "XUI24 admitted UI closure lacks GUI, GFX, or selected-language localisation files",
            ));
        }
        for file in gui_files
            .iter()
            .chain(gfx_files.iter())
            .chain(loc_files.iter())
            .chain(scripted_files.iter())
        {
            if resolver.admitted_content_key(&file.rel_path).is_none() {
                return Err(GameUiError::missing(format!(
                    "XUI24 effective file is outside portable admission: {}",
                    file.rel_path
                )));
            }
        }
    }
    let rows = gui_files
        .iter()
        .map(|row| ResolvedGuiDefinitionFile::new(&row.rel_path, &row.path))
        .collect();
    let catalog = load_effective_definition_catalog(rows)?;
    let mut registry = SpriteRegistry::new();
    for file in &gfx_files {
        registry.parse_file(&file.path, &file.rel_path)?;
    }
    let localisation = if evidence_game_root.is_none() {
        Localisation::load_admitted_language(resolver, localisation_language)?
    } else {
        Localisation::load_language(resolver, localisation_language)?
    };
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
            mod_layer_winner: is_mod_layer_winner(resolver, &row.source_file).unwrap_or(false),
            mod_added_file: evidence_game_root
                .is_some_and(|game_root| is_mod_added(game_root, &row.source_file, &gui_files)),
        })
        .collect::<Vec<_>>();
    // Losing-layer presence is not part of a UsedContentKey. Product
    // discovery can state which effective winners are non-base, but cannot
    // guess whether a non-base winner added or overrode a base identity.
    // Historical evidence fixtures still retain the old split because their
    // raw base root is explicitly supplied to this non-product helper.
    let (mod_added_gui_files, overridden_gui_files) = if let Some(game_root) = evidence_game_root {
        (
            gui_files
                .iter()
                .filter(|row| !game_root.join(rel_path(&row.rel_path)).is_file())
                .count(),
            gui_files
                .iter()
                .filter(|row| {
                    let vanilla = game_root.join(rel_path(&row.rel_path));
                    vanilla.is_file() && row.path != vanilla
                })
                .count(),
        )
    } else {
        (0, 0)
    };
    let mod_layer_gui_files = gui_files
        .iter()
        .filter(|row| is_mod_layer_winner(resolver, &row.rel_path) == Some(true))
        .count();
    let recovery_parsed_gui_files = catalog.recovery_parsed_files();
    let decode_lossy_gui_files = catalog.decode_lossy_files();
    let decode_lossy_gfx_files = registry.decode_lossy_files();
    let localisation_parse_failures = localisation.parse_failures.clone();
    // A SEPARATE field, not merged into localisation_parse_failures: that field
    // is named and documented as unreadable/line-parse damage, and folding
    // byte-level loss into it would silently widen an existing status exactly
    // as folding decode loss into `recoveries` would have.
    let decode_lossy_localisation_files = localisation.decode_lossy_files.clone();
    // A localisation file that could not be read, or a line that could not be
    // interpreted, is content this discovery silently did not see. That, not
    // the mere fact that the function returned, is what admission means.
    //
    // A `.gui` file whose bytes were destroyed at decode is the same kind of
    // loss one layer down, so it counts the same way. Dropping it here would
    // let the serialized result claim "no discovered input was lost" over
    // damage the catalog had already measured.
    let production_admitted = localisation_parse_failures.is_empty()
        && decode_lossy_gui_files.is_empty()
        && decode_lossy_gfx_files.is_empty()
        && decode_lossy_localisation_files.is_empty();
    Ok(ModUiDiscovery {
        schema: "hoi4-xui24-mod-ui-discovery-v2".to_string(),
        gui_files: gui_files.len(),
        gfx_files: gfx_files.len(),
        localisation_files: loc_files.len(),
        scripted_localisation_files: scripted_files.len(),
        mod_added_gui_files,
        overridden_gui_files,
        mod_layer_gui_files,
        sprite_definitions: registry.len(),
        localisation_keys: localisation.len(),
        panels,
        scripted_localisation,
        recovery_parsed_gui_files,
        decode_lossy_gui_files,
        decode_lossy_gfx_files,
        decode_lossy_localisation_files,
        localisation_parse_failures,
        production_admitted,
    })
}

fn localisation_directory(language: &str) -> Result<String> {
    let language = language.trim().to_ascii_lowercase();
    let directory = language.strip_prefix("l_").unwrap_or(&language);
    if directory.is_empty()
        || !directory
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err(GameUiError::parse(
            "XUI24 playback language is not a safe Clausewitz language identity",
        ));
    }
    Ok(directory.to_string())
}

fn is_mod_layer_winner(resolver: &AssetResolver, rel: &str) -> Option<bool> {
    resolver
        .admitted_content_key(rel)
        .map(|row| row.layer_kind == replay_core::ContentLayerKind::Mod)
}

fn parse_scripted_localisation(file: &ResolvedFile) -> Result<Vec<ScriptedLocalisationDefinition>> {
    let bytes = std::fs::read(&file.path)?;
    // Destroyed bytes here are refused, not merely recorded — the same stance
    // this function already takes on parse recoveries, and for the same reason:
    // unlike `.gui`, the scripted-localisation corpus ships clean, so a strict
    // gate cannot refuse the base game. Measured: zero destructive decode events
    // across the shipped install. A U+FFFD inside a `name` or
    // `localization_key` is syntactically valid and would otherwise bind an
    // identity that was never in the file.
    let decode_events = destructive_event_count(&diagnose_text_decode(&bytes));
    if decode_events != 0 {
        return Err(GameUiError::parse(format!(
            "scripted-localisation decode destroyed {decode_events} byte sequences in {}",
            file.rel_path
        )));
    }
    let doc = parse_bytes(&bytes);
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
