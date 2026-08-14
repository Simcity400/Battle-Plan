//! Run the repo-wide citation gate where comments are edited.
//!
//! `extractor/native/scripts/verify_citation_integrity.py` is the authority on
//! whether a `<file>:<line>` or `<file>::<item>` reference written anywhere in
//! the source or document trees still resolves. It exists because the
//! world_fold gate covers exactly one cited file: lane HK measured 1,143 further
//! citations in the wire modules alone, 307 of which resolved to nothing and 252
//! of which named `lib.rs` when 71 files carry that name.
//!
//! Running it only from `Invoke-CentralRustValidation.ps1` would repeat the
//! mistake the world_fold gate already paid for — an edit stays silent for the
//! whole lane that makes it and surfaces several merges later. This target is
//! the same script the central runner invokes, so there is one authority and no
//! second implementation to drift.
//!
//! WHY THIS CRATE. The check is repo-wide and belongs to no crate in
//! particular; it lives beside `world_fold_citation_authority.rs` so the two
//! citation gates are found together. Nothing here is specific to
//! `recorder_host`.
//!
//! WHY THIS RUN IS THE INCREMENTAL ONE. Lane JD measured this single test at
//! 70.8 s -- 66% of the time the Extractor serial half holds the machine-wide
//! mutex, because a half's wall clock is its slowest test and not its count.
//! Lane JN found the whole of that cost was three regular expressions being
//! re-run over 98 MB of files that had not changed, and gave the verifier a
//! cache: a file whose bytes, whose allowance, whose script and whose every
//! question about the rest of the tree are all unchanged reports the result it
//! reported before. Nothing here needs to know that. The command is the same
//! command, the report it demands below is the same report, and a cache that
//! cannot be trusted -- absent, damaged, written under another allowance --
//! simply verifies everything and takes the old 70 s.
//!
//! The cache-free run still happens every battery: `Invoke-CentralRustValidation.ps1`
//! invokes this same script with `--full` before cargo starts, out where the
//! mutex is not held. So a landing never rests on the cache, and this test --
//! the one that costs the machine -- finds it warm.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("recorder_host manifest dir has a repository root")
        .to_path_buf()
}

/// Locate the interpreter the repository already requires. A missing
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
        "no Python interpreter on PATH (tried {CANDIDATES:?}); the citation \
         integrity gate cannot run, and this repository already requires Python \
         for central validation"
    );
}

#[test]
fn source_citations_resolve_or_are_grandfathered_with_a_reason() {
    let repo = repo_root();
    let script = repo.join("extractor/native/scripts/verify_citation_integrity.py");
    assert!(
        script.is_file(),
        "citation integrity verifier is missing: {}",
        script.display()
    );
    let allowance = repo.join("extractor/native/scripts/citation_integrity_allowance.json");
    assert!(
        allowance.is_file(),
        "the citation allowance is missing: {}. It is the ratchet's recorded \
         starting position; without it the gate cannot tell an old citation \
         from a new one.",
        allowance.display()
    );

    let output = Command::new(python())
        .arg(&script)
        .arg("--repo-root")
        .arg(&repo)
        .output()
        .expect("run the citation integrity verifier");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "the citation integrity gate failed. Read the verifier's own message \
         below; it names every citation and says which rule fired.\n\
         * `is a new free-text line number` means a comment or document gained \
         a `file.rs:<line>` reference. Line numbers drift on the next insertion \
         into the cited file and nothing hashes them. Cite the item instead: \
         `<file>::<item_name>`.\n\
         * `is ambiguous` means the basename is carried by several files. Write \
         enough of the path to name one.\n\
         * `past the end` / `names no file` means a grandfathered citation \
         stopped resolving. Find what it meant (`git log -S`) and re-point it at \
         the item, or correct the sentence.\n\
         * `none of [..] defines` means an item citation's item was renamed or \
         deleted.\n\
         Never widen the allowance to silence a citation you can repair.\n\
         --- verifier stdout ---\n{}\n--- verifier stderr ---\n{}",
        stdout.trim(),
        String::from_utf8_lossy(&output.stderr).trim(),
    );

    // A zero exit alone is not proof the gate did any work: it would also
    // follow from the scan roots being renamed, which leaves every citation
    // unchecked and the run still green. Demand the exact success line, and
    // demand the file count is in the range the shipped tree produces. The
    // world_fold gate learned this the same way — a count nobody compares
    // against anything is not a check.
    let reported = stdout.trim();
    assert!(
        !reported.contains('\n') && reported.starts_with("citation integrity check: PASS ("),
        "the verifier exited zero without reporting a single-line PASS: {reported:?}"
    );
    let scanned: usize = reported
        .split_once("PASS (")
        .and_then(|(_, tail)| tail.split_once(" files scanned"))
        .and_then(|(count, _)| count.parse().ok())
        .unwrap_or_else(|| panic!("no scanned-file count in {reported:?}"));
    assert!(
        scanned >= 800,
        "the gate scanned only {scanned} files, far below the shipped tree, so \
         it was checking something that is not this repository. Did a scan root \
         move? Reported: {reported:?}"
    );
    assert!(
        reported.contains(" line citations grandfathered, ")
            && reported.contains(" item citations resolved, ")
            && reported.ends_with(" unused allowance entries)"),
        "the verifier's report lost one of its three counts, so a rule could be \
         short-circuited without this test noticing: {reported:?}"
    );
}
