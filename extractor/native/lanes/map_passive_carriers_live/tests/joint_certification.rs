//! Joint certification, producer half.
//!
//! The replay consumer authenticates the XMAP17 faction carrier by re-hashing
//! `serde_json::to_vec` of its OWN struct definitions. That makes the contract
//! byte-level: an added, renamed, reordered or differently-typed field on
//! either side changes the digest and the hour becomes unavailable, silently,
//! at replay time.
//!
//! This test pins the exact bytes the producer emits into a committed fixture;
//! `replay_interface/native/replay_core/src/xmap17_player_labels.rs`'s
//! `lane_hy_joint_certification_fixture_is_admitted` reads the SAME file and
//! drives it through `admit_xmap17_faction_labels`. Either side drifting fails
//! a gate here instead of producing an unreadable recording.
//!
//! # What keeps this from being vacuous
//!
//! A producer that computes its own hashes and feeds its own output into the
//! consumer proves schema agreement and nothing else. So the fixture engine
//! layout below is built to exercise the parts a self-consistent round trip
//! and a census whose map order differs from its numeric order. The consumer
//! half additionally mutates one byte of the recorder's payload and requires
//! the CONSUMER's own authentication to reject it, so neither side can pass by
//! agreeing with itself.
//!
//! Regenerate deliberately, never casually:
//!
//! ```text
//! HY_WRITE_XMAP17_FIXTURE=1 cargo test -p map_passive_carriers_live_lane --test joint_certification
//! ```

use map_passive_carriers_live_lane::xmap17::{
    build_faction_labels_carrier, FactionCensusEntry, FrameIdentity,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

mod engine {
    //! A synthetic engine layout, constructed independently of the crate's own
    //! unit-test fixture so the two cannot drift together.

    use map_passive_carriers_live_lane::anchor::GAMESTATE_SINGLETON_RVA;
    use map_passive_carriers_live_lane::memory::SliceMemory;
    use map_passive_carriers_live_lane::xmap17::{
        FACTION_FALLBACK_HOLDER_OFFSET, FACTION_OWN_TEXT_OFFSET, FACTION_PACKED_IDENTITY_OFFSET,
        FACTION_TYPE_EXPECTED, FALLBACK_HOLDER_TEXT_OFFSET,
    };

    pub const MODULE_BASE: u64 = 0x1_4000_0000;
    pub const GAMESTATE: u64 = 0x6_0000_0000;
    pub const FACTION_ALLIES: u64 = 0x6_0010_0000;
    pub const FACTION_AXIS: u64 = 0x6_0011_0000;
    pub const AXIS_HOLDER: u64 = 0x6_0012_0000;
    pub const COMINTERN_HEAP: u64 = 0x6_0013_0000;
    pub const FACTION_COMINTERN: u64 = 0x6_0014_0000;

    fn put_inline(memory: &mut SliceMemory, object: u64, text: &str) {
        assert!(text.len() <= 0xf, "inline strings are at most 15 bytes");
        memory.put_u64(object + 0x10, text.len() as u64);
        memory.put_u64(object + 0x18, 0xf);
        memory.put_bytes(object, text.as_bytes());
    }

    fn put_heap(memory: &mut SliceMemory, object: u64, heap: u64, text: &str) {
        memory.put_u64(object, heap);
        memory.put_u64(object + 0x10, text.len() as u64);
        memory.put_u64(object + 0x18, text.len() as u64 + 1);
        memory.put_bytes(heap, text.as_bytes());
    }

    pub fn memory() -> SliceMemory {
        let mut memory = SliceMemory::new(MODULE_BASE);
        memory.map_object(MODULE_BASE + GAMESTATE_SINGLETON_RVA, 8);
        memory.put_u64(MODULE_BASE + GAMESTATE_SINGLETON_RVA, GAMESTATE);
        memory.map_object(GAMESTATE, 0x400);
        memory.map_object(FACTION_ALLIES, 0x600);
        memory.map_object(FACTION_AXIS, 0x600);
        memory.map_object(FACTION_COMINTERN, 0x600);
        memory.map_object(AXIS_HOLDER, 0x200);
        memory.map_object(COMINTERN_HEAP, 0x200);

        // The engine's packed identity word: id in the HI32, type in the LO32.
        // The producer re-reads it to authenticate the census it is handed.
        for (object, id) in [
            (FACTION_ALLIES, 2u64),
            (FACTION_COMINTERN, 7),
            (FACTION_AXIS, 11),
        ] {
            memory.put_u64(
                object + FACTION_PACKED_IDENTITY_OFFSET,
                (id << 32) | u64::from(FACTION_TYPE_EXPECTED),
            );
        }

        // Arm 1: an own name, inline.
        put_inline(&mut memory, FACTION_ALLIES + FACTION_OWN_TEXT_OFFSET, "Allies");
        // Arm 2: an empty own name and a fallback holder, inline.
        put_inline(&mut memory, FACTION_AXIS + FACTION_OWN_TEXT_OFFSET, "");
        memory.put_u64(FACTION_AXIS + FACTION_FALLBACK_HOLDER_OFFSET, AXIS_HOLDER);
        put_inline(&mut memory, AXIS_HOLDER + FALLBACK_HOLDER_TEXT_OFFSET, "Axis");
        // Arm 1 again, but long enough to be a heap string.
        put_heap(
            &mut memory,
            FACTION_COMINTERN + FACTION_OWN_TEXT_OFFSET,
            COMINTERN_HEAP,
            "The Third International",
        );
        memory
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("xmap17_faction_joint_certification.json")
}

/// Deliberately not in numeric order, so the producer's sort is exercised.
fn census() -> Vec<FactionCensusEntry> {
    vec![
        FactionCensusEntry {
            faction_id: "7".into(),
            faction_object: engine::FACTION_COMINTERN,
        },
        FactionCensusEntry {
            faction_id: "2".into(),
            faction_object: engine::FACTION_ALLIES,
        },
        FactionCensusEntry {
            faction_id: "11".into(),
            faction_object: engine::FACTION_AXIS,
        },
    ]
}

/// `BTreeMap` orders these lexically as 11, 2, 7 — a different order from the
/// numeric 2, 7, 11 the consumer sorts by. A producer that emitted map order
/// would pass a self-consistent round trip and fail the consumer.
fn roster() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([
        ("2".to_string(), vec!["ENG".to_string(), "USA".to_string()]),
        ("7".to_string(), vec!["SOV".to_string()]),
        ("11".to_string(), vec!["GER".to_string(), "ITA".to_string()]),
    ])
}

fn frame() -> FrameIdentity {
    FrameIdentity {
        frame_hour: 9,
        producer_epoch_sha256: "2".repeat(64),
        source_sequence: 7,
    }
}

fn produced_bundle() -> Value {
    let carrier = build_faction_labels_carrier(
        &engine::memory(),
        &census(),
        &roster(),
        &"a".repeat(64),
        &frame(),
    )
    .expect("the faction carrier builds from the fixture engine layout");
    json!({
        "_comment": "Lane HY joint-certification fixture. Producer half: extractor/native/lanes/map_passive_carriers_live/tests/joint_certification.rs. Consumer half: replay_interface/native/replay_core/src/xmap17_player_labels.rs. Regenerate with HY_WRITE_XMAP17_FIXTURE=1.",
        "frame_hour": frame().frame_hour,
        "producer_epoch_sha256": frame().producer_epoch_sha256,
        "source_sequence": frame().source_sequence,
        "domain53_snapshot_payload_sha256": "a".repeat(64),
        "faction_members": roster(),
        "_xmap17_faction_labels_exact": serde_json::to_value(&carrier).expect("faction carrier"),
    })
}

/// kills: pinning the carrier to an evidence identity with no evidence behind
/// it. Adversarial review round 3, finding 5, observed that the new manifest
/// hash appeared only in constants and prose — nothing in the worktree carried
/// the artifact it claims to name. This recomputes the hash from the committed
/// manifest, so the constant is a statement about a file that exists.
#[test]
fn the_faction_evidence_manifest_hash_is_the_committed_manifests_own_hash() {
    use sha2::{Digest, Sha256};
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runs/hy_map_producers/xmap17_faction_law_manifest.json");
    let bytes = std::fs::read(&manifest).unwrap_or_else(|error| {
        panic!(
            "the XMAP17 faction evidence manifest {} is missing ({error}); it is the artifact the \
             carrier's evidence_manifest_sha256 names",
            manifest.display()
        )
    });
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        map_passive_carriers_live_lane::xmap17::FACTION_EVIDENCE_MANIFEST_SHA256,
        "the pinned manifest hash is not the hash of the committed manifest"
    );
    let parsed: Value = serde_json::from_slice(&bytes).expect("the manifest is JSON");
    let text = parsed.to_string();
    assert!(
        text.contains("140d77360"),
        "the manifest must cover the function the producer implements"
    );
    assert!(
        text.contains("870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"),
        "the manifest must carry the pinned a729 binary identity"
    );
}

/// kills: any change to the mirrored carrier structs — a field added, renamed,
/// reordered or retyped — and any change to the record-hash inputs. Both sides
/// hash their own struct definitions, so such a change is invisible to every
/// other test and lethal at replay time.
#[test]
fn the_producer_emits_the_committed_joint_certification_bytes() {
    let rendered = serde_json::to_string_pretty(&produced_bundle()).expect("bundle renders") + "\n";
    let path = fixture_path();
    if std::env::var("HY_WRITE_XMAP17_FIXTURE").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().expect("fixtures dir")).expect("create fixtures dir");
        std::fs::write(&path, &rendered).expect("write fixture");
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "the joint-certification fixture {} is missing ({error}); regenerate it with \
             HY_WRITE_XMAP17_FIXTURE=1",
            path.display()
        )
    });
    // Compared with CR stripped: this repository checks out on Windows with
    // `core.autocrlf` active, so a byte-for-byte comparison would fail on the
    // line endings rather than on the contract, and a gate that fails for the
    // wrong reason gets disabled.
    assert_eq!(
        committed.replace('\r', ""),
        rendered.replace('\r', ""),
        "the producer's bytes drifted from the committed joint-certification fixture; the replay \
         consumer authenticates this carrier by re-hashing its own struct definitions, so this is \
         a contract break, not a formatting difference"
    );
}

/// kills: emitting rows in the roster's `BTreeMap` order instead of the
/// consumer's numeric order. This is the case a self-consistent round trip
/// cannot catch, because the producer would hash whatever it emitted.
#[test]
fn rows_follow_numeric_identity_order_not_map_order() {
    let carrier = build_faction_labels_carrier(
        &engine::memory(),
        &census(),
        &roster(),
        &"a".repeat(64),
        &frame(),
    )
    .unwrap();
    let ids: Vec<&str> = carrier
        .factions
        .iter()
        .map(|row| row.faction_id.as_str())
        .collect();
    assert_eq!(ids, ["2", "7", "11"]);
    let roster = roster();
    let map_order: Vec<&str> = roster.keys().map(String::as_str).collect();
    assert_eq!(
        map_order,
        ["11", "2", "7"],
        "the fixture must keep map order different from numeric order, or this test proves nothing"
    );
    assert_eq!(
        carrier.factions[0].engine_display_text, "Allies",
        "arm 1, inline storage"
    );
    assert_eq!(
        carrier.factions[1].engine_display_text, "The Third International",
        "arm 1, heap storage"
    );
    assert_eq!(
        carrier.factions[2].engine_display_text, "Axis",
        "arm 2, the fallback holder"
    );
}
