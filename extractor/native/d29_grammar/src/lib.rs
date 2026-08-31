//! D29 `ships` byte grammar.
//!
//! WHY THIS CRATE EXISTS. The four ship ROW FLAGS -- what a row's flag word
//! means -- were declared four times in four crates that cannot see each other:
//! the collector that writes the word, the host's domain-data authority and
//! world fold that read it, and the independent wire reference that decodes a
//! recording. Exchanging `DEFINITION` and `OWNER` in any ONE of them left the
//! known-flag mask identical and every self-check true, while a genuine
//! owner-only row was read as carrying a definition and refused for having an
//! empty definition string (lane UT, Codex round 43, finding 5).
//!
//! A LEAF CRATE, for the reason `d28_grammar` and `d34_grammar` are: the wire
//! reference admits `compact_contract` only under an owner-reviewed allow-list.

#![forbid(unsafe_code)]

/// Bytes before the first ship row.
pub const HEADER_LEN: usize = 40;

/// The FIXED part of a row. A row is this plus a u16-length-prefixed
/// `definition` string, so it is not the whole row width.
pub const FIXED_ROW_LEN: usize = 40;

/// The u16 length prefix that follows the fixed part.
pub const DEFINITION_LEN_PREFIX: usize = 2;

/// The header's version word.
pub const PAYLOAD_VERSION: u32 = 2;

/// A set bit REMOVES the key: a wire tombstone, not a record.
pub const ROW_FLAG_DELETED: u32 = 1;
pub const ROW_FLAG_TASK_FORCE: u32 = 1 << 1;
pub const ROW_FLAG_DEFINITION: u32 = 1 << 2;
pub const ROW_FLAG_OWNER: u32 = 1 << 3;

/// Every flag this grammar defines. Any other bit is a hard error: an
/// unrecognised flag word means the row is not this grammar.
pub const KNOWN_ROW_FLAGS: u32 =
    ROW_FLAG_DELETED | ROW_FLAG_TASK_FORCE | ROW_FLAG_DEFINITION | ROW_FLAG_OWNER;

/// Every flag, in bit order, with the field each one says is present.
///
/// The names are what makes an exchange visible: two flags that only ever
/// appear as bits in a mask can be swapped with every mask assertion true.
pub const ROW_FLAGS: [(u32, &str); 4] = [
    (ROW_FLAG_DELETED, "deleted"),
    (ROW_FLAG_TASK_FORCE, "task_force"),
    (ROW_FLAG_DEFINITION, "definition"),
    (ROW_FLAG_OWNER, "owner"),
];

const _: () = {
    let mut i = 0;
    let mut union = 0;
    while i < ROW_FLAGS.len() {
        assert!(
            ROW_FLAGS[i].0 == 1 << i,
            "a ship row flag is not its own bit, so the flag word and this \
             table disagree about which field a bit announces",
        );
        assert!(union & ROW_FLAGS[i].0 == 0, "two ship row flags share a bit");
        union |= ROW_FLAGS[i].0;
        let mut j = i + 1;
        while j < ROW_FLAGS.len() {
            assert!(
                !str_eq(ROW_FLAGS[i].1, ROW_FLAGS[j].1),
                "two ship row flags are published under the same name",
            );
            j += 1;
        }
        i += 1;
    }
    assert!(union == KNOWN_ROW_FLAGS, "the known mask is not the flags above");
    assert!(
        DEFINITION_LEN_PREFIX < FIXED_ROW_LEN,
        "the definition prefix is not smaller than the fixed row it follows",
    );
};

const fn str_eq(left: &str, right: &str) -> bool {
    let (l, r) = (left.as_bytes(), right.as_bytes());
    if l.len() != r.len() {
        return false;
    }
    let mut i = 0;
    while i < l.len() {
        if l[i] != r[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// The field a set bit announces, or `None` for a bit this grammar does not
/// define.
pub const fn row_flag_name(flag: u32) -> Option<&'static str> {
    let mut i = 0;
    while i < ROW_FLAGS.len() {
        if ROW_FLAGS[i].0 == flag {
            return Some(ROW_FLAGS[i].1);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_flag_names_the_field_it_announces_by_its_own_literal() {
        // LITERALS, not the table this exists to check: reading the expected
        // name out of `ROW_FLAGS` passes while two of its rows are exchanged.
        assert_eq!(row_flag_name(ROW_FLAG_DELETED), Some("deleted"));
        assert_eq!(row_flag_name(ROW_FLAG_TASK_FORCE), Some("task_force"));
        assert_eq!(row_flag_name(ROW_FLAG_DEFINITION), Some("definition"));
        assert_eq!(row_flag_name(ROW_FLAG_OWNER), Some("owner"));
        assert_eq!(ROW_FLAG_DELETED, 1);
        assert_eq!(ROW_FLAG_TASK_FORCE, 2);
        assert_eq!(ROW_FLAG_DEFINITION, 4);
        assert_eq!(ROW_FLAG_OWNER, 8);
        assert_eq!(row_flag_name(1 << 4), None);
        assert_eq!(row_flag_name(0), None);
        assert_eq!(KNOWN_ROW_FLAGS, 0b1111);
    }

    #[test]
    fn the_row_grammar_is_the_one_the_producer_writes() {
        assert_eq!(HEADER_LEN, 40);
        assert_eq!(FIXED_ROW_LEN, 40);
        assert_eq!(DEFINITION_LEN_PREFIX, 2);
        assert_eq!(PAYLOAD_VERSION, 2);
    }
}
