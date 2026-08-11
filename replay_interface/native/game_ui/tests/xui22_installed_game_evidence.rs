use game_ui::assets::AssetResolver;
use game_ui::gui::LayoutSet;
use game_ui::scripted_gui_runtime::load_scripted_gui_registry;
use std::collections::{BTreeMap, BTreeSet};

const EXACT_A729_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

#[derive(Default)]
struct AuthoredRootCensus {
    standard_tooltips: usize,
    delayed_tooltips: usize,
    disabled_tooltips: usize,
    unsupported_tooltip_mechanisms: usize,
    progress_bars: usize,
    grids: usize,
}

fn census_authored_root(element: &game_ui::gui::GuiElement, census: &mut AuthoredRootCensus) {
    for entry in element.raw.fields() {
        let Some(key) = entry.key.as_deref() else {
            continue;
        };
        if key.eq_ignore_ascii_case("pdx_tooltip") {
            census.standard_tooltips += 1;
        } else if key.eq_ignore_ascii_case("pdx_tooltip_delayed") {
            census.delayed_tooltips += 1;
        } else if key.eq_ignore_ascii_case("pdx_disabled_tooltip")
            || key.eq_ignore_ascii_case("pdx_disabled_tooltip_delayed")
        {
            census.disabled_tooltips += 1;
        } else if [
            "context_aware_tooltip",
            "bound_tooltip",
            "tooltip",
            "tooltiptext",
            "delayedtooltiptext",
        ]
        .iter()
        .any(|candidate| key.eq_ignore_ascii_case(candidate))
        {
            census.unsupported_tooltip_mechanisms += 1;
        }
    }
    if element.element_type.eq_ignore_ascii_case("progressbarType") {
        census.progress_bars += 1;
    }
    if element.element_type.eq_ignore_ascii_case("gridBoxType") {
        census.grids += 1;
    }
    for child in &element.children {
        census_authored_root(child, census);
    }
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "XUI22 installed-game Evidence requires the current the game install"
)]
fn xui22_installed_exact_a729_scripted_gui_census_is_complete() {
    let root = replay_test_gates::require_vanilla_game_root();
    let executable = std::fs::read(root.join("hoi4.exe")).expect("read exact-a729 hoi4.exe");
    assert_eq!(
        portable_content_manifest::foundation::sha256_bytes(&executable),
        EXACT_A729_SHA256,
        "XUI22 installed-game census is valid only for exact a729"
    );
    let resolver = AssetResolver::new(vec![root]);
    let registry = load_scripted_gui_registry(&resolver).expect("parse effective scripted GUIs");
    assert!(
        registry.production_admitted(),
        "installed scripted-GUI registry must be exact: {:?}",
        registry.source_issues
    );
    assert_eq!(registry.engine_tolerated_eof_closures.len(), 1);
    assert!(registry.engine_tolerated_eof_closures[0]
        .starts_with("common/scripted_guis/AST_cabinet_trust_scripted_gui.txt:"));
    assert_eq!(registry.files, 17);
    assert_eq!(registry.definitions.len(), 17);
    assert_eq!(
        registry
            .source_issues
            .iter()
            .filter(|issue| issue.contains(".dynamic_lists"))
            .count(),
        0,
        "exact-a729 scripted GUI definitions contain no dynamic_lists declaration"
    );
    assert!(registry.definitions.values().all(|definition| {
        definition
            .context_type
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("decision_category"))
    }));
    assert!(registry
        .definitions
        .values()
        .all(|definition| definition.window_name.is_some()));
    assert_eq!(
        registry
            .definitions
            .values()
            .flat_map(|definition| definition.properties.keys())
            .filter(|key| key.ends_with(".image"))
            .count(),
        31
    );
    assert_eq!(
        registry
            .definitions
            .values()
            .flat_map(|definition| definition.properties.keys())
            .filter(|key| key.ends_with(".frame"))
            .count(),
        51
    );
    assert_eq!(
        registry
            .definitions
            .values()
            .map(|definition| definition.triggers.len())
            .sum::<usize>(),
        4
    );
    assert_eq!(
        registry
            .definitions
            .values()
            .map(|definition| definition.effects.len())
            .sum::<usize>(),
        0
    );

    let window_names = registry
        .definitions
        .values()
        .map(|definition| definition.window_name.clone().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(window_names.len(), 17);
    let mut owners = window_names
        .iter()
        .map(|name| (name.clone(), Vec::<String>::new()))
        .collect::<BTreeMap<_, _>>();
    let mut census = AuthoredRootCensus::default();
    let interface_files = resolver
        .list_files_recursive("interface", Some("gui"))
        .expect("enumerate effective interface GUI files");
    for file in interface_files {
        let mut layouts = LayoutSet::default();
        layouts
            .parse_file(&file.path, &file.rel_path)
            .unwrap_or_else(|error| panic!("parse {}: {error}", file.rel_path));
        for (window_name, files) in &mut owners {
            let roots = layouts.find_by_name(window_name);
            if !roots.is_empty() {
                files.push(file.rel_path.clone());
                for root in roots {
                    census_authored_root(root, &mut census);
                }
            }
        }
    }
    for (window_name, files) in &owners {
        assert_eq!(
            files.len(),
            1,
            "scripted window {window_name:?} must resolve to one effective GUI file: {files:?}"
        );
    }
    let resolved_files = owners
        .values()
        .map(|files| files[0].clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(resolved_files.len(), 17);
    assert_eq!(census.standard_tooltips, 74);
    assert_eq!(census.delayed_tooltips, 0);
    assert_eq!(census.disabled_tooltips, 0);
    assert_eq!(census.unsupported_tooltip_mechanisms, 0);
    assert_eq!(census.progress_bars, 0);
    assert_eq!(census.grids, 0);
}
