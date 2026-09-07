use super::authored_inputs::DesignerAuthoredInputs;
use super::item_database::{collect_country_pools, ItemPoolKey};
use super::tests::{admitted_production_renderer, production_fixture};
use crate::design_catalog::ObserverDesignerViewState;
use crate::readonly_design_peace::DesignerViewKind;
use crate::stored_country_selection::StoredCountrySelection;
use replay_core::designer_icon_context::{CapitalSlotOutcome, DesignerIconContextRefusal};
use replay_core::{BuildSchedule, RawFrame, WorldFrame};
use serde_json::Value;
use std::cell::Cell;

const JOINT: &[u8] = include_bytes!("../../../../fixtures/cpni3_capital_slot_joint.json");
const JOINT_SHA256: &str = "ddd1134dcd6ca27e00951653fc3f856efc53e04e39f5af3b7f3d651bbedf96a5";

fn digest(bytes: &[u8]) -> String {
    let mut hash = snapshot::sha256::Sha256::new();
    hash.update(bytes);
    snapshot::sha256::to_hex(&hash.finalize())
}

fn viewer() -> ObserverDesignerViewState {
    ObserverDesignerViewState {
        family: DesignerViewKind::Tank,
        anchor_design_id: String::new(), anchor_catalogue_hour: -1,
        anchor_catalogue_owner_tag: StoredCountrySelection::of("GER"),
        anchor_catalogue_family: DesignerViewKind::Tank,
        selected_design_id: None, our_filter_enabled: true, foreign_filter_enabled: true,
        obsolete_filter_enabled: true, excluded_country_filter_tags: vec![],
        scroll_wheel_steps: 0, country_scroll_wheel_steps: 0,
    }
}

#[test]
fn actual_cpni3_four_stages_preserve_refusal_and_bind_only_authenticated_template() {
    assert_eq!(JOINT.len(), 38_399);
    assert_eq!(digest(JOINT), JOINT_SHA256);
    let joint: Value = serde_json::from_slice(JOINT).unwrap();
    assert_eq!(joint["schema"], 1);
    assert_eq!(joint["producer"], "actual_bracketed_cpni3_publisher_fold_fixture_keeps_historical_refusal_until_next_hour");
    assert_eq!(joint["acquisition"], "ReaderClockBracketGuard/read_compact_country_domain_bundle/country_bundle_publication_records");
    let stages = joint["stages"].as_array().unwrap();
    assert_eq!(stages.iter().map(|stage| stage["stage"].as_str().unwrap()).collect::<Vec<_>>(),
        ["H_accept", "H_refusal", "H_raw_recovery_historical_refusal", "H_plus_1_clean"]);

    let (root, _) = production_fixture();
    for dir in ["map", "history/states"] {
        std::fs::create_dir_all(root.0.join(dir)).unwrap();
    }
    for (path, text) in [
        ("map/default.map", r#"continent="continent.txt" definitions="definition.csv" provinces="provinces.bmp""#),
        ("map/continent.txt", "continents={ europe asia africa }"),
        ("history/states/template.txt", "state={id=17 provinces={1}}"),
        ("history/states/slot.txt", "state={id=1 provinces={2}}"),
        ("history/states/runtime.txt", "state={id=73 provinces={3}}"),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;1\n2;2;0;0;land;false;plains;2\n3;3;0;0;land;false;plains;3\n"),
    ] {
        std::fs::write(root.0.join(path), text).unwrap();
    }
    let mut bitmap = vec![0u8; 66];
    bitmap[..2].copy_from_slice(b"BM");
    bitmap[10..14].copy_from_slice(&54u32.to_le_bytes());
    bitmap[14..18].copy_from_slice(&40u32.to_le_bytes());
    bitmap[18..22].copy_from_slice(&4i32.to_le_bytes());
    bitmap[22..26].copy_from_slice(&1i32.to_le_bytes());
    bitmap[26..28].copy_from_slice(&1u16.to_le_bytes());
    bitmap[28..30].copy_from_slice(&24u16.to_le_bytes());
    for (pixel, red) in bitmap[54..].chunks_exact_mut(3).zip([1, 2, 3, 1]) {
        pixel.copy_from_slice(&[0, 0, red]);
    }
    std::fs::write(root.0.join("map/provinces.bmp"), bitmap).unwrap();
    let renderer = admitted_production_renderer(&root.0, None);
    let authored = DesignerAuthoredInputs::load(renderer.resolver()).unwrap();
    let lease = renderer.resolver().icon_content_lease().unwrap();
    let context = map_viewer::authored_icon_context::AuthoredIconContext::from_admission_lease(lease).unwrap();
    let states = context.state_continents().unwrap();
    assert_eq!(states.get(&17).unwrap().as_deref(), Some("europe"));
    assert_eq!(states.get(&1).unwrap().as_deref(), Some("asia"));
    assert_eq!(states.get(&73).unwrap().as_deref(), Some("africa"));

    let mut admitted = Vec::new();
    for (index, stage) in stages.iter().enumerate() {
        let label = stage["stage"].as_str().unwrap();
        let raw_hex = stage["raw_h4cr_hex"].as_str().unwrap();
        assert_eq!(raw_hex.len() % 2, 0);
        let raw: Vec<u8> = raw_hex.as_bytes().chunks_exact(2).map(|pair|
            u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect();
        assert_eq!(digest(&raw), stage["raw_h4cr_sha256"].as_str().unwrap(), "{label}");
        let value = &stage["snapshot"];
        assert_eq!(digest(&serde_json::to_vec(value).unwrap()), stage["snapshot_sha256"].as_str().unwrap(), "{label}");
        let body = snapshot::Snapshot::from_world_value(value.clone()).unwrap();
        let expected_rest = body.rest.clone();
        let raw = RawFrame {
            hour_index: body.hour_index.unwrap(), date: body.date,
            save_version: body.save_version, schema_version: body.schema_version,
            rest: body.rest.into(),
        };
        assert_eq!(raw.rest.to_map(), expected_rest, "{label}: no identity, ledger or readiness supplementation");
        let frame = WorldFrame::from_raw_scheduled(raw, None, None, None, BuildSchedule::Serial);
        assert_eq!(frame.hour_index(), 16_959_360 + i64::from(index == 3));
        assert_eq!(frame.country_identity_for_tag("GER").unwrap(), 1);
        assert_eq!(value["designs"], serde_json::json!({}));
        assert!(frame.design_catalog().is_none());
        let politics = &value["countries"]["GER"]["politics"];
        assert_eq!(politics["capital_state"], 1);
        assert_eq!(politics["capital_override_raw"], 0);
        let observed = frame.designer_capital_slot("GER");
        if index == 1 || index == 2 {
            assert_eq!(observed, Err(DesignerIconContextRefusal::DomainProblem("malformed_payload".into())), "{label}");
            if index == 1 {
                assert!(politics["capital_slot"].is_null());
            } else {
                assert_eq!(politics["capital_slot"]["outcome"]["kind"], "present");
                assert_eq!(politics["capital_slot"]["observation_ordinal"], 7);
            }
        } else {
            let observation = observed.unwrap();
            assert_eq!(observation.game_hour_raw(), 60_759_359 + i32::from(index == 3));
            assert_eq!(observation.observation_ordinal(), if index == 0 { 3 } else { 9 });
            assert_eq!(observation.tag_db_id(), 1);
            assert_eq!(observation.operands().base_raw, Some(1));
            assert_eq!(observation.operands().override_raw, Some(0));
            assert_eq!(observation.outcome(), CapitalSlotOutcome::Present { slot_count: 4, state_id: 73, template_id: 17 });
            assert_eq!(authored.capital_continent(observation).unwrap().as_deref(), Some("europe"), "{label}");
            admitted.push(label);
        }

        // These producer snapshots do not supply the prerequisites for a complete designer panel.
        assert!(matches!(frame.ordered_faction_roster().unwrap(), replay_core::Leaf::Absent));
        let callback = Cell::new(false);
        let mut reached = Vec::new();
        let collected = collect_country_pools::<()>(
            &frame, "GER", |key| {
                assert_eq!(key, ItemPoolKey::Country("GER"));
                reached.push("GER");
                Ok(Vec::new())
            },
            |_| { callback.set(true); Ok(None) },
        );
        assert_eq!(collected.unwrap_err(), "XUI13 item pool faction order: Absent", "{label}");
        assert_eq!(reached, ["GER"]);
        assert!(!callback.get(), "{label}: no capital callback before authenticated faction order");
        let panel = renderer.compose_xui13_designer_surface(
            &frame, frame.named_country("GER").unwrap(), &viewer(), 160, 120,
        );
        assert_eq!(panel.unwrap_err(), crate::production_renderer::ProductionUiRejection::IncompleteComposition(
            "XUI13 strict designer compositor rejected: XUI13 catalogue admission rejected: MissingCatalogue".into()), "{label}");
        assert_eq!(digest(&serde_json::to_vec(value).unwrap()), stage["snapshot_sha256"].as_str().unwrap(), "{label}: input remains verbatim");
    }
    assert_eq!(admitted, ["H_accept", "H_plus_1_clean"]);
    assert_eq!(digest(JOINT), JOINT_SHA256);
}
