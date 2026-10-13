//! The character-scope trigger inputs the character role jobs do not read:
//! every country-leader, advisor and scientist role's traits
//! (CharacterRoleTraits) and each nonzero unit-leader reference with its
//! object's operands (CharacterOperatives). Their writers (effects, commands
//! and operative recruitment) run at any hour; see
//! [`engine::character_inputs`].
//!
//! Every hour the Gated job walks the roster job's admitted role census
//! ([`super::characters::census`]) instead of the roster
//! ([`Captured::capture_census`]); [`RoleStore`] carries each character's
//! country-leader and advisor rows while no hooked writer touched them
//! ([`crate::character_changes`]), and on a quiet hour carries them all
//! without a lookup. Without an admitted census the pack walks the roster
//! itself and the store forgets its rows.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::allocation::{copy_vec, map_slice, owned_vec, vec};
use engine::{
    Memory,
    character_inputs::{self as native, Role},
    walk::Outcome,
};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{CharacterOperativeRow, CharacterRoleTraitRow, Recorded, TableId, tables},
};

mod role_store;
pub(crate) use role_store::RoleStore;

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::CharacterRoleTraits, TableId::CharacterOperatives];
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("character role traits and operative links", 0, &[0, 1])];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    traits: usize,
    values: usize,
    operatives: usize,
    definitions: usize,
    bytes: usize,
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
        raise(&mut self.bytes, rows(demand.bytes, 1));
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
    pub bytes: usize,
}

impl Demand {
    /// The live demand, from the walk with no storage.
    pub(crate) fn observe(
        memory: &impl Memory,
        base: usize,
        game_state: usize,
    ) -> Result<Self, &'static str> {
        let walk = native::walk(
            memory,
            base,
            game_state,
            native::Buffers {
                traits: &mut [],
                trait_values: &mut [],
                operatives: &mut [],
                definitions: &mut [],
                bytes: &mut [],
            },
        );
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

    pub(crate) fn exceeds(&self, seen: &Self) -> bool {
        self.traits > seen.traits
            || self.values > seen.values
            || self.operatives > seen.operatives
            || self.definitions > seen.definitions
            || self.bytes > seen.bytes
    }

    pub(crate) fn max(self, other: Self) -> Self {
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
    /// The role store content whose CharacterRoleTraits table these buffers
    /// hold since their latest census walk, with its result. A walk or a
    /// failure clears it; `reset` and growth, which only appends default rows
    /// after the held ones, keep the contents.
    held: Option<(u64, HeldTraits)>,
}

/// A held CharacterRoleTraits table's outcome and trait arena.
#[derive(Clone, Copy, Debug)]
struct HeldTraits {
    outcome: Outcome,
    values_len: usize,
    values_required: usize,
}

/// Optional name failures keep the preexisting row and are counted off-thread.
#[derive(Default)]
pub(crate) struct NamingWork {
    refusals: Vec<(&'static str, u64)>,
}

impl NamingWork {
    pub(super) fn refuse(&mut self, reason: &'static str) -> std::io::Result<()> {
        let index = match self.refusals.binary_search_by_key(&reason, |entry| entry.0) {
            Ok(index) => index,
            Err(index) => {
                engine::allocation::reserve(&mut self.refusals, 1)?;
                self.refusals.insert(index, (reason, 0));
                index
            }
        };
        self.refusals[index].1 = self.refusals[index].1.saturating_add(1);
        Ok(())
    }

    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "refused": self.refusals.iter().fold(0_u64, |sum, (_, count)| sum.saturating_add(*count)),
            "refusals": crate::status::map(self.refusals.iter().map(|&(reason, count)| (reason, count))),
            "scope": "optional hourly CharacterOperatives names and UnitLeaders codenames; existing row fields remain available",
        })
    }
}

impl Captured {
    pub(crate) fn observe_naming(&self, work: &mut NamingWork) -> std::io::Result<()> {
        for row in &self.operatives.rows[..self.operatives.outcome.len()] {
            for reason in [row.name_reason, row.codename_reason].into_iter().flatten() {
                work.refuse(reason)?;
            }
        }
        Ok(())
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
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
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.held = None;
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
        self.capture_mode(memory, base, game_state, hour, force_full, true)
    }

    #[cfg(test)]
    fn capture_fixed(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_mode(memory, base, game_state, hour, force_full, false)
    }

    /// The hour's walk over the roster job's admitted role census, carrying
    /// each certified character's country-leader and advisor rows from
    /// `store`. Without a census the roster is walked and `store` forgets.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture_census(
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
            &crate::character_changes::Observer,
            &role_store::LeaderObserver,
            crate::sampled_verify::Check::burned_in(hour, crate::character_changes::VERIFY),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture_census_with(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        census: Option<super::characters::census::Admitted<'_>>,
        store: Option<&mut RoleStore>,
        changes: &dyn crate::character_changes::Changes,
        leaders: &dyn role_store::LeaderChanges,
        check: crate::sampled_verify::Check,
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
        let _reads = crate::read_accounting::table(TABLES[0]);
        let started = qpc();
        let mut walk = native::CensusWalk::growing(native::GrowingBuffers {
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
                check,
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
            // The frame's buffers hold this hour's CharacterRoleTraits.
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

    /// Which tables this hour walks; `None` marks both skipped.
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

    fn capture_mode(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        growing: bool,
    ) -> [u64; N] {
        self.held = None;
        self.reset();
        let Some(walked) = self.walked(hour, force_full) else {
            return [0; N];
        };
        let _reads = crate::read_accounting::table(TABLES[0]);
        let started = qpc();
        let walk = if growing {
            native::walk_growing(
                memory,
                base,
                game_state,
                native::GrowingBuffers {
                    traits: &mut self.traits.rows,
                    trait_values: &mut self.values.first,
                    operatives: &mut self.operatives.rows,
                    definitions: &mut self.definitions.first,
                    bytes: &mut self.bytes.first,
                },
            )
        } else {
            native::walk(
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
            )
        };
        self.install(walked, walk);
        [qpc().saturating_sub(started).max(0) as u64, 0]
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
    pub fn tables(&self) -> [TableView; N] {
        [self.traits_table(), self.operatives_table()]
    }
    fn traits_table(&self) -> TableView {
        let values = &self.values.first[..self.values.len.min(self.values.first.len())];
        let Some(rows) = self.traits.rows.get(..self.traits.outcome.len()) else {
            return unavailable("character role trait rows exceed buffer");
        };
        if rows
            .iter()
            .any(|row| span(values, row.start, row.len).is_none())
        {
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
                tokens: owned_vec(map_slice(traits, |value| value.token)),
                definitions: owned_vec((|| {
                    let mut definitions = vec(traits.len())?;
                    definitions.extend(traits.iter().filter_map(|value| value.ordinal));
                    Ok(definitions)
                })()),
            }
        })
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

#[cfg(test)]
pub(crate) fn allocation_failure_exercise() {
    let _home = super::daily_intel::Home::try_new(2, 16);
    if crate::allocation::failed() {
        return;
    }
    let _roles = super::characters::census::RoleCensus::new(2);
    if crate::allocation::failed() {
        return;
    }
    let mut captured = Captured::with_capacities(&Capacities {
        traits: 1,
        values: 2,
        operatives: 1,
        definitions: 2,
        bytes: 32,
    });
    if crate::allocation::failed() {
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
    name: engine::cname_group_member::Inputs,
    bytes: &[u8],
) -> replay::rows::NameOperands {
    replay::rows::NameOperands {
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
#[path = "character_inputs/role_store_tests.rs"]
mod role_store_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use engine::character_inputs::Leader;
    use replay::rows::TableRows;

    #[test]
    fn programmatic_override_survives_projection_when_native_shows_it() {
        let naming = operands(
            engine::cname_group_member::Inputs {
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
            codename: Some(engine::cname_group_member::Inputs {
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
                    codename: Recorded::Value(Some(replay::rows::NameOperands {
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
    fn cold_character_input_defaults_have_no_population_or_text_capacity() {
        assert_eq!(
            Captured::with_capacities(&Capacities::default()).allocated_bytes(),
            0
        );
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
