//! U2 event-chrome gates (REPLAY_PRODUCTION_PLAN §6a / UI_COVERAGE_MATRIX §3).
//!
//! Two FILE-derived gates against the shipped install (skipped when absent):
//!   1. `eventwindow.gui` layout — the chrome rects/sprites/fonts each event
//!      window variant declares (byte-exact vs the shipped `.gui`).
//!   2. Event-def → window resolution — every `events/*.txt` definition's
//!      localisation keys + picture GFX resolved against the loaded
//!      localisation + sprite registry, measuring the renderable rate.
//!
//! Expected values are transcribed from the shipped files
//! (`interface/eventwindow.gui`), not produced by the code under test.

use game_ui::assets::AssetResolver;
use game_ui::event_chrome::{
    event_option_entry_chrome, event_window_chrome, load_event_layout, load_events,
    resolve_events, EVENT_WINDOW_VARIANTS,
};
use game_ui::gfx::SpriteRegistry;
use game_ui::loc::Localisation;
use game_ui::paradox_text::parse_file;
use std::path::PathBuf;

const DEFAULT_GAME_ROOT: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

fn game_root() -> Option<PathBuf> {
    let root = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_ROOT));
    root.is_dir().then_some(root)
}

fn resolver(root: PathBuf) -> AssetResolver {
    AssetResolver::new(vec![root])
}

fn pair(x: &str, y: &str) -> Option<(String, String)> {
    Some((x.to_string(), y.to_string()))
}

// ---------------------------------------------------------------------------
// GATE 1 — eventwindow.gui layout gates (byte-exact vs shipped .gui)
// ---------------------------------------------------------------------------

#[test]
fn eventwindow_declares_all_four_variants_and_option_row() {
    let Some(root) = game_root() else {
        eprintln!("SKIP eventwindow_declares_all_four_variants_and_option_row: the game root missing");
        return;
    };
    let layout = load_event_layout(&resolver(root)).expect("parse eventwindow.gui");

    // All four window variants + the shared option-row template resolve.
    for name in EVENT_WINDOW_VARIANTS {
        assert!(
            event_window_chrome(&layout, name).is_some(),
            "event window variant {name} missing"
        );
    }
    let (entry, name) = event_option_entry_chrome(&layout).expect("event_option_entry");
    assert_eq!(entry.size, pair("300", "47"), "option row size");
    assert_eq!(name.font.as_deref(), Some("hoi_20bs"), "option name font");
    assert_eq!(name.position, pair("5", "0"), "option name position");
    assert_eq!(name.format.as_deref(), Some("center"), "option name format");
}

#[test]
fn default_event_window_chrome_matches_shipped_rects() {
    let Some(root) = game_root() else {
        eprintln!("SKIP default_event_window_chrome_matches_shipped_rects: the game root missing");
        return;
    };
    let layout = load_event_layout(&resolver(root)).expect("parse eventwindow.gui");
    let chrome = event_window_chrome(&layout, "EventWindow").expect("EventWindow");

    assert_eq!(chrome.window_size, pair("581", "427"));
    assert_eq!(
        chrome.background_sprite.as_deref(),
        Some("GFX_tiled_window_transparent")
    );

    // Title text-box (top_Window/Title).
    let title = chrome.title.expect("Title");
    assert_eq!(title.element_type, "instantTextBoxType");
    assert_eq!(title.position, pair("14", "70"));
    assert_eq!(title.font.as_deref(), Some("hoi4_typewriter22"));
    assert_eq!(title.format.as_deref(), Some("centre"));
    assert_eq!(title.max_width.as_deref(), Some("551"));

    // Description text-box (midsection/Description).
    let desc = chrome.description.expect("Description");
    assert_eq!(desc.position, pair("31", "0"));
    assert_eq!(desc.font.as_deref(), Some("hoi4_typewriter16"));
    assert_eq!(desc.format.as_deref(), Some("left"));
    assert_eq!(desc.max_width.as_deref(), Some("512"));

    // Event picture icon (bottom_Window/event_picture).
    let pic = chrome.picture.expect("event_picture");
    assert_eq!(pic.element_type, "iconType");
    assert_eq!(pic.position, pair("5", "5"));
    assert_eq!(pic.sprite.as_deref(), Some("GFX_report_event_001"));

    // Options grid (bottom_Window/options_grid).
    let grid = chrome.options_grid.expect("options_grid");
    assert_eq!(grid.element_type, "gridBoxType");
    assert_eq!(grid.position, pair("215", "-1"));
    assert_eq!(grid.size, pair("300", "30"));
}

#[test]
fn news_event_window_chrome_matches_shipped_rects() {
    let Some(root) = game_root() else {
        eprintln!("SKIP news_event_window_chrome_matches_shipped_rects: the game root missing");
        return;
    };
    let layout = load_event_layout(&resolver(root)).expect("parse eventwindow.gui");
    let chrome = event_window_chrome(&layout, "EventWindow_News").expect("EventWindow_News");

    assert_eq!(chrome.window_size, pair("528", "595"));
    assert_eq!(chrome.background_sprite.as_deref(), Some("GFX_event_news_bg"));

    // News window uses the news picture sprite default + centre title.
    let pic = chrome.picture.expect("news event_picture");
    assert_eq!(pic.sprite.as_deref(), Some("GFX_news_event_001"));
    let title = chrome.title.expect("news Title");
    assert_eq!(title.format.as_deref(), Some("centre"));
    assert_eq!(title.max_width.as_deref(), Some("490"));
}

// ---------------------------------------------------------------------------
// GATE 2 — event-def → window resolution rate (loc + gfx joins)
// ---------------------------------------------------------------------------

/// The full corpus parses, every visible event maps to a chrome window, and a
/// strong majority resolve to a fully renderable window (title + desc +
/// picture + option all resolve). Structural invariants are exact; the
/// renderable rate is floored (install version drift is expected to only add
/// events) and the precise counts are printed for the LANE_REPORT.
#[test]
fn event_def_to_window_resolution_rate() {
    let Some(root) = game_root() else {
        eprintln!("SKIP event_def_to_window_resolution_rate: the game root missing");
        return;
    };
    let resolver = resolver(root);
    let corpus = load_events(&resolver).expect("load events");
    let loc = Localisation::load_english(&resolver).expect("load localisation");

    // Build the sprite registry from every interface/*.gfx (event pictures live
    // across several .gfx files).
    let mut registry = SpriteRegistry::new();
    for file in resolver
        .list_files_recursive("interface", Some("gfx"))
        .expect("list gfx")
    {
        if let Ok(doc) = parse_file(&file.path) {
            registry.add_document(&file.rel_path, &doc);
        }
    }

    let report = resolve_events(&corpus, &loc, &registry);

    // --- structural invariants (exact) ---
    assert!(corpus.parse_failures.is_empty(), "event parse failures: {:?}", corpus.parse_failures);
    assert_eq!(
        report.total,
        report.hidden + report.visible,
        "total = hidden + visible"
    );
    assert_eq!(
        report.visible_by_kind.iter().sum::<usize>(),
        report.visible,
        "per-kind visible sums to visible"
    );
    assert_eq!(
        report.renderable_by_kind.iter().sum::<usize>(),
        report.renderable,
        "per-kind renderable sums to renderable"
    );
    assert!(report.renderable <= report.visible);
    assert!(report.title_resolved <= report.visible);
    assert!(report.desc_resolved <= report.visible);
    assert!(report.options_resolved_any <= report.visible);
    assert!(report.picture_resolved <= report.picture_declared);

    // --- magnitude floors (install-version-robust) ---
    assert!(
        report.total >= 6000,
        "expected the full vanilla event corpus (>=6000), got {}",
        report.total
    );
    assert!(
        report.visible_by_kind[1] >= 600,
        "expected >=600 visible news events, got {}",
        report.visible_by_kind[1]
    );
    // Renderable rate over visible events.
    let rate = report.renderable as f64 / report.visible as f64;
    assert!(
        rate >= 0.90,
        "event renderable rate {:.4} below 0.90 floor ({}/{} visible)",
        rate,
        report.renderable,
        report.visible
    );
    // Title/desc/option resolve rates are each near-total.
    assert!(report.title_resolved as f64 / report.visible as f64 >= 0.95);
    assert!(report.desc_resolved as f64 / report.visible as f64 >= 0.95);
    assert!(report.options_resolved_any as f64 / report.visible as f64 >= 0.95);

    eprintln!(
        "EVENT-CHROME RESOLUTION: total={} hidden={} visible={} renderable={} ({:.2}%)\n  \
         title_resolved={} desc_resolved={} options_resolved_any={} \
         picture_declared={} picture_resolved={}\n  \
         visible_by_kind(country,news,state,unitleader,operative)={:?}\n  \
         renderable_by_kind={:?}\n  \
         gfx_files_parsed_sprites={} events_files_parsed={}",
        report.total,
        report.hidden,
        report.visible,
        report.renderable,
        rate * 100.0,
        report.title_resolved,
        report.desc_resolved,
        report.options_resolved_any,
        report.picture_declared,
        report.picture_resolved,
        report.visible_by_kind,
        report.renderable_by_kind,
        registry.len(),
        corpus.files_parsed,
    );
}
