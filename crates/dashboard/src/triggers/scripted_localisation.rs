//! The scriptable weights used by CRandomLocList::Resolve (140A9CD70).

use installed_content::scripted_localisation::{RandomList, RandomMember};

use super::{Scopes, eval::Evaluator, world::fixed_round};
use crate::state_resources::ONE;

/// 141581FF0's final fixed5 multiplication and 1424D8760 rounding.
pub(crate) fn constant_weight(member: &RandomMember) -> i32 {
    fixed_round(ONE.wrapping_mul(member.weight) / ONE) as i32
}

impl Evaluator<'_, '_> {
    /// are never checked. Additions accumulate into the base, factors into
    /// a separate product; a zero factor stops before later operands.
    pub fn localisation_weight(
        &self,
        member: &RandomMember,
        scopes: Scopes,
    ) -> Result<i32, String> {
        if member.modifiers.is_empty() {
            return Ok(constant_weight(member));
        }
        let mut base = member.weight;
        let mut product = ONE;
        for fields in &member.modifiers {
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

    /// An explicit list seed is the low u32 of the raw fixed5 operand
    /// (140A9CF27), with the scope object's identity added (140529EE0).
    pub fn localisation_seed(
        &self,
        list: &RandomList,
        scopes: Scopes,
    ) -> Result<Option<u32>, String> {
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
            super::Scope::State(id) if (id as i32) > 0 => id.wrapping_mul(59),
            super::Scope::State(_) | super::Scope::Province(_) => 0,
            super::Scope::Leader(reference) => {
                match self.world().unit_leader(reference)? {
                    Some(leader) => reference_hash((u64::from(leader.character_id) << 32) | u64::from(leader.character_type)),
                    None => 0,
                }
            }
            super::Scope::Character(id) => {
                if self.world().character_exists(id)? {
                    // The pinned character reference kind is shared with
                    // the authenticated engine reader (1406AB520 registers
                    // characters); only the instance id is content/runtime.
                    reference_hash((u64::from(id) << 32) | u64::from(engine::ccharacter::REFERENCE_TYPE))
                } else {
                    0
                }
            }
            super::Scope::Combat => return Err("random_list seed: the combat scope's native reference identity (140529EE0) is not given".into()),
        };
        Ok(Some((value as u32).wrapping_add(identity)))
    }
}

/// HashReferenceHandle (14220A010): word swap, wrapping u64 multiplication,
/// then high XOR low. UnitLeaders records the character's complete handle.
fn reference_hash(reference: u64) -> u32 {
    let product = reference.rotate_left(32).wrapping_mul(0x4963_1a45);
    (product >> 32) as u32 ^ product as u32
}
