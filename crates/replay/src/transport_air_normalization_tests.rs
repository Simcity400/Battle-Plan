use super::*;
use crate::rows::{
    TransportAirCombatModifier as CombatModifier, TransportAirEnvironment as Environment,
};

fn defines() -> Defines {
    Defines {
        max_attack: P,
        max_defence: P,
        max_speed: P,
        ace_wing_size: 100 * P,
        ace_maximum: 10 * P,
        night_threshold: 50_000,
    }
}
fn modifiers() -> Modifiers {
    Modifiers {
        direct: [Some(0); 2],
        positive: [Some(0); 2],
        effective: [Some(0); 16],
    }
}
fn source() -> Source {
    Source {
        attack: Some(P),
        defence: Some(P),
        speed: Some(P),
        wing: Some(modifiers()),
        country: Some(modifiers()),
        ace: Some(None),
        environment: Some(None),
        ..Default::default()
    }
}
#[test]
fn current_mission_and_day_night_carrier_inputs_are_separate_and_lazy() {
    let mut source = source();
    source.enabled_missions = 8;
    source.country.as_mut().unwrap().effective[2] = Some(10_000); // Current interception4.
    source.country.as_mut().unwrap().effective[8] = Some(20_000); // Home region.
    source.country.as_mut().unwrap().effective[11] = Some(-20_000); // Enabled bomber8.
    source.country.as_mut().unwrap().effective[14] = Some(10_000); // Carrier task.
    source.environment = Some(Some(Environment {
        weather: [Some(-10_000); 2],
        night_factor: Some(50_000),
        night: [None; 2],
        target_region: Some(7),
    }));
    source.base_region = Some(7);
    assert_eq!(combat(&source, 4, defines()), [Ok(120_000), Ok(120_000)]);
    source
        .environment
        .as_mut()
        .unwrap()
        .as_mut()
        .unwrap()
        .night_factor = Some(50_001);
    assert!(combat(&source, 4, defines())[0].is_err());
    source.night_stat = Some(-10_000);
    source.carrier_mission = Some(Some(-1));
    source.environment.as_mut().unwrap().as_mut().unwrap().night = [Some(-10_000); 2];
    assert_eq!(combat(&source, 4, defines()), [Ok(114_000), Ok(114_000)]);
}
#[test]
fn ordered_ace_rounding_and_type_occurrences_cannot_be_replaced_with_a_sum() {
    let mut source = source();
    let mut kind = modifiers();
    kind.positive = [Some(2), Some(0)];
    source.country.as_mut().unwrap().effective[7] = Some(1);
    source.ace = Some(Some(Ace {
        assigned_aircraft: Some(Some(3)),
        modifiers: Some(modifiers()),
        type_modifiers: Some(kind),
        owner_effectiveness: Some(0),
        combat: Some(vec![
            CombatModifier {
                lane: 0,
                raw: Some(2),
            },
            CombatModifier {
                lane: 0,
                raw: Some(2),
            },
        ]),
    }));
    let mut defines = defines();
    defines.ace_wing_size = P;
    defines.ace_maximum = P;
    // Each raw2 * scale33333 truncates to0. Summing raw4 first would yield1.
    assert_eq!(combat(&source, 4, defines), [Ok(100_002), Ok(P)]);
}
#[test]
fn independent_defence_and_speed_skip_unrelated_combat_and_environment_faults() {
    let mut source = source();
    source.attack = None;
    assert_eq!(
        combat(&source, 4, defines()),
        [
            Err("transport original current-mission combat stat refused"),
            Ok(P)
        ]
    );
    source.attack = Some(P);
    source.wing.as_mut().unwrap().direct[0] = None;
    assert_eq!(combat(&source, 4, defines())[1], Ok(P));
    source.ace = Some(Some(Ace {
        assigned_aircraft: Some(None),
        modifiers: Some(modifiers()),
        type_modifiers: Some(modifiers()),
        owner_effectiveness: Some(0),
        combat: Some(vec![CombatModifier { lane: 0, raw: None }]),
    }));
    assert_eq!(combat(&source, 4, defines())[1], Ok(P));
    source.ace = Some(None);
    source.environment = None;
    source.country = None;
    assert_eq!(speed(&source, defines()), Ok(P));
    source.speed = Some(i64::MAX);
    let mut defines = defines();
    defines.max_speed = 0;
    assert_eq!(speed(&source, defines), Ok(SENTINEL));
}
#[test]
fn native_signed_scale_and_speed_use_default_stat_and_effectiveness() {
    let mut source = source();
    let mut ace_mods = modifiers();
    ace_mods.effective[0] = Some(10_000);
    source.ace = Some(Some(Ace {
        assigned_aircraft: Some(Some(-2)),
        modifiers: Some(ace_mods),
        type_modifiers: Some(modifiers()),
        owner_effectiveness: Some(P),
        combat: Some(vec![]),
    }));
    let mut defines = defines();
    defines.ace_wing_size = 2 * P;
    assert_eq!(speed(&source, defines), Ok(80_000));
    assert_eq!(
        ace_scale(source.ace.as_ref().unwrap().as_ref().unwrap(), defines),
        Ok(-P)
    );
}
