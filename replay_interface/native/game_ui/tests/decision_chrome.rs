//! Decisions-panel chrome gates. Generic element geometry follows
//! `replay_interface/documents/UI_LAYOUT_LAW.md` §§3 and 6.1; this suite does
//! not replace XUI19's packet-local production gates.
//!
//! Drives the SHIPPED vanilla files through `game_ui::decision_chrome`:
//!   * the `countrydecisionview.gui` layout parses to the full chrome (root
//!     window + grid + category header + all four card templates, each with an
//!     icon + name element);
//!   * every decision definition (`common/decisions/*.txt`) maps to a card and
//!     its name-loc-key + icon-sprite resolve against the shipped localisation +
//!     sprite registry at the pinned rates (the `N resolve / total` gate).
//!
//! Counts are pinned against the vanilla install. The central evidence gate
//! treats a missing install as an unmet prerequisite, not a passing result;
//! direct libtest discovery marks the individual tests ignored when absent.

use game_ui::assets::AssetResolver;
use game_ui::decision_chrome::{
    parse_decision_layout, parse_decisions, resolve_decisions, CARD_TEMPLATES,
};
use game_ui::loc::Localisation;
use game_ui::probe::parse_gfx_files;
use std::path::PathBuf;

/// The vanilla install root. Panics rather than returning `None`: every
/// caller is gated on `gate_hoi4_install`, so an absent install here is a
/// failure, never a silent skip that libtest would report as a pass.
fn game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
}

fn resolver() -> AssetResolver {
    AssetResolver::new(vec![game_root()])
}

/// GATE U2-LAYOUT — the shipped decisions-panel chrome parses complete: root
/// window, the scrolling grid, the category-tab header, and all four decision
/// card templates, each carrying at least an icon + a name text element.
#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn decision_panel_layout_is_complete() {
    let resolver = resolver();
    let layout = parse_decision_layout(&resolver).expect("parse countrydecisionview.gui");

    assert_eq!(layout.source_file, "interface/countrydecisionview.gui");
    assert_eq!(
        layout.recoveries, 0,
        "vanilla .gui parses with no recoveries"
    );
    assert!(layout.has_root_window, "countrydecisionview window present");
    assert!(layout.has_decision_grid, "decision_grid gridbox present");
    assert!(layout.has_category_header, "category_header present");
    assert_eq!(layout.cards.len(), CARD_TEMPLATES.len());

    for card in &layout.cards {
        assert!(card.present, "card template {} present", card.name);
        assert!(card.has_icon, "card {} has an icon element", card.name);
        assert!(
            card.has_name_text,
            "card {} has a name_text element",
            card.name
        );
    }

    // The plain decision card carries the full binding surface (icon, name,
    // cost/timer, target flag) and its icon defaults to GFX_decision_unknown.
    let decision_item = layout
        .cards
        .iter()
        .find(|c| c.name == "decision_item")
        .expect("decision_item card");
    assert!(decision_item.has_cost_and_timer_text);
    assert!(decision_item.has_target_flag);
    assert_eq!(
        decision_item.icon_default_sprite.as_deref(),
        Some("GFX_decision_unknown")
    );

    assert!(
        layout.is_complete(),
        "decision panel layout gate must be complete"
    );
}

/// GATE U2-RESOLVE — the full decision inventory parses and its
/// name-key/icon-sprite/category resolution hits the pinned vanilla rates.
///
/// Pinned counts are the vanilla ground truth (asserted exactly, like the
/// widget-layout rects). If a game update shifts the corpus these move together
/// and the pinned counts below are re-derived from the new corpus.
#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn decision_definitions_resolve_at_pinned_rates() {
    let resolver = resolver();
    let set = parse_decisions(&resolver).expect("parse decisions");
    let loc = Localisation::load_english(&resolver).expect("load english loc");
    let (registry, _) = parse_gfx_files(&resolver).expect("parse gfx");
    let report = resolve_decisions(&set, &loc, &registry);

    // --- structural invariants (version-robust) ---
    assert_eq!(
        set.category_file_count, PIN_CATEGORY_FILES,
        "category files"
    );
    assert_eq!(
        set.decision_file_count, PIN_DECISION_FILES,
        "decision files"
    );
    assert_eq!(
        set.recoveries, 0,
        "decision scripts parse with no recoveries"
    );
    assert!(
        set.orphan_category_blocks.is_empty(),
        "every decision-file top-level block is a known category; orphans: {:?}",
        set.orphan_category_blocks
    );
    // Every decision's category is a known category def (cross-reference clean).
    assert_eq!(
        report.category_known, report.total_decisions,
        "every decision maps to a known category"
    );
    // Card-template classification covers every decision.
    let template_total: usize = report.template_histogram.values().sum();
    assert_eq!(template_total, report.total_decisions);

    // --- pinned exact counts (the N-resolve/total gate) ---
    assert_eq!(report.total_decisions, PIN_TOTAL_DECISIONS);
    assert_eq!(report.name_key_resolved, PIN_NAME_RESOLVED);
    assert_eq!(report.desc_key_resolved, PIN_DESC_RESOLVED);
    assert_eq!(report.icon_declared, PIN_ICON_DECLARED);
    assert_eq!(report.icon_sprite_resolved, PIN_ICON_RESOLVED);
    assert_eq!(report.total_categories, PIN_TOTAL_CATEGORIES);
    assert_eq!(report.category_name_resolved, PIN_CATEGORY_NAME_RESOLVED);
    assert_eq!(report.category_icon_resolved, PIN_CATEGORY_ICON_RESOLVED);
    assert_eq!(report.category_icon_absent, PIN_CATEGORY_ICON_ABSENT);
    assert_eq!(
        report.template_histogram,
        expected_template_histogram(),
        "card-template classification"
    );

    // The name loc-key and declared-icon resolution rates are high (the naming
    // laws are exact); the residuals are vanilla corpus anomalies, enumerated
    // here: 286 decisions without a name loc entry (dev/debug +
    // dynamic-name decisions), and 77 declared icons whose token does not map to
    // a shipped sprite (double-`decision_` typos, bare `generic`,
    // referenced-but-unshipped sprites) — all degrade to the panel's
    // `GFX_decision_unknown` fail-closed default.
    assert!(
        report.name_key_resolved * 100 >= report.total_decisions * 92,
        "name-key resolution rate >= 92%"
    );
    assert!(
        report.icon_sprite_resolved * 100 >= report.icon_declared * 97,
        "declared-icon resolution rate >= 97%"
    );
}

// --- pinned vanilla ground truth (the game 1.16.x; refreshed if the corpus moves) ---
const PIN_CATEGORY_FILES: usize = 57;
const PIN_DECISION_FILES: usize = 88;
const PIN_TOTAL_DECISIONS: usize = 4125;
const PIN_NAME_RESOLVED: usize = 3839;
const PIN_DESC_RESOLVED: usize = 1373;
const PIN_ICON_DECLARED: usize = 3595;
const PIN_ICON_RESOLVED: usize = 3518;
const PIN_TOTAL_CATEGORIES: usize = 591;
const PIN_CATEGORY_NAME_RESOLVED: usize = 571;
const PIN_CATEGORY_ICON_RESOLVED: usize = 548;
const PIN_CATEGORY_ICON_ABSENT: usize = 37;

fn expected_template_histogram() -> std::collections::BTreeMap<String, usize> {
    std::collections::BTreeMap::from([
        ("decision_item".to_string(), 1313),
        ("targeted_decision_item".to_string(), 782),
        ("timed_decision_item".to_string(), 2030),
    ])
}
