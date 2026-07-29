//! Falsifiable admission tests for the `THS2` all-country carrier.
//!
//! Every test here names, in its doc comment, the production change that turns
//! it red. A test whose named mutation cannot be applied is vacuous and must be
//! deleted rather than kept for coverage.

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
            priority_ordinal: PriorityOrdinal::Unavailable,
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

// ---------------------------------------------------------------------------
// All-country enumeration
// ---------------------------------------------------------------------------

/// Red when `collect_and_encode` stops enumerating the census and emits only
/// the recorder's own country (the `THS1` behaviour this carrier replaces).
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

/// Red when the producer restricts its walk to the recorder's current country:
/// the observer country's air hierarchy would then be missing or unavailable.
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

/// Red when a country with no theatres is transported as `Unavailable`
/// instead of an admitted, measured-empty section -- the two carry opposite
/// meanings to the panel.
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

/// Red when a per-country failure is converted into an empty admitted body, or
/// when it aborts the whole frame instead of only its own section.
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
    // Neighbours survive.
    assert!(matches!(
        snapshot.section(RECORDER_COUNTRY).unwrap().admission,
        SectionAdmission::Admitted(_)
    ));
    assert!(matches!(
        snapshot.section(OBSERVER_COUNTRY).unwrap().admission,
        SectionAdmission::Admitted(_)
    ));
}

/// Red when a country absent from the census silently resolves to some other
/// country's section (first-available or recorder-POV fallback).
#[test]
fn country_outside_the_census_has_no_section() {
    let frame = produced();
    let snapshot = frame.snapshot.expect("exact frame carries sections");
    assert!(snapshot.section(999).is_none());
}

/// Red when `producer_complete` is hardcoded true, or stops tracking whether
/// every section was admitted.
#[test]
fn producer_complete_tracks_section_admission() {
    let frame = produced();
    let snapshot = frame.snapshot.expect("exact frame carries sections");
    assert!(!snapshot.producer_complete);

    let mut source = fixture();
    source.bodies.insert(BROKEN_COUNTRY, Ok(SectionBody::default()));
    let clean = decode_payload(&collect_and_encode(&source, 900, 901, 7).unwrap()).unwrap();
    assert!(clean.snapshot.unwrap().producer_complete);
}

/// Red when `validate_snapshot` stops cross-checking the producer-complete
/// flag, letting a frame claim completeness while carrying a refused section.
#[test]
fn producer_complete_contradiction_refuses() {
    let mut frame = exact_frame();
    frame.snapshot.as_mut().unwrap().producer_complete = true;
    assert!(encode_payload(&frame).is_err());
}

// ---------------------------------------------------------------------------
// Section admission laws
// ---------------------------------------------------------------------------

/// Red when the section/census length equality check is removed.
///
/// The extra section duplicates an admitted one, so a length-blind consumer
/// would zip it away and every other guard would still pass: only the length
/// equality can catch it.
#[test]
fn extra_section_beyond_the_census_refuses() {
    let mut frame = exact_frame();
    let sections = &mut frame.snapshot.as_mut().unwrap().sections;
    let extra = sections[0].clone();
    sections.push(extra);
    assert!(encode_payload(&frame).is_err());
}

/// Red when sections are allowed to appear in an order other than the census
/// order, which would let a consumer join the wrong country to a body.
///
/// Both sections are measured-empty on purpose: with no theatre rows, the
/// per-row owner check cannot mask a missing section-order law.
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

/// A country section with nothing in it. Used where a test must isolate a
/// census/section law from the per-row owner checks.
fn empty_section(owner: u32) -> CountrySection {
    CountrySection {
        owner_tag_db_id: owner,
        admission: SectionAdmission::Admitted(SectionBody::default()),
    }
}

/// Red when the census ascending/nonzero law is dropped, which would admit a
/// duplicated or zero country identity.
///
/// Each case rebuilds the sections to agree with the bad census, so the
/// section-order and per-row owner checks cannot mask the missing law.
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

/// Red when the per-row owner check is removed, letting one country's section
/// carry another country's theatre.
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

/// Red when contiguous engine order per kind stops being enforced, which is
/// how an out-of-order or dropped theatre row would slip through as a prefix.
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

/// Red when duplicate theatre identities are admitted inside one section.
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

/// Red when duplicate member identities are admitted inside one theatre.
#[test]
fn duplicate_member_identity_rejects_the_section() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[0].admission
    else {
        panic!("fixture section 0 is admitted");
    };
    let mut clone = body.theatres[0].members[0].clone();
    // The next engine order satisfies the contiguity law, so only the
    // identity-uniqueness law can catch this.
    clone.engine_order = 1;
    body.theatres[0].members.push(clone);
    assert!(encode_payload(&frame).is_err());
}

/// Red when the parent-existence law is dropped: an air wing whose group is
/// not carried would become an orphan row the panel cannot place.
#[test]
fn orphan_member_rejects_the_section() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[1].admission
    else {
        panic!("fixture section 1 is admitted");
    };
    body.theatres[0].members.remove(0); // drop the AirGroup parent
    assert!(encode_payload(&frame).is_err());
}

/// Red when free-wing ordering stops being per-section contiguous, which would
/// let two countries' free wings interleave into one order space.
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

/// Red when alert categories are allowed on non-land theatres or out of
/// ascending scan order.
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

// ---------------------------------------------------------------------------
// Envelope and identity laws
// ---------------------------------------------------------------------------

/// Red when the executable or any producer manifest hash stops being compared
/// on decode, which would let a payload built by a different binary through.
#[test]
fn executable_and_manifest_identity_mismatches_refuse() {
    let payload = encode_payload(&exact_frame()).unwrap();
    // 4 u32 + 3 u64 header, then five 32-byte hashes.
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

/// Red when the schema version stops being compared, which would let a `THS1`
/// recorder-POV payload be read as an all-country one.
#[test]
fn schema_version_mismatch_refuses() {
    let mut payload = encode_payload(&exact_frame()).unwrap();
    payload[4] = 1;
    assert!(decode_payload(&payload).is_err());
}

/// Red when trailing or missing bytes stop being detected.
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

/// Red when the nonzero source-sequence law is dropped from frame validation.
#[test]
fn zero_source_sequence_refuses() {
    let mut frame = exact_frame();
    frame.source_sequence = 0;
    assert!(encode_payload(&frame).is_err());
}

/// Red when `SourceSequenceGate` stops requiring a strict increase, which is
/// how a replayed or reordered frame would be admitted.
#[test]
fn source_sequence_must_strictly_increase() {
    let mut gate = SourceSequenceGate::new();
    assert!(gate.admit(0).is_err());
    assert!(gate.admit(10).is_ok());
    assert!(gate.admit(10).is_err());
    assert!(gate.admit(9).is_err());
    assert!(gate.admit(11).is_ok());
}

/// Red when an exact frame is allowed to omit its content identity, which
/// would leave engine-materialized display names unauthenticated against the
/// replay-side effective VFS.
#[test]
fn exact_frame_without_content_identity_refuses() {
    let mut frame = exact_frame();
    frame.content_identity = None;
    assert!(encode_payload(&frame).is_err());
}

/// Red when the content-layer ordinal contiguity or duplicate check is
/// dropped, which would let a mod load order be transported ambiguously.
#[test]
fn content_layer_order_and_uniqueness_are_enforced() {
    let mut gapped = exact_frame();
    gapped
        .content_identity
        .as_mut()
        .unwrap()
        .ordered_layers[1]
        .ordinal = 5;
    assert!(encode_payload(&gapped).is_err());

    let mut duplicated = exact_frame();
    let layers = &mut duplicated.content_identity.as_mut().unwrap().ordered_layers;
    layers[1].layer_kind = ContentLayerKind::Base;
    layers[1].layer_id = "base".into();
    assert!(encode_payload(&duplicated).is_err());
}

/// Red when the language token stops being required, which would let a frame
/// carry display names with no way to tell which localization produced them.
#[test]
fn missing_language_refuses() {
    let mut frame = exact_frame();
    frame.content_identity.as_mut().unwrap().language = String::new();
    assert!(encode_payload(&frame).is_err());
}

/// Red when an unavailable frame is allowed to be clean or to carry a body.
#[test]
fn unavailable_frame_is_dirty_and_bodyless() {
    assert!(encode_source_unavailable(900, 901, 7, 0).is_err());
    let decoded = decode_payload(&encode_source_unavailable(900, 901, 7, 1).unwrap()).unwrap();
    assert_eq!(decoded.authority, AuthorityState::SourceUnavailable);
    assert!(decoded.snapshot.is_none());
    assert!(decoded.content_identity.is_none());
    assert_eq!(decoded.source_sequence, 901);
}

/// Red when the producer stops authenticating its roots before walking, which
/// would let a non-a729 process produce a nominally exact frame.
#[test]
fn unauthenticated_source_refuses_to_produce() {
    let mut source = fixture();
    source.exact = false;
    assert!(matches!(
        collect_and_encode(&source, 900, 901, 7),
        Err(CarrierError::Source(_))
    ));
}

// ---------------------------------------------------------------------------
// Uncaptured controller fields
// ---------------------------------------------------------------------------

/// Red when the producer starts emitting a fabricated priority ordinal --
/// including the tempting `Measured(0)` default -- for a row whose controller
/// read law is not established.
#[test]
fn priority_ordinal_is_explicitly_unavailable_never_defaulted() {
    let frame = produced();
    let snapshot = frame.snapshot.unwrap();
    let mut checked = 0;
    for section in &snapshot.sections {
        if let SectionAdmission::Admitted(body) = &section.admission {
            for theatre in &body.theatres {
                assert_eq!(theatre.priority_ordinal, PriorityOrdinal::Unavailable);
                assert_eq!(theatre.priority_ordinal.measured(), None);
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "fixture must exercise at least one theatre");
}

/// Red when `Unavailable` and `Measured(0)` stop being distinguishable on the
/// wire -- the exact collapse that would turn a hole into a false measurement.
#[test]
fn unavailable_priority_is_distinct_from_measured_zero() {
    let (unavailable_bytes, unavailable) = encode_with_priority(PriorityOrdinal::Unavailable);
    let (measured_bytes, measured) = encode_with_priority(PriorityOrdinal::Measured(0));
    assert_eq!(unavailable, PriorityOrdinal::Unavailable);
    assert_eq!(unavailable.measured(), None);
    assert_eq!(measured, PriorityOrdinal::Measured(0));
    assert_eq!(measured.measured(), Some(0));
    assert_ne!(
        unavailable_bytes, measured_bytes,
        "an unmeasured priority must not encode identically to a measured zero"
    );
}

/// Encode the fixture with one theatre's priority replaced, returning both the
/// bytes and what they decode back to.
fn encode_with_priority(value: PriorityOrdinal) -> (Vec<u8>, PriorityOrdinal) {
    let mut frame = exact_frame();
    {
        let SectionAdmission::Admitted(body) =
            &mut frame.snapshot.as_mut().unwrap().sections[0].admission
        else {
            panic!("fixture section 0 is admitted");
        };
        body.theatres[0].priority_ordinal = value;
    }
    let bytes = encode_payload(&frame).unwrap();
    let decoded = decode_payload(&bytes).unwrap();
    let SectionAdmission::Admitted(body) = &decoded.snapshot.unwrap().sections[0].admission else {
        panic!("still admitted");
    };
    (bytes, body.theatres[0].priority_ordinal)
}

/// Red when a wire payload claims `Unavailable` while smuggling a nonzero
/// ordinal in the value slot.
/// The corrupted payload is built deterministically: the byte where an
/// `Unavailable` encoding differs from a `Measured(3)` encoding *is* the tag
/// byte, so this test cannot pass by stumbling on some unrelated field that
/// happens to refuse.
#[test]
fn unavailable_priority_with_nonzero_ordinal_refuses() {
    let (unavailable, _) = encode_with_priority(PriorityOrdinal::Unavailable);
    let (mut measured, _) = encode_with_priority(PriorityOrdinal::Measured(3));
    assert_eq!(unavailable.len(), measured.len());
    let tag_at = unavailable
        .iter()
        .zip(measured.iter())
        .position(|(left, right)| left != right)
        .expect("the two encodings must differ at the priority tag");
    assert_eq!(measured[tag_at], 1, "located byte must be the priority tag");
    // Tag now claims "unavailable" while the value slot still says 3.
    measured[tag_at] = 0;
    assert!(
        decode_payload(&measured).is_err(),
        "an unavailable priority carrying a nonzero ordinal must refuse"
    );
}

/// Red when a named member loses its provenance, or a provenance appears with
/// no name -- either would let replay treat a runtime string as a
/// re-resolvable localization key.
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

/// Red when a localization-keyed name is admitted with an empty key, which
/// would make the key useless as provenance.
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

/// Red when the enumerated uncaptured-field list is emptied or silently
/// shrunk without the corresponding read law actually landing. The list is the
/// carrier's own record of what it does not claim.
#[test]
fn uncaptured_controller_fields_are_declared() {
    assert_eq!(UNCAPTURED_CONTROLLER_FIELDS.len(), 25);
    let mut sorted = UNCAPTURED_CONTROLLER_FIELDS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), UNCAPTURED_CONTROLLER_FIELDS.len());
    for field in UNCAPTURED_CONTROLLER_FIELDS {
        assert!(field.contains('.'), "{field} must name template.field");
    }
}

// ---------------------------------------------------------------------------
// Round trip
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// Section-wide identity (found by adversarial review: the identity sets were
// scoped per theatre, which admitted one object under two parents)
// ---------------------------------------------------------------------------

/// Red when member-identity uniqueness is scoped per theatre instead of per
/// section, which would admit the same fleet under two navy theatres.
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
        priority_ordinal: PriorityOrdinal::Unavailable,
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
        }],
        alert_categories: Vec::new(),
    };
    body.theatres.push(shared_fleet(800, 0));
    body.theatres.push(shared_fleet(801, 1));
    assert!(encode_payload(&frame).is_err());
}

/// Red when a wing is allowed to be both grouped and ungrouped in the same
/// section -- two mutually exclusive states of one object.
#[test]
fn a_wing_that_is_both_grouped_and_free_rejects_the_section() {
    let mut frame = exact_frame();
    let SectionAdmission::Admitted(body) =
        &mut frame.snapshot.as_mut().unwrap().sections[1].admission
    else {
        panic!("fixture section 1 is admitted");
    };
    // Wing 600 is already carried under AirGroup 500.
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

/// Red when a member or free-wing priority slot is dropped from the wire, or
/// when either stops being explicitly unavailable.
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

/// Red when the producer's declaration drifts from the list the replay
/// consumer independently requires. The two crates share no dependency -- the
/// recording file is their only interface -- so this equality is the only thing
/// keeping a producer-side edit from silently un-declaring a hole the consumer
/// still refuses to render without.
#[test]
fn producer_declaration_matches_the_replay_required_list() {
    // Kept in sync by hand with
    // `replay_core::theatre_selector::REQUIRED_UNCAPTURED_CONTROLLER_FIELDS`.
    const REPLAY_REQUIRED: &[&str] = &[
        "theater_group_settings_view.current_priority_ordinal",
        "theater_group_settings_view.delete_eligibility",
        "orders_group_item.army_training_state",
        "orders_group_item.exercise_stop_at_max_state",
        "orders_group_item.border_conflict_state",
        "navy_theater_fleet_item.live_spotting_state",
        "navy_theater_fleet_item.exercise_stop_at_max_state",
        "theater_group_item.supply_priority_ordinal",
        "theater_group_item.combat_log_ownership_and_eligibility",
        "navy_theater_group_item.current_priority_ordinal",
        "navy_theater_group_item.combat_log_ownership_and_eligibility",
        "reserve_navy_theater_group_item.combat_log_ownership_and_eligibility",
        "new_theater_group_item.current_eligibility",
        "strategic_region_alert_entry.category_to_atlas_frame_law",
        "strategic_region_alert_entry.runtime_glow_state",
        "strategic_region_item.region_and_parent_identity",
        "strategic_region_item.engine_order",
        "strategic_region_item.alert_identities_and_order",
        "strategic_region_item.naval_access_row_input",
        "fleet_row_item_box.split_off_state_and_count",
        "reserve_fleet_row_item_box.split_off_state_and_count",
        "air_theatre_wing_item.active_visual_frame",
        "air_theatre_item.current_priority_ordinal",
        "air_theatre_group_item.current_priority_ordinal",
        "air_theatre_free_wings_item.current_priority_ordinal",
    ];
    let mut declared = UNCAPTURED_CONTROLLER_FIELDS.to_vec();
    let mut required = REPLAY_REQUIRED.to_vec();
    declared.sort_unstable();
    required.sort_unstable();
    assert_eq!(declared, required);
}

/// Red when the declared uncaptured-field list stops naming every template the
/// dependency contract lists as a producer dependency.
#[test]
fn every_contract_template_family_is_declared_uncaptured() {
    for family in [
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
    ] {
        assert!(
            UNCAPTURED_CONTROLLER_FIELDS
                .iter()
                .any(|field| field.starts_with(&format!("{family}."))),
            "{family} has no declared uncaptured field"
        );
    }
}

/// Red when any carried field is dropped or reordered by the codec.
#[test]
fn all_country_frame_round_trips_exactly() {
    let frame = exact_frame();
    assert_eq!(reencode(&frame).unwrap(), frame);
}
