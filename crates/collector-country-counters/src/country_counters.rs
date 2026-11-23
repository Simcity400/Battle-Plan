use super::{country_naval_movement_inputs as naval, reserved_visibility};
use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, table_iter, unavailable},
};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::{
    ccountry,
    cname_group_member,
    crelation::status as crelation_status,
    cship,
};
use engine_native_parts::creference_registry;
use replay_data::rows::{
    CountryAttackModifiers, CountryCivilWarRow, CountryCounterInputRow, CountryMovementModifiers,
    ManpowerReferences, Recorded, Retired, TableId, Witnessed, tables,
};
use replay_frame::frame::TableView;

mod carry;
pub use carry::CounterStore;

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::CountryCounterInputs, TableId::CountryCivilWars];

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

pub struct Navies {
    holder: naval::Captured,
    ticks: Option<u64>,
}

impl Navies {
    pub fn allocated_bytes(&self) -> usize {
        self.holder.allocated_bytes()
    }

    pub fn begin<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        base: usize,
        state: usize,
        hour: i32,
        launch: &mut naval::Launch,
    ) -> engine_naval_history::country_naval_movement_inputs::Reader<'a, M> {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        let reader = self.holder.begin(memory, base, state, Some(hour), launch);
        self.ticks = Some(elapsed(begin));
        reader
    }

    pub fn ticks(&self) -> u64 {
        self.ticks.unwrap_or(0)
    }

    pub fn holder(&mut self) -> &mut naval::Captured {
        &mut self.holder
    }

    pub fn finish(&mut self) {
        self.holder.finish();
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    civil_wars: usize,
    states: usize,
    leaders: usize,
    neighbors: usize,
    policies: usize,
    co_belligerents: usize,
    pride_names: usize,
    naval: naval::Capacities,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    civil_wars,
    states,
    leaders,
    neighbors,
    policies,
    co_belligerents,
    pride_names,
    naval,
});

impl Capacities {
    pub fn raise_for_pride_names(&mut self, bytes: usize) {
        crate::country_scale::raise(&mut self.pride_names, crate::country_scale::rows(bytes, 1));
    }

    pub fn raise_for_physical_navies(&mut self, demand: naval::Capacities) {
        self.naval.raise(demand);
    }
    pub fn raise_for_co_belligerents(&mut self, targets: usize) {
        crate::country_scale::raise(
            &mut self.co_belligerents,
            crate::country_scale::rows(targets, 1),
        );
    }

    pub fn raise_for_effect_rosters(&mut self, demand: ccountry::EffectRosterDemand) {
        crate::country_scale::raise(
            &mut self.neighbors,
            crate::country_scale::rows(demand.neighbors, 1),
        );
        crate::country_scale::raise(
            &mut self.policies,
            crate::country_scale::rows(demand.policies, 1),
        );
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.rows, crate::country_scale::rows(slots, 1));
    }

    pub fn raise_for_civil_wars(&mut self, entries: usize) {
        crate::country_scale::raise(&mut self.civil_wars, crate::country_scale::rows(entries, 1));
    }

    pub fn raise_for_rosters(&mut self, rosters: ccountry::RosterDemand) {
        let raise = crate::country_scale::raise;
        let rows = crate::country_scale::rows;
        raise(&mut self.states, rows(rosters.states, 1));
        raise(&mut self.leaders, rows(rosters.leaders, 1));
    }
}

pub struct Captured {
    rows: CaptureRows<ccountry::CounterInputsRow>,
    civil_wars: CaptureRows<ccountry::CivilWarRow>,
    states: Arena<u32>,
    leaders: Arena<u64>,
    neighbors: Arena<u32>,
    policies: Arena<u32>,
    relations: Arena<RelationRow>,
    co_belligerents: Arena<u32>,
    pride_names: Arena<u8>,
    naval: naval::Captured,
    manpower_references: std::sync::Mutex<Vec<(u32, ManpowerReferences)>>,
}

#[derive(Default, Clone, Copy)]
struct RelationRow {
    pride_name: Option<cname_group_member::Span>,
    pride_name_reason: Option<&'static str>,
    co_belligerents: Option<ccountry::RosterSpan>,
    zero_war: Option<[u8; 2]>,
    co_reason: Option<&'static str>,
    zero_reason: Option<&'static str>,
    zero_subject: Option<replay_data::rows::ZeroSubjectInputs>,
    zero_subject_reason: Option<&'static str>,
    forward_visibility: Option<replay_data::rows::ReverseZeroVisibilityInputs>,
    forward_visibility_reason: Option<&'static str>,
    forward_visibility_failures: [Option<&'static str>; 4],
}

impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            civil_wars: 0,
            ..Default::default()
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            civil_wars: CaptureRows::new(capacity.civil_wars),
            states: Arena::new(capacity.states),
            leaders: Arena::new(capacity.leaders),
            neighbors: Arena::new(capacity.neighbors),
            policies: Arena::new(capacity.policies),
            relations: Arena::new(capacity.rows),
            co_belligerents: Arena::new(capacity.co_belligerents),
            pride_names: Arena::new(capacity.pride_names),
            naval: naval::Captured::new(capacity.naval),
            manpower_references: std::sync::Mutex::new(Vec::new()),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.civil_wars.reset();
        self.states.reset();
        self.leaders.reset();
        self.neighbors.reset();
        self.policies.reset();
        self.relations.reset();
        self.co_belligerents.reset();
        self.pride_names.reset();
        self.naval.reset();
    }
    pub fn fail(&mut self) {
        let demand = self.naval.required();
        self.reset();
        self.naval.retain_required(demand);
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.civil_wars.allocated_bytes()
            + self.states.allocated_bytes()
            + self.leaders.allocated_bytes()
            + self.neighbors.allocated_bytes()
            + self.policies.allocated_bytes()
            + self.relations.allocated_bytes()
            + self.co_belligerents.allocated_bytes()
            + self.pride_names.allocated_bytes()
            + self.naval.allocated_bytes()
            + {
                let cache = self.manpower_references.lock().unwrap();
                cache.capacity() * size_of::<(u32, ManpowerReferences)>()
                    + cache
                        .iter()
                        .map(|(_, values)| values.allocated_bytes())
                        .sum::<usize>()
            }
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        self.civil_wars.grow(&mut capacity.civil_wars);
        self.states.grow(&mut capacity.states);
        self.leaders.grow(&mut capacity.leaders);
        self.neighbors.grow(&mut capacity.neighbors);
        self.policies.grow(&mut capacity.policies);
        self.relations.grow(&mut capacity.rows);
        self.co_belligerents.grow(&mut capacity.co_belligerents);
        self.pride_names.grow(&mut capacity.pride_names);
        self.naval.grow(&mut capacity.naval);
    }
    pub fn take_navies(&mut self) -> Navies {
        Navies {
            holder: std::mem::replace(&mut self.naval, naval::Captured::new(Default::default())),
            ticks: None,
        }
    }

    pub fn restore_navies(&mut self, navies: Navies) {
        self.naval = navies.holder;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.capture_with_reverse(memory, base, state, None)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn capture_with_reverse(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        reverse: Option<reserved_visibility::Request<'_>>,
    ) -> [u64; N] {
        self.capture_carried(memory, base, state, reverse, None)
    }

    pub fn capture_carried(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        reverse: Option<reserved_visibility::Request<'_>>,
        mut store: Option<&mut CounterStore>,
    ) -> [u64; N] {
        let mut ticks = [0; N];
        {
            let _reads = crate::read_accounting::table(TABLES[0]);
            let begin = qpc();
            if let Some(store) = store.as_deref_mut() {
                store.begin(memory, base, state);
            }
            let mut rosters = ccountry::CounterRosters {
                capture_sent_expeditionaries: store.as_deref().map_or(
                    cfg!(any(test, feature = "test-support")),
                    CounterStore::has_certificate,
                ),
                states: engine_native_memory::walk::Buffer::Growing(&mut self.states.first),
                leaders: engine_native_memory::walk::Buffer::Growing(&mut self.leaders.first),
                state_len: 0,
                leader_len: 0,
                required: Default::default(),
            };
            let mut effects = ccountry::EffectRosters {
                neighbors: engine_native_memory::walk::Buffer::Growing(&mut self.neighbors.first),
                policies: engine_native_memory::walk::Buffer::Growing(&mut self.policies.first),
                neighbor_len: 0,
                policy_len: 0,
                required: Default::default(),
            };
            let mut prefix_reader = None;
            self.rows.outcome = match store.as_deref_mut() {
                Some(store) => ccountry::walk_counter_inputs_carried(
                    memory,
                    base,
                    state,
                    &mut self.rows.rows,
                    &mut rosters,
                    &mut effects,
                    &mut prefix_reader,
                    &mut |country: engine_native_memory::country_identity::Country,
                          rosters: &mut ccountry::CounterRosters<'_>,
                          effects: &mut ccountry::EffectRosters<'_>| {
                        store.offer(memory, country, rosters, effects)
                    },
                ),
                None => ccountry::walk_counter_inputs_with_effect_rosters_observed(
                    memory,
                    base,
                    state,
                    &mut self.rows.rows,
                    &mut rosters,
                    &mut effects,
                    &mut prefix_reader,
                ),
            };
            self.states.len = rosters.state_len;
            self.states.required = rosters.required.states;
            self.leaders.len = rosters.leader_len;
            self.leaders.required = rosters.required.leaders;
            self.neighbors.len = effects.neighbor_len;
            self.neighbors.required = effects.required.neighbors;
            self.policies.len = effects.policy_len;
            self.policies.required = effects.required.policies;
            let reverse_ticks = self.capture_relations_with_reverse(
                memory,
                base,
                state,
                reverse,
                prefix_reader.as_ref(),
                store.as_deref(),
            );
            if let Some(store) = store {
                store.finish(self);
            }
            ticks[0] = elapsed(begin).saturating_sub(reverse_ticks);
        }
        {
            let _reads = crate::read_accounting::table(TABLES[1]);
            let begin = qpc();
            self.civil_wars.outcome =
                ccountry::walk_civil_wars(memory, base, state, &mut self.civil_wars.rows);
            ticks[1] = qpc().saturating_sub(begin).max(0) as u64;
        }
        ticks
    }

    #[cfg(test)]
    fn capture_relations(&mut self, memory: &impl Memory, base: usize, state: usize) {
        self.capture_relations_with_reverse(memory, base, state, None, None, None);
    }

    fn capture_relations_with_reverse<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        mut reverse: Option<reserved_visibility::Request<'_>>,
        prefix_reader: Option<&Result<ccountry::Reader<'_, M>, &'static str>>,
        store: Option<&CounterStore>,
    ) -> u64 {
        self.relations.reset();
        self.co_belligerents.reset();
        self.pride_names.reset();
        let len = self.rows.outcome.len();
        self.relations.required = len;
        if let Err(reason) =
            engine_native_memory::walk::Buffer::Growing(&mut self.relations.first).ensure(len)
        {
            self.rows.outcome = Outcome::Unavailable { reason };
            return 0;
        }
        let source = reverse.as_mut().and_then(|request| {
            request.capture.prepare(
                memory,
                request.hour,
                request.observation.take(),
                request.prefix_ready,
            )
        });
        let positive_reader = (len != 0).then(|| ccountry::Reader::new(memory, base, state));
        let countries = positive_reader.as_ref().or(prefix_reader);
        if let (Some(request), Some(source)) = (reverse.as_mut(), source.as_ref()) {
            request.capture.capture_cell(
                source,
                memory,
                base,
                countries.and_then(|countries| countries.as_ref().ok()),
                0,
                None,
            );
        }
        if len == 0 {
            return reverse
                .as_ref()
                .map_or(0, |request| request.capture.ticks());
        }
        let countries = countries.expect("nonempty positive prefix above");
        let mut references = creference_registry::ScopedResolver::new(base);
        for (index, prefix) in self.rows.rows.iter().take(len).enumerate() {
            let Some(row) = self.relations.first.get_mut(index) else {
                break;
            };
            *row = RelationRow::default();
            match capture_pride_name(
                memory,
                base,
                prefix.pride_of_the_fleet,
                &mut references,
                &mut self.pride_names,
            ) {
                Ok(span) => row.pride_name = span,
                Err(reason) => row.pride_name_reason = Some(reason),
            }
            let country = countries
                .as_ref()
                .map_err(|&reason| reason)
                .and_then(|countries| countries.resolve(prefix.country));
            let country = match country {
                Ok(country) => country,
                Err(reason) => {
                    row.co_reason = Some(reason);
                    row.zero_reason = Some(reason);
                    row.zero_subject_reason = Some(reason);
                    row.forward_visibility_reason = Some(reason);
                    self.relations.len = index + 1;
                    continue;
                }
            };
            if let Some((held, targets)) =
                store.and_then(|store| store.carried_relation(index, len, prefix.country))
            {
                let start = self.co_belligerents.len;
                let mut output = engine_native_memory::walk::Buffer::GrowingFrom(&mut self.co_belligerents.first, start);
                if output.ensure(targets.len()) == Ok(true) && targets.len() <= output.len() {
                    output[..targets.len()].copy_from_slice(targets);
                    *row = RelationRow {
                        pride_name: row.pride_name,
                        pride_name_reason: row.pride_name_reason,
                        co_belligerents: Some(ccountry::RosterSpan {
                            start,
                            count: targets.len(),
                        }),
                        ..held
                    };
                    self.co_belligerents.len += targets.len();
                    self.co_belligerents.required =
                        self.co_belligerents.required.saturating_add(targets.len());
                    if let (Some(request), Some(source)) = (reverse.as_mut(), source.as_ref()) {
                        request.capture.capture_cell(
                            source,
                            memory,
                            base,
                            countries.as_ref().ok(),
                            country.canonical_slot,
                            None,
                        );
                    }
                    self.relations.len = index + 1;
                    continue;
                }
            }
            let mut forward = crelation_status::ForwardZeroObservation::default();
            let targets = {
                engine_native_memory::walk::Buffer::GrowingFrom(
                    &mut self.co_belligerents.first,
                    self.co_belligerents.len,
                )
            };
            let inputs = crelation_status::co_belligerent_inputs_observed_buffer(
                memory,
                base,
                countries.as_ref().expect("authenticated above"),
                country,
                targets,
                &mut forward,
            );
            let visibility = crelation_status::forward_zero_visibility_inputs(
                memory,
                base,
                countries.as_ref().expect("authenticated above"),
                &mut forward,
            );
            row.forward_visibility_reason = visibility.value.err();
            row.forward_visibility_failures = visibility.failures;
            row.forward_visibility =
                visibility
                    .value
                    .ok()
                    .map(|value| replay_data::rows::ReverseZeroVisibilityInputs {
                        target: value.target,
                        war: value.war,
                        subject: value.subject.map(|subject| {
                            replay_data::rows::ZeroSubjectInputs {
                                relation_type: subject.relation_type,
                                parties: subject.parties,
                            }
                        }),
                        military_access: value.military_access,
                    });
            if let (Some(request), Some(source)) = (reverse.as_mut(), source.as_ref()) {
                request.capture.capture_cell(
                    source,
                    memory,
                    base,
                    countries.as_ref().ok(),
                    country.canonical_slot,
                    Some(&forward),
                );
            }
            self.co_belligerents.required = self
                .co_belligerents
                .required
                .saturating_add(inputs.required_targets);
            match inputs.targets {
                Ok(count) => {
                    row.co_belligerents = Some(ccountry::RosterSpan {
                        start: self.co_belligerents.len,
                        count,
                    });
                    self.co_belligerents.len += count;
                }
                Err(reason) => row.co_reason = Some(reason),
            }
            match inputs.zero_war {
                Ok(value) => row.zero_war = Some(value),
                Err(reason) => row.zero_reason = Some(reason),
            }
            match inputs.zero_subject {
                Ok(value) => {
                    row.zero_subject_reason = value.read_refusal();
                    row.zero_subject = Some(replay_data::rows::ZeroSubjectInputs {
                        relation_type: value.relation_type,
                        parties: value.parties,
                    });
                }
                Err(reason) => row.zero_subject_reason = Some(reason),
            }
            self.relations.len = index + 1;
        }
        reverse
            .as_ref()
            .map_or(0, |request| request.capture.ticks())
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome, self.civil_wars.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        [
            self.counter_table(),
            table::<tables::CountryCivilWars, _>(
                self.civil_wars.outcome,
                &self.civil_wars.rows,
                |r| CountryCivilWarRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    target: r.target,
                },
            ),
        ]
    }

    fn counter_table(&self) -> TableView {
        let Some(rows) = self.rows.rows.get(..self.rows.outcome.len()) else {
            return unavailable("country counter row count exceeds allocated buffer");
        };
        for row in rows {
            if row.state_rosters.is_some_and(|spans| {
                spans
                    .iter()
                    .any(|span| !valid_span(&self.states, span.start, span.count))
            }) {
                return unavailable("country state roster span outside acquired arena");
            }
            if row.leader_rosters.is_some_and(|spans| {
                spans
                    .iter()
                    .any(|span| !valid_span(&self.leaders, span.start, span.count))
            }) {
                return unavailable("country leader roster span outside acquired arena");
            }
        }
        table_iter::<tables::CountryCounterInputs, _>(
            self.rows.outcome,
            rows.iter().enumerate(),
            |(index, r)| {
                let relation = self
                    .relations
                    .first
                    .get(index)
                    .filter(|_| index < self.relations.len);
                CountryCounterInputRow {
                    directed_wars: Recorded::Value(None),
                    country: r.country,
                    recruitment_modifiers: Recorded::Value(r.recruitment_modifiers),
                    air_modifiers: Recorded::Value(r.air_modifiers),
                    air_combat_modifiers_known: (r.air_combat_modifiers.is_some()).into(),
                    air_superiority_efficiency: (r
                        .air_combat_modifiers
                        .and_then(|values| values[0]))
                    .into(),
                    air_intercept_efficiency: (r.air_combat_modifiers.and_then(|values| values[1]))
                        .into(),
                    air_cas_efficiency: (r.air_combat_modifiers.and_then(|values| values[2]))
                        .into(),
                    air_nav_efficiency: (r.air_combat_modifiers.and_then(|values| values[3]))
                        .into(),
                    air_attack_factor: (r.air_combat_modifiers.and_then(|values| values[4])).into(),
                    air_defence_factor: (r.air_combat_modifiers.and_then(|values| values[5]))
                        .into(),
                    air_strategic_bomber_defence_factor: (r
                        .air_combat_modifiers
                        .and_then(|values| values[6]))
                    .into(),
                    mines_effect_reduction: Recorded::Value(r.mines_effect_reduction),
                    naval_detection: Recorded::Value(r.naval_detection),
                    attack: Recorded::Value(Witnessed(None)),
                    attack_modifiers: Recorded::Value(r.attack_modifiers.map(|[night, bunker]| {
                        CountryAttackModifiers {
                            land_night_attack: night,
                            land_bunker_effectiveness_factor: bunker,
                        }
                    })),
                    combat_width_factor: Recorded::Value(
                        r.movement_modifiers.and_then(|entries| entries[6]),
                    ),
                    movement_modifiers: Recorded::Value(r.movement_modifiers.map(
                        |[
                            army_speed_factor,
                            army_armor_speed_factor,
                            attrition,
                            winter_attrition_factor,
                            heat_attrition_factor,
                            out_of_supply_factor,
                            _combat_width_factor,
                            special_forces_out_of_supply_factor,
                            army_attack_speed_factor,
                            army_retreat_speed_factor,
                        ]| CountryMovementModifiers {
                            army_speed_factor,
                            army_armor_speed_factor,
                            attrition,
                            winter_attrition_factor,
                            heat_attrition_factor,
                            out_of_supply_factor,
                            special_forces_out_of_supply_factor,
                            army_attack_speed_factor,
                            army_retreat_speed_factor,
                        },
                    )),
                    pride_of_the_fleet: r.pride_of_the_fleet,
                    owned_state_count: r.owned_state_count,
                    controlled_state_count: r.controlled_state_count,
                    first_division: r.first_division,
                    owned_states: Recorded::Value(
                        r.state_rosters.map(|spans| self.state_list(spans[0])),
                    ),
                    controlled_states: Recorded::Value(
                        r.state_rosters.map(|spans| self.state_list(spans[1])),
                    ),
                    core_states: Recorded::Value(
                        r.state_rosters.map(|spans| self.state_list(spans[2])),
                    ),
                    army_leaders: Recorded::Value(
                        r.leader_rosters.map(|spans| self.leader_list(spans[0])),
                    ),
                    sent_expeditionaries: Recorded::Value(
                        r.sent_expeditionaries
                            .and_then(|span| self.manpower_reference_list(r.country, span)),
                    ),
                    navy_leaders: Recorded::Value(
                        r.leader_rosters.map(|spans| self.leader_list(spans[1])),
                    ),
                    mio_policies: Recorded::Value(
                        r.mio_policies
                            .and_then(|span| self.effect_list(&self.policies, span)),
                    ),
                    neighbors: Recorded::Value(
                        r.neighbors
                            .and_then(|span| self.effect_list(&self.neighbors, span)),
                    ),
                    co_belligerents: Recorded::Value(
                        relation
                            .and_then(|row| row.co_belligerents)
                            .and_then(|span| self.effect_list(&self.co_belligerents, span)),
                    ),
                    zero_war: Recorded::Value(relation.and_then(|row| row.zero_war)),
                    movement: Recorded::Value(Retired),
                    zero_subject: Recorded::Value(relation.and_then(|row| row.zero_subject)),
                    forward_visibility: Recorded::Value(
                        relation.and_then(|row| row.forward_visibility),
                    ),
                    pride_name: Recorded::Value(relation.and_then(|row| row.pride_name).and_then(
                        |(start, count)| {
                            valid_span(&self.pride_names, start, count).then(|| {
                                engine_allocation::owned_vec(engine_allocation::copy_vec(
                                    &self.pride_names.first[start..start + count],
                                ))
                            })
                        },
                    )),
                }
            },
        )
    }

    fn manpower_reference_list(
        &self,
        country: u32,
        span: ccountry::RosterSpan,
    ) -> Option<ManpowerReferences> {
        if !valid_span(&self.leaders, span.start, span.count) {
            return None;
        }
        let values = &self.leaders.first[span.start..span.start + span.count];
        let mut cache = self.manpower_references.lock().unwrap();
        let index = match cache.binary_search_by_key(&country, |(country, _)| *country) {
            Ok(index) => index,
            Err(index) => {
                if let Err(error) = engine_allocation::reserve(&mut cache, 1) {
                    engine_allocation::owned_vec::<()>(Err(error));
                    return None;
                }
                cache.insert(index, (country, ManpowerReferences::default()));
                index
            }
        };
        if &*cache[index].1 != values {
            cache[index].1 =
                engine_allocation::owned_vec(engine_allocation::copy_vec(values)).into();
        }
        Some(cache[index].1.clone())
    }

    fn state_list(&self, span: ccountry::RosterSpan) -> Vec<u32> {
        engine_allocation::owned_vec(engine_allocation::copy_vec(
            &self.states.first[span.start..span.start + span.count],
        ))
    }

    fn leader_list(&self, span: ccountry::RosterSpan) -> Vec<u64> {
        engine_allocation::owned_vec(engine_allocation::copy_vec(
            &self.leaders.first[span.start..span.start + span.count],
        ))
    }

    fn effect_list(&self, arena: &Arena<u32>, span: ccountry::RosterSpan) -> Option<Vec<u32>> {
        valid_span(arena, span.start, span.count).then(|| {
            engine_allocation::owned_vec(engine_allocation::copy_vec(
                &arena.first[span.start..span.start + span.count],
            ))
        })
    }
}

fn capture_pride_name(
    memory: &impl Memory,
    base: usize,
    handle: u64,
    references: &mut creference_registry::ScopedResolver,
    names: &mut Arena<u8>,
) -> Result<Option<cname_group_member::Span>, &'static str> {
    if handle == 0 {
        return Ok(None);
    }
    let Some(pointer) = references.resolve(memory, handle)? else {
        return Ok(None);
    };
    let span = cship::cached_name_growing(
        memory,
        base,
        pointer,
        handle,
        &mut names.first,
        &mut names.len,
        &mut names.required,
    )?
    .ok_or("pride ship name arena capacity exceeded")?;
    names.len += span.1;
    Ok(Some(span))
}

fn valid_span<T>(arena: &Arena<T>, start: usize, count: usize) -> bool {
    start
        .checked_add(count)
        .is_some_and(|end| end <= arena.len && end <= arena.first.len())
}

pub fn co_belligerent_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> (usize, Option<&'static str>) {
    let countries = match ccountry::Reader::new(memory, base, state) {
        Ok(countries) => countries,
        Err(reason) => return (0, Some(reason)),
    };
    let mut required = 0_usize;
    let mut refusal = None;
    for slot in 1..countries.count() {
        let observed = countries
            .at(slot)
            .and_then(|country| {
                country
                    .map(|country| crelation_status::co_belligerent_demand(memory, base, country))
                    .transpose()
            })
            .and_then(|count| {
                count
                    .map(|count| {
                        required
                            .checked_add(count)
                            .ok_or("co-belligerent sizing demand overflow")
                    })
                    .transpose()
            });
        match observed {
            Ok(Some(total)) => required = total,
            Ok(None) => {}
            Err(reason) => {
                refusal.get_or_insert(reason);
            }
        }
    }
    (required, refusal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const PRIDE_HANDLE: u64 = (91_u64 << 32) | cship::HANDLE_TYPE as u64;
    const PRIDE_SHIP: usize = 0x17000;
    const PRIDE_MEMBER: usize = 0x18000;
    const PRIDE_TABLE: usize = 0x19000;
    const PRIDE_ENTRIES: usize = 0x1a000;
    const PRIDE_HEAP: usize = 0x1b000;

    struct PrideMemory<'a> {
        bytes: &'a [u8],
        root: u64,
        denied: Option<usize>,
        root_reads: std::cell::Cell<usize>,
    }

    impl Memory for PrideMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if self
                .denied
                .is_some_and(|at| address <= at && at < address.saturating_add(destination.len()))
            {
                return false;
            }
            let root = creference_registry::LOW_ROOTS_RVA.0 + cship::HANDLE_TYPE as usize * 8;
            if address == root && destination.len() == 8 {
                self.root_reads.set(self.root_reads.get() + 1);
                destination.copy_from_slice(&self.root.to_le_bytes());
                return true;
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }

    fn pride_memory(bytes: &[u8], denied: Option<usize>) -> PrideMemory<'_> {
        PrideMemory {
            bytes,
            root: PRIDE_TABLE as u64,
            denied,
            root_reads: 0.into(),
        }
    }

    fn pride_world(text: &[u8]) -> Vec<u8> {
        use engine_native_parts::reference_bucket as bucket;
        let mut bytes = co_world();
        co_put64(&mut bytes, PRIDE_SHIP, cship::VTABLE_RVA.0 as u64);
        co_put64(&mut bytes, PRIDE_SHIP + cship::HANDLE, PRIDE_HANDLE);
        co_put64(
            &mut bytes,
            PRIDE_SHIP + cship::NAME_MEMBER,
            PRIDE_MEMBER as u64,
        );
        co_put64(
            &mut bytes,
            PRIDE_MEMBER,
            cname_group_member::VTABLE_RVA.0 as u64,
        );
        let string = PRIDE_MEMBER + cname_group_member::CACHED_NAME;
        co_put64(&mut bytes, string + 16, text.len() as u64);
        if text.len() <= 15 {
            bytes[string..string + text.len()].copy_from_slice(text);
            co_put64(&mut bytes, string + 24, 15);
        } else {
            co_put64(&mut bytes, string, PRIDE_HEAP as u64);
            co_put64(&mut bytes, string + 24, text.len() as u64);
            bytes[PRIDE_HEAP..PRIDE_HEAP + text.len()].copy_from_slice(text);
        }
        co_put64(
            &mut bytes,
            PRIDE_TABLE + creference_registry::ENTRIES,
            PRIDE_ENTRIES as u64,
        );
        co_put32(&mut bytes, PRIDE_TABLE + creference_registry::COUNT, 1);
        co_put32(&mut bytes, PRIDE_TABLE + creference_registry::MASK, 7);
        bytes[PRIDE_TABLE + creference_registry::OVERFLOW_DISTANCE] = 5;
        bytes[PRIDE_ENTRIES + 13 * bucket::STRIDE + bucket::DISTANCE] = 255;
        let hash = PRIDE_HANDLE
            .rotate_left(32)
            .wrapping_mul(creference_registry::HASH_MULTIPLIER);
        let first = ((hash >> 32) as u32 ^ hash as u32) & 7;
        let at = PRIDE_ENTRIES + first as usize * bucket::STRIDE;
        bytes[at + bucket::DISTANCE] = 1;
        co_put64(&mut bytes, at + bucket::TYPE, PRIDE_HANDLE);
        co_put64(&mut bytes, at + bucket::OBJECT, PRIDE_SHIP as u64);
        bytes
    }

    #[test]
    fn pride_name_zero_is_lazy_and_known_stale_is_not_a_read_refusal() {
        struct NoRead;
        impl Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("zero pride reference attempted a read")
            }
        }
        let mut names = Arena::new(0);
        let mut references = creference_registry::ScopedResolver::new(0);
        assert_eq!(
            capture_pride_name(&NoRead, 0, 0, &mut references, &mut names),
            Ok(None)
        );
        let memory = PrideMemory {
            bytes: &[],
            root: 0,
            denied: None,
            root_reads: 0.into(),
        };
        assert_eq!(
            capture_pride_name(&memory, 0, PRIDE_HANDLE, &mut references, &mut names),
            Ok(None)
        );
        assert_eq!(
            (names.len, names.required, memory.root_reads.get()),
            (0, 0, 1)
        );
        let bytes = pride_world(b"unused");
        let memory = pride_memory(&bytes, None);
        let mut references = creference_registry::ScopedResolver::new(0);
        assert_eq!(
            capture_pride_name(
                &memory,
                0,
                PRIDE_HANDLE + (1_u64 << 32),
                &mut references,
                &mut names
            ),
            Ok(None)
        );
        assert_eq!((names.len, names.required), (0, 0));
    }

    #[test]
    fn pride_name_grows_on_the_first_descriptor_and_relation_failure_preserves_current_bytes() {
        let text = b"Current\0peaceful ship name with suffix";
        let bytes = pride_world(text);
        let memory = pride_memory(
            &bytes,
            Some(CO_STATE + engine_native::cgame_state::COUNTRIES),
        );
        let mut capture = co_capture(4);
        capture.rows.rows[0].pride_of_the_fleet = PRIDE_HANDLE;
        capture.capture_relations(&memory, 0, CO_STATE);
        assert_eq!(capture.rows.outcome, Outcome::Rows { len: 2 });
        assert_eq!(capture.pride_names.required, text.len());
        assert_eq!(capture.relations.first[0].pride_name_reason, None);
        assert_eq!(&capture.pride_names.first[..capture.pride_names.len], text);
        assert!(capture.relations.first[0].co_reason.is_some());
        assert_eq!(capture.relations.first[1].pride_name_reason, None);
        let mut capacity = Capacities {
            rows: 2,
            co_belligerents: 4,
            ..Default::default()
        };
        capture.grow(&mut capacity);
        assert!(capacity.pride_names >= text.len());
        capture.capture_relations(&memory, 0, CO_STATE);
        let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
            capture.counter_table()
        else {
            panic!("prefix lost")
        };
        assert_eq!(rows[0].pride_of_the_fleet, PRIDE_HANDLE);
        assert_eq!(rows[0].owned_state_count, 7);
        assert_eq!(rows[0].co_belligerents, Recorded::Value(None));
        assert_eq!(rows[0].pride_name, Recorded::Value(Some(text.to_vec())));
        assert_eq!(rows[1].pride_name, Recorded::Value(None));
        capture.pride_names.first.fill(0xff);
        assert_eq!(rows[0].pride_name, Recorded::Value(Some(text.to_vec())));
    }

    #[test]
    fn pride_name_empty_and_identity_fault_keep_country_and_relation_fields() {
        let mut bytes = pride_world(b"");
        let mut capture = co_capture(4);
        capture.rows.rows[0].pride_of_the_fleet = PRIDE_HANDLE;
        capture.capture_relations(&pride_memory(&bytes, None), 0, CO_STATE);
        assert_eq!(capture.pride_names.required, 0);
        assert_eq!(capture.relations.first[0].pride_name, Some((0, 0)));
        let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
            capture.counter_table()
        else {
            panic!("prefix lost")
        };
        assert_eq!(rows[0].pride_name, Recorded::Value(Some(vec![])));
        bytes[PRIDE_SHIP + cship::HANDLE + 4] ^= 1;
        capture.capture_relations(&pride_memory(&bytes, None), 0, CO_STATE);
        assert!(capture.relations.first[0].pride_name_reason.is_some());
        assert_eq!(capture.relations.first[0].zero_war, Some([0, 0]));
        assert_eq!(capture.rows.rows[0].pride_of_the_fleet, PRIDE_HANDLE);
        assert_eq!(capture.pride_names.required, 0);
    }

    const CO_STATE: usize = 0x1000;
    const CO_COUNTRIES: [usize; 2] = [0x4000, 0x8000];
    const CO_DIPLOMACY: [usize; 2] = [0xc000, 0xd000];
    fn co_status(owner: usize, target: usize) -> usize {
        0x10000 + owner * 0x4000 + target * 0x800
    }
    fn co_put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn co_put64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn co_header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        co_put64(bytes, at, data as u64);
        co_put32(bytes, at + 8, count);
        co_put32(bytes, at + 12, count);
    }
    fn co_world() -> Vec<u8> {
                use engine_native::cgame_state;
        let mut bytes = vec![0_u8; 0x20000];
        co_header(&mut bytes, CO_STATE + cgame_state::COUNTRIES, 0x3000, 3);
        co_put64(&mut bytes, 0x3000, u64::MAX);
        for (index, pointer) in CO_COUNTRIES.into_iter().enumerate() {
            co_put64(&mut bytes, 0x3008 + index * 8, pointer as u64);
            co_put64(&mut bytes, pointer, ccountry::VTABLE_RVA.0 as u64);
            co_put32(&mut bytes, pointer + ccountry::ID, index as u32 + 1);
            co_put64(
                &mut bytes,
                pointer + ccountry::DIPLOMACY_STATUS,
                CO_DIPLOMACY[index] as u64,
            );
        }
        co_header(&mut bytes, CO_STATE + cgame_state::COUNTRY_LINKS, 0x3100, 4);
        for (index, value) in [0, 1, 2, 2].into_iter().enumerate() {
            co_put32(&mut bytes, 0x3100 + index * 4, value);
        }
        for owner in 0..2 {
            let root = CO_DIPLOMACY[owner];
            co_put64(&mut bytes, root, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64);
            co_put64(
                &mut bytes,
                root + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
                CO_COUNTRIES[owner] as u64,
            );
            let matrix = 0x3200 + owner * 0x40;
            co_header(&mut bytes, root + engine_native_parts::cdiplomacy_status_layout::RELATIONS, matrix, 3);
            for target in 0..3 {
                let status = co_status(owner, target);
                co_put64(&mut bytes, matrix + target * 8, status as u64);
                co_put64(&mut bytes, status, crelation_status::VTABLE_RVA.0 as u64);
                co_put64(
                    &mut bytes,
                    status + crelation_status::DIPLOMACY_STATUS,
                    root as u64,
                );
                co_put32(
                    &mut bytes,
                    status + crelation_status::TARGET_COUNTRY,
                    target as u32,
                );
            }
            bytes[co_status(owner, 0) + crelation_status::CO_BELLIGERENT] = 0x80;
        }
        co_header(
            &mut bytes,
            CO_DIPLOMACY[0] + engine_native_parts::cdiplomacy_status_layout::WARTIME_ALLIES,
            0x3300,
            2,
        );
        co_put32(&mut bytes, 0x3300, 3);
        co_put32(&mut bytes, 0x3304, 2);
        co_put32(
            &mut bytes,
            co_status(0, 2) + crelation_status::TARGET_COUNTRY,
            3,
        );
        bytes[co_status(0, 2) + crelation_status::CO_BELLIGERENT] = 0x7f;
        bytes
    }
    fn co_capture(capacity: usize) -> Captured {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 2,
            co_belligerents: capacity,
            ..Default::default()
        });
        for (index, row) in capture.rows.rows.iter_mut().enumerate() {
            *row = ccountry::CounterInputsRow {
                country: index as u32 + 1,
                owned_state_count: 7 + index as i32,
                ..Default::default()
            };
        }
        capture.rows.outcome = Outcome::Rows { len: 2 };
        capture
    }

    #[test]
    fn relation_suffix_grows_and_preserves_later_country_and_independent_zero_war() {
        let mut bytes = co_world();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut capture = co_capture(1);
        capture.capture_relations_with_reverse(&memory, 0, CO_STATE, None, None, None);
        assert_eq!(capture.rows.outcome, Outcome::Rows { len: 2 });
        assert_eq!(
            (
                capture.co_belligerents.len,
                capture.co_belligerents.required
            ),
            (3, 3)
        );
        let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
            capture.counter_table()
        else {
            panic!("prefix lost")
        };
        assert_eq!(rows[0].owned_state_count, 7);
        assert_eq!(rows[0].co_belligerents, Recorded::Value(Some(vec![0, 2])));
        assert_eq!(rows[0].zero_war, Recorded::Value(Some([0, 0])));
        assert_eq!(rows[1].co_belligerents, Recorded::Value(Some(vec![0])));
        let mut capacity = Capacities {
            rows: 2,
            co_belligerents: 1,
            ..Default::default()
        };
        capture.grow(&mut capacity);
        assert!(capacity.co_belligerents >= 3);
        capture.capture_relations(&memory, 0, CO_STATE);
        let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
            capture.counter_table()
        else {
            panic!("prefix lost")
        };
        assert_eq!(rows[0].co_belligerents, Recorded::Value(Some(vec![0, 2])));
        assert_eq!(rows[1].co_belligerents, Recorded::Value(Some(vec![0])));
        capture.co_belligerents.first.fill(99);
        assert_eq!(rows[0].co_belligerents, Recorded::Value(Some(vec![0, 2])));
        for (owner, target) in [(0, 0), (0, 2), (1, 0)] {
            bytes[co_status(owner, target) + crelation_status::CO_BELLIGERENT] = 0;
        }
        capture.capture_relations(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            CO_STATE,
        );
        assert_eq!(capture.co_belligerents.len, 0);
        let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
            capture.counter_table()
        else {
            panic!("prefix lost")
        };
        assert!(
            rows.iter()
                .all(|row| row.co_belligerents == Recorded::Value(Some(Vec::new())))
        );
        capture.reset();
        assert_eq!(
            (
                capture.relations.len,
                capture.co_belligerents.len,
                capture.co_belligerents.required
            ),
            (0, 0, 0)
        );
    }

    struct FailCoOnce<'a> {
        memory: SliceMemory<'a>,
        at: usize,
        attempts: std::cell::Cell<usize>,
    }
    impl Memory for FailCoOnce<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address <= self.at && self.at - address < output.len() {
                let previous = self.attempts.get();
                self.attempts.set(previous + 1);
                if previous == 0 {
                    return false;
                }
            }
            self.memory.copy(address, output)
        }
    }

    #[test]
    fn relation_suffix_guard_is_not_retried_and_keeps_cache_and_peer_country() {
        let bytes = co_world();
        let memory = FailCoOnce {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            at: co_status(0, 0) + crelation_status::WAR_RELATION,
            attempts: std::cell::Cell::new(0),
        };
        let mut capture = co_capture(3);
        capture.capture_relations(&memory, 0, CO_STATE);
        assert_eq!(memory.attempts.get(), 1);
        let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
            capture.counter_table()
        else {
            panic!("prefix lost")
        };
        assert_eq!(rows[0].co_belligerents, Recorded::Value(Some(vec![0, 2])));
        assert_eq!(rows[0].zero_war, Recorded::Value(None));
        assert_eq!(rows[1].zero_war, Recorded::Value(Some([0, 0])));
        assert_eq!(rows[1].owned_state_count, 8);
        assert_eq!(
            capture.relations.first[0].zero_reason,
            Some("slot-0 war pointer unreadable")
        );
    }

    #[test]
    fn zero_subject_fault_preserves_war_cache_and_peer_country() {
        let bytes = co_world();
        let memory = FailCoOnce {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            at: co_status(0, 0) + crelation_status::SUBJECT_RELATION,
            attempts: std::cell::Cell::new(0),
        };
        let mut capture = co_capture(3);
        capture.capture_relations(&memory, 0, CO_STATE);
        assert_eq!(memory.attempts.get(), 1);
        let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
            capture.counter_table()
        else {
            panic!("country prefix lost");
        };
        assert_eq!(rows[0].zero_subject, Recorded::Value(None));
        assert_eq!(rows[0].zero_war, Recorded::Value(Some([0, 0])));
        assert_eq!(rows[0].co_belligerents, Recorded::Value(Some(vec![0, 2])));
        assert_eq!(
            rows[1].zero_subject,
            Recorded::Value(Some(Default::default()))
        );
        assert_eq!(
            capture.relations.first[0].zero_subject_reason,
            Some("slot-0 subject pointer unreadable")
        );
    }

    #[test]
    fn zero_subject_known_zero_survives_first_fault_and_counts_only_attempted_refusal() {
        use engine_native::crelation::{self, subject as csubject_relation};

        let subject = 0x18000;
        for (first, fault_offset, expected, attempts, reason) in [
            (
                7,
                crelation::FIRST_COUNTRY,
                [None, Some(0)],
                1,
                Some("slot-0 subject first party unreadable"),
            ),
            (0, crelation::SECOND_COUNTRY, [Some(0), None], 0, None),
        ] {
            let mut bytes = co_world();
            co_put64(
                &mut bytes,
                co_status(0, 0) + crelation_status::SUBJECT_RELATION,
                subject as u64,
            );
            co_put64(&mut bytes, subject, csubject_relation::VTABLE_RVA.0 as u64);
            co_put32(
                &mut bytes,
                subject + crelation::TYPE_ID,
                csubject_relation::TYPE_VALUE,
            );
            co_put32(&mut bytes, subject + crelation::FIRST_COUNTRY, first);
            co_put32(&mut bytes, subject + crelation::SECOND_COUNTRY, 0);
            let memory = FailCoOnce {
                memory: SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                at: subject + fault_offset,
                attempts: std::cell::Cell::new(0),
            };
            let mut capture = co_capture(3);
            capture.capture_relations(&memory, 0, CO_STATE);
            assert_eq!(memory.attempts.get(), attempts);
            assert_eq!(capture.relations.first[0].zero_subject_reason, reason);
            let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
                capture.counter_table()
            else {
                panic!("country prefix lost after partial zero-subject read");
            };
            assert_eq!(
                rows[0].zero_subject,
                Recorded::Value(Some(replay_data::rows::ZeroSubjectInputs {
                    relation_type: Some(csubject_relation::TYPE_VALUE),
                    parties: Some(expected),
                }))
            );
            assert_eq!(rows[0].owned_state_count, 7);
            assert_eq!(rows[0].zero_war, Recorded::Value(Some([0, 0])));
            assert_eq!(rows[0].co_belligerents, Recorded::Value(Some(vec![0, 2])));
            assert_eq!(rows[1].owned_state_count, 8);
        }
    }

    #[test]
    fn relation_header_demand_keeps_known_floor_and_local_projection_refuses_bad_span() {
        let mut bytes = co_world();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let known = co_belligerent_demand(&memory, 0, CO_STATE);
        assert_eq!(known, (4, None));
        co_put64(&mut bytes, CO_DIPLOMACY[1], 0);
        assert_eq!(
            co_belligerent_demand(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                CO_STATE,
            ),
            (3, Some("diplomacy vtable mismatch"))
        );
        let mut capture = co_capture(3);
        capture.capture_relations(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            CO_STATE,
        );
        capture.relations.first[0].co_belligerents = Some(ccountry::RosterSpan {
            start: usize::MAX,
            count: 1,
        });
        let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
            capture.counter_table()
        else {
            panic!("prefix lost")
        };
        assert_eq!(rows[0].co_belligerents, Recorded::Value(None));
        assert_eq!(rows[0].zero_war, Recorded::Value(Some([0, 0])));
        assert_eq!(rows[1].owned_state_count, 8);
        assert_eq!(rows[1].zero_war, Recorded::Value(None));
    }

    #[test]
    fn co_belligerent_hint_names_root_country_and_each_owner_failure() {
        let (floor, refusal) = co_belligerent_demand(
            &SliceMemory {
                base: 0,
                bytes: &[],
            },
            0,
            CO_STATE,
        );
        assert_eq!(floor, 0);
        assert!(refusal.is_some());
        let mut bytes = co_world();
        co_put64(&mut bytes, CO_COUNTRIES[0], 0);
        let (floor, refusal) = co_belligerent_demand(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            CO_STATE,
        );
        assert_eq!(floor, 1);
        assert!(refusal.is_some());
        let mut bytes = co_world();
        for diplomacy in CO_DIPLOMACY {
            co_put64(&mut bytes, diplomacy, 0);
        }
        let (floor, refusal) = co_belligerent_demand(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            CO_STATE,
        );
        assert_eq!(floor, 0);
        assert_eq!(refusal, Some("diplomacy vtable mismatch"));
    }

    #[test]
    fn native_null_mio_roster_is_named_and_preserves_counter_values() {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: 1,
            ..Default::default()
        });
        captured.rows.rows[0] = ccountry::CounterInputsRow {
            country: 5_000,
            owned_state_count: 7,
            state_rosters: Some([Default::default(); 3]),
            leader_rosters: Some([Default::default(); 2]),
            sent_expeditionaries: Some(Default::default()),
            neighbors: Some(Default::default()),
            policy_reason: Some("MIO policy production status is null"),
            ..Default::default()
        };
        captured.rows.outcome = Outcome::Rows { len: 1 };
        captured.relations.len = 1;
        captured.relations.first[0] = RelationRow {
            co_belligerents: Some(Default::default()),
            zero_war: Some([0, 0]),
            zero_subject: Some(Default::default()),
            forward_visibility: Some(replay_data::rows::ReverseZeroVisibilityInputs {
                target: Some(0),
                war: Some([0, 0]),
                subject: Some(Default::default()),
                military_access: Some(0),
            }),
            ..Default::default()
        };
        let tables = captured.tables();
        let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
            &tables[0]
        else {
            panic!("existing country row lost");
        };
        assert_eq!(rows[0].owned_state_count, 7);
        assert_eq!(rows[0].mio_policies, Recorded::Value(None));
        assert_eq!(rows[0].neighbors, Recorded::Value(Some(Vec::new())));
    }
    #[test]
    fn unreadable_census_is_unavailable_and_country_operands_preserve_primary_values() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            civil_wars: 2,
            ..Default::default()
        });
        capture.capture(&memory, 0, 0);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.rows[0] = ccountry::CounterInputsRow {
            country: 7,
            pride_of_the_fleet: (32 << 32) | 51,
            owned_state_count: 3,
            controlled_state_count: 5,
            first_division: (9 << 32) | 4,
            ..Default::default()
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.civil_wars.rows[0] = ccountry::CivilWarRow {
            country: 7,
            ordinal: 0,
            target: 0x8000_0002,
        };
        capture.civil_wars.outcome = Outcome::Rows { len: 1 };
        let [
            TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)),
            TableView::Complete(replay_data::rows::TableRows::CountryCivilWars(wars)),
        ] = capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].pride_of_the_fleet, (32 << 32) | 51);
        assert_eq!(rows[0].country, 7);
        assert_eq!(rows[0].owned_state_count, 3);
        assert_eq!(rows[0].first_division, (9 << 32) | 4);
        assert_eq!(rows[0].controlled_state_count, 5);
        assert_eq!(
            wars,
            [CountryCivilWarRow {
                country: 7,
                ordinal: 0,
                target: 0x8000_0002,
            }]
        );
    }

    #[test]
    fn civil_war_capacity_follows_the_live_entry_total() {
        let mut capacity = Capacities::default();
        capacity.raise_for_civil_wars(3_000);
        assert!(capacity.civil_wars >= 4_000);
        let raised = capacity;
        capacity.raise_for_civil_wars(1);
        assert!(capacity == raised);
    }

    #[test]
    fn live_roster_demand_sizes_country_arenas_and_projection_owns_every_operand() {
        const COUNTRIES: usize = 5;
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(COUNTRIES + 1);
        capacity.raise_for_rosters(ccountry::RosterDemand {
            states: COUNTRIES * 9,
            leaders: COUNTRIES * 6,
        });
        assert!(capacity.states >= COUNTRIES * 12 && capacity.leaders >= COUNTRIES * 8);
        let mut captured = Captured::with_capacities(&capacity);
        for index in 0..COUNTRIES {
            let country = index as u32 + 1;
            let state_start = index * 9;
            for list in 0..3 {
                captured.states.first[state_start + list * 3..state_start + list * 3 + 3]
                    .copy_from_slice(&[country + 10_000, 0, country]);
            }
            let leader_start = index * 6;
            let handle = (u64::from(country) << 32) | 4713;
            for list in 0..2 {
                captured.leaders.first[leader_start + list * 3..leader_start + list * 3 + 3]
                    .copy_from_slice(&[handle, 0, handle]);
            }
            captured.rows.rows[index] = ccountry::CounterInputsRow {
                country,
                owned_state_count: 3,
                controlled_state_count: 3,
                state_rosters: Some(std::array::from_fn(|list| ccountry::RosterSpan {
                    start: state_start + list * 3,
                    count: 3,
                })),
                leader_rosters: Some(std::array::from_fn(|list| ccountry::RosterSpan {
                    start: leader_start + list * 3,
                    count: 3,
                })),
                ..Default::default()
            };
        }
        captured.rows.outcome = Outcome::Rows { len: COUNTRIES };
        captured.states.len = COUNTRIES * 9;
        captured.leaders.len = COUNTRIES * 6;
        let [
            TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)),
            _,
        ] = captured.tables()
        else {
            panic!("complete");
        };
        captured.states.first.fill(0);
        captured.leaders.first.fill(0);
        for (index, row) in rows.iter().enumerate() {
            let country = index as u32 + 1;
            let expected_states = [country + 10_000, 0, country];
            assert_eq!(
                row.owned_states
                    .require("CountryCounterInputs.owned_states")
                    .unwrap()
                    .as_deref(),
                Some(expected_states.as_slice())
            );
            assert_eq!(
                row.controlled_states
                    .require("CountryCounterInputs.controlled_states")
                    .unwrap()
                    .as_deref(),
                Some(expected_states.as_slice())
            );
            assert_eq!(
                row.core_states
                    .require("CountryCounterInputs.core_states")
                    .unwrap()
                    .as_deref(),
                Some(expected_states.as_slice())
            );
            let handle = (u64::from(country) << 32) | 4713;
            let expected_leaders = [handle, 0, handle];
            assert_eq!(
                row.army_leaders
                    .require("CountryCounterInputs.army_leaders")
                    .unwrap()
                    .as_deref(),
                Some(expected_leaders.as_slice())
            );
            assert_eq!(
                row.navy_leaders
                    .require("CountryCounterInputs.navy_leaders")
                    .unwrap()
                    .as_deref(),
                Some(expected_leaders.as_slice())
            );
        }
        let raised = capacity;
        capacity.raise_for_rosters(Default::default());
        assert!(capacity == raised);
    }

    #[test]
    fn observed_empty_rosters_are_present_and_unobserved_inputs_remain_absent() {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: 1,
            ..Default::default()
        });
        captured.rows.rows[0] = ccountry::CounterInputsRow {
            country: 1,
            ..Default::default()
        };
        captured.rows.outcome = Outcome::Rows { len: 1 };
        let [
            TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)),
            _,
        ] = captured.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].owned_states, Recorded::Value(None));
        assert_eq!(rows[0].army_leaders, Recorded::Value(None));
        captured.rows.rows[0].state_rosters = Some([ccountry::RosterSpan::default(); 3]);
        captured.rows.rows[0].leader_rosters = Some([ccountry::RosterSpan::default(); 2]);
        let views = captured.tables();
        let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
            &views[0]
        else {
            panic!("observed empty rosters are present");
        };
        assert_eq!(rows[0].owned_states, Recorded::Value(Some(Vec::new())));
        assert_eq!(rows[0].controlled_states, Recorded::Value(Some(Vec::new())));
        assert_eq!(rows[0].core_states, Recorded::Value(Some(Vec::new())));
        assert_eq!(rows[0].army_leaders, Recorded::Value(Some(Vec::new())));
        assert_eq!(rows[0].navy_leaders, Recorded::Value(Some(Vec::new())));
        captured.reset();
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn roster_shortfalls_keep_scalar_rows_and_count_each_native_reason() {
        const COUNTRIES: usize = 6;
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(COUNTRIES + 1);
        let mut captured = Captured::with_capacities(&capacity);
        for (index, row) in captured.rows.rows[..COUNTRIES].iter_mut().enumerate() {
            let country = index as u32 + 1;
            *row = ccountry::CounterInputsRow {
                country,
                pride_of_the_fleet: (u64::from(country) << 32) | 51,
                owned_state_count: 7,
                controlled_state_count: 11,
                first_division: (u64::from(country) << 32) | 4,
                roster_reason: Some(if index % 2 == 0 {
                    "country state roster arena capacity exceeded"
                } else {
                    "unit leader vtable mismatch"
                }),
                ..Default::default()
            };
        }
        captured.rows.outcome = Outcome::Rows { len: COUNTRIES };
        captured.states.required = COUNTRIES * 9;
        captured.leaders.required = COUNTRIES * 6;
        captured.relations.len = COUNTRIES;
        for row in &mut captured.relations.first[..COUNTRIES] {
            *row = RelationRow {
                co_belligerents: Some(Default::default()),
                zero_war: Some([0, 0]),
                zero_subject: Some(Default::default()),
                forward_visibility: Some(replay_data::rows::ReverseZeroVisibilityInputs {
                    target: Some(0),
                    war: Some([0, 0]),
                    subject: Some(Default::default()),
                    military_access: Some(0),
                }),
                ..Default::default()
            };
        }
        let [
            TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)),
            _,
        ] = captured.tables()
        else {
            panic!("scalar rows unavailable");
        };
        assert_eq!(rows.len(), COUNTRIES);
        for row in rows {
            assert_eq!(row.pride_of_the_fleet, (u64::from(row.country) << 32) | 51);
            assert_eq!(row.first_division, (u64::from(row.country) << 32) | 4);
            assert_eq!((row.owned_state_count, row.controlled_state_count), (7, 11));
            assert_eq!(row.owned_states, Recorded::Value(None));
            assert_eq!(row.controlled_states, Recorded::Value(None));
            assert_eq!(row.core_states, Recorded::Value(None));
            assert_eq!(row.army_leaders, Recorded::Value(None));
            assert_eq!(row.navy_leaders, Recorded::Value(None));
        }
    }

    #[test]
    fn invalid_arena_spans_are_named_unavailable_before_any_slice() {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: 1,
            states: 4,
            leaders: 4,
            ..Default::default()
        });
        captured.rows.outcome = Outcome::Rows { len: 1 };
        captured.states.len = 2;
        captured.leaders.len = 2;
        for (start, count) in [(usize::MAX, 1), (1, 2), (4, 1)] {
            let span = ccountry::RosterSpan { start, count };
            captured.rows.rows[0].state_rosters = Some([span; 3]);
            assert!(matches!(
                &captured.tables()[0],
                TableView::Unavailable { reason }
                    if reason == "country state roster span outside acquired arena"
            ));
            captured.rows.rows[0].state_rosters = None;
            captured.rows.rows[0].leader_rosters = Some([span; 2]);
            assert!(matches!(
                &captured.tables()[0],
                TableView::Unavailable { reason }
                    if reason == "country leader roster span outside acquired arena"
            ));
            captured.rows.rows[0].leader_rosters = None;
        }
    }

    #[test]
    fn forward_visibility_alias_zero_projects_independently_of_old_suffix_refusal() {
        let mut bytes = co_world();
        co_header(
            &mut bytes,
            CO_STATE + engine_native::cgame_state::COUNTRY_LINKS,
            0x3100,
            5,
        );
        co_put32(&mut bytes, 0x3110, 0);
        co_put32(
            &mut bytes,
            co_status(0, 0) + crelation_status::TARGET_COUNTRY,
            4,
        );
        co_put64(
            &mut bytes,
            co_status(0, 0) + crelation_status::MILITARY_ACCESS,
            1_u64 << 40,
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut capture = co_capture(3);
        capture.capture_relations(&memory, 0, CO_STATE);
        let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
            capture.counter_table()
        else {
            panic!("prefix lost");
        };
        assert_eq!(rows[0].owned_state_count, 7);
        assert_eq!(rows[0].zero_war, Recorded::Value(None));
        assert_eq!(rows[0].zero_subject, Recorded::Value(None));
        assert_eq!(rows[0].co_belligerents, Recorded::Value(None));
        assert_eq!(
            rows[0].forward_visibility,
            Recorded::Value(Some(replay_data::rows::ReverseZeroVisibilityInputs {
                target: Some(4),
                war: Some([0, 0]),
                subject: Some(Default::default()),
                military_access: Some(1),
            }))
        );
        assert_eq!(rows[1].owned_state_count, 8);
        assert_eq!(rows[1].zero_war, Recorded::Value(Some([0, 0])));
        assert_eq!(
            rows[1].forward_visibility,
            Recorded::Value(Some(replay_data::rows::ReverseZeroVisibilityInputs {
                target: Some(0),
                war: Some([0, 0]),
                subject: Some(Default::default()),
                military_access: Some(0),
            }))
        );
    }

    #[test]
    fn forward_visibility_military_fault_keeps_prefix_peer_and_names_only_new_fault() {
        let bytes = co_world();
        let memory = FailCoOnce {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            at: co_status(0, 0) + crelation_status::MILITARY_ACCESS,
            attempts: std::cell::Cell::new(0),
        };
        let mut capture = co_capture(3);
        capture.capture_relations(&memory, 0, CO_STATE);
        assert_eq!(memory.attempts.get(), 1);
        let TableView::Complete(replay_data::rows::TableRows::CountryCounterInputs(rows)) =
            capture.counter_table()
        else {
            panic!("prefix lost");
        };
        let Recorded::Value(Some(value)) = rows[0].forward_visibility else {
            panic!("new packet lost");
        };
        assert_eq!(value.military_access, None);
        assert_eq!(value.war, Some([0, 0]));
        assert_eq!(value.subject, Some(Default::default()));
        assert_eq!(rows[0].zero_war, Recorded::Value(Some([0, 0])));
        assert_eq!(rows[1].owned_state_count, 8);
        assert_eq!(
            capture.relations.first[0]
                .forward_visibility_failures
                .map(|failure| failure.is_some()),
            [false, false, false, true]
        );
        capture.reset();
        capture.rows.outcome = Outcome::Rows { len: 2 };
        capture.capture_relations(&memory, 0, CO_STATE);
        assert_eq!(
            capture.relations.first[0].forward_visibility_failures,
            [None; 4]
        );
        assert_eq!(
            capture.relations.first[0]
                .forward_visibility
                .unwrap()
                .military_access,
            Some(0)
        );
    }

    #[test]
    fn forward_visibility_reused_failure_has_role_counter_without_duplicate_old_reason() {
        let bytes = co_world();
        let memory = FailCoOnce {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            at: co_status(0, 0) + crelation_status::WAR_RELATION,
            attempts: std::cell::Cell::new(0),
        };
        let mut capture = co_capture(3);
        capture.capture_relations(&memory, 0, CO_STATE);
        assert_eq!(memory.attempts.get(), 1);
        assert_eq!(
            capture.relations.first[0]
                .forward_visibility_failures
                .map(|failure| failure.is_some()),
            [false, true, false, false]
        );
        assert_eq!(capture.relations.first[0].zero_war, None);
        assert_eq!(
            capture.relations.first[0].forward_visibility.unwrap().war,
            None
        );
    }
}

#[cfg(test)]
mod manpower_projection_tests {
    use super::*;
    #[test]
    fn manpower_projection_reuses_quiet_reference_storage_and_preserves_old_rows() {
        let mut capture = Captured::vacant();
        capture.leaders.first = vec![7, 0, 7];
        capture.leaders.len = 3;
        let span = ccountry::RosterSpan { start: 0, count: 3 };
        let first = capture.manpower_reference_list(1, span).unwrap();
        for _ in 0..100 {
            let quiet = capture.manpower_reference_list(1, span).unwrap();
            assert_eq!(quiet.as_ptr(), first.as_ptr());
        }
        capture.leaders.first[0] = 9;
        let changed = capture.manpower_reference_list(1, span).unwrap();
        assert_eq!(&*first, &[7, 0, 7]);
        assert_eq!(&*changed, &[9, 0, 7]);
        assert_ne!(changed.as_ptr(), first.as_ptr());
        assert!(
            capture
                .manpower_reference_list(1, ccountry::RosterSpan { start: 3, count: 1 })
                .is_none()
        );
    }
}
