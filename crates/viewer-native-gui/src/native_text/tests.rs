use super::*;
use std::cell::Cell;

struct Mono;

impl Glyphs for Mono {
    fn own(&self, ch: char) -> Option<Glyph> {
        let advance = match ch {
            ' ' => 4,
            '.' => 3,
            ch if (' '..='~').contains(&ch) => 7,
            _ => return None,
        };
        Some(Glyph {
            ch,
            advance,
            width: advance,
            height: if ch == ' ' { 0 } else { 14 },
            xoffset: 0,
            yoffset: 1,
        })
    }

    fn kerning(&self, a: char, b: char) -> i32 {
        if (a, b) == ('A', 'V') { -2 } else { 0 }
    }

    fn line_height(&self) -> i32 {
        14
    }
}

fn limits(width: i32) -> Limits {
    Limits {
        width,
        height: i32::MAX,
        border: [0; 2],
        newlines: true,
        preserve_indent: false,
        ellipsis: false,
        multiline: true,
        special: true,
        icons_add_height: false,
    }
}

fn laid(text: &str, limits: Limits) -> (String, Vec<i32>) {
    let chars: Vec<char> = text.chars().collect();
    let laid = lay_out(&Mono, &|_| None, &chars, limits).unwrap();
    (laid.text.iter().collect(), laid.widths)
}

#[test]
fn empty_inline_icon_name_skips_even_a_defined_empty_stem() {
    let calls = Cell::new(0);
    let sprite = |name: &[u8]| {
        calls.set(calls.get() + 1);
        (name == b"GFX_").then_some(([100, 100], 1))
    };
    let limits = Limits {
        icons_add_height: true,
        ..limits(i32::MAX)
    };
    let empty: Vec<_> = "a\u{13} b".chars().collect();
    let laid = lay_out(&Mono, &sprite, &empty, limits).unwrap();
    assert_eq!(laid.widths, [14]);
    assert_eq!(laid.heights, [14]);
    let colors = Colors {
        own: [1.; 4],
        font: &BTreeMap::new(),
        global: &BTreeMap::new(),
    };
    let pieces = draw(
        &Mono,
        &sprite,
        &laid,
        true,
        &Placement {
            top: 0,
            start: &|_, _| 0,
        },
        &colors,
    )
    .unwrap();
    assert_eq!(pieces.len(), 2);
    assert_eq!(calls.get(), 0);
    let nonempty: Vec<_> = "a\u{13}|1 b".chars().collect();
    let laid = lay_out(&Mono, &sprite, &nonempty, limits).unwrap();
    assert_eq!(laid.widths, [114]);
    assert_eq!(laid.heights, [100]);
    assert_eq!(calls.get(), 1);
}

#[test]
fn lines_break_as_the_game_breaks_them() {
    assert_eq!(laid("ab cd", limits(25)), ("ab \ncd".into(), vec![14, 14]));
    assert_eq!(
        laid("ab  cd", limits(26)),
        ("ab  \ncd".into(), vec![18, 14])
    );
    assert_eq!(laid("abcd", limits(15)), ("ab\ncd".into(), vec![14, 14]));
    assert_eq!(laid("a", limits(5)), ("\na".into(), vec![0, 7]));
    let mut bordered = limits(29);
    bordered.border = [2, 0];
    assert_eq!(laid("ab cd", bordered).1, vec![14, 14]);
    assert_eq!(laid("AV\nx", limits(100)), ("AV\nx".into(), vec![12, 7]));
}

#[test]
fn tooltip_soft_wrap_preserves_literal_leading_spaces() {
    let indent = Limits {
        preserve_indent: true,
        ..limits(25)
    };
    assert_eq!(laid(" ab cd", indent), (" ab \n cd".into(), vec![18, 18]));
    assert_eq!(
        laid(" ab cdef", indent),
        (" ab \n cd\n ef".into(), vec![18, 18, 18])
    );
    assert_eq!(
        laid(" ab cd\nxy", indent),
        (" ab \n cd\nxy".into(), vec![18, 18, 14])
    );
    assert_eq!(
        laid(&format!("{COLOR}R ab cd"), indent),
        (format!("{COLOR}R ab \ncd"), vec![18, 14])
    );
    assert_eq!(
        laid("\t ab cd", indent),
        ("\t ab \ncd".into(), vec![18, 14])
    );
    assert_eq!(
        laid("  x", Limits { width: 8, ..indent }),
        (" \n  \nx".into(), vec![0, 4, 7])
    );
    assert_eq!(
        laid(" ab cd", limits(25)),
        (" ab \ncd".into(), vec![18, 14])
    );
}

#[test]
fn tooltip_indent_uses_space_descriptor_fallback() {
    struct MissingSpace(bool);
    impl Glyphs for MissingSpace {
        fn own(&self, ch: char) -> Option<Glyph> {
            if ch == ' ' || (ch == '?' && self.0) {
                None
            } else {
                Mono.own(ch)
            }
        }
        fn kerning(&self, a: char, b: char) -> i32 {
            Mono.kerning(a, b)
        }
        fn line_height(&self) -> i32 {
            14
        }
    }
    let chars: Vec<char> = " abcd".chars().collect();
    for (missing_fallback, width, expected_widths) in
        [(false, 25, vec![21, 21]), (true, 15, vec![14, 14])]
    {
        let laid = lay_out(
            &MissingSpace(missing_fallback),
            &|_| None,
            &chars,
            Limits {
                preserve_indent: true,
                ..limits(width)
            },
        )
        .unwrap();
        assert_eq!(laid.text.iter().collect::<String>(), " ab\n cd");
        assert_eq!(laid.widths, expected_widths);
    }
}

#[test]
fn cut_text_ends_with_three_dots() {
    let cut = |text: &str, width: i32| {
        let chars: Vec<char> = text.chars().collect();
        let laid = lay_out(
            &Mono,
            &|_| None,
            &chars,
            Limits {
                width,
                height: 20,
                border: [0; 2],
                newlines: false,
                preserve_indent: false,
                ellipsis: true,
                multiline: false,
                special: true,
                icons_add_height: false,
            },
        )
        .unwrap();
        (laid.text.iter().collect::<String>(), laid.truncated)
    };
    assert_eq!(cut("abcdef", 30), ("ab...".into(), true));
    assert_eq!(cut("ab\ncd", 100), ("ab".into(), true));
    assert_eq!(cut("abc\ncd", 100), ("abc...".into(), true));
    assert_eq!(cut("abc", 100), ("abc".into(), false));
    assert_eq!(cut("abc", 15), ("ab".into(), true));
}

#[test]
fn special_sequences_have_their_widths() {
    let sprite = |name: &[u8]| (name == b"flag_x").then_some(([60, 30], 3));
    let chars: Vec<char> = format!("a{COLOR}Rb{ICON}flag_x|2 c").chars().collect();
    let laid = lay_out(&Mono, &sprite, &chars, limits(1000)).unwrap();
    assert_eq!(laid.widths, vec![7 + 7 + 20 + 7]);
    assert_eq!(laid.heights, vec![14]);
    let raised = lay_out(
        &Mono,
        &sprite,
        &chars,
        Limits {
            icons_add_height: true,
            ..limits(1000)
        },
    )
    .unwrap();
    assert_eq!(raised.heights, vec![30]);
    let zero = |_: &[u8]| Some(([60, 30], 0));
    assert!(lay_out(&Mono, &zero, &chars, limits(1000)).is_err());
    assert_eq!(
        special(&[ICON, 'a', '_', 'b', '£', 'c']),
        Some((5, Special::Icon("a_b".into())))
    );
}

#[test]
fn kerning_reads_the_original_codepoint() {
    struct Kerned;
    impl Glyphs for Kerned {
        fn own(&self, ch: char) -> Option<Glyph> {
            Mono.own(ch)
        }
        fn kerning(&self, a: char, b: char) -> i32 {
            match (a, b) {
                ('A', '\u{e9}') => -3,
                ('A', '?') => -1,
                _ => 0,
            }
        }
        fn line_height(&self) -> i32 {
            14
        }
    }
    let chars: Vec<char> = "A\u{e9}".chars().collect();
    let laid = lay_out(&Kerned, &|_| None, &chars, limits(1000)).unwrap();
    assert_eq!(laid.widths, vec![7 + 7 - 3]);
}

#[test]
fn the_draw_colours_and_places_glyphs() {
    let chars: Vec<char> = format!("a{COLOR}Rb{COLOR}Gc{COLOR}!d{COLOR}!e\nAV")
        .chars()
        .collect();
    let laid = lay_out(&Mono, &|_| None, &chars, limits(1000)).unwrap();
    let font = BTreeMap::from([(b'R', [1.0, 0.0, 0.0, 1.0])]);
    let global = BTreeMap::from([(b'G', [0.0, 1.0, 0.0, 1.0])]);
    let colors = Colors {
        own: [0.5, 0.5, 0.5, 1.0],
        font: &font,
        global: &global,
    };
    let pieces = draw(
        &Mono,
        &|_| None,
        &laid,
        true,
        &Placement {
            top: 10,
            start: &|_, _| 100,
        },
        &colors,
    )
    .unwrap();
    let glyph = |index: usize| match &pieces[index] {
        Piece::Glyph { ch, at, color } => (*ch, *at, *color),
        _ => panic!("not a glyph"),
    };
    let grey = packed([0.5, 0.5, 0.5, 1.0]);
    assert_eq!(glyph(0), ('a', [100, 11], grey));
    assert_eq!(glyph(1).2, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(glyph(2).2, [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(glyph(3).2, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(glyph(4).2, grey);
    assert_eq!(glyph(5).1, [100, 25]);
    assert_eq!(glyph(6).1, [105, 25]);
}

#[test]
fn icons_and_flags_draw_where_the_font_puts_them() {
    let sprite = |name: &[u8]| (name == b"GFX_x").then_some(([30, 20], 3));
    let chars: Vec<char> = format!("{ICON}x|3 {ICON}x| {FLAG}GER{ICON}y z")
        .chars()
        .collect();
    let laid = lay_out(&Mono, &sprite, &chars, limits(1000)).unwrap();
    let colors = Colors {
        own: [1.0; 4],
        font: &BTreeMap::new(),
        global: &BTreeMap::new(),
    };
    let pieces = draw(
        &Mono,
        &sprite,
        &laid,
        true,
        &Placement {
            top: 10,
            start: &|_, _| 100,
        },
        &colors,
    )
    .unwrap();
    let third = 1.0f32 / 3.0;
    assert_eq!(
        pieces[0],
        Piece::Icon {
            sprite: b"GFX_x".to_vec(),
            at: [100, 10 + 7 - 10],
            size: [10, 20],
            uv: [2.0 * third, 0.0, (3.0 * third).min(1.0), 1.0],
        }
    );
    assert!(matches!(&pieces[1], Piece::Icon { at, uv, .. }
        if at[0] == 110 && uv[0] == -third && uv[2] == 0.0));
    assert_eq!(
        pieces[2],
        Piece::Flag {
            tag: "GER".into(),
            at: [120, 10 + 5],
            resolved: false,
        }
    );
    assert_eq!(pieces[3], Piece::Undefined("y".into()));
    assert!(matches!(
        pieces[4],
        Piece::Glyph {
            ch: 'z',
            at: [132, _],
            ..
        }
    ));
    let chars: Vec<char> = format!("a{CURRENCY}").chars().collect();
    let laid = lay_out(&Mono, &sprite, &chars, limits(1000)).unwrap();
    let placement = Placement {
        top: 0,
        start: &|_, _| 0,
    };
    let error = draw(&Mono, &sprite, &laid, true, &placement, &colors).unwrap_err();
    assert_eq!(
        error,
        "shows a currency or number sprite (0x14, 0x1A), whose record reads an uninitialised allocation: the game reads uninitialised memory"
    );
}

#[test]
fn resolved_flags_are_scoped_and_keep_the_full_atlas_key() {
    let chars: Vec<_> = format!("a{RESOLVED_FLAG}SYN-Western_group, b{RESOLVED_FLAG}XYZ_other")
        .chars()
        .collect();
    let key = |text: &[char]| {
        ["SYN", "SYN-Western_group", "XYZ_other"]
            .into_iter()
            .map(|key| key.chars().collect::<Vec<_>>())
            .filter(|key| text.starts_with(key))
            .map(|key| key.len())
            .max()
    };
    assert!(special(&chars[1..]).is_none());
    let native = lay_out(&Mono, &|_| None, &chars, limits(1000)).unwrap();
    let bound = lay_out_resolved_flags(&Mono, &|_| None, &chars, limits(1000), &key).unwrap();
    assert_eq!(bound.text, chars);
    assert_eq!(bound.widths, [49]);
    assert!(native.widths[0] > bound.widths[0]);
    let colors = Colors {
        own: [1.0; 4],
        font: &BTreeMap::new(),
        global: &BTreeMap::new(),
    };
    let placement = Placement {
        top: 10,
        start: &|_, _| 100,
    };
    let pieces = draw(&Mono, &|_| None, &bound, true, &placement, &colors).unwrap();
    let flags: Vec<_> = pieces
        .iter()
        .filter_map(|piece| match piece {
            Piece::Flag { tag, at, resolved } => {
                assert!(*resolved);
                Some((tag.as_str(), *at))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        flags,
        [("SYN-Western_group", [107, 15]), ("XYZ_other", [137, 15])]
    );
    assert!(pieces.iter().any(|piece| matches!(
        piece,
        Piece::Glyph {
            ch: ',',
            at: [119, _],
            ..
        }
    )));
    assert!(
        draw(&Mono, &|_| None, &native, true, &placement, &colors)
            .unwrap()
            .iter()
            .all(|piece| !matches!(piece, Piece::Flag { .. }))
    );
    let ignored = lay_out_resolved_flags(
        &Mono,
        &|_| None,
        &chars,
        Limits {
            special: false,
            ..limits(1000)
        },
        &key,
    )
    .unwrap();
    assert_eq!(ignored.widths, native.widths);
    assert!(
        draw(&Mono, &|_| None, &ignored, false, &placement, &colors)
            .unwrap()
            .iter()
            .all(|piece| !matches!(piece, Piece::Flag { .. }))
    );
}
