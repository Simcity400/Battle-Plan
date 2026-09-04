
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
            assert_eq!(
                strip.badges.len(),
                AlertKind::ALL.len(),
                "{tag} canonical badges @t{t}"
            );

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

#[test]
fn absent_pov_renders_fail_closed_strip() {
    let tl = fixture("fx_topbar_scalars_sweep");
    let frame = tl.frame(tl.first_hour()).unwrap();
    let strip = alerts_strip_from_frame(&frame, "USA");
    assert_eq!(strip.pov_tag, "USA");
    assert_eq!(strip.badges.len(), AlertKind::ALL.len());
    for badge in &strip.badges {
        assert_eq!(badge.count, None, "{:?} fail-closed None", badge.kind);
        assert!(!badge.badge_shown);
        assert_eq!(badge.count_text, None);
    }
}

#[test]
fn alert_feed_is_fail_closed_absent() {
    let feed = alert_feed_binding();
    assert!(!feed.available);
    assert!(feed.icons.is_empty());
}


struct MemorySource {
    hours: Vec<i64>,
    frames: std::collections::BTreeMap<i64, serde_json::Value>,
}

impl replay_core::ReplaySource for MemorySource {
    fn game_id(&self) -> &str {
        "alerts_status_test"
    }
    fn hours(&self) -> &[i64] {
        &self.hours
    }
    fn warnings(&self) -> &[String] {
        &[]
    }
    fn frame(
        &self,
        hour_index: i64,
    ) -> Result<replay_core::RawFrame, replay_core::SourceError> {
        let mut rest = self
            .frames
            .get(&hour_index)
            .and_then(serde_json::Value::as_object)
            .cloned()
            .ok_or_else(|| replay_core::SourceError::Open(format!("no frame {hour_index}")))?;
        if replay_core::COUNTRY_COMPOSED_MEMBERS
            .iter()
            .any(|member| rest.contains_key(*member))
        {
            rest.entry("tag_db_id_index")
                .or_insert_with(|| serde_json::json!({"country_composition_version": 2}));
        }
        Ok(replay_core::RawFrame {
            hour_index,
            date: None,
            save_version: None,
            schema_version: None,
            rest,
        })
    }
}

#[test]
fn general_alert_availability_is_not_borrowed_from_countries_domain() {
    let source = MemorySource {
        hours: vec![0, 1],
        frames: [
            (
                0,
                serde_json::json!({
                    "countries": { "GER": { "scalars": { "industrial_ratio": 0.5 } } }
                }),
            ),
            (
                1,
                serde_json::json!({
                    "countries": { "GER": { "scalars": { "alerts": { "initiative": 2 } } } }
                }),
            ),
        ]
        .into_iter()
        .collect(),
    };
    let tl = replay_core::WorldTimeline::open(Box::new(source)).expect("open memory timeline");

    let no_alert_datum = tl.frame(0).unwrap();
    assert_eq!(
        no_alert_datum.topbar_extras_status(),
        replay_core::ViewStatus::Present,
        "precondition: the countries-derived status still claims Present"
    );
    assert_ne!(
        no_alert_datum.topbar_alerts_status(),
        replay_core::ViewStatus::Present
    );
    let strip = alerts_strip_from_frame(&no_alert_datum, "GER");
    assert!(
        !strip.available,
        "no alert datum anywhere => the strip must not claim availability"
    );
    assert!(strip
        .badges
        .iter()
        .all(|badge| badge.count.is_none() && !badge.badge_shown));
    let feed = game_ui::alerts::alert_feed_from_frame(&no_alert_datum, "GER");
    assert!(!feed.available, "the feed is gated on the same status");

    let with_alerts = tl.frame(1).unwrap();
    assert_eq!(
        with_alerts.topbar_alerts_status(),
        replay_core::ViewStatus::Present
    );
    let strip = alerts_strip_from_frame(&with_alerts, "GER");
    assert!(strip.available);
    assert_eq!(strip.badge(AlertKind::Initiative).count, Some(2));
}

#[test]
fn prototype_reward_badge_requires_complete_owner_joined_d65_rows() {
    let malformed = MemorySource {
        hours: vec![0],
        frames: [(
            0,
            serde_json::json!({
                "countries": {
                    "GER": {
                        "_r2_tag_db_id": 7,
                        "special_project_programs": [{ "unread_prototype_rewards": 5 }]
                    }
                }
            }),
        )]
        .into_iter()
        .collect(),
    };
    let malformed = replay_core::WorldTimeline::open(Box::new(malformed))
        .expect("open malformed D65 timeline");
    let malformed = malformed.frame(0).unwrap();
    assert_eq!(
        malformed.special_project_programs("GER"),
        replay_core::Leaf::Unobtained,
        "missing program identity/province is not a complete D65 row"
    );
    let strip = alerts_strip_from_frame(&malformed, "GER");
    let badge = strip.badge(AlertKind::PrototypeRewards);
    assert_eq!(badge.count, None);
    assert!(!badge.badge_shown);

    let complete = MemorySource {
        hours: vec![0],
        frames: [(
            0,
            serde_json::json!({
                "countries": {
                    "GER": {
                        "_r2_tag_db_id": 7,
                        "special_project_programs": [{
                            "owner_tag_db_id": 7,
                            "program_id": 11,
                            "province": 42,
                            "unread_prototype_rewards": 5
                        }]
                    }
                }
            }),
        )]
        .into_iter()
        .collect(),
    };
    let complete = replay_core::WorldTimeline::open(Box::new(complete))
        .expect("open complete D65 timeline");
    let complete = complete.frame(0).unwrap();
    assert!(matches!(
        complete.special_project_programs("GER"),
        replay_core::Leaf::Measured(_)
    ));
    let strip = alerts_strip_from_frame(&complete, "GER");
    let badge = strip.badge(AlertKind::PrototypeRewards);
    assert!(strip.available);
    assert_eq!(badge.count, Some(5));
    assert!(badge.badge_shown);
    assert_eq!(badge.count_text.as_deref(), Some("5"));
}


mod layout_gate {
    use super::*;
    use game_ui::alerts::TOPBAR_GUI;
    use game_ui::assets::AssetResolver;
    use game_ui::compose::{resolve_element_rect, ComposeOptions, Rect};
    use game_ui::gui::LayoutSet;
    use game_ui::paradox_text::parse_file;
    use game_ui::topbar::{resolve_topbar_rects_pure, TopbarLayout, TOPBAR_WINDOW_NAME};

    fn hoi4_root() -> PathBuf {
        replay_test_gates::require_vanilla_game_root()
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

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
    )]
    fn topbar_alert_badges_resolve_via_shared_parser() {
        let root = hoi4_root();
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
            println!("{kind:?} -> {name} @ ({},{})", rect.x, rect.y);
        }

        assert!(
            set.sprite_refs().contains(ALERT_BADGE_BG_SPRITE),
            "topbar.gui must cite {ALERT_BADGE_BG_SPRITE}"
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
    )]
    fn alerts_feed_window_resolves_via_shared_parser() {
        let root = hoi4_root();
        let resolver = AssetResolver::new(vec![root]);
        let set = load_set(&resolver, ALERTS_GUI);

        let window = set.find_by_name(ALERT_FEED_WINDOW);
        let window = *window
            .first()
            .unwrap_or_else(|| panic!("{ALERT_FEED_WINDOW} not found in {ALERTS_GUI}"));

        assert!(
            set.sprite_refs().contains(ALERT_FEED_ICON_SPRITE),
            "{ALERTS_GUI} must cite {ALERT_FEED_ICON_SPRITE}"
        );

        let options = ComposeOptions::new(1920, 1080).with_root_size(1920, 1080);
        let parent = Rect::new(0, 0, 1920, 1080);
        let rect = resolve_element_rect(window, parent, (0, 0), &options);
        assert_eq!((rect.w(), rect.h()), (43, 38), "feed window size (FILE)");
    }
}
