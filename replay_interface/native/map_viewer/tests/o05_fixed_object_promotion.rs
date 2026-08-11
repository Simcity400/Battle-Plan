use map_viewer::{
    admit_o05_fixed_objects, admit_o05_fixed_objects_from_resolver,
    o05_static_building_source_identities_from_resolver,
    o05_static_building_source_identities_from_source, GameFileResolver,
    O05_FIXED_OBJECT_SOURCE_HASHES,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn evidence(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../re_evidence/rendering_laws/O05")
        .join(name);
    serde_json::from_slice(
        &fs::read(&path)
            .unwrap_or_else(|error| panic!("read O05 evidence {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("parse O05 evidence {}: {error}", path.display()))
}

fn frozen_hashes() -> BTreeMap<String, String> {
    O05_FIXED_OBJECT_SOURCE_HASHES
        .iter()
        .map(|(source, hash)| ((*source).to_string(), (*hash).to_string()))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    out
}

#[test]
fn source_line_identity_is_hash_bound_and_fail_closed() {
    let rows = o05_static_building_source_identities_from_source(
        b"# exact line ownership\n9;landmark_spawn;3031.29;11.30;1525.67;5.96;0\n",
    )
    .expect("parse exact O05 source row");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source_line, 2);
    assert!(rows[0].semantic_id.starts_with("map/buildings.txt@"));
    assert!(rows[0].semantic_id.ends_with("#L2"));

    let error = o05_static_building_source_identities_from_source(
        b"9;landmark_spawn;3031.29;11.30;1525.67;5.96\n",
    )
    .expect_err("malformed source must reject");
    assert!(error.to_string().contains("exactly seven"));
}

#[test]
fn any_fixed_source_drift_blocks_all_fixed_objects() {
    let mut hashes = frozen_hashes();
    hashes.insert(
        "gfx/entities/mapitems.gfx".to_string(),
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string(),
    );
    let error = admit_o05_fixed_objects(&hashes).expect_err("source drift must reject");
    assert!(error.to_string().contains("admission blocked"));
    assert!(error.to_string().contains("gfx/entities/mapitems.gfx"));
}

#[test]
#[cfg_attr(not(gate_hoi4_install), ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path")]
fn vanilla_admission_matches_all_frozen_o05_semantics() {
    let game_dir = replay_test_gates::require_vanilla_game_root();
    let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");

    let actual_static = o05_static_building_source_identities_from_resolver(&resolver)
        .expect("O05 static source identities");
    assert_eq!(actual_static.len(), 66_664);
    let by_line = actual_static
        .iter()
        .map(|row| (row.source_line, row))
        .collect::<BTreeMap<_, _>>();
    let semantic = evidence("semantic_identity.json");
    let records = semantic["records"].as_array().expect("semantic records");
    assert_eq!(records.len(), 281);
    for expected in records {
        let line = expected["source_line"].as_u64().unwrap() as usize;
        let actual = by_line
            .get(&line)
            .unwrap_or_else(|| panic!("missing source line {line}"));
        assert_eq!(
            actual.semantic_id,
            expected["semantic_id"].as_str().unwrap()
        );
        let placement = &expected["placement"];
        assert_eq!(
            actual.placement.state_id,
            placement["state_id"].as_u64().unwrap() as u32
        );
        assert_eq!(
            actual.placement.building,
            placement["building"].as_str().unwrap()
        );
        assert_eq!(
            actual.placement.pos[0],
            placement["x"].as_f64().unwrap() as f32
        );
        assert_eq!(
            actual.placement.pos[1],
            placement["y"].as_f64().unwrap() as f32
        );
        assert_eq!(
            actual.placement.pos[2],
            placement["z"].as_f64().unwrap() as f32
        );
        assert_eq!(
            actual.placement.rotation,
            placement["rotation"].as_f64().unwrap() as f32
        );
        assert_eq!(
            actual.placement.adjacent_sea_province,
            placement["adjacency"].as_u64().unwrap() as u32
        );
    }

    let actual_fixed =
        admit_o05_fixed_objects_from_resolver(&resolver).expect("O05 exact fixed-object admission");
    let fixed = evidence("fixed_object_semantics.json");
    let expected_fixed = fixed["objects"].as_array().expect("fixed objects");
    assert_eq!(actual_fixed.len(), 4);
    assert_eq!(expected_fixed.len(), 4);
    let actual_by_ordinal = actual_fixed
        .iter()
        .map(|row| (row.retained_oracle_ordinal, row))
        .collect::<BTreeMap<_, _>>();
    for expected in expected_fixed {
        let ordinal = expected["oracle_ordinal"].as_u64().unwrap() as u32;
        let actual = actual_by_ordinal.get(&ordinal).unwrap();
        assert_eq!(
            actual.semantic_id,
            expected["semantic_id"].as_str().unwrap()
        );
        assert_eq!(
            actual.class.evidence_label(),
            expected["identity_class"].as_str().unwrap()
        );
        assert_eq!(
            actual.source_mesh,
            expected["captured"]["mesh"].as_str().unwrap()
        );
        assert_eq!(
            actual.shape,
            expected["captured"]["shape"].as_str().unwrap()
        );
        assert_eq!(
            hex(&actual.world_matrix_raw),
            expected["captured"]["world_matrix_raw_hex"]
                .as_str()
                .unwrap()
        );
    }
}
