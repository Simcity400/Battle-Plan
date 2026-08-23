//! there is nobody left to release it. `inline_detour` enforces that on itself

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

pub struct AllocationProbe;

std::thread_local! {
    static OBSERVED_ALLOCATIONS: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

unsafe impl std::alloc::GlobalAlloc for AllocationProbe {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_one();
        unsafe { std::alloc::System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        count_one();
        unsafe { std::alloc::System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(
        &self,
        pointer: *mut u8,
        layout: std::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        count_one();
        unsafe { std::alloc::System.realloc(pointer, layout, new_size) }
    }

    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_one();
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
}

fn count_one() {
    let _ = OBSERVED_ALLOCATIONS.try_with(|cell| {
        if let Some(count) = cell.get() {
            cell.set(Some(count.saturating_add(1)));
        }
    });
}

pub fn observed<T>(body: impl FnOnce() -> T) -> (T, usize) {
    struct Nesting(Option<usize>);

    impl Drop for Nesting {
        fn drop(&mut self) {
            let counted = OBSERVED_ALLOCATIONS
                .with(|cell| cell.replace(None))
                .unwrap_or_default();
            OBSERVED_ALLOCATIONS
                .with(|cell| cell.set(self.0.map(|outer| outer.saturating_add(counted))));
        }
    }

    let nesting = Nesting(OBSERVED_ALLOCATIONS.with(|cell| cell.replace(Some(0))));
    let value = body();
    let count = OBSERVED_ALLOCATIONS
        .with(|cell| cell.get())
        .unwrap_or_default();
    drop(nesting);
    (value, count)
}

pub fn the_probe_is_installed() -> bool {
    let (_, allocations) = observed(|| {
        let mut grown: Vec<u8> = Vec::new();
        grown.push(1);
        grown.len()
    });
    allocations > 0
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    AllocatesInWindow {
        file: String,
        function: String,
        via: Vec<String>,
        construct: &'static str,
    },
    WindowEscapes {
        file: String,
        function: String,
    },
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

pub struct Source {
    pub label: String,
    pub text: String,
}

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
                text: without_test_modules(&text.replace("\r\n", "\n")),
            });
        }
    }
    out.sort_by(|left, right| left.label.cmp(&right.label));
    out
}

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

pub fn audit(sources: &[Source], acquisitions: &[&str]) -> Vec<Finding> {
    let mut findings = Vec::new();
    let bodies = function_bodies(sources);
    let declared = declared_scopes(sources);
    let prelude_drop = !declares_its_own_drop(sources);
    let files_with_inline_modules: Vec<String> = sources
        .iter()
        .filter(|source| declares_an_inline_module(&source.text))
        .map(|source| source.label.clone())
        .collect();
    for source in sources {
        let text = normalise(&source.text);
        for (function, body) in functions(&text) {
            for window in windows_in(&body, acquisitions, prelude_drop) {
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
                let mut reached: Vec<usize> = Vec::new();
                let mut frontier = vec![(source.label.clone(), window.text.clone())];
                while let Some((from, current)) = frontier.pop() {
                    for index in resolve_calls(
                        &bodies,
                        &from,
                        &current,
                        &declared,
                        &files_with_inline_modules,
                    ) {
                        if reached.contains(&index) {
                            continue;
                        }
                        reached.push(index);
                        frontier.push((bodies[index].label.clone(), bodies[index].body.clone()));
                    }
                }
                for index in &reached {
                    let callee = &bodies[*index];
                    for construct in ALLOCATING_OR_BLOCKING {
                        if callee.body.contains(construct) {
                            findings.push(Finding::AllocatesInWindow {
                                file: source.label.clone(),
                                function: function.clone(),
                                via: vec![callee.name.clone()],
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
        // A char literal such as `'"'` or `'{'` must neither toggle string
        // state nor count as a brace: skip `'x'` and `'\x'` / `'\u{..}'`
        // forms whole. A lifetime or label (`'a` with no closing quote inside
        // the literal span) is left alone.
        if current == '\'' {
            if let Some(len) = char_literal_len(&characters[index..]) {
                out.push(' ');
                index += len;
                continue;
            }
        }
        out.push(current);
        index += 1;
    }
    out
}

/// Length in chars of the char literal starting at `chars[0] == '\''`, or
/// `None` when the quote does not open one (a lifetime or a loop label).
fn char_literal_len(chars: &[char]) -> Option<usize> {
    match chars.get(1)? {
        '\\' => chars
            .iter()
            .enumerate()
            .skip(3)
            .take(12)
            .find(|(_, c)| **c == '\'')
            .map(|(i, _)| i + 1),
        _ => (chars.get(2)? == &'\'').then_some(3),
    }
}

fn functions(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut cursor = 0usize;
    while let Some(at) = text[cursor..].find("fn ") {
        let start = cursor + at;
        cursor = start + 3;
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
        // `open` is a byte offset into `text`; `bytes` is indexed by char, so a
        // non-ASCII character earlier in the file (a BOM, an em dash in code)
        // must not skew the brace walk.
        let open = text[..open].chars().count();
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
        // `open`/`index` are char indices into `bytes`; slicing `text` by them
        // would mix byte and char offsets again.
        out.push((name, bytes[open..index].iter().collect::<String>()));
    }
    out
}

struct Callee {
    label: String,
    name: String,
    body: String,
}

fn declares_its_own_drop(sources: &[Source]) -> bool {
    for source in sources {
        let text = normalise(&source.text);
        if text.contains(" as drop;") || text.contains(" as drop,") || text.contains(" as drop}") {
            return true;
        }
        let mut cursor = 0usize;
        while let Some(at) = text[cursor..].find("fn drop(") {
            let start = cursor + at;
            cursor = start + 8;
            if start > 0 && is_identifier_character(text.as_bytes()[start - 1] as char) {
                continue;
            }
            let parameters = text[cursor..].trim_start();
            if !parameters.starts_with("&mut self") && !parameters.starts_with("&self") {
                return true;
            }
        }
    }
    false
}

fn declares_an_inline_module(text: &str) -> bool {
    let text = normalise(text);
    let mut cursor = 0usize;
    while let Some(at) = text[cursor..].find("mod ") {
        let start = cursor + at;
        cursor = start + 4;
        if start > 0 && is_identifier_character(text.as_bytes()[start - 1] as char) {
            continue;
        }
        let rest = &text[cursor..];
        let after_name = rest.trim_start_matches(|c| is_identifier_character(c) || c == ' ');
        if after_name.starts_with('{') {
            return true;
        }
    }
    false
}

fn function_bodies(sources: &[Source]) -> Vec<Callee> {
    let mut out: Vec<Callee> = Vec::new();
    for source in sources {
        for (name, body) in functions(&normalise(&source.text)) {
            match out
                .iter_mut()
                .find(|candidate| candidate.name == name && candidate.label == source.label)
            {
                Some(existing) => {
                    existing.body.push('\n');
                    existing.body.push_str(&body);
                }
                None => out.push(Callee {
                    label: source.label.clone(),
                    name,
                    body,
                }),
            }
        }
    }
    out
}

fn declared_scopes(sources: &[Source]) -> Vec<String> {
    let mut out: Vec<String> = ["Self", "self", "crate", "super"]
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    for source in sources {
        let text = normalise(&source.text);
        let mut cursor = 0usize;
        while let Some(at) = text[cursor..].find(" as ") {
            let start = cursor + at;
            cursor = start + 4;
            let alias: String = text[cursor..]
                .chars()
                .skip_while(|c| *c == ' ')
                .take_while(|c| is_identifier_character(*c))
                .collect();
            if alias.is_empty() || out.iter().any(|seen| *seen == alias) {
                continue;
            }
            out.push(alias);
        }
        for keyword in [
            "struct ", "enum ", "trait ", "mod ", "impl ", "type ", "union ",
        ] {
            let mut cursor = 0usize;
            while let Some(at) = text[cursor..].find(keyword) {
                let start = cursor + at;
                cursor = start + keyword.len();
                if start > 0 && is_identifier_character(text.as_bytes()[start - 1] as char) {
                    continue;
                }
                let header: String = text[cursor..]
                    .chars()
                    .take_while(|c| *c != '{' && *c != ';' && *c != '\n')
                    .collect();
                for word in header.split(|c: char| !is_identifier_character(c)) {
                    if word.is_empty() || word == "for" || word == "where" || word == "dyn" {
                        continue;
                    }
                    if !out.iter().any(|seen| seen == word) {
                        out.push(word.to_string());
                    }
                }
            }
        }
    }
    out
}

fn is_identifier_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn resolve_calls(
    bodies: &[Callee],
    from: &str,
    text: &str,
    declared: &[String],
    files_with_inline_modules: &[String],
) -> Vec<usize> {
    let mut out = Vec::new();
    let prefer_same_file = !files_with_inline_modules.iter().any(|label| label == from);
    let mut named: Vec<&str> = Vec::new();
    if prefer_same_file {
        for (index, callee) in bodies.iter().enumerate() {
            let shape = calls(text, &callee.name, declared);
            if callee.label != from || !shape.unqualified || shape.qualified {
                continue;
            }
            named.push(&callee.name);
            out.push(index);
        }
    }
    for (index, callee) in bodies.iter().enumerate() {
        if out.contains(&index)
            || named.iter().any(|seen| *seen == callee.name)
            || !calls(text, &callee.name, declared).any()
        {
            continue;
        }
        out.push(index);
    }
    out
}

fn calls(text: &str, name: &str, declared: &[String]) -> CallShape {
    let mut shape = CallShape::default();
    for spelling in [format!("{name}("), format!("{name}::<")] {
        let mut cursor = 0usize;
        while let Some(at) = text[cursor..].find(&spelling) {
            let start = cursor + at;
            cursor = start + 1;
            let before = text[..start].as_bytes();
            if before
                .last()
                .is_some_and(|byte| is_identifier_character(*byte as char))
            {
                continue;
            }
            if !text[..start].ends_with("::") {
                shape.unqualified = true;
                continue;
            }
            let qualifier: String = text[..start - 2]
                .chars()
                .rev()
                .take_while(|c| is_identifier_character(*c))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if qualifier.is_empty() || declared.iter().any(|scope| *scope == qualifier) {
                shape.qualified = true;
            }
        }
    }
    shape
}

#[derive(Clone, Copy, Default)]
struct CallShape {
    unqualified: bool,
    qualified: bool,
}

impl CallShape {
    fn any(self) -> bool {
        self.unqualified || self.qualified
    }
}

fn depth_zero_drop(text: &str, name: &str, prelude_drop: bool) -> Option<usize> {
    if name.is_empty() || !prelude_drop {
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

fn without_text_after_a_nested_drop(text: &str, name: &str, prelude_drop: bool) -> String {
    if name.is_empty() || !prelude_drop {
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

fn moves_the_guard(text: &str, spelling: &str, name: &str, prelude_drop: bool) -> bool {
    let within = spelling.find(name).unwrap_or(0);
    let mut cursor = 0usize;
    while let Some(at) = text[cursor..].find(spelling) {
        let start = cursor + at;
        cursor = start + 1;
        let bound = start + within;
        let preceding = &text[..bound];
        if preceding.ends_with('&') {
            continue;
        }
        if prelude_drop && preceding.ends_with("drop(") {
            continue;
        }
        if is_a_binding_pattern(text, bound, start + spelling.len()) {
            continue;
        }
        return true;
    }
    false
}

fn is_a_binding_pattern(text: &str, name_start: usize, match_end: usize) -> bool {
    let after = text[match_end..].trim_start();
    if !after.starts_with('=') || after.starts_with("==") || after.starts_with("=>") {
        return false;
    }
    let mut preceding = text[..name_start].trim_end();
    loop {
        let Some(without_paren) = preceding.strip_suffix('(') else {
            break;
        };
        let peeled = without_paren.trim_end_matches(is_identifier_character);
        if peeled.len() == without_paren.len() {
            preceding = without_paren.trim_end();
            continue;
        }
        preceding = peeled.trim_end();
    }
    preceding.ends_with("let")
}

struct Window {
    text: String,
    escapes: bool,
}

fn windows_in(body: &str, acquisitions: &[&str], prelude_drop: bool) -> Vec<Window> {
    let mut out = Vec::new();
    for acquisition in acquisitions {
        let mut cursor = 0usize;
        while let Some(at) = body[cursor..].find(acquisition) {
            let start = cursor + at;
            cursor = start + acquisition.len();
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
            let end = depth_zero_drop(tail, &name, prelude_drop).unwrap_or(tail.len());
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
                .any(|spelling| moves_the_guard(searched, spelling, &name, prelude_drop));
            out.push(Window {
                text: without_text_after_a_nested_drop(&tail[..end], &name, prelude_drop),
                escapes,
            });
        }
    }
    out
}

fn crate_name_of(src_dir: &std::path::Path) -> String {
    src_dir
        .parent()
        .and_then(|directory| directory.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| src_dir.display().to_string())
}

pub fn assert_ratchet(
    src_dir: &std::path::Path,
    acquisitions: &[&str],
    open_findings: usize,
    pinned_fingerprint: u64,
) {
    let crate_name = crate_name_of(src_dir);
    let crate_name = crate_name.as_str();
    let sources = crate_sources(src_dir);
    assert!(
        sources.len() >= 2,
        "{crate_name}: the scan found only {} source file(s); it is not \
         reaching the crate",
        sources.len()
    );
    let findings = audit(&sources, acquisitions);
    let listed = findings
        .iter()
        .map(|finding| format!("  {finding}"))
        .collect::<Vec<_>>()
        .join("\n");
    let actual = fingerprint(&findings);
    if let Some(directory) = std::env::var_os("FREEZE_WINDOW_AUDIT_DUMP_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).expect("a writable dump directory");
        std::fs::write(
            directory.join(format!("{crate_name}.txt")),
            format!(
                "{crate_name}\ncount {}\nfingerprint {actual}\n\n{listed}\n",
                findings.len()
            ),
        )
        .expect("a writable dump file");
    }
    assert_eq!(
        (findings.len(), actual),
        (open_findings, pinned_fingerprint),
        "{crate_name}: the held-freeze windows of this crate changed. If you \
         FIXED one, lower the pinned count and pin the printed fingerprint. If \
         this GREW, the change put new work inside a window where the whole \
         game is stopped:\n{listed}"
    );
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

    #[test]
    fn a_bom_or_non_ascii_code_before_the_window_does_not_skew_the_brace_walk() {
        // kills: lane PV landing - functions() found `{` by byte offset but
        // walked a char vector, so one leading BOM (3 bytes, 1 char) shifted
        // every brace and the walk underflowed instead of auditing the window.
        let source = format!(
            "{bom}const NOTE: &str = \"\"; // {dash} prose\nfn patch() {{ let held = freeze_process().unwrap(); \
             let ranges: Vec<_> = sites.iter().collect(); drop(held); }}",
            bom = '\u{feff}',
            dash = '\u{2014}'
        );
        let findings = one(&source);
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "collect"
            )),
            "{findings:?}"
        );
    }

    #[test]
    fn a_quote_char_literal_before_the_window_does_not_hide_the_allocation() {
        // kills: normalise() treated the `"` inside `'"'` as a string opener,
        // inverting string state for the rest of the file and hiding real
        // allocations inside a held freeze window.
        let findings = one(
            "fn escape(c: char) -> bool { c == '\"' } \
             fn patch() { let held = freeze_process().unwrap(); \
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

    #[test]
    fn an_allocation_between_the_freeze_and_its_drop_is_found() {
        let findings = one("fn patch() { let held = freeze_process().unwrap(); \
             let ranges: Vec<_> = sites.iter().collect(); drop(held); }");
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "collect"
            )),
            "{findings:?}"
        );
    }

    #[test]
    fn an_allocation_after_the_drop_is_not_a_finding() {
        let findings = one(
            "fn patch() { let held = freeze_process().unwrap(); drop(held); \
             let ranges: Vec<_> = sites.iter().collect(); }",
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

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

    #[test]
    fn a_drop_inside_an_early_return_arm_does_not_end_the_window() {
        let findings = one("fn patch() { let held = freeze_process().unwrap(); \
             if bad { drop(held); return Err(format!(\"clean\")); } \
             let scratch = vec![0u8; 8]; drop(held); }");
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "vec!["
            )),
            "{findings:?}"
        );
        assert!(
            !findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "format!"
            )),
            "{findings:?}"
        );
    }

    #[test]
    fn a_window_that_escapes_the_function_is_a_finding() {
        let findings = one("fn patch() -> Set { let held = freeze_process().unwrap(); \
             Set { suspend: Some(held) } }");
        assert!(
            findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    #[test]
    fn an_escape_before_the_drop_is_reported_even_when_a_later_drop_exists() {
        let findings = one("fn patch() { let held = freeze_process().unwrap(); \
             if bad { return Ok(held); } drop(held); }");
        assert!(
            findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

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
        assert!(
            !findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "format!"
            )),
            "{findings:?}"
        );
    }

    #[test]
    fn a_longer_name_ending_in_a_helpers_name_is_not_a_call_to_it() {
        let findings = one("fn patch() { let held = freeze_process().unwrap(); \
             relay_targets_hook(); drop(held); } \
             fn relay_targets_hook() { } \
             fn hook() { let m = format!(\"x\"); }");
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn the_same_helper_is_still_reached_when_it_is_actually_called() {
        let findings = one(
            "fn patch() { let held = freeze_process().unwrap(); hook(); drop(held); } \
             fn hook() { let m = format!(\"x\"); }",
        );
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { via, .. } if via == &vec!["hook".to_string()]
            )),
            "{findings:?}"
        );
    }

    #[test]
    fn a_foreign_qualifier_is_not_a_call_into_this_crate() {
        let findings = one(
            "struct Own; impl Own { fn new() { let m = format!(\"x\"); } } \
             fn patch() { let held = freeze_process().unwrap(); \
             let v = Vec::new(); drop(held); }",
        );
        assert!(
            !findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { via, .. } if via == &vec!["new".to_string()]
            )),
            "{findings:?}"
        );
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { via, construct, .. }
                    if via.is_empty() && *construct == "Vec::new"
            )),
            "{findings:?}"
        );
    }

    #[test]
    fn a_qualifier_this_crate_declares_is_still_a_call_into_it() {
        let findings = one(
            "struct Own; impl Own { fn build() { let m = format!(\"x\"); } } \
             fn patch() { let held = freeze_process().unwrap(); \
             Own::build(); drop(held); }",
        );
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { via, construct, .. }
                    if via == &vec!["build".to_string()] && *construct == "format!"
            )),
            "{findings:?}"
        );
    }

    #[test]
    fn a_namesake_in_another_file_is_not_the_function_that_was_called() {
        let sources = [
            Source {
                label: "near.rs".into(),
                text: "fn patch() { let held = freeze_process().unwrap(); \
                       read_exact(); drop(held); } fn read_exact() { }"
                    .into(),
            },
            Source {
                label: "far.rs".into(),
                text: "fn read_exact() { let m = format!(\"x\"); }".into(),
            },
        ];
        let findings = audit(&sources, &["freeze_process()"]);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_helper_declared_only_in_another_file_is_still_reached() {
        let sources = [
            Source {
                label: "near.rs".into(),
                text: "fn patch() { let held = freeze_process().unwrap(); \
                       helper(); drop(held); }"
                    .into(),
            },
            Source {
                label: "far.rs".into(),
                text: "fn helper() { let m = format!(\"x\"); }".into(),
            },
        ];
        let findings = audit(&sources, &["freeze_process()"]);
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { via, .. } if via == &vec!["helper".to_string()]
            )),
            "{findings:?}"
        );
    }

    #[test]
    fn a_call_through_an_import_alias_is_still_a_call_into_this_crate() {
        let findings = one("use crate::inner as Alias; \
             fn build() { let m = format!(\"x\"); } \
             fn patch() { let held = freeze_process().unwrap(); \
             Alias::build(); drop(held); }");
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { via, construct, .. }
                    if via == &vec!["build".to_string()] && *construct == "format!"
            )),
            "{findings:?}"
        );
    }

    #[test]
    fn a_file_with_an_inline_module_scans_every_namesake() {
        let sources = [
            Source {
                label: "near.rs".into(),
                text: "mod nested { fn helper() { } } \
                       fn patch() { let held = freeze_process().unwrap(); \
                       helper(); drop(held); }"
                    .into(),
            },
            Source {
                label: "far.rs".into(),
                text: "fn helper() { let m = format!(\"x\"); }".into(),
            },
        ];
        let findings = audit(&sources, &["freeze_process()"]);
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { via, .. } if via == &vec!["helper".to_string()]
            )),
            "a file with an inline module must not prefer its own namesake: {findings:?}"
        );
    }

    #[test]
    fn a_crate_that_shadows_drop_no_longer_gets_the_drop_exemption() {
        let findings = one("fn drop(guard: ProcessFreeze) { PARKED.set(guard); } \
             fn patch() { let held = freeze_process().unwrap(); \
             if bad { drop(held); return; } }");
        assert!(
            findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    #[test]
    fn a_shadowed_drop_at_the_top_level_does_not_end_the_window() {
        let findings = one("fn drop(g: ProcessFreeze) { PARKED.set(g); } \
             fn patch() { let held = freeze_process().unwrap(); \
             drop(held); let m = format!(\"still frozen\"); }");
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "format!"
            )),
            "the window must run past a `drop` that is not the prelude's: {findings:?}"
        );
        assert!(
            findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    #[test]
    fn an_alias_named_drop_is_a_shadow_too() {
        let findings = one("use sink::park as drop; \
             fn patch() { let held = freeze_process().unwrap(); \
             drop(held); let m = format!(\"still frozen\"); }");
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "format!"
            )),
            "{findings:?}"
        );
    }

    #[test]
    fn a_qualified_call_does_not_take_the_module_local_preference() {
        let sources = [
            Source {
                label: "lib.rs".into(),
                text: "mod far; fn helper() { } \
                       fn patch() { let held = freeze_process().unwrap(); \
                       far::helper(); drop(held); }"
                    .into(),
            },
            Source {
                label: "far.rs".into(),
                text: "fn helper() { let m = format!(\"allocates\"); }".into(),
            },
        ];
        let findings = audit(&sources, &["freeze_process()"]);
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { via, construct, .. }
                    if via == &vec!["helper".to_string()] && *construct == "format!"
            )),
            "the qualified call must reach far.rs: {findings:?}"
        );
    }

    #[test]
    fn an_unqualified_call_still_takes_the_module_local_preference() {
        let sources = [
            Source {
                label: "lib.rs".into(),
                text: "mod far; fn helper() { } \
                       fn patch() { let held = freeze_process().unwrap(); \
                       helper(); drop(held); }"
                    .into(),
            },
            Source {
                label: "far.rs".into(),
                text: "fn helper() { let m = format!(\"allocates\"); }".into(),
            },
        ];
        let findings = audit(&sources, &["freeze_process()"]);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn an_ordinary_drop_impl_does_not_withdraw_the_exemption() {
        let findings = one("struct T; impl Drop for T { fn drop(&mut self) { } } \
             fn patch() { let held = freeze_process().unwrap(); \
             if bad { drop(held); return; } drop(held); }");
        assert!(
            !findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    #[test]
    fn passing_the_guard_by_reference_is_not_an_escape() {
        let findings = one("fn patch() { let held = freeze_process().unwrap(); \
             let inside = any_instruction_pointer_in(&held, &ranges); drop(held); }");
        assert!(
            !findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    #[test]
    fn an_early_return_arms_own_drop_is_not_an_escape() {
        let findings = one("fn patch() { let held = freeze_process().unwrap(); \
             if bad { drop(held); return; } drop(held); }");
        assert!(
            !findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    #[test]
    fn destructuring_the_guard_out_of_a_result_is_not_an_escape() {
        let findings = one("fn patch() { let held = freeze_process(); \
             if let Ok(held) = held { drop(held); } }");
        assert!(
            !findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    #[test]
    fn returning_the_guard_in_an_ok_is_still_an_escape() {
        let findings = one("fn patch() -> R { let held = freeze_process().unwrap(); Ok(held) }");
        assert!(
            findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    #[test]
    fn handing_the_guard_to_another_function_by_value_is_still_an_escape() {
        let findings = one("fn patch() { let held = freeze_process().unwrap(); \
             park(held); }");
        assert!(
            findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    #[test]
    fn observed_reports_zero_without_a_probe_and_restores_the_previous_state() {
        assert!(!the_probe_is_installed());
        let (value, allocations) = observed(|| {
            let mut grown: Vec<u8> = Vec::new();
            grown.push(1);
            grown.len()
        });
        assert_eq!(value, 1);
        assert_eq!(allocations, 0, "no probe is installed in this crate");
        let ((), outer) = observed(|| {
            let ((), _inner) = observed(|| {});
            OBSERVED_ALLOCATIONS.with(|cell| {
                assert_eq!(cell.get(), Some(0), "the outer observation is still armed");
            });
        });
        assert_eq!(outer, 0);
        OBSERVED_ALLOCATIONS.with(|cell| assert_eq!(cell.get(), None, "disarmed again"));
    }

    #[test]
    fn a_nested_observation_hands_its_count_to_the_one_around_it() {
        let ((), outer) = observed(|| {
            let ((), inner) = observed(|| {
                OBSERVED_ALLOCATIONS.with(|cell| cell.set(Some(7)));
            });
            assert_eq!(inner, 7);
        });
        assert_eq!(outer, 7, "the outer observation must not lose the inner 7");
        OBSERVED_ALLOCATIONS.with(|cell| assert_eq!(cell.get(), None, "disarmed again"));
    }

    #[test]
    fn a_panicking_body_does_not_leave_the_thread_armed() {
        let panicked = std::panic::catch_unwind(|| {
            observed(|| panic!("the body under measurement failed"));
        });
        assert!(panicked.is_err(), "the panic must still propagate");
        OBSERVED_ALLOCATIONS
            .with(|cell| assert_eq!(cell.get(), None, "the thread is still armed after a panic"));
    }

    #[test]
    fn a_construct_named_inside_a_string_literal_is_not_a_finding() {
        let findings = one("fn patch() { let held = freeze_process().unwrap(); \
             refuse(\"do not call format! here\"); drop(held); }");
        assert!(findings.is_empty(), "{findings:?}");
    }
}
