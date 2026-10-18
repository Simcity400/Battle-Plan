//! Anchor placement and province label naming shared by map labels and counter hovers.
use installed_content_core::parser::c_string;
use installed_content_localisation::{localisation::Localisation, localisation::Unlocalised};
use std::collections::BTreeMap;

pub const VICTORY_POINT: u32 = 0x26;

pub fn label_anchors(
    provinces: impl Iterator<Item = u32>,
    anchor: impl Fn(u32) -> Option<[f32; 3]>,
    seeded: impl Fn(u32) -> bool,
) -> (BTreeMap<u32, [f32; 3]>, Vec<u32>) {
    let mut anchors = BTreeMap::new();
    let mut fallbacks = Vec::new();
    for province in provinces.filter(|&province| province != 0) {
        if let Some(at) = anchor(province) {
            anchors.insert(province, at);
            if seeded(province) {
                fallbacks.push(province);
            }
        }
    }
    fallbacks.sort_unstable();
    (anchors, fallbacks)
}

/// Try `<TAG>_VICTORY_POINTS_<id>` before `VICTORY_POINTS_<id>`, using the
/// same owner-override pattern as state names. The caller takes the first
/// key its localisation has.
pub fn victory_point_keys(tag: Option<&str>, province: u32) -> Vec<String> {
    match tag.filter(|tag| !tag.is_empty()) {
        Some(tag) => vec![
            format!("{tag}_VICTORY_POINTS_{province}"),
            format!("VICTORY_POINTS_{province}"),
        ],
        None => vec![format!("VICTORY_POINTS_{province}")],
    }
}

/// A recorded rename replaces the static key ladder. Localise it as a key,
/// falling back to its literal bytes after the first-NUL cut. Otherwise use
/// the first declared key from [`victory_point_keys`], or draw no name if none
/// exists. Label text keeps brackets as glyphs.
pub fn label_name(
    localisation: &Localisation,
    recorded: Option<&[u8]>,
    tag: Option<&str>,
    province: u32,
) -> Result<Option<String>, Unlocalised> {
    if let Some(bytes) = recorded.filter(|bytes| overrides_ladder(bytes)) {
        return localisation.localise(bytes).map(Some);
    }
    let Some(key) = victory_point_keys(tag, province)
        .into_iter()
        .find(|key| localisation.get(key).is_some())
    else {
        return Ok(None);
    };
    localisation.localise(key.as_bytes()).map(Some)
}

/// Whether a recorded override replaces the static ladder here: a non-empty
/// one the port can read.
pub fn overrides_ladder(bytes: &[u8]) -> bool {
    !bytes.is_empty() && !unreadable(bytes)
}

/// Whether a recorded override is bytes this port cannot draw as text.
///
/// Only a non-UTF-8 string qualifies: an empty one and one that starts with a
/// NUL are names the *engine* does not draw either, so they are not a shortfall
/// of the port and are not reported as one.
pub fn unreadable(bytes: &[u8]) -> bool {
    let key = c_string(bytes);
    !key.is_empty() && std::str::from_utf8(key).is_err()
}
