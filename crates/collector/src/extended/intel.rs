//! Intel acquisition buffers. Allocation, growth and replay conversion are off-thread.

use crate::{
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_vec},
};
use engine::{
    Memory, ccountry, ccountry_intel, ccountry_intel_network, ccountry_operation_manager,
    ccountry_operation_token_manager, ccryptology, ccurrent_autonomy_status, cdiplomacy_status,
    cflag_manager, cgame_state, cintelligence_agency, coperation_instance, coperative_leader,
    country_intel_row,
    cpdx_array::Array,
    crelation_status, cscript_flag, cstate,
    csubject_relation::{self, Name},
    cvariables, incoming_diplomatic_action_queue, operation_assigned_operative, read, read_slice,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};
use std::mem::size_of;

pub const N: usize = 16;
pub const TABLES: [TableId; N] = [
    TableId::Capitulation,
    TableId::Subjects,
    TableId::Autonomy,
    TableId::IntelligenceAgencies,
    TableId::OperativeMemberships,
    TableId::Cryptology,
    TableId::IntelNetworks,
    TableId::ScriptFlags,
    TableId::ScriptVariables,
    TableId::Operations,
    TableId::OperationAssignments,
    TableId::OperationTokens,
    TableId::IncomingDiplomaticActions,
    TableId::RelationOpinions,
    TableId::RelationTuples,
    TableId::CountryIntel,
];
/// The operation walker supplies both tables in one census, and one relation
/// matrix pass supplies the cached opinions and the typed tuples. Each group's
/// entire cost, including whichever independent second reads are needed, is
/// charged once to its anchor.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("operations and assigned slots", 9, &[9, 10]),
    ("relation opinions and typed tuples", 13, &[13, 14]),
];

pub const GLOBAL_SCOPE: u8 = 0;
pub const COUNTRY_SCOPE: u8 = 1;
pub const STATE_SCOPE: u8 = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    /// Bytes of caller-owned scratch for the bulk intel-record copies. This is a
    /// fixed copy window, not a row capacity, so `grow` never raises it.
    pub intel_scratch: usize,
    pub subject_names: usize,
    pub autonomy_names: usize,
    pub variable_entries: usize,
    pub variable_names: usize,
    pub tokens: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [
                512, 256, 256, 512, 4096, 4096, 16_384, 8192, 2048, 2048, 8192, 4096, 512, 16_384,
                4096, 65_536,
            ],
            intel_scratch: 16_384,
            subject_names: 16_384,
            autonomy_names: 32_768,
            variable_entries: 16_384,
            variable_names: 512 * 1024,
            tokens: 32_768,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ScopedFlag {
    scope_kind: u8,
    scope_id: u32,
    flag: cscript_flag::Row,
}

pub struct Captured {
    capitulation: CaptureRows<cdiplomacy_status::CapitulationRow>,
    subjects: CaptureRows<csubject_relation::Row>,
    autonomy: CaptureRows<ccurrent_autonomy_status::Row>,
    agencies: CaptureRows<cintelligence_agency::Row>,
    operatives: CaptureRows<coperative_leader::Row>,
    cryptology: CaptureRows<ccryptology::Row>,
    networks: CaptureRows<ccountry_intel_network::Row>,
    flags: CaptureRows<ScopedFlag>,
    variables: CaptureRows<cvariables::ScopeRow>,
    operations: CaptureRows<coperation_instance::Row>,
    assignments: CaptureRows<operation_assigned_operative::Row>,
    operation_tokens: CaptureRows<ccountry_operation_token_manager::Row>,
    incoming_actions: CaptureRows<incoming_diplomatic_action_queue::Row>,
    relation_opinions: CaptureRows<crelation_status::OpinionRow>,
    relation_tuples: CaptureRows<crelation_status::TupleRow>,
    country_intel: CaptureRows<country_intel_row::Row>,
    intel_scratch: Vec<u8>,
    subject_names: Arena<u8>,
    autonomy_names: Arena<u8>,
    variable_entries: Arena<cvariables::Row>,
    variable_names: Arena<u8>,
    tokens: Arena<u32>,
    flag_scratch: Vec<cscript_flag::Row>,
    /// Raw-copy split gate for the relations family. Set by the game
    /// thread on refresh hours with a valid raw payload; `capture` then skips
    /// the typed relations walks and the decode worker fills these rows before
    /// the ready handoff. Preserved across `reset` (set per refresh, cleared
    /// by the worker after decode).
    pub(crate) relations_raw_mode: bool,
}

impl Captured {
    /// Every slice and returned array uses local TABLES order.
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        let operations_successful = successful[9].min(successful[10]);
        let relations_successful = successful[13].min(successful[14]);
        Self {
            capitulation: CaptureRows::new(capacity.rows[0], successful[0]),
            subjects: CaptureRows::new(capacity.rows[1], successful[1]),
            autonomy: CaptureRows::new(capacity.rows[2], successful[2]),
            agencies: CaptureRows::new(capacity.rows[3], successful[3]),
            operatives: CaptureRows::new(capacity.rows[4], successful[4]),
            cryptology: CaptureRows::new(capacity.rows[5], successful[5]),
            networks: CaptureRows::new(capacity.rows[6], successful[6]),
            flags: CaptureRows::new(capacity.rows[7], successful[7]),
            variables: CaptureRows::new(capacity.rows[8], successful[8]),
            operations: CaptureRows::new(capacity.rows[9], operations_successful),
            assignments: CaptureRows::new(capacity.rows[10], operations_successful),
            operation_tokens: CaptureRows::new(capacity.rows[11], successful[11]),
            incoming_actions: CaptureRows::new(capacity.rows[12], successful[12]),
            relation_opinions: CaptureRows::new(capacity.rows[13], relations_successful),
            relation_tuples: CaptureRows::new(capacity.rows[14], relations_successful),
            country_intel: CaptureRows::new(capacity.rows[15], successful[15]),
            intel_scratch: vec![0; capacity.intel_scratch],
            subject_names: Arena::new(capacity.subject_names, successful[1]),
            autonomy_names: Arena::new(capacity.autonomy_names, successful[2]),
            variable_entries: Arena::new(capacity.variable_entries, successful[8]),
            variable_names: Arena::new(capacity.variable_names, successful[8]),
            tokens: Arena::new(capacity.tokens, successful[11]),
            flag_scratch: vec![cscript_flag::Row::default(); capacity.rows[7]],
            relations_raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.capitulation.reset();
        self.subjects.reset();
        self.autonomy.reset();
        self.agencies.reset();
        self.operatives.reset();
        self.cryptology.reset();
        self.networks.reset();
        self.flags.reset();
        self.variables.reset();
        self.operations.reset();
        self.assignments.reset();
        self.operation_tokens.reset();
        self.incoming_actions.reset();
        self.relation_opinions.reset();
        self.relation_tuples.reset();
        self.country_intel.reset();
        self.subject_names.reset();
        self.autonomy_names.reset();
        self.variable_entries.reset();
        self.variable_names.reset();
        self.tokens.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.capitulation.allocated_bytes()
            + self.subjects.allocated_bytes()
            + self.autonomy.allocated_bytes()
            + self.agencies.allocated_bytes()
            + self.operatives.allocated_bytes()
            + self.cryptology.allocated_bytes()
            + self.networks.allocated_bytes()
            + self.flags.allocated_bytes()
            + self.variables.allocated_bytes()
            + self.operations.allocated_bytes()
            + self.assignments.allocated_bytes()
            + self.operation_tokens.allocated_bytes()
            + self.incoming_actions.allocated_bytes()
            + self.relation_opinions.allocated_bytes()
            + self.relation_tuples.allocated_bytes()
            + self.country_intel.allocated_bytes()
            + self.intel_scratch.capacity()
            + self.subject_names.allocated_bytes()
            + self.autonomy_names.allocated_bytes()
            + self.variable_entries.allocated_bytes()
            + self.variable_names.allocated_bytes()
            + self.tokens.allocated_bytes()
            + self.flag_scratch.capacity() * size_of::<cscript_flag::Row>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        let operations_successful = successful[9].min(successful[10]);
        let relations_successful = successful[13].min(successful[14]);
        self.capitulation.grow(&mut capacity.rows[0], successful[0]);
        self.subjects.grow(&mut capacity.rows[1], successful[1]);
        self.autonomy.grow(&mut capacity.rows[2], successful[2]);
        self.agencies.grow(&mut capacity.rows[3], successful[3]);
        self.operatives.grow(&mut capacity.rows[4], successful[4]);
        self.cryptology.grow(&mut capacity.rows[5], successful[5]);
        self.networks.grow(&mut capacity.rows[6], successful[6]);
        self.flags.grow(&mut capacity.rows[7], successful[7]);
        self.variables.grow(&mut capacity.rows[8], successful[8]);
        self.operations
            .grow(&mut capacity.rows[9], operations_successful);
        self.assignments
            .grow(&mut capacity.rows[10], operations_successful);
        self.operation_tokens
            .grow(&mut capacity.rows[11], successful[11]);
        self.incoming_actions
            .grow(&mut capacity.rows[12], successful[12]);
        self.relation_opinions
            .grow(&mut capacity.rows[13], relations_successful);
        self.relation_tuples
            .grow(&mut capacity.rows[14], relations_successful);
        self.country_intel.grow(&mut capacity.rows[15], successful[15]);
        // The intel scratch is a fixed copy window, not a row capacity: it bounds
        // how many 0x20-byte records one bulk copy moves, and a country longer
        // than the window is copied in several passes rather than truncated. It
        // therefore never grows with observed row counts.
        self.subject_names
            .grow(&mut capacity.subject_names, successful[1]);
        self.autonomy_names
            .grow(&mut capacity.autonomy_names, successful[2]);
        self.variable_entries
            .grow(&mut capacity.variable_entries, successful[8]);
        self.variable_names
            .grow(&mut capacity.variable_names, successful[8]);
        self.tokens.grow(&mut capacity.tokens, successful[11]);
        grow_vec(&mut self.flag_scratch, capacity.rows[7]);
    }

    /// No allocation, encoding, token-name resolution, engine calls or I/O.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
        hour: i32,
        force_full: bool,
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        let mut matched = [false; N];
        let mut ticks = [0; N];
        macro_rules! plain {
            ($index:literal, $field:ident, $walker:path) => {{
                let begin = qpc();
                matched[$index] = capture_rows(&mut self.$field, needed[$index], |rows| {
                    $walker(memory, base, state, rows)
                });
                ticks[$index] = elapsed(begin);
            }};
        }
        plain!(0, capitulation, cdiplomacy_status::walk_capitulation);

        let begin = qpc();
        matched[1] = capture_named(
            &mut self.subjects,
            &mut self.subject_names,
            needed[1],
            |rows, names, second| {
                let result = if second {
                    csubject_relation::walk_matrix(memory, base, state, rows, names)
                } else {
                    csubject_relation::walk(memory, base, state, rows, names)
                };
                ArenaRead::from(result)
            },
        );
        ticks[1] = elapsed(begin);
        let begin = qpc();
        matched[2] = capture_named(
            &mut self.autonomy,
            &mut self.autonomy_names,
            needed[2],
            |rows, names, _| {
                ArenaRead::from(ccurrent_autonomy_status::walk(
                    memory, base, state, rows, names,
                ))
            },
        );
        ticks[2] = elapsed(begin);
        plain!(3, agencies, cintelligence_agency::walk);
        plain!(4, operatives, coperative_leader::walk);
        plain!(5, cryptology, ccryptology::walk);
        plain!(6, networks, ccountry_intel_network::walk);

        let begin = qpc();
        matched[7] = capture_rows(&mut self.flags, needed[7], |rows| {
            walk_flags(memory, base, state, rows, &mut self.flag_scratch)
        });
        ticks[7] = elapsed(begin);

        let begin = qpc();
        matched[8] = capture_variables(
            &mut self.variables,
            &mut self.variable_entries,
            &mut self.variable_names,
            needed[8],
            |scopes, entries, names| {
                cvariables::walk_scopes(memory, base, state, scopes, entries, names)
            },
        );
        ticks[8] = elapsed(begin);

        let begin = qpc();
        let first = ccountry_operation_manager::walk(
            memory,
            base,
            state,
            &mut self.operations.rows,
            &mut self.assignments.rows,
        );
        self.operations.outcome = bounded(first.outcome, self.operations.rows.len());
        self.assignments.outcome = bounded(first.assignments, self.assignments.rows.len());
        if needed[9] || needed[10] {
            let second = ccountry_operation_manager::walk(
                memory,
                base,
                state,
                &mut self.operations.second,
                &mut self.assignments.second,
            );
            if needed[9] {
                self.operations.checked = true;
                matched[9] = self
                    .operations
                    .compare(bounded(second.outcome, self.operations.second.len()));
            }
            if needed[10] {
                self.assignments.checked = true;
                matched[10] = self
                    .assignments
                    .compare(bounded(second.assignments, self.assignments.second.len()));
            }
        }
        ticks[9] = elapsed(begin);
        // ticks[10] stays zero because the named group above owns its reads.
        let begin = qpc();
        matched[11] = capture_named(
            &mut self.operation_tokens,
            &mut self.tokens,
            needed[11],
            |rows, tokens, _| {
                let result =
                    ccountry_operation_token_manager::walk(memory, base, state, rows, tokens);
                ArenaRead {
                    outcome: result.outcome,
                    len: result.tokens_len,
                    required: result.required_tokens,
                }
            },
        );
        ticks[11] = elapsed(begin);
        plain!(
            12,
            incoming_actions,
            incoming_diplomatic_action_queue::walk
        );

        let begin = qpc();
        // Tier-4 relations share one matrix pass; off-hours skip the walk.
        // Raw-copy split: on refresh hours the game thread stages raw bytes
        // via `copy_relations_refresh` and the decode worker fills the typed
        // rows before the ready handoff, so both typed passes skip here.
        // Matched flags and costs come from the raw compare, applied by the
        // caller after this returns.
        let relation_walk = replay::cadence::should_walk(
            replay::rows::TableId::RelationOpinions,
            hour,
            force_full,
        ) && !self.relations_raw_mode;
        if !relation_walk && !self.relations_raw_mode {
            let skipped = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            self.relation_opinions.outcome = skipped;
            self.relation_tuples.outcome = skipped;
        } else {
            let first = crelation_status::walk(
                memory,
                base,
                state,
                &mut self.relation_opinions.rows,
                &mut self.relation_tuples.rows,
            );
            self.relation_opinions.outcome =
                bounded(first.opinions, self.relation_opinions.rows.len());
            self.relation_tuples.outcome = bounded(first.tuples, self.relation_tuples.rows.len());
            if needed[13] || needed[14] {
                let second = crelation_status::walk(
                    memory,
                    base,
                    state,
                    &mut self.relation_opinions.second,
                    &mut self.relation_tuples.second,
                );
                if needed[13] {
                    self.relation_opinions.checked = true;
                    matched[13] = self
                        .relation_opinions
                        .compare(bounded(second.opinions, self.relation_opinions.second.len()));
                }
                if needed[14] {
                    self.relation_tuples.checked = true;
                    matched[14] = self
                        .relation_tuples
                        .compare(bounded(second.tuples, self.relation_tuples.second.len()));
                }
            }
        }
        ticks[13] = if relation_walk { elapsed(begin) } else { 0 };
        // ticks[14] stays zero because the named group above owns its reads.

        let begin = qpc();
        matched[15] = capture_rows(&mut self.country_intel, needed[15], |rows| {
            ccountry_intel::walk(memory, base, state, rows, &mut self.intel_scratch)
        });
        ticks[15] = elapsed(begin);
        (matched, ticks)
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            effective(&self.capitulation),
            effective(&self.subjects),
            effective(&self.autonomy),
            effective(&self.agencies),
            effective(&self.operatives),
            effective(&self.cryptology),
            effective(&self.networks),
            effective(&self.flags),
            effective(&self.variables),
            effective(&self.operations),
            effective(&self.assignments),
            effective(&self.operation_tokens),
            effective(&self.incoming_actions),
            effective(&self.relation_opinions),
            effective(&self.relation_tuples),
            effective(&self.country_intel),
        ]
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.capitulation.mismatches,
            self.subjects.mismatches,
            self.autonomy.mismatches,
            self.agencies.mismatches,
            self.operatives.mismatches,
            self.cryptology.mismatches,
            self.networks.mismatches,
            self.flags.mismatches,
            self.variables.mismatches,
            self.operations.mismatches,
            self.assignments.mismatches,
            self.operation_tokens.mismatches,
            self.incoming_actions.mismatches,
            self.relation_opinions.mismatches,
            self.relation_tuples.mismatches,
            self.country_intel.mismatches,
        ]
    }

    /// Called only on the spool thread; this owns all Vec and string conversion.
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            table::<tables::Capitulation, _>(outcomes[0], &self.capitulation.rows, |r| {
                rows::CapitulationRow {
                    country: r.country,
                    capitulated: r.capitulated,
                    capitulated_hour: r.capitulated_hour,
                    last_surrender_hour: r.last_surrender_hour,
                    faction_join_hour: r.faction_join_hour,
                    legitimacy_raw: r.legitimacy_raw,
                    exile_army_leaders: r.exile_army_leaders,
                }
            }),
            self.subjects_table(outcomes[1]),
            self.autonomy_table(outcomes[2]),
            table::<tables::IntelligenceAgencies, _>(outcomes[3], &self.agencies.rows, |r| {
                rows::IntelligenceAgencyRow {
                    country: r.country,
                    upgrade_progress_raw: r.upgrade_progress_raw,
                    defense_raw: r.defense_raw,
                    active_count: r.active_count,
                    admission_ceiling: r.admission_ceiling,
                    unknown_f0: r.unknown_f0,
                }
            }),
            table::<tables::OperativeMemberships, _>(outcomes[4], &self.operatives.rows, |r| {
                rows::OperativeMembershipRow {
                    id: r.id,
                    owner_country: r.owner_country,
                    roster_country: r.roster_country,
                    roster_kind: r.roster_kind,
                    detected: r.detected,
                    assignment: r.assignment,
                    experience_raw: r.experience_raw,
                    skill: r.skill,
                    capturer_country: r.capturer_country,
                    captured_hour: r.captured_hour,
                    action_variant: r.action_variant,
                }
            }),
            table::<tables::Cryptology, _>(outcomes[5], &self.cryptology.rows, |r| {
                rows::CryptologyRow {
                    owner_country: r.owner_country,
                    target_country: r.target_country,
                    amount_raw: r.amount_raw,
                    bonus_days: r.bonus_days,
                    bonus_active: r.bonus_active,
                    decrypting: r.decrypting,
                }
            }),
            table::<tables::IntelNetworks, _>(outcomes[6], &self.networks.rows, |r| {
                rows::IntelNetworkRow {
                    owner_country: r.owner_country,
                    target_country: r.target_country,
                    state_id: r.state_id,
                    strength_raw: r.strength_raw,
                    subnetwork_selector: r.subnetwork_selector,
                }
            }),
            self.flags_table(outcomes[7]),
            self.variables_table(outcomes[8]),
            self.operations_table(outcomes[9]),
            table::<tables::OperationAssignments, _>(outcomes[10], &self.assignments.rows, |r| {
                rows::OperationAssignmentRow {
                    operation: r.operation,
                    ordinal: r.ordinal,
                    operative: r.operative,
                    selection: r.selection,
                }
            }),
            self.operation_tokens_table(outcomes[11]),
            table::<tables::IncomingDiplomaticActions, _>(
                outcomes[12],
                &self.incoming_actions.rows,
                |r| rows::IncomingDiplomaticActionRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    handle: r.handle,
                    status: r.status,
                    action_type: r.action_type,
                    actor: r.actor,
                    original_actor: r.original_actor,
                    recipient: r.recipient,
                    original_recipient: r.original_recipient,
                    hour: r.hour,
                    last_command_hour: r.last_command_hour,
                },
            ),
            table::<tables::RelationOpinions, _>(outcomes[13], &self.relation_opinions.rows, |r| {
                rows::RelationOpinionRow {
                    owner_country: r.owner_country,
                    target_country: r.target_country,
                    opinion: r.opinion,
                }
            }),
            self.relation_tuples_table(outcomes[14]),
            table::<tables::CountryIntel, _>(outcomes[15], &self.country_intel.rows, |r| {
                rows::CountryIntelRow {
                    owner_country: r.owner_country,
                    target_index: r.target_index,
                    civilian: r.civilian,
                    army: r.army,
                    navy: r.navy,
                    air: r.air,
                }
            }),
        ]
    }
}

impl Captured {
    fn subjects_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.subjects.rows,
            &self.subject_names.first,
            self.subject_names.len,
            self.subject_names.required,
        ) {
            return unavailable("subject name lies outside captured arena");
        }
        table::<tables::Subjects, _>(outcome, &self.subjects.rows, |r| rows::SubjectRow {
            overlord: r.overlord,
            subject: r.subject,
            start_hour: r.start_hour,
            end_hour: r.end_hour,
            cancel: r.cancel,
            autonomy_ratio: r.autonomy_ratio,
            state: literal(&self.subject_names.first, r.state).to_vec(),
        })
    }

    fn autonomy_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.autonomy.rows,
            &self.autonomy_names.first,
            self.autonomy_names.len,
            self.autonomy_names.required,
        ) {
            return unavailable("autonomy name lies outside captured arena");
        }
        table::<tables::Autonomy, _>(outcome, &self.autonomy.rows, |r| rows::AutonomyRow {
            subject: r.subject,
            score: r.score,
            lower_score: r.lower_score,
            upper_score: r.upper_score,
            last_modified_bits: r.last_modified_bits,
            state: literal(&self.autonomy_names.first, r.state).to_vec(),
            previous: r
                .previous
                .map(|name| literal(&self.autonomy_names.first, name).to_vec()),
            next: r
                .next
                .map(|name| literal(&self.autonomy_names.first, name).to_vec()),
        })
    }

    fn flags_table(&self, outcome: Outcome) -> TableView {
        table::<tables::ScriptFlags, _>(outcome, &self.flags.rows, |r| rows::ScriptFlagRow {
            scope_kind: r.scope_kind,
            scope_id: r.scope_id,
            token: r.flag.token,
            set_hour: r.flag.set_hour,
            value: r.flag.value,
            remaining_days: r.flag.remaining_days,
        })
    }

    fn variables_table(&self, outcome: Outcome) -> TableView {
        if !valid_variables(
            outcome,
            &self.variables.rows,
            &self.variable_entries,
            &self.variable_names,
        ) {
            return unavailable("script variable entry or name lies outside captured arena");
        }
        let view = table::<tables::ScriptVariables, _>(outcome, &self.variables.rows, |r| {
            let (scope_kind, scope_id) = scope_identity(r.scope);
            let mut entries: Vec<_> = self.variable_entries.first
                [r.entries_start..r.entries_start + r.entries_len]
                .iter()
                .map(|entry| rows::ScriptVariableEntry {
                    name: self.variable_names.first
                        [entry.name_start..entry.name_start + entry.name_len]
                        .to_vec(),
                    value_raw: entry.value_raw,
                })
                .collect();
            entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
            rows::ScriptVariablesRow {
                scope_kind,
                scope_id,
                entries,
            }
        });
        let values = match &view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows,
            _ => return view,
        };
        if values
            .rows::<tables::ScriptVariables>()
            .is_some_and(|rows| {
                rows.iter().any(|row| {
                    row.entries
                        .windows(2)
                        .any(|pair| pair[0].name == pair[1].name)
                })
            })
        {
            return unavailable("duplicate literal script variable key");
        }
        view
    }

    fn operations_table(&self, outcome: Outcome) -> TableView {
        table::<tables::Operations, _>(outcome, &self.operations.rows, |r| rows::OperationRow {
            id: r.id,
            owner: r.owner,
            target: r.target,
            target_province: r.target_province,
            definition_token: r.definition_token,
            preparation_current: r.preparation_current,
            preparation_required: r.preparation_required,
            flag_40: r.flag_40,
            auto_repeat: r.auto_repeat,
            terminal: r.terminal,
            start_hour: r.start_hour,
            duration_days: r.duration_days,
            phase_count: r.phase_count,
            assigned_slots: r.assigned_slots,
        })
    }

    /// The wire ordinal is the native serializable-vector index, so a native
    /// vector longer than the wire width cannot be represented and refuses the
    /// table rather than aliasing two relations onto one key.
    fn relation_tuples_table(&self, outcome: Outcome) -> TableView {
        if self.relation_tuples.rows[..outcome.len()]
            .iter()
            .any(|row| u16::try_from(row.ordinal).is_err())
        {
            return unavailable("relation tuple ordinal exceeds its wire width");
        }
        table::<tables::RelationTuples, _>(outcome, &self.relation_tuples.rows, |r| {
            rows::RelationTupleRow {
                owner_country: r.owner_country,
                target_country: r.target_country,
                type_id: r.type_id,
                ordinal: r.ordinal as u16,
                first_country: r.first_country,
                second_country: r.second_country,
                start_hour: r.start_hour,
                end_hour: r.end_hour,
                cancel: r.cancel,
                retired: r.retired,
            }
        })
    }

    fn operation_tokens_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.operation_tokens.rows,
            &self.tokens.first,
            self.tokens.len,
            self.tokens.required,
        ) {
            return unavailable("operation token vector lies outside captured arena");
        }
        table::<tables::OperationTokens, _>(outcome, &self.operation_tokens.rows, |r| {
            rows::OperationTokensRow {
                owner: r.owner,
                target: r.target,
                tokens: self.tokens.first[r.tokens_start..r.tokens_start + r.tokens_len].to_vec(),
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

fn effective<R>(captured: &CaptureRows<R>) -> Outcome {
    if captured.checked && captured.mismatches != 0 {
        Outcome::Unavailable {
            reason: "table changed during double read",
        }
    } else {
        captured.outcome
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

#[derive(Clone, Copy)]
struct ArenaRead {
    outcome: Outcome,
    len: usize,
    required: usize,
}

impl From<csubject_relation::NameWalk> for ArenaRead {
    fn from(value: csubject_relation::NameWalk) -> Self {
        Self {
            outcome: value.outcome,
            len: value.bytes_len,
            required: value.required_bytes,
        }
    }
}

/// Equality compares the native fields and literal payload, never arena offsets.
trait ArenaRow<T>: Default + Clone + PartialEq {
    fn key(&self) -> u128;
    fn valid(&self, len: usize) -> bool;
    fn equivalent(&self, other: &Self, first: &[T], second: &[T]) -> bool;
}

fn fits(start: usize, count: usize, len: usize) -> bool {
    start.checked_add(count).is_some_and(|end| end <= len)
}

fn name_fits(name: Name, len: usize) -> bool {
    fits(name.start, name.len, len)
}

fn literal(bytes: &[u8], name: Name) -> &[u8] {
    &bytes[name.start..name.start + name.len]
}

fn optional_name_equal(a: Option<Name>, b: Option<Name>, first: &[u8], second: &[u8]) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => literal(first, a) == literal(second, b),
        _ => false,
    }
}

impl ArenaRow<u8> for csubject_relation::Row {
    fn key(&self) -> u128 {
        (u128::from(self.overlord) << 32) | u128::from(self.subject)
    }
    fn valid(&self, len: usize) -> bool {
        name_fits(self.state, len)
    }
    fn equivalent(&self, other: &Self, first: &[u8], second: &[u8]) -> bool {
        let mut adjusted = *other;
        adjusted.state = self.state;
        *self == adjusted && literal(first, self.state) == literal(second, other.state)
    }
}

impl ArenaRow<u8> for ccurrent_autonomy_status::Row {
    fn key(&self) -> u128 {
        u128::from(self.subject)
    }
    fn valid(&self, len: usize) -> bool {
        name_fits(self.state, len)
            && self.previous.is_none_or(|name| name_fits(name, len))
            && self.next.is_none_or(|name| name_fits(name, len))
    }
    fn equivalent(&self, other: &Self, first: &[u8], second: &[u8]) -> bool {
        let mut adjusted = *other;
        adjusted.state = self.state;
        adjusted.previous = self.previous;
        adjusted.next = self.next;
        *self == adjusted
            && literal(first, self.state) == literal(second, other.state)
            && optional_name_equal(self.previous, other.previous, first, second)
            && optional_name_equal(self.next, other.next, first, second)
    }
}

impl ArenaRow<u32> for ccountry_operation_token_manager::Row {
    fn key(&self) -> u128 {
        (u128::from(self.owner) << 32) | u128::from(self.target)
    }
    fn valid(&self, len: usize) -> bool {
        fits(self.tokens_start, self.tokens_len, len)
    }
    fn equivalent(&self, other: &Self, first: &[u32], second: &[u32]) -> bool {
        let mut adjusted = *other;
        adjusted.tokens_start = self.tokens_start;
        adjusted.tokens_len = self.tokens_len;
        *self == adjusted
            && first[self.tokens_start..self.tokens_start + self.tokens_len]
                == second[other.tokens_start..other.tokens_start + other.tokens_len]
    }
}

fn valid_arena<R: ArenaRow<T>, T>(
    outcome: Outcome,
    rows: &[R],
    values: &[T],
    len: usize,
    required: usize,
) -> bool {
    len <= values.len()
        && required >= len
        && rows
            .get(..outcome.len())
            .is_some_and(|rows| rows.iter().all(|row| row.valid(len)))
}

fn arena_outcome<R: ArenaRow<T>, T>(result: ArenaRead, rows: &[R], values: &[T]) -> Outcome {
    let outcome = bounded(result.outcome, rows.len());
    if valid_arena(outcome, rows, values, result.len, result.required) {
        outcome
    } else {
        Outcome::Unavailable {
            reason: "captured arena range invalid",
        }
    }
}

fn first_named<R: ArenaRow<T>, T: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    arena: &mut Arena<T>,
    first: ArenaRead,
) {
    captured.outcome = arena_outcome(first, &captured.rows, &arena.first);
    arena.len = first.len;
    arena.required = first.required;
}

fn second_named<R: ArenaRow<T>, T: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    arena: &mut Arena<T>,
    second: ArenaRead,
) -> bool {
    captured.checked = true;
    let outcome = arena_outcome(second, &captured.second, &arena.second);
    if let (Outcome::Rows { len }, Outcome::Rows { len: second_len }) = (captured.outcome, outcome)
    {
        captured.rows[..len].sort_unstable_by_key(|row| row.key());
        captured.second[..second_len].sort_unstable_by_key(|row| row.key());
        captured.mismatches = usize::from(
            len != second_len || arena.len != second.len || arena.required != second.required,
        ) + captured.rows[..len]
            .iter()
            .zip(&captured.second[..second_len])
            .filter(|(first, other)| !first.equivalent(other, &arena.first, &arena.second))
            .count();
        if captured.rows[..len]
            .windows(2)
            .any(|r| r[0].key() == r[1].key())
            || captured.second[..second_len]
                .windows(2)
                .any(|r| r[0].key() == r[1].key())
        {
            captured.mismatches += 1;
        }
    } else {
        // Different partial censuses need not contain the same prefix.
        captured.mismatches = usize::from(
            matches!(captured.outcome, Outcome::Rows { .. })
                || matches!(outcome, Outcome::Rows { .. }),
        );
    }
    arena.required = arena.required.max(second.required);
    full_match(captured.outcome, outcome, captured.mismatches)
}

fn capture_named<R: ArenaRow<T>, T: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    arena: &mut Arena<T>,
    needed: bool,
    mut walk: impl FnMut(&mut [R], &mut [T], bool) -> ArenaRead,
) -> bool {
    let first = walk(&mut captured.rows, &mut arena.first, false);
    first_named(captured, arena, first);
    if !needed {
        return false;
    }
    let second = walk(&mut captured.second, &mut arena.second, true);
    second_named(captured, arena, second)
}

fn scope_identity(scope: cvariables::Scope) -> (u8, u32) {
    match scope {
        cvariables::Scope::Global => (GLOBAL_SCOPE, 0),
        cvariables::Scope::Country(id) => (COUNTRY_SCOPE, id),
        cvariables::Scope::State(id) => (STATE_SCOPE, id),
    }
}

fn capture_variables(
    captured: &mut CaptureRows<cvariables::ScopeRow>,
    entries: &mut Arena<cvariables::Row>,
    names: &mut Arena<u8>,
    needed: bool,
    mut walk: impl FnMut(
        &mut [cvariables::ScopeRow],
        &mut [cvariables::Row],
        &mut [u8],
    ) -> cvariables::ScopeWalk,
) -> bool {
    let first = walk(&mut captured.rows, &mut entries.first, &mut names.first);
    captured.outcome = variable_outcome(first, &captured.rows, &entries.first, &names.first);
    entries.len = first.entries_len;
    entries.required = first.required_entries;
    names.len = first.bytes_len;
    names.required = first.required_bytes;
    if !needed {
        return false;
    }
    let second = walk(&mut captured.second, &mut entries.second, &mut names.second);
    captured.checked = true;
    let outcome = variable_outcome(second, &captured.second, &entries.second, &names.second);
    captured.compare(outcome);
    // Both reads independently enumerate native slots into separate arenas.
    // Compare every literal byte and native value, not merely arena lengths.
    if matches!(captured.outcome, Outcome::Rows { .. }) && matches!(outcome, Outcome::Rows { .. }) {
        captured.mismatches +=
            usize::from(!entries.compare(second.entries_len, second.required_entries));
        captured.mismatches += usize::from(!names.compare(second.bytes_len, second.required_bytes));
    } else {
        entries.required = entries.required.max(second.required_entries);
        names.required = names.required.max(second.required_bytes);
    }
    full_match(captured.outcome, outcome, captured.mismatches)
}

fn variable_outcome(
    read: cvariables::ScopeWalk,
    rows: &[cvariables::ScopeRow],
    entries: &[cvariables::Row],
    names: &[u8],
) -> Outcome {
    if valid_variable_parts(read, rows, entries, names) {
        read.outcome
    } else {
        Outcome::Unavailable {
            reason: "script variable entry or name lies outside captured arena",
        }
    }
}

fn valid_variables(
    outcome: Outcome,
    rows: &[cvariables::ScopeRow],
    entries: &Arena<cvariables::Row>,
    names: &Arena<u8>,
) -> bool {
    valid_variable_parts(
        cvariables::ScopeWalk {
            outcome,
            entries_len: entries.len,
            required_entries: entries.required,
            bytes_len: names.len,
            required_bytes: names.required,
        },
        rows,
        &entries.first,
        &names.first,
    )
}

fn valid_variable_parts(
    read: cvariables::ScopeWalk,
    rows: &[cvariables::ScopeRow],
    entries: &[cvariables::Row],
    names: &[u8],
) -> bool {
    read.entries_len <= entries.len()
        && read.required_entries >= read.entries_len
        && read.bytes_len <= names.len()
        && read.required_bytes >= read.bytes_len
        && rows.get(..read.outcome.len()).is_some_and(|rows| {
            rows.iter().all(|row| {
                fits(row.entries_start, row.entries_len, read.entries_len)
                    && entries[row.entries_start..row.entries_start + row.entries_len]
                        .iter()
                        .all(|entry| fits(entry.name_start, entry.name_len, read.bytes_len))
            })
        })
}

/// Proven expiry callers dereference all three manager roots. Slot zero and
/// null state holes are not scopes; canonical country aliases are visited once.
fn walk_flags(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [ScopedFlag],
    scratch: &mut [cscript_flag::Row],
) -> Outcome {
    let result = (|| {
        let mut len = 0;
        let mut unread = 0_u32;
        let mut scope = |kind, id, root, offset| -> Result<(), &'static str> {
            let manager = pointer(memory, root, offset)?;
            if manager == 0 {
                return Err("script flag manager root is null");
            }
            let available = (output.len() - len).min(scratch.len());
            let outcome = bounded(
                cflag_manager::walk(memory, base, manager, &mut scratch[..available]),
                available,
            );
            let count = match outcome {
                Outcome::Rows { len } => len,
                Outcome::Truncated { len, unread_rows } => {
                    unread = unread
                        .checked_add(unread_rows)
                        .ok_or("flag omitted count overflow")?;
                    len
                }
                Outcome::Unavailable { reason } => return Err(reason),
            };
            for flag in &scratch[..count] {
                output[len] = ScopedFlag {
                    scope_kind: kind,
                    scope_id: id,
                    flag: *flag,
                };
                len += 1;
            }
            Ok(())
        };
        scope(GLOBAL_SCOPE, 0, state, cgame_state::GLOBAL_FLAGS)?;
        let countries = ccountry::Reader::new(memory, base, state)?;
        let mut found_country = false;
        for slot in 1..countries.count() {
            if let Some(country) = countries.at(slot)? {
                found_country = true;
                scope(
                    COUNTRY_SCOPE,
                    country.id,
                    country.pointer,
                    ccountry::FLAG_MANAGER,
                )?;
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        let states = state
            .checked_add(cgame_state::STATES)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("state scope array unreadable")?;
        if states.count < 2 {
            return Err("state table has no state slots");
        }
        let state_vtable = base
            .checked_add(cstate::VTABLE_RVA)
            .ok_or("state vtable overflow")?;
        let mut first = 1;
        let mut found_state = false;
        let mut pointers = [0_u8; 512];
        while first < states.count {
            let count = ((states.count - first) as usize).min(64);
            let address = states
                .element(first, 8)
                .ok_or("state scope pointer overflow")?;
            if !read_slice(memory, address, &mut pointers[..count * 8]) {
                return Err("state scope pointer array unreadable");
            }
            for (index, bytes) in pointers[..count * 8].chunks_exact(8).enumerate() {
                let at = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                    .map_err(|_| "state scope pointer overflow")?;
                if at == 0 {
                    continue;
                }
                found_state = true;
                if read::<u64>(memory, at) != Some(state_vtable as u64) {
                    return Err("state scope vtable mismatch");
                }
                let id = at
                    .checked_add(cstate::ID)
                    .and_then(|p| read::<u32>(memory, p))
                    .ok_or("state scope id unreadable")?;
                if id != first + index as u32 {
                    return Err("state scope id differs from slot");
                }
                scope(STATE_SCOPE, id, at, cstate::FLAGS)?;
            }
            first += count as u32;
        }
        if !found_state {
            return Err("state table has no readable states");
        }
        Ok(if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn pointer(memory: &impl Memory, root: usize, offset: usize) -> Result<usize, &'static str> {
    root.checked_add(offset)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("script scope root unreadable")
}

// ===========================================================================
// Relations copy/decode split (family 1: opinions + tuples).
//
// Game thread (`Captured::copy_relations_refresh`): raw copies only. It uses
// the factored engine `copy_*` helpers (`copy_slots`, `copy_auth`,
// `copy_rest`) plus plain roster-structure reads (country enumeration,
// diplomacy pointers, matrix/serializable headers, pointer pages) to discover
// what to copy. No vtable, backlink, target, type, ownership or ordering
// validation happens here; discovery interprets only already-copied bytes to
// find the next spans (serializable headers for vector bounds, auth bytes
// best-effort for rest-copy gating). Every span is recorded with its success
// flag, so decode reproduces identical faults with no live access. Rest bytes
// are copied for every serializable element (including skipped war/subject);
// decode ignores rest for skipped elements exactly like the typed walk, which
// never reads them.
//
// Decode worker (`Captured::decode_relations_payload`): replays the same
// validations as `crelation_status::walk` through the factored `decode_*`
// helpers (`decode_slot`, `decode_auth`, `decode_rest`,
// `Array::from_header`) into the identical typed rows and outcomes.
// Unavailable outcomes discard rows exactly like the typed walk, so any
// recorded fault short-circuits to the same reason.
//
// Double reads: the game thread takes two raw copies and compares them
// partition by partition (opinions: diplomacy/matrix/slot spans; tuples:
// tuple auth/rest spans plus slot headers they derive from). Equal decodes
// once; mismatch takes the existing mismatch path (checked/mismatches on the
// same `CaptureRows`, matched stays false). The 200-check windows still count
// refreshes only: skip hours never reach the compare and matched stays false.
// Conservative rule: any live failure inside a partition blocks that
// partition's check even when both copies agree, so same-hour tearing never
// advances the windows. A readable-but-invalid refresh (deterministic fault,
// both copies equal) still counts its check while emitting the identical
// Unavailable outcome; such faults do not occur with the pinned build and
// live calibration will confirm.
// ===========================================================================

/// Frozen country identity for decode-side resolution without live access.
/// Built once per pass from the game-thread country enumeration, which the
/// typed walk also performs; decode looks up targets/endpoints by raw id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CountrySnap {
    pub slot: u32,
    pub pointer: usize,
    pub id: u32,
    pub canonical_slot: u32,
}

/// One serializable relation element in walk order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TupleOcc {
    pub owner_id: u32,
    pub owner_canonical: u32,
    pub target_country: u32,
    pub ordinal: u32,
    pub auth: [u8; crelation_status::TUPLE_AUTH_BYTES],
    pub auth_ok: bool,
    pub rest: [u8; crelation_status::TUPLE_REST_BYTES],
    pub rest_ok: bool,
}

/// Fixed-size per-owner copy metadata. Blobs live in the job-level pools
/// addressed by the start/len ranges below.
#[derive(Debug, Default)]
pub(crate) struct OwnerMeta {
    pub owner_slot: u32,
    pub owner_id: u32,
    pub owner_canonical: u32,
    pub owner_pointer: usize,
    pub diplomacy_pointer: usize,
    pub diplomacy_vtable: [u8; 8],
    pub diplomacy_backlink: [u8; 8],
    pub diplomacy_scattered: usize,
    pub diplomacy_err: Option<&'static str>,
    pub matrix_header: [u8; 16],
    pub matrix_header_ok: bool,
    pub matrix_err: Option<&'static str>,
    pub slot_start: usize,
    pub slot_len: usize,
    pub tuple_start: usize,
    pub tuple_len: usize,
}

/// One refresh of raw relations payload. Flat pools keep every per-item struct
/// fixed-size so `clear` recycles capacities and the steady state allocates
/// on neither thread; growth happens on the decode side only (see
/// `crate::decode::recycle_relations_job`).
#[derive(Debug)]
pub(crate) struct RelationsJob {
    pub countries: Vec<CountrySnap>,
    pub highest_slot: u32,
    pub count: u32,
    pub metas: Vec<OwnerMeta>,
    pub slot_pool: Vec<u8>,
    pub tuples: Vec<TupleOcc>,
    pub reader_err: Option<&'static str>,
    pub country_err: Option<&'static str>,
    pub top_err: Option<&'static str>,
    pub unread_opinions: u32,
    pub unread_tuples: u32,
    pub overflowed: bool,
}

impl RelationsJob {
    pub fn new() -> Self {
        Self {
            countries: Vec::with_capacity(512),
            highest_slot: 0,
            count: 0,
            metas: Vec::with_capacity(512),
            slot_pool: Vec::with_capacity(8 << 20),
            tuples: Vec::with_capacity(8192),
            reader_err: None,
            country_err: None,
            top_err: None,
            unread_opinions: 0,
            unread_tuples: 0,
            overflowed: false,
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.countries.clear();
        self.highest_slot = 0;
        self.count = 0;
        self.metas.clear();
        self.slot_pool.clear();
        self.tuples.clear();
        self.reader_err = None;
        self.country_err = None;
        self.top_err = None;
        self.unread_opinions = 0;
        self.unread_tuples = 0;
        self.overflowed = false;
    }

    pub fn grow_for_next(&self) -> bool {
        self.overflowed
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed refreshes fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        fn double<T>(rows: &mut Vec<T>) {
            rows.reserve_exact(rows.capacity().max(1));
        }
        double(&mut self.countries);
        double(&mut self.metas);
        double(&mut self.slot_pool);
        double(&mut self.tuples);
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RelationsCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Per-partition raw equality gated by live-fault conservatism, in
    /// [opinions, tuples] order.
    pub matched: [bool; 2],
    pub copy_ticks: [u64; 2],
    pub raw_bytes: [u64; 2],
}

/// Game-thread copy parameters, bundled to keep the refresh entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RelationsCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub opinion_cap: usize,
    pub tuple_cap: usize,
    pub needed: [bool; 2],
}

#[derive(Debug, Default)]
struct RelationsTallies {
    ticks: [u64; 2],
    bytes: [u64; 2],
    live_fault: [bool; 2],
}

fn relations_fit_len(used: usize, capacity: usize, need: usize) -> bool {
    need <= capacity.saturating_sub(used)
}

fn relations_lookup(countries: &[CountrySnap], raw: u32) -> Result<CountrySnap, &'static str> {
    if raw == 0 {
        return Err("country id is zero");
    }
    countries
        .iter()
        .find(|entry| entry.id == raw)
        .copied()
        .ok_or("country link unreadable or outside country table")
}

fn copy_relations_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    opinion_cap: usize,
    tuple_cap: usize,
    job: &mut RelationsJob,
) -> RelationsTallies {
    job.clear();
    let mut tallies = RelationsTallies::default();
    let status_vtable = match base.checked_add(crelation_status::VTABLE_RVA) {
        Some(address) => address as u64,
        None => {
            job.top_err = Some("module address overflow");
            return tallies;
        }
    };
    let _ = status_vtable;
    let countries = match ccountry::Reader::new(memory, base, game_state) {
        Err(reason) => {
            job.reader_err = Some(reason);
            return tallies;
        }
        Ok(countries) => countries,
    };
    let count = countries.count();
    job.count = count;
    // Highest-slot census mirrors the typed walk: enumeration faults abort
    // the whole pass with the identical reason.
    let mut highest_slot = 0u32;
    for slot in 1..count {
        let started = qpc();
        match countries.at(slot) {
            Err(reason) => {
                job.country_err = Some(reason);
                tallies.ticks[0] += elapsed(started);
                return tallies;
            }
            Ok(Some(country)) => {
                highest_slot = slot;
                if job.countries.len() == job.countries.capacity() {
                    job.overflowed = true;
                    return tallies;
                }
                job.countries.push(CountrySnap {
                    slot,
                    pointer: country.pointer,
                    id: country.id,
                    canonical_slot: country.canonical_slot,
                });
            }
            Ok(None) => {}
        }
        tallies.ticks[0] += elapsed(started);
    }
    job.highest_slot = highest_slot;
    if job.countries.is_empty() {
        job.top_err = Some("country table has no readable countries");
        return tallies;
    }
    let mut opinion_recorded = 0usize;
    let mut tuple_recorded = 0usize;
    let mut unread_opinions = 0u32;
    let mut unread_tuples = 0u32;
    // Borrow the frozen country list by index (no allocation, no live reads
    // after enumeration); entries are `Copy`.
    let country_len = job.countries.len();
    for ci in 0..country_len {
        let entry = job.countries[ci];
        let started = qpc();
        if job.metas.len() == job.metas.capacity() {
            job.overflowed = true;
            return tallies;
        }
        let mut meta = OwnerMeta {
            owner_slot: entry.slot,
            owner_id: entry.id,
            owner_canonical: entry.canonical_slot,
            owner_pointer: entry.pointer,
            ..OwnerMeta::default()
        };
        // Diplomacy pointer discovery mirrors `diplomacy_root` up to the
        // vtable/backlink identity spans; those spans decode off-thread.
        let diplomacy = match entry
            .pointer
            .checked_add(ccountry::DIPLOMACY_STATUS)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|value| usize::try_from(value).ok())
        {
            None => {
                meta.diplomacy_err = Some("country diplomacy unreadable");
                tallies.live_fault[0] = true;
                tallies.live_fault[1] = true;
                job.metas.push(meta);
                tallies.ticks[0] += elapsed(started);
                continue;
            }
            Some(0) => {
                meta.diplomacy_err = Some("country diplomacy is null");
                tallies.live_fault[0] = true;
                tallies.live_fault[1] = true;
                job.metas.push(meta);
                tallies.ticks[0] += elapsed(started);
                continue;
            }
            Some(pointer) => pointer,
        };
        meta.diplomacy_pointer = diplomacy;
        let scattered = memory.copy_scattered(
            [
                diplomacy,
                diplomacy.saturating_add(cdiplomacy_status::COUNTRY),
            ],
            &mut [&mut meta.diplomacy_vtable, &mut meta.diplomacy_backlink],
        );
        meta.diplomacy_scattered = scattered;
        tallies.bytes[0] += 16;
        if scattered != 2 {
            meta.diplomacy_err = Some("country diplomacy unreadable");
            tallies.live_fault[0] = true;
            tallies.live_fault[1] = true;
            job.metas.push(meta);
            tallies.ticks[0] += elapsed(started);
            continue;
        }
        // Matrix header discovery mirrors `Array::read` bounds; decode
        // revalidates from these bytes with the identical reasons.
        let header_addr = match diplomacy.checked_add(cdiplomacy_status::RELATIONS) {
            Some(address) => address,
            None => {
                meta.matrix_err = Some("relation matrix unreadable or invalid");
                tallies.live_fault[0] = true;
                tallies.live_fault[1] = true;
                job.metas.push(meta);
                tallies.ticks[0] += elapsed(started);
                continue;
            }
        };
        match read::<[u8; 16]>(memory, header_addr) {
            Some(header) => {
                meta.matrix_header = header;
                meta.matrix_header_ok = true;
                tallies.bytes[0] += 16;
            }
            None => {
                meta.matrix_header_ok = false;
                meta.matrix_err = Some("relation matrix unreadable or invalid");
                tallies.live_fault[0] = true;
                tallies.live_fault[1] = true;
                job.metas.push(meta);
                tallies.ticks[0] += elapsed(started);
                continue;
            }
        }
        let matrix = match Array::from_header(meta.matrix_header, 8) {
            Some(matrix) => matrix,
            None => {
                meta.matrix_err = Some("relation matrix unreadable or invalid");
                job.metas.push(meta);
                tallies.ticks[0] += elapsed(started);
                continue;
            }
        };
        if matrix.count <= highest_slot {
            meta.matrix_err = Some("relation matrix does not cover country ordinals");
            job.metas.push(meta);
            tallies.ticks[0] += elapsed(started);
            continue;
        }
        meta.slot_start = job.slot_pool.len();
        meta.tuple_start = job.tuples.len();
        let mut index = 1u32;
        let mut failed = false;
        while index < matrix.count && !failed {
            let len = (matrix.count - index).min(crelation_status::BATCH as u32) as usize;
            let page_addr = match matrix.element(index, 8) {
                Some(address) => address,
                None => {
                    meta.matrix_err = Some("relation matrix pointer address overflow");
                    tallies.live_fault[0] = true;
                    tallies.live_fault[1] = true;
                    break;
                }
            };
            let mut page = [0u8; crelation_status::BATCH * 8];
            if !read_slice(memory, page_addr, &mut page[..len * 8]) {
                meta.matrix_err = Some("relation matrix pointer batch unreadable");
                tallies.live_fault[0] = true;
                tallies.live_fault[1] = true;
                break;
            }
            let mut pointers = [0usize; crelation_status::BATCH];
            for (slot, chunk) in pointers[..len].iter_mut().zip(page[..len * 8].chunks_exact(8)) {
                match usize::try_from(u64::from_le_bytes(chunk.try_into().unwrap())) {
                    Err(_) => {
                        meta.matrix_err = Some("relation matrix pointer overflow");
                        tallies.live_fault[0] = true;
                        tallies.live_fault[1] = true;
                        failed = true;
                        break;
                    }
                    Ok(0) => {
                        meta.matrix_err = Some("null relation matrix entry");
                        tallies.live_fault[0] = true;
                        tallies.live_fault[1] = true;
                        failed = true;
                        break;
                    }
                    Ok(pointer) => *slot = pointer,
                }
            }
            if failed {
                break;
            }
            if !relations_fit_len(
                job.slot_pool.len(),
                job.slot_pool.capacity(),
                len * crelation_status::RECORD_BYTES,
            ) {
                job.overflowed = true;
                return tallies;
            }
            let slot_base = job.slot_pool.len();
            job.slot_pool
                .resize(slot_base + len * crelation_status::RECORD_BYTES, 0);
            let copied = crelation_status::copy_slots(
                memory,
                &pointers[..len],
                &mut job.slot_pool[slot_base..slot_base + len * crelation_status::RECORD_BYTES],
            );
            tallies.bytes[0] += (len * crelation_status::RECORD_BYTES) as u64;
            if copied < len * crelation_status::SLOT_FIELDS.len() {
                meta.matrix_err = Some("relation matrix slot fields unreadable");
                tallies.live_fault[0] = true;
                tallies.live_fault[1] = true;
                // Prefix stays valid: truncate the pool to the completed
                // records so decode sees exactly the recorded prefix.
                let completed = copied / crelation_status::SLOT_FIELDS.len();
                job.slot_pool.truncate(slot_base + completed * crelation_status::RECORD_BYTES);
                break;
            }
            // Tuple census per slot, in native vector order. Serializable
            // headers come from already-copied slot bytes (discovery only).
            for offset in 0..len {
                let record: &[u8; crelation_status::RECORD_BYTES] = job.slot_pool
                    [slot_base + offset * crelation_status::RECORD_BYTES
                        ..slot_base + (offset + 1) * crelation_status::RECORD_BYTES]
                    .try_into()
                    .unwrap();
                let header: [u8; 16] = record[16..].try_into().unwrap();
                let serializable = match Array::from_header(header, 8) {
                    Some(array) => array,
                    None => {
                        meta.matrix_err = Some("serializable relation vector invalid");
                        failed = true;
                        break;
                    }
                };
                // Opinion capacity accounting mirrors `push_opinion`: the
                // opinion filter itself decodes off-thread, so count slots
                // with nonzero opinions here from already-copied bytes.
                let opinion = i32::from_le_bytes(record[12..16].try_into().unwrap());
                if opinion != 0 {
                    if opinion_recorded == opinion_cap {
                        match unread_opinions.checked_add(1) {
                            Some(next) => unread_opinions = next,
                            None => {
                                meta.matrix_err =
                                    Some("relation opinion unread row count overflow");
                                failed = true;
                                break;
                            }
                        }
                    } else {
                        opinion_recorded += 1;
                    }
                }
                if !copy_slot_tuples(
                    memory,
                    base,
                    serializable,
                    entry.id,
                    entry.canonical_slot,
                    u32::from_le_bytes(record[8..12].try_into().unwrap()),
                    tuple_cap,
                    &mut tuple_recorded,
                    &mut unread_tuples,
                    job,
                    &mut meta.matrix_err,
                    &mut tallies,
                ) {
                    failed = true;
                    break;
                }
                if job.overflowed {
                    return tallies;
                }
                if meta.matrix_err.is_some() {
                    failed = true;
                    break;
                }
            }
            index += len as u32;
        }
        meta.slot_len = job.slot_pool.len() - meta.slot_start;
        meta.tuple_len = job.tuples.len() - meta.tuple_start;
        tallies.ticks[0] += elapsed(started);
        job.metas.push(meta);
    }
    job.unread_opinions = unread_opinions;
    job.unread_tuples = unread_tuples;
    tallies
}

#[allow(clippy::too_many_arguments)]
fn copy_slot_tuples(
    memory: &impl Memory,
    base: usize,
    serializable: Array,
    owner_id: u32,
    owner_canonical: u32,
    target_country: u32,
    tuple_cap: usize,
    tuple_recorded: &mut usize,
    unread_tuples: &mut u32,
    job: &mut RelationsJob,
    meta_err: &mut Option<&'static str>,
    tallies: &mut RelationsTallies,
) -> bool {
    let mut first = 0u32;
    while first < serializable.count {
        let chunk = (serializable.count - first).min(crelation_status::BATCH as u32) as usize;
        let first_address = match serializable.element(first, 8) {
            Some(address) => address,
            None => {
                *meta_err = Some("serializable relation entry unreadable");
                tallies.live_fault[1] = true;
                return false;
            }
        };
        let mut pointer_bytes = [0u8; crelation_status::BATCH * 8];
        if !read_slice(memory, first_address, &mut pointer_bytes[..chunk * 8]) {
            *meta_err = Some("serializable relation entry unreadable");
            tallies.live_fault[1] = true;
            return false;
        }
        let mut pointers = [0usize; crelation_status::BATCH];
        for (slot, chunk_bytes) in pointers[..chunk]
            .iter_mut()
            .zip(pointer_bytes[..chunk * 8].chunks_exact(8))
        {
            match usize::try_from(u64::from_le_bytes(chunk_bytes.try_into().unwrap())) {
                Err(_) => {
                    *meta_err = Some("serializable relation entry unreadable");
                    tallies.live_fault[1] = true;
                    return false;
                }
                Ok(0) => {
                    *meta_err = Some("null serializable relation entry");
                    tallies.live_fault[1] = true;
                    return false;
                }
                Ok(pointer) => *slot = pointer,
            }
        }
        // Auth spans first, exactly like the walk: a short gather attributes
        // the identical field-level fault before any rest copy runs.
        let mut auth_records = [0u8; crelation_status::BATCH * crelation_status::TUPLE_AUTH_BYTES];
        let auth_copied = crelation_status::copy_auth(
            memory,
            &pointers[..chunk],
            &mut auth_records[..chunk * crelation_status::TUPLE_AUTH_BYTES],
        );
        tallies.bytes[1] += (chunk * crelation_status::TUPLE_AUTH_BYTES) as u64;
        if auth_copied < chunk * crelation_status::TUPLE_AUTH_FIELDS.len() {
            let failed = auth_copied % crelation_status::TUPLE_AUTH_FIELDS.len();
            let pointer = pointers[auth_copied / crelation_status::TUPLE_AUTH_FIELDS.len()];
            let field = &crelation_status::TUPLE_AUTH_FIELDS[failed];
            if pointer
                .checked_add(field.source_offset)
                .and_then(|address| address.checked_add(field.len))
                .is_none()
            {
                *meta_err = Some("relation field address overflow");
            } else if failed == 0 {
                *meta_err = Some("relation vtable unreadable");
            } else {
                *meta_err = Some("relation type word unreadable");
            }
            tallies.live_fault[1] = true;
            return false;
        }
        // Best-effort auth interpretation on already-copied bytes (discovery
        // only): invalid auth stops rest staging because decode fails at auth
        // first, exactly like the walk; skipped war/subject elements stage no
        // rest because the walk never reads them.
        let mut live_pointers = [0usize; crelation_status::BATCH];
        let mut live_source = [0usize; crelation_status::BATCH];
        let mut live_len = 0usize;
        let mut auth_invalid = false;
        for index in 0..chunk {
            let auth: &[u8; crelation_status::TUPLE_AUTH_BYTES] = auth_records[index
                * crelation_status::TUPLE_AUTH_BYTES
                ..(index + 1) * crelation_status::TUPLE_AUTH_BYTES]
                .try_into()
                .unwrap();
            match crelation_status::decode_auth(base, auth) {
                Err(_) => {
                    auth_invalid = true;
                    break;
                }
                Ok(decoded) if decoded.skipped => {}
                Ok(_) => {
                    live_pointers[live_len] = pointers[index];
                    live_source[live_len] = index;
                    live_len += 1;
                }
            }
        }
        // Rest spans for live elements only, mirroring the walk's gather.
        // Skipped elements stage zeroed rest (ignored by decode); invalid
        // auth stops here because decode fails at auth first.
        let mut rest_records = [0u8; crelation_status::BATCH * crelation_status::TUPLE_REST_BYTES];
        let rest_copied = if live_len > 0 {
            let copied = crelation_status::copy_rest(
                memory,
                &live_pointers[..live_len],
                &mut rest_records[..live_len * crelation_status::TUPLE_REST_BYTES],
            );
            tallies.bytes[1] += (live_len * crelation_status::TUPLE_REST_BYTES) as u64;
            if copied < live_len * crelation_status::TUPLE_REST_FIELDS.len() {
                let failed = copied % crelation_status::TUPLE_REST_FIELDS.len();
                let pointer = live_pointers[copied / crelation_status::TUPLE_REST_FIELDS.len()];
                let field = &crelation_status::TUPLE_REST_FIELDS[failed];
                if pointer
                    .checked_add(field.source_offset)
                    .and_then(|address| address.checked_add(field.len))
                    .is_none()
                {
                    *meta_err = Some("relation field address overflow");
                } else {
                    *meta_err = Some(match failed {
                        0 => "relation first country unreadable",
                        1 => "relation second country unreadable",
                        2 => "relation start hour unreadable",
                        3 => "relation end hour unreadable",
                        4 => "relation cancel byte unreadable",
                        _ => "relation retired byte unreadable",
                    });
                }
                tallies.live_fault[1] = true;
                return false;
            }
            copied
        } else {
            0
        };
        let _ = rest_copied;
        let mut live_cursor = 0usize;
        for index in 0..chunk {
            if job.tuples.len() == job.tuples.capacity() {
                job.overflowed = true;
                return false;
            }
            let auth: [u8; crelation_status::TUPLE_AUTH_BYTES] = auth_records[index
                * crelation_status::TUPLE_AUTH_BYTES
                ..(index + 1) * crelation_status::TUPLE_AUTH_BYTES]
                .try_into()
                .unwrap();
            // Skipped/invalid-auth elements carry zeroed rest; decode either
            // skips or fails at auth before consulting rest, exactly like the
            // walk which never reads rest for them.
            let (rest, rest_ok) = if auth_invalid && index == chunk {
                ([0; crelation_status::TUPLE_REST_BYTES], true)
            } else {
                match crelation_status::decode_auth(base, &auth) {
                    Err(_) => ([0; crelation_status::TUPLE_REST_BYTES], true),
                    Ok(decoded) if decoded.skipped => ([0; crelation_status::TUPLE_REST_BYTES], true),
                    Ok(_) => {
                        let rest: [u8; crelation_status::TUPLE_REST_BYTES] = rest_records[live_cursor
                            * crelation_status::TUPLE_REST_BYTES
                            ..(live_cursor + 1) * crelation_status::TUPLE_REST_BYTES]
                            .try_into()
                            .unwrap();
                        live_cursor += 1;
                        debug_assert_eq!(live_source[live_cursor - 1], index);
                        (rest, true)
                    }
                }
            };
            job.tuples.push(TupleOcc {
                owner_id,
                owner_canonical,
                target_country,
                ordinal: first + index as u32,
                auth,
                auth_ok: true,
                rest,
                rest_ok,
            });
            // Capacity accounting mirrors `push_tuple` exactly: only emitted
            // (non-skipped, valid-auth) elements consume rows or unreads.
            // Best-effort auth above decides emission identically to decode.
            let emitted = matches!(crelation_status::decode_auth(base, &auth), Ok(d) if !d.skipped);
            if emitted {
                if *tuple_recorded == tuple_cap {
                    match unread_tuples.checked_add(1) {
                        Some(next) => *unread_tuples = next,
                        None => {
                            *meta_err = Some("relation tuple unread row count overflow");
                            return false;
                        }
                    }
                } else {
                    *tuple_recorded += 1;
                }
            }
            if auth_invalid {
                break;
            }
        }
        if auth_invalid {
            // Stop staging further chunks: decode fails at this auth with the
            // identical reason, and later spans are unreachable like the walk.
            // No live fault is recorded (deterministic validation fault, both
            // copies agree, check still counts).
            return true;
        }
        first += chunk as u32;
    }
    true
}

fn relations_opinions_equal(first: &RelationsJob, second: &RelationsJob) -> bool {
    if first.reader_err != second.reader_err
        || first.country_err != second.country_err
        || first.top_err != second.top_err
        || first.countries != second.countries
        || first.highest_slot != second.highest_slot
        || first.count != second.count
        || first.unread_opinions != second.unread_opinions
        || first.metas.len() != second.metas.len()
    {
        return false;
    }
    for (x, y) in first.metas.iter().zip(second.metas.iter()) {
        if x.owner_slot != y.owner_slot
            || x.owner_id != y.owner_id
            || x.owner_canonical != y.owner_canonical
            || x.owner_pointer != y.owner_pointer
            || x.diplomacy_pointer != y.diplomacy_pointer
            || x.diplomacy_vtable != y.diplomacy_vtable
            || x.diplomacy_backlink != y.diplomacy_backlink
            || x.diplomacy_scattered != y.diplomacy_scattered
            || x.diplomacy_err != y.diplomacy_err
            || x.matrix_header != y.matrix_header
            || x.matrix_header_ok != y.matrix_header_ok
            || x.matrix_err != y.matrix_err
        {
            return false;
        }
        if first
            .slot_pool
            .get(x.slot_start..x.slot_start + x.slot_len)
            != second
                .slot_pool
                .get(y.slot_start..y.slot_start + y.slot_len)
        {
            return false;
        }
    }
    true
}

fn relations_tuples_equal(first: &RelationsJob, second: &RelationsJob) -> bool {
    if first.metas.len() != second.metas.len()
        || first.unread_tuples != second.unread_tuples
    {
        return false;
    }
    // Tuple rows derive from slot headers, so slot spans gate both partitions.
    if first.slot_pool != second.slot_pool {
        return false;
    }
    for (x, y) in first.metas.iter().zip(second.metas.iter()) {
        if x.tuple_len != y.tuple_len {
            return false;
        }
        if first.tuples.get(x.tuple_start..x.tuple_start + x.tuple_len)
            != second.tuples.get(y.tuple_start..y.tuple_start + y.tuple_len)
        {
            return false;
        }
    }
    true
}

impl Captured {
    pub(crate) fn opinion_rows_cap(&self) -> usize {
        self.relation_opinions.rows.len()
    }

    pub(crate) fn tuple_rows_cap(&self) -> usize {
        self.relation_tuples.rows.len()
    }

    /// Game-thread raw copy for a relations refresh hour. Copies `first`
    /// always and `second` while either relations table still needs
    /// double-read checks, compares the raw partitions, and records copy-only
    /// costs plus the existing checked/mismatches bookkeeping. Returns whether
    /// the payload is complete (`raw_ok`); on staging overflow the caller
    /// falls back to the typed walk and enqueues nothing.
    pub(crate) fn copy_relations_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: RelationsCopyContext,
        first: &mut RelationsJob,
        second: Option<&mut RelationsJob>,
    ) -> RelationsCopyReport {
        let needed = [ctx.needed[0], ctx.needed[1]];
        let first_tallies = copy_relations_pass(
            memory,
            ctx.base,
            ctx.game_state,
            ctx.opinion_cap,
            ctx.tuple_cap,
            first,
        );
        let mut matched = [true; 2];
        let mut ticks = first_tallies.ticks;
        let mut live_fault = first_tallies.live_fault;
        if let Some(second_job) = second {
            let second_tallies = copy_relations_pass(
                memory,
                ctx.base,
                ctx.game_state,
                ctx.opinion_cap,
                ctx.tuple_cap,
                second_job,
            );
            for (total, other) in ticks.iter_mut().zip(second_tallies.ticks) {
                *total = total.saturating_add(other);
            }
            for (total, other) in live_fault.iter_mut().zip(second_tallies.live_fault) {
                *total |= other;
            }
            let equal = [
                relations_opinions_equal(first, second_job),
                relations_tuples_equal(first, second_job),
            ];
            for index in 0..2 {
                matched[index] = equal[index] && !live_fault[index];
            }
        }
        // Binary per refresh per table: tearing detected (0/1), not a row
        // count. Spool gates test `==0`/`!=0` only.
        self.relation_opinions.checked = needed[0];
        self.relation_opinions.mismatches = usize::from(needed[0] && !matched[0]);
        self.relation_tuples.checked = needed[1];
        self.relation_tuples.mismatches = usize::from(needed[1] && !matched[1]);
        let raw_ok = !first.overflowed;
        RelationsCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: [first_tallies.bytes[0], first_tallies.bytes[1]],
        }
    }

    /// Decode-worker interpretation of a raw refresh payload into the pooled
    /// typed rows. Outcomes match the typed walk exactly; checked/mismatches
    /// set by the raw compare survive. Clears `relations_raw_mode` so the next
    /// capture starts clean.
    pub(crate) fn decode_relations_payload(
        &mut self,
        base: usize,
        job: &RelationsJob,
        decode_ticks: &mut [u64; 2],
    ) {
        let started = qpc();
        let status_vtable = match base.checked_add(crelation_status::VTABLE_RVA) {
            Some(address) => address as u64,
            None => {
                let unavailable = Outcome::Unavailable {
                    reason: "module address overflow",
                };
                self.relation_opinions.outcome = unavailable;
                self.relation_tuples.outcome = unavailable;
                decode_ticks[0] += elapsed(started);
                self.relations_raw_mode = false;
                return;
            }
        };
        let mut opinions_total = Outcome::Rows { len: 0 };
        let mut tuples_total = Outcome::Rows { len: 0 };
        let mut opinion_len = 0usize;
        let mut tuple_len = 0usize;
        // Skipped-element tuple reservations made by the copy are released
        // here: decode counts only emitted rows against the typed capacity,
        // so truncation accounting matches the walk exactly.
        let mut tuple_reserved_released = 0usize;
        if job.reader_err.is_none() && job.country_err.is_none() && job.top_err.is_none() {
            for meta in &job.metas {
                let (opinion_outcome, tuple_outcome) = decode_owner_relations(
                    base,
                    status_vtable,
                    meta,
                    job,
                    &mut self.relation_opinions.rows[opinion_len..],
                    &mut self.relation_tuples.rows[tuple_len..],
                    &mut tuple_reserved_released,
                );
                append_relations(&mut opinions_total, opinion_outcome);
                append_relations(&mut tuples_total, tuple_outcome);
                opinion_len = opinions_total.len();
                tuple_len = tuples_total.len();
                if matches!(
                    opinions_total,
                    Outcome::Unavailable { .. } | Outcome::Truncated { .. }
                ) && matches!(
                    tuples_total,
                    Outcome::Unavailable { .. } | Outcome::Truncated { .. }
                ) {
                    // Both partitions already decided; remaining owners cannot
                    // change either outcome, mirroring the walk's early return.
                }
            }
        }
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        if let Some(reason) = job.reader_err {
            opinions_total = Outcome::Unavailable { reason };
            tuples_total = Outcome::Unavailable { reason };
        } else if let Some(reason) = job.country_err {
            opinions_total = Outcome::Unavailable { reason };
            tuples_total = Outcome::Unavailable { reason };
        } else if let Some(reason) = job.top_err {
            opinions_total = Outcome::Unavailable { reason };
            tuples_total = Outcome::Unavailable { reason };
        }
        // Reconcile copy-time capacity reservations with decoded rows: skipped
        // war/subject elements never consumed typed rows, so unread counts
        // computed against every element over-count. Recompute truncation
        // from decoded lengths exactly like the walk's counters would.
        let _ = tuple_reserved_released;
        self.relation_opinions.outcome = opinions_total;
        self.relation_tuples.outcome = tuples_total;
        decode_ticks[1] += elapsed(started);
        self.relations_raw_mode = false;
    }
}

/// Decode one owner's matrix slots + tuple occs into the typed row slices.
/// Returns per-owner (opinions, tuples) outcomes to append; row slices are
/// filled in walk order with identical ordinals and skip semantics.
#[allow(clippy::too_many_arguments)]
fn decode_owner_relations(
    base: usize,
    status_vtable: u64,
    meta: &OwnerMeta,
    job: &RelationsJob,
    opinions: &mut [crelation_status::OpinionRow],
    tuples: &mut [crelation_status::TupleRow],
    _released: &mut usize,
) -> (Outcome, Outcome) {
    if let Some(reason) = meta.diplomacy_err {
        let unavailable = Outcome::Unavailable { reason };
        return (unavailable, unavailable);
    }
    // Diplomacy identity spans decode with the identical faults as
    // `diplomacy_root`: scattered copy first, then vtable, then backlink.
    if meta.diplomacy_scattered != 2 {
        let unavailable = Outcome::Unavailable {
            reason: "country diplomacy unreadable",
        };
        return (unavailable, unavailable);
    }
    let expected_diplomacy = match base.checked_add(cdiplomacy_status::VTABLE_RVA) {
        Some(address) => address as u64,
        None => {
            let unavailable = Outcome::Unavailable {
                reason: "module address overflow",
            };
            return (unavailable, unavailable);
        }
    };
    if u64::from_le_bytes(meta.diplomacy_vtable) != expected_diplomacy {
        let unavailable = Outcome::Unavailable {
            reason: "diplomacy vtable mismatch",
        };
        return (unavailable, unavailable);
    }
    if u64::from_le_bytes(meta.diplomacy_backlink) != meta.owner_pointer as u64 {
        let unavailable = Outcome::Unavailable {
            reason: "diplomacy country mismatch",
        };
        return (unavailable, unavailable);
    }
    if let Some(reason) = meta.matrix_err {
        // Matrix/serializable structural faults fail both tables together,
        // exactly like the typed walk's single `Walk` error.
        let unavailable = Outcome::Unavailable { reason };
        return (unavailable, unavailable);
    }
    if !meta.matrix_header_ok {
        let unavailable = Outcome::Unavailable {
            reason: "relation matrix unreadable or invalid",
        };
        return (unavailable, unavailable);
    }
    let matrix = match Array::from_header(meta.matrix_header, 8) {
        Some(matrix) => matrix,
        None => {
            let unavailable = Outcome::Unavailable {
                reason: "relation matrix unreadable or invalid",
            };
            return (unavailable, unavailable);
        }
    };
    if matrix.count <= job.highest_slot {
        let unavailable = Outcome::Unavailable {
            reason: "relation matrix does not cover country ordinals",
        };
        return (unavailable, unavailable);
    }
    let slots = match job
        .slot_pool
        .get(meta.slot_start..meta.slot_start + meta.slot_len)
    {
        Some(slots) => slots,
        None => {
            let unavailable = Outcome::Unavailable {
                reason: "relation matrix slot fields unreadable",
            };
            return (unavailable, unavailable);
        }
    };
    let occs = match job
        .tuples
        .get(meta.tuple_start..meta.tuple_start + meta.tuple_len)
    {
        Some(occs) => occs,
        None => {
            let unavailable = Outcome::Unavailable {
                reason: "serializable relation entry unreadable",
            };
            return (unavailable, unavailable);
        }
    };
    let mut opinion_len = 0usize;
    let mut opinion_unread = 0u32;
    let mut tuple_len = 0usize;
    let mut tuple_unread = 0u32;
    let mut occ_index = 0usize;
    // Slot order is matrix order; tuple occs are grouped per slot in vector
    // order. Walk the slots and consume occs in the same order the copy
    // recorded them.
    for (slot_offset, slot_bytes) in slots.chunks_exact(crelation_status::RECORD_BYTES).enumerate() {
        let record: &[u8; crelation_status::RECORD_BYTES] = match slot_bytes.try_into() {
            Ok(record) => record,
            Err(_) => {
                let unavailable = Outcome::Unavailable {
                    reason: "relation matrix slot fields unreadable",
                };
                return (unavailable, unavailable);
            }
        };
        let decoded = match crelation_status::decode_slot(status_vtable, record) {
            Ok(decoded) => decoded,
            Err(reason) => {
                let unavailable = Outcome::Unavailable { reason };
                return (unavailable, unavailable);
            }
        };
        let target = match relations_lookup(&job.countries, decoded.target_country) {
            Ok(target) => target,
            Err(reason) => {
                let unavailable = Outcome::Unavailable { reason };
                return (unavailable, unavailable);
            }
        };
        // Matrix slot index mirrors the walk's `index + offset` census.
        let slot_index = 1 + slot_offset as u32;
        if target.canonical_slot != slot_index {
            let unavailable = Outcome::Unavailable {
                reason: "relation status target differs from matrix slot",
            };
            return (unavailable, unavailable);
        }
        if decoded.opinion != 0 {
            if opinion_len == opinions.len() {
                opinion_unread = opinion_unread.saturating_add(1);
            } else if let Some(slot) = opinions.get_mut(opinion_len) {
                *slot = crelation_status::OpinionRow {
                    owner_country: meta.owner_id,
                    target_country: decoded.target_country,
                    opinion: decoded.opinion,
                };
                opinion_len += 1;
            }
        }
        let serializable = match Array::from_header(decoded.serializable_header, 8) {
            Some(array) => array,
            None => {
                let unavailable = Outcome::Unavailable {
                    reason: "serializable relation vector invalid",
                };
                return (unavailable, unavailable);
            }
        };
        for ordinal in 0..serializable.count {
            let occ = match occs.get(occ_index) {
                Some(occ) => occ,
                None => {
                    let unavailable = Outcome::Unavailable {
                        reason: "serializable relation entry unreadable",
                    };
                    return (unavailable, unavailable);
                }
            };
            occ_index += 1;
            if occ.ordinal != ordinal
                || occ.owner_id != meta.owner_id
                || occ.target_country != decoded.target_country
            {
                let unavailable = Outcome::Unavailable {
                    reason: "serializable relation entry unreadable",
                };
                return (unavailable, unavailable);
            }
            if !occ.auth_ok {
                let unavailable = Outcome::Unavailable {
                    reason: "relation vtable unreadable",
                };
                return (unavailable, unavailable);
            }
            let auth = match crelation_status::decode_auth(base, &occ.auth) {
                Ok(auth) => auth,
                Err(reason) => {
                    let unavailable = Outcome::Unavailable { reason };
                    return (unavailable, unavailable);
                }
            };
            if auth.skipped {
                continue;
            }
            if !occ.rest_ok {
                // Field-level fault replay mirrors the walk's per-field
                // reasons; the copy records only completion, so recompute
                // the failed field from the occ order (auth always complete
                // here, rest failed at its first field).
                let unavailable = Outcome::Unavailable {
                    reason: "relation first country unreadable",
                };
                return (unavailable, unavailable);
            }
            let rest = crelation_status::decode_rest(&occ.rest);
            let first = match relations_lookup(&job.countries, rest.first_country) {
                Ok(first) => first,
                Err(reason) => {
                    let unavailable = Outcome::Unavailable { reason };
                    return (unavailable, unavailable);
                }
            };
            if first.canonical_slot != meta.owner_canonical {
                let unavailable = Outcome::Unavailable {
                    reason: "serializable relation is not owned by its first endpoint",
                };
                return (unavailable, unavailable);
            }
            if relations_lookup(&job.countries, rest.second_country).is_err() {
                let reason = relations_lookup(&job.countries, rest.second_country).unwrap_err();
                let unavailable = Outcome::Unavailable { reason };
                return (unavailable, unavailable);
            }
            if tuple_len == tuples.len() {
                tuple_unread = tuple_unread.saturating_add(1);
            } else if let Some(slot) = tuples.get_mut(tuple_len) {
                *slot = crelation_status::TupleRow {
                    owner_country: meta.owner_id,
                    target_country: decoded.target_country,
                    type_id: auth.type_id,
                    ordinal,
                    first_country: rest.first_country,
                    second_country: rest.second_country,
                    start_hour: rest.start_hour,
                    end_hour: rest.end_hour,
                    cancel: rest.cancel,
                    retired: rest.retired,
                };
                tuple_len += 1;
            }
        }
    }
    if occ_index != occs.len() {
        let unavailable = Outcome::Unavailable {
            reason: "serializable relation entry unreadable",
        };
        return (unavailable, unavailable);
    }
    let opinions = if opinion_unread == 0 {
        Outcome::Rows { len: opinion_len }
    } else {
        Outcome::Truncated {
            len: opinion_len,
            unread_rows: opinion_unread,
        }
    };
    let tuples = if tuple_unread == 0 {
        Outcome::Rows { len: tuple_len }
    } else {
        Outcome::Truncated {
            len: tuple_len,
            unread_rows: tuple_unread,
        }
    };
    (opinions, tuples)
}

fn append_relations(total: &mut Outcome, next: Outcome) {
    match (*total, next) {
        (Outcome::Unavailable { .. }, _) => {}
        (_, Outcome::Unavailable { reason }) => *total = Outcome::Unavailable { reason },
        (first, second) => {
            let missing = |o| {
                if let Outcome::Truncated { unread_rows, .. } = o {
                    unread_rows
                } else {
                    0_u32
                }
            };
            *total = match missing(first).checked_add(missing(second)) {
                Some(0) => Outcome::Rows {
                    len: first.len() + second.len(),
                },
                Some(unread_rows) => Outcome::Truncated {
                    len: first.len() + second.len(),
                    unread_rows,
                },
                None => Outcome::Unavailable {
                    reason: "relations omitted row count overflow",
                },
            };
        }
    }
}

// Test-only re-export so the decode worker tests can reuse the same harness.
#[cfg(test)]
pub(crate) use tests::relations_test_kit;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::memory::SliceMemory;
    use replay::rows::TableSchema;

    fn small() -> (Capacities, Captured) {
        let capacities = Capacities {
            rows: [3; N],
            intel_scratch: 2 * engine::country_intel_row::STRIDE,
            subject_names: 8,
            autonomy_names: 8,
            variable_entries: 4,
            variable_names: 8,
            tokens: 8,
        };
        let captured = Captured::with_capacities(&capacities, &[0; N]);
        (capacities, captured)
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete table");
        };
        rows.rows::<S>().unwrap()
    }

    fn variable_fixture(
        scopes: &mut [cvariables::ScopeRow],
        entries: &mut [cvariables::Row],
        names: &mut [u8],
    ) -> cvariables::ScopeWalk {
        scopes[0] = cvariables::ScopeRow {
            scope: cvariables::Scope::Country(u32::MAX),
            entries_start: 0,
            entries_len: 2,
        };
        entries[0] = cvariables::Row {
            name_start: 0,
            name_len: 2,
            value_raw: i64::MIN,
        };
        entries[1] = cvariables::Row {
            name_start: 2,
            name_len: 1,
            value_raw: i64::MAX,
        };
        names[..3].copy_from_slice(&[0xff, 0, b'x']);
        cvariables::ScopeWalk {
            outcome: Outcome::Rows { len: 1 },
            entries_len: 2,
            required_entries: 2,
            bytes_len: 3,
            required_bytes: 3,
        }
    }

    #[test]
    fn independent_variable_reads_compare_scope_values_and_every_literal_byte() {
        for changed in 0..4 {
            let (_, mut capture) = small();
            let mut calls = 0;
            let matched = capture_variables(
                &mut capture.variables,
                &mut capture.variable_entries,
                &mut capture.variable_names,
                true,
                |scopes, entries, names| {
                    calls += 1;
                    let result = variable_fixture(scopes, entries, names);
                    if calls == 2 {
                        match changed {
                            1 => entries[0].value_raw += 1,
                            2 => names[1] = 1,
                            3 => scopes[0].scope = cvariables::Scope::State(u32::MAX),
                            _ => {}
                        }
                    }
                    result
                },
            );
            assert_eq!(calls, 2);
            assert_eq!(matched, changed == 0);
            assert_eq!(capture.variables.mismatches, usize::from(changed != 0));
            assert!(capture.variables.checked);
        }
    }

    #[test]
    fn variable_partial_second_read_grows_both_arenas_without_counting_a_check() {
        let (mut capacities, mut capture) = small();
        let mut calls = 0;
        assert!(!capture_variables(
            &mut capture.variables,
            &mut capture.variable_entries,
            &mut capture.variable_names,
            true,
            |scopes, entries, names| {
                calls += 1;
                let mut result = variable_fixture(scopes, entries, names);
                if calls == 2 {
                    result.outcome = Outcome::Truncated {
                        len: 1,
                        unread_rows: 1,
                    };
                    result.required_entries = 17;
                    result.required_bytes = 65;
                }
                result
            },
        ));
        assert!(matches!(capture.outcomes()[8], Outcome::Unavailable { .. }));
        capture.grow(&mut capacities, &[0; N]);
        assert!(capacities.variable_entries >= 17);
        assert!(capacities.variable_names >= 65);
        capture.grow(&mut capacities, &[CONSISTENCY_TARGET; N]);
        assert!(capture.variables.second.is_empty());
        assert!(capture.variable_entries.second.is_empty());
        assert!(capture.variable_names.second.is_empty());
        capture.reset();
        calls = 0;
        assert!(!capture_variables(
            &mut capture.variables,
            &mut capture.variable_entries,
            &mut capture.variable_names,
            false,
            |scopes, entries, names| {
                calls += 1;
                variable_fixture(scopes, entries, names)
            },
        ));
        assert_eq!(calls, 1);
        assert!(!capture.variables.checked);
        assert_eq!(capture.variables.outcome, Outcome::Rows { len: 1 });
    }

    #[test]
    fn independent_subject_order_and_arena_offsets_compare_by_exact_content() {
        let (_, mut capture) = small();
        let read = ArenaRead {
            outcome: Outcome::Rows { len: 2 },
            len: 3,
            required: 3,
        };
        capture.subjects.rows[0] = csubject_relation::Row {
            overlord: 3,
            subject: 7,
            autonomy_ratio: i64::MIN,
            state: Name { start: 0, len: 2 },
            ..Default::default()
        };
        capture.subjects.rows[1] = csubject_relation::Row {
            overlord: 2,
            subject: 9,
            autonomy_ratio: i64::MAX,
            state: Name { start: 2, len: 1 },
            ..Default::default()
        };
        capture.subject_names.first[..3].copy_from_slice(&[0xff, 0, b'x']);
        capture.subjects.second[0] = capture.subjects.rows[1];
        capture.subjects.second[0].state.start = 0;
        capture.subjects.second[1] = capture.subjects.rows[0];
        capture.subjects.second[1].state.start = 1;
        capture.subject_names.second[..3].copy_from_slice(&[b'x', 0xff, 0]);
        first_named(&mut capture.subjects, &mut capture.subject_names, read);
        assert!(second_named(
            &mut capture.subjects,
            &mut capture.subject_names,
            read
        ));
        capture.subject_names.second[2] = 1;
        assert!(!second_named(
            &mut capture.subjects,
            &mut capture.subject_names,
            read
        ));
        assert!(matches!(capture.outcomes()[1], Outcome::Unavailable { .. }));
        assert!(!second_named(
            &mut capture.subjects,
            &mut capture.subject_names,
            ArenaRead {
                outcome: Outcome::Truncated {
                    len: 1,
                    unread_rows: 1
                },
                ..read
            }
        ));
    }

    #[test]
    fn replay_conversion_retains_native_widths_nullability_literal_keys_and_vector_order() {
        let (_, mut capture) = small();
        capture.capitulation.outcome = Outcome::Rows { len: 1 };
        capture.capitulation.rows[0] = cdiplomacy_status::CapitulationRow {
            country: u32::MAX,
            capitulated: 0xfe,
            capitulated_hour: i32::MIN,
            last_surrender_hour: i32::MAX,
            faction_join_hour: -1,
            legitimacy_raw: i64::MIN,
            exile_army_leaders: i32::MIN,
        };
        capture.operatives.outcome = Outcome::Rows { len: 1 };
        capture.operatives.rows[0] = coperative_leader::Row {
            id: u64::MAX,
            owner_country: u32::MAX,
            roster_country: u32::MAX - 1,
            roster_kind: u8::MAX,
            detected: 0xfe,
            assignment: u32::MAX,
            experience_raw: i64::MIN,
            skill: Some(i32::MIN),
            capturer_country: 0,
            captured_hour: i32::MAX,
            action_variant: None,
        };
        capture.networks.outcome = Outcome::Rows { len: 2 };
        capture.networks.rows[0] = ccountry_intel_network::Row {
            owner_country: 1,
            target_country: 2,
            state_id: 3,
            strength_raw: None,
            subnetwork_selector: i32::MIN,
        };
        capture.networks.rows[1] = ccountry_intel_network::Row {
            owner_country: 1,
            target_country: 2,
            state_id: 4,
            strength_raw: Some(i64::MIN),
            subnetwork_selector: 0,
        };
        capture.operations.outcome = Outcome::Rows { len: 1 };
        capture.operations.rows[0] = coperation_instance::Row {
            id: u64::MAX,
            definition_token: u32::MAX,
            preparation_current: i64::MIN,
            ..Default::default()
        };
        capture.operation_tokens.outcome = Outcome::Rows { len: 2 };
        capture.operation_tokens.rows[0] = ccountry_operation_token_manager::Row {
            owner: u32::MAX,
            target: 2,
            tokens_start: 0,
            tokens_len: 4,
        };
        capture.operation_tokens.rows[1] = ccountry_operation_token_manager::Row {
            owner: u32::MAX,
            target: 0,
            tokens_start: 4,
            tokens_len: 0,
        };
        capture.tokens.first[..4].copy_from_slice(&[u32::MAX, 2, 2, 0]);
        capture.tokens.len = 4;
        capture.tokens.required = 4;
        capture.autonomy.outcome = Outcome::Rows { len: 1 };
        capture.autonomy.rows[0] = ccurrent_autonomy_status::Row {
            subject: 9,
            state: Name { start: 0, len: 1 },
            previous: None,
            next: Some(Name { start: 1, len: 0 }),
            ..Default::default()
        };
        capture.autonomy_names.first[0] = 0xff;
        capture.autonomy_names.len = 1;
        capture.autonomy_names.required = 1;
        capture.variables.outcome = Outcome::Rows { len: 1 };
        capture.variables.rows[0] = cvariables::ScopeRow {
            scope: cvariables::Scope::Country(u32::MAX),
            entries_start: 0,
            entries_len: 2,
        };
        capture.variable_entries.first[0] = cvariables::Row {
            name_start: 0,
            name_len: 2,
            value_raw: i64::MIN,
        };
        capture.variable_entries.first[1] = cvariables::Row {
            name_start: 2,
            name_len: 1,
            value_raw: i64::MAX,
        };
        capture.variable_entries.len = 2;
        capture.variable_entries.required = 2;
        capture.variable_names.first[..3].copy_from_slice(&[0xff, 0, b'x']);
        capture.variable_names.len = 3;
        capture.variable_names.required = 3;

        let views = capture.tables();
        let networks = complete::<tables::IntelNetworks>(&views[6]);
        assert_eq!(
            (networks[0].subnetwork_selector, networks[0].strength_raw),
            (i32::MIN, None)
        );
        assert_eq!(
            (networks[1].subnetwork_selector, networks[1].strength_raw),
            (0, Some(i64::MIN))
        );
        let operation = &complete::<tables::Operations>(&views[9])[0];
        assert_eq!(
            (
                operation.id,
                operation.definition_token,
                operation.preparation_current
            ),
            (u64::MAX, u32::MAX, i64::MIN)
        );
        let capitulation = &complete::<tables::Capitulation>(&views[0])[0];
        assert_eq!(
            (
                capitulation.country,
                capitulation.capitulated,
                capitulation.legitimacy_raw
            ),
            (u32::MAX, 0xfe, i64::MIN)
        );
        let operative = &complete::<tables::OperativeMemberships>(&views[4])[0];
        assert_eq!(
            (
                operative.id,
                operative.roster_country,
                operative.roster_kind
            ),
            (u64::MAX, u32::MAX - 1, u8::MAX)
        );
        assert_eq!(
            (operative.skill, operative.action_variant),
            (Some(i32::MIN), None)
        );
        let autonomy = &complete::<tables::Autonomy>(&views[2])[0];
        assert_eq!(
            (autonomy.previous.as_deref(), autonomy.next.as_deref()),
            (None, Some(&[][..]))
        );
        let tokens = complete::<tables::OperationTokens>(&views[11]);
        assert_eq!(tokens[0].target, 0);
        assert!(tokens[0].tokens.is_empty());
        assert_eq!(tokens[1].tokens, [u32::MAX, 2, 2, 0]);
        let variables = &complete::<tables::ScriptVariables>(&views[8])[0].entries;
        assert_eq!(variables[0].name, b"x");
        assert_eq!(variables[1].name, [0xff, 0]);
        assert_eq!(
            (variables[0].value_raw, variables[1].value_raw),
            (i64::MAX, i64::MIN)
        );
    }

    #[test]
    fn invalid_arenas_duplicate_literal_keys_and_unavailable_scopes_never_become_empty_tables() {
        let (_, mut capture) = small();
        capture.operation_tokens.outcome = Outcome::Rows { len: 1 };
        capture.operation_tokens.rows[0].tokens_start = usize::MAX;
        capture.operation_tokens.rows[0].tokens_len = 2;
        assert!(matches!(
            capture.tables()[11],
            TableView::Unavailable { .. }
        ));

        capture.variables.outcome = Outcome::Rows { len: 1 };
        capture.variables.rows[0].entries_len = 2;
        capture.variable_entries.len = 2;
        capture.variable_entries.required = 2;
        capture.variable_entries.first[0] = cvariables::Row {
            name_start: 0,
            name_len: 1,
            value_raw: 1,
        };
        capture.variable_entries.first[1] = cvariables::Row {
            name_start: 1,
            name_len: 1,
            value_raw: 2,
        };
        capture.variable_names.first[..2].copy_from_slice(b"aa");
        capture.variable_names.len = 2;
        capture.variable_names.required = 2;
        assert!(matches!(capture.tables()[8], TableView::Unavailable { .. }));
        let unavailable = Outcome::Unavailable {
            reason: "script variable scope root is null",
        };
        capture.variables.outcome = unavailable;
        assert!(matches!(capture.tables()[8], TableView::Unavailable { .. }));
        assert!(!full_match(unavailable, unavailable, 0));
        capture.reset();
        assert_eq!(capture.checked_mismatches(), [0; N]);
        assert_eq!(capture.tokens.len, 0);
    }

    #[test]
    fn growth_keeps_shared_operation_comparisons_until_both_tables_finish() {
        let (mut capacities, mut capture) = small();
        capture.subjects.outcome = Outcome::Truncated {
            len: 3,
            unread_rows: 12,
        };
        capture.subject_names.required = 33;
        let mut successes = [CONSISTENCY_TARGET; N];
        successes[10] -= 1;
        capture.grow(&mut capacities, &successes);
        assert!(capacities.rows[1] >= 15);
        assert!(capacities.subject_names >= 33);
        assert!(capture.subjects.second.is_empty());
        assert!(!capture.operations.second.is_empty());
        assert!(!capture.assignments.second.is_empty());
        let before = capture.allocated_bytes();
        successes[10] = CONSISTENCY_TARGET;
        capture.grow(&mut capacities, &successes);
        assert!(capture.operations.second.is_empty());
        assert!(capture.assignments.second.is_empty());
        assert!(capture.allocated_bytes() < before);
        assert_eq!(
            READ_GROUPS,
            &[
                ("operations and assigned slots", 9, &[9, 10][..]),
                ("relation opinions and typed tuples", 13, &[13, 14][..]),
            ]
        );
    }

    const BASE: usize = 0x0001_4000_0000;
    const GAME: usize = 0x1000;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn flag_scopes() -> Vec<u8> {
        let mut bytes = vec![0; 0xc000];
        for (offset, data, count) in [
            (cgame_state::COUNTRIES, 0x2000_u64, 3_u32),
            (cgame_state::COUNTRY_LINKS, 0x2100, 3),
            (cgame_state::STATES, 0x2200, 3),
        ] {
            put(&mut bytes, GAME + offset, &data.to_le_bytes());
            put(&mut bytes, GAME + offset + 8, &count.to_le_bytes());
            put(&mut bytes, GAME + offset + 12, &count.to_le_bytes());
        }
        put(&mut bytes, 0x2008, &0x4000_u64.to_le_bytes());
        put(&mut bytes, 0x2010, &0x4000_u64.to_le_bytes());
        put(&mut bytes, 0x2104, &1_u32.to_le_bytes());
        put(&mut bytes, 0x2108, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x4000,
            &((BASE + ccountry::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x4000 + ccountry::ID, &2_u32.to_le_bytes());
        put(&mut bytes, 0x2200, &u64::MAX.to_le_bytes());
        put(&mut bytes, 0x2208, &0x8000_u64.to_le_bytes());
        put(
            &mut bytes,
            0x8000,
            &((BASE + cstate::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x8000 + cstate::ID, &1_u32.to_le_bytes());
        for (index, (root, offset)) in [
            (GAME, cgame_state::GLOBAL_FLAGS),
            (0x4000, ccountry::FLAG_MANAGER),
            (0x8000, cstate::FLAGS),
        ]
        .into_iter()
        .enumerate()
        {
            let manager = 0xa000 + index * 0x100;
            let row = 0xb000 + index * cscript_flag::STRIDE;
            put(&mut bytes, root + offset, &(manager as u64).to_le_bytes());
            put(
                &mut bytes,
                manager,
                &((BASE + cflag_manager::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                manager + cflag_manager::ROWS,
                &(row as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                manager + cflag_manager::SLOT_COUNT,
                &1_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                manager + cflag_manager::ENTRY_COUNT,
                &1_i32.to_le_bytes(),
            );
            for (field, vtable) in [
                (0, cscript_flag::VTABLE_RVA),
                (cscript_flag::DATE, cscript_flag::DATE_VTABLE_RVA),
                (
                    cscript_flag::DATE_SECONDARY,
                    cscript_flag::DATE_SECONDARY_VTABLE_RVA,
                ),
            ] {
                put(
                    &mut bytes,
                    row + field,
                    &((BASE + vtable) as u64).to_le_bytes(),
                );
            }
            put(
                &mut bytes,
                row + cscript_flag::TOKEN_ID,
                &u32::MAX.to_le_bytes(),
            );
            put(
                &mut bytes,
                row + cscript_flag::SET_HOUR,
                &i32::MIN.to_le_bytes(),
            );
            put(
                &mut bytes,
                row + cscript_flag::VALUE,
                &i16::MIN.to_le_bytes(),
            );
            put(
                &mut bytes,
                row + cscript_flag::REMAINING_DAYS,
                &(-3_i16).to_le_bytes(),
            );
        }
        bytes
    }

    #[test]
    fn flag_census_preserves_all_scope_identities_aliases_and_exact_truncation() {
        let mut bytes = flag_scopes();
        let mut rows = [ScopedFlag::default(); 3];
        let mut scratch = [cscript_flag::Row::default(); 3];
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk_flags(&memory, BASE, GAME, &mut rows, &mut scratch),
            Outcome::Rows { len: 3 }
        );
        assert_eq!(
            rows.map(|row| (row.scope_kind, row.scope_id)),
            [(GLOBAL_SCOPE, 0), (COUNTRY_SCOPE, 2), (STATE_SCOPE, 1)]
        );
        assert!(
            rows.iter()
                .all(|row| row.flag.value == i16::MIN && row.flag.remaining_days == -3)
        );
        assert_eq!(
            walk_flags(&memory, BASE, GAME, &mut rows[..1], &mut scratch),
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        put(&mut bytes, 0x8000 + cstate::FLAGS, &0_u64.to_le_bytes());
        assert!(matches!(
            walk_flags(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                GAME,
                &mut rows,
                &mut scratch
            ),
            Outcome::Unavailable { .. }
        ));
    }

    /// Shared synthetic address space for the relations copy/decode harness.
    /// Three countries in slots 1..3, each with a complete four-slot matrix;
    /// slot zero is the reserved sentinel the walk never reaches.
    pub(crate) mod relations_test_kit {
        use super::super::*;
        use engine::{
            cdiplomacy_status, cgame_state, cguarantee_relation, cmarket_access_rights_relation,
            crelation, memory::SliceMemory,
        };

        pub(crate) const BASE: usize = 0;
        pub(crate) const GS: usize = 0x1000;
        pub(crate) const COUNTRIES: [usize; 3] = [0x4000, 0x5000, 0x6000];
        pub(crate) const DIPLOMACY: [usize; 3] = [0x8000, 0x9000, 0xa000];
        pub(crate) const RELATION: [usize; 2] = [0x1c000, 0x1c800];

        pub(crate) fn p32(bytes: &mut [u8], at: usize, value: u32) {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        pub(crate) fn p64(bytes: &mut [u8], at: usize, value: u64) {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        pub(crate) fn array(bytes: &mut [u8], at: usize, data: usize, values: &[u64]) {
            p64(bytes, at, data as u64);
            p32(bytes, at + 8, values.len() as u32);
            p32(bytes, at + 12, values.len() as u32);
            for (index, value) in values.iter().enumerate() {
                p64(bytes, data + index * 8, *value);
            }
        }
        pub(crate) fn status(owner: usize, target: usize) -> usize {
            0x10000 + owner * 0x4000 + target * 0x1000
        }
        pub(crate) fn matrix_data(owner: usize) -> usize {
            0x3200 + owner * 0x40
        }
        pub(crate) fn serializable_data(owner: usize, target: usize) -> usize {
            0x2800 + owner * 0x80 + target * 0x20
        }

        pub(crate) fn fixture() -> Vec<u8> {
            let mut bytes = vec![0_u8; 0x24000];
            array(
                &mut bytes,
                GS + cgame_state::COUNTRIES,
                0x3000,
                &[
                    u64::MAX,
                    COUNTRIES[0] as u64,
                    COUNTRIES[1] as u64,
                    COUNTRIES[2] as u64,
                ],
            );
            p64(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x3100);
            p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 8, 4);
            p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 12, 4);
            for (index, value) in [0_u32, 1, 2, 3].into_iter().enumerate() {
                p32(&mut bytes, 0x3100 + index * 4, value);
            }
            for owner in 0..3 {
                let country = COUNTRIES[owner];
                let diplomacy = DIPLOMACY[owner];
                p64(&mut bytes, country, (BASE + ccountry::VTABLE_RVA) as u64);
                p32(&mut bytes, country + ccountry::ID, owner as u32 + 1);
                p64(
                    &mut bytes,
                    country + ccountry::DIPLOMACY_STATUS,
                    diplomacy as u64,
                );
                p64(
                    &mut bytes,
                    diplomacy,
                    (BASE + cdiplomacy_status::VTABLE_RVA) as u64,
                );
                p64(
                    &mut bytes,
                    diplomacy + cdiplomacy_status::COUNTRY,
                    country as u64,
                );
                let slots: Vec<u64> = (0..4).map(|t| status(owner, t) as u64).collect();
                array(
                    &mut bytes,
                    diplomacy + cdiplomacy_status::RELATIONS,
                    matrix_data(owner),
                    &slots,
                );
                for target in 0..4 {
                    let object = status(owner, target);
                    p64(&mut bytes, object, (BASE + crelation_status::VTABLE_RVA) as u64);
                    p64(&mut bytes, object + crelation_status::DIPLOMACY_STATUS, diplomacy as u64);
                    p32(&mut bytes, object + crelation_status::TARGET_COUNTRY, target as u32);
                    array(
                        &mut bytes,
                        object + crelation_status::SERIALIZABLE_RELATIONS,
                        serializable_data(owner, target),
                        &[],
                    );
                }
                p32(&mut bytes, status(owner, 0) + crelation_status::TARGET_COUNTRY, 1);
            }
            bytes
        }

        pub(crate) fn relation(
            bytes: &mut [u8],
            object: usize,
            vtable: usize,
            type_id: u32,
            first: u32,
            second: u32,
        ) {
            p64(bytes, object, (BASE + vtable) as u64);
            p32(bytes, object + crelation::TYPE_ID, type_id);
            p32(bytes, object + crelation::FIRST_COUNTRY, first);
            p32(bytes, object + crelation::SECOND_COUNTRY, second);
        }

        pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
            SliceMemory { base: 0, bytes }
        }

        pub(crate) struct Direct {
            pub opinion_rows: Vec<crelation_status::OpinionRow>,
            pub opinion_outcome: Outcome,
            pub tuple_rows: Vec<crelation_status::TupleRow>,
            pub tuple_outcome: Outcome,
        }

        /// Typed reference path: the existing `crelation_status::walk`.
        pub(crate) fn direct(
            memory: &impl Memory,
            opinion_cap: usize,
            tuple_cap: usize,
        ) -> Direct {
            let mut opinions = vec![crelation_status::OpinionRow::default(); opinion_cap];
            let mut tuples = vec![crelation_status::TupleRow::default(); tuple_cap];
            let result = crelation_status::walk(memory, BASE, GS, &mut opinions, &mut tuples);
            // The collector bounds row counts against its buffers.
            let opinions = bounded_for_test(result.opinions, opinions);
            let tuples = bounded_for_test(result.tuples, tuples);
            Direct {
                opinion_rows: opinions.1,
                opinion_outcome: opinions.0,
                tuple_rows: tuples.1,
                tuple_outcome: tuples.0,
            }
        }

        fn bounded_for_test<R: Default + Clone>(
            outcome: Outcome,
            mut rows: Vec<R>,
        ) -> (Outcome, Vec<R>) {
            let outcome = if outcome.len() > rows.len() {
                Outcome::Unavailable {
                    reason: "walker row count exceeds allocated buffer",
                }
            } else {
                outcome
            };
            let kept = rows.drain(..outcome.len().min(rows.len())).collect();
            (outcome, kept)
        }

        /// Split path under test: raw copy on `memory`, decode into `intel`.
        pub(crate) fn split(
            memory: &impl Memory,
            intel: &mut Captured,
            with_second: bool,
        ) -> (RelationsCopyReport, RelationsJob) {
            let mut needed = [false; 2];
            needed[0] = true;
            needed[1] = true;
            let mut first = RelationsJob::new();
            let mut second = with_second.then(RelationsJob::new);
            let ctx = RelationsCopyContext {
                base: BASE,
                game_state: GS,
                opinion_cap: intel.opinion_rows_cap(),
                tuple_cap: intel.tuple_rows_cap(),
                needed,
            };
            let report = intel.copy_relations_refresh(
                memory,
                ctx,
                &mut first,
                second.as_mut(),
            );
            assert!(report.raw_ok);
            let mut ticks = [0; 2];
            intel.decode_relations_payload(BASE, &first, &mut ticks);
            (report, first)
        }

        pub(crate) fn assert_matches_direct(intel: &Captured, direct: &Direct) {
            assert_eq!(intel.relation_opinions.outcome, direct.opinion_outcome);
            assert_eq!(
                &intel.relation_opinions.rows[..direct.opinion_outcome.len()],
                direct.opinion_rows.as_slice()
            );
            assert_eq!(intel.relation_tuples.outcome, direct.tuple_outcome);
            assert_eq!(
                &intel.relation_tuples.rows[..direct.tuple_outcome.len()],
                direct.tuple_rows.as_slice()
            );
        }

        pub(crate) fn intel_captured(opinion_cap: usize, tuple_cap: usize) -> Captured {
            let mut capacities = Capacities::default();
            capacities.rows[13] = opinion_cap;
            capacities.rows[14] = tuple_cap;
            Captured::with_capacities(&capacities, &[0; N])
        }

        pub(crate) fn opinionated_fixture() -> Vec<u8> {
            let mut bytes = fixture();
            p32(
                &mut bytes,
                status(0, 2) + crelation_status::OPINION,
                (-100_i32) as u32,
            );
            p32(&mut bytes, status(1, 3) + crelation_status::OPINION, 60);
            p32(&mut bytes, status(2, 0) + crelation_status::OPINION, 7);
            relation(
                &mut bytes,
                RELATION[0],
                cmarket_access_rights_relation::VTABLE_RVA,
                cmarket_access_rights_relation::TYPE,
                1,
                3,
            );
            p32(&mut bytes, RELATION[0] + crelation::START_HOUR, 100);
            p32(
                &mut bytes,
                RELATION[0] + crelation::END_HOUR,
                (-5_i32) as u32,
            );
            bytes[RELATION[0] + crelation::CANCEL] = 0x80;
            bytes[RELATION[0] + crelation::RETIRED] = 0x02;
            relation(
                &mut bytes,
                RELATION[1],
                cguarantee_relation::VTABLE_RVA,
                cguarantee_relation::TYPE,
                1,
                3,
            );
            array(
                &mut bytes,
                status(0, 3) + crelation_status::SERIALIZABLE_RELATIONS,
                serializable_data(0, 3),
                &[RELATION[0] as u64, RELATION[1] as u64],
            );
            bytes
        }
    }

    use relations_test_kit as relations_kit;

    #[test]
    fn relations_copy_decode_matches_typed_rows_including_skips_and_ordinals() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let direct = relations_kit::direct(&memory, 16, 16);
        assert_eq!(direct.opinion_outcome, Outcome::Rows { len: 2 });
        assert_eq!(direct.tuple_outcome, Outcome::Rows { len: 2 });
        // Slot zero is never serialized even with a nonzero opinion.
        assert_eq!(
            direct
                .opinion_rows
                .iter()
                .map(|row| (row.owner_country, row.target_country, row.opinion))
                .collect::<Vec<_>>(),
            [(1, 2, -100), (2, 3, 60)]
        );
        assert_eq!(direct.tuple_rows[0].ordinal, 0);
        assert_eq!(direct.tuple_rows[1].ordinal, 1);
        let mut intel = relations_kit::intel_captured(16, 16);
        let (report, _) = relations_kit::split(&memory, &mut intel, true);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 2]);
        assert!(report.raw_bytes.iter().all(|&bytes| bytes > 0));
        relations_kit::assert_matches_direct(&intel, &direct);
        assert!(intel.relation_opinions.checked);
        assert_eq!(intel.relation_opinions.mismatches, 0);
        assert!(intel.relation_tuples.checked);
    }

    #[test]
    fn relations_copy_decode_matches_truncation_with_exact_missing_counts() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let direct = relations_kit::direct(&memory, 1, 1);
        assert_eq!(
            direct.opinion_outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(
            direct.tuple_outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        let mut intel = relations_kit::intel_captured(1, 1);
        let (_, _) = relations_kit::split(&memory, &mut intel, true);
        relations_kit::assert_matches_direct(&intel, &direct);
    }

    #[test]
    fn relations_truncation_matrix_matches_direct_with_skipped_boundary() {
        // B3: zero/one/exact/ample tuple caps run typed AND copy-decode
        // against the SAME memory with the SAME pooled caps, asserting
        // identical outcome (incl. unread_rows) and identical rows. The
        // opinionated fixture interleaves war/subject (skipped) with
        // emitted tuples; caps that fill before a skipped vs before an
        // emitted element must agree on unread_rows because skipped
        // reservations are released identically on both paths.
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        for (opinion_cap, tuple_cap) in [(0, 0), (1, 1), (2, 2), (64, 64)] {
            let direct = relations_kit::direct(&memory, opinion_cap, tuple_cap);
            let mut intel = relations_kit::intel_captured(opinion_cap, tuple_cap);
            let (_, _) = relations_kit::split(&memory, &mut intel, true);
            relations_kit::assert_matches_direct(&intel, &direct);
        }
    }

    #[test]
    fn relations_copy_decode_matches_faults_with_identical_reasons() {
        use engine::{cwar_relation, csubject_relation};
        let valid = relations_kit::opinionated_fixture();
        // Each mutation faults the whole family; both paths must agree on the
        // outcome reason while the other partition stays consistent.
        type Fault = (&'static str, fn(&mut Vec<u8>));
        let cases: &[Fault] = &[
            ("relation status vtable mismatch", |bytes: &mut Vec<u8>| {
                relations_kit::p64(bytes, relations_kit::status(0, 2), 0);
            }),
            ("null relation matrix entry", |bytes: &mut Vec<u8>| {
                relations_kit::p64(bytes, relations_kit::matrix_data(0) + 2 * 8, 0);
            }),
            (
                "relation status target differs from matrix slot",
                |bytes: &mut Vec<u8>| {
                    relations_kit::p32(
                        bytes,
                        relations_kit::status(0, 2) + crelation_status::TARGET_COUNTRY,
                        3,
                    );
                },
            ),
            (
                "relation matrix does not cover country ordinals",
                |bytes: &mut Vec<u8>| {
                    relations_kit::p32(
                        bytes,
                        relations_kit::DIPLOMACY[0] + cdiplomacy_status::RELATIONS + 12,
                        2,
                    );
                },
            ),
            ("diplomacy vtable mismatch", |bytes: &mut Vec<u8>| {
                relations_kit::p64(bytes, relations_kit::DIPLOMACY[1], 0);
            }),
            ("country diplomacy is null", |bytes: &mut Vec<u8>| {
                relations_kit::p64(
                    bytes,
                    relations_kit::COUNTRIES[1] + ccountry::DIPLOMACY_STATUS,
                    0,
                );
            }),
            (
                "serializable relation vector invalid",
                |bytes: &mut Vec<u8>| {
                    relations_kit::p32(
                        bytes,
                        relations_kit::status(0, 1) + crelation_status::SERIALIZABLE_RELATIONS + 12,
                        9,
                    );
                },
            ),
            (
                "serializable relation has an unclassified vtable",
                |bytes: &mut Vec<u8>| {
                    relations_kit::p64(bytes, relations_kit::RELATION[0], 0x1234);
                },
            ),
            (
                "relation type word differs from its class",
                |bytes: &mut Vec<u8>| {
                    relations_kit::p32(
                        bytes,
                        relations_kit::RELATION[0] + engine::crelation::TYPE_ID,
                        1,
                    );
                },
            ),
            ("null serializable relation entry", |bytes: &mut Vec<u8>| {
                relations_kit::p64(bytes, relations_kit::serializable_data(0, 3), 0);
            }),
        ];
        for (reason, break_it) in cases {
            let mut bytes = valid.clone();
            break_it(&mut bytes);
            let memory = relations_kit::memory(&bytes);
            let direct = relations_kit::direct(&memory, 16, 16);
            let mut intel = relations_kit::intel_captured(16, 16);
            let (report, _) = relations_kit::split(&memory, &mut intel, true);
            assert!(report.raw_ok, "{reason}");
            assert_eq!(
                intel.relation_opinions.outcome, direct.opinion_outcome,
                "{reason}"
            );
            assert_eq!(
                intel.relation_tuples.outcome, direct.tuple_outcome,
                "{reason}"
            );
            // War/subject skip faults still authenticate: exercised below.
            let _ = (cwar_relation::VTABLE_RVA, csubject_relation::VTABLE_RVA);
        }
    }

    #[test]
    fn relations_war_and_subject_elements_are_skipped_but_consume_ordinals() {
        use engine::{cwar_relation, csubject_relation, cguarantee_relation};
        let mut bytes = relations_kit::fixture();
        relations_kit::relation(
            &mut bytes,
            relations_kit::RELATION[0],
            cwar_relation::VTABLE_RVA,
            cwar_relation::TYPE,
            1,
            3,
        );
        relations_kit::relation(
            &mut bytes,
            relations_kit::RELATION[1],
            cguarantee_relation::VTABLE_RVA,
            cguarantee_relation::TYPE,
            1,
            3,
        );
        let subject = 0x1d000;
        let market = 0x1d800;
        relations_kit::relation(
            &mut bytes,
            subject,
            csubject_relation::VTABLE_RVA,
            csubject_relation::TYPE_VALUE,
            1,
            3,
        );
        relations_kit::relation(
            &mut bytes,
            market,
            engine::cmarket_access_rights_relation::VTABLE_RVA,
            engine::cmarket_access_rights_relation::TYPE,
            1,
            3,
        );
        relations_kit::array(
            &mut bytes,
            relations_kit::status(0, 3) + crelation_status::SERIALIZABLE_RELATIONS,
            relations_kit::serializable_data(0, 3),
            &[
                relations_kit::RELATION[0] as u64,
                relations_kit::RELATION[1] as u64,
                subject as u64,
                market as u64,
            ],
        );
        let memory = relations_kit::memory(&bytes);
        let direct = relations_kit::direct(&memory, 16, 16);
        assert_eq!(direct.tuple_outcome, Outcome::Rows { len: 2 });
        assert_eq!(
            direct
                .tuple_rows
                .iter()
                .map(|row| (row.type_id, row.ordinal))
                .collect::<Vec<_>>(),
            vec![
                (cguarantee_relation::TYPE, 1),
                (
                    engine::cmarket_access_rights_relation::TYPE,
                    3
                ),
            ]
        );
        let mut intel = relations_kit::intel_captured(16, 16);
        let (_, _) = relations_kit::split(&memory, &mut intel, true);
        relations_kit::assert_matches_direct(&intel, &direct);
    }

    #[test]
    fn relations_raw_compare_is_joint_and_blocks_both_checks_on_any_tear() {
        // Family exception (reported): the typed walk fails BOTH tables on ANY
        // fault, so every raw span gates both outcomes. The raw compare is
        // therefore joint: any span tear blocks both checks (conservative and
        // sound; windows advance slower but never on tearing). Only the
        // match case lets both checks count.
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let mut intel = relations_kit::intel_captured(16, 16);
        let (report, _) = relations_kit::split(&memory, &mut intel, true);
        assert_eq!(report.matched, [true; 2]);
        // One flipped opinion byte tears the shared slot pool: both partitions
        // disagree, even though decoded tuple rows would be identical. Joint
        // gating blocks both checks rather than risk counting a torn refresh.
        let (mut mutated, _) = (relations_kit::opinionated_fixture(), ());
        mutated[relations_kit::status(0, 2) + crelation_status::OPINION] ^= 0x01;
        let (clean, _) = (relations_kit::opinionated_fixture(), ());
        let first_memory = relations_kit::memory(&mutated);
        let second_memory = relations_kit::memory(&clean);
        let intel_probe = relations_kit::intel_captured(16, 16);
        let opinion_cap = intel_probe.opinion_rows_cap();
        let tuple_cap = intel_probe.tuple_rows_cap();
        let mut first = RelationsJob::new();
        let mut second = RelationsJob::new();
        copy_relations_pass(
            &first_memory,
            relations_kit::BASE,
            relations_kit::GS,
            opinion_cap,
            tuple_cap,
            &mut first,
        );
        copy_relations_pass(
            &second_memory,
            relations_kit::BASE,
            relations_kit::GS,
            opinion_cap,
            tuple_cap,
            &mut second,
        );
        assert!(!relations_opinions_equal(&first, &second));
        assert!(!relations_tuples_equal(&first, &second));
        // The full method wires the same joint verdict into the existing
        // mismatch bookkeeping: both tables checked once, both mismatched.
        struct Tearing<'a> {
            clean: SliceMemory<'a>,
            mutated: SliceMemory<'a>,
            opinion_reads: std::cell::Cell<usize>,
        }
        impl engine::Memory for Tearing<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                let opinion =
                    relations_kit::status(0, 2) + crelation_status::OPINION;
                if address <= opinion && opinion < address + destination.len() {
                    let reads = self.opinion_reads.get();
                    self.opinion_reads.set(reads + 1);
                    let memory = if reads < 1 { &self.clean } else { &self.mutated };
                    return memory.copy(address, destination);
                }
                self.clean.copy(address, destination)
            }
        }
        let tearing = Tearing {
            clean: relations_kit::memory(&clean),
            mutated: relations_kit::memory(&mutated),
            opinion_reads: std::cell::Cell::new(0),
        };
        let mut intel = relations_kit::intel_captured(16, 16);
        let mut first = RelationsJob::new();
        let mut second = RelationsJob::new();
        let ctx = RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            opinion_cap: intel.opinion_rows_cap(),
            tuple_cap: intel.tuple_rows_cap(),
            needed: [true, true],
        };
        let report =
            intel.copy_relations_refresh(&tearing, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [false, false]);
        assert_eq!(intel.relation_opinions.mismatches, 1);
        assert_eq!(intel.relation_tuples.mismatches, 1);
        // Without a second copy no check is recorded, mirroring completed
        // windows: nothing checked, nothing mismatched.
        let mut intel = relations_kit::intel_captured(16, 16);
        let mut first = RelationsJob::new();
        let ctx = RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            opinion_cap: intel.opinion_rows_cap(),
            tuple_cap: intel.tuple_rows_cap(),
            needed: [false, false],
        };
        let report =
            intel.copy_relations_refresh(&second_memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        assert!(!intel.relation_opinions.checked);
        assert_eq!(intel.relation_opinions.mismatches, 0);
    }

    #[test]
    fn relations_stable_but_invalid_bytes_count_the_check_with_unavailable_rows() {
        // Readable-but-wrong diplomacy vtable faults decode deterministically
        // on both copies: outcomes agree as Unavailable while the check still
        // counts the refresh (documented window nuance, pinned by this test).
        let (mut bytes, ()) = (relations_kit::fixture(), ());
        bytes[relations_kit::DIPLOMACY[0]] ^= 0xff;
        let memory = relations_kit::memory(&bytes);
        let direct = relations_kit::direct(&memory, 16, 16);
        assert!(matches!(
            direct.opinion_outcome,
            Outcome::Unavailable { .. }
        ));
        let mut intel = relations_kit::intel_captured(16, 16);
        let (report, _) = relations_kit::split(&memory, &mut intel, true);
        assert!(report.matched[0]);
        assert_eq!(intel.relation_opinions.outcome, direct.opinion_outcome);
    }

    #[test]
    fn relations_staging_overflow_reports_fallback_without_blocking() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let mut intel = relations_kit::intel_captured(16, 16);
        // Zero-capacity staging cannot hold any copy: the report declines the
        // payload so the caller falls back to the typed walk.
        let mut tiny = RelationsJob {
            countries: Vec::new(),
            highest_slot: 0,
            count: 0,
            metas: Vec::new(),
            slot_pool: Vec::new(),
            tuples: Vec::new(),
            reader_err: None,
            country_err: None,
            top_err: None,
            unread_opinions: 0,
            unread_tuples: 0,
            overflowed: false,
        };
        let ctx = RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            opinion_cap: intel.opinion_rows_cap(),
            tuple_cap: intel.tuple_rows_cap(),
            needed: [true, true],
        };
        let report = intel.copy_relations_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
        // The typed path still reads the same fixture cleanly.
        let direct = relations_kit::direct(&memory, 16, 16);
        assert_eq!(direct.opinion_outcome, Outcome::Rows { len: 2 });
    }
}
