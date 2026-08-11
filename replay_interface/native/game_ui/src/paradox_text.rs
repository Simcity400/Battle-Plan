//! Clausewitz text parsing for the UI layer.
//!
//! Re-exports the portable parser, but DELIBERATELY SHADOWS `parse_file` with a
//! version that refuses byte-level decode damage. See [`parse_file`].

pub use portable_content_manifest::paradox_text::*;

use crate::{GameUiError, Result};
use std::path::Path;

/// Parse one Clausewitz file, REFUSING content whose bytes were destroyed at
/// decode.
///
/// This shadows `portable_content_manifest::paradox_text::parse_file` for the
/// whole UI crate, and that is the point: making the safe behaviour the DEFAULT
/// is the only thing that converges here. Decode damage is invisible to the
/// parser — a U+FFFD substituted for a destroyed byte is an ordinary character
/// to the tokenizer — so a damaged identifier, sprite name, `replace_path`,
/// colour key or atlas path parses with ZERO recoveries and becomes a trusted
/// definition naming something that was never in the file. Auditing each of the
/// crate's loaders to remember a separate check would make the safety property
/// caller-dependent; keeping it inside the shared function makes it uniform.
///
/// What is refused is narrow and measured:
///
/// * Only DESTRUCTIVE loss (`TextDecodeLoss::discards_input`). Lossless Latin-1
///   attribution is NOT damage — the shipped game itself writes raw `0xA7` `§`
///   colour codes, and the fallback preserves every byte — so correct Latin-1
///   content is never refused.
/// * MEASURED against the shipped the game 1.19.2 install with the shipping
///   `diagnose_text_decode`: 8798 text files (all 10 localisation languages;
///   `.gui` `.gfx` `.txt` `.yml` `.fnt` `.shader` `.fxh` `.asset`, plus the one
///   packed archive) produce ZERO destructive decode events. So this cannot
///   refuse the base game — the trap that made a stricter `.gui` parse gate be
///   withdrawn, since four shipped `.gui` files genuinely need parse recovery.
///   Decode damage is not like that: no shipped file has any.
///
/// `decode_text` itself is untouched and byte-identical, and the HUD closure
/// emitters keep their record-and-mark-incomplete channel rather than refusing,
/// because there the design intent is to bind the content and withdraw the
/// completeness claim. Callers that deliberately want the unchecked parse can
/// still name `portable_content_manifest::paradox_text::parse_file` in full.
pub fn parse_file(path: &Path) -> Result<Document> {
    let bytes = std::fs::read(path)?;
    reject_decode_damage(&bytes, &path.display().to_string())?;
    Ok(parse_bytes(&bytes))
}

/// [`parse_file`] with an explicit logical path for the error message.
pub fn parse_file_refusing_decode_damage(path: &Path, rel_path: &str) -> Result<Document> {
    let bytes = std::fs::read(path)?;
    reject_decode_damage(&bytes, rel_path)?;
    Ok(parse_bytes(&bytes))
}

/// Refuse bytes that could not be decoded without loss. Shared by the loaders
/// that read content themselves rather than through [`parse_file`].
pub fn reject_decode_damage(bytes: &[u8], source: &str) -> Result<()> {
    let events = destructive_event_count(&diagnose_text_decode(bytes));
    if events == 0 {
        return Ok(());
    }
    let first = destructive_losses(&diagnose_text_decode(bytes))
        .first()
        .map(ToString::to_string)
        .unwrap_or_default();
    Err(GameUiError::parse(format!(
        "{source}: decode destroyed {events} byte sequences: {first}"
    )))
}
