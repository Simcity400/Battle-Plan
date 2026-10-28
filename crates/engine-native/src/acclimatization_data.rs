//! Embedded division acclimatization state; hourly consistency is unverified.

/// Embedded acclimatization object vtable.
pub const VTABLE_RVA: usize = 0x0285_bb80;
/// Category/weight vector data.
pub const DATA: usize = 0x08;
/// Vector capacity, u32.
pub const CAPACITY: usize = 0x10;
/// Vector count, u32.
pub const COUNT: usize = 0x14;
/// Maximum, i64 raw value.
pub const MAXIMUM: usize = 0x38;
/// The category whose climate the division stands in now: the pointer the
/// movement availability calculation compares the winning entry's category with
/// (null: no climate, factor one).
pub const ACTIVE: usize = 0x20;
/// An acclimatization category's factor, i64 fixed5, that the movement
/// availability calculation scales the winning weight's share by.
pub const CATEGORY_FACTOR: usize = 0x30;
/// `NDefines::NGraphics::ACCLIMATIZATION_CAMO_SHOW_AT`, a signed fixed5 word
/// loaded from the content's defines and multiplied by the maximum for the
/// camouflage threshold.
pub const CAMO_SHOW_AT_RVA: usize = 0x033e_1658;

/// Select from a division's entries offered in native order: the first strictly
/// greatest weight above zero. It starts from zero and replaces its winner only
/// on a strictly greater weight, so ties keep the first and a list whose
/// weights are all zero or negative has none.
#[derive(Clone, Copy, Debug)]
pub struct Strongest<C> {
    winner: Option<(C, i64)>,
}

impl<C: Copy> Default for Strongest<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Copy> Strongest<C> {
    pub const fn new() -> Self {
        Self { winner: None }
    }

    pub fn offer(&mut self, category: C, weight: i64) {
        if weight > self.winner.map_or(0, |(_, highest)| highest) {
            self.winner = Some((category, weight));
        }
    }

    /// The winning entry's category and weight.
    pub fn winner(&self) -> Option<(C, i64)> {
        self.winner
    }
}

/// The division shows the winning category's camouflage unless its weight is
/// below `maximum * show_at / 100000`, using a wrapping signed multiply and
/// division truncating toward zero.
pub fn shows(weight: i64, maximum: i64, show_at: i64) -> bool {
    weight >= maximum.wrapping_mul(show_at) / 100_000
}

/// Movement availability once [`Strongest`]'s winner is the active category: the winning weight's
/// share of the maximum behind the native guard (a zero maximum, or a weight of `i64::MIN`, `MIN +
/// 1` or `MAX`, stores `0xFFFFFFFF`), times the category's factor, clamped to `[0, 1]` and taken
/// from one. A negative product is one. Products wrap and divisions truncate toward zero.
pub fn movement_factor(weight: i64, maximum: i64, category_factor: i64) -> i64 {
    const ONE: i64 = 100_000;
    let guarded = (weight as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc;
    let fraction = if maximum == 0 || guarded {
        0xffff_ffff
    } else {
        weight.wrapping_mul(ONE).wrapping_div(maximum)
    };
    let scaled = fraction.wrapping_mul(category_factor) / ONE;
    if scaled < 0 {
        ONE
    } else {
        ONE - scaled.min(ONE)
    }
}

/// The division's camouflage category: the entry [`Strongest`] selects when its
/// weight [`shows`], otherwise none. A caller holding category pointers still
/// treats a null winner as none before testing the threshold.
pub fn camouflage<C: Copy>(
    entries: impl IntoIterator<Item = (C, i64)>,
    maximum: i64,
    show_at: i64,
) -> Option<C> {
    let mut strongest = Strongest::new();
    for (category, weight) in entries {
        strongest.offer(category, weight);
    }
    let (category, weight) = strongest.winner()?;
    shows(weight, maximum, show_at).then_some(category)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movement_factor_follows_the_native_guard_and_clamp() {
        assert_eq!(movement_factor(50, 100, 50_000), 75_000);
        assert_eq!(movement_factor(100, 100, 100_000), 0);
        // Beyond one clamps; a negative product is one.
        assert_eq!(movement_factor(300, 100, 100_000), 0);
        assert_eq!(movement_factor(50, 100, -50_000), 100_000);
        // A zero maximum or a guarded weight stores 0xFFFFFFFF before scaling.
        assert_eq!(movement_factor(50, 0, 1), 100_000 - 42_949);
        assert_eq!(movement_factor(i64::MAX, 100, 1), 100_000 - 42_949);
        assert_eq!(movement_factor(i64::MIN + 1, 100, 0), 100_000);
    }

    fn strongest(weights: &[i64]) -> Option<(usize, i64)> {
        let mut strongest = Strongest::new();
        for (index, weight) in weights.iter().enumerate() {
            strongest.offer(index, *weight);
        }
        strongest.winner()
    }

    #[test]
    fn the_first_strict_positive_maximum_wins_and_must_reach_the_threshold() {
        // Ties keep the first; zero and negative weights never win.
        assert_eq!(strongest(&[0, 5, 9, 9, -3]), Some((2, 9)));
        assert_eq!(strongest(&[0, -1, i64::MIN]), None);
        assert_eq!(strongest(&[]), None);
        // Half of a maximum of 100 is 50; 49.99999 is below it.
        assert!(shows(5_000_000, 10_000_000, 50_000));
        assert!(!shows(4_999_999, 10_000_000, 50_000));
        // The product wraps and the quotient truncates toward zero.
        assert!(shows(-1, -150_001, 1));
        assert!(!shows(-2, -150_001, 1));
        assert!(shows(0, i64::MAX, 2));
        assert_eq!(
            camouflage([(1, 75), (2, 75)], 100, 75_000),
            Some(1),
            "a tie keeps the first category"
        );
        assert_eq!(camouflage([(1, 74)], 100, 75_000), None);
        assert_eq!(camouflage([(1, 0), (2, -5)], 0, 0), None);
        assert_eq!(camouflage::<u32>([], 0, 0), None);
        // A later, weaker entry above the threshold cannot win.
        assert_eq!(
            camouflage([(1, 10), (2, 90), (3, 80)], 100, 75_000),
            Some(2)
        );
        assert_eq!(camouflage([(1, 10), (2, 70), (3, 69)], 100, 75_000), None);
    }
}
