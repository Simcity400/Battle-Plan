use super::*;
use std::collections::BTreeMap;

const RECORDER_COUNTRY: u32 = 23;
const OBSERVER_COUNTRY: u32 = 77;
const EMPTY_COUNTRY: u32 = 91;
const BROKEN_COUNTRY: u32 = 105;

struct FixtureSource {
    exact: bool,
    census: Vec<u32>,
    identity: ContentIdentity,
    bodies: BTreeMap<u32, Result<SectionBody, SectionRejection>>,
}

impl ExactTheatreSelectorSource for FixtureSource {
    fn is_exact_a729_theatre_selector_source(&self) -> bool {
        self.exact
    }
    fn read_country_census(&self) -> Result<Vec<u32>, CarrierError> {
        Ok(self.census.clone())
    }
    fn read_content_identity(&self) -> Result<ContentIdentity, CarrierError> {
        Ok(self.identity.clone())
    }
    fn read_country_section(&self, owner_tag_db_id: u32) -> Result<SectionBody, SectionRejection> {
        self.bodies
            .get(&owner_tag_db_id)
            .cloned()
            .unwrap_or(Err(SectionRejection::IncompleteWalk))
    }
}

fn identity() -> ContentIdentity {
    ContentIdentity {
        language: "l_english".into(),
        localization_content_sha256: [7; 32],
        ordered_layers: vec![
            ContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                content_sha256: [1; 32],
            },
            ContentLayer {
                ordinal: 1,
                layer_kind: ContentLayerKind::Mod,
                layer_id: "ugc_1234".into(),
                content_sha256: [2; 32],
            },
        ],
        engine_roster: engine_roster(),
    }
}

fn engine_roster() -> EngineContentRoster {
    EngineContentRoster {
        requested_mods: vec!["ugc_1234".into()],
        requested_disabled_dlcs: vec!["dlc/quiet/quiet.dlc".into()],
        discovered: vec![EngineRosterEntry {
            kind: ContentLayerKind::Mod,
            descriptor_entry: "ugc_1234".into(),
            name: "Example Mod".into(),
            selected: true,
            mounted: true,
        }],
        roster_sha256: [9; 32],
    }
}

fn land_body(owner: u32, theatre_id: u32) -> SectionBody {
    SectionBody {
        theatres: vec![TheatreRow {
            kind: ObjectKind::LandTheatre,
            object_id: theatre_id,
            owner_tag_db_id: owner,
            engine_order: 0,
            display_name: "Western Front".into(),
            name_provenance: NameProvenance::EngineRuntimeString,
            priority_ordinal: PriorityOrdinal::Measured(2),
            delete_eligible: Some(true),
            members: vec![
                MemberRow {
                    kind: ObjectKind::ArmyGroup,
                    object_id: theatre_id + 1,
                    parent_kind: ObjectKind::LandTheatre,
                    parent_id: theatre_id,
                    engine_order: 0,
                    reserve_navy: false,
                    display_name: None,
                    name_provenance: None,
                    priority_ordinal: PriorityOrdinal::Unavailable,
                    orders_visibility: Some(ArmyOrdersVisibility {
                        training_visible: true,
                        stop_at_max_visible: true,
                        border_conflict_visible: false,
                    }),
                },
                MemberRow {
                    kind: ObjectKind::Army,
                    object_id: theatre_id + 2,
                    parent_kind: ObjectKind::ArmyGroup,
                    parent_id: theatre_id + 1,
                    engine_order: 0,
                    reserve_navy: false,
                    display_name: None,
                    name_provenance: None,
                    priority_ordinal: PriorityOrdinal::Unavailable,
                    orders_visibility: Some(ArmyOrdersVisibility {
                        training_visible: false,
                        stop_at_max_visible: false,
                        border_conflict_visible: true,
                    }),
                },
            ],
            alert_categories: vec![3, 9],
        }],
        free_air_wings: vec![],
    }
}

fn air_body(owner: u32) -> SectionBody {
    SectionBody {
        theatres: vec![TheatreRow {
            kind: ObjectKind::AirTheatre,
            object_id: 400,
            owner_tag_db_id: owner,
            engine_order: 0,
            display_name: "Home Air".into(),
            name_provenance: NameProvenance::EngineRuntimeString,
            priority_ordinal: PriorityOrdinal::Unavailable,
            delete_eligible: None,
            members: vec![
                MemberRow {
                    kind: ObjectKind::AirGroup,
                    object_id: 500,
                    parent_kind: ObjectKind::AirTheatre,
                    parent_id: 400,
                    engine_order: 0,
                    reserve_navy: false,
                    display_name: Some("Fighter Group".into()),
                    name_provenance: Some(NameProvenance::EngineRuntimeString),
                    priority_ordinal: PriorityOrdinal::Unavailable,
                    orders_visibility: None,
                },
                MemberRow {
                    kind: ObjectKind::AirWing,
                    object_id: 600,
                    parent_kind: ObjectKind::AirGroup,
                    parent_id: 500,
                    engine_order: 0,
                    reserve_navy: false,
                    display_name: None,
                    name_provenance: None,
                    priority_ordinal: PriorityOrdinal::Unavailable,
                    orders_visibility: None,
                },
            ],
            alert_categories: vec![],
        }],
        free_air_wings: vec![FreeAirWingRow {
            wing_id: 601,
            engine_order: 0,
            priority_ordinal: PriorityOrdinal::Unavailable,
        }],
    }
}

fn fixture() -> FixtureSource {
    let mut bodies = BTreeMap::new();
    bodies.insert(RECORDER_COUNTRY, Ok(land_body(RECORDER_COUNTRY, 100)));
    bodies.insert(OBSERVER_COUNTRY, Ok(air_body(OBSERVER_COUNTRY)));
    bodies.insert(EMPTY_COUNTRY, Ok(SectionBody::default()));
    bodies.insert(
        BROKEN_COUNTRY,
        Err(SectionRejection::IdentityAuthenticationFailed),
    );
    FixtureSource {
        exact: true,
        census: vec![
            RECORDER_COUNTRY,
            OBSERVER_COUNTRY,
            EMPTY_COUNTRY,
            BROKEN_COUNTRY,
        ],
        identity: identity(),
        bodies,
    }
}

fn produced() -> TheatreSelectorFrame {
    let payload = collect_and_encode(&fixture(), 900, 901, 7).expect("fixture encodes");
    decode_payload(&payload).expect("fixture decodes")
}

fn exact_frame() -> TheatreSelectorFrame {
    produced()
}

fn reencode(frame: &TheatreSelectorFrame) -> Result<TheatreSelectorFrame, CarrierError> {
    decode_payload(&encode_payload(frame)?)
}

#[test]
fn every_selectable_country_gets_a_section() {
    let frame = produced();
    let snapshot = frame.snapshot.expect("exact frame carries sections");
    assert_eq!(
        snapshot.census,
        vec![
            RECORDER_COUNTRY,
            OBSERVER_COUNTRY,
            EMPTY_COUNTRY,
            BROKEN_COUNTRY
        ]
    );
    assert_eq!(snapshot.sections.len(), snapshot.census.len());
    for (section, owner) in snapshot.sections.iter().zip(snapshot.census.iter()) {
        assert_eq!(section.owner_tag_db_id, *owner);
    }
}

#[test]
fn observer_country_is_served_without_recorder_pov() {
    let frame = produced();
    let snapshot = frame.snapshot.expect("exact frame carries sections");
    let section = snapshot
        .section(OBSERVER_COUNTRY)
        .expect("observer country present");
    let SectionAdmission::Admitted(body) = &section.admission else {
        panic!("observer country must be admitted, not unavailable");
    };
    assert_eq!(body.theatres.len(), 1);
    assert_eq!(body.theatres[0].kind, ObjectKind::AirTheatre);
    assert_eq!(body.free_air_wings.len(), 1);
}

#[test]
fn measured_empty_country_is_admitted_not_unavailable() {
    let frame = produced();
    let snapshot = frame.snapshot.expect("exact frame carries sections");
    let section = snapshot
        .section(EMPTY_COUNTRY)
        .expect("measured-empty country present");
    let SectionAdmission::Admitted(body) = &section.admission else {
        panic!("measured empty must stay admitted");
    };
    assert!(body.theatres.is_empty());
    assert!(body.free_air_wings.is_empty());
}

#[test]
fn rejected_country_is_unavailable_and_isolated() {
    let frame = produced();
    let snapshot = frame.snapshot.expect("exact frame carries sections");
    let broken = snapshot
        .section(BROKEN_COUNTRY)
        .expect("broken country still gets a section");
    assert_eq!(
        broken.admission,
        SectionAdmission::Unavailable(SectionRejection::IdentityAuthenticationFailed)
    );
    assert!(matches!(
        snapshot.section(RECORDER_COUNTRY).unwrap().admission,
        SectionAdmission::Admitted(_)
    ));
    assert!(matches!(
        snapshot.section(OBSERVER_COUNTRY).unwrap().admission,
        SectionAdmission::Admitted(_)
    ));
}

#[test]
fn country_outside_the_census_has_no_section() {
    let frame = produced();
    let snapshot = frame.snapshot.expect("exact frame carries sections");
    assert!(snapshot.section(999).is_none());
}

#[test]
fn producer_complete_tracks_section_admission() {
    let frame = produced();
    let snapshot = frame.snapshot.expect("exact frame carries sections");
    assert!(!snapshot.producer_complete);

    let mut source = fixture();
    source
        .bodies
        .insert(BROKEN_COUNTRY, Ok(SectionBody::default()));
    let clean = decode_payload(&collect_and_encode(&source, 900, 901, 7).unwrap()).unwrap();
    assert!(clean.snapshot.unwrap().producer_complete);
}

#[test]
fn producer_complete_contradiction_refuses() {
    let mut frame = exact_frame();
    frame.snapshot.as_mut().unwrap().producer_complete = true;
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn extra_section_beyond_the_census_refuses() {
    let mut frame = exact_frame();
    let sections = &mut frame.snapshot.as_mut().unwrap().sections;
    let extra = sections[0].clone();
    sections.push(extra);
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn section_order_must_follow_the_census() {
    let mut frame = exact_frame();
    let snapshot = frame.snapshot.as_mut().unwrap();
    snapshot.census = vec![RECORDER_COUNTRY, OBSERVER_COUNTRY];
    snapshot.sections = vec![
        empty_section(OBSERVER_COUNTRY),
        empty_section(RECORDER_COUNTRY),
    ];
    snapshot.producer_complete = true;
    assert!(encode_payload(&frame).is_err());
}

fn empty_section(owner: u32) -> CountrySection {
    CountrySection {
        owner_tag_db_id: owner,
        admission: SectionAdmission::Admitted(SectionBody::default()),
    }
}

#[test]
fn census_must_be_strictly_ascending_and_nonzero() {
    for census in [
        vec![OBSERVER_COUNTRY, RECORDER_COUNTRY],
        vec![RECORDER_COUNTRY, RECORDER_COUNTRY],
        vec![0, RECORDER_COUNTRY],
    ] {
        let mut frame = exact_frame();
        let snapshot = frame.snapshot.as_mut().unwrap();
        snapshot.sections = census.iter().copied().map(empty_section).collect();
        snapshot.census = census.clone();
        snapshot.producer_complete = true;
        assert!(
            encode_payload(&frame).is_err(),
            "census {census:?} must be refused"
        );
    }
}

#[test]
fn cross_owner_theatre_row_rejects_the_section() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[0].admission
    else {
        panic!("fixture section 0 is admitted");
    };
    body.theatres[0].owner_tag_db_id = OBSERVER_COUNTRY;
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn non_contiguous_theatre_order_rejects_the_section() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[0].admission
    else {
        panic!("fixture section 0 is admitted");
    };
    body.theatres[0].engine_order = 1;
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn duplicate_theatre_identity_rejects_the_section() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[0].admission
    else {
        panic!("fixture section 0 is admitted");
    };
    let mut clone = body.theatres[0].clone();
    clone.engine_order = 1;
    body.theatres.push(clone);
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn duplicate_member_identity_rejects_the_section() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[0].admission
    else {
        panic!("fixture section 0 is admitted");
    };
    let mut clone = body.theatres[0].members[0].clone();
    clone.engine_order = 1;
    body.theatres[0].members.push(clone);
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn orphan_member_rejects_the_section() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[1].admission
    else {
        panic!("fixture section 1 is admitted");
    };
    body.theatres[0].members.remove(0);
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn free_wing_order_must_be_contiguous_within_a_section() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[1].admission
    else {
        panic!("fixture section 1 is admitted");
    };
    body.free_air_wings[0].engine_order = 4;
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn alert_categories_are_land_only_and_ascending() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[1].admission
    else {
        panic!("fixture section 1 is admitted");
    };
    body.theatres[0].alert_categories = vec![1];
    assert!(encode_payload(&frame).is_err());

    let mut descending = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut descending.snapshot.as_mut().unwrap().sections[0].admission
    else {
        panic!("fixture section 0 is admitted");
    };
    body.theatres[0].alert_categories = vec![9, 3];
    assert!(encode_payload(&descending).is_err());
}

#[test]
fn executable_and_manifest_identity_mismatches_refuse() {
    let payload = encode_payload(&exact_frame()).unwrap();
    let first_hash = 4 * 4 + 3 * 8;
    for hash_index in 0..5 {
        let mut corrupted = payload.clone();
        corrupted[first_hash + hash_index * 32] ^= 0xff;
        assert!(
            decode_payload(&corrupted).is_err(),
            "hash {hash_index} must be authenticated"
        );
    }
}

#[test]
fn schema_version_mismatch_refuses() {
    let mut payload = encode_payload(&exact_frame()).unwrap();
    payload[4] = 1;
    assert!(decode_payload(&payload).is_err());
}

#[test]
fn truncation_and_trailing_bytes_refuse() {
    let payload = encode_payload(&exact_frame()).unwrap();
    assert_eq!(
        decode_payload(&payload[..payload.len() - 1]).unwrap_err(),
        CarrierError::Truncated
    );
    let mut extended = payload.clone();
    extended.push(0);
    assert_eq!(
        decode_payload(&extended).unwrap_err(),
        CarrierError::TrailingBytes
    );
}

#[test]
fn zero_source_sequence_refuses() {
    let mut frame = exact_frame();
    frame.source_sequence = 0;
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn source_sequence_must_strictly_increase() {
    let mut gate = SourceSequenceGate::new();
    assert!(gate.admit(0).is_err());
    assert!(gate.admit(10).is_ok());
    assert!(gate.admit(10).is_err());
    assert!(gate.admit(9).is_err());
    assert!(gate.admit(11).is_ok());
}

#[test]
fn exact_frame_without_content_identity_refuses() {
    let mut frame = exact_frame();
    frame.content_identity = None;
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn content_layer_order_and_uniqueness_are_enforced() {
    let mut gapped = exact_frame();
    gapped.content_identity.as_mut().unwrap().ordered_layers[1].ordinal = 5;
    assert!(encode_payload(&gapped).is_err());

    let mut duplicated = exact_frame();
    let layers = &mut duplicated.content_identity.as_mut().unwrap().ordered_layers;
    layers[1].layer_kind = ContentLayerKind::Base;
    layers[1].layer_id = "base".into();
    assert!(encode_payload(&duplicated).is_err());
}

#[test]
fn missing_language_refuses() {
    let mut frame = exact_frame();
    frame.content_identity.as_mut().unwrap().language = String::new();
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn unavailable_frame_is_dirty_and_bodyless() {
    assert!(encode_source_unavailable(900, 901, 7, 0).is_err());
    let decoded = decode_payload(&encode_source_unavailable(900, 901, 7, 1).unwrap()).unwrap();
    assert_eq!(decoded.authority, AuthorityState::SourceUnavailable);
    assert!(decoded.snapshot.is_none());
    assert!(decoded.content_identity.is_none());
    assert_eq!(decoded.source_sequence, 901);
}

#[test]
fn unauthenticated_source_refuses_to_produce() {
    let mut source = fixture();
    source.exact = false;
    assert!(matches!(
        collect_and_encode(&source, 900, 901, 7),
        Err(CarrierError::Source(_))
    ));
}

#[test]
fn priority_disposition_is_enforced_in_both_directions() {
    assert!(encode_payload(&exact_frame()).is_ok());

    let mutate = |section: usize, apply: &dyn Fn(&mut SectionBody)| {
        let mut frame = exact_frame();
        let SectionAdmission::Admitted(body) =
            &mut frame.snapshot.as_mut().unwrap().sections[section].admission
        else {
            panic!("fixture section {section} is admitted");
        };
        apply(body);
        encode_payload(&frame)
    };

    assert!(mutate(0, &|body| {
        body.theatres[0].priority_ordinal = PriorityOrdinal::Unavailable;
    })
    .is_err());
    assert!(mutate(1, &|body| {
        body.theatres[0].priority_ordinal = PriorityOrdinal::Measured(0);
    })
    .is_err());
    assert!(mutate(0, &|body| {
        body.theatres[0].members[0].priority_ordinal = PriorityOrdinal::Measured(1);
    })
    .is_err());
    assert!(mutate(1, &|body| {
        body.free_air_wings[0].priority_ordinal = PriorityOrdinal::Measured(1);
    })
    .is_err());
}

#[test]
fn unavailable_priority_is_distinct_from_measured_zero() {
    let mut unavailable_bytes = Vec::new();
    priority(&mut unavailable_bytes, PriorityOrdinal::Unavailable);
    let mut measured_bytes = Vec::new();
    priority(&mut measured_bytes, PriorityOrdinal::Measured(0));
    assert_ne!(
        unavailable_bytes, measured_bytes,
        "an unmeasured priority must not encode identically to a measured zero"
    );

    let decode = |bytes: &[u8]| {
        Reader {
            payload: bytes,
            at: 0,
        }
        .priority()
    };
    assert_eq!(decode(&unavailable_bytes), Ok(PriorityOrdinal::Unavailable));
    assert_eq!(decode(&measured_bytes), Ok(PriorityOrdinal::Measured(0)));
    assert_eq!(PriorityOrdinal::Unavailable.measured(), None);
    assert_eq!(PriorityOrdinal::Measured(0).measured(), Some(0));
}

#[test]
fn unavailable_priority_with_nonzero_ordinal_refuses() {
    let mut forged = Vec::new();
    priority(&mut forged, PriorityOrdinal::Measured(3));
    assert_eq!(forged[0], 1, "the first byte is the priority tag");
    forged[0] = 0;
    assert!(
        Reader {
            payload: &forged,
            at: 0
        }
        .priority()
        .is_err(),
        "an unavailable priority carrying a nonzero ordinal must refuse"
    );
}

#[test]
fn member_name_and_provenance_must_agree() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[1].admission
    else {
        panic!("fixture section 1 is admitted");
    };
    body.theatres[0].members[0].name_provenance = None;
    assert!(encode_payload(&frame).is_err());

    let mut orphaned = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut orphaned.snapshot.as_mut().unwrap().sections[1].admission
    else {
        panic!("fixture section 1 is admitted");
    };
    body.theatres[0].members[1].name_provenance = Some(NameProvenance::EngineRuntimeString);
    assert!(encode_payload(&orphaned).is_err());
}

#[test]
fn localization_keyed_name_requires_a_key() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[0].admission
    else {
        panic!("fixture section 0 is admitted");
    };
    body.theatres[0].name_provenance = NameProvenance::LocalizationKey(String::new());
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn uncaptured_controller_fields_are_declared() {
    assert_eq!(UNCAPTURED_CONTROLLER_FIELDS.len(), 18);
    for captured in [
        "theater_group_settings_view.current_priority_ordinal",
        "theater_group_settings_view.delete_eligibility",
        "theater_group_item.supply_priority_ordinal",
        "navy_theater_group_item.current_priority_ordinal",
        "orders_group_item.army_training_state",
        "orders_group_item.exercise_stop_at_max_state",
        "orders_group_item.border_conflict_state",
    ] {
        assert!(
            !UNCAPTURED_CONTROLLER_FIELDS.contains(&captured),
            "{captured} is measured now and must not be declared uncaptured"
        );
    }
    for permanent in [
        "theater_group_item.combat_log_ownership_and_eligibility",
        "navy_theater_group_item.combat_log_ownership_and_eligibility",
        "reserve_navy_theater_group_item.combat_log_ownership_and_eligibility",
    ] {
        assert!(
            UNCAPTURED_CONTROLLER_FIELDS.contains(&permanent),
            "{permanent} is client-local UI state and must stay declared uncaptured"
        );
    }
    let mut sorted = UNCAPTURED_CONTROLLER_FIELDS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), UNCAPTURED_CONTROLLER_FIELDS.len());
    for field in UNCAPTURED_CONTROLLER_FIELDS {
        assert!(field.contains('.'), "{field} must name template.field");
    }
}

#[test]
fn one_member_under_two_theatres_rejects_the_section() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[0].admission
    else {
        panic!("fixture section 0 is admitted");
    };
    let shared_fleet = |theatre_id: u32, order: u32| TheatreRow {
        kind: ObjectKind::NavyTheatre,
        object_id: theatre_id,
        owner_tag_db_id: RECORDER_COUNTRY,
        engine_order: order,
        display_name: "Home Fleet Area".into(),
        name_provenance: NameProvenance::EngineRuntimeString,
        priority_ordinal: PriorityOrdinal::Measured(1),
        delete_eligible: None,
        members: vec![MemberRow {
            kind: ObjectKind::Fleet,
            object_id: 7_000,
            parent_kind: ObjectKind::NavyTheatre,
            parent_id: theatre_id,
            engine_order: 0,
            reserve_navy: false,
            display_name: None,
            name_provenance: None,
            priority_ordinal: PriorityOrdinal::Unavailable,
            orders_visibility: None,
        }],
        alert_categories: Vec::new(),
    };
    body.theatres.push(shared_fleet(800, 0));
    body.theatres.push(shared_fleet(801, 1));
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn a_wing_that_is_both_grouped_and_free_rejects_the_section() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[1].admission
    else {
        panic!("fixture section 1 is admitted");
    };
    body.free_air_wings.insert(
        0,
        FreeAirWingRow {
            wing_id: 600,
            engine_order: 0,
            priority_ordinal: PriorityOrdinal::Unavailable,
        },
    );
    for (index, wing) in body.free_air_wings.iter_mut().enumerate() {
        wing.engine_order = index as u32;
    }
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn member_and_free_wing_priority_are_explicitly_unavailable() {
    let frame = produced();
    let snapshot = frame.snapshot.unwrap();
    let mut members = 0;
    let mut wings = 0;
    for section in &snapshot.sections {
        if let SectionAdmission::Admitted(body) = &section.admission {
            for theatre in &body.theatres {
                for member in &theatre.members {
                    assert_eq!(member.priority_ordinal, PriorityOrdinal::Unavailable);
                    members += 1;
                }
            }
            for wing in &body.free_air_wings {
                assert_eq!(wing.priority_ordinal, PriorityOrdinal::Unavailable);
                wings += 1;
            }
        }
    }
    assert!(members > 0 && wings > 0, "fixture must exercise both rows");
}

#[test]
fn every_declared_hole_names_a_contract_template_and_leaf() {
    const CONTRACT_FAMILIES: &[&str] = &[
        "theater_group_settings_view",
        "orders_group_item",
        "navy_theater_fleet_item",
        "theater_group_item",
        "navy_theater_group_item",
        "reserve_navy_theater_group_item",
        "new_theater_group_item",
        "strategic_region_alert_entry",
        "strategic_region_item",
        "fleet_row_item_box",
        "reserve_fleet_row_item_box",
        "air_theatre_wing_item",
        "air_theatre_item",
        "air_theatre_free_wings_item",
        "air_theatre_group_item",
    ];
    for field in UNCAPTURED_CONTROLLER_FIELDS {
        let (family, leaf) = field
            .split_once('.')
            .unwrap_or_else(|| panic!("{field} must name template.field"));
        assert!(!leaf.is_empty(), "{field} must name a leaf");
        assert!(
            CONTRACT_FAMILIES.contains(&family),
            "{family} is not an XUI07 dependency-contract template family"
        );
    }
}

#[test]
fn every_contract_template_family_is_declared_uncaptured() {
    for family in [
        "navy_theater_fleet_item",
        "theater_group_item",
        "navy_theater_group_item",
        "reserve_navy_theater_group_item",
        "new_theater_group_item",
        "strategic_region_alert_entry",
        "strategic_region_item",
        "fleet_row_item_box",
        "reserve_fleet_row_item_box",
        "air_theatre_wing_item",
        "air_theatre_item",
        "air_theatre_free_wings_item",
        "air_theatre_group_item",
    ] {
        assert!(
            UNCAPTURED_CONTROLLER_FIELDS
                .iter()
                .any(|field| field.starts_with(&format!("{family}."))),
            "{family} has no declared uncaptured field"
        );
    }
}

#[test]
fn all_country_frame_round_trips_exactly() {
    let frame = exact_frame();
    assert_eq!(reencode(&frame).unwrap(), frame);
}

#[test]
fn a_disagreement_between_the_disk_stack_and_the_engine_survives_the_wire() {
    let mut frame = exact_frame();
    let identity = frame.content_identity.as_mut().unwrap();
    assert!(identity
        .ordered_layers
        .iter()
        .any(|layer| layer.layer_id == "ugc_1234"));
    identity.engine_roster.discovered[0].mounted = false;
    identity
        .engine_roster
        .requested_mods
        .push("mod/vanished.mod".into());

    let decoded = reencode(&frame).expect("a disagreement is recordable, never an error");
    assert_eq!(decoded, frame);

    let roster = &decoded.content_identity.as_ref().unwrap().engine_roster;
    assert!(
        roster.discovered.iter().any(|e| e.selected && !e.mounted),
        "selected-but-unmounted must be readable off the wire"
    );
    let discovered: Vec<&str> = roster
        .discovered
        .iter()
        .map(|e| e.descriptor_entry.as_str())
        .collect();
    assert!(
        !discovered.contains(&"mod/vanished.mod"),
        "requested-but-undiscovered must be readable as a request with no entry"
    );
    assert!(roster
        .requested_mods
        .contains(&"mod/vanished.mod".to_string()));
}

#[test]
fn an_empty_engine_roster_round_trips() {
    let mut frame = exact_frame();
    frame.content_identity.as_mut().unwrap().engine_roster = EngineContentRoster::default();
    assert_eq!(reencode(&frame).unwrap(), frame);
}

#[test]
fn a_roster_entry_may_carry_an_empty_name_or_descriptor_entry() {
    let mut frame = exact_frame();
    let entry = &mut frame
        .content_identity
        .as_mut()
        .unwrap()
        .engine_roster
        .discovered[0];
    entry.name = String::new();
    entry.descriptor_entry = String::new();
    let decoded = reencode(&frame).expect("empty engine strings are representable");
    assert_eq!(decoded, frame);
    assert!(
        decoded.content_identity.unwrap().engine_roster.discovered[0]
            .name
            .is_empty()
    );
}

#[test]
fn a_roster_entry_claiming_to_be_a_base_or_integrated_layer_is_refused() {
    for kind in [ContentLayerKind::Base, ContentLayerKind::IntegratedDlc] {
        let mut frame = exact_frame();
        frame
            .content_identity
            .as_mut()
            .unwrap()
            .engine_roster
            .discovered[0]
            .kind = kind;
        assert!(
            encode_payload(&frame).is_err(),
            "{kind:?} must not be a roster entry kind"
        );
    }
}

#[test]
fn two_roster_entries_may_share_a_descriptor_entry() {
    let mut frame = exact_frame();
    let roster = &mut frame.content_identity.as_mut().unwrap().engine_roster;
    let twin = roster.discovered[0].clone();
    roster.discovered.push(twin);
    assert_eq!(reencode(&frame).unwrap(), frame);
}

#[test]
fn an_over_long_roster_string_is_refused() {
    let mut frame = exact_frame();
    frame
        .content_identity
        .as_mut()
        .unwrap()
        .engine_roster
        .discovered[0]
        .descriptor_entry = "m".repeat(MAX_TOKEN_BYTES + 1);
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn an_empty_requested_entry_is_refused() {
    let mut frame = exact_frame();
    frame
        .content_identity
        .as_mut()
        .unwrap()
        .engine_roster
        .requested_mods
        .push(String::new());
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn a_payload_from_the_previous_schema_is_rejected_rather_than_misread() {
    let mut payload = encode_payload(&exact_frame()).unwrap();
    payload[4..8].copy_from_slice(&2u32.to_le_bytes());
    assert!(matches!(
        decode_payload(&payload),
        Err(CarrierError::Invalid(_))
    ));
}

#[test]
fn the_roster_digest_travels_byte_for_byte() {
    let mut frame = exact_frame();
    frame
        .content_identity
        .as_mut()
        .unwrap()
        .engine_roster
        .roster_sha256 = [0xab; 32];
    let decoded = reencode(&frame).unwrap();
    assert_eq!(
        decoded
            .content_identity
            .unwrap()
            .engine_roster
            .roster_sha256,
        [0xab; 32]
    );
}

#[test]
fn measured_controller_leaves_survive_a_round_trip() {
    let frame = exact_frame();
    let decoded =
        decode_payload(&encode_payload(&frame).expect("fixture encodes")).expect("fixture decodes");
    let SectionAdmission::Admitted(body) =
        &decoded.snapshot.as_ref().unwrap().sections[0].admission
    else {
        panic!("fixture section 0 is admitted");
    };
    let land = &body.theatres[0];
    assert_eq!(land.priority_ordinal, PriorityOrdinal::Measured(2));
    assert_eq!(land.delete_eligible, Some(true));
    assert_eq!(
        land.members[0].orders_visibility,
        Some(ArmyOrdersVisibility {
            training_visible: true,
            stop_at_max_visible: true,
            border_conflict_visible: false,
        })
    );
    assert_eq!(
        land.members[1].orders_visibility,
        Some(ArmyOrdersVisibility {
            training_visible: false,
            stop_at_max_visible: false,
            border_conflict_visible: true,
        })
    );
    assert_eq!(decoded, frame);
}

#[test]
fn delete_eligibility_is_confined_to_land_theatres() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[1].admission
    else {
        panic!("fixture section 1 is admitted");
    };
    body.theatres[0].delete_eligible = Some(false);
    assert!(encode_payload(&frame).is_err());

    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[0].admission
    else {
        panic!("fixture section 0 is admitted");
    };
    body.theatres[0].delete_eligible = None;
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn orders_group_visibility_is_confined_to_army_rows() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[1].admission
    else {
        panic!("fixture section 1 is admitted");
    };
    body.theatres[0].members[0].orders_visibility = Some(ArmyOrdersVisibility {
        training_visible: false,
        stop_at_max_visible: false,
        border_conflict_visible: false,
    });
    assert!(encode_payload(&frame).is_err());

    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[0].admission
    else {
        panic!("fixture section 0 is admitted");
    };
    body.theatres[0].members[0].orders_visibility = None;
    assert!(encode_payload(&frame).is_err());
}

#[test]
fn orders_group_visibility_pairs_the_engine_cannot_produce_are_refused() {
    for illegal in [
        ArmyOrdersVisibility {
            training_visible: false,
            stop_at_max_visible: true,
            border_conflict_visible: false,
        },
        ArmyOrdersVisibility {
            training_visible: true,
            stop_at_max_visible: false,
            border_conflict_visible: true,
        },
    ] {
        let mut frame = exact_frame();
        let SectionAdmission::Admitted(body) =
            &mut frame.snapshot.as_mut().unwrap().sections[0].admission
        else {
            panic!("fixture section 0 is admitted");
        };
        body.theatres[0].members[0].orders_visibility = Some(illegal);
        assert!(
            encode_payload(&frame).is_err(),
            "illegal orders pair {illegal:?} must not encode"
        );

        let mut forged = Vec::new();
        orders_visibility(&mut forged, Some(illegal));
        assert_eq!(forged[0], 1, "the first byte is the presence flag");
        assert!(
            Reader {
                payload: &forged,
                at: 0
            }
            .orders_visibility()
            .is_err(),
            "illegal orders pair {illegal:?} must not decode"
        );
    }
}

#[test]
fn an_absent_controller_leaf_with_a_set_value_byte_is_refused() {
    assert_eq!(
        Reader {
            payload: &[0, 0, 0, 0],
            at: 0
        }
        .opt_bool(),
        Ok(None),
        "a cleanly absent optional bool decodes as absent"
    );
    assert!(
        Reader {
            payload: &[0, 1, 0, 0],
            at: 0
        }
        .opt_bool()
        .is_err(),
        "an absent optional bool carrying a true value is a contradiction"
    );

    assert_eq!(
        Reader {
            payload: &[0, 0, 0, 0],
            at: 0
        }
        .orders_visibility(),
        Ok(None)
    );
    for forged in [[0u8, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]] {
        assert!(
            Reader {
                payload: &forged,
                at: 0
            }
            .orders_visibility()
            .is_err(),
            "absent orders visibility carrying {forged:?} is a contradiction"
        );
    }
}
