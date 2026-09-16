//! Hourly combat-window operands. One bounded census feeds three independent
//! tables; projection and capacity growth happen only on the spool thread.
use crate::{
    sampling::{full_match, qpc},
    spooling::table,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::land_tactic_inputs;
use engine::{
    Memory, carmy_combat_inputs, cland_combat, land_reinforcement_inputs as reinforcement,
    walk::Outcome,
};
use replay::rows::LandDivisionManpowerTerm;
use replay::rows::{LandCombatCommanderPortraitModifier, LandCombatCommanderPortraitTooltip};
use replay::rows::{
    LandCombatTacticCandidate, LandCombatTacticCountryWeight, LandCombatTacticInputs,
    LandCombatTacticUnlock, LandCombatTacticWeightTerm,
};
use replay::{
    frame::TableView,
    rows::{
        CombatModifierValue, DivisionCombatModifiers, LandCombatCommanderLearningTrait,
        LandCombatCommanderTrait, LandCombatDetailsRow, LandCombatDivisionRow,
        LandCombatModifierRow, LandCombatSideDetails, LandDivisionContext, LandDivisionModifier,
        LandDivisionWidthTerm, LandReinforcementAcclimatization, LandReinforcementInputs,
        LandReinforcementModifier, LandReinforcementSupplyTerm, LandReinforcementTransportTerm,
        TableId, tables,
    },
};
use std::mem::size_of;

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::LandCombatDetails,
    TableId::LandCombatDivisions,
    TableId::LandCombatModifiers,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "land combat detail census and member authentication",
    0,
    &[0, 1, 2],
)];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    combats: usize,
    census_participants: usize,
    details: usize,
    divisions: usize,
    sides: usize,
    rosters: usize,
    modifier_members: usize,
    modifiers: usize,
    width_terms: usize,
    manpower: usize,
    reinforcement_modifiers: usize,
    supply_terms: usize,
    acclimatization: usize,
    transport_terms: usize,
    tactic_candidates: usize,
    tactic_weights: usize,
    tactic_unlocks: usize,
    division_commanders: usize,
    commander_keys: usize,
    window_divisions: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            combats: 512,
            census_participants: 8192,
            details: 64,
            divisions: 1024,
            sides: 128,
            rosters: 2048,
            modifier_members: 1024,
            modifiers: 16384,
            width_terms: 8192,
            manpower: 8192,
            reinforcement_modifiers: 16384,
            supply_terms: 4096,
            acclimatization: 2048,
            transport_terms: 1024,
            tactic_candidates: 256,
            tactic_weights: 1024,
            tactic_unlocks: 4096,
            division_commanders: 32,
            commander_keys: 2048,
            window_divisions: 2048,
        }
    }
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Detail {
    row: cland_combat::DetailsRow,
    tactic_context: land_tactic_inputs::ContextRow,
    hq_unit: [u64; 2],
    window_start: usize,
    window_len: usize,
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Division {
    combat: u64,
    side: u8,
    ordinal: u32,
    province: u32,
    role: u8,
    stats: carmy_combat_inputs::Row,
    direction: u32,
    direction_group: i32,
    template_frames: u8,
    commander_input: Option<usize>,
    context: carmy_combat_inputs::ContextRow,
    width_start: usize,
    manpower_start: usize,
    manpower_len: usize,
    reinforcement: reinforcement::ContextRow,
    reinforcement_starts: [usize; 4],
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct ModifierSide {
    combat: u64,
    side: u8,
    flags: u32,
    indicator: u8,
    border_values: Option<[i64; 3]>,
    start: usize,
    len: usize,
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
struct Destination<'a> {
    division_commanders: &'a mut [engine::naval_commander_inputs::DivisionCommanderRow],
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
    divisions: &'a mut [Division],
    sides: &'a mut [ModifierSide],
    rosters: &'a mut [u64],
    members: &'a mut [ModifierMember],
    modifiers: &'a mut [carmy_combat_inputs::Modifier],
    width_terms: &'a mut [carmy_combat_inputs::WidthTerm],
    width_progress: Progress,
    manpower: &'a mut [carmy_combat_inputs::ManpowerTerm],
    manpower_progress: Progress,
    reinforcement_modifiers: &'a mut [reinforcement::ModifierTerm],
    supply_terms: &'a mut [reinforcement::SupplyTerm],
    acclimatization: &'a mut [reinforcement::AcclimatizationTerm],
    transport_terms: &'a mut [reinforcement::TransportTerm],
    reinforcement_progress: [Progress; 4],
    progress: [Progress; N],
    arenas: [Progress; N],
    active: [bool; N],
    ticks: [u64; N],
    window: &'a mut [cland_combat::WindowDivision],
    window_progress: Progress,
    current_window: Option<(u64, usize, usize, [u64; 2])>,
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
            if self.commander_progress.len == self.division_commanders.len() {
                return None;
            }
            match engine::naval_commander_inputs::capture_division_commander(memory, base, handle) {
                Ok(row) => {
                    let index = self.commander_progress.len;
                    self.division_commanders[index] = row;
                    self.commander_progress.len += 1;
                    self.commander_keys[slot].index = index;
                    return Some(index);
                }
                Err(reason) => {
                    self.progress[1].failure = Some(reason);
                    return None;
                }
            }
        }
        self.required_commander_keys = self.commander_keys.len().saturating_add(1);
        None
    }
    fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        source: cland_combat::Source,
    ) {
        self.current_window = None;
        if self.active[0] || self.active[1] {
            let started = qpc();
            // Upper bound: one operand row per full participant. Bumped before
            // the walk so a capacity skip still grows the arena off-thread.
            let bound = (source.attackers.count as usize)
                .saturating_add(source.defenders.count as usize);
            self.window_progress.required += bound;
            let free = self
                .window
                .len()
                .saturating_sub(self.window_progress.len);
            if bound <= free {
                let start = self.window_progress.len;
                match cland_combat::read_window_operands(
                    memory,
                    base,
                    state,
                    source,
                    &mut self.window[start..],
                ) {
                    Ok(operands) => {
                        let len = operands.divisions_len;
                        self.window_progress.len += len;
                        self.current_window = Some((source.id, start, len, operands.hq_unit));
                    }
                    Err(reason) => {
                        // Fail the consuming tables closed: a missing HQ unit
                        // would invent a hidden badge, missing operands icons.
                        if self.active[0] && self.progress[0].failure.is_none() {
                            self.progress[0].failure = Some(reason);
                        }
                        if self.active[1] && self.progress[1].failure.is_none() {
                            self.progress[1].failure = Some(reason);
                        }
                    }
                }
            } else {
                // Capacity grows off-thread; skip this combat rather than
                // failing the hour or inventing operands. The bound above
                // already raised the growth requirement.
                if self.active[1] {
                    self.ticks[1] += elapsed(started);
                } else {
                    self.ticks[0] += elapsed(started);
                }
                return;
            }
            if self.active[1] {
                self.ticks[1] += elapsed(started);
            } else {
                self.ticks[0] += elapsed(started);
            }
        }
        if self.active[0] {
            let started = qpc();
            self.detail(memory, base, state, source);
            self.ticks[0] += elapsed(started);
        }
        let mut side_indices = [None; 2];
        if self.active[2] {
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
                                flags: context.side_flags[side],
                                indicator: context.indicator,
                                border_values: context.border_values[side],
                                start: self.arenas[1].len,
                                len: 0,
                            };
                            *index = Some(next);
                            self.progress[2].len += 1;
                        }
                    }
                }
                Err(reason) => self.progress[2].failure = Some(reason),
            }
            self.ticks[2] += elapsed(started);
        }
        if self.active[1] || self.active[2] {
            let started = qpc();
            let before = self.ticks[1] + self.ticks[2];
            let result = cland_combat::visit_units(
                memory,
                base,
                source,
                |side, ordinal, pointer, handle, province, role| {
                    if self.active[1] {
                        let started = qpc();
                        self.progress[1].required += 1;
                        // The window walk in visit() covers every full
                        // participant, so every visited handle joins. A miss
                        // fails the table closed rather than zero-filling
                        // operands the viewer would draw as absent icons.
                        let operand = match self.current_window {
                            Some((combat, start, len, _)) if combat == source.id => self
                                .window[start..start + len]
                                .iter()
                                .find(|row| row.handle == handle)
                                .copied(),
                            _ => None,
                        };
                        if operand.is_none() && self.progress[1].failure.is_none() {
                            self.progress[1].failure =
                                Some("land window operands missing");
                        }
                        if self.progress[1].failure.is_none()
                            && self.progress[1].len < self.divisions.len()
                            && let Some(operand) = operand
                        {
                            let mut terms = [carmy_combat_inputs::WidthTerm::default();
                                carmy_combat_inputs::WIDTH_TERM_LIMIT];
                            let mut manpower = [carmy_combat_inputs::ManpowerTerm::default();
                                carmy_combat_inputs::MANPOWER_TERM_LIMIT];
                            let mut reinforcement_modifiers =
                                [reinforcement::ModifierTerm::default();
                                    reinforcement::MODIFIER_LIMIT];
                            let mut supply_terms = [reinforcement::SupplyTerm::default();
                                reinforcement::SUPPLY_TERM_LIMIT];
                            let mut acclimatization =
                                [reinforcement::AcclimatizationTerm::default();
                                    reinforcement::ACCLIMATIZATION_LIMIT];
                            let mut transport_terms = [reinforcement::TransportTerm::default();
                                reinforcement::TRANSPORT_TERM_LIMIT];
                            let result =
                                carmy_combat_inputs::capture_stats(memory, base, pointer, handle)
                                    .and_then(|stats| {
                                        carmy_combat_inputs::capture_context(
                                            memory, base, state, pointer, handle, &mut terms,
                                        )
                                        .and_then(
                                            |context| {
                                                reinforcement::capture_context(
                                                    memory,
                                                    base,
                                                    state,
                                                    pointer,
                                                    handle,
                                                    &mut reinforcement_modifiers,
                                                    &mut supply_terms,
                                                    &mut acclimatization,
                                                    &mut transport_terms,
                                                )
                                                .and_then(|reinforcement| {
                                                    carmy_combat_inputs::capture_manpower(
                                                        memory,
                                                        base,
                                                        pointer,
                                                        handle,
                                                        &mut manpower,
                                                    )
                                                    .map(|count| {
                                                        (stats, context, reinforcement, count)
                                                    })
                                                })
                                            },
                                        )
                                    });
                            match result {
                                Ok((stats, context, reinforcement, manpower_len)) => {
                                    let commander_input =
                                        self.commander(memory, base, stats.commander);
                                    self.manpower_progress.required += manpower_len;
                                    let manpower_start = self.manpower_progress.len;
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
                                    let capacities = [
                                        self.reinforcement_modifiers.len(),
                                        self.supply_terms.len(),
                                        self.acclimatization.len(),
                                        self.transport_terms.len(),
                                    ];
                                    for (p, count) in
                                        self.reinforcement_progress.iter_mut().zip(counts)
                                    {
                                        p.required += count;
                                    }
                                    if count <= self.width_terms.len().saturating_sub(start)
                                        && (stats.commander == 0 || commander_input.is_some())
                                        && manpower_len
                                            <= self.manpower.len().saturating_sub(manpower_start)
                                        && (0..4).all(|i| {
                                            counts[i] <= capacities[i].saturating_sub(starts[i])
                                        })
                                    {
                                        self.reinforcement_modifiers
                                            [starts[0]..starts[0] + counts[0]]
                                            .copy_from_slice(&reinforcement_modifiers[..counts[0]]);
                                        self.supply_terms[starts[1]..starts[1] + counts[1]]
                                            .copy_from_slice(&supply_terms[..counts[1]]);
                                        self.acclimatization[starts[2]..starts[2] + counts[2]]
                                            .copy_from_slice(&acclimatization[..counts[2]]);
                                        self.transport_terms[starts[3]..starts[3] + counts[3]]
                                            .copy_from_slice(&transport_terms[..counts[3]]);
                                        for (p, count) in
                                            self.reinforcement_progress.iter_mut().zip(counts)
                                        {
                                            p.len += count;
                                        }
                                        self.width_terms[start..start + count]
                                            .copy_from_slice(&terms[..count]);
                                        self.width_progress.len += count;
                                        self.manpower
                                            [manpower_start..manpower_start + manpower_len]
                                            .copy_from_slice(&manpower[..manpower_len]);
                                        self.manpower_progress.len += manpower_len;
                                        self.divisions[self.progress[1].len] = Division {
                                            combat: source.id,
                                            side,
                                            ordinal,
                                            province,
                                            role,
                                            stats,
                                            direction: operand.direction,
                                            direction_group: operand.direction_group,
                                            template_frames: operand.template_frames,
                                            commander_input,
                                            context,
                                            width_start: start,
                                            manpower_start,
                                            manpower_len,
                                            reinforcement,
                                            reinforcement_starts: starts,
                                        };
                                        self.progress[1].len += 1;
                                    }
                                }
                                Err(reason) => self.progress[1].failure = Some(reason),
                            }
                        }
                        self.ticks[1] += elapsed(started);
                    }
                    if self.active[2] {
                        let started = qpc();
                        self.modifier_member(
                            memory,
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
                    if self.active[index] {
                        self.progress[index].failure = Some(reason);
                    }
                }
            }
        }
    }
    fn detail(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        source: cland_combat::Source,
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
        // Both committed/reserve lists on each side have independent 1024 bounds.
        let mut scratch = [0_u64; cland_combat::DETAIL_UNIT_LIMIT as usize * 4];
        match cland_combat::read_details(memory, base, source, &mut scratch) {
            Ok((mut row, count)) => {
                self.arenas[0].required += count;
                let mut context = land_tactic_inputs::capture_context(
                    memory,
                    base,
                    state,
                    source,
                    &mut self.tactic_unlocks[self.tactic_unlock_progress.len..],
                );
                self.tactic_unlock_progress.required += context.row.required_unlocks;
                match context.outcome {
                    Outcome::Unavailable { reason } => {
                        self.progress[0].failure = Some(reason);
                        return;
                    }
                    Outcome::Truncated { .. } => return,
                    Outcome::Rows { .. } => {}
                }
                if self.progress[0].len == self.details.len()
                    || count > self.rosters.len().saturating_sub(self.arenas[0].len)
                {
                    return;
                }
                // The window walk ran before this detail in visit(); without
                // its HQ row the badge would be invented hidden. Fail closed.
                let Some((window_combat, window_start, window_len, hq_unit)) =
                    self.current_window
                else {
                    self.progress[0].failure = Some("land window operands missing");
                    return;
                };
                if window_combat != source.id {
                    self.progress[0].failure = Some("land window operands mismatch");
                    return;
                }
                let start = self.arenas[0].len;
                self.rosters[start..start + count].copy_from_slice(&scratch[..count]);
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
                    hq_unit,
                    window_start,
                    window_len,
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
        let mut scratch =
            [carmy_combat_inputs::Modifier::default(); carmy_combat_inputs::MODIFIER_LIMIT];
        match carmy_combat_inputs::capture_modifiers(memory, base, pointer, handle, &mut scratch) {
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
                self.modifiers[start..start + count].copy_from_slice(&scratch[..count]);
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
    commander_keys: Vec<CommanderKey>,
    required_commander_keys: usize,
    details: CaptureRows<Detail>,
    tactic_candidates: Arena<land_tactic_inputs::Candidate>,
    tactic_weights: Arena<land_tactic_inputs::WeightTerm>,
    tactic_unlocks: Arena<land_tactic_inputs::Unlock>,
    divisions: CaptureRows<Division>,
    sides: CaptureRows<ModifierSide>,
    rosters: Arena<u64>,
    members: Arena<ModifierMember>,
    modifiers: Arena<carmy_combat_inputs::Modifier>,
    width_terms: Arena<carmy_combat_inputs::WidthTerm>,
    manpower: Arena<carmy_combat_inputs::ManpowerTerm>,
    reinforcement_modifiers: Arena<reinforcement::ModifierTerm>,
    supply_terms: Arena<reinforcement::SupplyTerm>,
    acclimatization: Arena<reinforcement::AcclimatizationTerm>,
    transport_terms: Arena<reinforcement::TransportTerm>,
    window: Arena<cland_combat::WindowDivision>,
    census: Vec<cland_combat::Row>,
    census_participants: Vec<u64>,
    required_census: usize,
    required_census_participants: usize,
}
impl Captured {
    pub fn with_capacities(c: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            division_commanders: Arena::new(c.division_commanders, successful[1]),
            commander_keys: vec![CommanderKey::default(); c.commander_keys],
            required_commander_keys: 0,
            details: CaptureRows::new(c.details, successful[0]),
            tactic_candidates: Arena::new(c.tactic_candidates, successful[0]),
            tactic_weights: Arena::new(c.tactic_weights, successful[0]),
            tactic_unlocks: Arena::new(c.tactic_unlocks, successful[0]),
            divisions: CaptureRows::new(c.divisions, successful[1]),
            sides: CaptureRows::new(c.sides, successful[2]),
            rosters: Arena::new(c.rosters, successful[0]),
            members: Arena::new(c.modifier_members, successful[2]),
            modifiers: Arena::new(c.modifiers, successful[2]),
            width_terms: Arena::new(c.width_terms, successful[1]),
            manpower: Arena::new(c.manpower, successful[1]),
            reinforcement_modifiers: Arena::new(c.reinforcement_modifiers, successful[1]),
            supply_terms: Arena::new(c.supply_terms, successful[1]),
            acclimatization: Arena::new(c.acclimatization, successful[1]),
            transport_terms: Arena::new(c.transport_terms, successful[1]),
            window: Arena::new(c.window_divisions, successful[1]),
            census: vec![cland_combat::Row::default(); c.combats],
            census_participants: vec![0; c.census_participants],
            required_census: 0,
            required_census_participants: 0,
        }
    }
    pub fn reset(&mut self) {
        self.division_commanders.reset();
        self.required_commander_keys = 0;
        self.details.reset();
        self.tactic_candidates.reset();
        self.tactic_weights.reset();
        self.tactic_unlocks.reset();
        self.divisions.reset();
        self.sides.reset();
        self.rosters.reset();
        self.members.reset();
        self.modifiers.reset();
        self.width_terms.reset();
        self.manpower.reset();
        self.reinforcement_modifiers.reset();
        self.supply_terms.reset();
        self.acclimatization.reset();
        self.transport_terms.reset();
        self.window.reset();
        self.required_census = 0;
        self.required_census_participants = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.details.allocated_bytes()
            + self.division_commanders.allocated_bytes()
            + self.commander_keys.capacity() * size_of::<CommanderKey>()
            + self.tactic_candidates.allocated_bytes()
            + self.tactic_weights.allocated_bytes()
            + self.tactic_unlocks.allocated_bytes()
            + self.divisions.allocated_bytes()
            + self.sides.allocated_bytes()
            + self.rosters.allocated_bytes()
            + self.members.allocated_bytes()
            + self.modifiers.allocated_bytes()
            + self.width_terms.allocated_bytes()
            + self.manpower.allocated_bytes()
            + self.reinforcement_modifiers.allocated_bytes()
            + self.supply_terms.allocated_bytes()
            + self.acclimatization.allocated_bytes()
            + self.transport_terms.allocated_bytes()
            + self.window.allocated_bytes()
            + self.census.capacity() * size_of::<cland_combat::Row>()
            + self.census_participants.capacity() * size_of::<u64>()
    }
    pub fn grow(&mut self, c: &mut Capacities, successful: &[u16]) {
        self.division_commanders
            .grow(&mut c.division_commanders, successful[1]);
        grow_capacity(&mut c.commander_keys, self.required_commander_keys);
        grow_vec(&mut self.commander_keys, c.commander_keys);
        self.details.grow(&mut c.details, successful[0]);
        self.tactic_candidates
            .grow(&mut c.tactic_candidates, successful[0]);
        self.tactic_weights
            .grow(&mut c.tactic_weights, successful[0]);
        self.tactic_unlocks
            .grow(&mut c.tactic_unlocks, successful[0]);
        self.divisions.grow(&mut c.divisions, successful[1]);
        self.sides.grow(&mut c.sides, successful[2]);
        self.rosters.grow(&mut c.rosters, successful[0]);
        self.members.grow(&mut c.modifier_members, successful[2]);
        self.modifiers.grow(&mut c.modifiers, successful[2]);
        self.width_terms.grow(&mut c.width_terms, successful[1]);
        self.manpower.grow(&mut c.manpower, successful[1]);
        self.reinforcement_modifiers
            .grow(&mut c.reinforcement_modifiers, successful[1]);
        self.supply_terms.grow(&mut c.supply_terms, successful[1]);
        self.acclimatization
            .grow(&mut c.acclimatization, successful[1]);
        self.transport_terms
            .grow(&mut c.transport_terms, successful[1]);
        self.window.grow(&mut c.window_divisions, successful[1]);
        grow_capacity(&mut c.combats, self.required_census);
        grow_capacity(
            &mut c.census_participants,
            self.required_census_participants,
        );
        grow_vec(&mut self.census, c.combats);
        grow_vec(&mut self.census_participants, c.census_participants);
    }
    fn pass(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        second: bool,
        active: [bool; N],
    ) -> ([Outcome; N], [u64; N]) {
        let started = qpc();
        let commander_started = qpc();
        if active[1] {
            self.commander_keys.fill(CommanderKey::default());
        }
        let commander_ticks = if active[1] {
            elapsed(commander_started)
        } else {
            0
        };
        let mut destination = Destination {
            division_commanders: if second {
                &mut self.division_commanders.second
            } else {
                &mut self.division_commanders.first
            },
            commander_progress: Progress::default(),
            commander_keys: &mut self.commander_keys,
            commander_key_count: 0,
            required_commander_keys: 0,
            tactic_candidates: if second {
                &mut self.tactic_candidates.second
            } else {
                &mut self.tactic_candidates.first
            },
            tactic_weights: if second {
                &mut self.tactic_weights.second
            } else {
                &mut self.tactic_weights.first
            },
            tactic_unlocks: if second {
                &mut self.tactic_unlocks.second
            } else {
                &mut self.tactic_unlocks.first
            },
            tactic_registry: None,
            tactic_unlock_progress: Progress::default(),
            details: if second {
                &mut self.details.second
            } else {
                &mut self.details.rows
            },
            divisions: if second {
                &mut self.divisions.second
            } else {
                &mut self.divisions.rows
            },
            sides: if second {
                &mut self.sides.second
            } else {
                &mut self.sides.rows
            },
            rosters: if second {
                &mut self.rosters.second
            } else {
                &mut self.rosters.first
            },
            members: if second {
                &mut self.members.second
            } else {
                &mut self.members.first
            },
            modifiers: if second {
                &mut self.modifiers.second
            } else {
                &mut self.modifiers.first
            },
            width_terms: if second {
                &mut self.width_terms.second
            } else {
                &mut self.width_terms.first
            },
            width_progress: Progress::default(),
            manpower: if second {
                &mut self.manpower.second
            } else {
                &mut self.manpower.first
            },
            manpower_progress: Progress::default(),
            reinforcement_modifiers: if second {
                &mut self.reinforcement_modifiers.second
            } else {
                &mut self.reinforcement_modifiers.first
            },
            supply_terms: if second {
                &mut self.supply_terms.second
            } else {
                &mut self.supply_terms.first
            },
            acclimatization: if second {
                &mut self.acclimatization.second
            } else {
                &mut self.acclimatization.first
            },
            transport_terms: if second {
                &mut self.transport_terms.second
            } else {
                &mut self.transport_terms.first
            },
            reinforcement_progress: [Progress::default(); 4],
            progress: [Progress::default(); N],
            arenas: [Progress::default(); N],
            active,
            ticks: [0, commander_ticks, 0],
            window: if second {
                &mut self.window.second
            } else {
                &mut self.window.first
            },
            window_progress: Progress::default(),
            current_window: None,
        };
        let census = cland_combat::walk_with(
            memory,
            base,
            state,
            &mut self.census,
            &mut self.census_participants,
            |source| destination.visit(memory, base, state, source),
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
        let window = destination.window_progress;
        let manpower = destination.manpower_progress;
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
        if second {
            if active[0] {
                let comparison_started = qpc();
                self.details.mismatches +=
                    usize::from(!self.rosters.compare(arenas[0].len, arenas[0].required));
                self.details.mismatches += usize::from(!self.tactic_candidates.compare(
                    tactic_registry.candidate_count,
                    tactic_registry.required_candidates,
                ));
                self.details.mismatches += usize::from(!self.tactic_weights.compare(
                    tactic_registry.weight_count,
                    tactic_registry.required_weights,
                ));
                self.details.mismatches += usize::from(
                    !self
                        .tactic_unlocks
                        .compare(tactic_unlocks.len, tactic_unlocks.required),
                );
                ticks[0] += elapsed(comparison_started);
            }
            if active[1] {
                let comparison_started = qpc();
                self.divisions.mismatches += usize::from(
                    !self
                        .division_commanders
                        .compare(commanders.len, commanders.required),
                );
                self.divisions.mismatches +=
                    usize::from(!self.width_terms.compare(width.len, width.required));
                self.divisions.mismatches +=
                    usize::from(!self.manpower.compare(manpower.len, manpower.required));
                self.divisions.mismatches += usize::from(
                    !self
                        .reinforcement_modifiers
                        .compare(reinforcement[0].len, reinforcement[0].required),
                );
                self.divisions.mismatches += usize::from(
                    !self
                        .supply_terms
                        .compare(reinforcement[1].len, reinforcement[1].required),
                );
                self.divisions.mismatches += usize::from(
                    !self
                        .acclimatization
                        .compare(reinforcement[2].len, reinforcement[2].required),
                );
                self.divisions.mismatches += usize::from(
                    !self
                        .transport_terms
                        .compare(reinforcement[3].len, reinforcement[3].required),
                );
                self.divisions.mismatches +=
                    usize::from(!self.window.compare(window.len, window.required));
                ticks[1] += elapsed(comparison_started);
            }
            if active[2] {
                let comparison_started = qpc();
                self.sides.mismatches +=
                    usize::from(!self.members.compare(arenas[1].len, arenas[1].required));
                self.sides.mismatches +=
                    usize::from(!self.modifiers.compare(arenas[2].len, arenas[2].required));
                ticks[2] += elapsed(comparison_started);
            }
        } else {
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
            self.window.len = window.len;
            self.window.required = window.required;
            self.manpower.len = manpower.len;
            self.manpower.required = manpower.required;
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
        }
        (outcomes, ticks)
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        let started = qpc();
        let needed: [bool; N] = needed.try_into().expect("land combat table count");
        self.details.mismatches = 0;
        self.divisions.mismatches = 0;
        self.sides.mismatches = 0;
        let (first, mut ticks) = self.pass(memory, base, state, false, [true; N]);
        self.details.outcome = first[0];
        self.divisions.outcome = first[1];
        self.sides.outcome = first[2];
        self.details.checked = needed[0];
        self.divisions.checked = needed[1];
        self.sides.checked = needed[2];
        let mut matched = [false; N];
        if needed.iter().any(|v| *v) {
            let (second, costs) = self.pass(memory, base, state, true, needed);
            for index in 0..N {
                ticks[index] += costs[index];
            }
            if needed[0] {
                let comparison_started = qpc();
                let arena_mismatches = self.details.mismatches;
                self.details.compare(second[0]);
                self.details.mismatches += arena_mismatches;
                matched[0] = full_match(first[0], second[0], self.details.mismatches);
                ticks[0] += elapsed(comparison_started);
            }
            if needed[1] {
                let comparison_started = qpc();
                let arena_mismatches = self.divisions.mismatches;
                self.divisions.compare(second[1]);
                self.divisions.mismatches += arena_mismatches;
                matched[1] = full_match(first[1], second[1], self.divisions.mismatches);
                ticks[1] += elapsed(comparison_started);
            }
            if needed[2] {
                let comparison_started = qpc();
                let arena_mismatches = self.sides.mismatches;
                self.sides.compare(second[2]);
                self.sides.mismatches += arena_mismatches;
                matched[2] = full_match(first[2], second[2], self.sides.mismatches);
                ticks[2] += elapsed(comparison_started);
            }
        }
        ticks[0] += elapsed(started).saturating_sub(ticks.iter().sum::<u64>());
        (matched, ticks)
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            checked(&self.details),
            checked(&self.divisions),
            checked(&self.sides),
        ]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.details.mismatches,
            self.divisions.mismatches,
            self.sides.mismatches,
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
                    indicator: r.indicator,
                    phase: r.phase,
                    width_factor: r.width_factor,
                    attacker_bonus: r.attacker_bonus,
                    defender_bonus: r.defender_bonus,
                    terrain_name: r.terrain_name[..r.terrain_name_len].to_vec(),
                    stored_terrain_name: r.stored_terrain_name[..r.stored_terrain_name_len]
                        .to_vec(),
                    stored_terrain_valid: r.stored_terrain_valid,
                    terrain_width: r.terrain_width,
                    terrain_additional_width: r.terrain_additional_width,
                    air_defines: r.air_defines,
                    armour_icon_defines: r.armour_icon_defines,
                    commander_learning_threshold: r.commander_learning_threshold,
                    parent_commander_factor: r.parent_commander_factor,
                    portrait_feature_flags: r.portrait_feature_flags,
                    portrait_callback_kind: r.portrait_callback_kind,
                    recon_skill_impact: r.recon_skill_impact,
                    piercing_thresholds: r.piercing_thresholds[..r.piercing_len].to_vec(),
                    piercing_damage: r.piercing_damage[..r.piercing_len].to_vec(),
                    sides: std::array::from_fn(|side| {
                        let s = &r.sides[side];
                        LandCombatSideDetails {
                        flags: s.flags,
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
                        organisation_loss_index: s.organisation_loss_index,
                        organisation_loss_count: s.organisation_loss_count,
                        border_modifier: s.border_modifier,
                        border_dig_in_factor: s.border_dig_in_factor,
                        border_terrain_factor: s.border_terrain_factor,
                        commander: s.commander,
                        commander_skill: s.commander_skill,
                        commander_next_skill: super::naval_details::commander_skill(
                            s.commander_next_skill,
                        ),
                        commander_category: s.commander_category,
                        commander_country: s.commander_country,
                        commander_description: s.commander_description
                            [..s.commander_description_len]
                            .to_vec(),
                        parent_commander: s.parent_commander,
                        parent_commander_name: s.parent_commander_name
                            [..s.parent_commander_name_len]
                            .to_vec(),
                        parent_command_capacity: s.parent_command_capacity,
                        parent_formation_kind: s.parent_formation_kind,
                        parent_command_count: s.parent_command_count,
                        portrait_modifiers: s.portrait_modifiers[..s.portrait_modifiers_len]
                            .iter()
                            .map(|m| LandCombatCommanderPortraitModifier {
                                source: m.source,
                                id: m.id,
                                value: m.value,
                                metadata_flags: m.metadata_flags,
                                category_mask: m.category_mask,
                                feature_selector: m.feature_selector,
                            })
                            .collect(),
                        portrait_tooltips: s.portrait_tooltips[..s.portrait_tooltips_len]
                            .iter()
                            .map(|t| LandCombatCommanderPortraitTooltip {
                                source: t.source,
                                bucket: t.bucket,
                                key: t.key,
                                text: s.portrait_text[t.text_start..t.text_start + t.text_len]
                                    .to_vec(),
                            })
                            .collect(),
                        commander_experience: s.commander_experience,
                        commander_name: s.commander_name[..s.commander_name_len].to_vec(),
                        commander_modifiers: s.commander_modifiers,
                        commander_modifier_present: s.commander_modifier_present,
                        learning_traits: s.learning_traits[..s.learning_traits_len]
                            .iter()
                            .map(|t| LandCombatCommanderLearningTrait {
                                token_id: t.token_id,
                                definition_ordinal: t.definition_ordinal,
                                required_experience: t.required_experience,
                                experience: t.experience,
                                token: s.learning_text[t.token_start..t.token_start + t.token_len]
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
                        hq_unit: captured.hq_unit[side],
                        }
                    }),
                }
            }),
            table::<tables::LandCombatDivisions, _>(outcomes[1], &self.divisions.rows, |r| {
                let s = r.stats;
                LandCombatDivisionRow {
                    combat: r.combat,
                    side: r.side,
                    ordinal: r.ordinal,
                    division: s.division,
                    name: s.name.as_bytes().to_vec(),
                    role: r.role,
                    province: r.province,
                    strength: s.strength,
                    strength_combat_factor: s.strength_combat_factor,
                    organisation: s.organisation,
                    experience: s.experience,
                    entrenchment: s.entrenchment,
                    entrenchment_cap: s.entrenchment_cap,
                    retreat: s.retreat,
                    stat_values: s.stat_values,
                    max_organisation: s.max_organisation,
                    max_strength: s.max_strength,
                    armour: s.armour,
                    piercing: s.piercing,
                    air_attack: s.air_attack,
                    reconnaissance: s.reconnaissance,
                    initiative: s.initiative,
                    commander: s.commander,
                    direction: r.direction,
                    direction_group: r.direction_group,
                    template_frames: r.template_frames,
                    commander_inputs: r.commander_input.map(|index| {
                        let c = &self.division_commanders.first[index];
                        replay::rows::LandCombatDivisionCommander {
                            handle: c.handle,
                            skill: c.skill,
                            name: c.name[..c.name_len].to_vec(),
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
                    manpower: self.manpower.first
                        [r.manpower_start..r.manpower_start + r.manpower_len]
                        .iter()
                        .map(|m| LandDivisionManpowerTerm {
                            source: m.source,
                            amount: m.amount,
                        })
                        .collect(),
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
                        effective_owner: r.context.effective_owner,
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
                        defines: r.context.defines,
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
                    flags: r.flags,
                    indicator: r.indicator,
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
fn elapsed(start: i64) -> u64 {
    qpc().saturating_sub(start).max(0) as u64
}
fn checked<R>(rows: &CaptureRows<R>) -> Outcome {
    if rows.checked && rows.mismatches != 0 {
        Outcome::Unavailable {
            reason: "land combat window operands changed during double read",
        }
    } else {
        rows.outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_capacity_omits_whole_side_and_counts_arena_pressure() {
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
        let mut destination = Destination {
            details: &mut [],
            division_commanders: &mut [],
            commander_keys: &mut [],
            commander_progress: Progress::default(),
            commander_key_count: 0,
            required_commander_keys: 0,
            tactic_candidates: &mut [],
            tactic_weights: &mut [],
            tactic_unlocks: &mut [],
            tactic_registry: None,
            tactic_unlock_progress: Progress::default(),
            divisions: &mut [],
            sides: &mut sides,
            rosters: &mut [],
            members: &mut members,
            modifiers: &mut values,
            width_terms: &mut [],
            width_progress: Progress::default(),
            progress: [Progress::default(); N],
            arenas: [Progress::default(); N],
            active: [false, false, true],
            manpower: &mut [],
            manpower_progress: Progress::default(),
            reinforcement_modifiers: &mut [],
            supply_terms: &mut [],
            acclimatization: &mut [],
            transport_terms: &mut [],
            reinforcement_progress: [Progress::default(); 4],
            ticks: [0; N],
            window: &mut [],
            window_progress: Progress::default(),
            current_window: None,
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
    }

    #[test]
    fn tactic_registry_and_country_unlock_spans_keep_raw_order() {
        let c = Capacities {
            combats: 1,
            census_participants: 1,
            details: 1,
            divisions: 1,
            sides: 1,
            rosters: 1,
            modifier_members: 1,
            modifiers: 1,
            width_terms: 1,
            manpower: 1,
            reinforcement_modifiers: 1,
            supply_terms: 1,
            acclimatization: 1,
            transport_terms: 1,
            tactic_candidates: 1,
            tactic_weights: 1,
            tactic_unlocks: 3,
            division_commanders: 1,
            commander_keys: 2,
            window_divisions: 1,
        };
        let mut captured = Captured::with_capacities(&c, &[200; N]);
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
            rosters: 6,
            modifier_members: 1,
            modifiers: 1,
            width_terms: 1,
            manpower: 1,
            reinforcement_modifiers: 1,
            supply_terms: 1,
            acclimatization: 1,
            transport_terms: 1,
            tactic_candidates: 1,
            tactic_weights: 1,
            tactic_unlocks: 1,
            division_commanders: 1,
            commander_keys: 2,
            window_divisions: 1,
        };
        let mut captured = Captured::with_capacities(&c, &[200; N]);
        captured.details.outcome = Outcome::Rows { len: 2 };
        captured.rosters.len = 6;
        captured.rosters.first.copy_from_slice(&[13, 6, 11, 4, 1, 9]);
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
        captured.details.rows[0].hq_unit = [0x1234_0000_0033, 0];
        let [
            TableView::Complete(replay::rows::TableRows::LandCombatDetails(details)),
            _,
            _,
        ] = captured.tables()
        else {
            panic!("complete land details were not projected")
        };
        let lists: Vec<_> = details
            .iter()
            .map(|row| row.sides.clone().map(|side| [side.committed, side.reserves]))
            .collect();
        assert_eq!(
            lists,
            [
                [[vec![13, 6], vec![11]], [vec![], vec![]]],
                [[vec![], vec![]], [vec![4], vec![1, 9]]],
            ]
        );
        assert_eq!(details[0].sides[0].hq_unit, 0x1234_0000_0033);
        assert_eq!(details[0].sides[1].hq_unit, 0);
        assert_eq!(details[1].sides[0].hq_unit, 0);
    }

    #[test]
    fn window_operands_project_onto_division_rows() {
        let c = Capacities {
            combats: 1,
            census_participants: 1,
            details: 1,
            divisions: 1,
            sides: 1,
            rosters: 1,
            modifier_members: 1,
            modifiers: 1,
            width_terms: 1,
            manpower: 1,
            reinforcement_modifiers: 1,
            supply_terms: 1,
            acclimatization: 1,
            transport_terms: 1,
            tactic_candidates: 1,
            tactic_weights: 1,
            tactic_unlocks: 1,
            division_commanders: 1,
            commander_keys: 2,
            window_divisions: 1,
        };
        let mut captured = Captured::with_capacities(&c, &[200; N]);
        captured.divisions.outcome = Outcome::Rows { len: 1 };
        captured.divisions.rows[0] = Division {
            combat: 0x1000_0000_003e,
            side: 0,
            ordinal: 0,
            province: 362,
            role: 1,
            direction: 9263,
            direction_group: -3,
            template_frames: 3,
            ..Default::default()
        };
        let [
            _,
            TableView::Complete(replay::rows::TableRows::LandCombatDivisions(divisions)),
            _,
        ] = captured.tables()
        else {
            panic!("complete land divisions were not projected")
        };
        assert_eq!(divisions[0].province, 362);
        assert_eq!(divisions[0].direction, 9263);
        assert_eq!(divisions[0].direction_group, -3);
        assert_eq!(divisions[0].template_frames, 3);
    }

    #[test]
    fn source_failures_and_independent_window_retirement_are_explicit() {
        for successes in [0, 200] {
            let pool = Captured::with_capacities(&Capacities::default(), &[successes; N]);
            eprintln!(
                "land default pool at {successes} successful checks: {} bytes",
                pool.allocated_bytes()
            );
        }
        eprintln!(
            "land combat static row bytes: details={} division={} modifier_side={} modifier_member={} modifier={} width={} reinforcement_modifier={} supply={} acclimatization={} transport={} manpower={}; native live cost unmeasured",
            size_of::<Detail>(),
            size_of::<Division>(),
            size_of::<ModifierSide>(),
            size_of::<ModifierMember>(),
            size_of::<carmy_combat_inputs::Modifier>(),
            size_of::<carmy_combat_inputs::WidthTerm>(),
            size_of::<reinforcement::ModifierTerm>(),
            size_of::<reinforcement::SupplyTerm>(),
            size_of::<reinforcement::AcclimatizationTerm>(),
            size_of::<reinforcement::TransportTerm>(),
            size_of::<carmy_combat_inputs::ManpowerTerm>(),
        );
        let mut c = Capacities {
            combats: 1,
            census_participants: 2,
            details: 1,
            divisions: 2,
            sides: 2,
            rosters: 2,
            modifier_members: 2,
            modifiers: 2,
            width_terms: 2,
            manpower: 2,
            reinforcement_modifiers: 2,
            supply_terms: 2,
            acclimatization: 2,
            transport_terms: 2,
            tactic_candidates: 2,
            tactic_weights: 2,
            tactic_unlocks: 2,
            division_commanders: 2,
            commander_keys: 2,
            window_divisions: 2,
        };
        let mut captured = Captured::with_capacities(&c, &[200, 0, 200]);
        assert!(captured.details.second.is_empty() && captured.rosters.second.is_empty());
        assert!(
            captured.tactic_candidates.second.is_empty()
                && captured.tactic_weights.second.is_empty()
                && captured.tactic_unlocks.second.is_empty()
        );
        assert!(captured.sides.second.is_empty() && captured.members.second.is_empty());
        assert_eq!(captured.divisions.second.len(), 2);
        assert_eq!(captured.division_commanders.second.len(), 2);
        assert_eq!(captured.width_terms.second.len(), 2);
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        assert_eq!(
            captured.capture(&memory, 0, 0, &[false, true, false]).0,
            [false; N]
        );
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
        captured.grow(&mut c, &[200; N]);
        assert!(c.division_commanders >= 5 && c.commander_keys >= 7);
        assert!(captured.division_commanders.second.is_empty());
        assert!(c.tactic_candidates >= 7 && c.tactic_weights >= 11 && c.tactic_unlocks >= 13);
        assert!(c.sides >= 4 && c.modifier_members >= 7 && c.modifiers >= 19 && c.width_terms >= 9);
        assert!(captured.divisions.second.is_empty() && captured.width_terms.second.is_empty());
        captured.details.outcome = Outcome::Rows { len: 1 };
        captured.divisions.outcome = Outcome::Rows { len: 1 };
        captured.sides.outcome = Outcome::Rows { len: 1 };
        captured.divisions.checked = true;
        captured.divisions.mismatches = 1;
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 1 });
        assert!(matches!(
            captured.outcomes()[1],
            Outcome::Unavailable { .. }
        ));
        assert_eq!(captured.outcomes()[2], Outcome::Rows { len: 1 });
    }
}
