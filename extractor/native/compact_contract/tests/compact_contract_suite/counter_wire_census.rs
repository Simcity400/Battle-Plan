//! The census that keeps [`REGISTRY`] honest.
//!
//! A diagnostic counter is only an accounted gap if a reader can find its value
//! in the capture. These tests read the sources rather than importing a
//! generated table: a table the producer also builds from would agree with the
//! producer by construction and prove nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compact_contract::counter_wire_registry::{PRODUCERS, REGISTRY};
use compact_contract::diagnostic_channel::{declares_explicit_unit_accumulation, Accumulation};

/// Domains whose DOMAIN_PERF record carries NO diagnostic trailer.
///
/// Every entry is a domain the collector emits with the plain
/// `append_compact_domain_perf_record`, so nothing it counts can reach the
/// wire on that channel. The list is asserted equal to the tree, so a new
/// trailer-free domain cannot be added without saying so here.
const DOMAINS_WITH_NO_DIAGNOSTIC_TRAILER: &[&str] = &[
    "COMPACT_DOMAIN_CHARACTER_PRESENTATION_LIVE",
    "COMPACT_DOMAIN_POWER_BALANCE_STATE_LIVE",
    "COMPACT_DOMAIN_NATIONAL_FOCUS_PRESENTATION_LIVE",
    "COMPACT_DOMAIN_SESSION_HEADER_LIVE",
    "COMPACT_DOMAIN_POV_PROVINCE_VISIBILITY_LIVE",
    "COMPACT_DOMAIN_AIR_WINGS",
    "COMPACT_DOMAIN_AIR_REGION_ACTIVITY",
    "COMPACT_DOMAIN_AIR_VARIANT_DEFS",
    "COMPACT_DOMAIN_AIR_VARIANT_STATS",
    "COMPACT_DOMAIN_AIR_VARIANT_ARCHETYPES",
    "COMPACT_DOMAIN_AIR_VARIANT_RESERVES",
    "COMPACT_DOMAIN_AIR_BASE_STATUS",
    "COMPACT_DOMAIN_STOCKPILE_VARIANT_DEFS",
    "COMPACT_DOMAIN_AIR_VARIANT_UPGRADES",
    "COMPACT_DOMAIN_STOCKPILE_VARIANT_UPGRADES",
    "COMPACT_DOMAIN_OCCUPATION_RESISTANCE_COMPLIANCE_LIVE",
    "COMPACT_DOMAIN_MAP_DETAIL",
    "COMPACT_DOMAIN_MAP_CONTROL",
];

const COLLECTOR_EMISSION_FILE: &str = "collector/src/attach_log.rs";

/// The collector crate root, so a census can read every PRODUCTION file the
/// binary compiles rather than the one file the emissions live in today.
const COLLECTOR_CRATE_ROOT: &str = "collector/src/lib.rs";

/// Directories under `extractor/native` that hold no producer source: build
/// output and evidence. Everything else is scanned, so a producer cannot escape
/// the registry by living in a crate nobody thought to list.
const NOT_PRODUCER_DIRS: &[&str] = &["target", "runs", "fixtures", ".git"];

/// Names of things that count. An exemption may not carry one: a list meant for
/// addresses and id allocators cannot be used to excuse a counter.
const COUNTER_SHAPED: &[&str] = &[
    "count",
    "counts",
    "calls",
    "events",
    "rows",
    "records",
    "drops",
    "dropped",
    "skipped",
    "refused",
    "rejected",
    "invalid",
    "missing",
    "unresolved",
    "excluded",
    "omitted",
    "unreadable",
    "overflow",
    "overflows",
    "mismatch",
    "mismatches",
    "duplicate",
    "duplicates",
    "failed",
    "failures",
    "truncated",
    "blocked",
    "degraded",
    "retries",
    "abandoned",
    "evicted",
    "total",
    "seen",
    "observed",
    "emitted",
    "admitted",
    "lost",
    "poisoned",
    "anomaly",
    "anomalies",
    "calls",
    "sequence",
    "sequences",
    "generation",
    "generations",
    "epoch",
    "epochs",
    "hits",
    "misses",
    "attempts",
];

/// Field types a counter can have on a source struct. Anything else is not a
/// number a reader could fold, so it is not a counter.
const COUNTER_TYPES: &[&str] = &["bool", "u8", "u16", "u32", "u64", "usize"];

/// Types a value can ACCUMULATE in, whatever it is called.
///
/// The declaration backstop is name-blind over these: nothing in a registered
/// lane's source uses one of them for an address, so every declaration of one
/// must say whether it counts.
const ACCUMULATOR_SCALARS: &[&str] = &[
    "u8",
    "u16",
    "u32",
    "u64",
    "AtomicU8",
    "AtomicU16",
    "AtomicU32",
    "AtomicU64",
];

/// The pointer-width types, which these files use for ADDRESSES far more often
/// than for counts. Name-tested rather than name-blind: holding every `usize`
/// in a hook file to the registry would be an exemption list of pointers.
const ADDRESS_WIDTH_SCALARS: &[&str] = &["usize", "AtomicUsize"];

fn native_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compact_contract sits inside extractor/native")
        .to_path_buf()
}

/// Read with carriage returns stripped: a worktree checks out LF and the gate
/// checks out CRLF, and every scan below is line-shaped.
fn read(relative: &str) -> String {
    let path = native_root().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        .replace('\r', "")
}

/// A source file and every module reachable from it by `mod name;`.
///
/// A counter declared in a submodule beside the hook is as reachable as one in
/// the hook -- `mod accounting;` and a `static DROPPED_ROWS` inside it is the
/// same counter to the domain -- so the backstop reads the closure, not the one
/// file the registry names. `#[path]` is honoured; this repository uses it.
///
/// It stops at the CRATE, not at the domain's own reach: a lane crate can hold
/// another domain's producer, and holding d78's registry to d85's counters
/// would say the wrong thing about both. What the crate declares outside this
/// closure is named by `crate_modules_outside_the_domain` and checked by
/// `every_module_of_a_registered_crate_is_scanned_or_named`, so a sibling module
/// cannot be added quietly either.
fn source_closure(relative: &str) -> Vec<(String, String)> {
    closure_of(relative, false)
}

/// The same closure with `#[cfg(test)]` modules left out: the files that are
/// COMPILED INTO THE SHIPPING COLLECTOR. A test fixture that writes a
/// DOMAIN_PERF record by hand is a fixture, not an emission.
fn production_closure(relative: &str) -> Vec<(String, String)> {
    closure_of(relative, true)
}

fn closure_of(relative: &str, production_only: bool) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // The crate ROOT is read too, and read as a leaf: a counter declared in
    // `lib.rs` is the lane's own, while a module the root declares beside this
    // domain's may be ANOTHER domain's producer -- one lane crate holds two --
    // and holding this registry to that one's counters would say the wrong
    // thing about both. What the root declares and this closure does not reach
    // is named by `crate_modules_outside_the_domain`.
    // The followed seed goes LAST so the LIFO pop reaches it first: when the
    // registry names the crate root itself, the leaf entry below is the same
    // path and must not be the one that wins.
    let mut queue = Vec::new();
    if let Some((crate_dir, _)) = relative.rsplit_once("/src/") {
        let root = format!("{crate_dir}/src/lib.rs");
        if root != relative {
            queue.push((root, false));
        }
    }
    queue.push((relative.to_string(), true));
    let mut seen: BTreeSet<String> = BTreeSet::new();
    while let Some((relative, follow)) = queue.pop() {
        if !seen.insert(relative.clone()) {
            continue;
        }
        let path = native_root().join(&relative);
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let text = text.replace('\r', "");
        // The directory a `mod name;` in THIS file resolves against: the file's
        // own directory for `lib.rs`/`mod.rs`, and `<stem>/` otherwise.
        let (directory, stem) = relative
            .rsplit_once('/')
            .map(|(head, file)| (head.to_string(), file.trim_end_matches(".rs").to_string()))
            .unwrap_or_else(|| (String::new(), relative.trim_end_matches(".rs").to_string()));
        let base = match stem.as_str() {
            "lib" | "main" | "mod" => directory.clone(),
            stem => match directory.is_empty() {
                true => stem.to_string(),
                false => format!("{directory}/{stem}"),
            },
        };
        // `#[path = "..."]` overrides the convention, and this repository uses
        // it. It is carried forward to the next `mod name;`, which is where the
        // attribute sits.
        let mut path_attribute: Option<String> = None;
        let mut cfg_test = false;
        if !follow {
            out.push((relative, text));
            continue;
        }
        // `mod outer { mod inner; }` resolves `inner` under `outer/`, so the
        // inline modules a `mod name;` sits inside are part of its path.
        let mut inline: Vec<(usize, String)> = Vec::new();
        let mut depth = 0usize;
        for line in without_comments_or_strings(&text).lines() {
            let trimmed = line.trim();
            let opening = without_visibility(without_attributes(trimmed))
                .strip_prefix("mod ")
                .and_then(|rest| rest.split_once('{'))
                .map(|(name, _)| name.trim().to_string())
                .filter(|name| {
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                });
            let mut opened = false;
            for byte in trimmed.bytes() {
                match byte {
                    b'{' => {
                        depth += 1;
                        if !opened {
                            if let Some(name) = opening.clone() {
                                inline.push((depth, name));
                            }
                            opened = true;
                        }
                    }
                    b'}' => {
                        while inline.last().is_some_and(|(at, _)| *at >= depth) {
                            inline.pop();
                        }
                        depth = depth.saturating_sub(1);
                    }
                    _ => {}
                }
            }
            let base = match inline.is_empty() {
                true => base.clone(),
                false => {
                    let nested: Vec<&str> =
                        inline.iter().map(|(_, name)| name.as_str()).collect();
                    match base.is_empty() {
                        true => nested.join("/"),
                        false => format!("{base}/{}", nested.join("/")),
                    }
                }
            };
            if let Some(rest) = trimmed.strip_prefix("#[path") {
                path_attribute = rest
                    .split_once('"')
                    .and_then(|(_, tail)| tail.split_once('"'))
                    .map(|(value, _)| value.to_string());
                continue;
            }
            if trimmed.starts_with("#[cfg(test)]") {
                cfg_test = true;
            }
            let trimmed = without_visibility(without_attributes(trimmed));
            let Some(rest) = trimmed.strip_prefix("mod ") else {
                if !trimmed.is_empty() {
                    path_attribute = None;
                    cfg_test = false;
                }
                continue;
            };
            let Some(name) = rest.strip_suffix(';') else {
                // `mod name {` is inline: its body is already in this file.
                path_attribute = None;
                cfg_test = false;
                continue;
            };
            let name = name.trim();
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                path_attribute = None;
                cfg_test = false;
                continue;
            }
            let mut candidates = Vec::new();
            if let Some(value) = path_attribute.take() {
                // Both bases, because which one Rust uses depends on whether the
                // declaring file is a crate root; taking either is safe, because
                // over-reaching only shows this scan MORE source.
                for base in [directory.as_str(), base.as_str()] {
                    candidates.push(match base.is_empty() {
                        true => value.clone(),
                        false => format!("{base}/{value}"),
                    });
                }
            } else {
                let head = match base.is_empty() {
                    true => name.to_string(),
                    false => format!("{base}/{name}"),
                };
                candidates.push(format!("{head}.rs"));
                candidates.push(format!("{head}/mod.rs"));
            }
            let gated = production_only && cfg_test;
            cfg_test = false;
            for candidate in candidates {
                let normalised = normalise_relative(&candidate);
                if native_root().join(&normalised).is_file() {
                    if !gated {
                        queue.push((normalised, true));
                    }
                    break;
                }
            }
        }
        out.push((relative, text));
    }
    out.sort();
    out
}

/// `a/b/../c` as `a/c`, so a `#[path]` that climbs out of `src/` is one key.
fn normalise_relative(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// Every `.rs` file under `extractor/native`, build output and evidence aside.
fn producer_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let skip = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| NOT_PRODUCER_DIRS.contains(&name));
                if !skip {
                    walk(&path, out);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&native_root(), &mut files);
    files.sort();
    files
}

/// Does this identifier name something that counts?
fn is_counter_shaped(name: &str) -> bool {
    name.split('_').any(|word| COUNTER_SHAPED.contains(&word))
}

/// A source fragment with line comments removed, layout kept.
fn without_comments_only(text: &str) -> String {
    text.lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A source fragment with comments removed and all whitespace collapsed away,
/// so a pinned expression matches however rustfmt broke it across lines.
fn without_comments_or_layout(text: &str) -> String {
    text.lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
        .split_whitespace()
        .collect()
}

/// The body of `fn <name>(`, which must appear exactly once.
fn fn_body<'a>(text: &'a str, name: &str) -> &'a str {
    let needle = format!("fn {name}");
    let mut found: Option<&str> = None;
    let mut search = 0usize;
    while let Some(at) = text[search..].find(&needle) {
        let start = search + at;
        search = start + needle.len();
        // `fn read_census<M: FogInputMemory>(` is the same declaration as
        // `fn read_census(`, and `fn read_census_of(` is a different one.
        if !matches!(text[search..].chars().next(), Some('(') | Some('<')) {
            continue;
        }
        let Some(brace) = text[start..].find('{') else {
            continue;
        };
        assert!(
            found.is_none(),
            "fn {name} is declared more than once in the scanned file"
        );
        found = Some(balanced(text, start + brace + 1, b'{', b'}'));
    }
    found.unwrap_or_else(|| panic!("fn {name} is not declared in the scanned file"))
}

/// The text between `open` and its matching close, starting just past `from`.
fn balanced(text: &str, from: usize, open: u8, close: u8) -> &str {
    let bytes = text.as_bytes();
    let mut depth = 1usize;
    let mut index = from;
    while index < bytes.len() {
        if bytes[index] == open {
            depth += 1;
        } else if bytes[index] == close {
            depth -= 1;
            if depth == 0 {
                return &text[from..index];
            }
        }
        index += 1;
    }
    panic!("unbalanced {} in a scanned source", open as char);
}

/// The body of `struct <name> { .. }`, which must appear exactly once.
fn struct_body<'a>(text: &'a str, name: &str) -> &'a str {
    let mut found: Option<&str> = None;
    let needle = format!("struct {name} ");
    let mut search = 0usize;
    while let Some(at) = text[search..].find(&needle) {
        let start = search + at;
        search = start + needle.len();
        let Some(brace) = text[start..].find('{') else {
            continue;
        };
        // A declaration, not a use: the line must begin the item.
        let line_start = text[..start].rfind('\n').map(|at| at + 1).unwrap_or(0);
        let head = text[line_start..start].trim();
        if !head.is_empty() && head != "pub" && !head.starts_with("pub(") {
            continue;
        }
        assert!(
            found.is_none(),
            "struct {name} is declared more than once in the scanned file, so a \
             registry row cannot say which one it means"
        );
        found = Some(balanced(text, start + brace + 1, b'{', b'}'));
    }
    found.unwrap_or_else(|| panic!("struct {name} is not declared in the scanned file"))
}

/// Counter-typed fields of a struct body, with their types, in declaration
/// order.
fn counter_fields_with_types(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        let trimmed = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
        let trimmed = match trimmed.find(") ") {
            Some(at) if trimmed.starts_with("pub(") => &trimmed[at + 2..],
            _ => trimmed,
        };
        let Some(rest) = trimmed.strip_suffix(',') else {
            continue;
        };
        let Some((name, ty)) = rest.split_once(':') else {
            continue;
        };
        let (name, ty) = (name.trim(), ty.trim());
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            continue;
        }
        if COUNTER_TYPES.contains(&ty) {
            out.push((name.to_string(), ty.to_string()));
        }
    }
    out
}

/// Counter-typed fields of a struct body, in declaration order.
fn counter_fields(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        let trimmed = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
        let trimmed = match trimmed.find(") ") {
            Some(at) if trimmed.starts_with("pub(") => &trimmed[at + 2..],
            _ => trimmed,
        };
        let Some(rest) = trimmed.strip_suffix(',') else {
            continue;
        };
        let Some((name, ty)) = rest.split_once(':') else {
            continue;
        };
        let (name, ty) = (name.trim(), ty.trim());
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            continue;
        }
        if COUNTER_TYPES.contains(&ty) {
            out.push(name.to_string());
        }
    }
    out
}

/// Comments and string-literal CONTENT blanked, byte for byte, so a brace or a
/// colon inside either is not read as source structure. Whitespace replaces
/// them rather than nothing, so every offset in the result is an offset in the
/// source.
fn without_comments_or_strings(text: &str) -> String {
    let bytes = text.as_bytes();
    // Byte for byte, so every offset in the result is an offset in the source.
    let mut out = vec![b' '; bytes.len()];
    let mut index = 0usize;
    let copy = |out: &mut Vec<u8>, at: usize, count: usize| {
        for offset in at..(at + count).min(bytes.len()) {
            out[offset] = bytes[offset];
        }
    };
    while index < bytes.len() {
        // A raw string, with any number of hashes.
        if bytes[index] == b'r' {
            let mut hashes = 0usize;
            while index + 1 + hashes < bytes.len() && bytes[index + 1 + hashes] == b'#' {
                hashes += 1;
            }
            if index + 1 + hashes < bytes.len() && bytes[index + 1 + hashes] == b'"' {
                let closing = format!("\"{}", "#".repeat(hashes));
                let body_start = index + 2 + hashes;
                let end = text[body_start..]
                    .find(&closing)
                    .map(|at| body_start + at + closing.len())
                    .unwrap_or(bytes.len());
                copy(&mut out, index, 2 + hashes);
                index = end;
                continue;
            }
        }
        // A comment, blanked in the SAME pass as the strings: strip either one
        // first and the other's delimiters inside it desynchronise the scan --
        // a `//` inside a string literal, or a lone quote inside a comment.
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index = text[index + 2..]
                .find("*/")
                .map(|at| index + 2 + at + 2)
                .unwrap_or(bytes.len());
            continue;
        }
        match bytes[index] {
            b'"' => {
                copy(&mut out, index, 1);
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                index = (index + 1).min(bytes.len());
            }
            // A char literal, which can hold a brace. A lifetime (`'a`) has no
            // closing quote and is left as source.
            b'\'' => {
                let escaped = bytes.get(index + 1) == Some(&b'\\');
                let close = if escaped { index + 3 } else { index + 2 };
                if bytes.get(close) == Some(&b'\'') {
                    index = close + 1;
                } else {
                    copy(&mut out, index, 1);
                    index += 1;
                }
            }
            _ => {
                copy(&mut out, index, 1);
                index += 1;
            }
        }
    }
    String::from_utf8(out).expect("blanking replaces whole characters with spaces")
}

/// `#[cfg(test)]` items removed. A fixture is not production, and holding a
/// test's locals to the registry would say a counter reached the wire because a
/// test declared one.
fn without_test_items(text: &str) -> String {
    const ATTRIBUTE: &str = "#[cfg(test)]";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(ATTRIBUTE) {
        out.push_str(&rest[..at]);
        let after = &rest[at + ATTRIBUTE.len()..];
        let Some(brace) = after.find('{') else {
            out.push_str(after);
            return out;
        };
        // A `#[cfg(test)]` item that ends in `;` before it ever opens a brace --
        // a `use`, a `const` -- is not a block. Cutting to the next brace would
        // swallow whatever production item comes after it.
        if after[..brace].contains(';') {
            out.push_str(&after[..brace]);
            rest = &after[brace..];
            continue;
        }
        let body = balanced(after, brace + 1, b'{', b'}');
        let consumed = brace + 1 + body.len() + 1;
        rest = &after[consumed.min(after.len())..];
    }
    out.push_str(rest);
    out
}

/// One counter-capable declaration: the item that holds it, its name, its type.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Declaration {
    /// The `struct`/`enum` the field belongs to, or `""` for a module-level
    /// `static`.
    owner: String,
    name: String,
    ty: String,
}

impl Declaration {
    /// How the exemption list names it.
    fn key(&self) -> String {
        if self.owner.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.owner, self.name)
        }
    }
}

/// The innermost type argument of `ty`: `Cell<u64>` and `[u64; 4]` and
/// `Option<u64>` all accumulate a `u64`, and `BTreeMap<Reason, u64>` is R7's
/// keyed-counter shape.
///
/// The result is the last path segment, so `std::sync::atomic::AtomicU64` and
/// an imported `AtomicU64` are one answer.
fn innermost_type(ty: &str) -> String {
    let ty: String = ty.split_whitespace().collect();
    let ty = ty.trim_start_matches('&').to_string();
    if let Some(inner) = ty.strip_prefix('[') {
        let inner = inner.rsplit_once(';').map(|(head, _)| head).unwrap_or(inner);
        let inner = inner.trim_end_matches(']');
        return innermost_type(inner);
    }
    let Some(open) = ty.find('<') else {
        return ty.rsplit("::").next().unwrap_or(&ty).to_string();
    };
    let Some(close) = ty.rfind('>') else {
        return ty.rsplit("::").next().unwrap_or(&ty).to_string();
    };
    let arguments = &ty[open + 1..close];
    let mut depth = 0usize;
    let mut last = 0usize;
    for (index, byte) in arguments.as_bytes().iter().enumerate() {
        match byte {
            b'<' | b'(' | b'[' => depth += 1,
            b'>' | b')' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => last = index + 1,
            _ => {}
        }
    }
    innermost_type(&arguments[last..])
}

/// Every type a value of `ty` could accumulate in.
///
/// One for an ordinary type, and one per ELEMENT for a tuple: a tuple has no
/// privileged position the way a map has its value type, so `(u32, u32)` holds
/// two places a count could live and reading only the last would miss the
/// first. Nesting is followed, so `Option<(u64, Reason)>` reports both.
fn accumulator_leaves(ty: &str) -> Vec<String> {
    let ty: String = ty.split_whitespace().collect();
    let ty = ty.trim_start_matches('&');
    if let Some(inner) = ty.strip_prefix('[') {
        let inner = inner.rsplit_once(';').map(|(head, _)| head).unwrap_or(inner);
        return accumulator_leaves(inner.trim_end_matches(']'));
    }
    // A tuple, and a generic's ARGUMENTS: `Result<u64, ReadError>` holds a count
    // in the first and `BTreeMap<Reason, u64>` in the last, so reading one
    // position would always be reading the wrong one somewhere.
    let arguments = ty
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .or_else(|| match (ty.find('<'), ty.rfind('>')) {
            (Some(open), Some(close)) if close > open => Some(&ty[open + 1..close]),
            _ => None,
        });
    let Some(arguments) = arguments else {
        return vec![ty.rsplit("::").next().unwrap_or(ty).to_string()];
    };
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, byte) in arguments.as_bytes().iter().enumerate() {
        match byte {
            b'<' | b'(' | b'[' => depth += 1,
            b'>' | b')' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                out.extend(accumulator_leaves(&arguments[start..index]));
                start = index + 1;
            }
            _ => {}
        }
    }
    out.extend(accumulator_leaves(&arguments[start..]));
    out
}

/// `text` with leading `#[..]` attributes removed.
///
/// `#[cfg(windows)] mod accounting;` is one line and one item, and a reader
/// that only looked for a line STARTING with `mod` would neither follow it nor
/// refuse it.
fn without_attributes(text: &str) -> &str {
    let mut text = text.trim_start();
    while let Some(rest) = text.strip_prefix("#[").or_else(|| text.strip_prefix("#![")) {
        let mut depth = 1usize;
        let mut end = None;
        for (at, ch) in rest.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(at);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            return text;
        };
        text = rest[end + 1..].trim_start();
    }
    text
}

/// `text` with a leading visibility removed, whatever shape it takes.
fn without_visibility(text: &str) -> &str {
    let text = text.trim_start();
    let Some(rest) = text.strip_prefix("pub") else {
        return text;
    };
    let rest = match rest.strip_prefix('(') {
        // `pub(crate)`, `pub(super)`, `pub(in crate::a::b)`.
        Some(inside) => match inside.find(')') {
            Some(at) => &inside[at + 1..],
            None => return text,
        },
        None => rest,
    };
    match rest.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_') {
        // `public_rows: u64` is not a visibility.
        true => text,
        false => rest.trim_start(),
    }
}

/// `type Dword = u32;` bindings declared in this file.
///
/// A counter behind an alias is the same counter. This reads ONE file; the
/// callers gather the result across the whole module closure, so an alias
/// declared in one module and used in another resolves. `use path::X as Y;` is
/// read here too, because a rename hides a type exactly as `type` does.
fn type_aliases(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    // Statements, not lines: `type DropCount =\n    u64;` is one declaration
    // however rustfmt broke it.
    let collapsed: String = text
        .lines()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ");
    for statement in collapsed.split(';') {
        let statement = format!("{statement};");
        let trimmed = without_visibility(&statement);
        // `use crate::accounting::DropCount as Hidden;` renames a type exactly
        // as `type` does, and hides it from a scan that reads only `type`.
        if let Some(rest) = trimmed.strip_prefix("use ") {
            if let Some((path, alias)) = rest.split_once(" as ") {
                let alias = alias.trim().trim_end_matches(';').trim();
                let target = path.trim().rsplit("::").next().unwrap_or(path).trim();
                if !alias.is_empty()
                    && alias
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                {
                    out.insert(alias.to_string(), target.to_string());
                }
            }
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("type ") else {
            continue;
        };
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let Some(value) = value.trim().strip_suffix(';') else {
            continue;
        };
        if !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            out.insert(name.to_string(), value.split_whitespace().collect());
        }
    }
    out
}

fn resolve_alias(aliases: &BTreeMap<String, String>, leaf: &str) -> String {
    let mut leaf = leaf.to_string();
    // Bounded: an alias chain longer than this is a cycle, and a cycle does not
    // compile, so stopping is not a gap.
    for _ in 0..8 {
        let Some(next) = aliases.get(&leaf) else {
            return leaf;
        };
        let next = innermost_type(next);
        if next == leaf {
            return leaf;
        }
        leaf = next;
    }
    leaf
}

/// Can this declaration hold a count?
fn accumulates(aliases: &BTreeMap<String, String>, name: &str, ty: &str) -> bool {
    accumulator_leaves(ty).into_iter().any(|leaf| {
        let leaf = resolve_alias(aliases, &leaf);
        ACCUMULATOR_SCALARS.contains(&leaf.as_str())
            || (ADDRESS_WIDTH_SCALARS.contains(&leaf.as_str()) && is_counter_shaped(name))
    })
}

/// `head` with the visibility markers and `#[..]` attributes that can stand
/// between a field and its opening `{` or `,` removed, so the opener is what
/// the field position is read from.
fn before_the_declaration(head: &str) -> &str {
    let mut head = head.trim_end();
    loop {
        if let Some(rest) = head.strip_suffix("pub") {
            head = rest.trim_end();
            continue;
        }
        // `pub(crate)`, `pub(super)`, `pub(in crate::a::b)` -- every restricted
        // visibility ends in `)`, and the one before a declaration is the only
        // `)` a `pub` can close.
        if head.ends_with(')') {
            let mut depth = 0usize;
            let mut open = None;
            for (at, ch) in head.char_indices().rev() {
                match ch {
                    ')' => depth += 1,
                    '(' => {
                        depth -= 1;
                        if depth == 0 {
                            open = Some(at);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(open) = open {
                if let Some(rest) = head[..open].trim_end().strip_suffix("pub") {
                    head = rest.trim_end();
                    continue;
                }
            }
        }
        if !head.ends_with(']') {
            return head;
        }
        let mut depth = 0usize;
        let mut open = None;
        for (at, ch) in head.char_indices().rev() {
            match ch {
                ']' => depth += 1,
                '[' => {
                    depth -= 1;
                    if depth == 0 {
                        open = Some(at);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(open) = open else {
            return head;
        };
        let before = head[..open].trim_end();
        // `#[..]` and `#![..]` are attributes; `slot[index]` is not, and a
        // declaration does not follow one.
        let Some(before) = before
            .strip_suffix('#')
            .or_else(|| before.strip_suffix("#!"))
        else {
            return head;
        };
        head = before.trim_end();
    }
}

/// The elements of a tuple declaration, as `owner.0`, `owner.1`, ...
///
/// `struct Tally(u64);` and `enum Gap { Dropped(u64) }` hold a count in a
/// position that has no name, so the `name: Type` grammar cannot see them. The
/// position IS the name here, which is also how the code refers to it.
fn tuple_declarations(
    aliases: &BTreeMap<String, String>,
    owner: &str,
    elements: &str,
) -> Vec<Declaration> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let push = |out: &mut Vec<Declaration>, position: usize, element: &str| {
        let ty: String = without_visibility(element).split_whitespace().collect();
        if ty.is_empty() {
            return;
        }
        let name = position.to_string();
        if accumulates(aliases, &name, &ty) {
            out.push(Declaration {
                owner: owner.to_string(),
                name,
                ty,
            });
        }
    };
    let mut position = 0usize;
    for (index, byte) in elements.as_bytes().iter().enumerate() {
        match byte {
            b'<' | b'(' | b'[' => depth += 1,
            b'>' | b')' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                push(&mut out, position, &elements[start..index]);
                position += 1;
                start = index + 1;
            }
            _ => {}
        }
    }
    push(&mut out, position, &elements[start..]);
    out
}

/// Every counter-capable DECLARATION in a source file: struct and enum-variant
/// fields, tuple positions, and `static`s including thread-locals.
///
/// This is the whole grammar the backstop reads. A declaration is `name: Type`
/// or a tuple position, which is why the family is finite where the ways of
/// incrementing one are not.
fn counter_typed_declarations(text: &str) -> Vec<Declaration> {
    counter_typed_declarations_with(text, &BTreeMap::new())
}

/// The same, with type aliases the rest of the closure declares: `type
/// DropCount = u64;` in one module and `dropped: DropCount` in another is one
/// counter written across two files.
fn counter_typed_declarations_with(
    text: &str,
    imported: &BTreeMap<String, String>,
) -> Vec<Declaration> {
    let text = without_test_items(&without_comments_or_strings(text));
    let mut aliases = type_aliases(&text);
    for (name, value) in imported {
        aliases.entry(name.clone()).or_insert_with(|| value.clone());
    }
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0usize;
    // `(depth the item's body opens at, item name, the item is an enum)`.
    let mut items: Vec<(usize, String, bool)> = Vec::new();
    let mut pending: Option<(String, bool)> = None;
    let mut index = 0usize;
    let owner_of = |items: &Vec<(usize, String, bool)>| -> String {
        items
            .last()
            .map(|(_, name, _)| name.clone())
            .unwrap_or_default()
    };
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b'(' | b'[' => {
                depth += 1;
                index += 1;
                continue;
            }
            b')' | b']' => {
                depth = depth.saturating_sub(1);
                index += 1;
                continue;
            }
            b'{' => {
                depth += 1;
                if let Some((name, is_enum)) = pending.take() {
                    items.push((depth, name, is_enum));
                }
                index += 1;
                continue;
            }
            b'}' => {
                while items.last().is_some_and(|(at, _, _)| *at >= depth) {
                    items.pop();
                }
                depth = depth.saturating_sub(1);
                index += 1;
                continue;
            }
            b';' => {
                pending = None;
                index += 1;
                continue;
            }
            _ => {}
        }
        if !(byte.is_ascii_alphabetic() || byte == b'_') {
            index += 1;
            continue;
        }
        let word_end = index
            + text[index..]
                .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .unwrap_or(text.len() - index);
        let word = &text[index..word_end];
        let after = text[word_end..].trim_start();
        match word {
            "struct" | "enum" | "union" | "trait" | "impl" | "fn" | "mod" => {
                // The item's own name, so a field reports the struct it is in.
                let name: String = after
                    .chars()
                    .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                    .collect();
                if !name.is_empty() {
                    pending = Some((name.clone(), word == "enum"));
                }
                // `struct Tally(u64);` -- a body with no field names at all.
                if word == "struct" && !name.is_empty() {
                    let tail = after[name.len()..].trim_start();
                    let tail = match tail.starts_with('<') {
                        true => &tail[balanced(tail, 1, b'<', b'>').len() + 2..],
                        false => tail,
                    };
                    if tail.starts_with('(') {
                        out.extend(tuple_declarations(
                            &aliases,
                            &name,
                            balanced(tail, 1, b'(', b')'),
                        ));
                    }
                }
                index = word_end;
                continue;
            }
            "static" | "const" => {
                // A `const` cannot accumulate -- its value is fixed at compile
                // time -- so only `static` is a declaration a counter could
                // live in. `static mut` is one too.
                let after = after.strip_prefix("mut").map(str::trim_start).unwrap_or(after);
                let name: String = after
                    .chars()
                    .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                    .collect();
                let tail = after[name.len()..].trim_start();
                if word == "static" && !name.is_empty() {
                    if let Some(tail) = tail.strip_prefix(':') {
                        let ty = read_type(tail, &['=', ';']);
                        if accumulates(&aliases, &name.to_ascii_lowercase(), &ty) {
                            out.push(Declaration {
                                owner: String::new(),
                                name,
                                ty,
                            });
                        }
                    }
                }
                index = word_end;
                continue;
            }
            "let" => {
                // A local is not a declaration that outlives the function; the
                // residual this backstop does not reach.
                index = word_end;
                let Some(semicolon) = text[index..].find(';') else {
                    break;
                };
                index += semicolon;
                continue;
            }
            _ => {}
        }
        // A field: `name: Type`, opened by the item's brace or by the comma
        // after the previous field. A function parameter looks the same and is
        // told apart by the delimiter it sits inside.
        let head = before_the_declaration(&text[..index]);
        let previous = head.chars().next_back();
        let is_field_position = matches!(previous, Some('{') | Some(','));
        // `Dropped(u64),` inside an enum body: a variant's payload has no field
        // name, so the position is the name, exactly as for a tuple struct.
        if is_field_position
            && items.last().is_some_and(|(_, _, is_enum)| *is_enum)
            && word.starts_with(|ch: char| ch.is_ascii_uppercase())
        {
            let tail = text[word_end..].trim_start();
            if tail.starts_with('(') {
                out.extend(tuple_declarations(
                    &aliases,
                    &format!("{}::{word}", owner_of(&items)),
                    balanced(tail, 1, b'(', b')'),
                ));
            }
        }
        let lowercase = word
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if is_field_position
            && lowercase
            && bytes.get(word_end).copied() == Some(b':')
            && bytes.get(word_end + 1).copied() != Some(b':')
        {
            let inside_a_brace = {
                // The innermost delimiter must be a brace: `(` is a parameter
                // list, `[` is an array.
                let mut brace = false;
                let mut nesting = 0usize;
                for byte in text[..index].bytes().rev() {
                    match byte {
                        b')' | b']' | b'}' => nesting += 1,
                        b'(' | b'[' if nesting == 0 => break,
                        b'{' if nesting == 0 => {
                            brace = true;
                            break;
                        }
                        b'(' | b'[' | b'{' => nesting -= 1,
                        _ => {}
                    }
                }
                brace
            };
            if inside_a_brace {
                let ty = read_type(&text[word_end + 1..], &[',']);
                if accumulates(&aliases, word, &ty) {
                    out.push(Declaration {
                        owner: owner_of(&items),
                        name: word.to_string(),
                        ty,
                    });
                }
            }
        }
        index = word_end;
    }
    out.sort();
    out.dedup();
    out
}

/// A type, read to the first `stop` byte outside any bracket. `}` and a blank
/// line always stop it, so a malformed read cannot swallow the rest of a file.
fn read_type(text: &str, stop: &[char]) -> String {
    let text = text.trim_start();
    let mut depth = 0usize;
    let mut end = text.len();
    for (index, ch) in text.char_indices() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            // A line break inside a path or a bound continues the type; one
            // after a complete type ends it, which is what keeps a malformed
            // read from swallowing the rest of the file.
            '\n' if depth == 0
                && matches!(
                    text[..index].trim_end().chars().next_back(),
                    Some(':') | Some('+') | Some('&') | Some(',')
                ) => {}
            '}' | '\n' if depth == 0 => {
                end = index;
                break;
            }
            _ if depth == 0 && stop.contains(&ch) => {
                end = index;
                break;
            }
            _ => {}
        }
    }
    text[..end].split_whitespace().collect()
}

/// `(struct name, prefix, fields)` for every `wire_counters!` invocation.
///
/// All three delimiters are recognised, and whitespace before the `!` and
/// before the delimiter is skipped: a macro invocation is the same item however
/// it is spelled, and a scan that knew one spelling would miss the others.
fn wire_counter_invocations(text: &str) -> Vec<(String, String, Vec<(String, String)>)> {
    const NAME: &str = "wire_counters";
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(at) = text[search..].find(NAME) {
        let name_start = search + at;
        search = name_start + NAME.len();
        // `macro_rules! wire_counters` is the definition, not an invocation,
        // and a longer identifier ending in the name is a different item.
        let before = text[..name_start].chars().next_back();
        if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == '!') {
            continue;
        }
        let tail = text[search..].trim_start();
        let Some(after_bang) = tail.strip_prefix('!') else {
            continue;
        };
        let after_bang = after_bang.trim_start();
        let (open, close) = match after_bang.as_bytes().first() {
            Some(b'{') => (b'{', b'}'),
            Some(b'(') => (b'(', b')'),
            Some(b'[') => (b'[', b']'),
            _ => continue,
        };
        let start = text.len() - after_bang.len() + 1;
        search = start;
        let body = balanced(text, start, open, close);
        // `wire_counters!(@role ..)` is the macro calling its own internal arm,
        // not a declaration of anything.
        if body.trim_start().starts_with('@') {
            continue;
        }
        let mut name = String::new();
        let mut prefix = String::new();
        let mut fields = Vec::new();
        for line in body.lines() {
            let mut trimmed = line.trim();
            for marker in ["pub(crate) ", "pub(super) ", "pub "] {
                if let Some(rest) = trimmed.strip_prefix(marker) {
                    trimmed = rest.trim_start();
                    break;
                }
            }
            if let Some(rest) = trimmed.strip_prefix("struct ") {
                let Some((ident, tail)) = rest.split_once(',') else {
                    continue;
                };
                name = ident.trim().to_string();
                if let Some(open) = tail.find('"') {
                    let after = &tail[open + 1..];
                    if let Some(end) = after.find('"') {
                        prefix = after[..end].to_string();
                    }
                }
                continue;
            }
            if let Some(declaration) = trimmed.strip_suffix(',') {
                // `field => Role,` declares the unit role beside the name; a
                // bare `field,` publishes it as `Undeclared`.
                let (field, role) = match declaration.split_once("=>") {
                    Some((field, role)) => (field.trim(), role.trim()),
                    None => (declaration.trim(), "Undeclared"),
                };
                if !field.is_empty()
                    && field
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                {
                    fields.push((field.to_string(), role.to_string()));
                }
            }
        }
        assert!(
            !name.is_empty() && !prefix.is_empty(),
            "a wire_counters! invocation has no struct name or no prefix"
        );
        out.push((name, prefix, fields));
    }
    out
}

/// The argument list of every CALL to `name`.
///
/// A longer identifier ending in `name`, and the `fn name(` declaration, are
/// not calls: the declaration's parameter list would otherwise be read as an
/// argument list carrying no domain.
fn call_arguments<'a>(text: &'a str, name: &str) -> Vec<(usize, &'a str)> {
    let needle = format!("{name}(");
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(at) = text[search..].find(&needle) {
        let name_start = search + at;
        search = name_start + needle.len();
        let head = &text[..name_start];
        if head
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        {
            continue;
        }
        if head.trim_end().ends_with("fn") {
            continue;
        }
        out.push((name_start, balanced(text, search, b'(', b')')));
    }
    out
}

/// Both funnels that write a DOMAIN_PERF record WITH a diagnostic trailer.
///
/// The typed one is current production practice for the migrated domains, so a
/// census that inventoried only the untyped one would let a second emission for
/// a registered domain through the other door.
const DIAGNOSTIC_EMITTERS: &[&str] = &[
    "append_compact_domain_perf_record_with_diagnostics",
    "append_compact_domain_perf_record_with_declared_diagnostics",
];

/// The argument list of every diagnostic emission call.
fn diagnostic_emission_arguments(text: &str) -> Vec<&str> {
    diagnostic_emission_call_sites(text)
        .into_iter()
        .map(|(_, args)| args)
        .collect()
}

/// The same calls, with the byte offset each one starts at.
fn diagnostic_emission_call_sites(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    for emitter in DIAGNOSTIC_EMITTERS {
        out.extend(call_arguments(text, emitter));
    }
    out
}

/// The name of the `fn` whose body contains `offset`.
fn enclosing_fn(text: &str, offset: usize) -> String {
    let head = &text[..offset];
    let mut best = String::new();
    let mut search = 0usize;
    while let Some(at) = head[search..].find("\nfn ") {
        let start = search + at + "\nfn ".len();
        search = start;
        let name: String = head[start..]
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect();
        if !name.is_empty() {
            best = name;
        }
    }
    best
}

/// Every spelling that denotes the same domain as `domain_const`.
///
/// The collector declares `const COMPACT_DOMAIN_X: u16 = lane::DOMAIN_ID;`, so
/// one domain has two names and a check that compared spellings would attribute
/// an emission written with the other one to nobody. Resolved through the
/// declarations, which are a finite list.
fn declared_u16_constants(text: &str) -> BTreeMap<String, String> {
    let mut declared: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        // ANY VISIBILITY: `pub(crate) const PERF_KIND: u16 = ..;` renames the
        // record type exactly as a private one does (Codex VB2 round 7).
        let trimmed = without_visibility(line.trim());
        let Some(rest) = trimmed.strip_prefix("const ") else {
            continue;
        };
        let Some((name, tail)) = rest.split_once(':') else {
            continue;
        };
        let Some((ty, value)) = tail.split_once('=') else {
            continue;
        };
        if ty.trim() != "u16" {
            continue;
        }
        let Some(value) = value.trim().strip_suffix(';') else {
            continue;
        };
        declared.insert(name.trim().to_string(), value.trim().to_string());
    }
    declared
}

/// Every spelling that denotes the same `u16` constant as `domain_const`.
fn domain_spellings(text: &str, domain_const: &str) -> BTreeSet<String> {
    let declared = declared_u16_constants(text);
    // Follow each name to what it ultimately stands for: a literal, or a lane
    // crate's own `DOMAIN_ID`.
    let root_of = |start: &str| -> String {
        let mut current = start.to_string();
        // The chain is followed further than anyone would write one; a name
        // that is still unresolved after this is its own root, and a root that
        // is not the record type's fails the identifier audit instead.
        for _ in 0..64 {
            match declared.get(&current) {
                Some(next) if *next != current => current = next.clone(),
                _ => break,
            }
        }
        current
    };
    let root = root_of(domain_const);
    let mut out = BTreeSet::from([domain_const.to_string(), root.clone()]);
    for name in declared.keys() {
        if root_of(name) == root {
            out.insert(name.clone());
        }
    }
    out
}

/// Is this `domain_id:` value a constant naming one domain?
///
/// A plain path, so `X as u32` and a local are not one.
fn is_domain_constant(value: &str) -> bool {
    let looks_like_a_path = !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == ':');
    looks_like_a_path
        && (value.starts_with("COMPACT_DOMAIN_")
            || value.ends_with("DOMAIN_ID")
            || value.rsplit("::").next().is_some_and(|tail| {
                tail.starts_with("COMPACT_DOMAIN_") || tail.ends_with("DOMAIN_ID")
            }))
}

/// The value of the `domain_id:` field in a `CompactDomainPerfRecord` literal.
///
/// Matched at a field boundary so `source_domain_id:` is not mistaken for it.
fn perf_record_domain_id(args: &str) -> Option<&str> {
    let mut search = 0usize;
    while let Some(at) = args[search..].find("domain_id:") {
        let start = search + at;
        search = start + "domain_id:".len();
        let before = args[..start].chars().next_back();
        if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
            continue;
        }
        return Some(args[search..].split(',').next().unwrap_or("").trim());
    }
    None
}

/// Domain constants emitted with the plain, trailer-free perf record.
///
/// A call whose `domain_id` is not a constant this scan can name is a call the
/// declared list cannot describe, so it panics rather than skipping: a helper
/// that forwards a computed record would otherwise slip a domain past the list.
fn trailer_free_domains(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (_, args) in call_arguments(text, "append_compact_domain_perf_record") {
        let value = perf_record_domain_id(args).unwrap_or_else(|| {
            panic!(
                "an append_compact_domain_perf_record call names no domain_id, so the \
                 trailer-free domain list cannot account for it:\n{}",
                &args[..args.len().min(400)]
            )
        });
        assert!(
            is_domain_constant(value),
            "an append_compact_domain_perf_record call carries domain_id {value:?}, which \
             is not a domain constant, so the trailer-free domain list cannot account for it"
        );
        out.insert(value.to_string());
    }
    out
}

/// The body of every `<name> {` struct literal in `text`.
fn struct_literals<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let needle = format!("{name} {{");
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(at) = text[search..].find(&needle) {
        let start = search + at;
        search = start + needle.len();
        let head = &text[..start];
        if head
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        {
            continue;
        }
        // `struct HookDiagnostics {` is the declaration, not a literal.
        if head.trim_end().ends_with("struct") {
            continue;
        }
        out.push(balanced(text, search, b'{', b'}'));
    }
    out
}

/// What one struct literal assigns to `field`, whitespace removed. A literal
/// that omits the field and carries a `..rest` reports `..`, which is a lift
/// like any other and must be registered as one.
fn literal_assignment(literal: &str, field: &str) -> String {
    let literal = without_comments_only(literal);
    let mut depth = 0usize;
    let mut last = 0usize;
    let mut spans: Vec<&str> = Vec::new();
    for (index, byte) in literal.as_bytes().iter().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                spans.push(&literal[last..index]);
                last = index + 1;
            }
            _ => {}
        }
    }
    spans.push(&literal[last..]);
    let mut rest = false;
    for span in &spans {
        let trimmed = span.trim();
        if trimmed.starts_with("..") {
            rest = true;
            continue;
        }
        let Some((name, value)) = trimmed.split_once(':') else {
            // Field-shorthand: `field,`.
            if trimmed == field {
                return field.to_string();
            }
            continue;
        };
        if name.trim() == field {
            return value.split_whitespace().collect();
        }
    }
    if rest {
        "..".to_string()
    } else {
        String::new()
    }
}

/// Is `name` incremented by one anywhere in `text`?
///
/// A field incremented by one is a counter however it is named, which is the
/// half of "is this a counter" that does not depend on a vocabulary.
/// Anything that ADDS TO `name`, by one or by a batch.
///
/// This is the guard on an exemption, and it is deliberately broad where
/// `is_incremented` is narrow. `is_incremented` answers "is this a counter",
/// which needs every spelling and is therefore open; this answers "does the
/// file accumulate into this", which a claim of "not a counter" must be able to
/// survive. Over-approximating here can only refuse an exemption.
/// Is `name` assigned the SIZE of something?
///
/// A field that takes `rows.len()` or `iter.filter(..).count()` holds a count,
/// whatever it is called and however innocent its exemption reads; addition is
/// not the only way to arrive at one (Codex VB2 round 7, finding 2).
fn assigns_a_count(text: &str, name: &str) -> bool {
    let text = without_comments_or_strings(text);
    for opener in [format!("{name}:"), format!("{name} =")] {
        let mut search = 0usize;
        while let Some(at) = text[search..].find(&opener) {
            let start = search + at;
            search = start + opener.len();
            let before = text[..start].chars().next_back();
            if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == '.') {
                continue;
            }
            // The assigned expression, to the end of the statement or the field.
            let tail = &text[search..];
            let end = tail
                .find([',', ';'])
                .unwrap_or(tail.len().min(200))
                .min(200);
            let assigned = &tail[..end];
            if assigned.contains(".len()") || assigned.contains(".count()") {
                return true;
            }
            // TO A FIXED POINT, not one hop. `let n = rows.len(); let m = n;`
            // is the same count two assignments away, and a scan that followed a
            // fixed number of them would always have one more to miss. The set
            // of locals in a file is finite, so closing over it terminates.
            for local in sized_locals(&text) {
                if assigned
                    .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                    .any(|token| token == local)
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Locals that hold the SIZE of something, closed over assignment.
///
/// `let n = rows.len();` and then `let m = n;` are both the count. The closure
/// is taken to a FIXED POINT: each pass adds the locals bound to an expression
/// naming a local already in the set, and the file has finitely many locals, so
/// it terminates.
fn sized_locals(text: &str) -> BTreeSet<String> {
    let bindings = local_bindings(text);
    let mut out: BTreeSet<String> = bindings
        .iter()
        .filter(|(_, value)| value.contains(".len()") || value.contains(".count()"))
        .map(|(name, _)| name.clone())
        .collect();
    loop {
        let mut grown = out.clone();
        for (name, value) in &bindings {
            if out.contains(name) {
                continue;
            }
            // A COPY, not any mention. `let m = n;` and `let m = n as u64;`
            // carry the size; `let id = frame.pov_country_id;` does not become
            // one because some other local in the expression is a size.
            // Brackets are not a computation: `let copied = (n);` and
            // `let copied = { n };` are the copy `let copied = n;` with a
            // bracket around it (Codex VB2 rounds 10 and 11).
            let value = value.replace(['(', ')', '{', '}'], "");
            let copied: String = value
                .trim()
                .trim_start_matches('=')
                .split(" as ")
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect();
            let copied = copied
                .trim_end_matches(".try_into()")
                .trim_end_matches("?")
                .to_string();
            if out.contains(&copied) {
                grown.insert(name.clone());
            }
        }
        if grown.len() == out.len() {
            return out;
        }
        out = grown;
    }
}

/// Every `let <name> = <value>;` in `text`, as (name, value).
fn local_bindings(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(at) = text[search..].find("let ") {
        let keyword = search + at;
        let start = keyword + "let ".len();
        search = start;
        // The char before the KEYWORD, so `outlet x` is not a binding.
        let head = text[..keyword].trim_end();
        if head
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
        {
            continue;
        }
        let rest = text[start..].trim_start();
        let rest = rest.strip_prefix("mut ").unwrap_or(rest);
        let name: String = rest
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        let tail = &rest[name.len()..];
        let end = tail.find(';').unwrap_or(tail.len().min(300)).min(300);
        out.push((name, tail[..end].to_string()));
    }
    out
}

/// Does this exemption say where the number it excuses ends up?
///
/// The bar for a COUNT: naming the record that carries it, or the comparison it
/// exists for, is a claim a reader can check against the code. "not a counter"
/// is not, and the defect class is a count that reaches nothing.
fn states_a_destination(reason: &str) -> bool {
    ["record", "publish", "capture", "wire", "compared", "validate"]
        .iter()
        .any(|word| reason.contains(word))
}

fn accumulates_into_by_batch(text: &str, name: &str) -> bool {
    accumulates_into(text, name) && !is_incremented(text, name)
}

fn accumulates_into(text: &str, name: &str) -> bool {
    let collapsed = without_comments_or_layout(text);
    // ADDITION only. Subtraction is how this repository walks backwards through
    // an engine structure -- `count_address.checked_sub(HEADER)` is a field's
    // address, not a tally -- and a counter that only ever goes down is not one
    // anybody would excuse as "not a counter" anyway. A gauge that moves both
    // ways still trips the add.
    const ADDS: &[&str] = &[
        "saturating_add(",
        "wrapping_add(",
        "checked_add(",
        "fetch_add(",
        "fetch_update(",
    ];
    let mut search = 0usize;
    while let Some(at) = collapsed[search..].find(name) {
        let start = search + at;
        search = start + 1;
        let before = collapsed[..start].chars().next_back();
        if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
            // A longer identifier that ends in this one.
            continue;
        }
        let after = &collapsed[start + name.len()..];
        if after.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_') {
            continue;
        }
        // `x += n`, and its bitwise twin.
        if after.starts_with("+=") {
            return true;
        }
        // `x.fetch_add(..)`, `x.set(x.get() + ..)`, and every other method that
        // reads the same name and puts a larger value back.
        if let Some(tail) = after.strip_prefix('.') {
            if ADDS.iter().any(|add| tail.starts_with(add)) {
                return true;
            }
        }
        // `x = <anything>.x.saturating_add(n)` -- the target and the read are
        // both qualified, which is how a batched accumulation is spelled.
        let Some(tail) = after.strip_prefix('=') else {
            continue;
        };
        if tail.starts_with('=') {
            continue;
        }
        let window = &tail[..tail.len().min(160)];
        // `x = 1 + x` reads the name AFTER the operator; the walk below finds
        // the name and looks forward, so the operator before it counts too.
        let mut cursor = 0usize;
        while let Some(at) = window[cursor..].find(name) {
            let at = cursor + at;
            cursor = at + 1;
            let before = window[..at].chars().next_back();
            if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
                continue;
            }
            if before == Some('+') {
                return true;
            }
            let rest = &window[at + name.len()..];
            if let Some(rest) = rest.strip_prefix('.') {
                if ADDS.iter().any(|add| rest.starts_with(add)) {
                    return true;
                }
            }
            if rest.starts_with('+') {
                return true;
            }
            // `x.set(x.get()+n)` and `x.replace(x.get()+n)`: the read of the
            // same name inside the write is what makes it an accumulation.
            if rest.starts_with(".get()+") || rest.starts_with(".load(") {
                return true;
            }
        }
    }
    false
}

fn is_incremented(text: &str, name: &str) -> bool {
    is_incremented_where(text, name, true)
}

/// `as_a_field`: whether `owner.name += 1` counts. It does for a field, and it
/// does not for a LOCAL -- `stamp.value += 1` says nothing about a `let mut
/// value` in another function.
fn is_incremented_where(text: &str, name: &str, as_a_field: bool) -> bool {
    let collapsed = without_comments_or_layout(text);
    [
        format!("{name}+=1"),
        format!("{name}.saturating_add(1)"),
        format!("{name}.wrapping_add(1)"),
        format!("{name}.checked_add(1)"),
        format!("{name}.fetch_add(1"),
    ]
    .iter()
    .any(|form| {
        // WHOLE WORD: `country_count.saturating_add(1)` increments
        // `country_count`, not the `count` a neighbouring struct declares.
        let mut search = 0usize;
        while let Some(at) = collapsed[search..].find(form.as_str()) {
            let start = search + at;
            search = start + 1;
            let previous = collapsed[..start].chars().next_back();
            let opens_a_word =
                previous.is_some_and(|ch| ch.is_alphanumeric() || ch == '_');
            let reaches_through_an_owner = previous == Some('.');
            if !opens_a_word && (as_a_field || !reaches_through_an_owner) {
                return true;
            }
        }
        false
    })
}

/// Identifiers in `text` that are not reached through a `.`.
///
/// `state.counters.callback_count` names ONE thing at the top level -- `state`.
/// A `static callback_count` beside it is not what that expression loads, and a
/// scan that read every identifier would let the expression vouch for it.
fn top_level_names_in(text: &str) -> BTreeSet<&str> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    let mut consumed = 0usize;
    while let Some(start) = rest.find(|ch: char| ch.is_alphanumeric() || ch == '_') {
        let tail = &rest[start..];
        let end = tail
            .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .unwrap_or(tail.len());
        let before = text[..consumed + start].trim_end();
        // `state.counters.callback_count` and `helpers::callback_count()` both
        // name something that is not a module static of that name.
        if !before.ends_with('.') && !before.ends_with("::") {
            out.insert(&tail[..end]);
        }
        consumed += start + end;
        rest = &tail[end..];
    }
    out
}

/// Every identifier in `text`, as whole words.
fn names_in(text: &str) -> BTreeSet<&str> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(start) = rest.find(|ch: char| ch.is_alphanumeric() || ch == '_') {
        let tail = &rest[start..];
        let end = tail
            .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .unwrap_or(tail.len());
        out.insert(&tail[..end]);
        rest = &tail[end..];
    }
    out
}

fn rows_by_producer() -> BTreeMap<&'static str, Vec<&'static str>> {
    let mut out: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
    for row in REGISTRY {
        out.entry(row.producer_struct).or_default().push(row.field);
    }
    out
}

/// The lane's stored counters ARE the macro's declaration.
///
/// `HookState` holds one `wire_counters!` struct, so a stored counter's field
/// exists only because the declaration lists it -- and the declaration lists
/// only what the registry carries. No spelling of "increment" can create an
/// unregistered counter, because the field it would increment cannot exist.
///
/// The declared set is derived from the LIFTS, not from a second list: a
/// registered row whose lift reads `state.counters.x` must have `x` declared,
/// and a declared field with no such row is unaccounted.
///
/// MUTATION: add a field to a lane's `wire_counters!` body, or drop one, and
/// this fails naming it.
#[test]
fn the_stored_counters_of_a_registered_lane_are_the_macro_declaration() {
    for producer in PRODUCERS {
        let text = read(producer.source_file);
        let invocations = wire_counter_invocations(&text);
        let (_, prefix, declared) = invocations
            .iter()
            .find(|(name, _, _)| name == producer.state_counters_struct)
            .unwrap_or_else(|| {
                panic!(
                    "{} declares no wire_counters! struct named {}, so its stored counters \
                     are ordinary fields again and an increment could invent one",
                    producer.source_file, producer.state_counters_struct
                )
            });
        let unit_prefix = REGISTRY
            .iter()
            .find(|row| row.producer_struct == producer.producer_struct)
            .map(|row| {
                row.unit
                    .strip_suffix(row.field)
                    .expect("a unit is concat!(prefix, field)")
                    .to_string()
            })
            .expect("a producer has rows");
        assert_eq!(
            *prefix, unit_prefix,
            "{} publishes under {prefix:?} and its producer publishes under {unit_prefix:?}, \
             so the same counter would reach the wire under two names",
            producer.state_counters_struct
        );
        // The state struct must HOLD it, or the declaration is decoration and
        // the counters live somewhere the registry cannot see.
        let state = without_comments_or_layout(struct_body(&text, producer.state_struct));
        assert!(
            state.contains(&format!("counters:{}", producer.state_counters_struct)),
            "{}::{} does not hold a {} field named `counters`",
            producer.source_file,
            producer.state_struct,
            producer.state_counters_struct
        );
        let expected: BTreeSet<&str> = REGISTRY
            .iter()
            .filter(|row| row.producer_struct == producer.producer_struct)
            .filter(|row| {
                row.lifts
                    .iter()
                    .any(|lift| lift.contains(&format!("counters.{}", row.field)))
            })
            .map(|row| row.field)
            .collect();
        let declared: BTreeSet<&str> =
            declared.iter().map(|(field, _)| field.as_str()).collect();
        assert_eq!(
            declared, expected,
            "{} declares {declared:?}; the rows lifted out of `state.counters` are \
             {expected:?}",
            producer.state_counters_struct
        );
    }
}

/// Nothing counter-shaped is declared outside the macro.
///
/// This is the backstop, and the reason the family is closed. The census does
/// not try to recognise every spelling in which a counter can be INCREMENTED --
/// rounds 4 to 7 named seven of those and the set does not terminate. It reads
/// DECLARATIONS instead, which are one small grammar: `name: Type`. A counter
/// has to be declared somewhere before any spelling can add to it.
///
/// Name-blind over `u8/u16/u32/u64` and their atomics, and over any container
/// of one (`Cell<u64>`, `[u64; N]`, `BTreeMap<_, u64>` -- R7's findings 5 and
/// 6). Name-tested over `usize`, which these files use for addresses.
///
/// MUTATION: declare `static DROPPED_FRAMES: AtomicU64` beside a registered
/// lane's hook, or add `dropped_frames: u64` to any struct in the file, and
/// this fails until it is registered or excused.
#[test]
fn every_counter_shaped_declaration_in_a_registered_lane_comes_from_the_macro() {
    let rows = rows_by_producer();
    let mut report: Vec<String> = Vec::new();
    for producer in PRODUCERS {
        let closure = source_closure(producer.source_file);
        let text: String = closure
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("
");
        // PER FILE, so two modules each declaring a `static NEXT_SEQUENCE` are
        // two declarations. Scanning the joined text would dedupe them into the
        // one the registry's lift accounts for. The type aliases are gathered
        // across the WHOLE closure first, because a module can declare the alias
        // and another can declare the field.
        let aliases: BTreeMap<String, String> = closure
            .iter()
            .flat_map(|(_, text)| type_aliases(&without_comments_or_strings(text)))
            .collect();
        let declarations: Vec<Declaration> = closure
            .iter()
            .flat_map(|(_, text)| counter_typed_declarations_with(text, &aliases))
            .collect();
        let registered: BTreeSet<&str> = rows
            .get(producer.producer_struct)
            .map(|fields| fields.iter().copied().collect())
            .unwrap_or_default();
        let excused_state: BTreeSet<&str> = producer
            .state_fields_off_the_channel
            .iter()
            .map(|(field, _)| *field)
            .collect();
        let excused: BTreeSet<&str> = producer
            .declarations_that_are_not_counters
            .iter()
            .map(|(key, _)| *key)
            .collect();
        // A module `static` is accounted by the LIFT that reads it: that is a
        // proof it reaches the wire, where a name match would only be a
        // coincidence.
        let lifted: String = REGISTRY
            .iter()
            .filter(|row| row.producer_struct == producer.producer_struct)
            .flat_map(|row| row.lifts.iter())
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let mut unaccounted = Vec::new();
        let mut used: BTreeSet<String> = BTreeSet::new();
        for declaration in &declarations {
            let accounted = if declaration.owner == producer.source_struct {
                registered.contains(declaration.name.as_str())
            } else if declaration.owner == producer.state_struct {
                registered.contains(declaration.name.as_str())
                    || excused_state.contains(declaration.name.as_str())
            } else if declaration.owner.is_empty() {
                // WHOLE identifier: `NEXT_SEQUENCE.load(..)` in a lift is not a
                // proof that a `SEQUENCE` static beside it reaches the wire.
                //
                // And only when the closure declares ONE static of that name: a
                // second `NEXT_SEQUENCE` in a submodule is a different counter
                // that the same lift would otherwise vouch for, and the registry
                // names an expression, not a module path.
                top_level_names_in(&lifted).contains(declaration.name.as_str())
                    && declarations
                        .iter()
                        .filter(|other| other.owner.is_empty() && other.name == declaration.name)
                        .count()
                        == 1
            } else {
                false
            };
            if accounted {
                continue;
            }
            if excused.contains(declaration.key().as_str()) {
                used.insert(declaration.key());
                continue;
            }
            unaccounted.push(declaration.key());
        }
        if !unaccounted.is_empty() {
            let files: Vec<&str> = closure.iter().map(|(path, _)| path.as_str()).collect();
            report.push(format!("{files:?}: {unaccounted:?}"));
        }
        for (key, reason) in producer.declarations_that_are_not_counters {
            assert!(
                used.contains(*key),
                "{} excuses {key}, which {} does not declare (or which is registered \
                 already), so the exemption is stale",
                producer.producer_struct,
                producer.source_file
            );
            let name = key.rsplit('.').next().unwrap_or(key);
            assert!(
                !accumulates_into(&text, name),
                "{} accumulates into {key} -- by one or by a batch -- which makes it a \
                 counter whatever the exemption says",
                producer.source_file
            );
            assert!(
                !assigns_a_count(&text, name) || states_a_destination(reason),
                "{} assigns {key} the SIZE of something, and its exemption does not say \
                 which record carries the number",
                producer.source_file
            );
            assert!(
                reason.len() > 20,
                "{key}'s exemption gives no reason a reader could check"
            );
        }
    }
    assert!(
        report.is_empty(),
        "these declarations can hold a count, and neither REGISTRY nor \
         declarations_that_are_not_counters accounts for them: {}. A counter has to be \
         declared before anything can add to it, so this is where an unpublished counter \
         is caught -- whatever spelling increments it.",
        report.join("; ")
    );
}

/// The lane counts into its own state struct and lifts the result into the
/// published diagnostics. A counter kept in the state and never lifted is the
/// d102 defect one step earlier, and the published struct cannot see it.
///
/// MUTATION: add `rejected_foo_count: u64` to any registered `state_struct`.
/// This fails until the counter is registered, which forces it onto the
/// published struct, into the `wire_counters!` declaration and onto the wire.
#[test]
fn every_counter_a_registered_lane_keeps_in_its_own_state_is_registered() {
    let rows = rows_by_producer();
    for producer in PRODUCERS {
        let text = read(producer.source_file);
        let fields = counter_fields(struct_body(&text, producer.state_struct));
        let registered: BTreeSet<&str> = rows
            .get(producer.producer_struct)
            .map(|fields| fields.iter().copied().collect())
            .unwrap_or_default();
        let excused: BTreeSet<&str> = producer
            .state_fields_off_the_channel
            .iter()
            .map(|(field, _)| *field)
            .collect();
        let unaccounted: Vec<&String> = fields
            .iter()
            .filter(|field| {
                !registered.contains(field.as_str()) && !excused.contains(field.as_str())
            })
            .collect();
        assert!(
            unaccounted.is_empty(),
            "{}::{} (or a module-level counter beside it) keeps {unaccounted:?} and neither \
             REGISTRY nor state_fields_off_the_channel accounts for them. A counter the lane \
             keeps but never lifts into {} cannot reach the wire.",
            producer.source_file,
            producer.state_struct,
            producer.source_struct,
        );
        // `*tally.counter_mut(reason) += 1` names no field, so no field census
        // can reach it. A registered lane may not keep counters that way; the
        // one it does keep must be a field the registry can name.
        for statement in without_comments_or_layout(&text).split(['{', '}', ';']) {
            let Some(target) = statement.split("+=").next() else {
                continue;
            };
            assert!(
                !statement.contains("+=") || !target.trim_start().starts_with('*'),
                "{} increments through a dereference, which names no field the registry \
                 could carry:\n{}",
                producer.source_file,
                &statement[..statement.len().min(160)]
            );
        }
        // The exemption is stale unless the state struct really declares the
        // field. Both readings count: the plain-integer scan, and the
        // declaration scan, which also reaches `Option<u64>` and the containers.
        let declared_in_state: BTreeSet<String> = fields
            .iter()
            .cloned()
            .chain(
                counter_typed_declarations(&text)
                    .into_iter()
                    .filter(|declaration| declaration.owner == producer.state_struct)
                    .map(|declaration| declaration.name),
            )
            .collect();
        for (field, reason) in producer.state_fields_off_the_channel {
            assert!(
                declared_in_state.contains(*field),
                "{}::{} has no field {field}, so its exemption is stale",
                producer.source_file,
                producer.state_struct
            );
            assert!(
                !registered.contains(field),
                "{field} is excused from the channel and also registered on it"
            );
            assert!(
                !is_counter_shaped(field),
                "{field} is named like something that counts, so it cannot be excused from \
                 the diagnostic channel whatever the reason says. The exemption list is for \
                 addresses, timestamps and id allocators."
            );
            assert!(
                !accumulates_into(&text, field),
                "{} accumulates into {field} -- by one or by a batch -- which makes it a \
                 counter whatever it is called, so it cannot be excused from the \
                 diagnostic channel",
                producer.source_file
            );
            assert!(
                !assigns_a_count(&text, field) || states_a_destination(reason),
                "{} assigns {field} the SIZE of something, and its exemption does not say \
                 which record carries the number",
                producer.source_file
            );
            assert!(
                reason.len() > 20,
                "{field}'s exemption gives no reason a reader could check"
            );
        }
    }
}

/// The lift from the lane's own state into its published diagnostics.
///
/// The collector cannot drive this function offline -- it reads process-global
/// hook state -- so the arrival harness starts one step downstream of it. That
/// leaves exactly one unproven link, and this closes it: the registry names the
/// expression each field is lifted by, and the census reads the lane's lift
/// function to check it.
///
/// Every RETURN PATH is read, not just one: a lift function with an error
/// branch could keep the registered expression on the branch nobody takes and
/// read a neighbour on the one that runs. The registry lists what each field
/// may be assigned, and the sets must match exactly.
///
/// MUTATION: change any lift in a lane's `diagnostics()` to read a neighbouring
/// field, or add a second return path that lifts differently, and this fails
/// naming the field; the value would otherwise arrive under the right name and
/// be wrong.
#[test]
fn every_registered_counter_is_lifted_by_the_expression_the_registry_names() {
    for producer in PRODUCERS {
        let text = read(producer.source_file);
        let body = fn_body(&text, producer.lift_fn);
        let literals = struct_literals(body, producer.source_struct);
        assert!(
            !literals.is_empty(),
            "{}::{} builds no {} literal",
            producer.source_file,
            producer.lift_fn,
            producer.source_struct
        );
        // `return HookDiagnostics::default()` is a return path like any other,
        // and it publishes zero for every field. A `..Struct::default()` REST
        // inside a literal is already reported as `..` and is not this.
        let collapsed = without_comments_or_layout(body);
        let default_call = format!("{}::default()", producer.source_struct);
        let mut default_returns = 0usize;
        let mut search = 0usize;
        while let Some(at) = collapsed[search..].find(&default_call) {
            let start = search + at;
            search = start + default_call.len();
            if !collapsed[..start].ends_with("..") {
                default_returns += 1;
            }
        }
        let default_return = default_returns > 0;
        // The lifts are a set, so a SECOND degraded path returning `default()`
        // would change nothing in it. Counting the ways out is what notices.
        //
        // Counted twice, by two different readings, and both must agree with
        // the registry: the exits (every `return`, plus the tail expression),
        // which is spelling-independent, and the things this scan can actually
        // READ at an exit. A `return degraded();` moves the first and not the
        // second, so an exit the census cannot read is a failure rather than a
        // silent gap.
        // `return Err(..)` is a REFUSAL, not a way out that publishes a
        // reading: the record it produces carries no source struct at all, and
        // a walk that refuses on thirteen engine reads is thirteen refusals,
        // not thirteen lifts.
        assert_ne!(
            producer.source_struct, "Err",
            "a producer whose published struct is Err makes the refusal rule below wrong"
        );
        let exits = collapsed
            .match_indices("return")
            .filter(|(at, _)| !collapsed[at + "return".len()..].starts_with("Err("))
            .count()
            + 1;
        assert_eq!(
            exits, producer.lift_return_paths,
            "{}::{} has {exits} way(s) out; the registry says {}. A new return path \
             publishes values nobody has read.",
            producer.source_file, producer.lift_fn, producer.lift_return_paths
        );
        assert_eq!(
            literals.len() + default_returns,
            producer.lift_return_paths,
            "{}::{} has {} literal(s) and {default_returns} default return(s), which does \
             not account for its {} way(s) out: one exit returns something this census \
             cannot read.",
            producer.source_file,
            producer.lift_fn,
            literals.len(),
            producer.lift_return_paths
        );
        // A field written AFTER the struct is built is a lift the literal scan
        // cannot see, because the literal it reads is already correct.
        for row in REGISTRY
            .iter()
            .filter(|row| row.producer_struct == producer.producer_struct)
        {
            for statement in collapsed.split(['{', '}', ';']) {
                let Some((target, value)) = statement.split_once('=') else {
                    continue;
                };
                // `==`, `!=`, `<=`, `>=` and the shift-assigns are comparisons
                // or not increments; everything else that ends in `=` writes.
                if value.starts_with('=') || target.ends_with(['=', '<', '>', '!']) {
                    continue;
                }
                // R7 finding 2's shape: `diagnostics.callback_count += 1` after
                // the literal is built. The compound operator is part of the
                // assignment, not part of the target.
                let target = target.trim().trim_end_matches(['+', '-', '*', '/', '%', '&', '|', '^']);
                assert!(
                    !target.trim().ends_with(&format!(".{}", row.field)),
                    "{}::{} assigns {} after building the struct, so the literal the census \
                     reads is not what the function returns",
                    producer.source_file,
                    producer.lift_fn,
                    row.field
                );
            }
        }
        for row in REGISTRY
            .iter()
            .filter(|row| row.producer_struct == producer.producer_struct)
        {
            let mut observed: BTreeSet<String> = literals
                .iter()
                .map(|literal| literal_assignment(literal, row.field))
                .collect();
            if default_return {
                observed.insert("default".to_string());
            }
            let registered: BTreeSet<String> = row
                .lifts
                .iter()
                .map(|lift| lift.split_whitespace().collect::<String>())
                .collect();
            assert_eq!(
                observed, registered,
                "{}::{} assigns {} the expressions {observed:?}; the registry says \
                 {registered:?}. Nothing downstream of the lift can tell a wrong source \
                 from a right one.",
                producer.source_file, producer.lift_fn, row.field
            );
        }
    }
}

/// A row must describe the producer it belongs to, and the field's type must be
/// the type the source declares.
///
/// MUTATION: point a row's `source_file` at another lane, or call a `usize`
/// field a `u64`, and this fails.
#[test]
fn every_registry_row_describes_its_own_producer_and_the_declared_field_type() {
    for producer in PRODUCERS {
        let text = read(producer.source_file);
        let declared: BTreeMap<String, String> =
            counter_fields_with_types(struct_body(&text, producer.source_struct))
                .into_iter()
                .collect();
        for row in REGISTRY
            .iter()
            .filter(|row| row.producer_struct == producer.producer_struct)
        {
            assert_eq!(
                row.source_file, producer.source_file,
                "{} cites a file its producer does not read",
                row.unit
            );
            assert_eq!(
                row.source_struct, producer.source_struct,
                "{} cites a struct its producer does not read",
                row.unit
            );
            assert_eq!(
                declared.get(row.field).map(String::as_str),
                Some(row.source_type),
                "{} says {} is a {}, and the source disagrees",
                row.unit,
                row.field,
                row.source_type
            );
        }
    }
}

/// MUTATION: drop any `CounterWireRow` and this fails naming the field.
#[test]
fn every_source_counter_of_a_registered_producer_has_a_registry_row() {
    let rows = rows_by_producer();
    for producer in PRODUCERS {
        let text = read(producer.source_file);
        let body = struct_body(&text, producer.source_struct);
        let fields = counter_fields(body);
        assert!(
            !fields.is_empty(),
            "{}::{} has no counter fields, so it is not a producer",
            producer.source_file,
            producer.source_struct
        );
        let registered: BTreeSet<&str> = rows
            .get(producer.producer_struct)
            .map(|fields| fields.iter().copied().collect())
            .unwrap_or_default();
        let missing: Vec<&String> = fields
            .iter()
            .filter(|field| !registered.contains(field.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "{}::{} computes {missing:?}, and no row in \
             compact_contract::counter_wire_registry::REGISTRY says which wire name \
             carries them. A counter nobody can read is not an accounted gap.",
            producer.source_file,
            producer.source_struct,
        );
        let declared: BTreeSet<&str> = fields.iter().map(String::as_str).collect();
        let stale: Vec<&&str> = registered
            .iter()
            .filter(|field| !declared.contains(**field))
            .collect();
        assert!(
            stale.is_empty(),
            "REGISTRY rows {stale:?} name fields {}::{} does not have",
            producer.source_file,
            producer.source_struct,
        );
    }
}

/// MUTATION: delete a field line from a `wire_counters!` invocation and this
/// fails; the emission array loses the counter at the same moment.
#[test]
fn every_registry_row_is_published_by_its_producer_declaration() {
    let mut invocations: BTreeMap<String, (String, Vec<(String, String)>)> = BTreeMap::new();
    for producer in PRODUCERS {
        let text = read(producer.producer_file);
        for (name, prefix, fields) in wire_counter_invocations(&text) {
            invocations.insert(name, (prefix, fields));
        }
    }
    let rows = rows_by_producer();
    for producer in PRODUCERS {
        let (prefix, fields) = invocations
            .get(producer.producer_struct)
            .unwrap_or_else(|| {
                panic!(
                    "{} has no wire_counters! invocation in {}",
                    producer.producer_struct, producer.producer_file
                )
            });
        let declared: BTreeSet<&str> =
            fields.iter().map(|(field, _)| field.as_str()).collect();
        let registered: BTreeSet<&str> = rows
            .get(producer.producer_struct)
            .map(|fields| fields.iter().copied().collect())
            .unwrap_or_default();
        assert_eq!(
            declared, registered,
            "{} declares {declared:?} but REGISTRY holds {registered:?}",
            producer.producer_struct
        );
        for row in REGISTRY
            .iter()
            .filter(|row| row.producer_struct == producer.producer_struct)
        {
            assert_eq!(
                row.unit,
                format!("{prefix}{}", row.field),
                "{}'s published name is not concat!(prefix, field)",
                row.field
            );
            // The ROLE the declaration gives the field, READ OUT OF THE SOURCE
            // rather than off the generated table. The arrival harness checks
            // the wire against the registry, so this is what keeps the two
            // from being one statement made twice.
            let declared_role = fields
                .iter()
                .find(|(field, _)| field == row.field)
                .map(|(_, role)| role.as_str())
                .expect("a registered field is declared");
            assert_eq!(
                declared_role,
                format!("{:?}", row.role),
                "{} is declared with a role the registry does not carry",
                row.unit
            );
        }
    }
}

/// MUTATION: declare a `wire_counters!` struct anywhere in the producer trees
/// and leave it out of `PRODUCERS`, and this fails. The scan covers whole
/// directories, not the files `PRODUCERS` already names, so a new producer in a
/// new file has nowhere to hide.
#[test]
fn every_wire_counters_declaration_is_a_registered_producer() {
    let registered: BTreeSet<&str> = PRODUCERS
        .iter()
        .flat_map(|producer| [producer.producer_struct, producer.state_counters_struct])
        .collect();
    let mut seen = BTreeSet::new();
    for path in producer_files() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let text = text.replace('\r', "");
        for (name, _, _) in wire_counter_invocations(&text) {
            assert!(
                registered.contains(name.as_str()),
                "{name} is declared in {} but is not in \
                 counter_wire_registry::PRODUCERS, so nothing checks its rows or its \
                 arrival",
                path.display()
            );
            seen.insert(name);
        }
    }
    for producer in PRODUCERS {
        for name in [producer.producer_struct, producer.state_counters_struct] {
            assert!(
                seen.contains(name),
                "{name} is registered but is declared nowhere"
            );
        }
    }
}

/// The units argument of a call, with comments and layout removed.
///
/// Comments go FIRST: a comma inside one is not an argument separator, and the
/// emission sites carry commented arguments.
fn units_argument(args: &str) -> String {
    let args: String = args
        .lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let args = args.as_str();
    let bytes = args.as_bytes();
    let mut depth = 0usize;
    let mut last = 0usize;
    let mut spans = Vec::new();
    for (index, byte) in bytes.iter().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                spans.push(&args[last..index]);
                last = index + 1;
            }
            _ => {}
        }
    }
    spans.push(&args[last..]);
    let tail = spans
        .iter()
        .rev()
        .find(|span| !span.trim().is_empty())
        .copied()
        .unwrap_or("");
    let joined: String = tail
        .lines()
        .collect::<Vec<_>>()
        .join("")
        .split_whitespace()
        .collect();
    // rustfmt breaks a long call across lines and leaves a trailing comma
    // inside it; the comma is layout, not an argument.
    joined.replace(",)", ")")
}

/// MUTATION: drop `&<domain>_hook_counter_units(..)` from the emission call,
/// wrap it in anything, or hand it a default instead of the lane's live
/// diagnostics, and this fails. The whole argument is pinned because
/// `&fn(&Default::default())` also names the function and publishes zeros.
#[test]
fn every_producer_reaches_its_domain_emission_site() {
    let text = read(COLLECTOR_EMISSION_FILE);
    let calls = diagnostic_emission_arguments(&text);
    for producer in PRODUCERS {
        // EVERY spelling of the domain, not just the one the registry writes:
        // `COMPACT_DOMAIN_HUD_SURFACE_ACTIVATION_LIVE` and
        // `hud_role_observer_live_lane::DOMAIN_ID` are the same domain, and an
        // emission written with the other name would be attributed to nobody.
        let spellings = domain_spellings(&text, producer.domain_const);
        let expected: String = producer.emission_argument.split_whitespace().collect();
        assert!(
            expected.starts_with(&format!("&{}(", producer.emission_fn)),
            "{}'s emission_argument does not call its emission_fn, so the two can drift",
            producer.producer_struct
        );
        let observed: Vec<String> = calls
            .iter()
            .filter(|args| {
                perf_record_domain_id(args).is_some_and(|value| spellings.contains(value))
            })
            .map(|args| units_argument(args))
            .collect();
        assert!(
            !observed.is_empty(),
            "no diagnostic emission carries {} under any of its spellings {spellings:?}",
            producer.domain_const
        );
        // EVERY call, not one of them: a second emission path for the same
        // domain passing `&[]` would otherwise publish a trailer-free record
        // beside the good one, and the existential form could not see it.
        for argument in &observed {
            assert_eq!(
                *argument, expected,
                "a DOMAIN_PERF emission for {} passes {argument} rather than {expected}, so \
                 {}'s counters do not reach that record unaltered",
                producer.domain_const, producer.producer_struct
            );
        }
    }
}

/// Emission sites whose domain is a parameter rather than a constant.
///
/// The per-domain checks cannot attribute these, so each is named here with the
/// function that holds it. The list is asserted to be the whole set, so a new
/// unattributable emission cannot be added quietly — which is what would let a
/// second path publish `&[]` for a registered domain.
const EMISSIONS_WITH_A_DYNAMIC_DOMAIN: &[(&str, &str)] = &[
    (
        "replace_terminal_hook_domain_records",
        "rewrites a hook domain's already-emitted PERF record in place, so it carries the \
         domain it was handed rather than one it names",
    ),
    (
        "append_compact_domain_perf_record",
        "the trailer-free emitter itself, which forwards an empty unit list; the domains \
         that reach it are accounted by DOMAINS_WITH_NO_DIAGNOSTIC_TRAILER",
    ),
    (
        "append_compact_domain_perf_record_with_diagnostics",
        "the untyped funnel itself, which forwards its caller's record and units to the \
         typed one; its callers are the sites this census reads",
    ),
];

/// Every diagnostic emission must name its domain where the scan can read it,
/// or be one of the sites that says out loud that it cannot.
///
/// A call handed a `CompactDomainPerfRecord` built into a local is attributable
/// to no domain, so the per-domain checks above would skip it and it could pass
/// `&[]`.
///
/// MUTATION: build the record into a local and pass the local, and this fails
/// until the enclosing function is named above.
#[test]
fn every_diagnostic_emission_names_its_domain_inline() {
    let text = read(COLLECTOR_EMISSION_FILE);
    let excused: BTreeSet<&str> = EMISSIONS_WITH_A_DYNAMIC_DOMAIN
        .iter()
        .map(|(function, _)| *function)
        .collect();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for (start, args) in diagnostic_emission_call_sites(&text) {
        let owner = enclosing_fn(&text, start);
        let inline = args.contains("CompactDomainPerfRecord {")
            && perf_record_domain_id(args).is_some_and(is_domain_constant);
        if inline {
            continue;
        }
        assert!(
            excused.contains(owner.as_str()),
            "the DOMAIN_PERF emission in {owner} names no domain constant, so no producer \
             check can claim it and it could publish an empty trailer for a registered \
             domain:\n{}",
            &args[..args.len().min(300)]
        );
        seen.insert(
            EMISSIONS_WITH_A_DYNAMIC_DOMAIN
                .iter()
                .find(|(function, _)| *function == owner)
                .map(|(function, _)| *function)
                .expect("just matched"),
        );
    }
    for (function, reason) in EMISSIONS_WITH_A_DYNAMIC_DOMAIN {
        assert!(
            seen.contains(function),
            "{function} is excused from naming its domain and holds no such emission, so \
             the exemption is stale"
        );
        assert!(
            reason.len() > 20,
            "{function}'s exemption gives no reason a reader could check"
        );
    }
}

/// Helpers that REBUILD a domain's already-emitted PERF record.
///
/// `replace_terminal_hook_domain_records` writes a fresh trailer holding only
/// its custody diagnostic, so a registered domain routed through it would lose
/// every counter after the good emission had already happened. It is excused
/// from naming its domain inline, so nothing else would notice.
///
/// MUTATION: pass a registered domain's constant to it and this fails.
#[test]
fn no_registered_domain_is_routed_through_a_record_rebuild() {
    let text = read(COLLECTOR_EMISSION_FILE);
    let registered: BTreeSet<String> = PRODUCERS
        .iter()
        .flat_map(|producer| domain_spellings(&text, producer.domain_const))
        .collect();
    for (_, args) in call_arguments(&text, "replace_terminal_hook_domain_records") {
        let args = without_comments_or_layout(args);
        // The domain is the fourth positional argument. It must be a CONSTANT,
        // or a local binding could carry a registered domain past the check
        // below.
        let arguments: Vec<&str> = args.split(',').map(str::trim).collect();
        let domain = arguments.get(3).copied().unwrap_or("");
        assert!(
            is_domain_constant(domain),
            "a replace_terminal_hook_domain_records call names its domain as {domain:?} \
             rather than as a constant, so nothing can tell whether a registered domain \
             loses its counters there"
        );
        for candidate in &registered {
            assert!(
                domain != candidate,
                "{candidate} is rebuilt by replace_terminal_hook_domain_records, which writes \
                 a trailer holding only its custody diagnostic; every registered counter of \
                 that domain would be dropped from the record a reader sees"
            );
        }
    }
}

/// A renamed import of the macro would expand exactly the same and be invisible
/// to a scan that hunts for its name.
///
/// MUTATION: `use compact_contract::wire_counters as publish_units;` and this
/// fails.
#[test]
fn the_counter_macro_is_never_imported_under_another_name() {
    for path in producer_files() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        // This file spells the forbidden import in order to forbid it.
        if path.ends_with("counter_wire_census.rs") {
            continue;
        }
        // Whitespace removed, so a grouped import broken across lines by
        // rustfmt reads the same as a one-line one.
        let collapsed = without_comments_or_layout(&text.replace('\r', ""));
        assert!(
            !collapsed.contains("wire_countersas"),
            "{} imports wire_counters under another name, which hides every producer it \
             declares from the census",
            path.display()
        );
    }
}

/// A citation nothing resolves is a claim, not evidence.
///
/// Resolving the symbol is only half of it: `recorder_host`'s own
/// `registered_counter_wire_units_reach_the_manifest` asserts
/// [`PERF_UNIT_CONSUMER`] is the function IT drives, so re-pointing the
/// citation at some other real symbol fails there.
///
/// MUTATION: rename `decode_compact_domain_perf_diagnostic_units` in the host
/// and this fails.
#[test]
fn every_consumer_citation_resolves_to_a_symbol_that_exists() {
    let mut checked = BTreeSet::new();
    for row in REGISTRY {
        assert!(
            !row.consumer.is_empty(),
            "{} has no consumer citation",
            row.unit
        );
        if !checked.insert(row.consumer) {
            continue;
        }
        let (file, symbol) = row
            .consumer
            .split_once("::")
            .unwrap_or_else(|| panic!("{} is not a <file>::<symbol> citation", row.consumer));
        let text = read(file);
        assert!(
            text.contains(&format!("fn {symbol}(")),
            "{} cites {symbol} in {file}, which declares no such function",
            row.unit
        );
    }
}

/// The trailer-free scan reads direct calls, so the plain emitter must have no
/// other way of being reached.
///
/// MUTATION: `let emit = append_compact_domain_perf_record;` and call through
/// the binding. The scan would not see the call site and the declared list
/// would stay silent; this fails instead.
#[test]
fn the_trailer_free_emitter_is_only_ever_called_directly() {
    const NAME: &str = "append_compact_domain_perf_record";
    let text = without_comments_only(&read(COLLECTOR_EMISSION_FILE));
    let mut search = 0usize;
    while let Some(at) = text[search..].find(NAME) {
        let start = search + at;
        search = start + NAME.len();
        let before = text[..start].chars().next_back();
        if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
            continue;
        }
        let after = &text[search..];
        // A longer name -- the two `_with_..._diagnostics` emitters.
        if after.starts_with('_') {
            continue;
        }
        if after.starts_with('(') {
            continue;
        }
        let head = text[..start].trim_end();
        if head.ends_with("fn") {
            continue;
        }
        panic!(
            "{NAME} is used somewhere other than a direct call or its own declaration, so \
             the trailer-free domain scan cannot see every domain that reaches it:\n{}",
            &text[start..(start + 120).min(text.len())]
        );
    }
}

/// A name a fold cannot classify is a number whose meaning the consumer guesses.
#[test]
fn every_registry_unit_name_is_classified_and_distinct() {
    let mut seen = BTreeSet::new();
    for row in REGISTRY {
        assert!(
            declares_explicit_unit_accumulation(row.unit),
            "{} has no accumulation class in compact_contract::diagnostic_channel",
            row.unit
        );
        assert_ne!(
            compact_contract::diagnostic_channel::unit_accumulation(row.unit),
            Accumulation::Undeclared,
            "{} is inside a stateful family with no class of its own",
            row.unit
        );
        assert!(seen.insert(row.unit), "{} is registered twice", row.unit);
    }
}

/// The trailer-free domains are the remaining backlog of this defect class, and
/// the list says so out loud.
///
/// MUTATION: add a domain that emits the plain perf record and this fails until
/// the list names it.
#[test]
fn the_trailer_free_domain_list_matches_the_collector() {
    let observed = trailer_free_domains(&read(COLLECTOR_EMISSION_FILE));
    let declared: BTreeSet<String> = DOMAINS_WITH_NO_DIAGNOSTIC_TRAILER
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let added: Vec<&String> = observed.difference(&declared).collect();
    let removed: Vec<&String> = declared.difference(&observed).collect();
    assert!(
        added.is_empty(),
        "these domains emit a DOMAIN_PERF record with no diagnostic trailer and are not \
         in DOMAINS_WITH_NO_DIAGNOSTIC_TRAILER: {added:?}"
    );
    assert!(
        removed.is_empty(),
        "these domains now carry a trailer and must leave \
         DOMAINS_WITH_NO_DIAGNOSTIC_TRAILER: {removed:?}"
    );
}

/// The scans must see what they are meant to protect, or every assertion above
/// is checking an empty set.
#[test]
fn the_source_scans_find_what_they_protect() {
    let collector = read(COLLECTOR_EMISSION_FILE);
    assert!(
        wire_counter_invocations(&collector).len() >= PRODUCERS.len(),
        "the wire_counters! scan found fewer declarations than there are producers"
    );
    assert!(
        diagnostic_emission_arguments(&collector).len() > 1,
        "the emission-call scan found nothing"
    );
    assert!(
        !trailer_free_domains(&collector).is_empty(),
        "the trailer-free scan found nothing, so its list proves nothing"
    );
    let hud = read("lanes/hud_role_observer_live/src/a729_hook.rs");
    let fields = counter_fields(struct_body(&hud, "HookDiagnostics"));
    for expected in [
        // A plain `u64`, and the `usize` and `Option<String>` beside it: the
        // scan must read the first two and skip the third, or its exclusions
        // would be doing the work its inclusions claim to.
        "callback_count",
        "rejected_observation_count",
        "authenticated_surface_count",
    ] {
        assert!(
            fields.iter().any(|field| field == expected),
            "the field scan missed {expected}, so it could miss a counter"
        );
    }
    assert!(
        !fields.iter().any(|field| field == "current_error"),
        "the field scan read an Option<String> as a counter"
    );
    let declarations = counter_typed_declarations(&hud);
    let named = |name: &str| declarations.iter().any(|found| found.name == name);
    for expected in [
        // The three shapes the backstop exists for, all in one file: a module
        // `static` that accumulates, a `Cell` inside a thread-local, and an
        // ordinary field.
        "NEXT_SEQUENCE",
        "ACTIVE_CALLBACKS",
        "callback_count",
    ] {
        assert!(
            named(expected),
            "the declaration scan missed {expected}, so a counter declared that way could \
             hide from the registry. It found {declarations:?}"
        );
    }
    for absent in [
        // An address and a flag: neither can hold a count, and a backstop that
        // read them would be an exemption list of pointers.
        "module_base",
        "installed",
    ] {
        assert!(
            !named(absent),
            "the declaration scan read {absent} as something that counts"
        );
    }
    assert!(
        counter_typed_declarations("fn f(count: u64) {}").is_empty(),
        "the declaration scan reads a function parameter as a field"
    );
    assert_eq!(
        counter_typed_declarations(
            "struct S { calls: u64, }\n#[cfg(test)]\nmod t { struct T { hits: u64, } }"
        )
        .len(),
        1,
        "the declaration scan reads a test fixture's counter as production"
    );
    // A `//` inside a string literal, and a lone quote inside a comment: strip
    // comments and strings in separate passes and either one desynchronises the
    // brace tracking, which would silently empty the scan.
    assert_eq!(
        counter_typed_declarations("struct S { calls: u64, }\nfn f() { let _ = \"a // b\"; }").len(),
        1,
        "a // inside a string literal truncates the scan"
    );
    assert_eq!(
        counter_typed_declarations("// a lone \" in a comment\nstruct S { calls: u64, }").len(),
        1,
        "a quote inside a comment starts a string literal"
    );
    assert_eq!(
        counter_typed_declarations(
            "#[cfg(test)]\nuse std::x;\nstruct S { calls: u64, }"
        )
        .len(),
        1,
        "a #[cfg(test)] item with no block of its own swallows the next one"
    );
    assert_eq!(
        counter_typed_declarations("struct S {\n    #[allow(dead_code)]\n    calls: u64,\n}").len(),
        1,
        "an attribute between the opener and the field hides the declaration"
    );
    assert_eq!(
        counter_typed_declarations(
            "static CALLS: std::sync::atomic::AtomicU64 =\n    std::sync::atomic::AtomicU64::new(0);"
        )
        .len(),
        1,
        "a path-qualified type is a different type to the scan than its imported spelling"
    );
    assert_eq!(
        counter_typed_declarations("struct S {\n    calls:\n        u64,\n}").len(),
        1,
        "a field whose type is on the next line reads as no type at all"
    );
    // The closure: the file the registry names, the crate root read as a leaf,
    // and every module the named file declares.
    let closure: BTreeSet<String> = source_closure("lanes/hud_role_observer_live/src/a729_hook.rs")
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    assert_eq!(
        closure,
        ["lanes/hud_role_observer_live/src/a729_hook.rs".to_string(),
         "lanes/hud_role_observer_live/src/lib.rs".to_string()]
            .into_iter()
            .collect::<BTreeSet<String>>(),
        "the module closure is not what the backstop is documented to read"
    );
    assert!(
        crate_closure("lanes/design_catalog_context_live/src/a729_hook.rs").len() > 5,
        "the crate walk does not follow the root's own modules, so the inventory          of what sits outside a domain's closure would always be empty"
    );
    assert_eq!(
        counter_typed_declarations("struct Tally(u64);").len(),
        1,
        "a tuple struct holds a count in a position the name grammar cannot see"
    );
    assert_eq!(
        counter_typed_declarations("enum Gap {
    Dropped(u64),
    Refused,
}").len(),
        1,
        "an enum variant payload holds a count the same way"
    );
    assert_eq!(
        counter_typed_declarations("struct S {
    pair: (u32, Reason),
}").len(),
        1,
        "a tuple has no privileged position, so every element is a place a count could live"
    );
    assert_eq!(
        counter_typed_declarations("type Tally = u64;
struct S {
    calls: Tally,
}").len(),
        1,
        "a counter behind a same-file type alias is the same counter"
    );
    assert!(
        counter_typed_declarations("struct S(String);").is_empty(),
        "a tuple struct that holds no number is not a counter"
    );
    assert_eq!(innermost_type("BTreeMap<Reason, u64>"), "u64");
    assert_eq!(innermost_type("[u64; 4]"), "u64");
    assert_eq!(innermost_type("std::sync::atomic::AtomicU64"), "AtomicU64");
    assert_eq!(
        innermost_type("std::collections::BTreeMap<Reason, core::primitive::u64>"),
        "u64"
    );
    assert_eq!(
        innermost_type("BTreeMap<HudSurface, RawHudControllerRoot>"),
        "RawHudControllerRoot"
    );
    let files = producer_files();
    assert!(
        files.len() > 200,
        "the producer-file walk found {} files, so a new producer could sit outside it",
        files.len()
    );
    assert!(
        files.iter().any(|path| path.ends_with("attach_log.rs")),
        "the producer-file walk does not reach the collector"
    );
    assert_eq!(
        perf_record_domain_id("source_domain_id: X, domain_id: COMPACT_DOMAIN_Y,"),
        Some("COMPACT_DOMAIN_Y"),
        "the domain_id scan reads source_domain_id as the record's own domain"
    );
}

/// The domains that build their DOMAIN_PERF trailer by hand.
///
/// Every one of them is the shape this registry replaces: a name typed at the
/// emission site with nothing tying it to the field whose value it carries.
/// They are listed rather than converted because converting a domain is a
/// migration per domain, and the list is asserted EQUAL to the tree below, so a
/// new hand-written producer cannot be added quietly -- which is what a
/// registry that only checks its own members could not say.
const DOMAINS_WITH_HAND_WRITTEN_DIAGNOSTICS: &[&str] = &[
    "COMPACT_DOMAIN_ACTIVE_COMMAND_ABILITIES_LIVE",
    "COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE",
    "COMPACT_DOMAIN_AIR_LOSS_STATS_LIVE",
    "COMPACT_DOMAIN_AIR_SUPERIORITY_INPUTS",
    "COMPACT_DOMAIN_AIR_WING_LOADOUTS",
    "COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE",
    "COMPACT_DOMAIN_ARMY_ORDERS",
    "COMPACT_DOMAIN_CAPITULATION_WAR_STATE_LIVE",
    "COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE",
    "COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE",
    "COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE",
    "COMPACT_DOMAIN_COUNTRY_ARMY_UPGRADE_REQUESTS_LIVE",
    "COMPACT_DOMAIN_COUNTRY_CORE_SCALARS_LIVE",
    "COMPACT_DOMAIN_COUNTRY_DYNAMIC_MODIFIERS_LIVE",
    "COMPACT_DOMAIN_COUNTRY_IDENTITY",
    "COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_LIVE",
    "COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_WEIGHT_LIVE",
    "COMPACT_DOMAIN_DELAYED_EVENTS_LIVE",
    "COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE",
    "COMPACT_DOMAIN_DIVISIONS",
    "COMPACT_DOMAIN_DIVISION_COMBAT_STATS_LIVE",
    "COMPACT_DOMAIN_DIVISION_EQUIPMENT",
    "COMPACT_DOMAIN_DYNAMIC_COLLECTION_TRACE_LIVE",
    "COMPACT_DOMAIN_FIRED_EVENT_POPUPS_LIVE",
    "COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE",
    "COMPACT_DOMAIN_HUB_MOTORIZATION_LIVE",
    "COMPACT_DOMAIN_LAND_COMBATS",
    "COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE",
    "COMPACT_DOMAIN_MAP_FURNITURE_VICTORY_POINTS_LIVE",
    "COMPACT_DOMAIN_MARKET_TRADE_LIVE",
    "COMPACT_DOMAIN_NAVAL_COMBAT_LIFECYCLE_LIVE",
    "COMPACT_DOMAIN_NAVAL_COMBAT_MEMBERS_LIVE",
    "COMPACT_DOMAIN_NAVAL_MISSIONS_STATUS_LIVE",
    "COMPACT_DOMAIN_NUKE_STOCKPILE_LIVE",
    "COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE",
    "COMPACT_DOMAIN_OOB_COMMAND",
    "COMPACT_DOMAIN_PEACE_CONFERENCE_LIVE",
    "COMPACT_DOMAIN_PROJECTS",
    "COMPACT_DOMAIN_RAID_INSTANCES_LIVE",
    "COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE",
    "COMPACT_DOMAIN_SCRIPTED_STATE_FLAGS_VARIABLES_LIVE",
    "COMPACT_DOMAIN_SELECTED_DESIGNER_IDENTITY_LIVE",
    "COMPACT_DOMAIN_SESSION_MULTIPLAYER_METADATA_LIVE",
    "COMPACT_DOMAIN_SHIPS",
    "COMPACT_DOMAIN_SPECIAL_PROJECT_PROGRAMS_LIVE",
    "COMPACT_DOMAIN_SUNK_CONVOY_HISTORY_LIVE",
    "COMPACT_DOMAIN_SUNK_SHIP_HISTORY_LIVE",
    "COMPACT_DOMAIN_SUPPLY_LOGISTICS_ATTRITION_LIVE",
    "COMPACT_DOMAIN_TECH_SHARING_GROUPS_LIVE",
    "COMPACT_DOMAIN_THEATRE_SELECTOR_LIVE",
    "COMPACT_DOMAIN_WEATHER",
    "COMPACT_DOMAIN_XMAP07_SNOW_MUD_LIVE",
    "RC_PROBE_DOMAIN_ID",
];

/// Every domain that emits a diagnostic trailer is registered or listed.
///
/// The registry's other tests all iterate `PRODUCERS`, so a brand-new producer
/// that never invokes `wire_counters!` is invisible to them: it can compute a
/// counter, publish a name for it by hand, drop the counter, and fail nothing.
/// This is the check that sees it (Codex round 2, finding 2).
///
/// MUTATION: add a diagnostic emission for a domain in neither set and this
/// fails naming it.
#[test]
fn every_domain_with_a_diagnostic_trailer_is_registered_or_listed() {
    let text = read(COLLECTOR_EMISSION_FILE);
    let mut registered: BTreeSet<String> = BTreeSet::new();
    for producer in PRODUCERS {
        registered.extend(domain_spellings(&text, producer.domain_const));
    }
    let dynamic: BTreeSet<&str> = EMISSIONS_WITH_A_DYNAMIC_DOMAIN
        .iter()
        .map(|(function, _)| *function)
        .collect();
    let mut emitting: BTreeSet<String> = BTreeSet::new();
    for (offset, args) in diagnostic_emission_call_sites(&text) {
        // An emission whose domain is a parameter is accounted by
        // `every_diagnostic_emission_names_its_domain_inline`, which pins the
        // function that may do it and what it may pass.
        if dynamic.contains(enclosing_fn(&text, offset).as_str()) {
            continue;
        }
        let Some(domain) = perf_record_domain_id(args) else {
            continue;
        };
        emitting.insert(domain.to_string());
    }
    let unaccounted: Vec<&String> = emitting
        .iter()
        .filter(|domain| !registered.contains(*domain))
        .filter(|domain| !DOMAINS_WITH_HAND_WRITTEN_DIAGNOSTICS.contains(&domain.as_str()))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "these domains emit a DOMAIN_PERF trailer and are neither registered producers \
         nor listed as hand-written: {unaccounted:?}. A hand-written trailer is the shape \
         the registry replaces, so a new one has to be declared here to ship.",
    );
    let stale: Vec<&&str> = DOMAINS_WITH_HAND_WRITTEN_DIAGNOSTICS
        .iter()
        .filter(|domain| !emitting.contains(**domain) || registered.contains(**domain))
        .collect();
    assert!(
        stale.is_empty(),
        "these entries name a domain that no longer emits a hand-written trailer, or one \
         that is a registered producer now: {stale:?}"
    );
}

/// Every local a file counts in: `let mut x` that something adds one to.
///
/// A local declares nothing that outlives the function, so the declaration
/// grammar cannot reach it -- and a lane that counts into one and never stores
/// it has computed a number the capture does not carry, which is the defect
/// class exactly. The names are small in practice and are held to the registry
/// like any other declaration.
fn incremented_locals(text: &str) -> BTreeSet<String> {
    let text = without_test_items(&without_comments_or_strings(text));
    let mut out = BTreeSet::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("let mut ") {
        rest = &rest[at + "let mut ".len()..];
        let name: String = rest
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect();
        // FROM THE DECLARATION ON: a local cannot be added to before it exists,
        // so reading backwards would only pick up another binding of the name.
        if !name.is_empty() && is_incremented_where(rest, &name, false) {
            out.insert(name);
        }
    }
    out
}

/// The lane crate's own module tree, from its root.
fn crate_closure(source_file: &str) -> BTreeSet<String> {
    let root = match source_file.rsplit_once("/src/") {
        Some((crate_dir, _)) => format!("{crate_dir}/src/lib.rs"),
        None => source_file.to_string(),
    };
    source_closure(&root)
        .into_iter()
        .map(|(path, _)| path)
        .collect()
}

/// Nothing in a registered lane's crate is outside the backstop unnamed.
///
/// The backstop reads the module closure of the file the registry names, so a
/// counter moved one `mod` sideways -- declared by the crate ROOT rather than by
/// the hook -- would be out of its reach. This names what the crate holds beyond
/// that closure and asserts the two sets equal, so a new sibling module has to
/// be declared here to ship (Codex round 3).
///
/// MUTATION: add `mod accounting;` to a registered lane's `lib.rs` and this
/// fails naming it.
#[test]
fn every_module_of_a_registered_crate_is_scanned_or_named() {
    let mut report: Vec<String> = Vec::new();
    for producer in PRODUCERS {
        let scanned: BTreeSet<String> = source_closure(producer.source_file)
            .into_iter()
            .map(|(path, _)| path)
            .collect();
        let outside: BTreeSet<String> = crate_closure(producer.source_file)
            .into_iter()
            .filter(|path| !scanned.contains(path))
            .collect();
        let named: BTreeSet<String> = producer
            .crate_modules_outside_the_domain
            .iter()
            .map(|(path, _)| (*path).to_string())
            .collect();
        if outside != named {
            report.push(format!(
                "{} scans {scanned:?}; its crate also holds {outside:?} and \
                 crate_modules_outside_the_domain names {named:?}",
                producer.producer_struct
            ));
        }
        for (path, reason) in producer.crate_modules_outside_the_domain {
            assert!(
                reason.len() > 20,
                "{path} is excluded from {}'s scan with no reason a reader could check",
                producer.producer_struct
            );
        }
    }
    assert!(
        report.is_empty(),
        "a registered lane's crate declares modules the declaration backstop does not \
         read, and crate_modules_outside_the_domain does not name them: {}. A counter one \
         `mod` sideways of the hook is still in the crate the lane ships.",
        report.join("; ")
    );
}

/// Every local a registered lane counts in is registered or named.
///
/// A local is invisible to the declaration grammar -- it declares nothing that
/// outlives its function -- and rounds 4 to 7 established that recognising every
/// spelling of "increment" does not terminate. This does not try to: it reads
/// `let mut <name>` and asks only whether something adds ONE to that name, which
/// is a closed set of four forms, and then holds the name to the registry.
///
/// A local is accounted when the producer's own lifts NAME it, which is a proof
/// it reaches the wire. Everything else is listed with what it is, and the list
/// is asserted equal to the tree, so a lane that starts counting into a new
/// local has to say so here (Codex round 3, finding 2).
///
/// MUTATION: add `let mut dropped_rows = 0u64;` and a `+= 1` to a registered
/// lane and this fails naming it.
#[test]
fn every_local_a_registered_lane_counts_in_is_registered_or_named() {
    let mut report: Vec<String> = Vec::new();
    for producer in PRODUCERS {
        let named: BTreeSet<&str> = producer
            .locals_that_are_not_counters
            .iter()
            .map(|(name, _)| *name)
            .collect();
        // KEYED BY FILE: `index` in one module being a loop variable says
        // nothing about an `index` counting drops in another.
        let mut found: BTreeSet<String> = BTreeSet::new();
        for (path, text) in source_closure(producer.source_file) {
            for name in incremented_locals(&text) {
                found.insert(format!("{path}::{name}"));
            }
        }
        let unaccounted: Vec<&String> = found
            .iter()
            .filter(|name| !named.contains(name.as_str()))
            .collect();
        if !unaccounted.is_empty() {
            report.push(format!("{}: {unaccounted:?}", producer.producer_struct));
        }
        for (name, reason) in producer.locals_that_are_not_counters {
            assert!(
                found.contains(*name),
                "{} names the local {name}, which nothing in its closure counts in, so \
                 the entry is stale",
                producer.producer_struct
            );
            assert!(
                reason.len() > 20,
                "{name} is named with no reason a reader could check"
            );
            // The same guard the other inventories carry: a name here is a
            // claim that nothing accumulates into it beyond the one-at-a-time
            // walk it was listed for.
            let (path, local) = name.rsplit_once("::").expect("a local is keyed by its file");
            assert!(
                !accumulates_into_by_batch(&read(path), local),
                "{path} accumulates into {local} by a batch, which makes it a counter \
                 whatever the entry says"
            );
        }
    }
    assert!(
        report.is_empty(),
        "these locals are counted up inside a registered lane and neither the registry's \
         lifts nor locals_that_are_not_counters accounts for them: {}. A number a domain \
         computes and never stores is a number the capture does not carry.",
        report.join("; ")
    );
}

/// Every function that appends a DOMAIN_PERF record is a name the scans know.
///
/// The emission scans search for the two funnel names as TEXT, so a third
/// funnel, or either of the two called under an alias, would be invisible to
/// every check that reads an emission -- including the one that says a domain
/// with a trailer is registered or listed (Codex round 3, finding 6).
///
/// MUTATION: add `fn append_compact_domain_perf_record_with_x(` to the
/// collector, or `use ... append_compact_domain_perf_record_with_diagnostics as
/// emit;`, and this fails.
#[test]
fn the_domain_perf_emitters_are_the_ones_the_scans_know() {
    const PLAIN_EMITTER: &str = "append_compact_domain_perf_record";
    let text = read(COLLECTOR_EMISSION_FILE);
    let known: BTreeSet<&str> = DIAGNOSTIC_EMITTERS
        .iter()
        .copied()
        .chain([PLAIN_EMITTER])
        .collect();
    let mut declared: BTreeSet<&str> = BTreeSet::new();
    let mut rest = text.as_str();
    let needle = format!("fn {PLAIN_EMITTER}");
    while let Some(at) = rest.find(&needle) {
        let tail = &rest[at + 3..];
        let name: &str = &tail[..tail
            .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .unwrap_or(tail.len())];
        declared.insert(name);
        rest = &rest[at + needle.len()..];
    }
    assert_eq!(
        declared, known,
        "the collector declares DOMAIN_PERF appenders the emission scans do not know"
    );
    // Every mention of a funnel must be a CALL. An alias import, a binding
    // (`let emit = append_..._with_diagnostics;`), a function pointer -- each
    // lets an emission be written under a name no scan searches for.
    for path in producer_files() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if path.ends_with("counter_wire_census.rs") {
            continue;
        }
        // STRINGS BLANKED as well as comments: a source-scanning test names the
        // funnel in a literal, and a literal is not a call.
        let collapsed =
            without_comments_or_layout(&without_comments_or_strings(&text.replace('\r', "")));
        for emitter in known.iter() {
            let mut search = 0usize;
            while let Some(at) = collapsed[search..].find(emitter) {
                let start = search + at;
                search = start + emitter.len();
                let before = collapsed[..start].chars().next_back();
                if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
                    // A longer identifier that ends in this one.
                    continue;
                }
                let after = &collapsed[start + emitter.len()..];
                if after.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_') {
                    continue;
                }
                assert!(
                    collapsed[..start].ends_with("fn") || after.starts_with('('),
                    "{} mentions {emitter} without calling it, which lets an emission be \
                     written under a name no scan searches for: {}",
                    path.display(),
                    &after[..after.len().min(60)]
                );
            }
        }
    }
}

/// One call's arguments, split at the commas that are not inside anything.
fn top_level_arguments(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut last = 0usize;
    for (index, byte) in args.as_bytes().iter().enumerate() {
        match byte {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                out.push(args[last..index].trim());
                last = index + 1;
            }
            _ => {}
        }
    }
    let tail = args[last..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Every macro invoked at ITEM LEVEL -- outside every brace -- in `text`.
///
/// Read over the whole text with a brace depth rather than line by line,
/// because `macro_rules !` and an invocation split across lines are the same
/// two items as their one-line spellings (Codex VB2 round 7, finding 4).
fn item_macro_invocations(text: &str) -> BTreeSet<String> {
    let text = without_comments_or_strings(text);
    let bytes = text.as_bytes();
    let mut out = BTreeSet::new();
    let mut depth = 0usize;
    for (index, byte) in bytes.iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b'!' if depth == 0 => {
                let head = text[..index].trim_end();
                let start = head
                    .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == ':'))
                    .map(|at| at + 1)
                    .unwrap_or(0);
                let name = &head[start..];
                // `a != b` and `!flag` are not invocations; a path is. The
                // delimiter is found past any whitespace, because `hidden ! ()`
                // is the same item as `hidden!()`.
                let opens = text[index + 1..]
                    .trim_start()
                    .starts_with(['(', '{', '[']);
                // `macro_rules ! name {` puts the NAME between the bang and the
                // brace, and is an item all the same -- the one that can expand
                // to anything at all.
                let defines = name == "macro_rules";
                if !name.is_empty() && !name.ends_with(':') && (opens || defines) {
                    out.insert(name.to_string());
                }
            }
            _ => {}
        }
    }
    out
}

/// The string literals in `text`, in order.
fn string_literals(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != b'"' {
            // 92 is the backslash: an escaped quote does not end the literal.
            end += if bytes[end] == 92 { 2 } else { 1 };
        }
        if end > bytes.len() {
            break;
        }
        out.push(text[start..end.min(bytes.len())].to_string());
        index = end + 1;
    }
    out
}

/// Every top-level `fn` of `text`, as (signature, body).
fn top_level_fns(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(at) = text[search..].find("
fn ") {
        let start = search + at + 1;
        search = start + 3;
        let Some(brace) = text[start..].find('{') else {
            break;
        };
        let body = balanced(text, start + brace + 1, b'{', b'}');
        out.push((&text[start..start + brace], body));
    }
    out
}

/// The hand-written units of a registered domain are inventoried.
///
/// A REGISTERED domain's trailer is not only its counters. d102's emission
/// copies a MEASURED slice -- units the collector builds by hand for the hour --
/// and appends the registered counters after it, so a unit can join that
/// trailer with no registry row at all (Codex VB2 round 6, finding 1). The
/// counter mechanism cannot make those units impossible; what it can do is stop
/// one arriving unremarked, which is what this asserts, in both directions and
/// by two independent readings:
///
/// * every unit built in a function that CONSTRUCTS the carrier type, whatever
///   the unit is called, and
/// * every unit anywhere in the collector whose name starts with a registered
///   producer's PREFIX, wherever it was built.
///
/// A producer whose emission passes no measured slice declares no carrier, and
/// that is asserted against the emission argument rather than assumed.
///
/// MUTATION: add a `CompactDomainPerfDiagnosticUnit` to any d102 read
/// constructor, or a `fog_derivation_inputs_*` unit anywhere in the collector,
/// and this fails.
#[test]
fn every_unit_that_can_join_a_registered_trailer_is_registered_or_named() {
    const UNIT_LITERAL: &str = "CompactDomainPerfDiagnosticUnit";
    let text = without_test_items(&without_comments_only(&read(COLLECTOR_EMISSION_FILE)));
    let registered: BTreeSet<&str> = REGISTRY.iter().map(|row| row.unit).collect();
    for producer in PRODUCERS {
        // THE CARRIER IS NOT ASSUMED: the emission argument says whether the
        // trailer takes anything besides the producer's own counters.
        let takes_measured = producer
            .emission_argument
            .matches(',')
            .count()
            .saturating_sub(0)
            > 0;
        assert_eq!(
            takes_measured,
            !producer.measured_units_carrier.is_empty(),
            "{}'s emission argument is {:?}, which does not agree with its measured-unit              carrier {:?}",
            producer.producer_struct,
            producer.emission_argument,
            producer.measured_units_carrier
        );
        let named: BTreeSet<&str> = producer
            .measured_units
            .iter()
            .map(|(name, _)| *name)
            .collect();
        for (name, why) in producer.measured_units {
            assert!(
                why.len() > 20,
                "{name} is named with no reason a reader could check"
            );
            assert!(
                !registered.contains(name),
                "{name} is both a registry row and a measured unit, so one of the two                  accounts for it twice"
            );
        }
        if producer.measured_units_carrier.is_empty() {
            assert!(
                named.is_empty(),
                "{} names measured units but no carrier builds them",
                producer.producer_struct
            );
            continue;
        }
        // READING ONE: the functions that construct the carrier.
        let mut found: BTreeSet<String> = BTreeSet::new();
        let mut unresolved: BTreeSet<String> = BTreeSet::new();
        for (signature, body) in top_level_fns(&text) {
            if !signature.contains(producer.measured_units_carrier) {
                continue;
            }
            for literal in struct_literals(body, UNIT_LITERAL) {
                let assigned = literal_assignment(literal, "name");
                let names = string_literals(&assigned);
                if names.is_empty() {
                    let (_, why) = producer
                        .measured_unit_name_sources
                        .iter()
                        .find(|(expression, _)| *expression == assigned)
                        .unwrap_or_else(|| {
                            panic!(
                                "a {UNIT_LITERAL} built in {signature:?} takes its name from                                  {assigned:?}, which is not a literal and is not a name source                                  {} accounts for; nothing can say what it publishes",
                                producer.producer_struct
                            )
                        });
                    assert!(
                        why.len() > 20,
                        "{assigned} is named with no reason a reader could check"
                    );
                    unresolved.insert(assigned.clone());
                    continue;
                }
                found.extend(names);
            }
        }
        assert_eq!(
            found,
            named.iter().map(|name| name.to_string()).collect(),
            "{}'s carrier {} builds units the registry does not account for, or names              units it no longer builds",
            producer.producer_struct,
            producer.measured_units_carrier
        );
        assert_eq!(
            unresolved,
            producer
                .measured_unit_name_sources
                .iter()
                .map(|(expression, _)| expression.to_string())
                .collect(),
            "{} names a unit-name source its carrier does not build any more",
            producer.producer_struct
        );
        // READING ONE AND A HALF: the units BUILDER copies and appends, and
        // builds nothing of its own. A unit pushed there joins the trailer of a
        // registered domain in a function that names no carrier, under any name
        // it likes (Codex VB2 round 7, finding 1).
        let builder = fn_body(&text, producer.emission_fn);
        assert!(
            struct_literals(builder, UNIT_LITERAL).is_empty(),
            "{} builds a diagnostic unit of its own; it may copy the measured slice and              append the registered counters, and nothing else",
            producer.emission_fn
        );
        // READING TWO: the producer's PREFIX, wherever the unit was built.
        let prefix = REGISTRY
            .iter()
            .find(|row| row.producer_struct == producer.producer_struct)
            .map(|row| {
                row.unit
                    .strip_suffix(row.field)
                    .expect("a unit name is its prefix and its field")
            })
            .expect("a producer has at least one registry row");
        for literal in struct_literals(&text, UNIT_LITERAL) {
            for name in string_literals(&literal_assignment(literal, "name")) {
                if !name.starts_with(prefix) {
                    continue;
                }
                assert!(
                    registered.contains(name.as_str()) || named.contains(name.as_str()),
                    "{name} joins {}'s trailer under its own prefix and is neither a                      registry row nor a named measured unit",
                    producer.producer_struct
                );
            }
        }
    }
}

/// Every DOMAIN_PERF payload built outside an emitter carries no trailer.
///
/// The emitter census above reads FUNCTION NAMES. A function that built the
/// payload itself and pushed the record would emit a DOMAIN_PERF record no
/// name-based scan sees (Codex round 5, finding 7). One does: the country
/// prebuild path. It is safe for a reason a reader can check rather than
/// assume -- its units argument is the empty literal, and a record with no
/// trailer cannot carry a counter.
///
/// The record type is read through its SPELLINGS, not its one token: a
/// `const PERF_KIND: u16 = COMPACT_RECORD_TYPE_DOMAIN_PERF;` and an append
/// under that name is the same record. And the whole COLLECTOR is read, not
/// the one emission file: a child module calling the raw appender would be a
/// DOMAIN_PERF emission in a file this census never opened (Codex VB2 round 6,
/// finding 3).
///
/// MUTATION: hand that call a units slice, add a second payload builder, build
/// a DOMAIN_PERF record in a function this test does not name, alias the record
/// type through a second constant, or write either name in any other
/// production file of the collector, and it fails.
#[test]
fn a_measured_slice_is_only_ever_filled_where_the_read_is_built() {
    let text = without_test_items(&without_comments_only(&read(COLLECTOR_EMISSION_FILE)));
    let mut search = 0usize;
    let mut sites = 0usize;
    while let Some(at) = text[search..].find("diagnostic_units") {
        let start = search + at;
        search = start + "diagnostic_units".len();
        let before = text[..start].chars().next_back();
        if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
            continue;
        }
        if text[search..].starts_with(|ch: char| ch.is_alphanumeric() || ch == '_') {
            continue;
        }
        // A DECLARATION or a struct-literal field, not an access: those are the
        // reads the fill rule below is not about.
        if !text[..start].trim_end().ends_with('.') {
            continue;
        }
        let tail = text[search..].trim_start();
        // ANY MUTATION, not the two method names a previous round modelled: a
        // method call, an index write, or an assignment to the field itself.
        // `&read.diagnostic_units,` at an emission site is a READ and passes
        // (Codex VB2 round 8, finding 2).
        // READS ARE NAMED, everything else is a mutation. The direction matters:
        // a new method this list does not know is refused rather than assumed
        // harmless, which is what makes the rule closed over method names.
        const READS: &[&str] = &[
            ".clone()",
            ".len()",
            ".is_empty()",
            ".iter()",
            ".as_slice()",
            ".to_vec()",
        ];
        let reads = READS.iter().any(|read| tail.starts_with(read));
        let mutates = !reads
            && (tail.starts_with('.')
                || tail.starts_with('[')
                || (tail.starts_with('=') && !tail.starts_with("==")));
        if !mutates {
            continue;
        }
        sites += 1;
        // The enclosing function's SIGNATURE names the read it fills. A unit
        // added anywhere else -- at the emission site, after the read is
        // finalized -- joins a trailer in a function no carrier census reads.
        let owner = enclosing_fn(&text, start);
        let signature = text
            .find(&format!("fn {owner}("))
            .map(|at| &text[at..at + text[at..].find('{').unwrap_or(0)])
            .unwrap_or("");
        assert!(
            signature
                .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .any(|token| token.ends_with("Read") && token.len() > "Read".len()),
            "{owner} fills a diagnostic slice without naming the read it belongs to, so              nothing can say whose trailer the unit joins"
        );
    }
    assert!(
        sites >= 8,
        "the measured-slice scan found {sites} fill sites, which is fewer than the          collector has; it is reading the wrong thing"
    );
    // EVERY MENTION IN THE REGIONS THIS CLAIM IS ABOUT, PINNED IN CONTEXT.
    //
    // The regions are the functions that CONSTRUCT a registered carrier and
    // the function that EMITS it, which is where a unit can join or leave a
    // registered trailer. Inside them, every mention of the field name is
    // pinned by the 22 characters before and 14 after it, keyed by the
    // function. A borrow, a `mem::take`, a destructuring bind, a read swapped
    // for a write: each changes a context or adds one, and none can be
    // reached by naming spellings, which is what four rounds of review
    // demonstrated (owner ruling 2026-08-29: close the class by counted,
    // asserted completeness).
    const SLICE_MENTIONS: &[&str] = &[
        "build_compact_extraction_gate_records|) as u32; let mut|= domain56_dia",
        "build_compact_extraction_gate_records|.walk_counters, &read.|), );",
        "build_compact_extraction_gate_records|.walk_counters, &read.|), );",
        "build_compact_extraction_gate_records|::mem::take(&mut read.|); d93_tra",
        "build_compact_extraction_gate_records|= ship_read { let|= domain29_dia",
        "build_compact_extraction_gate_records|_units_from_declared(&|); let d42",
        "build_compact_extraction_gate_records|agnostic_units = read.|.clone();",
        "build_compact_extraction_gate_records|ain_status, row_count,|)) = peace",
        "build_compact_extraction_gate_records|ain_status, row_count,|)) = selec",
        "build_compact_extraction_gate_records|all_ms, &read.|, );",
        "build_compact_extraction_gate_records|all_ms, &read.|, );",
        "build_compact_extraction_gate_records|all_ms, &read.|, );",
        "build_compact_extraction_gate_records|all_ms, &read.|, );",
        "build_compact_extraction_gate_records|all_ms, &read.|, );",
        "build_compact_extraction_gate_records|all_ms, &read.|, );",
        "build_compact_extraction_gate_records|all_ms, &read.|, );",
        "build_compact_extraction_gate_records|all_ms, &read.|, );",
        "build_compact_extraction_gate_records|all_ms, &read.|, );",
        "build_compact_extraction_gate_records|all_ms, &read.|, );",
        "build_compact_extraction_gate_records|all_ms, &read.|, );",
        "build_compact_extraction_gate_records|all_ms, &read.|, );",
        "build_compact_extraction_gate_records|ar_rows); let|= capwar60_dia",
        "build_compact_extraction_gate_records|c_units( &|,",
        "build_compact_extraction_gate_records|ding_units_from_perf(&|), );",
        "build_compact_extraction_gate_records|ding_units_from_perf(&|), );",
        "build_compact_extraction_gate_records|ding_units_from_perf(&|), );",
        "build_compact_extraction_gate_records|ding_units_from_perf(&|), );",
        "build_compact_extraction_gate_records|ding_units_from_perf(&|), );",
        "build_compact_extraction_gate_records|ding_units_from_perf(&|), );",
        "build_compact_extraction_gate_records|extras_read { let|= read",
        "build_compact_extraction_gate_records|ges), });|.push(CompactD",
        "build_compact_extraction_gate_records|let|= compact_air_",
        "build_compact_extraction_gate_records|let|= compact_coun",
        "build_compact_extraction_gate_records|let|= d85_declared",
        "build_compact_extraction_gate_records|let|= domain42_dec",
        "build_compact_extraction_gate_records|let|= read.extensi",
        "build_compact_extraction_gate_records|n_live_read { let|= hub_motoriza",
        "build_compact_extraction_gate_records|nt == 0 {|.push(CompactD",
        "build_compact_extraction_gate_records|ock_bracket); let|= d97_declared",
        "build_compact_extraction_gate_records|ograms_read { let|= special_proj",
        "build_compact_extraction_gate_records|row_count,|, )) }",
        "build_compact_extraction_gate_records|row_count,|, )) }",
        "build_compact_extraction_gate_records|s_live_read { let|= air",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|sed_wall_ms, &|, );",
        "build_compact_extraction_gate_records|size) as u32; let|= domain55_dia",
        "build_compact_extraction_gate_records|sweep_snapshot();|.push(CompactD",
        "build_compact_extraction_gate_records|t_live_read { let|= naval_combat",
        "build_compact_extraction_gate_records|units_from_perf(&read.|), );",
        "build_compact_extraction_gate_records|units_from_perf(&read.|), );",
        "build_compact_extraction_gate_records|units_from_perf(&read.|); let d64",
        "build_compact_extraction_gate_records|units_from_perf(&read.|); let d93",
        "build_compact_extraction_gate_records|wall_ms, &|, );",
        "build_compact_extraction_gate_records|wall_ms, &|, );",
        "build_compact_extraction_gate_records|wall_ms, &|, );",
        "build_compact_extraction_gate_records|wall_ms, &|, );",
        "build_compact_extraction_gate_records||.extend(domain",
        "build_compact_extraction_gate_records||.extend(heap_s",
        "build_compact_extraction_gate_records||.extend(heap_s",
        "build_compact_extraction_gate_records||.extend(heap_s",
        "build_compact_extraction_gate_records||.push(CompactD",
        "build_compact_extraction_gate_records|} let mut|= domain47_dia",
        "build_compact_extraction_gate_records|} let mut|= domain53_dia",
        "build_compact_extraction_gate_records|}|.push(CompactD",
        "compute_compact_fog_derivation_inputs|d_hour_raw: None,|: vec![",
        "compute_compact_naval_combat_results|d_hour_raw: None,|: Vec::new(),",
        "finalize_compact_fog_derivation_inputs_read|::new(); read.|.push(CompactD",
        "finalize_compact_fog_derivation_inputs_read|} } read.|.push(CompactD",
        "finalize_compact_naval_combat_results_read|); read.|.push(CompactD",
        "finalize_compact_naval_combat_results_read|e(); read.|.push(CompactD",
        "finalize_compact_naval_combat_results_read|read.|",
        "finalize_compact_naval_combat_results_read|read.|.extend([",
        "finalize_compact_naval_combat_results_read|read.|.push(CompactD",
        "finalize_compact_naval_combat_results_read|read.|.push(CompactD",
        "finalize_compact_naval_combat_results_read|} } read.|.push(CompactD",
    ];
    let raw = read(COLLECTOR_EMISSION_FILE).replace('\r', "");
    let raw = without_comments_or_strings(&raw);
    let mut mentions: Vec<String> = Vec::new();
    for (signature, body) in top_level_fns(&raw) {
        let owner: String = signature
            .trim_start_matches("fn ")
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect();
        let carries = PRODUCERS.iter().any(|producer| {
            !producer.measured_units_carrier.is_empty()
                && signature.contains(producer.measured_units_carrier)
        });
        if !carries && owner != "build_compact_extraction_gate_records" {
            continue;
        }
        let mut search = 0usize;
        while let Some(at) = body[search..].find("diagnostic_units") {
            let start = search + at;
            search = start + "diagnostic_units".len();
            if body[..start]
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
            {
                continue;
            }
            if body[search..].starts_with(|ch: char| ch.is_alphanumeric() || ch == '_') {
                continue;
            }
            let before: String = body[start.saturating_sub(26)..start]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let before: String = before
                .chars()
                .skip(before.chars().count().saturating_sub(22))
                .collect();
            let after: String = body[search..(search + 18).min(body.len())]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let after: String = after.chars().take(14).collect();
            mentions.push(format!("{owner}|{before}|{after}"));
        }
    }
    mentions.sort();
    assert_eq!(
        mentions,
        SLICE_MENTIONS,
        "a function that builds or emits a registered trailer names its diagnostic \
         slice somewhere this census does not account for, or one of the mentions \
         changed; read it and say what it does to the trailer"
    );
}

#[test]
fn a_domain_perf_payload_built_outside_an_emitter_carries_no_trailer() {
    const PAYLOAD_BUILDER: &str = "compact_domain_perf_payload_with_declared";
    const RECORD_TYPE: &str = "COMPACT_RECORD_TYPE_DOMAIN_PERF";
    let text = without_test_items(&without_comments_or_strings(
        &read(COLLECTOR_EMISSION_FILE).replace('\r', ""),
    ));
    // NO OTHER PRODUCTION FILE. The two names below are private to the
    // emission file today, and this is what keeps them there: a scan of one
    // file cannot see an emission written in another.
    for (path, other) in production_closure(COLLECTOR_CRATE_ROOT) {
        if path == COLLECTOR_EMISSION_FILE {
            continue;
        }
        let other = without_test_items(&without_comments_or_strings(&other));
        for forbidden in [RECORD_TYPE, "append_compact_record"] {
            assert!(
                !other.contains(forbidden),
                "{path} names {forbidden}; a DOMAIN_PERF record written there is an                  emission no census of the emission file can see"
            );
        }
    }
    let mut builders: BTreeSet<&str> = BTreeSet::new();
    let mut search = 0usize;
    while let Some(at) = text[search..].find("
fn compact_domain_perf_payload") {
        let start = search + at + "
fn ".len();
        search = start;
        let name = &text[start..];
        builders.insert(
            &name[..name
                .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .unwrap_or(name.len())],
        );
    }
    assert_eq!(
        builders,
        BTreeSet::from([PAYLOAD_BUILDER]),
        "the collector builds DOMAIN_PERF payloads with a function this test does not          account for, so a trailer can be assembled where nothing reads it"
    );
    for (offset, args) in call_arguments(&text, PAYLOAD_BUILDER) {
        let owner = enclosing_fn(&text, offset);
        if DIAGNOSTIC_EMITTERS.contains(&owner.as_str()) {
            continue;
        }
        assert_eq!(
            units_argument(args),
            "&[]",
            "{owner} builds a DOMAIN_PERF payload outside the emitters the scans read,              and hands it a trailer; a registered domain could publish counters there              that no census of the emitters would ever see"
        );
    }
    // COUNTED COMPLETENESS, not a rule about what a spelling looks like.
    //
    // Four rounds of review each named another way to write a record type that
    // a shape test would accept -- a second `const`, another visibility, a
    // qualified path, an uppercase binding, an import renamed past the check.
    // The set of spellings the collector actually uses is finite and small, so
    // it is PINNED: any new one, whatever it looks like, changes this set and
    // has to be added here in a diff a reader sees. That is what terminates the
    // family, rather than recognising one more spelling (owner ruling
    // 2026-08-29: close the class, do not run another round).
    const RECORD_TYPE_ARGUMENTS: &[(&str, usize, &str)] = &[
        ("COMPACT_RECORD_TYPE_DOMAIN_DATA", 85, "a domain's data record"),
        ("COMPACT_RECORD_TYPE_DOMAIN_PERF", 1, "THE PERF RECORD, whose writers are pinned below"),
        ("COMPACT_RECORD_TYPE_FRAME_END", 1, "the frame terminator"),
        ("COMPACT_RECORD_TYPE_STREAM_HEADER", 1, "the stream's own header"),
        ("compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP", 8, "gap ledger rows including unfunded D22 publication"),
        ("compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS", 2, "the freeze census"),
        (
            "compact_contract::COMPACT_RECORD_TYPE_CONTENT_IDENTITY_CENSUS",
            2,
            "the content-identity census",
        ),
        (
            "compact_contract::COMPACT_RECORD_TYPE_STATE_TOPOLOGY_CENSUS",
            2,
            "the state-topology census",
        ),
        ("compact_contract::COMPACT_RECORD_TYPE_FRAME_DEGRADED", 1, "a degraded frame"),
        ("compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED", 1, "a refused frame"),
        (
            "compact_contract::COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL",
            1,
            "an interval the clock could not be read across",
        ),
        (
            "compact_contract::COMPACT_RECORD_TYPE_UNTRUSTED_CLOCK_RECOVERY",
            1,
            "the clock coming back",
        ),
        ("compact_contract::COMPACT_RECORD_TYPE_CAPTURE_TERMINATED", 1, "the capture ending"),
        (
            "header.record_type",
            3,
            "a record being rewritten carries the type it was decoded with;              no_registered_domain_is_routed_through_a_record_rebuild holds those paths",
        ),
        (
            "record.record_type",
            1,
            "the same, for a record rebuilt from a decoded struct rather than a header",
        ),
    ];
    // AND THE NAME IS ONLY EVER A CALL. `let write = append_compact_record;`
    // publishes through a name no argument census searches for, and the counts
    // below would not move (Codex VB2 round 11, finding 6).
    {
        let collapsed = without_comments_or_layout(&text);
        let mut search = 0usize;
        while let Some(at) = collapsed[search..].find("append_compact_record") {
            let start = search + at;
            search = start + "append_compact_record".len();
            let before = collapsed[..start].chars().next_back();
            if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
                continue;
            }
            let after = &collapsed[search..];
            if after.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_') {
                continue;
            }
            assert!(
                collapsed[..start].ends_with("fn") || after.starts_with('('),
                "the raw appender is mentioned without being called, which lets a record                  be written under a name no census of its arguments reads: {}",
                &after[..after.len().min(60)]
            );
        }
    }
    // COUNTED, not merely enumerated: a SET is unchanged by a SECOND call under
    // a spelling already in it, and a second call is a second record. The count
    // per spelling is pinned, so a new writer of any kind fails here.
    let mut written: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, args) in call_arguments(&text, "append_compact_record") {
        let arguments = top_level_arguments(args);
        let Some(record_type) = arguments.get(4) else {
            panic!("an append_compact_record call has fewer arguments than the appender takes");
        };
        *written.entry(record_type.trim()).or_default() += 1;
    }
    assert_eq!(
        written,
        RECORD_TYPE_ARGUMENTS
            .iter()
            .map(|(spelling, calls, _)| (*spelling, *calls))
            .collect(),
        "the collector writes a record type under a spelling this census does not          account for, writes one of them a different number of times, or the list          names one it no longer writes"
    );
    for (spelling, _, why) in RECORD_TYPE_ARGUMENTS {
        assert!(why.len() > 12, "{spelling} is listed with no reason");
    }
    // The second net, one level out: the record type itself. A hand-rolled
    // payload would not call the builder at all, so the functions that build a
    // DOMAIN_PERF record are pinned to this list.
    // WRITES ONLY. `header.record_type == RECORD_TYPE` and a match arm on it
    // read a record someone else wrote; the two spellings that WRITE one are a
    // `record_type:` field and the type argument of the appender.
    let appends: Vec<(usize, usize)> = call_arguments(&text, "append_compact_record")
        .into_iter()
        .map(|(at, args)| (at, at + args.len() + "append_compact_record(".len()))
        .collect();
    // EVERY SPELLING of the record type, resolved through the collector's own
    // `const` declarations, so a second name for it is the same record here.
    let spellings = domain_spellings(&text, RECORD_TYPE);
    let constructors: BTreeSet<String> = spellings
        .iter()
        .flat_map(|spelling| text.match_indices(spelling.as_str()))
        .filter(|(at, _)| {
            let before: String = text[..*at].split_whitespace().collect();
            before.ends_with("record_type:")
                || appends.iter().any(|(from, to)| at > from && at < to)
        })
        .map(|(at, _)| enclosing_fn(&text, at))
        .filter(|owner| {
            !DIAGNOSTIC_EMITTERS.contains(&owner.as_str()) && owner != "append_compact_record"
        })
        .collect();
    assert_eq!(
        constructors,
        BTreeSet::from(["country_bundle_publication_records".to_string()]),
        "a function outside the emitters writes a DOMAIN_PERF record; if it can carry a          trailer, a registered domain publishes counters through a door no scan reads"
    );
}

/// Every `macro_rules!` in `text`, as (name, a digest of the whole macro).
///
/// The digest is FNV-1a over the whitespace-collapsed body. It is not a
/// cryptographic claim; it is a pin, and its only job is that any edit changes
/// it and has to be re-pinned by someone who read the edit.
fn collector_macros(relative: &str, text: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(at) = text[search..].find("macro_rules") {
        let start = search + at;
        search = start + "macro_rules".len();
        let rest = text[search..].trim_start().trim_start_matches('!').trim_start();
        let name: String = rest
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        let Some(brace) = text[start..].find('{') else {
            continue;
        };
        let body = balanced(text, start + brace + 1, b'{', b'}');
        let collapsed: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in collapsed.bytes() {
            digest ^= u64::from(byte);
            digest = digest.wrapping_mul(0x100_0000_01b3);
        }
        out.push((format!("{relative}::{name}"), digest));
    }
    out
}

/// The COLLECTOR brings in production source the closure can follow.
///
/// The registered crates are held to a stricter rule -- no macro at all -- which
/// the collector cannot meet: it defines and invokes plenty. What it must meet
/// is that none of them, and no other form, can introduce a module this census
/// never opens (Codex VB2 round 7, finding 4). The collector emits every
/// registered domain, so a production file outside the closure is a file the
/// DOMAIN_PERF census does not read.
///
/// MUTATION: `include!` a file, put a `path` inside a `cfg_attr`, write a `mod`
/// declaration inside a macro body, or declare a module in a shape the closure
/// does not read, and this fails.
#[test]
fn the_collector_brings_in_production_source_only_the_closure_can_follow() {
    let mut observed_macros: Vec<String> = Vec::new();
    for (relative, source) in production_closure(COLLECTOR_CRATE_ROOT) {
        let text = without_comments_or_strings(&source);
        let collapsed = without_comments_or_layout(&text);
        assert!(
            !collapsed.contains("include!"),
            "{relative} uses include!, which pastes source in with no module of its own,              so this census cannot know what production source the collector really has"
        );
        // A `cfg_attr` is refused only when it CARRIES A PATH: the collector
        // uses one to switch a lint, and a lint introduces no source.
        let mut search = 0usize;
        while let Some(at) = collapsed[search..].find("cfg_attr(") {
            let start = search + at + "cfg_attr(".len();
            search = start;
            let carried = balanced(&collapsed, start, b'(', b')');
            assert!(
                !carried.contains("path"),
                "{relative} puts a path inside a cfg_attr, which the line-shaped reader                  does not see: {carried}"
            );
        }
        // A MACRO BODY THAT DECLARES A MODULE. The collector's macros expand to
        // impls, enums and tables; a `mod` among them would be a file no reader
        // of these declarations opens.
        let mut search = 0usize;
        while let Some(at) = text[search..].find("macro_rules") {
            let start = search + at;
            search = start + "macro_rules".len();
            let Some(brace) = text[start..].find('{') else {
                break;
            };
            let body = balanced(&text, start + brace + 1, b'{', b'}');
            // NO `mod` TOKEN AT ALL. A macro can name its module with a
            // metavariable, spell it across lines, or paste it together, and a
            // reader looking for a declaration SHAPE will always have one more
            // spelling to miss. The collector's macros expand to impls, enums
            // and tables and need the token for nothing, so the token itself is
            // refused -- which is a closed rule rather than another spelling
            // (owner ruling 2026-08-29: close the class).
            // NO FILE MODULE, by what FOLLOWS the token rather than by the
            // name in between. A macro can name its module with a metavariable,
            // spell it across lines or paste it together, and a reader matching
            // a declaration SHAPE will always have one more spelling to miss.
            // What separates the two kinds is unambiguous: an INLINE module
            // reaches `{` first and its body is in this file, a FILE module
            // reaches `;` first and its file is one nothing here opens. The
            // collector declares inline modules inside a table macro; it may
            // not declare a file one (owner ruling 2026-08-29: close the class).
            // THE EXPANSIONS, not the matchers. A matcher names the macro's own
            // arguments -- `mod $module:ident, ..;` is a pattern, and its
            // semicolon ends the pattern rather than a declaration. What a macro
            // EXPANDS TO is what introduces source.
            let whole: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
            let mut expansions = String::new();
            let mut arm = 0usize;
            while let Some(found) = whole[arm..].find("=>") {
                let start = arm + found + "=>".len();
                // ANY DELIMITER. A transcriber may be written `=> ( .. )` or
                // `=> [ .. ]` as well as `=> { .. }`, and a reader that knew
                // only the brace would collect no expansion at all and pass.
                let tail = whole[start..].trim_start();
                let open = whole.len() - tail.len();
                let Some((opener, closer)) = tail
                    .chars()
                    .next()
                    .and_then(|ch| match ch {
                        '{' => Some((b'{', b'}')),
                        '(' => Some((b'(', b')')),
                        '[' => Some((b'[', b']')),
                        _ => None,
                    })
                else {
                    arm = start + "=>".len();
                    continue;
                };
                let expansion = balanced(&whole, open + 1, opener, closer);
                arm = open + 1 + expansion.len();
                expansions.push_str(expansion);
                expansions.push(' ');
            }
            let body = expansions;
            let mut at = 0usize;
            while let Some(found) = body[at..].find("mod") {
                let start = at + found;
                at = start + "mod".len();
                let before = body[..start].chars().next_back();
                if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
                    continue;
                }
                let after = &body[at..];
                if after.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_') {
                    continue;
                }
                let brace = after.find('{').unwrap_or(usize::MAX);
                let semicolon = after.find(';').unwrap_or(usize::MAX);
                assert!(
                    semicolon == usize::MAX || brace < semicolon,
                    "{relative} declares a FILE module inside a macro body; whatever                      supplies its name, no reader of module declarations opens that file"
                );
            }
        }
        // AND EVERY MACRO IS PINNED, BODY AND ALL.
        //
        // A macro need not spell `mod` to declare a module: a `tt` argument
        // supplies the keyword, `($kind:tt, $name:ident) => ($kind $name;)`
        // expands to one, and its transcriber holds no `mod` for any reader to
        // find. That is the fourth spelling of the same defect in as many
        // rounds, so the macros themselves are pinned: a name and a digest of
        // the whole macro. A new macro, or one word changed in an existing one,
        // fails here and has to be re-pinned deliberately, in a diff a reader
        // sees (owner ruling 2026-08-29: close the class by counted, asserted
        // completeness).
        for (name, digest) in collector_macros(&relative, &text) {
            observed_macros.push(format!("{name} {digest:016x}"));
        }

        // A MACRO'S ARGUMENTS ARE NOT ITEMS. The collector has a table macro
        // whose arguments begin `mod <name>,`, and that is a name in a list, not
        // a module declaration -- so the shape check reads the file's own items
        // and the bodies of its inline modules, and nothing inside a macro call.
        let mut opened_by_a_macro: Vec<bool> = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            let inside_a_macro = opened_by_a_macro.iter().any(|opened| *opened);
            let head = trimmed.split_once('{').map(|(head, _)| head).unwrap_or(trimmed);
            let opens_a_macro = head.contains('!');
            for byte in trimmed.bytes() {
                match byte {
                    b'{' => opened_by_a_macro.push(opens_a_macro),
                    b'}' => {
                        opened_by_a_macro.pop();
                    }
                    _ => {}
                }
            }
            if inside_a_macro {
                continue;
            }
            let bare = without_visibility(without_attributes(trimmed));
            let Some(rest) = bare.strip_prefix("mod ") else {
                continue;
            };
            assert!(
                rest.trim_end().ends_with(';') || rest.contains('{'),
                "{relative} declares a module in a shape the closure does not read: {trimmed}"
            );
        }
    }
    observed_macros.sort();
    const COLLECTOR_MACROS: &[&str] = &[
        "collector/src/attach_log.rs::carry_over_surviving_tracker 4ecd8c79a21a10f1",
        "collector/src/attach_log.rs::commit_one_country_tracker f30293bce5ac6398",
        "collector/src/attach_log.rs::d86_refusal_code_names 44a616ebc421f4f0",
        "collector/src/attach_log.rs::declare_country_child_bracket_tags f4233d35466e3e89",
        "collector/src/attach_log.rs::declare_country_scoped_tracker_table 357d0386567ae532",
        "collector/src/attach_log.rs::declare_domain_scoped_tracker_table 00346ea173170d3c",
        "collector/src/attach_log.rs::declare_journalled_tracker_table c99361d383244a9b",
        "collector/src/attach_log.rs::domain47_wire_code_enum ae801aeb8ad250e1",
        "collector/src/attach_log.rs::fail_early_render_startup 7741a71b07138f7f",
        "collector/src/attach_log.rs::for_each_country_scoped_tracker d012121803dfefcd",
        "collector/src/attach_log.rs::for_each_domain_scoped_tracker c0f5e5b6c623024e",
        "collector/src/attach_log.rs::for_each_journalled_baseline_tracker 992361adeeb75352",
        "collector/src/attach_log.rs::pair_fail 4adb4e29be243ce3",
        "collector/src/attach_log.rs::read_or_unreadable 882cbc700abc610d",
        "collector/src/attach_log.rs::reinstall_country_tracker 673d5af46aadc8ff",
        "collector/src/attach_log.rs::reinstall_journalled_tracker a7d9fb89b4f7e38e",
        "collector/src/attach_log.rs::reset_campaign_scoped_capture_state 21d040fe21809d18",
        "collector/src/attach_log.rs::restore_journalled_tracker_after_refusal bf184d80febaa936",
        "collector/src/attach_log.rs::restore_one_country_tracker 10a3ba182a404ac4",
        "collector/src/attach_log.rs::restore_or_carry_journalled_tracker 1efc01d30d752b77",
        "collector/src/attach_log.rs::roll_back_or_carry_country 9c738e86215255f5",
        "collector/src/attach_log.rs::settle_journalled_tracker 51d8e1786c9b717f",
        "collector/src/attach_log.rs::slot_fail 0ae04709a82efb8f",
        "collector/src/attach_log.rs::slot_fail a1ce4fd88dda2dcb",
        "collector/src/attach_log.rs::variant_list_enum 33851ca19d58f0c5",
        "collector/src/attach_log/d81_vector_store.rs::d81_rail_tap_outcomes 1062bfb6f60318b0",
        "collector/src/attach_log/ordinal_taxonomy.rs::d81_coded_unit_names ef32258b6d4cfb0a",
        "collector/src/attach_log/ordinal_taxonomy.rs::d81_indexed_unit_names eb8b06605b2c65a4",
        "collector/src/attach_log/ordinal_taxonomy.rs::d81_plan_ordinal_bins 51300e6139eef5e9",
        "collector/src/attach_log/ordinal_taxonomy.rs::numbered_cause_bins 24d8dd793cd03560",
        "collector/src/attach_log/ordinal_taxonomy.rs::ordinal_named_cause_bins 6f2498ec5d2fb034",
        "collector/src/snapshot_recipes/executor.rs::carry 1a3669c2865f5f51",
    ];
    assert_eq!(
        observed_macros,
        COLLECTOR_MACROS,
        "the collector's macros are not the ones this census accounts for. A macro \
         can declare a module without spelling `mod` - a `tt` argument supplies the \
         keyword - so each one is pinned whole; read the change and re-pin it."
    );
}

/// The module closure reads every form the registered crates actually use.
///
/// `source_closure` recognises `mod name;` and `#[path = ".."] mod name;`, each
/// on its own line, and inline `mod name {`. Rust has more ways to bring source
/// into a crate, and a form the parser did not model would be a file the
/// backstop never reads -- silently. This FAILS CLOSED on every one of them
/// instead: the forms below are refused outright, and a `mod name;` whose file
/// the parser cannot find is a failure rather than a shrug (Codex round 4,
/// finding 5).
///
/// MUTATION: add `include!("accounting.rs");` or
/// `#[cfg_attr(windows, path = "..")] mod accounting;` to a registered crate,
/// or put an attribute and its module on one line, and this fails.
#[test]
fn a_registered_crate_brings_in_source_only_the_closure_can_follow() {
    for producer in PRODUCERS {
        let mut files: BTreeSet<String> = crate_closure(producer.source_file);
        files.extend(
            source_closure(producer.source_file)
                .into_iter()
                .map(|(path, _)| path),
        );
        for relative in files {
            let text = read(&relative);
            let text = without_comments_or_strings(&text);
            let collapsed = without_comments_or_layout(&text);
            for (form, why) in [
                // `include_str!` pastes a STRING, which declares nothing.
                ("include!", "pastes source in with no module of its own"),
                ("cfg_attr", "can carry a `path` the line-shaped reader does not see"),
            ] {
                assert!(
                    // WHITESPACE OUT: `include !( .. )` is the same item.
                    !collapsed.contains(form),
                    "{relative} uses {form}, which {why}, so the declaration backstop \
                     cannot know what source the crate really has"
                );
            }
            // A MACRO CAN DECLARE A MODULE. `macro_rules! hidden { () => { mod
            // accounting; } } hidden!();` is a real module neither the closure
            // nor the module check above can see (Codex VB2 round 6, finding 4).
            // A registered crate defines no macro, and the only item-position
            // invocations it makes are the ones named here -- each of which
            // expands to a struct and an impl, not to a module.
            // `wire_counters!` expands to a struct and its impl,
            // `wire_coded_codes!` to an impl, and `thread_local!` wraps `static`
            // declarations the backstop reads where they stand. None of the
            // three can introduce a module. `macro_rules` is not among them, so
            // a registered crate defines no macro either.
            const ITEM_MACROS: &[&str] = &[
                "compact_contract::wire_counters",
                "compact_contract::wire_coded_codes",
                "thread_local",
            ];
            // THE RAW SOURCE: `text` here has already had its strings blanked,
            // and blanking twice eats from an unmatched closing quote onward.
            for invocation in item_macro_invocations(&read(&relative)) {
                assert!(
                    ITEM_MACROS.contains(&invocation.as_str()),
                    "{relative} invokes {invocation}! at item level, and a macro there can                      declare a module the closure never reads"
                );
            }
            let mut depth = 0usize;
            for line in text.lines() {
                let trimmed = line.trim();
                let opening_depth = depth;
                for byte in trimmed.bytes() {
                    match byte {
                        b'{' => depth += 1,
                        b'}' => depth = depth.saturating_sub(1),
                        _ => {}
                    }
                }
                let _ = opening_depth;
                let bare = without_visibility(without_attributes(trimmed));
                let Some(rest) = bare.strip_prefix("mod ") else {
                    continue;
                };
                // Every module declaration is one of the two shapes the closure
                // reads: a file module ending in `;`, or an inline one opening a
                // brace on the same line.
                assert!(
                    rest.trim_end().ends_with(';') || rest.contains('{'),
                    "{relative} declares a module in a shape source_closure does not \
                     read: {trimmed}"
                );
                if let Some(name) = rest.trim_end().strip_suffix(';') {
                    assert!(
                        name.trim()
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
                        "{relative} declares a module whose name the closure cannot \
                         resolve: {trimmed}"
                    );
                }
            }
            let _ = depth;
        }
    }
}
