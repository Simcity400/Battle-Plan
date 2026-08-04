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

/// Build every record needed to carry one error, in order.
///
/// A single-record error is byte-identical to [`format_record`], phase and all.
/// A longer one is CHUNKED rather than cut: each record carries `phase#i/n` and
/// its own share of the text, so the reason a degraded frame gives survives to
/// whoever reads the log instead of ending at an arbitrary buffer size. Each
/// record is still one small self-validating append, which is the property the
/// per-record bound exists to protect.
pub fn format_records(
    pid: u32,
    creation_filetime: u64,
    phase: &str,
    error: &str,
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
                &format!("{phase}#{}/{total}", index + 1),
                piece,
            )
        })
        .collect()
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
        let records = format_records(4321, 77, "frame_degraded", &error);
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
            let expected_phase = format!("phase=frame_degraded#{}/{} error=", index + 1, records.len());
            assert!(
                body.starts_with(&expected_phase),
                "chunk {} must name its position, got {body:.80}",
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
        let records = format_records(4321, 77, "capture", &error);
        assert_eq!(records.len(), MAX_RECORD_CHUNKS, "the cap bounds what is emitted");
        let declared: usize = records[0]
            .split("phase=capture#1/")
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
