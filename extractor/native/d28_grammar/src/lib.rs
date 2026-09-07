//! D28 `oob_commands` byte grammar.
//!
//! WHY THIS CRATE EXISTS. The four OOB command KINDS -- what a row's `kind`
//! word means -- were declared four times in four crates that cannot see each
//! other: the collector that writes the word, the host's domain-data authority
//! and world fold that read it, and the independent wire reference that decodes
//! a recording. Each was checked only against itself, so renumbering the
//! collector's `KIND_ARMY` compiled and passed the collector's own tests while
//! every consumer went on reading `1` as an army: a real army row was then
//! discarded as an invalid kind rather than decoded (lane UT, Codex round 42,
//! finding 7). The row shape and the row flags were duplicated the same way.
//!
//! A LEAF CRATE, for the reason `d34_grammar` is one: `wire_format` is an
//! independent reference implementation whose production library code admits
//! `compact_contract` only under an owner-reviewed allow-list.

#![forbid(unsafe_code)]

/// Bytes before the first command row, at the current payload version.
pub const HEADER_LEN: usize = 56;

/// The v1 header, kept because a v1 payload is still decodable.
pub const V1_HEADER_LEN: usize = 48;

/// The header's version word.
pub const PAYLOAD_VERSION: u32 = 3;

/// Bytes per command row.
pub const ROW_LEN: usize = 80;

/// Bytes per membership-edge row.
pub const MEMBER_ROW_LEN: usize = 8;

pub const KIND_ARMY: u32 = 1;
pub const KIND_ARMY_GROUP: u32 = 2;
pub const KIND_FLEET: u32 = 3;
pub const KIND_TASK_FORCE: u32 = 4;

/// Every command kind, in code order: `KINDS[k as usize - 1]` names `k`.
///
/// The kinds start at 1, so 0 is "no kind was written" and cannot be read as
/// one of them.
pub const KINDS: [(u32, &str); 4] = [
    (KIND_ARMY, "army"),
    (KIND_ARMY_GROUP, "army_group"),
    (KIND_FLEET, "fleet"),
    (KIND_TASK_FORCE, "task_force"),
];

pub const ROW_FLAG_DELETED: u32 = 1;
pub const ROW_FLAG_PARENT: u32 = 1 << 1;
pub const ROW_FLAG_OWNER: u32 = 1 << 2;
pub const ROW_FLAG_LEADER: u32 = 1 << 3;
pub const ROW_FLAG_LEADER_SKILL: u32 = 1 << 4;
pub const ROW_FLAG_MEMBER_COUNT_ENGINE: u32 = 1 << 5;
pub const ROW_FLAG_MISSING_MEMBER_COUNT: u32 = 1 << 6;
pub const ROW_FLAG_LOCATION: u32 = 1 << 7;
pub const ROW_FLAG_LOCATION_UNREADABLE: u32 = 1 << 8;

/// Every flag this grammar defines, so a consumer can refuse an unknown bit
/// without writing the set out again.
pub const KNOWN_ROW_FLAGS: u32 = ROW_FLAG_DELETED
    | ROW_FLAG_PARENT
    | ROW_FLAG_OWNER
    | ROW_FLAG_LEADER
    | ROW_FLAG_LEADER_SKILL
    | ROW_FLAG_MEMBER_COUNT_ENGINE
    | ROW_FLAG_MISSING_MEMBER_COUNT
    | ROW_FLAG_LOCATION
    | ROW_FLAG_LOCATION_UNREADABLE;

const _: () = {
    let mut i = 0;
    while i < KINDS.len() {
        // EACH KIND IS ITS OWN POSITION (one-based), so a reader may index this
        // table by the word it read and a writer cannot renumber one kind
        // without moving it.
        assert!(
            KINDS[i].0 == i as u32 + 1,
            "an OOB command kind is not published at its own code",
        );
        let mut j = i + 1;
        while j < KINDS.len() {
            assert!(
                !str_eq(KINDS[i].1, KINDS[j].1),
                "two OOB command kinds are published under the same name",
            );
            j += 1;
        }
        i += 1;
    }
    assert!(
        V1_HEADER_LEN < HEADER_LEN,
        "the v1 header is not shorter than the current one, so the version \
         word cannot sit past it",
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

/// The name of a command kind, or `None` for a word this grammar does not know.
pub const fn kind_name(kind: u32) -> Option<&'static str> {
    if kind >= 1 && (kind as usize) <= KINDS.len() {
        Some(KINDS[kind as usize - 1].1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_resolves_to_its_own_name_and_nothing_else_resolves() {
        // THE NAMES AS LITERALS. Taking the expected name from the table this
        // test exists to check made it pass while `"army"` and `"army_group"`
        // were exchanged in it, so `kind_name(KIND_ARMY)` answered
        // `"army_group"` (Codex round 43, finding 4).
        assert_eq!(kind_name(KIND_ARMY), Some("army"));
        assert_eq!(kind_name(KIND_ARMY_GROUP), Some("army_group"));
        assert_eq!(kind_name(KIND_FLEET), Some("fleet"));
        assert_eq!(kind_name(KIND_TASK_FORCE), Some("task_force"));
        for (slot, (code, name)) in KINDS.iter().enumerate() {
            assert_eq!(*code as usize, slot + 1, "{name} is not at its own code");
            assert_eq!(kind_name(*code), Some(*name));
        }
        assert_eq!(kind_name(0), None);
        assert_eq!(kind_name(KINDS.len() as u32 + 1), None);
        assert_eq!(kind_name(u32::MAX), None);
    }

    #[test]
    fn the_known_flag_mask_is_exactly_the_flags_this_grammar_defines() {
        let flags = [
            ROW_FLAG_DELETED,
            ROW_FLAG_PARENT,
            ROW_FLAG_OWNER,
            ROW_FLAG_LEADER,
            ROW_FLAG_LEADER_SKILL,
            ROW_FLAG_MEMBER_COUNT_ENGINE,
            ROW_FLAG_MISSING_MEMBER_COUNT,
            ROW_FLAG_LOCATION,
            ROW_FLAG_LOCATION_UNREADABLE,
        ];
        let mut union = 0u32;
        for (index, flag) in flags.iter().enumerate() {
            assert_eq!(*flag, 1 << index, "flag {index} is not its own bit");
            assert_eq!(union & flag, 0, "flag {index} shares a bit with an earlier one");
            union |= flag;
        }
        assert_eq!(union, KNOWN_ROW_FLAGS);
        assert_eq!(KNOWN_ROW_FLAGS.count_ones() as usize, flags.len());
    }

    #[test]
    fn the_row_grammar_is_the_one_the_producer_writes() {
        assert_eq!(HEADER_LEN, 56);
        assert_eq!(V1_HEADER_LEN, 48);
        assert_eq!(PAYLOAD_VERSION, 3);
        assert_eq!(ROW_LEN, 80);
        assert_eq!(MEMBER_ROW_LEN, 8);
    }
}

#[path = "../../../../portable_replay/native/compact_stream/src/country_roster.rs"]
pub mod country_roster;

pub fn country_roster_bytes(payload: &[u8]) -> Result<&[u8], String> {
    if payload.len() < HEADER_LEN { return Err("D28 header truncated".into()); }
    let word = |at| u32::from_le_bytes(payload[at..at + 4].try_into().unwrap());
    if word(48) != PAYLOAD_VERSION || word(52) != 0 { return Err("D28 version or reserved header refused".into()); }
    let at = (word(8) as usize).checked_mul(ROW_LEN).and_then(|n| n.checked_add(HEADER_LEN))
        .and_then(|n| (word(32) as usize).checked_mul(MEMBER_ROW_LEN).and_then(|m| n.checked_add(m)))
        .ok_or("D28 roster offset overflow")?;
    let bytes = payload.get(at..).ok_or("D28 rows truncated")?;
    country_roster::Observation::parse(bytes, country_roster::Kind::Fleet)?;
    Ok(bytes)
}
