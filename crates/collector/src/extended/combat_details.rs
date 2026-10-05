//! Hourly combat-window and marker operands. One bounded census feeds the
//! independent tables. Division destinations grow from their original counts on the
//! acquisition worker; projection and other capacity growth stay on the spool.
//!
//! The pass holds the simulation thread, so the walk reuses what it has read
//! in the pass instead of reading it again: the census's authenticated
//! participants (`cland_combat::Members`) serve the commander, detail and unit
//! readers; each side's commander is selected once for the detail and tactic
//! readers (`cland_combat::Commanders`); a combat's divisions' army and stat block
//! fields are gathered in two calls (`combat_sources::Images`) for their four
//! readers and modifier member; and the division readers share per-source
//! results (`combat_sources::Sources`). Each reader meets a shared value, or
//! its failure, where it would have read it; the tests' reference walk
//! (`reference_walk`, test-only copies of `visit` and `detail`) shares none of
//! it and must publish the same.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::land_tactic_inputs;
use engine::{
    Memory, allocation, carmy_combat_inputs, cland_combat,
    combat_sources::{Images, Sources},
    land_reinforcement_inputs as reinforcement,
    walk::Outcome,
};
use replay::rows::{
    LandCombatCommanderTimedTrait, LandCombatTacticCountryWeight, LandCombatTacticInputs, RowCodec,
};
use replay::{
    frame::TableView,
    rows::{
        CombatModifierValue, DivisionCombatModifiers, LandCombatCommanderLearningTrait,
        LandCombatCommanderTrait, LandCombatDetailsRow, LandCombatDivisionRow,
        LandCombatExileInputs, LandCombatModifierRow, LandCombatSideDetails, LandCombatSideRow,
        LandDivisionContext, LandDivisionModifier, LandDivisionWidthTerm,
        LandReinforcementAcclimatization, LandReinforcementInputs, LandReinforcementModifier,
        LandReinforcementSupplyTerm, LandReinforcementTransportTerm, TableId, tables,
    },
};
use std::{
    collections::{HashMap, hash_map::Entry},
    mem::size_of,
};

pub const N: usize = 5;
pub const TABLES: [TableId; N] = [
    TableId::LandCombatDetails,
    TableId::LandCombatDivisions,
    TableId::LandCombatModifiers,
    TableId::LandCombatSides,
    TableId::ReserveModifierBreakdowns,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "land combat detail census and member authentication",
    0,
    &[0, 1, 2, 3, 4],
)];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    combats: usize,
    census_participants: usize,
    details: usize,
    divisions: usize,
    sides: usize,
    combat_sides: usize,
    country_tags: usize,
    rosters: usize,
    modifier_members: usize,
    modifiers: usize,
    width_terms: usize,
    exile_tags: usize,
    /// Name bytes: player names and each pass's group keys.
    division_names: usize,
    /// Pass-scoped naming memo sizes (template handles, group pointers).
    name_templates: usize,
    name_groups: usize,
    reinforcement_modifiers: usize,
    supply_terms: usize,
    acclimatization: usize,
    transport_terms: usize,
    division_commanders: usize,
    /// Division commanders' raw name tokens, bytes.
    commander_names: usize,
    commander_keys: usize,
    /// Every detail list is sized from its live vectors' demand: all token
    /// bytes, commander assigned/timed/learning traits and air-support counts
    /// (committed and reserve handles use `rosters`).
    commander_traits: usize,
    detail_text: usize,
    timed_traits: usize,
    air_counts: usize,
    learning_traits: usize,
}
impl Capacities {
    /// Presize from the live country registry. Each exile list holds at most
    /// one tag per country slot (the reader refuses longer ones), so twice the
    /// slot count always fits one committed division's cores and occupiers.
    /// The naming memos are caches sized to the same scale; misses only cost
    /// reads.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.exile_tags, crate::country_scale::rows(slots, 2));
        crate::country_scale::raise(&mut self.name_templates, slots.saturating_mul(4));
        crate::country_scale::raise(&mut self.name_groups, slots.saturating_mul(2));
    }
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            combats: 512,
            census_participants: 8192,
            details: 64,
            divisions: 1024,
            sides: 128,
            combat_sides: 512,
            country_tags: 2048,
            rosters: 2048,
            modifier_members: 1024,
            modifiers: 16384,
            width_terms: 8192,
            exile_tags: 64,
            division_names: 16384,
            name_templates: 1024,
            name_groups: 256,
            reinforcement_modifiers: 16384,
            supply_terms: 4096,
            acclimatization: 2048,
            transport_terms: 1024,
            division_commanders: 32,
            commander_names: 2048,
            commander_keys: 2048,
            commander_traits: 256,
            detail_text: 8192,
            timed_traits: 64,
            air_counts: 256,
            learning_traits: 1024,
        }
    }
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Detail {
    row: cland_combat::DetailsRow,
    tactic_context: land_tactic_inputs::ContextRow,
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Division {
    pointer: usize,
    combat: u64,
    side: u8,
    ordinal: u32,
    province: u32,
    role: u8,
    stats: carmy_combat_inputs::Row,
    exile: Option<carmy_combat_inputs::ExileRow>,
    commander_input: Option<usize>,
    context: carmy_combat_inputs::ContextRow,
    width_start: usize,
    reinforcement: reinforcement::ContextRow,
    reinforcement_starts: [usize; 4],
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct ModifierSide {
    combat: u64,
    side: u8,
    border_values: Option<[i64; 3]>,
    start: usize,
    len: usize,
}
/// One combat's side country sets, with the indicator and flag words read
/// with its modifier context (1418B4BD0's operands).
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct CombatSides {
    sets: cland_combat::CountrySetsRow,
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct ModifierMember {
    division: u64,
    ordinal: u32,
    start: usize,
    len: usize,
}
#[derive(Default, Clone, Copy)]
struct CommanderKey {
    handle: u64,
    index: usize,
}
#[derive(Default, Clone, Copy)]
struct Progress {
    len: usize,
    required: usize,
    failure: Option<&'static str>,
}
impl Progress {
    fn outcome(self, parent: Outcome) -> Outcome {
        match (self.failure, parent) {
            (Some(reason), _) | (_, Outcome::Unavailable { reason }) => {
                Outcome::Unavailable { reason }
            }
            // The parent count is combats, not divisions or modifier sides. Do
            // not label an unknown dependent census as a counted prefix.
            (_, Outcome::Truncated { .. }) => Outcome::Unavailable {
                reason: "land combat detail source census truncated",
            },
            _ if self.required > self.len => Outcome::Truncated {
                len: self.len,
                unread_rows: self
                    .required
                    .saturating_sub(self.len)
                    .min(u32::MAX as usize) as u32,
            },
            _ => Outcome::Rows { len: self.len },
        }
    }
}
/// Where one division sits in its combat, and where its exile tags start.
#[derive(Clone, Copy)]
struct Placement {
    pointer: usize,
    combat: u64,
    side: u8,
    ordinal: u32,
    province: u32,
    role: u8,
    exile_start: usize,
}
/// One division's successful native reads, before admission. Its term
/// prefixes are in the walk's [`Scratch`].
struct Acquired {
    stats: carmy_combat_inputs::Row,
    exile: carmy_combat_inputs::Exile,
    context: carmy_combat_inputs::ContextRow,
    reinforcement: reinforcement::ContextRow,
}

/// Per-division and per-combat reader storage the walk reuses, sized once at
/// each reader's bound, instead of zeroing about 15 KB for every division.
/// Readers write only the prefix they return.
#[derive(Default)]
struct Scratch {
    widths: Vec<carmy_combat_inputs::WidthTerm>,
    reinforcement_modifiers: Vec<reinforcement::ModifierTerm>,
    supply: Vec<reinforcement::SupplyTerm>,
    acclimatization: Vec<reinforcement::AcclimatizationTerm>,
    modifiers: Vec<carmy_combat_inputs::Modifier>,
}
impl Scratch {
    /// Sizes each buffer at its reader's bound; a no-op once sized.
    fn ensure(&mut self) -> bool {
        grow_vec(&mut self.widths, carmy_combat_inputs::WIDTH_TERM_LIMIT)
            && grow_vec(
                &mut self.reinforcement_modifiers,
                reinforcement::MODIFIER_LIMIT,
            )
            && grow_vec(&mut self.supply, reinforcement::SUPPLY_TERM_LIMIT)
            && grow_vec(
                &mut self.acclimatization,
                reinforcement::ACCLIMATIZATION_LIMIT,
            )
            && grow_vec(&mut self.modifiers, carmy_combat_inputs::MODIFIER_LIMIT)
    }
    fn allocated_bytes(&self) -> usize {
        self.widths.capacity() * size_of::<carmy_combat_inputs::WidthTerm>()
            + self.reinforcement_modifiers.capacity() * size_of::<reinforcement::ModifierTerm>()
            + self.supply.capacity() * size_of::<reinforcement::SupplyTerm>()
            + self.acclimatization.capacity() * size_of::<reinforcement::AcclimatizationTerm>()
            + self.modifiers.capacity() * size_of::<carmy_combat_inputs::Modifier>()
    }
}

/// One division's stats, exile, context and reinforcement rows.
type DivisionRead = (
    carmy_combat_inputs::Row,
    carmy_combat_inputs::Exile,
    carmy_combat_inputs::ContextRow,
    reinforcement::ContextRow,
);

/// Where one division's reads write their variable-length results: the
/// naming memo and name arena, and the exile tags past the arena's length.
struct DivisionOutputs<'r, 'n> {
    name_groups: &'r mut engine::cname_group_member::GroupKeys,
    names: &'r mut engine::cname_group_member::Names<'n>,
    exile_tags: &'r mut [u32],
    transport_terms: &'r mut Vec<reinforcement::TransportTerm>,
    transport_start: usize,
}

/// One division's readers in the walk's order, through `memory` (the
/// division's image, engine::combat_sources), into `scratch`. `countries` is
/// the combat's one country reader.
#[allow(clippy::too_many_arguments)]
fn read_division<M: Memory, C: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    countries: &Result<engine::ccountry::Reader<'_, C>, &'static str>,
    sources: &mut Sources,
    scratch: &mut Scratch,
    outputs: DivisionOutputs<'_, '_>,
    role: u8,
    pointer: usize,
    handle: u64,
) -> Result<DivisionRead, &'static str> {
    let stats = carmy_combat_inputs::capture_stats(
        memory,
        base,
        sources,
        pointer,
        handle,
        outputs.name_groups,
        outputs.names,
    )?;
    let countries = countries.as_ref().map_err(|reason| *reason)?;
    // 1415CA660 evaluates the exile frames for committed entries only.
    let exile = match role {
        1 => carmy_combat_inputs::capture_exile(memory, countries, pointer, outputs.exile_tags)?,
        _ => carmy_combat_inputs::Exile::Absent,
    };
    let context = carmy_combat_inputs::capture_context(
        memory,
        base,
        countries,
        sources,
        pointer,
        handle,
        &mut scratch.widths,
    )?;
    let row = reinforcement::capture_context(
        memory,
        base,
        state,
        sources,
        pointer,
        handle,
        &mut scratch.reinforcement_modifiers,
        &mut scratch.supply,
        &mut scratch.acclimatization,
        outputs.transport_terms,
        outputs.transport_start,
    )?;
    Ok((stats, exile, context, row))
}

struct Destination<'a> {
    sources: &'a mut Sources,
    /// This combat's commander selections, shared by its readers.
    commanders: cland_combat::Commanders,
    /// This combat's division images.
    images: &'a mut Images,
    scratch: &'a mut Scratch,
    name_groups: &'a mut engine::cname_group_member::GroupKeys,
    names: engine::cname_group_member::Names<'a>,
    /// Tests only: read as main's readers read, sharing nothing between
    /// readers (no census members, selections, pass shares or division
    /// image) nor between divisions (no shared sources).
    #[cfg(test)]
    reference: bool,
    division_commanders: &'a mut Vec<engine::naval_commander_inputs::DivisionCommanderRow>,
    /// Kept commanders' name tokens; a token that does not fit is counted
    /// for growth and its divisions are omitted this hour.
    commander_names: engine::cname_group_member::Names<'a>,
    commander_progress: Progress,
    commander_keys: &'a mut [CommanderKey],
    commander_key_count: usize,
    required_commander_keys: usize,
    details: &'a mut [Detail],
    divisions: &'a mut Vec<Division>,
    sides: &'a mut [ModifierSide],
    combat_sides: &'a mut [CombatSides],
    country_tags: &'a mut [u32],
    country_tags_progress: Progress,
    /// Nodes one country set can hold: the live country count plus tag 0.
    country_set_limit: Result<usize, &'static str>,
    rosters: &'a mut [u64],
    commander_traits: &'a mut [cland_combat::CommanderTrait],
    detail_text: &'a mut [u8],
    timed_traits: &'a mut [cland_combat::CommanderTimedTrait],
    air_counts: &'a mut [i32],
    learning_traits: &'a mut [cland_combat::CommanderLearningTrait],
    unit_scratch: &'a mut Vec<usize>,
    /// Stored entries of committed details, and every entry read; the
    /// participant counts live in `arenas[0]`.
    detail_used: cland_combat::DetailCounts,
    detail_required: cland_combat::DetailCounts,
    members: &'a mut [ModifierMember],
    modifiers: &'a mut [carmy_combat_inputs::Modifier],
    width_terms: &'a mut Vec<carmy_combat_inputs::WidthTerm>,
    width_progress: Progress,
    exile_tags: &'a mut [u32],
    exile_progress: Progress,
    reinforcement_modifiers: &'a mut Vec<reinforcement::ModifierTerm>,
    supply_terms: &'a mut Vec<reinforcement::SupplyTerm>,
    acclimatization: &'a mut Vec<reinforcement::AcclimatizationTerm>,
    transport_terms: &'a mut Vec<reinforcement::TransportTerm>,
    reinforcement_progress: [Progress; 4],
    progress: [Progress; N],
    arenas: [Progress; N],
    ticks: [u64; N],
}
impl Destination<'_> {
    fn commander(&mut self, memory: &impl Memory, base: usize, handle: u64) -> Option<usize> {
        if handle == 0 {
            return None;
        }
        let hash = handle.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let hash = (hash ^ (hash >> 32)) as usize;
        for probe in 0..self.commander_keys.len() {
            let slot = hash.wrapping_add(probe) % self.commander_keys.len();
            let key = self.commander_keys[slot];
            if key.handle == handle {
                return (key.index != usize::MAX).then_some(key.index);
            }
            if key.handle != 0 {
                continue;
            }
            self.commander_keys[slot] = CommanderKey {
                handle,
                index: usize::MAX,
            };
            self.commander_key_count += 1;
            self.commander_progress.required += 1;
            self.required_commander_keys =
                self.required_commander_keys.max(self.commander_key_count);
            match engine::naval_commander_inputs::capture_division_commander(
                memory,
                base,
                handle,
                &mut self.commander_names,
            ) {
                Ok(Some(row)) => {
                    let index = self.commander_progress.len;
                    if !grow_vec(self.division_commanders, index + 1) {
                        return None;
                    }
                    self.division_commanders[index] = row;
                    self.commander_progress.len += 1;
                    // The token was written uncommitted at the arena's length.
                    self.commander_names.len += row.name_token.1;
                    self.commander_keys[slot].index = index;
                    return Some(index);
                }
                // Memoized unfit (index stays usize::MAX), so its demand counts
                // once and every division it leads is omitted this hour.
                Ok(None) => return None,
                Err(reason) => {
                    self.progress[1].failure = Some(reason);
                    return None;
                }
            }
        }
        self.required_commander_keys = self.commander_keys.len().saturating_add(1);
        None
    }
    /// Retains one division whose native reads all succeeded, or counts it
    /// out. A name, exile list or commander that has no room this hour omits
    /// the whole division (the table reports it as unread) and its demand is
    /// counted so the spool grows the arena for the next hour.
    fn admit(&mut self, memory: &impl Memory, base: usize, at: Placement, acquired: Acquired) {
        let Acquired {
            stats,
            exile,
            context,
            reinforcement,
        } = acquired;
        let commander_input = self.commander(memory, base, stats.commander);
        let (exile, exile_len) = match exile {
            carmy_combat_inputs::Exile::Absent => (Some(None), 0),
            carmy_combat_inputs::Exile::Row(mut row) => {
                row.cores.0 += at.exile_start;
                row.occupiers.0 += at.exile_start;
                (Some(Some(row)), row.cores.1 + row.occupiers.1)
            }
            carmy_combat_inputs::Exile::DoesNotFit { required } => (None, required),
        };
        self.exile_progress.required += exile_len;
        self.width_progress.required += context.width_term_count;
        let start = self.width_progress.len;
        let count = context.width_term_count;
        let counts = [
            reinforcement.modifier_count,
            reinforcement.supply_term_count,
            reinforcement.acclimatization_count,
            reinforcement.transport_term_count,
        ];
        let starts = self.reinforcement_progress.map(|p| p.len);
        for (p, count) in self.reinforcement_progress.iter_mut().zip(counts) {
            p.required += count;
        }
        let (Some(naming), Some(exile)) = (stats.naming, exile) else {
            return;
        };
        if stats.commander != 0 && commander_input.is_none() {
            return;
        }
        // Every prefix was read successfully above. Grow from those exact
        // counts and retain it now, without retrying native reads or
        // dropping the containing division.
        if !grow_vec(self.width_terms, start + count)
            || !grow_vec(self.reinforcement_modifiers, starts[0] + counts[0])
            || !grow_vec(self.supply_terms, starts[1] + counts[1])
            || !grow_vec(self.acclimatization, starts[2] + counts[2])
        {
            return;
        }
        let scratch = &*self.scratch;
        self.reinforcement_modifiers[starts[0]..starts[0] + counts[0]]
            .copy_from_slice(&scratch.reinforcement_modifiers[..counts[0]]);
        self.supply_terms[starts[1]..starts[1] + counts[1]]
            .copy_from_slice(&scratch.supply[..counts[1]]);
        self.acclimatization[starts[2]..starts[2] + counts[2]]
            .copy_from_slice(&scratch.acclimatization[..counts[2]]);
        // Transport terms were written in place at starts[3]; admission only
        // advances the length.
        for (p, count) in self.reinforcement_progress.iter_mut().zip(counts) {
            p.len += count;
        }
        self.width_terms[start..start + count].copy_from_slice(&scratch.widths[..count]);
        self.width_progress.len += count;
        self.exile_progress.len += exile_len;
        // The player's name was written uncommitted at the arena's length.
        debug_assert!(naming.custom.1 == 0 || naming.custom.0 == self.names.len);
        self.names.len += naming.custom.1;
        self.divisions[self.progress[1].len] = Division {
            pointer: at.pointer,
            combat: at.combat,
            side: at.side,
            ordinal: at.ordinal,
            province: at.province,
            role: at.role,
            stats,
            exile,
            commander_input,
            context,
            width_start: start,
            reinforcement,
            reinforcement_starts: starts,
        };
        self.progress[1].len += 1;
    }
    fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        source: cland_combat::Source,
        members: cland_combat::Members<'_>,
    ) {
        if crate::allocation::failed() {
            return;
        }
        // Selections are the combat's own.
        self.commanders = cland_combat::Commanders::default();
        // The census has already retained every participant in this combat;
        // its counts bound the division destination without another read.
        let bound =
            (source.attackers.count as usize).saturating_add(source.defenders.count as usize);
        if !grow_vec(self.divisions, self.progress[1].required + bound) {
            return;
        }
        // A side's committed and reserve lists are subsets of its full
        // participants, so the larger census count bounds the role scratch.
        if !grow_vec(
            self.unit_scratch,
            (source.attackers.count.max(source.defenders.count)) as usize,
        ) {
            return;
        }
        // One country reader per combat serves every division's contextual
        // owner lookup and exile operands; no division rebuilds it.
        let countries = engine::ccountry::Reader::new(memory, base, state);
        {
            let started = qpc();
            self.detail(memory, base, state, source, &members);
            self.ticks[0] += elapsed(started);
        }
        let mut side_indices = [None; 2];
        {
            let _reads = crate::read_accounting::table(TABLES[2]);
            let started = qpc();
            self.progress[2].required += 2;
            match cland_combat::read_modifiers_context(memory, base, source) {
                Ok(context) => {
                    for (side, index) in side_indices.iter_mut().enumerate() {
                        if self.progress[2].len < self.sides.len() {
                            let next = self.progress[2].len;
                            self.sides[next] = ModifierSide {
                                combat: source.id,
                                side: side as u8,
                                border_values: context.border_values[side],
                                start: self.arenas[1].len,
                                len: 0,
                            };
                            *index = Some(next);
                            self.progress[2].len += 1;
                        }
                    }
                }
                Err(reason) => {
                    self.progress[2].failure = Some(reason);
                }
            }
            self.ticks[2] += elapsed(started);
        }
        // Every division's army and stat block fields, two gathers for the
        // combat, for its four readers and its modifier member. Lent while
        // the visitor borrows `self`.
        let mut images = std::mem::take(self.images);
        if self.progress[1].failure.is_none() || self.progress[2].failure.is_none() {
            let _reads = crate::read_accounting::table(TABLES[1]);
            let started = qpc();
            if images
                .capture(memory, members.pointers)
                .map_err(crate::allocation::io)
                .is_err()
            {
                *self.images = images;
                return;
            }
            self.ticks[1] += elapsed(started);
        } else {
            images.clear();
        }
        {
            let _reads = crate::read_accounting::table(TABLES[3]);
            let started = qpc();
            self.combat_sides(memory, base, source);
            self.ticks[3] += elapsed(started);
        }
        {
            let started = qpc();
            let before = self.ticks[1] + self.ticks[2];
            // Moved out for the visitor's borrow of the destination; no allocation.
            let mut owned = std::mem::take(self.unit_scratch);
            let result = cland_combat::visit_units(
                memory,
                base,
                source,
                &members,
                &mut owned,
                |side, ordinal, pointer, handle, province, role| {
                    if crate::allocation::failed() {
                        return;
                    }
                    let row = ordinal as usize
                        + if side == 0 {
                            0
                        } else {
                            source.attackers.count as usize
                        };
                    let divisional = &images.division(memory, row, pointer);
                    {
                        let _reads = crate::read_accounting::table(TABLES[1]);
                        let started = qpc();
                        self.progress[1].required += 1;
                        if self.progress[1].failure.is_none() {
                            // Enemy transport terms go straight into the arena
                            // past its admitted length; the reader grows it on
                            // this worker, one successfully read term at a time.
                            let transport_start = self.reinforcement_progress[3].len;
                            // Exile tags go straight into the preallocated arena
                            // past its admitted length; an overflow is counted
                            // for the spool's growth, never allocated here.
                            let exile_start = self.exile_progress.len;
                            let result = read_division(
                                divisional,
                                base,
                                state,
                                &countries,
                                self.sources,
                                self.scratch,
                                DivisionOutputs {
                                    name_groups: self.name_groups,
                                    names: &mut self.names,
                                    exile_tags: &mut self.exile_tags[exile_start..],
                                    transport_terms: self.transport_terms,
                                    transport_start,
                                },
                                role,
                                pointer,
                                handle,
                            );
                            match result {
                                Ok((stats, exile, context, reinforcement)) => self.admit(
                                    memory,
                                    base,
                                    Placement {
                                        pointer,
                                        combat: source.id,
                                        side,
                                        ordinal,
                                        province,
                                        role,
                                        exile_start,
                                    },
                                    Acquired {
                                        stats,
                                        exile,
                                        context,
                                        reinforcement,
                                    },
                                ),
                                Err(reason) => self.progress[1].failure = Some(reason),
                            }
                        }
                        self.ticks[1] += elapsed(started);
                    }
                    if crate::allocation::failed() {
                        return;
                    }
                    {
                        let _reads = crate::read_accounting::table(TABLES[2]);
                        let started = qpc();
                        self.modifier_member(
                            divisional,
                            base,
                            pointer,
                            handle,
                            ordinal,
                            side_indices[side as usize],
                        );
                        self.ticks[2] += elapsed(started);
                    }
                },
            );
            *self.unit_scratch = owned;
            self.ticks[0] +=
                elapsed(started).saturating_sub(self.ticks[1] + self.ticks[2] - before);
            let result = result.and_then(|visited| match visited {
                cland_combat::Visited::Complete => Ok(()),
                cland_combat::Visited::Scratch { .. } => {
                    Err("land role subsets exceed their full participants")
                }
            });
            if let Err(reason) = result {
                for index in [1, 2] {
                    {
                        self.progress[index].failure = Some(reason);
                    }
                }
            }
        }
        *self.images = images;
    }
    /// The country sets with the marker's indicator and flag words, which
    /// `read_country_sets` reads itself.
    fn combat_sides(&mut self, memory: &impl Memory, base: usize, source: cland_combat::Source) {
        self.progress[3].required += 1;
        if self.progress[3].failure.is_some() {
            return;
        }
        let limit = match self.country_set_limit {
            Ok(limit) => limit,
            Err(reason) => {
                self.progress[3].failure = Some(reason);
                return;
            }
        };
        let start = self.country_tags_progress.len;
        match cland_combat::read_country_sets(
            memory,
            base,
            source,
            limit,
            &mut self.country_tags[start..],
        ) {
            Ok((mut row, count)) => {
                self.country_tags_progress.required += count;
                if self.progress[3].len == self.combat_sides.len() {
                    return;
                }
                row.start = start;
                self.combat_sides[self.progress[3].len] = CombatSides { sets: row };
                self.progress[3].len += 1;
                self.country_tags_progress.len += count;
            }
            Err(cland_combat::CountrySetError::Truncated { required }) => {
                self.country_tags_progress.required += required;
            }
            Err(cland_combat::CountrySetError::Unavailable(reason)) => {
                self.progress[3].failure = Some(reason);
            }
        }
    }
    fn detail(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        source: cland_combat::Source,
        members: &cland_combat::Members<'_>,
    ) {
        self.progress[0].required += 1;
        if self.progress[0].failure.is_some() {
            return;
        }
        let at = self.detail_used;
        let roster = self.arenas[0].len;
        let mut arenas = cland_combat::DetailArenas::new(
            &mut self.rosters[roster..],
            &mut self.air_counts[at.air..],
            &mut self.detail_text[at.text..],
            &mut self.commander_traits[at.traits..],
            &mut self.timed_traits[at.timed..],
            &mut self.learning_traits[at.learning..],
        );
        let result = cland_combat::read_details(
            memory,
            base,
            source,
            members,
            &mut self.commanders,
            &mut arenas,
        );
        let (complete, used, required) = (arenas.complete(), arenas.used, arenas.required);
        self.arenas[0].required += required.participants;
        for (total, value) in [
            (&mut self.detail_required.air, required.air),
            (&mut self.detail_required.text, required.text),
            (&mut self.detail_required.traits, required.traits),
            (&mut self.detail_required.timed, required.timed),
            (&mut self.detail_required.learning, required.learning),
        ] {
            *total += value;
        }
        match result {
            Ok(mut row) => {
                let context = land_tactic_inputs::capture_context(
                    memory,
                    base,
                    state,
                    source,
                    members,
                    &mut self.commanders,
                );
                if let Outcome::Unavailable { reason } = context.outcome {
                    self.progress[0].failure = Some(reason);
                    return;
                }
                // A detail whose lists did not all fit is never published;
                // its demand grows the arenas for the next hour.
                if self.progress[0].len == self.details.len() || !complete {
                    return;
                }
                let text = at.text;
                for side in &mut row.sides {
                    side.committed_start += roster;
                    side.reserve_start += roster;
                    side.traits_start += at.traits;
                    side.timed_traits_start += at.timed;
                    side.air_counts.start += at.air;
                    side.learning_traits.start += at.learning;
                    for token in [
                        &mut side.tactic_token,
                        &mut side.commander_name_token,
                        &mut side.commander_description_token,
                    ] {
                        token.start += text;
                    }
                }
                row.stored_terrain_name.start += text;
                for stored in &mut self.commander_traits[at.traits..at.traits + used.traits] {
                    stored.token_start += text;
                }
                for stored in &mut self.learning_traits[at.learning..at.learning + used.learning] {
                    stored.token_start += text;
                }
                self.arenas[0].len += used.participants;
                self.detail_used.air += used.air;
                self.detail_used.text += used.text;
                self.detail_used.traits += used.traits;
                self.detail_used.timed += used.timed;
                self.detail_used.learning += used.learning;
                self.details[self.progress[0].len] = Detail {
                    row,
                    tactic_context: context.row,
                };
                self.progress[0].len += 1;
            }
            Err(reason) => self.progress[0].failure = Some(reason),
        }
    }
    fn modifier_member(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        handle: u64,
        ordinal: u32,
        side: Option<usize>,
    ) {
        self.arenas[1].required += 1;
        if self.progress[2].failure.is_some() {
            return;
        }
        let scratch = &mut self.scratch.modifiers;
        match carmy_combat_inputs::capture_modifiers(memory, base, pointer, handle, scratch) {
            Ok(count) => {
                self.arenas[2].required += count;
                let Some(side) = side else {
                    return;
                };
                if self.arenas[1].len == self.members.len()
                    || count > self.modifiers.len().saturating_sub(self.arenas[2].len)
                {
                    // Omit the whole side during projection, never its last entries.
                    self.sides[side].len = usize::MAX;
                    return;
                }
                if self.sides[side].len == usize::MAX {
                    return;
                }
                if self.sides[side].len == 0 {
                    self.sides[side].start = self.arenas[1].len;
                }
                let start = self.arenas[2].len;
                self.modifiers[start..start + count]
                    .copy_from_slice(&self.scratch.modifiers[..count]);
                self.members[self.arenas[1].len] = ModifierMember {
                    division: handle,
                    ordinal,
                    start,
                    len: count,
                };
                self.arenas[1].len += 1;
                self.arenas[2].len += count;
                self.sides[side].len += 1;
            }
            Err(reason) => self.progress[2].failure = Some(reason),
        }
    }
    fn finish(&mut self) {
        let mut retained = 0;
        for index in 0..self.progress[2].len {
            if self.sides[index].len != usize::MAX {
                self.sides[retained] = self.sides[index];
                retained += 1;
            }
        }
        self.progress[2].len = retained;
    }
}

pub struct Captured {
    breakdowns: Vec<replay::rows::ReserveModifierBreakdownRow>,
    breakdown_outcome: Outcome,
    division_commanders: Arena<engine::naval_commander_inputs::DivisionCommanderRow>,
    commander_names: Arena<u8>,
    commander_keys: Vec<CommanderKey>,
    required_commander_keys: usize,
    details: CaptureRows<Detail>,
    divisions: CaptureRows<Division>,
    sides: CaptureRows<ModifierSide>,
    combat_sides: CaptureRows<CombatSides>,
    country_tags: Arena<u32>,
    rosters: Arena<u64>,
    members: Arena<ModifierMember>,
    modifiers: Arena<carmy_combat_inputs::Modifier>,
    width_terms: Arena<carmy_combat_inputs::WidthTerm>,
    exile_tags: Arena<u32>,
    division_names: Arena<u8>,
    name_cache: engine::cname_group_member::GroupKeys,
    reinforcement_modifiers: Arena<reinforcement::ModifierTerm>,
    supply_terms: Arena<reinforcement::SupplyTerm>,
    acclimatization: Arena<reinforcement::AcclimatizationTerm>,
    transport_terms: Arena<reinforcement::TransportTerm>,
    commander_traits: Arena<cland_combat::CommanderTrait>,
    detail_text: Arena<u8>,
    timed_traits: Arena<cland_combat::CommanderTimedTrait>,
    air_counts: Arena<i32>,
    learning_traits: Arena<cland_combat::CommanderLearningTrait>,
    /// One side's committed and reserve pointers while roles are visited;
    /// grown on the acquisition worker from the census's participant counts.
    unit_scratch: Vec<usize>,
    census: Vec<cland_combat::Row>,
    census_participants: Vec<u64>,
    /// Each census participant's pointer, beside its handle.
    census_pointers: Vec<usize>,
    required_census: usize,
    required_census_participants: usize,
    /// Per-pass shared division sources; cleared before each pass.
    sources: Sources,
    /// Each combat's division images, storage reused.
    images: Images,
    /// Reader storage reused across divisions and passes.
    scratch: Scratch,
    /// Tests only: the reference walk (see `Destination`).
    #[cfg(test)]
    reference: bool,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            combats: 0,
            census_participants: 0,
            details: 0,
            divisions: 0,
            sides: 0,
            combat_sides: 0,
            country_tags: 0,
            rosters: 0,
            modifier_members: 0,
            modifiers: 0,
            width_terms: 0,
            exile_tags: 0,
            division_names: 0,
            name_templates: 0,
            name_groups: 0,
            reinforcement_modifiers: 0,
            supply_terms: 0,
            acclimatization: 0,
            transport_terms: 0,
            division_commanders: 0,
            commander_names: 0,
            commander_keys: 0,
            commander_traits: 0,
            detail_text: 0,
            timed_traits: 0,
            air_counts: 0,
            learning_traits: 0,
        })
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            breakdowns: Vec::new(),
            breakdown_outcome: Outcome::Rows { len: 0 },
            division_commanders: Arena::new(c.division_commanders),
            commander_names: Arena::new(c.commander_names),
            commander_keys: allocation::owned_vec(allocation::default_vec(
                c.commander_keys.max(c.census_participants),
            )),
            required_commander_keys: 0,
            details: CaptureRows::new(c.details),
            divisions: CaptureRows::new(c.divisions),
            sides: CaptureRows::new(c.sides),
            combat_sides: CaptureRows::new(c.combat_sides),
            country_tags: Arena::new(c.country_tags),
            rosters: Arena::new(c.rosters),
            members: Arena::new(c.modifier_members),
            modifiers: Arena::new(c.modifiers),
            width_terms: Arena::new(c.width_terms),
            exile_tags: Arena::new(c.exile_tags),
            division_names: Arena::new(c.division_names),
            name_cache: engine::cname_group_member::GroupKeys::try_with_capacity(
                c.name_templates,
                c.name_groups,
            )
            .map_err(crate::allocation::io)
            .unwrap_or_default(),
            reinforcement_modifiers: Arena::new(c.reinforcement_modifiers),
            supply_terms: Arena::new(c.supply_terms),
            acclimatization: Arena::new(c.acclimatization),
            transport_terms: Arena::new(c.transport_terms),
            commander_traits: Arena::new(c.commander_traits),
            detail_text: Arena::new(c.detail_text),
            timed_traits: Arena::new(c.timed_traits),
            air_counts: Arena::new(c.air_counts),
            learning_traits: Arena::new(c.learning_traits),
            unit_scratch: Vec::new(),
            census: allocation::owned_vec(allocation::default_vec(c.combats)),
            census_participants: allocation::owned_vec(allocation::default_vec(
                c.census_participants,
            )),
            census_pointers: allocation::owned_vec(allocation::default_vec(c.census_participants)),
            required_census: 0,
            required_census_participants: 0,
            sources: Sources::new(),
            images: Images::default(),
            // Sized with the capture, off the game thread; a vacant
            // placeholder stays allocation-free and is sized if it is walked.
            scratch: if c.combats == 0 {
                Scratch::default()
            } else {
                let mut scratch = Scratch::default();
                scratch.ensure();
                scratch
            },
            #[cfg(test)]
            reference: false,
        }
    }
    pub fn reset(&mut self) {
        self.breakdowns.clear();
        self.breakdown_outcome = Outcome::Rows { len: 0 };
        self.division_commanders.reset();
        self.commander_names.reset();
        self.required_commander_keys = 0;
        self.details.reset();
        self.divisions.reset();
        self.sides.reset();
        self.combat_sides.reset();
        self.country_tags.reset();
        self.rosters.reset();
        self.members.reset();
        self.modifiers.reset();
        self.width_terms.reset();
        self.exile_tags.reset();
        self.division_names.reset();
        self.reinforcement_modifiers.reset();
        self.supply_terms.reset();
        self.acclimatization.reset();
        self.transport_terms.reset();
        self.commander_traits.reset();
        self.detail_text.reset();
        self.timed_traits.reset();
        self.air_counts.reset();
        self.learning_traits.reset();
        self.required_census = 0;
        self.required_census_participants = 0;
    }
    pub(crate) fn fail(&mut self, reason: &'static str) {
        self.reset();
        let outcome = Outcome::Unavailable { reason };
        self.details.outcome = outcome;
        self.divisions.outcome = outcome;
        self.sides.outcome = outcome;
        self.combat_sides.outcome = outcome;
        self.breakdown_outcome = outcome;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.breakdowns.capacity() * size_of::<replay::rows::ReserveModifierBreakdownRow>()
            + self
                .breakdowns
                .iter()
                .map(|r| {
                    r.nodes.capacity() * size_of::<engine::modifier_breakdown::Node>()
                        + r.nodes.iter().map(|n| n.name.capacity()).sum::<usize>()
                })
                .sum::<usize>()
            + self.details.allocated_bytes()
            + self.division_commanders.allocated_bytes()
            + self.commander_names.allocated_bytes()
            + self.commander_keys.capacity() * size_of::<CommanderKey>()
            + self.divisions.allocated_bytes()
            + self.sides.allocated_bytes()
            + self.combat_sides.allocated_bytes()
            + self.country_tags.allocated_bytes()
            + self.rosters.allocated_bytes()
            + self.members.allocated_bytes()
            + self.modifiers.allocated_bytes()
            + self.width_terms.allocated_bytes()
            + self.exile_tags.allocated_bytes()
            + self.division_names.allocated_bytes()
            + self.name_cache.allocated_bytes()
            + self.reinforcement_modifiers.allocated_bytes()
            + self.supply_terms.allocated_bytes()
            + self.acclimatization.allocated_bytes()
            + self.transport_terms.allocated_bytes()
            + self.commander_traits.allocated_bytes()
            + self.detail_text.allocated_bytes()
            + self.timed_traits.allocated_bytes()
            + self.air_counts.allocated_bytes()
            + self.learning_traits.allocated_bytes()
            + self.unit_scratch.capacity() * size_of::<usize>()
            + self.census.capacity() * size_of::<cland_combat::Row>()
            + self.census_participants.capacity() * size_of::<u64>()
            + self.census_pointers.capacity() * size_of::<usize>()
            + self.scratch.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        c.division_commanders = c
            .division_commanders
            .max(self.division_commanders.first.capacity());
        c.commander_keys = c.commander_keys.max(self.commander_keys.capacity());
        c.divisions = c.divisions.max(self.divisions.rows.capacity());
        c.width_terms = c.width_terms.max(self.width_terms.first.capacity());
        c.exile_tags = c.exile_tags.max(self.exile_tags.first.capacity());
        c.division_names = c.division_names.max(self.division_names.first.capacity());
        c.commander_names = c.commander_names.max(self.commander_names.first.capacity());
        c.reinforcement_modifiers = c
            .reinforcement_modifiers
            .max(self.reinforcement_modifiers.first.capacity());
        c.supply_terms = c.supply_terms.max(self.supply_terms.first.capacity());
        c.acclimatization = c.acclimatization.max(self.acclimatization.first.capacity());
        c.transport_terms = c.transport_terms.max(self.transport_terms.first.capacity());
        self.division_commanders.grow(&mut c.division_commanders);
        self.commander_names.grow(&mut c.commander_names);
        grow_capacity(&mut c.commander_keys, self.required_commander_keys);
        grow_vec(&mut self.commander_keys, c.commander_keys);
        self.details.grow(&mut c.details);
        self.divisions.grow(&mut c.divisions);
        self.sides.grow(&mut c.sides);
        self.combat_sides.grow(&mut c.combat_sides);
        self.country_tags.grow(&mut c.country_tags);
        self.rosters.grow(&mut c.rosters);
        self.members.grow(&mut c.modifier_members);
        self.modifiers.grow(&mut c.modifiers);
        self.width_terms.grow(&mut c.width_terms);
        self.exile_tags.grow(&mut c.exile_tags);
        self.division_names.grow(&mut c.division_names);
        // The memo tables are caches: they are rebuilt only when the country
        // scale raised their size, never from a pass's demand.
        let current = self.name_cache.capacities();
        let sized = [c.name_templates, c.name_groups].map(|n| n.checked_next_power_of_two());
        if sized
            .iter()
            .zip(current)
            .any(|(wanted, current)| wanted.is_some_and(|wanted| wanted > current))
        {
            let Ok(cache) = engine::cname_group_member::GroupKeys::try_with_capacity(
                c.name_templates,
                c.name_groups,
            )
            .map_err(crate::allocation::io) else {
                return;
            };
            self.name_cache = cache;
        }
        self.reinforcement_modifiers
            .grow(&mut c.reinforcement_modifiers);
        self.supply_terms.grow(&mut c.supply_terms);
        self.acclimatization.grow(&mut c.acclimatization);
        self.transport_terms.grow(&mut c.transport_terms);
        self.commander_traits.grow(&mut c.commander_traits);
        self.detail_text.grow(&mut c.detail_text);
        self.timed_traits.grow(&mut c.timed_traits);
        self.air_counts.grow(&mut c.air_counts);
        self.learning_traits.grow(&mut c.learning_traits);
        grow_capacity(&mut c.combats, self.required_census);
        grow_capacity(
            &mut c.census_participants,
            self.required_census_participants,
        );
        grow_vec(&mut self.census, c.combats);
        grow_vec(&mut self.census_participants, c.census_participants);
        grow_vec(&mut self.census_pointers, c.census_participants);
        c.commander_keys = c.commander_keys.max(self.census_participants.len());
        grow_vec(&mut self.commander_keys, c.commander_keys);
    }
    fn pass(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> ([Outcome; N], [u64; N]) {
        self.sources.clear();
        if crate::allocation::failed() || !self.scratch.ensure() {
            return (
                [Outcome::Unavailable {
                    reason: "collector allocation failed",
                }; N],
                [0; N],
            );
        }
        let _reads = crate::read_accounting::table(TABLES[0]);
        // Naming memos never outlive the pass that authenticated them.
        self.name_cache.reset();
        let started = qpc();
        let commander_started = qpc();
        // walk_with visits a combat only after every participant fits in this
        // owned census. Each visited participant has at most one commander, so
        // this bounds all distinct nonzero keys before the first insertion.
        if !grow_vec(&mut self.commander_keys, self.census_participants.len()) {
            return (
                [Outcome::Unavailable {
                    reason: "collector allocation failed",
                }; N],
                [0; N],
            );
        }
        self.commander_keys.fill(CommanderKey::default());
        let commander_ticks = elapsed(commander_started);
        let mut destination = Destination {
            sources: &mut self.sources,
            commanders: cland_combat::Commanders::default(),
            images: &mut self.images,
            scratch: &mut self.scratch,
            name_groups: &mut self.name_cache,
            names: engine::cname_group_member::Names {
                bytes: &mut self.division_names.first,
                len: 0,
                required: 0,
            },
            #[cfg(test)]
            reference: self.reference,
            division_commanders: &mut self.division_commanders.first,
            commander_names: engine::cname_group_member::Names {
                bytes: &mut self.commander_names.first,
                len: 0,
                required: 0,
            },
            commander_progress: Progress::default(),
            commander_keys: &mut self.commander_keys,
            commander_key_count: 0,
            required_commander_keys: 0,
            details: &mut self.details.rows,
            divisions: &mut self.divisions.rows,
            sides: &mut self.sides.rows,
            combat_sides: &mut self.combat_sides.rows,
            country_tags: &mut self.country_tags.first,
            country_tags_progress: Progress::default(),
            country_set_limit: engine::ccountry::Reader::new(memory, base, state)
                .map(|countries| countries.count() as usize + 1),
            rosters: &mut self.rosters.first,
            commander_traits: &mut self.commander_traits.first,
            detail_text: &mut self.detail_text.first,
            timed_traits: &mut self.timed_traits.first,
            air_counts: &mut self.air_counts.first,
            learning_traits: &mut self.learning_traits.first,
            unit_scratch: &mut self.unit_scratch,
            detail_used: cland_combat::DetailCounts::default(),
            detail_required: cland_combat::DetailCounts::default(),
            members: &mut self.members.first,
            modifiers: &mut self.modifiers.first,
            width_terms: &mut self.width_terms.first,
            width_progress: Progress::default(),
            exile_tags: &mut self.exile_tags.first,
            exile_progress: Progress::default(),
            reinforcement_modifiers: &mut self.reinforcement_modifiers.first,
            supply_terms: &mut self.supply_terms.first,
            acclimatization: &mut self.acclimatization.first,
            transport_terms: &mut self.transport_terms.first,
            reinforcement_progress: [Progress::default(); 4],
            progress: [Progress::default(); N],
            arenas: [Progress::default(); N],
            ticks: [0, commander_ticks, 0, 0, 0],
        };
        let census = cland_combat::walk_members(
            memory,
            base,
            state,
            &mut self.census,
            &mut self.census_participants,
            &mut self.census_pointers,
            |source, members| {
                // Tests only: the reference walk (`reference_walk`).
                #[cfg(test)]
                if destination.reference {
                    return destination.visit_reference(memory, base, state, source, members);
                }
                destination.visit(memory, base, state, source, members)
            },
        );
        destination.finish();
        let outcomes = destination.progress.map(|p| p.outcome(census.outcome));
        let arenas = destination.arenas;
        let commanders = destination.commander_progress;
        self.required_commander_keys = self
            .required_commander_keys
            .max(destination.required_commander_keys);
        let width = destination.width_progress;
        let exile = destination.exile_progress;
        let names = (destination.names.len, destination.names.required);
        let commander_names = (
            destination.commander_names.len,
            destination.commander_names.required,
        );
        let country_tags = destination.country_tags_progress;
        let reinforcement = destination.reinforcement_progress;
        let (detail_used, detail_required) = (destination.detail_used, destination.detail_required);
        let mut ticks = destination.ticks;
        ticks[0] += elapsed(started).saturating_sub(ticks.iter().sum::<u64>());
        self.required_census = self.required_census.max(match census.outcome {
            Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
            _ => census.outcome.len(),
        });
        self.required_census_participants = self
            .required_census_participants
            .max(census.required_participants);
        self.division_commanders.len = commanders.len;
        self.division_commanders.required = commanders.required;
        self.rosters.len = arenas[0].len;
        self.rosters.required = arenas[0].required;
        self.commander_traits.len = detail_used.traits;
        self.commander_traits.required = detail_required.traits;
        self.detail_text.len = detail_used.text;
        self.detail_text.required = detail_required.text;
        self.timed_traits.len = detail_used.timed;
        self.timed_traits.required = detail_required.timed;
        self.air_counts.len = detail_used.air;
        self.air_counts.required = detail_required.air;
        self.learning_traits.len = detail_used.learning;
        self.learning_traits.required = detail_required.learning;
        self.width_terms.len = width.len;
        self.width_terms.required = width.required;
        self.exile_tags.len = exile.len;
        self.exile_tags.required = exile.required;
        (self.division_names.len, self.division_names.required) = names;
        (self.commander_names.len, self.commander_names.required) = commander_names;
        self.reinforcement_modifiers.len = reinforcement[0].len;
        self.reinforcement_modifiers.required = reinforcement[0].required;
        self.supply_terms.len = reinforcement[1].len;
        self.supply_terms.required = reinforcement[1].required;
        self.acclimatization.len = reinforcement[2].len;
        self.acclimatization.required = reinforcement[2].required;
        self.transport_terms.len = reinforcement[3].len;
        self.transport_terms.required = reinforcement[3].required;
        self.members.len = arenas[1].len;
        self.members.required = arenas[1].required;
        self.modifiers.len = arenas[2].len;
        self.modifiers.required = arenas[2].required;
        self.country_tags.len = country_tags.len;
        self.country_tags.required = country_tags.required;
        (outcomes, ticks)
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let started = qpc();
        let (outcomes, mut ticks) = self.pass(memory, base, state);
        self.details.outcome = outcomes[0];
        self.divisions.outcome = outcomes[1];
        self.sides.outcome = outcomes[2];
        self.combat_sides.outcome = outcomes[3];
        if crate::allocation::failed() {
            self.fail("collector allocation failed");
            return ticks;
        }
        let started_breakdowns = qpc();
        let _reads = crate::read_accounting::table(TABLES[4]);
        self.capture_breakdowns(memory, base, state);
        ticks[4] = elapsed(started_breakdowns);
        if crate::allocation::failed() {
            self.fail("collector allocation failed");
        }
        ticks[0] += elapsed(started).saturating_sub(ticks.iter().sum::<u64>());
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.details.outcome,
            self.divisions.outcome,
            self.sides.outcome,
            self.combat_sides.outcome,
            self.breakdown_outcome,
        ]
    }
    fn capture_breakdowns(&mut self, memory: &impl Memory, base: usize, state: usize) {
        self.breakdowns.clear();
        let result = (|| {
            if !matches!(self.divisions.outcome, Outcome::Rows { .. }) {
                return Err("reserve modifier breakdown division census incomplete");
            }
            if !self.divisions.rows[..self.divisions.outcome.len()]
                .iter()
                .any(|r| r.role == 2)
            {
                return Ok(());
            }
            let countries = engine::ccountry::Reader::new(memory, base, state)?;
            // Each reserve division selects at most its owner and commander.
            let bound = self.divisions.rows[..self.divisions.outcome.len()]
                .iter()
                .filter(|row| row.role == 2)
                .count()
                .checked_mul(2)
                .ok_or_else(|| {
                    allocation::oom();
                    "collector allocation failed"
                })?;
            let mut sources = HashMap::new();
            allocation::reserve_map(&mut sources, bound)
                .map_err(|_| "collector allocation failed")?;
            for row in &self.divisions.rows[..self.divisions.outcome.len()] {
                if row.role != 2 {
                    continue;
                }
                let owners = row
                    .pointer
                    .checked_add(engine::cunit::ROSTER_OWNER)
                    .and_then(|p| engine::read::<[u8; 8]>(memory, p))
                    .ok_or("reserve modifier breakdown owner unreadable")?;
                let own = u32::from_le_bytes(owners[..4].try_into().unwrap());
                let expeditionary = u32::from_le_bytes(owners[4..].try_into().unwrap());
                let owner = if expeditionary as i32 > 0 {
                    expeditionary
                } else {
                    own
                };
                let key = (0, owner as u64);
                if let Entry::Vacant(entry) = sources.entry(key) {
                    let country = countries.resolve(owner)?;
                    let address = country
                        .pointer
                        .checked_add(engine::ccountry::MODIFIER_ACCUMULATOR)
                        .ok_or("reserve modifier breakdown country address overflow")?;
                    entry.insert(address);
                }
                let kind = row.context.leader_modifier_kind;
                let key = (kind, row.stats.commander);
                if kind != 0
                    && let Entry::Vacant(entry) = sources.entry(key)
                {
                    let accumulator =
                        engine::carmy_leader::movement_accumulator(memory, base, row.pointer)?
                            .ok_or("reserve modifier breakdown selected commander missing")?;
                    entry.insert(accumulator);
                }
            }
            engine::allocation::reserve_push_batch(&mut self.breakdowns, sources.len())
                .map_err(crate::allocation::io)
                .map_err(|_| "collector allocation failed")?;
            for &(kind, owner) in sources.keys() {
                self.breakdowns
                    .push(replay::rows::ReserveModifierBreakdownRow {
                        kind,
                        owner,
                        nodes: Vec::new(),
                    });
            }
            // Preserve the original tree read order and replay row order.
            self.breakdowns.sort_unstable_by_key(RowCodec::key);
            for row in &mut self.breakdowns {
                row.nodes = engine::modifier_breakdown::tree(
                    memory,
                    base,
                    sources[&(row.kind, row.owner)],
                    engine::modifier_keys::LAND_REINFORCE_RATE,
                )?;
            }
            Ok(())
        })();
        self.breakdown_outcome = match result {
            Ok(()) => Outcome::Rows {
                len: self.breakdowns.len(),
            },
            Err(reason) => Outcome::Unavailable { reason },
        };
    }
    fn tactic_inputs(context: &land_tactic_inputs::ContextRow) -> LandCombatTacticInputs {
        LandCombatTacticInputs {
            preferred_tactic_ids: context.preferred_tactic_ids,
            country_weights: context
                .country_weights
                .map(|w| LandCombatTacticCountryWeight {
                    country: w.country,
                    tactic_id: w.tactic_id,
                    modifier_key: w.modifier_key,
                    metadata_flags: w.metadata_flags,
                    metadata_gate: w.metadata_gate,
                    country_factor_raw: w.country_factor_raw,
                    accumulator_raw: w.accumulator_raw,
                    accumulator_entry_present: w.accumulator_entry_present,
                }),
            country_weight_present: context.country_weight_present,
        }
    }
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            table::<tables::LandCombatDetails, _>(outcomes[0], &self.details.rows, |captured| {
                let r = &captured.row;
                LandCombatDetailsRow {
                    tactic_inputs: Self::tactic_inputs(&captured.tactic_context),
                    combat: r.id,
                    province: r.province,
                    phase: r.phase,
                    width_factor: r.width_factor,
                    attacker_bonus: r.attacker_bonus,
                    defender_bonus: r.defender_bonus,
                    stored_terrain_name: allocation::owned_vec(allocation::copy_vec(
                        &self.detail_text.first[r.stored_terrain_name.range()],
                    )),
                    stored_terrain_valid: r.stored_terrain_valid,
                    sides: std::array::from_fn(|side| {
                        let s = &r.sides[side];
                        LandCombatSideDetails {
                            presentation: commander_presentation(&s.presentation),
                            committed: allocation::owned_vec(allocation::copy_vec(
                                &self.rosters.first
                                    [s.committed_start..s.committed_start + s.committed_len],
                            )),
                            reserves: allocation::owned_vec(allocation::copy_vec(
                                &self.rosters.first
                                    [s.reserve_start..s.reserve_start + s.reserve_len],
                            )),
                            tactic_id: s.tactic_id,
                            tactic_kind: s.tactic_kind,
                            tactic_counter_id: s.tactic_counter_id,
                            tactic_token: allocation::owned_vec(allocation::copy_vec(
                                &self.detail_text.first[s.tactic_token.range()],
                            )),
                            air_counts: allocation::owned_vec(allocation::copy_vec(
                                &self.air_counts.first[s.air_counts.range()],
                            )),
                            anti_air_attack: s.anti_air_attack,
                            air_kills: s.air_kills,
                            damage: s.damage,
                            organisation_loss: s.organisation_loss,
                            organisation_loss_count: s.organisation_loss_count,
                            commander: s.commander,
                            commander_skill: s.commander_skill,
                            commander_category: s.commander_category,
                            commander_country: s.commander_country,
                            commander_experience: s.commander_experience,
                            commander_name_token: allocation::owned_vec(allocation::copy_vec(
                                &self.detail_text.first[s.commander_name_token.range()],
                            )),
                            commander_description_token: allocation::owned_vec(
                                allocation::copy_vec(
                                    &self.detail_text.first[s.commander_description_token.range()],
                                ),
                            ),
                            commander_formation: s.commander_formation,
                            timed_traits: allocation::owned_vec(allocation::collect_vec(
                                self.timed_traits.first[s.timed_traits_start
                                    ..s.timed_traits_start + s.timed_traits_len]
                                    .iter()
                                    .map(|t| LandCombatCommanderTimedTrait {
                                        token_id: t.token_id,
                                        definition_ordinal: t.definition_ordinal,
                                        remaining_days: t.remaining_days,
                                    }),
                            )),
                            learning_traits: allocation::owned_vec(allocation::collect_vec(
                                self.learning_traits.first[s.learning_traits.range()]
                                    .iter()
                                    .map(|t| LandCombatCommanderLearningTrait {
                                        token_id: t.token_id,
                                        definition_ordinal: t.definition_ordinal,
                                        required_experience: t.required_experience,
                                        experience: t.experience,
                                        token: allocation::owned_vec(allocation::copy_vec(
                                            &self.detail_text.first
                                                [t.token_start..t.token_start + t.token_len],
                                        )),
                                    }),
                            )),
                            traits: allocation::owned_vec(allocation::collect_vec(
                                self.commander_traits.first
                                    [s.traits_start..s.traits_start + s.traits_len]
                                    .iter()
                                    .map(|t| LandCombatCommanderTrait {
                                        token_id: t.token_id,
                                        definition_ordinal: t.definition_ordinal,
                                        kind: t.kind,
                                        relevant: t.relevant,
                                        token: allocation::owned_vec(allocation::copy_vec(
                                            &self.detail_text.first
                                                [t.token_start..t.token_start + t.token_len],
                                        )),
                                    }),
                            )),
                        }
                    }),
                }
            }),
            table::<tables::LandCombatDivisions, _>(outcomes[1], &self.divisions.rows, |r| {
                let s = r.stats;
                // Admitted rows always carry their naming spans.
                let naming = s.naming.unwrap_or_default();
                let name = |(start, len): (usize, usize)| {
                    allocation::owned_vec(allocation::copy_vec(
                        &self.division_names.first[start..start + len],
                    ))
                };
                LandCombatDivisionRow {
                    combat: r.combat,
                    side: r.side,
                    ordinal: r.ordinal,
                    division: s.division,
                    name_type: naming.kind,
                    name_order: naming.order,
                    name_ordered: naming.ordered,
                    name_programmatic: naming.programmatic,
                    name_custom: name(naming.custom),
                    name_group: name(naming.group),
                    role: r.role,
                    province: r.province,
                    strength_combat_factor: s.strength_combat_factor,
                    stat_values: s.stat_values,
                    armour: s.armour,
                    piercing: s.piercing,
                    air_attack: s.air_attack,
                    reconnaissance: s.reconnaissance,
                    initiative: s.initiative,
                    commander: s.commander,
                    direction_group: s.direction_group,
                    categories: s.categories,
                    exile: r.exile.map(|exile| {
                        let tags = |(start, len): (usize, usize)| {
                            allocation::owned_vec(allocation::copy_vec(
                                &self.exile_tags.first[start..start + len],
                            ))
                        };
                        LandCombatExileInputs {
                            origin_type: exile.origin_type,
                            country: exile.country,
                            country_fallback: exile.country_fallback,
                            location_cores: tags(exile.cores),
                            occupiers: tags(exile.occupiers),
                        }
                    }),
                    commander_inputs: r.commander_input.map(|index| {
                        let c = &self.division_commanders.first[index];
                        replay::rows::LandCombatDivisionCommander {
                            skill: c.skill,
                            name_token: allocation::owned_vec(allocation::copy_vec(
                                &self.commander_names.first
                                    [c.name_token.0..c.name_token.0 + c.name_token.1],
                            )),
                            traits: allocation::owned_vec(allocation::collect_vec(
                                c.traits[..c.traits_len]
                                    .iter()
                                    .map(|t| LandCombatCommanderTrait {
                                        token_id: t.token_id,
                                        definition_ordinal: t.definition_ordinal,
                                        kind: t.kind,
                                        relevant: t.relevant,
                                        token: allocation::owned_vec(allocation::copy_vec(
                                            &c.trait_text
                                                [t.token_start..t.token_start + t.token_len],
                                        )),
                                    }),
                            )),
                        }
                    }),
                    scales: s.scales,
                    fuel_inputs: s.fuel_inputs,
                    reinforcement: LandReinforcementInputs {
                        scalar_values: r.reinforcement.scalar_values,
                        flags: r.reinforcement.flags,
                        ids: r.reinforcement.ids,
                    },
                    reinforcement_modifiers: allocation::owned_vec(allocation::collect_vec(
                        self.reinforcement_modifiers.first[r.reinforcement_starts[0]
                            ..r.reinforcement_starts[0] + r.reinforcement.modifier_count]
                            .iter()
                            .map(|m| LandReinforcementModifier {
                                scope: m.scope,
                                key: m.key,
                                raw: m.raw,
                                present: m.present,
                                metadata_flags: m.metadata_flags,
                                feature_selector: m.feature_selector,
                            }),
                    )),
                    reinforcement_supply: allocation::owned_vec(allocation::collect_vec(
                        self.supply_terms.first[r.reinforcement_starts[1]
                            ..r.reinforcement_starts[1] + r.reinforcement.supply_term_count]
                            .iter()
                            .map(|s| LandReinforcementSupplyTerm {
                                subunit: s.subunit,
                                count: s.count,
                                special: s.special,
                            }),
                    )),
                    reinforcement_acclimatization: allocation::owned_vec(allocation::collect_vec(
                        self.acclimatization.first[r.reinforcement_starts[2]
                            ..r.reinforcement_starts[2] + r.reinforcement.acclimatization_count]
                            .iter()
                            .map(|a| LandReinforcementAcclimatization {
                                weight: a.weight,
                                active: a.active,
                            }),
                    )),
                    reinforcement_transport: allocation::owned_vec(allocation::collect_vec(
                        self.transport_terms.first[r.reinforcement_starts[3]
                            ..r.reinforcement_starts[3] + r.reinforcement.transport_term_count]
                            .iter()
                            .map(|t| LandReinforcementTransportTerm {
                                country: t.country,
                                regional_value: t.regional_value,
                                own_intel: t.own_intel,
                                enemy_intel: t.enemy_intel,
                            }),
                    )),
                    context: LandDivisionContext {
                        template: r.context.template,
                        country_modifiers: r.context.country_modifiers.map(|m| {
                            LandDivisionModifier {
                                raw: m.raw,
                                present: m.present,
                            }
                        }),
                        leader_modifiers: r.context.leader_modifiers.map(|m| {
                            LandDivisionModifier {
                                raw: m.raw,
                                present: m.present,
                            }
                        }),
                        modifier_metadata: r.context.modifier_metadata,
                        leader_modifier_kind: r.context.leader_modifier_kind,
                        feature_flags: r.context.feature_flags,
                        eligibility_callback: r.context.eligibility_callback,
                        base_speed: r.context.base_speed,
                    },
                    width_terms: allocation::owned_vec(allocation::collect_vec(
                        self.width_terms.first
                            [r.width_start..r.width_start + r.context.width_term_count]
                            .iter()
                            .map(|t| LandDivisionWidthTerm {
                                subunit: t.subunit,
                                count: t.count,
                                width: t.width,
                            }),
                    )),
                }
            }),
            table::<tables::LandCombatModifiers, _>(outcomes[2], &self.sides.rows, |r| {
                LandCombatModifierRow {
                    combat: r.combat,
                    side: r.side,
                    border_values: r.border_values,
                    divisions: allocation::owned_vec(allocation::collect_vec(
                        self.members.first[r.start..r.start + r.len]
                            .iter()
                            .map(|m| DivisionCombatModifiers {
                                division: m.division,
                                ordinal: m.ordinal,
                                entries: allocation::owned_vec(allocation::collect_vec(
                                    self.modifiers.first[m.start..m.start + m.len].iter().map(
                                        |v| CombatModifierValue {
                                            id: v.id,
                                            value_a: v.value_a,
                                            value_b: v.value_b,
                                        },
                                    ),
                                )),
                            }),
                    )),
                }
            }),
            table::<tables::LandCombatSides, _>(outcomes[3], &self.combat_sides.rows, |r| {
                let mut start = r.sets.start;
                let mut set = |index: usize| {
                    let tags = allocation::owned_vec(allocation::copy_vec(
                        &self.country_tags.first[start..start + r.sets.lens[index]],
                    ));
                    start += r.sets.lens[index];
                    tags
                };
                LandCombatSideRow {
                    combat: r.sets.combat,
                    indicator: r.sets.indicator,
                    flags: r.sets.flags,
                    attacker_primary: set(0),
                    attacker_secondary: set(1),
                    defender_primary: set(2),
                    defender_secondary: set(3),
                }
            }),
            {
                let failed = std::cell::Cell::new(false);
                let view = table::<tables::ReserveModifierBreakdowns, _>(
                    outcomes[4],
                    &self.breakdowns,
                    |row| match row.try_clone() {
                        Ok(row) => row,
                        Err(error) => {
                            crate::allocation::observe(&error);
                            failed.set(true);
                            Default::default()
                        }
                    },
                );
                if failed.get() {
                    unavailable("reserve modifier breakdown projection failed")
                } else {
                    view
                }
            },
        ]
    }
}
/// Spool-thread portrait projection shared by hourly land and naval contexts.
pub(super) fn commander_presentation(
    p: &engine::naval_commander_inputs::Presentation,
) -> replay::rows::CombatCommanderPresentation {
    use replay::rows::{CombatCommanderPortraitEntry, CombatCommanderPresentation};
    CombatCommanderPresentation {
        character: p.character,
        character_state: p.character_state,
        character_country: p.character_country,
        character_gender: p.character_gender,
        character_role_leader: p.character_role_leader,
        character_fallback_leader: p.character_fallback_leader,
        character_role_source: p.character_role_source,
        fallback_text: std::array::from_fn(|index| {
            allocation::owned_vec(allocation::copy_vec(
                &p.fallback_text[index][..p.fallback_text_len[index]],
            ))
        }),
        portraits: allocation::owned_vec(allocation::collect_vec(
            p.portraits[..p.portraits_len]
                .iter()
                .map(|v| CombatCommanderPortraitEntry {
                    role: v.role,
                    size: v.size,
                    is_set: v.is_set,
                    kind: v.kind,
                    text: allocation::owned_vec(allocation::copy_vec(
                        &p.portrait_text[v.text_start..v.text_start + v.text_len],
                    )),
                }),
        )),
        role_traits: allocation::owned_vec(allocation::collect_vec(
            p.role_traits[..p.role_traits_len]
                .iter()
                .map(|v| LandCombatCommanderTrait {
                    token_id: v.token_id,
                    definition_ordinal: v.definition_ordinal,
                    kind: v.kind,
                    relevant: v.relevant,
                    token: allocation::owned_vec(allocation::copy_vec(
                        &p.role_trait_text[v.token_start..v.token_start + v.token_len],
                    )),
                }),
        )),
    }
}
#[cfg(test)]
mod reference_walk;

fn elapsed(start: i64) -> u64 {
    qpc().saturating_sub(start).max(0) as u64
}

#[cfg(test)]
mod tests;

/// The whole land combat family over a synthetic combat world, walked as the
/// recorder walks it and as the reference walk (main's reads, nothing
/// shared): the census, window operands, details, tactic registry and
/// context, modifier sides and every division reader run on total memory
/// (any address from 0x1_0000 reads, as zero unless written) with the module
/// at base 0.
#[cfg(test)]
mod family;

#[cfg(test)]
pub(crate) fn allocation_failure_exercise(arm: impl FnOnce(), disarm: impl FnOnce()) {
    family::allocation_failure_exercise(arm, disarm);
}
