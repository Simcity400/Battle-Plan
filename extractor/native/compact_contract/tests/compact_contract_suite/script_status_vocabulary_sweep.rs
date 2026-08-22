//! It is deliberately a PARSE of the scripts rather than a generated header

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compact_contract::{
    assigned_descriptor_by_name, domain_status_registry_key, lane_local_status_mask,
    CompactDomainStatus, ASSIGNED_COMPACT_DOMAINS, COMPACT_STATUS_FAILURE_MASK,
    COMPACT_STATUS_V1_KNOWN_MASK,
};

fn scripts_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts")
}

/// kills: lane QI console-window defect class. The powershell probes below
/// must remain real child processes (they exercise real exit codes), but a
/// console-subsystem child of a console-less harness allocates a visible
/// terminal window per call; CREATE_NO_WINDOW suppresses that allocation. The
/// adapter keeps the suppression attached to every spawn in this file.
trait NoConsoleWindow {
    fn no_console_window(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl NoConsoleWindow for std::process::Command {
    fn no_console_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(not(windows))]
impl NoConsoleWindow for std::process::Command {
    fn no_console_window(&mut self) -> &mut Self {
        self
    }
}

fn read_script(name: &str) -> String {
    let path = scripts_root().join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn read_code(name: &str) -> String {
    strip_non_code_in(&read_script(name), dialect_of(name))
}

fn registered_domains() -> BTreeMap<u16, u32> {
    (0..=u16::MAX)
        .filter_map(|domain| {
            let mask = lane_local_status_mask(domain);
            (mask != 0).then_some((domain, mask))
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dialect {
    Python,
    PowerShell,
}

fn dialect_of(script_name: &str) -> Dialect {
    if script_name.ends_with(".ps1") || script_name.ends_with(".psm1") {
        Dialect::PowerShell
    } else {
        Dialect::Python
    }
}

fn strip_non_code(text: &str) -> String {
    strip_non_code_in(text, Dialect::Python)
}

fn strip_non_code_in(text: &str, dialect: Dialect) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;

    let blank = |out: &mut String, from: usize, to: usize| {
        for ch in &chars[from..to] {
            if *ch == '\n' {
                out.push('\n');
            } else {
                for _ in 0..ch.len_utf8() {
                    out.push(' ');
                }
            }
        }
    };

    while index < chars.len() {
        let ch = chars[index];

        if dialect == Dialect::PowerShell && ch == '<' && chars.get(index + 1) == Some(&'#') {
            let mut end = index + 2;
            let mut closed = false;
            while end < chars.len() {
                if chars[end] == '#' && chars.get(end + 1) == Some(&'>') {
                    closed = true;
                    break;
                }
                end += 1;
            }
            assert!(
                closed,
                "unterminated `<# ... #>` block comment at character {index}; the \
                 script does not parse, and this sweep refuses to report on a \
                 file it cannot read"
            );
            let end = end + 2;
            blank(&mut out, index, end);
            index = end;
            continue;
        }

        if dialect == Dialect::PowerShell
            && ch == '@'
            && matches!(chars.get(index + 1), Some('"') | Some('\''))
        {
            let quote = chars[index + 1];
            let mut end = index + 2;
            let mut closed = false;
            while end < chars.len() {
                // The terminator is only a terminator at the start of a line.
                let at_line_start = end == 0 || chars[end - 1] == '\n';
                if at_line_start && chars[end] == quote && chars.get(end + 1) == Some(&'@') {
                    closed = true;
                    break;
                }
                end += 1;
            }
            assert!(
                closed,
                "unterminated here-string at character {index}; the script does \
                 not parse, and this sweep refuses to report on a file it \
                 cannot read"
            );
            if quote == '"' {
                let body = &chars[index + 2..end];
                let live_subexpression = body.windows(2).enumerate().any(|(at, pair)| {
                    if pair != ['$', '('] {
                        return false;
                    }
                    let backticks = body[..at].iter().rev().take_while(|c| **c == '`').count();
                    backticks % 2 == 0
                });
                assert!(
                    !live_subexpression,
                    "double-quoted here-string at character {index} contains an \
                     unescaped `$( ... )` subexpression, which is EXECUTABLE code. \
                     Blanking it would hide live code from this sweep. Parse the \
                     subexpression or move it out of the here-string."
                );
            }
            let end = end + 2;
            blank(&mut out, index, end);
            index = end;
            continue;
        }

        if ch == '#' {
            let mut end = index;
            while end < chars.len() && chars[end] != '\n' {
                end += 1;
            }
            blank(&mut out, index, end);
            index = end;
            continue;
        }

        if ch == '"' || ch == '\'' {
            // An `f` prefix makes the string EXECUTABLE: its `{ ... }`
            // replacement fields are expressions. `c0_pass2_spans.py` really
            // has `f'''  RUN VERDICT: {'RECONCILED' if reconciled else ...}'''`.
            // So an f-prefixed triple-quoted string is NOT the inert
            // commented-out-code form and must not be blanked; it falls through
            // to the ordinary-string path, which keeps it verbatim. Only a plain
            // triple-quoted block is inert, and only that is blanked.
            //
            // The WHOLE prefix token is read, not just the character before the
            // quote: `rf'''` and `fr'''` are both valid Python and both
            // executable, and looking at one character sees `r` in one case and
            // an alphanumeric neighbour in the other.
            let prefix: String = chars[..index]
                .iter()
                .rev()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            let prefix_is_a_token = prefix.len() == chars[..index].len()
                || !chars[index - prefix.len() - 1].is_alphanumeric()
                    && chars[index - prefix.len() - 1] != '_';
            let f_prefixed = prefix_is_a_token
                && prefix.len() <= 2
                && prefix.chars().any(|c| c == 'f' || c == 'F');

            // Triple-quoted Python block: inert, and the idiomatic way to
            // comment a block out. PowerShell has no such form.
            if dialect == Dialect::Python
                && !f_prefixed
                && chars.get(index + 1) == Some(&ch)
                && chars.get(index + 2) == Some(&ch)
            {
                let mut end = index + 3;
                let mut closed = false;
                let mut escaped = false;
                while end < chars.len() {
                    let c = chars[end];
                    if escaped {
                        // A backslash-escaped delimiter does NOT close the
                        // block; treating it as a terminator would expose the
                        // rest of an inert region as live code.
                        escaped = false;
                        end += 1;
                        continue;
                    }
                    if c == '\\' {
                        escaped = true;
                        end += 1;
                        continue;
                    }
                    if c == ch && chars.get(end + 1) == Some(&ch) && chars.get(end + 2) == Some(&ch)
                    {
                        closed = true;
                        break;
                    }
                    end += 1;
                }
                assert!(
                    closed,
                    "unterminated triple-quoted block at character {index}; the \
                     script does not parse, and this sweep refuses to report on \
                     a file it cannot read"
                );
                let end = end + 3;
                blank(&mut out, index, end);
                index = end;
                continue;
            }

            // Ordinary string: kept VERBATIM, because the badge names live in
            // these and the downstream parsers rely on seeing them. Only the
            // EXTENT matters, and the escape rule is the DIALECT'S:
            //
            //   * Python escapes with a backslash, in both quote styles;
            //   * PowerShell escapes with a backtick, and ONLY inside a
            //     double-quoted string. A backslash there is an ordinary
            //     character -- `TrimEnd('\')` is real code in this repo.
            //
            // A doubled-quote rule (`''`/`""`) is deliberately absent: dropping
            // it only re-pairs one string into two adjacent ones, which shifts
            // the in/out partition by a whole quote pair and so preserves
            // parity -- every `#` stays on the side it was on. Verified by
            // exhaustive search over every string of length <= 8 in
            // `' " # a \n ;`: 1,963,046 inputs, zero differences.
            let escape = match (dialect, ch) {
                (Dialect::Python, _) => Some('\\'),
                (Dialect::PowerShell, '"') => Some('`'),
                (Dialect::PowerShell, _) => None,
            };
            out.push(ch);
            index += 1;
            while index < chars.len() {
                let c = chars[index];
                if Some(c) == escape {
                    out.push(c);
                    index += 1;
                    if index < chars.len() {
                        out.push(chars[index]);
                        index += 1;
                    }
                    continue;
                }
                out.push(c);
                index += 1;
                if c == ch || c == '\n' {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
        index += 1;
    }
    out
}

/// Byte offset of `marker`, requiring it to occur EXACTLY once.
///
/// A marker that matches twice would silently bind the parser to whichever
/// copy came first, which is how a "checked" table quietly stops being the
/// table the runtime reads.
fn unique_marker(text: &str, marker: &str) -> usize {
    let count = text.matches(marker).count();
    assert_eq!(
        count, 1,
        "marker {marker:?} must occur exactly once in the script, found {count}"
    );
    text.find(marker).expect("counted above")
}

/// The delimited value that follows `marker`, exclusive of the outermost pair.
/// `text` must already be comment-stripped.
///
/// The opening delimiter must be the FIRST non-whitespace thing after the
/// marker (PowerShell's `@{` is allowed). Searching forward for the next `{`
/// would happily parse a later decoy: `LANE_LOCAL_BITS = OTHER` followed
/// anywhere below by an unrelated brace literal holding the expected registry
/// would pass this sweep while the runtime used `OTHER`.
fn delimited_after(text: &str, marker: &str, open_ch: char, close_ch: char) -> String {
    let start = unique_marker(text, marker);
    let rest = &text[start + marker.len()..];
    let head = rest.trim_start();
    let head = head.strip_prefix('@').unwrap_or(head); // PowerShell `@{`
    assert!(
        head.starts_with(open_ch),
        "the value assigned at {marker:?} does not open with {open_ch:?}; this \
         sweep only reads a literal written in place, and refuses to search \
         forward for one that may not be the value the runtime uses. Found: {:?}",
        &head[..head.len().min(60)]
    );
    let open = rest.find(open_ch).expect("asserted above");
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut out = String::new();
    for ch in rest[open..].chars() {
        if let Some(q) = quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            out.push(ch);
            continue;
        }
        if ch == open_ch {
            depth += 1;
            if depth > 1 {
                out.push(ch);
            }
            continue;
        }
        if ch == close_ch {
            depth -= 1;
            if depth == 0 {
                return out;
            }
            out.push(ch);
            continue;
        }
        out.push(ch);
    }
    panic!("unterminated {open_ch:?}-delimited value after {marker:?}");
}

fn block_after(text: &str, marker: &str) -> String {
    delimited_after(text, marker, '{', '}')
}

fn paren_expr_after(text: &str, marker: &str) -> String {
    delimited_after(text, marker, '(', ')')
}

fn squeeze(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn top_level_elements(block: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in block.chars() {
        if let Some(q) = quote {
            current.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                current.push(ch);
            }
            '{' | '[' | '(' => {
                depth += 1;
                current.push(ch);
            }
            '}' | ']' | ')' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    out.push(current);
    assert!(quote.is_none(), "unterminated string literal in {block:?}");
    out.into_iter()
        .filter(|element| !element.trim().is_empty())
        .collect()
}

/// Top-level `<key>: { <inner> }` entries of a Python dict literal.
///
/// FAIL-CLOSED: every top-level element must have this exact shape. An element
/// whose value is not a brace literal, or which has no `:` at all, panics
/// rather than being skipped.
fn nested_entries(block: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for element in top_level_elements(block) {
        let colon = top_level_colon(&element)
            .unwrap_or_else(|| panic!("dict element has no top-level `:`: {element:?}"));
        let (key, value) = element.split_at(colon);
        let value = value[1..].trim();
        assert!(
            value.starts_with('{') && value.ends_with('}'),
            "dict element value is not a brace literal (this sweep only \
             understands `<key>: {{...}}`, and refuses to skip what it cannot \
             read): {element:?}"
        );
        out.push((
            key.trim().to_string(),
            value[1..value.len() - 1].to_string(),
        ));
    }
    out
}

/// Byte offset of the first `:` at nesting depth 0 and outside a string.
fn top_level_colon(element: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (offset, ch) in element.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            ':' if depth == 0 => return Some(offset),
            _ => {}
        }
    }
    None
}

fn int_keys(block: &str) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    for element in top_level_elements(block) {
        let colon = top_level_colon(&element)
            .unwrap_or_else(|| panic!("dict element has no top-level `:`: {element:?}"));
        let key = element[..colon].trim();
        let bit: u32 = key.parse().unwrap_or_else(|_| {
            panic!(
                "status-bit key {key:?} is not a plain integer literal; this \
                 sweep refuses to guess what the runtime would compute"
            )
        });
        assert!(
            bit < 32,
            "status-bit key {bit} is outside a u32 status word"
        );
        assert!(out.insert(bit), "status-bit key {bit} is declared twice");
    }
    out
}

fn mask_of(bits: &BTreeSet<u32>) -> u32 {
    bits.iter().fold(0u32, |mask, bit| mask | (1u32 << bit))
}

fn assert_code_contains_once(text: &str, dialect: Dialect, needle: &str, what: &str) {
    let count = strip_non_code_in(text, dialect).matches(needle).count();
    assert_eq!(
        count, 1,
        "{what}: expected exactly one executable occurrence of {needle:?}, found \
         {count} (0 = the line is gone, or survives only in a comment; >1 = a \
         later assignment shadows the checked one)"
    );
}

fn assert_squeezed_code_contains_once(text: &str, dialect: Dialect, needle: &str, what: &str) {
    let count = squeeze(&strip_non_code_in(text, dialect))
        .matches(needle)
        .count();
    assert_eq!(
        count, 1,
        "{what}: expected exactly one executable occurrence of {needle:?}, found {count}"
    );
}

fn check_table(label: &str, observed: &BTreeMap<u16, u32>) {
    let expected = registered_domains();
    assert_eq!(
        observed, &expected,
        "{label} disagrees with compact_contract::lane_local_status_mask.\n  \
         script:   {observed:#x?}\n  contract: {expected:#x?}\n  \
         A bit missing here is not a labelling nit: every one of these \
         instruments treats an unregistered high bit as an undefined status \
         bit and BLOCKS the capture for it."
    );
}

#[test]
fn report_raw_capture_lane_local_table_matches_the_contract() {
    let text = read_code("report_raw_capture.py");
    let block = block_after(&text, "LANE_LOCAL_BITS = ");
    let mut observed = BTreeMap::new();
    for (key, inner) in nested_entries(&block) {
        let domain: u16 = key
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("non-numeric domain key {key:?}"));
        observed.insert(domain, mask_of(&int_keys(&inner)));
    }
    check_table("report_raw_capture.py LANE_LOCAL_BITS", &observed);
}

#[test]
fn report_raw_capture_global_masks_match_the_contract() {
    let text = read_script("report_raw_capture.py");
    let stripped = strip_non_code_in(&text, Dialect::Python);
    let known_shift = COMPACT_STATUS_V1_KNOWN_MASK.count_ones();
    assert_code_contains_once(
        &text,
        Dialect::Python,
        &format!("KNOWN_MASK = (1 << {known_shift}) - 1"),
        &format!(
            "report_raw_capture.py KNOWN_MASK must be the contract's \
             COMPACT_STATUS_V1_KNOWN_MASK ({COMPACT_STATUS_V1_KNOWN_MASK:#x})"
        ),
    );
    assert_eq!(
        COMPACT_STATUS_V1_KNOWN_MASK,
        (1u32 << known_shift) - 1,
        "the assigned vocabulary is no longer a contiguous run from bit 0, so the \
         scripts' `(1 << N) - 1` idiom is wrong and this sweep is checking the wrong shape"
    );
    let failure_bits: BTreeSet<u32> = (0..32)
        .filter(|bit| COMPACT_STATUS_FAILURE_MASK & (1u32 << bit) != 0)
        .collect();
    let declared: BTreeSet<u32> = paren_expr_after(&stripped, "FAILURE_MASK = ")
        .split('|')
        .map(|term| term.trim().trim_matches(['(', ')']).trim().to_string())
        .map(|term| {
            let shift = term
                .strip_prefix("1 <<")
                .unwrap_or_else(|| panic!("FAILURE_MASK term {term:?} is not `1 << <bit>`"));
            shift
                .trim()
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("FAILURE_MASK term {term:?} has a non-literal shift"))
        })
        .collect();
    assert_eq!(
        declared, failure_bits,
        "report_raw_capture.py FAILURE_MASK is not compact_contract::COMPACT_STATUS_FAILURE_MASK \
         ({COMPACT_STATUS_FAILURE_MASK:#x})"
    );
}

/// `extractor/native/domain_registry.json`, the generated mirror this crate's
#[test]
fn verify_sink_reconciliation_admits_exactly_the_contract_gap_reasons() {
    use compact_contract::DomainGapReason;

    let code = read_code("verify_sink_reconciliation.py");
    let block = block_after(&code, "ALLOWED_GAP_REASONS = ");
    let declared: BTreeSet<String> = top_level_elements(&block)
        .into_iter()
        .map(|element| {
            let trimmed = element.trim().to_string();
            let unquoted = trimmed
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .unwrap_or_else(|| {
                    panic!("ALLOWED_GAP_REASONS element is not a plain string literal: {trimmed:?}")
                });
            unquoted.to_string()
        })
        .collect();
    let expected: BTreeSet<String> = DomainGapReason::ALL
        .iter()
        .map(|reason| reason.wire_name().to_string())
        .collect();
    assert_eq!(
        expected.len(),
        DomainGapReason::ALL.len(),
        "two reasons share a wire name, so the manifest cannot distinguish them"
    );
    assert_eq!(
        declared, expected,
        "verify_sink_reconciliation.py's ALLOWED_GAP_REASONS must be exactly the \
         contract's reason vocabulary; a manifest naming a reason it omits is \
         graded `domain_gap_N_reason_invalid` for being honest"
    );
}

#[test]
fn report_raw_capture_pseudo_domains_match_the_contract() {
    let text = read_script("report_raw_capture.py");

    let infrastructure: Vec<_> = ASSIGNED_COMPACT_DOMAINS
        .iter()
        .filter(|descriptor| descriptor.status == CompactDomainStatus::Infrastructure)
        .collect();
    assert!(
        !infrastructure.is_empty(),
        "the contract registers no infrastructure domain, so this sweep is \
         checking nothing"
    );
    for descriptor in &infrastructure {
        assert_eq!(
            descriptor.canonical_name, descriptor.wire_name,
            "infrastructure domain {} exports canonical name {:?} in \
             domain_registry.json but emits wire name {:?}. report_raw_capture.py \
             derives its pseudo-domain set from the mirror's `name` field and \
             matches it against `domains_seen`, which carries WIRE names, so this \
             domain would be reported as an unexpected producer on every capture. \
             Either keep the two equal or export `wire_name` in the mirror and \
             teach the script to read it.",
            descriptor.id, descriptor.canonical_name, descriptor.wire_name
        );
        assert!(
            !descriptor.schedulable,
            "infrastructure domain {} is schedulable, so it CAN appear in a \
             scheduled roster and the report must not blanket-exempt it",
            descriptor.id
        );
    }

    assert_code_contains_once(
        &text,
        Dialect::Python,
        &format!(
            "INFRASTRUCTURE_STATUS_KEY = \"{}\"",
            domain_status_registry_key(CompactDomainStatus::Infrastructure)
        ),
        "report_raw_capture.py must filter the registry on the contract's own \
         status key, or its derivation silently selects nothing and every \
         capture reports `frame` as unexpected again",
    );
    assert_squeezed_code_contains_once(
        &text,
        Dialect::Python,
        "os.path.dirname(os.path.abspath(__file__)), os.pardir, \"domain_registry.json\"",
        "report_raw_capture.py must resolve extractor/native/domain_registry.json \
         from its own location",
    );

    let declared: BTreeSet<String> = paren_expr_after(
        &strip_non_code_in(&text, Dialect::Python),
        "PSEUDO_DOMAIN_NAMES_FALLBACK = ",
    )
    .split(',')
    .map(str::trim)
    .filter(|term| !term.is_empty())
    .map(|term| {
        let name = term
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or_else(|| {
                panic!("PSEUDO_DOMAIN_NAMES_FALLBACK term {term:?} is not a double-quoted literal")
            });
        name.to_string()
    })
    .collect();
    let expected: BTreeSet<String> = infrastructure
        .iter()
        .map(|descriptor| descriptor.wire_name.to_string())
        .collect();
    assert_eq!(
        declared, expected,
        "report_raw_capture.py PSEUDO_DOMAIN_NAMES_FALLBACK is not the contract's \
         infrastructure wire-name set. A name missing here is not a labelling nit: \
         when domain_registry.json cannot be read, the report calls that deliberate \
         producer an unexpected domain on every capture."
    );
}

#[cfg(test)]
fn python_program() -> String {
    for candidate in ["python", "python3", "py"] {
        if std::process::Command::new(candidate)
            .arg("-c")
            .arg("pass")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
        {
            return candidate.to_string();
        }
    }
    panic!(
        "no python interpreter on PATH (tried python, python3, py). The gate \
         scripts this sweep guards cannot run here, so this check cannot be \
         reported as passing."
    );
}

#[test]
fn report_raw_capture_really_reads_the_registry_it_is_pointed_at() {
    let dir = std::env::temp_dir().join(format!(
        "hoi4_lane_et_registry_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).expect("create probe dir");
    let script = scripts_root().join("report_raw_capture.py");
    let program = python_program();

    let cases: [(&str, &str, &str, Option<&str>); 3] = [
        (
            "a name only the synthetic registry has is derived",
            r#"{"domains":[{"id":4242,"name":"sentinel_infra_domain","status":"infrastructure","schedulable":false,"requires_data_record":false}]}"#,
            "sentinel_infra_domain",
            None,
        ),
        (
            "a registry with no infrastructure entry does not claim registry authority",
            r#"{"domains":[{"id":10,"name":"fuel_live","status":"existing","schedulable":true,"requires_data_record":true}]}"#,
            "SOURCE=pinned fallback",
            Some("sentinel_infra_domain"),
        ),
        (
            "the real mirror derives exactly the contract's infrastructure set",
            "",
            "SOURCE_IS_REGISTRY=True",
            None,
        ),
    ];

    for (label, registry_json, wanted, forbidden) in cases {
        let registry = dir.join(format!("registry_{}.json", label.len()));
        let target = if registry_json.is_empty() {
            "None".to_string()
        } else {
            fs::write(&registry, registry_json).expect("write probe registry");
            format!("r'{}'", registry.display())
        };
        let code = format!(
            "import importlib.util\n\
             spec = importlib.util.spec_from_file_location('rrc', r'{script}')\n\
             module = importlib.util.module_from_spec(spec)\n\
             spec.loader.exec_module(module)\n\
             target = {target}\n\
             names, source = module.load_pseudo_domain_names(\n\
             \x20   *([] if target is None else [target]))\n\
             print('NAMES=' + ','.join(sorted(names)))\n\
             print('SOURCE=' + source)\n\
             print('SOURCE_IS_REGISTRY=' + str(source == module.DOMAIN_REGISTRY_PATH))\n",
            script = script.display(),
            target = target,
        );
        let output = std::process::Command::new(&program)
            .arg("-c")
            .arg(&code)
            .output()
            .unwrap_or_else(|error| panic!("run {program}: {error}"));
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            output.status.success(),
            "{label}: the probe did not run\nstdout: {stdout}\nstderr: {stderr}"
        );
        assert!(
            stdout.contains(wanted),
            "{label}: expected {wanted:?} in the derivation's answer\n{stdout}"
        );
        if let Some(forbidden) = forbidden {
            assert!(
                !stdout.contains(forbidden),
                "{label}: {forbidden:?} leaked into the answer\n{stdout}"
            );
        }
        for descriptor in ASSIGNED_COMPACT_DOMAINS
            .iter()
            .filter(|d| d.status == CompactDomainStatus::Infrastructure)
        {
            assert!(
                stdout.contains(descriptor.wire_name),
                "{label}: infrastructure domain {} ({}) is missing from the \
                 derived set\n{stdout}",
                descriptor.id,
                descriptor.wire_name
            );
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn c0_domain_census_lane_local_table_matches_the_contract() {
    let text = read_code("c0_domain_census.py");
    let block = block_after(&text, "LANE_LOCAL = ");
    let mut observed = BTreeMap::new();
    for (key, inner) in nested_entries(&block) {
        let name = key.trim().trim_matches(['"', '\'']);
        let descriptor = assigned_descriptor_by_name(name)
            .unwrap_or_else(|| panic!("c0_domain_census.py names unknown domain {name:?}"));
        observed.insert(descriptor.id, mask_of(&int_keys(&inner)));
    }
    check_table("c0_domain_census.py LANE_LOCAL", &observed);
}

#[test]
fn c0_pass2_spans_lane_local_table_matches_the_contract() {
    let raw = read_script("c0_pass2_spans.py");
    let text = strip_non_code_in(&raw, Dialect::Python);
    let block = block_after(
        &text,
        "DOMAIN_STATUS_BITS: Mapping[int, Mapping[int, StatusSpec]] = ",
    );
    let mut observed = BTreeMap::new();
    for (key, inner) in nested_entries(&block) {
        let domain: u16 = key
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("non-numeric domain key {key:?}"));
        observed.insert(domain, mask_of(&int_keys(&inner)));
    }
    check_table("c0_pass2_spans.py DOMAIN_STATUS_BITS", &observed);
    // The file's second table must be DERIVED from the first, not a second
    // hand-written registry: two local copies in one file drifted apart from
    // each other before they drifted from the contract.
    assert_squeezed_code_contains_once(
        &raw,
        Dialect::Python,
        "LANE_LOCAL_MASKS = { domain: sum(1 << bit for bit in bits) \
         for domain, bits in DOMAIN_STATUS_BITS.items() }",
        "c0_pass2_spans.py LANE_LOCAL_MASKS must be DERIVED from DOMAIN_STATUS_BITS",
    );
}

#[test]
fn production_promotion_lane_local_table_matches_the_contract() {
    let text = read_code("ProductionCapture.Promotion.ps1");
    let block = block_after(&text, "$script:ProductionPromotionLaneLocalStatusMask = ");
    let mut observed = BTreeMap::new();
    for line in block.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Fail closed: every non-blank line in this hashtable must be an entry
        // this sweep can read. Skipping the rest is how a second, unchecked
        // entry would ride along unseen.
        let (domain, value) = line.split_once('=').unwrap_or_else(|| {
            panic!("promotion lane-local hashtable line is not `<domain> = <mask>`: {line:?}")
        });
        let domain: u16 = domain
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("non-numeric domain key {domain:?}"));
        let hex = value
            .trim()
            .strip_prefix("[uint32]0x")
            .unwrap_or_else(|| panic!("promotion mask for domain {domain} is not [uint32]0x..."));
        let mask = u32::from_str_radix(hex.trim(), 16)
            .unwrap_or_else(|_| panic!("promotion mask for domain {domain} is not hex"));
        observed.insert(domain, mask);
    }
    check_table(
        "ProductionCapture.Promotion.ps1 ProductionPromotionLaneLocalStatusMask",
        &observed,
    );
}

#[test]
fn production_promotion_unknown_status_mask_is_the_contract_complement() {
    let text = read_script("ProductionCapture.Promotion.ps1");
    let expected = format!("{:08X}", !COMPACT_STATUS_V1_KNOWN_MASK);
    assert_code_contains_once(
        &text,
        Dialect::PowerShell,
        &format!("[uint32]::Parse('{expected}'"),
        &format!(
            "ProductionCapture.Promotion.ps1's unknown-status mask must be the \
             complement of COMPACT_STATUS_V1_KNOWN_MASK \
             ({COMPACT_STATUS_V1_KNOWN_MASK:#x})"
        ),
    );
    let expected_failure = format!("[uint32]0x{:X}", COMPACT_STATUS_FAILURE_MASK);
    assert_code_contains_once(
        &text,
        Dialect::PowerShell,
        &format!("$script:ProductionPromotionFailureMask = {expected_failure}"),
        "ProductionCapture.Promotion.ps1's failure mask must be COMPACT_STATUS_FAILURE_MASK",
    );
}

/// The promotion gate must test DEFINEDNESS on the globalized word. Testing it
/// on the raw word is what threw "lacks real-source completeness" for d53 and
/// d60, and it is a silent failure mode: the gate looks strict, and it is
/// strict about the wrong thing.
#[test]
fn production_promotion_tests_definedness_on_the_globalized_word() {
    // Comment-stripped throughout: a `contains` over raw source is satisfied by
    // the text sitting in a comment, and this file's comments quote both forms.
    let code = read_code("ProductionCapture.Promotion.ps1");
    assert!(
        !code.contains("($rawStatusFlags -band $script:ProductionPromotionUnknownStatusMask)"),
        "the promotion gate applies the unknown-status mask to the RAW per-domain status \
         word; registered lane-local badges must be stripped first"
    );
    // Two sites: the throwing check, and the completeness receipt it writes.
    assert_eq!(
        code.matches("($globalStatusFlags -band $script:ProductionPromotionUnknownStatusMask)")
            .count(),
        2,
        "the promotion gate must test the unknown-status mask against the GLOBALIZED \
         word at both its throwing check and its completeness receipt; a receipt that \
         disagrees with the check it accompanies is evidence of nothing"
    );
    assert_code_contains_once(
        &code,
        Dialect::PowerShell,
        "function Get-ProductionPromotionGlobalStatusFlags",
        "the promotion gate's globalization helper",
    );
}

// --- Parser regression tests -------------------------------------------------
//
// These pin the parsers against the ways a hand-written parser silently
// under-reads. Each one is a shape a real script edit could take that the
// RUNTIME would honour; if the parser skipped it instead of panicking, this
// sweep would keep reporting "table matches the contract" about a table it had
// only partly read.

#[test]
fn a_comment_cannot_truncate_a_block() {
    // `# }` inside the table: Python ignores it, and an unstripped scanner
    // would end the block there and never see bit 23.
    let source = "TABLE = {\n    53: {16: 'a'},  # }\n    60: {23: 'b'},\n}\n";
    let block = block_after(&strip_non_code(source), "TABLE = ");
    let entries = nested_entries(&block);
    assert_eq!(
        entries.len(),
        2,
        "the comment truncated the block: {entries:?}"
    );
    assert_eq!(mask_of(&int_keys(&entries[1].1)), 1 << 23);
}

#[test]
fn a_comment_cannot_truncate_a_paren_expression() {
    let source = "MASK = (\n    (1 << 0)  # )\n    | (1 << 9)\n)\n";
    let expr = paren_expr_after(&strip_non_code(source), "MASK = ");
    assert_eq!(
        expr.split('|').count(),
        2,
        "the comment truncated: {expr:?}"
    );
}

#[test]
#[should_panic(expected = "not a plain integer literal")]
fn a_computed_key_is_refused_not_skipped() {
    // Valid Python. The runtime derives bit 23 from it and strips that bit;
    // a parser that only collected integer keys would report mask 0x10000.
    int_keys("16: 'a', 23 + 0: 'b'");
}

#[test]
#[should_panic(expected = "declared twice")]
fn a_duplicated_bit_key_is_refused() {
    int_keys("16: 'a', 16: 'b'");
}

#[test]
#[should_panic(expected = "outside a u32 status word")]
fn a_bit_key_beyond_the_status_word_is_refused() {
    int_keys("32: 'a'");
}

#[test]
#[should_panic(expected = "not a brace literal")]
fn a_non_dict_table_value_is_refused_not_skipped() {
    // `dict(...)` and a bare alias are both valid Python that the runtime would
    // expand into real bits.
    nested_entries("53: {16: 'a'}, 60: dict(((17, 'b'),))");
}

#[test]
#[should_panic(expected = "must occur exactly once")]
fn an_ambiguous_marker_is_refused() {
    // Two candidate tables and a parser that silently takes the first is a
    // "checked" table that is no longer the one the runtime reads.
    block_after("TABLE = {53: {16: 'a'}}\nTABLE = {53: {}}\n", "TABLE = ");
}

#[test]
fn a_hash_inside_a_string_is_not_a_comment() {
    // The badge names in these tables are quoted text; stripping a `#` inside
    // one would corrupt the source the later scanners read.
    let source = "TABLE = {\n    53: {16: 'a#}b'},\n}\n";
    let block = block_after(&strip_non_code(source), "TABLE = ");
    assert_eq!(mask_of(&int_keys(&nested_entries(&block)[0].1)), 1 << 16);
}

#[test]
fn a_colon_inside_a_badge_name_is_not_a_key_separator() {
    // Every real badge name in these tables contains a `:` (`d53:intel_map_...`,
    // `LANE:TORN_ROWS_PRESENT`), so this is the live case, not a hypothetical.
    assert_eq!(
        mask_of(&int_keys(
            "16: \"d53:operations_detail_resisted\", 17: 'LANE:X'"
        )),
        (1 << 16) | (1 << 17)
    );
}

#[test]
fn a_powershell_block_comment_cannot_satisfy_a_code_check() {
    let source = "<#\n.SYNOPSIS\n$Mask = [uint32]0x13CF\n#>\n$Mask = [uint32]0x0001\n";
    let code = strip_non_code_in(source, Dialect::PowerShell);
    assert!(
        !code.contains("[uint32]0x13CF"),
        "a `<# ... #>` block comment survived stripping: {code:?}"
    );
    assert!(
        code.contains("$Mask = [uint32]0x0001"),
        "stripping removed live code: {code:?}"
    );
}

#[test]
fn a_powershell_block_comment_cannot_truncate_a_block() {
    let source = "TABLE = @{\n    53 = 1  <# } #>\n    60 = 2\n}\n";
    let block = block_after(&strip_non_code_in(source, Dialect::PowerShell), "TABLE = ");
    assert!(
        block.contains("60 = 2"),
        "a commented-out closing brace truncated the block: {block:?}"
    );
}

#[test]
fn a_python_triple_quoted_block_cannot_satisfy_a_code_check() {
    let source = "\"\"\"\nKNOWN_MASK = (1 << 15) - 1\n\"\"\"\nKNOWN_MASK = (1 << 9) - 1\n";
    let code = strip_non_code(source);
    assert!(
        !code.contains("(1 << 15) - 1"),
        "a triple-quoted block survived stripping: {code:?}"
    );
    assert!(
        code.contains("KNOWN_MASK = (1 << 9) - 1"),
        "stripping removed live code: {code:?}"
    );
}

#[test]
fn stripping_preserves_offsets_so_a_marker_still_opens_its_literal() {
    let source = "TABLE = {  # note\n    53: {16: 'a'},\n}\n";
    let code = strip_non_code(source);
    assert_eq!(code.len(), source.len(), "stripping changed the length");
    let block = block_after(&code, "TABLE = ");
    assert_eq!(mask_of(&int_keys(&nested_entries(&block)[0].1)), 1 << 16);
}

fn powershell_if_blocks(text: &str, condition: &str) -> Vec<String> {
    let marker = format!("if ({condition}) {{");
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(found) = text[from..].find(&marker) {
        let start = from + found;
        let rest = &text[start..];
        let mut depth = 0i32;
        let mut end = None;
        for (offset, ch) in rest.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(offset + ch.len_utf8());
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end.unwrap_or_else(|| panic!("unterminated if block for {condition:?}"));
        out.push(rest[..end].to_string());
        from = start + end;
    }
    out
}

fn powershell_if_block(text: &str, condition: &str) -> String {
    let marker = format!("if ({condition}) {{");
    let start = unique_marker(text, &marker);
    let rest = &text[start..];
    let mut depth = 0i32;
    for (offset, ch) in rest.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return rest[..=offset].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unterminated if block for {condition:?}");
}

/// Exit code 2 is the whole point of this lane's change: callers separate
#[test]
#[cfg(windows)]
fn the_harness_status_verdict_survives_every_route_out() {
    use std::io::Write;

    let harness = read_script("Invoke-HeadlessRawCapture.ps1");
    let branches = powershell_if_blocks(
        &harness,
        "-not [string]::IsNullOrWhiteSpace($undefinedStatusBitFailure)",
    );
    assert_eq!(
        branches.len(),
        1,
        "expected exactly ONE arbitration point converting the verdict into \
         exit 2. More than one means some route out of the script is served by \
         a copy that can drift; none means the verdict cannot be delivered at \
         all. Found {}",
        branches.len()
    );
    let arbitration = &branches[0];

    let preference = "$ErrorActionPreference = 'Stop'";
    assert!(
        harness.contains(preference),
        "the harness no longer sets {preference}, so this proof no longer \
         reproduces its conditions"
    );

    let routes: &[(&str, &str)] = &[
        (
            "normal completion",
            "try { } finally { try { } finally { BODY } }",
        ),
        (
            "pending exception from the try body",
            "try { throw 'boom' } finally { try { } finally { BODY } }",
        ),
        (
            "cleanup fatal about to throw",
            "try { } finally { try { } finally { BODY }\n throw 'cleanup fatal' }",
        ),
        (
            "terminating error in the cleanup prefix",
            "try { } finally { try { Get-Item 'Z:\\no\\such\\path' } finally { BODY } }",
        ),
    ];

    let dir = std::env::temp_dir().join(format!("hoi4_lane_i_exit2_{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    // kills: qg_gate_speed — the four route probes are real powershell.exe
    // processes (that is the proof), but they ran back to back, serializing
    // ~2 s of interpreter startup. They run concurrently now; same script
    // bytes per route, failures collected in route order.
    let route_outcomes: Vec<(String, Option<i32>)> = std::thread::scope(|scope| {
        routes
            .iter()
            .map(|(label, shape)| {
                let dir = dir.clone();
                let arbitration = arbitration.clone();
                scope.spawn(move || {
                    let script = format!(
                        "{preference}\n\
                         $undefinedStatusBitFailure = 'proof'\n\
                         $cleanupFatalMessage = 'proof cleanup failure'\n\
                         $collectorInjectionStarted = $true\n\
                         $collectorTeardownAuthenticated = $false\n\
                         $gradedWindowStarted = $true\n\
                         {body}\n\
                         exit 0\n",
                        body = shape.replace("BODY", &arbitration),
                    );
                    let path = dir.join(format!("probe_{}.ps1", label.replace(' ', "_")));
                    let mut file = fs::File::create(&path).expect("create probe script");
                    file.write_all(script.as_bytes())
                        .expect("write probe script");
                    drop(file);
                    let status = std::process::Command::new("powershell.exe")
                        .args([
                            "-NoProfile",
                            "-NonInteractive",
                            "-ExecutionPolicy",
                            "Bypass",
                            "-File",
                        ])
                        .arg(&path)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .no_console_window()
                        .status()
                        .expect("run powershell");
                    let _ = fs::remove_file(&path);
                    ((*label).to_string(), status.code())
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().expect("probe thread"))
            .collect()
    });
    let mut failures = Vec::new();
    for ((label, _), (_, code)) in routes.iter().zip(route_outcomes.iter()) {
        if code != &Some(2) {
            failures.push(format!("{label}: exit={code:?} (wanted 2)"));
        }
    }

    let script = format!(
        "{preference}\n\
         $undefinedStatusBitFailure = $null\n\
         $cleanupFatalMessage = ''\n\
         $collectorInjectionStarted = $false\n\
         $collectorTeardownAuthenticated = $true\n\
         $gradedWindowStarted = $false\n\
         try {{ }} finally {{ try {{ }} finally {{ {arbitration} }} }}\n\
         exit 0\n"
    );
    let path = dir.join("probe_no_verdict.ps1");
    let mut file = fs::File::create(&path).expect("create probe script");
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);
    let status = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .no_console_window()
        .status()
        .expect("run powershell");
    if status.code() != Some(0) {
        failures.push(format!(
            "no verdict set: exit={:?} (wanted 0 -- the arbitration point must \
             be invisible when there is nothing to report)",
            status.code()
        ));
    }
    let _ = fs::remove_dir_all(&dir);

    assert!(
        failures.is_empty(),
        "the harness's compact-status arbitration point does not deliver exit 2 on \
         every route out of the script:\n  {}\nArbitration point was:\n{arbitration}",
        failures.join("\n  ")
    );
}

fn powershell_if_chain(text: &str, condition: &str) -> String {
    let marker = format!("if ({condition}) {{");
    let start = unique_marker(text, &marker);
    let rest = &text[start..];
    let mut cursor = 0usize;
    loop {
        let mut depth = 0i32;
        let mut end = None;
        for (offset, ch) in rest[cursor..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(cursor + offset + ch.len_utf8());
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end.unwrap_or_else(|| panic!("unterminated if chain for {condition:?}"));
        let tail = rest[end..].trim_start();
        if tail.starts_with("elseif") || tail.starts_with("else") {
            cursor = end + (rest[end..].len() - tail.len());
        } else {
            return rest[..end].to_string();
        }
    }
}

#[test]
#[cfg(windows)]
fn the_harness_report_dispatch_blocks_on_every_unanswered_path() {
    use std::io::Write;

    let harness = read_script("Invoke-HeadlessRawCapture.ps1");
    let chain = powershell_if_chain(&harness, "$haveStreamReport -and $haveManifest");
    let missing_reporter =
        powershell_if_block(&harness, "$haveManifest -and -not $haveStreamReport");

    let dir = std::env::temp_dir().join(format!("hoi4_lane_i_dispatch_{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let manifest = dir.join("native_session_manifest.json");
    let reporter = dir.join("report_raw_capture.py");
    fs::write(&manifest, "{}").expect("write probe manifest");
    fs::write(&reporter, "").expect("write probe reporter");
    let absent = dir.join("does_not_exist");

    let receipt = |code: i32| format!("COMPACT_STATUS_VERDICT={code}");

    let cases: Vec<(&str, String, bool, bool, bool, bool)> = vec![
        (
            "report says clean",
            format!("@{{Stdout='';Stderr='{}';ExitCode=0}}", receipt(0)),
            true,
            true,
            false,
            false,
        ),
        (
            "report says blocking",
            format!("@{{Stdout='';Stderr='{}';ExitCode=1}}", receipt(1)),
            true,
            true,
            false,
            false,
        ),
        (
            "report says undefined bits",
            format!("@{{Stdout='';Stderr='{}';ExitCode=2}}", receipt(2)),
            true,
            true,
            false,
            true,
        ),
        (
            "report crashed (3)",
            format!("@{{Stdout='';Stderr='{}';ExitCode=3}}", receipt(3)),
            true,
            true,
            false,
            true,
        ),
        (
            "report exited outside its vocabulary",
            format!("@{{Stdout='';Stderr='{}';ExitCode=9}}", receipt(9)),
            true,
            true,
            false,
            true,
        ),
        (
            "report died before its own code ran",
            "@{Stdout='';Stderr='SyntaxError';ExitCode=1}".to_string(),
            true,
            true,
            false,
            true,
        ),
        (
            "report printed a receipt that disagrees with its exit code",
            format!("@{{Stdout='';Stderr='{}';ExitCode=0}}", receipt(2)),
            true,
            true,
            false,
            true,
        ),
        (
            "report timed out (throw)",
            "throw 'raw-capture report timed out'".to_string(),
            true,
            true,
            false,
            true,
        ),
        (
            "reporter missing",
            format!("@{{Stdout='';Stderr='{}';ExitCode=0}}", receipt(0)),
            true,
            false,
            false,
            true,
        ),
        (
            "no manifest, game lived",
            format!("@{{Stdout='';Stderr='{}';ExitCode=0}}", receipt(0)),
            false,
            true,
            false,
            true,
        ),
        (
            "no manifest, game died",
            format!("@{{Stdout='';Stderr='{}';ExitCode=0}}", receipt(0)),
            false,
            true,
            true,
            false,
        ),
    ];

    // kills: qg_gate_speed — every case runs its probe in a REAL powershell.exe
    // (that is what makes the proof carry: the harness text is executed by the
    // shipping interpreter), but they were spawned one after another, so ~13
    // interpreter startups serialized into ~4.8 s. The probes are written and
    // run CONCURRENTLY now — same script bytes per case, same fresh process per
    // case, results collected in case order so the assertions below are
    // byte-identical.
    let absent_ref = &absent;
    let chain_ref = &chain;
    let missing_reporter_ref = &missing_reporter;
    let outcomes: Vec<(String, Option<i32>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = cases
            .iter()
            .map(|(label, stub, have_manifest, have_reporter, game_died, _want_block)| {
                let manifest = manifest.clone();
                let reporter = reporter.clone();
                let dir = dir.clone();
                let chain = chain_ref;
                let missing_reporter = missing_reporter_ref;
                let absent = absent_ref;
                scope.spawn(move || {
                    let manifest_path = if *have_manifest { &manifest } else { &absent };
                    let reporter_path = if *have_reporter { &reporter } else { &absent };
                    let body = if stub.starts_with("throw") {
                        stub.clone()
                    } else {
                        format!("[pscustomobject]{stub}")
                    };
                    let script = format!(
                        "$ErrorActionPreference = 'Stop'\n\
                         function Invoke-BoundedNativeCapture {{\n\
                         \x20 param($FilePath, $Arguments, $TimeoutSeconds, $Purpose, [switch]$AllowNonzeroExit)\n\
                         \x20 {body}\n\
                         }}\n\
                         $undefinedStatusBitFailure = 'standing verdict from capture start'\n\
                         $manifestPath = '{manifest}'\n\
                         $streamReport = '{reporter}'\n\
                         $pythonExe = 'python.exe'\n\
                         $OutDir = '{out_dir}'\n\
                         $gameDied = ${game_died}\n\
                         $haveManifest = Test-Path -LiteralPath $manifestPath -PathType Leaf\n\
                         $haveStreamReport = Test-Path -LiteralPath $streamReport -PathType Leaf\n\
                         {missing_reporter}\n\
                         {chain}\n\
                         if ([string]::IsNullOrWhiteSpace($undefinedStatusBitFailure)) {{ exit 0 }}\n\
                         exit 2\n",
                        manifest = manifest_path.display(),
                        reporter = reporter_path.display(),
                        out_dir = dir.display(),
                        game_died = if *game_died { "true" } else { "false" },
                    );
                    let path = dir.join(format!(
                        "probe_{}_{}.ps1",
                        label.replace([' ', ',', '(', ')'], "_"),
                        std::process::id(),
                    ));
                    let mut file = fs::File::create(&path).expect("create probe");
                    file.write_all(script.as_bytes()).expect("write probe");
                    drop(file);
                    let status = std::process::Command::new("powershell.exe")
                        .args([
                            "-NoProfile",
                            "-NonInteractive",
                            "-ExecutionPolicy",
                            "Bypass",
                            "-File",
                        ])
                        .arg(&path)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .no_console_window()
                        .status()
                        .expect("run powershell");
                    let _ = fs::remove_file(&path);
                    ((*label).to_string(), status.code())
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("probe thread"))
            .collect()
    });

    let mut failures = Vec::new();
    for ((_label, .., want_block), (ran_label, code)) in
        cases.iter().zip(outcomes.iter())
    {
        let blocked = code == &Some(2);
        if blocked != *want_block {
            failures.push(format!(
                "{ran_label}: blocked={blocked} (wanted {want_block}), exit={code:?}"
            ));
        }
    }
    let _ = fs::remove_dir_all(&dir);
    assert!(
        failures.is_empty(),
        "the harness's report dispatch does not block on every unanswered path:\n  {}",
        failures.join("\n  ")
    );
}

#[test]
fn the_reporter_and_manifest_checks_are_not_recombined() {
    let code = squeeze(&read_code("Invoke-HeadlessRawCapture.ps1"));
    assert!(
        !code.contains(
            "if ((Test-Path -LiteralPath $streamReport -PathType Leaf) -and \
             (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {"
        ),
        "the reporter and manifest existence checks are combined again, which makes a \
         finalized capture with a missing reporter exit 0"
    );
}

#[test]
fn the_harness_arbitration_is_nested_inside_the_cleanup_finally() {
    let code = read_code("Invoke-HeadlessRawCapture.ps1");
    let arbitration = "if (-not [string]::IsNullOrWhiteSpace($undefinedStatusBitFailure)) {";
    let position = code
        .find(arbitration)
        .expect("the harness has no compact-status arbitration point");

    let outer = code
        .find("} finally {\n    $cleanupFatalMessage = $null")
        .or_else(|| code.find("} finally {\r\n    $cleanupFatalMessage = $null"))
        .expect("the harness's outer cleanup finally moved");

    let inner_try = outer
        + code[outer..].find("try {").expect(
            "no `try` wraps the cleanup prefix, so a terminating error \
                    in it abandons the block before the verdict is delivered",
        );

    let after_try_body = matching_close(&code, inner_try + "try ".len())
        .expect("the cleanup-prefix `try` block is unterminated");
    let tail = &code[after_try_body..];
    let finally_at = tail
        .find('{')
        .expect("the cleanup-prefix `try` is not followed by a block");
    assert!(
        tail[..finally_at].contains("finally"),
        "the cleanup-prefix `try` is not followed by a `finally`: {:?}",
        &tail[..finally_at.min(80)]
    );
    let inner_finally_open = after_try_body + finally_at;
    let inner_finally_close = matching_close(&code, inner_finally_open)
        .expect("the inner `finally` block is unterminated");

    assert!(
        position > inner_finally_open && position < inner_finally_close,
        "the arbitration point is not INSIDE the inner `finally` that wraps the \
         cleanup prefix (arbitration at {position}, inner finally spans \
         {inner_finally_open}..{inner_finally_close}). Outside it, a terminating \
         error in the cleanup prefix -- every failing cmdlet is terminating \
         under $ErrorActionPreference = 'Stop' -- abandons the block before the \
         verdict is delivered."
    );
}

fn matching_close(text: &str, from: usize) -> Option<usize> {
    let open = from + text[from..].find('{')?;
    let mut depth = 0i32;
    for (offset, ch) in text[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

#[test]
#[should_panic(expected = "unterminated `<# ... #>` block comment")]
fn an_unterminated_block_comment_is_refused() {
    strip_non_code_in(
        "$Mask = 1\n<# never closed\n$Mask = 2\n",
        Dialect::PowerShell,
    );
}

#[test]
#[should_panic(expected = "unterminated triple-quoted block")]
fn an_unterminated_triple_quoted_block_is_refused() {
    strip_non_code("MASK = 1\n\"\"\" never closed\nMASK = 2\n");
}

#[test]
fn an_escaped_triple_delimiter_does_not_end_an_inert_block() {
    let source = "\"\"\"\ndocs with an escaped \\\"\"\" delimiter\nKNOWN_MASK = (1 << 15) - 1\n\"\"\"\nKNOWN_MASK = (1 << 9) - 1\n";
    let code = strip_non_code(source);
    assert!(
        !code.contains("(1 << 15) - 1"),
        "an escaped triple delimiter ended the inert block early: {code:?}"
    );
    assert!(code.contains("KNOWN_MASK = (1 << 9) - 1"));
}

#[test]
fn a_powershell_backtick_escape_does_not_end_a_string_early() {
    let source = "$a = \"has a `\" quote and a # hash\"; $Mask = [uint32]0x1\n";
    let code = strip_non_code_in(source, Dialect::PowerShell);
    assert!(
        code.contains("# hash"),
        "the `#` inside the string was taken for a comment, so the string ended \
         at the backtick-escaped quote: {code:?}"
    );
    assert!(
        code.contains("$Mask = [uint32]0x1"),
        "live code after the string was blanked: {code:?}"
    );
}

#[test]
fn a_doubled_quote_re_pairs_without_changing_what_counts_as_code() {
    let source = "$a = 'it''s fine # not a comment'; $Mask = [uint32]0x1\n";
    let code = strip_non_code_in(source, Dialect::PowerShell);
    assert!(
        code.contains("# not a comment"),
        "the `#` inside the doubled-quote string was taken for a comment: {code:?}"
    );
    assert!(
        code.contains("$Mask = [uint32]0x1"),
        "live code after the string was blanked: {code:?}"
    );
}

#[test]
fn blanking_preserves_byte_length_for_non_ascii() {
    let source = "TABLE = {  # note — with an em dash\n    53: {16: 'a'},\n}\n";
    let code = strip_non_code(source);
    assert_eq!(
        code.len(),
        source.len(),
        "stripping changed the BYTE length, so every offset after a non-ASCII \
         comment is wrong"
    );
    let block = block_after(&code, "TABLE = ");
    assert_eq!(mask_of(&int_keys(&nested_entries(&block)[0].1)), 1 << 16);
}

#[test]
fn a_powershell_backslash_is_not_an_escape() {
    let source = concat!(
        r"$root = $Dir.TrimEnd('\'); $Mask = [uint32]0x13CF  # note",
        "\n"
    );
    let code = strip_non_code_in(source, Dialect::PowerShell);
    assert!(
        code.contains("$Mask = [uint32]0x13CF"),
        "a lone backslash in a PowerShell string swallowed the live code after \
         it: {code:?}"
    );
    assert!(
        !code.contains("# note"),
        "the trailing comment was not stripped, so the string never closed and \
         the backslash was wrongly treated as an escape: {code:?}"
    );
}

#[test]
fn a_powershell_here_string_is_inert() {
    let source = r#"$c = @"
$script:ProductionPromotionFailureMask = [uint32]0x13CF
an unclosed <# inside data
"@
$script:ProductionPromotionFailureMask = [uint32]0x0001
"#;
    let code = strip_non_code_in(source, Dialect::PowerShell);
    assert!(
        !code.contains("[uint32]0x13CF"),
        "here-string contents survived stripping and can satisfy a code check: {code:?}"
    );
    assert!(
        code.contains("[uint32]0x0001"),
        "stripping removed the live assignment after the here-string: {code:?}"
    );
}

#[test]
#[should_panic(expected = "unterminated here-string")]
fn an_unterminated_here_string_is_refused() {
    let source = "$c = @\"\nnever closed\n";
    strip_non_code_in(source, Dialect::PowerShell);
}

#[test]
fn the_two_dialects_really_differ() {
    let source = "x = 'a\\' + '#b'\n";
    let python = strip_non_code_in(source, Dialect::Python);
    let powershell = strip_non_code_in(source, Dialect::PowerShell);
    assert_ne!(
        python, powershell,
        "the dialects produced identical output for an input whose escape rule \
         differs between them, so the dialect argument is doing nothing"
    );
}

fn classify_powershell_value(value: &str) -> Option<bool> {
    let trimmed = value.trim();
    if trimmed == "$null" || trimmed.eq_ignore_ascii_case("[string]::Empty") {
        return Some(false);
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let mut index = 0usize;
    let mut proven_non_empty = false;
    loop {
        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }
        if index >= chars.len() {
            break;
        }
        let quote = chars[index];
        if quote != '\'' && quote != '"' {
            return None; // not a quoted literal: a call, an operator, a cast
        }
        index += 1;
        let mut literal_text = false;
        let mut closed = false;
        while index < chars.len() {
            let ch = chars[index];
            if quote == '"' && ch == '`' {
                index += 2;
                continue;
            }
            if ch == quote {
                index += 1;
                closed = true;
                break;
            }
            if quote == '"' && ch == '$' {
                // Skip the interpolation: `$( ... )` or `$name`.
                index += 1;
                if chars.get(index) == Some(&'(') {
                    let mut depth = 0i32;
                    while index < chars.len() {
                        match chars[index] {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    index += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        index += 1;
                    }
                } else {
                    while index < chars.len()
                        && (chars[index].is_alphanumeric() || chars[index] == '_')
                    {
                        index += 1;
                    }
                }
                continue;
            }
            if !ch.is_whitespace() {
                literal_text = true;
            }
            index += 1;
        }
        if !closed {
            return None; // unterminated literal
        }
        proven_non_empty |= literal_text;

        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }
        match chars.get(index) {
            None => break,
            Some('+') => index += 1,
            Some(_) => return None, // `.Substring(...)`, `-replace`, `-as`, ...
        }
    }
    Some(proven_non_empty)
}

/// The verdict DEFAULTS to blocking from the moment a capture exists.
///
/// Before this, everything between injection and the report -- a passive-witness
/// write, a control call, any bug -- could throw straight into cleanup with the
/// verdict still null, and null meant "checked and clean" when nothing had been
/// checked. Now the standing verdict is set beside `$collectorInjectionStarted`
/// and only an ANSWER retires it: a report exit of 0 or 1 with a matching
/// receipt, or the explicitly accepted game-died route.
#[test]
fn the_verdict_defaults_to_blocking_once_a_capture_exists() {
    let code = read_code("Invoke-HeadlessRawCapture.ps1");

    let injection = code
        .find("$collectorInjectionStarted = $true")
        .expect("the harness no longer marks injection start");
    let standing = code
        .find("$undefinedStatusBitFailure =\n    'the capture started but never reached")
        .or_else(|| {
            code.find("$undefinedStatusBitFailure =\r\n    'the capture started but never reached")
        })
        .expect(
            "the harness does not arm a standing compact-status verdict when the \
             capture goes live, so a throw before the report reads as checked-and-clean",
        );
    // NOTHING may sit between them. A byte-distance bound is not enough: a
    // `throw` fits inside any window, and the whole point is that no statement
    // can run -- and therefore fail -- before the verdict is armed. The span is
    // measured on the COMMENT-STRIPPED text, so the explanatory comment there
    // is already blanked.
    assert!(
        standing > injection,
        "the standing verdict precedes injection start"
    );
    let between = &code[injection + "$collectorInjectionStarted = $true".len()..standing];
    assert!(
        between.trim().is_empty(),
        "there is executable code between `$collectorInjectionStarted = $true` \
         and the standing compact-status verdict; anything there that throws \
         jumps to cleanup before the verdict is armed, and a null verdict reads \
         as checked-and-clean. Found: {:?}",
        between.trim()
    );

    // EVERY assignment is audited, not just the literal `= $null` ones. The
    // arbitration tests the verdict with `IsNullOrWhiteSpace`, so `= ''` and
    // `= '   '` retire it just as effectively as `$null` while leaving a
    // `$null`-only count unchanged.
    let mut retiring = Vec::new();
    let mut arming = Vec::new();
    for (at, _) in code.match_indices("$undefinedStatusBitFailure =") {
        let rest = &code[at + "$undefinedStatusBitFailure =".len()..];
        // The assignment's VALUE: the rest of this line, plus continuation lines
        // while the previous one ends in `+` (PowerShell string concatenation).
        // Anything beyond that belongs to the next statement and must not be
        // read as part of this one.
        // A value may begin on the SAME line as the `=` or on the next one, so
        // leading blank lines are part of this statement, not the next.
        let mut value = String::new();
        for line in rest.lines() {
            let blank_so_far = value.trim().is_empty();
            value.push_str(line);
            value.push('\n');
            if !blank_so_far && !line.trim_end().ends_with('+') {
                break;
            }
            if blank_so_far && !line.trim().is_empty() && !line.trim_end().ends_with('+') {
                break;
            }
        }
        // Is this value null, empty or whitespace AT RUNTIME? Decided
        // STRUCTURALLY, and fail-closed on anything the audit cannot prove.
        //
        // Inspecting only the start of the expression is not proof: every one
        // of `'message' -replace 'message', ''`, `"message".Substring(7)` and
        // `'message' -as [int]` begins with a quoted word and evaluates to
        // null/empty, and each would retire the verdict while looking like a
        // message. So the value must be a `+`-join of quoted literals and
        // nothing else; a method call or any other operator is refused.
        let trimmed = value.trim();
        let classification = classify_powershell_value(trimmed);
        let retires = classification == Some(false);
        assert!(
            classification.is_some(),
            "assignment to $undefinedStatusBitFailure at {at} has a value this \
             audit cannot PROVE is either verdict-retiring or verdict-arming, \
             so it cannot vouch for the gate. Only `$null`, `[string]::Empty` \
             and a `+`-join of quoted literals are recognised -- a method call, \
             a `-replace`, or any other operator can turn a message into an \
             empty string and silently retire the verdict. Value: {trimmed:?}"
        );
        let carries_text = !retires;
        if carries_text {
            arming.push(at);
        } else {
            retiring.push(at);
        }
    }
    assert_eq!(
        retiring.len(),
        3,
        "expected exactly three assignments that can leave the verdict \
         null/empty/whitespace -- the pre-capture declaration, and the two \
         routes that may RETIRE it (a report exit of 0 or 1 backed by a matching \
         receipt, and the game-died route). Found {} at {:?}. Each extra one is \
         a new way for an unchecked capture to report as clean, and an empty \
         string counts because the arbitration uses IsNullOrWhiteSpace.",
        retiring.len(),
        retiring
    );
    assert!(
        retiring[0] < injection,
        "the only verdict-retiring assignment before injection must be the declaration"
    );
    assert!(
        retiring[1] > standing && retiring[2] > standing,
        "a route retires the standing verdict before it is even armed"
    );
    assert!(
        !arming.is_empty(),
        "no assignment sets a verdict MESSAGE, so nothing can ever block"
    );
}

/// Every script must be read under ITS OWN dialect.
///
/// The lexical rules genuinely conflict -- a backslash escapes in Python and is
/// an ordinary character in PowerShell -- so routing a `.ps1` through the Python
/// reader means the sweep is checking something other than what the runtime
/// runs. Pinned directly because the table tests reach `strip_non_code_in` by
/// way of `read_code`, and a `dialect_of` that always answered Python would
/// leave most of them green.
#[test]
fn each_script_is_read_under_its_own_dialect() {
    for name in [
        "ProductionCapture.Promotion.ps1",
        "Invoke-HeadlessRawCapture.ps1",
    ] {
        assert_eq!(
            dialect_of(name),
            Dialect::PowerShell,
            "{name} is not read as PowerShell"
        );
    }
    for name in [
        "report_raw_capture.py",
        "c0_domain_census.py",
        "c0_pass2_spans.py",
    ] {
        assert_eq!(
            dialect_of(name),
            Dialect::Python,
            "{name} is not read as Python"
        );
    }
}

#[test]
#[should_panic(expected = "subexpression")]
fn a_double_quoted_here_string_with_a_subexpression_is_refused() {
    // `@" ... "@` INTERPOLATES: a `$( ... )` inside it is executable code, so
    // blanking the here-string would hide it from this sweep. None of the four
    // scripts has one, so this fails closed rather than guessing.
    let source = "$c = @\"\nvalue is $($Mask -band 1)\n\"@\n";
    strip_non_code_in(source, Dialect::PowerShell);
}

#[test]
fn a_single_quoted_here_string_is_still_inert() {
    let source = "$c = @'\nliteral $($Mask -band 1) text\n'@\n$Mask = [uint32]0x1\n";
    let code = strip_non_code_in(source, Dialect::PowerShell);
    assert!(
        !code.contains("literal"),
        "single-quoted here-string contents survived stripping: {code:?}"
    );
    assert!(code.contains("$Mask = [uint32]0x1"));
}

#[test]
fn a_triple_quoted_f_string_is_not_treated_as_inert() {
    let source = "x = f'''value {compute_mask()} here'''\n";
    let code = strip_non_code(source);
    assert!(
        code.contains("compute_mask()"),
        "an f-string's replacement expression was blanked as if it were an inert \
         commented-out block: {code:?}"
    );
}

#[test]
fn a_plain_triple_quoted_block_is_still_inert() {
    let source = "'''\nKNOWN_MASK = (1 << 15) - 1\n'''\nKNOWN_MASK = (1 << 9) - 1\n";
    let code = strip_non_code(source);
    assert!(
        !code.contains("(1 << 15) - 1"),
        "a plain triple-quoted block survived stripping: {code:?}"
    );
    assert!(code.contains("(1 << 9) - 1"));
}

#[test]
fn combined_f_string_prefixes_are_recognised() {
    for source in [
        "x = rf'''value {compute_mask()} here'''\n",
        "x = fr'''value {compute_mask()} here'''\n",
        "x = F'''value {compute_mask()} here'''\n",
    ] {
        let code = strip_non_code(source);
        assert!(
            code.contains("compute_mask()"),
            "an f-string prefix was not recognised, so its replacement \
             expression was blanked: {source:?} -> {code:?}"
        );
    }
}

#[test]
fn a_raw_triple_quoted_block_without_an_f_is_still_inert() {
    let source = "r'''\nKNOWN_MASK = (1 << 15) - 1\n'''\nKNOWN_MASK = (1 << 9) - 1\n";
    let code = strip_non_code(source);
    assert!(
        !code.contains("(1 << 15) - 1"),
        "a raw (non-f) triple-quoted block survived stripping: {code:?}"
    );
}

#[test]
fn an_escaped_subexpression_in_a_here_string_is_not_a_subexpression() {
    let source = "$c = @\"\nliteral `$(not code) text\n\"@\n$Mask = [uint32]0x1\n";
    let code = strip_non_code_in(source, Dialect::PowerShell);
    assert!(
        code.contains("$Mask = [uint32]0x1"),
        "the here-string was not blanked: {code:?}"
    );
    assert!(
        !code.contains("literal"),
        "here-string contents survived: {code:?}"
    );
}

#[test]
#[should_panic(expected = "unescaped")]
fn a_doubly_escaped_backtick_still_leaves_a_live_subexpression() {
    let source = "$c = @\"\nvalue ``$($Mask -band 1)\n\"@\n";
    strip_non_code_in(source, Dialect::PowerShell);
}

#[test]
fn the_verdict_value_classifier_proves_rather_than_guesses() {
    let arms: &[&str] = &[
        "'the capture started but never reached the raw-capture report'",
        "'[compact status] report did not complete'",
        "\"the raw-capture report exited $($result.ExitCode) without its receipt\"",
        "\"'$verdictReceipt'; the two disagree, so its verdict is UNKNOWN.\"",
        "'first half ' +\n    'second half'",
    ];
    let retires: &[&str] = &[
        "$null",
        "''",
        "\"\"",
        "[string]::Empty",
        "[String]::Empty",
        "' '",
        "\"$null\"",
        "\"$missing\"",
        "\"`t\"",
        "\"$($x)\"",
        "'' + \"\"",
    ];
    let refuse: &[&str] = &[
        "'message' -replace 'message', ''",
        "\"message\".Substring(7)",
        "'message' -as [int]",
        "'' + [string]::Empty",
        "$someHelper.Reset()",
        "'unterminated",
    ];

    for value in arms {
        assert_eq!(
            classify_powershell_value(value),
            Some(true),
            "should be provably non-empty: {value:?}"
        );
    }
    for value in retires {
        assert_eq!(
            classify_powershell_value(value),
            Some(false),
            "should be provably null/empty/whitespace: {value:?}"
        );
    }
    for value in refuse {
        assert_eq!(
            classify_powershell_value(value),
            None,
            "should be REFUSED rather than classified: {value:?}"
        );
    }
}

fn gap_ledger_python_set(marker: &str) -> BTreeSet<String> {
    let code = read_code("gap_ledger.py");
    let block = block_after(&code, marker);
    top_level_elements(&block)
        .into_iter()
        .map(|element| {
            let trimmed = element.trim().to_string();
            trimmed
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .unwrap_or_else(|| {
                    panic!("{marker} element is not a plain string literal: {trimmed:?}")
                })
                .to_string()
        })
        .collect()
}

#[test]
fn gap_ledger_python_admits_exactly_the_contract_gap_reasons() {
    use compact_contract::GapReason;
    let declared = gap_ledger_python_set(
        "
GAP_REASONS = frozenset(",
    );
    let expected: BTreeSet<String> = GapReason::ALL
        .iter()
        .map(|reason| reason.wire_name().to_string())
        .collect();
    assert_eq!(
        expected.len(),
        GapReason::ALL.len(),
        "two reasons share a name"
    );
    assert_eq!(declared, expected, "gap_ledger.py's GAP_REASONS is stale");
}

#[test]
fn gap_ledger_python_admits_exactly_the_contract_gap_counters() {
    use compact_contract::GapCounter;
    let declared = gap_ledger_python_set("GAP_COUNTERS = frozenset(");
    let expected: BTreeSet<String> = GapCounter::ALL
        .iter()
        .map(|counter| counter.wire_name().to_string())
        .collect();
    assert_eq!(declared, expected, "gap_ledger.py's GAP_COUNTERS is stale");
}

#[test]
fn gap_ledger_python_admits_exactly_the_contract_domain_gap_reasons() {
    use compact_contract::DomainGapReason;
    let declared = gap_ledger_python_set("DOMAIN_GAP_REASONS = frozenset(");
    let expected: BTreeSet<String> = DomainGapReason::ALL
        .iter()
        .map(|reason| reason.wire_name().to_string())
        .collect();
    assert_eq!(
        declared, expected,
        "gap_ledger.py's DOMAIN_GAP_REASONS is stale"
    );
}

#[test]
fn gap_ledger_python_admits_exactly_the_contract_gap_parts() {
    use compact_contract::DomainGapPart;
    let declared = gap_ledger_python_set("DOMAIN_GAP_PARTS = frozenset(");
    let expected: BTreeSet<String> = DomainGapPart::ALL
        .iter()
        .map(|part| part.wire_name().to_string())
        .collect();
    assert_eq!(
        declared, expected,
        "gap_ledger.py's DOMAIN_GAP_PARTS is stale"
    );
}
