//! Convert the sealed native census once without changing signed words or order.
use super::{country_roster_readset::Budget, division_combat_membership as native};
use compact_contract::division_routes as wire;

pub(super) fn refusal(value: native::Refusal) -> wire::CombatMembershipRefusal {
    use native::{Cause as C, Field as F, Stage as S};
    use wire::{
        CombatMembershipCause as WC, CombatMembershipField as WF, CombatMembershipStage as WS,
    };
    let cause = match value.cause {
        C::NotObserved => WC::NotObserved,
        C::Unreadable => WC::Unreadable,
        C::Changed => WC::Changed,
        C::InvalidIdentity => WC::InvalidIdentity,
        C::NullSide => WC::NullSide,
        C::Cycle => WC::Cycle,
        C::VerificationBudgetExceeded => WC::VerificationBudgetExceeded,
        C::RetainedBudgetExceeded => WC::RetainedBudgetExceeded,
        C::AllocationFailed => WC::AllocationFailed,
        C::AddressOverflow => WC::AddressOverflow,
        C::InvalidAddress => WC::InvalidAddress,
        C::RootChanged => WC::RootChanged,
        C::HourChanged => WC::HourChanged,
        C::HourUnreadable => WC::HourUnreadable,
    };
    let field = match value.field {
        F::FrameRoot => WF::FrameRoot,
        F::Hour => WF::Hour,
        F::DivisionIdentity => WF::DivisionIdentity,
        F::FirstCombatVector => WF::FirstCombatVector,
        F::FirstCombatPointer => WF::FirstCombatPointer,
        F::CombatIdentity => WF::CombatIdentity,
        F::SidePointer => WF::SidePointer,
        F::SideClass => WF::SideClass,
        F::SideBacklink => WF::SideBacklink,
        F::Head => WF::Head,
        F::Country => WF::Country,
        F::Next => WF::Next,
        F::OrdinaryRoute => WF::OrdinaryRoute,
        F::AlternateRoute => WF::AlternateRoute,
        F::RouteHead => WF::RouteHead,
        F::Numerator => WF::Numerator,
        F::Transfer => WF::Transfer,
        F::Target => WF::Target,
        F::Retreat => WF::Retreat,
        F::DivisionCountryOperand => WF::DivisionCountryOperand,
        F::AttachmentIdentity => WF::AttachmentIdentity,
        F::CampaignIdentity => WF::CampaignIdentity,
        F::ProducerBinding => WF::ProducerBinding,
        F::Strength => WF::Strength,
        F::Organisation => WF::Organisation,
    };
    let stage = match value.stage {
        S::Admission => WS::Admission,
        S::Open => WS::Open,
        S::Close => WS::Close,
    };
    let ordinals = value
        .ordinal
        .map(u32::try_from)
        .transpose()
        .and_then(|ordinal| {
            value
                .original_ordinal
                .map(u32::try_from)
                .transpose()
                .map(|original_ordinal| (ordinal, original_ordinal))
        });
    match ordinals {
        Ok((ordinal, original_ordinal)) => wire::CombatMembershipRefusal {
            cause,
            field,
            stage,
            ordinal,
            original_ordinal,
        },
        Err(_) => wire::CombatMembershipRefusal {
            cause: WC::AddressOverflow,
            field,
            stage: WS::Admission,
            ordinal: None,
            original_ordinal: None,
        },
    }
}

pub(super) fn observation(
    value: native::FirstCombatCountries,
    budget: &Budget,
) -> wire::FirstCombatCountriesObservation {
    use native::FirstCombatCountries as N;
    use wire::{CountryWordsObservation as Words, FirstCombatCountriesObservation as W};
    match value {
        N::NoFirstCombat => W::NoFirstCombat,
        N::Unavailable(reason) => W::Unavailable(refusal(reason)),
        N::Present { combat, countries } => W::Present {
            combat,
            countries: match countries.and_then(|words| words.try_into_raw_words(budget)) {
                Ok(words) => Words::Measured(words),
                Err(reason) => Words::Unavailable(refusal(reason)),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinal_overflow_is_named_without_truncating_to_a_different_node() {
        let result = refusal(native::Refusal {
            cause: native::Cause::Cycle,
            field: native::Field::Next,
            stage: native::Stage::Admission,
            ordinal: Some(u32::MAX as usize + 1),
            original_ordinal: Some(0),
        });
        assert_eq!(result.cause, wire::CombatMembershipCause::AddressOverflow);
        assert_eq!(result.field, wire::CombatMembershipField::Next);
        assert_eq!(result.ordinal, None);
        assert_eq!(result.original_ordinal, None);
    }
    #[test]
    fn side_refusal_retains_sealed_combat_projection() {
        let combat = wire::CombatIdentity {
            class: wire::CombatClass::Land,
            kind: 62,
            id: 79,
        };
        let reason = native::Refusal::at(
            native::Cause::Unreadable,
            native::Field::Next,
            native::Stage::Admission,
            Some(4),
        );
        let value = observation(
            native::FirstCombatCountries::Present {
                combat,
                countries: Err(reason),
            },
            &Budget::default(),
        );
        assert!(
            matches!(value, wire::FirstCombatCountriesObservation::Present { combat: found, countries: wire::CountryWordsObservation::Unavailable(_) } if found == combat)
        );
    }
}
