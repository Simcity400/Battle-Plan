use super::*;
use content_reader_layers::test_support::Stack;
use content_reader_sound::combat_sounds::{BattleDefines, Layers};
use std::collections::BTreeMap;

fn battle(key: u64, center: [f32; 2], radius: f32, divisions: usize, infantry: usize) -> Battle {
    Battle {
        key,
        provinces: vec![key as u32],
        center,
        radius,
        divisions,
        sub_units: BTreeMap::from([("infantry".to_owned(), infantry)]),
    }
}

fn content(name: &str) -> (Stack, Content) {
    let stack = Stack::new(name);
    let base = stack.base();
    stack.write(
        &base,
        "sound/test.asset",
        r#"sound = { name = boom file = "boom.wav" }
           falloff = { name = far max_distance = 100 }
           soundeffect = { name = distant sounds = { sound = boom } falloff = far }
           soundeffect = { name = rifles sounds = { sound = boom } }"#,
    );
    stack.write(
        &base,
        "sound/combat_sounds/00.txt",
        "rifle = { sound_effect = rifles units = { infantry } divisions_range = { 5 -1 } }",
    );
    let layers = stack.layers(&[]);
    let catalog = content_reader_sound::sound::Catalog::read(&layers).unwrap();
    let combat = Layers::read(&layers).unwrap();
    let content = Content::new(catalog, None).with_battles(
        Some(BattleDefines {
            effect: "distant".into(),
            falloff_default: 50.0,
            min_unit_count: 6,
        }),
        combat,
    );
    (stack, content)
}

#[test]
fn audible_battles_play_their_ambience_and_layers() {
    let (_stack, content) = content("battle_emitters");
    let battles = Battles {
        battles: vec![
            battle(1, [0.0, 0.0], 9.0, 6, 5),
            battle(2, [500.0, 0.0], 9.0, 5, 9),
            battle(3, [1000.0, 0.0], 9.0, 9, 4),
            battle(4, [3000.0, 0.0], 400.0, 6, 0),
        ],
        refusals: Vec::new(),
    };
    let mut out = Vec::new();
    emitters(&battles, &content, [100.0, 300.0, 0.0], &mut out);
    let effects: Vec<_> = out.iter().map(|emitter| &*emitter.effect).collect();
    assert_eq!(effects, ["distant", "rifles"]);
    assert_eq!(out[0].position, [0.0, BATTLE_HEIGHT, 0.0]);
    assert!(
        out.iter()
            .all(|emitter| emitter.restart && emitter.group == Group::Combat)
    );
    out.clear();
    emitters(&battles, &content, [109.5, 0.0, 0.0], &mut out);
    assert!(out.is_empty());
    emitters(&battles, &content, [500.0, 0.0, 0.0], &mut out);
    assert!(out.is_empty());
    emitters(&battles, &content, [1000.0, 0.0, 0.0], &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(&*out[0].effect, "distant");
    out.clear();
    emitters(&battles, &content, [2550.0, 0.0, 0.0], &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].position, [3000.0, BATTLE_HEIGHT, 0.0]);
}

#[test]
fn emitter_keys_follow_the_battle_key_not_its_place() {
    let (_stack, content) = content("battle_keys");
    let near = battle(7, [0.0, 0.0], 9.0, 6, 5);
    let far = battle(8, [5000.0, 0.0], 9.0, 6, 5);
    let mut first = Vec::new();
    emitters(
        &Battles {
            battles: vec![near.clone(), far.clone()],
            refusals: Vec::new(),
        },
        &content,
        [0.0; 3],
        &mut first,
    );
    let mut second = Vec::new();
    emitters(
        &Battles {
            battles: vec![far, near.clone()],
            refusals: Vec::new(),
        },
        &content,
        [0.0; 3],
        &mut second,
    );
    assert_eq!(first, second);
    let mut other = Vec::new();
    emitters(
        &Battles {
            battles: vec![Battle { key: 9, ..near }],
            refusals: Vec::new(),
        },
        &content,
        [0.0; 3],
        &mut other,
    );
    assert!(other.iter().zip(&first).all(|(a, b)| a.key != b.key));
}

#[test]
fn a_missing_distant_effect_silences_every_battle_sound() {
    let (_declared_stack, declared) = content("battle_declared");
    let (_stack, mut content) = content("battle_no_effect");
    content.battle.as_mut().unwrap().effect = "absent".into();
    let battles = Battles {
        battles: vec![battle(1, [0.0, 0.0], 9.0, 9, 9)],
        refusals: Vec::new(),
    };
    let mut out = Vec::new();
    emitters(&battles, &content, [0.0; 3], &mut out);
    assert!(out.is_empty());
    assert_eq!(
        silence(&content).unwrap(),
        "the battle sound effect absent is not declared, so no battle plays (the game skips its whole battle-sound update)"
    );
    content.battle = None;
    assert!(
        silence(&content)
            .unwrap()
            .contains("defines are unreadable")
    );
    assert_eq!(silence(&declared), None);
}
