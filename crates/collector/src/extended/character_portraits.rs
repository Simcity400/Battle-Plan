//! Hourly ordered native inputs with independently bounded owned payloads.
use crate::{
    projection::{Projection, project_table},
    sampling::qpc,
    spooling::unavailable,
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, ccharacter, walk::Outcome};
#[cfg(test)]
use replay::frame::TableView;
use replay::{
    frame::TableInput,
    rows::{CharacterPortraitRow, TableId, tables},
};
pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::CharacterPortraits];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// Inputs for one exclusively owned hourly acquisition.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
}

/// Acquisition ticks in TABLES order. Interrupted work has no completed timing.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureReport {
    pub ticks: [u64; N],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    arena: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 8192,
            arena: 16384,
        }
    }
}
/// The roster is walked as this many contiguous shares, each its own pool job.
pub const PARTS: usize = 4;

#[derive(Clone, Copy)]
struct Certificate {
    scope: (usize, usize, usize, bool),
    revision: (u64, u64),
    acquired: engine::cprovince::UnitOrderWalk,
}

/// One share of the roster: its rows and their portrait entries.
pub struct Part {
    rows: CaptureRows<ccharacter::PortraitRow>,
    arena: Arena<engine::character_portrait_entry::Row>,
    cache: engine::portrait_cache::Cache,
    plan: crate::snapshot_plan::SnapshotPlan,
    certified: Option<Certificate>,
}
impl Part {
    fn new(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(c.rows),
            arena: Arena::new(c.arena),
            cache: engine::portrait_cache::Cache::new(if cfg!(feature = "portrait_changes") {
                c.rows
            } else {
                0
            }),
            plan: crate::snapshot_plan::SnapshotPlan::new(c.rows * 5, c.rows * 48),
            certified: None,
        }
    }
    pub(crate) fn vacant() -> Self {
        Self {
            rows: CaptureRows::new(0),
            arena: Arena::new(0),
            cache: engine::portrait_cache::Cache::new(0),
            plan: crate::snapshot_plan::SnapshotPlan::new(0, 0),
            certified: None,
        }
    }
    fn reset(&mut self) {
        self.rows.reset();
        self.arena.reset();
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.arena.allocated_bytes() + self.cache.allocated_bytes()
            + self.plan.allocated_bytes()
    }
    fn grow(&mut self, c: &mut Capacities) {
        self.rows.grow(&mut c.rows);
        self.arena.grow(&mut c.arena);
        if cfg!(feature = "portrait_changes") {
            self.cache.grow(c.rows);
        }
        self.plan.prepare();
    }
    /// Walk share `part` of the roster.
    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        part: usize,
        census: Option<&engine::ccharacter_manager::PortraitCensus>,
    ) -> u64 {
        self.capture_observed(memory, context, part, census, &crate::portrait_changes::Observer)
    }

    fn capture_observed(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        part: usize,
        census: Option<&engine::ccharacter_manager::PortraitCensus>,
        observer: &dyn engine::portrait_cache::Changes,
    ) -> u64 {
        let _reads = crate::read_accounting::table(TableId::CharacterPortraits);
        let begin = qpc();
        self.reset();
        let before = observer.quiet_revision();
        let identities = census.and_then(|census| census.rows(context.base, context.game_state));
        let scope = (context.base, context.game_state, part, identities.is_some());
        let same_census = identities.is_none_or(|rows| {
            let first = rows.len() * part / PARTS;
            let last = rows.len() * (part + 1) / PARTS;
            self.cache.matches_census(&rows[first..last])
        });
        let certified = !cfg!(feature = "portrait_verify") && before.is_some()
            && same_census && self.certified.is_some_and(|previous| previous.scope == scope
                && (Some(previous.revision) == before || self.cache.unchanged(observer)));
        if self.plan.begin(memory, certified) {
            if before == observer.quiet_revision() {
                let acquired = self.certified.expect("certified previous rows").acquired;
                self.certified = before.map(|revision| Certificate { scope, revision, acquired });
                self.rows.outcome = acquired.outcome;
                self.arena.len = acquired.len;
                self.arena.required = acquired.required;
                crate::portrait_changes::record(engine::portrait_cache::Counts {
                    reused_characters: acquired.outcome.len() as u64,
                    reused_entries: acquired.len as u64,
                    ..Default::default()
                });
                crate::portrait_changes::record_planned_part();
            } else {
                self.rows.outcome = Outcome::Unavailable { reason: "character portrait mutated during acquisition" };
                self.invalidate();
            }
            let ticks = qpc().saturating_sub(begin).max(0) as u64;
            crate::portrait_changes::record_work(true, ticks);
            return ticks;
        }
        let observation = cfg!(feature = "portrait_changes").then_some((
            &mut self.cache,
            observer,
        ));
        let acquired = ccharacter::walk_portraits_with_census(
            &self.plan.view(memory),
            context.base,
            context.game_state,
            part,
            PARTS,
            &mut self.rows.rows,
            &mut self.arena.first,
            observation,
            census,
        );
        if cfg!(feature = "portrait_changes") {
            crate::portrait_changes::record(self.cache.counts);
        }
        self.rows.outcome = acquired.outcome;
        self.arena.len = acquired.len;
        self.arena.required = acquired.required;
        let complete = matches!(acquired.outcome, Outcome::Rows { .. });
        // A cold or changed walk acquires descriptor contents. Compiling that
        // trace would silently keep rereading those contents on every later
        // "unchanged" pass. Prepare only a fully reused walk: its native trace
        // then contains exactly the fresh ownership/header dependencies.
        let reusable_trace = complete && cfg!(feature = "portrait_changes")
            && !cfg!(feature = "portrait_verify") && self.cache.counts.read_characters == 0;
        if !self.plan.finish(reusable_trace) {
            self.rows.outcome = Outcome::Unavailable { reason: "character portrait prepared prefix differs" };
            self.cache.invalidate();
        }
        self.certified = before.filter(|_| complete && before == observer.quiet_revision())
            .map(|revision| Certificate { scope, revision, acquired });
        let ticks = qpc().saturating_sub(begin).max(0) as u64;
        crate::portrait_changes::record_work(false, ticks);
        ticks
    }

    fn invalidate(&mut self) {
        self.cache.invalidate();
        self.plan.invalidate();
        self.certified = None;
    }
}

pub struct Captured {
    parts: Vec<Part>,
    terminal_failure: Option<&'static str>,
}
impl Captured {
    #[cfg(test)]
    fn vacant() -> Self {
        Self {
            parts: (0..PARTS).map(|_| Part::vacant()).collect(),
            terminal_failure: None,
        }
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            parts: (0..PARTS).map(|_| Part::new(c)).collect(),
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
        self.parts.iter().map(Part::allocated_bytes).sum()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        for part in &mut self.parts {
            part.grow(c);
        }
    }
    pub(crate) fn take_part(&mut self, part: usize) -> Part {
        std::mem::replace(&mut self.parts[part], Part::vacant())
    }
    pub(crate) fn restore_part(&mut self, part: usize, mut share: Part) {
        if self.terminal_failure.is_some() {
            share.invalidate();
        }
        self.parts[part] = share;
    }

    /// Run the existing hourly capture on its owning thread. The frame reset
    /// must precede dispatch, just as it precedes direct capture.
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        CaptureReport {
            ticks: self.capture(memory, context.base, context.game_state),
        }
    }

    /// Complete an accepted failed request without reading or retrying native
    /// state. Keep observed growth demand and allocations behind a publication
    /// failure until recycling; partial rows must never reach table conversion.
    /// The caller stores this terminal report even though its ticks are unmeasured.
    pub(crate) fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        self.terminal_failure = Some(reason);
        for part in &mut self.parts {
            part.invalidate();
        }
        CaptureReport::default()
    }

    /// Whole-roster capture on the calling thread, for a callback that never
    /// dispatched the parts.
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

    #[cfg(test)]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection disabled"),
        })
    }

    pub(crate) fn table_inputs(&self, projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        let outcome = self.outcomes()[0];
        if let Outcome::Unavailable { reason } = outcome {
            return [unavailable(reason).into()];
        }
        // Each share is sorted by character; merge them into the order one
        // walk produces and apply its duplicate check across shares.
        let mut rows = Vec::with_capacity(outcome.len());
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
                rows.push((part, row));
            }
        }
        rows.sort_unstable_by_key(|(_, row)| row.character);
        if rows
            .windows(2)
            .any(|pair| pair[0].1.character == pair[1].1.character)
        {
            return [unavailable("duplicate character portrait identity").into()];
        }
        [project_table::<tables::CharacterPortraits, _>(
            projection,
            outcome,
            &rows,
            |(_, row)| u128::from(row.character),
            |(part, row), previous| {
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
            |(part, r)| CharacterPortraitRow {
                character: r.character,
                portraits: part.arena.first[r.portraits[0]..r.portraits[0] + r.portraits[1]]
                    .iter()
                    .map(|p| replay::rows::CharacterPortrait {
                        category: p.category,
                        size: p.size,
                        is_set: p.is_set,
                        kind: p.kind,
                        identity: p.identity.as_bytes().to_vec(),
                    })
                    .collect(),
            },
        )]
    }
}

#[cfg(test)]
mod owned_tests {
    use super::*;

    #[cfg(all(feature = "portrait_changes", not(feature = "portrait_verify")))]
    #[test]
    fn warm_shared_plan_reads_only_headers_and_mutation_reacquires_descriptors() {
        use crate::extended::characters::characters_test_kit as kit;
        use engine::ccharacter_manager::{PortraitCensus, PortraitIdentity, Roster};
        use std::cell::{Cell, RefCell};
        struct Observer(Cell<u64>);
        impl engine::portrait_cache::Changes for Observer {
            fn revision(&self, _: usize) -> Option<(u64, u64)> { Some((1, self.0.get())) }
            fn quiet_revision(&self) -> Option<(u64, u64)> { Some((1, self.0.get())) }
            fn verify(&self) -> bool { false }
        }
        struct Source<'a> { bytes: &'a [u8], calls: RefCell<Vec<(usize, usize)>>, fault: Option<usize> }
        impl Memory for Source<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, output.len()));
                self.fault != Some(address) && kit::memory(self.bytes).copy(address, output)
            }
        }
        let mut bytes = kit::mapped_fixture();
        let roster = Roster::read(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE).unwrap();
        let mut census = PortraitCensus::new(4);
        census.begin(kit::BASE, kit::GAME_STATE);
        for index in 0..roster.count() {
            let entry = roster.at(&kit::memory(&bytes), index).unwrap();
            census.push(PortraitIdentity { pointer: entry.pointer, character: entry.id });
        }
        census.finish(roster.count() as usize);
        let pointer = census.rows(kit::BASE, kit::GAME_STATE).unwrap()[0].pointer;
        let header = pointer + ccharacter::PORTRAIT_DATA;
        let entry = bytes.len();
        bytes.resize(entry + engine::character_portrait_entry::STRIDE, 0);
        let offset = header - kit::GAME_STATE;
        bytes[offset..offset + 8].copy_from_slice(&((kit::GAME_STATE + entry) as u64).to_le_bytes());
        bytes[offset + 8..offset + 12].copy_from_slice(&1_u32.to_le_bytes());
        bytes[offset + 12..offset + 16].copy_from_slice(&1_u32.to_le_bytes());
        bytes[entry + 0x28..entry + 0x30].copy_from_slice(&15_u64.to_le_bytes());
        let context = CaptureContext { base: kit::BASE, game_state: kit::GAME_STATE };
        let mut capacities = Capacities { rows: 4, arena: 4 };
        let mut part = Part::new(&capacities);
        let observer = Observer(Cell::new(0));
        for version in 0..2 {
            bytes[entry + 8] = version;
            observer.0.set(u64::from(version));
            for pass in 0..3 {
                let source = Source { bytes: &bytes, calls: RefCell::new(vec![]), fault: None };
                // Three roster entries: share 1 contains exactly the first one.
                part.capture_observed(&source, context, 1, Some(&census), &observer);
                assert_eq!(part.rows.outcome, Outcome::Rows { len: 1 });
                assert_eq!(part.arena.first[0].is_set, version);
                if pass == 0 { assert!(source.calls.borrow().len() > 1); }
                else { assert_eq!(*source.calls.borrow(), [(header, 16)]); }
                part.grow(&mut capacities);
            }
        }
        let source = Source { bytes: &bytes, calls: RefCell::new(vec![]), fault: Some(header) };
        part.capture_observed(&source, context, 1, Some(&census), &observer);
        assert!(matches!(part.rows.outcome, Outcome::Unavailable { .. }));
        assert_eq!(*source.calls.borrow(), [(header, 16)], "failed prepared header must not be retried");
    }

    #[test]
    fn projection_compares_every_portrait_field_and_discovers_new_characters() {
        use replay::{
            frame::{DecodedFrame, FrameEncoder, Metrics},
            rows::TableRows,
        };
        let mut capture = Captured::with_capacities(&Capacities { rows: 2, arena: 2 });
        for part in &mut capture.parts {
            part.rows.outcome = Outcome::Rows { len: 0 };
        }
        for (part, character) in [(0, 900), (1, 22)] {
            capture.parts[part].rows.outcome = Outcome::Rows { len: 1 };
            capture.parts[part].rows.rows[0] = ccharacter::PortraitRow {
                character,
                portraits: [0, 1],
            };
            capture.parts[part].arena.len = 1;
            let entry = &mut capture.parts[part].arena.first[0];
            entry.category = 2;
            entry.size = 1;
            entry.is_set = 255;
            entry.kind = 1;
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
                    tables: baseline,
                    metrics: Metrics::default(),
                },
                false,
            )
            .unwrap();
        let mut scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &mut scratch);
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::ComparedEqual(_)
        ));
        // Unpublished bytes may retain a previous longer string in the reused slot.
        capture.parts[0].arena.first[0].identity.bytes[200] = 99;
        assert!(matches!(
            capture.table_inputs(Some(&mut projection))[0],
            TableInput::ComparedEqual(_)
        ));
        let original = capture.parts[0].arena.first[0];
        for field in 0..6 {
            let entry = &mut capture.parts[0].arena.first[0];
            *entry = original;
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
        capture.parts[0].arena.first[0] = original;
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
