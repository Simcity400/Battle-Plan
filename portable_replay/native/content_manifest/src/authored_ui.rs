//! Portable XUI24 authored-window namespace and capability contract.
//!
//! These identities are shared by the recorder and replay admission. Keeping
//! them below both products prevents a string-only producer/consumer contract
//! from drifting while still leaving GUI parsing and composition to the UI
//! layer.

use crate::{
    normalize_game_relative_path, UsedContentKey, MAX_REQUIRED_CONTENT_ROWS,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

pub const GUI_NAMESPACE_CONSUMER: &str = "replay_app.authored_gui_effective_namespace.complete.v1";
pub const GUI_EMPTY_NAMESPACE_CONSUMER: &str =
    "replay_app.authored_gui_effective_namespace.empty.v1";
pub const GFX_NAMESPACE_CONSUMER: &str = "replay_app.authored_gfx_effective_namespace.complete.v1";
pub const GFX_EMPTY_NAMESPACE_CONSUMER: &str =
    "replay_app.authored_gfx_effective_namespace.empty.v1";
pub const LOCALISATION_NAMESPACE_CONSUMER: &str =
    "replay_app.authored_localisation_effective_namespace.complete.v1";
pub const LOCALISATION_EMPTY_NAMESPACE_CONSUMER: &str =
    "replay_app.authored_localisation_effective_namespace.empty.v1";
pub const SCRIPTED_LOCALISATION_NAMESPACE_CONSUMER: &str =
    "replay_app.authored_scripted_localisation_effective_namespace.complete.v1";
/// Authenticated representation of a valid effective VFS in which
/// `common/scripted_localisation/**/*.txt` has no winners (for example after
/// a selected mod's `replace_path`). The marker is carried by the complete GUI
/// namespace rows; replay verifies that carrier set and independently proves
/// that the scripted-localisation namespace is empty.
pub const SCRIPTED_LOCALISATION_EMPTY_NAMESPACE_CONSUMER: &str =
    "replay_app.authored_scripted_localisation_effective_namespace.empty.v1";
pub const DEPENDENCY_CLOSURE_CONSUMER: &str =
    "replay_app.authored_ui_dependency_closure.complete.v1";
pub const DEPENDENCY_CLOSURE_EMPTY_CONSUMER: &str =
    "replay_app.authored_ui_dependency_closure.empty.v1";
/// Per-file authenticated carrier for the case-preserved spelling used by the
/// engine's recursive definition enumeration. [`UsedContentKey`] intentionally
/// retains a lowercase portable identity, so this authenticated path digest
/// plus compact ASCII-case mask is the separate semantic input needed to
/// reproduce case-sensitive parse order. The digest makes the consumer unique
/// to one manifest row even when several paths have the same capitalization
/// mask.
pub const ENGINE_PATH_SPELLING_CONSUMER_PREFIX: &str =
    "replay_app.authored_ui_engine_path_spelling.v1/";
/// Exact-a729 bitmap-text effect selected by the text compositor. The path is
/// an engine resource identity; its bytes and winning layer are always derived
/// from the effective VFS and authenticated as ordinary content.
pub const AUTHORED_BITMAP_FONT_TEXT_EFFECT_PATH: &str = "gfx/FX/text.shader";
/// Exact graphics defines source consumed by the authenticated tooltip shell.
/// Its winning bytes remain ordinary effective-VFS content.
pub const AUTHORED_GRAPHICS_DEFINES_PATH: &str = "common/defines/00_graphics.lua";

#[derive(Serialize)]
struct LocalisationContentIdentityRow<'a> {
    normalized_game_relative_path: &'a str,
    payload_sha256: &'a str,
    payload_bytes: u64,
}

/// Hash the complete winning selected-language localisation namespace.
///
/// The caller must independently prove namespace completeness (by walking the
/// producer VFS or presenting XUI24's complete/empty capability). This helper
/// rejects rows outside that namespace, duplicate paths, and mixed effective
/// VFS authorities before hashing only the path-delimited content identity.
pub fn localisation_content_identity_sha256(
    localisation_language: &str,
    rows: &[UsedContentKey],
) -> Result<String, String> {
    let language = canonical_localisation_language(localisation_language)?;
    if rows.len() > MAX_REQUIRED_CONTENT_ROWS {
        return Err("localisation content identity exceeds the deterministic row bound".into());
    }
    let mut ordered = rows.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.normalized_game_relative_path
            .cmp(&right.normalized_game_relative_path)
    });
    let mut authority = None;
    let mut previous_path = None;
    for row in &ordered {
        row.validate()?;
        if classify_authored_ui_path(&row.normalized_game_relative_path, &language)?
            != Some(AuthoredUiNamespace::Localisation)
        {
            return Err(format!(
                "localisation content identity contains an out-of-namespace row {:?}",
                row.normalized_game_relative_path
            ));
        }
        if previous_path == Some(row.normalized_game_relative_path.as_str()) {
            return Err(format!(
                "localisation content identity duplicates path {:?}",
                row.normalized_game_relative_path
            ));
        }
        previous_path = Some(row.normalized_game_relative_path.as_str());
        match authority {
            None => authority = Some(row.effective_vfs_manifest_sha256.as_str()),
            Some(expected) if expected == row.effective_vfs_manifest_sha256 => {}
            Some(_) => {
                return Err(
                    "localisation content identity mixes effective-VFS authorities".into(),
                )
            }
        }
    }
    let canonical_rows = ordered
        .iter()
        .map(|row| LocalisationContentIdentityRow {
            normalized_game_relative_path: &row.normalized_game_relative_path,
            payload_sha256: &row.payload_sha256,
            payload_bytes: row.payload_bytes,
        })
        .collect::<Vec<_>>();
    let canonical = serde_json::json!({
        "schema": "hoi4.d98-theatre-selector-localisation-content.v1",
        "language": language,
        "rows": canonical_rows,
    });
    let bytes = serde_json::to_vec(&canonical).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthoredUiNamespace {
    Gui,
    Gfx,
    Localisation,
    ScriptedLocalisation,
}

impl AuthoredUiNamespace {
    pub fn consumer(self) -> &'static str {
        match self {
            Self::Gui => GUI_NAMESPACE_CONSUMER,
            Self::Gfx => GFX_NAMESPACE_CONSUMER,
            Self::Localisation => LOCALISATION_NAMESPACE_CONSUMER,
            Self::ScriptedLocalisation => SCRIPTED_LOCALISATION_NAMESPACE_CONSUMER,
        }
    }

    pub fn empty_consumer(self) -> &'static str {
        match self {
            Self::Gui => GUI_EMPTY_NAMESPACE_CONSUMER,
            Self::Gfx => GFX_EMPTY_NAMESPACE_CONSUMER,
            Self::Localisation => LOCALISATION_EMPTY_NAMESPACE_CONSUMER,
            Self::ScriptedLocalisation => SCRIPTED_LOCALISATION_EMPTY_NAMESPACE_CONSUMER,
        }
    }
}

/// Classify a retained effective source path into the authored-window
/// definition namespaces used by the exact a729 engine.
///
/// Selected mod/DLC content appears at the same root logical namespace as base
/// content. The winning [`crate::UsedContentKey`] retains which layer supplied
/// those bytes. Raw install paths below `dlc/` or `integrated_dlc/` are not
/// product VFS identities and must not enter this classifier.
pub fn classify_authored_ui_path(
    path: &str,
    localisation_language: &str,
) -> Result<Option<AuthoredUiNamespace>, String> {
    let path = normalize_game_relative_path(path)?;
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() >= 2 && parts[0].eq_ignore_ascii_case("interface") {
        let extension = parts
            .last()
            .and_then(|file| file.rsplit_once('.'))
            .map(|(_, extension)| extension);
        return Ok(match extension {
            Some(extension) if extension.eq_ignore_ascii_case("gui") => {
                Some(AuthoredUiNamespace::Gui)
            }
            Some(extension) if extension.eq_ignore_ascii_case("gfx") => {
                Some(AuthoredUiNamespace::Gfx)
            }
            _ => None,
        });
    }
    let language = safe_language_directory(localisation_language)?;
    if parts.len() >= 3
        && parts[0].eq_ignore_ascii_case("localisation")
        && parts[1].eq_ignore_ascii_case(&language)
        && parts
            .last()
            .is_some_and(|file| file.to_ascii_lowercase().ends_with(".yml"))
    {
        return Ok(Some(AuthoredUiNamespace::Localisation));
    }
    if parts.len() >= 3
        && parts[0].eq_ignore_ascii_case("common")
        && parts[1].eq_ignore_ascii_case("scripted_localisation")
        && parts
            .last()
            .is_some_and(|file| file.to_ascii_lowercase().ends_with(".txt"))
    {
        return Ok(Some(AuthoredUiNamespace::ScriptedLocalisation));
    }
    Ok(None)
}

pub fn safe_language_directory(language: &str) -> Result<String, String> {
    let language = language.trim().to_ascii_lowercase();
    let directory = language.strip_prefix("l_").unwrap_or(&language);
    if directory.is_empty()
        || !directory
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err(
            "authored observer UI playback language is not a safe Clausewitz language identity"
                .into(),
        );
    }
    Ok(directory.to_string())
}

/// Canonical Clausewitz localisation identity shared by recorder and replay.
///
/// Playback metadata may carry either `english` or `l_english` and may differ
/// in ASCII case/outer whitespace. Directory selection and the required YML
/// header must be derived from one canonical identity so those representations
/// cannot select the same directory but disagree on which files are valid.
pub fn canonical_localisation_language(language: &str) -> Result<String, String> {
    Ok(format!("l_{}", safe_language_directory(language)?))
}

/// Exact-a729 recursive PHYSFS enumeration order for authored definition
/// files. Each path component is compared case-sensitively; a directory is
/// visited before a same-prefix sibling file. Registry identities themselves
/// remain case-insensitive and the last parsed definition wins.
pub fn engine_recursive_path_cmp(left: &str, right: &str) -> Ordering {
    let left = left.replace('\\', "/");
    let right = right.replace('\\', "/");
    let mut left_parts = left.trim_start_matches('/').split('/');
    let mut right_parts = right.trim_start_matches('/').split('/');
    loop {
        match (left_parts.next(), right_parts.next()) {
            (Some(left), Some(right)) => {
                let order = left.cmp(right);
                if order != Ordering::Equal {
                    return order;
                }
            }
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => return Ordering::Equal,
        }
    }
}

pub fn engine_path_spelling_consumer(path: &str) -> Result<String, String> {
    let preserved = path.trim().replace('\\', "/");
    let normalized = normalize_game_relative_path(&preserved)?;
    let mut mask = vec![0u8; preserved.len().div_ceil(8)];
    for (index, (actual, folded)) in preserved.bytes().zip(normalized.bytes()).enumerate() {
        if actual == folded {
            continue;
        }
        if !actual.is_ascii_uppercase() || actual.to_ascii_lowercase() != folded {
            return Err(format!(
                "authored engine path spelling is not an ASCII-case variant: {path:?}"
            ));
        }
        mask[index / 8] |= 1 << (index % 8);
    }
    let encoded = mask
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let normalized_sha256 = format!("{:x}", Sha256::digest(normalized.as_bytes()));
    Ok(format!(
        "{ENGINE_PATH_SPELLING_CONSUMER_PREFIX}{normalized_sha256}/{encoded}"
    ))
}

pub fn decode_engine_path_spelling_consumer(
    consumer: &str,
    normalized_path: &str,
) -> Result<Option<String>, String> {
    let Some(encoded) = consumer.strip_prefix(ENGINE_PATH_SPELLING_CONSUMER_PREFIX) else {
        return Ok(None);
    };
    let normalized = normalize_game_relative_path(normalized_path)?;
    let (claimed_path_sha256, encoded) = encoded
        .split_once('/')
        .ok_or_else(|| "authored engine path spelling carrier lacks its path digest".to_string())?;
    if claimed_path_sha256.len() != 64
        || !claimed_path_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("authored engine path spelling carrier has invalid path digest".into());
    }
    let expected_path_sha256 = format!("{:x}", Sha256::digest(normalized.as_bytes()));
    if claimed_path_sha256 != expected_path_sha256 {
        return Err("authored engine path spelling carrier names a different path".into());
    }
    let expected_bytes = normalized.len().div_ceil(8);
    if encoded.len() != expected_bytes.saturating_mul(2)
        || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("authored engine path spelling mask has invalid length or hex".into());
    }
    let mut mask = Vec::with_capacity(expected_bytes);
    for index in 0..expected_bytes {
        mask.push(
            u8::from_str_radix(&encoded[index * 2..index * 2 + 2], 16)
                .map_err(|_| "authored engine path spelling mask is invalid")?,
        );
    }
    if normalized.len() % 8 != 0 {
        let used = normalized.len() % 8;
        let unused_mask = !((1u8 << used) - 1);
        if mask.last().is_some_and(|last| last & unused_mask != 0) {
            return Err("authored engine path spelling mask sets trailing bits".into());
        }
    }
    let mut restored = normalized.into_bytes();
    for (index, byte) in restored.iter_mut().enumerate() {
        if mask[index / 8] & (1 << (index % 8)) == 0 {
            continue;
        }
        if !byte.is_ascii_lowercase() {
            return Err("authored engine path spelling mask uppercases a non-letter".into());
        }
        *byte = byte.to_ascii_uppercase();
    }
    let restored = String::from_utf8(restored)
        .map_err(|_| "authored engine path spelling is not UTF-8".to_string())?;
    Ok(Some(restored))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_only_effective_root_namespaces() {
        assert_eq!(
            classify_authored_ui_path("interface/mod_panel.gui", "l_english").unwrap(),
            Some(AuthoredUiNamespace::Gui)
        );
        assert_eq!(
            classify_authored_ui_path(
                "integrated_dlc/dlc023_man_the_guns/interface/mtg_music.gui",
                "l_english"
            )
            .unwrap(),
            None
        );
        assert_eq!(
            classify_authored_ui_path(
                "dlc/dlc019_sabaton_vol2/interface/sabaton_music.gui",
                "l_english"
            )
            .unwrap(),
            None
        );
        assert_eq!(
            classify_authored_ui_path(
                "mod/arbitrary/interface/not_an_effective_source.gui",
                "l_english"
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn engine_path_spelling_mask_round_trips_without_changing_portable_identity() {
        let spelling = "Interface/Zeta/A.gui";
        let consumer = engine_path_spelling_consumer(spelling).unwrap();
        let normalized = normalize_game_relative_path(spelling).unwrap();
        assert_eq!(
            decode_engine_path_spelling_consumer(&consumer, &normalized).unwrap(),
            Some(spelling.replace('\\', "/"))
        );
        let lower_one = engine_path_spelling_consumer("interface/alpha/a.gui").unwrap();
        let lower_two = engine_path_spelling_consumer("interface/bravo/b.gui").unwrap();
        assert_ne!(lower_one, lower_two);
        assert_eq!(
            lower_one.rsplit_once('/').unwrap().1,
            lower_two.rsplit_once('/').unwrap().1,
            "the path digest, not a coincidentally different case mask, must make carriers unique"
        );
        assert!(decode_engine_path_spelling_consumer(
            &format!("{ENGINE_PATH_SPELLING_CONSUMER_PREFIX}ff"),
            "a"
        )
        .is_err());
        let (_, payload) = consumer
            .strip_prefix(ENGINE_PATH_SPELLING_CONSUMER_PREFIX)
            .unwrap()
            .split_once('/')
            .unwrap();
        assert!(decode_engine_path_spelling_consumer(
            &format!(
                "{ENGINE_PATH_SPELLING_CONSUMER_PREFIX}{}/{payload}",
                "0".repeat(64)
            ),
            &normalized
        )
        .is_err());
        assert!(decode_engine_path_spelling_consumer(&consumer, "interface/other/a.gui").is_err());
        let uppercase_digest = consumer.replacen(
            consumer
                .strip_prefix(ENGINE_PATH_SPELLING_CONSUMER_PREFIX)
                .unwrap()
                .split_once('/')
                .unwrap()
                .0,
            &"A".repeat(64),
            1,
        );
        assert!(
            decode_engine_path_spelling_consumer(&uppercase_digest, &normalized).is_err(),
            "path digests must use canonical lowercase SHA-256"
        );
    }

    #[test]
    fn localisation_capability_is_language_scoped() {
        assert_eq!(
            classify_authored_ui_path("localisation/braz_por/panel_l_braz_por.yml", "l_braz_por")
                .unwrap(),
            Some(AuthoredUiNamespace::Localisation)
        );
        assert_eq!(
            classify_authored_ui_path("localisation/english/panel_l_english.yml", "l_braz_por")
                .unwrap(),
            None
        );
    }

    #[test]
    fn localisation_identity_is_canonical_for_directory_and_header_use() {
        assert_eq!(
            canonical_localisation_language("  L_BRAZ_POR  ").unwrap(),
            "l_braz_por"
        );
        assert_eq!(
            canonical_localisation_language("english").unwrap(),
            "l_english"
        );
    }

    fn localisation_row(path: &str, payload: char) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: crate::ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: payload.to_string().repeat(64),
            payload_sha256: payload.to_string().repeat(64),
            payload_bytes: 17,
        }
    }

    #[test]
    fn d98_localisation_identity_is_order_independent_and_content_sensitive() {
        let alpha = localisation_row(
            "localisation/english/alpha_l_english.yml",
            '1',
        );
        let bravo = localisation_row(
            "localisation/english/bravo_l_english.yml",
            '2',
        );
        let forward = localisation_content_identity_sha256(
            "l_english",
            &[alpha.clone(), bravo.clone()],
        )
        .unwrap();
        let reverse =
            localisation_content_identity_sha256("english", &[bravo.clone(), alpha.clone()])
                .unwrap();
        assert_eq!(forward, reverse);

        let mut changed = bravo;
        changed.payload_sha256 = "3".repeat(64);
        assert_ne!(
            forward,
            localisation_content_identity_sha256("l_english", &[alpha, changed]).unwrap()
        );
        assert_ne!(
            forward,
            localisation_content_identity_sha256("l_english", &[]).unwrap(),
            "authenticated-empty is a distinct exact namespace identity"
        );
    }

    #[test]
    fn d98_localisation_identity_rejects_wrong_namespace_duplicate_and_mixed_vfs() {
        let alpha = localisation_row(
            "localisation/english/alpha_l_english.yml",
            '1',
        );
        assert!(localisation_content_identity_sha256(
            "l_english",
            &[localisation_row("localisation/french/alpha_l_french.yml", '1')]
        )
        .is_err());
        assert!(
            localisation_content_identity_sha256("l_english", &[alpha.clone(), alpha.clone()])
                .is_err()
        );
        let mut mixed = localisation_row(
            "localisation/english/bravo_l_english.yml",
            '2',
        );
        mixed.effective_vfs_manifest_sha256 = "b".repeat(64);
        assert!(
            localisation_content_identity_sha256("l_english", &[alpha, mixed]).is_err()
        );
    }

    #[test]
    fn exact_a729_recursive_order_is_componentwise_and_case_sensitive() {
        let mut paths = [
            "interface/a.gui",
            "interface/a/nested.gui",
            "interface/Z.gui",
        ];
        paths.sort_by(|left, right| engine_recursive_path_cmp(left, right));
        assert_eq!(
            paths,
            [
                "interface/Z.gui",
                "interface/a/nested.gui",
                "interface/a.gui",
            ]
        );
    }
}
