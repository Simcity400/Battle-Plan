use super::*;

#[test]
fn a_refused_portrait_or_tactic_picture_hides_the_earlier_sprite() {
    content_reader_script::test_game::enter();
    let mut out = Bindings::default();
    let has = |name: &str| name == "GFX_known" || name == "combat_tactic:tactic_a";
    sprite(&mut out.values, "leader", "GFX_earlier".into());
    sprite(&mut out.values, "tactic", "GFX_earlier".into());
    let value = CombatCommanderPresentation {
        character_state: 2,
        portraits: vec![CombatCommanderPortraitEntry {
            role: 1,
            size: 1,
            is_set: 1,
            kind: 0,
            text: b"GFX_missing".to_vec(),
        }],
        ..Default::default()
    };
    portrait(&mut out, &has, "leader", &value, 2);
    assert_eq!(out.values["leader"].sprite.as_deref(), Some("GFX_earlier"));
    let tactics =
        super::super::tactics::Content::from_database(content_reader_military::combat_tactics::parse(
            b"a = { picture = tactic_a } b = { picture = tactic_b }",
            &content_reader_script::key_names::KeyNames::default(),
        ));
    tactic(&mut out, &has, &tactics, "tactic", 2, b"b");
    assert_eq!(out.values["tactic"].sprite, None);
    assert!(out.values["tactic"].hidden);
    let refused: Vec<_> = out.refused().keys().cloned().collect();
    assert_eq!(
        refused,
        [
            "Combat portrait leader: the content does not declare sprite GFX_missing, so the game's sprite setter keeps the widget's previous image; the viewer shows the window's own sprite",
            "Combat tactic picture tactic is hidden: the content lacks sprite combat_tactic:tactic_b for tactic b",
        ]
    );
    tactic(&mut out, &has, &tactics, "tactic", 1, b"a");
    assert_eq!(
        out.values["tactic"].sprite.as_deref(),
        Some("combat_tactic:tactic_a")
    );
    assert!(!out.values["tactic"].hidden);
}

#[test]
fn portrait_role_fallback_stays_at_the_requested_size() {
    let mut value = CombatCommanderPresentation {
        character_state: 2,
        ..Default::default()
    };
    for (role, size, text) in [(1, 0, "wrong_size"), (4, 1, "naval"), (1, 1, "first_role")] {
        value.portraits.push(CombatCommanderPortraitEntry {
            role,
            size,
            is_set: 1,
            kind: 0,
            text: text.as_bytes().into(),
        });
    }
    assert_eq!(
        select_portrait(&value, 3).unwrap(),
        PortraitAsset::Declaration("naval".into())
    );
    assert_eq!(
        select_portrait(&value, 2).unwrap(),
        PortraitAsset::Declaration("first_role".into())
    );
    value.portraits[1].is_set = 0;
    assert!(select_portrait(&value, 3).is_err());
}

#[test]
fn authenticated_character_does_not_use_unresolved_leader_fallback() {
    let value = CombatCommanderPresentation {
        character_state: 2,
        fallback_text: [b"gfx/leaders/old.dds".to_vec(), Vec::new(), Vec::new()],
        ..Default::default()
    };
    assert!(select_portrait(&value, 0).is_err());
    assert!(texture(b"../outside.dds").is_err());
    assert!(texture(b"C:\\outside.dds").is_err());
}

#[test]
fn exile_operand_refusal_hides_only_badge_and_keeps_commander_portrait() {
    let mut out = Bindings::default();
    sprite(&mut out.values, "leader", "GFX_synthetic_portrait".into());
    let mut values = Values::new();
    exile_entry(&mut out, &mut values, Ok("synthetic_flag".into()));
    assert_eq!(values["exile_flag"].flag.as_deref(), Some("synthetic_flag"));
    assert!(!values.contains_key("Background"));
    exile_entry(
        &mut out,
        &mut values,
        Err("missing commander country".into()),
    );
    assert!(values["exile_flag"].hidden && values["exile_flag_frame"].hidden);
    assert_eq!(
        out.values["leader"].sprite.as_deref(),
        Some("GFX_synthetic_portrait")
    );
    assert_eq!(out.refused().values().sum::<usize>(), 1);
}

#[test]
fn null_tactic_keeps_authored_art_and_counters_the_real_defender() {
    content_reader_script::test_game::enter();
    let mut out = Bindings::default();
    for prefix in ["attacker", "defender"] {
        hide(&mut out.values, &format!("{prefix}_tactic"));
        hide(
            &mut out.values,
            &format!("{prefix}_countered_tactic_overlay"),
        );
    }
    let content =
        super::super::tactics::Content::from_database(content_reader_military::combat_tactics::parse(
            b"defend = { picture = mod_defense }",
            &content_reader_script::key_names::KeyNames::default(),
        ));
    let mut sides = [
        LandCombatSideDetails {
            tactic_kind: 1,
            ..Default::default()
        },
        LandCombatSideDetails {
            tactic_kind: 2,
            tactic_id: 1,
            tactic_token: b"defend".to_vec(),
            tactic_counter_id: 0,
            ..Default::default()
        },
    ];
    let has = |name: &str| name == "combat_tactic:mod_defense";
    land_tactics(&mut out, &has, &content, false, &sides);
    assert!(!out.values["attacker_tactic"].hidden);
    assert!(
        out.values["attacker_tactic"].sprite.is_none(),
        "the .gui supplies its own artwork"
    );
    assert!(out.values["attacker_tactic"].retain_sprite);
    assert!(out.values["attacker_countered_tactic_overlay"].hidden);
    assert!(!out.values["defender_countered_tactic_overlay"].hidden);
    assert_eq!(
        out.values["defender_tactic"].sprite.as_deref(),
        Some("combat_tactic:mod_defense")
    );

    sides[0].tactic_kind = 0;
    land_tactics(&mut out, &has, &content, false, &sides);
    assert!(out.values["defender_countered_tactic_overlay"].hidden);
}

#[test]
fn null_tactic_draws_the_empty_picture_path_when_content_supplies_it() {
    let mut out = Bindings::default();
    let content = super::super::tactics::Content::default();
    let sides = std::array::from_fn(|_| LandCombatSideDetails {
        tactic_kind: 1,
        ..Default::default()
    });
    let has = |name: &str| name == super::super::tactics::NULL_PICTURE;
    land_tactics(&mut out, &has, &content, true, &sides);
    assert_eq!(
        out.values["attacker_tactic"].sprite.as_deref(),
        Some("combat_tactic:")
    );
    assert!(!out.values["attacker_tactic"].hidden);
    assert!(!out.values["attacker_countered_tactic_overlay"].hidden);
    assert!(!out.values["defender_countered_tactic_overlay"].hidden);
    land_tactics(&mut out, &|_| false, &content, true, &sides);
    assert!(out.values["attacker_tactic"].hidden);
    assert_eq!(out.refused().values().sum::<usize>(), 2);
}
