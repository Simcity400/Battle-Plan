//! Stored faction goals, carried under complete native writer certificates.
//! Every occurrence has its own key so a changed scalar does not re-encode a
//! faction's whole goal list. Definitions resolve through the content stack.
use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FactionGoalStatusRow {
    pub faction: u64,
    pub manifest: Option<u32>,
}
typed_codec!(FactionGoalStatusRow, 64, |row| row.faction as u128,
    |key| Self { faction: key as u64, ..Default::default() },
    { 0 => (manifest): OptionalU32 }, |_row| Ok(()));

/// Native ordered lists have independent ordinals. Empty lists have no entries;
/// their owner's complete FactionGoalStatus row distinguishes this from an
/// absent faction. Nullable historical definitions remain explicit occurrences.
/// Kind 0 runtime goal, 1 completed history, 2 cancelled history, 3 extra slot,
/// 4 term date. These are recording list kinds, not authored goal terms.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FactionGoalEntryRow {
    pub faction: u64,
    pub kind: u8,
    pub ordinal: u32,
    pub token: Option<u32>,
    /// Runtime goals retain all native states, including pending transitions.
    pub status: i32,
    pub progress_raw: i64,
    pub active_sections: Vec<u32>,
    /// Stored raw game hour; a native null date remains a native null date.
    pub date: i32,
    /// Signed extra slots, including zero and negative values.
    pub slots: i32,
}
typed_codec!(FactionGoalEntryRow, 104,
|row| (row.faction as u128) << 40 | (row.kind as u128) << 32 | row.ordinal as u128,
|key| Self { faction: (key >> 40) as u64, kind: (key >> 32) as u8, ordinal: key as u32, ..Default::default() },
{ 0 => (token): OptionalU32, 1 => (status): I32, 2 => (progress_raw): I64, 3 => (date): I32, 4 => (slots): I32, 5 => (active_sections): U32List },
|row| {
    if row.kind > 4 { return Err(invalid("unknown faction goal list kind")); }
    if row.kind == 0 && row.token.is_none() { return Err(invalid("runtime faction goal has no definition")); }
    Ok(())
});

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn faction_goal_rows_keep_full_handles_pending_states_null_occurrences_and_signed_slots() {
        for kind in 0..=4 {
            let row = FactionGoalEntryRow {
                faction: u64::MAX,
                kind,
                ordinal: u32::MAX,
                token: (kind == 0).then_some(u32::MAX),
                status: 4,
                progress_raw: i64::MIN,
                active_sections: vec![0, u32::MAX],
                date: -17,
                slots: i32::MIN,
            };
            super::super::round_trip(&row);
            let mut next = row.clone();
            next.progress_raw += 1;
            assert_eq!(next.delta(Some(&row)).unwrap().unwrap().mask, 1 << 2);
        }
        for manifest in [None, Some(0), Some(u32::MAX)] {
            super::super::round_trip(&FactionGoalStatusRow {
                faction: u64::MAX,
                manifest,
            });
        }
    }
}
