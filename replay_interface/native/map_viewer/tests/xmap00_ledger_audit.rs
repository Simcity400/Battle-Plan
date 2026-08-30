//! The XMAP00 ledger audit, run where it can actually fail a landing.
//!
//! The audit is the instrument that holds every ledger column to the code it
//! describes: consumer and evidence anchors bounded to real declarations, the
//! owner-proven admission table transcribed column for column, the carrier
//! classification derived from the render-stream structs, the effective-VFS
//! paths derived from the resolver calls that fetch them. None of that was
//! checked by anything that ran. It lived in a standalone PowerShell script no
//! gate invoked, and it had stopped completing: at this lane's start it aborted
//! on the first of five drifted rows, so every check after that one was dead
//! text.
//!
//! Running it from a test is what makes the ledger's columns claims instead of
//! prose. The audit stays the single implementation - a Rust re-implementation
//! of the same rules would be a second authority to keep in step.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const AUDIT_SCRIPT: &str =
    "replay_interface/re_evidence/rendering_laws/XMAP00/Audit-NonUiVisualInputLedger.ps1";

const LEDGER_TSV: &str = include_str!(
    "../../../re_evidence/rendering_laws/XMAP00/non_ui_visual_input_ledger_20260730.tsv"
);

/// The line the audit prints only after every check has passed.
const OK_MARKER: &str = "XMAP00_NON_UI_VISUAL_INPUT_LEDGER_OK";

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("repository root")
        .to_path_buf()
}

/// The ledger's own class census, read here rather than taken from the audit.
fn ledger_class_counts() -> (usize, BTreeMap<String, usize>) {
    let mut lines = LEDGER_TSV.lines();
    let header: Vec<&str> = lines.next().expect("ledger header").split('\t').collect();
    let class_index = header
        .iter()
        .position(|column| *column == "class")
        .expect("ledger has a class column");
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut rows = 0usize;
    for line in lines.filter(|line| !line.trim().is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), header.len(), "ragged ledger row: {line}");
        rows += 1;
        *counts.entry(fields[class_index].to_string()).or_default() += 1;
    }
    (rows, counts)
}

/// The audit completes, and it completed over THIS ledger.
///
/// Two halves, because either alone is weak. Exit zero plus the marker line is
/// the audit's own verdict; re-deriving the class census here and requiring the
/// audit's printed counts to match it is what rules out a pass that read an
/// empty or truncated table. The audit reports its counts for exactly this
/// reason, and an audit that silently stopped parsing rows would report a
/// smaller census than the file holds.
///
/// kills: letting a ledger column drift from the code it describes; editing the
/// audit into a pass; and a green run that inspected nothing.
#[test]
fn the_xmap00_ledger_audit_passes_over_this_ledger() {
    let root = repository_root();
    let script = root.join(AUDIT_SCRIPT);
    assert!(
        script.is_file(),
        "the XMAP00 ledger audit is missing at {}",
        script.display()
    );

    // A console-subsystem child; CREATE_NO_WINDOW keeps it from allocating a
    // visible terminal window on console-less harnesses, which is the lane QI
    // defect class `portable_test_fs_retry`'s scanner exists to hold closed.
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .arg("-RepositoryRoot")
        .arg(&root)
        .current_dir(&root)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .unwrap_or_else(|error| {
            panic!("the XMAP00 ledger audit could not be run: {error}");
        });
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "the XMAP00 ledger audit failed ({}):\n{stdout}\n{stderr}",
        output.status
    );

    let verdict = stdout
        .lines()
        .find(|line| line.contains(OK_MARKER))
        .unwrap_or_else(|| {
            panic!("the audit exited zero without its verdict line:\n{stdout}\n{stderr}")
        });

    let reported: BTreeMap<&str, usize> = verdict
        .split_whitespace()
        .filter_map(|field| field.split_once('='))
        .map(|(name, value)| {
            (
                name,
                value
                    .parse::<usize>()
                    .unwrap_or_else(|error| panic!("audit census field {name}: {error}")),
            )
        })
        .collect();
    assert!(
        !reported.is_empty(),
        "the audit verdict carries no census: {verdict}"
    );

    let (rows, counts) = ledger_class_counts();
    assert_eq!(
        reported.get("rows").copied(),
        Some(rows),
        "the audit counted a different number of ledger rows than this file holds: {verdict}"
    );
    for (class, count) in &counts {
        assert_eq!(
            reported.get(class.as_str()).copied(),
            Some(*count),
            "the audit's {class} census disagrees with the ledger: {verdict}"
        );
    }
    // Every class the audit reports is one the ledger has, so a census field
    // cannot be invented either.
    for name in reported.keys() {
        assert!(
            *name == "rows" || counts.contains_key(*name),
            "the audit reports a class the ledger does not carry: {name}"
        );
    }
}
