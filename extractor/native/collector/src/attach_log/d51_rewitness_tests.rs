//! Lane SE2: the collector half of the empty-scope re-witness ruling.
//!
//! Neither collector site can be driven by a fixture: `domain51_variable_scope`
//! and `domain51_flag_scope` reach memory through the live-process primitives
//! (`read_u64_stable`, `domain51_stable_flag_header`), not through the
//! `MemoryRead` their caller passes. So these read the source. The lane crate's
//! half of the ruling IS fixture-driven — see `country_variables_extractor`'s
//! `MovingHeaderFixture` and `ReadCountingFixture` tests.
//!
//! What they pin is not "these sites lost a re-witness". It is the interval
//! rule the ruling actually turns on:
//!
//! * a re-witness is worth its reads exactly when SOMETHING WAS READ inside its
//!   interval, because what it rules out is a structure moving under a read
//!   that already happened;
//! * `domain51_flag_scope` contains two textually identical HEADER
//!   re-witnesses. The one in the empty arm had nothing between it and the
//!   header sample and is gone; the one after the row loop closes over that
//!   loop and stays. A change that removed both would satisfy any test written
//!   as "the empty arm has no re-witness" and would be wrong;
//! * the ROOT re-witnesses are a different interval entirely — theirs opens
//!   when the owner's pointer is first read, before the RTTI type-name walk and
//!   before the container head. Those reads happen for an empty container too,
//!   so both root re-witnesses are unconditional. Codex round 1 caught this
//!   lane gating them on the walk, which turned "the owner's pointer moved
//!   while we were reading its container" from a refusal into an accepted empty
//!   scope.

// No `use super::*;`: every assertion in this file is over the collector's
// SOURCE TEXT, so nothing here resolves through the parent module and the
// import was warned as unused once the round 1-7 rewrites finished turning
// these into text assertions. The sibling module `d51_rtti_census_tests`
// reaches this file's two helpers by path, which needs no import here.

/// The collector source, newline-normalised so these assertions do not depend
/// on the checkout's line endings.
fn collector_source() -> String {
    include_str!("../attach_log.rs").replace("\r\n", "\n")
}

/// The text of one top-level function, from its signature to the closing brace
/// in column zero.
fn top_level_function<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} is not in attach_log.rs"));
    let rest = &source[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("{signature} does not close at column zero"));
    &rest[..end]
}

fn flag_scope() -> String {
    top_level_function(
        &collector_source(),
        "fn domain51_flag_scope<R: domain51::MemoryRead>(",
    )
    .to_string()
}

fn variable_scope() -> String {
    top_level_function(
        &collector_source(),
        "fn domain51_variable_scope<R: domain51::MemoryRead>(",
    )
    .to_string()
}

/// Where the flag scope's empty-manager arm starts. Spelled as the predicate
/// call, because the arm asking the shared predicate IN RELEASE is half of what
/// this file exists to hold: a `debug_assert!` beside an inline `entry_count ==
/// 0` would compile the guard out of the shipped collector (Codex round 1,
/// finding 2).
const EMPTY_ARM_OPENS: &str = "    if !domain51::rewitness_owed(entry_count as usize) {";
/// ...and where it ends: the first line of the measured path.
const MEASURED_PATH_OPENS: &str = "    if !is_map_control_heap_pointer(base)";

/// The flag scope's empty arm, from the predicate to the measured path.
fn flag_scope_empty_arm(body: &str) -> &str {
    let start = body
        .find(EMPTY_ARM_OPENS)
        .expect("the flag scope decides emptiness by asking the shared predicate");
    let arm = &body[start..];
    let end = arm
        .find(MEASURED_PATH_OPENS)
        .expect("the empty arm is followed by the measured path");
    &arm[..end]
}

/// Every spelling of "this code reads the game's memory" that could stand in
/// the empty arm. A re-witness reintroduced through a differently named helper
/// would still have to reach memory through one of these.
const READ_SPELLINGS: &[&str] = &[
    "read_u64_stable",
    "read_u32_stable",
    "read_i32_stable",
    "read_u8_stable",
    "read_scalar",
    "read_bytes",
    "domain51_stable_flag_header",
    "domain51_flag_row_entry",
    "domain51_flag_row_vtables",
    "rtti_type_name",
    "mem.",
];

/// The same text with its comment lines dropped. The comments around these
/// arms NAME the reads they are explaining, and a spelling that appears only in
/// prose is not a read.
pub(super) fn without_comments(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn count_reads(text: &str) -> usize {
    let code = without_comments(text);
    READ_SPELLINGS
        .iter()
        .map(|spelling| code.matches(spelling).count())
        .sum()
}

/// The brace depth at which `needle` sits inside a top-level function's text,
/// counting from the function's own opening brace. **Depth 1 is the function's
/// own statement level**; an enclosing `if`, `match`, `for`, `while` or closure
/// raises it.
///
/// This, and not indentation, is what "unconditional" means. Rust ignores
/// whitespace, so a wrapper written without re-indenting its contents leaves a
/// pinned literal byte-for-byte intact (Codex round 5, finding 1).
///
/// Only braces that are CODE are counted. A textual count is not enough
/// either - `if owner_id != 0 { // }` closes nothing in Rust and closes the
/// block in a naive counter, which is a wrapper that no pinned literal and no
/// naive depth would see (Codex round 6, finding 1). So this skips line
/// comments, block comments and string literals, and REFUSES to guess at
/// anything it does not lex: an apostrophe would be a char literal or a
/// lifetime and it does not tell them apart, so it asserts there is none rather
/// than mis-count one.
///
/// Callers pass text that has already been through [`without_comments`], which
/// drops whole comment LINES - that is what keeps the apostrophes out of ordi-
/// nary English prose. What remains for this scanner is the inline case, a
/// comment that shares its line with code, which is exactly the evasion above.
/// An apostrophe inside one of those trips the assertion and fails the test
/// loudly, which is the safe direction.
///
/// KNOWN LIMIT, recorded rather than closed: a raw string (`r#".."#`) is lexed
/// as an ordinary one, so one containing an internal quote and a closing brace
/// would mis-count. No scanned function contains a raw string, and writing one
/// there is not an accidental edit (Codex round 8).
pub(super) fn statement_depth(body: &str, needle: &str) -> usize {
    assert!(
        !body.contains('\''),
        "this scanner does not lex char literals or lifetimes; the d51 \
         functions it is used on contain neither, and one appearing means this \
         helper needs widening rather than trusting"
    );
    let at = body
        .find(needle)
        .unwrap_or_else(|| panic!("not in this function:\n{needle}"));
    let bytes = body.as_bytes();
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < at {
        let two = (bytes[index], bytes.get(index + 1).copied());
        match two {
            (b'/', Some(b'/')) => {
                while index < at && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            (b'/', Some(b'*')) => {
                // NESTED, because Rust's block comments nest: stopping at the
                // first `*/` reads `/* outer /* inner */ } */` as closing early
                // and hands the brace after it back to the counter (Codex
                // round 7, finding 3).
                let mut nesting = 1usize;
                index += 2;
                while index + 1 < at && nesting > 0 {
                    match (bytes[index], bytes[index + 1]) {
                        (b'/', b'*') => {
                            nesting += 1;
                            index += 2;
                        }
                        (b'*', b'/') => {
                            nesting -= 1;
                            index += 2;
                        }
                        _ => index += 1,
                    }
                }
            }
            (b'"', _) => {
                index += 1;
                while index < at && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                index += 1;
            }
            (b'{', _) => {
                depth = depth.saturating_add(1);
                index += 1;
            }
            (b'}', _) => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            _ => index += 1,
        }
    }
    depth
}

/// The flag scope's post-walk re-witness, pinned WITH its indentation.
///
/// Both halves and the refusal spelling. The literal says WHAT the re-witness
/// is; `statement_depth` says that nothing encloses it. Neither alone is
/// enough: a counted substring survives being wrapped, and so does a pinned
/// literal if the wrapper does not re-indent.
const POST_WALK_REWITNESS: &str =
    "    if read_u64_stable(root_addr).map(|value| value as usize) != Some(manager)
        || domain51_stable_flag_header(manager) != Some((base, slot_count, entry_count))
    {
        return domain51_unmeasured_flag_scope(scope_kind, owner_id);
    }
";

/// The variable scope's root re-witness, pinned the same way and for the same
/// reason.
const VARIABLE_ROOT_REWITNESS: &str = "    if read_u64_stable(root_addr) != Some(container) {
        return domain51_unmeasured_variable_scope(scope_kind, owner_id);
    }
";

/// The empty arm, in full, with its comments stripped.
///
/// Held as a LITERAL rather than as a list of things that must not appear,
/// because a list can always be stepped around: Codex round 2 pointed out that
/// a re-witness moved into a freshly named top-level helper and called from
/// this arm satisfies any spelling-based check while putting the removed header
/// re-read straight back.
///
/// Fourteen lines of safety-critical control flow are worth pinning exactly. If
/// a legitimate change to this arm makes this test fail, the right response is
/// to read the new arm, decide whether every read in it is owed, and update
/// this constant deliberately.
///
/// It ends at the closing brace with NO trailing newline, because the text it
/// is compared against comes through [`without_comments`], which is built on
/// `str::lines` and so does not keep one. Every line including that brace is
/// still pinned; only the slice boundary differs. (The landing gate found this
/// on first execution: the two sides were identical but for that one byte.)
const EMPTY_ARM_CODE: &str = "    if !domain51::rewitness_owed(entry_count as usize) {
        if slot_count > MAX_FLAG_ROWS {
            return domain51_unmeasured_flag_scope(scope_kind, owner_id);
        }
        if read_u64_stable(root_addr).map(|value| value as usize) != Some(manager) {
            return domain51_unmeasured_flag_scope(scope_kind, owner_id);
        }
        return domain51::FlagScopeRead {
            scope_kind,
            owner_id,
            status: domain51::ScopeReadStatus::MeasuredEmpty,
            entries: Vec::new(),
        };
    }";

#[test]
fn the_empty_flag_manager_arm_re_reads_the_root_and_nothing_else() {
    // Both halves of the ruling in one assertion, and it is the one that would
    // have caught this lane's first cut.
    //
    // The HEADER re-witness is gone: nothing was read between
    // `domain51_stable_flag_header` and it. The ROOT re-witness stays: its
    // interval opened at the owner's pointer read and contains the RTTI walk
    // and the header sample.
    //
    // kills: removing the root re-read as well; reintroducing the header
    // re-read directly OR through a helper of any name, which is what the
    // literal below catches and a spelling list does not.
    let body = flag_scope();
    let empty_arm = flag_scope_empty_arm(&body);

    assert_eq!(
        without_comments(empty_arm),
        EMPTY_ARM_CODE,
        "the empty arm is pinned in full"
    );
    // And the arm itself stands at the function's own level - an `if` around
    // the whole arm would leave its text untouched while sending empty scopes
    // down the measured path instead (Codex round 5).
    assert_eq!(
        statement_depth(&without_comments(&body), EMPTY_ARM_OPENS),
        1,
        "nothing may enclose the empty arm"
    );
    // The same property said the readable way, so a failure of the literal
    // above is easy to classify. On its own this one is NOT sufficient - a
    // helper with a new name defeats it - which is why the literal leads.
    assert_eq!(
        count_reads(empty_arm),
        1,
        "the empty arm performs exactly one read - the root re-witness"
    );
    assert_eq!(
        without_comments(empty_arm)
            .matches("read_u64_stable(root_addr).map(|value| value as usize) != Some(manager)")
            .count(),
        1,
        "and that one read is the root re-witness, spelled as a REFUSAL: \
         flipping `!=` to `==` would accept exactly the scopes it exists to \
         refuse"
    );
}

#[test]
fn the_flag_manager_header_re_witness_after_the_row_loop_survives() {
    // THE KILL for this file. The surviving header re-witness is textually
    // identical to the one that was removed, so a change that deleted "the
    // re-witness in domain51_flag_scope" without asking what it closes over
    // would take this one too - and a walked row set would then be published
    // against a header that no longer describes it.
    let body = flag_scope();
    let empty_arm_end = body
        .find(MEASURED_PATH_OPENS)
        .expect("the empty arm is followed by the measured path");
    let measured_path = without_comments(&body[empty_arm_end..]);

    // What it is, and that nothing encloses it. A counted substring survives
    // being wrapped in an outer guard, and so does a pinned literal when the
    // wrapper does not re-indent: `if owner_id != 0 {` around this preserves
    // every string and every ordering while letting the global scope publish
    // rows read against a header that has since moved (Codex rounds 4 and 5).
    // The literal is the content; the depth is the property.
    assert_eq!(
        measured_path.matches(POST_WALK_REWITNESS).count(),
        1,
        "the row loop's re-witness must still be here, refusing on \
         disagreement rather than requiring it"
    );
    // Depth is counted over the WHOLE function, from its signature, because
    // that is where depth 1 means "the function's own statement level".
    assert_eq!(
        statement_depth(&without_comments(&body), POST_WALK_REWITNESS),
        1,
        "and it must stand at the function's own level, with nothing enclosing it"
    );
    // And it must come AFTER the loop it closes over, not before it.
    let loop_start = measured_path
        .find("for idx in 0..entry_count {")
        .expect("the row loop");
    let rewitness = measured_path
        .find("domain51_stable_flag_header(manager)")
        .expect("the re-witness");
    assert!(
        loop_start < rewitness,
        "a re-witness that runs before the walk it witnesses is not one"
    );
}

#[test]
fn the_variable_scopes_root_re_read_is_unconditional() {
    // The third site, and the one Codex round 1 sent back. This re-witness does
    // not close over the row walk; it closes over everything since `container`
    // was read out of the owner - the RTTI type-name walk and the container
    // head sample included. Those reads happen for an empty container too.
    //
    // kills: deleting it (count goes to zero); gating it on
    // `rewitness_owed` again, or on any other condition, which is what this
    // lane did first and what turned an owner-pointer swap under an empty
    // container into an accepted scope.
    let body = without_comments(&variable_scope());

    // Content and depth, for the reason given on `statement_depth`: an outer
    // `if owner_id != 0 {` keeps every counted string AND every pinned literal
    // intact while quietly exempting the global scope.
    assert_eq!(
        body.matches(VARIABLE_ROOT_REWITNESS).count(),
        1,
        "the root re-witness must still be here, spelled as a refusal"
    );
    assert_eq!(
        statement_depth(&body, VARIABLE_ROOT_REWITNESS),
        1,
        "and it must stand at the function's own level, with no condition in \
         front of it and nothing around it"
    );
    assert_eq!(
        body.matches("rewitness_owed").count(),
        0,
        "the walk predicate has no business here: this interval opened before \
         the walk did"
    );
}

#[test]
fn the_flag_scopes_emptiness_test_is_the_shared_predicate_and_not_a_debug_assertion() {
    // One place to be right. `rewitness_owed` lives in the lane crate and is
    // tested there against fixtures; if a collector site grew its own inline
    // `entry_count == 0` and kept the predicate only in a `debug_assert!`, the
    // shipped collector would decide emptiness somewhere the tests never look
    // (Codex round 1, finding 2).
    let body = flag_scope();

    assert!(
        body.contains(EMPTY_ARM_OPENS),
        "the empty arm must be spelled as the predicate, in release"
    );
    assert_eq!(
        body.matches("debug_assert!(!domain51::rewitness_owed")
            .count(),
        0,
        "a debug assertion is not a guard; it disappears from the shipped build"
    );
    assert_eq!(
        body.matches("entry_count == 0").count(),
        0,
        "and the inline emptiness test it replaced must not come back beside it"
    );
}
