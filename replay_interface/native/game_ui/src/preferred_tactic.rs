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

/// Product-facing state for the officer-corps preferred-tactic control.
///
/// `NoneSelected` is an authenticated D92 measurement, not missing data.
/// `Selected` is published only when the matching D96 fixed-point inputs were
/// admitted by replay_core. Save-only ordinals and partial D92/D96 joins stay
/// `Leaf::Unobtained`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreferredTacticUi {
    NoneSelected,
    Selected(PreferredTacticOddsUi),
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

/// Admit the exact D92 selection and, when selected, its exact D96 multiplier.
///
/// This is the semantic companion for pixels owned by the authenticated
/// selected-Present UI pass. It never authorizes reconstructed pixel output.
pub fn preferred_tactic_ui(identity: &CountryIdentity) -> Leaf<PreferredTacticUi> {
    let selected = match &identity.preferred_tactic {
        Leaf::Absent => return Leaf::Absent,
        Leaf::Unobtained => return Leaf::Unobtained,
        Leaf::Measured(selected) => selected,
    };
    if selected.authority.as_deref() != Some("a729_ccountry_15d0_definition_v1") {
        return Leaf::Unobtained;
    }
    let tactic_id = match (selected.tactic_database_id, selected.tactic_id.as_ref()) {
        (0, None) => return Leaf::Measured(PreferredTacticUi::NoneSelected),
        (database_id, Some(tactic_id)) if database_id > 0 => tactic_id,
        _ => return Leaf::Unobtained,
    };
    let weight = match &selected.preferred_weight {
        Leaf::Absent => return Leaf::Absent,
        Leaf::Unobtained => return Leaf::Unobtained,
        Leaf::Measured(weight) => weight,
    };
    Leaf::Measured(PreferredTacticUi::Selected(PreferredTacticOddsUi {
        tactic_database_id: selected.tactic_database_id,
        tactic_id: tactic_id.clone(),
        accumulator_entry_present: weight.accumulator_entry_present,
        country_factor_raw: weight.country_factor_raw,
        accumulator_raw: weight.accumulator_raw,
        effective_bonus_raw: weight.effective_bonus_raw,
        multiplier_numerator_raw: weight.multiplier_numerator_raw,
    }))
}

/// Backwards-compatible odds-only view. Exact no-selection remains
/// `Unobtained` here because it has no multiplier; callers that must preserve
/// that measured state use [`preferred_tactic_ui`].
pub fn preferred_tactic_odds(identity: &CountryIdentity) -> Leaf<PreferredTacticOddsUi> {
    match preferred_tactic_ui(identity) {
        Leaf::Measured(PreferredTacticUi::Selected(odds)) => Leaf::Measured(odds),
        Leaf::Measured(PreferredTacticUi::NoneSelected) | Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Absent => Leaf::Absent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::tactic_weight::CountryPreferredTacticWeight;
    use replay_core::CountryPreferredTactic;

    fn identity(
        tactic_database_id: i32,
        tactic_id: Option<&str>,
        authority: Option<&str>,
        preferred_weight: Leaf<CountryPreferredTacticWeight>,
    ) -> CountryIdentity {
        CountryIdentity {
            cosmetic_tag: Leaf::Absent,
            ruling_ideology: Leaf::Absent,
            cores: Leaf::Absent,
            claims: Leaf::Absent,
            preferred_tactic: Leaf::Measured(CountryPreferredTactic {
                tactic_database_id,
                tactic_id: tactic_id.map(str::to_string),
                authority: authority.map(str::to_string),
                preferred_weight,
            }),
        }
    }

    #[test]
    fn exact_none_selection_is_measured_not_unavailable() {
        let identity = identity(
            0,
            None,
            Some("a729_ccountry_15d0_definition_v1"),
            Leaf::Unobtained,
        );
        assert_eq!(
            preferred_tactic_ui(&identity),
            Leaf::Measured(PreferredTacticUi::NoneSelected)
        );
    }

    #[test]
    fn selected_tactic_requires_the_d96_join() {
        let identity = identity(
            19,
            Some("tactic_basic_attack"),
            Some("a729_ccountry_15d0_definition_v1"),
            Leaf::Unobtained,
        );
        assert_eq!(preferred_tactic_ui(&identity), Leaf::Unobtained);
    }

    #[test]
    fn selected_tactic_publishes_exact_fixed_point_inputs() {
        let identity = identity(
            19,
            Some("tactic_basic_attack"),
            Some("a729_ccountry_15d0_definition_v1"),
            Leaf::Measured(CountryPreferredTacticWeight {
                source_sequence: 7,
                observed_country_count: 3,
                modifier_key_runtime_provenance: 11,
                accumulator_entry_present: true,
                country_factor_raw: 25_000,
                accumulator_raw: 100_000,
                effective_bonus_raw: 125_000,
                multiplier_numerator_raw: 225_000,
            }),
        );
        let Leaf::Measured(PreferredTacticUi::Selected(model)) =
            preferred_tactic_ui(&identity)
        else {
            panic!("exact D92/D96 join must publish the selected tactic");
        };
        assert_eq!(model.tactic_database_id, 19);
        assert_eq!(model.tactic_id, "tactic_basic_attack");
        assert_eq!(model.multiplier_numerator_raw, 225_000);
        assert_eq!(model.apply_to_base_weight(40_000), Some(90_000));
    }
}
