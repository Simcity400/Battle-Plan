
pub use portable_content_manifest::paradox_text::*;

use crate::{GameUiError, Result};

pub fn parse_checked_bytes(bytes: &[u8], source: &str) -> Result<Document> {
    reject_decode_damage(bytes, source)?;
    Ok(parse_bytes(bytes))
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
