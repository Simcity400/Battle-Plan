//! Every `remove` / `remove_batch` call site in the workspace, and how each one
//! satisfies the quiescence contract written on [`inline_detour::remove`].
//!
//! # The defect class this closes
//!
//! `remove` is `MH_RemoveHook`, and `MH_RemoveHook` FREES THE TRAMPOLINE. The
//! freeze's instruction-pointer check asks whether any suspended thread is
//! inside the TARGET or the TRAMPOLINE -- and a thread executing the caller's
//! own DETOUR BODY is inside neither, because the detour is the caller's
//! function at an address this crate is never told. When that body finishes it
//! RETURNS THROUGH THE TRAMPOLINE, into memory the free has released.
//!
//! So `remove`'s contract is the caller's: admission closed, the in-flight
//! count observed at zero AFTER closing it, and neither undone since. `remove`
//! itself refuses a target MinHook still reports as enabled
//! (`InlineDetourReleaseError::StillEnabled`), which is necessary and NOT
//! sufficient -- it cannot see the caller's own body count at all.
//!
//! Lane KZ wrote that contract and audited the 80 call sites against it with a
//! source pass that read only the ENCLOSING FUNCTION, and said so: "it is a
//! source pass and cannot follow a call into a helper, so ABSENCE HERE IS A
//! QUESTION, NOT A VERDICT". Fifty-two sites came back as questions. This test
//! is the standing answer, and it is what stops the fifty-third from being
//! written.
//!
//! # What it requires, and in what order
//!
//! For each function containing a call, ONE of these must hold:
//!
//! 1. **Structural, in the function itself.** Its own text disables a target
//!    and compares an in-flight counter against zero.
//! 2. **Structural, through its own crate.** The same two halves appear in the
//!    same-crate call closure -- the helper it delegates to. This is the half
//!    KZ's pass could not see, and it accounts for most of the fifty-two:
//!    `character_presentation_live`'s teardown calls
//!    `wait_for_callback_quiescence`, which is a drain the enclosing text does
//!    not spell.
//! 3. **Declared.** A marker comment inside the function names which discipline
//!    covers it, out of the fixed vocabulary in [`MARKERS`]. None of them is a
//!    waiver; each is a claim someone can be shown to be wrong about:
//!    * `QUIESCED:` -- the drain is real and in this function, spelled in a way
//!      the two structural patterns do not match.
//!    * `NEVER-ENABLED:` -- the hook was created and never enabled, so nothing
//!      was patched and no thread can have entered a body that does not exist.
//!      READ THE NEXT PARAGRAPH BEFORE WRITING THIS ONE.
//!    * `DRAINED-BY-CALLER:` -- the function is a helper and its callers hold
//!      the disable and the drain. The marker must name them, and they must
//!      RETURN rather than continue when either fails.
//!    * `NEVER-CALLED:` -- implemented so that "this lane never frees its
//!      trampoline" is assertable, and never reached.
//!
//! # "The enable failed, so nothing was patched" IS FALSE IN THIS PRODUCT
//!
//! `minhook_h4` is a FORK, and `EnableHookLL` in it deliberately writes the
//! patch, THEN sets `isEnabled`/`queueEnable`, and only then reports
//! `MH_ERROR_MEMORY_PROTECT` if the protection restore or the instruction-cache
//! flush did not complete -- "the bytes ARE written by the time these run, so
//! the state is reported as it is". Six `NEVER-ENABLED:` markers in this
//! workspace once rested on the opposite reading. An enable-failure arm is a
//! LIVE-DETOUR arm until proven otherwise.
//!
//! # What it is not
//!
//! A LEXER, not a Rust parser, with the honest limits of one: it cannot follow
//! a trait object, a function pointer, or a call into another crate; it merges
//! same-named functions within a crate rather than resolving modules; it does
//! not know raw strings; and a marker is only as true as the person who wrote
//! it. It does read CODE ONLY -- comments and string literals are stripped
//! before anything is matched -- because every one of these files explains its
//! drain in prose, and a scan that reads prose lets a marker satisfy itself.
//!
//! What it does do is make the UNDECLARED case impossible: a new `remove` call
//! site with no drain and no marker fails this test, so "nobody has looked at
//! this one" stops being a reachable state.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The declarations a call site may carry, and what each one asserts.
const MARKERS: &[&str] = &[
    // Admission closed and the in-flight count drained to zero, in this
    // function, before the call. Use when the drain is real but spelled in a
    // way the two patterns below do not match.
    "QUIESCED:",
    // Created and never enabled: no detour body exists, so there is nothing to
    // drain. `remove` refuses at runtime if this is wrong.
    "NEVER-ENABLED:",
    // A helper. The marker must name the caller that holds the disable and the
    // drain.
    "DRAINED-BY-CALLER:",
    // Implemented so that "this lane never frees its trampoline" is assertable,
    // and never reached. `fow_visibility_live`'s `HookOps::remove_and_free` is
    // the one of these: its teardown tests read back the performed sequence and
    // FAIL if the call appears. A `DRAINED-BY-CALLER:` here would have named a
    // caller that does not exist, which reads as audited and is worse than no
    // marker at all.
    "NEVER-CALLED:",
];

fn native_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is .../extractor/native/inline_detour
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("inline_detour sits inside extractor/native")
        .to_path_buf()
}

/// Every `.rs` file the sweep reads. `inline_detour` itself is excluded: it is
/// where `remove` is DEFINED, and its own frozen implementation is governed by
/// the two source scans in `lib.rs` rather than by this contract.
fn source_files(root: &Path) -> Vec<PathBuf> {
    fn walk(directory: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if matches!(
                    name.as_ref(),
                    "target" | ".cargo-target" | "runs" | "minhook_h4" | "inline_detour" | ".git"
                ) {
                    continue;
                }
                walk(&path, out);
            } else if name.ends_with(".rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

/// `(name, body)` for every `fn` with a body. A trait method DECLARATION has
/// none: `fn f(..) -> T;` followed by the next function's brace made an earlier
/// version of this walker attribute that function's body to the declaration,
/// which invented four call sites in `fow_visibility_live` that do not exist.
fn functions(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(found) = text[search..].find("fn ") {
        let start = search + found;
        search = start + 3;
        // `fn` must be a whole token: `.rs` inside `bfn ` is not a function.
        if start > 0 {
            let previous = text[..start].chars().next_back().unwrap_or(' ');
            if previous.is_alphanumeric() || previous == '_' {
                continue;
            }
        }
        let rest = &text[start + 3..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        // WHERE THE SIGNATURE ENDS: at the first `{` or `;` that is not inside
        // a bracket. A body-less trait method declaration reaches the `;`
        // first; a real function reaches the `{`.
        //
        // "Does the signature contain a `;` anywhere" was the first rule here
        // and it was wrong in the way that matters:
        // `fn remove_created(targets: &[usize; 6], ..)` has one INSIDE AN ARRAY
        // TYPE, so the whole function was read as a declaration and dropped --
        // and that function frees a trampoline it has only just disabled. Two
        // more sites went the same way.
        let mut bracket_depth = 0usize;
        let mut signature_end = None;
        for (index, character) in rest.char_indices() {
            match character {
                '(' | '[' => bracket_depth += 1,
                ')' | ']' => bracket_depth = bracket_depth.saturating_sub(1),
                '{' if bracket_depth == 0 => {
                    signature_end = Some((index, true));
                    break;
                }
                ';' if bracket_depth == 0 => {
                    signature_end = Some((index, false));
                    break;
                }
                _ => {}
            }
        }
        let Some((relative_brace, has_body)) = signature_end else {
            continue;
        };
        if !has_body {
            continue;
        }
        let open = start + 3 + relative_brace;
        // BYTE OFFSETS ALL THE WAY DOWN. The first spelling of this bounded a
        // byte-offset walk with `Vec<char>::len()`, so in any file containing a
        // multibyte character -- an em dash, and these files are full of them
        // -- the scan stopped early and every function after that point was
        // silently dropped. It hid three real call sites, one of them a
        // teardown that freed a live trampoline. A sweep that quietly reads
        // less than it claims is worse than no sweep.
        let mut depth = 0usize;
        let mut end = None;
        for (index, character) in text[open..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + index);
                        break;
                    }
                }
                _ => {}
            }
        }
        if let Some(end) = end {
            out.push((name, text[open..=end].to_string()));
        }
    }
    out
}

fn crate_of(relative: &str) -> String {
    let parts: Vec<&str> = relative.split('/').collect();
    if parts.first() == Some(&"lanes") && parts.len() > 1 {
        format!("lanes/{}", parts[1])
    } else {
        parts.first().copied().unwrap_or("").to_string()
    }
}

/// Callee names, free (`helper(`) and method (`ops.disable(`) alike. Half these
/// lanes reach their drain through `self.quiesce()` or `ops.disable(target)`,
/// and a free-call-only walker reports the thin trait wrapper instead of the
/// discipline above it.
fn callees(body: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let chars: Vec<char> = body.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index].is_alphabetic() || chars[index] == '_' {
            let start = index;
            while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                index += 1;
            }
            if index < chars.len() && chars[index] == '(' {
                let name: String = chars[start..index].iter().collect();
                if name.chars().next().is_some_and(|c| c.is_lowercase() || c == '_') {
                    out.insert(name);
                }
            }
        } else {
            index += 1;
        }
    }
    out
}

/// A real call, not the NAME of one inside a string literal.
///
/// `land_combat_detail_events_live`'s `x2_carriers` has a test that BANS this
/// call and holds `"inline_detour::remove("` in a list of forbidden fragments to
/// do it. Reading that as a call site asks the crate that forbids the call to
/// justify making it, which is backwards. Only occurrences outside a double
/// quoted region on their own line count.
fn calls_remove(body: &str) -> bool {
    for line in body.lines() {
        let mut in_string = false;
        let mut escaped = false;
        let bytes = line.as_bytes();
        for index in 0..bytes.len() {
            let byte = bytes[index];
            if escaped {
                escaped = false;
                continue;
            }
            match byte {
                b'\\' if in_string => escaped = true,
                b'"' => in_string = !in_string,
                _ => {
                    // `line.is_char_boundary` first: these files carry em
                    // dashes, and slicing into one panics.
                    if !in_string
                        && line.is_char_boundary(index)
                        && (line[index..].starts_with("inline_detour::remove(")
                            || line[index..].starts_with("inline_detour::remove_batch("))
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn disables(text: &str) -> bool {
    text.contains("inline_detour::disable(")
        || text.contains("inline_detour::disable_batch(")
        || text.contains(".disable(")
}

/// An in-flight count compared against zero. Names rather than a shape, because
/// every lane spells its own counter and the comparison is what matters.
fn drains(text: &str) -> bool {
    const COUNTERS: &[&str] = &[
        "IN_FLIGHT",
        "in_flight",
        "ACTIVE_CALLBACKS",
        "active_callbacks",
        "CALLBACKS_IN_FLIGHT",
        "callbacks_in_flight",
    ];
    // CODE ONLY. Every one of these files EXPLAINS its drain in prose, so a
    // scan that reads comments finds the word "quiesce" in the paragraph above
    // a function that does not drain at all -- including in the markers this
    // same test accepts, which would make every declaration self-satisfying.
    for line in text.lines() {
        if COUNTERS.iter().any(|counter| line.contains(counter))
            && (line.contains("== 0") || line.contains("!= 0") || line.contains("> 0"))
        {
            return true;
        }
        // A named quiescence wait is the other spelling, and it is a CALL
        // rather than a comparison -- the comparison is inside it. Requiring
        // the parenthesis is what keeps it a call and not a mention.
        if ["quiesce(", "quiescence(", "wait_for_callbacks("]
            .iter()
            .any(|call| line.contains(call))
        {
            return true;
        }
    }
    false
}

/// CODE ONLY: comments (`//` and `/* */`) and string literals removed.
///
/// All three hide the same way. Every one of these files EXPLAINS its drain in
/// prose, so a scan that reads comments finds "quiesce" in the paragraph above
/// a function that does not drain -- including inside the markers this test
/// accepts, which would make every declaration self-satisfying. Strings hide it
/// too: `land_combat_detail_events`'s `x2_carriers` holds the literal
/// `"inline_detour::remove("` in a list of fragments it BANS.
///
/// This is a lexer, not a Rust parser, and the difference is stated rather than
/// left to be found: it does not know raw strings (`r#".."#`), lifetimes, or
/// char literals containing a quote. What it does do is remove the three places
/// prose can masquerade as code.
fn code_only(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    let mut in_string = false;
    let mut in_block_comment = 0usize;
    while let Some((index, character)) = chars.next() {
        if in_block_comment > 0 {
            if character == '*' && text[index..].starts_with("*/") {
                in_block_comment -= 1;
                chars.next();
            } else if character == '/' && text[index..].starts_with("/*") {
                in_block_comment += 1;
                chars.next();
            }
            continue;
        }
        if in_string {
            out.push(' ');
            if character == '\\' {
                chars.next();
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => {
                in_string = true;
                out.push(' ');
            }
            '/' if text[index..].starts_with("//") => {
                // To end of line.
                for (_, skipped) in chars.by_ref() {
                    if skipped == '\n' {
                        break;
                    }
                }
                out.push('\n');
            }
            '/' if text[index..].starts_with("/*") => {
                in_block_comment = 1;
                chars.next();
            }
            _ => out.push(character),
        }
    }
    out
}

#[test]
fn every_remove_call_site_drains_or_declares_why_it_need_not() {
    let root = native_root();
    let mut crate_bodies: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    let mut sites: Vec<(String, String, String)> = Vec::new();

    for path in source_files(&root) {
        let relative = path
            .strip_prefix(&root)
            .expect("walked from the root")
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let text = text.replace("\r\n", "\n");
        let name_of_crate = crate_of(&relative);
        let parsed = functions(&text);
        let bucket = crate_bodies.entry(name_of_crate.clone()).or_default();
        for (name, body) in &parsed {
            bucket.entry(name.clone()).or_default().push(body.clone());
        }
        if !text.contains("inline_detour::remove") {
            continue;
        }
        for (name, body) in parsed {
            // THE CALL IS LOOKED FOR IN CODE, not in the file's prose. A block
            // comment quoting the call -- or a doc example -- is not a call
            // site, and asking a crate to justify a mention is how the X2
            // carrier test, which BANS this call, was asked to declare that it
            // makes it. (`calls_remove` also skips ordinary string literals on
            // its own; this covers `/* */` and doc comments as well.)
            if calls_remove(&code_only(&body)) {
                sites.push((relative.clone(), name, body));
            }
        }
    }

    assert!(
        sites.len() >= 40,
        "the sweep found only {} call sites; it has stopped reading the workspace",
        sites.len()
    );

    // THE BLINDNESS GUARD, and it is not decoration. The first parser here
    // bounded a byte-offset walk with a character count, so every function
    // after the first multibyte character in a file was silently dropped --
    // and these three are exactly the functions that were lost, each late in a
    // long file full of em dashes. A sweep that reads less than it claims
    // passes for the wrong reason, which is the failure mode a green source
    // scan cannot otherwise show.
    for (file, function) in [
        (
            "lanes/dynamic_collection_trace_live/src/a729_named_overlap_hook.rs",
            "remove_created",
        ),
        (
            "lanes/direct_frd_transitions_live/src/a729_hook.rs",
            "uninstall_locked",
        ),
        (
            "lanes/land_combat_detail_events_live/src/a729_hook.rs",
            "release_hook_receipts",
        ),
    ] {
        assert!(
            sites
                .iter()
                .any(|(site_file, site_function, _)| site_file == file
                    && site_function == function),
            "the sweep no longer sees {file}::{function}; its parser has gone blind again"
        );
    }

    let mut undeclared = Vec::new();
    for (file, function, body) in &sites {
        // MARKERS ARE READ FROM THE RAW BODY -- they live in comments -- and
        // EVERYTHING ELSE FROM CODE ONLY.
        let code = code_only(body);
        if disables(&code) && drains(&code) {
            continue;
        }
        if MARKERS.iter().any(|marker| body.contains(marker)) {
            continue;
        }
        // The same-crate call closure: the helper the function delegates to.
        let bucket = crate_bodies
            .get(&crate_of(file))
            .expect("every file contributed its crate");
        let mut seen = BTreeSet::new();
        let mut frontier = vec![code.clone()];
        let mut closure = code.clone();
        for _ in 0..3 {
            let mut next = Vec::new();
            for chunk in &frontier {
                for callee in callees(chunk) {
                    if !seen.insert(callee.clone()) {
                        continue;
                    }
                    for callee_body in bucket.get(&callee).into_iter().flatten() {
                        let callee_code = code_only(callee_body);
                        closure.push('\n');
                        closure.push_str(&callee_code);
                        next.push(callee_code);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        if disables(&closure) && drains(&closure) {
            continue;
        }
        undeclared.push(format!("  {file}::{function}"));
    }

    assert!(
        undeclared.is_empty(),
        "\n{} `inline_detour::remove` call site(s) neither drain nor declare why they \
         need not.\n\n`remove` FREES THE TRAMPOLINE, and a thread inside your detour body \
         is invisible to the freeze -- it returns THROUGH that trampoline. Close admission, \
         disable, observe your in-flight count at zero, and only then free; or, if this \
         path genuinely cannot reach a live detour, say which and why with one of {:?} \
         (see `supply_logistics_attrition_live`'s \
         `take_down_freeing_only_when_quiet` for the worked example).\n\n{}\n",
        undeclared.len(),
        MARKERS,
        undeclared.join("\n")
    );
}
