//! GP-F10's completion proof: the three identity-bearing carriers in ONE frame.
//!
//! Before the shared base-frame projection, XUI12, XUI15 and XUI23 each hashed
//! "the whole raw snapshot minus MY OWN keys", so each carrier's digest covered
//! the other two carriers' payloads — and each payload CONTAINS its own digest.
//! With two or more real carriers that is a fixed point of composed SHA-256
//! evaluations, which no emission order can reach. This file could not be
//! written at all until the projection was shared; that it exists and passes is
//! the proof the cycle is gone.
//!
//! It is a child module of `trade_compositor` rather than a `tests/` integration
//! test because XUI15's authentication path is private to that module. Reaching
//! it from a sibling would have meant widening the visibility of the opaque
//! authenticated capability, which exists precisely so a UI caller cannot mint
//! one — a test is not a reason to weaken it.

use super::*;
use crate::active_abilities::{
    admit_observer_ability_frame, observer_ability_replay_frame_identity_sha256,
    AdmittedAbilityCatalog, AdmittedAbilityDefinition, ObserverAbilityAuthority,
    ObserverAbilityContext, ObserverAbilityFramePayload, ObserverAbilityLifecycle,
    ObserverAbilityRuntimeRow, ObserverAbilityRuntimeSnapshot, ObserverAbilityScope,
    ObserverAbilityScopeKind, XUI23_EXECUTABLE_SHA256, XUI23_PRODUCER_AUTHORITY_FIELD,
};
use crate::production_lines::{
    seal_xui12_producer_canvas_for_frame_and_viewer, xui12_completeness_record_sha256,
    xui12_replay_frame_identity_sha256, Xui12CanvasLayoutRejection, Xui12ProducerCanvasPayload,
    Xui12ProducerCanvasRow, Xui12ProducerCompletenessAuthority,
    Xui12ProducerCountryCompletenessAuthority, Xui12ProducerFramePayload,
    Xui12ProducerRowPresentation, Xui12ProductionKind, Xui12RowIdentity, Xui12ViewerState,
    ViewerRowPresentation, XUI12_PRODUCER_AUTHORITY_FIELD, XUI12_PRODUCER_PAYLOAD_FIELD,
};
use crate::window_manager_law::ReplayAuthenticatedElementSemantic;
use replay_core::{RawFrame, ReplaySource, SourceError, ValidatedContentAdmission, WorldTimeline};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The hour every carrier in this file binds to. XUI23's runtime rows are
/// validated against it (activation hour + authored duration), so it is shared
/// rather than picked per carrier.
const FRAME_HOUR: i64 = 1_000;
const GER_TAG_DB_ID: u32 = 73;

struct CoexistenceSource {
    rest: Map<String, Value>,
}

impl ReplaySource for CoexistenceSource {
    fn game_id(&self) -> &str {
        "gp-f10-coexistence"
    }

    fn hours(&self) -> &[i64] {
        &[FRAME_HOUR]
    }

    fn warnings(&self) -> &[String] {
        &[]
    }

    fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
        Ok(RawFrame {
            hour_index,
            date: Some("1936.1.2.12".into()),
            save_version: Some("1.19.2".into()),
            schema_version: None,
            rest: self.rest.clone(),
        })
    }
}

fn frame(rest: Map<String, Value>) -> Arc<WorldFrame> {
    WorldTimeline::open(Box::new(CoexistenceSource { rest }))
        .expect("timeline")
        .state_at(FRAME_HOUR)
        .expect("frame")
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("object")
}

/// One country carrying everything all three consumers read off the frame: the
/// D18 identity XUI12 and XUI23 census against, and the complete resource-trade
/// rows XUI15's catalogue roster requires.
fn complete_country() -> Value {
    let rows = TRADE_RESOURCE_ORDER
        .iter()
        .enumerate()
        .map(|(ordinal, resource)| {
            let mut row = json!({
                "resource": resource,
                "visible_order": ordinal,
                "balance": 0.0,
                "produced": 0.0,
                "imported": 0.0,
                "exported": 0.0,
                "special_project": 0.0,
                "production_line": 0.0,
                "needed": 0.0,
                "needed_type": if *resource == "oil" { "current_max" } else { "surplus" }
            });
            if *resource == "oil" {
                row["needed_current"] = json!(0.0);
                row["needed_max"] = json!(0.0);
            }
            row
        })
        .collect::<Vec<_>>();
    json!({
        "_r2_tag_db_id": GER_TAG_DB_ID,
        "economy": {"resource_trade": {
            "resource_rows_complete": true,
            "resource_rows": rows,
            "deals_complete": true,
            "deals": [],
            "factories": {"total": 0, "used": 0, "available": 0}
        }}
    })
}

/// The base frame every carrier in this file binds to.
///
/// It carries NO `map` member on purpose. An earlier revision gave it one, and
/// that hid a real defect: the projection rebuilt an empty `map` object after
/// removing the map-scoped sidecars, so a snapshot whose only map member was a
/// registered sidecar projected differently from one without it, and landing
/// that single carrier moved all three identities. With the map absent here,
/// `an_unrelated_registered_sidecar_landing_does_not_move_any_identity` lands
/// the map-scoped carriers into a `map` that did not exist before — the worst
/// case, not the convenient one.
fn base_rest() -> Map<String, Value> {
    object(json!({"countries": {"GER": complete_country()}}))
}

/// A validated content admission whose manifest digest all three carriers pin.
fn content_admission() -> ValidatedContentAdmission {
    use replay_core::{
        ContentLayerKind, ContentManifest, LayerStorageKind, PlaybackContentInventory,
        PlaybackContentLayer, RecordedContentLayer, SourceGameIdentity, SourceProfileIdentity,
    };
    let source_game = SourceGameIdentity {
        version: "1.19.2".into(),
        build: "a729".into(),
        executable_sha256: "8".repeat(64),
        build_profile: "release".into(),
    };
    let mut source_profile = SourceProfileIdentity {
        profile_id: "gp-f10-profile".into(),
        playset_id: "gp-f10-playset".into(),
        selection_sha256: "0".repeat(64),
    };
    let ordered_layer = RecordedContentLayer {
        ordinal: 0,
        layer_kind: ContentLayerKind::Base,
        display_name: "Base".into(),
        layer_id: "base".into(),
        descriptor_sha256: None,
        source_container_sha256: None,
        storage_kind: LayerStorageKind::Directory,
        replace_paths: Vec::new(),
        required_for_playback: true,
    };
    let mut manifest = ContentManifest {
        schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
        product_modset_sha256: "9".repeat(64),
        source_game: source_game.clone(),
        source_profile: source_profile.clone(),
        effective_vfs_manifest_sha256: "e".repeat(64),
        ordered_layers: vec![ordered_layer.clone()],
        dlc_selection: Vec::new(),
        localisation_language: "l_english".into(),
        required_settings: std::collections::BTreeMap::new(),
        recorder_local_settings: BTreeMap::new(),
        required_content: Vec::new(),
        manifest_digest_sha256: None,
    };
    source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    manifest.source_profile = source_profile.clone();
    manifest = manifest.with_computed_digest().unwrap();
    let inventory = PlaybackContentInventory {
        source_game,
        source_profile,
        layers: vec![PlaybackContentLayer {
            recorded_ordinal: 0,
            layer_kind: ordered_layer.layer_kind,
            layer_id: ordered_layer.layer_id,
            descriptor_sha256: None,
            source_container_sha256: None,
            storage_kind: LayerStorageKind::Directory,
            replace_paths: Vec::new(),
        }],
        dlc_selection: Vec::new(),
        localisation_language: "l_english".into(),
        settings: std::collections::BTreeMap::new(),
        resolved_content: Vec::new(),
    };
    manifest.admit_playback(&inventory).unwrap()
}

fn semantics() -> BTreeMap<String, ReplayAuthenticatedElementSemantic> {
    BTreeMap::from([(
        "row".into(),
        ReplayAuthenticatedElementSemantic::Structural,
    )])
}

fn xui12_completeness(
    payload: &Xui12ProducerFramePayload,
) -> Xui12ProducerCompletenessAuthority {
    Xui12ProducerCompletenessAuthority {
        countries: payload
            .countries
            .iter()
            .map(|country| Xui12ProducerCountryCompletenessAuthority {
                country_tag: country.country_tag.clone(),
                tag_db_id: country.tag_db_id,
                source_ordinal: country.source_ordinal,
                row_identities: country
                    .rows
                    .iter()
                    .map(|row| row.row_identity.clone())
                    .collect(),
            })
            .collect(),
    }
}

fn xui12_payload(manifest_sha256: &str, frame_identity: String) -> Xui12ProducerFramePayload {
    let mut payload = Xui12ProducerFramePayload {
        executable_sha256: map_viewer::XMAP03_A729_EXECUTABLE_SHA256.into(),
        producer_manifest_sha256: manifest_sha256.to_owned(),
        replay_frame_identity_sha256: frame_identity,
        completeness_record_sha256: String::new(),
        countries: vec![Xui12ProducerCanvasPayload {
            country_tag: "GER".into(),
            tag_db_id: GER_TAG_DB_ID,
            source_ordinal: 0,
            measured_empty: false,
            owner_semantics: semantics(),
            rows: vec![Xui12ProducerCanvasRow {
                row_identity: Xui12RowIdentity::ExistingLine {
                    kind: Xui12ProductionKind::Military,
                    line_id: 77,
                },
                global_source_ordinal: 0,
                presentations: BTreeMap::from([
                    (
                        ViewerRowPresentation::Expanded,
                        Xui12ProducerRowPresentation {
                            template_name: "production_military_line_entry".into(),
                            resolved_width_px: 300,
                            resolved_height_px: 80,
                            semantics: semantics(),
                        },
                    ),
                    (
                        ViewerRowPresentation::Collapsed,
                        Xui12ProducerRowPresentation {
                            template_name: "production_military_line_entry_collapsed".into(),
                            resolved_width_px: 300,
                            resolved_height_px: 40,
                            semantics: semantics(),
                        },
                    ),
                ]),
            }],
        }],
    };
    payload.completeness_record_sha256 =
        xui12_completeness_record_sha256(&xui12_completeness(&payload));
    payload
}

fn xui15_completeness(payload: &Xui15ProducerPayload) -> Xui15ProducerCompletenessAuthority {
    Xui15ProducerCompletenessAuthority {
        evaluated_filter_token_ids: payload.evaluated_filter_token_ids.clone(),
        catalogues: payload
            .catalogues
            .iter()
            .map(|catalogue| Xui15ProducerImporterCompletenessAuthority {
                country_tag: catalogue.country_tag.clone(),
                source_ordinal: catalogue.source_ordinal,
                evaluated_country_tags: catalogue.evaluated_country_tags.clone(),
                eligible_country_tags: catalogue.eligible_country_tags.clone(),
            })
            .collect(),
    }
}

fn xui15_payload(manifest_sha256: &str, frame_identity: String) -> Xui15ProducerPayload {
    let mut payload = Xui15ProducerPayload {
        executable_sha256: map_viewer::XMAP03_A729_EXECUTABLE_SHA256.into(),
        producer_manifest_sha256: manifest_sha256.to_owned(),
        replay_frame_identity_sha256: frame_identity,
        completeness_record_sha256: String::new(),
        evaluated_filter_token_ids: vec![1],
        country_filters: vec![Xui15ProducerCountryFilter {
            token_id: 1,
            source_ordinal: 0,
            localisation_key: "TRADE_FILTER_ALL".into(),
        }],
        catalogues: vec![Xui15ProducerSelectedCountryCatalogue {
            country_tag: "GER".into(),
            source_ordinal: 0,
            evaluated_country_tags: vec!["GER".into()],
            eligible_country_tags: Vec::new(),
            countries: Vec::new(),
        }],
    };
    payload.completeness_record_sha256 =
        xui15_completeness_record_sha256(&xui15_completeness(&payload));
    payload
}

fn xui23_catalog() -> AdmittedAbilityCatalog {
    AdmittedAbilityCatalog {
        abilities: vec![AdmittedAbilityDefinition {
            authored_order: 0,
            ability_id: "force_attack".into(),
            target_kind: "army_leader".into(),
            name_key: "ABILITY_FORCE_ATTACK".into(),
            description_key: "ABILITY_FORCE_ATTACK_DESC".into(),
            name: "Force Attack".into(),
            description: "Attack without regard for losses".into(),
            icon: "GFX_ability_force_attack".into(),
            active_icon: "GFX_ability_force_attack_animated".into(),
            base_cost_fraction: 0.22,
            authored_duration_hours: 168,
            authored_cooldown_hours: Some(72),
        }],
    }
}

fn xui23_payload(manifest_sha256: &str, frame_identity: String) -> ObserverAbilityFramePayload {
    let caster = Some(ObserverAbilityScope {
        kind: ObserverAbilityScopeKind::ArmyLeader,
        stable_id: 73,
    });
    ObserverAbilityFramePayload {
        executable_sha256: XUI23_EXECUTABLE_SHA256.into(),
        producer_manifest_sha256: manifest_sha256.to_owned(),
        replay_frame_identity_sha256: frame_identity,
        completeness_record_sha256: "ef".repeat(32),
        source_record_id: "xui23:gp-f10:1000:all-countries".into(),
        authority: ObserverAbilityAuthority {
            executable_sha256: XUI23_EXECUTABLE_SHA256.into(),
            frame_hour: FRAME_HOUR,
            country_roster_complete: true,
            country_count: 1,
        },
        snapshots: vec![ObserverAbilityRuntimeSnapshot {
            country_tag: "GER".into(),
            complete: true,
            context_count: 2,
            contexts: vec![
                ObserverAbilityContext {
                    caster: None,
                    group_target: None,
                },
                ObserverAbilityContext {
                    caster: caster.clone(),
                    group_target: None,
                },
            ],
            row_count: 1,
            rows: vec![ObserverAbilityRuntimeRow {
                ability_id: "force_attack".into(),
                caster,
                group_target: None,
                definition_order: 0,
                target_kind_code: 1,
                available: false,
                lifecycle: ObserverAbilityLifecycle::Active,
                activation_hour: Some(900),
                authored_duration_hours: 168,
                authored_cooldown_hours: Some(72),
                duration_remaining_hours: Some(68),
                cooldown_remaining_hours: None,
                effective_command_power_cost_fixed: -2_200_000,
                effective_command_power_cost_scale: 100_000,
                disabled_reason: None,
            }],
        }],
    }
}

/// Everything a producer would emit for one hour, plus the identities each
/// consumer will re-derive.
struct ThreeCarrierHour {
    admission: ValidatedContentAdmission,
    xui12: Xui12ProducerFramePayload,
    xui15: Xui15ProducerPayload,
    xui23: ObserverAbilityFramePayload,
    identities: [String; 3],
    rest: Map<String, Value>,
}

/// Produce all three carriers in ONE pass over ONE base frame.
///
/// This is the shape a real producer has: it computes the three identities from
/// the base frame BEFORE any carrier exists, writes them into the three
/// payloads, and inserts all five keys. Under the old per-consumer projections
/// the last step invalidated the first, whichever order it was done in.
fn produce_three_carrier_hour(mutate_base: impl FnOnce(&mut Map<String, Value>)) -> ThreeCarrierHour {
    let admission = content_admission();
    let manifest = admission.manifest_digest_sha256().to_owned();

    let mut rest = base_rest();
    mutate_base(&mut rest);
    let base = frame(rest.clone());
    let identities = [
        xui12_replay_frame_identity_sha256(base.as_ref()).expect("XUI12 base-frame identity"),
        xui15_replay_frame_identity_sha256(base.as_ref()).expect("XUI15 base-frame identity"),
        observer_ability_replay_frame_identity_sha256(base.as_ref())
            .expect("XUI23 base-frame identity"),
    ];

    let xui12 = xui12_payload(&manifest, identities[0].clone());
    let xui15 = xui15_payload(&manifest, identities[1].clone());
    let xui23 = xui23_payload(&manifest, identities[2].clone());

    rest.insert(
        XUI12_PRODUCER_PAYLOAD_FIELD.into(),
        serde_json::to_value(&xui12).unwrap(),
    );
    rest.insert(
        XUI12_PRODUCER_AUTHORITY_FIELD.into(),
        xui12.replay_authority_record(FRAME_HOUR, &xui12_completeness(&xui12)),
    );
    rest.insert(
        XUI15_PRODUCER_PAYLOAD_FIELD.into(),
        serde_json::to_value(&xui15).unwrap(),
    );
    rest.insert(
        XUI15_PRODUCER_AUTHORITY_FIELD.into(),
        xui15.replay_authority_record(FRAME_HOUR, &xui15_completeness(&xui15)),
    );
    rest.insert(
        XUI23_PRODUCER_AUTHORITY_FIELD.into(),
        xui23.replay_authority_record(),
    );

    ThreeCarrierHour {
        admission,
        xui12,
        xui15,
        xui23,
        identities,
        rest,
    }
}

/// kills: reverting ANY of the three consumers to "the whole frame minus my own
/// keys". This is the test the GP-F10 finding said could not be written: with
/// two or more real carriers, each per-consumer projection covered the other
/// carriers' payloads, and each payload contains its own digest, so satisfying
/// all three at once was a SHA-256 fixed-point problem. Under the shared
/// base-frame projection the producer computes the three digests once, before
/// any carrier exists, and all three consumers still agree afterwards.
#[test]
fn all_three_identity_bearing_carriers_coexist_and_admit_in_one_frame() {
    let hour = produce_three_carrier_hour(|_| {});
    let full = frame(hour.rest.clone());

    // The fixed point, evaluated: the identities were taken over a base frame
    // that held NO carriers, and each consumer re-derives the same value from
    // the frame that holds all five carrier keys.
    assert_eq!(
        xui12_replay_frame_identity_sha256(full.as_ref()).unwrap(),
        hour.identities[0]
    );
    assert_eq!(
        xui15_replay_frame_identity_sha256(full.as_ref()).unwrap(),
        hour.identities[1]
    );
    assert_eq!(
        observer_ability_replay_frame_identity_sha256(full.as_ref()).unwrap(),
        hour.identities[2]
    );
    // Three carriers, three distinct domains, three distinct digests.
    let mut distinct = hour.identities.to_vec();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 3, "two carriers share a domain separator");

    // And all three admissions pass against that one frame.
    let mut viewer = Xui12ViewerState::default();
    viewer.panel_open = true;
    let sealed = seal_xui12_producer_canvas_for_frame_and_viewer(
        hour.xui12.clone(),
        full.as_ref(),
        &hour.admission,
        "GER",
        &viewer,
    )
    .expect("XUI12 admits with XUI15 and XUI23 present");
    assert_eq!(sealed.replay_frame_identity_sha256(), hour.identities[0]);
    assert_eq!(sealed.rows.len(), 1);

    AuthenticatedXui15Producer::authenticate_from_frame(
        hour.xui15.clone(),
        full.as_ref(),
        hour.admission.manifest_digest_sha256(),
    )
    .expect("XUI15 admits with XUI12 and XUI23 present");

    let admitted = admit_observer_ability_frame(
        full.as_ref(),
        hour.admission.manifest_digest_sha256(),
        &xui23_catalog(),
        "GER",
    )
    .expect("XUI23 admits with XUI12 and XUI15 present");
    assert_eq!(admitted.country_tag, "GER");
    assert_eq!(admitted.rows.len(), 1);
}

/// kills: making the base-frame projection drop game state (for instance by
/// projecting away everything, or by hashing only hour/date/save version). Each
/// consumer must still REFUSE a payload sealed against a different base frame —
/// that refusal is the entire reason a carrier carries an identity. One case
/// per consumer, all three on the same perturbation.
#[test]
fn every_consumer_refuses_a_payload_sealed_against_a_different_base_frame() {
    // Sealed against the ordinary base frame...
    let hour = produce_three_carrier_hour(|_| {});
    // ...and re-hosted in a frame whose base differs by one ordinary game-state
    // value. No sidecar key is touched.
    let mut perturbed = hour.rest.clone();
    perturbed.insert("gp_f10_base_frame_probe".into(), json!(1));
    let stale = frame(perturbed);

    assert_ne!(
        xui12_replay_frame_identity_sha256(stale.as_ref()).unwrap(),
        hour.identities[0],
        "a perturbed base frame must not keep its identity"
    );

    let mut viewer = Xui12ViewerState::default();
    viewer.panel_open = true;
    assert_eq!(
        seal_xui12_producer_canvas_for_frame_and_viewer(
            hour.xui12.clone(),
            stale.as_ref(),
            &hour.admission,
            "GER",
            &viewer,
        ),
        Err(Xui12CanvasLayoutRejection::InvalidProducerAuthority)
    );

    assert_eq!(
        AuthenticatedXui15Producer::authenticate_from_frame(
            hour.xui15.clone(),
            stale.as_ref(),
            hour.admission.manifest_digest_sha256(),
        ),
        Err(Xui15ProducerRejection::InvalidAuthority)
    );

    let refusal = admit_observer_ability_frame(
        stale.as_ref(),
        hour.admission.manifest_digest_sha256(),
        &xui23_catalog(),
        "GER",
    )
    .expect_err("XUI23 must refuse a payload sealed against another base frame");
    assert!(
        refusal.0.contains("does not authenticate this frame"),
        "{}",
        refusal.0
    );

    // The same three payloads, produced against the perturbed base instead, all
    // admit again — so the refusal above is the identity binding and not some
    // unrelated shape check the probe key happened to trip.
    let rehosted = produce_three_carrier_hour(|rest| {
        rest.insert("gp_f10_base_frame_probe".into(), json!(1));
    });
    let rehosted_frame = frame(rehosted.rest.clone());
    assert_ne!(rehosted.identities, hour.identities);
    seal_xui12_producer_canvas_for_frame_and_viewer(
        rehosted.xui12.clone(),
        rehosted_frame.as_ref(),
        &rehosted.admission,
        "GER",
        &viewer,
    )
    .expect("XUI12 admits against the base frame it was sealed for");
    AuthenticatedXui15Producer::authenticate_from_frame(
        rehosted.xui15.clone(),
        rehosted_frame.as_ref(),
        rehosted.admission.manifest_digest_sha256(),
    )
    .expect("XUI15 admits against the base frame it was sealed for");
    admit_observer_ability_frame(
        rehosted_frame.as_ref(),
        rehosted.admission.manifest_digest_sha256(),
        &xui23_catalog(),
        "GER",
    )
    .expect("XUI23 admits against the base frame it was sealed for");
    let _ = &rehosted.xui23;
}

/// kills: dropping a registered sidecar key from the projection's exclusion
/// set. An unrelated Wave-2 carrier landing in the same hour must not move any
/// of the three identities — that perturbation is the second half of GP-F10,
/// and it is what made "add one key" a silent whole-hour rejection for three
/// carriers a lane had never heard of.
#[test]
fn an_unrelated_registered_sidecar_landing_does_not_move_any_identity() {
    let hour = produce_three_carrier_hour(|_| {});
    let mut with_neighbour = hour.rest.clone();
    for key in snapshot::replay_frame_identity::REGISTERED_ROOT_SIDECAR_KEYS {
        with_neighbour
            .entry(key.to_owned())
            .or_insert_with(|| json!({"landed": key}));
    }
    // The map-scoped carriers land into a `map` member that did not exist in
    // the base frame at all. If the projection left an empty `map` behind, this
    // frame would differ from the base and every identity below would move.
    assert!(!with_neighbour.contains_key("map"));
    let mut map_scope = Map::new();
    for key in snapshot::replay_frame_identity::REGISTERED_MAP_SIDECAR_KEYS {
        map_scope.insert(key.to_owned(), json!({"landed": key}));
    }
    with_neighbour.insert("map".into(), Value::Object(map_scope));
    let crowded = frame(with_neighbour);

    assert_eq!(
        xui12_replay_frame_identity_sha256(crowded.as_ref()).unwrap(),
        hour.identities[0]
    );
    assert_eq!(
        xui15_replay_frame_identity_sha256(crowded.as_ref()).unwrap(),
        hour.identities[1]
    );
    assert_eq!(
        observer_ability_replay_frame_identity_sha256(crowded.as_ref()).unwrap(),
        hour.identities[2]
    );

    let mut viewer = Xui12ViewerState::default();
    viewer.panel_open = true;
    seal_xui12_producer_canvas_for_frame_and_viewer(
        hour.xui12.clone(),
        crowded.as_ref(),
        &hour.admission,
        "GER",
        &viewer,
    )
    .expect("XUI12 still admits beside every other registered sidecar");
    AuthenticatedXui15Producer::authenticate_from_frame(
        hour.xui15.clone(),
        crowded.as_ref(),
        hour.admission.manifest_digest_sha256(),
    )
    .expect("XUI15 still admits beside every other registered sidecar");
    admit_observer_ability_frame(
        crowded.as_ref(),
        hour.admission.manifest_digest_sha256(),
        &xui23_catalog(),
        "GER",
    )
    .expect("XUI23 still admits beside every other registered sidecar");
}
