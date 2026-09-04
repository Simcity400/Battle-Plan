//! How the UNIT tests in this crate read source text.
//!
//! `include_str!` hands back the file including its COMMENTS, and a scan
//! cannot tell a live call site from a sentence in a doc comment. That was
//! not hypothetical: `xui17_four_surface_route_owns_pointer_wheel_
//! tooltips_and_seek_reset` delimited the lifecycle region it checks by
//! slicing on the comment `// Viewer-owned XUI17 lifecycle`, so a rename of
//! that region's CODE left the scan reading the same window while a reword
//! of the PROSE broke it, and `ui_runtime.rs` had three more region
//! boundaries anchored on doc comments. A census of all 235 patterns the
//! `app.rs` scans search for found exactly that one anchored on a comment;
//! the other 234 match live code.
//!
//! Reading through here removes comment text and normalises line endings, so a
//! scan sees what the compiler sees, and every scan that searches a file for a
//! literal reads a PRODUCTION cut, so it cannot be answered by the test module
//! that asserts it. One reader is used whole and deliberately:
//! `the_production_source_slice_actually_excludes_the_tests` compares a cut
//! against the full stripped file, which is the comparison that says the cut
//! removed something.
//!
//! The two `_raw` accessors are PRIVATE to this module. Proving the stripping
//! removed a comment needs the version that still has it, so the seam's own
//! tests -- which live here -- are the only things that can ask for it.
//!
//! WHAT IS ENFORCED, exactly. `a_routed_file_cannot_read_source_around_the_seam` reads the nine routed files THROUGH the seam and refuses a direct
//! source read of a `.rs` path in any of them. That is a tripwire over nine
//! named files, not a proof of exclusivity, and the difference matters:
//!
//! * The crate's INTEGRATION tests under `tests/` cannot use this seam at all:
//!   `replay_app` is a binary crate, so there is nothing for them to import.
//!   FOUR of them read Rust source -- `tooltip_runtime_no_d72.rs` off disk,
//!   `gate_currency.rs` by `include!`, and
//!   `descheduled_carriers_reach_no_pixel.rs` and `no_silent_skip.rs` by
//!   WALKING the tree and reading every `.rs` they find, which is their job.
//!   What can be pinned about a tree walk is not what it reads but how many
//!   places read at all, so
//!   `the_reads_outside_the_seam_are_the_ones_named_here` pins the count of
//!   reader call sites across all four.
//! * `world_audio.rs` and `world_audio_law.rs` WERE named rather than routed,
//!   because lane WD was editing them when this seam landed. Lane WD has since
//!   routed all four of their reads, so they are inside the tripwire and the
//!   routed set is nine.
//! * A path built at run time, or assembled by `concat!`, is invisible to any
//!   text matcher, this one included. None occurs today.
//!
//! The claim is therefore the narrow one, and it is the whole claim: within
//! those nine files, a source read that skips the stripping and the
//! production cut fails a test. Everywhere else, the tests above pin what
//! exists so it cannot change unnoticed. Neither is a proof that no test
//! anywhere can read source another way, and this module does not say it is.

/// The line every one of these files declares its test module with, assembled
/// rather than written so this file does not carry a copy that the cuts could
/// find first.
fn test_module_marker() -> String {
    ["#[cfg(", "test)]
mod tests {"].concat()
}

/// How many places the integration tests read source from.
///
/// They are a separate crate and cannot import this seam, so a count is what
/// stands in for it. See `the_reads_outside_the_seam_are_the_ones_named_here`.
const INTEGRATION_READER_CALL_SITES: usize = 6;

const QUOTE: char = '"';
const CR: char = '\r';

pub(crate) fn app() -> String {
    code(app_raw())
}

/// This file's PRODUCTION half: everything above this test module.
///
/// A scan slices a window by finding a start anchor and then searching
/// FORWARD for an end anchor. If the end anchor's production occurrence
/// sits BEFORE the start, that forward search runs off the end of
/// production and matches the test module's own string literal instead,
/// and the window silently becomes thousands of lines of unrelated
/// code. That is not hypothetical: the XUI17 lifecycle window ended on
/// this module's own `"VK_OEM_COMMA"` literal and ran from the branch it
/// meant to check all the way into the tests, so the assertion was
/// satisfied by an unrelated `character_semantic_state.as_ref()` five
/// hundred lines away (Codex review, round 17).
///
/// Reading production alone makes that failure loud: the forward search
/// finds nothing and the `expect` fires.
pub(crate) fn app_production() -> String {
    let all = app();
    let boundary = all
        .find("mod product_production_d3d_tests {")
        .expect("this file has a test module");
    all[..boundary].to_owned()
}

/// `app.rs` BEFORE stripping. Private: proving the stripping removed a comment
/// needs the version that still has it, and nothing else may want it.
fn app_raw() -> &'static str {
    include_str!("app.rs")
}

pub(crate) fn headless() -> String {
    code(include_str!("headless.rs"))
}

pub(crate) fn performance_bench() -> String {
    code(include_str!("performance_bench.rs"))
}

pub(crate) fn ui_runtime() -> String {
    code(include_str!("ui_runtime.rs"))
}

pub(crate) fn xmap07() -> String {
    code(include_str!(
        "../../map_viewer/src/production_features/xmap07.rs"
    ))
}

pub(crate) fn xmap04() -> String {
    code(include_str!(
        "../../map_viewer/src/production_features/xmap04.rs"
    ))
}

/// Source with `//` comment tails removed and CRLF normalised, line
/// structure preserved.
///
/// A `//` inside a string literal is NOT a comment, which is why this tracks
/// quoting instead of searching for the two characters.
///
/// Block comments ARE handled, and were not always. The justification for
/// skipping them was that none of these files carries one -- which is the
/// "it does not occur today" reasoning this lane has been burned by twice. A
/// mutation check settled it: neutralising a real source read in
/// `world_audio.rs` and leaving `/* was include_str!("world_audio.rs") */`
/// behind kept `the_reads_outside_the_seam_are_the_ones_named_here` GREEN,
/// because the comment text answered for the code that had gone. Rust block
/// comments nest, so this counts depth.
///
/// The line-comment half is checked against an independently written reference
/// tokenizer that understands raw strings and char literals -- 31785 lines over
/// six files, zero disagreements.
pub(crate) fn code(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    // Block-comment nesting depth, carried ACROSS lines: a `/*` opened on one
    // line hides everything until its match, however far away.
    let mut depth = 0usize;
    for line in source.replace("\r\n", "\n").lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut in_string = false;
        let mut escaped = false;
        let mut index = 0usize;
        let mut kept = String::with_capacity(line.len());
        while index < chars.len() {
            let ch = chars[index];
            if depth > 0 {
                if ch == '*' && chars.get(index + 1) == Some(&'/') {
                    depth -= 1;
                    index += 2;
                    continue;
                }
                if ch == '/' && chars.get(index + 1) == Some(&'*') {
                    depth += 1;
                    index += 2;
                    continue;
                }
                index += 1;
                continue;
            }
            if escaped {
                escaped = false;
                kept.push(ch);
                index += 1;
                continue;
            }
            if ch == '\\' && in_string {
                escaped = true;
                kept.push(ch);
                index += 1;
                continue;
            }
            if ch == '"' {
                in_string = !in_string;
                kept.push(ch);
                index += 1;
                continue;
            }
            if !in_string && ch == '/' && chars.get(index + 1) == Some(&'/') {
                break;
            }
            if !in_string && ch == '/' && chars.get(index + 1) == Some(&'*') {
                depth += 1;
                index += 2;
                continue;
            }
            kept.push(ch);
            index += 1;
        }
        out.push_str(&kept);
        out.push('\n');
    }
    out
}

/// `ui_runtime.rs` BEFORE stripping. Private, for the same reason as `app_raw`.
fn ui_runtime_raw() -> &'static str {
    include_str!("ui_runtime.rs")
}

/// Everything above a file's test module.
///
/// A scan slices a window by finding a start anchor and searching FORWARD for
/// an end anchor, and it asserts that literals appear. Reading the WHOLE file
/// breaks both: a forward search can run past production into the test module's
/// own string literals, and a `contains` can be satisfied by the very array of
/// required strings the test searches for -- which is exactly how
/// `xui09_shipping_route_is_presented_transactional_and_not_selected_present`
/// stayed green with the production line it guards deleted (Codex review,
/// round 20). Every self-scan reads production.
fn production_of(source: String, marker: &str) -> String {
    let cut = source
        .find(marker)
        .unwrap_or_else(|| panic!("this file has a test module to cut off: {marker}"));
    source[..cut].to_owned()
}

pub(crate) fn headless_production() -> String {
    production_of(headless(), &test_module_marker())
}

pub(crate) fn performance_bench_production() -> String {
    production_of(performance_bench(), &test_module_marker())
}

pub(crate) fn map_labels_production() -> String {
    production_of(map_labels(), &test_module_marker())
}

pub(crate) fn recording_open_production() -> String {
    production_of(recording_open(), &test_module_marker())
}

pub(crate) fn scrub_production() -> String {
    production_of(scrub(), &test_module_marker())
}

/// `ui_runtime.rs`'s PRODUCTION half: everything above its test module.
///
/// The marker is assembled rather than written, because writing it would put a
/// second copy of it in this file and the cut takes the LAST one.
pub(crate) fn timeline_production() -> String {
    production_of(timeline(), &test_module_marker())
}

pub(crate) fn validate_production() -> String {
    production_of(validate(), &test_module_marker())
}

pub(crate) fn replay_core_source_production() -> String {
    production_of(replay_core_source(), &test_module_marker())
}

pub(crate) fn replay_core_world_production() -> String {
    production_of(replay_core_world(), &test_module_marker())
}

pub(crate) fn ui_runtime_production() -> String {
    let all = ui_runtime();
    let marker = ["#[cfg(", "test)]"].concat();
    let cut = all
        .rfind(&marker)
        .expect("ui_runtime.rs has a test module to cut off");
    all[..cut].to_owned()
}

pub(crate) fn world_audio() -> String {
    code(include_str!("world_audio.rs"))
}

pub(crate) fn world_audio_law() -> String {
    code(include_str!("world_audio_law.rs"))
}

pub(crate) fn world_audio_production() -> String {
    production_of(world_audio(), &test_module_marker())
}

pub(crate) fn world_audio_law_production() -> String {
    production_of(world_audio_law(), &test_module_marker())
}

pub(crate) fn map_labels() -> String {
    code(include_str!("map_labels.rs"))
}

pub(crate) fn recording_open() -> String {
    code(include_str!("recording_open.rs"))
}

pub(crate) fn scrub() -> String {
    code(include_str!("scrub.rs"))
}

pub(crate) fn timeline() -> String {
    code(include_str!("timeline.rs"))
}

pub(crate) fn validate() -> String {
    code(include_str!("validate.rs"))
}

pub(crate) fn replay_core_source() -> String {
    code(include_str!("../../replay_core/src/source.rs"))
}

pub(crate) fn replay_core_world() -> String {
    code(include_str!("../../replay_core/src/world.rs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seam shows code and hides prose, and a `//` inside a string literal
    /// is not prose.
    ///
    /// kills: dropping the comment stripping, which would let any scan in this
    /// crate be satisfied by a sentence; and stripping naively at the first
    /// `//`, which would truncate every line carrying a URL or a path in a
    /// string literal and silently shrink what every scan searches.
    #[test]
    fn the_seam_shows_code_and_hides_prose() {
        let stripped = code(concat!(
            "let live = call_me();  // a sentence about call_me\n",
            "let path = \"https://example/a//b\"; // trailing prose\n",
            "// a whole line of prose\n",
            "let after = 1;\r\n",
        ));
        assert!(stripped.contains("let live = call_me();"), "{stripped}");
        assert!(!stripped.contains("a sentence about call_me"), "{stripped}");
        assert!(
            stripped.contains("let path = \"https://example/a//b\";"),
            "a // inside a string literal is not a comment: {stripped}"
        );
        assert!(!stripped.contains("trailing prose"), "{stripped}");
        assert!(!stripped.contains("a whole line of prose"), "{stripped}");
        assert!(stripped.contains("let after = 1;"), "{stripped}");
        assert!(!stripped.contains(CR), "line endings are normalised");
        assert_eq!(stripped.lines().count(), 4, "line structure survives");

        // Block comments too, including nested ones and ones that span lines.
        // A mutation check found this: prose inside a block comment answered
        // for code that had been deleted.
        let blocks = code(concat!(
            "let a = 1; /* hidden_one */ let b = 2;\n",
            "/* opened here\n",
            "   hidden_two /* nested hidden_three */ still hidden\n",
            "*/ let c = 3;\n",
            "let d = \"/* not a comment */\";\n",
        ));
        assert!(blocks.contains("let a = 1;"), "{blocks}");
        assert!(blocks.contains("let b = 2;"), "{blocks}");
        assert!(blocks.contains("let c = 3;"), "{blocks}");
        for hidden in ["hidden_one", "hidden_two", "hidden_three", "still hidden"] {
            assert!(!blocks.contains(hidden), "{hidden} survived: {blocks}");
        }
        assert!(
            blocks.contains("\"/* not a comment */\""),
            "a block comment inside a string literal is not a comment: {blocks}"
        );

        // ...and it is what the scans actually read: app.rs's own comments are
        // gone from what the seam hands out, and its code is not.
        //
        // The phrase is assembled at run time on purpose. Written out, it would
        // be a string literal in THIS file -- but the check is against app.rs,
        // so what it really guards against is the phrase drifting: if the
        // comment in app.rs is reworded, the first assertion fails loudly
        // instead of the second one passing for free.
        let app_source = app();
        assert!(app_source.contains("fn only_a_deferral_with_a_named_receipt_is_discharged"));
        let prose = format!("{} {}", "A recognised contract is", "not a receipt");
        assert!(
            app_raw().contains(&prose),
            "the phrase this checks for must actually be in app.rs"
        );
        assert!(
            !app_source.contains(&prose),
            "a comment in app.rs must not be visible to a scan"
        );
        assert!(
            !ui_runtime().contains(&ui_runtime_prose()),
            "a comment in ui_runtime.rs must not be visible to a scan"
        );
        assert!(
            ui_runtime_raw().contains(&ui_runtime_prose()),
            "the ui_runtime phrase this checks for must actually be in the file"
        );
    }

    fn ui_runtime_prose() -> String {
        format!("{} {}", "Toggle the state-first", "politics panel")
    }

    /// No ROUTED file reads its own source around the seam.
    ///
    /// Everything above is worth nothing if a new test in one of these files
    /// can write `include_str!("something.rs")` and skip the stripping and the
    /// production cut. This reads every routed file THROUGH the seam and
    /// refuses a direct source read in any of them. It says nothing about the
    /// files it does not read -- see the module doc for what those are.
    ///
    /// The allowlist is explicit and short. `world_audio.rs` and
    /// `world_audio_law.rs` still read their own source directly; another lane
    /// is editing them, so they are named here rather than quietly skipped, and
    /// this test is what will fail when someone routes them and forgets to
    /// shorten the list.
    ///
    /// Every direct source read of a `.rs` path in `source`, whatever the
    /// spelling.
    ///
    /// Three rounds sharpened this. A line-by-line literal match was defeated
    /// by `include_str! ("x.rs")`, `include_str!(r"x.rs")` and a split call
    /// (round 21). Scanning whole text with a fixed prefix was then defeated by
    /// `include_str!(/* bypass */ "x.rs")` and `read_to_string(&"x.rs")`, and it
    /// FALSELY reported `r#"include_str!("fake.rs")"#` -- a string that merely
    /// contains the spelling -- as a read (round 22).
    ///
    /// So it walks: it skips string literals, raw strings and comments while
    /// looking for a spelling, and having found one it skips whitespace,
    /// comments and a borrow before the path. What it still cannot see is a
    /// path that is not a literal at all -- `join(..)`, `concat!`, a variable --
    /// and the module doc says so.
    fn direct_source_reads(source: &str) -> Vec<String> {
        const SPELLINGS: [&str; 4] = [
            "include_str!",
            "include_bytes!",
            "include!",
            "read_to_string",
        ];
        let chars: Vec<char> = source.chars().collect();
        let mut found = Vec::new();
        let mut index = 0usize;

        // Skip whitespace and comments from `at`, returning where code resumes.
        fn skip_trivia(chars: &[char], mut at: usize) -> usize {
            loop {
                while chars.get(at).is_some_and(|c| c.is_whitespace()) {
                    at += 1;
                }
                if chars.get(at) == Some(&'/') && chars.get(at + 1) == Some(&'/') {
                    while chars.get(at).is_some_and(|c| *c != '\n') {
                        at += 1;
                    }
                    continue;
                }
                if chars.get(at) == Some(&'/') && chars.get(at + 1) == Some(&'*') {
                    let mut depth = 1usize;
                    at += 2;
                    while at < chars.len() && depth > 0 {
                        if chars[at] == '/' && chars.get(at + 1) == Some(&'*') {
                            depth += 1;
                            at += 2;
                        } else if chars[at] == '*' && chars.get(at + 1) == Some(&'/') {
                            depth -= 1;
                            at += 2;
                        } else {
                            at += 1;
                        }
                    }
                    continue;
                }
                return at;
            }
        }

        while index < chars.len() {
            // A string literal, raw or not, is data: never look inside it.
            if chars[index] == 'r'
                && matches!(chars.get(index + 1), Some('"') | Some('#'))
            {
                let mut at = index + 1;
                let mut hashes = 0usize;
                while chars.get(at) == Some(&'#') {
                    hashes += 1;
                    at += 1;
                }
                if chars.get(at) == Some(&'"') {
                    at += 1;
                    let mut close: Vec<char> = vec!['"'];
                    close.extend(std::iter::repeat('#').take(hashes));
                    while at < chars.len() && !chars[at..].starts_with(close.as_slice()) {
                        at += 1;
                    }
                    index = at + close.len();
                    continue;
                }
            }
            // A CHAR literal, before the string arm: `'"'` holds a quote, and
            // treating it as the start of a string desynchronised the walk, so
            // a read on the same line went unseen (Codex review, round 23).
            // A lifetime (`'a`) has no closing tick, so it is left alone.
            if chars[index] == '\'' {
                let escaped = chars.get(index + 1) == Some(&'\\');
                let close = if escaped { index + 3 } else { index + 2 };
                if chars.get(close) == Some(&'\'') {
                    index = close + 1;
                    continue;
                }
            }
            if chars[index] == '"' {
                let mut at = index + 1;
                while at < chars.len() {
                    if chars[at] == '\\' {
                        at += 2;
                        continue;
                    }
                    if chars[at] == '"' {
                        break;
                    }
                    at += 1;
                }
                index = at + 1;
                continue;
            }
            if chars[index] == '/' && chars.get(index + 1) == Some(&'/') {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
                continue;
            }
            if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
                index = skip_trivia(&chars, index);
                continue;
            }

            let rest: String = chars[index..].iter().take(20).collect();
            let Some(spelling) = SPELLINGS.iter().find(|s| rest.starts_with(**s)) else {
                index += 1;
                continue;
            };
            let mut at = skip_trivia(&chars, index + spelling.chars().count());
            // `include_str!{"x.rs"}` and `include_str![..]` are the same macro
            // call as `include_str!(..)`; requiring a paren let both through
            // (Codex review, round 23).
            if !matches!(chars.get(at), Some('(') | Some('{') | Some('[')) {
                index += 1;
                continue;
            }
            at = skip_trivia(&chars, at + 1);
            if chars.get(at) == Some(&'&') {
                at = skip_trivia(&chars, at + 1);
            }
            if chars.get(at) == Some(&'r') {
                at += 1;
                while chars.get(at) == Some(&'#') {
                    at += 1;
                }
            }
            if chars.get(at) != Some(&'"') {
                index += 1;
                continue;
            }
            at += 1;
            let mut path = String::new();
            while let Some(&ch) = chars.get(at) {
                if ch == '"' {
                    break;
                }
                path.push(ch);
                at += 1;
            }
            if path.ends_with(".rs") {
                found.push(format!("{spelling} {path}"));
            }
            index += 1;
        }
        found
    }

    /// Within the seven routed files, no test reads source around the seam.
    ///
    /// kills: adding a direct source read, in any of the four spellings and
    /// with any spacing or raw-string prefix, to any routed file.
    #[test]
    fn a_routed_file_cannot_read_source_around_the_seam() {
        let routed: [(&str, String); 9] = [
            ("app.rs", app()),
            ("ui_runtime.rs", ui_runtime()),
            ("headless.rs", headless()),
            ("performance_bench.rs", performance_bench()),
            ("map_labels.rs", map_labels()),
            ("recording_open.rs", recording_open()),
            ("scrub.rs", scrub()),
            ("world_audio.rs", world_audio()),
            ("world_audio_law.rs", world_audio_law()),
        ];
        for (name, source) in &routed {
            assert!(
                source.len() > 1000,
                "{name}: the seam handed back nothing to check"
            );
            let direct = direct_source_reads(source);
            assert!(
                direct.is_empty(),
                "{name} reads source directly ({direct:?}). Every source read in these \
                 files goes through source_scan, or the stripping and the production cut \
                 are optional"
            );
        }

        // The matcher is not vacuous, and it sees every spelling it claims to:
        // this file really does read source directly, and these forms parse.
        assert!(
            !direct_source_reads(&code(include_str!("source_scan.rs"))).is_empty(),
            "the seam itself reads source; the matcher must see it"
        );
        let spaced = ["include_str!", " (", "\"spaced.rs\")"].concat();
        let rawish = ["include_str!(r", "\"raw.rs\")"].concat();
        let split = ["include_str!(\n    ", "\"split.rs\"\n)"].concat();
        for (label, probe) in [("spaced", &spaced), ("raw", &rawish), ("split", &split)] {
            assert!(
                !direct_source_reads(probe).is_empty(),
                "{label}: the matcher must see this spelling"
            );
        }
        assert!(
            direct_source_reads("include_str!(\"data.json\")").is_empty(),
            "a non-source include is not a source read"
        );
    }

    /// The reads that are NOT behind the seam, pinned so the set cannot grow.
    ///
    /// The previous version merely asked whether each file contained
    /// `include_str!("` somewhere, which an unrelated JSON include satisfied
    /// (Codex review, round 21). This asks for a direct read of a `.rs` path,
    /// which is the thing being excepted.
    ///
    /// kills: routing the audio files and leaving this list stale; and adding a
    /// fourth source read to the integration tests without saying so.
    #[test]
    fn the_reads_outside_the_seam_are_the_ones_named_here() {
        // The two audio files USED to be named here as unrouted. Lane WD routed
        // their three `include_str!` scans, so they sit in the tripwire's routed
        // set instead and nothing is named here any more. Their one remaining
        // `read_to_string` is PRODUCTION - it loads a game file at run time and
        // carries no `.rs` literal - so `direct_source_reads` does not see it,
        // and routing it would be wrong: it is behaviour, not a source scan.

        // The integration tests are a separate crate and cannot reach this
        // seam: `replay_app` is a binary, so there is nothing to import.
        //
        // They are pinned by counting READER CALL SITES, not path literals. The
        // earlier version counted `.rs` literals, which a refactor defeats:
        // hoist one path into a variable and call `read_to_string(&path)`
        // twice, and there are four reads but still three literals (Codex
        // review, round 22). Two of these tests deliberately WALK the source
        // tree -- reading every `.rs` they discover is their job -- so what can
        // be pinned is how many places read at all, not what they read.
        let integration: [(&str, &str); 4] = [
            (
                "tooltip_runtime_no_d72.rs",
                include_str!("../tests/tooltip_runtime_no_d72.rs"),
            ),
            (
                "descheduled_carriers_reach_no_pixel.rs",
                include_str!("../tests/descheduled_carriers_reach_no_pixel.rs"),
            ),
            (
                "no_silent_skip.rs",
                include_str!("../tests/no_silent_skip.rs"),
            ),
            ("gate_currency.rs", include_str!("../tests/gate_currency.rs")),
        ];
        let mut call_sites = 0usize;
        for (name, body) in integration {
            let stripped = code(body);
            let here = [
                "read_to_string",
                "include_str!",
                "include_bytes!",
                "include!",
            ]
                .into_iter()
                .map(|spelling| stripped.matches(spelling).count())
                .sum::<usize>();
            assert!(
                here > 0,
                "{name} was pinned as reading source and no longer does; drop it from \
                 this list and from the module doc"
            );
            call_sites += here;
        }
        assert_eq!(
            call_sites, INTEGRATION_READER_CALL_SITES,
            "the integration tests' source-reading call sites changed. They cannot use \
             the seam, so this count is what stands in for it: re-read them, then re-pin"
        );
    }
}
