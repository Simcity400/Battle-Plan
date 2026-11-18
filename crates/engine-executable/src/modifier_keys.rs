//! Built-in modifier indices consumed by fixed-key readers and playback.
//!
//! Fixed names come from verified executable metadata, independently of content
//! order. Dynamic keys come from live data.

pub use engine_reference_values::modifier_keys::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub index: u32,
    pub name: &'static str,
}

/// Fixed modifier names in the executable's own index order.
pub fn names() -> std::io::Result<&'static [&'static str]> {
    Ok(&crate::executable_tables::keywords()?.modifier_names)
}

/// The name of a fixed modifier index, while the modifier table matches this release's.
pub fn fixed_name(index: u32) -> std::io::Result<&'static str> {
    crate::executable_tables::keywords()?
        .fixed_modifiers()?
        .get(index as usize)
        .copied()
        .ok_or_else(|| std::io::Error::other(format!("modifier {index:#x} is not fixed")))
}

/// The fixed modifier indices, which hold only while the modifier table matches this release's.
pub fn all() -> std::io::Result<impl Iterator<Item = Key>> {
    let names = crate::executable_tables::keywords()?.fixed_modifiers()?;
    Ok(ALL_INDICES.iter().map(move |&index| Key {
        index,
        name: names[index as usize],
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn organisation_cost_keys_name_their_verified_modifiers() {
        crate::executable_tables::initialize_for_tests()
            .expect("modifier name tests require BATTLE_PLAN_GAME_DIR with the supported installation");
        assert_eq!(
            super::fixed_name(super::MILITARY_INDUSTRIAL_ORGANIZATION_POLICY_COST).unwrap(),
            "military_industrial_organization_policy_cost"
        );
        assert_eq!(
            super::fixed_name(super::MILITARY_INDUSTRIAL_ORGANIZATION_DESIGN_TEAM_CHANGE_COST)
                .unwrap(),
            "military_industrial_organization_design_team_change_cost"
        );
    }
}
