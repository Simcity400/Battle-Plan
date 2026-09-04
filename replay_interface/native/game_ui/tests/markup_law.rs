
use game_ui::assets::AssetResolver;
use game_ui::loc::Localisation;
use game_ui::markup::{parse, TextColorTable, Token, SECTION};
use std::path::PathBuf;

fn game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn text_color_table_matches_shipped_core_gfx() {
    let root = game_root();
    let resolver = AssetResolver::new(vec![root]);
    let table = TextColorTable::load(&resolver).expect("core.gfx loads");

    let expected: &[(char, [u8; 3])] = &[
        ('C', [35, 206, 255]),
        ('L', [195, 176, 145]),
        ('W', [255, 255, 255]),
        ('B', [0, 0, 255]),
        ('G', [0, 159, 3]),
        ('R', [255, 50, 50]),
        ('b', [0, 0, 0]),
        ('g', [176, 176, 176]),
        ('Y', [255, 189, 0]),
        ('H', [255, 189, 0]),
        ('T', [255, 255, 255]),
        ('O', [255, 112, 25]),
        ('0', [203, 0, 203]),
        ('1', [128, 120, 211]),
        ('2', [81, 112, 243]),
        ('3', [81, 143, 220]),
        ('4', [90, 190, 231]),
        ('5', [63, 181, 194]),
        ('6', [119, 204, 186]),
        ('7', [153, 209, 153]),
        ('8', [204, 163, 51]),
        ('9', [252, 169, 125]),
        ('t', [255, 76, 77]),
    ];
    for (code, rgb) in expected {
        assert_eq!(
            table.get(*code),
            Some(*rgb),
            "colour code §{code} must decode to {rgb:?}"
        );
    }
    assert_ne!(table.get('b'), table.get('B'));
    assert_ne!(table.get('g'), table.get('G'));
    assert!(
        table.len() >= expected.len(),
        "table has {} codes",
        table.len()
    );
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn shipped_english_localisation_tokenises_cleanly() {
    let root = game_root();
    let resolver = AssetResolver::new(vec![root]);
    let table = TextColorTable::load(&resolver).expect("core.gfx loads");
    let loc = Localisation::load_english(&resolver).expect("localisation loads");
    assert!(
        loc.entries.len() > 10_000,
        "loaded {} keys",
        loc.entries.len()
    );

    let mut unknown_codes = std::collections::BTreeSet::new();
    let mut strings_with_markup = 0usize;
    let mut icon_tokens = 0usize;

    for value in loc.entries.values() {
        let mut chars = value.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == SECTION {
                if chars.peek() == Some(&'[') {
                    for next in chars.by_ref() {
                        if next == ']' {
                            break;
                        }
                    }
                    continue;
                }
                if let Some(code) = chars.next() {
                    if code != '!' && table.get(code).is_none() {
                        unknown_codes.insert(code);
                    }
                }
            }
        }

        let tokens = parse(value, &table);
        if tokens.len() > 1
            || tokens
                .iter()
                .any(|t| !matches!(t, Token::Text { color: None, .. }))
        {
            strings_with_markup += 1;
        }
        icon_tokens += tokens
            .iter()
            .filter(|t| matches!(t, Token::Icon { .. }))
            .count();

        let concat: String = tokens
            .iter()
            .filter_map(|t| match t {
                Token::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            concat,
            game_ui::markup::strip(value),
            "text-run concatenation must equal stripped text for: {value:?}"
        );
    }

    assert!(
        unknown_codes.is_empty(),
        "localisation uses §-codes absent from core.gfx: {unknown_codes:?}"
    );
    assert!(
        strings_with_markup > 1000,
        "only {strings_with_markup} marked-up strings"
    );
    assert!(
        icon_tokens > 0,
        "expected some £ inline icons in the corpus"
    );
    eprintln!(
        "markup gate: {} keys, {} with markup, {} £-icon tokens, 0 unknown §-codes",
        loc.entries.len(),
        strings_with_markup,
        icon_tokens
    );
}
