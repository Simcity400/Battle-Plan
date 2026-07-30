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
pub const RECORD_TEXT_LIMIT: usize = 3072;

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
