use collector_primitives::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
};
use collector_projection::{
    projection::{Projection, Witness, held_input, project_table},
    table_conversion::unavailable,
};
use engine_allocation::{collect_vec, copy_vec, map_slice, owned_vec, reserve_exact};
use engine_native_memory::{Memory, walk::Outcome};
use replay_data::{rows::CharacterPortraitRow, rows::TableId, rows::tables};
use replay_frame::frame::TableInput;
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::TableView;
use std::cell::RefCell;
pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::CharacterPortraits];

#[derive(Clone, Copy, Debug)]
pub struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CaptureReport {
    pub ticks: [u64; N],
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    arena: usize,
}

collector_primitives::capacity_max!(Capacities { rows, arena });
impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let acquired = engine_native_politics::character_portraits::count_portraits(memory, base, state, Default::default());
        let count = collector_extended_support::demand::rows(acquired.outcome)?;
        collector_primitives::table_capture::grow_capacity(&mut self.rows, count.div_ceil(PARTS));
        collector_primitives::table_capture::grow_capacity(
            &mut self.arena,
            acquired.required.div_ceil(PARTS),
        );
        Ok(())
    }
}
pub const PARTS: usize = 4;

#[derive(Clone, Copy)]
struct Certificate {
    scope: (usize, usize, usize, bool),
    revision: (u64, u64),
    acquired: engine_native::cprovince::UnitOrderWalk,
    watched: bool,
}

pub struct Part {
    rows: CaptureRows<engine_native_politics::character_portraits::PortraitRow>,
    arena: Arena<engine_native_politics::character_portrait_entry::Row<engine_native_memory::capture_text::Owned>>,
    cache: engine_native_politics::portrait_cache::Cache,
    plan: collector_snapshots::snapshot_plan::SnapshotPlan,
    certified: Option<Certificate>,
    matched: Option<u64>,
    witness: Option<[u64; 6]>,
}

impl Part {
    fn new(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(c.rows),
            arena: Arena::new(c.arena),
            cache: engine_native_politics::portrait_cache::Cache::new(
                if cfg!(feature = "portrait_changes") {
                    c.rows
                } else {
                    0
                },
            )
            .map_err(collector_memory::terminal::io)
            .unwrap_or_else(|_| engine_native_politics::portrait_cache::Cache::empty()),
            plan: collector_snapshots::snapshot_plan::SnapshotPlan::default(),
            certified: None,
            matched: None,
            witness: None,
        }
    }
    pub fn vacant() -> Self {
        Self {
            rows: CaptureRows::new(0),
            arena: Arena::new(0),
            cache: engine_native_politics::portrait_cache::Cache::empty(),
            plan: collector_snapshots::snapshot_plan::SnapshotPlan::default(),
            certified: None,
            matched: None,
            witness: None,
        }
    }
    fn reset(&mut self) {
        self.witness = None;
        self.rows.reset();
        self.arena.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.arena.allocated_bytes()
            + self
                .arena
                .first
                .iter()
                .map(|entry| entry.identity.allocated_bytes())
                .sum::<usize>()
            + self.cache.allocated_bytes()
            + self.plan.allocated_bytes()
    }
    fn grow(&mut self, c: &mut Capacities) {
        self.rows.grow(&mut c.rows);
        self.arena.grow(&mut c.arena);
        if cfg!(feature = "portrait_changes")
            && self
                .cache
                .grow(c.rows)
                .map_err(collector_memory::terminal::io)
                .is_err()
        {
            return;
        }
        self.plan.prepare();
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        part: usize,
        census: Option<&engine_native::ccharacter::manager::PortraitCensus>,
    ) -> u64 {
        self.capture_observed(
            memory,
            context,
            part,
            census,
            &crate::portrait_changes::Observer,
        )
    }

    pub fn capture_observed(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        part: usize,
        census: Option<&engine_native::ccharacter::manager::PortraitCensus>,
        observer: &dyn engine_native_politics::portrait_cache::Changes,
    ) -> u64 {
        let _reads = collector_memory::read_accounting::table(TableId::CharacterPortraits);
        let begin = qpc();
        self.reset();
        let before = observer.quiet_revision();
        let roster = collector_observers::roster_changes::roster();
        let before_any = observer.any_revision();
        let identities = census.and_then(|census| census.rows(context.base, context.game_state));
        let scope = (context.base, context.game_state, part, identities.is_some());
        let stamp = census.map(engine_native::ccharacter::manager::PortraitCensus::stamp);
        let same_census = identities.is_none_or(|rows| {
            let first = rows.len() * part / PARTS;
            let last = rows.len() * (part + 1) / PARTS;
            (stamp.is_some() && self.matched == stamp)
                || self.cache.matches_census(&rows[first..last])
        });
        let quiet_since =
            |previous: Certificate| previous.watched && Some(previous.revision) == before;
        let certified = before.is_some()
            && same_census
            && self.certified.is_some_and(|previous| {
                previous.scope == scope && (quiet_since(previous) || self.cache.unchanged(observer))
            });
        let quiet = certified && identities.is_some() && self.certified.is_some_and(quiet_since);
        self.matched = stamp.filter(|_| quiet);
        if quiet {
            crate::portrait_changes::record_quiet_part();
        }
        if quiet || self.plan.begin(memory, certified) {
            if before == observer.quiet_revision() {
                let previous = self.certified.expect("certified previous rows");
                let acquired = previous.acquired;
                self.certified = before.map(|revision| Certificate {
                    scope,
                    revision,
                    acquired,
                    watched: previous.watched,
                });
                self.rows.outcome = acquired.outcome;
                self.arena.len = acquired.len;
                self.arena.required = acquired.required;
                crate::portrait_changes::record(engine_native_politics::portrait_cache::Counts {
                    reused_characters: acquired.outcome.len() as u64,
                    reused_entries: acquired.len as u64,
                    ..Default::default()
                });
                crate::portrait_changes::record_planned_part();
                if quiet
                    && let (Some(revision), Some(token)) = (before, roster)
                    && collector_observers::roster_changes::roster() == roster
                {
                    self.witness = Some([
                        context.base as u64,
                        context.game_state as u64,
                        revision.0,
                        revision.1,
                        token.0,
                        token.1,
                    ]);
                }
                let ticks = qpc().saturating_sub(begin).max(0) as u64;
                crate::portrait_changes::record_work(true, ticks);
                return ticks;
            }
            self.invalidate();
            self.plan.begin(memory, false);
        }
        let observation = cfg!(feature = "portrait_changes").then_some((&mut self.cache, observer));
        let acquired = engine_native_politics::character_portraits::walk_portraits(
            &self.plan.view(memory),
            context.base,
            context.game_state,
            &mut self.rows.rows,
            &mut self.arena.first,
            engine_native_politics::character_portraits::PortraitWalk {
                part,
                parts: PARTS,
                observation,
                census,
            },
        );
        if cfg!(feature = "portrait_changes") {
            crate::portrait_changes::record(self.cache.counts);
        }
        self.rows.outcome = acquired.outcome;
        self.arena.len = acquired.len;
        self.arena.required = acquired.required;
        let complete = matches!(acquired.outcome, Outcome::Rows { .. });
        let reusable_trace = complete
            && cfg!(feature = "portrait_changes")
            && self.cache.counts.read_characters == 0;
        if !self.plan.finish(reusable_trace) {
            self.rows.outcome = Outcome::Unavailable {
                reason: "character portrait prepared prefix differs",
            };
            self.cache.invalidate();
        }
        let watched = cfg!(feature = "portrait_changes") && self.cache.all_watched();
        self.certified = before
            .filter(|_| {
                complete
                    && before == observer.quiet_revision()
                    && before_any == observer.any_revision()
            })
            .map(|revision| Certificate {
                scope,
                revision,
                acquired,
                watched,
            });
        let ticks = qpc().saturating_sub(begin).max(0) as u64;
        crate::portrait_changes::record_work(false, ticks);
        ticks
    }

    fn invalidate(&mut self) {
        self.cache.invalidate();
        self.plan.invalidate();
        self.certified = None;
        self.matched = None;
        self.witness = None;
    }
}

pub struct Captured {
    parts: Vec<Part>,
    ordering: RefCell<Vec<(usize, usize)>>,
    terminal_failure: Option<&'static str>,
}
impl Captured {
    #[cfg(test)]
    fn vacant() -> Self {
        Self {
            parts: owned_vec(collect_vec((0..PARTS).map(|_| Part::vacant()))),
            ordering: RefCell::default(),
            terminal_failure: None,
        }
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            parts: owned_vec(collect_vec((0..PARTS).map(|_| Part::new(c)))),
            ordering: RefCell::default(),
            terminal_failure: None,
        }
    }
    pub fn reset(&mut self) {
        self.terminal_failure = None;
        for part in &mut self.parts {
            part.reset();
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        self.parts.iter().map(Part::allocated_bytes).sum::<usize>()
            + self.ordering.borrow().capacity() * size_of::<(usize, usize)>()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        for part in &mut self.parts {
            part.grow(c);
        }
    }
    pub fn take_part(&mut self, part: usize) -> Part {
        std::mem::replace(&mut self.parts[part], Part::vacant())
    }
    pub fn restore_part(&mut self, part: usize, mut share: Part) {
        if self.terminal_failure.is_some() {
            share.invalidate();
        }
        self.parts[part] = share;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        CaptureReport {
            ticks: self.capture(memory, context.base, context.game_state),
        }
    }

    pub fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        self.terminal_failure = Some(reason);
        for part in &mut self.parts {
            part.invalidate();
        }
        CaptureReport::default()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let context = CaptureContext {
            base,
            game_state: state,
        };
        self.terminal_failure = None;
        let mut ticks = 0;
        for (part, share) in self.parts.iter_mut().enumerate() {
            ticks += share.capture(memory, context, part, None);
        }
        [ticks]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        if let Some(reason) = self.terminal_failure {
            return [Outcome::Unavailable { reason }];
        }
        let (mut len, mut unread) = (0_usize, 0_u32);
        for part in &self.parts {
            match part.rows.outcome {
                Outcome::Unavailable { reason } => return [Outcome::Unavailable { reason }],
                Outcome::Rows { len: rows } => len += rows,
                Outcome::Truncated {
                    len: rows,
                    unread_rows,
                } => {
                    len += rows;
                    unread = unread.saturating_add(unread_rows);
                }
            }
        }
        [if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        }]
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection disabled"),
        })
    }

    fn witness(&self) -> Option<Witness> {
        if self.terminal_failure.is_some() {
            return None;
        }
        let first = self.parts.first()?.witness?;
        self.parts
            .iter()
            .all(|part| part.witness == Some(first))
            .then(|| Witness::new(first))
    }

    pub fn table_inputs(&self, projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        [held_input::<tables::CharacterPortraits>(
            projection,
            self.witness(),
            |projection| {
                let [input] = self.projected(projection);
                input
            },
        )]
    }

    fn projected(&self, projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        let outcome = self.outcomes()[0];
        if let Outcome::Unavailable { reason } = outcome {
            return [unavailable(reason).into()];
        }
        let mut rows = self.ordering.borrow_mut();
        let additional = outcome.len().saturating_sub(rows.len());
        if reserve_exact(&mut rows, additional).is_err() {
            return [unavailable("collector allocation failed").into()];
        }
        for part in &self.parts {
            let Some(source) = part.rows.rows.get(..part.rows.outcome.len()) else {
                return [unavailable("walker row count exceeds allocated buffer").into()];
            };
            let Some(entries) = part.arena.first.get(..part.arena.len) else {
                return [unavailable("character portraits exceed captured arena").into()];
            };
            for row in source {
                let Some(_) = row.portraits[0]
                    .checked_add(row.portraits[1])
                    .and_then(|end| entries.get(row.portraits[0]..end))
                else {
                    return [unavailable("character portraits exceed captured arena").into()];
                };
            }
        }
        if rows.len() != outcome.len()
            || rows.iter().any(|&(part, row)| {
                self.parts
                    .get(part)
                    .is_none_or(|part| row >= part.rows.outcome.len())
            })
        {
            rows.clear();
            for (part, share) in self.parts.iter().enumerate() {
                rows.extend((0..share.rows.outcome.len()).map(|row| (part, row)));
            }
        }
        let character = |&(part, row): &(usize, usize)| self.parts[part].rows.rows[row].character;
        if !rows
            .windows(2)
            .all(|pair| character(&pair[0]) < character(&pair[1]))
        {
            rows.sort_unstable_by_key(character);
        }
        if rows
            .windows(2)
            .any(|pair| character(&pair[0]) == character(&pair[1]))
        {
            return [unavailable("duplicate character portrait identity").into()];
        }
        [project_table::<tables::CharacterPortraits, _>(
            projection,
            outcome,
            &rows,
            |entry| u128::from(character(entry)),
            |(part, row), previous| {
                let part = &self.parts[*part];
                let row = &part.rows.rows[*row];
                let portraits =
                    &part.arena.first[row.portraits[0]..row.portraits[0] + row.portraits[1]];
                row.character == previous.character
                    && portraits.len() == previous.portraits.len()
                    && portraits
                        .iter()
                        .zip(&previous.portraits)
                        .all(|(current, previous)| {
                            current.category == previous.category
                                && current.size == previous.size
                                && current.is_set == previous.is_set
                                && current.kind == previous.kind
                                && current.identity.as_bytes() == previous.identity
                        })
            },
            |(part, r)| {
                let part = &self.parts[*part];
                let r = &part.rows.rows[*r];
                CharacterPortraitRow {
                    character: r.character,
                    portraits: owned_vec(map_slice(
                        &part.arena.first[r.portraits[0]..r.portraits[0] + r.portraits[1]],
                        |p| replay_data::rows::CharacterPortrait {
                            category: p.category,
                            size: p.size,
                            is_set: p.is_set,
                            kind: p.kind,
                            identity: owned_vec(copy_vec(p.identity.as_bytes())),
                        },
                    )),
                }
            },
        )]
    }
}

#[cfg(test)]
#[path = "character_portraits/projection_tests.rs"]
mod projection_tests;

#[cfg(test)]
mod owned_tests {
    use engine_native::ccharacter;
    use super::*;
    use crate::extended::characters::characters_test_kit as kit;
    use engine_native::ccharacter::manager::{PortraitCensus, PortraitIdentity, Roster};
    #[cfg(feature = "portrait_changes")]
    use std::cell::{Cell, RefCell};

    fn share_fixture() -> (Vec<u8>, PortraitCensus, usize, usize) {
        let mut bytes = kit::mapped_fixture();
        let roster = Roster::read(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE).unwrap();
        let mut census = PortraitCensus::try_new(4).unwrap();
        census.begin(kit::BASE, kit::GAME_STATE);
        for index in 0..roster.count() {
            let entry = roster.at(&kit::memory(&bytes), index).unwrap();
            census.push(PortraitIdentity {
                pointer: entry.pointer,
                character: entry.id,
            });
        }
        census.finish(roster.count() as usize);
        let pointer = census.rows(kit::BASE, kit::GAME_STATE).unwrap()[0].pointer;
        let header = pointer + ccharacter::PORTRAIT_DATA;
        let entry = bytes.len();
        bytes.resize(entry + engine_native_politics::character_portrait_entry::STRIDE, 0);
        let offset = header - kit::GAME_STATE;
        bytes[offset..offset + 8]
            .copy_from_slice(&((kit::GAME_STATE + entry) as u64).to_le_bytes());
        bytes[offset + 8..offset + 12].copy_from_slice(&1_u32.to_le_bytes());
        bytes[offset + 12..offset + 16].copy_from_slice(&1_u32.to_le_bytes());
        bytes[entry + 0x28..entry + 0x30].copy_from_slice(&15_u64.to_le_bytes());
        (bytes, census, header, entry)
    }

    #[cfg(feature = "portrait_changes")]
    #[test]
    fn unwatched_writers_keep_shares_quiet_and_void_a_walk_they_overlap() {
        struct Observer {
            any: Cell<u64>,
            during: Cell<bool>,
        }
        impl engine_native_politics::portrait_cache::Changes for Observer {
            fn revision(&self, _: usize) -> Option<(u64, u64)> {
                Some((1, 0))
            }
            fn quiet_revision(&self) -> Option<(u64, u64)> {
                Some((1, 0))
            }
            fn any_revision(&self) -> Option<(u64, u64)> {
                let any = self.any.get();
                self.any.set(any + u64::from(self.during.get()));
                Some((1, any))
            }
        }
        struct Counted<'a> {
            bytes: &'a [u8],
            reads: Cell<usize>,
        }
        impl Memory for Counted<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                kit::memory(self.bytes).copy(address, output)
            }
        }
        let (bytes, census, _, _) = share_fixture();
        let context = CaptureContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut capacities = Capacities { rows: 4, arena: 4 };
        let mut part = Part::new(&capacities);
        let observer = Observer {
            any: Cell::new(0),
            during: Cell::new(true),
        };
        let mut capture = |part: &mut Part| {
            let memory = Counted {
                bytes: &bytes,
                reads: Cell::new(0),
            };
            part.capture_observed(&memory, context, 1, Some(&census), &observer);
            assert_eq!(part.rows.outcome, Outcome::Rows { len: 1 });
            part.grow(&mut capacities);
            memory.reads.get()
        };
        assert!(capture(&mut part) > 0);
        observer.during.set(false);
        assert!(
            capture(&mut part) > 0,
            "an overlapped walk certifies nothing"
        );
        for _ in 0..3 {
            observer.any.set(observer.any.get() + 5);
            assert_eq!(
                capture(&mut part),
                0,
                "unwatched writers leave the share quiet"
            );
        }
    }

    #[cfg(feature = "portrait_changes")]
    #[test]
    fn quiet_shares_read_nothing_warm_plans_read_only_headers_and_mutation_reacquires() {
        struct Observer {
            holders: Cell<u64>,
            domain: Cell<u64>,
        }
        impl engine_native_politics::portrait_cache::Changes for Observer {
            fn revision(&self, _: usize) -> Option<(u64, u64)> {
                Some((1, self.holders.get()))
            }
            fn quiet_revision(&self) -> Option<(u64, u64)> {
                Some((1, self.domain.get()))
            }
        }
        struct Source<'a> {
            bytes: &'a [u8],
            calls: RefCell<Vec<(usize, usize)>>,
            fault: Option<usize>,
        }
        impl Memory for Source<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, output.len()));
                self.fault != Some(address) && kit::memory(self.bytes).copy(address, output)
            }
        }
        let (mut bytes, census, header, entry) = share_fixture();
        let context = CaptureContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut capacities = Capacities { rows: 4, arena: 4 };
        let mut part = Part::new(&capacities);
        let observer = Observer {
            holders: Cell::new(0),
            domain: Cell::new(0),
        };
        let bump = |cell: &Cell<u64>| cell.set(cell.get() + 1);
        for version in 0..2 {
            bytes[entry + 8] = version;
            observer.holders.set(u64::from(version));
            bump(&observer.domain);
            for pass in 0..4 {
                if pass == 2 {
                    bump(&observer.domain);
                }
                let source = Source {
                    bytes: &bytes,
                    calls: RefCell::new(vec![]),
                    fault: None,
                };
                part.capture_observed(&source, context, 1, Some(&census), &observer);
                assert_eq!(part.rows.outcome, Outcome::Rows { len: 1 });
                assert_eq!(part.arena.first[0].is_set, version);
                match pass {
                    0 => assert!(source.calls.borrow().len() > 1),
                    2 => assert_eq!(*source.calls.borrow(), [(header, 16)]),
                    _ => assert!(
                        source.calls.borrow().is_empty(),
                        "a quiet share reads nothing"
                    ),
                }
                part.grow(&mut capacities);
            }
        }
        bump(&observer.domain);
        let source = Source {
            bytes: &bytes,
            calls: RefCell::new(vec![]),
            fault: Some(header),
        };
        part.capture_observed(&source, context, 1, Some(&census), &observer);
        assert!(matches!(part.rows.outcome, Outcome::Unavailable { .. }));
        assert_eq!(
            *source.calls.borrow(),
            [(header, 16)],
            "failed prepared header must not be retried"
        );
    }

    #[cfg(feature = "portrait_changes")]
    fn census_of(bytes: &[u8]) -> PortraitCensus {
        let roster = Roster::read(&kit::memory(bytes), kit::BASE, kit::GAME_STATE).unwrap();
        let mut census = PortraitCensus::try_new(4).unwrap();
        census.begin(kit::BASE, kit::GAME_STATE);
        for index in 0..roster.count() {
            let entry = roster.at(&kit::memory(bytes), index).unwrap();
            census.push(PortraitIdentity {
                pointer: entry.pointer,
                character: entry.id,
            });
        }
        census.finish(roster.count() as usize);
        census
    }

    #[cfg(feature = "portrait_changes")]
    #[test]
    fn a_quiet_share_stays_quiet_across_growth_and_a_new_census() {
        struct Observer;
        impl engine_native_politics::portrait_cache::Changes for Observer {
            fn revision(&self, _: usize) -> Option<(u64, u64)> {
                Some((1, 0))
            }
            fn quiet_revision(&self) -> Option<(u64, u64)> {
                Some((1, 0))
            }
        }
        struct Counted<'a> {
            bytes: &'a [u8],
            reads: Cell<usize>,
        }
        impl Memory for Counted<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                kit::memory(self.bytes).copy(address, output)
            }
        }
        let (bytes, census, _, _) = share_fixture();
        let context = CaptureContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
        };
        let mut capacities = Capacities { rows: 4, arena: 4 };
        let mut part = Part::new(&capacities);
        let capture = |part: &mut Part, census: &PortraitCensus| {
            let memory = Counted {
                bytes: &bytes,
                reads: Cell::new(0),
            };
            part.capture_observed(&memory, context, 1, Some(census), &Observer);
            assert_eq!(part.rows.outcome, Outcome::Rows { len: 1 });
            memory.reads.get()
        };
        assert!(capture(&mut part, &census) > 0);
        part.grow(&mut capacities);
        assert_eq!(capture(&mut part, &census), 0);
        capacities.rows = 64;
        part.grow(&mut capacities);
        let again = census_of(&bytes);
        assert_ne!(again.stamp(), census.stamp());
        assert_eq!(
            capture(&mut part, &again),
            0,
            "growth keeps the share's certified descriptors"
        );
    }

    #[test]
    fn projection_compares_every_portrait_field_and_discovers_new_characters() {
        use replay_data::rows::TableRows;
        use replay_frame::{frame::DecodedFrame, frame::FrameEncoder, frame::Metrics};
        let mut capture = Captured::with_capacities(&Capacities { rows: 2, arena: 2 });
        for part in &mut capture.parts {
            part.rows.outcome = Outcome::Rows { len: 0 };
        }
        for (part, character) in [(0, 900), (1, 22)] {
            capture.parts[part].rows.outcome = Outcome::Rows { len: 1 };
            capture.parts[part].rows.rows[0] = engine_native_politics::character_portraits::PortraitRow {
                character,
                portraits: [0, 1],
            };
            capture.parts[part].arena.len = 1;
            let entry = &mut capture.parts[part].arena.first[0];
            entry.category = 2;
            entry.size = 1;
            entry.is_set = 255;
            entry.kind = 1;
            entry.identity.bytes.resize(201, 0);
            entry.identity.bytes[..3].copy_from_slice(&[0xff, 0, b'a']);
            entry.identity.len = 3;
        }
        let mut baseline = std::array::from_fn(|_| unavailable("outside portrait fixture"));
        baseline[TABLES[0].index()] = capture.tables()[0].clone();
        let mut encoder = FrameEncoder::new();
        encoder
            .encode(
                DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: Metrics::default(),
                },
                false,
            )
            .unwrap();
        let scratch = collector_projection::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &scratch);
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::ComparedEqual(_)
        ));
        capture.parts[0].arena.first[0].identity.bytes[200] = 99;
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::ComparedEqual(_)
        ));
        let original = capture.parts[0].arena.first[0].clone();
        for field in 0..6 {
            let entry = &mut capture.parts[0].arena.first[0];
            *entry = original.clone();
            match field {
                0 => entry.category ^= 1,
                1 => entry.size ^= 1,
                2 => entry.is_set ^= 1,
                3 => entry.kind ^= 1,
                4 => entry.identity.bytes[0] ^= 1,
                _ => entry.identity.len -= 1,
            }
            let [
                TableInput::ComparedChanges {
                    indices,
                    rows: TableRows::CharacterPortraits(rows),
                    ..
                },
            ] = capture.table_inputs(Some(&mut projection))
            else {
                panic!("changed portrait field {field} must replace its row");
            };
            assert_eq!(indices, [1]);
            let [TableView::Complete(TableRows::CharacterPortraits(expected))] = capture.tables()
            else {
                panic!("complete portrait fixture")
            };
            assert_eq!(rows, expected[1..]);
        }
        capture.parts[0].arena.first[0] = original.clone();
        capture.parts[0].rows.rows[0].portraits[1] = 0;
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::ComparedChanges { .. }
        ));
        capture.parts[0].rows.rows[0].portraits[1] = 1;
        capture.parts[0].rows.rows[0].character = 901;
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::Sampled(TableView::Complete(_))
        ));
        capture.parts[0].rows.rows[0].character = 22;
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        capture.parts[0].rows.rows[0].character = 900;
        capture.parts[0].rows.rows[0].portraits[0] = usize::MAX;
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
    }

    fn allocations(c: &Captured) -> [(usize, usize, usize); 2] {
        fn vector<T>(v: &Vec<T>) -> (usize, usize, usize) {
            (v.as_ptr() as usize, v.len(), v.capacity())
        }
        [
            vector(&c.parts[0].rows.rows),
            vector(&c.parts[0].arena.first),
        ]
    }

    #[test]
    fn owned_failure_gates_publication_and_preserves_primary_growth_demand() {
        let initial = Capacities { rows: 2, arena: 2 };
        for (outcome, expected_rows) in [
            (Outcome::Rows { len: 2 }, 4),
            (
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 5,
                },
                8,
            ),
        ] {
            let mut capacities = initial;
            let mut expected_capacity = initial;
            let mut frame = Captured::with_capacities(&initial);
            let mut owned = std::mem::replace(&mut frame, Captured::vacant());
            let mut expected = Captured::with_capacities(&initial);
            assert_eq!(frame.allocated_bytes(), 0);
            for capture in [&mut owned, &mut expected] {
                capture.parts[0].rows.outcome = outcome;
                capture.parts[0].rows.rows[0].portraits = [usize::MAX, 1];
                capture.parts[0].arena.len = 1;
                capture.parts[0].arena.required = 9;
            }
            let before = allocations(&owned);
            let report = owned.fail_owned_capture("owned acquisition failed");
            assert_eq!(report.ticks, [0]);
            assert_eq!(allocations(&owned), before);
            assert_eq!(owned.parts[0].rows.outcome, outcome);
            assert_eq!(
                (owned.parts[0].arena.len, owned.parts[0].arena.required),
                (1, 9)
            );
            assert_eq!(
                owned.outcomes(),
                [Outcome::Unavailable {
                    reason: "owned acquisition failed"
                }]
            );
            assert!(matches!(owned.tables()[0], TableView::Unavailable { .. }));
            frame = owned;
            frame.grow(&mut capacities);
            expected.grow(&mut expected_capacity);
            assert!(capacities == expected_capacity);
            assert_eq!((capacities.rows, capacities.arena), (expected_rows, 16));
            assert_eq!(
                allocations(&frame).map(|(_, len, cap)| (len, cap)),
                allocations(&expected).map(|(_, len, cap)| (len, cap))
            );
            let retained = allocations(&frame);
            frame.reset();
            frame.grow(&mut capacities);
            assert!(frame.terminal_failure.is_none());
            assert_eq!(
                (frame.parts[0].arena.len, frame.parts[0].arena.required),
                (0, 0)
            );
            assert_eq!(allocations(&frame), retained);
        }
    }

    #[test]
    fn owned_and_direct_capture_keep_one_first_fault_read_after_previous_failure() {
        use std::cell::RefCell;
        #[derive(Default)]
        struct Unreadable(RefCell<Vec<(usize, usize)>>);
        impl Memory for Unreadable {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.0.borrow_mut().push((address, output.len()));
                false
            }
        }
        let capacities = Capacities { rows: 2, arena: 3 };
        let context = CaptureContext {
            base: 0x140000000,
            game_state: 0x1000,
        };
        let mut direct = Captured::with_capacities(&capacities);
        let mut owned = Captured::with_capacities(&capacities);
        let before = allocations(&owned);
        let direct_memory = Unreadable::default();
        let owned_memory = Unreadable::default();
        direct.capture(&direct_memory, context.base, context.game_state);
        for owned_entry in [false, true] {
            owned.fail_owned_capture("previous request failed");
            owned_memory.0.borrow_mut().clear();
            if owned_entry {
                owned.capture_owned(&owned_memory, context);
            } else {
                owned.capture(&owned_memory, context.base, context.game_state);
            }
            assert!(owned.terminal_failure.is_none());
            assert_eq!(*owned_memory.0.borrow(), *direct_memory.0.borrow());
            assert_eq!(owned_memory.0.borrow().len(), PARTS);
            assert_eq!(owned.outcomes(), direct.outcomes());
            assert_eq!(owned.tables(), direct.tables());
            assert_eq!(
                (owned.parts[0].arena.len, owned.parts[0].arena.required),
                (0, 0)
            );
            assert_eq!(allocations(&owned), before);
        }
    }
}
