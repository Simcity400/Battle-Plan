use map_viewer::PlaybackEnvironment;
use super::*;
use game_ui::production_renderer::{ProductionUiRenderer, REQUIRED_AUTHORED_HUD_SOURCES};
use portable_content_manifest::{
    ContentLayerKind, ContentManifest, ContentNamespaceScope, LayerStorageKind,
    RecordedContentLayer, RequiredContent, RequiredContentKind, SourceGameIdentity,
    SourceProfileIdentity,
};
use portable_content_manifest::effective_vfs::{EffectiveVfs, EffectiveVfsLayerMount};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

struct Fixture {
    renderer: ProductionUiRenderer,
    world: WorldFrame,
    viewer: game_ui::production_renderer::ObserverViewerUiState,
    pending: Xui08PresentedFrame,
}

fn write(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn tga() -> Vec<u8> {
    let mut bytes = vec![0; 18];
    bytes[2] = 2;
    bytes[12] = 1;
    bytes[14] = 1;
    bytes[16] = 32;
    bytes[17] = 0x28;
    bytes.extend([255; 4]);
    bytes
}

fn write_gui_fixture(root: &Path, missing_glyph: bool) {
    for path in REQUIRED_AUTHORED_HUD_SOURCES { write(root, path, b""); }
    write(root, "interface/peaceconferencewindow.gfx", br#"
spriteTypes = { spriteType = { name = GFX_test texturefile = "gfx/test.tga" noOfFrames = 1 } }
"#);
    write(root, "interface/core.gfx", b"bitmapfonts = { textcolors = {} }");
    write(root, "gfx/test.tga", tga());
    write(root, "gfx/fonts/test.tga", tga());
    write(root, "gfx/FX/text.shader", include_bytes!("../../tests/fixtures/exact_a729/text.shader"));
    let glyph = if missing_glyph { 65 } else { 48 };
    write(root, "gfx/fonts/test.fnt", format!(
        "info face=\"test\" size=1 padding=0,0,0,0 spacing=0,0 outline=0\n\
         common lineHeight=1 base=1 scaleW=1 scaleH=1 pages=1\n\
         page id=0 file=\"test.tga\"\n\
         chars count=1\n\
         char id={glyph} x=0 y=0 width=1 height=1 xoffset=0 yoffset=0 xadvance=1 page=0 chnl=15\n",
    ));
    write(root, "localisation/english/test_l_english.yml", b"\xef\xbb\xbfl_english:\n");
    write(root, "interface/core.gui", br#"
guiTypes = {
 textBoxType = { name = ToolTip textureFile = "gfx/test.tga" borderSize = { x=0 y=0 }
  font = test maxWidth = 32 maxHeight = 32 }
 extendedScrollbarType = { name = test_scroll position = {x=0 y=0} size = {width=4 height=40}
  orientation = upper_right origo = upper_right smooth_scrolling = 0.25
  background = { quadTextureSprite = GFX_test }
  track = { quadTextureSprite = GFX_test }
  slider = { quadTextureSprite = GFX_test }
  decreaseButton = { quadTextureSprite = GFX_test }
  increaseButton = { position = {x=-1 y=-1} quadTextureSprite = GFX_test }
 }
}
"#);
    let actions = ["take_state_action", "liberate_action", "puppet_action",
        "force_government_action", "take_navy_action", "demilitarize_zone_action",
        "resource_rights_action", "war_reparation_action", "dismantle_industry_action"]
        .map(|name| format!("buttonType = {{ name = {name} hide = yes }}")).join("\n");
    let templates = ["aggregated_action_item", "available_action_item",
        "conference_winner_biddings", "conference_beneficiary_biddings",
        "conference_bidding_aggregated_action", "conference_bidding_aggregated_country",
        "conference_bidding_entry", "conference_bidding_quick_access_button",
        "peace_expandable_separator", "biddings_popup_item"]
        .map(|name| format!("containerWindowType = {{ name = {name} size = {{width=4 height=4}} }}"))
        .join("\n");
    write(root, "interface/peaceconferencewindow.gui", format!(r#"
guiTypes = {{
 containerWindowType = {{ name = peaceconference_full_window size = {{width=64 height=64}}
  orientation = center origo = center
  position = {{x=0 y=0}} show_position = {{x=0 y=0}} hide_position = {{x=-8 y=0}}
  animation_type = linear animation_time = 1000 fade_type = linear fade_time = 1000
  instantTextBoxType = {{ name = current_score position = {{x=12 y=3}}
   maxWidth = 20 maxHeight = 8 font = test text = "0" }}
  buttonType = {{ name = close_button position = {{x=0 y=0}} size = {{width=2 height=2}} }}
  containerWindowType = {{ name = available_actions position = {{x=0 y=12}}
   size = {{width=28 height=40}} verticalScrollbar = test_scroll
   containerWindowType = {{ name = available_actions_list size = {{width=24 height=40}} }} }}
  containerWindowType = {{ name = current_and_history_actions position = {{x=32 y=12}}
   size = {{width=28 height=40}} verticalScrollbar = test_scroll
   containerWindowType = {{ name = country_biddings_list size = {{width=24 height=40}} }} }}
  {actions}
 }}
 containerWindowType = {{ name = biddings_popup_window size = {{width=16 height=32}}
  gridBoxType = {{ name = biddings_grid size = {{width=16 height=32}} verticalScrollbar = test_scroll
    scroll_wheel_factor = 1 smooth_scrolling = no }} }}
 {templates}
}}
"#));
}

fn renderer(root: PathBuf) -> ProductionUiRenderer {
    let vfs_id = "e".repeat(64);
    let vfs = EffectiveVfs::new(&vfs_id, [EffectiveVfsLayerMount {
        ordinal: 0, layer_kind: ContentLayerKind::Base, layer_id: "base".into(),
        storage_kind: LayerStorageKind::Directory, local_source: root.clone(),
        source_container_sha256: None, replace_paths: Vec::new(),
    }]).unwrap();
    let observed = vfs.observe_namespace(&ContentNamespaceScope::root()).unwrap();
    let sealed = observed.seal(&vfs).unwrap();
    let required_content = sealed.winners().iter().map(|key| RequiredContent {
        content_key: key.normalized_game_relative_path.clone(),
        kind: RequiredContentKind::Other, observation_count: 1,
        consumers: vec!["xui08-public-entry-fixture".into()], content: key.clone(),
    }).collect();
    let mut manifest = ContentManifest {
        native_graphics_query: Default::default(),
        schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
        product_modset_sha256: "9".repeat(64),
        source_game: SourceGameIdentity { version: "1.19.2".into(), build: "a729".into(),
            executable_sha256: replay_core::peace_conference_presentation::XUI08_EXECUTABLE_SHA256.into(),
            build_profile: "release".into() },
        source_profile: SourceProfileIdentity { profile_id: "test".into(), playset_id: "test".into(),
            selection_sha256: "0".repeat(64) },
        effective_vfs_manifest_sha256: vfs_id,
        ordered_layers: vec![RecordedContentLayer { ordinal: 0, layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(), display_name: "base".into(), descriptor_sha256: None,
            source_container_sha256: None, storage_kind: LayerStorageKind::Directory,
            replace_paths: Vec::new(), required_for_playback: true }],
        dlc_selection: Vec::new(), localisation_language: "l_english".into(),
        required_settings: BTreeMap::new(), recorder_local_settings: BTreeMap::new(),
        required_content, witnessed_content_namespaces: Vec::new(),
        witnessed_absent_content: Vec::new(), manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    manifest.bind_observed_namespace(sealed).unwrap();
    let manifest = manifest.with_computed_digest().unwrap();
    let resolver = map_viewer::GameFileResolver::vanilla(root).unwrap();
    let inventory = resolver.playback_inventory(&manifest, PlaybackEnvironment {
        source_game: manifest.source_game.clone(), source_profile: manifest.source_profile.clone(),
        dlc_selection: Vec::new(), localisation_language: "l_english".into(), settings: BTreeMap::new(),
    }).unwrap();
    let admission = manifest.admit_playback(&inventory).unwrap();
    let resolver = resolver.with_content_admission(&admission).unwrap();
    let assets = AssetResolver::from_content_admission(&resolver, &admission).unwrap();
    ProductionUiRenderer::new(assets, &admission).unwrap()
}

fn fixture(missing_glyph: bool) -> Fixture {
    use replay_core::peace_conference_presentation as pcs3;
    let root = std::env::temp_dir().join(format!("arrow-xui08-public-{}-{}",
        std::process::id(), std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    write_gui_fixture(&root, missing_glyph);
    let renderer = renderer(root);
    let text = |id: &str| pcs3::PeaceFinalTextExact {
        authored_identity: id.into(), raw_value: "0".into(), final_text: "0".into(),
        final_text_sha256: pcs3::text_sha256("0"),
    };
    let country = pcs3::PeaceObjectReferenceExact { object_type: 3, object_id: 47 };
    let participant = pcs3::PeaceParticipantPresentationExact {
        source_ordinal: 0, stable_participant_id: pcs3::participant_identity(&country),
        country_reference: country, country_tag: "GER".into(), country_tag_db_id: 47,
        flag_texture_path: "gfx/flags/GER.tga".into(), role: pcs3::PeaceParticipantRoleExact::Defeated,
        country_display: text("participant.country"), score: text("participant.score"),
        score_ratio: text("participant.ratio"), expected_allocation_count: 0, expected_result_count: 0,
        allocation_ids: Vec::new(), result_ids: Vec::new(), bid_count_display: text("participant.bids"),
        contested_bid_count_display: text("participant.contested"),
        tooltip: pcs3::PeaceTooltipExact { primary: text("participant.tooltip"), delayed: None },
    };
    let mut carrier = pcs3::PeaceConferencePresentationExact {
        schema_version: pcs3::XUI08_SCHEMA_VERSION, authority: pcs3::XUI08_AUTHORITY.into(),
        executable_sha256: pcs3::XUI08_EXECUTABLE_SHA256.into(),
        content_manifest_sha256: renderer.content_manifest_digest_sha256().into(),
        producer_manifest_sha256: pcs3::XUI08_PRODUCER_CONTRACT_MANIFEST_SHA256.into(),
        evidence_manifest_sha256: pcs3::XUI08_EVIDENCE_MANIFEST_SHA256.into(),
        frame_hour: 20, capture_sequence: 7, passive_global: true, ui_open_independent: true,
        recorder_pov_independent: true, producer_complete: true, previous_record_sha256: None,
        record_sha256: String::new(),
        observation: pcs3::PeaceConferencePresentationObservationExact::Active {
            refresh_sequence: 11, refresh_epoch: 3,
            conference_reference: pcs3::PeaceObjectReferenceExact { object_type: 9, object_id: 40 },
            phase: pcs3::PeacePhaseExact::Completed, turn: 1, remaining_score_turns: 0,
            current_participant_id: None, expected_available_state_count: 0,
            expected_contested_state_count: 0, expected_participant_count: 1,
            expected_allocation_count: 0, expected_allocation_group_count: 0,
            expected_contested_popup_count: 0, expected_result_count: 0,
            available_state_ids: Vec::new(), contested_state_ids: Vec::new(),
            current_participant_display: text("conference.current"), phase_display: text("conference.phase"),
            current_score: text("conference.score"), score_this_turn: text("conference.turn_score"),
            remaining_turns_display: text("conference.remaining"), participants: vec![participant],
            allocations: Vec::new(), allocation_groups: Vec::new(), contested_popups: Vec::new(), results: Vec::new(),
        },
    };
    carrier.record_sha256 = pcs3::canonical_record_sha256(&carrier).unwrap();
    let rest = serde_json::json!({
        "tag_db_id_index": {"country_composition_version": 2},
        "countries": {"GER": {"_r2_tag_db_id": 47}},
        "_r2_peace_conference_live": {
            "schema_version": 1, "authority": "exact_a729_peace_conference",
            "executable_sha256": pcs3::XUI08_EXECUTABLE_SHA256,
            "producer_manifest_sha256": "e7d306a82cc81b1f5c78dac6288fc0c8366c11bcac5cfda8ac03bfb661addb30",
            "elapsed_wall_ms": 1, "frame_hour": 20,
            "observation": {"kind": "active", "refresh_sequence": 11, "refresh_epoch": 3,
                "conference_reference": {"object_type": 9, "object_id": 40},
                "current_turn_raw": 1, "completed": true, "semantic_snapshot_complete": false,
                "participants": null, "allocations": null, "results": null}
        },
        "_r2_peace_conference_presentation_exact": carrier,
    });
    let world = WorldFrame::from_raw_scheduled(replay_core::RawFrame {
        hour_index: 20, date: None, save_version: None, schema_version: None,
        rest: rest.as_object().unwrap().clone().into(),
    }, None, None, None, replay_core::BuildSchedule::Serial);
    assert!(world.d18_country_roster().complete().is_ok(),
        "fixture must admit its current country composition: {:?}", world.coverage_warnings());
    assert!(world.peace_conference_exact().conference.is_some(),
        "fixture must admit its same-hour D97 lifecycle: {:?}", world.coverage_warnings());
    assert!(world.peace_conference_presentation_exact().presentation.is_some(),
        "authenticated PCS3 fixture must admit through WorldFrame: {:?}", world.coverage_warnings());
    let snapshot = game_ui::readonly_design_peace::peace_conference_snapshot_from_world(&world).unwrap();
    let admitted = game_ui::readonly_design_peace::admit_peace_conference(&snapshot).unwrap();
    let mut viewer = game_ui::production_renderer::ObserverViewerUiState::default();
    viewer.peace_conference.synchronize(&admitted);
    viewer.peace_conference.set_open(true);
    let pending = Xui08PresentedFrame {
        frame_hour: 20, capture_sequence: 7, record_sha256: carrier.record_sha256,
        admitted, input_region: None, hit_targets: Vec::new(), tooltip_targets: Vec::new(),
    };
    Fixture { renderer, world, viewer, pending }
}

fn framebuffer(width: usize, height: usize) -> game_ui::dds::RgbaImage {
    game_ui::dds::RgbaImage { width, height, pixels: vec![0; width * height * 4] }
}

fn stage(fixture: &mut Fixture, image: &mut game_ui::dds::RgbaImage, delta: f32)
    -> std::result::Result<Xui08InstalledSurface, Xui08SurfaceStageError>
{
    stage_at(fixture, image, [64, 64], delta)
}

fn stage_at(fixture: &mut Fixture, image: &mut game_ui::dds::RgbaImage,
    viewport: [usize; 2], delta: f32)
    -> std::result::Result<Xui08InstalledSurface, Xui08SurfaceStageError>
{
    compose_and_install_xui08_surface(&fixture.renderer, &fixture.world, viewport, delta, None,
        &mut fixture.viewer.peace_conference, &mut fixture.viewer.sprite_resources,
        &mut fixture.pending, image, None)
}

#[test]
fn public_renderer_report_refusal_does_not_install_a_transition() {
    let mut fixture = fixture(true);
    let before = fixture.viewer.peace_conference.clone();
    let mut image = framebuffer(64, 64);
    let pixels = image.pixels.clone();
    let error = stage(&mut fixture, &mut image, 0.1).err().expect("missing glyph must refuse");
    let Xui08SurfaceStageError::Renderer(
        game_ui::production_renderer::ProductionUiRejection::IncompleteComposition(report)) = &error
    else { panic!("expected missing glyph report: {error:?}"); };
    assert!(report.contains("missing_fonts={}"), "{report}");
    assert!(report.contains("EngineTextLayoutRefused"), "{report}");
    assert!(report.contains("bitmap font has no glyph for U+0030 at stripped byte 0"), "{report}");
    assert_eq!(fixture.viewer.peace_conference, before);
    assert_eq!(image.pixels, pixels);
    assert!(fixture.pending.input_region.is_none());
    assert!(fixture.pending.hit_targets.is_empty());
}

#[test]
fn public_renderer_success_then_destination_failure_preserves_published_history() {
    let mut fixture = fixture(false);
    let mut image = framebuffer(64, 64);
    let background = [32, 64, 96, 255];
    for pixel in image.pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&background);
    }
    let transaction = ObserverViewerPresentTransaction::begin(&fixture.viewer);
    let installed = stage(&mut fixture, &mut image, 0.1).unwrap();
    assert!(installed.visible);
    // This fixture reaches one opaque atlas texel at linear fade t=0.1.
    // Existing text tint is [221,214,190,255]. Native RGB is tint*0.1
    // + destination*0.9, with MAX alpha. A preblended transparent surface
    // blended again yields [31,60,88,255] instead and must fail this assertion.
    let changed: Vec<[u8; 4]> = image.pixels.chunks_exact(4)
        .map(|pixel| <[u8; 4]>::try_from(pixel).unwrap())
        .filter(|pixel| *pixel != background).collect();
    assert_eq!(changed, vec![[51, 79, 105, 255]]);
    assert!(!fixture.pending.hit_targets.is_empty());
    transaction.commit();
    let before = fixture.viewer.peace_conference.clone();
    let input = fixture.pending.input_region.clone();
    let hits = fixture.pending.hit_targets.clone();
    let mut wrong_size = framebuffer(63, 64);
    let pixels = wrong_size.pixels.clone();
    assert!(matches!(stage(&mut fixture, &mut wrong_size, 0.1),
        Err(Xui08SurfaceStageError::Execution(_))));
    assert_eq!(fixture.viewer.peace_conference, before);
    assert_eq!(fixture.pending.input_region, input);
    assert_eq!(fixture.pending.hit_targets, hits);
    assert_eq!(wrong_size.pixels, pixels);
}

#[test]
fn actual_present_repeat_reject_and_terminal_hide_keep_one_retained_history() {
    let mut fixture = fixture(false);
    let mut image = framebuffer(64, 64);
    let transaction = ObserverViewerPresentTransaction::begin(&fixture.viewer);
    assert!(stage(&mut fixture, &mut image, 0.1).unwrap().visible);
    transaction.commit();
    let first = fixture.viewer.peace_conference.clone();
    let published_hits = fixture.pending.hit_targets.clone();
    let mut pending = Some(fixture.pending.clone());
    let mut pending_staged = true;
    let mut viewer_checkpoint = None;
    let mut pending_input = None;
    begin_xui08_staged_candidate(&mut fixture.viewer.peace_conference,
        &mut pending_input, &mut viewer_checkpoint).unwrap();
    assert!(compose_and_install_xui08_surface(&fixture.renderer, &fixture.world,
        [64, 64], 0.1, None, &mut fixture.viewer.peace_conference,
        &mut fixture.viewer.sprite_resources,
        pending.as_mut().unwrap(), &mut image, None).unwrap().visible);
    let second = fixture.viewer.peace_conference.clone();
    assert_ne!(second, first, "same recording hour retains and advances viewer animation");
    rollback_xui08_staged_candidate(&mut fixture.viewer.peace_conference,
        &mut viewer_checkpoint, &mut pending, &mut pending_staged).unwrap();
    assert!(pending.is_none());
    assert!(!pending_staged);
    assert!(viewer_checkpoint.is_none());
    assert_eq!(fixture.pending.hit_targets, published_hits);
    assert_eq!(fixture.viewer.peace_conference, first);
    begin_xui08_staged_candidate(&mut fixture.viewer.peace_conference,
        &mut pending_input, &mut viewer_checkpoint).unwrap();
    pending = Some(fixture.pending.clone());
    pending_staged = true;
    assert!(compose_and_install_xui08_surface(&fixture.renderer, &fixture.world,
        [64, 64], 0.1, None, &mut fixture.viewer.peace_conference,
        &mut fixture.viewer.sprite_resources,
        pending.as_mut().unwrap(), &mut image, None).unwrap().visible);
    assert_eq!(fixture.viewer.peace_conference, second);
    fixture.pending = commit_xui08_staged_candidate(&mut pending,
        &mut pending_staged, &mut viewer_checkpoint).unwrap().unwrap();
    assert!(pending.is_none());
    assert!(!pending_staged);
    assert!(viewer_checkpoint.is_none());
    fixture.viewer.peace_conference.set_open(false);
    let mut hidden = false;
    for _ in 0..16 {
        let transaction = ObserverViewerPresentTransaction::begin(&fixture.viewer);
        let result = stage(&mut fixture, &mut image, 0.1).unwrap();
        transaction.commit();
        if !result.visible {
            assert!(result.input_region.is_none());
            assert!(fixture.pending.input_region.is_none());
            assert!(fixture.pending.hit_targets.is_empty());
            assert!(fixture.pending.tooltip_targets.is_empty());
            hidden = true;
            break;
        }
    }
    assert!(hidden, "native terminal movement update hides the retained widget");
}


#[test]
fn public_resize_reconstruction_rejects_old_geometry_then_recovers_after_occlusion() {
    use game_ui::peace_conference_ui::{PeaceConferenceScrollRegion, PeaceConferenceScrollbarDrag};
    let mut fixture = fixture(false);
    let mut original = framebuffer(64, 64);
    assert!(stage(&mut fixture, &mut original, 0.1).unwrap().visible);
    let before = fixture.viewer.peace_conference.clone();
    let mut resized = framebuffer(96, 80);
    // Dimensions alone cannot manufacture a reconstruction event.
    assert!(stage_at(&mut fixture, &mut resized, [96, 80], 0.1).is_err());
    assert_eq!(fixture.viewer.peace_conference, before);

    fixture.viewer.peace_conference.active_scrollbar_drag = Some(PeaceConferenceScrollbarDrag {
        region: PeaceConferenceScrollRegion::AvailableActions, grab_offset_y: 2,
    });
    let mut owner = ObserverUiPresentTransactionOwner::default();
    owner.begin(&fixture.viewer).unwrap();
    fixture.viewer.peace_conference.invalidate_viewport_container();
    owner.make_resize_invalidation_durable(&fixture.viewer).unwrap();
    let invalidated = fixture.viewer.peace_conference.clone();
    assert!(invalidated.open);
    assert_eq!(invalidated.history_tab, before.history_tab);
    assert_eq!(invalidated.selected_participant_id, before.selected_participant_id);
    assert_eq!(invalidated.expanded_participant_ids, before.expanded_participant_ids);
    assert!(invalidated.active_scrollbar_drag.is_none());

    let candidate = stage_at(&mut fixture, &mut resized, [96, 80], 0.1).unwrap();
    assert!(candidate.visible);
    // Modal input covers the viewport, independently of the centered root.
    let input = candidate.input_region.as_ref().unwrap();
    assert_eq!(input.panel, [0, 0, 96, 80]);
    // Actual descendant geometry proves the root was reconstructed at (16,8):
    // the close button is root-local (0,0), lists are (0,12) and (32,12).
    assert_eq!(input.available_actions, [16, 20, 28, 40]);
    assert_eq!(input.current_and_history, [48, 20, 28, 40]);
    let close: Vec<_> = fixture.pending.hit_targets.iter().filter(|hit|
        hit.action == game_ui::peace_conference_ui::PeaceConferenceHitAction::ClosePanel
    ).map(|hit| hit.rect).collect();
    assert_eq!(close, vec![[16, 8, 2, 2]]);
    owner.finish_disposition(&mut fixture.viewer, ObserverUiFrameDisposition::Occluded, None).unwrap();
    assert_eq!(fixture.viewer.peace_conference, invalidated,
        "rejection must preserve reconstruction and cannot revive old dimensions");

    owner.begin(&fixture.viewer).unwrap();
    let recovery = stage_at(&mut fixture, &mut resized, [96, 80], 0.1).unwrap();
    assert!(recovery.visible);
    // Modal input covers the viewport, independently of the centered root.
    let input = recovery.input_region.as_ref().unwrap();
    assert_eq!(input.panel, [0, 0, 96, 80]);
    // Actual descendant geometry proves the root was reconstructed at (16,8):
    // the close button is root-local (0,0), lists are (0,12) and (32,12).
    assert_eq!(input.available_actions, [16, 20, 28, 40]);
    assert_eq!(input.current_and_history, [48, 20, 28, 40]);
    let close: Vec<_> = fixture.pending.hit_targets.iter().filter(|hit|
        hit.action == game_ui::peace_conference_ui::PeaceConferenceHitAction::ClosePanel
    ).map(|hit| hit.rect).collect();
    assert_eq!(close, vec![[16, 8, 2, 2]]);
    assert!(!fixture.pending.hit_targets.is_empty());
    owner.finish_disposition(&mut fixture.viewer, ObserverUiFrameDisposition::Presented, None).unwrap();
    assert!(fixture.viewer.peace_conference.open);
    assert!(stage_at(&mut fixture, &mut resized, [96, 80], 0.1).unwrap().visible);
}
