use crate::fixed_point::{fixed5_to_q15, q15_to_fixed5};
pub use game_model::war_participation::{DeliveryHistory, Scores};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Factors {
    pub industry_given: i64,
    pub fuel_given: i64,
    pub industry_received: i64,
    pub fuel_received: i64,
}
fn fuel_score(fuel: i64, factor: i64) -> i64 {
    let units = (fuel / 100_000) as i32;
    let weighted = i64::from(units).wrapping_mul(factor).wrapping_mul(100_000) / 100_000;
    if (weighted as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
        weighted.wrapping_mul(100_000) / 10_000_000
    } else {
        0xffff_ffff
    }
}

pub fn breakdown(
    ledgers: impl IntoIterator<Item = Scores>,
    histories: impl IntoIterator<Item = DeliveryHistory>,
    factors: Factors,
) -> Scores {
    let mut result = Scores::default();
    for ledger in ledgers {
        result.add(ledger);
    }
    let mut history = DeliveryHistory::default();
    for h in histories {
        history.industry_sent = history.industry_sent.wrapping_add(h.industry_sent);
        history.fuel_sent = history.fuel_sent.wrapping_add(h.fuel_sent);
        history.industry_received = history.industry_received.wrapping_add(h.industry_received);
        history.fuel_received = history.fuel_received.wrapping_add(h.fuel_received);
    }
    result.lend_lease_sent = result
        .lend_lease_sent
        .wrapping_add(history.industry_sent.wrapping_mul(factors.industry_given) / 100_000)
        .wrapping_add(fuel_score(history.fuel_sent, factors.fuel_given));
    result.lend_lease_received = result
        .lend_lease_received
        .wrapping_sub(
            history
                .industry_received
                .wrapping_mul(factors.industry_received)
                / 100_000,
        )
        .wrapping_sub(fuel_score(history.fuel_received, factors.fuel_received));
    result
}

pub fn share(selected: i64, scores: impl IntoIterator<Item = i64>) -> i64 {
    share_of(selected, share_denominator(scores))
}

pub fn share_denominator(scores: impl IntoIterator<Item = i64>) -> i64 {
    scores
        .into_iter()
        .fold(0_i64, |sum, score| sum.wrapping_add(fixed5_to_q15(score)))
}

pub fn share_of(selected: i64, denominator: i64) -> i64 {
    let ratio = if denominator == 0 {
        0
    } else {
        fixed5_to_q15(selected)
            .wrapping_shl(15)
            .wrapping_div(denominator)
    };
    q15_to_fixed5(ratio)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn participation_share_quantizes_each_score_before_summing() {
        assert_eq!(share(1, [1, 4]), 0);
        assert_ne!(share(1, [1, 4]), 100_000 / 5);
        assert_eq!(share(100_000, [100_000, 100_000]), 50_000);
        assert_eq!(share(0, [0, 0]), 0);
    }

    #[test]
    fn the_side_denominator_can_be_shared_between_contributors() {
        for (scores, expected) in [
            ([0, 0, 0], [0, 0, 0]),
            ([1, 4, 0], [0, 100_000, 0]),
            ([100_000, 100_000, 0], [50_000, 50_000, 0]),
        ] {
            let denominator = share_denominator(scores);
            for (selected, expected) in scores.into_iter().zip(expected) {
                assert_eq!(share_of(selected, denominator), expected);
                assert_eq!(share(selected, scores), expected);
            }
        }
    }
    #[test]
    fn participation_breakdown_sums_fuel_before_integer_truncation_and_keeps_native_maximum() {
        let factors = Factors {
            industry_given: 7,
            fuel_given: 300,
            industry_received: 11,
            fuel_received: 200,
        };
        let scores = breakdown(
            [
                Scores {
                    equipment_damage: 15,
                    unknown_casualties: 8,
                    lend_lease_sent: 2,
                    ..Default::default()
                },
                Scores {
                    equipment_damage: -3,
                    unknown_casualties: 5,
                    ..Default::default()
                },
            ],
            [DeliveryHistory {
                industry_sent: 100_000,
                fuel_sent: 50_000,
                fuel_received: 50_000,
                ..Default::default()
            }; 2],
            factors,
        );
        assert_eq!(scores.equipment_damage, 12);
        assert_eq!(scores.unknown_casualties, 8);
        assert_eq!(scores.lend_lease_sent, 19);
        assert_eq!(scores.lend_lease_received, -2);
        assert_eq!(scores.total(false), 37);
    }
}
