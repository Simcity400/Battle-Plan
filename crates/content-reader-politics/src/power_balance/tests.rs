use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn description_fields_keep_fixed5_range_bounds_and_template_defaults() {
    let read = read_file(b"SYN={initial_value=-0.25 left_side=SYN_LEFT right_side=SYN_RIGHT side={id=SYN_LEFT range={id=SYN_RANGE min=-1 max=0 max=bad}}}", modifiers::UNKNOWN).unwrap();
    let items = read.items[0].value.items().unwrap();
    let side = items
        .iter()
        .find(|item| item.keyword_is("side"))
        .unwrap()
        .value
        .items()
        .unwrap();
    let mut notes = Vec::new();
    let range = range(
        side.iter()
            .find(|item| item.keyword_is("range"))
            .unwrap()
            .value
            .items()
            .unwrap(),
        &mut notes,
        modifiers::UNKNOWN,
    );
    assert_eq!((range.min, range.max), (-100_000, 0));
    assert_eq!((Range::default().min, Range::default().max), (0, 100_000));
    let database = Database::from_parsed([(
        Definition {
            id: b"SYN".to_vec(),
            initial_value: -25_000,
            left: b"SYN_LEFT".to_vec(),
            ..Default::default()
        },
        vec![range],
    )]);
    assert_eq!(database.definition(b"syn").unwrap().initial_value, -25_000);
    assert_eq!(
        database.range(b"syn", b"SYN_RANGE").unwrap().unwrap().min,
        -100_000
    );
}

fn read_file(input: &[u8], registry: modifiers::Registry<'_>) -> Result<native::Read, String> {
    content_reader_script::test_game::enter();
    super::read_file(input, registry, &KeyNames::default())
}

#[test]
fn range_values_and_rule_defaults() {
    let read = read_file(
        br#"@value = 0.25 @tip = TIP balance = { range = { id = @value
        modifier = { first = @value hidden_modifier = { first = 2 } first = 3 }
        rule = { can_create_factions = yes can_join_factions = invalid }
        on_activate = { add_political_power = 1 }
        on_activate = { custom_effect_tooltip = @tip
            effect_tooltip { custom_effect_tooltip = IGNORED }
            effect_tooltip = { custom_effect_tooltip = @tip } }
        on_deactivate = { hidden_effect = { custom_effect_tooltip = HIDDEN } }
    } }"#,
        &|_| Some(true),
    )
    .unwrap();
    let mut notes = Vec::new();
    let range = range(
        read.items[0].value.items().unwrap()[0]
            .value
            .items()
            .unwrap(),
        &mut notes,
        &|_| Some(true),
    );
    assert_eq!(range.id, b"@value");
    assert_eq!(range.modifiers.values[0].value, 300_000);
    assert!(range.modifiers.listed().next().is_none());
    let rule = |name| range.rules[rules().iter().position(|r| r.0 == name).unwrap()];
    assert!(rule("can_create_factions"));
    assert!(!rule("can_join_factions"));
    assert!(rule("can_puppet"));
    assert!(notes.iter().any(|note| note.contains("can_join_factions")));
    assert!(range.activation[0].keyword_is("add_political_power"));
    assert_eq!(range.activation[0].value.scalar(), Some(b"1".as_slice()));
    assert_eq!(range.activation[1].value.scalar(), Some(b"TIP".as_slice()));
    assert!(
        range.activation[2]
            .value
            .items()
            .unwrap_or_default()
            .is_empty()
    );
    let group = range.activation[3].value.items().unwrap();
    assert_eq!(group[0].value.scalar(), Some(b"TIP".as_slice()));
    let hidden = range.deactivation[0].value.items().unwrap();
    assert_eq!(hidden[0].value.scalar(), Some(b"HIDDEN".as_slice()));
}

#[test]
fn unsettled_modifier_and_effect_misreads_are_named() {
    let error = read_file(
        b"balance={range={modifier={unknown={a=1}}}}",
        modifiers::UNKNOWN,
    )
    .unwrap_err();
    assert!(error.contains("unknown") && error.contains("has a block value"));
    let read = read_file(
        b"balance={range={on_activate={x @value=1}}}",
        modifiers::UNKNOWN,
    )
    .unwrap();
    assert!(read.unsettled.is_none());
    assert!(read.misreads.iter().any(|note| note.line == 1
        && note.what.contains("@value")
        && note.what.contains("has no operator")));
}

#[test]
fn ranges_keep_full_effect_scopes_and_operands() {
    content_reader_script::test_game::enter();
    let mut names = KeyNames::default();
    names.countries.insert(b"SYN".to_vec());
    let read = super::read_file(b"balance = { range = { id = scope on_activate = {
        SYN = { if = { limit = { always = yes } add_stability = 0.25 }
            else = { transfer_state = FROM } }
        effect_tooltip = { add_doctrine_cost_reduction = { name = bonus cost_reduction = 0.2 uses = 1 category = land_doctrine } }
    } } }", modifiers::UNKNOWN, &names).unwrap();
    let mut notes = Vec::new();
    let range = range(
        read.items[0].value.items().unwrap()[0]
            .value
            .items()
            .unwrap(),
        &mut notes,
        modifiers::UNKNOWN,
    );
    let scope = range.activation[0].value.items().unwrap();
    let body = scope[0].value.items().unwrap();
    assert!(body[0].keyword_is("limit"));
    assert_eq!(body[1].value.scalar(), Some(b"0.25".as_slice()));
    assert!(scope[1].keyword_is("else"));
    assert_eq!(
        scope[1].value.items().unwrap()[0].value.scalar(),
        Some(b"FROM".as_slice())
    );
    let doctrine = range.activation[1].value.items().unwrap()[0]
        .value
        .items()
        .unwrap();
    assert!(doctrine.iter().any(|item| item.keyword_is("category")
        && item.value.scalar() == Some(b"land_doctrine".as_slice())));
}

#[test]
fn layered_templates_replace_and_root_ranges_precede_sides() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "bop-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let base = root.join("base");
    let overlay = root.join("overlay");
    for folder in [&base, &overlay] {
        std::fs::create_dir_all(folder.join("common/bop")).unwrap();
    }
    std::fs::write(
        base.join("common/bop/a.txt"),
        b"\"BALANCE\"={range={id=old}} discarded={range={id=old}}",
    )
    .unwrap();
    std::fs::write(overlay.join("common/bop/b.txt"), b"balance={side={range={id=same modifier={value=2}}} range={id=same modifier={value=1}}} unnamed={range={modifier={value=7}}}").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&base, &[]), (&overlay, &[])]).unwrap();
    let database = read(&layers, &|_| Some(true)).unwrap();
    assert!(
        database
            .range(b"BALANCE", b"old")
            .unwrap_err()
            .contains("missing authored range old")
    );
    assert_eq!(
        database.balances[b"balance".as_slice()]
            .iter()
            .map(|r| r.modifiers.values[0].value)
            .collect::<Vec<_>>(),
        [100_000, 200_000]
    );
    assert!(
        database
            .range(b"BaLaNcE", b"same")
            .unwrap_err()
            .contains("2 ranges named same")
    );
    assert!(
        database
            .range(b"unnamed", b"")
            .unwrap_err()
            .contains("1 unnamed authored ranges")
    );
    let layers =
        ResolvedLayers::from_folders(&[(&base, &[]), (&overlay, &["common/bop"])]).unwrap();
    assert!(
        read(&layers, &|_| Some(true))
            .unwrap()
            .range(b"discarded", b"old")
            .is_err()
    );
    std::fs::remove_dir_all(root).unwrap();
}
