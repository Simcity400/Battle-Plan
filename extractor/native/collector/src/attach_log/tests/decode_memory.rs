//! The shared decode surface, pinned against the memory it reads from.
//!
//! # What lane GB changed and what these hold
//!
//! Task #167. The owner-ratified copy-only-brackets architecture requires
//! every classification, identity read, string decode and admission test to run
//! AFTER the bracket, over the hour-labelled copy. Lane FW built d47's copy
//! plan and its view adapter and then stopped, because the decode side of every
//! one of the 73 domains bottoms out in FREE FUNCTIONS over the live process:
//! `read_scalar`, `read_bytes_with_reason` and `read_into`, and the composed
//! readers built on them (`ObjectSnapshot::read`, the `read_msvc_string`
//! family, the `snapshot_*_or_read` field helpers).
//!
//! Those composed readers now take a [`DecodeMemory`](super::super::DecodeMemory),
//! and the shipped names are one-line delegations that pass `&LIVE_MEMORY`.
//! The three primitives themselves are UNTOUCHED — `LiveMemory` routes to them
//! by name — so the live path is the shipped code and not a re-implementation
//! of it.
//!
//! # The obligation
//!
//! "No behaviour change on shipped paths" is not established by the delegation
//! existing. Two things have to hold and neither implies the other:
//!
//! 1. **The shipped name has no body of its own.** Source law, because "this
//!    function computes nothing" is not observable through its return value.
//!    A wrapper that grew a guard would make the live reader and the generic
//!    one two different readers, and every behavioural test would go on
//!    passing because they all call the wrapper.
//! 2. **The generic body decides what the pre-migration body decided.** A
//!    behavioural A/B over staged headers, comparing the shipped name against
//!    the generic form driven by `LiveMemory` — over the same fixture, in the
//!    same frame — INCLUDING the refusals, spelled out, because the refusal
//!    reason is what `domain47_portrait_reader_reason` publishes on the wire.
//!
//! And one more, which is the point of the whole lane: the generic body must
//! decide the same things over a SNAPSHOT VIEW. That proof needs a copy plan
//! and lives beside lane FW's, in `d47_cost_bench.rs`.
//!
//! # What is NOT here
//!
//! The three primitives' own guards (`range_confirmed_good`, `copy_selected`,
//! the read-cost probe). They did not move, they are not reachable through a
//! view, and `attach_log/tests.rs` already holds them.

use super::*;

use super::state_fixture::FixtureRegion;

// ==================================================================
// 1. SOURCE LAW — the shipped names are delegations and the generic
//    bodies do not reach past their injected memory.
// ==================================================================

/// EVERY MIGRATED READER'S SHIPPED NAME IS ONE CALL AND NOTHING ELSE.
///
/// This is lane FW's pinning pattern applied to the whole migrated surface:
/// FW pinned `domain47_country_leader_identity_with` against the production
/// spelling so that its A/B compared the copy against the SHIPPED decode rather
/// than against a second implementation of it. Here the relationship is
/// stronger — the shipped name IS the generic body, reached with
/// `&LIVE_MEMORY` — and this is what keeps it so.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: giving any of these names a
/// body of its own (an early return, an extra guard, a second read, a different
/// memory), or renaming a generic form without updating its live wrapper. Any
/// of those turns "the live path is the shipped code" from a fact into a claim.
#[test]
fn the_live_forms_are_pure_delegations_to_the_generic_bodies() {
    // `collector_source_lf` and not `include_str!`: these patterns span line
    // breaks and this repo checks the file out with CRLF.
    let source = collector_source_lf();
    for delegation in [
        // The object window. Every one of the ~30 `ObjectSnapshot::<_>::read`
        // call sites in this collector goes through this.
        "    fn read(addr: usize) -> Option<ObjectSnapshot<N>> {\n        Self::read_in(&LIVE_MEMORY, addr)\n    }",
        // The by-address MSVC string body, which the ~40 domains that read a
        // token reach through `read_msvc_string` / `_with_reason`.
        ") -> Result<(String, MsvcStringEncoding), MsvcStringRefusal> {\n    read_msvc_string_with_policy_in(&LIVE_MEMORY, addr, max_len, policy)\n}",
        // The snapshot-served body.
        ") -> Result<(String, MsvcStringEncoding), MsvcStringRefusal> {\n    read_msvc_string_with_snapshot_with_policy_in(\n        &LIVE_MEMORY,\n        base,\n        off,\n        max_len,\n        snapshot,\n        policy,\n    )\n}",
        // The double-read token/name snapshot, behind `read_stable_msvc_string
        // _snapshot` and the token caches.
        "fn read_msvc_string_snapshot_once(addr: usize, max_len: usize) -> Option<MsvcStringSnapshot> {\n    read_msvc_string_snapshot_once_in(&LIVE_MEMORY, addr, max_len)\n}",
        // The two nullable readers: display names and cosmetic tokens.
        "fn read_msvc_display_string_nullable(addr: usize, max_len: usize) -> Option<Option<String>> {\n    read_msvc_display_string_nullable_in(&LIVE_MEMORY, addr, max_len)\n}",
        "fn read_msvc_token_nullable(addr: usize, max_len: usize) -> Option<Option<String>> {\n    read_msvc_token_nullable_in(&LIVE_MEMORY, addr, max_len)\n}",
        // The field helpers, 44 call sites between them.
        ") -> Option<u64> {\n    snapshot_u64_or_read_in(&LIVE_MEMORY, snapshot, base, off)\n}",
        ") -> Option<usize> {\n    snapshot_ptr_or_read_in(&LIVE_MEMORY, snapshot, base, off)\n}",
        ") -> Option<u32> {\n    snapshot_u32_or_read_in(&LIVE_MEMORY, snapshot, base, off)\n}",
        ") -> Option<ObjectSnapshot<N>> {\n    compact_object_window_in(&LIVE_MEMORY, addr, use_windows)\n}",
        // The acceptance proof's decode.
        ") -> Result<Option<(String, MsvcStringEncoding)>, Domain47PortraitIdentityRefusal> {\n    domain47_portrait_identity_with_snapshot_in(&LIVE_MEMORY, entry_addr, entry)\n}",
    ] {
        assert_eq!(
            source.matches(delegation).count(),
            1,
            "the live form\n{delegation}\nis not in the collector source exactly once, \
             so it is no longer a pure delegation to the generic body"
        );
    }
}

/// NO GENERIC BODY READS THE LIVE PROCESS BY NAME.
///
/// The defect this closes is the one that makes a migration LOOK done and
/// leave the conversion broken: a `_in` body that takes a `mem` and then reads
/// one field through `read_u64` anyway. Nothing behavioural catches it — over
/// `LiveMemory` the two are the same function — and the first sign of it would
/// be a post-bracket decode that silently reached into the running game for one
/// value, publishing this hour's row with last hour's field in it, which is
/// exactly the hour misattribution the owner named as a stop condition.
///
/// The check is textual and scoped to the bodies: from each `_in` function's
/// signature to the next top-level `}` at column 0. That is coarse — it also
/// covers nested items — and coarse in the safe direction.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: any of the live primitive names
/// appearing inside a generic body, which is precisely the missed call site.
#[test]
fn no_generic_decode_body_reaches_the_live_process_by_name() {
    let source = collector_source_lf();
    // The live-process readers a generic body must never name. `read_into` and
    // `read_scalar` are the primitives; the rest are the composed live forms,
    // which reach the primitives through `LIVE_MEMORY`.
    // MATCHED AS WHOLE IDENTIFIERS, not as `name(`. A function can be reached
    // WITHOUT a parenthesis — `and_then(read_u64)` passes it as a value and
    // calls it a moment later, which is a live read inside a post-bracket
    // decode that every call-shaped needle waves through. (Codex 019fe8c7,
    // finding 2.)
    //
    // Identifier boundaries are what keep this from over-matching: `read_u64`
    // does not match inside `read_u64_stable`, because the character after it
    // continues the identifier. Comments and strings are already gone, so any
    // remaining occurrence is a use of the thing.
    const LIVE_READERS: &[&str] = &[
        "read_scalar",
        "read_bytes",
        "read_bytes_with_reason",
        "read_into",
        "read_u64",
        "read_u32",
        "read_u16",
        "read_u8",
        "read_i32",
        "read_f32",
        "read_msvc_string",
        "read_msvc_string_with_reason",
        "read_msvc_string_with_policy",
        "read_msvc_string_with_snapshot",
        "read_msvc_string_with_snapshot_with_reason",
        "read_msvc_string_with_snapshot_with_policy",
        "read_msvc_string_snapshot_once",
        "read_msvc_display_string_nullable",
        "read_msvc_token_nullable",
        "snapshot_u64_or_read",
        "snapshot_u32_or_read",
        "snapshot_ptr_or_read",
        "compact_object_window",
        "domain47_portrait_identity_with_snapshot",
        // THE ESCAPES NO CALL-SHAPE SCAN CAN SEE, named as tokens instead.
        // None is a free-function call, so `the_scanned_set_is_closed_under
        // _calls` would wave them through, and each puts a live read inside a
        // post-bracket decode. (Codex 019fe678 finding 1; 019fe8c7 finding 2.)
        //
        //  * `read` reaches `ObjectSnapshot::<N>::read`, the LIVE object
        //    window; the injected one is `read_in`, whose identifier continues
        //    past `read` and so does not match.
        //  * `LIVE_MEMORY` / `LiveMemory` named anywhere inside a generic body
        //    is the whole migration undone in one token.
        //  * the lane-side live adapters, which a generic body could reach
        //    through TRAIT DISPATCH — `Domain47InProcessMemory::read_array`
        //    bottoms out in `read_into` — and a method call is invisible to
        //    every free-call scan.
        //
        // NOT A COMPLETE DEFENCE for the last of those, and said so plainly:
        // any live-memory adapter this list does not know is still invisible.
        // What catches an unnamed one is `a_decode_over_the_copy_cannot_see_a
        // _write_made_after_the_bracket`, which is behavioural and does not
        // care how the read was spelled.
        "read",
        "LIVE_MEMORY",
        "LiveMemory",
        "Domain47InProcessMemory",
        // The surviving live-process `SnapshotMemory`. `Domain47SnapshotMemory`
        // stood here until d47's copy bracket was retired on 2026-08-13; the
        // entry is renamed rather than dropped because the name this blacklist
        // has to catch is whichever adapter still bottoms out in `read_into`,
        // and a blacklist entry for a deleted type is a check that cannot fail.
        "Domain81SnapshotMemory",
    ];
    // THE SCAN IS TRANSITIVE. Scanning the generic bodies alone is evaded by
    // one level of indirection — a generic body calling a pure-looking helper
    // that itself reads the process would put a live read inside a post-bracket
    // decode and this law would stay quiet (Codex 019fe65a, finding 4). So the
    // set below is the generic bodies PLUS every non-generic helper they reach,
    // and `the_scanned_set_is_closed_under_calls` is what holds that the "plus"
    // is complete rather than a list someone once wrote down.
    // SANITIZED, not raw: every law below asks a question about CODE, and
    // `sanitize_lines` is what makes the text it reads only code.
    let lines = sanitize_lines(&source);
    let mut checked = 0usize;
    for signature in SCANNED_BODIES {
        let (start, end) = body_span(&lines, signature);
        // THE WHOLE BODY AS ONE STRING, not line by line — see `body_code`. A
        // call split across lines, with or without a turbofish, is put back
        // together before matching.
        let code = body_code(&lines, start, end);
        // THE LAW IS LOOKING AT SOMETHING. A prohibition over an empty string
        // passes trivially, so a sanitizer that blanked too much would switch
        // every assertion below off while looking exactly like success — which
        // is precisely what treating a lifetime `'a` as a char literal did
        // before it was fixed. Every generic body routes its reads through the
        // injected memory — either by reading through it (`mem.u64(..)`) or by
        // forwarding it to another generic body (`..._in(mem, ..)`, which is
        // all `snapshot_ptr_or_read_in` does) — so `mem` must appear. The
        // helpers take no memory and are held only to being non-trivial.
        assert!(
            code.len() > 40,
            "`{signature}` sanitized to {} characters: the scan is reading \
             almost nothing, so the prohibitions below are vacuous",
            code.len()
        );
        if signature.contains("DecodeMemory + ?Sized") {
            assert!(
                code.contains("mem"),
                "`{signature}` sanitized to text that never mentions `mem`, so \
                 either the body no longer uses its injected memory at all or \
                 the sanitizer blanked the code that does"
            );
        }
        for reader in LIVE_READERS {
            assert!(
                !names_identifier(&code, reader),
                "`{signature}` (source line {}) names the live reader \
                 `{reader}`: a decode that takes an injected memory and then \
                 reads the running process for one field publishes this hour's \
                 row with last hour's value in it",
                start + 1
            );
        }
        checked += 1;
    }
    // NOT `checked == SCANNED_BODIES.len()`, which cannot fail: `body_span`
    // already panics on a body it cannot locate, so the counter increments
    // exactly once per entry by construction (Codex 019fe887, finding 3).
    //
    // The statement that CAN fail is the one that matters: every generic decode
    // body defined in the file must be in the scanned set. A new `_in` reader
    // added without an entry here is a body nothing scans — which is the exact
    // gap `read_in` was in until this round.
    let declared: BTreeSet<String> = lines
        .iter()
        .filter_map(|line| {
            let code = line.trim_start();
            let rest = code.strip_prefix("fn ")?;
            rest.contains("<M: DecodeMemory + ?Sized")
                .then(|| rest.split(['<', '(']).next().unwrap_or("").to_string())
        })
        .collect();
    let scanned: BTreeSet<String> = SCANNED_BODIES
        .iter()
        .map(|signature| {
            signature
                .trim_start_matches("fn ")
                .split(['<', '('])
                .next()
                .unwrap_or("")
                .to_string()
        })
        .collect();
    let unscanned: Vec<&String> = declared.difference(&scanned).collect();
    assert!(
        unscanned.is_empty(),
        "these generic decode bodies are defined but not in SCANNED_BODIES, so \
         nothing checks whether they read the live process: {unscanned:?}"
    );
    // `checked >= declared.len()` used to be here and was dropped: the subset
    // assertion above already implies it (Codex 019fe8a1, finding 3). What is
    // left is the only part that adds anything — a declaration scan that
    // matched nothing would make the subset assertion trivially true.
    assert!(
        !declared.is_empty(),
        "the declaration scan found no generic bodies, so it is no longer \
         matching this file's spelling and the subset check above is vacuous"
    );
    let _ = checked;
}

/// The functions [`no_generic_decode_body_reaches_the_live_process_by_name`]
/// scans: the generic decode bodies, and the helpers they reach.
///
/// Each entry is the exact source line that opens the function, so a signature
/// that changes shape fails loudly rather than silently scanning nothing.
const SCANNED_BODIES: &[&str] = &[
    // The generic bodies.
    "fn read_msvc_string_with_policy_in<M: DecodeMemory + ?Sized>(",
    "fn read_msvc_string_with_snapshot_with_policy_in<M: DecodeMemory + ?Sized, const N: usize>(",
    "fn read_msvc_string_snapshot_once_in<M: DecodeMemory + ?Sized>(",
    "fn read_msvc_display_string_nullable_in<M: DecodeMemory + ?Sized>(",
    "fn read_msvc_token_nullable_in<M: DecodeMemory + ?Sized>(",
    "fn snapshot_u64_or_read_in<M: DecodeMemory + ?Sized, const N: usize>(",
    "fn snapshot_u32_or_read_in<M: DecodeMemory + ?Sized, const N: usize>(",
    "fn snapshot_ptr_or_read_in<M: DecodeMemory + ?Sized, const N: usize>(",
    "fn compact_object_window_in<M: DecodeMemory + ?Sized, const N: usize>(",
    "fn domain47_portrait_identity_with_snapshot_in<M: DecodeMemory + ?Sized>(",
    // THE OBJECT WINDOW'S GENERIC BODY. An ASSOCIATED function, which the
    // closure scan discards by design — so `compact_object_window_in` reaching
    // it was an unscanned edge, and swapping its `mem.fill(...)` for
    // `read_into(...)` would have evaded the source law entirely. (Codex
    // 019fe887, finding 2.)
    "fn read_in<M: DecodeMemory + ?Sized>(mem: &M, addr: usize) -> Option<ObjectSnapshot<N>> {",
    // The helpers they reach.
    "fn decode_and_bound_msvc_string_bytes(",
    "fn decode_msvc_string_bytes_with_policy(",
    "fn decode_msvc_string_bytes(mut bytes: Vec<u8>, size: usize) -> Option<String> {",
    "fn decode_msvc_string_bytes_tolerant(",
    // FOUND BY `the_scanned_set_is_closed_under_calls`, not by reading the
    // code: the first version of that list had the tolerant decoder without the
    // function it delegates the fallback to.
    "fn decode_windows_1252(bytes: &[u8]) -> Option<String> {",
    "fn windows_1252_decode_byte(byte: u8) -> Option<char> {",
    "fn is_cosmetic_token_byte(value: u8) -> bool {",
    "fn domain47_portrait_reader_reason(",
    "fn domain47_portrait_reader_pointer(",
    "fn domain47_portrait_reader_terminator(",
];

/// `(function, name)` pairs where `function` binds `name` locally, shadowing a
/// free function of the same name in `attach_log.rs`.
///
/// Every entry is a decision: it says "this collision is a local binding, not a
/// call out of the body, and someone checked". The four string readers each
/// open with `let refuse = MsvcStringRefusal::new;` while the file also has a
/// free `fn refuse(` elsewhere, which is the entire population today.
///
/// Held honest by `every_locally_bound_name_is_really_bound`: an entry for a
/// binding that is not there would silently excuse a real call.
const LOCALLY_BOUND: &[(&str, &str)] = &[
    ("read_msvc_string_with_policy_in", "refuse"),
    ("read_msvc_string_with_snapshot_with_policy_in", "refuse"),
    ("domain47_portrait_identity_with_snapshot_in", "refuse"),
    ("decode_and_bound_msvc_string_bytes", "refuse"),
];

/// EVERY EXCUSED NAME IS REALLY BOUND, IN THE BODY THAT CLAIMS IT.
///
/// [`LOCALLY_BOUND`] is an exception list, and an exception list nobody checks
/// is a hole with a comment over it: one stale entry silently excuses a real
/// call to a function that might read the process.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: removing or renaming a local
/// binding while leaving its entry here — at which point the entry starts
/// excusing calls instead of describing a shadow.
#[test]
fn every_locally_bound_name_is_really_bound() {
    let source = collector_source_lf();
    // SANITIZED, not raw: every law below asks a question about CODE, and
    // `sanitize_lines` is what makes the text it reads only code.
    let lines = sanitize_lines(&source);
    for (function, name) in LOCALLY_BOUND {
        let signature = SCANNED_BODIES
            .iter()
            .find(|signature| {
                signature
                    .trim_start_matches("fn ")
                    .starts_with(&format!("{function}("))
                    || signature
                        .trim_start_matches("fn ")
                        .starts_with(&format!("{function}<"))
            })
            .unwrap_or_else(|| {
                panic!("`{function}` is not a scanned body, so its entry excuses nothing")
            });
        let (start, end) = body_span(&lines, signature);
        let binding = format!("let {name} =");
        assert!(
            lines[start..=end]
                .iter()
                .any(|line| line.trim_start().starts_with(&binding)),
            "`{function}` does not bind `{name}`, so its entry in LOCALLY_BOUND \
             is excusing real calls to the free function of that name"
        );
    }
}

/// A function body as ONE normalised string, ready for needle matching.
///
/// Line-by-line scanning cannot see a call whose name and parenthesis are on
/// different lines, and rustfmt produces those: `attach_log.rs` already
/// contains
///
/// ```text
///     let entry = ObjectSnapshot::<
///         { domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_LEN },
///     >::read(entry_addr)
/// ```
///
/// so a generic body could reach a live reader in exactly that shape and every
/// per-line scan would stay quiet. (Codex 019fe8a1, finding 1.)
///
/// Three steps, in this order:
///  1. drop comment lines, so prose naming a reader is not a call;
///  2. join what is left with single spaces, which puts a split call back
///     together;
///  3. erase turbofish, then close the gap before `(`, so `name ::<T> (args)`
///     and `name(args)` are the same text.
fn body_code(lines: &[String], start: usize, end: usize) -> String {
    // WHITESPACE IS REMOVED ENTIRELY, not collapsed. Rust's grammar does not
    // care how much whitespace sits between a name, a turbofish and a `(`, so
    // neither can a law about calls: `name (args)`, `name\n(args)` and
    // `name::\n<N>(args)` are all the same call as `name(args)`. Anything that
    // makes an adjacency NOT a call — `;`, `,`, an operator — is punctuation
    // and survives.
    //
    // `lines` must already be SANITIZED (see `sanitize_lines`): comments and
    // string literals blanked to spaces. Otherwise a comment could break a real
    // call in half, or supply a fake one.
    let joined: String = lines[start..=end]
        .iter()
        .flat_map(|line| line.chars())
        .filter(|c| !c.is_whitespace())
        .collect();
    strip_turbofish(&joined)
}

/// Whether `code` uses `name` AS A WHOLE IDENTIFIER — called, referenced, or
/// passed as a value.
///
/// Both source laws ask this rather than looking for `name(`, because a
/// function reached WITHOUT a parenthesis is still reached:
/// `and_then(read_u64)` hands the live reader to a combinator that calls it a
/// moment later, and a call-shaped needle sees nothing. (Codex 019fe8c7,
/// finding 2.)
///
/// The boundaries are what make that safe: an occurrence flanked by identifier
/// characters is part of a LONGER name and is not this one — `read_u64` does
/// not match inside `read_u64_stable`, and `read` does not match inside
/// `read_in`. `code` is expected to be sanitized and whitespace-free, so every
/// remaining occurrence is a use.
fn names_identifier(code: &str, name: &str) -> bool {
    let bytes = code.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    code.match_indices(name).any(|(at, _)| {
        let before_ok = at == 0 || !is_ident(bytes[at - 1]);
        let after = at + name.len();
        let after_ok = after >= bytes.len() || !is_ident(bytes[after]);
        before_ok && after_ok
    })
}

/// Every comment and string literal in `source`, blanked to spaces, with the
/// line structure preserved.
///
/// # Why this exists
///
/// Three source laws in this file scan function bodies for call names and for
/// brace depth. Both questions have the same enemy: Rust text that LOOKS like
/// code and is not. Five review rounds found five separate instances of it —
/// a doc comment naming a reader, a `}` inside a string ending a body scan
/// early, a comment splitting a call in two, a comment supplying a call that
/// was never written, and `"...NUL terminator"` reading as a raw-string opener.
/// Each was patched individually and the next round found another. (Codex
/// 019fe65a, 019fe678, 019fe887, 019fe8a1, 019fe8b3.)
///
/// Patching lexical cases one at a time is unbounded. Handling the lexical
/// grammar ONCE is not, and this is it: after this pass no law in this file can
/// see anything but code, so the whole class is closed rather than sampled.
///
/// Blanking rather than deleting keeps every line index and column stable, so
/// error messages still point at real source lines.
///
/// Handles what this file's grammar actually contains: line comments, NESTED
/// block comments (Rust allows them), char literals, normal strings with
/// backslash escapes, and raw strings with any prefix (`r`, `br`, `cr`) and any
/// number of hashes.
/// Visible to the parent test module since 2026-08-15: lane JX's
/// `no_collector_module_calls_the_a25_runtime_text_installer` scans EVERY
/// collector source file, and `code_only` mis-lexes hashed raw strings, which
/// would let a forbidden call hide behind one (Codex 01a00360 round 2,
/// finding 1). This handles the lexical grammar once, which is why it is
/// shared rather than re-implemented.
pub(super) fn sanitize_lines(source: &str) -> Vec<String> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Code,
        LineComment,
        BlockComment(u32),
        Str,
        RawStr(usize),
        Char,
    }
    let mut state = State::Code;
    let mut out = String::with_capacity(source.len());
    let chars: Vec<char> = source.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        let c = chars[index];
        let next = chars.get(index + 1).copied();
        if c == '\n' {
            // A newline always survives, and it ends a line comment.
            if state == State::LineComment {
                state = State::Code;
            }
            out.push('\n');
            index += 1;
            continue;
        }
        match state {
            State::Code => {
                if c == '/' && next == Some('/') {
                    state = State::LineComment;
                    out.push_str("  ");
                    index += 2;
                } else if c == '/' && next == Some('*') {
                    state = State::BlockComment(1);
                    out.push_str("  ");
                    index += 2;
                } else if c == '\'' {
                    // A `'` OPENS A CHAR LITERAL ONLY SOMETIMES. `&'a str`,
                    // `SnapshotViewMemory<'a>` and `'_` are LIFETIMES, and
                    // `attach_log.rs` has 112 of them. Treating every `'` as an
                    // opener would blank from one lifetime to the next —
                    // swallowing whole functions of real code and silently
                    // switching off the laws that read them. A false blank is
                    // worse than no sanitizer, because it looks like success.
                    //
                    // A char literal is `'x'` or `'\n'`; anything else is a
                    // lifetime and is code.
                    let escaped = next == Some('\\');
                    let simple = chars.get(index + 2) == Some(&'\'');
                    if escaped || simple {
                        state = State::Char;
                        out.push(' ');
                        index += 1;
                    } else {
                        out.push(c);
                        index += 1;
                    }
                } else if c == '"' {
                    state = State::Str;
                    out.push(' ');
                    index += 1;
                } else if c == 'r' || c == 'b' || c == 'c' {
                    // A raw-string opener: an optional `b`/`c`, then `r`, then
                    // hashes, then a quote — and the `r` must START an
                    // identifier, or the last letter of an ordinary word (as in
                    // `"...NUL terminator"`) reads as one.
                    let starts_identifier = index
                        .checked_sub(1)
                        .map(|prev| !(chars[prev].is_alphanumeric() || chars[prev] == '_'))
                        .unwrap_or(true);
                    let mut cursor = index;
                    if (c == 'b' || c == 'c') && chars.get(cursor + 1) == Some(&'r') {
                        cursor += 1;
                    }
                    if starts_identifier && chars.get(cursor) == Some(&'r') {
                        let mut hashes = 0usize;
                        let mut probe = cursor + 1;
                        while chars.get(probe) == Some(&'#') {
                            hashes += 1;
                            probe += 1;
                        }
                        if chars.get(probe) == Some(&'"') {
                            state = State::RawStr(hashes);
                            for _ in index..=probe {
                                out.push(' ');
                            }
                            index = probe + 1;
                            continue;
                        }
                    }
                    out.push(c);
                    index += 1;
                } else {
                    out.push(c);
                    index += 1;
                }
            }
            State::LineComment => {
                out.push(' ');
                index += 1;
            }
            State::BlockComment(depth) => {
                if c == '/' && next == Some('*') {
                    state = State::BlockComment(depth + 1);
                    out.push_str("  ");
                    index += 2;
                } else if c == '*' && next == Some('/') {
                    state = if depth == 1 {
                        State::Code
                    } else {
                        State::BlockComment(depth - 1)
                    };
                    out.push_str("  ");
                    index += 2;
                } else {
                    out.push(' ');
                    index += 1;
                }
            }
            State::Str => {
                if c == '\\' {
                    out.push_str("  ");
                    index += 2;
                } else if c == '"' {
                    state = State::Code;
                    out.push(' ');
                    index += 1;
                } else {
                    out.push(' ');
                    index += 1;
                }
            }
            State::Char => {
                // Byte-for-byte blanking; a `'` closes it, an escape consumes
                // two. Lifetimes (`'a`) close at the next `'` or run to the end
                // of the line, and either way only whitespace is produced.
                if c == '\\' {
                    out.push_str("  ");
                    index += 2;
                } else if c == '\'' {
                    state = State::Code;
                    out.push(' ');
                    index += 1;
                } else {
                    out.push(' ');
                    index += 1;
                }
            }
            State::RawStr(hashes) => {
                if c == '"' && chars[index + 1..].iter().take(hashes).all(|h| *h == '#') {
                    state = State::Code;
                    for _ in 0..=hashes {
                        out.push(' ');
                    }
                    index += 1 + hashes;
                } else {
                    out.push(' ');
                    index += 1;
                }
            }
        }
    }
    out.lines().map(str::to_owned).collect()
}

/// `body_code` NEITHER MISSES A CALL NOR INVENTS ONE.
///
/// Both laws in this file match needles against its output, so it has two ways
/// to be wrong and they fail in opposite directions: a missed call is a law
/// that stopped looking, and an invented one is a law that blocks a change
/// nobody made. The cases below are both kinds.
///
/// The joining rule rests on a fact about Rust, not a hope: `name` at the end
/// of one line and `(args)` at the start of the next IS a call, which is why
/// putting them back together is correct rather than merely convenient. The
/// separators that make an adjacency NOT a call — `;`, `,` — survive the join
/// and keep the needle from matching.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: dropping the comment filter
/// (prose naming a reader would read as a call), dropping the space-before-`(`
/// closure (every multiline call would go unseen), or joining without a
/// separator (adjacent identifiers would fuse into names nobody wrote).
#[test]
fn body_code_reassembles_calls_without_inventing_them() {
    // Through the SANITIZER, exactly as the laws reach it: comments and
    // literals are already gone by the time `body_code` sees a line, and
    // testing it any other way would be testing a function nothing calls.
    let case = |lines: &[&str]| {
        let sanitized = sanitize_lines(&lines.join("\n"));
        body_code(&sanitized, 0, sanitized.len() - 1)
    };

    // A call split across lines is put back together.
    assert!(case(&["fn f() {", "    read_u64", "        (addr);", "}"])
        .contains("read_u64("));
    // Including with a multiline turbofish — the escape this was built for.
    assert!(case(&[
        "fn f() {",
        "    snapshot_u64_or_read::<",
        "        N,",
        "    >(snapshot, base, off);",
        "}",
    ])
    .contains("snapshot_u64_or_read("));
    // And the ordinary single-line spelling still matches.
    assert!(case(&["fn f() {", "    read_u64(addr);", "}"]).contains("read_u64("));

    // A COMMENT LINE naming a reader is not a call.
    assert!(!case(&["fn f() {", "    // read_u64(addr) used to be here", "}"])
        .contains("read_u64("));
    // A trailing comment on a code line does not hide the code before it.
    assert!(case(&["fn f() {", "    mem.u64(addr); // not read_u64(addr)", "}"])
        .contains("mem.u64("));

    // NOT A CALL, and not invented: the separators survive the join.
    assert!(!case(&["fn f() {", "    let g = read_u64;", "    (0..n).count();", "}"])
        .contains("read_u64("));
    assert!(!case(&["fn f() {", "    match x { A => read_u64,", "    (a, b) => y }", "}"])
        .contains("read_u64("));
}

/// Delete every `::<...>` turbofish from a source line, so a call spelled with
/// one matches the same needle as a call spelled without.
///
/// Both scans in this file look for `name(`, and `name::<T>(` is the SAME CALL
/// wearing type arguments — ordinary syntax that this collector already uses
/// (`compact_object_window::<0x20>(...)`). Without this, one turbofish takes a
/// call out of both laws. (Codex 019fe887, finding 2.)
///
/// Nesting is handled by depth, because `foo::<Bar<Baz>>(` closes twice.
fn strip_turbofish(code: &str) -> String {
    let bytes: Vec<char> = code.chars().collect();
    let mut out = String::with_capacity(code.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == ':'
            && bytes.get(index + 1) == Some(&':')
            && bytes.get(index + 2) == Some(&'<')
        {
            let mut depth = 0usize;
            // A `>` INSIDE A BRACED CONST ARGUMENT IS A COMPARISON, not a
            // delimiter: `read_scalar::<{ if 2 > 1 { 8 } else { 4 } }>(addr)`
            // is one turbofish, and stopping at that `>` would leave the call
            // unrecognised by both laws. (Codex 019fe8c7, finding 3.) Angle
            // brackets are only counted at brace depth zero.
            let mut braces = 0usize;
            let mut cursor = index + 2;
            while cursor < bytes.len() {
                match bytes[cursor] {
                    '{' => braces += 1,
                    '}' => braces = braces.saturating_sub(1),
                    '<' if braces == 0 => depth += 1,
                    '>' if braces == 0 => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                cursor += 1;
            }
            if cursor < bytes.len() {
                // Skip `::<...>` entirely, joining the name to what follows.
                index = cursor + 1;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

/// `strip_turbofish` DOES WHAT THE TWO SCANS ASSUME IT DOES.
///
/// Both laws in this file feed their lines through it before matching, so an
/// error in it makes both quietly stop seeing calls — the exact failure mode
/// they exist to prevent. It is thirty lines of hand-rolled scanning over
/// source text, which is precisely the kind of code that is right for the cases
/// its author thought of.
///
/// The cases below are the ones this collector actually contains (plain calls,
/// const-generic turbofish, nested generic arguments, path-qualified calls,
/// comparison operators that are NOT turbofish) plus the two that would break a
/// naive implementation: nesting, which needs depth rather than a first `>`,
/// and an unterminated `::<` which must not eat the rest of the line.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: any edit to `strip_turbofish`
/// that stops joining the name to its parenthesis, or that starts eating text
/// it should not.
#[test]
fn strip_turbofish_joins_the_name_to_its_parenthesis() {
    for (input, expected) in [
        // Untouched: no turbofish.
        ("read_u64(addr + 0x10)", "read_u64(addr + 0x10)"),
        // The escape this exists for.
        ("snapshot_u64_or_read::<N>(snapshot, base, off)", "snapshot_u64_or_read(snapshot, base, off)"),
        ("compact_object_window::<0x20>(addr, true)", "compact_object_window(addr, true)"),
        // NESTED generic arguments: `>>` closes twice, so a scan that stopped
        // at the first `>` would leave `>(` behind and match nothing.
        ("foo::<Bar<Baz>>(x)", "foo(x)"),
        ("foo::<Bar<Baz, Qux<u8>>>(x)", "foo(x)"),
        // A path-qualified call keeps its path; only the turbofish goes.
        ("ObjectSnapshot::<N>::read(addr)", "ObjectSnapshot::read(addr)"),
        // NOT a turbofish: a comparison. `::<` is the only trigger.
        ("if size < max && cap > size {", "if size < max && cap > size {"),
        ("let x = a < b;", "let x = a < b;"),
        // Unterminated: must be left alone rather than swallowing the line.
        ("foo::<Bar(x)", "foo::<Bar(x)"),
        // A `>` INSIDE A BRACED CONST ARGUMENT is a comparison, not the
        // turbofish's closer.
        (
            "read_scalar::<{ if 2 > 1 { 8 } else { 4 } }>(addr)",
            "read_scalar(addr)",
        ),
        // The const-generic form this collector actually uses.
        (
            "ObjectSnapshot::<{ ENTRY_LEN }>::read(addr)",
            "ObjectSnapshot::read(addr)",
        ),
    ] {
        assert_eq!(
            strip_turbofish(input),
            expected,
            "strip_turbofish mangled `{input}`"
        );
    }
}

/// `[first, last]` line indices of the item opened by `signature`.
///
/// `lines` must be SANITIZED (see [`sanitize_lines`]). That is the whole reason
/// this is now three statements instead of two competing heuristics: with every
/// comment and string literal blanked, a `{` is a block opener and a `}` is a
/// block closer, so counting them is not a guess. The previous version derived
/// the end twice — by indentation and by brace depth, skipping any line with a
/// quote in it — and both derivations truncated together on an ordinary
/// multiline string, which is the defect that made the whole approach worth
/// replacing. (Codex 019fe8b3, finding 1.)
///
/// Counting starts at the line that OPENS the body, not at the signature's
/// first line: a multi-line parameter list sits at depth 0 throughout, and a
/// const-generic argument like `ObjectSnapshot<{ ENTRY_LEN }>` opens and closes
/// a pair before the body begins.
fn body_span(lines: &[String], signature: &str) -> (usize, usize) {
    let start = lines
        .iter()
        .position(|line| line.trim_start() == signature)
        .unwrap_or_else(|| panic!("`{signature}` is no longer in the collector source"));
    let open = lines[start..]
        .iter()
        .position(|line| line.trim_end().ends_with('{'))
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("`{signature}` has no line opening its body"));
    let mut depth = 0i32;
    for (offset, line) in lines[open..].iter().enumerate() {
        depth += line.matches('{').count() as i32;
        depth -= line.matches('}').count() as i32;
        if depth == 0 {
            return (start, open + offset);
        }
    }
    panic!("`{signature}` never closes");
}

/// THE SANITIZER REMOVES EVERYTHING THAT IS NOT CODE, AND NOTHING THAT IS.
///
/// Every source law in this file rests on it, so its failure modes are theirs:
/// leave a comment in and a law reads prose as a call, or a call as two halves;
/// blank too much and a law stops seeing the code it exists to check.
///
/// The cases below are the five that five review rounds each found separately,
/// plus the raw-string forms this file does not currently contain but Rust
/// permits — `r##"..."##`, `br"..."`, `cr"..."` — because "we don't use that
/// yet" is how the previous four patches were justified.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: any edit to `sanitize_lines`
/// that stops blanking a literal form, blanks code, or loses a line.
#[test]
fn the_sanitizer_blanks_only_what_is_not_code() {
    let joined = |source: &str| sanitize_lines(source).join("\n");

    // Line and block comments go, including NESTED block comments, which Rust
    // allows and a naive scan-to-`*/` gets wrong.
    assert!(!joined("let x = 1; // read_u64(addr)").contains("read_u64"));
    assert!(!joined("let x = /* read_u64(addr) */ 1;").contains("read_u64"));
    assert!(!joined("let x = /* a /* read_u64(addr) */ b */ 1;").contains("read_u64"));
    // ...and the code around them stays.
    assert!(joined("mem.u64(addr); // read_u64(addr)").contains("mem.u64(addr);"));

    // Strings go, with their escapes — a `\"` does not end the string.
    assert!(!joined(r#"let s = "read_u64(addr)";"#).contains("read_u64"));
    assert!(!joined(r#"let s = "a\"read_u64(addr)"; "#).contains("read_u64"));
    // A BRACE INSIDE A STRING is not a block delimiter. This is the one that
    // made `body_span` truncate.
    assert_eq!(joined("let s = \"}\"; foo();").matches('}').count(), 0);

    // Raw strings, every prefix and any number of hashes.
    assert!(!joined(r####"let s = r"read_u64(addr)";"####).contains("read_u64"));
    assert!(!joined(r####"let s = r#"read_u64(addr)"#;"####).contains("read_u64"));
    assert!(!joined(r####"let s = r##"read_u64(addr)"##;"####).contains("read_u64"));
    assert!(!joined(r####"let s = br"read_u64(addr)";"####).contains("read_u64"));
    assert!(!joined(r####"let s = cr"read_u64(addr)";"####).contains("read_u64"));
    // A word ENDING in `r` before a quote is NOT a raw-string opener — the
    // false positive the previous version fired on.
    assert!(joined("f(\"NUL terminator\"); read_u64(addr);").contains("read_u64(addr)"));
    // A HASHED RAW STRING CONTAINING A BARE `"` ENDS AT ITS OWN `"#`, AND THE
    // CODE AFTER IT SURVIVES. This is the exact escape that made lane JX share
    // this sanitizer instead of using `code_only`: under a lexer that treats
    // the embedded quote as the terminator, everything after it reads as
    // string, so a call written there vanishes and an absence test passes
    // while the path it guards is live (Codex 01a00360 round 2, finding 1).
    assert!(joined(r####"let s = r#"one " quote"#; read_u64(addr);"####)
        .contains("read_u64(addr)"));
    assert!(!joined(r####"let s = r#"one " read_u64(addr)"#;"####).contains("read_u64"));

    // Char literals, and an escaped quote inside one.
    assert!(!joined(r"let c = '\'';").contains('\''));
    // A brace inside a char literal is not a delimiter either.
    assert_eq!(joined("let c = '}'; foo();").matches('}').count(), 0);

    // LIFETIMES ARE CODE, NOT LITERALS. This file has 112 of them, and
    // treating each `'` as an opener would blank from one lifetime to the next
    // — swallowing whole functions and switching off every law that reads
    // them, while looking exactly like success.
    assert!(joined("fn f<'a>(x: &'a str) { read_u64(addr); }").contains("read_u64(addr)"));
    assert!(joined("struct S<'a> { v: View<'a> } fn g() { read_u64(a); }")
        .contains("read_u64(a)"));
    assert!(joined("impl SnapshotViewMemory<'_> { fn h() { read_into(a, b); } }")
        .contains("read_into(a, b)"));
    // ...and a char literal AFTER two lifetimes still blanks, so the lifetime
    // rule did not simply disable char handling. Counted, not `contains`: the
    // function's own closing brace is code and must survive, so exactly one of
    // the two `}` in this input remains.
    assert_eq!(
        joined("fn f<'a>(x: &'a str) { let c = '}'; }")
            .matches('}')
            .count(),
        1
    );

    // LINE STRUCTURE IS PRESERVED, so every error message still points at a
    // real source line.
    assert_eq!(sanitize_lines("a\nb\nc").len(), 3);
    // Four lines, and a block comment spanning two of them does not merge or
    // drop either — the blanking is per character.
    assert_eq!(sanitize_lines("a\n/* x\ny */\nc").len(), 4);
    assert_eq!(sanitize_lines("a\n/* x\ny */\nc")[3], "c");
}



/// THE SCANNED SET IS CLOSED UNDER CALLS.
///
/// [`SCANNED_BODIES`] is only a guarantee if nothing it covers calls something
/// it does not. Otherwise the transitive scan is a list of names that happened
/// to be right the day it was written, and the first new helper walks straight
/// through it.
///
/// So: collect every free function DEFINED in `attach_log.rs`, then for each
/// scanned body find which of those names it calls. Every callee must itself be
/// scanned.
///
/// Three things are excluded, and the third is the one that had to be got
/// right. Method calls (`.name(`), because they are not free functions — the
/// `ObjectSnapshot` accessors that dominate them index a `[u8; N]` the caller
/// already holds. Names the body BINDS locally, which shadow any free function
/// of the same name. And `Type::assoc(`, an associated function — but NOT
/// `module::free_fn(`, which is a free call wearing a path: excluding every
/// `::` would let one `super::` prefix hide a callee from the scan completely,
/// and the callee is exactly where the live read would be.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: a generic body, or anything it
/// reaches, growing a call to a free function nobody has decided is
/// memory-pure. The fix is to add it to `SCANNED_BODIES` — at which point the
/// live-reader scan runs over it and says whether it was safe.
#[test]
fn the_scanned_set_is_closed_under_calls() {
    let source = collector_source_lf();
    // SANITIZED, not raw: every law below asks a question about CODE, and
    // `sanitize_lines` is what makes the text it reads only code.
    let lines = sanitize_lines(&source);

    // Every FREE function defined in this file, by name.
    //
    // METHODS ARE EXCLUDED, and they have to be: this law matches bare
    // identifiers, and `ObjectSnapshot`'s accessors are called `u8`, `u32`,
    // `u64`, `bytes`, `field` — names that are also ordinary local variables
    // all over these bodies. Counting them as free functions makes every
    // `let bytes = ...` look like a call to something unscanned. A method was
    // never part of the free-function surface this law is about, so dropping
    // them is the correct rule and not a convenience. (Codex 019fe8c7,
    // finding 2, and the over-match that fix first produced.)
    let mut defined: BTreeSet<&str> = BTreeSet::new();
    for (index, line) in lines.iter().enumerate() {
        let code = line.trim_start();
        let Some(rest) = code
            .strip_prefix("fn ")
            .or_else(|| code.strip_prefix("pub fn "))
            .or_else(|| code.strip_prefix("pub(super) fn "))
            .or_else(|| code.strip_prefix("pub(crate) fn "))
            .or_else(|| code.strip_prefix("unsafe fn "))
            .or_else(|| code.strip_prefix("pub unsafe fn "))
        else {
            continue;
        };
        let name = rest
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .unwrap_or("");
        if name.is_empty() {
            continue;
        }
        // The receiver, if any, is the first thing after `(` — on this line, or
        // on the next when the parameter list wraps.
        let after_paren = rest.split_once('(').map(|(_, tail)| tail.trim()).unwrap_or("");
        let first_param = if after_paren.is_empty() {
            lines.get(index + 1).map(|l| l.trim()).unwrap_or("")
        } else {
            after_paren
        };
        // A receiver is `self`, `&self`, `&mut self` or `mut self` (by value —
        // `MsvcStringRefusal::raw` is spelled that way, and missing it left
        // `raw` looking like a free function that every `let raw = ...` called).
        // Peeled rather than pattern-matched against a list, and the trailing
        // boundary is checked, so a parameter named `selfish` is not a
        // receiver.
        let receiver = first_param
            .trim_start_matches('&')
            .trim_start()
            .trim_start_matches("mut ")
            .trim_start();
        let is_method = receiver.strip_prefix("self").is_some_and(|tail| {
            tail.chars()
                .next()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        });
        if !is_method {
            defined.insert(name);
        }
    }
    assert!(
        defined.contains("read_u64") && defined.contains("read_bytes_with_reason"),
        "the definition scan found neither of the two readers it must; its \
         prefixes no longer match this file"
    );

    let scanned: BTreeSet<&str> = SCANNED_BODIES
        .iter()
        .map(|signature| {
            signature
                .trim_start_matches("fn ")
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or("")
        })
        .collect();

    let mut unlisted: BTreeSet<(&str, &str)> = BTreeSet::new();
    for signature in SCANNED_BODIES {
        let (start, end) = body_span(&lines, signature);
        let own = signature
            .trim_start_matches("fn ")
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .unwrap_or("");
        // Names this body BINDS locally, which shadow any free function of the
        // same name. An EXPLICIT, REVIEWED LIST and not a `let` scan: a scan is
        // scope- and order-insensitive, so one nested or later
        // `let helper = ...` anywhere in a body would silently suppress every
        // free `helper(...)` call in the whole of it (Codex 019fe678,
        // finding 1). Today the list is one name in four bodies, and a new
        // entry is a decision someone makes rather than a side effect.
        let shadowed: BTreeSet<&str> = LOCALLY_BOUND
            .iter()
            .filter(|(body, _)| *body == own)
            .map(|(_, name)| *name)
            .collect();
        // THE WHOLE BODY AS ONE STRING, for the same reason the live-reader law
        // takes it that way: a call split across lines is still a call. The
        // signature line is included, which is harmless — the only name it
        // carries is the body's own, and `own` is excused below.
        {
            let code = body_code(&lines, start, end);
            let code = code.as_str();
            for callee in defined.iter().copied() {
                // THE BARE IDENTIFIER, not `callee(`. A function reached
                // without a parenthesis is still reached — `and_then(read_u64)`
                // hands it to a combinator — so this law asks the same question
                // the live-reader law does. (Codex 019fe8c7, finding 2.)
                let mut from = 0usize;
                while let Some(offset) = code[from..].find(callee) {
                    let at = from + offset;
                    from = at + 1;
                    let before = code[..at].chars().next_back();
                    let after = code[at + callee.len()..].chars().next();
                    // Part of a LONGER identifier in either direction:
                    // `my_read_u64` and `read_u64_stable` are not `read_u64`.
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                        || after.is_some_and(|c| c.is_alphanumeric() || c == '_')
                    {
                        continue;
                    }
                    // A method call or field access on a value. Never a free
                    // function.
                    if before == Some('.') {
                        continue;
                    }
                    // A PATH-QUALIFIED name. `Type::assoc(` is an associated
                    // function and not the free-function surface this law is
                    // about; `module::free_fn(` IS a free call and must NOT be
                    // waved through, or one `super::` prefix hides a callee
                    // from the scan entirely — and the callee is where the read
                    // would be. Told apart by the case of the qualifying
                    // segment, which is Rust's own convention and is what every
                    // path in these bodies follows.
                    if before == Some(':') {
                        let qualifier = code[..at.saturating_sub(2)]
                            .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
                            .find(|segment| !segment.is_empty())
                            .unwrap_or("");
                        let is_type = qualifier
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_uppercase())
                            || qualifier == "Self";
                        if is_type {
                            continue;
                        }
                    }
                    if callee == own
                        || scanned.contains(callee)
                        || shadowed.contains(callee)
                    {
                        continue;
                    }
                    unlisted.insert((own, callee));
                }
            }
        }
    }
    assert!(
        unlisted.is_empty(),
        "these scanned bodies call free functions that are NOT themselves \
         scanned, so nothing has established that they do not read the live \
         process: {unlisted:?}"
    );
}

/// `ObjectSnapshot::read_in` IS the only body, and the live `read` is the only
/// caller that supplies the process.
///
/// Kept apart from the delegation law above because it is the const-generic
/// one: `ObjectSnapshot::<N>::read` is called at ~30 sites with ~25 distinct
/// `N`, so if the migration had left `read` with its own `read_into` call the
/// delegation law would still pass on the OTHER readers and this whole surface
/// would be unconverted.
#[test]
fn the_object_window_has_one_body() {
    let source = collector_source_lf();
    assert_eq!(
        source
            .matches("fn read_in<M: DecodeMemory + ?Sized>(mem: &M, addr: usize) -> Option<ObjectSnapshot<N>> {")
            .count(),
        1,
        "the object window's generic body moved or was duplicated"
    );
    // The live `read_into` primitive is reached from exactly one place inside
    // the `DecodeMemory` layer: `LiveMemory::fill`. Anything else calling it
    // from a decode is a call site the migration missed.
    assert_eq!(
        source
            .matches("    fn fill(&self, addr: usize, buf: &mut [u8]) -> bool {\n        read_into(addr, buf)\n    }")
            .count(),
        1,
        "`LiveMemory::fill` is no longer the delegation to `read_into`"
    );
}

// ==================================================================
// 2. THE ORACLE — the pre-migration bodies, frozen.
// ==================================================================
//
// A/B-ing the shipped name against its own `_in` body proves NOTHING: the
// shipped name IS a delegation to that body, so both sides execute the same
// current code and a defect introduced into the body moves both answers
// together. (Codex 019fe65a, finding 4: change `mem.u64` to `mem.u32` inside a
// generic body and every such comparison still passes, while a header with a
// size of `0x0000_0001_0000_0001` goes from a refusal to an acceptance.)
//
// So the oracle is the code as it stood at b346f6c2, the commit before this
// lane: transcribed verbatim below, reading the LIVE free functions by name,
// depending on nothing this lane touched. It is a FROZEN COPY and is meant to
// rot — the day someone deliberately changes one of these readers, the test
// that compares against it fails and the change is made here too, on purpose,
// with the same review the production edit gets. That is the point of an
// oracle and not a defect in it.
//
// Transcription is checked by the comparisons themselves: a mis-transcribed
// oracle disagrees with the shipped reader on the very first shape.

/// `read_msvc_string_with_policy` as of b346f6c2.
fn baseline_msvc_string_with_policy(
    addr: usize,
    max_len: usize,
    policy: MsvcStringDecodePolicy,
) -> Result<(String, MsvcStringEncoding), MsvcStringRefusal> {
    use MsvcStringRefusalReason as Reason;
    use MsvcStringStorage as Storage;
    let refuse = MsvcStringRefusal::new;
    let size = read_u64(addr + 0x10)
        .ok_or(refuse(Reason::SizeFieldUnreadable, Storage::NotDecided))? as usize;
    let cap = read_u64(addr + 0x18)
        .ok_or(refuse(Reason::CapacityFieldUnreadable, Storage::NotDecided))? as usize;
    if size == 0 {
        return Err(refuse(Reason::SizeZero, Storage::NotDecided));
    }
    if size > max_len {
        return Err(refuse(Reason::SizeOverMax, Storage::NotDecided));
    }
    if cap < size {
        return Err(refuse(Reason::CapacityBelowSize, Storage::NotDecided));
    }
    if cap > (1 << 20) {
        return Err(refuse(Reason::CapacityImplausible, Storage::NotDecided));
    }
    let (storage, chars_addr) = if cap < 16 {
        (Storage::Inline, addr)
    } else {
        (
            Storage::Heap,
            read_u64(addr).ok_or(refuse(Reason::PayloadPointerUnreadable, Storage::Heap))?
                as usize,
        )
    };
    let bytes = read_bytes_with_reason(chars_addr, size + 1)
        .map_err(|fetch| refuse(Reason::PayloadBytesUnreadable, storage).with_fetch(fetch))?;
    let terminator = bytes.get(size).copied();
    if terminator != Some(0) {
        return Err(refuse(Reason::TerminatorNotNul, storage).with_terminator(terminator));
    }
    decode_and_bound_msvc_string_bytes(bytes, size, max_len, policy, storage)
}

/// `read_msvc_string_with_snapshot_with_policy` as of b346f6c2.
fn baseline_msvc_string_with_snapshot_with_policy<const N: usize>(
    base: usize,
    off: usize,
    max_len: usize,
    snapshot: Option<&ObjectSnapshot<N>>,
    policy: MsvcStringDecodePolicy,
) -> Result<(String, MsvcStringEncoding), MsvcStringRefusal> {
    use MsvcStringRefusalReason as Reason;
    use MsvcStringStorage as Storage;
    let refuse = MsvcStringRefusal::new;
    let Some(snapshot) = snapshot else {
        let addr = base
            .checked_add(off)
            .ok_or(refuse(Reason::BaseOffsetOverflow, Storage::NotDecided))?;
        return baseline_msvc_string_with_policy(addr, max_len, policy);
    };
    let size = off
        .checked_add(0x10)
        .and_then(|field| snapshot.u64(field))
        .ok_or(refuse(Reason::SizeFieldUnreadable, Storage::NotDecided))? as usize;
    let cap = off
        .checked_add(0x18)
        .and_then(|field| snapshot.u64(field))
        .ok_or(refuse(Reason::CapacityFieldUnreadable, Storage::NotDecided))? as usize;
    if size == 0 {
        return Err(refuse(Reason::SizeZero, Storage::NotDecided));
    }
    if size > max_len {
        return Err(refuse(Reason::SizeOverMax, Storage::NotDecided));
    }
    if cap < size {
        return Err(refuse(Reason::CapacityBelowSize, Storage::NotDecided));
    }
    if cap > (1 << 20) {
        return Err(refuse(Reason::CapacityImplausible, Storage::NotDecided));
    }
    let (storage, bytes) = if cap < 16 {
        let end = size
            .checked_add(1)
            .and_then(|len| off.checked_add(len))
            .ok_or(refuse(Reason::InlinePayloadOutOfSnapshot, Storage::Inline))?;
        (
            Storage::Inline,
            snapshot
                .buf
                .get(off..end)
                .ok_or(refuse(Reason::InlinePayloadOutOfSnapshot, Storage::Inline))?
                .to_vec(),
        )
    } else {
        let chars_addr = snapshot
            .ptr(off)
            .ok_or(refuse(Reason::PayloadPointerUnreadable, Storage::Heap))?;
        let len = size
            .checked_add(1)
            .ok_or(refuse(Reason::PayloadBytesUnreadable, Storage::Heap))?;
        (
            Storage::Heap,
            read_bytes_with_reason(chars_addr, len).map_err(|fetch| {
                refuse(Reason::PayloadBytesUnreadable, Storage::Heap).with_fetch(fetch)
            })?,
        )
    };
    let terminator = bytes.get(size).copied();
    if terminator != Some(0) {
        return Err(refuse(Reason::TerminatorNotNul, storage).with_terminator(terminator));
    }
    decode_and_bound_msvc_string_bytes(bytes, size, max_len, policy, storage)
}

/// `read_msvc_string_snapshot_once` as of b346f6c2.
fn baseline_msvc_string_snapshot_once(addr: usize, max_len: usize) -> Option<MsvcStringSnapshot> {
    let size = read_u64(addr.checked_add(0x10)?)? as usize;
    let cap = read_u64(addr.checked_add(0x18)?)? as usize;
    if size > max_len || cap < size || cap > (1 << 20) {
        return None;
    }
    let chars_addr = if cap < 16 {
        addr
    } else {
        read_u64(addr)? as usize
    };
    let bytes_with_terminator = read_bytes(chars_addr, size.checked_add(1)?)?;
    if bytes_with_terminator.get(size).copied() != Some(0) {
        return None;
    }
    let bytes = bytes_with_terminator.get(..size)?.to_vec();
    Some(MsvcStringSnapshot {
        size,
        cap,
        chars_addr,
        bytes,
    })
}

/// `read_msvc_display_string_nullable` as of b346f6c2.
fn baseline_msvc_display_string_nullable(
    addr: usize,
    max_len: usize,
) -> Option<Option<String>> {
    let size = read_u64(addr.checked_add(0x10)?)? as usize;
    let cap = read_u64(addr.checked_add(0x18)?)? as usize;
    if size > max_len || cap < size || cap > (1 << 20) {
        return None;
    }
    let chars_addr = if cap < 16 {
        addr
    } else {
        read_u64(addr)? as usize
    };
    let bytes = read_bytes(chars_addr, size.checked_add(1)?)?;
    if bytes.get(size).copied() != Some(0) {
        return None;
    }
    if size == 0 {
        return Some(None);
    }
    let text = std::str::from_utf8(&bytes[..size]).ok()?;
    if text.chars().any(char::is_control) {
        return None;
    }
    Some(Some(text.to_owned()))
}

/// `read_msvc_token_nullable` as of b346f6c2.
fn baseline_msvc_token_nullable(addr: usize, max_len: usize) -> Option<Option<String>> {
    let size = read_u64(addr + 0x10)? as usize;
    let cap = read_u64(addr + 0x18)? as usize;
    if size > max_len || cap < size || cap > (1 << 20) {
        return None;
    }
    if size == 0 {
        return if cap < 16 { Some(None) } else { None };
    }
    let chars_addr = if cap < 16 {
        addr
    } else {
        read_u64(addr)? as usize
    };
    let bytes = read_bytes(chars_addr, size + 1)?;
    if bytes.get(size).copied() != Some(0) {
        return None;
    }
    let raw = &bytes[..size];
    if !raw.iter().all(|b| is_cosmetic_token_byte(*b)) {
        return None;
    }
    Some(Some(String::from_utf8(raw.to_vec()).ok()?))
}

/// `ObjectSnapshot::<N>::read` as of b346f6c2.
fn baseline_object_snapshot<const N: usize>(addr: usize) -> Option<[u8; N]> {
    let mut buf = [0u8; N];
    if read_into(addr, &mut buf) {
        Some(buf)
    } else {
        None
    }
}

/// `snapshot_u64_or_read` as of b346f6c2.
fn baseline_snapshot_u64_or_read<const N: usize>(
    snapshot: Option<&ObjectSnapshot<N>>,
    base: usize,
    off: usize,
) -> Option<u64> {
    match snapshot {
        Some(snapshot) => snapshot.u64(off),
        None => read_u64(base.checked_add(off)?),
    }
}

/// `snapshot_u32_or_read` as of b346f6c2.
fn baseline_snapshot_u32_or_read<const N: usize>(
    snapshot: Option<&ObjectSnapshot<N>>,
    base: usize,
    off: usize,
) -> Option<u32> {
    match snapshot {
        Some(snapshot) => snapshot.u32(off),
        None => read_u32(base.checked_add(off)?),
    }
}

/// `domain47_portrait_identity_with_snapshot` as of b346f6c2, on top of the
/// baseline string reader above, so the whole chain is independent of the
/// migrated code.
fn baseline_portrait_identity(
    entry_addr: usize,
    entry: &ObjectSnapshot<{ domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_LEN }>,
) -> Result<Option<(String, MsvcStringEncoding)>, Domain47PortraitIdentityRefusal> {
    use Domain47PortraitIdentityDetail as Detail;
    const GFX_OFF: usize = domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_GFX_OFF;
    let mut probe = Domain47PortraitIdentityProbe::default();
    let refuse = |detail: Detail, probe: Domain47PortraitIdentityProbe| {
        Domain47PortraitIdentityRefusal { detail, probe }
    };
    let Some(size) = entry.u64(GFX_OFF + 0x10) else {
        return Err(refuse(Detail::SizeFieldOutOfSnapshot, probe));
    };
    probe.size = size;
    if size == 0 {
        let Some(cap) = entry.u64(GFX_OFF + 0x18) else {
            return Err(refuse(Detail::CapacityFieldOutOfSnapshot, probe));
        };
        probe.capacity = cap;
        let cap = cap as usize;
        if cap > (1 << 20) {
            return Err(refuse(Detail::CapacityImplausible, probe));
        }
        if cap < 16 {
            let Some(terminator) = entry.u8(GFX_OFF) else {
                probe.terminator = Domain47PortraitProbeTerminator::Refused;
                return Err(refuse(Detail::EmptyInlineTerminatorOutOfSnapshot, probe));
            };
            probe.terminator = Domain47PortraitProbeTerminator::Read;
            probe.terminator_value = terminator;
            if terminator != 0 {
                return Err(refuse(Detail::EmptyInlineTerminatorNonZero, probe));
            }
            return Ok(None);
        }
        let Some(chars_addr) = entry.ptr(GFX_OFF) else {
            probe.pointer = Domain47PortraitProbePointer::OutOfSnapshot;
            return Err(refuse(Detail::EmptyHeapPointerOutOfSnapshot, probe));
        };
        probe.pointer = Domain47PortraitProbePointer::Decoded;
        let Some(terminator) = read_bytes(chars_addr, 1).and_then(|bytes| bytes.first().copied())
        else {
            probe.terminator = Domain47PortraitProbeTerminator::Refused;
            return Err(refuse(Detail::EmptyHeapTerminatorUnreadable, probe));
        };
        probe.terminator = Domain47PortraitProbeTerminator::Read;
        probe.terminator_value = terminator;
        if terminator != 0 {
            return Err(refuse(Detail::EmptyHeapTerminatorNonZero, probe));
        }
        return Ok(None);
    }
    match baseline_msvc_string_with_snapshot_with_policy(
        entry_addr,
        GFX_OFF,
        DOMAIN47_CCHARACTER_STRING_MAX_LEN,
        Some(entry),
        MsvcStringDecodePolicy::TolerantWindows1252,
    ) {
        Ok((text, encoding)) => Ok(Some((text, encoding))),
        Err(refusal) => {
            let mut probe = probe;
            probe.capacity = entry.u64(GFX_OFF + 0x18).unwrap_or(0);
            probe.reader_reason = domain47_portrait_reader_reason(refusal.reason);
            probe.pointer = domain47_portrait_reader_pointer(&refusal);
            let (terminator, terminator_value) = domain47_portrait_reader_terminator(&refusal);
            probe.terminator = terminator;
            probe.terminator_value = terminator_value;
            Err(refuse(Detail::NonEmptyTextUnreadable, probe))
        }
    }
}

/// THE ORACLES ARE INDEPENDENT OF THE MIGRATED READERS.
///
/// A transcription is only an oracle while it stays one. A `baseline_*` body
/// that started calling a migrated reader — directly, or through the shipped
/// name, which is now a delegation to the migrated body — would move WITH that
/// body under any defect, and every A/B in this file would go green over a
/// broken reader. That is exactly the circularity Codex 019fe65a finding 4
/// named, reintroduced one level down and much harder to see.
///
/// So: the baselines may call the LIVE free functions, the memory-pure decode
/// helpers, and each other. Nothing else. The `_in` suffix and the shipped
/// names of everything this lane migrated are both forbidden.
///
/// (That the transcriptions MATCH b346f6c2 is a separate, point-in-time check,
/// recorded in `extractor/native/runs/gb_lanememory/oracle_transcription.md`:
/// each body stripped of comments and whitespace and compared character for
/// character against `git show b346f6c2:...attach_log.rs`. All six are
/// identical but for three grammatically inert trailing commas.)
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: someone "simplifying" a
/// baseline by calling the real reader, which is the single edit that turns
/// this whole file back into a set of assertions that cannot fail.
#[test]
fn the_oracles_are_independent_of_the_migrated_readers() {
    let source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("attach_log")
            .join("tests")
            .join("decode_memory.rs"),
    )
    .expect("this module's own source must be readable")
    .replace("\r\n", "\n");
    // SANITIZED, not raw: every law below asks a question about CODE, and
    // `sanitize_lines` is what makes the text it reads only code.
    let lines = sanitize_lines(&source);

    // The names an oracle may never reach: every shipped name this lane
    // migrated (each is now a delegation to a migrated body) and every generic
    // body behind them.
    // MATCHED AS WHOLE IDENTIFIERS, like the other two laws. A call-shaped
    // needle is escaped by binding the function first —
    // `let reader = read_msvc_string_with_policy; reader(addr, ..)` — and the
    // name is then never followed by `(`. That would leave the A/B comparing
    // the current implementation against itself while this law stayed green,
    // which is the exact circularity it exists to prevent. (Codex 019fe8db.)
    const FORBIDDEN: &[&str] = &[
        "read_msvc_string_with_policy_in",
        "read_msvc_string_with_snapshot_with_policy_in",
        "read_msvc_string_snapshot_once_in",
        "read_msvc_display_string_nullable_in",
        "read_msvc_token_nullable_in",
        "snapshot_u64_or_read_in",
        "snapshot_u32_or_read_in",
        "snapshot_ptr_or_read_in",
        "compact_object_window_in",
        "domain47_portrait_identity_with_snapshot_in",
        "read_msvc_string_with_policy",
        "read_msvc_string_with_snapshot_with_policy",
        "read_msvc_string_snapshot_once",
        "read_msvc_display_string_nullable",
        "read_msvc_token_nullable",
        "snapshot_u64_or_read",
        "snapshot_u32_or_read",
        "snapshot_ptr_or_read",
        "compact_object_window",
        "domain47_portrait_identity_with_snapshot",
        "read_in",
        // AND ITS SHIPPED COUNTERPART. `read_in` was listed and `read` was not,
        // so an oracle could call `ObjectSnapshot::<N>::read(addr)` — the LIVE
        // object window — and this law stayed green while the object-window A/B
        // compared the shipped reader against itself. Every other entry here
        // comes in a `_in`/shipped pair; this one was half a pair. (Codex
        // 019fe8e7.)
        //
        // Safe under whole-identifier matching, which is why it can be spelled
        // this bare: `read` does not match inside `read_in`, `read_into`,
        // `read_u64` or any other longer name.
        "read",
        "LIVE_MEMORY",
        "SnapshotViewMemory",
        "DecodeMemory",
    ];
    let mut scanned = 0usize;
    let starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.trim_start().starts_with("fn baseline_"))
        .map(|(index, _)| index)
        .collect();
    for start in starts {
        let end = lines[start..]
            .iter()
            .position(|line| *line == "}")
            .map(|offset| start + offset)
            .expect("every oracle closes at column 0");
        // THE WHOLE BODY, NORMALISED — the same treatment the other two laws
        // take. This law used to scan per line, so an oracle could reach
        // migrated code with the name and the parenthesis on different lines,
        // or through a turbofish, and stay green while every A/B in this file
        // went circular one level down. (Codex 019fe8b3, finding 3.)
        let code = body_code(&lines, start, end);
        for forbidden in FORBIDDEN {
            assert!(
                !names_identifier(&code, forbidden),
                "the oracle opened at line {} reaches `{forbidden}`: it is no \
                 longer independent of the code it is supposed to be an oracle for",
                start + 1
            );
        }
        scanned += 1;
    }
    // EVERY oracle, not "the ones that happened to be found". Nine baselines:
    // six transcribed bodies and three three-statement helpers.
    assert_eq!(
        scanned, 9,
        "the oracle scan found {scanned} `baseline_*` functions, not the 9 this \
         file defines — one was added or removed without this law being told"
    );
}

// ==================================================================
// 3. BEHAVIOURAL PINNING — the shipped reader decides exactly what the
//    pre-migration reader decided, refusals included.
// ==================================================================

/// One MSVC `std::string` staged for these A/Bs: a header object and a heap
/// buffer beside it, both inside the reader's heap window.
///
/// Deliberately a second, smaller fixture than `tests.rs`'s
/// `SharedStringFixture` rather than a widening of it: this file's A/Bs need to
/// vary the header AND leave the payload unreachable, and a fixture shared with
/// the rule-restating tests would have to grow a mode for each.
struct StringFixture {
    header: FixtureRegion,
    _heap: FixtureRegion,
}

impl StringFixture {
    /// `heap_payload: None` with `cap >= 16` leaves the pointer NULL, which is
    /// a buffer `range_confirmed_good` refuses — the readable-header,
    /// unreachable-payload case that exercises the fetch-cause path.
    fn stage(size: u64, cap: u64, inline: &[u8], heap_payload: Option<&[u8]>) -> Self {
        let mut header = FixtureRegion::new(0x40);
        let mut heap = FixtureRegion::new(0x80);
        header.write_bytes(0, &[0u8; 0x20]);
        if !inline.is_empty() {
            header.write_bytes(0, inline);
        }
        match heap_payload {
            Some(bytes) => {
                heap.write_bytes(0, bytes);
                header.write_u64(0, heap.addr() as u64);
            }
            None if cap >= 16 => header.write_u64(0, 0),
            None => {}
        }
        header.write_u64(0x10, size);
        header.write_u64(0x18, cap);
        page_cache_new_frame();
        StringFixture {
            header,
            _heap: heap,
        }
    }

    fn addr(&self) -> usize {
        self.header.addr()
    }

    fn snapshot(&self) -> ObjectSnapshot<0x40> {
        ObjectSnapshot::<0x40>::read(self.addr()).expect("the staged header must be readable")
    }
}

/// The header shapes every A/B below sweeps.
///
/// Chosen so each names a DIFFERENT refusal point of the readers, and so both
/// storage modes, both terminator outcomes and an unreachable payload are
/// present. Without the malformed half these tests would compare two functions
/// on the population where neither refuses, which is the vacuous shape.
///
/// `(label, size, capacity, inline bytes, heap bytes)`
const STRING_SHAPES: &[(&str, u64, u64, &[u8], Option<&[u8]>)] = &[
    ("inline well-formed", 5, 15, b"abcde\0", None),
    ("inline single char", 1, 15, b"a\0", None),
    ("inline no terminator", 5, 15, b"abcdex", None),
    ("inline non-ascii", 3, 15, b"\xc3\xa9z\0", None),
    ("size zero inline", 0, 15, b"\0", None),
    ("size zero heap capacity", 0, 31, b"", Some(b"\0")),
    ("size over max", 40, 63, b"", Some(b"0123456789")),
    ("capacity below size", 12, 4, b"", None),
    ("capacity implausible", 4, (1 << 20) + 1, b"", None),
    ("heap well-formed", 6, 31, b"", Some(b"heapy\0")),
    ("heap no terminator", 6, 31, b"", Some(b"heapyx")),
    ("heap non-utf8", 4, 31, b"", Some(b"\xe9\xe8\xea\xeb\0")),
    ("heap pointer null", 6, 31, b"", None),
    ("heap control byte", 4, 31, b"", Some(b"ab\x01c\0")),
    // THE TWO SHAPES THAT MAKE THE HEADER READS' WIDTH OBSERVABLE.
    //
    // Every other shape's size and capacity fit in 32 bits, so a size word read
    // as `u32` instead of `u64` yields the SAME number and the whole A/B goes
    // on passing over a reader that now truncates every header it sees. That is
    // the surviving-defect case Codex 019fe65a raised, reproduced by mutation
    // (M3) before these were added and caught by them afterwards.
    //
    // With the high word set the two widths disagree about the value and
    // therefore about the verdict: read as 8 bytes both refuse
    // (`SizeOverMax` / `CapacityImplausible`), read as 4 they are an ordinary
    // 1- and 4-byte heap string.
    // The capacity is ORDINARY on this one, deliberately: if both words carried
    // a high half, the capacity rule would refuse the narrowed read too and the
    // two widths would agree again by accident.
    ("size high word set", 0x1_0000_0001, 31, b"", Some(b"a\0")),
    ("capacity high word set", 4, 0x1_0000_0010, b"", Some(b"abcd\0")),
];

/// The maximum every A/B passes, small enough that "size over max" is reachable
/// and large enough that the well-formed shapes are not clipped by it.
const A_B_MAX_LEN: usize = 8;

/// THE BY-ADDRESS STRING READER DECIDES WHAT IT DECIDED BEFORE THE MIGRATION.
///
/// Against [`baseline_msvc_string_with_policy`], the b346f6c2 body reading the
/// live free functions by name — NOT against `read_msvc_string_with_policy_in`,
/// which the shipped name now delegates to and which is therefore not an
/// oracle for anything. (Codex 019fe65a, finding 4.)
///
/// The FULL result is compared: the string and the encoding on success, and on
/// failure the refusal's reason, its storage mode, its fetch cause and its
/// terminator value — every field `MsvcStringRefusal` carries and every one
/// `domain47_portrait_reader_reason` / `_pointer` / `_terminator` publish.
///
/// Both policies, because the tolerant ladder is a second decode path and the
/// migration touched the statement that dispatches to it.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: routing one of the body's three
/// reads through a different width (`mem.u32` for `mem.u64` on the size word),
/// a different order, or a different primitive — `mem.bytes` where the live
/// body called `read_bytes_with_reason` loses the fetch cause and shows up as
/// an equal reason with an unequal `fetch`.
#[test]
fn the_by_address_string_reader_pins_against_the_pre_migration_body() {
    let mut refusals = 0usize;
    let mut successes = 0usize;
    for policy in [
        MsvcStringDecodePolicy::Strict,
        MsvcStringDecodePolicy::TolerantWindows1252,
    ] {
        for (label, size, cap, inline, heap) in STRING_SHAPES {
            let fixture = StringFixture::stage(*size, *cap, inline, *heap);
            page_cache_new_frame();
            let shipped = read_msvc_string_with_policy(fixture.addr(), A_B_MAX_LEN, policy);
            page_cache_new_frame();
            let baseline =
                baseline_msvc_string_with_policy(fixture.addr(), A_B_MAX_LEN, policy);
            match &shipped {
                Ok(_) => successes += 1,
                Err(_) => refusals += 1,
            }
            assert_eq!(
                format!("{shipped:?}"),
                format!("{baseline:?}"),
                "`{label}` under {policy:?}: the shipped by-address reader and the \
                 pre-migration body disagree"
            );
        }
    }
    // THE SWEEP REALLY DID STAGE BOTH HALVES. Without this the assertions above
    // would pass over a population where every shape refused identically for
    // the trivial reason that none of them read anything.
    assert!(
        successes > 0 && refusals > 0,
        "the shapes must stage both accepted and refused headers, or the \
         equality proves nothing (successes {successes}, refusals {refusals})"
    );
}

/// THE SNAPSHOT-SERVED STRING READER DECIDES WHAT IT DECIDED BEFORE THE
/// MIGRATION.
///
/// Against [`baseline_msvc_string_with_snapshot_with_policy`], and it covers
/// ONE MORE branch than the by-address test: the `snapshot: None` fallback,
/// which re-enters the by-address body — over the SAME memory in the migrated
/// spelling, which is what a post-bracket decode requires and what the
/// behavioural sentinel further down proves.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: a header rule changing which
/// inputs refuse, the inline slice's bounds moving, or the heap payload fetch
/// losing its cause.
#[test]
fn the_snapshot_served_string_reader_pins_against_the_pre_migration_body() {
    let mut with_snapshot = 0usize;
    let mut without_snapshot = 0usize;
    let mut refusals = 0usize;
    for policy in [
        MsvcStringDecodePolicy::Strict,
        MsvcStringDecodePolicy::TolerantWindows1252,
    ] {
        for (label, size, cap, inline, heap) in STRING_SHAPES {
            let fixture = StringFixture::stage(*size, *cap, inline, *heap);
            let snapshot = fixture.snapshot();
            for held in [true, false] {
                let held_snapshot = held.then_some(&snapshot);
                page_cache_new_frame();
                let shipped = read_msvc_string_with_snapshot_with_policy(
                    fixture.addr(),
                    0,
                    A_B_MAX_LEN,
                    held_snapshot,
                    policy,
                );
                page_cache_new_frame();
                let baseline = baseline_msvc_string_with_snapshot_with_policy(
                    fixture.addr(),
                    0,
                    A_B_MAX_LEN,
                    held_snapshot,
                    policy,
                );
                if held {
                    with_snapshot += 1;
                } else {
                    without_snapshot += 1;
                }
                if shipped.is_err() {
                    refusals += 1;
                }
                assert_eq!(
                    format!("{shipped:?}"),
                    format!("{baseline:?}"),
                    "`{label}` under {policy:?} (snapshot held: {held}): the shipped \
                     snapshot-served reader and the pre-migration body disagree"
                );
            }
        }
    }
    // ONLY `refusals`. The two dispatch-arm counters followed from the
    // unconditional `[true, false]` inner loop and could not be zero (Codex
    // 019fe8b3, finding 4). That the HELD arm really serves from the snapshot
    // is a different question and has its own test:
    // `a_held_snapshot_is_the_source_of_the_header_not_the_process`.
    let _ = (with_snapshot, without_snapshot);
    assert!(
        refusals > 0,
        "no header refused, so agreement on refusal reasons is untested"
    );
}

/// THE REMAINING MIGRATED READERS DECIDE WHAT THEY DECIDED BEFORE THE
/// MIGRATION.
///
/// `read_msvc_string_snapshot_once` (behind every token cache and every
/// double-read stable token), `read_msvc_display_string_nullable` (the country
/// roster's display names) and `read_msvc_token_nullable` (the cosmetic-tag
/// readers), each against its b346f6c2 body. One test rather than three because
/// they take the same arguments over the same fixture and their obligation is
/// identical.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: any of the three dropping,
/// reordering or re-widening one of its reads — `read_msvc_string_snapshot_once`
/// in particular publishes `chars_addr`, so an inline/heap branch that flipped
/// would compare unequal here and nowhere else.
#[test]
fn the_nullable_and_once_readers_pin_against_the_pre_migration_bodies() {
    let mut nonempty = 0usize;
    let mut refusals = 0usize;
    for (label, size, cap, inline, heap) in STRING_SHAPES {
        let fixture = StringFixture::stage(*size, *cap, inline, *heap);

        page_cache_new_frame();
        let shipped_once = read_msvc_string_snapshot_once(fixture.addr(), A_B_MAX_LEN);
        page_cache_new_frame();
        let baseline_once = baseline_msvc_string_snapshot_once(fixture.addr(), A_B_MAX_LEN);
        // `MsvcStringSnapshot` carries no `Debug`, so its FOUR fields are
        // spelled out rather than compared through a derived formatting. Every
        // one of them is load-bearing: `chars_addr` and `bytes` together are
        // what keeps the address-keyed token caches from serving a stale token
        // after a realloc, so a migration that flipped the inline/heap branch
        // would be visible here and nowhere else.
        let fields = |snapshot: &Option<MsvcStringSnapshot>| {
            snapshot
                .as_ref()
                .map(|s| (s.size, s.cap, s.chars_addr, s.bytes.clone()))
        };
        assert_eq!(
            fields(&shipped_once),
            fields(&baseline_once),
            "`{label}`: `read_msvc_string_snapshot_once` and the pre-migration body \
             disagree"
        );

        page_cache_new_frame();
        let shipped_display = read_msvc_display_string_nullable(fixture.addr(), A_B_MAX_LEN);
        page_cache_new_frame();
        let baseline_display =
            baseline_msvc_display_string_nullable(fixture.addr(), A_B_MAX_LEN);
        assert_eq!(
            format!("{shipped_display:?}"),
            format!("{baseline_display:?}"),
            "`{label}`: `read_msvc_display_string_nullable` and the pre-migration \
             body disagree"
        );

        page_cache_new_frame();
        let shipped_token = read_msvc_token_nullable(fixture.addr(), A_B_MAX_LEN);
        page_cache_new_frame();
        let baseline_token = baseline_msvc_token_nullable(fixture.addr(), A_B_MAX_LEN);
        assert_eq!(
            format!("{shipped_token:?}"),
            format!("{baseline_token:?}"),
            "`{label}`: `read_msvc_token_nullable` and the pre-migration body disagree"
        );

        match &shipped_once {
            Some(snapshot) if !snapshot.bytes.is_empty() => nonempty += 1,
            _ => refusals += 1,
        }
    }
    assert!(
        nonempty > 0 && refusals > 0,
        "both accepted and refused headers must be staged ({nonempty} / {refusals})"
    );
}

/// THE OBJECT WINDOW AND THE FIELD HELPERS DECIDE WHAT THEY DECIDED BEFORE THE
/// MIGRATION.
///
/// `ObjectSnapshot::<N>::read` is ~30 call sites, `snapshot_*_or_read` another
/// 44, and all of them are on live frame paths today. Each is compared against
/// its b346f6c2 body. The A/B covers both of the field helpers' arms — served
/// from a held snapshot, and read live when none is held — and an unreadable
/// address, which is where the object window's fail-closed contract lives.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: the object window accepting a
/// partial fill (the contract is all-or-nothing), or a field helper's `None`
/// arm reading a different width or a different base.
#[test]
fn the_object_window_and_field_helpers_pin_against_the_pre_migration_bodies() {
    let mut region = FixtureRegion::new(0x40);
    region.write_u64(0x00, 0x0123_4567_89AB_CDEF);
    region.write_u64(0x08, 0xFEDC_BA98_7654_3210);
    region.write_u64(0x18, 0x1122_3344_5566_7788);
    page_cache_new_frame();
    let addr = region.addr();

    // The window itself, both outcomes.
    page_cache_new_frame();
    let shipped = ObjectSnapshot::<0x20>::read(addr);
    page_cache_new_frame();
    let baseline = baseline_object_snapshot::<0x20>(addr);
    assert_eq!(
        shipped.map(|snap| snap.buf),
        baseline,
        "the object window and the pre-migration body disagree on a readable object"
    );
    let snapshot = ObjectSnapshot::<0x20>::read(addr).expect("staged object is readable");

    // AN ADDRESS NOTHING BACKS. Not an arbitrary offset from a live address —
    // the fixture regions are allocated across one process and this suite runs
    // in parallel, so "somewhere far away" is readable whenever a neighbouring
    // fixture happens to sit there. Address 0 is unmapped by construction on
    // this target and is the same refusal every guarded read gives it.
    page_cache_new_frame();
    let shipped_dead = ObjectSnapshot::<0x20>::read(0);
    page_cache_new_frame();
    let baseline_dead = baseline_object_snapshot::<0x20>(0);
    assert!(
        shipped_dead.is_none() && baseline_dead.is_none(),
        "an unbacked address must refuse on both spellings"
    );

    // The field helpers, both arms.
    for held in [true, false] {
        let held_snapshot = held.then_some(&snapshot);
        for off in [0x00usize, 0x08, 0x18, 0x40] {
            page_cache_new_frame();
            let shipped_u64 = snapshot_u64_or_read(held_snapshot, addr, off);
            page_cache_new_frame();
            let baseline_u64 = baseline_snapshot_u64_or_read(held_snapshot, addr, off);
            assert_eq!(
                shipped_u64, baseline_u64,
                "snapshot_u64_or_read at +{off:#x} (snapshot held: {held})"
            );

            page_cache_new_frame();
            let shipped_u32 = snapshot_u32_or_read(held_snapshot, addr, off);
            page_cache_new_frame();
            let baseline_u32 = baseline_snapshot_u32_or_read(held_snapshot, addr, off);
            assert_eq!(
                shipped_u32, baseline_u32,
                "snapshot_u32_or_read at +{off:#x} (snapshot held: {held})"
            );

            // `snapshot_ptr_or_read` was a `as usize` of the u64 form before the
            // migration and still is, so its oracle is that form.
            page_cache_new_frame();
            let shipped_ptr = snapshot_ptr_or_read(held_snapshot, addr, off);
            assert_eq!(
                shipped_ptr,
                baseline_u64.map(|value| value as usize),
                "snapshot_ptr_or_read at +{off:#x} (snapshot held: {held})"
            );
        }
    }

    // `+0x40` is PAST the snapshot and inside the staged region, which is the
    // one offset where the two arms legitimately differ from each other — the
    // held arm refuses (out of window) and the live arm reads. Stated so the
    // sweep is known to have entered it.
    assert!(
        snapshot_u64_or_read(Some(&snapshot), addr, 0x40).is_none(),
        "the sweep must include an offset past the snapshot window"
    );

    for use_windows in [true, false] {
        page_cache_new_frame();
        let shipped_window = compact_object_window::<0x20>(addr, use_windows);
        page_cache_new_frame();
        let baseline_window = use_windows
            .then(|| baseline_object_snapshot::<0x20>(addr))
            .flatten();
        assert_eq!(
            shipped_window.map(|snap| snap.buf),
            baseline_window,
            "compact_object_window (use_windows: {use_windows})"
        );
    }
}

/// THE PORTRAIT DECODE DECIDES WHAT IT DECIDED BEFORE THE MIGRATION.
///
/// The acceptance decode, pinned against [`baseline_portrait_identity`] — the
/// b346f6c2 body sitting on the b346f6c2 string reader, so the whole chain is
/// independent of the migrated code — before it is pointed at a view. Both
/// halves of the result are compared: the accepted
/// identity WITH ITS PROVENANCE (`MsvcStringEncoding`, which is what the wire's
/// `portrait_cp1252_identities` counts) and the refusal's detail AND its whole
/// probe — size, capacity, pointer outcome, terminator outcome, terminator
/// value and reader reason, which are seven separate wire words.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: the ladder losing its
/// UTF-8-first order (a genuinely UTF-8 identity would come back as
/// Windows-1252 mojibake with a different encoding word), the empty-heap
/// terminator read moving off the injected memory, or any probe field being
/// filled from a read the refusing path did not make.
#[test]
fn the_portrait_decode_pins_against_the_pre_migration_body() {
    const GFX_OFF: usize = domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_GFX_OFF;
    const ENTRY_LEN: usize = domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_LEN;

    let mut accepted = 0usize;
    let mut refused = 0usize;
    let mut cp1252 = 0usize;
    for (label, size, cap, inline, heap) in PORTRAIT_SHAPES {
        let mut entry = FixtureRegion::new(ENTRY_LEN + 0x40);
        let mut heap_region = FixtureRegion::new(0x80);
        entry.write_bytes(0, &vec![0u8; ENTRY_LEN]);
        if !inline.is_empty() {
            entry.write_bytes(GFX_OFF, inline);
        }
        match heap {
            Some(bytes) => {
                heap_region.write_bytes(0, bytes);
                entry.write_u64(GFX_OFF, heap_region.addr() as u64);
            }
            None if *cap >= 16 => entry.write_u64(GFX_OFF, 0),
            None => {}
        }
        entry.write_u64(GFX_OFF + 0x10, *size);
        entry.write_u64(GFX_OFF + 0x18, *cap);
        page_cache_new_frame();

        let window = ObjectSnapshot::<ENTRY_LEN>::read(entry.addr())
            .expect("the staged portrait entry must be readable");
        page_cache_new_frame();
        let shipped = domain47_portrait_identity_with_snapshot(entry.addr(), &window);
        page_cache_new_frame();
        let baseline = baseline_portrait_identity(entry.addr(), &window);
        match &shipped {
            Ok(Some((_, MsvcStringEncoding::Windows1252))) => {
                accepted += 1;
                cp1252 += 1;
            }
            Ok(_) => accepted += 1,
            Err(_) => refused += 1,
        }
        assert_eq!(
            format!("{shipped:?}"),
            format!("{baseline:?}"),
            "`{label}`: the shipped portrait decode and the pre-migration body \
             disagree"
        );
    }
    assert!(
        // `accepted > 0` was here and is implied: the Windows-1252 arm
        // increments BOTH counters, so `cp1252 > 0` already says an identity
        // was accepted. (Codex 019fe8b3, finding 4.)
        refused > 0 && cp1252 > 0,
        "the shapes must stage refused AND Windows-1252 identities, or the \
         tolerant ladder is not being compared at all \
         ({accepted} accepted / {refused} refused / {cp1252} cp1252)"
    );
}

/// Portrait entry shapes. Same form as [`STRING_SHAPES`], with the two the
/// portrait decode has of its own: the empty-with-heap-capacity case whose
/// terminator is fetched through the injected memory, and a real cp1252
/// identity of the kind char 5137 carries.
///
/// `(label, size, capacity, inline bytes, heap bytes)`
const PORTRAIT_SHAPES: &[(&str, u64, u64, &[u8], Option<&[u8]>)] = &[
    ("empty inline", 0, 15, b"\0", None),
    ("empty inline unterminated", 0, 15, b"x", None),
    ("empty heap capacity", 0, 31, b"", Some(b"\0")),
    ("empty heap unterminated", 0, 31, b"", Some(b"x")),
    ("empty heap pointer null", 0, 31, b"", None),
    ("empty capacity implausible", 0, (1 << 20) + 1, b"", None),
    ("ascii inline", 5, 15, b"gfx_a\0", None),
    ("ascii heap", 9, 31, b"", Some(b"gfx_hello\0")),
    // NOT valid UTF-8 and valid Windows-1252: the population the tolerant
    // ladder exists for.
    ("cp1252 heap", 6, 31, b"", Some(b"g\xe9n\xe9ra\0")),
    // Valid under BOTH encodings. UTF-8 must win, or the identity is mojibake.
    ("utf8 heap", 3, 31, b"", Some(b"\xc3\xa9z\0")),
    ("heap unterminated", 5, 31, b"", Some(b"gfx_ax")),
    ("heap control byte", 4, 31, b"", Some(b"gf\x01x\0")),
    ("size over max", 400, 511, b"", Some(b"gfx\0")),
];

// ==================================================================
// 3. THE VIEW ADAPTER's own contract.
// ==================================================================

/// A HELD SNAPSHOT IS REALLY WHERE THE HEADER COMES FROM.
///
/// `the_snapshot_served_string_reader_pins_against_the_pre_migration_body`
/// sweeps both dispatch arms, but it counts LOOP INPUTS: it passes
/// `Some(snapshot)` and `None` and asserts each happened. That does not
/// establish which arm the reader TOOK, because in that fixture the snapshot
/// holds the same bytes as live memory, so a reader that ignored the snapshot
/// and re-read the process would agree on every shape. (Codex 019fe678,
/// finding 2.)
///
/// This makes the two sources DISAGREE. The snapshot is taken, then the live
/// header is rewritten. A reader served by the snapshot must answer from the
/// bytes it was given; a reader that went back to the process must answer from
/// the new ones. They are an acceptance and a refusal, so no fixture accident
/// can make them equal.
///
/// That property is not new — it is what `ObjectSnapshot` has always been for,
/// and the whole point of the snapshot-served reader — but nothing pinned it,
/// and this lane routed that reader through a new memory parameter, which is
/// exactly when an unpinned property is worth pinning.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: the `Some(snapshot)` arm
/// re-reading the header by address (through `mem` or otherwise) instead of
/// serving it from the buffer — which widens the tearing window the object
/// window exists to narrow, and would look like a pure spelling change.
#[test]
fn a_held_snapshot_is_the_source_of_the_header_not_the_process() {
    let mut fixture = StringFixture::stage(5, 15, b"abcde\0", None);
    let snapshot = fixture.snapshot();

    // The engine rewrites the header AFTER the window was taken.
    fixture.header.write_u64(0x10, 0);
    page_cache_new_frame();

    let from_snapshot = read_msvc_string_with_snapshot_with_policy(
        fixture.addr(),
        0,
        A_B_MAX_LEN,
        Some(&snapshot),
        MsvcStringDecodePolicy::Strict,
    );
    let from_process = read_msvc_string_with_snapshot_with_policy::<0x40>(
        fixture.addr(),
        0,
        A_B_MAX_LEN,
        None,
        MsvcStringDecodePolicy::Strict,
    );

    assert_eq!(
        from_snapshot.as_ref().map(|(text, _)| text.as_str()),
        Ok("abcde"),
        "the held snapshot's header was not used; the reader went back to the \
         process, which is a wider tearing window than the caller asked for"
    );
    assert!(
        matches!(
            &from_process,
            Err(refusal) if refusal.reason == MsvcStringRefusalReason::SizeZero
        ),
        "the by-address arm must see the REWRITTEN header, or the two sources \
         never disagreed and this test proves nothing: {from_process:?}"
    );
}

/// A READ SPANNING TWO COPY TRANSACTIONS REFUSES — and that is a DIVERGENCE
/// from the live path in its own right, in the opposite direction from the one
/// the non-stitching form was chosen to prevent.
///
/// # What this lane got wrong first
///
/// An earlier version of this test claimed the seam was not producible, on the
/// strength of a measurement over two roots of ONE recipe: adjacent copies
/// coalesce into one interval, and non-adjacent ones leave a hole both view
/// spellings refuse. That measurement is correct and is kept below — but it did
/// not sample the topology that matters. TWO RECIPES sharing ONE
/// `ConsumerId` acquire adjacent spans as two transactions that do NOT
/// coalesce, and `snapshot_recipes/tests.rs`'s
/// `a_read_spanning_two_transactions_stitches_and_a_borrow_refuses` has proved
/// exactly that all along. `ViewRefusal::NotContiguous` is reachable. The claim
/// is retracted. (Codex 019fe887, finding 1.)
///
/// # The divergence, stated in both directions
///
/// The live gate and the copy's transaction boundaries are DIFFERENT
/// partitions of the same addresses, so neither view spelling matches the live
/// reader everywhere:
///
///  * STITCHING admits where live refuses — a span crossing two PROCESS regions
///    that the plan happened to copy is served off the arena, while
///    `range_confirmed_good` refuses it live. That is a row the live path never
///    publishes, and it is why round 1 asked for the non-stitching form.
///  * NOT STITCHING refuses where live admits — this test. Sixteen bytes inside
///    ONE confirmed process region, copied as two transactions because two
///    recipes asked for them, are all present in the arena and are refused
///    anyway.
///
/// The current spelling is the second, and it is kept because its error is
/// FAIL-CLOSED: it loses a row rather than inventing one, which is the
/// direction this project takes everywhere else. But it IS a loss, it is
/// reachable, and which partition the copy path should honour is an owner
/// decision rather than a lane's — it is reported as a residual.
///
/// What this test does is make the behaviour explicit and un-driftable: the
/// straddle is staged, the refusal is asserted with the word it reaches a
/// decoder as, and the fully-captured bytes are shown to be present.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: any `SnapshotViewMemory`
/// primitive going back to `SnapshotView::read_into`, which would flip the
/// divergence to the admitting direction without anyone deciding to.
#[test]
fn a_view_read_spanning_two_transactions_refuses_and_that_is_a_known_loss() {
    use crate::attach_log::d81_copy_bracket::Domain81SnapshotMemory;
    use crate::snapshot_recipes::executor::{observe, ObservationConfig, Recipe, RecipeOp, Root};
    use crate::snapshot_recipes::view::ViewRefusal;
    use crate::snapshot_recipes::{ConsumerId, HourStamp, RecipeId, Span};

    const HOUR: HourStamp = HourStamp(19_360_403);
    const CONSUMER: ConsumerId = ConsumerId(47);

    let mut region = FixtureRegion::new(0x40);
    region.write_u64(0x00, 0x1111_1111_1111_1111);
    region.write_u64(0x08, 0x2222_2222_2222_2222);
    page_cache_new_frame();
    let addr = region.addr() as u64;

    // TWO RECIPES, ONE CONSUMER. This is the arrangement that does not
    // coalesce, and it is not exotic: it is what any two domains — or one
    // domain with two plans — asking for neighbouring objects produce.
    let leg = |id: RecipeId, at: u64| {
        Recipe::new(
            id,
            CONSUMER,
            vec![
                RecipeOp::CopyHeader {
                    size: 8,
                    next: vec![1],
                },
                RecipeOp::Freeze,
            ],
            vec![Root { op: 0, addr: at }],
        )
    };
    let build = observe(
        &[leg(RecipeId(150), addr), leg(RecipeId(151), addr + 8)],
        &Domain81SnapshotMemory,
        ObservationConfig::new(HOUR),
    )
    .expect("the fixture is far inside the custody ceiling");
    let view = build
        .snapshot
        .view_for(CONSUMER, HOUR)
        .expect("the plan captured its consumer");

    // THE PRECONDITION, AND IT IS REAL THIS TIME. Two intervals, both owned,
    // together covering the span — so the bytes ARE captured and the refusal
    // below is a loss, not a gap.
    assert_eq!(
        build.snapshot.interval_count(),
        2,
        "the two recipes coalesced, so there is no seam and this test is about \
         nothing"
    );
    let mut stitched = [0u8; 16];
    assert!(
        view.read_into(addr, &mut stitched).is_ok(),
        "the stitching form must serve the span, or the bytes were not captured"
    );
    assert_eq!(
        stitched[..8],
        0x1111_1111_1111_1111u64.to_le_bytes(),
        "the captured bytes are the engine's"
    );
    assert_eq!(
        view.bytes(addr, 16).err(),
        Some(ViewRefusal::NotContiguous {
            span: Span::new(addr, 16),
        }),
        "the non-stitching form must refuse the seam by name"
    );

    let memory = SnapshotViewMemory::new(view);

    // ALL THREE PRIMITIVES REFUSE IT, and each says the one word a decoder's
    // vocabulary has.
    assert_eq!(
        DecodeMemory::scalar::<8>(&memory, (addr + 4) as usize),
        None,
        "a scalar straddling two transactions"
    );
    assert_eq!(
        memory.bytes_with_reason(addr as usize, 16),
        Err(GuardedReadRefusal::OutsideConfirmedRanges),
        "a payload straddling two transactions"
    );
    assert_eq!(
        ObjectSnapshot::<16>::read_in(&memory, addr as usize).map(|snap| snap.buf),
        None,
        "an object window straddling two transactions"
    );

    // AND EACH TRANSACTION IS SERVED ON ITS OWN, so the refusals are about the
    // seam and not about the plan having copied nothing.
    assert_eq!(
        DecodeMemory::scalar::<8>(&memory, addr as usize),
        Some(0x1111_1111_1111_1111u64.to_le_bytes())
    );
    assert_eq!(
        DecodeMemory::scalar::<8>(&memory, (addr + 8) as usize),
        Some(0x2222_2222_2222_2222u64.to_le_bytes())
    );
}

/// WITHIN ONE RECIPE, adjacent copies coalesce and non-adjacent ones leave a
/// hole. Kept because it is what makes the test above's arrangement the
/// INTERESTING one rather than the only one anybody tried.
///
/// It is also the measurement that says a single plan cannot accidentally
/// create a seam inside an object it copies in one op — which is why d47's
/// portrait proof never meets one.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: the substrate no longer
/// coalescing adjacent copies within a recipe, which would make seams far more
/// common than the two-recipe case and would raise the residual's urgency.
#[test]
fn within_one_recipe_adjacent_copies_coalesce_and_holes_refuse_on_both_forms() {
    use crate::attach_log::d81_copy_bracket::Domain81SnapshotMemory;
    use crate::snapshot_recipes::executor::{observe, ObservationConfig, Recipe, RecipeOp, Root};
    use crate::snapshot_recipes::{ConsumerId, HourStamp, RecipeId};

    const HOUR: HourStamp = HourStamp(19_360_404);
    const CONSUMER: ConsumerId = ConsumerId(47);
    const PLAN: RecipeId = RecipeId(152);

    let mut coalesced = 0usize;
    let mut holed = 0usize;
    for gap in [0u64, 8, 24, 40, 64, 128] {
        let mut region = FixtureRegion::new(0x400);
        region.write_u64(0x00, 0x1111_1111_1111_1111);
        let base = region.addr() as u64;
        page_cache_new_frame();
        let plan = Recipe::new(
            PLAN,
            CONSUMER,
            vec![
                RecipeOp::CopyHeader {
                    size: 8,
                    // `Freeze` and not an empty `next`: a copy op with no
                    // successor is a `DanglingBranch` and the recipe refuses.
                    next: vec![1],
                },
                RecipeOp::Freeze,
                RecipeOp::CopyHeader {
                    size: 8,
                    next: vec![1],
                },
            ],
            vec![
                Root { op: 0, addr: base },
                Root {
                    op: 2,
                    addr: base + 8 + gap,
                },
            ],
        );
        let build = observe(
            &[plan],
            &Domain81SnapshotMemory,
            ObservationConfig::new(HOUR),
        )
        .expect("the fixture is far inside the custody ceiling");
        let view = build
            .snapshot
            .view_for(CONSUMER, HOUR)
            .expect("the plan captured its consumer");
        let len = (16 + gap) as usize;
        let mut sink = vec![0u8; len];
        let stitched = view.read_into(base, &mut sink).is_ok();
        let whole = view.bytes(base, len).is_ok();
        if gap == 0 {
            assert!(
                stitched && whole && build.snapshot.interval_count() == 1,
                "adjacent copies within one recipe no longer coalesce (intervals \
                 {})",
                build.snapshot.interval_count()
            );
            coalesced += 1;
        } else {
            assert!(
                !stitched && !whole,
                "gap {gap}: a span crossing an uncopied hole was served; \
                 stitching joins intervals, it does not invent the gap"
            );
            holed += 1;
        }
    }
    // NO `coalesced == 1 && holed == 5` TALLY. It followed from the hard-coded
    // six inputs and two unconditional increments, so it could not fail (Codex
    // 019fe8a1, finding 3). The per-gap assertions inside the loop are what
    // carry this test, and the gap list is right there to read.
    let _ = (coalesced, holed);
}

/// THE ACCEPTANCE PROOF. d47's portrait identity decode, run over a SNAPSHOT

/// THE ACCEPTANCE PROOF. d47's portrait identity decode, run over a SNAPSHOT
/// VIEW, agreeing with the same decode over the engine — values, provenance
/// and refusal REASONS.
///
/// # Why this decode
///
/// It is lane FW's residual 4. FW proved d47's identity reads
/// (`read_character_raw_fields`, `read_advisor_raw_fields`) equivalent over its
/// copy, because those were already generic over the LANE's `LaneMemory`. It
/// could not do the same for the portrait identity and said why: that decode
/// reaches the live process through three different SHARED free functions —
/// the object window, the guarded bulk read, and the snapshot-served MSVC
/// string reader with its tolerant ladder — none of which took an injected
/// memory. It is therefore the decode that exercises the whole of this lane's
/// seam rather than a corner of it, and the one whose conversion was blocked.
///
/// # What is being compared
///
/// The SAME function, twice, with two different memories: once with
/// `LIVE_MEMORY` (which is what production calls) and once with a
/// `SnapshotViewMemory` over a frozen copy of the same objects. Not two
/// implementations — `domain47_portrait_identity_with_snapshot_in` has one
/// body, and `the_live_forms_are_pure_delegations_to_the_generic_bodies` holds
/// that the shipped name is that body.
///
/// The whole result is compared, which for a refusal is the detail AND the
/// seven-field probe: size, capacity, pointer outcome, terminator outcome,
/// terminator value and the shared reader's own refusal reason. A copy that
/// produced the right string but a different reason on the failures would fail
/// here, and that is the point: the wire carries the reason.
///
/// # The copy plan
///
/// Bulk copies only, as the architecture requires: the entry body, and the
/// gfx string's payload as a guarded NUL-trailing byte extent. Every decision
/// — the storage-mode branch, the four header rules, the terminator test, the
/// UTF-8-then-cp1252 ladder — runs AFTER, on the copy, and is the production
/// code running there.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: any read inside the decode
/// going back to the live process (it would succeed over the engine and refuse
/// over the copy, or worse, succeed on both and silently mix hours), the
/// tolerant ladder losing its UTF-8-first order, or the view adapter's whole-
/// range rule diverging from `read_into`'s.
#[test]
fn the_portrait_decode_reads_the_same_identities_off_a_copy_as_off_the_engine() {
    use crate::attach_log::d81_copy_bracket::Domain81SnapshotMemory;
    use crate::snapshot_recipes::executor::{
        observe, ExtentGuard, ObservationConfig, Recipe, RecipeOp, Root,
    };
    use crate::snapshot_recipes::{ConsumerId, HourStamp, RecipeId};

    const GFX_OFF: usize = domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_GFX_OFF;
    const ENTRY_LEN: usize = domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_LEN;
    const HOUR: HourStamp = HourStamp(19_360_401);
    const CONSUMER: ConsumerId = ConsumerId(47);
    const PLAN: RecipeId = RecipeId(147);
    /// Generous enough that no shape's payload is clipped by the PLAN — the
    /// bound this proof is about is the DECODER's
    /// (`DOMAIN47_CCHARACTER_STRING_MAX_LEN`), and a plan that refused first
    /// would hide it.
    const MAX_PAYLOAD: u32 = 512;

    let mut compared = 0usize;
    let mut accepted = 0usize;
    let mut refused = 0usize;
    let mut cp1252 = 0usize;
    let mut heap_backed = 0usize;
    let mut inline_backed = 0usize;

    for (label, size, cap, inline, heap) in PORTRAIT_SHAPES {
        // Each shape gets its OWN observation. A shared one would let a shape
        // whose copy refused gap the consumer for every other shape, and the
        // per-shape verdict is what a conversion needs to see.
        let mut entry = FixtureRegion::new(ENTRY_LEN + 0x40);
        let mut heap_region = FixtureRegion::new(0x400);
        entry.write_bytes(0, &vec![0u8; ENTRY_LEN]);
        if !inline.is_empty() {
            entry.write_bytes(GFX_OFF, inline);
        }
        match heap {
            Some(bytes) => {
                heap_region.write_bytes(0, bytes);
                entry.write_u64(GFX_OFF, heap_region.addr() as u64);
                heap_backed += 1;
            }
            None if *cap >= 16 => entry.write_u64(GFX_OFF, 0),
            None => inline_backed += 1,
        }
        entry.write_u64(GFX_OFF + 0x10, *size);
        entry.write_u64(GFX_OFF + 0x18, *cap);
        page_cache_new_frame();

        // THE COPY PLAN. Two ops and no decisions: the entry body, and the
        // payload as a guarded extent whose guard is the capacity word (below
        // 16 the characters are inside the body already copied, so the branch
        // ends without a dereference — the `_Myres >= 16` rule every MSVC
        // string extent in this substrate is built on).
        let plan = Recipe::new(
            PLAN,
            CONSUMER,
            vec![
                RecipeOp::CopyHeader {
                    size: ENTRY_LEN as u32,
                    next: vec![1],
                },
                RecipeOp::DecodeExtent {
                    count_offset: (GFX_OFF + 0x10) as u32,
                    count_width: 8,
                    ptr_offset: GFX_OFF as u32,
                    stride: 1,
                    max_count: MAX_PAYLOAD,
                    max_bytes: u64::from(MAX_PAYLOAD),
                    // The NUL the decoder tests for.
                    trailer_elements: 1,
                    guard: Some(ExtentGuard::at_least(
                        (GFX_OFF + 0x18) as u32,
                        8,
                        16,
                    )),
                    register: None,
                    element: Vec::new(),
                },
            ],
            vec![Root {
                op: 0,
                addr: entry.addr() as u64,
            }],
        );

        page_cache_new_frame();
        let build = observe(
            &[plan],
            &Domain81SnapshotMemory,
            ObservationConfig::new(HOUR),
        )
        .expect("the fixture is far inside the custody ceiling");
        let Ok(view) = build.snapshot.view_for(CONSUMER, HOUR) else {
            panic!("`{label}`: the copy plan gapped its consumer, so nothing can be compared");
        };
        let view_memory = SnapshotViewMemory::new(view);

        // The window itself comes off the copy, through the SAME
        // `ObjectSnapshot` the live census uses.
        let live_window = ObjectSnapshot::<ENTRY_LEN>::read(entry.addr())
            .expect("the staged entry is readable live");
        let copied_window = ObjectSnapshot::<ENTRY_LEN>::read_in(&view_memory, entry.addr())
            .unwrap_or_else(|| panic!("`{label}`: the plan did not copy the entry body"));
        assert_eq!(
            live_window.buf, copied_window.buf,
            "`{label}`: the copied entry body is not the engine's bytes"
        );

        page_cache_new_frame();
        // The live side is the SHIPPED name — the exact production entry point,
        // pinned to the pre-migration body by
        // `the_portrait_decode_pins_against_the_pre_migration_body`.
        let live = domain47_portrait_identity_with_snapshot(entry.addr(), &live_window);
        let copied = domain47_portrait_identity_with_snapshot_in(
            &view_memory,
            entry.addr(),
            &copied_window,
        );
        match &live {
            Ok(Some((_, MsvcStringEncoding::Windows1252))) => {
                accepted += 1;
                cp1252 += 1;
            }
            Ok(_) => accepted += 1,
            Err(_) => refused += 1,
        }
        assert_eq!(
            format!("{live:?}"),
            format!("{copied:?}"),
            "`{label}`: the portrait identity decodes differently off the copy \
             than off the engine"
        );
        compared += 1;
    }

    println!(
        "d47 portrait identity, copy vs engine: {compared} entries \
         ({accepted} accepted, of which {cp1252} Windows-1252; {refused} refusing \
         identically; {heap_backed} heap-backed, {inline_backed} inline)"
    );
    // THE POPULATION IS NOT DEGENERATE. Without these the assertions above
    // would pass over a set where every entry refused for the same trivial
    // reason, which is the vacuous shape this whole proof exists to avoid.
    //
    // `compared == PORTRAIT_SHAPES.len()` used to be here and was dropped: the
    // loop increments it unconditionally, so it could not fail (Codex 019fe887,
    // finding 3). The three counters below can, and each names a property the
    // proof needs.
    let _ = compared;
    assert!(
        // `accepted > 0` is implied by `cp1252 > 0`: the same arm increments both.
        refused > 0 && cp1252 > 0,
        "refused and Windows-1252 identities must both be present \
         ({accepted} accepted / {refused} refused / {cp1252} cp1252)"
    );
    assert!(
        heap_backed > 0 && inline_backed > 0,
        "both storage modes must be present ({heap_backed} / {inline_backed})"
    );
}

/// THE SENTINEL. A decode over the copy CANNOT see a write the engine made
/// after the bracket closed — not in the object body, and not in the heap
/// payload behind it.
///
/// # Why the proof above is not enough on its own
///
/// In that test the staged objects are readable BOTH ways: the fixture is
/// ordinary process memory, so a decode that took a view and then reached into
/// the live process for one field would read the same bytes and agree anyway.
/// Every value would match, the test would pass, and the conversion would be
/// broken in exactly the way the owner named as a stop condition — an hour's
/// row carrying a value read outside that hour.
///
/// So this stages the one condition that tells the two memories apart: the
/// engine's bytes CHANGE after the observation. Anything the decode reads from
/// the copy must still be the pre-change value, and the same decode over the
/// live memory must see the post-change one. If the two agree, a read leaked.
///
/// Both a HEADER field (the size word, inside the copied body) and a PAYLOAD
/// field (the heap characters, behind a pointer) are moved, because they are
/// reached by different mechanisms — the object window and the guarded bulk
/// read — and a leak in either is a leak.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: any statement in
/// `domain47_portrait_identity_with_snapshot_in`, in
/// `read_msvc_string_with_snapshot_with_policy_in` or in
/// `read_msvc_string_with_policy_in` calling a live reader by name instead of
/// going through `mem` — which is the missed call site that
/// `no_generic_decode_body_reaches_the_live_process_by_name` states as source
/// law and this states as behaviour.
#[test]
fn a_decode_over_the_copy_cannot_see_a_write_made_after_the_bracket() {
    use crate::attach_log::d81_copy_bracket::Domain81SnapshotMemory;
    use crate::snapshot_recipes::executor::{
        observe, ExtentGuard, ObservationConfig, Recipe, RecipeOp, Root,
    };
    use crate::snapshot_recipes::{ConsumerId, HourStamp, RecipeId};

    const GFX_OFF: usize = domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_GFX_OFF;
    const ENTRY_LEN: usize = domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_LEN;
    const HOUR: HourStamp = HourStamp(19_360_402);
    const CONSUMER: ConsumerId = ConsumerId(47);
    const PLAN: RecipeId = RecipeId(148);

    let plan_for = |addr: u64| {
        Recipe::new(
            PLAN,
            CONSUMER,
            vec![
                RecipeOp::CopyHeader {
                    size: ENTRY_LEN as u32,
                    next: vec![1],
                },
                RecipeOp::DecodeExtent {
                    count_offset: (GFX_OFF + 0x10) as u32,
                    count_width: 8,
                    ptr_offset: GFX_OFF as u32,
                    stride: 1,
                    max_count: 512,
                    max_bytes: 512,
                    trailer_elements: 1,
                    guard: Some(ExtentGuard::at_least((GFX_OFF + 0x18) as u32, 8, 16)),
                    register: None,
                    element: Vec::new(),
                },
            ],
            vec![Root { op: 0, addr }],
        )
    };

    // ---- 1. THE PAYLOAD MOVES. Same header, different characters.
    let mut entry = FixtureRegion::new(ENTRY_LEN + 0x40);
    let mut heap_region = FixtureRegion::new(0x400);
    entry.write_bytes(0, &vec![0u8; ENTRY_LEN]);
    heap_region.write_bytes(0, b"gfx_before\0");
    entry.write_u64(GFX_OFF, heap_region.addr() as u64);
    entry.write_u64(GFX_OFF + 0x10, 10);
    entry.write_u64(GFX_OFF + 0x18, 31);
    page_cache_new_frame();

    let build = observe(
        &[plan_for(entry.addr() as u64)],
        &Domain81SnapshotMemory,
        ObservationConfig::new(HOUR),
    )
    .expect("the fixture is far inside the custody ceiling");
    let view = build
        .snapshot
        .view_for(CONSUMER, HOUR)
        .expect("the plan captured its consumer");
    let view_memory = SnapshotViewMemory::new(view);

    // THE BRACKET IS CLOSED. Everything below happens in the engine's memory
    // after the copy was taken — which is exactly what the game does while the
    // post-bracket decode runs.
    heap_region.write_bytes(0, b"gfx_after_\0");
    page_cache_new_frame();

    // THE OBJECT WINDOW IS TAKEN AFTER THE WRITE, deliberately. Taking it
    // before would leave `ObjectSnapshot::read_in` outside this sentinel: a
    // `read_in` that filled its buffer from the live process would have been
    // handed the same bytes the copy holds, and the leak would compare equal.
    // Taken here, a leaking window sees "gfx_after_" and the assertion below
    // fails. (Codex 019fe887, finding 2.)
    let copied_window = ObjectSnapshot::<ENTRY_LEN>::read_in(&view_memory, entry.addr())
        .expect("the plan copied the entry body");

    let live_window = ObjectSnapshot::<ENTRY_LEN>::read(entry.addr())
        .expect("the staged entry is still readable live");
    let from_copy =
        domain47_portrait_identity_with_snapshot_in(&view_memory, entry.addr(), &copied_window);
    let from_engine =
        domain47_portrait_identity_with_snapshot_in(&LIVE_MEMORY, entry.addr(), &live_window);

    assert_eq!(
        from_copy.as_ref().map(|id| id.as_ref().map(|(text, _)| text.as_str())),
        Ok(Some("gfx_before")),
        "the decode over the copy read a payload the bracket did not take"
    );
    assert_eq!(
        from_engine.as_ref().map(|id| id.as_ref().map(|(text, _)| text.as_str())),
        Ok(Some("gfx_after_")),
        "the decode over the live memory did not see the post-bracket write, so \
         the fixture never entered the condition this test is about"
    );

    // ---- 2. THE HEADER MOVES. The size word is INSIDE the copied body, so
    // this is the object-window half of the same question: a torn header the
    // engine wrote after the bracket must not reach a reader of the copy.
    let mut entry = FixtureRegion::new(ENTRY_LEN + 0x40);
    let mut heap_region = FixtureRegion::new(0x400);
    entry.write_bytes(0, &vec![0u8; ENTRY_LEN]);
    heap_region.write_bytes(0, b"gfx_header\0");
    entry.write_u64(GFX_OFF, heap_region.addr() as u64);
    entry.write_u64(GFX_OFF + 0x10, 10);
    entry.write_u64(GFX_OFF + 0x18, 31);
    page_cache_new_frame();

    let build = observe(
        &[plan_for(entry.addr() as u64)],
        &Domain81SnapshotMemory,
        ObservationConfig::new(HOUR),
    )
    .expect("the fixture is far inside the custody ceiling");
    let view = build
        .snapshot
        .view_for(CONSUMER, HOUR)
        .expect("the plan captured its consumer");
    let view_memory = SnapshotViewMemory::new(view);

    // Past `DOMAIN47_CCHARACTER_STRING_MAX_LEN`, so the live decode refuses by
    // NAME (`SizeOverMax` -> `NonEmptyTextUnreadable`) where the copy still
    // reads a whole identity. The two answers are not merely different values,
    // they are an acceptance and a refusal.
    entry.write_u64(GFX_OFF + 0x10, 400);
    page_cache_new_frame();

    // THE OBJECT WINDOW IS TAKEN AFTER THE WRITE, and THIS is the part where
    // that discriminates: the rewritten word is INSIDE the entry body, so a
    // `ObjectSnapshot::read_in` that filled its buffer from the live process
    // would carry `size = 400` and the copy would refuse exactly as the engine
    // does. Part 1 cannot catch that — there only the heap payload moves, and
    // the body is identical on both sides. (Codex 019fe887, finding 2.)
    let copied_window = ObjectSnapshot::<ENTRY_LEN>::read_in(&view_memory, entry.addr())
        .expect("the plan copied the entry body");

    let live_window = ObjectSnapshot::<ENTRY_LEN>::read(entry.addr())
        .expect("the staged entry is still readable live");
    let from_copy =
        domain47_portrait_identity_with_snapshot_in(&view_memory, entry.addr(), &copied_window);
    let from_engine =
        domain47_portrait_identity_with_snapshot_in(&LIVE_MEMORY, entry.addr(), &live_window);

    assert_eq!(
        from_copy.as_ref().map(|id| id.as_ref().map(|(text, _)| text.as_str())),
        Ok(Some("gfx_header")),
        "the decode over the copy read a header the bracket did not take"
    );
    assert!(
        matches!(
            &from_engine,
            Err(refusal)
                if refusal.detail == Domain47PortraitIdentityDetail::NonEmptyTextUnreadable
        ),
        "the live decode must refuse the post-bracket header, or the fixture \
         never entered the condition this test is about: {from_engine:?}"
    );

    // ---- 3. THE EMPTY-HEAP TERMINATOR MOVES.
    //
    // Its own case, because it is the ONE read in this decode that is not
    // routed through the shared string reader: an empty string with a heap
    // capacity is a proven-empty identity only if its buffer really carries a
    // NUL, and the decode fetches that single byte itself. Parts 1 and 2 both
    // stage non-empty identities and never reach it, so without this a leak on
    // exactly that statement would be caught by source law alone — and source
    // law is the check a future refactor is most likely to satisfy while
    // breaking the behaviour. (Found by mutating that statement back to the
    // live reader: only the source-law test failed.)
    let mut entry = FixtureRegion::new(ENTRY_LEN + 0x40);
    let mut heap_region = FixtureRegion::new(0x400);
    entry.write_bytes(0, &vec![0u8; ENTRY_LEN]);
    heap_region.write_bytes(0, b"\0");
    entry.write_u64(GFX_OFF, heap_region.addr() as u64);
    entry.write_u64(GFX_OFF + 0x10, 0);
    entry.write_u64(GFX_OFF + 0x18, 31);
    page_cache_new_frame();

    let build = observe(
        &[plan_for(entry.addr() as u64)],
        &Domain81SnapshotMemory,
        ObservationConfig::new(HOUR),
    )
    .expect("the fixture is far inside the custody ceiling");
    let view = build
        .snapshot
        .view_for(CONSUMER, HOUR)
        .expect("the plan captured its consumer");
    let view_memory = SnapshotViewMemory::new(view);
    let copied_window = ObjectSnapshot::<ENTRY_LEN>::read_in(&view_memory, entry.addr())
        .expect("the plan copied the entry body");

    // The engine overwrites the terminator after the bracket.
    heap_region.write_bytes(0, b"x");
    page_cache_new_frame();

    let live_window = ObjectSnapshot::<ENTRY_LEN>::read(entry.addr())
        .expect("the staged entry is still readable live");
    let from_copy =
        domain47_portrait_identity_with_snapshot_in(&view_memory, entry.addr(), &copied_window);
    let from_engine =
        domain47_portrait_identity_with_snapshot_in(&LIVE_MEMORY, entry.addr(), &live_window);

    assert_eq!(
        from_copy.as_ref().map(|id| id.as_ref().map(|(text, _)| text.as_str())),
        Ok(None),
        "the empty-identity terminator was fetched from the live process \
         instead of from the copy"
    );
    assert!(
        matches!(
            &from_engine,
            Err(refusal)
                if refusal.detail
                    == Domain47PortraitIdentityDetail::EmptyHeapTerminatorNonZero
        ),
        "the live decode must refuse the post-bracket terminator, or the \
         fixture never entered the condition this test is about: {from_engine:?}"
    );
}

/// THE CONTAINMENT VERDICT IS THE ONE WIRE WORD A COPY CANNOT CARRY.
///
/// A refused payload fetch publishes WHERE the buffer was:
/// `FollowedPayloadOutsideConfirmedRanges` when the range was never confirmed,
/// `FollowedPayloadInsideConfirmedRanges` when it was confirmed and the guarded
/// copy failed anyway. Those are two different faults and d47 spends a wire
/// word telling them apart.
///
/// A `SnapshotViewMemory` can only ever produce the first. Its
/// `bytes_with_reason` returns `ZeroLength` or `OutsideConfirmedRanges` and
/// nothing else, because the cause is already gone by the time a `ViewRefusal`
/// exists: a live-process `SnapshotMemory::copy_into` (`Domain81SnapshotMemory`
/// is the surviving one) reduces both live failures to an untyped
/// `MemoryRefusal` during the bracket.
///
/// So this states the divergence rather than papering over it: for a payload
/// the engine refused AFTER confirming its range, the post-bracket decode
/// publishes `Outside` where the live decode publishes `Inside`. It is one
/// word, on a refusal, and it is the only known behavioural difference between
/// the two memories. (Codex 019fe65a, finding 2. Closing it needs
/// `MemoryRefusal` to carry the cause, which is `snapshot_recipes` and is
/// reported as a residual.)
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: `MemoryRefusal` gaining a cause
/// and the view adapter learning to say `CopyFailed` — at which point the
/// divergence is closed and this test is what says so out loud, by failing
/// until it is deleted. Also: the two wire words being collapsed into one,
/// which would silently make the divergence invisible instead of fixed.
#[test]
fn the_containment_verdict_is_the_one_word_a_copy_cannot_carry() {
    use Domain47PortraitProbePointer as Pointer;
    use MsvcStringRefusalReason as Reason;
    use MsvcStringStorage as Storage;

    // 1. THE TWO CAUSES REALLY DO PRODUCE DIFFERENT WIRE WORDS. Without this
    //    the rest of the test would be about a distinction that no longer
    //    exists, and would pass for the wrong reason.
    let outside = MsvcStringRefusal::new(Reason::PayloadBytesUnreadable, Storage::Heap)
        .with_fetch(GuardedReadRefusal::OutsideConfirmedRanges);
    let copy_failed = MsvcStringRefusal::new(Reason::PayloadBytesUnreadable, Storage::Heap)
        .with_fetch(GuardedReadRefusal::CopyFailed);
    assert_eq!(
        domain47_portrait_reader_pointer(&outside),
        Pointer::FollowedPayloadOutsideConfirmedRanges
    );
    assert_eq!(
        domain47_portrait_reader_pointer(&copy_failed),
        Pointer::FollowedPayloadInsideConfirmedRanges
    );
    // NO `assert_ne!` OF THE TWO: the two `assert_eq!`s above already name
    // different variants, so it is implied (Codex 019fe8b3, finding 4). If
    // those two ever mapped to the same word, they could not both hold.

    // 2. AND A VIEW CANNOT PRODUCE THE SECOND.
    //
    //    NOT RE-ASSERTED HERE. `the_view_adapter_speaks_the_decoders_refusal
    //    _vocabulary` already sweeps the SAME six `ViewRefusal` variants and
    //    asserts each maps to `OutsideConfirmedRanges` — which is strictly
    //    stronger than "not `CopyFailed`", so a second weaker loop over the
    //    same inputs added nothing. (Codex 019fe8b3, finding 4.) What is left
    //    below is the part that is this test's own.
    use crate::snapshot_recipes::view::ViewRefusal;
    use crate::snapshot_recipes::Span;
    let span = Span::new(0x1000, 8);

    // 3. AND THE DIVERGENCE IS EXACTLY THIS ONE. A payload the view cannot
    //    serve refuses through the same reason and the same storage mode as the
    //    live path; only the fetch cause, and therefore only the containment
    //    word, differs. Staged directly on the refusal so the assertion is
    //    about the mapping and not about whatever a fixture happened to do.
    let over_a_copy = MsvcStringRefusal::new(Reason::PayloadBytesUnreadable, Storage::Heap)
        .with_fetch(guarded_refusal_for_view(ViewRefusal::Unreadable {
            span,
            void: span,
        }));
    // NO `over_a_copy.reason == copy_failed.reason` HERE. Both are constructed
    // from the same two literals in this test, so comparing them tests the
    // literals and not the production mapping (Codex 019fe8a1, finding 3).
    // What follows does test it: the same refusal, differing ONLY in the fetch
    // cause the view can produce, must come out of
    // `domain47_portrait_reader_pointer` as the OUTSIDE verdict.
    assert_eq!(
        domain47_portrait_reader_pointer(&over_a_copy),
        Pointer::FollowedPayloadOutsideConfirmedRanges,
        "an engine refusal during the bracket must publish the OUTSIDE verdict \
         post-bracket, which is the divergence this test names"
    );
}

/// `SnapshotViewMemory`'s refusal mapping is TOTAL and says one word.
///
/// The decoders' vocabulary has exactly one way to say "these bytes are not
/// available" (`OutsideConfirmedRanges`), so every view refusal must map onto
/// it — otherwise a decode over a copy could not compare reason-equal with the
/// same decode over the engine, and the equivalence proof would be impossible
/// rather than merely failing.
///
/// The `ZeroLength` direction is the one that must NOT be mapped: it is a rule
/// about the request, both sides check it before touching memory, and a view
/// that answered `OutsideConfirmedRanges` to a zero-length request would
/// disagree with the live reader on an input no memory is involved in.
///
/// WHAT PRODUCTION CHANGE WOULD MAKE THIS FAIL: a new `ViewRefusal` variant
/// reaching the decoders as something other than "not available" (the match is
/// wildcard-free, so it stops the build first), or the zero-length rule moving
/// out of `SnapshotViewMemory::bytes_with_reason`.
#[test]
fn the_view_adapter_speaks_the_decoders_refusal_vocabulary() {
    use crate::snapshot_recipes::view::ViewRefusal;
    use crate::snapshot_recipes::{GapReason, IntervalId, Span};
    let span = Span::new(0x1000, 8);
    for refusal in [
        ViewRefusal::ForeignHour {
            snapshot: crate::snapshot_recipes::HourStamp(1),
            asked: crate::snapshot_recipes::HourStamp(2),
        },
        ViewRefusal::ConsumerGapped(GapReason::WitnessChanged {
            interval: IntervalId(0),
        }),
        ViewRefusal::NotCaptured { span },
        ViewRefusal::NotContiguous { span },
        ViewRefusal::NotOwned { span },
        ViewRefusal::Unreadable { span, void: span },
    ] {
        assert_eq!(
            guarded_refusal_for_view(refusal),
            GuardedReadRefusal::OutsideConfirmedRanges,
            "{refusal:?} must reach a decoder as the one word the live reader \
             can also say"
        );
    }
}
