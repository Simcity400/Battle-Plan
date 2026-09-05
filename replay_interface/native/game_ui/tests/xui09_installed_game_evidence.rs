use game_ui::gui::LayoutSet;

fn installed_game_root() -> std::path::PathBuf {
    replay_test_gates::require_vanilla_game_root()
}

fn parse(root: &std::path::Path, relative: &str) -> LayoutSet {
    let mut layouts = LayoutSet::default();
    layouts
        .parse_content(&map_viewer::GameFileResolver::vanilla(&root).unwrap().read_required_bytes(relative, "fixture content").unwrap(), relative)
        .unwrap_or_else(|error| panic!("parse {relative}: {error}"));
    layouts
}

fn require_unique(layouts: &LayoutSet, name: &str) {
    assert_eq!(
        layouts.find_by_name(name).len(),
        1,
        "installed exact-a729 GUI must contain exactly one {name:?}"
    );
}

fn contains_kind(element: &game_ui::gui::GuiElement, kind: &str) -> bool {
    element.element_type.eq_ignore_ascii_case(kind)
        || element.children.iter().any(|child| contains_kind(child, kind))
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn installed_exact_a729_research_gui_exposes_every_shipping_compositor_prototype() {
    let root = installed_game_root();
    let tree = parse(&root, "interface/countrytechtreeview.gui");
    for name in [
        "countrytechtreeview",
        "infantry_folder",
        "industry_folder",
        "techtree_line_item",
        "techtree_xor_item",
        "techtree_infantry_folder_item",
        "techtree_infantry_folder_small_item",
    ] {
        require_unique(&tree, name);
    }
    let infantry = tree
        .find_by_name("infantry_folder")
        .into_iter()
        .next()
        .expect("unique infantry folder");
    assert!(
        contains_kind(infantry, "gridBoxType"),
        "the shipping folder intersection requires an authored gridBoxType"
    );

    let slots = parse(&root, "interface/countrytechnologyview.gui");
    for name in ["research_slots_grid", "research_slot_entry"] {
        require_unique(&slots, name);
    }
}
