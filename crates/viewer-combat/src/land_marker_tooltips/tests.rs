use super::*;
use viewer_foundation::tooltip_text::NoIcons;

fn loc() -> Localisation {
    [
        (
            LAND_COMBAT_TOOLTIP_ATTACKER,
            "Attacker: $COUNT$ ($COUNTRIES$)",
        ),
        (
            LAND_COMBAT_TOOLTIP_DEFENDER,
            "Defender: $COUNT$ ($COUNTRIES$)",
        ),
        (COMBAT_WE_ARE_WINNING, "We win"),
        (COMBAT_WE_ARE_LOSING, "We lose"),
        (COMBAT_ATTACKER_WINNING, "Attacker wins"),
        (COMBAT_DEFENDER_WINNING, "Defender wins"),
        (COMBAT_UNDECIDED, "Undecided"),
        (COMBAT_LAST_LESS_THAN_DAY, "Less than a day"),
        (COMBAT_LAST_MORE_THAN_DAY, "$DAYS$ days"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}

#[test]
fn marker_uses_full_roster_primary_order_and_viewer_relative_survival() {
    let combat = LandCombatRow {
        attackers: vec![10, 11, 12],
        defenders: vec![20, 21],
        ..Default::default()
    };
    let sides = LandCombatSideRow {
        attacker_primary: vec![7, 3],
        attacker_secondary: vec![9],
        defender_primary: vec![8],
        ..Default::default()
    };
    let name = |id| Ok(format!("Country {id}"));
    assert_eq!(
        seen(
            &NoIcons,
            &loc(),
            &combat,
            &sides,
            [1_000_000, 100_000],
            [false, true],
            &name
        )
        .unwrap(),
        "\nAttacker: 3 (\u{11}YCountry 7\u{11}!, \u{11}YCountry 3\u{11}!)\nDefender: 2 (\u{11}YCountry 8\u{11}!)\nWe lose\nLess than a day"
    );
    assert_eq!(
        countries(&[7, 3, 7], &name).unwrap(),
        "\u{11}YCountry 7\u{11}!, \u{11}YCountry 3\u{11}!, \u{11}YCountry 7\u{11}!"
    ); // No second set builder here.
}

#[test]
fn winning_threshold_rounding_neutral_and_exceptional_branches_are_native() {
    assert_eq!(
        outcome([120_000, 100_000], [true, false]).0,
        COMBAT_UNDECIDED
    );
    assert_eq!(
        outcome([120_001, 100_000], [false, false]).0,
        COMBAT_ATTACKER_WINNING
    );
    assert_eq!(
        outcome([100_000, 120_001], [false, false]).0,
        COMBAT_DEFENDER_WINNING
    );
    assert_eq!(
        outcome([100_000, 120_001], [false, true]).0,
        COMBAT_WE_ARE_WINNING
    );
    assert_eq!(
        outcome([120_001, 100_000], [true, true]).0,
        COMBAT_WE_ARE_WINNING
    );
    assert_eq!(outcome([3_599_999, 3_599_999], [false, false]).1, 1);
    assert_eq!(outcome([3_600_000, 3_600_000], [false, false]).1, 2);
    assert_eq!(outcome([i64::MAX, i64::MAX], [true, false]).1, 42_950);
    assert_eq!(outcome([-3_600_000, -3_600_000], [true, false]).1, -2);
}

#[test]
fn marker_hit_precedes_navigation_and_no_intel_uses_native_key() {
    let hits = [
        super::super::Hit {
            selection: super::super::Selection::Land(17),
            rect: [1., 2., 3., 4.],
            layer: 0,
            order: 0,
            supported: true,
        },
        super::super::Hit {
            selection: super::super::Selection::Land(19),
            rect: [1., 2., 3., 4.],
            layer: 0,
            order: 1,
            supported: true,
        },
    ];
    assert_eq!(hit(&hits, [2., 3.]), Some((19, [1., 2., 3., 4.])));
    assert_eq!(hit(&hits, [4., 3.]), None);
    let loc: Localisation = [(
        LAND_COMBAT_TOOLTIP_NO_INTEL.into(),
        "Synthetic no intel".into(),
    )]
    .into_iter()
    .collect();
    assert_eq!(
        intel(&NoIcons, &loc, Engaged::No),
        Ok(Some("Synthetic no intel".into()))
    );
    assert_eq!(intel(&NoIcons, &loc, Engaged::Yes), Ok(None));
    assert!(intel(&NoIcons, &loc, Engaged::Unknown).is_err());
}

#[test]
fn we_uses_secondary_membership_and_canonical_equivalence_not_shared_vision() {
    let sides = LandCombatSideRow {
        attacker_primary: vec![7],
        attacker_secondary: vec![9],
        defender_primary: vec![8],
        ..Default::default()
    };
    let links = [(7, 70), (9, 90), (8, 80), (99, 90)].into_iter().collect();
    assert_eq!(membership(&sides, 90, &links), Ok([true, false]));
    assert_eq!(membership(&sides, 80, &links), Ok([false, true]));
    assert_eq!(membership(&sides, 99, &links), Ok([false, false]));
    assert!(membership(&sides, 1, &BTreeMap::new()).is_err());
}

#[test]
fn marker_country_list_and_roster_have_no_viewer_cap() {
    let combat = LandCombatRow {
        attackers: (0..5_000).collect(),
        ..Default::default()
    };
    let sides = LandCombatSideRow {
        attacker_primary: (0..5_000).collect(),
        ..Default::default()
    };
    let names = countries(&sides.attacker_primary, &|id| Ok(format!("C{id}"))).unwrap();
    assert!(names.ends_with("\u{11}YC4999\u{11}!"));
    let text = seen(
        &NoIcons,
        &loc(),
        &combat,
        &sides,
        [0, 0],
        [false, false],
        &|id| Ok(format!("C{id}")),
    )
    .unwrap();
    assert!(text.starts_with("\nAttacker: 5000 (\u{11}YC0\u{11}!, \u{11}YC1\u{11}!"));
    // The shared native localiser limits each substitution to its own
    // 0x1800-byte buffer; this is not a country-list cap.
    assert!(text.contains("\nDefender: 0 ()"));
}
