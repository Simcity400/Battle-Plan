//! Authored sub-unit file and key reads shared by counters and model content.
use crate::invalid;
use installed_content_core::{ResolvedLayers, parser};
use std::{collections::BTreeSet, io};

/// Sub-units live in the files directly under `common/units`; its child
/// directories hold equipment, historical names and unrelated data.
pub fn is_subunit_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".txt")
        && lower
            .strip_prefix("common/units/")
            .is_some_and(|relative| !relative.contains('/'))
}

/// Every authored sub-unit key, from the files [`crate::subunit_requirements::Content::load`] reads sub-units
/// from, once each.
pub fn subunit_keys(layers: &ResolvedLayers) -> io::Result<BTreeSet<String>> {
    let mut keys = BTreeSet::new();
    for path in subunit_paths(layers.paths()) {
        let document = parser::parse(&layers.read(path)?)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        for root in document
            .items
            .iter()
            .filter(|root| root.key.as_deref() == Some(b"sub_units"))
        {
            for item in root.value.items().unwrap_or_default() {
                if let Some(key) = item.key.as_deref() {
                    keys.insert(
                        std::str::from_utf8(key)
                            .map_err(|_| invalid("non-UTF8 authored model key"))?
                            .to_owned(),
                    );
                }
            }
        }
    }
    Ok(keys)
}

/// ResolvedLayers has already applied same-path mount replacement;
/// The game then reads the effective files in path byte order.
pub fn subunit_paths<'a>(paths: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut paths: Vec<_> = paths.filter(|path| is_subunit_file(path)).collect();
    paths.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sub_units_come_from_the_direct_unit_files_in_path_byte_order() {
        for path in ["common/units/infantry.txt", "common/units/Armor.TXT"] {
            assert!(is_subunit_file(path), "{path}");
        }
        for path in [
            "common/units/equipment/infantry.txt",
            "common/units/equipment/modules/00_tank_modules.txt",
            "common/units/names_divisions/BRA_names_divisions.txt",
            "common/units/codenames_operatives/GER.txt",
            "common/units/infantry.asset",
        ] {
            assert!(!is_subunit_file(path), "{path}");
        }
        assert_eq!(
            subunit_paths(
                [
                    "common/units/z_base.txt",
                    "common/units/equipment/a.txt",
                    "common/units/B.txt",
                ]
                .into_iter()
            ),
            ["common/units/B.txt", "common/units/z_base.txt"]
        );
    }
}
