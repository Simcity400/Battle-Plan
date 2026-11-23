use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native::{Memory, walk::Outcome};
use engine_production::nindustrial_organisation_corganisation as native;
use replay_data::{
    cadence::{SKIPPED_REASON, should_walk},
    rows::{
        IndustrialOrganisationDetailsRow, IndustrialOrganisationHistoryRow,
        IndustrialOrganisationPoliciesRow, IndustrialOrganisationRow, TableId, tables,
    },
};
use replay_frame::frame::TableView;

pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::IndustrialOrganisations,
    TableId::IndustrialOrganisationHistory,
    TableId::IndustrialOrganisationDetails,
    TableId::IndustrialOrganisationPolicies,
];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    organisations: usize,
    history: usize,
    bytes: usize,
}

collector_primitives::capacity_max!(Capacities {
    organisations,
    history,
    bytes
});

impl Capacities {
    /// # Safety
    /// Run only during attach or the active session's serial sizing pass against the authenticated
    /// session image, while embedded variable and flag observer storage remains live and teardown
    /// is excluded.
    pub unsafe fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let (organisations, history, bytes) = native::demand(memory, base, state)?;
        collector_intel::variable_changes::offer_embedded(organisations);
        collector_intel::flag_changes::offer_embedded(organisations);
        crate::table_capture::grow_capacity(&mut self.organisations, organisations);
        crate::table_capture::grow_capacity(&mut self.history, history);
        crate::table_capture::grow_capacity(&mut self.bytes, bytes);
        Ok(())
    }
}

pub struct Captured {
    organisations: CaptureRows<native::Row>,
    history: CaptureRows<native::HistoryRow>,
    details: CaptureRows<native::details::Row>,
    policies: CaptureRows<native::details::CountryPolicies>,
    scratch: native::details::Scratch,
    held: Vec<MioHeld>,
    held_policies: Vec<PolicyHeld>,
    holds_complete: bool,
    nested_bytes: usize,
    bytes: Arena<u8>,
}

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        if should_walk(TABLES[0], hour, force_full) {
            self.holds_complete = false;
        }
        self.organisations
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.history
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        self.details
            .fail(super::failed_capture(TABLES[2], hour, force_full, reason));
        self.policies
            .fail(super::failed_capture(TABLES[3], hour, force_full, reason));
    }
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            organisations: 0,
            history: 0,
            bytes: 0,
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            organisations: CaptureRows::new(capacity.organisations),
            history: CaptureRows::new(capacity.history),
            details: CaptureRows::new(capacity.organisations),
            policies: CaptureRows::new(0),
            scratch: native::details::Scratch::default(),
            held: Vec::new(),
            held_policies: Vec::new(),
            holds_complete: false,
            nested_bytes: 0,
            bytes: Arena::new(capacity.bytes),
        }
    }
    pub fn reset(&mut self) {
        self.organisations.reset();
        self.history.reset();
        self.details.reset();
        self.policies.reset();
        self.bytes.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.organisations.allocated_bytes()
            + self.history.allocated_bytes()
            + self.bytes.allocated_bytes()
            + self.scratch.allocated_bytes()
            + self.held.capacity() * std::mem::size_of::<MioHeld>()
            + self.held_policies.capacity() * std::mem::size_of::<PolicyHeld>()
            + self.policies.allocated_bytes()
            + self.details.allocated_bytes()
            + self.nested_bytes
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.organisations.grow(&mut capacity.organisations);
        self.history.grow(&mut capacity.history);
        self.details.grow(&mut capacity.organisations);
        self.bytes.grow(&mut capacity.bytes);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        if !should_walk(TABLES[0], hour, force_full) {
            self.fail_capture(hour, force_full, SKIPPED_REASON);
            return [0; N];
        }
        let _reads = crate::read_accounting::table(TABLES[0]);
        let started = qpc();
        collector_observers::world_input_changes::prepare_mios(self.details.rows.len(), force_full);
        let mut visitor = DetailsVisitor {
            memory,
            base,
            details: &mut self.details.rows,
            policies: &mut self.policies.rows,
            scratch: &mut self.scratch,
            held: &mut self.held,
            held_policies: &mut self.held_policies,
            fresh: force_full || !self.holds_complete,
            details_len: 0,
            policies_len: 0,
            details_failure: None,
            policies_failure: None,
        };
        let walk = native::walk_with(
            memory,
            base,
            game_state,
            &mut self.organisations.rows,
            &mut self.history.rows,
            &mut self.bytes.first,
            &mut visitor,
        );
        self.details.outcome = visitor.details_failure.map_or(
            Outcome::Rows {
                len: visitor.details_len,
            },
            |reason| Outcome::Unavailable { reason },
        );
        self.policies.outcome = visitor.policies_failure.map_or(
            Outcome::Rows {
                len: visitor.policies_len,
            },
            |reason| Outcome::Unavailable { reason },
        );
        if !matches!(walk.organisations, Outcome::Rows { .. }) {
            self.details.outcome = walk.organisations;
            self.policies.outcome = walk.organisations;
        }
        self.holds_complete = matches!(self.details.outcome, Outcome::Rows { .. })
            && matches!(self.policies.outcome, Outcome::Rows { .. });
        self.nested_bytes = self
            .policies
            .rows
            .iter()
            .map(|row| replay_data::rows::NativeValue::heap_bytes(&row.policies))
            .sum::<usize>()
            + self
                .details
                .rows
                .iter()
                .map(|row| {
                    use replay_data::rows::NativeValue;
                    row.unlocked.capacity() * 4
                        + row.queued.capacity() * 4
                        + row.allowed_policies.capacity() * 4
                        + row.tasks.heap_bytes()
                        + row.variables.heap_bytes()
                        + row.flags.heap_bytes()
                })
                .sum::<usize>();
        self.organisations.outcome = walk.organisations;
        self.history.outcome = walk.history;
        self.bytes.len = walk.bytes_len;
        self.bytes.required = walk.required_bytes;
        [qpc().saturating_sub(started).max(0) as u64, 0, 0, 0]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.organisations.outcome,
            self.history.outcome,
            self.details.outcome,
            self.policies.outcome,
        ]
    }
    pub fn tables(&self) -> [TableView; N] {
        fn text(bytes: &[u8], span: engine_native::cequipment_variant::TextSpan) -> Option<&[u8]> {
            span.start
                .checked_add(span.len)
                .and_then(|end| bytes.get(span.start..end))
        }
        let organisations = match (
            self.bytes.first.get(..self.bytes.len),
            self.organisations
                .rows
                .get(..self.organisations.outcome.len()),
        ) {
            (Some(bytes), Some(rows))
                if rows.iter().all(|row| {
                    text(bytes, row.name).is_some() && text(bytes, row.icon).is_some()
                }) =>
            {
                table::<tables::IndustrialOrganisations, _>(
                    self.organisations.outcome,
                    rows,
                    |row| IndustrialOrganisationRow {
                        organisation: row.reference,
                        country: row.country,
                        ordinal: row.ordinal,
                        owner: row.owner,
                        token: row.token,
                        name: owned_vec(copy_vec(text(bytes, row.name).unwrap())),
                        icon: owned_vec(copy_vec(text(bytes, row.icon).unwrap())),
                    },
                )
            }
            _ => unavailable("organisation strings outside captured arena"),
        };
        [
            organisations,
            table::<tables::IndustrialOrganisationHistory, _>(
                self.history.outcome,
                &self.history.rows,
                |row| IndustrialOrganisationHistoryRow {
                    organisation: row.organisation,
                    variant: row.variant,
                    units: row.units,
                    design_hour: row.design_hour.into(),
                },
            ),
            table::<tables::IndustrialOrganisationDetails, _>(
                self.details.outcome,
                &self.details.rows,
                |row| {
                    use replay_data::rows::NativeValue;
                    IndustrialOrganisationDetailsRow {
                        organisation: row.organisation,
                        country: row.country,
                        state: row.state.clone(),
                        unlocked: owned_vec(copy_vec(&row.unlocked)),
                        queued: owned_vec(copy_vec(&row.queued)),
                        allowed_policies: owned_vec(copy_vec(&row.allowed_policies)),
                        tasks: owned_vec(NativeValue::try_clone(&row.tasks)),
                        variables: replay_data::rows::IndustrialOrganisationVariables {
                            seed: row.variables.seed,
                            entries: owned_vec(
                                NativeValue::try_clone(&row.variables).map(|store| store.entries),
                            ),
                        },
                        flags: owned_vec(NativeValue::try_clone(&row.flags)),
                    }
                },
            ),
            table::<tables::IndustrialOrganisationPolicies, _>(
                self.policies.outcome,
                &self.policies.rows,
                |row| IndustrialOrganisationPoliciesRow {
                    country: row.country,
                    policies: owned_vec(replay_data::rows::NativeValue::try_clone(&row.policies)),
                    cost_modifier_raw: row.cost_modifier_raw.into(),
                },
            ),
        ]
    }
}

type Token = (u64, u64);
#[derive(Default)]
struct MioHeld {
    pointer: usize,
    reference: u64,
    rosters: Option<Token>,
    flags: Option<Token>,
    variables: Option<(u64, u64, u64)>,
}
#[derive(Default)]
struct PolicyHeld {
    status: usize,
    country: u32,
    token: Option<Token>,
}
struct DetailsVisitor<'a, M> {
    memory: &'a M,
    base: usize,
    details: &'a mut Vec<native::details::Row>,
    policies: &'a mut Vec<native::details::CountryPolicies>,
    scratch: &'a mut native::details::Scratch,
    held: &'a mut Vec<MioHeld>,
    held_policies: &'a mut Vec<PolicyHeld>,
    fresh: bool,
    details_len: usize,
    policies_len: usize,
    details_failure: Option<&'static str>,
    policies_failure: Option<&'static str>,
}
fn ensure<T: Default>(rows: &mut Vec<T>, index: usize) -> Result<(), &'static str> {
    if rows.len() <= index {
        engine_allocation::resize_default(rows, index + 1)
            .map_err(|_| "organisation capture allocation failed")?;
    }
    Ok(())
}
impl<M: Memory> native::Visitor for DetailsVisitor<'_, M> {
    fn country(&mut self, identity: &engine_native::ccountry::Country, status: usize) {
        if self.policies_failure.is_some() {
            return;
        }
        let country = identity.id;
        let result = (|| {
            ensure(self.policies, self.policies_len)?;
            ensure(self.held_policies, self.policies_len)?;
            let row = &mut self.policies[self.policies_len];
            let held = &mut self.held_policies[self.policies_len];
            let token = collector_observers::world_input_changes::mio_policies();
            let carry = !self.fresh
                && held.country == country
                && held.status == status
                && token.is_some()
                && token == held.token;
            native::details::capture_policies_selected(
                self.memory,
                self.base,
                status,
                &mut row.policies,
                !carry,
            )?;
            collector_observers::world_input_changes::record_daily(
                collector_observers::world_input_changes::Domain::IndustrialOrganisationPolicies,
                carry,
                !carry,
            );
            row.country = country;
            row.cost_modifier_raw =
                native::details::policy_cost_modifier(self.memory, self.base, identity.pointer)
                    .ok();
            *held = PolicyHeld {
                country,
                status,
                token: token
                    .filter(|_| token == collector_observers::world_input_changes::mio_policies()),
            };
            self.policies_len += 1;
            Ok::<_, &'static str>(())
        })();
        self.policies_failure = result.err();
    }
    fn organisation(&mut self, pointer: usize, identity: &native::Row) {
        if self.details_failure.is_some() {
            return;
        }
        let result = (|| {
            use collector_intel::flag_changes::Changes;
            ensure(self.details, self.details_len)?;
            ensure(self.held, self.details_len)?;
            let row = &mut self.details[self.details_len];
            let held = &mut self.held[self.details_len];
            let same =
                !self.fresh && held.pointer == pointer && held.reference == identity.reference;
            let rosters = collector_observers::world_input_changes::mio_rosters(pointer);
            let variables_at = pointer
                .checked_add(native::details::VARIABLES)
                .ok_or("organisation variable address overflow")?;
            let flags_at = pointer
                .checked_add(native::details::FLAGS)
                .ok_or("organisation flag address overflow")?;
            let variables = collector_intel::variable_changes::embedded_certificate(variables_at);
            let flags = collector_intel::flag_changes::Observer.sequence(flags_at);
            let carry = [
                same && rosters.is_some() && rosters == held.rosters,
                same && variables.is_some() && variables == held.variables,
                same && flags.is_some() && flags == held.flags,
            ];
            let fresh = native::details::Freshness {
                rosters: !carry[0],
                variables: !carry[1],
                flags: !carry[2],
            };
            native::details::capture_selected(
                self.memory,
                self.base,
                pointer,
                identity.reference,
                identity.country,
                row,
                self.scratch,
                fresh,
            )?;
            collector_observers::world_input_changes::record_daily(
                collector_observers::world_input_changes::Domain::IndustrialOrganisationRosters,
                carry[0],
                fresh.rosters,
            );
            *held = MioHeld {
                pointer,
                reference: identity.reference,
                rosters: rosters.filter(|_| {
                    rosters == collector_observers::world_input_changes::mio_rosters(pointer)
                }),
                variables: variables.filter(|_| {
                    variables
                        == collector_intel::variable_changes::embedded_certificate(variables_at)
                }),
                flags: flags.filter(|_| {
                    flags == collector_intel::flag_changes::Observer.sequence(flags_at)
                }),
            };
            self.details_len += 1;
            Ok::<_, &'static str>(())
        })();
        self.details_failure = result.err();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    #[test]
    fn walks_only_on_the_production_lines_slot_or_a_full_hour() {
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        for hour in 0..24 {
            captured.capture(&memory, 0, 0x1000, hour, false);
            let walks = should_walk(TableId::ProductionLines, hour, false);
            for outcome in captured.outcomes() {
                let skipped = outcome
                    == Outcome::Unavailable {
                        reason: SKIPPED_REASON,
                    };
                assert_eq!(skipped, !walks, "{hour}");
            }
        }
        captured.capture(&memory, 0, 0x1000, 0, true);
        assert!(
            captured
                .outcomes()
                .iter()
                .all(|outcome| !matches!(outcome, Outcome::Unavailable { reason } if *reason == SKIPPED_REASON))
        );
    }

    #[test]
    fn conversion_keeps_native_references_bytes_and_signed_units() {
        let mut captured = Captured::with_capacities(&Capacities {
            organisations: 1,
            history: 1,
            bytes: 4,
        });
        captured.bytes.first[..3].copy_from_slice(&[0xff, 0, b'k']);
        captured.bytes.len = 3;
        captured.organisations.rows[0] = native::Row {
            reference: u64::MAX,
            country: u32::MAX,
            ordinal: 7,
            owner: 0x8000_0000,
            token: u32::MAX - 1,
            name: engine_native::cequipment_variant::TextSpan { start: 0, len: 3 },
            icon: engine_native::cequipment_variant::TextSpan { start: 3, len: 0 },
        };
        captured.organisations.outcome = Outcome::Rows { len: 1 };
        captured.history.rows[0] = native::HistoryRow {
            organisation: u64::MAX,
            variant: (5 << 32) | 70,
            units: i32::MIN,
            design_hour: Some(i32::MIN),
        };
        captured.history.outcome = Outcome::Rows { len: 1 };
        let [organisations, history, _details, _policies] = captured.tables();
        let TableView::Complete(TableRows::IndustrialOrganisations(organisations)) = organisations
        else {
            panic!("{organisations:?}")
        };
        assert_eq!(
            organisations[0],
            IndustrialOrganisationRow {
                organisation: u64::MAX,
                country: u32::MAX,
                ordinal: 7,
                owner: 0x8000_0000,
                token: u32::MAX - 1,
                name: vec![0xff, 0, b'k'],
                icon: Vec::new(),
            }
        );
        let TableView::Complete(TableRows::IndustrialOrganisationHistory(history)) = history else {
            panic!("{history:?}")
        };
        assert_eq!(history[0].units, i32::MIN);
        assert_eq!(history[0].design_hour, Some(i32::MIN).into());
        captured.organisations.rows[0].icon.start = 4;
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
        assert!(matches!(captured.tables()[1], TableView::Complete(_)));
        captured.history.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 3,
        };
        let mut capacity = Capacities {
            organisations: 1,
            history: 1,
            bytes: 4,
        };
        captured.grow(&mut capacity);
        assert!(capacity.history >= 4);
    }
}

#[cfg(test)]
mod mutable_projection_tests {
    use super::*;
    use replay_data::rows::TableRows;
    #[test]
    fn mutable_state_projection_keeps_queue_repetitions_task_slots_and_literal_variable_bytes() {
        let mut captured = Captured::with_capacities(&Capacities {
            organisations: 1,
            ..Default::default()
        });
        let row = &mut captured.details.rows[0];
        row.organisation = (17_u64 << 32) | 79;
        row.country = 5;
        row.state.funds_raw = i64::MIN;
        row.state.points = 9;
        row.state.cooldown_hour = Some(-17);
        row.queued = vec![9, 3, 9];
        row.tasks = vec![native::details::Task::default(); 2];
        row.variables.entries = vec![native::details::Variable {
            name: vec![0xff, 0, 7],
            value_raw: i64::MAX,
        }];
        captured.details.outcome = Outcome::Rows { len: 1 };
        let [_, _, details, _] = captured.tables();
        let TableView::Complete(TableRows::IndustrialOrganisationDetails(rows)) = details else {
            panic!("details unavailable")
        };
        assert_eq!(rows[0].state.funds_raw, i64::MIN);
        assert_eq!(rows[0].state.points, 9);
        assert_eq!(rows[0].state.cooldown_hour, Some(-17));
        assert_eq!(rows[0].queued, [9, 3, 9]);
        assert_eq!(rows[0].tasks.len(), 2);
        assert_eq!(rows[0].variables.entries[0].name, [0xff, 0, 7]);
        assert_eq!(rows[0].variables.entries[0].value_raw, i64::MAX);
    }
}

#[cfg(test)]
mod country_scale_tests {
    use super::*;
    use replay_data::rows::TableRows;
    #[test]
    fn mutable_organisation_tables_preserve_distinct_country_links() {
        const COUNTRIES: usize = 6;
        let mut captured = Captured::with_capacities(&Capacities {
            organisations: COUNTRIES,
            ..Default::default()
        });
        for (index, row) in captured.details.rows.iter_mut().enumerate() {
            row.country = index as u32 + 1;
            row.organisation = (u64::from(row.country) << 32) | 79;
            row.state.points = -(index as i32);
            captured
                .policies
                .rows
                .push(native::details::CountryPolicies {
                    country: row.country,
                    policies: vec![native::details::Policy {
                        token: 91,
                        country: row.country | 0x8000_0000,
                        name: vec![0xff, 0],
                        cached_score_raw: -(index as i64),
                        ..Default::default()
                    }],
                    cost_modifier_raw: (index % 2 == 1).then_some(-(index as i64)),
                });
        }
        captured.details.outcome = Outcome::Rows { len: COUNTRIES };
        captured.policies.outcome = Outcome::Rows { len: COUNTRIES };
        let [_, _, details, policies] = captured.tables();
        let TableView::Complete(TableRows::IndustrialOrganisationDetails(details)) = details else {
            panic!("details unavailable")
        };
        let TableView::Complete(TableRows::IndustrialOrganisationPolicies(policies)) = policies
        else {
            panic!("policies unavailable")
        };
        assert_eq!(details.len(), COUNTRIES);
        assert_eq!(policies.len(), COUNTRIES);
        assert_eq!(details[COUNTRIES - 1].country, COUNTRIES as u32);
        assert_eq!(details[COUNTRIES - 1].state.points, 1 - COUNTRIES as i32);
        assert_eq!(
            policies[COUNTRIES - 1].policies[0].country,
            0x8000_0000 | COUNTRIES as u32
        );
        assert_eq!(policies[COUNTRIES - 1].policies[0].name, [0xff, 0]);
        assert_eq!(
            policies[COUNTRIES - 1].cost_modifier_raw,
            Some(1 - COUNTRIES as i64).into()
        );
        assert_eq!(policies[COUNTRIES - 2].cost_modifier_raw, None.into());
    }
}
