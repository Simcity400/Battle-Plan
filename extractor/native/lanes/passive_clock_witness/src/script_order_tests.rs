//! Source-order contracts for the two PowerShell scripts this crate judges.
//!
//! Several of the guards in the harness and the observer are ORDERING facts,
//! not value facts: a check that runs after the read it was meant to gate is
//! not a check, and a flag set after the statement that can throw past it is
//! not a flag. PowerShell has no test runner in this workspace, so those facts
//! would otherwise be unguarded — every one of them was introduced or corrected
//! during review precisely because nothing could catch it.
//!
//! These assert the order of exact, unique source markers. They fail if a
//! marker is missing, if it appears more than once (which would make the
//! ordering claim ambiguous), or if the order changes.
//!
//! # Line endings
//!
//! Every marker is matched against a normalised copy of the script. The
//! markers are written with `\n`, and git checks these files out with CRLF on
//! most Windows clones, so an un-normalised matcher finds zero occurrences of
//! any multi-line marker and the guard silently stops guarding. That is not a
//! hypothetical: it happened, and three of these tests failed in a CRLF
//! checkout while passing in an LF one. A test whose result depends on how git
//! checked the file out is not a test, so
//! `the_source_order_matcher_does_not_depend_on_how_git_checked_the_file_out`
//! runs every check against both renderings and requires identical verdicts.

use std::path::PathBuf;

const OBSERVER: &str = "Read-Hoi4GameState.ps1";
const HARNESS: &str = "Invoke-HeadlessRawCapture.ps1";

fn scripts_dir() -> PathBuf {
    // .../extractor/native/lanes/passive_clock_witness -> .../extractor/native/scripts
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|lanes| lanes.parent())
        .expect("the crate sits under extractor/native/lanes")
        .join("scripts")
}

/// The script exactly as it sits on disk, line endings and all. Normalisation
/// belongs to the checks, so the checks are what the agnosticism test can
/// exercise against both renderings.
fn read(name: &str) -> String {
    let path = scripts_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn to_lf(source: &str) -> String {
    source.replace("\r\n", "\n")
}

fn to_crlf(source: &str) -> String {
    to_lf(source).replace('\n', "\r\n")
}

/// Offset of the ONE occurrence of `marker`. A marker that moved, changed, or
/// was duplicated fails here rather than silently weakening the order claim.
fn only(source: &str, name: &str, marker: &str) -> Result<usize, String> {
    let count = source.matches(marker).count();
    if count != 1 {
        return Err(format!(
            "{name}: expected exactly one occurrence of {marker:?}, found {count}"
        ));
    }
    source.find(marker).ok_or_else(|| "counted above".to_string())
}

fn before(source: &str, name: &str, first: &str, second: &str, why: &str) -> Result<(), String> {
    let a = only(source, name, first)?;
    let b = only(source, name, second)?;
    if a >= b {
        return Err(format!(
            "{name}: {first:?} must come before {second:?} — {why}"
        ));
    }
    Ok(())
}

/// Every value a poll reads must sit behind the one check that admits it, and
/// the progress timer must stop with the loop it measures.
///
/// The list is exhaustive on purpose. Constraining only the first and last
/// reads leaves the ones between them free to move below the check and still be
/// promoted, which is the defect in miniature: a pause read after the budget
/// expired is engine state from outside the window the sample reports.
///
/// Moving ANY candidate read below the post-read check, promoting before it, or
/// moving `$progressStopwatch.Stop()` below the closing reads, makes this red.
fn check_poll_admission(raw: &str, name: &str) -> Result<(), String> {
    let source = to_lf(raw);
    const POST_READ_CHECK: &str =
        "if ($progressStopwatch.ElapsedMilliseconds -gt $ClockProgressBudgetMs) { break }";

    for candidate in [
        "$candidateHour = Read-RemoteInt32",
        "$candidateTickProgressBits = Read-RemoteInt32",
        "$candidatePaused = Read-RemoteByte",
        "$candidatePauseForced = Read-RemoteByte",
        "$candidateTickInhibit = Read-RemoteByte",
    ] {
        before(
            &source,
            name,
            candidate,
            POST_READ_CHECK,
            "every candidate read must be behind the one check that admits them",
        )?;
    }
    for promotion in [
        "$hourB = $candidateHour",
        "$tickProgressBitsB = $candidateTickProgressBits",
        "$observedPaused = $candidatePaused",
        "$observedPauseForced = $candidatePauseForced",
        "$observedTickInhibit = $candidateTickInhibit",
    ] {
        before(
            &source,
            name,
            POST_READ_CHECK,
            promotion,
            "values read outside the budget must never reach the sample",
        )?;
    }
    before(
        &source,
        name,
        "$progressStopwatch.Stop()",
        "$finalSpeedRaw = Read-RemoteInt32",
        "the closing reads are not part of the budgeted wait for the clock",
    )
}

/// The evidence bracket must enclose EVERY read that feeds the sample, and both
/// of its wall-clock endpoints must be read from the wall clock.
///
/// One exhaustive list, and every entry is placed between BOTH boundaries. The
/// earlier shape -- opening reads only after the starts, closing reads only
/// before the stop -- let an in-loop read move above the window start while
/// every assertion still passed, and a pause seen only before the window would
/// then have been promoted into `paused_in_window` and interrupted the stall
/// escalation on evidence from outside the window.
fn check_bracket_encloses_every_read(raw: &str, name: &str) -> Result<(), String> {
    let source = to_lf(raw);

    // Every read of engine memory whose value reaches the sample.
    const ENGINE_READS: [&str; 15] = [
        "$initialSpeedRaw = Read-RemoteInt32",
        "$initialPaused = Read-RemoteByte",
        "$initialPauseForced = Read-RemoteByte",
        "$initialTickInhibit = Read-RemoteByte",
        "$hourA = Read-RemoteInt32",
        "$tickProgressBitsA = Read-RemoteInt32",
        "$candidateHour = Read-RemoteInt32",
        "$candidateTickProgressBits = Read-RemoteInt32",
        "$candidatePaused = Read-RemoteByte",
        "$candidatePauseForced = Read-RemoteByte",
        "$candidateTickInhibit = Read-RemoteByte",
        "$finalSpeedRaw = Read-RemoteInt32",
        "$finalPaused = Read-RemoteByte",
        "$finalPauseForced = Read-RemoteByte",
        "$finalTickInhibit = Read-RemoteByte",
    ];
    for engine_read in ENGINE_READS {
        before(
            &source,
            name,
            "$windowStartedAt = [DateTimeOffset]::UtcNow",
            engine_read,
            "the wall-clock start is the bracket the sample carries, so it precedes every \
             read that feeds the sample",
        )?;
        before(
            &source,
            name,
            "$bracketStopwatch = [Diagnostics.Stopwatch]::StartNew()",
            engine_read,
            "a read taken before the bracket opens is attributed to a window that had not \
             started",
        )?;
        before(
            &source,
            name,
            engine_read,
            "$bracketStopwatch.Stop()",
            "a read that still feeds the sample is inside the bracket",
        )?;
    }

    // And the derived in-window updates, which are what classification reads.
    for update in [
        "if ($observedPaused -ne 0) { $pausedInWindow = $true }",
        "if ($observedPauseForced -ne 0) { $pauseForcedInWindow = $true }",
        "if ($observedTickInhibit -ne 0) { $tickInhibitedInWindow = $true }",
        "if ($finalPaused -ne 0) { $pausedInWindow = $true }",
        "if ($finalPauseForced -ne 0) { $pauseForcedInWindow = $true }",
        "if ($finalTickInhibit -ne 0) { $tickInhibitedInWindow = $true }",
    ] {
        before(
            &source,
            name,
            update,
            "$bracketStopwatch.Stop()",
            "the in-window flags are settled before the window they describe closes",
        )?;
    }

    before(
        &source,
        name,
        "$bracketStopwatch.Stop()",
        "$observedAt = [DateTimeOffset]::UtcNow",
        "the window end is read after the window closes",
    )?;
    if source.contains("$observedAt = $windowStartedAt.AddMilliseconds") {
        return Err(format!(
            "{name}: the window end must be read from the wall clock, not synthesized from \
             its start plus a monotonic duration — a scheduling gap between the two clocks \
             would report an observation as earlier than it happened"
        ));
    }
    Ok(())
}

/// The recorder-started flag must be set before anything that can throw past
/// it, and a graded window that never saw an injection must be closed before
/// the client-driving token is restored.
///
/// Moving `$collectorInjectionStarted = $true` below the budget setup, deleting
/// the no-injection closure, or moving it below the token restore, makes this
/// red.
fn check_no_stranded_graded_window(raw: &str, name: &str) -> Result<(), String> {
    let source = to_lf(raw);

    before(
        &source,
        name,
        "$recorderProcess = Start-Process -FilePath $collectorRecord",
        "$collectorInjectionStarted = $true",
        "the flag records that a recorder exists, so it cannot precede the process",
    )?;
    before(
        &source,
        name,
        "$collectorInjectionStarted = $true",
        "$recorderBudgetSeconds = $DurationSeconds + $RecorderOverheadSeconds",
        "a throw between the two would hide a live recorder from every cleanup decision",
    )?;
    before(
        &source,
        name,
        "if ($gradedWindowStarted -and -not $collectorInjectionStarted) {\n        $gradedWindowStarted = $false",
        "'BATTLE_PLAN_CLIENT_DRIVING_OPT_IN',\n            $clientDrivingOptInSuspended,",
        "the branch has to CLOSE the window, not merely notice it: closing is what \
         allows the token restore that follows",
    )
}

/// The client-driving authority is withdrawn before the graded boundary opens,
/// and given back only inside cleanup.
fn check_authority_withdrawn_before_the_boundary(raw: &str, name: &str) -> Result<(), String> {
    let source = to_lf(raw);

    before(
        &source,
        name,
        "$clientDrivingOptInSuspended = [Environment]::GetEnvironmentVariable(",
        "$gradedWindowStarted = $true",
        "the token must already be read when the graded window opens",
    )?;
    // The CLEAR itself, which is the statement that withdraws the authority.
    // Reading the old value and asserting afterwards proves nothing if the
    // clear moved past the boundary.
    let cleared = concat!(
        "[Environment]::SetEnvironmentVariable(\n",
        "    'BATTLE_PLAN_CLIENT_DRIVING_OPT_IN',\n",
        "    $null,\n",
        "    [EnvironmentVariableTarget]::Process\n",
        ")"
    );
    before(
        &source,
        name,
        cleared,
        "if ($null -ne [Environment]::GetEnvironmentVariable(",
        "the token is cleared and only then verified",
    )?;
    before(
        &source,
        name,
        cleared,
        "$gradedWindowStarted = $true",
        "the authority must be gone before the graded window opens, not after",
    )?;
    before(
        &source,
        name,
        "throw 'The graded window cannot open while the client-driving opt-in token is still set.'",
        "$gradedWindowStarted = $true",
        "a token that could not be cleared must stop the run rather than open the window",
    )
}

/// The sample is written to the witness file before any verdict is asked for.
///
/// The original harness threw before it wrote, so the observation that ended a
/// run could never be persisted and two incorrect diagnoses of the incident
/// were made from the surviving successes. Moving `Assert-PassiveWitnessVerdict`
/// above the `Add-Content` restores exactly that.
fn check_records_before_asking_for_a_verdict(raw: &str, name: &str) -> Result<(), String> {
    let source = to_lf(raw);

    before(
        &source,
        name,
        "Add-Content -LiteralPath $passiveWitnessPath -Encoding UTF8",
        "\n    Assert-PassiveWitnessVerdict\n",
        "the sample that ends a run is the one that must survive on disk",
    )
}

type Check = fn(&str, &str) -> Result<(), String>;

const OBSERVER_CHECKS: [(&str, Check); 2] = [
    ("poll admission", check_poll_admission),
    ("bracket encloses every read", check_bracket_encloses_every_read),
];

const HARNESS_CHECKS: [(&str, Check); 3] = [
    ("no stranded graded window", check_no_stranded_graded_window),
    (
        "authority withdrawn before the boundary",
        check_authority_withdrawn_before_the_boundary,
    ),
    (
        "records before asking for a verdict",
        check_records_before_asking_for_a_verdict,
    ),
];

#[test]
fn the_observer_admits_a_poll_only_after_re_checking_the_budget() {
    check_poll_admission(&read(OBSERVER), OBSERVER).expect("poll admission order");
}

#[test]
fn the_observer_brackets_every_read_that_feeds_the_sample() {
    check_bracket_encloses_every_read(&read(OBSERVER), OBSERVER).expect("bracket order");
}

#[test]
fn the_harness_cannot_strand_a_graded_window_that_never_saw_an_injection() {
    check_no_stranded_graded_window(&read(HARNESS), HARNESS).expect("graded window order");
}

#[test]
fn the_harness_withdraws_client_driving_authority_before_the_graded_boundary() {
    check_authority_withdrawn_before_the_boundary(&read(HARNESS), HARNESS)
        .expect("authority withdrawal order");
}

#[test]
fn the_harness_records_a_sample_before_it_asks_for_a_verdict() {
    check_records_before_asking_for_a_verdict(&read(HARNESS), HARNESS).expect("record-then-judge");
}

/// A test whose result depends on how git checked the file out is not a test.
///
/// These markers are written with `\n`, and git checks these scripts out with
/// CRLF on most Windows clones. An un-normalised matcher then finds zero
/// occurrences of every multi-line marker and the guard silently stops
/// guarding — which is exactly what happened: three of the checks above passed
/// in an LF worktree and failed in a CRLF one, with the production scripts
/// byte-identical apart from line endings.
///
/// Every check is run against both renderings of the same script, on a healthy
/// source AND on a deliberately broken one, and the verdicts must be identical.
/// Requiring only that both PASS would be satisfied by a matcher that had
/// stopped matching altogether, so the failure path is compared too: that is
/// the path the CRLF checkout actually took.
///
/// Deleting a `to_lf` call from any check makes this red.
#[test]
fn the_source_order_matcher_does_not_depend_on_how_git_checked_the_file_out() {
    for (name, checks, break_this) in [
        (
            OBSERVER,
            &OBSERVER_CHECKS[..],
            "$bracketStopwatch.Stop()",
        ),
        (
            HARNESS,
            &HARNESS_CHECKS[..],
            "$collectorInjectionStarted = $true",
        ),
    ] {
        let raw = read(name);
        let lf = to_lf(&raw);
        let crlf = to_crlf(&raw);

        // The two renderings must actually differ, or this proves nothing.
        assert_ne!(lf, crlf, "{name}: the two renderings are identical");
        assert!(crlf.contains("\r\n"), "{name}: the CRLF rendering has no CRLF");
        assert!(!lf.contains('\r'), "{name}: the LF rendering still has CR");

        // Healthy source: identical verdicts, and both Ok.
        for (label, check) in checks {
            let lf_verdict = check(&lf, name);
            let crlf_verdict = check(&crlf, name);
            assert_eq!(
                lf_verdict, crlf_verdict,
                "{name}/{label}: the verdict changed with the line endings"
            );
            assert!(
                lf_verdict.is_ok(),
                "{name}/{label}: {}",
                lf_verdict.unwrap_err()
            );
        }

        // Broken source: identical verdicts, and at least one check must fail.
        // A matcher that silently matches nothing would return the same error
        // for both renderings here as well, so the emptiness assertions above
        // and the Ok assertions on the healthy source are what exclude it.
        let broken_lf = lf.replace(break_this, "");
        assert_ne!(broken_lf, lf, "{name}: the break fixture changed nothing");
        let broken_crlf = to_crlf(&broken_lf);
        let mut any_failed = false;
        for (label, check) in checks {
            let lf_verdict = check(&broken_lf, name);
            let crlf_verdict = check(&broken_crlf, name);
            assert_eq!(
                lf_verdict, crlf_verdict,
                "{name}/{label}: the FAILURE verdict changed with the line endings"
            );
            any_failed |= lf_verdict.is_err();
        }
        assert!(
            any_failed,
            "{name}: removing {break_this:?} must break at least one check, or the failure \
             path was never compared"
        );
    }
}
