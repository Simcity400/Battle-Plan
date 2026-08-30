
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

/// The `|` that closes a closure's parameter list when the body brace carries
/// a RETURN TYPE between the two: `|text: &str| -> Option<String> {`.
///
/// Without this, such a body is not blanked and every `return` in it is
/// attributed to the enclosing test - a false green REPORT, which is the mirror
/// of the false green this file exists to catch.
///
/// Walking left from the brace: a run of bytes a type may contain, then the
/// arrow, then - modulo whitespace - the bar. `>` is itself a type byte, so the
/// run swallows the one in `->` and the arrow is `bytes[i - 1] == b'-'` with
/// `bytes[i] == b'>'`. Where the byte before the arrow is not a bar the walk
/// CONTINUES left past it, so a return type that is itself a function type -
/// `|| -> fn() -> u8 {` - reaches the outer arrow and then the bar.
///
/// Three things it deliberately does NOT match, all in the direction that
/// reports rather than hides:
///
/// - a run whose angle brackets do not balance. The brace of the anonymous
///   const in `|| -> Wrapper<{ 1 }> { .. }` sees `-> Wrapper<` to its left,
///   which would otherwise blank a piece of the TYPE and leave the real body
///   unrecognised.
/// - a type containing `;`, such as `[u8; 1]`. Allowing it would let the walk
///   cross a statement boundary into unrelated text. Such a closure is still
///   REPORTED, which is a false positive and not a false green.
/// - a brace that is a MACRO's delimiter. A macro invocation in type position -
///   `|| -> ty!{ u8 } { .. }` - offers its own `{` first, and that token tree is
///   part of the type.
fn closure_return_type_bar(bytes: &[u8], before_brace: usize, from: usize) -> Option<usize> {
    let type_byte = |b: u8| {
        is_ident_byte(b)
            || b.is_ascii_whitespace()
            || matches!(
                b,
                b'<' | b'>'
                    | b':'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b','
                    | b'['
                    | b']'
                    | b'.'
                    | b'*'
                    | b'+'
                    | b'!'
            )
    };
    if macro_bang_before(bytes, before_brace, from) {
        return None;
    }
    let mut scan_from = before_brace;
    loop {
        let mut i = scan_from;
        while i > from && type_byte(bytes[i - 1]) {
            i -= 1;
        }
        // the arrow, whose `>` the run above already passed
        if i >= scan_from || i < from + 2 || bytes[i] != b'>' || bytes[i - 1] != b'-' {
            return None;
        }
        // The type text runs from just after the arrow to the brace. Its angle
        // brackets must balance, or the `{` is inside the type rather than the
        // body it introduces.
        let mut depth = 0i32;
        for at in i + 1..before_brace {
            match bytes[at] {
                b'<' => depth += 1,
                // the `>` of an inner arrow is not a closing bracket
                b'>' if bytes[at - 1] != b'-' => depth -= 1,
                _ => {}
            }
        }
        if depth != 0 {
            return None;
        }
        let mut bar = i - 1;
        while bar > from && bytes[bar - 1].is_ascii_whitespace() {
            bar -= 1;
        }
        if bar > from && bytes[bar - 1] == b'|' {
            return Some(bar - 1);
        }
        // Not a bar: an inner arrow of a function type. Keep walking left.
        scan_from = i - 1;
        if scan_from <= from {
            return None;
        }
    }
}

/// Whether the token before `at` is a MACRO's `!` rather than a negation: a
/// macro name, or the tail of a path like `foo::bar!`, precedes the bang.
///
/// Neither gap is required to be empty. A macro invocation is a token sequence,
/// so `hoist ! ( .. )` is one, and `strip_noise` has already turned any comment
/// between the tokens into spaces. An earlier version required both to be
/// byte-adjacent, and `hoist /* here */ ! (|x| -> () { return; })` walked past
/// it into a blanked body - a false green.
///
/// A non-ASCII byte where the bang should be is UNSURE - Rust counts U+0085,
/// U+200E, U+200F, U+2028 and U+2029 as whitespace - so it answers true, the
/// direction that reports. The name side needs no such case: `is_ident_byte`
/// already counts every byte at or above 0x80 as part of an identifier.
fn macro_bang_before(bytes: &[u8], at: usize, from: usize) -> bool {
    let mut bang = at;
    while bang > from && bytes[bang - 1].is_ascii_whitespace() {
        bang -= 1;
    }
    if bang == from {
        return false;
    }
    if !bytes[bang - 1].is_ascii() {
        return true;
    }
    if bytes[bang - 1] != b'!' {
        return false;
    }
    let mut name = bang - 1;
    while name > from && bytes[name - 1].is_ascii_whitespace() {
        name -= 1;
    }
    name > from && (is_ident_byte(bytes[name - 1]) || bytes[name - 1] == b':')
}

/// Whether the byte at `open` is inside the token tree of a macro INVOCATION -
/// `name!( .. )`, `name![ .. ]`, `name!{ .. }` - at any nesting depth within
/// `[from, open)`.
///
/// A macro takes its argument as tokens and may expand them anywhere, so a
/// closure-shaped token tree inside one is not a closure and its `return` is
/// not the closure's. Blanking it would be a false GREEN. Where this is unsure
/// it answers true, which reports rather than hides.
///
/// The walk goes left, counting `(`, `[` and `{` alike. Every unmatched opener
/// it meets ENCLOSES `open`, so each one is tested for a macro bang and, when it
/// is not one, the walk continues out through it. Only reaching `from` - the
/// start of the test body, which bounds the walk - answers false. An earlier
/// version stopped at the first unmatched `(`, and treated `{` and `;` as proof
/// that no macro was open; `hoist!((..))` and `hoist! { .. }` both walked
/// straight past it.
fn inside_macro_invocation(bytes: &[u8], open: usize, from: usize) -> bool {
    let mut depth = 0i32;
    let mut i = open;
    while i > from {
        i -= 1;
        match bytes[i] {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' => {
                if depth == 0 {
                    if macro_bang_before(bytes, i, from) {
                        return true;
                    }
                } else {
                    depth -= 1;
                }
            }
            _ => {}
        }
    }
    false
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
            let bar = if j > from && bytes[j - 1] == b'|' {
                Some(j - 1)
            } else {
                // The typed path only. The direct-bar path has blanked
                // macro-argument closures since long before this fix, and
                // narrowing it would report closures the workspace scan passes
                // today - a separate change, not this one.
                //
                // The question is asked of the BRACE about to be blanked, not of
                // the bar. `|| -> ty!(({ return })) { 1 }` has its bar outside
                // the macro and the brace inside it.
                closure_return_type_bar(bytes, j, from)
                    .filter(|_| !inside_macro_invocation(bytes, i, from))
            };
            bar.is_some_and(|bar| closes_closure_params(bytes, bar, from))
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

        // A test written INSIDE a macro invocation is macro TOKENS, and the
        // macro may reorder or hoist them, so nothing in the body can be
        // attributed to a closure or a nested `fn`. The ancestry has to be read
        // here, against the whole file: the body is copied out below, and the
        // opener is outside the copy.
        let wrapped_in_macro = inside_macro_invocation(text.as_bytes(), brace, 0);
        let mut body: Vec<u8> = text.as_bytes()[brace..=end].to_vec();
        let len = body.len();
        if !wrapped_in_macro {
            blank_inner_bodies(&mut body, 0, len);
        }
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
        "fn typed_closure_return_is_not_the_tests_own() {\n",
        "    let f = |v: u8| -> u8 { return v; };\n",
        "    assert_eq!(f(1), 1);\n",
        "}\n",
        "#[test]\n",
        "fn a_function_typed_closure_return_is_not_the_tests_own() {\n",
        "    let f = || -> fn() -> u8 { return helper; };\n",
        "    assert_eq!(f()(), 3);\n",
        "}\n",
        "#[test]\n",
        "fn a_brace_inside_the_return_type_is_not_the_body() {\n",
        "    let f = || -> Wrapper<{ 1 }> { return Wrapper; };\n",
        "    assert!(f().ok());\n",
        "}\n",
        "#[test]\n",
        "fn a_closure_shaped_macro_argument_is_not_a_closure() {\n",
        "    hoist!(|x| -> () { return; });\n",
        "    assert!(false);\n",
        "}\n",
        "#[test]\n",
        "fn an_anonymous_const_hides_nothing() {\n",
        "    let f = || -> Wrapper<{ discard!(return) }> { Wrapper };\n",
        "    assert!(f().ok());\n",
        "}\n",
        "#[test]\n",
        "fn a_nested_macro_argument_is_not_a_closure() {\n",
        "    hoist!((|x| -> () { return; }));\n",
        "    assert!(false);\n",
        "}\n",
        "#[test]\n",
        "fn a_brace_delimited_macro_argument_is_not_a_closure() {\n",
        "    hoist!{ |x| -> () { return; } }\n",
        "    assert!(false);\n",
        "}\n",
        "#[test]\n",
        "fn a_spaced_macro_bang_is_still_a_macro() {\n",
        "    hoist /* here */ ! (|x| -> () { return; });\n",
        "    assert!(false);\n",
        "}\n",
        "#[test]\n",
        "fn a_nested_brace_in_a_type_macro_is_not_the_body() {\n",
        "    let f = || -> ty!(({ return })) { 1 };\n",
        "    assert_eq!(f(), 1);\n",
        "}\n",
        "#[test]\n",
        "fn a_spaced_type_macro_is_not_the_body() {\n",
        "    let f = || -> ty /* here */ ! { discard!(return) } { 1 };\n",
        "    assert_eq!(f(), 1);\n",
        "}\n",
        "#[test]\n",
        "fn a_brace_delimited_type_macro_is_not_the_body() {\n",
        "    let f = || -> ty!{ discard!(return) } { 1 };\n",
        "    assert_eq!(f(), 1);\n",
        "}\n",
        "#[test]\n",
        "fn a_negated_closure_is_still_a_closure() {\n",
        "    let ok = !(|| -> bool { return false; })();\n",
        "    assert!(ok);\n",
        "}\n",
        "#[test]\n",
        "fn a_typed_closure_does_not_hide_the_tests_own_return() {\n",
        "    let f = |v: u8| -> u8 { return v; };\n",
        "    if f(1) == 1 { return; }\n",
        "    assert!(false);\n",
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
        "passthrough! {\n",
        "#[test]\n",
        "fn a_test_inside_a_macro_invocation_is_macro_tokens() {\n",
        "    let f = || -> u8 { return 1; };\n",
        "    assert_eq!(f(), 1);\n",
        "}\n",
        "}\n",
    );
    scan_source(Path::new("synthetic.rs"), src, &mut found);
    let names: BTreeSet<&str> = found.iter().map(|o| o.test.as_str()).collect();
    assert!(
        names.contains("owned_return_is_caught"),
        "an owned early return must be caught: {found:?}"
    );
    assert!(
        names.contains("a_typed_closure_does_not_hide_the_tests_own_return"),
        "blanking a typed closure must not swallow the test's own return: {found:?}"
    );
    assert!(
        names.contains("a_brace_inside_the_return_type_is_not_the_body"),
        "a closure whose return type carries a brace must not have its own return \
         swallowed: {found:?}"
    );
    assert!(
        names.contains("an_anonymous_const_hides_nothing"),
        "the anonymous const in a return type is TYPE text, and a `return` token \
         inside it must be reported rather than blanked away: {found:?}"
    );
    assert!(
        names.contains("a_nested_macro_argument_is_not_a_closure"),
        "an extra pair of parens must not carry a closure-shaped token tree out of \
         its macro: {found:?}"
    );
    assert!(
        names.contains("a_brace_delimited_macro_argument_is_not_a_closure"),
        "a brace-delimited macro invocation is a macro invocation like any \
         other: {found:?}"
    );
    assert!(
        names.contains("a_spaced_macro_bang_is_still_a_macro"),
        "a macro invocation is a token sequence, so whitespace - or a comment, \
         which `strip_noise` leaves as whitespace - between the name, the bang \
         and the delimiter changes nothing: {found:?}"
    );
    assert!(
        names.contains("a_brace_delimited_type_macro_is_not_the_body"),
        "the token tree of a macro in TYPE position is part of the type, not the \
         body it precedes: {found:?}"
    );
    assert!(
        names.contains("a_spaced_type_macro_is_not_the_body"),
        "the same holds for a macro in type position: {found:?}"
    );
    assert!(
        names.contains("a_test_inside_a_macro_invocation_is_macro_tokens"),
        "a test written inside a macro invocation is macro TOKENS - the body is \
         copied out before it is blanked, so the ancestry is read against the \
         whole file or not at all: {found:?}"
    );
    assert!(
        names.contains("a_nested_brace_in_a_type_macro_is_not_the_body"),
        "a macro's token tree is not a body at ANY depth, and the brace about to \
         be blanked is the thing to ask about - not the bar, which can sit \
         outside the macro the brace is inside: {found:?}"
    );
    assert!(
        names.contains("a_closure_shaped_macro_argument_is_not_a_closure"),
        "a macro takes its argument as TOKENS and may expand them anywhere, so a \
         closure-shaped one is not a closure and its return must not be blanked: {found:?}"
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
        "typed_closure_return_is_not_the_tests_own",
        "a_function_typed_closure_return_is_not_the_tests_own",
        "a_negated_closure_is_still_a_closure",
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
