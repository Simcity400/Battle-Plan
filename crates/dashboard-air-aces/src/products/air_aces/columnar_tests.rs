//! Air aces' columnar preparation held to the row scan on the same hours, written as the native
//! recorder writes them: the manifest, every window and every portrait, across segment starts, a
//! gap, flushes and resumes.

use super::{
    emit::Emit,
    facts::{Hour, Project, Roster, TABLES},
    prepared::Prepared,
    tests::{ace, frame as row_frame},
    *,
};
use dashboard_base::prepared::series::{self, tests::Fixture};
use dashboard_base::products::equipment::VariantHour;
use dashboard_core::test_fixtures::{CAMPAIGN, continuation_records, records};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator as _},
};
use replay_data::{
    format::FormatVersion,
    rows::{AceDeath, Recorded, RowCodec, RowsVisit, TableRows},
    scope::CaptureMode,
};
use replay_frame::frame::TableView;

const BRANCH: u64 = 1;
const ROSTER_REFUSAL: &str = "synthetic unavailable roster";
/// The recording's blocks: four windows, a gap of two hours inside the second's end.
const BLOCKS: [(i32, i32); 4] = [(100, 111), (112, 117), (120, 133), (134, 147)];

fn hours() -> Vec<i32> {
    BLOCKS
        .iter()
        .flat_map(|&(first, last)| first..=last)
        .collect()
}

/// Whether the roster and the variants are read at `hour`.
fn read(hour: i32) -> bool {
    hour % 3 == 0
}

/// A recording over [`BLOCKS`]: each hour's row frame, and the hours the roster and the variant
/// catalogue are read.
struct Scenario {
    frame: fn(i32) -> DecodedFrame,
    roster: fn(i32) -> bool,
    catalogue: fn(i32) -> bool,
}

const SCENARIO: Scenario = Scenario {
    frame,
    roster: read,
    catalogue: read,
};

/// The hour as the row recording holds it. Ace 1 flies wing 70 and dies at 130, killed by ace 2,
/// which was first seen dead; ace 3 joins at 108; ace 1 is renamed from 121. The wing is renamed
/// at 112, the design at 126, the owner's tag at 136, and the roster read fails at 141. The
/// wing's amounts and the country's political power move every hour, which nothing reads.
fn frame(hour: i32) -> DecodedFrame {
    let mut first = ace(1, hour < 130);
    first.assigned_wing = 70;
    if hour >= 130 {
        first.death = Some(AceDeath {
            killer_ace: 2,
            killer_country: 11,
        });
    }
    if hour >= 121 {
        first.name = b"Renamed".to_vec();
    }
    let mut aces = vec![first, ace(2, false)];
    if hour >= 108 {
        aces.push(ace(3, true));
    }
    let mut frame = row_frame(hour, aces);
    if hour == 141 {
        *frame.tables[TableId::AirAces.index()] = TableView::Unavailable {
            reason: ROSTER_REFUSAL.into(),
        };
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(i64::from(hour));
        if hour >= 136 {
            rows[0].tag = b"GEX".to_vec();
        }
    }
    if hour >= 112
        && let TableView::Complete(TableRows::AirWings(rows)) =
            &mut *frame.tables[TableId::AirWings.index()]
    {
        rows[0].name = Recorded::Value(Some(b"Renamed wing".to_vec()));
    }
    if let TableView::Complete(TableRows::WingEquipment(rows)) =
        &mut *frame.tables[TableId::WingEquipment.index()]
    {
        rows[0].amount_raw = i64::from(hour);
    }
    if hour >= 126
        && let TableView::Complete(TableRows::VariantDefinitions(rows)) =
            &mut *frame.tables[TableId::VariantDefinitions.index()]
    {
        rows[0].display_name = b"Renamed design".to_vec();
    }
    frame
}

/// One table's rows written into a walk.
struct Write<'w> {
    walk: &'w mut Synthetic,
    table: TableId,
    read: bool,
}

impl RowsVisit for Write<'_> {
    type Output = io::Result<()>;

    fn rows<R: RowCodec>(self, rows: &[R]) -> io::Result<()> {
        self.walk.table(
            self.table,
            Input::Snapshot {
                rows,
                partial: None,
                read: self.read,
                sample: Sample::FromReadBit,
            },
        )
    }
}

/// Writes `frame`'s tables into `walk`, the roster and the variants timed only at their reads.
fn write(walk: &mut Synthetic, frame: &DecodedFrame, scenario: &Scenario) {
    for &table in TABLES {
        let read = match table {
            TableId::AirAces => (scenario.roster)(frame.hour),
            TableId::VariantDefinitions | TableId::VariantModules | TableId::VariantUpgrades => {
                (scenario.catalogue)(frame.hour)
            }
            _ => true,
        };
        match &*frame.tables[table.index()] {
            TableView::Complete(rows) => rows
                .visit(Write {
                    walk: &mut *walk,
                    table,
                    read,
                })
                .unwrap(),
            TableView::Unavailable { reason } => walk
                .table::<replay_data::rows::AirAceRow>(
                    table,
                    Input::Unavailable {
                        reason,
                        read,
                        sample: Sample::FromReadBit,
                    },
                )
                .unwrap(),
            _ => panic!("fixture tables are complete or unavailable"),
        }
    }
}

/// The hours the segments' operators hand on, a fresh operator starting at each of `starts`
/// and at each walk's first hour, and the row frames of the same hours.
fn projected(scenario: &Scenario, starts: &[i32]) -> (Vec<Hour>, Vec<DecodedFrame>) {
    let format = FormatVersion::CURRENT;
    let needs = super::facts::needs().unwrap();
    let mut out = Vec::new();
    let mut frames: Vec<DecodedFrame> = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut operator = Project::new(format).unwrap();
    for (ordinal, hour) in (0..).zip(hours()) {
        let gap = frames.last().is_some_and(|last| last.hour + 1 != hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(TABLES, BRANCH, hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        let mut observed = (scenario.frame)(hour);
        write(walk, &observed, scenario);
        let cold = starts.contains(&hour);
        let metrics = walk
            .end(|state, manifest, decoded, first| {
                let starts = first || cold;
                if starts {
                    operator = Project::new(format).unwrap();
                }
                let world = dashboard_world::World::at(
                    &mut world,
                    starts,
                    &needs,
                    (format, CaptureMode::Full),
                    state,
                    manifest,
                )
                .unwrap();
                operator
                    .hour(
                        &segments::Hour {
                            branch: BRANCH,
                            hour: decoded.hour,
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
                decoded.metrics.clone()
            })
            .unwrap();
        observed.metrics = metrics;
        frames.push(observed);
    }
    // As its emit is handed them: written to the spool and read back.
    (replay_spool::respool(out).unwrap(), frames)
}

/// The row recording of `frames`, a block per [`BLOCKS`] entry.
fn recorded(frames: Vec<DecodedFrame>) -> Source {
    let mut groups = Vec::new();
    for &(first, last) in &BLOCKS {
        let block: Vec<_> = frames
            .iter()
            .filter(|frame| (first..=last).contains(&frame.hour))
            .cloned()
            .collect();
        let group = if groups.is_empty() {
            records(BRANCH, block)
        } else {
            let prefix = Replay::from_records(CAMPAIGN.into(), &groups).unwrap();
            continuation_records(&prefix, BRANCH, block)
        };
        groups.push(group);
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &groups).unwrap();
    assert_eq!(replay.index.len(), BLOCKS.len());
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "aces-columnar".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// The pages the emit publishes from `hours`, flushed and committed after each of `cuts`' hours,
/// the last ending the branch; after each of `resumes`, the emit is saved and loaded again, as a
/// preparation of a grown recording goes on at the next segment's first hour.
fn build(hours: Vec<Hour>, source: &Source, cuts: &[i32], resumes: &[i32]) -> Fixture {
    let fixture = Fixture::new();
    let replay = source.replay_ref();
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let end = *cuts.last().unwrap();
    let mut emit = Emit::new(replay, BRANCH);
    let mut hours = hours.into_iter().peekable();
    for &cut in cuts {
        let mut writer = fixture.writer();
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.observe(&mut writer, source, replay, &context, hour, &flag)
                .unwrap();
        }
        let through = (cut == end).then_some(cut);
        emit.flush(&mut writer, source, replay, &context, through, &flag)
            .unwrap();
        writer.commit().unwrap();
        if resumes.contains(&cut) {
            let saved = emit.save().unwrap();
            emit = Emit::load(replay, BRANCH, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none(), "every hour is flushed");
    fixture
}

/// Holds the published pages to the row scan's replies over the same recording.
fn check(fixture: &Fixture, source: &Source, label: &str) {
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let params = AirAcesParams {
        branch_id: BRANCH.to_string(),
        view: AirAcesView::Manifest,
        window: None,
    };
    let expected = AirAces::load(source, &params, &context, &flag).unwrap();
    let mut state = Prepared::read(&fixture.store, &fixture.key, None, BRANCH).unwrap();
    let actual = state
        .manifest(
            source,
            &fixture.store,
            &fixture.key,
            &params,
            &context,
            &flag,
        )
        .unwrap();
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap(),
        "{label}: manifest"
    );
    for window in 0..BLOCKS.len() {
        let params = AirAcesParams {
            branch_id: BRANCH.to_string(),
            view: AirAcesView::Window,
            window: Some(window),
        };
        let expected = AirAces::load(source, &params, &context, &flag).unwrap();
        let actual = series::read_reply::<AirAcesWindowReply>(
            &fixture.store,
            &fixture.key,
            "air-aces-1-windows",
            window as u64,
        )
        .unwrap()
        .unwrap_or_else(|| panic!("{label}: window {window} is published"));
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "{label}: window {window}"
        );
    }
    let state = Prepared::read(&fixture.store, &fixture.key, None, BRANCH).unwrap();
    let scan = cached(source, BRANCH, &flag).unwrap();
    for hour in BLOCKS[0].0 - 1..=BLOCKS[3].1 + 1 {
        for id in [1, 2, 3, 999] {
            assert_eq!(
                state.portrait_at(id, hour),
                scan.at(id, hour).map(|run| PortraitOperands::from(&run.facts)),
                "{label}: ace {id}'s portrait at {hour}"
            );
        }
        let params = AirAcePortraitParams {
            branch_id: BRANCH.to_string(),
            hour,
            ids: vec!["1".into(), "2".into(), "3".into(), "999".into()],
        };
        let expected = AirAcePortraits::load(source, &params, &context, &flag).unwrap();
        let actual = portrait_reply(source, &params, &context, &flag, |id, hour| {
            state.portrait_at(id, hour)
        })
        .unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "{label}: portraits at {hour}"
        );
    }
}

#[test]
fn the_pages_answer_as_the_row_scan_across_segments_flushes_and_resumes() {
    let all = hours();
    for (starts, cuts, resumes) in [
        (&[][..], &[147][..], &[][..]),
        (&[106, 126][..], &[105, 117, 125, 147][..], &[125][..]),
        (
            &[103, 112, 134, 140][..],
            &[102, 111, 117, 133, 139, 147][..],
            &[102, 111, 117, 139][..],
        ),
        (&all[1..], &[110, 121, 130, 147][..], &[110, 121, 130][..]),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resumes {resumes:?}");
        let (projected, frames) = projected(&SCENARIO, starts);
        let source = recorded(frames);
        let fixture = build(projected, &source, cuts, resumes);
        check(&fixture, &source, &label);
    }
}

/// The scenario reads as the row scan finds it: ace 1 dead by its read at 132 after its last
/// alive read at 129, ace 3 first seen at 108, and the failed read at 141 a gap.
#[test]
fn the_scenario_reads_as_the_row_scan_finds_it() {
    let (_, frames) = projected(&SCENARIO, &[]);
    let source = recorded(frames);
    let scan = cached(&source, BRANCH, &AtomicBool::new(false)).unwrap();
    let first = &scan.aces[scan.indices[&1]];
    assert_eq!((first.last_alive, first.dead_by), (Some(129), Some(132)));
    assert_eq!(scan.aces[scan.indices[&3]].first_seen, 108);
    assert!(!scan.reads.contains(&141));
    assert!(scan.at(1, 141).is_none());
}

/// Hours where only what the roster does not read moves (the wing's amounts, the country's
/// power) hand the emit no ace and no design; a read after them gives no ace again, and the read
/// after the wing's rename gives its one ace alone.
#[test]
fn hours_where_nothing_the_roster_reads_moves_hand_on_nothing() {
    let (out, _) = projected(&SCENARIO, &[]);
    let held = |hour: i32| out.iter().find(|held| held.hour == hour).unwrap();
    for hour in [101, 104, 110, 113, 122] {
        assert!(
            matches!(held(hour).roster, Roster::Unread | Roster::Same),
            "{hour}"
        );
        assert!(held(hour).built.is_empty(), "{hour}");
    }
    assert!(matches!(held(105).roster, Roster::Same));
    let Roster::Read(read) = &held(114).roster else {
        panic!("the read after the wing's rename gives what changed")
    };
    assert!(!read.whole);
    assert_eq!(
        read.seen.iter().map(|seen| seen.row.id).collect::<Vec<_>>(),
        [1]
    );
    assert!(read.gone.is_empty());
}

/// The scenario with ace 3 retired from the roster at 124, inside the third window.
fn retired(hour: i32) -> DecodedFrame {
    let mut frame = frame(hour);
    if hour >= 124
        && let TableView::Complete(TableRows::AirAces(rows)) =
            &mut *frame.tables[TableId::AirAces.index()]
    {
        rows.retain(|row| row.id != 3);
    }
    frame
}

/// A resume inside a window not yet published keeps every run it holds: ace 1's runs before
/// and after its rename and its death, an ace gone from the roster, and the runs closed by the
/// roster's failed read.
#[test]
fn a_resume_inside_an_unpublished_window_keeps_every_run_it_holds() {
    let retire = Scenario {
        frame: retired,
        ..SCENARIO
    };
    for (scenario, name, starts, cuts, resumes) in [
        (
            &SCENARIO,
            "several runs of one ace",
            &[126, 133][..],
            &[125, 132, 147][..],
            &[125, 132][..],
        ),
        (
            &retire,
            "a retired ace",
            &[127, 131][..],
            &[126, 130, 147][..],
            &[126, 130][..],
        ),
        (
            &SCENARIO,
            "an unavailable roster",
            &[142, 144][..],
            &[141, 143, 147][..],
            &[141, 143][..],
        ),
    ] {
        let label = format!("{name}: starts {starts:?}, cuts {cuts:?}, resumes {resumes:?}");
        let (projected, frames) = projected(scenario, starts);
        let source = recorded(frames);
        let fixture = build(projected, &source, cuts, resumes);
        check(&fixture, &source, &label);
    }
}

/// The scenario with its design named by its origin, the ace owner, whose tag changes at 141
/// alone; the catalogue is timed every fourth hour, not at its untimed slot read at 142.
fn renamed_origin(hour: i32) -> DecodedFrame {
    let mut frame = frame(hour);
    // The roster stays read through 141 here, so 142 changes nothing it reads.
    if hour == 141 {
        frame.tables[TableId::AirAces.index()] =
            self::frame(142).tables[TableId::AirAces.index()].clone();
    }
    if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
        &mut *frame.tables[TableId::VariantDefinitions.index()]
    {
        rows[0].origin_link = 11;
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].tag = (if hour >= 141 { b"GEX" } else { b"GER" }).to_vec();
    }
    frame
}

fn catalogue_read(hour: i32) -> bool {
    hour % 4 == 0
}

/// The catalogue's slot read with no timing of its own names the design anew, as the row scan
/// does, though nothing the catalogue reads changed at that hour.
#[test]
fn an_untimed_catalogue_slot_read_names_what_changed_since_its_last_read() {
    let scenario = Scenario {
        frame: renamed_origin,
        roster: read,
        catalogue: catalogue_read,
    };
    let (out, frames) = projected(&scenario, &[]);
    let slot = frames.iter().find(|frame| frame.hour == 142).unwrap();
    assert_eq!(
        slot.metrics.table_ns[TableId::VariantDefinitions.index()],
        0,
        "the slot read at 142 is untimed"
    );
    let held = out
        .iter()
        .find(|held| held.hour == 142)
        .expect("the catalogue's slot read is projected");
    assert_eq!(held.variants, VariantHour::Read);
    assert!(held.built.iter().any(|(id, _)| *id == 80));
    let source = recorded(frames);
    let scan = cached(&source, BRANCH, &AtomicBool::new(false)).unwrap();
    assert!(
        scan.variants.versions[&80]
            .iter()
            .any(|version| version.first_hour == 142),
        "the row scan names the design anew at 142"
    );
    for (starts, cuts, resumes) in [
        (&[][..], &[147][..], &[][..]),
        (&[139][..], &[138, 147][..], &[138][..]),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resumes {resumes:?}");
        let (projected, frames) = projected(&scenario, starts);
        let source = recorded(frames);
        let fixture = build(projected, &source, cuts, resumes);
        check(&fixture, &source, &label);
    }
}

/// Each ace's run at `hour` in `reply`, its texts spelled and without its bounds or the dates a
/// single roster read cannot know.
fn roster_values(reply: &AirAcesWindowReply, hour: i32) -> serde_json::Value {
    let reply = serde_json::to_value(reply).unwrap();
    let texts = reply["texts"].as_array().unwrap();
    let rows: Vec<_> = reply["aces"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|ace| {
            let run = ace["runs"].as_array().unwrap().iter().find(|run| {
                run["firstHour"].as_i64().unwrap() <= i64::from(hour)
                    && run["lastHour"].as_i64().unwrap() >= i64::from(hour)
            })?;
            let mut value = run.clone();
            for field in ["name", "callsign", "wingName"] {
                value[field] = value[field]
                    .as_u64()
                    .map_or(serde_json::Value::Null, |index| {
                        texts[index as usize].clone()
                    });
            }
            for field in ["firstHour", "lastHour"] {
                value.as_object_mut().unwrap().remove(field);
            }
            if value["death"].is_object() {
                value["death"]["lastAlive"] = serde_json::Value::Null;
                value["death"]["byHour"] = serde_json::Value::Null;
            }
            Some(serde_json::json!({"id": ace["id"], "value": value}))
        })
        .collect();
    serde_json::json!(rows)
}

/// A cold roster, sampled from the recording at its latest read, holds the values the native
/// pages publish at that read, and names its variants as they do.
#[test]
fn a_cold_roster_holds_the_values_the_native_pages_publish_at_its_read() {
    let (projected, frames) = projected(&SCENARIO, &[]);
    let source = recorded(frames);
    let fixture = build(projected, &source, &[147], &[]);
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let reads = cached(&source, BRANCH, &flag).unwrap().reads.clone();
    let manifest = Prepared::read(&fixture.store, &fixture.key, None, BRANCH)
        .unwrap()
        .manifest(
            &source,
            &fixture.store,
            &fixture.key,
            &AirAcesParams {
                branch_id: BRANCH.to_string(),
                view: AirAcesView::Manifest,
                window: None,
            },
            &context,
            &flag,
        )
        .unwrap();
    let AirAcesReply::Manifest(manifest) = manifest else {
        panic!("a manifest")
    };
    let named = |variants: &BTreeMap<String, Vec<StockVariantName>>, read: i32| {
        let mut name = serde_json::to_value(
            variants["80"]
                .iter()
                .rev()
                .find(|name| name.first_hour <= read)
                .expect("the supplying variant name"),
        )
        .unwrap();
        name.as_object_mut().unwrap().remove("firstHour");
        name
    };
    for hour in [125, 131, 140, 146] {
        let (scan, read) = prepared::sample_roster(&source, BRANCH, hour, &flag).unwrap();
        let read = read.expect("a supplying read");
        assert_eq!(
            Some(read),
            reads.iter().rev().copied().find(|&at| at <= hour),
            "{hour}: the latest read"
        );
        let sampled = AirAcesParams {
            branch_id: BRANCH.to_string(),
            view: AirAcesView::Window,
            window: Some(0),
        };
        let cold = window_reply(&scan, &sampled, None, &flag).unwrap();
        assert!(!cold.aces.is_empty(), "{hour}: the cold roster");
        let window = BLOCKS
            .iter()
            .position(|&(first, last)| (first..=last).contains(&read))
            .unwrap();
        let exact = series::read_reply::<AirAcesWindowReply>(
            &fixture.store,
            &fixture.key,
            "air-aces-1-windows",
            window as u64,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            roster_values(&cold, read),
            roster_values(&exact, read),
            "{hour}: the roster's values"
        );
        let cold = super::manifest(&scan, &sampled, None, &flag).unwrap();
        assert_eq!(
            named(&cold.variants, read),
            named(&manifest.variants, read),
            "{hour}: the variant's name"
        );
    }
}

/// A kept manifest takes the content's warnings as each request finds them.
#[test]
fn kept_manifests_take_each_requests_content_warnings() {
    let (projected, frames) = projected(&SCENARIO, &[]);
    let source = recorded(frames);
    let fixture = build(projected, &source, &[147], &[]);
    let (cat, _folder) = catalogue::tests::folder_catalogue(&[]);
    let cat: io::Result<Arc<Catalogue>> = Ok(Arc::new(cat));
    let params = AirAcesParams {
        branch_id: BRANCH.to_string(),
        view: AirAcesView::Manifest,
        window: None,
    };
    let flag = AtomicBool::new(false);
    let read = || {
        serde_json::to_value(
            prepared::kept_manifest(&source, &fixture.store, &fixture.key, &params, &cat, &flag)
                .unwrap(),
        )
        .unwrap()
    };
    let refused = |reply: &serde_json::Value| {
        reply["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap().contains("synthetic refusal"))
    };
    let before = read();
    assert!(!refused(&before));
    // Another product's refusal joins the shared catalogue between two reads.
    cat.as_ref().unwrap().refuse("synthetic refusal", "another text");
    let after = read();
    assert!(refused(&after));
    let mut fresh = Prepared::read(&fixture.store, &fixture.key, None, BRANCH).unwrap();
    let expected = prepared::with_warnings(
        fresh
            .named_manifest(
                &source,
                &fixture.store,
                &fixture.key,
                &params,
                cat.as_deref().ok(),
                &flag,
            )
            .unwrap(),
        cat.as_deref(),
    )
    .unwrap();
    assert_eq!(after, serde_json::to_value(expected).unwrap());
    assert_eq!(
        dashboard_base::prepared::query::observations::computations(&source, "air-aces:manifest"),
        1
    );
}
