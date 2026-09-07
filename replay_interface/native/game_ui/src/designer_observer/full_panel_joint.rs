use super::tests::{admitted_production_renderer, production_fixture, TestRoot};
use crate::production_renderer::ProductionUiRenderer;
use std::fs;
use crate::design_catalog::ObserverDesignerViewState;
use crate::readonly_design_peace::DesignerViewKind;
use crate::stored_country_selection::StoredCountrySelection;
use replay_core::designer_icon_context::{CapitalSlotOutcome, DesignerIconContextRefusal};
use replay_core::{BuildSchedule, RawFrame, WorldFrame};
use serde_json::Value;
use snapshot::design_catalog::{DesignItemIconObservation, DesignTextObservation, DesignU32Observation, PortableDesignCatalog};
use std::sync::Arc;

pub(super) const REGIONAL_RGBA: [u8; 4] = [30, 80, 220, 255];
pub(super) const DEFAULT_RGBA: [u8; 4] = [20, 180, 70, 255];
pub(super) const FLAG_RGBA: [u8; 4] = [220, 180, 20, 255];

pub(super) fn authored_fixture() -> (TestRoot, ProductionUiRenderer) {
    let (root, _) = production_fixture();
    for directory in ["common/country_tags", "map", "history/states",
        "gfx/interface/equipmentdesigner/graphic_db"] {
        fs::create_dir_all(root.0.join(directory)).unwrap();
    }
    for (path, text) in [
        ("common/units/equipment/designer.txt",
            "equipments={ ship_hull={ is_archetype=yes type=screen_ship module_slots={ main={} } } }"),
        ("interface/designer_window.gui",
            r#"guiTypes={ containerWindowType={ name="equipment_designer_ship_hull" size={ width=1 height=1 } } }"#),
        ("common/country_tags/tags.txt", r#"GER="countries/Germany.txt""#),
        ("map/default.map", r#"continent="continent.txt" definitions="definition.csv" provinces="provinces.bmp""#),
        ("map/continent.txt", "continents={ europe asia africa }"),
        ("history/states/template.txt", "state={id=17 provinces={1}}"),
        ("history/states/slot.txt", "state={id=1 provinces={2}}"),
        ("history/states/runtime.txt", "state={id=73 provinces={3}}"),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;1\n2;2;0;0;land;false;plains;2\n3;3;0;0;land;false;plains;3\n"),
        ("gfx/interface/equipmentdesigner/graphic_db/joint.txt",
            "europe={ ship_hull={ pool={ icons={ gfx/interface/joint_regional.tga } weight=2 } } } default={ ship_hull={ pool={ icons={ gfx/interface/joint_default.tga } weight=1 } } }"),
    ] {
        fs::write(root.0.join(path), text).unwrap();
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
    fs::write(root.0.join("map/provinces.bmp"), bitmap).unwrap();
    let template = fs::read(root.0.join("gfx/interface/xui13_test.tga")).unwrap();
    assert_eq!(&template[..18], &[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 32, 0x20]);
    assert_eq!(template.len(), 26);
    for (path, [red, green, blue, alpha]) in [
        ("gfx/interface/joint_regional.tga", REGIONAL_RGBA),
        ("gfx/interface/joint_default.tga", DEFAULT_RGBA),
        ("gfx/flags/small/GER_neutrality.tga", FLAG_RGBA),
    ] {
        let mut texture = template[..18].to_vec();
        texture[12..14].copy_from_slice(&8u16.to_le_bytes());
        texture[14..16].copy_from_slice(&8u16.to_le_bytes());
        for _ in 0..64 {
            texture.extend_from_slice(&[blue, green, red, alpha]);
        }
        fs::write(root.0.join(path), texture).unwrap();
    }
    let font_path = root.0.join("gfx/fonts/xui13_test.fnt");
    let mut font = fs::read_to_string(&font_path).unwrap();
    for character in "Vessel".chars().collect::<std::collections::BTreeSet<_>>() {
        let prefix = format!("char id={} ", u32::from(character));
        if !font.lines().any(|line| line.starts_with(&prefix)) {
            font.push_str(&format!("{prefix}x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0\n"));
        }
    }
    fs::write(font_path, font).unwrap();
    let renderer = admitted_production_renderer(&root.0, None);
    (root, renderer)
}

const CATALOG: &[u8] = include_bytes!("../../../../fixtures/xui13_full_panel_catalog.json");
const CID1: &[u8] = include_bytes!("../../../../fixtures/xui13_full_panel_cid1.h4cr");
const D53: &[u8] = include_bytes!("../../../../fixtures/xui13_full_panel_d53.json");
const JOINT: &[u8] = include_bytes!("../../../../fixtures/xui13_full_panel_snapshots.json");

fn digest(bytes: &[u8]) -> String {
    let mut hash = snapshot::sha256::Sha256::new();
    hash.update(bytes);
    snapshot::sha256::to_hex(&hash.finalize())
}

fn viewer() -> ObserverDesignerViewState {
    ObserverDesignerViewState {
        family: DesignerViewKind::Ship,
        anchor_design_id: String::new(), anchor_catalogue_hour: -1,
        anchor_catalogue_owner_tag: StoredCountrySelection::of("GER"),
        anchor_catalogue_family: DesignerViewKind::Ship,
        selected_design_id: None, our_filter_enabled: true, foreign_filter_enabled: true,
        obsolete_filter_enabled: true, excluded_country_filter_tags: vec![],
        scroll_wheel_steps: 0, country_scroll_wheel_steps: 0,
    }
}

fn frame(value: &Value, catalog: Option<Arc<PortableDesignCatalog>>) -> WorldFrame {
    let body = snapshot::Snapshot::from_world_value(value.clone()).unwrap();
    let expected_rest = body.rest.clone();
    let raw = RawFrame {
        hour_index: body.hour_index.unwrap(), date: body.date,
        save_version: body.save_version, schema_version: body.schema_version,
        rest: body.rest.into(),
    };
    assert_eq!(raw.rest.to_map(), expected_rest, "no snapshot supplementation");
    WorldFrame::from_raw_scheduled(raw, catalog, None, None, BuildSchedule::Serial)
}

fn row_rect(surface: &super::ObserverDesignerSurface, element: &str) -> crate::compose::DebugRect {
    let rect = surface.output.debug_rects.iter().find(|rect|
        rect.name.contains("1:70") && rect.name.ends_with(&format!("::{element}")))
        .unwrap_or_else(|| panic!("missing {element}: {:?}", surface.output.debug_rects));
    let image = &surface.output.image;
    assert!(rect.x >= 0 && rect.y >= 0 && rect.w > 0 && rect.h > 0);
    assert!(rect.x as usize + rect.w as usize <= image.width);
    assert!(rect.y as usize + rect.h as usize <= image.height);
    rect.clone()
}

fn inside(rect: &crate::compose::DebugRect, x: usize, y: usize) -> bool {
    (rect.x as usize..(rect.x + rect.w) as usize).contains(&x)
        && (rect.y as usize..(rect.y + rect.h) as usize).contains(&y)
}

#[test]
fn actual_bound_a27_d53_capital_stages_compose_known_rows_and_local_refusals() {
    for (bytes, size, sha) in [
        (CATALOG, 6_744, "a7e2dff85388d04b771f6bdefe86d5b0a0f9f7e97009209a2264f8ac1512d9bd"),
        (CID1, 189, "6f81df0ba7cae5138ad36a28309a0e9689f37017cc614d3d104599a09a5f1010"),
        (D53, 4_346, "8868ce1d34c94887c74f2dcfed4c77283d9a8fb47df773d474ede2f368dc41c3"),
        (JOINT, 50_230, "f1b3919c9721ec0af8d126bd5a50dc1436da46fddcb838931c32d7209ca7565b"),
    ] {
        assert_eq!(bytes.len(), size);
        assert_eq!(digest(bytes), sha);
    }
    let joint: Value = serde_json::from_slice(JOINT).unwrap();
    let catalog_value: Value = serde_json::from_slice(CATALOG).unwrap();
    let d53: Value = serde_json::from_slice(D53).unwrap();
    assert_eq!(joint["schema"], 1);
    assert_eq!(joint["producer"], "full_panel_actual_generators_share_cid1_and_preserve_capital_gap_history");
    assert_eq!(joint["a27_catalog_sha256"], digest(CATALOG));
    assert_eq!(joint["cid1_h4cr_sha256"], digest(CID1));
    assert_eq!(joint["d53_source_sha256"], digest(D53));
    assert_eq!(catalog_value["authority"]["stream_binding"], joint["stream_binding"]);
    for key in ["cid1_payload_sha256", "game_unique_id", "game_unique_seed", "logical_stream_id"] {
        assert_eq!(d53[key], joint["stream_binding"][key], "{key}");
    }
    assert_eq!(d53["cid1_record_sha256"], digest(CID1));
    let catalog = Arc::new(PortableDesignCatalog::decode_a27_live_json(CATALOG).unwrap());
    // Root-only hour deltas retain each unchanged row's last measurement authority.
    assert_eq!(catalog.records_at_hour(16_959_360).unwrap(), catalog.records_at_hour(16_959_361).unwrap());
    let stages = joint["stages"].as_array().unwrap();
    assert_eq!(stages.iter().map(|stage| stage["stage"].as_str().unwrap()).collect::<Vec<_>>(),
        ["H_accept", "H_refusal", "H_raw_recovery_historical_refusal", "H_plus_1_clean"]);
    let (_root, renderer) = authored_fixture();
    for (path, rgba) in [
        ("gfx/interface/joint_regional.tga", REGIONAL_RGBA),
        ("gfx/interface/joint_default.tga", DEFAULT_RGBA),
        ("gfx/flags/small/GER_neutrality.tga", FLAG_RGBA),
    ] {
        let image = renderer.resolver().decode_texture(path).unwrap().image;
        assert_eq!((image.width, image.height), (8, 8));
        assert_eq!(image.pixels.len(), 8 * 8 * 4);
        assert!(image.pixels.chunks_exact(4).all(|pixel| pixel == rgba));
    }
    let mut surfaces = Vec::new();
    for (index, stage) in stages.iter().enumerate() {
        let label = stage["stage"].as_str().unwrap();
        let raw_hex = stage["raw_h4cr_hex"].as_str().unwrap();
        assert_eq!(raw_hex.len() % 2, 0);
        let raw: Vec<u8> = raw_hex.as_bytes().chunks_exact(2).map(|pair|
            u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect();
        assert_eq!(digest(&raw), stage["raw_h4cr_sha256"].as_str().unwrap(), "{label}");
        assert!(raw.starts_with(CID1), "{label}: actual joint stream starts with the catalog's CID1");
        let value = &stage["snapshot"];
        assert_eq!(digest(&serde_json::to_vec(value).unwrap()), stage["snapshot_sha256"].as_str().unwrap());
        assert_eq!(value["designs"], serde_json::json!({}));
        let frame = frame(value, Some(Arc::clone(&catalog)));
        let hour = 16_959_360 + u64::from(index == 3);
        assert_eq!(frame.hour_index(), hour as i64);
        let semantic_rosters = value["country_semantic_rosters"].as_object().unwrap();
        assert_eq!(semantic_rosters.len(), 5);
        assert_eq!(semantic_rosters["schema"], 2);
        assert_eq!(semantic_rosters["hour_index"], hour);
        for family in ["divisions", "fleets", "railways"] {
            assert_eq!(semantic_rosters.get(family), Some(&Value::Null), "{label}: {family} unobserved");
        }
        assert_eq!(frame.country_identity_for_tag("GER").unwrap(), 1);
        let native = catalog.native_designer_inputs_at_hour(hour).unwrap();
        assert_eq!(native.hour, hour);
        assert_eq!(native.runtime_dlc_mask, DesignU32Observation::Measured { raw_u32: 16 });
        assert_eq!(native.variants.len(), 1);
        assert_eq!(native.variants[0].design_id, "1:70");
        assert_eq!(native.variants[0].equipment_token_raw, 17);
        assert_eq!(native.variants[0].equipment_definition_token, "ship_hull");
        assert_eq!(native.variants[0].record.name.as_deref(), Some("Vessel"));
        assert_eq!(native.variants[0].record.owner.as_deref(), Some("GER"));
        assert_eq!(native.variants[0].source_occurrences.len(), 2);
        assert_eq!(native.variants[0].item_icon_override, DesignItemIconObservation::Measured {
            active: false, kind: 1,
            token: DesignTextObservation::Measured { value: ".".into(), utf8_hex: "2e".into() },
        });
        let roster = frame.ordered_faction_roster().unwrap();
        let replay_core::Leaf::Measured(rows) = roster else { panic!("{label}: {roster:?}") };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].faction_id, 1);
        assert_eq!(rows[0].members, [replay_core::faction_order::OrderedFactionMember { tag_db_id: 1, tag: "GER".into() }]);
        let d53_extra = &value["diplomacy_world"]["domain53_extra"];
        assert_eq!(d53_extra["subject_relation_status"], "measured_empty");
        assert_eq!(d53_extra["snapshot_payload_sha256"], d53["observations"][usize::from(index == 3)]["payload_sha256"]);
        let observation = frame.designer_capital_slot("GER");
        if index == 1 || index == 2 {
            assert_eq!(observation, Err(DesignerIconContextRefusal::DomainProblem("malformed_payload".into())), "{label}");
            if index == 1 {
                assert!(value["countries"]["GER"]["politics"]["capital_slot"].is_null());
            } else {
                assert_eq!(value["countries"]["GER"]["politics"]["capital_slot"]["observation_ordinal"], 5);
                assert_eq!(value["countries"]["GER"]["politics"]["capital_slot"]["outcome"]["kind"], "present");
            }
        } else {
            let observed = observation.unwrap();
            assert_eq!(observed.observation_ordinal(), if index == 0 { 1 } else { 7 });
            assert_eq!(observed.game_hour_raw(), 60_759_359 + i32::from(index == 3));
            assert_eq!(observed.outcome(), CapitalSlotOutcome::Present { slot_count: 4, state_id: 73, template_id: 17 });
        }
        let surface = renderer.compose_xui13_designer_surface(
            &frame, frame.named_country("GER").unwrap(), &viewer(), 160, 120,
        ).unwrap_or_else(|error| panic!("{label}: {error:?}"));
        assert_eq!(surface.ordered_design_ids, ["1:70"], "{label}");
        assert_eq!(surface.visible_design_ids, ["1:70"], "{label}");
        assert!(surface.membership_complete && surface.country_filters_complete, "{label}");
        assert!(surface.ordered_country_filter_tags.is_empty(), "own-country row has no foreign filter entry");
        if index == 1 || index == 2 {
            assert_eq!(surface.field_refusals.keys().map(String::as_str).collect::<Vec<_>>(), ["1:70/equipment_icon", "1:70/flag"], "{label}");
            assert!(surface.field_refusals["1:70/equipment_icon"].contains("malformed_payload"), "{label}: {:?}", surface.field_refusals);
            assert!(surface.field_refusals["1:70/flag"].contains("ruling ideology"), "{label}: {:?}", surface.field_refusals);
        } else {
            assert!(surface.field_refusals.is_empty(), "{label}: {:?}", surface.field_refusals);
        }
        surfaces.push(surface);
        assert_eq!(digest(&serde_json::to_vec(value).unwrap()), stage["snapshot_sha256"].as_str().unwrap(), "{label}: input unchanged");
    }
    let baseline = &surfaces[0];
    let equipment = row_rect(baseline, "equipment");
    let flag = row_rect(baseline, "creator_flag");
    let image = &baseline.output.image;
    for (offset, pixel) in image.pixels.chunks_exact(4).enumerate() {
        let (x, y) = (offset % image.width, offset / image.width);
        if inside(&equipment, x, y) { assert_eq!(pixel, REGIONAL_RGBA, "template17/europe must beat default/slot1/runtime73"); }
        if inside(&flag, x, y) { assert_eq!(pixel, FLAG_RGBA); }
    }
    assert_eq!(surfaces[3].output.image, *image, "clean next hour restores the full exact panel");
    for refused in &surfaces[1..3] {
        let actual = &refused.output.image;
        assert_eq!((actual.width, actual.height), (image.width, image.height));
        assert_ne!(actual, image);
        for (offset, (pixel, expected)) in actual.pixels.chunks_exact(4).zip(image.pixels.chunks_exact(4)).enumerate() {
            let (x, y) = (offset % image.width, offset / image.width);
            if inside(&equipment, x, y) {
                assert_ne!(pixel, REGIONAL_RGBA);
                assert_ne!(pixel, DEFAULT_RGBA, "unavailable capital must not select default");
            } else if inside(&flag, x, y) {
                assert_ne!(pixel, FLAG_RGBA);
            } else {
                assert_eq!(pixel, expected, "known row/name/chrome pixel {x},{y} survives refusal");
            }
        }
    }
    let without_catalog = frame(&stages[0]["snapshot"], None);
    assert_eq!(renderer.compose_xui13_designer_surface(
        &without_catalog, without_catalog.named_country("GER").unwrap(), &viewer(), 160, 120,
    ).unwrap_err(), crate::production_renderer::ProductionUiRejection::IncompleteComposition(
        "XUI13 strict designer compositor rejected: XUI13 catalogue admission rejected: MissingCatalogue".into()));
}
