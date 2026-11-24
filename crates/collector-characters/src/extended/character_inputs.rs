use collector_primitives::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
};
use collector_projection::table_conversion::{table, unavailable};
use engine_allocation::{copy_vec, map_slice, owned_vec, vec};
use engine_characters::{character_inputs as native, character_inputs::Role};
use engine_native_memory::{Memory, walk::Outcome};
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows::CharacterOperativeRow,
    rows::CharacterRoleTraitRow, rows::Recorded, rows::TableId, rows::tables,
};
use replay_frame::frame::{TableInput, TableView};

mod role_store;
pub use role_store::RoleStore;

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::CharacterRoleTraits, TableId::CharacterOperatives];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    traits: usize,
    values: usize,
    operatives: usize,
    definitions: usize,
    bytes: usize,
}

collector_primitives::capacity_max!(Capacities {
    traits,
    values,
    operatives,
    definitions,
    bytes
});
impl Capacities {
    pub fn raise_for_demand(&mut self, demand: Demand) {
        use collector_primitives::country_slots::{raise, rows};
        raise(&mut self.traits, rows(demand.traits, 1));
        raise(&mut self.values, rows(demand.values, 1));
        raise(&mut self.operatives, rows(demand.operatives, 1));
        raise(&mut self.definitions, rows(demand.definitions, 1));
        raise(&mut self.bytes, rows(demand.bytes, 1));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Demand {
    pub traits: usize,
    pub values: usize,
    pub operatives: usize,
    pub definitions: usize,
    pub bytes: usize,
}

impl Demand {
    pub fn observe(
        memory: &impl Memory,
        base: usize,
        game_state: usize,
    ) -> Result<Self, &'static str> {
        let walk = native::count(memory, base, game_state);
        let rows = |outcome: Outcome| match outcome {
            Outcome::Rows { len } => Ok(len),
            Outcome::Truncated { len, unread_rows } => len
                .checked_add(unread_rows as usize)
                .ok_or("character input demand row count overflow"),
            Outcome::Unavailable { reason } => Err(reason),
        };
        Ok(Self {
            traits: rows(walk.traits)?,
            values: walk.required_trait_values,
            operatives: rows(walk.operatives)?,
            definitions: walk.required_definitions,
            bytes: walk.required_bytes,
        })
    }

    pub fn exceeds(&self, seen: &Self) -> bool {
        self.traits > seen.traits
            || self.values > seen.values
            || self.operatives > seen.operatives
            || self.definitions > seen.definitions
            || self.bytes > seen.bytes
    }

    pub fn max(self, other: Self) -> Self {
        Self {
            traits: self.traits.max(other.traits),
            values: self.values.max(other.values),
            operatives: self.operatives.max(other.operatives),
            definitions: self.definitions.max(other.definitions),
            bytes: self.bytes.max(other.bytes),
        }
    }
}

pub struct Captured {
    traits: CaptureRows<native::TraitRow>,
    values: Arena<native::Trait>,
    operatives: CaptureRows<native::OperativeRow>,
    definitions: Arena<u32>,
    bytes: Arena<u8>,
    held: Option<(u64, HeldTraits)>,
}

#[derive(Clone, Copy, Debug)]
struct HeldTraits {
    outcome: Outcome,
    values_len: usize,
    values_required: usize,
}

impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            traits: 0,
            values: 0,
            operatives: 0,
            definitions: 0,
            bytes: 0,
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            traits: CaptureRows::new(capacity.traits),
            values: Arena::new(capacity.values),
            operatives: CaptureRows::new(capacity.operatives),
            definitions: Arena::new(capacity.definitions),
            bytes: Arena::new(capacity.bytes),
            held: None,
        }
    }

    fn held_traits(&self) -> HeldTraits {
        HeldTraits {
            outcome: self.traits.outcome,
            values_len: self.values.len,
            values_required: self.values.required,
        }
    }

    fn restore_traits(&mut self, held: HeldTraits) {
        self.traits.outcome = held.outcome;
        self.values.len = held.values_len;
        self.values.required = held.values_required;
    }
    pub fn reset(&mut self) {
        self.traits.reset();
        self.values.reset();
        self.operatives.reset();
        self.definitions.reset();
        self.bytes.reset();
    }
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.held = None;
        self.traits
            .fail(collector_extended_support::outcomes::failed_capture(
                TABLES[0], hour, force_full, reason,
            ));
        self.operatives
            .fail(collector_extended_support::outcomes::failed_capture(
                TABLES[1], hour, force_full, reason,
            ));
    }
    pub fn allocated_bytes(&self) -> usize {
        self.traits.allocated_bytes()
            + self.values.allocated_bytes()
            + self.operatives.allocated_bytes()
            + self.definitions.allocated_bytes()
            + self.bytes.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.traits.grow(&mut capacity.traits);
        self.values.grow(&mut capacity.values);
        self.operatives.grow(&mut capacity.operatives);
        self.definitions.grow(&mut capacity.definitions);
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
        self.held = None;
        self.reset();
        let Some(walked) = self.walked(hour, force_full) else {
            return [0; N];
        };
        let _reads = collector_memory::read_accounting::table(TABLES[0]);
        let started = qpc();
        let walk = native::walk(
            memory,
            base,
            game_state,
            native::Buffers {
                traits: &mut self.traits.rows,
                trait_values: &mut self.values.first,
                operatives: &mut self.operatives.rows,
                definitions: &mut self.definitions.first,
                bytes: &mut self.bytes.first,
            },
        );
        self.install(walked, walk);
        [qpc().saturating_sub(started).max(0) as u64, 0]
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the hourly capture operands (memory, base, game state, hour, full flag) travel beside the census admission and role store"
    )]
    pub fn capture_census(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        census: Option<super::characters::census::Admitted<'_>>,
        store: Option<&mut RoleStore>,
    ) -> [u64; N] {
        self.capture_census_with(
            memory,
            base,
            game_state,
            hour,
            force_full,
            census,
            store,
            &collector_observers::character_changes::Observer,
            &role_store::LeaderObserver,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the hourly capture operands (memory, base, game state, hour, full flag) travel beside the census, role store and both change sources"
    )]
    pub fn capture_census_with(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        census: Option<super::characters::census::Admitted<'_>>,
        store: Option<&mut RoleStore>,
        changes: &dyn collector_observers::character_changes::Changes,
        leaders: &dyn role_store::LeaderChanges,
    ) -> [u64; N] {
        let Some(census) = census else {
            if let Some(store) = store {
                store.forget();
            }
            return self.capture(memory, base, game_state, hour, force_full);
        };
        self.reset();
        let Some(walked) = self.walked(hour, force_full) else {
            return [0; N];
        };
        let _reads = collector_memory::read_accounting::table(TABLES[0]);
        let started = qpc();
        let mut walk = native::CensusWalk::new(native::Buffers {
            traits: &mut self.traits.rows,
            trait_values: &mut self.values.first,
            operatives: &mut self.operatives.rows,
            definitions: &mut self.definitions.first,
            bytes: &mut self.bytes.first,
        });
        let held = self.held.map(|(content, _)| content);
        let mut store = store;
        if let Some(store) = store.as_deref_mut() {
            store.begin_operatives();
        }
        let visited = role_store::visit(
            memory,
            base,
            game_state,
            census,
            &mut walk,
            store.as_deref_mut().map(|store| role_store::RoleReuse {
                store,
                changes,
                held,
                leaders,
            }),
        );
        let walk = walk.finish();
        if let Some(store) = store.as_deref_mut() {
            store.finish_operatives(matches!(walk.operatives, Outcome::Rows { .. }));
        }
        let traits = self.held.map(|(_, traits)| traits);
        self.install(walked, walk);
        match (visited, traits) {
            (role_store::Visited::Held, Some(traits)) if walked[0] => self.restore_traits(traits),
            _ => {
                self.held = store
                    .map(|store| store.content())
                    .filter(|&content| content != 0 && walked[0])
                    .filter(|_| matches!(self.traits.outcome, Outcome::Rows { .. }))
                    .map(|content| (content, self.held_traits()));
            }
        }
        [qpc().saturating_sub(started).max(0) as u64, 0]
    }

    fn walked(&mut self, hour: i32, force_full: bool) -> Option<[bool; N]> {
        let walked = TABLES.map(|table| should_walk(table, hour, force_full));
        if !walked.iter().any(|&walked| walked) {
            self.traits.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
            self.operatives.outcome = self.traits.outcome;
            return None;
        }
        Some(walked)
    }

    fn install(&mut self, walked: [bool; N], walk: native::Walk) {
        let skipped = Outcome::Unavailable {
            reason: SKIPPED_REASON,
        };
        self.traits.outcome = if walked[0] { walk.traits } else { skipped };
        self.operatives.outcome = if walked[1] { walk.operatives } else { skipped };
        self.values.required = walk.required_trait_values;
        self.definitions.required = walk.required_definitions;
        self.bytes.len = walk.bytes_len;
        self.bytes.required = walk.required_bytes;
        self.values.len = spans_end(&self.traits.rows[..walk.traits.len()], |row| {
            (row.start, row.len)
        });
        self.definitions.len = spans_end(&self.operatives.rows[..walk.operatives.len()], |row| {
            (row.start, row.len)
        });
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.traits.outcome, self.operatives.outcome]
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("no projection supplied"),
        })
    }
    pub fn table_inputs(
        &self,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> [TableInput; N] {
        [
            self.traits_input(projection),
            self.operatives_table().into(),
        ]
    }
    fn traits_input(
        &self,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> TableInput {
        let values = &self.values.first[..self.values.len.min(self.values.first.len())];
        let Some(rows) = self.traits.rows.get(..self.traits.outcome.len()) else {
            return unavailable("character role trait rows exceed buffer").into();
        };
        if rows
            .iter()
            .any(|row| span(values, row.start, row.len).is_none())
        {
            return unavailable("character role traits outside captured arena").into();
        }
        let role_key = |row: &native::TraitRow| match row.role {
            Role::CountryLeader(ideology) => {
                (CharacterRoleTraitRow::COUNTRY_LEADER, ideology as u32)
            }
            Role::Advisor(ordinal) => (CharacterRoleTraitRow::ADVISOR, ordinal),
            Role::Scientist => (CharacterRoleTraitRow::SCIENTIST, 0),
        };
        let witness = self
            .held
            .filter(|_| matches!(self.traits.outcome, Outcome::Rows { .. }))
            .map(|(content, _)| collector_projection::projection::Witness::generation(content));
        collector_projection::projection::held_input::<tables::CharacterRoleTraits>(
            projection,
            witness,
            |projection| {
                collector_projection::projection::ledger_table::<tables::CharacterRoleTraits, _, _>(
                    projection,
                    self.traits.outcome,
                    rows,
                    |row| {
                        let (kind, role) = role_key(row);
                        (u128::from(row.character) << 40)
                            | (u128::from(kind) << 32)
                            | u128::from(role)
                    },
                    |_, row, record| {
                        let traits = span(values, row.start, row.len).unwrap();
                        record.word(traits.len() as i64);
                        for value in traits {
                            record.word(i64::from(value.token));
                            record.word(value.ordinal.map_or(-1, i64::from));
                        }
                        Some(())
                    },
                    |row| {
                        let (kind, role) = role_key(row);
                        let traits = span(values, row.start, row.len).unwrap();
                        CharacterRoleTraitRow {
                            character: row.character,
                            kind,
                            role,
                            tokens: owned_vec(map_slice(traits, |value| value.token)),
                            definitions: owned_vec((|| {
                                let mut definitions = vec(traits.len())?;
                                definitions.extend(traits.iter().filter_map(|value| value.ordinal));
                                Ok(definitions)
                            })()),
                        }
                    },
                )
            },
        )
    }
    fn operatives_table(&self) -> TableView {
        let values =
            &self.definitions.first[..self.definitions.len.min(self.definitions.first.len())];
        let Some(rows) = self.operatives.rows.get(..self.operatives.outcome.len()) else {
            return unavailable("character operative rows exceed buffer");
        };
        if rows
            .iter()
            .any(|row| span(values, row.start, row.len).is_none())
        {
            return unavailable("character operative traits outside captured arena");
        }
        let bytes = &self.bytes.first[..self.bytes.len.min(self.bytes.first.len())];
        if rows.iter().any(|row| {
            row.name
                .is_some_and(|(start, len)| span(bytes, start, len).is_none())
                || row.codename.is_some_and(|name| {
                    span(bytes, name.custom.0, name.custom.1).is_none()
                        || span(bytes, name.group.0, name.group.1).is_none()
                })
        }) {
            return unavailable("character operative names outside captured arena");
        }
        table::<tables::CharacterOperatives, _>(self.operatives.outcome, rows, |row| {
            let leader = row.leader.unwrap_or_default();
            CharacterOperativeRow {
                character: row.character,
                reference: row.reference,
                resolved: row.leader.is_some(),
                category: leader.category,
                owner: leader.owner,
                skill_level: leader.skill_level,
                trait_definitions: owned_vec(copy_vec(span(values, row.start, row.len).unwrap())),
                codename: Recorded::Value(row.codename.map(|name| operands(name, bytes))),
                name: Recorded::Value(
                    row.name
                        .map(|(start, len)| owned_vec(copy_vec(span(bytes, start, len).unwrap()))),
                ),
            }
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn allocation_failure_exercise() {
    let _home = collector_intel::daily_intel::Home::try_new(2, 16);
    if collector_memory::terminal::failed() {
        return;
    }
    let _roles = super::characters::census::RoleCensus::new(2);
    if collector_memory::terminal::failed() {
        return;
    }
    let mut captured = Captured::with_capacities(&Capacities {
        traits: 1,
        values: 2,
        operatives: 1,
        definitions: 2,
        bytes: 32,
    });
    if collector_memory::terminal::failed() {
        return;
    }
    captured.values.first[..2].copy_from_slice(&[
        native::Trait {
            token: 11,
            ordinal: Some(21),
        },
        native::Trait {
            token: 12,
            ordinal: None,
        },
    ]);
    captured.values.len = 2;
    captured.traits.rows[0] = native::TraitRow {
        character: 1,
        role: Role::CountryLeader(0),
        start: 0,
        len: 2,
    };
    captured.traits.outcome = Outcome::Rows { len: 1 };
    captured.definitions.first[..2].copy_from_slice(&[31, 32]);
    captured.definitions.len = 2;
    captured.operatives.rows[0] = native::OperativeRow {
        character: 1,
        reference: 1,
        leader: None,
        start: 0,
        len: 2,
        ..Default::default()
    };
    captured.operatives.outcome = Outcome::Rows { len: 1 };
    let _ = captured.tables();
}

pub(super) fn operands(
    name: engine_native::cname_group_member::Inputs,
    bytes: &[u8],
) -> replay_data::rows::NameOperands {
    replay_data::rows::NameOperands {
        kind: name.kind,
        order: name.order,
        ordered: name.ordered,
        programmatic: name.programmatic,
        custom: owned_vec(copy_vec(span(bytes, name.custom.0, name.custom.1).unwrap())),
        group: owned_vec(copy_vec(span(bytes, name.group.0, name.group.1).unwrap())),
    }
}

fn span<V>(values: &[V], start: usize, len: usize) -> Option<&[V]> {
    values.get(start..start.checked_add(len)?)
}

fn spans_end<R>(rows: &[R], span: impl Fn(&R) -> (usize, usize)) -> usize {
    rows.iter()
        .map(|row| {
            let (start, len) = span(row);
            start.saturating_add(len)
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "character_inputs_scale_tests.rs"]
mod scale_tests;

#[cfg(test)]
#[path = "character_inputs/role_store_tests.rs"]
mod role_store_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use engine_characters::character_inputs::Leader;
    use replay_data::rows::TableRows;

    #[test]
    fn programmatic_override_survives_projection_when_native_shows_it() {
        let naming = operands(
            engine_native::cname_group_member::Inputs {
                kind: 2,
                programmatic: 1,
                custom: (0, 6),
                group: (6, 5),
                ..Default::default()
            },
            b"legacyGROUP",
        );
        assert_eq!(naming.custom, b"legacy");
        assert_eq!(naming.group, b"GROUP");
    }

    fn captured() -> Captured {
        let mut captured = Captured::with_capacities(&Capacities {
            traits: 3,
            values: 4,
            operatives: 2,
            definitions: 2,
            bytes: 32,
        });
        captured.values.first[..3].copy_from_slice(&[
            native::Trait {
                token: 101,
                ordinal: Some(1),
            },
            native::Trait {
                token: 102,
                ordinal: Some(2),
            },
            native::Trait {
                token: 201,
                ordinal: None,
            },
        ]);
        captured.values.len = 3;
        captured.traits.rows[0] = native::TraitRow {
            character: 11,
            role: Role::CountryLeader(-2),
            start: 0,
            len: 2,
        };
        captured.traits.rows[1] = native::TraitRow {
            character: 11,
            role: Role::Scientist,
            start: 2,
            len: 1,
        };
        captured.traits.outcome = Outcome::Rows { len: 2 };
        captured.definitions.first[0] = 3;
        captured.definitions.len = 1;
        captured.operatives.rows[0] = native::OperativeRow {
            character: 13,
            reference: 7,
            leader: None,
            start: 0,
            len: 0,
            ..Default::default()
        };
        captured.operatives.rows[1] = native::OperativeRow {
            character: 12,
            reference: 5,
            leader: Some(Leader {
                category: 3,
                owner: 9,
                skill_level: None,
            }),
            start: 0,
            len: 1,
            name: Some((0, 5)),
            codename: Some(engine_native::cname_group_member::Inputs {
                kind: 2,
                order: -1,
                ordered: 1,
                group: (5, 5),
                custom: (10, 4),
                ..Default::default()
            }),
            ..Default::default()
        };
        captured.bytes.first[..14].copy_from_slice(b"AliasGROUPUser");
        captured.bytes.len = 14;
        captured.operatives.outcome = Outcome::Rows { len: 2 };
        captured
    }

    #[test]
    fn rows_keep_their_role_keys_values_and_resolution() {
        let [
            TableView::Complete(TableRows::CharacterRoleTraits(traits)),
            TableView::Complete(TableRows::CharacterOperatives(operatives)),
        ] = captured().tables()
        else {
            panic!("complete character inputs");
        };
        assert_eq!(
            traits,
            [
                CharacterRoleTraitRow {
                    character: 11,
                    kind: CharacterRoleTraitRow::COUNTRY_LEADER,
                    role: -2_i32 as u32,
                    tokens: vec![101, 102],
                    definitions: vec![1, 2],
                },
                CharacterRoleTraitRow {
                    character: 11,
                    kind: CharacterRoleTraitRow::SCIENTIST,
                    role: 0,
                    tokens: vec![201],
                    definitions: Vec::new(),
                },
            ]
        );
        assert_eq!(
            operatives,
            [
                CharacterOperativeRow {
                    character: 12,
                    reference: 5,
                    resolved: true,
                    category: 3,
                    owner: 9,
                    skill_level: None,
                    trait_definitions: vec![3],
                    codename: Recorded::Value(Some(replay_data::rows::NameOperands {
                        kind: 2,
                        order: -1,
                        ordered: 1,
                        group: b"GROUP".to_vec(),
                        custom: b"User".to_vec(),
                        ..Default::default()
                    })),
                    name: Recorded::Value(Some(b"Alias".to_vec())),
                },
                CharacterOperativeRow {
                    character: 13,
                    reference: 7,
                    ..Default::default()
                },
            ]
        );
    }

    #[test]
    fn escaped_spans_and_failures_publish_nothing() {
        let mut captured = captured();
        captured.traits.rows[1].len = 2;
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
        let mut capacity = Capacities::default();
        captured.values.required = 1 << 20;
        captured.fail_capture(5, false, "job failed");
        assert!(
            captured
                .tables()
                .iter()
                .all(|table| matches!(table, TableView::Unavailable { .. }))
        );
        captured.grow(&mut capacity);
        assert!(capacity.values >= 1 << 20);
    }

    #[test]
    fn traits_projection_reuses_all_fields_and_preserves_frame_results() {
        use collector_projection::projection::{Projection, ProjectionScratch};
        use replay_frame::frame::{FrameEncoder, FrameInput, Metrics};
        let mut captured = captured();
        let mut encoder = FrameEncoder::new();
        let mut complete = FrameEncoder::new();
        let scratch = ProjectionScratch::default();
        for hour in 0..7 {
            match hour {
                2 => captured.values.first[0].ordinal = Some(9),
                3 => captured.values.first[0].token = 303,
                4 => captured.traits.rows.swap(0, 1),
                5 => {
                    captured.traits.outcome = Outcome::Truncated {
                        len: 1,
                        unread_rows: 1,
                    }
                }
                6 => captured.traits.outcome = Outcome::Rows { len: 2 },
                _ => {}
            }
            let force = hour == 4;
            let frame = |values: [TableInput; N]| {
                let mut tables = std::array::from_fn(|_| unavailable("fixture table").into());
                for (table, input) in TABLES.into_iter().zip(values) {
                    tables[table.index()] = input;
                }
                FrameInput {
                    branch: 1,
                    hour,
                    tables,
                    metrics: Metrics::default(),
                }
            };
            let expected = complete
                .encode_input(frame(captured.tables().map(Into::into)), force)
                .unwrap();
            let mut projection = Projection::new(&encoder, 1, hour, force, &scratch);
            let inputs = if hour == 1 {
                let traits = captured.traits_input(Some(&mut projection));
                assert!(matches!(traits, TableInput::ComparedEqual(_)));
                [traits, captured.operatives_table().into()]
            } else {
                captured.table_inputs(Some(&mut projection))
            };
            assert_eq!(
                encoder.encode_input(frame(inputs), force).unwrap(),
                expected,
                "hour {hour}"
            );
        }
    }

    #[test]
    fn held_traits_republish_on_quiet_hours_and_stay_sparse_on_changes() {
        use collector_projection::projection::{Projection, ProjectionScratch};
        use replay_frame::frame::{BaselineRetention, FrameEncoder, FrameInput, Metrics};
        let mut captured = captured();
        captured.held = Some((10, captured.held_traits()));
        let mut encoder = FrameEncoder::new();
        let scratch = ProjectionScratch::default();
        let commit = |encoder: &mut FrameEncoder, hour: i32, input: TableInput| {
            let mut tables = std::array::from_fn(|_| unavailable("fixture table").into());
            tables[TableId::CharacterRoleTraits.index()] = input;
            let frame = FrameInput {
                branch: 1,
                hour,
                tables,
                metrics: Metrics::default(),
            };
            let prepared = encoder
                .prepare_contained_keeping(
                    frame,
                    false,
                    &mut std::array::from_fn(|_| None),
                    |_| false,
                    |_, error| Err(error),
                )
                .unwrap();
            encoder
                .commit_retiring_mode(prepared, |table| {
                    if table == TableId::CharacterRoleTraits {
                        BaselineRetention::Meta
                    } else {
                        BaselineRetention::Drop
                    }
                })
                .unwrap();
        };
        let mut p0 = Projection::new(&encoder, 1, 0, false, &scratch);
        let first = captured.traits_input(Some(&mut p0));
        assert!(matches!(first, TableInput::SharedComplete(_)));
        commit(&mut encoder, 0, first);

        let mut p1 = Projection::new(&encoder, 1, 1, false, &scratch);
        let quiet = captured.traits_input(Some(&mut p1));
        assert!(matches!(quiet, TableInput::ComparedEqual(_)));
        assert_eq!(scratch.republish_counts(TableId::CharacterRoleTraits)[0], 1);
        commit(&mut encoder, 1, quiet);

        captured.values.first[0].token = 999;
        captured.held = Some((11, captured.held_traits()));
        let mut p2 = Projection::new(&encoder, 1, 2, false, &scratch);
        let updated = captured.traits_input(Some(&mut p2));
        let TableInput::SliceUpdate { update, .. } = &updated else {
            panic!("traits_input must emit a sparse SliceUpdate after a quiet held hour");
        };
        assert_eq!(update.rows.len(), 1);
        commit(&mut encoder, 2, updated);
    }
}
