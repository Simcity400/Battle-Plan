use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/positive/fx_owner_controller_flip.battleplan")
}

/// A `replay_app` invocation with every blocker-registry environment override
/// stripped. `--validate-recording` and ordinary startup both run
/// `reject_env_overrides` before doing anything else, so an inherited debug
/// variable in a developer's shell would fail these tests on a perfectly
/// healthy build. The names come from the registry itself (via
/// `--list-blockers`), never from a list copied into this file.
fn sanitized_command() -> Command {
    let names = registry_env_override_names();
    sanitized_command_with_env_override_names(&names)
}

fn sanitized_command_with_env_override_names(names: &[String]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_replay_app"));
    for name in names {
        command.env_remove(name);
    }
    command
}

fn bounded_output(mut command: Command) -> Output {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn replay_app preflight");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if child.try_wait().expect("poll replay_app preflight").is_some() {
            return child
                .wait_with_output()
                .expect("collect replay_app preflight output");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .expect("collect timed-out replay_app output");
            panic!(
                "replay_app did not exit within 15s; stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn invalid_canonical_dump_hour_exits_nonzero_with_requested_hour_and_valid_range() {
    let recording = fixture_path();
    let source = replay_core::open_recording(&recording).expect("open fixture index");
    let first = source.hours()[0];
    let last = *source.hours().last().expect("last fixture hour");

    for (requested, relation) in [(first - 1, "before"), (last + 1, "after")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_replay_app"));
        command
            .arg("--recording")
            .arg(&recording)
            .arg("--dump-frame")
            .arg("must_not_render.bmp")
            .arg("--dump-hour")
            .arg(requested.to_string());
        let output = bounded_output(command);

        assert_eq!(output.status.code(), Some(2), "invalid hour must exit 2");
        assert!(output.stdout.is_empty(), "invalid preflight wrote stdout");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!(
                "requested canonical hour {requested} is {relation}"
            )),
            "stderr omitted requested hour/relation: {stderr}"
        );
        assert!(
            stderr.contains(&format!(
                "recording's valid canonical range {first}..={last}"
            )),
            "stderr omitted canonical range: {stderr}"
        );
        assert!(
            !stderr.contains("resource"),
            "selection must fail before resource discovery: {stderr}"
        );
    }
}

// ---------------------------------------------------------------------------
// Production dump contract, driven by the blocker registry itself.
//
// These replace the `replay_qa` "recording ingest + scrub" gate, which was built
// against the OLD contract (a recording invocation could carry `--bundle`,
// `--capture`, `--dump-zoom`, `--dump-center`) and can no longer fire at all now
// that those are `Scope::ProductionRecording` registry rows. That gate also lived
// in replay_qa's library, reachable only from `run_harness` and from no `#[test]`,
// so it could rot green. Here it is an ordinary integration test over the BUILT
// binary: it proves the shipped `replay_app` actually invokes the refusal, which
// the unit test over `reject_production_recording_debug_cli` cannot.
//
// The token list is read from `--list-blockers` -- the registry's own mechanical
// emission -- so this file never holds a second copy of a table the code owns.
// ---------------------------------------------------------------------------

const REFUSAL_MARKER: &str = "is blocked in production recording playback";

struct BlockerRow {
    surface_kind: String,
    token: String,
    scope: String,
}

/// Parse `--list-blockers`. Every deviation from the documented shape is a hard
/// failure: a silently-skipped malformed line is exactly how a scoped row would
/// stop being exercised without anyone noticing.
fn list_blockers() -> Vec<BlockerRow> {
    // `--list-blockers` is intercepted before `parse_cli` AND before
    // `reject_env_overrides`, so it needs no environment sanitizing -- which is
    // what lets `sanitized_command` bootstrap from it without recursing.
    let mut command = Command::new(env!("CARGO_BIN_EXE_replay_app"));
    command.arg("--list-blockers");
    let output = bounded_output(command);
    assert_eq!(
        output.status.code(),
        Some(0),
        "--list-blockers must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("--list-blockers stdout is utf-8");
    let mut lines = stdout.lines();

    let header = lines.next().expect("--list-blockers emitted no header");
    let declared: usize = header
        .strip_prefix("replay_app product blockers: ")
        .unwrap_or_else(|| panic!("unexpected --list-blockers header: {header}"))
        .trim()
        .parse()
        .unwrap_or_else(|err| panic!("unparseable blocker count in {header:?}: {err}"));
    assert!(declared > 0, "--list-blockers declared zero blocker rows");

    let mut rows = Vec::with_capacity(declared);
    for index in 0..declared {
        let line = lines
            .next()
            .unwrap_or_else(|| panic!("--list-blockers ended after {index} of {declared} rows"));
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            7,
            "blocker row {index} is not 7 tab-separated fields: {line:?}"
        );
        // Validate the two enumerated fields this test filters on. Without this,
        // a row whose scope or surface kind is renamed or misspelled would simply
        // stop matching the filter below and quietly stop being exercised, while
        // the non-vacuity assertion still passed on the rows that survived. These
        // are closed sets in the code (`Scope::as_str`, `Surface::kind_str`), so
        // an unknown value means the emission changed and this test must be
        // updated deliberately rather than silently narrowing its coverage.
        assert!(
            matches!(fields[2], "cli-switch" | "env-override"),
            "blocker row {index} has an unknown surface kind {:?}; \
             --list-blockers emission changed: {line:?}",
            fields[2]
        );
        assert!(
            matches!(fields[5], "always" | "production-recording"),
            "blocker row {index} has an unknown scope {:?}; \
             --list-blockers emission changed: {line:?}",
            fields[5]
        );
        rows.push(BlockerRow {
            surface_kind: fields[2].to_string(),
            token: fields[3].to_string(),
            scope: fields[5].to_string(),
        });
    }

    // The trailing sanctioned-environment section has a different arity. Assert
    // its shape rather than letting a 4-field line be mistaken for a short
    // blocker row.
    let env_header = lines
        .next()
        .expect("--list-blockers omitted the sanctioned environment section");
    let env_declared: usize = env_header
        .strip_prefix("replay_app sanctioned environment inputs: ")
        .unwrap_or_else(|| panic!("unexpected environment section header: {env_header}"))
        .trim()
        .parse()
        .unwrap_or_else(|err| panic!("unparseable env-input count in {env_header:?}: {err}"));
    for index in 0..env_declared {
        let line = lines.next().unwrap_or_else(|| {
            panic!("environment section ended after {index} of {env_declared} rows")
        });
        assert_eq!(
            line.split('\t').count(),
            4,
            "environment row {index} is not 4 tab-separated fields: {line:?}"
        );
    }
    assert!(
        lines.next().is_none(),
        "--list-blockers emitted trailing content past both declared sections"
    );

    rows
}

fn registry_env_override_names() -> Vec<String> {
    registry_env_override_names_from(&list_blockers())
}

fn registry_env_override_names_from(rows: &[BlockerRow]) -> Vec<String> {
    rows
        .into_iter()
        .filter(|row| row.surface_kind == "env-override")
        .map(|row| row.token.clone())
        .collect()
}

/// The production-recording-scoped CLI switches, from the registry.
fn production_recording_scoped_tokens_from(rows: &[BlockerRow]) -> Vec<String> {
    let tokens: Vec<String> = rows
        .into_iter()
        .filter(|row| row.scope == "production-recording" && row.surface_kind == "cli-switch")
        .map(|row| row.token.clone())
        .collect();
    assert!(
        !tokens.is_empty(),
        "registry declared no production-recording-scoped CLI switches; the contract \
         gate would be vacuous"
    );
    let unique: BTreeSet<&String> = tokens.iter().collect();
    assert_eq!(
        unique.len(),
        tokens.len(),
        "registry repeats a production-recording-scoped token: {tokens:?}"
    );
    tokens
}

/// A syntactically VALID argument for each scoped switch. It must parse, because
/// the refusal is checked after `parse_cli` -- a malformed value would produce an
/// ordinary usage error that names the same token and would be mistaken for a
/// refusal. Fails closed on an unmapped token, mirroring how
/// `recording_scoped_cli_present` in main.rs refuses to guess.
fn probe_value(token: &str) -> &'static str {
    match token {
        "--bundle" => "probe_bundle_dir",
        "--capture" => "probe_capture_dir",
        "--dump-zoom" => "0.85",
        "--dump-center" => "3030,760",
        "--dump-hdr" => "probe_hdr.rgba16f",
        "--hour" => "0.5",
        other => panic!(
            "no probe value for production-recording-scoped registry token {other}; \
             add one here rather than letting the contract gate skip the row"
        ),
    }
}

/// Every production-recording-scoped CLI switch is refused by the BUILT binary,
/// before the recording is even opened.
///
/// The invocation deliberately also carries an out-of-range `--dump-hour`, so it
/// is hermetic: it can never reach the game install discovery, and the refusal must
/// preempt even the invalid-hour preflight. The paired control below runs the
/// identical argv WITHOUT a scoped switch and must reach that preflight instead,
/// which is what stops this test from passing on a binary that refuses everything.
#[test]
fn production_recording_scoped_cli_switches_are_refused_by_the_built_binary() {
    let recording = fixture_path();
    let source = replay_core::open_recording(&recording).expect("open fixture index");
    let out_of_range = *source.hours().last().expect("last fixture hour") + 1;
    let blocker_rows = list_blockers();
    let env_override_names = registry_env_override_names_from(&blocker_rows);
    let scoped_tokens = production_recording_scoped_tokens_from(&blocker_rows);

    for token in scoped_tokens {
        let mut command = sanitized_command_with_env_override_names(&env_override_names);
        command
            .arg("--recording")
            .arg(&recording)
            .arg("--dump-frame")
            .arg("must_not_render.bmp")
            .arg("--dump-hour")
            .arg(out_of_range.to_string())
            .arg(&token)
            .arg(probe_value(&token));
        let output = bounded_output(command);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert_eq!(
            output.status.code(),
            Some(2),
            "{token} must be refused with exit 2; stderr: {stderr}"
        );
        assert!(
            output.stdout.is_empty(),
            "{token} refusal wrote stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        // Exact refusal shape. A parse error also exits non-zero and also names
        // the token, so "non-zero and mentions the token" would be satisfied by
        // a broken build.
        let expected_prefix = format!("replay_app: {token} {REFUSAL_MARKER}");
        assert!(
            stderr.starts_with(&expected_prefix),
            "{token} did not produce the registry refusal.\n  expected prefix: \
             {expected_prefix}\n  actual stderr: {stderr}"
        );
        // The usage block itself contains the words "are blocked" for several
        // switches, so a usage dump must be excluded explicitly.
        assert!(
            !stderr.contains("usage:"),
            "{token} produced a usage/parse diagnostic, not a registry refusal: {stderr}"
        );
        // Ordering: the refusal must precede the dump-hour preflight, proving the
        // switch is rejected before the recording is opened.
        assert!(
            !stderr.contains("requested canonical hour"),
            "{token} reached the dump-hour preflight; the refusal must come first: {stderr}"
        );
    }
}

/// Control for the test above: the identical argv with NO scoped switch is NOT
/// refused by a blocker. Without this, the contract test would still pass if the
/// binary had regressed into refusing every recording invocation.
#[test]
fn production_contract_recording_invocation_is_not_blocker_refused() {
    let recording = fixture_path();
    let source = replay_core::open_recording(&recording).expect("open fixture index");
    let out_of_range = *source.hours().last().expect("last fixture hour") + 1;

    let mut command = sanitized_command();
    command
        .arg("--recording")
        .arg(&recording)
        .arg("--dump-frame")
        .arg("must_not_render.bmp")
        .arg("--dump-hour")
        .arg(out_of_range.to_string());
    let output = bounded_output(command);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stderr.contains(REFUSAL_MARKER),
        "a plain production-contract recording invocation was blocker-refused: {stderr}"
    );
    // It got past the registry gate and into the real preflight.
    assert!(
        stderr.contains("requested canonical hour"),
        "control invocation did not reach the dump-hour preflight: {stderr}"
    );
}

/// Ingest half of the retired scrub gate: the BUILT binary opens a committed,
/// multi-hour, snapshot-native recording end to end. `--validate-recording` runs
/// the whole recording through the exact playback open path and authenticates
/// every indexed frame payload, and it needs no map bundle, no GPU and no the game
/// install.
///
/// The hour span is asserted structurally rather than pinned to a golden number:
/// a pinned count would be a second copy of a fact the fixture already owns.
#[test]
fn committed_multi_hour_recording_is_ingested_by_the_built_binary() {
    let recording = fixture_path();
    let mut command = sanitized_command();
    command.arg("--validate-recording").arg(&recording);
    let output = bounded_output(command);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(0),
        "committed fixture must validate; stdout: {stdout}; stderr: {stderr}"
    );
    let summary = stdout.lines().next().expect("validator emitted no summary");
    assert!(
        summary.starts_with("opened \"fx_owner_controller_flip\":"),
        "unexpected validator summary: {summary}"
    );

    // `opened "<id>": <n> hour(s), <first>..=<last>`
    let tail = summary.split_once(": ").expect("summary has no ': '").1;
    let (count, span) = tail.split_once(" hour(s), ").expect("summary has no hour span");
    let count: usize = count.parse().expect("hour count is numeric");
    let (first, last) = span.split_once("..=").expect("span is not first..=last");
    let first: i64 = first.trim().parse().expect("first hour is numeric");
    let last: i64 = last.trim().parse().expect("last hour is numeric");

    assert!(
        count > 1,
        "a one-hour recording cannot exercise ingest across a timeline: {summary}"
    );
    assert!(first < last, "hour span is not increasing: {summary}");
    assert_eq!(
        last - first + 1,
        count as i64,
        "hour count and canonical span disagree: {summary}"
    );
}
