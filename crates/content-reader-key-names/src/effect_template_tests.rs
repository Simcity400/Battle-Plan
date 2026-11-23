use content_reader_script::{
    native::Read,
    test_game::effect_assertions::{assert_complete_template_read, body, scalar},
};

fn initialize_tables() {
    content_reader_script::test_game::enter();
}

fn read_with_synthetic_units(text: &[u8]) -> Read {
    read_with_synthetic_units_and_widths(text, TEMPLATE_TEST_UNITS, Some([3; 3]))
}
fn read_with_synthetic_units_and_widths(
    text: &[u8],
    units: &[u8],
    widths: Option<[i32; 3]>,
) -> Read {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let root = std::env::temp_dir().join(format!(
        "hoi4-effect-value-template-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let fixture = Fixture(root);
    let write = |path: &str, bytes: &[u8]| {
        let path = fixture.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    };
    write("common/units/effect_test.txt", units);
    write(
        "common/country_tags/effect_test.txt",
        b"AAA = \"history/countries/AAA.txt\"",
    );
    if let Some([line, support, regimental]) = widths {
        let lua = format!(
            "NDefines = {{ NMilitary = {{ MAX_DIVISION_BRIGADE_WIDTH = {line}, MAX_DIVISION_SUPPORT_WIDTH = {support}, MAX_REGIMENTAL_SUPPORT_WIDTH = {regimental} }} }}"
        );
        write("common/defines/effect_test.lua", lua.as_bytes());
    }
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&fixture.0, &[])]).unwrap();
    initialize_tables();
    let names = crate::key_name_products::KeyNameProducts::read(&layers)
        .unwrap()
        .names;
    content_reader_script::test_game::read_effect_body(text, &names)
}
#[test]
fn template_unit_points_accept_the_recorded_line_and_support_categories() {
    let result = read_with_synthetic_units(
        br#"
        body={division_template={name=T
            regiments={fixture_line={0 1}}
            support={fixture_support={x=0 y=1}}
            regimental_support={fixture_support={x=1 y=0}}
        } add_stability=0} final=9
    "#,
    );
    assert_complete_template_read(&result);
    let template = body(body(&result.items, "body"), "division_template");
    let point = body(body(template, "regiments"), "fixture_line");
    assert_eq!(point.len(), 2);
    assert!(point.iter().all(|item| item.key.is_none()));
    assert_eq!(point[0].value.scalar(), Some(b"0".as_slice()));
    assert_eq!(point[1].value.scalar(), Some(b"1".as_slice()));
    assert!(!body(body(template, "support"), "fixture_support").is_empty());
    assert!(!body(body(template, "regimental_support"), "fixture_support").is_empty());
    assert_eq!(scalar(&result.items, "final"), b"9");
}
#[test]
fn template_unit_points_skip_unknown_quoted_and_wrong_category_units() {
    let result = read_with_synthetic_units(
        br#"
        body={division_template={name=T
            regiments={
                "fixture_line"={@quoted=1}
                fixture_missing={@missing=2}
                fixture_support={@wrong_line=3}
                fixture_line={x=0 y=0}
            }
            support={fixture_line={@wrong_support=4}}
            regimental_support={fixture_line={@wrong_regimental=5}}
        } add_stability=0}
        quoted=@quoted missing=@missing wrong_line=@wrong_line
        wrong_support=@wrong_support wrong_regimental=@wrong_regimental
    "#,
    );
    assert_complete_template_read(&result);
    for key in [
        "quoted",
        "missing",
        "wrong_line",
        "wrong_support",
        "wrong_regimental",
    ] {
        assert_eq!(
            scalar(&result.items, key),
            format!("@{key}").as_bytes(),
            "{key}"
        );
    }
    assert_eq!(scalar(body(&result.items, "body"), "add_stability"), b"0");
}
#[test]
fn template_points_do_not_define_variables_and_skip_bad_component_blocks() {
    let result = read_with_synthetic_units(
        br#"
        body={division_template={
            regiments={fixture_line={@raw=7 1}}
            support={fixture_support={"x"={@hidden=4} x=0 y=0}}
            name=T
        } add_stability=0} raw=@raw hidden=@hidden
    "#,
    );
    assert_complete_template_read(&result);
    assert_eq!(scalar(&result.items, "raw"), b"@raw");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    let template = body(body(&result.items, "body"), "division_template");
    let point = body(body(template, "regiments"), "fixture_line");
    assert_eq!(point.len(), 2);
    assert_eq!(point[0].value.scalar(), Some(b"7".as_slice()));
    assert_eq!(point[1].value.scalar(), Some(b"1".as_slice()));
    assert_eq!(scalar(template, "name"), b"T");
}
#[test]
fn colonial_template_reads_its_nested_template_and_skips_inherited_body() {
    let result = read_with_synthetic_units(
        br#"
        body={create_colonial_division_template={subject=AAA division_template={
            name=T inherit={@hidden=4}
            regiments={fixture_line={x=0 y=0}}
        }} add_stability=0} hidden=@hidden
    "#,
    );
    assert_complete_template_read(&result);
    let effects = body(&result.items, "body");
    let colonial = body(effects, "create_colonial_division_template");
    assert_eq!(scalar(colonial, "subject"), b"AAA");
    assert_eq!(scalar(body(colonial, "division_template"), "name"), b"T");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(effects, "add_stability"), b"0");
}
#[test]
fn add_template_units_take_scalar_columns_with_the_live_categories() {
    let result = read_with_synthetic_units(
        br#"
        body={add_units_to_division_template={template_name=T
            regiments={fixture_line=1}
            support={fixture_support=0}
            regimental_support={fixture_support=1}
            unknown={@hidden=7}
        } add_stability=0} hidden=@hidden
    "#,
    );
    assert_complete_template_read(&result);
    let effects = body(&result.items, "body");
    let add = body(effects, "add_units_to_division_template");
    assert_eq!(scalar(add, "template_name"), b"T");
    assert_eq!(scalar(body(add, "regiments"), "fixture_line"), b"1");
    assert_eq!(scalar(body(add, "support"), "fixture_support"), b"0");
    assert_eq!(
        scalar(body(add, "regimental_support"), "fixture_support"),
        b"1"
    );
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(effects, "add_stability"), b"0");
}
#[test]
fn add_template_units_do_not_read_block_columns_as_grid_points() {
    let result = read_with_synthetic_units(
        br#"
        body={add_units_to_division_template={
            regiments={fixture_line={@seen=7 x=0 y=0}
            template_name=T
        } add_stability=0} seen=@seen
    "#,
    );
    assert_complete_template_read(&result);
    let effects = body(&result.items, "body");
    let add = body(effects, "add_units_to_division_template");
    assert_eq!(scalar(body(add, "regiments"), "fixture_line"), b"{");
    assert_eq!(scalar(add, "template_name"), b"T");
    assert_eq!(scalar(&result.items, "seen"), b"7");
    assert_eq!(scalar(effects, "add_stability"), b"0");
}
#[test]
fn add_template_unit_column_errors_follow_the_recorded_width_defines() {
    let result = read_with_synthetic_units_and_widths(
        br#"body={add_units_to_division_template={template_name=T
            regiments={fixture_line={@line=1}}
            support={fixture_support={@support=2}}
            regimental_support={fixture_support={@regimental=3}}
        } add_stability=0} line=@line support=@support regimental=@regimental"#,
        TEMPLATE_TEST_UNITS,
        Some([0; 3]),
    );
    assert_complete_template_read(&result);
    for key in ["line", "support", "regimental"] {
        assert_eq!(scalar(&result.items, key), format!("@{key}").as_bytes());
    }
    assert_eq!(scalar(body(&result.items, "body"), "add_stability"), b"0");
}
#[test]
fn template_support_classification_follows_subunit_type_forms() {
    let units = br#"
        @category = support
        sub_units = {
            numeric_scalar = { type = 16777216 }
            numeric_list = { type = { 16777216 } }
            alias_scalar = { type = @category }
            alias_list = { type = { @category } }
            quoted_value = { type = "support" }
            reset_unit = { type = support }
            repeated_type = { type = infantry type = support }
        }
        sub_units = { reset_unit = { } }
    "#;
    let result = read_with_synthetic_units_and_widths(
        br#"body={division_template={support={
            numeric_scalar={@num_scalar=1}
            numeric_list={@num_list=2}
            alias_scalar={@alias_scalar=3}
            quoted_value={@quoted_value=4}
            reset_unit={@reset_unit=5}
            alias_list={x=0 y=0}
            repeated_type={x=1 y=0}
        } name=T} add_stability=0}
        num_scalar=@num_scalar num_list=@num_list alias_scalar=@alias_scalar
        quoted_value=@quoted_value reset_unit=@reset_unit"#,
        units,
        Some([3; 3]),
    );
    assert_complete_template_read(&result);
    let template = body(body(&result.items, "body"), "division_template");
    let support = body(template, "support");
    for key in [
        "numeric_scalar",
        "numeric_list",
        "alias_scalar",
        "quoted_value",
        "reset_unit",
    ] {
        assert!(body(support, key).is_empty(), "{key}: {result:?}");
    }
    assert_eq!(body(support, "alias_list").len(), 2);
    assert_eq!(body(support, "repeated_type").len(), 2);
    for key in [
        "num_scalar",
        "num_list",
        "alias_scalar",
        "quoted_value",
        "reset_unit",
    ] {
        assert_eq!(scalar(&result.items, key), format!("@{key}").as_bytes());
    }
}
#[test]
fn template_point_overflow_refuses_an_unavailable_component_label_by_name() {
    let result = read_with_synthetic_units(
        b"body={division_template={regiments={fixture_line={0 1 2 3 x=4}}}}",
    );
    let refused = result.unsettled.unwrap();
    assert!(refused.what.contains("fixture_line"), "{refused:?}");
    assert!(
        refused
            .what
            .contains("beyond the initialized x/y/z/w labels"),
        "{refused:?}"
    );
}

const TEMPLATE_TEST_UNITS: &[u8] = br#"
sub_units = {
    fixture_line = { type = infantry }
    fixture_support = { type = support }
}
"#;
