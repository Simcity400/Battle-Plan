use super::*;

#[test]
fn targeted_modifiers_keep_the_original_scoped_operand_and_absent_target() {
    for (body, tag, operand) in [
        ("tag=bBb attack=1", Some("BBB"), Some(b"bBb".as_slice())),
        (
            "tag=event_target:MixedCase attack=1",
            None,
            Some(b"event_target:MixedCase".as_slice()),
        ),
        (
            "tag=var:MixedCase tag=global.MixedCase attack=1",
            Some("GLOBAL.MIXEDCASE"),
            Some(b"global.MixedCase".as_slice()),
        ),
        ("attack=1", None, None),
    ] {
        let items = content_reader_parser::parser::parse(body.as_bytes()).unwrap().items;
        let modifier = targeted(&items);
        assert_eq!(modifier.tag.as_deref(), tag);
        assert_eq!(modifier.operand.as_deref(), operand);
        assert_eq!(modifier.modifiers.values[0].value, 100_000);
    }
}

#[test]
fn numeric_notes_skip_blocks_and_show_invalid_scalar_bytes_lossily() {
    let document =
        content_reader_parser::parser::parse(b"block = { } integer = \"\xfe\" fixed = \"\xfe\"").unwrap();
    let mut scans = ScanFailures::default();
    scans.int32("mod_owner", &document.items[0]);
    scans.fixed5("mod_owner", &document.items[0]);
    assert!(scans.warning().is_none());
    scans.int32("mod_owner", &document.items[1]);
    scans.fixed5("mod_owner", &document.items[2]);
    let warning = scans.warning().unwrap();
    assert!(warning.starts_with("2 content values failed"), "{warning}");
    assert!(
        warning.contains("integer = \u{fffd}") && warning.contains("fixed = \u{fffd}"),
        "{warning}"
    );
    assert!(!warning.contains("= {"), "{warning}");
}

fn read(text: &[u8]) -> (Vec<Item>, Vec<String>, Vec<Undecided>) {
    read_with(text, UNKNOWN)
}

fn read_with(text: &[u8], registry: Registry<'_>) -> (Vec<Item>, Vec<String>, Vec<Undecided>) {
    #[derive(Clone, Copy)]
    enum Test {
        Top,
        Modifier(Node),
    }
    let mut undecided = Vec::new();
    let read = native::read(
        text,
        native::Mode::KeepMark,
        native::Top::File(Test::Top),
        None,
        &mut |node, field| match node {
            Test::Top => match field.keyword().as_deref() {
                Some(b"block") => Shape::Object(Test::Modifier(Node::Modifier)),
                Some(b"t") => Shape::Object(Test::Modifier(Node::Targeted)),
                Some(b"e") => Shape::Object(Test::Modifier(Node::EquipmentBonuses)),
                _ => Shape::Skip,
            },
            Test::Modifier(node) => {
                shape(node, field, registry, &mut undecided).map(Test::Modifier)
            }
        },
    );
    let misreads = read.misreads.iter().map(ToString::to_string).collect();
    (read.items, misreads, undecided)
}

fn block<'a>(items: &'a [Item], key: &str) -> &'a [Item] {
    items
        .iter()
        .find(|item| item.key.as_deref() == Some(key.as_bytes()))
        .and_then(|item| item.value.items())
        .unwrap()
}

fn stored_values(modifiers: &Modifiers) -> Vec<(&str, i64)> {
    modifiers
        .values
        .iter()
        .map(|m| (m.name.as_str(), m.value))
        .collect()
}

#[test]
fn values_are_the_crt_conversion_into_a_zeroed_destination() {
    let (items, misreads, undecided) = read(
            b"@cap = 20 @v = 0.099999999999999999 block = { stability_factor = 0.1 command_cap_increase = @cap \
              disable = yes scripted = [?x] unknown = @missing quoted = \"@cap\" prefix = 1tail hex = 0x10 \
              empty = \"\" cut = -0.1234567 fine = @v }",
        );
    assert!(misreads.is_empty() && undecided.is_empty(), "{misreads:?}");
    let block = values(block(&items, "block"));
    assert_eq!(
        stored_values(&block),
        [
            ("stability_factor", 10_000),
            ("command_cap_increase", 2_000_000),
            ("disable", 0),
            ("scripted", 0),
            ("unknown", 0),
            ("quoted", 0),
            ("prefix", 100_000),
            ("hex", 1_600_000),
            ("empty", 0),
            ("cut", -12_345),
            ("fine", 9_999),
        ]
    );
    assert_eq!(block.values[3].text, "[?x]");
    assert_eq!(block.values[4].text, "@missing");
}

#[test]
fn modifiers_merge_hide_and_keep_children_by_identity() {
    let (items, misreads, undecided) = read(
        b"block = { STABILITY_FACTOR = 0.1 war_support_factor = 0.2 \"Stability_Factor\" = 0.3 \
              hidden_modifier = { WAR_SUPPORT_FACTOR = 0.5 } custom_modifier_tooltip = TIP \
              Custom_Modifier_Tooltip = TIP custom_modifier_tooltip = OTHER \
              ADDED_MODIFIER = { @x = 1 a = @x } b = @x }",
    );
    assert!(misreads.is_empty() && undecided.is_empty(), "{misreads:?}");
    let block = values(block(&items, "block"));
    assert_eq!(
        stored_values(&block),
        [
            ("stability_factor", 30_000),
            ("war_support_factor", 50_000),
            ("b", 100_000),
        ]
    );
    assert_eq!(block.values[0].key, "Stability_Factor");
    assert_eq!(
        block.hidden,
        BTreeSet::from(["war_support_factor".to_owned()])
    );
    assert_eq!(
        block.listed().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["stability_factor", "b"]
    );
    let mut tooltips = block.tooltips();
    tooltips.sort();
    assert_eq!(tooltips, ["OTHER", "TIP"]);
    assert_eq!(stored_values(&block.children[0]), [("a", 100_000)]);
}

#[test]
fn a_displacement_chain_past_the_overflow_grows_the_tooltip_table() {
    let keys = [
        "t49495", "t65458", "t59729", "t28854", "t22275", "t87584", "t2909", "t51252", "t65710",
    ];
    let text = keys
        .iter()
        .map(|key| format!("custom_modifier_tooltip = {key}"))
        .collect::<Vec<_>>()
        .join(" ");
    let (items, misreads, undecided) = read(format!("block = {{ {text} }}").as_bytes());
    assert!(misreads.is_empty() && undecided.is_empty());
    let block = values(block(&items, "block"));
    assert_eq!(
        block.tooltips(),
        [
            "t51252", "t65710", "t59729", "t22275", "t87584", "t2909", "t28854", "t49495", "t65458"
        ]
    );
    assert_eq!(block.tooltip_table().mask, 31);
    let mut copied = block
        .tooltip_table()
        .copied()
        .keys()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    copied.sort();
    let mut expected = keys.map(str::to_owned);
    expected.sort();
    assert_eq!(copied, expected);
    assert_eq!(TooltipTable::default().copied(), TooltipTable::default());
}

#[test]
fn targeted_and_equipment_bonuses() {
    let (items, _, _) = read(
            b"t = { tag = sou license_purchase_cost = -0.3 hidden_modifier = { license_purchase_cost = 1 } \
              custom_modifier_tooltip = T } s = { tag = [ROOT.x] a = 1 } v = { tag = event_target:leader a = 1 } \
              e = { ship_hull_heavy = { build_cost_ic = -0.15 instant = yes BUILD_COST_IC = -0.1 } armor = { armor_value = 0.05 } }",
        );
    let t = targeted(block(&items, "t"));
    assert_eq!(t.tag.as_deref(), Some("SOU"));
    assert_eq!(
        stored_values(&t.modifiers),
        [("license_purchase_cost", 100_000)]
    );
    assert!(t.modifiers.listed().next().is_none());
    assert_eq!(t.modifiers.tooltips(), ["T"]);
    assert_eq!(targeted(block(&items, "s")).tag, None);
    assert_eq!(targeted(block(&items, "v")).tag, None);
    let e = equipment_bonuses(block(&items, "e"), &mut Unheld::default());
    let stats: Vec<_> = e[0]
        .values
        .iter()
        .map(|m| (m.name.as_str(), m.value))
        .collect();
    assert_eq!(
        (e[0].equipment.as_str(), e[0].instant, stats),
        ("ship_hull_heavy", true, vec![("build_cost_ic", -25_000)])
    );
    assert_eq!((e[1].equipment.as_str(), e[1].instant), ("armor", false));
}

#[test]
fn block_values_under_modifier_names_are_undecided() {
    let (items, misreads, undecided) =
        read(b"block = { some_modifier = { @m = 1 a = 2 } b = @m x y = 3 } t = { z = { } }");
    assert_eq!(
        undecided
            .iter()
            .map(|found| found.name.as_str())
            .collect::<Vec<_>>(),
        ["some_modifier", "z"]
    );
    assert!(
        undecided[0].to_string().starts_with(
            "line 1: some_modifier has a block value, which modifier value reader reads"
        )
    );
    let b = values(block(&items, "block"));
    let texts: Vec<_> = b
        .values
        .iter()
        .map(|m| (m.key.as_str(), m.text.as_str(), m.value))
        .collect();
    assert_eq!(
        texts,
        [("some_modifier", "{", 0), ("b", "@m", 0), ("x", "@m", 0)]
    );
    assert!(
        misreads[0].contains("key `x` has no operator"),
        "{misreads:?}"
    );
}

#[test]
fn a_registry_decides_block_values_under_modifier_names() {
    let registry: Registry<'_> = &|name| match name {
        b"registered" => Some(true),
        b"unregistered" => Some(false),
        _ => None,
    };
    let keys = |text: &[u8]| {
        let (items, _, undecided) = read_with(text, registry);
        assert!(undecided.is_empty(), "{undecided:?}");
        values(block(&items, "block"))
            .values
            .into_iter()
            .map(|m| m.key)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        keys(b"block = { registered = { a = 1 } b = 2 }"),
        ["registered", "a"]
    );
    assert_eq!(
        keys(b"block = { unregistered = { c = 3 } d = 4 }"),
        ["unregistered", "d"]
    );
    let (_, _, undecided) = read_with(b"e = { armor = { registered = { f = 5 } } }", registry);
    assert_eq!(
        undecided
            .iter()
            .map(|found| (found.name.as_str(), found.reader))
            .collect::<Vec<_>>(),
        [("registered", "equipment stat reader")]
    );
}

#[test]
fn text_the_port_cannot_hold_is_named() {
    let (items, _, _) =
        read(b"block = { a = 1 \"b\xfe\" = 2 custom_modifier_tooltip = \"t\xfe\" c = 3 }");
    let modifiers = values(block(&items, "block"));
    assert_eq!(stored_values(&modifiers), [("a", 100_000), ("c", 300_000)]);
    assert_eq!(
        modifiers.unheld.cause(),
        Some(
            "a modifier key is the stored text `b\\xfe` is not UTF-8, which the port's text cannot hold"
        )
    );
}

#[test]
fn modifier_empty_test_is_lazy_and_counts_hidden_zero_keys() {
    let items = content_reader_parser::parser::parse(b"hidden_modifier={synthetic=0}")
        .unwrap()
        .items;
    let zero = values(&items);
    assert!(!zero.effectively_empty(&mut |_| Ok(true)).unwrap());
    assert!(zero.effectively_empty(&mut |_| Ok(false)).unwrap());
    assert_eq!(
        zero.effectively_empty(&mut |name| Err(format!("missing {name}")))
            .unwrap_err(),
        "missing synthetic"
    );
    for text in [
        b"custom_modifier_tooltip=TIP".as_slice(),
        b"added_modifier={synthetic=0}",
    ] {
        let source = values(&content_reader_parser::parser::parse(text).unwrap().items);
        assert!(
            !source
                .effectively_empty(&mut |_| panic!("eligibility is unreached"))
                .unwrap()
        );
    }
    assert!(
        Modifiers::default()
            .effectively_empty(&mut |_| panic!("no entries"))
            .unwrap()
    );
}

#[test]
fn whole_modifier_copy_reinserts_tooltip_tables_at_every_depth() {
    let source = values(&content_reader_parser::parser::parse(b"hidden_modifier={synthetic=0} custom_modifier_tooltip=A custom_modifier_tooltip=B added_modifier={custom_modifier_tooltip=C synthetic=1}").unwrap().items);
    let copy = source.copied();
    assert_eq!(copy.values, source.values);
    assert_eq!(copy.hidden, source.hidden);
    assert_eq!(copy.tooltip_table(), &source.tooltip_table().copied());
    assert_eq!(
        copy.children[0].tooltip_table(),
        &source.children[0].tooltip_table().copied()
    );
    assert_eq!(copy.children[0].values, source.children[0].values);
}
