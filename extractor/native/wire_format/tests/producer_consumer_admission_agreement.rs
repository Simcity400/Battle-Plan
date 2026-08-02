//! The producer half of the cross-workspace admission-agreement gate.
//!
//! # What this pins and why
//!
//! `extractor/native` and `replay_interface/native` are separate cargo
//! workspaces with no dependency on each other, so an admission rule that both
//! sides apply has to be written twice. Twice that has silently cost data: a
//! lane widened a producer-side rule, the rows were then collected correctly,
//! and the replay side re-applied the identical superseded rejection and threw
//! them away. The producer-only fix "worked" and produced nothing.
//!
//! `portable_admission` is the crate BOTH workspaces path-depend on, so it can
//! hold the value once. Some producer-side copies can be pointed straight at it
//! and now are. The copies pinned HERE are the ones that cannot be: they live
//! in `compact_contract`, which is deliberately kept to serde_json + sha2 and
//! whose dependency surface is itself policy
//! (`tests/compact_contract_dependency_policy.rs`). Taking a new path
//! dependency there to share a constant would be a policy change, not an
//! incidental import.
//!
//! So for those, the mechanism is this test rather than the type system: the
//! value stays written twice, and any edit that makes the two spellings
//! disagree fails here by name instead of silently discarding rows at replay.
//!
//! The consumer half is
//! `replay_interface/native/replay_core/src/world.rs`'s
//! `producer_consumer_admission_agreement` module. Both halves must exist:
//! a rule enforced on one side and not its sibling is not enforced.

/// `compact_contract::calendar` is the extractor's canonical calendar. It
/// cannot depend on `portable_admission` (see above), so its agreement with the
/// shared window is asserted instead of compiled in.
#[test]
fn compact_contract_calendar_agrees_with_the_shared_admission_window() {
    assert_eq!(
        compact_contract::calendar::DATE_EPOCH_CONST,
        portable_admission::DATE_EPOCH_CONST,
        "compact_contract::calendar::DATE_EPOCH_CONST drifted from \
         portable_admission::DATE_EPOCH_CONST. The replay consumer converts \
         raw dates with the shared value; a producer that stamps hours with a \
         different epoch has every row rejected as an out-of-window date."
    );

    // Agreement on the endpoints is not enough: the two copies must also agree
    // on whether each endpoint is INSIDE the window. A copy that spells the
    // upper comparison `>=` instead of `>` rejects exactly one value that every
    // other copy admits, and nothing else would notice.
    for probe in [
        portable_admission::RAW_DATE_MIN - 1,
        portable_admission::RAW_DATE_MIN,
        portable_admission::RAW_DATE_MIN + 1,
        portable_admission::RAW_DATE_MAX - 1,
        portable_admission::RAW_DATE_MAX,
        portable_admission::RAW_DATE_MAX + 1,
    ] {
        let producer = compact_contract::calendar::hour_index_from_raw(probe);
        let shared = portable_admission::hour_index_from_raw(probe);
        assert_eq!(
            producer, shared,
            "compact_contract::calendar::hour_index_from_raw and \
             portable_admission::hour_index_from_raw disagree at raw date \
             {probe}. One side admits an hour the other discards."
        );
    }
}

/// `wire_format::d67` carries its own copy of the same window, with source
/// comments citing `compact_contract/src/calendar.rs` as its authority.
#[test]
fn d67_raw_date_window_agrees_with_the_shared_admission_window() {
    assert_eq!(
        wire_format::d67::DATE_EPOCH_CONST,
        portable_admission::DATE_EPOCH_CONST,
        "wire_format::d67::DATE_EPOCH_CONST drifted from the shared epoch"
    );
    assert_eq!(
        wire_format::d67::RAW_DATE_MIN,
        portable_admission::RAW_DATE_MIN,
        "wire_format::d67::RAW_DATE_MIN drifted from the shared window"
    );
    assert_eq!(
        wire_format::d67::RAW_DATE_MAX,
        portable_admission::RAW_DATE_MAX,
        "wire_format::d67::RAW_DATE_MAX drifted from the shared window"
    );
}

/// The owner-identity bound. `compact_contract::build_profile` is the producer
/// authority; `replay_core` re-applies the same ceiling when it reads the rows
/// back, and now takes it from `portable_admission`.
#[test]
fn compact_owner_tag_db_id_bound_agrees_with_the_shared_bound() {
    assert_eq!(
        compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID,
        portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID,
        "compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID drifted \
         from portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID. Raising it here \
         alone means the extra owners are collected and then discarded by the \
         replay consumer, which still applies the shared ceiling."
    );
}

/// The shared crate's OWN invariants, asserted from a suite that runs.
///
/// `portable_replay/native/admission` is a path dependency of both workspaces
/// and a member of neither, so `cargo nextest run --workspace` compiles it and
/// never runs its `#[cfg(test)]` module. Its boundary tests are therefore dead
/// where they stand -- which is a poor place for the one gate that decides
/// whether a row survives the trip from the recorder to replay. These
/// assertions restate the same contract inside the extractor suite.
///
/// The values are pinned ABSOLUTELY. A test that spells a bound as the
/// constant it is testing passes for every value the constant could take,
/// which is precisely the change it exists to catch: the number itself is the
/// cross-workspace contract.
#[test]
fn the_shared_admission_bounds_hold_their_exact_documented_values() {
    assert_eq!(portable_admission::DATE_EPOCH_CONST, 43_799_999);
    assert_eq!(portable_admission::RAW_DATE_MIN, 60_005_999);
    assert_eq!(portable_admission::RAW_DATE_MAX, 63_071_999);
    assert_eq!(portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID, 2_048);
    assert_eq!(portable_admission::SUPPLY_MAX_PROVINCE_ID, 13_413);
    assert_eq!(
        portable_admission::MAX_DELAYED_EVENT_LIFECYCLE_TAG_DB_ID,
        100_000
    );
    assert_eq!(portable_admission::MAX_DELAYED_EVENT_LIFECYCLE_EVENTS, 65_536);
    assert_eq!(portable_admission::MAX_FRD_DIRECT_TRANSITIONS, 1_000_000);
}

/// The calendar window is closed at BOTH ends. A copy that spells the upper
/// comparison `>=` rejects exactly one value every other copy admits, and
/// nothing else would notice.
#[test]
fn the_shared_calendar_window_is_inclusive_at_both_ends() {
    assert_eq!(
        portable_admission::hour_index_from_raw(portable_admission::RAW_DATE_MIN - 1),
        None
    );
    assert!(portable_admission::hour_index_from_raw(portable_admission::RAW_DATE_MIN).is_some());
    assert!(portable_admission::hour_index_from_raw(portable_admission::RAW_DATE_MAX).is_some());
    assert_eq!(
        portable_admission::hour_index_from_raw(portable_admission::RAW_DATE_MAX + 1),
        None
    );
    assert_eq!(portable_admission::hour_index_from_raw(0), None);
    assert_eq!(portable_admission::hour_index_from_raw(i32::MIN), None);
    assert_eq!(portable_admission::hour_index_from_raw(i32::MAX), None);
}
