//! Authored sub-unit key reads shared by counters and model content.
use crate::invalid;
use installed_content_core::{ResolvedLayers, parser};
use std::{collections::BTreeSet, io};

/// Every authored sub-unit key in the sub-unit database's files, once each.
pub fn subunit_keys(layers: &ResolvedLayers) -> io::Result<BTreeSet<String>> {
    let mut keys = BTreeSet::new();
    for path in installed_content_core::unit_leader::sub_unit_files(layers) {
        let document = parser::parse(&path.read(layers)?)
            .map_err(|error| invalid(format!("{}: {error}", path.text())))?;
        for root in document
            .items
            .iter()
            .filter(|root| root.keyword_is("sub_units"))
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
