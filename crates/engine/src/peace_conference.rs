//! CPeaceConference identity and turn state from the game's conference roster.
//!
//! BAF880 appends native objects to CGameState+4E0; BAFB50 visits every
//! pointer and BAFB00 advances the first conference. Reading this roster needs
//! neither the old window-refresh hook nor an engine reference-resolver call.
//! Participants, bids, claims and settlement actions are not represented here.
//!
//! ## Settlement research (a729, 2026-10-01)
//!
//! This section preserves the round-1 findings. The subsequent user decision
//! approved the pre-destruction observation now implemented by
//! `peace_finalizer_site` and `peace_settlement`; mentions of pending readers
//! below describe the research stage. The completed-save limitation remains.
//!
//! The hourly settlement design fails the lifetime gate. The addresses below
//! are virtual addresses, researched with hoi4re's decompile, indexed
//! instructions and file-backed token registrations. No new reader or hook
//! was installed. A finalized resident row does not certify retained actions.
//!
//! ### Lifetime and the missing history
//!
//! `140E2D2B0` rebuilds outcomes from history (`140E245D0`), applies the
//! beneficiary/loser operations, then calls post-peace cleanup `140E2E250`.
//! The latter calls telemetry `1419EAAC0`, then `140E30E30`. That finalizer
//! dispatches the end-of-conference on-actions and finishes with these exact
//! instructions:
//! - `140E31651`: set conference +0xDD to 1.
//! - `140E31672`: call `140BAE7C0` on the history tree at +0x1F8.
//! - `140E31677..140E31682`: reset the sentinel and zero its size at +0x200.
//!
//! `140BAE7C0` destroys every owned bidding turn before freeing its tree node;
//! the turn's destructor (`140E23760`) destroys its action pointers. Reading
//! after finalization cannot recover that history, even before roster removal.
//! The reduced participant outcomes remain until their conference is deleted;
//! they do not preserve the original per-turn action order and negotiators.
//!
//! `140BAFB50` collects objects whose +0xDD is set, swap-removes them and calls
//! their deleting destructor (`140BAF780` -> `140BAF0D0`). It can also finalize
//! an object whose factor at +0x28 is zero (`140E35D30`, then `140E2D2B0`)
//! and delete it in that same pass. The destructor owns and frees the
//! participant arrays, current actions, bidding aggregates and history tree;
//! it does not move them to an archive.
//!
//! `140BAFB00` invokes this cleanup before advancing its first remaining
//! conference. `CGameState::HourlyUpdate` calls it at `1401DEA05`, after the
//! calendar updates and before clearing tick_busy at `1401DEA0A`. The recorder
//! hook is at HourlyUpdate entry (`1401DCEF0`), so this cleanup precedes the
//! next sample. A normal daily surrender (`1406D4AB0`, called by
//! `CCountry::daily_update@1406DAAF0`) creates a conference via `140BAF880`.
//! Creation immediately runs `140E3A940`: its pure-civil-war / zero-factor
//! path applies outcomes and calls `140E2E250` without waiting for another
//! hour. Such a conference can be born, settled and deleted in one tick.
//! `CStartPeaceConferenceEffect::Execute@140365FD0` and
//! `CWhitePeaceEffect::Execute@140368800` also call `140BAFB50`; even a
//! conference finalized between ticks has no unconditional next-read lifetime.
//!
//! No persistent settlement archive was found in these paths. The summary
//! popup is not a copied conference: `140B6A900` stores the original pointer
//! at popup +0xAB8, and `140B6FF40` immediately builds presentation from its
//! participants. Telemetry `1419EAAC0` conditionally builds and sends metrics,
//! not a saved ordered-action roster. State owners, subjects, governments and
//! timed effects retain the consequences, not conference/action identities.
//! This is a bounded finding about the inspected paths, not proof that every
//! unrelated game subsystem lacks another history.
//!
//! ### Claims, bids and the save contract
//!
//! Conference +0x1A0/count +0x1AC is the current action-pointer queue
//! (`140E24350`). +0x1F8/size +0x200 is a country-reference-keyed tree:
//! node +0x20 is the raw country; node +0x28 holds an ordered pointer vector
//! of CPeaceBiddingTurn, capacity +0x30 and count +0x34 (`140E3DED0`). A turn
//! has country +0x08, action pointers +0x10/count +0x1C, and updated state ids
//! +0x28/count +0x34 (`1419E0D30`, `1419E1A20`). `140E38920` updates the
//! state (+0x228), ship (+0x238) and country (+0x248) bidding-aggregate trees.
//! These are mutable contest/claim data, not a persistent completed-deal log.
//!
//! `140E245D0` walks country nodes, turns, then actions in native order to
//! rebuild outcomes. It excludes status 3 or 4 (`14150C8D0`), groups state
//! splits and calls `140E3D170`. That updater records beneficiary state
//! pointers (+0x18/count +0x24), puppet/liberation/government participants and
//! navy allocations. `140E240A0` groups enabled stackable flags by token into
//! state lists. Neither all historical bids nor the current queue can be
//! labelled settled without applying the native status and split semantics.
//! Automatic civil-war settlement can also transfer unclaimed states without
//! a bid for each: `140E344F0` collects them at winner participant +0x148/count
//! +0x154, then `1414436E0` applies them through `1409D1710`. `141443200`
//! handles losing-country cleanup, not these transfers. An empty history is
//! not proof that the conference transferred no territory.
//!
//! `CPeaceConferenceManager::Serialize@140BAFF00` writes every resident pointer
//! as active_peace in roster order, with no finalized filter. The conference
//! writer `140E3DED0` emits the following complete outer field set (conditional
//! fields are written only when their native source is nonempty/set):
//! - id: the registered full reference through `14220B400`.
//! - actor, recipient: raw main loser +0x34 and main winner +0x38 as tag text.
//! - occupied_winners: +0x90/count +0x9C country words, when nonempty.
//! - peace_conference_name_state_id: signed +0x18.
//! - peace_threat: fixed5 +0x218; completed: true when +0xDD is set.
//! - factor: fixed5 +0x28; message: +0x40 string when length +0x50 is nonzero.
//! - winner_scope, loser_scope: signed +0x1C and +0x20 selectors.
//! - winners: participant +0xF8/count +0x104, keyed by participant +0x08.
//! - solo_winner: +0x170 participant's country, when the pointer is nonzero.
//! - losers: +0x110/count +0x11C, keyed by loser participant +0x10.
//! - liberated: +0x128/count +0x134, when nonempty, via secondary base +0x40.
//! - subject: +0x140/count +0x14C, when nonempty, via secondary base +0x50.
//! - civil_war_losers: +0xC0/count +0xCC raw countries, when nonempty.
//! - done: +0x1D8/size +0x1E0 country set, when nonempty.
//! - history: +0x1F8/size +0x200, when nonempty, with one bidding per turn.
//! - time_duration: elapsed wall-clock ticks from +0x268 plus +0x270;
//!   this is not an in-game finalization hour.
//!
//! Winner/loser blocks are participant objects, not action histories:
//! `1414478A0` writes country, original_score, score, score_distribution,
//! ratio, war_score_breakdown, non_refunded_score, paired score_from_countries
//! (canonical slots) / score_from_amounts, and taken_states_civil_war.
//! `1414477E0` writes screening_ic and conditional civil_war_target and
//! civil_war_enemy. Liberated `141447740` writes country and conditional
//! liberators (+0x48/count +0x54 in its primary object). Subject `141447840`
//! writes country, overlord (+0x58), is_main_puppet (+0x70), ratio (+0x68).
//! Force-government `1414476E0` substitutes enforcer for overlord, but its
//! conference outcome array +0x158/count +0x164 is not emitted by `140E3DED0`.
//! These fields' registrar literal loads are `140095883`, `140095610`,
//! `140096651` and `140095649`. Derived participant blocks need their own
//! readers if retained aggregates are used; none are implemented here.
//!
//! `CPeaceBiddingTurn::Serialize@1419E1A20` writes country and conditional
//! actions / states. Each actions entry is keyed by action +0x08's raw lexer
//! token. `CPeaceAction::Serialize@14150DD00` always writes name (+0x10
//! string), taker (+0x30 raw country), giver (+0x34), negotiator (+0x38),
//! cost (+0x40 i32) and status (+0x3C i32). The negotiator and taker can differ.
//! `14150DDB0` adds state (+0x48 i32) and the enabled bytes +0x4C..+0x4F:
//! demilitarized_zone, war_reparation, resource_rights, dismantle_industry.
//! `14150DC80` adds puppet (+0x50 raw country), including force-government
//! objects; `14150DCC0` adds liberate (+0x50 raw country).
//! `14150DE40` adds a non-null ship's full reference (+0x48 pointer, object's
//! +0x08 reference) and ratio (+0x50 fixed5) for navy actions. A country/state
//! target union alone therefore cannot represent every native action.
//! Puppet +0x58 is also cloned (`1415093E0`) but is not written by `14150DC80`;
//! its meaning is unproved and must not be silently omitted from a new capture.
//!
//! The save action factory `141509550` recognizes take_states (0x30CF),
//! liberate (0x30D0), puppet (0x30D1), force_government (0x30D2), and take_navy
//! (0x30EE). Stackable operations are bytes on state actions, not separate
//! history entries. These are evidence for this binary's supported object
//! forms, not a replay enum or a maximum number of countries/actions. Annex,
//! release and take-colonies labels must not be invented as raw action tokens.
//! Each future reader must authenticate the actual class and refuse unknown
//! layouts while preserving the raw token.
//!
//! Field spellings above were independently read from `140082CB0`'s indexed
//! registrar: file-backed id loaded into EDX, literal loaded into R8, then
//! `1424AFC10`. Examples: take_states binds at `14009553C`, actions at
//! `140095977`, bidding at `140096628`, history at `1400A490B`, state at
//! `140089B8A`, actor at `1400A5AA2`, and recipient at `1400A5ADB`.
//! `141506C80` localizes the action token's spelling to initialize name;
//! the navy constructor overrides it (`141506E90` / `141509550`). The saved
//! name is localized text, not a localisation key. `140E2B150` selects the
//! stackable operation keys; `140E264D0` / `140E27C40` select demand labels
//! and description keys. Future presentation must resolve recorded tokens
//! and evidenced naming operands through the recorded content stack, with
//! explicit unknowns; no English or vanilla key list belongs in the product.
//!
//! ### War link and a design for lead review
//!
//! The proved stored link is to participants: main loser +0x34 and main winner
//! +0x38, plus winners +0x60/count +0x6C and losers +0x78/count +0x84 in native
//! order (`140E3A940`, `140E2BD40`). `140E2BD40` resolves their bilateral war
//! through CDiplomacyStatus's country relation and relation +0x2E8, then
//! computes the sides. `1410F3C00` also resolves a war before creation, but
//! passes the country references, factor, side selectors and an empty message
//! to `140BAF880`, not that war pointer. No stored CWar identity was proved in
//! the conference. +0x18 is the main loser's original/current capital fallback
//! (`1406EA260`), not a war id; +0x40 is a supplied message, not a proved war
//! name. Retain the raw participants; do not invent a unique war join from them.
//!
//! Minimal proposed capture: a lead-approved observation immediately after
//! `140E31651` and before history destruction at `140E31672`, copying the
//! authenticated conference, raw participant order, turns/actions, status,
//! stackable flags, operands and reduced beneficiary/loser outcome lists into
//! bounded owned storage. Default transfers without an action need separate
//! provenance; they must not be assigned an invented raw action token. This
//! preserves native inputs after settlement has run, without engine getters.
//! Publish once through the existing hourly pipeline, with the observed game
//! hour and explicit loss/failure on insufficient capacity; never defer live
//! pointers. Detour feasibility, reentrancy, register preservation, load/branch
//! invalidation and the settlement-hour cost still need approval and validation.
//! A hook on the finalizer's return would already be too late.
//!
//! The fresh-save oracle must expose that normal finalization destroys history;
//! a completed save cannot independently validate the original action list.
//! A pre-finalization text save can validate bid fields, but cannot certify the
//! eventual deal. Tag text also cannot certify raw reference bits; the save
//! lacks an explicit current turn, finalized game hour, stored war handle and
//! puppet +0x58. Before teardown, writer traversal order can compare action
//! occurrences without a serialized ordinal. Owner differences can be marked
//! derived only for a known conference interval: they do not recover an
//! entirely missed conference, action types, negotiators or non-territorial
//! effects, and several conferences/effects in an interval make attribution
//! ambiguous. No cadence increase, schema change, hook, derived product or
//! oracle waiver is authorized by this research result. Implementation awaits
//! the lead's lifetime decision.

use crate::{Memory, cgame_state, cpdx_array::Array, read, walk::Outcome};

/// CPeaceConference primary vtable; E21DD0 installs VA14293A818 at instruction E21E09.
pub const VTABLE_RVA: usize = 0x0293_a818;
/// CPeaceConference +0x08, native type/id u32 pair; E21DD0 initializes it and BAF880 registers the object.
pub const REFERENCE: usize = 0x08;
/// CPeaceConference +0xD8 i32 turn; E3C2B0 increments it, E34E60 rebuilds it from saved bidding history.
pub const CURRENT_TURN: usize = 0xd8;
/// CPeaceConference +0xDD finalized byte; E30E30 sets it and destroys history, then BAFB50 removes flagged objects.
pub const FINALIZED: usize = 0xdd;
/// CPeaceConference +0x170 participant pointer; E3A940 assigns the sole scored winner, E3DED0 saves its reference. This is not completion storage.
pub const SOLE_SCORED_WINNER: usize = 0x170;

/// Raw country arrays: 140E3A940 builds them, 140E30E30 walks them.
pub const WINNERS: usize = 0x60;
pub const LOSERS: usize = 0x78;
/// Owned concrete participant arrays; 140E2D2B0 applies them in this order.
pub const WINNER_PARTICIPANTS: usize = 0xf8;
pub const LIBERATED_PARTICIPANTS: usize = 0x128;
pub const SUBJECT_PARTICIPANTS: usize = 0x140;
pub const GOVERNMENT_PARTICIPANTS: usize = 0x158;
/// Country-keyed history header (head then u64 size), 140E3DED0/140BAE7C0.
pub const HISTORY: usize = 0x1f8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Full native reference, low u32 type and high u32 id.
    pub id: u64,
    /// Position in the native roster. BAFB00 processes ordinal zero first.
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
    let result = (|| {
        if base == 0 || game_state == 0 {
            return Err("peace conference module or game state is null");
        }
        let expected_vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("peace conference module address overflow")?;
        // BAF880 passes container+8 to the ordinary CPdxArray append helper
        // 120090, which reads signed capacity+8/count+C and copies qword slots.
        let header = game_state
            .checked_add(cgame_state::PEACE_CONFERENCES_DATA)
            .ok_or("peace conference roster address overflow")?;
        let roster = Array::read(memory, header, 8)
            .ok_or("peace conference roster unreadable or invalid")?;
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
