use std::collections::{BTreeMap, BTreeSet};

use map_viewer::{
    admit_map_authority_json, admit_map_authority_projection,
    map_authority_observed_status_label, map_authority_owner_proven_admission,
    map_authority_projection_from_world_frame,
    map_authority_requirement_metadata, refuse_missing_xmap00_authority,
    refuse_product_map_until_xmap01, require_promoted_xmap01_renderer, MapAuthorityProjection,
    MapAuthorityRequirement, MapAuthorityRequirementDisposition, MAP_AUTHORITY_INPUT_SCHEMA,
    MAP_AUTHORITY_OWNER_PROVEN_INPUTS, MAP_AUTHORITY_REQUIRED_INPUTS,
};
use replay_core::{
    ContentManifest, RawFrame, ReplaySource, SourceError, SourceGameIdentity,
    SourceProfileIdentity, ViewStatus, WorldFrame, WorldTimeline,
    PORTABLE_CONTENT_MANIFEST_SCHEMA,
};
use serde_json::{json, Map, Value};

/// The authoritative XMAP00 semantic ledger. The admission table in
/// `map_authority.rs` is a transcription of this file's `xmap00_disposition`
/// column and must not be able to drift from it.
const XMAP00_LEDGER_TSV: &str = include_str!(
    "../../../re_evidence/rendering_laws/XMAP00/non_ui_visual_input_ledger_20260730.tsv"
);

fn xmap00_ledger_rows() -> Vec<BTreeMap<String, String>> {
    let mut lines = XMAP00_LEDGER_TSV.lines();
    let header: Vec<&str> = lines.next().expect("ledger header").split('\t').collect();
    lines
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), header.len(), "ragged ledger row: {line}");
            header
                .iter()
                .zip(fields)
                .map(|(key, value)| ((*key).to_string(), value.to_string()))
                .collect()
        })
        .collect()
}

fn unresolved_requirement(input_id: &str) -> MapAuthorityRequirement {
    let metadata = map_authority_requirement_metadata(input_id).unwrap();
    assert_eq!(metadata.input_id, input_id);
    assert_eq!(metadata.payload_schema, None);
    assert_eq!(metadata.cadence, None);
    assert_eq!(metadata.identity_order, None);
    // An owner-proven family's refusal always names the status the frame
    // reported, because its availability is a fact about the recording rather
    // than a pending proof; an unproven family may be refused with no status
    // at all (the projection has no accessor to consult for it yet).
    let blocker = if map_authority_owner_proven_admission(input_id).is_some() {
        format!(
            "{}; WorldFrame {} is DomainUnavailable",
            metadata.blocker,
            map_authority_observed_status_label(input_id)
        )
    } else {
        metadata.blocker.to_string()
    };
    MapAuthorityRequirement {
        input_id: input_id.to_string(),
        payload_schema: None,
        cadence: None,
        identity_order: None,
        disposition: MapAuthorityRequirementDisposition::Unavailable,
        blocker: Some(blocker),
    }
}

// --------------------------------------------------------------------------
// A WorldFrame built from an in-memory snapshot, so the derived seam can be
// driven at every status a real recording can produce.
// --------------------------------------------------------------------------

struct InMemorySource {
    hours: Vec<i64>,
    rest: Map<String, Value>,
}

impl ReplaySource for InMemorySource {
    fn game_id(&self) -> &str {
        "xmap00-authority-seam"
    }
    fn hours(&self) -> &[i64] {
        &self.hours
    }
    fn warnings(&self) -> &[String] {
        &[]
    }
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        Ok(RawFrame {
            hour_index,
            date: None,
            save_version: None,
            schema_version: None,
            rest: self.rest.clone(),
        })
    }
}

fn world_frame_with(rest: Value) -> std::sync::Arc<WorldFrame> {
    let rest = rest.as_object().expect("snapshot object").clone();
    let timeline = WorldTimeline::open(Box::new(InMemorySource {
        hours: vec![7],
        rest,
    }))
    .expect("timeline");
    timeline.frame(7).expect("frame")
}

fn fixture_manifest() -> ContentManifest {
    let mut manifest = ContentManifest {
        schema: PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
        product_modset_sha256: "11".repeat(32),
        source_game: SourceGameIdentity {
            version: "1.19.2".into(),
            build: "a729".into(),
            executable_sha256:
                "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad".into(),
            build_profile: "xmap00-authority-seam".into(),
        },
        source_profile: SourceProfileIdentity {
            profile_id: "xmap00-authority-seam".into(),
            playset_id: "vanilla".into(),
            selection_sha256: "00".repeat(32),
        },
        effective_vfs_manifest_sha256: "22".repeat(32),
        ordered_layers: Vec::new(),
        dlc_selection: Vec::new(),
        localisation_language: "l_english".into(),
        required_settings: BTreeMap::new(),
        required_content: Vec::new(),
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    manifest.with_computed_digest().unwrap()
}

fn requirement<'a>(
    projection: &'a MapAuthorityProjection,
    input_id: &str,
) -> &'a MapAuthorityRequirement {
    projection
        .requirements
        .iter()
        .find(|row| row.input_id == input_id)
        .unwrap_or_else(|| panic!("projection omitted {input_id}"))
}

// --------------------------------------------------------------------------
// Owner-proven fixtures.
//
// Every proven row gets one snapshot that measures it and one that does not.
// For the rows whose ledger `status_accessor` is a COMPOSITE weaker than the
// admitted payload, the unmeasured snapshot deliberately leaves that composite
// `Present` — that is the frame in which reverting the row to its ledger
// accessor would wrongly admit, and `ledger_accessor_stays_present` marks it so
// the test can assert the fixture really is that frame instead of assuming it.
// --------------------------------------------------------------------------

/// Two countries with a complete, unique, nonzero D18 identity each.
fn measured_countries() -> Value {
    json!({
        "ENG": {"_r2_tag_db_id": 2},
        "GER": {"_r2_tag_db_id": 1},
    })
}

/// A frame in which every owner-proven input is measured.
fn fully_measured_snapshot() -> Value {
    json!({
        "combats": [{"combat_id": 11, "generation": 1, "kind": "land", "location_province": 42}],
        "ended_combats": [{"combat_id": 9, "generation": 2, "lifecycle": "ended"}],
        "naval_combats": [],
        "countries": measured_countries(),
        "diplomacy_world": {
            "domain53_extra": {
                "faction_roster_status": "measured_empty",
                "faction_rows": [],
            },
        },
    })
}

/// One proven row's unmeasured fixture.
struct UnmeasuredFixture {
    snapshot: Value,
    /// Whether this fixture leaves the row's ledger `status_accessor` reporting
    /// `Present`. True only for rows whose ledger accessor is a composite the
    /// certified predicate strengthens; for a row whose ledger accessor already
    /// IS its payload's fail-closed verdict there is no such frame, and
    /// claiming one would be a fiction.
    ledger_accessor_stays_present: bool,
}

/// The snapshot in which `input_id`'s own payload is not measured.
fn unmeasured_fixture(input_id: &str) -> UnmeasuredFixture {
    let mut snapshot = fully_measured_snapshot();
    let object = snapshot.as_object_mut().unwrap();
    let ledger_accessor_stays_present = match input_id {
        // `combats_status` IS this row's payload verdict: `build_combats`
        // downgrades to `DomainUnavailable` on a malformed row, so there is no
        // frame where the accessor is Present and the census is not measured.
        "combat.land_active_epoch" => {
            object.insert("combats".into(), Value::Null);
            false
        }
        // `ended_combats_status` stays `Present` after the decoder drops a
        // malformed terminal row; only the certified reading sees the loss.
        "combat.land_terminal_detail" => {
            object.insert(
                "ended_combats".into(),
                json!([{"combat_id": 0, "generation": 2, "lifecycle": "ended"}]),
            );
            true
        }
        // The merged naval/air root is `Present` because the AIR partition is
        // (`build_naval_air_combat` combines the naval, sunk-convoy and air
        // statuses, and any `Present` dominates); the naval census itself was
        // explicitly unobtained.
        "combat.naval_air_epoch" => {
            object.insert("naval_combats".into(), Value::Null);
            object.insert(
                "naval_air".into(),
                json!({"_r2_air_combat_losses_live": []}),
            );
            true
        }
        // `d18_country_roster.status` IS this row's payload verdict. ENG has no
        // identity, so the roster is unavailable — and so is the accessor.
        "country.d18_complete_roster" => {
            object.insert("countries".into(), json!({"ENG": {}, "GER": {"_r2_tag_db_id": 1}}));
            false
        }
        // `faction_roster_status` IS this row's payload verdict, over a single
        // atomic leaf. Removing the witness unavailables both.
        "diplomacy.faction_roster" => {
            object.remove("diplomacy_world");
            false
        }
        other => panic!("no unmeasured fixture for {other}"),
    };
    UnmeasuredFixture {
        snapshot,
        ledger_accessor_stays_present,
    }
}

/// The accessor the XMAP00 ledger's `status_accessor` column names for each
/// proven row, read off the frame directly. This is deliberately the LEDGER's
/// reading and not the admission table's predicate: the two agreeing or
/// disagreeing is the thing under test.
fn ledger_accessor_status(frame: &WorldFrame, input_id: &str) -> ViewStatus {
    match input_id {
        "combat.land_active_epoch" => frame.combats_status(),
        "combat.land_terminal_detail" => frame.ended_combats_status(),
        "combat.naval_air_epoch" => frame.naval_air_combat_status(),
        "country.d18_complete_roster" => frame.d18_country_roster().status,
        "diplomacy.faction_roster" => match frame.diplomacy().faction_roster_status() {
            replay_core::world::FactionRosterStatus::Unavailable => ViewStatus::DomainUnavailable,
            replay_core::world::FactionRosterStatus::MeasuredEmpty
            | replay_core::world::FactionRosterStatus::MeasuredNonEmpty => ViewStatus::Present,
        },
        other => panic!("no ledger accessor for {other}"),
    }
}

fn unresolved_projection() -> MapAuthorityProjection {
    // Nothing a caller assembles carries a frame-derivation witness, which is
    // exactly the point: this is the untrusted shape.
    MapAuthorityProjection::from_untrusted_parts(
        MAP_AUTHORITY_INPUT_SCHEMA.to_string(),
        42,
        "1".repeat(64),
        "2".repeat(64),
        "3".repeat(64),
        MAP_AUTHORITY_REQUIRED_INPUTS
            .iter()
            .map(|input_id| unresolved_requirement(input_id))
            .collect(),
    )
}

#[test]
fn every_stable_family_has_only_explicit_unresolved_metadata_and_blocker() {
    for input_id in MAP_AUTHORITY_REQUIRED_INPUTS {
        let metadata = map_authority_requirement_metadata(input_id).unwrap();
        assert_eq!(metadata.input_id, *input_id);
        assert_eq!(metadata.payload_schema, None, "{input_id}");
        assert_eq!(metadata.cadence, None, "{input_id}");
        assert_eq!(metadata.identity_order, None, "{input_id}");
        assert!(!metadata.blocker.is_empty(), "{input_id}");
    }
    assert!(map_authority_requirement_metadata("recorder.selected_present").is_none());
}

#[test]
fn xmap00_can_only_return_a_complete_typed_gap_report() {
    let admission = admit_map_authority_projection(&unresolved_projection()).unwrap();
    assert_eq!(admission.gaps().len(), MAP_AUTHORITY_REQUIRED_INPUTS.len());
    assert_eq!(
        admission
            .gaps()
            .iter()
            .map(|gap| gap.input_id.as_str())
            .collect::<Vec<_>>(),
        MAP_AUTHORITY_REQUIRED_INPUTS
    );

    let refusal = refuse_product_map_until_xmap01(&admission);
    assert_eq!(refusal.code, "XMAP00_AUTHORITY_UNAVAILABLE");
    assert!(!refusal.authority_ready);
    assert_eq!(refusal.gap_ids.len(), MAP_AUTHORITY_REQUIRED_INPUTS.len());
    assert!(require_promoted_xmap01_renderer(&admission).is_err());

    let missing_frame = refuse_missing_xmap00_authority("world_frame");
    assert_eq!(missing_frame.code, "XMAP00_AUTHORITY_UNAVAILABLE");
    assert!(!missing_frame.authority_ready);
    assert_eq!(missing_frame.gap_ids, vec!["world_frame"]);
}

#[test]
fn caller_hashes_legacy_ready_rows_and_measured_empty_witnesses_cannot_admit() {
    let projection = unresolved_projection();
    for forbidden_field in [
        "payload_sha256",
        "ordering_sha256",
        "completeness_sha256",
        "provenance_receipt_sha256",
        "measured_empty_witness",
    ] {
        let mut json = serde_json::to_value(&projection).unwrap();
        json["requirements"][0][forbidden_field] = serde_json::json!("1".repeat(64));
        assert!(
            admit_map_authority_json(&serde_json::to_vec(&json).unwrap()).is_err(),
            "{forbidden_field} was silently trusted"
        );
    }

    for forged_disposition in ["present", "measured_empty"] {
        let mut json = serde_json::to_value(&projection).unwrap();
        json["requirements"][0]["disposition"] = serde_json::json!(forged_disposition);
        assert!(
            admit_map_authority_json(&serde_json::to_vec(&json).unwrap()).is_err(),
            "{forged_disposition} was admitted"
        );
    }

    let authority_source = include_str!("../src/map_authority.rs");
    assert!(!authority_source.contains("MapAuthorityReceipt"));
    assert!(!authority_source.contains("MapAuthorityMeasuredEmptyWitness"));
    assert!(!authority_source.contains("Ready {"));
    assert!(!authority_source.contains("fn is_ready"));
}

#[test]
fn recorder_presentation_fields_are_rejected_not_ignored() {
    for forbidden in [
        "selected_present",
        "camera",
        "map_mode",
        "selection",
        "gui_state",
        "draw_geometry",
    ] {
        let mut json = serde_json::to_value(unresolved_projection()).unwrap();
        json.as_object_mut()
            .unwrap()
            .insert(forbidden.into(), serde_json::json!({}));
        assert!(
            admit_map_authority_json(&serde_json::to_vec(&json).unwrap()).is_err(),
            "{forbidden} was silently accepted"
        );
    }
}

#[test]
fn family_identity_order_and_unresolved_metadata_are_fail_closed() {
    let mut missing = unresolved_projection();
    missing.requirements.pop();
    assert!(admit_map_authority_projection(&missing).is_err());

    let mut duplicate = unresolved_projection();
    duplicate.requirements[1].input_id = duplicate.requirements[0].input_id.clone();
    assert!(admit_map_authority_projection(&duplicate).is_err());

    let mut unsorted = unresolved_projection();
    unsorted.requirements.swap(0, 1);
    assert!(admit_map_authority_projection(&unsorted).is_err());

    for field in ["payload_schema", "cadence", "identity_order"] {
        let mut json = serde_json::to_value(unresolved_projection()).unwrap();
        json["requirements"][0][field] = serde_json::json!("caller-invented");
        let error = admit_map_authority_json(&serde_json::to_vec(&json).unwrap()).unwrap_err();
        assert!(
            error.contains("unresolved map authority metadata"),
            "{error}"
        );
    }

    let mut changed_blocker = unresolved_projection();
    changed_blocker.requirements[0].blocker = Some("caller says complete".into());
    assert!(admit_map_authority_projection(&changed_blocker)
        .unwrap_err()
        .contains("canonical unresolved blocker"));
}

#[test]
fn absent_and_unavailable_require_distinct_canonical_blocker_forms() {
    let mut forged_absent = unresolved_projection();
    forged_absent.requirements[0].disposition = MapAuthorityRequirementDisposition::Absent;
    assert!(
        admit_map_authority_json(&serde_json::to_vec(&forged_absent).unwrap())
            .unwrap_err()
            .contains("canonical unresolved blocker")
    );

    let metadata =
        map_authority_requirement_metadata(&forged_absent.requirements[0].input_id).unwrap();
    forged_absent.requirements[0].blocker = Some(format!(
        "{}; WorldFrame {} is DomainAbsent",
        metadata.blocker,
        map_authority_observed_status_label(&forged_absent.requirements[0].input_id)
    ));
    let admitted = admit_map_authority_json(&serde_json::to_vec(&forged_absent).unwrap()).unwrap();
    assert_eq!(
        admitted.gaps()[0].disposition,
        MapAuthorityRequirementDisposition::Absent
    );
    assert_eq!(
        Some(&admitted.gaps()[0].reason),
        forged_absent.requirements[0].blocker.as_ref()
    );

    let mut forged_unavailable = unresolved_projection();
    let metadata =
        map_authority_requirement_metadata(&forged_unavailable.requirements[0].input_id).unwrap();
    forged_unavailable.requirements[0].blocker = Some(format!(
        "{}; WorldFrame {} is DomainAbsent",
        metadata.blocker,
        map_authority_observed_status_label(&forged_unavailable.requirements[0].input_id)
    ));
    assert!(
        admit_map_authority_json(&serde_json::to_vec(&forged_unavailable).unwrap())
            .unwrap_err()
            .contains("canonical unresolved blocker")
    );
}

// --------------------------------------------------------------------------
// The derived seam: the ledger's disposition column decides which inputs may
// admit; the authenticated frame decides whether they do.
// --------------------------------------------------------------------------

/// kills: adding, dropping or renaming an entry in
/// `MAP_AUTHORITY_OWNER_PROVEN_INPUTS` (or changing one of its transcribed
/// ledger columns) without the authoritative XMAP00 TSV's own
/// `xmap00_disposition`/`owner_packet`/`portable_path`/`status_accessor`
/// columns agreeing.
#[test]
fn the_owner_proven_admission_table_is_exactly_the_ledgers_disposition_column() {
    let rows = xmap00_ledger_rows();
    let semantic: Vec<&BTreeMap<String, String>> = rows
        .iter()
        .filter(|row| row["class"] == "semantic")
        .collect();
    assert_eq!(semantic.len(), 45, "XMAP00 semantic census drifted");

    let mut ledger_ids: Vec<&str> = semantic.iter().map(|row| row["id"].as_str()).collect();
    ledger_ids.sort_unstable();
    assert_eq!(
        ledger_ids, MAP_AUTHORITY_REQUIRED_INPUTS,
        "required-input list drifted from the ledger's semantic rows"
    );

    let ledger_proven: BTreeSet<&str> = semantic
        .iter()
        .filter(|row| row["xmap00_disposition"] == "owner_proven_production_consumer")
        .map(|row| row["id"].as_str())
        .collect();
    let code_proven: BTreeSet<&str> = MAP_AUTHORITY_OWNER_PROVEN_INPUTS
        .iter()
        .map(|admission| admission.input_id)
        .collect();
    assert_eq!(
        ledger_proven, code_proven,
        "admission table is not the ledger's owner-proven set"
    );
    // XMAP14's three combat rows (lane GX) plus the two lane HW proved from
    // committed capture evidence. Every other semantic row is still refused;
    // the reason each of lane HW's remaining ten candidates could NOT be
    // promoted is recorded per row in
    // `extractor/native/runs/hw_admission_a/ROW_PROOFS.md`.
    assert_eq!(code_proven.len(), 5);

    for row in &semantic {
        let id = row["id"].as_str();
        let disposition = row["xmap00_disposition"].as_str();
        match map_authority_owner_proven_admission(id) {
            Some(admission) => {
                assert_eq!(disposition, "owner_proven_production_consumer", "{id}");
                assert_eq!(admission.owner_packet, row["owner_packet"], "{id}");
                assert_eq!(admission.payload_schema, row["portable_path"], "{id}");
                assert_eq!(admission.status_accessor, row["status_accessor"], "{id}");
                assert!(!admission.cadence.is_empty(), "{id}");
                assert!(!admission.identity_order.is_empty(), "{id}");
            }
            None => {
                assert_eq!(disposition, "unavailable_until_owner_proves", "{id}");
                // An unproven family keeps its named blocker and nothing else.
                let metadata = map_authority_requirement_metadata(id).unwrap();
                assert!(!metadata.blocker.is_empty(), "{id}");
                assert_eq!(metadata.payload_schema, None, "{id}");
            }
        }
    }
}

/// kills: reverting `map_authority_projection_from_world_frame` to build every
/// row through `unavailable_requirement`, or admitting a proven input from
/// anything other than its own view's `Present` status.
#[test]
fn owner_proven_inputs_admit_from_the_frames_own_measured_views() {
    let manifest = fixture_manifest();
    let frame = world_frame_with(fully_measured_snapshot());
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();

    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        let row = requirement(&projection, proven.input_id);
        assert_eq!(
            row.disposition,
            MapAuthorityRequirementDisposition::Admitted,
            "{} was not admitted from measured data",
            proven.input_id
        );
        assert_eq!(row.payload_schema.as_deref(), Some(proven.payload_schema));
        assert_eq!(row.cadence.as_deref(), Some(proven.cadence));
        assert_eq!(row.identity_order.as_deref(), Some(proven.identity_order));
        assert_eq!(row.blocker, None);
    }

    let proven_count = MAP_AUTHORITY_OWNER_PROVEN_INPUTS.len();
    let admission = admit_map_authority_projection(&projection).unwrap();
    assert_eq!(admission.admitted().len(), proven_count);
    assert_eq!(
        admission.gaps().len(),
        MAP_AUTHORITY_REQUIRED_INPUTS.len() - proven_count
    );
    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        assert!(admission.is_admitted(proven.input_id));
        assert!(
            !admission
                .gaps()
                .iter()
                .any(|gap| gap.input_id == proven.input_id),
            "{} is both admitted and a gap",
            proven.input_id
        );
        let admitted = admission
            .admitted()
            .iter()
            .find(|input| input.input_id == proven.input_id)
            .unwrap();
        assert_eq!(admitted.owner_packet, proven.owner_packet);
        assert_eq!(admitted.payload_schema, proven.payload_schema);
        assert_eq!(admitted.cadence, proven.cadence);
        assert_eq!(admitted.identity_order, proven.identity_order);
    }

    // Nothing else moved: every unproven family keeps its named blocker.
    for gap in admission.gaps() {
        assert!(map_authority_owner_proven_admission(&gap.input_id).is_none());
        assert!(!gap.reason.is_empty(), "{} lost its blocker", gap.input_id);
    }
}

/// kills: admitting a proven input whose view the recording did not measure —
/// i.e. treating an absent or explicitly unobtained collection as data.
#[test]
fn owner_proven_inputs_refuse_with_their_named_blocker_when_unmeasured() {
    let manifest = fixture_manifest();

    let absent = map_authority_projection_from_world_frame(&world_frame_with(json!({})), &manifest)
        .unwrap();
    let unobtained = map_authority_projection_from_world_frame(
        &world_frame_with(json!({
            "combats": Value::Null,
            "ended_combats": Value::Null,
            "naval_combats": Value::Null,
            "countries": Value::Null,
            "diplomacy_live": Value::Null,
        })),
        &manifest,
    )
    .unwrap();

    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        let blocker = map_authority_requirement_metadata(proven.input_id)
            .unwrap()
            .blocker;
        let label = map_authority_observed_status_label(proven.input_id);
        // `diplomacy.faction_roster`'s carrier is a single `Leaf`, and
        // `faction_roster_status` maps BOTH of its refusal states to
        // `DomainUnavailable` on purpose: "no faction carrier at all" is not
        // positive knowledge of an empty roster, so it fails closed rather than
        // reporting the softer `DomainAbsent`. Every other proven row can tell
        // a missing key from an unobtained one and reports `DomainAbsent`.
        let empty_frame_disposition = if proven.input_id == "diplomacy.faction_roster" {
            MapAuthorityRequirementDisposition::Unavailable
        } else {
            MapAuthorityRequirementDisposition::Absent
        };
        let empty_frame_status = if proven.input_id == "diplomacy.faction_roster" {
            "DomainUnavailable"
        } else {
            "DomainAbsent"
        };
        let absent_row = requirement(&absent, proven.input_id);
        assert_eq!(
            absent_row.disposition, empty_frame_disposition,
            "{}",
            proven.input_id
        );
        assert_eq!(
            absent_row.blocker.as_deref(),
            Some(format!("{blocker}; WorldFrame {label} is {empty_frame_status}").as_str()),
            "{}",
            proven.input_id
        );
        assert_eq!(absent_row.payload_schema, None);

        let unobtained_row = requirement(&unobtained, proven.input_id);
        assert_eq!(
            unobtained_row.disposition,
            MapAuthorityRequirementDisposition::Unavailable,
            "{}",
            proven.input_id
        );
        assert_eq!(
            unobtained_row.blocker.as_deref(),
            Some(format!("{blocker}; WorldFrame {label} is DomainUnavailable").as_str()),
            "{}",
            proven.input_id
        );
        assert_eq!(unobtained_row.payload_schema, None);
    }

    for projection in [&absent, &unobtained] {
        let admission = admit_map_authority_projection(projection).unwrap();
        assert!(admission.admitted().is_empty());
        assert_eq!(admission.gaps().len(), MAP_AUTHORITY_REQUIRED_INPUTS.len());
        assert_eq!(
            refuse_product_map_until_xmap01(&admission).gap_ids.len(),
            MAP_AUTHORITY_REQUIRED_INPUTS.len()
        );
    }
}

/// kills: letting a caller mark an unproven family admitted, or letting an
/// admitted row carry a contract other than the one the owner proved.
#[test]
fn an_input_without_owner_proven_evidence_can_never_be_admitted() {
    let unproven = "control.controller_by_province";
    assert!(map_authority_owner_proven_admission(unproven).is_none());
    let mut forged = unresolved_projection();
    let index = forged
        .requirements
        .iter()
        .position(|row| row.input_id == unproven)
        .unwrap();
    forged.requirements[index].disposition = MapAuthorityRequirementDisposition::Admitted;
    forged.requirements[index].payload_schema = Some("WorldFrame.map.control_by_province".into());
    forged.requirements[index].cadence = Some("one decoded view per canonical recorded hour".into());
    forged.requirements[index].identity_order = Some("province id ascending".into());
    forged.requirements[index].blocker = None;
    assert!(admit_map_authority_projection(&forged)
        .unwrap_err()
        .contains("has no owner-proven production admission"));

    // A proven row still cannot invent its own contract.
    for (field, value) in [
        ("payload_schema", "WorldFrame.selected_present"),
        ("cadence", "whenever the viewer asks"),
        ("identity_order", "caller order"),
    ] {
        let mut drifted = unresolved_projection();
        drifted.requirements[0].disposition = MapAuthorityRequirementDisposition::Admitted;
        let proven = map_authority_owner_proven_admission(&drifted.requirements[0].input_id)
            .expect("requirements[0] is an owner-proven row");
        drifted.requirements[0].payload_schema = Some(proven.payload_schema.into());
        drifted.requirements[0].cadence = Some(proven.cadence.into());
        drifted.requirements[0].identity_order = Some(proven.identity_order.into());
        drifted.requirements[0].blocker = None;
        match field {
            "payload_schema" => drifted.requirements[0].payload_schema = Some(value.into()),
            "cadence" => drifted.requirements[0].cadence = Some(value.into()),
            _ => drifted.requirements[0].identity_order = Some(value.into()),
        }
        assert!(
            admit_map_authority_projection(&drifted)
                .unwrap_err()
                .contains("owner-proven payload/cadence/identity-order contract"),
            "{field} drift was accepted"
        );
    }
}

/// kills: dropping the frame-derivation witness (or checking it only at the
/// JSON door). Without it a caller can copy an admitted projection's rows into
/// a hand-assembled projection, or edit a derived one, and have the in-process
/// door accept the admission.
#[test]
fn only_a_frame_derived_projection_can_carry_an_admission() {
    let manifest = fixture_manifest();
    let frame = world_frame_with(json!({
        "combats": [],
        "ended_combats": [],
        "naval_combats": [],
    }));
    let derived = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    assert!(derived.is_frame_derived());
    assert_eq!(
        admit_map_authority_projection(&derived)
            .unwrap()
            .admitted()
            .len(),
        3
    );

    // Byte-identical rows, assembled by a caller rather than derived.
    let copied = MapAuthorityProjection::from_untrusted_parts(
        derived.schema.clone(),
        derived.hour_index,
        derived.content_manifest_digest_sha256.clone(),
        derived.product_modset_sha256.clone(),
        derived.effective_vfs_manifest_sha256.clone(),
        derived.requirements.clone(),
    );
    assert!(!copied.is_frame_derived());
    assert!(admit_map_authority_projection(&copied)
        .unwrap_err()
        .contains("not derived from an authenticated WorldFrame"));

    // Editing a derived projection breaks the witness too — including edits to
    // rows that are not themselves admitted.
    let mut edited_gap = derived.clone();
    let gap_index = edited_gap
        .requirements
        .iter()
        .position(|row| row.disposition != MapAuthorityRequirementDisposition::Admitted)
        .unwrap();
    edited_gap.requirements[gap_index].blocker = Some("caller says complete".into());
    assert!(!edited_gap.is_frame_derived());
    assert!(admit_map_authority_projection(&edited_gap).is_err());

    let mut edited_hour = derived.clone();
    edited_hour.hour_index += 1;
    assert!(!edited_hour.is_frame_derived());
    assert!(admit_map_authority_projection(&edited_hour)
        .unwrap_err()
        .contains("not derived from an authenticated WorldFrame"));

    // A projection with no admissions at all needs no witness: the untrusted
    // gap report stays a supported shape.
    assert!(admit_map_authority_projection(&unresolved_projection()).is_ok());
}

/// kills: admitting `combat.naval_air_epoch` on the merged root status alone.
/// That status lets any measured sibling dominate, so an unread naval census
/// would be presented as a measured-empty one.
#[test]
fn the_naval_air_epoch_needs_its_own_partition_measured_not_a_measured_sibling() {
    let manifest = fixture_manifest();
    let sibling_only = world_frame_with(json!({
        "naval_combats": Value::Null,
        "naval_air": {"_r2_air_combat_losses_live": []},
    }));
    // Precondition: the merged root status this input's ledger accessor names
    // really does read Present here, while the naval census does not.
    assert_eq!(
        sibling_only.naval_air_combat_status(),
        replay_core::ViewStatus::Present
    );
    assert_eq!(
        sibling_only.naval_air_combat().naval_combats_status,
        replay_core::ViewStatus::DomainUnavailable
    );

    let projection = map_authority_projection_from_world_frame(&sibling_only, &manifest).unwrap();
    let row = requirement(&projection, "combat.naval_air_epoch");
    assert_eq!(
        row.disposition,
        MapAuthorityRequirementDisposition::Unavailable,
        "an unread naval census was admitted behind a measured sibling"
    );
    let blocker = map_authority_requirement_metadata("combat.naval_air_epoch")
        .unwrap()
        .blocker;
    assert_eq!(
        row.blocker.as_deref(),
        Some(format!("{blocker}; WorldFrame naval_combats_status is DomainUnavailable").as_str()),
        "the refusal must name the census it actually read, not the root it did not"
    );
    let admission = admit_map_authority_projection(&projection).unwrap();
    assert!(!admission.is_admitted("combat.naval_air_epoch"));

    // The naval census on its own is what admits it.
    let measured = world_frame_with(json!({"naval_combats": []}));
    let projection = map_authority_projection_from_world_frame(&measured, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "combat.naval_air_epoch").disposition,
        MapAuthorityRequirementDisposition::Admitted
    );
}

/// kills: admitting `combat.land_terminal_detail` straight off
/// `ended_combats_status`. That status stays `Present` after the decoder skips
/// malformed terminal rows, so the raw reading turns a census that lost rows
/// into a measured one.
#[test]
fn a_terminal_census_that_dropped_rows_is_not_a_measured_one() {
    let manifest = fixture_manifest();
    let lossy = world_frame_with(json!({"ended_combats": [7]}));
    // Precondition: the raw status really does still read Present, and the
    // decoder really does record the drop in this exact vocabulary.
    assert_eq!(
        lossy.ended_combats_status(),
        replay_core::ViewStatus::Present
    );
    assert!(lossy.ended_combats().is_empty());
    assert!(
        lossy.coverage_warnings().iter().any(|warning| {
            warning.starts_with("ended_combats[") && warning.contains("row skipped")
        }),
        "decoder drop vocabulary moved; the completeness check no longer sees it: {:?}",
        lossy.coverage_warnings()
    );

    let projection = map_authority_projection_from_world_frame(&lossy, &manifest).unwrap();
    let row = requirement(&projection, "combat.land_terminal_detail");
    assert_eq!(
        row.disposition,
        MapAuthorityRequirementDisposition::Unavailable
    );
    let blocker = map_authority_requirement_metadata("combat.land_terminal_detail")
        .unwrap()
        .blocker;
    assert_eq!(
        row.blocker.as_deref(),
        Some(
            format!("{blocker}; WorldFrame certified ended_combats_status is DomainUnavailable")
                .as_str()
        )
    );
    assert!(!admit_map_authority_projection(&projection)
        .unwrap()
        .is_admitted("combat.land_terminal_detail"));

    // A clean terminal census still admits, so the check is a completeness
    // gate and not a blanket refusal.
    let clean = world_frame_with(json!({
        "ended_combats": [{"combat_id": 9, "generation": 2, "lifecycle": "ended"}],
    }));
    assert!(!clean.coverage_warnings().iter().any(|warning| {
        warning.starts_with("ended_combats[") && warning.contains("row skipped")
    }));
    let projection = map_authority_projection_from_world_frame(&clean, &manifest).unwrap();
    assert_eq!(
        requirement(&projection, "combat.land_terminal_detail").disposition,
        MapAuthorityRequirementDisposition::Admitted
    );
}

/// kills: pointing any proven row's `status` predicate at a view that reports a
/// measurement its own payload did not get. The property is one-directional and
/// holds for every proven row regardless of how strong its ledger accessor is:
/// the certified predicate may be STRICTER than the ledger's
/// `status_accessor`, never more permissive. A row wired to a neighbouring view
/// — the defect the old combinatorial test tried and failed to express — breaks
/// it on the frames below, because each unmeasures exactly one carrier.
#[test]
fn no_proven_predicate_is_more_permissive_than_its_ledger_accessor() {
    let manifest = fixture_manifest();
    let mut frames = vec![
        world_frame_with(fully_measured_snapshot()),
        world_frame_with(json!({})),
    ];
    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        frames.push(world_frame_with(unmeasured_fixture(proven.input_id).snapshot));
    }
    for frame in &frames {
        let projection = map_authority_projection_from_world_frame(frame, &manifest).unwrap();
        for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
            if requirement(&projection, proven.input_id).disposition
                == MapAuthorityRequirementDisposition::Admitted
            {
                assert_eq!(
                    ledger_accessor_status(frame, proven.input_id),
                    ViewStatus::Present,
                    "{} admitted while its ledger accessor {} refused",
                    proven.input_id,
                    proven.status_accessor
                );
            }
        }
    }
}

/// kills: weakening any proven row's predicate so that its own unmeasured
/// payload still admits. For the two rows whose ledger accessor is a composite
/// (`ended_combats_status`, `naval_air_combat_status`), the fixture ALSO leaves
/// that composite `Present` — asserted, not assumed — so reverting the row's
/// `status` field to its ledger accessor makes exactly that row admit.
#[test]
fn a_proven_input_refuses_when_its_own_payload_is_unmeasured() {
    let manifest = fixture_manifest();
    let mut composites_covered = 0;
    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        let fixture = unmeasured_fixture(proven.input_id);
        let frame = world_frame_with(fixture.snapshot);
        if fixture.ledger_accessor_stays_present {
            composites_covered += 1;
            assert_eq!(
                ledger_accessor_status(&frame, proven.input_id),
                ViewStatus::Present,
                "{}'s fixture no longer leaves its ledger accessor Present, so it proves nothing",
                proven.input_id
            );
        }
        let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
        let row = requirement(&projection, proven.input_id);
        assert_eq!(
            row.disposition,
            MapAuthorityRequirementDisposition::Unavailable,
            "{} admitted on an unmeasured payload",
            proven.input_id
        );
        assert_eq!(row.payload_schema, None, "{}", proven.input_id);
        let blocker = map_authority_requirement_metadata(proven.input_id)
            .unwrap()
            .blocker;
        let label = map_authority_observed_status_label(proven.input_id);
        assert_eq!(
            row.blocker.as_deref(),
            Some(format!("{blocker}; WorldFrame {label} is DomainUnavailable").as_str()),
            "{}",
            proven.input_id
        );
        assert!(!admit_map_authority_projection(&projection)
            .unwrap()
            .is_admitted(proven.input_id));
    }
    assert_eq!(
        composites_covered, 2,
        "the composite-accessor cases are the ones this test exists for"
    );
}

/// kills: unmeasuring one proven input silently moving another. Each fixture
/// mutates one carrier; every proven row that does not read that carrier must
/// still admit.
#[test]
fn unmeasuring_one_proven_input_leaves_its_siblings_admitted() {
    let manifest = fixture_manifest();
    for proven in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
        let projection = map_authority_projection_from_world_frame(
            &world_frame_with(unmeasured_fixture(proven.input_id).snapshot),
            &manifest,
        )
        .unwrap();
        for other in MAP_AUTHORITY_OWNER_PROVEN_INPUTS {
            if other.input_id == proven.input_id {
                continue;
            }
            assert_eq!(
                requirement(&projection, other.input_id).disposition,
                MapAuthorityRequirementDisposition::Admitted,
                "unmeasuring {} moved {}",
                proven.input_id,
                other.input_id
            );
        }
    }
}

/// kills: accepting a refusal that claims an observation the row's predicate
/// cannot make. `faction_roster_status` never reports `DomainAbsent` —
/// `build_diplomacy` forces a missing carrier to `Leaf::Unobtained` — and a
/// gap-only projection carries no frame witness, so without the
/// `can_report_absent` check a caller could hand back a fabricated absence and
/// have it validated.
#[test]
fn a_proven_row_cannot_be_refused_with_an_observation_it_cannot_make() {
    let mut projection = unresolved_projection();
    let index = projection
        .requirements
        .iter()
        .position(|row| row.input_id == "diplomacy.faction_roster")
        .unwrap();
    let blocker = map_authority_requirement_metadata("diplomacy.faction_roster")
        .unwrap()
        .blocker;
    let label = map_authority_observed_status_label("diplomacy.faction_roster");
    projection.requirements[index].disposition = MapAuthorityRequirementDisposition::Absent;
    projection.requirements[index].blocker =
        Some(format!("{blocker}; WorldFrame {label} is DomainAbsent"));
    let error = admit_map_authority_projection(&projection).unwrap_err();
    assert!(
        error.contains("cannot observe DomainAbsent"),
        "{error}"
    );

    // A row whose predicate CAN observe an absence keeps the same form legal.
    let mut projection = unresolved_projection();
    let index = projection
        .requirements
        .iter()
        .position(|row| row.input_id == "combat.land_active_epoch")
        .unwrap();
    let blocker = map_authority_requirement_metadata("combat.land_active_epoch")
        .unwrap()
        .blocker;
    let label = map_authority_observed_status_label("combat.land_active_epoch");
    projection.requirements[index].disposition = MapAuthorityRequirementDisposition::Absent;
    projection.requirements[index].blocker =
        Some(format!("{blocker}; WorldFrame {label} is DomainAbsent"));
    assert!(admit_map_authority_projection(&projection).is_ok());
}

/// kills: admitting the D18 roster from a partial identity census. Each row
/// below keeps `countries_status` `Present` and breaks the roster in one way
/// `exact_d18_country_roster` is documented to refuse.
#[test]
fn the_d18_roster_refuses_every_partial_identity_census() {
    let manifest = fixture_manifest();
    let cases: &[(&str, Value)] = &[
        (
            "missing identity",
            json!({"ENG": {}, "GER": {"_r2_tag_db_id": 1}}),
        ),
        (
            "zero identity",
            json!({"ENG": {"_r2_tag_db_id": 0}, "GER": {"_r2_tag_db_id": 1}}),
        ),
        (
            "duplicate identity",
            json!({"ENG": {"_r2_tag_db_id": 1}, "GER": {"_r2_tag_db_id": 1}}),
        ),
        ("empty census", json!({})),
    ];
    for (name, countries) in cases {
        let frame = world_frame_with(json!({"countries": countries}));
        assert_eq!(
            frame.countries_status(),
            ViewStatus::Present,
            "{name}: fixture no longer keeps the weak accessor Present"
        );
        let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
        assert_ne!(
            requirement(&projection, "country.d18_complete_roster").disposition,
            MapAuthorityRequirementDisposition::Admitted,
            "{name} was admitted as a complete roster"
        );
    }
}

/// kills: admitting the faction roster on the diplomacy root, collapsing a
/// measured-empty roster into an unavailable one, or — the case Codex's
/// round-2 review found — admitting the LEGACY `diplomacy_live.faction_members`
/// object. Every frame below keeps `diplomacy.status` `Present` through the
/// `countries` sibling, so the root alone cannot distinguish any of them.
///
/// The legacy reader accepts any well-formed nonempty object as measured: it
/// has no expected faction count and no completeness witness, so a recording
/// that captured one faction and missed another is indistinguishable from a
/// complete one. Only the modern D53 route carries the producer's own coverage
/// claim, and only that may admit. A measured-EMPTY roster under that witness
/// is a measurement and must admit.
#[test]
fn only_a_witnessed_faction_roster_admits() {
    let manifest = fixture_manifest();
    let admits = |snapshot: Value| {
        let frame = world_frame_with(snapshot);
        assert_eq!(frame.diplomacy().status, ViewStatus::Present);
        let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
        requirement(&projection, "diplomacy.faction_roster").disposition
            == MapAuthorityRequirementDisposition::Admitted
    };
    assert!(
        admits(json!({
            "countries": measured_countries(),
            "diplomacy_world": {
                "domain53_extra": {
                    "faction_roster_status": "measured_nonempty",
                    // The modern row schema `parse_domain53_faction_rows`
                    // enforces: numeric nonzero `faction_id`, a nonempty
                    // duplicate-free TAG array, a `member_count` that agrees
                    // with it, and a `leader` drawn from the members.
                    "faction_rows": [{
                        "faction_id": 1,
                        "members": ["GER", "ENG"],
                        "member_count": 2,
                        "leader": "GER",
                    }],
                },
            },
        })),
        "a witnessed nonempty roster must admit"
    );
    assert!(
        admits(json!({
            "countries": measured_countries(),
            "diplomacy_world": {
                "domain53_extra": {
                    "faction_roster_status": "measured_empty",
                    "faction_rows": [],
                },
            },
        })),
        "a witnessed measured-empty roster is a measurement and must admit"
    );
    assert!(
        !admits(json!({
            "countries": measured_countries(),
            "diplomacy_live": {"faction_members": {"1": ["GER", "ENG"]}},
        })),
        "an unwitnessed legacy roster was admitted as complete"
    );
    assert!(
        !admits(json!({
            "countries": measured_countries(),
            "diplomacy_world": {
                "domain53_extra": {"faction_roster_status": "unavailable"},
            },
        })),
        "an explicitly unavailable modern roster was admitted"
    );
    assert!(
        !admits(json!({
            "countries": measured_countries(),
            "diplomacy_live": {"faction_members": Value::Null},
        })),
        "an unobtained legacy faction carrier was admitted"
    );
    assert!(
        !admits(json!({"countries": measured_countries()})),
        "an absent faction carrier was admitted"
    );
}

/// kills: dropping the member-identity join from `faction_roster_status`.
///
/// When the recorder cannot map a member's `tag_db_id` to a TAG it emits the
/// numeric id rather than fabricating one, and `parse_domain53_faction_rows`
/// accepts any nonempty string as a member. A witnessed row naming an
/// unresolved member therefore decodes as a measured nonempty roster, and
/// without the join it would be admitted as a measured membership census while
/// that member is explicitly unidentified. Found by Codex's round-4 review.
#[test]
fn a_faction_member_outside_the_d18_roster_is_not_a_measured_identity() {
    let manifest = fixture_manifest();
    let witnessed = |members: Value, member_count: u64, leader: &str| {
        json!({
            "countries": measured_countries(),
            "diplomacy_world": {
                "domain53_extra": {
                    "faction_roster_status": "measured_nonempty",
                    "faction_rows": [{
                        "faction_id": 1,
                        "members": members,
                        "member_count": member_count,
                        "leader": leader,
                    }],
                },
            },
        })
    };
    let disposition = |snapshot: Value| {
        let frame = world_frame_with(snapshot);
        // The precondition the whole test rests on: replay_core really does
        // decode the unresolved member as a MEASURED roster, so the refusal
        // below can only come from the admission predicate.
        assert!(
            matches!(
                frame.diplomacy().faction_roster_status(),
                replay_core::world::FactionRosterStatus::MeasuredNonEmpty
            ),
            "fixture no longer decodes as a measured nonempty roster"
        );
        let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
        requirement(&projection, "diplomacy.faction_roster").disposition
    };
    // "999" is what the fold emits for a member whose tag_db_id it could not
    // resolve; the D18 roster is GER/ENG only.
    assert_eq!(
        disposition(witnessed(json!(["GER", "999"]), 2, "GER")),
        MapAuthorityRequirementDisposition::Unavailable,
        "an unresolved faction member was admitted as measured identity"
    );
    assert_eq!(
        disposition(witnessed(json!(["GER", "ENG"]), 2, "GER")),
        MapAuthorityRequirementDisposition::Admitted,
        "a fully resolved witnessed roster must still admit"
    );
}

/// kills: propagating the D18 roster's own status out of `faction_roster_status`.
///
/// The faction carrier is independent of the `countries` key, so a witnessed
/// nonempty roster can be measured on a frame that carries no country census at
/// all. `complete_d18_roster` reports `DomainAbsent` for that frame; returning
/// it verbatim would describe a different view's absence, and because this row
/// declares `can_report_absent: false` the derived projection would then be
/// REJECTED WHOLESALE by `validate_requirement` instead of recording a gap —
/// one row's missing identity would cost the entire admission report.
#[test]
fn a_faction_roster_without_a_country_census_is_a_gap_not_a_rejected_projection() {
    let manifest = fixture_manifest();
    let frame = world_frame_with(json!({
        "diplomacy_world": {
            "domain53_extra": {
                "faction_roster_status": "measured_nonempty",
                "faction_rows": [{
                    "faction_id": 1,
                    "members": ["GER", "ENG"],
                    "member_count": 2,
                    "leader": "GER",
                }],
            },
        },
    }));
    assert!(
        matches!(
            frame.diplomacy().faction_roster_status(),
            replay_core::world::FactionRosterStatus::MeasuredNonEmpty
        ),
        "fixture no longer decodes as a measured nonempty roster"
    );
    assert_eq!(
        frame.d18_country_roster().status,
        ViewStatus::DomainAbsent,
        "fixture must have no country census for this test to mean anything"
    );
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    let row = requirement(&projection, "diplomacy.faction_roster");
    assert_eq!(
        row.disposition,
        MapAuthorityRequirementDisposition::Unavailable,
        "the roster's own DomainAbsent leaked out of the faction predicate"
    );
    // The whole projection must still admit as a gap report.
    let admission = admit_map_authority_projection(&projection).unwrap();
    assert!(!admission.is_admitted("diplomacy.faction_roster"));
}

/// kills: promoting one of lane HW's nine remaining candidates without first
/// closing the producer-side defect that blocks it. Each id below is a row the
/// gap map classifies ADMISSION-PROOF-ONLY and this lane proved is not:
/// `extractor/native/runs/hw_admission_a/ROW_PROOFS.md` names the exact producer
/// fact for each. Promoting any of them here — without that fact changing —
/// would put a row into the admission table that no capture can ever satisfy,
/// which is worse than a gap because it reads as a live seam.
#[test]
fn lane_hw_rows_with_an_open_producer_defect_stay_unproven() {
    for input_id in [
        // `capital_state` is minted only by the collector's direct-JSON country
        // renderer; `git grep -c capital_state HEAD -- extractor/native/recorder_host/`
        // returns no hits, so the fold never writes it into the snapshot.
        "country.current_capital_state",
        // XMAP05 production needs the all-country `complete_pov_roster` FOW1
        // census; `complete_pov_roster` appears nowhere in `extractor/`.
        "fog.province_visibility_by_pov",
        "fog.spotted_land_by_pov",
        "fog.spotted_taskforces_by_pov",
        // The producer spells a legitimately-absent cosmetic tag as JSON null,
        // which arrives at the typed seam as `Unobtained`.
        "country.presentation_identity",
        // `build_rail_replay` admits only `exact_a729_crailwaygfx_carrier`; the
        // recorder emits `exact_a729_supply_simulation_tables`.
        "logistics.rail_hub_frames",
        "logistics.rail_topology_state",
        "mapmode.supply_state",
        // The ledger's own evidence anchor for this row is `map_authority.rs`
        // itself and its engine object reads "intelligence projection
        // unresolved"; promoting it would be circular.
        "mapmode.intelligence",
        // D57 is scheduled in none of the committed captures.
        "session.player_country_set",
    ] {
        assert!(
            map_authority_owner_proven_admission(input_id).is_none(),
            "{input_id} was promoted while its producer-side blocker is open"
        );
        let metadata = map_authority_requirement_metadata(input_id).unwrap();
        assert!(!metadata.blocker.is_empty(), "{input_id}");
    }
}

/// kills: dropping the JSON door's admission refusal, which would let bytes
/// that never saw a WorldFrame assert that a proven input was measured.
#[test]
fn caller_supplied_bytes_cannot_establish_an_admission() {
    let manifest = fixture_manifest();
    let frame = world_frame_with(fully_measured_snapshot());
    let projection = map_authority_projection_from_world_frame(&frame, &manifest).unwrap();
    // The same value admits in process, where it was derived from the frame.
    assert_eq!(
        admit_map_authority_projection(&projection)
            .unwrap()
            .admitted()
            .len(),
        MAP_AUTHORITY_OWNER_PROVEN_INPUTS.len()
    );
    let error = admit_map_authority_json(&serde_json::to_vec(&projection).unwrap()).unwrap_err();
    assert!(
        error.contains("derived from the authenticated WorldFrame")
            && error.contains("combat.land_active_epoch"),
        "{error}"
    );
}

/// kills: letting an admitted row keep a blocker, or a refused row drop one —
/// either would make the gap report's reason strings unreliable.
#[test]
fn admitted_rows_carry_no_blocker_and_refused_rows_must_keep_theirs() {
    let mut admitted_with_blocker = unresolved_projection();
    let proven = map_authority_owner_proven_admission(&admitted_with_blocker.requirements[0].input_id)
        .expect("requirements[0] is an owner-proven row");
    admitted_with_blocker.requirements[0].disposition =
        MapAuthorityRequirementDisposition::Admitted;
    admitted_with_blocker.requirements[0].payload_schema = Some(proven.payload_schema.into());
    admitted_with_blocker.requirements[0].cadence = Some(proven.cadence.into());
    admitted_with_blocker.requirements[0].identity_order = Some(proven.identity_order.into());
    assert!(admit_map_authority_projection(&admitted_with_blocker)
        .unwrap_err()
        .contains("is admitted and carries a blocker"));

    let mut refused_without_blocker = unresolved_projection();
    refused_without_blocker.requirements[0].blocker = None;
    assert!(admit_map_authority_projection(&refused_without_blocker)
        .unwrap_err()
        .contains("is refused without a blocker"));

    // A proven input cannot be refused while its own view reported Present:
    // the derivation turns that status into an admission, so the pairing is a
    // contradiction rather than a conservative refusal.
    let mut contradictory = unresolved_projection();
    let blocker = map_authority_requirement_metadata(&contradictory.requirements[0].input_id)
        .unwrap()
        .blocker;
    contradictory.requirements[0].blocker = Some(format!(
        "{blocker}; WorldFrame {} is Present",
        map_authority_observed_status_label(&contradictory.requirements[0].input_id)
    ));
    assert!(admit_map_authority_projection(&contradictory)
        .unwrap_err()
        .contains("canonical unresolved blocker"));
    // The same pairing stays legal for a family whose owner still owes a proof.
    let mut unproven_present = unresolved_projection();
    let index = unproven_present
        .requirements
        .iter()
        .position(|row| map_authority_owner_proven_admission(&row.input_id).is_none())
        .unwrap();
    let blocker = map_authority_requirement_metadata(&unproven_present.requirements[index].input_id)
        .unwrap()
        .blocker;
    unproven_present.requirements[index].blocker = Some(format!(
        "{blocker}; WorldFrame {} is Present",
        map_authority_observed_status_label(&unproven_present.requirements[index].input_id)
    ));
    assert!(admit_map_authority_projection(&unproven_present).is_ok());
}

fn shipping_performance_run(source: &str) -> &str {
    let start = source
        .find("pub(crate) fn run(")
        .expect("shipping performance run");
    let end = source[start..]
        .find("pub(crate) fn run_stress(")
        .expect("stress benchmark boundary");
    let bounded = &source[start..start + end];
    let function_end = bounded.rfind("\n}").expect("shipping performance end") + 2;
    &bounded[..function_end]
}

fn shipping_app_run(source: &str) -> &str {
    let start = source.find("pub fn run(").expect("shipping app run");
    let end = source[start..]
        .find("#[cfg(test)]")
        .expect("app test-module boundary");
    &source[start..start + end]
}

fn shipping_headless_dump(source: &str) -> &str {
    let start = source
        .find("fn dump_production_recording_frame(")
        .expect("shipping headless dump");
    let end = source[start..]
        .find("fn reject_production_dump_debug_inputs(")
        .expect("next headless function boundary");
    let bounded = &source[start..start + end];
    let function_end = bounded.rfind("\n}").expect("shipping headless end") + 2;
    &bounded[..function_end]
}

fn shipping_composition_start(source: &str) -> &str {
    let start = source
        .find("impl ProductMapCompositionSession {")
        .expect("XMAP22 composition implementation");
    let end = source[start..]
        .find("pub(crate) fn adapter_identity(")
        .expect("XMAP22 composition start boundary");
    &source[start..start + end]
}

fn assert_gate_sequence(name: &str, source: &str) -> usize {
    source
        .find("crate::validate::prepare_product_map_foundation(")
        .or_else(|| source.find("ProductMapCompositionSession::start_for_headless("))
        .or_else(|| source.find("ProductMapCompositionSession::start_for_benchmark("))
        .or_else(|| source.find("ProductMapCompositionSession::start_for_window_target("))
        .or_else(|| source.find("ProductMapFoundationSession::start_with_assigned_stage_schedule("))
        .or_else(|| source.find("ProductMapFoundationSession::start("))
        .unwrap_or_else(|| panic!("{name} does not enter the shared product foundation"))
}

fn assert_shared_foundation_sequence() {
    let source = include_str!("../../replay_app/src/validate.rs");
    for required_sequence in [
        &[
            "product_foundation_assigned_stage_plan()",
            "derive_bundle_content_authority(",
            "execute_foundation_content_preflight(",
            "product_feature_adapters_with_assigned_stage_schedule(",
        ][..],
        &[
            "map_authority_projection_from_world_frame(",
            "admit_map_authority_projection(",
            "prepare_with_xmap12_inputs(",
            "require_fresh_foundation_frame(",
        ][..],
    ] {
        let mut cursor = 0;
        for required in required_sequence {
            let offset = source[cursor..]
                .find(required)
                .unwrap_or_else(|| panic!("shared foundation omits {required}"));
            cursor += offset + required.len();
        }
    }
}

fn compact_rust_source(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect()
}

#[test]
fn shipping_product_gates_precede_all_presentation_and_device_prerequisites() {
    assert_shared_foundation_sequence();
    let public_api = compact_rust_source(include_str!("../src/lib.rs"));
    assert!(
        !public_api.contains("product_feature_adapters,"),
        "public map_viewer API must not expose the incomplete unscheduled adapter core"
    );
    let app = shipping_app_run(include_str!("../../replay_app/src/app.rs"));
    let headless = shipping_headless_dump(include_str!("../../replay_app/src/headless.rs"));
    let performance =
        shipping_performance_run(include_str!("../../replay_app/src/performance_bench.rs"));

    let app_gate = assert_gate_sequence("app", app);
    let headless_gate = assert_gate_sequence("headless", headless);
    let performance_gate = assert_gate_sequence("performance", performance);

    let app_prefix = &app[..app_gate];
    let headless_prefix = &headless[..headless_gate];
    let performance_prefix = &performance[..performance_gate];
    for (name, prefix) in [
        ("app", app_prefix),
        ("headless", headless_prefix),
        ("performance", performance_prefix),
    ] {
        for forbidden in [
            "WorkloadContract",
            "validate_contract(",
            "production_render_sha256",
            ".production_render(",
            ".production_render_status(",
            "selected_present_sha256",
            ".selected_present",
            "launch_epoch",
            "capture_receipt",
            "open_product_recording_segment_with_authority",
            "authority_sha256",
            "expected_map_operations",
            "expected_selected_present_operations",
            "map_operation_count",
            "selected_present_ui_operation_count",
            ".submit(",
            "ProductObserverUiRuntime::from_playback_content(",
            "Camera::fit(",
            "Window::create(",
            "Gfx::create(",
            "Gfx::create_offscreen(",
            "read_target(",
            "write_bmp(",
        ] {
            assert!(
                !prefix.contains(forbidden),
                "{name} consumes {forbidden} before XMAP refusal"
            );
        }
    }

    let app_condition = app[..app_gate]
        .rfind("if production_recording {")
        .expect("app production-only gate condition");
    assert!(app_condition < app_gate);
    assert!(app[app_gate..].contains("if observer_render_admitted && !production_recording"));

    for (name, source, gate) in [
        ("headless", headless, headless_gate),
        ("performance", performance, performance_gate),
    ] {
        let tail = &source[gate..];
        assert!(
            tail.contains("composition.execute("),
            "{name} does not execute the shared XMAP22 composition"
        );
        for forbidden in [
            "production_render",
            "selected_present",
            ".submit(",
            "ProductObserverUiRuntime",
        ] {
            assert!(
                !tail.contains(forbidden),
                "{name} retains success tail token {forbidden}"
            );
        }
    }
}

#[test]
fn headless_and_recording_benchmark_execute_shared_composition() {
    let headless = shipping_headless_dump(include_str!("../../replay_app/src/headless.rs"));
    let performance =
        shipping_performance_run(include_str!("../../replay_app/src/performance_bench.rs"));
    for (name, source) in [("headless", headless), ("performance", performance)] {
        let gate = assert_gate_sequence(name, source);
        let execution = source[gate..]
            .find("composition.execute(")
            .unwrap_or_else(|| panic!("{name} does not execute the XMAP22 composition"));
        assert!(
            execution > 0,
            "{name} executes before starting the XMAP22 composition"
        );
    }

    let composition = shipping_composition_start(include_str!("../../replay_app/src/app.rs"));
    let foundation = composition
        .find("ProductMapFoundationSession::start_with_assigned_stage_schedule(")
        .expect("XMAP22 composition does not enter the shared product foundation");
    let device = composition
        .find("Gfx::create_offscreen(")
        .expect("XMAP22 composition does not create its offscreen target");
    assert!(
        foundation < device,
        "XMAP22 composition creates a device before content/foundation admission"
    );
}

#[test]
fn shipping_help_and_source_truth_are_bound_to_all_three_foundation_gates() {
    let main = include_str!("../../replay_app/src/main.rs");
    let usage_start = main.find("const USAGE: &str").expect("shipping usage");
    let usage_end = main[usage_start..]
        .find("const SENSITIVE_GAME_FILE_PATHS")
        .expect("usage boundary");
    let usage = &main[usage_start..usage_start + usage_end];
    for required in [
        "executes and measures the complete packet-native map composition with GPU completion, and writes no benchmark receipt unless exhaustive map-family admission and native execution succeed",
        "executes the complete packet-native map composition, and emits no map pixels, BMP, or success receipt unless exhaustive map-family admission and native execution succeed",
    ] {
        assert!(compact_rust_source(usage).contains(&compact_rust_source(required)));
    }

    let headless_source = include_str!("../../replay_app/src/headless.rs");
    let headless_prelude = &headless_source[..headless_source
        .find("use std::io::Write;")
        .expect("headless module prelude boundary")];
    assert!(
        compact_rust_source(headless_prelude).contains(&compact_rust_source(
            "same XMAP22 packet-native composition used by windowed playback",
        ))
    );

    let status = include_str!("../../../documents/MAP_GRAPHICS_PRODUCT_STATUS.md");
    for required in [
        "Windowed playback",
        "headless frame export",
        "recording benchmark",
        "ProductMapCompositionSession::execute_on_target",
    ] {
        assert!(
            compact_rust_source(status).contains(&compact_rust_source(required)),
            "map product status omits current route token {required}"
        );
    }

    let app = shipping_app_run(include_str!("../../replay_app/src/app.rs"));
    let headless = shipping_headless_dump(headless_source);
    let performance =
        shipping_performance_run(include_str!("../../replay_app/src/performance_bench.rs"));
    assert_gate_sequence("app", app);
    assert_gate_sequence("headless", headless);
    assert_gate_sequence("performance", performance);
}

#[test]
fn content_manifest_absence_is_a_pre_admission_envelope_error() {
    for (name, source, message) in [
        (
            "app",
            shipping_app_run(include_str!("../../replay_app/src/app.rs")),
            "portable playback has no admitted content manifest after binding",
        ),
        (
            "headless",
            shipping_headless_dump(include_str!("../../replay_app/src/headless.rs")),
            "portable playback has no admitted content manifest after binding",
        ),
        (
            "performance",
            shipping_performance_run(include_str!("../../replay_app/src/performance_bench.rs")),
            "performance benchmark recording has no admitted content manifest",
        ),
    ] {
        let structural_error = source
            .find(message)
            .unwrap_or_else(|| panic!("{name} has no stable content-envelope error"));
        let projection = assert_gate_sequence(name, source);
        assert!(
            structural_error < projection,
            "{name} mislabeled a pre-admission content-envelope failure as a family gap"
        );
    }

    let shared_foundation = include_str!("../../replay_app/src/validate.rs");
    assert!(
        shared_foundation.contains("map_authority_projection_from_world_frame("),
        "shared product foundation has no XMAP00 projection"
    );
}
