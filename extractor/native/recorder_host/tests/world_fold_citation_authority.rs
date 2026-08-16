//! Run the world-fold citation authority gate where world_fold.rs is edited.
//!
//! `extractor/native/scripts/verify_world_fold_citations.py` is the authority on
//! whether the dNN wire modules still cite the shipping fold truthfully.  It ran
//! in exactly one place: `Invoke-CentralRustValidation.ps1`, before Cargo.  An
//! edit to `world_fold.rs` that inserted lines therefore stayed silent for the
//! whole lane that made it, and only surfaced when someone ran central
//! validation — by which point the shift was several merges deep.  That is not
//! hypothetical: commit aa0f0f05 inserted 25 net lines and left 32 of the 45 dNN
//! markers pointing at the wrong range.
//!
//! The marker also pins the cited functions' CALLEE CLOSURE — themselves plus
//! every function in `world_fold.rs` they can reach — because an envelope digest
//! covers a function where it stands and nothing else.  Lifting a helper out of
//! a cited fold narrowed the authority to the residual and the gate went on
//! saying PASS while the moved rendering could be changed freely; d20's document
//! build is where that happened.  The count reported below is how many functions
//! are inside some domain's closure, and this test demands the clause so the
//! coverage cannot be dropped from the report without failing here.
//!
//! The same script also carries the LEAF SOURCE AUTHORITY. A closure follows
//! calls inside one file, so when a shared walk moves into its own crate the
//! closure shrinks and the digest stops covering it: d13's went from 80 names
//! to 73 the day `d13_grammar` took its structural walk, after which a changed
//! section length or flag mask inside that crate was invisible to every marker
//! in the tree. A `@leaf-source-authority` marker at each site that walks or
//! writes the leaf pins the crate's source, so editing the shared grammar
//! without re-attesting at each consumer fails here — which is exactly what
//! editing a cited fold already did.
//!
//! The same script also carries the free-text line-number ratchet: outside the
//! one gated `lines=`/sha256 range, a dNN module may not cite `world_fold.rs:N`
//! at all unless that exact reference is already grandfathered in
//! `world_fold_prose_reference_allowance.json`.  Free-text line numbers are what
//! drifted here in the first place, and nothing hashes them.
//!
//! This target closes that gap at the cheapest point.  `world_fold.rs` belongs to
//! `recorder_host`, so any lane that edits it and runs this crate's tests — or
//! the workspace suite — now sees the drift immediately, in the same run it was
//! already doing.  The check is the identical script the central runner uses, so
//! there is one authority and no second implementation to drift.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("recorder_host manifest dir has a repository root")
        .to_path_buf()
}

/// Locate the interpreter the repository already requires.  A missing
/// interpreter fails the test rather than skipping it: a citation gate that
/// silently reports success when it did not run is worse than no gate.
fn python() -> &'static str {
    const CANDIDATES: [&str; 3] = ["python", "python3", "py"];
    for candidate in CANDIDATES {
        let probe = Command::new(candidate).arg("--version").output();
        if matches!(probe, Ok(ref output) if output.status.success()) {
            return candidate;
        }
    }
    panic!(
        "no Python interpreter on PATH (tried {CANDIDATES:?}); the world-fold \
         citation gate cannot run, and this repository already requires Python \
         for central validation"
    );
}

#[test]
fn world_fold_citations_match_the_shipping_fold() {
    let repo = repo_root();
    let script = repo.join("extractor/native/scripts/verify_world_fold_citations.py");
    assert!(
        script.is_file(),
        "citation verifier is missing: {}",
        script.display()
    );
    // A silent pass would also follow from the verifier finding no modules at
    // all, so pin that the tree it checks is really there.
    let wire_src = repo.join("extractor/native/wire_format/src");
    let modules = std::fs::read_dir(&wire_src)
        .expect("wire_format/src is readable")
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with('d')
                && name.ends_with(".rs")
                && name[1..name.len() - 3].chars().all(|c| c.is_ascii_digit())
                && name.len() > 4
        })
        .count();
    assert!(
        modules >= 40,
        "found only {modules} dNN modules under {} — far below the shipped set, \
         so the gate would be checking a tree that is not the wire format",
        wire_src.display()
    );

    let output = Command::new(python())
        .arg(&script)
        .arg("--repo-root")
        .arg(&repo)
        .output()
        .expect("run the world-fold citation verifier");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "the world-fold citation gate failed. Read the verifier's own message \
         below first; it says which of the two checks fired.\n\
         * A marker/digest failure means the dNN markers no longer describe \
         `{}`. Re-derive the ranges: a marker whose recorded sha256 still \
         matches the bytes at its new range is a pure line shift and may be \
         re-cited; a changed digest means the shipping fold changed and every \
         transcription it backs needs review. Never edit a digest to match new \
         content.\n\
         * A `free-text line number` failure means a comment -- or a document \
         under `extractor/documents` -- gained a `world_fold.rs:<line>` \
         reference outside the one gated range. Nothing hashes those and they \
         drift on the next insertion. Cite the item instead: \
         `world_fold.rs::<item_name>`.\n\
         --- verifier stdout ---\n{}\n--- verifier stderr ---\n{}",
        Path::new("extractor/native/recorder_host/src/world_fold.rs").display(),
        stdout.trim(),
        String::from_utf8_lossy(&output.stderr).trim(),
    );

    // A zero exit alone is not proof the gate did any work: the verifier only
    // requires a non-empty module set, so a module could stop being scanned and
    // still leave a successful exit behind.  Demand the exact success line, and
    // demand that the count in it is the count of modules on disk.  A loose
    // substring search would accept a malformed line that merely contains the
    // right number somewhere, so the whole line is matched, not a fragment.
    //
    // The trailing clauses are the two free-text line-number ratchets -- the
    // modules' and the design documents'.  Requiring both here is what stops
    // either from being dropped or short-circuited without this test noticing;
    // the document one is the newer and the easier to lose, because unlike the
    // module check nothing else in the tree reads a document.
    let reported = stdout.trim();
    let expected_prefix = format!(
        "world-fold citation authority check: PASS ({modules} dNN modules, "
    );
    assert!(
        !reported.contains('\n')
            && reported.starts_with(&expected_prefix)
            && reported.contains(" lane authorities, ")
            && reported.contains(" items inside an authority closure, ")
            // The leaf clause, demanded for the same reason as the closure one.
            // A shared grammar crate is coverage that left this file entirely:
            // d13's closure shrank from 80 names to 73 when `d13_grammar` took
            // its structural walk, and the leaf markers are what replaced it.
            // Requiring the clause here is what stops that half being dropped
            // from the report — and the count is asserted to be non-zero below,
            // because "0 leaf source authorities" is exactly what a registry
            // someone emptied would print.
            && reported.contains(" leaf source authorities over ")
            && reported.contains(" grandfathered prose line references, ")
            && reported.ends_with(" grandfathered document line references)"),
        "the verifier exited zero but did not report checking the {modules} dNN \
         modules under {}. Expected a single line `{expected_prefix}<n> lane \
         authorities, <n> items inside an authority closure, <n> leaf source \
         authorities over <n> consumer sites, <n> grandfathered prose line \
         references, <n> grandfathered document line references)`, got: \
         {reported:?}",
        wire_src.display()
    );

    // A clause is not coverage. `0 leaf source authorities over 0 consumer
    // sites` satisfies every check above and is what an emptied
    // `REQUIRED_LEAF_AUTHORITIES` prints — the mechanism removed, the gate
    // still green. The count is read out of the line it just matched.
    let leaves: u32 = reported
        .split(", ")
        .find(|clause| clause.contains("leaf source authorities over "))
        .and_then(|clause| clause.split_whitespace().next())
        .and_then(|count| count.parse().ok())
        .expect("the leaf clause reports a count");
    assert!(
        leaves >= 1,
        "the verifier reported {leaves} leaf source authorities. At least one \
         shared grammar crate (`d13_grammar`) is attested by five consumer \
         sites in this tree; zero means the registry was emptied and the leaf \
         half of the gate now verifies nothing. Line: {reported:?}"
    );

    // And the count is not the claim either: once a second leaf exists, d13
    // could be dropped from the registry while the number stays non-zero. The
    // five sites are named here, in Rust, so removing one from the Python
    // registry fails this test rather than quietly reducing coverage. This is
    // the only check in this file that does not go through the script.
    const D13_CONSUMERS: [&str; 5] = [
        "extractor/native/collector/src/attach_log.rs",
        "extractor/native/collector/src/attach_log/domain13_tech_delta_tests.rs",
        "extractor/native/recorder_host/src/domain_data_authority.rs",
        "extractor/native/recorder_host/src/world_fold.rs",
        "extractor/native/wire_format/src/d13.rs",
    ];
    // d43's three sites, named for the same reason. The fourth d43 site is the
    // lane gate (`extractor/native/lanes/market_trade_live`), which does not
    // take the leaf yet and is therefore not listed: this array states what is
    // attested, not what is planned.
    const D43_CONSUMERS: [&str; 3] = [
        "extractor/native/collector/src/attach_log.rs",
        "extractor/native/recorder_host/src/domain_data_authority.rs",
        "extractor/native/recorder_host/src/world_fold.rs",
    ];
    for (crate_name, consumers) in [
        ("d13_grammar", &D13_CONSUMERS[..]),
        ("d43_grammar", &D43_CONSUMERS[..]),
    ] {
        let prefix = format!("// @leaf-source-authority crate={crate_name} ");
        for consumer in consumers {
            let path = repo.join(consumer);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            let attestations = source
                .lines()
                .filter(|line| line.trim_start().starts_with(&prefix))
                .count();
            assert_eq!(
                attestations,
                1,
                "{consumer} carries {attestations} `{crate_name}` leaf authority \
                 markers, expected exactly one. Every site that walks or writes \
                 that shared grammar attests to its source; a site that stops \
                 attesting is a site that can be left behind by a change to the \
                 leaf, which is how D13 lost 59 of 59 records of a live capture \
                 before the crate existed."
            );
        }
    }
}
