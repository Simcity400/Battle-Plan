//! Text shared by combat windows and ordinary-map hovers. Call sites own native keys and scopes.
use dashboard::scripted_text::stored_text;
use installed_content::localisation::{Argument as Arg, Localisation};

// One declaration supplies the producer's keys and the preload inventory.
macro_rules! tooltip_keys {
    ($($key:ident = $text:literal),* $(,)?) => {
        $(const $key: &str = $text;)*
        pub(super) const TOOLTIP_KEYS: &[&str] = &[$($key),*];
    };
}
pub(crate) use tooltip_keys;

/// CState::display_name (1409CC380): the custom name (+0x38), else the owner
/// name (+0xE8), else the template's. set_state_name (140310B50) stores the
/// localised text of its argument as the custom name; SetOwner (1409D1710,
/// also on load) has 1409D3770 store the localised `<owner original tag or
/// tag>_<template name key>` as the owner name, or clear it when the key is
/// missing. Both are [`stored_text`]: never localised again, nor rebuilt from
/// today's owner.
pub(crate) fn state_name(
    loc: &Localisation,
    recorded: &str,
    id: u32,
    custom: &[u8],
    owner: &[u8],
    template: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    for (bytes, what) in [
        (custom, "the custom name (CState+0x38)"),
        (owner, "the owner name (CState+0xE8)"),
    ] {
        if !bytes.is_empty() {
            return stored_text(loc, recorded, bytes, &format!("state {id}: {what}"));
        }
    }
    template()
}

pub(crate) fn localise(
    loc: &Localisation,
    key: &str,
    args: &[(&str, Arg<'_>)],
) -> Result<String, String> {
    loc.localise_with(key.as_bytes(), args)
        .map_err(|e| e.to_string())
}

/// EnsureTrailingNewlines@1424B6660 adds only missing trailing newlines.
pub(crate) fn ensure_newlines(text: &mut String, count: usize) {
    if !text.is_empty() {
        let have = text.bytes().rev().take_while(|&b| b == b'\n').count();
        for _ in have..count {
            text.push('\n');
        }
    }
}

/// 140BBA780: qualify with the simulation tag before trying the plain key.
pub(crate) fn country_localised(
    loc: &Localisation,
    tag: Result<&str, &str>,
    key: &str,
) -> Result<String, String> {
    let tag = tag.map_err(|reason| {
        format!("{key}'s name needs its country's TAG_{key} key, but {reason}")
    })?;
    let qualified = format!("{tag}_{key}");
    localise(
        loc,
        if loc.get(&qualified).is_some() {
            &qualified
        } else {
            key
        },
        &[],
    )
}

/// 141580360 preserves signed integer limits, including an invalid interval.
pub(crate) fn value_limit(
    loc: &Localisation,
    min: Option<i32>,
    max: Option<i32>,
) -> Result<String, String> {
    let key = match (min, max) {
        (None, None) => return Ok(String::new()),
        (Some(_), None) => "value_limit_min",
        (None, Some(_)) => "value_limit_max",
        (Some(a), Some(b)) if b < a => "value_limit_invalid",
        (Some(a), Some(b)) if a == b => "value_limit_exact",
        _ => "value_limit_min_max",
    };
    let mut args = Vec::new();
    if let Some(n) = min {
        args.push(("MIN", Arg::Int32(n)));
    }
    if let Some(n) = max {
        args.push(("MAX", Arg::Int32(n)));
    }
    localise(loc, key, &args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_state_names_precede_the_template_in_the_recording_language_only() {
        let mut loc = Localisation::default();
        loc.language = "english".into();
        let name = |custom: &[u8], owner: &[u8], recorded| {
            state_name(&loc, recorded, 7, custom, owner, || Ok("template".into()))
        };
        // The stored bytes are drawn as they are, never localised again.
        assert_eq!(
            name(b"$literal$", b"owner", "english").as_deref(),
            Ok("$literal$")
        );
        assert_eq!(name(b"", b"owner", "english").as_deref(), Ok("owner"));
        assert_eq!(name(b"", b"", "english").as_deref(), Ok("template"));
        assert_eq!(name(b"", b"", "german").as_deref(), Ok("template"));
        assert_eq!(
            name(b"", b"Bohmen", "german"),
            Err("state 7: the owner name (CState+0xE8) is text the game stored in german without its key, and the viewer's language is english".into())
        );
        assert!(
            name(b"Sudetenland", b"", "german")
                .unwrap_err()
                .contains("custom name (CState+0x38)")
        );
        assert_eq!(
            name(&[255], b"owner", "english"),
            Err("state 7: the custom name (CState+0x38) is not UTF-8".into())
        );
    }
}
