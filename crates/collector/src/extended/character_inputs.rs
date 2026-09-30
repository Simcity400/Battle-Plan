//! The character-scope trigger inputs the character role jobs do not read,
//! from one hourly pass over the character roster: every country-leader,
//! advisor and scientist role's traits (CharacterRoleTraits) and each
//! nonzero unit-leader reference with its object's operands
//! (CharacterOperatives). Their writers (effects, commands and operative
//! recruitment) run at any hour; see [`engine::character_inputs`].
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory,
    character_inputs::{self as native, Role},
    walk::Outcome,
};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{CharacterOperativeRow, CharacterRoleTraitRow, TableId, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::CharacterRoleTraits, TableId::CharacterOperatives];
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("character role traits and operative links", 0, &[0, 1])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    traits: usize,
    values: usize,
    operatives: usize,
    definitions: usize,
}
impl Default for Capacities {
    /// Starting sizes only; the live demand (`country_scale::observe_politics`)
    /// and each hour's walk raise them.
    fn default() -> Self {
        Self {
            traits: 8192,
            values: 16_384,
            operatives: 512,
            definitions: 2048,
        }
    }
}

impl Capacities {
    /// From the live walk with no storage, with the three-quarters growth
    /// rule's headroom. Never lowers.
    pub(crate) fn raise_for_demand(&mut self, demand: Demand) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.traits, rows(demand.traits, 1));
        raise(&mut self.values, rows(demand.values, 1));
        raise(&mut self.operatives, rows(demand.operatives, 1));
        raise(&mut self.definitions, rows(demand.definitions, 1));
    }
}

/// Every row and value of the live roster.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Demand {
    pub traits: usize,
    /// Trait values (token and definition) of the role trait rows.
    pub values: usize,
    pub operatives: usize,
    pub definitions: usize,
}

impl Demand {
    /// The live demand, from the walk with no storage.
    pub(crate) fn observe(memory: &impl Memory, base: usize, game_state: usize) -> Self {
        let walk = native::walk(
            memory,
            base,
            game_state,
            native::Buffers {
                traits: &mut [],
                trait_values: &mut [],
                operatives: &mut [],
                definitions: &mut [],
            },
        );
        let rows = |outcome: Outcome| match outcome {
            Outcome::Rows { len } => len,
            Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
            Outcome::Unavailable { .. } => 0,
        };
        Self {
            traits: rows(walk.traits),
            values: walk.required_trait_values,
            operatives: rows(walk.operatives),
            definitions: walk.required_definitions,
        }
    }

    pub(crate) fn exceeds(&self, seen: &Self) -> bool {
        self.traits > seen.traits
            || self.values > seen.values
            || self.operatives > seen.operatives
            || self.definitions > seen.definitions
    }

    pub(crate) fn max(self, other: Self) -> Self {
        Self {
            traits: self.traits.max(other.traits),
            values: self.values.max(other.values),
            operatives: self.operatives.max(other.operatives),
            definitions: self.definitions.max(other.definitions),
        }
    }
}

pub struct Captured {
    traits: CaptureRows<native::TraitRow>,
    values: Arena<native::Trait>,
    operatives: CaptureRows<native::OperativeRow>,
    definitions: Arena<u32>,
}

impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            traits: 0,
            values: 0,
            operatives: 0,
            definitions: 0,
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            traits: CaptureRows::new(capacity.traits),
            values: Arena::new(capacity.values),
            operatives: CaptureRows::new(capacity.operatives),
            definitions: Arena::new(capacity.definitions),
        }
    }
    pub fn reset(&mut self) {
        self.traits.reset();
        self.values.reset();
        self.operatives.reset();
        self.definitions.reset();
    }
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.traits
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.operatives
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
    }
    pub fn allocated_bytes(&self) -> usize {
        self.traits.allocated_bytes()
            + self.values.allocated_bytes()
            + self.operatives.allocated_bytes()
            + self.definitions.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.traits.grow(&mut capacity.traits);
        self.values.grow(&mut capacity.values);
        self.operatives.grow(&mut capacity.operatives);
        self.definitions.grow(&mut capacity.definitions);
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
        let walked = TABLES.map(|table| should_walk(table, hour, force_full));
        if !walked.iter().any(|&walked| walked) {
            self.traits.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
            self.operatives.outcome = self.traits.outcome;
            return [0; N];
        }
        let _reads = crate::read_accounting::table(TABLES[0]);
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
            },
        );
        let skipped = Outcome::Unavailable {
            reason: SKIPPED_REASON,
        };
        self.traits.outcome = if walked[0] { walk.traits } else { skipped };
        self.operatives.outcome = if walked[1] { walk.operatives } else { skipped };
        self.values.required = walk.required_trait_values;
        self.definitions.required = walk.required_definitions;
        self.values.len = spans_end(&self.traits.rows[..walk.traits.len()], |row| {
            (row.start, row.len)
        });
        self.definitions.len = spans_end(&self.operatives.rows[..walk.operatives.len()], |row| {
            (row.start, row.len)
        });
        [qpc().saturating_sub(started).max(0) as u64, 0]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.traits.outcome, self.operatives.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [self.traits_table(), self.operatives_table()]
    }
    fn traits_table(&self) -> TableView {
        let values = &self.values.first[..self.values.len.min(self.values.first.len())];
        let Some(rows) = self.traits.rows.get(..self.traits.outcome.len()) else {
            return unavailable("character role trait rows exceed buffer");
        };
        if rows.iter().any(|row| span(values, row.start, row.len).is_none()) {
            return unavailable("character role traits outside captured arena");
        }
        table::<tables::CharacterRoleTraits, _>(self.traits.outcome, rows, |row| {
            let (kind, role) = match row.role {
                Role::CountryLeader(ideology) => {
                    (CharacterRoleTraitRow::COUNTRY_LEADER, ideology as u32)
                }
                Role::Advisor(ordinal) => (CharacterRoleTraitRow::ADVISOR, ordinal),
                Role::Scientist => (CharacterRoleTraitRow::SCIENTIST, 0),
            };
            let traits = span(values, row.start, row.len).unwrap();
            CharacterRoleTraitRow {
                character: row.character,
                kind,
                role,
                tokens: traits.iter().map(|value| value.token).collect(),
                definitions: traits.iter().filter_map(|value| value.ordinal).collect(),
            }
        })
    }
    fn operatives_table(&self) -> TableView {
        let values = &self.definitions.first[..self.definitions.len.min(self.definitions.first.len())];
        let Some(rows) = self.operatives.rows.get(..self.operatives.outcome.len()) else {
            return unavailable("character operative rows exceed buffer");
        };
        if rows.iter().any(|row| span(values, row.start, row.len).is_none()) {
            return unavailable("character operative traits outside captured arena");
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
                trait_definitions: span(values, row.start, row.len).unwrap().to_vec(),
            }
        })
    }
}

fn span<V>(values: &[V], start: usize, len: usize) -> Option<&[V]> {
    values.get(start..start.checked_add(len)?)
}

/// The end of the last value span the stored rows use.
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
mod tests {
    use super::*;
    use engine::character_inputs::Leader;
    use replay::rows::TableRows;

    fn captured() -> Captured {
        let mut captured = Captured::with_capacities(&Capacities {
            traits: 3,
            values: 4,
            operatives: 2,
            definitions: 2,
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
        };
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
        assert!(matches!(captured.tables()[0], TableView::Unavailable { .. }));
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
    fn walks_every_hour() {
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        for hour in 0..24 {
            captured.capture(&memory, 0, 0x1000, hour, false);
            assert!(captured.outcomes().iter().all(|outcome| matches!(
                outcome,
                Outcome::Unavailable { reason } if *reason != SKIPPED_REASON
            )));
        }
    }
}
