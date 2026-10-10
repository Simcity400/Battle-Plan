use super::*;
use crate::map::names::GlyphBox;
use fixed::SCALE;

/// Every letter is 10 font pixels wide and tall, advancing 10.
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
    // An army-group move with no counted members: order name and group.
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
    // A line never carries the order name.
    let line = compose(super::super::TYPE_LINE, "Army 28", false, 2);
    assert_eq!(line.text(Variant::Full).as_deref(), Ok("2 Divs - Army 28"));
    assert_eq!(line.text(Variant::Middle).as_deref(), Ok("2 Divs"));
    // A group name that is not UTF-8 refuses only the variant using it.
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
    // A unique name that is not UTF-8 is refused, not drawn lossily.
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
    let reads = std::cell::Cell::new(0);
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

    // A native zero count is appended even when the assigned-member count
    // is zero; the suffix is outside the count-dependent composition.
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

    // Unsupported content in the appended suffix is handled by the same
    // map-text refusal as a control in the ordinary label.
    let loc = [(KEY_PARADROP_TRANSPORTS.into(), "\u{14}".into())]
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
    // Width 40 px, pen starts at -20; the space emits nothing.
    assert_eq!(run.len(), 3);
    assert!((run[0].left + 1.0).abs() < 1e-6);
    assert!((run[2].right - 1.0).abs() < 1e-6);
    assert!((run[0].top - 0.75).abs() < 1e-6);
    assert!((run[0].bottom - 0.25).abs() < 1e-6);
}

#[test]
fn the_text_path_is_the_longest_x_run_read_left_to_right() {
    let p = |x: i64, z: i64| [x * SCALE, z * SCALE];
    // Five units west, then a short eastward hook: the westward run is the
    // longer one and is read reversed; the last control is not included.
    let controls = [p(10, 0), p(5, 0), p(0, 0), p(1, 1), p(2, 2)];
    assert_eq!(text_path(&controls), vec![[5.0, 0.0], [10.0, 0.0]]);
    // A one-point backstep between five-point runs (1 < 5 * 0.25) is
    // merged, and the equal-direction runs then join.
    let controls: Vec<Point> = [0, 1, 2, 3, 4, 5, 4, 6, 7, 8, 9, 10]
        .into_iter()
        .map(|x| p(x, 0))
        .collect();
    let path = text_path(&controls);
    assert_eq!(path.len(), 11);
    assert_eq!(path[0], [0.0, 0.0]);
    // Beside three-point runs (0.75 <= 1) the backstep survives, and the
    // longer of the two eastward runs is chosen.
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
    let tried = std::cell::RefCell::new(Vec::new());
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
    // The path is ten units (the last control is not part of it) and the
    // full label, twenty cells of half a unit, is exactly as wide.
    assert_eq!(placed.text, "LONG LONG LONG LABEL");
    // A label that fits in full builds no other variant.
    assert_eq!(tried.take(), [Variant::Full]);
    // Glyphs lie along +x, centred across z = 0, at the sampled height.
    let first = placed.glyphs[0].corners;
    assert!(first[0][2] > 0.0 && first[3][2] < 0.0);
    assert_eq!(first[0][1], 10.0);
    // Too short a path takes the middle variant, then none at all.
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

    // The height callback gets the glyph's segment's control index over
    // the control count, as `RebuildMapArrowTextMesh` passes it.
    let seen = std::cell::RefCell::new(Vec::new());
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

/// Content where the short label holds a text icon and the empty-short
/// key is missing, loaded as the game loads it.
fn modded() -> Localisation {
    crate::map::labels::tests::loaded(concat!(
        "l_english:\n",
        " MAP_ARROWS_LOCALIZATION:0 \"$DIVISIONS$ - $GROUP$\"\n",
        " MAP_ARROWS_LOCALIZATION_EMPTY:0 \"Keine Divisionen\"\n",
        " MAP_ARROWS_DIVISIONS_COUNT:0 \"$DIV$ Div. ¤\"\n",
        " MAP_ARROWS_DIVISIONS_COUNT_SHORT:0 \"£div_icon $DIV$\"\n",
        " MAP_ARROWS_PARATROOPERS_COUNT:0 \"$DIV$ Fj.\"\n",
    ))
}

/// A variant is localised and checked only when the layout reaches it:
/// an unused short variant with an icon does not hide a full label that
/// fits, and a reached one refuses the label by name (1412585E0).
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
    // "4 Fj. - Armée 3": fifteen characters, the `é` outside the test font.
    let long: Vec<Point> = (0..=11).map(p).collect();
    let placed = place(&drawn, &long, &Fixed, 1.0, 0.0, &flat)
        .unwrap()
        .unwrap();
    assert_eq!(placed.text, "4 Fj. - Armée 3");
    // Too short for the full label: the middle variant is reached and
    // holds the 0x14 control, which map text does not draw.
    let short: Vec<Point> = (0..=4).map(p).collect();
    assert_eq!(
        place(&drawn, &short, &Fixed, 1.0, 0.0, &flat).unwrap_err(),
        "it holds the control byte 0x14 (an authored `¤`), which map text does not draw"
    );
}

/// A `MAP_ARROWS_*` key the executable names (14123FF10) and a recorded
/// name token are both copied as their bytes when the content lacks
/// them, as the localiser copies a missing top-level key (142230940).
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
