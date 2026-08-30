//! Lane TE: the d43 oil transition self-check, over its two pure pieces.
//!
//! The instrument itself cannot be driven offline — it lives inside
//! `compact_market_trade_oil_current_max_usage`, which needs a live engine —
//! but the two decisions it makes are pure functions of what the walk
//! collected, and those are what a wrong verdict would come from:
//!
//! * `Domain43OilSelfCheckSample::roster` decides whether the two arms saw the
//!   same roster, which is the difference between `shape_diverged` and a
//!   comparison at all;
//! * `domain43_oil_selfcheck_compare` decides `equal` vs `diverged`, and it
//!   must be ABLE to answer `diverged` -- a comparator that cannot is a
//!   counter that means nothing.
//!
//! The counters these produce are what
//! `runs/te_d43_piggyback/SELFCHECK.md` §3 says the next capture is read for.
//! Note what §2.3 of that file says about the comparison's REACH: production
//! cannot separate the two sides, because both consume the same integers, so
//! `trio_diverged` is a drift guard rather than an input-sufficiency proof.
//! These tests are about the comparator, not about that reach.

use super::*;

/// An ordinary production need. It cancels out of every comparison below --
/// both sides receive it -- so nothing here depends on its value; it is a
/// constant rather than a literal so that is visible.
const NEED: i32 = 12;

// @leaf-source-authority crate=d43_grammar root=extractor/native/d43_grammar files=8 sha256=1c09d4e7cfb0eb8d9f6f15f471b36197031c998521e6a715747308ea59134748
fn sample() -> Domain43OilSelfCheckSample {
    Domain43OilSelfCheckSample {
        fuel: d43_grammar::oil_trio::FuelScalars {
            base_first: 900_000,
            base_second: 100_000,
            stock: 50_000,
            denominator: 1_000,
        },
        globals: d43_grammar::oil_trio::ScalingGlobals {
            unit_a: 60_000,
            unit_b: 40_000,
            unit_c: 100_000,
            naval_a: 200_000,
            naval_b: 50_000,
        },
        land_current: vec![20_000, 7],
        land_base: vec![500_000, 1],
        naval_current: vec![1_000],
        naval_base: vec![250_000],
        // One task force, and the max arm re-read the same address.  A
        // fabricated address: nothing here dereferences it, and the only thing
        // the walk does with it is compare it against what the max arm re-read.
        naval_addresses: vec![0x1_0000],
        naval_roster_changed: false,
        air_land_current: 3_000,
        air_land_max: 4_000,
    }
}

/// The usage pair the walk would have produced for [`sample`], computed the
/// walk's way rather than the shared arithmetic's, so the "equal" case below is
/// an agreement between two derivations and not a tautology.
fn walk_usage(sample: &Domain43OilSelfCheckSample) -> (i32, i32) {
    let fuel = sample.fuel;
    let mut base = fuel.base_first + fuel.base_second;
    if fuel.stock < 0 {
        base += fuel.stock.abs();
    }
    let current_units: i64 = sample.land_current.iter().sum();
    let current_naval: i64 = sample.naval_current.iter().sum();
    let mut current_residual =
        base - current_units - current_naval - sample.air_land_current;
    if fuel.stock > 0 {
        current_residual -= fuel.stock;
    }
    let unit_factor = sample.globals.unit_a + sample.globals.unit_b;
    let max_units: i64 = sample
        .land_base
        .iter()
        .map(|base_fuel| (unit_factor * base_fuel) / 100_000 * sample.globals.unit_c / 100_000)
        .sum();
    let max_naval: i64 = sample
        .naval_base
        .iter()
        .map(|base_fuel| {
            (base_fuel * sample.globals.naval_a) / 100_000 * sample.globals.naval_b / 100_000
        })
        .sum();
    let max_residual =
        base - fuel.stock.max(0) - max_units - max_naval - sample.air_land_max;
    (
        compact_market_trade_oil_requirement(current_residual, fuel.denominator)
            .expect("current requirement"),
        compact_market_trade_oil_requirement(max_residual, fuel.denominator)
            .expect("max requirement"),
    )
}

#[test]
fn te_selfcheck_counts_a_faithful_sample_as_equal() {
    let sample = sample();
    let (current, maximum) = walk_usage(&sample);
    let mut trace = Domain43OilTrace::default();
    domain43_oil_selfcheck_compare(&sample, NEED, current, maximum, &mut trace);
    assert_eq!(trace.selfcheck_trio_roots, 1);
    assert_eq!(trace.selfcheck_trio_equal, 1);
    assert_eq!(trace.selfcheck_trio_diverged, 0);
    assert_eq!(trace.selfcheck_shape_diverged, 0);
}

#[test]
fn te_selfcheck_is_not_agreement_by_construction() {
    // THE TEST THAT MAKES THE COUNTER MEAN SOMETHING. If `compare` could only
    // ever answer "equal", a capture full of zeros would prove nothing at all
    // and the whole wire change would rest on a tautology. Feeding it a usage
    // pair one off the walk's must land in `diverged`.
    //
    // kills: any rewrite of `domain43_oil_selfcheck_compare` that reports
    // agreement unconditionally, or that compares a value against itself.
    let sample = sample();
    let (current, maximum) = walk_usage(&sample);
    let mut trace = Domain43OilTrace::default();
    domain43_oil_selfcheck_compare(&sample, NEED, current + 1, maximum, &mut trace);
    domain43_oil_selfcheck_compare(&sample, NEED, current, maximum - 1, &mut trace);
    assert_eq!(trace.selfcheck_trio_roots, 2);
    assert_eq!(trace.selfcheck_trio_equal, 0);
    assert_eq!(trace.selfcheck_trio_diverged, 2);
    assert_eq!(trace.selfcheck_shape_diverged, 0);
}

#[test]
fn te_selfcheck_reports_a_roster_that_changed_between_the_two_arms_as_shape_not_arithmetic() {
    // The current arm and the max arm walk `ptr_vector(outer, 0xB8, 0xC4)`
    // separately, which is deliberate engine load-count parity. A task force
    // that appeared or vanished between them gives the two arms different
    // lengths, and that is an ENGINE event: pairing whatever lines up would
    // report it as an arithmetic divergence and send the next lane looking for
    // a bug in the arithmetic.
    //
    // kills: zipping the two arms with `zip`'s truncate-to-shortest behaviour
    // instead of refusing on a length mismatch.
    let mut sample = sample();
    sample.naval_base.push(7);
    let mut trace = Domain43OilTrace::default();
    domain43_oil_selfcheck_compare(&sample, NEED, 0, 0, &mut trace);
    assert_eq!(trace.selfcheck_trio_roots, 1);
    assert_eq!(trace.selfcheck_shape_diverged, 1);
    assert_eq!(trace.selfcheck_trio_equal, 0);
    assert_eq!(trace.selfcheck_trio_diverged, 0);
}

#[test]
fn te_selfcheck_reports_a_same_length_naval_replacement_as_shape_diverged() {
    // THE GAP CODEX ROUND 7 BLOCKING 1 FOUND. The max arm re-reads every
    // fleet's `ptr_vector(outer, 0xB8, 0xC4)`, and until this existed the only
    // cross-arm check was a LENGTH comparison. A task force replaced by a
    // different one between the two arms -- same cardinality, different
    // address -- passed `roster`, was never re-censused for type or duplicate
    // (the census runs on the current arm's copy only), and so produced a
    // silent `equal` on a root where the two arms summed different objects.
    //
    // The walk now records the current arm's addresses and the max arm
    // compares positionally; this pins that the flag routes to `shape_diverged`
    // and NOT to the arithmetic counter, because a roster that moved is an
    // engine event.
    //
    // kills: dropping the `naval_roster_changed` check from
    // `domain43_oil_selfcheck_compare`, or folding it into `trio_diverged`.
    let sample_with_changed_roster = || {
        let mut sample = sample();
        sample.naval_roster_changed = true;
        sample
    };

    // It is reported even when every length agrees and the arithmetic would
    // have said EQUAL -- which is the whole point, since that is exactly the
    // case that used to pass.
    let faithful = sample();
    let (current, maximum) = walk_usage(&faithful);
    let mut trace = Domain43OilTrace::default();
    domain43_oil_selfcheck_compare(&faithful, NEED, current, maximum, &mut trace);
    assert_eq!(trace.selfcheck_trio_equal, 1, "control: the sample agrees");

    let mut trace = Domain43OilTrace::default();
    domain43_oil_selfcheck_compare(
        &sample_with_changed_roster(),
        NEED,
        current,
        maximum,
        &mut trace,
    );
    assert_eq!(trace.selfcheck_trio_roots, 1);
    assert_eq!(trace.selfcheck_shape_diverged, 1);
    assert_eq!(trace.selfcheck_trio_equal, 0);
    assert_eq!(trace.selfcheck_trio_diverged, 0);
}

#[test]
fn te_selfcheck_counts_a_refusing_derivation_as_a_divergence_and_not_as_equal() {
    // A `None` from the shared arithmetic where the walk produced numbers is
    // the case that matters most for the wire change: a replay layer fed those
    // inputs would publish NEEDED_UNAVAILABLE where the recorder published a
    // value. Silence would be the wrong answer.
    //
    // `denominator = 0` is refused by `requirement` on both sides, so the
    // derivation returns `None` while the caller hands over a pair the walk
    // supposedly produced.
    //
    // kills: treating `None` as "nothing to compare" and skipping the root.
    let mut sample = sample();
    sample.fuel.denominator = 0;
    let mut trace = Domain43OilTrace::default();
    domain43_oil_selfcheck_compare(&sample, NEED, 1, 1, &mut trace);
    assert_eq!(trace.selfcheck_trio_diverged, 1);
    assert_eq!(trace.selfcheck_trio_equal, 0);
}

#[test]
fn te_selfcheck_roster_pairs_by_position_and_refuses_a_length_mismatch() {
    // WHAT THE PAIRING IS AND IS NOT FOR. An earlier version of this test said
    // a reordered collection would produce a wrong `max_units` because the max
    // arm's truncation is "not order-invariant". That is false (Codex round 3,
    // blocking 5): `oil_trio::usage` applies `max_contribution` to each member
    // independently and sums, and it sums `current_fuel` independently too, so
    // the VALUE depends on the two multisets and on nothing else -- not on
    // which `current_fuel` is paired with which `base_fuel`.
    //
    // "And nothing else" was still too strong (Codex round 7, blocking 4). The
    // folds are `checked_add`, so WHETHER a value is produced at all depends on
    // the order: `oil_trio`'s
    // `usage_is_order_sensitive_at_the_overflow_boundary` exhibits one multiset
    // in two orders, one refusing. The value is order-free; the availability is
    // not. Non-distribution over a combined total, which
    // `the_max_arm_does_not_distribute_over_a_sum_of_base_fuels` pins, is a
    // third property again.
    //
    // So this test pins `roster`'s CONTRACT rather than an arithmetic
    // consequence: equal lengths pair positionally, unequal lengths refuse.
    // The refusal is the load-bearing half -- it is what routes a roster that
    // changed between the two arms to `shape_diverged` instead of to the
    // arithmetic counter.
    //
    // kills: replacing the length check with `zip`, which truncates silently.
    let members = Domain43OilSelfCheckSample::roster(&[1, 2, 3], &[10, 20, 30])
        .expect("equal lengths");
    assert_eq!(members[0].current_fuel, 1);
    assert_eq!(members[0].base_fuel, 10);
    assert_eq!(members[2].current_fuel, 3);
    assert_eq!(members[2].base_fuel, 30);
    assert_eq!(Domain43OilSelfCheckSample::roster(&[1, 2], &[10]), None);
    assert_eq!(
        Domain43OilSelfCheckSample::roster(&[], &[]),
        Some(Vec::new())
    );
}

#[test]
fn te_the_selfcheck_window_is_bounded_and_the_bound_is_stated() {
    // The window exists so the per-element halves are a TRANSITION cost rather
    // than a permanent one (`runs/te_d43_piggyback/PRICING.md` §5). A zero
    // window would publish nothing; an unbounded one would make the instrument
    // part of the steady state it is measuring.
    assert!(domain43_oil_selfcheck::FRAMES > 0);
    assert!(domain43_oil_selfcheck::FRAMES <= 4_096);
}
