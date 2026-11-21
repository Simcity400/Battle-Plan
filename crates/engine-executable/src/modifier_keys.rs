
pub use engine_reference_values::modifier_keys::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub index: u32,
    pub name: &'static str,
}

pub fn names() -> std::io::Result<&'static [&'static str]> {
    Ok(&crate::executable_tables::keywords()?.modifier_names)
}

pub fn fixed_name(index: u32) -> std::io::Result<&'static str> {
    crate::executable_tables::keywords()?
        .fixed_modifiers()?
        .get(index as usize)
        .copied()
        .ok_or_else(|| std::io::Error::other(format!("modifier {index:#x} is not fixed")))
}

pub fn all() -> std::io::Result<impl Iterator<Item = Key>> {
    let names = crate::executable_tables::keywords()?.fixed_modifiers()?;
    Ok(ALL_INDICES.iter().map(move |&index| Key {
        index,
        name: names[index as usize],
    }))
}

#[cfg(test)]
mod tests {
    use crate::executable_tables::{self, synthetic};

    fn entered(pinned: bool) {
        let names = (0..crate::build::FIXED_MODIFIER_GAP)
            .map(|index| executable_tables::leak_string(format!("synthetic_modifier_{index}")))
            .collect();
        let tables = synthetic::Parts::empty()
            .keywords(synthetic::keyword_table(Vec::new(), names, pinned))
            .assemble();
        executable_tables::enter(Some(tables));
    }

    #[test]
    fn fixed_keys_name_the_pinned_row_and_are_refused_on_an_unpinned_table() {
        entered(true);
        assert_eq!(
            super::fixed_name(super::MILITARY_INDUSTRIAL_ORGANIZATION_POLICY_COST).unwrap(),
            format!(
                "synthetic_modifier_{}",
                super::MILITARY_INDUSTRIAL_ORGANIZATION_POLICY_COST
            )
        );
        assert!(super::fixed_name(crate::build::FIXED_MODIFIER_GAP).is_err());
        let key = super::all().unwrap().next().unwrap();
        assert_eq!(key.name, format!("synthetic_modifier_{}", key.index));
        entered(false);
        assert!(super::fixed_name(0).is_err());
        assert!(super::all().is_err());
        assert_eq!(super::names().unwrap().len(), crate::build::FIXED_MODIFIER_GAP as usize);
    }
}
