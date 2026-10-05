use super::*;

/// A refused tactic picture clears the sprite bound before it and hides
/// the widget; an undeclared portrait sprite leaves the widget as it was
/// (1422E6410). Both are named and counted.
#[test]
fn a_refused_portrait_or_tactic_picture_hides_the_earlier_sprite() {
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
    // An undeclared sprite: the setter keeps the widget as it was.
    portrait(&mut out, &has, "leader", &value, 2);
    assert_eq!(out.values["leader"].sprite.as_deref(), Some("GFX_earlier"));
    let tactics = super::super::tactics::Content::from_database(installed_content::combat_tactics::parse(
        b"a = { picture = tactic_a } b = { picture = tactic_b }",
        &installed_content::key_names::KeyNames::default(),
    ));
    tactic(&mut out, &has, &tactics, "tactic", 2, b"b");
    assert_eq!(out.values["tactic"].sprite, None);
    assert!(out.values["tactic"].hidden);
    let refused: Vec<_> = out.refused().keys().cloned().collect();
    assert_eq!(
        refused,
        [
            "Combat portrait leader: the content does not declare sprite GFX_missing, so the game's sprite setter (1422E6410) keeps the widget's previous image; the viewer shows the window's own sprite",
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
