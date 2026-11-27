
pub const FIXED: i64 = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupplyThresholds {
    pub very_low: i64,
    pub low: i64,
    pub show: i64,
}

impl SupplyThresholds {
    pub fn read(defines: &content_reader_defines::defines::Defines) -> std::io::Result<Self> {
        let value = |name| defines.fixed5("NGraphics", name);
        Ok(Self {
            very_low: value("SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD")?,
            low: value("SUPPLY_UNIT_COUNTER_LOW_THRESHOLD")?,
            show: value("SUPPLY_UNIT_COUNTER_SHOW_THRESHOLD")?,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub const FIXTURE: Self = Self {
        very_low: 17_000,
        low: 33_000,
        show: 57_000,
    };
}

pub const MAXIMUM_FLOOR: i64 = 100;

pub fn effective_maximum(recorded: i64) -> i64 {
    recorded.max(MAXIMUM_FLOOR)
}

pub fn ratio(current: i64, maximum: i64) -> i64 {
    let maximum = effective_maximum(maximum);
    if (current as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        FIXED
    } else {
        (current.wrapping_mul(FIXED) / maximum).clamp(0, FIXED)
    }
}

pub fn modified_strength(current: i64, maximum: i64, factor: i64) -> i64 {
    (ratio(current, maximum).wrapping_mul(factor) / FIXED).clamp(0, FIXED)
}

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

pub fn stack_fill(ratios: &[i64]) -> f32 {
    super::counter_inputs::member_mean(ratios.iter().copied())
        .map_or(0.0, super::counter_inputs::bar_fill)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupplyRatio {
    Exact(i64),
    Interval([i64; 2]),
    Missing,
}

impl Default for SupplyRatio {
    fn default() -> Self {
        Self::Exact(0)
    }
}

impl SupplyRatio {
    pub const fn exact(value: i64) -> Self {
        Self::Exact(value)
    }

    pub const fn from_bounds(bounds: [i64; 2]) -> Self {
        if bounds[0] == bounds[1] {
            Self::Exact(bounds[0])
        } else {
            Self::Interval(bounds)
        }
    }

    pub const fn bounds(self) -> Option<[i64; 2]> {
        match self {
            Self::Exact(value) => Some([value, value]),
            Self::Interval(bounds) => Some(bounds),
            Self::Missing => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SupplyFrame {
    #[default]
    Hidden,
    Frame(u32),
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    pub supply_ratio: SupplyRatio,
    pub naval_transfer: Option<bool>,
    pub entrenchment: i64,
    pub entrenchment_cap: i64,
    pub attrition: bool,
    pub exercising: Option<bool>,
    pub grouped: bool,
    pub own: bool,
    pub has_order: Option<bool>,
    pub invalid_orders: u8,
}

impl Default for Member {
    fn default() -> Self {
        Self {
            supply_ratio: SupplyRatio::default(),
            naval_transfer: Some(false),
            entrenchment: 0,
            entrenchment_cap: 0,
            attrition: false,
            exercising: None,
            grouped: false,
            own: false,
            has_order: None,
            invalid_orders: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Glyphs {
    pub supply: SupplyFrame,
    pub digin: Option<u32>,
    pub attrition: bool,
    pub training: Option<bool>,
    pub no_orders: Option<bool>,
    pub invalid_orders: Option<bool>,
}

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

pub fn supply_frame_interval(bounds: [i64; 2], thresholds: SupplyThresholds) -> SupplyFrame {
    let [low, high] = bounds;
    let first = supply_frame(low, thresholds);
    for threshold in [thresholds.very_low, thresholds.low, thresholds.show] {
        if low < threshold && threshold <= high && supply_frame(threshold, thresholds) != first {
            return SupplyFrame::Uncertain;
        }
    }
    match first {
        Some(frame) => SupplyFrame::Frame(frame),
        None => SupplyFrame::Hidden,
    }
}

pub fn supply_frame_for(ratio: SupplyRatio, thresholds: SupplyThresholds) -> SupplyFrame {
    match ratio.bounds() {
        Some(bounds) => supply_frame_interval(bounds, thresholds),
        None => SupplyFrame::Uncertain,
    }
}

pub fn stack_supply(members: &[Member]) -> SupplyRatio {
    members
        .iter()
        .filter(|member| member.naval_transfer != Some(true))
        .map(|member| member.supply_ratio)
        .next_back()
        .unwrap_or(SupplyRatio::Exact(FIXED))
}

pub fn stack_supply_frame(members: &[Member], thresholds: SupplyThresholds) -> SupplyFrame {
    let mut decided: Option<SupplyFrame> = None;
    let mut observe = |candidate: SupplyFrame| -> bool {
        if candidate == SupplyFrame::Uncertain {
            return false;
        }
        match decided {
            None => {
                decided = Some(candidate);
                true
            }
            Some(existing) => existing == candidate,
        }
    };
    let mut guaranteed = false;
    for member in members.iter().rev() {
        match member.naval_transfer {
            Some(true) => continue,
            Some(false) => {
                if !observe(supply_frame_for(member.supply_ratio, thresholds)) {
                    return SupplyFrame::Uncertain;
                }
                guaranteed = true;
                break;
            }
            None => {
                if !observe(supply_frame_for(member.supply_ratio, thresholds)) {
                    return SupplyFrame::Uncertain;
                }
            }
        }
    }
    if !guaranteed && !observe(supply_frame_for(SupplyRatio::Exact(FIXED), thresholds)) {
        return SupplyFrame::Uncertain;
    }
    decided.unwrap_or(SupplyFrame::Hidden)
}

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

pub fn order_glyphs(members: &[Member]) -> (Option<bool>, Option<bool>) {
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

pub fn stack_training(members: &[Member]) -> Option<bool> {
    if members.iter().any(|member| member.exercising == Some(true)) {
        Some(true)
    } else {
        members
            .iter()
            .all(|member| member.exercising.is_some())
            .then_some(false)
    }
}

pub fn glyphs(members: &[Member], thresholds: Option<SupplyThresholds>) -> Glyphs {
    let training = stack_training(members);
    let (no_orders, invalid_orders) = order_glyphs(members);
    Glyphs {
        supply: thresholds.map_or(SupplyFrame::Hidden, |limits| {
            stack_supply_frame(members, limits)
        }),
        digin: match training {
            Some(false) => digin_frame(members),
            _ => None,
        },
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

    const FIXTURE_SUPPLY: SupplyThresholds = SupplyThresholds::FIXTURE;

    fn member(supply: i64) -> Member {
        Member {
            supply_ratio: SupplyRatio::exact(supply),
            ..Member::default()
        }
    }

    #[test]
    fn a_ratio_is_the_division_over_its_own_maximum() {
        assert_eq!(ratio(12 * FIXED + FIXED / 2, 25 * FIXED), FIXED / 2);
        assert_eq!(ratio(0, 25 * FIXED), 0);
        assert_eq!(ratio(25 * FIXED, 25 * FIXED), FIXED);
        assert_eq!(ratio(30 * FIXED, 25 * FIXED), FIXED);
        assert_eq!(ratio(-5 * FIXED, 25 * FIXED), 0);
        assert_eq!(ratio(i64::MAX, 25 * FIXED), FIXED);
        assert_eq!(ratio(2 * FIXED, 3 * FIXED), 66_666);
    }

    #[test]
    fn the_maximum_floor_is_a_raw_guard_and_not_a_hundred_whole_points() {
        assert_eq!(effective_maximum(30 * FIXED), 30 * FIXED);
        assert_eq!(effective_maximum(0), MAXIMUM_FLOOR);
        assert_eq!(effective_maximum(-1), MAXIMUM_FLOOR);
        assert_eq!(ratio(30 * FIXED, 30 * FIXED), FIXED);
    }

    #[test]
    fn the_strength_bar_multiplies_the_ratio_by_the_division_factor() {
        let full = 5_560_832;
        assert_eq!(modified_strength(full, full, FIXED), FIXED);
        assert_eq!(modified_strength(full, full, 55_442), 55_442);
        assert_eq!(modified_strength(2 * FIXED, 3 * FIXED, 50_000), 33_333);
        assert_eq!(modified_strength(30 * FIXED, 25 * FIXED, 150_000), FIXED);
        assert_eq!(modified_strength(full, full, -1), 0);
        assert_eq!(modified_strength(-5 * FIXED, 25 * FIXED, FIXED), 0);
        assert_eq!(modified_strength(i64::MAX, 25 * FIXED, 40_000), 40_000);
    }

    #[test]
    fn the_supply_crate_steps_through_the_three_defines() {
        let limits = FIXTURE_SUPPLY;
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
        assert_eq!(stack_supply(&[]), SupplyRatio::exact(FIXED));
        assert_eq!(
            stack_supply(&[member(FIXED), member(10_000)]),
            SupplyRatio::exact(10_000)
        );
        assert_eq!(
            stack_supply(&[member(10_000), member(FIXED)]),
            SupplyRatio::exact(FIXED)
        );
        let at_sea = Member {
            naval_transfer: Some(true),
            ..member(0)
        };
        assert_eq!(
            stack_supply(&[member(10_000), at_sea]),
            SupplyRatio::exact(10_000)
        );
        assert_eq!(stack_supply(&[at_sea]), SupplyRatio::exact(FIXED));
        let unknown_low = Member {
            naval_transfer: None,
            ..member(10_000)
        };
        let unknown_full = Member {
            naval_transfer: None,
            ..member(80_000)
        };
        assert_eq!(
            stack_supply_frame(&[unknown_low], FIXTURE_SUPPLY),
            SupplyFrame::Uncertain
        );
        assert_eq!(
            stack_supply_frame(&[unknown_full], FIXTURE_SUPPLY),
            SupplyFrame::Hidden
        );
        assert_eq!(
            stack_supply_frame(&[unknown_low, member(10_000)], FIXTURE_SUPPLY),
            SupplyFrame::Frame(2)
        );
        assert_eq!(
            stack_supply_frame(&[member(10_000), unknown_low], FIXTURE_SUPPLY),
            SupplyFrame::Frame(2)
        );
        assert_eq!(
            stack_supply_frame(&[member(80_000), unknown_low], FIXTURE_SUPPLY),
            SupplyFrame::Uncertain
        );
        assert_eq!(
            glyphs(&[at_sea], Some(FIXTURE_SUPPLY)).supply,
            SupplyFrame::Hidden
        );
    }

    #[test]
    fn supply_frame_interval_proves_every_on_grid_step_and_flags_straddling_mod_thresholds() {
        use replay_data::rows::DivisionStatusRow;
        for step in -2..=105 {
            let shown = step * 1_000;
            let bounds = DivisionStatusRow::SUPPLY_RATIO_STEP.toward_zero_bounds(shown);
            let expected = match supply_frame(shown, FIXTURE_SUPPLY) {
                Some(frame) => SupplyFrame::Frame(frame),
                None => SupplyFrame::Hidden,
            };
            assert_eq!(
                supply_frame_interval(bounds, FIXTURE_SUPPLY),
                expected,
                "{shown} -> {bounds:?}"
            );
        }
        let off_grid = SupplyThresholds {
            very_low: 20_500,
            low: 35_500,
            show: 50_500,
        };
        assert_eq!(
            supply_frame_interval([34_000, 34_999], off_grid),
            SupplyFrame::Frame(1)
        );
        assert_eq!(
            supply_frame_interval([35_000, 35_999], off_grid),
            SupplyFrame::Uncertain
        );
        assert_eq!(
            supply_frame_interval([36_000, 36_999], off_grid),
            SupplyFrame::Frame(0)
        );
        assert_eq!(
            supply_frame_interval([35_499, 35_499], off_grid),
            SupplyFrame::Frame(1)
        );
        assert_eq!(
            supply_frame_interval([35_500, 35_500], off_grid),
            SupplyFrame::Frame(0)
        );
        let shadowed = SupplyThresholds {
            very_low: 40_000,
            low: 35_500,
            show: FIXTURE_SUPPLY.show,
        };
        assert_eq!(
            supply_frame_interval([35_000, 35_999], shadowed),
            SupplyFrame::Frame(2)
        );
        let zero_boundary = SupplyThresholds {
            very_low: 0,
            low: FIXTURE_SUPPLY.low,
            show: FIXTURE_SUPPLY.show,
        };
        assert_eq!(
            supply_frame_interval([-999, 999], zero_boundary),
            SupplyFrame::Uncertain
        );
        assert_eq!(
            supply_frame_for(SupplyRatio::Missing, FIXTURE_SUPPLY),
            SupplyFrame::Uncertain
        );
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
        assert_eq!(
            digin_frame(&[dug(5 * FIXED, 5 * FIXED), dug(FIXED, 5 * FIXED)]),
            Some(0)
        );
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
            exercising: Some(true),
            grouped: true,
            own: true,
            has_order: Some(false),
            ..member(FIXED)
        };
        let out = glyphs(&[quiet, busy], Some(FIXTURE_SUPPLY));
        assert!(out.attrition);
        assert_eq!(out.training, Some(true));
        assert_eq!(out.no_orders, Some(true));
        assert_eq!(
            out.invalid_orders,
            Some(false),
            "the byte is clear on this member"
        );
        assert_eq!(out.digin, None);
        assert_eq!(
            glyphs(&[quiet], Some(FIXTURE_SUPPLY)).supply,
            SupplyFrame::Hidden
        );
        let calm = glyphs(
            &[Member {
                exercising: Some(false),
                ..busy
            }],
            Some(FIXTURE_SUPPLY),
        );
        assert_eq!(calm.digin, Some(1));
        assert_eq!(calm.training, Some(false));
    }

    #[test]
    fn an_unknown_exercise_predicate_is_never_off_but_never_hides_a_known_one() {
        let known = |exercising| Member {
            exercising: Some(exercising),
            entrenchment: FIXED,
            entrenchment_cap: FIXED,
            ..member(FIXED)
        };
        let unknown = Member {
            exercising: None,
            ..known(false)
        };
        assert_eq!(stack_training(&[]), Some(false));
        assert_eq!(stack_training(&[known(false), known(false)]), Some(false));
        assert_eq!(stack_training(&[unknown, known(true)]), Some(true));
        assert_eq!(stack_training(&[known(false), unknown]), None);
        let unsure = glyphs(&[known(false), unknown], Some(FIXTURE_SUPPLY));
        assert_eq!((unsure.training, unsure.digin), (None, None));
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
