//! Every `dNN.rs` wire module must name the domain id its filename claims, and
//! that id must be assigned in the contract's descriptor table.
//!
//! This exists because of a real collision. Two lanes branched from the same
//! seed independently allocated domain 98; neither could see the other, and
//! `ASSIGNED_COMPACT_DOMAINS`'s duplicate-id assertion only fires once both
//! tables are in one tree, which is at merge. Renaming a lane's domain out of
//! the collision then touches four places — the descriptor, the JSON mirror, the
//! module filename and the module's own `DOMAIN` constant — and nothing checked
//! that the last two agreed with the first two. A `d99.rs` still declaring
//! `DOMAIN = 98`, or a module whose id no descriptor assigns, decodes records
//! under a domain the registry gives to a different name.
//!
//! Scope: this is an ASSIGNMENT gate, not a schema or naming-intent gate.
//!
//! It deliberately does not check that the module's `schema()` declares the same
//! id, because `schema()` reads the same constant this test already pins.
//!
//! It also does not check that a module is attached to the *semantically
//! intended* domain name, and no version of that check belongs here: looking a
//! descriptor up by id and then looking its own `canonical_name` back up returns
//! the same descriptor by construction, so such an assertion reads as a
//! safeguard while proving nothing. Real name-intent enforcement would need an
//! independent id-to-name authority, which would duplicate the registry and
//! drift from it. What this test does establish is narrow and true: filename id,
//! declared `DOMAIN` constant, and an assigned descriptor all agree.

use std::fs;
use std::path::PathBuf;

#[test]
fn every_wire_module_domain_id_matches_its_filename_and_is_assigned() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = Vec::new();

    for entry in fs::read_dir(&src).expect("wire_format src must be readable") {
        let path = entry.expect("src entry must be readable").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("source file stem");
        // `dNN` only: `derived.rs` also starts with `d`, and the shared modules
        // (`body`, `schema`, …) carry no domain id at all.
        let Some(digits) = stem.strip_prefix('d').filter(|rest| {
            !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
        }) else {
            continue;
        };
        let filename_id: u32 = digits.parse().expect("digit run parses");

        let source = fs::read_to_string(&path).expect("wire module must be readable");
        let declared = source
            .lines()
            .find_map(|line| {
                line.strip_prefix("pub const DOMAIN: u32 = ")
                    .and_then(|rest| rest.strip_suffix(';'))
                    .and_then(|value| value.trim().parse::<u32>().ok())
            })
            .unwrap_or_else(|| {
                panic!("{stem}.rs declares no `pub const DOMAIN: u32 = N;`")
            });

        assert_eq!(
            declared, filename_id,
            "{stem}.rs declares DOMAIN {declared} but its filename claims {filename_id}"
        );

        let id = u16::try_from(filename_id)
            .unwrap_or_else(|_| panic!("{stem}.rs domain id {filename_id} is not a domain id"));
        let descriptor = compact_contract::assigned_descriptor_by_id(id).unwrap_or_else(|| {
            panic!(
                "{stem}.rs claims domain {id}, which ASSIGNED_COMPACT_DOMAINS does not assign"
            )
        });
        checked.push((id, descriptor.canonical_name));
    }

    // A silent zero here would make the whole test vacuous: if the glob stopped
    // matching, every assertion above would simply never run.
    assert!(
        checked.len() >= 40,
        "expected the full dNN module set, found only {} — the scan is not matching",
        checked.len()
    );
    // d99 `division_combat_stats_live` deliberately has NO wire module yet: that
    // boundary is gated on a dispatched `fold_*` authority by
    // `scripts/verify_world_fold_citations.py`, and it has no producer. It joins
    // this scan when the fold lands.
    assert!(
        !checked.iter().any(|(id, _)| *id == 99),
        "d99 gained a wire module — it now needs a world-fold authority marker too"
    );
}
