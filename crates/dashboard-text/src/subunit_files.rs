//! Authored sub-unit file and key reads shared by counters and model content.
use crate::invalid;
use installed_content_core::{ResolvedLayers, parser};
use std::{collections::BTreeSet, io};

/// Sub-units live in the files directly under `common/units`; its child
/// directories hold equipment, historical names and unrelated data.
pub fn is_subunit_file(path: &[u8]) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(b".txt")
        && lower
            .strip_prefix(b"common/units/")
            .is_some_and(|relative| !relative.contains(&b'/'))
}

/// Every authored sub-unit key, from the files [`crate::subunit_requirements::Content::load`] reads sub-units
/// from, once each.
pub fn subunit_keys(layers: &ResolvedLayers) -> io::Result<BTreeSet<String>> {
    let mut keys = BTreeSet::new();
    for path in subunit_paths(layers.paths()) {
        let document = parser::parse(&layers.read_path(path)?).map_err(|error| {
            invalid(format!("{}: {error}", String::from_utf8_lossy(path)))
        })?;
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
pub fn subunit_paths<'a>(paths: impl Iterator<Item = &'a [u8]>) -> Vec<&'a [u8]> {
    let mut paths: Vec<_> = paths.filter(|path| is_subunit_file(path)).collect();
    paths.sort();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sub_units_come_from_the_direct_unit_files_in_path_byte_order() {
        for path in ["common/units/infantry.txt", "common/units/Armor.TXT"] {
            assert!(is_subunit_file(path.as_bytes()), "{path}");
        }
        for path in [
            "common/units/equipment/infantry.txt",
            "common/units/equipment/modules/00_tank_modules.txt",
            "common/units/names_divisions/BRA_names_divisions.txt",
            "common/units/codenames_operatives/GER.txt",
            "common/units/infantry.asset",
        ] {
            assert!(!is_subunit_file(path.as_bytes()), "{path}");
        }
        assert_eq!(
            subunit_paths(
                [
                    "common/units/z_base.txt",
                    "common/units/equipment/a.txt",
                    "common/units/B.txt",
                ]
                .into_iter()
                .map(str::as_bytes)
            ),
            [
                b"common/units/B.txt".as_slice(),
                b"common/units/z_base.txt".as_slice()
            ]
        );
    }
}
