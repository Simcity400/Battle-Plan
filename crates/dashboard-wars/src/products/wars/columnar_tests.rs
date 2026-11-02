//! The family's columnar preparation held to the row walk ([`scan`]) on the same hours, written
//! as the native recorder writes them: every viewpoint's wars, their casualties at every hour,
//! and the groups and contributions at every hour ([`at_hour`]), across segment starts, gaps,
//! cuts and a resume.

use super::test_fixtures::recorded;
use super::{
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    query::{Walked, walk_snapshot},
    *,
};
use dashboard_base::prepared::series::tests::Fixture;
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{TableRows, TableSchema},
    scope::CaptureMode,
};
use std::sync::Arc;

fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

fn write_hour(walk: &mut Synthetic, frame: &DecodedFrame) {
    macro_rules! tables { ($($table:ident),* $(,)?) => { $(write::<tables::$table>(walk, frame);)* }; }
    tables!(
        Campaign,
        Countries,
        CountryLinks,
        WarRelations,
        WarOwnedRelations,
        WarRelationGoals,
        WarOverviewCountries,
        WarNameRules,
        Factions,
        FactionMembers,
        LocalPlayer,
        Capitulation,
        LexicalTokens,
        CountryAppearance,
        ReservedCountryNaming,
        Politics,
        PoliticalParties,
        Autonomy,
        States,
        WarParticipation,
        WarLendLeaseHistories,
    );
}

/// The projected hours of `frames`, a projector starting cold at each of `starts` and at each
/// walk's first hour (after a gap the walk starts again).
fn projected(frames: &[DecodedFrame], starts: &[i32]) -> Vec<facts::Hour> {
    let mut out = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT).unwrap();
    let needs = facts::needs().unwrap();
    let mut last: Option<i32> = None;
    for (ordinal, frame) in (0..).zip(frames) {
        let gap = last.is_some_and(|last| last + 1 != frame.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, frame.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write_hour(walk, frame);
        let cold = starts.contains(&frame.hour);
        if cold {
            projector = Projector::new(FormatVersion::CURRENT).unwrap();
        }
        walk.end(|state, manifest, _, first| {
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &needs,
                (FormatVersion::CURRENT, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: frame.hour,
                        run: 0,
                        ordinal,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                        world,
                    },
                    &mut out,
                )
                .unwrap();
        })
        .unwrap();
        last = Some(frame.hour);
    }
    out
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

/// A publication built from projected hours, cut after each of `cuts`' hours; when `resume`
/// names one of them, the emit is saved there and goes on from the publication.
fn build(
    hours: Vec<facts::Hour>,
    cuts: &[i32],
    resume: Option<i32>,
    budget: &Budget,
) -> (Memory, u64) {
    let cache = Cache::new(budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(budget, identity(), first).unwrap();
    let mut emit = Emit::new();
    let mut root = 0;
    let mut generation = 0;
    let mut hours = hours.into_iter().peekable();
    for &cut in cuts {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        generation += 1;
        root = publish(
            &mut writer,
            &mut pages,
            &cache,
            Publication {
                generation,
                segment: generation - 1,
                accepted: Span::new(first, cut).unwrap(),
                reached: cut,
                refusal: None,
            },
            Blob::EMPTY,
        )
        .unwrap();
        if resume == Some(cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, &cache, budget, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::load(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    (pages, root)
}

/// Every frame of `replay`'s branch 1 with the tables a projection reads.
fn frames(replay: &Replay) -> Vec<DecodedFrame> {
    let mut frames = Vec::new();
    for block in replay.index.iter().filter(|block| block.branch == 1) {
        let mut decoder = FrameDecoder::for_tables(facts::TABLES);
        for frame in replay.read_block_selected(block, facts::TABLES).unwrap() {
            frames.push(decoder.advance(&frame).unwrap().clone());
        }
    }
    frames
}

fn source(replay: &Replay) -> Source {
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "wars-columnar".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::open(replay.bytes().to_vec()).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// The publication of `replay`'s branch 1 holds what the row walk reads for each of `views`:
/// their wars with every hour's casualties, and the groups and contributions at every hour, for
/// segments starting at each of `starts`, cut at each of `cuts`, the emit saved and resumed at
/// `resume`.
fn assert_placed(
    replay: &Replay,
    cat: Option<&Arc<Catalogue>>,
    views: &[Option<&str>],
    (starts, cuts, resume): (&[i32], &[i32], Option<i32>),
) {
    let flag = AtomicBool::new(false);
    let frames = frames(replay);
    let last = frames.last().unwrap().hour;
    let mut cuts = cuts.to_vec();
    cuts.retain(|&cut| cut < last);
    cuts.push(last);
    let budget = Budget::new(64 << 20);
    let (pages, root) = build(projected(&frames, starts), &cuts, resume, &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(pages, &cache, root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let source = source(replay);
    let fixture = Fixture::new();
    let store = &fixture.key;
    let start = replay.branches.iter().find(|start| start.id == 1).unwrap();
    let bounds = branch_bounds(replay, start);
    let hours: BTreeSet<i32> = frames.iter().map(|frame| frame.hour).collect();
    let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
    for &view in views {
        let walked = Walked {
            source: &source,
            store,
            branch: 1,
            bounds,
            view,
            selected: None,
            cat,
            cancelled: &flag,
        };
        let (actual, current) =
            walk_snapshot(&mut snapshot, &directory, &walked, &hours, &hours).unwrap();
        let expected = scan(replay, 1, view, cat.map(|cat| &**cat), &flag).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "{label}: view {view:?}"
        );
        assert_eq!(current.len(), frames.len(), "{label}: view {view:?}");
        for (current, frame) in current.iter().zip(&frames) {
            let expected = at_hour(1, frame, view, cat.map(|cat| &**cat)).unwrap();
            assert_eq!(
                serde_json::to_value(current).unwrap(),
                serde_json::to_value(expected).unwrap(),
                "{label}: view {view:?} at {}",
                frame.hour
            );
        }
    }
}

/// [`assert_placed`] for segments starting only at the walk's first hours, at every other hour,
/// and at a few hours with cuts between them and a resume.
pub(super) fn assert_native_parity(
    replay: &Replay,
    cat: Option<&Arc<Catalogue>>,
    views: &[Option<&str>],
) {
    let frames = frames(replay);
    let hours: Vec<i32> = frames.iter().map(|frame| frame.hour).collect();
    let every_other: Vec<i32> = hours.iter().copied().step_by(2).collect();
    let middle = hours[hours.len() / 2];
    let late = hours[hours.len() * 3 / 4];
    assert_placed(replay, cat, views, (&[], &[], None));
    assert_placed(replay, cat, views, (&every_other, &every_other, None));
    let placed = (&[middle, late][..], &[middle - 1, late][..], Some(middle - 1));
    assert_placed(replay, cat, views, placed);
}

#[test]
fn native_wars_preserve_viewpoints_future_capitulations_truncation_gaps_and_native_identity_changes()
 {
    let mut frames: Vec<_> = (100..=108)
        .filter(|hour| *hour != 104)
        .map(recorded)
        .collect();
    for frame in &mut frames {
        if let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut *frame.tables[TableId::WarRelations.index()]
        {
            for row in rows {
                row.first_casualties -= i64::from(frame.hour);
                row.second_casualties = i64::from(frame.hour) * 7;
            }
        }
        frame.tables[TableId::Capitulation.index()] =
            TableView::Complete(TableRows::Capitulation(vec![rows::CapitulationRow {
                country: 2,
                capitulated: 1,
                capitulated_hour: 102,
                ..Default::default()
            }]))
            .into();
        if frame.hour == 103 {
            let rows = frame
                .complete::<tables::WarOverviewCountries>()
                .unwrap()
                .to_vec();
            frame.tables[TableId::WarOverviewCountries.index()] = TableView::Truncated {
                rows: TableRows::WarOverviewCountries(rows),
                missing: 1,
            }
            .into();
        }
        if frame.hour >= 106
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"mod-rename".to_vec();
        }
        if frame.hour == 108
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].id = 99;
            rows.sort_by_key(|row| row.id);
        }
    }
    let replay = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(1, frames)],
    )
    .unwrap();
    assert_native_parity(
        &replay,
        None,
        &[None, Some("ONE"), Some("TWO"), Some("mod-rename")],
    );
}

/// Casualties read at every hour match the row walk where four overlapping groups hold the same
/// relations, one relation leaves its wars and returns under the same lifetime, and the read is
/// from either side.
#[test]
fn native_wars_read_overlapping_groups_and_a_relation_leaving_and_rejoining() {
    let mut frames: Vec<_> = super::tests::overlapping_frames(12, false).collect();
    for frame in &mut frames[4..8] {
        // C1 and C2 are at peace for four hours, then the same lifetime holds again.
        let lapsed = |first: u32, second: u32| (first, second) == (1, 2);
        if let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut *frame.tables[TableId::WarRelations.index()]
        {
            rows.retain(|row| !lapsed(row.first_country, row.second_country));
        }
        if let TableView::Complete(TableRows::WarOwnedRelations(rows)) =
            &mut *frame.tables[TableId::WarOwnedRelations.index()]
        {
            rows.retain(|row| !lapsed(row.first_country, row.second_country));
            // Each owner's relations stay numbered from zero.
            let mut next = HashMap::<u32, u32>::new();
            for row in rows {
                let ordinal = next.entry(row.owner).or_default();
                row.ordinal = *ordinal;
                *ordinal += 1;
            }
        }
    }
    let replay = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(1, frames)],
    )
    .unwrap();
    assert_native_parity(&replay, None, &[None, Some("C1"), Some("C2"), Some("C6")]);
}

/// Hours whose casualties alone move publish no change input, and a segment's first hour that
/// reads every table again publishes only what changed since the input before.
#[test]
fn counter_only_hours_and_segment_starts_publish_only_changed_inputs() {
    let frames: Vec<_> = (100..=111)
        .map(|hour| {
            let mut frame = recorded(100);
            frame.hour = hour;
            if let TableView::Complete(TableRows::WarRelations(rows)) =
                &mut *frame.tables[TableId::WarRelations.index()]
            {
                for row in rows {
                    row.first_casualties = i64::from(hour) * 3;
                }
            }
            frame
        })
        .collect();
    let budget = Budget::new(64 << 20);
    for starts in [&[][..], &[104, 108][..]] {
        let (pages, root) = build(projected(&frames, starts), &[111], None, &budget);
        let cache = Cache::new(&budget);
        let mut snapshot = Snapshot::open(pages, &cache, root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        let input = directory
            .key(super::emit::FAMILY, super::emit::INPUT, None)
            .unwrap();
        assert_eq!(
            snapshot.event_count_through(input, 111, None).unwrap(),
            1,
            "starts {starts:?}: one change input"
        );
    }
}

/// Of 1,500 countries at war, an hour moving one relation's casualties hands the emit that
/// relation's counters alone, and an hour changing nothing hands it nothing.
#[test]
fn an_hour_hands_the_emit_only_the_counters_that_moved() {
    let frames: Vec<_> = (100..=103)
        .map(|hour| {
            let mut frame = super::test_fixtures::scaled_recorded(100, 1_500);
            frame.hour = hour;
            if hour == 102
                && let TableView::Complete(TableRows::WarRelations(rows)) =
                    &mut *frame.tables[TableId::WarRelations.index()]
            {
                rows[0].first_casualties += 1;
            }
            frame
        })
        .collect();
    let hours = projected(&frames, &[]);
    assert!(hours[0].facts.as_ref().is_some_and(|facts| facts.whole.is_some()));
    match hours[2].facts.as_ref().and_then(|facts| facts.relations.as_ref()) {
        Some(facts::Relations::Values(values)) => assert_eq!(values.len(), 1),
        _ => panic!("the moved counters alone"),
    }
    assert!(
        hours[1]
            .facts
            .as_ref()
            .is_none_or(|facts| facts.relations.is_none() && facts.changed == 0),
        "an hour changing nothing hands nothing"
    );
}

/// An hour moving only the war overview's scores publishes them without joining the relations
/// again, and the replies still match the row walk.
#[test]
fn overview_scores_alone_publish_without_joining_the_relations_again() {
    let frames: Vec<_> = (100..=104)
        .map(|hour| {
            let mut frame = super::test_fixtures::scaled_recorded(hour, 40);
            if let TableView::Complete(TableRows::WarOverviewCountries(rows)) =
                &mut *frame.tables[TableId::WarOverviewCountries.index()]
            {
                rows[0].divisions += hour - 100;
                rows[1].factories += 2 * (hour - 100);
            }
            frame
        })
        .collect();
    let hours = projected(&frames, &[]);
    let place = super::inputs::input(TableId::WarOverviewCountries).unwrap();
    for hour in &hours[1..] {
        let facts = hour.facts.as_ref().expect("the scores moved");
        assert_eq!(facts.changed, 1 << place, "at {}", hour.hour);
        assert!(facts.packet.is_some(), "at {}", hour.hour);
        assert!(
            !matches!(facts.relations, Some(facts::Relations::Whole(_))),
            "at {}: joined again",
            hour.hour
        );
    }
    let replay = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(1, frames)],
    )
    .unwrap();
    assert_native_parity(&replay, None, &[None]);
}
