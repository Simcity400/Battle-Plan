//! CRandomLocList::Resolve (140A9CD70); no RNG state is advanced.

use super::*;
use crate::triggers::{Evaluator, Temps};
use installed_content::scripted_localisation::RandomList;

/// CEventScope's seeded constructor (1405286D0) stores this second hash
/// lane at +0x0C. 1410CDDF0 constructs promoted objects with 424242.
pub(super) fn scope_seed(seed: u32) -> u32 {
    lane(seed, 0x205a_d647, 0xb4f3_c315, 0x387a_9f3a)
}

fn lane(seed: u32, first: u32, add: u32, last: u32) -> u32 {
    let value = seed.wrapping_mul(first);
    let value = (value ^ (value >> 8)).wrapping_add(add);
    let value = ((value << 8) ^ value).wrapping_mul(last);
    value ^ (value >> 8)
}

/// 140A9CF4A..140A9D025: two hash lanes, combined and mixed in u32,
/// then the sign bit cleared. No key, date, player or global seed is read.
fn draw(seed: u32) -> i32 {
    let a = lane(seed, 0x7eb8_da44, 0x10d8_365e, 0x6d22_f31b);
    let b = scope_seed(seed);
    let value = a.wrapping_sub(b.wrapping_mul(0x4ad6_85b3));
    let value = (value ^ (value >> 8)).wrapping_add(0x68e3_1da4);
    let value = ((value << 8) ^ value).wrapping_mul(0x1b56_c4e9);
    let value = (value ^ (value >> 8)).wrapping_mul(0x92d6_8ca2);
    ((value ^ (value >> 8)) & 0x7fff_ffff) as i32
}

/// The one-member fast path reads no weight or seed. Otherwise all weights
/// are evaluated first, their i32 sum wraps, and a nonpositive total returns
/// the shared empty CBoundLocalization. Negative weights are not clamped.
fn pick(
    count: usize,
    mut weight: impl FnMut(usize) -> Result<i32, String>,
    seed: impl FnOnce() -> Result<u32, String>,
) -> Result<Option<usize>, String> {
    if count == 1 {
        return Ok(Some(0));
    }
    let weights = (0..count).map(&mut weight).collect::<Result<Vec<_>, _>>()?;
    let total = weights
        .iter()
        .fold(0_i32, |sum, weight| sum.wrapping_add(*weight));
    if total <= 0 {
        return Ok(None);
    }
    let mut cursor = draw(seed()?) % total;
    for (index, weight) in weights.into_iter().enumerate() {
        cursor = cursor.wrapping_sub(weight);
        if cursor < 0 {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

impl<'a> ScriptedText<'a> {
    pub(super) fn random_key<'r>(
        &self,
        list: &'r RandomList,
        bound: &ExpansionScopes,
        at: usize,
        scopes: crate::triggers::Scopes,
        evaluator: Option<&Evaluator<'_, 'a>>,
        temps: Temps,
    ) -> Result<Option<&'r [u8]>, String> {
        if list.members.len() == 1 {
            return Ok(Some(&list.members[0].key));
        }
        let fallback = if evaluator.is_none()
            && list
                .members
                .iter()
                .any(|member| !member.modifiers.is_empty())
        {
            Some(Evaluator::new(self.world()?, None).with_temps(temps))
        } else {
            None
        };
        let evaluator = evaluator.or(fallback.as_ref());
        let index = pick(
            list.members.len(),
            // An evaluator exists whenever a member has modifiers.
            |i| match evaluator {
                Some(evaluator) => evaluator.localisation_weight(&list.members[i], scopes),
                None => Ok(crate::triggers::scripted_localisation::constant_weight(
                    &list.members[i],
                )),
            },
            || {
                let default = bound.scope(at).random_seed;
                if list.seed.is_empty() {
                    return Ok(default);
                }
                let seeded;
                let evaluator = match evaluator {
                    Some(evaluator) => evaluator,
                    None => {
                        seeded = Evaluator::new(self.world()?, None).with_temps(temps);
                        &seeded
                    }
                };
                Ok(evaluator
                    .localisation_seed(list, scopes)?
                    .unwrap_or(default))
            },
        )?;
        Ok(index.map(|i| list.members[i].key.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_fixed_seed_draw_and_cumulative_pick() {
        // 140A9CF4A..140A9D025: lanes 0x926D254E/0x1EF7E295,
        // final draw 683596854; %60 = 54, minus 10 then 20 then 30.
        assert_eq!(scope_seed(424242), 0xa4fb_a302);
        assert_eq!(draw(1), 683_596_854);
        assert_eq!(pick(3, |i| Ok([10, 20, 30][i]), || Ok(1)), Ok(Some(2)));
    }

    #[test]
    fn native_zero_single_negative_and_wrapping_weights() {
        let never = || Err("seed must not be read".into());
        assert_eq!(pick(0, |_| panic!(), never), Ok(None));
        assert_eq!(pick(1, |_| panic!(), never), Ok(Some(0)));
        assert_eq!(pick(2, |_| Ok(0), never), Ok(None));
        assert_eq!(pick(2, |_| Ok(i32::MAX), never), Ok(None));
        assert_eq!(pick(3, |i| Ok([-3, 0, 7][i]), || Ok(1)), Ok(Some(2)));
    }
}
