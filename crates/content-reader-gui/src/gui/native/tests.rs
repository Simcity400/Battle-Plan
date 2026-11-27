use content_reader_script::localization_binding::{BoundValue, FormattedLocalization};
use super::*;
use content_reader_parser::crt::truncate_f32;

fn gui(text: &str) -> Gui {
    read(text.as_bytes())
}

fn window(body: &str) -> Element {
    gui(&format!(
        "guiTypes = {{ containerWindowType = {{ name = \"w\" {body} }} }}\n"
    ))
    .template("w")
    .unwrap()
    .clone()
}

fn names(element: &Element) -> Vec<String> {
    element
        .children
        .iter()
        .map(|child| child.name().into_owned())
        .collect()
}

fn reasons(element: &Element) -> Vec<String> {
    element
        .subtree_refusals()
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn icon<'a>(window: &'a Element, name: &str) -> &'a Icon {
    window.find(Kind::Icon, name).unwrap().icon().unwrap()
}

fn button<'a>(window: &'a Element, name: &str) -> &'a Button {
    window.find(Kind::Button, name).unwrap().button().unwrap()
}

fn text_box<'a>(window: &'a Element, name: &str) -> &'a TextBox {
    window
        .find(Kind::TextBox, name)
        .unwrap()
        .text_box()
        .unwrap()
}

fn grid<'a>(window: &'a Element, name: &str) -> &'a Grid {
    window.find(Kind::Grid, name).unwrap().grid().unwrap()
}

const SYNTHETIC_COMBAT: &str = r#"
    name = "synthetic_combat"
    size = { width = 71 height = 29 } position = { x = -4 y = 6 }
    clipping = no orientation = upper_left
    buttonType = { name = "bg_btn" quadTextureSprite = "fixture_background"
        centerPosition = yes position = { x = 6 y = -12 } Orientation = center }
    buttonType = { name = "frame_btn" spriteType = "fixture_frame"
        centerposition = yes position = { x = 9 y = 5 } }
    gridBoxType = { name = "attacker_adds_grid" slotsize = { width = 7 height = 11 }
        size = { width = 65% height = 80%% } max_slots_horizontal = 3
        position = { x = -8 y = 4 } format = RIGHT }
    iconType = { name = "progress" spriteType = "fixture_progress"
        centerPosition = yes Orientation = center position = { x = 3 y = -2 } }
    instantTextboxType = { name = "count" font = "fixture_font" format = center
        maxWidth = 31 maxHeight = 17 alwaystransparent = yes position = { x = 8 y = 2 } }
    gridBoxType = { name = "attacker_grid" max_slots_horizontal = 2 add_horizontal = no
        format = "UPPER_LEFT" position = { x = -15 y = 7 }
        slotsize = { width = 13 height = 9 } size = { width = 45% height = 60%% } }
"#;

#[test]
fn nested_paths_keep_native_name_order_without_losing_backgrounds() {
    content_reader_script::test_game::enter();
    let gui = read(
        br#"guiTypes={containerWindowType={name=view size={width=20 height=20}
        background={name=Background spriteType=bg}
        iconType={name=duplicate spriteType=first}
        iconType={name=duplicate spriteType=last}
        containerWindowType={name=inner size={width=5 height=5}
            iconType={name=duplicate spriteType=first}
            iconType={name=duplicate spriteType=last}}}}"#,
    );
    let window = gui.template("view").unwrap();
    assert!(window.path_named("Background").is_some());
    assert!(std::ptr::eq(
        window.path_named("duplicate").unwrap(),
        &raw const window.children[0]
    ));
    let inner = window.path_named("inner").unwrap();
    assert!(std::ptr::eq(
        inner.path_named("duplicate").unwrap(),
        &raw const inner.children[0]
    ));
}

#[test]
fn a_synthetic_window_reads_without_refusal_into_typed_children() {
    content_reader_script::test_game::enter();
    let gui = gui(&format!(
        "guiTypes = {{ containerWindowType = {{ {SYNTHETIC_COMBAT} }} }}\n"
    ));
    let window = gui.template("SYNTHETIC_COMBAT").unwrap();
    assert!(reasons(window).is_empty(), "{:?}", reasons(window));
    assert!(gui.refusals().is_empty());
    let container = window.container().unwrap();
    assert_eq!(container.size.size[0], Coordinate::pixels(71));
    assert_eq!(container.clipping, Some(false));
    assert_eq!(
        names(window),
        [
            "bg_btn",
            "frame_btn",
            "attacker_adds_grid",
            "progress",
            "count",
            "attacker_grid"
        ]
    );
    let bg = button(window, "bg_btn");
    assert_eq!(bg.sprite, b"fixture_background");
    assert_eq!(bg.orientation, Some(Orientation::Center));
    assert!(bg.center_position && bg.enabled);
    assert_eq!((bg.position, bg.frame), ([6, -12], 0));
    let adds = grid(window, "attacker_adds_grid");
    assert_eq!(GridFormat::from_value(adds.format), GridFormat::Right);
    assert_eq!(adds.max_slots, [3, -1]);
    assert!(adds.add_horizontal);
    assert_eq!(
        adds.size.size[1],
        Coordinate {
            value: 80,
            percent: true,
            from_position: true,
            auto: false
        }
    );
    let tiles = grid(window, "attacker_grid");
    assert_eq!(GridFormat::from_value(tiles.format), GridFormat::UpperLeft);
    assert!(!tiles.add_horizontal);
    let count = text_box(window, "count");
    assert_eq!(count.format, TextFormat::Center);
    assert_eq!((count.max_width, count.max_height), (31, 17));
    assert!(count.always_transparent && count.truncate && count.multiline);
    let progress = icon(window, "progress");
    assert_eq!((progress.frame, progress.scale), (1, 1.0));
    assert!(window.find(Kind::Icon, "bg_btn").is_none());
    assert_eq!(window.child_named("BG_BTN").unwrap().kind, Kind::Button);
    assert_eq!(window.children[3].name_bytes(), b"progress");
}

#[test]
fn window_types_list_their_children_and_inherit_as_the_game_does() {
    content_reader_script::test_game::enter();
    let gui = gui("guiTypes = {
         windowType = { name = \"base\" size = { x = 40 y = 20 } position = { x = 5 y = 6 }
           background = \"bg\" horizontalBorder = \"h\" verticalBorder = \"v\" }
         windowType = { name = \"kid\" parent = \"base\" size = { x = 10 y = 0 }
           position = { x = 3 y = 4 } horizontalBorder = \"own\"
           guiButtonType = { name = \"bg\" } background = \"bg\"
           guiButtonType = { name = \"bg\" } multiSpriteButtonType = { name = \"m\" }
           iconType = { name = \"bg\" } }
         windowType = { name = \"full\" fullScreen = yes size = { x = 1 y = 1 } }
         windowType = { name = \"lost\" parent = \"nowhere\" }
         windowType = { name = \"loop\" parent = \"loop\" }
         windowType = { name = \"wide\" parent = \"base\" position = { x = 1 y = 1 }
           size = { x = max_resolution y = 5 }
           textBoxType = { name = \"label\" } }
         }
");
    let kid = gui.template("kid").unwrap().window().unwrap();
    assert_eq!(kid.unlisted, [1]);
    assert_eq!(
        kid.size,
        [WindowExtent::Pixels(40), WindowExtent::Pixels(20)]
    );
    assert_eq!(kid.position, [3, 4]);
    assert_eq!(kid.background, b"bg");
    assert_eq!(kid.horizontal_border, b"own");
    assert_eq!(kid.vertical_border, b"v");
    assert_eq!(
        gui.template("full").unwrap().window().unwrap().size,
        [WindowExtent::MaxResolution { less: 0 }; 2]
    );
    for (name, key) in [("lost", "parent"), ("loop", "parent")] {
        let refused = gui.template(name).unwrap().refusals();
        assert!(
            refused
                .iter()
                .any(|refusal| refusal.key == key && refusal.cause == Cause::Unread),
            "{name}: {refused:?}"
        );
    }
    let wide = gui.template("wide").unwrap().refusals();
    assert!(
        wide.iter().any(|refusal| refusal.key == "size"
            && refusal.cause == Cause::Unrecorded
            && refusal
                .reason
                .starts_with("was read with no load-time GUI viewport given")),
        "{wide:?}"
    );
    assert!(gui.text_box_named(b"label") && !gui.text_box_named(b"Label"));
    let source = "guiTypes = {
         windowType = { name = \"base\" size = { x = 40 y = 20 } position = { x = 5 y = 6 }
           background = \"bg\" horizontalBorder = \"h\" verticalBorder = \"v\" }
         windowType = { name = \"wide\" parent = \"base\" position = { x = 1 y = 1 }
           size = { x = max_resolution y = 5 } }
         }";
    let wide = read_at(source.as_bytes(), [1920, 1080]);
    let wide = wide.template("wide").unwrap();
    assert!(wide.refusals().is_empty(), "{:?}", wide.refusals());
    let window = wide.window().unwrap();
    assert_eq!(
        window.size,
        [
            WindowExtent::MaxResolution { less: 1 },
            WindowExtent::Pixels(5)
        ]
    );
    assert_eq!(
        (
            window.background.as_slice(),
            window.vertical_border.as_slice()
        ),
        (b"bg".as_slice(), b"v".as_slice())
    );
    let narrow = read_at(source.as_bytes(), [1, 1080]);
    let narrow = narrow.template("wide").unwrap();
    assert!(narrow.refusals().is_empty());
    assert_eq!(
        narrow.window().unwrap().size,
        [WindowExtent::Pixels(40), WindowExtent::Pixels(20)]
    );
}

#[test]
fn comments_and_separators_are_the_game_lexer_s() {
    content_reader_script::test_game::enter();
    let window = window(
        "gridBoxType = { name = \"g\" position = { x = 0 y = 0; x = 40\n } }\n\
         iconType = { name=\"i\" frame=3}\n",
    );
    assert_eq!(grid(&window, "g").position.x, Coordinate::default());
    assert_eq!(icon(&window, "i").frame, 3);
}

#[test]
fn an_unknown_key_or_kind_is_skipped_whole() {
    content_reader_script::test_game::enter();
    let window = window(
        "my_mod_key = { name = \"renamed\" iconType = { name = \"inner\" } }\n\
         textBoxType = { name = \"text_sprite\" frame = 7 }\n\
         windowType = { name = \"fixed\" }\n\
         \"iconType\" = { name = \"quoted\" }\n\
         buttonType = { name = \"b\" allwaystransparent = { enabled = no } buttonMesh = x }\n\
         iconType = { name = \"after\" frame = 2 3dposition = { x = 1 frame = 9 } }",
    );
    assert_eq!(window.name(), "w");
    assert_eq!(names(&window), ["b", "after"]);
    assert!(button(&window, "b").enabled);
    assert_eq!(icon(&window, "after").frame, 2);
    assert!(reasons(&window).is_empty(), "{:?}", reasons(&window));
}

#[test]
fn a_block_under_a_scalar_key_is_read_as_further_keys() {
    content_reader_script::test_game::enter();
    let window = window(
        "iconType = { name = \"i\" frame = { name = \"renamed\" scale = 2 } rotation = 1 }\n\
         iconType = { name = \"later\" }",
    );
    let renamed = icon(&window, "renamed");
    assert_eq!((renamed.frame, renamed.scale), (1, 2.0));
    assert_eq!(names(&window), ["renamed"]);
    let sprite = self::window("iconType = { name = \"s\" spriteType = { frame = 4 } }");
    assert_eq!(icon(&sprite, "s").sprite, b"{");
    assert_eq!(icon(&sprite, "s").frame, 4);
    let ignored =
        self::window("iconType = { name = \"d\" dontRender = { frame = 5 } text = { scale = 3 } }");
    assert_eq!(icon(&ignored, "d").frame, 5);
}

#[test]
fn a_key_without_an_operator_reads_the_previous_value() {
    content_reader_script::test_game::enter();
    let window = window(
        "iconType = { name = \"i\" frame = 3 scale \"x\" }\n\
         iconType = { name = \"j\" frame = 9 unknown { frame = 1 } }",
    );
    assert_eq!(icon(&window, "i").scale, 3.0);
    assert_eq!(icon(&window, "j").frame, 9);
}

#[test]
fn script_variables_resolve_in_file_order() {
    content_reader_script::test_game::enter();
    let window = window(
        "@a = 4 @a = 5 @b = @a\n\
         iconType = { name = \"i\" frame = @a scale = \"@a\" rotation = @missing }\n\
         iconType = { name = \"j\" position = { x = @a y = @b } }",
    );
    let first = icon(&window, "i");
    assert_eq!(first.frame, 4);
    assert_eq!(first.scale, 1.0);
    assert_eq!(first.rotation, 0.0);
    assert_eq!(icon(&window, "j").position, [4.0, 4.0]);
}

const SHARED_VARIABLE_RULES: &[(&str, &[(&[u8], &[u8], bool)], (&[u8], bool), (&[u8], bool))] = &[
    (
        "the first definition wins",
        &[(b"a", b"4", false), (b"a", b"5", false)],
        (b"@a", false),
        (b"4", false),
    ),
    (
        "names are case-sensitive",
        &[(b"N", b"1", false)],
        (b"@n", false),
        (b"@n", false),
    ),
    (
        "names are the bytes as authored",
        &[(b"N", b"1", false)],
        (b"@N", false),
        (b"1", false),
    ),
    (
        "a quoted token is never a reference",
        &[(b"a", b"4", false)],
        (b"@a", true),
        (b"@a", true),
    ),
    (
        "an undefined reference keeps its text",
        &[(b"a", b"4", false)],
        (b"@missing", false),
        (b"@missing", false),
    ),
    (
        "a reference takes its definition's quoted flag",
        &[(b"s", b"center", true)],
        (b"@s", false),
        (b"center", true),
    ),
    (
        "a token not starting with @ is its own",
        &[(b"a", b"4", false)],
        (b"a", false),
        (b"a", false),
    ),
];

#[test]
fn variables_agree_with_the_parser_model() {
    content_reader_script::test_game::enter();
    use content_reader_script::native;
    let token = |text: &[u8], quoted: bool| {
        if quoted {
            [b"\"".as_slice(), text, b"\""].concat()
        } else {
            text.to_vec()
        }
    };
    for (rule, definitions, (text, quoted), expected) in SHARED_VARIABLE_RULES {
        let mut source = Vec::new();
        for (name, value, flag) in *definitions {
            source.extend([b"@".as_slice(), name, b" = ", &token(value, *flag), b"\n"].concat());
        }
        source.extend([b"key = ".as_slice(), &token(text, *quoted), b"\n"].concat());
        let mut reader = native::Reader::new(native::lex(&source, native::Mode::KeepMark));
        let mut value = None;
        while reader.next_key() {
            if reader.key.text == b"key" {
                value = Some(reader.value.clone());
            }
        }
        let value = value.unwrap();
        assert_eq!((value.text.as_slice(), value.quoted()), *expected, "{rule}");
    }
    let quoted = window(
        "@q = \"center\" @k = center\n\
         instantTextBoxType = { name = \"t\" format = @q } \
         instantTextBoxType = { name = \"u\" format = @k }",
    );
    assert_eq!(text_box(&quoted, "t").format, TextFormat::Left);
    assert_eq!(text_box(&quoted, "u").format, TextFormat::Center);
    let first = window(
        "iconType = { name = \"i\" frame = @f }\n@f = 3\n\
         iconType = { name = \"j\" frame = @f }",
    );
    assert_eq!(icon(&first, "i").frame, 1);
    assert_eq!(icon(&first, "j").frame, 3);
    let second = window("iconType = { name = \"j\" frame = @f }");
    assert_eq!(icon(&second, "j").frame, 1);
}

#[test]
fn a_bound_skips_what_it_does_not_read_as_the_game_does() {
    content_reader_script::test_game::enter();
    let window = window(
        "@o = {
position = { min = { junk x = 3 } max = { other = { a = { @o } } y = 9 } }",
    );
    let container = window.container().unwrap();
    assert_eq!(container.position.min[0], Coordinate::pixels(3));
    assert_ne!(container.position.max[1], Coordinate::pixels(9));
}

#[test]
fn positions_and_sizes_keep_their_bits() {
    content_reader_script::test_game::enter();
    let window = window(
        "position = { x = 10% x = 7 y = auto min = { x = 3 } step = { y = 99999999999 } }\n\
         size = { width = 50%% height = \"12px\" max = { height = 40% } step = { width = 9 } \
         preserve_aspect_ratio = yes }",
    );
    let container = window.container().unwrap();
    assert_eq!(
        container.position.x,
        Coordinate {
            value: 7,
            percent: true,
            from_position: false,
            auto: false
        }
    );
    assert!(container.position.y.auto && container.position.y.value == 0);
    assert_eq!(container.position.min[0], Coordinate::pixels(3));
    assert_eq!(container.position.step[1], Coordinate::pixels(i32::MAX));
    assert!(container.position.min[1].auto && container.position.max[0].auto);
    let size = container.size;
    assert!(size.size[0].from_position && size.size[0].value == 50);
    assert_eq!(size.size[1], Coordinate::pixels(12));
    assert!(size.max[1].percent && !size.max[1].auto && size.max[0].auto);
    assert_eq!(size.step[0], Coordinate::pixels(9));
    assert!(size.preserve_aspect_ratio);
    assert_eq!(container.show_position, container.position);
}

#[test]
fn short_coordinate_tokens_refuse_the_field_for_good() {
    content_reader_script::test_game::enter();
    for body in [
        "position = { x = \"\" x = 3 }",
        "size = { width = \"%\" width = 36 }",
        "size = { min = { height = \"\" } height = 4 }",
    ] {
        let window = window(body);
        assert_eq!(window.refusals().len(), 1, "{body}");
        assert_eq!(window.refusals()[0].cause, Cause::Unread, "{body}");
    }
    let window = window("position = { x = \"%\" }");
    assert!(window.refusals().is_empty());
    assert!(window.container().unwrap().position.x.percent);
}

#[test]
fn components_end_at_the_end_of_the_text() {
    content_reader_script::test_game::enter();
    for tail in [&b""[..], b"\xff", b"\xff x = 2"] {
        let text = [
            b"guiTypes = { editBoxType = { name=\"e\" size={ 1 ".as_slice(),
            tail,
        ]
        .concat();
        let gui = read(&text);
        let edit = gui.template("e").unwrap().clone();
        let Fields::EditBox(fields) = &edit.fields else {
            panic!()
        };
        assert_eq!(fields.size()[0], 1, "{tail:?}");
    }
}

#[test]
fn coordinates_read_their_value_to_its_nul() {
    content_reader_script::test_game::enter();
    let marked = window("position = { x = 10%\0ignored } size = { width = 10%%\0ignored }");
    let container = marked.container().unwrap();
    assert!(container.position.x.percent);
    assert_eq!(container.position.x.value, 10);
    assert!(container.size.size[0].percent && container.size.size[0].from_position);
    assert_eq!(container.size.size[0].value, 10);
    for body in [
        "position = { x = \0ignored x = 3 }",
        "size = { width = %\0ignored width = 36 }",
    ] {
        let window = window(body);
        assert_eq!(window.refusals().len(), 1, "{body:?}");
        assert_eq!(window.refusals()[0].cause, Cause::Unread, "{body:?}");
    }
}

#[test]
fn numbers_scan_as_the_crt_scans_them() {
    content_reader_script::test_game::enter();
    let window = window(
        "iconType = { name = \"hex\" frame = 0x1F scale = 0x1p3 rotation = -inf }\n\
         iconType = { name = \"octal\" frame = 017 scale = \"  2px\" }\n\
         iconType = { name = \"none\" frame = x scale = 1e }\n\
         iconType = { name = \"huge\" frame = 99999999999 scale = 1e39 }\n\
         instantTextBoxType = { name = \"t\" maxWidth = -1 maxHeight = 4294967296 }",
    );
    let hex = icon(&window, "hex");
    assert_eq!((hex.frame, hex.scale), (31, 8.0));
    assert_eq!(hex.rotation, f32::NEG_INFINITY);
    let octal = icon(&window, "octal");
    assert_eq!((octal.frame, octal.scale), (15, 2.0));
    let none = icon(&window, "none");
    assert_eq!((none.frame, none.scale), (1, 1.0));
    let huge = icon(&window, "huge");
    assert_eq!((huge.frame, huge.scale), (1_215_752_191, f32::INFINITY));
    let text = text_box(&window, "t");
    assert_eq!((text.max_width, text.max_height), (u32::MAX, 0));
    assert!(reasons(&window).is_empty(), "{:?}", reasons(&window));
}

#[test]
fn the_first_child_of_a_name_wins() {
    content_reader_script::test_game::enter();
    let window = window(
        "iconType = { name = \"a\" frame = 2 }\n\
         buttonType = { name = \"A\" }\n\
         iconType = { frame = 3 }\n\
         iconType = { frame = 4 }\n\
         positionType = { position = { x = 1 y = 1 } }\n\
         iconType = { name = \"unnamed_gui_1\" frame = 5 }",
    );
    assert_eq!(
        names(&window),
        ["a", "unnamed_gui_0", "unnamed_gui_1", "unnamed_gui_2"]
    );
    assert_eq!(icon(&window, "a").frame, 2);
    assert!(window.find(Kind::Button, "a").is_none());
    assert_eq!(icon(&window, "unnamed_gui_1").frame, 4);
    assert_eq!(window.children[3].position_type().unwrap().position, [1, 1]);
}

#[test]
fn generated_names_follow_the_load_order() {
    content_reader_script::test_game::enter();
    let mut context = Context::new();
    let first = context.read(
        b"guiTypes = { containerWindowType = { background = { spriteType = \"x\" } \
          iconType = { name = \"a\" } iconType = { name = \"a\" } iconType = { } \
          dropDownBoxType = { name = \"dd\" iconType = { } } } }",
    );
    let second = context.read(b"guiTypes = { iconType = { } }");
    let window = first.template("unnamed_gui_2").unwrap();
    let background = &window.container().unwrap().backgrounds[0];
    assert!(background.name().is_empty());
    assert_eq!(names(window), ["a", "unnamed_gui_0", "dd"]);
    let Fields::DropDown(drop_down) = &window.children[2].fields else {
        panic!();
    };
    assert!(drop_down.inner.name().is_empty());
    assert_eq!(drop_down.inner.children[0].name(), "unnamed_gui_1");
    assert!(second.template("unnamed_gui_3").is_some());
}

#[test]
fn templates_register_in_the_batch_order() {
    content_reader_script::test_game::enter();
    for text in [
        "guiTypes = { iconType = { name = \"x\" frame = 1 } iconType = { name = \"X\" frame = 2 } }",
        "guiTypes = { iconType = { name = \"X\" frame = 2 } iconType = { name = \"x\" frame = 1 } }",
    ] {
        let gui = gui(text);
        assert_eq!(
            gui.template("x").unwrap().icon().unwrap().frame,
            2,
            "{text}"
        );
    }
    assert_eq!(batch_order(3), [1, 0, 2]);
    let later = gui("guiTypes = { iconType = { name = \"t\" frame = 1 } } \
         guiTypes = { iconType = { name = \"T\" frame = 3 } }");
    assert_eq!(later.template("t").unwrap().icon().unwrap().frame, 3);
}

#[test]
fn a_button_reads_every_key_its_parser_reads() {
    content_reader_script::test_game::enter();
    let window = window(
        "buttonType = { name = \"b\" enabled = no origo = \"Main_14\" buttonText = \"KEY\" \
         format = left vertical_alignment = bottom multiline = yes size = { x = 20 y = 10 } \
         borderSize = { x = 2 y = 1 } parent = \"p\" orientation = CENTER_UP \
         orientation = lower_right frame = 2 open_browser = { frame = 3 } \
         clicksound = none shortcut = \"a\" shortcut = \"b\" web_link = \"url\" \
         spriteType = \"s\" upSprite = \"u\" }",
    );
    let b = button(&window, "b");
    assert!(!b.enabled && b.multiline && b.no_click_sound);
    assert_eq!(
        (b.font.as_slice(), b.text.as_slice()),
        (&b"Main_14"[..], &b"KEY"[..])
    );
    assert_eq!(b.format, TextFormat::Left);
    assert_eq!(b.vertical_alignment, VerticalAlignment::Bottom);
    assert_eq!((b.size, b.border_size), ([20, 10], [2, 1]));
    assert_eq!(b.parent, b"p");
    assert_eq!(b.orientation, Some(Orientation::LowerRight));
    assert_eq!(b.frame, 2);
    assert_eq!(b.shortcuts, [b"a".to_vec(), b"b".to_vec()]);
    assert_eq!(
        (b.callback_type.as_slice(), b.callback_argument.as_slice()),
        (&b"web_link"[..], &b"url"[..])
    );
    assert_eq!(b.up_sprite, b"u");
}

#[test]
fn buttons_inherit_their_sprite_in_the_post_load_pass() {
    content_reader_script::test_game::enter();
    let window = window(
        "buttonType = { name = \"base\" spriteType = \"GFX_base\" }\n\
         buttonType = { name = \"mid\" parent = \"BASE\" }\n\
         checkboxType = { name = \"box\" parent = \"mid\" }\n\
         buttonType = { name = \"lost\" parent = \"nowhere\" }\n\
         buttonType = { name = \"loop_a\" parent = \"loop_b\" }\n\
         buttonType = { name = \"loop_b\" parent = \"loop_a\" }\n\
         iconType = { name = \"icon\" }\n\
         buttonType = { name = \"confused\" parent = \"icon\" }",
    );
    assert_eq!(button(&window, "mid").sprite, b"GFX_base");
    let check = window
        .find(Kind::Checkbox, "box")
        .unwrap()
        .button()
        .unwrap();
    assert_eq!(check.sprite, b"GFX_base");
    for (name, text) in [
        ("lost", "crashes"),
        ("loop_a", "hangs"),
        ("confused", "type-confused"),
    ] {
        let refusals = window.child_named(name).unwrap().refusals();
        assert!(
            refusals.iter().any(|refusal| refusal.reason.contains(text)),
            "{name}: {refusals:?}"
        );
    }
    let bg = self::window("background = { name = \"bg\" orientation = center }");
    assert_eq!(
        bg.container().unwrap().backgrounds[0]
            .button()
            .unwrap()
            .orientation,
        None
    );
}

#[test]
fn a_text_box_reads_every_key_its_parser_reads() {
    content_reader_script::test_game::enter();
    let window = window(
        "instantTextBoxType = { name = \"t\" fixedsize = yes truncate = no multiline = no \
         text_color_code = \"R\" borderSize = { x = 3 y = 4 } scrollbarType = \"s\" \
         orientation = center_left vertical_alignment = centre ignore_special_chars = yes \
         size = { x = 99 y = 99 } textureFile = \"x\" text = \"KEY\" format = \"center\" \
         context_aware_text = { localization_key = \"LOC\" name = \"x\" nested = { \
         localization_key = \"N\" } } bound_text = \"IGNORED\" }\n\
         instantTextBoxType = { name = \"u\" text_color_code = RG orientation = \"CENTER_DOWN\" }",
    );
    let t = text_box(&window, "t");
    assert!(t.fixed_size && !t.truncate && !t.multiline && t.ignore_special_chars);
    assert_eq!(t.text_color_code, b'R');
    assert_eq!(t.border_size, [3, 4]);
    assert_eq!(t.orientation, Orientation::UpperLeft);
    assert_eq!(t.format, TextFormat::Left);
    assert_eq!(t.vertical_alignment, VerticalAlignment::Center);
    assert_eq!(t.text, b"KEY");
    assert!(t.context_aware);
    let nodes = &t.bound_text.nodes;
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].key, FormattedLocalization::Key(b"LOC".to_vec()));
    assert_eq!(nodes[0].arguments[1].value, BoundValue::Node(1));
    assert_eq!(nodes[1].key, FormattedLocalization::Key(b"N".to_vec()));
    let u = text_box(&window, "u");
    assert_eq!(u.text_color_code, 0);
    assert_eq!(u.orientation, Orientation::CenterDown);
    assert!(reasons(&window).is_empty(), "{:?}", reasons(&window));
}

#[test]
fn bound_localisations_read_their_tree() {
    content_reader_script::test_game::enter();
    let window = window(
        "iconType = { name = \"a\" bound_tooltip = \"KEY\" }\n\
         iconType = { name = \"b\" context_aware_tooltip = { x = \"token:T\" y = \"fmt|v\" } }\n\
         iconType = { name = \"c\" bound_tooltip = { localization_key = K } frame = 7 }",
    );
    let a = &window.child_named("a").unwrap().common;
    assert_eq!(
        a.bound_tooltip.nodes[0].key,
        FormattedLocalization::Key(b"KEY".to_vec())
    );
    assert!(!a.context_aware_tooltip);
    let b = &window.child_named("b").unwrap().common;
    assert!(b.bound_tooltip.is_empty() && b.context_aware_tooltip);
    assert_eq!(window.child_named("c").unwrap().icon().unwrap().frame, 7);
    assert_eq!(
        FormattedLocalization::parse(b"x token:T"),
        FormattedLocalization::Token(b"T".to_vec())
    );
    assert_eq!(
        FormattedLocalization::parse(b"fmt|v"),
        FormattedLocalization::Formatter {
            formatter: b"fmt".to_vec(),
            token: b"v".to_vec()
        }
    );
}

#[test]
fn a_grid_reads_every_key_its_parser_reads() {
    content_reader_script::test_game::enter();
    let window = window(
        "gridBoxType = { name = \"h\" orientation = \"CENTER_LEFT\" }\n\
         gridBoxType = { name = \"m\" orientation = center max_slots = { 1 2 3 7 } }\n\
         gridBoxType = { name = \"i\" max_slots = { 1 2 3 4 256 } }\n\
         gridBoxType = { name = \"j\" add_horizontal = no max_slots = { 1 2 3 4 5 6 } }\n\
         gridBoxType = { name = \"k\" max_slots = { 1 2 3 4 5 6 7 } }\n\
         gridBoxType = { name = \"g\" format = upper_left orientation = center_left \
         padding = { left = 3 bottom = 2% } max_slots = { 2 y = 3 } \
         background = { name = \"bg\" spriteType = \"GFX_x\" } \
         background = { max_slots_vertical = 9 } }",
    );
    let g = grid(&window, "g");
    assert_eq!(g.format, 0);
    assert_eq!(
        Orientation::from_value(g.orientation),
        Some(Orientation::CenterLeft)
    );
    assert_eq!(g.padding.left, Coordinate::pixels(3));
    assert!(g.padding.bottom.percent);
    assert_eq!(g.max_slots, [2, 9]);
    assert_eq!(
        g.background.as_ref().unwrap().button().unwrap().sprite,
        b"GFX_x"
    );
    let h = grid(&window, "h");
    assert_eq!(
        Orientation::from_value(h.orientation),
        Some(Orientation::Center)
    );
    let m = grid(&window, "m");
    assert_eq!((m.max_slots, m.format, m.orientation), ([1, 2], 3, 7));
    assert!(!grid(&window, "i").add_horizontal);
    let j = window.child_named("j").unwrap();
    assert!(j.grid().unwrap().add_horizontal && j.refusals().is_empty());
    assert_eq!(window.child_named("k").unwrap().refusals().len(), 1);
}

#[test]
fn a_label_past_four_components_is_not_followed() {
    content_reader_script::test_game::enter();
    let gui = gui("guiTypes = { containerWindowType = { name = \"w\"\n\
         gridBoxType = { name = \"g\" max_slots = { 1 2 3 4 q = 5 } } }\n\
         containerWindowType = { name = \"after\" } }\n");
    let window = gui.template("w").unwrap();
    assert!(
        window
            .subtree_refusals()
            .iter()
            .any(|refusal| refusal.cause == Cause::Unaligned)
    );
}

#[test]
fn a_list_reads_every_key_its_parser_reads() {
    content_reader_script::test_game::enter();
    let window = window(
        "OverlappingElementsBoxType = { name = \"l\" format = up format = { spacing = 9 } \
         spacing = 1.5 first_on_top = yes orientation = center_up size = { x = 400 y = 64 } \
         parent = \"p\" }",
    );
    let list = window.find(Kind::List, "l").unwrap().list().unwrap();
    assert_eq!(list.format, ListFormat::Up);
    assert_eq!(list.spacing, 1);
    assert!(list.first_on_top);
    assert_eq!(list.orientation, Orientation::CenterUp);
    assert_eq!(list.size, [400, 64]);
    assert_eq!(list.parent, b"p");
}

#[test]
fn a_container_reads_every_key_its_parser_reads() {
    content_reader_script::test_game::enter();
    let window = window(
        "orientation = \"center\" origo = CENTER_DOWN clipping = maybe scale = 0.5 \
         fade_type = linear fade_time = 0 animation_type = smoothstep \
         hide_animation_type = none verticalScrollbar = \"v\" \
         background = { name = \"bg\" quadTextureSprite = \"GFX_x\" } \
         background_margin = { left = 1 } drag_scroll = { left middle } drag_scroll = right \
         moveable = yes click_to_front = yes autohide_scrollbars = no smooth_scrolling = yes \
         scroll_wheel_factor = 3 show_sound = \"s\" hide_sound = \"h\" \
         peek_check_size = { width = 3 } closed_position = { x = 1 } \
         margin = { left = 4 top = 2 { right = 9 } bottom = 1 }",
    );
    let c = window.container().unwrap();
    assert_eq!(c.orientation, Orientation::UpperLeft);
    assert_eq!(c.origo, Orientation::CenterDown);
    assert_eq!(c.clipping, Some(false));
    assert_eq!(c.scale, 0.5);
    assert_eq!((c.fade, c.fade_time), (Animation::Linear, 0));
    assert!(c.moveable && !c.autohide_scrollbars && c.click_to_front && c.smooth_scrolling);
    assert_eq!(c.show_animation, Animation::Smoothstep);
    assert_eq!(c.hide_animation, Animation::None);
    assert_eq!(c.vertical_scrollbar, b"v");
    assert_eq!(c.drag_scroll, 7);
    assert_eq!(c.scroll_wheel_factor, 3.0);
    assert_eq!(
        (c.show_sound.as_slice(), c.hide_sound.as_slice()),
        (&b"s"[..], &b"h"[..])
    );
    assert!(c.peek);
    assert_eq!(c.closed_position.x, Coordinate::pixels(1));
    assert_eq!(c.peek_check_size.size[0], Coordinate::pixels(3));
    assert_eq!(c.margin.left, Coordinate::pixels(4));
    assert_eq!(c.margin.right, Coordinate::pixels(9));
    assert_eq!(c.margin.bottom, Coordinate::default());
    assert_eq!(c.backgrounds.len(), 1);
    assert_eq!(c.background_margin.left, Coordinate::pixels(1));
    assert!(reasons(&window).is_empty(), "{:?}", reasons(&window));
}

#[test]
fn generic_keys_are_read_on_every_kind() {
    content_reader_script::test_game::enter();
    let window = window(
        "iconType = { name = \"i\" hide = yes pdx_tooltip = \"x\" hint_tag = y \
         pdx_tooltip_anchor = { x = 1 relative = no other = { frame = 3 } } frame = 2 }",
    );
    let element = window.find(Kind::Icon, "i").unwrap();
    assert!(element.hide);
    assert_eq!(element.common.tooltip, reader::fnv64(b"x"));
    assert_eq!(element.common.tooltip_key, b"x");
    assert_eq!(element.common.hint_tag, b"y");
    let anchor = element.common.tooltip_anchor;
    assert!(anchor.set && !anchor.relative && anchor.below && anchor.x == 1);
    assert_eq!(icon(&window, "i").frame, 2);
}

#[test]
fn a_refusal_names_the_template_path() {
    content_reader_script::test_game::enter();
    let window = window("gridBoxType = { name = \"g\" position = { x = \"\" } }");
    let refusal = &window.find(Kind::Grid, "g").unwrap().refusals()[0];
    assert_eq!(
        (refusal.path.as_str(), refusal.key.as_str()),
        ("w.g", "position.x")
    );
}

#[test]
fn valid_native_keys_do_not_poison_later_siblings() {
    content_reader_script::test_game::enter();
    for body in [
        "iconType = { name = \"i\" 3dposition = { 1 2 3 } }",
        "instantTextBoxType = { name = \"t\" bound_text = { a = b } }",
        "iconType = { name = \"i\" context_aware_tooltip = { a = b } }",
        "extendedScrollbarType = { name = \"s\" track = { spriteType = \"x\" } }",
        "dropDownBoxType = { name = \"d\" expandButton = { } hide = yes }",
        "scrollbarType = { name = \"s\" guiButtonType = { name = \"b\" } }",
        "browserType = { name = \"b\" position = { 1 2 } }",
        "curveGraphType = { name = \"c\" size = { 1 } }",
        "smoothListboxType = { name = \"l\" size = { x = 1 y = 2 } }",
        "editBoxType = { name = \"e\" size = { 1 2 } cursor = { x = 1 y = 2 } }",
    ] {
        let gui = gui(&format!(
            "guiTypes = {{ containerWindowType = {{ name = \"w\" {body} }}\n\
             containerWindowType = {{ name = \"after\" }} textBoxType = {{ name = \"tb\" \
             textblock = {{ text = \"x\" }} }} }}\n"
        ));
        for template in gui.templates() {
            assert!(
                reasons(template).is_empty(),
                "{body}: {:?}",
                reasons(template)
            );
        }
        assert!(gui.template("after").is_some(), "{body}");
    }
}

#[test]
fn widget_families_keep_their_fields() {
    content_reader_script::test_game::enter();
    let window = window(
        "listBoxType = { name = \"l\" size = { x = 3 y = 4 } spacing = 5 scrollbar_side = LEFT \
         priority = 2 clipping = no }\n\
         listBoxType = { name = \"p\" parent = \"l\" }\n\
         editBoxType = { name = \"e\" position = { 1 2 3 4 5 6 7 8 9 } only_numbers = yes \
         ignore_enter = maybe is_multiline = no orientation = CENTER cursor = { x = 5 y = 6 } }\n\
         scrollbarType = { name = \"s\" slider_pos_shift = { 1 2 1065353216 } \
         guiButtonType = { name = \"k\" spriteType = \"a\" } buttonType = { name = \"K\" \
         spriteType = \"b\" } maxValue = 5 rangeLimitMin = 1.5 }\n\
         dropDownBoxType = { name = \"d\" expandButton = { name = \"eb\" } \
         iconType = { name = \"inner\" } hide = yes show_position = { frame = 1 } }\n\
         browserType = { name = \"b\" position = { 1 2 3 4 } }\n\
         curveGraphType = { name = \"c\" size = { 1 2 3 } }\n\
         extendedScrollbarType = { name = \"x\" track = { spriteType = \"t\" } \
         track = { lockable = yes } }",
    );
    let l = window.child_named("l").unwrap();
    let Fields::ListBox(list) = &l.fields else {
        panic!()
    };
    assert_eq!(
        (list.size, list.spacing, list.scrollbar_side),
        ([3, 4], 5, 0)
    );
    assert_eq!((list.priority, list.clipping), (Some(2.0), false));
    assert!(
        window.child_named("p").unwrap().refusals()[0]
            .reason
            .contains("never")
    );
    let Fields::EditBox(edit) = &window.child_named("e").unwrap().fields else {
        panic!()
    };
    assert_eq!(
        (edit.position(), edit.size(), edit.border_size()),
        ([1, 2], [3, 4], [5, 6])
    );
    assert_eq!(edit.cursor(), [5, 6]);
    assert_eq!(edit.orientation(), 4);
    assert_eq!(edit.flags(), 0x2 | 0x4 | 0x8);
    assert!(!edit.is_multiline());
    let s = window.child_named("s").unwrap();
    let Fields::Scrollbar(bar) = &s.fields else {
        panic!()
    };
    assert_eq!(bar.slider_pos_shift(), [1, 2]);
    assert_eq!(bar.max_value(), 5.0);
    assert_eq!(bar.range_limit_min, 150_000);
    assert_eq!(s.children.len(), 1);
    assert_eq!(s.children[0].button().unwrap().sprite, b"b");
    let Fields::ExtendedScrollbar(x) = &window.child_named("x").unwrap().fields else {
        panic!()
    };
    assert_eq!(x.track.as_ref().unwrap().button().unwrap().sprite, b"t");
    assert!(x.lockable);
    let d = window.child_named("d").unwrap();
    assert!(!d.hide);
    let Fields::DropDown(drop_down) = &d.fields else {
        panic!()
    };
    assert!(drop_down.inner.hide);
    assert_eq!(drop_down.inner.children[0].name(), "inner");
    assert_eq!(drop_down.expand_button.as_ref().unwrap().name(), "eb");
    let Fields::Browser(browser) = &window.child_named("b").unwrap().fields else {
        panic!()
    };
    assert_eq!((browser.position(), browser.size()), ([1, 2], [3, 4]));
    let c = window.child_named("c").unwrap();
    assert!(c.refusals().is_empty(), "{:?}", c.refusals());
    let Fields::Browser(curve) = &c.fields else {
        panic!()
    };
    assert!(curve.sprite.is_empty());
}

#[test]
fn components_land_in_strings_and_pointers_past_their_fields() {
    content_reader_script::test_game::enter();
    let window = window(
        "browserType = { name = \"a\" spriteType = \"abcdef\" position = { 1 2 3 4 65 } }\n\
         browserType = { name = \"b\" spriteType = \"abc\" position = { 1 2 3 4 0 0 0 0 40 } }\n\
         browserType = { name = \"c\" position = { 1 2 3 4 0 0 0 0 0 0 15 0 9 } }\n\
         scrollbarType = { name = \"s\" slider_pos_shift = { 1 2 3 4 5 6 7 8 9 10 } }\n\
         scrollbarType = { name = \"t\" slider_pos_shift = { 1 2 3 4 5 6 7 8 9 10 11 } \
         buttonType = { name = \"k\" } }",
    );
    let Fields::Browser(a) = &window.child_named("a").unwrap().fields else {
        panic!()
    };
    assert_eq!(a.sprite, b"A\0\0\0ef");
    assert!(reasons(window.child_named("b").unwrap())[0].contains("0x20 bytes"));
    assert!(reasons(window.child_named("c").unwrap())[0].contains("heap overflow"));
    let s = window.child_named("s").unwrap();
    assert!(s.refusals().is_empty(), "{:?}", reasons(s));
    let Fields::Scrollbar(bar) = &s.fields else {
        panic!()
    };
    assert_eq!(bar.slots[7], 8);
    assert!(reasons(window.child_named("t").unwrap())[0].contains("child factory pointer"));
}

#[test]
fn slider_components_reach_the_scrollbar_types_end() {
    content_reader_script::test_game::enter();
    let spill = |values: &[(usize, &str)]| {
        let last = values.iter().map(|(at, _)| (at - 0x1F0) / 4).max().unwrap();
        let mut components = vec!["-"; last + 1];
        for (at, value) in values {
            components[(at - 0x1F0) / 4] = value;
        }
        components.join(" ")
    };
    let window = window(&format!(
        "scrollbarType = {{ name = \"p\" track = \"pt\" slider = \"ps\" size = {{ x = 5 y = 6 }} \
         maxValue = 9 minValue = 1 }}\n\
         scrollbarType = {{ name = \"s\" parent = \"p\" slider_pos_shift = {{ {} }} }}\n\
         scrollbarType = {{ name = \"n\" slider_pos_shift = {{ {} }} }}\n\
         scrollbarType = {{ name = \"o\" slider_pos_shift = {{ {} }} }}\n\
         scrollbarType = {{ name = \"q\" slider_pos_shift = {{ {} }} }}",
        spill(&[
            (0x240, "0"),
            (0x260, "1"),
            (0x274, "1"),
            (0x280, "1"),
            (0x288, "150000"),
            (0x298, "1"),
            (0x29C, "3"),
        ]),
        spill(&[(0x274, "13")]),
        spill(&[(0x274, "14")]),
        spill(&[(0x2A0, "1")]),
    ));
    let s = window.child_named("s").unwrap();
    assert!(s.refusals().is_empty(), "{:?}", reasons(s));
    let Fields::Scrollbar(bar) = &s.fields else {
        panic!()
    };
    assert_eq!((bar.horizontal, bar.listed, bar.scroll_speed), (1, 1, 3));
    assert!(bar.lockable && !bar.drag && bar.use_range_limit);
    assert_eq!(bar.range_limit_min, 150_000);
    assert_eq!(
        (bar.track.as_slice(), bar.slider.as_slice()),
        (&b"pt"[..], &b""[..])
    );
    assert_eq!(bar.size, [0, 0]);
    assert!(window.child_named("n").unwrap().refusals().is_empty());
    assert!(reasons(window.child_named("o").unwrap())[0].contains("listed field ids' count"));
    assert!(reasons(window.child_named("q").unwrap())[0].contains("heap overflow"));
}

#[test]
fn an_extended_scrollbar_background_inherits_through_the_lookup_it_was_given() {
    content_reader_script::test_game::enter();
    let window = window(
        "buttonType = { name = \"p\" spriteType = \"GFX_from_parent\" }\n\
         extendedScrollbarType = { name = \"x\" background = { parent = \"p\" } \
         track = { parent = \"p\" } }\n\
         extendedScrollbarType = { name = \"y\" background = { parent = \"missing\" } }",
    );
    let x = window.child_named("x").unwrap();
    assert!(x.subtree_refusals().is_empty(), "{:?}", reasons(x));
    let Fields::ExtendedScrollbar(bar) = &x.fields else {
        panic!()
    };
    for part in [&bar.background, &bar.track] {
        assert_eq!(
            part.as_ref().unwrap().button().unwrap().sprite,
            b"GFX_from_parent"
        );
    }
    let y = window.child_named("y").unwrap();
    assert!(
        reasons(y)[0].contains("button sprite inheritance"),
        "{:?}",
        reasons(y)
    );
}

#[test]
fn points_read_seven_tokens_through_one_local() {
    content_reader_script::test_game::enter();
    let window = window(
        "iconType = { name = \"a\" position = { y = 3 x = 4 } }\n\
         iconType = { name = \"b\" position = { x = 5 y = q } }\n\
         iconType = { name = \"c\" position = { x = q y = 5 } }\n\
         iconType = { name = \"d\" position = { x = 16777217 y = 2147483647 } }",
    );
    assert_eq!(icon(&window, "a").position, [3.0, 4.0]);
    assert_eq!(icon(&window, "b").position, [5.0, 5.0]);
    assert_eq!(window.child_named("c").unwrap().refusals().len(), 1);
    let d = icon(&window, "d").position;
    assert_eq!(d, [16_777_216.0, 2_147_483_648.0]);
    assert_eq!(truncate_f32(d[1]), i32::MIN);
    let icon_cut = gui(
        "guiTypes = { iconType = { name = \"i\" position = { x = 3 y = 4 } \
                        position = { x = 7 y ",
    );
    assert_eq!(
        icon_cut.template("i").unwrap().icon().unwrap().position,
        [7.0, 0.0]
    );
    let button_cut = gui(
        "guiTypes = { containerWindowType = { name = \"w\" buttonType = { \
                          name = \"b\" position = { x = 3 y = 4 } position = { x = 7 y ",
    );
    let w = button_cut.template("w").unwrap();
    assert_eq!(button(w, "b").position, [7, 4]);
}

#[test]
fn the_stream_ends_as_the_game_ends_it() {
    content_reader_script::test_game::enter();
    let gui = read(
        b"guiTypes = { containerWindowType = { name = \"w\" } \xff guiTypes = { \
                     iconType = { name = \"i\" } } }",
    );
    assert!(gui.template("w").is_some() && gui.template("i").is_some());
    assert!(gui.refusals().is_empty());
    let gui = read(b"guiTypes = { iconType = { name = \"i\" frame = 3");
    assert_eq!(gui.template("i").unwrap().icon().unwrap().frame, 3);
    let gui = self::gui("} guiTypes = { containerWindowType = { name = \"w\" } }\n");
    assert!(gui.template("w").is_none());
    let bom = read(
        b"\xef\xbb\xbfguiTypes = { iconType = { name = \"i\" } }\n\
                    guiTypes = { iconType = { name = \"j\" } }",
    );
    assert!(bom.template("i").is_none() && bom.template("j").is_some());
    let nul = read(b"guiTypes = { iconType = { name = \"a\0b\" } }");
    assert!(nul.template("a").is_some());
}

#[test]
fn instance_lookup_checks_exact_local_position_names() {
    content_reader_script::test_game::enter();
    let gui = self::gui(
        r#"guiTypes = { containerWindowType = { name = root
            positionType = { name = Offset position = { x = 3 y = 4 } }
            iconType = { name = not_a_position }
            containerWindowType = { name = nested
                positionType = { name = nested_offset position = { x = 5 y = 6 } }
            }
        } }"#,
    );
    let root = gui.template("root").unwrap();
    assert_eq!(
        root.find_instance("Offset")
            .unwrap()
            .position_type()
            .unwrap()
            .position,
        [3, 4]
    );
    assert!(root.find_instance("offset").is_none());
    assert!(root.find(Kind::PositionType, "offset").is_none());
    assert!(root.find_instance("nested_offset").is_none());
    assert!(root.find_instance("not_a_position").is_none());
    assert!(root.find_instance(b"Offset\0").is_none());
}

#[test]
fn recursive_lookup_tries_local_kind_then_child_containers_in_order() {
    content_reader_script::test_game::enter();
    let gui = self::gui(
        r#"guiTypes = { containerWindowType = { name = root
        iconType = { name = shared spriteType = local }
        containerWindowType = { name = first
            iconType = { name = shared spriteType = nested }
            instantTextboxType = { name = count font = nested_font }
        }
        containerWindowType = { name = second
            instantTextboxType = { name = count font = second_font }
        }
        windowType = { name = fixed
            iconType = { name = fixed_only spriteType = ignored }
        }
    } }"#,
    );
    let root = gui.template("root").unwrap();
    assert_eq!(
        root.find_recursive(Kind::Icon, "shared")
            .unwrap()
            .icon()
            .unwrap()
            .sprite,
        b"local"
    );
    assert_eq!(
        root.find_recursive(Kind::TextBox, "count")
            .unwrap()
            .text_box()
            .unwrap()
            .font,
        b"nested_font"
    );
    assert!(root.find(Kind::TextBox, "count").is_none());
    assert!(root.find_recursive(Kind::Icon, "fixed_only").is_none());
}

#[test]
fn deep_nesting_is_read_on_the_reader_s_own_stack() {
    content_reader_script::test_game::enter();
    let depth = 3000;
    let mut text = String::from("guiTypes = { ");
    for _ in 0..depth {
        text.push_str("containerWindowType = { name = \"n\" ");
    }
    text.push_str("iconType = { name = \"leaf\" frame = 9 }");
    for _ in 0..depth {
        text.push_str(" }");
    }
    text.push_str(" }");
    let gui = self::gui(&text);
    let mut element = gui.template("n").unwrap();
    let mut levels = 1;
    while let Some(child) = element.find(Kind::Container, "n") {
        element = child;
        levels += 1;
    }
    assert_eq!(levels, depth);
    assert_eq!(icon(element, "leaf").frame, 9);
    assert!(gui.template("n").unwrap().subtree_refusals().is_empty());
}

#[test]
fn files_read_at_once_are_named_as_in_turn() {
    content_reader_script::test_game::enter();
    let files: Vec<Vec<u8>> = (0..40)
        .map(|index| match index % 3 {
            0 => format!("guiTypes = {{ containerWindowType = {{ name = \"w{index}\" iconType = {{ }} iconType = {{ name = \"i\" }} }} containerWindowType = {{ }} }}"),
            1 => format!("guiTypes = {{ containerWindowType = {{ name = \"plain{index}\" }} }}"),
            _ => "guiTypes = { containerWindowType = { buttonType = { } } }".to_owned(),
        }.into_bytes())
        .collect();
    for start in [0, 7] {
        let mut turn = super::Context { unnamed: start };
        let in_turn: Vec<_> = files.iter().map(|file| turn.read(file)).collect();
        let mut once = super::Context { unnamed: start };
        assert_eq!(once.read_all(&files), in_turn);
        assert_eq!(once.unnamed, turn.unnamed);
        assert!(turn.unnamed > start + 20);
    }
}

#[test]
fn viewport_layouts_join_one_build_and_follow_the_content_generation() {
    use content_reader_layers::content::Generation;
    use std::sync::{Arc, Barrier, Mutex};

    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "gui_viewport_product_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let interface = root.join("interface");
    std::fs::create_dir_all(&interface).unwrap();
    std::fs::write(
        interface.join("synthetic_layout.gui"),
        br#"guiTypes = {
            windowType = { name = "shared_base" size = { x = 42 y = 27 }
                position = { x = 2 y = 3 } background = "test_bg" horizontalBorder = "test_h" verticalBorder = "test_v" }
            windowType = { name = "shared_child" parent = "shared_base"
                position = { x = 1 y = 1 } size = { x = max_resolution y = 5 } }
            windowType = { name = "shared_refusal" parent = "undefined_fixture_parent" }
        }"#,
    )
    .unwrap();
    let layers = {
        let _generation = Generation::enter();
        ResolvedLayers::from_folders(&[(&root, &[])]).unwrap()
    };
    let viewport = [901, 603];
    let barrier = Barrier::new(2);
    let read = |viewport| {
        barrier.wait();
        read_layers_at(&layers, viewport).unwrap()
    };
    let (one, two) = std::thread::scope(|scope| {
        let first = content_reader_layers::parallel::spawn(scope, || read(viewport));
        let second = content_reader_layers::parallel::spawn(scope, || read([902, 603]));
        (first.join().unwrap(), second.join().unwrap())
    });
    assert!(Arc::ptr_eq(&one, &two));
    let reads = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&reads);
    let traced = layers.with_read_trace(Arc::new(move |read| seen.lock().unwrap().push(read)));
    let hit = read_layers_at(&traced, viewport).unwrap();
    assert!(Arc::ptr_eq(&one, &hit));
    assert!(reads.lock().unwrap().iter().any(|read| read.path == b"interface/synthetic_layout.gui"));
    assert!(one.template("shared_refusal").unwrap().refusals().iter()
        .any(|refusal| refusal.key == "parent" && refusal.cause == Cause::Unread));
    let other = read_layers_at(&layers, [1, viewport[1]]).unwrap();
    let size = |guis: &Guis| guis.template("shared_child").unwrap().window().unwrap().size;
    assert_ne!(size(&one), size(&other));
    assert_eq!(size(&other)[0], WindowExtent::Pixels(42));
    let builds: Vec<_> = layers
        .product_counts()
        .into_iter()
        .filter(|entry| entry.parser == "GUI layouts at viewport")
        .map(|entry| entry.builds)
        .collect();
    assert_eq!(builds, [1, 1]);
    let refreshed = {
        let _generation = Generation::enter();
        ResolvedLayers::from_folders(&[(&root, &[])]).unwrap()
    };
    let fresh = read_layers_at(&refreshed, viewport).unwrap();
    assert!(!Arc::ptr_eq(&one, &fresh));
    assert_eq!(*one, *fresh);
    let weak = Arc::downgrade(&one);
    drop((one, two, hit));
    assert!(weak.upgrade().is_none());
    drop((layers, traced, refreshed));
    std::fs::remove_dir_all(root).unwrap();
}
