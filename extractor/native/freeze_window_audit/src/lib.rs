//! What runs while the game is stopped, read out of a crate's own source.
//!
//! # The law this checks
//!
//! `inline_detour::freeze_process()` suspends every other thread of the game
//! process. Until the returned guard is dropped, this thread is the only one
//! running -- so a call into the allocator, or any lock, is a hang and not a
//! slow path: the lock may be held by a thread that can no longer run, and
//! there is nobody left to release it. `inline_detour` enforces that on itself
//! with two source scans and a call closure. The four other crates that take
//! the guard directly had no guard at all, and every one of them allocated
//! inside a held window.
//!
//! # What it is, exactly
//!
//! A SUBSTRING PASS over one crate's own text, closed over that crate's own
//! calls. That is the same instrument `inline_detour` uses on itself, and the
//! same honest limits apply -- they are stated here rather than left to be
//! discovered:
//!
//! * it cannot follow a call into another crate, so a helper that allocates in
//!   a dependency is invisible;
//! * it cannot follow an `impl Fn` parameter to the closure that will be passed;
//! * it cannot enumerate every spelling of "allocate", only the ones in
//!   [`ALLOCATING_OR_BLOCKING`];
//! * it cannot see a macro that assembles a call from fragments;
//! * and a construct behind `#[cfg(not(test))]` is visible to it but compiled
//!   out of every behavioural test.
//!
//! It is NOT a proof that no allocation happens. It is the difference between a
//! crate whose stopped windows nobody has read and one where every ordinary way
//! of writing this defect fails a test.
//!
//! # Where a window starts and ends
//!
//! It starts where the guard is bound and ends at the earliest of: an explicit
//! `drop(<guard>)`, the end of the enclosing function, or the guard being moved
//! somewhere the scan cannot follow. THE LAST CASE IS ITSELF A FINDING
//! ([`Finding::WindowEscapes`]): a window that leaves the function leaves the
//! part of it this instrument can read, and the game is stopped for all of it.

/// Constructs that allocate, free, or take a lock.
///
/// Fragments rather than exact call spellings, because a roster of exact calls
/// is a list of the ones already thought of: `.collect()` misses
/// `collect::<Vec<_>>()`, and `Vec::with_capacity` misses
/// `String::with_capacity`.
///
/// `push` is deliberately ABSENT and its absence is the rule, not an omission:
/// the correct shape for these windows is a buffer reserved before the first
/// suspension and pushed into afterwards, which is what `inline_detour` itself
/// does.
pub const ALLOCATING_OR_BLOCKING: &[&str] = &[
    "Vec::new",
    "Vec::from",
    "String::new",
    "String::from",
    "Box::new",
    "with_capacity",
    "reserve(",
    "collect",
    "format!",
    "to_string",
    "to_owned",
    "to_vec",
    "vec![",
    ".clone(",
    ".into()",
    ".lock(",
    "extend(",
    "extend_from_slice",
    "insert(",
    "sort_by",
    "sort_unstable",
    "println!",
    "eprintln!",
    "Default::default",
    "Instant::now",
    "sleep(",
    "join(",
    "recv(",
];

/// One thing the audit found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// A construct that can allocate or block runs while the process is
    /// stopped. `via` is the chain from the window to the function it is in --
    /// empty when it is in the window's own body.
    AllocatesInWindow {
        file: String,
        function: String,
        via: Vec<String>,
        construct: &'static str,
    },
    /// The freeze guard leaves the function that took it, so the window is not
    /// bounded by anything this instrument can read.
    WindowEscapes { file: String, function: String },
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AllocatesInWindow {
                file,
                function,
                via,
                construct,
            } => {
                write!(f, "{file}: `{function}` ")?;
                if !via.is_empty() {
                    write!(f, "(reached through {}) ", via.join(" -> "))?;
                }
                write!(
                    f,
                    "runs `{construct}` while every other thread in the process \
                     is suspended; the allocator's lock may be held by one of \
                     them and there is nobody left to release it"
                )
            }
            Self::WindowEscapes { file, function } => write!(
                f,
                "{file}: `{function}` lets the process freeze escape the \
                 function that took it, so the stopped window is not bounded by \
                 anything this scan can read",
            ),
        }
    }
}

/// One source file of the crate being audited.
pub struct Source {
    /// How the finding names it.
    pub label: String,
    /// Its production text -- the caller strips its own test modules.
    pub text: String,
}

/// Every `.rs` file under `src_dir`, with its test modules stripped.
///
/// # Why this exists rather than a hand-written list
///
/// The first version of the per-crate tests listed their files by name, and a
/// new module containing a new freeze would simply not be scanned -- the guard
/// would stay green while the window it could not see hung the game (review
/// round 1, MED). The directory is the list.
pub fn crate_sources(src_dir: &std::path::Path) -> Vec<Source> {
    let mut out = Vec::new();
    let mut stack = vec![src_dir.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let entries = std::fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("{}: {error}", directory.display()));
        for entry in entries {
            let entry = entry.expect("a readable directory entry");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let label = path
                .strip_prefix(src_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(Source {
                label,
                // EVERY test module is cut out, and the PRODUCTION TAIL
                // after it is KEPT. Truncating at the first
                // `#[cfg(test)] mod` threw away every item declared
                // after it, and an unscanned item is an unscanned
                // window (review round 2, MED).
                text: without_test_modules(&text.replace("\r\n", "\n")),
            });
        }
    }
    out.sort_by(|left, right| left.label.cmp(&right.label));
    out
}

/// Remove every `#[cfg(test)]` module, keeping everything around them.
fn without_test_modules(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..at]);
        let after = &rest[at..];
        let Some(brace) = after.find('{') else {
            out.push_str(after);
            return out;
        };
        let mut depth = 0i32;
        let mut end = after.len();
        for (offset, character) in after[brace..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = brace + offset + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// A stable fingerprint of a finding set, for a ratchet that a SWAP cannot pass.
///
/// A count alone is not a ratchet: fixing one finding and adding another leaves
/// it unchanged (review round 1, MED). This hashes the sorted findings, so any
/// change to WHICH findings exist fails the assertion even when there are the
/// same number of them. FNV-1a, because a dependency for this would be absurd
/// and nothing here is adversarial -- the failure prints the whole list.
pub fn fingerprint(findings: &[Finding]) -> u64 {
    let mut rendered: Vec<String> = findings.iter().map(|f| f.to_string()).collect();
    rendered.sort();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for line in rendered {
        for byte in line.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= u64::from(b'\n');
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Audit every held-freeze window in `sources`.
///
/// `acquisitions` are the call spellings that RETURN a freeze guard in this
/// crate -- `inline_detour::freeze_process()` and whatever local helper wraps
/// it. Every binding of one opens a window.
pub fn audit(sources: &[Source], acquisitions: &[&str]) -> Vec<Finding> {
    let mut findings = Vec::new();
    let bodies = function_bodies(sources);
    for source in sources {
        let text = normalise(&source.text);
        for (function, body) in functions(&text) {
            for window in windows_in(&body, acquisitions) {
                if window.escapes {
                    findings.push(Finding::WindowEscapes {
                        file: source.label.clone(),
                        function: function.clone(),
                    });
                }
                for construct in ALLOCATING_OR_BLOCKING {
                    if window.text.contains(construct) {
                        findings.push(Finding::AllocatesInWindow {
                            file: source.label.clone(),
                            function: function.clone(),
                            via: Vec::new(),
                            construct,
                        });
                    }
                }
                // AND EVERY FUNCTION THE WINDOW CALLS, TO A FIXED POINT. A
                // roster of helpers would be the same mistake one level up.
                let mut reached: Vec<String> = Vec::new();
                let mut frontier = vec![window.text.clone()];
                while let Some(current) = frontier.pop() {
                    for (name, callee) in &bodies {
                        if reached.iter().any(|seen| seen == name) {
                            continue;
                        }
                        if !current.contains(&format!("{name}("))
                            && !current.contains(&format!("{name}::<"))
                        {
                            continue;
                        }
                        reached.push(name.clone());
                        frontier.push(callee.clone());
                    }
                }
                for name in &reached {
                    let body = &bodies
                        .iter()
                        .find(|(candidate, _)| candidate == name)
                        .expect("a reached function has a body")
                        .1;
                    for construct in ALLOCATING_OR_BLOCKING {
                        if body.contains(construct) {
                            findings.push(Finding::AllocatesInWindow {
                                file: source.label.clone(),
                                function: function.clone(),
                                via: vec![name.clone()],
                                construct,
                            });
                        }
                    }
                }
            }
        }
    }
    findings
}

/// Strip comments and string literals, then collapse layout.
///
/// STRING LITERALS ARE EMPTIED, not kept: a literal can spell any code shape,
/// and a scan that reads `"format!"` inside an error message as a call would
/// report a finding that is not there -- and, worse, a guard written the other
/// way round could be satisfied by a literal.
fn normalise(source: &str) -> String {
    let source = source.replace("\r\n", "\n");
    let characters: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut index = 0usize;
    let mut block_depth = 0usize;
    let mut in_line_comment = false;
    let mut in_string = false;
    while index < characters.len() {
        let current = characters[index];
        let next = characters.get(index + 1).copied();
        if in_line_comment {
            if current == '\n' {
                in_line_comment = false;
                out.push('\n');
            }
            index += 1;
            continue;
        }
        if block_depth > 0 {
            if current == '/' && next == Some('*') {
                block_depth += 1;
                index += 2;
                continue;
            }
            if current == '*' && next == Some('/') {
                block_depth -= 1;
                index += 2;
                if block_depth == 0 {
                    out.push(' ');
                }
                continue;
            }
            index += 1;
            continue;
        }
        if in_string {
            if current == '\\' {
                index += 2;
                continue;
            }
            if current == '"' {
                in_string = false;
                out.push('"');
            }
            index += 1;
            continue;
        }
        if current == '/' && next == Some('/') {
            in_line_comment = true;
            index += 2;
            continue;
        }
        if current == '/' && next == Some('*') {
            block_depth = 1;
            index += 2;
            continue;
        }
        if current == '"' {
            in_string = true;
            out.push('"');
            index += 1;
            continue;
        }
        out.push(current);
        index += 1;
    }
    out
}

/// Every `fn` in the text, with the body between its braces.
fn functions(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut cursor = 0usize;
    while let Some(at) = text[cursor..].find("fn ") {
        let start = cursor + at;
        cursor = start + 3;
        // A name, then a body.
        let rest = &text[start + 3..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        let Some(open) = text[start..].find('{').map(|i| start + i) else {
            continue;
        };
        let mut depth = 0usize;
        let mut index = open;
        while index < bytes.len() {
            if bytes[index] == '{' {
                depth += 1;
            } else if bytes[index] == '}' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            index += 1;
        }
        if index >= bytes.len() {
            continue;
        }
        out.push((name, text[open..index].to_string()));
    }
    out
}

/// Every function of every source, for the call closure.
fn function_bodies(sources: &[Source]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for source in sources {
        for (name, body) in functions(&normalise(&source.text)) {
            match out.iter_mut().find(|(candidate, _)| *candidate == name) {
                // Two functions of the same name in different modules: scan
                // both, because a call site names only the name.
                Some((_, existing)) => {
                    existing.push('\n');
                    existing.push_str(&body);
                }
                None => out.push((name, body)),
            }
        }
    }
    out
}

/// The first `drop(<name>)` at the same brace depth the window opened at.
///
/// A drop nested inside a `match` arm or an `if` block belongs to that path
/// only; it does not end the window for the statements after the block.
fn depth_zero_drop(text: &str, name: &str) -> Option<usize> {
    if name.is_empty() {
        return None;
    }
    let needle = format!("drop({name})");
    let mut depth = 0i32;
    for (offset, character) in text.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && text[offset..].starts_with(&needle) {
            return Some(offset);
        }
    }
    None
}

/// Cut out everything between a NESTED `drop(<name>)` and the end of the block
/// it is in: those statements run after the guard is gone on that path.
fn without_text_after_a_nested_drop(text: &str, name: &str) -> String {
    if name.is_empty() {
        return text.to_string();
    }
    let needle = format!("drop({name})");
    let mut kept = String::with_capacity(text.len());
    let mut depth = 0i32;
    let mut skipping_until: Option<i32> = None;
    for (offset, character) in text.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if let Some(until) = skipping_until {
                    if depth < until {
                        skipping_until = None;
                    }
                }
            }
            _ => {}
        }
        if skipping_until.is_some() {
            continue;
        }
        if depth > 0 && text[offset..].starts_with(&needle) {
            skipping_until = Some(depth);
            continue;
        }
        kept.push(character);
    }
    kept
}

struct Window {
    text: String,
    escapes: bool,
}

/// The held-freeze windows inside one function body.
fn windows_in(body: &str, acquisitions: &[&str]) -> Vec<Window> {
    let mut out = Vec::new();
    for acquisition in acquisitions {
        let mut cursor = 0usize;
        while let Some(at) = body[cursor..].find(acquisition) {
            let start = cursor + at;
            cursor = start + acquisition.len();
            // The binding this acquisition produced, so the explicit drop can
            // be recognised. `let <name> = ...` and `let <name> = match ...`
            // are the two shapes these crates use; anything else is treated as
            // unnamed, which ends the window at the function instead -- the
            // conservative direction.
            let preceding = &body[..start];
            let name = preceding
                .rfind("let ")
                .map(|at| {
                    preceding[at + 4..]
                        .trim_start_matches("mut ")
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect::<String>()
                })
                .unwrap_or_default();
            // THE WINDOW OPENS WHEN THE BINDING STATEMENT COMPLETES, not at
            // the call. Until then there is no guard: the `Err` arm of a
            // `match` on the acquisition, and the `map_err` of a `?`, both run
            // with nothing suspended, and reporting them would be reporting a
            // window that does not exist.
            let tail = {
                let after = &body[start..];
                let mut depth = 0i32;
                let mut opened = after.len();
                for (offset, character) in after.char_indices() {
                    match character {
                        '{' | '(' | '[' => depth += 1,
                        '}' | ')' | ']' => depth -= 1,
                        ';' if depth <= 0 => {
                            opened = offset + 1;
                            break;
                        }
                        _ => {}
                    }
                }
                &after[opened.min(after.len())..]
            };
            // THE WINDOW ENDS AT THE DROP THAT ENDS IT ON THIS PATH.
            //
            // The first spelling of this took the FIRST `drop(<guard>)` in the
            // text, and that was wrong in the shape every one of these crates
            // is written in: an early-return arm drops the guard and returns,
            // and everything after that arm -- the rest of the function, still
            // frozen -- was then outside the scan. ARC1 hid a `vec![0u8; ..]`
            // there, in the stopped window, behind an earlier arm's drop.
            //
            // So the end is the first drop at the window's OWN brace depth, and
            // a drop inside a nested block ends the window only for the rest of
            // that block, which is cut out instead.
            let end = depth_zero_drop(tail, &name).unwrap_or(tail.len());
            // ESCAPES: the guard is moved into a value that outlives the
            // function -- returned, or stored in a struct field or a static.
            // ESCAPE SPELLINGS. Not a proof of absence -- a guard can leave a
            // function in ways no substring finds -- but every shape these
            // crates actually use, plus the ones review round 1 named:
            // returning it bare, returning it in an `Ok`, storing it in a
            // struct field, handing it to another function, or defusing its
            // drop outright.
            // PATH-SENSITIVE. The first version looked only when NO drop
            // existed later, so `if bad { return Ok(guard); } drop(guard);`
            // escaped on one path and was reported on none (review round 2,
            // MED). What matters is whether an escape appears BEFORE the
            // drop that closes the window.
            let searched = &tail[..end];
            let escapes = !name.is_empty()
                && [
                    format!("Some({name})"),
                    format!("return Ok({name})"),
                    format!("return {name}"),
                    format!("Ok({name})"),
                    format!("{name},"),
                    format!("{name})"),
                    format!("mem::forget({name})"),
                    format!("ManuallyDrop::new({name})"),
                ]
                .iter()
                .any(|spelling| searched.contains(spelling.as_str()));
            out.push(Window {
                text: without_text_after_a_nested_drop(&tail[..end], &name),
                escapes,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Vec<Finding> {
        audit(
            &[Source {
                label: "x.rs".into(),
                text: text.into(),
            }],
            &["freeze_process()"],
        )
    }

    /// The instrument finds the shape it exists for.
    ///
    /// What production change would make this fail? Any narrowing of the window
    /// that stops at the acquisition's own statement instead of running to the
    /// drop.
    #[test]
    fn an_allocation_between_the_freeze_and_its_drop_is_found() {
        let findings = one(
            "fn patch() { let held = freeze_process().unwrap(); \
             let ranges: Vec<_> = sites.iter().collect(); drop(held); }",
        );
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "collect"
            )),
            "{findings:?}"
        );
    }

    /// And does not find it AFTER the drop, which is where the work belongs.
    #[test]
    fn an_allocation_after_the_drop_is_not_a_finding() {
        let findings = one(
            "fn patch() { let held = freeze_process().unwrap(); drop(held); \
             let ranges: Vec<_> = sites.iter().collect(); }",
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    /// A helper called from inside the window is scanned too, at any depth.
    #[test]
    fn a_helper_the_window_calls_is_scanned() {
        let findings = one(
            "fn patch() { let held = freeze_process().unwrap(); inner(); drop(held); } \
             fn inner() { deeper(); } \
             fn deeper() { let message = format!(\"x\"); }",
        );
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { via, construct, .. }
                    if via == &vec!["deeper".to_string()] && *construct == "format!"
            )),
            "{findings:?}"
        );
    }

    /// A drop on an EARLY-RETURN PATH does not end the window for the rest of
    /// the function, and the statements after that drop are not in it either.
    ///
    /// This is the hole the first version had: ARC1 allocated a buffer in the
    /// stopped window, after an early-return arm had dropped the guard, and the
    /// scan stopped at that arm and saw nothing.
    #[test]
    fn a_drop_inside_an_early_return_arm_does_not_end_the_window() {
        let findings = one(
            "fn patch() { let held = freeze_process().unwrap(); \
             if bad { drop(held); return Err(format!(\"clean\")); } \
             let scratch = vec![0u8; 8]; drop(held); }",
        );
        // The `vec![` after the arm IS in the window...
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "vec!["
            )),
            "{findings:?}"
        );
        // ...and the `format!` after the arm's own drop is NOT.
        assert!(
            !findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "format!"
            )),
            "{findings:?}"
        );
    }

    /// A guard that leaves the function is reported rather than ignored.
    #[test]
    fn a_window_that_escapes_the_function_is_a_finding() {
        let findings = one(
            "fn patch() -> Set { let held = freeze_process().unwrap(); \
             Set { suspend: Some(held) } }",
        );
        assert!(
            findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    /// An escape on ONE path is reported even though another path drops.
    #[test]
    fn an_escape_before_the_drop_is_reported_even_when_a_later_drop_exists() {
        let findings = one(
            "fn patch() { let held = freeze_process().unwrap(); \
             if bad { return Ok(held); } drop(held); }",
        );
        assert!(
            findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    /// Production items declared AFTER a test module are still scanned.
    #[test]
    fn a_window_after_a_test_module_is_still_scanned() {
        let text = "#[cfg(test)] mod t { fn helper() { let x = format!(\"a\"); } } \
                    fn patch() { let held = freeze_process().unwrap(); \
                    let scratch = vec![0u8; 8]; drop(held); }";
        let findings = audit(
            &[Source {
                label: "x.rs".into(),
                text: without_test_modules(text),
            }],
            &["freeze_process()"],
        );
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "vec!["
            )),
            "{findings:?}"
        );
        // ...and the test module's own `format!` is not reported.
        assert!(
            !findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "format!"
            )),
            "{findings:?}"
        );
    }

    /// A string literal is not code. Without this the scan reports a finding for
    /// every error message that names one of its own fragments.
    #[test]
    fn a_construct_named_inside_a_string_literal_is_not_a_finding() {
        let findings = one(
            "fn patch() { let held = freeze_process().unwrap(); \
             refuse(\"do not call format! here\"); drop(held); }",
        );
        assert!(findings.is_empty(), "{findings:?}");
    }
}
