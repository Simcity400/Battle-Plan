
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
