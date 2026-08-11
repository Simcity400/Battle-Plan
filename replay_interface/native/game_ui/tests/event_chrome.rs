//! Event-chrome gates. Generic element geometry follows
//! `replay_interface/documents/UI_LAYOUT_LAW.md` §§3 and 6.1; this suite does
//! not replace XUI21's packet-local production gates.
//!
//! Two FILE-derived gates against the shipped install. The central evidence
//! gate treats a missing install as an unmet prerequisite, not a passing
//! result; direct libtest discovery marks the individual tests ignored:
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
    compose_event_popup_surface, event_option_entry_chrome, event_popup_presentation,
    event_window_chrome, load_event_layout, load_events, resolve_events, AdmittedEventOption,
    AdmittedEventPopup, AdmittedFiredEvent, EventPopupSubtype, EventRuntimeImageIdentity,
    EVENT_WINDOW_VARIANTS,
};
use game_ui::compose::UiAssetCache;
use game_ui::gfx::SpriteRegistry;
use game_ui::loc::Localisation;
use game_ui::paradox_text::parse_file;
use game_ui::text_law::{ExactRuntimeText, RuntimeTextContext};
use replay_core::{RuntimeTextLayout, RuntimeTextSource};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The vanilla install root. Panics rather than returning `None`: every
/// caller is gated on `gate_hoi4_install`, so an absent install here is a
/// failure, never a silent skip that libtest would report as a pass.
fn game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
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
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn eventwindow_declares_all_four_variants_and_option_row() {
    let root = game_root();
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
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn default_event_window_chrome_matches_shipped_rects() {
    let root = game_root();
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
    assert_eq!(chrome.options_slot_size, pair("300", "47"));
    assert_eq!(chrome.options_max_slots_horizontal, Some(1));
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn news_event_window_chrome_matches_shipped_rects() {
    let root = game_root();
    let layout = load_event_layout(&resolver(root)).expect("parse eventwindow.gui");
    let chrome = event_window_chrome(&layout, "EventWindow_News").expect("EventWindow_News");

    assert_eq!(chrome.window_size, pair("528", "595"));
    assert_eq!(
        chrome.background_sprite.as_deref(),
        Some("GFX_event_news_bg")
    );

    // News window uses the news picture sprite default + centre title.
    let pic = chrome.picture.expect("news event_picture");
    assert_eq!(pic.sprite.as_deref(), Some("GFX_news_event_001"));
    let title = chrome.title.expect("news Title");
    assert_eq!(title.format.as_deref(), Some("centre"));
    assert_eq!(title.max_width.as_deref(), Some("490"));
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn positive_event_popup_reaches_the_shipping_compositor_with_shipped_assets() {
    let root = game_root();
    let resolver = resolver(root);
    let layout = load_event_layout(&resolver).expect("discover exact shipped event chrome");
    let localisation = Localisation::load_english(&resolver).expect("load shipped localisation");
    let mut registry = SpriteRegistry::new();
    for file in resolver
        .list_files_recursive("interface", Some("gfx"))
        .expect("list shipped GFX")
    {
        registry
            .parse_file(&file.path, &file.rel_path)
            .expect("parse shipped GFX losslessly");
    }
    for required in ["GFX_portrait_unknown", "GFX_intelligence_agency_logo_ger"] {
        assert!(registry.get(required).is_some(), "missing shipped sprite {required}");
    }
    let agency_insignia_path = registry
        .get("GFX_intelligence_agency_logo_ger")
        .and_then(|sprite| sprite.primary_texture())
        .expect("shipped agency insignia has a texture")
        .to_string();
    let operative_text = |element: &str, text: &str| RuntimeTextContext {
        exact: Some(ExactRuntimeText {
            view_id: "EventWindow_Operative".into(),
            element_id: element.into(),
            source: RuntimeTextSource::SetterCallerRva(0x0122_5b30),
            engine_utf8: text.into(),
            layout: RuntimeTextLayout::AuthoredReproducible,
        }),
        ..Default::default()
    };
    let mut cache = UiAssetCache::default();
    for (index, subtype) in [
        EventPopupSubtype::Country,
        EventPopupSubtype::State,
        EventPopupSubtype::News,
        EventPopupSubtype::UnitLeader,
        EventPopupSubtype::OperativeLeader,
    ]
    .into_iter()
    .enumerate()
    {
        let leader = matches!(
            subtype,
            EventPopupSubtype::UnitLeader | EventPopupSubtype::OperativeLeader
        );
        let operative = subtype == EventPopupSubtype::OperativeLeader;
        let event = AdmittedFiredEvent {
            source_file: "events/xui21_positive_compositor_evidence.txt".into(),
            event_id: format!("xui21.positive.{index}"),
            recipient_tag: "ENG".into(),
            fire_date: "1936.1.1.12".into(),
            fire_hour: 12,
            fire_tick: 120,
            event_sequence: index as i64 + 1,
            definition_hidden: false,
            chosen_option_ordinal: None,
            effective_timeout_hours: 24,
            timeout_tick: 144,
            resolved_tick: None,
            resolution_reason: "open".into(),
            runtime_title_text_context_id: Some("ctx:title".into()),
            runtime_description_text_context_id: Some("ctx:description".into()),
            runtime_option_text_context_ids: BTreeMap::from([(0, "ctx:option:0".into())]),
            popup: Some(AdmittedEventPopup {
                subtype,
                window_name: subtype.window_name(),
                title: "Replay event".into(),
                title_runtime_text: RuntimeTextContext::default(),
                description: "Exact positive compositor evidence.".into(),
                description_runtime_text: RuntimeTextContext::default(),
                selected_desc_key: "xui21.positive.desc".into(),
                picture_gfx: None,
                leader_portrait: leader.then(|| {
                    EventRuntimeImageIdentity::GfxName("GFX_portrait_unknown".into())
                }),
                leader_fullname: operative.then(|| {
                    (
                        "Jane Doe".into(),
                        operative_text("leader_fullname", "Jane Doe"),
                    )
                }),
                leader_codename: operative.then(|| {
                    (
                        "Sparrow".into(),
                        operative_text("leader_codename", "Sparrow"),
                    )
                }),
                agency_insignia: operative.then(|| {
                    EventRuntimeImageIdentity::FilePath(agency_insignia_path.clone())
                }),
                options: vec![AdmittedEventOption {
                    definition_ordinal: 0,
                    text: "Observe".into(),
                    runtime_text: RuntimeTextContext::default(),
                    selected: false,
                }],
            }),
            popup_suppression: None,
        };
        let presentation = event_popup_presentation(&event, &layout, "ENG", 120)
            .expect("all exact event subtypes reach presentation");
        let composed = compose_event_popup_surface(
            &resolver,
            &registry,
            &localisation,
            &mut cache,
            &presentation,
            &layout,
            1920,
            1080,
        )
        .unwrap_or_else(|error| {
            panic!("{subtype:?} must reach the shipping authored compositor: {error:?}")
        });
        assert_eq!(composed.event_sequence, index as i64 + 1);
        assert_eq!(composed.root_window, subtype.window_name());
        assert_eq!(composed.option_targets.len(), 1);
        assert!(composed.root_region.width > 0 && composed.root_region.height > 0);
        assert!(composed.output.report.drawn_elements > 0);
    }
}

// ---------------------------------------------------------------------------
// GATE 2 — event-def → window resolution rate (loc + gfx joins)
// ---------------------------------------------------------------------------

/// The full corpus parses, every visible event maps to a chrome window, and a
/// strong majority resolve to a fully renderable window (title + desc +
/// picture + option all resolve). Structural invariants are exact; the
/// renderable rate is floored (install version drift is expected to only add
/// events) and the precise counts are printed on every run.
#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn event_def_to_window_resolution_rate() {
    let root = game_root();
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
    assert!(
        corpus.parse_failures.is_empty(),
        "event parse failures: {:?}",
        corpus.parse_failures
    );
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

/// Measures the shipped event-definition SHAPE census against the real
/// install. This deliberately does not call the result "renderable": runtime
/// localisation/sprite joins are separate authorities from this exact
/// five-class definition-to-window shape check.
#[test]
#[ignore = "requires BATTLE_PLAN_GAME_ROOT, BATTLE_PLAN_GAME_DIR, and BATTLE_PLAN_USER_DIR"]
fn shipped_visible_event_definition_shape_census() {
    let game_root = PathBuf::from(
        std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
            .expect("BATTLE_PLAN_GAME_ROOT must name the real the game install"),
    );
    let game_dir = PathBuf::from(
        std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR"))
            .expect("BATTLE_PLAN_GAME_DIR must name the real the game install"),
    );
    let user_dir = PathBuf::from(
        std::env::var_os("BATTLE_PLAN_USER_DIR").or_else(|| std::env::var_os("HOI4_USER_DIR"))
            .expect("BATTLE_PLAN_USER_DIR must name the real the game user directory"),
    );
    assert!(game_root.is_dir(), "BATTLE_PLAN_GAME_ROOT is not a directory");
    assert!(game_dir.is_dir(), "BATTLE_PLAN_GAME_DIR is not a directory");
    assert!(user_dir.is_dir(), "BATTLE_PLAN_USER_DIR is not a directory");

    let resolver = resolver(game_dir);
    let corpus = load_events(&resolver).expect("parse every shipped events/*.txt definition");
    assert!(
        corpus.parse_failures.is_empty(),
        "event parse failures: {:?}",
        corpus.parse_failures
    );
    assert!(
        corpus.decode_lossy_files.is_empty(),
        "lossy event files: {:?}",
        corpus.decode_lossy_files
    );
    let layout = load_event_layout(&resolver).expect("discover shipped interface GUI definitions");

    let mut visible = 0usize;
    let mut structurally_complete = 0usize;
    let mut missing_mapped_window = 0usize;
    let mut incomplete_mapped_chrome = 0usize;
    let mut missing_title_key = 0usize;
    let mut missing_desc_key = 0usize;
    let mut missing_named_option = 0usize;
    let mut incomplete_by_window = BTreeMap::<String, usize>::new();
    let mut visible_by_window = BTreeMap::<String, usize>::new();
    let window_has = |window: &str, name: &str| {
        let roots = layout.find_by_name(window);
        if roots.len() != 1 {
            return false;
        }
        let mut matches = Vec::new();
        roots[0].find_by_name(name, &mut matches);
        matches.len() == 1
    };
    for def in corpus.defs.iter().filter(|def| !def.hidden) {
        visible += 1;
        *visible_by_window.entry(def.window_name().into()).or_default() += 1;
        let mapped_chrome = event_window_chrome(&layout, def.window_name());
        let has_mapped_window = mapped_chrome.is_some();
        let has_complete_mapped_chrome = mapped_chrome.is_some_and(|chrome| {
            let common = chrome.title.is_some()
                && chrome.description.is_some()
                && chrome.options_grid.is_some();
            let subtype_payload = match def.kind {
                game_ui::event_chrome::EventKind::UnitLeader => {
                    chrome.picture.is_none()
                        && window_has(def.window_name(), "leader")
                }
                game_ui::event_chrome::EventKind::OperativeLeader => {
                    chrome.picture.is_none()
                        && [
                            "leader",
                            "leader_fullname",
                            "leader_codename",
                            "agency_insignia",
                        ]
                        .into_iter()
                        .all(|name| window_has(def.window_name(), name))
                }
                _ => chrome.picture.is_some(),
            };
            common && subtype_payload
        });
        let has_title = def
            .title_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty());
        let has_desc = def.desc_keys.iter().any(|key| !key.trim().is_empty());
        let has_named_option = def.options.iter().any(|option| {
            option
                .name_key
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty())
        });
        missing_mapped_window += usize::from(!has_mapped_window);
        incomplete_mapped_chrome += usize::from(!has_complete_mapped_chrome);
        if !has_complete_mapped_chrome {
            *incomplete_by_window
                .entry(def.window_name().into())
                .or_default() += 1;
        }
        missing_title_key += usize::from(!has_title);
        missing_desc_key += usize::from(!has_desc);
        missing_named_option += usize::from(!has_named_option);
        structurally_complete +=
            usize::from(has_complete_mapped_chrome && has_title && has_desc && has_named_option);
    }

    let rate = structurally_complete as f64 / visible as f64;
    eprintln!(
        "EVENT DEFINITION SHAPE CENSUS: structurally_complete={structurally_complete} \
         visible={visible} rate={:.4}% missing_mapped_window={missing_mapped_window} \
         incomplete_mapped_chrome={incomplete_mapped_chrome} missing_title_key={missing_title_key} \
         missing_desc_key={missing_desc_key} missing_named_option={missing_named_option} \
         visible_by_window={visible_by_window:?} incomplete_by_window={incomplete_by_window:?}",
        rate * 100.0
    );

    // Pinned after measuring the a729-compatible shipped install: 6,745 of
    // 6,757 visible definitions (99.8224%). The 12 failures all lack a title
    // key; no definition lacked its keyword-mapped root, subtype-specific
    // payload slots, description, or named option. Leader/operative chrome is
    // deliberately authenticated by its exact portrait/name/agency slots, not
    // by the `event_picture` slot used by the other windows. This is not proof
    // of the runtime country-event subtype. Keep both count and ratio gates so
    // additions cannot hide a regression.
    const MEASURED_VISIBLE: usize = 6_757;
    const MEASURED_STRUCTURALLY_COMPLETE: usize = 6_745;
    assert!(
        visible >= MEASURED_VISIBLE,
        "visible event corpus regressed: {visible} < {MEASURED_VISIBLE}"
    );
    assert!(
        structurally_complete >= MEASURED_STRUCTURALLY_COMPLETE,
        "structurally complete event count regressed: {structurally_complete} < \
         {MEASURED_STRUCTURALLY_COMPLETE}"
    );
    assert!(
        structurally_complete * MEASURED_VISIBLE >= MEASURED_STRUCTURALLY_COMPLETE * visible,
        "definition-shape ratio regressed below measured baseline: \
         {structurally_complete}/{visible} < \
         {MEASURED_STRUCTURALLY_COMPLETE}/{MEASURED_VISIBLE}"
    );
}
