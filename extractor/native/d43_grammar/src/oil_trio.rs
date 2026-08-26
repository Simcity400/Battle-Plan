//! The d43 oil needed-trio's arithmetic, in ONE place.
//!
//! # Why this lives in the grammar crate
//!
//! The trio is three integers that determine five published words
//! (`../../runs/sz_d43_trio_derive/ARITHMETIC.md` §1). Today the collector is
//! the only thing that knows how to produce them, and it produces them by
//! walking the engine. Lane TE's plan moves that walk's INPUTS onto traversals
//! the recorder already runs and derives the trio from them, which means the
//! arithmetic acquires a second caller: whatever re-derives the trio from the
//! recorded inputs.
//!
//! Two callers of one arithmetic is exactly the shape that produced this crate
//! (owner ruling 2026-08-15, ONE shared decoder per domain, because the same
//! grammar was transcribed at four sites). So the arithmetic goes here, with no
//! dependencies, and both sides call it:
//!
//! * `collector` calls [`requirement`], [`round_100k`], [`max_contribution`],
//!   [`unit_factor`], [`fuel_base`], [`current_residual`], [`max_residual`] and
//!   [`publish`] from its own walk. What the walk keeps is **four**
//!   `checked_add` accumulation loops — current land, current naval, max land,
//!   max naval — which cannot be shared without materialising the rosters it
//!   deliberately does not materialise, and which have no law beyond
//!   `checked_add`'s own.
//!
//!   An earlier version of this list said "every arithmetic law exists once"
//!   while `unit_a + unit_b` was still spelled at both sites, and said "two"
//!   accumulation loops when there are four (Codex round 8, blocking 1).
//!   [`unit_factor`] closed the first; the count is corrected here and in the
//!   three places that describe what the self-check's drift counter still
//!   guards;
//! * `collector` calls [`derive`] for the transition self-check
//!   (`../../runs/te_d43_piggyback/SELFCHECK.md`), which compares a derivation
//!   over the inputs against the engine-path trio on a live game;
//! * the replay layer will call [`derive`] over the recorded inputs once those
//!   inputs are on the wire.
//!
//! # What is pinned here, and against what
//!
//! Every rule below is `ARITHMETIC.md`, which is itself read out of lane SV's
//! byte-exact decompiles of `hoi4.exe 1.19.2.0.a729` /
//! `870ebf1f...c65ad`: `func_1410e2140.decompile.c` (current),
//! `func_1410e22e0.decompile.c` (max) and `func_140e4f6d0.decompile.c` (need),
//! all in `../../runs/sv_d43_resource_signal/`.
//!
//! # What this module is NOT
//!
//! It is not a reader. It takes integers and returns integers; it never touches
//! process memory, and it has no opinion on where the integers came from. That
//! is what lets the collector call it with words it read this frame and the
//! replay layer call it with words it read off the file, and get the same
//! answer by construction.

/// The five scaling globals the max arm multiplies by
/// (`ARITHMETIC.md` §3, `INPUTS.md` §2.3).
///
/// They are process globals at fixed module-relative addresses, not per-country
/// state. `no-hardcoded-game-content` forbids baking their VALUES in — a mod or
/// a patch moves them — so they arrive as data, exactly like every other input.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScalingGlobals {
    /// `i64[image + 0x3319DC8]`, `OIL_CURRENT_MAX_GLOBAL_UNIT_A_RVA`.
    pub unit_a: i64,
    /// `i64[image + 0x3319B00]`, `..._UNIT_B_RVA`.
    pub unit_b: i64,
    /// `i64[image + 0x33199D8]`, `..._UNIT_C_RVA`.
    pub unit_c: i64,
    /// `i64[image + 0x331BBF0]`, `..._NAVAL_A_RVA`.
    pub naval_a: i64,
    /// `i64[image + 0x331BB68]`, `..._NAVAL_B_RVA`.
    pub naval_b: i64,
}

/// One roster member's two fuel words.
///
/// For a land unit the object is the division WRAPPER and the words are
/// `i64[wrapper + 0x628]` and `i64[u64[wrapper + 0x138] + 0x2C8]`; for the naval
/// arm it is the task-force SUBOBJECT and `+0x500` / the same definition hop
/// (`../../runs/te_d43_piggyback/RECONCILIATION.md` §0-§1).
///
/// The two arms use DIFFERENT globals over the SAME shape, which is why one
/// struct serves both.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RosterMember {
    /// The member's current fuel.
    pub current_fuel: i64,
    /// `BASE_FUEL(object)`, the definition's base fuel.
    pub base_fuel: i64,
}

/// The fuel-status scalars, as four raw words off `CCountryFuelStatus`
/// (`ARITHMETIC.md` §3).
///
/// `base_first`/`base_second` are kept apart rather than pre-added because
/// `base_first` is d10's `gain` and is already on the wire while `base_second`
/// is not (`INPUTS.md` §2.1, §3). A caller that has them from two different
/// sources must not have to pre-combine them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FuelScalars {
    /// `i64[fuel + 0x18]`. d10 publishes this as `gain`.
    pub base_first: i64,
    /// `i64[fuel + 0x20]`. Not on the wire today.
    pub base_second: i64,
    /// `i64[fuel + 0x38]`. d10 publishes this as `cost`.
    pub stock: i64,
    /// `i64[fuel + 0x40]`, the requirement DENOMINATOR. Not on the wire today,
    /// and its absence is what makes `ARITHMETIC.md` §4's rounding regime
    /// unmeasurable from any sealed capture.
    pub denominator: i64,
}

/// Everything one country-hour's trio is a function of.
///
/// The two air terms are the outputs of
/// `compact_market_trade_oil_air_land_{current,max}` — the arm
/// `RECONCILIATION.md` §4 rules out as a piggyback column, so it is an INPUT
/// here rather than something this module derives.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Inputs<'a> {
    /// `compact_market_trade_oil_production_need`'s result: the sum over every
    /// oil entry of `ROUND_100K(i64[entry+0x10] + i64[entry+0x18])`.
    ///
    /// An `i32` because the engine's accumulator is one and the collector's
    /// `checked_add` chain is over `i32` (`ARITHMETIC.md` §2).
    pub need: i32,
    pub fuel: FuelScalars,
    pub globals: ScalingGlobals,
    /// The land roster.
    ///
    /// **ORDER MATTERS, and an earlier version of this comment said it did
    /// not** (Codex round 7, blocking 4). The two arms are plain sums, so the
    /// VALUE depends only on the multiset — but the folds are `checked_add`,
    /// and where an intermediate overflows depends on the order. `[i64::MAX, 1,
    /// -1]` refuses at the second element; `[1, -1, i64::MAX]` is the same
    /// multiset and succeeds. So a roster reordered relative to the walk's
    /// iteration order can change a row from published to refused without
    /// changing any number, and a future d20/d28-derived roster has to preserve
    /// the engine walk's order, not merely its contents.
    /// `usage_is_order_sensitive_at_the_overflow_boundary` pins it.
    pub land: &'a [RosterMember],
    /// The naval roster (TASK FORCES, not ships — `RECONCILIATION.md` §0).
    /// Order matters here for the same reason as [`Inputs::land`].
    pub naval: &'a [RosterMember],
    /// `compact_market_trade_oil_air_land_current`'s result.
    pub air_land_current: i64,
    /// `compact_market_trade_oil_air_land_max`'s result.
    pub air_land_max: i64,
}

/// The three integers the five published words are determined by.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Trio {
    /// `-(need + current_usage)`.
    pub needed_max: i32,
    /// `needed_max - max_usage`.
    pub needed_current: i32,
    /// The SAME integer as [`Trio::needed_current`]. Carried because the wire
    /// carries two words and a reader comparing them must see what the producer
    /// wrote (`ARITHMETIC.md` §1).
    pub needed: i32,
}

/// `ROUND_100K` from `ARITHMETIC.md` §2: `ceil(raw / 100000)` on both signs.
///
/// The engine spells it as truncating division plus a conditional `+1`
/// (`func_140e4f6d0.decompile.c`), and the two agree because `/` truncates
/// TOWARD ZERO: for a positive non-multiple `raw/100000` is the floor and the
/// `+1` lifts it to the ceiling; for a negative non-multiple `raw/100000` is
/// ALREADY the ceiling and the `+0` leaves it there.
///
/// Returns `None` only on the `checked_mul` — that is, never for any `raw` in
/// `i64`, since `|raw/100000| * 100000 <= |raw|`. The `Option` is kept so the
/// shape matches the engine's own overflow-refusing spelling and so a future
/// widening of the divisor cannot silently start wrapping.
#[must_use]
pub fn round_100k(raw: i64) -> Option<i64> {
    let rounded = raw / 100_000;
    if raw == rounded.checked_mul(100_000)? {
        Some(rounded)
    } else {
        rounded.checked_add(i64::from(raw > 0))
    }
}

/// The engine's fixed-point requirement rounding, in the engine's own two steps
/// (`ARITHMETIC.md` §4-§5).
///
/// ```text
/// Q      = trunc(|residual| * 100000 / denominator)
/// result = Q/100000                    when Q is a multiple of 100000
///        = Q/100000 + 1                otherwise
/// ```
///
/// **Not `ceil(|residual| / denominator)`.** The two differ on exactly one
/// input class of VALUE — a fractional part in `(0, 1/100000)`, which forces
/// `denominator > 100000` — and that row is the whole divergence
/// `ARITHMETIC.md` §4 tabulates.
///
/// **They also differ in what they REFUSE, at every denominator.** The
/// `checked_mul` below is a deliberate parting from the engine (it wraps), and
/// the old one-step ceiling had no product to overflow, so for
/// `|residual| > i64::MAX / 100000` this form returns `None` where the old form
/// returned a number — at `denominator = 100000` as readily as anywhere else.
/// The equality claim is therefore **`denominator <= 100000` AND
/// `|residual| * 100000` fits `i64`**, not `denominator <= 100000` alone; an
/// earlier version of this sentence dropped the second half (Codex round 3,
/// blocking 3).
///
/// Four deliberate partings from the engine, all listed in `ARITHMETIC.md` §5:
///
/// * `denominator <= 0` refuses. The engine refuses only `== 0` and falls
///   through to return `42950` on it; publishing a fabricated `42950` as
///   observed state is worse than declaring the row unobserved
///   (`partial-data-still-seals`).
/// * `checked_mul` refuses where the engine's `m * 100000` would overflow,
///   rather than mirroring a wrap.
/// * the `+1` is unconditional rather than the engine's `+ (0 < Q)`, because
///   `Q >= 0` always here: `magnitude >= 0` and `denominator > 0`. The engine's
///   condition covers its own `D < 0` path, which this side refuses.
/// * `i32::try_from` refuses a result that does not fit; the engine truncates.
///   That one predates lane SZ and is unchanged.
#[must_use]
pub fn requirement(residual: i64, denominator: i64) -> Option<i32> {
    if denominator <= 0 {
        return None;
    }
    let magnitude = residual.checked_abs()?;
    let scaled = magnitude.checked_mul(100_000)?;
    let quotient = scaled / denominator;
    let step = quotient / 100_000;
    let rounded = if quotient == step.checked_mul(100_000)? {
        step
    } else {
        step.checked_add(1)?
    };
    i32::try_from(rounded).ok()
}

/// One roster member's max-arm contribution (`ARITHMETIC.md` §3):
/// `((first * BASE_FUEL) / 100000 * second) / 100000`.
///
/// **One function for both arms, and the reason is commutation, not
/// convenience.** The two arms are spelled with their operands the other way
/// round —
///
/// ```text
/// land : ((G_UNIT_A + G_UNIT_B) * BASE_FUEL) / 100000 * G_UNIT_C  / 100000
/// naval: (BASE_FUEL * G_NAVAL_A)             / 100000 * G_NAVAL_B / 100000
/// ```
///
/// — and `i64` multiplication is commutative, `checked_mul` included (`a*b`
/// overflows exactly when `b*a` does). So the two are the same function of
/// `(first, second, base_fuel)`, with `first = unit_a + unit_b, second = unit_c`
/// on the land arm and `first = naval_a, second = naval_b` on the naval one.
/// Nothing about the truncating divisions is affected: they sit AFTER each
/// product, in the same place on both arms.
///
/// **The two truncating divisions do NOT distribute over a sum**, which is why
/// no caller may hand this function a total of base fuels — the per-member
/// truncation has to happen per member. That is also why the wire has to carry
/// a per-object column rather than one number per country
/// (`../../runs/te_d43_piggyback/PRICING.md` §3), and
/// `the_max_arm_does_not_distribute_over_a_sum_of_base_fuels` is the witness.
#[must_use]
pub fn max_contribution(first: i64, second: i64, base_fuel: i64) -> Option<i64> {
    let scaled = first.checked_mul(base_fuel)? / 100_000;
    Some(scaled.checked_mul(second)? / 100_000)
}

/// The max arm's land scaling factor: `unit_a + unit_b`, checked.
///
/// One line, and it is here for the same reason the rest is: it was the LAST
/// arithmetic expression the collector walk and [`usage`] each spelled for
/// themselves (Codex round 8, blocking 1). A one-line law is still a law, and
/// two spellings of it can still part company — `checked_add` in one and a
/// plain `+` or a `saturating_add` in the other, or one of the two picking up
/// the wrong global after an edit to [`ScalingGlobals`].
///
/// What CANNOT part them is operand order: `checked_add` on two operands is
/// commutative in its overflow behaviour as well as its value, and an earlier
/// version of this comment offered a swap as an example (Codex round 9,
/// blocking 8). The order sensitivity that does exist is in the roster FOLDS,
/// where there are more than two operands — see [`Inputs::land`].
///
/// It applies only to the LAND arm. The naval arm multiplies by `naval_a` and
/// `naval_b` separately (`ARITHMETIC.md` §3) and has no equivalent sum.
#[must_use]
pub fn unit_factor(unit_a: i64, unit_b: i64) -> Option<i64> {
    unit_a.checked_add(unit_b)
}

/// The fuel base pair, with the negative-stock fold (`ARITHMETIC.md` §3):
/// `base_first + base_second`, plus `|stock|` when the stock is negative.
///
/// The three words rather than a [`FuelScalars`] because the walk folds them
/// BEFORE it reads the denominator, and passing a struct would either force the
/// denominator read earlier or invite a placeholder. The order of engine reads
/// is not something a refactor may quietly change.
#[must_use]
pub fn fuel_base(base_first: i64, base_second: i64, stock: i64) -> Option<i64> {
    let base = base_first.checked_add(base_second)?;
    if stock < 0 {
        return base.checked_add(stock.checked_abs()?);
    }
    Some(base)
}

/// The CURRENT arm's residual (`ARITHMETIC.md` §3):
/// `base - land - naval - air_land`, less the stock when the stock is positive.
///
/// The subtraction ORDER is the engine walk's and is preserved: `checked_sub`
/// can refuse at a different step under a different order, and where a step
/// refuses is what the stage code names.
#[must_use]
pub fn current_residual(
    base: i64,
    land_sum: i64,
    naval_sum: i64,
    air_land: i64,
    stock: i64,
) -> Option<i64> {
    let residual = base
        .checked_sub(land_sum)?
        .checked_sub(naval_sum)?
        .checked_sub(air_land)?;
    if stock > 0 {
        return residual.checked_sub(stock);
    }
    Some(residual)
}

/// The MAX arm's residual (`ARITHMETIC.md` §3):
/// `base - max(stock, 0) - land - naval - air_land`.
///
/// A different order from [`current_residual`], and deliberately so: it is the
/// order the engine's max arm subtracts in, and the two arms do not agree on
/// it.
#[must_use]
pub fn max_residual(
    base: i64,
    land_sum: i64,
    naval_sum: i64,
    air_land: i64,
    stock: i64,
) -> Option<i64> {
    base.checked_sub(if stock > 0 { stock } else { 0 })?
        .checked_sub(land_sum)?
        .checked_sub(naval_sum)?
        .checked_sub(air_land)
}

/// `current_usage` and `max_usage`, the two integers
/// `compact_market_trade_oil_current_max_usage` returns
/// (`ARITHMETIC.md` §3).
///
/// `None` is the collector's `?`: any step that refuses makes the row
/// `NEEDED_UNAVAILABLE` rather than publishing a number.
#[must_use]
pub fn usage(inputs: &Inputs<'_>) -> Option<(i32, i32)> {
    let fuel = inputs.fuel;
    let base = fuel_base(fuel.base_first, fuel.base_second, fuel.stock)?;

    let mut current_units = 0i64;
    for member in inputs.land {
        current_units = current_units.checked_add(member.current_fuel)?;
    }
    let mut current_naval = 0i64;
    for member in inputs.naval {
        current_naval = current_naval.checked_add(member.current_fuel)?;
    }

    let current_residual = current_residual(
        base,
        current_units,
        current_naval,
        inputs.air_land_current,
        fuel.stock,
    )?;
    let current = requirement(current_residual, fuel.denominator)?;

    let globals = inputs.globals;
    let factor = unit_factor(globals.unit_a, globals.unit_b)?;
    let mut max_units = 0i64;
    for member in inputs.land {
        max_units =
            max_units.checked_add(max_contribution(factor, globals.unit_c, member.base_fuel)?)?;
    }
    let mut max_naval = 0i64;
    for member in inputs.naval {
        max_naval = max_naval.checked_add(max_contribution(
            globals.naval_a,
            globals.naval_b,
            member.base_fuel,
        )?)?;
    }

    let max_residual = max_residual(
        base,
        max_units,
        max_naval,
        inputs.air_land_max,
        fuel.stock,
    )?;
    let maximum = requirement(max_residual, fuel.denominator)?;

    Some((current, maximum))
}

/// The PUBLICATION step: the three integers turned into the trio
/// (`ARITHMETIC.md` §1).
///
/// ```text
/// needed_max     = -(need + current_usage)
/// needed_current = needed_max - max_usage
/// needed         = needed_current
/// ```
///
/// Separate from [`derive`] because it has two callers that reach it by
/// different routes: the collector's row builder, which has the three integers
/// from its own walk, and [`derive`], which has them from [`usage`]. One
/// spelling for both is the point — a second transcription of two lines is
/// still a second transcription.
///
/// `None` is the arithmetic REFUSING, and **for THIS function the shipping
/// caller does not publish `NEEDED_UNAVAILABLE`**: `attach_log.rs`'s row
/// builder reaches `publish` only after the usage walk succeeded, and its `?`
/// propagates out of d43's per-country closure and truncates that country's
/// WHOLE row set. `runs/te_d43_piggyback/SELFCHECK.md` §6 sets that defect out
/// and hands it on.
///
/// This is NOT the same as a refusal earlier in the derivation. A refusal
/// inside [`usage`] leaves the row builder's `oil_inputs` at `None`, and that
/// arm DOES publish `NEEDED_UNAVAILABLE` with a zeroed trio, which is the
/// intended behaviour working. Round 7's correction of this comment flattened
/// the two into one (Codex round 8, blocking 6); the difference is which step
/// refused, and only the publication step truncates.
///
/// The refusal is reachable with two perfectly ordinary usages:
/// `ARITHMETIC.md` §5's counterexample is `need = i32::MIN` and a current usage
/// of `0`, where `checked_neg` refuses.
#[must_use]
pub fn publish(need: i32, current_usage: i32, max_usage: i32) -> Option<Trio> {
    let needed_max = need.checked_add(current_usage)?.checked_neg()?;
    let needed_current = needed_max.checked_sub(max_usage)?;
    Some(Trio {
        needed_max,
        needed_current,
        needed: needed_current,
    })
}

/// The whole trio, from the inputs (`ARITHMETIC.md` §1).
///
/// `None` is the arithmetic refusing. Three distinct things produce it, the
/// caller cannot tell them apart from the return value alone, and **the
/// shipping walk's equivalents of the three do not end the same way** — the
/// first two publish `NEEDED_UNAVAILABLE`, the third truncates the country's
/// row set (Codex round 8, blocking 6; see [`publish`]). That the return value
/// hides the difference is deliberate, because the wire hides it too; that the
/// shipping walk acts differently on the third is the defect
/// `runs/te_d43_piggyback/SELFCHECK.md` §6 hands on.
///
/// 1. a roster or fuel word that overflows a `checked_*` inside [`usage`];
/// 2. `denominator <= 0`, or a requirement that does not fit `i32`;
/// 3. the publication arithmetic itself refusing — see [`publish`].
#[must_use]
pub fn derive(inputs: &Inputs<'_>) -> Option<Trio> {
    let (current_usage, max_usage) = usage(inputs)?;
    publish(inputs.need, current_usage, max_usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    // ---- round_100k -------------------------------------------------------
    //
    // kills: replacing `ROUND_100K` with a plain truncating division, with a
    // floor, or with a sign-blind `+1`. Each of the three is a distinct row
    // below and each fails a different case.

    #[test]
    fn round_100k_is_ceiling_on_both_signs() {
        assert_eq!(round_100k(0), Some(0));
        assert_eq!(round_100k(100_000), Some(1));
        assert_eq!(round_100k(-100_000), Some(-1));
        // Positive non-multiple rounds UP (a plain `/` would answer 1).
        assert_eq!(round_100k(100_001), Some(2));
        assert_eq!(round_100k(1), Some(1));
        // Negative non-multiple rounds TOWARD ZERO, which is already the
        // ceiling (a floor would answer -2, a sign-blind `+1` would answer 0).
        assert_eq!(round_100k(-100_001), Some(-1));
        assert_eq!(round_100k(-1), Some(0));
    }

    #[test]
    fn round_100k_never_refuses_inside_i64() {
        for raw in [i64::MIN, i64::MIN + 1, -100_001, -1, 0, 1, 100_001, i64::MAX] {
            assert!(round_100k(raw).is_some(), "round_100k({raw}) refused");
        }
    }

    // ---- requirement ------------------------------------------------------

    /// `ceil(|residual| / denominator)`: the form the collector used BEFORE
    /// lane SZ, kept here only as the thing the new form is contrasted against.
    fn old_ceiling_form(residual: i64, denominator: i64) -> Option<i32> {
        if denominator <= 0 {
            return None;
        }
        let magnitude = residual.checked_abs()?;
        let quotient = magnitude / denominator;
        let rounded = quotient.checked_add(i64::from(magnitude % denominator != 0))?;
        i32::try_from(rounded).ok()
    }

    #[test]
    fn requirement_refuses_a_non_positive_denominator() {
        // kills: relaxing the guard to the engine's `== 0`, which would let the
        // engine's 42950 fallback -- or its D<0 zero -- be published as state.
        assert_eq!(requirement(1, 0), None);
        assert_eq!(requirement(1, -1), None);
        assert_eq!(requirement(0, i64::MIN), None);
    }

    #[test]
    fn requirement_refuses_where_the_engine_would_overflow() {
        // kills: swapping `checked_mul` for `*` or `wrapping_mul`. The engine
        // wraps here; publishing a wrapped product as observed state is the
        // parting `ARITHMETIC.md` §5 declares.
        let m = i64::MAX / 100_000 + 1;
        assert_eq!(requirement(m, 1), None);
        // One below the boundary still computes.
        assert!(requirement(i64::MAX / 100_000, i64::MAX).is_some());
    }

    #[test]
    fn requirement_refuses_a_result_that_does_not_fit_i32() {
        // `m * 100000 / 1 / 100000 == m`, so any m past i32::MAX refuses.
        let m = i64::from(i32::MAX) + 1;
        assert_eq!(requirement(m, 1), None);
        assert_eq!(requirement(i64::from(i32::MAX), 1), Some(i32::MAX));
    }

    #[test]
    fn requirement_is_symmetric_in_the_residual_sign() {
        // kills: dropping the `checked_abs`. The engine takes |residual| first
        for residual in [1i64, 7, 99_999, 100_000, 100_001, 1_000_000] {
            for denominator in [1i64, 3, 99_999, 100_000, 100_001, 7_000_003] {
                assert_eq!(
                    requirement(residual, denominator),
                    requirement(-residual, denominator),
                    "residual {residual} denominator {denominator}"
                );
            }
        }
    }

    #[test]
    fn requirement_equals_the_old_ceiling_form_below_the_bound_where_both_are_defined() {
        // THE BOUND, exhaustively over a dense grid rather than by argument.
        // `ARITHMETIC.md` §4 proves `D > 100000` is NECESSARY for a VALUE to
        // differ; this is the empirical half of that claim.
        //
        // "Where both are defined" is load-bearing and not hedging. The two
        // forms also differ in what they REFUSE: this one multiplies by 100000
        // first and `checked_mul` refuses past `i64::MAX / 100000`, where the
        // old one-step ceiling had no product to overflow and answered. Every
        // residual on this grid is far below that, so the grid tests the VALUE
        // claim and says nothing about the refusal one --
        // `requirement_refuses_where_the_engine_would_overflow` is that, and
        // `requirement_and_the_old_form_part_on_refusal_at_any_denominator`
        // below pins the two apart at the bound itself.
        //
        // kills: any rewrite of the two-step form that changes an answer in the
        // regime the two forms are supposed to agree on.
        let mut compared = 0u32;
        for denominator in 1i64..=300 {
            for residual in -1_500i64..=1_500 {
                assert_eq!(
                    requirement(residual, denominator),
                    old_ceiling_form(residual, denominator),
                    "residual {residual} denominator {denominator}"
                );
                compared += 1;
            }
        }
        // The four denominators nearest the bound, over a residual range wide
        // enough to sweep EVERY remainder class of the largest of them at least
        // twice. `100_000` is the exact bound and is the row that matters.
        for denominator in [50_000i64, 99_998, 99_999, 100_000] {
            for residual in -200_000i64..=200_000 {
                assert_eq!(
                    requirement(residual, denominator),
                    old_ceiling_form(residual, denominator),
                    "residual {residual} denominator {denominator}"
                );
                compared += 1;
            }
        }
        assert!(compared > 1_500_000, "grid shrank to {compared} comparisons");
    }

    #[test]
    fn requirement_and_the_old_form_part_on_refusal_at_any_denominator() {
        // Codex round 3 blocking 3's counterexample, kept as a test so the
        // qualified equality cannot quietly become the unqualified one again.
        // At the bound itself -- `denominator == 100000` -- a residual whose
        // scaled magnitude overflows makes the new form refuse and the old form
        // answer, and the answer fits `i32`, so it is not the narrowing either.
        //
        // kills: restoring the sentence "the two forms are equal on every input
        // when denominator <= 100000".
        let residual = i64::MAX / 100_000 + 1;
        let denominator = 100_000i64;
        assert_eq!(requirement(residual, denominator), None);
        assert_eq!(old_ceiling_form(residual, denominator), Some(922_337_204));
    }

    #[test]
    fn requirement_diverges_from_the_old_form_only_on_the_middle_row() {
        // `ARITHMETIC.md` §4's table, checked directly: with
        // `m/D = k + f`, the two forms differ exactly when `0 < f < 1/100000`.
        //
        // kills: reverting to the one-step ceiling. Each divergence below is a
        // value the collector used to publish one HIGHER than the engine's.
        let mut divergences = 0u32;
        for denominator in [100_001i64, 250_000, 1_000_000, 99_999_999] {
            for residual in 0i64..=400_000 {
                let new = requirement(residual, denominator);
                let old = old_ceiling_form(residual, denominator);
                if new == old {
                    continue;
                }
                divergences += 1;
                let m = residual;
                let k = m / denominator;
                let remainder = m % denominator;
                // The divergence condition, stated in integers: a non-zero
                // remainder whose fixed-point promotion is still below one.
                assert!(remainder > 0, "divergence with a zero remainder");
                assert!(
                    remainder.checked_mul(100_000).unwrap() < denominator,
                    "divergence outside 0 < f < 1/100000"
                );
                assert_eq!(new, i32::try_from(k).ok(), "engine form is floor here");
                assert_eq!(
                    old,
                    i32::try_from(k + 1).ok(),
                    "old form was one higher here"
                );
            }
        }
        assert!(
            divergences > 0,
            "the middle row never fired; the contrast is vacuous"
        );
    }

    #[test]
    fn requirement_matches_the_engines_two_step_spelling_directly() {
        // The engine's own text (`func_1410e2140.decompile.c:51`-`:54`),
        // transcribed independently of the implementation, over a grid that
        // straddles the 100000 boundary in both directions.
        //
        // kills: any reordering of the multiply/divide steps -- e.g. dividing
        // before scaling -- which would still pass the D <= 100000 test above.
        fn engine_form(residual: i64, denominator: i64) -> i64 {
            let m = residual.abs();
            let mut q = (m * 100_000) / denominator;
            if q != (q / 100_000) * 100_000 {
                q = (q / 100_000 + i64::from(0 < q)) * 100_000;
            }
            q / 100_000
        }
        for denominator in [1i64, 2, 99_999, 100_000, 100_001, 123_457, 9_999_991] {
            for residual in [-1_000_000i64, -7, -1, 0, 1, 7, 99_999, 100_000, 100_001, 1_000_000]
            {
                let expected = engine_form(residual, denominator);
                assert_eq!(
                    requirement(residual, denominator),
                    i32::try_from(expected).ok(),
                    "residual {residual} denominator {denominator}"
                );
            }
        }
    }

    // ---- the max-arm contributions ---------------------------------------

    #[test]
    fn the_two_arms_are_one_function_because_the_first_product_commutes() {
        // The justification for `max_contribution` serving both arms, checked
        // rather than asserted: the collector writes `unit_factor * base_fuel`
        // on the land arm and `base_fuel * naval_a` on the naval one, and the
        // rest of the expression is identical.
        //
        // kills: a rewrite that moves a division INSIDE the first product, or
        // that changes the operand order in a way that stops being commutation
        // -- e.g. `(first / 100000) * base_fuel`, which passes no case here.
        for base_fuel in [0i64, 1, 3, 7, 99_999, 100_000, 100_001, 12_345_678] {
            for first in [1i64, 2, 50_000, 100_000, 300_001] {
                for second in [1i64, 3, 100_000, 999_983] {
                    let as_written_by_the_land_arm =
                        (first.checked_mul(base_fuel).unwrap() / 100_000)
                            .checked_mul(second)
                            .unwrap()
                            / 100_000;
                    let as_written_by_the_naval_arm =
                        (base_fuel.checked_mul(first).unwrap() / 100_000)
                            .checked_mul(second)
                            .unwrap()
                            / 100_000;
                    assert_eq!(as_written_by_the_land_arm, as_written_by_the_naval_arm);
                    assert_eq!(
                        max_contribution(first, second, base_fuel),
                        Some(as_written_by_the_land_arm)
                    );
                }
            }
        }
    }

    #[test]
    fn the_two_residual_forms_are_not_interchangeable() {
        // `max_residual`'s doc says the two arms subtract in different orders
        // and that the difference is observable. This is the witness: with
        // `checked_sub`, an intermediate can overflow under one order and not
        // under the other, so swapping them would change a published value into
        // `NEEDED_UNAVAILABLE`.
        //
        // kills: collapsing the two into one helper, or "tidying" either one's
        // subtraction order.
        let base = i64::MAX;
        let land = -1i64;
        let stock = 1i64;
        assert_eq!(current_residual(base, land, 0, 0, stock), None);
        assert_eq!(max_residual(base, land, 0, 0, stock), Some(i64::MAX));
    }

    #[test]
    fn fuel_base_folds_a_negative_stock_and_leaves_a_positive_one() {
        // `ARITHMETIC.md` §3's `if stock < 0 { base += |stock| }`, isolated.
        // The positive and zero cases must NOT touch the base -- the positive
        // stock is subtracted from the RESIDUAL instead, once, by whichever
        // arm is running.
        //
        // kills: folding a positive stock in here as well, which would subtract
        // it twice.
        assert_eq!(fuel_base(10, 5, -3), Some(18));
        assert_eq!(fuel_base(10, 5, 0), Some(15));
        assert_eq!(fuel_base(10, 5, 7), Some(15));
        // `i64::MIN` has no positive counterpart, so the fold refuses rather
        // than wrapping to itself.
        assert_eq!(fuel_base(0, 0, i64::MIN), None);
        assert_eq!(fuel_base(i64::MAX, 1, 0), None);
    }

    #[test]
    fn the_max_arm_does_not_distribute_over_a_sum_of_base_fuels() {
        // THE REASON THE WIRE HAS TO CARRY A PER-OBJECT COLUMN. If the sum of
        // per-member contributions equalled the contribution of the summed base
        // fuels, the recorder could publish one number per country and the
        // whole `PRICING.md` §3 size question would evaporate. It does not.
        //
        // `first = 50000, second = 100000` makes the contribution `base/2`
        // truncated: three members of 1 contribute 0 each, one member of 3
        // contributes 1.
        //
        // kills: any future "optimisation" that sums base fuels first.
        let first = 50_000i64;
        let second = 100_000i64;
        let members = [1i64, 1, 1];
        let per_member: i64 = members
            .iter()
            .map(|base| max_contribution(first, second, *base).unwrap())
            .sum();
        let summed = max_contribution(first, second, members.iter().sum()).unwrap();
        assert_eq!(per_member, 0);
        assert_eq!(summed, 1);
        assert_ne!(per_member, summed);
    }

    // ---- derive -----------------------------------------------------------

    fn ordinary_inputs<'a>(land: &'a [RosterMember], naval: &'a [RosterMember]) -> Inputs<'a> {
        Inputs {
            need: 12,
            fuel: FuelScalars {
                base_first: 900_000,
                base_second: 100_000,
                stock: 50_000,
                denominator: 1_000,
            },
            globals: ScalingGlobals {
                unit_a: 60_000,
                unit_b: 40_000,
                unit_c: 100_000,
                naval_a: 200_000,
                naval_b: 50_000,
            },
            land,
            naval,
            air_land_current: 3_000,
            air_land_max: 4_000,
        }
    }

    #[test]
    fn derive_reproduces_the_published_relationships() {
        // `ARITHMETIC.md` §1: needed_max = -(need + current_usage),
        // needed_current = needed_max - max_usage, needed == needed_current.
        let land = vec![
            RosterMember { current_fuel: 20_000, base_fuel: 500_000 },
            RosterMember { current_fuel: 7, base_fuel: 1 },
        ];
        let naval = vec![RosterMember { current_fuel: 1_000, base_fuel: 250_000 }];
        let inputs = ordinary_inputs(&land, &naval);
        let (current_usage, max_usage) = usage(&inputs).expect("usage");
        let trio = derive(&inputs).expect("trio");
        assert_eq!(trio.needed_max, -(inputs.need + current_usage));
        assert_eq!(trio.needed_current, trio.needed_max - max_usage);
        assert_eq!(trio.needed, trio.needed_current);
    }

    #[test]
    fn a_negative_stock_is_added_to_base_and_not_subtracted_from_the_residual() {
        // `ARITHMETIC.md` §3: `if stock < 0 { base += |stock| }`, and the
        // `stock > 0` subtraction then does NOT fire. The two branches are
        // mutually exclusive and a sign error would be invisible at stock == 0.
        //
        // kills: subtracting a negative stock, or applying both branches.
        let land: Vec<RosterMember> = Vec::new();
        let naval: Vec<RosterMember> = Vec::new();
        let mut positive = ordinary_inputs(&land, &naval);
        positive.fuel.stock = 100_000;
        let mut negative = ordinary_inputs(&land, &naval);
        negative.fuel.stock = -100_000;

        // base = 900_000 + 100_000 = 1_000_000, empty rosters, air term 3_000.
        // Positive stock: 1_000_000 - 3_000 - 100_000 = 897_000.
        // Negative stock: base becomes 1_100_000 and NOTHING is subtracted for
        // the stock: 1_100_000 - 3_000 = 1_097_000.
        let (positive_current, _) = usage(&positive).expect("positive");
        let (negative_current, _) = usage(&negative).expect("negative");
        assert_eq!(positive_current, requirement(897_000, 1_000).unwrap());
        assert_eq!(negative_current, requirement(1_097_000, 1_000).unwrap());
        assert_ne!(positive_current, negative_current);
    }

    #[test]
    fn an_empty_roster_is_a_zero_contribution_and_not_a_refusal() {
        // `partial-data-still-seals`: a country with no divisions and no task
        // forces has an ANSWER, not an absence.
        let empty: Vec<RosterMember> = Vec::new();
        let inputs = ordinary_inputs(&empty, &empty);
        assert!(derive(&inputs).is_some());
    }

    #[test]
    fn derive_and_publish_agree_on_the_two_published_lines() {
        // WHAT THIS DOES NOT DO. An earlier version was named
        // `publish_is_the_only_spelling_of_the_two_published_lines` and claimed
        // to kill "re-inlining the two lines inside `derive`". It does not, and
        // no value test can: an exact transcription of `publish`'s two lines
        // inside `derive` produces the same values and passes every assertion
        // below (Codex round 7, blocking 3). That `derive` CALLS `publish` is a
        // property of the source, guaranteed by reading it, not by this test.
        //
        // kills: `derive` publishing from a DIFFERENT rule than `publish` --
        // swapping `needed_current` and `needed_max`, dropping the negation, or
        // subtracting `max_usage` on the wrong line. Each changes a value here
        // while leaving `publish`'s own tests green.
        let empty: Vec<RosterMember> = Vec::new();
        let mut inputs = ordinary_inputs(&empty, &empty);
        for need in [-1_000i32, -1, 0, 1, 1_000] {
            inputs.need = need;
            let (current_usage, max_usage) = usage(&inputs).expect("usage");
            assert_eq!(derive(&inputs), publish(need, current_usage, max_usage));
        }
    }

    #[test]
    fn usage_is_order_sensitive_at_the_overflow_boundary() {
        // The VALUE of each arm depends only on the multiset -- both arms are
        // plain sums -- but the folds are `checked_add`, so WHETHER the arm
        // produces a value at all depends on the order (Codex round 7,
        // blocking 4). Two rosters with identical contents, one refusing.
        //
        // kills: restoring `Inputs::land`'s withdrawn "in any order" contract,
        // and any future roster source that reorders relative to the engine
        // walk on the grounds that a sum is commutative.
        let base = |current: i64| RosterMember {
            current_fuel: current,
            // Zero base fuel keeps the MAX arm out of it: this test is about
            // the current arm's fold and nothing else.
            base_fuel: 0,
        };
        // The multiset sums to 1, small enough that `requirement`'s own
        // `* 100_000` does not overflow -- so the ONLY thing separating the two
        // orders below is where the fold's intermediate lands.
        let overflowing = [base(i64::MAX), base(1), base(-i64::MAX)];
        let ordered = [base(1), base(-i64::MAX), base(i64::MAX)];

        let empty: Vec<RosterMember> = Vec::new();
        let mut inputs = ordinary_inputs(&empty, &empty);
        // A fuel base of zero and a denominator of one, so the only thing that
        // can refuse is the roster fold itself.
        inputs.fuel = FuelScalars {
            base_first: 0,
            base_second: 0,
            stock: 0,
            denominator: 1,
        };
        inputs.air_land_current = 0;
        inputs.air_land_max = 0;

        inputs.land = &overflowing;
        assert_eq!(usage(&inputs), None, "0 + MAX + 1 must overflow the fold");

        inputs.land = &ordered;
        assert!(
            usage(&inputs).is_some(),
            "the same multiset in the engine's order must not"
        );

        // And the naval arm has the same fold, so it has the same property.
        inputs.land = &empty;
        inputs.naval = &overflowing;
        assert_eq!(usage(&inputs), None);
        inputs.naval = &ordered;
        assert!(usage(&inputs).is_some());
    }

    #[test]
    fn publish_matches_the_engine_arms_two_expressions() {
        // `ARITHMETIC.md` §1, transcribed independently of the implementation.
        for (need, current, maximum) in [
            (0i32, 0i32, 0i32),
            (12, 3, 5),
            (-7, 11, -2),
            (1_000_000, -999, 4),
        ] {
            let expected_max = -(need + current);
            let expected_current = expected_max - maximum;
            let trio = publish(need, current, maximum).expect("in range");
            assert_eq!(trio.needed_max, expected_max);
            assert_eq!(trio.needed_current, expected_current);
            assert_eq!(trio.needed, expected_current);
        }
    }

    #[test]
    fn the_publication_arithmetic_can_refuse_where_both_usages_succeeded() {
        // `ARITHMETIC.md` §5, Codex's counterexample, checked as a behaviour of
        // this module: with need = i32::MIN and a current usage of 0,
        // `checked_neg` on i32::MIN refuses, so this module publishes nothing.
        //
        // What the SHIPPING caller does with that refusal is not "the row goes
        // UNAVAILABLE", which an earlier version of this comment said (Codex
        // round 8, blocking 6). A refusal at the publication step propagates
        // through `?` and truncates the country's whole row set;
        // `runs/te_d43_piggyback/SELFCHECK.md` §6 hands that on. Only a refusal
        // EARLIER, inside `usage`, reaches the `NEEDED_UNAVAILABLE` arm.
        //
        // kills: replacing the checked chain with wrapping arithmetic, which
        // would publish i32::MIN as an observed "needed_max".
        let empty: Vec<RosterMember> = Vec::new();
        let mut inputs = ordinary_inputs(&empty, &empty);
        inputs.need = i32::MIN;
        // Make the current usage exactly 0 so `need + current` is i32::MIN.
        inputs.fuel = FuelScalars {
            base_first: 0,
            base_second: 0,
            stock: 0,
            denominator: 1_000,
        };
        inputs.air_land_current = 0;
        inputs.air_land_max = 0;
        assert_eq!(usage(&inputs).map(|pair| pair.0), Some(0));
        assert_eq!(derive(&inputs), None);
    }

    #[test]
    fn the_rounding_change_moves_published_words_up_and_never_down() {
        // `ARITHMETIC.md` §5: the new helper returning one LOWER raises both
        // published words, because §1 negates both usages. Constructed from the
        // two helper forms directly so the direction is checked, not asserted.
        //
        // kills: a future "fix" that reverts to the ceiling form on the grounds
        // that it reads more simply -- that would move published words DOWN.
        let denominator = 100_001i64;
        let residual = 1i64; // f = 1/100001, inside (0, 1/100000)
        let new = requirement(residual, denominator).unwrap();
        let old = old_ceiling_form(residual, denominator).unwrap();
        assert_eq!(new, 0);
        assert_eq!(old, 1);
        let need = 5i32;
        let needed_max_new = -(need + new);
        let needed_max_old = -(need + old);
        assert!(needed_max_new > needed_max_old);
        assert_eq!(needed_max_new - needed_max_old, old - new);
    }
}
