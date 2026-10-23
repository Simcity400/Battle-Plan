//! Change-driven naval control and minefields against every hour observed whole.
use super::*;
use dashboard_base::prepared::{
    frame_changes::{FrameChangeTracker, FrameChanges},
    prepass::Family,
    series::tests::{Fixture, assert_payloads_equal},
};
use replay::container::TableFields;
use replay_frame::frame::SharedView;
use serde::{Deserialize, Serialize};
use std::io;

/// The family with every hour observed whole: the reference its changes match.
#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
struct Whole<O>(O);

impl<O: Observer> Observer for Whole<O> {
    const NAME: &'static str = O::NAME;
    const TABLES: &'static [TableId] = O::TABLES;
    const READ: TableId = O::READ;
    fn family() -> &'static Family {
        O::family()
    }
    fn fields() -> io::Result<Vec<TableFields>> {
        O::fields()
    }
    fn changed(changes: &FrameChanges) -> bool {
        O::changed(changes)
    }
    fn observe(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        self.0.observe(format, frame, values)
    }
}

fn raw(country: u32) -> u32 {
    2000 + country
}
fn tag(country: u32) -> String {
    String::from_utf8(test_fixtures::scale_tag(country)).unwrap()
}
const REGIONS: [u32; 4] = [73, 74, 75, 80];

/// Contested seas and minefields with every kind of churn their observers read.
fn world(countries: u32, hour: i32) -> DecodedFrame {
    let n = countries;
    let h = hour;
    let mut frame = test_fixtures::source(1, hour, Some(1));
    let present = |i: u32| !(i == 1 && (14..20).contains(&h)) && !(i + 1 == n && h < 12);
    put(
        &mut frame,
        TableId::Countries,
        TableRows::Countries(
            (0..n)
                .filter(|&i| present(i))
                .map(|i| rows::CountryRow {
                    id: raw(i),
                    canonical_slot: i + 1,
                    tag: if i == 2 && h >= 23 {
                        b"REN".to_vec()
                    } else {
                        test_fixtures::scale_tag(i)
                    },
                    political_power_raw: Some(i64::from(h)),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::CountryLinks,
        TableRows::CountryLinks(
            (0..n)
                .map(|i| rows::CountryLinkRow {
                    id: raw(i),
                    canonical_slot: i + 1,
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::ReservedCountryNaming,
        TableRows::ReservedCountryNaming(vec![rows::ReservedCountryNamingRow {
            tag: if h >= 28 { b"RSV".to_vec() } else { b"RES".to_vec() },
            built: (h % 3) as u8,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableId::FactionMembers,
        TableRows::FactionMembers(
            (0..n)
                .filter(|i| i % 3 == 0 || (i % 3 == 1 && h >= 20))
                .map(|i| rows::FactionMemberRow {
                    country: raw(i),
                    faction: 1 + u64::from(i % 2),
                    ordinal: i,
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::Subjects,
        TableRows::Subjects(
            (1..n)
                .filter(|i| i % 4 == 1 && !(i % 8 == 1 && h >= 26))
                .map(|i| rows::SubjectRow {
                    overlord: raw(i - 1),
                    subject: raw(i),
                    autonomy_ratio: i64::from(h),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::WarRelations,
        TableRows::WarRelations(
            (0..n.saturating_sub(1))
                .filter(|i| i % 2 == 0 && (h >= 17 || i % 4 == 0))
                .map(|i| rows::WarRelationRow {
                    first_country: raw(i),
                    second_country: raw(i + 1),
                    first_casualties: i64::from(h),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::AirCountryRelations,
        TableRows::AirCountryRelations(
            (0..n)
                .filter(|i| i % 5 == 0 && h >= 15)
                .map(|i| rows::AirCountryRelationRow {
                    country: raw(i),
                    kind: 1,
                    ordinal: 0,
                    other: raw((i + 1) % n),
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::RelationTuples,
        TableRows::RelationTuples(
            (0..n)
                .filter(|i| i % 3 == 2)
                .map(|i| rows::RelationTupleRow {
                    owner_country: raw(i),
                    target_country: raw((i + 1) % n),
                    type_id: 4,
                    ordinal: 0,
                    first_country: raw(i),
                    second_country: if h >= 21 { raw((i + 2) % n) } else { raw((i + 1) % n) },
                    start_hour: h,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableId::CountryCounterInputs,
        TableRows::CountryCounterInputs(
            (0..n)
                .map(|i| rows::CountryCounterInputRow {
                    country: raw(i),
                    forward_visibility: rows::Recorded::Value((i % 6 == 5).then(|| {
                        rows::ReverseZeroVisibilityInputs {
                            target: Some(raw((i + 1) % n)),
                            war: Some([u8::from(h >= 19), 0]),
                            subject: None,
                            military_access: Some(u8::from(h >= 25)),
                        }
                    })),
                    owned_state_count: h,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    // Daily reads at midnight; the currents also move between them.
    let read = h - h.rem_euclid(24);
    let mut currents = Vec::new();
    for &region in &REGIONS {
        if region == 80 {
            // The reserved country and a reference no country links.
            currents.push(rows::NavalSupremacyRow {
                region,
                country: 0,
                current: 5_000 + i64::from(h % 3),
                region_slot: 9,
                ..Default::default()
            });
        }
        for i in 0..n {
            let at_sea = (i + region + (h as u32) / 3) % 4 != 0;
            if !at_sea {
                continue;
            }
            currents.push(rows::NavalSupremacyRow {
                region,
                country: raw(i),
                current: 100_000 * i64::from(i % 5 + 1)
                    + if i % 3 == 0 { i64::from(h) * 10 } else { 0 },
                region_slot: if region == 74 && h >= 22 && i + 1 == n {
                    6
                } else {
                    region % 7
                },
                target: i64::from(h),
                ..Default::default()
            });
        }
        if region == 80 {
            currents.push(rows::NavalSupremacyRow {
                region,
                country: 77_777,
                current: 1,
                region_slot: 9,
                ..Default::default()
            });
        }
    }
    frame.tables[TableId::NavalSupremacy.index()] =
        SharedView::from(TableView::Complete(TableRows::NavalSupremacy(currents)))
            .with_sampled_at(Some(read));
    if h == read {
        frame.metrics.table_ns[TableId::NavalSupremacy.index()] = 1;
    }
    if h == 31 {
        frame.tables[TableId::NavalSupremacy.index()] = TableView::Unavailable {
            reason: "fixture".into(),
        }
        .into();
    }
    let mut mines = Vec::new();
    for i in 0..n {
        if i % 4 == 3 {
            continue;
        }
        let laid = |region: u32| (i + region + h as u32 / 2) % 3 == 0;
        let mut vector = vec![rows::NavalRegionalMineRow {
            country: raw(i),
            kind: 0,
            region: 90,
            amount: 0,
        }];
        // A witness short of the vector's values refuses the owner for a while.
        if i % 7 == 2 && (20..24).contains(&h) {
            vector[0].region = 74;
        }
        // Zero mines are no amount row under the witness.
        for region in REGIONS.into_iter().filter(|&region| laid(region) && i % 5 != 1) {
            vector.push(rows::NavalRegionalMineRow {
                country: raw(i),
                kind: 1,
                region,
                amount: 1_000 * i64::from(i + 1) + i64::from(h),
            });
        }
        mines.extend(vector);
    }
    // Vectors of a reference no country links.
    if h % 6 < 3 {
        mines.push(rows::NavalRegionalMineRow {
            country: 99_999,
            kind: 0,
            region: 90,
            amount: 0,
        });
    }
    if h == 35 {
        frame.tables[TableId::NavalRegionalMines.index()] = TableView::Unavailable {
            reason: "fixture".into(),
        }
        .into();
    } else {
        put(
            &mut frame,
            TableId::NavalRegionalMines,
            TableRows::NavalRegionalMines(mines),
        );
    }
    frame
}

/// The hours, sharing equal tables' allocations as a decoder does but every fifth hour.
fn worlds(countries: u32, hours: std::ops::RangeInclusive<i32>) -> Vec<DecodedFrame> {
    let mut frames: Vec<DecodedFrame> = Vec::new();
    for hour in hours {
        let mut frame = world(countries, hour);
        if let Some(before) = frames.last()
            && hour % 5 != 0
        {
            for (now, held) in frame.tables.iter_mut().zip(&before.tables) {
                if *now == *held {
                    *now = SharedView::clone(held).with_sampled_at(now.sampled_at());
                }
            }
        }
        frames.push(frame);
    }
    frames
}

/// `frames` observed whole every hour in blocks ending before each of `cuts`.
fn every_hour<O: Observer>(frames: &[DecodedFrame], cuts: &[usize]) -> Fixture {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<O>::default();
    let mut start = 0;
    for &end in cuts.iter().chain([frames.len()].iter()) {
        let mut tracker = FrameChangeTracker::new();
        for frame in &frames[start..end] {
            let read = tracker.observe(frame, FormatVersion::CURRENT).slot_read[O::READ.index()];
            scan.prepare(FormatVersion::CURRENT, frame, read).unwrap();
        }
        let mut writer = fixture.writer();
        scan.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        start = end;
    }
    fixture
}

/// `frames` prepared in blocks ending before each of `cuts`, each continued from its catalogue with `restore`.
fn prepare<O: Observer>(frames: &[DecodedFrame], cuts: &[usize], restore: bool) -> Fixture {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<O>::default();
    let mut start = 0;
    for &end in cuts.iter().chain([frames.len()].iter()) {
        let mut tracker = FrameChangeTracker::new();
        for frame in &frames[start..end] {
            let changes = tracker.observe(frame, FormatVersion::CURRENT);
            scan.prepare_changed(FormatVersion::CURRENT, frame, &changes)
                .unwrap();
        }
        let mut writer = fixture.writer();
        scan.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        if restore {
            scan = serde_json::from_slice(&serde_json::to_vec(&scan).unwrap()).unwrap();
        }
        start = end;
    }
    fixture
}

fn replies(
    source: &dashboard_base::Source,
    [supremacy, mines]: [&Fixture; 2],
    tags: &[String],
    (first, last): (i32, i32),
) -> Vec<serde_json::Value> {
    let flag = AtomicBool::new(false);
    let prefix = Prefix::new(
        source,
        1,
        Coverage {
            schema: 1,
            first_hour: first,
            last_hour: last,
            complete: true,
        },
        None,
    );
    let mut replies = Vec::new();
    let mut ask = |params: serde_json::Value, fixture: &Fixture| {
        let q = Query {
            source,
            params: &params,
            content: None,
            store: &fixture.store,
            key: &fixture.key,
            branch: 1,
            prefix: &prefix,
            cancelled: &flag,
        };
        replies.push(if std::ptr::eq(fixture, supremacy) {
            naval_supremacy::query(&q).unwrap()
        } else {
            minefields::query(&q).unwrap()
        });
    };
    for tag in tags {
        ask(json!({"branchId": "1", "tag": tag, "daily": true}), supremacy);
        ask(
            json!({"branchId": "1", "tag": tag, "daily": true, "firstHour": first + 2, "lastHour": last - 2}),
            supremacy,
        );
    }
    ask(json!({"branchId": "1"}), mines);
    ask(json!({"branchId": "1", "tags": tags}), mines);
    ask(
        json!({"branchId": "1", "tags": [tags[0]], "firstHour": first + 2, "lastHour": last - 2}),
        mines,
    );
    replies
}

/// Pages and replies prepared from changes, with and without restores, equal main's whole observation every hour.
fn check(countries: u32, hours: std::ops::RangeInclusive<i32>, cuts: &[usize], tags: &[String]) {
    let window = (*hours.start(), *hours.end());
    let frames = worlds(countries, hours);
    fn family<O: Observer, R: Observer>(frames: &[DecodedFrame], cuts: &[usize]) -> [Fixture; 4] {
        let reference = every_hour::<R>(frames, cuts);
        let whole = prepare::<Whole<O>>(frames, cuts, false);
        let changed = prepare::<O>(frames, cuts, false);
        let restored = prepare::<O>(frames, cuts, true);
        for fixture in [&whole, &changed, &restored] {
            assert_payloads_equal(&fixture.payloads(), &reference.payloads(), O::NAME);
        }
        [reference, whole, changed, restored]
    }
    let supremacy = family::<NavalSupremacyObserver, reference::Supremacy>(&frames, cuts);
    let mines = family::<MinefieldsObserver, reference::Minefields>(&frames, cuts);
    let source = source_for_queries(frames);
    let whole = replies(&source, [&supremacy[0], &mines[0]], tags, window);
    for at in 1..4 {
        assert_eq!(
            replies(&source, [&supremacy[at], &mines[at]], tags, window),
            whole
        );
    }
}

#[test]
fn changed_naval_hours_publish_and_reply_exactly_as_whole_hours_across_blocks_and_restores() {
    let tags: Vec<String> = [0, 1, 2, 3, 5, 9, 11]
        .into_iter()
        .map(tag)
        .chain(["REN".to_owned(), "RES".to_owned(), "RSV".to_owned()])
        .collect();
    check(12, 7..=40, &[5, 11, 16, 23, 29], &tags);
}

#[test]
fn fifteen_hundred_countries_contesting_seas_publish_exactly_as_whole_hours() {
    let tags: Vec<String> = [0, 1, 749, 1499].into_iter().map(tag).collect();
    check(1500, 14..=21, &[3], &tags);

}
