use super::super::tests::{context, parsed};
use super::*;
use viewer_foundation::tooltip_text::NoIcons;

fn localisation() -> Localisation {
    [
        ("HEADER", "$KEY$\n"),
        ("attack", "Attack"),
        ("defend", "Defend"),
        ("other", "Other"),
        ("TACTICS_DAMAGE_ATTACKER", "Attacker:"),
        ("TACTICS_DAMAGE_DEFENDER", "Defender:"),
        ("TACTICS_VALUE_POSITIVE", "good $VALUE$"),
        ("TACTICS_VALUE_NEGATIVE", "bad $VALUE$"),
        ("TACTIC_COUNTERED_BY", "Countered by $NAME$"),
        ("TACTIC_COUNTERS", "Counters:"),
        ("TACTICS_LOCKED_BY_TECH", "Sources:$TECH$"),
        ("TACTICS_LOCKED_BY_NOT_HANDLED", "No doctrine source"),
        ("TACTICS_ACTIVATION_REQUIREMENTS", "Requirements:"),
        ("TACTIC_CONDITIONS_NOT_MET", "Conditions failed\n"),
        ("TACTIC_CHANCE_OF_SELECTION", "Chance:$CHANCE$\n"),
        ("TACTIC_TOTAL_WEIGHT", "Total:$WEIGHT$\n"),
        ("TACTIC_WEIGHT_OF_TACTIC", "Weight:$WEIGHT$\n"),
        ("TACTICS_BASE_WEIGHT", "Base:$WEIGHT$\n"),
        ("TACTICS_SCOPE_WEIGHT", "Scoped:$WEIGHT$\n"),
        (
            "TACTIC_WEIGHT_MODIFIER_PREFERRED_TACTIC",
            "Country:$MODIFIER$\n",
        ),
        (
            "TACTIC_WEIGHT_MODIFIER_ARMY_GENERAL_PREFERRED_TACTIC",
            "General:$MODIFIER$\n",
        ),
        (
            "TACTIC_WEIGHT_MODIFIER_FIELD_MARSHAL_PREFERRED_TACTIC",
            "Marshal:$MODIFIER$\n",
        ),
        (
            "TACTIC_CHANCE_CALCULATION",
            "$WEIGHT$/$TOTAL_WEIGHT$=$CHANCE$",
        ),
    ]
    .into_iter()
    .map(|(key, text)| (key.into(), text.into()))
    .collect()
}

fn candidate(id: i32, name: &str) -> Candidate {
    Candidate {
        tactic_id: id,
        token: name.as_bytes().to_vec(),
        ..Default::default()
    }
}

#[test]
fn possible_tactic_main_uses_authored_polarity_and_ordered_counter_names() {
    let content = parsed(
        b"attack={is_attacker=yes attacker=0.1 countered_by=defend}
        defend={is_attacker=no defender=0.2}
        other={countered_by=defend}",
    );
    let loc = localisation();
    let attack = candidate(1, "attack");
    assert_eq!(
        main(
            &NoIcons,
            &loc,
            &content,
            &attack,
            content.definition(&attack).unwrap(),
            &mut BTreeSet::new(),
        )
        .unwrap(),
        "Attack\nAttacker: good 0.10\n\nCountered by Defend\n"
    );
    let defend = candidate(2, "defend");
    assert_eq!(
        main(
            &NoIcons,
            &loc,
            &content,
            &defend,
            content.definition(&defend).unwrap(),
            &mut BTreeSet::new(),
        )
        .unwrap(),
        "Defend\nDefender: good 0.20\n\nCounters:\n\u{11}Y  Attack\u{11}!\n\u{11}Y  Other\u{11}!\n"
    );
    let duplicate = parsed(b"attack={countered_by=defend} defend={} defend={}");
    assert_eq!(
        counter_id(
            &duplicate,
            duplicate.registry[0].definition.as_ref().unwrap()
        ),
        Ok(3)
    );
    let missing = parsed(b"attack={countered_by=absent}");
    assert_eq!(
        counter_id(&missing, missing.registry[0].definition.as_ref().unwrap()),
        Ok(0)
    );
}

#[test]
fn possible_tactic_locked_keeps_source_order_duplicates_and_native_newlines() {
    assert_eq!(
        locked(
            &NoIcons,
            &localisation(),
            &["First".into(), "Second".into(), "First".into()]
        ),
        Ok("Sources:\nFirst\nSecond\nFirst\n".into())
    );
    assert_eq!(
        locked(&NoIcons, &localisation(), &[]),
        Ok("No doctrine source\n".into())
    );
    assert_eq!(
        requirements(
            &NoIcons,
            &localisation(),
            b"attack",
            Ok("Yes\n\nNo\n".into()),
            &mut BTreeSet::new(),
        ),
        Ok("Requirements:\n  Yes\n\n  No\n".into())
    );
}

#[test]
fn possible_tactic_status_checks_locked_conditions_and_uses_signed_total() {
    let content = parsed(
        b"attack={base={factor=4 modifier={factor=1.5 always=yes}}}
        defend={active=no trigger={is_attacker=no}}",
    );
    let detail = LandCombatDetailsRow::default();
    let aliases = BTreeMap::new();
    let context = context(&detail, &content, &aliases);
    let attack = candidate(1, "attack");
    let weights = [Some(600_000), Some(-200_000)];
    assert_eq!(
        status(
            &NoIcons,
            &localisation(),
            &context,
            &attack,
            content.definition(&attack).unwrap(),
            &weights,
            0,
            &mut BTreeSet::new(),
        )
        .unwrap(),
        "Chance:150.00%\nTotal:4.00\nWeight:6.00\nBase:4.00\nScoped:6.00\n6.00/4.00=150.00%"
    );
    let defend = candidate(2, "defend");
    assert_eq!(
        status(
            &NoIcons,
            &localisation(),
            &context,
            &defend,
            content.definition(&defend).unwrap(),
            &[None, Some(0)],
            1,
            &mut BTreeSet::new(),
        ),
        Ok("Conditions failed\n".into())
    );
}

#[test]
fn possible_tactic_status_lists_country_then_commander_preferences() {
    let content = parsed(b"attack={}");
    let mut detail = LandCombatDetailsRow::default();
    detail.tactic_inputs.preferred_tactic_ids[0] = [1, 1, 1];
    detail.tactic_inputs.country_weight_present[0] = 1;
    detail.tactic_inputs.country_weights[0].accumulator_entry_present = 1;
    detail.tactic_inputs.country_weights[0].accumulator_raw = 15_000;
    let aliases = BTreeMap::new();
    let context = context(&detail, &content, &aliases);
    let attack = candidate(1, "attack");
    let text = status(
        &NoIcons,
        &localisation(),
        &context,
        &attack,
        content.definition(&attack).unwrap(),
        &[Some(215_625)],
        0,
        &mut BTreeSet::new(),
    )
    .unwrap();
    assert!(text.contains("Base:1.00\nCountry:15%\nGeneral:50%\nMarshal:25%\n"));
    assert!(text.ends_with("2.15/2.15=100.00%"));
}

#[test]
fn possible_tactic_optional_fragment_refusal_preserves_known_effects_and_weight() {
    let mut content = parsed(b"attack={is_attacker=yes attacker=0.1} other={}");
    content.registry[1].definition = Err("unread later tactic".into());
    let attack = candidate(1, "attack");
    let mut gaps = BTreeSet::new();
    assert_eq!(
        main(
            &NoIcons,
            &localisation(),
            &content,
            &attack,
            content.definition(&attack).unwrap(),
            &mut gaps
        )
        .unwrap(),
        "Attack\nAttacker: good 0.10\n\n"
    );
    assert!(gaps.contains("tactic attack's counters hover: unread later tactic"));
    gaps.clear();
    let detail = LandCombatDetailsRow::default();
    let aliases = BTreeMap::new();
    let context = context(&detail, &content, &aliases);
    let loc = localisation()
        .iter()
        .filter(|(key, _)| *key != "TACTICS_BASE_WEIGHT")
        .map(|(key, text)| (key.to_owned(), text.to_owned()))
        .collect();
    let text = status(
        &NoIcons,
        &loc,
        &context,
        &attack,
        content.definition(&attack).unwrap(),
        &[Some(400_000)],
        0,
        &mut gaps,
    )
    .unwrap();
    assert!(text.starts_with("Chance:100.00%\nTotal:4.00\nWeight:4.00\n"));
    assert!(text.ends_with("4.00/4.00=100.00%"));
    // A missing key is the native key-copy fallback, not an unavailable fragment.
    assert!(text.contains("TACTICS_BASE_WEIGHT"));
    assert!(gaps.is_empty(), "{gaps:?}");
    // A reached text control that the viewer cannot draw is a real refusal.
    let loc = localisation()
        .iter()
        .map(|(key, text)| {
            (
                key.to_owned(),
                if key == "TACTICS_BASE_WEIGHT" {
                    "Base:$WEIGHT$\u{14}\n".into()
                } else {
                    text.to_owned()
                },
            )
        })
        .collect();
    let text = status(
        &NoIcons,
        &loc,
        &context,
        &attack,
        content.definition(&attack).unwrap(),
        &[Some(400_000)],
        0,
        &mut gaps,
    )
    .unwrap();
    assert_eq!(
        text,
        "Chance:100.00%\nTotal:4.00\nWeight:4.00\n4.00/4.00=100.00%"
    );
    assert_eq!(gaps.len(), 1);
    assert!(
        gaps.iter()
            .any(|why| why.contains("base weight") && why.contains("TACTICS_BASE_WEIGHT"))
    );
    gaps.clear();
    let text = status(
        &NoIcons,
        &localisation(),
        &context,
        &attack,
        content.definition(&attack).unwrap(),
        &[Some(400_000), None],
        0,
        &mut gaps,
    )
    .unwrap();
    assert_eq!(text, "Weight:4.00\nBase:1.00\n");
    assert_eq!(gaps.len(), 3); // chance, total, calculation depend on the unknown other tactic.
    gaps.clear();
    let large = 1_i64 << 58;
    let text = status(
        &NoIcons,
        &localisation(),
        &context,
        &attack,
        content.definition(&attack).unwrap(),
        &[Some(large), Some(-large - 1)],
        0,
        &mut gaps,
    )
    .unwrap();
    assert!(text.contains("Base:1.00"));
    assert!(gaps.iter().any(|why| why.contains("division would trap")));
}

#[test]
fn possible_tactic_hover_refuses_only_the_missing_fragment() {
    let mut row = Values::new();
    let mut gaps = BTreeSet::new();
    let requirements = requirements(
        &NoIcons,
        &localisation(),
        b"attack",
        Err("missing terrain input".into()),
        &mut gaps,
    );
    bind(
        &mut row,
        "attack",
        Ok("Attack\n".into()),
        requirements,
        Ok("Conditions failed\n".into()),
        &mut gaps,
    );
    assert_eq!(row[""].tooltip.as_deref(), Some("Attack"));
    assert_eq!(
        row[""].tooltip_description.as_deref(),
        Some("Requirements:\nConditions failed")
    );
    assert!(row[""].hover_owned);
    assert_eq!(
        gaps,
        BTreeSet::from([
            "tactic attack's requirement descriptions hover: missing terrain input".into()
        ])
    );
}
