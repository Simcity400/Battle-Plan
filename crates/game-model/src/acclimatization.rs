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

    pub fn winner(&self) -> Option<(C, i64)> {
        self.winner
    }
}

pub fn shows(weight: i64, maximum: i64, show_at: i64) -> bool {
    weight >= maximum.wrapping_mul(show_at) / 100_000
}

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
        assert_eq!(movement_factor(300, 100, 100_000), 0);
        assert_eq!(movement_factor(50, 100, -50_000), 100_000);
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
        assert_eq!(strongest(&[0, 5, 9, 9, -3]), Some((2, 9)));
        assert_eq!(strongest(&[0, -1, i64::MIN]), None);
        assert_eq!(strongest(&[]), None);
        assert!(shows(5_000_000, 10_000_000, 50_000));
        assert!(!shows(4_999_999, 10_000_000, 50_000));
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
        assert_eq!(
            camouflage([(1, 10), (2, 90), (3, 80)], 100, 75_000),
            Some(2)
        );
        assert_eq!(camouflage([(1, 10), (2, 70), (3, 69)], 100, 75_000), None);
    }
}
