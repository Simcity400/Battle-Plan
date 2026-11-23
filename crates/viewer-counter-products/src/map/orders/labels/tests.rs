use super::*;
use fixed::SCALE;
use std::cell::Cell;
use std::cell::RefCell;
use viewer_foundation::test_support::localisation::loaded;
use viewer_native_gui::text::GlyphBox;

struct Fixed;

impl GlyphMetrics for Fixed {
    fn advance(&self, ch: char) -> Option<f32> {
        self.glyph(ch).map(|g| g.xadvance)
    }
    fn line_height(&self) -> f32 {
        20.0
    }
    fn base(&self) -> f32 {
        16.0
    }
    fn glyph(&self, ch: char) -> Option<GlyphBox> {
        (ch.is_ascii_graphic() || ch == ' ').then_some(GlyphBox {
            width: 10.0,
            height: if ch == ' ' { 0.0 } else { 10.0 },
            xoffset: 0.0,
            yoffset: 5.0,
            xadvance: 10.0,
            uv: [0.0, 0.0, 0.1, 0.1],
        })
    }
}

impl LabelFont for Fixed {
    fn page(&self, _: char) -> Option<usize> {
        Some(0)
    }
}

fn localisation() -> Localisation {
    [
        (KEY_FULL, "$ORDERNAME$$DIVISIONS$ - $GROUP$"),
        (KEY_EMPTY, "No divisions"),
        (KEY_DIVISIONS, "$DIV$ Divs"),
        (KEY_DIVISIONS_SHORT, "$DIV$ Divs"),
        (KEY_EMPTY_SHORT, "No Divs"),
        (KEY_PARATROOPERS, "$DIV$ Paras"),
        (KEY_INVASION_TRANSPORTS, " ($TR$ convoys)"),
        (KEY_PARADROP_TRANSPORTS, " ($TR$ planes)"),
        ("o_strat_offensive_ru", "Strategic Offensive Operation"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

#[test]
fn the_order_name_uses_the_stored_unique_text_and_localised_keys() {
    let loc = localisation();
    let name = OrderName {
        unique: b"South Sudetenland",
        postfix: b"o_strat_offensive_ru",
        ..Default::default()
    };
    assert_eq!(
        order_name(&loc, &name).as_deref(),
        Ok("South Sudetenland Strategic Offensive Operation")
    );
    let codename = OrderName {
        first: b"missing_first",
        second: b"missing_second",
        ..Default::default()
    };
    assert_eq!(
        order_name(&loc, &codename).as_deref(),
        Ok("missing_first missing_second")
    );
}

#[test]
fn the_variants_follow_the_count_the_type_and_the_group_kind() {
    let loc = localisation();
    let name = OrderName {
        unique: b"South Sudetenland",
        postfix: b"o_strat_offensive_ru",
        ..Default::default()
    };
    let compose = |order_type, group_name: &'static str, army_group, count| Composer {
        localisation: &loc,
        order_type,
        name,
        group_name: group_name.as_bytes(),
        army_group,
        count,
    };
    let empty_group = compose(super::super::TYPE_MOVE, "Army Group 11", true, 0);
    assert_eq!(
        empty_group.text(Variant::Full).as_deref(),
        Ok("South Sudetenland Strategic Offensive Operation - Army Group 11")
    );
    assert_eq!(
        empty_group.text(Variant::Middle).as_deref(),
        Ok("No divisions")
    );
    assert_eq!(empty_group.text(Variant::Short).as_deref(), Ok("No Divs"));
    let line = compose(super::super::TYPE_LINE, "Army 28", false, 2);
    assert_eq!(line.text(Variant::Full).as_deref(), Ok("2 Divs - Army 28"));
    assert_eq!(line.text(Variant::Middle).as_deref(), Ok("2 Divs"));
    let latin1 = Composer {
        group_name: b"Arm\xe9e 28",
        ..line
    };
    assert_eq!(
        latin1.text(Variant::Full),
        Err("the order's display group name is not UTF-8".into())
    );
    assert_eq!(latin1.text(Variant::Middle).as_deref(), Ok("2 Divs"));
    let empty_army = compose(super::super::TYPE_LINE, "Army 28", false, 0);
    assert_eq!(
        empty_army.text(Variant::Full).as_deref(),
        Ok("No divisions")
    );
    let drop = compose(super::super::TYPE_PARADROP, "Army 3", false, 4);
    assert_eq!(
        drop.text(Variant::Full).as_deref(),
        Ok("South Sudetenland Strategic Offensive Operation - 4 Paras - Army 3")
    );
    let broken = Composer {
        name: OrderName {
            unique: b"S\xff",
            ..name
        },
        ..drop
    };
    assert_eq!(
        broken.text(Variant::Full),
        Err("the order's unique name is not UTF-8".into())
    );
}

#[test]
fn extended_transports_append_to_full_only_after_the_strict_point_threshold() {
    let loc = localisation();
    let composer = Composer {
        localisation: &loc,
        order_type: super::super::TYPE_INVASION,
        name: OrderName {
            unique: b"Landing",
            ..Default::default()
        },
        group_name: b"Army",
        army_group: false,
        count: 2,
    };
    let reads = Cell::new(0);
    let supplied = || {
        reads.set(reads.get() + 1);
        Ok(-3)
    };
    assert_eq!(
        composer
            .drawn_with_transports(Variant::Full, 5, 5, &supplied)
            .as_deref(),
        Ok("Landing - 2 Divs - Army")
    );
    assert_eq!(
        composer
            .drawn_with_transports(Variant::Middle, 6, 5, &supplied)
            .as_deref(),
        Ok("2 Divs")
    );
    assert_eq!(
        composer
            .drawn_with_transports(Variant::Short, 6, 5, &supplied)
            .as_deref(),
        Ok("2 Divs")
    );
    assert_eq!(reads.get(), 0);
    assert_eq!(
        composer
            .drawn_with_transports(Variant::Full, 6, 5, &supplied)
            .as_deref(),
        Ok("Landing - 2 Divs - Army (-3 convoys)")
    );
    assert_eq!(reads.get(), 1);

    let drop = Composer {
        order_type: super::super::TYPE_PARADROP,
        count: 0,
        ..composer
    };
    assert_eq!(
        drop.drawn_with_transports(Variant::Full, 6, 5, &|| Ok(0))
            .as_deref(),
        Ok("No divisions (0 planes)")
    );
    let line = Composer {
        order_type: super::super::TYPE_LINE,
        ..composer
    };
    assert_eq!(
        line.drawn_with_transports(Variant::Full, 6, 5, &|| {
            Err("a line must not read transport inputs".into())
        })
        .as_deref(),
        Ok("2 Divs - Army")
    );
}

#[test]
fn a_missing_transport_operand_refuses_only_the_full_variant_that_uses_it() {
    let loc = localisation();
    let composer = Composer {
        localisation: &loc,
        order_type: super::super::TYPE_PARADROP,
        name: OrderName::default(),
        group_name: b"Army",
        army_group: false,
        count: 2,
    };
    let missing = || Err("the source transport-wing inputs were not recorded".into());
    assert_eq!(
        composer.drawn_with_transports(Variant::Full, 7, 5, &missing),
        Err("the source transport-wing inputs were not recorded".into())
    );
    assert_eq!(
        composer
            .drawn_with_transports(Variant::Middle, 7, 5, &missing)
            .as_deref(),
        Ok("2 Divs")
    );

    let loc = [(KEY_PARADROP_TRANSPORTS.into(), "\u{12}GER".into())]
        .into_iter()
        .collect();
    let composer = Composer {
        localisation: &loc,
        ..composer
    };
    assert!(
        composer
            .drawn_with_transports(Variant::Full, 7, 5, &|| Ok(2))
            .is_err()
    );
}

#[test]
fn the_run_is_centred_on_the_integer_half_width() {
    let run = centred_run(&Fixed, "AB C", 1.0);
    assert_eq!(run.len(), 3);
    assert!((run[0].left + 1.0).abs() < 1e-6);
    assert!((run[2].right - 1.0).abs() < 1e-6);
    assert!((run[0].top - 0.75).abs() < 1e-6);
    assert!((run[0].bottom - 0.25).abs() < 1e-6);
}

#[test]
fn the_text_path_is_the_longest_x_run_read_left_to_right() {
    let p = |x: i64, z: i64| [x * SCALE, z * SCALE];
    let controls = [p(10, 0), p(5, 0), p(0, 0), p(1, 1), p(2, 2)];
    assert_eq!(text_path(&controls), vec![[5.0, 0.0], [10.0, 0.0]]);
    let controls: Vec<Point> = [0, 1, 2, 3, 4, 5, 4, 6, 7, 8, 9, 10]
        .into_iter()
        .map(|x| p(x, 0))
        .collect();
    let path = text_path(&controls);
    assert_eq!(path.len(), 11);
    assert_eq!(path[0], [0.0, 0.0]);
    let controls: Vec<Point> = [0, 1, 2, 3, 2, 4, 5, 6]
        .into_iter()
        .map(|x| p(x, 0))
        .collect();
    assert_eq!(
        text_path(&controls),
        vec![[2.0, 0.0], [4.0, 0.0], [5.0, 0.0]]
    );
    assert!(text_path(&controls[..1]).is_empty());
}

#[test]
fn the_first_fitting_variant_is_placed_along_the_arc() {
    let p = |x: i64| [x * SCALE, 0];
    let controls: Vec<Point> = (0..=11).map(p).collect();
    let tried = RefCell::new(Vec::new());
    let variants = |variant: Variant| {
        tried.borrow_mut().push(variant);
        Ok(match variant {
            Variant::Full => "LONG LONG LONG LABEL",
            Variant::Middle => "2 Divs",
            Variant::Short => "2 D",
        }
        .to_owned())
    };
    let flat = |_: f32, _: f32, _: f32| 10.0;
    let placed = place(&variants, &controls, &Fixed, 1.0, 0.0, &flat)
        .unwrap()
        .unwrap();
    assert_eq!(placed.text, "LONG LONG LONG LABEL");
    assert_eq!(tried.take(), [Variant::Full]);
    let first = placed.glyphs[0].corners;
    assert!(first[0][2] > 0.0 && first[3][2] < 0.0);
    assert_eq!(first[0][1], 10.0);
    let short: Vec<Point> = (0..=4).map(p).collect();
    let placed = place(&variants, &short, &Fixed, 1.0, 0.0, &flat)
        .unwrap()
        .unwrap();
    assert_eq!(placed.text, "2 Divs");
    assert_eq!(tried.take(), [Variant::Full, Variant::Middle]);
    let tiny: Vec<Point> = (0..=1).map(p).collect();
    assert!(
        place(&variants, &tiny, &Fixed, 1.0, 0.0, &flat)
            .unwrap()
            .is_none()
    );

    let seen = RefCell::new(Vec::new());
    let lift = |_: f32, _: f32, t: f32| {
        seen.borrow_mut().push(t);
        t * 100.0
    };
    let placed = place(&|_| Ok("AB".to_owned()), &controls, &Fixed, 1.0, 0.0, &lift)
        .unwrap()
        .unwrap();
    let count = controls.len() as f32;
    for t in seen.take() {
        let index = t * count;
        assert!(
            (index - index.round()).abs() < 1e-4 && index < count - 1.0,
            "{t}"
        );
    }
    assert!(placed.glyphs[0].corners[0][1] > 0.0);
}

fn modded() -> Localisation {
    loaded(concat!(
        "l_english:\n",
        " MAP_ARROWS_LOCALIZATION:0 \"$DIVISIONS$ - $GROUP$\"\n",
        " MAP_ARROWS_LOCALIZATION_EMPTY:0 \"Keine Divisionen\"\n",
        " MAP_ARROWS_DIVISIONS_COUNT:0 \"$DIV$ Div. ¤\"\n",
        " MAP_ARROWS_DIVISIONS_COUNT_SHORT:0 \"£div_icon $DIV$\"\n",
        " MAP_ARROWS_PARATROOPERS_COUNT:0 \"$DIV$ Fj.\"\n",
    ))
}

#[test]
fn only_a_reached_variant_can_refuse_the_label() {
    let loc = modded();
    let composer = Composer {
        localisation: &loc,
        order_type: super::super::TYPE_PARADROP,
        name: OrderName::default(),
        group_name: "Armée 3".as_bytes(),
        army_group: false,
        count: 4,
    };
    let p = |x: i64| [x * SCALE, 0];
    let flat = |_: f32, _: f32, _: f32| 0.0;
    let drawn = |variant| composer.drawn(variant);
    let long: Vec<Point> = (0..=11).map(p).collect();
    let placed = place(&drawn, &long, &Fixed, 1.0, 0.0, &flat)
        .unwrap()
        .unwrap();
    assert_eq!(placed.text, "4 Fj. - Armée 3");
    let short: Vec<Point> = (0..=4).map(p).collect();
    let middle = place(&drawn, &short, &Fixed, 1.0, 0.0, &flat)
        .unwrap()
        .unwrap();
    assert_eq!(middle.text, "4 Div. \u{14}");
    assert_eq!(
        middle.glyphs.len(),
        centred_run(&Fixed, "4 Div.", 1.0).len()
    );
    let shorter: Vec<Point> = (0..=2).map(p).collect();
    assert_eq!(
        place(&drawn, &shorter, &Fixed, 1.0, 0.0, &flat).unwrap_err(),
        "it holds the icon 0x13 \"div_icon\" (an authored `£`), which map text does not draw"
    );
}

#[test]
fn the_currency_control_takes_no_advance_and_ends_kerning() {
    struct Kerned;
    impl GlyphMetrics for Kerned {
        fn advance(&self, ch: char) -> Option<f32> {
            Fixed.advance(ch)
        }
        fn line_height(&self) -> f32 {
            Fixed.line_height()
        }
        fn base(&self) -> f32 {
            Fixed.base()
        }
        fn glyph(&self, ch: char) -> Option<GlyphBox> {
            Fixed.glyph(ch)
        }
        fn kerning(&self, a: char, b: char) -> f32 {
            if (a, b) == ('A', 'V') { -4.0 } else { 0.0 }
        }
    }
    impl LabelFont for Kerned {
        fn page(&self, _: char) -> Option<usize> {
            Some(0)
        }
    }
    let left = |run: &[RunGlyph]| run.iter().map(|glyph| glyph.left).collect::<Vec<_>>();
    assert_eq!(left(&centred_run(&Kerned, "AV", 1.0)), [-0.4, -0.1]);
    assert_eq!(left(&centred_run(&Kerned, "A\u{14}V", 1.0)), [-0.5, 0.0]);
    assert_eq!(drawable("A\u{14}V".to_owned()), Ok("A\u{14}V".to_owned()));
}

#[test]
fn a_missing_label_key_and_a_missing_token_are_copied() {
    let loc = modded();
    let composer = Composer {
        localisation: &loc,
        order_type: super::super::TYPE_MOVE,
        name: OrderName {
            first: b"Unternehmen",
            second: b"Wei\xc3\x9f",
            ..Default::default()
        },
        group_name: "Heeresgruppe Süd".as_bytes(),
        army_group: true,
        count: 0,
    };
    assert_eq!(
        composer.text(Variant::Full).as_deref(),
        Ok("Unternehmen Weiß - Heeresgruppe Süd")
    );
    assert_eq!(
        composer.text(Variant::Middle).as_deref(),
        Ok("Keine Divisionen")
    );
    assert_eq!(
        composer.drawn(Variant::Short).as_deref(),
        Ok("MAP_ARROWS_LOCALIZATION_EMPTY_SHORT")
    );
}

#[test]
fn order_label_color_runs_preserve_colors_and_break_kerning() {
    struct Colored;
    impl GlyphMetrics for Colored {
        fn advance(&self, ch: char) -> Option<f32> {
            Fixed.advance(ch)
        }
        fn line_height(&self) -> f32 {
            Fixed.line_height()
        }
        fn base(&self) -> f32 {
            Fixed.base()
        }
        fn glyph(&self, ch: char) -> Option<GlyphBox> {
            Fixed.glyph(ch)
        }
        fn kerning(&self, a: char, b: char) -> f32 {
            if (a, b) == ('A', 'V') { -4.0 } else { 0.0 }
        }
        fn text_color(&self, code: char) -> Option<[f32; 4]> {
            (code == 'G').then_some([0.2, 0.8, 0.3, 1.0])
        }
    }
    impl LabelFont for Colored {
        fn page(&self, _: char) -> Option<usize> {
            Some(0)
        }
    }
    let run = centred_run(&Colored, "A\u{11}GV\u{11}!A", 1.0);
    assert_eq!(run.len(), 3);
    assert_eq!(run[0].color, None);
    assert_eq!(
        run[1].color,
        Some(viewer_native_gui::native_text::packed([0.2, 0.8, 0.3, 1.0]))
    );
    assert_eq!(run[2].color, None);
    let pair = centred_run(&Colored, "A\u{11}GV\u{11}!", 1.0);
    assert_eq!(pair.iter().map(|g| g.left).collect::<Vec<_>>(), [-0.5, 0.0]);
}
