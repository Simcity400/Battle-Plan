//! Change-driven yard observation against every hour observed whole.
use super::*;
use dashboard_base::prepared::{
    frame_changes::{FrameChangeTracker, FrameChanges},
    prepass::{self, Family},
    series::tests::assert_payloads_equal,
};
use replay::container::TableFields;
use replay_frame::frame::SharedView;
use serde::{Deserialize, Serialize};
use serde_json::json;
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
    fn bind(
        &mut self,
        source: &dashboard_base::Source,
        content: Option<&dashboard_core::config::ContentOptions>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.0.bind(source, content, check)
    }
    fn supplying_read(&self) -> Option<i32> {
        self.0.supplying_read()
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
    1000 + country
}
fn ship_id(country: u32, ordinal: u64) -> u64 {
    ((u64::from(country) + 1) << 32) | ordinal
}
fn task_id(country: u32, ordinal: u64) -> u64 {
    ((u64::from(country) + 1) << 32) | (100 + ordinal)
}
fn line_id(country: u32) -> u64 {
    ((u64::from(country) + 1) << 32) | 900
}
fn port_of(country: u32) -> u32 {
    10 + country
}
fn tag(country: u32) -> String {
    String::from_utf8(test_fixtures::scale_tag(country)).unwrap()
}

fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let table = rows.table();
    frame.tables[table.index()] = TableView::Complete(rows).into();
}

/// Navies with every kind of churn the yard observers read.
fn world(countries: u32, hour: i32) -> DecodedFrame {
    let n = countries;
    let h = hour;
    let mut frame = test_fixtures::source(1, hour, Some(1));
    let present = |i: u32| !(i == 1 && (14..20).contains(&h)) && !(i + 1 == n && n > 2 && h < 12);
    put(
        &mut frame,
        TableRows::Countries(
            (0..n)
                .filter(|&i| present(i))
                .map(|i| CountryRow {
                    id: raw(i),
                    canonical_slot: i + 1,
                    tag: test_fixtures::scale_tag(i),
                    political_power_raw: Some(i64::from(h)),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::CountryLinks(
            (0..n)
                .map(|i| CountryLinkRow {
                    id: raw(i),
                    canonical_slot: i + 1,
                })
                .collect(),
        ),
    );
    put(&mut frame, TableRows::CountryAppearance(Vec::new()));
    put(
        &mut frame,
        TableRows::ReservedCountryNaming(vec![Default::default()]),
    );
    put(
        &mut frame,
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 5,
            literal: b"ship_class".to_vec(),
        }]),
    );
    put(
        &mut frame,
        TableRows::VariantDefinitions(
            (1..=if h >= 25 { 4 } else { 3 })
                .map(|id| VariantDefinitionRow {
                    id,
                    equipment_token: 5,
                    creator_link: raw(0),
                    display_name: format!("Design {id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::Provinces(
            (0..n)
                .map(|i| ProvinceRow {
                    id: port_of(i),
                    state: i + 1,
                    controller: raw(i) + h as u32,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::States(
            (0..n)
                .map(|i| StateRow {
                    id: i + 1,
                    owner: raw(i),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(&mut frame, TableRows::ProvinceNames(Vec::new()));
    put(
        &mut frame,
        TableRows::NavalBases(
            (0..n)
                .map(|i| NavalBaseRow {
                    province: port_of(i),
                    // Captured, and briefly held by a raw reference no country links.
                    country: if i == 0 && h >= 22 && n > 2 {
                        raw(2)
                    } else if i == 3 && (30..33).contains(&h) {
                        9
                    } else {
                        raw(i)
                    },
                    level: h,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::NavalRepairBases(
            (0..n)
                .filter(|i| i % 4 != 2)
                .map(|i| NavalRepairBaseRow {
                    province: port_of(i),
                    priority: (i % 3) as i32 - 1,
                    allocated_dockyards: if i % 5 == 0 { h % 4 } else { 2 },
                })
                .collect(),
        ),
    );
    let exists = |i: u32, k: u64| match k {
        1 => !(i % 7 == 0 && h >= 18),
        3 => i % 3 == 0 && h >= 16,
        _ => true,
    };
    let disbanded = |i: u32, j: u64| j == 1 && i % 5 == 4 && h >= 24;
    let mut ships = Vec::new();
    let mut models = Vec::new();
    let mut tasks = Vec::new();
    let mut counters = Vec::new();
    let mut orders = Vec::new();
    let mut children = Vec::new();
    let mut queue = Vec::new();
    let mut links = Vec::new();
    let mut progress = Vec::new();
    let mut lines = Vec::new();
    for i in 0..n {
        for k in (0..4).filter(|&k| exists(i, k)) {
            ships.push(ShipRow {
                id: ship_id(i, k),
                task_force: if i % 6 == 0 && k == 0 && h >= 27 {
                    task_id(i, 1)
                } else {
                    task_id(i, k % 2)
                },
                strength: 100_000 + 1000 * k as i64 + if i % 4 == 0 { i64::from(h) * 100 } else { 0 },
                organisation: i64::from(h),
                class_id: 5,
                name: Recorded::Value((k != 2).then(|| b"Ship".to_vec())),
                naming: Recorded::Value(None),
                ..Default::default()
            });
            if k != 2 {
                models.push(ShipModelRow {
                    ship: ship_id(i, k),
                    variant: 1 + k % 4,
                    counter: Some(ShipCounterInputs {
                        maximum_strength: 200_000,
                        ..Default::default()
                    }),
                    ..Default::default()
                });
            }
        }
        for j in (0..2).filter(|&j| !disbanded(i, j)) {
            tasks.push(TaskForceRow {
                id: task_id(i, j),
                roster_owner: if i % 6 == 3 && j == 1 && h >= 21 {
                    raw((i + 1) % n)
                } else {
                    raw(i)
                },
                province: h as u32,
                ..Default::default()
            });
            counters.push(TaskForceCounterInputRow {
                task_force: task_id(i, j),
                name: Recorded::Value(Some(if i % 5 == 0 && h >= 19 {
                    b"Renamed".to_vec()
                } else {
                    b"Task".to_vec()
                })),
                received_supply_ratio: i64::from(h),
                ..Default::default()
            });
        }
        orders.push(NavalRepairTaskForceRow {
            task_force: task_id(i, 0),
            repair_mode: 1,
            merge_after_repair: task_id(i, 1),
            refit_variant: 2,
            refit_after_repair: if h >= 29 { 3 } else { 0 },
            ..Default::default()
        });
        children.push(NavalRepairChildRow {
            task_force: task_id(i, 0),
            ordinal: 0,
            child: task_id(i, 1),
        });
        let mut ordinal = 0;
        let mut first = None;
        for k in (0..4).filter(|&k| exists(i, k) && (h as u64 + k + u64::from(i)) % 3 != 0) {
            first.get_or_insert(ship_id(i, k));
            queue.push(NavalRepairQueueRow {
                province: port_of(i),
                ordinal,
                ship: ship_id(i, k),
            });
            ordinal += 1;
        }
        if let Some(ship) = first.filter(|_| i % 4 == 1) {
            queue.push(NavalRepairQueueRow {
                province: port_of(i),
                ordinal,
                ship,
            });
        }
        let refitting = (h / 4 + i as i32) % 2 == 0;
        if refitting {
            links.push(NavalRefitShipRow {
                ship: ship_id(i, 0),
                line: line_id(i),
            });
        }
        if i % 3 == 0 && exists(i, 1) {
            links.push(NavalRefitShipRow {
                ship: ship_id(i, 1),
                line: line_id(i),
            });
        }
        // A stale carried link whose line holds no progress.
        if i % 8 == 3 {
            links.push(NavalRefitShipRow {
                ship: ship_id(i, 2),
                line: line_id(i) + 1,
            });
        }
        if refitting || (i % 3 == 0 && exists(i, 1)) {
            progress.push(NavalRefitProgressRow {
                line: line_id(i),
                progress_raw: 1000 * i64::from(h % 4 + 1) * (i64::from(i) + 1),
            });
        }
        if i % 7 != 5 {
            lines.push(ProductionLineRow {
                handle: line_id(i),
                country: raw(i),
                cost_raw: if i % 9 == 4 { 0 } else { 1_000_000 },
                active_factories: 1 + (i % 2) as i32,
                variant: 2,
                speed_raw: i64::from(h),
                ..Default::default()
            });
        }
    }
    put(&mut frame, TableRows::Ships(ships));
    put(&mut frame, TableRows::ShipModels(models));
    put(&mut frame, TableRows::TaskForces(tasks));
    put(&mut frame, TableRows::TaskForceCounterInputs(counters));
    put(&mut frame, TableRows::NavalRepairTaskForces(orders));
    put(&mut frame, TableRows::NavalRepairChildren(children));
    put(&mut frame, TableRows::NavalRepairQueue(queue));
    put(
        &mut frame,
        TableRows::NavalRepairCountries(
            (0..n)
                .map(|i| NavalRepairCountryRow {
                    country: raw(i),
                    num_used_dockyards: if i % 2 == 0 { h % 5 } else { 1 },
                    max_allowed_dockyards_to_repair: 10,
                    max_factories_for_repair: 3,
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::CountryFactories(
            (0..n)
                .map(|i| CountryFactoriesRow {
                    country: raw(i),
                    naval_total_raw: 300_000 * (1 + i64::from(i % 3))
                        + if h >= 24 { 100_000 } else { 0 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(&mut frame, TableRows::NavalRefitShips(links));
    put(&mut frame, TableRows::NavalRefitProgress(progress));
    put(&mut frame, TableRows::ProductionLines(lines));
    if h == 26 {
        missing(&mut frame, TableId::ProductionLines);
    }
    if h == 33 {
        missing(&mut frame, TableId::Ships);
    }
    if h % 8 == 0 {
        frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
        frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
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
fn every_hour<O: Observer>(
    source: &dashboard_base::Source,
    frames: &[DecodedFrame],
    cuts: &[usize],
) -> Fixture {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<O>::default();
    scan.observer.bind(source, None, &mut || Ok(())).unwrap();
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
fn prepare<O: Observer>(
    source: &dashboard_base::Source,
    frames: &[DecodedFrame],
    cuts: &[usize],
    restore: bool,
) -> Fixture {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<O>::default();
    scan.observer.bind(source, None, &mut || Ok(())).unwrap();
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
            scan.observer.bind(source, None, &mut || Ok(())).unwrap();
        }
        start = end;
    }
    fixture
}

fn replies(
    source: &dashboard_base::Source,
    fixture: &Fixture,
    tags: &[String],
    (first, last): (i32, i32),
) -> Vec<serde_json::Value> {
    let flag = AtomicBool::new(false);
    let prefix = prefix(first, last, true);
    let mut replies = Vec::new();
    for tag in tags {
        for params in [
            json!({"branchId": "1", "tag": tag}),
            json!({"branchId": "1", "tag": tag, "firstHour": first + 2, "lastHour": last - 2}),
        ] {
            let query = Query {
                source,
                params: &params,
                content: None,
                store: &fixture.store,
                key: &fixture.key,
                branch: 1,
                prefix: &prefix,
                cancelled: &flag,
            };
            replies.push(naval_repairs::query(&query).unwrap());
            replies.push(naval_refits::query(&query).unwrap());
        }
    }
    replies
}

/// Pages and replies prepared from changes, with and without restores, equal main's whole observation every hour.
fn check(countries: u32, hours: std::ops::RangeInclusive<i32>, cuts: &[usize], tags: &[String]) {
    let window = (*hours.start(), *hours.end());
    let frames = worlds(countries, hours);
    let source = source(frames.clone());
    fn family<O: Observer, R: Observer>(
        source: &dashboard_base::Source,
        frames: &[DecodedFrame],
        cuts: &[usize],
    ) -> [Fixture; 4] {
        let reference = every_hour::<R>(source, frames, cuts);
        let projects = prepass::noted("naval-yards.project");
        let whole = prepare::<Whole<O>>(source, frames, cuts, false);
        let whole_projects = prepass::noted("naval-yards.project") - projects;
        let projects = prepass::noted("naval-yards.project");
        let changed = prepare::<O>(source, frames, cuts, false);
        let changed_projects = prepass::noted("naval-yards.project") - projects;
        let restored = prepare::<O>(source, frames, cuts, true);
        for fixture in [&whole, &changed, &restored] {
            assert_payloads_equal(&fixture.payloads(), &reference.payloads(), O::NAME);
        }
        // Hours after the first of each block project only what changed.
        assert!(
            changed_projects < whole_projects,
            "{} projected {changed_projects} of {whole_projects} hours whole",
            O::NAME
        );
        [reference, whole, changed, restored]
    }
    let repairs = family::<RepairsObserver, yards::reference::Repairs>(&source, &frames, cuts);
    let refits = family::<RefitsObserver, yards::reference::Refits>(&source, &frames, cuts);
    for fixtures in [&repairs, &refits] {
        let whole = replies(&source, &fixtures[0], tags, window);
        for fixture in &fixtures[1..] {
            assert_eq!(replies(&source, fixture, tags, window), whole);
        }
    }
}

#[test]
fn changed_yard_hours_publish_and_reply_exactly_as_whole_hours_across_blocks_and_restores() {
    let tags: Vec<String> = [0, 1, 2, 3, 4, 6, 9, 11].into_iter().map(tag).collect();
    check(12, 7..=40, &[5, 11, 16, 23, 29], &tags);
}

#[test]
fn fifteen_hundred_country_navies_publish_exactly_as_whole_hours() {
    let tags: Vec<String> = [0, 1, 749, 1499].into_iter().map(tag).collect();
    check(1500, 14..=21, &[3], &tags);

}
