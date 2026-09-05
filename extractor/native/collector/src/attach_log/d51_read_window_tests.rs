//! Lane SE's proof that cutting work out of the d51 read window left every
//! recorded byte where it was.
//!
//! Five claims, one test group each:
//!
//! 1. the three windowed identity reads decide on the SAME FIELDS the paired
//!    scalar `_stable` reads decided on - no more (a torn field still
//!    refuses) and no less (the live gameplay bytes that happen to fall
//!    inside a window are not required to hold still);
//! 2. both scope readers keep the pre-window COLD path, ORDER included: the
//!    RTTI type name is confirmed (and the frame's vtable cached) BEFORE the
//!    object's header or container is read, which is the one thing about a
//!    windowed read that no fixture can observe;
//! 3. the encoder's skipped defensive sort is a no-op only where it is a
//!    no-op: canonical and shuffled inputs encode to the same bytes;
//! 4. `PayloadShape::RowsOnly` differs from `Encoded` in the payload field and
//!    in nothing else, and re-encoding a `RowsOnly` frame reproduces the
//!    `Encoded` payload byte for byte;
//! 5. the one-pass delta selection emits exactly what the two-pass shape it
//!    replaced emitted - the reference half is that shape, restated here
//!    statement for statement.
//!
//! Claims 3-5 are post-read computation over rows the reader had already
//! materialised, which is why this file can prove them by restating what the
//! old code did and comparing outputs. The phase-loop pre-count does stop
//! issuing guarded reads, but only reads that fed parse passes which could
//! not have been accepted - see `runs/se_flags_worker/MEASUREMENT.md` §3.
//!
//! The fourth windowed identity read - the vtable-pinned variables container
//! read - is proved in the lane crate beside the reader it changed
//! (`country_variables_extractor.rs`, `the_vtable_pinned_read_is_...`), and
//! lane SE3's re-derived pricing is in `runs/se3_d51_window_merges/`.

use super::*;

use domain51::scripted_state_domain51_collector::compact as d51c;

// ---------------------------------------------------------------------
// 1. the windowed identity reads
// ---------------------------------------------------------------------

fn flag_manager_window(vtable: u64, base: u64, slot_count: u32, entry_count: u32) -> Vec<u8> {
    let mut buf = vec![0u8; DOMAIN51_FLAG_MANAGER_WINDOW_LEN];
    buf[0..8].copy_from_slice(&vtable.to_le_bytes());
    let rows = domain51::scripted_state_domain51_collector::CFLAGMANAGER_ROWS;
    let packed = domain51::scripted_state_domain51_collector::CFLAGMANAGER_PACKED_SLOT_ENTRY_COUNT;
    buf[rows..rows + 8].copy_from_slice(&base.to_le_bytes());
    let word = u64::from(slot_count) | (u64::from(entry_count) << 32);
    buf[packed..packed + 8].copy_from_slice(&word.to_le_bytes());
    buf
}

#[test]
fn the_flag_manager_head_decodes_all_three_fields_from_one_window() {
    let window = flag_manager_window(0x7FF7_0000_1000, 0x2000_0000_4000, 96, 7);
    let head = domain51_flag_manager_head_from_samples(&window, &window)
        .expect("two identical samples authenticate");
    assert_eq!(head.vtable, Some(0x7FF7_0000_1000));
    assert_eq!(head.base, 0x2000_0000_4000);
    assert_eq!(head.slot_count, 96);
    assert_eq!(head.entry_count, 7);
}

#[test]
fn a_torn_flag_manager_row_base_or_packed_word_refuses_the_manager() {
    // kills: dropping either half of the pair `domain51_stable_flag_header`
    // required to hold still. Both are now read out of one window and both
    // must still refuse.
    let first = flag_manager_window(0x7FF7_0000_1000, 0x2000_0000_4000, 96, 7);
    for torn_offset in [
        domain51::scripted_state_domain51_collector::CFLAGMANAGER_ROWS,
        domain51::scripted_state_domain51_collector::CFLAGMANAGER_PACKED_SLOT_ENTRY_COUNT,
    ] {
        let mut second = first.clone();
        second[torn_offset] ^= 0x01;
        assert_eq!(
            domain51_flag_manager_head_from_samples(&first, &second),
            None,
            "a moving byte at +{torn_offset:#x} must refuse the manager"
        );
    }
}

#[test]
fn a_torn_flag_manager_vtable_is_reported_and_not_confused_with_a_torn_header() {
    // The vtable is the OTHER predicate. Before the window it was
    // `read_u64_stable(manager)` inside the cached-RTTI gate, and
    // `domain51_stable_flag_header` never saw it - a torn vtable therefore
    // reached that gate and was refused there, or, on the first manager of a
    // frame, was admitted on type-name evidence and simply not cached.
    // kills: folding the vtable into the header's stability requirement,
    // which would make this read refuse and skip the RTTI gate entirely.
    let first = flag_manager_window(0x7FF7_0000_1000, 0x2000_0000_4000, 96, 7);
    let mut second = first.clone();
    second[0] ^= 0x01;
    let head = domain51_flag_manager_head_from_samples(&first, &second)
        .expect("a torn vtable is not a torn header");
    assert_eq!(head.vtable, None);
    assert_eq!(head.base, 0x2000_0000_4000);

    // The warm arm of `domain51_flag_scope` is `head.vtable != Some(expected)`,
    // which is `read_u64_stable(manager).is_some_and(|v| v == expected)`:
    // a torn vtable is `None` and can equal no cache, and an untorn one is
    // admitted only against its own frame's cached value.
    assert_ne!(head.vtable, Some(0x7FF7_0000_1000usize));
    let untorn = domain51_flag_manager_head_from_samples(&first, &first)
        .expect("two identical samples authenticate");
    assert_ne!(untorn.vtable, Some(0xDEAD_BEEFusize));
    assert_eq!(untorn.vtable, Some(0x7FF7_0000_1000usize));
}

fn state_identity_window(vtable: u64, state_id: u32) -> Vec<u8> {
    let mut buf = vec![0u8; DOMAIN51_STATE_IDENTITY_WINDOW_LEN];
    let id_off = domain51::scripted_state_domain51_collector::STATE_ID;
    buf[0..8].copy_from_slice(&vtable.to_le_bytes());
    buf[id_off..id_off + 4].copy_from_slice(&state_id.to_le_bytes());
    buf
}

#[test]
fn the_state_identity_lets_the_gameplay_bytes_inside_its_window_move() {
    // THE property that makes one window read equivalent to the two scalar
    // `_stable` reads instead of stricter than them. `CState + 0x08 ..
    // + 0x58` is live state - it changes constantly - and the pre-window
    // spelling never looked at it. kills: comparing the raw windows
    // (`first.buf == second.buf`), which would refuse nearly every state on
    // nearly every frame and silently empty the domain.
    let first = state_identity_window(0x7FF7_0000_2000, 412);
    let mut second = first.clone();
    for offset in 8..domain51::scripted_state_domain51_collector::STATE_ID {
        second[offset] = 0xA5;
    }
    assert_eq!(
        domain51_state_identity_from_samples(&first, &second),
        Some((0x7FF7_0000_2000, 412))
    );
}

#[test]
fn a_torn_state_vtable_or_id_still_refuses_the_roster() {
    let first = state_identity_window(0x7FF7_0000_2000, 412);
    for torn_offset in [
        0usize,
        domain51::scripted_state_domain51_collector::STATE_ID,
    ] {
        let mut second = first.clone();
        second[torn_offset] ^= 0x01;
        assert_eq!(
            domain51_state_identity_from_samples(&first, &second),
            None,
            "a moving byte at +{torn_offset:#x} must refuse the state"
        );
    }
}

fn country_identity_window(vtable: u64, tag_db_id: u32) -> Vec<u8> {
    let mut buf = vec![0u8; DOMAIN51_COUNTRY_IDENTITY_WINDOW_LEN];
    buf[0..8].copy_from_slice(&vtable.to_le_bytes());
    buf[OBJ_TAG_DB_ID..OBJ_TAG_DB_ID + 4].copy_from_slice(&tag_db_id.to_le_bytes());
    buf
}

#[test]
fn the_country_identity_pairs_the_vtable_with_the_published_tag_and_refuses_a_tear() {
    let first = country_identity_window(0x7FF7_0000_3000, 77);
    assert_eq!(
        domain51_country_identity_from_samples(&first, &first),
        Some((0x7FF7_0000_3000, 77))
    );
    for torn_offset in [0usize, OBJ_TAG_DB_ID] {
        let mut second = first.clone();
        second[torn_offset] ^= 0x01;
        assert_eq!(
            domain51_country_identity_from_samples(&first, &second),
            None,
            "a moving byte at +{torn_offset:#x} must refuse the slot"
        );
    }
}

/// The collector source, newline-normalised so the ordering assertions below
/// do not depend on the checkout's line endings.
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

/// The name a column-zero line declares, if it declares a function at all.
///
/// Everything Rust allows between column zero and `fn` is a qualifier -
/// `pub`, `pub(crate)`, `pub(super)`, `unsafe`, `const`, `async`, `default`,
/// `extern "C"` - so the parse skips qualifier tokens and refuses anything
/// else. Codex round 6: the first cut matched only `fn` and `pub fn` and
/// silently missed twenty `pub(super)` / `pub(crate)` / `unsafe` functions.
fn top_level_fn_name(line: &str) -> Option<&str> {
    let mut rest = line;
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("fn ") {
            let name_end = match after.find(|c: char| !c.is_alphanumeric() && c != '_') {
                Some(end) => end,
                None => after.len(),
            };
            let name = &after[..name_end];
            return if name.is_empty() { None } else { Some(name) };
        }
        let token_end = rest.find(char::is_whitespace)?;
        let token = &rest[..token_end];
        let qualifier = matches!(token, "pub" | "unsafe" | "async" | "const" | "default")
            || token == "extern"
            || (token.starts_with("pub(") && token.ends_with(')'))
            || (token.starts_with('"') && token.ends_with('"'));
        if !qualifier {
            return None;
        }
        rest = &rest[token_end..];
    }
}

/// Every top-level function in the file, as (name, body-with-comments-removed).
///
/// Used where a rule has to hold for a SET of functions rather than a listed
/// few: a hand-written list of names went stale twice under review, and the
/// second time (Codex round 5) it was missing a function that had exactly the
/// shape the rule is about. Top level means the declaration starts in column
/// zero, so functions nested in modules, impls and test modules are outside
/// this. A name may appear twice (two `cfg` arms); both bodies are kept, and
/// every rule below is applied to each.
fn top_level_functions_stripped(source: &str) -> Vec<(&str, String)> {
    let mut found = Vec::new();
    let mut offset = 0usize;
    for line in source.split('\n') {
        let start = offset;
        offset += line.len() + 1;
        if line.is_empty() || line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some(name) = top_level_fn_name(line) else {
            continue;
        };
        let rest = &source[start..];
        let body = match rest.find("\n}\n") {
            Some(end) => &rest[..end],
            None => panic!("{name} does not close at column zero"),
        };
        let code = body
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        found.push((name, code));
    }
    assert!(
        found.len() > 100,
        "attach_log.rs has thousands of top-level functions; the scan found \
         only {} and is not working",
        found.len()
    );
    found
}

/// Every maximal `[A-Za-z0-9_]+` run in `code`.
///
/// Whole identifiers, not substrings: `read_u64` is a substring of
/// `read_u64_stable`, and treating one as a use of the other would pull half
/// the read stack into any closure computed below.
fn identifiers(code: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None;
    for (at, ch) in code.char_indices() {
        match (ch.is_alphanumeric() || ch == '_', start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                out.push(&code[from..at]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        out.push(&code[from..]);
    }
    out
}

/// Every top-level function reachable from `seeds` by name, transitively.
///
/// Codex round 6, the finding that retired selection-by-body-shape: a rule
/// applied to "functions that themselves take a bulk read" is not call-graph
/// closed, so an ordinary helper extraction escapes it - leave the bulk read
/// where it is, and move the CHOICE between it and the fallback into a new
/// function, which contains no bulk read and is therefore not selected. A
/// closure has no such hole: the new chooser is named by the function it was
/// extracted from, so it is reached the moment it exists.
///
/// Which makes the SEEDS the whole argument. They are derived rather than
/// named - see the call site, and Codex rounds 7 and 8, for the two shapes of
/// hole that forced it.
fn reachable_from<'a>(functions: &[(&'a str, String)], seeds: &[&'a str]) -> Vec<&'a str> {
    use std::collections::HashSet;

    let names: HashSet<&'a str> = functions.iter().map(|(name, _)| *name).collect();
    for seed in seeds {
        assert!(
            names.contains(seed),
            "{seed} is not a top-level function in attach_log.rs"
        );
    }
    let mut reached: Vec<&'a str> = seeds.to_vec();
    let mut seen: HashSet<&'a str> = seeds.iter().copied().collect();
    let mut cursor = 0;
    while cursor < reached.len() {
        let current = reached[cursor];
        cursor += 1;
        // A name may be declared twice under different `cfg`s; both bodies
        // count, so neither arm can hide an edge.
        for code in functions
            .iter()
            .filter(|(name, _)| *name == current)
            .map(|(_, code)| code)
        {
            for token in identifiers(code) {
                if let Some(known) = names.get(token) {
                    if seen.insert(*known) {
                        reached.push(*known);
                    }
                }
            }
        }
    }
    reached
}

#[test]
fn the_cold_rtti_gate_still_runs_before_the_read_it_used_to_precede() {
    // The one thing a windowed read can get wrong that no fixture can see.
    //
    // Before lane SE, both scope readers confirmed the RTTI type name (and
    // sampled the vtable they cache) BEFORE reading the object's header or
    // walking its container. The window makes it tempting to read the header
    // first, because the vtable rides in it - and on the warm arm that is
    // exactly the saving. On the COLD arm it is a behaviour change: an object
    // whose header moves DURING the two type-name walks was sampled AFTER the
    // walks and admitted, and would now be pinned to its pre-walk header and
    // refused by the post-walk re-witness. `Unmeasured` here fails the whole
    // country/state/global family closed.
    //
    // kills: hoisting the head/container read above the gate in either cold
    // arm. Nothing else pins it - the reads go through the live-memory
    // primitives, not through the `MemoryRead` the caller passes, so no
    // fixture can observe their order.
    let source = collector_source();
    for (signature, cold_read) in [
        (
            "fn domain51_flag_scope<R: domain51::MemoryRead>(",
            "domain51_flag_manager_head_after_rtti(manager)",
        ),
        (
            "fn domain51_variable_scope<R: domain51::MemoryRead>(",
            "domain51::read_variables_container(mem,",
        ),
    ] {
        let body = top_level_function(&source, signature);
        let cold_arm_start = body
            .rfind("None => {")
            .unwrap_or_else(|| panic!("{signature} has no cold arm"));
        let cold_arm = &body[cold_arm_start..];
        let gate = cold_arm
            .find("domain51_object_matches_cached_rtti(")
            .unwrap_or_else(|| panic!("{signature} cold arm does not run the RTTI gate"));
        let read = cold_arm
            .find(cold_read)
            .unwrap_or_else(|| panic!("{signature} cold arm does not contain {cold_read}"));
        assert!(
            gate < read,
            "{signature}: the cold arm must confirm the RTTI type name before it reads \
             {cold_read}"
        );
    }
}

#[test]
fn each_arms_scalar_fallback_is_the_sequence_that_arm_used_before_the_window() {
    // Codex round 1, and the round-4 defect wearing a different hat. The rule,
    // as MEASUREMENT.md §3 states it:
    //
    //   a fallback must preserve the SAMPLES, the PREDICATES and the TEAR
    //   INTERVALS of the sequence that arm used before the window existed.
    //
    // Not the global order - the owner's ruling is precisely a licence to move
    // the vtable and header groups relative to each other. What may not change
    // is how many samples a field gets, what is asserted about it, and how much
    // reading happens BETWEEN the two samples of one field. A read the arm
    // never made sits inside an interval that used to be shorter, and a field
    // moving during it is sampled through the transition and refused where base
    // accepted.
    //
    // The three flag-manager read sites had three different pre-window
    // sequences, so they get three different fallbacks:
    //
    //  * WARM went cached-RTTI gate (`read_u64_stable(manager)`) then
    //    `domain51_stable_flag_header`, so its fallback keeps the vtable pair;
    //  * COLD had already spent its vtable read inside the gate and went
    //    straight to `domain51_stable_flag_header`, so its fallback must not
    //    take another one. SE3's first cut routed both through the warm
    //    fallback and inserted an unused pair on the cold path;
    //  * the post-walk re-witness read the header alone and still does.
    //
    // Held from the source: these fallbacks reach memory through the live
    // primitives, so no fixture can count their reads.
    //
    // The table below is an ADJACENCY MATRIX over the four helpers, not a spot
    // check, because two rounds of review walked around spot checks by editing
    // a call edge rather than a body: round 2 pointed the warm ARM at the cold
    // helper, and round 3 pointed the warm helper's own FALLBACK at it. Either
    // hands a refused-window warm scope the cold helper's deliberate
    // `vtable: None`, which fails the warm check and loses the d51 family. So
    // every helper says which of the others it may name and which it may not.
    let source = collector_source();
    for (signature, must_read, must_not_reach) in [
        (
            // The WARM head: window, else the vtable-carrying scalar sequence.
            "fn domain51_stable_flag_manager_head(manager: usize)",
            "domain51_stable_flag_manager_head_scalar(manager)",
            &[
                "domain51_flag_manager_head_after_rtti",
                "domain51_flag_manager_head_still",
                "domain51_stable_flag_header",
                "read_u64_stable",
            ][..],
        ),
        (
            // The COLD head: window, else the header alone.
            "fn domain51_flag_manager_head_after_rtti(manager: usize)",
            "domain51_stable_flag_header(manager)",
            &[
                "read_u64_stable",
                "domain51_stable_flag_manager_head",
                "domain51_flag_manager_head_still",
            ][..],
        ),
        (
            // The post-walk re-witness: window, else the header alone.
            "fn domain51_flag_manager_head_still(",
            "domain51_stable_flag_header(manager)",
            &[
                "read_u64_stable",
                "domain51_stable_flag_manager_head",
                "domain51_flag_manager_head_after_rtti",
            ][..],
        ),
        (
            // The warm fallback itself, which is the only one that may spend a
            // vtable pair.
            "fn domain51_stable_flag_manager_head_scalar(manager: usize)",
            "read_u64_stable(manager)",
            &[
                "domain51_flag_manager_head_after_rtti",
                "domain51_flag_manager_head_still",
                "ObjectSnapshot",
            ][..],
        ),
    ] {
        let body = top_level_function(&source, signature);
        let code = body
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            code.contains(must_read),
            "{signature} must fall back through {must_read}"
        );
        for forbidden in must_not_reach {
            assert_eq!(
                code.matches(forbidden).count(),
                0,
                "{signature} must not reach {forbidden}: that is a read, or a \
                 route to one, that its arm did not take before the window \
                 existed"
            );
        }
    }

    // And the fallback must be reached ONLY when the window fails.
    //
    // Codex round 4. Adjacency says which route exists; it does not say the
    // route is lazy. `window_head.or(domain51_stable_flag_manager_head_scalar(
    // manager))` keeps exactly the one allowed edge and adds no forbidden one,
    // and `Option::or` evaluates its argument EAGERLY - so every SUCCESSFUL
    // window would also run the whole scalar sequence it was meant to replace.
    // Two reads become eight, §4's price is wrong, and the interval from the
    // root read to the re-witness grows by six reads, which is new `Unmeasured`
    // outcomes. `or_else` is the safe spelling, which is what makes this a
    // realistic maintenance slip rather than an attack.
    //
    // So: no eager combinator may appear in any function this lane gave a
    // window-then-fallback shape. `let .. else`, `match` and `or_else` are the
    // spellings that keep the fallback behind the failure.
    //
    // The set the ban covers is a CALL-GRAPH CLOSURE, and getting the closure
    // right took four tries because the first three each had a way out:
    //
    //  * a LIST of names went stale twice - round 4 wrote it, Codex round 5
    //    found it already missing `stable_header_pair_windowless`;
    //  * selection by SHAPE - "every function that itself takes a bulk read" -
    //    is not call-graph closed (round 6): leave the bulk read where it is,
    //    move the CHOICE between it and the fallback into a new function, and
    //    that function takes no bulk read, is not selected, and is where the
    //    eager combinator sits;
    //  * a closure seeded at the READ HELPERS is forward-only (rounds 7 and 8),
    //    so it never sees a chooser extracted ABOVE one of them -
    //
    //        choose_warm_head(domain51_stable_flag_manager_head(manager), manager)
    //
    //    with `windowed.or(..._scalar(manager))` inside. Every helper body and
    //    every adjacency edge stays intact and the warm arm still names its own
    //    helper exactly once, so the dispatch assertions pass; `choose_warm_head`
    //    is a SIBLING callee of the arm, never a callee of the helper. Round 7
    //    added the two scope readers as seeds; round 8 found the same hole one
    //    domain over, at the identity readers' own callers
    //    (`domain51_read_complete_country_roster_diagnostic` and
    //    `domain51_validate_state_roots`), which no flag-arm assertion covers.
    //
    // So the seeds are not named at all any more. They are derived, in two
    // steps that between them cover every place the defect can live:
    //
    //   1. every top-level function that takes one of this lane's windows -
    //      its body names an `ObjectSnapshot::<DOMAIN51_...>`;
    //   2. every top-level function that NAMES one of those.
    //
    // and the ban is the forward closure of that. The argument that this is
    // complete: the defect needs the window's `Option` and the fallback call in
    // ONE expression, so it lives in a function that either takes the window
    // itself (step 1), or holds an `Option` it did not take - which it can only
    // have got by calling a step-1 function (step 2) or by being handed it as
    // an argument by something that did (a callee of a step-2 function, so in
    // the forward closure).
    //
    // The two scope readers are added as well. They hold the arms, they are
    // step-2 functions for the flag merges anyway, and naming them keeps
    // `domain51_variable_scope` in even though its window lives in the lane
    // crate. Fifty-four functions today, all clean.
    let functions = top_level_functions_stripped(&source);
    let windowed: Vec<&str> = functions
        .iter()
        .filter(|(_, code)| code.contains("ObjectSnapshot::<DOMAIN51_"))
        .map(|(name, _)| *name)
        .collect();
    assert!(
        windowed.len() >= 5,
        "this lane gave five collector-side reads a window; the scan found \
         {windowed:?}"
    );
    let mut seeds = windowed.clone();
    for (name, code) in &functions {
        if seeds.contains(name) {
            continue;
        }
        // Cheap prefilter, then whole-identifier confirmation: `contains`
        // alone would count a longer name that merely ends in a shorter one.
        if !windowed.iter().any(|window| code.contains(*window)) {
            continue;
        }
        let tokens = identifiers(code);
        if windowed.iter().any(|window| tokens.contains(window)) {
            seeds.push(*name);
        }
    }
    for reader in ["domain51_flag_scope", "domain51_variable_scope"] {
        if !seeds.contains(&reader) {
            seeds.push(reader);
        }
    }
    let checked = reachable_from(&functions, &seeds);
    // The derivation has to reach the callers round 8 named, and the closure
    // has to walk past its own seeds. `domain51_stable_flag_header` is one hop
    // out and `copy_selected` is the bottom of the read stack.
    for expected in [
        "domain51_read_complete_country_roster_diagnostic",
        "domain51_validate_state_roots",
    ] {
        assert!(
            seeds.contains(&expected),
            "the seed derivation must reach the caller {expected}; it found \
             {seeds:?}"
        );
    }
    for expected in ["domain51_stable_flag_header", "copy_selected"] {
        assert!(
            checked.contains(&expected),
            "the closure must reach {expected}; it reached {checked:?}"
        );
    }
    for name in &checked {
        for (_, code) in functions.iter().filter(|(each, _)| each == name) {
            for eager in [".or(", ".unwrap_or(", ".map_or(", ".and("] {
                assert_eq!(
                    code.matches(eager).count(),
                    0,
                    "{name} is on the d51 windowed read path and uses the \
                     EAGER combinator `{eager}`, which runs the fallback even \
                     when the window succeeded; use `or_else` / `let .. else` \
                     / `match` so the fallback stays behind the failure"
                );
            }
        }
    }
    // The warm scalar fallback's own ORDER: the gate's vtable pair came before
    // the header, and it still must. Holding only "it reads both" would let
    // them swap, which puts two reads inside the header pair's interval.
    let warm_scalar = top_level_function(
        &source,
        "fn domain51_stable_flag_manager_head_scalar(manager: usize)",
    );
    assert!(
        warm_scalar.find("read_u64_stable(manager)")
            < warm_scalar.find("domain51_stable_flag_header(manager)"),
        "the warm fallback samples the vtable before the header, as the gate did"
    );

    // And the DISPATCH EDGES, which the helper bodies above cannot speak for
    // (Codex round 2). Two one-line edits defeat everything above on their own:
    // pointing the WARM arm at the cold helper hands a refused-window warm
    // scope `vtable: None` and rejects it, and hoisting the `head.vtable` check
    // out of the warm arm applies it to the cold arm, whose `vtable: None` is
    // deliberate. Each arm must name its own helper and nothing else, and the
    // vtable check must live inside the warm arm.
    //
    // Comments stripped first: the warm arm's own comment quotes
    // `head.vtable == Some(expected)` while explaining it, and prose is not
    // code.
    let scope = top_level_function(&source, "fn domain51_flag_scope<R: domain51::MemoryRead>(")
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let split = scope.rfind("None => {").expect("the cold arm");
    let (warm_arm, cold_arm) = scope.split_at(split);

    assert_eq!(
        warm_arm
            .matches("domain51_stable_flag_manager_head(manager)")
            .count(),
        1,
        "the warm arm reads its head through the vtable-carrying helper"
    );
    assert_eq!(
        warm_arm
            .matches("domain51_flag_manager_head_after_rtti(manager)")
            .count(),
        0,
        "and never through the cold one, which reports no vtable"
    );
    assert_eq!(
        cold_arm
            .matches("domain51_flag_manager_head_after_rtti(manager)")
            .count(),
        1,
        "the cold arm reads its head through the header-only helper"
    );
    assert_eq!(
        cold_arm
            .matches("domain51_stable_flag_manager_head(manager)")
            .count(),
        0,
        "and never through the warm one - which is exactly what the first cut \
         got wrong"
    );
    assert_eq!(
        warm_arm.matches("head.vtable != Some(expected)").count(),
        1,
        "the vtable check is the warm arm's, spelled as the refusal the old \
         cached-RTTI gate spelled"
    );
    assert_eq!(
        cold_arm.matches("head.vtable").count(),
        0,
        "the cold arm must not look at a field its helper does not sample: \
         `vtable: None` there is deliberate, not a refusal"
    );
    assert_eq!(
        scope.matches("head.vtable").count(),
        1,
        "and there is exactly one vtable check in the whole reader, so it \
         cannot be hoisted below the match and applied to both arms"
    );
}

// ---------------------------------------------------------------------
// 2-4. the frame-shaping and delta cuts
// ---------------------------------------------------------------------

fn variable_row(
    scope_kind: u32,
    owner_id: u32,
    key: &str,
    value_raw: i64,
) -> d51c::ScriptedStateRow {
    d51c::ScriptedStateRow {
        scope_kind,
        owner_id,
        value_kind: d51c::VALUE_VARIABLE_FIXED_I64,
        key: Some(key.to_string()),
        token_id: None,
        value_raw: Some(value_raw),
        date_hour_raw: None,
        deleted: false,
        delayed_due_hours: None,
    }
}

fn token_flag_row(
    scope_kind: u32,
    owner_id: u32,
    token_id: u32,
    hour: i32,
) -> d51c::ScriptedStateRow {
    d51c::ScriptedStateRow {
        scope_kind,
        owner_id,
        value_kind: d51c::VALUE_FLAG_TOKEN_ONLY,
        key: None,
        token_id: Some(token_id),
        value_raw: Some(1),
        date_hour_raw: Some(hour),
        deleted: false,
        delayed_due_hours: None,
    }
}

fn mixed_rows() -> Vec<d51c::ScriptedStateRow> {
    vec![
        variable_row(d51c::SCOPE_GLOBAL, 0, "global_war_support", 550_000),
        variable_row(d51c::SCOPE_COUNTRY, 12, "alpha_counter", 1),
        variable_row(d51c::SCOPE_COUNTRY, 12, "beta_counter", -7),
        variable_row(d51c::SCOPE_COUNTRY, 40, "alpha_counter", 900_000),
        variable_row(d51c::SCOPE_STATE, 412, "state_var", 3),
        token_flag_row(d51c::SCOPE_COUNTRY, 12, 9001, 4_812_000),
        token_flag_row(d51c::SCOPE_STATE, 412, 9002, 4_812_001),
    ]
}

fn test_header(rows: &[d51c::ScriptedStateRow]) -> d51c::ScriptedStateHeader {
    d51c::ScriptedStateHeader {
        elapsed_wall_ms: 4,
        status_flags: 0,
        changed_rows: rows.len() as u32,
        observed_scopes: 8,
        measured_scopes: 8,
        rejected_scopes: 0,
    }
}

#[test]
fn the_encoder_produces_the_same_bytes_from_canonical_and_shuffled_rows() {
    // kills: making the skipped defensive sort unconditional. `encode_payload`
    // now sorts only when the input is not already non-decreasing under
    // `canonical_row_cmp`; if that check were replaced by `true` the shuffled
    // input below would serialise in caller order and these bytes would part.
    let mut canonical = mixed_rows();
    canonical.sort_by(d51c::canonical_row_cmp);
    let mut shuffled = canonical.clone();
    shuffled.reverse();
    shuffled.swap(0, 3);

    let header = test_header(&canonical);
    assert_eq!(
        d51c::encode_payload(&header, &canonical),
        d51c::encode_payload(&header, &shuffled)
    );
}

fn scope_pair(
    scope_kind: u32,
    owner_id: u32,
    variables: &[(&str, i64)],
    flags: &[(u32, i32)],
) -> (domain51::VariableScopeRead, domain51::FlagScopeRead) {
    let variable_status = if variables.is_empty() {
        domain51::ScopeReadStatus::MeasuredEmpty
    } else {
        domain51::ScopeReadStatus::Measured
    };
    let flag_status = if flags.is_empty() {
        domain51::ScopeReadStatus::MeasuredEmpty
    } else {
        domain51::ScopeReadStatus::Measured
    };
    (
        domain51::VariableScopeRead {
            scope_kind,
            owner_id,
            status: variable_status,
            entries: variables
                .iter()
                .map(|(key, value_raw)| domain51::VariableEntry {
                    key: (*key).to_string(),
                    value_raw: *value_raw,
                })
                .collect(),
        },
        domain51::FlagScopeRead {
            scope_kind,
            owner_id,
            status: flag_status,
            entries: flags
                .iter()
                .map(|(token_id, hour)| domain51::FlagEntry {
                    key: None,
                    token_id: Some(*token_id),
                    date_hour_raw: Some(*hour),
                    value: Some(1),
                })
                .collect(),
        },
    )
}

fn a13_input() -> domain51::A13FrameInput {
    let pairs = vec![
        scope_pair(
            d51c::SCOPE_GLOBAL,
            0,
            &[("global_war_support", 550_000)],
            &[],
        ),
        scope_pair(
            d51c::SCOPE_COUNTRY,
            12,
            &[("alpha_counter", 1), ("beta_counter", -7)],
            &[(9001, 4_812_000)],
        ),
        scope_pair(d51c::SCOPE_COUNTRY, 40, &[("alpha_counter", 900_000)], &[]),
        scope_pair(
            d51c::SCOPE_STATE,
            412,
            &[("state_var", 3)],
            &[(9002, 4_812_001)],
        ),
        scope_pair(d51c::SCOPE_STATE, 413, &[], &[]),
    ];
    domain51::A13FrameInput {
        elapsed_wall_ms: 4,
        executable_sha256:
            domain51::scripted_state_domain51_collector::EXACT_A729_EXECUTABLE_SHA256,
        game_hour_raw: 4_812_000,
        observed_game_hour_before: 4_812_000,
        observed_game_hour_after: 4_812_000,
        source_sequence: 9,
        variable_scopes: pairs.iter().map(|(v, _)| v.clone()).collect(),
        flag_scopes: pairs.iter().map(|(_, f)| f.clone()).collect(),
    }
}

#[test]
fn the_rows_only_shape_changes_the_payload_field_and_nothing_else() {
    let encoded =
        domain51::collect_a13_domain51_frame_shaped(a13_input(), domain51::PayloadShape::Encoded)
            .expect("the fixture roster is complete");
    let rows_only =
        domain51::collect_a13_domain51_frame_shaped(a13_input(), domain51::PayloadShape::RowsOnly)
            .expect("the fixture roster is complete");

    assert!(rows_only.payload.is_empty());
    assert!(!encoded.payload.is_empty());
    assert_eq!(rows_only.rows, encoded.rows);
    assert_eq!(rows_only.row_count, encoded.row_count);
    assert_eq!(rows_only.status_flags, encoded.status_flags);
    assert_eq!(rows_only.observed_scopes, encoded.observed_scopes);
    assert_eq!(rows_only.measured_scopes, encoded.measured_scopes);
    assert_eq!(rows_only.rejected_scopes, encoded.rejected_scopes);
    assert_eq!(rows_only.elapsed_wall_ms, encoded.elapsed_wall_ms);
    assert_eq!(rows_only.domain_id, encoded.domain_id);
    assert_eq!(
        rows_only.validated_scope_partial_failure,
        encoded.validated_scope_partial_failure
    );

    // The deferred path rebuilds through exactly this helper, and it has to
    // land on the bytes the eager encode would have produced.
    assert_eq!(domain51_payload_from_frame(&rows_only), encoded.payload);
}

/// The pre-lane-SE delta selection, restated statement for statement: build
/// the whole baseline map from cloned rows first, then walk the rows a second
/// time to pick the changed ones, then append the deleted keys.
fn reference_delta_selection(
    rows: &[d51c::ScriptedStateRow],
    last_emitted: &BTreeMap<Domain51RowKey, d51c::ScriptedStateRow>,
) -> (
    Vec<d51c::ScriptedStateRow>,
    BTreeMap<Domain51RowKey, d51c::ScriptedStateRow>,
) {
    let mut current = BTreeMap::new();
    for row in rows {
        if let Some(key) = domain51_row_key(row) {
            current.insert(key, row.clone());
        }
    }
    let mut emitted = Vec::new();
    for row in rows {
        if let Some(key) = domain51_row_key(row) {
            let changed = last_emitted.get(&key).map(|old| old != row).unwrap_or(true);
            if changed {
                emitted.push(row.clone());
            }
        } else if domain51_full_emit_in_delta(row) {
            emitted.push(row.clone());
        }
    }
    for key in last_emitted.keys() {
        if !current.contains_key(key) {
            emitted.push(domain51_deleted_row(key));
        }
    }
    emitted.sort_by(d51c::canonical_row_cmp);
    (emitted, current)
}

/// A collected frame in the shape the reader hands to the emit-mode step.
///
/// `payload_shape` MUST match the `payload_deferred` flag the test then
/// passes: `Encoded` is what the reader produces when it did not defer, and
/// its payload has to survive the priming path untouched, which is only
/// checkable if it is a real payload and not an empty `Vec`.
fn frame_from_rows(
    rows: Vec<d51c::ScriptedStateRow>,
    payload_shape: domain51::PayloadShape,
) -> domain51::Domain51CollectedFrame {
    let mut rows = rows;
    rows.sort_by(d51c::canonical_row_cmp);
    let row_count = rows.len() as u32;
    let mut frame = domain51::Domain51CollectedFrame {
        domain_id: d51c::DOMAIN_ID,
        elapsed_wall_ms: 4,
        payload: Vec::new(),
        rows,
        row_count,
        status_flags: 0,
        observed_scopes: 8,
        measured_scopes: 8,
        rejected_scopes: 0,
        validated_scope_partial_failure: false,
    };
    if payload_shape == domain51::PayloadShape::Encoded {
        frame.payload = domain51_payload_from_frame(&frame);
        assert!(!frame.payload.is_empty());
    }
    frame
}

#[test]
fn the_one_pass_delta_emits_what_the_two_pass_shape_emitted() {
    // Second frame against a seeded baseline, exercising all four outcomes at
    // once: an unchanged row (dropped), a changed row (emitted), a brand new
    // row (emitted), and a vanished row (emitted as a deletion).
    let mut baseline_rows = mixed_rows();
    baseline_rows.sort_by(d51c::canonical_row_cmp);
    let (_, baseline) = reference_delta_selection(&baseline_rows, &BTreeMap::new());

    let mut next_rows = mixed_rows();
    next_rows.retain(|row| row.key.as_deref() != Some("beta_counter"));
    for row in next_rows.iter_mut() {
        if row.key.as_deref() == Some("state_var") {
            row.value_raw = Some(99);
        }
    }
    next_rows.push(variable_row(d51c::SCOPE_COUNTRY, 40, "gamma_counter", 5));
    // The production reader hands the emit-mode step canonically sorted rows;
    // the reference half must see the same sequence.
    next_rows.sort_by(d51c::canonical_row_cmp);

    let (expected_emitted, expected_baseline) = reference_delta_selection(&next_rows, &baseline);

    let mut tracker = CompactScriptedStateTracker::default();
    tracker.last_emitted.advance(baseline);
    let read = compact_scripted_state_apply_emit_mode(
        CompactScriptedStateRead {
            frame: frame_from_rows(next_rows.clone(), domain51::PayloadShape::RowsOnly),
            read_time_us: 0,
            status_flags: 0,
            country_roster_failure: None,
            // Lane SE2 widened this struct. These fixtures drive the emit-mode
            // step over rows that were already materialised; no scope was read,
            // so the RTTI gate was never entered and the honest census is the
            // one every un-exercised read carries.
            rtti_census: Domain51RttiColdArmCensus::default(),
        },
        D51EmitMode::Delta,
        &mut tracker,
        true,
    );

    assert_eq!(read.frame.rows, expected_emitted);
    assert_eq!(read.frame.row_count, expected_emitted.len() as u32);
    assert_eq!(*tracker.last_emitted, expected_baseline);
    assert!(read.status_flags & COMPACT_STATUS_DELTA_DATA != 0);
    assert!(read.frame.status_flags & COMPACT_STATUS_DELTA_DATA != 0);
    assert_eq!(read.frame.payload, domain51_payload_from_frame(&read.frame));
    assert!(
        expected_emitted
            .iter()
            .any(|row| row.value_kind & d51c::ROW_VALUE_KIND_DELETED != 0),
        "the fixture must produce a deletion or it proves nothing about that arm"
    );
}

#[test]
fn the_first_delta_frame_still_publishes_the_whole_table_and_seeds_the_baseline() {
    let rows = mixed_rows();
    let (_, expected_baseline) = reference_delta_selection(&rows, &BTreeMap::new());
    let frame = frame_from_rows(rows.clone(), domain51::PayloadShape::Encoded);
    let whole_table = frame.rows.clone();
    let whole_table_payload = frame.payload.clone();

    let mut tracker = CompactScriptedStateTracker::default();
    let read = compact_scripted_state_apply_emit_mode(
        CompactScriptedStateRead {
            frame,
            read_time_us: 0,
            status_flags: 0,
            country_roster_failure: None,
            // As above: this fixture reads no scope.
            rtti_census: Domain51RttiColdArmCensus::default(),
        },
        D51EmitMode::Delta,
        &mut tracker,
        false,
    );

    assert_eq!(read.frame.rows, whole_table);
    assert_eq!(read.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_eq!(*tracker.last_emitted, expected_baseline);
    // kills: deferring the payload on the priming frame. This is the exit that
    // publishes the WHOLE TABLE and does not re-encode, so it is the one path
    // where `PayloadShape::RowsOnly` would reach a publish with nothing in it.
    // Asserting the exact bytes, not merely non-emptiness, also catches the
    // priming frame publishing something other than the eager encode.
    assert!(!read.frame.payload.is_empty());
    assert_eq!(read.frame.payload, whole_table_payload);
    assert_eq!(
        read.frame.payload,
        d51c::encode_payload(&test_header(&whole_table), &whole_table)
    );
}
