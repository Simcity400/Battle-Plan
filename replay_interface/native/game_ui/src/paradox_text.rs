
pub use portable_content_manifest::paradox_text::*;

use crate::{GameUiError, Result};
use std::path::Path;

pub fn parse_file(path: &Path) -> Result<Document> {
    let bytes = std::fs::read(path)?;
    reject_decode_damage(&bytes, &path.display().to_string())?;
    Ok(parse_bytes(&bytes))
}

pub fn parse_file_refusing_decode_damage(path: &Path, rel_path: &str) -> Result<Document> {
    let bytes = std::fs::read(path)?;
    reject_decode_damage(&bytes, rel_path)?;
    Ok(parse_bytes(&bytes))
}

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
