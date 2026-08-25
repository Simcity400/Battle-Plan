// Lane ST (2026-08-25): a panic in a split reader's DEFERRED half marks the
// bracket that has already closed over its reads.
//
// THE DEFECT CLASS, in one sentence. `spawn_gated` holds its
// `ReaderClockBracketGuard` to the end of the worker body, so a panic anywhere
// in that worker unwinds THROUGH the guard, `std::thread::panicking()` is true
// in its `Drop`, and the bracket is recorded `worker_panicked`. Every helper
// that moves work PAST the guard's drop moves it out of the only thing that can
// see the unwind: the bracket goes into `clock_brackets` clean, reconciliation
// has nothing to reject, and the domain's `..._worker_panic_read` substitute --
// minted at the join and badged `COMPACT_STATUS_WORKER_PANIC` -- PUBLISHES,
// sitting over whatever baseline the observe half had already advanced.
//
// Lane SQ found this on `spawn_gated_finishing` (d53) and closed it there with
// [`ClosedBracketPanicMarker`]. The same gap was open on TWO other helpers:
//
//   * `spawn_gated_split`, the older one, with three users -- d81
//     (`rail_replay_state_live`), d47 (`characters_leaders_cabinet_live`) and
//     d101 (`map_furniture_victory_points_live`);
//   * `country_child_bracket`, whose eight callers encode their publication
//     records after the guard closes (lane RU moved that encode onto the child).
//     Codex round 1 found this one, against an earlier version of this file that
//     classified the helper by its own body and missed that its CALLERS defer.
//     For seven of the eight children a panic there was only a wrong cause on
//     the wire; for the TECH child it also advanced d13's wire baseline inside
//     `country_bundle_publication_records` and then published an empty badged
//     record over it.
//
// This lane arms the same marker in both.
//
// THE FAMILY, OLD RULE vs NEW RULE. `every_bracket_helper_that_defers_work_past_
// its_guard_is_classified` derives the left-hand column from the source and
// enforces the right-hand one; this table is the reading of it, not its source.
//
//   helper                  | bracket held over      | panic past the bracket
//                           |                        | OLD          -> NEW
//   ------------------------+------------------------+-----------------------
//   spawn_gated             | the whole worker body  | (nothing is past it)
//   spawn_gated_lane_only   | opens NO bracket       | (there is no bracket)
//   spawn_gated_publishing  | read + bookkeeping     | caught, cannot unwind
//   spawn_gated_split       | observe only           | CLEAN -> panicked
//   spawn_gated_finishing   | observe only           | panicked (lane SQ)
//   country_child_bracket   | the read only          | CLEAN -> panicked
//   lane_time_main          | the whole call         | (nothing is past it)
//
// WHY MAIN-THREAD BRACKETS ARE NOT IN THE CLASS, even where a serial reader
// defers work past its guard. The marker exists because a WORKER's panic is
// turned into a PUBLISHED RECORD: `handle.join()` returns `Err` and
// `joined_or_panic_outcome` substitutes a badged read that then seals. A panic
// on the collector thread is caught only at the capture loop's own
// `catch_unwind`, which keeps it as a terminal error and stops the loop; the
// interrupted frame builder returns no bytes, so nothing is sealed for a missing
// gap to contradict. The accounting gap needs a substitute to exist. That is why
// `build_compact_extraction_gate_records` holds fifteen bracket guards and is
// exempt, and why `no_scoped_worker_opens_a_bracket_outside_the_classified_helpers`
// is the guard that keeps the exemption honest: it is only sound while every
// bracket opened on a WORKER comes from a helper this file classified.

use super::d53_phase_split::{code_only, item_body};
use super::state_fixture::fixture_game_hour_raw;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// `code` with every comment body and every string/char literal blanked to
/// spaces, byte offsets preserved.
///
/// Brace depth is the only reliable way to ask "does this guard drop before the
/// item's last statement", and a `{` inside a comment or a string would answer
/// it wrongly -- `lw_d{tag}_crit_us` appears in this file's prose more than once.
///
/// TWO PROPERTIES THE CALLERS DEPEND ON. Byte offsets are preserved, so an index
/// found in the blanked copy indexes the ORIGINAL source unchanged. And LINE
/// BREAKS ARE PRESERVED even inside a comment or a multi-line string literal, so
/// the blanked copy has exactly as many lines as the source and the two can be
/// walked together.
fn blank_comments_and_literals(code: &str) -> String {
    let bytes = code.as_bytes();
    let mut out = vec![b' '; bytes.len()];
    let mut at = 0usize;
    // Keep the line structure identical to the source's.
    let mut skip = |out: &mut Vec<u8>, at: &mut usize| {
        if bytes[*at] == b'\n' {
            out[*at] = b'\n';
        }
        *at += 1;
    };
    while at < bytes.len() {
        let byte = bytes[at];
        match byte {
            b'/' if at + 1 < bytes.len() && bytes[at + 1] == b'/' => {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
            }
            b'"' => {
                at += 1;
                while at < bytes.len() {
                    if bytes[at] == b'\\' && at + 1 < bytes.len() {
                        skip(&mut out, &mut at);
                        skip(&mut out, &mut at);
                        continue;
                    }
                    if bytes[at] == b'"' {
                        at += 1;
                        break;
                    }
                    skip(&mut out, &mut at);
                }
            }
            // AN ESCAPED CHAR LITERAL, of any length: `'\n'`, `'\\'`, `'\u{11}'`.
            // Bounded by the closing quote rather than by a fixed width, because
            // `'\u{11}'` exists in this file and a fixed-width rule copied its
            // braces through (Codex round 1, non-blocking). Bounded by the line
            // too, so a stray quote cannot swallow the rest of the file.
            b'\'' if at + 1 < bytes.len() && bytes[at + 1] == b'\\' => {
                let mut end = at + 2;
                while end < bytes.len() && bytes[end] != b'\'' && bytes[end] != b'\n' {
                    end += 1;
                }
                at = if end < bytes.len() && bytes[end] == b'\'' {
                    end + 1
                } else {
                    at + 1
                };
            }
            // A PLAIN CHAR LITERAL, distinguished from a lifetime by its closing
            // quote: `'a` in `<'scope>` has none, `'{'` does.
            b'\'' if at + 2 < bytes.len() && bytes[at + 2] == b'\'' => {
                at += 3;
            }
            _ => {
                out[at] = byte;
                at += 1;
            }
        }
    }
    String::from_utf8(out).expect("blanking replaces bytes one for one with ASCII")
}

/// Where `body`'s guard's enclosing block closes, or `None` when the guard is
/// held to the end of the body.
///
/// Returns `Some(index)` only when there is CODE after the close -- closing
/// delimiters and separators do not count, because `spawn_gated`'s guard is
/// followed by `}\n    })\n}` and that is not deferred work.
fn where_the_bracket_stops_covering(body: &str) -> Option<usize> {
    let blanked = blank_comments_and_literals(body);
    let guard_at = blanked.find("ReaderClockBracketGuard::new")?;
    let bytes = blanked.as_bytes();
    let mut depth = 0i32;
    for byte in &bytes[..guard_at] {
        match byte {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
    }
    let guard_depth = depth;
    for (offset, byte) in bytes[guard_at..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if depth < guard_depth {
            let closes_at = guard_at + offset;
            let tail: String = blanked[closes_at + 1..]
                .chars()
                .filter(|c| !c.is_whitespace() && !matches!(c, '}' | ')' | ';' | ','))
                .collect();
            return (!tail.is_empty()).then_some(closes_at);
        }
    }
    None
}

/// EVERY function in `attach_log.rs` -- free or `impl` method, at any
/// indentation -- as `(name, body)`, from the BLANKED source so a name inside a
/// comment or a string cannot be mistaken for a declaration.
///
/// TWO EARLIER VERSIONS WERE NARROWER AND BOTH WERE WRONG. The first keyed on a
/// bare column-zero `fn `, which a `pub(super) fn` would have slipped (Codex
/// round 1). The second accepted the visibility prefixes but stayed at column
/// zero, so an indented `impl` method that opened a clock bracket would have
/// been invisible to the roster while a new worker called it and deferred past
/// it (Codex round 2). A third counted only parentheses when looking for the
/// body, so a function whose return type is an ARRAY -- the `;` in `[T; N]` --
/// read as bodyless and vanished (Codex round 3). This walks every `fn` in the
/// file -- around three thousand of them -- and brace-matches each body, which
/// is also what lets a nested item be found rather than swallowed.
///
/// A function with no body (a trait signature ending in `;`) is skipped: the
/// scan stops at the first `;` outside the parameter list.
fn function_items(blanked: &str) -> Vec<(String, &str)> {
    let bytes = blanked.as_bytes();
    let mut items = Vec::new();
    let mut from = 0usize;
    while let Some(at) = blanked[from..].find("fn ") {
        let keyword = from + at;
        from = keyword + 3;
        // `fn` must be a word of its own: preceded by start-of-file or a byte
        // that cannot continue an identifier. That rejects `..._fn `.
        if keyword > 0 {
            let previous = bytes[keyword - 1];
            if previous.is_ascii_alphanumeric() || previous == b'_' {
                continue;
            }
        }
        let name: String = blanked[keyword + 3..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        // Walk to the body's opening brace, past the generics, the parameter
        // list and any `where` clause. A `;` outside every bracket means a
        // declaration with no body.
        //
        // SQUARE BRACKETS ARE COUNTED, and Codex round 3 was right that they
        // have to be: an array type carries a semicolon, so
        // `fn f() -> [ForbiddenSiteOwner; 3] {` read as bodyless under a
        // parens-only rule and the function vanished from the roster. Several
        // functions in this file have that shape. None of them opens a bracket
        // today, so the seven-name assertion was correct — but a future
        // array-returning bracket helper would have been invisible, which is the
        // one thing this roster exists to prevent.
        let mut cursor = keyword + 3 + name.len();
        let mut parens = 0i32;
        let mut squares = 0i32;
        let body_open = loop {
            let Some(&byte) = bytes.get(cursor) else {
                break None;
            };
            match byte {
                b'(' => parens += 1,
                b')' => parens -= 1,
                b'[' => squares += 1,
                b']' => squares -= 1,
                b'{' if parens == 0 && squares == 0 => break Some(cursor),
                b';' if parens == 0 && squares == 0 => break None,
                _ => {}
            }
            cursor += 1;
        };
        let Some(body_open) = body_open else {
            continue;
        };
        let Some(body_close) = matching_delimiter(bytes, body_open, b'{', b'}') else {
            continue;
        };
        items.push((name, &blanked[keyword..body_close + 1]));
    }
    items
}

/// Every function in `attach_log.rs` that opens a clock bracket, its bracket
/// discipline, and whether it arms the closed-bracket panic marker -- the set is
/// DERIVED from the source, not listed.
///
/// THE ROSTER IS THE POINT. A new bracket-opening function fails this test on its
/// name alone, which is what forces its author to decide the question this lane
/// answered for the six: does anything run after the guard's `Drop`, and if so,
/// what tells the closed bracket about a panic in it? An earlier version of this
/// test enumerated `fn spawn_*` instead, and Codex round 1 was right that the
/// narrower roster is what let `country_child_bracket`'s deferred publish encode
/// stay in the defect class: the helper's OWN body holds its guard to the end,
/// and the deferral lives one level up in its eight callers. That is why the
/// classification below is `defers` computed over the helper AND
/// `no_scoped_worker_opens_a_bracket_outside_the_classified_helpers` over the
/// callers, and why `country_child_bracket` now takes its callers' tail as a
/// parameter instead of leaving it outside.
///
/// SPLIT.md section 10's rule -- a new spawn helper spelling must also be added
/// to the reader-call spelling table in `domain_scoped_reset_tests.rs` -- is the
/// other half of the same obligation.
///
/// WHY `spawn_gated_publishing` MAY DEFER WITHOUT A MARKER: its deferred half is
/// `catch_unwind`-wrapped, so it cannot unwind the worker and there is no closed
/// bracket to teach. That is asserted here and demonstrated by
/// `the_publishing_helper_catches_instead_of_marking`, so removing the catch
/// breaks a test instead of silently reopening the class.
///
/// WHY `build_compact_extraction_gate_records` IS EXEMPT: it is the frame builder
/// on the COLLECTOR thread, and its fifteen brackets are main-thread serial
/// reads. A panic there IS caught -- at the capture loop's own `catch_unwind`,
/// which keeps it as a terminal error and stops the loop -- but it is never
/// turned into a per-domain SUBSTITUTE record: the interrupted builder returns no
/// frame bytes, so nothing is sealed for a missing gap to contradict (Codex
/// round 2, non-blocking 3, correcting an earlier "caught by nothing"). The
/// marker only repairs an accounting that a substitute distorts.
///
/// kills: deleting the marker from any of the three helpers that need one;
/// adding a bracket-opening function without deciding this; dropping
/// `spawn_gated_publishing`'s `catch_unwind` (the arm below then demands a marker
/// it does not have); giving `spawn_gated`, `spawn_gated_lane_only` or
/// `lane_time_main` a deferred half.
#[test]
fn every_bracket_helper_that_defers_work_past_its_guard_is_classified() {
    let source = super::collector_source_lf();
    let blanked = blank_comments_and_literals(&source);
    assert_eq!(
        blanked.lines().count(),
        source.lines().count(),
        "the blanked copy must keep the source's line structure, or every offset below \
         indexes the wrong place",
    );

    let items = function_items(&blanked);
    // THE SCANNER'S OWN GUARD, before the roster it produces is trusted. An
    // array return type carries a semicolon and a parens-only body scan read it
    // as a declaration, silently dropping the function (Codex round 3). This is
    // that shape, named: `forbidden_site_owners` returns
    // `[ForbiddenSiteOwner; 3]`. If the scan regresses, the roster below can
    // stay green while a bracket opener of this shape is invisible.
    assert!(
        items
            .iter()
            .any(|(name, _)| name == "forbidden_site_owners"),
        "the scan must see a function whose return type is an array; if it does not, every \
         roster this test derives is missing an unknown number of functions",
    );
    let mut openers: Vec<&(String, &str)> = items
        .iter()
        .filter(|(_, body)| body.contains("ReaderClockBracketGuard::new"))
        .collect();
    openers.sort_by(|left, right| left.0.cmp(&right.0));
    let names: Vec<&str> = openers.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "build_compact_extraction_gate_records",
            "country_child_bracket",
            "lane_time_main",
            "spawn_gated",
            "spawn_gated_finishing",
            "spawn_gated_publishing",
            "spawn_gated_split",
        ],
        "a new function that opens a clock bracket must be classified here",
    );

    // (function, defers work past its guard, arms the marker)
    let expected: [(&str, bool, bool); 6] = [
        ("spawn_gated", false, false),
        ("spawn_gated_publishing", true, false),
        ("spawn_gated_split", true, true),
        ("spawn_gated_finishing", true, true),
        ("country_child_bracket", true, true),
        ("lane_time_main", false, false),
    ];
    for (name, defers, arms_marker) in expected {
        let (_, body) = openers
            .iter()
            .find(|(item, _)| item == name)
            .unwrap_or_else(|| panic!("`{name}` must be one of the bracket openers"));
        let stops_at = where_the_bracket_stops_covering(body);
        assert_eq!(
            stops_at.is_some(),
            defers,
            "`{name}` runs code after its guard has dropped: expected {defers}",
        );
        assert_eq!(
            body.contains("ClosedBracketPanicMarker"),
            arms_marker,
            "`{name}` panic-marker expectation",
        );
        if defers && !arms_marker {
            let closes_at = stops_at.expect("a deferring helper has a close");
            assert!(
                body[closes_at..].contains("std::panic::catch_unwind"),
                "`{name}` defers work past its bracket and arms no marker, so its deferred \
                 half must be unable to unwind the worker",
            );
        }
    }

    // `spawn_gated_lane_only` opens no bracket at all, which is why it is absent
    // from the roster above rather than classified in it. Asserted rather than
    // left implicit: giving it a bracket would put a parent span with no domain
    // back into `frame_boundary_position`'s straddle counters.
    let (_, lane_only) = items
        .iter()
        .find(|(name, _)| name == "spawn_gated_lane_only")
        .expect("the lane-only helper must exist");
    assert!(
        !lane_only.contains("ReaderClockBracketGuard"),
        "the lane-only helper must stay bracket-free",
    );
}

/// No SCOPED WORKER opens a clock bracket except through a helper this file
/// classified.
///
/// THIS IS WHAT THE ROSTER TEST CANNOT SEE, and the gap Codex round 1 found. A
/// helper can hold its guard to the end of its own body and still leave the
/// defect open, because the work that runs after the bracket lives in the CALLER
/// -- which is exactly what `country_child_bracket`'s eight workers did with
/// lane RU's publish encode. So the callers are enumerated too: every
/// `scope.spawn(` in the file, with the bracket-opening spellings its worker body
/// uses.
///
/// It is also the whole licence for exempting main-thread brackets. That argument
/// -- a collector-thread panic seals no frame, so there is no substitute to
/// account for -- is only sound while every bracket opened ON A WORKER comes from
/// a classified helper. This is the assertion that keeps it true.
///
/// THE SPELLINGS IT LOOKS FOR ARE DERIVED, not listed: the roster of
/// bracket-opening functions is recomputed here and each name becomes a call
/// spelling, so a worker that reaches a bracket through a NEW helper is caught
/// without this test being edited.
///
/// LIMIT, STATED. The scan sees one step -- the guard, or a call to a function
/// that opens one. A worker that reached a bracket through TWO hops (calling a
/// function that calls an opener) would not be caught here. Today there is no
/// such path: the roster is seven functions and every one of them is either a
/// bracket helper called directly by a worker or the main-thread frame builder,
/// which no worker calls. A new intermediate would have to be added to the
/// roster's callees to create one, and the roster test is where that lands.
///
/// kills: a new `scope.spawn` worker that opens its own `ReaderClockBracketGuard`
/// inline; a country child that stops going through `country_child_bracket`; a
/// worker pool inside a reader that starts holding a bracket per child; a new
/// bracket helper called from a worker.
#[test]
fn no_scoped_worker_opens_a_bracket_outside_the_classified_helpers() {
    let source = super::collector_source_lf();
    let blanked = blank_comments_and_literals(&source);
    let items = function_items(&blanked);

    let mut spellings: Vec<String> = vec!["ReaderClockBracketGuard::new".to_string()];
    for (name, body) in &items {
        if body.contains("ReaderClockBracketGuard::new") {
            let spelling = format!("{name}(");
            if !spellings.contains(&spelling) {
                spellings.push(spelling);
            }
        }
    }

    // (enclosing function, bracket-opening spelling the worker body uses)
    let mut found: Vec<(String, String)> = Vec::new();
    for (name, body) in &items {
        let bytes = body.as_bytes();
        let mut from = 0usize;
        while let Some(at) = body[from..].find("scope.spawn(") {
            let open = from + at + "scope.spawn(".len() - 1;
            from = open + 1;
            let Some(close) = matching_delimiter(bytes, open, b'(', b')') else {
                continue;
            };
            let worker = &body[open + 1..close];
            for spelling in &spellings {
                if worker.contains(spelling.as_str()) {
                    found.push((name.clone(), spelling.clone()));
                }
            }
        }
    }
    found.sort();

    let mut expected: Vec<(String, String)> = Vec::new();
    for helper in [
        "spawn_gated",
        "spawn_gated_publishing",
        "spawn_gated_split",
        "spawn_gated_finishing",
    ] {
        expected.push((
            helper.to_string(),
            "ReaderClockBracketGuard::new".to_string(),
        ));
    }
    // The eight country children, each through the helper that arms the marker.
    for _ in 0..8 {
        expected.push((
            "read_compact_country_domain_bundles_parallel".to_string(),
            "country_child_bracket(".to_string(),
        ));
    }
    expected.sort();
    assert_eq!(
        found, expected,
        "every bracket a scoped worker opens must come from a classified helper; a new one \
         needs a row in `every_bracket_helper_that_defers_work_past_its_guard_is_classified` \
         and a decision about the panic marker",
    );
}

/// Where the delimiter opened at `open` closes, ignoring nothing -- the input is
/// already blanked, so no delimiter inside a comment or a literal reaches here.
fn matching_delimiter(bytes: &[u8], open: usize, opener: u8, closer: u8) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in bytes[open..].iter().enumerate() {
        if *byte == opener {
            depth += 1;
        } else if *byte == closer {
            depth -= 1;
            if depth == 0 {
                return Some(open + offset);
            }
        }
    }
    None
}

/// Every country child hands its publish encode to `country_child_bracket` as
/// the post-bracket tail, rather than running it as the worker's next statement.
///
/// WHY THE SHAPE MATTERS AND NOT JUST THE MARKER. Arming a marker at each of the
/// eight sites would work and would be wrong: the tag would be repeated by hand
/// eight times, and a mistyped one marks ANOTHER domain's bracket -- a silent
/// gap on a domain that read cleanly. Handing the tail to the helper makes the
/// marker's tag the guard's tag by construction.
///
/// kills: a ninth child added the old way; any child that assigns
/// `published_records` outside the helper's tail closure.
#[test]
fn every_country_child_publishes_through_the_bracket_helper() {
    let source = super::collector_source_lf();
    let blanked = blank_comments_and_literals(&source);
    let items = function_items(&blanked);
    let (_, parallel) = items
        .iter()
        .find(|(name, _)| name == "read_compact_country_domain_bundles_parallel")
        .expect("the country fan-out must exist");

    let calls = parallel.matches("country_child_bracket(").count();
    assert_eq!(
        calls, 8,
        "each of the eight country children holds its own bracket through the helper",
    );
    let publishes = parallel
        .matches("read.published_records = Some(records);")
        .count();
    assert_eq!(
        publishes, 8,
        "and each publishes exactly once, from inside the helper's tail closure",
    );
    // THE TAIL IS INSIDE THE CALL. Every `published_records` assignment must fall
    // between a `country_child_bracket(` and its matching `)`, which is what
    // makes the marker cover it.
    let bytes = parallel.as_bytes();
    let mut covered = 0usize;
    let mut from = 0usize;
    while let Some(at) = parallel[from..].find("country_child_bracket(") {
        let open = from + at + "country_child_bracket(".len() - 1;
        from = open + 1;
        let Some(close) = matching_delimiter(bytes, open, b'(', b')') else {
            panic!("a `country_child_bracket(` call must close");
        };
        covered += parallel[open..close]
            .matches("read.published_records = Some(records);")
            .count();
    }
    assert_eq!(
        covered, 8,
        "every child's publish encode must run inside the helper's call, where the panic \
         marker covers it -- not after it, where the guard has closed and nothing does",
    );
}

/// The marker is armed with NOTHING between it and the split guard's drop, and
/// before every other post-bracket local.
///
/// TWO SEPARATE CLAIMS. "Nothing between" is Codex's finding against lane SQ's
/// first spelling of the same fix: a statement in that gap is a statement whose
/// panic loses the `worker_panicked` verdict, the `WorkerPanic` gap and the
/// baseline rollback, exactly as a panic in `compute` would. "Before every other
/// local" is the other end -- Rust drops locals in reverse declaration order, so
/// declaring the marker first is what keeps it alive across `stamp`,
/// `record_bracket_split`, `lane_cost_observe`, `drop(permit)` and
/// `lanes.record`, all of which ran under `spawn_gated`'s guard.
///
/// COMPARED AS AN EXACT SLICE, not as an ordering, for the reason Codex gave
/// lane SQ: `close < marker < stamp` admits any statement someone inserts in the
/// gap, which is the defect itself, re-openable.
///
/// kills: constructing the marker after the `bracket_closed` stamp; moving it
/// below the `verdict.get()`; inserting any statement between the guarded block
/// and the marker; declaring another local before it.
#[test]
fn nothing_runs_between_the_split_guards_drop_and_the_panic_marker() {
    let source = super::collector_source_lf();
    let helper = item_body(&source, "fn spawn_gated_split<");
    let open = helper
        .find("let observed = {")
        .expect("the guarded block must be bound to `observed`");
    let close = helper[open..]
        .find("\n        };")
        .map(|at| open + at)
        .expect("the guarded block must close");
    let marker = "let _panic_marker = ClosedBracketPanicMarker { lanes, tag };";
    let marker_at = helper
        .find(marker)
        .expect("the closed-bracket panic marker must be armed");
    assert!(
        marker_at > close,
        "the marker must be armed after the guarded block, not inside it",
    );
    let between = code_only(&helper[close + "\n        };".len()..marker_at]);
    assert!(
        between.is_empty(),
        "nothing may run between the guard's drop and the panic marker; found:\n{between}",
    );
    for later in [
        "let bracket_closed = lanes.origin.elapsed();",
        "let guard_outcome = verdict.get();",
        "let out = match guard_outcome.refusal {",
        "let out = stamp(out, guard_outcome.observed_hour_raw);",
        "let end = lanes.origin.elapsed();",
    ] {
        let at = helper
            .find(later)
            .unwrap_or_else(|| panic!("`{later}` must appear in the helper"));
        assert!(
            marker_at < at,
            "the marker must be declared before `{later}`, or it is dropped before that \
             statement runs and a panic there loses its bracket",
        );
    }
    // AND THE CLOSING BOOKKEEPING IS STILL INSIDE IT. These ran under
    // `spawn_gated`'s guard too, so a panic in them was a `worker_panicked`
    // bracket before the split existed and must stay one.
    for tail in [
        "lanes.record_bracket_split(",
        "lane_cost_observe(tag, duration_micros_u64(end.saturating_sub(start)));",
        "drop(permit);",
        "lanes.record(tag, lane, start, end);",
    ] {
        assert!(
            helper
                .find(tail)
                .is_some_and(|at| at > marker_at),
            "`{tail}` must run while the marker is still alive",
        );
    }
}

/// A coherent bracket in the frame's own hour, so the helper's `compute` arm --
/// not one of its refusal arms -- is the one under test.
///
/// A `FrameLaneTracker::new(_, _, 0, 0)` has no readable clock, so every split
/// worker built on one takes `SplitBracketRefusal::BracketIncoherent` and never
/// reaches `compute` at all (`a_refused_frames_measurements_survive_both_
/// discard_sites` says so in as many words). Every behavioural test below is
/// built on the staged map fixture instead, whose scheduler clock authenticates
/// and reads `fixture_game_hour_raw()`.
fn coherent_split_lanes() -> (
    super::state_fixture::CollectorStateFixture,
    FrameLaneTracker,
    i32,
) {
    // A FRESH PAGE-CACHE EPOCH, as every other fixture-backed test in this tree
    // takes. Each call stages a new fixture, and a previous one freed at the
    // same address would otherwise be served out of the cache -- an authentic
    // clock read of somebody else's bytes.
    page_cache_new_frame();
    let fixture = super::shared_map_topology::staged_map_fixture();
    let state = fixture.collector_state();
    let lanes = super::shared_map_topology::lanes_for(&state);
    (fixture, lanes, fixture_game_hour_raw())
}

/// Drive one split worker whose `compute` half panics, and hand back the
/// brackets the frame recorded.
///
/// `preserve`, `rejected` and `stamp` are the identity-ish minimum: the arm
/// under test is `compute`, and a refusal arm reached by accident is caught by
/// the sentinel the caller asserts on rather than by a silent pass.
fn brackets_after_a_panicking_compute_half(
    lanes: &FrameLaneTracker,
    tag: u16,
    frame_hour_raw: i32,
) -> Vec<DomainClockBracket> {
    let joined = thread::scope(|scope| {
        spawn_gated_split(
            scope,
            None,
            lanes,
            tag,
            frame_hour_raw,
            |_bracket_opening| 1u32,
            |_observed: u32| -> u32 { panic!("a latent bug in the deferred compute half") },
            |observed: &u32| *observed,
            |_refusal, _observed| REFUSAL_SENTINEL,
            |out, _observed_hour_raw| out,
        )
        .join()
    });
    assert!(
        joined.is_err(),
        "the panic must still propagate to the join, which is where the domain's panic \
         substitute is minted; got {joined:?}",
    );
    lanes.take_clock_brackets()
}

/// What a refusal arm returns when the fixture's clock did not behave. No real
/// domain returns a `u32`, so this can only come from these tests.
const REFUSAL_SENTINEL: u32 = 0xDEAD_BEEF;

/// The control: on this fixture the split really does reach `compute`.
///
/// Without it every panic test below could be passing because the REFUSAL arm
/// panicked, which is a different statement about a different arm.
///
/// kills: a fixture whose clock stops authenticating; a frame label that no
/// longer matches the staged hour; `spawn_gated_split` routing a coherent
/// in-hour bracket anywhere but `compute`.
#[test]
fn the_staged_fixture_reaches_the_split_helpers_compute_arm() {
    const TAG: u16 = 0xD5B;
    let (_fixture, lanes, hour) = coherent_split_lanes();
    let out = thread::scope(|scope| {
        spawn_gated_split(
            scope,
            None,
            &lanes,
            TAG,
            hour,
            |_bracket_opening| 7u32,
            |observed: u32| observed * 6,
            |observed: &u32| *observed,
            |_refusal, _observed| REFUSAL_SENTINEL,
            |out, _observed_hour_raw| out,
        )
        .join()
        .expect("a clean worker joins cleanly")
    });
    assert_eq!(
        out, 42,
        "the compute half's value is what the join returns; {REFUSAL_SENTINEL:#x} would mean \
         the bracket was refused and no test below is testing what it says",
    );
    let brackets = lanes.take_clock_brackets();
    let ours = brackets
        .iter()
        .find(|bracket| bracket.tag == TAG)
        .expect("the worker recorded its bracket");
    assert!(
        !ours.worker_panicked,
        "a worker that completed must not be reported as panicked; every split frame in \
         every capture would become a gap",
    );
    assert!(
        lanes
            .take_bracket_splits()
            .iter()
            .any(|(tag, _, _)| *tag == TAG),
        "and the split pair must be recorded, or `lw_d*_inb_us` never reaches a capture",
    );
}

/// THE DEFECT, closed: a panic in the deferred compute half marks the bracket
/// that has already closed.
///
/// A SECOND BRACKET UNDER THE SAME TAG IS STAGED FIRST, and it must come back
/// unmarked. `mark_bracket_worker_panicked` marks the LAST bracket carrying the
/// tag, on the reasoning that a tag's other brackets are recorded EARLIER in the
/// frame. With one bracket in the tracker that reasoning is untested.
///
/// PRODUCTION CHANGE THAT WOULD MAKE THIS FAIL: deleting
/// `let _panic_marker = ClosedBracketPanicMarker { lanes, tag };` from
/// `spawn_gated_split` -- which is exactly the state of the code before this
/// lane, and the state in which d81, d47 and d101 published a badged
/// `..._worker_panic_read` where the stream should have carried a
/// `DomainGapReason::WorkerPanic` gap.
///
/// kills: deleting the marker; constructing it after `compute`; dropping the
/// `std::thread::panicking()` test (which would mark EVERY bracket);
/// `mark_bracket_worker_panicked` selecting the FIRST same-tag bracket instead
/// of the last, or marking every same-tag bracket.
#[test]
fn a_panic_in_the_deferred_compute_half_marks_the_closed_split_bracket() {
    const TAG: u16 = 0xD5C;
    let (_fixture, lanes, hour) = coherent_split_lanes();
    // The earlier same-tag bracket. Recorded before the worker runs, so it is
    // the one the marker must NOT touch.
    lanes.record_clock_bracket(TAG, None, None, false);
    let brackets = brackets_after_a_panicking_compute_half(&lanes, TAG, hour);
    let ours: Vec<bool> = brackets
        .iter()
        .filter(|bracket| bracket.tag == TAG)
        .map(|bracket| bracket.worker_panicked)
        .collect();
    assert_eq!(
        ours,
        vec![false, true],
        "exactly the LAST bracket carrying the tag -- this worker's -- must carry the \
         panic; the earlier one is a different read and marking it would gap a domain \
         whose bracket was fine",
    );
}

/// The three domains that reach `spawn_gated_split`, one test each, taken all
/// the way to the gap the wire carries.
///
/// WHY PER DOMAIN AND NOT ONCE ON THE HELPER. The helper is shared, but what a
/// marked bracket MEANS is per domain: `for_each_bracket_covered_domain` decides
/// which domain ids the tag speaks for, and `domain_scoped_tracker_tag_survived`
/// decides which trackers roll back. A tag that covered a neighbour would gap
/// the neighbour too, and a tag that covered nothing would gap nobody -- both
/// are silent failures of exactly this fix.
///
/// WHAT EACH DOMAIN'S OWN REFUSAL PATH THEN DOES with the gap this produces:
///
/// * d81 -- the join mints `compact_rail_replay_worker_panic_read`, badged
///   `COMPACT_STATUS_WORKER_PANIC` with `D81FailureReason::WorkerPanic` and
///   `D81DiagnosticsProvenance::Substituted`. Before this lane that record
///   PUBLISHED on this path, with `Unobserved` witnesses, because its bracket
///   closed coherently; now `replace_rejected_domain_records_with_gaps` replaces
///   it with the `WorkerPanic` gap that carries the bracket's real
///   opening/closing pair. d81's geometry memo is a deliberate carve-out of
///   `reset_trackers_after_rejected_bracket` and still survives.
/// * d47 -- the join mints
///   `compact_characters_leaders_cabinet_worker_panic_read`; the same
///   replacement applies. Its discovery cache is the other deliberate carve-out
///   (lane MX): a byte-witnessed cache the next frame's witness replay decides
///   inside its own bracket, not frame product.
/// * d101 -- the join mints
///   `compact_map_furniture_victory_points_worker_panic_read`, whose snapshot is
///   a refusal, so `settle_compact_victory_point_pin` already rolled the staged
///   census pin back on this path. What the marker adds for d101 is the gap
///   itself -- the wire saying `worker_panic` instead of carrying a refusal
///   record -- plus the tag-scoped reset of `map_furniture_victory_points`,
///   which is a domain-scoped tracker and does NOT survive a rejected bracket.
///
/// PRODUCTION CHANGE THAT WOULD MAKE THIS FAIL: the same deletion as above, or
/// giving one of these tags a `for_each_bracket_covered_domain` arm that names a
/// different domain.
///
/// kills: the marker deleted; a covered-domain arm added or removed for these
/// three tags; `rejected_bracket_domains` no longer mapping `WorkerPanic`
/// brackets to `DomainGapReason::WorkerPanic`.
#[test]
fn a_panicked_deferred_half_gaps_each_split_domain_through_its_own_refusal_path() {
    use compact_contract::domain_gap::DomainGapReason;

    for (tag, domain, label) in [
        (
            COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
            COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
            "d81 rail_replay_state_live",
        ),
        (
            COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
            COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
            "d47 characters_leaders_cabinet_live",
        ),
        (
            COMPACT_DOMAIN_MAP_FURNITURE_VICTORY_POINTS_LIVE,
            COMPACT_DOMAIN_MAP_FURNITURE_VICTORY_POINTS_LIVE,
            "d101 map_furniture_victory_points_live",
        ),
    ] {
        let (_fixture, lanes, hour) = coherent_split_lanes();
        let brackets = brackets_after_a_panicking_compute_half(&lanes, tag, hour);
        let ours: Vec<&DomainClockBracket> =
            brackets.iter().filter(|b| b.tag == tag).collect();
        assert_eq!(ours.len(), 1, "{label}: the worker records one bracket");
        assert!(
            ours[0].worker_panicked,
            "{label}: the closed bracket must carry the deferred half's panic",
        );
        assert!(
            ours[0].opening.is_some() && ours[0].closing.is_some(),
            "{label}: and it must still carry the real clock pair the gap reports",
        );

        let rejected =
            rejected_bracket_domains(&brackets, hour, HOUR_LABEL_MISMATCH_POLICY);
        assert_eq!(
            rejected
                .iter()
                .map(|(domain, (reason, _, _))| (*domain, *reason))
                .collect::<Vec<_>>(),
            vec![(domain, DomainGapReason::WorkerPanic)],
            "{label}: exactly this domain is gapped, and the gap names the panic",
        );

        let rejected_ids: BTreeSet<u16> = rejected.keys().copied().collect();
        assert!(
            !domain_scoped_tracker_tag_survived(tag, &rejected_ids),
            "{label}: a panicked bracket must not leave a domain-scoped tracker standing; \
             the next frame would delta against an hour the stream never carried",
        );

        // AND THE WIRE. Everything above is reconciliation's INPUT; this is the
        // byte that reaches the stream. The frame's record for this domain --
        // standing in for the badged `..._worker_panic_read` substitute the join
        // minted -- must come back a `DOMAIN_GAP` naming the panic, carrying the
        // bracket's real tick witnesses (Codex round 1, blocking 2: the earlier
        // version of this test stopped at `rejected_bracket_domains` and would
        // have passed with the frame's call to
        // `replace_rejected_domain_records_with_gaps` deleted).
        let mut records = Vec::new();
        append_compact_record(
            &mut records,
            1,
            hour,
            domain,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &[9u8; 8],
        );
        let (_next, replaced) = replace_rejected_domain_records_with_gaps(
            &mut records,
            1,
            hour,
            &brackets,
            &mut BTreeMap::new(),
            HOUR_LABEL_MISMATCH_POLICY,
        )
        .expect("a worker-panic refusal must be encodable");
        assert_eq!(
            replaced,
            BTreeSet::from([domain]),
            "{label}: this domain and no other loses its records",
        );
        let header = compact_contract::parse_compact_record_header(&records)
            .expect("the replacement is a well-formed record");
        assert_eq!(
            header.record_type,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
            "{label}: the badged substitute must not survive to the wire",
        );
        let gap = compact_contract::domain_gap::DomainGapRecordV1::decode(
            &records[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()],
        )
        .expect("the replacement must satisfy the H4DG grammar");
        assert_eq!(
            gap.reason,
            DomainGapReason::WorkerPanic,
            "{label}: and the gap must name the panic, not some other refusal",
        );
        assert_eq!(
            gap.target_hour_raw, hour,
            "{label}: the lost hour is the one the frame was publishing under",
        );
        assert_eq!(
            (gap.tick_before, gap.tick_after),
            (
                clock_tick_witness(ours[0].opening.expect("the bracket opened")),
                clock_tick_witness(ours[0].closing.expect("the bracket closed")),
            ),
            "{label}: the witnesses are this bracket's own observations, not the \
             substitute's `Unobserved` zeros",
        );
    }
}

/// The one split domain whose tracker is DOMAIN-SCOPED loses its delta pin when
/// its deferred half panics.
///
/// SPLIT FROM THE TEST ABOVE because this is the tracker's fate, not the wire's,
/// and because `domain_scoped_tracker_tag_survived` is only the PREDICATE --
/// this calls `reset_trackers_after_rejected_bracket` itself, which is what the
/// frame calls (Codex round 1, blocking 2: the earlier version asserted the
/// predicate and would have passed with the reset deleted).
///
/// WHY ONLY d101 OF THE THREE. `for_each_domain_scoped_tracker` lists
/// `map_furniture_victory_points`; d47's discovery cache (lane MX) and d81's
/// geometry memo are deliberate carve-outs restored by name after the wipe, and
/// their survival is already pinned by
/// `d47_cache_survives_refusal_tests::d47s_own_rejection_keeps_roots_witness_and_walk`
/// and its siblings. Re-asserting them here would be a second copy of somebody
/// else's contract.
///
/// d101 ALSO HAS ITS OWN HALF OF THIS, and it already worked: `observe` stages
/// the census as `uncommitted_pin` and `settle_compact_victory_point_pin` rolls
/// it back for any refusal, which the panic substitute is. What the marker adds
/// is the reset below, which returns the WHOLE tracker -- `previous`,
/// `baselined`, `last_full_census_hour_raw` -- to its default so nothing derived
/// from the panicked hour survives.
///
/// kills: `map_furniture_victory_points` moved out of
/// `for_each_domain_scoped_tracker`; the reset no longer wiping a rejected
/// domain's pin.
#[test]
fn a_panicked_d101_loses_the_census_pin_its_observe_half_advanced() {
    let mut trackers = CompactPerfGateTrackers::default();
    trackers.map_furniture_victory_points.baselined = true;
    trackers
        .map_furniture_victory_points
        .last_full_census_hour_raw = 4_242;

    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_MAP_FURNITURE_VICTORY_POINTS_LIVE].into();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);

    assert!(
        !trackers.map_furniture_victory_points.baselined,
        "d101's census baseline describes an hour the stream did not carry, so it must be \
         gone; keeping it makes every later delta patch a census the fold never received",
    );
    assert_eq!(
        trackers
            .map_furniture_victory_points
            .last_full_census_hour_raw,
        0,
        "and the keyframe cadence must restart from the reset, not from the lost hour",
    );
}

/// Without the marker the very same worker leaves the frame with NOTHING to
/// reject -- the shape this lane removed.
///
/// The negative control for the test above. `rejected_bracket_domains` is fed
/// the bracket a pre-fix worker would have recorded (same tag, same clock pair,
/// `worker_panicked: false`) and returns empty, so the badged substitute would
/// have published. This is the fault, written down, rather than described.
///
/// kills: `local_bracket_verdict` deciding `WorkerPanic` from anything other
/// than the flag (which would make the marker redundant and this assertion
/// wrong in the other direction).
#[test]
fn an_unmarked_bracket_rejects_nothing_which_is_what_the_marker_repairs() {
    let (_fixture, lanes, hour) = coherent_split_lanes();
    let tag = COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE;
    let out = thread::scope(|scope| {
        spawn_gated_split(
            scope,
            None,
            &lanes,
            tag,
            hour,
            |_bracket_opening| 3u32,
            |observed: u32| observed,
            |observed: &u32| *observed,
            |_refusal, _observed| REFUSAL_SENTINEL,
            |out, _observed_hour_raw| out,
        )
        .join()
        .expect("a clean worker joins cleanly")
    });
    assert_eq!(out, 3, "the coherent arm ran");
    let clean = lanes.take_clock_brackets();
    assert!(
        rejected_bracket_domains(&clean, hour, HOUR_LABEL_MISMATCH_POLICY).is_empty(),
        "a clean bracket rejects nothing -- so a worker that panicked and left one behind \
         published its badged substitute, which is precisely the defect",
    );
    let mut panicked = clean;
    for bracket in &mut panicked {
        if bracket.tag == tag {
            bracket.worker_panicked = true;
        }
    }
    assert!(
        rejected_bracket_domains(&panicked, hour, HOUR_LABEL_MISMATCH_POLICY)
            .contains_key(&tag),
        "and the single flag the marker sets is the whole of the difference",
    );
}

/// `spawn_gated_publishing` defers too, and needs no marker because its deferred
/// half cannot unwind the worker.
///
/// WHY IT IS NOT THE SAME TRADE. The encode that helper moved used to run on the
/// MAIN thread in the publish tail, so a bug in it took the whole frame; lane RY
/// caught the unwind precisely so that stayed true. There is no closed-bracket
/// panic to carry because there is no panic that reaches the bracket at all.
/// This test ties the two facts together: delete the `catch_unwind` and both
/// this and `every_bracket_helper_that_defers_work_past_its_guard_is_classified`
/// fail, instead
/// of the accounting quietly going wrong.
///
/// kills: removing `std::panic::catch_unwind` from `spawn_gated_publishing`;
/// letting its encode panic escape the worker.
#[test]
fn the_publishing_helper_catches_instead_of_marking() {
    const TAG: u16 = 0xD5D;
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let published = thread::scope(|scope| {
        spawn_gated_publishing(
            scope,
            None,
            &lanes,
            TAG,
            || 5u32,
            |_read: &u32| -> Vec<u8> { panic!("a latent bug in the deferred encode") },
        )
        .join()
        .expect("the encode's panic is caught, so the worker joins cleanly")
    });
    let (read, payload) = published.split();
    assert_eq!(read, 5, "the read still comes back");
    assert!(
        payload.is_none(),
        "and the caught encode leaves no payload, so the tail encodes inline exactly as \
         it did before the helper existed",
    );
    let brackets = lanes.take_clock_brackets();
    let ours = brackets
        .iter()
        .find(|bracket| bracket.tag == TAG)
        .expect("the worker recorded its bracket");
    assert!(
        !ours.worker_panicked,
        "no unwind reached the worker, so there is nothing for a marker to record and the \
         bracket must stay clean",
    );
}

/// The `spawn_gated_split` call sites are these three domains, and no others.
///
/// Nothing pinned WHICH before this. `domain_scoped_reset_tests::reader_call_spans`
/// knows the spelling (so a split reader's tracker borrow is attributed to a
/// reader call rather than read as `OUTSIDE-EVERY-READER-CALL`) but says nothing
/// about the roster, and the per-domain gap test above is only complete if the
/// roster it walks is the roster the fan-out actually spawns.
///
/// kills: a fourth split domain added without a gap test; one of the three
/// reverted to `spawn_gated` (which would put its compute back in the bracket
/// and change every frame's `lw_d*_inb_us`).
#[test]
fn the_split_helper_serves_exactly_d81_d47_and_d101() {
    let source = super::collector_source_lf();
    // OVER THE BLANKED COPY, and matching the call regardless of how it is
    // formatted. The first spelling required the literal `spawn_gated_split(`
    // followed by a newline, and Codex round 1 was right that a fourth site
    // written `spawn_gated_split(scope, ...)` would have been invisible to it.
    let blanked = blank_comments_and_literals(&source);
    let bytes = blanked.as_bytes();
    let needle = "spawn_gated_split(";
    let mut tags: Vec<String> = Vec::new();
    let mut from = 0usize;
    while let Some(at) = blanked[from..].find(needle) {
        let call_start = from + at;
        let open = call_start + needle.len() - 1;
        from = open + 1;
        // The declaration is `fn spawn_gated_split<`, which does not match this
        // needle, but a future `fn spawn_gated_split(` would; skip it by name.
        if blanked[..call_start].trim_end().ends_with("fn") {
            continue;
        }
        let close = matching_delimiter(bytes, open, b'(', b')')
            .expect("a `spawn_gated_split(` call must close");
        let tag = blanked[open + 1..close]
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with("COMPACT_DOMAIN_"))
            .expect("a split spawn passes its domain tag")
            .trim_end_matches(',')
            .to_string();
        tags.push(tag);
    }
    tags.sort();
    assert_eq!(
        tags,
        vec![
            "COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE".to_string(),
            "COMPACT_DOMAIN_MAP_FURNITURE_VICTORY_POINTS_LIVE".to_string(),
            "COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE".to_string(),
        ],
        "the split readers are d47, d101 and d81 -- add a domain here and it needs its own \
         row in `a_panicked_deferred_half_gaps_each_split_domain_through_its_own_refusal_path`",
    );
}

/// THE COUNTRY CHILD's post-bracket tail marks its closed bracket too, and d13's
/// advanced wire baseline is rolled back because of it.
///
/// THE WHOLE SEQUENCE, on the real d13 tag, because the marker alone is not the
/// deliverable (Codex round 2, blocking 1: an earlier version panicked on a
/// synthetic tag with an empty tail, and preserving `tech_country_delta` through
/// `reset_trackers_after_rejected_bracket` would have left it green while the
/// central d13 defect was reopened). The tail here does what
/// `country_bundle_publication_records(read, .., Some(tech_delta))` does -- it
/// ADVANCES the delta tracker's baseline -- and then panics, exactly as a latent
/// bug in the encode after that advance would.
///
/// The four steps the frame then takes, all asserted:
///
/// 1. the child's closed bracket carries `worker_panicked`;
/// 2. `rejected_bracket_domains` gaps d13 and only d13
///    (`for_each_bracket_covered_domain(COMPACT_DOMAIN_TECH)` is the `own`
///    arm -- lane LE split the tag that used to cover d12 as well);
/// 3. `replace_rejected_domain_records_with_gaps` replaces the record the join's
///    substitute would have published with a `WorkerPanic` gap;
/// 4. `reset_trackers_after_rejected_bracket` takes the advanced baseline away.
///    It also calls `d13_tech_change_gate_lane::gate::note_pin_lost()` on the way
///    -- which is the accounting the lost XUI09 pins are owed -- and that is NOT
///    asserted here: it sets a process-global latch that a parallel test could
///    observe, and the wipe is what this test is for.
///
/// PRODUCTION CHANGE THAT WOULD MAKE THIS FAIL: deleting
/// `let _panic_marker = ClosedBracketPanicMarker { lanes, tag };` from
/// `country_child_bracket`, moving it below the `after_bracket` call, or
/// carrying `tech_country_delta` through the reset.
///
/// kills: the marker deleted or moved in `country_child_bracket`; the tail
/// hoisted back into the caller (which
/// `every_country_child_publishes_through_the_bracket_helper` also catches);
/// `tech_country_delta` added to a carry-over list; `COMPACT_DOMAIN_TECH` given
/// a covered-domain arm that names another domain.
#[test]
fn a_panic_in_the_tech_childs_publish_tail_gaps_d13_and_takes_its_baseline() {
    use compact_contract::domain_gap::DomainGapReason;

    let (_fixture, lanes, hour) = coherent_split_lanes();
    // The frame's tracker as the tech child receives it: COLD, so the advance
    // below is observable and the test cannot pass on a tail that never ran.
    let mut trackers = CompactPerfGateTrackers::default();
    assert!(!trackers.tech_country_delta.baseline_seeded);

    let joined = thread::scope(|scope| {
        let tech_delta = &mut trackers.tech_country_delta;
        scope
            .spawn(|| {
                country_child_bracket(
                    &lanes,
                    COMPACT_DOMAIN_TECH,
                    || 1u32,
                    |_read: &mut u32| {
                        // WHAT THE ENCODE DOES BEFORE IT CAN FAIL. D13's pin
                        // advances once per frame during the payload build; this
                        // is that advance, standing in for the whole of
                        // `country_bundle_publication_records`.
                        tech_delta.baseline_seeded = true;
                        panic!("a latent bug in the publication encode, after the pin advanced");
                    },
                )
            })
            .join()
    });
    assert!(
        joined.is_err(),
        "the panic must still propagate to the join, where the bundle substitutes its \
         badged zero-row read",
    );
    assert!(
        trackers.tech_country_delta.baseline_seeded,
        "THE HARM, before the repair: the pin the encode advanced survives the unwind, \
         because it lives on the frame's tracker and not on the worker",
    );

    let brackets = lanes.take_clock_brackets();
    let ours: Vec<&DomainClockBracket> = brackets
        .iter()
        .filter(|bracket| bracket.tag == COMPACT_DOMAIN_TECH)
        .collect();
    assert_eq!(ours.len(), 1, "the child records one bracket");
    assert!(
        ours[0].worker_panicked,
        "the closed bracket must carry the tail's panic, or nothing below happens at all",
    );

    let rejected = rejected_bracket_domains(&brackets, hour, HOUR_LABEL_MISMATCH_POLICY);
    assert_eq!(
        rejected
            .iter()
            .map(|(domain, (reason, _, _))| (*domain, *reason))
            .collect::<Vec<_>>(),
        vec![(COMPACT_DOMAIN_TECH, DomainGapReason::WorkerPanic)],
        "d13 and no sibling is gapped, and the gap names the panic",
    );

    let mut records = Vec::new();
    append_compact_record(
        &mut records,
        1,
        hour,
        COMPACT_DOMAIN_TECH,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &[9u8; 8],
    );
    let (_next, replaced) = replace_rejected_domain_records_with_gaps(
        &mut records,
        1,
        hour,
        &brackets,
        &mut BTreeMap::new(),
        HOUR_LABEL_MISMATCH_POLICY,
    )
    .expect("a worker-panic refusal must be encodable");
    assert_eq!(replaced, BTreeSet::from([COMPACT_DOMAIN_TECH]));
    let header = compact_contract::parse_compact_record_header(&records)
        .expect("the replacement is a well-formed record");
    assert_eq!(
        header.record_type,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        "the badged substitute must not survive to the wire",
    );
    let gap = compact_contract::domain_gap::DomainGapRecordV1::decode(
        &records[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()],
    )
    .expect("the replacement must satisfy the H4DG grammar");
    assert_eq!(gap.reason, DomainGapReason::WorkerPanic);

    let rejected_ids: BTreeSet<u16> = rejected.keys().copied().collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected_ids);
    assert!(
        !trackers.tech_country_delta.baseline_seeded,
        "the pin the panicking encode advanced must be gone: the wire carries a gap for \
         this hour, so a producer that kept the baseline would send the next hour as a \
         patch against a block the fold never received",
    );
}

/// And a country child that completes leaves its bracket clean.
///
/// The other half of the marker's contract at this site: a `Drop` that marked
/// unconditionally would gap all eight country domains in every frame of every
/// capture.
///
/// kills: `ClosedBracketPanicMarker::drop` marking without testing
/// `std::thread::panicking()`; `country_child_bracket` losing its tail call.
#[test]
fn a_clean_country_child_leaves_its_bracket_unmarked() {
    const TAG: u16 = 0xC14;
    let lanes = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    let out = thread::scope(|scope| {
        scope
            .spawn(|| {
                country_child_bracket(
                    &lanes,
                    TAG,
                    || 1u32,
                    |read: &mut u32| {
                        *read += 41;
                    },
                )
            })
            .join()
            .expect("a clean child joins cleanly")
    });
    assert_eq!(out, 42, "the tail's mutation is what the child returns");
    let brackets = lanes.take_clock_brackets();
    let ours = brackets
        .iter()
        .find(|bracket| bracket.tag == TAG)
        .expect("the child recorded its bracket");
    assert!(
        !ours.worker_panicked,
        "a child that completed must not be reported as panicked",
    );
}
