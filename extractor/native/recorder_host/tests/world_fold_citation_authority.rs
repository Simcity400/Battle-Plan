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
            && reported.contains(" grandfathered prose line references, ")
            && reported.ends_with(" grandfathered document line references)"),
        "the verifier exited zero but did not report checking the {modules} dNN \
         modules under {}. Expected a single line `{expected_prefix}<n> lane \
         authorities, <n> items inside an authority closure, <n> \
         grandfathered prose line references, <n> grandfathered document line \
         references)`, got: {reported:?}",
        wire_src.display()
    );
}
