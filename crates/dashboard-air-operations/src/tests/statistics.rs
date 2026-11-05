//! Air statistics published as the native driver publishes them: each frame's hour observed
//! whole by a segment's reader (the row path, as the walk observed every hour), or by segments'
//! operators over a synthetic walk; cut, resumed and read back as a reply reads them.

use super::*;
use crate::products::air_statistics::{
    emit::{self, Emit},
    segment::{FIELDS, SLOT, Segments, TABLES, WRAPPED},
};
use dashboard_base::prepared::{frame_changes::FrameChanges, published::Opened};
use dashboard_keyed::{
    native::testing::{operate_segmented, resuming},
    query::KeyedWindow,
    segment::{self as keyed, Hour, Segmented, Touched},
};
use dashboard_series::{
    Blob, Budget, Cache, Identity, PageRead, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};

static BUDGET: Budget = Budget::new(256 << 20);
const SECTION: &str = "air-statistics-test";

fn identity() -> Identity {
    Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: emit::SCHEMA,
    }
}

/// Each frame's hour as a segment's reader hands it on, every table restated, a segment
/// starting at each of `starts` (frame indices).
pub(super) fn observed(frames: &[DecodedFrame], starts: &[usize]) -> Vec<Hour<Segments>> {
    let format = FormatVersion::CURRENT;
    let mut touched = Touched::default();
    for &table in TABLES {
        touched.of_mut(table).restated = true;
    }
    let mut reader = Segments::reader(format);
    frames
        .iter()
        .enumerate()
        .map(|(at, frame)| {
            if at > 0 && starts.contains(&at) {
                reader = Segments::reader(format);
            }
            let changes = FrameChanges::alone(frame, format);
            let dated = changes.slot_read[TableId::AirStatistics.index()];
            let (facts, bytes) = reader.read(frame, &changes, &touched).unwrap();
            Hour::new(frame.hour, dated, Some(facts), bytes)
        })
        .collect()
}

/// A publication of branch 1's hours.
pub(super) struct Published {
    pages: Memory,
    root: u64,
}

/// `hours` published, cut after each of `cuts`' hours, the last of which is the last hour;
/// where `resume` names one of them, the emit is saved there and goes on from the publication.
pub(super) fn publish_hours(
    hours: Vec<Hour<Segments>>,
    cuts: &[i32],
    resume: Option<i32>,
) -> Published {
    let cache = Cache::new(&BUDGET);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(&BUDGET, identity(), first).unwrap();
    let mut emit = Emit::new();
    let mut generation = 0;
    let mut root = 0;
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
            writer = Writer::resume(&mut pages, &cache, &BUDGET, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
            emit = Emit::load(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none(), "the last cut is the last hour");
    Published { pages, root }
}

/// `frames` by the row path, cut after the last, as one segment.
pub(super) fn published(frames: &[DecodedFrame]) -> Stored {
    let last = frames.last().unwrap().hour;
    publish_hours(observed(frames, &[]), &[last], None).store()
}

pub(super) struct Stored {
    pub(super) fixture: Fixture,
    root: u64,
}

impl Published {
    pub(super) fn store(mut self) -> Stored {
        let fixture = Fixture::new();
        let mut writer = fixture.writer();
        for id in 0..=self.root {
            let mut bytes = vec![0; self.pages.len(id).unwrap() as usize];
            self.pages.read(id, 0, &mut bytes).unwrap();
            assert_eq!(writer.write(SECTION, &bytes[..]).unwrap(), id);
        }
        writer.commit().unwrap();
        Stored {
            fixture,
            root: self.root,
        }
    }
}

impl Stored {
    pub(super) fn open(&self) -> Opened {
        let snapshot = self
            .fixture
            .store
            .session(&self.fixture.key)
            .unwrap()
            .unwrap()
            .snapshot();
        Opened::from_snapshot(
            snapshot,
            SECTION.into(),
            self.root,
            identity(),
            emit::SCHEMA,
            None,
        )
        .unwrap()
        .unwrap()
    }

    /// The refused observations, and the window `from..=to` of every unit's every field.
    pub(super) fn window(&self, from: i32, to: i32) -> (BTreeMap<String, u64>, KeyedWindow) {
        let opened = self.open();
        let cancelled = AtomicBool::new(false);
        let (refusals, own, keyed) = opened
            .read(&cancelled, |_, directory| {
                let refusals: BTreeMap<String, u64> = directory
                    .scalar(emit::REFUSALS)
                    .and_then(|scalar| scalar.text())
                    .map_or_else(BTreeMap::new, |json| serde_json::from_slice(json).unwrap());
                let own = [
                    directory.key(emit::FAMILY, emit::KNOWN, None),
                    directory.key(emit::FAMILY, emit::READ, None),
                ];
                let mut keyed = Vec::new();
                for (actor, unit) in directory.actors.iter().enumerate() {
                    let unit = std::str::from_utf8(&unit.name).unwrap();
                    for field in 0..FIELDS {
                        let spelled = match field {
                            SLOT => "slot".to_owned(),
                            WRAPPED => "wrapped".to_owned(),
                            channel => format!("c{channel}"),
                        };
                        let key = directory.key(actor as u64, field as u64, None);
                        keyed.push((format!("{unit}|{spelled}"), key));
                    }
                }
                Ok((refusals, own, keyed))
            })
            .unwrap();
        let window = dashboard_keyed::query::published_window(
            &opened,
            own,
            keyed,
            (from, to),
            None,
            &cancelled,
        )
        .unwrap();
        (refusals, window)
    }
}

/// A reply of `params` from `stored`'s publication, or before any; the gate's prefix through
/// `through`.
pub(super) fn reply(
    stored: Option<&Stored>,
    frames: Vec<DecodedFrame>,
    params: Value,
    through: Option<i32>,
) -> Value {
    let empty = Fixture::new();
    let fixture = stored.map_or(&empty, |stored| &stored.fixture);
    replied(fixture, frames, params, through, |query| {
        let opened = stored.map(Stored::open);
        air_statistics::answer(query, opened.as_ref())
    })
}

#[test]
fn statistics_all_24_signed_channels_ring_state_and_both_divisors() {
    let channels: [i64; 24] = std::array::from_fn(|channel| {
        if channel == 0 {
            i64::MIN
        } else if channel == 23 {
            i64::MAX
        } else {
            -(channel as i64)
        }
    });
    let mut row = stat(3, channels);
    row.year_slot = 7;
    row.year_window_wrapped = 1;
    let frames = vec![statistics(100, vec![row])];
    let stored = published(&frames);
    let (_, window) = stored.window(100, 100);
    assert_eq!(window.series.len(), 26);
    let value = |name: &str| {
        let series = window.series.iter().find(|series| series.name == name);
        series.unwrap().runs[0].2.clone()
    };
    for (channel, raw) in channels.into_iter().enumerate() {
        assert_eq!(value(&format!("GER@7|3|c{channel}")), raw.to_string());
    }
    assert_eq!(value("GER@7|3|slot"), "7");
    assert_eq!(value("GER@7|3|wrapped"), "1");
    for channel in 0..12u8 {
        let reply = reply(
            Some(&stored),
            frames.clone(),
            json!({"branchId":"1","tag":"GER","channel":channel}),
            None,
        );
        let ours = format!("GER@7|3|c{channel}");
        let enemy = format!("GER@7|3|c{}", channel + 12);
        let names: Vec<&str> = reply["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .map(|series| series["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, [ours.as_str(), enemy.as_str(), "GER@7|3|slot", "GER@7|3|wrapped"]);
    }
    assert_eq!(air_statistics::divisor(0), 1000);
    for channel in 1..12 {
        assert_eq!(air_statistics::divisor(channel), 100_000);
    }
}

#[test]
fn statistics_absent_is_not_zero_and_wrap_is_not_subtracted() {
    let mut wrapped = stat(3, [0; 24]);
    wrapped.year_slot = 1;
    wrapped.year_window_wrapped = 1;
    let mut unknown = statistics(103, vec![]);
    *unknown.tables[TableId::AirStatistics.index()] = TableView::Unavailable {
        reason: "synthetic missing statistics".into(),
    };
    let frames = vec![
        statistics(100, vec![stat(3, [0; 24])]),
        statistics(101, vec![]),
        statistics(102, vec![wrapped]),
        unknown,
    ];
    let stored = published(&frames);
    let reply = reply(
        Some(&stored),
        frames,
        json!({"branchId":"1","tag":"GER","channel":2}),
        None,
    );
    assert_eq!(
        runs(&reply, "GER@7|3|c2"),
        &json!([[100, 100, "0", true], [102, 102, "0", true]])
    );
    assert_eq!(
        runs(&reply, "GER@7|3|wrapped"),
        &json!([[100, 100, "0", true], [102, 102, "1", true]])
    );
    assert_eq!(reply["window"]["series"].as_array().unwrap().len(), 4);
    assert_eq!(
        reply["window"]["known"],
        json!([{"firstHour":100,"lastHour":102}])
    );
}

#[test]
fn queries_select_one_country_and_pair_and_clip_accepted_prefix() {
    let frames = vec![
        statistics(
            100,
            vec![
                stat(3, [10; 24]),
                AirStatisticsRow {
                    country: 21,
                    ..stat(4, [90; 24])
                },
            ],
        ),
        statistics(101, vec![stat(3, [20; 24])]),
    ];
    let stored = published(&frames);
    let reply = reply(
        Some(&stored),
        frames,
        json!({"branchId":"1","tag":"GER@7","channel":10}),
        Some(100),
    );
    assert_eq!(
        runs(&reply, "GER@7|3|c10"),
        &json!([[100, 100, "10", true]])
    );
    assert_eq!(reply["window"]["series"].as_array().unwrap().len(), 4);
    assert_eq!(reply["regions"].as_array().unwrap().len(), 1);
    assert!(
        reply["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap().contains("Game folders"))
    );
    assert!(
        reply["channels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["ours"] == "Channel name unavailable")
    );
}

#[test]
fn cold_prefix_has_empty_window_not_zero_and_keeps_channel_choices() {
    let frames = vec![
        statistics(100, vec![stat(3, [0; 24])]),
        statistics(101, vec![]),
    ];
    let reply = reply(
        None,
        frames,
        json!({"branchId":"1","tag":"GER","channel":3}),
        Some(99),
    );
    assert_eq!(reply["window"]["known"], json!([]));
    assert_eq!(reply["window"]["series"], json!([]));
    assert_eq!(reply["channels"].as_array().unwrap().len(), 12);
}

#[test]
fn cancellation_is_respected_before_any_content_or_page_read() {
    let source = source(vec![statistics(100, vec![stat(3, [0; 24])])]);
    let fixture = Fixture::new();
    let params = json!({"branchId":"1","tag":"GER","channel":3});
    let prefix = Prefix::new(
        &source,
        1,
        Coverage {
            schema: 1,
            first_hour: 100,
            last_hour: 100,
            complete: true,
        },
        None,
    );
    let cancelled = AtomicBool::new(true);
    let query = Query {
        source: &source,
        params: &params,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &cancelled,
    };
    assert_eq!(
        air_statistics::answer(&query, None).unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
}

#[test]
fn equal_daily_statistics_carry_the_actual_supplying_read() {
    let frames = [statistics(100, vec![stat(3, [0; 24])])];
    let mut hours = observed(&frames, &[]);
    hours.push(Hour::new(101, false, None, 0));
    hours.push(Hour::new(102, true, None, 0));
    let stored = publish_hours(hours, &[102], None).store();
    let (_, window) = stored.window(100, 102);
    assert_eq!(window.reads, [(100, 101, 100), (102, 102, 102)]);
    let c0 = window
        .series
        .iter()
        .find(|series| series.name.ends_with("|c0"))
        .unwrap();
    assert_eq!(c0.runs, [(100, 102, "0".into(), true)]);
}

/// The native publication holds what the row path's does, whatever segments start, wherever the
/// publisher cuts and after a resume: the units in the same order, every value's runs, the known
/// hours, the reads and the refusals.
#[test]
fn segments_cuts_and_resumes_publish_what_every_hour_observed_whole_does() {
    let frames = native_statistics();
    let (from, to) = (frames[0].hour, frames[frames.len() - 1].hour);
    let (refusals, expected) = publish_hours(observed(&frames, &[]), &[to], None)
        .store()
        .window(from, to);
    assert!(!refusals.is_empty());
    for (starts, cuts, resume) in [
        (&[][..], &[132][..], None),
        (&[4, 9][..], &[106, 114, 132][..], Some(114)),
        (
            &[3, 14, 15, 20][..],
            &[104, 117, 122, 127, 132][..],
            Some(122),
        ),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let starts = resuming(&frames, starts, resume);
        let hours = operate_segmented(&frames, &starts, native_statistics_tables, || {
            keyed::Operator::<Segments>::new(FormatVersion::CURRENT).unwrap()
        });
        let (native, window) = publish_hours(hours, cuts, resume)
            .store()
            .window(from, to);
        assert_eq!(native, refusals, "{label}: refusals");
        assert_eq!(window.known, expected.known, "{label}: known hours");
        assert_eq!(window.reads, expected.reads, "{label}: reads");
        assert_eq!(window.series.len(), expected.series.len(), "{label}: series");
        for (series, expected) in window.series.iter().zip(&expected.series) {
            assert_eq!(series.name, expected.name, "{label}: series order");
            assert_eq!(series.runs, expected.runs, "{label}: {}'s runs", series.name);
        }
    }
}
