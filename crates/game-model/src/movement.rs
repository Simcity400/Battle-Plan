
pub mod army {

    #[doc(hidden)]
    pub const MOVEMENT_ONE: i64 = 100_000;

    fn movement_product(a: i64, b: i64) -> i64 {
        a.wrapping_mul(b) / MOVEMENT_ONE
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[expect(
        clippy::large_enum_variant,
        reason = "capture keeps owned movement operands inline"
    )]
    pub enum MovementSpeedInputs {
        Fixed(i64),
        Scaled {
            speed: i64,
            scale: i64,
        },
        Normal {
            common: NormalSpeedOperands,
            edge: NormalEdgeOperands,
        },
    }
    impl Default for MovementSpeedInputs {
        fn default() -> Self {
            Self::Fixed(0)
        }
    }
    impl From<i64> for MovementSpeedInputs {
        fn from(value: i64) -> Self {
            Self::Fixed(value)
        }
    }
    impl MovementSpeedInputs {
        pub fn evaluate(self) -> i64 {
            match self {
                Self::Fixed(value) => value,
                Self::Scaled { speed, scale } => movement_product(speed, scale),
                Self::Normal { common, edge } => common.prepare().evaluate(edge),
            }
        }
        pub fn is_zero(self) -> bool {
            if let Self::Normal { common, edge } = self
                && common.bounded
                && edge.bounded()
            {
                return false;
            }
            self.evaluate() == 0
        }
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub struct NormalSpeedOperands {
        modifier: NormalModifierOperands,
        base_speed: i64,
        organisation: Option<(i64, i64)>,
        fuel: Option<(i64, i64)>,
        outer_scale: i64,
        bounded: bool,
    }
    impl NormalSpeedOperands {
        pub fn prepare(self) -> PreparedNormalSpeed {
            PreparedNormalSpeed {
                modifier: self.modifier.evaluate(),
                base_speed: self.base_speed,
                organisation: self.organisation.map(|(ratio, penalty)| {
                    MOVEMENT_ONE
                        .wrapping_add(movement_product(MOVEMENT_ONE.wrapping_sub(ratio), penalty))
                }),
                fuel: self.fuel.map(|(ratio, minimum)| {
                    movement_product(MOVEMENT_ONE.wrapping_sub(minimum), ratio)
                        .wrapping_add(minimum)
                }),
                outer_scale: self.outer_scale,
            }
        }
    }
    #[derive(Clone, Copy)]
    pub struct PreparedNormalSpeed {
        pub modifier: i64,
        pub base_speed: i64,
        pub organisation: Option<i64>,
        pub fuel: Option<i64>,
        pub outer_scale: i64,
    }
    impl PreparedNormalSpeed {
        pub fn evaluate(self, edge: NormalEdgeOperands) -> i64 {
            let mut speed =
                movement_product(self.base_speed, self.modifier.wrapping_add(edge.evaluate()));
            if let Some(factor) = self.organisation {
                speed = movement_product(factor, speed);
            }
            if let Some(factor) = self.fuel {
                speed = movement_product(factor, speed);
            }
            movement_product(speed.max(MOVEMENT_ONE), self.outer_scale)
        }
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    struct NormalModifierOperands {
        additive: i64,
        weather: [(i64, i64); 4],
        supply: (i64, i64),
        air: MovementAirOperands,
    }
    impl NormalModifierOperands {
        fn evaluate(self) -> i64 {
            let mut value = self.additive;
            for (acclimatization, modifier) in self.weather {
                value = value.wrapping_add(movement_product(acclimatization, modifier));
            }
            value
                .wrapping_add(movement_product(self.supply.0, self.supply.1))
                .wrapping_add(self.air.evaluate())
        }
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    struct MovementAirOperands {
        fraction: i64,
        impact: i64,
        scale: i64,
        defense: i64,
    }
    impl MovementAirOperands {
        fn evaluate(self) -> i64 {
            movement_product(movement_product(self.impact, self.fraction), self.scale)
                .wrapping_add(self.defense)
                .min(0)
        }
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub struct NormalEdgeOperands {
        pub additive: i64,
        pub river: Option<(i64, i64, i64)>,
    }
    impl NormalEdgeOperands {
        fn evaluate(self) -> i64 {
            let mut value = self.additive;
            if let Some((combined, mut adjustment, river)) = self.river {
                if adjustment < 0 {
                    adjustment = movement_product(combined, adjustment);
                }
                value = value
                    .wrapping_add(adjustment)
                    .wrapping_add(movement_product(
                        MOVEMENT_ONE.wrapping_add(combined).max(0),
                        river,
                    ));
            }
            value
        }
        fn bounded(self) -> bool {
            movement_bounded(self.additive, 100_000_000)
                && self.river.is_none_or(|(a, b, c)| {
                    [a, b, c]
                        .into_iter()
                        .all(|value| movement_bounded(value, 1_000_000))
                })
        }
    }

    fn movement_bounded(value: i64, bound: i64) -> bool {
        (-bound..=bound).contains(&value)
    }
    #[cfg(test)]
    mod tests {

        #[test]
        fn owned_normal_speed_preserves_native_products_and_safe_zero_guard() {
            use super::*;
            let mut common = NormalSpeedOperands {
                modifier: NormalModifierOperands {
                    additive: 80_000,
                    weather: [(50_000, -10_000), (0, 0), (0, 0), (0, 0)],
                    supply: (-20_000, 50_000),
                    air: MovementAirOperands {
                        fraction: 50_000,
                        impact: 100_000,
                        scale: -20_000,
                        defense: 1_000,
                    },
                },
                base_speed: 1_000_000,
                organisation: Some((50_000, -40_000)),
                fuel: Some((50_000, 20_000)),
                outer_scale: 50_000,
                bounded: true,
            };
            let edge = NormalEdgeOperands {
                additive: 10_000,
                river: Some((50_000, -10_000, -20_000)),
            };
            let inputs = MovementSpeedInputs::Normal { common, edge };
            assert_eq!(inputs.evaluate(), 74_400);
            assert!(edge.bounded());
            assert!(!inputs.is_zero());
            for scale in [0, i64::MIN] {
                common.outer_scale = scale;
                common.bounded = false;
                let inputs = MovementSpeedInputs::Normal { common, edge };
                assert_eq!(inputs.evaluate(), 0);
                assert_eq!(inputs.is_zero(), inputs.evaluate() == 0);
            }
        }

        #[test]
        fn owned_normal_nonzero_proof_covers_extreme_allowed_operands() {
            use super::*;
            for sign in [-1, 1] {
                for base_speed in [-1_000_000, 0, 1_000_000] {
                    for scale in [1, 100_000] {
                        for organisation in [
                            None,
                            Some((0, -100_000)),
                            Some((100_000, 0)),
                            Some((50_000, -40_000)),
                        ] {
                            for fuel in [None, Some((100_000, 100_000)), Some((50_000, 20_000))] {
                                let mut common = NormalSpeedOperands {
                                    modifier: NormalModifierOperands {
                                        additive: sign * 100_000_000,
                                        weather: [(sign * 100_000, 1_000_000); 4],
                                        supply: (sign * 1_000_000, 1_000_000),
                                        air: MovementAirOperands {
                                            fraction: sign * 100_000,
                                            impact: 1_000_000,
                                            scale: 1_000_000,
                                            defense: sign * 1_000_000,
                                        },
                                    },
                                    base_speed,
                                    organisation,
                                    fuel,
                                    outer_scale: scale,
                                    bounded: true,
                                };
                                let edge = NormalEdgeOperands {
                                    additive: sign * 100_000_000,
                                    river: Some((sign * 1_000_000, -1_000_000, sign * 1_000_000)),
                                };
                                assert!(edge.bounded());
                                let inputs = MovementSpeedInputs::Normal { common, edge };
                                assert!(!inputs.is_zero());
                                assert!(inputs.evaluate() > 0);
                                common.modifier.additive = i64::MAX;
                                common.bounded = false;
                                let inputs = MovementSpeedInputs::Normal { common, edge };
                                assert_eq!(inputs.is_zero(), inputs.evaluate() == 0);
                            }
                        }
                    }
                }
            }
        }

        #[test]
        fn native_speed_products_do_not_cancel_scale_or_use_wider_multiplication() {
            assert_eq!(super::movement_product(100_001, -50_001), -50_001);
            assert_eq!(super::movement_product(i64::MAX, 100_000), -1);
            assert_eq!(super::movement_product(i64::MIN, -1), i64::MIN / 100_000);
        }
    }
}

pub mod unit {

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub struct EtaLeg {
        pub distance: i64,
        pub speed: EtaSpeed,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum EtaSpeed {
        Fixed(i64),
        Scaled { speed: i64, scale: i64 },
        Normal(super::army::NormalEdgeOperands),
    }
    impl Default for EtaSpeed {
        fn default() -> Self {
            Self::Fixed(0)
        }
    }
    impl From<i64> for EtaSpeed {
        fn from(value: i64) -> Self {
            Self::Fixed(value)
        }
    }
    impl EtaSpeed {
        fn evaluate(
            self,
            normal: Option<super::army::PreparedNormalSpeed>,
        ) -> Result<i64, &'static str> {
            Ok(match self {
                Self::Fixed(value) => value,
                Self::Scaled { speed, scale } => {
                    super::army::MovementSpeedInputs::Scaled { speed, scale }.evaluate()
                }
                Self::Normal(edge) => normal
                    .ok_or("ETA normal speed inputs missing")?
                    .evaluate(edge),
            })
        }
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub struct EtaInputs {
        pub division: u64,
        pub next: u32,
        pub destination: u32,
        pub path_total: u32,
        pub leg_start: usize,
        pub leg_len: usize,
        pub next_leg_count: usize,
        pub numerator: i64,
        pub current_speed: EtaSpeed,
        pub normal: Option<super::army::NormalSpeedOperands>,
    }

    pub fn eta_ratio(value: i64, divisor: i64) -> Result<i64, &'static str> {
        if divisor == 0
            || (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
        {
            return Ok(0xffff_ffff);
        }
        value
            .wrapping_mul(100_000)
            .checked_div(divisor)
            .ok_or("ETA native signed division overflows")
    }

    fn eta_route_duration(
        legs: &[EtaLeg],
        normal: Option<super::army::PreparedNormalSpeed>,
        progress: impl FnOnce() -> Result<i64, &'static str>,
    ) -> Result<i64, &'static str> {
        let mut duration = 0_i64;
        for leg in legs {
            let speed = leg.speed.evaluate(normal)?;
            if speed == 0 {
                return Ok(0);
            }
            duration = duration.wrapping_add(eta_ratio(leg.distance, speed)?);
        }
        Ok(duration.wrapping_sub(progress()?))
    }

    pub fn eta_remaining(input: &EtaInputs, legs: &[EtaLeg]) -> Result<(i64, i64), &'static str> {
        if input.next_leg_count > 1
            || input.next_leg_count > input.leg_len
            || input.leg_len > input.path_total as usize
        {
            return Err("ETA owned leg counts invalid");
        }
        let end = input
            .leg_start
            .checked_add(input.leg_len)
            .ok_or("ETA owned leg range overflow")?;
        let legs = legs
            .get(input.leg_start..end)
            .ok_or("ETA owned leg range invalid")?;
        if input.path_total == 0 {
            return Ok((0, 0));
        }
        if input.next == 0 || input.destination == 0 {
            return Err("ETA owned route endpoints missing");
        }
        let normal = input.normal.map(super::army::NormalSpeedOperands::prepare);
        let progress = std::cell::OnceCell::new();
        let evaluate = |legs: &[EtaLeg]| {
            eta_route_duration(legs, normal, || {
                *progress.get_or_init(|| {
                    input
                        .current_speed
                        .evaluate(normal)
                        .and_then(|speed| eta_ratio(input.numerator, speed))
                })
            })
        };
        Ok((evaluate(&legs[..input.next_leg_count])?, evaluate(legs)?))
    }

    pub fn eta_hours(duration: i64) -> i64 {
        let quotient = duration / 100_000;
        if duration == quotient * 100_000 {
            return quotient;
        }
        quotient
            .wrapping_add(i64::from(duration > 0))
            .wrapping_mul(100_000)
            / 100_000
    }
}
