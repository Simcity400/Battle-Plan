//! The collector's worker-error record: one shared format, written by the
//! collector and read by the recorder host.
//!
//! `collector_worker_error.log` sits beside `collector.dll`. It is opened
//! append-only, is never truncated, accumulates across days, and is shared by
//! every collector loaded from that directory. A reader therefore cannot
//! attribute a record to one capture by file position, and a single append is
//! not guaranteed to be atomic: `write_all` may retry after a short write, so
//! two writers that each split can interleave arbitrarily.
//!
//! Each record is one physical line carrying everything needed to decide
//! ownership on its own:
//!
//! ```text
//! pid=<pid> ctime=<creation> len=<NNNNNNNN> sum=<XXXXXXXX> <body>
//! ```
//!
//! * `pid` and `ctime` identify the writing process instance. A pid alone is
//!   only unique while that process object lives.
//! * `len` is the exact byte length of `body`, in a fixed-width field so
//!   declaring the length cannot change it.
//! * `sum` is a digest over `body`. Length alone only proves a body is the
//!   right SIZE; the digest proves it is the right BYTES, so an interleaving
//!   that happens to substitute an equal-length run is rejected too.
//! * `body` ends with `end=pid=<pid>`.
//!
//! Both halves of this contract live here so the writer and the reader cannot
//! drift apart.

/// Width of the fixed-width byte-length field.
pub const LEN_FIELD_WIDTH: usize = 8;
/// Width of the fixed-width body digest field.
pub const SUM_FIELD_WIDTH: usize = 8;
/// Longest error text kept in one record. Bounding the record keeps the append
/// a single small write, which is what makes interleaving unlikely at all.
///
/// This is a bound on ONE RECORD, and deliberately not a bound on one ERROR:
/// see [`format_records`], which splits a long error across several records
/// rather than destroying its tail.
pub const RECORD_TEXT_LIMIT: usize = 3072;

/// The most records one error may be split across by [`format_records`].
///
/// The largest producer is the collector's `frame_degraded` summary, which
/// names every domain that went unknown for an hour with that domain's own
/// typed reason. Its worst case is bounded by the wire record it mirrors:
/// `frame_degraded::FRAME_DEGRADED_MAX_REASON_LEN` (512) per domain plus the
/// `d<id>:0x........[]` framing and a separator, so roughly 530 bytes per
/// degraded domain. At the 73-domain scheduled roster that is under 39 KB --
/// thirteen records -- and this cap leaves it a fivefold margin while still
/// refusing an absurd one.
///
/// Exceeding it is not silent. Every chunk is labelled `i/n` with the TRUE
/// `n`, so a reader that receives fewer records than `n` can see exactly how
/// many were dropped, and by how much the cap was passed.
pub const MAX_RECORD_CHUNKS: usize = 64;

/// Largest `n` a reader will believe in a `i/n` chunk label.
///
/// Distinct from [`MAX_RECORD_CHUNKS`], which bounds what is WRITTEN. A label
/// legitimately declares more than that -- the excess is exactly how a reader
/// learns the cap was passed -- but [`reassemble_records`] sizes a slot table
/// from the declared total, so an unbounded one is an allocation a foreign or
/// corrupt record gets to choose. Sixty-four times the write cap leaves the
/// real overrun signal intact while refusing an absurd one.
pub const MAX_DECLARED_CHUNK_TOTAL: usize = MAX_RECORD_CHUNKS * 64;

/// FNV-1a over the body bytes, folded to 32 bits.
///
/// This detects accidental interleaving and truncation, which is what the log
/// is exposed to. It is deliberately not a cryptographic commitment: anyone who
/// can forge a colliding body can also overwrite `collector.dll` in the same
/// directory, so a stronger digest would not buy a stronger guarantee.
pub fn body_digest(body: &[u8]) -> u32 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in body {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    ((hash >> 32) as u32) ^ (hash as u32)
}

/// Build the exact bytes of one record, including its terminating newline.
pub fn format_record(pid: u32, creation_filetime: u64, phase: &str, error: &str) -> String {
    let mut text = format!("{error:?}");
    if text.len() > RECORD_TEXT_LIMIT {
        let mut cut = RECORD_TEXT_LIMIT;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("...\"");
    }
    let body = format!("phase={phase} error={text} end=pid={pid}");
    format!(
        "pid={pid} ctime={creation_filetime} len={:0len_width$} sum={:0sum_width$x} {body}\n",
        body.len().min(99_999_999),
        body_digest(body.as_bytes()),
        len_width = LEN_FIELD_WIDTH,
        sum_width = SUM_FIELD_WIDTH,
    )
}

/// Byte length this character contributes to a `{:?}` rendering of the string
/// containing it.
///
/// `char::escape_debug` is a strict UPPER bound on what `str`'s own `Debug`
/// spells for the same character: `str` escapes grapheme-extended characters
/// only in leading position and never escapes `'`, while this escapes both
/// unconditionally. Over-estimating is the safe direction -- it makes a chunk
/// slightly shorter than it had to be -- and under-estimating would silently
/// hand `format_record` a chunk it then truncates, which is the whole defect
/// this splitter exists to remove.
fn debug_escaped_len(character: char) -> usize {
    character.escape_debug().map(char::len_utf8).sum()
}

/// Split one error into the largest pieces that each still render inside
/// [`RECORD_TEXT_LIMIT`] once `{:?}`-escaped.
///
/// Splits on CHARACTER boundaries of the raw text, never inside an escape
/// sequence: each piece is independently valid UTF-8 and is escaped on its own,
/// so no piece can end mid-`\u{...}`.
fn split_error_text(error: &str) -> Vec<&str> {
    // Two bytes of the budget always go to the surrounding quotes.
    let budget = RECORD_TEXT_LIMIT.saturating_sub(2);
    let mut pieces = Vec::new();
    let mut start = 0usize;
    let mut used = 0usize;
    for (index, character) in error.char_indices() {
        let cost = debug_escaped_len(character);
        if used != 0 && used + cost > budget {
            pieces.push(&error[start..index]);
            start = index;
            used = 0;
        }
        used += cost;
    }
    // The tail, and the empty error: an empty piece is still a record, because
    // "this phase failed and said nothing" is itself the report.
    pieces.push(&error[start..]);
    pieces
}

/// Next reason ORDINAL this load will mint. Starts at 1.
static NEXT_REASON_ORDINAL: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(1);

/// A reason id, unique among the reasons one reader can see together.
///
/// The high half is a per-LOAD salt and the low half a counter, and the salt is
/// what makes the id sufficient. A counter alone is unique only within one load
/// of this code, and the reader's grouping key is (reason id, phase) among
/// records attributed to one process instance -- so a collector unloaded and
/// re-injected into the same game process would restart at 1, and the reader
/// would join the first reason of the second capture onto the first reason of
/// the first. Both are that pid and creation time's records, so nothing else in
/// the envelope separates them, and the result would be one merged text that
/// reads as complete.
///
/// The salt is forced non-zero, so a zero id remains a parse failure rather than
/// a plausible first reason.
fn next_reason_id() -> u64 {
    use core::sync::atomic::Ordering;
    static SALT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let salt = *SALT.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos() as u64)
            .unwrap_or(0);
        u64::from((((nanos >> 32) ^ nanos) as u32) | 1)
    });
    let ordinal = NEXT_REASON_ORDINAL.fetch_add(1, Ordering::Relaxed);
    (salt << 32) | (ordinal & 0xFFFF_FFFF)
}

/// The marker that opens a chunk label, chosen so it cannot occur in a phase.
/// Every phase this crate's callers pass is a bare identifier.
const CHUNK_LABEL_MARKER: &str = "#r";

/// Build every record needed to carry one error, in order.
///
/// A single-record error is byte-identical to [`format_record`], phase and all.
/// A longer one is CHUNKED rather than cut: each record carries
/// `phase#r<reason_id>#i/n` and its own share of the text, so the reason a
/// degraded frame gives survives to whoever reads the log instead of ending at
/// an arbitrary buffer size. Each record is still one small self-validating
/// append, which is the property the per-record bound exists to protect.
///
/// The REASON ID is what makes reassembly possible at all. The log is shared
/// and append-only, a single append is not atomic, and one process writes many
/// errors: `i/n` alone cannot say which reason a chunk belongs to, so two
/// interleaved multi-chunk reports reassemble into one nonsense string. The id
/// is minted per call from a process-local counter, so two identical reasons
/// written back to back stay two reasons.
pub fn format_records(
    pid: u32,
    creation_filetime: u64,
    phase: &str,
    error: &str,
) -> Vec<String> {
    format_records_with_reason_id(pid, creation_filetime, phase, error, next_reason_id())
}

/// [`format_records`] with the reason id supplied, so the chunking and the
/// reassembly are testable as pure functions.
pub fn format_records_with_reason_id(
    pid: u32,
    creation_filetime: u64,
    phase: &str,
    error: &str,
    reason_id: u64,
) -> Vec<String> {
    let pieces = split_error_text(error);
    let total = pieces.len();
    if total <= 1 {
        return vec![format_record(pid, creation_filetime, phase, error)];
    }
    pieces
        .iter()
        .take(MAX_RECORD_CHUNKS)
        .enumerate()
        .map(|(index, piece)| {
            format_record(
                pid,
                creation_filetime,
                &format!("{phase}{CHUNK_LABEL_MARKER}{reason_id:016x}#{}/{total}", index + 1),
                piece,
            )
        })
        .collect()
}

/// The chunk label a chunked record carries in its phase field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkLabel {
    /// The phase as it would have been written unchunked.
    pub phase: String,
    /// Which reason these chunks belong to.
    pub reason_id: u64,
    /// 1-based position of this chunk.
    pub index: usize,
    /// How many chunks the reason NEEDED. May exceed [`MAX_RECORD_CHUNKS`], and
    /// that is how a reader learns the cap was passed.
    pub total: usize,
}

/// One record's body, split into the parts a reader needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedBody<'a> {
    /// The phase field exactly as written, chunk label and all.
    pub phase_field: &'a str,
    /// Present when the phase field carries a chunk label.
    pub chunk: Option<ChunkLabel>,
    /// The `{:?}`-rendered error text, quotes included.
    pub escaped_text: &'a str,
}

/// Split one record body (`phase=... error=... end=pid=...`) into its parts.
///
/// Returns `None` for anything that is not that shape. The closing stamp is
/// located from the END, because the error text is free-form and may itself
/// contain the literal `end=pid=`.
pub fn parse_body(body: &str) -> Option<ParsedBody<'_>> {
    let rest = body.strip_prefix("phase=")?;
    let (phase_field, rest) = rest.split_once(" error=")?;
    let stamp = rest.rfind(" end=pid=")?;
    let escaped_text = &rest[..stamp];
    // A phase field carrying the chunk MARKER but no parseable label is a
    // malformed record, not an unchunked one. Downgrading it would report a
    // single arbitrary slice of some longer reason as a complete error --
    // silently, and with the reason's own text as the evidence for it.
    let chunk = parse_chunk_label(phase_field);
    if chunk.is_none() && phase_field.contains(CHUNK_LABEL_MARKER) {
        return None;
    }
    Some(ParsedBody {
        phase_field,
        chunk,
        escaped_text,
    })
}

fn parse_chunk_label(phase_field: &str) -> Option<ChunkLabel> {
    let (phase, rest) = phase_field.split_once(CHUNK_LABEL_MARKER)?;
    let (reason_hex, position) = rest.split_once('#')?;
    let reason_id = u64::from_str_radix(reason_hex, 16).ok()?;
    if reason_id == 0 {
        return None;
    }
    let (index, total) = position.split_once('/')?;
    let index: usize = index.parse().ok()?;
    let total: usize = total.parse().ok()?;
    // `total` is the count the writer NEEDED and may legitimately exceed
    // `MAX_RECORD_CHUNKS` -- that is how a reader learns the cap was passed --
    // but a reassembler sizes a slot table from it, so it cannot be unbounded.
    // Past this ceiling the label is not something this contract wrote.
    if index == 0 || total == 0 || index > total || total > MAX_DECLARED_CHUNK_TOTAL {
        return None;
    }
    Some(ChunkLabel {
        phase: phase.to_owned(),
        reason_id,
        index,
        total,
    })
}

/// Invert the `{:?}` rendering [`format_record`] applied.
///
/// Only the escapes `char::escape_debug` can emit are accepted; anything else
/// is a malformed record and returns `None` rather than a guess. The input
/// includes the surrounding quotes.
pub fn unescape_debug_text(escaped: &str) -> Option<String> {
    let inner = escaped.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            // `format_record` escapes both of these unconditionally, so a raw
            // one means the text was not produced by this contract's writer.
            // Copying it through would turn noncanonical noise into a plausible
            // reason -- and a raw `"` would additionally let a body claim a
            // shorter text than it carries.
            if character == '"' || character.is_control() {
                return None;
            }
            out.push(character);
            continue;
        }
        match chars.next()? {
            '0' => out.push('\0'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut digits = String::new();
                loop {
                    match chars.next()? {
                        '}' => break,
                        digit if digit.is_ascii_hexdigit() && digits.len() < 6 => {
                            digits.push(digit)
                        }
                        _ => return None,
                    }
                }
                out.push(char::from_u32(u32::from_str_radix(&digits, 16).ok()?)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// One error recovered from the log, chunked or not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReassembledError {
    pub phase: String,
    /// `None` for an error that was written as a single unchunked record.
    pub reason_id: Option<u64>,
    /// The concatenated, unescaped text of every chunk that was present.
    pub text: String,
    /// How many chunks the writer said the reason needed.
    pub chunks_expected: usize,
    /// How many of them this reader actually holds.
    pub chunks_present: usize,
    /// The 1-based positions that are missing, in order. Non-empty exactly when
    /// the text below is INCOMPLETE, which is what makes a shortfall
    /// attributable instead of merely suspected.
    pub missing_chunks: Vec<usize>,
    /// The 1-based positions where two records claiming the same reason
    /// disagreed, plus position 0 when a later record declared a different
    /// total.
    ///
    /// Its own axis because it is a different fact from a missing chunk. A hole
    /// is a record that did not arrive; a conflict is two records that cannot
    /// both be true, and the reader has no way to choose. Silently keeping the
    /// first was the defect: `#1/2 "A1"`, `#1/2 "B1"`, `#2/2 "B2"` reassembled
    /// to `A1B2` and reported it COMPLETE -- a text neither writer ever wrote.
    pub conflicting_chunks: Vec<usize>,
}

impl ReassembledError {
    /// Every chunk arrived and none of them contradicted another.
    pub fn complete(&self) -> bool {
        self.missing_chunks.is_empty() && self.conflicting_chunks.is_empty()
    }
}

/// Recover every error carried by these record bodies, in first-appearance
/// order, joining a reason's chunks back into one text.
///
/// Bodies that do not parse are dropped: the caller has already validated each
/// record's envelope, so an unparseable body is a record written by a different
/// build, and inventing a position for it would corrupt a real reason.
///
/// A chunk index that arrives twice keeps the FIRST copy, and a second copy
/// carrying DIFFERENT text is recorded as a conflict rather than discarded: the
/// two records cannot both be true and nothing here can choose between them, so
/// the reason must not read as complete. The same applies to a later record of
/// the same reason declaring a different total, which is recorded as conflict
/// position 0.
pub fn reassemble_records(bodies: &[&str]) -> Vec<ReassembledError> {
    // (reason_id, phase) -> position in `out`. A reason id is minted per call
    // in one process, but the log is shared, so the phase is carried in the key
    // as well rather than trusting the id to be globally unique.
    let mut index_of: alloc_map::Map<(u64, String), usize> = alloc_map::Map::new();
    let mut chunks: Vec<Vec<Option<String>>> = Vec::new();
    let mut out: Vec<ReassembledError> = Vec::new();
    for body in bodies {
        let Some(parsed) = parse_body(body) else {
            continue;
        };
        let Some(text) = unescape_debug_text(parsed.escaped_text) else {
            continue;
        };
        let Some(label) = parsed.chunk else {
            out.push(ReassembledError {
                phase: parsed.phase_field.to_owned(),
                reason_id: None,
                text,
                chunks_expected: 1,
                chunks_present: 1,
                missing_chunks: Vec::new(),
                conflicting_chunks: Vec::new(),
            });
            chunks.push(Vec::new());
            continue;
        };
        let key = (label.reason_id, label.phase.clone());
        let slot = match index_of.get(&key) {
            Some(slot) => *slot,
            None => {
                let slot = out.len();
                index_of.insert(key, slot);
                out.push(ReassembledError {
                    phase: label.phase.clone(),
                    reason_id: Some(label.reason_id),
                    text: String::new(),
                    chunks_expected: label.total,
                    chunks_present: 0,
                    missing_chunks: Vec::new(),
                    conflicting_chunks: Vec::new(),
                });
                chunks.push(vec![None; label.total]);
                slot
            }
        };
        // A later record of the same reason declaring a different total is two
        // writers disagreeing about how long the reason is. The first
        // declaration stays -- it is what sized the slot table -- but the
        // disagreement is reported, because a text assembled under a total that
        // is in dispute is not a complete text.
        if out[slot].chunks_expected != label.total
            && !out[slot].conflicting_chunks.contains(&0)
        {
            out[slot].conflicting_chunks.push(0);
        }
        let Some(cell) = chunks[slot].get_mut(label.index - 1) else {
            // An index past the first declared total. It belongs to no position
            // this reason has, and the total disagreement above already says so.
            continue;
        };
        match cell {
            None => *cell = Some(text),
            Some(existing) if *existing == text => {}
            Some(_) => {
                if !out[slot].conflicting_chunks.contains(&label.index) {
                    out[slot].conflicting_chunks.push(label.index);
                }
            }
        }
    }
    for entry in &mut out {
        entry.conflicting_chunks.sort_unstable();
    }
    for (entry, pieces) in out.iter_mut().zip(chunks) {
        if entry.reason_id.is_none() {
            continue;
        }
        for (position, piece) in pieces.into_iter().enumerate() {
            match piece {
                Some(text) => {
                    entry.text.push_str(&text);
                    entry.chunks_present += 1;
                }
                None => entry.missing_chunks.push(position + 1),
            }
        }
    }
    out
}

/// A tiny insertion-ordered map, so this module keeps its `alloc`-only surface
/// and does not pull `std::collections` into the contract crate's core.
mod alloc_map {
    pub struct Map<K, V>(Vec<(K, V)>);

    impl<K: PartialEq, V> Map<K, V> {
        pub fn new() -> Self {
            Map(Vec::new())
        }

        pub fn get(&self, key: &K) -> Option<&V> {
            self.0
                .iter()
                .find(|(existing, _)| existing == key)
                .map(|(_, value)| value)
        }

        pub fn insert(&mut self, key: K, value: V) {
            self.0.push((key, value));
        }
    }
}

/// The header a record written by this exact process instance must open with.
pub fn owned_header(pid: u32, creation_filetime: u64) -> String {
    format!("pid={pid} ctime={creation_filetime} len=")
}

/// The stamp a record written by this pid must close with.
pub fn owned_suffix(pid: u32) -> String {
    format!("end=pid={pid}")
}

/// Is this one physical line (no trailing newline) an intact record written by
/// the process instance described by `owned_header` / `owned_suffix`?
///
/// Operates on RAW BYTES on purpose. Validating after a lossy UTF-8 conversion
/// would let a body containing invalid bytes measure longer than it is — each
/// invalid byte becomes a three-byte replacement character — so a severed body
/// could satisfy a declared length it does not actually have.
///
/// Returns the body on success so the caller cannot re-derive it differently.
pub fn owned_body<'a>(line: &'a [u8], owned_header: &str, owned_suffix: &str) -> Option<&'a [u8]> {
    let rest = line.strip_prefix(owned_header.as_bytes())?;
    // len=<digits> sum=<hex> <body>
    let sum_label = b"sum=";
    let fixed = LEN_FIELD_WIDTH + 1 + sum_label.len() + SUM_FIELD_WIDTH + 1;
    if rest.len() < fixed {
        return None;
    }
    let (digits, rest) = rest.split_at(LEN_FIELD_WIDTH);
    if !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let (space, rest) = rest.split_at(1);
    if space != b" " {
        return None;
    }
    let (label, rest) = rest.split_at(sum_label.len());
    if label != sum_label {
        return None;
    }
    let (sum_digits, rest) = rest.split_at(SUM_FIELD_WIDTH);
    if !sum_digits.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let (space, body) = rest.split_at(1);
    if space != b" " {
        return None;
    }
    let declared_len: usize = core::str::from_utf8(digits).ok()?.parse().ok()?;
    let declared_sum = u32::from_str_radix(core::str::from_utf8(sum_digits).ok()?, 16).ok()?;
    if body.len() != declared_len
        || body_digest(body) != declared_sum
        || !body.ends_with(owned_suffix.as_bytes())
    {
        return None;
    }
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_formatted_record_round_trips_through_the_reader() {
        let line = format_record(4321, 77, "capture", "something broke");
        let line = line.strip_suffix('\n').expect("record ends with a newline");
        let body = owned_body(
            line.as_bytes(),
            &owned_header(4321, 77),
            &owned_suffix(4321),
        )
        .expect("the writer's own record must be readable");
        assert!(String::from_utf8_lossy(body).contains("something broke"));
    }

    #[test]
    fn another_process_instance_does_not_own_the_record() {
        let line = format_record(4321, 77, "capture", "something broke");
        let line = line.strip_suffix('\n').expect("newline");
        assert!(owned_body(
            line.as_bytes(),
            &owned_header(4321, 88),
            &owned_suffix(4321)
        )
        .is_none());
        assert!(owned_body(line.as_bytes(), &owned_header(999, 77), &owned_suffix(999)).is_none());
    }

    #[test]
    fn an_equal_length_substituted_body_is_rejected_by_the_digest() {
        // Length alone only proves a body is the right SIZE. An interleaving
        // that replaces the body with a same-length run ending in our closing
        // stamp would satisfy it; the digest is what rejects that.
        let line = format_record(4321, 77, "capture", "AAAAAAAA");
        let line = line.strip_suffix('\n').expect("newline");
        let header = owned_header(4321, 77);
        let suffix = owned_suffix(4321);
        let body = owned_body(line.as_bytes(), &header, &suffix).expect("intact record");
        let substituted = {
            let mut bytes = line.as_bytes().to_vec();
            let start = bytes.len() - body.len();
            // Same length, same closing stamp, different content.
            let mut forged = body.to_vec();
            forged[0] = b'Z';
            bytes.truncate(start);
            bytes.extend_from_slice(&forged);
            bytes
        };
        assert_eq!(
            substituted.len(),
            line.len(),
            "the forgery is the same size"
        );
        assert!(
            substituted.ends_with(suffix.as_bytes()),
            "the forgery keeps the closing stamp"
        );
        assert!(
            owned_body(&substituted, &header, &suffix).is_none(),
            "an equal-length substituted body must be rejected"
        );
    }

    #[test]
    fn an_invalid_utf8_body_cannot_inflate_its_way_to_the_declared_length() {
        // `from_utf8_lossy` turns each invalid byte into a three-byte
        // replacement character. Validating after that conversion would let a
        // 14-byte body satisfy a declared length of 16.
        let suffix = owned_suffix(4321);
        let header = owned_header(4321, 77);
        let mut body = vec![b'X', 0xA9];
        body.extend_from_slice(suffix.as_bytes());
        let short_len = body.len();
        let mut line = format!(
            "{header}{:0len_width$} sum={:0sum_width$x} ",
            short_len + 2,
            body_digest(&body),
            len_width = LEN_FIELD_WIDTH,
            sum_width = SUM_FIELD_WIDTH,
        )
        .into_bytes();
        line.extend_from_slice(&body);
        assert!(
            owned_body(&line, &header, &suffix).is_none(),
            "a raw body shorter than declared must be rejected before any lossy conversion"
        );
    }

    /// The defect: an error longer than one record used to lose everything
    /// past the bound, permanently. `format_records` splits instead, and the
    /// concatenated pieces are the WHOLE original error.
    ///
    /// Production change that makes this fail: `format_records` falling back to
    /// the single truncating `format_record` for a long error, or
    /// `split_error_text` dropping a piece.
    #[test]
    fn a_long_error_survives_whole_across_records() {
        // Long enough to need several records, and not a round multiple of the
        // budget, so an off-by-one in the tail cannot hide.
        let error = format!(
            "hour 60831436 published with degraded domains: {}",
            (0..40)
                .map(|domain| format!("d{domain}:0x00000004[{}]", "R".repeat(500)))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let records = format_records_with_reason_id(4321, 77, "frame_degraded", &error, 0x5EED);
        assert!(
            records.len() > 1,
            "an error this size must not fit one record, got {} record(s)",
            records.len()
        );

        let header = owned_header(4321, 77);
        let suffix = owned_suffix(4321);
        let mut recovered = String::new();
        for (index, record) in records.iter().enumerate() {
            let line = record.strip_suffix('\n').expect("record ends with a newline");
            // Every chunk is a COMPLETE record in its own right: the reader
            // validates each independently, so a lost or interleaved chunk
            // costs that chunk and nothing else.
            let body = owned_body(line.as_bytes(), &header, &suffix)
                .expect("each chunk must be an intact owned record");
            let body = String::from_utf8(body.to_vec()).expect("chunk body is UTF-8");
            let expected_phase = format!(
                "phase=frame_degraded#r{:016x}#{}/{} error=",
                0x5EEDu64,
                index + 1,
                records.len()
            );
            assert!(
                body.starts_with(&expected_phase),
                "chunk {} must name its reason and its position, got {body:.80}",
                index + 1
            );
            let text = body
                .strip_prefix(&expected_phase)
                .and_then(|rest| rest.strip_suffix(&format!(" {suffix}")))
                .expect("chunk body is phase + debug text + stamp");
            assert!(
                !text.ends_with("...\""),
                "chunk {} was truncated by format_record, so the split budget is wrong",
                index + 1
            );
            // Undo the `{:?}` rendering the same way the writer applied it.
            recovered.push_str(&unescape_debug(text));
        }
        assert_eq!(
            recovered, error,
            "the concatenated chunks must be the whole original error"
        );
    }

    /// Minimal inverse of `{:?}` for the escapes this text can contain.
    fn unescape_debug(text: &str) -> String {
        let inner = text
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .expect("debug text is quoted");
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(character) = chars.next() {
            if character != '\\' {
                out.push(character);
                continue;
            }
            match chars.next().expect("escape has a body") {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                '\'' => out.push('\''),
                other => panic!("unexpected escape \\{other}"),
            }
        }
        out
    }

    /// No chunk may exceed the per-record bound, whatever the text is made of.
    ///
    /// The budget is computed from `char::escape_debug`, so a character whose
    /// escaped form is far longer than its raw form is exactly what would break
    /// a byte-counting splitter.
    ///
    /// Production change that makes this fail: `split_error_text` measuring raw
    /// byte length instead of escaped length.
    #[test]
    fn no_chunk_exceeds_the_single_record_bound() {
        for error in [
            "\u{7}".repeat(RECORD_TEXT_LIMIT), // escapes to \u{7}, 6 bytes each
            "\"\\\n\t".repeat(RECORD_TEXT_LIMIT / 2), // two bytes each
            "é".repeat(RECORD_TEXT_LIMIT),    // two raw bytes, one escaped char
            "A".repeat(RECORD_TEXT_LIMIT * 3),
        ] {
            for record in format_records(4321, 77, "capture", &error) {
                let line = record.strip_suffix('\n').expect("newline");
                let body = owned_body(
                    line.as_bytes(),
                    &owned_header(4321, 77),
                    &owned_suffix(4321),
                )
                .expect("intact record");
                let body = String::from_utf8_lossy(body);
                assert!(
                    !body.contains("...\""),
                    "a chunk was truncated by format_record: {body:.120}"
                );
                assert!(
                    body.len() <= RECORD_TEXT_LIMIT + 128,
                    "chunk body is {} byte(s), past the per-record bound",
                    body.len()
                );
            }
        }
    }

    /// A short error is still exactly one record, with its phase untouched.
    /// The chunk labelling must not appear on the overwhelmingly common case.
    #[test]
    fn a_short_error_is_one_unlabelled_record() {
        let records = format_records(4321, 77, "capture", "something broke");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0], format_record(4321, 77, "capture", "something broke"));
        assert!(records[0].contains("phase=capture error="), "{}", records[0]);
    }

    /// An error past the chunk cap loses records, but says so: `n` is the
    /// count that was NEEDED, so the shortfall is arithmetic, not a guess.
    ///
    /// Production change that makes this fail: labelling chunks `i/emitted`
    /// instead of `i/total`.
    #[test]
    fn passing_the_chunk_cap_is_countable_from_the_records_themselves() {
        let error = "A".repeat(RECORD_TEXT_LIMIT * (MAX_RECORD_CHUNKS + 7));
        let records = format_records_with_reason_id(4321, 77, "capture", &error, 0x5EED);
        assert_eq!(records.len(), MAX_RECORD_CHUNKS, "the cap bounds what is emitted");
        let declared: usize = records[0]
            .split(&format!("phase=capture#r{:016x}#1/", 0x5EEDu64))
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .expect("the first chunk declares the total")
            .parse()
            .expect("the total is a number");
        assert!(
            declared > MAX_RECORD_CHUNKS,
            "the declared total must be the count NEEDED ({declared}), not the count emitted"
        );
    }

    /// Build the long multi-domain `frame_degraded` reason the chunking exists
    /// for: every scheduled domain named with its own typed reason.
    fn multi_domain_degraded_reason(domains: usize) -> String {
        format!(
            "hour 60831436 published with degraded domains: {}",
            (0..domains)
                .map(|domain| format!(
                    "d{domain}:0x00000004[reader refused: {}]",
                    "unit=".repeat(60)
                ))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }

    /// THE ROUND TRIP: a long multi-domain reason written as chunks and read
    /// back is the SAME STRING, byte for byte.
    ///
    /// Chunking alone did not achieve this. The chunks carried `i/n` and no
    /// reason id, so a reader had no way to know which reason a chunk belonged
    /// to and did not try: it joined whole physical records to a byte cap and
    /// appended `[truncated]`. The text the collector wrote reached nobody.
    ///
    /// Production change that makes this fail: dropping the reason id from the
    /// chunk label (`{phase}#{i}/{total}`), or `reassemble_records` joining
    /// chunks in arrival order instead of by index.
    #[test]
    fn a_long_multi_domain_reason_survives_the_round_trip_whole() {
        let error = multi_domain_degraded_reason(40);
        let records = format_records(4321, 77, "frame_degraded", &error);
        assert!(
            records.len() > 4,
            "the fixture must need several records, got {}",
            records.len()
        );

        let header = owned_header(4321, 77);
        let suffix = owned_suffix(4321);
        // Deliver the chunks OUT OF ORDER, with the last one first. The log is
        // shared and append-only; nothing guarantees a reader sees a reason's
        // records contiguously, and an index that is only decorative would pass
        // a test that fed them in order.
        let mut delivered: Vec<String> = Vec::new();
        for record in records.iter().rev() {
            let line = record.strip_suffix('\n').expect("record ends with a newline");
            let body = owned_body(line.as_bytes(), &header, &suffix)
                .expect("each chunk must be an intact owned record");
            delivered.push(String::from_utf8(body.to_vec()).expect("chunk body is UTF-8"));
        }
        let bodies: Vec<&str> = delivered.iter().map(String::as_str).collect();

        let reasons = reassemble_records(&bodies);
        assert_eq!(reasons.len(), 1, "one reason, not one per record: {reasons:?}");
        let reason = &reasons[0];
        assert_eq!(reason.phase, "frame_degraded");
        assert!(reason.reason_id.is_some());
        assert_eq!(reason.chunks_expected, records.len());
        assert_eq!(reason.chunks_present, records.len());
        assert!(reason.complete(), "{:?}", reason.missing_chunks);
        assert_eq!(reason.text, error, "the reassembled reason must be the whole original");
    }

    /// Two reasons written back to back do not merge, even with the same phase
    /// and identical text.
    ///
    /// This is the whole argument for the reason id. `i/n` alone cannot tell
    /// "chunk 2 of the first report" from "chunk 2 of the second", so a reader
    /// that grouped on the phase would interleave them into one nonsense string
    /// and call it complete.
    ///
    /// Production change that makes this fail: minting the reason id from the
    /// phase or from the error text instead of a per-call counter.
    #[test]
    fn two_reports_of_the_same_reason_stay_two_reasons() {
        let error = multi_domain_degraded_reason(12);
        let first = format_records(4321, 77, "frame_degraded", &error);
        let second = format_records(4321, 77, "frame_degraded", &error);
        assert!(first.len() > 1 && second.len() > 1);

        let header = owned_header(4321, 77);
        let suffix = owned_suffix(4321);
        let mut bodies = Vec::new();
        // Interleaved, which is what a shared append-only log actually does.
        for index in 0..first.len().max(second.len()) {
            for record in [first.get(index), second.get(index)].into_iter().flatten() {
                let line = record.strip_suffix('\n').expect("newline");
                let body = owned_body(line.as_bytes(), &header, &suffix).expect("intact record");
                bodies.push(String::from_utf8(body.to_vec()).expect("UTF-8"));
            }
        }
        let bodies: Vec<&str> = bodies.iter().map(String::as_str).collect();

        let reasons = reassemble_records(&bodies);
        assert_eq!(reasons.len(), 2, "the two reports must stay apart: {reasons:?}");
        for reason in &reasons {
            assert!(reason.complete(), "{reason:?}");
            assert_eq!(reason.text, error);
        }
        assert_ne!(
            reasons[0].reason_id, reasons[1].reason_id,
            "each call must mint its own reason id"
        );
    }

    /// A reason id is unique across a RELOAD of this code inside one process,
    /// not merely across calls within one load.
    ///
    /// The reader's grouping key is (reason id, phase) among records attributed
    /// to one process instance. A collector unloaded and re-injected into the
    /// same game process writes under the same pid and creation time, so if the
    /// counter restarted at 1 the second capture's first reason would be joined
    /// onto the first capture's first reason and read as one complete text.
    ///
    /// A test cannot reload the module, so it asserts the property that makes a
    /// reload safe: the id is not a small ordinal, and its high half -- the
    /// per-load salt -- is non-zero and shared by every id this load mints.
    ///
    /// Production change that makes this fail: minting the id from the ordinal
    /// counter alone.
    #[test]
    fn a_reason_id_is_not_a_bare_ordinal_a_reload_would_repeat() {
        let first = next_reason_id();
        let second = next_reason_id();
        assert_ne!(first, second);
        let salt = |id: u64| id >> 32;
        assert_ne!(salt(first), 0, "the per-load salt must be non-zero: {first:#018x}");
        assert_eq!(
            salt(first),
            salt(second),
            "every id from one load shares its salt"
        );
        assert_eq!(
            second & 0xFFFF_FFFF,
            (first & 0xFFFF_FFFF) + 1,
            "the low half is the per-load ordinal"
        );
    }

    /// A chunk that never arrives is NAMED, and the reason says so instead of
    /// reading as complete.
    ///
    /// Production change that makes this fail: `reassemble_records` pushing
    /// present chunks without reserving the absent positions, so a hole closes
    /// up silently.
    #[test]
    fn a_missing_chunk_is_named_not_silently_closed_over() {
        let error = multi_domain_degraded_reason(30);
        let records = format_records(4321, 77, "frame_degraded", &error);
        assert!(records.len() >= 3, "need at least three chunks to drop a middle one");

        let header = owned_header(4321, 77);
        let suffix = owned_suffix(4321);
        let mut bodies = Vec::new();
        for (index, record) in records.iter().enumerate() {
            if index == 1 {
                continue;
            }
            let line = record.strip_suffix('\n').expect("newline");
            let body = owned_body(line.as_bytes(), &header, &suffix).expect("intact record");
            bodies.push(String::from_utf8(body.to_vec()).expect("UTF-8"));
        }
        let bodies: Vec<&str> = bodies.iter().map(String::as_str).collect();

        let reasons = reassemble_records(&bodies);
        assert_eq!(reasons.len(), 1);
        assert!(!reasons[0].complete());
        assert_eq!(reasons[0].missing_chunks, vec![2]);
        assert_eq!(reasons[0].chunks_present, records.len() - 1);
        assert_eq!(reasons[0].chunks_expected, records.len());
        assert!(
            reasons[0].text.len() < error.len(),
            "a reason missing a chunk must not claim the whole text"
        );
    }

    /// Two records claiming the same chunk of the same reason cannot both be
    /// true, and the result must NOT read as a complete text.
    ///
    /// The shared log is append-only and every record is validated on its own,
    /// so a foreign build, a retry that changed its mind, or a reason id
    /// collision can put two different `#1/2` records in front of one reader.
    /// Keeping the first silently produced a hybrid -- the first record's chunk
    /// 1 joined to the second's chunk 2 -- reported complete, with a text
    /// neither writer ever wrote.
    ///
    /// Production change that makes this fail: `reassemble_records` keeping the
    /// first copy without recording the disagreement (`if cell.is_none() { *cell
    /// = Some(text) }`).
    #[test]
    fn two_records_claiming_one_chunk_make_the_reason_conflicted_not_complete() {
        let reason_id = 0xABCDu64;
        let stamp = owned_suffix(4321);
        let body = |index: usize, total: usize, text: &str| {
            format!("phase=frame_degraded#r{reason_id:016x}#{index}/{total} error={text:?} {stamp}")
        };
        let bodies = [body(1, 2, "A1"), body(1, 2, "B1"), body(2, 2, "B2")];
        let bodies: Vec<&str> = bodies.iter().map(String::as_str).collect();

        let reasons = reassemble_records(&bodies);
        assert_eq!(reasons.len(), 1);
        assert_eq!(
            reasons[0].conflicting_chunks,
            vec![1],
            "the contested position must be named"
        );
        assert!(
            !reasons[0].complete(),
            "a reason with a contested chunk must not read as complete: {:?}",
            reasons[0]
        );
        assert!(reasons[0].missing_chunks.is_empty(), "nothing is missing");

        // An IDENTICAL duplicate is a retry, not a disagreement.
        let repeated = [body(1, 2, "A1"), body(1, 2, "A1"), body(2, 2, "A2")];
        let repeated: Vec<&str> = repeated.iter().map(String::as_str).collect();
        let reasons = reassemble_records(&repeated);
        assert_eq!(reasons.len(), 1);
        assert!(reasons[0].complete(), "{:?}", reasons[0]);
        assert_eq!(reasons[0].text, "A1A2");
    }

    /// A later record of the same reason declaring a DIFFERENT total is the
    /// same disagreement one level up, and is reported at position 0.
    ///
    /// `#1/2` followed by `#2/3` used to reassemble as a complete `2/2`,
    /// because a later total was never compared with the first.
    ///
    /// Production change that makes this fail: dropping the
    /// `out[slot].chunks_expected != label.total` comparison.
    #[test]
    fn a_disagreeing_declared_total_is_a_conflict_not_a_complete_reason() {
        let reason_id = 0x1234u64;
        let stamp = owned_suffix(4321);
        let body = |index: usize, total: usize, text: &str| {
            format!("phase=capture#r{reason_id:016x}#{index}/{total} error={text:?} {stamp}")
        };
        let bodies = [body(1, 2, "A1"), body(2, 3, "A2")];
        let bodies: Vec<&str> = bodies.iter().map(String::as_str).collect();

        let reasons = reassemble_records(&bodies);
        assert_eq!(reasons.len(), 1);
        assert!(
            reasons[0].conflicting_chunks.contains(&0),
            "a disagreeing total is reported at position 0: {:?}",
            reasons[0]
        );
        assert!(!reasons[0].complete(), "{:?}", reasons[0]);
    }

    /// A phase field carrying the chunk MARKER but no parseable label is a
    /// malformed record, not an unchunked one.
    ///
    /// `parse_chunk_label` refuses an absurd total, a zero reason id and an
    /// out-of-range index by returning `None` -- and `parse_body` used to hand
    /// that back as an ordinary phase, so one arbitrary slice of a longer reason
    /// was reported as a whole, complete error.
    ///
    /// Production change that makes this fail: dropping the
    /// `phase_field.contains(CHUNK_LABEL_MARKER)` guard from `parse_body`.
    #[test]
    fn a_marker_bearing_body_with_an_unparseable_label_is_refused() {
        for phase_field in [
            format!("capture#r{:016x}#1/{}", 7u64, MAX_DECLARED_CHUNK_TOTAL + 1),
            format!("capture#r{:016x}#1/2", 0u64),
            "capture#r0000000000000007#3/2".to_string(),
            "capture#rzzzz#1/2".to_string(),
            "capture#r0000000000000007".to_string(),
        ] {
            let body = format!("phase={phase_field} error=\"x\" end=pid=1");
            assert!(
                parse_body(&body).is_none(),
                "a marker-bearing body with an unusable label must be refused: {body}"
            );
            assert!(
                reassemble_records(&[body.as_str()]).is_empty(),
                "and it must not reassemble into a complete unchunked error: {body}"
            );
        }
        // The same body with no marker at all is still an ordinary record.
        let plain = "phase=capture error=\"x\" end=pid=1";
        assert!(parse_body(plain).is_some());
    }

    /// The unescape refuses text `format_record` could not have written.
    ///
    /// Every non-backslash character used to be copied through, so a raw
    /// interior quote or a raw control byte became part of a "recovered" reason.
    /// A raw `"` is the dangerous one: `"a"b"` returned `a"b`, i.e. a body
    /// claiming a shorter text than it carries.
    ///
    /// Production change that makes this fail: removing the
    /// `character == '"' || character.is_control()` refusal.
    #[test]
    fn the_unescape_refuses_text_the_writer_could_not_have_produced() {
        assert_eq!(unescape_debug_text("\"a\"b\""), None, "a raw interior quote");
        assert_eq!(unescape_debug_text("\"a\tb\""), None, "a raw control byte");
        assert_eq!(unescape_debug_text("\"a\nb\""), None);
        // The canonical renderings of the same characters still round-trip.
        assert_eq!(
            unescape_debug_text(&format!("{:?}", "a\"b")).as_deref(),
            Some("a\"b")
        );
        assert_eq!(
            unescape_debug_text(&format!("{:?}", "a\tb")).as_deref(),
            Some("a\tb")
        );
    }

    /// Passing the write cap is ARITHMETIC in the reassembled reason, not a
    /// suspicion: the declared total is the count that was needed, so every
    /// dropped position is named.
    ///
    /// Production change that makes this fail: labelling chunks `i/emitted`, or
    /// `reassemble_records` sizing its slot table from what arrived.
    #[test]
    fn passing_the_write_cap_names_every_dropped_chunk() {
        let error = "A".repeat(RECORD_TEXT_LIMIT * (MAX_RECORD_CHUNKS + 3));
        let records = format_records(4321, 77, "capture", &error);
        assert_eq!(records.len(), MAX_RECORD_CHUNKS);

        let header = owned_header(4321, 77);
        let suffix = owned_suffix(4321);
        let bodies: Vec<String> = records
            .iter()
            .map(|record| {
                let line = record.strip_suffix('\n').expect("newline");
                let body = owned_body(line.as_bytes(), &header, &suffix).expect("intact record");
                String::from_utf8(body.to_vec()).expect("UTF-8")
            })
            .collect();
        let bodies: Vec<&str> = bodies.iter().map(String::as_str).collect();

        let reasons = reassemble_records(&bodies);
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons[0].chunks_present, MAX_RECORD_CHUNKS);
        assert!(
            reasons[0].chunks_expected > MAX_RECORD_CHUNKS,
            "the declared total must be the count NEEDED ({}), not the count emitted",
            reasons[0].chunks_expected
        );
        assert_eq!(
            reasons[0].missing_chunks,
            (MAX_RECORD_CHUNKS + 1..=reasons[0].chunks_expected).collect::<Vec<_>>(),
            "every position past the cap must be named"
        );
    }

    /// An unchunked record still parses, and carries no chunk label at all --
    /// so the overwhelmingly common single-record case is untouched by any of
    /// this.
    #[test]
    fn an_unchunked_record_parses_with_no_chunk_label() {
        let line = format_record(4321, 77, "capture", "something broke");
        let line = line.strip_suffix('\n').expect("newline");
        let body = owned_body(line.as_bytes(), &owned_header(4321, 77), &owned_suffix(4321))
            .expect("intact record");
        let body = core::str::from_utf8(body).expect("UTF-8");
        let parsed = parse_body(body).expect("an ordinary record parses");
        assert_eq!(parsed.phase_field, "capture");
        assert!(parsed.chunk.is_none());
        assert_eq!(
            unescape_debug_text(parsed.escaped_text).as_deref(),
            Some("something broke")
        );
    }

    /// The unescape is the exact inverse of what the writer applied, for every
    /// escape `{:?}` can produce -- including `\u{...}` and NUL.
    ///
    /// Production change that makes this fail: dropping the `'u'` arm, or
    /// treating an unknown escape as a literal instead of refusing.
    #[test]
    fn the_unescape_inverts_every_escape_the_writer_can_emit() {
        for original in [
            "plain",
            "quote\" backslash\\ tick'",
            "tab\t newline\n carriage\r",
            "nul\u{0} bell\u{7} delete\u{7f}",
            "wide \u{1F600} and combining e\u{301}",
            "",
        ] {
            let rendered = format!("{original:?}");
            assert_eq!(
                unescape_debug_text(&rendered).as_deref(),
                Some(original),
                "round trip failed for {original:?} rendered as {rendered}"
            );
        }
        assert_eq!(unescape_debug_text("no quotes"), None);
        assert_eq!(unescape_debug_text("\"\\q\""), None, "an unknown escape must refuse");
        assert_eq!(unescape_debug_text("\"\\u{zz}\""), None);
    }

    /// A declared total no writer of this contract could have produced is
    /// rejected as a label, so a foreign record cannot choose a reassembler's
    /// allocation.
    ///
    /// Production change that makes this fail: removing the
    /// `total > MAX_DECLARED_CHUNK_TOTAL` clause from `parse_chunk_label`.
    #[test]
    fn an_absurd_declared_chunk_total_is_not_a_chunk_label() {
        let body = format!(
            "phase=capture#r{:016x}#1/{} error=\"x\" end=pid=1",
            7u64,
            MAX_DECLARED_CHUNK_TOTAL + 1
        );
        assert!(
            parse_chunk_label("capture#r0000000000000007#1/4097").is_none(),
            "a total past the ceiling is not a label"
        );
        // And because the marker IS present, the body as a whole is refused
        // rather than downgraded to an unchunked record -- see
        // `a_marker_bearing_body_with_an_unparseable_label_is_refused`.
        assert!(parse_body(&body).is_none());
        let ok = format!(
            "phase=capture#r{:016x}#1/{MAX_DECLARED_CHUNK_TOTAL} error=\"x\" end=pid=1",
            7u64
        );
        assert!(
            parse_body(&ok).expect("body parses").chunk.is_some(),
            "the ceiling itself is still accepted"
        );
    }

    #[test]
    fn a_long_error_is_truncated_and_still_round_trips() {
        let huge = "E".repeat(RECORD_TEXT_LIMIT * 3);
        let line = format_record(4321, 77, "capture", &huge);
        assert!(line.len() < RECORD_TEXT_LIMIT * 2, "record stays bounded");
        let line = line.strip_suffix('\n').expect("newline");
        assert!(
            owned_body(
                line.as_bytes(),
                &owned_header(4321, 77),
                &owned_suffix(4321)
            )
            .is_some(),
            "a truncated record is still a valid record"
        );
    }
}
