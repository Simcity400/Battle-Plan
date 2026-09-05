
use crate::assets::{AssetResolver, DecodedTexture};
use crate::dds;
use crate::font::BitmapFont;
use crate::gfx::SpriteRegistry;
use crate::gui::{parse_gui_document, GuiElement};
use crate::loc::{parse_language_yml_with_duplicates, Localisation};
use crate::paradox_text::{
    decode_text, destructive_event_count, destructive_losses, diagnose_text_decode, parse_bytes,
    Document,
};
use crate::scripted_gui_runtime::{load_scripted_gui_registry, ScriptedGuiRegistry};
use crate::tga;
use crate::{GameUiError, Result};
use map_viewer::{EffectFile, GameFileResolver, PdxAssetTree};
use replay_core::{UsedContentKey, ValidatedContentAdmission};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Interpreted<T> {
    pub provenance: Vec<UsedContentKey>,
    pub value: T,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GfxDocument {
    pub document: Document,
    pub sprites: SpriteRegistry,
}

pub struct ProductionContentInterpreter<'a> {
    resolver: &'a GameFileResolver,
    admission: &'a ValidatedContentAdmission,
    assets: AssetResolver,
    localisation_language: String,
}

impl<'a> ProductionContentInterpreter<'a> {
    pub fn new(
        resolver: &'a GameFileResolver,
        admission: &'a ValidatedContentAdmission,
        localisation_language: impl Into<String>,
    ) -> Result<Self> {
        if !resolver.has_content_admission() {
            return Err(GameUiError::missing(
                "production content interpreter requires an admission-bound VFS",
            ));
        }
        let assets = AssetResolver::from_content_admission(resolver, admission)?;
        let localisation_language = localisation_language.into();
        if localisation_language.trim().is_empty() {
            return Err(GameUiError::parse(
                "production content interpreter requires a playback-selected localisation language",
            ));
        }
        Ok(Self {
            resolver,
            admission,
            assets,
            localisation_language,
        })
    }

    pub fn content_key(&self, logical_path: &str) -> Result<UsedContentKey> {
        self.admission
            .require_used_content(logical_path)
            .cloned()
            .map_err(GameUiError::missing)
    }

    pub fn read(&self, expected: &UsedContentKey) -> Result<Interpreted<Vec<u8>>> {
        self.require_exact_key(expected)?;
        let bytes = self
            .resolver
            .read_admitted_used_content(expected)
            .map_err(|error| GameUiError::missing(error.to_string()))?;
        Ok(Interpreted {
            provenance: vec![expected.clone()],
            value: bytes,
        })
    }

    pub fn parse_clausewitz(&self, expected: &UsedContentKey) -> Result<Interpreted<Document>> {
        let bytes = self.read(expected)?;
        reject_decode_damage(expected, &bytes.value)?;
        let document = parse_bytes(&bytes.value);
        reject_recoveries(expected, &document)?;
        Ok(Interpreted {
            provenance: bytes.provenance,
            value: document,
        })
    }

    pub fn parse_gui(&self, expected: &UsedContentKey) -> Result<Interpreted<Vec<GuiElement>>> {
        require_extension(expected, "gui")?;
        let parsed = self.parse_clausewitz(expected)?;
        let roots = parse_gui_document(&parsed.value);
        Ok(Interpreted {
            provenance: parsed.provenance,
            value: roots,
        })
    }

    pub fn parse_gfx(&self, expected: &UsedContentKey) -> Result<Interpreted<GfxDocument>> {
        require_extension(expected, "gfx")?;
        let parsed = self.parse_clausewitz(expected)?;
        let mut sprites = SpriteRegistry::new();
        sprites.add_document(&expected.normalized_game_relative_path, &parsed.value);
        Ok(Interpreted {
            provenance: parsed.provenance,
            value: GfxDocument {
                document: parsed.value,
                sprites,
            },
        })
    }

    pub fn parse_localisation(
        &self,
        expected: &UsedContentKey,
    ) -> Result<Interpreted<Localisation>> {
        require_extension(expected, "yml")?;
        let bytes = self.read(expected)?;
        reject_decode_damage(expected, &bytes.value)?;
        let text = decode_text(&bytes.value);
        strict_localisation_body(
            &expected.normalized_game_relative_path,
            &text,
            &self.localisation_language,
        )?;
        let mut entries = BTreeMap::new();
        let (parse_failures, duplicate_keys) =
            parse_language_yml_with_duplicates(
                &text,
                &self.localisation_language,
                &mut entries,
            );
        let parse_failures = parse_failures
            .into_iter()
            .map(|failure| format!("{}: {failure}", expected.normalized_game_relative_path))
            .collect();
        let source_by_key = entries
            .keys()
            .filter(|key| !duplicate_keys.contains(*key))
            .map(|key| (key.clone(), expected.normalized_game_relative_path.clone()))
            .collect();
        let ambiguous_sources_by_key = duplicate_keys
            .into_iter()
            .map(|key| {
                (
                    key,
                    vec![expected.normalized_game_relative_path.clone()],
                )
            })
            .collect();
        Ok(Interpreted {
            provenance: bytes.provenance,
            value: Localisation {
                language: self.localisation_language.clone(),
                autonomy_names: Default::default(),
                autonomy_names_unavailable: Some("single-file localisation interpretation does not admit autonomy definitions".into()),
                source_by_key,
                ambiguous_sources_by_key,
                provenance_complete: true,
                entries,
                source_files: vec![expected.normalized_game_relative_path.clone()],
                parse_failures,
                decode_lossy_files: Vec::new(),
            },
        })
    }

    pub fn parse_shader_effect(
        &self,
        expected: &UsedContentKey,
    ) -> Result<Interpreted<EffectFile>> {
        require_one_extension(expected, &["shader", "fxh"])?;
        let bytes = self.read(expected)?;
        reject_decode_damage(expected, &bytes.value)?;
        let source = decode_text(&bytes.value);
        let effect = map_viewer::parse_effect_file(&source)
            .map_err(|error| GameUiError::parse(error.to_string()))?;
        Ok(Interpreted {
            provenance: bytes.provenance,
            value: effect,
        })
    }

    pub fn parse_pdx_asset(&self, expected: &UsedContentKey) -> Result<Interpreted<PdxAssetTree>> {
        require_one_extension(expected, &["mesh", "anim"])?;
        let bytes = self.read(expected)?;
        let tree = map_viewer::parse_pdx_asset(&bytes.value)
            .map_err(|error| GameUiError::parse(error.to_string()))?;
        Ok(Interpreted {
            provenance: bytes.provenance,
            value: tree,
        })
    }

    pub fn decode_texture(&self, expected: &UsedContentKey) -> Result<Interpreted<DecodedTexture>> {
        require_one_extension(expected, &["dds", "tga", "png"])?;
        self.require_exact_key(expected)?;
        let bytes = self.read(expected)?;
        let rel_path = &expected.normalized_game_relative_path;
        let ext = rel_path
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_ascii_lowercase())
            .unwrap_or_default();
        let (format, image) = match ext.as_str() {
            "dds" => {
                let decoded = dds::decode_bytes(&bytes.value)
                    .map_err(|error| add_logical_path(error, rel_path))?;
                (decoded.format, decoded.image)
            }
            "tga" => {
                let decoded = tga::decode_bytes(&bytes.value)
                    .map_err(|error| add_logical_path(error, rel_path))?;
                (decoded.format, decoded.image)
            }
            "png" => {
                let decoded = crate::png::decode_bytes(&bytes.value)
                    .map_err(|error| add_logical_path(error, rel_path))?;
                (decoded.format, decoded.image)
            }
            _ => {
                return Err(GameUiError::unsupported_texture_extension(
                    ext.clone(),
                    format!("{rel_path} uses unsupported texture extension .{ext}"),
                ));
            }
        };
        let resolved = self.assets.resolve_texture_file(rel_path)?;
        Ok(Interpreted {
            provenance: bytes.provenance,
            value: DecodedTexture {
                rel_path: resolved.rel_path,
                path: resolved.path,
                format,
                image,
            },
        })
    }

    pub fn load_bitmap_font(
        &self,
        symbolic_name: &str,
        expected_inputs: &[UsedContentKey],
    ) -> Result<Interpreted<BitmapFont>> {
        if expected_inputs.is_empty() {
            return Err(GameUiError::missing(
                "bitmap font load requires explicit admitted input identities",
            ));
        }
        for key in expected_inputs {
            self.read(key)?;
        }
        let restricted = self
            .resolver
            .restrict_to(
                expected_inputs
                    .iter()
                    .map(|key| map_viewer::RequiredGameContent {
                        logical_path: key.normalized_game_relative_path.clone(),
                        sha256: key.payload_sha256.clone(),
                        byte_len: key.payload_bytes,
                    }),
            )
            .map_err(|error| GameUiError::missing(error.to_string()))?;
        let font_assets = AssetResolver::from_restricted_game_files(&restricted)?;
        let font = BitmapFont::load(&font_assets, symbolic_name)?;
        Ok(Interpreted {
            provenance: expected_inputs.to_vec(),
            value: font,
        })
    }

    pub fn load_scripted_gui_registry(&self) -> Result<Interpreted<ScriptedGuiRegistry>> {
        let registry = load_scripted_gui_registry(&self.assets)?;
        if !registry.production_admitted() {
            return Err(GameUiError::parse(format!(
                "scripted-GUI interpreter rejected source issues: {}",
                registry.source_issues.join("; ")
            )));
        }
        let mut provenance = self
            .assets
            .list_files_recursive("common/scripted_guis", Some("txt"))?
            .into_iter()
            .map(|file| self.content_key(&file.rel_path))
            .collect::<Result<Vec<_>>>()?;
        provenance.sort_by(|left, right| {
            left.normalized_game_relative_path
                .cmp(&right.normalized_game_relative_path)
        });
        for key in &provenance {
            self.read(key)?;
        }
        Ok(Interpreted {
            provenance,
            value: registry,
        })
    }

    fn require_exact_key(&self, expected: &UsedContentKey) -> Result<()> {
        let admitted = self
            .admission
            .require_used_content(&expected.normalized_game_relative_path)
            .map_err(GameUiError::missing)?;
        if admitted != expected {
            return Err(GameUiError::missing(format!(
                "content key does not equal replay admission for {}",
                expected.normalized_game_relative_path
            )));
        }
        Ok(())
    }
}

fn add_logical_path(mut error: GameUiError, rel_path: &str) -> GameUiError {
    error.message = format!("{rel_path}: {}", error.message);
    error
}

fn reject_decode_damage(expected: &UsedContentKey, bytes: &[u8]) -> Result<()> {
    let losses = diagnose_text_decode(bytes);
    let events = destructive_event_count(&losses);
    if events == 0 {
        return Ok(());
    }
    let first = destructive_losses(&losses)
        .first()
        .map(ToString::to_string)
        .unwrap_or_default();
    Err(GameUiError::parse(format!(
        "{} decode destroyed {events} byte sequences: {first}",
        expected.normalized_game_relative_path
    )))
}

fn reject_recoveries(expected: &UsedContentKey, document: &Document) -> Result<()> {
    if let Some(recovery) = document.recoveries.first() {
        return Err(GameUiError::parse(format!(
            "{}:{}:{} requires unproven parser recovery: {}",
            expected.normalized_game_relative_path,
            recovery.line,
            recovery.column,
            recovery.message
        )));
    }
    Ok(())
}

fn require_extension(expected: &UsedContentKey, extension: &str) -> Result<()> {
    require_one_extension(expected, &[extension])
}

fn require_one_extension(expected: &UsedContentKey, extensions: &[&str]) -> Result<()> {
    let actual = expected
        .normalized_game_relative_path
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or("");
    if !extensions
        .iter()
        .any(|candidate| actual.eq_ignore_ascii_case(candidate))
    {
        return Err(GameUiError::parse(format!(
            "{} is not valid input for the requested interpreter ({})",
            expected.normalized_game_relative_path,
            extensions.join("/")
        )));
    }
    Ok(())
}

fn strict_localisation_body(source: &str, text: &str, expected_language: &str) -> Result<String> {
    let mut header = None;
    let mut body = String::new();
    for (index, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if header.is_none() {
            let candidate = trimmed.strip_suffix(':').ok_or_else(|| {
                GameUiError::parse(format!("{source}:{} lacks language header", index + 1))
            })?;
            if !candidate.eq_ignore_ascii_case(expected_language) {
                return Err(GameUiError::parse(format!(
                    "{source}:{} language header {} does not match replay language {}",
                    index + 1,
                    candidate,
                    expected_language
                )));
            }
            header = Some(candidate.to_string());
            continue;
        }
        if !trimmed.contains(':') {
            return Err(GameUiError::parse(format!(
                "{source}:{} malformed localisation row",
                index + 1
            )));
        }
        body.push_str(raw);
        body.push('\n');
    }
    if header.is_none() {
        return Err(GameUiError::parse(format!(
            "{source} has no localisation language header"
        )));
    }
    Ok(body)
}

pub const PRODUCTION_INTERPRETER_DATA_NEEDS: &[(&str, &str)] = &[
    (
        "generic_lua_execution",
        "only consumer-specific proven numeric define readers are admitted; arbitrary Lua evaluation remains fail-closed",
    ),
    (
        "generic_clausewitz_script_execution",
        "syntax is parsed losslessly, but arbitrary trigger/effect/controller semantics require an authenticated engine law",
    ),
    (
        "generic_entity_particle_material_execution",
        "definitions resolve through VFS; only specialised proven render consumers may execute their semantics",
    ),
    (
        "positional_map_text_grammars",
        "railway, supply-node and other positional map files remain owned by their proven map-specific interpreters",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localisation_header_is_runtime_language_not_english_default() {
        let body = strict_localisation_body(
            "localisation/french/test_l_french.yml",
            "l_french:\n KEY:0 \"Valeur\"\n",
            "l_french",
        )
        .unwrap();
        let mut entries = BTreeMap::new();
        assert!(crate::loc::parse_english_yml(&body, &mut entries).is_empty());
        assert_eq!(entries.get("KEY").map(String::as_str), Some("Valeur"));
        assert!(strict_localisation_body(
            "localisation/english/test.yml",
            "l_english:\n KEY:0 \"Value\"\n",
            "l_french"
        )
        .is_err());
    }

    #[test]
    fn unproven_clausewitz_recovery_fails_closed() {
        let document = parse_bytes(b"root = { value = yes");
        let key = UsedContentKey {
            effective_vfs_manifest_sha256: "0".repeat(64),
            stored_game_relative_path: "common/test.txt".into(),
            normalized_game_relative_path: "common/test.txt".into(),
            winning_layer_ordinal: 0,
            layer_kind: replay_core::ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "1".repeat(64),
            payload_sha256: "1".repeat(64),
            payload_bytes: 1,
        };
        assert!(reject_recoveries(&key, &document).is_err());
    }
}
