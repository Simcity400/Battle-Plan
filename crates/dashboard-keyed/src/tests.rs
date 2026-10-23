use crate::{Observer, Scan, Values, countries::Countries, query};
use dashboard_base::prepared::prepass::Family;
use dashboard_base::test_fixtures::world_frame;
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{NavalRegionalMineRow, TableId, TableRows, tables},
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};

/// A test observer: each owner's mines in each region, by name.
#[derive(Default, Serialize, Deserialize)]
struct Mines {
    observed: u32,
}

static FAMILY: Family = crate::block::family::<Mines>(
    "test-mines",
    "test-mines.extract",
    "test-mines.sequential",
    "test-mines.fallback",
);

impl Observer for Mines {
    const NAME: &'static str = "test-mines";
    const TABLES: &'static [TableId] = &[
        TableId::Countries,
        TableId::CountryLinks,
        TableId::NavalRegionalMines,
    ];
    const READ: TableId = TableId::NavalRegionalMines;

    fn family() -> &'static Family {
        &FAMILY
    }

    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![TableFields::all(TableId::NavalRegionalMines)])
    }

    fn observe(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let (Some(countries), Ok(rows)) = (
            Countries::read(frame),
            frame.complete::<tables::NavalRegionalMines>(),
        ) else {
            return Ok(false);
        };
        self.observed += 1;
        for row in rows.iter().filter(|row| row.kind == 1) {
            let Some(owner) = countries.raw(row.country) else {
                continue;
            };
            values.push(format!("{owner}|{}", row.region), i128::from(row.amount));
        }
        Ok(true)
    }
}

fn frame(hour: i32, mines: Option<&[(u32, u32, i64)]>) -> DecodedFrame {
    let mut frame = world_frame(hour, false);
    *frame.tables[TableId::NavalRegionalMines.index()] = match mines {
        Some(mines) => TableView::Complete(TableRows::NavalRegionalMines(
            mines
                .iter()
                .map(|&(country, region, amount)| NavalRegionalMineRow {
                    country,
                    kind: 1,
                    region,
                    amount,
                })
                .collect(),
        )),
        None => TableView::Unavailable {
            reason: "Synthetic missing mines".into(),
        },
    };
    frame
}

#[test]
fn keyed_runs_keep_values_absence_unknown_hours_and_continue_across_flushes() {
    let format = FormatVersion::CURRENT;
    let frames = [
        frame(100, Some(&[(11, 3, 5)])),
        frame(101, Some(&[(11, 3, 5)])),
        frame(102, Some(&[(11, 3, 7), (21, 4, 2)])),
        frame(103, None),
        frame(104, Some(&[(9001, 3, 7)])),
    ];
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<Mines>::default();
    for batch in [&frames[..2], &frames[2..]] {
        let mut writer = fixture.writer();
        for frame in batch {
            scan.prepare(format, frame, true).unwrap();
        }
        scan.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        scan = dashboard_base::prepared::series::read_catalogue::<Scan<Mines>>(
            &fixture.store,
            &fixture.key,
            Mines::NAME,
            1,
        )
        .unwrap()
        .unwrap()
        .1;
    }
    assert_eq!(scan.observer.observed, 4);
    let mut opened = query::open::<Mines>(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    let window = opened.window(100, 104, None, |_| true).unwrap();
    assert_eq!(
        window
            .known
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(100, 102), (104, 104)]
    );
    // Raw references 11 and 9001 both link to GER's slot: one name.
    let runs = |name: &str| {
        window
            .series
            .iter()
            .find(|series| series.name == name)
            .map(|series| series.runs.clone())
            .unwrap_or_default()
    };
    assert_eq!(
        runs("GER@7|3"),
        [
            (100, 101, "5".to_owned(), true),
            (102, 102, "7".to_owned(), false),
            (104, 104, "7".to_owned(), true),
        ]
    );
    // A name first read starts its own observation.
    assert_eq!(runs("ITA@8|4"), [(102, 102, "2".to_owned(), true)]);
    assert!(
        window
            .reads
            .iter()
            .all(|(first, last, read)| first == last && read == first)
    );
    // Clipped to an accepted prefix.
    let early = opened
        .window(100, 104, Some(101), |name| name.starts_with("GER"))
        .unwrap();
    assert_eq!(early.series.len(), 1);
    assert_eq!(early.series[0].runs, [(100, 101, "5".to_owned(), true)]);
}

#[test]
fn keyed_quiet_hours_hold_values_and_date_them_by_their_reads() {
    let format = FormatVersion::CURRENT;
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<Mines>::default();
    let mut writer = fixture.writer();
    scan.prepare(format, &frame(10, Some(&[(21, 1, 9)])), true)
        .unwrap();
    scan.advance_quiet(11, false).unwrap();
    scan.advance_quiet(12, true).unwrap();
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut opened = query::open::<Mines>(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    let window = opened.window(10, 12, None, |_| true).unwrap();
    assert_eq!(window.series[0].runs, [(10, 12, "9".to_owned(), true)]);
    assert_eq!(window.reads, [(10, 11, 10), (12, 12, 12)]);
}

/// A test observer that refuses on request and vouches for its last read.
#[derive(Default, Serialize, Deserialize)]
struct Vouching {
    read: Option<i32>,
    vouch: bool,
}

static VOUCHING: Family = crate::block::family::<Vouching>(
    "test-vouching",
    "test-vouching.extract",
    "test-vouching.sequential",
    "test-vouching.fallback",
);

impl Observer for Vouching {
    const NAME: &'static str = "test-vouching";
    const TABLES: &'static [TableId] = &[TableId::NavalRegionalMines];
    const READ: TableId = TableId::NavalRegionalMines;

    fn family() -> &'static Family {
        &VOUCHING
    }

    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![TableFields::all(TableId::NavalRegionalMines)])
    }

    fn supplying_read(&self) -> Option<i32> {
        self.read.filter(|_| self.vouch)
    }

    fn observe(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let Ok(rows) = frame.complete::<tables::NavalRegionalMines>() else {
            return Ok(false);
        };
        values.push("total".into(), rows.iter().map(|row| i128::from(row.amount)).sum());
        Ok(true)
    }
}

fn vouched(frames: &[(DecodedFrame, bool)], vouch: bool) -> query::KeyedWindow {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<Vouching>::default();
    let mut writer = fixture.writer();
    for (frame, read) in frames {
        if *read {
            scan.observer.read = Some(frame.hour);
        }
        scan.observer.vouch = vouch;
        scan.prepare(FormatVersion::CURRENT, frame, *read).unwrap();
    }
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let first = frames.first().unwrap().0.hour;
    let last = frames.last().unwrap().0.hour;
    query::open::<Vouching>(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap()
        .window(first, last, None, |_| true)
        .unwrap()
}

#[test]
fn a_resumption_after_refused_hours_keeps_its_vouched_read_but_never_across_a_gap() {
    let mines = Some(&[(11, 3, 5)][..]);
    // Read at 10, refused at 11 and 12, resumed (no fresh read) at 13.
    let refused = [
        (frame(10, mines), true),
        (frame(11, None), false),
        (frame(12, None), false),
        (frame(13, mines), false),
    ];
    assert_eq!(vouched(&refused, true).reads, [(10, 10, 10), (13, 13, 10)]);
    // Without the observer's evidence the resumed hour stays undated.
    assert_eq!(vouched(&refused, false).reads, [(10, 10, 10)]);
    // A gap between the refusal and the resumption drops the held read.
    let gapped = [
        (frame(10, mines), true),
        (frame(11, None), false),
        (frame(13, mines), false),
    ];
    assert_eq!(vouched(&gapped, true).reads, [(10, 10, 10)]);
    // A gap before the refusal does too.
    let late = [
        (frame(10, mines), true),
        (frame(12, None), false),
        (frame(13, mines), false),
    ];
    assert_eq!(vouched(&late, true).reads, [(10, 10, 10)]);
}

/// Gives each hour after a known one as its changes from the whole values it last returned.
#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
struct Changing<O> {
    inner: O,
    #[serde(skip)]
    last: Option<std::collections::BTreeMap<String, i128>>,
}

impl<O: Observer> Observer for Changing<O> {
    const NAME: &'static str = O::NAME;
    const TABLES: &'static [TableId] = O::TABLES;
    const READ: TableId = O::READ;

    fn family() -> &'static Family {
        O::family()
    }

    fn fields() -> io::Result<Vec<TableFields>> {
        O::fields()
    }

    fn observe(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        self.last = None;
        let known = self.inner.observe(format, frame, values)?;
        if known {
            self.last = Some(values.iter().map(|(n, v)| (n.to_owned(), v)).collect());
        }
        Ok(known)
    }

    fn observe_changes(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        _: &crate::delta::Names<'_>,
        delta: &mut crate::Delta,
    ) -> io::Result<Option<bool>> {
        let Some(last) = self.last.take() else {
            return Ok(None);
        };
        let mut values = Values::default();
        if !self.inner.observe(format, frame, &mut values)? {
            return Ok(Some(false));
        }
        let now: std::collections::BTreeMap<String, i128> =
            values.iter().map(|(n, v)| (n.to_owned(), v)).collect();
        for name in last.keys().filter(|name| !now.contains_key(*name)) {
            delta.remove(name.clone());
        }
        // Each changed name once, where a whole observation first pushes it.
        let mut seen = std::collections::BTreeSet::new();
        for (name, _) in values.iter() {
            if seen.insert(name) && last.get(name) != now.get(name) {
                delta.set(name.to_owned(), now[name]);
            }
        }
        self.last = Some(now);
        Ok(Some(true))
    }
}

#[test]
fn hours_given_as_changes_publish_exactly_as_whole_hours() {
    let format = FormatVersion::CURRENT;
    let frames = [
        frame(100, Some(&[(11, 3, 5)])),
        frame(101, Some(&[(11, 3, 5), (9001, 3, 6)])),
        frame(102, Some(&[(11, 3, 7), (21, 4, 2)])),
        frame(103, Some(&[(21, 4, 2), (21, 5, 1), (11, 6, 1)])),
        frame(104, None),
        frame(105, Some(&[(9001, 3, 7)])),
        frame(106, Some(&[(9001, 3, 7), (21, 9, 4)])),
        frame(107, Some(&[(11, 3, 1), (9001, 3, 7)])),
        frame(108, Some(&[])),
        frame(109, Some(&[(21, 4, 3)])),
    ];
    let publish = |changing: bool, restore: bool| {
        let fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let flag = AtomicBool::new(false);
        let mut whole = Scan::<Mines>::default();
        let mut changes = Scan::<Changing<Mines>>::default();
        for batch in [&frames[..3], &frames[3..6], &frames[6..]] {
            let mut writer = fixture.writer();
            for frame in batch {
                if changing {
                    changes.prepare(format, frame, true).unwrap();
                } else {
                    whole.prepare(format, frame, true).unwrap();
                }
            }
            if changing {
                changes.flush(&mut writer, 1, &flag).unwrap();
            } else {
                whole.flush(&mut writer, 1, &flag).unwrap();
            }
            writer.commit().unwrap();
            if restore {
                changes =
                    serde_json::from_slice(&serde_json::to_vec(&changes).unwrap()).unwrap();
            }
        }
        fixture.payloads()
    };
    let whole = publish(false, false);
    dashboard_base::prepared::series::tests::assert_payloads_equal(
        &publish(true, false),
        &whole,
        "changes",
    );
    dashboard_base::prepared::series::tests::assert_payloads_equal(
        &publish(true, true),
        &whole,
        "restored changes",
    );
}
