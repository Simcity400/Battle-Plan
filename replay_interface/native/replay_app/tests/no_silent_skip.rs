
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn strip_noise(src: &str) -> String {
    let bytes: Vec<char> = src.chars().collect();
    let mut out = bytes.clone();
    let mut i = 0usize;
    let n = bytes.len();
    let blank = |out: &mut Vec<char>, from: usize, to: usize| {
        for slot in out.iter_mut().take(to.min(n)).skip(from) {
            if *slot != '\n' {
                *slot = ' ';
            }
        }
    };
    while i < n {
        let c = bytes[i];
        if c == '/' && i + 1 < n && bytes[i + 1] == '/' {
            let mut j = i;
            while j < n && bytes[j] != '\n' {
                j += 1;
            }
            blank(&mut out, i, j);
            i = j;
        } else if c == '/' && i + 1 < n && bytes[i + 1] == '*' {
            let mut j = i + 2;
            let mut depth = 1usize;
            while j < n && depth > 0 {
                if bytes[j] == '/' && j + 1 < n && bytes[j + 1] == '*' {
                    depth += 1;
                    j += 2;
                } else if bytes[j] == '*' && j + 1 < n && bytes[j + 1] == '/' {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            blank(&mut out, i, j);
            i = j;
        } else if c == 'r' && i + 1 < n && (bytes[i + 1] == '"' || bytes[i + 1] == '#') {
            // Raw string: r"..." / r#"..."# / r##"..."## ...
            let mut hashes = 0usize;
            let mut j = i + 1;
            while j < n && bytes[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < n && bytes[j] == '"' {
                j += 1;
                loop {
                    if j >= n {
                        break;
                    }
                    if bytes[j] == '"' {
                        let mut k = j + 1;
                        let mut seen = 0usize;
                        while k < n && bytes[k] == '#' && seen < hashes {
                            seen += 1;
                            k += 1;
                        }
                        if seen == hashes {
                            j = k;
                            break;
                        }
                    }
                    j += 1;
                }
                blank(&mut out, i, j);
                i = j;
            } else {
                i += 1;
            }
        } else if c == '"' {
            let mut j = i + 1;
            while j < n {
                if bytes[j] == '\\' {
                    j += 2;
                    continue;
                }
                if bytes[j] == '"' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            blank(&mut out, i, j);
            i = j;
        } else if c == '\'' {
            // A char literal, or a lifetime. Only literals are blanked.
            if i + 2 < n && bytes[i + 1] == '\\' {
                let mut j = i + 2;
                while j < n && bytes[j] != '\'' {
                    j += 1;
                }
                blank(&mut out, i, (j + 1).min(n));
                i = j + 1;
            } else if i + 2 < n && bytes[i + 2] == '\'' {
                blank(&mut out, i, i + 3);
                i += 3;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out.into_iter().collect()
}

/// Byte index of the `}` matching the `{` at `open`.
///
/// Byte indices throughout: `{`, `}`, `|`, `f`, `n` are all single-byte in
/// UTF-8 and can never occur as a continuation byte, so byte scanning is exact
/// even in a file with non-ASCII identifiers. (Mixing `str` byte offsets with
/// `Vec<char>` indices, as an earlier version did, silently skewed every
/// offset after the first multi-byte character.)
fn matching_brace(bytes: &[u8], open: usize) -> Option<usize> {
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    for (idx, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// Byte index of the delimiter matching the `{`, `(` or `[` at `open`.
fn matching_delimiter(bytes: &[u8], open: usize) -> Option<usize> {
    let (opener, closer) = match bytes.get(open)? {
        b'{' => (b'{', b'}'),
        b'(' => (b'(', b')'),
        b'[' => (b'[', b']'),
        _ => return None,
    };
    let mut depth = 0usize;
    for (idx, byte) in bytes.iter().enumerate().skip(open) {
        if *byte == opener {
            depth += 1;
        } else if *byte == closer {
            depth -= 1;
            if depth == 0 {
                return Some(idx);
            }
        }
    }
    None
}

/// Byte index just past a `#[test]` attribute starting at or after `from`,
/// tolerating the whitespace Rust allows (`#[ test ]`, `#[test ]`).
fn find_test_attribute(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i + 2 < bytes.len() {
        if bytes[i] == b'#' && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes[j..].starts_with(b"test") {
                let mut k = j + 4;
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                if bytes.get(k) == Some(&b']') {
                    return Some((i, k + 1));
                }
            }
        }
        i += 1;
    }
    None
}

/// Byte index of the next `fn` keyword, accepting any Rust whitespace between
/// `fn` and the function name.
fn find_fn_keyword(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 2 < bytes.len() {
        if bytes[i] == b'f'
            && bytes[i + 1] == b'n'
            && bytes[i + 2].is_ascii_whitespace()
            && (i == 0 || !is_ident_byte(bytes[i - 1]))
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Is the `|` at `bar` the closing bar of a closure parameter list?
///
/// `||` is one. Otherwise find the candidate opening `|` — refusing to cross a
/// statement or block boundary — and then decide from what precedes IT whether
/// this is a parameter list or a chain of bitwise ORs. `true | false | { .. }`
/// has an identifier before the candidate opening bar, so it is an operand,
/// not a parameter list; `move |x| { .. }`, `f(|x| { .. })` and `let g = |x| {`
/// do not.
///
/// Returning `false` when unsure is deliberate: an unrecognised block stays IN
/// the scanned body, so a `return` inside it is REPORTED. A false positive
/// here is a noisy failure someone fixes; a false negative is exactly the
/// silent pass this file exists to prevent.
fn closes_closure_params(bytes: &[u8], bar: usize, from: usize) -> bool {
    let open = if bar > from && bytes[bar - 1] == b'|' {
        bar - 1 // candidate `||`; still inspect its left context below
    } else {
        let mut open = bar;
        loop {
            if open == from {
                return false;
            }
            open -= 1;
            match bytes[open] {
                b'|' => break open,
                b';' | b'{' | b'}' => return false,
                _ => {}
            }
        }
    };
    // What sits immediately before the opening bar?
    let mut i = open;
    while i > from && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    if i == from {
        return true;
    }
    let prev = bytes[i - 1];
    if prev == b')' || prev == b']' || prev == b'}' {
        return false; // `f(x) | { .. }` / `{ value } | { .. }`
    }
    if is_ident_byte(prev) {
        // An identifier or literal: a bitwise operand — UNLESS it is the
        // `move` keyword introducing a closure.
        let end = i;
        let mut start = end;
        while start > from && is_ident_byte(bytes[start - 1]) {
            start -= 1;
        }
        let keyword = &bytes[start..end];
        return keyword == b"move" || keyword == b"async";
    }
    // `(`, `,`, `=`, `{`, `;`, an operator, ... — an expression position.
    true
}

/// Blank out closure bodies and nested `fn` bodies inside `[from, to)`, so a
/// `return` belonging to them is not attributed to the enclosing test.
fn blank_inner_bodies(bytes: &mut [u8], from: usize, to: usize) {
    let mut i = from;
    while i < to {
        // Nested `fn` item: the `fn` keyword, then an identifier, then `(`
        // before any `{` or `;`. The old version accepted a bare `fn` token and
        // scanned forward without that shape check.
        let is_fn_keyword = bytes[i] == b'f'
            && i + 2 < to
            && bytes[i + 1] == b'n'
            && bytes[i + 2].is_ascii_whitespace()
            && (i == from || !is_ident_byte(bytes[i - 1]));
        let closure_brace = bytes[i] == b'{' && {
            let mut j = i;
            while j > from && bytes[j - 1].is_ascii_whitespace() {
                j -= 1;
            }
            j > from && bytes[j - 1] == b'|' && closes_closure_params(bytes, j - 1, from)
        };

        let body_open = if is_fn_keyword {
            let mut j = i + 3;
            while j < to && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let name_start = j;
            while j < to && is_ident_byte(bytes[j]) {
                j += 1;
            }
            let named = j > name_start;
            // Find `(` before any `{` or `;`; then the body brace after it.
            let mut k = j;
            while k < to && bytes[k] != b'(' && bytes[k] != b'{' && bytes[k] != b';' {
                k += 1;
            }
            if named && k < to && bytes[k] == b'(' {
                let mut b = k;
                while b < to && bytes[b] != b'{' && bytes[b] != b';' {
                    b += 1;
                }
                (b < to && bytes[b] == b'{').then_some(b)
            } else {
                None
            }
        } else if closure_brace {
            Some(i)
        } else {
            None
        };

        if let Some(open) = body_open {
            if let Some(end) = matching_brace(bytes, open) {
                for slot in bytes.iter_mut().take(end.min(to)).skip(open) {
                    if *slot != b'\n' {
                        *slot = b' ';
                    }
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
}

/// A `macro_rules!` body containing `return` can smuggle an early exit into a
/// test body where no lexical scan of that body could see it
/// (`macro_rules! skip { () => { return; } }` then `skip!();`). There are no
/// such macros in these trees, and this keeps it that way.
fn scan_macro_definitions(path: &Path, src: &str, out: &mut Vec<Offence>) {
    let clean = strip_noise(src);
    let bytes = clean.as_bytes();
    let mut search = 0usize;
    while let Some(rel) = clean[search..].find("macro_rules") {
        let at = search + rel;
        let token_end = at + "macro_rules".len();
        search = token_end;
        if (at > 0 && is_ident_byte(bytes[at - 1]))
            || bytes.get(token_end).is_some_and(|byte| is_ident_byte(*byte))
        {
            continue;
        }
        while search < bytes.len() && bytes[search].is_ascii_whitespace() {
            search += 1;
        }
        if bytes.get(search) != Some(&b'!') {
            continue;
        }
        search += 1;
        while search < bytes.len() && bytes[search].is_ascii_whitespace() {
            search += 1;
        }
        if bytes
            .get(search..search + 2)
            .is_some_and(|prefix| prefix == b"r#")
        {
            search += 2;
        }
        let name_start = search;
        while search < bytes.len() && is_ident_byte(bytes[search]) {
            search += 1;
        }
        if search == name_start {
            continue;
        }
        while search < bytes.len() && bytes[search].is_ascii_whitespace() {
            search += 1;
        }
        // The definition body may be delimited by `{}`, `()` or `[]` — all
        // three are legal, and looking only for `{` would walk past a
        // `macro_rules! skip ( () => (return) );` into the next item's brace.
        if !bytes
            .get(search)
            .is_some_and(|byte| matches!(*byte, b'{' | b'(' | b'['))
        {
            continue;
        }
        let open = search;
        let Some(end) = matching_delimiter(bytes, open) else {
            continue;
        };
        let body = &clean[open..=end];
        if find_test_attribute(body.as_bytes(), 0).is_some() {
            out.push(Offence {
                file: path.to_path_buf(),
                line: clean[..at].matches('\n').count() + 1,
                test: "macro-generated #[test]".to_string(),
                what: "a macro definition emits `#[test]`; a body supplied at another expansion \
                       site can hide an early return from this lexical guard"
                    .to_string(),
            });
        }
        if contains_return_token(body) {
            out.push(Offence {
                file: path.to_path_buf(),
                line: clean[..at].matches('\n').count() + 1,
                test: "macro_rules!".to_string(),
                what: "a macro whose expansion contains `return` can end a test early where no \
                       scan of the test body can see it"
                    .to_string(),
            });
        }
        search = end;
    }
}

/// Does `text` contain `return` as a token (not as part of an identifier)?
fn contains_return_token(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.match_indices("return").any(|(idx, _)| {
        let before_ok = idx == 0 || !is_ident_byte(bytes[idx - 1]);
        let after = bytes.get(idx + "return".len());
        let after_ok = after.is_none_or(|byte| !is_ident_byte(*byte));
        before_ok && after_ok
    })
}

#[derive(Debug)]
struct Offence {
    file: PathBuf,
    line: usize,
    test: String,
    what: String,
}

fn push_unscannable_test(
    path: &Path,
    line: usize,
    detail: &str,
    out: &mut Vec<Offence>,
) {
    out.push(Offence {
        file: path.to_path_buf(),
        line,
        test: "<unscannable #[test]>".to_string(),
        what: format!(
            "test attribute could not be structurally scanned ({detail}); macro-generated test \
             bodies are unsupported because they can hide an early return"
        ),
    });
}

fn scan_source(path: &Path, src: &str, out: &mut Vec<Offence>) {
    scan_macro_definitions(path, src, out);
    let text = strip_noise(src);
    let line_of = |idx: usize| text[..idx].matches('\n').count() + 1;

    let mut search = 0usize;
    while let Some((attr, attr_end)) = find_test_attribute(text.as_bytes(), search) {
        search = attr_end;
        // The `fn` that this attribute decorates: the next `fn` token, unless
        // another `#[test]` intervenes (which would mean a malformed scan).
        let Some(fn_at) = find_fn_keyword(text.as_bytes(), search) else {
            push_unscannable_test(path, line_of(attr), "no following `fn` token", out);
            continue;
        };
        if find_test_attribute(text.as_bytes(), search).is_some_and(|(next, _)| next < fn_at) {
            push_unscannable_test(
                path,
                line_of(attr),
                "another test attribute intervenes before the function",
                out,
            );
            continue;
        }
        let mut name_start = fn_at + 2;
        while name_start < text.len() && text.as_bytes()[name_start].is_ascii_whitespace() {
            name_start += 1;
        }
        let name: String = text[name_start..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            push_unscannable_test(path, line_of(attr), "function name is not literal", out);
            continue;
        }
        let body_marker = text.as_bytes()[fn_at..]
            .iter()
            .position(|byte| matches!(*byte, b'{' | b';' | b'}'))
            .map(|offset| fn_at + offset);
        let Some(brace) = body_marker.filter(|idx| text.as_bytes()[*idx] == b'{') else {
            push_unscannable_test(path, line_of(attr), "no literal function body", out);
            continue;
        };
        // Anything between the closing `)` of the parameter list and the body
        // brace that contains `->` is a non-unit return type.
        if text[fn_at..brace].contains("->") {
            out.push(Offence {
                file: path.to_path_buf(),
                line: line_of(fn_at),
                test: name.clone(),
                what: "declares a non-unit return type; a test that can return a success value \
                       can end early with a passing verdict"
                    .to_string(),
            });
        }
        let Some(end) = matching_brace(text.as_bytes(), brace) else {
            push_unscannable_test(path, line_of(attr), "unmatched function body brace", out);
            continue;
        };

        let mut body: Vec<u8> = text.as_bytes()[brace..=end].to_vec();
        let len = body.len();
        blank_inner_bodies(&mut body, 0, len);
        let body_text = String::from_utf8_lossy(&body).into_owned();
        let body_bytes = body_text.as_bytes();
        for (idx, _) in body_text.match_indices("return") {
            let before_ok = idx == 0 || !is_ident_byte(body_bytes[idx - 1]);
            let after = body_bytes.get(idx + "return".len());
            let after_ok = after.is_none_or(|byte| !is_ident_byte(*byte));
            if before_ok && after_ok {
                out.push(Offence {
                    file: path.to_path_buf(),
                    line: line_of(brace) + body_text[..idx].matches('\n').count(),
                    test: name.clone(),
                    what: "returns early; libtest reports an early return as a PASS. Use \
                           if/else, or a build-time gate from `replay_test_gates`"
                        .to_string(),
                });
            }
        }
    }
}

fn scan_tree(root: &Path, out: &mut Vec<Offence>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        panic!("cannot read scan root {}", root.display());
    };
    for entry in entries {
        let path = entry.expect("directory entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            // Build output and vendored trees are not our source.
            if name == "target" || name == ".git" || name.starts_with(".cargo-target") {
                continue;
            }
            scan_tree(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let src = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            scan_source(&path, &src, out);
        }
    }
}

/// The scanner's own rules, proven on synthetic source, so the workspace scan
/// below is sound rather than pattern-lucky.
#[test]
fn scanner_rules_are_sound() {
    let mut found = Vec::new();
    let src = concat!(
        "#[test]\n",
        "fn owned_return_is_caught() {\n",
        "    if x { return; }\n",
        "}\n",
        "#[test]\n",
        "fn closure_return_is_not_the_tests_own() {\n",
        "    let f = |v: u8| { return v; };\n",
        "    assert_eq!(f(1), 1);\n",
        "}\n",
        "#[test]\n",
        "fn nested_fn_return_is_not_the_tests_own() {\n",
        "    fn helper() -> u8 { return 3; }\n",
        "    assert_eq!(helper(), 3);\n",
        "}\n",
        "#[test]\n",
        "fn commented_and_quoted_returns_are_not_code() {\n",
        "    // return;\n",
        "    let s = \"return;\";\n",
        "    assert_eq!(s.len(), 7);\n",
        "}\n",
        "#[test]\n",
        "fn value_return_is_caught() -> Result<(), String> {\n",
        "    return Ok(());\n",
        "}\n",
        "fn not_a_test() {\n",
        "    return;\n",
        "}\n",
    );
    scan_source(Path::new("synthetic.rs"), src, &mut found);
    let names: BTreeSet<&str> = found.iter().map(|o| o.test.as_str()).collect();
    assert!(
        names.contains("owned_return_is_caught"),
        "an owned early return must be caught: {found:?}"
    );
    assert!(
        names.contains("value_return_is_caught"),
        "a value-carrying return must be caught: {found:?}"
    );
    assert_eq!(
        found
            .iter()
            .filter(|o| o.test == "value_return_is_caught")
            .count(),
        2,
        "the non-unit return type and the `return Ok(())` are both offences: {found:?}"
    );
    for benign in [
        "closure_return_is_not_the_tests_own",
        "nested_fn_return_is_not_the_tests_own",
        "commented_and_quoted_returns_are_not_code",
        "not_a_test",
    ] {
        assert!(
            !names.contains(benign),
            "{benign} must not be reported: {found:?}"
        );
    }
}

#[test]
fn scanner_is_not_fooled_by_the_known_evasions() {
    let mut found = Vec::new();
    scan_source(
        Path::new("synthetic.rs"),
        concat!(
            "macro_rules! skip { () => { return; } }\n",
            "#[test]\n",
            "fn hidden_behind_a_macro() {\n",
            "    skip!();\n",
            "    assert!(false);\n",
            "}\n",
        ),
        &mut found,
    );
    assert!(
        found.iter().any(|o| o.test == "macro_rules!"),
        "a macro expanding to `return` must be reported: {found:?}"
    );

    let mut found = Vec::new();
    scan_source(
        Path::new("synthetic.rs"),
        concat!(
            "#[test]\n",
            "fn bitwise_or_before_a_block() {\n",
            "    let _ = true | { return; false };\n",
            "    assert!(false);\n",
            "}\n",
        ),
        &mut found,
    );
    assert!(
        found.iter().any(|o| o.test == "bitwise_or_before_a_block"),
        "a block after a bitwise `|` must stay in the scanned body: {found:?}"
    );

    let mut found = Vec::new();
    scan_source(
        Path::new("synthetic.rs"),
        concat!(
            "#[test]\n",
            "fn ééééééééééééééé() {\n",
            "    return;\n",
            "    { assert!(false); }\n",
            "}\n",
        ),
        &mut found,
    );
    assert_eq!(
        found.len(),
        1,
        "a multi-byte test name must not shift the scan off the return: {found:?}"
    );

    let mut found = Vec::new();
    scan_source(
        Path::new("synthetic.rs"),
        concat!(
            "#[test]\n",
            "fn chained_bitwise_or_before_a_block() {\n",
            "    let _ = true | false | { return; false };\n",
            "    assert!(false);\n",
            "}\n",
        ),
        &mut found,
    );
    assert!(
        found
            .iter()
            .any(|o| o.test == "chained_bitwise_or_before_a_block"),
        "a chain of bitwise ORs is not a closure parameter list: {found:?}"
    );

    for definition in [
        "macro_rules! skip ( () => (return) );\n",
        "macro_rules! skip [ () => (return) ];\n",
    ] {
        let mut found = Vec::new();
        scan_source(
            Path::new("synthetic.rs"),
            &format!(
                "{definition}#[test]\nfn uses_it() {{\n    skip!();\n    assert!(false);\n}}\n"
            ),
            &mut found,
        );
        assert!(
            found.iter().any(|o| o.test == "macro_rules!"),
            "a {definition:?}-delimited macro expanding to `return` must be reported: {found:?}"
        );
    }

    for attr in ["#[test ]", "#[ test]", "#[ test ]"] {
        let mut found = Vec::new();
        scan_source(
            Path::new("synthetic.rs"),
            &format!("{attr}\nfn spaced_attribute() {{\n    return;\n    assert!(false);\n}}\n"),
            &mut found,
        );
        assert!(
            found.iter().any(|o| o.test == "spaced_attribute"),
            "{attr} must be recognised as a test attribute: {found:?}"
        );
    }

    let mut found = Vec::new();
    scan_source(
        Path::new("synthetic.rs"),
        concat!(
            "#[test]\n",
            "fn block_valued_bitwise_or() {\n",
            "    let _ = { 1u8 } | 2u8 | { return; 0u8 };\n",
            "    assert!(false);\n",
            "}\n",
        ),
        &mut found,
    );
    assert!(
        found.iter().any(|o| o.test == "block_valued_bitwise_or"),
        "a block-valued bitwise operand is not a closure: {found:?}"
    );

    let mut found = Vec::new();
    scan_source(
        Path::new("synthetic.rs"),
        concat!(
            "#[test]\n",
            "fn logical_or_before_a_block() {\n",
            "    let _ = false || { return; false };\n",
            "    assert!(false);\n",
            "}\n",
        ),
        &mut found,
    );
    assert!(
        found.iter().any(|o| o.test == "logical_or_before_a_block"),
        "logical OR before a block is not a zero-argument closure: {found:?}"
    );

    let mut found = Vec::new();
    scan_source(
        Path::new("synthetic.rs"),
        "#[test]\nfn\nnewline_after_fn() {\n    return;\n    assert!(false);\n}\n",
        &mut found,
    );
    assert!(
        found.iter().any(|o| o.test == "newline_after_fn"),
        "a newline after `fn` must not hide the test: {found:?}"
    );

    let mut found = Vec::new();
    scan_source(
        Path::new("synthetic.rs"),
        concat!(
            "macro_rules ! skip { () => { return; } }\n",
            "#[test]\n",
            "fn macro_rules_with_space() {\n",
            "    skip!();\n",
            "    assert!(false);\n",
            "}\n",
        ),
        &mut found,
    );
    assert!(
        found.iter().any(|o| o.test == "macro_rules!"),
        "whitespace before the macro_rules bang must not hide a return: {found:?}"
    );

    let mut found = Vec::new();
    scan_source(
        Path::new("synthetic.rs"),
        concat!(
            "macro_rules! make_test {\n",
            "    ($name:ident, $body:block) => {\n",
            "        #[test]\n",
            "        fn $name() $body\n",
            "    };\n",
            "}\n",
        ),
        &mut found,
    );
    assert!(
        found
            .iter()
            .any(|o| o.test == "<unscannable #[test]>"),
        "macro-generated test bodies must be rejected as structurally unscannable: {found:?}"
    );

    let mut found = Vec::new();
    scan_source(
        Path::new("synthetic.rs"),
        concat!(
            "macro_rules! make_test {\n",
            "    ($body:block) => {\n",
            "        #[test]\n",
            "        fn literal_name() { $body }\n",
            "    };\n",
            "}\n",
        ),
        &mut found,
    );
    assert!(
        found
            .iter()
            .any(|o| o.test == "macro-generated #[test]"),
        "a macro that emits a test shell must fail closed: {found:?}"
    );

    let mut found = Vec::new();
    scan_source(
        Path::new("synthetic.rs"),
        concat!(
            "#[test]\n",
            "fn real_closures_are_still_excluded() {\n",
            "    let a = |v: u8| { return v; };\n",
            "    let b = move |v: u8| { return v; };\n",
            "    let c = || { return 1u8; };\n",
            "    let d = vec![1u8].into_iter().map(|v| { return v; }).count();\n",
            "    assert_eq!(a(1) + b(1) + c() + d as u8, 4);\n",
            "}\n",
        ),
        &mut found,
    );
    assert!(
        found.is_empty(),
        "closure bodies must still be excluded: {found:?}"
    );
}

#[test]
fn no_test_in_the_workspace_returns_early() {
    let native = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("replay_interface/native")
        .to_path_buf();
    let portable = native
        .parent()
        .and_then(Path::parent)
        .expect("repository root")
        .join("portable_replay")
        .join("native");

    let mut found = Vec::new();
    scan_tree(&native, &mut found);
    scan_tree(&portable, &mut found);

    if !found.is_empty() {
        let mut report = String::new();
        for offence in &found {
            report.push_str(&format!(
                "\n  {}:{} {} -- {}",
                offence.file.display(),
                offence.line,
                offence.test,
                offence.what
            ));
        }
        panic!(
            "{} test(s) can end without executing their assertions. libtest reports an early \
             return as a pass, so this is a false green, not a skip. Replace the early exit with \
             if/else, or gate the test at build time with `replay_test_gates` so an unmet \
             precondition is reported as `ignored`.{report}",
            found.len()
        );
    }
}
