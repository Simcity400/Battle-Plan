//! D34 `air_base_status` byte grammar.
//!
//! WHY THIS CRATE EXISTS. The three ANCHOR KINDS -- what a row's `anchor_kind`
//! word means -- were declared four times, in four crates that cannot see each
//! other: the collector that writes the word, the host's domain-data authority
//! and world fold that read it, and the independent wire reference that decodes
//! a recording. Each declared `STATE = 0`, `HOST = 1`, `UNRESOLVED = 2` on its
//! own, and each was checked only against itself: renumbering the collector's
//! `STATE` to `3` compiled, and its own test compared the written word with the
//! same mutated constant, while every consumer went on reading `0` as `State`.
//! An ordinary state-anchored air base then reached them as an unknown kind and
//! was refused instead of decoded (lane UT, Codex round 41, finding 3).
//!
//! A LEAF CRATE, not a `compact_contract` module: `wire_format` is an
//! independent reference implementation whose production library code admits
//! `compact_contract` only under an owner-reviewed allow-list, and widening
//! that list would give this port the recorder's policy closure. The same
//! shape as `d11_grammar`, `d13_grammar`, `d48_grammar` and `id_set_codec` --
//! dependency-free, byte boundaries and coded meanings only.

#![forbid(unsafe_code)]

/// Bytes before the first row.
pub const HEADER_LEN: usize = 36;

/// Bytes per row. Fixed; there is no legacy form.
pub const ROW_LEN: usize = 44;

/// The header's version word. A payload declaring any other version is refused
/// rather than reinterpreted.
pub const PAYLOAD_VERSION: u32 = 1;

/// The ONLY legal row flag bit: any other bit set is a hard error.
pub const ROW_FLAG_DELETED: u32 = 1;

/// Reached through the per-state air-base table; `province_id` is the base's own.
pub const ANCHOR_KIND_STATE: u32 = 0;

/// Reached through the manager's stateless vector; the host handle is the only
/// placement the base has.
pub const ANCHOR_KIND_HOST: u32 = 1;

/// The wing-pool walk named this id and no `CAirBase` walk reached it.
pub const ANCHOR_KIND_POOL_OBSERVED_UNRESOLVED: u32 = 2;

/// Every anchor kind, in code order: `ANCHOR_KINDS[k as usize]` names `k`.
pub const ANCHOR_KINDS: [(u32, &str); 3] = [
    (ANCHOR_KIND_STATE, "state"),
    (ANCHOR_KIND_HOST, "host"),
    (ANCHOR_KIND_POOL_OBSERVED_UNRESOLVED, "pool_observed_unresolved"),
];

const _: () = {
    let mut i = 0;
    while i < ANCHOR_KINDS.len() {
        // EACH KIND IS ITS OWN POSITION, so a reader may index this table by
        // the word it read and a writer cannot renumber one kind without
        // moving it.
        assert!(
            ANCHOR_KINDS[i].0 == i as u32,
            "an anchor kind is not published at its own code",
        );
        let mut j = i + 1;
        while j < ANCHOR_KINDS.len() {
            assert!(
                !str_eq(ANCHOR_KINDS[i].1, ANCHOR_KINDS[j].1),
                "two anchor kinds are published under the same name",
            );
            j += 1;
        }
        i += 1;
    }
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

/// The name of an anchor kind, or `None` for a word this grammar does not know.
///
/// The DECODE side, so a consumer never writes the codes out again.
pub const fn anchor_kind_name(kind: u32) -> Option<&'static str> {
    if (kind as usize) < ANCHOR_KINDS.len() {
        Some(ANCHOR_KINDS[kind as usize].1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_anchor_kind_resolves_to_its_own_name_and_nothing_else_resolves() {
        assert_eq!(anchor_kind_name(ANCHOR_KIND_STATE), Some("state"));
        assert_eq!(anchor_kind_name(ANCHOR_KIND_HOST), Some("host"));
        assert_eq!(
            anchor_kind_name(ANCHOR_KIND_POOL_OBSERVED_UNRESOLVED),
            Some("pool_observed_unresolved")
        );
        assert_eq!(anchor_kind_name(ANCHOR_KINDS.len() as u32), None);
        assert_eq!(anchor_kind_name(u32::MAX), None);
        for (slot, (code, name)) in ANCHOR_KINDS.iter().enumerate() {
            assert_eq!(*code as usize, slot, "{name} is not at its own code");
            assert_eq!(anchor_kind_name(*code), Some(*name));
        }
    }

    #[test]
    fn the_row_grammar_is_the_one_the_producer_writes() {
        assert_eq!(HEADER_LEN, 36);
        assert_eq!(ROW_LEN, 44);
        assert_eq!(PAYLOAD_VERSION, 1);
        assert_eq!(ROW_FLAG_DELETED, 1);
    }
}
