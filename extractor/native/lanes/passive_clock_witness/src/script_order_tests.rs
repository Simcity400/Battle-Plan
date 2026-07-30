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

use std::path::PathBuf;

fn scripts_dir() -> PathBuf {
    // .../extractor/native/lanes/passive_clock_witness -> .../extractor/native/scripts
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|lanes| lanes.parent())
        .expect("the crate sits under extractor/native/lanes")
        .join("scripts")
}

fn read(name: &str) -> String {
    let path = scripts_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// Offset of the ONE occurrence of `marker`. A marker that moved, changed, or
/// was duplicated fails here rather than silently weakening the order claim.
fn only(source: &str, name: &str, marker: &str) -> usize {
    let count = source.matches(marker).count();
    assert_eq!(
        count, 1,
        "{name}: expected exactly one occurrence of {marker:?}, found {count}"
    );
    source.find(marker).expect("counted above")
}

fn assert_before(source: &str, name: &str, first: &str, second: &str, why: &str) {
    let a = only(source, name, first);
    let b = only(source, name, second);
    assert!(a < b, "{name}: {first:?} must come before {second:?} — {why}");
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
#[test]
fn the_observer_admits_a_poll_only_after_re_checking_the_budget() {
    let name = "Read-Hoi4GameState.ps1";
    let source = read(name);
    const POST_READ_CHECK: &str =
        "if ($progressStopwatch.ElapsedMilliseconds -gt $ClockProgressBudgetMs) { break }";

    for candidate in [
        "$candidateHour = Read-RemoteInt32",
        "$candidateTickProgressBits = Read-RemoteInt32",
        "$candidatePaused = Read-RemoteByte",
        "$candidatePauseForced = Read-RemoteByte",
        "$candidateTickInhibit = Read-RemoteByte",
    ] {
        assert_before(
            &source,
            name,
            candidate,
            POST_READ_CHECK,
            "every candidate read must be behind the one check that admits them",
        );
    }
    for promotion in [
        "$hourB = $candidateHour",
        "$tickProgressBitsB = $candidateTickProgressBits",
        "$observedPaused = $candidatePaused",
        "$observedPauseForced = $candidatePauseForced",
        "$observedTickInhibit = $candidateTickInhibit",
    ] {
        assert_before(
            &source,
            name,
            POST_READ_CHECK,
            promotion,
            "values read outside the budget must never reach the sample",
        );
    }
    assert_before(
        &source,
        name,
        "$progressStopwatch.Stop()",
        "$finalSpeedRaw = Read-RemoteInt32",
        "the closing reads are not part of the budgeted wait for the clock",
    );
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
#[test]
fn the_observer_brackets_every_read_that_feeds_the_sample() {
    let name = "Read-Hoi4GameState.ps1";
    let source = read(name);

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
        assert_before(
            &source,
            name,
            "$windowStartedAt = [DateTimeOffset]::UtcNow",
            engine_read,
            "the wall-clock start is the bracket the sample carries, so it precedes every \
             read that feeds the sample",
        );
        assert_before(
            &source,
            name,
            "$bracketStopwatch = [Diagnostics.Stopwatch]::StartNew()",
            engine_read,
            "a read taken before the bracket opens is attributed to a window that had not \
             started",
        );
        assert_before(
            &source,
            name,
            engine_read,
            "$bracketStopwatch.Stop()",
            "a read that still feeds the sample is inside the bracket",
        );
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
        assert_before(
            &source,
            name,
            update,
            "$bracketStopwatch.Stop()",
            "the in-window flags are settled before the window they describe closes",
        );
    }

    assert_before(
        &source,
        name,
        "$bracketStopwatch.Stop()",
        "$observedAt = [DateTimeOffset]::UtcNow",
        "the window end is read after the window closes",
    );
    assert!(
        !source.contains("$observedAt = $windowStartedAt.AddMilliseconds"),
        "{name}: the window end must be read from the wall clock, not synthesized from its \
         start plus a monotonic duration — a scheduling gap between the two clocks would \
         report an observation as earlier than it happened"
    );
}

/// The recorder-started flag must be set before anything that can throw past
/// it, and a graded window that never saw an injection must be closed before
/// the client-driving token is restored.
///
/// Moving `$collectorInjectionStarted = $true` below the budget setup, deleting
/// the no-injection closure, or moving it below the token restore, makes this
/// red.
#[test]
fn the_harness_cannot_strand_a_graded_window_that_never_saw_an_injection() {
    let name = "Invoke-HeadlessRawCapture.ps1";
    let source = read(name);

    assert_before(
        &source,
        name,
        "$recorderProcess = Start-Process -FilePath $collectorRecord",
        "$collectorInjectionStarted = $true",
        "the flag records that a recorder exists, so it cannot precede the process",
    );
    assert_before(
        &source,
        name,
        "$collectorInjectionStarted = $true",
        "$recorderBudgetSeconds = $DurationSeconds + $RecorderOverheadSeconds",
        "a throw between the two would hide a live recorder from every cleanup decision",
    );
    assert_before(
        &source,
        name,
        "if ($gradedWindowStarted -and -not $collectorInjectionStarted) {\n        $gradedWindowStarted = $false",
        "'BATTLE_PLAN_CLIENT_DRIVING_OPT_IN',\n            $clientDrivingOptInSuspended,",
        "the branch has to CLOSE the window, not merely notice it: closing is what \
         allows the token restore that follows",
    );
}

/// The client-driving authority is withdrawn before the graded boundary opens,
/// and given back only inside cleanup.
#[test]
fn the_harness_withdraws_client_driving_authority_before_the_graded_boundary() {
    let name = "Invoke-HeadlessRawCapture.ps1";
    let source = read(name);

    assert_before(
        &source,
        name,
        "$clientDrivingOptInSuspended = [Environment]::GetEnvironmentVariable(",
        "$gradedWindowStarted = $true",
        "the token must already be read when the graded window opens",
    );
    // The CLEAR itself, which is the statement that withdraws the authority.
    // Reading the old value and asserting afterwards proves nothing if the
    // clear moved past the boundary.
    let cleared = concat!(
        "[Environment]::SetEnvironmentVariable(
",
        "    'BATTLE_PLAN_CLIENT_DRIVING_OPT_IN',
",
        "    $null,
",
        "    [EnvironmentVariableTarget]::Process
",
        ")"
    );
    assert_before(
        &source,
        name,
        cleared,
        "if ($null -ne [Environment]::GetEnvironmentVariable(",
        "the token is cleared and only then verified",
    );
    assert_before(
        &source,
        name,
        cleared,
        "$gradedWindowStarted = $true",
        "the authority must be gone before the graded window opens, not after",
    );
    assert_before(
        &source,
        name,
        "throw 'The graded window cannot open while the client-driving opt-in token is still set.'",
        "$gradedWindowStarted = $true",
        "a token that could not be cleared must stop the run rather than open the window",
    );
}

/// The sample is written to the witness file before any verdict is asked for.
///
/// The original harness threw before it wrote, so the observation that ended a
/// run could never be persisted and two incorrect diagnoses of the incident
/// were made from the surviving successes. Moving `Assert-PassiveWitnessVerdict`
/// above the `Add-Content` restores exactly that.
#[test]
fn the_harness_records_a_sample_before_it_asks_for_a_verdict() {
    let name = "Invoke-HeadlessRawCapture.ps1";
    let source = read(name);

    assert_before(
        &source,
        name,
        "Add-Content -LiteralPath $passiveWitnessPath -Encoding UTF8",
        "
    Assert-PassiveWitnessVerdict
",
        "the sample that ends a run is the one that must survive on disk",
    );
}
