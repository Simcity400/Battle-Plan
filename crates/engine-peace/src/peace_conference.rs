//! Peace-conference identity and turn state from the game-state roster.
//! The roster reader requires neither window refresh nor a game resolver call.
//! It records resident conferences, not participants, bids or settlement actions.
//!
//! Finalization applies outcomes, marks the conference finalized and destroys
//! its history tree, bidding turns and actions. Subsequent roster cleanup deletes
//! the conference and its participant and aggregate storage. Creation can apply
//! and finalize a civil-war or zero-factor conference immediately; a complete
//! conference may therefore occur between hourly samples.
//!
//! [`peace_finalizer_site`] supplies the pre-destruction observation used by
//! [`peace_settlement`]. History is ordered by country, turn and action; statuses
//! three and four are excluded from settlement. Reduced outcomes and automatic
//! transfers are retained separately because accepted bids do not establish
//! one state transfer per action. An empty action history can still accompany
//! territory transferred without a bid.
//!
//! Resident-conference saves retain participants, current bids and reduced outcomes
//! but finalization has already destroyed original action history. A completed
//! save cannot validate that original action list. A pre-finalization fresh save
//! can compare bid operands at its sampled hour; it cannot certify the eventual
//! deal, raw reference bits, finalized hour or a war identity. Stored owner changes
//! cannot recover a missed conference or unambiguously attribute overlapping
//! settlements and other effects. Conference IDs are distinct from war IDs, and
//! messages do not establish a war name.

use crate::{Memory, cgame_state, cpdx_array::Array, read, walk::Outcome};

/// Conference primary vtable.
pub const VTABLE_RVA: usize = 0x0295_0e58;
/// Conference full reference, two u32 words.
pub const REFERENCE: usize = 0x08;
/// Signed i32 turn, incremented live and rebuilt from saved history.
pub const CURRENT_TURN: usize = 0xd8;
/// Finalized byte set before history destruction and roster removal.
pub const FINALIZED: usize = 0xdd;

/// Raw country arrays used during settlement.
pub const WINNERS: usize = 0x60;
pub const LOSERS: usize = 0x78;
/// Owned participant arrays in settlement-application order.
pub const WINNER_PARTICIPANTS: usize = 0xf8;
pub const LIBERATED_PARTICIPANTS: usize = 0x128;
pub const SUBJECT_PARTICIPANTS: usize = 0x140;
pub const GOVERNMENT_PARTICIPANTS: usize = 0x158;
/// Country-keyed history header, sentinel pointer followed by u64 size.
pub const HISTORY: usize = 0x1f8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Full native reference, low u32 type and high u32 id.
    pub id: u64,
    /// Position in the roster; ordinal zero is processed first.
    pub ordinal: u32,
    pub current_turn: i32,
    /// Native byte, preserved without conversion to bool.
    pub finalized: u8,
}

/// Copies resident conferences, including any queued behind the first one, into
/// caller-owned rows. A readable zero-length roster is empty; an unreadable or
/// unauthenticated nonzero entry is unavailable. No UI state or pointers enter
/// the rows. Capacity stops the walk and reports every remaining roster entry.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    walk_buffer(memory, base, game_state, crate::walk::Buffer::Fixed(output))
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        if base == 0 || game_state == 0 {
            return Err("peace conference module or game state is null");
        }
        let expected_vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("peace conference module address overflow")?;
        // Roster append copies pointer slots using the signed capacity/count header.
        let header = game_state
            .checked_add(cgame_state::PEACE_CONFERENCES_DATA)
            .ok_or("peace conference roster address overflow")?;
        let roster = Array::read(memory, header, 8)
            .ok_or("peace conference roster unreadable or invalid")?;
        output.ensure(roster.count as usize)?;
        let len = output.len().min(roster.count as usize);
        for (index, slot) in output[..len].iter_mut().enumerate() {
            let address = roster
                .element(index as u32, 8)
                .ok_or("peace conference roster address overflow")?;
            let pointer = read::<u64>(memory, address)
                .and_then(|value| usize::try_from(value).ok())
                .filter(|&value| value != 0)
                .ok_or("peace conference pointer unreadable or null")?;
            let at = |offset| {
                pointer
                    .checked_add(offset)
                    .ok_or("peace conference object address overflow")
            };
            // Vtable and handle are adjacent. Authenticate before reading
            // non-header fields; no unrelated object bytes are copied.
            let identity =
                read::<[u8; 16]>(memory, pointer).ok_or("peace conference identity unreadable")?;
            if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected_vtable as u64 {
                return Err("peace conference vtable mismatch");
            }
            let id = u64::from_le_bytes(identity[REFERENCE..].try_into().unwrap());
            if id == 0 {
                return Err("non-null peace conference has null handle");
            }
            let mut turn = [0; 4];
            let mut finalized = [0; 1];
            if memory.copy_scattered(
                [at(CURRENT_TURN)?, at(FINALIZED)?],
                &mut [&mut turn, &mut finalized],
            ) != 2
            {
                return Err("peace conference turn or finalized byte unreadable");
            }
            *slot = Row {
                id,
                ordinal: index as u32,
                current_turn: i32::from_le_bytes(turn),
                finalized: finalized[0],
            };
        }
        // An in-place sort is bounded by caller capacity; native ordering is
        // retained separately in ordinal and duplicate handles cannot alias rows.
        output[..len].sort_unstable_by_key(|row| row.id);
        if output[..len]
            .windows(2)
            .any(|pair| pair[0].id == pair[1].id)
        {
            return Err("duplicate peace conference handle");
        }
        Ok(if len == roster.count as usize {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: roster.count - len as u32,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const BASE: usize = 0x140000000;
    const GAME: usize = 0x1000;
    const ARRAY: usize = 0x1600;
    const FIRST: usize = 0x1700;
    const SECOND: usize = 0x1900;

    fn put<const N: usize>(bytes: &mut [u8], address: usize, value: [u8; N]) {
        bytes[address - GAME..address - GAME + N].copy_from_slice(&value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0xc00];
        put(
            &mut bytes,
            GAME + cgame_state::PEACE_CONFERENCES_DATA,
            (ARRAY as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            GAME + cgame_state::PEACE_CONFERENCES_DATA + 8,
            2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            GAME + cgame_state::PEACE_CONFERENCES_COUNT,
            2_u32.to_le_bytes(),
        );
        for (ordinal, pointer, id, turn, finalized) in [
            (0, FIRST, u64::MAX - 1, i32::MIN, 0x80_u8),
            (1, SECOND, 1_u64, i32::MAX, 1_u8),
        ] {
            put(
                &mut bytes,
                ARRAY + ordinal * 8,
                (pointer as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer,
                ((BASE + VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(&mut bytes, pointer + REFERENCE, id.to_le_bytes());
            put(&mut bytes, pointer + CURRENT_TURN, turn.to_le_bytes());
            put(&mut bytes, pointer + FINALIZED, [finalized]);
        }
        bytes
    }

    fn capture(bytes: &[u8], output: &mut [Row]) -> Outcome {
        walk(&SliceMemory { base: GAME, bytes }, BASE, GAME, output)
    }

    #[test]
    fn native_ids_turns_finalized_bytes_and_roster_order_survive() {
        let bytes = fixture();
        let mut output = [Row::default(); 2];
        assert_eq!(capture(&bytes, &mut output), Outcome::Rows { len: 2 });
        assert_eq!(
            output[0],
            Row {
                id: 1,
                ordinal: 1,
                current_turn: i32::MAX,
                finalized: 1
            }
        );
        assert_eq!(
            output[1],
            Row {
                id: u64::MAX - 1,
                ordinal: 0,
                current_turn: i32::MIN,
                finalized: 0x80
            }
        );
    }

    #[test]
    fn capacity_reports_unread_conferences_without_touching_their_fields() {
        let bytes = fixture();
        // The second object is inaccessible, but lies beyond caller capacity.
        let bytes = &bytes[..SECOND - GAME];
        let mut output = [Row::default(); 1];
        assert_eq!(
            capture(bytes, &mut output),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(output[0].ordinal, 0);
        assert_eq!(
            capture(bytes, &mut []),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        assert!(matches!(
            capture(bytes, &mut [Row::default(); 2]),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn a_proven_empty_roster_is_distinct_from_an_invalid_header() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            GAME + cgame_state::PEACE_CONFERENCES_COUNT,
            0_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            GAME + cgame_state::PEACE_CONFERENCES_DATA,
            0_u64.to_le_bytes(),
        );
        assert_eq!(capture(&bytes, &mut []), Outcome::Rows { len: 0 });
        for count in [1_u32, 3, u32::MAX] {
            put(
                &mut bytes,
                GAME + cgame_state::PEACE_CONFERENCES_COUNT,
                count.to_le_bytes(),
            );
            assert!(matches!(
                capture(&bytes, &mut []),
                Outcome::Unavailable { .. }
            ));
        }
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: GAME,
                    bytes: &bytes
                },
                BASE,
                0,
                &mut []
            ),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn bad_identity_duplicate_keys_and_late_read_faults_do_not_publish_a_prefix() {
        for (address, value) in [
            (ARRAY + 8, 0_u64),
            (SECOND, 0_u64),
            (SECOND + REFERENCE, 0_u64),
            (SECOND + REFERENCE, u64::MAX - 1),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, value.to_le_bytes());
            let outcome = capture(&bytes, &mut [Row::default(); 2]);
            assert!(matches!(outcome, Outcome::Unavailable { .. }));
            assert_eq!(outcome.len(), 0);
        }
        let bytes = fixture();
        let bytes = &bytes[..SECOND + FINALIZED - GAME];
        assert!(matches!(
            capture(bytes, &mut [Row::default(); 2]),
            Outcome::Unavailable { .. }
        ));
    }
}
