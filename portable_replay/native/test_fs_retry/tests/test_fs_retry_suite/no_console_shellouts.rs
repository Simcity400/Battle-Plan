//! kills: lane QI console-window defect class, at its source. The class was:
//! harness code spawning console-subsystem children (the cmd link builtin,
//! cmd ping holders, powershell probes -- via std Command or raw
//! CreateProcessW) without suppressing console allocation, so every
//! landing-gate run opened blank terminal windows on the desktop. Junctions
//! are now native in-process calls, and every remaining sanctioned spawn
//! carries the windowless creation flag. This pin scans the three native
//! trees and fails if the shell-out ever reappears or if a new bare spawn of
//! the same programs lands without its flag.
//!
//! Known static limits (accepted by review): a program name that only reaches
//! cmd/powershell through an arbitrarily renamed variable whose spelling
//! never mentions a shell cannot be resolved by a text scan; ComSpec-variable
//! spellings stay anchored. Builder chains split across several statements
//! behind an intermediate binding are bounded by the `;` window instead of
//! dataflow.

use std::fs;
use std::path::{Path, PathBuf};

/// Only these exact files may contain the guarded vocabulary itself (they
/// define and document the pin). Exemption is by exact path, not by stem.
fn exempt_files(root: &Path) -> Vec<PathBuf> {
    ["portable_replay/native/test_fs_retry/tests/junction_pin.rs",
     "portable_replay/native/test_fs_retry/tests/no_console_shellouts.rs"]
    .iter()
    .map(|relative| {
        let mut path = root.to_path_buf();
        for component in relative.split('/') {
            path.push(component);
        }
        path
    })
    .collect()
}

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <root>/portable_replay/native/test_fs_retry
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("the crate sits three levels below the repository root")
        .to_path_buf()
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// One character kept from the source, with its original 1-based line and
/// whether it is real code (`code`) rather than string/char-literal content.
#[derive(Clone, Copy)]
struct Kept {
    ch: char,
    line: usize,
    code: bool,
}

/// Reduce Rust source to a comment-free, whitespace-free character stream so
/// triggers match across newlines and comments can never satisfy a guard.
///
/// Comments (nested block comments included) are dropped entirely. String and
/// char literal contents are KEPT but marked non-code: trigger detection needs
/// quoted program names like `"cmd.exe"`, while guards must appear as real
/// code. Raw strings -- any hash count including zero -- carry no escape
/// processing (a trailing backslash cannot swallow the closing quote);
/// double-quoted strings track escapes including LF continuations so a `//`
/// inside a literal cannot start a bogus comment strip. The second vector
/// lists every string literal's kept-index span (opening quote .. just past
/// the close).
fn normalize(content: &str) -> (Vec<Kept>, Vec<(usize, usize)>) {
    let chars: Vec<char> = content.chars().collect();
    let mut kept: Vec<Kept> = Vec::with_capacity(chars.len());
    let mut literals: Vec<(usize, usize)> = Vec::new();
    let mut line = 1usize;
    let mut i = 0usize;

    enum State {
        Normal,
        LineComment,
        BlockComment(usize),
        DoubleQuoted,
        RawString(usize),
        CharLiteral,
    }
    let mut state = State::Normal;
    let mut literal_open = 0usize;

    while i < chars.len() {
        let ch = chars[i];
        let at = |offset: usize| chars.get(i + offset).copied();
        match state {
            State::Normal => {
                if ch == '/' && at(1) == Some('/') {
                    state = State::LineComment;
                    i += 2;
                    continue;
                }
                if ch == '/' && at(1) == Some('*') {
                    state = State::BlockComment(0);
                    i += 2;
                    continue;
                }
                // Zero-hash raw strings (r"...") take the raw path -- they
                // have NO escape processing, so a trailing `\` must not eat
                // the closing quote. Byte strings b"..." keep escape rules.
                let byte_plain = ch == 'b' && at(1) == Some('"');
                let raw_zero_hash = ch == 'r' && at(1) == Some('"');
                if ch == '"' || byte_plain {
                    state = State::DoubleQuoted;
                    let span = if byte_plain { 2 } else { 1 };
                    literal_open = kept.len();
                    for offset in 0..span {
                        kept.push(Kept { ch: chars[i + offset], line, code: false });
                    }
                    i += span;
                    continue;
                }
                if raw_zero_hash {
                    literal_open = kept.len();
                    kept.push(Kept { ch, line, code: false });
                    kept.push(Kept { ch: '"', line, code: false });
                    state = State::RawString(0);
                    i += 2;
                    continue;
                }
                if ch == 'r' && at(1) == Some('#') {
                    let mut hashes = 0usize;
                    while at(1 + hashes) == Some('#') {
                        hashes += 1;
                    }
                    if at(1 + hashes) == Some('"') {
                        literal_open = kept.len();
                        for offset in 0..=(hashes + 1) {
                            kept.push(Kept { ch: chars[i + offset], line, code: false });
                        }
                        state = State::RawString(hashes);
                        i += hashes + 2;
                        continue;
                    }
                }
                if ch == '\'' {
                    let closes_plain = at(1).is_some() && at(2) == Some('\'');
                    let closes_escaped = at(1) == Some('\\') && at(3) == Some('\'');
                    if closes_plain || closes_escaped {
                        state = State::CharLiteral;
                    }
                }
                if ch.is_whitespace() {
                    if ch == '\n' {
                        line += 1;
                    }
                    i += 1;
                    continue;
                }
                kept.push(Kept { ch, line, code: true });
            }
            State::LineComment => {
                if ch == '\n' {
                    state = State::Normal;
                }
            }
            State::BlockComment(depth) => {
                if ch == '/' && at(1) == Some('*') {
                    state = State::BlockComment(depth + 1);
                    i += 2;
                    continue;
                }
                if ch == '*' && at(1) == Some('/') {
                    if depth == 0 {
                        state = State::Normal;
                    } else {
                        state = State::BlockComment(depth - 1);
                    }
                    i += 2;
                    continue;
                }
            }
            State::DoubleQuoted => {
                kept.push(Kept { ch, line, code: false });
                if ch == '\\' {
                    if let Some(escaped) = at(1) {
                        if escaped == '\n' {
                            line += 1;
                        }
                        kept.push(Kept { ch: escaped, line, code: false });
                        i += 2;
                        continue;
                    }
                } else if ch == '"' {
                    literals.push((literal_open, kept.len()));
                    state = State::Normal;
                }
            }
            State::RawString(hashes) => {
                kept.push(Kept { ch, line, code: false });
                if ch == '"' {
                    let mut seen = 0usize;
                    while seen < hashes && at(1 + seen) == Some('#') {
                        seen += 1;
                    }
                    if seen == hashes {
                        for _ in 0..hashes {
                            kept.push(Kept { ch: '#', line, code: false });
                        }
                        literals.push((literal_open, kept.len()));
                        i += hashes;
                        state = State::Normal;
                    }
                }
            }
            State::CharLiteral => {
                kept.push(Kept { ch, line, code: false });
                if ch == '\\' {
                    if let Some(escaped) = at(1) {
                        kept.push(Kept { ch: escaped, line, code: false });
                        i += 2;
                        continue;
                    }
                } else if ch == '\'' {
                    state = State::Normal;
                }
            }
        }
        if ch == '\n' {
            line += 1;
        }
        i += 1;
    }
    (kept, literals)
}

fn line_of(kept: &[Kept], index: usize) -> usize {
    kept.get(index).map(|k| k.line).unwrap_or(0)
}

/// Allocation-free needle match against the lowered stream.
fn matches_at(lowered: &[char], at: usize, needle: &[char]) -> bool {
    at + needle.len() <= lowered.len() && &lowered[at..at + needle.len()] == needle
}

/// Needle match where every matched character must be real code.
///
/// Written as an explicit index loop deliberately: the previous form,
/// `(at..at + needle.len()).all(|offset| is_code_at(kept, at + offset))`,
/// double-added the absolute position and inspected 2*at..2*at+len instead
/// of the matched slice, so every guard scan silently failed.
fn matches_code_at(kept: &[Kept], lowered: &[char], at: usize, needle: &[char]) -> bool {
    if !matches_at(lowered, at, needle) {
        return false;
    }
    for offset in 0..needle.len() {
        if !is_code_at(kept, at + offset) {
            return false;
        }
    }
    true
}

fn is_code_at(kept: &[Kept], at: usize) -> bool {
    kept.get(at).map(|k| k.code).unwrap_or(false)
}

/// Rust identifiers are Unicode XID sequences; this pin has no Unicode
/// database, so every non-ASCII character is conservatively treated as an
/// identifier continuation (a neighbor can never split or fake a token, and
/// combining marks cannot smuggle a separately bounded CREATE_NO_WINDOW).
fn is_ident_char(ch: char) -> bool {
    !ch.is_ascii() || ch.is_ascii_alphanumeric() || ch == '_'
}

/// End of the builder-chain window: the first CODE terminal method or the
/// statement end. String-literal characters can never terminate a window (a
/// `";"` inside `"echo;done"` is not a statement boundary).
fn window_end_after(
    lowered: &[char],
    kept: &[Kept],
    start: usize,
    terminals: &[Vec<char>],
) -> usize {
    let mut best = lowered.len();
    for terminal in terminals {
        let mut probe = start;
        while probe + terminal.len() <= lowered.len() {
            if matches_code_at(kept, lowered, probe, terminal) {
                if probe < best {
                    best = probe;
                }
                break;
            }
            probe += 1;
        }
    }
    best
}

/// Needle match where the token is delimited by non-identifier characters on
/// both sides (NOT_CREATE_NO_WINDOW must not satisfy CREATE_NO_WINDOW).
fn matches_token_at(kept: &[Kept], lowered: &[char], at: usize, needle: &[char]) -> bool {
    if !matches_code_at(kept, lowered, at, needle) {
        return false;
    }
    let left_bounded = at == 0
        || !is_code_at(kept, at - 1)
        || !is_ident_char(lowered[at - 1]);
    let end = at + needle.len();
    let right_bounded = end == lowered.len()
        || !is_code_at(kept, end)
        || !is_ident_char(lowered[end]);
    left_bounded && right_bounded
}

/// The only flag token that satisfies a `.creation_flags(` guard.
const CREATE_NO_WINDOW_TOKEN: [char; 16] = [
    'c', 'r', 'e', 'a', 't', 'e', '_', 'n', 'o', '_', 'w', 'i', 'n', 'd', 'o', 'w',
];

/// A `.creation_flags(` guard counts only when its own ARGUMENTS contain the
/// CREATE_NO_WINDOW identifier as a whole token -- `.creation_flags(
/// CREATE_SUSPENDED)` or `.creation_flags(FAKE_CREATE_NO_WINDOW)` suppress
/// nothing. Parenthesis tracking and token splitting ignore string-literal
/// characters: text inside `"..."` can neither contribute the token nor
/// close the argument scan early.
fn flags_arg_names_no_window(
    lowered: &[char],
    kept: &[Kept],
    from: usize,
    to: usize,
    open_needle: &[char],
) -> bool {
    let mut at = from;
    while at + open_needle.len() <= to {
        if !matches_code_at(kept, lowered, at, open_needle) {
            at += 1;
            continue;
        }
        let mut depth = 1usize;
        let mut cursor = at + open_needle.len();
        let arg_start = cursor;
        while cursor < to && depth > 0 {
            if !is_code_at(kept, cursor) {
                cursor += 1;
                continue;
            }
            match lowered[cursor] {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                break;
            }
            cursor += 1;
        }
        let arg_end = cursor.min(to);
        let mut index = arg_start;
        while index < arg_end {
            if matches_token_at(kept, lowered, index, &CREATE_NO_WINDOW_TOKEN) {
                return true;
            }
            index += 1;
        }
        at = arg_end.max(at + open_needle.len());
    }
    false
}

fn window_has_windowless_guard(
    lowered: &[char],
    kept: &[Kept],
    from: usize,
    to: usize,
    flags_open: &[char],
    adapter: &[char],
) -> bool {
    if flags_arg_names_no_window(lowered, kept, from, to, flags_open) {
        return true;
    }
    let mut probe = from;
    while probe + adapter.len() <= to {
        if matches_code_at(kept, lowered, probe, adapter) {
            return true;
        }
        probe += 1;
    }
    false
}

/// Console-shell reference inside a string literal's content.
fn shell_literal_ref(content_lower: &str) -> bool {
    content_lower.contains("cmd.exe")
        || content_lower.contains("powershell")
        || content_lower.contains("comspec")
        || content_lower == "cmd"
        || content_lower.ends_with("\\cmd")
}

#[test]
fn no_native_tree_builds_junctions_or_drives_disks_through_shellouts_anymore() {
    // The link builtin (`mklink`), robocopy, and fsutil have zero legitimate
    // callers left: junctions come from portable_test_fs_retry's native
    // helper. Any reappearance -- production or test, including inside a
    // string argument -- fails here. Matched case-insensitively; program
    // names are case-insensitive on Windows too.
    let mut offenders: Vec<String> = Vec::new();
    let root = repo_root();
    let exempt = exempt_files(&root);
    let vocabularies: Vec<Vec<char>> = ["mklink", "robocopy", "fsutil"]
        .iter()
        .map(|token| token.chars().collect())
        .collect();
    for tree in ["extractor/native", "portable_replay/native", "replay_interface/native"] {
        for file in rust_sources(&root.join(tree)) {
            if exempt.iter().any(|e| e == &file) {
                continue;
            }
            let content = match fs::read_to_string(&file) {
                Ok(content) => content,
                Err(_) => continue,
            };
            let (kept, _) = normalize(&content);
            let lowered: Vec<char> =
                kept.iter().map(|k| k.ch.to_ascii_lowercase()).collect();
            for token in &vocabularies {
                let hit = (0..lowered.len().saturating_sub(token.len() - 1))
                    .find(|&start| matches_at(&lowered, start, token));
                if let Some(start) = hit {
                    offenders.push(format!(
                        "  {}:{}: shell-out vocabulary {}",
                        file.display(),
                        line_of(&kept, start),
                        token.iter().collect::<String>()
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "console shell-out vocabulary returned to the source trees:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn every_remaining_spawn_of_the_console_shells_carries_the_windowless_flag() {
    // Sanctioned survivors must each carry real window suppression inside the
    // spawn's statement window. A spawn counts as shell-spawning when it is a
    // `Command::new(` call -- receiver identifier exactly `command`, bounded
    // on the left by a non-identifier (so OtherCommand::new stays ignored),
    // so OsStr::new/PathBuf::new over shell text stay ignored too -- whose
    // statement window references a console shell anywhere: a program STRING
    // naming one ("cmd", absolute r"C:\Windows\System32\cmd.exe" paths,
    // powershell), or ComSpec-variable code in the window. Raw Win32 spawns
    // are held to the same bar per #[cfg(test)] span. A guard counts only
    // when it is REAL suppression: `.creation_flags(` whose arguments contain
    // the CREATE_NO_WINDOW token, or the compact_contract
    // `.no_console_window()` adapter. Comments and string text can never
    // satisfy a guard; multiline builders are caught by the whitespace-free
    // stream.
    let command_new: Vec<char> = "command::new(".chars().collect();
    let raw_spawn: Vec<char> = "createprocessw(".chars().collect();
    let raw_guard: Vec<char> = "create_no_window".chars().collect();
    let flags_open: Vec<char> = ".creation_flags(".chars().collect();
    let adapter: Vec<char> = ".no_console_window(".chars().collect();
    let comspec: Vec<char> = "comspec".chars().collect();
    let cfg_test_marker: Vec<char> = "#[cfg(test)]".chars().collect();
    let terminals: Vec<Vec<char>> = [".spawn(", ".status(", ".output(", ";"]
        .iter()
        .map(|terminal| terminal.chars().collect())
        .collect();
    let shell_refs: Vec<Vec<char>> = ["cmd.exe", "comspec", "powershell"]
        .iter()
        .map(|shell| shell.chars().collect())
        .collect();

    let mut offenders: Vec<String> = Vec::new();
    let root = repo_root();
    let exempt = exempt_files(&root);
    for tree in ["extractor/native", "portable_replay/native", "replay_interface/native"] {
        for file in rust_sources(&root.join(tree)) {
            if exempt.iter().any(|e| e == &file) {
                continue;
            }
            let content = match fs::read_to_string(&file) {
                Ok(content) => content,
                Err(_) => continue,
            };
            let (kept, literals) = normalize(&content);
            let lowered: Vec<char> =
                kept.iter().map(|k| k.ch.to_ascii_lowercase()).collect();

            // Positions are collected ONCE per file and reused across spans;
            // every needle is precomputed, so no per-character allocations
            // happen inside the scans (attach_log.rs alone is megabytes).
            let mut command_new_positions: Vec<usize> = Vec::new();
            let mut raw_spawn_positions: Vec<usize> = Vec::new();
            let mut marker_positions: Vec<usize> = Vec::new();
            let mut at = 0usize;
            while at < lowered.len() {
                if is_code_at(&kept, at) {
                    if matches_at(&lowered, at, &command_new)
                        && (at == 0
                            || !is_code_at(&kept, at - 1)
                            || !is_ident_char(lowered[at - 1]))
                    {
                        command_new_positions.push(at);
                    }
                    if matches_at(&lowered, at, &raw_spawn) {
                        raw_spawn_positions.push(at);
                    }
                    if matches_at(&lowered, at, &cfg_test_marker) {
                        marker_positions.push(at);
                    }
                }
                at += 1;
            }

            // ---- std Command spawns -------------------------------------
            for &start in &command_new_positions {
                let end = window_end_after(
                    &lowered,
                    &kept,
                    start + command_new.len(),
                    &terminals,
                );
                let mut references_shell = false;
                for &(open, close_exclusive) in &literals {
                    if open < start + command_new.len() || open >= end {
                        continue;
                    }
                    // The literal span opens at its prefix marks (r/b/#) and
                    // closes after the closing quote (plus hashes). The
                    // content runs from just past the FIRST quote to just
                    // before the LAST quote of the span.
                    let mut content_start = open;
                    while content_start < close_exclusive
                        && kept[content_start].ch != '"'
                    {
                        content_start += 1;
                    }
                    content_start += 1;
                    let mut closing = close_exclusive.saturating_sub(1);
                    while closing > content_start && kept[closing].ch != '"' {
                        closing -= 1;
                    }
                    if content_start >= closing {
                        continue;
                    }
                    let content_lower: String =
                        lowered[content_start..closing].iter().collect();
                    if shell_literal_ref(&content_lower) {
                        references_shell = true;
                        break;
                    }
                }
                if !references_shell {
                    // ComSpec-variable CODE in the same window also counts.
                    let mut probe = start;
                    while probe + comspec.len() <= end {
                        if matches_token_at(&kept, &lowered, probe, &comspec) {
                            references_shell = true;
                            break;
                        }
                        probe += 1;
                    }
                }
                if references_shell
                    && !window_has_windowless_guard(
                        &lowered,
                        &kept,
                        start,
                        end,
                        &flags_open,
                        &adapter,
                    )
                {
                    offenders.push(format!(
                        "  {}:{}: bare Command::new of a console shell with no \
                         windowless flag",
                        file.display(),
                        line_of(&kept, start)
                    ));
                }
            }

            // ---- raw Win32 spawns ---------------------------------------
            // The file is split into spans at its `#[cfg(test)]` markers, and
            // a span that references a console shell anywhere must have every
            // one of its CreateProcessW calls carry CREATE_NO_WINDOW. The
            // shell reference normally precedes the call (the wide command
            // line is built first), so no fragile function-boundary parsing
            // is involved. Spans without shell references -- production
            // launches of the game's own image -- pass. FFI declarations and
            // non-code string mentions are skipped.
            marker_positions.push(lowered.len());
            let mut span_start = 0usize;
            for marker_at in &marker_positions {
                let segment_end = *marker_at;
                if segment_end <= span_start {
                    span_start = segment_end;
                    continue;
                }
                let span_spawns: Vec<usize> = raw_spawn_positions
                    .iter()
                    .copied()
                    .filter(|&spawn_at| {
                        spawn_at >= span_start
                            && spawn_at < segment_end
                            // skip FFI declarations: `fn CreateProcessW(`
                            && !(spawn_at >= span_start + 2
                                && lowered[spawn_at - 2..spawn_at] == ['f', 'n'])
                    })
                    .collect();
                let references_a_shell = shell_refs
                    .iter()
                    .any(|needle| {
                        (span_start..segment_end.saturating_sub(needle.len() - 1))
                            .any(|probe| matches_at(&lowered, probe, needle))
                    });
                if !span_spawns.is_empty() && references_a_shell {
                    for start in span_spawns {
                        let end = window_end_after(
                            &lowered,
                            &kept,
                            start + raw_spawn.len(),
                            &terminals,
                        );
                        let mut guarded = false;
                        let mut probe = start;
                        while probe + raw_guard.len() <= end {
                            if matches_token_at(&kept, &lowered, probe, &raw_guard) {
                                guarded = true;
                                break;
                            }
                            probe += 1;
                        }
                        if !guarded {
                            offenders.push(format!(
                                "  {}:{}: raw CreateProcessW of a console shell with no \
                                 CREATE_NO_WINDOW",
                                file.display(),
                                line_of(&kept, start)
                            ));
                        }
                    }
                }
                span_start = segment_end;
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "console-shell spawns lost their window suppression:\n{}",
        offenders.join("\n")
    );
}
