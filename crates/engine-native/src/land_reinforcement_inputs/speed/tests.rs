use super::*;

/// One reserve's operands, owned so a test can edit them.
#[derive(Default)]
struct Owned {
    context: ContextRow,
    modifiers: Vec<ModifierTerm>,
    supply: Vec<SupplyTerm>,
    acclimatization: Vec<AcclimatizationTerm>,
    transport: Vec<TransportTerm>,
}

impl Owned {
    /// A supplied land reserve with every country movement key present at 0.
    fn normal() -> Self {
        let mut owned = Self {
            modifiers: keys(0, &[0x2b, 0x1e9, 0x5e, 0x17f, 0x286]),
            ..Self::default()
        };
        owned.context.ids[0] = 1;
        owned.context.flags[2] = 1;
        owned
    }

    fn factor(&self) -> Speed {
        movement_factor(&Terms {
            context: &self.context,
            modifiers: &self.modifiers,
            supply: &self.supply,
            acclimatization: &self.acclimatization,
            transport: &self.transport,
        })
    }

    fn set(&mut self, scope: u8, key: u32, value: i64) {
        let term = self
            .modifiers
            .iter_mut()
            .find(|term| term.scope == scope && term.key == key)
            .unwrap();
        term.raw = value;
        term.present = 1;
    }
}

fn keys(scope: u8, keys: &[u32]) -> Vec<ModifierTerm> {
    keys.iter()
        .map(|&key| ModifierTerm {
            scope,
            key,
            ..Default::default()
        })
        .collect()
}

#[test]
fn absent_optional_sources_are_zero_but_partial_scopes_refuse() {
    let mut owned = Owned::normal();
    assert_eq!(owned.factor(), Ok(ONE));
    owned.modifiers.extend(keys(3, &[0x2b]));
    assert_eq!(
        owned.factor(),
        Err("a reinforcement modifier scope holds keys outside the movement-speed calculation")
    );
    assert!(Owned::default().factor().is_err());
}

#[test]
fn feature_gating_and_metadata_clamps_apply_before_land_addition() {
    let mut owned = Owned::normal();
    owned.set(0, 0x2b, 150_000);
    owned.modifiers[0].metadata_flags = 0x10;
    owned.modifiers[0].feature_selector = 8;
    // No eligibility callback: a feature-gated key is zero.
    assert_eq!(owned.factor(), Ok(ONE));
    owned.context.ids[6] = 1;
    owned.context.ids[5] = 8;
    // Clamped to one before it is added.
    assert_eq!(owned.factor(), Ok(2 * ONE));
    owned.context.ids[6] = 2;
    assert!(owned.factor().is_err());
}

#[test]
fn weather_takes_the_first_maximum_and_scales_each_term_separately() {
    let mut owned = Owned::normal();
    owned.modifiers.extend(keys(3, &[0x2b, 0x1e9]));
    owned.context.flags[4] = 1;
    owned.context.scalar_values[0] = ONE;
    owned.context.scalar_values[1] = ONE / 2;
    owned.acclimatization = vec![
        AcclimatizationTerm {
            weight: ONE,
            active: 0,
        },
        AcclimatizationTerm {
            weight: ONE,
            active: 1,
        },
    ];
    owned.set(3, 0x2b, -10_000);
    // The inactive first maximum wins: no adjustment.
    assert_eq!(owned.factor(), Ok(90_000));
    owned.acclimatization.swap(0, 1);
    // The active winner halves the penalty.
    assert_eq!(owned.factor(), Ok(95_000));
    owned.context.flags[5] = 1;
    owned.set(3, 0x2b, 1);
    owned.set(3, 0x1e9, 1);
    // Each 0.5 x 0.00001 truncates to zero; combining first would give 1.
    assert_eq!(owned.factor(), Ok(ONE));
}

#[test]
fn supply_days_are_not_clamped_and_the_special_share_adjusts_the_factor() {
    let mut owned = Owned::normal();
    owned.context.flags[2] = 0;
    owned.context.flags[11] = 1;
    owned.context.flags[8] = 1;
    let s = &mut owned.context.scalar_values;
    s[2] = ONE;
    s[4] = ONE / 2;
    s[5] = 10;
    s[6] = 24;
    s[7] = 5;
    s[8] = -50_000;
    s[9] = -10_000;
    owned.supply = vec![
        SupplyTerm {
            subunit: 1,
            count: 2,
            special: 0,
        },
        SupplyTerm {
            subunit: 2,
            count: 2,
            special: 1,
        },
    ];
    owned.set(0, 0x5e, -20_000);
    owned.set(0, 0x17f, -40_000);
    // Severity (1 - 0.5) x 10/5 = 1, scaled by 1 - 0.2 + 0.5 x -0.4 = 0.6:
    // -0.5 x 0.6 - 0.1.
    assert_eq!(owned.factor(), Ok(60_000));
    // A zero grace define gives the sentinel, which clamps supplied to one.
    owned.context.scalar_values[6] = 0;
    assert_eq!(owned.factor(), Ok(90_000));
    owned.context.flags[11] = 0;
    assert!(owned.factor().is_err());
}

#[test]
fn infrastructure_uses_the_healthy_level_and_retreat_takes_precedence() {
    let mut owned = Owned::normal();
    owned.context.ids[3] = 9;
    owned.context.flags[9] = 1;
    owned.context.flags[10] = 1;
    let s = &mut owned.context.scalar_values;
    s[20] = 10;
    s[23] = -10_000;
    s[24] = 8;
    assert_eq!(owned.factor(), Ok(80_000));
    owned.context.flags[0] = 1;
    owned.context.flags[1] = 1;
    owned.context.scalar_values[11] = 30_000;
    owned.context.scalar_values[12] = 80_000;
    owned.set(0, 0x286, 20_000);
    assert_eq!(owned.factor(), Ok(130_000));
}

#[test]
fn air_defence_can_cancel_but_cannot_reverse_the_air_penalty() {
    let mut owned = Owned::normal();
    owned.context.flags[7] = 1;
    let s = &mut owned.context.scalar_values;
    s[13] = ONE;
    s[14] = ONE;
    s[15] = ONE;
    s[16] = ONE;
    s[17] = -50_000;
    s[18] = ONE;
    s[19] = 20_000;
    assert_eq!(owned.factor(), Ok(85_000));
    owned.context.scalar_values[19] = ONE;
    assert_eq!(owned.factor(), Ok(ONE));
}

#[test]
fn transport_scales_the_raw_static_term_before_clamping_and_bypasses_land_inputs() {
    let mut owned = Owned::default();
    owned.context.ids[0] = 1;
    owned.context.ids[5] = 1 << 4;
    owned.context.flags[11] = 2;
    owned.modifiers.extend(keys(8, &[0x29]));
    owned.modifiers.extend(keys(9, &[0x1a7]));
    owned.set(8, 0x29, 200_000);
    owned.modifiers[0].metadata_flags = 0x10;
    let s = &mut owned.context.scalar_values;
    s[25] = ONE;
    s[27] = ONE;
    s[29] = 1;
    owned.transport.push(TransportTerm {
        country: 1,
        regional_value: 50_000,
        own_intel: 0,
        enemy_intel: 0,
    });
    // 0.5 density x 2.0 raw = 1.0, then clamped by the metadata bit.
    assert_eq!(owned.factor(), Ok(200_000));
    owned.modifiers.pop();
    assert!(owned.factor().is_err());
    owned.modifiers.clear();
    assert!(owned.factor().is_err());
}

/// The reinforcement share's final scaling: the share of base speed times
/// factor over SLOWEST_SPEED above one, clamped to [0, 2]; a zero define or an
/// exceptional speed takes the upper bound.
#[test]
fn the_speed_share_clamps_to_the_native_bounds() {
    // 4 km/h at factor 1 over a 4 km/h floor: no excess.
    assert_eq!(speed_share(400_000, ONE, 400_000), Ok(0));
    // 6 km/h: half again.
    assert_eq!(speed_share(600_000, ONE, 400_000), Ok(50_000));
    // 0.9 x 6 = 5.4 km/h over 4: 0.35.
    assert_eq!(speed_share(600_000, 90_000, 400_000), Ok(35_000));
    // Slower than the floor clamps to zero, faster than three times to two.
    assert_eq!(speed_share(200_000, ONE, 400_000), Ok(0));
    assert_eq!(speed_share(2_000_000, ONE, 400_000), Ok(SHARE_MAXIMUM));
    assert_eq!(speed_share(600_000, ONE, 0), Ok(SHARE_MAXIMUM));
    // The product truncates before the division: 0.00001 x 1.5 is 0.00001,
    // which equals the floor; 0.00002 x 1.5 is three times it.
    assert_eq!(speed_share(1, 150_000, 1), Ok(0));
    assert_eq!(speed_share(2, 150_000, 1), Ok(SHARE_MAXIMUM));
    // A negative define divides as native signed division does.
    assert_eq!(speed_share(600_000, ONE, -400_000), Ok(0));
}
