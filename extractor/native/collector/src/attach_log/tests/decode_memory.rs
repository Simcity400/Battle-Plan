use super::*;

use super::state_fixture::FixtureRegion;

#[test]
fn the_live_forms_are_pure_delegations_to_the_generic_bodies() {
    let source = collector_source_lf();
    for delegation in [
        "    fn read(addr: usize) -> Option<ObjectSnapshot<N>> {\n        Self::read_in(&LIVE_MEMORY, addr)\n    }",
        ") -> Result<(String, MsvcStringEncoding), MsvcStringRefusal> {\n    read_msvc_string_with_policy_in(&LIVE_MEMORY, addr, max_len, policy)\n}",
        ") -> Result<(String, MsvcStringEncoding), MsvcStringRefusal> {\n    read_msvc_string_with_snapshot_with_policy_in(\n        &LIVE_MEMORY,\n        base,\n        off,\n        max_len,\n        snapshot,\n        policy,\n    )\n}",
        "fn read_msvc_string_snapshot_once(addr: usize, max_len: usize) -> Option<MsvcStringSnapshot> {\n    read_msvc_string_snapshot_once_in(&LIVE_MEMORY, addr, max_len)\n}",
        "fn read_msvc_display_string_nullable(addr: usize, max_len: usize) -> Option<Option<String>> {\n    read_msvc_display_string_nullable_in(&LIVE_MEMORY, addr, max_len)\n}",
        "fn read_msvc_token_nullable(addr: usize, max_len: usize) -> Option<Option<String>> {\n    read_msvc_token_nullable_in(&LIVE_MEMORY, addr, max_len)\n}",
        ") -> Option<u64> {\n    snapshot_u64_or_read_in(&LIVE_MEMORY, snapshot, base, off)\n}",
        ") -> Option<usize> {\n    snapshot_ptr_or_read_in(&LIVE_MEMORY, snapshot, base, off)\n}",
        ") -> Option<u32> {\n    snapshot_u32_or_read_in(&LIVE_MEMORY, snapshot, base, off)\n}",
        ") -> Option<ObjectSnapshot<N>> {\n    compact_object_window_in(&LIVE_MEMORY, addr, use_windows)\n}",
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

#[test]
fn no_generic_decode_body_reaches_the_live_process_by_name() {
    let source = collector_source_lf();
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
        "read",
        "LIVE_MEMORY",
        "LiveMemory",
        "Domain47InProcessMemory",
        "Domain81SnapshotMemory",
    ];
    let lines = sanitize_lines(&source);
    let mut checked = 0usize;
    for signature in SCANNED_BODIES {
        let (start, end) = body_span(&lines, signature);
        let code = body_code(&lines, start, end);
        // is precisely what treating a lifetime `'a` as a char literal did
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
    assert!(
        !declared.is_empty(),
        "the declaration scan found no generic bodies, so it is no longer \
         matching this file's spelling and the subset check above is vacuous"
    );
    let _ = checked;
}

const SCANNED_BODIES: &[&str] = &[
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
    "fn read_in<M: DecodeMemory + ?Sized>(mem: &M, addr: usize) -> Option<ObjectSnapshot<N>> {",
    "fn decode_and_bound_msvc_string_bytes(",
    "fn decode_msvc_string_bytes_with_policy(",
    "fn decode_msvc_string_bytes(mut bytes: Vec<u8>, size: usize) -> Option<String> {",
    "fn decode_msvc_string_bytes_tolerant(",
    "fn decode_windows_1252(bytes: &[u8]) -> Option<String> {",
    "fn windows_1252_decode_byte(byte: u8) -> Option<char> {",
    "fn is_cosmetic_token_byte(value: u8) -> bool {",
    "fn domain47_portrait_reader_reason(",
    "fn domain47_portrait_reader_pointer(",
    "fn domain47_portrait_reader_terminator(",
];

const LOCALLY_BOUND: &[(&str, &str)] = &[
    ("read_msvc_string_with_policy_in", "refuse"),
    ("read_msvc_string_with_snapshot_with_policy_in", "refuse"),
    ("domain47_portrait_identity_with_snapshot_in", "refuse"),
    ("decode_and_bound_msvc_string_bytes", "refuse"),
];

#[test]
fn every_locally_bound_name_is_really_bound() {
    let source = collector_source_lf();
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

fn body_code(lines: &[String], start: usize, end: usize) -> String {
    let joined: String = lines[start..=end]
        .iter()
        .flat_map(|line| line.chars())
        .filter(|c| !c.is_whitespace())
        .collect();
    strip_turbofish(&joined)
}

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
                    // `SnapshotViewMemory<'a>` and `'_` are LIFETIMES, and
                    // opener would blank from one lifetime to the next —
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

#[test]
fn body_code_reassembles_calls_without_inventing_them() {
    let case = |lines: &[&str]| {
        let sanitized = sanitize_lines(&lines.join("\n"));
        body_code(&sanitized, 0, sanitized.len() - 1)
    };

    assert!(case(&["fn f() {", "    read_u64", "        (addr);", "}"]).contains("read_u64("));
    assert!(case(&[
        "fn f() {",
        "    snapshot_u64_or_read::<",
        "        N,",
        "    >(snapshot, base, off);",
        "}",
    ])
    .contains("snapshot_u64_or_read("));
    assert!(case(&["fn f() {", "    read_u64(addr);", "}"]).contains("read_u64("));

    assert!(
        !case(&["fn f() {", "    // read_u64(addr) used to be here", "}"]).contains("read_u64(")
    );
    assert!(
        case(&["fn f() {", "    mem.u64(addr); // not read_u64(addr)", "}"]).contains("mem.u64(")
    );

    assert!(!case(&[
        "fn f() {",
        "    let g = read_u64;",
        "    (0..n).count();",
        "}"
    ])
    .contains("read_u64("));
    assert!(!case(&[
        "fn f() {",
        "    match x { A => read_u64,",
        "    (a, b) => y }",
        "}"
    ])
    .contains("read_u64("));
}

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
                index = cursor + 1;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

#[test]
fn strip_turbofish_joins_the_name_to_its_parenthesis() {
    for (input, expected) in [
        ("read_u64(addr + 0x10)", "read_u64(addr + 0x10)"),
        (
            "snapshot_u64_or_read::<N>(snapshot, base, off)",
            "snapshot_u64_or_read(snapshot, base, off)",
        ),
        (
            "compact_object_window::<0x20>(addr, true)",
            "compact_object_window(addr, true)",
        ),
        ("foo::<Bar<Baz>>(x)", "foo(x)"),
        ("foo::<Bar<Baz, Qux<u8>>>(x)", "foo(x)"),
        (
            "ObjectSnapshot::<N>::read(addr)",
            "ObjectSnapshot::read(addr)",
        ),
        (
            "if size < max && cap > size {",
            "if size < max && cap > size {",
        ),
        ("let x = a < b;", "let x = a < b;"),
        ("foo::<Bar(x)", "foo::<Bar(x)"),
        (
            "read_scalar::<{ if 2 > 1 { 8 } else { 4 } }>(addr)",
            "read_scalar(addr)",
        ),
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

#[test]
fn the_sanitizer_blanks_only_what_is_not_code() {
    let joined = |source: &str| sanitize_lines(source).join("\n");

    assert!(!joined("let x = 1; // read_u64(addr)").contains("read_u64"));
    assert!(!joined("let x = /* read_u64(addr) */ 1;").contains("read_u64"));
    assert!(!joined("let x = /* a /* read_u64(addr) */ b */ 1;").contains("read_u64"));
    assert!(joined("mem.u64(addr); // read_u64(addr)").contains("mem.u64(addr);"));

    assert!(!joined(r#"let s = "read_u64(addr)";"#).contains("read_u64"));
    assert!(!joined(r#"let s = "a\"read_u64(addr)"; "#).contains("read_u64"));
    assert_eq!(joined("let s = \"}\"; foo();").matches('}').count(), 0);

    assert!(!joined(r####"let s = r"read_u64(addr)";"####).contains("read_u64"));
    assert!(!joined(r####"let s = r#"read_u64(addr)"#;"####).contains("read_u64"));
    assert!(!joined(r####"let s = r##"read_u64(addr)"##;"####).contains("read_u64"));
    assert!(!joined(r####"let s = br"read_u64(addr)";"####).contains("read_u64"));
    assert!(!joined(r####"let s = cr"read_u64(addr)";"####).contains("read_u64"));
    assert!(joined("f(\"NUL terminator\"); read_u64(addr);").contains("read_u64(addr)"));
    assert!(joined(r####"let s = r#"one " quote"#; read_u64(addr);"####).contains("read_u64(addr)"));
    assert!(!joined(r####"let s = r#"one " read_u64(addr)"#;"####).contains("read_u64"));

    assert!(!joined(r"let c = '\'';").contains('\''));
    assert_eq!(joined("let c = '}'; foo();").matches('}').count(), 0);

    // LIFETIMES ARE CODE, NOT LITERALS. This file has 112 of them, and
    // treating each `'` as an opener would blank from one lifetime to the next
    assert!(joined("fn f<'a>(x: &'a str) { read_u64(addr); }").contains("read_u64(addr)"));
    assert!(joined("struct S<'a> { v: View<'a> } fn g() { read_u64(a); }").contains("read_u64(a)"));
    assert!(
        joined("impl SnapshotViewMemory<'_> { fn h() { read_into(a, b); } }")
            .contains("read_into(a, b)")
    );
    // ...and a char literal AFTER two lifetimes still blanks, so the lifetime
    assert_eq!(
        joined("fn f<'a>(x: &'a str) { let c = '}'; }")
            .matches('}')
            .count(),
        1
    );

    assert_eq!(sanitize_lines("a\nb\nc").len(), 3);
    assert_eq!(sanitize_lines("a\n/* x\ny */\nc").len(), 4);
    assert_eq!(sanitize_lines("a\n/* x\ny */\nc")[3], "c");
}

#[test]
fn the_scanned_set_is_closed_under_calls() {
    let source = collector_source_lf();
    let lines = sanitize_lines(&source);

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
        let after_paren = rest
            .split_once('(')
            .map(|(_, tail)| tail.trim())
            .unwrap_or("");
        let first_param = if after_paren.is_empty() {
            lines.get(index + 1).map(|l| l.trim()).unwrap_or("")
        } else {
            after_paren
        };
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
        let shadowed: BTreeSet<&str> = LOCALLY_BOUND
            .iter()
            .filter(|(body, _)| *body == own)
            .map(|(_, name)| *name)
            .collect();
        {
            let code = body_code(&lines, start, end);
            let code = code.as_str();
            for callee in defined.iter().copied() {
                let mut from = 0usize;
                while let Some(offset) = code[from..].find(callee) {
                    let at = from + offset;
                    from = at + 1;
                    let before = code[..at].chars().next_back();
                    let after = code[at + callee.len()..].chars().next();
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                        || after.is_some_and(|c| c.is_alphanumeric() || c == '_')
                    {
                        continue;
                    }
                    if before == Some('.') {
                        continue;
                    }
                    if before == Some(':') {
                        let qualifier = code[..at.saturating_sub(2)]
                            .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
                            .find(|segment| !segment.is_empty())
                            .unwrap_or("");
                        let is_type = qualifier.chars().next().is_some_and(|c| c.is_uppercase())
                            || qualifier == "Self";
                        if is_type {
                            continue;
                        }
                    }
                    if callee == own || scanned.contains(callee) || shadowed.contains(callee) {
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
    assert_eq!(
        source
            .matches("    fn fill(&self, addr: usize, buf: &mut [u8]) -> bool {\n        read_into(addr, buf)\n    }")
            .count(),
        1,
        "`LiveMemory::fill` is no longer the delegation to `read_into`"
    );
}

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
        .ok_or(refuse(Reason::CapacityFieldUnreadable, Storage::NotDecided))?
        as usize;
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
            read_u64(addr).ok_or(refuse(Reason::PayloadPointerUnreadable, Storage::Heap))? as usize,
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
    let cap =
        off.checked_add(0x18)
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

fn baseline_msvc_display_string_nullable(addr: usize, max_len: usize) -> Option<Option<String>> {
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

fn baseline_object_snapshot<const N: usize>(addr: usize) -> Option<[u8; N]> {
    let mut buf = [0u8; N];
    if read_into(addr, &mut buf) {
        Some(buf)
    } else {
        None
    }
}

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
    let lines = sanitize_lines(&source);

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
    assert_eq!(
        scanned, 9,
        "the oracle scan found {scanned} `baseline_*` functions, not the 9 this \
         file defines — one was added or removed without this law being told"
    );
}

struct StringFixture {
    header: FixtureRegion,
    _heap: FixtureRegion,
}

impl StringFixture {
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
    ("size high word set", 0x1_0000_0001, 31, b"", Some(b"a\0")),
    (
        "capacity high word set",
        4,
        0x1_0000_0010,
        b"",
        Some(b"abcd\0"),
    ),
];

const A_B_MAX_LEN: usize = 8;

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
            let baseline = baseline_msvc_string_with_policy(fixture.addr(), A_B_MAX_LEN, policy);
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
    assert!(
        successes > 0 && refusals > 0,
        "the shapes must stage both accepted and refused headers, or the \
         equality proves nothing (successes {successes}, refusals {refusals})"
    );
}

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
    let _ = (with_snapshot, without_snapshot);
    assert!(
        refusals > 0,
        "no header refused, so agreement on refusal reasons is untested"
    );
}

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
        let baseline_display = baseline_msvc_display_string_nullable(fixture.addr(), A_B_MAX_LEN);
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

#[test]
fn the_object_window_and_field_helpers_pin_against_the_pre_migration_bodies() {
    let mut region = FixtureRegion::new(0x40);
    region.write_u64(0x00, 0x0123_4567_89AB_CDEF);
    region.write_u64(0x08, 0xFEDC_BA98_7654_3210);
    region.write_u64(0x18, 0x1122_3344_5566_7788);
    page_cache_new_frame();
    let addr = region.addr();

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

    page_cache_new_frame();
    let shipped_dead = ObjectSnapshot::<0x20>::read(0);
    page_cache_new_frame();
    let baseline_dead = baseline_object_snapshot::<0x20>(0);
    assert!(
        shipped_dead.is_none() && baseline_dead.is_none(),
        "an unbacked address must refuse on both spellings"
    );

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

            page_cache_new_frame();
            let shipped_ptr = snapshot_ptr_or_read(held_snapshot, addr, off);
            assert_eq!(
                shipped_ptr,
                baseline_u64.map(|value| value as usize),
                "snapshot_ptr_or_read at +{off:#x} (snapshot held: {held})"
            );
        }
    }

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
        refused > 0 && cp1252 > 0,
        "the shapes must stage refused AND Windows-1252 identities, or the \
         tolerant ladder is not being compared at all \
         ({accepted} accepted / {refused} refused / {cp1252} cp1252)"
    );
}

const PORTRAIT_SHAPES: &[(&str, u64, u64, &[u8], Option<&[u8]>)] = &[
    ("empty inline", 0, 15, b"\0", None),
    ("empty inline unterminated", 0, 15, b"x", None),
    ("empty heap capacity", 0, 31, b"", Some(b"\0")),
    ("empty heap unterminated", 0, 31, b"", Some(b"x")),
    ("empty heap pointer null", 0, 31, b"", None),
    ("empty capacity implausible", 0, (1 << 20) + 1, b"", None),
    ("ascii inline", 5, 15, b"gfx_a\0", None),
    ("ascii heap", 9, 31, b"", Some(b"gfx_hello\0")),
    ("cp1252 heap", 6, 31, b"", Some(b"g\xe9n\xe9ra\0")),
    ("utf8 heap", 3, 31, b"", Some(b"\xc3\xa9z\0")),
    ("heap unterminated", 5, 31, b"", Some(b"gfx_ax")),
    ("heap control byte", 4, 31, b"", Some(b"gf\x01x\0")),
    ("size over max", 400, 511, b"", Some(b"gfx\0")),
];

#[test]
fn a_held_snapshot_is_the_source_of_the_header_not_the_process() {
    let mut fixture = StringFixture::stage(5, 15, b"abcde\0", None);
    let snapshot = fixture.snapshot();

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

/// `ConsumerId` acquire adjacent spans as two transactions that do NOT
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

    assert_eq!(
        DecodeMemory::scalar::<8>(&memory, addr as usize),
        Some(0x1111_1111_1111_1111u64.to_le_bytes())
    );
    assert_eq!(
        DecodeMemory::scalar::<8>(&memory, (addr + 8) as usize),
        Some(0x2222_2222_2222_2222u64.to_le_bytes())
    );
}

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
    let _ = (coalesced, holed);
}

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
    const MAX_PAYLOAD: u32 = 512;

    let mut compared = 0usize;
    let mut accepted = 0usize;
    let mut refused = 0usize;
    let mut cp1252 = 0usize;
    let mut heap_backed = 0usize;
    let mut inline_backed = 0usize;

    for (label, size, cap, inline, heap) in PORTRAIT_SHAPES {
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
                    trailer_elements: 1,
                    guard: Some(ExtentGuard::at_least((GFX_OFF + 0x18) as u32, 8, 16)),
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

        let live_window = ObjectSnapshot::<ENTRY_LEN>::read(entry.addr())
            .expect("the staged entry is readable live");
        let copied_window = ObjectSnapshot::<ENTRY_LEN>::read_in(&view_memory, entry.addr())
            .unwrap_or_else(|| panic!("`{label}`: the plan did not copy the entry body"));
        assert_eq!(
            live_window.buf, copied_window.buf,
            "`{label}`: the copied entry body is not the engine's bytes"
        );

        page_cache_new_frame();
        let live = domain47_portrait_identity_with_snapshot(entry.addr(), &live_window);
        let copied =
            domain47_portrait_identity_with_snapshot_in(&view_memory, entry.addr(), &copied_window);
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
    let _ = compared;
    assert!(
        refused > 0 && cp1252 > 0,
        "refused and Windows-1252 identities must both be present \
         ({accepted} accepted / {refused} refused / {cp1252} cp1252)"
    );
    assert!(
        heap_backed > 0 && inline_backed > 0,
        "both storage modes must be present ({heap_backed} / {inline_backed})"
    );
}

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

    heap_region.write_bytes(0, b"gfx_after_\0");
    page_cache_new_frame();

    let copied_window = ObjectSnapshot::<ENTRY_LEN>::read_in(&view_memory, entry.addr())
        .expect("the plan copied the entry body");

    let live_window = ObjectSnapshot::<ENTRY_LEN>::read(entry.addr())
        .expect("the staged entry is still readable live");
    let from_copy =
        domain47_portrait_identity_with_snapshot_in(&view_memory, entry.addr(), &copied_window);
    let from_engine =
        domain47_portrait_identity_with_snapshot_in(&LIVE_MEMORY, entry.addr(), &live_window);

    assert_eq!(
        from_copy
            .as_ref()
            .map(|id| id.as_ref().map(|(text, _)| text.as_str())),
        Ok(Some("gfx_before")),
        "the decode over the copy read a payload the bracket did not take"
    );
    assert_eq!(
        from_engine
            .as_ref()
            .map(|id| id.as_ref().map(|(text, _)| text.as_str())),
        Ok(Some("gfx_after_")),
        "the decode over the live memory did not see the post-bracket write, so \
         the fixture never entered the condition this test is about"
    );

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

    entry.write_u64(GFX_OFF + 0x10, 400);
    page_cache_new_frame();

    let copied_window = ObjectSnapshot::<ENTRY_LEN>::read_in(&view_memory, entry.addr())
        .expect("the plan copied the entry body");

    let live_window = ObjectSnapshot::<ENTRY_LEN>::read(entry.addr())
        .expect("the staged entry is still readable live");
    let from_copy =
        domain47_portrait_identity_with_snapshot_in(&view_memory, entry.addr(), &copied_window);
    let from_engine =
        domain47_portrait_identity_with_snapshot_in(&LIVE_MEMORY, entry.addr(), &live_window);

    assert_eq!(
        from_copy
            .as_ref()
            .map(|id| id.as_ref().map(|(text, _)| text.as_str())),
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

    heap_region.write_bytes(0, b"x");
    page_cache_new_frame();

    let live_window = ObjectSnapshot::<ENTRY_LEN>::read(entry.addr())
        .expect("the staged entry is still readable live");
    let from_copy =
        domain47_portrait_identity_with_snapshot_in(&view_memory, entry.addr(), &copied_window);
    let from_engine =
        domain47_portrait_identity_with_snapshot_in(&LIVE_MEMORY, entry.addr(), &live_window);

    assert_eq!(
        from_copy
            .as_ref()
            .map(|id| id.as_ref().map(|(text, _)| text.as_str())),
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

#[test]
fn the_containment_verdict_is_the_one_word_a_copy_cannot_carry() {
    use Domain47PortraitProbePointer as Pointer;
    use MsvcStringRefusalReason as Reason;
    use MsvcStringStorage as Storage;

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

    use crate::snapshot_recipes::view::ViewRefusal;
    use crate::snapshot_recipes::Span;
    let span = Span::new(0x1000, 8);

    let over_a_copy =
        MsvcStringRefusal::new(Reason::PayloadBytesUnreadable, Storage::Heap).with_fetch(
            guarded_refusal_for_view(ViewRefusal::Unreadable { span, void: span }),
        );
    assert_eq!(
        domain47_portrait_reader_pointer(&over_a_copy),
        Pointer::FollowedPayloadOutsideConfirmedRanges,
        "an engine refusal during the bracket must publish the OUTSIDE verdict \
         post-bracket, which is the divergence this test names"
    );
}

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
