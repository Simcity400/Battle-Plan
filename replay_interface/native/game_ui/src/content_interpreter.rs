//! Admission-bound interpreters for production the game content.
//!
//! This module deliberately contains no vanilla catalog.  A caller must name
//! an exact [`UsedContentKey`] carried by the portable replay.  The central
//! launcher-equivalent VFS revalidates that complete identity before bytes are
//! exposed to an interpreter.  Engine laws live in the existing specialised
//! parsers; content and winning-layer provenance remain runtime data.

use crate::assets::{AssetResolver, DecodedTexture};
use crate::font::BitmapFont;
use crate::gfx::SpriteRegistry;
use crate::gui::{parse_gui_document, GuiElement};
use crate::loc::{parse_english_yml, Localisation};
use crate::paradox_text::{decode_text, parse_bytes, Document};
use crate::scripted_gui_runtime::{load_scripted_gui_registry, ScriptedGuiRegistry};
use crate::{GameUiError, Result};
use map_viewer::{EffectFile, GameFileResolver, PdxAssetTree};
use replay_core::{UsedContentKey, ValidatedContentAdmission};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Interpreted<T> {
    /// Every payload consumed to produce `value`, with its portable identity.
    pub provenance: Vec<UsedContentKey>,
    pub value: T,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GfxDocument {
    /// Lossless parsed source for font, material and future proven consumers.
    pub document: Document,
    /// The already-proven effective sprite interpreter.
    pub sprites: SpriteRegistry,
}

/// Production interpreter facade. It cannot be constructed without both a
/// resolver restricted to, and the exact admission derived from, a replay.
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
                "production content interpreter requires a recorded localisation language",
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

    /// Read exact admitted bytes. This is also the hand-off for specialised
    /// proven map/define/entity/particle consumers that own deeper semantics.
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

    /// Strict Clausewitz/Jomini text parsing. Recovery is useful during RE but
    /// not a production engine-law substitute, so any recovery fails closed.
    pub fn parse_clausewitz(&self, expected: &UsedContentKey) -> Result<Interpreted<Document>> {
        let bytes = self.read(expected)?;
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

    /// Parse one language file using the language recorded by the replay.
    /// Unsupported/mismatched headers and malformed rows fail closed.
    pub fn parse_localisation(
        &self,
        expected: &UsedContentKey,
    ) -> Result<Interpreted<Localisation>> {
        require_extension(expected, "yml")?;
        let bytes = self.read(expected)?;
        let text = decode_text(&bytes.value);
        let body = strict_localisation_body(
            &expected.normalized_game_relative_path,
            &text,
            &self.localisation_language,
        )?;
        let mut entries = BTreeMap::new();
        // The proven line interpreter is language-neutral after the header.
        parse_english_yml(&body, &mut entries);
        Ok(Interpreted {
            provenance: bytes.provenance,
            value: Localisation {
                entries,
                source_files: vec![expected.normalized_game_relative_path.clone()],
                parse_failures: Vec::new(),
            },
        })
    }

    pub fn parse_shader_effect(
        &self,
        expected: &UsedContentKey,
    ) -> Result<Interpreted<EffectFile>> {
        require_one_extension(expected, &["shader", "fxh"])?;
        let bytes = self.read(expected)?;
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
        let texture = self
            .assets
            .decode_texture(&expected.normalized_game_relative_path)?;
        // Re-read through the central seam so an on-disk mutation between
        // resolver construction and decode cannot escape admission checking.
        self.read(expected)?;
        Ok(Interpreted {
            provenance: vec![expected.clone()],
            value: texture,
        })
    }

    /// Load a symbolic BMFont through the admitted VFS. The caller supplies
    /// the exact descriptors/pages it expects this alias to consume; no font
    /// catalog or fallback path is embedded here.
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

    /// Compile the effective scripted-GUI directory with the existing proven
    /// evaluator. Every effective file must already be present in admission.
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

/// Grammar/semantic families intentionally not generalized. Their specialised
/// exact consumers may use [`ProductionContentInterpreter::read`], but product
/// code must not infer missing laws from vanilla examples.
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
        parse_english_yml(&body, &mut entries);
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
