use std::env;
use std::fs;
use std::path::PathBuf;

use naval_combat_live_lane::{
    build_capture_frame, build_transition_capture_frame, gate_stream, result_json, CombatSide,
    CshipSnapshotRow, MemberBridgeRow, NavalCombatBodyBridgeRow, ValidatedCombatSnapshot,
    CURRENT_870EBF_BUILD_SHA256, CURRENT_870EBF_CSHIP_ROUTE, DOMAIN_NAVAL_COMBAT_LIFECYCLE,
    DOMAIN_NAVAL_COMBAT_MEMBERS, STATUS_LOCATION_CANDIDATE_UNVALIDATED,
};
use serde::Deserialize;
use serde_json::{json, Value};
use snapshot::{pack, Container, ExpectedIdentity, PackEntry, PackIdentity};

fn main() {
    let mut out_dir = PathBuf::from("extractor/native/runs/naval_combat_live_lane_stack_proof");
    let mut fixture_json_path: Option<PathBuf> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--out-dir" {
            let Some(value) = args.next() else {
                eprintln!("--out-dir requires a path");
                std::process::exit(2);
            };
            out_dir = PathBuf::from(value);
        } else if arg == "--fixture-json" {
            let Some(value) = args.next() else {
                eprintln!("--fixture-json requires a path");
                std::process::exit(2);
            };
            fixture_json_path = Some(PathBuf::from(value));
        } else {
            eprintln!("unknown argument: {arg}");
            std::process::exit(2);
        }
    }

    if let Err(err) = fs::create_dir_all(&out_dir) {
        eprintln!("create {} failed: {err}", out_dir.display());
        std::process::exit(1);
    }

    if let Some(path) = fixture_json_path {
        match run_fixture_json_proof(&path, &out_dir) {
            Ok(ok) => std::process::exit(if ok { 0 } else { 1 }),
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
    }

    let fixture = fixture();
    let mut updated_fixture = fixture.clone();
    updated_fixture.lifecycle.attacker_positioning_milli = Some(835);
    updated_fixture.members[0].engagement_weight_raw = Some(1_250);
    let cships = [cship(1001)];
    let start =
        match build_transition_capture_frame(&[], &[fixture.clone()], &cships, 100, 60787539, 7) {
            Ok(frame) => frame,
            Err(errors) => {
                eprintln!("synthetic start fixture unexpectedly failed: {errors:?}");
                std::process::exit(1);
            }
        };
    let update = match build_transition_capture_frame(
        &[fixture.clone()],
        &[updated_fixture.clone()],
        &cships,
        102,
        60787540,
        17,
    ) {
        Ok(frame) => frame,
        Err(errors) => {
            eprintln!("synthetic update fixture unexpectedly failed: {errors:?}");
            std::process::exit(1);
        }
    };
    let end =
        match build_transition_capture_frame(&[updated_fixture], &[], &cships, 104, 60787541, 29) {
            Ok(frame) => frame,
            Err(errors) => {
                eprintln!("synthetic end fixture unexpectedly failed: {errors:?}");
                std::process::exit(1);
            }
        };
    let mut compact_stream = start.compact_stream;
    compact_stream.extend_from_slice(&update.compact_stream);
    compact_stream.extend_from_slice(&end.compact_stream);
    let compact_path = out_dir.join("naval_combat_live_domain49_50_compact.bin");
    if let Err(err) = fs::write(&compact_path, &compact_stream) {
        eprintln!("write {} failed: {err}", compact_path.display());
        std::process::exit(1);
    }

    let gate = gate_stream(&compact_stream);
    let gate_json = result_json(&gate);
    let gate_path = out_dir.join("naval_combat_live_domain49_50_gate.json");
    if let Err(err) = fs::write(&gate_path, &gate_json) {
        eprintln!("write {} failed: {err}", gate_path.display());
        std::process::exit(1);
    }

    let replay_path = out_dir.join("native_session.battleplan");
    let replay_verify = match write_and_verify_replay(
        &replay_path,
        &compact_stream,
        "naval_combat_live_lane_synthetic",
        "lane-local-compact-v1",
    ) {
        Ok(report) => report,
        Err(err) => {
            eprintln!("replay verification failed: {err}");
            std::process::exit(1);
        }
    };

    let empty_fixture_errors = build_capture_frame(&[], &cships, 200, 60787540, 1)
        .err()
        .unwrap_or_default();
    let proof_json = format!(
        concat!(
            "{{\n",
            "  \"status\": \"passed\",\n",
            "  \"domains\": {{\"49\": \"naval_combat_lifecycle_live\", \"50\": \"naval_combat_members_live\"}},\n",
            "  \"domain_ids\": [{}, {}],\n",
            "  \"compact_path\": {},\n",
            "  \"gate_path\": {},\n",
            "  \"replay_path\": {},\n",
            "  \"hoi4replay_verified\": {},\n",
            "  \"collector_compact_stream_byte_match\": {},\n",
            "  \"collector_compact_stream_bytes\": {},\n",
            "  \"gate_ok\": {},\n",
            "  \"lifecycle_rows\": {},\n",
            "  \"member_rows\": {},\n",
            "  \"two_sided_combat_count\": {},\n",
            "  \"lifecycle_location_row_count\": {},\n",
            "  \"lifecycle_region_row_count\": {},\n",
            "  \"location_candidate_unvalidated\": {},\n",
            "  \"lifecycle_started_row_count\": {},\n",
            "  \"lifecycle_updated_row_count\": {},\n",
            "  \"lifecycle_ended_row_count\": {},\n",
            "  \"member_deleted_row_count\": {},\n",
            "  \"cship_validated_member_count\": {},\n",
            "  \"task_force_linked_member_count\": {},\n",
            "  \"fleet_linked_member_count\": {},\n",
            "  \"hull_telemetry_member_count\": {},\n",
            "  \"sunk_convoy_telemetry_member_count\": {},\n",
            "  \"empty_fixture_rejected\": {},\n",
            "  \"empty_fixture_errors\": {}\n",
            "}}\n"
        ),
        DOMAIN_NAVAL_COMBAT_LIFECYCLE,
        DOMAIN_NAVAL_COMBAT_MEMBERS,
        json_str(&compact_path.to_string_lossy()),
        json_str(&gate_path.to_string_lossy()),
        json_str(&replay_path.to_string_lossy()),
        replay_verify.verified,
        replay_verify.byte_match,
        replay_verify.collector_compact_stream_bytes,
        gate.ok,
        gate.summary.lifecycle_row_count,
        gate.summary.member_row_count,
        gate.summary.two_sided_combat_count,
        gate.summary.lifecycle_location_row_count,
        gate.summary.lifecycle_region_row_count,
        (gate.summary.status_flags_or & STATUS_LOCATION_CANDIDATE_UNVALIDATED) != 0,
        gate.summary.lifecycle_started_row_count,
        gate.summary.lifecycle_updated_row_count,
        gate.summary.lifecycle_ended_row_count,
        gate.summary.member_deleted_row_count,
        gate.summary.cship_validated_member_count,
        gate.summary.task_force_linked_member_count,
        gate.summary.fleet_linked_member_count,
        gate.summary.hull_telemetry_member_count,
        gate.summary.sunk_convoy_telemetry_member_count,
        !empty_fixture_errors.is_empty(),
        json_string_array(&empty_fixture_errors),
    );
    let proof_path = out_dir.join("naval_combat_live_lane_stack_proof.json");
    if let Err(err) = fs::write(&proof_path, &proof_json) {
        eprintln!("write {} failed: {err}", proof_path.display());
        std::process::exit(1);
    }
    println!("{proof_json}");
    std::process::exit(
        if gate.ok
            && gate.summary.lifecycle_started_row_count > 0
            && gate.summary.lifecycle_updated_row_count > 0
            && gate.summary.lifecycle_ended_row_count > 0
            && gate.summary.lifecycle_location_row_count == 0
            && gate.summary.lifecycle_region_row_count == 0
            && (gate.summary.status_flags_or & STATUS_LOCATION_CANDIDATE_UNVALIDATED) != 0
            && gate.summary.member_deleted_row_count > 0
            && gate.summary.task_force_linked_member_count > 0
            && gate.summary.fleet_linked_member_count > 0
            && gate.summary.hull_telemetry_member_count > 0
            && !empty_fixture_errors.is_empty()
        {
            0
        } else {
            1
        },
    );
}

#[derive(Debug, Deserialize)]
struct FixtureProofInput {
    #[serde(default)]
    source: Value,
    #[serde(default = "default_game_hour_raw")]
    game_hour_raw: i32,
    #[serde(default = "default_elapsed_wall_ms")]
    elapsed_wall_ms: u64,
    snapshots: Vec<FixtureSnapshotInput>,
    cships: Vec<CshipSnapshotInput>,
}

#[derive(Debug, Deserialize)]
struct FixtureSnapshotInput {
    lifecycle: LifecycleInput,
    #[serde(default)]
    members: Vec<MemberInput>,
}

#[derive(Debug, Deserialize)]
struct LifecycleInput {
    combat_id: u32,
    generation: u64,
    body_ptr: u64,
    location_id: Option<u32>,
    region_id: Option<u32>,
    attacker_member_count: u32,
    defender_member_count: u32,
    attacker_positioning_milli: Option<i32>,
    defender_positioning_milli: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct MemberInput {
    combat_id: u32,
    generation: u64,
    side: String,
    group_index: u32,
    member_index: u32,
    member_ptr: u64,
    unit_type_tag: u32,
    ship_instance_id: Option<u32>,
    #[serde(default)]
    is_convoy: bool,
    owner_tag_db_id: Option<u32>,
    designed_max_strength_raw: Option<u32>,
    #[serde(default)]
    battle_slot_index_raw: u32,
    state_bitmask: Option<u32>,
    engagement_weight_raw: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct CshipSnapshotInput {
    instance_id: u32,
    body_ptr: u64,
    #[serde(default = "default_cship_vtable_rva")]
    vtable_rva: u64,
    #[serde(default = "default_cship_type_tag")]
    handle_type_tag: u32,
    alias_instance_id: u32,
    #[serde(default = "default_cship_ref100")]
    ref100_sentinel_raw: u32,
    task_force_id: Option<u32>,
    fleet_id: Option<u32>,
    strength_raw: u32,
    organisation_raw: u32,
    experience_raw: u32,
    sunk_convoys: u32,
    category_enum: u32,
}

fn run_fixture_json_proof(fixture_path: &PathBuf, out_dir: &PathBuf) -> Result<bool, String> {
    let input_text = fs::read_to_string(fixture_path)
        .map_err(|err| format!("read {} failed: {err}", fixture_path.display()))?;
    let input: FixtureProofInput = serde_json::from_str(input_text.trim_start_matches('\u{feff}'))
        .map_err(|err| format!("parse {} failed: {err}", fixture_path.display()))?;
    validate_fixture_source_build(&input.source)?;
    let snapshots = input.to_snapshots()?;
    let cships = input.to_cships()?;
    let frame = build_capture_frame(
        &snapshots,
        &cships,
        1,
        input.game_hour_raw,
        input.elapsed_wall_ms,
    )
    .map_err(|errors| format!("fixture-json capture failed: {errors:?}"))?;

    let compact_path = out_dir.join("naval_combat_live_domain49_50_compact.bin");
    fs::write(&compact_path, &frame.compact_stream)
        .map_err(|err| format!("write {} failed: {err}", compact_path.display()))?;
    let gate = gate_stream(&frame.compact_stream);
    let gate_json = result_json(&gate);
    let gate_path = out_dir.join("naval_combat_live_domain49_50_gate.json");
    fs::write(&gate_path, &gate_json)
        .map_err(|err| format!("write {} failed: {err}", gate_path.display()))?;

    let replay_path = out_dir.join("native_session.battleplan");
    let replay_verify = write_and_verify_replay(
        &replay_path,
        &frame.compact_stream,
        "naval_combat_live_active_fixture_bridge",
        "lane-local-active-fixture-v1",
    )?;
    let ok = gate.ok
        && gate.summary.lifecycle_row_count > 0
        && gate.summary.member_row_count > 0
        && gate.summary.two_sided_combat_count > 0
        && gate.summary.lifecycle_location_row_count == 0
        && gate.summary.lifecycle_region_row_count == 0
        && (gate.summary.status_flags_or & STATUS_LOCATION_CANDIDATE_UNVALIDATED) != 0
        && gate.summary.cship_validated_member_count > 0
        && gate.summary.task_force_linked_member_count == gate.summary.cship_validated_member_count
        && gate.summary.fleet_linked_member_count == gate.summary.cship_validated_member_count
        && gate.summary.hull_telemetry_member_count == gate.summary.cship_validated_member_count
        && gate.summary.sunk_convoy_telemetry_member_count
            == gate.summary.cship_validated_member_count
        && replay_verify.verified
        && replay_verify.byte_match;

    let report = json!({
        "status": if ok { "passed" } else { "failed" },
        "proof_kind": "active_fixture_bridge_json",
        "domains": {
            "49": "naval_combat_lifecycle_live",
            "50": "naval_combat_members_live"
        },
        "domain_ids": [DOMAIN_NAVAL_COMBAT_LIFECYCLE, DOMAIN_NAVAL_COMBAT_MEMBERS],
        "fixture_json_path": fixture_path.to_string_lossy(),
        "source": input.source,
        "compact_path": compact_path.to_string_lossy(),
        "gate_path": gate_path.to_string_lossy(),
        "replay_path": replay_path.to_string_lossy(),
        "hoi4replay_verified": replay_verify.verified,
        "collector_compact_stream_byte_match": replay_verify.byte_match,
        "collector_compact_stream_bytes": replay_verify.collector_compact_stream_bytes,
        "gate_ok": gate.ok,
        "gate_errors": gate.errors,
        "lifecycle_rows": gate.summary.lifecycle_row_count,
        "member_rows": gate.summary.member_row_count,
        "two_sided_combat_count": gate.summary.two_sided_combat_count,
        "lifecycle_location_row_count": gate.summary.lifecycle_location_row_count,
        "lifecycle_region_row_count": gate.summary.lifecycle_region_row_count,
        "location_candidate_unvalidated": (gate.summary.status_flags_or & STATUS_LOCATION_CANDIDATE_UNVALIDATED) != 0,
        "lifecycle_started_row_count": gate.summary.lifecycle_started_row_count,
        "lifecycle_updated_row_count": gate.summary.lifecycle_updated_row_count,
        "lifecycle_ended_row_count": gate.summary.lifecycle_ended_row_count,
        "member_deleted_row_count": gate.summary.member_deleted_row_count,
        "cship_validated_member_count": gate.summary.cship_validated_member_count,
        "task_force_linked_member_count": gate.summary.task_force_linked_member_count,
        "fleet_linked_member_count": gate.summary.fleet_linked_member_count,
        "hull_telemetry_member_count": gate.summary.hull_telemetry_member_count,
        "sunk_convoy_telemetry_member_count": gate.summary.sunk_convoy_telemetry_member_count,
    });
    let proof_path = out_dir.join("naval_combat_live_lane_stack_proof.json");
    fs::write(
        &proof_path,
        serde_json::to_string_pretty(&report).map_err(|err| err.to_string())? + "\n",
    )
    .map_err(|err| format!("write {} failed: {err}", proof_path.display()))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|err| err.to_string())?
    );
    Ok(ok)
}

impl FixtureProofInput {
    fn to_snapshots(&self) -> Result<Vec<ValidatedCombatSnapshot>, String> {
        self.snapshots
            .iter()
            .map(FixtureSnapshotInput::to_snapshot)
            .collect()
    }

    fn to_cships(&self) -> Result<Vec<CshipSnapshotRow>, String> {
        self.cships.iter().map(CshipSnapshotInput::to_row).collect()
    }
}

fn validate_fixture_source_build(source: &Value) -> Result<(), String> {
    let root = source
        .get("root")
        .and_then(Value::as_object)
        .ok_or_else(|| "fixture-json source.root is required".to_string())?;
    let sha = root
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| "fixture-json source.root.sha256 is required".to_string())?;
    let sha_ok = root
        .get("sha256_ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !sha_ok {
        return Err("fixture-json source.root.sha256_ok must be true".to_string());
    }
    if !sha.eq_ignore_ascii_case(CURRENT_870EBF_BUILD_SHA256) {
        return Err(format!(
            "fixture-json build sha mismatch: got {sha}, expected {CURRENT_870EBF_BUILD_SHA256}"
        ));
    }
    Ok(())
}

impl FixtureSnapshotInput {
    fn to_snapshot(&self) -> Result<ValidatedCombatSnapshot, String> {
        if self.lifecycle.location_id.is_some() || self.lifecycle.region_id.is_some() {
            return Err(
                "fixture-json lifecycle location_id/region_id are unvalidated; keep them absent until manual promotion"
                    .to_string(),
            );
        }
        Ok(ValidatedCombatSnapshot {
            lifecycle: NavalCombatBodyBridgeRow {
                combat_id: self.lifecycle.combat_id,
                generation: self.lifecycle.generation,
                body_ptr: self.lifecycle.body_ptr,
                location_id: None,
                region_id: None,
                attacker_member_count: self.lifecycle.attacker_member_count,
                defender_member_count: self.lifecycle.defender_member_count,
                attacker_positioning_milli: self.lifecycle.attacker_positioning_milli,
                defender_positioning_milli: self.lifecycle.defender_positioning_milli,
            },
            members: self
                .members
                .iter()
                .map(MemberInput::to_row)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl MemberInput {
    fn to_row(&self) -> Result<MemberBridgeRow, String> {
        let side = match self.side.as_str() {
            "attacker" => CombatSide::Attacker,
            "defender" => CombatSide::Defender,
            other => return Err(format!("unknown fixture member side {other:?}")),
        };
        Ok(MemberBridgeRow {
            combat_id: self.combat_id,
            generation: self.generation,
            side,
            group_index: self.group_index,
            member_index: self.member_index,
            member_ptr: self.member_ptr,
            unit_type_tag: self.unit_type_tag,
            ship_instance_id: self.ship_instance_id,
            is_convoy: self.is_convoy,
            owner_tag_db_id: self.owner_tag_db_id,
            designed_max_strength_raw: self.designed_max_strength_raw,
            battle_slot_index_raw: self.battle_slot_index_raw,
            state_bitmask: self.state_bitmask,
            engagement_weight_raw: self.engagement_weight_raw,
        })
    }
}

impl CshipSnapshotInput {
    fn to_row(&self) -> Result<CshipSnapshotRow, String> {
        Ok(CshipSnapshotRow {
            instance_id: self.instance_id,
            body_ptr: self.body_ptr,
            vtable_rva: self.vtable_rva,
            handle_type_tag: self.handle_type_tag,
            alias_instance_id: self.alias_instance_id,
            ref100_sentinel_raw: self.ref100_sentinel_raw,
            task_force_id: self.task_force_id,
            fleet_id: self.fleet_id,
            strength_raw: self.strength_raw,
            organisation_raw: self.organisation_raw,
            experience_raw: self.experience_raw,
            sunk_convoys: self.sunk_convoys,
            category_enum: self.category_enum,
        })
    }
}

fn default_game_hour_raw() -> i32 {
    0
}

fn default_elapsed_wall_ms() -> u64 {
    1_000
}

fn default_cship_vtable_rva() -> u64 {
    CURRENT_870EBF_CSHIP_ROUTE.cship_vtable_rva
}

fn default_cship_type_tag() -> u32 {
    CURRENT_870EBF_CSHIP_ROUTE.type_tag
}

fn default_cship_ref100() -> u32 {
    CURRENT_870EBF_CSHIP_ROUTE.ref100_sentinel_raw
}

struct ReplayVerifyReport {
    verified: bool,
    byte_match: bool,
    collector_compact_stream_bytes: usize,
}

fn write_and_verify_replay(
    replay_path: &PathBuf,
    compact_stream: &[u8],
    game_id: &str,
    schema_version: &str,
) -> Result<ReplayVerifyReport, String> {
    let index = format!(
        "{{\"game_id\":{},\"schema_version\":{},\"snapshots\":[]}}",
        json_str(game_id),
        json_str(schema_version)
    )
    .into_bytes();
    let entries = vec![
        PackEntry {
            path: format!("payload/{game_id}/index.json"),
            bytes: index,
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{game_id}/collector_stream.bin"),
            bytes: compact_stream.to_vec(),
            role: "collector_compact_stream",
            hour_index: None,
        },
    ];
    let identity = PackIdentity {
        schema_version: Some(schema_version.to_string()),
        build_exe_sha256: Some(CURRENT_870EBF_BUILD_SHA256.to_string()),
        build_pinned: true,
        mod_identity: None,
        created_utc: Some("2026-07-01T00:00:00Z".to_string()),
    };
    let replay = pack(game_id, &entries, &identity, true).map_err(|err| err.to_string())?;
    fs::write(replay_path, replay).map_err(|err| err.to_string())?;
    let container = Container::open(replay_path, &ExpectedIdentity::default())
        .map_err(|err| err.to_string())?;
    container.verify_all().map_err(|err| err.to_string())?;
    let stream_entries = container.entries_by_role("collector_compact_stream");
    if stream_entries.len() != 1 {
        return Err(format!(
            "expected one collector_compact_stream entry, found {}",
            stream_entries.len()
        ));
    }
    let stored = container
        .read_entry_bytes(&stream_entries[0].path)
        .map_err(|err| err.to_string())?;
    if stored.as_slice() != compact_stream {
        return Err("collector_compact_stream bytes differ from compact stream".to_string());
    }
    Ok(ReplayVerifyReport {
        verified: true,
        byte_match: true,
        collector_compact_stream_bytes: stored.len(),
    })
}

fn fixture() -> ValidatedCombatSnapshot {
    ValidatedCombatSnapshot {
        lifecycle: NavalCombatBodyBridgeRow {
            combat_id: 240_580,
            generation: 1,
            body_ptr: 0x22aa_0004_000,
            location_id: None,
            region_id: None,
            attacker_member_count: 1,
            defender_member_count: 1,
            attacker_positioning_milli: Some(812),
            defender_positioning_milli: Some(765),
        },
        members: vec![
            MemberBridgeRow {
                combat_id: 240_580,
                generation: 1,
                side: CombatSide::Attacker,
                group_index: 0,
                member_index: 0,
                member_ptr: 0x22aa_0020_000,
                unit_type_tag: CURRENT_870EBF_CSHIP_ROUTE.type_tag,
                ship_instance_id: Some(1001),
                is_convoy: false,
                owner_tag_db_id: Some(17),
                designed_max_strength_raw: Some(10_000_000),
                battle_slot_index_raw: 0,
                state_bitmask: Some(1),
                engagement_weight_raw: Some(1_000),
            },
            MemberBridgeRow {
                combat_id: 240_580,
                generation: 1,
                side: CombatSide::Defender,
                group_index: 0,
                member_index: 0,
                member_ptr: 0x22aa_0021_000,
                unit_type_tag: 0,
                ship_instance_id: None,
                is_convoy: true,
                owner_tag_db_id: Some(18),
                designed_max_strength_raw: Some(10_000_000),
                battle_slot_index_raw: 1,
                state_bitmask: Some(1),
                engagement_weight_raw: Some(500),
            },
        ],
    }
}

fn cship(id: u32) -> CshipSnapshotRow {
    CshipSnapshotRow {
        instance_id: id,
        body_ptr: 0x22aa_0100_000,
        vtable_rva: CURRENT_870EBF_CSHIP_ROUTE.cship_vtable_rva,
        handle_type_tag: CURRENT_870EBF_CSHIP_ROUTE.type_tag,
        alias_instance_id: id,
        ref100_sentinel_raw: CURRENT_870EBF_CSHIP_ROUTE.ref100_sentinel_raw,
        task_force_id: Some(477),
        fleet_id: Some(88),
        strength_raw: 10_000_000,
        organisation_raw: 9_000_000,
        experience_raw: 100_000,
        sunk_convoys: 0,
        category_enum: 3,
    }
}

fn json_string_array(values: &[String]) -> String {
    let parts = values
        .iter()
        .map(|value| json_str(value))
        .collect::<Vec<_>>();
    format!("[{}]", parts.join(", "))
}

fn json_str(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fixture_source_requires_expected_build_sha() {
        let source = json!({
            "root": {
                "sha256": CURRENT_870EBF_BUILD_SHA256,
                "sha256_ok": true
            }
        });

        assert!(validate_fixture_source_build(&source).is_ok());
    }

    #[test]
    fn fixture_source_rejects_missing_or_mismatched_build_sha() {
        assert!(validate_fixture_source_build(&json!({})).is_err());
        assert!(validate_fixture_source_build(&json!({
            "root": {
                "sha256": CURRENT_870EBF_BUILD_SHA256,
                "sha256_ok": false
            }
        }))
        .is_err());
        assert!(validate_fixture_source_build(&json!({
            "root": {
                "sha256": "not-870ebf",
                "sha256_ok": true
            }
        }))
        .is_err());
    }

    #[test]
    fn fixture_snapshot_rejects_promoted_location_fields() {
        let fixture: FixtureSnapshotInput = serde_json::from_value(json!({
            "lifecycle": {
                "combat_id": 1,
                "generation": 1,
                "body_ptr": 4096,
                "location_id": 2799,
                "region_id": null,
                "attacker_member_count": 1,
                "defender_member_count": 1
            },
            "members": []
        }))
        .expect("fixture parses");

        let error = fixture.to_snapshot().unwrap_err();
        assert!(error.contains("location_id/region_id are unvalidated"));
    }
}
