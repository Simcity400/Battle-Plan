//! `common/raids/*.txt`: the raid types, for the map icon's sprite key.
//!
//! `common/raids/_documentation.md` states the icon rule:
//!
//! 1. `GFX_raid_type_icon_<raid name>`, or `custom_map_icon` when the type
//!    declares one;
//! 2. that key with `_<building template>` appended, preferred when the sprite
//!    set declares it.
//!
//! Only the name and `custom_map_icon` are read here; the rest of a raid type
//! is script the map does not draw.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    fields::{block, invalid, key_str, last},
    parser::{self, Value},
};

/// The directory the shipped raid types live in.
pub const RAIDS_DIR: &str = "common/raids/";

/// One raid type's map-icon inputs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaidType {
    pub name: String,
    /// `custom_map_icon`, the sprite key that replaces the name-derived one.
    pub custom_map_icon: Option<String>,
}

impl RaidType {
    /// The sprite key before the building-template suffix.
    pub fn base_icon(&self) -> String {
        match &self.custom_map_icon {
            Some(icon) => icon.clone(),
            None => format!("GFX_raid_type_icon_{}", self.name),
        }
    }

    /// The sprite key for this type against a target building template, using
    /// `known` to decide whether the specialised key exists.
    pub fn icon(&self, template: Option<&str>, known: impl Fn(&str) -> bool) -> String {
        let base = self.base_icon();
        if let Some(template) = template.filter(|template| !template.is_empty()) {
            let specialised = format!("{base}_{template}");
            if known(&specialised) {
                return specialised;
            }
        }
        base
    }
}

/// Every raid type the layers declare, by name.
pub fn read(layers: &ResolvedLayers) -> io::Result<BTreeMap<String, RaidType>> {
    let mut out = BTreeMap::new();
    let mut paths: Vec<String> = layers
        .paths()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.starts_with(RAIDS_DIR)
                && lower.ends_with(".txt")
                && !lower[RAIDS_DIR.len()..].contains('/')
        })
        .map(str::to_owned)
        .collect();
    paths.sort();
    for path in paths {
        let bytes = layers.read(&path)?;
        let document =
            parser::parse(&bytes).map_err(|error| invalid(format!("{path}: {error}")))?;
        for raid in parse(&document)? {
            out.insert(raid.name.clone(), raid);
        }
    }
    Ok(out)
}

/// The `types = { <name> = { ... } }` blocks of one parsed file.
pub fn parse(document: &parser::Document) -> io::Result<Vec<RaidType>> {
    let mut out = Vec::new();
    for root in crate::fields::all(&document.items, b"types") {
        for item in block(root, "types")? {
            let name = key_str(item, "types")?.to_owned();
            let items = block(&item.value, &name)?;
            out.push(RaidType {
                name,
                custom_map_icon: last(items, b"custom_map_icon")
                    .and_then(Value::scalar_str)
                    .map(str::to_owned),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"
types = {
    facility_strike = {
        days_to_prepare = 90
        custom_map_icon = GFX_raid_type_icon_facility_raid
    }
    dam_bombing_raid = {
        days_to_prepare = 30
    }
}
"#;

    #[test]
    fn the_icon_key_follows_the_documented_two_step_lookup() {
        let types = parse(&crate::parser::parse(SAMPLE).unwrap()).unwrap();
        let facility = &types[0];
        let dam = &types[1];
        // No custom icon: the key is derived from the raid type's own name.
        assert_eq!(dam.base_icon(), "GFX_raid_type_icon_dam_bombing_raid");
        assert_eq!(facility.base_icon(), "GFX_raid_type_icon_facility_raid");
        // The building template specialises the key only when it is declared.
        let known = |name: &str| name == "GFX_raid_type_icon_facility_raid_land_facility";
        assert_eq!(
            facility.icon(Some("land_facility"), known),
            "GFX_raid_type_icon_facility_raid_land_facility"
        );
        assert_eq!(
            facility.icon(Some("supply_node"), known),
            "GFX_raid_type_icon_facility_raid"
        );
        assert_eq!(
            facility.icon(None, known),
            "GFX_raid_type_icon_facility_raid"
        );
        assert_eq!(
            dam.icon(Some("land_facility"), known),
            "GFX_raid_type_icon_dam_bombing_raid"
        );
    }
}
