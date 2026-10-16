//! Counter bars and status glyphs from `DivisionStatus`.
//!
//! Rules take plain numbers so arithmetic can be checked on synthetic inputs.
//! Hit points, organisation and status magnitudes are signed i64 values over
//! 100,000 ([`FIXED`]). [`stack_fill`] converts a ratio to the drawn fraction.
//! Bar geometry comes from the resolved sprite and window; the fill uses the
//! integer steps in [`super::counter_inputs::bar_pixels`].

/// The native fixed-point scale: one whole unit is 100 000.
pub const FIXED: i64 = 100_000;

/// Raw fixed5 NGraphics thresholds from the matching content, in native test
/// order. Mods may replace them, so the renderer must not bake in vanilla.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupplyThresholds {
    pub very_low: i64,
    pub low: i64,
    pub show: i64,
}

impl SupplyThresholds {
    /// `SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD`, `_LOW_THRESHOLD` and
    /// `_SHOW_THRESHOLD` through the native fixed-point getter.
    pub fn read(defines: &installed_content::defines::Defines) -> std::io::Result<Self> {
        let value = |name| defines.fixed5("NGraphics", name);
        Ok(Self {
            very_low: value("SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD")?,
            low: value("SUPPLY_UNIT_COUNTER_LOW_THRESHOLD")?,
            show: value("SUPPLY_UNIT_COUNTER_SHOW_THRESHOLD")?,
        })
    }
}

/// Raw fixed-point divide-by-zero guard: one thousandth of a whole unit.
pub const MAXIMUM_FLOOR: i64 = 100;

/// Maximum used by a counter bar. Recorded maxima already apply this floor;
/// retain the clamp for synthetic inputs and a recorded zero maximum.
pub fn effective_maximum(recorded: i64) -> i64 {
    recorded.max(MAXIMUM_FLOOR)
}

/// Fixed-point current value over the effective maximum, clamped to a whole unit.
/// The three unusable signed current-value sentinels read as full.
pub fn ratio(current: i64, maximum: i64) -> i64 {
    let maximum = effective_maximum(maximum);
    if (current as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        FIXED
    } else {
        (current.wrapping_mul(FIXED) / maximum).clamp(0, FIXED)
    }
}

/// Strength-bar contribution is [`ratio`] times the recorded strength factor,
/// with wrapping multiplication, truncating fixed-point division and a whole-unit
/// clamp. Organisation has no strength factor.
pub fn modified_strength(current: i64, maximum: i64, factor: i64) -> i64 {
    (ratio(current, maximum).wrapping_mul(factor) / FIXED).clamp(0, FIXED)
}

/// maximum gates the getter; remaining grace
/// divides by the content's integer SUPPLY_GRACE, then competes with received supply.
pub fn effective_supply(received: i64, maximum: i64, remaining: i64, grace: i32) -> i64 {
    if maximum < 1 {
        return 0;
    }
    let denominator = i64::from(grace) * FIXED;
    let fraction = if denominator == 0
        || (remaining as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
    {
        super::counter_inputs::UNDEFINED
    } else {
        remaining.wrapping_mul(FIXED) / denominator
    };
    received.max(fraction).clamp(0, FIXED)
}

/// Mean of visible members' fixed-point ratios, using integer truncation before
/// conversion to the drawn fraction. Members contribute equally regardless of
/// their maxima.
pub fn stack_fill(ratios: &[i64]) -> f32 {
    super::counter_inputs::member_mean(ratios.iter().copied())
        .map_or(0.0, super::counter_inputs::bar_fill)
}

/// One division's status, as far as the counter's glyphs are concerned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Member {
    /// The raw recorded `DivisionStatus.supply_ratio`.
    pub supply_ratio: i64,
    /// `DivisionMovement.naval_transfer`: the member is riding a convoy.
    pub naval_transfer: bool,
    pub entrenchment: i64,
    pub entrenchment_cap: i64,
    /// The division's stored attrition exceeds its recorded threshold. This is the counter's
    /// attrition-presence predicate.
    pub attrition: bool,
    pub exercising: u8,
    /// The raw division's group is present; an ungrouped member skips both
    /// order-icon branches even when its invalid-orders byte is nonzero.
    pub grouped: bool,
    /// Canonical roster owner equals the viewer's chosen country.
    pub own: bool,
    /// Derived from the same-hour order roots and assigned division vectors.
    /// `None` means a reached root or group was unavailable.
    pub has_order: Option<bool>,
    /// The recorded `DivisionStatus.invalid_orders`.
    pub invalid_orders: u8,
}

/// Which glyphs one stack's counter hangs off its plate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Glyphs {
    /// Frame of `GFX_unitcounter_supply_status_strip`, or `None` for no crate.
    pub supply: Option<u32>,
    /// Frame of `GFX_onmap_digin_icon`, or `None` for no spade.
    pub digin: Option<u32>,
    pub attrition: bool,
    pub training: bool,
    /// Unknown when missing order inputs could change the red latch.
    pub no_orders: Option<bool>,
    /// The yellow `!` can coexist with the red one in a mixed stack.
    pub invalid_orders: Option<bool>,
}

/// Compare supply with the content's very-low, low and show thresholds in that
/// order. Strict comparisons select zero-based frames two, one and zero; supply
/// at or above the show threshold hides the crate.
pub fn supply_frame(supply_ratio: i64, thresholds: SupplyThresholds) -> Option<u32> {
    if supply_ratio < thresholds.very_low {
        Some(2)
    } else if supply_ratio < thresholds.low {
        Some(1)
    } else if supply_ratio < thresholds.show {
        Some(0)
    } else {
        None
    }
}

/// Supply follows the last visible member outside naval transfer, in stack order.
/// A stack consisting only of transferred members retains full supply.
pub fn stack_supply(members: &[Member]) -> i64 {
    members
        .iter()
        .filter(|member| !member.naval_transfer)
        .map(|member| member.supply_ratio)
        .next_back()
        .unwrap_or(FIXED)
}

/// Show a spade when any member has positive entrenchment. Use its full frame
/// only when every entrenched member has reached its own cap.
pub fn digin_frame(members: &[Member]) -> Option<u32> {
    let mut any = false;
    let mut at_cap = true;
    for member in members {
        if member.entrenchment > 0 {
            any = true;
            if member.entrenchment < member.entrenchment_cap {
                at_cap = false;
            }
        }
    }
    any.then(|| u32::from(at_cap))
}

/// The two independent order-icon latches. It visits only fully
/// visible members in their native stack order. A grouped own member without
/// an order latches red; once red is set, later grouped members can set yellow
/// from their raw invalid-orders byte without consulting their orders. The two
/// icons can therefore coexist. A missing order input branches both possible
/// outcomes, and an icon is certain only when every remaining state agrees.
pub fn order_glyphs(members: &[Member]) -> (Option<bool>, Option<bool>) {
    // Four possible (red, yellow) states; bit 0 is red and bit 1 is yellow.
    let mut possible = 0b0001_u8;
    for member in members {
        if !member.grouped {
            continue;
        }
        let mut next = 0_u8;
        for state in 0..4_u8 {
            if possible & (1 << state) == 0 {
                continue;
            }
            let red = state & 1 != 0;
            let yellow = state & 2 != 0;
            let yellow_state =
                u8::from(red) | (u8::from(yellow || member.invalid_orders != 0) << 1);
            if !member.own || red {
                next |= 1 << yellow_state;
            } else {
                match member.has_order {
                    Some(true) => next |= 1 << yellow_state,
                    Some(false) => next |= 1 << (state | 1),
                    None => {
                        next |= 1 << yellow_state;
                        next |= 1 << (state | 1);
                    }
                }
            }
        }
        possible = next;
    }
    let icon = |mask: u8| {
        let on = (0..4_u8).any(|state| possible & (1 << state) != 0 && state & mask != 0);
        let off = (0..4_u8).any(|state| possible & (1 << state) != 0 && state & mask == 0);
        match (on, off) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        }
    };
    (icon(1), icon(2))
}

/// Fold supply, dig-in, attrition and training over the stack. Recorded attrition
/// and exercise predicates retain their own modifier and deployment gates; raw
/// training requests alone do not establish exercise. Order glyphs use their
/// ordered latches and preserve unavailable membership. Training hides the spade.
pub fn glyphs(members: &[Member], thresholds: Option<SupplyThresholds>) -> Glyphs {
    let training = members.iter().any(|member| member.exercising != 0);
    let (no_orders, invalid_orders) = order_glyphs(members);
    Glyphs {
        supply: thresholds.and_then(|limits| supply_frame(stack_supply(members), limits)),
        digin: if training { None } else { digin_frame(members) },
        attrition: members.iter().any(|member| member.attrition),
        training,
        no_orders,
        invalid_orders,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_supply_keeps_native_grace_gate_wrapping_and_sentinels() {
        assert_eq!(effective_supply(FIXED, 0, FIXED, 4), 0);
        assert_eq!(effective_supply(20_000, FIXED, 200_000, 4), 50_000);
        assert_eq!(effective_supply(80_000, FIXED, 200_000, 4), 80_000);
        assert_eq!(effective_supply(-1, FIXED, -1, 4), 0);
        assert_eq!(effective_supply(0, FIXED, FIXED, 0), FIXED);
        for remaining in [i64::MAX, i64::MIN, i64::MIN + 1] {
            assert_eq!(effective_supply(0, FIXED, remaining, 4), FIXED);
        }
        let remaining = i64::MAX / FIXED + 1;
        assert_eq!(effective_supply(0, FIXED, remaining, 4), 0);
    }

    const VANILLA_SUPPLY: SupplyThresholds = SupplyThresholds {
        very_low: 20_000,
        low: 35_000,
        show: 50_000,
    };

    fn member(supply: i64) -> Member {
        Member {
            supply_ratio: supply,
            ..Member::default()
        }
    }

    #[test]
    fn a_ratio_is_the_division_over_its_own_maximum() {
        // 12.5 organisation out of a 25.0 maximum is half, whatever the scale.
        assert_eq!(ratio(12 * FIXED + FIXED / 2, 25 * FIXED), FIXED / 2);
        assert_eq!(ratio(0, 25 * FIXED), 0);
        assert_eq!(ratio(25 * FIXED, 25 * FIXED), FIXED);
        // A division over its template clamps rather than overdrawing.
        assert_eq!(ratio(30 * FIXED, 25 * FIXED), FIXED);
        // A negative current is an empty bar, not a negative one.
        assert_eq!(ratio(-5 * FIXED, 25 * FIXED), 0);
        // The engine's sentinel currents read as full.
        assert_eq!(ratio(i64::MAX, 25 * FIXED), FIXED);
        // The native division truncates: 2/3 is 66666, not 66667.
        assert_eq!(ratio(2 * FIXED, 3 * FIXED), 66_666);
    }

    #[test]
    fn the_maximum_floor_is_a_raw_guard_and_not_a_hundred_whole_points() {
        assert_eq!(effective_maximum(30 * FIXED), 30 * FIXED);
        assert_eq!(effective_maximum(0), MAXIMUM_FLOOR);
        assert_eq!(effective_maximum(-1), MAXIMUM_FLOOR);
        // The raw maximum floor preserves a full bar at full organisation.
        assert_eq!(ratio(30 * FIXED, 30 * FIXED), FIXED);
    }

    #[test]
    fn the_strength_bar_multiplies_the_ratio_by_the_division_factor() {
        // A full-strength synthetic division with a manpower factor near half draws
        // a strength bar near half while its unmodified ratio stays full.
        let full = 5_560_832;
        assert_eq!(modified_strength(full, full, FIXED), FIXED);
        // A fixed-point factor of 55442 leaves the full-strength ratio at 55442.
        assert_eq!(modified_strength(full, full, 55_442), 55_442);
        // The plain ratio is truncated before the factor applies, as the native integer steps are.
        assert_eq!(modified_strength(2 * FIXED, 3 * FIXED, 50_000), 33_333);
        // Both steps clamp: an over-full division and a factor above one stay full, a negative
        // factor or hit points stay empty.
        assert_eq!(modified_strength(30 * FIXED, 25 * FIXED, 150_000), FIXED);
        assert_eq!(modified_strength(full, full, -1), 0);
        assert_eq!(modified_strength(-5 * FIXED, 25 * FIXED, FIXED), 0);
        // The engine's sentinel hit points read as a full ratio.
        assert_eq!(modified_strength(i64::MAX, 25 * FIXED, 40_000), 40_000);
    }

    #[test]
    fn a_stack_averages_its_members_fixed_point_ratios() {
        // One full division with a small maximum and one empty division with a large one.
        let stack = [ratio(20 * FIXED, 20 * FIXED), ratio(0, 200 * FIXED)];
        assert_eq!(stack_fill(&stack), 0.5);
        assert_eq!(stack_fill(&[]), 0.0);
        // A stack of equally spent divisions is a quarter bar, not a full one.
        assert_eq!(stack_fill(&[25_000, 25_000]), 0.25);
        // The mean truncates in fixed point before the float conversion.
        assert_eq!(stack_fill(&[1, 2]), 0.00001);
    }

    #[test]
    fn the_supply_crate_steps_through_the_three_defines() {
        let limits = VANILLA_SUPPLY;
        assert_eq!(supply_frame(FIXED, limits), None);
        assert_eq!(supply_frame(limits.show, limits), None);
        assert_eq!(supply_frame(limits.show - 1, limits), Some(0));
        assert_eq!(supply_frame(limits.low, limits), Some(0));
        assert_eq!(supply_frame(limits.low - 1, limits), Some(1));
        assert_eq!(supply_frame(limits.very_low, limits), Some(1));
        assert_eq!(supply_frame(limits.very_low - 1, limits), Some(2));
        assert_eq!(supply_frame(0, limits), Some(2));
        let modded = SupplyThresholds {
            very_low: 10_000,
            low: 20_000,
            show: 30_000,
        };
        assert_eq!(supply_frame(25_000, modded), Some(0));
        assert_eq!(supply_frame(25_000, limits), Some(1));
    }

    #[test]
    fn the_stacks_supply_is_its_last_member_that_is_not_at_sea() {
        assert_eq!(stack_supply(&[]), FIXED);
        assert_eq!(stack_supply(&[member(FIXED), member(10_000)]), 10_000);
        // The engine overwrites rather than taking a minimum, so order decides.
        assert_eq!(stack_supply(&[member(10_000), member(FIXED)]), FIXED);
        let at_sea = Member {
            naval_transfer: true,
            ..member(0)
        };
        assert_eq!(stack_supply(&[member(10_000), at_sea]), 10_000);
        // A stack wholly under naval transfer keeps the seeded full ratio and draws no crate.
        assert_eq!(stack_supply(&[at_sea]), FIXED);
        assert_eq!(glyphs(&[at_sea], Some(VANILLA_SUPPLY)).supply, None);
    }

    #[test]
    fn the_spade_appears_for_any_entrenched_member_and_fills_only_when_all_are_capped() {
        let dug = |entrenchment, cap| Member {
            entrenchment,
            entrenchment_cap: cap,
            ..Member::default()
        };
        assert_eq!(digin_frame(&[]), None);
        assert_eq!(digin_frame(&[dug(0, 5 * FIXED)]), None);
        assert_eq!(digin_frame(&[dug(FIXED, 5 * FIXED)]), Some(0));
        assert_eq!(digin_frame(&[dug(5 * FIXED, 5 * FIXED)]), Some(1));
        // One member short of its cap keeps the partial frame for the whole stack.
        assert_eq!(
            digin_frame(&[dug(5 * FIXED, 5 * FIXED), dug(FIXED, 5 * FIXED)]),
            Some(0)
        );
        // A member with no entrenchment at all does not hold the stack back.
        assert_eq!(
            digin_frame(&[dug(5 * FIXED, 5 * FIXED), dug(0, 5 * FIXED)]),
            Some(1)
        );
    }

    #[test]
    fn the_glyph_predicates_are_ored_over_the_stack_and_exercising_hides_the_spade() {
        let quiet = member(FIXED);
        let busy = Member {
            entrenchment: FIXED,
            entrenchment_cap: FIXED,
            attrition: true,
            exercising: 1,
            grouped: true,
            own: true,
            has_order: Some(false),
            ..member(FIXED)
        };
        let out = glyphs(&[quiet, busy], Some(VANILLA_SUPPLY));
        assert!(out.attrition);
        assert!(out.training);
        assert_eq!(out.no_orders, Some(true));
        assert_eq!(
            out.invalid_orders,
            Some(false),
            "the byte is clear on this member"
        );
        // Hide the dig-in icon while the stack is exercising.
        assert_eq!(out.digin, None);
        // A supplied stack carries no crate at all.
        assert_eq!(glyphs(&[quiet], Some(VANILLA_SUPPLY)).supply, None);
        let calm = glyphs(
            &[Member {
                exercising: 0,
                ..busy
            }],
            Some(VANILLA_SUPPLY),
        );
        assert_eq!(calm.digin, Some(1));
        assert!(!calm.training);
    }

    #[test]
    fn order_glyphs_follow_own_group_and_prior_red_latch_in_member_order() {
        let own_ordered = Member {
            grouped: true,
            own: true,
            has_order: Some(true),
            invalid_orders: 1,
            ..Member::default()
        };
        let own_unordered = Member {
            has_order: Some(false),
            ..own_ordered
        };
        let foreign_unordered = Member {
            own: false,
            ..own_unordered
        };
        assert_eq!(order_glyphs(&[own_ordered]), (Some(false), Some(true)));
        assert_eq!(order_glyphs(&[own_unordered]), (Some(true), Some(false)));
        assert_eq!(
            order_glyphs(&[foreign_unordered]),
            (Some(false), Some(true))
        );
        assert_eq!(
            order_glyphs(&[own_unordered, own_ordered]),
            (Some(true), Some(true)),
            "the later member can light yellow after red latched"
        );
        assert_eq!(
            order_glyphs(&[own_ordered, own_unordered]),
            (Some(true), Some(true)),
            "an earlier yellow survives a later red"
        );
        assert_eq!(
            order_glyphs(&[Member {
                grouped: false,
                invalid_orders: u8::MAX,
                ..own_unordered
            }]),
            (Some(false), Some(false))
        );
    }

    #[test]
    fn order_glyphs_preserve_unknown_membership_without_erasing_certain_icons() {
        let unknown = Member {
            grouped: true,
            own: true,
            has_order: None,
            invalid_orders: 1,
            ..Member::default()
        };
        assert_eq!(order_glyphs(&[unknown]), (None, None));
        assert_eq!(
            order_glyphs(&[
                unknown,
                Member {
                    own: false,
                    ..unknown
                }
            ]),
            (None, Some(true)),
            "the foreign member always lights yellow"
        );
        assert_eq!(
            order_glyphs(&[
                Member {
                    own: false,
                    ..unknown
                },
                unknown
            ]),
            (None, Some(true)),
            "a proven yellow survives a later unknown"
        );
        assert_eq!(
            order_glyphs(&[
                Member {
                    has_order: Some(false),
                    ..unknown
                },
                unknown
            ]),
            (Some(true), Some(true)),
            "the red latch bypasses later missing membership"
        );
    }
}
