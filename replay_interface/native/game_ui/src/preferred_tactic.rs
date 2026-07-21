//! Exact replay/UI binding for a country's selected combat-tactic odds input.
//!
//! This module consumes only replay_core's admitted d92/d96 join. It does not
//! infer a tactic weight from authored defines, save ordinals, or UI text.

use replay_core::{CountryIdentity, Leaf};

pub const FIXED_SCALE: i64 = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreferredTacticOddsUi {
    pub tactic_database_id: i32,
    pub tactic_id: String,
    pub accumulator_entry_present: bool,
    pub country_factor_raw: i64,
    pub accumulator_raw: i64,
    pub effective_bonus_raw: i64,
    pub multiplier_numerator_raw: i64,
}

impl PreferredTacticOddsUi {
    /// Apply the engine's selected-tactic integer law without floating-point
    /// conversion. `None` means the caller supplied a value that would exceed
    /// the signed 64-bit arithmetic admitted by the replay layer.
    pub fn apply_to_base_weight(&self, base_weight: i64) -> Option<i64> {
        self.multiplier_numerator_raw
            .checked_mul(base_weight)
            .map(|product| product / FIXED_SCALE)
    }
}

pub fn preferred_tactic_odds(identity: &CountryIdentity) -> Leaf<PreferredTacticOddsUi> {
    let selected = match &identity.preferred_tactic {
        Leaf::Absent => return Leaf::Absent,
        Leaf::Unobtained => return Leaf::Unobtained,
        Leaf::Measured(selected) => selected,
    };
    let Some(tactic_id) = selected.tactic_id.as_ref() else {
        return Leaf::Unobtained;
    };
    let weight = match &selected.preferred_weight {
        Leaf::Absent => return Leaf::Absent,
        Leaf::Unobtained => return Leaf::Unobtained,
        Leaf::Measured(weight) => weight,
    };
    Leaf::Measured(PreferredTacticOddsUi {
        tactic_database_id: selected.tactic_database_id,
        tactic_id: tactic_id.clone(),
        accumulator_entry_present: weight.accumulator_entry_present,
        country_factor_raw: weight.country_factor_raw,
        accumulator_raw: weight.accumulator_raw,
        effective_bonus_raw: weight.effective_bonus_raw,
        multiplier_numerator_raw: weight.multiplier_numerator_raw,
    })
}
