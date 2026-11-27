use super::{Arc, BTreeMap, Bounds, DecodedFrame, Geometry, GroupDefines, Memo, ProvinceRow, core, io, tables};
use viewer_counter_kit::counter_groups::{self, Icon, Level};
use replay_data::rows::{TableId, TableRows};
use replay_frame::frame::TableView;

#[derive(Clone)]
struct Content {
    bounds: Arc<BTreeMap<u32, Bounds>>,
    regions: Arc<BTreeMap<u32, u32>>,
    defines: GroupDefines,
    width: f32,
}

impl Default for Content {
    fn default() -> Self {
        Self {
            bounds: Arc::new(
                [
                    (1, [-0.0, 1.25, 5.5, 2.75]),
                    (2, [20.5, 2.5, 45.0, 6.25]),
                    (3, [90.0, 4.0, 110.0, 12.0]),
                    (4, [198.0, -2.0, 205.0, 3.0]),
                ]
                .into(),
            ),
            regions: Arc::new([(1, 10), (2, 10), (3, 20), (4, 20)].into()),
            defines: GroupDefines {
                state_huge: 40.0,
                strategic_area_huge: 100.0,
                ..GroupDefines::FIXTURE
            },
            width: 200.0,
        }
    }
}

impl Content {
    fn read(&self, memo: &Memo, frame: &DecodedFrame) -> io::Result<Arc<Geometry>> {
        memo.geometry(
            frame,
            &self.bounds,
            &self.regions,
            &self.defines,
            self.width,
        )
    }

    fn compare(&self, memo: &Memo, frame: &DecodedFrame) -> Arc<Geometry> {
        let reference = Geometry::build(
            core::<tables::Provinces>(frame).unwrap(),
            &self.bounds,
            &self.regions,
            &self.defines,
            self.width,
        );
        let actual = self.read(memo, frame).unwrap();
        assert_eq!(reference.state_of, actual.state_of);
        reference.extents.assert_same_bits(&actual.extents);
        let icons = |geometry: &Geometry| {
            self.bounds
                .keys()
                .map(|&province| Icon {
                    kind: 1,
                    owner: 1 + province % 2,
                    province,
                    state: geometry.state_of.get(&province).copied(),
                    region: self.regions.get(&province).copied(),
                    anchor: [province as f32 * 49.5, 3.25, province as f32 * 2.0],
                    track: None,
                })
                .collect::<Vec<_>>()
        };
        for level in [Level::None, Level::Province, Level::State, Level::Strategic] {
            let a = icons(&reference);
            let b = icons(&actual);
            let a =
                counter_groups::group(&a, &vec![level; a.len()], &reference.extents, &|_, _| true);
            let b = counter_groups::group(&b, &vec![level; b.len()], &actual.extents, &|_, _| true);
            let bits = |groups: Vec<counter_groups::Group>| {
                groups
                    .into_iter()
                    .map(|group| (group.members, group.anchor.map(f32::to_bits)))
                    .collect::<Vec<_>>()
            };
            assert_eq!(bits(a), bits(b));
        }
        actual
    }
}

fn frame(hour: i32, states: &[(u32, u32)]) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Default::default(),
        tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
    };
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        states
            .iter()
            .map(|&(id, state)| ProvinceRow {
                id,
                state,
                ..Default::default()
            })
            .collect(),
    ));
    frame
}

#[test]
fn geography_memo_preserves_geometry_across_hours_reordering_duplicates_and_seek() {
    let memo = Memo::default();
    let content = Content::default();
    let first = frame(100, &[(1, 7), (2, 7), (3, 8), (4, 0)]);
    let a = content.compare(&memo, &first);
    assert_eq!(a.state_of, [(1, 7), (2, 7), (3, 8)].into());
    assert!(a.extents.state_is_huge(7));
    assert!(a.extents.region_is_huge(20));
    for hour in 101..111 {
        let mut next = frame(hour, &[(1, 7), (2, 7), (3, 8), (4, 0)]);
        if let TableView::Complete(TableRows::Provinces(rows)) =
            &mut *next.tables[TableId::Provinces.index()]
        {
            rows[0].owner = hour as u32;
            rows[1].controller = (hour + 1) as u32;
        }
        let b = content.compare(&memo, &next);
        assert!(Arc::ptr_eq(&a, &b));
    }
    let changed = content.compare(&memo, &frame(111, &[(1, 9), (2, 7), (3, 8), (4, 0)]));
    assert!(!Arc::ptr_eq(&a, &changed));
    assert!(!changed.extents.state_is_huge(7));
    let duplicates = [(1, 7), (2, 7), (1, 9), (1, 0), (3, 8), (4, 0)];
    let b = content.compare(&memo, &frame(112, &duplicates));
    assert_eq!(b.state_of.get(&1), Some(&9));
    let mut reordered = duplicates;
    reordered.swap(0, 2);
    let c = content.compare(&memo, &frame(113, &reordered));
    assert_eq!(c.state_of.get(&1), Some(&7));
    let mut seek = first;
    seek.branch = 2;
    seek.hour = 3;
    let d = content.compare(&memo, &seek);
    assert!(Arc::ptr_eq(&a, &d));
    assert_eq!(a.state_of.get(&1), Some(&7));
}

#[test]
fn geography_core_refusals_precede_lookup_even_after_a_complete_hit() {
    let content = Content::default();
    let memo = Memo::default();
    let mut frame = frame(7, &[(1, 8)]);
    let complete = content.compare(&memo, &frame);
    assert!(Arc::ptr_eq(&complete, &content.compare(&memo, &frame)));
    let bad = [
        TableView::Truncated {
            rows: TableRows::Provinces(vec![ProvinceRow {
                id: 1,
                state: 8,
                ..Default::default()
            }]),
            missing: 4,
        },
        TableView::Unavailable {
            reason: "synthetic unavailable province census".into(),
        },
        TableView::Complete(TableRows::Countries(Vec::new())),
    ];
    let expected = [
        "hour 7 Provinces is truncated (4 missing rows); the core tables Provinces, CountryLinks and Countries carry the political map itself, so this hour cannot be drawn",
        "hour 7 Provinces is unavailable: synthetic unavailable province census; the core tables Provinces, CountryLinks and Countries carry the political map itself, so this hour cannot be drawn",
        "provinces holds rows of another kind",
    ];
    for (bad, expected) in bad.into_iter().zip(expected) {
        *frame.tables[TableId::Provinces.index()] = bad;
        let why = content.read(&memo, &frame).err().expect("core refusal");
        assert_eq!(why.kind(), io::ErrorKind::InvalidData);
        assert_eq!(why.to_string(), expected);
    }
    *frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(Vec::new()));
    let empty = content.compare(&memo, &frame);
    assert!(empty.state_of.is_empty());
    assert!(Arc::ptr_eq(&empty, &content.compare(&memo, &frame)));
}

#[test]
fn geography_content_keys_retain_allocations_and_exact_scalar_bits() {
    let memo = Memo::default();
    let mut content = Content::default();
    let frame = frame(1, &[(1, 7), (2, 7), (3, 8)]);
    let a = content.compare(&memo, &frame);
    let before = Arc::as_ptr(&content.bounds);
    assert!(
        Arc::strong_count(&content.bounds) > 1,
        "the key must retain bounds"
    );
    Arc::make_mut(&mut content.bounds).insert(2, [5.0, 2.5, 10.0, 6.25]);
    assert_ne!(before, Arc::as_ptr(&content.bounds));
    let b = content.compare(&memo, &frame);
    assert!(a.extents.state_is_huge(7));
    assert!(!b.extents.state_is_huge(7));
    let mut previous = b;
    Arc::make_mut(&mut content.regions).insert(2, 20);
    let next = content.compare(&memo, &frame);
    assert!(!Arc::ptr_eq(&previous, &next));
    previous = next;
    content.defines.state_huge = 1.0;
    let next = content.compare(&memo, &frame);
    assert!(!Arc::ptr_eq(&previous, &next));
    previous = next;
    content.defines.strategic_area_huge = 2.0;
    let next = content.compare(&memo, &frame);
    assert!(!Arc::ptr_eq(&previous, &next));
    previous = next;
    content.width = 400.0;
    let next = content.compare(&memo, &frame);
    assert!(!Arc::ptr_eq(&previous, &next));
    previous = next;
    for limit in [0.0, -0.0, f32::INFINITY, f32::from_bits(0x7fc0_0001)] {
        content.defines.state_huge = limit;
        let next = content.compare(&memo, &frame);
        assert!(!Arc::ptr_eq(&previous, &next));
        assert!(Arc::ptr_eq(&next, &content.compare(&memo, &frame)));
        previous = next;
    }
    let other = Content::default();
    let next = other.compare(&memo, &frame);
    assert!(!Arc::ptr_eq(&previous, &next));
    assert!(Arc::ptr_eq(&next, &other.compare(&memo, &frame)));
}

#[test]
fn geography_memo_keeps_a_held_build_and_concurrent_requests_build_the_same_geometry() {
    let memo = Memo::default();
    let content = Content::default();
    let first = frame(1, &[(1, 7)]);
    let barrier = std::sync::Barrier::new(8);
    let reads = std::thread::scope(|scope| {
        let threads = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    content.read(&memo, &first).unwrap()
                })
            })
            .collect::<Vec<_>>();
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    for read in &reads[1..] {
        assert_eq!(read.state_of, reads[0].state_of);
        read.extents.assert_same_bits(&reads[0].extents);
    }
    for state in 20..28 {
        let next = content.compare(&memo, &frame(state as i32, &[(1, state)]));
        assert!(!Arc::ptr_eq(&reads[0], &next));
    }
    let rebuilt = content.compare(&memo, &first);
    assert!(reads.iter().any(|read| Arc::ptr_eq(read, &rebuilt)));
    reads[0].extents.assert_same_bits(&rebuilt.extents);
}
