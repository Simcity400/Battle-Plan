//! Strategic state acquisition. Replay conversion and buffer growth run off-thread.

use crate::{
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::{
    Memory, ccountry_raid_status, cnuke, craid_instance, craid_system,
    nproject_cbreakthrough_progress, nproject_cprogram, nproject_cproject,
    nproject_sproject_history, peace_conference, scientist_roster, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};
use std::mem::size_of;

pub const N: usize = 10;
pub const TABLES: [TableId; N] = [
    TableId::ProjectPrograms,
    TableId::Projects,
    TableId::ProjectHistory,
    TableId::ProjectBreakthroughs,
    TableId::Nukes,
    TableId::RaidCountries,
    TableId::Raids,
    TableId::PeaceConferences,
    TableId::Scientists,
    TableId::ScientistSkills,
];

/// Each rooted walk produces two tables with independent consistency windows.
/// Its entire cost is attributed once to the anchor named by each group.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("project pool and reward histories", 1, &[1, 2]),
    ("raid countries and instances", 5, &[5, 6]),
    ("scientist rosters and specializations", 8, &[8, 9]),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub program_names: usize,
    pub program_candidates: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [256, 32768, 1024, 2048, 1024, 512, 256, 32, 2048, 8192],
            program_names: 8192,
            program_candidates: 1024,
        }
    }
}

pub struct Captured {
    programs: CaptureRows<nproject_cprogram::Row>,
    projects: CaptureRows<nproject_cproject::Row>,
    history: CaptureRows<nproject_sproject_history::Row>,
    breakthroughs: CaptureRows<nproject_cbreakthrough_progress::Row>,
    nukes: CaptureRows<cnuke::Row>,
    raid_countries: CaptureRows<ccountry_raid_status::Row>,
    raids: CaptureRows<craid_instance::Row>,
    peace: CaptureRows<peace_conference::Row>,
    scientists: CaptureRows<scientist_roster::Row>,
    scientist_skills: CaptureRows<scientist_roster::SkillRow>,
    names: Arena<u8>,
    candidates: Vec<usize>,
    required_candidates: usize,
}

impl Captured {
    /// All capacity, success, outcome and cost arrays use local TABLES order.
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        Self {
            programs: CaptureRows::new(capacity.rows[0], successful[0]),
            projects: CaptureRows::new(capacity.rows[1], successful[1]),
            history: CaptureRows::new(capacity.rows[2], successful[2]),
            breakthroughs: CaptureRows::new(capacity.rows[3], successful[3]),
            nukes: CaptureRows::new(capacity.rows[4], successful[4]),
            raid_countries: CaptureRows::new(capacity.rows[5], successful[5]),
            raids: CaptureRows::new(capacity.rows[6], successful[6]),
            peace: CaptureRows::new(capacity.rows[7], successful[7]),
            scientists: CaptureRows::new(capacity.rows[8], successful[8]),
            scientist_skills: CaptureRows::new(capacity.rows[9], successful[9]),
            names: Arena::new(capacity.program_names, successful[0]),
            candidates: vec![0; capacity.program_candidates],
            required_candidates: 0,
        }
    }

    pub fn reset(&mut self) {
        self.programs.reset();
        self.projects.reset();
        self.history.reset();
        self.breakthroughs.reset();
        self.nukes.reset();
        self.raid_countries.reset();
        self.raids.reset();
        self.peace.reset();
        self.scientists.reset();
        self.scientist_skills.reset();
        self.names.reset();
        self.required_candidates = 0;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.programs.allocated_bytes()
            + self.projects.allocated_bytes()
            + self.history.allocated_bytes()
            + self.breakthroughs.allocated_bytes()
            + self.nukes.allocated_bytes()
            + self.raid_countries.allocated_bytes()
            + self.raids.allocated_bytes()
            + self.peace.allocated_bytes()
            + self.scientists.allocated_bytes()
            + self.scientist_skills.allocated_bytes()
            + self.names.allocated_bytes()
            + self.candidates.capacity() * size_of::<usize>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        self.programs.grow(&mut capacity.rows[0], successful[0]);
        self.projects.grow(&mut capacity.rows[1], successful[1]);
        self.history.grow(&mut capacity.rows[2], successful[2]);
        self.breakthroughs
            .grow(&mut capacity.rows[3], successful[3]);
        self.nukes.grow(&mut capacity.rows[4], successful[4]);
        self.raid_countries
            .grow(&mut capacity.rows[5], successful[5]);
        self.raids.grow(&mut capacity.rows[6], successful[6]);
        self.peace.grow(&mut capacity.rows[7], successful[7]);
        self.scientists.grow(&mut capacity.rows[8], successful[8]);
        self.scientist_skills
            .grow(&mut capacity.rows[9], successful[9]);
        self.names.grow(&mut capacity.program_names, successful[0]);
        grow_capacity(&mut capacity.program_candidates, self.required_candidates);
        if self.required_candidates > 0
            && self.required_candidates
                >= capacity
                    .program_candidates
                    .saturating_sub(capacity.program_candidates / 4)
        {
            let required = capacity.program_candidates.saturating_add(1);
            grow_capacity(&mut capacity.program_candidates, required);
        }
        grow_vec(&mut self.candidates, capacity.program_candidates);
    }

    /// Only preallocated reads and comparisons run in the hourly callback.
    /// Definition/reward/specialization tokens retain their native u32 bits.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        let mut matched = [false; N];
        let mut ticks = [0; N];
        let begin = qpc();
        matched[0] = self.capture_programs(needed[0], |candidates, rows, names| {
            nproject_cprogram::walk(memory, base, game_state, candidates, rows, names)
        });
        ticks[0] = elapsed(begin);

        macro_rules! plain {
            ($field:ident, $index:literal, $module:ident) => {{
                let begin = qpc();
                matched[$index] = capture_rows(&mut self.$field, needed[$index], |rows| {
                    $module::walk(memory, base, game_state, rows)
                });
                ticks[$index] = elapsed(begin);
            }};
        }
        let begin = qpc();
        let project_matches = capture_pair(
            &mut self.projects,
            &mut self.history,
            [needed[1], needed[2]],
            |projects, histories| {
                nproject_cproject::walk_with_history(memory, base, game_state, projects, histories)
            },
        );
        matched[1..3].copy_from_slice(&project_matches);
        ticks[1] = elapsed(begin);
        plain!(breakthroughs, 3, nproject_cbreakthrough_progress);
        plain!(nukes, 4, cnuke);

        let begin = qpc();
        let raid_matches = capture_pair(
            &mut self.raid_countries,
            &mut self.raids,
            [needed[5], needed[6]],
            |parents, instances| craid_system::walk(memory, base, game_state, parents, instances),
        );
        matched[5..7].copy_from_slice(&raid_matches);
        ticks[5] = elapsed(begin);
        plain!(peace, 7, peace_conference);

        // One roster traversal fills both scientist tables. Their consistency
        // windows stay independent, so a full skill buffer cannot shorten the
        // roster census or silently drop a scientist's remaining records.
        let begin = qpc();
        let scientist_matches = capture_pair(
            &mut self.scientists,
            &mut self.scientist_skills,
            [needed[8], needed[9]],
            |scientists, skills| {
                let walk = scientist_roster::walk(
                    memory,
                    base,
                    game_state,
                    &mut scientist_roster::Buffers { scientists, skills },
                );
                (walk.scientists, walk.skills)
            },
        );
        matched[8..10].copy_from_slice(&scientist_matches);
        ticks[8] = elapsed(begin);
        (matched, ticks)
    }

    fn capture_programs(
        &mut self,
        needed: bool,
        mut walk: impl FnMut(
            &mut [usize],
            &mut [nproject_cprogram::Row],
            &mut [u8],
        ) -> nproject_cprogram::NameWalk,
    ) -> bool {
        let first = walk(
            &mut self.candidates,
            &mut self.programs.rows,
            &mut self.names.first,
        );
        self.programs.outcome = program_outcome(first, &self.programs.rows, &self.names.first);
        self.names.len = first.bytes_len;
        self.names.required = first.required_bytes;
        self.required_candidates = first.required_candidates;
        if !needed {
            return false;
        }
        self.programs.checked = true;
        // The pointer census is scratch, not captured data. Both reads can use
        // it sequentially without retaining a second copy in every pool buffer.
        let second = walk(
            &mut self.candidates,
            &mut self.programs.second,
            &mut self.names.second,
        );
        self.required_candidates = self.required_candidates.max(second.required_candidates);
        let outcome = program_outcome(second, &self.programs.second, &self.names.second);
        self.programs.compare(outcome);
        self.programs.mismatches +=
            usize::from(!self.names.compare(second.bytes_len, second.required_bytes));
        full_match(self.programs.outcome, outcome, self.programs.mismatches)
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.programs.mismatches,
            self.projects.mismatches,
            self.history.mismatches,
            self.breakthroughs.mismatches,
            self.nukes.mismatches,
            self.raid_countries.mismatches,
            self.raids.mismatches,
            self.peace.mismatches,
            self.scientists.mismatches,
            self.scientist_skills.mismatches,
        ]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            effective(&self.programs),
            effective(&self.projects),
            effective(&self.history),
            effective(&self.breakthroughs),
            effective(&self.nukes),
            effective(&self.raid_countries),
            effective(&self.raids),
            effective(&self.peace),
            effective(&self.scientists),
            effective(&self.scientist_skills),
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            self.program_table(outcomes[0]),
            table::<tables::Projects, _>(outcomes[1], &self.projects.rows, |r| rows::ProjectRow {
                owner: r.owner,
                local_id: r.local_id,
                definition_id: r.definition_id,
                program_reference: r.program_reference,
                started: r.started,
                completed: r.completed,
                state: rows::ProjectState {
                    current_offset: r.state.current_offset,
                    state_enums: r.state.state_enums,
                    not_completed: r.state.not_completed,
                    phase_progress: r.state.phase_progress,
                    phase_target: r.state.phase_target,
                    prototype_progress: r.state.prototype_progress,
                    iterations: r.state.iterations,
                },
            }),
            table::<tables::ProjectHistory, _>(outcomes[2], &self.history.rows, |r| {
                rows::ProjectHistoryRow {
                    owner: r.owner,
                    project: r.project,
                    ordinal: r.ordinal,
                    prototype_reward_handle: r.prototype_reward_handle,
                    option_handle: r.option_handle,
                    date: r.date,
                    context: rows::ProjectContext {
                        province: r.context.province,
                        progress: r.context.progress,
                        character_reference: r.context.character_reference,
                        country_present: r.context.country_present,
                        country_reference: r.context.country_reference,
                        scientist_present: r.context.scientist_present,
                        scientist_reference: r.context.scientist_reference,
                        state_present: r.context.state_present,
                        state_reference: r.context.state_reference,
                    },
                }
            }),
            table::<tables::ProjectBreakthroughs, _>(outcomes[3], &self.breakthroughs.rows, |r| {
                rows::ProjectBreakthroughRow {
                    owner: r.owner,
                    specialization: r.specialization,
                    points: r.points,
                }
            }),
            table::<tables::Nukes, _>(outcomes[4], &self.nukes.rows, |r| rows::NukeRow {
                country: r.country,
                nuke_type: r.nuke_type,
                amount_raw: r.amount_raw,
                ready: r.ready,
            }),
            table::<tables::RaidCountries, _>(outcomes[5], &self.raid_countries.rows, |r| {
                rows::RaidCountryRow {
                    canonical_slot: r.canonical_slot,
                    country: r.country,
                    priority: r.priority,
                    child_count: r.child_count,
                }
            }),
            table::<tables::Raids, _>(outcomes[6], &self.raids.rows, |r| rows::RaidRow {
                reference_type: r.reference_type,
                id: r.id,
                parent_slot: r.parent_slot,
                ordinal: r.ordinal,
                preparation_hours: r.preparation_hours,
                phase: r.phase,
                outcome: r.outcome,
                end_date_present: r.end_date_present,
                victim_country: r.victim_country,
                source: rows::RaidSource {
                    kind: r.source.kind,
                    kind_present: r.source.kind_present,
                    province: r.source.province,
                    ship_reference_type: r.source.ship_reference_type,
                    ship_reference_id: r.source.ship_reference_id,
                    country: r.source.country,
                },
            }),
            table::<tables::PeaceConferences, _>(outcomes[7], &self.peace.rows, |r| {
                rows::PeaceConferenceRow {
                    id: r.id,
                    ordinal: r.ordinal,
                    current_turn: r.current_turn,
                    finalized: r.finalized,
                }
            }),
            table::<tables::Scientists, _>(outcomes[8], &self.scientists.rows, |r| {
                rows::ScientistRow {
                    character: r.character,
                    country: r.country,
                    ordinal: r.ordinal,
                    trait_count: r.trait_count,
                    skill_count: r.skill_count,
                }
            }),
            table::<tables::ScientistSkills, _>(outcomes[9], &self.scientist_skills.rows, |r| {
                rows::ScientistSkillRow {
                    character: r.character,
                    ordinal: r.ordinal,
                    token: r.token,
                    level: r.level,
                    experience_raw: r.experience_raw,
                }
            }),
        ]
    }

    fn program_table(&self, outcome: Outcome) -> TableView {
        if self.names.required < self.names.len
            || !valid_names(
                outcome,
                &self.programs.rows,
                &self.names.first,
                self.names.len,
            )
        {
            return unavailable("program building range exceeds captured arena");
        }
        table::<tables::ProjectPrograms, _>(outcome, &self.programs.rows, |r| {
            rows::ProjectProgramRow {
                id: r.id,
                owner: r.owner,
                province: r.province,
                active_project: r.active_project,
                scientist: r.scientist,
                unread_rewards: r.unread_rewards,
                building: self.names.first[r.building_start..r.building_start + r.building_len]
                    .to_vec(),
            }
        })
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "walker marked truncation without missing rows",
        }
    } else {
        outcome
    }
}

fn capture_rows<R: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    needed: bool,
    mut walk: impl FnMut(&mut [R]) -> Outcome,
) -> bool {
    captured.outcome = bounded(walk(&mut captured.rows), captured.rows.len());
    if !needed {
        return false;
    }
    captured.checked = true;
    let second = bounded(walk(&mut captured.second), captured.second.len());
    captured.compare(second)
}

fn capture_pair<P: Default + Clone + PartialEq, C: Default + Clone + PartialEq>(
    parents: &mut CaptureRows<P>,
    instances: &mut CaptureRows<C>,
    needed: [bool; 2],
    mut walk: impl FnMut(&mut [P], &mut [C]) -> (Outcome, Outcome),
) -> [bool; 2] {
    let first = walk(&mut parents.rows, &mut instances.rows);
    parents.outcome = bounded(first.0, parents.rows.len());
    instances.outcome = bounded(first.1, instances.rows.len());
    let mut matched = [false; 2];
    if needed[0] || needed[1] {
        // A completed table can have no comparison storage. The native walker
        // still authenticates its root census and counts omitted rows,
        // so either table can finish its own window independently.
        let second = walk(
            if needed[0] {
                &mut parents.second
            } else {
                &mut []
            },
            if needed[1] {
                &mut instances.second
            } else {
                &mut []
            },
        );
        if needed[0] {
            parents.checked = true;
            matched[0] = parents.compare(bounded(second.0, parents.second.len()));
        }
        if needed[1] {
            instances.checked = true;
            matched[1] = instances.compare(bounded(second.1, instances.second.len()));
        }
    }
    matched
}

fn effective<R>(captured: &CaptureRows<R>) -> Outcome {
    if captured.checked && captured.mismatches != 0 {
        Outcome::Unavailable {
            reason: "table changed during double read",
        }
    } else {
        captured.outcome
    }
}

fn program_outcome(
    result: nproject_cprogram::NameWalk,
    rows: &[nproject_cprogram::Row],
    bytes: &[u8],
) -> Outcome {
    let outcome = bounded(result.outcome, rows.len());
    if result.required_bytes < result.bytes_len
        || (matches!(outcome, Outcome::Rows { .. }) && result.required_bytes != result.bytes_len)
        || !valid_names(outcome, rows, bytes, result.bytes_len)
    {
        Outcome::Unavailable {
            reason: "program building range exceeds captured arena",
        }
    } else {
        outcome
    }
}

fn valid_names(
    outcome: Outcome,
    rows: &[nproject_cprogram::Row],
    bytes: &[u8],
    len: usize,
) -> bool {
    len <= bytes.len()
        && rows.get(..outcome.len()).is_some_and(|rows| {
            rows.iter().all(|r| {
                r.building_start
                    .checked_add(r.building_len)
                    .is_some_and(|end| end <= len)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use replay::rows::TableSchema;

    fn small() -> (Capacities, Captured) {
        let capacities = Capacities {
            rows: [2; N],
            program_names: 4,
            program_candidates: 2,
        };
        let captured = Captured::with_capacities(&capacities, &[0; N]);
        (capacities, captured)
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete table")
        };
        rows.rows::<S>().unwrap()
    }

    #[test]
    fn conversion_preserves_native_keys_fixed_values_presence_and_literal_building_bytes() {
        let (_, mut captured) = small();
        captured.programs.outcome = Outcome::Rows { len: 1 };
        captured.programs.rows[0] = nproject_cprogram::Row {
            id: u32::MAX,
            active_project: Some(0),
            scientist: u64::MAX,
            unread_rewards: i32::MIN,
            building_start: 1,
            building_len: 3,
            ..Default::default()
        };
        captured.names.first.copy_from_slice(&[99, 0xff, 0, b'a']);
        captured.names.len = 4;
        captured.names.required = 4;
        captured.projects.outcome = Outcome::Rows { len: 1 };
        captured.projects.rows[0] = nproject_cproject::Row {
            owner: u32::MAX,
            local_id: u32::MAX,
            definition_id: u32::MAX,
            program_reference: u64::MAX,
            started: 1,
            completed: 0,
            state: engine::nproject_cproject_state_machine::State {
                current_offset: u32::MAX,
                state_enums: [1, 2, 3, u32::MAX],
                not_completed: 1,
                phase_progress: i64::MIN,
                phase_target: i32::MAX,
                prototype_progress: i32::MIN,
                iterations: -17,
            },
        };
        captured.history.outcome = Outcome::Rows { len: 1 };
        captured.history.rows[0] = nproject_sproject_history::Row {
            owner: u32::MAX,
            project: u32::MAX,
            ordinal: u32::MAX,
            prototype_reward_handle: u32::MAX,
            option_handle: u32::MAX - 1,
            date: i32::MIN,
            context: engine::nproject_sproject_context::Row {
                province: i32::MIN,
                progress: i32::MAX,
                character_reference: u64::MAX,
                country_present: 1,
                country_reference: Some(u32::MAX),
                scientist_present: 0,
                scientist_reference: None,
                state_present: 1,
                state_reference: Some(0),
            },
        };
        captured.breakthroughs.outcome = Outcome::Rows { len: 1 };
        captured.breakthroughs.rows[0] = nproject_cbreakthrough_progress::Row {
            owner: u32::MAX,
            specialization: u32::MAX,
            points: i32::MIN,
        };
        captured.nukes.outcome = Outcome::Rows { len: 1 };
        captured.nukes.rows[0] = cnuke::Row {
            country: u32::MAX,
            nuke_type: 1,
            amount_raw: i64::MAX,
            ready: i32::MIN,
        };
        captured.raid_countries.outcome = Outcome::Rows { len: 1 };
        captured.raid_countries.rows[0] = ccountry_raid_status::Row {
            canonical_slot: 0,
            country: -1,
            priority: i32::MIN,
            child_count: 1,
        };
        captured.raids.outcome = Outcome::Rows { len: 1 };
        captured.raids.rows[0] = craid_instance::Row {
            reference_type: u32::MAX - 1,
            id: u32::MAX,
            parent_slot: 0,
            ordinal: 7,
            preparation_hours: i32::MIN,
            phase: 3,
            outcome: -1,
            end_date_present: 1,
            victim_country: -1,
            source: engine::craid_source::Row {
                kind: None,
                kind_present: 0,
                province: u32::MAX,
                ship_reference_type: u32::MAX,
                ship_reference_id: u32::MAX - 1,
                country: -1,
            },
        };
        captured.peace.outcome = Outcome::Rows { len: 1 };
        captured.peace.rows[0] = peace_conference::Row {
            id: u64::MAX,
            ordinal: u32::MAX,
            current_turn: i32::MIN,
            finalized: 1,
        };
        let views = captured.tables();
        let program = &complete::<tables::ProjectPrograms>(&views[0])[0];
        assert_eq!(
            (
                program.active_project,
                program.scientist,
                program.unread_rewards
            ),
            (Some(0), u64::MAX, i32::MIN)
        );
        assert_eq!(program.building, [0xff, 0, b'a']);
        let project = &complete::<tables::Projects>(&views[1])[0];
        assert_eq!(project.key(), u128::from(u64::MAX));
        assert_eq!(
            (project.definition_id, project.program_reference),
            (u32::MAX, u64::MAX)
        );
        assert_eq!(
            (
                project.state.phase_progress,
                project.state.phase_target,
                project.state.prototype_progress,
                project.state.iterations
            ),
            (i64::MIN, i32::MAX, i32::MIN, -17)
        );
        let history = &complete::<tables::ProjectHistory>(&views[2])[0];
        assert_eq!(history.key(), (1_u128 << 96) - 1);
        assert_eq!(
            (history.prototype_reward_handle, history.option_handle),
            (u32::MAX, u32::MAX - 1)
        );
        assert_eq!(
            (
                history.context.country_reference,
                history.context.scientist_reference,
                history.context.state_reference
            ),
            (Some(u32::MAX), None, Some(0))
        );
        let breakthrough = &complete::<tables::ProjectBreakthroughs>(&views[3])[0];
        assert_eq!(
            (breakthrough.key(), breakthrough.points),
            (u128::from(u64::MAX), i32::MIN)
        );
        let nuke = &complete::<tables::Nukes>(&views[4])[0];
        assert_eq!((nuke.amount_raw, nuke.ready), (i64::MAX, i32::MIN));
        let parent = &complete::<tables::RaidCountries>(&views[5])[0];
        assert_eq!(
            (parent.canonical_slot, parent.country, parent.priority),
            (0, -1, i32::MIN)
        );
        let raid = &complete::<tables::Raids>(&views[6])[0];
        assert_eq!(
            raid.key(),
            (u128::from(u32::MAX) << 32) | u128::from(u32::MAX - 1)
        );
        assert_eq!(
            (
                raid.source.kind,
                raid.source.province,
                raid.source.ship_reference_id
            ),
            (None, u32::MAX, u32::MAX - 1)
        );
        let peace = &complete::<tables::PeaceConferences>(&views[7])[0];
        assert_eq!(
            (peace.id, peace.ordinal, peace.current_turn, peace.finalized),
            (u64::MAX, u32::MAX, i32::MIN, 1)
        );
    }

    #[test]
    fn invalid_building_ranges_and_impossible_row_counts_refuse_without_panicking() {
        let (_, mut captured) = small();
        captured.programs.outcome = Outcome::Rows { len: 1 };
        captured.names.len = 2;
        captured.names.required = 2;
        for (start, len) in [(1, 2), (usize::MAX, 1), (4, 0)] {
            captured.programs.rows[0].building_start = start;
            captured.programs.rows[0].building_len = len;
            assert!(matches!(
                captured.program_table(captured.programs.outcome),
                TableView::Unavailable { .. }
            ));
        }
        for outcome in [
            Outcome::Rows { len: 3 },
            Outcome::Truncated {
                len: 1,
                unread_rows: 0,
            },
        ] {
            assert!(matches!(bounded(outcome, 2), Outcome::Unavailable { .. }));
        }
        assert!(matches!(
            program_outcome(
                nproject_cprogram::NameWalk {
                    outcome: Outcome::Rows { len: 0 },
                    bytes_len: 0,
                    required_bytes: 1,
                    required_candidates: 1,
                },
                &[],
                &[],
            ),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn building_only_change_is_a_mismatch_and_complete_reads_are_required() {
        let (_, mut captured) = small();
        let mut calls = 0;
        assert!(!captured.capture_programs(true, |_, rows, bytes| {
            calls += 1;
            rows[0] = nproject_cprogram::Row {
                id: 1,
                building_len: 1,
                ..Default::default()
            };
            bytes[0] = calls;
            nproject_cprogram::NameWalk {
                outcome: Outcome::Rows { len: 1 },
                bytes_len: 1,
                required_bytes: 1,
                required_candidates: 1,
            }
        }));
        assert_eq!(captured.checked_mismatches()[0], 1);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
        captured.reset();
        assert!(
            !captured.capture_programs(true, |_, _, _| nproject_cprogram::NameWalk {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 1
                },
                bytes_len: 0,
                required_bytes: 9,
                required_candidates: 1,
            })
        );
        assert_eq!(captured.checked_mismatches()[0], 0);
        assert_eq!(
            captured.outcomes()[0],
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
    }

    #[test]
    fn shared_raid_reads_keep_parent_and_child_consistency_windows_independent() {
        let (_, mut captured) = small();
        let mut calls = 0;
        let matched = capture_pair(
            &mut captured.raid_countries,
            &mut captured.raids,
            [true, true],
            |parents, instances| {
                calls += 1;
                parents[0] = ccountry_raid_status::Row {
                    child_count: 1,
                    ..Default::default()
                };
                instances[0] = craid_instance::Row {
                    id: 7,
                    preparation_hours: calls,
                    ..Default::default()
                };
                (Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 })
            },
        );
        assert_eq!((calls, matched), (2, [true, false]));
        assert_eq!(captured.outcomes()[5], Outcome::Rows { len: 1 });
        assert!(matches!(
            captured.outcomes()[6],
            Outcome::Unavailable { .. }
        ));

        for needed in [[true, false], [false, true], [false, false]] {
            captured.reset();
            calls = 0;
            let matched = capture_pair(
                &mut captured.raid_countries,
                &mut captured.raids,
                needed,
                |parents, instances| {
                    calls += 1;
                    if calls == 2 {
                        assert_eq!(parents.is_empty(), !needed[0]);
                        assert_eq!(instances.is_empty(), !needed[1]);
                    }
                    let outcome = |empty| {
                        if empty {
                            Outcome::Truncated {
                                len: 0,
                                unread_rows: 1,
                            }
                        } else {
                            Outcome::Rows { len: 1 }
                        }
                    };
                    if let Some(parent) = parents.first_mut() {
                        *parent = ccountry_raid_status::Row::default();
                    }
                    if let Some(instance) = instances.first_mut() {
                        *instance = craid_instance::Row::default();
                    }
                    (outcome(parents.is_empty()), outcome(instances.is_empty()))
                },
            );
            assert_eq!(matched, needed);
            assert_eq!(calls, if needed[0] || needed[1] { 2 } else { 1 });
            assert_eq!(
                [captured.raid_countries.checked, captured.raids.checked],
                needed
            );
            assert_eq!(&captured.outcomes()[5..7], &[Outcome::Rows { len: 1 }; 2]);
        }
    }

    #[test]
    fn scratch_and_arena_growth_use_both_reads_and_release_finished_comparisons() {
        let (mut capacities, mut captured) = small();
        let mut calls = 0;
        assert!(!captured.capture_programs(true, |_, _, _| {
            calls += 1;
            nproject_cprogram::NameWalk {
                outcome: Outcome::Unavailable {
                    reason: "program pointer scratch capacity insufficient",
                },
                bytes_len: 0,
                required_bytes: 17 * calls,
                required_candidates: 9 * calls,
            }
        }));
        captured.projects.outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 63,
        };
        captured.grow(&mut capacities, &[0; N]);
        assert!(captured.candidates.len() >= 18 && captured.names.first.len() >= 34);
        assert!(captured.projects.rows.len() >= 65);
        let before = captured.allocated_bytes();
        let mut successful = [CONSISTENCY_TARGET; N];
        successful[6] = 0;
        captured.reset();
        captured.grow(&mut capacities, &successful);
        assert!(captured.programs.second.is_empty() && captured.names.second.is_empty());
        assert!(captured.raid_countries.second.is_empty() && !captured.raids.second.is_empty());
        assert!(captured.allocated_bytes() < before);
        assert_eq!(captured.checked_mismatches(), [0; N]);
    }
}
