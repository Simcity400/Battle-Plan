use crate::triggers::Failure;

use content_reader_scripted_localisation::{
    scripted_localisation::RandomList, scripted_localisation::RandomMember,
};

use super::{Scopes, eval::Evaluator, world::fixed_round};
use crate::state_resources::ONE;

pub(crate) fn constant_weight(member: &RandomMember) -> i32 {
    fixed_round(ONE.wrapping_mul(member.weight) / ONE) as i32
}

impl Evaluator<'_, '_> {
    pub fn localisation_weight(
        &self,
        member: &RandomMember,
        scopes: Scopes,
    ) -> Result<i32, Failure> {
        if member.modifiers.is_empty() {
            return Ok(constant_weight(member));
        }
        let mut base = member.weight;
        let mut product = ONE;
        for fields in &member.modifiers {
            let fields = &fields.items();
            base = base.wrapping_add(self.mtth_operand("random_list", fields, "add", 0, scopes)?);
            product = product.wrapping_mul(self.mtth_operand(
                "random_list",
                fields,
                "factor",
                ONE,
                scopes,
            )?) / ONE;
            if product == 0 {
                return Ok(0);
            }
        }
        Ok(fixed_round(product.wrapping_mul(base) / ONE) as i32)
    }

    pub fn localisation_seed(
        &self,
        list: &RandomList,
        scopes: Scopes,
    ) -> Result<Option<u32>, Failure> {
        if !list
            .seed
            .iter()
            .any(|field| field.value.scalar().is_some_and(|text| !text.is_empty()))
        {
            return Ok(None);
        }
        let value = self.mtth_operand("random_list", &list.seed, "seed", 0, scopes)?;
        let identity = match scopes.this {
            super::Scope::Country(slot) => slot.wrapping_mul(31),
            super::Scope::CountryLeader { country, .. }
                if self.variable_scope_has_country(country)? =>
            {
                country.wrapping_mul(31)
            }
            super::Scope::State(id) if (id as i32) > 0 => id.wrapping_mul(59),
            super::Scope::State(_) | super::Scope::Province(_) => 0,
            super::Scope::Leader(reference)
            | super::Scope::CountryLeader {
                leader: reference, ..
            } => match self.world().unit_leader(reference)? {
                Some(leader) => reference_hash(
                    (u64::from(leader.character_id) << 32) | u64::from(leader.character_type),
                ),
                None => 0,
            },
            super::Scope::Character(id) => {
                if self.world().character_exists(id)? {
                    reference_hash(
                        (u64::from(id) << 32)
                            | u64::from(engine_native::ccharacter::REFERENCE_TYPE),
                    )
                } else {
                    0
                }
            }
            super::Scope::IndustrialOrganisation(reference) => {
                self.world().industrial_organisation(reference)?;
                ((reference >> 32) as u32).wrapping_mul(82)
            }
            super::Scope::Unit(reference) => {
                self.world().unit(reference)?;
                ((reference >> 32) as u32).wrapping_mul(3)
            }
            super::Scope::Raid(reference) => {
                self.world().raid(reference)?;
                ((reference >> 32) as u32).wrapping_mul(0x79d)
            }
            super::Scope::Project { .. } => 0,
            super::Scope::Combat => {
                return Err(
                    "random_list seed: the combat scope's reference identity is not given".into(),
                );
            }
        };
        Ok(Some((value as u32).wrapping_add(identity)))
    }
}

pub(super) fn reference_hash(reference: u64) -> u32 {
    let product = reference.rotate_left(32).wrapping_mul(0x4963_1a45);
    (product >> 32) as u32 ^ product as u32
}
