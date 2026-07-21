//! U1 alerts-strip binding + layout gates (REPLAY_PRODUCTION_PLAN §6a).
//!
//! Binding gates drive the packed snapshot-flavor fixture through the real
//! `replay_core` `ReplaySource`/`WorldTimeline` and assert that
//! `game_ui::alerts::alerts_strip_from_frame` produces the exact generator
//! alert counts (present/absent per the FIT zero-suppression convention) at
//! every probed hour and POV, and that an unavailable view / absent POV renders
//! the fail-closed absent form (never a default `0`).
//!
//! Expected values are transcribed from the fixture GENERATOR SOURCE
//! (`replay_fixture_gen/src/main.rs` `fx_topbar_scalars_sweep`), NOT by running
//! the code under test:
//!   initiative = tick % 3, decisions_available = tick % 4,
//!   intel = tick % 2, market_contracts = tick % 5   (identical for every tag).
//!
//! The layout gate parses `interface/topbar.gui` + `interface/alerts.gui`
//! through the shared `game_ui` parser and asserts the four alert count-badge
//! elements + the feed window/sprite resolve (skipped when the game root is
//! absent, matching `topbar::topbar_rects_follow_engine_layout_law`).

use game_ui::alerts::{
    alert_feed_binding, alerts_strip_from_frame, AlertKind, ALERTS_GUI, ALERT_BADGE_BG_SPRITE,
    ALERT_FEED_ICON_SPRITE, ALERT_FEED_WINDOW,
};
use replay_core::{open_recording, WorldTimeline};
use std::path::PathBuf;

fn fixture(name: &str) -> WorldTimeline {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/positive")
        .join(format!("{name}.battleplan"));
    let source = open_recording(&path)
        .unwrap_or_else(|err| panic!("open {name}: {err} ({})", path.display()));
    WorldTimeline::open(source).unwrap_or_else(|err| panic!("timeline {name}: {err}"))
}

// --- generator formulas (transcribed from fx_topbar_scalars_sweep) ---
fn expected_count(kind: AlertKind, tick: i64) -> Option<i64> {
    match kind {
        AlertKind::Initiative => Some(tick % 3),
        AlertKind::DecisionsAvailable => Some(tick % 4),
        AlertKind::Intel => Some(tick % 2),
        AlertKind::MarketContracts => Some(tick % 5),
        AlertKind::DecisionsTimeout
        | AlertKind::IntelPreparedOrCompleted
        | AlertKind::PrototypeRewards
        | AlertKind::MarketContractsOngoing => None,
    }
}

/// GATE 1 — every alert badge renders == the generator count at every probed
/// hour for multiple POVs, and the FIT badge_shown / count_text follow the
/// documented convention (shown, with raw text, exactly when count > 0).
#[test]
fn alert_counts_render_generator_values_per_pov() {
    let tl = fixture("fx_topbar_scalars_sweep");
    let hours = tl.hours().to_vec();
    assert_eq!(hours.len(), 12);

    for (t_idx, &hour) in hours.iter().enumerate() {
        let t = t_idx as i64;
        let frame = tl.frame(hour).unwrap();
        for tag in ["GER", "ENG", "SOV"] {
            let strip = alerts_strip_from_frame(&frame, tag);
            assert!(strip.available, "{tag} extras view present @t{t}");
            assert_eq!(strip.badges.len(), 8, "{tag} eight badges @t{t}");

            for kind in AlertKind::ALL {
                let exp = expected_count(kind, t);
                let badge = strip.badge(kind);
                assert_eq!(badge.count, exp, "{tag} {:?} count @t{t}", kind);
                assert_eq!(
                    badge.badge_shown,
                    exp.is_some_and(|count| count > 0),
                    "{tag} {:?} badge_shown @t{t} (count {:?})",
                    kind,
                    exp
                );
                assert_eq!(
                    badge.count_text.as_deref(),
                    exp.filter(|count| *count > 0).map(|count| count.to_string()).as_deref(),
                    "{tag} {:?} count_text @t{t}",
                    kind
                );
                assert_eq!(
                    badge.gui_element,
                    kind.gui_element(),
                    "{tag} {:?} FILE element @t{t}",
                    kind
                );
            }
        }
    }
}

/// GATE 2 — the probed sweep endpoint is exact. Generator writes
/// `market_contracts = tick % 5`; at t=11 that is 1 (matches the fixture probe
/// `countries.GER.scalars.alerts.market_contracts == 1`).
#[test]
fn market_contracts_probe_endpoint_is_exact() {
    let tl = fixture("fx_topbar_scalars_sweep");
    let hours = tl.hours().to_vec();
    let frame = tl.frame(hours[11]).unwrap();
    let strip = alerts_strip_from_frame(&frame, "GER");
    let badge = strip.badge(AlertKind::MarketContracts);
    assert_eq!(badge.count, Some(1));
    assert!(badge.badge_shown);
    assert_eq!(badge.count_text.as_deref(), Some("1"));

    // t=0: all four categories are zero → every badge suppressed (present/absent
    // coverage) while the view is still available.
    let f0 = tl.frame(hours[0]).unwrap();
    let s0 = alerts_strip_from_frame(&f0, "GER");
    assert!(s0.available);
    assert!(
        s0.badges
            .iter()
            .all(|b| b.count.is_none_or(|count| count == 0) && !b.badge_shown),
        "t=0 all measured badges are zero and all badges are suppressed"
    );
}

/// GATE 3 — a POV absent from an otherwise-present frame renders an all-absent
/// strip (`count: None`, never defaulted to `0`), while the view still reports
/// available for the frame.
#[test]
fn absent_pov_renders_fail_closed_strip() {
    let tl = fixture("fx_topbar_scalars_sweep");
    let frame = tl.frame(tl.first_hour()).unwrap();
    let strip = alerts_strip_from_frame(&frame, "USA"); // not in the fixture
    assert_eq!(strip.pov_tag, "USA");
    assert_eq!(strip.badges.len(), 8);
    for badge in &strip.badges {
        assert_eq!(badge.count, None, "{:?} fail-closed None", badge.kind);
        assert!(!badge.badge_shown);
        assert_eq!(badge.count_text, None);
    }
}

/// GATE 4 — the scrolling alert feed is fail-closed absent (recording-Absent
/// stream; data gap filed in EXTRACTION_NEEDS §A).
#[test]
fn alert_feed_is_fail_closed_absent() {
    let feed = alert_feed_binding();
    assert!(!feed.available);
    assert!(feed.icons.is_empty());
}

// ===========================================================================
// Layout gate — shared game_ui parser resolves the alert elements + feed window
// ===========================================================================

mod layout_gate {
    use super::*;
    use game_ui::alerts::TOPBAR_GUI;
    use game_ui::assets::AssetResolver;
    use game_ui::compose::{resolve_element_rect, ComposeOptions, Rect};
    use game_ui::gui::LayoutSet;
    use game_ui::paradox_text::parse_file;
    use game_ui::probe::DEFAULT_GAME_ROOT;
    use game_ui::topbar::{resolve_topbar_rects_pure, TopbarLayout, TOPBAR_WINDOW_NAME};

    fn hoi4_root() -> Option<PathBuf> {
        let root = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_ROOT));
        root.is_dir().then_some(root)
    }

    fn load_set(resolver: &AssetResolver, rel: &str) -> LayoutSet {
        let resolved = resolver
            .resolve_file(rel)
            .unwrap_or_else(|err| panic!("resolve {rel}: {err}"));
        let doc = parse_file(&resolved.path).unwrap_or_else(|err| panic!("parse {rel}: {err}"));
        let mut set = LayoutSet::default();
        set.add_document(&resolved.rel_path, &doc);
        set
    }

    /// GATE L1 — the four topbar alert count-badge elements resolve to a rect
    /// through the shared parser + engine layout law, and the shared badge
    /// background sprite is cited by `topbar.gui`.
    #[test]
    fn topbar_alert_badges_resolve_via_shared_parser() {
        let Some(root) = hoi4_root() else {
            eprintln!("SKIP alerts layout gate: the game root missing");
            return;
        };
        let resolver = AssetResolver::new(vec![root]);
        let set = load_set(&resolver, TOPBAR_GUI);
        let root_el = set
            .documents
            .iter()
            .flat_map(|d| d.roots.iter())
            .find(|r| {
                r.name.eq_ignore_ascii_case(TOPBAR_WINDOW_NAME)
                    && r.element_type.eq_ignore_ascii_case("containerWindowType")
            })
            .cloned()
            .expect("topbar containerWindowType");
        let layout = TopbarLayout {
            source_file: TOPBAR_GUI.to_string(),
            window_name: TOPBAR_WINDOW_NAME.to_string(),
            root: root_el,
            viewport_width: 1920,
            viewport_height: 1080,
        };
        let rects = resolve_topbar_rects_pure(&layout, 1920, 1080);

        for kind in AlertKind::ALL {
            let name = kind.gui_element();
            let rect = rects
                .iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("alert element `{name}` ({kind:?}) not resolved"));
            // A resolved instantTextBoxType has a zero natural size but a valid
            // anchored origin — the layout law composed it (non-degenerate path).
            println!("{kind:?} -> {name} @ ({},{})", rect.x, rect.y);
        }

        // The shared alert-badge background sprite is cited by topbar.gui.
        assert!(
            set.sprite_refs().contains(ALERT_BADGE_BG_SPRITE),
            "topbar.gui must cite {ALERT_BADGE_BG_SPRITE}"
        );
    }

    /// GATE L2 — the alerts feed window (`global_alerticon_window`) parses,
    /// cites its feed icon sprite, and resolves to its declared 43x38 rect.
    #[test]
    fn alerts_feed_window_resolves_via_shared_parser() {
        let Some(root) = hoi4_root() else {
            eprintln!("SKIP alerts feed layout gate: the game root missing");
            return;
        };
        let resolver = AssetResolver::new(vec![root]);
        let set = load_set(&resolver, ALERTS_GUI);

        let window = set.find_by_name(ALERT_FEED_WINDOW);
        let window = *window
            .first()
            .unwrap_or_else(|| panic!("{ALERT_FEED_WINDOW} not found in {ALERTS_GUI}"));

        // The feed icon sprite is cited by the feed window's button.
        assert!(
            set.sprite_refs().contains(ALERT_FEED_ICON_SPRITE),
            "{ALERTS_GUI} must cite {ALERT_FEED_ICON_SPRITE}"
        );

        // Resolve the window rect through the shared law: size = 43x38 (FILE).
        let options = ComposeOptions::new(1920, 1080).with_root_size(1920, 1080);
        let parent = Rect::new(0, 0, 1920, 1080);
        let rect = resolve_element_rect(window, parent, (0, 0), &options);
        assert_eq!((rect.w(), rect.h()), (43, 38), "feed window size (FILE)");
    }
}
