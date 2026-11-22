use collector_observers::division_panel_changes as changes;
use collector_production::production::design_changes::{
    self, Availability, Forwarder, Phase, Writer,
};
use collector_ship_models::ship_model_changes as ships;
static RECEIVER: Forwarder = Forwarder {
    call: receive,
    enabled,
};
fn enabled(availability: Availability) {
    changes::variant_availability(
        availability.stamp(),
        design_changes::availability_is_current,
    );
    ships::variant_availability(
        availability.stamp(),
        design_changes::availability_is_current,
    );
}
/// # Safety
/// `receiver` is an opaque identity in every phase: this handler uses it only as a registry key,
/// so any value, zero included, meets the contract.
unsafe fn receive(writer: Writer, receiver: usize, phase: Phase) {
    let every = !matches!(
        writer,
        Writer::RebuildStats
            | Writer::RefreshDerivedState
            | Writer::MoveAssign
            | Writer::Update
            | Writer::Publish
            | Writer::Destroy
            | Writer::Edit
    );
    match (every, phase) {
        (true, Phase::Before) => ships::variants_before(),
        (true, Phase::After) => ships::variants_after(false),
        (true, Phase::Unwound) => ships::variants_after(true),
        (false, Phase::Before) => ships::variant_before(receiver),
        (false, Phase::After) => ships::variant_after(receiver, false),
        (false, Phase::Unwound) => ships::variant_after(receiver, true),
    }
    match writer {
        Writer::Edit | Writer::Roster => return,
        Writer::SourceInvalidation
        | Writer::IdeaInvalidation
        | Writer::ProviderInvalidation
        | Writer::CountryInvalidation
        | Writer::ContentInvalidation => {
            match phase {
                Phase::Before => changes::variant_invalidation_before(),
                Phase::After => changes::variant_invalidation_after(false),
                Phase::Unwound => changes::variant_invalidation_after(true),
            }
            return;
        }
        _ => {}
    }
    match phase {
        Phase::Before => {
            changes::variant_before(receiver);
            if matches!(writer, Writer::MoveAssign | Writer::Destroy) {
                changes::variant_retiring(receiver);
            }
        }
        Phase::After => changes::variant_after(receiver, false),
        Phase::Unwound => changes::variant_after(receiver, true),
    }
}
pub struct Registration;
pub(crate) fn attach() -> Registration {
    design_changes::forwarder(Some(&RECEIVER));
    Registration
}
impl Drop for Registration {
    fn drop(&mut self) {
        design_changes::forwarder(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observe(enabled: bool) {
        changes::variant_availability(u64::from(enabled), |stamp| stamp == 1);
    }
    #[test]
    fn every_owner_writer_revokes_only_the_shared_variant_operand_and_unwind_refuses_it() {
        changes::demand(2, 2, 2);
        changes::presize();
        changes::boundary(true);
        changes::patched(true);
        observe(false);
        let pool = changes::sequence(0x100).unwrap();
        assert!(changes::variant_sequence(0x200).is_none());
        observe(true);
        for writer in [
            Writer::RebuildStats,
            Writer::RefreshDerivedState,
            Writer::MoveAssign,
            Writer::Update,
            Writer::Publish,
            Writer::Destroy,
        ] {
            let old = changes::variant_sequence(0x200).unwrap();
            unsafe {
                receive(writer, 0x200, Phase::Before);
            }
            assert!(changes::variant_sequence(0x200).is_none());
            unsafe {
                receive(writer, 0x200, Phase::After);
            }
            assert_ne!(changes::variant_sequence(0x200), Some(old));
            assert_eq!(changes::sequence(0x100), Some(pool));
        }
        for writer in [Writer::Edit, Writer::Roster] {
            let old = changes::variant_sequence(0x200).unwrap();
            unsafe {
                receive(writer, 0x200, Phase::Before);
                receive(writer, 0x200, Phase::After);
            }
            assert_eq!(changes::variant_sequence(0x200), Some(old));
        }
        let mut retired = Vec::new();
        changes::take_lifetimes(|pointer| retired.push(pointer)).unwrap();
        assert_eq!(retired, [0x200]);
        changes::take_lifetimes(|_| panic!("quiet lifetime was marked")).unwrap();
        let other = 0x300;
        changes::variant_sequence(other).unwrap();
        for writer in [
            Writer::SourceInvalidation,
            Writer::IdeaInvalidation,
            Writer::ProviderInvalidation,
            Writer::CountryInvalidation,
            Writer::ContentInvalidation,
        ] {
            let old = changes::variant_sequence(0x200).unwrap();
            let old_other = changes::variant_sequence(other).unwrap();
            collector_test_support::with_allocations_denied(|| unsafe {
                receive(writer, usize::MAX, Phase::Before);
                assert!(changes::variant_sequence(0x200).is_none());
                assert!(changes::variant_sequence(other).is_none());
                receive(writer, usize::MAX, Phase::After);
            });
            assert_ne!(changes::variant_sequence(0x200), Some(old));
            assert_ne!(changes::variant_sequence(other), Some(old_other));
            assert_eq!(changes::sequence(0x100), Some(pool));
            changes::take_lifetimes(|_| panic!("a source is not a retired variant")).unwrap();
        }
        collector_test_support::with_allocations_denied(|| unsafe {
            receive(Writer::MoveAssign, 0x200, Phase::Before);
            receive(Writer::MoveAssign, other, Phase::Before);
            receive(Writer::MoveAssign, other, Phase::After);
            assert!(changes::variant_sequence(0x200).is_none());
            receive(Writer::MoveAssign, 0x200, Phase::After);
        });
        retired.clear();
        changes::take_lifetimes(|pointer| retired.push(pointer)).unwrap();
        retired.sort_unstable();
        assert_eq!(retired, [0x200, other]);
        let old = changes::variant_sequence(other).unwrap();
        collector_test_support::with_allocations_denied(|| unsafe {
            receive(Writer::Update, other, Phase::Before);
            observe(false);
            assert!(changes::variant_sequence(other).is_none());
            receive(Writer::Update, other, Phase::After);
            assert!(changes::variant_sequence(other).is_none());
            observe(true);
        });
        assert_ne!(changes::variant_sequence(other), Some(old));
        unsafe {
            receive(Writer::ProviderInvalidation, usize::MAX, Phase::Before);
            receive(Writer::ProviderInvalidation, usize::MAX, Phase::Unwound);
        }
        assert!(changes::variant_sequence(0x200).is_none());
        assert!(changes::variant_sequence(other).is_none());
        observe(false);
        changes::patched(false);
    }
}
