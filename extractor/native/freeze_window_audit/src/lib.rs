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
/// does. A text pass CANNOT tell a push into reserved capacity from a push that
/// grows -- that is [`observed`], which watches the allocator instead of the
/// source.
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

/// WATCHING THE ALLOCATOR, because reading the source cannot settle every case.
///
/// # The case it exists for
///
/// [`ALLOCATING_OR_BLOCKING`] cannot list `push`. A push into capacity reserved
/// before the freeze is the CORRECT shape for a stopped window -- it is the
/// owner's pre-allocate-before-the-freeze lever, and it is what `inline_detour`
/// itself does -- while a push into exhausted capacity is the exact hang the
/// audit exists to prevent. The two are the same six characters. Banning them
/// would ban the right answer; permitting them leaves the wrong one invisible.
/// Lane KQ's review agreed on that and named this instrument as what closes it;
/// this is that instrument.
///
/// # What it is
///
/// A global allocator, installed in a crate's TEST BINARY only, that counts the
/// allocations made by the calling thread while [`observed`] is running a
/// closure. The proof it gives is EMPIRICAL and complements the source pass
/// rather than replacing it:
///
/// * the source pass says nothing in this window is SPELLED like an allocation,
///   over every path, whether or not a test drives it;
/// * this says the code a window actually runs made ZERO allocations when it
///   ran, whatever it is spelled like.
///
/// Its limit is the mirror of the other's: it only sees the paths a test drives.
/// Neither is a proof alone. Together, a window whose helpers are driven here
/// and whose text is clean has no ordinary way left to hide the defect.
///
/// # Using it
///
/// In the crate under audit:
///
/// ```ignore
/// #[cfg(test)]
/// #[global_allocator]
/// static PROBE: freeze_window_audit::AllocationProbe = freeze_window_audit::AllocationProbe;
///
/// #[test]
/// fn the_frozen_window_allocates_nothing() {
///     let mut scratch = Scratch::reserved_for(&hooks);   // BEFORE the freeze
///     let (_, allocations) = freeze_window_audit::observed(|| {
///         fill(&mut scratch, &hooks)                     // what the window runs
///     });
///     assert_eq!(allocations, 0);
/// }
/// ```
pub struct AllocationProbe;

std::thread_local! {
    /// Per-thread, and `const`-initialised so that arming it cannot itself be
    /// the first allocation on this thread. Per-thread because the test harness
    /// runs tests in parallel and a process-wide counter would be measuring
    /// every other test at the same time.
    static OBSERVED_ALLOCATIONS: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

unsafe impl std::alloc::GlobalAlloc for AllocationProbe {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_one();
        unsafe { std::alloc::System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        // A FREE COUNTS TOO. `HeapFree` takes the same heap lock `HeapAlloc`
        // does, so dropping a `String` inside a stopped window hangs exactly
        // like building one.
        count_one();
        unsafe { std::alloc::System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(
        &self,
        pointer: *mut u8,
        layout: std::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        // THIS IS THE ONE THE TEXT PASS CANNOT SEE: a `push` that grows.
        count_one();
        unsafe { std::alloc::System.realloc(pointer, layout, new_size) }
    }

    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_one();
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
}

fn count_one() {
    // `try_with`, not `with`: during thread teardown the local is gone, and a
    // panic from inside the allocator is not recoverable.
    let _ = OBSERVED_ALLOCATIONS.try_with(|cell| {
        if let Some(count) = cell.get() {
            cell.set(Some(count.saturating_add(1)));
        }
    });
}

/// Run `body` and report how many times it reached the allocator.
///
/// Counts allocations, frees, reallocations and zeroed allocations made by THIS
/// THREAD while `body` runs. Requires [`AllocationProbe`] to be the crate's
/// test-binary global allocator; without it the count is always zero, so a
/// crate using this asserts the probe is installed with
/// [`the_probe_is_installed`].
pub fn observed<T>(body: impl FnOnce() -> T) -> (T, usize) {
    /// Restores the enclosing observation, AND GIVES IT WHAT THIS ONE COUNTED.
    ///
    /// Two things the first spelling got wrong, and both made a zero mean less
    /// than it looked like. It restored the outer count as it found it, so an
    /// allocation made inside a NESTED `observed` was subtracted from the outer
    /// one -- an outer zero could hide an inner allocation entirely. And it
    /// restored nothing at all if `body` panicked, leaving the thread armed and
    /// every later observation on it counting from a stale base. Doing it in
    /// `Drop` fixes the second; adding rather than replacing fixes the first.
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

/// Is [`AllocationProbe`] actually this test binary's global allocator?
///
/// WITHOUT THIS EVERY `assert_eq!(allocations, 0)` IS VACUOUS: a crate that
/// forgets the `#[global_allocator]` attribute gets zero from [`observed`] for
/// every window, however much it allocates, and every proof built on it passes
/// while proving nothing.
pub fn the_probe_is_installed() -> bool {
    let (_, allocations) = observed(|| {
        // Deliberately opaque to the optimiser: a `Vec` that escapes into a
        // value the caller returns cannot be elided.
        let mut grown: Vec<u8> = Vec::new();
        grown.push(1);
        grown.len()
    });
    allocations > 0
}

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
                // AND EVERY FUNCTION THE WINDOW CALLS, TO A FIXED POINT. A
                // roster of helpers would be the same mistake one level up.
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

/// One function the call closure may reach.
struct Callee {
    /// The file it is declared in, so a call resolves module-locally first.
    label: String,
    name: String,
    body: String,
}

/// Does this crate declare a free `fn drop` of its own?
///
/// `impl Drop for T { fn drop(&mut self) }` is not one: it takes `&mut self`
/// and cannot consume a guard. What this looks for is a free function named
/// `drop` taking something by value, which would shadow the prelude's inside
/// its module and make `drop(held)` an arbitrary transfer rather than the
/// statement that ends the window.
fn declares_its_own_drop(sources: &[Source]) -> bool {
    for source in sources {
        let text = normalise(&source.text);
        // `use sink::park as drop;` shadows it without declaring anything
        // (review round 2), and reads exactly like the prelude's at the call.
        if text.contains(" as drop;") || text.contains(" as drop,") || text.contains(" as drop}")
        {
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

/// Does this file declare an inline `mod name { .. }`?
///
/// SAME FILE IS NOT SAME MODULE, and the module-local preference is only sound
/// while the two coincide (review round 1). A file containing
/// `mod nested { fn helper() {} }` while its top level imports
/// `crate::far::helper` has two `helper`s, and preferring the same-file one
/// would suppress the edge that actually exists. When a file declares an inline
/// module the preference is switched OFF for it and every same-named definition
/// is scanned -- the conservative direction, which over-reports rather than
/// missing a window.
///
/// `#[cfg(test)]` modules do not count: `crate_sources` has already cut them
/// out, so an ordinary crate keeps the sharper resolution.
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

/// Every function of every source, for the call closure.
///
/// SAME-NAMED FUNCTIONS ARE NO LONGER MERGED INTO ONE BODY. The first version
/// concatenated them, on the reasoning that a call site names only the name --
/// but Rust resolves an unqualified call MODULE-LOCALLY FIRST, and the merge
/// charged a window with a namesake in a different file that it cannot call.
/// ARC1 is the proof: `a729_attrition_hook.rs` has an infallible
/// `unsafe fn read_exact(..) -> bool` with no allocation in it at all, and
/// `a729_tooltip_hook.rs` has a completely unrelated
/// `fn read_exact(..) -> Result<(), String>`. The attrition installer's frozen
/// window was reported as building the TOOLTIP module's error strings, which it
/// has no way to reach. Resolution now prefers a definition in the calling file
/// and only falls back to other files when that file has none -- which is what
/// the language does.
fn function_bodies(sources: &[Source]) -> Vec<Callee> {
    let mut out: Vec<Callee> = Vec::new();
    for source in sources {
        for (name, body) in functions(&normalise(&source.text)) {
            match out
                .iter_mut()
                .find(|candidate| candidate.name == name && candidate.label == source.label)
            {
                // Two functions of the same name in the SAME file (two `impl`
                // blocks, say): those a call site really cannot tell apart, so
                // they are still scanned together.
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

/// The names this crate DECLARES, for deciding whether `Q::f()` is its own.
///
/// A qualified call names the scope it is resolved in, and a scope this crate
/// does not declare is out of the crate -- which this instrument states up
/// front that it cannot follow. Reading that scope is the difference between
/// `Vec::new()` (the standard library's, and already reported on its own by
/// [`ALLOCATING_OR_BLOCKING`]) and `InstalledHook::install()` (this crate's,
/// and a real edge). Without it, every `Vec::new()` in a stopped window pulled
/// in EVERY function named `new` in the crate.
fn declared_scopes(sources: &[Source]) -> Vec<String> {
    let mut out: Vec<String> = ["Self", "self", "crate", "super"]
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    for source in sources {
        let text = normalise(&source.text);
        // AN IMPORT ALIAS IS A SCOPE TOO. `use crate::inner as Alias;` makes
        // `Alias::run()` a call into this crate under a name no declaration
        // spells, and reading only declarations dropped that edge (review
        // round 1). Every `as <Ident>` is taken, from a plain `use` and from
        // inside a braced group alike, because both produce a usable qualifier.
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
        for keyword in ["struct ", "enum ", "trait ", "mod ", "impl ", "type ", "union "] {
            let mut cursor = 0usize;
            while let Some(at) = text[cursor..].find(keyword) {
                let start = cursor + at;
                cursor = start + keyword.len();
                if start > 0 && is_identifier_character(text.as_bytes()[start - 1] as char) {
                    continue;
                }
                // `impl<T> Thing`, `impl Trait for Thing`: take every
                // identifier of the header, because any of them may be the
                // type a call site qualifies with.
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

/// Which functions `text` (declared in `from`) calls, MODULE-LOCALLY FIRST.
fn resolve_calls(
    bodies: &[Callee],
    from: &str,
    text: &str,
    declared: &[String],
    files_with_inline_modules: &[String],
) -> Vec<usize> {
    let mut out = Vec::new();
    // The module-local preference is only sound where one file is one module.
    let prefer_same_file = !files_with_inline_modules.iter().any(|label| label == from);
    // Names a same-file definition has already answered for.
    let mut named: Vec<&str> = Vec::new();
    if prefer_same_file {
        for (index, callee) in bodies.iter().enumerate() {
            let shape = calls(text, &callee.name, declared);
            // A QUALIFIED CALL NAMES ITS MODULE, AND IT NEED NOT BE THIS ONE.
            // `mod far;` plus a same-file namesake let `far::helper()` resolve
            // to the local `helper` and suppress the real one (review round 2).
            // The preference is for unqualified calls only; a name reached
            // both ways is treated as qualified, which scans both.
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

/// Does `text` call `name`, as this crate's own function?
///
/// TWO THINGS THE FIRST VERSION GOT WRONG, and both inflated every list:
///
/// * `text.contains("<name>(")` is a SUBSTRING test, so `to_string(` was read
///   as a call to a local `fn string`, `relay_targets_convoy_terminal_hook(`
///   as a call to `fn convoy_terminal_hook` -- which walked D90's INSTALLER
///   window into the body of its runtime DETOUR and everything that calls --
///   and `read_exact_bytes(` as a call to `read_exact`. The name must be a
///   whole identifier: the character before it cannot be one.
/// * a QUALIFIER is not part of the name but it does decide the scope.
///   `Vec::new(` is not a call to this crate's `fn new`. A qualified call is an
///   edge only when the crate declares that scope.
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

/// How a window names a callee, which decides whether module-local resolution
/// may be applied to it.
#[derive(Clone, Copy, Default)]
struct CallShape {
    /// `helper()` -- resolved module-locally by the language.
    unqualified: bool,
    /// `far::helper()` -- the qualifier names the module, and it need not be
    /// this one.
    qualified: bool,
}

impl CallShape {
    fn any(self) -> bool {
        self.unqualified || self.qualified
    }
}

/// The first `drop(<name>)` at the same brace depth the window opened at.
///
/// A drop nested inside a `match` arm or an `if` block belongs to that path
/// only; it does not end the window for the statements after the block.
fn depth_zero_drop(text: &str, name: &str, prelude_drop: bool) -> Option<usize> {
    // A CRATE THAT SHADOWS `drop` HAS NO STATEMENT THAT ENDS A WINDOW HERE.
    //
    // Withdrawing the exemption in `moves_the_guard` alone was not enough
    // (review round 2): a top-level `drop(held)` TRUNCATED the window before
    // the escape question was ever asked, so `fn drop(g: ProcessFreeze) {
    // PARKED.set(g) }` followed by `drop(held); format!("still frozen")` was
    // reported as nothing at all. When the spelling is not the prelude's, the
    // window runs to the end of the function -- the conservative direction.
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

/// Cut out everything between a NESTED `drop(<name>)` and the end of the block
/// it is in: those statements run after the guard is gone on that path.
fn without_text_after_a_nested_drop(text: &str, name: &str, prelude_drop: bool) -> String {
    // Nothing is cut for a crate that shadows `drop`, for the reason in
    // [`depth_zero_drop`]: that call is not known to end anything.
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

/// Does this spelling of the guard's name actually MOVE it out of the window?
///
/// Two shapes matched the escape spellings without moving anything, and they
/// are the two shapes an early-returning installer is written in, so between
/// them they reported an escape for almost every real window:
///
/// * `drop(held)` matches `held)`. That is the opposite of an escape: it is the
///   statement that ENDS the window. It is only reported at all when it sits
///   inside a nested block, where it closes the window for that path only.
/// * `any_instruction_pointer_in(&held, ..)` matches `held,`, and
///   `patch_sites_are_clear(&held, ..)` matches `held,` too. Passing the guard
///   BY REFERENCE is how the freeze is asked its own questions; the guard is
///   still owned by, and still dropped in, this function.
///
/// A third shape does the same: `if let Ok(held) = held {` matches `Ok(held)`,
/// but a DESTRUCTURING PATTERN unwraps the guard into a binding of the same
/// function -- it is how a freeze that may have been refused is used at all.
/// It is told from the constructor `Ok(held)` by what follows: a pattern is
/// followed by `=`, a construction is not.
///
/// So a borrow is not a move, the window's own `drop` is not a move, and a
/// pattern is not a move. Everything else that consumes the binding by value
/// still is -- including handing it to some OTHER function by value, which is
/// what the spellings ending in `)` are for.
fn moves_the_guard(text: &str, spelling: &str, name: &str, prelude_drop: bool) -> bool {
    let within = spelling.find(name).unwrap_or(0);
    let mut cursor = 0usize;
    while let Some(at) = text[cursor..].find(spelling) {
        let start = cursor + at;
        cursor = start + 1;
        // Where the guard's own name sits in this match, which is what the
        // character before it has to be read against.
        let bound = start + within;
        let preceding = &text[..bound];
        if preceding.ends_with('&') {
            continue;
        }
        // `drop(held)` ends the window rather than exporting the guard -- BUT
        // ONLY WHEN IT IS THE PRELUDE'S `drop`. A crate that declares its own
        // free `fn drop(guard: ProcessFreeze)` could retain or forget the
        // guard under exactly that spelling, so the exemption is withdrawn for
        // any crate that declares one (review round 1).
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

/// Is the guard's name here a `let`/`if let`/`while let` PATTERN, not a value?
///
/// Two things have to hold, and both are needed: what FOLLOWS the pattern is a
/// single `=` (a construction is never followed by one), and what PRECEDES the
/// name -- once any enclosing pattern constructors such as `Ok(` or
/// `Some(Ok(` are stripped -- is the `let` keyword.
fn is_a_binding_pattern(text: &str, name_start: usize, match_end: usize) -> bool {
    let after = text[match_end..].trim_start();
    if !after.starts_with('=') || after.starts_with("==") || after.starts_with("=>") {
        return false;
    }
    let mut preceding = text[..name_start].trim_end();
    // `if let Ok(held)` and `let Some(Ok(held))` alike: peel `Ident(`.
    loop {
        let Some(without_paren) = preceding.strip_suffix('(') else {
            break;
        };
        let peeled = without_paren.trim_end_matches(is_identifier_character);
        if peeled.len() == without_paren.len() {
            // A bare `(`, not a constructor: this is a tuple or a call.
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

/// The held-freeze windows inside one function body.
fn windows_in(body: &str, acquisitions: &[&str], prelude_drop: bool) -> Vec<Window> {
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
            let end = depth_zero_drop(tail, &name, prelude_drop).unwrap_or(tail.len());
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
                .any(|spelling| moves_the_guard(searched, spelling, &name, prelude_drop));
            out.push(Window {
                text: without_text_after_a_nested_drop(&tail[..end], &name, prelude_drop),
                escapes,
            });
        }
    }
    out
}

/// The whole ratchet, as ONE implementation instead of one copy per crate.
///
/// Every crate that takes a freeze asserts the same three things -- that the
/// scan actually reached the crate, that the finding COUNT is what a human last
/// agreed to, and that the finding SET is that same set -- and each copy of
/// that assertion was a place the wording could drift. This is it, once.
///
/// # Reading the list without breaking the pin
///
/// Set `FREEZE_WINDOW_AUDIT_DUMP_DIR` to a directory and every crate that runs
/// this writes `<crate>.txt` there: the current count, the current fingerprint,
/// and every finding. That is how the lists in
/// `extractor/native/runs/kz_freeze_audit_ratchets/` were produced, and it does
/// not weaken the assertion -- the dump is written and then the pin is checked
/// exactly as it would have been.
/// The crate's name, taken from the directory that holds its `src`.
///
/// DERIVED RATHER THAN PASSED, and that is not merely tidier. The first version
/// took the name as a `&str` literal at each call site, and that literal is a
/// snake_case name inside a producer crate -- which
/// `compact_contract`'s diagnostic-unit sweep reads as a CONSTRUCTED UNIT NAME
/// and demands be classified as a per-frame quantity or a running total. It is
/// neither; it is a test label. Deriving it means there is no literal to
/// misread, and no second place for the name to drift from the path.
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
    // THE DIRECTORY IS THE FILE LIST. A hand-written one cannot see a module
    // added later, and an unscanned module is an unscanned window.
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
            format!("{crate_name}\ncount {}\nfingerprint {actual}\n\n{listed}\n", findings.len()),
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

    /// A CALLEE NAME IS A WHOLE IDENTIFIER, not a substring of a longer one.
    ///
    /// This is what walked D90's installer window into the body of its runtime
    /// detour: `relay_targets_convoy_terminal_hook(` contains
    /// `convoy_terminal_hook(`, so the closure charged the installer with
    /// everything the detour does. What production change would make this
    /// fail? Dropping the identifier-boundary check in `calls`.
    #[test]
    fn a_longer_name_ending_in_a_helpers_name_is_not_a_call_to_it() {
        let findings = one(
            "fn patch() { let held = freeze_process().unwrap(); \
             relay_targets_hook(); drop(held); } \
             fn relay_targets_hook() { } \
             fn hook() { let m = format!(\"x\"); }",
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    /// ...and the helper is still reached when it really is called.
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

    /// `Vec::new()` is not a call to this crate's `fn new`.
    ///
    /// D75 had five findings from this alone: a window containing `Vec::new()`
    /// pulled in every `fn new` the crate declares. The construct itself is
    /// still reported -- from `ALLOCATING_OR_BLOCKING`, once, where it is.
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

    /// ...but a qualifier the crate DECLARES still resolves into it.
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

    /// A namesake in ANOTHER file is not what an unqualified call resolves to.
    ///
    /// ARC1's proof: its attrition installer's infallible `read_exact` was
    /// charged with the tooltip module's unrelated `read_exact` error strings.
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

    /// ...and a name the calling file does NOT declare still falls back.
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

    /// AN IMPORT ALIAS IS A SCOPE THIS CRATE CAN BE CALLED THROUGH.
    ///
    /// `declared_scopes` decides a qualified call is out-of-crate when it does
    /// not recognise the qualifier, and `use crate::inner as Alias;` gives a
    /// real intra-crate edge a name no declaration spells (review round 1).
    #[test]
    fn a_call_through_an_import_alias_is_still_a_call_into_this_crate() {
        let findings = one(
            "use crate::inner as Alias; \
             fn build() { let m = format!(\"x\"); } \
             fn patch() { let held = freeze_process().unwrap(); \
             Alias::build(); drop(held); }",
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

    /// SAME FILE IS NOT SAME MODULE. A file that declares an inline `mod`
    /// gives up the module-local preference, because the same-file definition
    /// may be the one in the nested module and the call may resolve to an
    /// import from elsewhere (review round 1). The conservative direction is
    /// to scan both.
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

    /// A CRATE THAT DECLARES ITS OWN FREE `fn drop` LOSES THE EXEMPTION.
    ///
    /// `drop(held)` is read as the statement that ends the window because it is
    /// the prelude's. A crate that shadows it could retain or forget the guard
    /// under exactly that spelling (review round 1), so the exemption is
    /// withdrawn and the transfer is reported.
    #[test]
    fn a_crate_that_shadows_drop_no_longer_gets_the_drop_exemption() {
        let findings = one(
            "fn drop(guard: ProcessFreeze) { PARKED.set(guard); } \
             fn patch() { let held = freeze_process().unwrap(); \
             if bad { drop(held); return; } }",
        );
        assert!(
            findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    /// ...AND A SHADOWED `drop` DOES NOT END THE WINDOW EITHER.
    ///
    /// Withdrawing the exemption in `moves_the_guard` alone left this reported
    /// as nothing at all (review round 2): a TOP-LEVEL `drop(held)` truncated
    /// the window before the escape question was asked, so both the transfer
    /// and the `format!` after it were outside everything the scan looked at.
    #[test]
    fn a_shadowed_drop_at_the_top_level_does_not_end_the_window() {
        let findings = one(
            "fn drop(g: ProcessFreeze) { PARKED.set(g); } \
             fn patch() { let held = freeze_process().unwrap(); \
             drop(held); let m = format!(\"still frozen\"); }",
        );
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

    /// An ALIASED import shadows `drop` without declaring anything.
    #[test]
    fn an_alias_named_drop_is_a_shadow_too() {
        let findings = one(
            "use sink::park as drop; \
             fn patch() { let held = freeze_process().unwrap(); \
             drop(held); let m = format!(\"still frozen\"); }",
        );
        assert!(
            findings.iter().any(|finding| matches!(
                finding,
                Finding::AllocatesInWindow { construct, .. } if *construct == "format!"
            )),
            "{findings:?}"
        );
    }

    /// A QUALIFIED CALL NAMES ITS MODULE, AND IT NEED NOT BE THIS FILE'S.
    ///
    /// `mod far;` is not an inline module, so the file keeps the module-local
    /// preference -- and `far::helper()` resolved to the same-file `helper`,
    /// suppressing the real one (review round 2). The preference is for
    /// unqualified calls only.
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

    /// ...and an UNQUALIFIED call still takes it, so the ARC1 case that
    /// motivated the preference stays closed.
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

    /// ...and `impl Drop for T { fn drop(&mut self) }` is not a shadow, so an
    /// ordinary crate keeps it.
    #[test]
    fn an_ordinary_drop_impl_does_not_withdraw_the_exemption() {
        let findings = one(
            "struct T; impl Drop for T { fn drop(&mut self) { } } \
             fn patch() { let held = freeze_process().unwrap(); \
             if bad { drop(held); return; } drop(held); }",
        );
        assert!(
            !findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    /// A BORROW OF THE GUARD IS NOT AN ESCAPE. Asking the freeze its own
    /// questions -- which is the whole point of holding it -- passes `&held`,
    /// and `&held,` matched the `held,` escape spelling.
    #[test]
    fn passing_the_guard_by_reference_is_not_an_escape() {
        let findings = one(
            "fn patch() { let held = freeze_process().unwrap(); \
             let inside = any_instruction_pointer_in(&held, &ranges); drop(held); }",
        );
        assert!(
            !findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    /// AND `drop(held)` IS NOT AN ESCAPE EITHER -- it is the statement that
    /// ends the window. A nested one, on an early-return path, matched `held)`.
    #[test]
    fn an_early_return_arms_own_drop_is_not_an_escape() {
        let findings = one(
            "fn patch() { let held = freeze_process().unwrap(); \
             if bad { drop(held); return; } drop(held); }",
        );
        assert!(
            !findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    /// AND UNWRAPPING A FREEZE THAT MAY HAVE BEEN REFUSED IS NOT AN ESCAPE.
    /// `if let Ok(held) = held` matched the `Ok(held)` escape spelling, but the
    /// guard lands in a binding of this same function.
    #[test]
    fn destructuring_the_guard_out_of_a_result_is_not_an_escape() {
        let findings = one(
            "fn patch() { let held = freeze_process(); \
             if let Ok(held) = held { drop(held); } }",
        );
        assert!(
            !findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    /// ...while RETURNING it in an `Ok` -- the same three characters, not
    /// followed by `=` -- still is.
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

    /// ...while handing the guard to some OTHER function by value still is.
    #[test]
    fn handing_the_guard_to_another_function_by_value_is_still_an_escape() {
        let findings = one(
            "fn patch() { let held = freeze_process().unwrap(); \
             park(held); }",
        );
        assert!(
            findings
                .iter()
                .any(|finding| matches!(finding, Finding::WindowEscapes { .. })),
            "{findings:?}"
        );
    }

    /// THE ALLOCATOR PROBE SEES WHAT THE TEXT CANNOT: a push that grows.
    ///
    /// This crate does not install the probe as its own global allocator (it
    /// has no frozen windows of its own to prove), so what is asserted here is
    /// the arithmetic of `observed` -- that it counts on the calling thread,
    /// restores the previous state, and reports zero when no probe is
    /// installed. The crates that DO install it assert
    /// `the_probe_is_installed()` first, which is what makes their zeroes mean
    /// something.
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
        // Nesting restores rather than clearing...
        let ((), outer) = observed(|| {
            let ((), _inner) = observed(|| {});
            OBSERVED_ALLOCATIONS.with(|cell| {
                assert_eq!(cell.get(), Some(0), "the outer observation is still armed");
            });
        });
        assert_eq!(outer, 0);
        OBSERVED_ALLOCATIONS.with(|cell| assert_eq!(cell.get(), None, "disarmed again"));
    }

    /// AN INNER OBSERVATION'S ALLOCATIONS REACH THE OUTER ONE.
    ///
    /// Without this an outer zero is worth nothing: a window whose helper
    /// happens to run its own `observed` would have its allocations subtracted
    /// from the enclosing measurement instead of added to it.
    ///
    /// The counter is driven directly rather than through the allocator,
    /// because this crate deliberately does not install the probe on itself.
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

    /// A PANIC INSIDE THE BODY STILL DISARMS THE THREAD.
    ///
    /// A failing allocation proof is exactly when this matters: leaving the
    /// thread armed would make every later test on it count from a stale base,
    /// so one real failure would produce a cascade of meaningless ones.
    #[test]
    fn a_panicking_body_does_not_leave_the_thread_armed() {
        let panicked = std::panic::catch_unwind(|| {
            observed(|| panic!("the body under measurement failed"));
        });
        assert!(panicked.is_err(), "the panic must still propagate");
        OBSERVED_ALLOCATIONS.with(|cell| {
            assert_eq!(cell.get(), None, "the thread is still armed after a panic")
        });
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
