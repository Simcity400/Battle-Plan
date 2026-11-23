use content_reader_parser::parser::c_string;
use content_reader_localisation::{localisation::Localisation, localisation::Unlocalised};
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

pub fn victory_point_keys(tag: Option<&str>, province: u32) -> Vec<String> {
    match tag.filter(|tag| !tag.is_empty()) {
        Some(tag) => vec![
            format!("{tag}_VICTORY_POINTS_{province}"),
            format!("VICTORY_POINTS_{province}"),
        ],
        None => vec![format!("VICTORY_POINTS_{province}")],
    }
}

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

pub fn overrides_ladder(bytes: &[u8]) -> bool {
    !bytes.is_empty() && !unreadable(bytes)
}

pub fn unreadable(bytes: &[u8]) -> bool {
    let key = c_string(bytes);
    !key.is_empty() && std::str::from_utf8(key).is_err()
}
