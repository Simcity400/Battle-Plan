//! Hourly combat-window and marker operands. One bounded census feeds four
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
    spooling::table,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::land_tactic_inputs;
use engine::{
    Memory, carmy_combat_inputs, cland_combat,
    combat_sources::{Images, Sources},
    land_reinforcement_inputs as reinforcement,
    walk::Outcome,
};
use replay::rows::{
    LandCombatTacticCandidate, LandCombatTacticCountryWeight, LandCombatTacticInputs,
    LandCombatTacticUnlock, LandCombatTacticWeightTerm,
};
use replay::{
    frame::TableView,
    rows::{
        CombatModifierValue, DivisionCombatModifiers, LandCombatCommanderLearningTrait,
        LandCombatCommanderTrait, LandCombatDetailsRow, LandCombatDivisionRow, LandCombatExileInputs,
        LandCombatModifierRow, LandCombatSideDetails, LandCombatSideRow, LandDivisionContext, LandDivisionModifier,
        LandDivisionWidthTerm, LandReinforcementAcclimatization, LandReinforcementInputs,
        LandReinforcementModifier, LandReinforcementSupplyTerm, LandReinforcementTransportTerm,
        TableId, tables,
    },
};
use std::mem::size_of;

pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::LandCombatDetails,
    TableId::LandCombatDivisions,
    TableId::LandCombatModifiers,
    TableId::LandCombatSides,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "land combat detail census and member authentication",
    0,
    &[0, 1, 2, 3],
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
    tactic_candidates: usize,
    tactic_weights: usize,
    tactic_unlocks: usize,
    division_commanders: usize,
    /// Commander text bytes: division commanders' name tokens and each kept
    /// combat side's selected commander name and description tokens.
    commander_names: usize,
    commander_keys: usize,
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
            tactic_candidates: 256,
            tactic_weights: 1024,
            tactic_unlocks: 4096,
            division_commanders: 32,
            commander_names: 2048,
            commander_keys: 2048,
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
    /// Both committed/reserve lists on each side have independent bounds.
    rosters: Vec<u64>,
    modifiers: Vec<carmy_combat_inputs::Modifier>,
}
impl Scratch {
    /// Sizes each buffer at its reader's bound; a no-op once sized.
    fn ensure(&mut self) {
        grow_vec(&mut self.widths, carmy_combat_inputs::WIDTH_TERM_LIMIT);
        grow_vec(
            &mut self.reinforcement_modifiers,
            reinforcement::MODIFIER_LIMIT,
        );
        grow_vec(&mut self.supply, reinforcement::SUPPLY_TERM_LIMIT);
        grow_vec(
            &mut self.acclimatization,
            reinforcement::ACCLIMATIZATION_LIMIT,
        );
        grow_vec(
            &mut self.rosters,
            cland_combat::DETAIL_UNIT_LIMIT as usize * 4,
        );
        grow_vec(&mut self.modifiers, carmy_combat_inputs::MODIFIER_LIMIT);
    }
    fn allocated_bytes(&self) -> usize {
        self.widths.capacity() * size_of::<carmy_combat_inputs::WidthTerm>()
            + self.reinforcement_modifiers.capacity() * size_of::<reinforcement::ModifierTerm>()
            + self.supply.capacity() * size_of::<reinforcement::SupplyTerm>()
            + self.acclimatization.capacity() * size_of::<reinforcement::AcclimatizationTerm>()
            + self.rosters.capacity() * size_of::<u64>()
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
    /// Per-pass shares of the combat readers, and their storage.
    combat: &'a mut cland_combat::CombatScratch,
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
    /// Kept commanders' texts: division commanders' name tokens and detail
    /// sides' selected commander tokens. A token that does not fit is counted
    /// for growth and its divisions, or its combat's detail, are omitted this
    /// hour.
    commander_names: engine::cname_group_member::Names<'a>,
    commander_progress: Progress,
    commander_keys: &'a mut [CommanderKey],
    commander_key_count: usize,
    required_commander_keys: usize,
    details: &'a mut [Detail],
    tactic_candidates: &'a mut [land_tactic_inputs::Candidate],
    tactic_weights: &'a mut [land_tactic_inputs::WeightTerm],
    tactic_unlocks: &'a mut [land_tactic_inputs::Unlock],
    tactic_registry: Option<land_tactic_inputs::RegistryCapture>,
    tactic_unlock_progress: Progress,
    divisions: &'a mut Vec<Division>,
    sides: &'a mut [ModifierSide],
    combat_sides: &'a mut [CombatSides],
    country_tags: &'a mut [u32],
    country_tags_progress: Progress,
    /// Nodes one country set can hold: the live country count plus tag 0.
    country_set_limit: Result<usize, &'static str>,
    rosters: &'a mut [u64],
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
                    grow_vec(self.division_commanders, index + 1);
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
    fn admit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        at: Placement,
        acquired: Acquired,
    ) {
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
        grow_vec(self.width_terms, start + count);
        grow_vec(self.reinforcement_modifiers, starts[0] + counts[0]);
        grow_vec(self.supply_terms, starts[1] + counts[1]);
        grow_vec(self.acclimatization, starts[2] + counts[2]);
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
        // Selections are the combat's own.
        self.commanders = cland_combat::Commanders::default();
        // The census has already retained every participant in this combat;
        // its counts bound the division destination without another read.
        let bound =
            (source.attackers.count as usize).saturating_add(source.defenders.count as usize);
        grow_vec(self.divisions, self.progress[1].required + bound);
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
            images.capture(memory, members.pointers);
            self.ticks[1] += elapsed(started);
        } else {
            images.clear();
        }
        // The unit visit's storage, lent while the visitor borrows `self`.
        let mut shares = std::mem::take(self.combat);
        {
            let _reads = crate::read_accounting::table(TABLES[3]);
            let started = qpc();
            self.combat_sides(memory, base, source);
            self.ticks[3] += elapsed(started);
        }
        {
            let started = qpc();
            let before = self.ticks[1] + self.ticks[2];
            let result = cland_combat::visit_units(
                memory,
                base,
                source,
                &members,
                &mut shares,
                |side, ordinal, pointer, handle, province, role| {
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
            self.ticks[0] +=
                elapsed(started).saturating_sub(self.ticks[1] + self.ticks[2] - before);
            if let Err(reason) = result {
                for index in [1, 2] {
                    {
                        self.progress[index].failure = Some(reason);
                    }
                }
            }
        }
        *self.combat = shares;
        *self.images = images;
    }
    /// The country sets with the marker's indicator and flag words, which
    /// `read_country_sets` reads itself.
    fn combat_sides(
        &mut self,
        memory: &impl Memory,
        base: usize,
        source: cland_combat::Source,
    ) {
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
        if self.tactic_registry.is_none() {
            self.tactic_registry = Some(land_tactic_inputs::capture_registry(
                memory,
                base,
                self.tactic_candidates,
                self.tactic_weights,
            ));
        }
        match self.tactic_registry.expect("registry supplied").outcome {
            Outcome::Unavailable { reason } => {
                self.progress[0].failure = Some(reason);
                return;
            }
            Outcome::Truncated { .. } => return,
            Outcome::Rows { .. } => {}
        }
        // Selected commanders' texts share the commander text arena; they stay
        // committed only if this combat's row is kept.
        let texts = self.commander_names.len;
        match cland_combat::read_details(
            memory,
            base,
            source,
            members,
            &mut self.commanders,
            &mut self.scratch.rosters,
            &mut self.commander_names,
        ) {
            Ok(cland_combat::Details {
                mut row,
                participants: count,
                texts_fit,
            }) => {
                self.arenas[0].required += count;
                let mut context = land_tactic_inputs::capture_context(
                    memory,
                    base,
                    state,
                    source,
                    members,
                    &mut self.commanders,
                    &mut self.tactic_unlocks[self.tactic_unlock_progress.len..],
                );
                self.tactic_unlock_progress.required += context.row.required_unlocks;
                match context.outcome {
                    Outcome::Unavailable { reason } => {
                        self.progress[0].failure = Some(reason);
                        self.commander_names.len = texts;
                        return;
                    }
                    Outcome::Truncated { .. } => {
                        self.commander_names.len = texts;
                        return;
                    }
                    Outcome::Rows { .. } => {}
                }
                // A text that did not fit was counted for growth: the whole
                // combat is omitted this hour, as for a full roster arena.
                if !texts_fit
                    || self.progress[0].len == self.details.len()
                    || count > self.rosters.len().saturating_sub(self.arenas[0].len)
                {
                    self.commander_names.len = texts;
                    return;
                }
                let start = self.arenas[0].len;
                self.rosters[start..start + count].copy_from_slice(&self.scratch.rosters[..count]);
                for side in &mut row.sides {
                    side.committed_start += start;
                    side.reserve_start += start;
                }
                let unlock_count = context.row.unlock_count.iter().sum::<usize>();
                for start in &mut context.row.unlock_start {
                    *start += self.tactic_unlock_progress.len;
                }
                self.details[self.progress[0].len] = Detail {
                    row,
                    tactic_context: context.row,
                };
                self.tactic_unlock_progress.len += unlock_count;
                self.progress[0].len += 1;
                self.arenas[0].len += count;
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
    division_commanders: Arena<engine::naval_commander_inputs::DivisionCommanderRow>,
    commander_names: Arena<u8>,
    commander_keys: Vec<CommanderKey>,
    required_commander_keys: usize,
    details: CaptureRows<Detail>,
    tactic_candidates: Arena<land_tactic_inputs::Candidate>,
    tactic_weights: Arena<land_tactic_inputs::WeightTerm>,
    tactic_unlocks: Arena<land_tactic_inputs::Unlock>,
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
    census: Vec<cland_combat::Row>,
    census_participants: Vec<u64>,
    /// Each census participant's pointer, beside its handle.
    census_pointers: Vec<usize>,
    required_census: usize,
    required_census_participants: usize,
    /// Per-pass shared division sources; cleared before each pass.
    sources: Sources,
    /// Per-pass shares of the combat readers; cleared before each pass.
    combat: cland_combat::CombatScratch,
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
            tactic_candidates: 0,
            tactic_weights: 0,
            tactic_unlocks: 0,
            division_commanders: 0,
            commander_names: 0,
            commander_keys: 0,
        })
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            division_commanders: Arena::new(c.division_commanders),
            commander_names: Arena::new(c.commander_names),
            commander_keys: vec![
                CommanderKey::default();
                c.commander_keys.max(c.census_participants)
            ],
            required_commander_keys: 0,
            details: CaptureRows::new(c.details),
            tactic_candidates: Arena::new(c.tactic_candidates),
            tactic_weights: Arena::new(c.tactic_weights),
            tactic_unlocks: Arena::new(c.tactic_unlocks),
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
            name_cache: engine::cname_group_member::GroupKeys::with_capacity(
                c.name_templates,
                c.name_groups,
            ),
            reinforcement_modifiers: Arena::new(c.reinforcement_modifiers),
            supply_terms: Arena::new(c.supply_terms),
            acclimatization: Arena::new(c.acclimatization),
            transport_terms: Arena::new(c.transport_terms),
            census: vec![cland_combat::Row::default(); c.combats],
            census_participants: vec![0; c.census_participants],
            census_pointers: vec![0; c.census_participants],
            required_census: 0,
            required_census_participants: 0,
            sources: Sources::new(),
            combat: cland_combat::CombatScratch::default(),
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
        self.division_commanders.reset();
        self.commander_names.reset();
        self.required_commander_keys = 0;
        self.details.reset();
        self.tactic_candidates.reset();
        self.tactic_weights.reset();
        self.tactic_unlocks.reset();
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
    }
    pub fn allocated_bytes(&self) -> usize {
        self.details.allocated_bytes()
            + self.division_commanders.allocated_bytes()
            + self.commander_names.allocated_bytes()
            + self.commander_keys.capacity() * size_of::<CommanderKey>()
            + self.tactic_candidates.allocated_bytes()
            + self.tactic_weights.allocated_bytes()
            + self.tactic_unlocks.allocated_bytes()
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
        c.commander_names = c
            .commander_names
            .max(self.commander_names.first.capacity());
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
        self.tactic_candidates.grow(&mut c.tactic_candidates);
        self.tactic_weights.grow(&mut c.tactic_weights);
        self.tactic_unlocks.grow(&mut c.tactic_unlocks);
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
            self.name_cache = engine::cname_group_member::GroupKeys::with_capacity(
                c.name_templates,
                c.name_groups,
            );
        }
        self.reinforcement_modifiers
            .grow(&mut c.reinforcement_modifiers);
        self.supply_terms.grow(&mut c.supply_terms);
        self.acclimatization.grow(&mut c.acclimatization);
        self.transport_terms.grow(&mut c.transport_terms);
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
        self.scratch.ensure();
        let _reads = crate::read_accounting::table(TABLES[0]);
        // Naming memos never outlive the pass that authenticated them.
        self.name_cache.reset();
        let started = qpc();
        let commander_started = qpc();
        // walk_with visits a combat only after every participant fits in this
        // owned census. Each visited participant has at most one commander, so
        // this bounds all distinct nonzero keys before the first insertion.
        grow_vec(&mut self.commander_keys, self.census_participants.len());
        self.commander_keys.fill(CommanderKey::default());
        let commander_ticks = elapsed(commander_started);
        let mut destination = Destination {
            sources: &mut self.sources,
            combat: &mut self.combat,
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
            tactic_candidates: &mut self.tactic_candidates.first,
            tactic_weights: &mut self.tactic_weights.first,
            tactic_unlocks: &mut self.tactic_unlocks.first,
            tactic_registry: None,
            tactic_unlock_progress: Progress::default(),
            details: &mut self.details.rows,
            divisions: &mut self.divisions.rows,
            sides: &mut self.sides.rows,
            combat_sides: &mut self.combat_sides.rows,
            country_tags: &mut self.country_tags.first,
            country_tags_progress: Progress::default(),
            country_set_limit: engine::ccountry::Reader::new(memory, base, state)
                .map(|countries| countries.count() as usize + 1),
            rosters: &mut self.rosters.first,
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
            ticks: [0, commander_ticks, 0, 0],
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
        let tactic_registry = destination
            .tactic_registry
            .map(|r| r.row)
            .unwrap_or_default();
        let tactic_unlocks = destination.tactic_unlock_progress;
        let width = destination.width_progress;
        let exile = destination.exile_progress;
        let names = (destination.names.len, destination.names.required);
        let commander_names = (
            destination.commander_names.len,
            destination.commander_names.required,
        );
        let country_tags = destination.country_tags_progress;
        let reinforcement = destination.reinforcement_progress;
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
        self.tactic_candidates.len = tactic_registry.candidate_count;
        self.tactic_candidates.required = tactic_registry.required_candidates;
        self.tactic_weights.len = tactic_registry.weight_count;
        self.tactic_weights.required = tactic_registry.required_weights;
        self.tactic_unlocks.len = tactic_unlocks.len;
        self.tactic_unlocks.required = tactic_unlocks.required;
        self.rosters.len = arenas[0].len;
        self.rosters.required = arenas[0].required;
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
        ticks[0] += elapsed(started).saturating_sub(ticks.iter().sum::<u64>());
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.details.outcome,
            self.divisions.outcome,
            self.sides.outcome,
            self.combat_sides.outcome,
        ]
    }
    fn tactic_inputs(&self, context: &land_tactic_inputs::ContextRow) -> LandCombatTacticInputs {
        LandCombatTacticInputs {
            countries: context.countries,
            preferred_tactic_ids: context.preferred_tactic_ids,
            commander_handles: context.commander_handles,
            preference_factors: context.preference_factors,
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
            unlocks: std::array::from_fn(|index| {
                self.tactic_unlocks.first[context.unlock_start[index]
                    ..context.unlock_start[index] + context.unlock_count[index]]
                    .iter()
                    .map(|u| LandCombatTacticUnlock {
                        country: u.country,
                        ordinal: u.ordinal,
                        unlock_id: u.unlock_id,
                    })
                    .collect()
            }),
        }
    }
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        let commander_text = |(start, len): engine::cname_group_member::Span| {
            self.commander_names.first[start..start + len].to_vec()
        };
        [
            table::<tables::LandCombatDetails, _>(outcomes[0], &self.details.rows, |captured| {
                let r = &captured.row;
                LandCombatDetailsRow {
                    tactic_inputs: self.tactic_inputs(&captured.tactic_context),
                    tactic_candidates: self.tactic_candidates.first[..self.tactic_candidates.len]
                        .iter()
                        .map(|t| LandCombatTacticCandidate {
                            ordinal: t.ordinal,
                            tactic_id: t.tactic_id,
                            unlock_id: t.unlock_id,
                            only_show_for: t.only_show_for,
                            active: t.active,
                            base_weight: t.base_weight,
                            token: t.token.as_bytes().to_vec(),
                            weights: self.tactic_weights.first
                                [t.weight_start..t.weight_start + t.weight_count]
                                .iter()
                                .map(|w| LandCombatTacticWeightTerm {
                                    tactic_id: w.tactic_id,
                                    ordinal: w.ordinal,
                                    factor: w.factor,
                                    add: w.add,
                                })
                                .collect(),
                        })
                        .collect(),
                    combat: r.id,
                    province: r.province,
                    phase: r.phase,
                    width_factor: r.width_factor,
                    attacker_bonus: r.attacker_bonus,
                    defender_bonus: r.defender_bonus,
                    stored_terrain_name: r.stored_terrain_name[..r.stored_terrain_name_len]
                        .to_vec(),
                    stored_terrain_valid: r.stored_terrain_valid,
                    sides: std::array::from_fn(|side| {
                        let s = &r.sides[side];
                        LandCombatSideDetails {
                            presentation: commander_presentation(&s.presentation),
                            committed: self.rosters.first
                                [s.committed_start..s.committed_start + s.committed_len]
                                .to_vec(),
                            reserves: self.rosters.first
                                [s.reserve_start..s.reserve_start + s.reserve_len]
                                .to_vec(),
                            tactic_id: s.tactic_id,
                            tactic_kind: s.tactic_kind,
                            tactic_counter_id: s.tactic_counter_id,
                            tactic_token: s.tactic_token[..s.tactic_token_len].to_vec(),
                            air_counts: s.air_counts[..s.air_count_len].to_vec(),
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
                            commander_name_token: commander_text(s.commander_name_token),
                            commander_description_token: commander_text(
                                s.commander_description_token,
                            ),
                            commander_formation: s.commander_formation,
                            learning_traits: s.learning_traits[..s.learning_traits_len]
                                .iter()
                                .map(|t| LandCombatCommanderLearningTrait {
                                    token_id: t.token_id,
                                    definition_ordinal: t.definition_ordinal,
                                    required_experience: t.required_experience,
                                    experience: t.experience,
                                    token: s.learning_text
                                        [t.token_start..t.token_start + t.token_len]
                                        .to_vec(),
                                })
                                .collect(),
                            traits: s.traits[..s.traits_len]
                                .iter()
                                .map(|t| LandCombatCommanderTrait {
                                    token_id: t.token_id,
                                    definition_ordinal: t.definition_ordinal,
                                    kind: t.kind,
                                    relevant: t.relevant,
                                    token: s.trait_text[t.token_start..t.token_start + t.token_len]
                                        .to_vec(),
                                })
                                .collect(),
                        }
                    }),
                }
            }),
            table::<tables::LandCombatDivisions, _>(outcomes[1], &self.divisions.rows, |r| {
                let s = r.stats;
                // Admitted rows always carry their naming spans.
                let naming = s.naming.unwrap_or_default();
                let name = |(start, len): (usize, usize)| {
                    self.division_names.first[start..start + len].to_vec()
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
                            self.exile_tags.first[start..start + len].to_vec()
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
                            name_token: self.commander_names.first
                                [c.name_token.0..c.name_token.0 + c.name_token.1]
                                .to_vec(),
                            traits: c.traits[..c.traits_len]
                                .iter()
                                .map(|t| LandCombatCommanderTrait {
                                    token_id: t.token_id,
                                    definition_ordinal: t.definition_ordinal,
                                    kind: t.kind,
                                    relevant: t.relevant,
                                    token: c.trait_text[t.token_start..t.token_start + t.token_len]
                                        .to_vec(),
                                })
                                .collect(),
                        }
                    }),
                    scales: s.scales,
                    fuel_inputs: s.fuel_inputs,
                    reinforcement: LandReinforcementInputs {
                        scalar_values: r.reinforcement.scalar_values,
                        flags: r.reinforcement.flags,
                        ids: r.reinforcement.ids,
                    },
                    reinforcement_modifiers: self.reinforcement_modifiers.first[r
                        .reinforcement_starts[0]
                        ..r.reinforcement_starts[0] + r.reinforcement.modifier_count]
                        .iter()
                        .map(|m| LandReinforcementModifier {
                            scope: m.scope,
                            key: m.key,
                            raw: m.raw,
                            present: m.present,
                            metadata_flags: m.metadata_flags,
                            feature_selector: m.feature_selector,
                        })
                        .collect(),
                    reinforcement_supply: self.supply_terms.first[r.reinforcement_starts[1]
                        ..r.reinforcement_starts[1] + r.reinforcement.supply_term_count]
                        .iter()
                        .map(|s| LandReinforcementSupplyTerm {
                            subunit: s.subunit,
                            count: s.count,
                            special: s.special,
                        })
                        .collect(),
                    reinforcement_acclimatization:
                        self.acclimatization.first[r.reinforcement_starts[2]
                            ..r.reinforcement_starts[2] + r.reinforcement.acclimatization_count]
                            .iter()
                            .map(|a| LandReinforcementAcclimatization {
                                weight: a.weight,
                                active: a.active,
                            })
                            .collect(),
                    reinforcement_transport: self.transport_terms.first[r.reinforcement_starts[3]
                        ..r.reinforcement_starts[3] + r.reinforcement.transport_term_count]
                        .iter()
                        .map(|t| LandReinforcementTransportTerm {
                            country: t.country,
                            regional_value: t.regional_value,
                            own_intel: t.own_intel,
                            enemy_intel: t.enemy_intel,
                        })
                        .collect(),
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
                    width_terms: self.width_terms.first
                        [r.width_start..r.width_start + r.context.width_term_count]
                        .iter()
                        .map(|t| LandDivisionWidthTerm {
                            subunit: t.subunit,
                            count: t.count,
                            width: t.width,
                        })
                        .collect(),
                }
            }),
            table::<tables::LandCombatModifiers, _>(outcomes[2], &self.sides.rows, |r| {
                LandCombatModifierRow {
                    combat: r.combat,
                    side: r.side,
                    border_values: r.border_values,
                    divisions: self.members.first[r.start..r.start + r.len]
                        .iter()
                        .map(|m| DivisionCombatModifiers {
                            division: m.division,
                            ordinal: m.ordinal,
                            entries: self.modifiers.first[m.start..m.start + m.len]
                                .iter()
                                .map(|v| CombatModifierValue {
                                    id: v.id,
                                    value_a: v.value_a,
                                    value_b: v.value_b,
                                })
                                .collect(),
                        })
                        .collect(),
                }
            }),
            table::<tables::LandCombatSides, _>(outcomes[3], &self.combat_sides.rows, |r| {
                let mut start = r.sets.start;
                let mut set = |index: usize| {
                    let tags = self.country_tags.first[start..start + r.sets.lens[index]].to_vec();
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
            p.fallback_text[index][..p.fallback_text_len[index]].to_vec()
        }),
        portraits: p.portraits[..p.portraits_len]
            .iter()
            .map(|v| CombatCommanderPortraitEntry {
                role: v.role,
                size: v.size,
                is_set: v.is_set,
                kind: v.kind,
                text: p.portrait_text[v.text_start..v.text_start + v.text_len].to_vec(),
            })
            .collect(),
        role_traits: p.role_traits[..p.role_traits_len]
            .iter()
            .map(|v| LandCombatCommanderTrait {
                token_id: v.token_id,
                definition_ordinal: v.definition_ordinal,
                kind: v.kind,
                relevant: v.relevant,
                token: p.role_trait_text[v.token_start..v.token_start + v.token_len].to_vec(),
            })
            .collect(),
    }
}
#[cfg(test)]
mod reference_walk;

fn elapsed(start: i64) -> u64 {
    qpc().saturating_sub(start).max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_capacity_and_division_growth_keep_independent_failures() {
        let mut image = vec![0_u8; 0x4000];
        for (address, value) in [
            (0x1000, engine::carmy::VTABLE_RVA as u64),
            (0x1018, 0x1234_0000_0033),
            (0x1170, 0x3000),
            (0x1178, 2_u64 << 32 | 2),
            (0x3000, 27),
            (0x3008, (-900_i64) as u64),
            (0x3010, 800),
            (0x3018, 2),
        ] {
            image[address..address + 8].copy_from_slice(&value.to_le_bytes());
        }
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &image,
        };
        let mut sides = [
            ModifierSide {
                combat: 62,
                ..Default::default()
            },
            ModifierSide {
                combat: 62,
                side: 1,
                ..Default::default()
            },
        ];
        let mut members = [ModifierMember::default(); 2];
        let mut values = [carmy_combat_inputs::Modifier::default(); 1];
        let mut test_sources = Sources::new();
        let mut combat = cland_combat::CombatScratch::default();
        let mut images = Images::default();
        let mut scratch = Scratch::default();
        scratch.ensure();
        let mut commander_bytes = [0_u8; 16];
        let mut grown_commander_bytes = vec![0_u8; 300];
        let mut destination = Destination {
            sources: &mut test_sources,
            combat: &mut combat,
            commanders: Default::default(),
            images: &mut images,
            scratch: &mut scratch,
            name_groups: &mut engine::cname_group_member::GroupKeys::default(),
            names: engine::cname_group_member::Names {
                bytes: &mut [],
                len: 0,
                required: 0,
            },
            reference: false,
            details: &mut [],
            division_commanders: &mut Vec::new(),
            commander_names: engine::cname_group_member::Names {
                bytes: &mut commander_bytes,
                len: 0,
                required: 0,
            },
            commander_keys: &mut [],
            commander_progress: Progress::default(),
            commander_key_count: 0,
            required_commander_keys: 0,
            tactic_candidates: &mut [],
            tactic_weights: &mut [],
            tactic_unlocks: &mut [],
            tactic_registry: None,
            tactic_unlock_progress: Progress::default(),
            divisions: &mut Vec::new(),
            sides: &mut sides,
            combat_sides: &mut [],
            country_tags: &mut [],
            country_tags_progress: Progress::default(),
            country_set_limit: Ok(1),
            rosters: &mut [],
            members: &mut members,
            modifiers: &mut values,
            width_terms: &mut Vec::new(),
            width_progress: Progress::default(),
            progress: [Progress::default(); N],
            arenas: [Progress::default(); N],
            exile_tags: &mut [],
            exile_progress: Progress::default(),
            reinforcement_modifiers: &mut Vec::new(),
            supply_terms: &mut Vec::new(),
            acclimatization: &mut Vec::new(),
            transport_terms: &mut Vec::new(),
            reinforcement_progress: [Progress::default(); 4],
            ticks: [0; N],
        };
        destination.progress[2] = Progress {
            len: 2,
            required: 2,
            failure: None,
        };
        destination.modifier_member(&memory, 0, 0x1000, 0x1234_0000_0033, 0, Some(0));
        assert_eq!(destination.arenas[1].required, 1);
        assert_eq!(destination.arenas[2].required, 2);
        assert_eq!(destination.arenas[2].len, 0);
        destination.finish();
        assert_eq!(
            destination.progress[2].outcome(Outcome::Rows { len: 1 }),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(destination.sides[0].side, 1);
        assert_eq!(destination.sides[0].len, 0);

        // The empty division destination grows from the original source
        // count; native failures still fail both consumers and do not prevent
        // the independent modifiers census below.
        destination.progress = [Progress::default(); N];
        destination.visit(
            &memory,
            0,
            0,
            cland_combat::Source {
                pointer: 0,
                id: 62,
                attackers: engine::cpdx_array::Array { data: 0, count: 1 },
                defenders: engine::cpdx_array::Array { data: 0, count: 0 },
                provinces: engine::cpdx_array::Array { data: 0, count: 0 },
            },
            cland_combat::Members::none(),
        );
        assert_eq!(destination.divisions.len(), 1);
        for progress in &destination.progress[..2] {
            assert!(matches!(
                progress.outcome(Outcome::Rows { len: 1 }),
                Outcome::Unavailable { .. }
            ));
        }
        assert_eq!(destination.progress[2].required, 2);

        // A commander that did not fit the old row arena must still be read
        // exactly once and retained; compare every guarded-read failure prefix
        // with the native reader called directly.
        use engine::{creference_registry as registry, cunit_leader as leader};
        use std::cell::{Cell, RefCell};
        struct Traced<'a> {
            bytes: &'a [u8],
            reads: RefCell<Vec<(usize, usize)>>,
            fail: Cell<Option<usize>>,
        }
        impl Memory for Traced<'_> {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, out.len()));
                if self.fail.get() == Some(address) {
                    return false;
                }
                if address == registry::HIGH_ROOT_RVA && out.len() == 8 {
                    out.copy_from_slice(&0x1000_u64.to_le_bytes());
                    return true;
                }
                engine::memory::SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, out)
            }
        }
        let handle = (0xf000_0001_u64 << 32) | leader::REFERENCE_TYPE as u64;
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let slot = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let entry = 0x1100 + slot * 24;
        let mut image = vec![0_u8; 0x5000];
        for (address, value) in [
            (0x1000 + registry::ENTRIES, 0x1100),
            (0x1000 + registry::COUNT, 7_u64 << 32 | 1),
            (entry + 8, handle),
            (entry + 16, 0x2000),
            (0x2000, engine::carmy_leader::VTABLE_RVA as u64),
            (0x2000 + leader::TYPE, handle),
            (0x2000 + leader::SKILL, 0x4000),
            (0x2000 + leader::TOKEN + 16, 2),
            (0x2000 + leader::TOKEN + 24, 15),
            (0x4000, engine::cunit_leader_skill::VTABLE_RVA as u64),
        ] {
            image[address..address + 8].copy_from_slice(&value.to_le_bytes());
        }
        image[0x1000 + registry::OVERFLOW_DISTANCE] = 5;
        image[0x1100 + 13 * 24 + 4] = 255;
        image[entry + 4] = 1;
        image[0x2000 + leader::TOKEN..0x2000 + leader::TOKEN + 2].copy_from_slice(&[b'n', 0xff]);
        let skill = 0x4000 + engine::cunit_leader_skill::LEVEL;
        image[skill..skill + 4].copy_from_slice(&i32::MIN.to_le_bytes());
        let memory = Traced {
            bytes: &image,
            reads: RefCell::default(),
            fail: Cell::new(None),
        };
        let mut direct_bytes = [0_u8; 16];
        let mut direct = |memory: &Traced<'_>| {
            engine::naval_commander_inputs::capture_division_commander(
                memory,
                0,
                handle,
                &mut engine::cname_group_member::Names {
                    bytes: &mut direct_bytes,
                    len: 0,
                    required: 0,
                },
            )
        };
        let expected = direct(&memory).unwrap().unwrap();
        let complete_reads = memory.reads.take();
        let mut keys = [CommanderKey::default(); 1];
        destination.commander_keys = &mut keys;
        destination.progress = [Progress::default(); N];
        assert_eq!(destination.commander(&memory, 0, handle), Some(0));
        assert_eq!(destination.division_commanders[0], expected);
        assert_eq!(*memory.reads.borrow(), complete_reads);
        assert_eq!(destination.commander(&memory, 0, handle), Some(0));
        assert_eq!(*memory.reads.borrow(), complete_reads);
        for (address, _) in complete_reads {
            memory.fail.set(Some(address));
            memory.reads.borrow_mut().clear();
            let expected = direct(&memory).unwrap_err();
            let expected_reads = memory.reads.take();
            destination.commander_keys.fill(CommanderKey::default());
            destination.commander_progress = Progress::default();
            destination.commander_key_count = 0;
            destination.commander_names.len = 0;
            destination.division_commanders.clear();
            destination.progress = [Progress::default(); N];
            assert_eq!(destination.commander(&memory, 0, handle), None);
            assert_eq!(destination.progress[1].failure, Some(expected));
            assert_eq!(*memory.reads.borrow(), expected_reads);
            assert!(destination.division_commanders.is_empty());
        }

        // A token longer than any fixed buffer, held on the heap: it is
        // counted for growth once (the unfit key is memoized), its divisions
        // are omitted without a failure, and after growth it is kept whole.
        let token = 0x2000 + leader::TOKEN;
        image[token..token + 8].copy_from_slice(&0x4800_u64.to_le_bytes());
        image[token + 16..token + 24].copy_from_slice(&300_u64.to_le_bytes());
        image[token + 24..token + 32].copy_from_slice(&300_u64.to_le_bytes());
        image[0x4800..0x4800 + 300].fill(b'k');
        image[0x4800 + 300] = 0;
        let memory = Traced {
            bytes: &image,
            reads: RefCell::default(),
            fail: Cell::new(None),
        };
        let reset = |destination: &mut Destination<'_>| {
            destination.commander_keys.fill(CommanderKey::default());
            destination.commander_progress = Progress::default();
            destination.commander_key_count = 0;
            destination.commander_names.len = 0;
            destination.commander_names.required = 0;
            destination.division_commanders.clear();
            destination.progress = [Progress::default(); N];
        };
        reset(&mut destination);
        for _ in 0..2 {
            assert_eq!(destination.commander(&memory, 0, handle), None);
            assert_eq!(destination.progress[1].failure, None);
            assert_eq!(destination.commander_names.required, 300);
        }
        assert!(destination.division_commanders.is_empty());
        destination.commander_names = engine::cname_group_member::Names {
            bytes: &mut grown_commander_bytes,
            len: 0,
            required: 0,
        };
        reset(&mut destination);
        assert_eq!(destination.commander(&memory, 0, handle), Some(0));
        assert_eq!(destination.division_commanders[0].name_token, (0, 300));
        assert_eq!(destination.commander_names.len, 300);
        assert!(destination.commander_names.bytes.iter().all(|&byte| byte == b'k'));
    }

    #[test]
    fn tactic_registry_and_country_unlock_spans_keep_raw_order() {
        let c = Capacities {
            combats: 1,
            census_participants: 1,
            details: 1,
            divisions: 1,
            sides: 1,
            combat_sides: 1,
            country_tags: 1,
            rosters: 1,
            modifier_members: 1,
            modifiers: 1,
            width_terms: 1,
            exile_tags: 1,
            division_names: 1,
            name_templates: 1,
            name_groups: 1,
            reinforcement_modifiers: 1,
            supply_terms: 1,
            acclimatization: 1,
            transport_terms: 1,
            tactic_candidates: 1,
            tactic_weights: 1,
            tactic_unlocks: 3,
            division_commanders: 1,
            commander_names: 1,
            commander_keys: 2,
        };
        let mut captured = Captured::with_capacities(&c);
        captured.details.outcome = Outcome::Rows { len: 1 };
        captured.details.rows[0].row.id = 0x1000_0000_003e;
        captured.details.rows[0].tactic_context.unlock_start = [0, 1, 2];
        captured.details.rows[0].tactic_context.unlock_count = [1; 3];
        captured.tactic_candidates.len = 1;
        captured.tactic_weights.len = 1;
        captured.tactic_unlocks.len = 3;
        captured.tactic_candidates.first[0].tactic_id = 17;
        captured.tactic_candidates.first[0].weight_count = 1;
        captured.tactic_weights.first[0].tactic_id = 17;
        captured.tactic_weights.first[0].factor = i64::MIN;
        for (index, unlock) in captured.tactic_unlocks.first.iter_mut().enumerate() {
            *unlock = land_tactic_inputs::Unlock {
                country: index as u32 + 10,
                ordinal: 0,
                unlock_id: index as u32 + 30,
            };
        }
        let [
            TableView::Complete(replay::rows::TableRows::LandCombatDetails(details)),
            _,
            _,
            _,
        ] = captured.tables()
        else {
            panic!("complete tactic operands were not projected")
        };
        assert_eq!(details[0].tactic_candidates[0].weights[0].factor, i64::MIN);
        for (index, unlocks) in details[0].tactic_inputs.unlocks.iter().enumerate() {
            assert_eq!(unlocks[0].country, index as u32 + 10);
            assert_eq!(unlocks[0].unlock_id, index as u32 + 30);
        }
    }

    /// The viewer builds the land window's lists straight from these spans, so
    /// each combat's committed and reserve arrays keep their native index
    /// order (1415CA230/1415CA460) after the arena offset is applied.
    #[test]
    fn roster_spans_project_committed_and_reserve_lists_in_arena_order() {
        let c = Capacities {
            combats: 2,
            census_participants: 1,
            details: 2,
            divisions: 1,
            sides: 1,
            combat_sides: 1,
            country_tags: 1,
            rosters: 6,
            modifier_members: 1,
            modifiers: 1,
            width_terms: 1,
            exile_tags: 1,
            division_names: 1,
            name_templates: 1,
            name_groups: 1,
            reinforcement_modifiers: 1,
            supply_terms: 1,
            acclimatization: 1,
            transport_terms: 1,
            tactic_candidates: 1,
            tactic_weights: 1,
            tactic_unlocks: 1,
            division_commanders: 1,
            commander_names: 1,
            commander_keys: 2,
        };
        let mut captured = Captured::with_capacities(&c);
        captured.details.outcome = Outcome::Rows { len: 2 };
        captured.rosters.len = 6;
        captured
            .rosters
            .first
            .copy_from_slice(&[13, 6, 11, 4, 1, 9]);
        for (row, id, spans) in [
            (0, 0x1000_0000_003e, [(0, 2, 2, 1), (3, 0, 3, 0)]),
            (1, 0x2000_0000_003e, [(3, 0, 3, 0), (3, 1, 4, 2)]),
        ] {
            let detail = &mut captured.details.rows[row].row;
            detail.id = id;
            for (side, (committed_start, committed_len, reserve_start, reserve_len)) in
                detail.sides.iter_mut().zip(spans)
            {
                side.committed_start = committed_start;
                side.committed_len = committed_len;
                side.reserve_start = reserve_start;
                side.reserve_len = reserve_len;
            }
        }
        let [
            TableView::Complete(replay::rows::TableRows::LandCombatDetails(details)),
            _,
            _,
            _,
        ] = captured.tables()
        else {
            panic!("complete land details were not projected")
        };
        let lists: Vec<_> = details
            .iter()
            .map(|row| {
                row.sides
                    .clone()
                    .map(|side| [side.committed, side.reserves])
            })
            .collect();
        assert_eq!(
            lists,
            [
                [[vec![13, 6], vec![11]], [vec![], vec![]]],
                [[vec![], vec![]], [vec![4], vec![1, 9]]],
            ]
        );
    }

    /// The pass's destination over a capture's own buffers, as `pass` builds it.
    fn destination(c: &mut Captured) -> Destination<'_> {
        c.scratch.ensure();
        Destination {
            sources: &mut c.sources,
            combat: &mut c.combat,
            commanders: Default::default(),
            images: &mut c.images,
            scratch: &mut c.scratch,
            reference: false,
            name_groups: &mut c.name_cache,
            names: engine::cname_group_member::Names {
                bytes: &mut c.division_names.first,
                len: 0,
                required: 0,
            },
            division_commanders: &mut c.division_commanders.first,
            commander_names: engine::cname_group_member::Names {
                bytes: &mut c.commander_names.first,
                len: 0,
                required: 0,
            },
            commander_progress: Progress::default(),
            commander_keys: &mut c.commander_keys,
            commander_key_count: 0,
            required_commander_keys: 0,
            tactic_candidates: &mut c.tactic_candidates.first,
            tactic_weights: &mut c.tactic_weights.first,
            tactic_unlocks: &mut c.tactic_unlocks.first,
            tactic_registry: None,
            tactic_unlock_progress: Progress::default(),
            details: &mut c.details.rows,
            divisions: &mut c.divisions.rows,
            sides: &mut c.sides.rows,
            combat_sides: &mut c.combat_sides.rows,
            country_tags: &mut c.country_tags.first,
            country_tags_progress: Progress::default(),
            country_set_limit: Ok(1),
            rosters: &mut c.rosters.first,
            members: &mut c.members.first,
            modifiers: &mut c.modifiers.first,
            width_terms: &mut c.width_terms.first,
            width_progress: Progress::default(),
            exile_tags: &mut c.exile_tags.first,
            exile_progress: Progress::default(),
            reinforcement_modifiers: &mut c.reinforcement_modifiers.first,
            supply_terms: &mut c.supply_terms.first,
            acclimatization: &mut c.acclimatization.first,
            transport_terms: &mut c.transport_terms.first,
            reinforcement_progress: [Progress::default(); 4],
            progress: [Progress::default(); N],
            arenas: [Progress::default(); N],
            ticks: [0; N],
        }
    }

    /// Two committed exile divisions of one combat where the second's lists
    /// overflow the arena: the first keeps its rebased spans, the second is
    /// counted out as unread, and after the spool's growth the next pass
    /// keeps both. The native reads before admission are the engine's
    /// `capture_exile` tests; here each division's reads are given.
    #[test]
    fn an_overflowing_exile_division_is_counted_out_then_kept_after_growth() {
        let mut c = Capacities {
            exile_tags: 3,
            ..Capacities::default()
        };
        let mut captured = Captured::with_capacities(&c);
        let combat = 0x1000_0000_003e;
        let empty = memory_less();
        let division = |exile| Acquired {
            stats: carmy_combat_inputs::Row {
                naming: Some(Default::default()),
                ..Default::default()
            },
            exile,
            context: Default::default(),
            reinforcement: Default::default(),
        };
        let place = |ordinal, exile_start| Placement {
            combat,
            side: 0,
            ordinal,
            province: 7,
            role: 1,
            exile_start,
        };
        let row = |cores, occupiers| {
            carmy_combat_inputs::Exile::Row(carmy_combat_inputs::ExileRow {
                origin_type: 2,
                country: 4,
                cores,
                occupiers,
                ..Default::default()
            })
        };
        let pass = |captured: &mut Captured, second: carmy_combat_inputs::Exile| {
            let mut d = destination(captured);
            grow_vec(d.divisions, 2);
            // What capture_exile leaves: the first division's two cores.
            d.exile_tags[..2].copy_from_slice(&[11, 12]);
            for (ordinal, exile) in [row((0, 2), (2, 0)), second].into_iter().enumerate() {
                d.progress[1].required += 1;
                let start = d.exile_progress.len;
                if let carmy_combat_inputs::Exile::Row(_) = exile
                    && ordinal == 1
                {
                    d.exile_tags[start..start + 3].copy_from_slice(&[21, 22, 23]);
                }
                d.admit(&empty, 0, place(ordinal as u32, start), division(exile));
            }
            (d.progress[1], d.exile_progress)
        };
        // The second division needs three more tags than the arena holds.
        let (divisions, exile) = pass(
            &mut captured,
            carmy_combat_inputs::Exile::DoesNotFit { required: 3 },
        );
        assert_eq!(
            divisions.outcome(Outcome::Rows { len: 1 }),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!((exile.len, exile.required), (2, 5));
        let first = captured.divisions.rows[0].exile.unwrap();
        assert_eq!((first.cores, first.occupiers), ((0, 2), (2, 0)));
        // The spool grows the arena from the counted demand.
        captured.exile_tags.len = exile.len;
        captured.exile_tags.required = exile.required;
        captured.grow(&mut c);
        assert!(captured.exile_tags.first.len() >= 5);
        // The next pass keeps both, the second rebased past the first.
        let (divisions, exile) = pass(&mut captured, row((0, 1), (1, 2)));
        assert_eq!(
            divisions.outcome(Outcome::Rows { len: 1 }),
            Outcome::Rows { len: 2 }
        );
        assert_eq!((exile.len, exile.required), (5, 5));
        let second = captured.divisions.rows[1].exile.unwrap();
        assert_eq!((second.cores, second.occupiers), ((2, 1), (3, 2)));
        captured.exile_tags.len = exile.len;
        captured.divisions.outcome = Outcome::Rows { len: 2 };
        let [
            _,
            TableView::Complete(replay::rows::TableRows::LandCombatDivisions(rows)),
            _,
            _,
        ] = captured.tables()
        else {
            panic!("complete land divisions were not projected")
        };
        assert_eq!(rows[0].exile.as_ref().unwrap().location_cores, [11, 12]);
        assert_eq!(rows[1].exile.as_ref().unwrap().location_cores, [21]);
        assert_eq!(rows[1].exile.as_ref().unwrap().occupiers, [22, 23]);
    }

    /// No native reads happen during admission of a division without a
    /// commander; any read would fail.
    fn memory_less() -> engine::memory::SliceMemory<'static> {
        engine::memory::SliceMemory {
            base: 0,
            bytes: &[],
        }
    }

    /// The exile arena is a fixed destination during the pass: a division whose
    /// lists do not fit is counted, and the spool grows the arena from that
    /// demand rather than the pass allocating it.
    #[test]
    fn exile_arena_grows_off_the_pass_from_its_counted_demand() {
        let mut c = Capacities::default();
        let mut captured = Captured::with_capacities(&c);
        let before = captured.exile_tags.first.len();
        captured.exile_tags.len = 3;
        captured.exile_tags.required = before * 5 + 3;
        captured.grow(&mut c);
        assert!(captured.exile_tags.first.len() >= before * 5 + 3);
        assert!(c.exile_tags >= before * 5 + 3);
    }

    /// Exile spans index the shared tag arena; a division whose template is
    /// not exiled (140C79AC0) carries no exile operands.
    #[test]
    fn naming_direction_and_exile_operands_project_onto_division_rows() {
        let c = Capacities {
            combats: 1,
            census_participants: 1,
            details: 1,
            divisions: 2,
            sides: 1,
            combat_sides: 1,
            country_tags: 1,
            rosters: 1,
            modifier_members: 1,
            modifiers: 1,
            width_terms: 1,
            exile_tags: 1,
            division_names: 1,
            name_templates: 1,
            name_groups: 1,
            reinforcement_modifiers: 1,
            supply_terms: 1,
            acclimatization: 1,
            transport_terms: 1,
            tactic_candidates: 1,
            tactic_weights: 1,
            tactic_unlocks: 1,
            division_commanders: 1,
            commander_names: 1,
            commander_keys: 2,
        };
        let mut captured = Captured::with_capacities(&c);
        captured.divisions.outcome = Outcome::Rows { len: 2 };
        captured.exile_tags.first = vec![99, 7, 8, 11];
        captured.exile_tags.len = 4;
        // Names index the pass's byte arena: a key and a player's name.
        captured.division_names.first = b"xGER_INF_01Guards".to_vec();
        captured.division_names.len = 17;
        let stats = carmy_combat_inputs::Row {
            direction_group: -3,
            categories: 0x4000_0000_0000_0001,
            naming: Some(engine::cname_group_member::Inputs {
                kind: 2,
                order: 14,
                ordered: 1,
                custom: (11, 6),
                group: (1, 10),
                ..Default::default()
            }),
            ..Default::default()
        };
        captured.divisions.rows[0] = Division {
            combat: 0x1000_0000_003e,
            province: 362,
            role: 1,
            stats,
            exile: Some(carmy_combat_inputs::ExileRow {
                origin_type: 2,
                country: 0,
                country_fallback: 12,
                cores: (1, 2),
                occupiers: (3, 1),
            }),
            ..Default::default()
        };
        captured.divisions.rows[1] = Division {
            combat: 0x1000_0000_003e,
            ordinal: 1,
            role: 1,
            exile: None,
            ..Default::default()
        };
        let [
            _,
            TableView::Complete(replay::rows::TableRows::LandCombatDivisions(divisions)),
            _,
            _,
        ] = captured.tables()
        else {
            panic!("complete land divisions were not projected")
        };
        assert_eq!(divisions[0].province, 362);
        assert_eq!(divisions[0].direction_group, -3);
        assert_eq!(divisions[0].categories, 0x4000_0000_0000_0001);
        assert_eq!(
            (
                divisions[0].name_type,
                divisions[0].name_order,
                divisions[0].name_ordered,
                divisions[0].name_group.as_slice(),
                divisions[0].name_custom.as_slice()
            ),
            (2, 14, 1, b"GER_INF_01".as_slice(), b"Guards".as_slice())
        );
        assert_eq!(
            divisions[0].exile,
            Some(LandCombatExileInputs {
                origin_type: 2,
                country: 0,
                country_fallback: 12,
                location_cores: vec![7, 8],
                occupiers: vec![11],
            })
        );
        assert_eq!(divisions[1].exile, None);
    }

    /// The reinforcement reader writes enemy transport terms straight into the
    /// arena past its admitted length and grows it on the worker; a division
    /// with thousands of enemies projects every term in native order and the
    /// grown arena feeds the next capacity.
    #[test]
    fn transport_terms_beyond_the_default_arena_project_every_enemy() {
        let mut c = Capacities {
            combats: 1,
            census_participants: 1,
            details: 1,
            divisions: 2,
            sides: 1,
            combat_sides: 1,
            country_tags: 1,
            rosters: 1,
            modifier_members: 1,
            modifiers: 1,
            width_terms: 1,
            exile_tags: 1,
            division_names: 1,
            name_templates: 1,
            name_groups: 1,
            reinforcement_modifiers: 1,
            supply_terms: 1,
            acclimatization: 1,
            transport_terms: 1,
            tactic_candidates: 1,
            tactic_weights: 1,
            tactic_unlocks: 1,
            division_commanders: 1,
            commander_names: 1,
            commander_keys: 2,
        };
        let mut captured = Captured::with_capacities(&c);
        let enemies = 5_000;
        let term = |index: usize| reinforcement::TransportTerm {
            country: index as u32 + 2,
            regional_value: index as i64 * 10,
            own_intel: i64::MIN + index as i64,
            enemy_intel: -(index as i64),
        };
        // What the reader leaves behind: the arena grown past its capacity.
        captured
            .transport_terms
            .first
            .resize(1 + enemies, reinforcement::TransportTerm::default());
        captured.transport_terms.first[0] = term(usize::MAX >> 40);
        for index in 0..enemies {
            captured.transport_terms.first[1 + index] = term(index);
        }
        captured.transport_terms.len = 1 + enemies;
        captured.transport_terms.required = 1 + enemies;
        captured.divisions.outcome = Outcome::Rows { len: 2 };
        for (ordinal, row) in captured.divisions.rows[..2].iter_mut().enumerate() {
            row.combat = 0x1000_0000_003e;
            row.ordinal = ordinal as u32;
        }
        captured.divisions.rows[0].reinforcement.transport_term_count = 1;
        captured.divisions.rows[1].reinforcement_starts[3] = 1;
        captured.divisions.rows[1].reinforcement.transport_term_count = enemies;
        let [
            _,
            TableView::Complete(replay::rows::TableRows::LandCombatDivisions(divisions)),
            _,
            _,
        ] = captured.tables()
        else {
            panic!("complete land divisions were not projected")
        };
        assert_eq!(divisions[0].reinforcement_transport.len(), 1);
        let projected = &divisions[1].reinforcement_transport;
        assert_eq!(projected.len(), enemies);
        for (index, row) in projected.iter().enumerate() {
            let expected = term(index);
            assert_eq!(
                (row.country, row.regional_value, row.own_intel, row.enemy_intel),
                (
                    expected.country,
                    expected.regional_value,
                    expected.own_intel,
                    expected.enemy_intel
                )
            );
        }
        captured.grow(&mut c);
        assert!(c.transport_terms > enemies);
        assert!(captured.transport_terms.first.len() >= c.transport_terms);
    }

    #[test]
    fn source_failures_and_primary_arena_capacity_are_explicit() {
        let mut c = Capacities {
            combats: 1,
            census_participants: 2,
            details: 1,
            divisions: 2,
            sides: 2,
            combat_sides: 2,
            country_tags: 2,
            rosters: 2,
            modifier_members: 2,
            modifiers: 2,
            width_terms: 2,
            exile_tags: 2,
            division_names: 1,
            name_templates: 1,
            name_groups: 1,
            reinforcement_modifiers: 2,
            supply_terms: 2,
            acclimatization: 2,
            transport_terms: 2,
            tactic_candidates: 2,
            tactic_weights: 2,
            tactic_unlocks: 2,
            division_commanders: 2,
            commander_names: 64,
            commander_keys: 1,
        };
        let mut captured = Captured::with_capacities(&c);
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        captured.capture(&memory, 0, 0);
        assert!(captured.commander_keys.len() >= captured.census_participants.len());
        assert!(
            captured
                .tables()
                .iter()
                .all(|t| matches!(t, TableView::Unavailable { .. }))
        );
        assert!(matches!(
            Progress {
                len: 1,
                required: 1,
                failure: None
            }
            .outcome(Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }),
            Outcome::Unavailable { .. }
        ));
        captured.reset();
        captured.sides.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 3,
        };
        captured.members.required = 7;
        captured.modifiers.required = 19;
        captured.width_terms.required = 9;
        captured.division_commanders.required = 5;
        captured.required_commander_keys = 7;
        captured.tactic_candidates.required = 7;
        captured.tactic_weights.required = 11;
        captured.tactic_unlocks.required = 13;
        captured.grow(&mut c);
        assert!(c.division_commanders >= 5 && c.commander_keys >= 7);
        assert!(c.tactic_candidates >= 7 && c.tactic_weights >= 11 && c.tactic_unlocks >= 13);
        assert!(c.sides >= 4 && c.modifier_members >= 7 && c.modifiers >= 19 && c.width_terms >= 9);
        captured.details.outcome = Outcome::Rows { len: 1 };
        captured.divisions.outcome = Outcome::Unavailable {
            reason: "native division read failed",
        };
        captured.sides.outcome = Outcome::Rows { len: 1 };
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 1 });
        assert!(matches!(
            captured.outcomes()[1],
            Outcome::Unavailable { .. }
        ));
        assert_eq!(captured.outcomes()[2], Outcome::Rows { len: 1 });
        let allocated = captured.allocated_bytes();
        captured.fail("ordinary worker unavailable");
        assert_eq!(
            captured.outcomes(),
            [Outcome::Unavailable {
                reason: "ordinary worker unavailable"
            }; N],
        );
        assert_eq!(captured.allocated_bytes(), allocated);
    }

    #[test]
    fn combat_sides_project_independently_of_the_larger_tables() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.details.outcome = Outcome::Unavailable {
            reason: "tactic context failed",
        };
        captured.country_tags.first[..5].copy_from_slice(&[7, 0, 9, 4, 2]);
        captured.combat_sides.rows[0] = CombatSides {
            sets: cland_combat::CountrySetsRow {
                combat: 62,
                indicator: 1,
                flags: [0x4000, 0x800],
                start: 0,
                lens: [2, 1, 2, 0],
            },
        };
        captured.combat_sides.outcome = Outcome::Rows { len: 1 };
        let [
            TableView::Unavailable { .. },
            _,
            _,
            TableView::Complete(replay::rows::TableRows::LandCombatSides(sides)),
        ] = captured.tables()
        else {
            panic!("the side table was not projected independently")
        };
        assert_eq!(
            sides,
            [LandCombatSideRow {
                combat: 62,
                indicator: 1,
                flags: [0x4000, 0x800],
                attacker_primary: vec![7, 0],
                attacker_secondary: vec![9],
                defender_primary: vec![4, 2],
                defender_secondary: vec![],
            }]
        );
    }
}

/// The whole land combat family over a synthetic combat world, walked as the
/// recorder walks it and as the reference walk (main's reads, nothing
/// shared): the census, window operands, details, tactic registry and
/// context, modifier sides and every division reader run on total memory
/// (any address from 0x1_0000 reads, as zero unless written) with the module
/// at base 0.
#[cfg(test)]
mod family {
    use super::*;
    use engine::{
        carmy, carmy_combat_inputs as inputs, cbuilding_database, ccombat_tactic, ccountry,
        cgame_state, cland_combatant, cnaval_combat, cprovince,
        creferenced_division_template as template, cstate, cstrategic_region, csub_unit_definition,
        cterrain_type, cunit, cweather_manager, modifier_definition, province_definition,
        sweather_per_region,
    };
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const PAGE: usize = 4096;
    const GAME: usize = 0x4000_0000;

    #[derive(Default)]
    struct World {
        pages: HashMap<usize, Box<[u8; PAGE]>>,
        next: usize,
        /// Guarded calls, as the recorder's live reader counts them.
        calls: AtomicUsize,
        /// Byte ranges whose reads fail.
        poisoned: Vec<(usize, usize)>,
        /// Every guarded span, when logging.
        log: std::sync::Mutex<Option<Vec<(usize, usize)>>>,
        /// Every guarded call's first address, when logging calls.
        call_log: std::sync::Mutex<Option<Vec<usize>>>,
    }

    impl World {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            for (index, byte) in bytes.iter().enumerate() {
                let address = at + index;
                self.pages
                    .entry(address / PAGE)
                    .or_insert_with(|| Box::new([0; PAGE]))[address % PAGE] = *byte;
            }
        }
        fn u64(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn u32(&mut self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }
        fn alloc(&mut self, size: usize) -> usize {
            if self.next == 0 {
                self.next = 0x1_0000_0000;
            }
            let at = self.next;
            self.next += size.next_multiple_of(64);
            at
        }
        /// A CPdxArray header: data, capacity, count.
        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.u64(at, data as u64);
            self.u32(at + 8, count);
            self.u32(at + 12, count);
        }
        fn object(&mut self, size: usize, vtable: usize) -> usize {
            let at = self.alloc(size);
            self.u64(at, vtable as u64);
            at
        }
        /// An inline MSVC string.
        fn text(&mut self, at: usize, text: &[u8]) {
            assert!(text.len() <= 15);
            self.put(at, text);
            self.u64(at + 16, text.len() as u64);
            self.u64(at + 24, 15);
        }
        fn accumulator(&mut self, at: usize, entries: &[(u32, i64)]) {
            let data = self.alloc(entries.len().max(1) * 16);
            for (index, (key, value)) in entries.iter().enumerate() {
                self.u32(data + index * 16, *key);
                self.u64(data + index * 16 + 8, *value as u64);
            }
            self.array(at, data, entries.len() as u32);
        }
        fn poison(&mut self, at: usize, len: usize) {
            self.poisoned.push((at, len));
        }
        fn get_u64(&self, at: usize) -> u64 {
            let mut bytes = [0; 8];
            assert!(self.fill(at, &mut bytes));
            u64::from_le_bytes(bytes)
        }
        fn fill(&self, address: usize, output: &mut [u8]) -> bool {
            if let Some(log) = self.log.lock().unwrap().as_mut() {
                log.push((address, output.len()));
            }
            // A process's first pages fault: a null object's field fails.
            if address < 0x1_0000
                || self
                    .poisoned
                    .iter()
                    .any(|&(at, len)| address < at + len && at < address + output.len())
            {
                return false;
            }
            let mut done = 0;
            while done < output.len() {
                let at = address + done;
                let offset = at % PAGE;
                let chunk = (PAGE - offset).min(output.len() - done);
                match self.pages.get(&(at / PAGE)) {
                    Some(page) => {
                        output[done..done + chunk].copy_from_slice(&page[offset..offset + chunk])
                    }
                    None => output[done..done + chunk].fill(0),
                }
                done += chunk;
            }
            true
        }
    }

    impl World {
        fn call(&self, address: usize) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if let Some(log) = self.call_log.lock().unwrap().as_mut() {
                log.push(address);
            }
        }
    }

    impl Memory for World {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.call(address);
            self.fill(address, output)
        }
        fn copy_scattered<const M: usize>(
            &self,
            addresses: [usize; M],
            destinations: &mut [&mut [u8]; M],
        ) -> usize {
            self.call(addresses.first().copied().unwrap_or(0));
            for (ordinal, (address, destination)) in
                addresses.into_iter().zip(destinations).enumerate()
            {
                if !destination.is_empty() && !self.fill(address, destination) {
                    return ordinal;
                }
            }
            M
        }
        /// One guarded call, as the recorder's live gather makes one.
        fn copy_records(
            &self,
            sources: engine::memory::RecordSources<'_>,
            layout: &engine::memory::RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.call(sources.pointer(0).unwrap_or(0));
            let mut completed = 0;
            for row in 0..sources.len() {
                for field in layout.fields() {
                    if field.len != 0 {
                        let Some(address) = sources
                            .pointer(row)
                            .filter(|&pointer| pointer != 0)
                            .and_then(|pointer| pointer.checked_add(field.source_offset))
                        else {
                            return completed;
                        };
                        let start = row * layout.stride() + field.destination_offset;
                        if !self.fill(address, &mut destination[start..start + field.len]) {
                            return completed;
                        }
                    }
                    completed += 1;
                }
            }
            completed
        }
    }

    /// The Ordinary job's country roster over `world`: an authenticated
    /// census, as the callback prepares it.
    fn roster(world: &World) -> engine::country_memory::Cache {
        let mut cache = engine::country_memory::Cache::with_capacity(
            engine::country_memory::Cache::cells_for_countries(8192, 1024),
        );
        cache.prepare(world, 0, GAME);
        world.calls.store(0, Ordering::Relaxed);
        cache
    }

    /// The fixture behind the recorder's read accounting.
    struct Shared<'a>(&'a World);

    impl Memory for Shared<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.0.copy(address, output)
        }
        fn copy_scattered<const M: usize>(
            &self,
            addresses: [usize; M],
            destinations: &mut [&mut [u8]; M],
        ) -> usize {
            self.0.copy_scattered(addresses, destinations)
        }
        fn copy_records(
            &self,
            sources: engine::memory::RecordSources<'_>,
            layout: &engine::memory::RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.0.copy_records(sources, layout, destination)
        }
    }

    type Rostered<'a> =
        engine::country_memory::CountryMemory<'a, engine::memory::CountedMemory<Shared<'a>>, false>;

    /// Guarded calls per table, attributed as the recorder attributes them,
    /// over the authenticated roster.
    fn per_table(
        world: &World,
        cache: &engine::country_memory::Cache,
        work: impl FnOnce(&Rostered<'_>),
    ) -> [u64; N] {
        let counted = engine::memory::CountedMemory::new(Shared(world));
        let capture = crate::read_accounting::Capture::new();
        work(&cache.roster_view(&counted));
        let counts = capture.finish();
        TABLES.map(|table| {
            let c = counts[table.index()];
            c.copy_calls + c.scattered_calls + c.record_calls + c.plan_calls
        })
    }

    struct Shape {
        countries: u32,
        combats: usize,
        /// Divisions per combat, both sides.
        divisions: usize,
        owners: usize,
        templates: usize,
        states: usize,
        provinces: usize,
        relations: usize,
        transports: usize,
        tactics: u32,
        /// Army leaders; every fourth division, from the second, has one.
        leaders: usize,
    }

    impl Shape {
        fn small() -> Self {
            Self {
                countries: 24,
                combats: 7,
                divisions: 5,
                owners: 5,
                templates: 7,
                states: 5,
                provinces: 11,
                relations: 6,
                transports: 3,
                tactics: 2,
                leaders: 3,
            }
        }
        fn scale(divisions: usize) -> Self {
            Self {
                countries: 1_500,
                combats: divisions.div_ceil(20),
                divisions: 20,
                owners: (divisions / 25).clamp(6, 150),
                templates: (divisions / 6).max(20),
                states: (divisions / 8).max(10),
                provinces: (divisions / 5).max(20),
                relations: 20,
                transports: divisions / 50,
                tactics: 40,
                leaders: (divisions / 20).clamp(1, 48),
            }
        }
    }

    /// A built world and what the variants break.
    struct Built {
        world: World,
        /// The CReferencedDivisionTemplate objects, registered by handle.
        templates: Vec<usize>,
        combats: Vec<usize>,
        /// Army pointers per combat, attackers first.
        armies: Vec<Vec<usize>>,
        /// Attacker and defender CLandCombatant per combat.
        sides: Vec<[usize; 2]>,
    }

    fn world(shape: &Shape) -> Built {
        let mut w = World::default();
        let slots = shape.countries as usize + 1;
        // Module data: definitions, defines and singletons.
        let table = w.alloc(0x78 * 0x300);
        w.u64(modifier_definition::TABLE_RVA, table as u64);
        for key in [0x2b_usize, 0x5e, 0x17f, 0x1e9, 0xfb, 0x10f] {
            w.u32(
                table + key * modifier_definition::STRIDE + modifier_definition::FLAGS,
                3,
            );
        }
        let database = w.alloc(0x400);
        w.u64(cbuilding_database::SINGLETON_RVA, database as u64);
        let definition = w.alloc(0x400);
        w.u64(database + 0x3b0, definition as u64);
        // The tactic database: its null entry and `tactics` definitions.
        let tactics = w.alloc(0x100);
        w.u64(land_tactic_inputs::DATABASE_RVA, tactics as u64);
        let entries = w.alloc((shape.tactics as usize + 1) * 8);
        w.array(tactics + 0x10, entries, shape.tactics + 1);
        for id in 1..=shape.tactics {
            let tactic = w.object(0x200, ccombat_tactic::VTABLE_RVA);
            w.u32(tactic + ccombat_tactic::ID, id);
            w.text(
                tactic + ccombat_tactic::TOKEN,
                format!("tactic_{id}").as_bytes(),
            );
            w.u32(tactic + 0xc0, 10 * id);
            w.u64(entries + id as usize * 8, tactic as u64);
        }
        let doctrine = w.alloc(0x40);
        w.u64(GAME + land_tactic_inputs::DOCTRINE_SYSTEM, doctrine as u64);
        // Countries.
        let countries = w.alloc(slots * 8);
        let links = w.alloc(slots * 4);
        w.array(GAME + cgame_state::COUNTRIES, countries, slots as u32);
        w.array(GAME + cgame_state::COUNTRY_LINKS, links, slots as u32);
        let relation = w.alloc(0x40);
        w.u32(relation + 8, 0x3801);
        let air_manager = w.alloc(0x100);
        w.u64(
            GAME + cgame_state::STRATEGIC_AIR_MANAGER,
            air_manager as u64,
        );
        let air_countries = w.alloc(slots * 8);
        w.array(air_manager + 0x30, air_countries, slots as u32);
        for slot in 1..slots {
            w.u32(links + slot * 4, slot as u32);
        }
        let mut owners = Vec::new();
        let mut diplomacies = Vec::new();
        for owner in 0..shape.owners {
            let slot = 1 + owner * (slots - 1) / shape.owners.max(1);
            let country = w.object(0x1000, ccountry::VTABLE_RVA);
            w.u32(country + ccountry::ID, slot as u32);
            w.u64(countries + slot * 8, country as u64);
            w.accumulator(
                country + ccountry::MODIFIER_ACCUMULATOR,
                &[
                    (0x2b, 5_000),
                    (0x5e, -2_000),
                    (0xfb, 7),
                    (0x17f, 11),
                    (0x1e9, 13),
                ],
            );
            let diplomacy = w.object(0x300, engine::cdiplomacy_status::VTABLE_RVA);
            w.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
            w.u64(
                diplomacy + engine::cdiplomacy_status::COUNTRY,
                country as u64,
            );
            diplomacies.push(diplomacy);
            // Every third owner is at war with the next owner: its enemy list,
            // whose count (CDiplomacyStatus+0xA4) is the air test's at-war
            // answer.
            let next = 1 + ((owner + 1) % shape.owners) * (slots - 1) / shape.owners;
            let tags = w.alloc(8);
            w.u32(tags, slot as u32);
            w.u32(tags + 4, next as u32);
            w.array(diplomacy + inputs::OCCUPIER_SET, tags, 2);
            let list = w.alloc(4);
            let at_war = owner % 3 == 0;
            if at_war {
                let enemy = (owner + 1) % shape.owners;
                w.u32(list, (1 + enemy * (slots - 1) / shape.owners) as u32);
            }
            w.array(
                diplomacy + engine::cdiplomacy_status::ENEMY_COUNTRIES,
                list,
                u32::from(at_war),
            );
            let list = w.alloc(shape.relations.max(1) * 8);
            for index in 0..shape.relations {
                w.u64(list + index * 8, relation as u64);
            }
            w.array(diplomacy + 0x20, list, shape.relations as u32);
            let air = w.alloc(0x100);
            w.u64(air_countries + slot * 8, air as u64);
            let regions = w.alloc(0xa0 * 4);
            w.array(air + 0xe0, regions, 4);
            for index in 0..4 {
                w.u64(regions + index * 0xa0, 100 + index as u64);
                w.u64(regions + index * 0xa0 + 8, index as u64);
                w.u64(regions + index * 0xa0 + 0x18, 900 + index as u64);
            }
            owners.push(slot as u32);
        }
        for (from, &diplomacy) in diplomacies.iter().enumerate() {
            let row = w.alloc(slots * 8);
            w.array(
                diplomacy + engine::cdiplomacy_status::RELATIONS,
                row,
                slots as u32,
            );
            for (to, &target) in owners.iter().enumerate() {
                if to == from {
                    continue;
                }
                let tuple = w.object(0x300, engine::crelation_status::VTABLE_RVA);
                w.u64(
                    tuple + engine::crelation_status::DIPLOMACY_STATUS,
                    diplomacy as u64,
                );
                w.u32(tuple + engine::crelation_status::TARGET_COUNTRY, target);
                w.u64(tuple + 0x2a0, u64::from((from + to) % 2 == 0));
                w.u64(row + target as usize * 8, tuple as u64);
            }
        }
        // Weather: two strategic regions with records.
        let weather = w.object(0x100, cweather_manager::VTABLE_RVA);
        w.u64(GAME + cgame_state::WEATHER_MANAGER, weather as u64);
        let records = w.alloc(sweather_per_region::STRIDE * 2);
        w.array(weather + cweather_manager::REGIONS, records, 2);
        let mut regions = Vec::new();
        for id in 0..2_u32 {
            let record = records + id as usize * sweather_per_region::STRIDE;
            w.u64(record, sweather_per_region::VTABLE_RVA as u64);
            w.u32(record + sweather_per_region::ID, id);
            w.accumulator(record + sweather_per_region::MODIFIERS, &[(0x29, 3)]);
            let region = w.object(0x100, cstrategic_region::VTABLE_RVA);
            w.u32(region + cstrategic_region::ID, id);
            w.u32(region + 0x60, id + 1);
            regions.push(region);
        }
        // Sub-units and templates.
        let subunits: Vec<usize> = (0..6)
            .map(|index| {
                let unit = w.object(0x600, csub_unit_definition::VTABLE_RVA);
                w.u32(unit + 8, 40 + index);
                w.u64(unit + 0x58, 200_000 + u64::from(index) * 50_000);
                unit
            })
            .collect();
        let templates: Vec<usize> = (0..shape.templates)
            .map(|index| {
                let t = w.object(0x600, template::VTABLE_RVA);
                w.u64(
                    t + 8,
                    ((index as u64 + 1) << 32) | u64::from(template::REFERENCE_TYPE),
                );
                let terms = w.alloc(16 * 4);
                let count = 2 + index % 3;
                for term in 0..count {
                    w.u64(terms + term * 16, subunits[(index + term) % 6] as u64);
                    w.u32(terms + term * 16 + 8, 1 + term as u32);
                }
                w.array(t + 0xc0, terms, count as u32);
                w.u64(t + template::PRIORITY_UNIT, subunits[index % 6] as u64);
                // Every other template is an exile template of an owner.
                if index % 2 == 0 {
                    w.u32(t + inputs::TEMPLATE_ORIGIN_TYPE, 2);
                    w.u32(t + inputs::TEMPLATE_COUNTRY, owners[index % owners.len()]);
                }
                t
            })
            .collect();
        register_templates(&mut w, &templates);
        // Three naming groups the templates share, so several divisions'
        // generated names store one group key between them.
        let name_groups: Vec<usize> = (0..3)
            .map(|index| {
                let group = w.object(0x40, engine::cname_group_member::GROUP_VTABLE_RVA);
                w.text(
                    group + engine::cname_group_member::GROUP_KEY,
                    format!("GROUP_{index}").as_bytes(),
                );
                group
            })
            .collect();
        for (index, &t) in templates.iter().enumerate() {
            w.u64(
                t + engine::cname_group_member::TEMPLATE_NAME_GROUP,
                name_groups[index % name_groups.len()] as u64,
            );
        }
        // States, terrain and provinces, registered by id.
        let states: Vec<usize> = (0..shape.states)
            .map(|index| {
                let state = w.object(0x1000, cstate::VTABLE_RVA);
                w.u32(state + cstate::ID, 500 + index as u32);
                w.u32(state + cstate::CONTROLLER, owners[index % owners.len()]);
                let cores = w.alloc(4);
                w.u32(cores, owners[(index + 1) % owners.len()]);
                w.array(state + cstate::CORES, cores, 1);
                let buckets = w.alloc(0x40);
                w.u64(state + cstate::COUNTRY_MODIFIERS, buckets as u64);
                w.accumulator(
                    state + cstate::RESOURCE_MODIFIER + 0x10,
                    &[(0x2b, 4), (0x1e7, 9)],
                );
                state
            })
            .collect();
        let terrains: Vec<usize> = (0..2)
            .map(|index| {
                let terrain = w.object(0x100, cterrain_type::VTABLE_RVA);
                w.put(terrain + cterrain_type::VALID, &[1]);
                w.text(
                    terrain + cterrain_type::NAME,
                    [&b"plains"[..], &b"forest"[..]][index],
                );
                w.u64(
                    terrain + cterrain_type::COMBAT_WIDTH,
                    9_000_000 + index as u64,
                );
                terrain
            })
            .collect();
        let first_province = 1000;
        let registered = w.alloc((first_province + shape.provinces) * 8);
        w.array(
            GAME + cgame_state::PROVINCES,
            registered,
            (first_province + shape.provinces) as u32,
        );
        let provinces: Vec<usize> = (0..shape.provinces)
            .map(|index| {
                let province = w.object(0x200, cprovince::VTABLE_RVA);
                let definition = w.alloc(0x100);
                let id = first_province + index;
                w.u32(province + cprovince::ID, id as u32);
                w.u32(definition + province_definition::ID, id as u32);
                w.u64(province + cprovince::DEFINITION, definition as u64);
                w.u64(
                    province + cprovince::STATE,
                    states[index % states.len()] as u64,
                );
                if index % 4 != 3 {
                    w.u64(
                        province + cprovince::STRATEGIC_REGION,
                        regions[index % 2] as u64,
                    );
                }
                w.u64(registered + id * 8, province as u64);
                province
            })
            .collect();
        // Army leaders with their orders groups, resolved by reference.
        let groups = leaders(&mut w, shape.leaders, owners[0]);
        // Divisions.
        let total = shape.combats * shape.divisions;
        let mut armies = Vec::new();
        for index in 0..total {
            let army = w.object(0x800, carmy::VTABLE_RVA);
            w.u64(
                army + carmy::SECONDARY_BASE,
                carmy::SECONDARY_VTABLE_RVA as u64,
            );
            let handle = ((index as u64 + 1) << 32) | 3;
            w.u64(army + cunit::HANDLE, handle);
            let stats = w.object(0x600, csub_unit_definition::VTABLE_RVA);
            w.u64(army + cunit::STAT_BLOCK, stats as u64);
            let province = provinces[(index * 7) % provinces.len()];
            w.u64(army + cunit::LOCATION, province as u64);
            w.u64(
                army + carmy::STANDARD_TEMPLATE,
                templates[(index * 5) % templates.len()] as u64,
            );
            let owner = owners[index % owners.len()];
            w.u32(army + cunit::ROSTER_OWNER, owner);
            w.u32(army + cunit::LOGICAL_OWNER, owner);
            // Three naming shapes: a type-zero name from the template's shared
            // group, a player's custom name, and a generated name of nonzero
            // type (which reads nothing further).
            let name = w.alloc(0x100);
            w.u64(army + inputs::NAME_GROUP_MEMBER, name as u64);
            w.u64(name, engine::cname_group_member::VTABLE_RVA as u64);
            w.u32(name + engine::cname_group_member::ORDER, index as u32);
            let template = templates[(index * 5) % templates.len()];
            match index % 3 {
                0 => {
                    w.text(name + engine::cname_group_member::OVERRIDE_NAME, b"");
                    w.u64(
                        name + engine::cname_group_member::TEMPLATE_REFERENCE,
                        w.get_u64(template + 8),
                    );
                }
                1 => w.text(
                    name + engine::cname_group_member::OVERRIDE_NAME,
                    format!("Guard {}", index % 1000).as_bytes(),
                ),
                _ => {
                    w.u32(name + engine::cname_group_member::TYPE, 1);
                    w.put(name + engine::cname_group_member::FLAG_B, &[1]);
                }
            }
            w.put(army + 0x2e, &[1]);
            if index < shape.transports {
                w.u64(army + carmy::NAVAL_TRANSFER_MEMBERSHIP, 1);
                w.u64(army + cunit::LOCATION, provinces[0] as u64);
            }
            let modifiers = index % 3;
            let data = w.alloc(modifiers.max(1) * 24);
            for entry in 0..modifiers {
                w.u32(data + entry * 24, 10 + entry as u32);
                w.u64(data + entry * 24 + 8, (index * 100 + entry) as u64);
                w.u64(data + entry * 24 + 16, (entry as i64 - 1) as u64);
            }
            w.array(army + cunit::COMBAT_MODIFIER_DATA, data, modifiers as u32);
            if !groups.is_empty() && index % 4 == 1 {
                w.u64(
                    army + carmy::ORDERS_GROUP,
                    groups[(index / 4) % groups.len()] as u64,
                );
            }
            armies.push(army);
        }
        // Combats: a naval combat first, which the land census skips.
        let registry = w.alloc((shape.combats + 1) * 8);
        w.array(
            GAME + cgame_state::COMBATS + 8,
            registry,
            shape.combats as u32 + 1,
        );
        let naval = w.object(0x100, cnaval_combat::VTABLE_RVA);
        w.u64(registry, naval as u64);
        let mut combats = Vec::new();
        let mut members = Vec::new();
        let mut all_sides = Vec::new();
        for combat in 0..shape.combats {
            let object = w.object(0x600, cland_combat::VTABLE_RVA);
            w.u64(
                object + cland_combat::BODY,
                cland_combat::BODY_VTABLE_RVA as u64,
            );
            w.u64(
                object + cland_combat::HANDLE,
                ((combat as u64 + 1) << 32) | u64::from(cland_combat::TYPE_WORD),
            );
            w.u64(
                object + cland_combat::PROVINCE,
                provinces[(combat * 3) % provinces.len()] as u64,
            );
            w.u64(
                object + cland_combat::STORED_TERRAIN,
                terrains[combat % 2] as u64,
            );
            w.put(object + cland_combat::INDICATOR_FRAME_HIGH, &[combat as u8]);
            w.u64(object + cland_combat::WIDTH_FACTOR, (combat as u64) * 1_000);
            let own = armies[combat * shape.divisions..(combat + 1) * shape.divisions].to_vec();
            let attackers = (shape.divisions / 2 + combat % 2).clamp(1, shape.divisions - 1);
            let mut pair = [0; 2];
            for (index, (units, is_attacker)) in [(&own[..attackers], 1_u8), (&own[attackers..], 0)]
                .into_iter()
                .enumerate()
            {
                let side = w.object(0x600, cland_combatant::VTABLE_RVA);
                w.u64(side + cland_combatant::COMBAT, object as u64);
                w.put(side + cland_combatant::IS_ATTACKER, &[is_attacker]);
                w.u32(side + cland_combatant::FLAGS, (combat * 2 + index) as u32);
                let list = w.alloc(units.len() * 8);
                for (ordinal, &unit) in units.iter().enumerate() {
                    w.u64(list + ordinal * 8, unit as u64);
                }
                w.array(side + cland_combatant::UNITS, list, units.len() as u32);
                // Committed first, then one reserve, the rest unassigned.
                let committed = units.len().div_ceil(2);
                let reserves = usize::from(units.len() > committed);
                w.array(side + cland_combatant::COMMITTED, list, committed as u32);
                w.array(
                    side + cland_combatant::RESERVES,
                    list + committed * 8,
                    reserves as u32,
                );
                pair[index] = side;
            }
            w.u64(object + cland_combat::ATTACKER, pair[0] as u64);
            w.u64(object + cland_combat::DEFENDER, pair[1] as u64);
            w.u64(registry + (combat + 1) * 8, object as u64);
            combats.push(object);
            members.push(own);
            all_sides.push(pair);
        }
        Built {
            world: w,
            templates,
            combats,
            armies: members,
            sides: all_sides,
        }
    }

    /// `count` category-2 army leaders (no formation), each in its own orders
    /// group, resolvable through the high reference root. Returns the groups.
    /// Registers the templates by their handle in the reference registry's
    /// root for their type, one bucket each at its home slot.
    fn register_templates(w: &mut World, templates: &[usize]) {
        use engine::{creference_registry as registry, reference_bucket as bucket};
        // At least twice the templates in buckets, a power of two: the
        // header's overflow distance is the exponent plus two.
        let buckets = (templates.len() * 2).next_power_of_two().max(8);
        let (mask, overflow) = (buckets as u32 - 1, buckets.trailing_zeros() as u8 + 2);
        let table = w.alloc(0x40);
        let entries = w.alloc((mask as usize + 1 + overflow as usize + 1) * bucket::STRIDE);
        w.u64(
            registry::LOW_ROOTS_RVA + template::REFERENCE_TYPE as usize * 8,
            table as u64,
        );
        w.u64(table + registry::ENTRIES, entries as u64);
        w.u32(table + registry::COUNT, templates.len() as u32);
        w.u32(table + registry::MASK, mask);
        w.put(table + registry::OVERFLOW_DISTANCE, &[overflow]);
        w.put(
            entries + (mask as usize + 1 + overflow as usize) * bucket::STRIDE + bucket::DISTANCE,
            &[bucket::TERMINAL_DISTANCE],
        );
        // Robin Hood insertion, as the registry keeps its buckets: a probe
        // stops at a bucket whose distance is below its own, so an entry
        // displaced from home takes the slot of any closer-to-home entry.
        let mut buckets: Vec<Option<(u64, usize, u32)>> =
            vec![None; mask as usize + 1 + overflow as usize];
        for &t in templates {
            let mut entry = (w.get_u64(t + 8), t, 1_u32);
            let hash = entry
                .0
                .rotate_left(32)
                .wrapping_mul(registry::HASH_MULTIPLIER);
            let mut index = (((hash >> 32) as u32 ^ hash as u32) & mask) as usize;
            loop {
                assert!(entry.2 <= u32::from(overflow) + 1, "template buckets overflow");
                match &mut buckets[index] {
                    slot @ None => {
                        *slot = Some(entry);
                        break;
                    }
                    Some(resident) if resident.2 < entry.2 => std::mem::swap(resident, &mut entry),
                    Some(_) => {}
                }
                index += 1;
                entry.2 += 1;
            }
        }
        for (index, filled) in buckets.iter().enumerate() {
            if let Some((handle, t, distance)) = *filled {
                let cell = entries + index * bucket::STRIDE;
                w.put(cell + bucket::DISTANCE, &[distance as u8]);
                w.u64(cell + 8, handle);
                w.u64(cell + 16, t as u64);
            }
        }
    }

    /// Divisions sharing group keys store each key once, as the reference
    /// walk does: with a name arena too small for every name, both publish
    /// the same rows, outcomes and demand, and after growth both keep all.
    #[test]
    fn shared_naming_keys_and_custom_names_match_the_reference_under_pressure() {
        let shape = Shape::small();
        let built = world(&shape);
        for names in [0, 8, 24, 64] {
            assert_equivalent_with(
                &built.world,
                Capacities {
                    division_names: names,
                    ..roomy()
                },
            );
        }
        // A naming group whose key cannot be read fails the same division in
        // both walks.
        let mut built = world(&shape);
        let group = built
            .world
            .get_u64(built.templates[0] + engine::cname_group_member::TEMPLATE_NAME_GROUP)
            as usize;
        built
            .world
            .poison(group + engine::cname_group_member::GROUP_KEY + 16, 8);
        assert_equivalent(&built.world);
    }

    fn leaders(w: &mut World, count: usize, country: u32) -> Vec<usize> {
        use engine::{
            carmy_leader, cnull_combat_tactic, corders_group, creference_registry as registry,
            cunit_leader, cunit_leader_skill, reference_bucket as bucket,
        };
        if count == 0 {
            return Vec::new();
        }
        let skill = w.object(0x200, cunit_leader_skill::VTABLE_RVA);
        w.u32(skill + cunit_leader_skill::LEVEL, 3);
        w.u32(skill + cunit_leader_skill::CATEGORY, 2);
        let skills = w.alloc(0x200);
        w.u64(
            engine::naval_commander_inputs::SKILL_DATABASE_RVA,
            skills as u64,
        );
        let levels = w.alloc(8 * 5);
        for level in 0..5 {
            w.u64(levels + level * 8, skill as u64);
        }
        w.array(skills + 0xe8 + 2 * 0x18, levels, 5);
        let preferred = w.object(0x100, cnull_combat_tactic::VTABLE_RVA);
        // One bucket per leader in a 64-slot table, without collisions.
        let (mask, overflow) = (63_u32, 8_u8);
        let table = w.alloc(0x40);
        let entries = w.alloc((mask as usize + 1 + overflow as usize + 1) * bucket::STRIDE);
        w.u64(registry::HIGH_ROOT_RVA, table as u64);
        w.u64(table + registry::ENTRIES, entries as u64);
        w.u32(table + registry::COUNT, count as u32);
        w.u32(table + registry::MASK, mask);
        w.put(table + registry::OVERFLOW_DISTANCE, &[overflow]);
        w.put(
            entries + (mask as usize + 1 + overflow as usize) * bucket::STRIDE + bucket::DISTANCE,
            &[bucket::TERMINAL_DISTANCE],
        );
        let mut used = Vec::new();
        let mut groups = Vec::new();
        let mut id = 0_u64;
        while groups.len() < count {
            id += 1;
            let handle = (id << 32) | u64::from(cunit_leader::REFERENCE_TYPE);
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(registry::HASH_MULTIPLIER);
            let slot = ((hash >> 32) as u32 ^ hash as u32) & mask;
            if used.contains(&slot) {
                continue;
            }
            used.push(slot);
            let leader = w.object(0x1100, carmy_leader::VTABLE_RVA);
            w.u64(leader + cunit_leader::TYPE, handle);
            w.u64(leader + cunit_leader::SKILL, skill as u64);
            w.u32(leader + cunit_leader::CATEGORY, 2);
            w.u64(leader + cunit_leader::EXPERIENCE, 10 * id);
            w.text(
                leader + cunit_leader::NAME,
                format!("leader {id}").as_bytes(),
            );
            // The raw saved name the detail and division commander read.
            w.text(
                leader + cunit_leader::TOKEN,
                format!("LEADER_{id}").as_bytes(),
            );
            // An empty inline description token.
            w.u64(leader + cunit_leader::DESCRIPTION_TOKEN + 24, 15);
            for offset in [0xc0, 0xe0, 0x100] {
                w.text(leader + offset, b"fallback");
            }
            w.u32(leader + 0x120, country);
            w.u64(
                leader + land_tactic_inputs::LEADER_PREFERRED_TACTIC,
                preferred as u64,
            );
            let cell = entries + slot as usize * bucket::STRIDE;
            w.put(cell + bucket::DISTANCE, &[1]);
            w.u64(cell + 8, handle);
            w.u64(cell + 16, leader as u64);
            let group = w.object(corders_group::SIZE, corders_group::VTABLE_RVA);
            w.u64(
                group + corders_group::HANDLE,
                (id << 32) | u64::from(corders_group::REFERENCE_TYPE),
            );
            w.u64(group + corders_group::LEADER, leader as u64);
            groups.push(group);
        }
        groups
    }

    /// Large enough for every fixture here, so nothing truncates.
    fn roomy() -> Capacities {
        Capacities {
            divisions: 8192,
            details: 1024,
            sides: 2048,
            ..Capacities::default()
        }
    }

    /// Everything a walk publishes: tables, outcomes, and the demand of every
    /// arena and census.
    #[derive(Debug, PartialEq)]
    struct Observed {
        tables: [TableView; N],
        outcomes: [Outcome; N],
        demand: Vec<usize>,
    }

    fn demand(captured: &Captured) -> Vec<usize> {
        vec![
            captured.division_commanders.required,
            captured.rosters.required,
            captured.members.required,
            captured.modifiers.required,
            captured.width_terms.required,
            captured.exile_tags.required,
            captured.division_names.required,
            captured.commander_names.required,
            captured.country_tags.required,
            captured.reinforcement_modifiers.required,
            captured.supply_terms.required,
            captured.acclimatization.required,
            captured.transport_terms.required,
            captured.tactic_unlocks.required,
            captured.tactic_candidates.required,
            captured.tactic_weights.required,
            captured.required_census,
            captured.required_census_participants,
            captured.required_commander_keys,
        ]
    }

    fn observe(captured: &Captured) -> Observed {
        Observed {
            tables: captured.tables(),
            outcomes: captured.outcomes(),
            demand: demand(captured),
        }
    }

    fn pass(captured: &mut Captured, world: &impl Memory) {
        captured.reset();
        captured.capture(world, 0, GAME);
    }

    fn run(world: &impl Memory, c: Capacities) -> Observed {
        let mut captured = Captured::with_capacities(&c);
        pass(&mut captured, world);
        observe(&captured)
    }

    /// One walk as main's readers read: nothing shared (see `Destination`).
    fn run_reference(world: &impl Memory, c: Capacities) -> Observed {
        let mut captured = Captured::with_capacities(&c);
        captured.reference = true;
        pass(&mut captured, world);
        observe(&captured)
    }

    /// The walk and the reference walk publish the same, with and without
    /// the authenticated roster: every table, outcome, failure and demand.
    fn assert_equivalent_with(world: &World, c: Capacities) -> Observed {
        let cache = roster(world);
        let rostered = cache.roster_view(world);
        let one = run(world, c);
        assert_eq!(run_reference(world, c), one, "reference walk");
        let with_roster = run(&rostered, c);
        assert_eq!(
            run_reference(&rostered, c),
            with_roster,
            "reference walk, roster"
        );
        if one
            .outcomes
            .iter()
            .all(|outcome| !matches!(outcome, Outcome::Unavailable { .. }))
        {
            assert_eq!(with_roster, one, "authenticated roster");
        }
        one
    }

    fn assert_equivalent(world: &World) -> Observed {
        assert_equivalent_with(world, roomy())
    }

    /// Every failure is the reference walk's: a read that fails at a chosen
    /// ordinal of the reference walk publishes the tables, outcomes and
    /// demand the reference walk publishes.
    #[test]
    fn walk_fails_as_the_reference_walk() {
        let shape = Shape::small();
        let mut built = world(&shape);
        *built.world.log.lock().unwrap() = Some(Vec::new());
        run_reference(&built.world, roomy());
        let log = built.world.log.lock().unwrap().take().unwrap();
        let mut seen = std::collections::HashSet::new();
        let spans: Vec<_> = log.into_iter().filter(|span| seen.insert(*span)).collect();
        let step = (spans.len() / 150).max(1);
        let mut failed = 0;
        for &span in spans.iter().step_by(step) {
            built.world.poisoned = vec![span];
            let one = assert_equivalent(&built.world);
            failed += usize::from(
                one.outcomes
                    .iter()
                    .any(|outcome| matches!(outcome, Outcome::Unavailable { .. })),
            );
        }
        assert!(failed > 0);
    }

    fn rows(view: &TableView) -> usize {
        match view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
            _ => 0,
        }
    }

    /// Every table complete and identical to the reference walk's, with
    /// shared owners, templates, states, provinces and regions, leaders,
    /// transport divisions, a naval combat the census skips and a tactic
    /// registry.
    #[test]
    fn family_walk_publishes_the_reference_tables() {
        let shape = Shape::small();
        let built = world(&shape);
        let one = assert_equivalent(&built.world);
        for (table, view) in one.tables.iter().enumerate() {
            assert!(
                matches!(view, TableView::Complete(_)),
                "table {table}: {view:?}"
            );
        }
        assert_eq!(rows(&one.tables[0]), shape.combats);
        assert_eq!(rows(&one.tables[1]), shape.combats * shape.divisions);
        assert_eq!(rows(&one.tables[2]), shape.combats * 2);
        let TableView::Complete(replay::rows::TableRows::LandCombatDivisions(divisions)) =
            &one.tables[1]
        else {
            unreachable!()
        };
        let commanded = divisions
            .iter()
            .filter(|row| row.commander_inputs.is_some())
            .count();
        assert_eq!(
            commanded,
            (0..shape.combats * shape.divisions)
                .filter(|index| index % 4 == 1)
                .count()
        );
        assert!(divisions.iter().any(|row| !row.width_terms.is_empty()));
        // Committed divisions on exile templates carry their exile operands;
        // every division carries its generated name.
        assert!(divisions.iter().any(|row| row.exile.is_some()));
        assert!(divisions.iter().any(|row| row.exile.is_none()));
        // Group, custom and generated names all occur.
        assert!(divisions.iter().any(|row| !row.name_group.is_empty()));
        assert!(divisions.iter().any(|row| !row.name_custom.is_empty()));
        assert!(divisions.iter().any(|row| row.name_programmatic != 0));
        let TableView::Complete(replay::rows::TableRows::LandCombatDetails(details)) =
            &one.tables[0]
        else {
            unreachable!()
        };
        assert!(details.iter().all(|row| row.tactic_candidates.len() == 2));
        assert!(
            details
                .iter()
                .any(|row| row.sides.iter().any(|side| side.commander != 0))
        );
        let TableView::Complete(replay::rows::TableRows::LandCombatModifiers(sides)) =
            &one.tables[2]
        else {
            unreachable!()
        };
        assert!(
            sides
                .iter()
                .any(|side| side.divisions.iter().any(|d| !d.entries.is_empty()))
        );
    }

    fn reason(outcome: Outcome) -> &'static str {
        match outcome {
            Outcome::Unavailable { reason } => reason,
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    /// Guarded division failures keep the earliest combat's reason.
    #[test]
    fn earliest_guarded_failure_wins() {
        let shape = Shape::small();
        let stat_block = |built: &mut Built, combat: usize| {
            let army = built.armies[combat][0];
            built.world.poison(army + cunit::STAT_BLOCK, 8);
        };
        let stat_vtable = |built: &mut Built, combat: usize| {
            let army = built.armies[combat][1];
            let stats = built.world.get_u64(army + cunit::STAT_BLOCK) as usize;
            built.world.u64(stats, 0x1234);
        };
        // Each failure alone, to show the reasons differ.
        let mut built = world(&shape);
        stat_block(&mut built, 3);
        let first = reason(assert_equivalent(&built.world).outcomes[1]);
        let mut built = world(&shape);
        stat_vtable(&mut built, 4);
        let second = reason(assert_equivalent(&built.world).outcomes[1]);
        assert_ne!(first, second);
        for (early, late, expected) in [(3, 4, first), (2, 5, second)] {
            let mut built = world(&shape);
            if expected == first {
                stat_block(&mut built, early);
                stat_vtable(&mut built, late);
            } else {
                stat_vtable(&mut built, early);
                stat_block(&mut built, late);
            }
            let observed = assert_equivalent(&built.world);
            assert_eq!(reason(observed.outcomes[1]), expected);
            assert!(matches!(observed.tables[0], TableView::Complete(_)));
        }
    }

    /// A failure that replaces earlier ones (a combat's unit list) keeps the
    /// latest combat's reason, also over an earlier or later guarded failure.
    #[test]
    fn latest_overwriting_failure_wins() {
        let shape = Shape::small();
        let duplicate = |built: &mut Built, combat: usize| {
            let side = built.sides[combat][0];
            let list = built.world.get_u64(side + cland_combatant::COMMITTED) as usize;
            let first = built.world.get_u64(list);
            built.world.u64(list + 8, first);
        };
        let overlap = |built: &mut Built, combat: usize| {
            let side = built.sides[combat][0];
            let list = built.world.get_u64(side + cland_combatant::COMMITTED);
            built.world.u64(side + cland_combatant::RESERVES, list);
        };
        for (duplicated, overlapped, expected) in [
            (1, 4, "land role subsets overlap"),
            (5, 2, "land role subset has duplicate unit"),
            (0, 6, "land role subsets overlap"),
        ] {
            let mut built = world(&shape);
            duplicate(&mut built, duplicated);
            overlap(&mut built, overlapped);
            // A guarded division failure between them never replaces one.
            let army = built.armies[3][0];
            built.world.poison(army + cunit::STAT_BLOCK, 8);
            let observed = assert_equivalent(&built.world);
            assert_eq!(reason(observed.outcomes[1]), expected);
            assert_eq!(reason(observed.outcomes[2]), expected);
        }
    }

    /// A division read failure fails the division table alone and a detail
    /// failure the detail table alone, whichever combat each is in, as the
    /// reference walk publishes them.
    #[test]
    fn division_and_detail_failures_stay_in_their_tables() {
        let shape = Shape::small();
        // The name member is in the division's army image.
        let division = |built: &mut Built, combat: usize| {
            let army = built.armies[combat][0];
            built.world.poison(army + inputs::NAME_GROUP_MEMBER, 8);
        };
        // The stored terrain is read only by detail(): point it at the combat.
        let detail = |built: &mut Built, combat: usize| {
            let object = built.combats[combat];
            built
                .world
                .u64(object + cland_combat::STORED_TERRAIN, object as u64);
        };
        for (divided, detailed) in [(1, 2), (4, 1), (3, 3)] {
            let mut built = world(&shape);
            division(&mut built, divided);
            detail(&mut built, detailed);
            let observed = assert_equivalent(&built.world);
            let [details, divisions] = [0, 1].map(|table| reason(observed.outcomes[table]));
            assert_ne!(details, divisions);
            let mut built = world(&shape);
            detail(&mut built, detailed);
            assert_eq!(
                reason(assert_equivalent(&built.world).outcomes[0]),
                details
            );
        }
    }

    /// A failed tactic registry fails the details at the first combat; a
    /// truncated one reads no details.
    #[test]
    fn tactic_registry_outcome_governs_the_details() {
        let shape = Shape::small();
        let detail = |built: &mut Built, combat: usize| {
            let object = built.combats[combat];
            built.world.poison(object + cland_combat::STORED_TERRAIN, 8);
        };
        let mut built = world(&shape);
        built.world.poison(land_tactic_inputs::DATABASE_RVA, 8);
        detail(&mut built, 1);
        let observed = assert_equivalent(&built.world);
        assert_eq!(reason(observed.outcomes[0]), "tactic input unreadable");
        // Truncated: two tactics, one candidate slot.
        let mut built = world(&shape);
        detail(&mut built, 3);
        let one = assert_equivalent_with(
            &built.world,
            Capacities {
                tactic_candidates: 1,
                ..roomy()
            },
        );
        assert_eq!(
            one.outcomes[0],
            Outcome::Truncated {
                len: 0,
                unread_rows: shape.combats as u32
            }
        );
        assert!(matches!(one.tables[1], TableView::Complete(_)));
    }

    /// A census that fails at a later combat stops the walk there; an
    /// earlier division failure still names the division table.
    #[test]
    fn census_failure_stops_the_walk_at_its_combat() {
        let shape = Shape::small();
        let mut built = world(&shape);
        let army = built.armies[5][0];
        built.world.poison(army + carmy::SECONDARY_BASE, 16);
        let census = reason(assert_equivalent(&built.world).outcomes[0]);
        let army = built.armies[2][0];
        built.world.poison(army + cunit::STAT_BLOCK, 8);
        let observed = assert_equivalent(&built.world);
        assert_eq!(reason(observed.outcomes[0]), census);
        assert_eq!(reason(observed.outcomes[2]), census);
        assert_ne!(reason(observed.outcomes[1]), census);
    }

    /// Detail and side slots too few for the census truncate as the
    /// reference walk does, and publish every row after growth.
    #[test]
    fn truncated_walk_matches_the_reference_and_grows() {
        let shape = Shape {
            combats: 5,
            ..Shape::small()
        };
        let built = world(&shape);
        let mut c = Capacities {
            details: 4,
            sides: 8,
            ..Capacities::default()
        };
        let truncated = assert_equivalent_with(&built.world, c);
        assert_eq!(
            truncated.outcomes[0],
            Outcome::Truncated {
                len: 4,
                unread_rows: 1
            }
        );
        assert_eq!(
            truncated.outcomes[2],
            Outcome::Truncated {
                len: 8,
                unread_rows: 2
            }
        );
        let mut captured = Captured::with_capacities(&c);
        pass(&mut captured, &built.world);
        captured.grow(&mut c);
        pass(&mut captured, &built.world);
        let grown = observe(&captured);
        assert_eq!(grown, run_reference(&built.world, c));
        assert!(
            grown
                .tables
                .iter()
                .all(|view| matches!(view, TableView::Complete(_)))
        );
    }

    /// Every leader a combat's army points to.
    fn army_leaders(built: &Built) -> Vec<usize> {
        let mut leaders = Vec::new();
        for &army in built.armies.iter().flatten() {
            let group = built.world.get_u64(army + carmy::ORDERS_GROUP) as usize;
            if group == 0 {
                continue;
            }
            let leader = built
                .world
                .get_u64(group + engine::corders_group::LEADER) as usize;
            if !leaders.contains(&leader) {
                leaders.push(leader);
            }
        }
        leaders
    }

    fn detail_rows(view: &TableView) -> &[LandCombatDetailsRow] {
        match view {
            TableView::Complete(replay::rows::TableRows::LandCombatDetails(rows)) => rows,
            other => panic!("expected complete details, got {other:?}"),
        }
    }

    /// 140C16FB0 stores a selected commander's +0x20 name as an ordinary MSVC
    /// string and 142230F20 takes any length, empty included; +0x80 is the
    /// same kind of string. A long name omits its combat's detail (and its
    /// divisions' commander) until the shared text arena grows, then both
    /// publish it whole, as the reference walk does.
    #[test]
    fn selected_commander_texts_keep_native_length_and_grow() {
        use engine::cunit_leader::{DESCRIPTION_TOKEN, TOKEN};
        let shape = Shape::small();
        let mut built = world(&shape);
        let leaders = army_leaders(&built);
        assert!(!leaders.is_empty());
        let long = vec![b'k'; 3000];
        let heap = built.world.alloc(long.len() + 1);
        built.world.put(heap, &long);
        let empty = built.world.alloc(32);
        for &leader in &leaders {
            built.world.u64(leader + TOKEN, heap as u64);
            built.world.u64(leader + TOKEN + 16, long.len() as u64);
            built.world.u64(leader + TOKEN + 24, long.len() as u64);
            // An empty heap description.
            built.world.u64(leader + DESCRIPTION_TOKEN, empty as u64);
            built.world.u64(leader + DESCRIPTION_TOKEN + 16, 0);
            built.world.u64(leader + DESCRIPTION_TOKEN + 24, 31);
        }
        let mut c = Capacities {
            commander_names: 64,
            ..roomy()
        };
        let truncated = assert_equivalent_with(&built.world, c);
        assert!(
            matches!(truncated.outcomes[0], Outcome::Truncated { .. }),
            "{:?}",
            truncated.outcomes[0]
        );
        let mut captured = Captured::with_capacities(&c);
        pass(&mut captured, &built.world);
        captured.grow(&mut c);
        pass(&mut captured, &built.world);
        let grown = observe(&captured);
        assert_eq!(grown, run_reference(&built.world, c));
        assert!(
            grown
                .tables
                .iter()
                .all(|view| matches!(view, TableView::Complete(_)))
        );
        let commanded: Vec<_> = detail_rows(&grown.tables[0])
            .iter()
            .flat_map(|row| row.sides.iter())
            .filter(|side| side.commander != 0)
            .collect();
        assert!(!commanded.is_empty());
        assert!(commanded.iter().all(|side| side.commander_name_token == long
            && side.commander_description_token.is_empty()));

        // An empty name is a valid text, not a failure.
        for &leader in &leaders {
            built.world.u64(leader + TOKEN, 0);
            built.world.text(leader + TOKEN, b"");
        }
        let one = assert_equivalent(&built.world);
        let commanded: Vec<_> = detail_rows(&one.tables[0])
            .iter()
            .flat_map(|row| row.sides.iter())
            .filter(|side| side.commander != 0)
            .collect();
        assert!(!commanded.is_empty());
        assert!(
            commanded
                .iter()
                .all(|side| side.commander_name_token.is_empty())
        );
    }

    /// The live rate on the run3 save: 44,741 guarded calls in 0.96 ms.
    const LIVE_NS_PER_CALL: f64 = 21.5;

    /// Guarded calls per pass for the whole land combat family (census,
    /// details, divisions and modifiers) over the authenticated roster, as
    /// the Ordinary job reads, at 153, 1,000, 3,000, 5,000, 10,000 and 50,000 divisions in
    /// combat: the reference walk (main's reads, nothing shared) and the
    /// walk. A gather (`copy_records`) or scatter is one guarded call, as
    /// live. The estimate uses run3's 21.5 ns per guarded call; the
    /// fixture's own time is its slow sparse memory, not the game's. Run in
    /// release with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn family_cost_at_scale() {
        const PASSES: usize = 5;
        let live = |calls: usize| calls as f64 * LIVE_NS_PER_CALL / 1e6;
        println!(
            "| divisions | combats | reference calls | walk calls | walk live ms | calls per division | fixture ms |"
        );
        println!("|---|---|---|---|---|---|---|");
        let mut attribution = Vec::new();
        let mut kinds = Vec::new();
        let mut by_stage = Vec::new();
        for divisions in [153, 1_000, 3_000, 5_000, 10_000, 50_000] {
            let shape = Shape::scale(divisions);
            let built = world(&shape);
            let w = &built.world;
            let cache = roster(w);
            let view = &cache.roster_view(w);
            let mut capacities = Capacities::default();
            let mut walk = Captured::with_capacities(&capacities);
            // Grow to the census and warm storage, as a running recorder has.
            for _ in 0..3 {
                pass(&mut walk, view);
                walk.grow(&mut capacities);
            }
            pass(&mut walk, view);
            let published = observe(&walk);
            assert!(
                published
                    .tables
                    .iter()
                    .all(|view| matches!(view, TableView::Complete(_)))
            );
            let mut reference = Captured::with_capacities(&capacities);
            reference.reference = true;
            pass(&mut reference, view);
            w.calls.store(0, Ordering::Relaxed);
            pass(&mut reference, view);
            let reference_calls = w.calls.swap(0, Ordering::Relaxed);
            assert_eq!(observe(&reference), published);
            let (mut fixture_ms, mut calls) = (Vec::new(), 0);
            for _ in 0..PASSES {
                walk.reset();
                w.calls.store(0, Ordering::Relaxed);
                let started = std::time::Instant::now();
                walk.capture(view, 0, GAME);
                fixture_ms.push(started.elapsed().as_secs_f64() * 1e3);
                calls = w.calls.swap(0, Ordering::Relaxed);
            }
            fixture_ms.sort_by(f64::total_cmp);
            assert_eq!(observe(&walk), published);
            let tables = per_table(w, &cache, |memory| {
                walk.reset();
                walk.capture(memory, 0, GAME);
            });
            kinds.push((divisions, calls_by_object(&built, view, &mut walk)));
            let total = shape.combats * shape.divisions;
            let stages = stages(w, view, tables);
            assert_eq!(
                stages.iter().map(|(_, calls)| calls).sum::<u64>(),
                calls as u64,
                "stages cover the walk"
            );
            by_stage.push((total, stages));
            attribution.push(format!(
                "| {total} | {} | {} | {} |",
                tables[0], tables[1], tables[2]
            ));
            println!(
                "| {total} | {} | {reference_calls} | {calls} | {:.2} | {:.1} | {:.2} |",
                shape.combats,
                live(calls),
                calls as f64 / total as f64,
                fixture_ms[PASSES / 2],
            );
        }
        println!();
        println!("Guarded calls by table, as the recorder attributes them:");
        println!("| divisions | details | divisions table | modifiers |");
        println!("|---|---|---|---|");
        for row in attribution {
            println!("{row}");
        }
        println!();
        println!("Guarded calls by stage (live ms at 21.5 ns per call):");
        for (divisions, stages) in by_stage {
            let row: Vec<String> = stages
                .iter()
                .map(|(stage, calls)| {
                    format!("{stage} {calls} ({:.2} ms)", live(*calls as usize))
                })
                .collect();
            println!("{divisions} divisions: {}", row.join("; "));
        }
        println!();
        println!("The walk's guarded calls by the object each call reads first:");
        for (divisions, kinds) in kinds {
            println!("{divisions} divisions: {kinds:?}");
        }
    }

    /// One walk's guarded calls by stage over `memory` (the roster view of
    /// `w`). Each stage runs in the walk's order with the walk's shares and
    /// counts as the difference from the stages before it; the division
    /// readers (with their commanders) and the modifier members are what the
    /// divisions and modifiers tables (`tables`) count beyond those stages.
    fn stages(w: &World, memory: &impl Memory, tables: [u64; N]) -> Vec<(&'static str, u64)> {
        use engine::land_tactic_inputs as tactic;
        let through = |stage: u8| {
            let mut census = vec![cland_combat::Row::default(); 4096];
            let mut participants = vec![0; 1 << 17];
            let mut pointers = vec![0; 1 << 17];
            let mut shares = cland_combat::CombatScratch::default();
            let mut images = engine::combat_sources::Images::default();
            let mut rosters = vec![0; cland_combat::DETAIL_UNIT_LIMIT as usize * 4];
            let mut candidates = vec![tactic::Candidate::default(); 1024];
            let mut weights = vec![tactic::WeightTerm::default(); 4096];
            let mut unlocks = vec![tactic::Unlock::default(); 1 << 16];
            let mut text = vec![0; 1 << 22];
            let mut texts = engine::cname_group_member::Names {
                bytes: &mut text,
                len: 0,
                required: 0,
            };
            w.calls.store(0, Ordering::Relaxed);
            if stage >= 1 {
                tactic::capture_registry(memory, 0, &mut candidates, &mut weights);
            }
            let walk = cland_combat::walk_members(
                memory,
                0,
                GAME,
                &mut census,
                &mut participants,
                &mut pointers,
                |source, members| {
                    let mut commanders = cland_combat::Commanders::default();
                    if stage >= 1 {
                        cland_combat::read_details(
                            memory,
                            0,
                            source,
                            &members,
                            &mut commanders,
                            &mut rosters,
                            &mut texts,
                        )
                        .unwrap();
                        tactic::capture_context(
                            memory,
                            0,
                            GAME,
                            source,
                            &members,
                            &mut commanders,
                            &mut unlocks,
                        );
                    }
                    if stage >= 2 {
                        cland_combat::read_modifiers_context(memory, 0, source).unwrap();
                    }
                    if stage >= 3 {
                        images.capture(memory, members.pointers);
                    }
                    if stage >= 4 {
                        cland_combat::visit_units(
                            memory,
                            0,
                            source,
                            &members,
                            &mut shares,
                            |_, _, _, _, _, _| {},
                        )
                        .unwrap();
                    }
                },
            );
            assert!(matches!(walk.outcome, Outcome::Rows { .. }));
            w.calls.swap(0, Ordering::Relaxed) as u64
        };
        let counts: Vec<u64> = (0..=4).map(through).collect();
        let stage = |index: usize| counts[index] - counts[index - 1];
        vec![
            ("census", counts[0]),
            ("details and tactic context", stage(1)),
            ("unit visit", stage(4)),
            ("division images", stage(3)),
            ("division readers and commanders", tables[1] - stage(3)),
            ("modifier context", stage(2)),
            ("modifier members", tables[2] - stage(2)),
            ("country sets", tables[3]),
        ]
    }

    /// One walk's guarded calls by the object each call reads first: the
    /// divisions' own armies and stat blocks, the combats and their sides,
    /// module and game-state words, and every other object (countries,
    /// provinces, states, templates, leaders, participant lists).
    fn calls_by_object(
        built: &Built,
        memory: &impl Memory,
        captured: &mut Captured,
    ) -> Vec<(&'static str, usize)> {
        let w = &built.world;
        let armies: Vec<usize> = built.armies.iter().flatten().copied().collect();
        let mut stats: Vec<usize> = armies
            .iter()
            .map(|&army| w.get_u64(army + cunit::STAT_BLOCK) as usize)
            .collect();
        stats.sort_unstable();
        let mut sorted = armies.clone();
        sorted.sort_unstable();
        let mut sides: Vec<usize> = built.sides.iter().flatten().copied().collect();
        sides.sort_unstable();
        let mut combats = built.combats.clone();
        combats.sort_unstable();
        *w.call_log.lock().unwrap() = Some(Vec::new());
        captured.reset();
        captured.capture(memory, 0, GAME);
        let log = w.call_log.lock().unwrap().take().unwrap();
        let within = |objects: &[usize], size: usize, address: usize| {
            let at = objects.partition_point(|&start| start <= address);
            at > 0 && address < objects[at - 1] + size
        };
        let mut counts = [
            ("division army", 0),
            ("division stat block", 0),
            ("combat", 0),
            ("combat side", 0),
            ("module or game state", 0),
            ("other", 0),
        ];
        for address in log {
            let kind = if within(&sorted, 0x800, address) {
                0
            } else if within(&stats, 0x600, address) {
                1
            } else if within(&combats, 0x600, address) {
                2
            } else if within(&sides, 0x600, address) {
                3
            } else if address < 0x1_0000_0000 {
                4
            } else {
                5
            };
            counts[kind].1 += 1;
        }
        counts.to_vec()
    }
}
